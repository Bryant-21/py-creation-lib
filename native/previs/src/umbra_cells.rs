//! CK Umbra's cell optimizer, modes 1 (box growth) and 2 (pairing), as read
//! from CK 1.11.240 (dispatcher 0x18bf910;
//! `tools/re/projects/ck_previs_precombine/HOW_PREVIS_WORKS.md` §9). A set of
//! cells merges while a camera on its boundary sees most of the rest of that
//! boundary: `cost = blocked / sampled × boundary area ≤ p²`, sampled with
//! float32 Halton points exactly as CK does.

use std::sync::OnceLock;

use crate::f32ops::Vec3;

/// An axis-aligned rect on a plane. `face` is `axis * 2 + side`, as on
/// portals: the side its cell leaves through. `u` and `v` span axes
/// `(axis + 1) % 3` and `(axis + 2) % 3`, CK's order, which its sampling
/// depends on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub face: u8,
    pub plane: f32,
    pub u: [f32; 2],
    pub v: [f32; 2],
}

/// A cell of one tile: its open rects on the tile's faces (footprints) and
/// its portals to other cells of the tile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub footprints: Vec<Rect>,
    pub portals: Vec<(Rect, u32)>,
}

const BOX_GROWTH_SAMPLES: u32 = 200;
const PAIRING_SAMPLES: u32 = 1000;
const BOX_GROWTH_PASSES: usize = 100;

type Bounds = (Vec3, Vec3);

impl Rect {
    fn axis(&self) -> usize {
        (self.face >> 1) as usize
    }

    fn area(&self) -> f32 {
        (self.v[1] - self.v[0]) * (self.u[1] - self.u[0])
    }

    fn turned(self) -> Rect {
        Rect { face: self.face ^ 1, ..self }
    }

    fn point(&self, a: f32, b: f32) -> Vec3 {
        let axis = self.axis();
        let mut p = [0.0; 3];
        p[axis] = self.plane;
        p[(axis + 1) % 3] = (self.u[1] - self.u[0]) * a + self.u[0];
        p[(axis + 2) % 3] = (self.v[1] - self.v[0]) * b + self.v[0];
        p
    }

    fn bounds(&self) -> Bounds {
        let axis = self.axis();
        let (mut lo, mut hi) = ([self.plane; 3], [self.plane; 3]);
        (lo[(axis + 1) % 3], hi[(axis + 1) % 3]) = (self.u[0], self.u[1]);
        (lo[(axis + 2) % 3], hi[(axis + 2) % 3]) = (self.v[0], self.v[1]);
        (lo, hi)
    }

    /// Whether `p` is strictly on the side a turned rect faces.
    fn faces(&self, p: &Vec3) -> bool {
        let x = p[self.axis()];
        if self.face & 1 == 1 { x > self.plane } else { x < self.plane }
    }

    /// CK's portal crossing test (0x18c5e20, 0x18ba5d0): the line through
    /// `p` and `q`, not the segment, against the closed rect.
    fn crosses(&self, p: &Vec3, q: &Vec3) -> bool {
        let axis = self.axis();
        let (ua, va) = ((axis + 1) % 3, (axis + 2) % 3);
        if p[axis] == self.plane || q[axis] == self.plane {
            return false;
        }
        let code = |x: &Vec3| {
            let u = if self.u[0] > x[ua] { 1 } else if x[ua] > self.u[1] { 4 } else { 0 };
            let v = if self.v[0] > x[va] { 2 } else if x[va] > self.v[1] { 8 } else { 0 };
            u | v
        };
        let (from, to) = (code(p), code(q));
        if from == 0 && to == 0 {
            return true;
        }
        if from & to != 0 {
            return false;
        }
        let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
        if d[axis] == 0.0 {
            return false;
        }
        let t = (self.plane - p[axis]) / d[axis];
        let (mut lo, mut hi) = (t, t);
        for (k, [a, b]) in [(ua, self.u), (va, self.v)] {
            if d[k] != 0.0 {
                let (ta, tb) = ((a - p[k]) / d[k], (b - p[k]) / d[k]);
                lo = lo.max(ta.min(tb));
                hi = hi.min(ta.max(tb));
            }
        }
        hi >= lo
    }
}

