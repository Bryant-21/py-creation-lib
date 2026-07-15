/// Merge multiple mesh sections (vertex + triangle arrays) into a single mesh.
///
/// Each section is an independent (vertices, triangles) pair.  This utility
/// concatenates them, offsetting triangle indices so they refer to the correct
/// position in the merged vertex array.

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single mesh section: vertices and triangles.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshSection {
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Merge multiple mesh sections into one.
///
/// Sections are concatenated in order.  Triangle indices are offset by the
/// running vertex count so each section maps into the correct sub-range of
/// the merged vertex array.
///
/// Returns the merged section.  An empty input produces an empty section.
pub fn merge_sections(sections: &[MeshSection]) -> MeshSection {
    let total_verts: usize = sections.iter().map(|s| s.vertices.len()).sum();
    let total_tris: usize = sections.iter().map(|s| s.triangles.len()).sum();

    let mut vertices: Vec<[f32; 3]> = Vec::with_capacity(total_verts);
    let mut triangles: Vec<[u32; 3]> = Vec::with_capacity(total_tris);
    let mut base: u32 = 0;

    for sec in sections {
        vertices.extend_from_slice(&sec.vertices);
        for tri in &sec.triangles {
            triangles.push([tri[0] + base, tri[1] + base, tri[2] + base]);
        }
        base += sec.vertices.len() as u32;
    }

    MeshSection {
        vertices,
        triangles,
    }
}

/// Split a merged mesh back into sections of at most `max_verts` vertices each.
///
/// Triangles that span a section boundary are not split — if a triangle has
/// vertices in multiple output sections after a greedy bin assignment, it is
/// placed in the section containing its first vertex.  For best results, feed
/// a mesh that was originally built with a per-section vertex limit in mind.
pub fn split_sections(mesh: &MeshSection, max_verts: usize) -> Vec<MeshSection> {
    if mesh.vertices.is_empty() || max_verts == 0 {
        return vec![];
    }

    // Assign each original vertex to a bin
    let n = mesh.vertices.len();
    let mut vertex_bin: Vec<usize> = Vec::with_capacity(n);
    let mut bin_count = 0usize;
    let mut current_bin_size = 0usize;

    for _ in 0..n {
        if current_bin_size >= max_verts {
            bin_count += 1;
            current_bin_size = 0;
        }
        vertex_bin.push(bin_count);
        current_bin_size += 1;
    }
    let num_bins = bin_count + 1;

    // Build per-bin vertex → local-index maps
    let mut bin_local: Vec<std::collections::HashMap<usize, u32>> =
        vec![std::collections::HashMap::new(); num_bins];
    let mut bin_verts: Vec<Vec<[f32; 3]>> = vec![Vec::new(); num_bins];

    for (gi, &v) in mesh.vertices.iter().enumerate() {
        let bin = vertex_bin[gi];
        let local_idx = bin_verts[bin].len() as u32;
        bin_local[bin].insert(gi, local_idx);
        bin_verts[bin].push(v);
    }

    // Distribute triangles into bins by first vertex
    let mut bin_tris: Vec<Vec<[u32; 3]>> = vec![Vec::new(); num_bins];
    for tri in &mesh.triangles {
        let bin = vertex_bin[tri[0] as usize];
        let map = &bin_local[bin];
        let la = *map.get(&(tri[0] as usize)).unwrap_or(&0);
        let lb = *map.get(&(tri[1] as usize)).unwrap_or(&0);
        let lc = *map.get(&(tri[2] as usize)).unwrap_or(&0);
        bin_tris[bin].push([la, lb, lc]);
    }

    bin_verts
        .into_iter()
        .zip(bin_tris.into_iter())
        .filter(|(v, t)| !v.is_empty() || !t.is_empty())
        .map(|(vertices, triangles)| MeshSection {
            vertices,
            triangles,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_section(offset: f32) -> MeshSection {
        MeshSection {
            vertices: vec![
                [offset, 0.0, 0.0],
                [offset + 1.0, 0.0, 0.0],
                [offset, 1.0, 0.0],
            ],
            triangles: vec![[0, 1, 2]],
        }
    }

    #[test]
    fn merge_two_sections_offsets_indices() {
        let sections = vec![simple_section(0.0), simple_section(10.0)];
        let merged = merge_sections(&sections);
        assert_eq!(merged.vertices.len(), 6);
        assert_eq!(merged.triangles.len(), 2);
        assert_eq!(merged.triangles[0], [0, 1, 2]);
        assert_eq!(merged.triangles[1], [3, 4, 5]);
    }

    #[test]
    fn merge_empty_input_is_empty() {
        let merged = merge_sections(&[]);
        assert!(merged.vertices.is_empty());
        assert!(merged.triangles.is_empty());
    }

    #[test]
    fn split_restores_sections() {
        let sections = vec![simple_section(0.0), simple_section(10.0)];
        let merged = merge_sections(&sections);
        let split = split_sections(&merged, 3);
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].vertices.len(), 3);
        assert_eq!(split[1].vertices.len(), 3);
    }
}
