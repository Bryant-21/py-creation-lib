use super::*;
use pyo3::exceptions::{PyKeyError, PyValueError};
use serde::Serialize;
use std::borrow::Cow;

// FO76→FO4 WRLD header carry.
//
// Both regen paths (terrain cell-slice and full plugin-port) synthesize a bare
// 5-field WRLD skeleton (EDID/NAMA/DATA/NAM0/NAM9) in
// `terrain::authoring_emit::write_world_yaml`. The FO4 renderer/map never
// activates without the header fields that define the worldspace frame, its map
// image, and its climate/water/location links. This carries those from the
// source worldspace onto the target skeleton, translating FO76 field names and
// formids to the FO4 schema.

// Non-ref WRLD header subrecords whose FO76 and FO4 byte layouts are identical
// (verified against generated/{fo4,fo76}.rs): DNAM struct:f,f, MNAM
// struct:i,i,h,h,h,h, NAM4 float32, ONAM struct:f,f,f,f, NAM0/NAM9 f32,f32.
const NONREF_CARRY: &[&str] = &["DNAM", "MNAM", "NAM4", "ONAM", "NAM0", "NAM9"];

// FO76 WRLD runtime/cache tables keyed to the source cell topology. RNAM
// (large-ref), OFST (offset), and CLSZ (cell-size) are dropped; the nested
// CELL/LAND topology is authoritative until a FO4-native rebuilder exists.
const UNSAFE_FO76_RUNTIME_TABLES: &[&str] = &["RNAM", "OFST", "CLSZ"];

// FO76 WRLD max-height grid. Layout is byte-identical FO76↔FO4 (struct h,h,h,h
// dims + width*height 2x2-corner byte cells) and it self-indexes by absolute
// cell coords, which the converted worldspace preserves — so it carries
// verbatim, guarded only against a source blob truncated relative to its own
// dimension header. The MHDT rectangle legitimately differs from the MNAM map
// bounds, so it is NOT cross-checked against them.
const MAX_HEIGHT_DATA: &str = "MHDT";

// FO76 keeps the worldspace map-image path in NAM5 (zstring); FO4 reads it from
// ICON (zstring). The payload is generally byte-identical after prefix cleanup,
// except Appalachia's legacy RegionMap path: FO4's Pip-Boy expects the source
// paper-map texture under Interface\Pip-Boy.
const SOURCE_MAP_IMAGE: &str = "NAM5";
const TARGET_MAP_IMAGE: &str = "ICON";
const FO76_APPALACHIA_PIPBOY_MAP_IMAGE: &[u8] = b"Interface\\Pip-Boy\\papermap_city_d.dds\0";

// WRLD formid header links: (4cc, required target record signature). The source
// ref is master-0 (source-local); it is remapped to the target's own index and
// only carried if the target actually holds a record of the wanted signature.
const FORMID_CARRY: &[(&str, &str)] = &[
    ("XLCN", "LCTN"),
    ("CNAM", "CLMT"),
    ("NAM2", "WATR"),
    ("NAM3", "WATR"),
];

const FO4_MASTER_NAME: &str = "Fallout4.esm";
const FO4_EXT_LAKE_WATER_OBJECT_ID: u32 = 0x0C_8633;

// FO4 WRLD subrecord order (generated/fo4.rs). Carried fields are reinserted in
// schema order so xEdit does not flag "out of order subrecord".
const FO4_WRLD_ORDER: &[&str] = &[
    "EDID", "RNAM", "MHDT", "FULL", "WCTR", "LTMP", "XEZN", "XLCN", "WNAM", "PNAM", "CNAM", "NAM2",
    "NAM3", "NAM4", "DNAM", "ICON", "MODL", "MODB", "MODT", "MODS", "MNAM", "ONAM", "NAMA", "DATA",
    "NAM0", "NAM9", "ZNAM", "NNAM", "XWEM", "TNAM", "UNAM",
];

#[derive(Default, Serialize)]
pub struct WorldspaceHeaderCarryPayload {
    pub copied: u32,
    pub warnings: Vec<String>,
}

