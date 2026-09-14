//! Skeleton-map loading, body-part remap, and unmapped-bone weight handling.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum SkeletonMapError {
    #[error("read: {0}")]
    Read(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(String),
}

#[derive(Debug, Clone, Default)]
pub struct SkeletonMap {
    forward: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct YamlSkeletonFile {
    bones: HashMap<String, String>,
}

impl SkeletonMap {
    pub fn load(dir: &Path, source: &str, target: &str) -> Result<Self, SkeletonMapError> {
        for src_candidate in Self::candidates(source) {
            let path = dir.join(format!("skeleton_{src_candidate}_to_{target}.yaml"));
            if !path.exists() {
                continue;
            }

            let text = std::fs::read_to_string(&path)?;
            let parsed: YamlSkeletonFile = serde_saphyr::from_str(&text)
                .map_err(|err| SkeletonMapError::Parse(format!("{}: {err}", path.display())))?;
            return Ok(Self {
                forward: parsed.bones,
            });
        }

        Err(SkeletonMapError::Parse(format!(
            "no skeleton map for {source} -> {target} in {}",
            dir.display()
        )))
    }

    fn candidates(source: &str) -> Vec<&str> {
        match source {
            "fnv" => vec!["fnv", "fo3"],
            other => vec![other],
        }
    }

    pub fn lookup(&self, source_bone: &str) -> Option<&str> {
        if let Some(value) = self.forward.get(source_bone) {
            return Some(value.as_str());
        }

        let lower = source_bone.to_ascii_lowercase();
        self.forward
            .iter()
            .find(|(key, _)| key.to_ascii_lowercase() == lower)
            .map(|(_, value)| value.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.forward.is_empty()
    }

    pub fn len(&self) -> usize {
        self.forward.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoneEntry {
    pub name: String,
    pub parent: i32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VertexInfluences {
    pub slots: Vec<(usize, f32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyPartRemap {
    pub fo4_partition: u16,
    pub segment_user_index: u32,
}

pub fn fo3_body_part_to_fo4_segment(fo3_flag: u16) -> Option<BodyPartRemap> {
    let (fo4_partition, segment_user_index) = match fo3_flag {
        0 => (32, 32),
        1 => (30, 30),
        2 => (34, 34),
        3 => (33, 33),
        4 => (38, 38),
        5 => (37, 37),
        6 => (30, 30),
        _ => return None,
    };

    Some(BodyPartRemap {
        fo4_partition,
        segment_user_index,
    })
}

pub fn body_part_to_fo4_segment(source_game: &str, source_flag: u16) -> Option<BodyPartRemap> {
    if source_game != "skyrimse" {
        return fo3_body_part_to_fo4_segment(source_flag);
    }

    let segment_user_index = match source_flag {
        30 => 30,
        31 | 41 => 31,
        32 => 33,
        33 => 34,
        34 => 37,
        35 | 45 => 50,
        36 => 51,
        37 | 38 => 39,
        39 => 59,
        40 | 48 | 60 | 61 => 61,
        42 | 43 => 46,
        44 | 55 => 49,
        46 | 56 => 41,
        47 => 54,
        49 | 52 | 53 | 54 => 44,
        50 | 51 => 53,
        57 | 58 | 59 => 42,
        _ => return None,
    };
    Some(BodyPartRemap {
        fo4_partition: segment_user_index as u16,
        segment_user_index,
    })
}

#[derive(Debug, Clone, Default)]
pub struct RedistributeReport {
    pub dropped_unmapped: Vec<String>,
    pub weights_redistributed: usize,
}

pub fn redistribute_unmapped(
    influences: &mut [VertexInfluences],
    bones: &[BoneEntry],
    map: &SkeletonMap,
) -> RedistributeReport {
    let nearest_mapped: Vec<i32> = bones
        .iter()
        .enumerate()
        .map(|(index, _)| nearest_mapped_ancestor(index, bones, map))
        .collect();

    let mut dropped_seen = HashSet::new();
    let mut dropped_unmapped = Vec::new();
    let mut weights_redistributed = 0usize;

    for vertex in influences {
        let mut remapped_slots = Vec::new();
        for (bone_index, weight) in vertex.slots.drain(..) {
            let Some(bone) = bones.get(bone_index) else {
                continue;
            };

            if map.lookup(&bone.name).is_some() {
                merge_slot(&mut remapped_slots, bone_index, weight);
                continue;
            }

            if dropped_seen.insert(bone.name.clone()) {
                dropped_unmapped.push(bone.name.clone());
            }

            let ancestor = nearest_mapped.get(bone_index).copied().unwrap_or(-1);
            if ancestor >= 0 {
                merge_slot(&mut remapped_slots, ancestor as usize, weight);
                weights_redistributed += 1;
            }
        }
        vertex.slots = remapped_slots;
    }

    RedistributeReport {
        dropped_unmapped,
        weights_redistributed,
    }
}

fn nearest_mapped_ancestor(start: usize, bones: &[BoneEntry], map: &SkeletonMap) -> i32 {
    let mut current = bones[start].parent;
    let mut seen = HashSet::new();
    while current >= 0 && seen.insert(current) {
        let index = current as usize;
        let Some(bone) = bones.get(index) else {
            return -1;
        };
        if map.lookup(&bone.name).is_some() {
            return current;
        }
        current = bone.parent;
    }
    -1
}

fn merge_slot(slots: &mut Vec<(usize, f32)>, bone_index: usize, weight: f32) {
    if let Some((_, existing_weight)) = slots.iter_mut().find(|(index, _)| *index == bone_index) {
        *existing_weight += weight;
    } else {
        slots.push((bone_index, weight));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skyrim_body_partitions_map_to_fo4_segments() {
        assert_eq!(
            body_part_to_fo4_segment("skyrimse", 32),
            Some(BodyPartRemap {
                fo4_partition: 33,
                segment_user_index: 33,
            })
        );
        assert_eq!(
            body_part_to_fo4_segment("skyrimse", 34)
                .unwrap()
                .segment_user_index,
            37
        );
        assert_eq!(
            body_part_to_fo4_segment("skyrimse", 38)
                .unwrap()
                .segment_user_index,
            39
        );
    }
}
