use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::objects::geometry::LodGeometry;

#[derive(Clone, Copy, Debug)]
pub struct QemOptions {
    pub max_uv_delta: f32,
    pub min_normal_dot: f32,
    pub max_color_delta: f32,
    pub boundary_weight: f32,
    pub max_passes: usize,
}

impl Default for QemOptions {
    fn default() -> Self {
        Self {
            max_uv_delta: 0.35,
            min_normal_dot: -0.15,
            max_color_delta: 0.65,
            boundary_weight: 4.0,
            max_passes: 24,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Quadric([f64; 10]);

impl Quadric {
    fn add(&mut self, rhs: Quadric) {
        for (a, b) in self.0.iter_mut().zip(rhs.0) {
            *a += b;
        }
    }

    fn from_plane(a: f64, b: f64, c: f64, d: f64) -> Self {
        Self([
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

    fn cost(&self, p: [f32; 3]) -> f64 {
        let x = p[0] as f64;
        let y = p[1] as f64;
        let z = p[2] as f64;
        let q = self.0;
        q[0] * x * x
            + 2.0 * q[1] * x * y
            + 2.0 * q[2] * x * z
            + 2.0 * q[3] * x
            + q[4] * y * y
            + 2.0 * q[5] * y * z
            + 2.0 * q[6] * y
            + q[7] * z * z
            + 2.0 * q[8] * z
            + q[9]
    }
}

#[derive(Clone, Copy, Debug)]
struct Candidate {
    a: usize,
    b: usize,
    pos: [f32; 3],
    cost: f64,
}

pub fn decimate(geometry: &mut LodGeometry, target_triangles: usize, options: QemOptions) -> bool {
    let target_triangles = target_triangles.max(1);
    if geometry.triangles.len() <= target_triangles || geometry.vertices.len() < 4 {
        return false;
    }

    let original_triangles = geometry.triangles.len();
    let mut changed = false;
    for _ in 0..options.max_passes.max(1) {
        if geometry.triangles.len() <= target_triangles {
            break;
        }
        let quadrics = vertex_quadrics(geometry);
        let edge_counts = edge_counts(&geometry.triangles);
        let mut candidates =
            collapse_candidates(geometry, &quadrics, &edge_counts, options, target_triangles);
        if candidates.is_empty() {
            break;
        }
        candidates.sort_by(compare_candidates);
        if !collapse_pass(geometry, &candidates, target_triangles) {
            break;
        }
        changed = true;
    }

    changed && geometry.triangles.len() < original_triangles
}

fn vertex_quadrics(geometry: &LodGeometry) -> Vec<Quadric> {
    let mut quadrics = vec![Quadric::default(); geometry.vertices.len()];
    for tri in &geometry.triangles {
        let Some(q) = triangle_quadric(geometry, *tri) else {
            continue;
        };
        for &idx in tri {
            if let Some(vq) = quadrics.get_mut(idx as usize) {
                vq.add(q);
            }
        }
    }
    quadrics
}

fn triangle_quadric(geometry: &LodGeometry, tri: [u32; 3]) -> Option<Quadric> {
    let a = geometry.vertices.get(tri[0] as usize).copied()?;
    let b = geometry.vertices.get(tri[1] as usize).copied()?;
    let c = geometry.vertices.get(tri[2] as usize).copied()?;
    let n = normalize(cross(sub(b, a), sub(c, a)))?;
    let d = -dot(n, a) as f64;
    Some(Quadric::from_plane(
        n[0] as f64,
        n[1] as f64,
        n[2] as f64,
        d,
    ))
}

fn edge_counts(triangles: &[[u32; 3]]) -> HashMap<(usize, usize), usize> {
    let mut counts = HashMap::new();
    for tri in triangles {
        for (a, b) in tri_edges(*tri) {
            *counts.entry(ordered_edge(a, b)).or_insert(0) += 1;
        }
    }
    counts
}

fn collapse_candidates(
    geometry: &LodGeometry,
    quadrics: &[Quadric],
    edge_counts: &HashMap<(usize, usize), usize>,
    options: QemOptions,
    target_triangles: usize,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for tri in &geometry.triangles {
        for (a, b) in tri_edges(*tri) {
            let (a, b) = ordered_edge(a, b);
            if !seen.insert((a, b)) || !collapse_allowed(geometry, a, b, options) {
                continue;
            }
            let qa = quadrics.get(a).copied().unwrap_or_default();
            let qb = quadrics.get(b).copied().unwrap_or_default();
            let mut q = qa;
            q.add(qb);
            let (pos, mut cost) = best_position(geometry.vertices[a], geometry.vertices[b], q);
            if edge_counts.get(&(a, b)).copied().unwrap_or(0) == 1 {
                cost *= options.boundary_weight.max(1.0) as f64;
            }
            if geometry.triangles.len() - out.len().min(geometry.triangles.len())
                <= target_triangles
            {
                cost *= 1.25;
            }
            out.push(Candidate { a, b, pos, cost });
        }
    }
    out
}

fn collapse_allowed(geometry: &LodGeometry, a: usize, b: usize, options: QemOptions) -> bool {
    if a == b || a >= geometry.vertices.len() || b >= geometry.vertices.len() {
        return false;
    }
    if has_parallel_array(&geometry.uvcoords, geometry.vertices.len()) {
        let uv_a = geometry.uvcoords[a];
        let uv_b = geometry.uvcoords[b];
        if dist2_2d(uv_a, uv_b).sqrt() > options.max_uv_delta.max(0.0) {
            return false;
        }
    }
    if has_parallel_array(&geometry.normals, geometry.vertices.len()) {
        let na = normalize(geometry.normals[a]).unwrap_or([0.0, 0.0, 1.0]);
        let nb = normalize(geometry.normals[b]).unwrap_or([0.0, 0.0, 1.0]);
        if dot(na, nb) < options.min_normal_dot.clamp(-1.0, 1.0) {
            return false;
        }
    }
    if has_parallel_array(&geometry.vertex_colors, geometry.vertices.len())
        && color_delta(geometry.vertex_colors[a], geometry.vertex_colors[b])
            > options.max_color_delta.max(0.0)
    {
        return false;
    }
    true
}

fn best_position(a: [f32; 3], b: [f32; 3], q: Quadric) -> ([f32; 3], f64) {
    let mid = [
        (a[0] + b[0]) * 0.5,
        (a[1] + b[1]) * 0.5,
        (a[2] + b[2]) * 0.5,
    ];
    let mut best = (a, q.cost(a));
    let cb = q.cost(b);
    if cb < best.1 {
        best = (b, cb);
    }
    let cm = q.cost(mid);
    if cm < best.1 {
        best = (mid, cm);
    }
    best
}

fn compare_candidates(a: &Candidate, b: &Candidate) -> Ordering {
    a.cost
        .total_cmp(&b.cost)
        .then_with(|| a.a.cmp(&b.a))
        .then_with(|| a.b.cmp(&b.b))
}

fn collapse_pass(
    geometry: &mut LodGeometry,
    candidates: &[Candidate],
    target_triangles: usize,
) -> bool {
    let n = geometry.vertices.len();
    let mut parent: Vec<usize> = (0..n).collect();
    let mut used = vec![false; n];
    let mut collapsed = 0usize;

    for candidate in candidates {
        if geometry.triangles.len().saturating_sub(collapsed) <= target_triangles {
            break;
        }
        let a = find(&mut parent, candidate.a);
        let b = find(&mut parent, candidate.b);
        if a == b || used[a] || used[b] {
            continue;
        }
        let keep = a.min(b);
        let drop = a.max(b);
        parent[drop] = keep;
        used[keep] = true;
        used[drop] = true;
        write_merged_vertex(geometry, keep, drop, candidate.pos);
        collapsed += 1;
    }

    if collapsed == 0 {
        return false;
    }

    let mut new_triangles = Vec::with_capacity(geometry.triangles.len());
    let mut seen = HashSet::new();
    for tri in &geometry.triangles {
        let t = [
            find(&mut parent, tri[0] as usize) as u32,
            find(&mut parent, tri[1] as usize) as u32,
            find(&mut parent, tri[2] as usize) as u32,
        ];
        if t[0] == t[1] || t[1] == t[2] || t[0] == t[2] {
            continue;
        }
        if triangle_area2(
            geometry.vertices[t[0] as usize],
            geometry.vertices[t[1] as usize],
            geometry.vertices[t[2] as usize],
        ) <= 1e-10
        {
            continue;
        }
        let mut key = t;
        key.sort_unstable();
        if seen.insert(key) {
            new_triangles.push(t);
        }
    }

    if new_triangles.len() == geometry.triangles.len() {
        return false;
    }
    geometry.triangles = new_triangles;
    geometry.remove_unused();
    geometry.update_bbox();
    true
}

fn write_merged_vertex(geometry: &mut LodGeometry, keep: usize, drop: usize, pos: [f32; 3]) {
    geometry.vertices[keep] = pos;
    if has_parallel_array(&geometry.uvcoords, geometry.vertices.len()) {
        geometry.uvcoords[keep] = avg2(geometry.uvcoords[keep], geometry.uvcoords[drop]);
    }
    if has_parallel_array(&geometry.normals, geometry.vertices.len()) {
        geometry.normals[keep] = normalize(avg3(geometry.normals[keep], geometry.normals[drop]))
            .unwrap_or(geometry.normals[keep]);
    }
    if has_parallel_array(&geometry.tangents, geometry.vertices.len()) {
        geometry.tangents[keep] = normalize(avg3(geometry.tangents[keep], geometry.tangents[drop]))
            .unwrap_or(geometry.tangents[keep]);
    }
    if has_parallel_array(&geometry.bitangents, geometry.vertices.len()) {
        geometry.bitangents[keep] =
            normalize(avg3(geometry.bitangents[keep], geometry.bitangents[drop]))
                .unwrap_or(geometry.bitangents[keep]);
    }
    if has_parallel_array(&geometry.vertex_colors, geometry.vertices.len()) {
        geometry.vertex_colors[keep] =
            avg4(geometry.vertex_colors[keep], geometry.vertex_colors[drop]);
    }
}

fn find(parent: &mut [usize], x: usize) -> usize {
    let mut root = x;
    while parent[root] != root {
        root = parent[root];
    }
    let mut cur = x;
    while parent[cur] != cur {
        let next = parent[cur];
        parent[cur] = root;
        cur = next;
    }
    root
}

fn tri_edges(tri: [u32; 3]) -> [(usize, usize); 3] {
    [
        (tri[0] as usize, tri[1] as usize),
        (tri[1] as usize, tri[2] as usize),
        (tri[2] as usize, tri[0] as usize),
    ]
}

fn ordered_edge(a: usize, b: usize) -> (usize, usize) {
    if a <= b { (a, b) } else { (b, a) }
}

fn has_parallel_array<T>(array: &[T], vertex_count: usize) -> bool {
    array.len() == vertex_count
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = dot(v, v).sqrt();
    if len <= f32::EPSILON || !len.is_finite() {
        return None;
    }
    Some([v[0] / len, v[1] / len, v[2] / len])
}

fn dist2_2d(a: [f32; 2], b: [f32; 2]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    dx * dx + dy * dy
}

fn color_delta(a: [f32; 4], b: [f32; 4]) -> f32 {
    ((a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs()) / 3.0
}

fn avg2(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
}

fn avg3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        (a[0] + b[0]) * 0.5,
        (a[1] + b[1]) * 0.5,
        (a[2] + b[2]) * 0.5,
    ]
}

fn avg4(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        (a[0] + b[0]) * 0.5,
        (a[1] + b[1]) * 0.5,
        (a[2] + b[2]) * 0.5,
        (a[3] + b[3]) * 0.5,
    ]
}

fn triangle_area2(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    dot(cross(sub(b, a), sub(c, a)), cross(sub(b, a), sub(c, a)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::BBox;

    fn grid(size: usize) -> LodGeometry {
        let mut g = LodGeometry::new();
        for y in 0..=size {
            for x in 0..=size {
                let fx = x as f32 / size as f32;
                let fy = y as f32 / size as f32;
                g.vertices.push([x as f32, y as f32, 0.0]);
                g.uvcoords.push([fx, fy]);
                g.normals.push([0.0, 0.0, 1.0]);
                g.tangents.push([1.0, 0.0, 0.0]);
                g.bitangents.push([0.0, 1.0, 0.0]);
                g.vertex_colors.push([1.0, 1.0, 1.0, 1.0]);
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
        g.bbox = BBox::empty();
        g.update_bbox();
        g
    }

    #[test]
    fn qem_decimates_subdivided_plane_to_target() {
        let mut g = grid(8);
        let before = g.triangles.len();
        assert!(decimate(
            &mut g,
            32,
            QemOptions {
                max_uv_delta: 0.5,
                ..QemOptions::default()
            }
        ));
        assert!(g.triangles.len() <= 32, "{} > 32", g.triangles.len());
        assert!(g.triangles.len() < before);
        assert_eq!(g.vertices.len(), g.uvcoords.len());
        assert_eq!(g.vertices.len(), g.normals.len());
        assert_eq!(g.vertices.len(), g.vertex_colors.len());
    }

    #[test]
    fn qem_respects_uv_guard() {
        let mut g = grid(3);
        let before = g.triangles.len();
        let changed = decimate(
            &mut g,
            1,
            QemOptions {
                max_uv_delta: 0.01,
                ..QemOptions::default()
            },
        );
        assert!(!changed);
        assert_eq!(g.triangles.len(), before);
    }

    #[test]
    fn qem_is_deterministic() {
        let mut a = grid(6);
        let mut b = grid(6);
        let options = QemOptions {
            max_uv_delta: 0.5,
            ..QemOptions::default()
        };
        decimate(&mut a, 24, options);
        decimate(&mut b, 24, options);
        assert_eq!(a.vertices, b.vertices);
        assert_eq!(a.uvcoords, b.uvcoords);
        assert_eq!(a.triangles, b.triangles);
    }
}
