use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::hkx::types::HkxType;

/// Capability tag distinguishing authoring-side from runtime classes.
///
/// `Setup` classes are used during the cloth/animation authoring pipeline and
/// produce `Runtime` output; they are not present at game runtime. `Internal`
/// covers helper structs (math, utility) that appear only as inline members.
/// `Runtime` is the default for all classes not otherwise categorized.
///
/// The heuristic: classes whose names contain "Setup" are tagged `Setup`;
/// all others default to `Runtime`. Override via `DescriptorRegistry::insert`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClassKind {
    #[default]
    Runtime,
    Setup,
    Internal,
}

/// First-class enum/flags descriptor: ordered named values with i64 storage
/// so bitmask flags (> 2^31) don't truncate.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct EnumDef {
    pub values: Vec<(String, i64)>,
}

impl EnumDef {
    pub fn value_to_name(&self, int_value: i64) -> Option<&str> {
        self.values
            .iter()
            .find_map(|(name, v)| (*v == int_value).then(|| name.as_str()))
    }

    pub fn name_to_value(&self, name: &str) -> Option<i64> {
        self.values
            .iter()
            .find_map(|(n, v)| (n == name).then_some(*v))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DescriptorError {
    #[error("classxml directory does not exist: {0}")]
    MissingClassxmlDir(PathBuf),

    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("invalid _index.json in {path}: {source}")]
    InvalidIndex {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[error("unsafe classxml index path for {class_name}: {filename}")]
    UnsafeIndexPath {
        class_name: String,
        filename: String,
    },

    #[error("invalid classxml XML in {path}: {source}")]
    Xml {
        path: PathBuf,
        source: roxmltree::Error,
    },

    #[error("malformed classxml in {path}: {message}")]
    Malformed { path: PathBuf, message: String },

    #[error("inheritance cycle while resolving {class_name}")]
    InheritanceCycle { class_name: String },

    #[error("unknown contents version prefix — no classxml mapping for {0}")]
    UnknownVersion(String),
}

pub type DescriptorResult<T> = Result<T, DescriptorError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StructureLayout {
    #[default]
    Msvc,
    Generic,
}

fn align_to(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemberTemplate {
    pub name: String,
    pub offset: usize,
    pub vtype: HkxType,
    pub vsubtype: HkxType,
    pub ctype: String,
    pub arrsize: usize,
    pub flags: String,
    pub etype: String,
    /// Optional default value for this member. When set, the tagxml writer
    /// skips this member if its value equals the default; the tagxml reader
    /// fills in the default when the member is absent in the source XML.
    /// Not parsed from classxml (no SDK equivalent in the XML format) — set
    /// programmatically by code that knows class-specific defaults.
    pub default: Option<crate::hkx::types::HkxValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassDescriptor {
    pub name: String,
    pub version: u32,
    pub signature: String,
    pub parent: Option<String>,
    pub is_struct: bool,
    pub members: Vec<MemberTemplate>,
    pub enums: HashMap<String, EnumDef>,
    /// Capability tag: authoring-side Setup vs runtime vs internal.
    /// Not derived from classxml; inferred from class name heuristic or
    /// set explicitly via `DescriptorRegistry::insert`.
    pub kind: ClassKind,
}

#[derive(Debug, Clone)]
pub struct DescriptorRegistry {
    dir: PathBuf,
    index: HashMap<String, String>,
    cache: HashMap<String, Option<ClassDescriptor>>,
    structure_layout: StructureLayout,
    layout_cache: HashMap<String, Vec<MemberTemplate>>,
}

impl Default for DescriptorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DescriptorRegistry {
    pub fn new() -> Self {
        let mut registry =
            Self::from_dir(default_classxml_dir()).expect("bundled classxml registry is valid");
        registry.install_fo4_ragdoll_descriptors();
        registry
    }

    pub fn for_version(version: &str) -> DescriptorResult<Self> {
        let versioned = resource_root().join(format!("classxml_{version}"));
        if versioned.exists() {
            Self::from_dir(versioned)
        } else {
            Err(DescriptorError::MissingClassxmlDir(versioned))
        }
    }

    /// Map a contents version string (e.g. `hk_2014.1.0-r1`) to the
    /// appropriate classxml variant and load it.
    ///
    /// Falls back to the default FO4 classxml for any recognised `hk_20xx`
    /// prefix when the versioned classxml directory does not exist, but logs
    /// a warning for completely unknown prefixes (e.g. `hk_2009`, `hk_2019+`)
    /// so callers are not silently using wrong member offsets.
    pub fn for_contents_version(contents_version: &str) -> Self {
        let suffix = if contents_version.starts_with("hk_2010")
            || contents_version.starts_with("hk_2012")
            || contents_version.starts_with("hk_2013")
        {
            Some("2012")
        } else if contents_version.starts_with("hk_2015")
            || contents_version.starts_with("hk_2016")
            || contents_version.starts_with("hk_2017")
            || contents_version.starts_with("hk_2018")
        {
            Some("2015")
        } else if contents_version.starts_with("hk_2014") || contents_version.is_empty() {
            // Default: FO4 (hk_2014) uses Self::new() directly.
            None
        } else {
            // Completely unknown version — warn so the caller knows member
            // offsets may be wrong. DescriptorError::UnknownVersion is the
            // typed form for callers that can propagate it.
            eprintln!(
                "havok: for_contents_version: no classxml mapping for {:?}; \
                 falling back to FO4 descriptors — member offsets may be incorrect",
                contents_version
            );
            None
        };
        suffix
            .and_then(|version| Self::for_version(version).ok())
            .unwrap_or_else(Self::new)
    }

    pub fn from_dir(path: impl AsRef<Path>) -> DescriptorResult<Self> {
        let dir = path.as_ref().to_path_buf();
        if !dir.exists() {
            return Err(DescriptorError::MissingClassxmlDir(dir));
        }
        let index = load_index(&dir)?;
        Ok(Self {
            dir,
            index,
            cache: HashMap::new(),
            structure_layout: StructureLayout::Msvc,
            layout_cache: HashMap::new(),
        })
    }

    pub fn set_structure_layout(&mut self, structure_layout: StructureLayout) {
        if self.structure_layout != structure_layout {
            self.structure_layout = structure_layout;
            self.layout_cache.clear();
        }
    }

    pub fn get(&mut self, class_name: &str) -> DescriptorResult<Option<&ClassDescriptor>> {
        if !self.cache.contains_key(class_name) {
            let desc = self.load_descriptor(class_name)?;
            self.cache.insert(class_name.to_string(), desc);
        }
        Ok(self.cache.get(class_name).and_then(Option::as_ref))
    }

    /// Insert or replace a descriptor in the registry cache. Used by tests
    /// and code generators that need to set member defaults programmatically.
    pub fn insert(&mut self, descriptor: ClassDescriptor) {
        self.cache.insert(descriptor.name.clone(), Some(descriptor));
    }

    /// Return the `ClassKind` for a class. Falls back to the name heuristic
    /// when the class is not in the registry (e.g. no classxml file exists).
    pub fn class_kind(&mut self, class_name: &str) -> ClassKind {
        if let Ok(Some(desc)) = self.get(class_name) {
            return desc.kind;
        }
        // Name-based fallback: used for classes without classxml (e.g. cloth Setup classes).
        if class_name.contains("Setup") {
            ClassKind::Setup
        } else {
            ClassKind::Runtime
        }
    }

    /// Convenience: returns true when the class is a build-time Setup class.
    pub fn is_setup(&mut self, class_name: &str) -> bool {
        self.class_kind(class_name) == ClassKind::Setup
    }

    pub fn get_all_members(&mut self, class_name: &str) -> DescriptorResult<Vec<MemberTemplate>> {
        if self.structure_layout == StructureLayout::Generic {
            return self.get_generic_members(class_name);
        }
        self.get_msvc_members(class_name)
    }

    fn get_msvc_members(&mut self, class_name: &str) -> DescriptorResult<Vec<MemberTemplate>> {
        let mut chain = Vec::new();
        let mut current = Some(class_name.to_string());
        let mut seen_classes = HashSet::new();

        while let Some(name) = current {
            if !seen_classes.insert(name.clone()) {
                return Err(DescriptorError::InheritanceCycle {
                    class_name: class_name.to_string(),
                });
            }
            let Some(desc) = self.get(&name)? else {
                break;
            };
            chain.push(desc.clone());
            current = desc.parent.clone().filter(|parent| !parent.is_empty());
        }

        let mut members = Vec::new();
        let mut seen_names = Vec::<String>::new();
        for desc in chain.iter().rev() {
            for member in &desc.members {
                if !seen_names.iter().any(|name| name == &member.name) {
                    seen_names.push(member.name.clone());
                    members.push(member.clone());
                }
            }
        }
        members.sort_by_key(|member| member.offset);
        Ok(members)
    }

    fn get_generic_members(&mut self, class_name: &str) -> DescriptorResult<Vec<MemberTemplate>> {
        if let Some(members) = self.layout_cache.get(class_name) {
            return Ok(members.clone());
        }

        let mut chain = Vec::new();
        let mut current_class = Some(class_name.to_string());
        let mut seen_classes = HashSet::new();
        while let Some(name) = current_class {
            if !seen_classes.insert(name.clone()) {
                return Err(DescriptorError::InheritanceCycle {
                    class_name: class_name.to_string(),
                });
            }
            let Some(desc) = self.get(&name)?.cloned() else {
                break;
            };
            current_class = desc.parent.clone().filter(|parent| !parent.is_empty());
            chain.push(desc);
        }
        chain.reverse();

        let first_member_offset = chain
            .iter()
            .flat_map(|descriptor| descriptor.members.iter())
            .map(|member| member.offset)
            .next()
            .unwrap_or(0);
        let mut current_offset = first_member_offset;
        let mut members = Vec::new();
        let mut seen_names = Vec::<String>::new();

        for descriptor in chain {
            let mut previous_msvc_end = None;
            for member in descriptor.members {
                if seen_names.iter().any(|name| name == &member.name) {
                    continue;
                }

                let alignment = self.member_alignment(&member)?.max(1);
                current_offset = align_to(current_offset, alignment);

                if let Some(previous_end) = previous_msvc_end {
                    let expected_msvc_offset = align_to(previous_end, alignment);
                    current_offset += member.offset.saturating_sub(expected_msvc_offset);
                }

                let span = self.member_span_for_layout(&member)?;
                previous_msvc_end = Some(member.offset + span);

                let mut positioned = member;
                positioned.offset = current_offset;
                current_offset += span;
                seen_names.push(positioned.name.clone());
                members.push(positioned);
            }
        }

        members.sort_by_key(|member| member.offset);
        self.layout_cache
            .insert(class_name.to_string(), members.clone());
        Ok(members)
    }

    fn member_alignment(&mut self, member: &MemberTemplate) -> DescriptorResult<usize> {
        let inferred = match member.vtype.family() {
            crate::hkx::types::HkxTypeFamily::Complex => 16,
            crate::hkx::types::HkxTypeFamily::Object if !member.ctype.is_empty() => self
                .get_generic_members(&member.ctype)?
                .iter()
                .map(|nested| self.member_alignment(nested))
                .collect::<DescriptorResult<Vec<_>>>()?
                .into_iter()
                .max()
                .unwrap_or(1),
            crate::hkx::types::HkxTypeFamily::Pointer
            | crate::hkx::types::HkxTypeFamily::String => 8,
            crate::hkx::types::HkxTypeFamily::Array => member.vtype.size().min(8).max(1),
            crate::hkx::types::HkxTypeFamily::Enum => member.vsubtype.size().max(1),
            _ => member.vtype.size().min(8).max(1),
        };
        Ok(if member.flags.contains("ALIGN_16") {
            inferred.max(16)
        } else if member.flags.contains("ALIGN_8") {
            inferred.max(8)
        } else {
            inferred
        })
    }

    fn member_span_for_layout(&mut self, member: &MemberTemplate) -> DescriptorResult<usize> {
        let element_span = if member.vtype == HkxType::Struct && !member.ctype.is_empty() {
            let nested = self.get_generic_members(&member.ctype)?;
            let alignment = nested
                .iter()
                .map(|nested_member| self.member_alignment(nested_member))
                .collect::<DescriptorResult<Vec<_>>>()?
                .into_iter()
                .max()
                .unwrap_or(1);
            let end = nested
                .last()
                .map(|nested_member| {
                    self.member_span_for_layout(nested_member)
                        .map(|span| nested_member.offset + span)
                })
                .transpose()?
                .unwrap_or(0);
            align_to(end, alignment)
        } else if member.vtype.family() == crate::hkx::types::HkxTypeFamily::Enum {
            member.vsubtype.size().max(1)
        } else {
            member.vtype.size().max(1)
        };
        Ok(element_span * member.arrsize.max(1))
    }

    pub fn get_enum_value(&mut self, class_name: &str, enum_name: &str, int_value: i32) -> String {
        let mut current = Some(class_name.to_string());
        let mut seen_classes = HashSet::new();
        while let Some(name) = current {
            if !seen_classes.insert(name.clone()) {
                break;
            }
            let Ok(Some(desc)) = self.get(&name) else {
                break;
            };
            if let Some(enum_def) = desc.enums.get(enum_name) {
                return enum_def
                    .value_to_name(int_value as i64)
                    .map(str::to_string)
                    .unwrap_or_else(|| int_value.to_string());
            }
            current = desc.parent.clone().filter(|parent| !parent.is_empty());
        }
        int_value.to_string()
    }

    pub fn get_enum_int(&mut self, class_name: &str, enum_name: &str, str_value: &str) -> i32 {
        if let Ok(value) = str_value.parse::<i32>() {
            return value;
        }

        let mut current = Some(class_name.to_string());
        let mut seen_classes = HashSet::new();
        while let Some(name) = current {
            if !seen_classes.insert(name.clone()) {
                break;
            }
            let Ok(Some(desc)) = self.get(&name) else {
                break;
            };
            if let Some(enum_def) = desc.enums.get(enum_name) {
                return enum_def.name_to_value(str_value).unwrap_or(0) as i32;
            }
            current = desc.parent.clone().filter(|parent| !parent.is_empty());
        }
        0
    }

    /// Resolve an enum symbol declared outside the class inheritance chain.
    ///
    /// Havok classxml describes members such as
    /// `hkbBlendingTransitionEffect::blendCurve` with `etype="BlendCurve"`,
    /// while the enum itself belongs to the utility struct
    /// `hkbBlendCurveUtils`. The classxml does not carry that owner link, so
    /// TagXML readers must resolve the enum by its globally unique type and
    /// item names. Conflicting declarations remain unresolved rather than
    /// choosing one arbitrarily.
    pub(crate) fn get_external_enum_int(
        &mut self,
        enum_name: &str,
        str_value: &str,
    ) -> Option<i32> {
        let class_names: Vec<String> = self.index.keys().cloned().collect();
        let mut resolved = None;
        for class_name in class_names {
            let candidate = self
                .get(&class_name)
                .ok()
                .flatten()
                .and_then(|descriptor| descriptor.enums.get(enum_name))
                .and_then(|enum_def| enum_def.name_to_value(str_value));
            let Some(candidate) = candidate else {
                continue;
            };
            if resolved.is_some_and(|value| value != candidate) {
                return None;
            }
            resolved = Some(candidate);
        }
        resolved.and_then(|value| i32::try_from(value).ok())
    }

    /// Inverse of [`Self::get_external_enum_int`] for TagXML emission.
    /// Conflicting symbolic names for the same enum value are treated as
    /// ambiguous and left numeric by the caller.
    pub(crate) fn get_external_enum_name(
        &mut self,
        enum_name: &str,
        int_value: i32,
    ) -> Option<String> {
        let class_names: Vec<String> = self.index.keys().cloned().collect();
        let mut resolved: Option<String> = None;
        for class_name in class_names {
            let candidate = self
                .get(&class_name)
                .ok()
                .flatten()
                .and_then(|descriptor| descriptor.enums.get(enum_name))
                .and_then(|enum_def| enum_def.value_to_name(int_value as i64))
                .map(str::to_string);
            let Some(candidate) = candidate else {
                continue;
            };
            if resolved.as_ref().is_some_and(|name| name != &candidate) {
                return None;
            }
            resolved = Some(candidate);
        }
        resolved
    }

    fn install_fo4_ragdoll_descriptors(&mut self) {
        for (name, xml) in FO4_RAGDOLL_CLASSXML {
            let descriptor = parse_classxml(Path::new(name), xml)
                .expect("built-in FO4 ragdoll classxml is valid");
            self.insert(descriptor);
        }
    }

    fn load_descriptor(&self, class_name: &str) -> DescriptorResult<Option<ClassDescriptor>> {
        let Some(filename) = self.index.get(class_name) else {
            return Ok(None);
        };
        let path = self.dir.join(filename);
        let xml = fs::read_to_string(&path).map_err(|source| DescriptorError::Read {
            path: path.clone(),
            source,
        })?;
        Ok(Some(parse_classxml(&path, &xml)?))
    }
}

// The bundled classxml corpus omits the legacy Physics 2012 bridge classes
// that Havok Content Tools 2014 emits for FO4 ragdolls. These layouts are the
// exact 64-bit hk_2014 descriptors whose signatures appear in the tutorial
// fixture; keeping them scoped to DescriptorRegistry::new avoids applying
// FO4 offsets to the versioned 2012/2015 registries.
const FO4_RAGDOLL_CLASSXML: &[(&str, &str)] = &[
    (
        "hkaRagdollInstance.xml",
        r#"<class name='hkaRagdollInstance' version='1' signature='0x5448f464' parent='hkReferencedObject'><members>
<member name='rigidBodies' offset='16' ctype='hkpRigidBody' vtype='TYPE_ARRAY' vsubtype='TYPE_POINTER'/>
<member name='constraints' offset='32' ctype='hkpConstraintInstance' vtype='TYPE_ARRAY' vsubtype='TYPE_POINTER'/>
<member name='boneToRigidBodyMap' offset='48' vtype='TYPE_ARRAY' vsubtype='TYPE_INT32'/>
<member name='skeleton' offset='64' ctype='hkaSkeleton' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT'/>
</members></class>"#,
    ),
    (
        "hkpConstraintInstance.xml",
        r#"<class name='hkpConstraintInstance' version='1' signature='0xda4ce91e' parent='hkReferencedObject'><enums>
<enum name='ConstraintPriority'><enumitem name='PRIORITY_INVALID' value='0'/><enumitem name='PRIORITY_PSI' value='1'/><enumitem name='PRIORITY_SIMPLIFIED_TOI_UNUSED' value='2'/><enumitem name='PRIORITY_TOI' value='3'/><enumitem name='PRIORITY_TOI_HIGHER' value='4'/><enumitem name='PRIORITY_TOI_FORCED' value='5'/><enumitem name='NUM_PRIORITIES' value='6'/></enum>
<enum name='OnDestructionRemapInfo'><enumitem name='ON_DESTRUCTION_REMAP' value='0'/><enumitem name='ON_DESTRUCTION_REMOVE' value='1'/><enumitem name='ON_DESTRUCTION_RESET_REMOVE' value='2'/></enum>
</enums><members>
<member name='owner' offset='16' vtype='TYPE_POINTER' flags='SERIALIZE_IGNORED'/>
<member name='data' offset='24' ctype='hkpConstraintData' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT'/>
<member name='constraintModifiers' offset='32' ctype='hkpModifierConstraintAtom' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT'/>
<member name='entities' offset='40' ctype='hkpEntity' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT' arrsize='2'/>
<member name='priority' offset='56' etype='ConstraintPriority' vtype='TYPE_ENUM' vsubtype='TYPE_UINT8'/>
<member name='wantRuntime' offset='57' vtype='TYPE_BOOL'/>
<member name='destructionRemapInfo' offset='58' etype='OnDestructionRemapInfo' vtype='TYPE_ENUM' vsubtype='TYPE_UINT8'/>
<member name='listeners' offset='64' ctype='hkpConstraintInstanceSmallArraySerializeOverrideType' vtype='TYPE_STRUCT' flags='SERIALIZE_IGNORED'/>
<member name='name' offset='80' vtype='TYPE_STRINGPTR'/>
<member name='userData' offset='88' vtype='TYPE_ULONG'/>
<member name='internal' offset='96' vtype='TYPE_POINTER' flags='SERIALIZE_IGNORED'/>
<member name='uid' offset='104' vtype='TYPE_UINT32' flags='SERIALIZE_IGNORED'/>
</members></class>"#,
    ),
    (
        "hkpPhysicsData.xml",
        r#"<class name='hkpPhysicsData' version='1' signature='0x47a8ca83' parent='hkReferencedObject'><members>
<member name='worldCinfo' offset='16' ctype='hkpWorldCinfo' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT'/>
<member name='systems' offset='24' ctype='hkpPhysicsSystem' vtype='TYPE_ARRAY' vsubtype='TYPE_POINTER'/>
</members></class>"#,
    ),
    (
        "hkpPhysicsSystem.xml",
        r#"<class name='hkpPhysicsSystem' version='1' signature='0xb3cc6e64' parent='hkReferencedObject'><members>
<member name='rigidBodies' offset='16' ctype='hkpRigidBody' vtype='TYPE_ARRAY' vsubtype='TYPE_POINTER'/>
<member name='constraints' offset='32' ctype='hkpConstraintInstance' vtype='TYPE_ARRAY' vsubtype='TYPE_POINTER'/>
<member name='actions' offset='48' ctype='hkpAction' vtype='TYPE_ARRAY' vsubtype='TYPE_POINTER'/>
<member name='phantoms' offset='64' ctype='hkpPhantom' vtype='TYPE_ARRAY' vsubtype='TYPE_POINTER'/>
<member name='name' offset='80' vtype='TYPE_STRINGPTR'/>
<member name='userData' offset='88' vtype='TYPE_ULONG'/>
<member name='active' offset='96' vtype='TYPE_BOOL'/>
</members></class>"#,
    ),
    (
        "hkpRigidBody.xml",
        r#"<class name='hkpRigidBody' version='1' signature='0xcd2e69e5' parent='hkpEntity'><members/></class>"#,
    ),
    (
        "hkpWorldObject.xml",
        r#"<class name='hkpWorldObject' version='1' signature='0xeff021bd' parent='hkReferencedObject'><members>
<member name='world' offset='16' vtype='TYPE_POINTER' flags='SERIALIZE_IGNORED'/>
<member name='userData' offset='24' vtype='TYPE_ULONG'/>
<member name='collidable' offset='32' ctype='hkpLinkedCollidable' vtype='TYPE_STRUCT'/>
<member name='multiThreadCheck' offset='160' ctype='hkMultiThreadCheck' vtype='TYPE_STRUCT'/>
<member name='name' offset='176' vtype='TYPE_STRINGPTR'/>
<member name='properties' offset='184' ctype='hkSimpleProperty' vtype='TYPE_ARRAY' vsubtype='TYPE_STRUCT'/>
</members></class>"#,
    ),
    (
        "hkpEntity.xml",
        r#"<class name='hkpEntity' version='1' signature='0x88049864' parent='hkpWorldObject'><members>
<member name='material' offset='200' ctype='hkpMaterial' vtype='TYPE_STRUCT'/>
<member name='limitContactImpulseUtilAndFlag' offset='216' vtype='TYPE_POINTER' flags='SERIALIZE_IGNORED'/>
<member name='damageMultiplier' offset='224' vtype='TYPE_REAL'/>
<member name='breakableBody' offset='232' vtype='TYPE_POINTER' flags='SERIALIZE_IGNORED'/>
<member name='solverData' offset='240' vtype='TYPE_UINT32' flags='SERIALIZE_IGNORED'/>
<member name='storageIndex' offset='244' vtype='TYPE_UINT16'/>
<member name='contactPointCallbackDelay' offset='246' vtype='TYPE_UINT16'/>
<member name='constraintsMaster' offset='248' ctype='hkpEntitySmallArraySerializeOverrideType' vtype='TYPE_STRUCT' flags='SERIALIZE_IGNORED'/>
<member name='constraintsSlave' offset='264' ctype='hkpConstraintInstance' vtype='TYPE_ARRAY' vsubtype='TYPE_POINTER' flags='SERIALIZE_IGNORED'/>
<member name='constraintRuntime' offset='280' vtype='TYPE_ARRAY' vsubtype='TYPE_UINT8' flags='SERIALIZE_IGNORED'/>
<member name='simulationIsland' offset='296' vtype='TYPE_POINTER' flags='SERIALIZE_IGNORED'/>
<member name='autoRemoveLevel' offset='304' vtype='TYPE_INT8'/>
<member name='numShapeKeysInContactPointProperties' offset='305' vtype='TYPE_UINT8'/>
<member name='responseModifierFlags' offset='306' vtype='TYPE_UINT8'/>
<member name='uid' offset='308' vtype='TYPE_UINT32'/>
<member name='spuCollisionCallback' offset='312' ctype='hkpEntitySpuCollisionCallback' vtype='TYPE_STRUCT'/>
<member name='motion' offset='336' ctype='hkpMaxSizeMotion' vtype='TYPE_STRUCT'/>
<member name='contactListeners' offset='656' ctype='hkpEntitySmallArraySerializeOverrideType' vtype='TYPE_STRUCT' flags='SERIALIZE_IGNORED'/>
<member name='actions' offset='672' ctype='hkpEntitySmallArraySerializeOverrideType' vtype='TYPE_STRUCT' flags='SERIALIZE_IGNORED'/>
<member name='localFrame' offset='688' ctype='hkLocalFrame' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT'/>
<member name='extendedListeners' offset='696' ctype='hkpEntityExtendedListeners' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT' flags='SERIALIZE_IGNORED'/>
<member name='npData' offset='704' vtype='TYPE_UINT32'/>
</members></class>"#,
    ),
    (
        "hkpCdBody.xml",
        r#"<class name='hkpCdBody' version='1' signature='0x54a4b841'><members>
<member name='shape' offset='0' ctype='hkpShape' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT'/>
<member name='shapeKey' offset='8' vtype='TYPE_UINT32'/>
<member name='motion' offset='16' vtype='TYPE_POINTER' flags='SERIALIZE_IGNORED'/>
<member name='parent' offset='24' ctype='hkpCdBody' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT' flags='SERIALIZE_IGNORED'/>
</members></class>"#,
    ),
    (
        "hkpCollidable.xml",
        r#"<class name='hkpCollidable' version='1' signature='0x2eaeea47' parent='hkpCdBody'><members>
<member name='ownerOffset' offset='32' vtype='TYPE_INT8' flags='SERIALIZE_IGNORED'/>
<member name='forceCollideOntoPpu' offset='33' vtype='TYPE_UINT8'/>
<member name='shapeSizeOnSpu' offset='34' vtype='TYPE_UINT16' flags='SERIALIZE_IGNORED'/>
<member name='broadPhaseHandle' offset='36' ctype='hkpTypedBroadPhaseHandle' vtype='TYPE_STRUCT'/>
<member name='boundingVolumeData' offset='48' ctype='hkpCollidableBoundingVolumeData' vtype='TYPE_STRUCT' flags='SERIALIZE_IGNORED'/>
<member name='allowedPenetrationDepth' offset='104' vtype='TYPE_REAL'/>
</members></class>"#,
    ),
    (
        "hkpLinkedCollidable.xml",
        r#"<class name='hkpLinkedCollidable' version='1' signature='0x5508bc75' parent='hkpCollidable'><members>
<member name='collisionEntries' offset='112' vtype='TYPE_ARRAY' flags='SERIALIZE_IGNORED'/>
</members></class>"#,
    ),
    (
        "hkpBroadPhaseHandle.xml",
        r#"<class name='hkpBroadPhaseHandle' version='1' signature='0x940569dc'><members>
<member name='id' offset='0' vtype='TYPE_UINT32' flags='SERIALIZE_IGNORED'/>
</members></class>"#,
    ),
    (
        "hkpTypedBroadPhaseHandle.xml",
        r#"<class name='hkpTypedBroadPhaseHandle' version='1' signature='0xf4b0f799' parent='hkpBroadPhaseHandle'><members>
<member name='type' offset='4' vtype='TYPE_INT8'/>
<member name='ownerOffset' offset='5' vtype='TYPE_INT8' flags='SERIALIZE_IGNORED'/>
<member name='objectQualityType' offset='6' vtype='TYPE_INT8'/>
<member name='collisionFilterInfo' offset='8' vtype='TYPE_UINT32'/>
</members></class>"#,
    ),
    (
        "hkpMaterial.xml",
        r#"<class name='hkpMaterial' version='1' signature='0x33be6570'><enums>
<enum name='ResponseType'><enumitem name='RESPONSE_INVALID' value='0'/><enumitem name='RESPONSE_SIMPLE_CONTACT' value='1'/><enumitem name='RESPONSE_REPORTING' value='2'/><enumitem name='RESPONSE_NONE' value='3'/><enumitem name='RESPONSE_MAX_ID' value='4'/></enum>
</enums><members>
<member name='responseType' offset='0' etype='ResponseType' vtype='TYPE_ENUM' vsubtype='TYPE_INT8'/>
<member name='rollingFrictionMultiplier' offset='2' vtype='TYPE_HALF'/>
<member name='friction' offset='4' vtype='TYPE_REAL'/>
<member name='restitution' offset='8' vtype='TYPE_REAL'/>
</members></class>"#,
    ),
    (
        "hkpEntitySpuCollisionCallback.xml",
        r#"<class name='hkpEntitySpuCollisionCallback' version='1' signature='0x81147f05'><members>
<member name='util' offset='0' vtype='TYPE_POINTER' flags='SERIALIZE_IGNORED'/>
<member name='capacity' offset='8' vtype='TYPE_UINT16' flags='SERIALIZE_IGNORED'/>
<member name='eventFilter' offset='10' vtype='TYPE_UINT8'/>
<member name='userFilter' offset='11' vtype='TYPE_UINT8'/>
</members></class>"#,
    ),
    (
        "hkpMotion.xml",
        r#"<class name='hkpMotion' version='1' signature='0x867134af' parent='hkReferencedObject'><enums>
<enum name='MotionType'><enumitem name='MOTION_INVALID' value='0'/><enumitem name='MOTION_DYNAMIC' value='1'/><enumitem name='MOTION_SPHERE_INERTIA' value='2'/><enumitem name='MOTION_BOX_INERTIA' value='3'/><enumitem name='MOTION_KEYFRAMED' value='4'/><enumitem name='MOTION_FIXED' value='5'/><enumitem name='MOTION_THIN_BOX_INERTIA' value='6'/><enumitem name='MOTION_CHARACTER' value='7'/><enumitem name='MOTION_MAX_ID' value='8'/></enum>
</enums><members>
<member name='type' offset='16' etype='MotionType' vtype='TYPE_ENUM' vsubtype='TYPE_UINT8'/>
<member name='deactivationIntegrateCounter' offset='17' vtype='TYPE_UINT8'/>
<member name='deactivationNumInactiveFrames' offset='18' vtype='TYPE_UINT16' arrsize='2'/>
<member name='motionState' offset='32' ctype='hkMotionState' vtype='TYPE_STRUCT'/>
<member name='inertiaAndMassInv' offset='208' vtype='TYPE_VECTOR4'/>
<member name='linearVelocity' offset='224' vtype='TYPE_VECTOR4'/>
<member name='angularVelocity' offset='240' vtype='TYPE_VECTOR4'/>
<member name='deactivationRefPosition' offset='256' vtype='TYPE_VECTOR4' arrsize='2'/>
<member name='deactivationRefOrientation' offset='288' vtype='TYPE_UINT32' arrsize='2'/>
<member name='savedMotion' offset='296' ctype='hkpMaxSizeMotion' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT'/>
<member name='savedQualityTypeIndex' offset='304' vtype='TYPE_UINT16'/>
<member name='gravityFactor' offset='306' vtype='TYPE_HALF'/>
</members></class>"#,
    ),
    (
        "hkpKeyframedRigidMotion.xml",
        r#"<class name='hkpKeyframedRigidMotion' version='1' signature='0x98bf2cff' parent='hkpMotion'><members/></class>"#,
    ),
    (
        "hkpMaxSizeMotion.xml",
        r#"<class name='hkpMaxSizeMotion' version='1' signature='0xb285f48c' parent='hkpKeyframedRigidMotion'><members/></class>"#,
    ),
    (
        "hkpShapeInfo.xml",
        r#"<class name='hkpShapeInfo' version='1' signature='0xd3fdd2e8' parent='hkReferencedObject'><members>
<member name='shape' offset='16' ctype='hkpShape' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT'/>
<member name='isHierarchicalCompound' offset='24' vtype='TYPE_BOOL'/>
<member name='hkdShapesCollected' offset='25' vtype='TYPE_BOOL'/>
<member name='childShapeNames' offset='32' vtype='TYPE_ARRAY' vsubtype='TYPE_STRINGPTR'/>
<member name='childTransforms' offset='48' vtype='TYPE_ARRAY' vsubtype='TYPE_TRANSFORM'/>
<member name='transform' offset='64' vtype='TYPE_TRANSFORM'/>
</members></class>"#,
    ),
    (
        "hkpShapeBase.xml",
        r#"<class name='hkpShapeBase' version='1' signature='0x16c1941e' parent='hkcdShape'><members/></class>"#,
    ),
    (
        "hkpShape.xml",
        r#"<class name='hkpShape' version='1' signature='0xdee9ace0' parent='hkpShapeBase'><members>
<member name='userData' offset='24' vtype='TYPE_ULONG'/>
</members></class>"#,
    ),
    (
        "hkpSphereRepShape.xml",
        r#"<class name='hkpSphereRepShape' version='1' signature='0xd99d7463' parent='hkpShape'><members/></class>"#,
    ),
    (
        "hkpConvexShape.xml",
        r#"<class name='hkpConvexShape' version='1' signature='0xf54e3c3a' parent='hkpSphereRepShape'><members>
<member name='radius' offset='32' vtype='TYPE_REAL'/>
</members></class>"#,
    ),
    (
        "hkpConvexVerticesShape.xml",
        r#"<class name='hkpConvexVerticesShape' version='1' signature='0xc21c8b5a' parent='hkpConvexShape'><members>
<member name='aabbHalfExtents' offset='48' vtype='TYPE_VECTOR4'/>
<member name='aabbCenter' offset='64' vtype='TYPE_VECTOR4'/>
<member name='rotatedVertices' offset='80' vtype='TYPE_ARRAY' vsubtype='TYPE_MATRIX3'/>
<member name='numVertices' offset='96' vtype='TYPE_INT32'/>
<member name='useSpuBuffer' offset='100' vtype='TYPE_BOOL' flags='SERIALIZE_IGNORED'/>
<member name='planeEquations' offset='104' vtype='TYPE_ARRAY' vsubtype='TYPE_VECTOR4'/>
<member name='connectivity' offset='120' ctype='hkpConvexVerticesConnectivity' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT'/>
</members></class>"#,
    ),
    (
        "hkpShapeCollection.xml",
        r#"<class name='hkpShapeCollection' version='1' signature='0x093c7f0b' parent='hkpShape'><enums>
<enum name='CollectionType'><enumitem name='COLLECTION_LIST' value='0'/><enumitem name='COLLECTION_EXTENDED_MESH' value='1'/><enumitem name='COLLECTION_TRISAMPLED_HEIGHTFIELD' value='2'/><enumitem name='COLLECTION_USER' value='3'/><enumitem name='COLLECTION_SIMPLE_MESH' value='4'/><enumitem name='COLLECTION_MESH_SHAPE' value='5'/><enumitem name='COLLECTION_COMPRESSED_MESH' value='6'/><enumitem name='COLLECTION_MAX' value='7'/></enum>
</enums><members>
<member name='disableWelding' offset='40' vtype='TYPE_BOOL'/>
<member name='collectionType' offset='41' etype='CollectionType' vtype='TYPE_ENUM' vsubtype='TYPE_UINT8'/>
</members></class>"#,
    ),
    (
        "hkpListShape.xml",
        r#"<class name='hkpListShape' version='1' signature='0xf2ec3ed5' parent='hkpShapeCollection'><members>
<member name='childInfo' offset='48' ctype='hkpListShapeChildInfo' vtype='TYPE_ARRAY' vsubtype='TYPE_STRUCT'/>
<member name='flags' offset='64' vtype='TYPE_UINT16'/>
<member name='numDisabledChildren' offset='66' vtype='TYPE_UINT16'/>
<member name='aabbHalfExtents' offset='80' vtype='TYPE_VECTOR4'/>
<member name='aabbCenter' offset='96' vtype='TYPE_VECTOR4'/>
<member name='enabledChildren' offset='112' vtype='TYPE_UINT32' arrsize='8'/>
</members></class>"#,
    ),
    (
        "hkpListShapeChildInfo.xml",
        r#"<class name='hkpListShapeChildInfo' version='1' signature='0x21940c23'><members>
<member name='shape' offset='0' ctype='hkpShape' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT'/>
<member name='collisionFilterInfo' offset='8' vtype='TYPE_UINT32'/>
<member name='shapeInfo' offset='12' vtype='TYPE_UINT16'/>
<member name='shapeSize' offset='14' vtype='TYPE_INT16' flags='SERIALIZE_IGNORED'/>
<member name='numChildShapes' offset='16' vtype='TYPE_INT32' flags='SERIALIZE_IGNORED'/>
</members></class>"#,
    ),
    (
        "hkpBvTreeShape.xml",
        r#"<class name='hkpBvTreeShape' version='1' signature='0x13f65b68' parent='hkpShape'><enums>
<enum name='BvTreeType'><enumitem name='BVTREE_MOPP' value='0'/><enumitem name='BVTREE_TRISAMPLED_HEIGHTFIELD' value='1'/><enumitem name='BVTREE_STATIC_COMPOUND' value='2'/><enumitem name='BVTREE_COMPRESSED_MESH' value='3'/><enumitem name='BVTREE_USER' value='4'/><enumitem name='BVTREE_MAX' value='5'/></enum>
</enums><members><member name='bvTreeType' offset='32' etype='BvTreeType' vtype='TYPE_ENUM' vsubtype='TYPE_UINT8'/></members></class>"#,
    ),
    (
        "hkMoppBvTreeShapeBase.xml",
        r#"<class name='hkMoppBvTreeShapeBase' version='1' signature='0x4f70a521' parent='hkpBvTreeShape'><members>
<member name='code' offset='40' ctype='hkpMoppCode' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT'/>
<member name='moppData' offset='48' vtype='TYPE_POINTER' flags='SERIALIZE_IGNORED'/>
<member name='moppDataSize' offset='56' vtype='TYPE_UINT32' flags='SERIALIZE_IGNORED'/>
<member name='codeInfoCopy' offset='64' vtype='TYPE_VECTOR4' flags='SERIALIZE_IGNORED'/>
</members></class>"#,
    ),
    (
        "hkpShapeContainer.xml",
        r#"<class name='hkpShapeContainer' version='1' signature='0xe0708a00'><members/></class>"#,
    ),
    (
        "hkpSingleShapeContainer.xml",
        r#"<class name='hkpSingleShapeContainer' version='1' signature='0x73aa1d38' parent='hkpShapeContainer'><members>
<member name='childShape' offset='8' ctype='hkpShape' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT'/>
</members></class>"#,
    ),
    (
        "hkpMoppBvTreeShape.xml",
        r#"<class name='hkpMoppBvTreeShape' version='1' signature='0x73f28bee' parent='hkMoppBvTreeShapeBase'><members>
<member name='child' offset='80' ctype='hkpSingleShapeContainer' vtype='TYPE_STRUCT'/>
<member name='childSize' offset='96' vtype='TYPE_INT32' flags='SERIALIZE_IGNORED'/>
</members></class>"#,
    ),
    (
        "hkpMoppCodeCodeInfo.xml",
        r#"<class name='hkpMoppCodeCodeInfo' version='1' signature='0xd8fdbb08'><members>
<member name='offset' offset='0' vtype='TYPE_VECTOR4'/>
</members></class>"#,
    ),
    (
        "hkpMoppCode.xml",
        r#"<class name='hkpMoppCode' version='1' signature='0x5104fd7d' parent='hkReferencedObject'><enums>
<enum name='BuildType'><enumitem name='BUILT_WITH_CHUNK_SUBDIVISION' value='0'/><enumitem name='BUILT_WITHOUT_CHUNK_SUBDIVISION' value='1'/><enumitem name='BUILD_NOT_SET' value='2'/></enum>
</enums><members>
<member name='info' offset='16' ctype='hkpMoppCodeCodeInfo' vtype='TYPE_STRUCT'/>
<member name='data' offset='32' vtype='TYPE_ARRAY' vsubtype='TYPE_UINT8'/>
<member name='buildType' offset='48' etype='BuildType' vtype='TYPE_ENUM' vsubtype='TYPE_INT8'/>
</members></class>"#,
    ),
    (
        "hkpConvexVerticesConnectivity.xml",
        r#"<class name='hkpConvexVerticesConnectivity' version='1' signature='0x127f728b' parent='hkReferencedObject'><members>
<member name='vertexIndices' offset='16' vtype='TYPE_ARRAY' vsubtype='TYPE_UINT16'/>
<member name='numVerticesPerFace' offset='32' vtype='TYPE_ARRAY' vsubtype='TYPE_UINT8'/>
</members></class>"#,
    ),
];

fn parse_classxml(path: &Path, xml: &str) -> DescriptorResult<ClassDescriptor> {
    let doc = roxmltree::Document::parse(xml).map_err(|source| DescriptorError::Xml {
        path: path.to_path_buf(),
        source,
    })?;
    let root = doc.root_element();
    if !root.has_tag_name("class") && !root.has_tag_name("struct") {
        return Err(malformed(path, "root must be class or struct"));
    }
    let name = required_attr(path, root, "name")?;

    let class_kind = if name.contains("Setup") {
        ClassKind::Setup
    } else {
        ClassKind::Runtime
    };
    let mut desc = ClassDescriptor {
        name: name.to_string(),
        version: parse_optional_u32(path, root, "version")?.unwrap_or(0),
        signature: root.attribute("signature").unwrap_or_default().to_string(),
        parent: root.attribute("parent").map(str::to_string),
        is_struct: root.tag_name().name() == "struct",
        members: Vec::new(),
        enums: HashMap::new(),
        kind: class_kind,
    };

    for enum_el in root.descendants().filter(|node| node.has_tag_name("enum")) {
        let enum_name = enum_el.attribute("name").unwrap_or_default().to_string();
        let mut enum_def = EnumDef::default();
        for item in enum_el
            .children()
            .filter(|node| node.has_tag_name("enumitem"))
        {
            let item_name = item.attribute("name").unwrap_or_default().to_string();
            let item_value = parse_required_i64(path, item, "value")?;
            enum_def.values.push((item_name, item_value));
        }
        desc.enums.insert(enum_name, enum_def);
    }

    for member_el in root
        .descendants()
        .filter(|node| node.has_tag_name("member"))
    {
        let vtype_name = member_el.attribute("vtype").unwrap_or("TYPE_VOID");
        let vsubtype_name = member_el.attribute("vsubtype").unwrap_or("TYPE_VOID");
        desc.members.push(MemberTemplate {
            name: member_el.attribute("name").unwrap_or_default().to_string(),
            offset: parse_optional_usize(path, member_el, "offset")?.unwrap_or(0),
            vtype: HkxType::from_classxml_name(vtype_name)
                .ok_or_else(|| malformed(path, format!("unknown vtype {vtype_name}")))?,
            vsubtype: HkxType::from_classxml_name(vsubtype_name)
                .ok_or_else(|| malformed(path, format!("unknown vsubtype {vsubtype_name}")))?,
            ctype: member_el.attribute("ctype").unwrap_or_default().to_string(),
            arrsize: parse_optional_usize(path, member_el, "arrsize")?.unwrap_or(0),
            flags: member_el
                .attribute("flags")
                .unwrap_or("FLAGS_NONE")
                .to_string(),
            etype: member_el.attribute("etype").unwrap_or_default().to_string(),
            default: None,
        });
    }

    Ok(desc)
}

fn load_index(dir: &Path) -> DescriptorResult<HashMap<String, String>> {
    let index_path = dir.join("_index.json");
    if let Ok(text) = fs::read_to_string(index_path) {
        let index = serde_json::from_str::<HashMap<String, String>>(&text).map_err(|source| {
            DescriptorError::InvalidIndex {
                path: dir.join("_index.json"),
                source,
            }
        })?;
        for (class_name, filename) in &index {
            validate_index_filename(class_name, filename)?;
        }
        return Ok(index);
    }

    let mut index = HashMap::new();
    let entries = fs::read_dir(dir).map_err(|source| DescriptorError::Read {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("xml") {
            continue;
        }
        let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let class_name = stem.rsplit_once('_').map_or(stem, |(name, _)| name);
        index.insert(class_name.to_string(), filename.to_string());
    }
    Ok(index)
}

fn validate_index_filename(class_name: &str, filename: &str) -> DescriptorResult<()> {
    let path = Path::new(filename);
    if path.is_absolute()
        || path.components().count() != 1
        || filename.contains('/')
        || filename.contains('\\')
        || path.extension().and_then(|ext| ext.to_str()) != Some("xml")
    {
        return Err(DescriptorError::UnsafeIndexPath {
            class_name: class_name.to_string(),
            filename: filename.to_string(),
        });
    }
    Ok(())
}

fn required_attr<'a>(
    path: &Path,
    node: roxmltree::Node<'a, 'a>,
    attr: &str,
) -> DescriptorResult<&'a str> {
    node.attribute(attr)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| malformed(path, format!("missing {attr}")))
}

fn parse_required_i64(
    path: &Path,
    node: roxmltree::Node<'_, '_>,
    attr: &str,
) -> DescriptorResult<i64> {
    let raw = required_attr(path, node, attr)?;
    // Accept decimal, or 0x-prefixed hex (used in some classxml flag values)
    if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).map_err(|_| malformed(path, format!("invalid {attr}")))
    } else {
        raw.parse::<i64>()
            .map_err(|_| malformed(path, format!("invalid {attr}")))
    }
}

