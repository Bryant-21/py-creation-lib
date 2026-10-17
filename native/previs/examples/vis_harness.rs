//! Offline visibility harness: walks a tome's portal graph from camera samples
//! the way the runtime portal culler does (a perspective frustum clipped by
//! every portal it passes, in screen-space rectangles) and counts the objects
//! whose bounds stay inside the clipped view.
//!
//! `cargo run --release --example vis_harness -- [options] <a.uvd>[=<a.scene>] [<b.uvd>[=<b.scene>]]`
//!
//! - `--seeds <file.scene>`: camera positions are the scene's point view volumes
//!   (CK's navmesh seeds, already at eye height), raised by `--eye` (default 0).
//! - `--at x y z`: an explicit camera position (repeatable); printed per direction.
//! - `--stride n`: keep every n-th seed (default 1).
//! - `--yaws n` (default 8) and `--pitches a,b,..` in degrees (default 0).
//! - `--heading h x` (repeatable): an explicit direction as the game's `GetAngle Z`
//!   and `GetAngle X` give it (degrees clockwise from north; positive tilts down).
//!   `LIST=1` prints each explicit camera's visible IDs (first tome).
//! - `--gates open|closed` (default closed): whether user (door) portals pass.
//! - `--distance`: cull objects beyond their object-distance record.
//! - `--no-expand`: ignore the tiles' portal expand (portals grow by it by default).
//! - `--lod <factor>`: walk the runtime's LOD cut (coarse inner tiles beyond
//!   `factor` times their LOD level) instead of leaf tiles only.
//! - `--count-truth`: also count every object with a clear line of sight (slow).
//! - `--fov-y <deg>` (default 55.4, FO4's 70 degree 4:3 world FOV) and `--aspect` (16/9).
//!
//! A tome given as `<uvd>=<scene>` is also checked against ground truth: every
//! object it hides that has a vertex inside the frustum with a clear line of
//! sight past the scene's occluder triangles counts as wrongly culled.
//!
//! With two tomes it also reports the objects the first shows and the second
//! hides: by user ID for reference FormIDs both list, else by whether the
//! second tome's walk exposes the object's box at all.
use std::collections::{HashMap, HashSet};

use previs_native::scene::{INSTANCE_OCCLUDER, Scene};
use previs_native::umbra_query::{CellHit, KdView, cell_bounds, cell_objects, find_cell, portal_quad};
use previs_native::umbra_tome::{Portal, Tile, Tome, parse};
use rayon::prelude::*;

type Vec3 = [f32; 3];
type Rect = [f32; 4];
const SCREEN: Rect = [-1.0, 1.0, -1.0, 1.0];
const NEAR: f32 = 4.0;
/// Portals clip just in front of the eye, not at `NEAR`: a ray through a
/// portal point closer than the near plane still sees what lies beyond it.
const PORTAL_NEAR: f32 = 0.01;
/// Rects kept per reached cell before they collapse into their bounding rect.
const MAX_RECTS: usize = 24;
/// CK lists an object in cells up to 32 units from its box (every CK listing
/// in the Whitespring bunker tome is), so a box counts as exposed by a
/// reached cell that close.
const LIST_SLACK: f32 = 32.0;
/// Line-of-sight probes per object.
const PROBES: usize = 24;
/// Half the spread of the eyes a line of sight must be clear from.
const BEAM: f32 = 6.0;

#[derive(Clone, Copy)]
struct Camera {
    eye: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    tan_x: f32,
    tan_y: f32,
}

impl Camera {
    fn new(eye: Vec3, yaw: f32, pitch: f32, fov_y: f32, aspect: f32) -> Self {
        let forward = [pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), pitch.sin()];
        let right = normalize(cross(&forward, &[0.0, 0.0, 1.0]));
        let up = cross(&right, &forward);
        let tan_y = (fov_y * 0.5).tan();
        Camera { eye, right, up, forward, tan_x: tan_y * aspect, tan_y }
    }

    fn view(&self, p: &Vec3) -> Vec3 {
        let d = sub(p, &self.eye);
        [dot(&d, &self.right), dot(&d, &self.up), dot(&d, &self.forward)]
    }

    fn project(&self, v: &Vec3) -> (f32, f32) {
        (v[0] / (v[2] * self.tan_x), v[1] / (v[2] * self.tan_y))
    }

    /// Screen rect of a convex polygon (view space) clipped at depth `near`.
    fn polygon_rect(&self, points: &[Vec3], near: f32) -> Option<Rect> {
        let mut rect: Option<Rect> = None;
        let mut add = |p: &Vec3| {
            let (x, y) = self.project(p);
            rect = Some(rect.map_or([x, x, y, y], |r| [r[0].min(x), r[1].max(x), r[2].min(y), r[3].max(y)]));
        };
        for i in 0..points.len() {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            if a[2] >= near {
                add(&a);
            }
            if (a[2] >= near) != (b[2] >= near) {
                let t = (near - a[2]) / (b[2] - a[2]);
                add(&[a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, near]);
            }
        }
        rect
    }

    /// Screen rect of a box; the whole screen when it reaches the near plane.
    fn box_rect(&self, lo: &Vec3, hi: &Vec3) -> Option<Rect> {
        let mut rect = [f32::MAX, f32::MIN, f32::MAX, f32::MIN];
        let mut behind = 0;
        for k in 0..8 {
            let p = [if k & 1 != 0 { hi[0] } else { lo[0] }, if k & 2 != 0 { hi[1] } else { lo[1] }, if k & 4 != 0 { hi[2] } else { lo[2] }];
            let v = self.view(&p);
            if v[2] < NEAR {
                behind += 1;
                continue;
            }
            let (x, y) = self.project(&v);
            rect = [rect[0].min(x), rect[1].max(x), rect[2].min(y), rect[3].max(y)];
        }
        match behind {
            8 => None,
            0 => intersect(&rect, &SCREEN),
            _ => Some(SCREEN),
        }
    }

    fn sees_point(&self, p: &Vec3) -> bool {
        let v = self.view(p);
        if v[2] < NEAR {
            return false;
        }
        let (x, y) = self.project(&v);
        x.abs() <= 1.0 && y.abs() <= 1.0
    }
}

