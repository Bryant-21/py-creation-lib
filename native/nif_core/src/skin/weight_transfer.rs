//! Opt-in reference-body morph-bone weight transfer.

use std::collections::{HashMap, HashSet};

use crate::skin::bone_remap::VertexInfluences;

#[derive(Debug, Clone, Copy)]
pub struct MorphTransferConfig {
    pub morph_weight_cap: f32,
    pub k_neighbors: usize,
}

impl Default for MorphTransferConfig {
    fn default() -> Self {
        Self {
            morph_weight_cap: 0.5,
            k_neighbors: 4,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MorphTransferStats {
    pub vertices_morph_weighted: usize,
    pub morph_bones_added: Vec<String>,
}

pub fn transfer_morph_weights(
    target_positions: &[[f32; 3]],
    target_influences: &mut Vec<VertexInfluences>,
    target_bones: &mut Vec<String>,
    ref_positions: &[[f32; 3]],
    ref_influences: &[VertexInfluences],
    ref_bones: &[String],
    cfg: &MorphTransferConfig,
) -> MorphTransferStats {
    if target_positions.is_empty() || ref_positions.is_empty() || ref_bones.is_empty() {
        return MorphTransferStats::default();
    }
    if target_influences.len() < target_positions.len() {
        target_influences.resize_with(target_positions.len(), VertexInfluences::default);
    }

    let target_bone_set: HashSet<&str> = target_bones.iter().map(String::as_str).collect();
    let morph_bone_indices: HashSet<usize> = ref_bones
        .iter()
        .enumerate()
        .filter(|(_, name)| !target_bone_set.contains(name.as_str()))
        .map(|(index, _)| index)
        .collect();
    if morph_bone_indices.is_empty() {
        return MorphTransferStats::default();
    }

    let mut stats = MorphTransferStats::default();
    let mut added_bone_lookup: HashMap<String, usize> = target_bones
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect();
    let cap = cfg.morph_weight_cap.clamp(0.0, 1.0);
    if cap <= 0.0 {
        return stats;
    }

    for (target_index, target_position) in target_positions.iter().enumerate() {
        let neighbors = nearest_neighbors(target_position, ref_positions, cfg.k_neighbors.max(1));
        let mut weighted_morph: HashMap<String, f32> = HashMap::new();
        let mut total_distance_weight = 0.0_f32;

        for (ref_index, distance_squared) in neighbors {
            let distance_weight = 1.0 / (distance_squared.sqrt() + 1e-6);
            total_distance_weight += distance_weight;
            let Some(reference_influence) = ref_influences.get(ref_index) else {
                continue;
            };
            for (bone_index, weight) in &reference_influence.slots {
                if !morph_bone_indices.contains(bone_index) || *weight <= 0.0 {
                    continue;
                }
                let Some(bone_name) = ref_bones.get(*bone_index) else {
                    continue;
                };
                *weighted_morph.entry(bone_name.clone()).or_insert(0.0) +=
                    *weight * distance_weight;
            }
        }

        if total_distance_weight <= 0.0 || weighted_morph.is_empty() {
            continue;
        }
        for weight in weighted_morph.values_mut() {
            *weight /= total_distance_weight;
        }

        let morph_total: f32 = weighted_morph.values().sum();
        if morph_total <= 0.0 {
            continue;
        }
        let morph_scale = if morph_total > cap {
            cap / morph_total
        } else {
            1.0
        };
        let final_morph_total: f32 = weighted_morph
            .values()
            .map(|weight| *weight * morph_scale)
            .sum();
        let target_existing_total = (1.0 - final_morph_total).max(0.0);

        let influence = &mut target_influences[target_index];
        let existing_total: f32 = influence.slots.iter().map(|(_, weight)| *weight).sum();
        if existing_total > 0.0 {
            let scale = target_existing_total / existing_total;
            for (_, weight) in &mut influence.slots {
                *weight *= scale;
            }
        }

        let mut applied_morph_indices = HashSet::new();
        for (bone_name, weight) in weighted_morph {
            let scaled_weight = weight * morph_scale;
            if scaled_weight <= 0.0 {
                continue;
            }
            let bone_index = match added_bone_lookup.get(&bone_name).copied() {
                Some(index) => index,
                None => {
                    let index = target_bones.len();
                    target_bones.push(bone_name.clone());
                    added_bone_lookup.insert(bone_name.clone(), index);
                    stats.morph_bones_added.push(bone_name);
                    index
                }
            };
            merge_weight(&mut influence.slots, bone_index, scaled_weight);
            applied_morph_indices.insert(bone_index);
        }
        normalize_top_four(&mut influence.slots);
        enforce_morph_cap(&mut influence.slots, &applied_morph_indices, cap);
        stats.vertices_morph_weighted += 1;
    }

    stats
}

fn nearest_neighbors(
    target: &[f32; 3],
    reference_positions: &[[f32; 3]],
    k_neighbors: usize,
) -> Vec<(usize, f32)> {
    let mut distances: Vec<(usize, f32)> = reference_positions
        .iter()
        .enumerate()
        .map(|(index, position)| (index, distance_squared(target, position)))
        .collect();
    distances.sort_by(|left, right| {
        left.1
            .total_cmp(&right.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    distances.truncate(k_neighbors.min(distances.len()));
    distances
}

fn distance_squared(left: &[f32; 3], right: &[f32; 3]) -> f32 {
    let dx = left[0] - right[0];
    let dy = left[1] - right[1];
    let dz = left[2] - right[2];
    dx * dx + dy * dy + dz * dz
}

fn merge_weight(slots: &mut Vec<(usize, f32)>, bone_index: usize, weight: f32) {
    if let Some((_, existing_weight)) = slots.iter_mut().find(|(index, _)| *index == bone_index) {
        *existing_weight += weight;
    } else {
        slots.push((bone_index, weight));
    }
}

fn normalize_top_four(slots: &mut Vec<(usize, f32)>) {
    slots.retain(|(_, weight)| *weight > 0.0);
    slots.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    slots.truncate(4);
    let total: f32 = slots.iter().map(|(_, weight)| *weight).sum();
    if total > 0.0 {
        for (_, weight) in slots.iter_mut() {
            *weight /= total;
        }
    }
}

fn enforce_morph_cap(slots: &mut [(usize, f32)], morph_bones: &HashSet<usize>, cap: f32) {
    if morph_bones.is_empty() {
        return;
    }

    let morph_total: f32 = slots
        .iter()
        .filter(|(bone_index, _)| morph_bones.contains(bone_index))
        .map(|(_, weight)| *weight)
        .sum();
    if morph_total <= cap + 1e-6 {
        return;
    }

    let non_morph_total: f32 = slots
        .iter()
        .filter(|(bone_index, _)| !morph_bones.contains(bone_index))
        .map(|(_, weight)| *weight)
        .sum();
    let morph_scale = cap / morph_total;
    let non_morph_scale = (non_morph_total > 0.0).then_some((1.0 - cap) / non_morph_total);

    for (bone_index, weight) in slots {
        if morph_bones.contains(bone_index) {
            *weight *= morph_scale;
        } else if let Some(scale) = non_morph_scale {
            *weight *= scale;
        }
    }
}