/// Radical inverse of `s` in base `b`, accumulated in float32 as CK does.
fn radical(mut s: u32, b: u32) -> f32 {
    let (mut f, mut base) = (0.0f32, b);
    while s > 0 {
        f += (s % b) as f32 / base as f32;
        s /= b;
        base *= b;
    }
    f
}

/// Halton points in bases 2, 3, 5, 7 and 11 for samples 0..=1000.
fn halton() -> &'static [[f32; 5]] {
    static TABLE: OnceLock<Vec<[f32; 5]>> = OnceLock::new();
    TABLE.get_or_init(|| (0..=PAIRING_SAMPLES).map(|s| [2, 3, 5, 7, 11].map(|b| radical(s, b))).collect())
}

/// Picks rects in proportion to their area.
struct Picker {
    cumulative: Vec<f64>,
}

impl Picker {
    fn new(rects: &[Rect]) -> Self {
        let mut sum = 0.0;
        Picker { cumulative: rects.iter().map(|r| { sum += r.area() as f64; sum }).collect() }
    }

    fn pick(&self, at: f32) -> usize {
        let n = self.cumulative.len();
        let target = at as f64 * self.cumulative[n - 1];
        let index = self.cumulative.partition_point(|&c| c < target);
        if index < n { index } else { ((n as f64 * at as f64) as usize).min(n - 1) }
    }
}

fn sample_point(s: u32, samples: u32) -> (f32, [f32; 5]) {
    (((s - 1) as f32 + 0.5) / samples as f32, halton()[s as usize])
}

fn portal_box(graph: &[Node], cells: impl IntoIterator<Item = u32>) -> Bounds {
    let mut out = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for cell in cells {
        for (rect, _) in &graph[cell as usize].portals {
            let (lo, hi) = rect.bounds();
            for k in 0..3 {
                out.0[k] = out.0[k].min(lo[k]);
                out.1[k] = out.1[k].max(hi[k]);
            }
        }
    }
    out
}

/// Closed overlap on all three axes and positive overlap on at least two.
fn touches(rect: &Rect, bounds: &Bounds) -> bool {
    let (lo, hi) = rect.bounds();
    let (mut closed, mut strict) = (0, 0);
    for k in 0..3 {
        if hi[k] >= bounds.0[k] && bounds.1[k] >= lo[k] {
            closed += 1;
            strict += usize::from(hi[k] > bounds.0[k] && bounds.1[k] > lo[k]);
        }
    }
    closed == 3 && strict >= 2
}

/// The unassigned cells reached from `seed` through portals touching
/// `bounds` that do not lie on its face on their side, unless their face is
/// `open` (0x18b64f0). Sorted.
fn flood(graph: &[Node], seed: u32, open: Option<u8>, bounds: &Bounds, assigned: &[bool]) -> Vec<u32> {
    let mut inside = vec![false; graph.len()];
    inside[seed as usize] = true;
    let (mut out, mut stack) = (vec![seed], vec![seed]);
    while let Some(cell) = stack.pop() {
        for &(rect, target) in &graph[cell as usize].portals {
            if assigned[target as usize] || inside[target as usize] || !touches(&rect, bounds) {
                continue;
            }
            let edge = if rect.face & 1 == 0 { bounds.0[rect.axis()] } else { bounds.1[rect.axis()] };
            if open == Some(rect.face) || edge != rect.plane {
                inside[target as usize] = true;
                out.push(target);
                stack.push(target);
            }
        }
    }
    out.sort_unstable();
    out
}

struct Walk {
    seen: Vec<bool>,
    touched: Vec<u32>,
    stack: Vec<u32>,
}

impl Walk {
    fn new(cells: usize) -> Self {
        Walk { seen: vec![false; cells], touched: Vec::new(), stack: Vec::new() }
    }

