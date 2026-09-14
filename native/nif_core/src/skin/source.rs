//! Parse legacy skin chains and fold partition-local weights into per-shape
//! global influences.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use crate::model::{NifBlock, NifFile, NifValue};
use crate::skin::bone_remap::{BoneEntry, VertexInfluences};

#[derive(Debug, thiserror::Error)]
pub enum SkinParseError {
    #[error("shape block {0} not found")]
    ShapeNotFound(usize),
    #[error("skin instance block {0} not found")]
    InstanceNotFound(usize),
    #[error("skin instance has no NiSkinData ref")]
    MissingSkinData,
    #[error("NiSkinData block {0} not found")]
    SkinDataNotFound(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacySkinKind {
    NonArmor,
    Armor,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinTransform {
    pub translation: [f32; 3],
    pub rotation: [[f32; 3]; 3],
    pub scale: f32,
}

impl SkinTransform {
    pub fn identity() -> Self {
        Self {
            translation: [0.0, 0.0, 0.0],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            scale: 1.0,
        }
    }

    pub fn is_identity(&self) -> bool {
        let identity = Self::identity();
        self.translation
            .iter()
            .zip(identity.translation.iter())
            .all(|(left, right)| (*left - *right).abs() <= 1e-6)
            && self
                .rotation
                .iter()
                .flatten()
                .zip(identity.rotation.iter().flatten())
                .all(|(left, right)| (*left - *right).abs() <= 1e-6)
            && (self.scale - 1.0).abs() <= 1e-6
    }

    pub fn transform_point(&self, point: [f32; 3]) -> [f32; 3] {
        let scaled = [
            point[0] * self.scale,
            point[1] * self.scale,
            point[2] * self.scale,
        ];
        [
            self.rotation[0][0] * scaled[0]
                + self.rotation[0][1] * scaled[1]
                + self.rotation[0][2] * scaled[2]
                + self.translation[0],
            self.rotation[1][0] * scaled[0]
                + self.rotation[1][1] * scaled[1]
                + self.rotation[1][2] * scaled[2]
                + self.translation[1],
            self.rotation[2][0] * scaled[0]
                + self.rotation[2][1] * scaled[1]
                + self.rotation[2][2] * scaled[2]
                + self.translation[2],
        ]
    }

    pub fn transform_vector(&self, vector: [f32; 3]) -> [f32; 3] {
        [
            self.rotation[0][0] * vector[0]
                + self.rotation[0][1] * vector[1]
                + self.rotation[0][2] * vector[2],
            self.rotation[1][0] * vector[0]
                + self.rotation[1][1] * vector[1]
                + self.rotation[1][2] * vector[2],
            self.rotation[2][0] * vector[0]
                + self.rotation[2][1] * vector[1]
                + self.rotation[2][2] * vector[2],
        ]
    }
}

impl Default for SkinTransform {
    fn default() -> Self {
        Self::identity()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyPartition {
    pub body_part: u16,
    pub vertex_map: Vec<u32>,
    pub influences: Vec<Vec<(u32, f32)>>,
    pub bones: Vec<u32>,
    pub triangles: Vec<[u32; 3]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacySkin {
    pub kind: LegacySkinKind,
    pub instance_block_id: usize,
    pub skin_data_block_id: usize,
    pub skin_partition_block_id: Option<usize>,
    pub skeleton_root: Option<usize>,
    pub bones: Vec<BoneEntry>,
    pub skin_transform: SkinTransform,
    pub data_influences: Vec<VertexInfluences>,
    pub bone_transforms: Vec<SkinTransform>,
    pub bone_bounds: Vec<Option<NifValue>>,
    pub partitions: Vec<LegacyPartition>,
}

pub fn parse_skin_chain(
    nif: &NifFile,
    shape_id: usize,
) -> Result<Option<LegacySkin>, SkinParseError> {
    let shape = nif
        .get_block(shape_id)
        .ok_or(SkinParseError::ShapeNotFound(shape_id))?;
    let Some(instance_id) = positive_ref(shape, &["Skin Instance", "Skin"]) else {
        return Ok(None);
    };

    let instance = nif
        .get_block(instance_id)
        .ok_or(SkinParseError::InstanceNotFound(instance_id))?;
    let kind = match instance.type_name.as_str() {
        "NiSkinInstance" => LegacySkinKind::NonArmor,
        "BSDismemberSkinInstance" => LegacySkinKind::Armor,
        _ => return Ok(None),
    };

    let skin_data_id = match instance.get_field("Data") {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
        _ => return Err(SkinParseError::MissingSkinData),
    };
    let skin_data = nif
        .get_block(skin_data_id)
        .ok_or(SkinParseError::SkinDataNotFound(skin_data_id))?;

    let partition_id = match instance.get_field("Skin Partition") {
        Some(NifValue::Ref(reference)) if *reference >= 0 => Some(*reference as usize),
        _ => None,
    };

    let partitions = partition_id
        .and_then(|id| nif.get_block(id))
        .map(parse_partitions)
        .unwrap_or_default();
    let (data_influences, bone_transforms, bone_bounds) = parse_skin_data_bind(skin_data);

    Ok(Some(LegacySkin {
        kind,
        instance_block_id: instance_id,
        skin_data_block_id: skin_data_id,
        skin_partition_block_id: partition_id,
        skeleton_root: positive_ref(instance, &["Skeleton Root"]),
        bones: collect_bone_entries(instance, nif),
        skin_transform: transform_or_identity(skin_data.get_field("Skin Transform")),
        data_influences,
        bone_transforms,
        bone_bounds,
        partitions,
    }))
}

pub fn fold_partitions_to_global(
    partitions: &[LegacyPartition],
    num_global_vertices: usize,
) -> Vec<VertexInfluences> {
    let mut out = vec![VertexInfluences::default(); num_global_vertices];

    for partition in partitions {
        for (local_vertex_index, partition_slots) in partition.influences.iter().enumerate() {
            let Some(global_vertex_index) = partition.vertex_map.get(local_vertex_index).copied()
            else {
                continue;
            };
            let Some(destination) = out.get_mut(global_vertex_index as usize) else {
                continue;
            };

            for (local_bone_slot, weight) in partition_slots {
                let Some(global_bone_index) =
                    partition.bones.get(*local_bone_slot as usize).copied()
                else {
                    continue;
                };

                if let Some(existing) = destination
                    .slots
                    .iter_mut()
                    .find(|(bone_index, _)| *bone_index == global_bone_index as usize)
                {
                    existing.1 += *weight;
                } else {
                    destination
                        .slots
                        .push((global_bone_index as usize, *weight));
                }
            }
        }
    }

    for influences in &mut out {
        clamp_top_four_and_normalize(&mut influences.slots);
    }

    out
}

fn collect_bone_entries(instance: &NifBlock, nif: &NifFile) -> Vec<BoneEntry> {
    let Some(NifValue::Array(items)) = instance.get_field("Bones") else {
        return Vec::new();
    };

    let bone_refs = items
        .iter()
        .filter_map(|value| match value {
            NifValue::Ref(reference) if *reference >= 0 => Some(*reference as usize),
            _ => None,
        })
        .collect::<Vec<_>>();
    let bone_lookup = bone_refs
        .iter()
        .copied()
        .enumerate()
        .map(|(local_index, block_id)| (block_id, local_index))
        .collect::<HashMap<_, _>>();
    let block_parents = block_parent_map(nif);

    bone_refs
        .iter()
        .copied()
        .map(|bone_id| BoneEntry {
            name: nif
                .get_block(bone_id)
                .and_then(|block| block.get_field("Name"))
                .and_then(|value| match value {
                    NifValue::String(name) => Some(name.trim_end_matches('\0').to_string()),
                    _ => None,
                })
                .unwrap_or_default(),
            parent: nearest_skin_parent(bone_id, &block_parents, &bone_lookup),
        })
        .collect()
}

fn block_parent_map(nif: &NifFile) -> HashMap<usize, usize> {
    let mut parents = HashMap::new();
    for block in &nif.blocks {
        let Some(NifValue::Array(children)) = block.get_field("Children") else {
            continue;
        };
        for child in children {
            let NifValue::Ref(child_id) = child else {
                continue;
            };
            if *child_id >= 0 {
                parents.entry(*child_id as usize).or_insert(block.block_id);
            }
        }
    }
    parents
}

fn nearest_skin_parent(
    bone_id: usize,
    block_parents: &HashMap<usize, usize>,
    bone_lookup: &HashMap<usize, usize>,
) -> i32 {
    let mut current = block_parents.get(&bone_id).copied();
    let mut seen = HashSet::new();
    while let Some(block_id) = current {
        if !seen.insert(block_id) {
            break;
        }
        if let Some(local_index) = bone_lookup.get(&block_id) {
            return *local_index as i32;
        }
        current = block_parents.get(&block_id).copied();
    }
    -1
}

fn positive_ref(block: &NifBlock, names: &[&str]) -> Option<usize> {
    names.iter().find_map(|name| match block.get_field(name) {
        Some(NifValue::Ref(reference)) if *reference >= 0 => Some(*reference as usize),
        _ => None,
    })
}

fn parse_partitions(block: &NifBlock) -> Vec<LegacyPartition> {
    let Some(NifValue::Array(items)) = block.get_field("Partitions") else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|value| match value {
            NifValue::Struct(fields) => Some(LegacyPartition {
                body_part: fields
                    .get("Body Part")
                    .or_else(|| fields.get("Body Part Type"))
                    .map(as_u16)
                    .unwrap_or(0),
                vertex_map: value_array(fields.get("Vertex Map")),
                influences: parse_partition_influences(fields),
                bones: value_array(fields.get("Bones")),
                triangles: parse_partition_triangles(fields),
            }),
            _ => None,
        })
        .collect()
}

fn parse_skin_data_bind(
    skin_data: &NifBlock,
) -> (
    Vec<VertexInfluences>,
    Vec<SkinTransform>,
    Vec<Option<NifValue>>,
) {
    let Some(NifValue::Array(bone_list)) = skin_data.get_field("Bone List") else {
        return (Vec::new(), Vec::new(), Vec::new());
    };

    let mut influences = Vec::<VertexInfluences>::new();
    let mut bone_transforms = Vec::with_capacity(bone_list.len());
    let mut bone_bounds = Vec::with_capacity(bone_list.len());
    for (bone_index, bone_value) in bone_list.iter().enumerate() {
        let NifValue::Struct(fields) = bone_value else {
            bone_transforms.push(SkinTransform::identity());
            bone_bounds.push(None);
            continue;
        };
        bone_transforms.push(transform_or_identity(fields.get("Skin Transform")));
        bone_bounds.push(fields.get("Bounding Sphere").cloned());

        let Some(NifValue::Array(vertex_weights)) = fields.get("Vertex Weights") else {
            continue;
        };
        for weight_value in vertex_weights {
            let NifValue::Struct(weight_fields) = weight_value else {
                continue;
            };
            let Some(vertex_index) = weight_fields.get("Index").map(NifValue::as_i64) else {
                continue;
            };
            if vertex_index < 0 {
                continue;
            }
            let Some(weight) = value_f64(weight_fields.get("Weight")) else {
                continue;
            };
            if weight <= 0.0 {
                continue;
            }
            let vertex_index = vertex_index as usize;
            if influences.len() <= vertex_index {
                influences.resize_with(vertex_index + 1, VertexInfluences::default);
            }
            influences[vertex_index]
                .slots
                .push((bone_index, weight as f32));
        }
    }

    for influence in &mut influences {
        clamp_top_four_and_normalize(&mut influence.slots);
    }

    (influences, bone_transforms, bone_bounds)
}

fn parse_partition_influences(fields: &IndexMap<String, NifValue>) -> Vec<Vec<(u32, f32)>> {
    let Some(NifValue::Array(bone_index_rows)) = fields.get("Bone Indices") else {
        return Vec::new();
    };
    let Some(NifValue::Array(weight_rows)) = fields.get("Vertex Weights") else {
        return Vec::new();
    };

    bone_index_rows
        .iter()
        .zip(weight_rows.iter())
        .map(|(index_row, weight_row)| {
            let NifValue::Array(index_values) = index_row else {
                return Vec::new();
            };
            let NifValue::Array(weight_values) = weight_row else {
                return Vec::new();
            };

            index_values
                .iter()
                .zip(weight_values.iter())
                .filter_map(|(index_value, weight_value)| {
                    let bone_index = match index_value {
                        NifValue::UInt(value) => *value as u32,
                        NifValue::Int(value) if *value >= 0 => *value as u32,
                        _ => return None,
                    };
                    let weight = match weight_value {
                        NifValue::Float(value) if *value >= 0.0 => *value as f32,
                        _ => return None,
                    };
                    Some((bone_index, weight))
                })
                .collect()
        })
        .collect()
}

fn parse_partition_triangles(fields: &IndexMap<String, NifValue>) -> Vec<[u32; 3]> {
    let Some(NifValue::Array(triangles)) = fields.get("Triangles") else {
        return Vec::new();
    };

    triangles.iter().filter_map(triangle_from_value).collect()
}

fn clamp_top_four_and_normalize(slots: &mut Vec<(usize, f32)>) {
    slots.retain(|(_, weight)| *weight > 0.0);
    if slots.is_empty() {
        return;
    }

    slots.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });

    if slots.len() > 4 {
        slots.truncate(4);
    }

    let total: f32 = slots.iter().map(|(_, weight)| *weight).sum();
    if total <= 0.0 {
        slots.clear();
        return;
    }

    for (_, weight) in slots.iter_mut() {
        *weight /= total;
    }

    slots.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.partial_cmp(&right.1).unwrap_or(Ordering::Equal))
    });
}

