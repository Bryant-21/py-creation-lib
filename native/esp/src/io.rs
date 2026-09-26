use super::*;
use flate2::read::DeflateDecoder;

pub(crate) fn read_u16(data: &[u8], offset: usize) -> PyResult<u16> {
    if offset + 2 > data.len() {
        return Err(value_error(format!("u16 read past end at offset {offset}")));
    }
    Ok(u16::from_le_bytes([data[offset], data[offset + 1]]))
}

pub(crate) fn read_u32(data: &[u8], offset: usize) -> PyResult<u32> {
    if offset + 4 > data.len() {
        return Err(value_error(format!("u32 read past end at offset {offset}")));
    }
    Ok(u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

pub(crate) fn read_i32(data: &[u8], offset: usize) -> PyResult<i32> {
    Ok(read_u32(data, offset)? as i32)
}

pub(crate) fn detect_header_size(data: &[u8]) -> usize {
    if data.len() < MODERN_HEADER_SIZE {
        return MODERN_HEADER_SIZE;
    }
    if &data[..4] != b"TES4" {
        return MODERN_HEADER_SIZE;
    }
    let tes4_data_size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
    let pos24 = MODERN_HEADER_SIZE + tes4_data_size;
    let pos20 = LEGACY_HEADER_SIZE + tes4_data_size;
    if pos24 + 4 <= data.len() && &data[pos24..pos24 + 4] == b"GRUP" {
        return MODERN_HEADER_SIZE;
    }
    if pos20 + 4 <= data.len() && &data[pos20..pos20 + 4] == b"GRUP" {
        return LEGACY_HEADER_SIZE;
    }
    MODERN_HEADER_SIZE
}

pub(crate) fn decode_cp1252(data: &[u8]) -> String {
    let mut end = data.len();
    while end > 0 && data[end - 1] == 0 {
        end -= 1;
    }
    let (decoded, _, _) = WINDOWS_1252.decode(&data[..end]);
    decoded.into_owned()
}

pub(crate) fn encode_cp1252(text: &str, null_terminated: bool) -> Vec<u8> {
    let (encoded, _, _) = WINDOWS_1252.encode(text);
    let mut out = encoded.into_owned();
    if null_terminated {
        out.push(0);
    }
    out
}

pub(crate) fn parse_subrecords(payload: &Bytes) -> PyResult<Vec<ParsedSubrecord>> {
    parse_subrecords_from(payload, 0, payload.len())
}

/// Split a (decompressed) record payload into subrecords, honouring `XXXX`
/// size overrides — the same walk `parse_plugin_file` uses. Public so the
/// conversion crate's mmap-backed source store (`store2`) reuses the exact
/// loader semantics instead of duplicating the format walk.
pub fn split_record_payload(payload: &Bytes) -> Result<Vec<ParsedSubrecord>, String> {
    parse_subrecords(payload).map_err(|e| e.to_string())
}

/// Cheap forward scan counting how many `ParsedSubrecord`s the parse loop
/// below will push over `[start, end)`. Mirrors that loop's advancement so an
/// `XXXX` size-override prefix and its following subrecord count as one (they
/// produce a single push). No allocations — only the 4-byte signature + the
/// 2-byte (or `XXXX` 4-byte) size are read. Lets `parse_subrecords_from`
/// presize its Vec, killing the doubling cap-slack across 24.7M subrecords.
fn count_subrecords(data: &Bytes, start: usize, end: usize) -> usize {
    let mut count = 0usize;
    let mut offset = start;
    while offset + 6 <= end {
        if &data[offset..offset + 4] == b"XXXX" {
            if offset + 10 > end {
                break;
            }
            let real_size = u32::from_le_bytes([
                data[offset + 6],
                data[offset + 7],
                data[offset + 8],
                data[offset + 9],
            ]) as usize;
            offset += 10;
            if offset + 6 > end {
                break;
            }
            count += 1;
            offset = (offset + 6).saturating_add(real_size);
            continue;
        }
        let data_size = u16::from_le_bytes([data[offset + 4], data[offset + 5]]) as usize;
        count += 1;
        offset = (offset + 6).saturating_add(data_size);
    }
    count
}

pub(crate) fn parse_subrecords_from(
    data: &Bytes,
    start: usize,
    end: usize,
) -> PyResult<Vec<ParsedSubrecord>> {
    let mut subrecords = Vec::with_capacity(count_subrecords(data, start, end));
    let mut offset = start;
    while offset + 6 <= end {
        let signature = &data[offset..offset + 4];
        if signature == b"XXXX" {
            if offset + 10 > end {
                break;
            }
            let real_size = read_u32(data, offset + 6)? as usize;
            offset += 10;
            if offset + 6 > end {
                break;
            }
            let signature = SmolStr::new(String::from_utf8_lossy(&data[offset..offset + 4]));
            let data_offset = offset + 6;
            let data_end = data_offset.saturating_add(real_size).min(end);
            subrecords.push(ParsedSubrecord {
                signature,
                data: data.slice(data_offset..data_end),
                semantic_type: None,
            });
            offset = data_offset.saturating_add(real_size);
            continue;
        }
        let signature = SmolStr::new(String::from_utf8_lossy(signature));
        let data_size = read_u16(data, offset + 4)? as usize;
        let data_offset = offset + 6;
        let data_end = data_offset.saturating_add(data_size).min(end);
        subrecords.push(ParsedSubrecord {
            signature,
            data: data.slice(data_offset..data_end),
            semantic_type: None,
        });
        offset = data_offset.saturating_add(data_size);
    }
    Ok(subrecords)
}

pub struct DecodedCompressedSubrecords {
    pub subrecords: Vec<ParsedSubrecord>,
    pub salvaged_bad_checksum: bool,
}

fn inflate_compressed_record<R: Read>(
    mut decoder: R,
    declared_size: usize,
) -> Result<Vec<u8>, String> {
    let mut expanded = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    while expanded.len() <= declared_size {
        let remaining = declared_size + 1 - expanded.len();
        let read_len = remaining.min(chunk.len());
        let bytes_read = decoder
            .read(&mut chunk[..read_len])
            .map_err(|err| err.to_string())?;
        if bytes_read == 0 {
            break;
        }
        expanded.extend_from_slice(&chunk[..bytes_read]);
        if expanded.len() > declared_size {
            return Err(format!(
                "compressed record expanded past declared size {declared_size}"
            ));
        }
    }
    if expanded.len() != declared_size {
        return Err(format!(
            "compressed record expanded to {} bytes, declared size is {declared_size}",
            expanded.len()
        ));
    }
    Ok(expanded)
}

fn validate_complete_subrecord_framing(data: &[u8]) -> Result<(), String> {
    let mut offset = 0usize;
    while offset < data.len() {
        if data.len() - offset < 6 {
            return Err(format!(
                "compressed record has {} trailing byte(s) after complete subrecords",
                data.len() - offset
            ));
        }
        if &data[offset..offset + 4] == b"XXXX" {
            if data.len() - offset < 10 {
                return Err("compressed record has truncated XXXX size override".to_string());
            }
            let override_field_size = u16::from_le_bytes([data[offset + 4], data[offset + 5]]);
            if override_field_size != 4 {
                return Err(format!(
                    "compressed record XXXX size field is {override_field_size}, expected 4"
                ));
            }
            let real_size = u32::from_le_bytes([
                data[offset + 6],
                data[offset + 7],
                data[offset + 8],
                data[offset + 9],
            ]) as usize;
            offset += 10;
            if data.len() - offset < 6 {
                return Err(
                    "compressed record XXXX override is missing its subrecord header".to_string(),
                );
            }
            let placeholder_size = u16::from_le_bytes([data[offset + 4], data[offset + 5]]);
            if placeholder_size != 0 {
                return Err(format!(
                    "compressed record XXXX target size placeholder is {placeholder_size}, expected 0"
                ));
            }
            offset += 6;
            offset = offset.checked_add(real_size).ok_or_else(|| {
                "compressed record XXXX subrecord size overflows address space".to_string()
            })?;
        } else {
            let data_size = u16::from_le_bytes([data[offset + 4], data[offset + 5]]) as usize;
            offset += 6;
            offset = offset.checked_add(data_size).ok_or_else(|| {
                "compressed record subrecord size overflows address space".to_string()
            })?;
        }
        if offset > data.len() {
            return Err(format!(
                "compressed record subrecord extends {} byte(s) past payload end",
                offset - data.len()
            ));
        }
    }
    Ok(())
}

fn adler32(data: &[u8]) -> u32 {
    const MOD_ADLER: u64 = 65_521;
    let mut a = 1u64;
    let mut b = 0u64;
    for byte in data {
        a = (a + u64::from(*byte)) % MOD_ADLER;
        b = (b + a) % MOD_ADLER;
    }
    ((b as u32) << 16) | a as u32
}

fn salvage_bad_zlib_checksum(
    payload: &Bytes,
    declared_size: usize,
) -> Result<Vec<ParsedSubrecord>, String> {
    if payload.len() < 12 {
        return Err("compressed record is too short for a zlib wrapper".to_string());
    }
    let cmf = payload[4];
    let flg = payload[5];
    let header = (u16::from(cmf) << 8) | u16::from(flg);
    if cmf & 0x0F != 8 || cmf >> 4 > 7 || header % 31 != 0 || flg & 0x20 != 0 {
        return Err("compressed record has invalid or unsupported zlib framing".to_string());
    }

    let trailer_offset = payload.len() - 4;
    let inner_stream = &payload[6..trailer_offset];
    let mut decoder = DeflateDecoder::new(inner_stream);
    let expanded = inflate_compressed_record(&mut decoder, declared_size)?;
    if decoder.total_in() as usize != inner_stream.len() {
        return Err(format!(
            "compressed record raw DEFLATE stream left {} trailing byte(s)",
            inner_stream.len() - decoder.total_in() as usize
        ));
    }
    let stored_checksum = u32::from_be_bytes([
        payload[trailer_offset],
        payload[trailer_offset + 1],
        payload[trailer_offset + 2],
        payload[trailer_offset + 3],
    ]);
    if stored_checksum == adler32(&expanded) {
        return Err("compressed record has a valid Adler-32 checksum".to_string());
    }
    validate_complete_subrecord_framing(&expanded)?;
    parse_subrecords(&Bytes::from(expanded)).map_err(|err| err.to_string())
}

pub fn decode_compressed_subrecords_from_payload(
    payload: &Bytes,
) -> Result<DecodedCompressedSubrecords, String> {
    if payload.len() < 4 {
        return Err("compressed record missing size prefix".to_string());
    }
    const MAX_COMPRESSED_RECORD_EXPANDED_SIZE: usize = 512 * 1024 * 1024;
    let declared_size =
        u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    if declared_size > MAX_COMPRESSED_RECORD_EXPANDED_SIZE {
        return Err(format!(
            "compressed record declared size {declared_size} exceeds safety limit {MAX_COMPRESSED_RECORD_EXPANDED_SIZE}"
        ));
    }
    let strict_stream = &payload[4..];
    let mut strict_decoder = ZlibDecoder::new(strict_stream);
    let strict_result =
        inflate_compressed_record(&mut strict_decoder, declared_size).and_then(|expanded| {
            let consumed = strict_decoder.total_in() as usize;
            if consumed != strict_stream.len() {
                return Err(format!(
                    "compressed record zlib stream left {} trailing byte(s)",
                    strict_stream.len() - consumed
                ));
            }
            Ok(expanded)
        });
    match strict_result {
        Ok(expanded) => Ok(DecodedCompressedSubrecords {
            subrecords: parse_subrecords(&Bytes::from(expanded)).map_err(|err| err.to_string())?,
            salvaged_bad_checksum: false,
        }),
        Err(strict_error) => match salvage_bad_zlib_checksum(payload, declared_size) {
            Ok(subrecords) => Ok(DecodedCompressedSubrecords {
                subrecords,
                salvaged_bad_checksum: true,
            }),
            Err(salvage_error) => Err(format!(
                "{strict_error}; raw DEFLATE salvage rejected: {salvage_error}"
            )),
        },
    }
}

pub(crate) fn parse_compressed_subrecords_from_payload(
    payload: &Bytes,
) -> PyResult<Vec<ParsedSubrecord>> {
    decode_compressed_subrecords_from_payload(payload)
        .map(|parsed| parsed.subrecords)
        .map_err(value_error)
}

pub(crate) fn lazy_subrecords_for_record(
    record: &ParsedRecord,
) -> PyResult<Option<Vec<ParsedSubrecord>>> {
    if (record.flags & COMPRESSED_RECORD_FLAG) == 0 || !record.subrecords.is_empty() {
        return Ok(None);
    }
    let Some(raw_payload) = &record.raw_payload else {
        return Ok(None);
    };
    if record.parse_error.is_some() {
        return Ok(None);
    }
    decode_compressed_subrecords_from_payload(raw_payload)
        .map(|parsed| Some(parsed.subrecords))
        .map_err(value_error)
}

pub(crate) fn parse_record(
    data: &Bytes,
    offset: usize,
    header_size: usize,
    eager_compressed: bool,
) -> PyResult<(ParsedRecord, usize)> {
    if offset + header_size > data.len() {
        return Err(value_error("record header extends past end of file"));
    }
    let signature = SmolStr::new(String::from_utf8_lossy(&data[offset..offset + 4]));
    let data_size = read_u32(data, offset + 4)? as usize;
    let flags = read_u32(data, offset + 8)?;
    let form_id = read_u32(data, offset + 12)?;
    let version_control = read_u32(data, offset + 16)?;
    let (form_version, version2) = if header_size == LEGACY_HEADER_SIZE {
        (None, None)
    } else {
        (
            Some(read_u16(data, offset + 20)?),
            Some(read_u16(data, offset + 22)?),
        )
    };
    let payload_start = offset + header_size;
    let payload_end = payload_start + data_size;
    if payload_end > data.len() {
        return Err(value_error("record extends past end of file"));
    }
    let compressed = (flags & COMPRESSED_RECORD_FLAG) != 0;
    let (subrecords, raw_payload, parse_error) = if compressed {
        if data_size < 4 {
            return Err(value_error("compressed record missing size prefix"));
        }
        // raw_payload is a refcount slice into the source mmap/Bytes buffer,
        // so the compressed payload is *not* duplicated in memory. The
        // serializer (record_bytes_from_parsed) prefers raw_payload over
        // re-zlib because Bethesda's compressor settings aren't reproducible.
        let stored_payload = data.slice(payload_start..payload_end);
        if eager_compressed {
            match decode_compressed_subrecords_from_payload(&stored_payload) {
                Ok(parsed) => (
                    parsed.subrecords,
                    (!parsed.salvaged_bad_checksum).then_some(stored_payload),
                    None,
                ),
                Err(err) => (Vec::new(), Some(stored_payload), Some(err.to_string())),
            }
        } else {
            (Vec::new(), Some(stored_payload), None)
        }
    } else {
        (
            parse_subrecords_from(data, payload_start, payload_end)?,
            None,
            None,
        )
    };
    Ok((
        ParsedRecord {
            signature,
            form_id,
            flags,
            version_control,
            form_version,
            version2,
            subrecords,
            raw_payload,
            parse_error,
        },
        payload_end,
    ))
}

/// Cheap forward scan over a GRUP/root span to count how many top-level
/// items it contains. Used to pre-size `parse_children`'s Vec so we don't
/// burn ~192 MB on Vec doublings when a Starfield CELL/REFR group has
/// millions of children. The scan only reads each item's 4-byte signature
/// and 4-byte size — no allocations, no recursion into nested groups.
pub(crate) fn count_children(
    data: &[u8],
    offset: usize,
    end: usize,
    header_size: usize,
) -> usize {
    let mut count = 0usize;
    let mut cursor = offset;
    while cursor + header_size <= end {
        if cursor + 8 > data.len() {
            break;
        }
        let size = u32::from_le_bytes([
            data[cursor + 4],
            data[cursor + 5],
            data[cursor + 6],
            data[cursor + 7],
        ]) as usize;
        let next = if &data[cursor..cursor + 4] == b"GRUP" {
            cursor + size
        } else {
            cursor + header_size + size
        };
        if next <= cursor || next > end {
            break;
        }
        count += 1;
        cursor = next;
    }
    count
}

#[cfg(test)]
/// Header-only walk recording `form_id -> byte offset` for every record at
/// every nesting level, without building the `ParsedItem` tree. Thin wrapper
/// over [`crate::record_cursor::RecordCursor`], which owns the walk.
pub(crate) fn scan_record_offsets(
    data: &Bytes,
    offset: usize,
    end: usize,
    header_size: usize,
    out: &mut rustc_hash::FxHashMap<u32, usize>,
) {
    let cursor = crate::record_cursor::RecordCursor::new(data, header_size, offset);
    cursor.scan_within(end, &mut |view| {
        out.insert(view.form_id, view.offset);
        std::ops::ControlFlow::Continue(())
    });
}

pub(crate) fn parse_children(
    data: &Bytes,
    offset: usize,
    end: usize,
    header_size: usize,
    eager_compressed: bool,
) -> PyResult<(Vec<ParsedItem>, usize)> {
    let expected = count_children(data, offset, end, header_size);
    if expected > 1 && end.saturating_sub(offset) >= 1024 * 1024 {
        use rayon::prelude::*;

        let mut spans = Vec::with_capacity(expected);
        let mut cursor = offset;
        while cursor + header_size <= end {
            let is_group = &data[cursor..cursor + 4] == b"GRUP";
            let size = read_u32(data, cursor + 4)? as usize;
            let next = if is_group {
                cursor.saturating_add(size)
            } else {
                cursor.saturating_add(header_size).saturating_add(size)
            };
            if next <= cursor || next > end {
                return Err(value_error(format!(
                    "plugin item at {cursor} extends past parent boundary"
                )));
            }
            spans.push((cursor, is_group));
            cursor = next;
        }
        let items = spans
            .par_iter()
            .map(|(item_offset, is_group)| {
                if *is_group {
                    parse_group(data, *item_offset, header_size, eager_compressed)
                        .map(|(group, _)| ParsedItem::Group(group))
                } else {
                    parse_record(data, *item_offset, header_size, eager_compressed)
                        .map(|(record, _)| ParsedItem::Record(record))
                }
            })
            .collect::<PyResult<Vec<_>>>()?;
        return Ok((items, cursor));
    }
    let mut items = Vec::with_capacity(expected);
    let mut cursor = offset;
    while cursor + header_size <= end {
        if &data[cursor..cursor + 4] == b"GRUP" {
            let (group, next) = parse_group(data, cursor, header_size, eager_compressed)?;
            items.push(ParsedItem::Group(group));
            cursor = next;
        } else {
            let (record, next) = parse_record(data, cursor, header_size, eager_compressed)?;
            items.push(ParsedItem::Record(record));
            cursor = next;
        }
    }
    Ok((items, cursor))
}

pub(crate) fn parse_group(
    data: &Bytes,
    offset: usize,
    header_size: usize,
    eager_compressed: bool,
) -> PyResult<(ParsedGroup, usize)> {
    if offset + header_size > data.len() || &data[offset..offset + 4] != b"GRUP" {
        return Err(value_error(format!("expected GRUP at {offset}")));
    }
    let size = read_u32(data, offset + 4)? as usize;
    let mut label = [0u8; 4];
    label.copy_from_slice(&data[offset + 8..offset + 12]);
    let group_type = read_i32(data, offset + 12)?;
    let tail_end = offset + header_size;
    if tail_end > data.len() {
        return Err(value_error("group header extends past end of file"));
    }
    let tail = data.slice(offset + 16..tail_end);
    let end = offset + size;
    if end > data.len() {
        return Err(value_error("group extends past end of file"));
    }
    let (children, _) = parse_children(
        data,
        offset + header_size,
        end,
        header_size,
        eager_compressed,
    )?;
    Ok((
        ParsedGroup {
            label,
            group_type,
            tail,
            children,
        },
        end,
    ))
}

pub(crate) fn parse_plugin_header(record: &ParsedRecord) -> ParsedPluginHeader {
    let mut header = ParsedPluginHeader {
        version: 1.0,
        num_records: 0,
        next_object_id: 0x0800,
        author: String::new(),
        description: String::new(),
        masters: Vec::new(),
        master_sizes: Vec::new(),
        overridden_forms: Vec::new(),
        flags: record.flags,
        extra_subrecords: Vec::new(),
        version_control: record.version_control,
        form_version: record.form_version,
        version2: record.version2,
        hedr_raw: None,
        raw_subrecords: record.subrecords.clone(),
    };
    let mut last_master = false;
    for subrecord in &record.subrecords {
        match subrecord.signature.as_str() {
            "HEDR" if subrecord.data.len() >= 12 => {
                header.hedr_raw = Some(subrecord.data.clone());
                header.version = f32::from_le_bytes([
                    subrecord.data[0],
                    subrecord.data[1],
                    subrecord.data[2],
                    subrecord.data[3],
                ]);
                header.num_records = u32::from_le_bytes([
                    subrecord.data[4],
                    subrecord.data[5],
                    subrecord.data[6],
                    subrecord.data[7],
                ]);
                header.next_object_id = u32::from_le_bytes([
                    subrecord.data[8],
                    subrecord.data[9],
                    subrecord.data[10],
                    subrecord.data[11],
                ]);
            }
            "CNAM" => header.author = decode_cp1252(&subrecord.data),
            "SNAM" => header.description = decode_cp1252(&subrecord.data),
            "MAST" => {
                header.masters.push(decode_cp1252(&subrecord.data));
                last_master = true;
            }
            "DATA" if last_master && header.master_sizes.len() < header.masters.len() => {
                let mut padded = [0u8; 8];
                let take = subrecord.data.len().min(8);
                padded[..take].copy_from_slice(&subrecord.data[..take]);
                header.master_sizes.push(u64::from_le_bytes(padded));
                last_master = false;
            }
            "ONAM" if subrecord.data.len() % 4 == 0 => {
                header.overridden_forms = subrecord
                    .data
                    .chunks_exact(4)
                    .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect();
            }
            _ => header.extra_subrecords.push(subrecord.clone()),
        }
    }
    while header.master_sizes.len() < header.masters.len() {
        header.master_sizes.push(0);
    }
    header
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initialize_python_for_tests() {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            if std::env::var_os("PYTHONHOME").is_none() {
                if let Some(home) = option_env!("PYO3_TEST_PYTHON_HOME") {
                    unsafe {
                        std::env::set_var("PYTHONHOME", home);
                    }
                }
            }
            Python::initialize();
        });
    }

    #[test]
    fn split_record_payload_handles_plain_and_xxxx_subrecords() {
        // EDID "AB\0" + XXXX(4)=5 + DATA of 5 bytes
        let mut payload: Vec<u8> = Vec::new();
        payload.extend_from_slice(b"EDID");
        payload.extend_from_slice(&3u16.to_le_bytes());
        payload.extend_from_slice(b"AB\0");
        payload.extend_from_slice(b"XXXX");
        payload.extend_from_slice(&4u16.to_le_bytes());
        payload.extend_from_slice(&5u32.to_le_bytes());
        payload.extend_from_slice(b"DATA");
        payload.extend_from_slice(&0u16.to_le_bytes()); // size ignored, XXXX wins
        payload.extend_from_slice(&[1, 2, 3, 4, 5]);

        let subs = split_record_payload(&Bytes::from(payload)).expect("split ok");
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].signature.as_str(), "EDID");
        assert_eq!(subs[0].data.as_ref(), b"AB\0");
        assert_eq!(subs[1].signature.as_str(), "DATA");
        assert_eq!(subs[1].data.as_ref(), &[1, 2, 3, 4, 5]);
    }

    fn compressed_payload(declared_size: u32, expanded: &[u8]) -> Bytes {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(expanded).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut payload = Vec::with_capacity(4 + compressed.len());
        payload.extend_from_slice(&declared_size.to_le_bytes());
        payload.extend_from_slice(&compressed);
        Bytes::from(payload)
    }

    fn corrupt_adler_checksum(payload: Bytes) -> Bytes {
        let mut payload = payload.to_vec();
        let last = payload.len() - 1;
        payload[last] ^= 0x01;
        Bytes::from(payload)
    }

    fn test_subrecord(signature: &str, data: Vec<u8>) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: SmolStr::new(signature),
            data: Bytes::from(data),
            semantic_type: None,
        }
    }

    fn land_shape_subrecords() -> Vec<ParsedSubrecord> {
        vec![
            test_subrecord("DATA", vec![0; 4]),
            test_subrecord("VNML", vec![0; 3267]),
            test_subrecord("VHGT", vec![0; 1096]),
        ]
    }

    fn legacy_compressed_record_bytes(signature: &str, form_id: u32, payload: &Bytes) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(LEGACY_HEADER_SIZE + payload.len());
        bytes.extend_from_slice(signature.as_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&COMPRESSED_RECORD_FLAG.to_le_bytes());
        bytes.extend_from_slice(&form_id.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn temp_plugin_path(test_name: &str) -> PathBuf {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "modkit21-{test_name}-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join("SeventySix.esm")
    }

    fn localized_subrecord(signature: &str, string_id: u32) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: SmolStr::new(signature),
            data: Bytes::from(string_id.to_le_bytes().to_vec()),
            semantic_type: None,
        }
    }

    fn localized_test_plugin_for_record(
        record_signature: &str,
        subrecords: Vec<ParsedSubrecord>,
    ) -> ParsedPlugin {
        let mut header = ParsedPluginHeader::default_for_test();
        header.flags = TES4_FLAG_LOCALIZED;
        ParsedPlugin {
            plugin_name: "SeventySix.esm".to_string(),
            file_path: String::new(),
            header_size: MODERN_HEADER_SIZE,
            header,
            root_items: vec![ParsedItem::Record(ParsedRecord {
                signature: SmolStr::new(record_signature),
                form_id: 0x0700_0800,
                flags: 0,
                version_control: 0,
                form_version: Some(131),
                version2: None,
                subrecords,
                raw_payload: None,
                parse_error: None,
            })],
            game: Some("fo4".to_string()),
        }
    }

    fn localized_test_plugin(subrecords: Vec<ParsedSubrecord>) -> ParsedPlugin {
        localized_test_plugin_for_record("BOOK", subrecords)
    }

    fn parity_record(sig: &str, form_id: u32, edid: &str) -> ParsedRecord {
        ParsedRecord {
            signature: SmolStr::new(sig),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: Some(131),
            version2: Some(0),
            subrecords: vec![ParsedSubrecord {
                signature: SmolStr::new("EDID"),
                data: Bytes::from({
                    let mut v = edid.as_bytes().to_vec();
                    v.push(0);
                    v
                }),
                semantic_type: None,
            }],
            raw_payload: None,
            parse_error: None,
        }
    }

    /// A top-level WRLD group with world children, a CELL, and cell children,
    /// plus a sibling WEAP group. This exercises real FO4 nesting and ordering.
    fn parity_multi_group_plugin() -> ParsedPlugin {
        let header = ParsedPluginHeader::default_for_test();
        let weap_group = ParsedGroup {
            label: *b"WEAP",
            group_type: 0,
            tail: Bytes::from(vec![0u8; MODERN_HEADER_SIZE - 16]),
            children: vec![
                ParsedItem::Record(parity_record("WEAP", 0x0700_0801, "WeapA")),
                ParsedItem::Record(parity_record("WEAP", 0x0700_0802, "WeapB")),
            ],
        };
        let inner = ParsedGroup {
            label: 0x0700_0900u32.to_le_bytes(),
            group_type: 6,
            tail: Bytes::from(vec![0u8; MODERN_HEADER_SIZE - 16]),
            children: vec![ParsedItem::Record(parity_record(
                "REFR",
                0xFF00_0901,
                "RefrA",
            ))],
        };
        let cell_children = ParsedGroup {
            label: 0x0700_0900u32.to_le_bytes(),
            group_type: 6,
            tail: Bytes::from(vec![0u8; MODERN_HEADER_SIZE - 16]),
            children: vec![ParsedItem::Group(inner)],
        };
        let world_children = ParsedGroup {
            label: 0x0700_0800u32.to_le_bytes(),
            group_type: 1,
            tail: Bytes::from(vec![0u8; MODERN_HEADER_SIZE - 16]),
            children: vec![
                ParsedItem::Record(parity_record("CELL", 0x0700_0900, "CellA")),
                ParsedItem::Group(cell_children),
            ],
        };
        let world_group = ParsedGroup {
            label: *b"WRLD",
            group_type: 0,
            tail: Bytes::from(vec![0u8; MODERN_HEADER_SIZE - 16]),
            children: vec![
                ParsedItem::Record(parity_record("WRLD", 0x0700_0800, "WorldA")),
                ParsedItem::Group(world_children),
            ],
        };
        ParsedPlugin {
            plugin_name: "Out.esm".to_string(),
            file_path: String::new(),
            header_size: MODERN_HEADER_SIZE,
            header,
            root_items: vec![
                ParsedItem::Group(weap_group),
                ParsedItem::Group(world_group),
            ],
            game: Some("fo4".to_string()),
        }
    }

    fn assert_group_lengths(bytes: &[u8], header_size: usize, start: usize, end: usize) -> usize {
        let mut offset = start;
        let mut groups = 0;
        while offset < end {
            assert!(offset + header_size <= end, "truncated item at {offset}");
            let payload_size =
                u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
            if &bytes[offset..offset + 4] == b"GRUP" {
                let group_end = offset
                    .checked_add(payload_size)
                    .expect("group end overflow");
                assert!(payload_size >= header_size, "undersized GRUP at {offset}");
                assert!(group_end <= end, "GRUP at {offset} exceeds its parent");
                groups +=
                    1 + assert_group_lengths(bytes, header_size, offset + header_size, group_end);
                offset = group_end;
            } else {
                offset = offset
                    .checked_add(header_size)
                    .and_then(|value| value.checked_add(payload_size))
                    .expect("record end overflow");
                assert!(offset <= end, "record exceeds its parent");
            }
        }
        assert_eq!(offset, end, "items did not end on their parent boundary");
        groups
    }

    fn assert_plugin_group_lengths(bytes: &[u8], header_size: usize) -> usize {
        let tes4_payload = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        assert_group_lengths(bytes, header_size, header_size + tes4_payload, bytes.len())
    }

    fn has_temporary_self_form_id(items: &[ParsedItem]) -> bool {
        items.iter().any(|item| match item {
            ParsedItem::Record(record) => record.form_id >> 24 == 0xFF,
            ParsedItem::Group(group) => has_temporary_self_form_id(&group.children),
        })
    }

    #[test]
    fn streaming_writer_matches_buffered_writer() {
        // streaming_writer_byte_matches_buffered_build_plugin_bytes
        {
            // The streaming serializer must produce byte-identical output to the
            // buffered build_plugin_bytes — same framing, same order, only the sink
            // differs. This is the parity guarantee for the Build-ESP memory win.
            let mut buffered_plugin = parity_multi_group_plugin();
            let buffered = build_plugin_bytes(&mut buffered_plugin).expect("buffered serialize");

            let mut streamed_plugin = parity_multi_group_plugin();
            let mut streamed = std::io::Cursor::new(Vec::new());
            write_plugin_seekable_to(&mut streamed_plugin, &mut streamed).expect("streamed serialize");
            let streamed = streamed.into_inner();

            assert_eq!(
                streamed, buffered,
                "streaming serializer diverged from buffered build_plugin_bytes"
            );
            assert!(!has_temporary_self_form_id(&streamed_plugin.root_items));
            assert!(!has_temporary_self_form_id(&buffered_plugin.root_items));
            assert_eq!(
                assert_plugin_group_lengths(&streamed, MODERN_HEADER_SIZE),
                5
            );

        }
        // streaming_writer_matches_legacy_headers
        {
            let mut buffered_plugin = parity_multi_group_plugin();
            buffered_plugin.header_size = LEGACY_HEADER_SIZE;
            let buffered = build_plugin_bytes(&mut buffered_plugin).expect("buffered legacy serialize");

            let mut streamed_plugin = parity_multi_group_plugin();
            streamed_plugin.header_size = LEGACY_HEADER_SIZE;
            let mut streamed = std::io::Cursor::new(Vec::new());
            write_plugin_seekable_to(&mut streamed_plugin, &mut streamed)
                .expect("streamed legacy serialize");
            let streamed = streamed.into_inner();

            assert_eq!(streamed, buffered);
            assert_eq!(
                assert_plugin_group_lengths(&streamed, LEGACY_HEADER_SIZE),
                5
            );

        }
        // streaming_writer_matches_compressed_raw_salvage_and_xxxx_records
        {
            initialize_python_for_tests();
            let plain = encode_subrecords_uncompressed(&[
                test_subrecord("EDID", b"CompressedRecord\0".to_vec()),
                test_subrecord("DATA", vec![7; 4096]),
            ]);
            let valid_raw = compressed_payload(plain.len() as u32, &plain);
            let bad_checksum = corrupt_adler_checksum(valid_raw.clone());

            let make_compressed = |form_id, raw_payload, subrecords| ParsedRecord {
                signature: SmolStr::new("MISC"),
                form_id,
                flags: COMPRESSED_RECORD_FLAG,
                version_control: 0,
                form_version: Some(131),
                version2: Some(0),
                subrecords,
                raw_payload,
                parse_error: None,
            };
            let mut plugin = parity_multi_group_plugin();
            let ParsedItem::Group(first_group) = &mut plugin.root_items[0] else {
                unreachable!()
            };
            first_group.children.extend([
                ParsedItem::Record(make_compressed(0x0700_0810, Some(valid_raw), Vec::new())),
                ParsedItem::Record(make_compressed(0x0700_0811, Some(bad_checksum), Vec::new())),
                ParsedItem::Record(make_compressed(
                    0x0700_0812,
                    None,
                    vec![test_subrecord("DATA", vec![9; 8192])],
                )),
                ParsedItem::Record(ParsedRecord {
                    signature: SmolStr::new("MISC"),
                    form_id: 0x0700_0813,
                    flags: 0,
                    version_control: 0,
                    form_version: Some(131),
                    version2: Some(0),
                    subrecords: vec![test_subrecord("DATA", vec![3; 70_000])],
                    raw_payload: None,
                    parse_error: None,
                }),
            ]);

            let mut buffered_plugin = plugin.clone();
            let buffered =
                build_plugin_bytes(&mut buffered_plugin).expect("buffered special serialize");
            let mut streamed_plugin = plugin;
            let mut streamed = std::io::Cursor::new(Vec::new());
            write_plugin_seekable_to(&mut streamed_plugin, &mut streamed)
                .expect("streamed special serialize");
            let streamed = streamed.into_inner();

            assert_eq!(streamed, buffered);
            assert_eq!(
                assert_plugin_group_lengths(&streamed, MODERN_HEADER_SIZE),
                5
            );
            assert!(streamed.windows(4).any(|window| window == b"XXXX"));

        }
        // streaming_writer_defers_only_groups_larger_than_its_buffer
        {
            let mut buffered_plugin = parity_multi_group_plugin();
            let buffered = build_plugin_bytes(&mut buffered_plugin).expect("buffered serialize");
            let mut streamed_plugin = parity_multi_group_plugin();
            let mut streamed = std::io::Cursor::new(Vec::new());

            let deferred = write_plugin_to_with_buffer(&mut streamed_plugin, &mut streamed, 64)
                .expect("small-buffer serialize");

            assert!(
                deferred > 0,
                "fixture did not exercise deferred group patches"
            );
            assert_eq!(streamed.into_inner(), buffered);

        }
        // streaming_writer_matches_across_record_batches_with_groups_open
        {
            let record_count = crate::default_job_count() * STREAM_RECORD_BATCH_RECORDS_PER_JOB + 1;
            let leaf_groups = (0..record_count)
                .map(|index| {
                    ParsedItem::Group(ParsedGroup {
                        label: (0x0701_0000u32 + index as u32).to_le_bytes(),
                        group_type: 6,
                        tail: Bytes::from(vec![0u8; MODERN_HEADER_SIZE - 16]),
                        children: vec![ParsedItem::Record(parity_record(
                            "REFR",
                            0x0702_0000u32 + index as u32,
                            "BatchRecord",
                        ))],
                    })
                })
                .collect();
            let mut plugin = parity_multi_group_plugin();
            plugin.root_items = vec![ParsedItem::Group(ParsedGroup {
                label: *b"WRLD",
                group_type: 0,
                tail: Bytes::from(vec![0u8; MODERN_HEADER_SIZE - 16]),
                children: vec![ParsedItem::Group(ParsedGroup {
                    label: 0x0700_0800u32.to_le_bytes(),
                    group_type: 1,
                    tail: Bytes::from(vec![0u8; MODERN_HEADER_SIZE - 16]),
                    children: leaf_groups,
                })],
            })];

            let mut buffered_plugin = plugin.clone();
            let buffered = build_plugin_bytes(&mut buffered_plugin).expect("buffered batch serialize");
            let mut streamed = std::io::Cursor::new(Vec::new());
            write_plugin_seekable_to(&mut plugin, &mut streamed).expect("streamed batch serialize");
            let streamed = streamed.into_inner();

            assert_eq!(streamed, buffered);
            assert_eq!(
                assert_plugin_group_lengths(&streamed, MODERN_HEADER_SIZE),
                record_count + 2
            );

        }
    }

    fn unique_temp_dir(test_name: &str) -> PathBuf {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "modkit21-{test_name}-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_plugin_atomic_commits_full_bytes_and_leaves_no_temp() {
        let dir = unique_temp_dir("atomic-ok");
        let target = dir.join("Out.esp");
        let body = b"TES4-COMPLETE-PLUGIN-BODY".to_vec();

        write_plugin_atomic(target.to_str().unwrap(), |writer| {
            std::io::Write::write_all(writer, &body)
        })
        .expect("save ok");

        assert_eq!(fs::read(&target).unwrap(), body, "saved bytes incomplete");
        // No leftover NamedTempFile siblings in the directory.
        let strays: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|name| name != "Out.esp")
            .collect();
        assert!(strays.is_empty(), "temp file leaked: {strays:?}");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn write_plugin_atomic_failure_preserves_existing_file() {
        // The core persistence guarantee: if the serialize step fails *after*
        // the write begins, the previous good file on disk must be left intact
        // (the old in-place `File::create` truncated the target to 0 first, so a
        // failed write destroyed the plugin while callers reported success).
        let dir = unique_temp_dir("atomic-fail");
        let target = dir.join("Good.esp");
        let good_bytes = b"TES4-GOOD-EXISTING-PLUGIN-BYTES".to_vec();
        fs::write(&target, &good_bytes).unwrap();

        let result = write_plugin_atomic(target.to_str().unwrap(), |writer| {
            std::io::Write::write_all(writer, &[0xCC; 4096])?;
            std::io::Seek::seek(writer, std::io::SeekFrom::Start(4))?;
            std::io::Write::write_all(writer, &128u32.to_le_bytes())?;
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "injected late group-size patch failure",
            ))
        });

        assert!(result.is_err(), "save must surface the write failure");
        assert_eq!(
            fs::read(&target).unwrap(),
            good_bytes,
            "existing plugin was destroyed by a failed save"
        );
        let strays: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|name| name != "Good.esp")
            .collect();
        assert!(strays.is_empty(), "temp file leaked on failure: {strays:?}");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn compressed_bad_adler_salvage_and_resave() {
        // compressed_bad_adler_land_payload_is_salvaged
        {
            initialize_python_for_tests();
            let expanded = encode_subrecords_uncompressed(&land_shape_subrecords());
            assert_eq!(expanded.len(), 4385);
            let payload = corrupt_adler_checksum(compressed_payload(expanded.len() as u32, &expanded));

            let parsed = decode_compressed_subrecords_from_payload(&payload).expect("salvage LAND");

            assert!(parsed.salvaged_bad_checksum);
            assert_eq!(
                parsed
                    .subrecords
                    .iter()
                    .map(|subrecord| (subrecord.signature.as_str(), subrecord.data.len()))
                    .collect::<Vec<_>>(),
                vec![("DATA", 4), ("VNML", 3267), ("VHGT", 1096)]
            );

        }
        // compressed_bad_adler_xxxx_payload_is_salvaged
        {
            initialize_python_for_tests();
            let mut expanded = Vec::new();
            expanded.extend_from_slice(b"XXXX");
            expanded.extend_from_slice(&4u16.to_le_bytes());
            expanded.extend_from_slice(&5u32.to_le_bytes());
            expanded.extend_from_slice(b"DATA");
            expanded.extend_from_slice(&0u16.to_le_bytes());
            expanded.extend_from_slice(&[1, 2, 3, 4, 5]);
            let payload = corrupt_adler_checksum(compressed_payload(expanded.len() as u32, &expanded));

            let parsed = decode_compressed_subrecords_from_payload(&payload).expect("salvage XXXX");

            assert!(parsed.salvaged_bad_checksum);
            assert_eq!(parsed.subrecords.len(), 1);
            assert_eq!(parsed.subrecords[0].signature.as_str(), "DATA");
            assert_eq!(parsed.subrecords[0].data.as_ref(), &[1, 2, 3, 4, 5]);

        }
        // compressed_bad_adler_record_recompresses_canonically_on_save
        {
            initialize_python_for_tests();
            let expanded = encode_subrecords_uncompressed(&land_shape_subrecords());
            let bad_payload =
                corrupt_adler_checksum(compressed_payload(expanded.len() as u32, &expanded));
            let source_bytes = legacy_compressed_record_bytes("LAND", 0x0015_0FC0, &bad_payload);

            let (salvaged, next) = parse_record(
                &Bytes::from(source_bytes.clone()),
                0,
                LEGACY_HEADER_SIZE,
                true,
            )
            .expect("parse salvageable record");
            assert_eq!(next, source_bytes.len());
            assert!(salvaged.raw_payload.is_none());
            assert!(salvaged.parse_error.is_none());
            assert_eq!(salvaged.subrecords.len(), 3);

            let canonical = record_bytes_from_parsed(&salvaged, LEGACY_HEADER_SIZE).unwrap();
            let canonical_payload = Bytes::copy_from_slice(&canonical[LEGACY_HEADER_SIZE..]);
            assert_ne!(canonical_payload, bad_payload);
            let strict = decode_compressed_subrecords_from_payload(&canonical_payload)
                .expect("canonical strict zlib decode");
            assert!(!strict.salvaged_bad_checksum);

            let (reloaded, next) =
                parse_record(&Bytes::from(canonical.clone()), 0, LEGACY_HEADER_SIZE, true)
                    .expect("reload canonical record");
            assert_eq!(next, canonical.len());
            assert_eq!(reloaded.raw_payload.as_ref(), Some(&canonical_payload));
            assert!(reloaded.parse_error.is_none());
            assert_eq!(reloaded.subrecords.len(), 3);

        }
        // compressed_bad_adler_lazy_record_recompresses_canonically_on_save
        {
            initialize_python_for_tests();
            let expanded = encode_subrecords_uncompressed(&land_shape_subrecords());
            let bad_payload =
                corrupt_adler_checksum(compressed_payload(expanded.len() as u32, &expanded));
            let source_bytes = legacy_compressed_record_bytes("LAND", 0x0015_0FC0, &bad_payload);

            let (lazy, next) = parse_record(
                &Bytes::from(source_bytes.clone()),
                0,
                LEGACY_HEADER_SIZE,
                false,
            )
            .expect("parse lazy salvageable record");
            assert_eq!(next, source_bytes.len());
            assert!(lazy.subrecords.is_empty());
            assert_eq!(lazy.raw_payload.as_ref(), Some(&bad_payload));

            let canonical = record_bytes_from_parsed(&lazy, LEGACY_HEADER_SIZE).unwrap();
            let canonical_payload = Bytes::copy_from_slice(&canonical[LEGACY_HEADER_SIZE..]);
            assert_ne!(canonical_payload, bad_payload);
            let strict = decode_compressed_subrecords_from_payload(&canonical_payload)
                .expect("lazy save canonical strict zlib decode");
            assert!(!strict.salvaged_bad_checksum);
            assert_eq!(strict.subrecords.len(), 3);

        }
        // compressed_strict_record_preserves_original_raw_payload
        {
            initialize_python_for_tests();
            let expanded = encode_subrecords_uncompressed(&[test_subrecord("DATA", vec![1])]);
            let payload = compressed_payload(expanded.len() as u32, &expanded);
            let source_record = ParsedRecord {
                signature: SmolStr::new("LAND"),
                form_id: 0x0015_0FC0,
                flags: COMPRESSED_RECORD_FLAG,
                version_control: 0,
                form_version: None,
                version2: None,
                subrecords: Vec::new(),
                raw_payload: Some(payload.clone()),
                parse_error: None,
            };
            let source_bytes = record_bytes_from_parsed(&source_record, LEGACY_HEADER_SIZE).unwrap();

            let (parsed, _) = parse_record(
                &Bytes::from(source_bytes.clone()),
                0,
                LEGACY_HEADER_SIZE,
                true,
            )
            .expect("parse strict record");

            assert_eq!(parsed.raw_payload.as_ref(), Some(&payload));
            assert_eq!(
                record_bytes_from_parsed(&parsed, LEGACY_HEADER_SIZE).unwrap(),
                source_bytes
            );

        }
    }

    #[test]
    fn compressed_salvage_rejects_malformed_payloads() {
        // compressed_salvage_rejects_xxxx_size_other_than_four
        {
            initialize_python_for_tests();
            let mut expanded = Vec::new();
            expanded.extend_from_slice(b"XXXX");
            expanded.extend_from_slice(&3u16.to_le_bytes());
            expanded.extend_from_slice(&5u32.to_le_bytes());
            expanded.extend_from_slice(b"DATA");
            expanded.extend_from_slice(&0u16.to_le_bytes());
            expanded.extend_from_slice(&[1, 2, 3, 4, 5]);
            let payload = corrupt_adler_checksum(compressed_payload(expanded.len() as u32, &expanded));

            let err = match decode_compressed_subrecords_from_payload(&payload) {
                Ok(_) => panic!("invalid XXXX size unexpectedly salvaged"),
                Err(err) => err,
            };

            assert!(err.contains("XXXX size field is 3, expected 4"));

        }
        // compressed_salvage_rejects_nonzero_xxxx_placeholder
        {
            initialize_python_for_tests();
            let mut expanded = Vec::new();
            expanded.extend_from_slice(b"XXXX");
            expanded.extend_from_slice(&4u16.to_le_bytes());
            expanded.extend_from_slice(&5u32.to_le_bytes());
            expanded.extend_from_slice(b"DATA");
            expanded.extend_from_slice(&1u16.to_le_bytes());
            expanded.extend_from_slice(&[1, 2, 3, 4, 5]);
            let payload = corrupt_adler_checksum(compressed_payload(expanded.len() as u32, &expanded));

            let err = match decode_compressed_subrecords_from_payload(&payload) {
                Ok(_) => panic!("nonzero XXXX placeholder unexpectedly salvaged"),
                Err(err) => err,
            };

            assert!(err.contains("XXXX target size placeholder is 1, expected 0"));

        }
        // compressed_strict_zlib_rejects_trailing_compressed_bytes
        {
            initialize_python_for_tests();
            let expanded = encode_subrecords_uncompressed(&[test_subrecord("DATA", vec![1])]);
            let mut payload = compressed_payload(expanded.len() as u32, &expanded).to_vec();
            payload.extend_from_slice(&[0xAA, 0xBB, 0xCC]);

            let err = match decode_compressed_subrecords_from_payload(&Bytes::from(payload)) {
                Ok(_) => panic!("zlib stream with trailing bytes unexpectedly parsed"),
                Err(err) => err,
            };

            assert!(err.contains("trailing byte"));

        }
        // compressed_salvage_rejects_invalid_zlib_header
        {
            initialize_python_for_tests();
            let expanded = encode_subrecords_uncompressed(&[test_subrecord("DATA", vec![1])]);
            let mut payload =
                corrupt_adler_checksum(compressed_payload(expanded.len() as u32, &expanded)).to_vec();
            payload[4] = 0x79;

            let err = match parse_compressed_subrecords_from_payload(&Bytes::from(payload)) {
                Ok(_) => panic!("invalid zlib header unexpectedly salvaged"),
                Err(err) => err,
            };

            assert!(
                err.to_string()
                    .contains("invalid or unsupported zlib framing")
            );

        }
        // compressed_salvage_rejects_invalid_deflate_body
        {
            initialize_python_for_tests();
            let expanded = encode_subrecords_uncompressed(&[test_subrecord("DATA", vec![1])]);
            let mut payload = compressed_payload(expanded.len() as u32, &expanded).to_vec();
            payload[6] = (payload[6] & 0xF8) | 0x07;

            assert!(parse_compressed_subrecords_from_payload(&Bytes::from(payload)).is_err());

        }
        // compressed_salvage_rejects_wrong_declared_size
        {
            initialize_python_for_tests();
            let expanded = encode_subrecords_uncompressed(&[test_subrecord("DATA", vec![1])]);
            let mut payload = compressed_payload(expanded.len() as u32, &expanded).to_vec();
            payload[..4].copy_from_slice(&((expanded.len() + 1) as u32).to_le_bytes());

            let err = match parse_compressed_subrecords_from_payload(&Bytes::from(payload)) {
                Ok(_) => panic!("wrong declared size unexpectedly salvaged"),
                Err(err) => err,
            };

            assert!(err.to_string().contains("declared size"));

        }
        // compressed_salvage_rejects_malformed_subrecord_framing
        {
            initialize_python_for_tests();
            let expanded = b"DATA\x04\0abc";
            let payload = corrupt_adler_checksum(compressed_payload(expanded.len() as u32, expanded));

            let err = match parse_compressed_subrecords_from_payload(&payload) {
                Ok(_) => panic!("malformed subrecord unexpectedly salvaged"),
                Err(err) => err,
            };

            assert!(err.to_string().contains("past payload end"));

        }
        // compressed_salvage_rejects_trailing_subrecord_bytes
        {
            initialize_python_for_tests();
            let mut expanded = encode_subrecords_uncompressed(&[test_subrecord("DATA", vec![1])]);
            expanded.push(0xAA);
            let payload = corrupt_adler_checksum(compressed_payload(expanded.len() as u32, &expanded));

            let err = match parse_compressed_subrecords_from_payload(&payload) {
                Ok(_) => panic!("trailing subrecord bytes unexpectedly salvaged"),
                Err(err) => err,
            };

            assert!(err.to_string().contains("trailing byte"));

        }
        // compressed_subrecords_reject_expansion_past_declared_size
        {
            initialize_python_for_tests();
            let expanded = b"EDID\x04\0Test";
            let payload = compressed_payload((expanded.len() - 1) as u32, expanded);

            let err = match parse_compressed_subrecords_from_payload(&payload) {
                Ok(_) => panic!("compressed payload unexpectedly parsed"),
                Err(err) => err,
            };

            assert!(err.to_string().contains("declared size"));

        }
    }

    #[test]
    fn compress_decompress_roundtrip() {
        initialize_python_for_tests();
        let subrecords = vec![ParsedSubrecord {
            signature: SmolStr::new("EDID"),
            data: Bytes::from(b"Test\0".to_vec()),
            semantic_type: None,
        }];
        let compressed = compress_subrecords_payload(&subrecords).expect("compress");
        // First 4 bytes = u32 declared decompressed size
        let declared =
            u32::from_le_bytes([compressed[0], compressed[1], compressed[2], compressed[3]]);
        assert!(declared > 0);
        let decompressed =
            parse_compressed_subrecords_from_payload(&Bytes::from(compressed.clone()))
                .expect("roundtrip");
        assert_eq!(decompressed.len(), 1);
        assert_eq!(decompressed[0].signature.as_str(), "EDID");
        assert_eq!(decompressed[0].data.as_ref(), b"Test\0");
    }

    #[test]
    fn localized_string_save_tables() {
        // localized_string_save_writes_referenced_id_to_each_required_table
        {
            let string_id = 0x110;
            let plugin = localized_test_plugin(vec![
                localized_subrecord("FULL", string_id),
                localized_subrecord("DESC", string_id),
            ]);
            let mut strings = LocalizedStringsState {
                default_language: "en".to_string(),
                ..LocalizedStringsState::default()
            };
            strings
                .by_language
                .entry("en".to_string())
                .or_default()
                .insert(string_id, "Shared text".to_string());
            strings.table_types.insert(string_id, "strings".to_string());
            let output_path = temp_plugin_path("localized-collision");
            let root = output_path.parent().unwrap().to_path_buf();

            write_localized_strings_for_parsed(&plugin, &strings, output_path.to_str().unwrap())
                .unwrap();

            let strings_values =
                strings::parse_string_table(&root.join("Strings").join("SeventySix_en.STRINGS"))
                    .unwrap();
            let dlstrings_values =
                strings::parse_string_table(&root.join("Strings").join("SeventySix_en.DLSTRINGS"))
                    .unwrap();
            assert_eq!(
                strings_values.get(&string_id).map(String::as_str),
                Some("Shared text")
            );
            assert_eq!(
                dlstrings_values.get(&string_id).map(String::as_str),
                Some("Shared text")
            );
            fs::remove_dir_all(root).unwrap();

        }
        // localized_string_save_ignores_plugin_extension_in_temp_suffix
        {
            let string_id = 0x110;
            let plugin = localized_test_plugin(vec![localized_subrecord("FULL", string_id)]);
            let mut strings = LocalizedStringsState {
                default_language: "en".to_string(),
                ..LocalizedStringsState::default()
            };
            strings
                .by_language
                .entry("en".to_string())
                .or_default()
                .insert(string_id, "Shared text".to_string());
            let output_path = temp_plugin_path("localized-temp-suffix").with_extension("esm.tmp");
            let root = output_path.parent().unwrap().to_path_buf();

            write_localized_strings_for_parsed(&plugin, &strings, output_path.to_str().unwrap())
                .unwrap();

            assert!(root.join("Strings").join("SeventySix_en.STRINGS").is_file());
            assert!(
                !root
                    .join("Strings")
                    .join("SeventySix.esm_en.STRINGS")
                    .exists()
            );
            fs::remove_dir_all(root).unwrap();

        }
        // localized_string_save_writes_placeholder_when_string_state_is_empty
        {
            let string_id = 0xBA;
            let plugin = localized_test_plugin(vec![localized_subrecord("DESC", string_id)]);
            let strings = LocalizedStringsState::default();
            let output_path = temp_plugin_path("localized-placeholder");
            let root = output_path.parent().unwrap().to_path_buf();

            write_localized_strings_for_parsed(&plugin, &strings, output_path.to_str().unwrap())
                .unwrap();

            let dlstrings_values =
                strings::parse_string_table(&root.join("Strings").join("SeventySix_en.DLSTRINGS"))
                    .unwrap();
            assert_eq!(
                dlstrings_values.get(&string_id).map(String::as_str),
                Some("LOC_000000BA")
            );
            fs::remove_dir_all(root).unwrap();

        }
        // localized_string_save_uses_schema_for_non_legacy_lstring_signature
        {
            let string_id = 0x423DB;
            let plugin =
                localized_test_plugin_for_record("MGEF", vec![localized_subrecord("DNAM", string_id)]);
            let strings = LocalizedStringsState::default();
            let output_path = temp_plugin_path("localized-schema");
            let root = output_path.parent().unwrap().to_path_buf();

            write_localized_strings_for_parsed(&plugin, &strings, output_path.to_str().unwrap())
                .unwrap();

            let strings_values =
                strings::parse_string_table(&root.join("Strings").join("SeventySix_en.STRINGS"))
                    .unwrap();
            assert_eq!(
                strings_values.get(&string_id).map(String::as_str),
                Some("LOC_000423DB")
            );
            fs::remove_dir_all(root).unwrap();

        }
        // localized_message_uses_default_language_when_translation_is_missing
        {
            let body_id = 0x6100;
            let title_id = 0x6101;
            let plugin = localized_test_plugin_for_record(
                "MESG",
                vec![
                    localized_subrecord("DESC", body_id),
                    localized_subrecord("FULL", title_id),
                ],
            );
            let mut state = LocalizedStringsState {
                default_language: "en".to_string(),
                ..LocalizedStringsState::default()
            };
            state.by_language.insert(
                "en".to_string(),
                HashMap::from([
                    (body_id, "Choose a bounty".to_string()),
                    (title_id, "Bounty".to_string()),
                ]),
            );
            state.by_language.insert(
                "de".to_string(),
                HashMap::from([(title_id, "Kopfgeld".to_string())]),
            );
            let output_path = temp_plugin_path("localized-message-language-fallback");
            let root = output_path.parent().unwrap().to_path_buf();
            write_localized_strings_for_parsed(&plugin, &state, output_path.to_str().unwrap()).unwrap();
            for language in ["en", "de"] {
                let body = strings::parse_string_table(
                    &root.join(format!("Strings/SeventySix_{language}.DLSTRINGS")),
                )
                .unwrap();
                assert_eq!(
                    body.get(&body_id).map(String::as_str),
                    Some("Choose a bounty")
                );
            }
            let titles =
                strings::parse_string_table(&root.join("Strings/SeventySix_de.STRINGS")).unwrap();
            assert_eq!(titles.get(&title_id).map(String::as_str), Some("Kopfgeld"));
            fs::remove_dir_all(root).unwrap();

        }
        // localized_string_save_uses_persistent_tables_for_terminal_and_messages
        {
            let term_title_id = 0x6100_EDB2;
            let term_item_id = 0x0003_A99F;
            let term_plugin = localized_test_plugin_for_record(
                "TERM",
                vec![
                    localized_subrecord("RNAM", term_title_id),
                    localized_subrecord("ITXT", term_item_id),
                ],
            );
            let output_path = temp_plugin_path("localized-term-persistent");
            let root = output_path.parent().unwrap().to_path_buf();

            write_localized_strings_for_parsed(
                &term_plugin,
                &LocalizedStringsState::default(),
                output_path.to_str().unwrap(),
            )
            .unwrap();

            let strings_values =
                strings::parse_string_table(&root.join("Strings").join("SeventySix_en.STRINGS"))
                    .unwrap();
            assert_eq!(
                strings_values.get(&term_title_id).map(String::as_str),
                Some("LOC_6100EDB2")
            );
            assert_eq!(
                strings_values.get(&term_item_id).map(String::as_str),
                Some("LOC_0003A99F")
            );
            fs::remove_dir_all(root).unwrap();

            let message_body_id = 0x0003_A99D;
            let message_item_id = 0x0003_A99B;
            let message_plugin = localized_test_plugin_for_record(
                "MESG",
                vec![
                    localized_subrecord("DESC", message_body_id),
                    localized_subrecord("ITXT", message_item_id),
                ],
            );
            let output_path = temp_plugin_path("localized-message-persistent");
            let root = output_path.parent().unwrap().to_path_buf();

            write_localized_strings_for_parsed(
                &message_plugin,
                &LocalizedStringsState::default(),
                output_path.to_str().unwrap(),
            )
            .unwrap();

            let strings_values =
                strings::parse_string_table(&root.join("Strings").join("SeventySix_en.STRINGS"))
                    .unwrap();
            assert_eq!(
                strings_values.get(&message_body_id).map(String::as_str),
                Some("LOC_0003A99D")
            );
            assert_eq!(
                strings_values.get(&message_item_id).map(String::as_str),
                Some("LOC_0003A99B")
            );
            let dlstrings_values =
                strings::parse_string_table(&root.join("Strings").join("SeventySix_en.DLSTRINGS"))
                    .unwrap();
            assert_eq!(
                dlstrings_values.get(&message_body_id).map(String::as_str),
                Some("LOC_0003A99D")
            );
            assert_eq!(
                dlstrings_values.get(&message_item_id).map(String::as_str),
                Some("LOC_0003A99B")
            );
            fs::remove_dir_all(root).unwrap();

        }
    }

    #[test]
    fn localized_string_save_rebuckets_by_field() {
        // localized_string_save_rebuckets_info_rnam_from_ilstrings_to_strings
        // FO76 files INFO.RNAM prompts under .ILSTRINGS; FO4/xEdit reads INFO.RNAM
        // from .STRINGS. A source-carried `table_types[id] = "ilstrings"` must be
        // overridden to the FO4 field table type so the carried text lands in
        // .STRINGS (where xEdit resolves it), not orphaned in .ILSTRINGS.
        {
            let string_id = 0xD900_3F09;
            let plugin =
                localized_test_plugin_for_record("INFO", vec![localized_subrecord("RNAM", string_id)]);
            let mut strings = LocalizedStringsState {
                default_language: "en".to_string(),
                ..LocalizedStringsState::default()
            };
            strings
                .by_language
                .entry("en".to_string())
                .or_default()
                .insert(string_id, "Who are you?".to_string());
            // Pre-seed the FO76 source table type that must be overridden.
            strings
                .table_types
                .insert(string_id, "ilstrings".to_string());
            let output_path = temp_plugin_path("localized-info-rnam-rebucket");
            let root = output_path.parent().unwrap().to_path_buf();

            write_localized_strings_for_parsed(&plugin, &strings, output_path.to_str().unwrap())
                .unwrap();

            let strings_values =
                strings::parse_string_table(&root.join("Strings").join("SeventySix_en.STRINGS"))
                    .unwrap();
            assert_eq!(
                strings_values.get(&string_id).map(String::as_str),
                Some("Who are you?"),
                "INFO.RNAM prompt text must land in .STRINGS with its real text"
            );
            // The carried text must NOT remain orphaned in .ILSTRINGS.
            let ilstrings_path = root.join("Strings").join("SeventySix_en.ILSTRINGS");
            if ilstrings_path.exists() {
                let ilstrings_values = strings::parse_string_table(&ilstrings_path).unwrap();
                assert!(
                    !ilstrings_values.contains_key(&string_id),
                    "INFO.RNAM id must not stay in .ILSTRINGS after rebucket"
                );
            }
            fs::remove_dir_all(root).unwrap();

        }
        // localized_string_save_rebuckets_lscr_desc_from_dlstrings_to_strings
        // FO76 files LSCR.DESC under .DLSTRINGS; FO4/xEdit reads it from .STRINGS
        // (unlike BOOK/SPEL/PERK DESC). The record-scoped exception must rebucket it.
        {
            let string_id = 0x0002_B4C4;
            let plugin =
                localized_test_plugin_for_record("LSCR", vec![localized_subrecord("DESC", string_id)]);
            let mut strings = LocalizedStringsState {
                default_language: "en".to_string(),
                ..LocalizedStringsState::default()
            };
            strings
                .by_language
                .entry("en".to_string())
                .or_default()
                .insert(string_id, "A loading screen tip.".to_string());
            strings
                .table_types
                .insert(string_id, "dlstrings".to_string());
            let output_path = temp_plugin_path("localized-lscr-desc-rebucket");
            let root = output_path.parent().unwrap().to_path_buf();

            write_localized_strings_for_parsed(&plugin, &strings, output_path.to_str().unwrap())
                .unwrap();

            let strings_values =
                strings::parse_string_table(&root.join("Strings").join("SeventySix_en.STRINGS"))
                    .unwrap();
            assert_eq!(
                strings_values.get(&string_id).map(String::as_str),
                Some("A loading screen tip."),
                "LSCR.DESC must land in .STRINGS"
            );
            let dlstrings_path = root.join("Strings").join("SeventySix_en.DLSTRINGS");
            if dlstrings_path.exists() {
                let dlstrings_values = strings::parse_string_table(&dlstrings_path).unwrap();
                assert!(
                    !dlstrings_values.contains_key(&string_id),
                    "LSCR.DESC id must not stay in .DLSTRINGS after rebucket"
                );
            }
            fs::remove_dir_all(root).unwrap();

        }
        // localized_string_save_rebuckets_book_cnam_to_dlstrings
        // BOOK.CNAM is xEdit's Description field and resolves from .DLSTRINGS in
        // FO4. Whole-plugin conversion can carry a CNAM id tagged as .STRINGS, so
        // the save pass must re-bucket it.
        {
            let string_id = 0x0003_F8BF;
            let plugin =
                localized_test_plugin_for_record("BOOK", vec![localized_subrecord("CNAM", string_id)]);
            let mut strings = LocalizedStringsState {
                default_language: "en".to_string(),
                ..LocalizedStringsState::default()
            };
            strings
                .by_language
                .entry("en".to_string())
                .or_default()
                .insert(string_id, "Care To Test Your Metal?".to_string());
            strings.table_types.insert(string_id, "strings".to_string());
            let output_path = temp_plugin_path("localized-book-cnam-rebucket");
            let root = output_path.parent().unwrap().to_path_buf();

            write_localized_strings_for_parsed(&plugin, &strings, output_path.to_str().unwrap())
                .unwrap();

            let dlstrings_values =
                strings::parse_string_table(&root.join("Strings").join("SeventySix_en.DLSTRINGS"))
                    .unwrap();
            assert_eq!(
                dlstrings_values.get(&string_id).map(String::as_str),
                Some("Care To Test Your Metal?"),
                "BOOK.CNAM description text must land in .DLSTRINGS"
            );
            let strings_path = root.join("Strings").join("SeventySix_en.STRINGS");
            if strings_path.exists() {
                let strings_values = strings::parse_string_table(&strings_path).unwrap();
                assert!(
                    !strings_values.contains_key(&string_id),
                    "BOOK.CNAM id must not stay in .STRINGS after rebucket"
                );
            }
            fs::remove_dir_all(root).unwrap();

        }
        // localized_string_save_rebuckets_qust_cnam_to_dlstrings
        // FO76 and FO4 both store QUST quest-log entries (QUST.CNAM) under
        // .DLSTRINGS. The emission classifier has no CNAM case, so it defaults the
        // id to .STRINGS; the QUST.CNAM exception must re-bucket it to .DLSTRINGS or
        // the CK fails the DLSTRINGS lookup for every quest-stage log entry.
        {
            let string_id = 0x0003_69F4;
            let plugin =
                localized_test_plugin_for_record("QUST", vec![localized_subrecord("CNAM", string_id)]);
            let mut strings = LocalizedStringsState {
                default_language: "en".to_string(),
                ..LocalizedStringsState::default()
            };
            strings
                .by_language
                .entry("en".to_string())
                .or_default()
                .insert(string_id, "You found the holotape.".to_string());
            // Whole-plugin conversion loses the source .DLSTRINGS classification, so
            // the id arrives tagged .STRINGS. The exception must force .DLSTRINGS.
            strings.table_types.insert(string_id, "strings".to_string());
            let output_path = temp_plugin_path("localized-qust-cnam-rebucket");
            let root = output_path.parent().unwrap().to_path_buf();

            write_localized_strings_for_parsed(&plugin, &strings, output_path.to_str().unwrap())
                .unwrap();

            let dlstrings_values =
                strings::parse_string_table(&root.join("Strings").join("SeventySix_en.DLSTRINGS"))
                    .unwrap();
            assert_eq!(
                dlstrings_values.get(&string_id).map(String::as_str),
                Some("You found the holotape."),
                "QUST.CNAM quest-log text must land in .DLSTRINGS"
            );
            let strings_path = root.join("Strings").join("SeventySix_en.STRINGS");
            if strings_path.exists() {
                let strings_values = strings::parse_string_table(&strings_path).unwrap();
                assert!(
                    !strings_values.contains_key(&string_id),
                    "QUST.CNAM id must not stay in .STRINGS after rebucket"
                );
            }
            fs::remove_dir_all(root).unwrap();

        }
    }

    fn scan_test_record_bytes(signature: &[u8; 4], form_id: u32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MODERN_HEADER_SIZE);
        bytes.extend_from_slice(signature);
        bytes.extend_from_slice(&0u32.to_le_bytes()); // data size (no payload)
        bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
        bytes.extend_from_slice(&form_id.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 8]); // vc + form version + unknown
        bytes
    }

    fn scan_test_grup_bytes(label: &[u8; 4], children: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MODERN_HEADER_SIZE + children.len());
        bytes.extend_from_slice(b"GRUP");
        bytes.extend_from_slice(&((MODERN_HEADER_SIZE + children.len()) as u32).to_le_bytes());
        bytes.extend_from_slice(label);
        bytes.extend_from_slice(&[0u8; 12]); // group type + stamp + unknown
        bytes.extend_from_slice(children);
        bytes
    }

    #[test]
    fn scan_record_offsets_continues_past_empty_grup() {
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(&scan_test_record_bytes(b"KYWD", 0x0000_0001));
        // Top-level empty group mid-stream (size == header only) — legal,
        // present in shipped ESMs (Fallout4.esm TREE).
        data.extend_from_slice(&scan_test_grup_bytes(b"TREE", &[]));
        let after_empty = data.len();
        data.extend_from_slice(&scan_test_record_bytes(b"PACK", 0x0000_0002));
        // Nested case: parent group whose first child is an empty group,
        // followed by a sibling record inside the same parent.
        let inner = [
            scan_test_grup_bytes(b"DOOR", &[]),
            scan_test_record_bytes(b"WRLD", 0x0000_0003),
        ]
        .concat();
        let nested_record_offset = data.len() + 2 * MODERN_HEADER_SIZE;
        data.extend_from_slice(&scan_test_grup_bytes(b"WRLD", &inner));
        data.extend_from_slice(&scan_test_record_bytes(b"QUST", 0x0000_0004));

        let mut offsets = rustc_hash::FxHashMap::default();
        let data = Bytes::from(data);
        scan_record_offsets(&data, 0, data.len(), MODERN_HEADER_SIZE, &mut offsets);

        assert_eq!(offsets.get(&0x0000_0001), Some(&0));
        assert_eq!(
            offsets.get(&0x0000_0002),
            Some(&after_empty),
            "record after a top-level empty GRUP must stay indexed"
        );
        assert_eq!(
            offsets.get(&0x0000_0003),
            Some(&nested_record_offset),
            "record after a nested empty GRUP must stay indexed"
        );
        assert_eq!(
            offsets.get(&0x0000_0004),
            Some(&(data.len() - MODERN_HEADER_SIZE)),
            "record after the parent group must stay indexed"
        );
        assert_eq!(offsets.len(), 4);
    }
}

