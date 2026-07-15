use super::two_bone::{TwoBoneParams, solve_two_bone};
/// Foot-placement IK solver.
///
/// Combines two-bone IK (hip→knee→ankle) with ground projection and
/// ankle-orient alignment. Used to drop feet onto uneven ground for outfit
/// previews and armor authoring.
use crate::animation::pose::{
    quat_from_axis_angle, quat_mul, quat_normalize, quat_rotate, vec3_add, vec3_cross, vec3_dot,
    vec3_len, vec3_normalize, vec3_scale, vec3_sub,
};

/// A simple ground plane defined by a point and normal.
#[derive(Debug, Clone)]
pub struct GroundPlane {
    /// A point on the plane.
    pub point: [f32; 3],
    /// Unit normal (pointing up, e.g. [0,0,1]).
    pub normal: [f32; 3],
}

impl GroundPlane {
    /// Project a point onto the plane along the normal direction.
    pub fn project(&self, pos: &[f32; 3]) -> [f32; 3] {
        let to_point = vec3_sub(pos, &self.point);
        let dist = vec3_dot(&to_point, &self.normal);
        vec3_sub(pos, &vec3_scale(&self.normal, dist))
    }

    /// Return the height (signed distance along normal) from the plane.
    pub fn height_of(&self, pos: &[f32; 3]) -> f32 {
        vec3_dot(&vec3_sub(pos, &self.point), &self.normal)
    }
}

/// Parameters for foot-placement IK.
#[derive(Debug, Clone)]
pub struct FootPlacementParams {
    /// World-space position of the hip joint.
    pub hip_ws: [f32; 3],
    /// World-space position of the knee joint.
    pub knee_ws: [f32; 3],
    /// World-space position of the ankle joint.
    pub ankle_ws: [f32; 3],
    /// World-space current rotation of the hip joint.
    pub hip_rot_ws: [f32; 4],
    /// World-space current rotation of the knee joint.
    pub knee_rot_ws: [f32; 4],
    /// Ground geometry for ankle projection.
    pub ground: GroundPlane,
    /// Maximum distance to lift ankle above its current height.
    pub max_lift_offset: f32,
    /// Maximum distance to drop ankle below its current height.
    pub max_drop_offset: f32,
    /// Blend weight [0,1] for ankle-orient alignment to ground normal.
    pub ankle_orient_blend: f32,
}

/// Result of the foot-placement solve.
#[derive(Debug, Clone)]
pub struct FootPlacementResult {
    pub hip_new_rot_ws: [f32; 4],
    pub knee_new_rot_ws: [f32; 4],
    /// New world-space rotation for the ankle (oriented to match the ground normal).
    pub ankle_new_rot_ws: [f32; 4],
}

/// Solve foot-placement IK.
pub fn solve_foot_placement(params: &FootPlacementParams) -> FootPlacementResult {
    // Project ankle onto the ground.
    let ankle_ground = params.ground.project(&params.ankle_ws);

    // Clamp vertical offset.
    let current_h = params.ground.height_of(&params.ankle_ws);
    let drop = (current_h - 0.0_f32).min(params.max_drop_offset).max(0.0);
    let lift = (0.0_f32 - current_h).min(params.max_lift_offset).max(0.0);
    let target_ankle = vec3_add(
        &ankle_ground,
        &vec3_scale(&params.ground.normal, lift - drop),
    );

    // Use two-bone IK to place the ankle at target_ankle.
    let pole = vec3_add(&params.knee_ws, &params.ground.normal);
    let tb = solve_two_bone(
        &params.hip_rot_ws,
        &params.knee_rot_ws,
        &TwoBoneParams {
            root_ws: params.hip_ws,
            mid_ws: params.knee_ws,
            end_ws: params.ankle_ws,
            target_ws: target_ankle,
            pole_ws: pole,
            gain: 1.0,
        },
    );

    // Orient ankle to align foot sole with ground normal.
    // The foot sole forward is assumed to be the X axis in world space.
    // We rotate the ankle so that its -Z (down) axis aligns with -ground.normal.
    let foot_down_ls = [0.0f32, 0.0, -1.0];
    let foot_down_ws = quat_rotate(&params.knee_rot_ws, &foot_down_ls);
    let desired_down = vec3_scale(&params.ground.normal, -1.0);
    let ankle_delta = rotation_between(&foot_down_ws, &desired_down);

    let ankle_blend = if (params.ankle_orient_blend - 1.0).abs() < 1e-6 {
        ankle_delta
    } else {
        quat_slerp(
            &[0.0, 0.0, 0.0, 1.0],
            &ankle_delta,
            params.ankle_orient_blend,
        )
    };
    let ankle_new_rot = quat_normalize(&quat_mul(&ankle_blend, &params.knee_rot_ws));

    FootPlacementResult {
        hip_new_rot_ws: tb.root_new_rot_ws,
        knee_new_rot_ws: tb.mid_new_rot_ws,
        ankle_new_rot_ws: ankle_new_rot,
    }
}

fn rotation_between(from: &[f32; 3], to: &[f32; 3]) -> [f32; 4] {
    let dot = vec3_dot(from, to).clamp(-1.0, 1.0);
    if dot > 0.9999 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    if dot < -0.9999 {
        let ax = if from[0].abs() < 0.9 {
            [1.0f32, 0.0, 0.0]
        } else {
            [0.0f32, 1.0, 0.0]
        };
        let perp = vec3_normalize(&vec3_cross(from, &ax));
        return quat_from_axis_angle(&perp, std::f32::consts::PI);
    }
    let axis = vec3_normalize(&vec3_cross(from, to));
    quat_from_axis_angle(&axis, dot.acos())
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

    #[test]
    fn foot_placement_flat_ground_noop() {
        // Ankle already on the flat ground plane (Z=0), no movement needed.
        let params = FootPlacementParams {
            hip_ws: [0.0, 0.0, 1.0],
            knee_ws: [0.0, 0.0, 0.5],
            ankle_ws: [0.0, 0.0, 0.0],
            hip_rot_ws: [0.0, 0.0, 0.0, 1.0],
            knee_rot_ws: [0.0, 0.0, 0.0, 1.0],
            ground: GroundPlane {
                point: [0.0, 0.0, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
            max_lift_offset: 0.5,
            max_drop_offset: 0.5,
            ankle_orient_blend: 0.0,
        };
        let result = solve_foot_placement(&params);
        // Result should still be normalized quaternions.
        let len = |q: [f32; 4]| (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        assert!((len(result.hip_new_rot_ws) - 1.0).abs() < 1e-4);
        assert!((len(result.knee_new_rot_ws) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn foot_placement_stepped_ground() {
        // Ankle above a step: ground at Z=0.5 (step up), ankle at Z=0.
        // Should try to drop the ankle onto the ground.
        let params = FootPlacementParams {
            hip_ws: [0.0, 0.0, 2.0],
            knee_ws: [0.0, 0.0, 1.0],
            ankle_ws: [0.0, 0.0, 0.0],
            hip_rot_ws: [0.0, 0.0, 0.0, 1.0],
            knee_rot_ws: [0.0, 0.0, 0.0, 1.0],
            ground: GroundPlane {
                point: [0.0, 0.0, 0.5],
                normal: [0.0, 0.0, 1.0],
            },
            max_lift_offset: 0.0,
            max_drop_offset: 1.0,
            ankle_orient_blend: 0.0,
        };
        let result = solve_foot_placement(&params);
        let len = |q: [f32; 4]| (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        assert!((len(result.hip_new_rot_ws) - 1.0).abs() < 1e-4);
    }
}
