use bytes::Bytes;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use smol_str::SmolStr;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::plugin_runtime::{
    LocalizedStringsState, ParsedItem, ParsedPlugin, ParsedRecord, ParsedSubrecord,
    parse_plugin_file_eager_compressed, remap_formids_in_items, save_parsed_plugin, strings,
};

const TES4_FLAG_LIGHT_PLUGIN: u32 = 0x0000_0200;
const TES4_FLAG_LOCALIZED: u32 = 0x0000_0080;
pub const RECORD_FLAG_NO_PREVIS: u32 = 0x0000_0080;
const CELL_CHILD_GROUP: i32 = 6;
const PERSISTENT_GROUP: i32 = 8;
const TEMPORARY_GROUP: i32 = 9;
const EMPTY_XPRI_FORMID: u32 = 0x001C_26C2;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct FormKey {
    plugin_lc: String,
    object_id: u32,
}

#[derive(Clone)]
struct SourceContext {
    masters: Vec<String>,
    plugin_name: String,
    self_owner_name: String,
    own_index: u8,
    is_light_plugin: bool,
}

#[derive(Clone)]
struct SourceCell {
    record: ParsedRecord,
    context: SourceContext,
}

#[derive(Default)]
struct MergeStats {
    combined_cells: usize,
    previs_cells: usize,
    missing_uvds: usize,
    removed_refrs: usize,
    warnings: Vec<String>,
}

impl MergeStats {
    fn into_payload(self) -> (usize, usize, usize, usize, Vec<String>) {
        (
            self.combined_cells,
            self.previs_cells,
            self.missing_uvds,
            self.removed_refrs,
            self.warnings,
        )
    }
}

#[pyfunction(name = "merge_previs_native")]
#[pyo3(signature = (target_plugin_path, combined_objects_path=None, previs_plugin_path=None, game=None, data_dir=None))]
pub fn merge_previs_native(
    py: Python<'_>,
    target_plugin_path: &str,
    combined_objects_path: Option<&str>,
    previs_plugin_path: Option<&str>,
    game: Option<&str>,
    data_dir: Option<&str>,
) -> PyResult<(usize, usize, usize, usize, Vec<String>)> {
    let target_path = target_plugin_path.to_string();
    let combined_path = combined_objects_path
        .filter(|path| !path.is_empty())
        .map(str::to_string);
    let previs_path = previs_plugin_path
        .filter(|path| !path.is_empty())
        .map(str::to_string);
    let game = game.map(str::to_string);
    let data_dir = data_dir.map(PathBuf::from);
    py.detach(move || {
        merge_previs_files(
            Path::new(&target_path),
            combined_path.as_deref().map(Path::new),
            previs_path.as_deref().map(Path::new),
            game,
            data_dir.as_deref(),
        )
        .map(MergeStats::into_payload)
    })
}

