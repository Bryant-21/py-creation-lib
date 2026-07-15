use super::*;
use pyo3::exceptions::{PyKeyError, PyValueError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Instant;

#[derive(Default, Serialize)]
pub struct CellSliceRootsPayload {
    pub cell_form_keys: Vec<String>,
    pub placed_form_keys: Vec<String>,
    pub static_base_form_keys: Vec<String>,
    pub leveled_base_entry_form_keys: Vec<String>,
    pub linked_ref_keyword_form_keys: Vec<String>,
    pub layer_form_keys: Vec<String>,
    pub location_form_keys: Vec<String>,
    pub location_data_form_keys: Vec<String>,
    pub region_form_keys: Vec<String>,
    pub region_data_form_keys: Vec<String>,
    pub audio_data_form_keys: Vec<String>,
    pub worldspace_form_keys: Vec<String>,
    pub worldspace_data_form_keys: Vec<String>,
    pub cell_children: BTreeMap<String, CellChildrenPayload>,
    pub cell_grids: BTreeMap<String, CellGridPayload>,
    /// When `include_worldspace_persistent_cell` was requested, the
    /// `cell_children` key holding the worldspace persistent cell's children
    /// (keyed by the WRLD form key, not a CELL key). Native orchestrators must
    /// exclude this entry when routing grid-cell children. Skipped from JSON
    /// when unset so the pinned legacy payload stays byte-identical.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worldspace_persistent_children_key: Option<String>,
    pub warnings: Vec<String>,
    pub timing: BTreeMap<String, u128>,
}

#[derive(Serialize)]
pub struct CellGridPayload {
    pub x: i32,
    pub y: i32,
}

#[derive(Default, Serialize)]
struct WorldspaceTerrainIdsPayload {
    world_form_id: Option<u32>,
    world_editor_id: String,
    cells: Vec<WorldspaceTerrainCellIds>,
    warnings: Vec<String>,
    timing: BTreeMap<String, u128>,
}

#[derive(Serialize)]
struct WorldspaceTerrainCellIds {
    x: i32,
    y: i32,
    cell_form_id: u32,
    cell_editor_id: String,
    land_form_id: Option<u32>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct CellChildrenPayload {
    #[serde(rename = "Persistent")]
    pub persistent: Vec<String>,
    #[serde(rename = "Temporary")]
    pub temporary: Vec<String>,
}

#[derive(Default, Serialize)]
pub struct CellSliceInsertPayload {
    pub cells_touched: usize,
    pub children_inserted: usize,
    pub children_rebucketed: usize,
    pub mapped_form_refs: usize,
    pub leveled_bases_resolved: usize,
    pub schema_subrecords_dropped: usize,
    pub child_form_ids_reallocated: usize,
    pub missing_base_children: usize,
    pub cell_region_refs_rewritten: usize,
    pub skipped_children: Vec<String>,
    pub warnings: Vec<String>,
    pub timing: BTreeMap<String, u128>,
}

#[derive(Default, Serialize)]
pub struct SynthesizePersistentCellPayload {
    pub cell_synthesized: bool,
    pub persistent_cell_form_key: String,
    pub persistent_refs_converted: usize,
    pub persistent_refs_skipped: usize,
    pub skip_reasons: BTreeMap<String, usize>,
    /// Per (NAME base FormKey, skip reason) tally over the COMPLETE skip set —
    /// unbounded by record count (one entry per distinct base/reason pair, not
    /// per ref). Lets a caller identify which base the dropped persistent refs
    /// (e.g. the missing MapMarkers, base `000010`) resolve to without a full
    /// re-export. Diagnostic-only; never affects conversion output.
    pub skip_base_histogram: BTreeMap<String, usize>,
    pub mapped_form_refs: usize,
    pub leveled_bases_resolved: usize,
    pub schema_subrecords_dropped: usize,
    pub child_form_ids_reallocated: usize,
    /// Bounded (200) sample of skipped refs, each formatted
    /// `<source_form_key>|base=<name_base_form_key>|reason=<reason>` so the
    /// Python layer can log the offending base per skipped ref.
    pub skipped_children: Vec<String>,
    pub warnings: Vec<String>,
    pub timing: BTreeMap<String, u128>,
}

#[derive(Default, Serialize)]
struct CellLocationSyncPayload {
    locations_indexed: usize,
    location_conflicts: usize,
    cells_changed: usize,
    cells_retagged: usize,
    cells_already_tagged: usize,
    warnings: Vec<String>,
    timing: BTreeMap<String, u128>,
}

#[derive(Default, Serialize)]
struct CellRegionSyncPayload {
    source_cells_indexed: usize,
    target_cells_seen: usize,
    cells_changed: usize,
    cells_retagged: usize,
    cells_already_tagged: usize,
    region_refs_written: usize,
    missing_target_regions: usize,
    unmatched_target_cells: usize,
    warnings: Vec<String>,
    timing: BTreeMap<String, u128>,
}

struct TargetFormIdContext {
    plugin_name: String,
    masters: Vec<String>,
    own_prefix: u32,
}

#[derive(Default, Serialize)]
struct WaterManifestPayload {
    default_water_object_id: u32,
    cells: Vec<WaterManifestCell>,
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct WaterManifestCell {
    x: i32,
    y: i32,
    height: f32,
}

const VALID_WATER_HEIGHT_LIMIT: f32 = 1.0e8;

const PLACEMENT_BASE_SIGNATURES: &[&str] = &[
    "ACTI", "ASPC", "CONT", "DOOR", "EXPL", "FLOR", "FURN", "IPCT", "IPDS", "LIGH", "LVLI", "MISC",
    "MOVT", "MSTT", "NPC_", "PROJ", "SCOL", "SOUN", "STAT", "TREE",
];

const INVALID_PLACED_BASE_SIGNATURES: &[&str] = &["LVLI"];

fn render_form_key(plugin: &ParsedPlugin, raw_form_id: u32) -> String {
    let own_name: Arc<str> = Arc::from(plugin.plugin_name.as_str());
    resolve_form_id_to_form_key(raw_form_id, &own_name, &plugin.header.masters).render()
}

fn subrecord_data(record: &ParsedRecord, signature: &str) -> Option<Bytes> {
    effective_subrecords_for_record(record)
        .iter()
        .find(|sub| sub.signature.as_str() == signature)
        .map(|sub| sub.data.clone())
}

fn editor_id(record: &ParsedRecord) -> String {
    editor_id_from_effective_subrecords(&effective_subrecords_for_record(record))
}

fn cell_grid(record: &ParsedRecord) -> Option<(i32, i32)> {
    let xclc = subrecord_data(record, "XCLC")?;
    if xclc.len() < 8 {
        return None;
    }
    let x = i32::from_le_bytes([xclc[0], xclc[1], xclc[2], xclc[3]]);
    let y = i32::from_le_bytes([xclc[4], xclc[5], xclc[6], xclc[7]]);
    Some((x, y))
}

fn inside_bounds(x: i32, y: i32, min_x: i32, min_y: i32, max_x: i32, max_y: i32) -> bool {
    min_x <= x && x <= max_x && min_y <= y && y <= max_y
}

fn decode_group_form_id(group: &ParsedGroup) -> Option<u32> {
    Some(u32::from_le_bytes(group.label))
}

fn collect_group_records<'a>(
    group: &'a ParsedGroup,
    signature: &str,
    records: &mut Vec<&'a ParsedRecord>,
) {
    for child in &group.children {
        match child {
            ParsedItem::Record(record) if record.signature.as_str() == signature => {
                records.push(record);
            }
            ParsedItem::Group(child_group) => {
                collect_group_records(child_group, signature, records);
            }
            _ => {}
        }
    }
}

fn top_group<'a>(plugin: &'a ParsedPlugin, signature: &str) -> Option<&'a ParsedGroup> {
    let wanted = signature.as_bytes();
    if wanted.len() != 4 {
        return None;
    }
    plugin.root_items.iter().find_map(|item| {
        let ParsedItem::Group(group) = item else {
            return None;
        };
        if group.group_type == 0 && group.label == [wanted[0], wanted[1], wanted[2], wanted[3]] {
            Some(group)
        } else {
            None
        }
    })
}

fn find_world<'a>(
    plugin: &'a ParsedPlugin,
    worldspace_editor_id: &str,
) -> (Option<&'a ParsedRecord>, Vec<String>) {
    let mut warnings = Vec::new();
    let Some(wrld_group) = top_group(plugin, "WRLD") else {
        warnings.push("WRLD top group not found".to_string());
        return (None, warnings);
    };
    for item in &wrld_group.children {
        let ParsedItem::Record(record) = item else {
            continue;
        };
        if record.signature.as_str() != "WRLD" {
            continue;
        }
        if editor_id(record).eq_ignore_ascii_case(worldspace_editor_id) {
            return (Some(record), warnings);
        }
    }
    warnings.push(format!("worldspace not found: {worldspace_editor_id}"));
    (None, warnings)
}

fn find_world_children_group<'a>(
    wrld_group: &'a ParsedGroup,
    world_form_id: u32,
) -> Option<&'a ParsedGroup> {
    wrld_group.children.iter().find_map(|item| {
        let ParsedItem::Group(group) = item else {
            return None;
        };
        if group.group_type == 1 && decode_group_form_id(group) == Some(world_form_id) {
            Some(group)
        } else {
            None
        }
    })
}

fn top_group_mut<'a>(plugin: &'a mut ParsedPlugin, signature: &str) -> Option<&'a mut ParsedGroup> {
    let wanted = signature.as_bytes();
    if wanted.len() != 4 {
        return None;
    }
    plugin.root_items.iter_mut().find_map(|item| {
        let ParsedItem::Group(group) = item else {
            return None;
        };
        if group.group_type == 0 && group.label == [wanted[0], wanted[1], wanted[2], wanted[3]] {
            Some(group)
        } else {
            None
        }
    })
}

fn find_world_children_group_mut<'a>(
    wrld_group: &'a mut ParsedGroup,
    world_form_id: u32,
) -> Option<&'a mut ParsedGroup> {
    wrld_group.children.iter_mut().find_map(|item| {
        let ParsedItem::Group(group) = item else {
            return None;
        };
        if group.group_type == 1 && decode_group_form_id(group) == Some(world_form_id) {
            Some(group)
        } else {
            None
        }
    })
}

fn direct_world_persistent_cell_id(world_children: &ParsedGroup) -> Option<u32> {
    world_children.children.iter().find_map(|item| {
        let ParsedItem::Record(record) = item else {
            return None;
        };
        (record.signature.as_str() == "CELL").then_some(record.form_id)
    })
}

fn collect_cell_child_groups<'a>(group: &'a ParsedGroup, out: &mut BTreeMap<u32, &'a ParsedGroup>) {
    if group.group_type == CELL_CHILD_GROUP {
        if let Some(cell_form_id) = decode_group_form_id(group) {
            out.entry(cell_form_id).or_insert(group);
        }
    }
    for item in &group.children {
        if let ParsedItem::Group(child_group) = item {
            collect_cell_child_groups(child_group, out);
        }
    }
}

fn collect_cell_child_keys(
    plugin: &ParsedPlugin,
    child_group: Option<&ParsedGroup>,
    locator: &LocatorSection,
    static_base_keys: &mut Vec<String>,
    static_base_seen: &mut BTreeSet<String>,
    leveled_base_entry_keys: &mut Vec<String>,
    leveled_base_entry_seen: &mut BTreeSet<String>,
    linked_ref_keyword_keys: &mut Vec<String>,
    linked_ref_keyword_seen: &mut BTreeSet<String>,
    layer_keys: &mut Vec<String>,
    layer_seen: &mut BTreeSet<String>,
) -> CellChildrenPayload {
    let mut children = CellChildrenPayload::default();
    let Some(child_group) = child_group else {
        return children;
    };
    fn collect_section_records(
        plugin: &ParsedPlugin,
        locator: &LocatorSection,
        items: &[ParsedItem],
        target: &mut Vec<String>,
        static_base_keys: &mut Vec<String>,
        static_base_seen: &mut BTreeSet<String>,
        leveled_base_entry_keys: &mut Vec<String>,
        leveled_base_entry_seen: &mut BTreeSet<String>,
        linked_ref_keyword_keys: &mut Vec<String>,
        linked_ref_keyword_seen: &mut BTreeSet<String>,
        layer_keys: &mut Vec<String>,
        layer_seen: &mut BTreeSet<String>,
    ) {
        for item in items {
            match item {
                ParsedItem::Record(record)
                    if is_placed_child_signature(record.signature.as_str()) =>
                {
                    append_layer_key_from_subrecord(
                        plugin, locator, record, "XLYR", layer_keys, layer_seen,
                    );
                    append_linked_ref_keyword_key(
                        plugin,
                        locator,
                        record,
                        linked_ref_keyword_keys,
                        linked_ref_keyword_seen,
                    );
                    append_static_placement_base_key(
                        plugin,
                        locator,
                        record,
                        static_base_keys,
                        static_base_seen,
                        leveled_base_entry_keys,
                        leveled_base_entry_seen,
                        layer_keys,
                        layer_seen,
                    );
                    target.push(render_form_key(plugin, record.form_id));
                }
                ParsedItem::Group(group) => collect_section_records(
                    plugin,
                    locator,
                    &group.children,
                    target,
                    static_base_keys,
                    static_base_seen,
                    leveled_base_entry_keys,
                    leveled_base_entry_seen,
                    linked_ref_keyword_keys,
                    linked_ref_keyword_seen,
                    layer_keys,
                    layer_seen,
                ),
                _ => {}
            }
        }
    }
    for item in &child_group.children {
        let ParsedItem::Group(section_group) = item else {
            continue;
        };
        let target = match section_group.group_type {
            PERSISTENT_GROUP => &mut children.persistent,
            TEMPORARY_GROUP => &mut children.temporary,
            _ => continue,
        };
        collect_section_records(
            plugin,
            locator,
            &section_group.children,
            target,
            static_base_keys,
            static_base_seen,
            leveled_base_entry_keys,
            leveled_base_entry_seen,
            linked_ref_keyword_keys,
            linked_ref_keyword_seen,
            layer_keys,
            layer_seen,
        );
    }
    children
}

fn append_static_placement_base_key(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    record: &ParsedRecord,
    static_base_keys: &mut Vec<String>,
    static_base_seen: &mut BTreeSet<String>,
    leveled_base_entry_keys: &mut Vec<String>,
    leveled_base_entry_seen: &mut BTreeSet<String>,
    layer_keys: &mut Vec<String>,
    layer_seen: &mut BTreeSet<String>,
) {
    if !matches!(
        record.signature.as_str(),
        "REFR" | "ACHR" | "PHZD" | "PGRE" | "PGRD"
    ) {
        return;
    }
    let Some(name) = subrecord_data(record, "NAME") else {
        return;
    };
    if name.len() != 4 {
        return;
    }
    let raw_form_id = u32::from_le_bytes([name[0], name[1], name[2], name[3]]);
    if raw_form_id == 0 {
        return;
    }
    let base_key = render_form_key(plugin, raw_form_id);
    let Some(base_entry) = locator_entry_by_form_key(locator, base_key.as_str()) else {
        return;
    };
    if !PLACEMENT_BASE_SIGNATURES.contains(&base_entry.signature.as_str()) {
        return;
    }
    if let Some(base_record) = locator.record(plugin, base_entry) {
        append_layer_key_from_subrecord(
            plugin,
            locator,
            base_record,
            "DEFL",
            layer_keys,
            layer_seen,
        );
        if base_entry.signature.as_str() == "LVLI" {
            append_leveled_item_entry_keys(
                plugin,
                locator,
                base_record,
                leveled_base_entry_keys,
                leveled_base_entry_seen,
                &mut BTreeSet::new(),
                0,
            );
        }
    }
    if static_base_seen.insert(base_key.clone()) {
        static_base_keys.push(base_key);
    }
}

fn append_linked_ref_keyword_key(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    record: &ParsedRecord,
    linked_ref_keyword_keys: &mut Vec<String>,
    linked_ref_keyword_seen: &mut BTreeSet<String>,
) {
    for subrecord in effective_subrecords_for_record(record)
        .iter()
        .filter(|subrecord| subrecord.signature.as_str() == "XLKR")
    {
        let data = &subrecord.data;
        if data.len() < 8 {
            continue;
        }
        let raw_form_id = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if raw_form_id == 0 {
            continue;
        }
        let keyword_key = render_form_key(plugin, raw_form_id);
        let Some(keyword_entry) = locator_entry_by_form_key(locator, keyword_key.as_str()) else {
            continue;
        };
        if keyword_entry.signature.as_str() != "KYWD" {
            continue;
        }
        if linked_ref_keyword_seen.insert(keyword_key.clone()) {
            linked_ref_keyword_keys.push(keyword_key);
        }
    }
}

fn lvlo_entry_form_ids(data: &[u8]) -> Vec<u32> {
    if data.len() == 4 {
        return vec![u32::from_le_bytes([data[0], data[1], data[2], data[3]])];
    }
    if data.len() >= 12 && data.len() % 12 == 0 {
        return data
            .chunks_exact(12)
            .map(|chunk| u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]))
            .collect();
    }
    Vec::new()
}

fn leveled_item_entry_form_ids(record: &ParsedRecord) -> Vec<u32> {
    effective_subrecords_for_record(record)
        .iter()
        .filter(|subrecord| subrecord.signature.as_str() == "LVLO")
        .flat_map(|subrecord| lvlo_entry_form_ids(subrecord.data.as_ref()))
        .filter(|raw_form_id| *raw_form_id != 0)
        .collect()
}

fn append_leveled_item_entry_keys(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    record: &ParsedRecord,
    target: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    depth: usize,
) {
    if depth >= 8 {
        return;
    }
    let record_key = render_form_key(plugin, record.form_id);
    if !visited.insert(record_key) {
        return;
    }
    for raw_form_id in leveled_item_entry_form_ids(record) {
        let entry_key = render_form_key(plugin, raw_form_id);
        if seen.insert(entry_key.clone()) {
            target.push(entry_key.clone());
        }
        let Some(entry) = locator_entry_by_form_key(locator, entry_key.as_str()) else {
            continue;
        };
        if entry.signature.as_str() != "LVLI" {
            continue;
        }
        if let Some(entry_record) = locator.record(plugin, entry) {
            append_leveled_item_entry_keys(
                plugin,
                locator,
                entry_record,
                target,
                seen,
                visited,
                depth + 1,
            );
        }
    }
}

fn append_layer_key_from_subrecord(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    record: &ParsedRecord,
    signature: &str,
    layer_keys: &mut Vec<String>,
    layer_seen: &mut BTreeSet<String>,
) {
    let Some(data) = subrecord_data(record, signature) else {
        return;
    };
    if data.len() != 4 {
        return;
    }
    append_layer_key(
        plugin,
        locator,
        u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
        layer_keys,
        layer_seen,
    );
}

fn append_layer_key(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    raw_form_id: u32,
    layer_keys: &mut Vec<String>,
    layer_seen: &mut BTreeSet<String>,
) {
    if raw_form_id == 0 {
        return;
    }
    let layer_key = render_form_key(plugin, raw_form_id);
    let Some(layer_entry) = locator_entry_by_form_key(locator, layer_key.as_str()) else {
        return;
    };
    if layer_entry.signature.as_str() != "LAYR" {
        return;
    }
    if layer_seen.insert(layer_key.clone()) {
        layer_keys.push(layer_key);
    }
}

fn append_form_keys_from_array_subrecord(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    record: &ParsedRecord,
    subrecord_signature: &str,
    target_record_signature: &str,
    target: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
) {
    let Some(data) = subrecord_data(record, subrecord_signature) else {
        return;
    };
    if data.is_empty() || data.len() % 4 != 0 {
        return;
    }
    for chunk in data.chunks_exact(4) {
        let raw_form_id = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if raw_form_id == 0 {
            continue;
        }
        let form_key = render_form_key(plugin, raw_form_id);
        let Some(entry) = locator_entry_by_form_key(locator, form_key.as_str()) else {
            continue;
        };
        if entry.signature.as_str() != target_record_signature {
            continue;
        }
        if seen.insert(form_key.clone()) {
            target.push(form_key);
        }
    }
}

fn append_verified_form_key(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    raw_form_id: u32,
    target_record_signatures: &[&str],
    target: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
) {
    let Some(form_key) = verified_form_key(plugin, locator, raw_form_id, target_record_signatures)
    else {
        return;
    };
    if seen.insert(form_key.clone()) {
        target.push(form_key);
    }
}

fn verified_form_key(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    raw_form_id: u32,
    target_record_signatures: &[&str],
) -> Option<String> {
    if raw_form_id == 0 || target_record_signatures.is_empty() {
        return None;
    }
    let form_key = render_form_key(plugin, raw_form_id);
    let Some(entry) = locator_entry_by_form_key(locator, form_key.as_str()) else {
        return None;
    };
    if !target_record_signatures.contains(&entry.signature.as_str()) {
        return None;
    }
    Some(form_key)
}

fn append_form_keys_from_subrecords(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    record: &ParsedRecord,
    subrecord_signature: &str,
    target_record_signatures: &[&str],
    target: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
) {
    for subrecord in effective_subrecords_for_record(record).iter() {
        let data = subrecord.data.as_ref();
        if subrecord.signature.as_str() != subrecord_signature || data.len() < 4 {
            continue;
        }
        append_verified_form_key(
            plugin,
            locator,
            u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            target_record_signatures,
            target,
            seen,
        );
    }
}

fn append_form_keys_from_array_subrecords(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    record: &ParsedRecord,
    subrecord_signature: &str,
    target_record_signatures: &[&str],
    target: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
) {
    for subrecord in effective_subrecords_for_record(record).iter() {
        let data = subrecord.data.as_ref();
        if subrecord.signature.as_str() != subrecord_signature || data.is_empty() {
            continue;
        }
        for chunk in data.chunks_exact(4) {
            append_verified_form_key(
                plugin,
                locator,
                u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
                target_record_signatures,
                target,
                seen,
            );
        }
    }
}

fn append_region_data_dependency_keys(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    region_form_keys: &[String],
    target: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
) {
    for region_key in region_form_keys {
        let Some(region_entry) = locator_entry_by_form_key(locator, region_key.as_str()) else {
            continue;
        };
        if region_entry.signature.as_str() != "REGN" {
            continue;
        }
        let Some(region) = locator.record(plugin, region_entry) else {
            continue;
        };
        for subrecord in effective_subrecords_for_record(region).iter() {
            let data = subrecord.data.as_ref();
            match subrecord.signature.as_str() {
                "RDMO" if data.len() >= 4 => {
                    append_verified_form_key(
                        plugin,
                        locator,
                        u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
                        &["MUSC"],
                        target,
                        seen,
                    );
                }
                "RDWT" => {
                    for row in data.chunks_exact(12) {
                        append_verified_form_key(
                            plugin,
                            locator,
                            u32::from_le_bytes([row[0], row[1], row[2], row[3]]),
                            &["WTHR"],
                            target,
                            seen,
                        );
                        append_verified_form_key(
                            plugin,
                            locator,
                            u32::from_le_bytes([row[8], row[9], row[10], row[11]]),
                            &["GLOB"],
                            target,
                            seen,
                        );
                    }
                }
                "RDSN" if data.len() >= 4 => {
                    append_verified_form_key(
                        plugin,
                        locator,
                        u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
                        &["SNDR", "SOUN"],
                        target,
                        seen,
                    );
                }
                "RDSA" => {
                    for row in data.chunks_exact(12) {
                        append_verified_form_key(
                            plugin,
                            locator,
                            u32::from_le_bytes([row[0], row[1], row[2], row[3]]),
                            &["SNDR", "SOUN"],
                            target,
                            seen,
                        );
                    }
                }
                "RDGS" => {
                    for row in data.chunks_exact(8) {
                        append_verified_form_key(
                            plugin,
                            locator,
                            u32::from_le_bytes([row[0], row[1], row[2], row[3]]),
                            &["GRAS"],
                            target,
                            seen,
                        );
                    }
                }
                _ => {}
            }
        }
    }
}

fn append_worldspace_data_dependency_keys(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    world_record: &ParsedRecord,
    target: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
) {
    for subrecord in effective_subrecords_for_record(world_record).iter() {
        let data = subrecord.data.as_ref();
        if data.len() < 4 {
            continue;
        }
        let (offset, expected_signature) = match subrecord.signature.as_str() {
            "CNAM" => (0, "CLMT"),
            "NAM2" | "NAM3" => (0, "WATR"),
            _ => continue,
        };
        append_verified_form_key(
            plugin,
            locator,
            u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]),
            &[expected_signature],
            target,
            seen,
        );
    }
}

fn append_lctn_world_cell_location_keys(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    world_form_id: u32,
    cell_grids: &BTreeMap<String, CellGridPayload>,
    target: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
    warnings: &mut Vec<String>,
) {
    let mut locations = BTreeMap::new();
    let mut conflicts = BTreeSet::new();
    collect_lctn_world_cell_locations(&plugin.root_items, &mut locations, &mut conflicts);
    let conflict_count = conflicts.len();
    for key in conflicts {
        locations.remove(&key);
    }
    if conflict_count > 0 {
        warnings.push(format!(
            "LCTN world-cell location conflicts skipped: {conflict_count}"
        ));
    }

    for grid in cell_grids.values() {
        if grid.x < i16::MIN as i32
            || grid.x > i16::MAX as i32
            || grid.y < i16::MIN as i32
            || grid.y > i16::MAX as i32
        {
            continue;
        }
        let Some(location_id) = locations
            .get(&(world_form_id, grid.x as i16, grid.y as i16))
            .copied()
        else {
            continue;
        };
        append_verified_form_key(plugin, locator, location_id, &["LCTN"], target, seen);
    }
}

fn append_location_data_dependency_keys(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    location_form_keys: &[String],
    target: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
) {
    let mut queue: VecDeque<String> = location_form_keys.iter().cloned().collect();
    let mut visited = BTreeSet::new();
    for key in location_form_keys {
        seen.insert(key.clone());
    }

    while let Some(form_key) = queue.pop_front() {
        if !visited.insert(form_key.clone()) {
            continue;
        }
        let Some(entry) = locator_entry_by_form_key(locator, form_key.as_str()) else {
            continue;
        };
        if entry.signature.as_str() != "LCTN" {
            continue;
        }
        let Some(record) = locator.record(plugin, entry) else {
            continue;
        };

        append_form_keys_from_array_subrecords(
            plugin,
            locator,
            record,
            "KWDA",
            &["KYWD"],
            target,
            seen,
        );
        append_form_keys_from_subrecords(plugin, locator, record, "NAM1", &["MUSC"], target, seen);
        append_form_keys_from_subrecords(plugin, locator, record, "FNAM", &["FACT"], target, seen);

        for subrecord in effective_subrecords_for_record(record).iter() {
            let data = subrecord.data.as_ref();
            if subrecord.signature.as_str() != "PNAM" || data.len() < 4 {
                continue;
            }
            let raw = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            let Some(parent_key) = verified_form_key(plugin, locator, raw, &["LCTN"]) else {
                continue;
            };
            if seen.insert(parent_key.clone()) {
                target.push(parent_key.clone());
            }
            queue.push_back(parent_key);
        }
    }
}

fn append_audio_data_dependency_keys(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    root_form_keys: &[String],
    target: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
) {
    let mut queue: VecDeque<String> = root_form_keys.iter().cloned().collect();
    let mut visited = BTreeSet::new();
    for key in root_form_keys {
        seen.insert(key.clone());
    }

    while let Some(form_key) = queue.pop_front() {
        if !visited.insert(form_key.clone()) {
            continue;
        }
        let Some(entry) = locator_entry_by_form_key(locator, form_key.as_str()) else {
            continue;
        };
        let Some(record) = locator.record(plugin, entry) else {
            continue;
        };
        append_record_audio_dependency_keys(plugin, locator, record, target, seen, &mut queue);
    }
}

fn queue_audio_form_key(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    raw_form_id: u32,
    target_record_signatures: &[&str],
    target: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
    queue: &mut VecDeque<String>,
) {
    let Some(form_key) = verified_form_key(plugin, locator, raw_form_id, target_record_signatures)
    else {
        return;
    };
    if seen.insert(form_key.clone()) {
        target.push(form_key.clone());
    }
    queue.push_back(form_key);
}

fn queue_audio_subrecord_form_keys(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    record: &ParsedRecord,
    subrecord_signature: &str,
    target_record_signatures: &[&str],
    target: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
    queue: &mut VecDeque<String>,
) {
    for subrecord in effective_subrecords_for_record(record).iter() {
        let data = subrecord.data.as_ref();
        if subrecord.signature.as_str() != subrecord_signature || data.len() < 4 {
            continue;
        }
        queue_audio_form_key(
            plugin,
            locator,
            u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            target_record_signatures,
            target,
            seen,
            queue,
        );
    }
}

fn queue_audio_array_subrecord_form_keys(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    record: &ParsedRecord,
    subrecord_signature: &str,
    target_record_signatures: &[&str],
    target: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
    queue: &mut VecDeque<String>,
) {
    for subrecord in effective_subrecords_for_record(record).iter() {
        let data = subrecord.data.as_ref();
        if subrecord.signature.as_str() != subrecord_signature || data.is_empty() {
            continue;
        }
        for chunk in data.chunks_exact(4) {
            queue_audio_form_key(
                plugin,
                locator,
                u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
                target_record_signatures,
                target,
                seen,
                queue,
            );
        }
    }
}

fn append_record_audio_dependency_keys(
    plugin: &ParsedPlugin,
    locator: &LocatorSection,
    record: &ParsedRecord,
    target: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
    queue: &mut VecDeque<String>,
) {
    match record.signature.as_str() {
        "ASPC" => {
            queue_audio_subrecord_form_keys(
                plugin,
                locator,
                record,
                "DEFL",
                &["LAYR"],
                target,
                seen,
                queue,
            );
            queue_audio_subrecord_form_keys(
                plugin,
                locator,
                record,
                "SNAM",
                &["SNDR"],
                target,
                seen,
                queue,
            );
            queue_audio_subrecord_form_keys(
                plugin,
                locator,
                record,
                "RDAT",
                &["REGN"],
                target,
                seen,
                queue,
            );
            queue_audio_subrecord_form_keys(
                plugin,
                locator,
                record,
                "BNAM",
                &["REVB"],
                target,
                seen,
                queue,
            );
        }
        "SOUN" => queue_audio_subrecord_form_keys(
            plugin,
            locator,
            record,
            "SDSC",
            &["SNDR"],
            target,
            seen,
            queue,
        ),
        "SNDR" => {
            queue_audio_subrecord_form_keys(
                plugin,
                locator,
                record,
                "GNAM",
                &["SNCT"],
                target,
                seen,
                queue,
            );
            queue_audio_subrecord_form_keys(
                plugin,
                locator,
                record,
                "SNAM",
                &["SNDR"],
                target,
                seen,
                queue,
            );
            queue_audio_subrecord_form_keys(
                plugin,
                locator,
                record,
                "ONAM",
                &["SOPM"],
                target,
                seen,
                queue,
            );
            queue_audio_subrecord_form_keys(
                plugin,
                locator,
                record,
                "BNAM",
                &["SNDR"],
                target,
                seen,
                queue,
            );
            queue_audio_subrecord_form_keys(
                plugin,
                locator,
                record,
                "DNAM",
                &["SNDR"],
                target,
                seen,
                queue,
            );
        }
        "SNCT" => {
            queue_audio_subrecord_form_keys(
                plugin,
                locator,
                record,
                "PNAM",
                &["SNCT"],
                target,
                seen,
                queue,
            );
            queue_audio_subrecord_form_keys(
                plugin,
                locator,
                record,
                "ONAM",
                &["SNCT"],
                target,
                seen,
                queue,
            );
        }
        "MUSC" => queue_audio_array_subrecord_form_keys(
            plugin,
            locator,
            record,
            "TNAM",
            &["MUST"],
            target,
            seen,
            queue,
        ),
        "LCTN" => queue_audio_subrecord_form_keys(
            plugin,
            locator,
            record,
            "NAM1",
            &["MUSC"],
            target,
            seen,
            queue,
        ),
        "REGN" => {
            let mut deps = Vec::new();
            append_region_data_dependency_keys(
                plugin,
                locator,
                &[render_form_key(plugin, record.form_id)],
                &mut deps,
                seen,
            );
            for key in deps {
                target.push(key.clone());
                queue.push_back(key);
            }
        }
        _ => {}
    }
}

fn append_unique(target: &mut Vec<String>, values: &[String], seen: &mut BTreeSet<String>) {
    for value in values {
        if seen.insert(value.clone()) {
            target.push(value.clone());
        }
    }
}

fn group_label_for_form_id(raw_form_id: u32) -> [u8; 4] {
    raw_form_id.to_le_bytes()
}

fn ensure_child_group_mut<'a>(
    parent: &'a mut ParsedGroup,
    group_type: i32,
    raw_form_id: u32,
    header_size: usize,
) -> &'a mut ParsedGroup {
    let label = group_label_for_form_id(raw_form_id);
    if let Some(index) = parent.children.iter().position(|item| {
        matches!(item, ParsedItem::Group(group) if group.group_type == group_type && group.label == label)
    }) {
        let ParsedItem::Group(group) = &mut parent.children[index] else {
            unreachable!();
        };
        return group;
    }
    let tail_len = header_size.saturating_sub(16);
    parent.children.push(ParsedItem::Group(ParsedGroup {
        label,
        group_type,
        tail: Bytes::from(vec![0u8; tail_len]),
        children: Vec::new(),
    }));
    let ParsedItem::Group(group) = parent.children.last_mut().expect("just pushed group") else {
        unreachable!();
    };
    group
}

fn object_id_from_form_key(form_key: &str) -> Option<u32> {
    split_form_key_text(form_key).map(|(_plugin_name, object_id)| object_id)
}

fn split_form_key_text(form_key: &str) -> Option<(String, u32)> {
    let value = form_key.trim();
    if let Some((plugin_name, object_id_text)) = value.rsplit_once(':') {
        if let Ok(object_id) = u32::from_str_radix(object_id_text.trim(), 16) {
            return Some((plugin_name.trim().to_string(), object_id & 0x00FF_FFFF));
        }
    }
    if let Some((object_id_text, plugin_name)) = value.split_once('@') {
        if let Ok(object_id) = u32::from_str_radix(object_id_text.trim(), 16) {
            return Some((plugin_name.trim().to_string(), object_id & 0x00FF_FFFF));
        }
    }
    None
}

fn normalized_form_key_text(form_key: &str) -> Option<String> {
    let (plugin_name, object_id) = split_form_key_text(form_key)?;
    Some(format!(
        "{}:{object_id:06X}",
        plugin_name.to_ascii_lowercase()
    ))
}

fn target_raw_form_id_for_key(target: &TargetFormIdContext, form_key: &str) -> Option<u32> {
    let (plugin_name, object_id) = split_form_key_text(form_key)?;
    if plugin_name.eq_ignore_ascii_case(target.plugin_name.as_str()) {
        return Some(target.own_prefix | object_id);
    }
    for (index, master_name) in target.masters.iter().enumerate() {
        if plugin_name.eq_ignore_ascii_case(master_name.as_str()) {
            return Some(((index as u32) << 24) | object_id);
        }
    }
    None
}

fn local_form_prefix(plugin: &ParsedPlugin) -> u32 {
    ((plugin.header.masters.len() as u32) & 0xFF) << 24
}

fn is_source_local_form_id(raw_form_id: u32, source_own_index: u8) -> bool {
    if raw_form_id == 0 {
        return false;
    }
    let index = ((raw_form_id >> 24) & 0xFF) as u8;
    index == source_own_index || index == LOCAL_FORM_INDEX
}

fn collect_record_form_ids(items: &[ParsedItem], target: &mut BTreeSet<u32>) {
    for item in items {
        match item {
            ParsedItem::Record(record) => {
                target.insert(record.form_id);
            }
            ParsedItem::Group(group) => collect_record_form_ids(&group.children, target),
        }
    }
}

fn next_available_local_form_id(used_form_ids: &mut BTreeSet<u32>, own_prefix: u32) -> Option<u32> {
    let mut next_object_id = used_form_ids
        .iter()
        .filter(|raw| (**raw & 0xFF00_0000) == own_prefix)
        .map(|raw| raw & 0x00FF_FFFF)
        .max()
        .unwrap_or(0x0007FF)
        .saturating_add(1)
        .max(0x000800);

    while next_object_id <= 0x00FF_FFFF {
        let raw_form_id = own_prefix | next_object_id;
        if used_form_ids.insert(raw_form_id) {
            return Some(raw_form_id);
        }
        next_object_id = next_object_id.saturating_add(1);
    }
    None
}

