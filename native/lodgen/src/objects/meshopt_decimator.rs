use std::mem;

use crate::objects::geometry::LodGeometry;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshoptDecimationStats {
    pub before_triangles: usize,
    pub after_triangles: usize,
    pub used_attr_simplifier: bool,
    pub used_sloppy: bool,
    pub changed: bool,
}

pub fn decimate(
    geometry: &mut LodGeometry,
    target_triangles: usize,
    target_error: f32,
    allow_sloppy: bool,
) -> MeshoptDecimationStats {
    let before = geometry.num_triangles();
    let mut stats = MeshoptDecimationStats {
        before_triangles: before,
        after_triangles: before,
        used_attr_simplifier: false,
        used_sloppy: false,
        changed: false,
    };

    let target_triangles = target_triangles.max(1);
    if before <= target_triangles || geometry.vertices.len() < 4 {
        return stats;
    }

    let target_index_count = target_triangles.saturating_mul(3);
    if target_index_count >= before.saturating_mul(3) {
        return stats;
    }

    let mut result_error = 0.0f32;
    let indices = flatten_indices(&geometry.triangles);
    let positions = vertex_adapter(geometry);
    let locks = vec![false; geometry.vertices.len()];
    let (attributes, weights, attribute_count) = vertex_attributes(geometry);

    let simplified = if attribute_count > 0 {
        stats.used_attr_simplifier = true;
        meshopt::simplify_with_attributes_and_locks(
            &indices,
            &positions,
            &attributes,
            &weights,
            attribute_count * mem::size_of::<f32>(),
            &locks,
            target_index_count,
            target_error,
            meshopt::SimplifyOptions::None,
            Some(&mut result_error),
        )
    } else {
        meshopt::simplify_with_locks(
            &indices,
            &positions,
            &locks,
            target_index_count,
            target_error,
            meshopt::SimplifyOptions::None,
            Some(&mut result_error),
        )
    };

    if simplified.len() >= 3 && simplified.len() < indices.len() {
        apply_indices(geometry, &simplified);
        stats.changed = geometry.num_triangles() < before;
        stats.after_triangles = geometry.num_triangles();
    }

    if allow_sloppy && geometry.num_triangles() > target_triangles {
        let sloppy_indices = flatten_indices(&geometry.triangles);
        let sloppy_positions = vertex_adapter(geometry);
        let sloppy = meshopt::simplify_sloppy(
            &sloppy_indices,
            &sloppy_positions,
            target_index_count.min(sloppy_indices.len()),
            target_error,
            Some(&mut result_error),
        );
        if sloppy.len() >= 3 && sloppy.len() < sloppy_indices.len() {
            let before_sloppy = geometry.num_triangles();
            apply_indices(geometry, &sloppy);
            if geometry.num_triangles() < before_sloppy {
                stats.used_sloppy = true;
                stats.changed = true;
                stats.after_triangles = geometry.num_triangles();
            }
        }
    }

    stats.after_triangles = geometry.num_triangles();
    stats
}

fn flatten_indices(triangles: &[[u32; 3]]) -> Vec<u32> {
    let mut indices = Vec::with_capacity(triangles.len() * 3);
    for tri in triangles {
        indices.extend_from_slice(tri);
    }
    indices
}

fn vertex_adapter(geometry: &LodGeometry) -> meshopt::VertexDataAdapter<'_> {
    meshopt::VertexDataAdapter::new(
        meshopt::typed_to_bytes(&geometry.vertices),
        mem::size_of::<[f32; 3]>(),
        0,
    )
    .expect("LodGeometry vertices must be tightly packed [f32; 3]")
}

fn vertex_attributes(geometry: &LodGeometry) -> (Vec<f32>, Vec<f32>, usize) {
    let vertex_count = geometry.vertices.len();
    let has_uv = geometry.uvcoords.len() == vertex_count;
    let has_normals = geometry.normals.len() == vertex_count;
    let has_colors = geometry.vertex_colors.len() == vertex_count;

    let mut attribute_count = 0usize;
    if has_uv {
        attribute_count += 2;
    }
    if has_normals {
        attribute_count += 3;
    }
    if has_colors {
        attribute_count += 4;
    }
    if attribute_count == 0 {
        return (Vec::new(), Vec::new(), 0);
    }

    let mut attributes = Vec::with_capacity(vertex_count * attribute_count);
    for i in 0..vertex_count {
        if has_uv {
            attributes.extend_from_slice(&geometry.uvcoords[i]);
        }
        if has_normals {
            attributes.extend_from_slice(&geometry.normals[i]);
        }
        if has_colors {
            attributes.extend_from_slice(&geometry.vertex_colors[i]);
        }
    }

    let mut weights = Vec::with_capacity(attribute_count);
    if has_uv {
        weights.extend_from_slice(&[1.0, 1.0]);
    }
    if has_normals {
        weights.extend_from_slice(&[0.25, 0.25, 0.25]);
    }
    if has_colors {
        weights.extend_from_slice(&[0.10, 0.10, 0.10, 0.10]);
    }

    (attributes, weights, attribute_count)
}

fn apply_indices(geometry: &mut LodGeometry, indices: &[u32]) {
    let vertex_count = geometry.vertices.len() as u32;
    let mut triangles = Vec::with_capacity(indices.len() / 3);
    for chunk in indices.chunks_exact(3) {
        let tri = [chunk[0], chunk[1], chunk[2]];
        if tri[0] >= vertex_count
            || tri[1] >= vertex_count
            || tri[2] >= vertex_count
            || tri[0] == tri[1]
            || tri[1] == tri[2]
            || tri[0] == tri[2]
        {
            continue;
        }
        triangles.push(tri);
    }
    if triangles.is_empty() {
        return;
    }
    geometry.triangles = triangles;
    geometry.remove_unused();
    geometry.update_bbox();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(size: usize) -> LodGeometry {
        let mut g = LodGeometry::new();
        for y in 0..=size {
            for x in 0..=size {
                g.vertices.push([x as f32, y as f32, 0.0]);
                g.uvcoords
                    .push([x as f32 / size as f32, y as f32 / size as f32]);
                g.normals.push([0.0, 0.0, 1.0]);
                g.vertex_colors.push([1.0, 0.5, 0.25, 1.0]);
            }
        }
        let row = size + 1;
        for y in 0..size {
            for x in 0..size {
                let v0 = (y * row + x) as u32;
                let v1 = v0 + 1;
                let v2 = v0 + row as u32;
                let v3 = v2 + 1;
                g.triangles.push([v0, v1, v3]);
                g.triangles.push([v0, v3, v2]);
            }
        }
        g
    }

    #[test]
    fn meshopt_decimates_dense_grid_to_target() {
        let mut g = grid(16);
        let before = g.num_triangles();
        let stats = decimate(&mut g, 64, 0.05, true);
        assert!(stats.changed);
        assert!(stats.used_attr_simplifier);
        assert!(g.num_triangles() <= 64, "{} > 64", g.num_triangles());
        assert!(g.num_triangles() < before);
        let max = g.vertices.len() as u32;
        assert!(g.triangles.iter().flatten().all(|idx| *idx < max));
    }

    #[test]
    fn meshopt_preserves_parallel_arrays_after_compaction() {
        let mut g = grid(12);
        let stats = decimate(&mut g, 48, 0.05, true);
        assert!(stats.changed);
        assert_eq!(g.vertices.len(), g.uvcoords.len());
        assert_eq!(g.vertices.len(), g.normals.len());
        assert_eq!(g.vertices.len(), g.vertex_colors.len());
    }
}