fn merge_previs_files(
    target_path: &Path,
    combined_path: Option<&Path>,
    previs_path: Option<&Path>,
    game: Option<String>,
    data_dir: Option<&Path>,
) -> PyResult<MergeStats> {
    if combined_path.is_none() && previs_path.is_none() {
        return Err(PyValueError::new_err(
            "at least one of combined_objects_path or previs_plugin_path is required",
        ));
    }

    let mut target = parse_plugin_file_eager_compressed(path_str(target_path)?, game.clone())?;

    let target_plugin_name = target.plugin_name.clone();
    let combined_cells = if let Some(combined_path) = combined_path {
        let combined = parse_plugin_file_eager_compressed(path_str(combined_path)?, game.clone())?;
        collect_source_cells(&combined, &target_plugin_name)
    } else {
        HashMap::new()
    };
    let previs_cells = if let Some(previs_path) = previs_path {
        let previs = parse_plugin_file_eager_compressed(path_str(previs_path)?, game)?;
        collect_source_cells(&previs, &target_plugin_name)
    } else {
        HashMap::new()
    };

    ensure_required_masters(
        &mut target,
        combined_cells.values().chain(previs_cells.values()),
    );

    let mut stats = MergeStats::default();
    let mut expected_uvds = HashSet::new();
    let target_context = SourceContext {
        masters: target.header.masters.clone(),
        plugin_name: target.plugin_name.clone(),
        self_owner_name: target.plugin_name.clone(),
        own_index: target.header.masters.len() as u8,
        is_light_plugin: (target.header.flags & TES4_FLAG_LIGHT_PLUGIN) != 0,
    };

    merge_cells_in_items(
        &mut target.root_items,
        &target_context,
        &combined_cells,
        &previs_cells,
        data_dir,
        &mut expected_uvds,
        &mut stats,
    );
    stats.removed_refrs += remove_please_remove_refs(&mut target.root_items);
    if stats.removed_refrs > 0 {
        target.header.raw_subrecords.clear();
    }

    let strings = if (target.header.flags & TES4_FLAG_LOCALIZED) != 0 {
        strings::hydrate_strings_state(path_str(target_path)?, &target.plugin_name, None, None)
    } else {
        LocalizedStringsState::default()
    };
    save_parsed_plugin(&mut target, &strings, path_str(target_path)?)?;
    Ok(stats)
}

fn path_str(path: &Path) -> PyResult<&str> {
    path.to_str().ok_or_else(|| {
        PyValueError::new_err(format!("path is not valid UTF-8: {}", path.display()))
    })
}

fn source_context(plugin: &ParsedPlugin, target_plugin_name: &str) -> SourceContext {
    let generated = matches!(
        plugin.plugin_name.to_ascii_lowercase().as_str(),
        "combinedobjects.esp" | "previs.esp"
    );
    SourceContext {
        masters: plugin.header.masters.clone(),
        plugin_name: plugin.plugin_name.clone(),
        self_owner_name: if generated {
            target_plugin_name.to_string()
        } else {
            plugin.plugin_name.clone()
        },
        own_index: plugin.header.masters.len() as u8,
        is_light_plugin: (plugin.header.flags & TES4_FLAG_LIGHT_PLUGIN) != 0,
    }
}

fn collect_source_cells(
    plugin: &ParsedPlugin,
    target_plugin_name: &str,
) -> HashMap<FormKey, SourceCell> {
    let context = source_context(plugin, target_plugin_name);
    let mut cells = HashMap::new();
    collect_source_cells_in_items(&plugin.root_items, &context, &mut cells);
    cells
}

fn collect_source_cells_in_items(
    items: &[ParsedItem],
    context: &SourceContext,
    cells: &mut HashMap<FormKey, SourceCell>,
) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "CELL" => {
                if let Some(key) = form_key_for_raw(record.form_id, context) {
                    cells.insert(
                        key,
                        SourceCell {
                            record: record.clone(),
                            context: context.clone(),
                        },
                    );
                }
            }
            ParsedItem::Group(group) => {
                collect_source_cells_in_items(&group.children, context, cells)
            }
            ParsedItem::Record(_) => {}
        }
    }
}

fn form_key_for_raw(raw: u32, context: &SourceContext) -> Option<FormKey> {
    let source_index = ((raw >> 24) & 0xFF) as u8;
    let object_id = raw & 0x00FF_FFFF;
    let owner = if source_index == context.own_index || source_index == 0xFF {
        Some(context.self_owner_name.as_str())
    } else {
        context
            .masters
            .get(source_index as usize)
            .map(|value| value.as_str())
    }?;
    Some(FormKey {
        plugin_lc: owner.to_ascii_lowercase(),
        object_id,
    })
}