fn sub(a: &Vec3, b: &Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: &Vec3, b: &Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: &Vec3, b: &Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: Vec3) -> Vec3 {
    let l = dot(&v, &v).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}

fn intersect(a: &Rect, b: &Rect) -> Option<Rect> {
    let r = [a[0].max(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].min(b[3])];
    (r[0] <= r[1] && r[2] <= r[3]).then_some(r)
}

fn contains(outer: &Rect, inner: &Rect) -> bool {
    outer[0] <= inner[0] && inner[1] <= outer[1] && outer[2] <= inner[2] && inner[3] <= outer[3]
}

fn union(a: &Rect, b: &Rect) -> Rect {
    [a[0].min(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].max(b[3])]
}

/// Occluder triangles in a bounding volume hierarchy, for line-of-sight tests.
struct Occluders {
    triangles: Vec<[Vec3; 3]>,
    nodes: Vec<Node>,
}

/// `count == 0`: inner node with children `first` and `first + 1`;
/// otherwise a leaf over `triangles[first..first + count]`.
struct Node {
    lo: Vec3,
    hi: Vec3,
    first: u32,
    count: u32,
}

impl Occluders {
    fn new(mut triangles: Vec<[Vec3; 3]>) -> Self {
        let mut nodes = vec![Node { lo: [0.0; 3], hi: [0.0; 3], first: 0, count: 0 }];
        let mut stack = vec![(0usize, 0usize, triangles.len())];
        while let Some((node, start, end)) = stack.pop() {
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for t in &triangles[start..end] {
                for p in t {
                    for a in 0..3 {
                        lo[a] = lo[a].min(p[a]);
                        hi[a] = hi[a].max(p[a]);
                    }
                }
            }
            nodes[node].lo = lo;
            nodes[node].hi = hi;
            if end - start <= 4 {
                nodes[node].first = start as u32;
                nodes[node].count = (end - start) as u32;
                continue;
            }
            let axis = (0..3).max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b]))).unwrap();
            let mid = (start + end) / 2;
            let centre = |t: &[Vec3; 3]| t[0][axis] + t[1][axis] + t[2][axis];
            triangles[start..end].select_nth_unstable_by(mid - start, |a, b| centre(a).total_cmp(&centre(b)));
            let first = nodes.len();
            nodes[node].first = first as u32;
            for _ in 0..2 {
                nodes.push(Node { lo: [0.0; 3], hi: [0.0; 3], first: 0, count: 0 });
            }
            stack.push((first, start, mid));
            stack.push((first + 1, mid, end));
        }
        Occluders { triangles, nodes }
    }

    /// Whether any triangle crosses the segment `from + t * dir` for `t` in `(0, t_max)`.
    fn blocked(&self, from: &Vec3, dir: &Vec3, t_max: f32) -> bool {
        let inv = dir.map(|d| 1.0 / d);
        let mut stack = vec![0usize];
        while let Some(index) = stack.pop() {
            let node = &self.nodes[index];
            let (mut t0, mut t1) = (0.0f32, t_max);
            for a in 0..3 {
                let (mut near, mut far) = ((node.lo[a] - from[a]) * inv[a], (node.hi[a] - from[a]) * inv[a]);
                if near > far {
                    std::mem::swap(&mut near, &mut far);
                }
                t0 = t0.max(near);
                t1 = t1.min(far);
            }
            if t0 > t1 {
                continue;
            }
            if node.count == 0 {
                stack.push(node.first as usize);
                stack.push(node.first as usize + 1);
                continue;
            }
            for t in &self.triangles[node.first as usize..(node.first + node.count) as usize] {
                let (e1, e2) = (sub(&t[1], &t[0]), sub(&t[2], &t[0]));
                let p = cross(dir, &e2);
                let det = dot(&e1, &p);
                if det.abs() < 1e-12 {
                    continue;
                }
                let s = sub(from, &t[0]);
                let u = dot(&s, &p) / det;
                if !(0.0..=1.0).contains(&u) {
                    continue;
                }
                let q = cross(&s, &e1);
                let v = dot(dir, &q) / det;
                if v < 0.0 || u + v > 1.0 {
                    continue;
                }
                let hit = dot(&e2, &q) / det;
                if hit > 1e-3 && hit < t_max {
                    return true;
                }
            }
        }
        false
    }
}

/// Serialized scene flag of gate (door) instances.
const INSTANCE_GATE: u32 = 0x10;

/// A scene's occluders (with its gates when they are closed) plus up to
/// [`PROBES`] surface points per target, by user ID.
struct Truth {
    occluders: Occluders,
    probes: HashMap<u32, Vec<Vec3>>,
}

impl Truth {
    fn load(path: &str, gates_open: bool) -> Self {
        let scene = Scene::decode(&std::fs::read(path).unwrap()).unwrap();
        let blocking = if gates_open { INSTANCE_OCCLUDER } else { INSTANCE_OCCLUDER | INSTANCE_GATE };
        let world = |t: &[f32; 12], p: &Vec3| -> Vec3 {
            std::array::from_fn(|a| t[a * 4] * p[0] + t[a * 4 + 1] * p[1] + t[a * 4 + 2] * p[2] + t[a * 4 + 3])
        };
        let mut triangles = Vec::new();
        let mut probes = HashMap::new();
        for instance in &scene.instances {
            let geometry = &scene.geometries[instance.model_index as usize];
            if instance.flags & blocking != 0 {
                let points: Vec<Vec3> = geometry.vertices.iter().map(|v| world(&instance.transform, v)).collect();
                triangles.extend(geometry.triangles.iter().map(|t| t.map(|i| points[i as usize])));
            }
            let step = geometry.vertices.len().div_ceil(PROBES).max(1);
            let points: Vec<Vec3> = geometry.vertices.iter().step_by(step).map(|v| world(&instance.transform, v)).collect();
            probes.entry(instance.object_id).or_insert_with(Vec::new).extend(points);
        }
        Truth { occluders: Occluders::new(triangles), probes }
    }