    /// Whether the line `p`–`q` gets from cell `from` to cell `to` through
    /// portals between cells `inside` the set.
    fn visible(&mut self, graph: &[Node], (from, p): (u32, Vec3), (to, q): (u32, Vec3), inside: &[bool]) -> bool {
        if from == to {
            return true;
        }
        self.seen[from as usize] = true;
        self.touched.push(from);
        self.stack.push(from);
        let mut found = false;
        'walk: while let Some(cell) = self.stack.pop() {
            for &(rect, target) in &graph[cell as usize].portals {
                if !inside[target as usize] || self.seen[target as usize] || !rect.crosses(&p, &q) {
                    continue;
                }
                if target == to {
                    found = true;
                    break 'walk;
                }
                self.seen[target as usize] = true;
                self.touched.push(target);
                self.stack.push(target);
            }
        }
        for cell in self.touched.drain(..) {
            self.seen[cell as usize] = false;
        }
        self.stack.clear();
        found
    }
}

/// Mode 1's cost of `cells` (0x18b9f30): its footprints and its portals out,
/// turned to face into it.
fn set_cost(graph: &[Node], cells: &[u32], walk: &mut Walk) -> f32 {
    let mut inside = vec![false; graph.len()];
    for &cell in cells {
        inside[cell as usize] = true;
    }
    let mut faces: Vec<(Rect, u32)> = Vec::new();
    for &cell in cells {
        let node = &graph[cell as usize];
        faces.extend(node.footprints.iter().map(|r| (r.turned(), cell)));
        faces.extend(node.portals.iter().filter(|(_, t)| !inside[*t as usize]).map(|(r, _)| (r.turned(), cell)));
    }
    if faces.is_empty() {
        return 0.0;
    }
    let rects: Vec<Rect> = faces.iter().map(|f| f.0).collect();
    let total = rects.iter().fold(0.0f32, |sum, r| sum + r.area());
    let picker = Picker::new(&rects);
    let (mut blocked, mut seen) = (0u32, 0u32);
    for s in 1..=BOX_GROWTH_SAMPLES {
        let (at, [h2, h3, h5, h7, h11]) = sample_point(s, BOX_GROWTH_SAMPLES);
        let (i, j) = (picker.pick(at), picker.pick(h2));
        let ((first, from), (second, to)) = (faces[i], faces[j]);
        if i == j || (first.face == second.face && first.plane == second.plane) {
            continue;
        }
        let (p, q) = (first.point(h3, h5), second.point(h7, h11));
        if first.faces(&q) && second.faces(&p) && walk.visible(graph, (from, p), (to, q), &inside) {
            seen += 1;
        } else {
            blocked += 1;
        }
    }
    if blocked + seen == 0 {
        return 0.0;
    }
    blocked as f32 / (blocked + seen) as f32 * total
}

/// Mode 2's cost of joining cells `a` and `b` (0x18ba5d0): lines from either
/// one's boundary to the other's that miss every portal between them.
fn pair_cost(graph: &[Node], a: u32, b: u32) -> f32 {
    let (mut from, mut to, mut between) = (Vec::new(), Vec::new(), Vec::new());
    let (na, nb) = (&graph[a as usize], &graph[b as usize]);
    from.extend(na.footprints.iter().map(|r| r.turned()));
    for &(rect, target) in &na.portals {
        if target == b { between.push(rect) } else { from.push(rect.turned()) }
    }
    to.extend(nb.footprints.iter().map(|r| r.turned()));
    to.extend(nb.portals.iter().filter(|(_, t)| *t != a).map(|(r, _)| r.turned()));
    if from.is_empty() || to.is_empty() {
        return 0.0;
    }
    let (pick_from, pick_to) = (Picker::new(&from), Picker::new(&to));
    let (mut blocked, mut seen) = (0u32, 0u32);
    for s in 1..=PAIRING_SAMPLES {
        let (at, [h2, h3, h5, h7, h11]) = sample_point(s, PAIRING_SAMPLES);
        let (first, second) = (from[pick_from.pick(at)], to[pick_to.pick(h2)]);
        if first.face == second.face && first.plane == second.plane {
            continue;
        }
        let (p, q) = (first.point(h3, h5), second.point(h7, h11));
        if between.iter().any(|r| r.crosses(&p, &q)) {
            seen += 1;
        } else {
            blocked += 1;
        }
    }
    if blocked + seen == 0 {
        return 0.0;
    }
    let total = from.iter().chain(&to).fold(0.0f32, |sum, r| sum + r.area());
    blocked as f32 * total / (blocked + seen) as f32
}

