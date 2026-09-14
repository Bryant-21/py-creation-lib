//! Read-only ActionScript Byte Code (ABC) inspection.
//!
//! [`parse_abc_strings`] stops at the constant pool's string table, which holds
//! every AS3 identifier: enough for a cheap "does this name appear" probe.
//! [`parse_abc_class_names`] continues through `method_info` and `metadata_info`
//! into `instance_info` and resolves each defined class's QName to
//! `package.Class`. A SymbolClass entry needs that, because a name in the string
//! pool may be a method, a variable, or a class defined in another file.
//!
//! Nothing here rewrites ABC. [`crate::class_abc`] synthesizes fresh blocks
//! instead, since u30 varints can be non-canonical and an in-place edit could
//! not promise byte identity.

/// DoABCDefine tag: `u32 flags`, NUL-terminated name, then the ABC block.
pub const DO_ABC_DEFINE: u16 = 82;
/// Legacy DoABC tag: the ABC block directly (no flags/name).
pub const DO_ABC: u16 = 72;

/// Constant-pool prefix of an ABC block: version + the section counts we skip past
/// and the fully-parsed string table (which contains every class/identifier name).
#[derive(Debug, Clone)]
pub struct AbcStringPool {
    pub minor: u16,
    pub major: u16,
    pub int_count: u32,
    pub uint_count: u32,
    pub double_count: u32,
    pub strings: Vec<String>,
}

/// Read a variable-length integer. ABC `u30`/`u32`/`s32` share one encoding: up to
/// five bytes, seven value bits each, low byte first, high bit = continue.
fn read_varint(data: &[u8], pos: &mut usize) -> Result<u32, String> {
    let mut result: u32 = 0;
    for i in 0..5 {
        let b = *data.get(*pos).ok_or("varint overruns ABC")?;
        *pos += 1;
        result |= ((b & 0x7F) as u32) << (7 * i);
        if b & 0x80 == 0 {
            break;
        }
    }
    Ok(result)
}

/// Skip `count.saturating_sub(1)` variable-length integers (a constant-pool scalar
/// section: the entry at index 0 is implicit and not stored).
fn skip_varints(data: &[u8], pos: &mut usize, count: u32) -> Result<(), String> {
    for _ in 1..count.max(1) {
        read_varint(data, pos)?;
    }
    Ok(())
}

/// Offset of the ABC block within a DoABC tag body: 0 for the legacy tag, past
/// the `u32 flags` + NUL-terminated name for `DoABCDefine`.
pub fn abc_block_offset(code: u16, body: &[u8]) -> usize {
    if code != DO_ABC_DEFINE {
        return 0;
    }
    let mut pos = 4usize; // u32 flags
    while pos < body.len() && body[pos] != 0 {
        pos += 1; // NUL-terminated name
    }
    pos + 1 // skip the NUL (or step past end; callers bounds-check)
}

/// Parse a DoABC tag body up through the constant-pool string table.
pub fn parse_abc_strings(code: u16, body: &[u8]) -> Result<AbcStringPool, String> {
    let mut pos = abc_block_offset(code, body);
    if pos + 4 > body.len() {
        return Err("ABC block too short for version fields".into());
    }
    let minor = u16::from_le_bytes([body[pos], body[pos + 1]]);
    let major = u16::from_le_bytes([body[pos + 2], body[pos + 3]]);
    pos += 4;

    // Constant pool: int, uint (variable-length scalars), double (8 bytes each),
    // then the string table — each string is `u30 length` + UTF-8 bytes.
    let int_count = read_varint(body, &mut pos)?;
    skip_varints(body, &mut pos, int_count)?;
    let uint_count = read_varint(body, &mut pos)?;
    skip_varints(body, &mut pos, uint_count)?;
    let double_count = read_varint(body, &mut pos)?;
    let skip = (double_count.saturating_sub(1) as usize) * 8;
    pos = pos
        .checked_add(skip)
        .filter(|&p| p <= body.len())
        .ok_or("double pool overruns ABC")?;

    let string_count = read_varint(body, &mut pos)?;
    let mut strings = Vec::with_capacity(string_count.saturating_sub(1) as usize);
    for _ in 1..string_count.max(1) {
        let len = read_varint(body, &mut pos)? as usize;
        let end = pos
            .checked_add(len)
            .filter(|&p| p <= body.len())
            .ok_or("string pool entry overruns ABC")?;
        strings.push(String::from_utf8_lossy(&body[pos..end]).into_owned());
        pos = end;
    }

    Ok(AbcStringPool {
        minor,
        major,
        int_count,
        uint_count,
        double_count,
        strings,
    })
}