    /// Whether some probe of `id` is in view with nothing in between.
    /// The first probe of `id` in view with a clear beam to it: five rays from
    /// eyes up to [`BEAM`] units apart, so seams between meshes, which the
    /// voxelization rightly closes, do not count as openings.
    fn clear_probe(&self, camera: &Camera, id: u32) -> Option<Vec3> {
        let eyes: Vec<Vec3> = [(0.0, 0.0), (1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)]
            .iter()
            .map(|&(r, u)| std::array::from_fn(|a| camera.eye[a] + BEAM * (r * camera.right[a] + u * camera.up[a])))
            .collect();
        self.probes.get(&id)?.iter().copied().find(|p| {
            camera.sees_point(p)
                && eyes.iter().all(|eye| {
                    let dir = sub(p, eye);
                    let length = dot(&dir, &dir).sqrt();
                    // Stop short of the surface the probe lies on.
                    length > 2.0 && !self.occluders.blocked(eye, &dir, 1.0 - 2.0 / length)
                })
        })
    }
}

#[derive(Clone, Copy)]
struct Options {
    gates_open: bool,
    distance: bool,
    expand: bool,
    /// Also count every object with a clear line of sight (slow).
    count_truth: bool,
    /// Walk the runtime's LOD cut with this distance factor instead of leaf tiles only.
    lod: Option<f32>,
}

/// Decoded tomes walked as one collection, with per-cell object lists
/// expanded once. Cells and objects are numbered across the tomes in order.
struct Vis {
    tomes: Vec<Tome>,
    /// Per tome, the global index of each tile's first cell (and the end).
    starts: Vec<Vec<usize>>,
    /// Global index of each tome's first object.
    object_starts: Vec<u32>,
    objects: Vec<Vec<u32>>,
    by_id: HashMap<u32, u32>,
    truth: Option<Truth>,
}

impl Vis {
    /// `a.uvd[+b.uvd...][=scene]`.
    fn load(spec: &str, gates_open: bool) -> Self {
        let (paths, truth) = spec.split_once('=').map_or((spec, None), |(t, s)| (t, Some(s)));
        let tomes: Vec<Tome> = paths.split('+').map(|path| parse(&std::fs::read(path).unwrap()).unwrap()).collect();
        let (mut starts, mut object_starts, mut objects, mut by_id) = (Vec::new(), Vec::new(), Vec::new(), HashMap::new());
        let mut first_object = 0u32;
        for tome in &tomes {
            let base = objects.len();
            let own: Vec<usize> = tome.cell_starts.iter().flatten().map(|&s| base + s as usize).collect();
            objects.resize(*own.last().unwrap_or(&base), Vec::new());
            for (t, tile) in tome.tiles.iter().enumerate() {
                let Some(tile) = tile else { continue };
                for c in 0..tile.num_cells() {
                    let list = &mut objects[own[t] + c];
                    cell_objects(tome, tile, c as u32, list).unwrap();
                    list.iter_mut().for_each(|o| *o += first_object);
                }
            }
            for (i, &id) in tome.user_ids.iter().flatten().enumerate() {
                by_id.entry(id).or_insert(first_object + i as u32);
            }
            starts.push(own);
            object_starts.push(first_object);
            first_object += tome.num_objects;
        }
        Vis { tomes, starts, object_starts, objects, by_id, truth: truth.map(|t| Truth::load(t, gates_open)) }
    }

    /// The tome holding global object `object`, and its index there.
    fn object(&self, object: u32) -> (&Tome, usize) {
        let k = self.object_starts.partition_point(|&s| s <= object) - 1;
        (&self.tomes[k], (object - self.object_starts[k]) as usize)
    }

    fn num_objects(&self) -> u32 {
        self.tomes.iter().map(|t| t.num_objects).sum()
    }

    fn user_id(&self, object: u32) -> u32 {
        let (tome, o) = self.object(object);
        tome.user_ids.as_ref().unwrap()[o]
    }

    fn object_box(&self, object: u32) -> (Vec3, Vec3) {
        let (tome, o) = self.object(object);
        let b = tome.object_bounds.as_ref().unwrap()[o];
        ([b[0], b[1], b[2]], [b[3], b[4], b[5]])
    }

    fn beyond_distance(&self, object: u32, eye: &Vec3) -> bool {
        let (tome, o) = self.object(object);
        let Some(record) = tome.object_distances.as_ref().and_then(|d| d.get(o)) else { return false };
        let f = record.map(f32::from_bits);
        let d2: f32 = (0..3).map(|a| (f[a] - eye[a]).max(eye[a] - f[a + 4]).max(0.0).powi(2)).sum();
        d2 > f[7]
    }

    /// `(tome, tile, cell)` of a global cell.
    fn cell_of(&self, global: usize) -> (usize, usize, u32) {
        let k = self.starts.partition_point(|s| s[0] <= global) - 1;
        let tile = self.starts[k].partition_point(|&s| s <= global) - 1;
        (k, tile, (global - self.starts[k][tile]) as u32)
    }

    fn global(&self, k: usize, tile: u32, cell: u32) -> usize {
        self.starts[k][tile as usize] + cell as usize
    }

