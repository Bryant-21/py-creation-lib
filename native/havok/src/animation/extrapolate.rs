/// Extends a short clip to a target duration by looping, holding the last frame,
/// or ping-ponging. Follows SDK `hkaParametricAnimationExtrapolationUtil` semantics.
use crate::animation::clip::{AnimationClip, AnimationKeyframe, BoneChannel};

/// Extension policy for `extrapolate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtrapolationPolicy {
    /// Repeat the clip cyclically.
    Cyclic,
    /// Hold the final keyframe value beyond the clip's end.
    LinearHold,
    /// Ping-pong: reflect back-and-forth.
    Mirror,
}

/// Extrapolate a clip to `target_duration` seconds.
///
/// The original clip's keyframes are preserved and additional keyframes are
/// appended to reach `target_duration`. If `target_duration ≤ clip.duration`,
/// the clip is returned unchanged.
pub fn extrapolate(
    clip: &AnimationClip,
    target_duration: f32,
    policy: ExtrapolationPolicy,
) -> AnimationClip {
    if target_duration <= clip.duration + 1e-6 {
        return clip.clone();
    }

    let orig_duration = clip.duration;
    let new_channels = clip
        .channels
        .iter()
        .map(|ch| {
            let trans =
                extrapolate_vec3_track(&ch.translations, orig_duration, target_duration, policy);
            let rots =
                extrapolate_quat_track(&ch.rotations, orig_duration, target_duration, policy);
            let scales = extrapolate_vec3_track(&ch.scales, orig_duration, target_duration, policy);
            BoneChannel {
                bone_name: ch.bone_name.clone(),
                translations: trans,
                rotations: rots,
                scales,
            }
        })
        .collect();

    AnimationClip {
        source_format: clip.source_format.clone(),
        duration: target_duration,
        native_fps: clip.native_fps,
        channels: new_channels,
        events: extrapolate_events(&clip.events, orig_duration, target_duration, policy),
        original_skeleton_name: clip.original_skeleton_name.clone(),
        warnings: clip.warnings.clone(),
        is_additive: clip.is_additive,
        track_to_bone_indices: clip.track_to_bone_indices.clone(),
        extracted_motion_ref: clip.extracted_motion_ref.clone(),
    }
}

fn extrapolate_vec3_track(
    kfs: &[AnimationKeyframe<[f32; 3]>],
    orig: f32,
    target: f32,
    policy: ExtrapolationPolicy,
) -> Vec<AnimationKeyframe<[f32; 3]>> {
    if kfs.is_empty() {
        return vec![];
    }
    let mut result = kfs.to_vec();
    // Add a closing keyframe at exactly orig if not present.
    let end_val = sample_vec3(kfs, orig);
    if let Some(last) = result.last() {
        if (last.time - orig).abs() > 1e-6 {
            result.push(AnimationKeyframe {
                time: orig,
                value: end_val,
            });
        }
    }
    // Now append keyframes in the extended region using the policy.
    let extra_times = extra_sample_times(&result, orig, target);
    for t in extra_times {
        let v = map_time_to_value_vec3(&result, orig, t, policy);
        result.push(AnimationKeyframe { time: t, value: v });
    }
    result
}

fn extrapolate_quat_track(
    kfs: &[AnimationKeyframe<[f32; 4]>],
    orig: f32,
    target: f32,
    policy: ExtrapolationPolicy,
) -> Vec<AnimationKeyframe<[f32; 4]>> {
    if kfs.is_empty() {
        return vec![];
    }
    let mut result = kfs.to_vec();
    let end_val = sample_quat(kfs, orig);
    if let Some(last) = result.last() {
        if (last.time - orig).abs() > 1e-6 {
            result.push(AnimationKeyframe {
                time: orig,
                value: end_val,
            });
        }
    }
    let extra_times = extra_sample_times(&result, orig, target);
    for t in extra_times {
        let v = map_time_to_value_quat(&result, orig, t, policy);
        result.push(AnimationKeyframe { time: t, value: v });
    }
    result
}