fn parse_optional_u32(
    path: &Path,
    node: roxmltree::Node<'_, '_>,
    attr: &str,
) -> DescriptorResult<Option<u32>> {
    node.attribute(attr)
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_| malformed(path, format!("invalid {attr}")))
        })
        .transpose()
}

fn parse_optional_usize(
    path: &Path,
    node: roxmltree::Node<'_, '_>,
    attr: &str,
) -> DescriptorResult<Option<usize>> {
    node.attribute(attr)
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| malformed(path, format!("invalid {attr}")))
        })
        .transpose()
}

fn malformed(path: &Path, message: impl Into<String>) -> DescriptorError {
    DescriptorError::Malformed {
        path: path.to_path_buf(),
        message: message.into(),
    }
}

fn default_classxml_dir() -> PathBuf {
    resource_root().join("classxml")
}

fn resource_root() -> PathBuf {
    if let Ok(value) = std::env::var("CREATION_LIB_RESOURCE_DIR") {
        return PathBuf::from(value);
    }
    lib_root()
        .join("python")
        .join("creation_lib")
        .join("resources")
}

/// Root of the `py_creation_lib` crate tree: two levels up from this crate's
/// `CARGO_MANIFEST_DIR` (`py_creation_lib/native/havok` -> `py_creation_lib`).
/// The classxml resources live at `python/creation_lib/resources` under this
/// root both in the private monorepo (where `py_creation_lib` is a
/// subdirectory) and in the standalone public repo (where `py_creation_lib`
/// itself is the repo root), so resolving from here works in both layouts.
fn lib_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("py_creation_lib/native/havok has a lib root two levels up")
        .to_path_buf()
}