fn mapped_target_local_raw(
    source_norm: &str,
    target: &TargetFormIdContext,
    form_key_map: &BTreeMap<String, String>,
) -> Option<u32> {
    form_key_map
        .get(source_norm)
        .and_then(|target_key| target_raw_form_id_for_key(target, target_key.as_str()))
        .filter(|raw| (*raw & 0xFF00_0000) == target.own_prefix)
}

fn reserve_copied_child_form_ids(
    children_by_target_cell: &BTreeMap<String, CellChildrenPayload>,
    source_plugin: &ParsedPlugin,
    target: &TargetFormIdContext,
    used_form_ids: &mut BTreeSet<u32>,
    form_key_map: &mut BTreeMap<String, String>,
) -> usize {
    let mut reallocated = 0usize;
    for sections in children_by_target_cell.values() {
        for target_child_key in sections.persistent.iter().chain(sections.temporary.iter()) {
            let Some(object_id) = object_id_from_form_key(target_child_key.as_str()) else {
                continue;
            };
            let source_key = FormKey::new(Arc::from(source_plugin.plugin_name.as_str()), object_id);
            let Some(source_norm) = normalized_form_key_text(source_key.render().as_str()) else {
                continue;
            };
            let mut needs_allocation = false;
            if let Some(mapped_raw) =
                mapped_target_local_raw(source_norm.as_str(), target, form_key_map)
            {
                if used_form_ids.insert(mapped_raw) {
                    continue;
                }
                needs_allocation = true;
            }

            let preserved_raw = target.own_prefix | object_id;
            if !needs_allocation && used_form_ids.insert(preserved_raw) {
                continue;
            }

            let Some(allocated_raw) =
                next_available_local_form_id(used_form_ids, target.own_prefix)
            else {
                continue;
            };
            form_key_map.insert(
                source_norm,
                format!("{}:{:06X}", target.plugin_name, allocated_raw & 0x00FF_FFFF),
            );
            reallocated += 1;
        }
    }
    reallocated
}

fn filter_record_to_target_schema(record: &mut ParsedRecord, target_game: Option<&str>) -> usize {
    let Some(game) = target_game else {
        return 0;
    };
    if game.trim().is_empty() {
        return 0;
    }
    let Ok(schema) = schema::compiled_schema_for_game(game) else {
        return 0;
    };
    let Some(record_spec) = schema::schema_record_spec(&schema, record.signature.as_str()) else {
        return 0;
    };
    let allowed: BTreeSet<&str> = record_spec
        .subrecords
        .iter()
        .map(|subrecord| subrecord.id.as_str())
        .collect();
    if allowed.is_empty() {
        return 0;
    }
    if record.subrecords.is_empty() {
        record.subrecords = effective_subrecords_for_record(record).into_owned();
    }
    let before = record.subrecords.len();
    record.subrecords.retain(|subrecord| {
        allowed.contains(subrecord.signature.as_str())
            && target_schema_accepts_payload(record_spec, subrecord)
    });
    let removed = before.saturating_sub(record.subrecords.len());
    if removed > 0 {
        record.raw_payload = None;
    }
    removed
}

fn target_schema_accepts_payload(
    record_spec: &SchemaRecordJson,
    subrecord: &ParsedSubrecord,
) -> bool {
    let candidates: Vec<&SchemaSubrecordJson> = record_spec
        .subrecords
        .iter()
        .filter(|spec| spec.id == subrecord.signature.as_str())
        .collect();
    if candidates.is_empty() {
        return false;
    }
    candidates
        .iter()
        .any(|spec| subrecord_payload_matches_schema(spec, subrecord.data.len()))
}

fn subrecord_payload_matches_schema(spec: &SchemaSubrecordJson, payload_len: usize) -> bool {
    let Some(codec) = spec.codec.as_deref() else {
        return true;
    };
    if codec == "empty" {
        return payload_len == 0;
    }
    match authoring::authoring_serialize::codec_accepts_payload_length(codec, payload_len) {
        Some(false) => false,
        Some(true) | None => true,
    }
}

fn parse_form_key_map_json(form_key_map_json: Option<&str>) -> PyResult<BTreeMap<String, String>> {
    let Some(text) = form_key_map_json
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return Ok(BTreeMap::new());
    };
    let raw: BTreeMap<String, String> = serde_json::from_str(text).map_err(|err| {
        PyValueError::new_err(format!("invalid cell children form-key map: {err}"))
    })?;
    let mut normalized = BTreeMap::new();
    for (source_key, target_key) in raw {
        let Some(source_norm) = normalized_form_key_text(source_key.as_str()) else {
            continue;
        };
        normalized.insert(source_norm, target_key);
    }
    Ok(normalized)
}

fn target_local_raw_matching_signature(
    raw_form_id: u32,
    target: &TargetFormIdContext,
    target_locator: &LocatorSection,
    expected_signatures: &[&str],
) -> Option<u32> {
    if expected_signatures.is_empty() {
        return None;
    }
    let target_raw = target.own_prefix | (raw_form_id & 0x00FF_FFFF);
    let signature = target_local_record_signature(target, target_locator, target_raw)?;
    expected_signatures
        .iter()
        .any(|expected| signature == *expected)
        .then_some(target_raw)
}

fn rewrite_source_form_id_with_map_or_target_local(
    data: &mut [u8],
    offset: usize,
    source_plugin: &ParsedPlugin,
    source_own_index: u8,
    target: &TargetFormIdContext,
    target_locator: &LocatorSection,
    form_key_map: &BTreeMap<String, String>,
    target_local_fallback_signatures: &[&str],
) -> bool {
    if data.len() < offset + 4 {
        return false;
    }
    let raw_form_id = u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]);
    if raw_form_id == 0 {
        return false;
    }
    let source_key = render_form_key(source_plugin, raw_form_id);
    let mapped_raw = normalized_form_key_text(source_key.as_str())
        .and_then(|source_norm| form_key_map.get(source_norm.as_str()))
        .and_then(|target_key| target_raw_form_id_for_key(target, target_key.as_str()));
    let rewritten = mapped_raw
        .or_else(|| {
            is_source_local_form_id(raw_form_id, source_own_index)
                .then_some(target.own_prefix | (raw_form_id & 0x00FF_FFFF))
        })
        .or_else(|| {
            target_local_raw_matching_signature(
                raw_form_id,
                target,
                target_locator,
                target_local_fallback_signatures,
            )
        });
    let Some(rewritten) = rewritten else {
        return false;
    };
    if rewritten == raw_form_id {
        return false;
    }
    data[offset..offset + 4].copy_from_slice(&rewritten.to_le_bytes());
    true
}

fn source_raw_form_id_to_target_raw(
    raw_form_id: u32,
    source_plugin: &ParsedPlugin,
    source_own_index: u8,
    target: &TargetFormIdContext,
    form_key_map: &BTreeMap<String, String>,
) -> Option<u32> {
    let source_key = render_form_key(source_plugin, raw_form_id);
    if let Some(mapped_raw) = normalized_form_key_text(source_key.as_str())
        .and_then(|source_norm| form_key_map.get(source_norm.as_str()))
        .and_then(|target_key| target_raw_form_id_for_key(target, target_key.as_str()))
    {
        return Some(mapped_raw);
    }
    if is_source_local_form_id(raw_form_id, source_own_index) {
        return Some(target.own_prefix | (raw_form_id & 0x00FF_FFFF));
    }
    None
}

fn target_local_record_signature<'a>(
    target: &TargetFormIdContext,
    target_locator: &'a LocatorSection,
    raw_form_id: u32,
) -> Option<&'a str> {
    if (raw_form_id & 0xFF00_0000) != target.own_prefix {
        return None;
    }
    let key = FormKey::new(
        Arc::from(target.plugin_name.as_str()),
        raw_form_id & 0x00FF_FFFF,
    );
    target_locator
        .by_form_key
        .get(&key)
        .map(|entry| entry.signature.as_str())
}

fn target_base_signature_is_valid(signature: &str) -> bool {
    !INVALID_PLACED_BASE_SIGNATURES.contains(&signature)
}

fn target_base_form_id_is_valid(
    raw_form_id: u32,
    target: &TargetFormIdContext,
    target_locator: &LocatorSection,
    target_existing_form_ids: &BTreeSet<u32>,
) -> bool {
    if (raw_form_id & 0xFF00_0000) != target.own_prefix {
        return true;
    }
    if !target_existing_form_ids.contains(&raw_form_id) {
        return false;
    }
    target_local_record_signature(target, target_locator, raw_form_id)
        .map(target_base_signature_is_valid)
        .unwrap_or(false)
}

fn stable_leveled_entry_index(seed: u64, len: usize) -> usize {
    let mut value = seed ^ 0x9E37_79B9_7F4A_7C15;
    value ^= value >> 30;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^= value >> 31;
    (value as usize) % len
}

fn collect_lvli_entry_target_raws(
    source_plugin: &ParsedPlugin,
    source_locator: &LocatorSection,
    source_own_index: u8,
    target: &TargetFormIdContext,
    target_locator: &LocatorSection,
    target_existing_form_ids: &BTreeSet<u32>,
    form_key_map: &BTreeMap<String, String>,
    lvli_record: &ParsedRecord,
    visited: &mut BTreeSet<String>,
    depth: usize,
    candidates: &mut Vec<(u32, Option<String>)>,
) {
    if depth >= 8 {
        return;
    }
    let lvli_key = render_form_key(source_plugin, lvli_record.form_id);
    if !visited.insert(lvli_key) {
        return;
    }
    for entry_raw in leveled_item_entry_form_ids(lvli_record) {
        let source_entry_key = render_form_key(source_plugin, entry_raw);
        let source_entry = locator_entry_by_form_key(source_locator, source_entry_key.as_str());
        let entry_record = source_entry.and_then(|e| source_locator.record(source_plugin, e));
        if let Some(source_entry) = source_entry {
            if source_entry.signature.as_str() == "LVLI" {
                if let Some(nested_record) = entry_record {
                    collect_lvli_entry_target_raws(
                        source_plugin,
                        source_locator,
                        source_own_index,
                        target,
                        target_locator,
                        target_existing_form_ids,
                        form_key_map,
                        nested_record,
                        visited,
                        depth + 1,
                        candidates,
                    );
                }
                continue;
            }
        }
        let Some(target_raw) = source_raw_form_id_to_target_raw(
            entry_raw,
            source_plugin,
            source_own_index,
            target,
            form_key_map,
        ) else {
            continue;
        };
        if target_base_form_id_is_valid(
            target_raw,
            target,
            target_locator,
            target_existing_form_ids,
        ) {
            let eid = entry_record.map(editor_id).filter(|s| !s.is_empty());
            candidates.push((target_raw, eid));
        }
    }
}

/// FO76 "Leveled Placed Item" convention: an `LPI_<name>` list used as a placed
/// base contains a `UseLPI_<name>` entry that is the default (non-nuked,
/// non-harvested) placement; nuke and harvested variants share the same list.
/// FO4 must pick one concrete base at conversion time, so prefer that default
/// entry — otherwise a placement can resolve to a nuke-only variant (e.g. a
/// flux-producing flora) that in FO76 only appears inside an active blast zone.
fn prefer_default_candidate(base_eid: &str, candidates: &[(u32, Option<String>)]) -> Option<u32> {
    if base_eid.is_empty() {
        return None;
    }
    let want = format!("use{}", base_eid.to_ascii_lowercase());
    candidates.iter().find_map(|(raw, eid)| match eid {
        Some(e) if e.to_ascii_lowercase() == want => Some(*raw),
        _ => None,
    })
}

fn replace_placed_lvli_base(
    record: &mut ParsedRecord,
    source_plugin: &ParsedPlugin,
    source_locator: &LocatorSection,
    source_own_index: u8,
    target: &TargetFormIdContext,
    target_locator: &LocatorSection,
    target_existing_form_ids: &BTreeSet<u32>,
    form_key_map: &BTreeMap<String, String>,
) -> Result<usize, &'static str> {
    let Some(base_form_id) = placed_child_base_form_id(record) else {
        return Ok(0);
    };
    if base_form_id == 0 {
        return Ok(0);
    }
    let base_key = render_form_key(source_plugin, base_form_id);
    let Some(base_entry) = locator_entry_by_form_key(source_locator, base_key.as_str()) else {
        return Ok(0);
    };
    if base_entry.signature.as_str() != "LVLI" {
        return Ok(0);
    }
    let Some(base_record) = source_locator.record(source_plugin, base_entry) else {
        return Err("unresolved_lvli_base");
    };
    let mut candidates: Vec<(u32, Option<String>)> = Vec::new();
    collect_lvli_entry_target_raws(
        source_plugin,
        source_locator,
        source_own_index,
        target,
        target_locator,
        target_existing_form_ids,
        form_key_map,
        base_record,
        &mut BTreeSet::new(),
        0,
        &mut candidates,
    );
    if candidates.is_empty() {
        return Err("unresolved_lvli_base");
    }
    let replacement_raw = prefer_default_candidate(&editor_id(base_record), &candidates)
        .unwrap_or_else(|| {
            let seed = ((record.form_id as u64) << 32) ^ base_form_id as u64;
            candidates[stable_leveled_entry_index(seed, candidates.len())].0
        });
    set_placed_child_base_form_id(record, replacement_raw)
        .then_some(1)
        .ok_or("unresolved_lvli_base")
}

/// FO76→FO4 placed-ref enum/flag normalization for copied REFR children.
///
/// FO76 carries enum/flag values FO4 rejects. xEdit flags these as
/// `<Unknown: N>` and FO4 can mis-handle the out-of-domain value:
///   - `XPRM` (struct:f×7,I) "Primitive \ Type" at offset 28: FO76 adds
///     6=value and 7=cylinder; FO4 only knows 0..=5 (none/box/sphere/
///     plane/line/ellipsoid). Clamp >5 to 0 (none).
///   - `XRDO` (struct:f,f,f,I) "Radio \ Flags" at offset 12: FO76 sets bit
///     8 (0x100) and bit 16 (0x10000); FO4 only defines bit 0 (0x1,
///     IgnoresDistanceChecks). Mask to 0x1.
///
/// Returns the rewritten subrecord bytes if a change was made.
fn normalize_placed_enum_flag_subrecord(sig: &str, data: &[u8]) -> Option<Vec<u8>> {
    let (offset, normalize): (usize, fn(u32) -> u32) = match sig {
        "XPRM" => (28, |v| if v > 5 { 0 } else { v }),
        "XRDO" => (12, |v| v & 0x1),
        _ => return None,
    };
    if data.len() < offset + 4 {
        return None;
    }
    let current = u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]);
    let normalized = normalize(current);
    if normalized == current {
        return None;
    }
    let mut out = data.to_vec();
    out[offset..offset + 4].copy_from_slice(&normalized.to_le_bytes());
    Some(out)
}

const XALG_NEVER_VISIBLE_DISTANT: u64 = 0x0000_0008;
const XALG_VISIBLE_DISTANT: u64 = 0x0000_0200;
const RECORD_FLAG_VISIBLE_WHEN_DISTANT: u32 = 0x0000_8000;

fn read_xalg_flags(data: &[u8]) -> u64 {
    let mut bytes = [0u8; 8];
    let len = data.len().min(bytes.len());
    bytes[..len].copy_from_slice(&data[..len]);
    u64::from_le_bytes(bytes)
}

fn placed_ref_xalg_flags(record: &ParsedRecord) -> u64 {
    effective_subrecords_for_record(record)
        .iter()
        .filter(|subrecord| subrecord.signature.as_str() == "XALG")
        .fold(0u64, |acc, subrecord| {
            acc | read_xalg_flags(subrecord.data.as_ref())
        })
}

fn carry_placed_ref_visible_distant_flag(record: &mut ParsedRecord) {
    if record.signature.as_str() != "REFR" {
        return;
    }
    let xalg_flags = placed_ref_xalg_flags(record);
    let previous = record.flags;
    if xalg_flags & XALG_NEVER_VISIBLE_DISTANT != 0 {
        record.flags &= !RECORD_FLAG_VISIBLE_WHEN_DISTANT;
    } else if xalg_flags & XALG_VISIBLE_DISTANT != 0 {
        record.flags |= RECORD_FLAG_VISIBLE_WHEN_DISTANT;
    }
    if record.flags != previous {
        record.raw_payload = None;
    }
}

/// REFR map markers carry a TNAM icon type. FO76 encodes it as a `uint16` (the
/// low byte is the type for every real value); FO4 expects `struct:B,B` (type
/// byte + unknown byte) with a valid icon range of 0–80. A verbatim copy lands
/// on the wrong icon (the enums are name-aligned, not index-aligned) and any
/// FO76 value > 80 overruns FO4's compass icon table and crashes the game.
/// Translate the FO76 type to its FO4 equivalent (generic icon when there is
/// none). Returns the rewritten 2-byte subrecord when the value changed.
fn remap_marker_tnam_type(data: &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() {
        return None;
    }
    let source_type = if data.len() >= 2 {
        u16::from_le_bytes([data[0], data[1]])
    } else {
        data[0] as u16
    };
    let fo4_type = super::marker_type::translate_marker_type_fo76_to_fo4(source_type);
    if data.len() == 2 && data[0] == fo4_type && data[1] == 0 {
        return None;
    }
    Some(vec![fo4_type, 0])
}

fn is_region_map_marker_ref(record: &ParsedRecord) -> bool {
    if record.signature.as_str() != "REFR" {
        return false;
    }
    if !editor_id(record)
        .to_ascii_lowercase()
        .starts_with("regionmapmarker")
    {
        return false;
    }
    effective_subrecords_for_record(record)
        .iter()
        .any(|sub| matches!(sub.signature.as_str(), "XMRK" | "FNAM" | "TNAM"))
}

fn strip_region_map_marker_payload(record: &mut ParsedRecord) -> usize {
    if !is_region_map_marker_ref(record) {
        return 0;
    }
    let before = record.subrecords.len();
    record
        .subrecords
        .retain(|sub| !matches!(sub.signature.as_str(), "XMRK" | "FNAM" | "FULL" | "TNAM"));
    let removed = before.saturating_sub(record.subrecords.len());
    if removed > 0 {
        record.raw_payload = None;
    }
    removed
}

fn xrgd_is_empty_bone_payload(data: &[u8]) -> bool {
    const XRGD_ROW_LEN: usize = 28;
    if data.is_empty() {
        return true;
    }
    if data.len() % XRGD_ROW_LEN != 0 {
        return false;
    }
    data.chunks_exact(XRGD_ROW_LEN).all(|row| {
        row[..4].iter().all(|byte| *byte == 0)
            && [4, 8, 12, 16, 20, 24].iter().all(|offset| {
                let offset = *offset;
                let value = f32::from_le_bytes([
                    row[offset],
                    row[offset + 1],
                    row[offset + 2],
                    row[offset + 3],
                ]);
                value == 0.0
            })
    })
}

fn rewrite_placed_child_local_refs(
    record: &mut ParsedRecord,
    source_plugin: &ParsedPlugin,
    source_own_index: u8,
    target: &TargetFormIdContext,
    target_locator: &LocatorSection,
    form_key_map: &BTreeMap<String, String>,
) -> usize {
    if record.subrecords.is_empty() {
        record.subrecords = effective_subrecords_for_record(record).into_owned();
    }

    let mut changed = 0usize;
    let record_is_refr = record.signature.as_str() == "REFR";
    carry_placed_ref_visible_distant_flag(record);

    // FO4 placed-ref XEZN (Encounter Zone) must point at an ECZN. FO76 has no
    // ECZN concept — its REFR.XEZN points at a LCTN (locations carry encounter
    // data there), and this conversion synthesizes no ECZN records. Remapping
    // the FO76 LCTN ref forward only yields a still-wrong-type REFR\XEZN->LCTN
    // (xEdit "Found a LCTN reference, expected: ECZN") which causes in-game
    // cell-load failures. With no derivable LCTN->ECZN mapping, strip XEZN from
    // copied placed refs (REFR/ACHR/PGRE): absent is valid; present-LCTN is not.
    if matches!(record.signature.as_str(), "REFR" | "ACHR" | "PGRE") {
        let before = record.subrecords.len();
        record.subrecords.retain(|sub| {
            if sub.signature.as_str() == "XEZN" {
                return false;
            }
            if record_is_refr
                && sub.signature.as_str() == "XRGD"
                && xrgd_is_empty_bone_payload(sub.data.as_ref())
            {
                return false;
            }
            true
        });
        let removed = before.saturating_sub(record.subrecords.len());
        if removed > 0 {
            record.raw_payload = None;
            changed += removed;
        }
    }

    if record_is_refr {
        changed += strip_region_map_marker_payload(record);
    }

    for subrecord in &mut record.subrecords {
        let sig = subrecord.signature.as_str();
        if record_is_refr && sig == "TNAM" {
            if let Some(data) = remap_marker_tnam_type(subrecord.data.as_ref()) {
                subrecord.data = Bytes::from(data);
                changed += 1;
            }
            continue;
        }
        if matches!(sig, "XPRM" | "XRDO") {
            if let Some(data) = normalize_placed_enum_flag_subrecord(sig, subrecord.data.as_ref()) {
                subrecord.data = Bytes::from(data);
                changed += 1;
            }
            continue;
        }
        if sig == "XLRT" {
            if subrecord.data.is_empty() || subrecord.data.len() % 4 != 0 {
                continue;
            }
            let mut data = subrecord.data.to_vec();
            let mut subrecord_changed = 0usize;
            for offset in (0..data.len()).step_by(4) {
                if rewrite_source_form_id_with_map_or_target_local(
                    &mut data,
                    offset,
                    source_plugin,
                    source_own_index,
                    target,
                    target_locator,
                    form_key_map,
                    &["LCRT"],
                ) {
                    subrecord_changed += 1;
                }
            }
            if subrecord_changed > 0 {
                subrecord.data = Bytes::from(data);
                changed += subrecord_changed;
            }
            continue;
        }
        let rewrite_offsets: &[usize] = if sig == "XLKR" {
            &[0, 4]
        } else if sig == "XLOC" {
            &[4]
        } else if sig == "XTEL" {
            // struct:I,f,f,f,f,f,f,I,I — door FormID at 0, transition_interior
            // FormID at 32 (offset 28 is a uint32 flags field, not a FormID).
            &[0, 32]
        } else if sig == "XPLK" {
            &[0]
        } else if matches!(
            sig,
            "NAME"
                | "XMSP"
                | "XLYR"
                | "XCZC"
                | "XLCN"
                | "XEZN"
                | "XLRL"
                | "XRFG"
                | "XOWN"
                | "XESP"
                | "XPWR"
                | "XAPR"
                | "XEMI"
                | "XATR"
                | "XLIB"
                | "XNDP"
                | "XTNM"
        ) {
            // XNDP (struct:I,h,B,B) Navmesh Door Link: only offset 0 is the
            // NAVM FormID. Offset 4 is the int16 triangle/marker index and the
            // two trailing bytes — none are FormIDs, so only &[0] is rewritten.
            // XTNM (Teleport Loc Name) is a bare 4-byte FormID targeting MESG.
            // XRDO (struct:f,f,f,I) and XPRM (struct:f,f,f,f,f,f,f,I) are
            // intentionally absent here: their FormID-shaped slot is a
            // uint32 flags/enum word, not a FormID. They are normalized
            // (mask/clamp) by normalize_placed_enum_flag_subrecord below.
            &[0]
        } else {
            continue;
        };
        let target_local_fallback_signatures: &[&str] = match sig {
            "XMSP" => &["MSWP"],
            "XLCN" => &["LCTN"],
            "XEZN" => &["ECZN"],
            "XLYR" => &["LAYR"],
            "XCZC" => &["CELL"],
            "XLOC" => &["KEYM"],
            "XPLK" => &["REFR", "ACHR"],
            // FO76 placed-ref FormID slots that must land on a target record
            // of the right type after copy. XEMI=Emittance, XATR=Attach Ref,
            // XLIB=Leveled Item Base Object.
            "XEMI" => &["LIGH", "REGN"],
            "XATR" => &[
                "REFR", "PGRE", "PHZD", "PMIS", "PARW", "PBAR", "PBEA", "PCON", "PFLA", "ACHR",
            ],
            "XLIB" => &["LVLI"],
            "XNDP" => &["NAVM"],
            "XTNM" => &["MESG"],
            _ => &[],
        };
        let mut data = subrecord.data.to_vec();
        let mut subrecord_changed = 0usize;
        for offset in rewrite_offsets {
            if rewrite_source_form_id_with_map_or_target_local(
                &mut data,
                *offset,
                source_plugin,
                source_own_index,
                target,
                target_locator,
                form_key_map,
                target_local_fallback_signatures,
            ) {
                subrecord_changed += 1;
            }
        }
        if subrecord_changed > 0 {
            subrecord.data = Bytes::from(data);
            changed += subrecord_changed;
        }
    }
    if changed > 0 {
        record.raw_payload = None;
    }
    changed
}

fn raw_target_local_ref_is_missing(
    raw_form_id: u32,
    target: &TargetFormIdContext,
    target_existing_form_ids: &BTreeSet<u32>,
) -> bool {
    raw_form_id != 0
        && (raw_form_id & 0xFF00_0000) == target.own_prefix
        && !target_existing_form_ids.contains(&raw_form_id)
}

fn subrecord_has_missing_target_local_ref(
    subrecord: &ParsedSubrecord,
    target: &TargetFormIdContext,
    target_existing_form_ids: &BTreeSet<u32>,
) -> bool {
    let offsets: &[usize] = match subrecord.signature.as_str() {
        // XLKR (linked refs) and XAPR (activate parents) are NOT dropped here.
        // The cell-slice copy runs in multiple passes (grid children vs.
        // persistent-cell synthesis), each with its own `target_existing_form_ids`
        // snapshot. A ref copied in a DIFFERENT pass looks "missing" from this
        // pass's snapshot and would be wrongly dropped. Dangling refs are dropped
        // later by `normalize_placed_records`, which sees the fully-assembled
        // target.
        "XRFG" | "XESP" | "XMSP" | "XCZC" | "XLCN" | "XEZN" | "XEMI" | "XATR" | "XLIB" | "XNDP"
        | "XTNM" => &[0],
        "XLRT" => {
            return !subrecord.data.is_empty()
                && subrecord.data.len() % 4 == 0
                && subrecord.data.chunks_exact(4).any(|chunk| {
                    raw_target_local_ref_is_missing(
                        u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
                        target,
                        target_existing_form_ids,
                    )
                });
        }
        _ => return false,
    };
    offsets.iter().any(|offset| {
        let offset = *offset;
        if subrecord.data.len() < offset + 4 {
            return false;
        }
        let raw_form_id = u32::from_le_bytes([
            subrecord.data[offset],
            subrecord.data[offset + 1],
            subrecord.data[offset + 2],
            subrecord.data[offset + 3],
        ]);
        raw_target_local_ref_is_missing(raw_form_id, target, target_existing_form_ids)
    })
}

fn drop_unresolved_placed_child_local_refs(
    record: &mut ParsedRecord,
    target: &TargetFormIdContext,
    target_existing_form_ids: &BTreeSet<u32>,
) -> usize {
    if record.subrecords.is_empty() {
        record.subrecords = effective_subrecords_for_record(record).into_owned();
    }
    let before = record.subrecords.len();
    record.subrecords.retain(|subrecord| {
        !subrecord_has_missing_target_local_ref(subrecord, target, target_existing_form_ids)
    });
    let removed = before.saturating_sub(record.subrecords.len());
    if removed > 0 {
        record.raw_payload = None;
    }
    removed
}

fn xprm_has_zero_extents(data: &[u8]) -> bool {
    if data.len() < 12 {
        return false;
    }
    let x = f32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let y = f32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let z = f32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    x == 0.0 && y == 0.0 && z == 0.0
}

fn drop_zero_extent_primitive_subrecords(record: &mut ParsedRecord) -> usize {
    if record.subrecords.is_empty() {
        record.subrecords = effective_subrecords_for_record(record).into_owned();
    }
    let before = record.subrecords.len();
    record.subrecords.retain(|subrecord| {
        subrecord.signature.as_str() != "XPRM" || !xprm_has_zero_extents(subrecord.data.as_ref())
    });
    let removed = before.saturating_sub(record.subrecords.len());
    if removed > 0 {
        record.raw_payload = None;
    }
    removed
}

fn placed_child_base_form_id(record: &ParsedRecord) -> Option<u32> {
    let name = subrecord_data(record, "NAME")?;
    if name.len() < 4 {
        return None;
    }
    Some(u32::from_le_bytes([name[0], name[1], name[2], name[3]]))
}

/// Diagnostic-only: render the SOURCE NAME base FormKey for a skipped persistent
/// ref, looked up by its source FormKey string. Returns `"<no-name>"` when the
/// record can't be resolved or has no NAME base. Used to surface which base the
/// dropped persistent refs (e.g. the missing MapMarkers) resolve to. Never
/// touches conversion state.
fn skipped_ref_source_base_key(
    source_plugin: &ParsedPlugin,
    source_locator: &LocatorSection,
    source_key: &str,
) -> String {
    let resolved = object_id_from_form_key(source_key).and_then(|object_id| {
        let key = FormKey::new(Arc::from(source_plugin.plugin_name.as_str()), object_id);
        let entry = source_locator.by_form_key.get(&key)?;
        let record = source_locator.record(source_plugin, entry)?;
        let base = placed_child_base_form_id(record)?;
        Some(render_form_key(source_plugin, base))
    });
    resolved.unwrap_or_else(|| "<no-name>".to_string())
}

fn set_placed_child_base_form_id(record: &mut ParsedRecord, raw_form_id: u32) -> bool {
    if record.subrecords.is_empty() {
        record.subrecords = effective_subrecords_for_record(record).into_owned();
    }
    for subrecord in &mut record.subrecords {
        if subrecord.signature.as_str() != "NAME" || subrecord.data.len() < 4 {
            continue;
        }
        let mut data = subrecord.data.to_vec();
        data[0..4].copy_from_slice(&raw_form_id.to_le_bytes());
        subrecord.data = Bytes::from(data);
        record.raw_payload = None;
        return true;
    }
    false
}

fn placed_child_has_valid_base(
    record: &ParsedRecord,
    target: &TargetFormIdContext,
    target_locator: &LocatorSection,
    target_existing_form_ids: &BTreeSet<u32>,
) -> bool {
    let Some(base_form_id) = placed_child_base_form_id(record) else {
        return false;
    };
    if base_form_id == 0 {
        return false;
    }
    if (base_form_id & 0xFF00_0000) != target.own_prefix {
        return true;
    }
    target_base_form_id_is_valid(
        base_form_id,
        target,
        target_locator,
        target_existing_form_ids,
    )
}

fn source_record_for_target_child(
    source_plugin: &ParsedPlugin,
    source_locator: &LocatorSection,
    target_child_key: &str,
    source_own_index: u8,
    target: &TargetFormIdContext,
    target_game: Option<&str>,
    target_locator: &LocatorSection,
    target_existing_form_ids: &BTreeSet<u32>,
    form_key_map: &BTreeMap<String, String>,
    offset: (f32, f32, f32),
) -> Result<(ParsedRecord, usize, usize, usize), &'static str> {
    let object_id = object_id_from_form_key(target_child_key).ok_or("invalid_child_key")?;
    let source_key = FormKey::new(Arc::from(source_plugin.plugin_name.as_str()), object_id);
    let Some(entry) = source_locator.by_form_key.get(&source_key) else {
        return Err("missing_source_child");
    };
    if !is_placed_child_signature(entry.signature.as_str()) {
        return Err("not_placed_child");
    }
    let Some(source_record) = source_locator.record(source_plugin, entry) else {
        return Err("missing_source_record");
    };
    let mut record = source_record.clone();
    record.form_id = normalized_form_key_text(source_key.render().as_str())
        .and_then(|source_norm| mapped_target_local_raw(source_norm.as_str(), target, form_key_map))
        .unwrap_or(target.own_prefix | object_id);
    let leveled_bases_resolved = replace_placed_lvli_base(
        &mut record,
        source_plugin,
        source_locator,
        source_own_index,
        target,
        target_locator,
        target_existing_form_ids,
        form_key_map,
    )?;
    let mapped_form_refs = rewrite_placed_child_local_refs(
        &mut record,
        source_plugin,
        source_own_index,
        target,
        target_locator,
        form_key_map,
    );
    let local_ref_subrecords_dropped =
        drop_unresolved_placed_child_local_refs(&mut record, target, target_existing_form_ids);
    let primitive_subrecords_dropped = drop_zero_extent_primitive_subrecords(&mut record);
    let schema_subrecords_dropped = local_ref_subrecords_dropped
        + primitive_subrecords_dropped
        + filter_record_to_target_schema(&mut record, target_game);
    if !placed_child_has_valid_base(&record, target, target_locator, target_existing_form_ids) {
        return Err("missing_base");
    }
    apply_placed_record_position_offset_to_record(&mut record, offset);
    Ok((
        record,
        mapped_form_refs,
        leveled_bases_resolved,
        schema_subrecords_dropped,
    ))
}

struct CopyCellChildrenContext<'a> {
    source_plugin: &'a ParsedPlugin,
    source_locator: &'a LocatorSection,
    source_own_index: u8,
    target: &'a TargetFormIdContext,
    target_game: Option<&'a str>,
    target_locator: &'a LocatorSection,
    target_existing_form_ids: &'a BTreeSet<u32>,
    target_cell_by_grid: &'a BTreeMap<(i32, i32), u32>,
    form_key_map: &'a BTreeMap<String, String>,
    header_size: usize,
    offset: (f32, f32, f32),
}

#[derive(Default)]
struct PreparedCellChildren {
    persistent: Vec<ParsedRecord>,
    temporary: Vec<ParsedRecord>,
}

const EXTERIOR_CELL_SIZE: f32 = 4096.0;

fn placed_record_cell_grid(record: &ParsedRecord) -> Option<(i32, i32)> {
    let data = subrecord_data(record, "DATA")?;
    if data.len() < 8 {
        return None;
    }
    let x = f32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let y = f32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let cell_x = (x / EXTERIOR_CELL_SIZE).floor();
    let cell_y = (y / EXTERIOR_CELL_SIZE).floor();
    if cell_x < i32::MIN as f32
        || cell_x > i32::MAX as f32
        || cell_y < i32::MIN as f32
        || cell_y > i32::MAX as f32
    {
        return None;
    }
    Some((cell_x as i32, cell_y as i32))
}

fn collect_target_cell_by_grid(items: &[ParsedItem], out: &mut BTreeMap<(i32, i32), u32>) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "CELL" => {
                if let Some(grid) = cell_grid(record) {
                    out.entry(grid).or_insert(record.form_id);
                }
            }
            ParsedItem::Group(group) => collect_target_cell_by_grid(&group.children, out),
            _ => {}
        }
    }
}

fn target_cell_for_placed_record(
    record: &ParsedRecord,
    fallback_raw_cell_id: u32,
    target_cell_by_grid: &BTreeMap<(i32, i32), u32>,
) -> (u32, bool) {
    let Some(grid) = placed_record_cell_grid(record) else {
        return (fallback_raw_cell_id, false);
    };
    let Some(raw_cell_id) = target_cell_by_grid.get(&grid).copied() else {
        return (fallback_raw_cell_id, false);
    };
    (raw_cell_id, raw_cell_id != fallback_raw_cell_id)
}

const PREPARE_PAR_THRESHOLD: usize = 64;

fn prepare_source_children_for_target_cells(
    children_by_target_cell: BTreeMap<String, CellChildrenPayload>,
    ctx: &CopyCellChildrenContext<'_>,
    payload: &mut CellSliceInsertPayload,
) -> BTreeMap<u32, PreparedCellChildren> {
    prepare_source_children_for_target_cells_with_threshold(
        children_by_target_cell,
        ctx,
        payload,
        PREPARE_PAR_THRESHOLD,
    )
}