pub fn parse_plugin_file(
    path: &str,
    game: Option<String>,
    eager_compressed: bool,
) -> PyResult<ParsedPlugin> {
    if eager_compressed {
        parse_plugin_file_with_compression(path, game, true)
    } else {
        parse_plugin_file_lazy_compressed(path, game)
    }
}

pub fn parse_plugin_file_lazy_compressed(
    path: &str,
    game: Option<String>,
) -> PyResult<ParsedPlugin> {
    parse_plugin_file_with_compression(path, game, false)
}

/// Eagerly decompresses every COMPRESSED-flagged record so callers get fully
/// decoded subrecord arrays — used by inspect/export entry points that need
/// to render `raw_payload_hex`-free YAML. General plugin queries should keep
/// using `parse_plugin_file_lazy_compressed` to preserve load-all semantics.
pub fn parse_plugin_file_eager_compressed(
    path: &str,
    game: Option<String>,
) -> PyResult<ParsedPlugin> {
    parse_plugin_file_with_compression(path, game, true)
}

fn parse_plugin_file_with_compression(
    path: &str,
    game: Option<String>,
    eager_compressed: bool,
) -> PyResult<ParsedPlugin> {
    let file_path = Path::new(path);
    let data = read_plugin_source_bytes(file_path)?;
    let plugin_name = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Plugin.esp")
        .to_string();
    parse_plugin_bytes_with_compression(
        data,
        plugin_name,
        file_path.to_string_lossy().into_owned(),
        game,
        eager_compressed,
    )
}