/// Generate a small number of evenly-spaced sample times in (orig, target].
fn extra_sample_times<T>(kfs: &[AnimationKeyframe<T>], orig: f32, target: f32) -> Vec<f32> {
    // Use the same keyframe density as the original for continuity.
    let n_orig = kfs.len().max(2);
    let orig_density = n_orig as f32 / orig.max(1e-6);
    let n_extra = ((target - orig) * orig_density).ceil() as usize + 1;
    let n_extra = n_extra.clamp(2, 256);
    (1..=n_extra)
        .map(|i| orig + (target - orig) * i as f32 / n_extra as f32)
        .collect()
}

fn map_time_to_value_vec3(
    kfs: &[AnimationKeyframe<[f32; 3]>],
    orig: f32,
    t: f32,
    policy: ExtrapolationPolicy,
) -> [f32; 3] {
    let mapped = map_time(t, orig, policy);
    sample_vec3(kfs, mapped)
}

fn map_time_to_value_quat(
    kfs: &[AnimationKeyframe<[f32; 4]>],
    orig: f32,
    t: f32,
    policy: ExtrapolationPolicy,
) -> [f32; 4] {
    let mapped = map_time(t, orig, policy);
    sample_quat(kfs, mapped)
}

/// Map an out-of-range time `t > orig` back into `[0, orig]` using the policy.
fn map_time(t: f32, orig: f32, policy: ExtrapolationPolicy) -> f32 {
    if orig < 1e-10 {
        return 0.0;
    }
    match policy {
        ExtrapolationPolicy::LinearHold => orig,
        ExtrapolationPolicy::Cyclic => {
            let cycle = t % orig;
            if cycle < 0.0 { cycle + orig } else { cycle }
        }
        ExtrapolationPolicy::Mirror => {
            // Cycle period = 2 * orig.
            let period = 2.0 * orig;
            let phase = t % period;
            let phase = if phase < 0.0 { phase + period } else { phase };
            if phase <= orig { phase } else { period - phase }
        }
    }
}

fn sample_vec3(kfs: &[AnimationKeyframe<[f32; 3]>], t: f32) -> [f32; 3] {
    if kfs.is_empty() {
        return [0.0, 0.0, 0.0];
    }
    if kfs.len() == 1 || t <= kfs[0].time {
        return kfs[0].value;
    }
    let last = kfs.last().unwrap();
    if t >= last.time {
        return last.value;
    }
    let idx = kfs.partition_point(|kf| kf.time <= t);
    let lo = &kfs[idx - 1];
    let hi = &kfs[idx];
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

fn sample_quat(kfs: &[AnimationKeyframe<[f32; 4]>], t: f32) -> [f32; 4] {
    if kfs.is_empty() {
        return [0.0, 0.0, 0.0, 1.0];
    }
    if kfs.len() == 1 || t <= kfs[0].time {
        return kfs[0].value;
    }
    let last = kfs.last().unwrap();
    if t >= last.time {
        return last.value;
    }
    let idx = kfs.partition_point(|kf| kf.time <= t);
    let lo = &kfs[idx - 1];
    let hi = &kfs[idx];
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
        let r = [
            a[0] + t * (b_adj[0] - a[0]),
            a[1] + t * (b_adj[1] - a[1]),
            a[2] + t * (b_adj[2] - a[2]),
            a[3] + t * (b_adj[3] - a[3]),
        ];
        let l = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2] + r[3] * r[3]).sqrt();
        return if l < 1e-10 {
            [0.0, 0.0, 0.0, 1.0]
        } else {
            [r[0] / l, r[1] / l, r[2] / l, r[3] / l]
        };
    }
    let t0 = dot.acos();
    let th = t0 * t;
    let s0 = (t0 - th).cos() - dot * th.sin() / t0.sin();
    let s1 = th.sin() / t0.sin();
    [
        s0 * a[0] + s1 * b_adj[0],
        s0 * a[1] + s1 * b_adj[1],
        s0 * a[2] + s1 * b_adj[2],
        s0 * a[3] + s1 * b_adj[3],
    ]
}

