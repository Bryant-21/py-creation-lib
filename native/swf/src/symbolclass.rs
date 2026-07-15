//! SymbolClass (76), ExportAssets (56), and ImportAssets2 (71): the tags that
//! bind a character ID to an export/class name. FO4 resolves a REFR.TNAM marker
//! type to its icon by the SymbolClass export name and order (FO4Edit derives
//! the TNAM enum names the same way), so these tags are the heart of marker
//! injection. SymbolClass and ExportAssets share the same body layout.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolEntry {
    pub character_id: u16,
    pub name: String,
}

/// Parse a SymbolClass / ExportAssets tag body: `u16 count`, then `count` ×
/// (`u16` character id, NUL-terminated UTF-8 name).
pub fn parse_symbol_table(tag_body: &[u8]) -> Result<Vec<SymbolEntry>, String> {
    if tag_body.len() < 2 {
        return Err("symbol table too short".into());
    }
    let count = u16::from_le_bytes([tag_body[0], tag_body[1]]) as usize;
    let mut p = 2;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if p + 2 > tag_body.len() {
            return Err("truncated symbol id".into());
        }
        let character_id = u16::from_le_bytes([tag_body[p], tag_body[p + 1]]);
        p += 2;
        let start = p;
        while p < tag_body.len() && tag_body[p] != 0 {
            p += 1;
        }
        if p >= tag_body.len() {
            return Err("unterminated symbol name".into());
        }
        // SymbolClass names are ASCII class identifiers, so lossy == identity;
        // the round-trip test enforces byte-exactness.
        let name = String::from_utf8_lossy(&tag_body[start..p]).into_owned();
        p += 1; // skip NUL
        out.push(SymbolEntry { character_id, name });
    }
    Ok(out)
}

pub fn encode_symbol_table(entries: &[SymbolEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for e in entries {
        out.extend_from_slice(&e.character_id.to_le_bytes());
        out.extend_from_slice(e.name.as_bytes());
        out.push(0);
    }
    out
}
