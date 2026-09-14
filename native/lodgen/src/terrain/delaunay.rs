/// S-hull flip-based Delaunay triangulator.
///
/// Port of `DelaunayTriangulator/Triangulator.cs` (the `Triangulation` method and
/// all its dependencies: `Triad`, `Hull`/`HullVertex`, `Vertex`, `Set`).
///
/// **Usage:** call [`triangulate`] with a slice of [`Vertex`] points (≥ 3).  The function
/// returns a `Vec<Triad>` where each triad holds 3 vertex indices into the input slice.
///
/// **Scope:** the object-LOD fan collapse in `objects::geometry`
/// (`Geometry.CreateTriangles`) calls [`convex_hull_len`] and
/// [`triangulate_reject_dups`]. Terrain water is a plain 2-triangle quad per cell and
/// does not use this; xLODGen's `specialWater` surface is not ported.
///
/// **Determinism:** the C# `Set<int>` uses a `SortedList` (keys sorted by default integer
/// comparator), which gives a deterministic iteration order, replicated here with a
/// `BTreeSet<usize>`.  No random seeding in this algorithm.

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A 2D point fed to the triangulator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub x: f32,
    pub y: f32,
}

impl Vertex {
    fn distance2_to(&self, other: &Vertex) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx * dx + dy * dy
    }
}

/// One triangle in the output mesh; `a`, `b`, `c` are indices into the input `points` slice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Triad {
    pub a: usize,
    pub b: usize,
    pub c: usize,
    // Adjacency: index of neighboring triad sharing the ab/bc/ac edge, or usize::MAX.
    ab: usize, // neighbor sharing edge a-b
    bc: usize, // neighbor sharing edge b-c
    ac: usize, // neighbor sharing edge a-c
    // Circumcircle
    circ_x: f32,
    circ_y: f32,
    circ_r2: f32,
}

const NO_NEIGHBOR: usize = usize::MAX;

impl Triad {
    fn new(a: usize, b: usize, c: usize) -> Self {
        Triad {
            a,
            b,
            c,
            ab: NO_NEIGHBOR,
            bc: NO_NEIGHBOR,
            ac: NO_NEIGHBOR,
            circ_x: 0.0,
            circ_y: 0.0,
            circ_r2: -1.0,
        }
    }

    fn new_with_adj(a: usize, b: usize, c: usize, ab: usize, bc: usize, ac: usize) -> Self {
        Triad {
            a,
            b,
            c,
            ab,
            bc,
            ac,
            circ_x: 0.0,
            circ_y: 0.0,
            circ_r2: -1.0,
        }
    }

    /// Initialize with new vertex/adjacency data and recompute circumcircle.
    fn initialize(
        &mut self,
        a: usize,
        b: usize,
        c: usize,
        ab: usize,
        bc: usize,
        ac: usize,
        points: &[Vertex],
    ) {
        self.a = a;
        self.b = b;
        self.c = c;
        self.ab = ab;
        self.bc = bc;
        self.ac = ac;
        self.find_circumcircle_precisely(points);
    }

    /// Compute circumcircle (Triad.cs `FindCircumcirclePrecisely`, using f64 for precision).
    /// Returns `true` if the circumcircle is valid; sets `circ_r2 = -1` when degenerate.
    fn find_circumcircle_precisely(&mut self, points: &[Vertex]) -> bool {
        let va = points[self.a];
        let vb = points[self.b];
        let vc = points[self.c];
        let ax = (vb.x - va.x) as f64;
        let ay = (vb.y - va.y) as f64;
        let bx = (vc.x - va.x) as f64;
        let by = (vc.y - va.y) as f64;
        let d = ax * by - ay * bx;
        if d == 0.0 {
            self.circ_x = 0.0;
            self.circ_y = 0.0;
            self.circ_r2 = -1.0;
            return false;
        }
        let d2 = ax * ax + ay * ay;
        let e2 = bx * bx + by * by;
        let inv2d = 0.5 / d;
        let cx = (by * d2 - ay * e2) * inv2d;
        let cy = (ax * e2 - bx * d2) * inv2d;
        let r2 = cx * cx + cy * cy;
        // Degenerate check from C# (Triad.cs:143-148)
        if r2 > 10_000_000_000.0 * d2 || r2 > 10_000_000_000.0 * e2 {
            self.circ_x = 0.0;
            self.circ_y = 0.0;
            self.circ_r2 = -1.0;
            return false;
        }
        self.circ_r2 = r2 as f32;
        self.circ_x = (va.x as f64 + cx) as f32;
        self.circ_y = (va.y as f64 + cy) as f32;
        true
    }