fn ensure_required_masters<'a>(
    target: &mut ParsedPlugin,
    cells: impl Iterator<Item = &'a SourceCell>,
) {
    let mut required = Vec::new();
    for cell in cells {
        collect_required_owner_names(&cell.record, &cell.context, &mut required);
    }
    if required.is_empty() {
        return;
    }

    let old_masters = target.header.masters.clone();
    let target_name = target.plugin_name.clone();
    for owner in required {
        if owner.eq_ignore_ascii_case(&target_name) {
            continue;
        }
        if !target
            .header
            .masters
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(&owner))
        {
            target.header.masters.push(owner);
            target.header.master_sizes.push(0);
        }
    }

    if target.header.masters != old_masters {
        let old_own_index = old_masters.len() as u8;
        let new_own_index = target.header.masters.len() as u8;
        remap_formids_in_items(
            &mut target.root_items,
            &old_masters,
            &target.header.masters,
            old_own_index,
            new_own_index,
        );
        for raw in &mut target.header.overridden_forms {
            *raw = remap_formid_between_master_lists(
                *raw,
                &old_masters,
                &target.header.masters,
                old_own_index,
                new_own_index,
            );
        }
        target.header.raw_subrecords.clear();
    }
}

fn collect_required_owner_names(
    record: &ParsedRecord,
    context: &SourceContext,
    required: &mut Vec<String>,
) {
    for subrecord in &record.subrecords {
        match subrecord.signature.as_str() {
            "RVIS" if subrecord.data.len() >= 4 => {
                push_owner_name(read_u32(&subrecord.data, 0), context, required);
            }
            "XPRI" => {
                for offset in (0..subrecord.data.len()).step_by(4) {
                    if offset + 4 <= subrecord.data.len() {
                        push_owner_name(read_u32(&subrecord.data, offset), context, required);
                    }
                }
            }
            "XCRI" => collect_xcri_owner_names(&subrecord.data, context, required),
            _ => {}
        }
    }
}

fn collect_xcri_owner_names(data: &[u8], context: &SourceContext, required: &mut Vec<String>) {
    let Some((reference_start, reference_count)) = xcri_reference_range(data) else {
        return;
    };
    for index in 0..reference_count {
        let offset = reference_start + index * 8;
        push_owner_name(read_u32(data, offset), context, required);
    }
}

fn push_owner_name(raw: u32, context: &SourceContext, required: &mut Vec<String>) {
    if raw == 0 || raw == u32::MAX {
        return;
    }
    let source_index = ((raw >> 24) & 0xFF) as u8;
    let owner = if source_index == context.own_index || source_index == 0xFF {
        Some(context.self_owner_name.as_str())
    } else {
        context
            .masters
            .get(source_index as usize)
            .map(|value| value.as_str())
    };
    let Some(owner) = owner else {
        return;
    };
    if !required
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(owner))
    {
        required.push(owner.to_string());
    }
}

fn merge_cells_in_items(
    items: &mut [ParsedItem],
    target_context: &SourceContext,
    combined_cells: &HashMap<FormKey, SourceCell>,
    previs_cells: &HashMap<FormKey, SourceCell>,
    data_dir: Option<&Path>,
    expected_uvds: &mut HashSet<String>,
    stats: &mut MergeStats,
) {
    for item in items {
        match item {
            ParsedItem::Record(record) if record.signature.as_str() == "CELL" => {
                let Some(key) = form_key_for_raw(record.form_id, target_context) else {
                    continue;
                };
                let had_xcri = has_subrecord(record, "XCRI");
                let had_xpri = has_subrecord(record, "XPRI");
                if let Some(source) = combined_cells.get(&key) {
                    if merge_combined_cell(record, source, target_context, had_xcri) {
                        stats.combined_cells += 1;
                    }
                } else if has_subrecord(record, "PCMB")
                    && !has_subrecord(record, "XCRI")
                    && had_xcri
                {
                    upsert_ordered_subrecord(record, empty_xcri_subrecord());
                    record.raw_payload = None;
                }

                if let Some(source) = previs_cells.get(&key) {
                    if merge_previs_cell(
                        record,
                        source,
                        target_context,
                        had_xpri,
                        data_dir,
                        expected_uvds,
                        stats,
                    ) {
                        stats.previs_cells += 1;
                    }
                }
            }
            ParsedItem::Group(group) => merge_cells_in_items(
                &mut group.children,
                target_context,
                combined_cells,
                previs_cells,
                data_dir,
                expected_uvds,
                stats,
            ),
            ParsedItem::Record(_) => {}
        }
    }
}

