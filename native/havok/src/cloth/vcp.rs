// Virtual Collision Point (VCP) builder with the SDK field shape
// (Cloth/SimCloth/VirtualCollisionPointsData/hclVirtualCollisionPointsData.h):
// triangle and edge fans around selected particles, with per-fan barycentric
// points indexing a deduplicated dictionary.
//
// Not wired into bake.rs::emit_sim_cloth_data; vanilla cape VCP blobs survive
// via the passthrough_members round-trip.

use std::collections::BTreeMap;

use crate::cloth::setup::mesh::SetupMesh;

/// Barycentric (u, v) pair on a triangle (third coord is `1 - u - v`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarycentricPair {
    pub u: f32,
    pub v: f32,
}

/// Run-time block descriptor — one per real particle that has VCPs.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Negated radius (SDK convention — stored as a negative value to avoid
    /// negation at run time).
    pub safe_displacement_radius: f32,
    pub starting_vcp_index: u16,
    pub num_vcps: u8,
}

/// Linear dictionary entry — points at a contiguous run of barycentrics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarycentricDictionaryEntry {
    pub starting_barycentric_index: u16,
    pub num_barycentrics: u8,
}

/// One section of a triangle fan around a real particle.
#[derive(Debug, Clone, PartialEq)]
pub struct TriangleFanSection {
    /// The two opposing real-particle indices defining the triangle (the
    /// third vertex is the fan's owning particle).
    pub opposite_real_particle_indices: [u16; 2],
    pub barycentric_dictionary_index: u16,
}

/// Triangle fan around a real particle.
#[derive(Debug, Clone, PartialEq)]
pub struct TriangleFan {
    pub real_particle_index: u16,
    pub vcp_start_index: u16,
    pub num_triangles: u8,
}

/// One section of an edge fan around a real particle.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeFanSection {
    pub opposite_real_particle_index: u16,
    pub barycentric_dictionary_index: u16,
}

/// Edge fan around a real particle.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeFan {
    pub real_particle_index: u16,
    pub edge_start_index: u16,
    pub num_edges: u8,
}

/// In-memory mirror of SDK `hclVirtualCollisionPointsData`. Landscape
/// variants are present but always empty in this scope.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VirtualCollisionPointsData {
    pub blocks: Vec<Block>,
    pub num_vc_points: u16,
    pub landscape_particles_block_index: Vec<u16>,
    pub num_landscape_vc_points: u16,

    pub edge_barycentrics_dictionary: Vec<f32>,
    pub edge_dictionary_entries: Vec<BarycentricDictionaryEntry>,
    pub triangle_barycentrics_dictionary: Vec<BarycentricPair>,
    pub triangle_dictionary_entries: Vec<BarycentricDictionaryEntry>,

    pub edges: Vec<EdgeFanSection>,
    pub edge_fans: Vec<EdgeFan>,
    pub triangles: Vec<TriangleFanSection>,
    pub triangle_fans: Vec<TriangleFan>,

    pub edges_landscape: Vec<EdgeFanSection>,
    pub edge_fans_landscape: Vec<EdgeFan>,
    pub triangles_landscape: Vec<TriangleFanSection>,
    pub triangle_fans_landscape: Vec<TriangleFan>,

    pub edge_fan_indices: Vec<u16>,
    pub triangle_fan_indices: Vec<u16>,
    pub edge_fan_indices_landscape: Vec<u16>,
    pub triangle_fan_indices_landscape: Vec<u16>,
}

/// Builder configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VcpBuilderConfig {
    /// Number of VCPs generated per fan section. With density=1, the single
    /// canonical point is the centroid (1/3, 1/3) for triangles and midpoint
    /// (0.5) for edges — matches SDK default for `m_virtualCollisionPointDensities`.
    pub density: u8,
    /// Initial value for `Block::safe_displacement_radius` (stored negated).
    pub safe_displacement_radius: f32,
}

impl Default for VcpBuilderConfig {
    fn default() -> Self {
        Self {
            density: 1,
            safe_displacement_radius: -0.05,
        }
    }
}