pub fn carry_worldspace_header_from_source(
    source: &ParsedPlugin,
    target: &mut ParsedPlugin,
    source_worldspace_editor_id: &str,
    target_worldspace_editor_id: &str,
    full_bytes: Option<Bytes>,
) -> WorldspaceHeaderCarryPayload {
    let mut payload = WorldspaceHeaderCarryPayload::default();

    let Some(source_wrld) = find_wrld(source, source_worldspace_editor_id) else {
        payload.warnings.push(format!(
            "source worldspace not found: {source_worldspace_editor_id}"
        ));
        return payload;
    };

    let mut nonref: HashMap<&'static str, Bytes> = HashMap::new();
    let mut map_image: Option<Bytes> = None;
    let mut max_height: Option<Bytes> = None;
    let mut formid: HashMap<&'static str, Bytes> = HashMap::new();
    let source_subrecords = source_subrecords_for_record(source_wrld);
    for sr in source_subrecords.iter() {
        let sig = sr.signature.as_str();
        if let Some(&key) = NONREF_CARRY.iter().find(|k| **k == sig) {
            nonref.entry(key).or_insert_with(|| sr.data.clone());
        } else if sig == SOURCE_MAP_IMAGE {
            map_image.get_or_insert_with(|| sr.data.clone());
        } else if sig == MAX_HEIGHT_DATA {
            max_height.get_or_insert_with(|| sr.data.clone());
        } else if let Some(&(key, _)) = FORMID_CARRY.iter().find(|(s, _)| *s == sig) {
            formid.entry(key).or_insert_with(|| sr.data.clone());
        }
    }

    // Build object-id → signature index of the target so each carried formid is
    // remapped to the converted record and validated against its expected type.
    let own_index = (target.header.masters.len() & 0xFF) as u32;
    let target_game = target.game.as_deref().unwrap_or_default().to_string();
    let fo4_master_index = target
        .header
        .masters
        .iter()
        .position(|master| master.eq_ignore_ascii_case(FO4_MASTER_NAME))
        .map(|idx| idx as u32);
    let mut obj_sig: HashMap<u32, SmolStr> = HashMap::new();
    index_object_signatures(&target.root_items, &mut obj_sig);

    let Some(target_wrld) = find_wrld_mut(target, target_worldspace_editor_id) else {
        payload.warnings.push(format!(
            "target worldspace not found: {target_worldspace_editor_id}"
        ));
        return payload;
    };

    // Drop any prior carry-target subrecords so a re-run is idempotent, keep the
    // rest of the skeleton, then re-add the carried fields.
    let carry_sigs = carry_target_signatures();
    let mut merged: Vec<ParsedSubrecord> = target_wrld
        .subrecords
        .iter()
        .filter(|s| {
            let sig = s.signature.as_str();
            !carry_sigs.contains(&sig) && !UNSAFE_FO76_RUNTIME_TABLES.contains(&sig)
        })
        .cloned()
        .collect();

    let mut copied = 0u32;
    for sig in NONREF_CARRY {
        if let Some(data) = nonref.get(sig) {
            merged.push(make_subrecord(sig, data.clone()));
            copied += 1;
        }
    }
    if let Some(data) = map_image {
        merged.push(make_subrecord(
            TARGET_MAP_IMAGE,
            normalize_map_image_path(data),
        ));
        copied += 1;
    }
    if let Some(data) = max_height {
        if mhdt_grid_is_well_formed(&data) {
            merged.push(make_subrecord(MAX_HEIGHT_DATA, data));
            copied += 1;
        } else {
            payload.warnings.push(format!(
                "dropped WRLD MHDT: {} bytes inconsistent with its dimension header",
                data.len()
            ));
        }
    }
    for (sig, want) in FORMID_CARRY {
        let Some(data) = formid.get(sig) else {
            continue;
        };
        match remap_leading_formid(data, own_index, &obj_sig, want) {
            Some(remapped) => {
                merged.push(make_subrecord(sig, remapped));
                copied += 1;
            }
            None => {
                if let Some(fallback) =
                    fallback_wrld_water_formid(sig, want, data, &target_game, fo4_master_index)
                {
                    merged.push(make_subrecord(sig, fallback));
                    copied += 1;
                    payload.warnings.push(format!(
                        "fell back WRLD {sig} to 0C8633:Fallout4.esm ExtLakeWater for unmapped source ref"
                    ));
                } else {
                    payload.warnings.push(format!(
                        "dropped WRLD {sig}: no target {want} for source ref"
                    ));
                }
            }
        }
    }
    if let Some(full) = full_bytes {
        merged.push(make_subrecord("FULL", full));
        copied += 1;
    }

    merged.sort_by_key(|s| {
        FO4_WRLD_ORDER
            .iter()
            .position(|o| *o == s.signature.as_str())
            .unwrap_or(usize::MAX)
    });
    target_wrld.subrecords = merged;
    payload.copied = copied;
    payload
}

fn carry_target_signatures() -> Vec<&'static str> {
    let mut sigs: Vec<&'static str> = NONREF_CARRY.to_vec();
    sigs.push(TARGET_MAP_IMAGE);
    sigs.push(MAX_HEIGHT_DATA);
    sigs.push("FULL");
    for (sig, _) in FORMID_CARRY {
        sigs.push(sig);
    }
    sigs
}

// The MHDT grid is an 8-byte dims header (min/max cell X/Y, int16) followed by
// one 2x2-corner byte cell (4 bytes) per grid cell. Reject a source blob that is
// not exactly the size its own dimensions describe.
fn mhdt_grid_is_well_formed(data: &[u8]) -> bool {
    if data.len() < 8 {
        return false;
    }
    let min_x = i16::from_le_bytes([data[0], data[1]]) as i32;
    let min_y = i16::from_le_bytes([data[2], data[3]]) as i32;
    let max_x = i16::from_le_bytes([data[4], data[5]]) as i32;
    let max_y = i16::from_le_bytes([data[6], data[7]]) as i32;
    let width = max_x - min_x + 1;
    let height = max_y - min_y + 1;
    if width <= 0 || height <= 0 {
        return false;
    }
    data.len() == 8 + (width as usize) * (height as usize) * 4
}

fn make_subrecord(signature: &str, data: Bytes) -> ParsedSubrecord {
    ParsedSubrecord {
        signature: SmolStr::new(signature),
        data,
        semantic_type: None,
    }
}

fn normalize_map_image_path(data: Bytes) -> Bytes {
    let mut bytes = data.as_ref();
    if bytes.len() >= 5
        && bytes[..4].eq_ignore_ascii_case(b"data")
        && matches!(bytes[4], b'\\' | b'/')
    {
        bytes = &bytes[5..];
    }
    if bytes.len() >= 9
        && bytes[..8].eq_ignore_ascii_case(b"textures")
        && matches!(bytes[8], b'\\' | b'/')
    {
        bytes = &bytes[9..];
    }
    if is_fo76_appalachia_legacy_map_image(bytes) {
        return Bytes::from_static(FO76_APPALACHIA_PIPBOY_MAP_IMAGE);
    }
    if bytes.len() == data.len() {
        data
    } else {
        Bytes::copy_from_slice(bytes)
    }
}