    /// Whether `p` lies strictly inside the circumcircle (Triad.cs `InsideCircumcircle`).
    fn inside_circumcircle(&self, p: &Vertex) -> bool {
        let dx = self.circ_x - p.x;
        let dy = self.circ_y - p.y;
        dx * dx + dy * dy < self.circ_r2
    }

    /// Ensure vertices are in clockwise winding (Triad.cs `MakeClockwise`).
    fn make_clockwise(&mut self, points: &[Vertex]) {
        let pa = points[self.a];
        let pb = points[self.b];
        let pc = points[self.c];
        let cx = (pa.x + pb.x + pc.x) / 3.0;
        let cy = (pa.y + pb.y + pc.y) / 3.0;
        let ax = pa.x - cx;
        let ay = pa.y - cy;
        let bx = pb.x - pa.x;
        let by = pb.y - pa.y;
        let cross = -bx * ay + by * ax;
        if cross > 0.0 {
            // swap b and c (and corresponding adjacencies)
            std::mem::swap(&mut self.b, &mut self.c);
            std::mem::swap(&mut self.ab, &mut self.ac);
        }
    }

    /// Update adjacency from `from_index` to `to_index` (Triad.cs `ChangeAdjacentIndex`).
    fn change_adjacent_index(&mut self, from: usize, to: usize) {
        if self.ab == from {
            self.ab = to;
        } else if self.bc == from {
            self.bc = to;
        } else if self.ac == from {
            self.ac = to;
        }
    }

    /// Given that this triad is adjacent to `triangle_index` via `vertex_index`'s edge,
    /// return `(index_opposite, index_left, index_right)`.
    /// Port of Triad.cs `FindAdjacency`.
    fn find_adjacency(&self, vertex_index: usize, triangle_index: usize) -> (usize, usize, usize) {
        if self.ab == triangle_index {
            let opp = self.c;
            if vertex_index == self.a {
                (opp, self.ac, self.bc)
            } else {
                (opp, self.bc, self.ac)
            }
        } else if self.ac == triangle_index {
            let opp = self.b;
            if vertex_index == self.a {
                (opp, self.ab, self.bc)
            } else {
                (opp, self.bc, self.ab)
            }
        } else if self.bc == triangle_index {
            let opp = self.a;
            if vertex_index == self.b {
                (opp, self.ab, self.ac)
            } else {
                (opp, self.ac, self.ab)
            }
        } else {
            (0, 0, 0)
        }
    }
}

// ---------------------------------------------------------------------------
// Hull (Hull.cs) — a circular list of hull vertices.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct HullVertex {
    x: f32,
    y: f32,
    points_index: usize,
    triad_index: usize,
}

impl HullVertex {
    fn new(points: &[Vertex], point_index: usize) -> Self {
        HullVertex {
            x: points[point_index].x,
            y: points[point_index].y,
            points_index: point_index,
            triad_index: 0,
        }
    }
}

/// Circular convex hull stored as a `Vec` of `HullVertex`.
/// Wrapping index arithmetic replaces `NextIndex` (Hull.cs:7-12).
struct Hull {
    verts: Vec<HullVertex>,
}

impl Hull {
    fn new() -> Self {
        Hull { verts: Vec::new() }
    }
    fn len(&self) -> usize {
        self.verts.len()
    }
    fn next_index(&self, index: usize) -> usize {
        if index == self.verts.len() - 1 {
            0
        } else {
            index + 1
        }
    }
    fn vector_to_next(&self, index: usize) -> (f32, f32) {
        let a = &self.verts[index];
        let b = &self.verts[self.next_index(index)];
        (b.x - a.x, b.y - a.y)
    }
    /// Hull.cs `EdgeVisibleFrom(int, float dx, float dy)`.
    fn edge_visible_from_dir(&self, index: usize, dx: f32, dy: f32) -> bool {
        let (dx2, dy2) = self.vector_to_next(index);
        let cross = -dy * dx2 + dx * dy2;
        cross < 0.0
    }
    /// Hull.cs `EdgeVisibleFrom(int, Vertex)`.
    fn edge_visible_from_vertex(&self, index: usize, point: &HullVertex) -> bool {
        let (dx2, dy2) = self.vector_to_next(index);
        let dx = point.x - self.verts[index].x;
        let dy = point.y - self.verts[index].y;
        let cross = -dy * dx2 + dx * dy2;
        cross < 0.0
    }
}

