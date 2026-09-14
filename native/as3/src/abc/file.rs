//! The ABC file model and its serializer.
//!
//! Written from the *AVM2 Overview*, section 4 ("abcFile"), which fixes both
//! the field order and the interleaving of `instance_info` and `class_info`.
//! Not derived from another compiler's source.

use super::pool::{ConstantPool, write_u30};

/// Emitted by every Flash-era authoring tool, and by the shipping FO4 menu
/// SWFs this toolkit reads back.
pub const ABC_MINOR: u16 = 16;
pub const ABC_MAJOR: u16 = 46;

/// `method_info` flags.
pub const METHOD_NEED_ARGUMENTS: u8 = 0x01;
pub const METHOD_NEED_ACTIVATION: u8 = 0x02;
pub const METHOD_NEED_REST: u8 = 0x04;
pub const METHOD_HAS_OPTIONAL: u8 = 0x08;
pub const METHOD_HAS_PARAM_NAMES: u8 = 0x80;

/// `instance_info` flags.
pub const CLASS_SEALED: u8 = 0x01;
pub const CLASS_FINAL: u8 = 0x02;
pub const CLASS_INTERFACE: u8 = 0x04;
pub const CLASS_PROTECTED_NS: u8 = 0x08;

#[derive(Debug, Clone, Default)]
pub struct MethodInfo {
    pub param_types: Vec<u32>,
    pub return_type: u32,
    pub name: u32,
    pub flags: u8,
}

#[derive(Debug, Clone)]
pub enum TraitKind {
    Slot {
        slot_id: u32,
        type_name: u32,
        /// Index into the pool section named by `value_kind`; 0 means no value.
        value_index: u32,
        value_kind: u8,
    },
    Method {
        disp_id: u32,
        method: u32,
    },
    Getter {
        disp_id: u32,
        method: u32,
    },
    Setter {
        disp_id: u32,
        method: u32,
    },
    Class {
        slot_id: u32,
        class_index: u32,
    },
}

