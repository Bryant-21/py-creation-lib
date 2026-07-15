/// Vertex normal calculation from triangle meshes.
///
/// Computes smooth per-vertex normals by area-weighting the face normals of
/// each incident triangle, then normalizing.  Degenerate triangles (zero-area)
/// contribute zero weight and are silently skipped.

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compute smooth per-vertex normals for a triangle mesh.
///
/// Returns one `[f32; 3]` per vertex.  Isolated vertices (no incident triangle)
/// get normal `[0.0, 0.0, 1.0]` as a safe default.
pub fn compute_normals(vertices: &[[f32; 3]], triangles: &[[u32; 3]]) -> Vec<[f32; 3]> {
    let n = vertices.len();
    let mut accum = vec![[0.0f32; 3]; n];

    for tri in triangles {
        let [ai, bi, ci] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        if ai >= n || bi >= n || ci >= n {
            continue;
        }
        let a = vertices[ai];
        let b = vertices[bi];
        let c = vertices[ci];
        // Cross product gives area-weighted normal
        let ab = sub(b, a);
        let ac = sub(c, a);
        let wn = cross(ab, ac);
        accum[ai] = add(accum[ai], wn);
        accum[bi] = add(accum[bi], wn);
        accum[ci] = add(accum[ci], wn);
    }

    accum
        .iter()
        .map(|&n| {
            let len = length(n);
            if len > 1e-10 {
                scale(n, 1.0 / len)
            } else {
                [0.0, 0.0, 1.0]
            }
        })
        .collect()
}

/// Compute flat (per-face) normals, returning one normal per triangle.
pub fn compute_flat_normals(vertices: &[[f32; 3]], triangles: &[[u32; 3]]) -> Vec<[f32; 3]> {
    let n = vertices.len();
    triangles
        .iter()
        .map(|tri| {
            let [ai, bi, ci] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
            if ai >= n || bi >= n || ci >= n {
                return [0.0, 0.0, 1.0];
            }
            let ab = sub(vertices[bi], vertices[ai]);
            let ac = sub(vertices[ci], vertices[ai]);
            let wn = cross(ab, ac);
            let len = length(wn);
            if len > 1e-10 {
                scale(wn, 1.0 / len)
            } else {
                [0.0, 0.0, 1.0]
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Vec3 helpers
// ---------------------------------------------------------------------------

#[inline]
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
#[inline]
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
#[inline]
fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_top_face_normal_points_up() {
        // Single quad (2 triangles) on the XY plane at Z=1
        let verts = vec![
            [0.0f32, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ];
        let tris = vec![[0u32, 1, 2], [0, 2, 3]];
        let normals = compute_normals(&verts, &tris);
        for n in &normals {
            assert!(
                (n[2] - 1.0).abs() < 1e-5,
                "Z component must be ~1.0, got {:?}",
                n
            );
            assert!(n[0].abs() < 1e-5);
            assert!(n[1].abs() < 1e-5);
        }
    }

    #[test]
    fn isolated_vertex_gets_default_normal() {
        let verts = vec![[0.0f32, 0.0, 0.0]];
        let tris: Vec<[u32; 3]> = vec![];
        let normals = compute_normals(&verts, &tris);
        assert_eq!(normals[0], [0.0, 0.0, 1.0]);
    }

    #[test]
    fn flat_normal_matches_z_up_plane() {
        let verts = vec![[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let tris = vec![[0u32, 1, 2]];
        let normals = compute_flat_normals(&verts, &tris);
        assert_eq!(normals.len(), 1);
        assert!((normals[0][2] - 1.0).abs() < 1e-5, "{:?}", normals[0]);
    }
}