fn merge_combined_cell(
    target: &mut ParsedRecord,
    source: &SourceCell,
    target_context: &SourceContext,
    target_had_xcri: bool,
) -> bool {
    let source_record = &source.record;
    let pcmb = find_subrecord(source_record, "PCMB")
        .map(|subrecord| copy_subrecord(subrecord, &source.context, target_context));
    let xcri = find_subrecord(source_record, "XCRI")
        .map(|subrecord| copy_subrecord(subrecord, &source.context, target_context));

    remove_subrecords(target, &["PCMB", "XPRI", "XCRI"]);
    let changed = true;
    if let Some(pcmb) = pcmb {
        upsert_ordered_subrecord(target, pcmb);
    }
    if let Some(xcri) = xcri {
        if xcri_reference_count(&xcri.data) > 0 {
            target.flags &= !RECORD_FLAG_NO_PREVIS;
        }
        upsert_ordered_subrecord(target, xcri);
    } else if has_subrecord(target, "PCMB") && target_had_xcri {
        upsert_ordered_subrecord(target, empty_xcri_subrecord());
    }

    target.version_control = source_record.version_control;
    target.form_version = source_record.form_version;
    target.version2 = source_record.version2;
    target.raw_payload = None;
    changed
}

fn merge_previs_cell(
    target: &mut ParsedRecord,
    source: &SourceCell,
    target_context: &SourceContext,
    target_had_xpri: bool,
    data_dir: Option<&Path>,
    expected_uvds: &mut HashSet<String>,
    stats: &mut MergeStats,
) -> bool {
    let source_record = &source.record;
    let source_rvis = find_subrecord(source_record, "RVIS")
        .map(|subrecord| copy_subrecord(subrecord, &source.context, target_context));
    let source_visi = find_subrecord(source_record, "VISI")
        .map(|subrecord| copy_subrecord(subrecord, &source.context, target_context));
    let source_xpri = find_subrecord(source_record, "XPRI")
        .map(|subrecord| copy_subrecord(subrecord, &source.context, target_context));

    let mut uvd_object_id = target.form_id & 0x00FF_FFFF;
    if !is_interior_cell(target) {
        if !has_subrecord(target, "RVIS") {
            if let Some(rvis) = source_rvis.clone() {
                upsert_ordered_subrecord(target, rvis);
            }
        }
        let Some(rvis) = find_subrecord(target, "RVIS") else {
            remove_subrecords(target, &["VISI", "XPRI"]);
            target.raw_payload = None;
            return true;
        };
        if rvis.data.len() >= 4 {
            uvd_object_id = read_u32(&rvis.data, 0) & 0x00FF_FFFF;
        }
    }

    remove_subrecords(target, &["VISI", "XPRI"]);
    let Some(visi) = source_visi else {
        target.raw_payload = None;
        return true;
    };

    let uvd_rel = uvd_relative_path(
        target_context.plugin_name.as_str(),
        uvd_object_id,
        target_context.is_light_plugin,
    );
    if let Some(data_dir) = data_dir {
        if !asset_exists(data_dir, &uvd_rel) {
            stats.missing_uvds += 1;
            stats.warnings.push(format!(
                "missing expected uvd for CELL {:08X}: {}",
                target.form_id, uvd_rel
            ));
            target.raw_payload = None;
            return true;
        }
    }
    expected_uvds.insert(uvd_rel.to_ascii_lowercase());

    upsert_ordered_subrecord(target, visi);
    if let Some(xpri) = source_xpri {
        upsert_ordered_subrecord(target, xpri);
    } else if (target.flags & RECORD_FLAG_NO_PREVIS) == 0 && target_had_xpri {
        upsert_ordered_subrecord(target, dummy_xpri_subrecord());
    }
    target.raw_payload = None;
    true
}