fn prepare_source_children_for_target_cells_with_threshold(
    children_by_target_cell: BTreeMap<String, CellChildrenPayload>,
    ctx: &CopyCellChildrenContext<'_>,
    payload: &mut CellSliceInsertPayload,
    par_threshold: usize,
) -> BTreeMap<u32, PreparedCellChildren> {
    // 1. Flatten serially — BTreeMap order, persistent-then-temporary, key
    //    order: exactly the legacy iteration order. Invalid-cell-key warnings
    //    fire here, in the same order as the legacy loop.
    let mut flat: Vec<(i32, u32, String)> = Vec::new();
    for (target_cell_key, sections) in children_by_target_cell {
        let Some(object_id) = object_id_from_form_key(target_cell_key.as_str()) else {
            payload
                .warnings
                .push(format!("invalid target cell key: {target_cell_key}"));
            continue;
        };
        let fallback_raw_cell_id = ctx.target.own_prefix | object_id;
        for (group_type, child_keys) in [
            (PERSISTENT_GROUP, sections.persistent),
            (TEMPORARY_GROUP, sections.temporary),
        ] {
            for key in child_keys {
                flat.push((group_type, fallback_raw_cell_id, key));
            }
        }
    }

    // 2. Parallel convert — pure per child over the frozen ctx; indexed collect
    //    preserves input order, so the serial fold below reproduces the legacy
    //    bucket/counter/skip order exactly, independent of thread count.
    type ConvertResult = Result<(ParsedRecord, usize, usize, usize), &'static str>;
    let convert = |(_, _, key): &(i32, u32, String)| -> ConvertResult {
        source_record_for_target_child(
            ctx.source_plugin,
            ctx.source_locator,
            key.as_str(),
            ctx.source_own_index,
            ctx.target,
            ctx.target_game,
            ctx.target_locator,
            ctx.target_existing_form_ids,
            ctx.form_key_map,
            ctx.offset,
        )
    };
    let results: Vec<ConvertResult> = if flat.len() < par_threshold {
        flat.iter().map(convert).collect()
    } else {
        use rayon::prelude::*;
        flat.par_iter().map(convert).collect()
    };

    // 3. Serial fold in flat order — byte-for-byte the legacy loop's effects.
    let mut prepared: BTreeMap<u32, PreparedCellChildren> = BTreeMap::new();
    for ((group_type, fallback_raw_cell_id, target_child_key), result) in
        flat.into_iter().zip(results)
    {
        match result {
            Ok((record, mapped_form_refs, leveled_bases_resolved, schema_subrecords_dropped)) => {
                payload.mapped_form_refs += mapped_form_refs;
                payload.leveled_bases_resolved += leveled_bases_resolved;
                payload.schema_subrecords_dropped += schema_subrecords_dropped;
                let (destination_cell_id, rebucketed) = target_cell_for_placed_record(
                    &record,
                    fallback_raw_cell_id,
                    ctx.target_cell_by_grid,
                );
                if rebucketed {
                    payload.children_rebucketed += 1;
                }
                let bucket = prepared.entry(destination_cell_id).or_default();
                if group_type == PERSISTENT_GROUP {
                    bucket.persistent.push(record);
                } else {
                    bucket.temporary.push(record);
                }
            }
            Err(reason) => {
                if reason == "missing_base" || reason == "unresolved_lvli_base" {
                    payload.missing_base_children += 1;
                }
                payload.skipped_children.push(target_child_key);
            }
        }
    }
    prepared
}

fn insert_prepared_children_into_target_cells(
    items: &mut [ParsedItem],
    remaining: &mut BTreeMap<u32, PreparedCellChildren>,
    ctx: &CopyCellChildrenContext<'_>,
    payload: &mut CellSliceInsertPayload,
) {
    for item in items {
        let ParsedItem::Group(group) = item else {
            continue;
        };
        if group.group_type == CELL_CHILD_GROUP {
            if let Some(raw_cell_id) = decode_group_form_id(group) {
                if let Some(sections) = remaining.remove(&raw_cell_id) {
                    let before = payload.children_inserted;
                    let persistent = ensure_child_group_mut(
                        group,
                        PERSISTENT_GROUP,
                        raw_cell_id,
                        ctx.header_size,
                    );
                    for record in sections.persistent {
                        persistent.children.push(ParsedItem::Record(record));
                        payload.children_inserted += 1;
                    }
                    let temporary = ensure_child_group_mut(
                        group,
                        TEMPORARY_GROUP,
                        raw_cell_id,
                        ctx.header_size,
                    );
                    for record in sections.temporary {
                        temporary.children.push(ParsedItem::Record(record));
                        payload.children_inserted += 1;
                    }
                    if payload.children_inserted > before {
                        payload.cells_touched += 1;
                    }
                }
            }
        }
        insert_prepared_children_into_target_cells(&mut group.children, remaining, ctx, payload);
    }
}

fn take_matching_record_for_form_key(
    items: &mut Vec<ParsedItem>,
    own_name: &Arc<str>,
    masters: &[String],
    form_key: &str,
) -> Option<ParsedRecord> {
    let mut index = 0;
    while index < items.len() {
        match &mut items[index] {
            ParsedItem::Record(record) => {
                if resolve_form_id_to_form_key(record.form_id, own_name, masters)
                    .render()
                    .eq_ignore_ascii_case(form_key)
                {
                    let ParsedItem::Record(record) = items.remove(index) else {
                        unreachable!();
                    };
                    return Some(record);
                }
                index += 1;
            }
            ParsedItem::Group(group) => {
                if let Some(record) = take_matching_record_for_form_key(
                    &mut group.children,
                    own_name,
                    masters,
                    form_key,
                ) {
                    return Some(record);
                }
                index += 1;
            }
        }
    }
    None
}

fn find_cell_child_group_mut(
    items: &mut [ParsedItem],
    raw_cell_id: u32,
) -> Option<&mut ParsedGroup> {
    for item in items {
        let ParsedItem::Group(group) = item else {
            continue;
        };
        if group.group_type == CELL_CHILD_GROUP && decode_group_form_id(group) == Some(raw_cell_id)
        {
            return Some(group);
        }
        if let Some(found) = find_cell_child_group_mut(&mut group.children, raw_cell_id) {
            return Some(found);
        }
    }
    None
}

fn collect_record_object_ids(items: &[ParsedItem], signature: &str, out: &mut BTreeSet<u32>) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == signature => {
                out.insert(record.form_id & 0x00FF_FFFF);
            }
            ParsedItem::Group(group) => collect_record_object_ids(&group.children, signature, out),
            _ => {}
        }
    }
}

fn rewrite_cell_region_refs_to_local_records(plugin: &mut ParsedPlugin) -> usize {
    let mut local_region_ids = BTreeSet::new();
    collect_record_object_ids(&plugin.root_items, "REGN", &mut local_region_ids);
    if local_region_ids.is_empty() {
        return 0;
    }
    let own_prefix = (plugin.header.masters.len() as u32) << 24;
    rewrite_cell_region_refs_in_items(&mut plugin.root_items, &local_region_ids, own_prefix)
}

fn rewrite_cell_region_refs_in_items(
    items: &mut [ParsedItem],
    local_region_ids: &BTreeSet<u32>,
    own_prefix: u32,
) -> usize {
    let mut changed = 0usize;
    for item in items {
        match item {
            ParsedItem::Record(record) => {
                changed += rewrite_cell_region_refs_in_record(record, local_region_ids, own_prefix);
            }
            ParsedItem::Group(group) => {
                changed += rewrite_cell_region_refs_in_items(
                    &mut group.children,
                    local_region_ids,
                    own_prefix,
                );
            }
        }
    }
    changed
}

fn rewrite_cell_region_refs_in_record(
    record: &mut ParsedRecord,
    local_region_ids: &BTreeSet<u32>,
    own_prefix: u32,
) -> usize {
    if record.signature.as_str() != "CELL" {
        return 0;
    }
    if record.subrecords.is_empty() {
        record.subrecords = effective_subrecords_for_record(record).into_owned();
    }
    let mut changed = 0usize;
    for subrecord in &mut record.subrecords {
        if subrecord.signature.as_str() != "XCLR"
            || subrecord.data.is_empty()
            || subrecord.data.len() % 4 != 0
        {
            continue;
        }
        let mut data = subrecord.data.to_vec();
        let mut subrecord_changed = 0usize;
        for chunk in data.chunks_exact_mut(4) {
            let raw_form_id = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            let object_id = raw_form_id & 0x00FF_FFFF;
            if raw_form_id == 0 || !local_region_ids.contains(&object_id) {
                continue;
            }
            let local_form_id = own_prefix | object_id;
            if local_form_id == raw_form_id {
                continue;
            }
            chunk.copy_from_slice(&local_form_id.to_le_bytes());
            subrecord_changed += 1;
        }
        if subrecord_changed > 0 {
            subrecord.data = Bytes::from(data);
            changed += subrecord_changed;
        }
    }
    if changed > 0 {
        record.raw_payload = None;
    }
    changed
}

fn collect_cell_regions_by_grid(
    source: &ParsedPlugin,
    worldspace_editor_id: &str,
    payload: &mut CellRegionSyncPayload,
) -> BTreeMap<(i32, i32), Vec<u32>> {
    let mut by_grid = BTreeMap::new();
    let mut source_region_ids = BTreeSet::new();
    collect_record_object_ids(&source.root_items, "REGN", &mut source_region_ids);
    if source_region_ids.is_empty() {
        payload
            .warnings
            .push("source has no REGN records for CELL.XCLR sync".to_string());
        return by_grid;
    }
    let Some(wrld_group) = top_group(source, "WRLD") else {
        payload.warnings.push(format!(
            "source WRLD top group not found: {worldspace_editor_id}"
        ));
        return by_grid;
    };
    let (world_record, warnings) = find_world(source, worldspace_editor_id);
    payload.warnings.extend(warnings);
    let Some(world_record) = world_record else {
        return by_grid;
    };
    let Some(world_children) = find_world_children_group(wrld_group, world_record.form_id) else {
        payload.warnings.push(format!(
            "source world children group not found: {worldspace_editor_id}"
        ));
        return by_grid;
    };

    let persistent_cell_id = direct_world_persistent_cell_id(world_children);
    let mut cells = Vec::new();
    collect_group_records(world_children, "CELL", &mut cells);
    for cell in cells {
        if persistent_cell_id == Some(cell.form_id) {
            continue;
        }
        let Some(grid) = cell_grid(cell) else {
            continue;
        };
        let Some(data) = subrecord_data(cell, "XCLR") else {
            continue;
        };
        if data.is_empty() || data.len() % 4 != 0 {
            continue;
        }
        let mut region_ids = Vec::new();
        let mut seen = BTreeSet::new();
        for chunk in data.chunks_exact(4) {
            let raw_form_id = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if raw_form_id == 0 || (raw_form_id >> 24) != 0 {
                continue;
            }
            let object_id = raw_form_id & 0x00FF_FFFF;
            if !source_region_ids.contains(&object_id) {
                continue;
            }
            if seen.insert(object_id) {
                region_ids.push(object_id);
            }
        }
        if !region_ids.is_empty() {
            payload.source_cells_indexed += 1;
            by_grid.insert(grid, region_ids);
        }
    }
    by_grid
}

fn sync_cell_regions_from_source_worldspace(
    source: &ParsedPlugin,
    target: &mut ParsedPlugin,
    source_worldspace_editor_id: &str,
    target_worldspace_editor_id: &str,
) -> CellRegionSyncPayload {
    let mut payload = CellRegionSyncPayload::default();
    let source_regions =
        collect_cell_regions_by_grid(source, source_worldspace_editor_id, &mut payload);
    if source_regions.is_empty() {
        return payload;
    }

    let mut local_region_ids = BTreeSet::new();
    collect_record_object_ids(&target.root_items, "REGN", &mut local_region_ids);
    if local_region_ids.is_empty() {
        payload
            .warnings
            .push("target has no converted REGN records for CELL.XCLR sync".to_string());
        return payload;
    }

    let target_world_form_id = {
        let (world_record, warnings) = find_world(target, target_worldspace_editor_id);
        payload.warnings.extend(warnings);
        let Some(world_record) = world_record else {
            return payload;
        };
        world_record.form_id
    };
    let own_prefix = (target.header.masters.len() as u32) << 24;
    let Some(wrld_group) = top_group_mut(target, "WRLD") else {
        payload.warnings.push(format!(
            "target WRLD top group not found: {target_worldspace_editor_id}"
        ));
        return payload;
    };
    let Some(world_children) = find_world_children_group_mut(wrld_group, target_world_form_id)
    else {
        payload.warnings.push(format!(
            "target world children group not found: {target_worldspace_editor_id}"
        ));
        return payload;
    };

    sync_cell_regions_in_items(
        &mut world_children.children,
        &source_regions,
        &local_region_ids,
        own_prefix,
        &mut payload,
    );
    payload
}

fn sync_cell_regions_in_items(
    items: &mut [ParsedItem],
    source_regions: &BTreeMap<(i32, i32), Vec<u32>>,
    local_region_ids: &BTreeSet<u32>,
    own_prefix: u32,
    payload: &mut CellRegionSyncPayload,
) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "CELL" => {
                let Some(grid) = cell_grid(record) else {
                    continue;
                };
                payload.target_cells_seen += 1;
                let Some(source_region_ids) = source_regions.get(&grid) else {
                    payload.unmatched_target_cells += 1;
                    continue;
                };
                let mut target_region_refs = Vec::new();
                for object_id in source_region_ids {
                    if !local_region_ids.contains(object_id) {
                        payload.missing_target_regions += 1;
                        continue;
                    }
                    target_region_refs.push(own_prefix | object_id);
                }
                if target_region_refs.is_empty() {
                    continue;
                }
                match set_or_replace_cell_regions(record, &target_region_refs) {
                    CellRegionMutation::Inserted => {
                        payload.cells_changed += 1;
                        payload.region_refs_written += target_region_refs.len();
                    }
                    CellRegionMutation::Replaced => {
                        payload.cells_changed += 1;
                        payload.cells_retagged += 1;
                        payload.region_refs_written += target_region_refs.len();
                    }
                    CellRegionMutation::AlreadyTagged => payload.cells_already_tagged += 1,
                }
            }
            ParsedItem::Group(group) => sync_cell_regions_in_items(
                &mut group.children,
                source_regions,
                local_region_ids,
                own_prefix,
                payload,
            ),
            _ => {}
        }
    }
}

enum CellRegionMutation {
    Inserted,
    Replaced,
    AlreadyTagged,
}

fn set_or_replace_cell_regions(
    record: &mut ParsedRecord,
    region_refs: &[u32],
) -> CellRegionMutation {
    let new_data = region_refs
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    if record.subrecords.is_empty() {
        record.subrecords = effective_subrecords_for_record(record).into_owned();
    }

    let mut old_data = None;
    let original_len = record.subrecords.len();
    record.subrecords.retain(|subrecord| {
        if subrecord.signature.as_str() == "XCLR" {
            if old_data.is_none() {
                old_data = Some(subrecord.data.to_vec());
            }
            false
        } else {
            true
        }
    });

    let insert_at = record
        .subrecords
        .iter()
        .position(|subrecord| subrecord.signature.as_str() == "XLCN")
        .or_else(|| {
            record
                .subrecords
                .iter()
                .rposition(|subrecord| {
                    matches!(subrecord.signature.as_str(), "XCLW" | "LTMP" | "XCLC")
                })
                .map(|index| index + 1)
        })
        .unwrap_or(record.subrecords.len());
    record.subrecords.insert(
        insert_at,
        ParsedSubrecord {
            signature: "XCLR".into(),
            data: Bytes::from(new_data.clone()),
            semantic_type: None,
        },
    );

    let mutation = if original_len == record.subrecords.len()
        && old_data.as_deref() == Some(new_data.as_slice())
    {
        CellRegionMutation::AlreadyTagged
    } else if old_data.is_some() {
        CellRegionMutation::Replaced
    } else {
        CellRegionMutation::Inserted
    };
    if !matches!(mutation, CellRegionMutation::AlreadyTagged) {
        record.raw_payload = None;
    }
    mutation
}

fn sync_cell_locations_from_lctn_world_cells(plugin: &mut ParsedPlugin) -> CellLocationSyncPayload {
    let mut locations = BTreeMap::new();
    let mut conflicts = BTreeSet::new();
    collect_lctn_world_cell_locations(&plugin.root_items, &mut locations, &mut conflicts);
    let conflict_count = conflicts.len();
    for key in conflicts {
        locations.remove(&key);
    }

    let mut payload = CellLocationSyncPayload {
        locations_indexed: locations.len(),
        location_conflicts: conflict_count,
        ..CellLocationSyncPayload::default()
    };
    if locations.is_empty() {
        return payload;
    }

    tag_cell_locations_in_items(&mut plugin.root_items, None, &locations, &mut payload);
    payload
}

fn collect_lctn_world_cell_locations(
    items: &[ParsedItem],
    locations: &mut BTreeMap<(u32, i16, i16), u32>,
    conflicts: &mut BTreeSet<(u32, i16, i16)>,
) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "LCTN" => {
                collect_lctn_record_world_cell_locations(record, locations, conflicts);
            }
            ParsedItem::Group(group) => {
                collect_lctn_world_cell_locations(&group.children, locations, conflicts);
            }
            _ => {}
        }
    }
}

fn collect_lctn_record_world_cell_locations(
    record: &ParsedRecord,
    locations: &mut BTreeMap<(u32, i16, i16), u32>,
    conflicts: &mut BTreeSet<(u32, i16, i16)>,
) {
    for subrecord in &record.subrecords {
        if !matches!(subrecord.signature.as_str(), "ACEC" | "LCEC") {
            continue;
        }
        let data = subrecord.data.as_ref();
        if data.len() < 8 || (data.len() - 4) % 4 != 0 {
            continue;
        }
        let world = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if world == 0 {
            continue;
        }
        for cell in data[4..].chunks_exact(4) {
            let y = i16::from_le_bytes([cell[0], cell[1]]);
            let x = i16::from_le_bytes([cell[2], cell[3]]);
            let key = (world, x, y);
            if let Some(existing) = locations.get(&key).copied() {
                if existing != record.form_id {
                    conflicts.insert(key);
                }
            } else {
                locations.insert(key, record.form_id);
            }
        }
    }
}

fn tag_cell_locations_in_items(
    items: &mut [ParsedItem],
    current_world: Option<u32>,
    locations: &BTreeMap<(u32, i16, i16), u32>,
    payload: &mut CellLocationSyncPayload,
) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "CELL" => {
                let Some(world) = current_world else {
                    continue;
                };
                let Some((x, y)) = cell_grid_i16(record) else {
                    continue;
                };
                let Some(location) = locations.get(&(world, x, y)).copied() else {
                    continue;
                };
                match set_or_replace_cell_location(record, location) {
                    CellLocationMutation::Inserted => payload.cells_changed += 1,
                    CellLocationMutation::Replaced => {
                        payload.cells_changed += 1;
                        payload.cells_retagged += 1;
                    }
                    CellLocationMutation::AlreadyTagged => payload.cells_already_tagged += 1,
                }
            }
            ParsedItem::Group(group) => {
                let child_world = if group.group_type == 1 {
                    Some(u32::from_le_bytes(group.label))
                } else {
                    current_world
                };
                tag_cell_locations_in_items(&mut group.children, child_world, locations, payload);
            }
            _ => {}
        }
    }
}

fn cell_grid_i16(record: &ParsedRecord) -> Option<(i16, i16)> {
    let (x, y) = cell_grid(record)?;
    if x < i16::MIN as i32 || x > i16::MAX as i32 || y < i16::MIN as i32 || y > i16::MAX as i32 {
        return None;
    }
    Some((x as i16, y as i16))
}

enum CellLocationMutation {
    Inserted,
    Replaced,
    AlreadyTagged,
}

fn set_or_replace_cell_location(record: &mut ParsedRecord, location: u32) -> CellLocationMutation {
    if record.subrecords.is_empty() {
        record.subrecords = effective_subrecords_for_record(record).into_owned();
    }
    if let Some(existing) = record
        .subrecords
        .iter_mut()
        .find(|subrecord| subrecord.signature.as_str() == "XLCN")
    {
        if existing.data.len() >= 4 {
            let raw = u32::from_le_bytes([
                existing.data[0],
                existing.data[1],
                existing.data[2],
                existing.data[3],
            ]);
            if raw == location {
                return CellLocationMutation::AlreadyTagged;
            }
        }
        existing.data = Bytes::copy_from_slice(&location.to_le_bytes());
        record.raw_payload = None;
        return CellLocationMutation::Replaced;
    }

    let subrecord = ParsedSubrecord {
        signature: "XLCN".into(),
        data: Bytes::copy_from_slice(&location.to_le_bytes()),
        semantic_type: None,
    };
    let insert_at = record
        .subrecords
        .iter()
        .rposition(|subrecord| matches!(subrecord.signature.as_str(), "XCLW" | "LTMP" | "XCLC"))
        .map(|index| index + 1)
        .unwrap_or(record.subrecords.len());
    record.subrecords.insert(insert_at, subrecord);
    record.raw_payload = None;
    CellLocationMutation::Inserted
}

/// Per-cell gather produced by the parallel phase of the roots collection
/// each key Vec holds the cell's first-occurrence candidates in
/// legacy per-record order, collected against fresh per-cell seen-sets.
struct CellRootsGather {
    cell_key: String,
    grid_x: i32,
    grid_y: i32,
    region_keys: Vec<String>,
    location_keys: Vec<String>,
    static_base_keys: Vec<String>,
    leveled_base_entry_keys: Vec<String>,
    linked_ref_keyword_keys: Vec<String>,
    layer_keys: Vec<String>,
    children: CellChildrenPayload,
}

/// Serial replay of a gathered candidate stream against a GLOBAL seen-set:
/// identical to the legacy in-loop `seen.insert(key) -> push(key)` gating.
fn replay_first_occurrences(
    local: Vec<String>,
    global: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
) {
    for key in local {
        if seen.insert(key.clone()) {
            global.push(key);
        }
    }
}

/// Enumerate every record nested under a cell's child group (group type 6),
/// across persistent (8) and temporary (9) subgroups. Works for interior cells
/// (under the CELL top group) and exterior cells (under WRLD) alike — the child
/// group walker is keyed by cell form id, not by worldspace. Returns each child
/// as `{form_key, signature, group_type}`; base/transform resolution is left to
/// the caller via the existing per-record helpers.
pub(crate) fn plugin_handle_collect_cell_children_json(
    handle_id: u64,
    cell_form_id: u32,
) -> PyResult<String> {
    #[derive(serde::Serialize)]
    struct ChildEntry {
        form_id: u32,
        form_key: String,
        signature: String,
        group_type: i32,
    }

    fn walk(plugin: &ParsedPlugin, group: &ParsedGroup, out: &mut Vec<ChildEntry>) {
        for item in &group.children {
            match item {
                ParsedItem::Record(record) => out.push(ChildEntry {
                    form_id: record.form_id,
                    form_key: render_form_key(plugin, record.form_id),
                    signature: record.signature.to_string(),
                    group_type: group.group_type,
                }),
                ParsedItem::Group(child) => walk(plugin, child, out),
            }
        }
    }

    let store = plugin_handle_store_ref().lock().unwrap();
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let plugin = &slot.parsed;

    let mut child_groups_by_cell = BTreeMap::new();
    for item in &plugin.root_items {
        if let ParsedItem::Group(group) = item {
            collect_cell_child_groups(group, &mut child_groups_by_cell);
        }
    }

    let target = cell_form_id & 0x00FF_FFFF;
    let mut out: Vec<ChildEntry> = Vec::new();
    if let Some((_, child_group)) = child_groups_by_cell
        .iter()
        .find(|(key, _)| (**key & 0x00FF_FFFF) == target)
    {
        walk(plugin, child_group, &mut out);
    }
    serde_json::to_string(&out)
        .map_err(|err| PyValueError::new_err(format!("failed to encode cell children: {err}")))
}

pub(crate) fn plugin_handle_collect_cell_slice_roots_json(
    handle_id: u64,
    worldspace_editor_id: &str,
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
    include_worldspace_persistent_cell: bool,
    worker_count: Option<usize>,
) -> PyResult<String> {
    let payload = collect_cell_slice_roots_payload(
        handle_id,
        worldspace_editor_id,
        min_x,
        min_y,
        max_x,
        max_y,
        include_worldspace_persistent_cell,
        worker_count,
    )?;
    serde_json::to_string(&payload)
        .map_err(|err| PyValueError::new_err(format!("failed to encode cell roots: {err}")))
}

/// Struct-returning core of the cell-slice roots collection, for native
/// callers (the conversion crate's projected-placed orchestrator). Identical
/// walk/ordering semantics to the JSON entry point — that wrapper delegates
/// here — but the multi-million-key topology never leaves Rust.
pub fn collect_cell_slice_roots_payload(
    handle_id: u64,
    worldspace_editor_id: &str,
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
    include_worldspace_persistent_cell: bool,
    worker_count: Option<usize>,
) -> PyResult<CellSliceRootsPayload> {
    let total_start = Instant::now();
    let mut payload = CellSliceRootsPayload::default();
    let mut cell_seen = BTreeSet::new();
    let mut placed_seen = BTreeSet::new();
    let mut static_base_seen = BTreeSet::new();
    let mut leveled_base_entry_seen = BTreeSet::new();
    let mut linked_ref_keyword_seen = BTreeSet::new();
    let mut layer_seen = BTreeSet::new();
    let mut location_seen = BTreeSet::new();
    let mut location_data_seen = BTreeSet::new();
    let mut region_seen = BTreeSet::new();
    let mut region_data_seen = BTreeSet::new();
    let mut audio_data_seen = BTreeSet::new();
    let mut worldspace_data_seen = BTreeSet::new();

    let store = plugin_handle_store_ref().lock().unwrap();
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let plugin = &slot.parsed;
    let locator = build_locator_section(plugin);

    let lookup_start = Instant::now();
    let Some(wrld_group) = top_group(plugin, "WRLD") else {
        payload
            .warnings
            .push(format!("WRLD top group not found: {worldspace_editor_id}"));
        payload.timing.insert(
            "world_lookup_ms".to_string(),
            lookup_start.elapsed().as_millis(),
        );
        payload.timing.insert("cell_traversal_ms".to_string(), 0);
        payload
            .timing
            .insert("total_ms".to_string(), total_start.elapsed().as_millis());
        return Ok(payload);
    };
    let (world_record, warnings) = find_world(plugin, worldspace_editor_id);
    payload.warnings.extend(warnings);
    let Some(world_record) = world_record else {
        payload.timing.insert(
            "world_lookup_ms".to_string(),
            lookup_start.elapsed().as_millis(),
        );
        payload.timing.insert("cell_traversal_ms".to_string(), 0);
        payload
            .timing
            .insert("total_ms".to_string(), total_start.elapsed().as_millis());
        return Ok(payload);
    };
    let Some(world_children) = find_world_children_group(wrld_group, world_record.form_id) else {
        payload.warnings.push(format!(
            "world children group not found: {worldspace_editor_id}"
        ));
        payload.timing.insert(
            "world_lookup_ms".to_string(),
            lookup_start.elapsed().as_millis(),
        );
        payload.timing.insert("cell_traversal_ms".to_string(), 0);
        payload
            .timing
            .insert("total_ms".to_string(), total_start.elapsed().as_millis());
        return Ok(payload);
    };
    payload.timing.insert(
        "world_lookup_ms".to_string(),
        lookup_start.elapsed().as_millis(),
    );
    payload
        .worldspace_form_keys
        .push(render_form_key(plugin, world_record.form_id));

    let traversal_start = Instant::now();
    let mut cells = Vec::new();
    let persistent_cell_id = direct_world_persistent_cell_id(world_children);
    let mut child_groups_by_cell = BTreeMap::new();
    collect_cell_child_groups(world_children, &mut child_groups_by_cell);
    collect_group_records(world_children, "CELL", &mut cells);

    // Serial pre-filter: persistent-cell skip, XCLC warning (legacy order),
    // bounds check — survivors keep the legacy cells order.
    let mut in_bounds: Vec<(&ParsedRecord, i32, i32)> = Vec::new();
    for cell in cells {
        if persistent_cell_id == Some(cell.form_id) {
            continue;
        }
        let Some((grid_x, grid_y)) = cell_grid(cell) else {
            payload.warnings.push(format!(
                "CELL without XCLC skipped: {}",
                render_form_key(plugin, cell.form_id)
            ));
            continue;
        };
        if !inside_bounds(grid_x, grid_y, min_x, min_y, max_x, max_y) {
            continue;
        }
        in_bounds.push((cell, grid_x, grid_y));
    }

    // Pass 1 (parallel): per-cell gather using the UNCHANGED
    // walkers/helpers with FRESH per-cell seen-sets. Equivalence to the legacy
    // shared-set loop: every push site in this path is gated as
    // `seen.insert(key) -> push(key)`, and candidate GENERATION (including the
    // unconditional base DEFL/LVLI expansion, whose recursion is bounded by a
    // per-call local `visited` set) never reads the seen-sets. Fresh-set
    // collection therefore yields each cell's first-occurrence candidates in
    // legacy per-record order; the serial replay below applies those streams
    // to the GLOBAL sets in cells order, reproducing the legacy global
    // first-occurrence order exactly, independent of thread count.
    let gathers: Vec<CellRootsGather> = {
        use rayon::prelude::*;
        let gather_cells = || {
            in_bounds
                .par_iter()
                .map(|&(cell, grid_x, grid_y)| {
                    let mut gather = CellRootsGather {
                        cell_key: render_form_key(plugin, cell.form_id),
                        grid_x,
                        grid_y,
                        region_keys: Vec::new(),
                        location_keys: Vec::new(),
                        static_base_keys: Vec::new(),
                        leveled_base_entry_keys: Vec::new(),
                        linked_ref_keyword_keys: Vec::new(),
                        layer_keys: Vec::new(),
                        children: CellChildrenPayload::default(),
                    };
                    let mut region_seen_local = BTreeSet::new();
                    let mut location_seen_local = BTreeSet::new();
                    let mut static_base_seen_local = BTreeSet::new();
                    let mut leveled_seen_local = BTreeSet::new();
                    let mut keyword_seen_local = BTreeSet::new();
                    let mut layer_seen_local = BTreeSet::new();
                    append_form_keys_from_array_subrecord(
                        plugin,
                        &locator,
                        cell,
                        "XCLR",
                        "REGN",
                        &mut gather.region_keys,
                        &mut region_seen_local,
                    );
                    append_form_keys_from_array_subrecord(
                        plugin,
                        &locator,
                        cell,
                        "XLCN",
                        "LCTN",
                        &mut gather.location_keys,
                        &mut location_seen_local,
                    );
                    gather.children = collect_cell_child_keys(
                        plugin,
                        child_groups_by_cell.get(&cell.form_id).copied(),
                        &locator,
                        &mut gather.static_base_keys,
                        &mut static_base_seen_local,
                        &mut gather.leveled_base_entry_keys,
                        &mut leveled_seen_local,
                        &mut gather.linked_ref_keyword_keys,
                        &mut keyword_seen_local,
                        &mut gather.layer_keys,
                        &mut layer_seen_local,
                    );
                    gather
                })
                .collect()
        };
        match worker_count {
            Some(workers) => rayon::ThreadPoolBuilder::new()
                .num_threads(workers.max(1))
                .build()
                .map_err(|err| PyRuntimeError::new_err(err.to_string()))?
                .install(gather_cells),
            None => gather_cells(),
        }
    };

    // Pass 2 (serial, cells order): replay every gathered stream against the
    // global seen-sets — byte-for-byte the legacy loop's effects.
    for gather in gathers {
        if cell_seen.insert(gather.cell_key.clone()) {
            payload.cell_form_keys.push(gather.cell_key.clone());
        }
        replay_first_occurrences(
            gather.region_keys,
            &mut payload.region_form_keys,
            &mut region_seen,
        );
        replay_first_occurrences(
            gather.location_keys,
            &mut payload.location_form_keys,
            &mut location_seen,
        );
        payload.cell_grids.insert(
            gather.cell_key.clone(),
            CellGridPayload {
                x: gather.grid_x,
                y: gather.grid_y,
            },
        );
        replay_first_occurrences(
            gather.static_base_keys,
            &mut payload.static_base_form_keys,
            &mut static_base_seen,
        );
        replay_first_occurrences(
            gather.leveled_base_entry_keys,
            &mut payload.leveled_base_entry_form_keys,
            &mut leveled_base_entry_seen,
        );
        replay_first_occurrences(
            gather.linked_ref_keyword_keys,
            &mut payload.linked_ref_keyword_form_keys,
            &mut linked_ref_keyword_seen,
        );
        replay_first_occurrences(
            gather.layer_keys,
            &mut payload.layer_form_keys,
            &mut layer_seen,
        );
        append_unique(
            &mut payload.placed_form_keys,
            &gather.children.persistent,
            &mut placed_seen,
        );
        append_unique(
            &mut payload.placed_form_keys,
            &gather.children.temporary,
            &mut placed_seen,
        );
        payload
            .cell_children
            .insert(gather.cell_key, gather.children);
    }

    if include_worldspace_persistent_cell {
        let world_key = render_form_key(plugin, world_record.form_id);
        let top_cell = world_children.children.iter().find_map(|item| {
            let ParsedItem::Record(record) = item else {
                return None;
            };
            (record.signature.as_str() == "CELL").then_some(record)
        });
        if let Some(top_cell) = top_cell {
            let children = collect_cell_child_keys(
                plugin,
                child_groups_by_cell.get(&top_cell.form_id).copied(),
                &locator,
                &mut payload.static_base_form_keys,
                &mut static_base_seen,
                &mut payload.leveled_base_entry_form_keys,
                &mut leveled_base_entry_seen,
                &mut payload.linked_ref_keyword_form_keys,
                &mut linked_ref_keyword_seen,
                &mut payload.layer_form_keys,
                &mut layer_seen,
            );
            append_unique(
                &mut payload.placed_form_keys,
                &children.persistent,
                &mut placed_seen,
            );
            append_unique(
                &mut payload.placed_form_keys,
                &children.temporary,
                &mut placed_seen,
            );
            payload.cell_children.insert(world_key.clone(), children);
            payload.worldspace_persistent_children_key = Some(world_key);
        }
    }

    append_region_data_dependency_keys(
        plugin,
        &locator,
        &payload.region_form_keys,
        &mut payload.region_data_form_keys,
        &mut region_data_seen,
    );
    append_form_keys_from_array_subrecord(
        plugin,
        &locator,
        world_record,
        "XLCN",
        "LCTN",
        &mut payload.location_form_keys,
        &mut location_seen,
    );
    append_lctn_world_cell_location_keys(
        plugin,
        &locator,
        world_record.form_id,
        &payload.cell_grids,
        &mut payload.location_form_keys,
        &mut location_seen,
        &mut payload.warnings,
    );
    append_location_data_dependency_keys(
        plugin,
        &locator,
        &payload.location_form_keys,
        &mut payload.location_data_form_keys,
        &mut location_data_seen,
    );
    append_worldspace_data_dependency_keys(
        plugin,
        &locator,
        world_record,
        &mut payload.worldspace_data_form_keys,
        &mut worldspace_data_seen,
    );
    let mut audio_roots = Vec::new();
    audio_roots.extend(payload.static_base_form_keys.iter().cloned());
    audio_roots.extend(payload.region_form_keys.iter().cloned());
    audio_roots.extend(payload.region_data_form_keys.iter().cloned());
    audio_roots.extend(payload.location_form_keys.iter().cloned());
    audio_roots.extend(payload.location_data_form_keys.iter().cloned());
    audio_roots.extend(payload.worldspace_data_form_keys.iter().cloned());
    append_audio_data_dependency_keys(
        plugin,
        &locator,
        &audio_roots,
        &mut payload.audio_data_form_keys,
        &mut audio_data_seen,
    );

    payload.timing.insert(
        "cell_traversal_ms".to_string(),
        traversal_start.elapsed().as_millis(),
    );
    payload
        .timing
        .insert("total_ms".to_string(), total_start.elapsed().as_millis());
    Ok(payload)
}

