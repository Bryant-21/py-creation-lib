/// Two-bone analytic IK solver.
///
/// Closed-form solve for root→mid→end chains. Given root, mid, end joint
/// world positions plus a target and pole vector, computes new world
/// rotations for root and mid. Standard law-of-cosines two-bone IK.
use crate::animation::pose::{
    quat_conjugate, quat_from_axis_angle, quat_mul, quat_normalize, quat_rotate, vec3_add,
    vec3_cross, vec3_dot, vec3_len, vec3_normalize, vec3_scale, vec3_sub,
};

/// Input for the two-bone IK solver.
#[derive(Debug, Clone)]
pub struct TwoBoneParams {
    /// World-space position of the root joint.
    pub root_ws: [f32; 3],
    /// World-space position of the mid joint.
    pub mid_ws: [f32; 3],
    /// World-space position of the end joint.
    pub end_ws: [f32; 3],
    /// World-space target position.
    pub target_ws: [f32; 3],
    /// Pole vector (world space) — controls which way the mid joint bends.
    pub pole_ws: [f32; 3],
    /// Blend weight in [0, 1].
    pub gain: f32,
}

/// IK solution: new world-space rotations for root and mid joints.
#[derive(Debug, Clone)]
pub struct TwoBoneResult {
    pub root_new_rot_ws: [f32; 4],
    pub mid_new_rot_ws: [f32; 4],
}

/// Solve two-bone IK analytically.
///
/// Returns new world-space rotations for root and mid, which callers convert
/// to local space via `inv(parent_world_rot) * new_world_rot`.
pub fn solve_two_bone(
    root_rot_ws: &[f32; 4],
    mid_rot_ws: &[f32; 4],
    params: &TwoBoneParams,
) -> TwoBoneResult {
    let upper_len = vec3_len(&vec3_sub(&params.mid_ws, &params.root_ws));
    let lower_len = vec3_len(&vec3_sub(&params.end_ws, &params.mid_ws));
    let target_vec = vec3_sub(&params.target_ws, &params.root_ws);
    let target_len = vec3_len(&target_vec).max(1e-6);

    // Clamp target distance to reachable range.
    let max_reach = upper_len + lower_len;
    let min_reach = (upper_len - lower_len).abs();
    let reach = target_len
        .min(max_reach * 0.9999)
        .max((min_reach + 1e-6).min(max_reach * 0.9999));
    let target_clamped = vec3_add(
        &params.root_ws,
        &vec3_scale(&vec3_normalize(&target_vec), reach),
    );

    // Law of cosines: angle at root.
    let cos_root_angle = ((upper_len * upper_len + reach * reach - lower_len * lower_len)
        / (2.0 * upper_len * reach))
        .clamp(-1.0, 1.0);
    let root_bend_angle = cos_root_angle.acos();

    let target_dir = vec3_normalize(&target_vec);

    let pole_dir = vec3_normalize(&vec3_sub(&params.pole_ws, &params.root_ws));
    let mut bend_dir = {
        let along = vec3_scale(&target_dir, vec3_dot(&pole_dir, &target_dir));
        vec3_normalize(&vec3_sub(&pole_dir, &along))
    };
    if vec3_len(&bend_dir) < 1e-6 {
        bend_dir = perpendicular_axis(&target_dir);
    }

    let desired_mid_ws = vec3_add(
        &params.root_ws,
        &vec3_add(
            &vec3_scale(&target_dir, upper_len * root_bend_angle.cos()),
            &vec3_scale(&bend_dir, upper_len * root_bend_angle.sin()),
        ),
    );

    let current_upper_dir = vec3_normalize(&vec3_sub(&params.mid_ws, &params.root_ws));
    let desired_upper_dir = vec3_normalize(&vec3_sub(&desired_mid_ws, &params.root_ws));
    let root_delta = rotation_between(&current_upper_dir, &desired_upper_dir);
    let root_new_ws = quat_normalize(&quat_mul(&root_delta, root_rot_ws));

    let current_lower_dir = vec3_normalize(&vec3_sub(&params.end_ws, &params.mid_ws));
    let desired_lower_dir = vec3_normalize(&vec3_sub(&target_clamped, &desired_mid_ws));
    let mid_delta_ws = rotation_between(&current_lower_dir, &desired_lower_dir);
    let mid_new_ws = quat_normalize(&quat_mul(&mid_delta_ws, mid_rot_ws));

    if (params.gain - 1.0).abs() < 1e-6 {
        TwoBoneResult {
            root_new_rot_ws: root_new_ws,
            mid_new_rot_ws: mid_new_ws,
        }
    } else {
        TwoBoneResult {
            root_new_rot_ws: quat_slerp(root_rot_ws, &root_new_ws, params.gain),
            mid_new_rot_ws: quat_slerp(mid_rot_ws, &mid_new_ws, params.gain),
        }
    }
}