/// Read the source plugin file. For files large enough to matter (Starfield.esm
/// at 1.39 GB, Skyrim/FO4 ESMs over a few hundred MB) this mmaps so the bytes
/// stay as disk-backed pages the OS can evict instead of committed RAM.
/// Smaller plugins fall back to `fs::read` to keep test fixtures simple.
const MMAP_THRESHOLD: u64 = 64 * 1024 * 1024;

pub(crate) fn read_plugin_source_bytes(file_path: &Path) -> PyResult<Bytes> {
    let metadata = fs::metadata(file_path).map_err(|err| {
        io_error(format!(
            "failed to stat plugin '{}': {err}",
            file_path.display()
        ))
    })?;

    if metadata.len() >= MMAP_THRESHOLD {
        let file = fs::File::open(file_path).map_err(|err| {
            io_error(format!(
                "failed to open plugin '{}': {err}",
                file_path.display()
            ))
        })?;
        // SAFETY: we own the File handle until the Mmap is dropped, and the
        // file is read-only from our perspective. Concurrent external writes
        // to the source ESM during a parse would be undefined, but that
        // would corrupt any reader regardless of mmap vs read.
        let mmap = unsafe { memmap2::Mmap::map(&file) }.map_err(|err| {
            io_error(format!(
                "failed to mmap plugin '{}': {err}",
                file_path.display()
            ))
        })?;
        return Ok(Bytes::from_owner(mmap));
    }

    let vec = fs::read(file_path).map_err(|err| {
        io_error(format!(
            "failed to read plugin '{}': {err}",
            file_path.display()
        ))
    })?;
    Ok(Bytes::from(vec))
}