/// QName multiname kinds — the only kinds a class definition's name can take.
const KIND_QNAME: u8 = 0x07;
const KIND_QNAME_A: u8 = 0x0D;

/// The parts of the constant pool needed to spell a class's QName out again.
struct QNamePool {
    strings: Vec<String>,
    /// Namespace → its name's string-pool index (0 for the implicit "any" entry).
    namespaces: Vec<u32>,
    /// Multiname → `(namespace index, string index)`, or `None` for kinds that
    /// are not a QName and therefore can never name a class definition.
    multinames: Vec<Option<(u32, u32)>>,
    /// Multiname → its kind byte, kept so a non-QName can still be described.
    /// Entry 0 is the implicit one and has no kind.
    kinds: Vec<u8>,
    /// Multiname → its simple name, for the kinds that store one even though
    /// they are not statically qualified (`Multiname`, `MultinameA`).
    simple_names: Vec<Option<u32>>,
    /// Namespace → its kind byte.
    ns_kinds: Vec<u8>,
    /// Multiname → the namespaces it is looked up in: one entry for a QName,
    /// the whole set for the set-based kinds.
    ns_of_multiname: Vec<Option<Vec<u32>>>,
}

impl QNamePool {
    fn string(&self, index: u32) -> Result<&str, String> {
        self.strings
            .get(index as usize)
            .map(String::as_str)
            .ok_or_else(|| format!("string index {index} out of range"))
    }

    /// A lenient spelling for trait and parameter types, which need not be
    /// QNames: index 0 is the any-type `*`, and a runtime-qualified or set-based
    /// multiname has no static namespace, so only its simple name comes back.
    /// A QName spells `package.Name`, so a dotted result tells a test that the
    /// emitter chose a statically resolved QName over runtime lookup.
    fn type_name(&self, multiname: u32) -> String {
        if multiname == 0 {
            return "*".to_string();
        }
        if let Ok(q) = self.qualified(multiname) {
            return q;
        }
        match self.simple_names.get(multiname as usize).copied().flatten() {
            Some(name) => self.string(name).unwrap_or("<bad-string>").to_string(),
            None => "<non-qname>".to_string(),
        }
    }

    fn multiname_kind(&self, multiname: u32) -> u8 {
        self.kinds.get(multiname as usize).copied().unwrap_or(0)
    }

    /// `package.Class`, or bare `Class` when the namespace is the unnamed
    /// (top-level) package — the same spelling a SymbolClass entry uses.
    fn qualified(&self, multiname: u32) -> Result<String, String> {
        let entry = self
            .multinames
            .get(multiname as usize)
            .ok_or_else(|| format!("multiname index {multiname} out of range"))?;
        let (ns, name) = entry.ok_or("class name is not a QName")?;
        let package_index = *self
            .namespaces
            .get(ns as usize)
            .ok_or_else(|| format!("namespace index {ns} out of range"))?;
        let package = self.string(package_index)?;
        let class = self.string(name)?;
        Ok(if package.is_empty() {
            class.to_string()
        } else {
            format!("{package}.{class}")
        })
    }
}

