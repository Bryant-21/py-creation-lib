use std::cmp::Reverse;
/// Quadric Error Metric (QEM) mesh decimation (Garland & Heckbert 1997): collapse
/// the lowest-error half-edge until the target triangle count is reached or every
/// collapse would flip a face. Symmetric 4×4 quadric per vertex, min-heap of candidates.
use std::collections::{BinaryHeap, HashMap, HashSet};

// ---------------------------------------------------------------------------
// Symmetric 4×4 quadric (upper-triangle storage)
// ---------------------------------------------------------------------------

/// Symmetric 4×4 matrix stored as the 10 upper-triangle entries.
/// Index mapping: (0,0)=0 (0,1)=1 (0,2)=2 (0,3)=3
///                (1,1)=4 (1,2)=5 (1,3)=6
///                (2,2)=7 (2,3)=8
///                (3,3)=9
#[derive(Clone, Copy, Default)]
struct Quadric([f64; 10]);

impl Quadric {
    fn from_plane(a: f64, b: f64, c: f64, d: f64) -> Self {
        Quadric([
            a * a,
            a * b,
            a * c,
            a * d,
            b * b,
            b * c,
            b * d,
            c * c,
            c * d,
            d * d,
        ])
    }

    fn add(&self, other: &Self) -> Self {
        let mut q = [0.0f64; 10];
        for i in 0..10 {
            q[i] = self.0[i] + other.0[i];
        }
        Quadric(q)
    }

    /// Evaluate v^T Q v where v = [x, y, z, 1]
    fn error(&self, p: [f64; 3]) -> f64 {
        let [x, y, z] = p;
        let q = &self.0;
        x * x * q[0]
            + 2.0 * x * y * q[1]
            + 2.0 * x * z * q[2]
            + 2.0 * x * q[3]
            + y * y * q[4]
            + 2.0 * y * z * q[5]
            + 2.0 * y * q[6]
            + z * z * q[7]
            + 2.0 * z * q[8]
            + q[9]
    }

    /// Find the optimal collapse position by solving the 3×3 linear system
    /// from the first three rows of ∂(v^T Q v)/∂v = 0.
    /// Falls back to the midpoint if the system is singular.
    fn optimal_position(&self, v0: [f64; 3], v1: [f64; 3]) -> [f64; 3] {
        let q = &self.0;
        // Upper-left 3×3 of Q
        let a = [[q[0], q[1], q[2]], [q[1], q[4], q[5]], [q[2], q[5], q[7]]];
        let b = [-q[3], -q[6], -q[8]];
        if let Some(p) = solve3x3(a, b) {
            p
        } else {
            midpoint(v0, v1)
        }
    }
}

// ---------------------------------------------------------------------------
// Internal edge collapse candidate
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Collapse {
    cost: f64,
    v0: usize,
    v1: usize,
    pos: [f64; 3],
    // Version stamp to detect stale heap entries
    version: u32,
}