impl TraitKind {
    fn tag(&self) -> u8 {
        match self {
            TraitKind::Slot { .. } => 0,
            TraitKind::Method { .. } => 1,
            TraitKind::Getter { .. } => 2,
            TraitKind::Setter { .. } => 3,
            TraitKind::Class { .. } => 4,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Trait {
    pub name: u32,
    pub kind: TraitKind,
}

impl Trait {
    fn write(&self, out: &mut Vec<u8>) {
        write_u30(out, self.name);
        // The high nibble carries attributes (final/override/metadata); none of
        // them are emitted yet, so the tag is the kind alone.
        out.push(self.kind.tag());
        match self.kind {
            TraitKind::Slot {
                slot_id,
                type_name,
                value_index,
                value_kind,
            } => {
                write_u30(out, slot_id);
                write_u30(out, type_name);
                write_u30(out, value_index);
                if value_index != 0 {
                    out.push(value_kind);
                }
            }
            TraitKind::Method { disp_id, method }
            | TraitKind::Getter { disp_id, method }
            | TraitKind::Setter { disp_id, method } => {
                write_u30(out, disp_id);
                write_u30(out, method);
            }
            TraitKind::Class {
                slot_id,
                class_index,
            } => {
                write_u30(out, slot_id);
                write_u30(out, class_index);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstanceInfo {
    pub name: u32,
    pub super_name: u32,
    pub flags: u8,
    pub protected_ns: Option<u32>,
    pub interfaces: Vec<u32>,
    pub iinit: u32,
    pub traits: Vec<Trait>,
}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub cinit: u32,
    pub traits: Vec<Trait>,
}

#[derive(Debug, Clone)]
pub struct ScriptInfo {
    pub init: u32,
    pub traits: Vec<Trait>,
}

#[derive(Debug, Clone)]
pub struct MethodBody {
    pub method: u32,
    pub max_stack: u32,
    pub local_count: u32,
    pub init_scope_depth: u32,
    pub max_scope_depth: u32,
    pub code: Vec<u8>,
    pub traits: Vec<Trait>,
}

#[derive(Debug)]
pub struct AbcFile {
    pub minor: u16,
    pub major: u16,
    pub pool: ConstantPool,
    pub methods: Vec<MethodInfo>,
    pub instances: Vec<InstanceInfo>,
    pub classes: Vec<ClassInfo>,
    pub scripts: Vec<ScriptInfo>,
    pub bodies: Vec<MethodBody>,
}

impl AbcFile {
    pub fn new(pool: ConstantPool) -> Self {
        Self {
            minor: ABC_MINOR,
            major: ABC_MAJOR,
            pool,
            methods: Vec::new(),
            instances: Vec::new(),
            classes: Vec::new(),
            scripts: Vec::new(),
            bodies: Vec::new(),
        }
    }

    /// Serialize the whole block. `instance_info` and `class_info` share one
    /// `class_count` and are written as two consecutive runs, not interleaved
    /// per class — a detail that is easy to get wrong and produces a file that
    /// parses right up to the first class trait.
    pub fn write(&self) -> Result<Vec<u8>, String> {
        if self.classes.len() != self.instances.len() {
            return Err(format!(
                "class_info count {} does not match instance_info count {}",
                self.classes.len(),
                self.instances.len()
            ));
        }
        let mut out = Vec::new();
        out.extend_from_slice(&self.minor.to_le_bytes());
        out.extend_from_slice(&self.major.to_le_bytes());
        self.pool.write(&mut out);

        write_u30(&mut out, self.methods.len() as u32);
        for m in &self.methods {
            write_u30(&mut out, m.param_types.len() as u32);
            write_u30(&mut out, m.return_type);
            for &p in &m.param_types {
                write_u30(&mut out, p);
            }
            write_u30(&mut out, m.name);
            out.push(m.flags);
            if m.flags & (METHOD_HAS_OPTIONAL | METHOD_HAS_PARAM_NAMES) != 0 {
                return Err(
                    "method_info optional-value and parameter-name tables are not emitted yet"
                        .into(),
                );
            }
        }

        write_u30(&mut out, 0); // metadata_count

        write_u30(&mut out, self.instances.len() as u32);
        for inst in &self.instances {
            write_u30(&mut out, inst.name);
            write_u30(&mut out, inst.super_name);
            let flags = if inst.protected_ns.is_some() {
                inst.flags | CLASS_PROTECTED_NS
            } else {
                inst.flags & !CLASS_PROTECTED_NS
            };
            out.push(flags);
            if let Some(ns) = inst.protected_ns {
                write_u30(&mut out, ns);
            }
            write_u30(&mut out, inst.interfaces.len() as u32);
            for &i in &inst.interfaces {
                write_u30(&mut out, i);
            }
            write_u30(&mut out, inst.iinit);
            write_u30(&mut out, inst.traits.len() as u32);
            for t in &inst.traits {
                t.write(&mut out);
            }
        }
        for class in &self.classes {
            write_u30(&mut out, class.cinit);
            write_u30(&mut out, class.traits.len() as u32);
            for t in &class.traits {
                t.write(&mut out);
            }
        }

        write_u30(&mut out, self.scripts.len() as u32);
        for script in &self.scripts {
            write_u30(&mut out, script.init);
            write_u30(&mut out, script.traits.len() as u32);
            for t in &script.traits {
                t.write(&mut out);
            }
        }

        write_u30(&mut out, self.bodies.len() as u32);
        for body in &self.bodies {
            write_u30(&mut out, body.method);
            write_u30(&mut out, body.max_stack);
            write_u30(&mut out, body.local_count);
            write_u30(&mut out, body.init_scope_depth);
            write_u30(&mut out, body.max_scope_depth);
            write_u30(&mut out, body.code.len() as u32);
            out.extend_from_slice(&body.code);
            write_u30(&mut out, 0); // exception_count
            write_u30(&mut out, body.traits.len() as u32);
            for t in &body.traits {
                t.write(&mut out);
            }
        }

        Ok(out)
    }
}

/// Wrap an ABC block as a `DoABCDefine` (tag 82) body: `u32 flags` then a
/// NUL-terminated name. `flags = 1` is lazy initialisation and the name is
/// empty, matching the shipping reference files.
pub fn do_abc_define_body(abc: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(abc.len() + 5);
    out.extend_from_slice(&1u32.to_le_bytes());
    out.push(0);
    out.extend_from_slice(abc);
    out
}