/// Cursor over an ABC block. Every read is bounds-checked so a malformed or
/// truncated block reports an error instead of panicking.
struct AbcCursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> AbcCursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn u8(&mut self) -> Result<u8, String> {
        let b = *self.data.get(self.pos).ok_or("ABC block truncated")?;
        self.pos += 1;
        Ok(b)
    }

    fn u16(&mut self) -> Result<u16, String> {
        let hi = self.pos + 2;
        if hi > self.data.len() {
            return Err("ABC block truncated".into());
        }
        let v = u16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos = hi;
        Ok(v)
    }

    fn u30(&mut self) -> Result<u32, String> {
        read_varint(self.data, &mut self.pos)
    }

    fn bytes(&mut self, n: usize) -> Result<Vec<u8>, String> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&p| p <= self.data.len())
            .ok_or("ABC block truncated")?;
        let out = self.data[self.pos..end].to_vec();
        self.pos = end;
        Ok(out)
    }

    fn skip(&mut self, n: usize) -> Result<(), String> {
        self.pos = self
            .pos
            .checked_add(n)
            .filter(|&p| p <= self.data.len())
            .ok_or("ABC block truncated")?;
        Ok(())
    }

    fn skip_u30s(&mut self, n: u32) -> Result<(), String> {
        for _ in 0..n {
            self.u30()?;
        }
        Ok(())
    }

    fn string(&mut self) -> Result<String, String> {
        let len = self.u30()? as usize;
        let end = self
            .pos
            .checked_add(len)
            .filter(|&p| p <= self.data.len())
            .ok_or("string pool entry overruns ABC")?;
        let s = String::from_utf8_lossy(&self.data[self.pos..end]).into_owned();
        self.pos = end;
        Ok(s)
    }

    /// Constant-pool sections store `count - 1` entries: index 0 is implicit. A
    /// count of 0 means the section is empty (that is what ASC emits for an
    /// unused pool), so the entry count is `count.saturating_sub(1)`.
    fn pool_len(&mut self) -> Result<u32, String> {
        Ok(self.u30()?.saturating_sub(1))
    }

    fn constant_pool(&mut self) -> Result<QNamePool, String> {
        let ints = self.pool_len()?;
        self.skip_u30s(ints)?;
        let uints = self.pool_len()?;
        self.skip_u30s(uints)?;
        let doubles = self.pool_len()?;
        self.skip(doubles as usize * 8)?;

        let string_count = self.pool_len()?;
        let mut strings = Vec::with_capacity(string_count as usize + 1);
        strings.push(String::new()); // implicit entry 0
        for _ in 0..string_count {
            strings.push(self.string()?);
        }

        let ns_count = self.pool_len()?;
        let mut namespaces = Vec::with_capacity(ns_count as usize + 1);
        let mut ns_kinds = Vec::with_capacity(ns_count as usize + 1);
        namespaces.push(0); // implicit entry 0 == "any namespace"
        ns_kinds.push(0);
        for _ in 0..ns_count {
            ns_kinds.push(self.u8()?);
            namespaces.push(self.u30()?);
        }

        let ns_set_count = self.pool_len()?;
        let mut ns_sets: Vec<Vec<u32>> = Vec::with_capacity(ns_set_count as usize + 1);
        ns_sets.push(Vec::new()); // implicit entry 0
        for _ in 0..ns_set_count {
            let n = self.u30()?;
            let mut set = Vec::with_capacity(n as usize);
            for _ in 0..n {
                set.push(self.u30()?);
            }
            ns_sets.push(set);
        }

        let mn_count = self.pool_len()?;
        let mut multinames = Vec::with_capacity(mn_count as usize + 1);
        let mut kinds = Vec::with_capacity(mn_count as usize + 1);
        let mut simple_names = Vec::with_capacity(mn_count as usize + 1);
        let mut ns_of_multiname: Vec<Option<Vec<u32>>> = Vec::with_capacity(mn_count as usize + 1);
        multinames.push(None); // implicit entry 0
        kinds.push(0);
        simple_names.push(None);
        ns_of_multiname.push(None);
        for _ in 0..mn_count {
            let kind = self.u8()?;
            kinds.push(kind);
            let mut simple = None;
            let mut ns_list = None;
            multinames.push(match kind {
                KIND_QNAME | KIND_QNAME_A => {
                    let ns = self.u30()?;
                    let name = self.u30()?;
                    simple = Some(name);
                    ns_list = Some(vec![ns]);
                    Some((ns, name))
                }
                0x0F | 0x10 => {
                    simple = Some(self.u30()?); // RTQName: name only
                    None
                }
                0x11 | 0x12 => None, // RTQNameL: nothing stored
                0x09 | 0x0E => {
                    simple = Some(self.u30()?); // Multiname: name
                    let set = self.u30()?;
                    ns_list = ns_sets.get(set as usize).cloned();
                    None
                }
                0x1B | 0x1C => {
                    // MultinameL: the name comes off the stack at runtime.
                    let set = self.u30()?;
                    ns_list = ns_sets.get(set as usize).cloned();
                    None
                }
                0x1D => {
                    self.u30()?; // TypeName: base name
                    let params = self.u30()?;
                    self.skip_u30s(params)?;
                    None
                }
                other => return Err(format!("unknown multiname kind {other:#x}")),
            });
            simple_names.push(simple);
            ns_of_multiname.push(ns_list);
        }

        Ok(QNamePool {
            strings,
            namespaces,
            multinames,
            kinds,
            simple_names,
            ns_kinds,
            ns_of_multiname,
        })
    }

    fn read_method_infos(&mut self, pool: &QNamePool) -> Result<Vec<AbcMethod>, String> {
        let count = self.u30()?;
        let mut out = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let param_count = self.u30()?;
            let return_type = self.u30()?;
            let mut param_types = Vec::with_capacity(param_count as usize);
            for _ in 0..param_count {
                param_types.push(pool.type_name(self.u30()?));
            }
            let name = self.u30()?;
            let flags = self.u8()?;
            if flags & 0x08 != 0 {
                // HAS_OPTIONAL: (u30 value, u8 kind) per default
                let options = self.u30()?;
                for _ in 0..options {
                    self.u30()?;
                    self.u8()?;
                }
            }
            if flags & 0x80 != 0 {
                self.skip_u30s(param_count)?; // HAS_PARAM_NAMES
            }
            out.push(AbcMethod {
                param_types,
                return_type: pool.type_name(return_type),
                name: pool.string(name).unwrap_or("").to_string(),
                flags,
            });
        }
        Ok(out)
    }

    fn skip_method_infos(&mut self) -> Result<(), String> {
        let count = self.u30()?;
        for _ in 0..count {
            let param_count = self.u30()?;
            self.u30()?; // return type
            self.skip_u30s(param_count)?;
            self.u30()?; // name
            let flags = self.u8()?;
            if flags & 0x08 != 0 {
                // HAS_OPTIONAL: (u30 value, u8 kind) per default
                let options = self.u30()?;
                for _ in 0..options {
                    self.u30()?;
                    self.u8()?;
                }
            }
            if flags & 0x80 != 0 {
                self.skip_u30s(param_count)?; // HAS_PARAM_NAMES
            }
        }
        Ok(())
    }

    fn skip_metadata_infos(&mut self) -> Result<(), String> {
        let count = self.u30()?;
        for _ in 0..count {
            self.u30()?; // name
            let items = self.u30()?;
            self.skip_u30s(items * 2)?; // (key, value) pairs
        }
        Ok(())
    }

    fn read_traits(&mut self, pool: &QNamePool) -> Result<Vec<AbcTrait>, String> {
        let count = self.u30()?;
        let mut out = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let name = self.u30()?;
            let tag = self.u8()?;
            let kind = tag & 0x0F;
            let mut slot_id = 0;
            let mut index = 0;
            let mut type_name = String::new();
            match kind {
                0 | 6 => {
                    slot_id = self.u30()?;
                    type_name = pool.type_name(self.u30()?);
                    let value_index = self.u30()?;
                    if value_index != 0 {
                        self.u8()?;
                    }
                }
                // Method, Getter, Setter
                1..=3 => {
                    self.u30()?; // disp id
                    index = self.u30()?;
                }
                4 | 5 => {
                    slot_id = self.u30()?;
                    index = self.u30()?;
                }
                other => return Err(format!("unknown trait kind {other}")),
            }
            if tag & 0x40 != 0 {
                let n = self.u30()?; // ATTR_Metadata
                self.skip_u30s(n)?;
            }
            out.push(AbcTrait {
                name: pool.type_name(name),
                kind,
                attributes: tag >> 4,
                slot_id,
                index,
                type_name,
            });
        }
        Ok(out)
    }

    fn skip_traits(&mut self) -> Result<(), String> {
        let count = self.u30()?;
        for _ in 0..count {
            self.u30()?; // name
            let tag = self.u8()?;
            match tag & 0x0F {
                0 | 6 => {
                    // Slot / Const
                    self.u30()?; // slot id
                    self.u30()?; // type name
                    if self.u30()? != 0 {
                        self.u8()?; // value kind, present only for a non-zero value index
                    }
                }
                1 | 2 | 3 => {
                    // Method / Getter / Setter
                    self.u30()?; // disp id
                    self.u30()?; // method
                }
                4 | 5 => {
                    // Class / Function
                    self.u30()?; // slot id
                    self.u30()?; // class or method index
                }
                other => return Err(format!("unknown trait kind {other}")),
            }
            if tag & 0x40 != 0 {
                let n = self.u30()?; // ATTR_Metadata
                self.skip_u30s(n)?;
            }
        }
        Ok(())
    }
}