    fn tile(&self, k: usize, tile: usize) -> &Tile {
        self.tomes[k].tiles[tile].as_ref().unwrap()
    }

    /// The first tome with a cell at `p`: `(tome, tile, cell)`.
    fn locate(&self, p: Vec3) -> Option<(usize, u32, u32)> {
        self.tomes.iter().enumerate().find_map(|(k, tome)| match find_cell(tome, p).ok()? {
            CellHit::Cell { tile, cell } => Some((k, tile, cell)),
            _ => None,
        })
    }
}

/// Every reached cell (global index) with the screen rects it is seen through.
struct Walk {
    reached: HashMap<usize, Vec<Rect>>,
    /// The cell each cell was first entered from.
    via: HashMap<usize, usize>,
}

/// Tiles the runtime's start-cell walk (`TileTraverseTree`) marks traversable:
/// it descends every untiled node and every node within `factor` times its LOD
/// level of the eye, and stops at the first tiled node beyond that.
fn lod_cut(tome: &Tome, eye: &Vec3, factor: f32) -> Vec<bool> {
    let kd = KdView::new(&tome.tree).unwrap();
    let lods = tome.tile_lod_levels.as_deref().unwrap_or(&[]);
    let mut boxes = vec![([0.0f32; 3], [0.0f32; 3]); kd.node_count as usize];
    kd.walk(tome.tree_min, tome.tree_max, |n, lo, hi, _| boxes[n as usize] = (*lo, *hi)).unwrap();
    let mut cut = vec![false; kd.node_count as usize];
    let mut stack = vec![0u32];
    while let Some(node) = stack.pop() {
        let (lo, hi) = boxes[node as usize];
        let tiled = tome.tiles.get(node as usize).is_some_and(Option::is_some);
        let distance = (0..3).map(|a| (lo[a] - eye[a]).max(eye[a] - hi[a]).max(0.0).powi(2)).sum::<f32>().sqrt();
        let lod = lods.get(node as usize).map_or(-1.0, |&b| f32::from_bits(b));
        if kd.is_leaf(node) || (tiled && lod >= 0.0 && distance > lod * factor) {
            cut[node as usize] = tiled;
        } else {
            let (a, b) = kd.children(node);
            stack.extend([a, b]);
        }
    }
    cut
}

/// The screen rect a non-gate portal of `tile` is seen through, or `None`
/// when the camera is past its plane.
fn portal_seen(camera: &Camera, tile: &Tile, p: &Portal, options: Options) -> Option<Option<Rect>> {
    let (plane, mut lo, mut hi) = portal_quad(&tile.tree_min, &tile.tree_max, p);
    let axis = (p.face() >> 1) as usize;
    let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
    if options.expand {
        for k in 0..2 {
            lo[k] -= tile.portal_expand;
            hi[k] += tile.portal_expand;
        }
    }
    let side = camera.eye[axis] - plane;
    // The target lies on the portal's face side; a camera already past
    // the plane looks back through it and cannot see beyond.
    if if p.face() & 1 == 1 { side > NEAR } else { side < -NEAR } {
        return None;
    }
    let over = (camera.eye[a] - lo[0]).min(hi[0] - camera.eye[a]).min(camera.eye[b] - lo[1]).min(hi[1] - camera.eye[b]);
    if side.abs() <= NEAR && over >= -NEAR {
        return Some(Some(SCREEN));
    }
    let corner = |u: f32, v: f32| {
        let mut q = [0.0; 3];
        q[axis] = plane;
        q[a] = u;
        q[b] = v;
        camera.view(&q)
    };
    Some(camera.polygon_rect(&[corner(lo[0], lo[1]), corner(hi[0], lo[1]), corner(hi[0], hi[1]), corner(lo[0], hi[1])], PORTAL_NEAR))
}

/// Cells of the other tomes behind an outside portal of tome `from`: those of
/// the walkable tiles on the far side of its plane whose boxes meet its quad.
/// An approximation of how the runtime's `TomeCollection` joins tomes.
fn across(vis: &Vis, from: usize, tile: &Tile, p: &Portal, cuts: Option<&Vec<Vec<bool>>>) -> Vec<(usize, u32, u32)> {
    let (plane, lo, hi) = portal_quad(&tile.tree_min, &tile.tree_max, p);
    let axis = (p.face() >> 1) as usize;
    let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
    let meets = |l: &Vec3, h: &Vec3| l[a] < hi[0] && lo[0] < h[a] && l[b] < hi[1] && lo[1] < h[b];
    let far_side = |l: &Vec3, h: &Vec3| if p.face() & 1 == 1 { (l[axis] - plane).abs() <= 1.0 } else { (h[axis] - plane).abs() <= 1.0 };
    let mut out = Vec::new();
    for (k, tome) in vis.tomes.iter().enumerate().filter(|(k, _)| *k != from) {
        if !far_side(&tome.tree_min, &tome.tree_max) {
            continue;
        }
        for (t, other) in tome.tiles.iter().enumerate() {
            let Some(other) = other else { continue };
            let walkable = cuts.map_or(other.is_leaf(), |cuts| cuts[k][t]);
            if !walkable || !far_side(&other.tree_min, &other.tree_max) || !meets(&other.tree_min, &other.tree_max) {
                continue;
            }
            for c in 0..other.num_cells() as u32 {
                let Some((cl, ch)) = cell_bounds(other, c) else { continue };
                if far_side(&cl, &ch) && meets(&cl, &ch) {
                    out.push((k, t as u32, c));
                }
            }
        }
    }
    out
}

