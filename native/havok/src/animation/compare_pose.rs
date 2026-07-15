/// Compare-pose utilities.
///
/// Diff two poses (same skeleton): per-bone position delta, rotation angle
/// delta, and summary statistics (max, mean). Used to validate FO76→FO4
/// conversion produced equivalent pose at each frame.
use crate::animation::pose::{Pose, vec3_len, vec3_sub};

/// Per-bone difference between two poses.
#[derive(Debug, Clone)]
pub struct BoneDiff {
    pub bone_name: String,
    /// Euclidean distance between model-space positions.
    pub position_delta: f32,
    /// Angular distance between model-space rotations (radians, in [0, π]).
    pub rotation_angle: f32,
}

/// Summary statistics for a full pose diff.
#[derive(Debug, Clone)]
pub struct PoseDiffSummary {
    pub per_bone: Vec<BoneDiff>,
    pub max_position_delta: f32,
    pub mean_position_delta: f32,
    pub max_rotation_angle: f32,
    pub mean_rotation_angle: f32,
}

/// Compute pose diff between two poses with the same skeleton.
///
/// Both poses must have the same bone count.
pub fn pose_diff(a: &mut Pose, b: &mut Pose) -> PoseDiffSummary {
    let n = a.bone_count();
    assert_eq!(n, b.bone_count(), "pose bone count mismatch");

    let mut per_bone = Vec::with_capacity(n);

    for i in 0..n {
        let ma = a.model_at(i);
        let mb = b.model_at(i);

        let pos_delta = vec3_len(&vec3_sub(&ma.translation, &mb.translation));
        let rot_angle = quat_angle_between(&ma.rotation, &mb.rotation);

        per_bone.push(BoneDiff {
            bone_name: a.skeleton().bone_names[i].clone(),
            position_delta: pos_delta,
            rotation_angle: rot_angle,
        });
    }

    let max_pos = per_bone
        .iter()
        .map(|b| b.position_delta)
        .fold(0.0f32, f32::max);
    let mean_pos = if n > 0 {
        per_bone.iter().map(|b| b.position_delta).sum::<f32>() / n as f32
    } else {
        0.0
    };
    let max_rot = per_bone
        .iter()
        .map(|b| b.rotation_angle)
        .fold(0.0f32, f32::max);
    let mean_rot = if n > 0 {
        per_bone.iter().map(|b| b.rotation_angle).sum::<f32>() / n as f32
    } else {
        0.0
    };

    PoseDiffSummary {
        per_bone,
        max_position_delta: max_pos,
        mean_position_delta: mean_pos,
        max_rotation_angle: max_rot,
        mean_rotation_angle: mean_rot,
    }
}

/// Angular distance between two unit quaternions in radians ∈ [0, π].
fn quat_angle_between(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    // dot(a, b) = cos(half-angle between them). Clamp for numerical stability.
    let dot = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3])
        .abs()
        .clamp(0.0, 1.0);
    2.0 * dot.acos()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::pose::{Pose, PoseSkeleton, QsTransform};

    fn single_bone_skel() -> PoseSkeleton {
        PoseSkeleton {
            bone_names: vec!["Root".into()],
            parent_indices: vec![-1],
            reference_local: vec![QsTransform::IDENTITY],
        }
    }

    #[test]
    fn diff_identical_poses_is_zero() {
        let mut a = Pose::from_reference(single_bone_skel());
        let mut b = Pose::from_reference(single_bone_skel());
        let diff = pose_diff(&mut a, &mut b);
        assert!(diff.max_position_delta < 1e-6);
        assert!(diff.max_rotation_angle < 1e-6);
    }

    #[test]
    fn diff_translated_pose() {
        let skel = single_bone_skel();
        let mut a = Pose::from_reference(skel.clone());
        let mut b = Pose::from_local(
            skel,
            vec![QsTransform {
                translation: [1.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            }],
        );
        let diff = pose_diff(&mut a, &mut b);
        assert!((diff.max_position_delta - 1.0).abs() < 1e-5);
        assert!(diff.max_rotation_angle < 1e-5);
    }

    #[test]
    fn diff_rotated_pose() {
        let skel = single_bone_skel();
        let mut a = Pose::from_reference(skel.clone());
        // 90° rotation around Z.
        let half = std::f32::consts::FRAC_PI_4;
        let mut b = Pose::from_local(
            skel,
            vec![QsTransform {
                translation: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, half.sin(), half.cos()],
                scale: [1.0, 1.0, 1.0],
            }],
        );
        let diff = pose_diff(&mut a, &mut b);
        // 90° rotation angle.
        assert!((diff.max_rotation_angle - std::f32::consts::FRAC_PI_2).abs() < 0.01);
    }
}
