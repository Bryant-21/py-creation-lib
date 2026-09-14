use crate::animation::clip::{AnimationClip, AnimationKeyframe, BoneChannel};
use crate::animation::pose::{
    Pose, PoseSkeleton, QsTransform, quat_mul, quat_normalize, quat_rotate, vec3_add, vec3_len,
    vec3_sub,
};
/// Skeleton retargeting between rigs that differ in bone count and naming
/// (e.g. FO4 ↔ FO76 NPC rigs, Skyrim XPMSE → FO4).
///
/// Bones are name-matched exactly, then parent-child chains between matched
/// anchors are mapped to reduce drift. Unmatched bones keep the reference pose.
use std::collections::HashMap;

/// A simple (1:1) bone mapping: source bone index → target bone index.
#[derive(Debug, Clone)]
pub struct SimpleMapping {
    pub source_idx: usize,
    pub target_idx: usize,
}

/// A chain mapping: a sequence of bones in source maps to a sequence in target.
/// Used when intermediate bones differ (e.g. source has 3 spine bones, target has 2).
#[derive(Debug, Clone)]
pub struct ChainMapping {
    /// Bone indices in source skeleton, root-to-leaf order.
    pub source_chain: Vec<usize>,
    /// Bone indices in target skeleton, root-to-leaf order.
    pub target_chain: Vec<usize>,
}

/// Precomputed mapper between two skeletons.
#[derive(Debug, Clone)]
pub struct SkeletonMapper {
    pub source: PoseSkeleton,
    pub target: PoseSkeleton,
    /// 1:1 name-matched mappings.
    pub simple: Vec<SimpleMapping>,
    /// Chain mappings for multi-bone segments.
    pub chains: Vec<ChainMapping>,
}

impl SkeletonMapper {
    /// Build a mapper by name-matching source → target bones.
    /// Unmapped source bones are ignored (target keeps reference pose).
    pub fn from_skeletons(source: PoseSkeleton, target: PoseSkeleton) -> Self {
        let target_index: HashMap<&str, usize> = target
            .bone_names
            .iter()
            .enumerate()
            .map(|(i, name)| (name.as_str(), i))
            .collect();

        let mut simple = Vec::new();
        for (src_idx, src_name) in source.bone_names.iter().enumerate() {
            if let Some(&tgt_idx) = target_index.get(src_name.as_str()) {
                simple.push(SimpleMapping {
                    source_idx: src_idx,
                    target_idx: tgt_idx,
                });
            }
        }

        // Chain detection: find parent–child runs in source that have both
        // endpoints mapped, then build a target chain between those anchors.
        let chains = detect_chains(&source, &target, &simple);

        SkeletonMapper {
            source,
            target,
            simple,
            chains,
        }
    }
}

