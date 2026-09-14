//! Garland & Heckbert "Terra" greedy-insertion TIN terrain approximation.
//!
//! Port of the decompiled C# Terra engine (`LODGenerator.Terra*`):
//!   - quad-edge incremental Delaunay (`Subdivision.cs`, `Edge.cs`)
//!   - in-circle / orientation predicates (`GeometryHelpers.cs`)
//!   - per-triangle plane fit + scan-conversion error metric
//!     (`Plane.cs`, `GreedySubdivision.cs`, `Candidate.cs`)
//!   - token-indexed binary max-heap (`Heap.cs`, `HeapNode.cs`)
//!   - greedy driver + stop predicate (`Terrain.cs`)
//!
//! The quad-edge is ported to an arena of edge records keyed by integer handles
//! (`EdgeId`) to avoid `Rc<RefCell>` reference cycles; navigation operators map
//! directly onto the C# `Edge` accessors (`Sym`/`Rot`/`InvRot`/`ONext`/...).
//!
//! Determinism deviation: `Subdivision.Locate` (`Subdivision.cs:175`) breaks an
//! exact on-edge / zero-area collinear tie with `rand.Next() & 1`; this port always
//! takes the `oNext` branch instead. Only exact-collinear ambiguities are affected;
//! generic interior points are not.
//!
//! Validation is by triangle-count parity + max-error bound, not byte/hash match:
//! f32 non-associativity in heap ordering makes exact reproduction of the C#
//! impossible.

const EPS: f32 = 1e-6;
const NOT_IN_HEAP: i32 = -47;
const NO_CANDIDATE: i32 = -69;

// ---------------------------------------------------------------------------
// Vector2 (LODGenerator.Common/Vector2.cs)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct V2 {
    x: f32,
    y: f32,
}

impl V2 {
    fn new(x: f32, y: f32) -> Self {
        V2 { x, y }
    }
    fn sub(self, o: V2) -> V2 {
        V2::new(self.x - o.x, self.y - o.y)
    }
    fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    fn approx_eq(self, o: V2) -> bool {
        self.x == o.x && self.y == o.y
    }
}

// ---------------------------------------------------------------------------
// Geometry predicates (LODGenerator.Terra.Geometry/GeometryHelpers.cs)
// ---------------------------------------------------------------------------

#[inline]
fn tri_area(a: V2, b: V2, c: V2) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

#[inline]
fn is_counter_clockwise(a: V2, b: V2, c: V2) -> bool {
    (tri_area(a, b, c) as f64) > 0.0
}

#[inline]
fn is_right_of(point: V2, origin: V2, destination: V2) -> bool {
    is_counter_clockwise(point, destination, origin)
}

#[inline]
fn is_left_of(point: V2, origin: V2, destination: V2) -> bool {
    is_counter_clockwise(point, origin, destination)
}

#[inline]
fn is_in_circle(a: V2, b: V2, c: V2, d: V2) -> bool {
    let num = (a.x * a.x + a.y * a.y) * tri_area(b, c, d);
    let num2 = (b.x * b.x + b.y * b.y) * tri_area(a, c, d);
    let num3 = (c.x * c.x + c.y * c.y) * tri_area(a, b, d);
    let num4 = (d.x * d.x + d.y * d.y) * tri_area(a, b, c);
    num - num2 + num3 - num4 > EPS
}

// ---------------------------------------------------------------------------
// Plane (LODGenerator.Terra.Geometry/Plane.cs)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct Plane {
    a: f32,
    b: f32,
    c: f32,
}

impl Plane {
    fn init(p: [f32; 3], q: [f32; 3], r: [f32; 3]) -> Plane {
        let num = q[0] - p[0];
        let num2 = q[1] - p[1];
        let num3 = q[2] - p[2];
        let num4 = r[0] - p[0];
        let num5 = r[1] - p[1];
        let num6 = r[2] - p[2];
        let num7 = num * num5 - num2 * num4;
        let a = (num3 * num5 - num2 * num6) / num7;
        let b = (num * num6 - num3 * num4) / num7;
        let c = p[2] - a * p[0] - b * p[1];
        Plane { a, b, c }
    }
    #[inline]
    fn eval_i(&self, x: i32, y: i32) -> f32 {
        self.a * (x as f32) + self.b * (y as f32) + self.c
    }
}

// ---------------------------------------------------------------------------
// Line (LODGenerator.Terra.Geometry/Line.cs) — used by OnEdge
// ---------------------------------------------------------------------------

struct Line {
    a: f32,
    b: f32,
    c: f32,
}

impl Line {
    fn new(p: V2, q: V2) -> Line {
        let v = q.sub(p);
        let length = v.length();
        let a = v.y / length;
        let b = -v.x / length;
        let c = -(a * p.x + b * p.y);
        Line { a, b, c }
    }
    fn eval(&self, p: V2) -> f32 {
        self.a * p.x + self.b * p.y + self.c
    }
}

// ---------------------------------------------------------------------------
// Quad-edge arena (LODGenerator.Terra.Geometry/Edge.cs)
//
// Each C# `new Edge()` allocates a quad of 4 Edge objects forming the rotation
// ring (qNext). Here a quad is 4 contiguous EdgeRecord slots. An EdgeId is the
// arena index of an individual directed edge. The rotation ring is implicit:
// the 4 members of a quad are {base, base+1, base+2, base+3} where base = id & !3
// (quads are 4-aligned). Rot advances within the quad mod 4 honoring the C#
// wiring (qNext: e0->e1->e2->e3->e0).
// ---------------------------------------------------------------------------

type EdgeId = usize;
type FaceId = usize;

