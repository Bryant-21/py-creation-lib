// Rust convex hull — 3D Quickhull algorithm producing Havok polytope topology.
//
// Output contract matches Python's _compute_hull_topology return value:
//   hull_verts  — deduplicated vertices used by the hull (subset of input)
//   planes      — [nx, ny, nz, d] per face (outward normals, d = –dot(n, point))
//   faces       — (firstIndex u16, numIndices u8, minHalfAngle u8) per face
//   indices     — flat index buffer consumed by faces
//   edges       — (faceIdx u16, edgeIdx u8, padding u8) per edge
//   vertex_edges — per-vertex first-edge: face_idx | (edge_idx << 16)
//
// The Quickhull implementation follows the standard recursive algorithm with
// epsilon-based coplanarity testing.  It produces triangular facets which are
// kept as-is (numIndices = 3) matching the previous ConvexHull simplices.

use crate::error::{HavokError, HavokResult};

const EPS: f32 = 1e-7;

// ---------------------------------------------------------------------------
// Pre-processing
// ---------------------------------------------------------------------------

/// Weld vertices that are closer than `epsilon` to each other.
///
/// Returns a deduplicated vertex array and a remap table: `remap[old_idx] = new_idx`.
pub fn weld_vertices(input: &[[f32; 3]], epsilon: f32) -> (Vec<[f32; 3]>, Vec<usize>) {
    let mut welded: Vec<[f32; 3]> = Vec::with_capacity(input.len());
    let mut remap: Vec<usize> = vec![0; input.len()];
    for (i, &v) in input.iter().enumerate() {
        let found = welded
            .iter()
            .enumerate()
            .find(|(_, w)| dist_sq3(v, **w).sqrt() < epsilon);
        match found {
            Some((j, _)) => remap[i] = j,
            None => {
                remap[i] = welded.len();
                welded.push(v);
            }
        }
    }
    (welded, remap)
}

/// Compute the max axis-aligned extent of `verts`.
pub fn max_extent(verts: &[[f32; 3]]) -> f32 {
    if verts.is_empty() {
        return 0.0;
    }
    let (mut mn, mut mx) = (verts[0], verts[0]);
    for &v in verts.iter().skip(1) {
        for k in 0..3 {
            if v[k] < mn[k] {
                mn[k] = v[k];
            }
            if v[k] > mx[k] {
                mx[k] = v[k];
            }
        }
    }
    let dx = mx[0] - mn[0];
    let dy = mx[1] - mn[1];
    let dz = mx[2] - mn[2];
    dx.max(dy).max(dz)
}

#[inline]
fn dist_sq3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = sub(a, b);
    dot(d, d)
}

// ---------------------------------------------------------------------------
// Public output type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct HullTopology {
    /// Hull vertices (deduplicated subset of input).
    pub vertices: Vec<[f32; 3]>,
    /// [nx, ny, nz, d] per face — outward normal, d = plane offset.
    pub planes: Vec<[f32; 4]>,
    /// (firstIndex, numIndices, minHalfAngle) per face.
    pub faces: Vec<(u16, u8, u8)>,
    /// Flat index buffer; faces[i] consumes faces[i].1 entries starting at faces[i].0.
    pub indices: Vec<u8>,
    /// (faceIdx, edgeIdx, padding) per edge.
    pub edges: Vec<(u16, u8, u8)>,
    /// Per-vertex first-edge lookup: face_idx | (edge_idx << 16).
    pub vertex_edges: Vec<u32>,
}

// ---------------------------------------------------------------------------
// Internal facet
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Facet {
    /// Vertex indices into the working vertex list (original input indices).
    verts: [usize; 3],
    /// Outward-facing plane: [nx, ny, nz, d].
    plane: [f32; 4],
}

