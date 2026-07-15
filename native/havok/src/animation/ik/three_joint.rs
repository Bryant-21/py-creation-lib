use super::two_bone::{TwoBoneParams, TwoBoneResult, solve_two_bone};
/// Three-joint analytic IK solver.
///
/// Extends two-bone IK by adding a third (base) joint. The solver first
/// positions joints 1–2–3 using two-bone IK, then adjusts joint 0 to
/// orient the chain toward the target. Useful for tail/spine/finger chains.
use crate::animation::pose::{
    quat_conjugate, quat_from_axis_angle, quat_mul, quat_normalize, quat_rotate, vec3_add,
    vec3_cross, vec3_dot, vec3_len, vec3_normalize, vec3_scale, vec3_sub,
};

#[derive(Debug, Clone)]
pub struct ThreeJointParams {
    pub joint0_ws: [f32; 3],
    pub joint1_ws: [f32; 3],
    pub joint2_ws: [f32; 3],
    pub joint3_ws: [f32; 3],
    pub target_ws: [f32; 3],
    pub pole_ws: [f32; 3],
    pub gain: f32,
}

#[derive(Debug, Clone)]
pub struct ThreeJointResult {
    /// New world-space rotation for joint 0 (base).
    pub joint0_new_rot_ws: [f32; 4],
    /// New world-space rotation for joint 1 (mid1).
    pub joint1_new_rot_ws: [f32; 4],
    /// New world-space rotation for joint 2 (mid2).
    pub joint2_new_rot_ws: [f32; 4],
}

/// Solve three-joint IK.
///
/// Strategy: split the chain at joint1; treat [0,1] as the upper segment and
/// [1,2,3] as a two-bone sub-chain. Solve two-bone for [1,2,3] against the
/// target, then rotate joint0 to bring joint1 to its new position.
pub fn solve_three_joint(
    rot0_ws: &[f32; 4],
    rot1_ws: &[f32; 4],
    rot2_ws: &[f32; 4],
    params: &ThreeJointParams,
) -> ThreeJointResult {
    // Segment lengths.
    let len01 = vec3_len(&vec3_sub(&params.joint1_ws, &params.joint0_ws));
    let len12 = vec3_len(&vec3_sub(&params.joint2_ws, &params.joint1_ws));
    let len23 = vec3_len(&vec3_sub(&params.joint3_ws, &params.joint2_ws));

    // Sub-target for the inner two-bone chain [1,2,3]: place joint3 at target.
    // Solve inner chain first (joints 1,2 → end at joint3 position = target).
    let inner = solve_two_bone(
        rot1_ws,
        rot2_ws,
        &TwoBoneParams {
            root_ws: params.joint1_ws,
            mid_ws: params.joint2_ws,
            end_ws: params.joint3_ws,
            target_ws: params.target_ws,
            pole_ws: params.pole_ws,
            gain: params.gain,
        },
    );

    // Now orient joint0 so that its chain direction roughly aims toward the target.
    // We rotate joint0 around the axis cross(joint0→joint1, joint0→target).
    let dir_to_j1 = vec3_normalize(&vec3_sub(&params.joint1_ws, &params.joint0_ws));
    let dir_to_target = vec3_normalize(&vec3_sub(&params.target_ws, &params.joint0_ws));
    let axis = vec3_normalize(&vec3_cross(&dir_to_j1, &dir_to_target));
    let dot = vec3_dot(&dir_to_j1, &dir_to_target).clamp(-1.0, 1.0);
    let angle = dot.acos();

    let joint0_delta = if vec3_len(&axis) < 1e-6 || angle < 1e-6 {
        [0.0f32, 0.0, 0.0, 1.0]
    } else {
        quat_from_axis_angle(&axis, angle * params.gain)
    };
    let joint0_new_ws = quat_normalize(&quat_mul(&joint0_delta, rot0_ws));

    ThreeJointResult {
        joint0_new_rot_ws: joint0_new_ws,
        joint1_new_rot_ws: inner.root_new_rot_ws,
        joint2_new_rot_ws: inner.mid_new_rot_ws,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::pose::vec3_len;

    #[test]
    fn three_joint_solve_returns_normalized_quats() {
        let params = ThreeJointParams {
            joint0_ws: [0.0, 0.0, 0.0],
            joint1_ws: [1.0, 0.0, 0.0],
            joint2_ws: [2.0, 0.0, 0.0],
            joint3_ws: [3.0, 0.0, 0.0],
            target_ws: [2.0, 1.5, 0.0],
            pole_ws: [0.0, 1.0, 0.0],
            gain: 1.0,
        };
        let id = [0.0f32, 0.0, 0.0, 1.0];
        let result = solve_three_joint(&id, &id, &id, &params);

        let check_normalized = |q: [f32; 4]| {
            let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
            assert!(
                (len - 1.0).abs() < 1e-4,
                "quaternion not normalized: len={len}"
            );
        };
        check_normalized(result.joint0_new_rot_ws);
        check_normalized(result.joint1_new_rot_ws);
        check_normalized(result.joint2_new_rot_ws);
    }

    #[test]
    fn three_joint_solve_straight_chain_to_target() {
        // Straight chain along X, target at (1.5, 0, 0). Already aligned, near-identity result.
        let params = ThreeJointParams {
            joint0_ws: [0.0, 0.0, 0.0],
            joint1_ws: [1.0, 0.0, 0.0],
            joint2_ws: [2.0, 0.0, 0.0],
            joint3_ws: [3.0, 0.0, 0.0],
            target_ws: [3.0, 0.0, 0.0],
            pole_ws: [0.0, 1.0, 0.0],
            gain: 1.0,
        };
        let id = [0.0f32, 0.0, 0.0, 1.0];
        let result = solve_three_joint(&id, &id, &id, &params);
        // Already at target — joint0 delta should be near-identity.
        assert!((result.joint0_new_rot_ws[3] - 1.0).abs() < 0.1);
    }
}