// ---------------------------------------------------------------------------
// Triangulator — the core S-hull algorithm
// (Triangulator.cs:12-454)
// ---------------------------------------------------------------------------

struct Triangulator<'a> {
    points: &'a [Vertex],
    fraction: f32,
}

impl<'a> Triangulator<'a> {
    fn new(points: &'a [Vertex]) -> Self {
        Triangulator {
            points,
            fraction: 0.3,
        }
    }

    /// `Triangulator.Analyse` (Triangulator.cs:12-242).
    ///
    /// Returns `None` for degenerate input the S-hull algorithm cannot seed:
    /// fewer than 3 points (or fewer than 3 unique points when rejecting
    /// duplicates), or no triple admitting an initial circumcircle (all
    /// collinear). Callers treat `None` as "no valid triangulation".
    fn analyse(&self, triads: &mut Vec<Triad>, reject_duplicates: bool) -> Option<Hull> {
        let points = self.points;
        let num = points.len();
        if num < 3 {
            return None;
        }

        // Sort by distance squared from points[0].
        let mut dist2: Vec<f32> = (0..num)
            .map(|i| points[0].distance2_to(&points[i]))
            .collect();
        let mut order: Vec<usize> = (0..num).collect();
        // Stable sort — C# uses Array.Sort which is not stable, but since we want
        // determinism we use stable sort (same output for equal-distance points).
        stable_sort_by_key(&mut order, |&i| float_ord(dist2[i]));
        // Also re-order dist2 to match (needed later for the circumcircle pruning heuristic).
        let dist2_sorted: Vec<f32> = order.iter().map(|&i| dist2[i]).collect();
        // Overwrite dist2 with the sorted values.
        for (k, &d) in dist2_sorted.iter().enumerate() {
            dist2[k] = d;
        }
        let order = order; // freeze

        let mut effective_n = num;
        let mut effective_order = order.clone();
        if reject_duplicates {
            let mut k = effective_n as isize - 2;
            while k >= 0 {
                let ki = k as usize;
                let a = &points[effective_order[ki]];
                let b = &points[effective_order[ki + 1]];
                if a.x == b.x && a.y == b.y {
                    effective_order.remove(ki + 1);
                    effective_n -= 1;
                }
                k -= 1;
            }
            if effective_n < 3 {
                return None;
            }
        }

        // Find the smallest circumcircle for the initial triangle (triad a=order[0], b=order[1]).
        let mut best_j = usize::MAX;
        let mut best_r2 = f32::MAX;
        let mut circ_x = 0.0f32;
        let mut circ_y = 0.0f32;
        {
            let mut triad = Triad::new(effective_order[0], effective_order[1], 0);
            for j in 2..effective_n {
                triad.c = effective_order[j];
                if triad.find_circumcircle_precisely(points) && triad.circ_r2 < best_r2 {
                    best_j = j;
                    best_r2 = triad.circ_r2;
                    circ_x = triad.circ_x;
                    circ_y = triad.circ_y;
                } else if best_r2 * 4.0 < dist2[j] {
                    // Heuristic early exit: no closer circumcircle possible.
                    break;
                }
            }
        }
        if best_j == usize::MAX {
            return None;
        }

        let mut effective_order = effective_order;
        // Move the best third vertex to position 2.
        if best_j != 2 {
            effective_order.swap(2, best_j);
            let saved_d = dist2[best_j];
            // Shift dist2[3..=best_j] right by 1, put saved at position 2.
            for k in (2..best_j).rev() {
                dist2[k + 1] = dist2[k];
            }
            dist2[2] = saved_d;
        }

        // Build the seed triangle.
        let mut seed = Triad::new(effective_order[0], effective_order[1], effective_order[2]);
        seed.make_clockwise(points);
        seed.find_circumcircle_precisely(points);
        triads.push(seed);

        let mut hull = Hull::new();
        hull.verts.push(HullVertex::new(points, triads[0].a));
        hull.verts.push(HullVertex::new(points, triads[0].b));
        hull.verts.push(HullVertex::new(points, triads[0].c));

        // Re-sort points 3..n by distance from circumcenter.
        let center = Vertex {
            x: circ_x,
            y: circ_y,
        };
        let mut re_dist: Vec<f32> = (3..effective_n)
            .map(|k| {
                let v = &points[effective_order[k]];
                (v.x - center.x) * (v.x - center.x) + (v.y - center.y) * (v.y - center.y)
            })
            .collect();
        let mut sub_order: Vec<usize> = (0..re_dist.len()).collect();
        stable_sort_by_key(&mut sub_order, |&i| float_ord(re_dist[i]));
        // Apply re-sort to effective_order[3..].
        let tail: Vec<usize> = sub_order.iter().map(|&i| effective_order[3 + i]).collect();
        for (k, v) in tail.into_iter().enumerate() {
            effective_order[3 + k] = v;
        }
        let _ = re_dist;

        // Sweep remaining points into the triangulation.
        for l in 3..effective_n {
            let pt_idx = effective_order[l];
            let hv = HullVertex::new(points, pt_idx);
            let dx = hv.x - hull.verts[0].x;
            let dy = hv.y - hull.verts[0].y;
            let num8 = hull.len();

            // Collect visible hull edges.
            let mut list: Vec<usize> = Vec::new(); // visible hull vertex points_index
            let mut list2: Vec<usize> = Vec::new(); // corresponding hull vertex triad_index
            let insertion_pos: usize;

            if hull.edge_visible_from_dir(0, dx, dy) {
                // First edge is visible — walk around, may wrap.
                insertion_pos = if hull.edge_visible_from_dir(num8 - 1, dx, dy) {
                    // Wrapping case: last edge also visible.
                    list.push(hull.verts[num8 - 1].points_index);
                    list2.push(hull.verts[num8 - 1].triad_index);
                    let mut i = 0usize;
                    while i < num8 - 1 {
                        list.push(hull.verts[i].points_index);
                        list2.push(hull.verts[i].triad_index);
                        if hull.edge_visible_from_vertex(i, &hv) {
                            hull.verts.remove(i);
                            // don't increment i since the list shifted
                        } else {
                            hull.verts.insert(0, hv.clone());
                            break;
                        }
                    }
                    // Walk backwards from the (new) end.
                    let mut j = hull.verts.len() as isize - 2;
                    while j > 0 && hull.edge_visible_from_vertex(j as usize, &hv) {
                        list.insert(0, hull.verts[j as usize].points_index);
                        list2.insert(0, hull.verts[j as usize].triad_index);
                        hull.verts.remove(j as usize + 1);
                        j -= 1;
                    }
                    // Find position of hv in hull.
                    hull.verts
                        .iter()
                        .position(|v| v.points_index == pt_idx)
                        .unwrap_or(0)
                } else {
                    // Normal case: only edges starting from 0.
                    list2.push(hull.verts[0].triad_index);
                    list.push(hull.verts[0].points_index);
                    let mut i = 1usize;
                    let mut pos = 0usize;
                    while i < hull.verts.len() {
                        list.push(hull.verts[i].points_index);
                        list2.push(hull.verts[i].triad_index);
                        if hull.edge_visible_from_vertex(i, &hv) {
                            hull.verts.remove(i);
                            // don't increment i
                        } else {
                            hull.verts.insert(i, hv.clone());
                            pos = i;
                            break;
                        }
                    }
                    pos
                };
            } else {
                // Scan for first visible edge from index 1 onward.
                let mut start = usize::MAX;
                let mut end = num8;
                for m in 1..num8 {
                    if hull.edge_visible_from_vertex(m, &hv) {
                        if start == usize::MAX {
                            start = m;
                        }
                    } else if start < usize::MAX {
                        end = m;
                        break;
                    }
                }
                if end < num8 {
                    for n in start..=end {
                        list.push(hull.verts[n].points_index);
                        list2.push(hull.verts[n].triad_index);
                    }
                } else {
                    if start != usize::MAX {
                        for n in start..end {
                            list.push(hull.verts[n].points_index);
                            list2.push(hull.verts[n].triad_index);
                        }
                    }
                    list.push(hull.verts[0].points_index);
                }
                if start != usize::MAX && start < end - 1 {
                    hull.verts.drain((start + 1)..end);
                }
                hull.verts.insert(start + 1, hv.clone());
                insertion_pos = start + 1;
            }

            // Build new triangles connecting pt_idx to the visible edge list.
            let num16 = list.len() - 1;
            let first_triad_idx = triads.len();
            let mut num17 = first_triad_idx;
            for num18 in 0..num16 {
                let ab_adj = if num18 > 0 { num17 - 1 } else { NO_NEIGHBOR };
                let ac_adj = num17 + 1;
                let mut t2 = Triad::new_with_adj(
                    pt_idx,
                    list[num18],
                    list[num18 + 1],
                    ab_adj,
                    list2[num18],
                    ac_adj,
                );
                t2.find_circumcircle_precisely(points);

                // Update the neighboring triad's adjacency to point back to num17.
                let nb_idx = list2[num18];
                if nb_idx < triads.len() {
                    let nb = &triads[nb_idx];
                    let ta_b = t2.b;
                    let ta_c = t2.c;
                    // Check which edge of nb shares with t2's b-c edge.
                    if (ta_b == nb.a && ta_c == nb.b) || (ta_b == nb.b && ta_c == nb.a) {
                        triads[nb_idx].ab = num17;
                    } else if (ta_b == nb.a && ta_c == nb.c) || (ta_b == nb.c && ta_c == nb.a) {
                        triads[nb_idx].ac = num17;
                    } else if (ta_b == nb.b && ta_c == nb.c) || (ta_b == nb.c && ta_c == nb.b) {
                        triads[nb_idx].bc = num17;
                    }
                }
                triads.push(t2);
                num17 += 1;
            }
            // Fix last triangle's ac to NO_NEIGHBOR.
            if num17 > first_triad_idx {
                triads[num17 - 1].ac = NO_NEIGHBOR;
            }
            // Update hull triadIndex references.
            if insertion_pos < hull.verts.len() {
                hull.verts[insertion_pos].triad_index = num17 - 1;
            }
            if insertion_pos > 0 && insertion_pos - 1 < hull.verts.len() {
                hull.verts[insertion_pos - 1].triad_index = first_triad_idx;
            } else {
                let last = hull.verts.len() - 1;
                if last < hull.verts.len() {
                    hull.verts[last].triad_index = first_triad_idx;
                }
            }
        }
        Some(hull)
    }