impl Facet {
    fn new(a: usize, b: usize, c: usize, verts: &[[f32; 3]], interior: [f32; 3]) -> Self {
        let pa = verts[a];
        let pb = verts[b];
        let pc = verts[c];
        let n = cross(sub(pb, pa), sub(pc, pa));
        let mag = length(n);
        let n = if mag < EPS {
            [0.0, 0.0, 1.0]
        } else {
            scale(n, 1.0 / mag)
        };
        // d = –dot(n, point_on_plane)
        let d = -(dot(n, pa));
        // Ensure outward normal (away from interior). If the RH-rule normal
        // points toward the interior we flip BOTH the plane and the vertex
        // order — keeping winding consistent with the (now outward) normal.
        // Without the matching winding flip, downstream face-merging breaks:
        // adjacent triangles can end up with the same directed half-edge,
        // collapsing boundary extraction (and producing missing-corner
        // polygons that fail the unit-cube quad invariant).
        let flipped = dot(n, interior) + d > 0.0;
        let plane = if flipped {
            let nf = scale(n, -1.0);
            [nf[0], nf[1], nf[2], -d]
        } else {
            [n[0], n[1], n[2], d]
        };
        let verts_out = if flipped { [a, c, b] } else { [a, b, c] };
        Facet {
            verts: verts_out,
            plane,
        }
    }

    fn signed_dist(&self, p: [f32; 3]) -> f32 {
        dot([self.plane[0], self.plane[1], self.plane[2]], p) + self.plane[3]
    }
}

// ---------------------------------------------------------------------------
// Quickhull
// ---------------------------------------------------------------------------

/// `compute_hull_topology` with automatic vertex welding.
///
/// Welds vertices closer than `max_extent * 1e-5` before running Quickhull.
/// Falls back to a capsule-like "axis-aligned box" hull for collinear inputs
/// (adds two synthetic off-axis vertices so the hull can always be formed).
pub fn compute_hull_topology_robust(input: &[[f32; 3]]) -> HavokResult<HullTopology> {
    if input.is_empty() {
        return Err(HavokError::InvalidInput("empty vertex set".to_string()));
    }
    // Weld vertices using epsilon = max_extent * 1e-5 (or 1e-5 absolute minimum)
    let ext = max_extent(input);
    let eps = (ext * 1e-5_f32).max(1e-5);
    let (welded, _) = weld_vertices(input, eps);
    if welded.len() < 4 {
        // Inflate to at least 4 non-coplanar points by adding synthetic off-axis verts
        return compute_hull_topology(&inflate_degenerate(&welded));
    }
    compute_hull_topology(&welded)
}

/// Inflate a degenerate (collinear / near-coplanar) point set to at least
/// 4 non-coplanar vertices suitable for Quickhull.
fn inflate_degenerate(input: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let mut pts = input.to_vec();
    // Ensure we have at least one pair with non-zero extent
    let ext = max_extent(input).max(1.0);
    let c = if !pts.is_empty() {
        let s: [f32; 3] = pts
            .iter()
            .fold([0.0; 3], |a, v| [a[0] + v[0], a[1] + v[1], a[2] + v[2]]);
        let n = pts.len() as f32;
        [s[0] / n, s[1] / n, s[2] / n]
    } else {
        [0.0; 3]
    };
    // Add 4 synthetic octahedral verts around the centroid
    let r = ext * 0.5;
    for &off in &[
        [r, 0.0, 0.0_f32],
        [-r, 0.0, 0.0],
        [0.0, r, 0.0],
        [0.0, 0.0, r],
    ] {
        pts.push([c[0] + off[0], c[1] + off[1], c[2] + off[2]]);
    }
    pts
}