pub fn plugin_handle_collect_worldspace_terrain_ids_json(
    handle_id: u64,
    worldspace_editor_id: &str,
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
) -> Result<String, String> {
    let total_start = Instant::now();
    let mut payload = WorldspaceTerrainIdsPayload::default();
    let store = plugin_handle_store_ref()
        .lock()
        .map_err(|e| format!("plugin handle store lock poisoned: {e}"))?;
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| format!("unknown plugin handle: {handle_id}"))?;
    let plugin = &slot.parsed;

    let lookup_start = Instant::now();
    let Some(wrld_group) = top_group(plugin, "WRLD") else {
        payload
            .warnings
            .push(format!("WRLD top group not found: {worldspace_editor_id}"));
        payload.timing.insert(
            "world_lookup_ms".to_string(),
            lookup_start.elapsed().as_millis(),
        );
        payload
            .timing
            .insert("total_ms".to_string(), total_start.elapsed().as_millis());
        return serde_json::to_string(&payload)
            .map_err(|err| format!("failed to encode terrain ids: {err}"));
    };
    let (world_record, warnings) = find_world(plugin, worldspace_editor_id);
    payload.warnings.extend(warnings);
    let Some(world_record) = world_record else {
        payload.timing.insert(
            "world_lookup_ms".to_string(),
            lookup_start.elapsed().as_millis(),
        );
        payload
            .timing
            .insert("total_ms".to_string(), total_start.elapsed().as_millis());
        return serde_json::to_string(&payload)
            .map_err(|err| format!("failed to encode terrain ids: {err}"));
    };
    payload.world_form_id = Some(world_record.form_id & 0x00FF_FFFF);
    payload.world_editor_id = editor_id(world_record);
    let Some(world_children) = find_world_children_group(wrld_group, world_record.form_id) else {
        payload.warnings.push(format!(
            "world children group not found: {worldspace_editor_id}"
        ));
        payload.timing.insert(
            "world_lookup_ms".to_string(),
            lookup_start.elapsed().as_millis(),
        );
        payload
            .timing
            .insert("total_ms".to_string(), total_start.elapsed().as_millis());
        return serde_json::to_string(&payload)
            .map_err(|err| format!("failed to encode terrain ids: {err}"));
    };
    payload.timing.insert(
        "world_lookup_ms".to_string(),
        lookup_start.elapsed().as_millis(),
    );

    let traversal_start = Instant::now();
    // A whole-worldspace regen passes the full-extent sentinel (max == -1). The
    // terrain emit resolves it to the real BTD header bounds via
    // resolve_full_extent_sentinel, but this collector runs earlier and has no
    // header — so honor the same sentinel here. Applying inside_bounds literally
    // would evaluate `min <= x <= -1` (an empty range) and drop every cell,
    // collecting zero source IDs and preserving no exterior CELL FormID/EDID.
    let full_extent = max_x == -1 && max_y == -1;
    let persistent_cell_id = direct_world_persistent_cell_id(world_children);
    let mut child_groups_by_cell = BTreeMap::new();
    collect_cell_child_groups(world_children, &mut child_groups_by_cell);
    let mut cells = Vec::new();
    collect_group_records(world_children, "CELL", &mut cells);
    for cell in cells {
        if persistent_cell_id == Some(cell.form_id) {
            continue;
        }
        let Some((grid_x, grid_y)) = cell_grid(cell) else {
            continue;
        };
        if !full_extent && !inside_bounds(grid_x, grid_y, min_x, min_y, max_x, max_y) {
            continue;
        }
        let land_form_id = child_groups_by_cell
            .get(&cell.form_id)
            .and_then(|child_group| {
                let mut lands = Vec::new();
                collect_group_records(child_group, "LAND", &mut lands);
                lands.first().map(|land| land.form_id & 0x00FF_FFFF)
            });
        payload.cells.push(WorldspaceTerrainCellIds {
            x: grid_x,
            y: grid_y,
            cell_form_id: cell.form_id & 0x00FF_FFFF,
            cell_editor_id: editor_id(cell),
            land_form_id,
        });
    }
    payload.timing.insert(
        "cell_traversal_ms".to_string(),
        traversal_start.elapsed().as_millis(),
    );
    payload
        .timing
        .insert("total_ms".to_string(), total_start.elapsed().as_millis());
    serde_json::to_string(&payload).map_err(|err| format!("failed to encode terrain ids: {err}"))
}

/// Result of grafting reused terrain + navmesh from a prior FO4 output ESM.
#[derive(Default)]
pub struct GraftTerrainReport {
    pub cells: usize,
    pub lands: usize,
    pub navms: usize,
    pub txst: usize,
    pub ltex: usize,
    pub gras: usize,
    /// All grafted OWN object-ids (CELL/LAND/NAVM/TXST/LTEX/GRAS), masked to 24 bits.
    /// The conversion `graft_terrain` phase reserves these in the run mapper so no
    /// later phase re-allocates a grafted id.
    pub object_ids: Vec<u32>,
    pub warnings: Vec<String>,
}

/// Recursively clone a worldspace group keeping only terrain records (CELL/LAND/
/// NAVM) and structural group shells; placed children (REFR/ACHR/…) and the
/// per-cell PERSISTENT group are dropped so the normal FO76→FO4 record phases
/// repopulate them. Empty group shells are pruned. Counts + object-ids accumulate
/// into `report`.
fn clone_terrain_only_group(group: &ParsedGroup, report: &mut GraftTerrainReport) -> ParsedGroup {
    let mut out = ParsedGroup {
        label: group.label,
        group_type: group.group_type,
        tail: group.tail.clone(),
        children: Vec::new(),
    };
    for child in &group.children {
        match child {
            ParsedItem::Record(record) => match record.signature.as_str() {
                "CELL" => {
                    report.cells += 1;
                    report.object_ids.push(record.form_id & 0x00FF_FFFF);
                    out.children.push(ParsedItem::Record(record.clone()));
                }
                "LAND" => {
                    report.lands += 1;
                    report.object_ids.push(record.form_id & 0x00FF_FFFF);
                    out.children.push(ParsedItem::Record(record.clone()));
                }
                "NAVM" => {
                    report.navms += 1;
                    report.object_ids.push(record.form_id & 0x00FF_FFFF);
                    out.children.push(ParsedItem::Record(record.clone()));
                }
                _ => {}
            },
            ParsedItem::Group(child_group) => {
                if child_group.group_type == PERSISTENT_GROUP {
                    continue;
                }
                let cloned = clone_terrain_only_group(child_group, report);
                if !cloned.children.is_empty() {
                    out.children.push(ParsedItem::Group(cloned));
                }
            }
        }
    }
    out
}

/// Graft exterior terrain (CELL shells + LAND), navmesh (NAVM), and the
/// terrain-texture records (TXST/LTEX/GRAS) from a prior FO4 output ESM
/// (`cache_handle_id`) into a freshly-built FO4 target plugin (`target_handle_id`),
/// preserving FormIDs. Both plugins are FO4 with the same master list, so this is a
/// structural clone — NOT a FO76→FO4 conversion. Placed children are stripped from
/// each cell child group so the normal record phases repopulate them. Backs
/// `regen.py --re-use-land`.
pub fn graft_terrain_navmesh_from_handle(
    cache_handle_id: u64,
    target_handle_id: u64,
) -> Result<GraftTerrainReport, String> {
    let mut report = GraftTerrainReport::default();
    let mut store = plugin_handle_store_ref()
        .lock()
        .map_err(|e| format!("plugin handle store lock poisoned: {e}"))?;

    // Per-worldspace cloned exterior block groups, keyed by worldspace EditorID
    // (matched to the target by EditorID, since both are FO4 with the same ids).
    struct WorldBlocks {
        editor_id: String,
        blocks: Vec<ParsedItem>,
    }

    // ── 1. Extract from cache (scoped immutable borrow → owned clones) ──────
    let (worlds, cache_terrain_records, cache_masters): (
        Vec<WorldBlocks>,
        Vec<ParsedRecord>,
        Vec<String>,
    ) = {
        let cache = store
            .get(&cache_handle_id)
            .ok_or_else(|| format!("unknown cache plugin handle: {cache_handle_id}"))?;
        let plugin = &cache.parsed;
        let wrld_group = top_group(plugin, "WRLD")
            .ok_or_else(|| "cache: WRLD top group not found".to_string())?;

        // For every worldspace, clone its exterior cell-block groups (type 4),
        // filtering each to terrain records only. Per-cell persistent cells (a
        // direct CELL record + type-6 group under world_children) live outside the
        // block groups and are naturally skipped, as is the whole interior CELL
        // top group (not under WRLD).
        let mut worlds: Vec<WorldBlocks> = Vec::new();
        for item in &wrld_group.children {
            let ParsedItem::Record(world_record) = item else {
                continue;
            };
            if world_record.signature.as_str() != "WRLD" {
                continue;
            }
            let Some(world_children) = find_world_children_group(wrld_group, world_record.form_id)
            else {
                continue;
            };
            let mut blocks: Vec<ParsedItem> = Vec::new();
            for child in &world_children.children {
                if let ParsedItem::Group(group) = child {
                    if group.group_type == EXTERIOR_CELL_BLOCK {
                        blocks.push(ParsedItem::Group(clone_terrain_only_group(
                            group,
                            &mut report,
                        )));
                    }
                }
            }
            if !blocks.is_empty() {
                worlds.push(WorldBlocks {
                    editor_id: editor_id(world_record),
                    blocks,
                });
            }
        }

        // Top-level terrain-texture records (added/deduped against the target below).
        let mut cache_terrain_records: Vec<ParsedRecord> = Vec::new();
        for sig in ["TXST", "LTEX", "GRAS"] {
            if let Some(group) = top_group(plugin, sig) {
                let mut recs = Vec::new();
                collect_group_records(group, sig, &mut recs);
                cache_terrain_records.extend(recs.into_iter().cloned());
            }
        }
        (worlds, cache_terrain_records, plugin.header.masters.clone())
    };

    // ── 2. Insert into target (mutable borrow) ─────────────────────────────
    let target = store
        .get_mut(&target_handle_id)
        .ok_or_else(|| format!("unknown target plugin handle: {target_handle_id}"))?;
    if target.parsed.header.masters != cache_masters {
        report.warnings.push(
            "cache/target master lists differ; grafted terrain references may be off (own \
             LAND/NAVM/CELL refs are unaffected)"
                .to_string(),
        );
    }
    let header_size = target.parsed.header_size;
    let game = target.parsed.game.clone();

    // Existing top-level terrain-texture ids — skip grafting any the target already
    // has (the translator may emit non-terrain TXST); only terrain records are added.
    let mut existing_texture_ids: BTreeSet<u32> = BTreeSet::new();
    for sig in ["TXST", "LTEX", "GRAS"] {
        if let Some(group) = top_group(&target.parsed, sig) {
            let mut recs = Vec::new();
            collect_group_records(group, sig, &mut recs);
            existing_texture_ids.extend(recs.iter().map(|r| r.form_id));
        }
    }

    for world in worlds {
        let world_form_id = {
            let (world_record, warnings) = find_world(&target.parsed, &world.editor_id);
            report.warnings.extend(warnings);
            match world_record {
                Some(record) => record.form_id,
                None => {
                    report.warnings.push(format!(
                        "target: worldspace not found, terrain graft skipped: {}",
                        world.editor_id
                    ));
                    continue;
                }
            }
        };
        let wrld_group = top_group_mut(&mut target.parsed, "WRLD")
            .ok_or_else(|| "target: WRLD top group not found".to_string())?;
        let world_children =
            ensure_world_children_group(&mut wrld_group.children, world_form_id, header_size);
        let new_labels: BTreeSet<[u8; 4]> = world
            .blocks
            .iter()
            .filter_map(|item| match item {
                ParsedItem::Group(g) => Some(g.label),
                _ => None,
            })
            .collect();
        world_children.children.retain(|item| {
            !matches!(item, ParsedItem::Group(g)
                if g.group_type == EXTERIOR_CELL_BLOCK && new_labels.contains(&g.label))
        });
        world_children.children.extend(world.blocks);
    }

    for record in cache_terrain_records {
        if existing_texture_ids.contains(&record.form_id) {
            continue;
        }
        match record.signature.as_str() {
            "TXST" => report.txst += 1,
            "LTEX" => report.ltex += 1,
            "GRAS" => report.gras += 1,
            _ => {}
        }
        report.object_ids.push(record.form_id & 0x00FF_FFFF);
        ensure_top_group_and_add(
            &mut target.parsed.root_items,
            record,
            header_size,
            game.as_deref(),
        );
    }

    // Grafted records bypass the run mapper, so raise the on-disk object-id
    // counter past every grafted id — otherwise build_esp/CK could re-hand a
    // grafted id to a new record.
    if let Some(max_local) = report.object_ids.iter().copied().max() {
        target.parsed.header.next_object_id =
            target.parsed.header.next_object_id.max(max_local + 1);
    }

    target.record_count_cache = None;
    target.invalidate_index_sections();
    Ok(report)
}

pub(crate) fn plugin_handle_insert_cell_slice_children_json(
    target_handle_id: u64,
    children_by_target_cell_json: &str,
) -> PyResult<String> {
    let total_start = Instant::now();
    let children_by_target_cell: BTreeMap<String, CellChildrenPayload> =
        serde_json::from_str(children_by_target_cell_json).map_err(|err| {
            PyValueError::new_err(format!("invalid cell children payload: {err}"))
        })?;
    let mut payload = CellSliceInsertPayload::default();

    let mut store = plugin_handle_store_ref().lock().unwrap();
    let slot = store
        .get_mut(&target_handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {target_handle_id}")))?;
    // Render form keys from the lightweight master context instead of cloning
    // the whole plugin tree (millions of records) just to read plugin_name +
    // masters, which is all `take_matching_record_for_form_key` needs.
    let own_name: Arc<str> = Arc::from(slot.parsed.plugin_name.as_str());
    let masters = slot.parsed.header.masters.clone();
    let header_size = slot.parsed.header_size;
    payload.cell_region_refs_rewritten =
        rewrite_cell_region_refs_to_local_records(&mut slot.parsed);

    for (target_cell_key, sections) in children_by_target_cell {
        let Some(cell_record) = ({
            let locator = build_locator_section(&slot.parsed);
            locator_entry_by_form_key(&locator, target_cell_key.as_str())
                .and_then(|entry| locator.record(&slot.parsed, entry))
                .cloned()
        }) else {
            payload
                .warnings
                .push(format!("target cell not found: {target_cell_key}"));
            continue;
        };
        let mut records_for_sections: Vec<(i32, ParsedRecord)> = Vec::new();
        for (group_type, child_keys) in [
            (PERSISTENT_GROUP, sections.persistent),
            (TEMPORARY_GROUP, sections.temporary),
        ] {
            for target_child_key in child_keys {
                match take_matching_record_for_form_key(
                    &mut slot.parsed.root_items,
                    &own_name,
                    &masters,
                    target_child_key.as_str(),
                ) {
                    Some(record) => records_for_sections.push((group_type, record)),
                    None => payload.skipped_children.push(target_child_key),
                }
            }
        }
        if records_for_sections.is_empty() {
            continue;
        }

        let raw_cell_id = cell_record.form_id;
        let Some(cell_child_group) =
            find_cell_child_group_mut(&mut slot.parsed.root_items, raw_cell_id)
        else {
            payload.warnings.push(format!(
                "target CELL child group not found: {target_cell_key}"
            ));
            continue;
        };
        payload.cells_touched += 1;

        for (group_type, record) in records_for_sections {
            let section_group =
                ensure_child_group_mut(cell_child_group, group_type, raw_cell_id, header_size);
            section_group.children.push(ParsedItem::Record(record));
            payload.children_inserted += 1;
        }
    }

    slot.record_count_cache = None;
    slot.invalidate_index_sections();
    payload
        .timing
        .insert("total_ms".to_string(), total_start.elapsed().as_millis());
    serde_json::to_string(&payload)
        .map_err(|err| PyValueError::new_err(format!("failed to encode insert result: {err}")))
}

pub fn plugin_handle_sync_cell_locations_from_lctn_json(handle_id: u64) -> PyResult<String> {
    let total_start = Instant::now();
    let mut store = plugin_handle_store_ref().lock().unwrap();
    let slot = store
        .get_mut(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;

    let mut payload = sync_cell_locations_from_lctn_world_cells(&mut slot.parsed);
    if payload.cells_changed > 0 {
        slot.record_count_cache = None;
        slot.invalidate_index_sections();
    }
    payload
        .timing
        .insert("total_ms".to_string(), total_start.elapsed().as_millis());
    serde_json::to_string(&payload).map_err(|err| {
        PyValueError::new_err(format!("failed to encode cell location sync result: {err}"))
    })
}

pub fn plugin_handle_sync_cell_regions_from_source_json(
    source_handle_id: u64,
    target_handle_id: u64,
    source_worldspace_editor_id: &str,
    target_worldspace_editor_id: &str,
) -> PyResult<String> {
    let total_start = Instant::now();
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

    let payload = {
        let source_slot = store.get(&source_handle_id).ok_or_else(|| {
            PyKeyError::new_err(format!("unknown source plugin handle: {source_handle_id}"))
        })?;
        sync_cell_regions_from_source_worldspace(
            &source_slot.parsed,
            &mut target_slot.parsed,
            source_worldspace_editor_id,
            target_worldspace_editor_id,
        )
    };

    let mut payload = payload;
    if payload.cells_changed > 0 {
        target_slot.record_count_cache = None;
        target_slot.invalidate_index_sections();
    }
    store.insert(target_handle_id, target_slot);
    payload
        .timing
        .insert("total_ms".to_string(), total_start.elapsed().as_millis());
    serde_json::to_string(&payload).map_err(|err| {
        PyValueError::new_err(format!("failed to encode cell region sync result: {err}"))
    })
}

// FO4/FO76 exterior CELL.MHDT is a 4-byte float offset + a 32x32 grid of
// max-height bytes (identical layout in both games). FO4 worldspace cells are
// synthesized bare by terrain, so the source per-cell max-height table is lost
// unless carried. Missing it makes flying actors (vertibirds) path into terrain.
const CELL_MHDT_LEN: usize = 1028;

#[derive(Default, Serialize)]
struct CellMaxHeightSyncPayload {
    source_cells_indexed: usize,
    malformed_source_cells: usize,
    target_cells_seen: usize,
    cells_changed: usize,
    cells_retagged: usize,
    cells_already_tagged: usize,
    unmatched_target_cells: usize,
    warnings: Vec<String>,
    timing: BTreeMap<String, u128>,
}

fn collect_cell_max_heights_by_grid(
    source: &ParsedPlugin,
    worldspace_editor_id: &str,
    payload: &mut CellMaxHeightSyncPayload,
) -> BTreeMap<(i32, i32), Bytes> {
    let mut by_grid = BTreeMap::new();
    let Some(wrld_group) = top_group(source, "WRLD") else {
        payload.warnings.push(format!(
            "source WRLD top group not found: {worldspace_editor_id}"
        ));
        return by_grid;
    };
    let (world_record, warnings) = find_world(source, worldspace_editor_id);
    payload.warnings.extend(warnings);
    let Some(world_record) = world_record else {
        return by_grid;
    };
    let Some(world_children) = find_world_children_group(wrld_group, world_record.form_id) else {
        payload.warnings.push(format!(
            "source world children group not found: {worldspace_editor_id}"
        ));
        return by_grid;
    };

    let persistent_cell_id = direct_world_persistent_cell_id(world_children);
    let mut cells = Vec::new();
    collect_group_records(world_children, "CELL", &mut cells);
    for cell in cells {
        if persistent_cell_id == Some(cell.form_id) {
            continue;
        }
        let Some(grid) = cell_grid(cell) else {
            continue;
        };
        let Some(data) = subrecord_data(cell, "MHDT") else {
            continue;
        };
        if data.len() != CELL_MHDT_LEN {
            payload.malformed_source_cells += 1;
            continue;
        }
        payload.source_cells_indexed += 1;
        by_grid.entry(grid).or_insert(data);
    }
    by_grid
}

fn set_or_replace_cell_max_height(record: &mut ParsedRecord, mhdt: &Bytes) -> CellRegionMutation {
    if record.subrecords.is_empty() {
        record.subrecords = effective_subrecords_for_record(record).into_owned();
    }

    let mut old_data = None;
    record.subrecords.retain(|subrecord| {
        if subrecord.signature.as_str() == "MHDT" {
            if old_data.is_none() {
                old_data = Some(subrecord.data.to_vec());
            }
            false
        } else {
            true
        }
    });

    // FO4 CELL schema order: ... XCLC, XCLL, TVDT, MHDT, XCGD, LTMP ... Insert
    // MHDT after the last of the fields that precede it, else after DATA/EDID.
    let insert_at = record
        .subrecords
        .iter()
        .rposition(|subrecord| {
            matches!(
                subrecord.signature.as_str(),
                "TVDT" | "XCLL" | "XCLC" | "DATA"
            )
        })
        .or_else(|| {
            record
                .subrecords
                .iter()
                .position(|subrecord| subrecord.signature.as_str() == "EDID")
        })
        .map(|index| index + 1)
        .unwrap_or(0);
    record.subrecords.insert(
        insert_at,
        ParsedSubrecord {
            signature: "MHDT".into(),
            data: mhdt.clone(),
            semantic_type: None,
        },
    );

    let mutation = if old_data.as_deref() == Some(mhdt.as_ref()) {
        CellRegionMutation::AlreadyTagged
    } else if old_data.is_some() {
        CellRegionMutation::Replaced
    } else {
        CellRegionMutation::Inserted
    };
    if !matches!(mutation, CellRegionMutation::AlreadyTagged) {
        record.raw_payload = None;
    }
    mutation
}

fn sync_cell_max_heights_in_items(
    items: &mut [ParsedItem],
    source_heights: &BTreeMap<(i32, i32), Bytes>,
    payload: &mut CellMaxHeightSyncPayload,
) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "CELL" => {
                let Some(grid) = cell_grid(record) else {
                    continue;
                };
                payload.target_cells_seen += 1;
                let Some(data) = source_heights.get(&grid) else {
                    payload.unmatched_target_cells += 1;
                    continue;
                };
                match set_or_replace_cell_max_height(record, data) {
                    CellRegionMutation::Inserted => payload.cells_changed += 1,
                    CellRegionMutation::Replaced => {
                        payload.cells_changed += 1;
                        payload.cells_retagged += 1;
                    }
                    CellRegionMutation::AlreadyTagged => payload.cells_already_tagged += 1,
                }
            }
            ParsedItem::Group(group) => {
                sync_cell_max_heights_in_items(&mut group.children, source_heights, payload);
            }
            _ => {}
        }
    }
}

fn sync_cell_max_height_from_source_worldspace(
    source: &ParsedPlugin,
    target: &mut ParsedPlugin,
    source_worldspace_editor_id: &str,
    target_worldspace_editor_id: &str,
) -> CellMaxHeightSyncPayload {
    let mut payload = CellMaxHeightSyncPayload::default();
    let source_heights =
        collect_cell_max_heights_by_grid(source, source_worldspace_editor_id, &mut payload);
    if source_heights.is_empty() {
        return payload;
    }

    let target_world_form_id = {
        let (world_record, warnings) = find_world(target, target_worldspace_editor_id);
        payload.warnings.extend(warnings);
        let Some(world_record) = world_record else {
            return payload;
        };
        world_record.form_id
    };
    let Some(wrld_group) = top_group_mut(target, "WRLD") else {
        payload.warnings.push(format!(
            "target WRLD top group not found: {target_worldspace_editor_id}"
        ));
        return payload;
    };
    let Some(world_children) = find_world_children_group_mut(wrld_group, target_world_form_id)
    else {
        payload.warnings.push(format!(
            "target world children group not found: {target_worldspace_editor_id}"
        ));
        return payload;
    };

    sync_cell_max_heights_in_items(&mut world_children.children, &source_heights, &mut payload);
    payload
}

pub(crate) fn plugin_handle_sync_cell_max_height_from_source_json(
    source_handle_id: u64,
    target_handle_id: u64,
    source_worldspace_editor_id: &str,
    target_worldspace_editor_id: &str,
) -> PyResult<String> {
    let total_start = Instant::now();
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

    let mut payload = {
        let source_slot = store.get(&source_handle_id).ok_or_else(|| {
            PyKeyError::new_err(format!("unknown source plugin handle: {source_handle_id}"))
        })?;
        sync_cell_max_height_from_source_worldspace(
            &source_slot.parsed,
            &mut target_slot.parsed,
            source_worldspace_editor_id,
            target_worldspace_editor_id,
        )
    };

    if payload.cells_changed > 0 {
        target_slot.record_count_cache = None;
        target_slot.invalidate_index_sections();
    }
    store.insert(target_handle_id, target_slot);
    payload
        .timing
        .insert("total_ms".to_string(), total_start.elapsed().as_millis());
    serde_json::to_string(&payload).map_err(|err| {
        PyValueError::new_err(format!(
            "failed to encode cell max-height sync result: {err}"
        ))
    })
}

pub(crate) fn plugin_handle_copy_cell_slice_children_json(
    source_handle_id: u64,
    target_handle_id: u64,
    children_by_target_cell_json: &str,
    offset_x: f32,
    offset_y: f32,
    offset_z: f32,
    form_key_map_json: Option<&str>,
) -> PyResult<String> {
    let children_by_target_cell: BTreeMap<String, CellChildrenPayload> =
        serde_json::from_str(children_by_target_cell_json).map_err(|err| {
            PyValueError::new_err(format!("invalid cell children payload: {err}"))
        })?;
    let form_key_map = parse_form_key_map_json(form_key_map_json)?;
    let payload = copy_cell_slice_children_payload(
        source_handle_id,
        target_handle_id,
        children_by_target_cell,
        (offset_x, offset_y, offset_z),
        form_key_map,
    )?;
    serde_json::to_string(&payload)
        .map_err(|err| PyValueError::new_err(format!("failed to encode copy result: {err}")))
}

/// Struct-based core of the placed-child copy, for native callers. The
/// mutation kernels (`prepare_source_children_for_target_cells` /
/// `insert_prepared_children_into_target_cells`) run here unchanged — this is
/// the production copy path; only the JSON marshalling was lifted out.
pub fn copy_cell_slice_children_payload(
    source_handle_id: u64,
    target_handle_id: u64,
    children_by_target_cell: BTreeMap<String, CellChildrenPayload>,
    offset: (f32, f32, f32),
    form_key_map: BTreeMap<String, String>,
) -> PyResult<CellSliceInsertPayload> {
    let total_start = Instant::now();
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

    let result = {
        let source_slot = store.get(&source_handle_id).ok_or_else(|| {
            PyKeyError::new_err(format!("unknown source plugin handle: {source_handle_id}"))
        })?;
        let source_plugin = &source_slot.parsed;
        let source_locator = build_locator_section(source_plugin);
        let source_own_index = (source_plugin.header.masters.len() & 0xFF) as u8;
        let target_own_prefix = local_form_prefix(&target_slot.parsed);
        let target_game = target_slot.parsed.game.clone();
        let target = TargetFormIdContext {
            plugin_name: target_slot.parsed.plugin_name.clone(),
            masters: target_slot.parsed.header.masters.clone(),
            own_prefix: target_own_prefix,
        };
        let mut target_existing_form_ids = BTreeSet::new();
        collect_record_form_ids(
            &target_slot.parsed.root_items,
            &mut target_existing_form_ids,
        );
        let target_locator = build_locator_section(&target_slot.parsed);
        let header_size = target_slot.parsed.header_size;
        let mut target_cell_by_grid = BTreeMap::new();
        collect_target_cell_by_grid(&target_slot.parsed.root_items, &mut target_cell_by_grid);
        let mut payload = CellSliceInsertPayload::default();
        payload.cell_region_refs_rewritten =
            rewrite_cell_region_refs_to_local_records(&mut target_slot.parsed);
        let mut form_key_map = form_key_map;
        payload.child_form_ids_reallocated = reserve_copied_child_form_ids(
            &children_by_target_cell,
            source_plugin,
            &target,
            &mut target_existing_form_ids,
            &mut form_key_map,
        );

        let ctx = CopyCellChildrenContext {
            source_plugin,
            source_locator: &source_locator,
            source_own_index,
            target: &target,
            target_game: target_game.as_deref(),
            target_locator: &target_locator,
            target_existing_form_ids: &target_existing_form_ids,
            target_cell_by_grid: &target_cell_by_grid,
            form_key_map: &form_key_map,
            header_size,
            offset,
        };
        let mut remaining =
            prepare_source_children_for_target_cells(children_by_target_cell, &ctx, &mut payload);
        insert_prepared_children_into_target_cells(
            &mut target_slot.parsed.root_items,
            &mut remaining,
            &ctx,
            &mut payload,
        );
        for raw_cell_id in remaining.keys() {
            payload
                .warnings
                .push(format!("target cell not found: {:08X}", raw_cell_id));
        }
        payload
            .timing
            .insert("total_ms".to_string(), total_start.elapsed().as_millis());
        Ok(payload)
    };

    target_slot.record_count_cache = None;
    target_slot.invalidate_index_sections();
    store.insert(target_handle_id, target_slot);
    result
}

/// Target-side (grid → CELL form key) map plus the target plugin name, for
/// native callers that pre-route source cells to target grid cells (the
/// conversion crate's projected-placed orchestrator). Pure read; no index
/// invalidation.
pub fn collect_cell_grid_form_keys(
    handle_id: u64,
) -> PyResult<(String, BTreeMap<(i32, i32), String>)> {
    let store = plugin_handle_store_ref().lock().unwrap();
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let plugin = &slot.parsed;
    let mut by_grid = BTreeMap::new();
    collect_target_cell_by_grid(&plugin.root_items, &mut by_grid);
    let mut out = BTreeMap::new();
    for (grid, form_id) in by_grid {
        out.insert(grid, render_form_key(plugin, form_id));
    }
    Ok((plugin.plugin_name.clone(), out))
}

/// Collect the form keys of every placed record (REFR/ACHR/PHZD) under a
/// CELL's Cell-Children(6) → Cell-Persistent(8) group, in source order.
fn collect_persistent_child_keys(
    source_plugin: &ParsedPlugin,
    persistent_cell_child_group: &ParsedGroup,
) -> Vec<String> {
    let mut keys = Vec::new();
    for item in &persistent_cell_child_group.children {
        let ParsedItem::Group(section) = item else {
            continue;
        };
        if section.group_type != PERSISTENT_GROUP {
            continue;
        }
        for child in &section.children {
            if let ParsedItem::Record(record) = child {
                if is_placed_child_signature(record.signature.as_str()) {
                    keys.push(render_form_key(source_plugin, record.form_id));
                }
            }
        }
    }
    keys
}

/// Collect the base (NAME) form keys of every placed record under a worldspace's
/// persistent cell — for ALL base record types, not just the placement-base
/// signatures the seed collector returns. The synthesis phase feeds these into
/// `conversion_run_form_key_map` so every base resolves to its translated objid
/// (including bases reallocated during translation), preventing `missing_base`
/// drops for refs whose base type is outside `PLACEMENT_BASE_SIGNATURES`
/// (IDLM/TERM/BOOK/TACT/NOTE/…).
pub(crate) fn plugin_handle_collect_worldspace_persistent_base_keys_json(
    source_handle_id: u64,
    worldspace_editor_id: &str,
) -> PyResult<String> {
    collect_worldspace_persistent_base_keys_json(source_handle_id, worldspace_editor_id, None)
}

pub(crate) fn plugin_handle_collect_worldspace_persistent_base_keys_in_bounds_json(
    source_handle_id: u64,
    worldspace_editor_id: &str,
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
) -> PyResult<String> {
    collect_worldspace_persistent_base_keys_json(
        source_handle_id,
        worldspace_editor_id,
        Some((min_x, min_y, max_x, max_y)),
    )
}

fn collect_worldspace_persistent_base_keys_json(
    source_handle_id: u64,
    worldspace_editor_id: &str,
    bounds: Option<(i32, i32, i32, i32)>,
) -> PyResult<String> {
    let keys =
        collect_worldspace_persistent_base_keys(source_handle_id, worldspace_editor_id, bounds)?;
    serde_json::to_string(&keys)
        .map_err(|e| PyValueError::new_err(format!("encode base keys: {e}")))
}

/// Vec-returning core of the persistent-base-key collection, for native
/// callers (the conversion crate's projected-placed orchestrator).
pub fn collect_worldspace_persistent_base_keys(
    source_handle_id: u64,
    worldspace_editor_id: &str,
    bounds: Option<(i32, i32, i32, i32)>,
) -> PyResult<Vec<String>> {
    let store = plugin_handle_store_ref().lock().unwrap();
    let slot = store.get(&source_handle_id).ok_or_else(|| {
        PyKeyError::new_err(format!("unknown source plugin handle: {source_handle_id}"))
    })?;
    let plugin = &slot.parsed;

    let mut keys: Vec<String> = Vec::new();
    let mut seen: BTreeSet<u32> = BTreeSet::new();

    let Some(wrld_group) = top_group(plugin, "WRLD") else {
        return Ok(keys);
    };
    let (world_record, _warnings) = find_world(plugin, worldspace_editor_id);
    let Some(world_record) = world_record else {
        return Ok(keys);
    };
    let Some(world_children) = find_world_children_group(wrld_group, world_record.form_id) else {
        return Ok(keys);
    };
    let Some(persistent_cell_id) = direct_world_persistent_cell_id(world_children) else {
        return Ok(keys);
    };
    let mut child_groups = BTreeMap::new();
    collect_cell_child_groups(world_children, &mut child_groups);
    if let Some(group) = child_groups.get(&persistent_cell_id) {
        for item in &group.children {
            let ParsedItem::Group(section) = item else {
                continue;
            };
            if section.group_type != PERSISTENT_GROUP {
                continue;
            }
            for child in &section.children {
                let ParsedItem::Record(record) = child else {
                    continue;
                };
                if !is_placed_child_signature(record.signature.as_str()) {
                    continue;
                }
                if let Some((min_x, min_y, max_x, max_y)) = bounds {
                    let Some((grid_x, grid_y)) = placed_record_cell_grid(record) else {
                        continue;
                    };
                    if !inside_bounds(grid_x, grid_y, min_x, min_y, max_x, max_y) {
                        continue;
                    }
                }
                let Some(name) = subrecord_data(record, "NAME") else {
                    continue;
                };
                if name.len() != 4 {
                    continue;
                }
                let base_raw = u32::from_le_bytes([name[0], name[1], name[2], name[3]]);
                if base_raw == 0 {
                    continue;
                }
                if seen.insert(base_raw) {
                    keys.push(render_form_key(plugin, base_raw));
                }
            }
        }
    }
    Ok(keys)
}

/// Build the FO4 worldspace-persistent CELL record from the FO76 source CELL.
///
/// The source FO76 CELL carries FO76-only subrecords (XILS/NAVH/CII0/CIDH/…) and
/// a 4-byte raw DATA flags field; FO4 expects a CELL whose subrecord set is a
/// subset of its schema and whose DATA is a 2-byte `uint16`. We schema-filter the
/// clone to drop everything FO4 rejects, then re-insert a correct 2-byte DATA
/// carrying the same flag value (the FO76 low 2 bytes — has_water etc. share bit
/// positions with FO4). The object id is preserved (the cell is a real record FO4
/// references by id); only the master/plugin prefix is rewritten to the target.
fn build_target_persistent_cell(
    source_cell: &ParsedRecord,
    target: &TargetFormIdContext,
    target_game: Option<&str>,
) -> ParsedRecord {
    let object_id = source_cell.form_id & 0x00FF_FFFF;
    let mut cell = source_cell.clone();
    cell.form_id = target.own_prefix | object_id;
    // 0x00040400 = compressed (0x40000) | persistent (0x400); the source already
    // carries this, but force it so a non-compressed source still flags persistent.
    cell.flags |= 0x0000_0400;
    // Materialize the subrecords BEFORE nulling raw_payload — the lazy/raw
    // fallback decodes from raw_payload, so nulling it first would yield an empty
    // subrecord list (losing DATA) for a record whose subrecords aren't eager.
    if cell.subrecords.is_empty() {
        cell.subrecords = effective_subrecords_for_record(&cell).into_owned();
    }
    cell.raw_payload = None;

    // Capture the source DATA flag value before the schema filter drops the
    // wrong-width 4-byte FO76 DATA.
    let source_data_flags: Option<u16> = cell
        .subrecords
        .iter()
        .find(|sub| sub.signature.as_str() == "DATA")
        .map(|sub| {
            let bytes = sub.data.as_ref();
            let lo = bytes.first().copied().unwrap_or(0);
            let hi = bytes.get(1).copied().unwrap_or(0);
            u16::from_le_bytes([lo, hi])
        });

    filter_record_to_target_schema(&mut cell, target_game);

    // Re-insert a FO4-correct 2-byte DATA (the filter dropped the 4-byte source
    // DATA). Place it after EDID/FULL, before XCLC, matching the FO4 CELL schema
    // order.
    if let Some(flags) = source_data_flags {
        let data = subrecord_with_data("DATA", flags.to_le_bytes().to_vec());
        let insert_at = cell
            .subrecords
            .iter()
            .position(|sub| !matches!(sub.signature.as_str(), "EDID" | "FULL"))
            .unwrap_or(cell.subrecords.len());
        cell.subrecords.insert(insert_at, data);
    }
    cell
}