fn uvd_relative_path(plugin_name: &str, object_id: u32, light_plugin: bool) -> String {
    let file_name = if light_plugin {
        format!("{:08X}.uvd", object_id & 0x0000_0FFF)
    } else {
        format!("00{:06X}.uvd", object_id & 0x00FF_FFFF)
    };
    format!("Vis/{plugin_name}/{file_name}")
}

fn asset_exists(data_dir: &Path, rel: &str) -> bool {
    let candidate = data_dir.join(rel);
    if candidate.is_file() {
        return true;
    }
    data_dir.join(rel.to_ascii_lowercase()).is_file()
}

fn copy_subrecord(
    subrecord: &ParsedSubrecord,
    source_context: &SourceContext,
    target_context: &SourceContext,
) -> ParsedSubrecord {
    let data = match subrecord.signature.as_str() {
        "RVIS" => remap_formid_scalar(&subrecord.data, source_context, target_context),
        "XPRI" => remap_formid_array(&subrecord.data, source_context, target_context),
        "XCRI" => remap_xcri(&subrecord.data, source_context, target_context),
        _ => subrecord.data.clone(),
    };
    ParsedSubrecord {
        signature: subrecord.signature.clone(),
        data,
        semantic_type: subrecord.semantic_type.clone(),
    }
}

fn remap_formid_scalar(
    data: &Bytes,
    source_context: &SourceContext,
    target_context: &SourceContext,
) -> Bytes {
    if data.len() < 4 {
        return data.clone();
    }
    let raw = read_u32(data, 0);
    let remapped = remap_formid_for_copy(raw, source_context, target_context);
    if remapped == raw {
        return data.clone();
    }
    let mut out = data.to_vec();
    out[0..4].copy_from_slice(&remapped.to_le_bytes());
    Bytes::from(out)
}

fn remap_formid_array(
    data: &Bytes,
    source_context: &SourceContext,
    target_context: &SourceContext,
) -> Bytes {
    if data.len() < 4 || data.len() % 4 != 0 {
        return data.clone();
    }
    let needs_remap = data.chunks_exact(4).any(|chunk| {
        let raw = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        remap_formid_for_copy(raw, source_context, target_context) != raw
    });
    if !needs_remap {
        return data.clone();
    }
    let mut out = data.to_vec();
    for chunk in out.chunks_exact_mut(4) {
        let raw = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        chunk.copy_from_slice(
            &remap_formid_for_copy(raw, source_context, target_context).to_le_bytes(),
        );
    }
    Bytes::from(out)
}

fn remap_xcri(
    data: &Bytes,
    source_context: &SourceContext,
    target_context: &SourceContext,
) -> Bytes {
    let Some((reference_start, reference_count)) = xcri_reference_range(data) else {
        return data.clone();
    };
    let needs_remap = (0..reference_count).any(|index| {
        let offset = reference_start + index * 8;
        let raw = read_u32(data, offset);
        remap_formid_for_copy(raw, source_context, target_context) != raw
    });
    if !needs_remap {
        return data.clone();
    }
    let mut out = data.to_vec();
    for index in 0..reference_count {
        let offset = reference_start + index * 8;
        let raw = read_u32(&out, offset);
        out[offset..offset + 4].copy_from_slice(
            &remap_formid_for_copy(raw, source_context, target_context).to_le_bytes(),
        );
    }
    Bytes::from(out)
}

