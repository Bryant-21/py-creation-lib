//! Offline audit of FO4 object-LOD tiles (.bto): structure, segments and duplicate geometry.
use std::collections::{HashMap, HashSet};
use std::path::Path;

use nif_core_native::fo4_block_types::is_fo4_block_type;
use nif_core_native::model::{NifBlock, NifFile, NifValue};

use crate::objects::fo76_bto::{extract_inline_geometry, val_array, val_f32, val_string, val_u64, vec3};
use crate::objects::geometry::LodGeometry;

pub const CELL: f32 = 4096.0;

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct TileAudit {
    pub path: String,
    pub level: u32,
    pub x: i32,
    pub y: i32,
    pub bytes: u64,
    pub root_children: usize,
    /// Root children whose children are not exactly one geometry: the engine prepares and fades
    /// only child [0] of each root child (LOD_RUNTIME.md Q6, Q11).
    pub root_children_not_single_geometry: usize,
    pub shapes: usize,
    pub triangles: u64,
    pub non_fo4_blocks: Vec<String>,
    pub instance_blocks: usize,
    pub segment_slots: Vec<usize>,
    pub out_of_cell_triangles: u64,
    pub duplicate_triangles: u64,
    pub toggle_ref_ids: usize,
    pub empty: bool,
    /// Shapes with a vertex outside their BSMultiBoundAABB or their own bounding sphere: the
    /// renderer culls by these, so a short bound drops visible geometry as the camera turns.
    pub shapes_outside_multibound: usize,
    pub shapes_outside_sphere: usize,
    /// Furthest any vertex lies outside either bound, in game units.
    pub max_bound_overflow: f32,
    pub culling_modes: Vec<String>,
    /// Triangles overlapping another same-facing, parallel triangle a small gap away, by the
    /// smallest such gap: [0.01, 0.5), [0.5, 2), [2, 8), [8, 32] units. Depth precision falls
    /// with distance, so layers that resolve up close z-fight at LOD range.
    pub layered_triangles: [u64; 4],
    /// Triangles overlapping a different same-facing triangle at the same height (< 0.01 units apart).
    /// Exact duplicates and back faces are excluded. Half-float vertex positions snap nearby heights
    /// together, so surfaces stacked a few units apart can end up here.
    pub coplanar_overlaps: u64,
    /// Mean triangle height in the tile, for relating counts to elevation.
    pub mean_z: f32,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct AuditSummary {
    pub tiles: usize,
    pub with_non_fo4_blocks: usize,
    pub with_out_of_cell: usize,
    pub with_duplicates: usize,
    pub with_multi_geometry_root_children: usize,
    pub with_shapes_outside_multibound: usize,
    pub with_shapes_outside_sphere: usize,
    pub empty: usize,
    pub max_root_children: usize,
    pub total_bytes: u64,
}

pub fn parse_tile_name(name: &str) -> Option<(String, u32, i32, i32)> {
    let stem = name.strip_suffix(".bto").or_else(|| name.strip_suffix(".BTO"))?;
    let mut parts = stem.rsplitn(4, '.');
    let y = parts.next()?.parse().ok()?;
    let x = parts.next()?.parse().ok()?;
    let level = parts.next()?.parse().ok()?;
    let world = parts.next()?.to_string();
    Some((world, level, x, y))
}

pub fn audit_tile(path: &Path, bytes: &[u8]) -> anyhow::Result<TileAudit> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let (_, level, x, y) =
        parse_tile_name(name).ok_or_else(|| anyhow::anyhow!("not a tile name: {name}"))?;
    let nif = NifFile::from_bytes(bytes, Some(path.to_path_buf()))
        .map_err(|e| anyhow::anyhow!("read {}: {e:?}", path.display()))?;
    let mut a = TileAudit {
        path: path.display().to_string(),
        level,
        x,
        y,
        bytes: bytes.len() as u64,
        ..Default::default()
    };
    a.non_fo4_blocks = non_fo4_types(&nif);
    a.instance_blocks = nif
        .blocks
        .iter()
        .filter(|b| b.type_name == "BSDistantObjectInstanceDataBlock")
        .count();
    a.toggle_ref_ids = nif
        .blocks
        .iter()
        .filter(|b| {
            b.type_name == "NiIntegerExtraData"
                && val_string(b.get_field("Name")).as_deref() == Some("ToggleRefID")
        })
        .count();
    let roots = root_child_ids(&nif);
    a.root_children = roots.len();
    a.root_children_not_single_geometry = roots
        .iter()
        .filter(|&&id| {
            let children = child_ids(&nif.blocks[id]);
            children.len() != 1 || !is_geometry(&nif.blocks[children[0]])
        })
        .count();
    a.culling_modes = nif
        .blocks
        .iter()
        .filter(|b| b.type_name == "BSMultiBoundNode")
        .filter_map(|b| culling_mode_name(b.get_field("Culling Mode")?))
        .collect();
    a.culling_modes.sort();
    a.culling_modes.dedup();
    let parents = parent_map(&nif);
    let mut seen: HashSet<[i32; 9]> = HashSet::new();
    let mut world_triangles: Vec<[[f32; 3]; 3]> = Vec::new();
    for shape in nif.blocks.iter().filter(|b| is_geometry(b)) {
        a.shapes += 1;
        a.segment_slots.push(val_array(shape.get_field("Segment")).len());
        let Some(geometry) = extract_inline_geometry(shape) else {
            continue;
        };
        let chain = world_chain(&nif, &parents, shape);
        let (multibound, sphere) = bound_overflow(&nif, &parents, shape, &geometry, &chain);
        a.shapes_outside_multibound += usize::from(multibound > 0.0);
        a.shapes_outside_sphere += usize::from(sphere > 0.0);
        a.max_bound_overflow = a.max_bound_overflow.max(multibound).max(sphere);
        for (tri, segment) in shape_triangles_world(shape, &geometry, &chain) {
            a.triangles += 1;
            world_triangles.push(tri);
            if !seen.insert(quantize(&tri)) {
                a.duplicate_triangles += 1;
            }
            if level == 4 {
                if let Some(seg) = segment {
                    if !triangle_in_segment_cell(&tri, seg, level, x, y) {
                        a.out_of_cell_triangles += 1;
                    }
                }
            }
        }
    }
    a.empty = a.triangles == 0 && a.instance_blocks == 0;
    (a.layered_triangles, a.coplanar_overlaps) = layered_triangles(&world_triangles);
    if !world_triangles.is_empty() {
        let sum: f64 = world_triangles.iter().map(|t| f64::from(t[0][2] + t[1][2] + t[2][2]) / 3.0).sum();
        a.mean_z = (sum / world_triangles.len() as f64) as f32;
    }
    Ok(a)
}