/// Mode 1 (0x18bc000): each unassigned cell in order grows a box of cells
/// one face at a time while the grown set's cost stays at most `p²`.
/// Returns each cell's group and the group count.
fn box_growth(graph: &[Node], p: f32) -> (Vec<u32>, u32) {
    let n = graph.len();
    let (mut group, mut assigned, mut groups) = (vec![0; n], vec![false; n], 0);
    let mut walk = Walk::new(n);
    for seed in 0..n as u32 {
        if assigned[seed as usize] {
            continue;
        }
        let mut bounds = portal_box(graph, [seed]);
        let mut current = vec![false; n];
        let mut size = 0;
        for cell in flood(graph, seed, None, &bounds, &assigned) {
            current[cell as usize] = true;
            size += 1;
        }
        for _ in 0..BOX_GROWTH_PASSES {
            let before = size;
            for face in 0..6 {
                let grown = flood(graph, seed, Some(face), &bounds, &assigned);
                if set_cost(graph, &grown, &mut walk) <= p * p {
                    // CK boxes the set as it was before this step.
                    bounds = portal_box(graph, (0..n as u32).filter(|&c| current[c as usize]));
                    for cell in grown {
                        size += usize::from(!std::mem::replace(&mut current[cell as usize], true));
                    }
                }
            }
            if size == before {
                break;
            }
        }
        for cell in (0..n).filter(|&c| current[c]) {
            group[cell] = groups;
            assigned[cell] = true;
        }
        groups += 1;
    }
    (group, groups)
}

/// Mode 2: each unassigned cell in order joins the first unassigned cell
/// behind one of its portals (only those on `seam`, `(axis, plane)`, when
/// given) whose pair cost is under `p²`. Pairs only.
fn pairing(graph: &[Node], p: f32, seam: Option<(usize, f32)>) -> (Vec<u32>, u32) {
    let n = graph.len();
    let (mut group, mut assigned, mut groups) = (vec![0; n], vec![false; n], 0);
    for a in 0..n {
        if assigned[a] {
            continue;
        }
        group[a] = groups;
        assigned[a] = true;
        for &(rect, target) in &graph[a].portals {
            let on_seam = seam.is_none_or(|(axis, plane)| rect.axis() == axis && rect.plane == plane);
            if assigned[target as usize] || !on_seam {
                continue;
            }
            if pair_cost(graph, a as u32, target) < p * p {
                group[target as usize] = groups;
                assigned[target as usize] = true;
                break;
            }
        }
        groups += 1;
    }
    (group, groups)
}

/// The graph of `group`'s groups: their members' footprints, and the portals
/// leading out of them.
fn grouped(graph: &[Node], group: &[u32], count: u32) -> Vec<Node> {
    let mut out = vec![Node::default(); count as usize];
    for (cell, node) in graph.iter().enumerate() {
        let into = &mut out[group[cell] as usize];
        into.footprints.extend_from_slice(&node.footprints);
        into.portals.extend(node.portals.iter().filter(|(_, t)| group[*t as usize] != group[cell]).map(|&(r, t)| (r, group[t as usize])));
    }
    out
}

/// What `ComputeTile` does to one input tile's cells (0x1854200): mode 1,
/// then mode 2, both at `p`. Returns each cell's new label, the label count
/// and the merged graph.
pub fn optimize_tile(graph: &[Node], p: f32) -> (Vec<u32>, u32, Vec<Node>) {
    let (first, count) = box_growth(graph, p);
    let grown = grouped(graph, &first, count);
    let (second, count) = pairing(&grown, p, None);
    let labels = first.iter().map(|&g| second[g as usize]).collect();
    (labels, count, grouped(&grown, &second, count))
}