fn remap_formid_for_copy(
    raw: u32,
    source_context: &SourceContext,
    target_context: &SourceContext,
) -> u32 {
    if raw == 0 || raw == u32::MAX {
        return raw;
    }
    let source_index = ((raw >> 24) & 0xFF) as u8;
    let object_id = raw & 0x00FF_FFFF;
    let owner = if source_index == source_context.own_index || source_index == 0xFF {
        Some(source_context.self_owner_name.as_str())
    } else {
        source_context
            .masters
            .get(source_index as usize)
            .map(|value| value.as_str())
    };
    let Some(owner) = owner else {
        return raw;
    };
    if owner.eq_ignore_ascii_case(&target_context.plugin_name) {
        return ((target_context.own_index as u32) << 24) | object_id;
    }
    target_context
        .masters
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(owner))
        .map(|target_index| ((target_index as u32) << 24) | object_id)
        .unwrap_or(raw)
}

fn remap_formid_between_master_lists(
    raw: u32,
    old_masters: &[String],
    new_masters: &[String],
    old_own_index: u8,
    new_own_index: u8,
) -> u32 {
    if raw == 0 || raw == u32::MAX {
        return raw;
    }
    let old_index = ((raw >> 24) & 0xFF) as u8;
    let object_id = raw & 0x00FF_FFFF;
    if old_index == old_own_index {
        return ((new_own_index as u32) << 24) | object_id;
    }
    if let Some(owner) = old_masters.get(old_index as usize) {
        return new_masters
            .iter()
            .position(|candidate| candidate.eq_ignore_ascii_case(owner))
            .map(|new_index| ((new_index as u32) << 24) | object_id)
            .unwrap_or(raw);
    }
    raw
}

// FO4's XCRI `reference_count` header field counts u32 *words* (2x the
// logical reference-row count) — see `crate::xcri` for the grounded byte
// layout. This helper stays row-oriented for its byte-offset callers
// (`collect_xcri_owner_names`, `remap_xcri`) by deriving the reference
// section's start offset and logical row count from the shared codec.
fn xcri_reference_range(data: &[u8]) -> Option<(usize, usize)> {
    let table = crate::xcri::decode_fo4(data)?;
    let reference_start = 8 + table.meshes.len() * 4;
    Some((reference_start, table.references.len()))
}

fn xcri_reference_count(data: &[u8]) -> u32 {
    crate::xcri::decode_fo4(data)
        .map(|table| table.references.len() as u32)
        .unwrap_or(0)
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn find_subrecord<'a>(record: &'a ParsedRecord, signature: &str) -> Option<&'a ParsedSubrecord> {
    record
        .subrecords
        .iter()
        .find(|subrecord| subrecord.signature.as_str() == signature)
}

fn has_subrecord(record: &ParsedRecord, signature: &str) -> bool {
    find_subrecord(record, signature).is_some()
}

fn remove_subrecords(record: &mut ParsedRecord, signatures: &[&str]) {
    record.subrecords.retain(|subrecord| {
        !signatures
            .iter()
            .any(|signature| subrecord.signature.as_str() == *signature)
    });
}

pub fn upsert_ordered_subrecord(record: &mut ParsedRecord, subrecord: ParsedSubrecord) {
    remove_subrecords(record, &[subrecord.signature.as_str()]);
    let rank = cell_subrecord_rank(subrecord.signature.as_str());
    let Some(rank) = rank else {
        record.subrecords.push(subrecord);
        return;
    };
    let insert_at = record
        .subrecords
        .iter()
        .position(|existing| {
            cell_subrecord_rank(existing.signature.as_str()).is_some_and(|r| r > rank)
        })
        .unwrap_or(record.subrecords.len());
    record.subrecords.insert(insert_at, subrecord);
}

pub fn cell_subrecord_rank(signature: &str) -> Option<usize> {
    const ORDER: &[&str] = &[
        "EDID", "FULL", "DATA", "VISI", "RVIS", "PCMB", "XCLC", "XCLL", "XCLW", "XCLR", "XCLT",
        "XCLF", "XCAS", "XCMO", "XCIM", "XCWT", "XCMT", "XEZN", "XLCN", "XPRI", "XCRI", "XOWN",
        "XRNK", "XGLB",
    ];
    ORDER.iter().position(|candidate| *candidate == signature)
}

