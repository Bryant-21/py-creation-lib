use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use thiserror::Error;

use crate::model::{NifBlock, NifFile, NifValue};
use crate::weapon_diff::weapon_block_diff;

#[derive(Debug, Error)]
pub enum ExtractAttachmentError {
    #[error("read: {0}")]
    Read(#[from] crate::io::ReadError),
    #[error("write: {0}")]
    Write(#[from] crate::io::WriteError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Default, Clone)]
pub struct ExtractAttachmentReport {
    pub blocks_copied: usize,
    pub changes: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn extract_attachment(
    base_path: &Path,
    sibling_path: &Path,
    slot: u8,
    output_attachment_path: &Path,
    anchor_node_name: &str,
) -> Result<ExtractAttachmentReport, ExtractAttachmentError> {
    let mut report = ExtractAttachmentReport::default();
    let mut base = NifFile::load(base_path.to_path_buf())?;
    let sibling = NifFile::load(sibling_path.to_path_buf())?;

    let diff_ids = weapon_block_diff(&base, &sibling);
    let top_level_ids = top_level_diff_ids(&sibling, &diff_ids);
    if top_level_ids.is_empty() {
        report
            .warnings
            .push(format!("slot {slot}: block diff is empty"));
        return Ok(report);
    }

    let mut attachment = build_attachment_nif(&sibling, &top_level_ids, slot);
    if let Some(parent) = output_attachment_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    attachment.save(Some(PathBuf::from(output_attachment_path)))?;
    report.blocks_copied = top_level_ids.len();
    report.changes.push(format!(
        "slot {slot}: wrote attachment NIF {}",
        output_attachment_path.display()
    ));

    patch_base_connect_points(&mut base, slot, anchor_node_name);
    base.save(Some(PathBuf::from(base_path)))?;
    report.changes.push(format!(
        "slot {slot}: patched base connect point P-Mod{slot}"
    ));
    Ok(report)
}

fn top_level_diff_ids(nif: &NifFile, diff_ids: &[i32]) -> Vec<i32> {
    let diff_set: HashSet<i32> = diff_ids.iter().copied().collect();
    let mut referenced = HashSet::new();
    for diff_id in diff_ids.iter().copied().filter(|id| *id >= 0) {
        let Some(block) = nif.get_block(diff_id as usize) else {
            continue;
        };
        collect_refs_from_value(block.get_field("Children"), &diff_set, &mut referenced);
        for (_, refs) in block.get_all_ref_fields(&crate::schema::SCHEMA) {
            for reference in refs {
                if diff_set.contains(&reference) {
                    referenced.insert(reference);
                }
            }
        }
    }

    diff_ids
        .iter()
        .copied()
        .filter(|id| *id >= 0 && !referenced.contains(id))
        .collect()
}

fn collect_refs_from_value(
    value: Option<&NifValue>,
    diff_set: &HashSet<i32>,
    referenced: &mut HashSet<i32>,
) {
    let Some(value) = value else {
        return;
    };
    match value {
        NifValue::Ref(id) if diff_set.contains(id) => {
            referenced.insert(*id);
        }
        NifValue::Array(items) => {
            for item in items {
                collect_refs_from_value(Some(item), diff_set, referenced);
            }
        }
        NifValue::Struct(fields) => {
            for item in fields.values() {
                collect_refs_from_value(Some(item), diff_set, referenced);
            }
        }
        _ => {}
    }
}

fn build_attachment_nif(sibling: &NifFile, top_level_ids: &[i32], slot: u8) -> NifFile {
    let mut out = NifFile::new("fo4");
    if let Some(root) = out.blocks.get_mut(0) {
        root.type_name = "NiNode".to_string();
        root.set_field("Name", NifValue::String(format!("##Mod{slot}")));
    }

    let mut id_map = HashMap::new();
    let mut visited = HashSet::new();
    let mut child_refs = Vec::new();
    for source_id in top_level_ids.iter().copied() {
        if let Some(new_id) = copy_block_recursive(
            sibling,
            &mut out,
            source_id as usize,
            &mut id_map,
            &mut visited,
        ) {
            child_refs.push(NifValue::Ref(new_id as i32));
        }
    }

    if let Some(root) = out.blocks.get_mut(0) {
        root.set_field("Num Children", NifValue::UInt(child_refs.len() as u64));
        root.set_field("Children", NifValue::Array(child_refs));
    }

    let mut child_fields = IndexMap::new();
    child_fields.insert("Skinned".to_string(), NifValue::Bool(false));
    child_fields.insert("Num Points".to_string(), NifValue::UInt(1));
    child_fields.insert(
        "Point Name".to_string(),
        NifValue::Array(vec![NifValue::String(format!("C-Mod{slot}"))]),
    );
    let child_block_id = out.add_block("BSConnectPoint::Children", Some(child_fields));
    attach_extra(&mut out, 0, child_block_id);
    out.header.footer_roots = vec![0];
    out.rebuild_header();
    out
}

fn copy_block_recursive(
    sibling: &NifFile,
    out: &mut NifFile,
    source_id: usize,
    id_map: &mut HashMap<usize, usize>,
    visited: &mut HashSet<usize>,
) -> Option<usize> {
    if let Some(existing) = id_map.get(&source_id).copied() {
        return Some(existing);
    }
    if !visited.insert(source_id) {
        return id_map.get(&source_id).copied();
    }
    let source_block = sibling.get_block(source_id)?.clone();
    let new_id = out.add_block(source_block.type_name.clone(), None);
    id_map.insert(source_id, new_id);

    let mut new_fields = IndexMap::new();
    for (key, value) in source_block.fields.iter() {
        new_fields.insert(
            key.clone(),
            remap_value_refs(sibling, out, value, id_map, visited),
        );
    }
    if let Some(block) = out.blocks.get_mut(new_id) {
        block.fields = new_fields;
        block.remainder = source_block.remainder.clone();
    }
    Some(new_id)
}

fn remap_value_refs(
    sibling: &NifFile,
    out: &mut NifFile,
    value: &NifValue,
    id_map: &mut HashMap<usize, usize>,
    visited: &mut HashSet<usize>,
) -> NifValue {
    match value {
        NifValue::Ref(id) if *id >= 0 => {
            let new_id = copy_block_recursive(sibling, out, *id as usize, id_map, visited);
            NifValue::Ref(new_id.map_or(-1, |mapped| mapped as i32))
        }
        NifValue::Array(items) => NifValue::Array(
            items
                .iter()
                .map(|item| remap_value_refs(sibling, out, item, id_map, visited))
                .collect(),
        ),
        NifValue::Struct(fields) => {
            let mut out_fields = IndexMap::new();
            for (key, item) in fields.iter() {
                out_fields.insert(
                    key.clone(),
                    remap_value_refs(sibling, out, item, id_map, visited),
                );
            }
            NifValue::Struct(out_fields)
        }
        other => other.clone(),
    }
}

fn patch_base_connect_points(base: &mut NifFile, slot: u8, anchor_node_name: &str) {
    let root_id = base
        .header
        .footer_roots
        .iter()
        .find_map(|id| (*id >= 0).then_some(*id as usize))
        .unwrap_or(0);
    let parent_block_id = ensure_parent_connect_point_block(base, root_id);
    let Some(parent_block) = base.blocks.get_mut(parent_block_id) else {
        return;
    };
    let mut connect_points = match parent_block.get_field("Connect Points").cloned() {
        Some(NifValue::Array(items)) => items,
        _ => Vec::new(),
    };
    let point_name = format!("P-Mod{slot}");
    let exists = connect_points.iter().any(|item| match item {
        NifValue::Struct(fields) => matches!(
            fields.get("Name"),
            Some(NifValue::String(name)) if name == &point_name
        ),
        _ => false,
    });
    if !exists {
        let mut point = IndexMap::new();
        point.insert(
            "Parent".to_string(),
            NifValue::String(anchor_node_name.to_string()),
        );
        point.insert("Name".to_string(), NifValue::String(point_name));
        point.insert(
            "Rotation".to_string(),
            NifValue::Quaternion([0.0, 0.0, 0.0, 1.0]),
        );
        point.insert("Translation".to_string(), NifValue::Vec3([0.0, 0.0, 0.0]));
        point.insert("Scale".to_string(), NifValue::Float(1.0));
        connect_points.push(NifValue::Struct(point));
    }
    parent_block.set_field(
        "Num Connect Points",
        NifValue::UInt(connect_points.len() as u64),
    );
    parent_block.set_field("Connect Points", NifValue::Array(connect_points));
    base.rebuild_header();
}

fn ensure_parent_connect_point_block(base: &mut NifFile, root_id: usize) -> usize {
    let extra_ids = base
        .get_block(root_id)
        .and_then(|root| root.get_field("Extra Data List"))
        .map(|value| match value {
            NifValue::Array(items) => items
                .iter()
                .filter_map(|item| match item {
                    NifValue::Ref(id) if *id >= 0 => Some(*id as usize),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .unwrap_or_default();
    for extra_id in extra_ids {
        let Some(extra) = base.get_block(extra_id) else {
            continue;
        };
        if extra.type_name == "BSConnectPoint::Parents" {
            return extra_id;
        }
    }

    let mut fields = IndexMap::new();
    fields.insert("Num Connect Points".to_string(), NifValue::UInt(0));
    fields.insert("Connect Points".to_string(), NifValue::Array(Vec::new()));
    let block_id = base.add_block("BSConnectPoint::Parents", Some(fields));
    attach_extra(base, root_id, block_id);
    block_id
}

fn attach_extra(nif: &mut NifFile, root_id: usize, extra_id: usize) {
    let mut extra_ids = nif
        .get_block(root_id)
        .and_then(|root| root.get_field("Extra Data List"))
        .and_then(|value| match value {
            NifValue::Array(items) => Some(
                items
                    .iter()
                    .filter_map(|item| match item {
                        NifValue::Ref(id) if *id >= 0 => Some(*id),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    if !extra_ids.contains(&(extra_id as i32)) {
        extra_ids.push(extra_id as i32);
    }
    if let Some(root) = nif.blocks.get_mut(root_id) {
        root.set_field(
            "Extra Data List",
            NifValue::Array(extra_ids.iter().copied().map(NifValue::Ref).collect()),
        );
        root.set_field(
            "Num Extra Data List",
            NifValue::UInt(extra_ids.len() as u64),
        );
    }
}