fn value_array(value: Option<&NifValue>) -> Vec<u32> {
    let Some(NifValue::Array(items)) = value else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|item| match item {
            NifValue::UInt(value) => Some(*value as u32),
            NifValue::Int(value) if *value >= 0 => Some(*value as u32),
            _ => None,
        })
        .collect()
}

fn as_u16(value: &NifValue) -> u16 {
    match value {
        NifValue::UInt(number) => *number as u16,
        NifValue::Int(number) if *number >= 0 => *number as u16,
        _ => 0,
    }
}

fn transform_or_identity(value: Option<&NifValue>) -> SkinTransform {
    match value {
        Some(NifValue::Struct(fields)) => SkinTransform {
            translation: fields
                .get("Translation")
                .and_then(vec3_value)
                .unwrap_or([0.0, 0.0, 0.0]),
            rotation: fields.get("Rotation").and_then(matrix33_value).unwrap_or([
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ]),
            scale: value_f64(fields.get("Scale")).unwrap_or(1.0) as f32,
        },
        Some(NifValue::Matrix44(matrix)) => SkinTransform {
            translation: [matrix[0][3], matrix[1][3], matrix[2][3]],
            rotation: [
                [matrix[0][0], matrix[0][1], matrix[0][2]],
                [matrix[1][0], matrix[1][1], matrix[1][2]],
                [matrix[2][0], matrix[2][1], matrix[2][2]],
            ],
            scale: 1.0,
        },
        _ => SkinTransform::identity(),
    }
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

fn vec3_value(value: &NifValue) -> Option<[f32; 3]> {
    match value {
        NifValue::Vec3(value) => Some(*value),
        NifValue::Struct(fields) => Some([
            value_f64(fields.get("x")).unwrap_or(0.0) as f32,
            value_f64(fields.get("y")).unwrap_or(0.0) as f32,
            value_f64(fields.get("z")).unwrap_or(0.0) as f32,
        ]),
        _ => None,
    }
}

pub(super) fn matrix33_value(value: &NifValue) -> Option<[[f32; 3]; 3]> {
    match value {
        NifValue::Matrix33(matrix) => Some(*matrix),
        NifValue::Struct(fields) => Some([
            [
                value_f64(fields.get("m11"))? as f32,
                value_f64(fields.get("m21"))? as f32,
                value_f64(fields.get("m31"))? as f32,
            ],
            [
                value_f64(fields.get("m12"))? as f32,
                value_f64(fields.get("m22"))? as f32,
                value_f64(fields.get("m32"))? as f32,
            ],
            [
                value_f64(fields.get("m13"))? as f32,
                value_f64(fields.get("m23"))? as f32,
                value_f64(fields.get("m33"))? as f32,
            ],
        ]),
        _ => None,
    }
}

fn value_f64(value: Option<&NifValue>) -> Option<f64> {
    match value? {
        NifValue::Float(value) => Some(*value),
        NifValue::Int(value) => Some(*value as f64),
        NifValue::UInt(value) => Some(*value as f64),
        _ => None,
    }
}
