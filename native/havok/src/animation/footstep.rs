/// Footstep timing / analysis.
///
/// Detects planted-foot events from per-frame ankle/heel world-space
/// velocities. Emits `FootstepEvent` candidates at velocity zero-crossings.
/// Used to auto-derive footstep events for sound-tag injection on imported
/// animations.

/// A detected footstep event candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct FootstepEvent {
    /// Time in seconds of the planted frame.
    pub time: f32,
    /// Which bone produced this event (e.g. "LeftAnkle").
    pub bone: String,
    /// Approximate velocity magnitude at the event (lower = more planted).
    pub intensity: f32,
}

/// Parameters controlling the footstep detector.
#[derive(Debug, Clone)]
pub struct FootstepParams {
    /// Frame rate of the dense sample sequence (frames per second).
    pub fps: f32,
    /// Maximum velocity (units/second) to be classified as "planted".
    pub velocity_threshold: f32,
    /// Smoothing window half-size in frames (0 = no smoothing).
    pub smoothing_half_window: usize,
}

impl Default for FootstepParams {
    fn default() -> Self {
        FootstepParams {
            fps: 60.0,
            velocity_threshold: 0.05,
            smoothing_half_window: 2,
        }
    }
}

/// Per-bone dense world-space position sequence.
#[derive(Debug, Clone)]
pub struct BonePositionSequence {
    pub bone_name: String,
    /// World-space positions at each frame, in order.
    pub positions: Vec<[f32; 3]>,
}

/// Detect footstep events in a set of position sequences.
///
/// For each bone, computes per-frame velocity magnitude, smooths it, then
/// finds frames where velocity drops below `params.velocity_threshold` after
/// a higher-velocity frame (zero-crossing into planted state).
pub fn detect_footsteps(
    sequences: &[BonePositionSequence],
    params: &FootstepParams,
) -> Vec<FootstepEvent> {
    let dt = 1.0 / params.fps;
    let mut events = Vec::new();

    for seq in sequences {
        let n = seq.positions.len();
        if n < 2 {
            continue;
        }

        // Compute per-frame velocity magnitudes using central differences where
        // possible; forward difference for frame 0, backward for last frame.
        let vel_at = |i: usize| -> f32 {
            if i == 0 {
                let dx = seq.positions[1][0] - seq.positions[0][0];
                let dy = seq.positions[1][1] - seq.positions[0][1];
                let dz = seq.positions[1][2] - seq.positions[0][2];
                (dx * dx + dy * dy + dz * dz).sqrt() / dt
            } else {
                let dx = seq.positions[i][0] - seq.positions[i - 1][0];
                let dy = seq.positions[i][1] - seq.positions[i - 1][1];
                let dz = seq.positions[i][2] - seq.positions[i - 1][2];
                (dx * dx + dy * dy + dz * dz).sqrt() / dt
            }
        };
        let mut vels: Vec<f32> = (0..n).map(vel_at).collect();

        // Smooth velocities with a box filter.
        let hw = params.smoothing_half_window;
        if hw > 0 {
            let raw = vels.clone();
            for i in 0..n {
                let lo = i.saturating_sub(hw);
                let hi = (i + hw + 1).min(n);
                let sum: f32 = raw[lo..hi].iter().sum();
                vels[i] = sum / (hi - lo) as f32;
            }
        }

        // Detect zero-crossings into the planted state.
        let mut was_planted = false;
        for i in 0..n {
            let planted = vels[i] <= params.velocity_threshold;
            if planted && !was_planted {
                events.push(FootstepEvent {
                    time: i as f32 * dt,
                    bone: seq.bone_name.clone(),
                    intensity: vels[i],
                });
            }
            was_planted = planted;
        }
    }

    events.sort_by(|a, b| {
        a.time
            .partial_cmp(&b.time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a walk-cycle-like sequence: foot is planted (stationary) at frames 0–5,
    /// then moves frames 6–25, then plants again at frames 26–30.
    fn walk_cycle_positions() -> Vec<[f32; 3]> {
        (0..31)
            .map(|i| {
                if i < 6 {
                    [0.0, 0.0, 0.0]
                } else if i <= 25 {
                    [(i - 5) as f32 * 0.1, 0.0, 0.0]
                } else {
                    [2.0, 0.0, 0.0]
                }
            })
            .collect()
    }

    #[test]
    fn detects_planted_frames_in_walk_cycle() {
        let seq = BonePositionSequence {
            bone_name: "LeftAnkle".into(),
            positions: walk_cycle_positions(),
        };
        let params = FootstepParams {
            fps: 60.0,
            velocity_threshold: 0.1,
            smoothing_half_window: 0,
        };
        let events = detect_footsteps(&[seq], &params);
        // Should detect at least one planted event (at the start of the plant phases).
        assert!(!events.is_empty(), "expected at least one footstep event");
        // All events should reference the correct bone.
        assert!(events.iter().all(|e| e.bone == "LeftAnkle"));
    }

    #[test]
    fn no_events_for_constant_motion() {
        // Constant velocity: no zero-crossing — no events.
        let positions: Vec<[f32; 3]> = (0..60).map(|i| [i as f32 * 1.0, 0.0, 0.0]).collect();
        let seq = BonePositionSequence {
            bone_name: "RightAnkle".into(),
            positions,
        };
        let params = FootstepParams {
            fps: 60.0,
            velocity_threshold: 0.05,
            smoothing_half_window: 0,
        };
        let events = detect_footsteps(&[seq], &params);
        assert!(
            events.is_empty(),
            "expected no events for constant-velocity motion"
        );
    }

    #[test]
    fn detects_events_for_multiple_bones() {
        // Left: planted (low velocity) at frame 0, then moving.
        // Right: moving at frame 0, planted at frame 2.
        // Using fps=1.0 so velocity = position difference directly.
        let left = BonePositionSequence {
            bone_name: "Left".into(),
            positions: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
        };
        let right = BonePositionSequence {
            bone_name: "Right".into(),
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
        };
        let params = FootstepParams {
            fps: 1.0,
            velocity_threshold: 0.05,
            smoothing_half_window: 0,
        };
        let events = detect_footsteps(&[left, right], &params);
        let bones: Vec<&str> = events.iter().map(|e| e.bone.as_str()).collect();
        assert!(
            bones.contains(&"Left"),
            "expected Left event, got: {bones:?}"
        );
        assert!(
            bones.contains(&"Right"),
            "expected Right event, got: {bones:?}"
        );
    }
}
