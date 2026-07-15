/// Skinning utilities: bind-pose baking and linear-blend (LBS) deformation.
///
/// Linear-blend skinning applies a weighted sum of bone transforms to each
/// vertex.  Only position deformation is implemented here; normal deformation
/// uses the inverse-transpose of the same blended matrix.

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A 4×4 column-major transform matrix stored as `[col0, col1, col2, col3]`.
pub type Mat4 = [[f32; 4]; 4];

/// Skinning weight: (bone_index, weight) pair.
/// Weights for a vertex must sum to 1.0 for correct LBS.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinWeight {
    pub bone: u16,
    pub weight: f32,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Apply linear-blend skinning to a set of vertices.
///
/// `bind_pose_inv` — one inverse-bind-pose matrix per bone.
/// `bone_pose`     — one current-pose matrix per bone (world space).
/// `weights`       — one `Vec<SkinWeight>` per vertex (may be empty → identity).
///
/// Returns deformed positions.  Vertices with empty weight lists are left
/// at their original bind-space position.
pub fn apply_lbs(
    vertices: &[[f32; 3]],
    bind_pose_inv: &[Mat4],
    bone_pose: &[Mat4],
    weights: &[Vec<SkinWeight>],
) -> Vec<[f32; 3]> {
    assert_eq!(
        vertices.len(),
        weights.len(),
        "weights length must equal vertex count"
    );

    vertices
        .iter()
        .zip(weights.iter())
        .map(|(&v, ws)| {
            if ws.is_empty() {
                return v;
            }
            let mut out = [0.0f32; 3];
            for sw in ws {
                let bi = sw.bone as usize;
                if bi >= bind_pose_inv.len() || bi >= bone_pose.len() {
                    continue;
                }
                // Skinning matrix = bone_pose * bind_pose_inv
                let sm = mat4_mul(bone_pose[bi], bind_pose_inv[bi]);
                let tv = transform_point(sm, v);
                out[0] += tv[0] * sw.weight;
                out[1] += tv[1] * sw.weight;
                out[2] += tv[2] * sw.weight;
            }
            out
        })
        .collect()
}

/// Normalize per-vertex skin weights so they sum to 1.0.
///
/// Entries with zero weight are removed.  If all weights are zero for a vertex
/// the list is left empty (the vertex will be treated as unskinned by `apply_lbs`).
pub fn normalize_weights(weights: &mut Vec<SkinWeight>) {
    weights.retain(|sw| sw.weight > 0.0);
    let total: f32 = weights.iter().map(|sw| sw.weight).sum();
    if total > 1e-10 {
        for sw in weights.iter_mut() {
            sw.weight /= total;
        }
    }
}

// ---------------------------------------------------------------------------
// Mat4 helpers
// ---------------------------------------------------------------------------

pub fn mat4_identity() -> Mat4 {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Column-major 4×4 matrix multiply: result = a * b.
pub fn mat4_mul(a: Mat4, b: Mat4) -> Mat4 {
    let mut out = [[0.0f32; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            let mut sum = 0.0f32;
            for k in 0..4 {
                sum += a[k][row] * b[col][k];
            }
            out[col][row] = sum;
        }
    }
    out
}

/// Transform a 3D point by a 4×4 column-major matrix (w-divide applied).
pub fn transform_point(m: Mat4, p: [f32; 3]) -> [f32; 3] {
    let x = m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0];
    let y = m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1];
    let z = m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2];
    let w = m[0][3] * p[0] + m[1][3] * p[1] + m[2][3] * p[2] + m[3][3];
    if w.abs() > 1e-10 {
        [x / w, y / w, z / w]
    } else {
        [x, y, z]
    }
}