    // -----------------------------------------------------------------------
    // FlipTriangle (Triangulator.cs:294-386)
    // -----------------------------------------------------------------------

    fn flip_triangle(triads: &mut Vec<Triad>, test_idx: usize, points: &[Vertex]) -> Option<usize> {
        // Try each of the three adjacency slots in the order bc, ab, ac —
        // matching the exact order from Triangulator.cs:301-384.

        // Slot bc (Triangulator.cs:301-327):
        let bc_adj = triads[test_idx].bc;
        if bc_adj != NO_NEIGHBOR {
            let flipped_idx = bc_adj;
            let v_b = triads[test_idx].b;
            let (index_opp, index_left, index_right) =
                triads[flipped_idx].find_adjacency(v_b, test_idx);
            if index_opp < points.len() && triads[test_idx].inside_circumcircle(&points[index_opp])
            {
                let ab2 = triads[test_idx].ab;
                let ac2 = triads[test_idx].ac;
                if ab2 != index_left && ac2 != index_right {
                    let a = triads[test_idx].a;
                    let b = triads[test_idx].b;
                    let c = triads[test_idx].c;
                    triads[test_idx].initialize(
                        a,
                        b,
                        index_opp,
                        ab2,
                        index_left,
                        flipped_idx,
                        points,
                    );
                    triads[flipped_idx].initialize(
                        a,
                        c,
                        index_opp,
                        ac2,
                        index_right,
                        test_idx,
                        points,
                    );
                    if index_left != NO_NEIGHBOR {
                        triads[index_left].change_adjacent_index(flipped_idx, test_idx);
                    }
                    if ac2 != NO_NEIGHBOR {
                        triads[ac2].change_adjacent_index(test_idx, flipped_idx);
                    }
                    return Some(flipped_idx);
                }
            }
        }

        // Slot ab (Triangulator.cs:329-355):
        let ab_adj = triads[test_idx].ab;
        if ab_adj != NO_NEIGHBOR {
            let flipped_idx = ab_adj;
            let v_a = triads[test_idx].a;
            let (index_opp, index_left, index_right) =
                triads[flipped_idx].find_adjacency(v_a, test_idx);
            if index_opp < points.len() && triads[test_idx].inside_circumcircle(&points[index_opp])
            {
                let ab2 = triads[test_idx].ac;
                let ac2 = triads[test_idx].bc;
                if ab2 != index_left && ac2 != index_right {
                    let a = triads[test_idx].a;
                    let b = triads[test_idx].b;
                    let c = triads[test_idx].c;
                    triads[test_idx].initialize(
                        c,
                        a,
                        index_opp,
                        ab2,
                        index_left,
                        flipped_idx,
                        points,
                    );
                    triads[flipped_idx].initialize(
                        c,
                        b,
                        index_opp,
                        ac2,
                        index_right,
                        test_idx,
                        points,
                    );
                    if index_left != NO_NEIGHBOR {
                        triads[index_left].change_adjacent_index(flipped_idx, test_idx);
                    }
                    if ac2 != NO_NEIGHBOR {
                        triads[ac2].change_adjacent_index(test_idx, flipped_idx);
                    }
                    return Some(flipped_idx);
                }
            }
        }

        // Slot ac (Triangulator.cs:357-384):
        let ac_adj = triads[test_idx].ac;
        if ac_adj != NO_NEIGHBOR {
            let flipped_idx = ac_adj;
            let v_a = triads[test_idx].a;
            let (index_opp, index_left, index_right) =
                triads[flipped_idx].find_adjacency(v_a, test_idx);
            if index_opp < points.len() && triads[test_idx].inside_circumcircle(&points[index_opp])
            {
                let ab2 = triads[test_idx].ab;
                let ac2 = triads[test_idx].bc;
                if ab2 != index_left && ac2 != index_right {
                    let a = triads[test_idx].a;
                    let b = triads[test_idx].b;
                    let c = triads[test_idx].c;
                    triads[test_idx].initialize(
                        b,
                        a,
                        index_opp,
                        ab2,
                        index_left,
                        flipped_idx,
                        points,
                    );
                    triads[flipped_idx].initialize(
                        b,
                        c,
                        index_opp,
                        ac2,
                        index_right,
                        test_idx,
                        points,
                    );
                    if index_left != NO_NEIGHBOR {
                        triads[index_left].change_adjacent_index(flipped_idx, test_idx);
                    }
                    if ac2 != NO_NEIGHBOR {
                        triads[ac2].change_adjacent_index(test_idx, flipped_idx);
                    }
                    return Some(flipped_idx);
                }
            }
        }
        None
    }