/// Build VCP data from a setup mesh and the indices of "real" particles
/// selected for VCP generation. Particles outside the mesh's vertex count are
/// skipped silently.
pub fn build_vcp(
    mesh: &SetupMesh,
    selected_particles: &[u16],
    config: VcpBuilderConfig,
) -> VirtualCollisionPointsData {
    let mut out = VirtualCollisionPointsData::default();
    if selected_particles.is_empty() || mesh.triangles.is_empty() {
        return out;
    }
    let n_verts = mesh.positions.len();
    let density = config.density.max(1);

    // Vertex → adjacent-triangle list.
    let vert_to_tris = build_vertex_to_triangles(&mesh.triangles, n_verts);

    // Vertex → set of adjacent vertices (via shared triangles).
    let vert_to_neighbors = build_vertex_neighbors(&mesh.triangles, n_verts);

    // Dedupe barycentric pairs/floats by quantized integer key (avoid f32 hash).
    let mut tri_dict: Vec<BarycentricPair> = Vec::new();
    let mut tri_dict_keys: BTreeMap<u64, u16> = BTreeMap::new();
    let mut edge_dict: Vec<f32> = Vec::new();
    let mut edge_dict_keys: BTreeMap<u32, u16> = BTreeMap::new();

    let mut total_vcps: u32 = 0;

    for &p in selected_particles {
        let p_usize = p as usize;
        if p_usize >= n_verts {
            continue;
        }

        // ---- Triangle fan ----
        let block_start_vcp = total_vcps as u16;
        let mut triangles_in_fan: u8 = 0;
        let mut tri_section_start: usize = out.triangles.len();
        let _ = tri_section_start;
        tri_section_start = out.triangles.len();

        // Append a barycentric run for this fan — `density` canonical points.
        let dict_run = canonical_triangle_barycentrics(density);
        let dict_idx = intern_triangle_run(&dict_run, &mut tri_dict, &mut tri_dict_keys, &mut out);

        if let Some(tri_indices) = vert_to_tris.get(p_usize) {
            for &ti in tri_indices {
                let tri = mesh.triangles[ti];
                // Identify the two opposite verts in canonical (CCW input) order.
                let (oa, ob) = match opposite_pair(&tri, p) {
                    Some(pair) => pair,
                    None => continue,
                };
                out.triangles.push(TriangleFanSection {
                    opposite_real_particle_indices: [oa, ob],
                    barycentric_dictionary_index: dict_idx,
                });
                triangles_in_fan = triangles_in_fan.saturating_add(1);
                total_vcps += density as u32;
                if triangles_in_fan == u8::MAX {
                    break;
                }
            }
        }

        if triangles_in_fan > 0 {
            out.triangle_fans.push(TriangleFan {
                real_particle_index: p,
                vcp_start_index: block_start_vcp,
                num_triangles: triangles_in_fan,
            });
        } else {
            // Drop this particle entirely if it produced no fan sections.
            continue;
        }

        // ---- Edge fan ----
        let edge_block_start_vcp = total_vcps as u16;
        let mut edges_in_fan: u8 = 0;

        let edge_run = canonical_edge_barycentrics(density);
        let edge_dict_idx =
            intern_edge_run(&edge_run, &mut edge_dict, &mut edge_dict_keys, &mut out);

        if let Some(neighbors) = vert_to_neighbors.get(p_usize) {
            for &nb in neighbors {
                if nb == p {
                    continue;
                }
                out.edges.push(EdgeFanSection {
                    opposite_real_particle_index: nb,
                    barycentric_dictionary_index: edge_dict_idx,
                });
                edges_in_fan = edges_in_fan.saturating_add(1);
                total_vcps += density as u32;
                if edges_in_fan == u8::MAX {
                    break;
                }
            }
        }

        if edges_in_fan > 0 {
            out.edge_fans.push(EdgeFan {
                real_particle_index: p,
                edge_start_index: edge_block_start_vcp,
                num_edges: edges_in_fan,
            });
        }

        // ---- Block ----
        let block_total = total_vcps as u16 - block_start_vcp;
        out.blocks.push(Block {
            safe_displacement_radius: config.safe_displacement_radius,
            starting_vcp_index: block_start_vcp,
            num_vcps: block_total.min(u8::MAX as u16) as u8,
        });
    }

    out.num_vc_points = total_vcps.min(u16::MAX as u32) as u16;

    // Random-access fan-by-particle indexers (SDK m_*FanIndices). Map each
    // selected particle to its fan index, or the count (sentinel = "absent")
    // for unselected ones — matches SDK convention of using ==numFans as
    // "no fan".
    out.triangle_fan_indices = build_fan_indices(&out.triangle_fans, n_verts);
    out.edge_fan_indices = build_fan_indices_edge(&out.edge_fans, n_verts);

    out
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_vertex_to_triangles(triangles: &[[u32; 3]], n_verts: usize) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n_verts];
    for (ti, tri) in triangles.iter().enumerate() {
        for &v in tri {
            let vi = v as usize;
            if vi < n_verts {
                out[vi].push(ti);
            }
        }
    }
    out
}