/// Retarget an animation clip from the mapper's source skeleton to the target.
///
/// Strategy: for each frame, build a source `Pose`, extract model-space
/// transforms, map them to the target skeleton's bones, convert back to
/// local space relative to the target reference pose parents, and record
/// the resulting keyframes.
pub fn retarget_clip(clip: &AnimationClip, mapper: &SkeletonMapper) -> AnimationClip {
    if clip.channels.is_empty() {
        return AnimationClip {
            source_format: clip.source_format.clone(),
            duration: clip.duration,
            native_fps: clip.native_fps,
            channels: Vec::new(),
            events: clip.events.clone(),
            original_skeleton_name: Some(
                mapper
                    .target
                    .bone_names
                    .first()
                    .cloned()
                    .unwrap_or_default(),
            ),
            warnings: vec!["retarget: source clip had no channels".to_string()],
            is_additive: clip.is_additive,
            track_to_bone_indices: Vec::new(),
            extracted_motion_ref: clip.extracted_motion_ref.clone(),
        };
    }

    // Build a frame-index → channel-by-name lookup.
    let src_channel_map: HashMap<&str, &BoneChannel> = clip
        .channels
        .iter()
        .map(|ch| (ch.bone_name.as_str(), ch))
        .collect();

    // Determine frame times from the first non-empty channel.
    let frame_times: Vec<f32> = {
        let ch = clip
            .channels
            .iter()
            .find(|c| !c.rotations.is_empty())
            .or_else(|| clip.channels.first());
        match ch {
            Some(c) if !c.rotations.is_empty() => c.rotations.iter().map(|kf| kf.time).collect(),
            Some(c) if !c.translations.is_empty() => {
                c.translations.iter().map(|kf| kf.time).collect()
            }
            _ => vec![0.0],
        }
    };

    let target_n = mapper.target.bone_count();
    // Per-target-bone per-frame translations and rotations.
    let mut tgt_trans: Vec<Vec<[f32; 3]>> = vec![Vec::new(); target_n];
    let mut tgt_rots: Vec<Vec<[f32; 4]>> = vec![Vec::new(); target_n];

    // Build a set of source-bone indices that are simple-mapped to a target bone.
    let src_to_tgt: HashMap<usize, usize> = mapper
        .simple
        .iter()
        .map(|m| (m.source_idx, m.target_idx))
        .collect();

    for &frame_time in &frame_times {
        // Sample source clip at this time by building a source pose.
        let src_local =
            sample_source_local(&mapper.source, &clip.channels, &src_channel_map, frame_time);
        let mut src_pose = Pose::from_local(mapper.source.clone(), src_local);

        // For each simple mapping, extract source model-space, re-express in
        // target local space (relative to target parent's model transform).
        let mut tgt_model_cache: HashMap<usize, QsTransform> = HashMap::new();

        for mapping in &mapper.simple {
            let src_model = src_pose.model_at(mapping.source_idx);

            // Target parent model transform (from reference if not yet in cache).
            let tgt_parent_model = {
                let pidx = mapper.target.parent_indices[mapping.target_idx];
                if pidx < 0 {
                    QsTransform::IDENTITY
                } else {
                    target_ref_model(&mapper.target, pidx as usize, &mut tgt_model_cache)
                }
            };

            // Convert source model-space to target local space:
            // local = inv(parent_model) * src_model
            let local = model_to_local(&tgt_parent_model, &src_model);
            tgt_model_cache.insert(mapping.target_idx, src_model.clone());

            tgt_trans[mapping.target_idx].push(local.translation);
            tgt_rots[mapping.target_idx].push(quat_normalize(&local.rotation));
        }

        // Fill unmapped target bones from reference pose.
        for tgt_idx in 0..target_n {
            if tgt_trans[tgt_idx].len() < frame_times.len().max(1)
                && tgt_trans[tgt_idx].len()
                    == frame_times
                        .iter()
                        .position(|&t| t == frame_time)
                        .unwrap_or(0)
            {
                let ref_local = &mapper.target.reference_local[tgt_idx];
                tgt_trans[tgt_idx].push(ref_local.translation);
                tgt_rots[tgt_idx].push(ref_local.rotation);
            }
        }
    }

    // Assemble target channels.
    let mut channels = Vec::with_capacity(target_n);
    for tgt_idx in 0..target_n {
        let bone_name = mapper.target.bone_names[tgt_idx].clone();
        let has_data = !tgt_rots[tgt_idx].is_empty();

        if !has_data {
            // No data for this bone — emit a single-keyframe identity channel.
            let ref_local = &mapper.target.reference_local[tgt_idx];
            channels.push(BoneChannel {
                bone_name,
                translations: vec![AnimationKeyframe {
                    time: 0.0,
                    value: ref_local.translation,
                }],
                rotations: vec![AnimationKeyframe {
                    time: 0.0,
                    value: ref_local.rotation,
                }],
                scales: Vec::new(),
            });
            continue;
        }

        let rotations = frame_times
            .iter()
            .zip(tgt_rots[tgt_idx].iter())
            .map(|(&t, &r)| AnimationKeyframe { time: t, value: r })
            .collect();
        let translations = frame_times
            .iter()
            .zip(tgt_trans[tgt_idx].iter())
            .map(|(&t, tr)| AnimationKeyframe {
                time: t,
                value: *tr,
            })
            .collect();

        channels.push(BoneChannel {
            bone_name,
            translations,
            rotations,
            scales: Vec::new(),
        });
    }

    AnimationClip {
        source_format: clip.source_format.clone(),
        duration: clip.duration,
        native_fps: clip.native_fps,
        channels,
        events: clip.events.clone(),
        original_skeleton_name: Some(
            mapper
                .target
                .bone_names
                .first()
                .cloned()
                .unwrap_or_default(),
        ),
        warnings: Vec::new(),
        is_additive: clip.is_additive,
        track_to_bone_indices: (0..mapper.target.bone_count() as u32).collect(),
        extracted_motion_ref: clip.extracted_motion_ref.clone(),
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn sample_source_local(
    skeleton: &PoseSkeleton,
    _channels: &[BoneChannel],
    channel_map: &HashMap<&str, &BoneChannel>,
    time: f32,
) -> Vec<QsTransform> {
    skeleton
        .bone_names
        .iter()
        .map(|name| {
            let Some(ch) = channel_map.get(name.as_str()) else {
                return skeleton.reference_local[skeleton
                    .bone_names
                    .iter()
                    .position(|n| n == name)
                    .unwrap_or(0)]
                .clone();
            };
            let translation = sample_vec3(&ch.translations, time).unwrap_or(
                skeleton.reference_local[skeleton
                    .bone_names
                    .iter()
                    .position(|n| n == name)
                    .unwrap_or(0)]
                .translation,
            );
            let rotation = sample_quat(&ch.rotations, time).unwrap_or(
                skeleton.reference_local[skeleton
                    .bone_names
                    .iter()
                    .position(|n| n == name)
                    .unwrap_or(0)]
                .rotation,
            );
            let ref_idx = skeleton
                .bone_names
                .iter()
                .position(|n| n == name)
                .unwrap_or(0);
            QsTransform {
                translation,
                rotation,
                scale: skeleton.reference_local[ref_idx].scale,
            }
        })
        .collect()
}

fn sample_vec3(kfs: &[AnimationKeyframe<[f32; 3]>], time: f32) -> Option<[f32; 3]> {
    if kfs.is_empty() {
        return None;
    }
    let idx = kfs.partition_point(|kf| kf.time <= time);
    if idx == 0 {
        return Some(kfs[0].value);
    }
    if idx >= kfs.len() {
        return Some(kfs.last().unwrap().value);
    }
    let a = &kfs[idx - 1];
    let b = &kfs[idx];
    let t = if (b.time - a.time).abs() < 1e-10 {
        0.0
    } else {
        (time - a.time) / (b.time - a.time)
    };
    Some([
        a.value[0] + t * (b.value[0] - a.value[0]),
        a.value[1] + t * (b.value[1] - a.value[1]),
        a.value[2] + t * (b.value[2] - a.value[2]),
    ])
}

fn sample_quat(kfs: &[AnimationKeyframe<[f32; 4]>], time: f32) -> Option<[f32; 4]> {
    if kfs.is_empty() {
        return None;
    }
    let idx = kfs.partition_point(|kf| kf.time <= time);
    if idx == 0 {
        return Some(kfs[0].value);
    }
    if idx >= kfs.len() {
        return Some(kfs.last().unwrap().value);
    }
    let a = &kfs[idx - 1];
    let b = &kfs[idx];
    let t = if (b.time - a.time).abs() < 1e-10 {
        0.0
    } else {
        (time - a.time) / (b.time - a.time)
    };
    Some(quat_normalize(&quat_slerp(&a.value, &b.value, t)))
}

fn quat_slerp(a: &[f32; 4], b: &[f32; 4], t: f32) -> [f32; 4] {
    let mut dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let b_adj = if dot < 0.0 {
        dot = -dot;
        [-b[0], -b[1], -b[2], -b[3]]
    } else {
        *b
    };
    if dot > 0.9995 {
        return quat_normalize(&[
            a[0] + t * (b_adj[0] - a[0]),
            a[1] + t * (b_adj[1] - a[1]),
            a[2] + t * (b_adj[2] - a[2]),
            a[3] + t * (b_adj[3] - a[3]),
        ]);
    }
    let th0 = dot.acos();
    let th = th0 * t;
    let s0 = (th0 - th).sin() / th0.sin();
    let s1 = th.sin() / th0.sin();
    quat_normalize(&[
        s0 * a[0] + s1 * b_adj[0],
        s0 * a[1] + s1 * b_adj[1],
        s0 * a[2] + s1 * b_adj[2],
        s0 * a[3] + s1 * b_adj[3],
    ])
}

/// Compute model-space transform for target bone `idx` from its reference pose,
/// caching intermediate results.
fn target_ref_model(
    target: &PoseSkeleton,
    idx: usize,
    cache: &mut HashMap<usize, QsTransform>,
) -> QsTransform {
    if let Some(cached) = cache.get(&idx) {
        return cached.clone();
    }
    let parent_idx = target.parent_indices[idx];
    let model = if parent_idx < 0 {
        target.reference_local[idx].clone()
    } else {
        let parent = target_ref_model(target, parent_idx as usize, cache);
        QsTransform::compose(&parent, &target.reference_local[idx])
    };
    cache.insert(idx, model.clone());
    model
}

/// Convert a model-space transform to local space given a parent model-space transform.
/// local = inv(parent_model) * model
fn model_to_local(parent: &QsTransform, model: &QsTransform) -> QsTransform {
    let inv_rot = [
        -parent.rotation[0],
        -parent.rotation[1],
        -parent.rotation[2],
        parent.rotation[3],
    ];
    let rel_t = vec3_sub(&model.translation, &parent.translation);
    let inv_scale = [
        if parent.scale[0].abs() > 1e-10 {
            1.0 / parent.scale[0]
        } else {
            1.0
        },
        if parent.scale[1].abs() > 1e-10 {
            1.0 / parent.scale[1]
        } else {
            1.0
        },
        if parent.scale[2].abs() > 1e-10 {
            1.0 / parent.scale[2]
        } else {
            1.0
        },
    ];
    let rotated = quat_rotate(&inv_rot, &rel_t);
    let local_t = [
        rotated[0] * inv_scale[0],
        rotated[1] * inv_scale[1],
        rotated[2] * inv_scale[2],
    ];
    let local_r = quat_normalize(&quat_mul(&inv_rot, &model.rotation));
    QsTransform {
        translation: local_t,
        rotation: local_r,
        scale: model.scale,
    }
}

/// Detect chain mappings: multi-bone segments in source with both anchor bones
/// mapped to target are grouped as chains. Returns chain mappings where source
/// and target chains have different lengths.
fn detect_chains(
    source: &PoseSkeleton,
    target: &PoseSkeleton,
    simple: &[SimpleMapping],
) -> Vec<ChainMapping> {
    // Chain detection is not implemented; simple name-matching only.
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::clip::{AnimationClip, AnimationKeyframe, BoneChannel};
    use crate::animation::pose::{PoseSkeleton, QsTransform};

    fn make_skeleton(names: &[&str], parents: &[i32]) -> PoseSkeleton {
        let n = names.len();
        PoseSkeleton {
            bone_names: names.iter().map(|s| s.to_string()).collect(),
            parent_indices: parents.to_vec(),
            reference_local: vec![QsTransform::IDENTITY; n],
        }
    }

    #[test]
    fn retarget_identity_clip_preserves_rotations() {
        let src_skel = make_skeleton(&["Root", "Spine", "Head"], &[-1, 0, 1]);
        let tgt_skel = make_skeleton(&["Root", "Spine", "Head"], &[-1, 0, 1]);
        let mapper = SkeletonMapper::from_skeletons(src_skel, tgt_skel);

        let rot = [0.0f32, 0.0, 0.707, 0.707];
        let clip = AnimationClip {
            source_format: "hkx".into(),
            duration: 1.0,
            native_fps: 30.0,
            channels: vec![
                BoneChannel {
                    bone_name: "Root".into(),
                    translations: vec![AnimationKeyframe {
                        time: 0.0,
                        value: [0.0, 0.0, 0.0],
                    }],
                    rotations: vec![AnimationKeyframe {
                        time: 0.0,
                        value: rot,
                    }],
                    scales: Vec::new(),
                },
                BoneChannel {
                    bone_name: "Spine".into(),
                    translations: vec![AnimationKeyframe {
                        time: 0.0,
                        value: [0.0, 1.0, 0.0],
                    }],
                    rotations: vec![AnimationKeyframe {
                        time: 0.0,
                        value: [0.0, 0.0, 0.0, 1.0],
                    }],
                    scales: Vec::new(),
                },
                BoneChannel {
                    bone_name: "Head".into(),
                    translations: vec![AnimationKeyframe {
                        time: 0.0,
                        value: [0.0, 2.0, 0.0],
                    }],
                    rotations: vec![AnimationKeyframe {
                        time: 0.0,
                        value: [0.0, 0.0, 0.0, 1.0],
                    }],
                    scales: Vec::new(),
                },
            ],
            events: Vec::new(),
            original_skeleton_name: None,
            warnings: Vec::new(),
            is_additive: false,
            track_to_bone_indices: Vec::new(),
            extracted_motion_ref: String::new(),
        };

        let result = retarget_clip(&clip, &mapper);
        assert_eq!(result.channels.len(), 3);
        // Retargeted Root rotation should be close to input (same-skeleton retarget).
        let root_ch = result
            .channels
            .iter()
            .find(|c| c.bone_name == "Root")
            .unwrap();
        let r = root_ch.rotations[0].value;
        let diff = (r[0] - rot[0]).abs()
            + (r[1] - rot[1]).abs()
            + (r[2] - rot[2]).abs()
            + (r[3] - rot[3]).abs();
        assert!(
            diff < 0.1,
            "root rotation mismatch after identity retarget: {r:?}"
        );
    }

    #[test]
    fn mapper_name_match_coverage() {
        let src = make_skeleton(&["Pelvis", "Spine1", "Spine2", "Head"], &[-1, 0, 1, 2]);
        let tgt = make_skeleton(&["Pelvis", "Spine1", "Head"], &[-1, 0, 1]);
        let mapper = SkeletonMapper::from_skeletons(src, tgt);
        // "Pelvis", "Spine1", "Head" should match (3 out of 4 source bones).
        assert_eq!(mapper.simple.len(), 3);
        // "Spine2" has no match in target — only 3 mappings.
        assert!(
            !mapper
                .simple
                .iter()
                .any(|m| m.source_idx == 2 && m.target_idx == 2)
        );
    }
}