    // -----------------------------------------------------------------------
    // FlipTriangles passes (Triangulator.cs:388-453)
    // -----------------------------------------------------------------------

    fn flip_pass_bool(
        triads: &mut Vec<Triad>,
        ids_to_test: &[bool],
        ids_flipped: &mut Vec<bool>,
        points: &[Vertex],
    ) -> usize {
        let count = triads.len();
        if ids_flipped.len() < count {
            ids_flipped.resize(count, false);
        }
        for b in ids_flipped.iter_mut() {
            *b = false;
        }
        let mut num = 0usize;
        for i in 0..count {
            if ids_to_test.len() <= i || ids_to_test[i] {
                if let Some(flipped_idx) = Self::flip_triangle(triads, i, points) {
                    num += 2;
                    if ids_flipped.len() <= i {
                        ids_flipped.resize(i + 1, false);
                    }
                    if ids_flipped.len() <= flipped_idx {
                        ids_flipped.resize(flipped_idx + 1, false);
                    }
                    ids_flipped[i] = true;
                    ids_flipped[flipped_idx] = true;
                }
            }
        }
        num
    }

    fn flip_pass_set(
        triads: &mut Vec<Triad>,
        ids_to_test: &std::collections::BTreeSet<usize>,
        ids_flipped: &mut std::collections::BTreeSet<usize>,
        points: &[Vertex],
    ) -> usize {
        ids_flipped.clear();
        let mut num = 0usize;
        for &i in ids_to_test.iter() {
            if i < triads.len() {
                if let Some(flipped_idx) = Self::flip_triangle(triads, i, points) {
                    num += 2;
                    ids_flipped.insert(i);
                    ids_flipped.insert(flipped_idx);
                }
            }
        }
        num
    }