fn extrapolate_events(
    events: &[crate::animation::clip::AnimationEvent],
    orig: f32,
    target: f32,
    policy: ExtrapolationPolicy,
) -> Vec<crate::animation::clip::AnimationEvent> {
    // Preserve original events, then add repeated events for cyclic.
    let mut result = events.to_vec();
    if policy == ExtrapolationPolicy::Cyclic {
        let mut cycle = orig;
        while cycle < target - 1e-6 {
            for ev in events {
                let t = cycle + ev.time;
                if t < target {
                    result.push(crate::animation::clip::AnimationEvent {
                        time: t,
                        text: ev.text.clone(),
                    });
                }
            }
            cycle += orig;
        }
    }
    result
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
                        time: duration * 0.5,
                        value: [0.5, 0.0, 0.0],
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
            events: vec![AnimationEvent {
                time: 0.25,
                text: "footstep".into(),
            }],
            original_skeleton_name: None,
            warnings: vec![],
            is_additive: false,
            track_to_bone_indices: vec![],
            extracted_motion_ref: String::new(),
        }
    }

    #[test]
    fn extrapolate_noop_when_duration_unchanged() {
        let clip = make_clip(1.0);
        let result = extrapolate(&clip, 1.0, ExtrapolationPolicy::Cyclic);
        assert_eq!(result.duration, clip.duration);
        assert_eq!(
            result.channels[0].translations.len(),
            clip.channels[0].translations.len()
        );
    }

    #[test]
    fn cyclic_second_half_mirrors_first() {
        // 1s clip extrapolated to 2s cyclically: second half should repeat.
        let clip = make_clip(1.0);
        let result = extrapolate(&clip, 2.0, ExtrapolationPolicy::Cyclic);
        assert_eq!(result.duration, 2.0);
        let trans = &result.channels[0].translations;
        // At t=0.5s original = 0.5 in X; at t=1.5s cyclic should also be ~0.5.
        let val_at = |t: f32| -> f32 {
            let idx = trans.partition_point(|kf| kf.time <= t);
            if idx == 0 {
                return trans[0].value[0];
            }
            if idx >= trans.len() {
                return trans.last().unwrap().value[0];
            }
            let lo = &trans[idx - 1];
            let hi = &trans[idx];
            let alpha = (t - lo.time) / (hi.time - lo.time);
            lo.value[0] + (hi.value[0] - lo.value[0]) * alpha
        };
        let at_half = val_at(0.5);
        let at_one_half = val_at(1.5);
        assert!(
            (at_half - at_one_half).abs() < 0.05,
            "cyclic: {at_half} vs {at_one_half}"
        );
    }

    #[test]
    fn mirror_policy_reverses_second_half() {
        let clip = make_clip(1.0);
        let result = extrapolate(&clip, 2.0, ExtrapolationPolicy::Mirror);
        assert_eq!(result.duration, 2.0);
        // Mirror: value at t=1.5 should mirror value at t=0.5.
        let trans = &result.channels[0].translations;
        let val_at = |t: f32| -> f32 {
            let idx = trans.partition_point(|kf| kf.time <= t);
            if idx == 0 {
                return trans[0].value[0];
            }
            if idx >= trans.len() {
                return trans.last().unwrap().value[0];
            }
            let lo = &trans[idx - 1];
            let hi = &trans[idx];
            let alpha = (t - lo.time) / (hi.time - lo.time);
            lo.value[0] + (hi.value[0] - lo.value[0]) * alpha
        };
        let a = val_at(0.5);
        let b = val_at(1.5);
        assert!((a - b).abs() < 0.1, "mirror: {a} vs {b}");
    }

    #[test]
    fn hold_policy_holds_last_value() {
        let clip = make_clip(1.0);
        let result = extrapolate(&clip, 2.0, ExtrapolationPolicy::LinearHold);
        assert_eq!(result.duration, 2.0);
        let trans = &result.channels[0].translations;
        // Every frame after t=1.0 should have value[0] == 1.0.
        for kf in trans.iter().filter(|kf| kf.time > 1.0) {
            assert!(
                (kf.value[0] - 1.0).abs() < 1e-4,
                "hold: t={} v={}",
                kf.time,
                kf.value[0]
            );
        }
    }
}