pub(crate) fn parse_plugin_bytes(
    data: Bytes,
    plugin_name: String,
    file_path: String,
    game: Option<String>,
) -> PyResult<ParsedPlugin> {
    parse_plugin_bytes_with_compression(data, plugin_name, file_path, game, true)
}

fn parse_plugin_bytes_with_compression(
    data: Bytes,
    plugin_name: String,
    file_path: String,
    game: Option<String>,
    eager_compressed: bool,
) -> PyResult<ParsedPlugin> {
    let header_size = detect_header_size(&data);
    let (header_record, offset) = parse_record(&data, 0, header_size, true)?;
    if header_record.signature != "TES4" {
        return Err(value_error(format!(
            "expected TES4 header record, got {}",
            header_record.signature
        )));
    }
    let header = parse_plugin_header(&header_record);
    let data_len = data.len();
    let (root_items, _) = parse_children(&data, offset, data_len, header_size, eager_compressed)?;
    Ok(ParsedPlugin {
        plugin_name,
        file_path,
        header_size,
        header,
        root_items,
        game,
    })
}

pub(crate) fn load_plugin_native_impl(
    py: Python<'_>,
    plugin_path: &str,
    game: Option<&str>,
    _jobs: Option<usize>,
    strings_dir: Option<&str>,
    language: Option<&str>,
    eager_compressed: bool,
) -> PyResult<Py<PyAny>> {
    let path = plugin_path.to_string();
    let game_owned = game.map(str::to_string);
    let strings_dir_owned = strings_dir.map(str::to_string);
    let language_owned = language.map(str::to_string);
    let (parsed, strings) = py.detach(move || {
        let parsed = parse_plugin_file(path.as_str(), game_owned, eager_compressed)?;
        let plugin_name = parsed.plugin_name.clone();
        let is_localized = (parsed.header.flags & 0x80) != 0;
        let strings = if is_localized {
            crate::plugin_runtime::strings::hydrate_strings_state(
                path.as_str(),
                &plugin_name,
                strings_dir_owned.as_deref(),
                language_owned.as_deref(),
            )
        } else {
            LocalizedStringsState::default()
        };
        Ok::<_, PyErr>((parsed, strings))
    })?;
    let handle_id = insert_plugin_handle(parsed, strings);
    Ok(handle_id.into_py_any(py)?)
}