pub fn compute_hull_topology(input: &[[f32; 3]]) -> HavokResult<HullTopology> {
    if input.len() < 4 {
        return Err(HavokError::InvalidInput(
            "at least 4 non-coplanar vertices required".to_string(),
        ));
    }

    // Step 1: find 6 extreme points to seed the initial simplex
    let mut min_x = 0usize;
    let mut max_x = 0usize;
    let mut min_y = 0usize;
    let mut max_y = 0usize;
    let mut min_z = 0usize;
    let mut max_z = 0usize;
    for (i, &v) in input.iter().enumerate() {
        if v[0] < input[min_x][0] {
            min_x = i;
        }
        if v[0] > input[max_x][0] {
            max_x = i;
        }
        if v[1] < input[min_y][1] {
            min_y = i;
        }
        if v[1] > input[max_y][1] {
            max_y = i;
        }
        if v[2] < input[min_z][2] {
            min_z = i;
        }
        if v[2] > input[max_z][2] {
            max_z = i;
        }
    }
    let extremes = [min_x, max_x, min_y, max_y, min_z, max_z];

    // Step 2: find the two most-distant extreme points → initial edge
    let (mut a, mut b) = (extremes[0], extremes[1]);
    let mut best_dist = dist_sq(input[a], input[b]);
    for &ei in &extremes {
        for &ej in &extremes {
            let d = dist_sq(input[ei], input[ej]);
            if d > best_dist {
                best_dist = d;
                a = ei;
                b = ej;
            }
        }
    }
    if best_dist < EPS {
        return Err(HavokError::InvalidInput(
            "all vertices are coincident".to_string(),
        ));
    }

    // Step 3: find point C farthest from line AB
    let mut c = usize::MAX;
    let mut best_line_dist = 0.0f32;
    let ab = sub(input[b], input[a]);
    for (i, &v) in input.iter().enumerate() {
        if i == a || i == b {
            continue;
        }
        let av = sub(v, input[a]);
        let d = length(cross(ab, av));
        if d > best_line_dist {
            best_line_dist = d;
            c = i;
        }
    }
    if c == usize::MAX || best_line_dist < EPS {
        return Err(HavokError::InvalidInput(
            "vertices are collinear".to_string(),
        ));
    }

    // Step 4: find point D farthest from plane ABC
    let n_abc = cross(sub(input[b], input[a]), sub(input[c], input[a]));
    let mut d = usize::MAX;
    let mut best_plane_dist = 0.0f32;
    for (i, &v) in input.iter().enumerate() {
        if i == a || i == b || i == c {
            continue;
        }
        let dist = dot(n_abc, sub(v, input[a])).abs();
        if dist > best_plane_dist {
            best_plane_dist = dist;
            d = i;
        }
    }
    if d == usize::MAX || best_plane_dist < EPS {
        return Err(HavokError::InvalidInput(
            "vertices are coplanar — cannot form convex hull".to_string(),
        ));
    }

    // Step 5: build initial tetrahedron
    // Interior = centroid of {a,b,c,d}
    let interior = scale(add(add(input[a], input[b]), add(input[c], input[d])), 0.25);

    let mut facets: Vec<Facet> = vec![
        Facet::new(a, b, c, input, interior),
        Facet::new(a, b, d, input, interior),
        Facet::new(a, c, d, input, interior),
        Facet::new(b, c, d, input, interior),
    ];

    // Step 6: iteratively expand the hull
    // For each facet, assign the points visible from it.
    let n_input = input.len();
    let mut assigned: Vec<Option<usize>> = vec![None; n_input]; // which facet each point is visible from
    let mut horizon_limit = 0usize;
    loop {
        let mut hull_vertices: std::collections::HashSet<usize> = Default::default();
        for facet in &facets {
            hull_vertices.extend(facet.verts.iter().copied());
        }
        // Assign unassigned points to the first facet they're visible from
        let mut any_visible = false;
        for i in 0..n_input {
            if assigned[i].is_some() || hull_vertices.contains(&i) {
                continue;
            }
            for (fi, facet) in facets.iter().enumerate() {
                if facet.signed_dist(input[i]) > EPS {
                    assigned[i] = Some(fi);
                    any_visible = true;
                    break;
                }
            }
        }
        if !any_visible {
            break;
        }

        // Find the farthest point overall
        let mut best_pt = usize::MAX;
        let mut best_d = 0.0f32;
        for (i, &asgn) in assigned.iter().enumerate() {
            if let Some(fi) = asgn {
                let d = facets[fi].signed_dist(input[i]);
                if d > best_d {
                    best_d = d;
                    best_pt = i;
                }
            }
        }
        if best_pt == usize::MAX {
            break;
        }

        // Find all facets visible from best_pt
        let p = input[best_pt];
        let visible: Vec<usize> = facets
            .iter()
            .enumerate()
            .filter(|(_, f)| f.signed_dist(p) > EPS)
            .map(|(fi, _)| fi)
            .collect();

        // Find horizon edges: edges shared by exactly one visible facet
        let mut edge_count: std::collections::HashMap<(usize, usize), usize> = Default::default();
        for &fi in &visible {
            let f = &facets[fi];
            for k in 0..3 {
                let e0 = f.verts[k];
                let e1 = f.verts[(k + 1) % 3];
                let key = (e0.min(e1), e0.max(e1));
                *edge_count.entry(key).or_insert(0) += 1;
            }
        }
        let mut horizon: Vec<(usize, usize)> = edge_count
            .into_iter()
            .filter(|(_, count)| *count == 1)
            .map(|(e, _)| e)
            .collect();
        // Canonical horizon order: `edge_count` is a HashMap, so its iteration
        // order (and therefore the order new facets are pushed below) is random
        // per build. Sorting the horizon edges makes the facet enumeration —
        // hence the merged-face output bytes — reproducible. Geometry-neutral:
        // the final hull is identical regardless of facet construction order.
        horizon.sort_unstable();

        if horizon.is_empty() {
            break;
        }

        // Remove visible facets. After swap_remove the index map changes, so we clear
        // ALL assignments — they are re-derived at the top of the next iteration.
        let mut to_remove = visible.clone();
        to_remove.sort_unstable_by(|a, b| b.cmp(a)); // descending so indices stay valid
        for fi in &to_remove {
            facets.swap_remove(*fi);
        }

        // Clear every assignment: swap_remove moves non-removed facets to new indices,
        // making any retained index stale. Re-derive next iteration.
        for asgn in assigned.iter_mut() {
            *asgn = None;
        }

        // Add new facets connecting horizon edges to best_pt
        for (e0, e1) in &horizon {
            facets.push(Facet::new(*e0, *e1, best_pt, input, interior));
        }

        // Safety valve
        horizon_limit += 1;
        if horizon_limit > 10_000 {
            break;
        }
    }

    // Step 7: deduplicate vertices actually used by hull facets
    let mut used: std::collections::HashSet<usize> = Default::default();
    for f in &facets {
        used.extend(f.verts.iter().copied());
    }
    let mut used_sorted: Vec<usize> = used.into_iter().collect();
    used_sorted.sort_unstable();
    let hull_verts: Vec<[f32; 3]> = used_sorted.iter().map(|&i| input[i]).collect();
    let old_to_new: std::collections::HashMap<usize, usize> = used_sorted
        .iter()
        .enumerate()
        .map(|(new, &old)| (old, new))
        .collect();

    // Step 8: merge coplanar adjacent facets into polygonal faces.
    //
    // Quickhull emits triangulated facets each carrying its own plane.
    // When the source has coplanar vertex groups (e.g. flat sides of an
    // extruded box), each flat side becomes N triangles with the SAME plane
    // equation. At runtime, hknpConvexPolytopeShape builds an edge-dihedral
    // table from `faces` + `indices`; two coplanar adjacent triangles have
    // dihedral = 0 (or 180°) and a downstream SDK lookup derefs null on
    // that singular case — workshop sphere casts crash in the broadphase
    // (Fallout4.exe+13E82D0). Vanilla FO4 (and pynifly's pack_convex_polytope)
    // emit one polygon per planar face. Group facets by plane, extract the
    // boundary loop of each group, emit one face per group with the shared
    // plane. See crash-2026-05-11-16-21-53.log.
    let merged = merge_coplanar_facets(&facets, &old_to_new);

    let mut out_planes: Vec<[f32; 4]> = Vec::with_capacity(merged.len());
    let mut out_faces: Vec<(u16, u8, u8)> = Vec::with_capacity(merged.len());
    let mut out_indices: Vec<u8> = Vec::new();
    let mut out_edges: Vec<(u16, u8, u8)> = Vec::new();

    for (fi, merged_face) in merged.iter().enumerate() {
        out_planes.push(merged_face.plane);
        let first_idx = out_indices.len() as u16;
        let n = merged_face.indices.len() as u8;
        for &vi in &merged_face.indices {
            out_indices.push(vi as u8);
        }
        out_faces.push((first_idx, n, 0u8));
        for k in 0..(n as usize) {
            out_edges.push((fi as u16, k as u8, 0u8));
        }
    }

    // Step 9: per-vertex first-edge lookup
    let n_hull = hull_verts.len();
    let mut vertex_edges: Vec<u32> = vec![0u32; n_hull];
    for &(fi, k, _) in out_edges.iter() {
        let base = (fi as usize) * 3;
        let vi = out_indices[base + k as usize] as usize;
        let encoded = (fi as u32) | ((k as u32) << 16);
        // Use first encounter
        if vertex_edges[vi] == 0 {
            vertex_edges[vi] = encoded;
        }
    }

    Ok(HullTopology {
        vertices: hull_verts,
        planes: out_planes,
        faces: out_faces,
        indices: out_indices,
        edges: out_edges,
        vertex_edges,
    })
}