/// Object triangles facing up and lying on the terrain tile, by the furthest of their vertices
/// from the terrain surface: [0, 0.5), [0.5, 2), [2, 8), [8, 32] units. Terrain water shapes
/// (effect shader) are not terrain. Terrain vertices are tile-local in x/y, so `origin` is the
/// tile's south-west corner in world units; object tiles are already in world space.
pub fn terrain_contact(object_bytes: &[u8], terrain_bytes: &[u8], origin: [f32; 2]) -> anyhow::Result<[u64; 4]> {
    const GAPS: [f32; 5] = [0.0, 0.5, 2.0, 8.0, 32.0];
    let objects = world_triangles(&read_nif(object_bytes)?, false);
    let surface = TerrainSurface::new(terrain_bytes, origin)?;
    let mut counts = [0u64; 4];
    for tri in &objects {
        let Some(plane) = plane_of(tri) else { continue };
        if plane.normal[2] * plane.facing < 0.7 {
            continue;
        }
        let gap = tri.iter().try_fold(0.0f32, |worst, p| Some(worst.max((p[2] - surface.height(p[0], p[1])?).abs())));
        if let Some(gap) = gap {
            if let Some(bin) = (0..4).find(|&b| gap >= GAPS[b] && (gap < GAPS[b + 1] || b == 3 && gap == GAPS[4])) {
                counts[bin] += 1;
            }
        }
    }
    Ok(counts)
}

/// Terrain LOD height at world (x, y), or None outside the tile. `origin` as in `terrain_contact`.
pub fn terrain_height_at(terrain_bytes: &[u8], origin: [f32; 2], x: f32, y: f32) -> anyhow::Result<Option<f32>> {
    Ok(TerrainSurface::new(terrain_bytes, origin)?.height(x, y))
}

fn read_nif(bytes: &[u8]) -> anyhow::Result<NifFile> {
    NifFile::from_bytes(bytes, None).map_err(|e| anyhow::anyhow!("read: {e:?}"))
}

struct TerrainSurface {
    triangles: Vec<[[f32; 3]; 3]>,
    grid: HashMap<(i32, i32), Vec<usize>>,
}

impl TerrainSurface {
    const CELL_STEP: f32 = 512.0;

    fn new(terrain_bytes: &[u8], origin: [f32; 2]) -> anyhow::Result<Self> {
        let triangles: Vec<[[f32; 3]; 3]> = world_triangles(&read_nif(terrain_bytes)?, true)
            .into_iter()
            .map(|t| t.map(|p| [p[0] + origin[0], p[1] + origin[1], p[2]]))
            .collect();
        let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, t) in triangles.iter().enumerate() {
            let lo = |k: usize| (t[0][k].min(t[1][k]).min(t[2][k]) / Self::CELL_STEP).floor() as i32;
            let hi = |k: usize| (t[0][k].max(t[1][k]).max(t[2][k]) / Self::CELL_STEP).floor() as i32;
            for gx in lo(0)..=hi(0) {
                for gy in lo(1)..=hi(1) {
                    grid.entry((gx, gy)).or_default().push(i);
                }
            }
        }
        Ok(Self { triangles, grid })
    }

    fn height(&self, x: f32, y: f32) -> Option<f32> {
        let cell = ((x / Self::CELL_STEP).floor() as i32, (y / Self::CELL_STEP).floor() as i32);
        self.grid.get(&cell)?.iter().find_map(|&i| {
            let [a, b, c] = self.triangles[i];
            let det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
            if det.abs() < 1e-6 {
                return None;
            }
            let l1 = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / det;
            let l2 = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / det;
            let l3 = 1.0 - l1 - l2;
            (l1 >= -1e-4 && l2 >= -1e-4 && l3 >= -1e-4).then(|| l1 * a[2] + l2 * b[2] + l3 * c[2])
        })
    }
}