fn is_fo76_appalachia_legacy_map_image(bytes: &[u8]) -> bool {
    let without_nul = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    let normalized = String::from_utf8_lossy(without_nul)
        .replace('\\', "/")
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "regionmap/76map.dds" | "interface/pip-boy/appalachia.dds"
    )
}

fn wrld_editor_id(record: &ParsedRecord) -> String {
    let subrecords = source_subrecords_for_record(record);
    subrecords
        .iter()
        .find(|s| s.signature.as_str() == "EDID")
        .map(|s| {
            let end = s.data.iter().position(|&b| b == 0).unwrap_or(s.data.len());
            String::from_utf8_lossy(&s.data[..end]).into_owned()
        })
        .unwrap_or_default()
}

fn source_subrecords_for_record(record: &ParsedRecord) -> Cow<'_, [ParsedSubrecord]> {
    if (record.flags & COMPRESSED_RECORD_FLAG) != 0
        && record.raw_payload.is_some()
        && record.parse_error.is_none()
    {
        if let Some(raw_payload) = &record.raw_payload {
            if let Ok(decoded) = parse_compressed_subrecords_from_payload(raw_payload) {
                return Cow::Owned(decoded);
            }
        }
    }
    effective_subrecords_for_record(record)
}

fn find_wrld<'a>(plugin: &'a ParsedPlugin, editor_id: &str) -> Option<&'a ParsedRecord> {
    for item in &plugin.root_items {
        let ParsedItem::Group(group) = item else {
            continue;
        };
        if group.group_type != 0 || group.label != *b"WRLD" {
            continue;
        }
        for child in &group.children {
            if let ParsedItem::Record(record) = child {
                if record.signature.as_str() == "WRLD"
                    && wrld_editor_id(record).eq_ignore_ascii_case(editor_id)
                {
                    return Some(record);
                }
            }
        }
    }
    None
}

fn find_wrld_mut<'a>(
    plugin: &'a mut ParsedPlugin,
    editor_id: &str,
) -> Option<&'a mut ParsedRecord> {
    for item in &mut plugin.root_items {
        let ParsedItem::Group(group) = item else {
            continue;
        };
        if group.group_type != 0 || group.label != *b"WRLD" {
            continue;
        }
        for child in &mut group.children {
            if let ParsedItem::Record(record) = child {
                if record.signature.as_str() == "WRLD"
                    && wrld_editor_id(record).eq_ignore_ascii_case(editor_id)
                {
                    return Some(record);
                }
            }
        }
    }
    None
}

fn index_object_signatures(items: &[ParsedItem], out: &mut HashMap<u32, SmolStr>) {
    for item in items {
        match item {
            ParsedItem::Record(record) => {
                out.entry(record.form_id & 0x00FF_FFFF)
                    .or_insert_with(|| record.signature.clone());
            }
            ParsedItem::Group(group) => index_object_signatures(&group.children, out),
        }
    }
}

fn remap_leading_formid(
    blob: &Bytes,
    own_index: u32,
    obj_sig: &HashMap<u32, SmolStr>,
    want: &str,
) -> Option<Bytes> {
    if blob.len() < 4 {
        return None;
    }
    let raw = u32::from_le_bytes([blob[0], blob[1], blob[2], blob[3]]);
    if raw == 0 {
        return None;
    }
    let object_id = raw & 0x00FF_FFFF;
    let matches_want = matches!(obj_sig.get(&object_id), Some(sig) if sig.as_str() == want);
    if !matches_want {
        return None;
    }
    // Source-local refs (FO76 master 0) are rewritten to the target own index;
    // an already-master-qualified ref keeps its bytes.
    let master = raw >> 24;
    let remapped = if master == 0 {
        (own_index << 24) | object_id
    } else {
        raw
    };
    let mut out = remapped.to_le_bytes().to_vec();
    out.extend_from_slice(&blob[4..]);
    Some(Bytes::from(out))
}

fn fallback_wrld_water_formid(
    sig: &str,
    want: &str,
    blob: &Bytes,
    target_game: &str,
    fo4_master_index: Option<u32>,
) -> Option<Bytes> {
    if !matches!(sig, "NAM2" | "NAM3") || want != "WATR" || !target_game.eq_ignore_ascii_case("fo4")
    {
        return None;
    }
    if blob.len() < 4 {
        return None;
    }
    let raw = u32::from_le_bytes([blob[0], blob[1], blob[2], blob[3]]);
    if raw == 0 {
        return None;
    }
    let master_index = fo4_master_index?;
    if master_index > 0xFF {
        return None;
    }
    let remapped = (master_index << 24) | FO4_EXT_LAKE_WATER_OBJECT_ID;
    let mut out = remapped.to_le_bytes().to_vec();
    out.extend_from_slice(&blob[4..]);
    Some(Bytes::from(out))
}

fn resolve_source_full_text(slot: &NativePluginSlot, editor_id: &str) -> Option<String> {
    let plugin = &slot.parsed;
    let wrld = find_wrld(plugin, editor_id)?;
    let subrecords = source_subrecords_for_record(wrld);
    let full = subrecords.iter().find(|s| s.signature.as_str() == "FULL")?;
    if (plugin.header.flags & TES4_FLAG_LOCALIZED) != 0 {
        if full.data.len() < 4 {
            return None;
        }
        let id = u32::from_le_bytes([full.data[0], full.data[1], full.data[2], full.data[3]]);
        resolve_localized_text(&slot.strings, id)
    } else {
        let end = full
            .data
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(full.data.len());
        if end == 0 {
            None
        } else {
            Some(String::from_utf8_lossy(&full.data[..end]).into_owned())
        }
    }
}