pub(crate) fn localized_table_type_for_signature(
    record_signature: Option<&str>,
    signature: &str,
) -> Option<&'static str> {
    // LSCR.DESC (loading-screen text) is a plain STRINGS entry in FO4, unlike the
    // BOOK/SPEL/PERK long descriptions that use DLSTRINGS — record-scoped exception.
    // BOOK.CNAM and QUST.CNAM are the inverse: CNAM defaults to STRINGS, but
    // FO4 reads these description/log entries from DLSTRINGS — record-scoped too.
    match (record_signature, signature) {
        (Some("TERM"), "ITXT" | "RNAM") | (Some("MESG"), "DESC" | "ITXT") => Some("strings"),
        (Some("LSCR"), "DESC") => Some("strings"),
        (Some("BOOK"), "CNAM") => Some("dlstrings"),
        (Some("QUST"), "CNAM") => Some("dlstrings"),
        (_, signature) => match signature {
            "DESC" | "ITXT" => Some("dlstrings"),
            "FULL" | "NNAM" | "SHRT" => Some("strings"),
            "NAM1" => Some("ilstrings"),
            // RNAM (INFO Prompt / FLOR Activate Text Override) is plain UI text,
            // never voiced — STRINGS, not ILSTRINGS.
            "RNAM" => Some("strings"),
            _ => None,
        },
    }
}