fn build_vertex_neighbors(triangles: &[[u32; 3]], n_verts: usize) -> Vec<Vec<u16>> {
    let mut sets: Vec<std::collections::BTreeSet<u16>> = (0..n_verts)
        .map(|_| std::collections::BTreeSet::new())
        .collect();
    for tri in triangles {
        let a = tri[0] as usize;
        let b = tri[1] as usize;
        let c = tri[2] as usize;
        if a < n_verts && b < n_verts {
            sets[a].insert(b as u16);
            sets[b].insert(a as u16);
        }
        if b < n_verts && c < n_verts {
            sets[b].insert(c as u16);
            sets[c].insert(b as u16);
        }
        if a < n_verts && c < n_verts {
            sets[a].insert(c as u16);
            sets[c].insert(a as u16);
        }
    }
    sets.into_iter().map(|s| s.into_iter().collect()).collect()
}

fn opposite_pair(tri: &[u32; 3], p: u16) -> Option<(u16, u16)> {
    let p32 = p as u32;
    let mut others: [u16; 2] = [0, 0];
    let mut idx = 0;
    for &v in tri {
        if v == p32 {
            continue;
        }
        if idx >= 2 {
            return None;
        }
        if v > u16::MAX as u32 {
            return None;
        }
        others[idx] = v as u16;
        idx += 1;
    }
    if idx == 2 {
        Some((others[0], others[1]))
    } else {
        None
    }
}

fn canonical_triangle_barycentrics(density: u8) -> Vec<BarycentricPair> {
    // Density=1 → centroid. Density=N → first centroid then evenly spaced
    // sub-points along the (1/(N+1), 1/(N+1)) → (1 - 2/(N+1), 1/(N+1)) line.
    let n = density as usize;
    let mut out = Vec::with_capacity(n);
    out.push(BarycentricPair {
        u: 1.0 / 3.0,
        v: 1.0 / 3.0,
    });
    for k in 1..n {
        let t = (k as f32) / (n as f32);
        let u = 1.0 / 3.0 + t * (1.0 / 3.0);
        let v = 1.0 / 3.0 - t * (1.0 / 6.0);
        out.push(BarycentricPair { u, v });
    }
    out
}

fn canonical_edge_barycentrics(density: u8) -> Vec<f32> {
    let n = density as usize;
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        // Spread points across (0,1) avoiding endpoints.
        let t = (k as f32 + 1.0) / (n as f32 + 1.0);
        out.push(t);
    }
    out
}

fn quantize_pair(p: BarycentricPair) -> u64 {
    let u = (p.u.clamp(0.0, 1.0) * 65535.0).round() as u64;
    let v = (p.v.clamp(0.0, 1.0) * 65535.0).round() as u64;
    (u << 32) | v
}

fn quantize_f32(f: f32) -> u32 {
    (f.clamp(0.0, 1.0) * 65535.0).round() as u32
}

fn intern_triangle_run(
    run: &[BarycentricPair],
    dict: &mut Vec<BarycentricPair>,
    keys: &mut BTreeMap<u64, u16>,
    out: &mut VirtualCollisionPointsData,
) -> u16 {
    // Hash on the run as a whole (concatenated key) for dictionary-entry dedup.
    let mut combined: u128 = 0xcbf29ce484222325;
    for p in run {
        combined ^= quantize_pair(*p) as u128;
        combined = combined.wrapping_mul(0x100000001b3);
    }
    let combined_key = combined as u64; // good enough for small dicts
    if let Some(&existing) = keys.get(&combined_key) {
        return existing;
    }
    let start = dict.len() as u16;
    for p in run {
        dict.push(*p);
    }
    let entry = BarycentricDictionaryEntry {
        starting_barycentric_index: start,
        num_barycentrics: run.len().min(u8::MAX as usize) as u8,
    };
    let entry_index = out.triangle_dictionary_entries.len() as u16;
    out.triangle_dictionary_entries.push(entry);
    out.triangle_barycentrics_dictionary = dict.clone();
    keys.insert(combined_key, entry_index);
    entry_index
}