fn world_triangles(nif: &NifFile, lighting_only: bool) -> Vec<[[f32; 3]; 3]> {
    let parents = parent_map(nif);
    let mut out = Vec::new();
    for shape in nif.blocks.iter().filter(|b| is_geometry(b)) {
        if lighting_only {
            let shader = shape.get_field("Shader Property").map(NifValue::as_i64).unwrap_or(-1);
            let is_lighting = usize::try_from(shader)
                .ok()
                .and_then(|id| nif.blocks.get(id))
                .is_some_and(|b| b.type_name == "BSLightingShaderProperty");
            if !is_lighting {
                continue;
            }
        }
        let Some(geometry) = extract_inline_geometry(shape) else { continue };
        let chain = world_chain(nif, &parents, shape);
        out.extend(shape_triangles_world(shape, &geometry, &chain).into_iter().map(|(t, _)| t));
    }
    out
}

const LAYER_GAPS: [f32; 5] = [0.01, 0.5, 2.0, 8.0, 32.0];

struct Plane {
    /// Unit normal flipped so its largest component is positive; `facing` keeps the original sign.
    normal: [f32; 3],
    offset: f32,
    facing: f32,
    /// The two axes other than the normal's dominant one: the projection used for overlap.
    uv: [[f32; 2]; 3],
}

fn plane_of(tri: &[[f32; 3]; 3]) -> Option<Plane> {
    let e1 = [tri[1][0] - tri[0][0], tri[1][1] - tri[0][1], tri[1][2] - tri[0][2]];
    let e2 = [tri[2][0] - tri[0][0], tri[2][1] - tri[0][1], tri[2][2] - tri[0][2]];
    let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    // Twice the area; slivers under ~1 unit² carry no visible surface.
    if len < 2.0 {
        return None;
    }
    let mut normal = [n[0] / len, n[1] / len, n[2] / len];
    let axis = (0..3).max_by(|&i, &j| normal[i].abs().total_cmp(&normal[j].abs())).unwrap_or(2);
    let facing = normal[axis].signum();
    normal = normal.map(|c| c * facing);
    let offset = normal[0] * tri[0][0] + normal[1] * tri[0][1] + normal[2] * tri[0][2];
    let (u, v) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    Some(Plane { normal, offset, facing, uv: tri.map(|p| [p[u], p[v]]) })
}

fn point_in_triangle(p: [f32; 2], t: &[[f32; 2]; 3]) -> bool {
    let side = |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
    let (d0, d1, d2) = (side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0]));
    !((d0 < 0.0 || d1 < 0.0 || d2 < 0.0) && (d0 > 0.0 || d1 > 0.0 || d2 > 0.0))
}

fn centroid2(t: &[[f32; 2]; 3]) -> [f32; 2] {
    [(t[0][0] + t[1][0] + t[2][0]) / 3.0, (t[0][1] + t[1][1] + t[2][1]) / 3.0]
}

/// Buckets triangles by normal (~1°), plane offset and projected position, then compares each with
/// its neighbouring buckets. A pair straddling a normal bucket edge is missed; both tile sets are
/// measured the same way, so the comparison holds.
fn layered_triangles(triangles: &[[[f32; 3]; 3]]) -> ([u64; 4], u64) {
    const NORMAL_STEP: f32 = 0.02;
    const CELL_STEP: f32 = 256.0;
    let planes: Vec<Option<Plane>> = triangles.iter().map(plane_of).collect();
    let keys: Vec<[i32; 9]> = triangles.iter().map(quantize).collect();
    let key = |p: &Plane| {
        let c = centroid2(&p.uv);
        (
            p.normal.map(|x| (x / NORMAL_STEP).round() as i32),
            (p.offset / LAYER_GAPS[4]).floor() as i32,
            (c[0] / CELL_STEP).floor() as i32,
            (c[1] / CELL_STEP).floor() as i32,
        )
    };
    let mut buckets: HashMap<([i32; 3], i32, i32, i32), Vec<usize>> = HashMap::new();
    for (i, plane) in planes.iter().enumerate() {
        if let Some(p) = plane {
            buckets.entry(key(p)).or_default().push(i);
        }
    }
    let mut counts = [0u64; 4];
    let mut coplanar = 0u64;
    for (i, plane) in planes.iter().enumerate() {
        let Some(p) = plane else { continue };
        let (n, d, cu, cv) = key(p);
        let mut best = f32::INFINITY;
        let mut on_same_plane = false;
        for dd in -1..=1 {
            for du in -1..=1 {
                for dv in -1..=1 {
                    let Some(others) = buckets.get(&(n, d + dd, cu + du, cv + dv)) else { continue };
                    for &j in others {
                        if j == i {
                            continue;
                        }
                        let Some(q) = &planes[j] else { continue };
                        let gap = (q.offset - p.offset).abs();
                        if p.facing != q.facing || gap > LAYER_GAPS[4] {
                            continue;
                        }
                        let overlaps = || {
                            point_in_triangle(centroid2(&p.uv), &q.uv) || point_in_triangle(centroid2(&q.uv), &p.uv)
                        };
                        if gap < LAYER_GAPS[0] {
                            on_same_plane = on_same_plane || (keys[i] != keys[j] && overlaps());
                            continue;
                        }
                        if gap < best && overlaps() {
                            best = gap;
                        }
                    }
                }
            }
        }
        if let Some(bin) = (0..4).find(|&b| best >= LAYER_GAPS[b] && best < LAYER_GAPS[b + 1] || b == 3 && best == LAYER_GAPS[4]) {
            counts[bin] += 1;
        }
        coplanar += u64::from(on_same_plane);
    }
    (counts, coplanar)
}