/// The collapse's merge of two tiles on a seam (0x1879cb0): mode 2 at `p`
/// over the portals on the seam plane.
pub fn merge_seam(graph: &[Node], p: f32, axis: usize, plane: f32) -> (Vec<u32>, u32, Vec<Node>) {
    let (labels, count) = pairing(graph, p, Some((axis, plane)));
    let merged = grouped(graph, &labels, count);
    (labels, count, merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(face: u8, plane: f32, u: [f32; 2], v: [f32; 2]) -> Rect {
        Rect { face, plane, u, v }
    }

    /// Cells of `n` × 1 × 1 boxes of edge `edge` along x, open on every tile
    /// face, with a portal of `hole` × `hole` (at the corner) between
    /// neighbours.
    fn row(n: usize, edge: f32, hole: f32) -> Vec<Node> {
        (0..n)
            .map(|i| {
                let (x0, x1) = (i as f32 * edge, (i + 1) as f32 * edge);
                // y faces span (z, x); z faces span (x, y).
                let mut footprints = vec![
                    rect(2, 0.0, [0.0, edge], [x0, x1]),
                    rect(3, edge, [0.0, edge], [x0, x1]),
                    rect(4, 0.0, [x0, x1], [0.0, edge]),
                    rect(5, edge, [x0, x1], [0.0, edge]),
                ];
                let mut portals = Vec::new();
                if i == 0 {
                    footprints.push(rect(0, 0.0, [0.0, edge], [0.0, edge]));
                } else {
                    portals.push((rect(0, x0, [0.0, hole], [0.0, hole]), i as u32 - 1));
                }
                if i == n - 1 {
                    footprints.push(rect(1, x1, [0.0, edge], [0.0, edge]));
                } else {
                    portals.push((rect(1, x1, [0.0, hole], [0.0, hole]), i as u32 + 1));
                }
                Node { footprints, portals }
            })
            .collect()
    }

    #[test]
    fn halton_points_are_radical_inverses() {
        assert_eq!([radical(1, 2), radical(2, 2), radical(3, 2), radical(6, 2)], [0.5, 0.25, 0.75, 0.375]);
        assert_eq!(radical(1, 3), 1.0 / 3.0);
        assert_eq!(halton()[5], [radical(5, 2), radical(5, 3), radical(5, 5), radical(5, 7), radical(5, 11)]);
    }

    #[test]
    fn lines_cross_rects_they_pass_through() {
        let r = rect(0, 10.0, [0.0, 4.0], [0.0, 4.0]);
        assert!(r.crosses(&[0.0, 1.0, 1.0], &[20.0, 3.0, 3.0]));
        assert!(!r.crosses(&[0.0, 5.0, 1.0], &[20.0, 9.0, 1.0]), "passes beside it");
        assert!(!r.crosses(&[10.0, 1.0, 1.0], &[20.0, 1.0, 1.0]), "starts on its plane");
    }

    #[test]
    fn open_space_merges_and_a_keyhole_does_not() {
        let open = row(4, 128.0, 128.0);
        let (labels, count, merged) = optimize_tile(&open, 128.0);
        assert_eq!((labels, count), (vec![0; 4], 1));
        assert!(merged[0].portals.is_empty() && merged[0].footprints.len() == 18);

        let keyhole = row(2, 512.0, 16.0);
        let (labels, count, _) = optimize_tile(&keyhole, 128.0);
        assert_eq!((labels, count), (vec![0, 1], 2), "a 16-unit hole hides most of each side");
    }

    #[test]
    fn a_seam_merge_pairs_only_across_the_seam() {
        let graph = row(4, 128.0, 128.0);
        let (labels, count, _) = merge_seam(&graph, 64.0, 0, 256.0);
        assert_eq!((labels, count), (vec![0, 1, 1, 2], 3));
    }
}