    /// `Triangulator.Triangulation` (Triangulator.cs:262-292).
    ///
    /// Returns `None` when [`analyse`](Self::analyse) cannot seed a triangulation
    /// on degenerate input (see its docs).
    fn triangulation(&self, reject_duplicates: bool) -> Option<Vec<Triad>> {
        let mut triads: Vec<Triad> = Vec::new();
        self.analyse(&mut triads, reject_duplicates)?;

        let count = triads.len();
        let mut arr_a: Vec<bool> = vec![false; count];
        let mut arr_b: Vec<bool> = vec![false; count];

        // First pass: all triangles, use bool arrays.
        // For the initial full pass we pass an empty ids_to_test (meaning "test all").
        let empty_test: Vec<bool> = Vec::new();
        let mut num = Self::flip_pass_bool(&mut triads, &empty_test, &mut arr_a, self.points);
        let mut pass = 1usize;
        while num > (self.fraction * count as f32) as usize && pass < 1000 {
            if pass & 1 == 1 {
                num = Self::flip_pass_bool(&mut triads, &arr_a, &mut arr_b, self.points);
            } else {
                num = Self::flip_pass_bool(&mut triads, &arr_b, &mut arr_a, self.points);
            }
            pass += 1;
        }

        // Second pass: use BTreeSet for deterministic set semantics (C# SortedList<int,int>).
        let arr_last = if (pass & 1) == 1 { &arr_a } else { &arr_b };
        let mut set_a: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        for (i, &v) in arr_last.iter().enumerate() {
            if v {
                set_a.insert(i);
            }
        }
        let mut set_b: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        num = Self::flip_pass_set(&mut triads, &set_a, &mut set_b, self.points);
        pass = 1;
        while num > 0 && pass < 2000 {
            if pass & 1 == 1 {
                num = Self::flip_pass_set(&mut triads, &set_b, &mut set_a, self.points);
            } else {
                num = Self::flip_pass_set(&mut triads, &set_a, &mut set_b, self.points);
            }
            pass += 1;
        }

        Some(triads)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Stable sort `slice` by a key function, mirroring `Array.Sort` semantics.
fn stable_sort_by_key<T, K: Ord>(slice: &mut Vec<T>, key: impl Fn(&T) -> K) {
    slice.sort_by(|a, b| key(a).cmp(&key(b)));
}

/// Wrap f32 for use as an `Ord` key (NaN-safe: NaN -> max).
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct FloatOrd(u32);

fn float_ord(f: f32) -> FloatOrd {
    if f.is_nan() {
        FloatOrd(u32::MAX)
    } else if f >= 0.0 {
        FloatOrd(f.to_bits() + 0x8000_0000)
    } else {
        FloatOrd(!(f.to_bits()))
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Triangulate `points` using the S-hull Delaunay algorithm.
///
/// Mirrors `Triangulator.Triangulation(points)` (Triangulator.cs:262-292).
/// Returns a `Vec<Triad>` with indices into `points`.
///
/// Panics on degenerate input (fewer than 3 points, or no seed circumcircle).
/// Only tests call it, always with ≥ 3 non-collinear points; the fan collapse in
/// `objects::geometry` uses the fallible [`triangulate_reject_dups`].
pub fn triangulate(points: &[Vertex]) -> Vec<Triad> {
    Triangulator::new(points)
        .triangulation(false)
        .expect("triangulate requires >= 3 non-collinear points")
}

/// Triangulate with the `rejectDuplicatePoints` flag, mirroring
/// `Triangulator.Triangulation(points, true)` (Triangulator.cs:266), as
/// `Geometry.CreateTriangles` always passes `rejectDuplicatePoints: true`.
///
/// Returns `None` on degenerate input (fewer than 3 unique points, or an
/// all-collinear fan with no initial circumcircle); the caller treats it as a
/// rejected collapse.
pub fn triangulate_reject_dups(points: &[Vertex]) -> Option<Vec<Triad>> {
    Triangulator::new(points).triangulation(true)
}

/// Number of points on the convex hull of `points`.
///
/// Mirrors `Triangulator.ConvexHull(points, rejectDuplicatePoints: true).Count`
/// (Triangulator.cs:249). `Geometry.CreateTriangles` uses it to reject any fan
/// with an interior point (hull count != point count, Geometry.cs:1549).
///
/// Returns `0` on degenerate input the triangulator cannot seed. A valid hull
/// always has ≥ 3 points, so `0` never equals a real point count and the caller's
/// `convex_hull_len(..) == points.len()` convexity check rejects the fan.
pub fn convex_hull_len(points: &[Vertex], reject_duplicates: bool) -> usize {
    let tri = Triangulator::new(points);
    let mut triads: Vec<Triad> = Vec::new();
    match tri.analyse(&mut triads, reject_duplicates) {
        Some(hull) => hull.len(),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_square_triangulates_to_two() {
        let pts = vec![
            Vertex { x: 0.0, y: 0.0 },
            Vertex { x: 1.0, y: 0.0 },
            Vertex { x: 1.0, y: 1.0 },
            Vertex { x: 0.0, y: 1.0 },
        ];
        let tris = triangulate(&pts);
        assert_eq!(tris.len(), 2, "4-point square = 2 triangles");
        // every output index is in range
        for t in &tris {
            for &i in &[t.a, t.b, t.c] {
                assert!(i < pts.len());
            }
        }
    }

    #[test]
    fn convex_hull_len_square_is_four() {
        let pts = vec![
            Vertex { x: 0.0, y: 0.0 },
            Vertex { x: 1.0, y: 0.0 },
            Vertex { x: 1.0, y: 1.0 },
            Vertex { x: 0.0, y: 1.0 },
        ];
        assert_eq!(convex_hull_len(&pts, true), 4, "square: all 4 on hull");
    }

    #[test]
    fn convex_hull_len_excludes_interior_point() {
        // Square + a point at the center → hull is the 4 corners, center is interior.
        let pts = vec![
            Vertex { x: 0.0, y: 0.0 },
            Vertex { x: 2.0, y: 0.0 },
            Vertex { x: 2.0, y: 2.0 },
            Vertex { x: 0.0, y: 2.0 },
            Vertex { x: 1.0, y: 1.0 }, // interior
        ];
        assert_eq!(
            convex_hull_len(&pts, true),
            4,
            "interior point excluded from hull"
        );
        assert_ne!(
            convex_hull_len(&pts, true),
            pts.len(),
            "hull != point count → reject in CreateTriangles"
        );
    }

    #[test]
    fn grid_triangulation_covers_all_points() {
        // 3x3 grid -> 8 triangles (2*(n-1)^2 for a regular grid)
        let mut pts = Vec::new();
        for y in 0..3 {
            for x in 0..3 {
                pts.push(Vertex {
                    x: x as f32,
                    y: y as f32,
                });
            }
        }
        let tris = triangulate(&pts);
        assert_eq!(tris.len(), 8);
    }

    // --- Degenerate input must return None / 0, never panic ------------------

    #[test]
    fn reject_dups_returns_some_for_convex_polygon() {
        let pts = vec![
            Vertex { x: 0.0, y: 0.0 },
            Vertex { x: 1.0, y: 0.0 },
            Vertex { x: 1.0, y: 1.0 },
            Vertex { x: 0.0, y: 1.0 },
        ];
        let tris = triangulate_reject_dups(&pts).expect("convex polygon triangulates");
        assert_eq!(tris.len(), 2, "4-point square = 2 triangles");
        for t in &tris {
            for &i in &[t.a, t.b, t.c] {
                assert!(i < pts.len());
            }
        }
    }

    #[test]
    fn reject_dups_none_for_collinear_points() {
        // All points on the x-axis: no triple has a finite circumcircle.
        let pts = vec![
            Vertex { x: 0.0, y: 0.0 },
            Vertex { x: 1.0, y: 0.0 },
            Vertex { x: 2.0, y: 0.0 },
            Vertex { x: 3.0, y: 0.0 },
        ];
        assert!(triangulate_reject_dups(&pts).is_none());
    }

    #[test]
    fn reject_dups_none_for_coincident_points() {
        let pts = vec![
            Vertex { x: 1.0, y: 1.0 },
            Vertex { x: 1.0, y: 1.0 },
            Vertex { x: 1.0, y: 1.0 },
            Vertex { x: 1.0, y: 1.0 },
        ];
        // After rejecting duplicates only one unique point remains (< 3).
        assert!(triangulate_reject_dups(&pts).is_none());
    }

    #[test]
    fn reject_dups_none_for_fewer_than_three_unique() {
        let pts = vec![
            Vertex { x: 0.0, y: 0.0 },
            Vertex { x: 1.0, y: 0.0 },
            Vertex { x: 1.0, y: 0.0 },
        ];
        // Two unique points after dedup → cannot triangulate.
        assert!(triangulate_reject_dups(&pts).is_none());
    }

    #[test]
    fn convex_hull_len_zero_for_collinear() {
        let pts = vec![
            Vertex { x: 0.0, y: 0.0 },
            Vertex { x: 1.0, y: 0.0 },
            Vertex { x: 2.0, y: 0.0 },
        ];
        // Degenerate → 0, which can never equal a real point count, so the
        // CreateTriangles convexity check (== point count) fails cleanly.
        assert_eq!(convex_hull_len(&pts, true), 0);
    }
}
