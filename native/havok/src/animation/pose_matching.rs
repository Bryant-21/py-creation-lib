/// Pose-matching utility.
///
/// Indexes a set of named poses (each a quantized rotation fingerprint) and
/// supports nearest-neighbor lookup. Used for "find the get-up animation that
/// ends in T-pose" and similar semantic animation queries.
use crate::animation::pose::{Pose, QsTransform, vec3_dot};

// ---------------------------------------------------------------------------
// Fingerprint
// ---------------------------------------------------------------------------

/// A compact, order-independent fingerprint for a pose.
#[derive(Debug, Clone)]
pub struct PoseFingerprint {
    /// Per-bone quantized rotation components (4 × u8 per bone).
    pub quats: Vec<[u8; 4]>,
    /// Quantized root displacement (3 × i16).
    pub root_displacement: [i16; 3],
}

impl PoseFingerprint {
    /// Build a fingerprint from a pose. Quantizes each quaternion component
    /// to an i8 in [-127, 127] mapped to u8 via bias, and the root translation
    /// to i16 (millimeters).
    pub fn from_pose(pose: &mut Pose) -> Self {
        let n = pose.bone_count();
        let mut quats = Vec::with_capacity(n);
        for i in 0..n {
            let model = pose.model_at(i);
            let q = model.rotation;
            // Ensure canonical hemisphere (w ≥ 0).
            let sign = if q[3] < 0.0 { -1.0 } else { 1.0 };
            let quantize = |v: f32| -> u8 {
                let v = (v * sign).clamp(-1.0, 1.0);
                ((v * 127.0) as i8 as u8).wrapping_add(128)
            };
            quats.push([
                quantize(q[0]),
                quantize(q[1]),
                quantize(q[2]),
                quantize(q[3]),
            ]);
        }
        let root = pose.model_at(0).translation;
        PoseFingerprint {
            quats,
            root_displacement: [
                (root[0] * 1000.0).round().clamp(-32768.0, 32767.0) as i16,
                (root[1] * 1000.0).round().clamp(-32768.0, 32767.0) as i16,
                (root[2] * 1000.0).round().clamp(-32768.0, 32767.0) as i16,
            ],
        }
    }

    /// L2-squared distance to another fingerprint (bone count must match).
    pub fn distance_sq(&self, other: &PoseFingerprint) -> f64 {
        assert_eq!(self.quats.len(), other.quats.len(), "bone count mismatch");
        let mut sum = 0.0f64;
        for (a, b) in self.quats.iter().zip(other.quats.iter()) {
            for (av, bv) in a.iter().zip(b.iter()) {
                let d = (*av as f64) - (*bv as f64);
                sum += d * d;
            }
        }
        let rdx = (self.root_displacement[0] as f64) - (other.root_displacement[0] as f64);
        let rdy = (self.root_displacement[1] as f64) - (other.root_displacement[1] as f64);
        let rdz = (self.root_displacement[2] as f64) - (other.root_displacement[2] as f64);
        // Root displacement weighted lower (scale to roughly match rotation units).
        sum + (rdx * rdx + rdy * rdy + rdz * rdz) * 1e-4
    }
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

/// An indexed database of named poses for nearest-neighbor lookup.
#[derive(Debug, Default)]
pub struct PoseDatabase {
    entries: Vec<(String, PoseFingerprint)>,
}

impl PoseDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a pose to the database under a given name.
    pub fn insert(&mut self, name: impl Into<String>, fp: PoseFingerprint) {
        self.entries.push((name.into(), fp));
    }

    /// Return the top-K closest entries by fingerprint distance.
    /// Returns `(name, distance_sq)` pairs sorted by ascending distance.
    pub fn query(&self, query: &PoseFingerprint, k: usize) -> Vec<(&str, f64)> {
        let mut scored: Vec<(&str, f64)> = self
            .entries
            .iter()
            .map(|(name, fp)| (name.as_str(), fp.distance_sq(query)))
            .collect();
        scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        scored
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::pose::{Pose, PoseSkeleton, QsTransform};

    fn single_bone_skeleton() -> PoseSkeleton {
        PoseSkeleton {
            bone_names: vec!["Root".into()],
            parent_indices: vec![-1],
            reference_local: vec![QsTransform::IDENTITY],
        }
    }

    fn make_pose(translation: [f32; 3], rotation: [f32; 4]) -> Pose {
        let skel = single_bone_skeleton();
        Pose::from_local(
            skel,
            vec![QsTransform {
                translation,
                rotation,
                scale: [1.0, 1.0, 1.0],
            }],
        )
    }

    #[test]
    fn query_returns_nearest_pose() {
        let mut db = PoseDatabase::new();

        // 10 distinct poses.
        let mut poses: Vec<Pose> = (0..10)
            .map(|i| {
                let angle = (i as f32) * std::f32::consts::TAU / 10.0;
                make_pose(
                    [0.0, 0.0, 0.0],
                    [0.0, 0.0, (angle * 0.5).sin(), (angle * 0.5).cos()],
                )
            })
            .collect();

        for (i, pose) in poses.iter_mut().enumerate() {
            let fp = PoseFingerprint::from_pose(pose);
            db.insert(format!("pose_{i}"), fp);
        }

        // Query with pose_0 perturbed slightly.
        let mut query_pose = make_pose([0.0, 0.0, 0.0], [0.0, 0.0, 0.001, 0.9999995]);
        let query_fp = PoseFingerprint::from_pose(&mut query_pose);
        let results = db.query(&query_fp, 1);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].0, "pose_0",
            "nearest should be pose_0, got {}",
            results[0].0
        );
    }

    #[test]
    fn fingerprint_self_distance_is_zero() {
        let mut pose = make_pose([1.0, 2.0, 3.0], [0.0, 0.0, 0.0, 1.0]);
        let fp = PoseFingerprint::from_pose(&mut pose);
        assert_eq!(fp.distance_sq(&fp), 0.0);
    }
}