fn table_type_for_localized_signature(
    record_signature: Option<&str>,
    signature: &str,
) -> &'static str {
    match (record_signature, signature) {
        (Some("TERM"), "ITXT" | "RNAM") | (Some("MESG"), "DESC" | "ITXT") => "strings",
        (Some("LSCR"), "DESC") => "strings",
        (Some("BOOK"), "CNAM") => "dlstrings",
        (Some("QUST"), "CNAM") => "dlstrings",
        (_, signature) => match signature {
            "DESC" | "ITXT" => "dlstrings",
            "NAM1" => "ilstrings",
            "RNAM" => "strings",
            _ => "strings",
        },
    }
}

fn infer_localized_table_types(plugin: &ParsedPlugin, table_types: &mut HashMap<u32, String>) {
    fn walk_items(items: &[ParsedItem], table_types: &mut HashMap<u32, String>) {
        for item in items {
            match item {
                ParsedItem::Group(group) => walk_items(&group.children, table_types),
                ParsedItem::Record(record) => {
                    for subrecord in &record.subrecords {
                        if subrecord.data.len() != 4 {
                            continue;
                        }
                        let Some(table_type) = localized_table_type_for_signature(
                            Some(record.signature.as_str()),
                            subrecord.signature.as_str(),
                        ) else {
                            continue;
                        };
                        let id = u32::from_le_bytes([
                            subrecord.data[0],
                            subrecord.data[1],
                            subrecord.data[2],
                            subrecord.data[3],
                        ]);
                        // OVERRIDE, not or_insert: the FO4 field that references this
                        // id determines which table file xEdit resolves it from. A
                        // table type carried from the source plugin (e.g. FO76 files
                        // RNAM prompts under .ILSTRINGS) is wrong for FO4 and must be
                        // re-tagged to the field's FO4 table type, or the string lands
                        // in the wrong .STRINGS/.ILSTRINGS/.DLSTRINGS file and fails to
                        // resolve. Source tables never share an id across table types,
                        // so the last-writer-wins risk for a genuinely shared id is nil.
                        table_types.insert(id, table_type.to_string());
                    }
                }
            }
        }
    }

    walk_items(&plugin.root_items, table_types);
}

fn schema_localized_table_type(
    schema: Option<&CompiledSchema>,
    record: &ParsedRecord,
    signature: &str,
    occurrence: usize,
) -> Option<&'static str> {
    let schema = schema?;
    let record_spec = schema_record_spec(schema, record.signature.as_str())?;
    let subrecord_spec = schema_subrecord_spec(record_spec, signature, occurrence)?;
    subrecord_spec
        .localized
        .then(|| table_type_for_localized_signature(Some(record.signature.as_str()), signature))
}

pub(crate) fn collect_record_localized_table_refs(
    record: &ParsedRecord,
    schema: Option<&CompiledSchema>,
    refs: &mut HashMap<&'static str, HashSet<u32>>,
) {
    let mut occurrences: HashMap<&str, usize> = HashMap::new();
    for subrecord in &record.subrecords {
        let signature = subrecord.signature.as_str();
        let occurrence = *occurrences.get(signature).unwrap_or(&0);
        occurrences.insert(signature, occurrence.saturating_add(1));
        if subrecord.data.len() != 4 {
            continue;
        }
        let Some(table_type) =
            schema_localized_table_type(schema, record, signature, occurrence).or_else(|| {
                localized_table_type_for_signature(Some(record.signature.as_str()), signature)
            })
        else {
            continue;
        };
        let id = u32::from_le_bytes([
            subrecord.data[0],
            subrecord.data[1],
            subrecord.data[2],
            subrecord.data[3],
        ]);
        refs.entry(table_type).or_default().insert(id);
        if record.signature.as_str() == "MESG" && matches!(signature, "DESC" | "ITXT") {
            refs.entry("dlstrings").or_default().insert(id);
        }
    }
}

fn collect_localized_table_refs(
    plugin: &ParsedPlugin,
    schema: Option<&CompiledSchema>,
) -> HashMap<&'static str, HashSet<u32>> {
    fn walk_items(
        items: &[ParsedItem],
        schema: Option<&CompiledSchema>,
        refs: &mut HashMap<&'static str, HashSet<u32>>,
    ) {
        for item in items {
            match item {
                ParsedItem::Group(group) => walk_items(&group.children, schema, refs),
                ParsedItem::Record(record) => {
                    collect_record_localized_table_refs(record, schema, refs);
                }
            }
        }
    }

    let mut refs: HashMap<&'static str, HashSet<u32>> = HashMap::new();
    walk_items(&plugin.root_items, schema, &mut refs);
    refs
}

fn localized_refs_are_empty(refs: &HashMap<&'static str, HashSet<u32>>) -> bool {
    refs.values().all(HashSet::is_empty)
}

fn missing_localized_string_placeholder(string_id: u32) -> String {
    format!("LOC_{string_id:08X}")
}

