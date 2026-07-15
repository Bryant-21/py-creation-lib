//! First-person sibling extraction for arm-weighted geometry.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::model::{NifFile, NifValue};
use crate::skin::bone_remap::VertexInfluences;

const ARM_THRESHOLD: f32 = 0.5;

#[derive(Debug, Clone)]
pub struct ArmExtract {
    pub kept_positions: Vec<[f32; 3]>,
    pub vertex_remap: Vec<u32>,
    pub kept_triangles: Vec<[u32; 3]>,
    pub arm_bones_used: Vec<usize>,
}

#[derive(Debug, thiserror::Error)]
pub enum FirstPersonError {
    #[error("read: {0}")]
    Read(#[from] crate::io::ReadError),
    #[error("write: {0}")]
    Write(#[from] crate::io::WriteError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub fn extract_arm_subset(
    positions: &[[f32; 3]],
    influences: &[VertexInfluences],
    triangles: &[[u32; 3]],
    bone_names: &[String],
) -> Option<ArmExtract> {
    let arm_bone_set: HashSet<usize> = bone_names
        .iter()
        .enumerate()
        .filter(|(_, name)| is_arm_bone_name(name))
        .map(|(index, _)| index)
        .collect();
    if arm_bone_set.is_empty() {
        return None;
    }

    let arm_weight_per_vertex: Vec<f32> = influences
        .iter()
        .map(|influence| {
            influence
                .slots
                .iter()
                .filter(|(bone_index, _)| arm_bone_set.contains(bone_index))
                .map(|(_, weight)| *weight)
                .sum()
        })
        .collect();

    let mut kept_original_triangles = Vec::new();
    for triangle in triangles {
        let average_arm_weight = triangle
            .iter()
            .map(|vertex_index| {
                arm_weight_per_vertex
                    .get(*vertex_index as usize)
                    .copied()
                    .unwrap_or(0.0)
            })
            .sum::<f32>()
            / 3.0;
        if average_arm_weight >= ARM_THRESHOLD {
            kept_original_triangles.push(*triangle);
        }
    }
    if kept_original_triangles.is_empty() {
        return None;
    }

    let mut old_to_new = HashMap::new();
    let mut vertex_remap = Vec::new();
    let mut kept_positions = Vec::new();
    let mut kept_triangles = Vec::with_capacity(kept_original_triangles.len());
    let mut arm_bones_used = HashSet::new();

    for triangle in kept_original_triangles {
        let mut remapped = [0u32; 3];
        for (slot, old_index) in triangle.iter().copied().enumerate() {
            let new_index = match old_to_new.get(&old_index).copied() {
                Some(index) => index,
                None => {
                    let index = vertex_remap.len() as u32;
                    old_to_new.insert(old_index, index);
                    vertex_remap.push(old_index);
                    kept_positions.push(
                        positions
                            .get(old_index as usize)
                            .copied()
                            .unwrap_or([0.0, 0.0, 0.0]),
                    );
                    index
                }
            };
            remapped[slot] = new_index;

            if let Some(influence) = influences.get(old_index as usize) {
                for (bone_index, weight) in &influence.slots {
                    if *weight > 0.0 && arm_bone_set.contains(bone_index) {
                        arm_bones_used.insert(*bone_index);
                    }
                }
            }
        }
        kept_triangles.push(remapped);
    }

    let mut arm_bones_used: Vec<usize> = arm_bones_used.into_iter().collect();
    arm_bones_used.sort_unstable();

    Some(ArmExtract {
        kept_positions,
        vertex_remap,
        kept_triangles,
        arm_bones_used,
    })
}

pub fn emit(
    converted: &NifFile,
    dst_path: &Path,
    reference_arms: Option<&Path>,
) -> Result<Option<String>, FirstPersonError> {
    let shape_ids: Vec<usize> = converted
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSSubIndexTriShape")
        .map(|block| block.block_id)
        .collect();
    if shape_ids.is_empty() {
        return Ok(None);
    }

    let mut out = converted.clone();
    let mut remove_shapes = Vec::new();
    let mut emitted_any = false;

    for shape_id in shape_ids {
        let Some(shape) = out.get_block(shape_id).cloned() else {
            continue;
        };
        let positions = positions_from_shape(&shape);
        let influences = influences_from_shape(&shape);
        let triangles = triangles_from_shape(&shape);
        let bone_names = bone_names_from_shape(&shape, &out);

        let Some(extracted) = extract_arm_subset(&positions, &influences, &triangles, &bone_names)
        else {
            remove_shapes.push(shape_id);
            continue;
        };

        let vertex_data = value_array(shape.get_field("Vertex Data"));
        let kept_vertex_data: Vec<NifValue> = extracted
            .vertex_remap
            .iter()
            .filter_map(|old_index| vertex_data.get(*old_index as usize).cloned())
            .collect();
        let kept_triangles: Vec<NifValue> = extracted
            .kept_triangles
            .iter()
            .map(|triangle| triangle_value(*triangle))
            .collect();

        if let Some(target_shape) = out.blocks.get_mut(shape_id) {
            target_shape.set_field(
                "Num Vertices",
                NifValue::UInt(kept_vertex_data.len() as u64),
            );
            target_shape.set_field("Num Triangles", NifValue::UInt(kept_triangles.len() as u64));
            target_shape.set_field(
                "Num Primitives",
                NifValue::UInt(kept_triangles.len() as u64),
            );
            target_shape.set_field("Vertex Data", NifValue::Array(kept_vertex_data));
            target_shape.set_field("Triangles", NifValue::Array(kept_triangles));
            set_single_segment(target_shape, extracted.kept_triangles.len());
            refresh_data_size(target_shape);
        }
        emitted_any = true;
    }

    if !emitted_any {
        return Ok(None);
    }
    if !remove_shapes.is_empty() {
        out.remove_blocks(&remove_shapes);
    }

    if let Some(reference) = reference_arms.filter(|path| path.exists()) {
        let reference_nif = NifFile::load(reference.to_path_buf())?;
        inject_reference_blocks(&mut out, &reference_nif);
    }

    let output_path = first_person_path(dst_path);
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    out.save(Some(output_path.clone()))?;
    Ok(Some(output_path.to_string_lossy().to_string()))
}

fn is_arm_bone_name(name: &str) -> bool {
    let key = name
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect::<String>();
    key.contains("arm")
        || key.contains("forearm")
        || key.contains("hand")
        || key.contains("wrist")
        || key.contains("clavicle")
}

fn first_person_path(dst_path: &Path) -> PathBuf {
    let stem = dst_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("converted");
    dst_path.with_file_name(format!("{stem}_1stPerson.nif"))
}

fn refresh_data_size(shape: &mut crate::model::NifBlock) {
    let stride = shape
        .get_field("Vertex Desc")
        .map(NifValue::as_i64)
        .unwrap_or(0)
        & 0xF;
    let vertices = shape
        .get_field("Num Vertices")
        .map(NifValue::as_i64)
        .unwrap_or(0)
        .max(0);
    let triangles = shape
        .get_field("Num Triangles")
        .map(NifValue::as_i64)
        .unwrap_or(0)
        .max(0);
    shape.set_field(
        "Data Size",
        NifValue::UInt((stride * vertices * 4 + triangles * 6) as u64),
    );
}

fn set_single_segment(shape: &mut crate::model::NifBlock, triangle_count: usize) {
    let mut segment = indexmap::IndexMap::new();
    segment.insert("Start Index".to_string(), NifValue::UInt(0));
    segment.insert(
        "Num Primitives".to_string(),
        NifValue::UInt(triangle_count as u64),
    );
    segment.insert(
        "Parent Array Index".to_string(),
        NifValue::UInt(u32::MAX as u64),
    );
    segment.insert("Num Sub Segments".to_string(), NifValue::UInt(0));
    segment.insert("Sub Segment".to_string(), NifValue::Array(Vec::new()));

    shape.set_field("Num Segments", NifValue::UInt(1));
    shape.set_field("Total Segments", NifValue::UInt(1));
    shape.set_field("Segment", NifValue::Array(vec![NifValue::Struct(segment)]));
    shape.fields.shift_remove("Segment Data");
}

fn inject_reference_blocks(out: &mut NifFile, reference: &NifFile) {
    if reference.blocks.len() <= 1 {
        return;
    }

    let offset = out.blocks.len();
    let mut appended_children = Vec::new();
    if let Some(root) = reference.get_block(0) {
        appended_children = ref_array(root.get_field("Children"))
            .into_iter()
            .filter(|id| *id > 0)
            .map(|id| NifValue::Ref((offset + id as usize - 1) as i32))
            .collect();
    }

    for source_block in reference.blocks.iter().skip(1) {
        let mut block = source_block.clone();
        block.block_id = out.blocks.len();
        remap_value_refs_in_fields(&mut block.fields, offset);
        out.blocks.push(block);
    }

    if !appended_children.is_empty() {
        if let Some(root) = out.blocks.get_mut(0) {
            let mut children = value_array(root.get_field("Children"));
            children.extend(appended_children);
            root.set_field("Num Children", NifValue::UInt(children.len() as u64));
            root.set_field("Children", NifValue::Array(children));
        }
    }
    out.rebuild_header();
}

fn remap_value_refs_in_fields(fields: &mut indexmap::IndexMap<String, NifValue>, offset: usize) {
    for value in fields.values_mut() {
        remap_value_refs(value, offset);
    }
}

fn remap_value_refs(value: &mut NifValue, offset: usize) {
    match value {
        NifValue::Ref(id) if *id > 0 => {
            *id = (offset + *id as usize - 1) as i32;
        }
        NifValue::Array(items) => {
            for item in items {
                remap_value_refs(item, offset);
            }
        }
        NifValue::Struct(fields) => remap_value_refs_in_fields(fields, offset),
        _ => {}
    }
}

fn positions_from_shape(shape: &crate::model::NifBlock) -> Vec<[f32; 3]> {
    value_array(shape.get_field("Vertex Data"))
        .iter()
        .filter_map(|value| match value {
            NifValue::Struct(fields) => fields.get("Vertex").and_then(vec3_value),
            _ => None,
        })
        .collect()
}

fn influences_from_shape(shape: &crate::model::NifBlock) -> Vec<VertexInfluences> {
    value_array(shape.get_field("Vertex Data"))
        .iter()
        .map(|value| {
            let NifValue::Struct(fields) = value else {
                return VertexInfluences::default();
            };
            let indices = numeric_array(fields.get("Bone Indices"));
            let weights = float_array(fields.get("Bone Weights"));
            let slots = indices
                .iter()
                .zip(weights.iter())
                .filter_map(|(bone_index, weight)| {
                    (*weight > 0.0).then_some((*bone_index as usize, *weight))
                })
                .collect();
            VertexInfluences { slots }
        })
        .collect()
}

fn triangles_from_shape(shape: &crate::model::NifBlock) -> Vec<[u32; 3]> {
    value_array(shape.get_field("Triangles"))
        .iter()
        .filter_map(triangle_from_value)
        .collect()
}

fn bone_names_from_shape(shape: &crate::model::NifBlock, nif: &NifFile) -> Vec<String> {
    let skin_ref = match shape
        .get_field("Skin")
        .or_else(|| shape.get_field("Skin Instance"))
    {
        Some(NifValue::Ref(id)) if *id >= 0 => *id as usize,
        _ => return Vec::new(),
    };
    let Some(skin) = nif.get_block(skin_ref) else {
        return Vec::new();
    };
    ref_array(skin.get_field("Bones"))
        .into_iter()
        .map(|bone_ref| {
            if bone_ref < 0 {
                return String::new();
            }
            nif.get_block(bone_ref as usize)
                .and_then(|block| block.get_field("Name"))
                .and_then(|value| match value {
                    NifValue::String(name) => Some(name.trim_end_matches('\0').to_string()),
                    _ => None,
                })
                .unwrap_or_default()
        })
        .collect()
}

fn triangle_value(triangle: [u32; 3]) -> NifValue {
    let mut fields = indexmap::IndexMap::new();
    fields.insert("v1".into(), NifValue::UInt(triangle[0] as u64));
    fields.insert("v2".into(), NifValue::UInt(triangle[1] as u64));
    fields.insert("v3".into(), NifValue::UInt(triangle[2] as u64));
    NifValue::Struct(fields)
}

fn triangle_from_value(value: &NifValue) -> Option<[u32; 3]> {
    let NifValue::Struct(fields) = value else {
        return None;
    };
    Some([
        fields.get("v1").map(NifValue::as_i64)? as u32,
        fields.get("v2").map(NifValue::as_i64)? as u32,
        fields.get("v3").map(NifValue::as_i64)? as u32,
    ])
}

fn value_array(value: Option<&NifValue>) -> Vec<NifValue> {
    match value {
        Some(NifValue::Array(items)) => items.clone(),
        _ => Vec::new(),
    }
}

fn ref_array(value: Option<&NifValue>) -> Vec<i32> {
    value_array(value)
        .iter()
        .filter_map(|value| match value {
            NifValue::Ref(id) => Some(*id),
            NifValue::Int(id) => Some(*id as i32),
            NifValue::UInt(id) => Some(*id as i32),
            _ => None,
        })
        .collect()
}

fn numeric_array(value: Option<&NifValue>) -> Vec<u32> {
    value_array(value)
        .iter()
        .map(|value| value.as_i64().max(0) as u32)
        .collect()
}

fn float_array(value: Option<&NifValue>) -> Vec<f32> {
    value_array(value)
        .iter()
        .filter_map(|value| match value {
            NifValue::Float(number) => Some(*number as f32),
            NifValue::Int(number) => Some(*number as f32),
            NifValue::UInt(number) => Some(*number as f32),
            _ => None,
        })
        .collect()
}

fn vec3_value(value: &NifValue) -> Option<[f32; 3]> {
    match value {
        NifValue::Vec3(vector) => Some(*vector),
        NifValue::Struct(fields) => Some([
            value_f64(fields.get("x")).unwrap_or(0.0) as f32,
            value_f64(fields.get("y")).unwrap_or(0.0) as f32,
            value_f64(fields.get("z")).unwrap_or(0.0) as f32,
        ]),
        _ => None,
    }
}

fn value_f64(value: Option<&NifValue>) -> Option<f64> {
    match value? {
        NifValue::Float(number) => Some(*number),
        NifValue::Int(number) => Some(*number as f64),
        NifValue::UInt(number) => Some(*number as f64),
        _ => None,
    }
}
