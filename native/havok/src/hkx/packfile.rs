use crate::error::{HavokError, HavokResult};

pub const HKX_MAGIC: &[u8; 8] = b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackfileHeader {
    pub version: u32,
    pub version_name: String,
    pub padding_size: usize,
    pub pointer_size: u8,
    pub section_header_size: usize,
    pub contents_section_index: u32,
    pub contents_section_offset: u32,
    pub contents_class_name_section_index: u32,
    pub contents_class_name_section_offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionHeader {
    pub name: String,
    pub offset: usize,
    pub data1: usize,
    pub data2: usize,
    pub data3: usize,
    pub exports: usize,
    pub imports: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassnameEntry {
    pub position: usize,
    pub signature: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFixup {
    pub source: u32,
    pub target: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalFixup {
    pub source: u32,
    pub section: u32,
    pub target: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualFixup {
    pub source: u32,
    pub section: u32,
    pub classname_offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPackfile {
    pub header: PackfileHeader,
    pub sections: Vec<SectionHeader>,
    pub classnames: Vec<ClassnameEntry>,
    pub local_fixups: Vec<LocalFixup>,
    pub global_fixups: Vec<GlobalFixup>,
    pub virtual_fixups: Vec<VirtualFixup>,
}

impl ParsedPackfile {
    pub fn section(&self, name: &str) -> Option<&SectionHeader> {
        self.sections.iter().find(|section| section.name == name)
    }
}

pub fn parse_packfile(data: &[u8]) -> HavokResult<ParsedPackfile> {
    let header = parse_header(data)?;
    let sections = parse_sections(data, &header)?;
    let classnames_section = find_section(&sections, "__classnames__")?;
    find_section(&sections, "__types__")?;
    let data_section = find_section(&sections, "__data__")?;
    let classnames = parse_classnames(data, classnames_section)?;
    let local_fixups = parse_local_fixups(data, data_section)?;
    let global_fixups = parse_global_fixups(data, data_section)?;
    let virtual_fixups = parse_virtual_fixups(data, data_section)?;

    Ok(ParsedPackfile {
        header,
        sections,
        classnames,
        local_fixups,
        global_fixups,
        virtual_fixups,
    })
}

pub fn parse_header(data: &[u8]) -> HavokResult<PackfileHeader> {
    ensure_len(data, 0x40, "packfile header")?;
    if &data[0..8] != HKX_MAGIC {
        return Err(HavokError::UnsupportedFormat(
            "not an HKX packfile magic".to_string(),
        ));
    }

    let version = read_u32(data, 0x0C, "packfile version")?;
    if version != 8 && version != 11 {
        return Err(HavokError::UnsupportedFormat(format!(
            "packfile version {version}"
        )));
    }

    let pointer_size = data[0x10];
    let version_name = read_c_string(data, 0x28, 0x3E, "version name")?;
    let padding_size = if version == 11 {
        data[0x3E] as usize
    } else {
        0
    };
    let section_header_size = if version == 11 { 0x40 } else { 0x30 };

    Ok(PackfileHeader {
        version,
        version_name,
        padding_size,
        pointer_size,
        section_header_size,
        contents_section_index: read_u32(data, 0x18, "contents section index")?,
        contents_section_offset: read_u32(data, 0x1C, "contents section offset")?,
        contents_class_name_section_index: read_u32(data, 0x20, "contents class section index")?,
        contents_class_name_section_offset: read_u32(data, 0x24, "contents class offset")?,
    })
}

pub fn parse_sections(data: &[u8], header: &PackfileHeader) -> HavokResult<Vec<SectionHeader>> {
    let section_table_start = checked_add(0x40, header.padding_size, "section table start")?;
    let mut sections = Vec::with_capacity(3);

    for index in 0..3 {
        let base = checked_add(
            section_table_start,
            index * header.section_header_size,
            "section header offset",
        )?;
        ensure_len(data, base + header.section_header_size, "section header")?;

        let name = read_c_string(data, base, base + 16, "section name")?;
        let offset = read_u32(data, base + 0x14, "section offset")? as usize;
        let data1 = checked_add(
            offset,
            read_u32(data, base + 0x18, "section data1")? as usize,
            "section data1 absolute",
        )?;
        let data2 = checked_add(
            offset,
            read_u32(data, base + 0x1C, "section data2")? as usize,
            "section data2 absolute",
        )?;
        let data3 = checked_add(
            offset,
            read_u32(data, base + 0x20, "section data3")? as usize,
            "section data3 absolute",
        )?;
        let exports = checked_add(
            offset,
            read_u32(data, base + 0x24, "section exports")? as usize,
            "section exports absolute",
        )?;
        let imports = checked_add(
            offset,
            read_u32(data, base + 0x28, "section imports")? as usize,
            "section imports absolute",
        )?;
        let end = checked_add(
            offset,
            read_u32(data, base + 0x2C, "section end")? as usize,
            "section end absolute",
        )?;

        if offset > data.len()
            || data1 > data.len()
            || data2 > data.len()
            || data3 > data.len()
            || exports > data.len()
            || imports > data.len()
            || end > data.len()
        {
            return Err(HavokError::InvalidInput(format!(
                "section {name} points outside file"
            )));
        }
        if !(offset <= data1
            && data1 <= data2
            && data2 <= data3
            && data3 <= exports
            && exports <= imports
            && imports <= end)
        {
            return Err(HavokError::InvalidInput(format!(
                "section {name} has non-monotonic ranges"
            )));
        }

        sections.push(SectionHeader {
            name,
            offset,
            data1,
            data2,
            data3,
            exports,
            imports,
            end,
        });
    }

    Ok(sections)
}

pub fn parse_classnames(data: &[u8], section: &SectionHeader) -> HavokResult<Vec<ClassnameEntry>> {
    let mut entries = Vec::new();
    let mut pos = section.offset;
    let end = section.data1;

    while pos + 5 <= end {
        let signature = read_u32(data, pos, "classname signature")?;
        if signature == u32::MAX {
            break;
        }
        pos += 4;
        if data[pos] != 0x09 {
            break;
        }
        pos += 1;

        let string_start = pos;
        while pos < end && data[pos] != 0 {
            pos += 1;
        }
        if pos >= end {
            return Err(HavokError::InvalidInput(
                "classname string is not null terminated".to_string(),
            ));
        }
        let name = decode_ascii(&data[string_start..pos], "classname")?;
        entries.push(ClassnameEntry {
            position: string_start - section.offset,
            signature,
            name,
        });
        pos += 1;
    }

    Ok(entries)
}

pub fn parse_local_fixups(data: &[u8], section: &SectionHeader) -> HavokResult<Vec<LocalFixup>> {
    let mut fixups = Vec::new();
    let mut pos = section.data1;

    while pos + 8 <= section.data2 {
        let source = read_u32(data, pos, "local fixup source")?;
        let target = read_u32(data, pos + 4, "local fixup target")?;
        if source == u32::MAX {
            break;
        }
        fixups.push(LocalFixup { source, target });
        pos += 8;
    }

    Ok(fixups)
}

pub fn parse_global_fixups(data: &[u8], section: &SectionHeader) -> HavokResult<Vec<GlobalFixup>> {
    let mut fixups = Vec::new();
    let mut pos = section.data2;

    while pos + 12 <= section.data3 {
        let source = read_u32(data, pos, "global fixup source")?;
        let section_index = read_u32(data, pos + 4, "global fixup section")?;
        let target = read_u32(data, pos + 8, "global fixup target")?;
        if source == u32::MAX {
            break;
        }
        fixups.push(GlobalFixup {
            source,
            section: section_index,
            target,
        });
        pos += 12;
    }

    Ok(fixups)
}

pub fn parse_virtual_fixups(
    data: &[u8],
    section: &SectionHeader,
) -> HavokResult<Vec<VirtualFixup>> {
    let mut fixups = Vec::new();
    let mut pos = section.data3;

    while pos + 12 <= section.exports {
        let source = read_u32(data, pos, "virtual fixup source")?;
        let section_index = read_u32(data, pos + 4, "virtual fixup section")?;
        let classname_offset = read_u32(data, pos + 8, "virtual fixup classname")?;
        if source == u32::MAX {
            break;
        }
        fixups.push(VirtualFixup {
            source,
            section: section_index,
            classname_offset,
        });
        pos += 12;
    }

    Ok(fixups)
}

// ─── Write ────────────────────────────────────────────────────────────────

/// Round up to next 16-byte boundary.
pub fn snap_to_16(pos: usize) -> usize {
    (pos.wrapping_add(0xF)) & !0xF
}

const HKX_MAGIC_BYTES: &[u8; 8] = HKX_MAGIC;

/// Write the packfile header.
///
/// v11 (FO4): 64-byte base + `padding_size` extension bytes.
/// v8 (Skyrim/FO3): always 64 bytes; `padding_size` is unused and 0x3E is not
/// the padding-size byte — it is part of the version-name string region.
pub fn write_header(header: &PackfileHeader) -> Vec<u8> {
    let total = if header.version == 8 {
        64
    } else {
        64 + header.padding_size
    };
    let mut buf = vec![0u8; total];

    buf[0..8].copy_from_slice(HKX_MAGIC_BYTES);
    buf[0x0C..0x10].copy_from_slice(&header.version.to_le_bytes());
    // [pointer_size=8, little_endian=1, reuse_padding=0, base_class_opt=1]
    buf[0x10..0x14].copy_from_slice(&0x01000108u32.to_le_bytes());
    // Number of sections = 3
    buf[0x14..0x18].copy_from_slice(&3u32.to_le_bytes());
    buf[0x18..0x1C].copy_from_slice(&header.contents_section_index.to_le_bytes());
    buf[0x1C..0x20].copy_from_slice(&header.contents_section_offset.to_le_bytes());
    buf[0x20..0x24].copy_from_slice(&header.contents_class_name_section_index.to_le_bytes());
    buf[0x24..0x28].copy_from_slice(&header.contents_class_name_section_offset.to_le_bytes());

    let name_bytes = header.version_name.as_bytes();
    let name_len = name_bytes.len().min(14);
    buf[0x28..0x28 + name_len].copy_from_slice(&name_bytes[..name_len]);

    if header.version == 11 {
        buf[0x36] = 0x00;
        buf[0x37] = 0xFF;
        buf[0x3C] = 0x15;
        buf[0x3D] = 0x00;
        buf[0x3E] = (header.padding_size & 0xFF) as u8;
        buf[0x3F] = 0x00;
        // v11 padding region: first byte is 0x14 in every vanilla FO4 packfile.
        if header.padding_size >= 16 {
            buf[0x40] = 0x14;
        }
    }

    buf
}

/// Write one v11 section header (64 bytes).
///
/// Use [`write_section_header_v8`] for v8 (Skyrim/FO3) packfiles.
pub fn write_section_header(section: &SectionHeader) -> Vec<u8> {
    let mut buf = vec![0u8; 64];
    // Bytes 0x30..0x3F are 0xFF in vanilla FO4 packfiles.
    for b in &mut buf[0x30..0x40] {
        *b = 0xFF;
    }
    write_section_header_body(&mut buf, section);
    buf
}

/// Write one v8 section header (48 bytes, no 0xFF trailer region).
pub fn write_section_header_v8(section: &SectionHeader) -> Vec<u8> {
    let mut buf = vec![0u8; 48];
    write_section_header_body(&mut buf, section);
    buf
}

fn write_section_header_body(buf: &mut [u8], section: &SectionHeader) {
    let name_bytes = section.name.as_bytes();
    let name_len = name_bytes.len().min(15);
    buf[0..name_len].copy_from_slice(&name_bytes[..name_len]);

    // Section header magic marker: 0xFF000000 (high byte 0xFF).
    buf[0x10..0x14].copy_from_slice(&0xFF000000u32.to_le_bytes());

    let offset = section.offset as u32;
    buf[0x14..0x18].copy_from_slice(&offset.to_le_bytes());
    let rel = |abs: usize| -> u32 { (abs - section.offset) as u32 };
    buf[0x18..0x1C].copy_from_slice(&rel(section.data1).to_le_bytes());
    buf[0x1C..0x20].copy_from_slice(&rel(section.data2).to_le_bytes());
    buf[0x20..0x24].copy_from_slice(&rel(section.data3).to_le_bytes());
    buf[0x24..0x28].copy_from_slice(&rel(section.exports).to_le_bytes());
    // imports == exports (vanilla content never uses the imports section).
    buf[0x28..0x2C].copy_from_slice(&rel(section.exports).to_le_bytes());
    buf[0x2C..0x30].copy_from_slice(&rel(section.end).to_le_bytes());
}

/// Write classnames section data.
pub fn write_classnames(entries: &[ClassnameEntry]) -> Vec<u8> {
    let mut buf = Vec::new();
    for entry in entries {
        buf.extend_from_slice(&entry.signature.to_le_bytes());
        buf.push(0x09);
        buf.extend_from_slice(entry.name.as_bytes());
        buf.push(0x00);
    }
    // Pad to 16-byte boundary with 0xFF.
    while buf.len() % 16 != 0 {
        buf.push(0xFF);
    }
    buf
}

/// Close a fixup table with the vanilla FO4 padding scheme.
///
/// Rule 3: if >= 8 bytes of padding are needed, write an 8-byte 0xFFFFFFFF
/// terminator; otherwise just pad with 0xFF bytes.
pub fn pad_fixup_table_to_16(buf: &mut Vec<u8>) {
    let pad_needed = buf.len().wrapping_neg() % 16;
    if pad_needed >= 8 {
        buf.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        buf.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    }
    while buf.len() % 16 != 0 {
        buf.push(0xFF);
    }
}

/// Write DATA1 local fixup table.
///
/// Rule 2: sort by (dst, src) — vanilla DATA1 order mirrors the data-section
/// emission order, not the source-pointer order.
pub fn write_local_fixups(fixups: &[(u32, u32)]) -> Vec<u8> {
    let mut sorted: Vec<(u32, u32)> = fixups.to_vec();
    sorted.sort_by_key(|&(src, dst)| (dst, src));
    let mut buf = Vec::with_capacity(sorted.len() * 8 + 16);
    for (src, dst) in sorted {
        buf.extend_from_slice(&src.to_le_bytes());
        buf.extend_from_slice(&dst.to_le_bytes());
    }
    pad_fixup_table_to_16(&mut buf);
    buf
}

/// Write DATA2 global fixup table.
pub fn write_global_fixups(fixups: &[(u32, u32, u32)]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(fixups.len() * 12 + 16);
    for (src, section, dst) in fixups {
        buf.extend_from_slice(&src.to_le_bytes());
        buf.extend_from_slice(&section.to_le_bytes());
        buf.extend_from_slice(&dst.to_le_bytes());
    }
    pad_fixup_table_to_16(&mut buf);
    buf
}

/// Write DATA3 virtual fixup table.
pub fn write_virtual_fixups(fixups: &[(u32, u32, u32)]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(fixups.len() * 12 + 16);
    for (src, section, cn_pos) in fixups {
        buf.extend_from_slice(&src.to_le_bytes());
        buf.extend_from_slice(&section.to_le_bytes());
        buf.extend_from_slice(&cn_pos.to_le_bytes());
    }
    pad_fixup_table_to_16(&mut buf);
    buf
}

fn find_section<'a>(sections: &'a [SectionHeader], name: &str) -> HavokResult<&'a SectionHeader> {
    sections
        .iter()
        .find(|section| section.name == name)
        .ok_or_else(|| HavokError::InvalidInput(format!("missing {name} section")))
}

fn ensure_len(data: &[u8], required: usize, label: &str) -> HavokResult<()> {
    if data.len() < required {
        return Err(HavokError::InvalidInput(format!(
            "{label} is too short: need {required} bytes, got {}",
            data.len()
        )));
    }
    Ok(())
}

fn checked_add(left: usize, right: usize, label: &str) -> HavokResult<usize> {
    left.checked_add(right)
        .ok_or_else(|| HavokError::InvalidInput(format!("{label} overflows")))
}

fn read_u32(data: &[u8], offset: usize, label: &str) -> HavokResult<u32> {
    ensure_len(data, offset + 4, label)?;
    Ok(u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

fn read_c_string(data: &[u8], start: usize, end: usize, label: &str) -> HavokResult<String> {
    ensure_len(data, end, label)?;
    let bytes = &data[start..end];
    let nul = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    decode_ascii(&bytes[..nul], label)
}

fn decode_ascii(bytes: &[u8], label: &str) -> HavokResult<String> {
    std::str::from_utf8(bytes)
        .map(str::to_string)
        .map_err(|error| HavokError::InvalidInput(format!("{label} is not UTF-8: {error}")))
}