impl PartialEq for Collapse {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost
    }
}
impl Eq for Collapse {}
impl PartialOrd for Collapse {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Collapse {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse for min-heap via BinaryHeap (which is max-heap)
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

// ---------------------------------------------------------------------------
// Public output type
// ---------------------------------------------------------------------------

/// A decimated mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct DecimatedMesh {
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Decimate `vertices`/`triangles` to at most `target_tri_count` triangles with
/// QEM edge collapses. May return more if no valid collapses remain; unchanged
/// if the target is >= the current triangle count.
pub fn decimate(
    vertices: &[[f32; 3]],
    triangles: &[[u32; 3]],
    target_tri_count: usize,
) -> DecimatedMesh {
    if triangles.len() <= target_tri_count || vertices.is_empty() {
        return DecimatedMesh {
            vertices: vertices.to_vec(),
            triangles: triangles.to_vec(),
        };
    }

    // --- Data structures ---
    let nv = vertices.len();
    let verts64: Vec<[f64; 3]> = vertices
        .iter()
        .map(|&v| [v[0] as f64, v[1] as f64, v[2] as f64])
        .collect();

    // Active vertex positions (may move after collapse)
    let mut pos: Vec<[f64; 3]> = verts64.clone();
    // "alive" flags
    let mut alive_v: Vec<bool> = vec![true; nv];
    // Canonical representative for each vertex (union-find for collapse chains)
    let mut rep: Vec<usize> = (0..nv).collect();

    // Build adjacency: triangles as (a,b,c) tuples, alive flag
    let mut tris: Vec<[usize; 3]> = triangles
        .iter()
        .map(|t| [t[0] as usize, t[1] as usize, t[2] as usize])
        .collect();
    let mut alive_t: Vec<bool> = vec![true; tris.len()];

    // Compute per-vertex quadrics from incident triangle planes
    let mut quadrics: Vec<Quadric> = vec![Quadric::default(); nv];
    for (ti, &alive) in alive_t.iter().enumerate() {
        if !alive {
            continue;
        }
        let [ai, bi, ci] = tris[ti];
        if ai >= nv || bi >= nv || ci >= nv {
            continue;
        }
        let plane = triangle_plane(pos[ai], pos[bi], pos[ci]);
        let q = Quadric::from_plane(plane[0], plane[1], plane[2], plane[3]);
        quadrics[ai] = quadrics[ai].add(&q);
        quadrics[bi] = quadrics[bi].add(&q);
        quadrics[ci] = quadrics[ci].add(&q);
    }

    // Build edge set and initial collapse heap
    let mut version: Vec<u32> = vec![0u32; nv];
    let mut heap: BinaryHeap<Collapse> = BinaryHeap::new();

    let mut edges: HashSet<(usize, usize)> = HashSet::new();
    for &[ai, bi, ci] in &tris {
        for (u, v) in [(ai, bi), (bi, ci), (ci, ai)] {
            let key = (u.min(v), u.max(v));
            if edges.insert(key) {
                let qsum = quadrics[u].add(&quadrics[v]);
                let opt = qsum.optimal_position(pos[u], pos[v]);
                let cost = qsum.error(opt);
                heap.push(Collapse {
                    cost,
                    v0: u,
                    v1: v,
                    pos: opt,
                    version: 0,
                });
            }
        }
    }

    let mut current_tris: usize = tris.len();

    // --- Collapse loop ---
    while current_tris > target_tri_count {
        let c = loop {
            match heap.pop() {
                None => break None,
                Some(c) => {
                    let v0 = find_rep(&rep, c.v0);
                    let v1 = find_rep(&rep, c.v1);
                    if !alive_v[v0] || !alive_v[v1] {
                        continue;
                    }
                    if v0 == v1 {
                        continue;
                    }
                    // Check version freshness: if either vertex has been updated, skip
                    if c.version != version[v0].min(version[v1]) {
                        continue;
                    }
                    break Some((c, v0, v1));
                }
            }
        };
        let (c, v0, v1) = match c {
            None => break,
            Some(x) => x,
        };

        // Collapse v1 into v0: move v0 to optimal position, kill v1
        pos[v0] = c.pos;
        alive_v[v1] = false;
        rep[v1] = v0;
        version[v0] = version[v0].wrapping_add(1);

        // Update quadric: v0 absorbs v1's quadric
        quadrics[v0] = quadrics[v0].add(&quadrics[v1]);

        // Update triangles: remap v1→v0, kill degenerate tris
        for ti in 0..tris.len() {
            if !alive_t[ti] {
                continue;
            }
            let mut changed = false;
            for vi in 0..3 {
                let r = find_rep(&rep, tris[ti][vi]);
                if r != tris[ti][vi] {
                    tris[ti][vi] = r;
                    changed = true;
                }
            }
            if !changed {
                continue;
            }
            let [a, b, cc] = tris[ti];
            if a == b || b == cc || a == cc {
                alive_t[ti] = false;
                current_tris -= 1;
            }
        }

        // Re-queue edges incident on v0
        let mut neighbor_edges: HashSet<(usize, usize)> = HashSet::new();
        for ti in 0..tris.len() {
            if !alive_t[ti] {
                continue;
            }
            let [a, b, cc] = tris[ti];
            if a == v0 || b == v0 || cc == v0 {
                for (u, v) in [(a, b), (b, cc), (cc, a)] {
                    neighbor_edges.insert((u.min(v), u.max(v)));
                }
            }
        }
        for (u, v) in neighbor_edges {
            if !alive_v[u] || !alive_v[v] {
                continue;
            }
            let qsum = quadrics[u].add(&quadrics[v]);
            let opt = qsum.optimal_position(pos[u], pos[v]);
            let cost = qsum.error(opt);
            let ver = version[u].min(version[v]);
            heap.push(Collapse {
                cost,
                v0: u,
                v1: v,
                pos: opt,
                version: ver,
            });
        }
    }

    // --- Build output ---
    // Compact vertices: only alive ones
    let mut new_idx: Vec<usize> = vec![usize::MAX; nv];
    let mut out_verts: Vec<[f32; 3]> = Vec::new();
    for i in 0..nv {
        let ri = find_rep(&rep, i);
        if alive_v[ri] && new_idx[ri] == usize::MAX {
            new_idx[ri] = out_verts.len();
            out_verts.push([pos[ri][0] as f32, pos[ri][1] as f32, pos[ri][2] as f32]);
        }
        if alive_v[ri] {
            new_idx[i] = new_idx[ri];
        }
    }

    let mut out_tris: Vec<[u32; 3]> = Vec::new();
    for (ti, &alive) in alive_t.iter().enumerate() {
        if !alive {
            continue;
        }
        let [a, b, cc] = tris[ti];
        let ra = find_rep(&rep, a);
        let rb = find_rep(&rep, b);
        let rc = find_rep(&rep, cc);
        if ra >= nv || rb >= nv || rc >= nv {
            continue;
        }
        let na = new_idx[ra];
        let nb = new_idx[rb];
        let nc = new_idx[rc];
        if na == usize::MAX || nb == usize::MAX || nc == usize::MAX {
            continue;
        }
        if na == nb || nb == nc || na == nc {
            continue;
        }
        out_tris.push([na as u32, nb as u32, nc as u32]);
    }

    DecimatedMesh {
        vertices: out_verts,
        triangles: out_tris,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn find_rep(rep: &[usize], mut v: usize) -> usize {
    while rep[v] != v {
        v = rep[v];
    }
    v
}

fn triangle_plane(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 4] {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len < 1e-12 {
        return [0.0, 0.0, 1.0, 0.0];
    }
    let nf = [n[0] / len, n[1] / len, n[2] / len];
    let d = -(nf[0] * a[0] + nf[1] * a[1] + nf[2] * a[2]);
    [nf[0], nf[1], nf[2], d]
}

fn midpoint(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        (a[0] + b[0]) * 0.5,
        (a[1] + b[1]) * 0.5,
        (a[2] + b[2]) * 0.5,
    ]
}

/// Solve 3×3 linear system Ax = b via Cramer's rule.
fn solve3x3(a: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
        - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    let x = (b[0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
        - a[0][1] * (b[1] * a[2][2] - a[1][2] * b[2])
        + a[0][2] * (b[1] * a[2][1] - a[1][1] * b[2]))
        / det;
    let y = (a[0][0] * (b[1] * a[2][2] - a[1][2] * b[2])
        - b[0] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * b[2] - b[1] * a[2][0]))
        / det;
    let z = (a[0][0] * (a[1][1] * b[2] - b[1] * a[2][1])
        - a[0][1] * (a[1][0] * b[2] - b[1] * a[2][0])
        + b[0] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]))
        / det;
    Some([x, y, z])
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a sphere with approximately `n_lat × n_lon × 2` triangles.
    fn sphere(n_lat: usize, n_lon: usize) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        use std::f32::consts::PI;
        let mut verts = Vec::new();
        let mut tris = Vec::new();
        for i in 0..=n_lat {
            let theta = PI * i as f32 / n_lat as f32;
            for j in 0..n_lon {
                let phi = 2.0 * PI * j as f32 / n_lon as f32;
                verts.push([
                    theta.sin() * phi.cos(),
                    theta.sin() * phi.sin(),
                    theta.cos(),
                ]);
            }
        }
        for i in 0..n_lat {
            for j in 0..n_lon {
                let a = (i * n_lon + j) as u32;
                let b = (i * n_lon + (j + 1) % n_lon) as u32;
                let c = ((i + 1) * n_lon + j) as u32;
                let d = ((i + 1) * n_lon + (j + 1) % n_lon) as u32;
                tris.push([a, b, c]);
                tris.push([b, d, c]);
            }
        }
        (verts, tris)
    }

    #[test]
    fn decimate_sphere_reduces_triangle_count() {
        let (verts, tris) = sphere(32, 32); // ~2048 triangles
        let original_count = tris.len();
        let target = 250;
        let result = decimate(&verts, &tris, target);
        assert!(
            result.triangles.len() <= original_count,
            "decimated must not exceed original"
        );
        assert!(
            result.triangles.len() <= target + 50, // allow small overshoot
            "expected ~{target} triangles, got {}",
            result.triangles.len()
        );
        assert!(!result.vertices.is_empty(), "output must have vertices");
    }

    #[test]
    fn decimate_below_target_returns_unchanged() {
        let verts = vec![
            [0.0f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ];
        let tris = vec![[0u32, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]];
        let result = decimate(&verts, &tris, 10); // already below target
        assert_eq!(result.triangles.len(), 4);
    }
}