pub fn summarize(audits: &[TileAudit]) -> AuditSummary {
    AuditSummary {
        tiles: audits.len(),
        with_non_fo4_blocks: audits.iter().filter(|a| !a.non_fo4_blocks.is_empty()).count(),
        with_out_of_cell: audits.iter().filter(|a| a.out_of_cell_triangles > 0).count(),
        with_duplicates: audits.iter().filter(|a| a.duplicate_triangles > 0).count(),
        with_multi_geometry_root_children: audits
            .iter()
            .filter(|a| a.root_children_not_single_geometry > 0)
            .count(),
        with_shapes_outside_multibound: audits.iter().filter(|a| a.shapes_outside_multibound > 0).count(),
        with_shapes_outside_sphere: audits.iter().filter(|a| a.shapes_outside_sphere > 0).count(),
        empty: audits.iter().filter(|a| a.empty).count(),
        max_root_children: audits.iter().map(|a| a.root_children).max().unwrap_or(0),
        total_bytes: audits.iter().map(|a| a.bytes).sum(),
    }
}

/// L4 segment index → cell, inverse of object_lod::generate_segments and of the engine's
/// BuildCellCache (LOD_RUNTIME.md Q1): seg = level*dx + dy.
pub fn segment_cell(seg: u32, level: u32, tile_x: i32, tile_y: i32) -> (i32, i32) {
    (tile_x + (seg / level) as i32, tile_y + (seg % level) as i32)
}

