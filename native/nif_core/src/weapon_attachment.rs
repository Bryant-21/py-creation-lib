use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use thiserror::Error;

use crate::model::{NifFile, NifValue};
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
            NifValue::Quaternion([1.0, 0.0, 0.0, 0.0]),
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

pub fn upsert_parent_connect_point_translation(
    nif: &mut NifFile,
    point_name: &str,
    translation: [f32; 3],
) -> bool {
    let root_id = nif
        .header
        .footer_roots
        .iter()
        .find_map(|id| (*id >= 0).then_some(*id as usize))
        .unwrap_or(0);
    let existing_parent_block = nif
        .get_block(root_id)
        .and_then(|root| root.get_field("Extra Data List"))
        .and_then(|value| match value {
            NifValue::Array(items) => items.iter().find_map(|item| match item {
                NifValue::Ref(id)
                    if *id >= 0
                        && nif
                            .get_block(*id as usize)
                            .is_some_and(|block| block.type_name == "BSConnectPoint::Parents") =>
                {
                    Some(*id as usize)
                }
                _ => None,
            }),
            _ => None,
        })
        .or_else(|| {
            nif.blocks
                .iter()
                .position(|block| block.type_name == "BSConnectPoint::Parents")
        });
    let parent_block_id =
        existing_parent_block.unwrap_or_else(|| ensure_parent_connect_point_block(nif, root_id));
    let Some(parent_block) = nif.blocks.get_mut(parent_block_id) else {
        return false;
    };
    let mut connect_points = match parent_block.get_field("Connect Points").cloned() {
        Some(NifValue::Array(items)) => items,
        _ => Vec::new(),
    };
    let mut changed = existing_parent_block.is_none();
    if let Some(NifValue::Struct(fields)) = connect_points.iter_mut().find(|item| {
        matches!(
            item,
            NifValue::Struct(fields)
                if matches!(
                    fields.get("Name"),
                    Some(NifValue::String(name)) if name == point_name
                )
        )
    }) {
        if nif_vec3(fields.get("Translation")) != Some(translation) {
            fields.insert("Translation".to_string(), vector3_value(translation));
            changed = true;
        }
    } else {
        let mut point = IndexMap::new();
        point.insert("Parent".to_string(), NifValue::String(String::new()));
        point.insert("Name".to_string(), NifValue::String(point_name.to_string()));
        point.insert(
            "Rotation".to_string(),
            NifValue::Quaternion([1.0, 0.0, 0.0, 0.0]),
        );
        point.insert("Translation".to_string(), vector3_value(translation));
        point.insert("Scale".to_string(), NifValue::Float(1.0));
        connect_points.push(NifValue::Struct(point));
        changed = true;
    }
    parent_block.set_field(
        "Num Connect Points",
        NifValue::UInt(connect_points.len() as u64),
    );
    parent_block.set_field("Connect Points", NifValue::Array(connect_points));
    if changed {
        nif.rebuild_header();
    }
    changed
}

#[derive(Clone, Debug)]
pub struct ParentConnectPoint {
    pub parent: String,
    pub name: String,
    pub rotation: [f32; 4],
    pub translation: [f32; 3],
    pub scale: f32,
}

pub fn replace_parent_connect_points_with_prefix(
    nif: &mut NifFile,
    prefix: &str,
    points: &[ParentConnectPoint],
) -> bool {
    let root_id = nif
        .header
        .footer_roots
        .iter()
        .find_map(|id| (*id >= 0).then_some(*id as usize))
        .unwrap_or(0);
    let existing_parent_block = nif
        .get_block(root_id)
        .and_then(|root| root.get_field("Extra Data List"))
        .and_then(|value| match value {
            NifValue::Array(items) => items.iter().find_map(|item| match item {
                NifValue::Ref(id)
                    if *id >= 0
                        && nif
                            .get_block(*id as usize)
                            .is_some_and(|block| block.type_name == "BSConnectPoint::Parents") =>
                {
                    Some(*id as usize)
                }
                _ => None,
            }),
            _ => None,
        })
        .or_else(|| {
            nif.blocks
                .iter()
                .position(|block| block.type_name == "BSConnectPoint::Parents")
        });
    if points.is_empty() && existing_parent_block.is_none() {
        return false;
    }
    let mut structure_changed = false;
    for parent in points
        .iter()
        .map(|point| point.parent.as_str())
        .filter(|parent| !parent.is_empty())
        .collect::<HashSet<_>>()
    {
        structure_changed |= ensure_named_child_node(nif, root_id, parent);
    }
    let parent_block_id =
        existing_parent_block.unwrap_or_else(|| ensure_parent_connect_point_block(nif, root_id));
    let Some(parent_block) = nif.blocks.get_mut(parent_block_id) else {
        return false;
    };
    let current = match parent_block.get_field("Connect Points").cloned() {
        Some(NifValue::Array(items)) => items,
        _ => Vec::new(),
    };
    let managed_names = points
        .iter()
        .map(|point| point.name.as_str())
        .collect::<HashSet<_>>();
    let existing_generated = current
        .iter()
        .filter(|item| {
            connect_point_name(item)
                .is_some_and(|name| name.starts_with(prefix) || managed_names.contains(name))
        })
        .collect::<Vec<_>>();
    let unchanged = existing_generated.len() == points.len()
        && existing_generated
            .iter()
            .zip(points)
            .all(|(existing, desired)| connect_point_matches(existing, desired));
    if unchanged {
        if structure_changed {
            nif.rebuild_header();
        }
        return structure_changed;
    }

    let mut replaced = current
        .into_iter()
        .filter(|item| {
            !connect_point_name(item)
                .is_some_and(|name| name.starts_with(prefix) || managed_names.contains(name))
        })
        .collect::<Vec<_>>();
    replaced.extend(points.iter().map(parent_connect_point_value));
    parent_block.set_field("Num Connect Points", NifValue::UInt(replaced.len() as u64));
    parent_block.set_field("Connect Points", NifValue::Array(replaced));
    nif.rebuild_header();
    true
}