pub(crate) fn resolve_localized_text(strings: &LocalizedStringsState, id: u32) -> Option<String> {
    if id == 0 {
        return None;
    }
    if let Some(text) = strings
        .by_language
        .get(strings.default_language.as_str())
        .and_then(|table| table.get(&id))
    {
        if !text.is_empty() {
            return Some(text.clone());
        }
    }
    strings
        .by_language
        .values()
        .find_map(|table| table.get(&id).filter(|s| !s.is_empty()).cloned())
}

fn allocate_target_string_id(strings: &LocalizedStringsState) -> Option<u32> {
    let mut used: HashSet<u32> = strings.table_types.keys().copied().collect();
    for table in strings.by_language.values() {
        used.extend(table.keys().copied());
    }

    if let Some(start) = used.iter().copied().max().and_then(|id| id.checked_add(1)) {
        if let Some(candidate) = first_free_string_id_from(&used, start) {
            return Some(candidate);
        }
    }
    first_free_string_id_from(&used, 1)
}

fn first_free_string_id_from(used: &HashSet<u32>, start: u32) -> Option<u32> {
    let mut candidate = start.max(1);
    loop {
        if !used.contains(&candidate) {
            return Some(candidate);
        }
        if candidate == u32::MAX {
            return None;
        }
        candidate += 1;
    }
}

fn register_target_string(strings: &mut LocalizedStringsState, id: u32, text: &str) {
    // WRLD FULL is a general string → the .STRINGS table. infer_localized_table_types
    // reclassifies by referencing subrecord at save time, so this is just the seed.
    strings
        .table_types
        .entry(id)
        .or_insert_with(|| "strings".to_string());
    if strings.by_language.is_empty() {
        let lang = if strings.default_language.is_empty() {
            "en".to_string()
        } else {
            strings.default_language.clone()
        };
        strings
            .by_language
            .entry(lang)
            .or_default()
            .insert(id, text.to_string());
    } else {
        for table in strings.by_language.values_mut() {
            table.insert(id, text.to_string());
        }
    }
}