// ---------------------------------------------------------------------------
// Coplanar facet merging
// ---------------------------------------------------------------------------

struct MergedFace {
    /// Hull-remapped vertex indices forming the boundary polygon, CCW
    /// when viewed from the outward normal direction.
    indices: Vec<usize>,
    /// Shared plane equation [nx, ny, nz, d] (outward normal, d = -dot(n, p)).
    plane: [f32; 4],
}

/// Tolerance for treating two planes as coplanar. Normals are unit vectors so
/// a cosine threshold of ~1e-4 corresponds to ~0.8° divergence; `d` is in
/// world units (havok-scaled, so ~1.0 across an FO4 set-dressing object) and
/// the same absolute tolerance matches the angle bound.
const PLANE_EPS: f32 = 1e-4;

fn planes_equal(a: &[f32; 4], b: &[f32; 4]) -> bool {
    (a[0] - b[0]).abs() < PLANE_EPS
        && (a[1] - b[1]).abs() < PLANE_EPS
        && (a[2] - b[2]).abs() < PLANE_EPS
        && (a[3] - b[3]).abs() < PLANE_EPS
}

fn merge_coplanar_facets(
    facets: &[Facet],
    old_to_new: &std::collections::HashMap<usize, usize>,
) -> Vec<MergedFace> {
    // Group facets by plane equation. Each group's facets share the same
    // supporting plane; their union forms a single planar face of the hull.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut group_planes: Vec<[f32; 4]> = Vec::new();
    for (fi, facet) in facets.iter().enumerate() {
        let mut found = None;
        for (gi, gp) in group_planes.iter().enumerate() {
            if planes_equal(gp, &facet.plane) {
                found = Some(gi);
                break;
            }
        }
        match found {
            Some(gi) => groups[gi].push(fi),
            None => {
                groups.push(vec![fi]);
                group_planes.push(facet.plane);
            }
        }
    }

    let mut out = Vec::with_capacity(groups.len());
    for (group, plane) in groups.iter().zip(group_planes.iter()) {
        let indices = if group.len() == 1 {
            // Single triangle — emit verts as-is, remapped to hull-vertex space.
            facets[group[0]]
                .verts
                .iter()
                .map(|v| *old_to_new.get(v).unwrap())
                .collect()
        } else {
            extract_boundary_loop(facets, group, old_to_new)
        };
        out.push(MergedFace {
            indices,
            plane: *plane,
        });
    }
    out
}