fn non_fo4_types(nif: &NifFile) -> Vec<String> {
    let mut names: Vec<String> = nif
        .blocks
        .iter()
        .filter(|b| !is_fo4_block_type(&b.type_name))
        .map(|b| b.type_name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn is_geometry(block: &NifBlock) -> bool {
    block.type_name == "BSSubIndexTriShape" || block.type_name == "BSTriShape"
}

fn child_ids(block: &NifBlock) -> Vec<usize> {
    val_array(block.get_field("Children"))
        .iter()
        .map(NifValue::as_i64)
        .filter(|&id| id >= 0)
        .map(|id| id as usize)
        .collect()
}

fn root_child_ids(nif: &NifFile) -> Vec<usize> {
    match nif.header.footer_roots.first() {
        Some(&root) if root >= 0 && (root as usize) < nif.blocks.len() => child_ids(&nif.blocks[root as usize])
            .into_iter()
            .filter(|&id| id < nif.blocks.len())
            .collect(),
        _ => Vec::new(),
    }
}

fn parent_map(nif: &NifFile) -> HashMap<usize, usize> {
    let mut parents = HashMap::new();
    for (id, block) in nif.blocks.iter().enumerate() {
        for child in child_ids(block) {
            parents.entry(child).or_insert(id);
        }
    }
    parents
}

struct Transform {
    translation: [f32; 3],
    rotation: [[f32; 3]; 3],
    scale: f32,
}

impl Transform {
    fn of(block: &NifBlock) -> Self {
        let rotation = match block.get_field("Rotation") {
            Some(NifValue::Matrix33(m)) => *m,
            _ => [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        Transform {
            translation: vec3(block.get_field("Translation")).unwrap_or([0.0; 3]),
            rotation,
            scale: val_f32(block.get_field("Scale")).unwrap_or(1.0),
        }
    }

    fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let r = &self.rotation;
        let mut out = [0.0; 3];
        for (i, row) in r.iter().enumerate() {
            out[i] = self.translation[i] + self.scale * (row[0] * p[0] + row[1] * p[1] + row[2] * p[2]);
        }
        out
    }
}

/// nif.xml `BSCPCullingType`; a parsed file holds the raw number, a built one the name.
fn culling_mode_name(value: &NifValue) -> Option<String> {
    const NAMES: [&str; 5] =
        ["CULL_NORMAL", "CULL_ALLPASS", "CULL_ALLFAIL", "CULL_IGNOREMULTIBOUNDS", "CULL_FORCEMULTIBOUNDSNOUPDATE"];
    match value {
        NifValue::String(name) => Some(name.clone()),
        other => {
            let n = other.as_i64();
            Some(usize::try_from(n).ok().and_then(|i| NAMES.get(i)).map_or_else(|| n.to_string(), |s| s.to_string()))
        }
    }
}

/// The shape's own transform, then each parent node's up to the root.
fn world_chain(nif: &NifFile, parents: &HashMap<usize, usize>, shape: &NifBlock) -> Vec<Transform> {
    let mut chain = vec![Transform::of(shape)];
    let mut id = shape.block_id;
    while let Some(&parent) = parents.get(&id) {
        chain.push(Transform::of(&nif.blocks[parent]));
        id = parent;
    }
    chain
}

fn to_world(chain: &[Transform], p: [f32; 3]) -> [f32; 3] {
    chain.iter().fold(p, |acc, t| t.apply(acc))
}

/// How far the shape's vertices reach outside the nearest ancestor BSMultiBoundAABB (world space)
/// and outside the shape's own bounding sphere (model space), each 0.0 when inside. A float
/// writer's rounding is not a defect, so each bound gets 1% slack.
fn bound_overflow(
    nif: &NifFile,
    parents: &HashMap<usize, usize>,
    shape: &NifBlock,
    geometry: &LodGeometry,
    chain: &[Transform],
) -> (f32, f32) {
    let mut multibound = 0.0f32;
    if let Some((center, extent)) = ancestor_multibound(nif, parents, shape.block_id) {
        for &p in &geometry.vertices {
            let w = to_world(chain, p);
            for i in 0..3 {
                let reach = extent[i].abs() * 1.01 + 1.0;
                multibound = multibound.max((w[i] - center[i]).abs() - reach);
            }
        }
    }
    let mut sphere = 0.0f32;
    if let Some(NifValue::Struct(bound)) = shape.get_field("Bounding Sphere") {
        if let (Some(center), Some(radius)) = (vec3(bound.get("Center")), val_f32(bound.get("Radius"))) {
            let reach = radius * 1.01 + 1.0;
            for p in &geometry.vertices {
                let d = ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2) + (p[2] - center[2]).powi(2)).sqrt();
                sphere = sphere.max(d - reach);
            }
        }
    }
    (multibound.max(0.0), sphere.max(0.0))
}

fn ancestor_multibound(nif: &NifFile, parents: &HashMap<usize, usize>, mut id: usize) -> Option<([f32; 3], [f32; 3])> {
    let block_ref = |block: &NifBlock, field: &str| match block.get_field(field) {
        Some(value) => usize::try_from(value.as_i64()).ok().filter(|&r| r < nif.blocks.len()),
        None => None,
    };
    while let Some(&parent) = parents.get(&id) {
        id = parent;
        let node = &nif.blocks[id];
        if node.type_name != "BSMultiBoundNode" {
            continue;
        }
        let data = block_ref(node, "Multi Bound").and_then(|mb| block_ref(&nif.blocks[mb], "Data"))?;
        let aabb = &nif.blocks[data];
        if aabb.type_name != "BSMultiBoundAABB" {
            return None;
        }
        return Some((vec3(aabb.get_field("Position"))?, vec3(aabb.get_field("Extent"))?));
    }
    None
}

/// World-space triangles of one shape with the segment each belongs to. Segment `Start Index`
/// counts indices, not triangles.
fn shape_triangles_world(
    shape: &NifBlock,
    geometry: &LodGeometry,
    chain: &[Transform],
) -> Vec<([[f32; 3]; 3], Option<u32>)> {
    let to_world = |p: [f32; 3]| to_world(chain, p);
    let ranges: Vec<(u32, u64, u64)> = val_array(shape.get_field("Segment"))
        .iter()
        .enumerate()
        .filter_map(|(seg, value)| match value {
            NifValue::Struct(fields) => {
                let start = val_u64(fields.get("Start Index")).unwrap_or(0) / 3;
                let count = val_u64(fields.get("Num Primitives")).unwrap_or(0);
                Some((seg as u32, start, start + count))
            }
            _ => None,
        })
        .collect();
    geometry
        .triangles
        .iter()
        .enumerate()
        .filter_map(|(index, tri)| {
            let vertex = |i: u32| geometry.vertices.get(i as usize).copied().map(to_world);
            let points = [vertex(tri[0])?, vertex(tri[1])?, vertex(tri[2])?];
            let index = index as u64;
            let segment = ranges
                .iter()
                .find(|(_, start, end)| index >= *start && index < *end)
                .map(|(seg, _, _)| *seg);
            Some((points, segment))
        })
        .collect()
}

/// Rotates to the smallest vertex but keeps the winding, so a two-sided back face is not a duplicate.
fn quantize(tri: &[[f32; 3]; 3]) -> [i32; 9] {
    let v: Vec<[i32; 3]> = tri
        .iter()
        .map(|p| [(p[0] * 8.0) as i32, (p[1] * 8.0) as i32, (p[2] * 8.0) as i32])
        .collect();
    let first = (0..3).min_by_key(|&i| v[i]).unwrap_or(0);
    let (a, b, c) = (v[first], v[(first + 1) % 3], v[(first + 2) % 3]);
    [a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2]]
}