fn ensure_named_child_node(nif: &mut NifFile, root_id: usize, name: &str) -> bool {
    if nif.blocks.iter().any(|block| {
        block.type_name == "NiNode"
            && matches!(
                block.get_field("Name"),
                Some(NifValue::String(existing_name)) if existing_name == name
            )
    }) {
        return false;
    }

    let node_id = nif.add_block("NiNode", None);
    if let Some(node) = nif.blocks.get_mut(node_id) {
        node.set_field("Name", NifValue::String(name.to_string()));
        node.set_field("Num Children", NifValue::UInt(0));
        node.set_field("Children", NifValue::Array(Vec::new()));
    }
    let mut children = nif
        .get_block(root_id)
        .and_then(|root| root.get_field("Children"))
        .and_then(|value| match value {
            NifValue::Array(items) => Some(items.clone()),
            _ => None,
        })
        .unwrap_or_default();
    children.push(NifValue::Ref(node_id as i32));
    if let Some(root) = nif.blocks.get_mut(root_id) {
        root.set_field("Num Children", NifValue::UInt(children.len() as u64));
        root.set_field("Children", NifValue::Array(children));
    }
    true
}

fn parent_connect_point_value(point: &ParentConnectPoint) -> NifValue {
    NifValue::Struct(IndexMap::from([
        ("Parent".to_string(), NifValue::String(point.parent.clone())),
        ("Name".to_string(), NifValue::String(point.name.clone())),
        ("Rotation".to_string(), NifValue::Quaternion(point.rotation)),
        ("Translation".to_string(), vector3_value(point.translation)),
        ("Scale".to_string(), NifValue::Float(point.scale as f64)),
    ]))
}

fn connect_point_name(value: &NifValue) -> Option<&str> {
    match value {
        NifValue::Struct(fields) => match fields.get("Name") {
            Some(NifValue::String(name)) => Some(name),
            _ => None,
        },
        _ => None,
    }
}

fn connect_point_matches(value: &NifValue, point: &ParentConnectPoint) -> bool {
    let NifValue::Struct(fields) = value else {
        return false;
    };
    matches!(fields.get("Parent"), Some(NifValue::String(parent)) if parent == &point.parent)
        && matches!(fields.get("Name"), Some(NifValue::String(name)) if name == &point.name)
        && nif_vec3(fields.get("Translation")).is_some_and(|translation| {
            translation
                .iter()
                .zip(point.translation)
                .all(|(left, right)| (*left - right).abs() <= 0.000001)
        })
        && nif_quaternion(fields.get("Rotation")).is_some_and(|rotation| {
            rotation
                .iter()
                .zip(point.rotation)
                .all(|(left, right)| (*left - right).abs() <= 0.000001)
        })
        && nif_float(fields.get("Scale"))
            .is_some_and(|scale| (scale - f64::from(point.scale)).abs() <= 0.000001)
}

fn vector3_value(value: [f32; 3]) -> NifValue {
    NifValue::Struct(IndexMap::from([
        ("x".to_string(), NifValue::Float(value[0] as f64)),
        ("y".to_string(), NifValue::Float(value[1] as f64)),
        ("z".to_string(), NifValue::Float(value[2] as f64)),
    ]))
}

fn nif_vec3(value: Option<&NifValue>) -> Option<[f32; 3]> {
    match value? {
        NifValue::Vec3(value) => Some(*value),
        NifValue::Struct(fields) => Some([
            nif_float(fields.get("x"))? as f32,
            nif_float(fields.get("y"))? as f32,
            nif_float(fields.get("z"))? as f32,
        ]),
        _ => None,
    }
}