fn walk(vis: &Vis, camera: &Camera, options: Options) -> Option<Walk> {
    let (k, tile, cell) = vis.locate(camera.eye)?;
    let cuts = options.lod.map(|factor| vis.tomes.iter().map(|tome| lod_cut(tome, &camera.eye, factor)).collect::<Vec<_>>());
    let mut reached: HashMap<usize, Vec<Rect>> = HashMap::new();
    let mut via = HashMap::new();
    let mut stack = vec![(k, tile, cell, SCREEN, usize::MAX)];
    let mut pushes = 0usize;
    while let Some((k, t, c, rect, from)) = stack.pop() {
        let tome = &vis.tomes[k];
        let cut = cuts.as_ref().map(|cuts| &cuts[k]);
        let vertices = tome.gate_vertices.as_deref().unwrap_or(&[]);
        let global = vis.global(k, t, c);
        via.entry(global).or_insert(from);
        let rects = reached.entry(global).or_default();
        if rects.iter().any(|r| contains(r, &rect)) {
            continue;
        }
        rects.retain(|r| !contains(&rect, r));
        rects.push(rect);
        let rect = if rects.len() > MAX_RECTS {
            let merged = rects.iter().skip(1).fold(rects[0], |a, r| union(&a, r));
            *rects = vec![merged];
            merged
        } else {
            rect
        };
        let tile = tome.tiles[t as usize].as_ref().unwrap();
        let node = &tile.cell_nodes.as_ref().unwrap()[c as usize];
        let portals = tile.portals.as_deref().unwrap_or(&[]);
        for p in &portals[node.portal_index as usize..(node.portal_index + node.portal_count) as usize] {
            if p.is_outside() {
                if let Some(Some(seen)) = portal_seen(camera, tile, p, options)
                    && let Some(next) = intersect(&seen, &rect)
                {
                    for (k2, t2, c2) in across(vis, k, tile, p, cuts.as_ref()) {
                        stack.push((k2, t2, c2, next, global));
                        pushes += 1;
                    }
                }
                continue;
            }
            if (p.is_hierarchy() && cut.is_none()) || (p.is_user() && !options.gates_open) {
                continue;
            }
            let enters = match &cut {
                Some(cut) => cut.get(p.target() as usize).copied().unwrap_or(false),
                None => tome.tiles.get(p.target() as usize).and_then(Option::as_ref).is_some_and(|t| t.is_leaf()),
            };
            if !enters {
                continue;
            }
            let seen = if p.is_user() {
                let from = p.gate_vertex_offset() as usize;
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for v in &vertices[from..from + p.gate_vertex_count() as usize] {
                    for a in 0..3 {
                        lo[a] = lo[a].min(v[a]);
                        hi[a] = hi[a].max(v[a]);
                    }
                }
                camera.box_rect(&lo, &hi)
            } else {
                let Some(seen) = portal_seen(camera, tile, p, options) else { continue };
                seen
            };
            if let Some(next) = seen.and_then(|s| intersect(&s, &rect)) {
                stack.push((k, p.target(), p.target_index as u32, next, global));
                pushes += 1;
            }
        }
    }
    if std::env::var_os("STATS").is_some() {
        let tiles: HashSet<(usize, usize)> = reached.keys().map(|&g| (vis.cell_of(g).0, vis.cell_of(g).1)).collect();
        println!("    walk: {} cells in {} tiles, {pushes} portal crossings", reached.len(), tiles.len());
    }
    Some(Walk { reached, via })
}