fn triangle_in_segment_cell(tri: &[[f32; 3]; 3], seg: u32, level: u32, tile_x: i32, tile_y: i32) -> bool {
    let (cx, cy) = segment_cell(seg, level, tile_x, tile_y);
    let centroid = [
        (tri[0][0] + tri[1][0] + tri[2][0]) / 3.0,
        (tri[0][1] + tri[1][1] + tri[2][1]) / 3.0,
    ];
    let (min_x, min_y) = (cx as f32 * CELL, cy as f32 * CELL);
    let slack = 64.0;
    centroid[0] >= min_x - slack
        && centroid[0] <= min_x + CELL + slack
        && centroid[1] >= min_y - slack
        && centroid[1] <= min_y + CELL + slack
}

#[cfg(test)]
mod test_tiles {
    use nif_core_native::model::{NifFile, NifValue};

    use crate::descriptors::QuadDesc;
    use crate::objects::geometry::LodGeometry;
    use crate::objects::object_lod::{BtoShader, BtoShape, MultiBoundAabb, generate_segments};
    use crate::output::bto::build_bto_nif;

    const CELL: f32 = 4096.0;

    fn l4_quad() -> QuadDesc {
        QuadDesc {
            z_order: 0,
            x: 0,
            y: 0,
            quad_level: 4,
            quad_index: 0,
            quad_offset: 16_384.0,
            static_indices: Vec::new(),
            statics: Vec::new(),
            out_values: Default::default(),
        }
    }

    /// One triangle near the low corner of cell (cx, cy), tagged with segment `segment_cell`'s id.
    fn triangle_shape(cx: i32, cy: i32, segment_cell: (i32, i32)) -> BtoShape {
        let (x0, y0) = (cx as f32 * CELL + 100.0, cy as f32 * CELL + 100.0);
        let mut geometry = LodGeometry::new();
        geometry.vertices = vec![[x0, y0, 0.0], [x0 + 100.0, y0, 0.0], [x0, y0 + 100.0, 0.0]];
        geometry.uvcoords = vec![[0.0, 0.0]; 3];
        geometry.normals = vec![[0.0, 0.0, 1.0]; 3];
        geometry.tangents = vec![[1.0, 0.0, 0.0]; 3];
        geometry.bitangents = vec![[0.0, 1.0, 0.0]; 3];
        geometry.triangles = vec![[0, 1, 2]];
        geometry.update_bbox();
        let segments = generate_segments(
            &l4_quad(),
            segment_cell.0 as f32 * CELL + 100.0,
            segment_cell.1 as f32 * CELL + 100.0,
            1,
        );
        BtoShape {
            geometry,
            segments,
            segment_count: 4,
            translation: [0.0, 0.0, 0.0],
            scale: 1.0,
            center: [x0, y0, 0.0],
            radius: 100.0,
            multibound: MultiBoundAabb { position: [x0, y0, 0.0], extent: [100.0, 100.0, 1.0] },
            name_index_at: false,
            shader: BtoShader::Lighting {
                texture_set: Default::default(),
                flags1: 0x8040_0001,
                flags2: 5,
                clamp: 0,
                backlight: None,
                grayscale_scale: None,
            },
            alpha: None,
            enable_parent: 0,
        }
    }

    fn serialize(shapes: &[BtoShape]) -> Vec<u8> {
        build_bto_nif(shapes).unwrap().to_bytes().unwrap()
    }

    pub fn clean_l4_tile() -> Vec<u8> {
        serialize(&[triangle_shape(0, 0, (0, 0)), triangle_shape(1, 0, (1, 0))])
    }

    pub fn l4_tile_with_duplicate_and_stray() -> Vec<u8> {
        serialize(&[
            triangle_shape(0, 0, (0, 0)),
            triangle_shape(1, 0, (1, 0)),
            triangle_shape(0, 0, (0, 0)),
            triangle_shape(3, 3, (0, 0)),
        ])
    }

    /// A shape and its back face (same vertices, reversed winding), as two-sided LOD meshes carry.
    pub fn l4_tile_with_back_face() -> Vec<u8> {
        let mut back = triangle_shape(0, 0, (0, 0));
        back.geometry.triangles = vec![[0, 2, 1]];
        serialize(&[triangle_shape(0, 0, (0, 0)), back])
    }