/// Build an inverse bind-pose matrix from a bind-pose matrix via
/// cofactor expansion (no LAPACK dependency).
///
/// Only valid for affine (non-shearing, non-projective) transforms.
/// Returns `None` if the matrix is singular (det ≈ 0).
pub fn mat4_inverse_affine(m: Mat4) -> Option<Mat4> {
    // Extract 3×3 rotation/scale block and translation
    let (r, t) = mat4_decompose_affine(m);
    // Invert 3×3 via adjugate
    let det = r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1])
        - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
        + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0]);
    if det.abs() < 1e-10 {
        return None;
    }
    let inv_det = 1.0 / det;
    let ri = [
        [
            (r[1][1] * r[2][2] - r[1][2] * r[2][1]) * inv_det,
            -(r[0][1] * r[2][2] - r[0][2] * r[2][1]) * inv_det,
            (r[0][1] * r[1][2] - r[0][2] * r[1][1]) * inv_det,
        ],
        [
            -(r[1][0] * r[2][2] - r[1][2] * r[2][0]) * inv_det,
            (r[0][0] * r[2][2] - r[0][2] * r[2][0]) * inv_det,
            -(r[0][0] * r[1][2] - r[0][2] * r[1][0]) * inv_det,
        ],
        [
            (r[1][0] * r[2][1] - r[1][1] * r[2][0]) * inv_det,
            -(r[0][0] * r[2][1] - r[0][1] * r[2][0]) * inv_det,
            (r[0][0] * r[1][1] - r[0][1] * r[1][0]) * inv_det,
        ],
    ];
    // inv_t = -R_inv * t
    let ti = [
        -(ri[0][0] * t[0] + ri[1][0] * t[1] + ri[2][0] * t[2]),
        -(ri[0][1] * t[0] + ri[1][1] * t[1] + ri[2][1] * t[2]),
        -(ri[0][2] * t[0] + ri[1][2] * t[1] + ri[2][2] * t[2]),
    ];
    Some([
        [ri[0][0], ri[0][1], ri[0][2], 0.0],
        [ri[1][0], ri[1][1], ri[1][2], 0.0],
        [ri[2][0], ri[2][1], ri[2][2], 0.0],
        [ti[0], ti[1], ti[2], 1.0],
    ])
}

fn mat4_decompose_affine(m: Mat4) -> ([[f32; 3]; 3], [f32; 3]) {
    let r = [
        [m[0][0], m[1][0], m[2][0]],
        [m[0][1], m[1][1], m[2][1]],
        [m[0][2], m[1][2], m[2][2]],
    ];
    let t = [m[3][0], m[3][1], m[3][2]];
    (r, t)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lbs_identity_bone_leaves_vertex_unchanged() {
        let id = mat4_identity();
        let verts = vec![[1.0f32, 2.0, 3.0]];
        let weights = vec![vec![SkinWeight {
            bone: 0,
            weight: 1.0,
        }]];
        let out = apply_lbs(&verts, &[id], &[id], &weights);
        assert!((out[0][0] - 1.0).abs() < 1e-5);
        assert!((out[0][1] - 2.0).abs() < 1e-5);
        assert!((out[0][2] - 3.0).abs() < 1e-5);
    }

    #[test]
    fn lbs_translation_applied() {
        // bind: identity; current: translate +5 on X
        let bind_inv = mat4_identity();
        let mut pose = mat4_identity();
        pose[3][0] = 5.0; // column 3, row 0 = translation X
        let verts = vec![[0.0f32, 0.0, 0.0]];
        let weights = vec![vec![SkinWeight {
            bone: 0,
            weight: 1.0,
        }]];
        let out = apply_lbs(&verts, &[bind_inv], &[pose], &weights);
        assert!(
            (out[0][0] - 5.0).abs() < 1e-5,
            "X should be 5, got {}",
            out[0][0]
        );
    }

    #[test]
    fn normalize_weights_sums_to_one() {
        let mut ws = vec![
            SkinWeight {
                bone: 0,
                weight: 2.0,
            },
            SkinWeight {
                bone: 1,
                weight: 2.0,
            },
        ];
        normalize_weights(&mut ws);
        let total: f32 = ws.iter().map(|w| w.weight).sum();
        assert!((total - 1.0).abs() < 1e-6);
    }

    #[test]
    fn mat4_inverse_identity() {
        let id = mat4_identity();
        let inv = mat4_inverse_affine(id).expect("identity is invertible");
        // Check inv ≈ identity
        for col in 0..4 {
            for row in 0..4 {
                let expected = if row == col { 1.0 } else { 0.0 };
                assert!(
                    (inv[col][row] - expected).abs() < 1e-5,
                    "[{col}][{row}] expected {expected}, got {}",
                    inv[col][row]
                );
            }
        }
    }
}
