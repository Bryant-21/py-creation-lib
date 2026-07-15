/// Compute per-vertex bone weights using inverse-distance weighting.
///
/// For each vertex, finds the nearest bones and assigns weights
/// proportional to 1/distance^falloff_power, normalized to sum to 1.
///
/// Returns a Vec<Vec<(bone_index, weight)>> mirroring the Python output.
pub fn auto_skin_to_cloth_bones(
    vertex_positions: &[[f32; 4]],
    bone_positions: &[[f32; 4]],
    max_bones_per_vertex: usize,
    falloff_power: f32,
) -> Vec<Vec<(usize, f32)>> {
    if bone_positions.is_empty() || vertex_positions.is_empty() {
        return vertex_positions.iter().map(|_| Vec::new()).collect();
    }

    let n_bones = bone_positions.len();
    let max_per = max_bones_per_vertex.min(n_bones);

    vertex_positions
        .iter()
        .map(|vpos| {
            // Compute distances to all bones
            let mut dists: Vec<(f32, usize)> = bone_positions
                .iter()
                .enumerate()
                .map(|(bi, bpos)| {
                    let dx = vpos[0] - bpos[0];
                    let dy = vpos[1] - bpos[1];
                    let dz = vpos[2] - bpos[2];
                    let d = (dx * dx + dy * dy + dz * dz).sqrt();
                    (d, bi)
                })
                .collect();

            dists.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let closest = &dists[..max_per];

            // Check for zero-distance (exact match)
            if closest[0].0 < 1e-10 {
                return vec![(closest[0].1, 1.0f32)];
            }

            // Inverse-distance weights
            let weights: Vec<(usize, f32)> = closest
                .iter()
                .map(|(d, bi)| {
                    let w = 1.0 / d.powf(falloff_power);
                    (*bi, w)
                })
                .collect();

            let total: f32 = weights.iter().map(|(_, w)| w).sum();
            if total > 0.0 {
                weights.into_iter().map(|(bi, w)| (bi, w / total)).collect()
            } else {
                Vec::new()
            }
        })
        .collect()
}

/// Convenience wrapper returning (bone_names, bone_weights) for SetupMesh.
pub fn skin_weights_for_setup_mesh<'a>(
    vertex_positions: &[[f32; 4]],
    bone_positions: &[[f32; 4]],
    bone_names: &'a [String],
    max_bones_per_vertex: usize,
) -> (&'a [String], Vec<Vec<(usize, f32)>>) {
    let weights =
        auto_skin_to_cloth_bones(vertex_positions, bone_positions, max_bones_per_vertex, 2.0);
    (bone_names, weights)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_returns_full_weight() {
        let v = vec![[0.0f32, 0.0, 0.0, 0.0]];
        let b = vec![[0.0f32, 0.0, 0.0, 0.0], [10.0, 0.0, 0.0, 0.0]];
        let w = auto_skin_to_cloth_bones(&v, &b, 4, 2.0);
        assert_eq!(w[0], vec![(0, 1.0)]);
    }

    #[test]
    fn nearest_bone_gets_highest_weight() {
        let v = vec![[1.0f32, 0.0, 0.0, 0.0]];
        let b = vec![[0.0f32, 0.0, 0.0, 0.0], [10.0, 0.0, 0.0, 0.0]];
        let w = auto_skin_to_cloth_bones(&v, &b, 2, 2.0);
        assert_eq!(w[0].len(), 2);
        // Bone 0 (distance 1) should have higher weight than bone 1 (distance 9)
        let w0 = w[0]
            .iter()
            .find(|(bi, _)| *bi == 0)
            .map(|(_, w)| *w)
            .unwrap();
        let w1 = w[0]
            .iter()
            .find(|(bi, _)| *bi == 1)
            .map(|(_, w)| *w)
            .unwrap();
        assert!(w0 > w1, "Nearest bone should have higher weight");
    }
}
