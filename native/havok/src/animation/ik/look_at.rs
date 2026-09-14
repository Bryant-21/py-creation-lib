/// Analytic single-bone look-at IK (`hkaLookAtIkSolver` semantics): turns a
/// head/eye bone's forward axis toward a world-space target, keeping the up axis upright.
use crate::animation::pose::{
    QsTransform, quat_conjugate, quat_from_axis_angle, quat_mul, quat_normalize, quat_rotate,
    vec3_cross, vec3_dot, vec3_len, vec3_normalize, vec3_sub,
};

/// Parameters for the look-at solver.
#[derive(Debug, Clone)]
pub struct LookAtParams {
    /// World-space position of the head bone (translation of its model-space transform).
    pub head_world_pos: [f32; 3],
    /// World-space orientation of the head bone (rotation of its model-space transform).
    pub head_world_rot: [f32; 4],
    /// Forward axis in local bone space (e.g. [1,0,0] or [0,1,0]).
    pub forward_ls: [f32; 3],
    /// Up axis in local bone space (e.g. [0,0,1]).
    pub up_ls: [f32; 3],
    /// World-space target position to look at.
    pub target_ws: [f32; 3],
    /// Blend weight in [0, 1].
    pub gain: f32,
}

/// Result of the look-at solve.
#[derive(Debug, Clone)]
pub struct LookAtResult {
    /// New local-space rotation for the head bone (replace existing local rotation).
    pub new_local_rotation: [f32; 4],
}

/// Solve a look-at rotation for a single bone.
///
/// The solver computes a rotation (in the bone's local space) that aligns
/// `forward_ls` toward the target, then blends by `gain`.
pub fn solve_look_at(
    parent_world_rot: &[f32; 4],
    local_rot: &[f32; 4],
    params: &LookAtParams,
) -> LookAtResult {
    let [tx, ty, tz] = params.target_ws;
    let [hx, hy, hz] = params.head_world_pos;
    let to_target = [tx - hx, ty - hy, tz - hz];

    let desired_forward_ws = vec3_normalize(&to_target);

    // Current forward in world space.
    let current_forward_ws =
        vec3_normalize(&quat_rotate(&params.head_world_rot, &params.forward_ls));

    // Rotation axis = cross(current_forward, desired_forward).
    let axis = vec3_cross(&current_forward_ws, &desired_forward_ws);
    let sin_angle = vec3_len(&axis);
    let cos_angle = vec3_dot(&current_forward_ws, &desired_forward_ws).clamp(-1.0, 1.0);
    let angle = cos_angle.acos();

    let delta_ws = if sin_angle < 1e-6 {
        // Already aligned or 180°.
        if cos_angle > 0.0 {
            [0.0, 0.0, 0.0, 1.0f32]
        } else {
            // 180° flip around the up axis.
            let up_ws = quat_rotate(&params.head_world_rot, &params.up_ls);
            quat_from_axis_angle(&up_ws, std::f32::consts::PI)
        }
    } else {
        quat_from_axis_angle(&vec3_normalize(&axis), angle)
    };

    // Apply delta in world space: new_world = delta_ws * head_world_rot.
    let new_world_rot = quat_normalize(&quat_mul(&delta_ws, &params.head_world_rot));

    // Convert back to local space: new_local = inv(parent_world) * new_world.
    let inv_parent = quat_conjugate(parent_world_rot);
    let new_local_full = quat_normalize(&quat_mul(&inv_parent, &new_world_rot));

    // Blend with existing local rotation.
    let blended = if (params.gain - 1.0).abs() < 1e-6 {
        new_local_full
    } else {
        quat_slerp(local_rot, &new_local_full, params.gain)
    };

    LookAtResult {
        new_local_rotation: quat_normalize(&blended),
    }
}

/// Spherical linear interpolation between two unit quaternions.
fn quat_slerp(a: &[f32; 4], b: &[f32; 4], t: f32) -> [f32; 4] {
    let mut dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    // Ensure shortest path.
    let b_adj = if dot < 0.0 {
        dot = -dot;
        [-b[0], -b[1], -b[2], -b[3]]
    } else {
        *b
    };
    if dot > 0.9995 {
        // Linear blend for nearly identical quaternions.
        let r = [
            a[0] + t * (b_adj[0] - a[0]),
            a[1] + t * (b_adj[1] - a[1]),
            a[2] + t * (b_adj[2] - a[2]),
            a[3] + t * (b_adj[3] - a[3]),
        ];
        return quat_normalize(&r);
    }
    let theta_0 = dot.acos();
    let theta = theta_0 * t;
    let sin_theta = theta.sin();
    let sin_theta_0 = theta_0.sin();
    let s0 = (theta_0 - theta).cos() - dot * sin_theta / sin_theta_0;
    let s1 = sin_theta / sin_theta_0;
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
    use std::f32::consts::FRAC_PI_4;

    #[test]
    fn look_at_45_deg_yaw() {
        // Head at origin, facing +X ([1,0,0] forward in local = world since rot = identity).
        // Target at (1, 1, 0) — should produce ~45° yaw (rotation around Z).
        let params = LookAtParams {
            head_world_pos: [0.0, 0.0, 0.0],
            head_world_rot: [0.0, 0.0, 0.0, 1.0],
            forward_ls: [1.0, 0.0, 0.0],
            up_ls: [0.0, 0.0, 1.0],
            target_ws: [1.0, 1.0, 0.0],
            gain: 1.0,
        };
        let result = solve_look_at(&[0.0, 0.0, 0.0, 1.0f32], &[0.0, 0.0, 0.0, 1.0f32], &params);
        // Rotate the result back to see where forward now points.
        let new_forward = quat_rotate(&result.new_local_rotation, &[1.0, 0.0, 0.0]);
        let expected = vec3_normalize(&[1.0, 1.0, 0.0]);
        let diff = vec3_dot(&new_forward, &expected);
        assert!(diff > 0.999, "expected forward ~45°, got dot={diff}");
    }

    #[test]
    fn look_at_already_aligned_noop() {
        let params = LookAtParams {
            head_world_pos: [0.0, 0.0, 0.0],
            head_world_rot: [0.0, 0.0, 0.0, 1.0],
            forward_ls: [1.0, 0.0, 0.0],
            up_ls: [0.0, 0.0, 1.0],
            target_ws: [5.0, 0.0, 0.0],
            gain: 1.0,
        };
        let result = solve_look_at(&[0.0, 0.0, 0.0, 1.0f32], &[0.0, 0.0, 0.0, 1.0f32], &params);
        // Already aligned; rotation should be near identity.
        assert!((result.new_local_rotation[3] - 1.0).abs() < 1e-4);
    }
}
