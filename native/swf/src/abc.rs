//! Read-only ActionScript Byte Code (ABC) inspection — just enough to answer
//! "does each SymbolClass export have a backing AS3 class?".
//!
//! Every AS3 class name lives in the ABC constant pool's string table, so we
//! parse the DoABC tag header and the constant pool up through that string table
//! and stop. The rest of the ABC (method/metadata/class/script/method-body info)
//! is left opaque. This is evidence-gathering for the marker-injection resolution
//! question, NOT an ABC editor — byte-exact ABC rewriting for class synthesis is a
//! separate, gated effort (u30 varints can be encoded non-canonically, which makes
//! round-trip byte-identity hard and is unnecessary for inspection).

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

/// Parse a DoABC tag body up through the constant-pool string table.
pub fn parse_abc_strings(code: u16, body: &[u8]) -> Result<AbcStringPool, String> {
    let mut pos = 0usize;
    if code == DO_ABC_DEFINE {
        pos += 4; // u32 flags
        while pos < body.len() && body[pos] != 0 {
            pos += 1; // NUL-terminated name
        }
        pos += 1; // skip the NUL (or step past end; bounds-checked below)
    }
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