fn nif_quaternion(value: Option<&NifValue>) -> Option<[f32; 4]> {
    match value? {
        NifValue::Quaternion(value) | NifValue::Vec4(value) => Some(*value),
        NifValue::Struct(fields) => Some([
            nif_float(fields.get("w"))? as f32,
            nif_float(fields.get("x"))? as f32,
            nif_float(fields.get("y"))? as f32,
            nif_float(fields.get("z"))? as f32,
        ]),
        _ => None,
    }
}

fn nif_float(value: Option<&NifValue>) -> Option<f64> {
    match value? {
        NifValue::Float(value) => Some(*value),
        NifValue::Int(value) => Some(*value as f64),
        NifValue::UInt(value) => Some(*value as f64),
        _ => None,
    }
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
    fields.insert("Name".to_string(), NifValue::String("CPA".to_string()));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn connect_point_translation(nif: &NifFile, name: &str) -> Option<[f32; 3]> {
        nif.blocks
            .iter()
            .find(|block| block.type_name == "BSConnectPoint::Parents")
            .and_then(|block| block.get_field("Connect Points"))
            .and_then(|value| match value {
                NifValue::Array(points) => points.iter().find_map(|point| match point {
                    NifValue::Struct(fields)
                        if matches!(
                            fields.get("Name"),
                            Some(NifValue::String(point_name)) if point_name == name
                        ) =>
                    {
                        nif_vec3(fields.get("Translation"))
                    }
                    _ => None,
                }),
                _ => None,
            })
    }

    #[test]
    fn upsert_parent_connect_point_adds_workshop_anchor() {
        let mut nif = NifFile::new("fo4");

        assert!(upsert_parent_connect_point_translation(
            &mut nif,
            "P-WS-Snap",
            [1.5, 33.5, 113.375],
        ));

        assert_eq!(
            connect_point_translation(&nif, "P-WS-Snap"),
            Some([1.5, 33.5, 113.375])
        );
        assert!(matches!(
            nif.blocks[0].get_field("Extra Data List"),
            Some(NifValue::Array(values)) if values.len() == 1
        ));
    }

    #[test]
    fn upsert_parent_connect_point_updates_existing_anchor() {
        let mut nif = NifFile::new("fo4");
        upsert_parent_connect_point_translation(&mut nif, "P-WS-Snap", [0.0, 0.0, 0.0]);

        assert!(upsert_parent_connect_point_translation(
            &mut nif,
            "P-WS-Snap",
            [3.5, 15.0, 77.0],
        ));
        assert_eq!(
            connect_point_translation(&nif, "P-WS-Snap"),
            Some([3.5, 15.0, 77.0])
        );
        assert!(!upsert_parent_connect_point_translation(
            &mut nif,
            "P-WS-Snap",
            [3.5, 15.0, 77.0],
        ));
    }

    #[test]
    fn replace_workshop_snap_points_preserves_non_generated_points() {
        let mut nif = NifFile::new("fo4");
        upsert_parent_connect_point_translation(&mut nif, "P-WS-Snap", [1.0, 2.0, 3.0]);
        let points = vec![
            ParentConnectPoint {
                parent: "WorkshopConnectPoints".to_string(),
                name: "P-76-0A7382".to_string(),
                rotation: [1.0, 0.0, 0.0, 0.0],
                translation: [0.0, 128.0, -32.0],
                scale: 1.0,
            },
            ParentConnectPoint {
                parent: "WorkshopConnectPoints".to_string(),
                name: "P-76-0A7382".to_string(),
                rotation: [0.70710677, 0.0, 0.0, 0.70710677],
                translation: [-128.0, 0.0, 0.0],
                scale: 1.0,
            },
            ParentConnectPoint {
                parent: "WorkshopConnectPoints".to_string(),
                name: "P-Floor".to_string(),
                rotation: [1.0, 0.0, 0.0, 0.0],
                translation: [0.0, 128.0, 0.0],
                scale: 1.0,
            },
        ];

        assert!(replace_parent_connect_points_with_prefix(
            &mut nif, "P-76-", &points,
        ));
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("workshop.nif");
        nif.save(Some(path.clone())).unwrap();
        let mut nif = NifFile::load(&path).unwrap();
        assert!(!replace_parent_connect_points_with_prefix(
            &mut nif, "P-76-", &points,
        ));
        assert_eq!(
            connect_point_translation(&nif, "P-WS-Snap"),
            Some([1.0, 2.0, 3.0])
        );
        assert_eq!(
            nif.blocks
                .iter()
                .find(|block| block.type_name == "BSConnectPoint::Parents")
                .and_then(|block| block.get_field("Connect Points"))
                .and_then(|value| match value {
                    NifValue::Array(points) => Some(points.len()),
                    _ => None,
                }),
            Some(4),
        );
        assert!(nif.blocks.iter().any(|block| {
            block.type_name == "NiNode"
                && matches!(
                    block.get_field("Name"),
                    Some(NifValue::String(name)) if name == "WorkshopConnectPoints"
                )
        }));
    }
}