pub(crate) fn plugin_handle_carry_worldspace_header_from_source_json(
    source_handle_id: u64,
    target_handle_id: u64,
    source_worldspace_editor_id: &str,
    target_worldspace_editor_id: &str,
) -> PyResult<String> {
    if source_handle_id == target_handle_id {
        return Err(PyValueError::new_err(
            "source and target handles must be different",
        ));
    }

    let mut store = plugin_handle_store_ref().lock().unwrap();
    if !store.contains_key(&source_handle_id) {
        return Err(PyKeyError::new_err(format!(
            "unknown source plugin handle: {source_handle_id}"
        )));
    }
    let mut target_slot = store.remove(&target_handle_id).ok_or_else(|| {
        PyKeyError::new_err(format!("unknown target plugin handle: {target_handle_id}"))
    })?;

    // Resolve the source worldspace name (FULL) to text. FO76 is localized, so
    // FULL is a string id resolved via the source strings; tolerate an inline
    // (non-localized) source too.
    let source_full_text: Option<String> = {
        let source_slot = store.get(&source_handle_id).ok_or_else(|| {
            PyKeyError::new_err(format!("unknown source plugin handle: {source_handle_id}"))
        })?;
        resolve_source_full_text(source_slot, source_worldspace_editor_id)
    };

    // Encode FULL for the target: inline zstring for a non-localized target, or a
    // freshly allocated string id (registered into the target strings tables, so
    // the save rewrites the .STRINGS) for a localized target.
    let target_localized = (target_slot.parsed.header.flags & TES4_FLAG_LOCALIZED) != 0;
    let target_has_wrld = find_wrld(&target_slot.parsed, target_worldspace_editor_id).is_some();
    let full_bytes: Option<Bytes> = match source_full_text {
        Some(text) if !text.is_empty() && target_has_wrld => {
            if target_localized {
                allocate_target_string_id(&target_slot.strings).map(|id| {
                    register_target_string(&mut target_slot.strings, id, &text);
                    Bytes::from(id.to_le_bytes().to_vec())
                })
            } else {
                let mut bytes = text.into_bytes();
                bytes.push(0);
                Some(Bytes::from(bytes))
            }
        }
        _ => None,
    };

    let payload = {
        let source_slot = store.get(&source_handle_id).ok_or_else(|| {
            PyKeyError::new_err(format!("unknown source plugin handle: {source_handle_id}"))
        })?;
        carry_worldspace_header_from_source(
            &source_slot.parsed,
            &mut target_slot.parsed,
            source_worldspace_editor_id,
            target_worldspace_editor_id,
            full_bytes,
        )
    };

    if payload.copied > 0 {
        target_slot.clear_record_count_cache();
        target_slot.invalidate_sections();
    }
    store.insert(target_handle_id, target_slot);
    serde_json::to_string(&payload).map_err(|err| {
        PyValueError::new_err(format!(
            "failed to encode worldspace header carry result: {err}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(signature: &str, data: Vec<u8>) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: SmolStr::new(signature),
            data: Bytes::from(data),
            semantic_type: None,
        }
    }

    fn edid(name: &str) -> ParsedSubrecord {
        let mut data = name.as_bytes().to_vec();
        data.push(0);
        sub("EDID", data)
    }

    fn mhdt_blob(min_x: i16, min_y: i16, max_x: i16, max_y: i16, fill: u8) -> Vec<u8> {
        let width = (max_x - min_x + 1) as usize;
        let height = (max_y - min_y + 1) as usize;
        let mut data = Vec::new();
        data.extend_from_slice(&min_x.to_le_bytes());
        data.extend_from_slice(&min_y.to_le_bytes());
        data.extend_from_slice(&max_x.to_le_bytes());
        data.extend_from_slice(&max_y.to_le_bytes());
        data.extend(std::iter::repeat(fill).take(width * height * 4));
        data
    }

    fn record(signature: &str, form_id: u32, subs: Vec<ParsedSubrecord>) -> ParsedItem {
        ParsedItem::Record(ParsedRecord {
            signature: SmolStr::new(signature),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: Some(131),
            version2: Some(1),
            subrecords: subs,
            raw_payload: None,
            parse_error: None,
        })
    }

    fn compressed_record(
        signature: &str,
        form_id: u32,
        parsed_subrecords: Vec<ParsedSubrecord>,
        raw_subrecords: &[ParsedSubrecord],
    ) -> ParsedItem {
        ParsedItem::Record(ParsedRecord {
            signature: SmolStr::new(signature),
            form_id,
            flags: COMPRESSED_RECORD_FLAG,
            version_control: 0,
            form_version: Some(131),
            version2: Some(1),
            subrecords: parsed_subrecords,
            raw_payload: Some(compressed_payload(raw_subrecords)),
            parse_error: None,
        })
    }

    fn compressed_payload(subrecords: &[ParsedSubrecord]) -> Bytes {
        let mut expanded = Vec::new();
        for subrecord in subrecords {
            expanded.extend_from_slice(subrecord.signature.as_str().as_bytes());
            expanded.extend_from_slice(&(subrecord.data.len() as u16).to_le_bytes());
            expanded.extend_from_slice(&subrecord.data);
        }
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &expanded).expect("write compressed WRLD payload");
        let compressed = encoder.finish().expect("finish compressed WRLD payload");
        let mut payload = (expanded.len() as u32).to_le_bytes().to_vec();
        payload.extend_from_slice(&compressed);
        Bytes::from(payload)
    }

    fn top_group(signature: &str, children: Vec<ParsedItem>) -> ParsedItem {
        let b = signature.as_bytes();
        ParsedItem::Group(ParsedGroup {
            label: [b[0], b[1], b[2], b[3]],
            group_type: 0,
            tail: Bytes::new(),
            children,
        })
    }

    fn plugin(name: &str, game: &str, masters: Vec<String>, root: Vec<ParsedItem>) -> ParsedPlugin {
        let mut header = ParsedPluginHeader::default_for_test();
        header.masters = masters;
        ParsedPlugin {
            plugin_name: name.to_string(),
            file_path: String::new(),
            header_size: MODERN_HEADER_SIZE,
            header,
            root_items: root,
            game: Some(game.to_string()),
        }
    }

    const MNAM_BYTES: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    const ONAM_BYTES: [u8; 16] = [
        17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
    ];
    const NAM0_BYTES: [u8; 8] = [0x42, 0x30, 0x3E, 0xC9, 0x33, 0xE8, 0x58, 0xC9];
    const NAM9_BYTES: [u8; 8] = [0xCD, 0xCC, 0x59, 0x49, 0x66, 0x66, 0x66, 0x49];
    const NAM5_PATH: &[u8] = b"Interface\\Pip-Boy\\Appalachia.dds\0";
    const NAM5_DATA_PREFIX_PATH: &[u8] = b"data\\Textures\\RegionMap\\76Map.dds\0";
    const PAPERMAP_PATH: &[u8] = b"Interface\\Pip-Boy\\papermap_city_d.dds\0";

    fn source_plugin() -> ParsedPlugin {
        source_plugin_with_map_path(NAM5_PATH)
    }

    fn source_plugin_with_map_path(map_path: &[u8]) -> ParsedPlugin {
        // FO76 WRLD with a populated header. FULL is an lstring id; XLCN/CNAM/NAM2/
        // NAM3 are master-0 source-local formids; NAM5 is the map image path.
        let wrld = record(
            "WRLD",
            0x0000_0F99,
            vec![
                edid("APPALACHIA"),
                sub("FULL", 0x1234_5678u32.to_le_bytes().to_vec()),
                sub("XLCN", 0x0001_558Cu32.to_le_bytes().to_vec()),
                sub("CNAM", 0x0000_727Du32.to_le_bytes().to_vec()),
                sub("NAM2", 0x0000_0018u32.to_le_bytes().to_vec()),
                sub("NAM3", 0x0000_0018u32.to_le_bytes().to_vec()),
                sub("NAM4", 0.0f32.to_le_bytes().to_vec()),
                sub(
                    "DNAM",
                    [1024.0f32.to_le_bytes(), 0.0f32.to_le_bytes()].concat(),
                ),
                sub("ICON", vec![0]),
                sub("MNAM", MNAM_BYTES.to_vec()),
                sub("ONAM", ONAM_BYTES.to_vec()),
                sub("NAM0", NAM0_BYTES.to_vec()),
                sub("NAM9", NAM9_BYTES.to_vec()),
                sub("NAM5", map_path.to_vec()),
            ],
        );
        plugin(
            "SeventySix.esm",
            "fo76",
            vec![],
            vec![top_group("WRLD", vec![wrld])],
        )
    }

    fn raw_payload_source_plugin() -> ParsedPlugin {
        let raw_subrecords = vec![
            edid("APPALACHIA"),
            sub("FULL", 0x1234_5678u32.to_le_bytes().to_vec()),
            sub("XLCN", 0x0001_558Cu32.to_le_bytes().to_vec()),
            sub("CNAM", 0x0000_727Du32.to_le_bytes().to_vec()),
            sub("NAM2", 0x0000_0018u32.to_le_bytes().to_vec()),
            sub("NAM3", 0x0000_0018u32.to_le_bytes().to_vec()),
            sub("NAM4", 0.0f32.to_le_bytes().to_vec()),
            sub(
                "DNAM",
                [1024.0f32.to_le_bytes(), 0.0f32.to_le_bytes()].concat(),
            ),
            sub("MNAM", MNAM_BYTES.to_vec()),
            sub("ONAM", ONAM_BYTES.to_vec()),
            sub("NAM0", NAM0_BYTES.to_vec()),
            sub("NAM9", NAM9_BYTES.to_vec()),
            sub("NAM5", NAM5_PATH.to_vec()),
        ];
        let wrld = compressed_record(
            "WRLD",
            0x0000_0F99,
            vec![edid("APPALACHIA")],
            &raw_subrecords,
        );
        plugin(
            "SeventySix.esm",
            "fo76",
            vec![],
            vec![top_group("WRLD", vec![wrld])],
        )
    }

    fn skeleton_target() -> ParsedPlugin {
        // FO4 skeleton WRLD + the converted records the formid carries resolve to
        // (own index 1 = one master). Object ids match the source refs.
        let wrld = record(
            "WRLD",
            0x0100_0F99,
            vec![
                edid("APPALACHIA"),
                sub("NAMA", 1.0f32.to_le_bytes().to_vec()),
                sub("DATA", vec![0]),
                sub("NAM0", vec![0; 8]),
                sub("NAM9", vec![0; 8]),
            ],
        );
        plugin(
            "Converted.esm",
            "fo4",
            vec!["Fallout4.esm".to_string()],
            vec![
                top_group("WRLD", vec![wrld]),
                top_group("LCTN", vec![record("LCTN", 0x0101_558C, vec![edid("LOC")])]),
                top_group("CLMT", vec![record("CLMT", 0x0100_727D, vec![edid("CLM")])]),
                top_group("WATR", vec![record("WATR", 0x0100_0018, vec![edid("WTR")])]),
            ],
        )
    }

    fn wrld_subs(plugin: &ParsedPlugin) -> Vec<ParsedSubrecord> {
        for item in &plugin.root_items {
            if let ParsedItem::Group(g) = item {
                if g.label == *b"WRLD" {
                    for child in &g.children {
                        if let ParsedItem::Record(r) = child {
                            return r.subrecords.clone();
                        }
                    }
                }
            }
        }
        Vec::new()
    }

    fn data_for(subs: &[ParsedSubrecord], sig: &str) -> Option<Bytes> {
        subs.iter()
            .find(|s| s.signature.as_str() == sig)
            .map(|s| s.data.clone())
    }

    #[test]
    fn drops_stale_fo76_runtime_tables_from_target_wrld() {
        let source = source_plugin();
        let mut target = skeleton_target();
        let wrld = find_wrld_mut(&mut target, "APPALACHIA").expect("target WRLD");
        for sig in UNSAFE_FO76_RUNTIME_TABLES {
            wrld.subrecords.push(sub(sig, vec![0, 1, 2, 3]));
        }

        let report = carry_worldspace_header_from_source(
            &source,
            &mut target,
            "APPALACHIA",
            "APPALACHIA",
            None,
        );

        assert_eq!(report.copied, 11, "warnings={:?}", report.warnings);
        let subs = wrld_subs(&target);
        for sig in UNSAFE_FO76_RUNTIME_TABLES {
            assert!(
                data_for(&subs, sig).is_none(),
                "{sig} must not survive FO76->FO4 WRLD header carry"
            );
        }
        assert!(data_for(&subs, "EDID").is_some());
        assert!(data_for(&subs, "NAMA").is_some());
    }

    #[test]
    fn carries_well_formed_wrld_max_height_data() {
        // Appalachia's real MHDT rectangle (X:-58..60, Y:-56..61) intentionally
        // exceeds the MNAM map bounds — carry must not reject on that basis.
        let mut source = source_plugin();
        let blob = mhdt_blob(-58, -56, 60, 61, 0x42);
        let expected = blob.clone();
        find_wrld_mut(&mut source, "APPALACHIA")
            .unwrap()
            .subrecords
            .push(sub("MHDT", blob));
        let mut target = skeleton_target();

        let report = carry_worldspace_header_from_source(
            &source,
            &mut target,
            "APPALACHIA",
            "APPALACHIA",
            None,
        );

        // 11 header fields + MHDT = 12.
        assert_eq!(report.copied, 12, "warnings={:?}", report.warnings);
        let subs = wrld_subs(&target);
        assert_eq!(
            data_for(&subs, "MHDT").unwrap().as_ref(),
            expected.as_slice()
        );

        // MHDT sits early in FO4 schema order (before ICON), after EDID.
        let pos = |sig: &str| subs.iter().position(|s| s.signature.as_str() == sig);
        assert!(pos("EDID").unwrap() < pos("MHDT").unwrap());
        assert!(pos("MHDT").unwrap() < pos("ICON").unwrap());
    }

    #[test]
    fn drops_malformed_wrld_max_height_data() {
        let mut source = source_plugin();
        let mut blob = mhdt_blob(-58, -56, 60, 61, 0);
        blob.truncate(blob.len() - 10); // grid shorter than its dims claim
        find_wrld_mut(&mut source, "APPALACHIA")
            .unwrap()
            .subrecords
            .push(sub("MHDT", blob));
        let mut target = skeleton_target();

        let report = carry_worldspace_header_from_source(
            &source,
            &mut target,
            "APPALACHIA",
            "APPALACHIA",
            None,
        );

        assert_eq!(report.copied, 11, "warnings={:?}", report.warnings);
        let subs = wrld_subs(&target);
        assert!(data_for(&subs, "MHDT").is_none());
        assert!(report.warnings.iter().any(|w| w.contains("MHDT")));
    }

    #[test]
    fn carries_and_translates_wrld_header() {
        let source = source_plugin();
        let mut target = skeleton_target();

        let report = carry_worldspace_header_from_source(
            &source,
            &mut target,
            "APPALACHIA",
            "APPALACHIA",
            None,
        );

        // DNAM/MNAM/NAM4/ONAM/NAM0/NAM9 carried verbatim, NAM5→ICON, 4 formid links = 11.
        assert_eq!(report.copied, 11, "warnings={:?}", report.warnings);

        let subs = wrld_subs(&target);

        // Legacy Appalachia NAM5 paths become the FO4-readable Pip-Boy paper map.
        assert_eq!(data_for(&subs, "ICON").unwrap().as_ref(), PAPERMAP_PATH);
        assert!(
            data_for(&subs, "NAM5").is_none(),
            "source NAM5 must be renamed, not duplicated"
        );

        // Verbatim struct carries.
        assert_eq!(data_for(&subs, "MNAM").unwrap().as_ref(), &MNAM_BYTES);
        assert_eq!(data_for(&subs, "ONAM").unwrap().as_ref(), &ONAM_BYTES);
        assert_eq!(
            data_for(&subs, "DNAM").unwrap().as_ref(),
            [1024.0f32.to_le_bytes(), 0.0f32.to_le_bytes()]
                .concat()
                .as_slice()
        );
        assert_eq!(
            data_for(&subs, "NAM4").unwrap().as_ref(),
            &0.0f32.to_le_bytes()
        );
        assert_eq!(data_for(&subs, "NAM0").unwrap().as_ref(), &NAM0_BYTES);
        assert_eq!(data_for(&subs, "NAM9").unwrap().as_ref(), &NAM9_BYTES);

        // formid links remapped master-0 → own index 1, validated by target sig.
        assert_eq!(
            data_for(&subs, "XLCN").unwrap().as_ref(),
            &0x0101_558Cu32.to_le_bytes()
        );
        assert_eq!(
            data_for(&subs, "CNAM").unwrap().as_ref(),
            &0x0100_727Du32.to_le_bytes()
        );
        assert_eq!(
            data_for(&subs, "NAM2").unwrap().as_ref(),
            &0x0100_0018u32.to_le_bytes()
        );
        assert_eq!(
            data_for(&subs, "NAM3").unwrap().as_ref(),
            &0x0100_0018u32.to_le_bytes()
        );

        // FULL not provided here (full_bytes=None) → must NOT be carried.
        assert!(data_for(&subs, "FULL").is_none());

        // Skeleton fields survive.
        assert!(data_for(&subs, "EDID").is_some());
        assert!(data_for(&subs, "NAMA").is_some());

        // Subrecords are in FO4 schema order (monotonic, no out-of-order).
        let positions: Vec<usize> = subs
            .iter()
            .map(|s| {
                FO4_WRLD_ORDER
                    .iter()
                    .position(|o| *o == s.signature.as_str())
                    .unwrap_or(usize::MAX)
            })
            .collect();
        assert!(
            positions.windows(2).all(|w| w[0] <= w[1]),
            "subrecords out of schema order: {:?}",
            subs.iter()
                .map(|s| s.signature.to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn strips_data_prefix_from_carried_wrld_map_image() {
        let source = source_plugin_with_map_path(NAM5_DATA_PREFIX_PATH);
        let mut target = skeleton_target();

        let report = carry_worldspace_header_from_source(
            &source,
            &mut target,
            "APPALACHIA",
            "APPALACHIA",
            None,
        );

        assert_eq!(report.copied, 11, "warnings={:?}", report.warnings);
        let subs = wrld_subs(&target);
        assert_eq!(data_for(&subs, "ICON").unwrap().as_ref(), PAPERMAP_PATH);
    }

    #[test]
    fn preserves_wrld_map_image_without_texture_prefix() {
        let path = Bytes::from_static(b"Interface\\Pip-Boy\\WorldMap_d.dds\0");
        assert_eq!(normalize_map_image_path(path.clone()), path);
    }

    #[test]
    fn resolves_full_from_compressed_raw_payload() {
        let mut source = raw_payload_source_plugin();
        source.header.flags = TES4_FLAG_LOCALIZED;
        let mut strings = LocalizedStringsState::default();
        strings.default_language = "en".to_string();
        strings
            .by_language
            .entry("en".to_string())
            .or_default()
            .insert(0x1234_5678, "Appalachia".to_string());
        let slot = NativePluginSlot {
            parsed: source,
            strings,
            localized_text_index: None,
            record_count_cache: None,
            sections: PluginIndexSections::default(),
            lazy: None,
        };

        assert_eq!(
            resolve_source_full_text(&slot, "APPALACHIA").as_deref(),
            Some("Appalachia")
        );
    }

    #[test]
    fn carries_wrld_header_from_compressed_raw_payload() {
        let source = raw_payload_source_plugin();
        let mut target = skeleton_target();

        let report = carry_worldspace_header_from_source(
            &source,
            &mut target,
            "APPALACHIA",
            "APPALACHIA",
            None,
        );

        assert_eq!(report.copied, 11, "warnings={:?}", report.warnings);
        let subs = wrld_subs(&target);
        assert_eq!(data_for(&subs, "ICON").unwrap().as_ref(), PAPERMAP_PATH);
        assert_eq!(data_for(&subs, "MNAM").unwrap().as_ref(), &MNAM_BYTES);
        assert_eq!(data_for(&subs, "ONAM").unwrap().as_ref(), &ONAM_BYTES);
        assert_eq!(data_for(&subs, "NAM0").unwrap().as_ref(), &NAM0_BYTES);
        assert_eq!(data_for(&subs, "NAM9").unwrap().as_ref(), &NAM9_BYTES);
    }

    #[test]
    fn allocates_string_id_after_normal_max() {
        let mut strings = LocalizedStringsState::default();
        strings.table_types.insert(7, "strings".to_string());
        strings
            .by_language
            .entry("en".to_string())
            .or_default()
            .insert(10, "Existing".to_string());

        assert_eq!(allocate_target_string_id(&strings), Some(11));
    }

    #[test]
    fn allocates_string_id_when_sentinel_max_is_used() {
        let mut strings = LocalizedStringsState::default();
        strings.default_language = "en".to_string();
        strings.table_types.insert(u32::MAX, "strings".to_string());
        let table = strings.by_language.entry("en".to_string()).or_default();
        table.insert(1, "One".to_string());
        table.insert(2, "Two".to_string());
        table.insert(3, "Three".to_string());
        table.insert(u32::MAX, "LOC_FFFFFFFF".to_string());

        let id = allocate_target_string_id(&strings).expect("free string id");
        assert_eq!(id, 4);

        register_target_string(&mut strings, id, "Appalachia");
        assert_eq!(
            strings.by_language["en"].get(&id).map(String::as_str),
            Some("Appalachia")
        );
        assert_eq!(
            strings.table_types.get(&id).map(String::as_str),
            Some("strings")
        );
    }

    #[test]
    fn localized_full_carry_allocates_past_sentinel_and_inserts_full() {
        let source = source_plugin();
        let mut target = skeleton_target();
        target.header.flags = TES4_FLAG_LOCALIZED;

        let mut target_strings = LocalizedStringsState::default();
        target_strings.default_language = "en".to_string();
        target_strings
            .table_types
            .insert(u32::MAX, "strings".to_string());
        target_strings
            .by_language
            .entry("en".to_string())
            .or_default()
            .insert(u32::MAX, "LOC_FFFFFFFF".to_string());

        let id = allocate_target_string_id(&target_strings).expect("free string id");
        register_target_string(&mut target_strings, id, "Appalachia");
        let report = carry_worldspace_header_from_source(
            &source,
            &mut target,
            "APPALACHIA",
            "APPALACHIA",
            Some(Bytes::from(id.to_le_bytes().to_vec())),
        );

        assert_eq!(report.copied, 12, "warnings={:?}", report.warnings);
        let subs = wrld_subs(&target);
        assert_eq!(data_for(&subs, "FULL").unwrap().as_ref(), &id.to_le_bytes());
        assert_eq!(
            target_strings.by_language["en"]
                .get(&id)
                .map(String::as_str),
            Some("Appalachia")
        );
    }

    #[test]
    fn skips_formid_when_target_signature_mismatches() {
        let source = source_plugin();
        let mut target = skeleton_target();
        // Replace the LCTN target with a STAT at the same object id: XLCN must not
        // be carried onto a record of the wrong type.
        target.root_items[1] = top_group(
            "STAT",
            vec![record("STAT", 0x0101_558C, vec![edid("STAT")])],
        );

        let report = carry_worldspace_header_from_source(
            &source,
            &mut target,
            "APPALACHIA",
            "APPALACHIA",
            None,
        );

        let subs = wrld_subs(&target);
        assert!(
            data_for(&subs, "XLCN").is_none(),
            "XLCN must skip wrong-type target"
        );
        assert_eq!(report.copied, 10);
        assert!(report.warnings.iter().any(|w| w.contains("XLCN")));
    }

    #[test]
    fn falls_back_to_fo4_ext_lake_water_when_target_water_missing() {
        let source = source_plugin();
        let mut target = skeleton_target();
        target
            .root_items
            .retain(|item| !matches!(item, ParsedItem::Group(group) if group.label == *b"WATR"));

        let report = carry_worldspace_header_from_source(
            &source,
            &mut target,
            "APPALACHIA",
            "APPALACHIA",
            None,
        );

        let subs = wrld_subs(&target);
        assert_eq!(
            data_for(&subs, "NAM2").unwrap().as_ref(),
            &0x000C_8633u32.to_le_bytes()
        );
        assert_eq!(
            data_for(&subs, "NAM3").unwrap().as_ref(),
            &0x000C_8633u32.to_le_bytes()
        );
        assert_eq!(report.copied, 11, "warnings={:?}", report.warnings);
        assert_eq!(
            report
                .warnings
                .iter()
                .filter(|warning| warning.contains("ExtLakeWater"))
                .count(),
            2
        );
        assert!(
            !report
                .warnings
                .iter()
                .any(|warning| warning.contains("dropped WRLD NAM2"))
        );
        assert!(
            !report
                .warnings
                .iter()
                .any(|warning| warning.contains("dropped WRLD NAM3"))
        );
    }

    #[test]
    fn carries_full_bytes_in_schema_order() {
        let source = source_plugin();
        let mut target = skeleton_target();
        let full = b"Appalachia\0".to_vec();

        let report = carry_worldspace_header_from_source(
            &source,
            &mut target,
            "APPALACHIA",
            "APPALACHIA",
            Some(Bytes::from(full.clone())),
        );

        // 11 header fields + FULL = 12.
        assert_eq!(report.copied, 12, "warnings={:?}", report.warnings);

        let subs = wrld_subs(&target);
        assert_eq!(data_for(&subs, "FULL").unwrap().as_ref(), full.as_slice());

        // FULL sits before WCTR/XLCN in FO4 schema order: confirm it precedes XLCN.
        let pos = |sig: &str| subs.iter().position(|s| s.signature.as_str() == sig);
        assert!(pos("FULL").unwrap() < pos("XLCN").unwrap());
        assert!(pos("EDID").unwrap() < pos("FULL").unwrap());
    }
}
