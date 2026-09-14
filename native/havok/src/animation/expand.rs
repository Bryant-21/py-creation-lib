/// Animation expand wrapper.
///
/// Wraps existing spline/interleaved decoders with a uniform `expand` API
/// that samples a compressed clip to a dense per-frame sequence at a given
/// rate. CLI: `modkit anim sample --rate 60 <clip.hkx>`.
use crate::animation::clip::{AnimationClip, AnimationKeyframe, BoneChannel};
use crate::error::HavokResult;

/// A dense (per-frame) animation clip, sampled at a uniform rate.
#[derive(Debug, Clone)]
pub struct DenseClip {
    /// Sample rate in Hz.
    pub rate_hz: f32,
    /// Duration in seconds.
    pub duration: f32,
    /// Total number of frames (== `(duration * rate_hz).ceil() + 1`).
    pub frame_count: usize,
    /// Per-bone dense channels (same bone_name order as source clip).
    pub channels: Vec<DenseBoneChannel>,
}

#[derive(Debug, Clone)]
pub struct DenseBoneChannel {
    pub bone_name: String,
    /// World-time of each sample.
    pub times: Vec<f32>,
    pub translations: Vec<[f32; 3]>,
    pub rotations: Vec<[f32; 4]>,
    pub scales: Vec<[f32; 3]>,
}

/// Resample a (possibly sparse / spline-compressed) `AnimationClip` onto a
/// uniform grid at `rate_hz`.
pub fn expand(clip: &AnimationClip, rate_hz: f32) -> DenseClip {
    assert!(rate_hz > 0.0, "rate_hz must be positive");
    let duration = clip.duration;
    let frame_count = (duration * rate_hz).ceil() as usize + 1;
    let dt = if frame_count > 1 {
        duration / (frame_count - 1) as f32
    } else {
        0.0
    };

    let times: Vec<f32> = (0..frame_count)
        .map(|i| (i as f32 * dt).min(duration))
        .collect();

    let channels = clip
        .channels
        .iter()
        .map(|ch| {
            let translations = times
                .iter()
                .map(|&t| sample_vec3(&ch.translations, t))
                .collect();
            let rotations = times
                .iter()
                .map(|&t| sample_quat(&ch.rotations, t))
                .collect();
            let scales = times.iter().map(|&t| sample_vec3(&ch.scales, t)).collect();
            DenseBoneChannel {
                bone_name: ch.bone_name.clone(),
                times: times.clone(),
                translations,
                rotations,
                scales,
            }
        })
        .collect();

    DenseClip {
        rate_hz,
        duration,
        frame_count,
        channels,
    }
}

/// Sample a sparse Vec3 keyframe track at time `t` using step/linear interpolation.
fn sample_vec3(keyframes: &[AnimationKeyframe<[f32; 3]>], t: f32) -> [f32; 3] {
    if keyframes.is_empty() {
        return [0.0, 0.0, 0.0];
    }
    if keyframes.len() == 1 || t <= keyframes[0].time {
        return keyframes[0].value;
    }
    let last = keyframes.last().unwrap();
    if t >= last.time {
        return last.value;
    }
    // Binary search for the bracketing pair.
    let idx = keyframes.partition_point(|kf| kf.time <= t);
    let lo = &keyframes[idx - 1];
    let hi = &keyframes[idx];
    let span = hi.time - lo.time;
    let alpha = if span > 1e-10 {
        (t - lo.time) / span
    } else {
        0.0
    };
    let a = lo.value;
    let b = hi.value;
    [
        a[0] + (b[0] - a[0]) * alpha,
        a[1] + (b[1] - a[1]) * alpha,
        a[2] + (b[2] - a[2]) * alpha,
    ]
}