/// One `method_info` entry, with its multiname operands resolved to names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbcMethod {
    pub param_types: Vec<String>,
    pub return_type: String,
    /// Informational only — the AVM does not use it, and many emitters leave it
    /// empty.
    pub name: String,
    pub flags: u8,
}

/// One trait (a member of a class, instance, script, or method body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbcTrait {
    pub name: String,
    /// 0 Slot, 1 Method, 2 Getter, 3 Setter, 4 Class, 5 Function, 6 Const.
    pub kind: u8,
    /// High nibble of the trait tag: FINAL 0x1, OVERRIDE 0x2, METADATA 0x4.
    pub attributes: u8,
    pub slot_id: u32,
    /// Method index for Method/Getter/Setter/Function, class index for Class.
    pub index: u32,
    /// Declared type, for Slot and Const traits only.
    pub type_name: String,
}

impl AbcTrait {
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            0 => "slot",
            1 => "method",
            2 => "getter",
            3 => "setter",
            4 => "class",
            5 => "function",
            6 => "const",
            _ => "?",
        }
    }
}

/// One class definition: its `instance_info` and the matching `class_info`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbcClass {
    pub name: String,
    pub super_name: String,
    pub flags: u8,
    pub interfaces: Vec<String>,
    /// Multiname kind of each entry of `interfaces`: 0x07 is a statically
    /// resolved QName, 0x09 a namespace-set lookup left to runtime.
    pub interface_kinds: Vec<u8>,
    pub iinit: u32,
    pub instance_traits: Vec<AbcTrait>,
    pub cinit: u32,
    pub class_traits: Vec<AbcTrait>,
}