fn empty_xcri_subrecord() -> ParsedSubrecord {
    let mut data = Vec::with_capacity(8);
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    ParsedSubrecord {
        signature: SmolStr::new_static("XCRI"),
        data: Bytes::from(data),
        semantic_type: None,
    }
}

fn dummy_xpri_subrecord() -> ParsedSubrecord {
    ParsedSubrecord {
        signature: SmolStr::new_static("XPRI"),
        data: Bytes::from(EMPTY_XPRI_FORMID.to_le_bytes().to_vec()),
        semantic_type: Some("formid_array".to_string()),
    }
}

fn is_interior_cell(record: &ParsedRecord) -> bool {
    find_subrecord(record, "DATA")
        .and_then(|data| data.data.first().copied())
        .is_some_and(|flags| (flags & 0x01) != 0)
}

fn remove_please_remove_refs(items: &mut [ParsedItem]) -> usize {
    let mut removed = 0;
    for item in items {
        match item {
            ParsedItem::Group(group) => {
                if group.group_type == CELL_CHILD_GROUP {
                    removed += remove_please_remove_refs_from_cell_child_group(&mut group.children);
                }
                removed += remove_please_remove_refs(&mut group.children);
            }
            ParsedItem::Record(_) => {}
        }
    }
    removed
}

fn remove_please_remove_refs_from_cell_child_group(items: &mut [ParsedItem]) -> usize {
    let mut removed = 0;
    for item in items {
        let ParsedItem::Group(group) = item else {
            continue;
        };
        if !matches!(group.group_type, PERSISTENT_GROUP | TEMPORARY_GROUP) {
            continue;
        }
        let before = group.children.len();
        group.children.retain(|child| !is_please_remove_refr(child));
        removed += before - group.children.len();
    }
    removed
}

