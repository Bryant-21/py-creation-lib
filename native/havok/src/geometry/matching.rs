/// Geometry-matching utility: align two meshes and find vertex correspondences.
///
/// Useful for matching LOD meshes to their source, or for finding morphs between
/// otherwise-unrelated meshes that represent the same shape at different resolutions.
use crate::geometry::weight_transfer::KdTree;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of matching a target mesh against a source mesh.
#[derive(Debug, Clone)]
pub struct MatchResult {
    /// For each target vertex, the index of the nearest source vertex.
    pub nearest: Vec<usize>,
    /// For each target vertex, the distance to its nearest source vertex.
    pub distances: Vec<f32>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Match each target vertex to the nearest source vertex.
///
/// Returns a `MatchResult` with per-target nearest-source index and distance.
pub fn match_nearest(source_verts: &[[f32; 3]], target_verts: &[[f32; 3]]) -> MatchResult {
    if source_verts.is_empty() || target_verts.is_empty() {
        return MatchResult {
            nearest: vec![0; target_verts.len()],
            distances: vec![0.0; target_verts.len()],
        };
    }
    let tree = KdTree::build(source_verts);
    let mut nearest = Vec::with_capacity(target_verts.len());
    let mut distances = Vec::with_capacity(target_verts.len());
    for &tv in target_verts {
        let idx = tree.nearest(tv).unwrap_or(0);
        let d = dist(source_verts[idx], tv);
        nearest.push(idx);
        distances.push(d);
    }
    MatchResult { nearest, distances }
}

/// Compute the centroid (mean position) of a vertex set.
pub fn centroid(verts: &[[f32; 3]]) -> [f32; 3] {
    if verts.is_empty() {
        return [0.0, 0.0, 0.0];
    }
    let n = verts.len() as f32;
    let sum = verts.iter().fold([0.0f32; 3], |acc, v| {
        [acc[0] + v[0], acc[1] + v[1], acc[2] + v[2]]
    });
    [sum[0] / n, sum[1] / n, sum[2] / n]
}

/// Translate all vertices so their centroid lands at the origin.
pub fn center_vertices(verts: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let c = centroid(verts);
    verts
        .iter()
        .map(|&v| [v[0] - c[0], v[1] - c[1], v[2] - c[2]])
        .collect()
}

/// Compute the mean nearest-neighbor distance between two meshes (a simple
/// one-sided Hausdorff approximation useful for verifying match quality).
pub fn mean_nearest_distance(source_verts: &[[f32; 3]], target_verts: &[[f32; 3]]) -> f32 {
    let result = match_nearest(source_verts, target_verts);
    if result.distances.is_empty() {
        return 0.0;
    }
    result.distances.iter().sum::<f32>() / result.distances.len() as f32
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[inline]
fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_nearest_finds_exact_hits() {
        let source = vec![[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        let target = vec![[0.01f32, 0.0, 0.0], [1.99, 0.0, 0.0]];
        let result = match_nearest(&source, &target);
        assert_eq!(result.nearest[0], 0);
        assert_eq!(result.nearest[1], 2);
        assert!(result.distances[0] < 0.1);
        assert!(result.distances[1] < 0.1);
    }

    #[test]
    fn centroid_of_cube_corners_is_center() {
        let verts = vec![
            [0.0f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ];
        let c = centroid(&verts);
        assert!((c[0] - 0.5).abs() < 1e-5);
        assert!((c[1] - 0.5).abs() < 1e-5);
        assert!((c[2] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn mean_nearest_distance_identical_meshes_is_zero() {
        let verts = vec![[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let d = mean_nearest_distance(&verts, &verts);
        assert!(d < 1e-6, "identical meshes should have zero MND");
    }
}