#[derive(Clone, Copy)]
struct EdgeRecord {
    next: EdgeId, // ONext
    data: Option<V2>,
    lface: Option<FaceId>,
}

struct Face {
    anchor: EdgeId,
    next_face: Option<FaceId>,
    // tracked-triangle candidate state (TrackedTriangle.cs / heap token)
    token: i32,
    cand_x: i32,
    cand_y: i32,
}

// ---------------------------------------------------------------------------
// Heap (LODGenerator.Terra.Memory/Heap.cs) — token-indexed binary max-heap.
// "Object" is a FaceId; the face's `token` field stores the heap slot index.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct HeapNode {
    importance: f32,
    face: FaceId,
}

struct Heap {
    nodes: Vec<HeapNode>,
    size: usize,
}

impl Heap {
    fn new() -> Heap {
        Heap {
            nodes: Vec::with_capacity(256),
            size: 0,
        }
    }

    fn insert(&mut self, faces: &mut [Face], face: FaceId, importance: f32) {
        let num = self.size;
        self.size += 1;
        let node = HeapNode { importance, face };
        faces[face].token = num as i32;
        if num < self.nodes.len() {
            self.nodes[num] = node;
        } else {
            self.nodes.push(node);
        }
        self.up_heap(faces, num);
    }

    fn update(&mut self, faces: &mut [Face], face: FaceId, importance: f32) {
        let token = faces[face].token as usize;
        let importance2 = self.nodes[token].importance;
        self.nodes[token].importance = importance;
        if importance < importance2 {
            self.down_heap(faces, token);
        } else {
            self.up_heap(faces, token);
        }
    }

    fn extract(&mut self, faces: &mut [Face]) -> Option<HeapNode> {
        if self.size < 1 {
            return None;
        }
        self.swap(faces, 0, self.size - 1);
        self.size -= 1;
        self.down_heap(faces, 0);
        let last = self.nodes[self.size];
        faces[last.face].token = NOT_IN_HEAP;
        Some(last)
    }

    fn top(&self) -> Option<HeapNode> {
        if self.size >= 1 {
            Some(self.nodes[0])
        } else {
            None
        }
    }

    fn kill(&mut self, faces: &mut [Face], i: usize) {
        self.swap(faces, i, self.size - 1);
        self.size -= 1;
        let last_face = self.nodes[self.size].face;
        faces[last_face].token = NOT_IN_HEAP;
        if self.nodes[i].importance < self.nodes[self.size].importance {
            self.down_heap(faces, i);
        } else {
            self.up_heap(faces, i);
        }
    }

    fn swap(&mut self, faces: &mut [Face], i: usize, j: usize) {
        self.nodes.swap(i, j);
        faces[self.nodes[i].face].token = i as i32;
        faces[self.nodes[j].face].token = j as i32;
    }

    fn parent(i: usize) -> usize {
        (i - 1) / 2
    }
    fn left(i: usize) -> usize {
        i * 2 + 1
    }
    fn right(i: usize) -> usize {
        i * 2 + 2
    }

    fn up_heap(&mut self, faces: &mut [Face], i: usize) {
        if i != 0 {
            let num = Self::parent(i);
            if !(self.nodes[i].importance <= self.nodes[num].importance) {
                self.swap(faces, i, num);
                self.up_heap(faces, num);
            }
        }
    }