fn is_please_remove_refr(item: &ParsedItem) -> bool {
    let ParsedItem::Record(record) = item else {
        return false;
    };
    if record.signature.as_str() != "REFR" {
        return false;
    }
    let Some(edid) = find_subrecord(record, "EDID") else {
        return false;
    };
    let text = String::from_utf8_lossy(&edid.data);
    text.trim_end_matches('\0')
        .get(0..12)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("pleaseremove"))
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

    fn cell(form_id: u32, subrecords: Vec<ParsedSubrecord>) -> ParsedRecord {
        ParsedRecord {
            signature: SmolStr::new_static("CELL"),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: Some(131),
            version2: Some(0),
            subrecords,
            raw_payload: None,
            parse_error: None,
        }
    }

    fn context(plugin_name: &str, masters: Vec<&str>) -> SourceContext {
        SourceContext {
            masters: masters.into_iter().map(str::to_string).collect(),
            plugin_name: plugin_name.to_string(),
            self_owner_name: plugin_name.to_string(),
            own_index: 0,
            is_light_plugin: false,
        }
    }

    fn source_cell(record: ParsedRecord, context: SourceContext) -> SourceCell {
        SourceCell { record, context }
    }

    fn xcri_with_reference(raw_ref: u32, mesh_id: u32) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_le_bytes()); // mesh_count = 1
        data.extend_from_slice(&2u32.to_le_bytes()); // reference_count field = 2x1 row
        data.extend_from_slice(&mesh_id.to_le_bytes());
        data.extend_from_slice(&raw_ref.to_le_bytes());
        data.extend_from_slice(&mesh_id.to_le_bytes());
        data
    }

    #[test]
    fn combined_merge_remaps_xcri_and_clears_no_previs() {
        let mut target = cell(
            0x02_000800,
            vec![
                sub("DATA", vec![1, 0]),
                sub("PCMB", vec![0xAA, 0xAA]),
                sub("XPRI", 0x02_000901u32.to_le_bytes().to_vec()),
            ],
        );
        target.flags = RECORD_FLAG_NO_PREVIS;
        let mut combined = cell(
            0x01_000800,
            vec![
                sub("PCMB", vec![0x01, 0x02]),
                sub("XCRI", xcri_with_reference(0x01_000901, 0x1234)),
            ],
        );
        combined.version_control = 0xAABB_CCDD;
        combined.form_version = Some(132);
        combined.version2 = Some(7);

        let mut source_context = context("CombinedObjects.esp", vec!["Fallout4.esm", "MyMod.esp"]);
        source_context.self_owner_name = "MyMod.esp".to_string();
        source_context.own_index = 2;
        let mut target_context = context("MyMod.esp", vec!["Fallout4.esm", "DLCRobot.esm"]);
        target_context.own_index = 2;

        let source = source_cell(combined, source_context);
        assert!(merge_combined_cell(
            &mut target,
            &source,
            &target_context,
            false
        ));

        assert!(has_subrecord(&target, "PCMB"));
        assert!(!has_subrecord(&target, "XPRI"));
        assert_eq!(target.flags & RECORD_FLAG_NO_PREVIS, 0);
        assert_eq!(target.version_control, 0xAABB_CCDD);
        assert_eq!(target.form_version, Some(132));
        assert_eq!(target.version2, Some(7));

        let xcri = find_subrecord(&target, "XCRI").expect("merged XCRI");
        let reference_start = 8 + 4;
        assert_eq!(read_u32(&xcri.data, reference_start), 0x02_000901);
    }

    #[test]
    fn previs_merge_requires_uvd_and_adds_dummy_xpri() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let vis_dir = tmp.path().join("Vis").join("MyMod.esp");
        std::fs::create_dir_all(&vis_dir).expect("vis dir");
        std::fs::write(vis_dir.join("00000800.uvd"), b"uvd").expect("uvd");

        let mut target = cell(
            0x01_000800,
            vec![
                sub("DATA", vec![1, 0]),
                sub("XPRI", 0x01_000901u32.to_le_bytes().to_vec()),
            ],
        );
        let previs = cell(0x01_000800, vec![sub("VISI", vec![0x11, 0x22])]);
        let mut source_context = context("PreVis.esp", vec!["Fallout4.esm", "MyMod.esp"]);
        source_context.self_owner_name = "MyMod.esp".to_string();
        source_context.own_index = 2;
        let mut target_context = context("MyMod.esp", vec!["Fallout4.esm"]);
        target_context.own_index = 1;
        let source = source_cell(previs, source_context);
        let mut stats = MergeStats::default();
        let mut expected = HashSet::new();

        assert!(merge_previs_cell(
            &mut target,
            &source,
            &target_context,
            true,
            Some(tmp.path()),
            &mut expected,
            &mut stats,
        ));

        assert!(has_subrecord(&target, "VISI"));
        let xpri = find_subrecord(&target, "XPRI").expect("dummy XPRI");
        assert_eq!(read_u32(&xpri.data, 0), EMPTY_XPRI_FORMID);
        assert_eq!(stats.missing_uvds, 0);
    }

    #[test]
    fn previs_merge_clears_visi_when_uvd_is_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut target = cell(
            0x01_000800,
            vec![sub("DATA", vec![1, 0]), sub("VISI", vec![0xAA, 0xBB])],
        );
        let previs = cell(0x01_000800, vec![sub("VISI", vec![0x11, 0x22])]);
        let mut source_context = context("PreVis.esp", vec!["Fallout4.esm", "MyMod.esp"]);
        source_context.self_owner_name = "MyMod.esp".to_string();
        source_context.own_index = 2;
        let mut target_context = context("MyMod.esp", vec!["Fallout4.esm"]);
        target_context.own_index = 1;
        let source = source_cell(previs, source_context);
        let mut stats = MergeStats::default();
        let mut expected = HashSet::new();

        assert!(merge_previs_cell(
            &mut target,
            &source,
            &target_context,
            false,
            Some(tmp.path()),
            &mut expected,
            &mut stats,
        ));

        assert!(!has_subrecord(&target, "VISI"));
        assert_eq!(stats.missing_uvds, 1);
    }
}