fn intern_edge_run(
    run: &[f32],
    dict: &mut Vec<f32>,
    keys: &mut BTreeMap<u32, u16>,
    out: &mut VirtualCollisionPointsData,
) -> u16 {
    let mut combined: u64 = 0xcbf29ce484222325;
    for f in run {
        combined ^= quantize_f32(*f) as u64;
        combined = combined.wrapping_mul(0x100000001b3);
    }
    let combined_key = combined as u32;
    if let Some(&existing) = keys.get(&combined_key) {
        return existing;
    }
    let start = dict.len() as u16;
    for f in run {
        dict.push(*f);
    }
    let entry = BarycentricDictionaryEntry {
        starting_barycentric_index: start,
        num_barycentrics: run.len().min(u8::MAX as usize) as u8,
    };
    let entry_index = out.edge_dictionary_entries.len() as u16;
    out.edge_dictionary_entries.push(entry);
    out.edge_barycentrics_dictionary = dict.clone();
    keys.insert(combined_key, entry_index);
    entry_index
}

fn build_fan_indices(fans: &[TriangleFan], n_verts: usize) -> Vec<u16> {
    let mut idx = vec![fans.len() as u16; n_verts];
    for (fi, fan) in fans.iter().enumerate() {
        let pi = fan.real_particle_index as usize;
        if pi < n_verts {
            idx[pi] = fi as u16;
        }
    }
    idx
}

