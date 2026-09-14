//! AVM2 constant pool.
//!
//! Written from the *ActionScript Virtual Machine 2 Overview*, section 4
//! (constant pool), not from another compiler's source.
//!
//! Every section is interned, so a repeated value returns the same index.
//! Indices are 1-based; index 0 is implicit in every section and means "none"
//! (for a namespace, the any-namespace).
//!
//! Entries keep insertion order. swftools re-sorts the pool by use count so hot
//! constants get shorter varints, but then indices are unstable until the whole
//! program is walked and the ABC must be serialized twice. Insertion order keeps
//! emission single-pass and deterministic, at a cost of a few bytes.

use std::collections::HashMap;

/// Namespace kinds (AVM2 overview, "Namespace").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NsKind {
    Namespace = 0x08,
    PackageNamespace = 0x16,
    PackageInternalNs = 0x17,
    ProtectedNamespace = 0x18,
    ExplicitNamespace = 0x19,
    StaticProtectedNs = 0x1A,
    PrivateNs = 0x05,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct NamespaceEntry {
    kind: NsKind,
    name: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum MultinameEntry {
    /// `QName`: a fully-resolved `namespace::name`.
    QName { ns: u32, name: u32 },
    /// `Multiname`: a name plus the set of namespaces to search, which is what
    /// an unqualified reference compiles to when resolution is left to runtime.
    Multiname { name: u32, ns_set: u32 },
    /// `MultinameL`: like `Multiname`, but the *name* is taken off the operand
    /// stack instead of the pool. This is what `a[i]` compiles to, and it is
    /// why an indexed access pops one more operand than a named one.
    MultinameL { ns_set: u32 },
}

const KIND_QNAME: u8 = 0x07;
const KIND_MULTINAME: u8 = 0x09;
const KIND_MULTINAME_L: u8 = 0x1B;

pub fn write_u30(out: &mut Vec<u8>, mut value: u32) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// `s32` shares the `u30` encoding; a negative value is written as its
/// two's-complement bit pattern, which is why the cast is the whole conversion.
pub fn write_s32(out: &mut Vec<u8>, value: i32) {
    write_u30(out, value as u32);
}

#[derive(Debug, Default)]
pub struct ConstantPool {
    ints: Vec<i32>,
    int_index: HashMap<i32, u32>,
    uints: Vec<u32>,
    uint_index: HashMap<u32, u32>,
    /// Stored as bit patterns so interning is exact — `0.0` and `-0.0` are
    /// different constants and `NaN` must still dedupe against itself.
    doubles: Vec<u64>,
    double_index: HashMap<u64, u32>,
    strings: Vec<String>,
    string_index: HashMap<String, u32>,
    namespaces: Vec<NamespaceEntry>,
    namespace_index: HashMap<NamespaceEntry, u32>,
    ns_sets: Vec<Vec<u32>>,
    ns_set_index: HashMap<Vec<u32>, u32>,
    multinames: Vec<MultinameEntry>,
    multiname_index: HashMap<MultinameEntry, u32>,
}

/// Intern `value` into `(items, index)`, returning its 1-based pool index.
fn intern<T: Clone + std::hash::Hash + Eq>(
    items: &mut Vec<T>,
    index: &mut HashMap<T, u32>,
    value: T,
) -> u32 {
    if let Some(&i) = index.get(&value) {
        return i;
    }
    items.push(value.clone());
    let i = items.len() as u32;
    index.insert(value, i);
    i
}

impl ConstantPool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn int(&mut self, value: i32) -> u32 {
        intern(&mut self.ints, &mut self.int_index, value)
    }

    pub fn uint(&mut self, value: u32) -> u32 {
        intern(&mut self.uints, &mut self.uint_index, value)
    }

    pub fn double(&mut self, value: f64) -> u32 {
        intern(&mut self.doubles, &mut self.double_index, value.to_bits())
    }

    /// The empty string is a real pool entry, not index 0: an unnamed package's
    /// namespace has `""` as its name and must point at a stored string.
    pub fn string(&mut self, text: &str) -> u32 {
        if let Some(&i) = self.string_index.get(text) {
            return i;
        }
        self.strings.push(text.to_string());
        let i = self.strings.len() as u32;
        self.string_index.insert(text.to_string(), i);
        i
    }

    pub fn namespace(&mut self, kind: NsKind, name: &str) -> u32 {
        let name = self.string(name);
        intern(
            &mut self.namespaces,
            &mut self.namespace_index,
            NamespaceEntry { kind, name },
        )
    }

    pub fn ns_set(&mut self, namespaces: &[u32]) -> u32 {
        intern(
            &mut self.ns_sets,
            &mut self.ns_set_index,
            namespaces.to_vec(),
        )
    }

    /// A `QName` from an already-interned namespace.
    pub fn qname_in(&mut self, ns: u32, local: &str) -> u32 {
        let name = self.string(local);
        intern(
            &mut self.multinames,
            &mut self.multiname_index,
            MultinameEntry::QName { ns, name },
        )
    }

    /// A `QName` for `package::local`, interning the package namespace first.
    ///
    /// The interning order decides pool indices, and so every byte downstream,
    /// which is why the namespace always goes before the local name.
    pub fn qname(&mut self, kind: NsKind, package: &str, local: &str) -> u32 {
        let ns = self.namespace(kind, package);
        self.qname_in(ns, local)
    }

    /// A `MultinameL` — the operand of an indexed read or write, where the
    /// property name arrives on the operand stack rather than from the pool.
    pub fn multiname_l(&mut self, ns_set: u32) -> u32 {
        intern(
            &mut self.multinames,
            &mut self.multiname_index,
            MultinameEntry::MultinameL { ns_set },
        )
    }

    /// A `Multiname`: an unqualified name resolved against a set of namespaces
    /// at runtime. This is what an unadorned identifier compiles to when the
    /// compiler cannot pin it to one namespace.
    pub fn multiname(&mut self, local: &str, ns_set: u32) -> u32 {
        let name = self.string(local);
        intern(
            &mut self.multinames,
            &mut self.multiname_index,
            MultinameEntry::Multiname { name, ns_set },
        )
    }

    pub fn string_count(&self) -> usize {
        self.strings.len()
    }

    pub fn multiname_count(&self) -> usize {
        self.multinames.len()
    }

    /// A section's count field: `entries + 1` because index 0 is implicit, but
    /// a bare `0` when the section is empty — which is what Adobe's compiler
    /// writes for an unused pool and what the reader in `swf_native::abc`
    /// expects (`pool_len` saturates at 0).
    fn section_count(len: usize) -> u32 {
        if len == 0 { 0 } else { len as u32 + 1 }
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        write_u30(out, Self::section_count(self.ints.len()));
        for &v in &self.ints {
            write_s32(out, v);
        }

        write_u30(out, Self::section_count(self.uints.len()));
        for &v in &self.uints {
            write_u30(out, v);
        }

        write_u30(out, Self::section_count(self.doubles.len()));
        for &bits in &self.doubles {
            out.extend_from_slice(&bits.to_le_bytes());
        }

        write_u30(out, Self::section_count(self.strings.len()));
        for s in &self.strings {
            write_u30(out, s.len() as u32);
            out.extend_from_slice(s.as_bytes());
        }

        write_u30(out, Self::section_count(self.namespaces.len()));
        for ns in &self.namespaces {
            out.push(ns.kind as u8);
            write_u30(out, ns.name);
        }

        write_u30(out, Self::section_count(self.ns_sets.len()));
        for set in &self.ns_sets {
            write_u30(out, set.len() as u32);
            for &ns in set {
                write_u30(out, ns);
            }
        }

        write_u30(out, Self::section_count(self.multinames.len()));
        for mn in &self.multinames {
            match *mn {
                MultinameEntry::QName { ns, name } => {
                    out.push(KIND_QNAME);
                    write_u30(out, ns);
                    write_u30(out, name);
                }
                MultinameEntry::Multiname { name, ns_set } => {
                    out.push(KIND_MULTINAME);
                    write_u30(out, name);
                    write_u30(out, ns_set);
                }
                MultinameEntry::MultinameL { ns_set } => {
                    out.push(KIND_MULTINAME_L);
                    write_u30(out, ns_set);
                }
            }
        }
    }
}