fn build_string_table_blob(values: &HashMap<u32, String>, table_type: &str) -> PyResult<Vec<u8>> {
    let length_prefixed = match table_type {
        "strings" => false,
        "ilstrings" | "dlstrings" => true,
        _ => {
            return Err(value_error(format!(
                "unsupported string table type: {table_type}"
            )));
        }
    };
    let mut ordered: Vec<(&u32, &String)> = values.iter().collect();
    ordered.sort_by_key(|(key, _)| **key);
    let mut directory = Vec::with_capacity(ordered.len() * 8);
    let mut payload = Vec::new();
    for (string_id, text) in ordered {
        let encoded = text.as_bytes();
        let offset = payload.len() as u32;
        directory.extend_from_slice(&string_id.to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        if length_prefixed {
            payload.extend_from_slice(&((encoded.len() + 1) as u32).to_le_bytes());
        }
        payload.extend_from_slice(encoded);
        payload.push(0);
    }
    let mut out = Vec::with_capacity(8 + directory.len() + payload.len());
    out.extend_from_slice(&(values.len() as u32).to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&directory);
    out.extend_from_slice(&payload);
    Ok(out)
}

fn build_subrecord_bytes(signature: &str, data: &[u8]) -> Vec<u8> {
    if data.len() > 0xFFFF {
        let mut out = Vec::with_capacity(data.len() + 16);
        out.extend_from_slice(b"XXXX");
        out.extend_from_slice(&(4u16).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(signature.as_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(data);
        return out;
    }
    let mut out = Vec::with_capacity(data.len() + 6);
    out.extend_from_slice(signature.as_bytes());
    out.extend_from_slice(&(data.len() as u16).to_le_bytes());
    out.extend_from_slice(data);
    out
}

pub(crate) fn encode_subrecords_uncompressed(subrecords: &[ParsedSubrecord]) -> Vec<u8> {
    let mut out = Vec::new();
    for subrecord in subrecords {
        out.extend_from_slice(&build_subrecord_bytes(
            subrecord.signature.as_str(),
            &subrecord.data,
        ));
    }
    out
}

/// Inverse of `parse_compressed_subrecords_from_payload`: encodes a subrecord
/// array into the on-disk COMPRESSED record payload — `u32` declared
/// decompressed-size (little-endian) followed by the zlib stream.
pub fn compress_subrecords_payload(subrecords: &[ParsedSubrecord]) -> PyResult<Vec<u8>> {
    let uncompressed = encode_subrecords_uncompressed(subrecords);
    let declared_size = uncompressed.len() as u32;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&uncompressed)
        .map_err(|err| io_error(format!("zlib compression failed: {err}")))?;
    let compressed = encoder
        .finish()
        .map_err(|err| io_error(format!("zlib compression finalize failed: {err}")))?;
    let mut out = Vec::with_capacity(4 + compressed.len());
    out.extend_from_slice(&declared_size.to_le_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
}

pub(crate) fn header_subrecords_from_parsed(plugin: &ParsedPlugin) -> Vec<Vec<u8>> {
    let header = &plugin.header;
    if !header.raw_subrecords.is_empty() {
        let mut out = Vec::with_capacity(header.raw_subrecords.len());
        for raw_subrecord in &header.raw_subrecords {
            out.push(build_subrecord_bytes(
                raw_subrecord.signature.as_str(),
                &raw_subrecord.data,
            ));
        }
        return out;
    }

    let mut out = Vec::new();
    let mut hedr_bytes: Vec<u8> = header
        .hedr_raw
        .as_ref()
        .map(|b| b.to_vec())
        .unwrap_or_default();
    if hedr_bytes.len() < 12 {
        hedr_bytes.clear();
        hedr_bytes.extend_from_slice(&header.version.to_le_bytes());
        hedr_bytes.extend_from_slice(&header.num_records.to_le_bytes());
        hedr_bytes.extend_from_slice(&header.next_object_id.to_le_bytes());
    } else {
        hedr_bytes[4..8].copy_from_slice(&header.num_records.to_le_bytes());
        hedr_bytes[0..4].copy_from_slice(&header.version.to_le_bytes());
        hedr_bytes[8..12].copy_from_slice(&header.next_object_id.to_le_bytes());
    }
    out.push(build_subrecord_bytes("HEDR", &hedr_bytes));
    if !header.author.is_empty() {
        out.push(build_subrecord_bytes(
            "CNAM",
            &encode_cp1252(&header.author, true),
        ));
    }
    if !header.description.is_empty() {
        out.push(build_subrecord_bytes(
            "SNAM",
            &encode_cp1252(&header.description, true),
        ));
    }
    for (index, master_name) in header.masters.iter().enumerate() {
        out.push(build_subrecord_bytes(
            "MAST",
            &encode_cp1252(master_name, true),
        ));
        let size = header.master_sizes.get(index).copied().unwrap_or(0);
        out.push(build_subrecord_bytes("DATA", &size.to_le_bytes()));
    }
    if !header.overridden_forms.is_empty() {
        let mut payload = Vec::with_capacity(header.overridden_forms.len() * 4);
        for raw in &header.overridden_forms {
            payload.extend_from_slice(&raw.to_le_bytes());
        }
        out.push(build_subrecord_bytes("ONAM", &payload));
    }
    for extra in &header.extra_subrecords {
        out.push(build_subrecord_bytes(extra.signature.as_str(), &extra.data));
    }
    out
}

/// Serialize a single [`ParsedRecord`] to its on-disk byte form (header +
/// payload), framed exactly as the buffered (`build_plugin_bytes`) and streamed
/// (`write_plugin_to`) serializers do.
pub(crate) fn record_bytes_from_parsed(
    record: &ParsedRecord,
    header_size: usize,
) -> PyResult<Vec<u8>> {
    let compressed = (record.flags & COMPRESSED_RECORD_FLAG) != 0;
    let mut payload: Vec<u8> = if let Some(raw_payload) = &record.raw_payload {
        if compressed && record.subrecords.is_empty() {
            match decode_compressed_subrecords_from_payload(raw_payload) {
                Ok(decoded) if decoded.salvaged_bad_checksum => {
                    compress_subrecords_payload(&decoded.subrecords)?
                }
                _ => raw_payload.to_vec(),
            }
        } else if record.subrecords.is_empty() || compressed {
            raw_payload.to_vec()
        } else {
            encode_subrecords_uncompressed(&record.subrecords)
        }
    } else if compressed {
        compress_subrecords_payload(&record.subrecords)?
    } else {
        encode_subrecords_uncompressed(&record.subrecords)
    };

    let mut out = Vec::with_capacity(payload.len() + header_size);
    out.extend_from_slice(record.signature.as_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&record.flags.to_le_bytes());
    out.extend_from_slice(&record.form_id.to_le_bytes());
    out.extend_from_slice(&record.version_control.to_le_bytes());
    if header_size == MODERN_HEADER_SIZE {
        out.extend_from_slice(&record.form_version.unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&record.version2.unwrap_or(0).to_le_bytes());
    }
    out.append(&mut payload);
    Ok(out)
}

/// Serialize a parsed GRUP (header + recursively-framed children) to its on-disk
/// bytes.
pub(crate) fn group_bytes_from_parsed(
    group: &ParsedGroup,
    header_size: usize,
) -> PyResult<Vec<u8>> {
    let mut children_bytes = Vec::new();
    append_item_bytes(&group.children, header_size, &mut children_bytes)?;
    let total_size = header_size + children_bytes.len();
    let mut out = Vec::with_capacity(total_size);
    out.extend_from_slice(b"GRUP");
    out.extend_from_slice(&(total_size as u32).to_le_bytes());
    out.extend_from_slice(&group.label);
    out.extend_from_slice(&group.group_type.to_le_bytes());
    let tail_len = header_size.saturating_sub(16);
    let mut padded_tail = vec![0u8; tail_len];
    let copy_len = group.tail.len().min(tail_len);
    padded_tail[..copy_len].copy_from_slice(&group.tail[..copy_len]);
    out.extend_from_slice(&padded_tail);
    out.extend_from_slice(&children_bytes);
    Ok(out)
}

fn item_bytes_from_parsed(item: &ParsedItem, header_size: usize) -> PyResult<Vec<u8>> {
    match item {
        ParsedItem::Group(group) => group_bytes_from_parsed(group, header_size),
        ParsedItem::Record(record) => record_bytes_from_parsed(record, header_size),
    }
}

fn append_item_bytes(
    items: &[ParsedItem],
    header_size: usize,
    output: &mut Vec<u8>,
) -> PyResult<()> {
    if items.len() < 32 {
        for item in items {
            output.extend_from_slice(&item_bytes_from_parsed(item, header_size)?);
        }
        return Ok(());
    }

    use rayon::prelude::*;

    let batch_size = crate::default_job_count() * 64;
    for batch in items.chunks(batch_size) {
        let encoded = batch
            .par_iter()
            .map(|item| item_bytes_from_parsed(item, header_size))
            .collect::<PyResult<Vec<_>>>()?;
        for bytes in encoded {
            output.extend_from_slice(&bytes);
        }
    }
    Ok(())
}

pub(crate) fn write_localized_strings_for_parsed(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    output_path: &str,
) -> PyResult<Vec<PathBuf>> {
    if (plugin.header.flags & TES4_FLAG_LOCALIZED) == 0 {
        return Ok(Vec::new());
    }
    let schema = plugin
        .game
        .as_deref()
        .and_then(|game| compiled_schema_for_game(game).ok());
    let localized_table_refs = collect_localized_table_refs(plugin, schema.as_deref());
    write_localized_strings_with_refs(plugin, strings, output_path, &localized_table_refs)
}

pub(crate) fn write_localized_strings_with_refs(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    output_path: &str,
    localized_table_refs: &HashMap<&'static str, HashSet<u32>>,
) -> PyResult<Vec<PathBuf>> {
    if (plugin.header.flags & TES4_FLAG_LOCALIZED) == 0
        || (strings.by_language.is_empty() && localized_refs_are_empty(localized_table_refs))
    {
        return Ok(Vec::new());
    }
    let output = Path::new(output_path);
    let strings_dir = match output.parent() {
        Some(parent) => parent.join("Strings"),
        None => PathBuf::from("Strings"),
    };
    fs::create_dir_all(&strings_dir).map_err(|err| {
        io_error(format!(
            "failed to create strings directory '{}': {err}",
            strings_dir.display()
        ))
    })?;
    let mut table_types = strings.table_types.clone();
    infer_localized_table_types(plugin, &mut table_types);
    let raw_stem = output
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Plugin");
    let raw_stem_lower = raw_stem.to_ascii_lowercase();
    let stem = [".esp", ".esm", ".esl"]
        .iter()
        .find_map(|suffix| {
            raw_stem_lower
                .strip_suffix(suffix)
                .map(|stripped| &raw_stem[..stripped.len()])
        })
        .unwrap_or(raw_stem);
    let mut languages: Vec<String> = strings.by_language.keys().cloned().collect();
    if languages.is_empty() {
        let language = strings.default_language.trim();
        languages.push(if language.is_empty() {
            "en".to_string()
        } else {
            language.to_string()
        });
    }
    languages.sort();
    let mut written: Vec<PathBuf> = Vec::new();
    let default_values = strings
        .by_language
        .get(strings.default_language.trim())
        .or_else(|| strings.by_language.get("en"));
    for language in languages {
        let values = strings.by_language.get(language.as_str());
        let mut buckets: HashMap<&str, HashMap<u32, String>> = HashMap::from([
            ("strings", HashMap::new()),
            ("ilstrings", HashMap::new()),
            ("dlstrings", HashMap::new()),
        ]);
        if let Some(values) = values {
            for (string_id, text) in values {
                let table_type = table_types
                    .get(string_id)
                    .map(|value| value.as_str())
                    .unwrap_or("strings");
                buckets
                    .entry(table_type)
                    .or_insert_with(HashMap::new)
                    .insert(*string_id, text.clone());
            }
        }
        for (table_type, string_ids) in localized_table_refs {
            for string_id in string_ids {
                let text = values
                    .and_then(|table| table.get(string_id))
                    .or_else(|| default_values.and_then(|table| table.get(string_id)))
                    .cloned()
                    .unwrap_or_else(|| missing_localized_string_placeholder(*string_id));
                buckets
                    .entry(*table_type)
                    .or_insert_with(HashMap::new)
                    .entry(*string_id)
                    .or_insert(text);
            }
        }
        for (table_type, table_values) in buckets {
            if table_values.is_empty() {
                continue;
            }
            let extension = match table_type {
                "strings" => ".STRINGS",
                "ilstrings" => ".ILSTRINGS",
                "dlstrings" => ".DLSTRINGS",
                _ => {
                    return Err(value_error(format!(
                        "unsupported string table type: {table_type}"
                    )));
                }
            };
            let file_name = format!("{stem}_{}{extension}", language);
            let target = strings_dir.join(file_name);
            let blob = build_string_table_blob(&table_values, table_type)?;
            fs::write(&target, blob).map_err(|err| {
                io_error(format!(
                    "failed to write localized strings '{}': {err}",
                    target.display()
                ))
            })?;
            written.push(target);
        }
    }
    Ok(written)
}

pub(crate) fn build_plugin_bytes(plugin: &mut ParsedPlugin) -> PyResult<Vec<u8>> {
    // Preserve Bethesda's stored `num_records` when the plugin was loaded
    // with an intact HEDR payload (binary load or JSON/YAML roundtrip).
    // Master ESMs ship `num_records` values that don't match the actual
    // record count; clobbering with `count_records` breaks byte-exact
    // roundtrip. When hedr_raw is None OR num_records is 0 (the default
    // for freshly-constructed plugins that haven't been serialized yet),
    // compute from root_items. A real ESM with num_records=0 is impossible
    // in practice and would produce the same result either way.
    if plugin.header.hedr_raw.is_none() || plugin.header.num_records == 0 {
        plugin.header.num_records = count_hedr_entries(&plugin.root_items) as u32;
    }
    rewrite_semantic_formids_in_place(plugin);
    let header_subrecords = header_subrecords_from_parsed(plugin);
    let mut header_payload = Vec::new();
    for subrecord in &header_subrecords {
        header_payload.extend_from_slice(subrecord);
    }
    let tes4 = tes4_record_from_parsed(plugin, header_payload);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&record_bytes_from_parsed(&tes4, plugin.header_size)?);
    append_item_bytes(&plugin.root_items, plugin.header_size, &mut bytes)?;
    Ok(bytes)
}

/// Build the synthetic TES4 header record from a parsed plugin's header state.
/// Shared by the buffered (`build_plugin_bytes`) and streamed
/// (`write_plugin_to`) serializers so they cannot drift.
fn tes4_record_from_parsed(plugin: &ParsedPlugin, header_payload: Vec<u8>) -> ParsedRecord {
    ParsedRecord {
        signature: SmolStr::new_static("TES4"),
        form_id: 0,
        flags: plugin.header.flags,
        version_control: plugin.header.version_control,
        form_version: if plugin.header_size == MODERN_HEADER_SIZE {
            plugin.header.form_version
        } else {
            None
        },
        version2: if plugin.header_size == MODERN_HEADER_SIZE {
            plugin.header.version2
        } else {
            None
        },
        subrecords: Vec::new(),
        raw_payload: Some(Bytes::from(header_payload)),
        parse_error: None,
    }
}

const STREAM_OUTPUT_BUFFER_BYTES: usize = 4 * 1024 * 1024;
const STREAM_RECORD_BATCH_MEMORY_BYTES: usize = 32 * 1024 * 1024;
const STREAM_RECORD_BATCH_RECORDS_PER_JOB: usize = 1024;

trait WriteSeek: std::io::Write + std::io::Seek {}
impl<T: std::io::Write + std::io::Seek + ?Sized> WriteSeek for T {}

struct StreamingPatchWriter<'a, W: WriteSeek + ?Sized> {
    inner: &'a mut W,
    buffer: Vec<u8>,
    buffer_start: u64,
    capacity: usize,
    deferred_group_sizes: Vec<(u64, u32)>,
}

impl<'a, W: WriteSeek + ?Sized> StreamingPatchWriter<'a, W> {
    fn new(inner: &'a mut W, capacity: usize) -> Self {
        let capacity = capacity.max(MODERN_HEADER_SIZE);
        Self {
            inner,
            buffer: Vec::with_capacity(capacity),
            buffer_start: 0,
            capacity,
            deferred_group_sizes: Vec::new(),
        }
    }

    fn position(&self) -> std::io::Result<u64> {
        self.buffer_start
            .checked_add(self.buffer.len() as u64)
            .ok_or_else(|| std::io::Error::other("plugin output position overflow"))
    }

    fn flush_buffer(&mut self) -> std::io::Result<()> {
        self.inner.write_all(&self.buffer)?;
        self.buffer_start = self
            .buffer_start
            .checked_add(self.buffer.len() as u64)
            .ok_or_else(|| std::io::Error::other("plugin output position overflow"))?;
        self.buffer.clear();
        Ok(())
    }

    fn write_bytes(&mut self, mut bytes: &[u8]) -> std::io::Result<()> {
        while !bytes.is_empty() {
            if self.buffer.len() == self.capacity {
                self.flush_buffer()?;
            }
            let take = bytes.len().min(self.capacity - self.buffer.len());
            self.buffer.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
        }
        Ok(())
    }

    fn write_contiguous(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        debug_assert!(bytes.len() <= self.capacity);
        if self.capacity - self.buffer.len() < bytes.len() {
            self.flush_buffer()?;
        }
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    fn patch_group_size(&mut self, offset: u64, size: u32) {
        let relative = offset.checked_sub(self.buffer_start);
        if let Some(relative) =
            relative.filter(|relative| relative.saturating_add(4) <= self.buffer.len() as u64)
        {
            let relative = relative as usize;
            self.buffer[relative..relative + 4].copy_from_slice(&size.to_le_bytes());
        } else {
            self.deferred_group_sizes.push((offset, size));
        }
    }

    fn finish(mut self) -> std::io::Result<usize> {
        self.flush_buffer()?;
        let end = self.buffer_start;
        self.deferred_group_sizes
            .sort_unstable_by_key(|(offset, _)| *offset);
        for (offset, size) in &self.deferred_group_sizes {
            self.inner.seek(std::io::SeekFrom::Start(*offset))?;
            self.inner.write_all(&size.to_le_bytes())?;
        }
        self.inner.seek(std::io::SeekFrom::Start(end))?;
        self.inner.flush()?;
        Ok(self.deferred_group_sizes.len())
    }
}

fn estimated_subrecord_size(subrecord: &ParsedSubrecord) -> usize {
    subrecord
        .data
        .len()
        .saturating_add(if subrecord.data.len() > 0xFFFF { 16 } else { 6 })
}

fn zlib_output_bound(input_size: usize) -> usize {
    input_size
        .saturating_add(input_size / 16)
        .saturating_add(64)
}

fn estimated_record_encoding_memory(record: &ParsedRecord, header_size: usize) -> usize {
    let compressed = (record.flags & COMPRESSED_RECORD_FLAG) != 0;
    let subrecord_size = || {
        record
            .subrecords
            .iter()
            .map(estimated_subrecord_size)
            .fold(0usize, usize::saturating_add)
    };
    let working_bytes = match &record.raw_payload {
        Some(raw_payload) if compressed && record.subrecords.is_empty() => {
            let declared_size = raw_payload
                .get(..4)
                .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()) as usize)
                .unwrap_or(0);
            declared_size.saturating_mul(2).saturating_add(
                raw_payload
                    .len()
                    .max(zlib_output_bound(declared_size))
                    .saturating_mul(2),
            )
        }
        Some(raw_payload) if record.subrecords.is_empty() || compressed => {
            raw_payload.len().saturating_mul(2)
        }
        _ if compressed => {
            let uncompressed = subrecord_size();
            uncompressed
                .saturating_mul(2)
                .saturating_add(zlib_output_bound(uncompressed).saturating_mul(2))
        }
        _ => subrecord_size().saturating_mul(2),
    };
    header_size.saturating_add(working_bytes).saturating_add(
        record
            .subrecords
            .len()
            .saturating_mul(std::mem::size_of::<ParsedSubrecord>()),
    )
}