impl AbcClass {
    /// A class is sealed unless it was declared `dynamic`; AVM2 spells that as
    /// the presence of `CLASS_SEALED` (0x01) rather than a "dynamic" bit.
    pub fn is_sealed(&self) -> bool {
        self.flags & 0x01 != 0
    }
    pub fn is_final(&self) -> bool {
        self.flags & 0x02 != 0
    }
    pub fn is_interface(&self) -> bool {
        self.flags & 0x04 != 0
    }
    pub fn has_protected_ns(&self) -> bool {
        self.flags & 0x08 != 0
    }
}

/// A structural view of an ABC block: every `method_info` and every class, with
/// multinames resolved back to names.
///
/// Where [`parse_abc_class_names`] lists classes, this gives their shape:
/// declared interfaces, traits, and method signatures. An `implements` is
/// satisfied by traits, not the interface name, so this is what it is checked
/// against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbcDetail {
    pub minor: u16,
    pub major: u16,
    pub methods: Vec<AbcMethod>,
    pub classes: Vec<AbcClass>,
    pub scripts: Vec<AbcScript>,
    pub bodies: Vec<AbcBody>,
}

/// One `script_info`: the initialiser that defines and publishes the classes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbcScript {
    pub init: u32,
    pub traits: Vec<AbcTrait>,
}

/// One `method_body_info`. The four depth fields are what the player's verifier
/// checks a body against, so they are the interesting part; `code` is kept raw
/// because nothing here disassembles bytecode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbcBody {
    pub method: u32,
    pub max_stack: u32,
    pub local_count: u32,
    pub init_scope_depth: u32,
    pub max_scope_depth: u32,
    pub code: Vec<u8>,
    pub exception_count: u32,
    pub traits: Vec<AbcTrait>,
}