    /// The clean tile with the first shape's multibound moved a cell away and its sphere shrunk.
    pub fn l4_tile_with_short_bounds() -> Vec<u8> {
        let mut short = triangle_shape(0, 0, (0, 0));
        short.multibound.position[0] += CELL;
        short.radius = 10.0;
        serialize(&[short, triangle_shape(1, 0, (1, 0))])
    }

    /// The first shape plus a copy raised `gap` units: two same-facing surfaces stacked.
    pub fn l4_tile_with_layer(gap: f32) -> Vec<u8> {
        let mut raised = triangle_shape(0, 0, (0, 0));
        for v in &mut raised.geometry.vertices {
            v[2] += gap;
        }
        serialize(&[triangle_shape(0, 0, (0, 0)), raised])
    }

    /// The first shape plus a copy moved 10 units along x and y in the same plane: two different
    /// triangles overlapping at exactly the same height.
    pub fn l4_tile_with_coplanar_overlap() -> Vec<u8> {
        let mut shifted = triangle_shape(0, 0, (0, 0));
        for v in &mut shifted.geometry.vertices {
            v[0] += 10.0;
            v[1] += 10.0;
        }
        serialize(&[triangle_shape(0, 0, (0, 0)), shifted])
    }

    /// As `l4_tile_with_layer`, but the raised copy faces the other way.
    pub fn l4_tile_with_opposite_layer(gap: f32) -> Vec<u8> {
        let mut raised = triangle_shape(0, 0, (0, 0));
        for v in &mut raised.geometry.vertices {
            v[2] += gap;
        }
        raised.geometry.triangles = vec![[0, 2, 1]];
        serialize(&[triangle_shape(0, 0, (0, 0)), raised])
    }

    /// Two triangles at height `z` covering cells (0,0)..(2,1), standing in for a terrain tile.
    pub fn flat_terrain(z: f32) -> Vec<u8> {
        let mut shape = triangle_shape(0, 0, (0, 0));
        shape.geometry.vertices = vec![[0.0, 0.0, z], [2.0 * CELL, 0.0, z], [2.0 * CELL, CELL, z], [0.0, CELL, z]];
        shape.geometry.uvcoords = vec![[0.0, 0.0]; 4];
        shape.geometry.normals = vec![[0.0, 0.0, 1.0]; 4];
        shape.geometry.tangents = vec![[1.0, 0.0, 0.0]; 4];
        shape.geometry.bitangents = vec![[0.0, 1.0, 0.0]; 4];
        shape.geometry.triangles = vec![[0, 1, 2], [0, 2, 3]];
        shape.geometry.update_bbox();
        serialize(&[shape])
    }

    pub fn tile_with_block(name: &str) -> Vec<u8> {
        let mut nif = NifFile::from_bytes(&clean_l4_tile(), None).unwrap();
        nif.add_block(name, None);
        nif.to_bytes().unwrap()
    }