fn visible(vis: &Vis, camera: &Camera, walk: &Walk, options: Options) -> Vec<u32> {
    let mut out = Vec::new();
    for (&cell, rects) in &walk.reached {
        for &object in &vis.objects[cell] {
            if options.distance && vis.beyond_distance(object, &camera.eye) {
                continue;
            }
            let (lo, hi) = vis.object_box(object);
            if camera.box_rect(&lo, &hi).is_some_and(|r| rects.iter().any(|c| intersect(c, &r).is_some())) {
                out.push(object);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Whether `vis`'s walk exposes box `[lo, hi]`: some reached cell near it is
/// seen through a rect that the box's projection meets.
fn exposes(vis: &Vis, walk: &Walk, camera: &Camera, lo: &Vec3, hi: &Vec3) -> bool {
    let Some(r) = camera.box_rect(lo, hi) else { return false };
    walk.reached.iter().any(|(&global, rects)| {
        let (k, tile, cell) = vis.cell_of(global);
        let Some((clo, chi)) = cell_bounds(vis.tile(k, tile), cell) else { return false };
        (0..3).all(|a| clo[a] <= hi[a] + LIST_SLACK && lo[a] <= chi[a] + LIST_SLACK) && rects.iter().any(|c| intersect(c, &r).is_some())
    })
}

/// Objects `vis` hides that its scene shows in the frustum with a clear line of sight.
fn wrongly_culled(vis: &Vis, camera: &Camera, shown: &[u32], options: Options) -> Vec<u32> {
    let Some(truth) = &vis.truth else { return Vec::new() };
    let shown: HashSet<u32> = shown.iter().copied().collect();
    (0..vis.num_objects())
        .filter(|o| !shown.contains(o) && (!options.distance || !vis.beyond_distance(*o, &camera.eye)))
        .filter(|&o| {
            let (lo, hi) = vis.object_box(o);
            camera.box_rect(&lo, &hi).is_some() && truth.clear_probe(camera, vis.user_id(o)).is_some()
        })
        .collect()
}

struct Sample {
    counts: Vec<Option<usize>>,
    /// Per tome with a scene and `--count-truth`: objects with a clear line of sight.
    truly: Vec<Option<usize>>,
    /// Per tome: objects hidden despite a clear line of sight.
    wrong: Vec<Vec<u32>>,
    /// With two tomes: what each shows that the other hides.
    first_only: Vec<Only>,
    second_only: Vec<Only>,
}

/// An object one tome shows and the other hides: `(user ID, box, matched by ID)`.
type Only = (u32, Vec3, Vec3, bool);

/// Objects `a` shows that `b` hides: by user ID for reference FormIDs both
/// list (0xFD/0xFE model IDs number each tome's own precombine set), else
/// by whether `b`'s walk exposes the object's box.
fn only_in(a: &Vis, shown_a: &[u32], b: &Vis, shown_b: &[u32], walk_b: &Walk, camera: &Camera) -> Vec<Only> {
    let ids_b: HashSet<u32> = shown_b.iter().map(|&o| b.user_id(o)).collect();
    shown_a
        .iter()
        .filter_map(|&o| {
            let id = a.user_id(o);
            let (lo, hi) = a.object_box(o);
            let by_id = id >> 24 < 0xFD && b.by_id.contains_key(&id);
            let hidden = if by_id { !ids_b.contains(&id) } else { !exposes(b, walk_b, camera, &lo, &hi) };
            hidden.then_some((id, lo, hi, by_id))
        })
        .collect()
}

fn evaluate(vis: &[Vis], camera: &Camera, options: Options) -> Sample {
    let walks: Vec<Option<Walk>> = vis.iter().map(|v| walk(v, camera, options)).collect();
    let sets: Vec<Option<Vec<u32>>> = vis.iter().zip(&walks).map(|(v, w)| w.as_ref().map(|w| visible(v, camera, w, options))).collect();
    let wrong = vis.iter().zip(&sets).map(|(v, s)| s.as_ref().map_or_else(Vec::new, |s| wrongly_culled(v, camera, s, options))).collect();
    let truly = vis
        .iter()
        .map(|v| {
            let truth = v.truth.as_ref().filter(|_| options.count_truth)?;
            Some(
                (0..v.num_objects())
                    .filter(|&o| {
                        let (lo, hi) = v.object_box(o);
                        !(options.distance && v.beyond_distance(o, &camera.eye))
                            && camera.box_rect(&lo, &hi).is_some()
                            && truth.clear_probe(camera, v.user_id(o)).is_some()
                    })
                    .count(),
            )
        })
        .collect();
    let (first_only, second_only) = match (vis, &sets[..], &walks[..]) {
        ([a, b], [Some(sa), Some(sb)], [Some(wa), Some(wb)]) => (only_in(a, sa, b, sb, wb, camera), only_in(b, sb, a, sa, wa, camera)),
        _ => (Vec::new(), Vec::new()),
    };
    Sample { counts: sets.iter().map(|s| s.as_ref().map(Vec::len)).collect(), truly, wrong, first_only, second_only }
}

/// Prints the cell chain through which `vis`'s walk reaches object `id`.
fn trace(vis: &Vis, camera: &Camera, options: Options, id: u32) {
    let Some(w) = walk(vis, camera, options) else { return };
    let Some(&object) = vis.by_id.get(&id) else { return };
    let (lo, hi) = vis.object_box(object);
    let Some(r) = camera.box_rect(&lo, &hi) else { return };
    let Some((&start, _)) = w.reached.iter().find(|(g, rects)| vis.objects[**g].contains(&object) && rects.iter().any(|c| intersect(c, &r).is_some())) else {
        return;
    };
    let mut at = start;
    while at != usize::MAX {
        let (k, tile, cell) = vis.cell_of(at);
        println!("      via tome {k} tile {tile} cell {cell} {:?} rects {:?}", cell_bounds(vis.tile(k, tile), cell), w.reached[&at]);
        at = w.via[&at];
    }
}

/// Walks the first clear line of sight to object `id` and prints each cell it
/// crosses, whether the walk reached it and whether its rects cover the ray.
fn follow_ray(vis: &Vis, camera: &Camera, options: Options, id: u32) {
    let Some(w) = walk(vis, camera, options) else { return };
    // Without a scene, aim at the centre of the object's box.
    let target = match &vis.truth {
        Some(truth) => truth.clear_probe(camera, id),
        None => vis.by_id.get(&id).map(|&o| {
            let (lo, hi) = vis.object_box(o);
            std::array::from_fn(|a| (lo[a] + hi[a]) * 0.5)
        }),
    };
    let Some(target) = &target else {
        return println!("      no clear probe");
    };
    let v = camera.view(target);
    let (x, y) = camera.project(&v);
    println!("      clear probe {target:?} at screen ({x:.3}, {y:.3})");
    let dir = sub(target, &camera.eye);
    let steps = (dot(&dir, &dir).sqrt() / 8.0).ceil() as usize;
    let mut last = None;
    for k in 0..=steps {
        let t = k as f32 / steps as f32;
        let p = [camera.eye[0] + dir[0] * t, camera.eye[1] + dir[1] * t, camera.eye[2] + dir[2] * t];
        let hit = vis.locate(p);
        if hit == last {
            continue;
        }
        last = hit;
        match hit {
            Some((k, tile, cell)) => {
                let global = vis.global(k, tile, cell);
                let rects = w.reached.get(&global);
                let covered = rects.is_some_and(|r| r.iter().any(|r| r[0] <= x && x <= r[1] && r[2] <= y && y <= r[3]));
                let listed = vis.by_id.get(&id).is_some_and(|o| vis.objects[global].contains(o));
                println!(
                    "      t={t:.3} {p:?} tome {k} tile {tile} cell {cell} {:?} reached={} covers ray={covered} lists={listed}",
                    cell_bounds(vis.tile(k, tile as usize), cell),
                    rects.is_some()
                );
            }
            None => println!("      t={t:.3} {p:?} no cell"),
        }
    }
}

/// How far `p` lies outside the box of the cell the tome places it in.
fn outside_own_cell(vis: &Vis, p: &Vec3) -> Option<f32> {
    let (k, tile, cell) = vis.locate(*p)?;
    let (lo, hi) = cell_bounds(vis.tile(k, tile as usize), cell)?;
    Some((0..3).map(|a| (lo[a] - p[a]).max(p[a] - hi[a]).max(0.0)).fold(0.0, f32::max))
}

fn percentiles(values: &mut [usize]) -> String {
    if values.is_empty() {
        return "none".into();
    }
    values.sort_unstable();
    let at = |p: f64| values[((values.len() - 1) as f64 * p).round() as usize];
    let mean = values.iter().sum::<usize>() as f64 / values.len() as f64;
    format!("p10 {} p50 {} p90 {} max {} mean {mean:.0} (n={})", at(0.1), at(0.5), at(0.9), at(1.0), values.len())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut tomes = Vec::new();
    let mut explicit = Vec::new();
    let (mut eye, mut stride, mut yaws) = (0.0f32, 1usize, 8usize);
    let mut pitches = vec![0.0f32];
    let mut directions: Vec<(f32, f32)> = Vec::new();
    let mut options = Options { gates_open: false, distance: false, expand: true, count_truth: false, lod: None };
    let (mut fov_y, mut aspect) = (55.4f32, 16.0f32 / 9.0);
    let mut seeds_path = None;
    while let Some(arg) = args.next() {
        let mut value = || args.next().unwrap();
        match arg.as_str() {
            "--seeds" => seeds_path = Some(value()),
            "--at" => explicit.push([0; 3].map(|_: i32| value().parse::<f32>().unwrap())),
            "--eye" => eye = value().parse().unwrap(),
            "--stride" => stride = value().parse().unwrap(),
            "--yaws" => yaws = value().parse().unwrap(),
            "--pitches" => pitches = value().split(',').map(|s| s.parse().unwrap()).collect(),
            "--heading" => {
                let heading: f32 = value().parse().unwrap();
                let tilt: f32 = value().parse().unwrap();
                directions.push((90.0 - heading, -tilt));
            }
            "--gates" => options.gates_open = value() == "open",
            "--distance" => options.distance = true,
            "--no-expand" => options.expand = false,
            "--count-truth" => options.count_truth = true,
            "--lod" => options.lod = Some(value().parse().unwrap()),
            "--fov-y" => fov_y = value().parse().unwrap(),
            "--aspect" => aspect = value().parse().unwrap(),
            _ => tomes.push(arg),
        }
    }
    let points: Vec<Vec3> = seeds_path.map_or_else(Vec::new, |path| {
        let scene = Scene::decode(&std::fs::read(path).unwrap()).unwrap();
        scene.volumes.iter().filter(|v| (0..3).all(|a| v[a] == v[a + 3])).step_by(stride).map(|v| [v[0], v[1], v[2] + eye]).collect()
    });
    let pool = rayon::ThreadPoolBuilder::new().num_threads(previs_native::default_workers()).build().unwrap();
    let vis: Vec<Vis> = pool.install(|| tomes.par_iter().map(|t| Vis::load(t, options.gates_open)).collect());
    let fov_y = fov_y.to_radians();
    let cameras = |p: Vec3| -> Vec<Camera> {
        if !directions.is_empty() {
            return directions.iter().map(|&(yaw, pitch)| Camera::new(p, yaw.to_radians(), pitch.to_radians(), fov_y, aspect)).collect();
        }
        pitches
            .iter()
            .flat_map(|&pitch| (0..yaws).map(move |k| (k as f32 * std::f32::consts::TAU / yaws as f32, pitch.to_radians())))
            .map(|(yaw, pitch)| Camera::new(p, yaw, pitch, fov_y, aspect))
            .collect()
    };
    let traced = std::env::var("TRACE").ok().and_then(|t| u32::from_str_radix(&t, 16).ok());
    // DISTANCES=<hex prefix> prints the distance records of matching user IDs in the first tome.
    if let Ok(prefix) = std::env::var("DISTANCES") {
        for (o, d) in vis[0].tomes[0].object_distances.iter().flatten().enumerate() {
            let id = format!("{:08X}", vis[0].user_id(o as u32));
            if id.starts_with(&prefix) {
                let d = d.map(f32::from_bits);
                println!("{id} box {:?}..{:?} max distance {}", &d[0..3], &d[4..7], d[7].sqrt());
            }
        }
    }
    // PORTALS=tile:cell prints that cell's portals in the first tome.
    if let Some((t, c)) = std::env::var("PORTALS").ok().and_then(|s| s.split_once(':').map(|(t, c)| (t.parse::<usize>().unwrap(), c.parse::<usize>().unwrap()))) {
        let tile = vis[0].tile(0, t);
        let node = &tile.cell_nodes.as_ref().unwrap()[c];
        println!("tile {t} cell {c} box {:?}", cell_bounds(tile, c as u32));
        for p in &tile.portals.as_deref().unwrap_or(&[])[node.portal_index as usize..(node.portal_index + node.portal_count) as usize] {
            let quad = (!p.is_user()).then(|| portal_quad(&tile.tree_min, &tile.tree_max, p));
            println!(
                "  face {} -> tile {} cell {} outside={} hierarchy={} user={} quad {quad:?}",
                p.face(),
                p.target(),
                p.target_index,
                p.is_outside(),
                p.is_hierarchy(),
                p.is_user()
            );
        }
    }

    for p in &explicit {
        for camera in cameras(*p) {
            let s = pool.install(|| evaluate(&vis, &camera, options));
            let f = camera.forward;
            let wrong: Vec<usize> = s.wrong.iter().map(Vec::len).collect();
            println!(
                "{p:?} dir [{:.2},{:.2},{:.2}]: visible {:?} wrongly culled {wrong:?} only in first {} / second {}",
                f[0], f[1], f[2], s.counts, s.first_only.len(), s.second_only.len()
            );
            if std::env::var_os("LIST").is_some()
                && let Some(walked) = walk(&vis[0], &camera, options)
            {
                let mut ids: Vec<String> =
                    visible(&vis[0], &camera, &walked, options).iter().map(|&o| format!("{:08X}", vis[0].user_id(o))).collect();
                ids.sort_unstable();
                println!("    visible: {}", ids.join(" "));
            }
            if std::env::var_os("SHOW").is_some() {
                for (label, only) in [("first", &s.first_only), ("second", &s.second_only)] {
                    for (id, lo, hi, _) in only {
                        println!("    only in {label} {id:08X} {lo:?}..{hi:?}");
                    }
                }
                for (k, wrong) in s.wrong.iter().enumerate() {
                    for &o in wrong {
                        let (lo, hi) = vis[k].object_box(o);
                        println!("    tome {k} wrongly culls {:08X} {lo:?}..{hi:?}", vis[k].user_id(o));
                    }
                }
            }
            if let Some(id) = traced {
                let k = std::env::var("TRACE_TOME").ok().and_then(|t| t.parse().ok()).unwrap_or(0usize);
                trace(&vis[k], &camera, options, id);
                follow_ray(&vis[k], &camera, options, id);
            }
        }
    }
    if points.is_empty() {
        return;
    }
    let started = std::time::Instant::now();
    let evaluated: Vec<(Camera, Sample)> =
        pool.install(|| points.par_iter().flat_map_iter(|p| cameras(*p)).map(|c| (c, evaluate(&vis, &c, options))).collect());
    println!("{} positions x {} directions in {:.1?}", points.len(), evaluated.len() / points.len(), started.elapsed());
    for (k, path) in tomes.iter().enumerate() {
        let mut counts: Vec<usize> = evaluated.iter().filter_map(|(_, s)| s.counts[k]).collect();
        let unplaced = evaluated.iter().filter(|(_, s)| s.counts[k].is_none()).count();
        println!("{path}\n  visible {} ; {unplaced} samples outside any cell", percentiles(&mut counts));
        let outside: Vec<f32> = points.iter().filter_map(|p| outside_own_cell(&vis[k], p)).collect();
        let beyond = |d: f32| outside.iter().filter(|&&o| o > d).count();
        println!(
            "  camera outside its cell's box: {} of {} positions, {} by over 8, {} by over 16",
            beyond(0.0),
            points.len(),
            beyond(8.0),
            beyond(16.0)
        );
        let mut truly: Vec<usize> = evaluated.iter().filter_map(|(_, s)| s.truly[k]).collect();
        if !truly.is_empty() {
            println!("  clear line of sight {}", percentiles(&mut truly));
        }
        if vis[k].truth.is_some() {
            let wrong: usize = evaluated.iter().map(|(_, s)| s.wrong[k].len()).sum();
            let samples = evaluated.iter().filter(|(_, s)| !s.wrong[k].is_empty()).count();
            println!("  wrongly culled: {wrong} objects over {samples} of {} samples", evaluated.len());
            let mut worst: Vec<&(Camera, Sample)> = evaluated.iter().filter(|(_, s)| !s.wrong[k].is_empty()).collect();
            worst.sort_by_key(|(_, s)| std::cmp::Reverse(s.wrong[k].len()));
            for (camera, s) in worst.iter().take(std::env::var("WORST").ok().and_then(|n| n.parse().ok()).unwrap_or(4)) {
                let (e, f) = (camera.eye, camera.forward);
                let ids: Vec<String> = s.wrong[k].iter().take(6).map(|&o| format!("{:08X}", vis[k].user_id(o))).collect();
                println!("    --at {} {} {} dir [{:.2},{:.2},{:.2}]: {} e.g. {ids:?}", e[0], e[1], e[2], f[0], f[1], f[2], s.wrong[k].len());
            }
        }
    }
    if vis.len() == 2 {
        let both: Vec<&Sample> = evaluated.iter().map(|(_, s)| s).filter(|s| s.counts[0].is_some() && s.counts[1].is_some()).collect();
        let mut ratios: Vec<usize> = both.iter().map(|s| (100 * s.counts[1].unwrap()).div_ceil(s.counts[0].unwrap().max(1))).collect();
        println!("second/first visible, percent: {}", percentiles(&mut ratios));
        if vis.iter().all(|v| v.truth.is_some()) {
            for (k, other) in [(0, 1), (1, 0)] {
                let alone: Vec<usize> =
                    both.iter().filter(|s| s.wrong[other].is_empty()).map(|s| s.wrong[k].len()).filter(|&n| n > 0).collect();
                println!("  only tome {k} culls wrongly: {} objects over {} samples", alone.iter().sum::<usize>(), alone.len());
            }
        }
        let mut gaps: Vec<&(Camera, Sample)> = evaluated.iter().filter(|(_, s)| s.counts[0].is_some() && s.counts[1].is_some()).collect();
        gaps.sort_by_key(|(_, s)| std::cmp::Reverse(s.counts[1].unwrap() as i64 - s.counts[0].unwrap() as i64));
        for (camera, s) in gaps.iter().take(6) {
            let (e, f) = (camera.eye, camera.forward);
            println!("  widest gap: --at {} {} {} dir [{:.2},{:.2},{:.2}] visible {:?}", e[0], e[1], e[2], f[0], f[1], f[2], s.counts);
        }
        for (label, second) in [("first", false), ("second", true)] {
            let lists: Vec<&Vec<Only>> = both.iter().map(|s| if second { &s.second_only } else { &s.first_only }).collect();
            let by_id: usize = lists.iter().map(|l| l.iter().filter(|o| o.3).count()).sum();
            let by_box: usize = lists.iter().map(|l| l.iter().filter(|o| !o.3).count()).sum();
            let samples = lists.iter().filter(|l| !l.is_empty()).count();
            println!("only the {label} shows: {by_id} by id, {by_box} by box, in {samples} of {} samples", both.len());
        }
    }
}