impl AbcDetail {
    pub fn class(&self, name: &str) -> Option<&AbcClass> {
        self.classes.iter().find(|c| c.name == name)
    }

    pub fn body(&self, method: u32) -> Option<&AbcBody> {
        self.bodies.iter().find(|b| b.method == method)
    }
}

/// One multiname, with its namespaces resolved to names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbcMultiname {
    /// 0x07 QName, 0x09 Multiname, 0x1B MultinameL, and so on.
    pub kind: u8,
    /// Empty for the `L` kinds, which take their name from the stack.
    pub name: String,
    /// The namespace (for a QName) or namespace set this name is looked up in.
    pub namespaces: Vec<(u8, String)>,
}

/// Every multiname in the constant pool, index-aligned (entry 0 is the implicit
/// one).
///
/// `a[i]` compiles to a `getproperty` on a runtime-qualified multiname whose
/// name comes off the stack, which changes how many operands it pops. Reading
/// the kind back lets an emitter's stack accounting be checked against a file.
pub fn parse_abc_multinames(code: u16, body: &[u8]) -> Result<Vec<AbcMultiname>, String> {
    let offset = abc_block_offset(code, body);
    let block = body
        .get(offset..)
        .ok_or("DoABC tag body truncated before the ABC block")?;
    let mut cur = AbcCursor::new(block);
    cur.u16()?;
    cur.u16()?;
    let pool = cur.constant_pool()?;

    let mut out = Vec::with_capacity(pool.kinds.len());
    for index in 0..pool.kinds.len() {
        let kind = pool.kinds[index];
        let name = pool
            .simple_names
            .get(index)
            .copied()
            .flatten()
            .and_then(|n| pool.string(n).ok())
            .unwrap_or("")
            .to_string();
        let namespaces = match pool.ns_of_multiname.get(index).and_then(|o| o.as_ref()) {
            Some(list) => list
                .iter()
                .map(|&ns| {
                    let name = pool.namespaces.get(ns as usize).copied().unwrap_or(0);
                    (
                        pool.ns_kinds.get(ns as usize).copied().unwrap_or(0),
                        pool.string(name).unwrap_or("").to_string(),
                    )
                })
                .collect(),
            None => Vec::new(),
        };
        out.push(AbcMultiname {
            kind,
            name,
            namespaces,
        });
    }
    Ok(out)
}

/// Every namespace in the constant pool as `(kind, name)`, in pool order.
///
/// The kind byte is not cosmetic: `PackageNamespace` (0x16), `Namespace`
/// (0x08) and `ProtectedNamespace` (0x18) select different lookup rules, so an
/// emitter that picks the wrong one produces traits the runtime cannot find.
pub fn parse_abc_namespaces(code: u16, body: &[u8]) -> Result<Vec<(u8, String)>, String> {
    let offset = abc_block_offset(code, body);
    let block = body
        .get(offset..)
        .ok_or("DoABC tag body truncated before the ABC block")?;
    let mut cur = AbcCursor::new(block);
    cur.u16()?;
    cur.u16()?;

    let ints = cur.pool_len()?;
    cur.skip_u30s(ints)?;
    let uints = cur.pool_len()?;
    cur.skip_u30s(uints)?;
    let doubles = cur.pool_len()?;
    cur.skip(doubles as usize * 8)?;

    let string_count = cur.pool_len()?;
    let mut strings = vec![String::new()];
    for _ in 0..string_count {
        strings.push(cur.string()?);
    }

    let ns_count = cur.pool_len()?;
    let mut out = Vec::with_capacity(ns_count as usize);
    for _ in 0..ns_count {
        let kind = cur.u8()?;
        let name = cur.u30()? as usize;
        out.push((
            kind,
            strings.get(name).cloned().unwrap_or_else(|| "?".into()),
        ));
    }
    Ok(out)
}