/// Sample a sparse quaternion keyframe track at time `t` using slerp.
fn sample_quat(keyframes: &[AnimationKeyframe<[f32; 4]>], t: f32) -> [f32; 4] {
    if keyframes.is_empty() {
        return [0.0, 0.0, 0.0, 1.0];
    }
    if keyframes.len() == 1 || t <= keyframes[0].time {
        return keyframes[0].value;
    }
    let last = keyframes.last().unwrap();
    if t >= last.time {
        return last.value;
    }
    let idx = keyframes.partition_point(|kf| kf.time <= t);
    let lo = &keyframes[idx - 1];
    let hi = &keyframes[idx];
    let span = hi.time - lo.time;
    let alpha = if span > 1e-10 {
        (t - lo.time) / span
    } else {
        0.0
    };
    quat_slerp(&lo.value, &hi.value, alpha)
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
        let len = |v: [f32; 4]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2] + v[3] * v[3]).sqrt();
        let r = [
            a[0] + t * (b_adj[0] - a[0]),
            a[1] + t * (b_adj[1] - a[1]),
            a[2] + t * (b_adj[2] - a[2]),
            a[3] + t * (b_adj[3] - a[3]),
        ];
        let l = len(r);
        return if l < 1e-10 {
            [0.0, 0.0, 0.0, 1.0]
        } else {
            [r[0] / l, r[1] / l, r[2] / l, r[3] / l]
        };
    }
    let theta_0 = dot.acos();
    let theta = theta_0 * t;
    let s0 = (theta_0 - theta).cos() - dot * theta.sin() / theta_0.sin();
    let s1 = theta.sin() / theta_0.sin();
    [
        s0 * a[0] + s1 * b_adj[0],
        s0 * a[1] + s1 * b_adj[1],
        s0 * a[2] + s1 * b_adj[2],
        s0 * a[3] + s1 * b_adj[3],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::clip::{AnimationClip, AnimationEvent, AnimationKeyframe, BoneChannel};

    fn make_clip(duration: f32) -> AnimationClip {
        AnimationClip {
            source_format: "test".into(),
            duration,
            native_fps: 30.0,
            channels: vec![BoneChannel {
                bone_name: "Root".into(),
                translations: vec![
                    AnimationKeyframe {
                        time: 0.0,
                        value: [0.0, 0.0, 0.0],
                    },
                    AnimationKeyframe {
                        time: duration,
                        value: [1.0, 0.0, 0.0],
                    },
                ],
                rotations: vec![
                    AnimationKeyframe {
                        time: 0.0,
                        value: [0.0, 0.0, 0.0, 1.0],
                    },
                    AnimationKeyframe {
                        time: duration,
                        value: [0.0, 0.0, 0.0, 1.0],
                    },
                ],
                scales: vec![
                    AnimationKeyframe {
                        time: 0.0,
                        value: [1.0, 1.0, 1.0],
                    },
                    AnimationKeyframe {
                        time: duration,
                        value: [1.0, 1.0, 1.0],
                    },
                ],
            }],
            events: vec![],
            original_skeleton_name: None,
            warnings: vec![],
            is_additive: false,
            track_to_bone_indices: vec![],
            extracted_motion_ref: String::new(),
        }
    }

    #[test]
    fn expand_frame_count_matches_duration_times_rate() {
        let clip = make_clip(1.0);
        let dense = expand(&clip, 60.0);
        // For a 1-second clip at 60 Hz: 61 frames.
        assert_eq!(dense.frame_count, 61);
        assert_eq!(dense.channels[0].times.len(), 61);
    }

    #[test]
    fn expand_interpolates_midpoint() {
        let clip = make_clip(1.0);
        let dense = expand(&clip, 2.0);
        // 3 frames: t=0, t=0.5, t=1.
        assert_eq!(dense.frame_count, 3);
        let mid_trans = dense.channels[0].translations[1];
        // At t=0.5, translation should be [0.5, 0, 0].
        assert!((mid_trans[0] - 0.5).abs() < 1e-5, "mid_trans={mid_trans:?}");
    }

    #[test]
    fn expand_first_and_last_frames() {
        let clip = make_clip(2.0);
        let dense = expand(&clip, 30.0);
        let first = dense.channels[0].translations[0];
        let last = *dense.channels[0].translations.last().unwrap();
        assert!((first[0] - 0.0).abs() < 1e-5);
        assert!((last[0] - 1.0).abs() < 1e-5);
    }
}
