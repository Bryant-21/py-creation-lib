use crate::error::HavokResult;
use crate::hkx::types::HkxValue;
use crate::hkx::{HkxMember, HkxObject};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassVersion {
    pub class_name: String,
    pub version: i32,
}

impl ClassVersion {
    pub fn new(class_name: impl Into<String>, version: i32) -> Self {
        Self {
            class_name: class_name.into(),
            version,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatchValue {
    Bool(bool),
    Int(i64),
    Real(f32),
    String(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatchOperation {
    MemberAdd {
        name: String,
        type_name: String,
        ctype: Option<String>,
        default: Option<PatchValue>,
    },
    MemberRemove {
        name: String,
        type_name: String,
    },
    MemberRename {
        old_name: String,
        new_name: String,
    },
    ParentSet {
        old_parent: Option<String>,
        new_parent: Option<String>,
    },
    Depends {
        class_name: String,
        version: i32,
    },
    CustomHook {
        name: String,
        inverse_name: Option<String>,
    },
    /// Marker emitted when an op cannot be reconstructed losslessly.
    ///
    /// Produced by `inverse()` when reversing a forward op would require
    /// information the corpus does not preserve (e.g. the original default
    /// value of a member dropped by a forward MemberRemove). Applying this
    /// op fails loudly rather than silently zero-defaulting the result.
    Unsupported {
        reason: String,
    },
}

impl PatchOperation {
    pub fn apply_to_object(&self, obj: &mut HkxObject) -> HavokResult<()> {
        match self {
            Self::MemberAdd {
                name,
                type_name,
                default,
                ..
            } => obj.members.push(HkxMember {
                name: name.clone(),
                value: patch_default_value(type_name, default.as_ref()),
            }),
            Self::MemberRemove { name, .. } => obj.members.retain(|member| member.name != *name),
            Self::MemberRename { old_name, new_name } => {
                if let Some(member) = obj
                    .members
                    .iter_mut()
                    .find(|member| member.name == *old_name)
                {
                    member.name = new_name.clone();
                }
            }
            Self::ParentSet { .. } | Self::Depends { .. } => {}
            Self::CustomHook { name, .. } => {
                return Err(crate::error::HavokError::InvalidInput(format!(
                    "custom hook {name} requires whole-file conversion context"
                )));
            }
            Self::Unsupported { reason } => {
                return Err(crate::error::HavokError::InvalidInput(format!(
                    "patch operation cannot be applied losslessly: {reason}"
                )));
            }
        }
        Ok(())
    }

    pub fn inverse(&self) -> Option<Self> {
        match self {
            Self::MemberAdd {
                name, type_name, ..
            } => Some(Self::MemberRemove {
                name: name.clone(),
                type_name: type_name.clone(),
            }),
            // The inverse of MemberRemove is fundamentally lossy: the corpus
            // does not preserve the original member's `default` or `ctype`,
            // so a fabricated MemberAdd would silently zero-default a member
            // that may legitimately have had a non-zero default. Surface this
            // as Unsupported so the manager refuses the chain rather than
            // producing a structurally-valid-but-semantically-wrong file.
            Self::MemberRemove { name, type_name } => Some(Self::Unsupported {
                reason: format!(
                    "inverse of MemberRemove({name}: {type_name}) requires the original \
                     default+ctype which the corpus does not preserve",
                ),
            }),
            Self::MemberRename { old_name, new_name } => Some(Self::MemberRename {
                old_name: new_name.clone(),
                new_name: old_name.clone(),
            }),
            Self::ParentSet {
                old_parent,
                new_parent,
            } => Some(Self::ParentSet {
                old_parent: new_parent.clone(),
                new_parent: old_parent.clone(),
            }),
            Self::Depends {
                class_name,
                version,
            } => Some(Self::Depends {
                class_name: class_name.clone(),
                version: *version,
            }),
            Self::CustomHook { name, inverse_name } => match inverse_name.as_ref() {
                Some(inverse_name) => Some(Self::CustomHook {
                    name: inverse_name.clone(),
                    inverse_name: Some(name.clone()),
                }),
                None => Some(Self::Unsupported {
                    reason: format!("custom hook {name} has no registered inverse"),
                }),
            },
            Self::Unsupported { reason } => Some(Self::Unsupported {
                reason: format!("inverse of an Unsupported op: {reason}"),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Patch {
    pub old: ClassVersion,
    pub new: ClassVersion,
    pub operations: Vec<PatchOperation>,
    pub custom_hooks: Vec<String>,
}

impl Patch {
    pub fn new(old: ClassVersion, new: ClassVersion) -> Self {
        Self {
            old,
            new,
            operations: Vec::new(),
            custom_hooks: Vec::new(),
        }
    }

    pub fn with_operation(mut self, operation: PatchOperation) -> Self {
        if let PatchOperation::CustomHook { name, .. } = &operation {
            self.custom_hooks.push(name.clone());
        }
        self.operations.push(operation);
        self
    }

    pub fn with_custom_hook(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        self.operations.push(PatchOperation::CustomHook {
            name: name.clone(),
            inverse_name: None,
        });
        self.custom_hooks.push(name);
        self
    }

    pub fn with_reversible_custom_hook(
        mut self,
        name: impl Into<String>,
        inverse_name: impl Into<String>,
    ) -> Self {
        let name = name.into();
        let inverse_name = inverse_name.into();
        self.operations.push(PatchOperation::CustomHook {
            name: name.clone(),
            inverse_name: Some(inverse_name),
        });
        self.custom_hooks.push(name);
        self
    }

    pub fn matches_object(&self, obj: &HkxObject) -> bool {
        obj.class_name == self.old.class_name && obj.signature as i32 == self.old.version
    }

    pub fn apply_to_object(&self, obj: &mut HkxObject) -> HavokResult<()> {
        for operation in &self.operations {
            operation.apply_to_object(obj)?;
        }
        obj.class_name = self.new.class_name.clone();
        obj.signature = self.new.version.max(0) as u32;
        Ok(())
    }

    pub fn inverse(&self) -> Self {
        Self {
            old: self.new.clone(),
            new: self.old.clone(),
            operations: self
                .operations
                .iter()
                .rev()
                .filter_map(PatchOperation::inverse)
                .collect(),
            custom_hooks: self
                .operations
                .iter()
                .rev()
                .filter_map(|operation| match operation {
                    PatchOperation::CustomHook {
                        inverse_name: Some(name),
                        ..
                    } => Some(name.clone()),
                    _ => None,
                })
                .collect(),
        }
    }
}

fn patch_default_value(type_name: &str, default: Option<&PatchValue>) -> HkxValue {
    // Sized integer / unsigned families
    fn int_default(type_name: &str, value: i64) -> HkxValue {
        match type_name {
            "int8" => HkxValue::I8(value as i8),
            "int16" => HkxValue::I16(value as i16),
            "int64" => HkxValue::I64(value),
            "uint" | "uint32" => HkxValue::U32(value as u32),
            "uint8" => HkxValue::U8(value as u8),
            "uint16" => HkxValue::U16(value as u16),
            "uint64" => HkxValue::U64(value as u64),
            "half" => HkxValue::U16(value as u16),
            _ => HkxValue::I32(value as i32),
        }
    }

    match (type_name, default) {
        ("bool", Some(PatchValue::Bool(value))) => HkxValue::Bool(*value),
        ("bool", None) => HkxValue::Bool(false),
        ("real", Some(PatchValue::Real(value))) => HkxValue::F32(*value),
        ("real", None) => HkxValue::F32(0.0),
        ("string", Some(PatchValue::String(value))) => HkxValue::String {
            value: value.clone(),
            is_null: false,
        },
        ("string", None) => HkxValue::String {
            value: String::new(),
            is_null: false,
        },
        ("pointer", _) => HkxValue::Pointer(None),
        ("array", _) => HkxValue::Array(Vec::new()),
        // Vec/transform/quaternion families: zero-initialized F32 lists.
        // vec4 = 4 floats, quaternion = 4, vec12/qstransform = 12,
        // vec16/transform = 16, matrix3 = 9.
        ("vec4" | "quaternion", Some(PatchValue::Real(value))) => {
            HkxValue::F32List(vec![*value, *value, *value, *value])
        }
        ("vec4" | "quaternion", _) => HkxValue::F32List(vec![0.0; 4]),
        ("vec12" | "qstransform", _) => HkxValue::F32List(vec![0.0; 12]),
        ("vec16" | "transform" | "matrix4", _) => HkxValue::F32List(vec![0.0; 16]),
        ("matrix3", _) => HkxValue::F32List(vec![0.0; 9]),
        // Enum / struct fall through to int-style default when caller supplies
        // an int default; otherwise zero. Struct creates an empty inline object.
        ("struct", _) => HkxValue::Object(Vec::new()),
        ("enum", Some(PatchValue::Int(value))) => HkxValue::I32(*value as i32),
        ("enum", _) => HkxValue::I32(0),
        // Sized integer families.
        (
            "int" | "int8" | "int16" | "int64" | "uint" | "uint8" | "uint16" | "uint32" | "uint64"
            | "half",
            Some(PatchValue::Int(value)),
        ) => int_default(type_name, *value),
        (
            "int" | "int8" | "int16" | "int64" | "uint" | "uint8" | "uint16" | "uint32" | "uint64"
            | "half",
            Some(PatchValue::Bool(value)),
        ) => int_default(type_name, if *value { 1 } else { 0 }),
        (
            "int" | "int8" | "int16" | "int64" | "uint" | "uint8" | "uint16" | "uint32" | "uint64"
            | "half",
            None,
        ) => int_default(type_name, 0),
        // Generic fallbacks for unfamiliar type names with a default.
        (_, Some(PatchValue::Int(value))) => HkxValue::I32(*value as i32),
        (_, Some(PatchValue::Bool(value))) => HkxValue::Bool(*value),
        (_, Some(PatchValue::Real(value))) => HkxValue::F32(*value),
        (_, Some(PatchValue::String(value))) => HkxValue::String {
            value: value.clone(),
            is_null: false,
        },
        _ => HkxValue::I32(0),
    }
}
