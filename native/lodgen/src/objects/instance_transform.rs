//! FO76 `BSDistantObjectInstancedNode` Matrix44 -> FO4 instance (position, uniform scale, quaternion).
//! Same convention fo76_bto.rs bakes merged geometry with: column vectors (v' = R v), translation in
//! `m[i][3]`, and uniform scale in `m[3][3]` times any scale carried by the 3x3.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InstanceTransform {
    /// `rot` is x, y, z, w (LOD_RUNTIME.md Q8).
    Uniform { pos: [f32; 3], scale: f32, rot: [f32; 4] },
    /// Non-uniform scale, shear or a mirror: the FO4 instance format has one scale and a rotation, so these
    /// stay merged geometry.
    NonUniform { matrix: [[f32; 4]; 4] },
    /// Zero or non-finite scale, or non-finite values anywhere: skipped.
    Degenerate,
}

const UNIFORM_TOLERANCE: f32 = 1e-3;

pub fn decompose(matrix: &[[f32; 4]; 4], node_translation: [f32; 3]) -> InstanceTransform {
    if matrix.iter().flatten().any(|v| !v.is_finite()) {
        return InstanceTransform::Degenerate;
    }
    let col = |c: usize| (matrix[0][c].powi(2) + matrix[1][c].powi(2) + matrix[2][c].powi(2)).sqrt();
    let norms = [col(0), col(1), col(2)];
    let w = matrix[3][3].abs();
    if norms.iter().any(|n| *n < 1e-6) || w < 1e-6 {
        return InstanceTransform::Degenerate;
    }
    let mean = (norms[0] + norms[1] + norms[2]) / 3.0;
    if norms.iter().any(|n| (n - mean).abs() > UNIFORM_TOLERANCE * mean) {
        return InstanceTransform::NonUniform { matrix: *matrix };
    }
    let mut r = [[0.0f32; 3]; 3];
    for (row, out) in r.iter_mut().enumerate() {
        for (c, value) in out.iter_mut().enumerate() {
            *value = matrix[row][c] / norms[c];
        }
    }
    let dot = |a: usize, b: usize| (0..3).map(|i| r[i][a] * r[i][b]).sum::<f32>();
    let det = r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1]) - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
        + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0]);
    if [(0, 1), (0, 2), (1, 2)].iter().any(|&(a, b)| dot(a, b).abs() > UNIFORM_TOLERANCE) || det <= 0.0 {
        return InstanceTransform::NonUniform { matrix: *matrix };
    }
    let pos = [
        matrix[0][3] + node_translation[0],
        matrix[1][3] + node_translation[1],
        matrix[2][3] + node_translation[2],
    ];
    InstanceTransform::Uniform { pos, scale: mean * w, rot: quat_from_rotation(&r) }
}

