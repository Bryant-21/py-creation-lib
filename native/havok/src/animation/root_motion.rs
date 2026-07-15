/// Animated reference frame / root motion.
///
/// Implements `AnimatedReferenceFrame`, which mirrors `hkaDefaultAnimatedReferenceFrame`
/// from the SDK. Stores per-frame root displacement and rotation extracted from
/// a baked animation clip (forward locomotion, turning, etc.).
///
/// Callers can attach an `AnimatedReferenceFrame` to an `AnimationClip`; the
/// writer then emits its `extractedMotion` data instead of `#null`.
use crate::animation::clip::{AnimationClip, AnimationKeyframe};
use crate::animation::pose::{quat_mul, quat_normalize, vec3_add, vec3_len, vec3_sub};

/// The type of reference frame (matches SDK `hkaAnimatedReferenceFrame::ReferenceFrameType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceFrameType {
    /// Default: animated displacement + rotation (locomotion).
    Default,
    /// Parametric: driven by motion parameters (rare).
    Parametric,
}

/// Per-frame root displacement and orientation for locomotion.
///
/// Mirrors `hkaDefaultAnimatedReferenceFrame`: a dense per-frame sample of
/// where the root should be in model space over the course of the animation.
#[derive(Debug, Clone)]
pub struct AnimatedReferenceFrame {
    pub frame_type: ReferenceFrameType,
    /// Animation duration in seconds.
    pub duration: f32,
    /// Sample rate of the reference frame data (Hz).
    pub frame_rate: f32,
    /// Per-frame displacement vectors (world-space translation of the root).
    pub displacements: Vec<[f32; 3]>,
    /// Per-frame rotations as quaternion (x,y,z,w).
    pub rotations: Vec<[f32; 4]>,
}

impl AnimatedReferenceFrame {
    /// Number of sample frames.
    pub fn num_frames(&self) -> usize {
        self.displacements.len().max(self.rotations.len())
    }

    /// Sample displacement at time `t` (seconds) by linear interpolation.
    pub fn displacement_at(&self, t: f32) -> [f32; 3] {
        if self.displacements.is_empty() {
            return [0.0, 0.0, 0.0];
        }
        let frame_f = (t * self.frame_rate).clamp(0.0, (self.displacements.len() - 1) as f32);
        let lo = frame_f as usize;
        let hi = (lo + 1).min(self.displacements.len() - 1);
        let frac = frame_f - lo as f32;
        let a = &self.displacements[lo];
        let b = &self.displacements[hi];
        [
            a[0] + frac * (b[0] - a[0]),
            a[1] + frac * (b[1] - a[1]),
            a[2] + frac * (b[2] - a[2]),
        ]
    }

    /// Sample rotation at time `t` (seconds) by slerp.
    pub fn rotation_at(&self, t: f32) -> [f32; 4] {
        if self.rotations.is_empty() {
            return [0.0, 0.0, 0.0, 1.0];
        }
        let frame_f = (t * self.frame_rate).clamp(0.0, (self.rotations.len() - 1) as f32);
        let lo = frame_f as usize;
        let hi = (lo + 1).min(self.rotations.len() - 1);
        let frac = frame_f - lo as f32;
        quat_slerp(&self.rotations[lo], &self.rotations[hi], frac)
    }

    /// Extract total displacement (start-to-end translation).
    pub fn total_displacement(&self) -> [f32; 3] {
        if self.displacements.len() < 2 {
            return [0.0, 0.0, 0.0];
        }
        vec3_sub(self.displacements.last().unwrap(), &self.displacements[0])
    }

    /// Extract root motion from the root channel of an `AnimationClip`.
    ///
    /// If the clip's first channel represents the root bone, samples its
    /// translation and rotation as the reference frame data. This is the
    /// most common extraction pattern for FO4/FO76 locomotion animations.
    pub fn extract_from_root_channel(clip: &AnimationClip) -> Option<AnimatedReferenceFrame> {
        let root_ch = clip.channels.first()?;
        if root_ch.translations.is_empty() || root_ch.rotations.is_empty() {
            return None;
        }
        let displacements = root_ch.translations.iter().map(|kf| kf.value).collect();
        let rotations = root_ch.rotations.iter().map(|kf| kf.value).collect();
        let frame_rate = if clip.native_fps > 0.0 {
            clip.native_fps
        } else {
            30.0
        };
        Some(AnimatedReferenceFrame {
            frame_type: ReferenceFrameType::Default,
            duration: clip.duration,
            frame_rate,
            displacements,
            rotations,
        })
    }
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
    let sin_th0 = th0.sin();
    let s0 = (th0 - th).sin() / sin_th0;
    let s1 = th.sin() / sin_th0;
    quat_normalize(&[
        s0 * a[0] + s1 * b_adj[0],
        s0 * a[1] + s1 * b_adj[1],
        s0 * a[2] + s1 * b_adj[2],
        s0 * a[3] + s1 * b_adj[3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::clip::{AnimationClip, AnimationKeyframe, BoneChannel};

    fn make_linear_frame() -> AnimatedReferenceFrame {
        AnimatedReferenceFrame {
            frame_type: ReferenceFrameType::Default,
            duration: 1.0,
            frame_rate: 30.0,
            displacements: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            rotations: vec![
                [0.0, 0.0, 0.0, 1.0],
                [0.0, 0.0, 0.0, 1.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    #[test]
    fn displacement_at_start() {
        let rf = make_linear_frame();
        let d = rf.displacement_at(0.0);
        assert!((d[0]).abs() < 1e-5);
    }

    #[test]
    fn displacement_at_end() {
        let rf = make_linear_frame();
        let d = rf.displacement_at(rf.duration);
        assert!((d[0] - 2.0).abs() < 0.1, "end displacement: {d:?}");
    }

    #[test]
    fn total_displacement_x() {
        let rf = make_linear_frame();
        let total = rf.total_displacement();
        assert!((total[0] - 2.0).abs() < 1e-5);
    }

    #[test]
    fn extract_from_root_channel_smoke() {
        let clip = AnimationClip {
            source_format: "hkx".into(),
            duration: 1.0,
            native_fps: 30.0,
            channels: vec![BoneChannel {
                bone_name: "Root".into(),
                translations: vec![
                    AnimationKeyframe {
                        time: 0.0,
                        value: [0.0, 0.0, 0.0],
                    },
                    AnimationKeyframe {
                        time: 1.0,
                        value: [5.0, 0.0, 0.0],
                    },
                ],
                rotations: vec![
                    AnimationKeyframe {
                        time: 0.0,
                        value: [0.0, 0.0, 0.0, 1.0],
                    },
                    AnimationKeyframe {
                        time: 1.0,
                        value: [0.0, 0.0, 0.0, 1.0],
                    },
                ],
                scales: Vec::new(),
            }],
            events: Vec::new(),
            original_skeleton_name: None,
            warnings: Vec::new(),
            is_additive: false,
            track_to_bone_indices: Vec::new(),
            extracted_motion_ref: String::new(),
        };
        let rf = AnimatedReferenceFrame::extract_from_root_channel(&clip).unwrap();
        assert_eq!(rf.displacements.len(), 2);
        assert!((rf.total_displacement()[0] - 5.0).abs() < 1e-5);
    }
}