/// Extract the boundary polygon of a coplanar facet group as an ordered
/// loop of hull-remapped vertex indices.
///
/// Each triangle contributes three directed half-edges (a→b, b→c, c→a) in its
/// CCW winding. An edge that is shared between two triangles in the group
/// appears once as (a→b) and once as (b→a) — these cancel as internal edges.
/// Edges that appear exactly once are boundary edges; chained tip-to-tail
/// they form the polygon.
fn extract_boundary_loop(
    facets: &[Facet],
    group: &[usize],
    old_to_new: &std::collections::HashMap<usize, usize>,
) -> Vec<usize> {
    use std::collections::HashMap;

    let remap = |v: usize| -> usize { *old_to_new.get(&v).unwrap() };

    // Collect directed half-edges with their multiplicity.
    let mut edges: HashMap<(usize, usize), i32> = HashMap::new();
    for &fi in group {
        let f = &facets[fi];
        let a = remap(f.verts[0]);
        let b = remap(f.verts[1]);
        let c = remap(f.verts[2]);
        *edges.entry((a, b)).or_insert(0) += 1;
        *edges.entry((b, c)).or_insert(0) += 1;
        *edges.entry((c, a)).or_insert(0) += 1;
    }

    // Boundary edges: those that have NO matching reverse partner in the
    // group. (a→b) is internal iff (b→a) also appears.
    //
    // `edges` is a HashMap, so `.iter()` order is random per build. When a
    // source vertex has more than one boundary edge (a pinched/non-manifold
    // boundary, which degenerate near-coplanar thin-slab groups produce), the
    // collected `a -> b` map would keep whichever pair iteration yielded last
    // — randomly rotating/reshaping the emitted face per build. Collect to a
    // sorted Vec first so the kept edge per source vertex is canonical.
    let mut boundary_edges: Vec<(usize, usize)> = edges
        .iter()
        .filter_map(|(&(a, b), &count)| {
            let reverse = edges.get(&(b, a)).copied().unwrap_or(0);
            if count > 0 && reverse == 0 {
                Some((a, b))
            } else {
                None
            }
        })
        .collect();
    boundary_edges.sort_unstable();
    let boundary: HashMap<usize, usize> = boundary_edges.into_iter().collect();

    if boundary.is_empty() {
        // Degenerate group (single edge or all internal) — fall back to
        // emitting the first triangle's verts so the polytope still has a
        // valid face. This branch should be unreachable for a well-formed
        // convex hull but guards against numerical edge cases.
        let f = &facets[group[0]];
        return vec![remap(f.verts[0]), remap(f.verts[1]), remap(f.verts[2])];
    }

    // Chain boundary edges tip-to-tail. Start at the minimum vertex index for a
    // canonical rotation of the cyclic loop: HashMap key order is random per
    // build, so an arbitrary start would emit byte-different faces. Any rotation
    // is geometrically identical.
    let &start = boundary.keys().min().unwrap();
    let mut loop_verts = vec![start];
    let mut current = start;
    loop {
        let Some(&next) = boundary.get(&current) else {
            break;
        };
        if next == start {
            break;
        }
        loop_verts.push(next);
        current = next;
        if loop_verts.len() > boundary.len() + 1 {
            // Safety valve against pathological inputs forming multiple loops.
            break;
        }
    }
    loop_verts
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
#[inline]
fn dist_sq(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = sub(a, b);
    dot(d, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Box with two coplanar triangles per face (8 verts → 12 triangles before
    /// merging) must emit exactly 6 unique planes after the coplanar-merge
    /// post-pass. Duplicate planes are the workshop-sweep CTD trigger — see
    /// crash-2026-05-11-16-21-53.log and the merge_coplanar_facets doc-comment.
    #[test]
    fn unit_cube_merges_to_six_polygonal_faces() {
        let verts: Vec<[f32; 3]> = vec![
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ];
        let hull = compute_hull_topology_robust(&verts).expect("hull built");

        assert_eq!(
            hull.faces.len(),
            6,
            "cube must collapse to 6 polygonal faces (got {})",
            hull.faces.len()
        );
        assert_eq!(
            hull.planes.len(),
            6,
            "cube must have 6 unique planes (got {})",
            hull.planes.len()
        );
        for (fi, &(_, num_indices, _)) in hull.faces.iter().enumerate() {
            assert_eq!(
                num_indices, 4,
                "cube face {fi} must be a quad (numIndices=4, got {num_indices})"
            );
        }

        // Confirm no duplicate planes survive (each pair must differ by ≥ PLANE_EPS).
        for i in 0..hull.planes.len() {
            for j in (i + 1)..hull.planes.len() {
                assert!(
                    !planes_equal(&hull.planes[i], &hull.planes[j]),
                    "plane[{i}]={:?} and plane[{j}]={:?} are duplicates",
                    hull.planes[i],
                    hull.planes[j],
                );
            }
        }
    }

    /// Bank-base verts (rectangular prism with two -y hinge bumps) — the
    /// shape that crashed the game until we added the coplanar merge pass.
    /// Asserts the merged plane count (8) matches the unique-plane count
    /// vanilla emits for this geometry.
    #[test]
    fn bank_base_hull_emits_eight_unique_planes() {
        let verts: Vec<[f32; 3]> = vec![
            [-0.734537, -0.444401, 0.000000],
            [-0.734537, 0.553338, 0.000000],
            [0.732185, 0.553338, 0.000000],
            [0.732185, -0.444401, 0.000000],
            [-0.734537, -0.444401, 1.535690],
            [0.732185, -0.444401, 1.535690],
            [0.732185, 0.553338, 1.535690],
            [-0.734537, 0.553338, 1.535690],
            [-0.648573, -0.549418, 0.000000],
            [0.646221, -0.549418, 0.000000],
            [0.646221, -0.549418, 1.535690],
            [-0.648573, -0.549418, 1.535690],
        ];
        let hull = compute_hull_topology_robust(&verts).expect("hull built");
        assert_eq!(
            hull.planes.len(),
            8,
            "bank-base hull must have 8 unique planes (got {}); pre-fix it \
             emitted 20 (triangulated facets each with their own plane), which \
             crashed the FO4 broadphase on workshop sphere casts.",
            hull.planes.len()
        );
        assert_eq!(hull.faces.len(), 8);
    }
}