fn build_fan_indices_edge(fans: &[EdgeFan], n_verts: usize) -> Vec<u16> {
    let mut idx = vec![fans.len() as u16; n_verts];
    for (fi, fan) in fans.iter().enumerate() {
        let pi = fan.real_particle_index as usize;
        if pi < n_verts {
            idx[pi] = fi as u16;
        }
    }
    idx
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn quad_mesh() -> SetupMesh {
        // Four corner verts of a unit quad in z=0, plus one center vert.
        // Triangles: (0,1,4), (1,2,4), (2,3,4), (3,0,4) — fan around vertex 4.
        let mut m = SetupMesh::default();
        m.positions = vec![
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.5, 0.5, 0.0, 1.0],
        ];
        m.triangles = vec![[0, 1, 4], [1, 2, 4], [2, 3, 4], [3, 0, 4]];
        m
    }

    #[test]
    fn build_vcp_on_quad_centered_particle_produces_one_fan() {
        let mesh = quad_mesh();
        let cfg = VcpBuilderConfig::default();
        let vcp = build_vcp(&mesh, &[4], cfg);

        assert_eq!(vcp.triangle_fans.len(), 1);
        let fan = &vcp.triangle_fans[0];
        assert_eq!(fan.real_particle_index, 4);
        assert_eq!(fan.num_triangles, 4);

        // Four triangle sections — one per fan triangle.
        assert_eq!(vcp.triangles.len(), 4);
        for sec in &vcp.triangles {
            // The two opposite verts must each be one of {0,1,2,3}.
            for &op in &sec.opposite_real_particle_indices {
                assert!(op < 4, "opposite vert {op} not in outer ring");
            }
        }

        // Edge fan: vertex 4 is connected to all four corners — 4 edges.
        assert_eq!(vcp.edge_fans.len(), 1);
        assert_eq!(vcp.edge_fans[0].num_edges, 4);
        assert_eq!(vcp.edges.len(), 4);

        // Total VCP count = 4 triangle points + 4 edge points = 8.
        assert_eq!(vcp.num_vc_points, 8);

        // Block accounting matches.
        assert_eq!(vcp.blocks.len(), 1);
        assert_eq!(vcp.blocks[0].starting_vcp_index, 0);
        assert_eq!(vcp.blocks[0].num_vcps, 8);
    }

    #[test]
    fn triangle_dictionary_is_deduped() {
        // Multiple selected particles — every fan uses the same canonical
        // (1/3, 1/3) centroid run, so the dict should have exactly one entry.
        let mesh = quad_mesh();
        let cfg = VcpBuilderConfig::default();
        let vcp = build_vcp(&mesh, &[0, 1, 2, 3, 4], cfg);

        assert_eq!(vcp.triangle_dictionary_entries.len(), 1);
        assert_eq!(vcp.triangle_barycentrics_dictionary.len(), 1);
        let bp = vcp.triangle_barycentrics_dictionary[0];
        assert!((bp.u - 1.0 / 3.0).abs() < 1e-6);
        assert!((bp.v - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn barycentric_weights_in_unit_simplex() {
        let mesh = quad_mesh();
        let cfg = VcpBuilderConfig {
            density: 3,
            ..Default::default()
        };
        let vcp = build_vcp(&mesh, &[4], cfg);

        for bp in &vcp.triangle_barycentrics_dictionary {
            assert!(
                bp.u >= 0.0 && bp.v >= 0.0 && bp.u + bp.v <= 1.0 + 1e-6,
                "barycentric ({}, {}) outside unit simplex",
                bp.u,
                bp.v,
            );
        }
        for &t in &vcp.edge_barycentrics_dictionary {
            assert!(
                (0.0..=1.0).contains(&t),
                "edge barycentric {t} outside [0,1]"
            );
        }
    }

    #[test]
    fn centroid_reconstruction_lies_on_triangle_for_density_1() {
        // For density=1 (centroid only) the reconstructed point must equal
        // the triangle centroid in world space.
        let mesh = quad_mesh();
        let vcp = build_vcp(&mesh, &[4], VcpBuilderConfig::default());
        // Pick the first triangle section (which corresponds to one of the
        // four fan triangles around vertex 4).
        let sec = &vcp.triangles[0];
        let entry = &vcp.triangle_dictionary_entries[sec.barycentric_dictionary_index as usize];
        let bp = vcp.triangle_barycentrics_dictionary[entry.starting_barycentric_index as usize];

        // Reconstruct a triangle from (vertex 4, two opposites). Since we
        // don't know which fan triangle this is exactly without a back-map,
        // verify the property holds: centroid of any (4, oa, ob) where oa,
        // ob are the section's two opposite real-particle indices.
        let p_owner = mesh.positions[4];
        let p_a = mesh.positions[sec.opposite_real_particle_indices[0] as usize];
        let p_b = mesh.positions[sec.opposite_real_particle_indices[1] as usize];

        // Reconstruct point at barycentric (bp.u, bp.v, 1-bp.u-bp.v) on
        // (owner, oa, ob).
        let w0 = 1.0 - bp.u - bp.v;
        let recon = [
            w0 * p_owner[0] + bp.u * p_a[0] + bp.v * p_b[0],
            w0 * p_owner[1] + bp.u * p_a[1] + bp.v * p_b[1],
            w0 * p_owner[2] + bp.u * p_a[2] + bp.v * p_b[2],
        ];
        let centroid = [
            (p_owner[0] + p_a[0] + p_b[0]) / 3.0,
            (p_owner[1] + p_a[1] + p_b[1]) / 3.0,
            (p_owner[2] + p_a[2] + p_b[2]) / 3.0,
        ];
        for k in 0..3 {
            assert!(
                (recon[k] - centroid[k]).abs() < 1e-5,
                "centroid mismatch axis {k}: {} vs {}",
                recon[k],
                centroid[k]
            );
        }
    }

    #[test]
    fn unselected_particles_get_sentinel_fan_index() {
        let mesh = quad_mesh();
        let vcp = build_vcp(&mesh, &[4], VcpBuilderConfig::default());

        // Only vertex 4 has a fan; vertices 0..3 should map to the
        // out-of-range sentinel value (== num_fans).
        let n_fans = vcp.triangle_fans.len() as u16;
        assert_eq!(vcp.triangle_fan_indices[4], 0);
        for i in 0..4 {
            assert_eq!(vcp.triangle_fan_indices[i], n_fans);
        }
    }

    #[test]
    fn empty_mesh_produces_empty_data() {
        let mut mesh = SetupMesh::default();
        mesh.positions = vec![[0.0, 0.0, 0.0, 1.0]];
        let vcp = build_vcp(&mesh, &[0], VcpBuilderConfig::default());
        assert_eq!(vcp.num_vc_points, 0);
        assert!(vcp.blocks.is_empty());
        assert!(vcp.triangle_fans.is_empty());
    }
}