/// Shepperd's method on a column-vector rotation; returns x, y, z, w.
pub fn quat_from_rotation(r: &[[f32; 3]; 3]) -> [f32; 4] {
    let trace = r[0][0] + r[1][1] + r[2][2];
    let [w, x, y, z] = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [0.25 * s, (r[2][1] - r[1][2]) / s, (r[0][2] - r[2][0]) / s, (r[1][0] - r[0][1]) / s]
    } else if r[0][0] > r[1][1] && r[0][0] > r[2][2] {
        let s = (1.0 + r[0][0] - r[1][1] - r[2][2]).sqrt() * 2.0;
        [(r[2][1] - r[1][2]) / s, 0.25 * s, (r[0][1] + r[1][0]) / s, (r[0][2] + r[2][0]) / s]
    } else if r[1][1] > r[2][2] {
        let s = (1.0 + r[1][1] - r[0][0] - r[2][2]).sqrt() * 2.0;
        [(r[0][2] - r[2][0]) / s, (r[0][1] + r[1][0]) / s, 0.25 * s, (r[1][2] + r[2][1]) / s]
    } else {
        let s = (1.0 + r[2][2] - r[0][0] - r[1][1]).sqrt() * 2.0;
        [(r[1][0] - r[0][1]) / s, (r[0][2] + r[2][0]) / s, (r[1][2] + r[2][1]) / s, 0.25 * s]
    };
    let n = (w * w + x * x + y * y + z * z).sqrt();
    [x / n, y / n, z / n, w / n]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(rot: [[f32; 3]; 3], scale: [f32; 3], t: [f32; 3], w: f32) -> [[f32; 4]; 4] {
        let mut out = [[0.0; 4]; 4];
        for r in 0..3 {
            for c in 0..3 {
                out[r][c] = rot[r][c] * scale[c];
            }
            out[r][3] = t[r];
        }
        out[3][3] = w;
        out
    }

    const I3: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    #[test]
    fn identity_is_uniform_unit() {
        match decompose(&m(I3, [1.0; 3], [10.0, 20.0, 30.0], 1.0), [0.0; 3]) {
            InstanceTransform::Uniform { pos, scale, rot } => {
                assert_eq!(pos, [10.0, 20.0, 30.0]);
                assert!((scale - 1.0).abs() < 1e-5);
                assert_eq!(rot, [0.0, 0.0, 0.0, 1.0]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn node_translation_is_added() {
        match decompose(&m(I3, [1.0; 3], [1.0, 2.0, 3.0], 1.0), [100.0, 200.0, 300.0]) {
            InstanceTransform::Uniform { pos, .. } => assert_eq!(pos, [101.0, 202.0, 303.0]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn scale_in_the_3x3_is_found() {
        match decompose(&m(I3, [2.0; 3], [0.0; 3], 1.0), [0.0; 3]) {
            InstanceTransform::Uniform { scale, .. } => assert!((scale - 2.0).abs() < 1e-5),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn legacy_w_scale_is_respected() {
        // fo76_bto.rs bakes merged geometry with uniform scale from m[3][3] and an orthonormal 3x3.
        match decompose(&m(I3, [1.0; 3], [0.0; 3], 3.0), [0.0; 3]) {
            InstanceTransform::Uniform { scale, .. } => assert!((scale - 3.0).abs() < 1e-5),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn non_uniform_scale_stays_merged() {
        assert!(matches!(decompose(&m(I3, [1.0, 1.0, 2.0], [0.0; 3], 1.0), [0.0; 3]), InstanceTransform::NonUniform { .. }));
    }

    #[test]
    fn shear_with_equal_column_norms_stays_merged() {
        // Unit columns, but the second leans 30 degrees toward the first: no rotation can represent it.
        let (s, c) = (0.5f32, 0.75f32.sqrt());
        let sheared = [[1.0, s, 0.0], [0.0, c, 0.0], [0.0, 0.0, 1.0]];
        assert!(matches!(decompose(&m(sheared, [2.0; 3], [0.0; 3], 1.0), [0.0; 3]), InstanceTransform::NonUniform { .. }));
    }

    #[test]
    fn mirrored_matrix_stays_merged() {
        let mirrored = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]];
        assert!(matches!(decompose(&m(mirrored, [1.0; 3], [0.0; 3], 1.0), [0.0; 3]), InstanceTransform::NonUniform { .. }));
    }

    #[test]
    fn rotated_uniform_scale_is_still_uniform() {
        let r = [[0.36, 0.48, -0.8], [-0.8, 0.6, 0.0], [0.48, 0.64, 0.6]];
        assert!(matches!(decompose(&m(r, [1.5; 3], [0.0; 3], 1.0), [0.0; 3]), InstanceTransform::Uniform { .. }));
    }

    #[test]
    fn degenerate_inputs_are_rejected() {
        assert!(matches!(decompose(&m(I3, [0.0; 3], [0.0; 3], 1.0), [0.0; 3]), InstanceTransform::Degenerate));
        assert!(matches!(decompose(&m(I3, [f32::NAN; 3], [0.0; 3], 1.0), [0.0; 3]), InstanceTransform::Degenerate));
        assert!(matches!(decompose(&m(I3, [1.0; 3], [0.0; 3], 0.0), [0.0; 3]), InstanceTransform::Degenerate));
    }

    #[test]
    fn quaternion_of_90_degrees_about_z() {
        // Column vectors, as fo76_bto bakes them (v' = R v): x maps to y.
        let r = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let q = quat_from_rotation(&r);
        let s = std::f32::consts::FRAC_1_SQRT_2;
        // x, y, z, w (LOD_RUNTIME.md Q8).
        assert!(q[0].abs() < 1e-5 && q[1].abs() < 1e-5 && (q[2] - s).abs() < 1e-5 && (q[3] - s).abs() < 1e-5, "{q:?}");
    }

    #[test]
    fn quaternion_rotates_like_the_instance_shader() {
        // Q8's vertex shader: t = 2 (q.xyz x v), v' = v + q.w t + q.xyz x t. It must match R v.
        let r = [[0.36, 0.48, -0.8], [-0.8, 0.6, 0.0], [0.48, 0.64, 0.6]];
        let q = quat_from_rotation(&r);
        let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
        let v = [1.0, 2.0, 3.0];
        let qv = [q[0], q[1], q[2]];
        let t = cross(qv, v).map(|c| 2.0 * c);
        let u = cross(qv, t);
        let shader = [v[0] + q[3] * t[0] + u[0], v[1] + q[3] * t[1] + u[1], v[2] + q[3] * t[2] + u[2]];
        for i in 0..3 {
            let expected = r[i][0] * v[0] + r[i][1] * v[1] + r[i][2] * v[2];
            assert!((shader[i] - expected).abs() < 1e-4, "{shader:?} vs R v");
        }
    }
}