pub fn parse_abc_detail(code: u16, body: &[u8]) -> Result<AbcDetail, String> {
    let offset = abc_block_offset(code, body);
    let block = body
        .get(offset..)
        .ok_or("DoABC tag body truncated before the ABC block")?;
    let mut cur = AbcCursor::new(block);
    let minor = cur.u16()?;
    let major = cur.u16()?;
    let pool = cur.constant_pool()?;
    let methods = cur.read_method_infos(&pool)?;
    cur.skip_metadata_infos()?;

    let class_count = cur.u30()?;
    let mut classes: Vec<AbcClass> = Vec::with_capacity(class_count as usize);
    for _ in 0..class_count {
        let name = cur.u30()?;
        let super_name = cur.u30()?;
        let flags = cur.u8()?;
        if flags & 0x08 != 0 {
            cur.u30()?; // CLASS_PROTECTED_NS
        }
        let interface_count = cur.u30()?;
        let mut interfaces = Vec::with_capacity(interface_count as usize);
        let mut interface_kinds = Vec::with_capacity(interface_count as usize);
        for _ in 0..interface_count {
            let mn = cur.u30()?;
            interfaces.push(pool.type_name(mn));
            interface_kinds.push(pool.multiname_kind(mn));
        }
        let iinit = cur.u30()?;
        let instance_traits = cur.read_traits(&pool)?;
        classes.push(AbcClass {
            name: pool.qualified(name)?,
            super_name: pool.type_name(super_name),
            flags,
            interfaces,
            interface_kinds,
            iinit,
            instance_traits,
            cinit: 0,
            class_traits: Vec::new(),
        });
    }
    // `class_info` is a second run after every `instance_info`, not interleaved.
    for class in classes.iter_mut() {
        class.cinit = cur.u30()?;
        class.class_traits = cur.read_traits(&pool)?;
    }

    let script_count = cur.u30()?;
    let mut scripts = Vec::with_capacity(script_count as usize);
    for _ in 0..script_count {
        let init = cur.u30()?;
        let traits = cur.read_traits(&pool)?;
        scripts.push(AbcScript { init, traits });
    }

    let body_count = cur.u30()?;
    let mut bodies = Vec::with_capacity(body_count as usize);
    for _ in 0..body_count {
        let method = cur.u30()?;
        let max_stack = cur.u30()?;
        let local_count = cur.u30()?;
        let init_scope_depth = cur.u30()?;
        let max_scope_depth = cur.u30()?;
        let code_len = cur.u30()? as usize;
        let code = cur.bytes(code_len)?;
        let exception_count = cur.u30()?;
        for _ in 0..exception_count {
            cur.u30()?; // from
            cur.u30()?; // to
            cur.u30()?; // target
            cur.u30()?; // exc_type
            cur.u30()?; // var_name
        }
        let traits = cur.read_traits(&pool)?;
        bodies.push(AbcBody {
            method,
            max_stack,
            local_count,
            init_scope_depth,
            max_scope_depth,
            code,
            exception_count,
            traits,
        });
    }

    Ok(AbcDetail {
        minor,
        major,
        methods,
        classes,
        scripts,
        bodies,
    })
}

/// Fully-qualified names (`package.Class`, or bare `Class` in the unnamed
/// package) of every class *defined* by this ABC block — i.e. one entry per
/// `instance_info`. This is the set a SymbolClass export name must be drawn from
/// for the binding to resolve.
pub fn parse_abc_class_names(code: u16, body: &[u8]) -> Result<Vec<String>, String> {
    let offset = abc_block_offset(code, body);
    let block = body
        .get(offset..)
        .ok_or("DoABC tag body truncated before the ABC block")?;
    let mut cur = AbcCursor::new(block);
    cur.u16()?; // minor version
    cur.u16()?; // major version
    let pool = cur.constant_pool()?;
    cur.skip_method_infos()?;
    cur.skip_metadata_infos()?;

    let class_count = cur.u30()?;
    let mut names = Vec::with_capacity(class_count as usize);
    for _ in 0..class_count {
        let name = cur.u30()?;
        cur.u30()?; // super name
        let flags = cur.u8()?;
        if flags & 0x08 != 0 {
            cur.u30()?; // CLASS_PROTECTED_NS
        }
        let interfaces = cur.u30()?;
        cur.skip_u30s(interfaces)?;
        cur.u30()?; // iinit
        cur.skip_traits()?;
        names.push(pool.qualified(name)?);
    }
    Ok(names)
}