fn write_group_start<W: WriteSeek + ?Sized>(
    group: &ParsedGroup,
    header_size: usize,
    out: &mut StreamingPatchWriter<'_, W>,
) -> PyResult<u64> {
    let start = out
        .position()
        .map_err(|err| io_error(format!("read plugin stream position: {err}")))?;
    let mut header = Vec::with_capacity(header_size);
    header.extend_from_slice(b"GRUP");
    header.extend_from_slice(&0u32.to_le_bytes());
    header.extend_from_slice(&group.label);
    header.extend_from_slice(&group.group_type.to_le_bytes());
    let tail_len = header_size.saturating_sub(16);
    let copy_len = group.tail.len().min(tail_len);
    header.extend_from_slice(&group.tail[..copy_len]);
    header.resize(header_size, 0);
    out.write_contiguous(&header)
        .map_err(|err| io_error(format!("write plugin group header: {err}")))?;
    Ok(start)
}

fn write_group_end<W: WriteSeek + ?Sized>(
    start: u64,
    out: &mut StreamingPatchWriter<'_, W>,
) -> PyResult<()> {
    let end = out
        .position()
        .map_err(|err| io_error(format!("read plugin stream position: {err}")))?;
    let total_size = end
        .checked_sub(start)
        .and_then(|size| u32::try_from(size).ok())
        .ok_or_else(|| value_error("serialized GRUP size exceeds the u32 file-format limit"))?;
    let size_offset = start
        .checked_add(4)
        .ok_or_else(|| value_error("serialized GRUP offset overflow"))?;
    out.patch_group_size(size_offset, total_size);
    Ok(())
}

enum StreamEvent<'a> {
    GroupStart(&'a ParsedGroup),
    Record(&'a ParsedRecord),
    GroupEnd,
}

struct TraversalFrame<'a> {
    items: &'a [ParsedItem],
    index: usize,
    closes_group: bool,
}

struct StreamTraversal<'a> {
    stack: Vec<TraversalFrame<'a>>,
}

impl<'a> StreamTraversal<'a> {
    fn new(items: &'a [ParsedItem]) -> Self {
        Self {
            stack: vec![TraversalFrame {
                items,
                index: 0,
                closes_group: false,
            }],
        }
    }
}

impl<'a> Iterator for StreamTraversal<'a> {
    type Item = StreamEvent<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let exhausted = self
                .stack
                .last()
                .is_none_or(|frame| frame.index == frame.items.len());
            if exhausted {
                let frame = self.stack.pop()?;
                if frame.closes_group {
                    return Some(StreamEvent::GroupEnd);
                }
                continue;
            }

            let item = {
                let frame = self.stack.last_mut().unwrap();
                let item = &frame.items[frame.index];
                frame.index += 1;
                item
            };
            match item {
                ParsedItem::Record(record) => return Some(StreamEvent::Record(record)),
                ParsedItem::Group(group) => {
                    self.stack.push(TraversalFrame {
                        items: &group.children,
                        index: 0,
                        closes_group: true,
                    });
                    return Some(StreamEvent::GroupStart(group));
                }
            }
        }
    }
}

fn write_items_streaming<W: WriteSeek + ?Sized>(
    items: &[ParsedItem],
    header_size: usize,
    out: &mut StreamingPatchWriter<'_, W>,
) -> PyResult<()> {
    use rayon::prelude::*;

    let max_batch_records = crate::default_job_count() * STREAM_RECORD_BATCH_RECORDS_PER_JOB;
    let max_batch_events = max_batch_records * 16;
    let mut traversal = StreamTraversal::new(items).peekable();
    let mut open_groups = Vec::new();
    while traversal.peek().is_some() {
        let mut events = Vec::with_capacity(max_batch_events);
        let mut estimated_memory = 0usize;
        let mut record_count = 0usize;
        while events.len() < max_batch_events {
            if let Some(StreamEvent::Record(record)) = traversal.peek() {
                let next_memory = estimated_record_encoding_memory(record, header_size);
                if record_count > 0
                    && estimated_memory.saturating_add(next_memory)
                        > STREAM_RECORD_BATCH_MEMORY_BYTES
                {
                    break;
                }
            }
            let Some(event) = traversal.next() else {
                break;
            };
            if let StreamEvent::Record(record) = &event {
                estimated_memory = estimated_memory
                    .saturating_add(estimated_record_encoding_memory(record, header_size));
                record_count += 1;
            }
            events.push(event);
            if record_count > 0
                && (record_count == max_batch_records
                    || estimated_memory == STREAM_RECORD_BATCH_MEMORY_BYTES)
            {
                break;
            }
        }

        let records: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::Record(record) => Some(*record),
                _ => None,
            })
            .collect();
        let encoded = if records.len() == 1 {
            vec![record_bytes_from_parsed(records[0], header_size)?]
        } else {
            records
                .par_iter()
                .map(|record| record_bytes_from_parsed(record, header_size))
                .collect::<PyResult<Vec<_>>>()?
        };
        let mut encoded = encoded.into_iter();
        for event in events {
            match event {
                StreamEvent::GroupStart(group) => {
                    open_groups.push(write_group_start(group, header_size, out)?);
                }
                StreamEvent::Record(_) => {
                    let bytes = encoded.next().unwrap();
                    out.write_bytes(&bytes)
                        .map_err(|err| io_error(format!("write plugin record: {err}")))?;
                }
                StreamEvent::GroupEnd => {
                    let start = open_groups
                        .pop()
                        .ok_or_else(|| value_error("unbalanced serialized GRUP traversal"))?;
                    write_group_end(start, out)?;
                }
            }
        }
    }
    if !open_groups.is_empty() {
        return Err(value_error("unclosed serialized GRUP traversal"));
    }
    Ok(())
}

fn write_plugin_to_with_buffer<W: WriteSeek + ?Sized>(
    plugin: &mut ParsedPlugin,
    out: &mut W,
    buffer_capacity: usize,
) -> PyResult<usize> {
    if plugin.header.hedr_raw.is_none() || plugin.header.num_records == 0 {
        plugin.header.num_records = count_hedr_entries(&plugin.root_items) as u32;
    }
    rewrite_semantic_formids_in_place(plugin);
    let header_subrecords = header_subrecords_from_parsed(plugin);
    let mut header_payload = Vec::new();
    for subrecord in &header_subrecords {
        header_payload.extend_from_slice(subrecord);
    }
    let tes4 = tes4_record_from_parsed(plugin, header_payload);
    let mut writer = StreamingPatchWriter::new(out, buffer_capacity);
    writer
        .write_bytes(&record_bytes_from_parsed(&tes4, plugin.header_size)?)
        .map_err(|err| io_error(format!("write plugin record: {err}")))?;
    write_items_streaming(&plugin.root_items, plugin.header_size, &mut writer)?;
    writer
        .finish()
        .map_err(|err| io_error(format!("finish plugin stream: {err}")))
}

/// Serialize a plugin without materializing any GRUP body. Record siblings are
/// encoded in byte-bounded parallel batches, while group sizes are backpatched
/// in the seekable output after their children have been written.
fn write_plugin_seekable_to<W: WriteSeek + ?Sized>(
    plugin: &mut ParsedPlugin,
    out: &mut W,
) -> PyResult<()> {
    write_plugin_to_with_buffer(plugin, out, STREAM_OUTPUT_BUFFER_BYTES)?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn write_plugin_to<W: std::io::Write + ?Sized>(
    plugin: &mut ParsedPlugin,
    out: &mut W,
) -> PyResult<()> {
    if plugin.header.hedr_raw.is_none() || plugin.header.num_records == 0 {
        plugin.header.num_records = count_hedr_entries(&plugin.root_items) as u32;
    }
    rewrite_semantic_formids_in_place(plugin);
    let header_subrecords = header_subrecords_from_parsed(plugin);
    let mut header_payload = Vec::new();
    for subrecord in &header_subrecords {
        header_payload.extend_from_slice(subrecord);
    }
    let tes4 = tes4_record_from_parsed(plugin, header_payload);
    out.write_all(&record_bytes_from_parsed(&tes4, plugin.header_size)?)
        .map_err(|err| io_error(format!("write plugin record: {err}")))?;
    use rayon::prelude::*;
    for batch in plugin.root_items.chunks(crate::default_job_count()) {
        let encoded = batch
            .par_iter()
            .map(|item| item_bytes_from_parsed(item, plugin.header_size))
            .collect::<PyResult<Vec<_>>>()?;
        for bytes in encoded {
            out.write_all(&bytes)
                .map_err(|err| io_error(format!("write plugin record: {err}")))?;
        }
    }
    Ok(())
}

pub(crate) fn save_parsed_plugin(
    plugin: &mut ParsedPlugin,
    strings: &LocalizedStringsState,
    output_path: &str,
) -> PyResult<()> {
    write_plugin_atomic(output_path, |writer| {
        write_plugin_seekable_to(plugin, writer)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err.to_string()))
    })
    .map_err(|err| io_error(format!("failed to save plugin '{}': {err}", output_path)))?;
    write_localized_strings_for_parsed(plugin, strings, output_path)?;
    Ok(())
}

/// GIL-free variant of `save_parsed_plugin`.
///
/// Identical logic but returns `Result<(), std::io::Error>` so it can be
/// called from Rust-native phases running under `py.allow_threads` where
/// PyO3's GIL handle is unavailable.
pub(crate) fn save_parsed_plugin_no_py(
    plugin: &mut ParsedPlugin,
    strings: &LocalizedStringsState,
    output_path: &str,
) -> std::io::Result<()> {
    write_plugin_atomic(output_path, |writer| {
        write_plugin_seekable_to(plugin, writer)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err.to_string()))
    })?;
    write_localized_strings_no_py(plugin, strings, output_path)?;
    Ok(())
}

/// Serialize a plugin to `output_path` **atomically**: write the full body to a
/// sibling temp file, fsync it, then rename it over the target. A failed or
/// interrupted write never truncates the previous good file; readers see the
/// old file or the complete new one.
///
/// `serialize` streams the plugin body into the provided writer; its `Err` is
/// surfaced (the target is left untouched). On Windows the final rename is
/// retried briefly to ride out a transient sharing/access violation (antivirus
/// scanning the just-written temp file, MO2/CK/game holding a read handle).
fn write_plugin_atomic<F>(output_path: &str, serialize: F) -> std::io::Result<()>
where
    F: FnOnce(&mut dyn WriteSeek) -> std::io::Result<()>,
{
    let target = Path::new(output_path);
    let parent = target.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = parent {
        fs::create_dir_all(dir)?;
    }
    // Temp file in the SAME directory as the target so the rename stays on one
    // filesystem (cross-device rename would fail / fall back to copy).
    let mut temp = match parent {
        Some(dir) => tempfile::NamedTempFile::new_in(dir)?,
        None => tempfile::NamedTempFile::new_in(".")?,
    };
    serialize(&mut temp)?;
    std::io::Write::flush(&mut temp)?;
    // fsync the data before the rename so a crash can't leave a renamed-but-empty
    // file (durability for the commit).
    temp.as_file().sync_all()?;
    persist_temp_over_target(temp, target)
}

#[cfg(windows)]
fn persist_temp_over_target(temp: tempfile::NamedTempFile, target: &Path) -> std::io::Result<()> {
    use std::io::ErrorKind;
    // ERROR_SHARING_VIOLATION (32) / ERROR_ACCESS_DENIED (5): another handle
    // (antivirus, MO2, the game) briefly holds the target. Retry the rename.
    let mut held = temp;
    let mut attempt = 0u32;
    loop {
        match held.persist(target) {
            Ok(_) => return Ok(()),
            Err(err) => {
                let raw = err.error.raw_os_error();
                let retryable = matches!(err.error.kind(), ErrorKind::PermissionDenied)
                    || matches!(raw, Some(5) | Some(32) | Some(33));
                if retryable && attempt < 20 {
                    attempt += 1;
                    held = err.file;
                    std::thread::sleep(std::time::Duration::from_millis(25));
                    continue;
                }
                return Err(err.error);
            }
        }
    }
}

#[cfg(not(windows))]
fn persist_temp_over_target(temp: tempfile::NamedTempFile, target: &Path) -> std::io::Result<()> {
    temp.persist(target).map(|_| ()).map_err(|err| err.error)
}

fn write_localized_strings_no_py(
    plugin: &ParsedPlugin,
    strings: &LocalizedStringsState,
    output_path: &str,
) -> std::io::Result<()> {
    write_localized_strings_for_parsed(plugin, strings, output_path)
        .map(|_| ())
        .map_err(|e| std::io::Error::other(e.to_string()))
}

fn plugin_handle_id_from_python(plugin: &Bound<'_, PyAny>) -> PyResult<Option<u64>> {
    if !plugin.hasattr("_rust_handle")? {
        return Ok(None);
    }
    let handle = plugin.getattr("_rust_handle")?;
    if handle.is_none() {
        return Ok(None);
    }
    Ok(Some(handle.extract::<u64>()?))
}

pub(crate) fn save_plugin_native_impl(
    py: Python<'_>,
    plugin: &Bound<'_, PyAny>,
    output_path: &str,
    _game: Option<&str>,
) -> PyResult<()> {
    if let Some(handle_id) = plugin_handle_id_from_python(plugin)? {
        let output_path = output_path.to_string();
        return py.detach(move || {
            let (mut parsed, strings) =
                crate::plugin_runtime::clone_plugin_handle_state(handle_id)?;
            save_parsed_plugin(&mut parsed, &strings, output_path.as_str())?;
            crate::plugin_runtime::update_plugin_handle_saved_path(handle_id, output_path.as_str());
            Ok(())
        });
    }
    let mut parsed = parsed_plugin_from_python(py, plugin)?;
    let strings = localized_strings_from_python_plugin(py, plugin)?;
    let output_path = output_path.to_string();
    py.detach(move || save_parsed_plugin(&mut parsed, &strings, output_path.as_str()))
}

pub(crate) fn plugin_to_bytes_native_impl(
    py: Python<'_>,
    plugin: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    if let Some(handle_id) = plugin_handle_id_from_python(plugin)? {
        let bytes = py.detach(move || {
            let (mut parsed, _) = crate::plugin_runtime::clone_plugin_handle_state(handle_id)?;
            build_plugin_bytes(&mut parsed)
        })?;
        return Ok(PyBytes::new(py, &bytes).into_py_any(py)?);
    }
    // Thin direct byte-export path for Python Plugin wrappers. This keeps
    // creation_lib.esp.plugin free of a parallel Python record writer.
    let mut parsed = parsed_plugin_from_python(py, plugin)?;
    let bytes = py.detach(move || build_plugin_bytes(&mut parsed))?;
    Ok(PyBytes::new(py, &bytes).into_py_any(py)?)
}