/// Return a quaternion that rotates `from` to `to` (both unit vectors).
fn rotation_between(from: &[f32; 3], to: &[f32; 3]) -> [f32; 4] {
    let dot = vec3_dot(from, to).clamp(-1.0, 1.0);
    if dot > 0.9999 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    if dot < -0.9999 {
        // 180°: pick an arbitrary perpendicular axis.
        let perp = perpendicular_axis(from);
        return quat_from_axis_angle(&perp, std::f32::consts::PI);
    }
    let axis = vec3_normalize(&vec3_cross(from, to));
    let angle = dot.acos();
    quat_from_axis_angle(&axis, angle)
}

fn perpendicular_axis(v: &[f32; 3]) -> [f32; 3] {
    if v[0].abs() < 0.9 {
        vec3_normalize(&vec3_cross(v, &[1.0, 0.0, 0.0]))
    } else {
        vec3_normalize(&vec3_cross(v, &[0.0, 1.0, 0.0]))
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

    fn end_pos_after_solve(params: &TwoBoneParams) -> [f32; 3] {
        // After solving, simulate where the end ends up given the new root+mid rotations.
        // We just test that the end moves toward the target.
        let result = solve_two_bone(&[0.0, 0.0, 0.0, 1.0], &[0.0, 0.0, 0.0, 1.0], params);
        // Apply root rotation to upper bone direction, then mid rotation to lower.
        let upper = vec3_sub(&params.mid_ws, &params.root_ws);
        let upper_len = vec3_len(&upper);
        let upper_dir_orig = vec3_normalize(&upper);
        let upper_dir_new = quat_rotate(&result.root_new_rot_ws, &upper_dir_orig);
        let mid_new = vec3_add(&params.root_ws, &vec3_scale(&upper_dir_new, upper_len));

        let lower = vec3_sub(&params.end_ws, &params.mid_ws);
        let lower_len = vec3_len(&lower);
        let lower_dir_orig = vec3_normalize(&lower);
        let lower_dir_new = quat_rotate(&result.mid_new_rot_ws, &lower_dir_orig);
        vec3_add(&mid_new, &vec3_scale(&lower_dir_new, lower_len))
    }

    #[test]
    fn two_bone_ik_reaches_toward_target() {
        // Straight chain along X: root(0,0,0), mid(1,0,0), end(2,0,0).
        // Target at (1, 1, 0) — should bend mid upward.
        let params = TwoBoneParams {
            root_ws: [0.0, 0.0, 0.0],
            mid_ws: [1.0, 0.0, 0.0],
            end_ws: [2.0, 0.0, 0.0],
            target_ws: [1.0, 1.0, 0.0],
            pole_ws: [0.0, 1.0, 0.0],
            gain: 1.0,
        };
        let end = end_pos_after_solve(&params);
        let dist_to_target = vec3_len(&vec3_sub(&end, &params.target_ws));
        // End should move substantially closer to target (within ~10% of chain length).
        assert!(
            dist_to_target < 0.5,
            "end={end:?} dist_to_target={dist_to_target}"
        );
    }

    #[test]
    fn two_bone_ik_90_degree_bend() {
        // Target places end at 90° bend: root at origin, target at (1,1,0), upper len=1, lower len=1.
        let params = TwoBoneParams {
            root_ws: [0.0, 0.0, 0.0],
            mid_ws: [0.0, 1.0, 0.0],
            end_ws: [0.0, 2.0, 0.0],
            target_ws: [1.0, 1.0, 0.0],
            pole_ws: [0.0, 0.0, 1.0],
            gain: 1.0,
        };
        let result = solve_two_bone(&[0.0, 0.0, 0.0, 1.0], &[0.0, 0.0, 0.0, 1.0], &params);
        // Result should give valid quaternions (normalized).
        let r_len = {
            let r = result.root_new_rot_ws;
            (r[0] * r[0] + r[1] * r[1] + r[2] * r[2] + r[3] * r[3]).sqrt()
        };
        assert!((r_len - 1.0).abs() < 1e-4);
    }
}