fn subrecord_with_data(signature: &str, data: Vec<u8>) -> ParsedSubrecord {
    ParsedSubrecord {
        signature: SmolStr::new(signature),
        data: Bytes::from(data),
        semantic_type: None,
    }
}

/// Resolve localized-string subrecords (FULL/DESC/…) on a synthesized persistent
/// record to inline text, using the SOURCE string table.
///
/// Synthesize the FO4 worldspace-persistent CELL and route the source
/// worldspace-persistent refs (REFR/ACHR/PHZD) under it, converted FO76→FO4.
///
/// The translator skips top-level CELL/REFR/ACHR (they need group-nested
/// placement) and the terrain phase only synthesizes grid cells, so the FO4
/// output otherwise has no worldspace persistent cell at all — dropping ~130k
/// persistent refs and dangling every QUST ALFR / LCTN MNAM that targets them.
///
/// This emits the persistent CELL as the FIRST record under the target World
/// Children GRUP(1) (before exterior block GRUPs, matching base FO4), with an
/// empty Cell-Children(6) → Cell-Persistent(8) holding the converted refs. The
/// refs go through the same `source_record_for_target_child` conversion the grid
/// refs use (form-id map, base resolve, ref rewrite, schema filter) and keep
/// their 0x400 persistent record-header flag. Grid rebucketing is intentionally
/// bypassed — persistent refs belong to the worldspace persistent cell, not to
/// the grid cell their position happens to fall in.
pub(crate) fn plugin_handle_synthesize_worldspace_persistent_cell_json(
    source_handle_id: u64,
    target_handle_id: u64,
    worldspace_editor_id: &str,
    offset_x: f32,
    offset_y: f32,
    offset_z: f32,
    form_key_map_json: Option<&str>,
) -> PyResult<String> {
    let form_key_map = parse_form_key_map_json(form_key_map_json)?;
    let payload = synthesize_worldspace_persistent_cell_payload(
        source_handle_id,
        target_handle_id,
        worldspace_editor_id,
        (offset_x, offset_y, offset_z),
        form_key_map,
    )?;
    serde_json::to_string(&payload)
        .map_err(|err| PyValueError::new_err(format!("failed to encode synthesize result: {err}")))
}

/// Struct-based core of the worldspace-persistent-cell synthesis, for native
/// callers. The conversion kernel (`source_record_for_target_child`) runs here
/// unchanged — only the JSON marshalling was lifted out.
pub fn synthesize_worldspace_persistent_cell_payload(
    source_handle_id: u64,
    target_handle_id: u64,
    worldspace_editor_id: &str,
    offset: (f32, f32, f32),
    form_key_map: BTreeMap<String, String>,
) -> PyResult<SynthesizePersistentCellPayload> {
    let total_start = Instant::now();
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

    let result = {
        let source_slot = store.get(&source_handle_id).ok_or_else(|| {
            PyKeyError::new_err(format!("unknown source plugin handle: {source_handle_id}"))
        })?;
        let source_plugin = &source_slot.parsed;

        let mut payload = SynthesizePersistentCellPayload::default();

        // ── Locate the source worldspace persistent cell + its children. ──────
        let (source_cell, persistent_keys) = {
            let Some(wrld_group) = top_group(source_plugin, "WRLD") else {
                payload
                    .warnings
                    .push("source WRLD top group not found".to_string());
                return finish_synthesize(
                    payload,
                    target_handle_id,
                    target_slot,
                    store,
                    total_start,
                );
            };
            let (world_record, warnings) = find_world(source_plugin, worldspace_editor_id);
            payload.warnings.extend(warnings);
            let Some(world_record) = world_record else {
                return finish_synthesize(
                    payload,
                    target_handle_id,
                    target_slot,
                    store,
                    total_start,
                );
            };
            let Some(world_children) = find_world_children_group(wrld_group, world_record.form_id)
            else {
                payload.warnings.push(format!(
                    "source world children group not found: {worldspace_editor_id}"
                ));
                return finish_synthesize(
                    payload,
                    target_handle_id,
                    target_slot,
                    store,
                    total_start,
                );
            };
            let Some(persistent_cell_id) = direct_world_persistent_cell_id(world_children) else {
                payload.warnings.push(format!(
                    "source worldspace has no persistent cell: {worldspace_editor_id}"
                ));
                return finish_synthesize(
                    payload,
                    target_handle_id,
                    target_slot,
                    store,
                    total_start,
                );
            };
            let Some(source_cell) = world_children.children.iter().find_map(|item| match item {
                ParsedItem::Record(record)
                    if record.signature.as_str() == "CELL"
                        && record.form_id == persistent_cell_id =>
                {
                    Some(record.clone())
                }
                _ => None,
            }) else {
                // direct_world_persistent_cell_id returned an id but no matching
                // CELL record resolved — treat as no persistent cell rather than
                // aborting the whole conversion.
                payload.warnings.push(format!(
                    "source persistent cell id {persistent_cell_id:08X} did not resolve to a CELL record"
                ));
                return finish_synthesize(
                    payload,
                    target_handle_id,
                    target_slot,
                    store,
                    total_start,
                );
            };
            let mut child_groups = BTreeMap::new();
            collect_cell_child_groups(world_children, &mut child_groups);
            let persistent_keys = child_groups
                .get(&persistent_cell_id)
                .map(|group| collect_persistent_child_keys(source_plugin, group))
                .unwrap_or_default();
            (source_cell, persistent_keys)
        };

        // ── Conversion context (mirrors plugin_handle_copy_cell_slice_children). ──
        let source_locator = build_locator_section(source_plugin);
        let source_own_index = (source_plugin.header.masters.len() & 0xFF) as u8;
        let target_own_prefix = local_form_prefix(&target_slot.parsed);
        let target_game = target_slot.parsed.game.clone();
        let target = TargetFormIdContext {
            plugin_name: target_slot.parsed.plugin_name.clone(),
            masters: target_slot.parsed.header.masters.clone(),
            own_prefix: target_own_prefix,
        };
        let header_size = target_slot.parsed.header_size;

        // ── Synthesize the persistent CELL (accuracy: build to FO4 shape). ────
        let target_cell =
            build_target_persistent_cell(&source_cell, &target, target_game.as_deref());
        let target_cell_form_id = target_cell.form_id;
        payload.persistent_cell_form_key =
            render_form_key(&target_slot.parsed, target_cell_form_id);

        // Collision guard: the cell's object id must be free in the target (CELL
        // is in skip_records, so the mapper never allocated it). If somehow taken
        // by a non-CELL record, refuse rather than emit a duplicate object id.
        {
            let mut existing = BTreeSet::new();
            collect_record_form_ids(&target_slot.parsed.root_items, &mut existing);
            if existing.contains(&target_cell_form_id) {
                payload.warnings.push(format!(
                    "persistent cell object id collision (refusing to synthesize): {}",
                    payload.persistent_cell_form_key
                ));
                return finish_synthesize(
                    payload,
                    target_handle_id,
                    target_slot,
                    store,
                    total_start,
                );
            }
        }

        insert_persistent_cell_into_world_children(
            &mut target_slot.parsed.root_items,
            worldspace_editor_id,
            target_cell,
            header_size,
            &mut payload,
        );
        if !payload.cell_synthesized {
            return finish_synthesize(payload, target_handle_id, target_slot, store, total_start);
        }

        // ── Convert + route the persistent refs into Cell-Persistent(8). ──────
        let mut target_existing_form_ids = BTreeSet::new();
        collect_record_form_ids(
            &target_slot.parsed.root_items,
            &mut target_existing_form_ids,
        );
        let target_locator = build_locator_section(&target_slot.parsed);
        let mut form_key_map = form_key_map;
        // Reserve object ids for the routed children so own-plugin refs don't
        // collide (matches the grid-copy path's reservation).
        let mut children_payload: BTreeMap<String, CellChildrenPayload> = BTreeMap::new();
        children_payload.insert(
            payload.persistent_cell_form_key.clone(),
            CellChildrenPayload {
                persistent: persistent_keys.clone(),
                temporary: Vec::new(),
            },
        );
        payload.child_form_ids_reallocated = reserve_copied_child_form_ids(
            &children_payload,
            source_plugin,
            &target,
            &mut target_existing_form_ids,
            &mut form_key_map,
        );

        // Pass the SOURCE key — source_record_for_target_child looks the record
        // up in the SOURCE locator by this key's object id and remaps the
        // output form_id internally via form_key_map (preserve-by-default).
        // Passing a target-remapped key here would make the source lookup miss
        // for any ref whose objid was remapped, silently dropping it.
        //
        // Parallel convert over the frozen context (same kernel as the placed
        // copy); the serial fold below reproduces the legacy loop's
        // counters, converted order, and skip diagnostics in key order.
        let convert = |source_key: &String| {
            source_record_for_target_child(
                source_plugin,
                &source_locator,
                source_key.as_str(),
                source_own_index,
                &target,
                target_game.as_deref(),
                &target_locator,
                &target_existing_form_ids,
                &form_key_map,
                offset,
            )
        };
        let results: Vec<Result<(ParsedRecord, usize, usize, usize), &'static str>> =
            if persistent_keys.len() < PREPARE_PAR_THRESHOLD {
                persistent_keys.iter().map(convert).collect()
            } else {
                use rayon::prelude::*;
                persistent_keys.par_iter().map(convert).collect()
            };
        let mut converted = Vec::with_capacity(persistent_keys.len());
        for (source_key, result) in persistent_keys.iter().zip(results) {
            match result {
                Ok((
                    record,
                    mapped_form_refs,
                    leveled_bases_resolved,
                    schema_subrecords_dropped,
                )) => {
                    payload.mapped_form_refs += mapped_form_refs;
                    payload.leveled_bases_resolved += leveled_bases_resolved;
                    payload.schema_subrecords_dropped += schema_subrecords_dropped;
                    converted.push(record);
                }
                Err(reason) => {
                    payload.persistent_refs_skipped += 1;
                    *payload.skip_reasons.entry(reason.to_string()).or_insert(0) += 1;
                    // Surface the NAME base of every skipped ref so the dropped
                    // persistent refs (e.g. the missing MapMarkers, base 000010)
                    // can be identified by base. The histogram keys on
                    // (base FormKey, reason) and is unbounded by record count
                    // (one entry per distinct pair); the 200-cap sample carries
                    // the full per-ref detail.
                    let base_key =
                        skipped_ref_source_base_key(source_plugin, &source_locator, source_key);
                    *payload
                        .skip_base_histogram
                        .entry(format!("base={base_key}|reason={reason}"))
                        .or_insert(0) += 1;
                    if payload.skipped_children.len() < 200 {
                        payload
                            .skipped_children
                            .push(format!("{source_key}|base={base_key}|reason={reason}"));
                    }
                }
            }
        }
        payload.persistent_refs_converted = converted.len();

        route_converted_persistent_refs(
            &mut target_slot.parsed.root_items,
            target_cell_form_id,
            converted,
            header_size,
        );

        Ok(payload)
    };

    target_slot.record_count_cache = None;
    target_slot.invalidate_index_sections();
    store.insert(target_handle_id, target_slot);
    result
}

/// Insert the synthesized persistent CELL as the FIRST record under the target
/// World Children GRUP(1), with an empty Cell-Children(6) group, matching base
/// FO4 worldspace layout (persistent cell precedes the exterior block GRUPs).
fn insert_persistent_cell_into_world_children(
    root_items: &mut Vec<ParsedItem>,
    worldspace_editor_id: &str,
    cell: ParsedRecord,
    header_size: usize,
    payload: &mut SynthesizePersistentCellPayload,
) {
    let cell_form_id = cell.form_id;
    let Some(wrld_group) = root_items.iter_mut().find_map(|item| match item {
        ParsedItem::Group(group) if group.group_type == 0 && group.label == *b"WRLD" => Some(group),
        _ => None,
    }) else {
        payload
            .warnings
            .push("target WRLD top group not found".to_string());
        return;
    };
    let Some(world_form_id) = wrld_group.children.iter().find_map(|item| match item {
        ParsedItem::Record(record)
            if record.signature.as_str() == "WRLD"
                && editor_id(record).eq_ignore_ascii_case(worldspace_editor_id) =>
        {
            Some(record.form_id)
        }
        _ => None,
    }) else {
        payload.warnings.push(format!(
            "target worldspace not found: {worldspace_editor_id}"
        ));
        return;
    };

    let world_children =
        ensure_world_children_group(&mut wrld_group.children, world_form_id, header_size);

    // A persistent cell already present means a prior run synthesized it; stay
    // idempotent and do not duplicate.
    if direct_world_persistent_cell_id(world_children).is_some() {
        payload.warnings.push(
            "target worldspace already has a persistent cell; skipping synthesis".to_string(),
        );
        return;
    }

    let cell_child_group = ParsedGroup {
        label: group_label_for_form_id(cell_form_id),
        group_type: CELL_CHILD_GROUP,
        tail: Bytes::from(vec![0u8; header_size.saturating_sub(16)]),
        children: Vec::new(),
    };
    world_children
        .children
        .insert(0, ParsedItem::Group(cell_child_group));
    world_children.children.insert(0, ParsedItem::Record(cell));
    payload.cell_synthesized = true;
}

/// Push the converted persistent refs into the synthesized cell's
/// Cell-Children(6) → Cell-Persistent(8) group.
fn route_converted_persistent_refs(
    root_items: &mut Vec<ParsedItem>,
    cell_form_id: u32,
    converted: Vec<ParsedRecord>,
    header_size: usize,
) {
    let Some(cell_child_group) = find_cell_child_group_mut(root_items, cell_form_id) else {
        return;
    };
    let persistent = ensure_child_group_mut(
        cell_child_group,
        PERSISTENT_GROUP,
        cell_form_id,
        header_size,
    );
    for record in converted {
        persistent.children.push(ParsedItem::Record(record));
    }
}

fn finish_synthesize(
    mut payload: SynthesizePersistentCellPayload,
    target_handle_id: u64,
    mut target_slot: NativePluginSlot,
    mut store: std::sync::MutexGuard<'_, std::collections::HashMap<u64, NativePluginSlot>>,
    total_start: Instant,
) -> PyResult<SynthesizePersistentCellPayload> {
    payload
        .timing
        .insert("total_ms".to_string(), total_start.elapsed().as_millis());
    target_slot.record_count_cache = None;
    target_slot.invalidate_index_sections();
    store.insert(target_handle_id, target_slot);
    Ok(payload)
}

pub fn plugin_handle_collect_water_manifest_json(
    handle_id: u64,
    worldspace_editor_id: &str,
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
) -> PyResult<String> {
    let mut payload = WaterManifestPayload {
        default_water_object_id: 0x0C8633,
        ..WaterManifestPayload::default()
    };

    let store = plugin_handle_store_ref().lock().unwrap();
    let slot = store
        .get(&handle_id)
        .ok_or_else(|| PyKeyError::new_err(format!("unknown plugin handle: {handle_id}")))?;
    let (world_record, warnings) = find_world(&slot.parsed, worldspace_editor_id);
    payload.warnings.extend(warnings);
    if let Some(world_record) = world_record {
        collect_worldspace_water_manifest_cells(
            world_record,
            min_x,
            min_y,
            max_x,
            max_y,
            &mut payload,
        );
    }

    serde_json::to_string(&payload)
        .map_err(|err| PyValueError::new_err(format!("failed to encode water manifest: {err}")))
}

fn collect_worldspace_water_manifest_cells(
    world_record: &ParsedRecord,
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
    payload: &mut WaterManifestPayload,
) {
    let mut coords = Vec::new();
    let mut heights = Vec::new();
    for subrecord in effective_subrecords_for_record(world_record).iter() {
        let data = subrecord.data.as_ref();
        match subrecord.signature.as_str() {
            "XCLW" => {
                if data.len() % 4 != 0 {
                    payload.warnings.push(format!(
                        "WRLD XCLW length is not a multiple of 4 bytes: {}",
                        data.len()
                    ));
                }
                for row in data.chunks_exact(4) {
                    let y = i16::from_le_bytes([row[0], row[1]]) as i32;
                    let x = i16::from_le_bytes([row[2], row[3]]) as i32;
                    coords.push((x, y));
                }
            }
            "WHGT" => {
                if data.len() % 4 != 0 {
                    payload.warnings.push(format!(
                        "WRLD WHGT length is not a multiple of 4 bytes: {}",
                        data.len()
                    ));
                }
                for row in data.chunks_exact(4) {
                    heights.push(f32::from_le_bytes([row[0], row[1], row[2], row[3]]));
                }
            }
            _ => {}
        }
    }
    if coords.len() != heights.len() {
        payload.warnings.push(format!(
            "WRLD water table count mismatch: XCLW={} WHGT={}",
            coords.len(),
            heights.len()
        ));
    }

    let mut cells = BTreeMap::new();
    for ((x, y), height) in coords.into_iter().zip(heights) {
        if !inside_water_manifest_bounds(x, y, min_x, min_y, max_x, max_y) {
            continue;
        }
        if !is_valid_water_height(height) {
            continue;
        }
        let entry = cells.entry((x, y)).or_insert(height);
        *entry = f32::max(*entry, height);
    }
    payload.cells = cells
        .into_iter()
        .map(|((x, y), height)| WaterManifestCell { x, y, height })
        .collect();
}

fn inside_water_manifest_bounds(
    x: i32,
    y: i32,
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
) -> bool {
    if max_x < min_x || max_y < min_y {
        return true;
    }
    inside_bounds(x, y, min_x, min_y, max_x, max_y)
}