    fn down_heap(&mut self, faces: &mut [Face], i: usize) {
        if i < self.size {
            let mut num = i;
            let num2 = Self::left(i);
            let num3 = Self::right(i);
            if num2 < self.size && self.nodes[num2].importance > self.nodes[num].importance {
                num = num2;
            }
            if num3 < self.size && self.nodes[num3].importance > self.nodes[num].importance {
                num = num3;
            }
            if num != i {
                self.swap(faces, i, num);
                self.down_heap(faces, num);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PointState (LODGenerator.Terra.Greedy/PointState.cs)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum PointState {
    Unused,
    Used,
    Ignored,
}

// ---------------------------------------------------------------------------
// Candidate (LODGenerator.Terra.Greedy/Candidate.cs)
// ---------------------------------------------------------------------------

struct Candidate {
    x: i32,
    y: i32,
    importance: f32,
}

impl Candidate {
    fn new() -> Candidate {
        Candidate {
            x: 0,
            y: 0,
            importance: f32::MIN,
        }
    }
    // Candidate.cs:18 — `!(imp <= best)` so the FIRST maximum wins (tie-break).
    fn consider(&mut self, sub_x: i32, sub_y: i32, importance: f32) {
        if !(importance <= self.importance) {
            self.x = sub_x;
            self.y = sub_y;
            self.importance = importance;
        }
    }
}

// ---------------------------------------------------------------------------
// Terra — GreedySubdivision + Terrain combined.
// ---------------------------------------------------------------------------

/// Greedy-insertion TIN over a `width × height` integer-grid heightmap.
pub struct Terra {
    // quad-edge arena
    edges: Vec<EdgeRecord>,
    faces: Vec<Face>,
    starting_edge: EdgeId,
    first_face: Option<FaceId>,
    free_quads: Vec<EdgeId>, // recycled quad base ids (4-aligned)

    // greedy / heightmap
    heights: Vec<f32>,
    width: usize,
    height: usize,
    point_states: Vec<PointState>, // [x + y*width]
    point_count: u32,
    heap: Heap,

    // terrain driver
    error_threshold: f32,
    point_count_limit: i64,
}

impl Terra {
    /// Mirrors `Terrain` ctor (`Terrain.cs:29-38`) composed with
    /// `GreedySubdivision` ctor (`GreedySubdivision.cs:21-51`).
    /// `heights` is row-major `[col + row*width]`; `Eval(x,y) = heights[x + y*width]`.
    pub fn new(
        error_threshold: f32,
        point_count_limit: i64,
        width: usize,
        height: usize,
        heights: &[f32],
    ) -> Self {
        assert_eq!(heights.len(), width * height, "heights size mismatch");
        // GreedySubdivision caps at 1025 (GreedySubdivision.cs:27-36).
        let w = width.min(1025);
        let h = height.min(1025);

        let limit = if point_count_limit > -1 {
            point_count_limit
        } else {
            (w as i64) * (h as i64)
        };

        let mut t = Terra {
            edges: Vec::new(),
            faces: Vec::new(),
            starting_edge: 0,
            first_face: None,
            free_quads: Vec::new(),
            heights: heights.to_vec(),
            width: w,
            height: h,
            point_states: vec![PointState::Unused; w * h],
            point_count: 0,
            heap: Heap::new(),
            error_threshold,
            point_count_limit: limit,
        };

        // InitMesh seeds two triangles from the 4 block corners.
        t.init_mesh(
            V2::new(0.0, 0.0),
            V2::new(0.0, (h - 1) as f32),
            V2::new((w - 1) as f32, (h - 1) as f32),
            V2::new((w - 1) as f32, 0.0),
        );
        t.set_state(0, 0, PointState::Used);
        t.set_state(0, h - 1, PointState::Used);
        t.set_state(w - 1, h - 1, PointState::Used);
        t.set_state(w - 1, 0, PointState::Used);
        t.point_count = 4;
        t
    }

    // ---- heightmap access (Map.cs Eval) ----

    #[inline]
    fn eval_height(&self, x: i32, y: i32) -> f32 {
        self.heights[x as usize + (y as usize) * self.width]
    }

    #[inline]
    fn state(&self, x: usize, y: usize) -> PointState {
        self.point_states[x + y * self.width]
    }

    #[inline]
    fn set_state(&mut self, x: usize, y: usize, s: PointState) {
        self.point_states[x + y * self.width] = s;
    }

    // -----------------------------------------------------------------------
    // Quad-edge primitives (Edge.cs)
    // -----------------------------------------------------------------------

    #[inline]
    fn rot(e: EdgeId) -> EdgeId {
        // qNext within the 4-aligned quad: e0->e1->e2->e3->e0
        let base = e & !3;
        base + ((e - base + 1) & 3)
    }
    #[inline]
    fn inv_rot(e: EdgeId) -> EdgeId {
        // qPrev: e0->e3->e2->e1->e0
        let base = e & !3;
        base + ((e - base + 3) & 3)
    }
    #[inline]
    fn sym(e: EdgeId) -> EdgeId {
        let base = e & !3;
        base + ((e - base + 2) & 3)
    }
    #[inline]
    fn onext(&self, e: EdgeId) -> EdgeId {
        self.edges[e].next
    }
    #[inline]
    fn oprev(&self, e: EdgeId) -> EdgeId {
        // Rot.ONext.Rot
        Self::rot(self.onext(Self::rot(e)))
    }
    #[inline]
    #[allow(dead_code)] // part of the complete quad-edge operator set (Edge.cs:26)
    fn dnext(&self, e: EdgeId) -> EdgeId {
        // Sym.ONext.Sym
        Self::sym(self.onext(Self::sym(e)))
    }
    #[inline]
    fn dprev(&self, e: EdgeId) -> EdgeId {
        // InvRot.ONext.InvRot
        Self::inv_rot(self.onext(Self::inv_rot(e)))
    }
    #[inline]
    fn lnext(&self, e: EdgeId) -> EdgeId {
        // InvRot.ONext.Rot
        Self::rot(self.onext(Self::inv_rot(e)))
    }
    #[inline]
    fn lprev(&self, e: EdgeId) -> EdgeId {
        // ONext.Sym
        Self::sym(self.onext(e))
    }
    #[inline]
    fn rnext(&self, e: EdgeId) -> EdgeId {
        // Rot.ONext.InvRot
        Self::inv_rot(self.onext(Self::rot(e)))
    }
    #[inline]
    fn rprev(&self, e: EdgeId) -> EdgeId {
        // Sym.ONext
        self.onext(Self::sym(e))
    }
    #[inline]
    fn orig(&self, e: EdgeId) -> V2 {
        self.edges[e].data.expect("edge orig unset")
    }
    #[inline]
    fn dest(&self, e: EdgeId) -> V2 {
        self.edges[Self::sym(e)].data.expect("edge dest unset")
    }
    #[inline]
    fn lface(&self, e: EdgeId) -> Option<FaceId> {
        self.edges[e].lface
    }
    #[inline]
    fn set_lface(&mut self, e: EdgeId, f: Option<FaceId>) {
        self.edges[e].lface = f;
    }

    fn set_endpoints(&mut self, e: EdgeId, orig: V2, dest: V2) {
        self.edges[e].data = Some(orig);
        let s = Self::sym(e);
        self.edges[s].data = Some(dest);
    }

    /// Allocate a fresh quad of 4 edges, return the primal base edge (id % 4 == 0).
    /// Mirrors `new Edge()` wiring (Edge.cs:54-66).
    fn make_edge(&mut self) -> EdgeId {
        let base = if let Some(b) = self.free_quads.pop() {
            for k in 0..4 {
                self.edges[b + k] = EdgeRecord {
                    next: 0,
                    data: None,
                    lface: None,
                };
            }
            b
        } else {
            let b = self.edges.len();
            for _ in 0..4 {
                self.edges.push(EdgeRecord {
                    next: 0,
                    data: None,
                    lface: None,
                });
            }
            b
        };
        // ONext wiring: e0->e0, e1->e3, e2->e2, e3->e1
        self.edges[base].next = base;
        self.edges[base + 1].next = base + 3;
        self.edges[base + 2].next = base + 2;
        self.edges[base + 3].next = base + 1;
        base
    }

    fn make_edge_pts(&mut self, orig: V2, dest: V2) -> EdgeId {
        let e = self.make_edge();
        self.set_endpoints(e, orig, dest);
        e
    }

    /// Edge.Splice (Edge.cs:105-117).
    fn splice(&mut self, a: EdgeId, b: EdgeId) {
        let rot_a = Self::rot(self.onext(a));
        let rot_b = Self::rot(self.onext(b));
        let onext_b = self.onext(b);
        let onext_a = self.onext(a);
        let onext_rot_b = self.onext(rot_b);
        let onext_rot_a = self.onext(rot_a);
        self.edges[a].next = onext_b;
        self.edges[b].next = onext_a;
        self.edges[rot_a].next = onext_rot_b;
        self.edges[rot_b].next = onext_rot_a;
    }

    /// DeleteEdge (Subdivision.cs:305-310).
    fn delete_edge(&mut self, e: EdgeId) {
        let op = self.oprev(e);
        self.splice(e, op);
        let s = Self::sym(e);
        let sop = self.oprev(s);
        self.splice(s, sop);
        // clear data (so misuse panics) and recycle the quad slots
        let base = e & !3;
        for k in 0..4 {
            self.edges[base + k].data = None;
            self.edges[base + k].lface = None;
        }
        self.free_quads.push(base);
    }

    /// Connect (Subdivision.cs:312-318).
    fn connect(&mut self, a: EdgeId, b: EdgeId) -> EdgeId {
        let e = self.make_edge();
        let a_lnext = self.lnext(a);
        self.splice(e, a_lnext);
        let e_sym = Self::sym(e);
        self.splice(e_sym, b);
        let a_dest = self.dest(a);
        let b_orig = self.orig(b);
        self.set_endpoints(e, a_dest, b_orig);
        e
    }

    /// Swap (Subdivision.cs:321-334).
    fn swap_edge(&mut self, e: EdgeId) {
        let lface = self.lface(e);
        let e_sym = Self::sym(e);
        let lface2 = self.lface(e_sym);
        let oprev = self.oprev(e);
        let oprev2 = self.oprev(e_sym);
        self.splice(e, oprev);
        self.splice(e_sym, oprev2);
        let oprev_lnext = self.lnext(oprev);
        self.splice(e, oprev_lnext);
        let oprev2_lnext = self.lnext(oprev2);
        self.splice(e_sym, oprev2_lnext);
        let od = self.dest(oprev);
        let od2 = self.dest(oprev2);
        self.set_endpoints(e, od, od2);
        if let Some(f) = lface {
            self.reshape(f, e);
        }
        if let Some(f) = lface2 {
            self.reshape(f, e_sym);
        }
    }

    // -----------------------------------------------------------------------
    // Faces / LinkedListTriangle (LinkedListTriangle.cs)
    // -----------------------------------------------------------------------

    fn tri_points(&self, f: FaceId) -> (V2, V2, V2) {
        let anchor = self.faces[f].anchor;
        let p1 = self.orig(anchor);
        let p2 = self.dest(anchor);
        let p3 = self.orig(self.lprev(anchor)); // Point3 = Anchor.LPrev.Orig
        (p1, p2, p3)
    }

    /// LinkedListTriangle.Reshape (LinkedListTriangle.cs:45-51).
    fn reshape(&mut self, f: FaceId, edge: EdgeId) {
        self.faces[f].anchor = edge;
        self.set_lface(edge, Some(f));
        let ln = self.lnext(edge);
        self.set_lface(ln, Some(f));
        let lp = self.lprev(edge);
        self.set_lface(lp, Some(f));
    }

    /// LinkedListTriangle.DontAnchor (LinkedListTriangle.cs:37-43).
    fn dont_anchor(&mut self, f: FaceId, edge: EdgeId) {
        if self.faces[f].anchor == edge {
            self.faces[f].anchor = self.lnext(edge);
        }
    }

    /// MakeFace + AllocFace (Subdivision.cs:298-303 / GreedySubdivision.cs:174-179).
    /// AllocFace inserts the new TrackedTriangle into the heap at -1, then
    /// Reshape sets it up; the candidate is recomputed by a later Update/ScanTriangle.
    fn make_face(&mut self, edge: EdgeId) -> FaceId {
        let f = self.faces.len();
        self.faces.push(Face {
            anchor: edge,
            next_face: self.first_face,
            token: NOT_IN_HEAP,
            cand_x: NO_CANDIDATE,
            cand_y: NO_CANDIDATE,
        });
        self.first_face = Some(f);
        // AllocFace: heap.Insert(tracked, -1f)
        self.heap.insert(&mut self.faces, f, -1.0);
        self.reshape(f, edge);
        f
    }

    // -----------------------------------------------------------------------
    // InitMesh (Subdivision.cs:253-279)
    // -----------------------------------------------------------------------

    fn init_mesh(&mut self, av: V2, bv: V2, cv: V2, dv: V2) {
        let edge = self.make_edge();
        self.set_endpoints(edge, av, bv);
        let edge2 = self.make_edge();
        self.splice(Self::sym(edge), edge2);
        self.set_endpoints(edge2, bv, cv);
        let edge3 = self.make_edge();
        self.splice(Self::sym(edge2), edge3);
        self.set_endpoints(edge3, cv, dv);
        let edge4 = self.make_edge();
        self.splice(Self::sym(edge3), edge4);
        self.set_endpoints(edge4, dv, av);
        self.splice(Self::sym(edge4), edge);
        let edge5 = self.make_edge();
        self.splice(Self::sym(edge4), edge5);
        self.splice(Self::sym(edge2), Self::sym(edge5));
        self.set_endpoints(edge5, av, cv);
        self.starting_edge = edge;
        self.first_face = None;
        let f1 = self.make_face(Self::sym(edge));
        self.update_face(f1);
        let f2 = self.make_face(Self::sym(edge3));
        self.update_face(f2);
    }

    // -----------------------------------------------------------------------
    // ShouldSwap / IsInterior (Subdivision.cs:27-40)
    // -----------------------------------------------------------------------

    fn should_swap(&self, point: V2, edge: EdgeId) -> bool {
        let oprev = self.oprev(edge);
        is_in_circle(self.orig(edge), self.dest(oprev), self.dest(edge), point)
    }

    fn is_interior(&self, edge: EdgeId) -> bool {
        if self.lnext(self.lnext(self.lnext(edge))) == edge {
            self.rnext(self.rnext(self.rnext(edge))) == edge
        } else {
            false
        }
    }

    // -----------------------------------------------------------------------
    // OnEdge / CCWBoundary (Subdivision.cs:336-357)
    // -----------------------------------------------------------------------

    fn on_edge(&self, point: V2, edge: EdgeId) -> bool {
        let orig = self.orig(edge);
        let dest = self.dest(edge);
        let length = point.sub(orig).length();
        let length2 = point.sub(dest).length();
        if length < EPS || length2 < EPS {
            return true;
        }
        let length3 = orig.sub(dest).length();
        if length > length3 || length2 > length3 {
            return false;
        }
        let line = Line::new(orig, dest);
        line.eval(point).abs() < EPS
    }

    fn ccw_boundary(&self, edge: EdgeId) -> bool {
        let op = self.oprev(edge);
        let opd = self.dest(op);
        !is_right_of(opd, self.orig(edge), self.dest(edge))
    }

    // -----------------------------------------------------------------------
    // Spoke (Subdivision.cs:42-103)
    // -----------------------------------------------------------------------

    fn spoke(&mut self, point: V2, mut edge: EdgeId) -> Option<EdgeId> {
        // up to 4 faces to reshape
        let mut faces_arr: [FaceId; 4] = [0; 4];
        let mut num = 0usize;
        if point.approx_eq(self.orig(edge)) || point.approx_eq(self.dest(edge)) {
            // tried to re-insert an existing point
            return None;
        }
        let mut edge2: Option<EdgeId> = None;
        let l_face = self.lface(edge).expect("spoke: edge has no LFace");
        self.dont_anchor(l_face, edge);
        faces_arr[num] = l_face;
        num += 1;
        if self.on_edge(point, edge) {
            if self.ccw_boundary(edge) {
                edge2 = Some(edge);
            } else {
                let sym = Self::sym(edge);
                let l_face2 = self.lface(sym).expect("spoke: sym has no LFace");
                faces_arr[num] = l_face2;
                num += 1;
                self.dont_anchor(l_face2, sym);
                edge = self.oprev(edge);
                let on = self.onext(edge);
                self.delete_edge(on);
            }
        }
        let mut edge3 = self.make_edge_pts(self.orig(edge), V2::new(point.x, point.y));
        self.splice(edge3, edge);
        self.starting_edge = edge3;
        loop {
            edge3 = self.connect(edge, Self::sym(edge3));
            edge = self.oprev(edge3);
            if self.lnext(edge) == self.starting_edge {
                break;
            }
        }
        if let Some(e2) = edge2 {
            self.delete_edge(e2);
        }
        edge3 = if edge2.is_some() {
            self.rprev(self.starting_edge)
        } else {
            Self::sym(self.starting_edge)
        };
        let stop = Self::sym(self.starting_edge);
        loop {
            if num != 0 {
                num -= 1;
                let f = faces_arr[num];
                self.reshape(f, edge3);
            } else {
                self.make_face(edge3);
            }
            edge3 = self.onext(edge3);
            if edge3 == stop {
                break;
            }
        }
        Some(self.starting_edge)
    }

    // -----------------------------------------------------------------------
    // Optimize (Subdivision.cs:105-130)
    // -----------------------------------------------------------------------

    fn optimize(&mut self, point: V2, edge: EdgeId) {
        let mut edge2 = edge;
        loop {
            let lnext = self.lnext(edge2);
            if self.is_interior(lnext) && self.should_swap(point, lnext) {
                self.swap_edge(lnext);
                continue;
            }
            edge2 = self.onext(edge2);
            if edge2 == edge {
                break;
            }
        }
        edge2 = edge;
        loop {
            let lnext2 = self.lnext(edge2);
            if let Some(f) = self.lface(lnext2) {
                self.update_face(f);
            }
            edge2 = self.onext(edge2);
            if edge2 == edge {
                break;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Locate (Subdivision.cs:132-188)
    // -----------------------------------------------------------------------

    fn locate(&mut self, point: V2) -> EdgeId {
        let hint = self.starting_edge;
        self.locate_hint(point, hint)
    }

    fn locate_hint(&mut self, point: V2, edge_hint: EdgeId) -> EdgeId {
        let mut edge = edge_hint;
        let mut num = tri_area(point, self.dest(edge), self.orig(edge));
        if num > 0.0 {
            num *= -1.0;
            edge = Self::sym(edge);
        }
        loop {
            let onext = self.onext(edge);
            let dprev = self.dprev(edge);
            let num2 = tri_area(point, self.dest(onext), self.orig(onext));
            let num3 = tri_area(point, self.dest(dprev), self.orig(dprev));
            if num3 > 0.0 {
                if num2 > 0.0 || (num2 == 0.0 && num == 0.0) {
                    self.starting_edge = edge;
                    return edge;
                }
                num = num2;
                edge = onext;
            } else if num2 > 0.0 {
                if num3 == 0.0 && num == 0.0 {
                    break;
                }
                num = num3;
                edge = dprev;
            } else if num == 0.0 && !is_left_of(self.dest(onext), self.orig(edge), self.dest(edge))
            {
                edge = Self::sym(edge);
            } else {
                // DETERMINISM DEVIATION (Subdivision.cs:175): C# uses `rand.Next() & 1`.
                // We always take the oNext branch (deterministic).
                num = num2;
                edge = onext;
            }
        }
        self.starting_edge = edge;
        edge
    }

    // -----------------------------------------------------------------------
    // Insert (Subdivision.cs:190-204)
    // -----------------------------------------------------------------------

    fn insert(&mut self, point: V2, tri: Option<FaceId>) -> Option<EdgeId> {
        let edge = match tri {
            Some(f) => {
                let anchor = self.faces[f].anchor;
                self.locate_hint(point, anchor)
            }
            None => self.locate(point),
        };
        let edge2 = self.spoke(point, edge);
        if let Some(e2) = edge2 {
            self.optimize(point, Self::sym(e2));
        }
        edge2
    }

    // -----------------------------------------------------------------------
    // Greedy: Select / ScanTriangle / GreedyInsert / MaxError
    // (GreedySubdivision.cs:53-179)
    // -----------------------------------------------------------------------

    fn select(&mut self, sub_x: i32, sub_y: i32, tri: Option<FaceId>) -> Option<EdgeId> {
        if self.state(sub_x as usize, sub_y as usize) == PointState::Used {
            return None;
        }
        self.set_state(sub_x as usize, sub_y as usize, PointState::Used);
        self.point_count += 1;
        self.insert(V2::new(sub_x as f32, sub_y as f32), tri)
    }

    /// TrackedTriangle.Update -> ScanTriangle (TrackedTriangle.cs:22-35).
    fn update_face(&mut self, f: FaceId) {
        self.scan_triangle(f);
    }

    /// ScanTriangle (GreedySubdivision.cs:73-122).
    fn scan_triangle(&mut self, f: FaceId) {
        let (p1, p2, p3) = self.tri_points(f);
        let plane = self.compute_plane(p1, p2, p3);
        let mut array = [p1, p2, p3];
        // Vector2TriangleComparer: sort by y only (stable). C# Array.Sort is NOT
        // stable, but the comparer returns 0 for equal-y so order among equal-y
        // entries is unspecified in C#; use a stable sort_by for determinism.
        array.sort_by(|a, b| {
            if a.y == b.y {
                std::cmp::Ordering::Equal
            } else if a.y < b.y {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            }
        });

        let mut num = array[0].x; // x1 left edge
        let mut num2 = array[0].x; // x2 right edge
        let num3 = (array[1].x - array[0].x) / (array[1].y - array[0].y);
        let num4 = (array[2].x - array[0].x) / (array[2].y - array[0].y);
        let mut candidate = Candidate::new();

        let num5 = array[0].y as i32;
        let num6 = array[1].y as i32;
        let mut i = num5;
        while i < num6 {
            self.scan_triangle_line(&plane, i, num, num2, &mut candidate);
            num += num3;
            num2 += num4;
            i += 1;
        }

        let num3b = (array[2].x - array[1].x) / (array[2].y - array[1].y);
        let mut num = array[1].x;
        let num5b = array[1].y as i32;
        let num6b = array[2].y as i32;
        let mut j = num5b;
        while j <= num6b {
            self.scan_triangle_line(&plane, j, num, num2, &mut candidate);
            num += num3b;
            num2 += num4;
            j += 1;
        }

        if candidate.importance < 0.0001 {
            let token = self.faces[f].token;
            if token != NOT_IN_HEAP {
                self.heap.kill(&mut self.faces, token as usize);
            }
            self.faces[f].cand_x = NO_CANDIDATE;
            self.faces[f].cand_y = NO_CANDIDATE;
        } else {
            self.faces[f].cand_x = candidate.x;
            self.faces[f].cand_y = candidate.y;
            if self.faces[f].token == NOT_IN_HEAP {
                self.heap.insert(&mut self.faces, f, candidate.importance);
            } else {
                self.heap.update(&mut self.faces, f, candidate.importance);
            }
        }
    }

    fn compute_plane(&self, p1: V2, p2: V2, p3: V2) -> Plane {
        let z1 = self.eval_height(p1.x as i32, p1.y as i32);
        let z2 = self.eval_height(p2.x as i32, p2.y as i32);
        let z3 = self.eval_height(p3.x as i32, p3.y as i32);
        Plane::init([p1.x, p1.y, z1], [p2.x, p2.y, z2], [p3.x, p3.y, z3])
    }

    /// ScanTriangleLine (GreedySubdivision.cs:192-212).
    fn scan_triangle_line(
        &self,
        plane: &Plane,
        y: i32,
        x1: f32,
        x2: f32,
        candidate: &mut Candidate,
    ) {
        let num = x1.min(x2).ceil() as i32;
        let num2 = x1.max(x2).floor() as i32;
        if num > num2 {
            return;
        }
        let mut num3 = plane.eval_i(num, y);
        let a = plane.a;
        let mut i = num;
        while i <= num2 {
            let st = self.state(i as usize, y as usize);
            if st != PointState::Used && st != PointState::Ignored {
                let num4 = self.eval_height(i, y);
                let importance = (num4 - num3).abs();
                candidate.consider(i, y, importance);
            }
            num3 += a;
            i += 1;
        }
    }

    /// GreedyInsert (GreedySubdivision.cs:124-142).
    fn greedy_insert(&mut self) -> bool {
        let node = self.heap.extract(&mut self.faces);
        let node = match node {
            None => return false,
            Some(n) => n,
        };
        let f = node.face;
        let x = self.faces[f].cand_x;
        let y = self.faces[f].cand_y;
        self.select(x, y, Some(f));
        true
    }

    /// MaxError (GreedySubdivision.cs:144-147).
    pub fn max_error(&self) -> f32 {
        self.heap.top().map(|n| n.importance).unwrap_or(0.0)
    }

    // -----------------------------------------------------------------------
    // Terrain driver (Terrain.cs)
    // -----------------------------------------------------------------------

    /// ScriptedPreInsertion (Terrain.cs:40-66). state 0 = mark Ignored.
    pub fn scripted_pre_insertion(&mut self, indices: &[(usize, usize)], state: i32) {
        for &(x, y) in indices {
            let cur = self.state(x, y);
            if cur != PointState::Unused && cur != PointState::Ignored {
                continue;
            }
            match state {
                0 => {
                    self.set_state(x, y, PointState::Ignored);
                    continue;
                }
                -1 => continue,
                _ => {}
            }
            if self.state(x, y) != PointState::Used {
                self.select(x as i32, y as i32, None);
                self.set_state(x, y, PointState::Used);
            }
        }
    }

    /// Triangulate (Terrain.cs:68-73).
    pub fn triangulate(&mut self) {
        while !self.goal_met() && self.greedy_insert() {}
    }

    /// GoalMet (Terrain.cs:156-163). Order matters: error<threshold first.
    fn goal_met(&self) -> bool {
        if !(self.max_error() < self.error_threshold) {
            return (self.point_count as i64) > self.point_count_limit;
        }
        true
    }

    pub fn point_count(&self) -> u32 {
        self.point_count
    }

    /// GenerateOutput (Terrain.cs:75-97): kept posts -> verts (x,y,Eval),
    /// triangles emitted as index triples into the dedup'd vertex list.
    pub fn generate_output(&self) -> (Vec<[f32; 3]>, Vec<[usize; 3]>) {
        let mut vertices: Vec<[f32; 3]> = Vec::with_capacity(self.point_count as usize);
        let mut indices: Vec<usize> = Vec::with_capacity(self.point_count as usize * 3);
        let mut vertex_positions = vec![-1i64; self.width * self.height];

        // OverFaces: walk the face linked list (Subdivision.cs:245-251).
        let mut cur = self.first_face;
        while let Some(f) = cur {
            let (p1, p2, p3) = self.tri_points(f);
            for v in [p1, p2, p3] {
                let idx = (v.x as usize) + (v.y as usize) * self.width;
                if vertex_positions[idx] == -1 {
                    vertex_positions[idx] = vertices.len() as i64;
                    let z = self.eval_height(v.x as i32, v.y as i32);
                    vertices.push([v.x, v.y, z]);
                }
                indices.push(vertex_positions[idx] as usize);
            }
            cur = self.faces[f].next_face;
        }

        let tris: Vec<[usize; 3]> = indices
            .chunks_exact(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect();
        (vertices, tris)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize) -> Vec<f32> {
        vec![0.0; w * h]
    }

    #[test]
    fn flat_decimates_to_two_triangles() {
        let w = 9;
        let h = 9;
        // error threshold high so only corners survive; a plane fits everything exactly
        let mut t = Terra::new(0.5, -1, w, h, &flat(w, h));
        t.triangulate();
        let (verts, tris) = t.generate_output();
        assert_eq!(verts.len(), 4, "flat map keeps only 4 corners");
        assert_eq!(tris.len(), 2, "flat map = 2 triangles");
    }

    #[test]
    fn spike_is_inserted() {
        let w = 9;
        let h = 9;
        let mut heights = flat(w, h);
        let (sx, sy) = (4usize, 4usize);
        heights[sx + sy * w] = 1000.0; // sharp central spike
        let mut t = Terra::new(0.5, -1, w, h, &heights);
        t.triangulate();
        let (verts, _tris) = t.generate_output();
        // the spike post must be present in the output verts
        assert!(
            verts
                .iter()
                .any(|v| v[0] as usize == sx && v[1] as usize == sy && v[2] == 1000.0),
            "spike vertex must be selected"
        );
        // Exactly 9 verts: 4 corners + spike + its 4 diagonal "shoulder" posts.
        // The shoulders are forced in because the plane from spike->corner has
        // large residual error at the intermediate posts; this is the faithful
        // Garland-Heckbert result (max_error -> 0).
        assert_eq!(verts.len(), 9, "got {} verts", verts.len());
        let coords: std::collections::BTreeSet<(usize, usize)> = verts
            .iter()
            .map(|v| (v[0] as usize, v[1] as usize))
            .collect();
        for shoulder in [(3, 3), (5, 5), (3, 5), (5, 3)] {
            assert!(
                coords.contains(&shoulder),
                "shoulder post {shoulder:?} must be selected; got {coords:?}"
            );
        }
        // residual error fully eliminated below threshold
        assert!(t.max_error() < 0.5, "max_error {}", t.max_error());
    }

    #[test]
    fn point_count_cap_honored() {
        let w = 17;
        let h = 17;
        // noisy map (every post differs) with a tight error threshold would insert many;
        // cap at 10 must stop the greedy loop (Terrain.GoalMet: PointCount > limit)
        let mut heights = vec![0.0; w * h];
        for i in 0..(w * h) {
            heights[i] = (i as f32 * 37.0) % 100.0;
        }
        let mut t = Terra::new(0.01, 10, w, h, &heights);
        t.triangulate();
        // PointCountLimit=10 ; loop stops once PointCount > 10, so count is 11
        assert!(t.point_count() <= 11, "point_count {}", t.point_count());
    }

    /// A planar ramp is exactly representable by 2 triangles; the plane fit and
    /// scan-conversion must report zero error so only the 4 corners survive.
    #[test]
    fn planar_ramp_keeps_only_corners() {
        let w = 13;
        let h = 13;
        let mut heights = vec![0.0; w * h];
        for y in 0..h {
            for x in 0..w {
                // z = 3*x + 5*y + 7 — an exact plane
                heights[x + y * w] = 3.0 * x as f32 + 5.0 * y as f32 + 7.0;
            }
        }
        let mut t = Terra::new(0.5, -1, w, h, &heights);
        t.triangulate();
        let (verts, tris) = t.generate_output();
        assert_eq!(verts.len(), 4, "planar ramp keeps only 4 corners");
        assert_eq!(tris.len(), 2);
    }

    /// The quality guarantee: when no point-count cap binds, the greedy loop runs
    /// until MaxError < threshold (Terrain.GoalMet). Validates that the bound holds
    /// on a structured terrain.
    #[test]
    fn max_error_bound_satisfied() {
        let w = 17;
        let h = 17;
        let mut heights = vec![0.0; w * h];
        for y in 0..h {
            for x in 0..w {
                // a smooth bumpy surface
                let fx = x as f32;
                let fy = y as f32;
                heights[x + y * w] = 40.0 * (fx * 0.6).sin() + 30.0 * (fy * 0.4).cos();
            }
        }
        let threshold = 5.0;
        let mut t = Terra::new(threshold, -1, w, h, &heights);
        t.triangulate();
        // greedy stopped because error dropped below threshold (no cap binds w*h limit)
        assert!(
            t.max_error() < threshold,
            "max_error {} not below threshold {}",
            t.max_error(),
            threshold
        );
        // and it actually decimated (kept far fewer than the full 17*17=289 posts)
        let (verts, _tris) = t.generate_output();
        assert!(verts.len() < w * h, "no decimation: {} verts", verts.len());
    }

    /// Repo rule: no nondeterminism. The deterministic Locate tie-break must make
    /// two identical runs produce byte-identical vertex/triangle output.
    #[test]
    fn output_is_deterministic() {
        let w = 17;
        let h = 17;
        let mut heights = vec![0.0; w * h];
        for i in 0..(w * h) {
            heights[i] = ((i as f32 * 13.0) % 50.0) + (i as f32 * 0.1).sin() * 7.0;
        }
        let run = || {
            let mut t = Terra::new(2.0, -1, w, h, &heights);
            t.triangulate();
            t.generate_output()
        };
        let (v1, t1) = run();
        let (v2, t2) = run();
        assert_eq!(v1.len(), v2.len());
        assert_eq!(t1, t2, "triangles must match across runs");
        for (a, b) in v1.iter().zip(v2.iter()) {
            assert_eq!(a, b, "vertex mismatch across runs");
        }
    }

    /// ScriptedPreInsertion(state=0) marks posts Ignored; the scan-conversion
    /// candidate search then skips Ignored posts (Terrain.cs:40-66,
    /// GreedySubdivision.cs:204), so an ignored secondary feature is never selected
    /// even though it otherwise would be.
    ///
    /// As in the C#, an Ignored post that is already the global-max stale candidate
    /// from the InitMesh scan can still be inserted (Select only guards Used, not
    /// Ignored). A dominant primary spike keeps the ignored bump off the heap top
    /// until its containing triangle is re-scanned, avoiding that case.
    #[test]
    fn ignored_posts_are_never_inserted() {
        let w = 17;
        let h = 17;
        let mut heights = flat(w, h);
        // dominant primary spike (drives the early inserts / re-scans)
        heights[8 + 8 * w] = 5000.0;
        // secondary bumps we will ignore — large enough to be selected normally
        let ignored = [(3usize, 3usize), (3, 4), (4, 3), (4, 4)];
        for &(x, y) in &ignored {
            heights[x + y * w] = 800.0;
        }

        // baseline: WITHOUT ignoring, at least one secondary bump is selected
        let mut base = Terra::new(2.0, -1, w, h, &heights);
        base.triangulate();
        let (bverts, _) = base.generate_output();
        assert!(
            ignored.iter().any(|&(x, y)| bverts
                .iter()
                .any(|v| v[0] as usize == x && v[1] as usize == y)),
            "baseline should select a secondary bump"
        );

        // with the bumps Ignored, none of them may be selected
        let mut t = Terra::new(2.0, -1, w, h, &heights);
        t.scripted_pre_insertion(&ignored, 0);
        t.triangulate();
        let (verts, _tris) = t.generate_output();
        for &(x, y) in &ignored {
            assert!(
                !verts
                    .iter()
                    .any(|v| v[0] as usize == x && v[1] as usize == y),
                "ignored post ({x},{y}) must not be selected"
            );
        }
        // the dominant spike is still selected
        assert!(
            verts
                .iter()
                .any(|v| v[0] as usize == 8 && v[1] as usize == 8),
            "dominant spike must still be selected"
        );
    }
}