    /// The clean tile with the second root child's shape also parented under the first root child.
    pub fn tile_with_two_geometries_in_one_root_child() -> Vec<u8> {
        let mut nif = NifFile::from_bytes(&clean_l4_tile(), None).unwrap();
        let root = nif.header.footer_roots[0] as usize;
        let children = |nif: &NifFile, id: usize| match nif.blocks[id].get_field("Children") {
            Some(NifValue::Array(items)) => items.iter().map(|v| v.as_i64() as usize).collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let root_children = children(&nif, root);
        let second_shape = children(&nif, root_children[1])[0];
        let first = root_children[0];
        let mut first_children = children(&nif, first);
        first_children.push(second_shape);
        let refs = first_children.iter().map(|&id| NifValue::Ref(id as i32)).collect();
        nif.blocks[first].set_field("Children", NifValue::Array(refs));
        nif.blocks[first].set_field("Num Children", NifValue::UInt(first_children.len() as u64));
        nif.to_bytes().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tile_names() {
        assert_eq!(parse_tile_name("APPALACHIA.8.98.75.bto"), Some(("APPALACHIA".into(), 8, 98, 75)));
        assert_eq!(parse_tile_name("Commonwealth.4.-12.-8.BTO"), Some(("Commonwealth".into(), 4, -12, -8)));
        assert_eq!(parse_tile_name("notatile.nif"), None);
    }

    #[test]
    fn clean_tile_has_no_defects() {
        let bytes = test_tiles::clean_l4_tile();
        let a = audit_tile(Path::new("W.4.0.0.bto"), &bytes).unwrap();
        assert_eq!(a.non_fo4_blocks, Vec::<String>::new());
        assert_eq!(a.out_of_cell_triangles, 0);
        assert_eq!(a.duplicate_triangles, 0);
        assert_eq!(a.root_children, 2);
        assert_eq!(a.root_children_not_single_geometry, 0);
        assert!(!a.empty);
        // lodgen trims trailing empty segments; the engine only walks the segments present (LOD_RUNTIME.md Q1).
        assert_eq!(a.segment_slots, vec![1, 5]);
    }

    #[test]
    fn flags_duplicates_and_out_of_cell_geometry() {
        let a = audit_tile(Path::new("W.4.0.0.bto"), &test_tiles::l4_tile_with_duplicate_and_stray()).unwrap();
        assert_eq!(a.duplicate_triangles, 1);
        assert_eq!(a.out_of_cell_triangles, 1);
    }

    #[test]
    fn back_faces_are_not_duplicates() {
        let a = audit_tile(Path::new("W.4.0.0.bto"), &test_tiles::l4_tile_with_back_face()).unwrap();
        assert_eq!(a.duplicate_triangles, 0);
    }

    #[test]
    fn flags_non_fo4_block_types() {
        let a = audit_tile(Path::new("W.4.0.0.bto"), &test_tiles::tile_with_block("BSDistantObjectInstancedNode")).unwrap();
        assert_eq!(a.non_fo4_blocks, vec!["BSDistantObjectInstancedNode".to_string()]);
    }

    #[test]
    fn clean_tile_bounds_cover_their_geometry() {
        let a = audit_tile(Path::new("W.4.0.0.bto"), &test_tiles::clean_l4_tile()).unwrap();
        assert_eq!(a.shapes_outside_multibound, 0);
        assert_eq!(a.shapes_outside_sphere, 0);
        assert_eq!(a.culling_modes, vec!["CULL_ALLPASS".to_string()]);
    }

    #[test]
    fn flags_bounds_that_miss_their_geometry() {
        let a = audit_tile(Path::new("W.4.0.0.bto"), &test_tiles::l4_tile_with_short_bounds()).unwrap();
        assert_eq!(a.shapes_outside_multibound, 1);
        assert_eq!(a.shapes_outside_sphere, 1);
    }

    #[test]
    fn counts_same_facing_layers_by_gap() {
        let a = audit_tile(Path::new("W.4.0.0.bto"), &test_tiles::l4_tile_with_layer(4.0)).unwrap();
        assert_eq!(a.layered_triangles, [0, 0, 2, 0]);
        let clean = audit_tile(Path::new("W.4.0.0.bto"), &test_tiles::clean_l4_tile()).unwrap();
        assert_eq!(clean.layered_triangles, [0; 4]);
        let back = audit_tile(Path::new("W.4.0.0.bto"), &test_tiles::l4_tile_with_back_face()).unwrap();
        assert_eq!(back.layered_triangles, [0; 4]);
    }

    #[test]
    fn counts_object_triangles_lying_on_terrain_by_gap() {
        let terrain = test_tiles::flat_terrain(-4.0);
        let objects = test_tiles::clean_l4_tile();
        assert_eq!(terrain_contact(&objects, &terrain, [0.0, 0.0]).unwrap(), [0, 0, 2, 0]);
        let far = test_tiles::flat_terrain(-100.0);
        assert_eq!(terrain_contact(&objects, &far, [0.0, 0.0]).unwrap(), [0; 4]);
    }

    #[test]
    fn terrain_height_at_reads_the_tile_surface() {
        let terrain = test_tiles::flat_terrain(-4.0);
        assert_eq!(terrain_height_at(&terrain, [0.0, 0.0], 100.0, 100.0).unwrap(), Some(-4.0));
        assert_eq!(terrain_height_at(&terrain, [CELL, 0.0], 100.0, 100.0).unwrap(), None);
    }

    #[test]
    fn terrain_tiles_are_placed_at_their_origin() {
        // Terrain vertices are tile-local; shifted a cell east the terrain only covers cell (1,0)'s triangle.
        let terrain = test_tiles::flat_terrain(-4.0);
        let objects = test_tiles::clean_l4_tile();
        assert_eq!(terrain_contact(&objects, &terrain, [CELL, 0.0]).unwrap(), [0, 0, 1, 0]);
    }

    #[test]
    fn counts_distinct_coplanar_overlaps() {
        let a = audit_tile(Path::new("W.4.0.0.bto"), &test_tiles::l4_tile_with_coplanar_overlap()).unwrap();
        assert_eq!(a.coplanar_overlaps, 2);
        let dup = audit_tile(Path::new("W.4.0.0.bto"), &test_tiles::l4_tile_with_duplicate_and_stray()).unwrap();
        assert_eq!(dup.coplanar_overlaps, 0);
        let back = audit_tile(Path::new("W.4.0.0.bto"), &test_tiles::l4_tile_with_back_face()).unwrap();
        assert_eq!(back.coplanar_overlaps, 0);
    }

    #[test]
    fn opposite_facing_layers_are_not_counted() {
        let a = audit_tile(Path::new("W.4.0.0.bto"), &test_tiles::l4_tile_with_opposite_layer(4.0)).unwrap();
        assert_eq!(a.layered_triangles, [0; 4]);
    }

    #[test]
    fn flags_root_children_with_more_than_one_geometry() {
        let a = audit_tile(Path::new("W.4.0.0.bto"), &test_tiles::tile_with_two_geometries_in_one_root_child()).unwrap();
        assert_eq!(a.root_children_not_single_geometry, 1);
    }
}