fn is_valid_water_height(value: f32) -> bool {
    value.is_finite() && value.abs() < VALID_WATER_HEIGHT_LIMIT
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value as JsonValue;

    #[test]
    fn prefer_default_candidate_picks_use_prefixed_entry() {
        // LPI_FloraSootFlower01 → UseLPI_FloraSootFlower01 (non-nuked default),
        // never the flux/nuke or harvested variants sharing the list.
        let candidates = vec![
            (0x0155_D76, Some("FloraRadGeigerBlossom01".to_string())),
            (
                0x03C6_0C9,
                Some("UseLPI_FloraSootFlower01_Harvested".to_string()),
            ),
            (0x01C0_E69, Some("UseLPI_FloraSootFlower01".to_string())),
        ];
        assert_eq!(
            prefer_default_candidate("LPI_FloraSootFlower01", &candidates),
            Some(0x01C0_E69)
        );
        // No matching default → None (caller falls back to the stable pick).
        let none = vec![
            (0x0155_D76, Some("FloraRadGeigerBlossom01".to_string())),
            (0x0AAAAAA, None),
        ];
        assert_eq!(
            prefer_default_candidate("LPI_FloraSootFlower01", &none),
            None
        );
        // Empty base editor-id never matches.
        assert_eq!(prefer_default_candidate("", &candidates), None);
    }

    fn subrecord(signature: &str, data: Vec<u8>) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: SmolStr::new(signature),
            data: Bytes::from(data),
            semantic_type: None,
        }
    }

    fn xprm_bytes(type_value: u32) -> Vec<u8> {
        let mut data = vec![0u8; 32];
        data[28..32].copy_from_slice(&type_value.to_le_bytes());
        data
    }

    fn xrdo_bytes(flags: u32) -> Vec<u8> {
        let mut data = vec![0u8; 16];
        data[12..16].copy_from_slice(&flags.to_le_bytes());
        data
    }

    #[test]
    fn xprm_type_clamps_out_of_domain_to_none() {
        // FO76 cylinder (7) / value (6) are out of FO4's 0..=5 domain.
        let cylinder = normalize_placed_enum_flag_subrecord("XPRM", &xprm_bytes(7)).unwrap();
        assert_eq!(&cylinder[28..32], &0u32.to_le_bytes());
        let value = normalize_placed_enum_flag_subrecord("XPRM", &xprm_bytes(6)).unwrap();
        assert_eq!(&value[28..32], &0u32.to_le_bytes());
        // In-domain values are left untouched (no rewrite).
        assert!(normalize_placed_enum_flag_subrecord("XPRM", &xprm_bytes(5)).is_none());
        assert!(normalize_placed_enum_flag_subrecord("XPRM", &xprm_bytes(0)).is_none());
    }

    #[test]
    fn xrdo_flags_mask_to_fo4_known_bit() {
        // FO76 unknown bits 0x100 (bit 8) and 0x10000 (bit 16) masked off,
        // FO4 IgnoresDistanceChecks (0x1) preserved.
        let mixed = normalize_placed_enum_flag_subrecord("XRDO", &xrdo_bytes(0x10101)).unwrap();
        assert_eq!(&mixed[12..16], &1u32.to_le_bytes());
        let only_unknown =
            normalize_placed_enum_flag_subrecord("XRDO", &xrdo_bytes(0x10000)).unwrap();
        assert_eq!(&only_unknown[12..16], &0u32.to_le_bytes());
        // Already-clean values are left untouched.
        assert!(normalize_placed_enum_flag_subrecord("XRDO", &xrdo_bytes(0x1)).is_none());
        assert!(normalize_placed_enum_flag_subrecord("XRDO", &xrdo_bytes(0x0)).is_none());
    }

    #[test]
    fn region_map_marker_refs_lose_visible_map_marker_payload() {
        let mut region = record("REFR", 0x00869989, Some("RegionMapMarkerTheForest"));
        region.raw_payload = Some(Bytes::from_static(b"raw"));
        region
            .subrecords
            .push(subrecord("NAME", 0x00000010_u32.to_le_bytes().to_vec()));
        region.subrecords.push(subrecord("XMRK", Vec::new()));
        region.subrecords.push(subrecord("FNAM", vec![0x03]));
        region
            .subrecords
            .push(subrecord("FULL", b"$REGION_THE_FOREST\0".to_vec()));
        region.subrecords.push(subrecord("TNAM", vec![64, 0]));
        region.subrecords.push(subrecord("DATA", vec![0; 24]));

        assert_eq!(strip_region_map_marker_payload(&mut region), 4);
        assert!(region.raw_payload.is_none());
        assert!(subrecord_data(&region, "XMRK").is_none());
        assert!(subrecord_data(&region, "FNAM").is_none());
        assert!(subrecord_data(&region, "FULL").is_none());
        assert!(subrecord_data(&region, "TNAM").is_none());
        assert!(subrecord_data(&region, "EDID").is_some());
        assert!(subrecord_data(&region, "NAME").is_some());
        assert!(subrecord_data(&region, "DATA").is_some());
    }

    #[test]
    fn non_region_map_marker_refs_keep_visible_map_marker_payload() {
        let mut vault = record("REFR", 0x000B1051, Some("Vault76MapMarker"));
        vault.subrecords.push(subrecord("XMRK", Vec::new()));
        vault.subrecords.push(subrecord("FNAM", vec![0x03]));
        vault
            .subrecords
            .push(subrecord("FULL", b"Vault 76\0".to_vec()));
        vault.subrecords.push(subrecord("TNAM", vec![68, 0]));

        assert_eq!(strip_region_map_marker_payload(&mut vault), 0);
        assert!(subrecord_data(&vault, "XMRK").is_some());
        assert!(subrecord_data(&vault, "FNAM").is_some());
        assert!(subrecord_data(&vault, "FULL").is_some());
        assert!(subrecord_data(&vault, "TNAM").is_some());
    }

    fn plain_group(group_type: i32, label: [u8; 4], children: Vec<ParsedItem>) -> ParsedGroup {
        ParsedGroup {
            label,
            group_type,
            tail: Bytes::new(),
            children,
        }
    }

    #[test]
    fn clone_terrain_only_keeps_land_navm_drops_placed() {
        let cell_id = 0x000801u32;
        let temp = plain_group(
            TEMPORARY_GROUP,
            cell_id.to_le_bytes(),
            vec![
                ParsedItem::Record(record("LAND", 0x000803, None)),
                ParsedItem::Record(record("NAVM", 0x000804, None)),
                ParsedItem::Record(record("REFR", 0x000805, None)),
            ],
        );
        let persistent = plain_group(
            PERSISTENT_GROUP,
            cell_id.to_le_bytes(),
            vec![ParsedItem::Record(record("REFR", 0x000806, None))],
        );
        let child_group = plain_group(
            CELL_CHILD_GROUP,
            cell_id.to_le_bytes(),
            vec![ParsedItem::Group(persistent), ParsedItem::Group(temp)],
        );
        let subblock = plain_group(
            EXTERIOR_CELL_SUBBLOCK,
            [0, 0, 0, 0],
            vec![
                ParsedItem::Record(cell_record(cell_id, "TestCell", 3, -2)),
                ParsedItem::Group(child_group),
            ],
        );
        let block = plain_group(
            EXTERIOR_CELL_BLOCK,
            [0, 0, 0, 0],
            vec![ParsedItem::Group(subblock)],
        );

        let mut report = GraftTerrainReport::default();
        let cloned = clone_terrain_only_group(&block, &mut report);

        assert_eq!(report.cells, 1);
        assert_eq!(report.lands, 1);
        assert_eq!(report.navms, 1);

        let mut lands = Vec::new();
        collect_group_records(&cloned, "LAND", &mut lands);
        assert_eq!(lands.len(), 1);
        let mut navms = Vec::new();
        collect_group_records(&cloned, "NAVM", &mut navms);
        assert_eq!(navms.len(), 1);
        let mut refrs = Vec::new();
        collect_group_records(&cloned, "REFR", &mut refrs);
        assert!(refrs.is_empty(), "placed children must be stripped");
        let mut cells = Vec::new();
        collect_group_records(&cloned, "CELL", &mut cells);
        assert_eq!(cells.len(), 1);

        fn has_group_type(group: &ParsedGroup, group_type: i32) -> bool {
            group.group_type == group_type
                || group.children.iter().any(
                    |child| matches!(child, ParsedItem::Group(g) if has_group_type(g, group_type)),
                )
        }
        assert!(
            !has_group_type(&cloned, PERSISTENT_GROUP),
            "per-cell persistent group must be dropped"
        );
        assert!(
            has_group_type(&cloned, TEMPORARY_GROUP),
            "temporary group (holding LAND/NAVM) must be kept"
        );

        for id in [0x000801u32, 0x000803, 0x000804] {
            assert!(
                report.object_ids.contains(&id),
                "object_ids must include {id:06X}"
            );
        }
        assert!(
            !report.object_ids.contains(&0x000805),
            "stripped placed child id must not be reserved"
        );
    }

    fn record(signature: &str, form_id: u32, editor_id: Option<&str>) -> ParsedRecord {
        let mut subrecords = Vec::new();
        if let Some(editor_id) = editor_id {
            let mut data = editor_id.as_bytes().to_vec();
            data.push(0);
            subrecords.push(subrecord("EDID", data));
        }
        ParsedRecord {
            signature: SmolStr::new(signature),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: Some(131),
            version2: Some(1),
            subrecords,
            raw_payload: None,
            parse_error: None,
        }
    }

    fn cell_record(form_id: u32, editor_id: &str, x: i32, y: i32) -> ParsedRecord {
        let mut record = record("CELL", form_id, Some(editor_id));
        let mut grid = Vec::new();
        grid.extend_from_slice(&x.to_le_bytes());
        grid.extend_from_slice(&y.to_le_bytes());
        grid.extend_from_slice(&[0, 0, 0, 0]);
        record.subrecords.push(subrecord("XCLC", grid));
        record
    }

    fn top_group(signature: &str, children: Vec<ParsedItem>) -> ParsedItem {
        let bytes = signature.as_bytes();
        ParsedItem::Group(ParsedGroup {
            label: [bytes[0], bytes[1], bytes[2], bytes[3]],
            group_type: 0,
            tail: Bytes::new(),
            children,
        })
    }

    fn plugin(root_items: Vec<ParsedItem>) -> ParsedPlugin {
        ParsedPlugin {
            plugin_name: "SeventySix.esm".to_string(),
            file_path: String::new(),
            header_size: MODERN_HEADER_SIZE,
            header: ParsedPluginHeader::default_for_test(),
            root_items,
            game: Some("fo76".to_string()),
        }
    }

    fn plugin_with_name(
        plugin_name: &str,
        game: &str,
        masters: Vec<String>,
        root_items: Vec<ParsedItem>,
    ) -> ParsedPlugin {
        let mut header = ParsedPluginHeader::default_for_test();
        header.masters = masters;
        ParsedPlugin {
            plugin_name: plugin_name.to_string(),
            file_path: String::new(),
            header_size: MODERN_HEADER_SIZE,
            header,
            root_items,
            game: Some(game.to_string()),
        }
    }

    #[test]
    fn water_manifest_uses_fo76_wrld_xclw_whgt_rows() {
        let mut world = record("WRLD", 0x25DA15, Some("APPALACHIA"));
        let mut xclw = Vec::new();
        for (x, y) in [(-29_i16, 28_i16), (0, 0), (-30, 30)] {
            xclw.extend_from_slice(&y.to_le_bytes());
            xclw.extend_from_slice(&x.to_le_bytes());
        }
        let mut whgt = Vec::new();
        for height in [196.0_f32, 12.5, 300.0] {
            whgt.extend_from_slice(&height.to_le_bytes());
        }
        world.subrecords.push(subrecord("XCLW", xclw));
        world.subrecords.push(subrecord("WHGT", whgt));

        let mut payload = WaterManifestPayload {
            default_water_object_id: 0x0C8633,
            ..WaterManifestPayload::default()
        };
        collect_worldspace_water_manifest_cells(&world, -34, 14, -19, 29, &mut payload);

        assert_eq!(payload.cells.len(), 1);
        assert_eq!(payload.cells[0].x, -29);
        assert_eq!(payload.cells[0].y, 28);
        assert_eq!(payload.cells[0].height, 196.0);
        assert!(payload.warnings.is_empty());
    }

    fn location_record(form_id: u32, world_form_id: u32, cells: &[(i16, i16)]) -> ParsedRecord {
        let mut record = record("LCTN", form_id, None);
        let mut data = Vec::new();
        data.extend_from_slice(&world_form_id.to_le_bytes());
        for (x, y) in cells {
            data.extend_from_slice(&y.to_le_bytes());
            data.extend_from_slice(&x.to_le_bytes());
        }
        record.subrecords.push(subrecord("LCEC", data));
        record
    }

    fn projected_world_with_cell(world_form_id: u32, cell: ParsedRecord) -> Vec<ParsedItem> {
        let world = record("WRLD", world_form_id, Some("APPALACHIA"));
        let cell_form_id = cell.form_id;
        let exterior_block = ParsedItem::Group(ParsedGroup {
            label: [0, 0, 0, 0],
            group_type: EXTERIOR_CELL_BLOCK,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(ParsedGroup {
                label: [0, 0, 0, 0],
                group_type: EXTERIOR_CELL_SUBBLOCK,
                tail: Bytes::new(),
                children: vec![
                    ParsedItem::Record(cell),
                    ParsedItem::Group(ParsedGroup {
                        label: cell_form_id.to_le_bytes(),
                        group_type: CELL_CHILD_GROUP,
                        tail: Bytes::new(),
                        children: Vec::new(),
                    }),
                ],
            })],
        });
        let world_children = ParsedItem::Group(ParsedGroup {
            label: world_form_id.to_le_bytes(),
            group_type: 1,
            tail: Bytes::new(),
            children: vec![exterior_block],
        });
        vec![top_group(
            "WRLD",
            vec![ParsedItem::Record(world), world_children],
        )]
    }

    fn find_record_by_form_id<'a>(
        items: &'a [ParsedItem],
        form_id: u32,
    ) -> Option<&'a ParsedRecord> {
        for item in items {
            match item {
                ParsedItem::Record(record) if record.form_id == form_id => return Some(record),
                ParsedItem::Group(group) => {
                    if let Some(record) = find_record_by_form_id(&group.children, form_id) {
                        return Some(record);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Pins the COMPLETE roots payload (minus timing) on a
    /// fixture covering cross-cell duplicate bases, LVLI expansion (nested),
    /// XLKR keywords, XLYR layers, XCLR regions, XLCN locations, an
    /// out-of-bounds cell, an XCLC-less cell warning, and the direct
    /// persistent-cell skip — so the parallel gather/serial replay refactor is
    /// provably output-identical.
    #[test]
    fn collect_roots_full_payload_pinned() {
        let world = record("WRLD", 0x0025DA15, Some("APPALACHIA"));
        let persistent_cell = cell_record(0x00050B2C, "PersistentWorldCell", 0, 0);
        let persistent_cell_children = ParsedItem::Group(ParsedGroup {
            label: 0x00050B2C_u32.to_le_bytes(),
            group_type: CELL_CHILD_GROUP,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(ParsedGroup {
                label: 0x00050B2C_u32.to_le_bytes(),
                group_type: PERSISTENT_GROUP,
                tail: Bytes::new(),
                children: vec![ParsedItem::Record(record("REFR", 0x008CC335, None))],
            })],
        });

        // Base/dependency records the locator must resolve.
        let stat_a = record("STAT", 0x001A6663, Some("RockBaseA"));
        let stat_b = record("STAT", 0x001A7777, Some("RockBaseB"));
        let stat_lvl1 = record("STAT", 0x00610000, Some("LeveledStatA"));
        let stat_lvl2 = record("STAT", 0x00620000, Some("LeveledStatB"));
        let mut lvli_inner = record("LVLI", 0x00600001, Some("InnerLeveled"));
        {
            let mut lvlo = vec![0u8; 12];
            lvlo[4..8].copy_from_slice(&0x00620000_u32.to_le_bytes());
            lvli_inner.subrecords.push(subrecord("LVLO", lvlo));
        }
        let mut lvli_outer = record("LVLI", 0x00600000, Some("OuterLeveled"));
        {
            let mut lvlo = vec![0u8; 12];
            lvlo[4..8].copy_from_slice(&0x00610000_u32.to_le_bytes());
            lvli_outer.subrecords.push(subrecord("LVLO", lvlo));
            let mut lvlo2 = vec![0u8; 12];
            lvlo2[4..8].copy_from_slice(&0x00600001_u32.to_le_bytes());
            lvli_outer.subrecords.push(subrecord("LVLO", lvlo2));
        }
        let keyword = record("KYWD", 0x001CA8D0, Some("LinkKeyword"));
        let layer = record("LAYR", 0x00700000, Some("PlacementLayer"));
        let region = record("REGN", 0x00400000, Some("CellRegion"));
        let location = record("LCTN", 0x00410000, Some("CellLocation"));

        // Three in-bounds cells at (0,0), (1,0), (0,1) with 30 placed each;
        // one out-of-bounds cell at (9,9); one CELL without XCLC.
        let mut subblock_children: Vec<ParsedItem> = Vec::new();
        for (cell_index, grid) in [(0u32, (0, 0)), (1, (1, 0)), (2, (0, 1))] {
            let cell_id = 0x00300000 + cell_index;
            let mut cell = cell_record(cell_id, &format!("Ext{cell_index}"), grid.0, grid.1);
            if cell_index == 0 {
                cell.subrecords
                    .push(subrecord("XCLR", 0x00400000_u32.to_le_bytes().to_vec()));
            }
            if cell_index == 1 {
                cell.subrecords
                    .push(subrecord("XLCN", 0x00410000_u32.to_le_bytes().to_vec()));
            }
            let mut placed_children: Vec<ParsedItem> = Vec::new();
            for i in 0..30u32 {
                let mut placed = record("REFR", 0x00500000 + cell_index * 0x100 + i, None);
                let base_raw: u32 = match i % 5 {
                    0 => 0x001A6663, // shared cross-cell base
                    1 => 0x001A7777, // second shared base
                    2 => 0x00600000, // LVLI base -> nested expansion
                    3 => 0x00DEAD00, // missing base -> not collected
                    _ => 0x001A6663, // duplicate again
                };
                placed
                    .subrecords
                    .push(subrecord("NAME", base_raw.to_le_bytes().to_vec()));
                if i % 7 == 0 {
                    let mut xlkr = Vec::new();
                    xlkr.extend_from_slice(&0x001CA8D0_u32.to_le_bytes());
                    xlkr.extend_from_slice(&(0x00500000_u32 + i).to_le_bytes());
                    placed.subrecords.push(subrecord("XLKR", xlkr));
                }
                if i % 11 == 0 {
                    placed
                        .subrecords
                        .push(subrecord("XLYR", 0x00700000_u32.to_le_bytes().to_vec()));
                }
                placed_children.push(ParsedItem::Record(placed));
            }
            // Split across persistent + temporary sections.
            let persistent: Vec<ParsedItem> = placed_children.drain(0..10).collect();
            let cell_children = ParsedItem::Group(ParsedGroup {
                label: cell_id.to_le_bytes(),
                group_type: CELL_CHILD_GROUP,
                tail: Bytes::new(),
                children: vec![
                    ParsedItem::Group(ParsedGroup {
                        label: cell_id.to_le_bytes(),
                        group_type: PERSISTENT_GROUP,
                        tail: Bytes::new(),
                        children: persistent,
                    }),
                    ParsedItem::Group(ParsedGroup {
                        label: cell_id.to_le_bytes(),
                        group_type: TEMPORARY_GROUP,
                        tail: Bytes::new(),
                        children: placed_children,
                    }),
                ],
            });
            subblock_children.push(ParsedItem::Record(cell));
            subblock_children.push(cell_children);
        }
        subblock_children.push(ParsedItem::Record(cell_record(0x00300009, "FarCell", 9, 9)));
        subblock_children.push(ParsedItem::Record(record(
            "CELL",
            0x0030000A,
            Some("NoGridCell"),
        )));

        let exterior_block = ParsedItem::Group(ParsedGroup {
            label: [0, 0, 0, 0],
            group_type: EXTERIOR_CELL_BLOCK,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(ParsedGroup {
                label: [0, 0, 0, 0],
                group_type: EXTERIOR_CELL_SUBBLOCK,
                tail: Bytes::new(),
                children: subblock_children,
            })],
        });
        let world_children = ParsedItem::Group(ParsedGroup {
            label: 0x0025DA15_u32.to_le_bytes(),
            group_type: 1,
            tail: Bytes::new(),
            children: vec![
                ParsedItem::Record(persistent_cell),
                persistent_cell_children,
                exterior_block,
            ],
        });
        let plugin = plugin(vec![
            top_group("WRLD", vec![ParsedItem::Record(world), world_children]),
            top_group(
                "STAT",
                vec![
                    ParsedItem::Record(stat_a),
                    ParsedItem::Record(stat_b),
                    ParsedItem::Record(stat_lvl1),
                    ParsedItem::Record(stat_lvl2),
                ],
            ),
            top_group(
                "LVLI",
                vec![
                    ParsedItem::Record(lvli_outer),
                    ParsedItem::Record(lvli_inner),
                ],
            ),
            top_group("KYWD", vec![ParsedItem::Record(keyword)]),
            top_group("LAYR", vec![ParsedItem::Record(layer)]),
            top_group("REGN", vec![ParsedItem::Record(region)]),
            top_group("LCTN", vec![ParsedItem::Record(location)]),
        ]);
        let handle = insert_plugin_handle(plugin, LocalizedStringsState::default());
        let payload_text = plugin_handle_collect_cell_slice_roots_json(
            handle,
            "APPALACHIA",
            0,
            0,
            1,
            1,
            false,
            None,
        )
        .expect("collect roots");
        plugin_handle_close_native(handle);

        let mut payload: JsonValue = serde_json::from_str(&payload_text).expect("json payload");
        payload
            .as_object_mut()
            .expect("payload object")
            .remove("timing");
        let canonical = serde_json::to_string(&payload).expect("canonical payload");
        let expected = r#"{"cell_form_keys":["SeventySix.esm:300000","SeventySix.esm:300001","SeventySix.esm:300002"],"placed_form_keys":["SeventySix.esm:500000","SeventySix.esm:500001","SeventySix.esm:500002","SeventySix.esm:500003","SeventySix.esm:500004","SeventySix.esm:500005","SeventySix.esm:500006","SeventySix.esm:500007","SeventySix.esm:500008","SeventySix.esm:500009","SeventySix.esm:50000A","SeventySix.esm:50000B","SeventySix.esm:50000C","SeventySix.esm:50000D","SeventySix.esm:50000E","SeventySix.esm:50000F","SeventySix.esm:500010","SeventySix.esm:500011","SeventySix.esm:500012","SeventySix.esm:500013","SeventySix.esm:500014","SeventySix.esm:500015","SeventySix.esm:500016","SeventySix.esm:500017","SeventySix.esm:500018","SeventySix.esm:500019","SeventySix.esm:50001A","SeventySix.esm:50001B","SeventySix.esm:50001C","SeventySix.esm:50001D","SeventySix.esm:500100","SeventySix.esm:500101","SeventySix.esm:500102","SeventySix.esm:500103","SeventySix.esm:500104","SeventySix.esm:500105","SeventySix.esm:500106","SeventySix.esm:500107","SeventySix.esm:500108","SeventySix.esm:500109","SeventySix.esm:50010A","SeventySix.esm:50010B","SeventySix.esm:50010C","SeventySix.esm:50010D","SeventySix.esm:50010E","SeventySix.esm:50010F","SeventySix.esm:500110","SeventySix.esm:500111","SeventySix.esm:500112","SeventySix.esm:500113","SeventySix.esm:500114","SeventySix.esm:500115","SeventySix.esm:500116","SeventySix.esm:500117","SeventySix.esm:500118","SeventySix.esm:500119","SeventySix.esm:50011A","SeventySix.esm:50011B","SeventySix.esm:50011C","SeventySix.esm:50011D","SeventySix.esm:500200","SeventySix.esm:500201","SeventySix.esm:500202","SeventySix.esm:500203","SeventySix.esm:500204","SeventySix.esm:500205","SeventySix.esm:500206","SeventySix.esm:500207","SeventySix.esm:500208","SeventySix.esm:500209","SeventySix.esm:50020A","SeventySix.esm:50020B","SeventySix.esm:50020C","SeventySix.esm:50020D","SeventySix.esm:50020E","SeventySix.esm:50020F","SeventySix.esm:500210","SeventySix.esm:500211","SeventySix.esm:500212","SeventySix.esm:500213","SeventySix.esm:500214","SeventySix.esm:500215","SeventySix.esm:500216","SeventySix.esm:500217","SeventySix.esm:500218","SeventySix.esm:500219","SeventySix.esm:50021A","SeventySix.esm:50021B","SeventySix.esm:50021C","SeventySix.esm:50021D"],"static_base_form_keys":["SeventySix.esm:1A6663","SeventySix.esm:1A7777","SeventySix.esm:600000"],"leveled_base_entry_form_keys":["SeventySix.esm:610000","SeventySix.esm:600001","SeventySix.esm:620000"],"linked_ref_keyword_form_keys":["SeventySix.esm:1CA8D0"],"layer_form_keys":["SeventySix.esm:700000"],"location_form_keys":["SeventySix.esm:410000"],"location_data_form_keys":[],"region_form_keys":["SeventySix.esm:400000"],"region_data_form_keys":[],"audio_data_form_keys":[],"worldspace_form_keys":["SeventySix.esm:25DA15"],"worldspace_data_form_keys":[],"cell_children":{"SeventySix.esm:300000":{"Persistent":["SeventySix.esm:500000","SeventySix.esm:500001","SeventySix.esm:500002","SeventySix.esm:500003","SeventySix.esm:500004","SeventySix.esm:500005","SeventySix.esm:500006","SeventySix.esm:500007","SeventySix.esm:500008","SeventySix.esm:500009"],"Temporary":["SeventySix.esm:50000A","SeventySix.esm:50000B","SeventySix.esm:50000C","SeventySix.esm:50000D","SeventySix.esm:50000E","SeventySix.esm:50000F","SeventySix.esm:500010","SeventySix.esm:500011","SeventySix.esm:500012","SeventySix.esm:500013","SeventySix.esm:500014","SeventySix.esm:500015","SeventySix.esm:500016","SeventySix.esm:500017","SeventySix.esm:500018","SeventySix.esm:500019","SeventySix.esm:50001A","SeventySix.esm:50001B","SeventySix.esm:50001C","SeventySix.esm:50001D"]},"SeventySix.esm:300001":{"Persistent":["SeventySix.esm:500100","SeventySix.esm:500101","SeventySix.esm:500102","SeventySix.esm:500103","SeventySix.esm:500104","SeventySix.esm:500105","SeventySix.esm:500106","SeventySix.esm:500107","SeventySix.esm:500108","SeventySix.esm:500109"],"Temporary":["SeventySix.esm:50010A","SeventySix.esm:50010B","SeventySix.esm:50010C","SeventySix.esm:50010D","SeventySix.esm:50010E","SeventySix.esm:50010F","SeventySix.esm:500110","SeventySix.esm:500111","SeventySix.esm:500112","SeventySix.esm:500113","SeventySix.esm:500114","SeventySix.esm:500115","SeventySix.esm:500116","SeventySix.esm:500117","SeventySix.esm:500118","SeventySix.esm:500119","SeventySix.esm:50011A","SeventySix.esm:50011B","SeventySix.esm:50011C","SeventySix.esm:50011D"]},"SeventySix.esm:300002":{"Persistent":["SeventySix.esm:500200","SeventySix.esm:500201","SeventySix.esm:500202","SeventySix.esm:500203","SeventySix.esm:500204","SeventySix.esm:500205","SeventySix.esm:500206","SeventySix.esm:500207","SeventySix.esm:500208","SeventySix.esm:500209"],"Temporary":["SeventySix.esm:50020A","SeventySix.esm:50020B","SeventySix.esm:50020C","SeventySix.esm:50020D","SeventySix.esm:50020E","SeventySix.esm:50020F","SeventySix.esm:500210","SeventySix.esm:500211","SeventySix.esm:500212","SeventySix.esm:500213","SeventySix.esm:500214","SeventySix.esm:500215","SeventySix.esm:500216","SeventySix.esm:500217","SeventySix.esm:500218","SeventySix.esm:500219","SeventySix.esm:50021A","SeventySix.esm:50021B","SeventySix.esm:50021C","SeventySix.esm:50021D"]}},"cell_grids":{"SeventySix.esm:300000":{"x":0,"y":0},"SeventySix.esm:300001":{"x":1,"y":0},"SeventySix.esm:300002":{"x":0,"y":1}},"warnings":["CELL without XCLC skipped: SeventySix.esm:30000A"]}"#;
        assert_eq!(canonical, expected, "pinned roots payload changed");
    }

    #[test]
    fn collect_roots_skips_direct_persistent_cell_for_bounded_slice() {
        let world = record("WRLD", 0x0025DA15, Some("APPALACHIA"));
        let persistent_cell = cell_record(0x00050B2C, "PersistentWorldCell", 0, 0);
        let persistent_ref = record("REFR", 0x008CC335, None);
        let exterior_cell = cell_record(0x002628FE, "OriginExt", 0, 0);
        let temp_ref = record("REFR", 0x004EA534, None);
        let persistent_cell_children = ParsedItem::Group(ParsedGroup {
            label: 0x00050B2C_u32.to_le_bytes(),
            group_type: CELL_CHILD_GROUP,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(ParsedGroup {
                label: 0x00050B2C_u32.to_le_bytes(),
                group_type: PERSISTENT_GROUP,
                tail: Bytes::new(),
                children: vec![ParsedItem::Record(persistent_ref)],
            })],
        });
        let exterior_cell_children = ParsedItem::Group(ParsedGroup {
            label: 0x002628FE_u32.to_le_bytes(),
            group_type: CELL_CHILD_GROUP,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(ParsedGroup {
                label: 0x002628FE_u32.to_le_bytes(),
                group_type: TEMPORARY_GROUP,
                tail: Bytes::new(),
                children: vec![ParsedItem::Record(temp_ref)],
            })],
        });
        let exterior_block = ParsedItem::Group(ParsedGroup {
            label: [0, 0, 0, 0],
            group_type: EXTERIOR_CELL_BLOCK,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(ParsedGroup {
                label: [0, 0, 0, 0],
                group_type: EXTERIOR_CELL_SUBBLOCK,
                tail: Bytes::new(),
                children: vec![ParsedItem::Record(exterior_cell), exterior_cell_children],
            })],
        });
        let world_children = ParsedItem::Group(ParsedGroup {
            label: 0x0025DA15_u32.to_le_bytes(),
            group_type: 1,
            tail: Bytes::new(),
            children: vec![
                ParsedItem::Record(persistent_cell),
                persistent_cell_children,
                exterior_block,
            ],
        });
        let plugin = plugin(vec![top_group(
            "WRLD",
            vec![ParsedItem::Record(world), world_children],
        )]);
        let handle = insert_plugin_handle(plugin, LocalizedStringsState::default());
        let payload_text = plugin_handle_collect_cell_slice_roots_json(
            handle,
            "APPALACHIA",
            0,
            0,
            0,
            0,
            false,
            None,
        )
        .expect("collect roots");
        plugin_handle_close_native(handle);
        let payload: JsonValue = serde_json::from_str(&payload_text).expect("json payload");

        assert_eq!(
            payload["cell_form_keys"],
            serde_json::json!(["SeventySix.esm:2628FE"])
        );
        assert_eq!(
            payload["placed_form_keys"],
            serde_json::json!(["SeventySix.esm:4EA534"])
        );
        assert_eq!(
            payload["cell_children"]["SeventySix.esm:2628FE"]["Temporary"],
            serde_json::json!(["SeventySix.esm:4EA534"])
        );
    }

    #[test]
    fn collect_roots_recurses_nested_section_groups_and_ignores_navmesh() {
        let world = record("WRLD", 0x0025DA15, Some("APPALACHIA"));
        let exterior_cell = cell_record(0x002628FE, "OriginExt", 0, 0);
        let static_base = record("STAT", 0x001A6663, Some("RockBase"));
        let linked_keyword = record("KYWD", 0x001C_A8D0, Some("DMP_Sandbox_Prim_SkipLoad"));
        let linked_keyword2 = record("KYWD", 0x0019_EDDA, Some("AnimFurnCoupleHolding"));
        let mut nested_ref = record("REFR", 0x004EA534, None);
        nested_ref
            .subrecords
            .push(subrecord("NAME", 0x001A6663_u32.to_le_bytes().to_vec()));
        let mut xlkr = Vec::new();
        xlkr.extend_from_slice(&0x001C_A8D0_u32.to_le_bytes());
        xlkr.extend_from_slice(&0x004EA535_u32.to_le_bytes());
        nested_ref.subrecords.push(subrecord("XLKR", xlkr));
        let mut xlkr2 = Vec::new();
        xlkr2.extend_from_slice(&0x0019_EDDA_u32.to_le_bytes());
        xlkr2.extend_from_slice(&0x004EA536_u32.to_le_bytes());
        nested_ref.subrecords.push(subrecord("XLKR", xlkr2));
        let navmesh = record("NAVM", 0x0047129C, None);
        let exterior_cell_children = ParsedItem::Group(ParsedGroup {
            label: 0x002628FE_u32.to_le_bytes(),
            group_type: CELL_CHILD_GROUP,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(ParsedGroup {
                label: 0x002628FE_u32.to_le_bytes(),
                group_type: TEMPORARY_GROUP,
                tail: Bytes::new(),
                children: vec![
                    ParsedItem::Record(navmesh),
                    ParsedItem::Group(ParsedGroup {
                        label: [0, 0, 0, 0],
                        group_type: EXTERIOR_CELL_SUBBLOCK,
                        tail: Bytes::new(),
                        children: vec![ParsedItem::Record(nested_ref)],
                    }),
                ],
            })],
        });
        let exterior_block = ParsedItem::Group(ParsedGroup {
            label: [0, 0, 0, 0],
            group_type: EXTERIOR_CELL_BLOCK,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(ParsedGroup {
                label: [0, 0, 0, 0],
                group_type: EXTERIOR_CELL_SUBBLOCK,
                tail: Bytes::new(),
                children: vec![ParsedItem::Record(exterior_cell), exterior_cell_children],
            })],
        });
        let world_children = ParsedItem::Group(ParsedGroup {
            label: 0x0025DA15_u32.to_le_bytes(),
            group_type: 1,
            tail: Bytes::new(),
            children: vec![exterior_block],
        });
        let plugin = plugin(vec![
            top_group("STAT", vec![ParsedItem::Record(static_base)]),
            top_group(
                "KYWD",
                vec![
                    ParsedItem::Record(linked_keyword),
                    ParsedItem::Record(linked_keyword2),
                ],
            ),
            top_group("WRLD", vec![ParsedItem::Record(world), world_children]),
        ]);
        let handle = insert_plugin_handle(plugin, LocalizedStringsState::default());
        let payload_text = plugin_handle_collect_cell_slice_roots_json(
            handle,
            "APPALACHIA",
            0,
            0,
            0,
            0,
            false,
            None,
        )
        .expect("collect roots");
        plugin_handle_close_native(handle);
        let payload: JsonValue = serde_json::from_str(&payload_text).expect("json payload");

        assert_eq!(
            payload["placed_form_keys"],
            serde_json::json!(["SeventySix.esm:4EA534"])
        );
        assert_eq!(
            payload["static_base_form_keys"],
            serde_json::json!(["SeventySix.esm:1A6663"])
        );
        assert_eq!(
            payload["linked_ref_keyword_form_keys"],
            serde_json::json!(["SeventySix.esm:1CA8D0", "SeventySix.esm:19EDDA"])
        );
        assert_eq!(
            payload["cell_children"]["SeventySix.esm:2628FE"]["Temporary"],
            serde_json::json!(["SeventySix.esm:4EA534"])
        );
    }

    #[test]
    fn collect_roots_includes_region_payload_dependencies() {
        let region_id: u32 = 0x001746B;
        let music_id: u32 = 0x0010001;
        let weather_id: u32 = 0x0010002;
        let global_id: u32 = 0x0010003;
        let sound_id: u32 = 0x0010004;
        let grass_id: u32 = 0x0010005;

        let mut exterior_cell = cell_record(0x002628FE, "OriginExt", 0, 0);
        exterior_cell
            .subrecords
            .push(subrecord("XCLR", region_id.to_le_bytes().to_vec()));

        let mut region = record("REGN", region_id, Some("RegionOrigin"));
        region
            .subrecords
            .push(subrecord("RDMO", music_id.to_le_bytes().to_vec()));
        let mut rdwt = Vec::new();
        rdwt.extend_from_slice(&weather_id.to_le_bytes());
        rdwt.extend_from_slice(&50u32.to_le_bytes());
        rdwt.extend_from_slice(&global_id.to_le_bytes());
        region.subrecords.push(subrecord("RDWT", rdwt));
        let mut rdsa = Vec::new();
        rdsa.extend_from_slice(&sound_id.to_le_bytes());
        rdsa.extend_from_slice(&100u32.to_le_bytes());
        rdsa.extend_from_slice(&0u32.to_le_bytes());
        region.subrecords.push(subrecord("RDSA", rdsa));
        let mut rdgs = Vec::new();
        rdgs.extend_from_slice(&grass_id.to_le_bytes());
        rdgs.extend_from_slice(&100u32.to_le_bytes());
        region.subrecords.push(subrecord("RDGS", rdgs));

        let mut root_items = projected_world_with_cell(0x0025DA15, exterior_cell);
        root_items.extend([
            top_group("REGN", vec![ParsedItem::Record(region)]),
            top_group(
                "MUSC",
                vec![ParsedItem::Record(record("MUSC", music_id, None))],
            ),
            top_group(
                "WTHR",
                vec![ParsedItem::Record(record("WTHR", weather_id, None))],
            ),
            top_group(
                "GLOB",
                vec![ParsedItem::Record(record("GLOB", global_id, None))],
            ),
            top_group(
                "SNDR",
                vec![ParsedItem::Record(record("SNDR", sound_id, None))],
            ),
            top_group(
                "GRAS",
                vec![ParsedItem::Record(record("GRAS", grass_id, None))],
            ),
        ]);
        let plugin = plugin(root_items);
        let handle = insert_plugin_handle(plugin, LocalizedStringsState::default());
        let payload_text = plugin_handle_collect_cell_slice_roots_json(
            handle,
            "APPALACHIA",
            0,
            0,
            0,
            0,
            false,
            None,
        )
        .expect("collect roots");
        plugin_handle_close_native(handle);
        let payload: JsonValue = serde_json::from_str(&payload_text).expect("json payload");

        assert_eq!(
            payload["region_form_keys"],
            serde_json::json!(["SeventySix.esm:01746B"])
        );
        assert_eq!(
            payload["region_data_form_keys"],
            serde_json::json!([
                "SeventySix.esm:010001",
                "SeventySix.esm:010002",
                "SeventySix.esm:010003",
                "SeventySix.esm:010004",
                "SeventySix.esm:010005"
            ])
        );
    }

    #[test]
    fn collect_roots_includes_worldspace_service_dependencies() {
        let climate_id: u32 = 0x0011001;
        let water_id: u32 = 0x0011002;
        let lod_water_id: u32 = 0x0011003;

        let mut root_items =
            projected_world_with_cell(0x0025DA15, cell_record(0x002628FE, "OriginExt", 0, 0));
        let ParsedItem::Group(wrld_group) = &mut root_items[0] else {
            panic!("WRLD group");
        };
        let ParsedItem::Record(world) = &mut wrld_group.children[0] else {
            panic!("WRLD record");
        };
        world
            .subrecords
            .push(subrecord("CNAM", climate_id.to_le_bytes().to_vec()));
        world
            .subrecords
            .push(subrecord("NAM2", water_id.to_le_bytes().to_vec()));
        world
            .subrecords
            .push(subrecord("NAM3", lod_water_id.to_le_bytes().to_vec()));

        root_items.extend([
            top_group(
                "CLMT",
                vec![ParsedItem::Record(record("CLMT", climate_id, None))],
            ),
            top_group(
                "WATR",
                vec![
                    ParsedItem::Record(record("WATR", water_id, None)),
                    ParsedItem::Record(record("WATR", lod_water_id, None)),
                ],
            ),
        ]);
        let plugin = plugin(root_items);
        let handle = insert_plugin_handle(plugin, LocalizedStringsState::default());
        let payload_text = plugin_handle_collect_cell_slice_roots_json(
            handle,
            "APPALACHIA",
            0,
            0,
            0,
            0,
            false,
            None,
        )
        .expect("collect roots");
        plugin_handle_close_native(handle);
        let payload: JsonValue = serde_json::from_str(&payload_text).expect("json payload");

        assert_eq!(
            payload["worldspace_form_keys"],
            serde_json::json!(["SeventySix.esm:25DA15"])
        );
        assert_eq!(
            payload["worldspace_data_form_keys"],
            serde_json::json!([
                "SeventySix.esm:011001",
                "SeventySix.esm:011002",
                "SeventySix.esm:011003"
            ])
        );
    }

    #[test]
    fn collect_roots_includes_locations_and_location_music_dependencies() {
        let world_id: u32 = 0x0025DA15;
        let cell_location_id: u32 = 0x001789F8;
        let world_location_id: u32 = 0x0001558C;
        let parent_location_id: u32 = 0x00039CED;
        let keyword_id: u32 = 0x00180819;
        let music_id: u32 = 0x001096F7;
        let track_id: u32 = 0x0000A743;

        let mut exterior_cell = cell_record(0x002628FE, "OriginExt", 0, 0);
        exterior_cell
            .subrecords
            .push(subrecord("XLCN", cell_location_id.to_le_bytes().to_vec()));
        let mut root_items = projected_world_with_cell(world_id, exterior_cell);
        let ParsedItem::Group(wrld_group) = &mut root_items[0] else {
            panic!("WRLD group");
        };
        let ParsedItem::Record(world) = &mut wrld_group.children[0] else {
            panic!("WRLD record");
        };
        world
            .subrecords
            .push(subrecord("XLCN", world_location_id.to_le_bytes().to_vec()));

        let mut cell_location = location_record(cell_location_id, world_id, &[(0, 0)]);
        cell_location
            .subrecords
            .push(subrecord("KWDA", keyword_id.to_le_bytes().to_vec()));
        cell_location
            .subrecords
            .push(subrecord("NAM1", music_id.to_le_bytes().to_vec()));
        cell_location
            .subrecords
            .push(subrecord("PNAM", parent_location_id.to_le_bytes().to_vec()));
        let world_location = record("LCTN", world_location_id, Some("AppalachiaLocation"));
        let parent_location = record("LCTN", parent_location_id, Some("ParentLocation"));
        let mut music = record("MUSC", music_id, Some("MUS76ExploreForest"));
        music
            .subrecords
            .push(subrecord("TNAM", track_id.to_le_bytes().to_vec()));

        root_items.extend([
            top_group(
                "LCTN",
                vec![
                    ParsedItem::Record(cell_location),
                    ParsedItem::Record(world_location),
                    ParsedItem::Record(parent_location),
                ],
            ),
            top_group(
                "KYWD",
                vec![ParsedItem::Record(record("KYWD", keyword_id, None))],
            ),
            top_group("MUSC", vec![ParsedItem::Record(music)]),
            top_group(
                "MUST",
                vec![ParsedItem::Record(record("MUST", track_id, None))],
            ),
        ]);
        let plugin = plugin(root_items);
        let handle = insert_plugin_handle(plugin, LocalizedStringsState::default());
        let payload_text = plugin_handle_collect_cell_slice_roots_json(
            handle,
            "APPALACHIA",
            0,
            0,
            0,
            0,
            false,
            None,
        )
        .expect("collect roots");
        plugin_handle_close_native(handle);
        let payload: JsonValue = serde_json::from_str(&payload_text).expect("json payload");

        assert_eq!(
            payload["location_form_keys"],
            serde_json::json!(["SeventySix.esm:1789F8", "SeventySix.esm:01558C"])
        );
        assert_eq!(
            payload["location_data_form_keys"],
            serde_json::json!([
                "SeventySix.esm:180819",
                "SeventySix.esm:1096F7",
                "SeventySix.esm:039CED"
            ])
        );
        assert_eq!(
            payload["audio_data_form_keys"],
            serde_json::json!(["SeventySix.esm:00A743"])
        );
    }

    #[test]
    fn collect_roots_includes_audio_dependencies_from_placed_bases() {
        let sound_marker_id: u32 = 0x0010001;
        let aspc_id: u32 = 0x0010002;
        let sound_descriptor_id: u32 = 0x0010003;
        let loop_descriptor_id: u32 = 0x0010004;
        let child_descriptor_id: u32 = 0x0010005;
        let category_id: u32 = 0x0010006;
        let parent_category_id: u32 = 0x0010007;
        let output_model_id: u32 = 0x0010008;
        let region_id: u32 = 0x0010009;
        let reverb_id: u32 = 0x001000A;
        let layer_id: u32 = 0x001000B;
        let music_id: u32 = 0x001000C;
        let track_id: u32 = 0x001000D;

        let exterior_cell = cell_record(0x002628FE, "OriginExt", 0, 0);
        let mut sound_ref = record("REFR", 0x004EA534, None);
        sound_ref
            .subrecords
            .push(subrecord("NAME", sound_marker_id.to_le_bytes().to_vec()));
        let mut aspc_ref = record("REFR", 0x004EA535, None);
        aspc_ref
            .subrecords
            .push(subrecord("NAME", aspc_id.to_le_bytes().to_vec()));
        let exterior_cell_children = ParsedItem::Group(ParsedGroup {
            label: 0x002628FE_u32.to_le_bytes(),
            group_type: CELL_CHILD_GROUP,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(ParsedGroup {
                label: 0x002628FE_u32.to_le_bytes(),
                group_type: TEMPORARY_GROUP,
                tail: Bytes::new(),
                children: vec![ParsedItem::Record(sound_ref), ParsedItem::Record(aspc_ref)],
            })],
        });
        let exterior_block = ParsedItem::Group(ParsedGroup {
            label: [0, 0, 0, 0],
            group_type: EXTERIOR_CELL_BLOCK,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(ParsedGroup {
                label: [0, 0, 0, 0],
                group_type: EXTERIOR_CELL_SUBBLOCK,
                tail: Bytes::new(),
                children: vec![ParsedItem::Record(exterior_cell), exterior_cell_children],
            })],
        });
        let world = record("WRLD", 0x0025DA15, Some("APPALACHIA"));
        let world_children = ParsedItem::Group(ParsedGroup {
            label: 0x0025DA15_u32.to_le_bytes(),
            group_type: 1,
            tail: Bytes::new(),
            children: vec![exterior_block],
        });

        let mut sound_marker = record("SOUN", sound_marker_id, Some("SoundMarker"));
        sound_marker.subrecords.push(subrecord(
            "SDSC",
            sound_descriptor_id.to_le_bytes().to_vec(),
        ));
        let mut aspc = record("ASPC", aspc_id, Some("AcousticSpace"));
        aspc.subrecords
            .push(subrecord("DEFL", layer_id.to_le_bytes().to_vec()));
        aspc.subrecords
            .push(subrecord("SNAM", loop_descriptor_id.to_le_bytes().to_vec()));
        aspc.subrecords
            .push(subrecord("RDAT", region_id.to_le_bytes().to_vec()));
        aspc.subrecords
            .push(subrecord("BNAM", reverb_id.to_le_bytes().to_vec()));
        let mut descriptor = record("SNDR", sound_descriptor_id, Some("Descriptor"));
        descriptor
            .subrecords
            .push(subrecord("GNAM", category_id.to_le_bytes().to_vec()));
        descriptor
            .subrecords
            .push(subrecord("ONAM", output_model_id.to_le_bytes().to_vec()));
        descriptor.subrecords.push(subrecord(
            "DNAM",
            child_descriptor_id.to_le_bytes().to_vec(),
        ));
        let loop_descriptor = record("SNDR", loop_descriptor_id, Some("LoopDescriptor"));
        let child_descriptor = record("SNDR", child_descriptor_id, Some("ChildDescriptor"));
        let mut category = record("SNCT", category_id, Some("Category"));
        category
            .subrecords
            .push(subrecord("PNAM", parent_category_id.to_le_bytes().to_vec()));
        let parent_category = record("SNCT", parent_category_id, Some("ParentCategory"));
        let mut region = record("REGN", region_id, Some("AudioRegion"));
        region
            .subrecords
            .push(subrecord("RDMO", music_id.to_le_bytes().to_vec()));
        let mut music = record("MUSC", music_id, Some("Music"));
        music
            .subrecords
            .push(subrecord("TNAM", track_id.to_le_bytes().to_vec()));

        let plugin = plugin(vec![
            top_group("WRLD", vec![ParsedItem::Record(world), world_children]),
            top_group("SOUN", vec![ParsedItem::Record(sound_marker)]),
            top_group("ASPC", vec![ParsedItem::Record(aspc)]),
            top_group(
                "SNDR",
                vec![
                    ParsedItem::Record(descriptor),
                    ParsedItem::Record(loop_descriptor),
                    ParsedItem::Record(child_descriptor),
                ],
            ),
            top_group(
                "SNCT",
                vec![
                    ParsedItem::Record(category),
                    ParsedItem::Record(parent_category),
                ],
            ),
            top_group(
                "SOPM",
                vec![ParsedItem::Record(record("SOPM", output_model_id, None))],
            ),
            top_group("REGN", vec![ParsedItem::Record(region)]),
            top_group(
                "REVB",
                vec![ParsedItem::Record(record("REVB", reverb_id, None))],
            ),
            top_group(
                "LAYR",
                vec![ParsedItem::Record(record("LAYR", layer_id, None))],
            ),
            top_group("MUSC", vec![ParsedItem::Record(music)]),
            top_group(
                "MUST",
                vec![ParsedItem::Record(record("MUST", track_id, None))],
            ),
        ]);
        let handle = insert_plugin_handle(plugin, LocalizedStringsState::default());
        let payload_text = plugin_handle_collect_cell_slice_roots_json(
            handle,
            "APPALACHIA",
            0,
            0,
            0,
            0,
            false,
            None,
        )
        .expect("collect roots");
        plugin_handle_close_native(handle);
        let payload: JsonValue = serde_json::from_str(&payload_text).expect("json payload");
        let audio = payload["audio_data_form_keys"]
            .as_array()
            .expect("audio deps");

        for expected in [
            "SeventySix.esm:010003",
            "SeventySix.esm:010004",
            "SeventySix.esm:010005",
            "SeventySix.esm:010006",
            "SeventySix.esm:010007",
            "SeventySix.esm:010008",
            "SeventySix.esm:010009",
            "SeventySix.esm:01000A",
            "SeventySix.esm:01000B",
            "SeventySix.esm:01000C",
            "SeventySix.esm:01000D",
        ] {
            assert!(
                audio.contains(&serde_json::json!(expected)),
                "missing {expected} in {audio:?}"
            );
        }
    }

    #[test]
    fn collect_roots_includes_entries_from_lvli_placement_bases() {
        let world = record("WRLD", 0x0025DA15, Some("APPALACHIA"));
        let exterior_cell = cell_record(0x002628FE, "OriginExt", 0, 0);
        let static_base = record("STAT", 0x001A6663, Some("RockBase"));
        let mut leveled_base = record("LVLI", 0x00100000, Some("LPI_RockBase"));
        leveled_base
            .subrecords
            .push(subrecord("LVLO", 0x001A6663_u32.to_le_bytes().to_vec()));
        let mut nested_ref = record("REFR", 0x004EA534, None);
        nested_ref
            .subrecords
            .push(subrecord("NAME", 0x00100000_u32.to_le_bytes().to_vec()));
        let exterior_cell_children = ParsedItem::Group(ParsedGroup {
            label: 0x002628FE_u32.to_le_bytes(),
            group_type: CELL_CHILD_GROUP,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(ParsedGroup {
                label: 0x002628FE_u32.to_le_bytes(),
                group_type: TEMPORARY_GROUP,
                tail: Bytes::new(),
                children: vec![ParsedItem::Record(nested_ref)],
            })],
        });
        let exterior_block = ParsedItem::Group(ParsedGroup {
            label: [0, 0, 0, 0],
            group_type: EXTERIOR_CELL_BLOCK,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(ParsedGroup {
                label: [0, 0, 0, 0],
                group_type: EXTERIOR_CELL_SUBBLOCK,
                tail: Bytes::new(),
                children: vec![ParsedItem::Record(exterior_cell), exterior_cell_children],
            })],
        });
        let world_children = ParsedItem::Group(ParsedGroup {
            label: 0x0025DA15_u32.to_le_bytes(),
            group_type: 1,
            tail: Bytes::new(),
            children: vec![exterior_block],
        });
        let plugin = plugin(vec![
            top_group("STAT", vec![ParsedItem::Record(static_base)]),
            top_group("LVLI", vec![ParsedItem::Record(leveled_base)]),
            top_group("WRLD", vec![ParsedItem::Record(world), world_children]),
        ]);
        let handle = insert_plugin_handle(plugin, LocalizedStringsState::default());
        let payload_text = plugin_handle_collect_cell_slice_roots_json(
            handle,
            "APPALACHIA",
            0,
            0,
            0,
            0,
            false,
            None,
        )
        .expect("collect roots");
        plugin_handle_close_native(handle);
        let payload: JsonValue = serde_json::from_str(&payload_text).expect("json payload");

        assert_eq!(
            payload["static_base_form_keys"],
            serde_json::json!(["SeventySix.esm:100000"])
        );
        assert_eq!(
            payload["leveled_base_entry_form_keys"],
            serde_json::json!(["SeventySix.esm:1A6663"])
        );
    }

    #[test]
    fn placement_base_collection_includes_door_bases() {
        let door = record("DOOR", 0x001A6663, Some("PaintedWoodDoorLoadD01"));
        let mut refr = record("REFR", 0x001052B66, None);
        refr.subrecords
            .push(subrecord("NAME", 0x001A6663_u32.to_le_bytes().to_vec()));
        let plugin = plugin(vec![top_group("DOOR", vec![ParsedItem::Record(door)])]);
        let locator = build_locator_section(&plugin);
        let mut base_keys = Vec::new();
        let mut base_seen = BTreeSet::new();
        let mut leveled_base_entry_keys = Vec::new();
        let mut leveled_base_entry_seen = BTreeSet::new();
        let mut layer_keys = Vec::new();
        let mut layer_seen = BTreeSet::new();

        append_static_placement_base_key(
            &plugin,
            &locator,
            &refr,
            &mut base_keys,
            &mut base_seen,
            &mut leveled_base_entry_keys,
            &mut leveled_base_entry_seen,
            &mut layer_keys,
            &mut layer_seen,
        );

        assert_eq!(base_keys, vec!["SeventySix.esm:1A6663"]);
        assert!(leveled_base_entry_keys.is_empty());
        assert!(layer_keys.is_empty());
    }

    #[test]
    fn sync_cell_locations_from_lctn_tags_projected_exterior_cells() {
        let world_form_id = 0x0725DA15;
        let location_form_id = 0x072CD2E2;
        let cell_form_id = 0x072628FE;
        let mut root_items = vec![top_group(
            "LCTN",
            vec![ParsedItem::Record(location_record(
                location_form_id,
                world_form_id,
                &[(19, -52)],
            ))],
        )];
        root_items.extend(projected_world_with_cell(
            world_form_id,
            cell_record(cell_form_id, "CellX19Y-52", 19, -52),
        ));
        let mut plugin = plugin(root_items);

        let payload = sync_cell_locations_from_lctn_world_cells(&mut plugin);

        assert_eq!(payload.locations_indexed, 1);
        assert_eq!(payload.location_conflicts, 0);
        assert_eq!(payload.cells_changed, 1);
        assert_eq!(payload.cells_retagged, 0);
        let cell = find_record_by_form_id(&plugin.root_items, cell_form_id).expect("CELL");
        let xlcn = subrecord_data(cell, "XLCN").expect("XLCN");
        assert_eq!(
            u32::from_le_bytes([xlcn[0], xlcn[1], xlcn[2], xlcn[3]]),
            location_form_id
        );
    }

    #[test]
    fn sync_cell_locations_from_lctn_repairs_mismatched_cell_location() {
        let world_form_id = 0x0725DA15;
        let location_form_id = 0x072CD2E2;
        let cell_form_id = 0x072628FE;
        let mut cell = cell_record(cell_form_id, "CellX19Y-52", 19, -52);
        cell.subrecords
            .push(subrecord("XLCN", 0x0700AAAA_u32.to_le_bytes().to_vec()));
        let mut root_items = vec![top_group(
            "LCTN",
            vec![ParsedItem::Record(location_record(
                location_form_id,
                world_form_id,
                &[(19, -52)],
            ))],
        )];
        root_items.extend(projected_world_with_cell(world_form_id, cell));
        let mut plugin = plugin(root_items);

        let payload = sync_cell_locations_from_lctn_world_cells(&mut plugin);

        assert_eq!(payload.cells_changed, 1);
        assert_eq!(payload.cells_retagged, 1);
        let cell = find_record_by_form_id(&plugin.root_items, cell_form_id).expect("CELL");
        let xlcn = subrecord_data(cell, "XLCN").expect("XLCN");
        assert_eq!(
            u32::from_le_bytes([xlcn[0], xlcn[1], xlcn[2], xlcn[3]]),
            location_form_id
        );
    }

    #[test]
    fn sync_cell_regions_from_source_tags_projected_exterior_cells() {
        let source_world_form_id: u32 = 0x0025DA15;
        let target_world_form_id: u32 = 0x0125DA15;
        let source_region_form_id: u32 = 0x00222222;
        let target_region_form_id: u32 = 0x01222222;
        let target_cell_form_id: u32 = 0x012628FE;

        let mut source_cell = cell_record(0x002628FE, "CellX19Y-52", 19, -52);
        source_cell.subrecords.push(subrecord(
            "XCLR",
            source_region_form_id.to_le_bytes().to_vec(),
        ));
        let source = plugin_with_name("SeventySix.esm", "fo76", Vec::new(), {
            let mut root_items = vec![top_group(
                "REGN",
                vec![ParsedItem::Record(record(
                    "REGN",
                    source_region_form_id,
                    Some("ForestRegion"),
                ))],
            )];
            root_items.extend(projected_world_with_cell(source_world_form_id, source_cell));
            root_items
        });

        let mut target_cell = cell_record(target_cell_form_id, "CellX19Y-52", 19, -52);
        target_cell
            .subrecords
            .push(subrecord("XLCN", 0x0100AAAA_u32.to_le_bytes().to_vec()));
        let mut target = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            vec!["Fallout4.esm".to_string()],
            {
                let mut root_items = vec![top_group(
                    "REGN",
                    vec![ParsedItem::Record(record(
                        "REGN",
                        target_region_form_id,
                        Some("ForestRegion"),
                    ))],
                )];
                root_items.extend(projected_world_with_cell(target_world_form_id, target_cell));
                root_items
            },
        );

        let payload = sync_cell_regions_from_source_worldspace(
            &source,
            &mut target,
            "APPALACHIA",
            "APPALACHIA",
        );

        assert_eq!(payload.source_cells_indexed, 1);
        assert_eq!(payload.target_cells_seen, 1);
        assert_eq!(payload.cells_changed, 1);
        assert_eq!(payload.region_refs_written, 1);
        let cell = find_record_by_form_id(&target.root_items, target_cell_form_id).expect("CELL");
        let xclr = subrecord_data(cell, "XCLR").expect("XCLR");
        assert_eq!(
            u32::from_le_bytes([xclr[0], xclr[1], xclr[2], xclr[3]]),
            target_region_form_id
        );
        let xclr_index = cell
            .subrecords
            .iter()
            .position(|subrecord| subrecord.signature.as_str() == "XCLR")
            .expect("XCLR index");
        let xlcn_index = cell
            .subrecords
            .iter()
            .position(|subrecord| subrecord.signature.as_str() == "XLCN")
            .expect("XLCN index");
        assert!(xclr_index < xlcn_index);
    }

    fn cell_mhdt_blob(fill: u8) -> Vec<u8> {
        let mut data = 128.0f32.to_le_bytes().to_vec();
        data.extend(std::iter::repeat(fill).take(1024));
        data
    }

    #[test]
    fn sync_cell_max_height_from_source_tags_projected_exterior_cells() {
        let source_world_form_id: u32 = 0x0025DA15;
        let target_world_form_id: u32 = 0x0125DA15;
        let source_cell_form_id: u32 = 0x002628FE;
        let target_cell_form_id: u32 = 0x012628FE;
        let mhdt = cell_mhdt_blob(0x42);

        let mut source_cell = cell_record(source_cell_form_id, "CellX19Y-52", 19, -52);
        source_cell.subrecords.push(subrecord("MHDT", mhdt.clone()));
        let source = plugin_with_name(
            "SeventySix.esm",
            "fo76",
            Vec::new(),
            projected_world_with_cell(source_world_form_id, source_cell),
        );

        let target_cell = cell_record(target_cell_form_id, "CellX19Y-52", 19, -52);
        let mut target = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            vec!["Fallout4.esm".to_string()],
            projected_world_with_cell(target_world_form_id, target_cell),
        );

        let payload = sync_cell_max_height_from_source_worldspace(
            &source,
            &mut target,
            "APPALACHIA",
            "APPALACHIA",
        );

        assert_eq!(payload.source_cells_indexed, 1);
        assert_eq!(payload.target_cells_seen, 1);
        assert_eq!(payload.cells_changed, 1);
        assert_eq!(payload.malformed_source_cells, 0);

        let cell = find_record_by_form_id(&target.root_items, target_cell_form_id).expect("CELL");
        let out = subrecord_data(cell, "MHDT").expect("MHDT");
        assert_eq!(out.as_ref(), mhdt.as_slice());
        let mhdt_index = cell
            .subrecords
            .iter()
            .position(|subrecord| subrecord.signature.as_str() == "MHDT")
            .expect("MHDT index");
        let xclc_index = cell
            .subrecords
            .iter()
            .position(|subrecord| subrecord.signature.as_str() == "XCLC")
            .expect("XCLC index");
        assert!(xclc_index < mhdt_index);
    }

    #[test]
    fn sync_cell_max_height_skips_malformed_source() {
        let source_world_form_id: u32 = 0x0025DA15;
        let target_world_form_id: u32 = 0x0125DA15;
        let mut source_cell = cell_record(0x002628FE, "CellX19Y-52", 19, -52);
        let mut short = cell_mhdt_blob(0);
        short.truncate(1027);
        source_cell.subrecords.push(subrecord("MHDT", short));
        let source = plugin_with_name(
            "SeventySix.esm",
            "fo76",
            Vec::new(),
            projected_world_with_cell(source_world_form_id, source_cell),
        );

        let target_cell = cell_record(0x012628FE, "CellX19Y-52", 19, -52);
        let mut target = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            vec!["Fallout4.esm".to_string()],
            projected_world_with_cell(target_world_form_id, target_cell),
        );

        let payload = sync_cell_max_height_from_source_worldspace(
            &source,
            &mut target,
            "APPALACHIA",
            "APPALACHIA",
        );

        assert_eq!(payload.source_cells_indexed, 0);
        assert_eq!(payload.malformed_source_cells, 1);
        assert_eq!(payload.cells_changed, 0);
        let cell = find_record_by_form_id(&target.root_items, 0x012628FE).expect("CELL");
        assert!(subrecord_data(cell, "MHDT").is_none());
    }

    #[test]
    fn placement_base_collection_includes_actor_bases() {
        let npc = record("NPC_", 0x00123456, Some("AppalachiaActorBase"));
        let mut achr = record("ACHR", 0x0010AA00, None);
        achr.subrecords
            .push(subrecord("NAME", 0x00123456_u32.to_le_bytes().to_vec()));
        let plugin = plugin(vec![top_group("NPC_", vec![ParsedItem::Record(npc)])]);
        let locator = build_locator_section(&plugin);
        let mut base_keys = Vec::new();
        let mut base_seen = BTreeSet::new();
        let mut leveled_base_entry_keys = Vec::new();
        let mut leveled_base_entry_seen = BTreeSet::new();
        let mut layer_keys = Vec::new();
        let mut layer_seen = BTreeSet::new();

        append_static_placement_base_key(
            &plugin,
            &locator,
            &achr,
            &mut base_keys,
            &mut base_seen,
            &mut leveled_base_entry_keys,
            &mut leveled_base_entry_seen,
            &mut layer_keys,
            &mut layer_seen,
        );

        assert_eq!(base_keys, vec!["SeventySix.esm:123456"]);
        assert!(leveled_base_entry_keys.is_empty());
        assert!(layer_keys.is_empty());
    }

    #[test]
    fn placed_child_local_ref_rewrite_maps_xlkr_keyword_and_ref() {
        let source = plugin(Vec::new());
        let target = TargetFormIdContext {
            plugin_name: "B21_Appalachia.esp".to_string(),
            masters: vec!["Fallout4.esm".to_string()],
            own_prefix: 0x0100_0000,
        };
        let target_plugin = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            target.masters.clone(),
            Vec::new(),
        );
        let target_locator = build_locator_section(&target_plugin);
        let mut achr = record("ACHR", 0x0055CBBB, None);
        let mut xlkr = Vec::new();
        xlkr.extend_from_slice(&0x0055DE21_u32.to_le_bytes());
        xlkr.extend_from_slice(&0x0058DBBB_u32.to_le_bytes());
        achr.subrecords.push(subrecord("XLKR", xlkr));

        let changed = rewrite_placed_child_local_refs(
            &mut achr,
            &source,
            0,
            &target,
            &target_locator,
            &BTreeMap::new(),
        );

        assert_eq!(changed, 2);
        let data = subrecord_data(&achr, "XLKR").expect("XLKR");
        assert_eq!(
            u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            0x0155DE21
        );
        assert_eq!(
            u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            0x0158DBBB
        );
    }

    #[test]
    fn placed_child_local_ref_rewrite_maps_xndp_offset0_only() {
        let source = plugin(Vec::new());
        let target = TargetFormIdContext {
            plugin_name: "B21_Appalachia.esp".to_string(),
            masters: vec!["Fallout4.esm".to_string()],
            own_prefix: 0x0100_0000,
        };
        let target_plugin = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            target.masters.clone(),
            Vec::new(),
        );
        let target_locator = build_locator_section(&target_plugin);
        let mut refr = record("REFR", 0x0055CBBB, None);
        // struct:I,h,B,B — formid at offset 0, int16 triangle index at offset 4,
        // two trailing bytes. Only offset 0 may be remapped.
        let mut xndp = Vec::new();
        xndp.extend_from_slice(&0x0049994D_u32.to_le_bytes());
        xndp.extend_from_slice(&0x1234_u16.to_le_bytes());
        xndp.push(0xAB);
        xndp.push(0xCD);
        refr.subrecords.push(subrecord("XNDP", xndp));

        let changed = rewrite_placed_child_local_refs(
            &mut refr,
            &source,
            0,
            &target,
            &target_locator,
            &BTreeMap::new(),
        );

        assert_eq!(changed, 1);
        let data = subrecord_data(&refr, "XNDP").expect("XNDP");
        assert_eq!(
            u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            0x0149994D
        );
        // Offset 4 (int16 triangle index) and trailing bytes untouched.
        assert_eq!(u16::from_le_bytes([data[4], data[5]]), 0x1234);
        assert_eq!(data[6], 0xAB);
        assert_eq!(data[7], 0xCD);
    }

    #[test]
    fn placed_child_local_ref_rewrite_maps_xtnm_offset0() {
        let source = plugin(Vec::new());
        let target = TargetFormIdContext {
            plugin_name: "B21_Appalachia.esp".to_string(),
            masters: vec!["Fallout4.esm".to_string()],
            own_prefix: 0x0100_0000,
        };
        let target_plugin = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            target.masters.clone(),
            Vec::new(),
        );
        let target_locator = build_locator_section(&target_plugin);
        let mut refr = record("REFR", 0x0055CBBB, None);
        // XTNM is a bare 4-byte FormID (Teleport Loc Name → MESG). FO76 master
        // byte 00 must be remapped to 07 (own-plugin index after copy).
        refr.subrecords
            .push(subrecord("XTNM", 0x0032BE70_u32.to_le_bytes().to_vec()));

        let changed = rewrite_placed_child_local_refs(
            &mut refr,
            &source,
            0,
            &target,
            &target_locator,
            &BTreeMap::new(),
        );

        assert_eq!(changed, 1);
        let data = subrecord_data(&refr, "XTNM").expect("XTNM");
        assert_eq!(
            u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            0x0132BE70
        );
    }

    #[test]
    fn placed_child_local_ref_rewrite_maps_reference_group() {
        let source = plugin(Vec::new());
        let target = TargetFormIdContext {
            plugin_name: "B21_Appalachia.esp".to_string(),
            masters: vec!["Fallout4.esm".to_string()],
            own_prefix: 0x0100_0000,
        };
        let target_plugin = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            target.masters.clone(),
            Vec::new(),
        );
        let target_locator = build_locator_section(&target_plugin);
        let mut refr = record("REFR", 0x0055CBBB, None);
        refr.subrecords
            .push(subrecord("XRFG", 0x001B1A6E_u32.to_le_bytes().to_vec()));

        let changed = rewrite_placed_child_local_refs(
            &mut refr,
            &source,
            0,
            &target,
            &target_locator,
            &BTreeMap::new(),
        );

        assert_eq!(changed, 1);
        let data = subrecord_data(&refr, "XRFG").expect("XRFG");
        assert_eq!(
            u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            0x011B1A6E
        );
    }

    #[test]
    fn placed_child_local_ref_rewrite_maps_activate_parent() {
        let source = plugin(Vec::new());
        let target = TargetFormIdContext {
            plugin_name: "B21_Appalachia.esp".to_string(),
            masters: vec!["Fallout4.esm".to_string()],
            own_prefix: 0x0100_0000,
        };
        let target_plugin = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            target.masters.clone(),
            Vec::new(),
        );
        let target_locator = build_locator_section(&target_plugin);
        let mut achr = record("ACHR", 0x0055CBBB, None);
        let mut xapr = Vec::new();
        xapr.extend_from_slice(&0x0081CC71_u32.to_le_bytes());
        xapr.extend_from_slice(&0_u32.to_le_bytes());
        achr.subrecords.push(subrecord("XAPR", xapr));

        let changed = rewrite_placed_child_local_refs(
            &mut achr,
            &source,
            0,
            &target,
            &target_locator,
            &BTreeMap::new(),
        );

        assert_eq!(changed, 1);
        let data = subrecord_data(&achr, "XAPR").expect("XAPR");
        assert_eq!(
            u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            0x0181CC71
        );
    }

    #[test]
    fn placed_child_local_ref_rewrite_maps_xloc_key_offset4() {
        let source = plugin(Vec::new());
        let target = TargetFormIdContext {
            plugin_name: "B21_Appalachia.esp".to_string(),
            masters: vec!["Fallout4.esm".to_string()],
            own_prefix: 0x0100_0000,
        };
        let target_plugin = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            target.masters.clone(),
            Vec::new(),
        );
        let target_locator = build_locator_section(&target_plugin);
        let mut refr = record("REFR", 0x0055CBBB, None);
        let mut xloc = vec![0, 0, 0, 0];
        xloc.extend_from_slice(&0x0055ADA7_u32.to_le_bytes());
        xloc.extend_from_slice(&[1, 2, 3, 4]);
        refr.subrecords.push(subrecord("XLOC", xloc));

        let changed = rewrite_placed_child_local_refs(
            &mut refr,
            &source,
            0,
            &target,
            &target_locator,
            &BTreeMap::new(),
        );

        assert_eq!(changed, 1);
        let data = subrecord_data(&refr, "XLOC").expect("XLOC");
        assert_eq!(&data[0..4], &[0, 0, 0, 0]);
        assert_eq!(
            u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            0x0155ADA7
        );
        assert_eq!(&data[8..12], &[1, 2, 3, 4]);
    }

    #[test]
    fn placed_child_local_ref_rewrite_maps_xplk_ref_offset0() {
        let source = plugin(Vec::new());
        let target = TargetFormIdContext {
            plugin_name: "B21_Appalachia.esp".to_string(),
            masters: vec!["Fallout4.esm".to_string()],
            own_prefix: 0x0100_0000,
        };
        let target_plugin = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            target.masters.clone(),
            Vec::new(),
        );
        let target_locator = build_locator_section(&target_plugin);
        let mut refr = record("REFR", 0x0055CBBB, None);
        let mut xplk = Vec::new();
        xplk.extend_from_slice(&0x0039E691_u32.to_le_bytes());
        xplk.extend_from_slice(&0xAABBCCDD_u32.to_le_bytes());
        refr.subrecords.push(subrecord("XPLK", xplk));

        let changed = rewrite_placed_child_local_refs(
            &mut refr,
            &source,
            0,
            &target,
            &target_locator,
            &BTreeMap::new(),
        );

        assert_eq!(changed, 1);
        let data = subrecord_data(&refr, "XPLK").expect("XPLK");
        assert_eq!(
            u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            0x0139E691
        );
        assert_eq!(
            u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            0xAABBCCDD,
            "XPLK trailing bytes are not a FormID"
        );
    }

    #[test]
    fn placed_child_local_ref_rewrite_maps_material_location_and_ref_types() {
        let source = plugin_with_name(
            "SeventySix.esm",
            "fo76",
            vec!["Fallout76.esm".to_string()],
            Vec::new(),
        );
        let target = TargetFormIdContext {
            plugin_name: "B21_Appalachia.esp".to_string(),
            masters: vec!["Fallout4.esm".to_string()],
            own_prefix: 0x0100_0000,
        };
        let target_plugin = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            target.masters.clone(),
            vec![
                top_group(
                    "MSWP",
                    vec![ParsedItem::Record(record("MSWP", 0x01114118, None))],
                ),
                top_group(
                    "LCTN",
                    vec![ParsedItem::Record(record("LCTN", 0x012CD2E2, None))],
                ),
                top_group(
                    "ECZN",
                    vec![ParsedItem::Record(record("ECZN", 0x01358545, None))],
                ),
                top_group(
                    "CELL",
                    vec![ParsedItem::Record(record("CELL", 0x01262208, None))],
                ),
                top_group(
                    "LCRT",
                    vec![
                        ParsedItem::Record(record("LCRT", 0x01111111, None)),
                        ParsedItem::Record(record("LCRT", 0x01222222, None)),
                    ],
                ),
            ],
        );
        let target_locator = build_locator_section(&target_plugin);
        let mut refr = record("REFR", 0x0055CBBB, None);
        refr.subrecords
            .push(subrecord("XMSP", 0x00114118_u32.to_le_bytes().to_vec()));
        refr.subrecords
            .push(subrecord("XLCN", 0x002CD2E2_u32.to_le_bytes().to_vec()));
        refr.subrecords
            .push(subrecord("XEZN", 0x00358545_u32.to_le_bytes().to_vec()));
        refr.subrecords
            .push(subrecord("XCZC", 0x00262208_u32.to_le_bytes().to_vec()));
        let mut xlrt = Vec::new();
        xlrt.extend_from_slice(&0x00111111_u32.to_le_bytes());
        xlrt.extend_from_slice(&0x00222222_u32.to_le_bytes());
        refr.subrecords.push(subrecord("XLRT", xlrt));

        let changed = rewrite_placed_child_local_refs(
            &mut refr,
            &source,
            1,
            &target,
            &target_locator,
            &BTreeMap::new(),
        );

        // changed still 6: the XEZN strip (+1) replaces the XEZN remap (+1);
        // XMSP/XLCN/XCZC (+1 each) + XLRT (+2) = 5; total 6.
        assert_eq!(changed, 6);
        let xmsp = subrecord_data(&refr, "XMSP").expect("XMSP");
        let xlcn = subrecord_data(&refr, "XLCN").expect("XLCN");
        let xczc = subrecord_data(&refr, "XCZC").expect("XCZC");
        let xlrt = subrecord_data(&refr, "XLRT").expect("XLRT");
        // FO4 placed-ref XEZN must be ECZN; with no ECZN to map to, XEZN is
        // stripped from copied REFR/ACHR/PGRE rather than left pointing at a LCTN.
        assert!(
            subrecord_data(&refr, "XEZN").is_none(),
            "REFR XEZN must be stripped (no valid FO4 ECZN target)"
        );
        assert_eq!(
            u32::from_le_bytes([xmsp[0], xmsp[1], xmsp[2], xmsp[3]]),
            0x01114118
        );
        assert_eq!(
            u32::from_le_bytes([xlcn[0], xlcn[1], xlcn[2], xlcn[3]]),
            0x012CD2E2
        );
        assert_eq!(
            u32::from_le_bytes([xczc[0], xczc[1], xczc[2], xczc[3]]),
            0x01262208
        );
        assert_eq!(
            u32::from_le_bytes([xlrt[0], xlrt[1], xlrt[2], xlrt[3]]),
            0x01111111
        );
        assert_eq!(
            u32::from_le_bytes([xlrt[4], xlrt[5], xlrt[6], xlrt[7]]),
            0x01222222
        );
    }

    #[test]
    fn placed_child_xezn_is_stripped_for_achr_and_pgre() {
        // FO4 placed-ref XEZN must be an ECZN; FO76 has none and we synthesize
        // none, so XEZN is stripped from every copied placed sig (ACHR/PGRE too,
        // not just REFR) rather than left pointing at a LCTN.
        let source = plugin(Vec::new());
        let target = TargetFormIdContext {
            plugin_name: "B21_Appalachia.esp".to_string(),
            masters: vec!["Fallout4.esm".to_string()],
            own_prefix: 0x0100_0000,
        };
        let target_plugin = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            target.masters.clone(),
            Vec::new(),
        );
        let target_locator = build_locator_section(&target_plugin);
        for sig in ["ACHR", "PGRE"] {
            let mut rec = record(sig, 0x0055CBBB, None);
            rec.subrecords
                .push(subrecord("XEZN", 0x00358545_u32.to_le_bytes().to_vec()));
            let changed = rewrite_placed_child_local_refs(
                &mut rec,
                &source,
                1,
                &target,
                &target_locator,
                &BTreeMap::new(),
            );
            assert_eq!(changed, 1, "{sig} XEZN strip should count one change");
            assert!(
                subrecord_data(&rec, "XEZN").is_none(),
                "{sig} XEZN must be stripped"
            );
        }
    }

    #[test]
    fn placed_child_empty_xrgd_is_stripped_from_refr_only() {
        let source = plugin(Vec::new());
        let target = TargetFormIdContext {
            plugin_name: "B21_Appalachia.esp".to_string(),
            masters: vec!["Fallout4.esm".to_string()],
            own_prefix: 0x0100_0000,
        };
        let target_plugin = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            target.masters.clone(),
            Vec::new(),
        );
        let target_locator = build_locator_section(&target_plugin);

        let mut refr = record("REFR", 0x0055CBBB, None);
        let mut empty_xrgd = vec![0; 28];
        empty_xrgd[19] = 0x80;
        empty_xrgd[23] = 0x80;
        empty_xrgd[27] = 0x80;
        refr.subrecords.push(subrecord("XRGD", empty_xrgd.clone()));
        let changed = rewrite_placed_child_local_refs(
            &mut refr,
            &source,
            1,
            &target,
            &target_locator,
            &BTreeMap::new(),
        );
        assert_eq!(changed, 1);
        assert!(subrecord_data(&refr, "XRGD").is_none());

        let mut nonzero_refr = record("REFR", 0x0055CBBC, None);
        let mut xrgd = vec![0; 28];
        xrgd[4] = 1;
        nonzero_refr.subrecords.push(subrecord("XRGD", xrgd));
        let changed = rewrite_placed_child_local_refs(
            &mut nonzero_refr,
            &source,
            1,
            &target,
            &target_locator,
            &BTreeMap::new(),
        );
        assert_eq!(changed, 0);
        assert!(subrecord_data(&nonzero_refr, "XRGD").is_some());

        let mut achr = record("ACHR", 0x0055CBBD, None);
        achr.subrecords.push(subrecord("XRGD", empty_xrgd));
        let changed = rewrite_placed_child_local_refs(
            &mut achr,
            &source,
            1,
            &target,
            &target_locator,
            &BTreeMap::new(),
        );
        assert_eq!(changed, 0);
        assert!(subrecord_data(&achr, "XRGD").is_some());
    }

    #[test]
    fn placed_child_local_ref_drop_keeps_xlkr_for_postcopy_validation() {
        // XLKR is deferred to post-copy `normalize_placed_records`: the per-pass
        // `target_existing_form_ids` snapshot can't see linked refs copied in a
        // different pass, so dropping here strands actor sleep/sandbox packages.
        let target = TargetFormIdContext {
            plugin_name: "B21_Appalachia.esp".to_string(),
            masters: vec!["Fallout4.esm".to_string()],
            own_prefix: 0x0100_0000,
        };
        let mut existing = BTreeSet::new();
        existing.insert(0x01195411);
        let mut refr = record("REFR", 0x014EA534, None);
        let mut xlkr = Vec::new();
        xlkr.extend_from_slice(&0x01195411_u32.to_le_bytes());
        // Linked ref absent from THIS pass's snapshot (copied in another pass).
        xlkr.extend_from_slice(&0x013BEF28_u32.to_le_bytes());
        refr.subrecords.push(subrecord("XLKR", xlkr));

        let removed = drop_unresolved_placed_child_local_refs(&mut refr, &target, &existing);

        assert_eq!(removed, 0);
        assert!(subrecord_data(&refr, "XLKR").is_some());
    }

    #[test]
    fn placed_child_missing_local_ref_drop_defers_activate_parent_refs() {
        let target = TargetFormIdContext {
            plugin_name: "B21_Appalachia.esp".to_string(),
            masters: vec!["Fallout4.esm".to_string()],
            own_prefix: 0x0100_0000,
        };
        let existing = BTreeSet::new();
        let mut refr = record("REFR", 0x014EA534, None);
        let mut xesp = Vec::new();
        xesp.extend_from_slice(&0x014337F1_u32.to_le_bytes());
        xesp.extend_from_slice(&0_u32.to_le_bytes());
        let mut xapr = Vec::new();
        xapr.extend_from_slice(&0x0181CC71_u32.to_le_bytes());
        xapr.extend_from_slice(&0_u32.to_le_bytes());
        refr.subrecords.push(subrecord("XESP", xesp));
        refr.subrecords.push(subrecord("XAPR", xapr));

        let removed = drop_unresolved_placed_child_local_refs(&mut refr, &target, &existing);

        assert_eq!(removed, 1);
        assert!(subrecord_data(&refr, "XESP").is_none());
        assert!(subrecord_data(&refr, "XAPR").is_some());
    }

    #[test]
    fn placed_child_missing_local_ref_drop_removes_unresolved_material_and_ref_types() {
        let target = TargetFormIdContext {
            plugin_name: "B21_Appalachia.esp".to_string(),
            masters: vec!["Fallout4.esm".to_string()],
            own_prefix: 0x0100_0000,
        };
        let existing = BTreeSet::new();
        let mut refr = record("REFR", 0x014EA534, None);
        refr.subrecords
            .push(subrecord("XMSP", 0x01114118_u32.to_le_bytes().to_vec()));
        let mut xlrt = Vec::new();
        xlrt.extend_from_slice(&0x01000001_u32.to_le_bytes());
        xlrt.extend_from_slice(&0x01000002_u32.to_le_bytes());
        refr.subrecords.push(subrecord("XLRT", xlrt));

        let removed = drop_unresolved_placed_child_local_refs(&mut refr, &target, &existing);

        assert_eq!(removed, 2);
        assert!(subrecord_data(&refr, "XMSP").is_none());
        assert!(subrecord_data(&refr, "XLRT").is_none());
    }

    /// The parallel prepare kernel must reproduce the serial loop's
    /// output exactly — bucket keys, per-bucket record order and content, every
    /// counter, warning order, and the skipped-children sample order.
    #[test]
    fn parallel_prepare_matches_serial_prepare() {
        // Source: 72 placed REFRs (over the 64 parallel threshold) + one LVLI
        // base record without entries (unresolved_lvli_base path).
        let mut source_children: Vec<ParsedItem> = Vec::new();
        for i in 0..72u32 {
            let mut placed = record("REFR", 0x01100000 + i, None);
            let name_raw: u32 = match i {
                7 => 0x01DEAD99,  // own-plugin base that exists nowhere -> missing_base
                11 => 0x01200000, // own-plugin LVLI without entries -> unresolved_lvli_base
                _ => 0x011A6663,  // mapped via form_key_map to a target-master base
            };
            placed
                .subrecords
                .push(subrecord("NAME", name_raw.to_le_bytes().to_vec()));
            if i == 13 {
                // linked-ref + layer subrecords exercise rewrite_placed_child_local_refs
                let mut xlkr = Vec::new();
                xlkr.extend_from_slice(&0x011A6663_u32.to_le_bytes());
                xlkr.extend_from_slice(&(0x01100000_u32 + 1).to_le_bytes());
                placed.subrecords.push(subrecord("XLKR", xlkr));
            }
            // i==24 is listed under cell (0,0) but positioned in grid (1,0)
            // (after the +2048 offset) -> rebuckets to cell 0x01000801.
            let x: f32 = if i == 24 { 4500.0 } else { 10.0 + i as f32 };
            let mut data = Vec::new();
            for value in [x, 20.0, 30.0, 0.0, 0.0, 0.0] {
                data.extend_from_slice(&value.to_le_bytes());
            }
            placed.subrecords.push(subrecord("DATA", data));
            source_children.push(ParsedItem::Record(placed));
        }
        let lvli = record("LVLI", 0x01200000, Some("EmptyLeveled"));
        let source = plugin_with_name(
            "SeventySix.esm",
            "fo76",
            vec!["Fallout76.esm".to_string()],
            vec![
                top_group("REFR", source_children),
                top_group("LVLI", vec![ParsedItem::Record(lvli)]),
            ],
        );

        // Target: three gridded CELLs so rebucketing has a destination.
        let target = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            vec!["Fallout4.esm".to_string()],
            vec![top_group(
                "CELL",
                vec![
                    ParsedItem::Record(cell_record(0x01000800, "Cell00", 0, 0)),
                    ParsedItem::Record(cell_record(0x01000801, "Cell10", 1, 0)),
                    ParsedItem::Record(cell_record(0x01000802, "Cell01", 0, 1)),
                ],
            )],
        );

        let source_locator = build_locator_section(&source);
        let target_locator = build_locator_section(&target);
        let target_ctx = TargetFormIdContext {
            plugin_name: target.plugin_name.clone(),
            masters: target.header.masters.clone(),
            own_prefix: local_form_prefix(&target),
        };
        let mut target_existing_form_ids = BTreeSet::new();
        collect_record_form_ids(&target.root_items, &mut target_existing_form_ids);
        let mut target_cell_by_grid = BTreeMap::new();
        collect_target_cell_by_grid(&target.root_items, &mut target_cell_by_grid);
        // Keys in normalized (lowercase-plugin) form — the JSON entry path
        // normalizes the incoming map before the prepare loop sees it.
        let form_key_map: BTreeMap<String, String> = BTreeMap::from([(
            "seventysix.esm:1A6663".to_string(),
            "Fallout4.esm:1A6663".to_string(),
        )]);
        let ctx = CopyCellChildrenContext {
            source_plugin: &source,
            source_locator: &source_locator,
            source_own_index: 1,
            target: &target_ctx,
            target_game: Some("fo4"),
            target_locator: &target_locator,
            target_existing_form_ids: &target_existing_form_ids,
            target_cell_by_grid: &target_cell_by_grid,
            form_key_map: &form_key_map,
            header_size: MODERN_HEADER_SIZE,
            offset: (2048.0, 2048.0, 0.0),
        };

        let build_children = || -> BTreeMap<String, CellChildrenPayload> {
            let mut cell0_persistent = Vec::new();
            let mut cell0_temporary = Vec::new();
            let mut cell1_temporary = Vec::new();
            for i in 0..72u32 {
                let key = format!("B21_Appalachia.esp:{:06X}", 0x100000 + i);
                match i % 3 {
                    0 => cell0_persistent.push(key),
                    1 => cell0_temporary.push(key),
                    _ => cell1_temporary.push(key),
                }
            }
            BTreeMap::from([
                (
                    "B21_Appalachia.esp:000800".to_string(),
                    CellChildrenPayload {
                        persistent: cell0_persistent,
                        temporary: cell0_temporary,
                    },
                ),
                (
                    "B21_Appalachia.esp:000801".to_string(),
                    CellChildrenPayload {
                        persistent: Vec::new(),
                        temporary: cell1_temporary,
                    },
                ),
                (
                    "not-a-key".to_string(),
                    CellChildrenPayload {
                        persistent: vec!["B21_Appalachia.esp:100000".to_string()],
                        temporary: Vec::new(),
                    },
                ),
            ])
        };

        fn flatten(prepared: &BTreeMap<u32, PreparedCellChildren>) -> String {
            let mut out = String::new();
            for (cell_id, children) in prepared {
                out.push_str(&format!("{cell_id:08X}\n"));
                for (label, records) in [("P", &children.persistent), ("T", &children.temporary)] {
                    for record in records {
                        out.push_str(&format!("  {label} {:08X}", record.form_id));
                        for sub in &record.subrecords {
                            out.push_str(&format!(" {}:{:02X?}", sub.signature, sub.data.as_ref()));
                        }
                        out.push('\n');
                    }
                }
            }
            out
        }

        let mut payload_par = CellSliceInsertPayload::default();
        let prepared_par = prepare_source_children_for_target_cells_with_threshold(
            build_children(),
            &ctx,
            &mut payload_par,
            0, // always parallel
        );
        let mut payload_ser = CellSliceInsertPayload::default();
        let prepared_ser = prepare_source_children_for_target_cells_with_threshold(
            build_children(),
            &ctx,
            &mut payload_ser,
            usize::MAX, // always serial (the legacy loop shape)
        );

        assert_eq!(
            flatten(&prepared_par),
            flatten(&prepared_ser),
            "bucket keys + per-bucket record order/content must be identical"
        );
        assert_eq!(
            serde_json::to_string(&payload_par).unwrap(),
            serde_json::to_string(&payload_ser).unwrap(),
            "payload counters, warnings, and skip samples must be identical"
        );
        // Fixture sanity: the interesting paths actually fired. Rebucketed=24:
        // the 23 children listed under cell (1,0) are positioned in grid (0,0),
        // plus the one cell-(0,0) child positioned in grid (1,0).
        let dump = serde_json::to_string(&payload_ser).unwrap();
        assert_eq!(payload_ser.children_rebucketed, 24, "payload: {dump}");
        assert_eq!(payload_ser.missing_base_children, 2, "payload: {dump}");
        assert_eq!(payload_ser.skipped_children.len(), 2, "payload: {dump}");
        assert_eq!(payload_ser.warnings.len(), 1, "payload: {dump}");
        assert!(payload_ser.mapped_form_refs > 0, "payload: {dump}");
    }

    #[test]
    fn copy_cell_slice_children_clones_source_records_into_target_cells() {
        let mut source_ref = record("REFR", 0x014EA534, None);
        source_ref
            .subrecords
            .push(subrecord("NAME", 0x011A6663_u32.to_le_bytes().to_vec()));
        source_ref
            .subrecords
            .push(subrecord("XALG", vec![0, 2, 0, 0, 0, 0, 0, 0]));
        let mut data = Vec::new();
        for value in [10.0_f32, 20.0, 30.0, 0.0, 0.0, 0.0] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        source_ref.subrecords.push(subrecord("DATA", data));
        let source = plugin_with_name(
            "SeventySix.esm",
            "fo76",
            vec!["Fallout76.esm".to_string()],
            vec![top_group("REFR", vec![ParsedItem::Record(source_ref)])],
        );

        let target_cell_group = ParsedItem::Group(ParsedGroup {
            label: 0x012628FE_u32.to_le_bytes(),
            group_type: CELL_CHILD_GROUP,
            tail: Bytes::new(),
            children: Vec::new(),
        });
        let target = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            vec!["Fallout4.esm".to_string()],
            vec![target_cell_group],
        );
        let source_handle = insert_plugin_handle(source, LocalizedStringsState::default());
        let target_handle = insert_plugin_handle(target, LocalizedStringsState::default());
        let children = serde_json::json!({
            "B21_Appalachia.esp:2628FE": {
                "Persistent": [],
                "Temporary": ["B21_Appalachia.esp:4EA534"]
            }
        });
        let form_key_map = serde_json::json!({
            "SeventySix.esm:1A6663": "Fallout4.esm:1A6663"
        });

        let payload_text = plugin_handle_copy_cell_slice_children_json(
            source_handle,
            target_handle,
            &children.to_string(),
            2048.0,
            2048.0,
            0.0,
            Some(&form_key_map.to_string()),
        )
        .expect("copy children");
        let payload: JsonValue = serde_json::from_str(&payload_text).expect("json payload");
        assert_eq!(payload["children_inserted"], serde_json::json!(1));
        assert_eq!(payload["mapped_form_refs"], serde_json::json!(1));
        assert_eq!(payload["schema_subrecords_dropped"], serde_json::json!(1));

        let store = plugin_handle_store_ref().lock().unwrap();
        let target_slot = store.get(&target_handle).expect("target handle");
        let ParsedItem::Group(cell_group) = &target_slot.parsed.root_items[0] else {
            panic!("expected cell child group");
        };
        let ParsedItem::Group(temp_group) = &cell_group.children[1] else {
            panic!("expected temporary group");
        };
        let ParsedItem::Record(inserted) = &temp_group.children[0] else {
            panic!("expected inserted record");
        };
        assert_eq!(inserted.form_id, 0x014EA534);
        assert_eq!(
            inserted.flags & RECORD_FLAG_VISIBLE_WHEN_DISTANT,
            RECORD_FLAG_VISIBLE_WHEN_DISTANT,
            "FO76 REFR XALG VisibleDistant must carry into the FO4 header flag"
        );
        let name = subrecord_data(inserted, "NAME").expect("NAME");
        assert_eq!(
            u32::from_le_bytes([name[0], name[1], name[2], name[3]]),
            0x001A6663
        );
        assert!(subrecord_data(inserted, "XALG").is_none());
        let placed_data = subrecord_data(inserted, "DATA").expect("DATA");
        assert_eq!(
            f32::from_le_bytes([
                placed_data[0],
                placed_data[1],
                placed_data[2],
                placed_data[3]
            ]),
            2058.0
        );
        assert_eq!(
            f32::from_le_bytes([
                placed_data[4],
                placed_data[5],
                placed_data[6],
                placed_data[7]
            ]),
            2068.0
        );
        drop(store);
        plugin_handle_close_native(source_handle);
        plugin_handle_close_native(target_handle);
    }

    #[test]
    fn copy_cell_slice_children_rebuckets_offset_refs_to_adjusted_cell() {
        let mut source_ref = record("REFR", 0x014EA534, None);
        source_ref
            .subrecords
            .push(subrecord("NAME", 0x011A6663_u32.to_le_bytes().to_vec()));
        let mut data = Vec::new();
        for value in [4090.0_f32, 20.0, 30.0, 0.0, 0.0, 0.0] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        source_ref.subrecords.push(subrecord("DATA", data));
        let source = plugin_with_name(
            "SeventySix.esm",
            "fo76",
            vec!["Fallout76.esm".to_string()],
            vec![top_group("REFR", vec![ParsedItem::Record(source_ref)])],
        );

        let target_cell_0 = cell_record(0x01000800, "Cell00", 0, 0);
        let target_cell_1 = cell_record(0x01000801, "Cell10", 1, 0);
        let target_cell_group_0 = ParsedItem::Group(ParsedGroup {
            label: 0x01000800_u32.to_le_bytes(),
            group_type: CELL_CHILD_GROUP,
            tail: Bytes::new(),
            children: Vec::new(),
        });
        let target_cell_group_1 = ParsedItem::Group(ParsedGroup {
            label: 0x01000801_u32.to_le_bytes(),
            group_type: CELL_CHILD_GROUP,
            tail: Bytes::new(),
            children: Vec::new(),
        });
        let target = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            vec!["Fallout4.esm".to_string()],
            vec![
                top_group(
                    "CELL",
                    vec![
                        ParsedItem::Record(target_cell_0),
                        ParsedItem::Record(target_cell_1),
                    ],
                ),
                target_cell_group_0,
                target_cell_group_1,
            ],
        );
        let source_handle = insert_plugin_handle(source, LocalizedStringsState::default());
        let target_handle = insert_plugin_handle(target, LocalizedStringsState::default());
        let children = serde_json::json!({
            "B21_Appalachia.esp:000800": {
                "Persistent": [],
                "Temporary": ["B21_Appalachia.esp:4EA534"]
            }
        });
        let form_key_map = serde_json::json!({
            "SeventySix.esm:1A6663": "Fallout4.esm:1A6663"
        });

        let payload_text = plugin_handle_copy_cell_slice_children_json(
            source_handle,
            target_handle,
            &children.to_string(),
            20.0,
            0.0,
            0.0,
            Some(&form_key_map.to_string()),
        )
        .expect("copy children");
        let payload: JsonValue = serde_json::from_str(&payload_text).expect("json payload");
        assert_eq!(payload["children_inserted"], serde_json::json!(1));
        assert_eq!(payload["children_rebucketed"], serde_json::json!(1));

        let store = plugin_handle_store_ref().lock().unwrap();
        let target_slot = store.get(&target_handle).expect("target handle");
        let ParsedItem::Group(original_cell_group) = &target_slot.parsed.root_items[1] else {
            panic!("expected original cell child group");
        };
        assert!(original_cell_group.children.is_empty());

        let ParsedItem::Group(adjusted_cell_group) = &target_slot.parsed.root_items[2] else {
            panic!("expected adjusted cell child group");
        };
        let ParsedItem::Group(temp_group) = &adjusted_cell_group.children[1] else {
            panic!("expected temporary group");
        };
        let ParsedItem::Record(inserted) = &temp_group.children[0] else {
            panic!("expected inserted record");
        };
        let placed_data = subrecord_data(inserted, "DATA").expect("DATA");
        assert_eq!(
            f32::from_le_bytes([
                placed_data[0],
                placed_data[1],
                placed_data[2],
                placed_data[3]
            ]),
            4110.0
        );
        drop(store);
        plugin_handle_close_native(source_handle);
        plugin_handle_close_native(target_handle);
    }

    #[test]
    fn copy_cell_slice_children_reallocates_colliding_child_form_ids() {
        let mut first_ref = record("REFR", 0x014EA534, None);
        first_ref
            .subrecords
            .push(subrecord("NAME", 0x011A6663_u32.to_le_bytes().to_vec()));
        let mut second_ref = record("REFR", 0x014EA535, None);
        second_ref
            .subrecords
            .push(subrecord("NAME", 0x011A6663_u32.to_le_bytes().to_vec()));
        second_ref
            .subrecords
            .push(subrecord("XLRL", 0x014EA534_u32.to_le_bytes().to_vec()));
        let source = plugin_with_name(
            "SeventySix.esm",
            "fo76",
            vec!["Fallout76.esm".to_string()],
            vec![top_group(
                "REFR",
                vec![
                    ParsedItem::Record(first_ref),
                    ParsedItem::Record(second_ref),
                ],
            )],
        );

        let target_cell_group = ParsedItem::Group(ParsedGroup {
            label: 0x012628FE_u32.to_le_bytes(),
            group_type: CELL_CHILD_GROUP,
            tail: Bytes::new(),
            children: Vec::new(),
        });
        let existing_collision = record("STAT", 0x014EA534, Some("ExistingCollision"));
        let high_water = record("GLOB", 0x018D804D, Some("HighWater"));
        let target = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            vec!["Fallout4.esm".to_string()],
            vec![
                target_cell_group,
                top_group("STAT", vec![ParsedItem::Record(existing_collision)]),
                top_group("GLOB", vec![ParsedItem::Record(high_water)]),
            ],
        );
        let source_handle = insert_plugin_handle(source, LocalizedStringsState::default());
        let target_handle = insert_plugin_handle(target, LocalizedStringsState::default());
        let children = serde_json::json!({
            "B21_Appalachia.esp:2628FE": {
                "Persistent": [],
                "Temporary": ["B21_Appalachia.esp:4EA534", "B21_Appalachia.esp:4EA535"]
            }
        });
        let form_key_map = serde_json::json!({
            "SeventySix.esm:1A6663": "Fallout4.esm:1A6663"
        });

        let payload_text = plugin_handle_copy_cell_slice_children_json(
            source_handle,
            target_handle,
            &children.to_string(),
            0.0,
            0.0,
            0.0,
            Some(&form_key_map.to_string()),
        )
        .expect("copy children");
        let payload: JsonValue = serde_json::from_str(&payload_text).expect("json payload");
        assert_eq!(payload["children_inserted"], serde_json::json!(2));
        assert_eq!(payload["child_form_ids_reallocated"], serde_json::json!(1));

        let store = plugin_handle_store_ref().lock().unwrap();
        let target_slot = store.get(&target_handle).expect("target handle");
        let ParsedItem::Group(cell_group) = &target_slot.parsed.root_items[0] else {
            panic!("expected cell child group");
        };
        let ParsedItem::Group(temp_group) = &cell_group.children[1] else {
            panic!("expected temporary group");
        };
        let ParsedItem::Record(inserted_first) = &temp_group.children[0] else {
            panic!("expected first inserted record");
        };
        let ParsedItem::Record(inserted_second) = &temp_group.children[1] else {
            panic!("expected second inserted record");
        };
        assert_eq!(inserted_first.form_id, 0x018D804E);
        assert_eq!(inserted_second.form_id, 0x014EA535);
        let linked_ref = subrecord_data(inserted_second, "XLRL").expect("XLRL");
        assert_eq!(
            u32::from_le_bytes([linked_ref[0], linked_ref[1], linked_ref[2], linked_ref[3]]),
            0x018D804E
        );
        drop(store);
        plugin_handle_close_native(source_handle);
        plugin_handle_close_native(target_handle);
    }

    #[test]
    fn copy_cell_slice_children_drops_fo76_workshop_pack_in_inam_bool() {
        let mut source_ref = record("REFR", 0x014EA534, None);
        source_ref
            .subrecords
            .push(subrecord("NAME", 0x011A6663_u32.to_le_bytes().to_vec()));
        source_ref
            .subrecords
            .push(subrecord("INAM", 1_u16.to_le_bytes().to_vec()));
        let source = plugin_with_name(
            "SeventySix.esm",
            "fo76",
            vec!["Fallout76.esm".to_string()],
            vec![top_group("REFR", vec![ParsedItem::Record(source_ref)])],
        );

        let target_cell_group = ParsedItem::Group(ParsedGroup {
            label: 0x012628FE_u32.to_le_bytes(),
            group_type: CELL_CHILD_GROUP,
            tail: Bytes::new(),
            children: Vec::new(),
        });
        let target = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            vec!["Fallout4.esm".to_string()],
            vec![target_cell_group],
        );
        let source_handle = insert_plugin_handle(source, LocalizedStringsState::default());
        let target_handle = insert_plugin_handle(target, LocalizedStringsState::default());
        let children = serde_json::json!({
            "B21_Appalachia.esp:2628FE": {
                "Persistent": [],
                "Temporary": ["B21_Appalachia.esp:4EA534"]
            }
        });
        let form_key_map = serde_json::json!({
            "SeventySix.esm:1A6663": "Fallout4.esm:1A6663"
        });

        let payload_text = plugin_handle_copy_cell_slice_children_json(
            source_handle,
            target_handle,
            &children.to_string(),
            0.0,
            0.0,
            0.0,
            Some(&form_key_map.to_string()),
        )
        .expect("copy children");
        let payload: JsonValue = serde_json::from_str(&payload_text).expect("json payload");
        assert_eq!(payload["children_inserted"], serde_json::json!(1));
        assert_eq!(payload["schema_subrecords_dropped"], serde_json::json!(1));

        let store = plugin_handle_store_ref().lock().unwrap();
        let target_slot = store.get(&target_handle).expect("target handle");
        let ParsedItem::Group(cell_group) = &target_slot.parsed.root_items[0] else {
            panic!("expected cell child group");
        };
        let ParsedItem::Group(temp_group) = &cell_group.children[1] else {
            panic!("expected temporary group");
        };
        let ParsedItem::Record(inserted) = &temp_group.children[0] else {
            panic!("expected inserted record");
        };
        assert!(subrecord_data(inserted, "INAM").is_none());
        drop(store);
        plugin_handle_close_native(source_handle);
        plugin_handle_close_native(target_handle);
    }

    #[test]
    fn copy_cell_slice_children_replaces_lvli_base_with_stable_random_entry() {
        let mut leveled_base = record("LVLI", 0x01100000, Some("LPI_RockBase"));
        leveled_base
            .subrecords
            .push(subrecord("LVLO", 0x011A6663_u32.to_le_bytes().to_vec()));
        leveled_base
            .subrecords
            .push(subrecord("LVLO", 0x011A7777_u32.to_le_bytes().to_vec()));
        let static_base = record("STAT", 0x011A6663, Some("RockBase"));
        let second_static_base = record("STAT", 0x011A7777, Some("RockBaseVariant"));
        let mut source_ref = record("REFR", 0x014EA534, None);
        source_ref
            .subrecords
            .push(subrecord("NAME", 0x01100000_u32.to_le_bytes().to_vec()));
        let source = plugin_with_name(
            "SeventySix.esm",
            "fo76",
            vec!["Fallout76.esm".to_string()],
            vec![
                top_group("LVLI", vec![ParsedItem::Record(leveled_base)]),
                top_group(
                    "STAT",
                    vec![
                        ParsedItem::Record(static_base),
                        ParsedItem::Record(second_static_base),
                    ],
                ),
                top_group("REFR", vec![ParsedItem::Record(source_ref)]),
            ],
        );

        let target_cell_group = ParsedItem::Group(ParsedGroup {
            label: 0x012628FE_u32.to_le_bytes(),
            group_type: CELL_CHILD_GROUP,
            tail: Bytes::new(),
            children: Vec::new(),
        });
        let target = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            vec!["Fallout4.esm".to_string()],
            vec![target_cell_group],
        );
        let source_handle = insert_plugin_handle(source, LocalizedStringsState::default());
        let target_handle = insert_plugin_handle(target, LocalizedStringsState::default());
        let children = serde_json::json!({
            "B21_Appalachia.esp:2628FE": {
                "Persistent": [],
                "Temporary": ["B21_Appalachia.esp:4EA534"]
            }
        });
        let form_key_map = serde_json::json!({
            "SeventySix.esm:1A6663": "Fallout4.esm:1A6663",
            "SeventySix.esm:1A7777": "Fallout4.esm:1A7777"
        });

        let payload_text = plugin_handle_copy_cell_slice_children_json(
            source_handle,
            target_handle,
            &children.to_string(),
            0.0,
            0.0,
            0.0,
            Some(&form_key_map.to_string()),
        )
        .expect("copy children");
        let payload: JsonValue = serde_json::from_str(&payload_text).expect("json payload");
        assert_eq!(payload["children_inserted"], serde_json::json!(1));
        assert_eq!(payload["leveled_bases_resolved"], serde_json::json!(1));

        let store = plugin_handle_store_ref().lock().unwrap();
        let target_slot = store.get(&target_handle).expect("target handle");
        let ParsedItem::Group(cell_group) = &target_slot.parsed.root_items[0] else {
            panic!("expected cell child group");
        };
        let ParsedItem::Group(temp_group) = &cell_group.children[1] else {
            panic!("expected temporary group");
        };
        let ParsedItem::Record(inserted) = &temp_group.children[0] else {
            panic!("expected inserted record");
        };
        let name = subrecord_data(inserted, "NAME").expect("NAME");
        assert_eq!(
            u32::from_le_bytes([name[0], name[1], name[2], name[3]]),
            0x001A7777
        );
        drop(store);
        plugin_handle_close_native(source_handle);
        plugin_handle_close_native(target_handle);
    }

    #[test]
    fn copy_cell_slice_children_drops_zero_extent_primitive() {
        let mut source_ref = record("REFR", 0x014EA534, None);
        source_ref
            .subrecords
            .push(subrecord("NAME", 0x011A6663_u32.to_le_bytes().to_vec()));
        let mut xprm = Vec::new();
        for value in [0.0_f32, 0.0, 0.0, 0.5, 0.5, 1.0, 0.3] {
            xprm.extend_from_slice(&value.to_le_bytes());
        }
        xprm.extend_from_slice(&1_u32.to_le_bytes());
        source_ref.subrecords.push(subrecord("XPRM", xprm));
        let source = plugin_with_name(
            "SeventySix.esm",
            "fo76",
            vec!["Fallout76.esm".to_string()],
            vec![
                top_group(
                    "STAT",
                    vec![ParsedItem::Record(record("STAT", 0x011A6663, Some("Base")))],
                ),
                top_group("REFR", vec![ParsedItem::Record(source_ref)]),
            ],
        );

        let target_cell_group = ParsedItem::Group(ParsedGroup {
            label: 0x012628FE_u32.to_le_bytes(),
            group_type: CELL_CHILD_GROUP,
            tail: Bytes::new(),
            children: Vec::new(),
        });
        let target = plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            vec!["Fallout4.esm".to_string()],
            vec![
                target_cell_group,
                top_group(
                    "STAT",
                    vec![ParsedItem::Record(record("STAT", 0x011A6663, Some("Base")))],
                ),
            ],
        );
        let source_handle = insert_plugin_handle(source, LocalizedStringsState::default());
        let target_handle = insert_plugin_handle(target, LocalizedStringsState::default());
        let children = serde_json::json!({
            "B21_Appalachia.esp:2628FE": {
                "Persistent": [],
                "Temporary": ["B21_Appalachia.esp:4EA534"]
            }
        });
        let form_key_map = serde_json::json!({
            "SeventySix.esm:1A6663": "Fallout4.esm:1A6663"
        });

        let payload_text = plugin_handle_copy_cell_slice_children_json(
            source_handle,
            target_handle,
            &children.to_string(),
            0.0,
            0.0,
            0.0,
            Some(&form_key_map.to_string()),
        )
        .expect("copy children");
        let payload: JsonValue = serde_json::from_str(&payload_text).expect("json payload");
        assert_eq!(payload["children_inserted"], serde_json::json!(1));
        assert_eq!(payload["schema_subrecords_dropped"], serde_json::json!(1));

        let store = plugin_handle_store_ref().lock().unwrap();
        let target_slot = store.get(&target_handle).expect("target handle");
        let ParsedItem::Group(cell_group) = &target_slot.parsed.root_items[0] else {
            panic!("expected cell child group");
        };
        let ParsedItem::Group(temp_group) = &cell_group.children[1] else {
            panic!("expected temporary group");
        };
        let ParsedItem::Record(inserted) = &temp_group.children[0] else {
            panic!("expected inserted record");
        };
        assert!(subrecord_data(inserted, "XPRM").is_none());
        drop(store);
        plugin_handle_close_native(source_handle);
        plugin_handle_close_native(target_handle);
    }

    // ── Worldspace persistent-cell synthesis ────────────────────────────────

    /// FO76 source: WRLD → World Children(1) → [persistent CELL 0x050B2C +
    /// Cell-Children(6) → Cell-Persistent(8) → REFR/ACHR] + an exterior grid
    /// cell. The persistent CELL carries FO76-only subrecords + a 4-byte DATA.
    fn source_world_with_persistent_cell() -> ParsedPlugin {
        let world = record("WRLD", 0x0025DA15, Some("APPALACHIA"));

        let mut persistent_cell = record("CELL", 0x00050B2C, Some("PersistentWorldCell"));
        persistent_cell.flags = 0x0004_0400;
        // FO76 4-byte raw DATA: has_water (bit 2).
        persistent_cell
            .subrecords
            .push(subrecord("DATA", vec![0x02, 0x00, 0x00, 0x00]));
        // XCLW = water height (FO4-valid, carried).
        persistent_cell
            .subrecords
            .push(subrecord("XCLW", f32::MAX.to_le_bytes().to_vec()));
        // FO76-only subrecords FO4 has no home for (must be dropped).
        persistent_cell
            .subrecords
            .push(subrecord("XILS", 1.0_f32.to_le_bytes().to_vec()));
        persistent_cell
            .subrecords
            .push(subrecord("CII0", (-1.0_f32).to_le_bytes().to_vec()));

        // Own-plugin records: master list has 1 entry so the own index is 1
        // (prefix 0x01); the persistent refs and the cell live in SeventySix.esm.
        let mut persistent_refr = record("REFR", 0x018CC335, None);
        persistent_refr.flags = 0x0000_0400;
        persistent_refr
            .subrecords
            .push(subrecord("NAME", 0x011A6663_u32.to_le_bytes().to_vec()));
        let mut persistent_achr = record("ACHR", 0x018CC400, None);
        persistent_achr.flags = 0x0000_0400;
        persistent_achr
            .subrecords
            .push(subrecord("NAME", 0x011A6663_u32.to_le_bytes().to_vec()));

        let persistent_section = ParsedItem::Group(ParsedGroup {
            label: 0x00050B2C_u32.to_le_bytes(),
            group_type: PERSISTENT_GROUP,
            tail: Bytes::new(),
            children: vec![
                ParsedItem::Record(persistent_refr),
                ParsedItem::Record(persistent_achr),
            ],
        });
        let persistent_cell_children = ParsedItem::Group(ParsedGroup {
            label: 0x00050B2C_u32.to_le_bytes(),
            group_type: CELL_CHILD_GROUP,
            tail: Bytes::new(),
            children: vec![persistent_section],
        });

        let grid_cell = cell_record(0x002628FE, "OriginExt", 0, 0);
        let grid_block = ParsedItem::Group(ParsedGroup {
            label: [0, 0, 0, 0],
            group_type: EXTERIOR_CELL_BLOCK,
            tail: Bytes::new(),
            children: vec![ParsedItem::Group(ParsedGroup {
                label: [0, 0, 0, 0],
                group_type: EXTERIOR_CELL_SUBBLOCK,
                tail: Bytes::new(),
                children: vec![ParsedItem::Record(grid_cell)],
            })],
        });

        let world_children = ParsedItem::Group(ParsedGroup {
            label: 0x0025DA15_u32.to_le_bytes(),
            group_type: 1,
            tail: Bytes::new(),
            children: vec![
                ParsedItem::Record(persistent_cell),
                persistent_cell_children,
                grid_block,
            ],
        });
        plugin_with_name(
            "SeventySix.esm",
            "fo76",
            vec!["Fallout76.esm".to_string()],
            vec![top_group(
                "WRLD",
                vec![ParsedItem::Record(world), world_children],
            )],
        )
    }

    /// FO4 target: WRLD record + an (empty) World Children GRUP holding one
    /// exterior block (no persistent cell yet — what the conversion produces
    /// before this synthesis phase runs).
    fn target_world_without_persistent_cell() -> ParsedPlugin {
        let world = record("WRLD", 0x0100_0000 | 0x0025DA15, Some("APPALACHIA"));
        let grid_block = ParsedItem::Group(ParsedGroup {
            label: [0, 0, 0, 0],
            group_type: EXTERIOR_CELL_BLOCK,
            tail: Bytes::new(),
            children: Vec::new(),
        });
        let world_children = ParsedItem::Group(ParsedGroup {
            label: world.form_id.to_le_bytes(),
            group_type: 1,
            tail: Bytes::new(),
            children: vec![grid_block],
        });
        plugin_with_name(
            "B21_Appalachia.esp",
            "fo4",
            vec!["Fallout4.esm".to_string()],
            vec![top_group(
                "WRLD",
                vec![ParsedItem::Record(world), world_children],
            )],
        )
    }

    fn run_synthesis(source: ParsedPlugin, target: ParsedPlugin) -> (u64, u64, JsonValue) {
        let source_handle = insert_plugin_handle(source, LocalizedStringsState::default());
        let target_handle = insert_plugin_handle(target, LocalizedStringsState::default());
        let form_key_map = serde_json::json!({
            "SeventySix.esm:1A6663": "Fallout4.esm:1A6663"
        });
        let payload_text = plugin_handle_synthesize_worldspace_persistent_cell_json(
            source_handle,
            target_handle,
            "APPALACHIA",
            0.0,
            0.0,
            0.0,
            Some(&form_key_map.to_string()),
        )
        .expect("synthesize persistent cell");
        let payload: JsonValue = serde_json::from_str(&payload_text).expect("json payload");
        (source_handle, target_handle, payload)
    }

    fn world_children_of<'a>(items: &'a [ParsedItem]) -> &'a ParsedGroup {
        let ParsedItem::Group(wrld) = &items[0] else {
            panic!("expected WRLD top group");
        };
        wrld.children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == 1 => Some(group),
                _ => None,
            })
            .expect("world children group")
    }

    #[test]
    fn synthesize_persistent_cell_emits_cell_as_first_world_child() {
        let (source_handle, target_handle, payload) = run_synthesis(
            source_world_with_persistent_cell(),
            target_world_without_persistent_cell(),
        );

        assert_eq!(payload["cell_synthesized"], serde_json::json!(true));
        assert_eq!(payload["persistent_refs_converted"], serde_json::json!(2));
        assert_eq!(payload["persistent_refs_skipped"], serde_json::json!(0));

        let store = plugin_handle_store_ref().lock().unwrap();
        let target_slot = store.get(&target_handle).expect("target handle");
        let world_children = world_children_of(&target_slot.parsed.root_items);

        // First child of World Children = the persistent CELL, before block GRUPs.
        let ParsedItem::Record(cell) = &world_children.children[0] else {
            panic!("expected persistent CELL as first World Children record");
        };
        assert_eq!(cell.signature.as_str(), "CELL");
        assert_eq!(cell.form_id & 0x00FF_FFFF, 0x00050B2C, "objid preserved");
        assert_eq!(cell.form_id & 0xFF00_0000, 0x0100_0000, "own-plugin prefix");
        assert_eq!(
            cell.flags & 0x0000_0400,
            0x0000_0400,
            "persistent header flag"
        );

        // DATA is re-encoded to 2 bytes (FO4 uint16) carrying has_water.
        let data = subrecord_data(cell, "DATA").expect("CELL DATA");
        assert_eq!(data.len(), 2, "FO4 CELL DATA is uint16");
        assert_eq!(u16::from_le_bytes([data[0], data[1]]), 0x0002, "has_water");
        // FO4-valid XCLW carried; FO76-only XILS/CII0 dropped.
        assert!(subrecord_data(cell, "XCLW").is_some());
        assert!(subrecord_data(cell, "XILS").is_none());
        assert!(subrecord_data(cell, "CII0").is_none());

        // Second child = the persistent CELL's Cell-Children(6) group.
        let ParsedItem::Group(cell_children) = &world_children.children[1] else {
            panic!("expected Cell Children group after persistent CELL");
        };
        assert_eq!(cell_children.group_type, CELL_CHILD_GROUP);
        assert_eq!(u32::from_le_bytes(cell_children.label), cell.form_id);

        // Block GRUP still present, after the persistent cell.
        assert!(world_children.children.iter().any(
            |item| matches!(item, ParsedItem::Group(g) if g.group_type == EXTERIOR_CELL_BLOCK)
        ));
        drop(store);
        plugin_handle_close_native(source_handle);
        plugin_handle_close_native(target_handle);
    }

    #[test]
    fn synthesize_persistent_cell_nests_converted_refs_in_cell_persistent() {
        let (source_handle, target_handle, _payload) = run_synthesis(
            source_world_with_persistent_cell(),
            target_world_without_persistent_cell(),
        );

        let store = plugin_handle_store_ref().lock().unwrap();
        let target_slot = store.get(&target_handle).expect("target handle");
        let world_children = world_children_of(&target_slot.parsed.root_items);
        let ParsedItem::Group(cell_children) = &world_children.children[1] else {
            panic!("expected Cell Children group");
        };
        let persistent = cell_children
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == PERSISTENT_GROUP => Some(group),
                _ => None,
            })
            .expect("Cell Persistent(8) group");
        assert_eq!(persistent.children.len(), 2, "both refs routed");
        // No Temporary(9) group emitted (FO4 persistent cell has none).
        assert!(
            !cell_children.children.iter().any(
                |item| matches!(item, ParsedItem::Group(g) if g.group_type == TEMPORARY_GROUP)
            )
        );

        for child in &persistent.children {
            let ParsedItem::Record(refr) = child else {
                panic!("expected placed record");
            };
            // Accuracy A: 0x400 persistent header flag preserved on each ref.
            assert_eq!(refr.flags & 0x0000_0400, 0x0000_0400, "ref persistent flag");
            // Base NAME remapped to the Fallout4.esm master.
            let name = subrecord_data(refr, "NAME").expect("NAME");
            assert_eq!(
                u32::from_le_bytes([name[0], name[1], name[2], name[3]]) & 0x00FF_FFFF,
                0x001A6663
            );
        }

        // Refs were NOT scattered into the grid cell / any grid-cell child group.
        let grid_block = world_children
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == EXTERIOR_CELL_BLOCK => Some(group),
                _ => None,
            })
            .expect("exterior block");
        let mut grid_persistent_count = 0usize;
        fn count_persistent(group: &ParsedGroup, count: &mut usize) {
            for item in &group.children {
                if let ParsedItem::Group(g) = item {
                    if g.group_type == PERSISTENT_GROUP {
                        *count += g.children.len();
                    }
                    count_persistent(g, count);
                }
            }
        }
        count_persistent(grid_block, &mut grid_persistent_count);
        assert_eq!(
            grid_persistent_count, 0,
            "no persistent refs leaked into grid cells"
        );
        drop(store);
        plugin_handle_close_native(source_handle);
        plugin_handle_close_native(target_handle);
    }

    #[test]
    fn synthesize_persistent_cell_is_idempotent() {
        let source_handle = insert_plugin_handle(
            source_world_with_persistent_cell(),
            LocalizedStringsState::default(),
        );
        let target_handle = insert_plugin_handle(
            target_world_without_persistent_cell(),
            LocalizedStringsState::default(),
        );
        let form_key_map = serde_json::json!({ "SeventySix.esm:1A6663": "Fallout4.esm:1A6663" });
        let call = || {
            let text = plugin_handle_synthesize_worldspace_persistent_cell_json(
                source_handle,
                target_handle,
                "APPALACHIA",
                0.0,
                0.0,
                0.0,
                Some(&form_key_map.to_string()),
            )
            .expect("synthesize");
            serde_json::from_str::<JsonValue>(&text).expect("json")
        };
        let first = call();
        assert_eq!(first["cell_synthesized"], serde_json::json!(true));
        let second = call();
        assert_eq!(
            second["cell_synthesized"],
            serde_json::json!(false),
            "no re-synthesis"
        );

        let store = plugin_handle_store_ref().lock().unwrap();
        let target_slot = store.get(&target_handle).expect("target handle");
        let world_children = world_children_of(&target_slot.parsed.root_items);
        let cell_count = world_children
            .children
            .iter()
            .filter(|item| matches!(item, ParsedItem::Record(r) if r.signature.as_str() == "CELL"))
            .count();
        assert_eq!(cell_count, 1, "exactly one persistent cell after two runs");
        drop(store);
        plugin_handle_close_native(source_handle);
        plugin_handle_close_native(target_handle);
    }

    #[test]
    fn synthesize_persistent_cell_routes_objid_remapped_ref() {
        // Regression: a persistent ref whose objid is REMAPPED via form_key_map
        // (collision-reallocation or QUST-translate remap) must still be found in
        // the SOURCE, converted, and land under Cell-Persistent(8) at the REMAPPED
        // target objid. The earlier code looked the source up by the *target* objid
        // → miss → silent drop + objid loss (breaking Owner A's ALFR-by-objid repair).
        let source_handle = insert_plugin_handle(
            source_world_with_persistent_cell(),
            LocalizedStringsState::default(),
        );
        let target_handle = insert_plugin_handle(
            target_world_without_persistent_cell(),
            LocalizedStringsState::default(),
        );
        // Remap the persistent REFR's own objid 8CC335 -> 099999 (own-plugin) and
        // its base NAME as usual.
        let form_key_map = serde_json::json!({
            "SeventySix.esm:1A6663": "Fallout4.esm:1A6663",
            "SeventySix.esm:8CC335": "B21_Appalachia.esp:099999",
        });
        let payload_text = plugin_handle_synthesize_worldspace_persistent_cell_json(
            source_handle,
            target_handle,
            "APPALACHIA",
            0.0,
            0.0,
            0.0,
            Some(&form_key_map.to_string()),
        )
        .expect("synthesize");
        let payload: JsonValue = serde_json::from_str(&payload_text).expect("json");
        // Both refs convert (none dropped) — the remapped one is NOT lost.
        assert_eq!(payload["persistent_refs_converted"], serde_json::json!(2));
        assert_eq!(payload["persistent_refs_skipped"], serde_json::json!(0));

        let store = plugin_handle_store_ref().lock().unwrap();
        let target_slot = store.get(&target_handle).expect("target handle");
        let world_children = world_children_of(&target_slot.parsed.root_items);
        let ParsedItem::Group(cell_children) = &world_children.children[1] else {
            panic!("expected Cell Children group");
        };
        let persistent = cell_children
            .children
            .iter()
            .find_map(|item| match item {
                ParsedItem::Group(group) if group.group_type == PERSISTENT_GROUP => Some(group),
                _ => None,
            })
            .expect("Cell Persistent(8) group");
        // The remapped REFR is present at the REMAPPED target form_id (own_prefix|099999).
        let remapped_present = persistent.children.iter().any(|child| {
            matches!(child, ParsedItem::Record(r)
                if r.signature.as_str() == "REFR" && r.form_id == (0x0100_0000 | 0x0009_9999))
        });
        assert!(
            remapped_present,
            "objid-remapped REFR routed at remapped form_id, not dropped"
        );
        drop(store);
        plugin_handle_close_native(source_handle);
        plugin_handle_close_native(target_handle);
    }

    #[test]
    fn collect_persistent_base_keys_returns_every_base_deduped() {
        // The persistent REFR + ACHR both point at base 1A6663; the collector
        // returns each distinct base once, regardless of base record type (so
        // non-PLACEMENT_BASE_SIGNATURES bases get seeded into the form_key_map).
        let source_handle = insert_plugin_handle(
            source_world_with_persistent_cell(),
            LocalizedStringsState::default(),
        );
        let text =
            plugin_handle_collect_worldspace_persistent_base_keys_json(source_handle, "APPALACHIA")
                .expect("collect base keys");
        let keys: Vec<String> = serde_json::from_str(&text).expect("json");
        plugin_handle_close_native(source_handle);
        // 1A6663 is the shared base of both persistent refs — present exactly once.
        assert_eq!(
            keys.iter().filter(|k| k.ends_with(":1A6663")).count(),
            1,
            "shared base collected once (deduped)"
        );
    }
}
