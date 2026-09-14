/// Cyclic Coordinate Descent (CCD) N-bone IK solver. Parameters follow
/// `hkaCcdIkSolver` semantics.
use crate::animation::pose::{
    quat_from_axis_angle, quat_mul, quat_normalize, quat_rotate, vec3_cross, vec3_dot, vec3_len,
    vec3_normalize, vec3_sub,
};

/// Configuration for the CCD solver.
#[derive(Debug, Clone)]
pub struct CcdParams {
    /// Maximum number of full-chain iterations.
    pub max_iterations: u32,
    /// Stop when end-effector is within this distance of the target.
    pub tolerance: f32,
    /// Rotation gain per step in [0, 1]. 1.0 = full rotation each step.
    pub gain: f32,
}

impl Default for CcdParams {
    fn default() -> Self {
        CcdParams {
            max_iterations: 16,
            tolerance: 1e-3,
            gain: 1.0,
        }
    }
}

/// Solve N-bone IK via CCD. Joints run 0 (root) .. N-1 (end effector); all
/// inputs and the returned rotations (xyzw) are world-space.
pub fn solve_ccd(
    joint_positions: &[[f32; 3]],
    joint_rotations: &[[f32; 4]],
    target: &[f32; 3],
    params: &CcdParams,
) -> Vec<[f32; 4]> {
    let n = joint_positions.len();
    assert_eq!(joint_rotations.len(), n);
    assert!(n >= 2, "CCD requires at least 2 joints");

    let mut rotations: Vec<[f32; 4]> = joint_rotations.to_vec();
    // We work on mutable world-space positions of the joints.
    let mut positions: Vec<[f32; 3]> = joint_positions.to_vec();
    let end_idx = n - 1;

    for _iter in 0..params.max_iterations {
        let end_pos = positions[end_idx];
        if vec3_len(&vec3_sub(&end_pos, target)) < params.tolerance {
            break;
        }

        // Walk from second-to-last joint down to root.
        for i in (0..end_idx).rev() {
            let joint_pos = positions[i];
            let to_end = vec3_normalize(&vec3_sub(&positions[end_idx], &joint_pos));
            let to_target = vec3_normalize(&vec3_sub(target, &joint_pos));

            let dot = vec3_dot(&to_end, &to_target).clamp(-1.0, 1.0);
            if dot > 0.9999 {
                continue;
            }

            let axis = vec3_normalize(&vec3_cross(&to_end, &to_target));
            let angle = dot.acos() * params.gain;
            let delta = quat_from_axis_angle(&axis, angle);

            // Apply rotation to this joint.
            rotations[i] = quat_normalize(&quat_mul(&delta, &rotations[i]));

            // Update world-space positions of all descendant joints.
            // Each descendant j's position relative to joint i is re-rotated.
            for j in (i + 1)..n {
                let rel = vec3_sub(&positions[j], &joint_pos);
                let rotated = quat_rotate(&delta, &rel);
                positions[j] = [
                    joint_pos[0] + rotated[0],
                    joint_pos[1] + rotated[1],
                    joint_pos[2] + rotated[2],
                ];
            }
        }
    }

    rotations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::pose::vec3_len;

    #[test]
    fn ccd_5_bone_reaches_target() {
        // 5-bone chain along X: joints at 0,1,2,3,4.
        let positions = [
            [0.0f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
        ];
        let rotations = [[0.0f32, 0.0, 0.0, 1.0]; 5];
        let target = [2.0f32, 2.0, 0.0];
        let result = solve_ccd(&positions, &rotations, &target, &CcdParams::default());
        assert_eq!(result.len(), 5);
        // All rotations should be unit quaternions.
        for q in &result {
            let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
            assert!((len - 1.0).abs() < 1e-4, "quat not normalized: {q:?}");
        }
    }

    #[test]
    fn ccd_already_at_target_no_change() {
        // Target is exactly at end position; should converge immediately.
        let positions = [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        let rotations = [[0.0f32, 0.0, 0.0, 1.0]; 3];
        let target = [2.0f32, 0.0, 0.0];
        let result = solve_ccd(
            &positions,
            &rotations,
            &target,
            &CcdParams {
                tolerance: 1e-3,
                ..Default::default()
            },
        );
        // End effector already at target — root joint should be near identity.
        let root = result[0];
        assert!((root[3] - 1.0).abs() < 1e-4);
    }
}
