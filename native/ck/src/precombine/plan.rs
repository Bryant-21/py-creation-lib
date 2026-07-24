//! Builds a [`PrecombinePlan`] for one interior CELL: eligible STAT-backed
//! Temporary-group REFRs, grouped by model path. Read-only — no NIF or ESP
//! mutation happens here (see `precombine::bake` / `precombine::stamp`).

use esp_authoring_core::plugin_runtime::{plugin_handle_store_ref, ParsedGroup, ParsedItem, ParsedRecord};
use std::collections::HashMap;
use std::path::PathBuf;

const CELL_CHILD_GROUP: i32 = 6;
const TEMPORARY_GROUP: i32 = 9;
const RECORD_FLAG_DELETED: u32 = 0x0000_0020;
const RECORD_FLAG_INITIALLY_DISABLED: u32 = 0x0000_0800;
const BLOCKED_REF_SUBRECORDS: [&str; 6] = ["VMAD", "XESP", "XTEL", "XAPD", "XAPR", "XLKR"];
const BLOCKED_BASE_SUBRECORDS: [&str; 2] = ["VMAD", "MODS"];

#[derive(serde::Deserialize)]
pub struct Params {
    pub target_handle_id: u64,
    pub plugin_name: String,
    pub data_root: PathBuf,
    pub include_cells: Vec<u32>,
    pub min_eligible_refs: usize,
    pub pcmb_date: u16,
    pub no_previs: bool,
    /// Extra loose search roots, tried in order after `data_root` and before
    /// `mesh_archives` — e.g. a pre-extracted vanilla/DLC asset directory
    /// that mirrors `Data\` layout. `#[serde(default)]` keeps params JSON
    /// without this key working.
    #[serde(default)]
    pub mesh_extract_roots: Vec<PathBuf>,
    /// Ordered fallback archives consulted (after loose roots) when a source
    /// MODL isn't a loose file — e.g. vanilla/DLC meshes packed in game BA2s.
    /// `#[serde(default)]` keeps params JSON without this key working.
    #[serde(default)]
    pub mesh_archives: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstanceRef {
    pub refr_form_id: u32,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelGroup {
    pub model_path: String,
    pub instances: Vec<InstanceRef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CellPlan {
    pub cell_form_id: u32,
    pub groups: Vec<ModelGroup>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrecombinePlan {
    pub cells: Vec<CellPlan>,
}

pub fn build_plan(params: &Params) -> Result<PrecombinePlan, String> {
    if params.include_cells.len() != 1 {
        return Err(format!(
            "include_cells must contain exactly one cell, got {}",
            params.include_cells.len()
        ));
    }
    let requested_object_id = params.include_cells[0] & 0x00FF_FFFF;

    let store = plugin_handle_store_ref().lock().unwrap();
    let slot = store
        .get(&params.target_handle_id)
        .ok_or_else(|| format!("no plugin handle: {}", params.target_handle_id))?;
    // own_index must fit the XCRI reference's top byte (own_index << 24) —
    // aligned with the same check in `stamp::stamp_cell` rather than
    // silently wrapping at 256 masters.
    let masters_len = slot.parsed.header.masters.len();
    if masters_len > 0xFF {
        return Err(format!(
            "own index overflow: {masters_len} masters exceeds the 1-byte XCRI reference field"
        ));
    }
    let own_index = masters_len as u8;

    // Pass 1: index eligible STAT bases (own-plugin only — v0 never opens
    // masters) by object id, before touching the requested cell at all.
    let mut eligible_bases: HashMap<u32, String> = HashMap::new();
    index_eligible_bases(&slot.parsed.root_items, own_index, &mut eligible_bases);

    // Pass 2: locate the requested CELL and walk its Temporary child group.
    let cell_record = find_cell_record(&slot.parsed.root_items, requested_object_id)
        .ok_or_else(|| format!("unknown cell: {requested_object_id:06X}"))?;
    let cell_form_id = cell_record.form_id;

    let temporary_children = find_temporary_group(&slot.parsed.root_items, cell_form_id)
        .map(|group| group.children.as_slice())
        .unwrap_or(&[]);

    let mut groups: HashMap<String, ModelGroup> = HashMap::new();
    for item in temporary_children {
        let ParsedItem::Record(refr) = item else {
            continue;
        };
        if refr.signature.as_str() != "REFR" {
            continue;
        }
        let Some(instance) = eligible_instance(refr, &eligible_bases, own_index) else {
            continue;
        };
        groups
            .entry(instance.0.clone())
            .or_insert_with(|| ModelGroup {
                model_path: instance.0.clone(),
                instances: Vec::new(),
            })
            .instances
            .push(instance.1);
    }
    drop(store); // release the ESP store lock before any further processing.

    let mut groups: Vec<ModelGroup> = groups
        .into_values()
        .filter(|group| group.instances.len() >= params.min_eligible_refs)
        .collect();
    groups.sort_by(|a, b| a.model_path.cmp(&b.model_path));
    for group in &mut groups {
        group.instances.sort_by_key(|instance| instance.refr_form_id);
    }

    Ok(PrecombinePlan {
        cells: vec![CellPlan {
            cell_form_id,
            groups,
        }],
    })
}

fn index_eligible_bases(items: &[ParsedItem], own_index: u8, out: &mut HashMap<u32, String>) {
    for item in items {
        match item {
            ParsedItem::Record(record) => {
                if record.signature.as_str() != "STAT" {
                    continue;
                }
                if ((record.form_id >> 24) & 0xFF) as u8 != own_index {
                    continue;
                }
                if has_blocked_subrecord(record, &BLOCKED_BASE_SUBRECORDS) {
                    continue;
                }
                if let Some(model_path) = model_path_of(record) {
                    out.insert(record.form_id & 0x00FF_FFFF, model_path);
                }
            }
            ParsedItem::Group(group) => index_eligible_bases(&group.children, own_index, out),
        }
    }
}

fn find_cell_record<'a>(items: &'a [ParsedItem], object_id: u32) -> Option<&'a ParsedRecord> {
    for item in items {
        match item {
            ParsedItem::Record(record)
                if record.signature.as_str() == "CELL"
                    && (record.form_id & 0x00FF_FFFF) == object_id =>
            {
                return Some(record);
            }
            ParsedItem::Group(group) => {
                if let Some(found) = find_cell_record(&group.children, object_id) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

/// The Cell-Children group (type 6) and its Temporary section (type 9) both
/// carry the owning CELL's form id as their group label — not an index.
fn find_temporary_group<'a>(items: &'a [ParsedItem], cell_form_id: u32) -> Option<&'a ParsedGroup> {
    let label = cell_form_id.to_le_bytes();
    let cell_children = find_group(items, &|g: &ParsedGroup| {
        g.group_type == CELL_CHILD_GROUP && g.label == label
    })?;
    find_group(&cell_children.children, &|g: &ParsedGroup| {
        g.group_type == TEMPORARY_GROUP && g.label == label
    })
}

fn find_group<'a>(
    items: &'a [ParsedItem],
    predicate: &dyn Fn(&ParsedGroup) -> bool,
) -> Option<&'a ParsedGroup> {
    for item in items {
        if let ParsedItem::Group(group) = item {
            if predicate(group) {
                return Some(group);
            }
            if let Some(found) = find_group(&group.children, predicate) {
                return Some(found);
            }
        }
    }
    None
}

fn has_blocked_subrecord(record: &ParsedRecord, blocked: &[&str]) -> bool {
    record
        .subrecords
        .iter()
        .any(|sub| blocked.contains(&sub.signature.as_str()))
}

fn model_path_of(record: &ParsedRecord) -> Option<String> {
    let modl = record
        .subrecords
        .iter()
        .find(|sub| sub.signature.as_str() == "MODL")?;
    // Grouping is case-insensitive (Windows paths), so the canonical form
    // stored on the group is lowercase.
    let text = String::from_utf8_lossy(modl.data.as_ref())
        .trim_end_matches('\0')
        .trim()
        .to_ascii_lowercase();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn eligible_instance(
    refr: &ParsedRecord,
    eligible_bases: &HashMap<u32, String>,
    own_index: u8,
) -> Option<(String, InstanceRef)> {
    if refr.flags & (RECORD_FLAG_DELETED | RECORD_FLAG_INITIALLY_DISABLED) != 0 {
        return None;
    }
    if has_blocked_subrecord(refr, &BLOCKED_REF_SUBRECORDS) {
        return None;
    }
    // An "override" REFR (top byte != own_index, e.g. master-owned) must be
    // excluded: XCRI is stamped from this same form id verbatim downstream,
    // so a non-own-index ref here would desync XCRI from the VC-stamped
    // record.
    if ((refr.form_id >> 24) & 0xFF) as u8 != own_index {
        return None;
    }
    let name = refr
        .subrecords
        .iter()
        .find(|sub| sub.signature.as_str() == "NAME")?;
    if name.data.len() < 4 {
        return None;
    }
    let base_raw = u32::from_le_bytes([name.data[0], name.data[1], name.data[2], name.data[3]]);
    if ((base_raw >> 24) & 0xFF) as u8 != own_index {
        return None;
    }
    let model_path = eligible_bases.get(&(base_raw & 0x00FF_FFFF))?.clone();

    let data = refr
        .subrecords
        .iter()
        .find(|sub| sub.signature.as_str() == "DATA")?;
    if data.data.len() < 24 {
        return None;
    }
    let read_f32 = |offset: usize| -> f32 {
        f32::from_le_bytes([
            data.data[offset],
            data.data[offset + 1],
            data.data[offset + 2],
            data.data[offset + 3],
        ])
    };
    let position = [read_f32(0), read_f32(4), read_f32(8)];
    let rotation = [read_f32(12), read_f32(16), read_f32(20)];

    let scale = refr
        .subrecords
        .iter()
        .find(|sub| sub.signature.as_str() == "XSCL")
        .filter(|sub| sub.data.len() >= 4)
        .map(|sub| f32::from_le_bytes([sub.data[0], sub.data[1], sub.data[2], sub.data[3]]))
        .unwrap_or(1.0);

    Some((
        model_path,
        InstanceRef {
            refr_form_id: refr.form_id,
            position,
            rotation,
            scale,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use esp_authoring_core::plugin_runtime::{
        ensure_interior_cell_and_child_group, insert_parsed_record,
        insert_placed_child_into_cell_group, plugin_handle_add_master_native,
        plugin_handle_new_native, ParsedSubrecord,
    };
    use smol_str::SmolStr;

    const TEMPORARY: i32 = 9;
    const PERSISTENT: i32 = 8;
    const CELL_INTERIOR_FLAG: u8 = 0x01;

    fn sub(sig: &str, data: Vec<u8>) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: SmolStr::new(sig),
            data: Bytes::from(data),
            semantic_type: None,
        }
    }

    fn parsed_record(sig: &str, form_id: u32, subrecords: Vec<ParsedSubrecord>) -> ParsedRecord {
        ParsedRecord {
            signature: SmolStr::new(sig),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: Some(131),
            version2: None,
            subrecords,
            raw_payload: None,
            parse_error: None,
        }
    }

    fn edid(name: &str) -> ParsedSubrecord {
        let mut bytes = name.as_bytes().to_vec();
        bytes.push(0);
        sub("EDID", bytes)
    }

    fn modl(path: &str) -> ParsedSubrecord {
        let mut bytes = path.as_bytes().to_vec();
        bytes.push(0);
        sub("MODL", bytes)
    }

    fn interior_cell(form_id: u32, name: &str) -> ParsedRecord {
        parsed_record(
            "CELL",
            form_id,
            vec![edid(name), sub("DATA", vec![CELL_INTERIOR_FLAG, 0x00])],
        )
    }

    fn stat_base(form_id: u32, model_path: &str) -> ParsedRecord {
        parsed_record(
            "STAT",
            form_id,
            vec![edid(&format!("Stat{form_id:06X}")), modl(model_path)],
        )
    }

    fn refr_data(pos: [f32; 3], rot: [f32; 3]) -> ParsedSubrecord {
        let mut bytes = Vec::with_capacity(24);
        for value in pos.iter().chain(rot.iter()) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        sub("DATA", bytes)
    }

    fn plain_refr(form_id: u32, base_form_id: u32, pos: [f32; 3], rot: [f32; 3]) -> ParsedRecord {
        parsed_record(
            "REFR",
            form_id,
            vec![
                sub("NAME", base_form_id.to_le_bytes().to_vec()),
                refr_data(pos, rot),
            ],
        )
    }

    fn refr_with_xscl(
        form_id: u32,
        base_form_id: u32,
        pos: [f32; 3],
        rot: [f32; 3],
        scale: f32,
    ) -> ParsedRecord {
        let mut record = plain_refr(form_id, base_form_id, pos, rot);
        record.subrecords.push(sub("XSCL", scale.to_le_bytes().to_vec()));
        record
    }

    fn new_target() -> u64 {
        plugin_handle_new_native("Test.esm", Some("fo4")).expect("target handle")
    }

    fn base_params(handle: u64, cell: u32) -> Params {
        Params {
            target_handle_id: handle,
            plugin_name: "Test.esm".into(),
            data_root: PathBuf::from("."),
            include_cells: vec![cell],
            min_eligible_refs: 1,
            pcmb_date: 0x1F24,
            no_previs: true,
            mesh_extract_roots: Vec::new(),
            mesh_archives: Vec::new(),
        }
    }

    #[test]
    fn unknown_handle_is_rejected() {
        let params = base_params(999_999, 0x001000);
        let err = build_plan(&params).expect_err("unknown handle must fail");
        assert!(err.contains("999999"), "error should name the handle: {err}");
    }

    #[test]
    fn zero_cells_is_rejected() {
        let target = new_target();
        let mut params = base_params(target, 0x001000);
        params.include_cells.clear();
        assert!(build_plan(&params).is_err());
    }

    #[test]
    fn multiple_cells_is_rejected() {
        let target = new_target();
        let mut params = base_params(target, 0x001000);
        params.include_cells.push(0x002000);
        assert!(build_plan(&params).is_err());
    }

    #[test]
    fn unknown_cell_is_rejected() {
        let target = new_target();
        ensure_interior_cell_and_child_group(target, interior_cell(0x001000, "Vault")).unwrap();
        let params = base_params(target, 0x009999);
        assert!(build_plan(&params).is_err());
    }

    #[test]
    fn selects_only_requested_cell_and_temporary_group() {
        let target = new_target();
        ensure_interior_cell_and_child_group(target, interior_cell(0x001000, "CellA")).unwrap();
        ensure_interior_cell_and_child_group(target, interior_cell(0x002000, "CellB")).unwrap();
        insert_parsed_record(target, stat_base(0x000500, "meshes\\a.nif")).unwrap();

        insert_placed_child_into_cell_group(
            target,
            0x001000,
            TEMPORARY,
            plain_refr(0x000600, 0x000500, [1.0, 2.0, 3.0], [0.0, 0.0, 0.0]),
        )
        .unwrap();
        insert_placed_child_into_cell_group(
            target,
            0x001000,
            PERSISTENT,
            plain_refr(0x000601, 0x000500, [4.0, 5.0, 6.0], [0.0, 0.0, 0.0]),
        )
        .unwrap();
        insert_placed_child_into_cell_group(
            target,
            0x002000,
            TEMPORARY,
            plain_refr(0x000602, 0x000500, [7.0, 8.0, 9.0], [0.0, 0.0, 0.0]),
        )
        .unwrap();

        let params = base_params(target, 0x001000);
        let plan = build_plan(&params).expect("plan");
        assert_eq!(plan.cells.len(), 1);
        let refr_ids: Vec<u32> = plan.cells[0]
            .groups
            .iter()
            .flat_map(|g| g.instances.iter().map(|i| i.refr_form_id))
            .collect();
        assert_eq!(refr_ids, vec![0x000600]);
    }

    #[test]
    fn accepts_plain_stat_backed_refr_with_default_and_explicit_scale() {
        let target = new_target();
        ensure_interior_cell_and_child_group(target, interior_cell(0x001000, "Cell")).unwrap();
        insert_parsed_record(target, stat_base(0x000500, "Meshes\\Furniture\\Chair01.nif"))
            .unwrap();
        insert_placed_child_into_cell_group(
            target,
            0x001000,
            TEMPORARY,
            plain_refr(0x000600, 0x000500, [1.0, 2.0, 3.0], [0.1, 0.2, 0.3]),
        )
        .unwrap();
        insert_placed_child_into_cell_group(
            target,
            0x001000,
            TEMPORARY,
            refr_with_xscl(0x000601, 0x000500, [4.0, 5.0, 6.0], [0.0, 0.0, 0.0], 2.5),
        )
        .unwrap();

        let params = base_params(target, 0x001000);
        let plan = build_plan(&params).expect("plan");
        let cell = &plan.cells[0];
        assert_eq!(cell.groups.len(), 1);
        let group = &cell.groups[0];
        assert_eq!(group.model_path, "meshes\\furniture\\chair01.nif");
        assert_eq!(group.instances.len(), 2);
        assert_eq!(group.instances[0].refr_form_id, 0x000600);
        assert_eq!(group.instances[0].position, [1.0, 2.0, 3.0]);
        assert_eq!(group.instances[0].rotation, [0.1, 0.2, 0.3]);
        assert_eq!(group.instances[0].scale, 1.0, "XSCL absent defaults to 1.0");
        assert_eq!(group.instances[1].refr_form_id, 0x000601);
        assert_eq!(group.instances[1].scale, 2.5);
    }

    #[test]
    fn excludes_deleted_and_initially_disabled_refs() {
        let target = new_target();
        ensure_interior_cell_and_child_group(target, interior_cell(0x001000, "Cell")).unwrap();
        insert_parsed_record(target, stat_base(0x000500, "meshes\\a.nif")).unwrap();

        let mut deleted = plain_refr(0x000600, 0x000500, [0.0; 3], [0.0; 3]);
        deleted.flags = RECORD_FLAG_DELETED;
        let mut disabled = plain_refr(0x000601, 0x000500, [0.0; 3], [0.0; 3]);
        disabled.flags = RECORD_FLAG_INITIALLY_DISABLED;
        let clean = plain_refr(0x000602, 0x000500, [0.0; 3], [0.0; 3]);

        insert_placed_child_into_cell_group(target, 0x001000, TEMPORARY, deleted).unwrap();
        insert_placed_child_into_cell_group(target, 0x001000, TEMPORARY, disabled).unwrap();
        insert_placed_child_into_cell_group(target, 0x001000, TEMPORARY, clean).unwrap();

        let params = base_params(target, 0x001000);
        let plan = build_plan(&params).expect("plan");
        let refr_ids: Vec<u32> = plan.cells[0]
            .groups
            .iter()
            .flat_map(|g| g.instances.iter().map(|i| i.refr_form_id))
            .collect();
        assert_eq!(refr_ids, vec![0x000602]);
    }

    #[test]
    fn excludes_refs_carrying_blocked_behavior_subrecords() {
        let target = new_target();
        ensure_interior_cell_and_child_group(target, interior_cell(0x001000, "Cell")).unwrap();
        insert_parsed_record(target, stat_base(0x000500, "meshes\\a.nif")).unwrap();

        let blocked_sigs = ["VMAD", "XESP", "XTEL", "XAPD", "XAPR", "XLKR"];
        let mut expected_excluded = Vec::new();
        for (i, sig) in blocked_sigs.iter().enumerate() {
            let form_id = 0x000700 + i as u32;
            let mut refr = plain_refr(form_id, 0x000500, [0.0; 3], [0.0; 3]);
            refr.subrecords.push(sub(sig, vec![0u8; 4]));
            insert_placed_child_into_cell_group(target, 0x001000, TEMPORARY, refr).unwrap();
            expected_excluded.push(form_id);
        }
        let clean = plain_refr(0x000800, 0x000500, [0.0; 3], [0.0; 3]);
        insert_placed_child_into_cell_group(target, 0x001000, TEMPORARY, clean).unwrap();

        let params = base_params(target, 0x001000);
        let plan = build_plan(&params).expect("plan");
        let refr_ids: Vec<u32> = plan.cells[0]
            .groups
            .iter()
            .flat_map(|g| g.instances.iter().map(|i| i.refr_form_id))
            .collect();
        assert_eq!(refr_ids, vec![0x000800]);
        for excluded in expected_excluded {
            assert!(!refr_ids.contains(&excluded), "{excluded:06X} must be excluded");
        }
    }

    #[test]
    fn excludes_bases_carrying_vmad_or_mods() {
        let target = new_target();
        ensure_interior_cell_and_child_group(target, interior_cell(0x001000, "Cell")).unwrap();

        let mut vmad_base = stat_base(0x000500, "meshes\\vmad.nif");
        vmad_base.subrecords.push(sub("VMAD", vec![0u8; 4]));
        let mut mods_base = stat_base(0x000501, "meshes\\mods.nif");
        mods_base.subrecords.push(sub("MODS", vec![0u8; 4]));
        let clean_base = stat_base(0x000502, "meshes\\clean.nif");
        insert_parsed_record(target, vmad_base).unwrap();
        insert_parsed_record(target, mods_base).unwrap();
        insert_parsed_record(target, clean_base).unwrap();

        insert_placed_child_into_cell_group(
            target,
            0x001000,
            TEMPORARY,
            plain_refr(0x000600, 0x000500, [0.0; 3], [0.0; 3]),
        )
        .unwrap();
        insert_placed_child_into_cell_group(
            target,
            0x001000,
            TEMPORARY,
            plain_refr(0x000601, 0x000501, [0.0; 3], [0.0; 3]),
        )
        .unwrap();
        insert_placed_child_into_cell_group(
            target,
            0x001000,
            TEMPORARY,
            plain_refr(0x000602, 0x000502, [0.0; 3], [0.0; 3]),
        )
        .unwrap();

        let params = base_params(target, 0x001000);
        let plan = build_plan(&params).expect("plan");
        let refr_ids: Vec<u32> = plan.cells[0]
            .groups
            .iter()
            .flat_map(|g| g.instances.iter().map(|i| i.refr_form_id))
            .collect();
        assert_eq!(refr_ids, vec![0x000602]);
    }

    #[test]
    fn excludes_refs_missing_data_and_bases_missing_modl() {
        let target = new_target();
        ensure_interior_cell_and_child_group(target, interior_cell(0x001000, "Cell")).unwrap();
        insert_parsed_record(target, stat_base(0x000500, "meshes\\a.nif")).unwrap();
        insert_parsed_record(target, parsed_record("STAT", 0x000501, vec![edid("NoModl")]))
            .unwrap();

        let no_data = parsed_record(
            "REFR",
            0x000600,
            vec![sub("NAME", 0x000500u32.to_le_bytes().to_vec())],
        );
        insert_placed_child_into_cell_group(target, 0x001000, TEMPORARY, no_data).unwrap();
        let no_modl_base_ref = plain_refr(0x000601, 0x000501, [0.0; 3], [0.0; 3]);
        insert_placed_child_into_cell_group(target, 0x001000, TEMPORARY, no_modl_base_ref)
            .unwrap();

        let params = base_params(target, 0x001000);
        let plan = build_plan(&params).expect("plan");
        assert!(plan.cells[0].groups.is_empty());
    }

    #[test]
    fn groups_case_insensitive_model_paths_deterministically() {
        let target = new_target();
        ensure_interior_cell_and_child_group(target, interior_cell(0x001000, "Cell")).unwrap();
        insert_parsed_record(target, stat_base(0x000500, "Meshes\\Foo.nif")).unwrap();
        insert_parsed_record(target, stat_base(0x000501, "meshes\\FOO.NIF")).unwrap();
        insert_parsed_record(target, stat_base(0x000502, "meshes\\Bar.nif")).unwrap();

        insert_placed_child_into_cell_group(
            target,
            0x001000,
            TEMPORARY,
            plain_refr(0x000600, 0x000500, [0.0; 3], [0.0; 3]),
        )
        .unwrap();
        insert_placed_child_into_cell_group(
            target,
            0x001000,
            TEMPORARY,
            plain_refr(0x000601, 0x000501, [0.0; 3], [0.0; 3]),
        )
        .unwrap();
        insert_placed_child_into_cell_group(
            target,
            0x001000,
            TEMPORARY,
            plain_refr(0x000602, 0x000502, [0.0; 3], [0.0; 3]),
        )
        .unwrap();

        let params = base_params(target, 0x001000);
        let plan = build_plan(&params).expect("plan");
        let groups = &plan.cells[0].groups;
        assert_eq!(groups.len(), 2, "same path, different case, collapses to one group");
        assert_eq!(groups[0].model_path, "meshes\\bar.nif");
        assert_eq!(groups[1].model_path, "meshes\\foo.nif");
        assert_eq!(groups[1].instances.len(), 2);
    }

    #[test]
    fn excludes_refs_whose_own_index_does_not_match_the_target_plugin() {
        let target = new_target();
        ensure_interior_cell_and_child_group(target, interior_cell(0x001000, "Cell")).unwrap();
        insert_parsed_record(target, stat_base(0x000500, "meshes\\a.nif")).unwrap();

        // An "override" REFR whose own form id belongs to a different plugin
        // index (e.g. a master-owned ref carried into this plugin's tree) —
        // must be excluded even though it points at an eligible own-plugin
        // base, so XCRI can never desync from the record actually stamped.
        let override_refr = plain_refr(0x01_000600, 0x000500, [0.0; 3], [0.0; 3]);
        insert_placed_child_into_cell_group(target, 0x001000, TEMPORARY, override_refr).unwrap();
        let clean = plain_refr(0x000601, 0x000500, [0.0; 3], [0.0; 3]);
        insert_placed_child_into_cell_group(target, 0x001000, TEMPORARY, clean).unwrap();

        let params = base_params(target, 0x001000);
        let plan = build_plan(&params).expect("plan");
        let refr_ids: Vec<u32> = plan.cells[0]
            .groups
            .iter()
            .flat_map(|g| g.instances.iter().map(|i| i.refr_form_id))
            .collect();
        assert_eq!(refr_ids, vec![0x000601]);
    }

    #[test]
    fn own_index_overflow_is_rejected() {
        let target = new_target();
        for i in 0..300u32 {
            plugin_handle_add_master_native(target, &format!("Master{i}.esm"), None)
                .expect("add master");
        }
        ensure_interior_cell_and_child_group(target, interior_cell(0x001000, "Cell")).unwrap();

        let params = base_params(target, 0x001000);
        let err = build_plan(&params).expect_err("overflow must fail");
        assert!(
            err.to_lowercase().contains("overflow"),
            "error should mention overflow: {err}"
        );
    }

    #[test]
    fn applies_min_eligible_refs_after_eligibility() {
        let target = new_target();
        ensure_interior_cell_and_child_group(target, interior_cell(0x001000, "Cell")).unwrap();
        insert_parsed_record(target, stat_base(0x000500, "meshes\\solo.nif")).unwrap();
        insert_parsed_record(target, stat_base(0x000501, "meshes\\pair.nif")).unwrap();

        insert_placed_child_into_cell_group(
            target,
            0x001000,
            TEMPORARY,
            plain_refr(0x000600, 0x000500, [0.0; 3], [0.0; 3]),
        )
        .unwrap();
        insert_placed_child_into_cell_group(
            target,
            0x001000,
            TEMPORARY,
            plain_refr(0x000601, 0x000501, [0.0; 3], [0.0; 3]),
        )
        .unwrap();
        insert_placed_child_into_cell_group(
            target,
            0x001000,
            TEMPORARY,
            plain_refr(0x000602, 0x000501, [1.0; 3], [0.0; 3]),
        )
        .unwrap();

        let mut params = base_params(target, 0x001000);
        params.min_eligible_refs = 2;
        let plan = build_plan(&params).expect("plan");
        assert_eq!(plan.cells[0].groups.len(), 1);
        assert_eq!(plan.cells[0].groups[0].model_path, "meshes\\pair.nif");
        assert_eq!(plan.cells[0].groups[0].instances.len(), 2);
    }
}
