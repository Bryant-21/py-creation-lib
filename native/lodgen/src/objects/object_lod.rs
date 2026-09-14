/// Per-NIF LOD geometry loader, segment IDs, multibound AABB, and BTO assembler.
///
/// Port sources:
/// - `generate_segments`  → LODApp.cs:261-293
/// - `expand_segments`    → BSSubIndexTriShape.cs:160-183
/// - `generate_multibound`→ LODApp.cs:241-259
/// - `parse_nif`          → LODApp.cs:1369 + LODApp.cs:295 (IterateNodes)
/// - `transform_shape`    → LODApp.cs:586
/// - `build_bto`          → LODApp.cs:2455-2651 (CreateLODNodesFO4)
use std::collections::BTreeMap;

use crate::descriptors::{BBox, QuadDesc};
use crate::objects::geometry::LodGeometry;
use crate::objects::static_desc::{ShaderKind, ShapeDesc, ShapeFlags, atlas_build_key};
use crate::progress::{ObjectModelTelemetry, ObjectQuadTelemetry, ObjectSimplifyStats};
use crate::settings::{Fo76BtoMultiboundMode, ObjectSettings};

const FO4_SAFE_BTO_ROOT_CHILDREN: usize = 8192;
const FO4_SAFE_BTO_SHAPE_VERTICES: usize = 60_000;
const FO4_SAFE_BTO_SHAPE_TRIANGLES: usize = 60_000;
const FO4_SHADER_FLAG_CAST_SHADOWS: u32 = 1 << 9;

// parse_nif / ShapeDesc loader lives in the `parse_nif` submodule
// (port: LODApp.IterateNodes/ParseNif + ShapeDesc ctor, FO4 object path).
pub use crate::objects::parse_nif::{iterate_nif, parse_nif};

#[derive(Clone)]
pub struct SegmentDesc {
    pub id: i32,
    pub start_triangle: u32,
    pub num_triangles: u16,
}

pub struct MultiBoundAabb {
    pub position: [f32; 3],
    pub extent: [f32; 3],
}

/// Port: LODApp.cs:261-293 GenerateSegments.
///
/// For quad_level==4 or level8: compute a 2D grid cell from shape's (X,Y) position,
/// clamp to [0, quad_level-1], then id = quad_level*num + num2.
/// Otherwise (e.g. level 16 without level8): id=0, single segment covers all triangles.
pub fn generate_segments(
    quad: &QuadDesc,
    shape_x: f32,
    shape_y: f32,
    num_triangles: u16,
) -> Vec<SegmentDesc> {
    // port: LODApp.cs:264 — level8 is a runtime flag; for pure level dispatch, only level==4 enters the grid branch
    let id = if quad.quad_level == 4 {
        // cell_size = quad_offset / quad_level
        let cell_size = quad.quad_offset / quad.quad_level as f32;
        let mut num = (shape_x / cell_size) as i32;
        let mut num2 = (shape_y / cell_size) as i32;
        // clamp: LODApp.cs:268-281
        if num >= quad.quad_level {
            num = quad.quad_level - 1;
        }
        if num < 0 {
            num = 0;
        }
        if num2 >= quad.quad_level {
            num2 = quad.quad_level - 1;
        }
        if num2 < 0 {
            num2 = 0;
        }
        quad.quad_level * num + num2
    } else {
        0
    };

    vec![SegmentDesc {
        id,
        start_triangle: 0,
        num_triangles,
    }]
}

/// Port: BSSubIndexTriShape.cs:160-183 SetSegments(List<SegmentDesc>, int count).
///
/// Expands to count*count slots (all zero), fills in the given segments by id,
/// then trims trailing zero-triangle entries from the end.
pub fn expand_segments(segments: &[SegmentDesc], count: i32) -> Vec<SegmentDesc> {
    let total = (count * count) as usize;
    let mut expanded: Vec<SegmentDesc> = (0..total)
        .map(|_| SegmentDesc {
            id: 0,
            start_triangle: 0,
            num_triangles: 0,
        })
        .collect();

    for seg in segments {
        let idx = seg.id as usize;
        if idx < total {
            expanded[idx] = SegmentDesc {
                id: seg.id,
                start_triangle: seg.start_triangle,
                num_triangles: seg.num_triangles,
            };
        }
    }

    // Trim trailing zero-triangle entries — port: BSSubIndexTriShape.cs:175-182
    let mut last = expanded.len();
    while last > 0 && expanded[last - 1].num_triangles == 0 {
        last -= 1;
    }
    expanded.truncate(last);

    expanded
}

/// Port: LODApp.cs:241-259 GenerateMultibound.
///
/// Computes the BSMultiBoundAABB position and extent in world space from bbox + quad origin.
/// When `!experimental` and pz2 < 0, clamps pz2 to 0 (underwater blocks).
pub fn generate_multibound(quad: &QuadDesc, bb: &BBox, experimental: bool) -> MultiBoundAabb {
    let qx = quad.x as f32 * 4096.0;
    let qy = quad.y as f32 * 4096.0;

    let px1 = bb.min[0];
    let px2 = bb.max[0];
    let py1 = bb.min[1];
    let py2 = bb.max[1];
    let pz1 = bb.min[2];
    // port: LODApp.cs:251-255 — z clamp when !experimental
    let pz2 = if !experimental && bb.max[2] < 0.0 {
        0.0
    } else {
        bb.max[2]
    };

    // port: LODApp.cs:256-257
    let pos_x = ((qx + px1) + (qx + px2)) / 2.0;
    let pos_y = ((qy + py1) + (qy + py2)) / 2.0;
    let pos_z = (pz1 + pz2) / 2.0;

    let ext_x = (px2 - px1) / 2.0;
    let ext_y = (py2 - py1) / 2.0;
    let ext_z = (pz2 - pz1) / 2.0;

    MultiBoundAabb {
        position: [pos_x, pos_y, pos_z],
        extent: [ext_x, ext_y, ext_z],
    }
}

fn generate_multibound_for_output_transform(
    local_bbox: &BBox,
    translation: [f32; 3],
    scale: f32,
    experimental: bool,
) -> MultiBoundAabb {
    let min = [
        translation[0] + local_bbox.min[0] * scale,
        translation[1] + local_bbox.min[1] * scale,
        translation[2] + local_bbox.min[2] * scale,
    ];
    let mut max = [
        translation[0] + local_bbox.max[0] * scale,
        translation[1] + local_bbox.max[1] * scale,
        translation[2] + local_bbox.max[2] * scale,
    ];
    if !experimental && max[2] < 0.0 {
        max[2] = 0.0;
    }

    MultiBoundAabb {
        position: [
            (min[0] + max[0]) / 2.0,
            (min[1] + max[1]) / 2.0,
            (min[2] + max[2]) / 2.0,
        ],
        extent: [
            (max[0] - min[0]) / 2.0,
            (max[1] - min[1]) / 2.0,
            (max[2] - min[2]) / 2.0,
        ],
    }
}

fn generate_tile_multibound_for_output_transform(
    quad: &QuadDesc,
    local_bbox: &BBox,
    translation: [f32; 3],
    scale: f32,
    experimental: bool,
) -> MultiBoundAabb {
    let tile_size = quad.quad_offset.max(4096.0);
    let pad_xy = 4096.0;
    let min_z = translation[2] + local_bbox.min[2] * scale;
    let mut max_z = translation[2] + local_bbox.max[2] * scale;
    if !experimental && max_z < 0.0 {
        max_z = 0.0;
    }

    MultiBoundAabb {
        position: [
            quad.x as f32 * 4096.0 + tile_size / 2.0,
            quad.y as f32 * 4096.0 + tile_size / 2.0,
            (min_z + max_z) / 2.0,
        ],
        extent: [
            tile_size / 2.0 + pad_xy,
            tile_size / 2.0 + pad_xy,
            ((max_z - min_z) / 2.0).max(65_536.0),
        ],
    }
}

// ---------------------------------------------------------------------------
// transform_shape — port: LODApp.cs:586-1052 TransformShape + GroupShape :496-531
// ---------------------------------------------------------------------------

/// Transform a shape's geometry into quad-local space and optionally remap UVs
/// through the atlas.
///
/// Port: `TransformShape` (LODApp.cs:586-1052) + atlas-UV branch of
/// `GroupShape` (LODApp.cs:496-531).
///
/// Returns `true` to keep the shape, `false` to drop it (zero tris after processing).
pub fn transform_shape(
    quad: &QuadDesc,
    stat: &crate::input::StaticDesc,
    shape: &mut crate::objects::static_desc::ShapeDesc,
    atlas: &crate::atlas::AtlasList,
    settings: &crate::settings::ObjectSettings,
) -> bool {
    transform_shape_impl(quad, stat, shape, atlas, settings, None)
}

pub fn transform_shape_with_world(
    quad: &QuadDesc,
    stat: &crate::input::StaticDesc,
    shape: &mut crate::objects::static_desc::ShapeDesc,
    atlas: &crate::atlas::AtlasList,
    settings: &crate::settings::ObjectSettings,
    world: &crate::input::WorldspaceInput,
) -> bool {
    transform_shape_impl(quad, stat, shape, atlas, settings, Some(world))
}

fn transform_shape_impl(
    quad: &QuadDesc,
    stat: &crate::input::StaticDesc,
    shape: &mut crate::objects::static_desc::ShapeDesc,
    atlas: &crate::atlas::AtlasList,
    settings: &crate::settings::ObjectSettings,
    world: Option<&crate::input::WorldspaceInput>,
) -> bool {
    if shape.geometry.num_triangles() == 0 {
        return false;
    }

    // Stat-relative translation: stat world pos minus quad origin.
    // port: LODApp.cs:757-765 (matrix4 translation).
    let tx = stat.pos[0] - quad.x as f32 * 4096.0;
    let ty = stat.pos[1] - quad.y as f32 * 4096.0;
    let tz = stat.pos[2];

    // shape.x / shape.y are the quad-local translation (set here for segments downstream).
    // port: LODApp.cs:941 area
    shape.x = tx;
    shape.y = ty;

    // TODO: C# TransformShape also implements three behaviours not ported here; static
    // object LOD doesn't need them, trees/grass do:
    //   - "scalexy" / "scalexy=<f>" name-hack — scales vertices in XY about the bbox
    //     center by `scaleXY` (or the parsed value) (LODApp.cs:830-855).
    //   - stat.scaleZ (flag6): per-vertex z-only scale when stat.scaleZ != 1
    //     (LODApp.cs:876, 882-885) — applied INSTEAD of the uniform stat.scale.
    //   - stat.color (flag7): per-vertex RGB tint multiply when vertex colors exist
    //     and stat.color is neither 0 nor 1 (LODApp.cs:877, 908-913).

    // The ref rotation matrix (upper-3x3 of matrix4): rotation_from_ref = Rx(-rx)·Ry(-ry)·Rz(-rz).
    // port: ShapeDesc.cs:361-367 — already stored in shape.rotation from parse_nif.
    let rot = shape.rotation;

    // Full accumulated node transform (matrix7 in C#): NiNode parent chain folded
    // with the geom block's own transform, translation column INCLUDED. Applied to
    // vertices as v' = node_transform · v (column-vector convention).
    // port: LODApp.cs:871 matrix7 = geom.GetTransform(parentScale) * parentTransform.
    let nt = mat4x4_mul(&stat.part_transform, &shape.node_transform);
    let node_rot = [
        [nt[0][0], nt[0][1], nt[0][2]],
        [nt[1][0], nt[1][1], nt[1][2]],
        [nt[2][0], nt[2][1], nt[2][2]],
    ];
    // Node-transform translation column — applied to vertices but NOT to normals.
    let node_trans = [nt[0][3], nt[1][3], nt[2][3]];

    // Combined rotation for normals/tangents: node_rot * ref_rot (no scale, no translation).
    // port: LODApp.cs:868 matrix6 = parentTransform.RemoveTranslation() * geom.GetTransform().RemoveTranslation() * matrix4.RemoveTranslation()
    let norm_rot = mat3x3_mul(&node_rot, &rot);

    let quad_level = quad.quad_level as f32;
    let node_scale = shape.node_scale * stat.part_scale;
    let stat_scale = stat.scale;

    // Transform each vertex and grow the bbox.
    // port: LODApp.cs:774-820
    let mut bbox = BBox::empty();
    let nv = shape.geometry.vertices.len();
    for i in 0..nv {
        let v = shape.geometry.vertices[i];
        // Step 1: apply node scale.
        let vx = v[0] * node_scale;
        let vy = v[1] * node_scale;
        let vz = v[2] * node_scale;
        // Step 2: apply the full accumulated node transform (rotation + translation).
        // port: LODApp.cs:881 `geometry.vertices *= matrix7` (matrix7 carries translation).
        let rx = node_rot[0][0] * vx + node_rot[0][1] * vy + node_rot[0][2] * vz + node_trans[0];
        let ry = node_rot[1][0] * vx + node_rot[1][1] * vy + node_rot[1][2] * vz + node_trans[1];
        let rz = node_rot[2][0] * vx + node_rot[2][1] * vy + node_rot[2][2] * vz + node_trans[2];
        // Step 3: apply stat scale.
        let sx = rx * stat_scale;
        let sy = ry * stat_scale;
        let sz = rz * stat_scale;
        // Step 4: apply ref rotation and add translation.
        let wx = rot[0][0] * sx + rot[0][1] * sy + rot[0][2] * sz + tx;
        let wy = rot[1][0] * sx + rot[1][1] * sy + rot[1][2] * sz + ty;
        let wz = rot[2][0] * sx + rot[2][1] * sy + rot[2][2] * sz + tz;
        // Grow world bbox (pre-divide).
        bbox.grow_vertex([wx, wy, wz]);
        // Step 5: divide into quad space.
        shape.geometry.vertices[i] = [wx / quad_level, wy / quad_level, wz / quad_level];
    }

    // Store bbox in world space (pre-divide).
    shape.bounding_box = bbox;

    // Transform normals, tangents, bitangents through rotation-only matrix (no scale/translation).
    // port: LODApp.cs:788-806
    for n in &mut shape.geometry.normals {
        *n = mat3x3_transform_vec(&norm_rot, *n);
    }
    for t in &mut shape.geometry.tangents {
        *t = mat3x3_transform_vec(&norm_rot, *t);
    }
    for b in &mut shape.geometry.bitangents {
        *b = mat3x3_transform_vec(&norm_rot, *b);
    }

    // Clear IS_HIGH_DETAIL — port: LODApp.cs:941
    shape.flags.remove(ShapeFlags::IS_HIGH_DETAIL);

    // Clear vertex colors if not passthru and no_vertex_colors — port: LODApp.cs:1033-1036
    if settings.no_vertex_colors && !shape.flags.contains(ShapeFlags::IS_PASSTHRU) {
        shape.flags.remove(ShapeFlags::HAS_VERTEX_COLOR);
        shape.geometry.vertex_colors.clear();
    }

    // Atlas UV remap — port: GroupShape LODApp.cs:496-531
    let tol_min = 0.0 - (settings.uv_range - 1.0);
    let tol_max = 1.0 + (settings.uv_range - 1.0);

    // force = IS_GRASS or texture_clamp_mode != 3 (WRAP_S_WRAP_T)
    let force = shape.flags.contains(ShapeFlags::IS_GRASS) || shape.texture_clamp_mode != 3;

    let textures_key = atlas_build_key(atlas, &shape.textures[..3], shape.alpha_threshold);
    shape.textures_key = textures_key.clone();

    // Check if the shape's key is in the atlas and not noatlas.
    let no_atlas = shape.name.to_lowercase().contains("noatlas")
        || shape.static_model.to_lowercase().contains("noatlas");

    if !no_atlas && atlas.contains(&textures_key) {
        if let Some(rect) = atlas.get(&textures_key) {
            let rect = rect.clone();

            // UV tolerance check (skip if force=true); port: GroupShape :506-515.
            //
            // Deliberate deviation: C# AtlasDesc.UVAtlas (AtlasDesc.cs:77) tests
            // `u > AtlasToleranceMax` twice, accepting an out-of-range v with an
            // in-range u. This tests `uv[1] > tol_max` as intended.
            if !force {
                for uv in &shape.geometry.uvcoords {
                    if uv[0] < tol_min || uv[0] > tol_max || uv[1] < tol_min || uv[1] > tol_max {
                        // UV out of tolerance — keep HD textures, don't remap.
                        return true;
                    }
                }
            }

            // Remap all UVs through the atlas rect — port: GroupShape :517-520
            let new_uvs: Vec<[f32; 2]> = shape
                .geometry
                .uvcoords
                .iter()
                .map(|uv| {
                    let (u, v) = rect.uv_atlas(uv[0], uv[1]);
                    [u, v]
                })
                .collect();
            shape.geometry.set_uvcoords(new_uvs);

            // Per-slot texture substitution — port: TransformShape LODApp.cs:741-816.
            // C# does NOT blindly overwrite [0]/[1]/[7]; it walks all 10 slots and
            // maps each by VALUE: diffuse→atlasDiffuse, normal→atlasNormal,
            // specular→atlasSpecular (FO4), and PRESERVES the White/Gray/Grey/Black/
            // Brightyellow/Default_n/FlatWhite01_d/FlatFlat_n/White01_s sentinels.
            // Any other slot is cleared to empty.
            let diffuse = shape.textures[0].clone();
            let normal = shape.textures[1].clone();
            let specular = shape.textures[7].clone();
            // C# also maps slot[2] glow → AtlasTextureG when present; no glow atlas
            // is built (AtlasRect has no glow field), so glow slots are not remapped.
            let mut new_slots: [String; 10] = Default::default();
            for l in 0..10 {
                let t = &shape.textures[l];
                if t.is_empty() {
                    continue;
                }
                if eq_ci(t, &diffuse) {
                    new_slots[l] = rect.atlas_diffuse.clone();
                } else if eq_ci(t, &normal) {
                    new_slots[l] = rect.atlas_normal.clone();
                } else if !specular.is_empty() && eq_ci(t, &specular) {
                    new_slots[l] = rect.atlas_specular.clone();
                } else if is_atlas_sentinel(t) {
                    new_slots[l] = t.clone();
                }
                // else: leave empty (C# leaves list[l] = string.Empty).
            }
            // FO4 greyscale-to-palette / greyscale-to-alpha keep the palette in
            // slot 3 (LODApp.cs:778-781, 812-815).
            if shape.flags.contains(ShapeFlags::IS_GREYSCALE_TO_PALETTE)
                || shape.flags.contains(ShapeFlags::IS_GREYSCALE_TO_ALPHA)
            {
                new_slots[3] = shape.textures[3].clone();
            }
            shape.textures = new_slots;
            shape.textures_key =
                atlas_build_key(atlas, &shape.textures[..3], shape.alpha_threshold);

            // Clear IS_HIGH_DETAIL and set clamp to 0 — port: GroupShape :531
            shape.flags.remove(ShapeFlags::IS_HIGH_DETAIL);
            shape.texture_clamp_mode = 0;
        }
    }

    if settings.remove_unseen_faces
        && world
            .map(|w| !remove_unseen_faces(quad, shape, w))
            .unwrap_or(false)
    {
        return false;
    }

    // GenerateSegments is the LAST step of TransformShape — port: LODApp.cs:1050.
    // It assigns the grid segment id from the shape's quad-local (X,Y) and the
    // post-transform triangle count, and stores it on the shape so build_bto /
    // expand_segments can emit a non-empty Segment array (Num Segments >= 1).
    shape.segments = generate_segments(
        quad,
        shape.x,
        shape.y,
        shape.geometry.num_triangles() as u16,
    );

    true
}

fn remove_unseen_faces(
    quad: &QuadDesc,
    shape: &mut crate::objects::static_desc::ShapeDesc,
    world: &crate::input::WorldspaceInput,
) -> bool {
    if shape.geometry.triangles.is_empty() || shape.geometry.vertices.is_empty() {
        return false;
    }

    let level = quad.quad_level as f32;
    let mut visible = vec![true; shape.geometry.vertices.len()];
    let mut cell_cache = std::collections::BTreeMap::<(i32, i32), Option<usize>>::new();

    for (i, vertex) in shape.geometry.vertices.iter().enumerate() {
        let local_x = vertex[0] * level;
        let local_y = vertex[1] * level;
        let z = vertex[2] * level;
        let cell_dx = (local_x / 4096.0).floor() as i32;
        let cell_dy = (local_y / 4096.0).floor() as i32;
        let cell_key = (quad.x + cell_dx, quad.y + cell_dy);

        let cell_index = *cell_cache.entry(cell_key).or_insert_with(|| {
            world
                .cells
                .iter()
                .position(|cell| cell.x == cell_key.0 && cell.y == cell_key.1)
        });
        let Some(cell_index) = cell_index else {
            continue;
        };
        let cell = &world.cells[cell_index];

        if is_under_water(cell.water_height, z) {
            visible[i] = false;
            continue;
        }

        let cell_local_x = local_x.rem_euclid(4096.0);
        let cell_local_y = local_y.rem_euclid(4096.0);
        let Some((patch_min, patch_max, sampled_height)) =
            terrain_patch_height(cell, cell_local_x, cell_local_y)
        else {
            continue;
        };

        if z < patch_min || (z <= patch_max && z < sampled_height) {
            visible[i] = false;
        }
    }

    let before = shape.geometry.triangles.len();
    shape.geometry.triangles.retain(|tri| {
        visible[tri[0] as usize] || visible[tri[1] as usize] || visible[tri[2] as usize]
    });
    if shape.geometry.triangles.len() != before {
        shape.geometry.remove_unused();
    }

    !shape.geometry.triangles.is_empty()
}

fn is_under_water(water_height: f32, z: f32) -> bool {
    !water_height.is_nan() && water_height < 16_777_216.0 && water_height > z
}

fn terrain_patch_height(
    cell: &crate::input::CellInput,
    local_x: f32,
    local_y: f32,
) -> Option<(f32, f32, f32)> {
    if cell.heights.len() != 33 * 33 {
        return None;
    }

    let grid_coord = |local: f32| {
        let f = (local / 128.0).clamp(0.0, 32.0);
        let base = (f.floor() as usize).min(31);
        (base, (f - base as f32).clamp(0.0, 1.0))
    };
    let (x0, tx) = grid_coord(local_x);
    let (y0, ty) = grid_coord(local_y);
    let h = |x: usize, y: usize| cell.heights[x + y * 33];
    let h00 = h(x0, y0);
    let h10 = h(x0 + 1, y0);
    let h01 = h(x0, y0 + 1);
    let h11 = h(x0 + 1, y0 + 1);
    let patch_min = h00.min(h10).min(h01).min(h11);
    let patch_max = h00.max(h10).max(h01).max(h11);
    let hx0 = h00 + (h10 - h00) * tx;
    let hx1 = h01 + (h11 - h01) * tx;
    let sampled_height = hx0 + (hx1 - hx0) * ty;
    Some((patch_min, patch_max, sampled_height))
}

/// Case-insensitive string equality (C# StringComparison.OrdinalIgnoreCase).
fn eq_ci(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// True if a texture slot is one of the engine sentinel textures that the atlas
/// swap must PRESERVE rather than clear/remap.
/// Port: the sentinel list in TransformShape (LODApp.cs:652 / :774).
fn is_atlas_sentinel(t: &str) -> bool {
    const SENTINELS: [&str; 9] = [
        "Textures\\White.dds",
        "Textures\\Gray.dds",
        "Textures\\Grey.dds",
        "Textures\\Black.dds",
        "Textures\\Brightyellow.dds",
        "Textures\\Default_n.dds",
        "Textures\\Shared\\FlatWhite01_d.dds",
        "Textures\\Shared\\FlatFlat_n.dds",
        "Textures\\Shared\\White01_s.dds",
    ];
    SENTINELS.iter().any(|s| eq_ci(t, s))
}

/// Multiply two 3x3 row-major matrices: C = A * B.
fn mat3x3_mul(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            for k in 0..3 {
                out[i][j] += a[i][k] * b[k][j];
            }
        }
    }
    out
}

/// Transform a 3-vector by a 3x3 row-major matrix: v' = M * v.
fn mat3x3_transform_vec(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn mat4x4_mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            for k in 0..4 {
                out[i][j] += a[i][k] * b[k][j];
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// build_bto — port: LODApp.CreateLODNodesFO4 (LODApp.cs:2455-2651)
// ---------------------------------------------------------------------------

/// One BSMultiBoundNode subtree to emit into the `.bto`.
/// Port: the per-shape state assembled in CreateLODNodesFO4 (LODApp.cs:2457-2649).
pub struct BtoShape {
    pub geometry: LodGeometry,
    pub segments: Vec<SegmentDesc>,
    /// Grid count for SetSegments expansion (= quad.quadLevel; count*count slots).
    pub segment_count: i32,
    pub translation: [f32; 3],
    pub scale: f32,
    pub center: [f32; 3],
    pub radius: f32,
    pub multibound: MultiBoundAabb,
    /// "obj" (false) vs "obj-at" (true, alpha shapes).
    pub name_index_at: bool,
    pub shader: BtoShader,
    pub alpha: Option<BtoAlpha>,
    pub enable_parent: u32,
}

/// The shader property to build for a shape.
/// Always emits a fresh BSLightingShaderProperty for non-passthru
/// object-LOD shapes (the dominant golden path). Passthru Effect shaders are a
/// later extension (carried from parse_nif) — noted in the report.
pub enum BtoShader {
    Lighting {
        texture_set: [String; 10],
        flags1: u32,
        flags2: u32,
        clamp: u32,
        backlight: Option<f32>,
        grayscale_scale: Option<f32>,
    },
}

/// NiAlphaProperty parameters for an alpha shape.
pub struct BtoAlpha {
    pub flags: u16,
    pub threshold: u8,
}

/// Build the `BtoShape` list for a quad from its merged/prepared shapes.
///
/// Port: LODApp.CreateLODNodesFO4 (LODApp.cs:2455-2651). Each input `ShapeDesc`
/// has already been transformed into quad space and segmented; this
/// derives the per-shape shader flags, scale/translation/bounds, multibound, and
/// alpha state, accumulating tri counts into `quad.out_values`.
///
/// The geometry bbox is expected in quad-local (post-`/quadLevel`) space (set by
/// transform_shape); the multibound and BoundingBox scale it back by quadLevel
/// (LODApp.cs:2478).
pub fn build_bto(
    quad: &mut QuadDesc,
    shapes: Vec<ShapeDesc>,
    settings: &ObjectSettings,
) -> Vec<BtoShape> {
    build_bto_with_telemetry(quad, shapes, settings).0
}

pub fn build_bto_with_telemetry(
    quad: &mut QuadDesc,
    shapes: Vec<ShapeDesc>,
    settings: &ObjectSettings,
) -> (Vec<BtoShape>, ObjectQuadTelemetry) {
    let mut telemetry = ObjectQuadTelemetry {
        level: quad.quad_level,
        x: quad.x,
        y: quad.y,
        ..ObjectQuadTelemetry::default()
    };
    let prepared = prepare_shapes_for_bto(shapes, true);
    telemetry.simplify.shapes_considered = prepared.len() as u64;
    set_final_simplify_triangle_counts(&prepared, &mut telemetry.simplify);
    telemetry.models = collect_model_telemetry(&prepared);
    let mut bto_shapes = build_bto_shapes(quad, prepared, settings, true);
    cap_bto_root_children_for_fo4(&mut bto_shapes, &mut telemetry);
    (bto_shapes, telemetry)
}

pub fn build_source_bto_with_telemetry(
    quad: &mut QuadDesc,
    shapes: Vec<ShapeDesc>,
    settings: &ObjectSettings,
) -> (Vec<BtoShape>, ObjectQuadTelemetry) {
    build_source_bto_with_telemetry_impl(
        quad,
        shapes,
        settings,
        settings.fo76_bto_merge_atlassed_shapes,
    )
}

pub fn build_atlassed_source_bto_with_telemetry(
    quad: &mut QuadDesc,
    shapes: Vec<ShapeDesc>,
    settings: &ObjectSettings,
) -> (Vec<BtoShape>, ObjectQuadTelemetry) {
    build_source_bto_with_telemetry_impl(
        quad,
        shapes,
        settings,
        settings.fo76_bto_merge_atlassed_shapes,
    )
}

fn build_source_bto_with_telemetry_impl(
    quad: &mut QuadDesc,
    shapes: Vec<ShapeDesc>,
    settings: &ObjectSettings,
    merge_compatible: bool,
) -> (Vec<BtoShape>, ObjectQuadTelemetry) {
    let mut telemetry = ObjectQuadTelemetry {
        level: quad.quad_level,
        x: quad.x,
        y: quad.y,
        ..ObjectQuadTelemetry::default()
    };
    let prepared = prepare_shapes_for_bto(shapes, false);
    telemetry.simplify.shapes_considered = prepared.len() as u64;
    set_final_simplify_triangle_counts(&prepared, &mut telemetry.simplify);
    telemetry.models = collect_model_telemetry(&prepared);
    let mut bto_shapes = build_bto_shapes(quad, prepared, settings, merge_compatible);
    cap_bto_root_children_for_fo4(&mut bto_shapes, &mut telemetry);
    (bto_shapes, telemetry)
}

struct PreparedShape {
    shape: ShapeDesc,
    model: String,
    triangles_before: usize,
}

fn prepare_shapes_for_bto(shapes: Vec<ShapeDesc>, optimize_geometry: bool) -> Vec<PreparedShape> {
    let mut prepared = Vec::with_capacity(shapes.len());
    for mut shape in shapes {
        if optimize_geometry {
            shape.geometry.optimize();
        }
        let nt = shape.geometry.num_triangles();
        if nt == 0 {
            continue;
        }

        let model = shape.static_model.clone();
        prepared.push(PreparedShape {
            shape,
            model,
            triangles_before: nt,
        });
    }
    prepared
}

fn build_bto_shapes(
    quad: &mut QuadDesc,
    prepared: Vec<PreparedShape>,
    settings: &ObjectSettings,
    merge_compatible: bool,
) -> Vec<BtoShape> {
    let quad_level = quad.quad_level as f32;
    let mut out = Vec::with_capacity(prepared.len());

    for PreparedShape { mut shape, .. } in prepared {
        let nt = shape.geometry.num_triangles();
        if nt == 0 {
            continue;
        }
        refresh_segments_for_final_geometry(quad, &mut shape, nt);

        // Quad-local bbox from the transformed geometry (post-optimize).
        let local_bbox = geom_bbox(&shape.geometry);

        // Bounding sphere center/radius in quad-local space (BSSITS uses the
        // local geometry frame; Translation carries the quad origin).
        let center = local_bbox.center(false);
        let radius = local_bbox.radius();

        let output_translation =
            shape
                .bto_translation
                .unwrap_or([quad.x as f32 * 4096.0, quad.y as f32 * 4096.0, 0.0]);
        let output_scale = shape.bto_scale.unwrap_or(quad_level);
        let multibound = match settings.fo76_bto_multibound_mode {
            Fo76BtoMultiboundMode::Shape => generate_multibound_for_output_transform(
                &local_bbox,
                output_translation,
                output_scale,
                false,
            ),
            Fo76BtoMultiboundMode::Tile => generate_tile_multibound_for_output_transform(
                quad,
                &local_bbox,
                output_translation,
                output_scale,
                false,
            ),
        };

        let is_alpha = shape.flags.contains(ShapeFlags::IS_ALPHA);

        // Shader flags — port: LODApp.cs:2589-2620 (non-passthru branch).
        let mut flags1: u32 = 2151677953; // 0x80400801 = Specular|Own_Emit|ZBuffer_Test
        let mut flags2: u32 = 1; // ZBuffer_Write
        if shape.flags.contains(ShapeFlags::CASTS_SHADOWS) {
            flags1 |= FO4_SHADER_FLAG_CAST_SHADOWS;
        }
        if shape.flags.contains(ShapeFlags::IS_DECAL) {
            flags1 |= 0x4000000;
            flags1 |= 0x8000000;
            flags2 |= 8;
        }
        let mut grayscale_scale = None;
        if shape.flags.contains(ShapeFlags::IS_GREYSCALE_TO_PALETTE) {
            flags1 |= 0x10;
            grayscale_scale = Some(shape.grayscale_to_palette_scale);
        }
        if shape.flags.contains(ShapeFlags::IS_GREYSCALE_TO_ALPHA) {
            flags1 |= 0x20;
            grayscale_scale = Some(shape.grayscale_to_palette_scale);
        }
        if shape.flags.contains(ShapeFlags::HAS_LOD_FLAG) {
            flags2 |= 4; // LOD_Objects
        }
        if shape.flags.contains(ShapeFlags::HAS_VERTEX_COLOR) {
            flags2 |= 0x20;
        }
        if shape.flags.contains(ShapeFlags::IS_DOUBLE_SIDED)
            || (is_alpha && alpha_double_sided(settings))
        {
            flags2 |= 0x10;
        }

        let backlight = if settings.use_backlight {
            Some(shape.backlight_power)
        } else {
            None
        };

        let shader = BtoShader::Lighting {
            texture_set: shape.textures.clone(),
            flags1,
            flags2,
            clamp: shape.texture_clamp_mode,
            backlight,
            grayscale_scale,
        };

        // Alpha property — port: LODApp.cs:2622-2647.
        let alpha = if is_alpha {
            let threshold = if settings.use_alpha_threshold {
                shape.alpha_threshold
            } else {
                settings.alpha_threshold
            };
            Some(BtoAlpha {
                flags: 4844,
                threshold,
            })
        } else {
            None
        };

        // Triangle-count accounting (OutDesc).
        quad.out_values.total_tri_count += nt as u32;
        quad.out_values.reduced_tri_count += nt as u32;

        out.push(BtoShape {
            geometry: shape.geometry,
            segments: shape.segments,
            segment_count: quad.quad_level,
            translation: output_translation,
            scale: output_scale,
            center,
            radius,
            multibound,
            name_index_at: is_alpha,
            shader,
            alpha,
            enable_parent: shape.enable_parent,
        });

        // Reference ShaderKind so the no-shader branch stays exhaustive-aware.
        let _ = ShaderKind::None;
    }

    if merge_compatible {
        merge_compatible_bto_shapes(quad, out, settings)
    } else {
        out
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct BtoMergeKey {
    name_index_at: bool,
    shader: BtoShaderKey,
    alpha: Option<BtoAlphaKey>,
    enable_parent: u32,
    translation: [u32; 3],
    scale: u32,
    segment_count: i32,
    has_uvs: bool,
    has_normals: bool,
    has_tangents: bool,
    has_vertex_colors: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum BtoShaderKey {
    Lighting {
        texture_set: [String; 10],
        flags1: u32,
        flags2: u32,
        clamp: u32,
        backlight: Option<u32>,
        grayscale_scale: Option<u32>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct BtoAlphaKey {
    flags: u16,
    threshold: u8,
}

struct BtoMergeGroup {
    shapes: Vec<BtoShape>,
}

fn merge_compatible_bto_shapes(
    quad: &QuadDesc,
    shapes: Vec<BtoShape>,
    settings: &ObjectSettings,
) -> Vec<BtoShape> {
    if shapes.len() <= 1 {
        return shapes;
    }

    let mut groups: Vec<BtoMergeGroup> = Vec::new();
    let mut group_by_key: BTreeMap<BtoMergeKey, usize> = BTreeMap::new();
    let mut passthrough = Vec::new();

    for shape in shapes {
        if mergeable_segment_id(&shape).is_none() {
            passthrough.push(shape);
            continue;
        }
        let key = BtoMergeKey::from_shape(&shape);
        if let Some(&idx) = group_by_key.get(&key) {
            groups[idx].shapes.push(shape);
        } else {
            let idx = groups.len();
            group_by_key.insert(key, idx);
            groups.push(BtoMergeGroup {
                shapes: vec![shape],
            });
        }
    }

    let mut merged = Vec::with_capacity(groups.len() + passthrough.len());
    for mut group in groups {
        group
            .shapes
            .sort_by_key(|shape| mergeable_segment_id(shape).unwrap_or(i32::MAX));
        merged.extend(merge_bto_group(quad, group.shapes, settings));
    }
    merged.extend(passthrough);
    merged
}

impl BtoMergeKey {
    fn from_shape(shape: &BtoShape) -> Self {
        Self {
            name_index_at: shape.name_index_at,
            shader: BtoShaderKey::from_shader(&shape.shader),
            alpha: shape.alpha.as_ref().map(BtoAlphaKey::from_alpha),
            enable_parent: shape.enable_parent,
            translation: [
                shape.translation[0].to_bits(),
                shape.translation[1].to_bits(),
                shape.translation[2].to_bits(),
            ],
            scale: shape.scale.to_bits(),
            segment_count: shape.segment_count,
            has_uvs: !shape.geometry.uvcoords.is_empty(),
            has_normals: !shape.geometry.normals.is_empty(),
            has_tangents: !shape.geometry.tangents.is_empty(),
            has_vertex_colors: !shape.geometry.vertex_colors.is_empty(),
        }
    }
}

impl BtoShaderKey {
    fn from_shader(shader: &BtoShader) -> Self {
        match shader {
            BtoShader::Lighting {
                texture_set,
                flags1,
                flags2,
                clamp,
                backlight,
                grayscale_scale,
            } => BtoShaderKey::Lighting {
                texture_set: texture_set.clone(),
                flags1: *flags1,
                flags2: *flags2,
                clamp: *clamp,
                backlight: backlight.map(f32::to_bits),
                grayscale_scale: grayscale_scale.map(f32::to_bits),
            },
        }
    }
}

impl BtoAlphaKey {
    fn from_alpha(alpha: &BtoAlpha) -> Self {
        Self {
            flags: alpha.flags,
            threshold: alpha.threshold,
        }
    }
}

fn merge_bto_group(
    quad: &QuadDesc,
    shapes: Vec<BtoShape>,
    settings: &ObjectSettings,
) -> Vec<BtoShape> {
    let mut buckets: Vec<BtoShape> = Vec::new();

    for shape in shapes {
        if let Some(bucket) = buckets
            .iter_mut()
            .find(|bucket| can_append_bto_shape(bucket, &shape))
        {
            append_bto_shape(bucket, shape);
        } else {
            buckets.push(shape);
        }
    }

    for bucket in &mut buckets {
        refresh_bto_shape_bounds(quad, bucket, settings);
    }

    buckets
}

fn mergeable_segment_id(shape: &BtoShape) -> Option<i32> {
    if shape.segments.len() == 1 {
        Some(shape.segments[0].id)
    } else {
        None
    }
}

fn can_append_bto_shape(bucket: &BtoShape, shape: &BtoShape) -> bool {
    if bucket.geometry.num_vertices() + shape.geometry.num_vertices() > FO4_SAFE_BTO_SHAPE_VERTICES
        || bucket.geometry.num_triangles() + shape.geometry.num_triangles()
            > FO4_SAFE_BTO_SHAPE_TRIANGLES
    {
        return false;
    }

    let Some(segment) = shape.segments.first() else {
        return false;
    };
    let existing = bucket
        .segments
        .iter()
        .filter(|s| s.id == segment.id)
        .map(|s| s.num_triangles as usize)
        .sum::<usize>();
    existing + segment.num_triangles as usize <= u16::MAX as usize
}

fn append_bto_shape(bucket: &mut BtoShape, shape: BtoShape) {
    let tri_offset = bucket.geometry.num_triangles() as u32;
    let incoming_segments = shape.segments.clone();
    append_lod_geometry(&mut bucket.geometry, shape.geometry);

    for segment in incoming_segments {
        append_segment(
            &mut bucket.segments,
            segment.id,
            tri_offset + segment.start_triangle,
            segment.num_triangles,
        );
    }
}

fn append_segment(
    segments: &mut Vec<SegmentDesc>,
    id: i32,
    start_triangle: u32,
    num_triangles: u16,
) {
    if let Some(last) = segments.last_mut() {
        let expected_start = last.start_triangle + last.num_triangles as u32;
        if last.id == id && expected_start == start_triangle {
            last.num_triangles = last.num_triangles.saturating_add(num_triangles);
            return;
        }
    }

    segments.push(SegmentDesc {
        id,
        start_triangle,
        num_triangles,
    });
}

fn append_lod_geometry(dst: &mut LodGeometry, src: LodGeometry) {
    let vertex_offset = dst.vertices.len() as u32;
    dst.vertices.extend(src.vertices);
    dst.uvcoords.extend(src.uvcoords);
    dst.normals.extend(src.normals);
    dst.tangents.extend(src.tangents);
    dst.bitangents.extend(src.bitangents);
    dst.vertex_colors.extend(src.vertex_colors);
    dst.triangles.extend(src.triangles.into_iter().map(|t| {
        [
            t[0] + vertex_offset,
            t[1] + vertex_offset,
            t[2] + vertex_offset,
        ]
    }));
    dst.update_bbox();
}

fn refresh_bto_shape_bounds(quad: &QuadDesc, shape: &mut BtoShape, settings: &ObjectSettings) {
    let local_bbox = geom_bbox(&shape.geometry);
    shape.center = local_bbox.center(false);
    shape.radius = local_bbox.radius();
    shape.multibound = match settings.fo76_bto_multibound_mode {
        Fo76BtoMultiboundMode::Shape => generate_multibound_for_output_transform(
            &local_bbox,
            shape.translation,
            shape.scale,
            false,
        ),
        Fo76BtoMultiboundMode::Tile => generate_tile_multibound_for_output_transform(
            quad,
            &local_bbox,
            shape.translation,
            shape.scale,
            false,
        ),
    };
}

fn cap_bto_root_children_for_fo4(shapes: &mut Vec<BtoShape>, telemetry: &mut ObjectQuadTelemetry) {
    if shapes.len() <= FO4_SAFE_BTO_ROOT_CHILDREN {
        return;
    }

    shapes.sort_by(|a, b| {
        bto_shape_priority(b)
            .total_cmp(&bto_shape_priority(a))
            .then_with(|| b.geometry.num_triangles().cmp(&a.geometry.num_triangles()))
    });

    let skipped = shapes.len() - FO4_SAFE_BTO_ROOT_CHILDREN;
    let skipped_triangles = shapes[FO4_SAFE_BTO_ROOT_CHILDREN..]
        .iter()
        .map(|shape| shape.geometry.num_triangles() as u64)
        .sum::<u64>();
    shapes.truncate(FO4_SAFE_BTO_ROOT_CHILDREN);

    telemetry.simplify.shapes_skipped += skipped as u64;
    telemetry.simplify.triangles_after = telemetry
        .simplify
        .triangles_after
        .saturating_sub(skipped_triangles);
}

fn bto_shape_priority(shape: &BtoShape) -> f32 {
    if shape.radius.is_finite() {
        shape.radius
    } else {
        0.0
    }
}

fn refresh_segments_for_final_geometry(quad: &QuadDesc, shape: &mut ShapeDesc, nt: usize) {
    shape.segments = generate_segments(quad, shape.x, shape.y, nt.min(u16::MAX as usize) as u16);
}

fn collect_model_telemetry(prepared: &[PreparedShape]) -> Vec<ObjectModelTelemetry> {
    let mut by_model: BTreeMap<String, ObjectModelTelemetry> = BTreeMap::new();
    for prepared_shape in prepared {
        let entry = by_model
            .entry(prepared_shape.model.clone())
            .or_insert_with(|| ObjectModelTelemetry {
                model: prepared_shape.model.clone(),
                ..ObjectModelTelemetry::default()
            });
        entry.shape_count += 1;
        entry.triangles_before += prepared_shape.triangles_before as u64;
        entry.triangles_after += prepared_shape.shape.geometry.num_triangles() as u64;
    }
    by_model.into_values().collect()
}

fn set_final_simplify_triangle_counts(
    prepared: &[PreparedShape],
    telemetry: &mut ObjectSimplifyStats,
) {
    telemetry.triangles_before = prepared
        .iter()
        .map(|shape| shape.triangles_before as u64)
        .sum();
    telemetry.triangles_after = prepared
        .iter()
        .map(|shape| shape.shape.geometry.num_triangles() as u64)
        .sum();
}

/// alpha-double-sided is an xLODGen toggle; FO4 default leaves alpha shapes
/// single-sided unless explicitly enabled. We mirror the default-off behaviour
/// (ObjectSettings has no alpha-double-sided field) — port: LODApp alphaDoubleSided.
fn alpha_double_sided(_settings: &ObjectSettings) -> bool {
    false
}

/// Compute the bbox of a geometry's vertices.
fn geom_bbox(g: &LodGeometry) -> BBox {
    let mut bb = BBox::empty();
    for &v in &g.vertices {
        bb.grow_vertex(v);
    }
    bb
}

#[cfg(test)]
mod unseen_tests {
    use super::*;
    use crate::atlas::AtlasList;
    use crate::input::{CellInput, RefInput, WorldspaceInput};
    use crate::objects::static_desc::{ShaderKind, ShapeDesc, ShapeFlags};
    use crate::settings::{Fo76BtoMultiboundMode, LodSettings};

    fn identity4() -> [[f32; 4]; 4] {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }

    fn identity3() -> [[f32; 3]; 3] {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    }

    fn flat_cell(height: f32) -> CellInput {
        CellInput {
            x: 0,
            y: 0,
            heights: vec![height; 33 * 33],
            vertex_colors: Vec::new(),
            layers: Vec::new(),
            hidden_quadrants: [false; 4],
            water_height: f32::MIN,
        }
    }

    fn stat() -> RefInput {
        RefInput {
            ref_id: "00000001".to_string(),
            ref_flags: 0,
            enable_parent: 0,
            cell: (0, 0),
            pos: [0.0; 3],
            rot: [0.0; 3],
            scale: 1.0,
            color: 1.0,
            alpha_threshold: 128,
            is_billboard: false,
            is_grass: false,
            base_name: "Test".to_string(),
            base_flags: 0,
            material_name: String::new(),
            full_model: String::new(),
            lod_models: [None, None, None, None],
            part_transform: crate::input::identity_part_transform(),
            part_scale: 1.0,
            material_swap: Default::default(),
        }
    }

    fn quad() -> QuadDesc {
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

    fn shape(z_values: [f32; 3]) -> ShapeDesc {
        let mut geometry = LodGeometry::new();
        geometry.vertices = vec![
            [0.0, 0.0, z_values[0]],
            [128.0, 0.0, z_values[1]],
            [0.0, 128.0, z_values[2]],
        ];
        geometry.uvcoords = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        geometry.normals = vec![[0.0, 0.0, 1.0]; 3];
        geometry.triangles = vec![[0, 1, 2]];

        ShapeDesc {
            name: "test".to_string(),
            static_model: "test.nif".to_string(),
            geometry,
            flags: ShapeFlags::empty(),
            textures: Default::default(),
            source_materials: Vec::new(),
            textures_key: String::new(),
            texture_clamp_mode: 3,
            alpha_threshold: 128,
            alpha_flags: 0,
            backlight_power: 0.0,
            grayscale_to_palette_scale: 1.0,
            enable_parent: 0,
            shader_type: ShaderKind::Lighting,
            x: 0.0,
            y: 0.0,
            bounding_box: BBox::empty(),
            segments: Vec::new(),
            uv_scale: [1.0, 1.0],
            uv_offset: [0.0, 0.0],
            ref_flags: 0,
            node_transform: identity4(),
            node_scale: 1.0,
            translation: [0.0; 3],
            rotation: identity3(),
            bto_translation: None,
            bto_scale: None,
        }
    }

    fn grid_shape(static_model: &str, size: usize) -> ShapeDesc {
        let mut s = shape([0.0, 0.0, 0.0]);
        s.static_model = static_model.to_string();
        s.geometry = LodGeometry::new();
        for y in 0..=size {
            for x in 0..=size {
                s.geometry.vertices.push([x as f32, y as f32, 0.0]);
                s.geometry
                    .uvcoords
                    .push([x as f32 / size as f32, y as f32 / size as f32]);
                s.geometry.normals.push([0.0, 0.0, 1.0]);
            }
        }
        let row = size + 1;
        for y in 0..size {
            for x in 0..size {
                let v0 = (y * row + x) as u32;
                let v1 = v0 + 1;
                let v2 = v0 + row as u32;
                let v3 = v2 + 1;
                s.geometry.triangles.push([v0, v1, v3]);
                s.geometry.triangles.push([v0, v3, v2]);
            }
        }
        s
    }

    #[test]
    fn build_bto_preserves_generated_lod_geometry_with_decimation_settings_enabled() {
        let mut settings = LodSettings::fo4_default().objects;
        settings.meshopt_decimate_object_lod = true;
        settings.qem_decimate_full_model_lod = true;
        settings.meshopt_lod_model_ratios = [0.10, 0.10, 0.10, 0.10];
        settings.meshopt_quad_tri_budgets = [8, 8, 8, 8];
        settings.meshopt_target_errors = [0.05, 0.05, 0.05, 0.05];

        let mut q = quad();
        q.quad_level = 16;
        let mut s = grid_shape(r"LOD\Architecture\Foo\Tower01_LOD.nif", 16);
        let before = s.geometry.num_triangles();
        let before_verts = s.geometry.vertices.len();
        s.segments = generate_segments(&q, s.x, s.y, before as u16);

        let (bto, telemetry) = build_bto_with_telemetry(&mut q, vec![s], &settings);

        assert_eq!(bto.len(), 1);
        assert_eq!(bto[0].geometry.num_triangles(), before);
        assert_eq!(bto[0].geometry.vertices.len(), before_verts);
        assert_eq!(
            bto[0]
                .segments
                .iter()
                .map(|segment| segment.num_triangles as usize)
                .sum::<usize>(),
            before
        );
        assert_eq!(telemetry.simplify.triangles_before, before as u64);
        assert_eq!(telemetry.simplify.triangles_after, before as u64);
        assert_eq!(telemetry.simplify.shapes_simplified, 0);
    }

    #[test]
    fn source_bto_builder_preserves_geometry_with_decimation_enabled() {
        let mut settings = LodSettings::fo4_default().objects;
        settings.meshopt_decimate_object_lod = true;
        settings.meshopt_lod_model_ratios = [0.10, 0.10, 0.10, 0.10];
        settings.meshopt_quad_tri_budgets = [8, 8, 8, 8];

        let mut q = quad();
        q.quad_level = 16;
        let mut s = grid_shape(
            r"meshes\terrain\appalachia\objects\appalachia.16.-14.19.bto",
            16,
        );
        s.geometry.vertices.push([999.0, 999.0, 999.0]);
        s.geometry.uvcoords.push([0.0, 0.0]);
        s.geometry.normals.push([0.0, 0.0, 1.0]);
        let before_tris = s.geometry.num_triangles();
        let before_verts = s.geometry.vertices.len();
        s.segments = generate_segments(&q, s.x, s.y, before_tris as u16);

        let (bto, telemetry) = build_source_bto_with_telemetry(&mut q, vec![s], &settings);

        assert_eq!(bto.len(), 1);
        assert_eq!(bto[0].geometry.num_triangles(), before_tris);
        assert_eq!(bto[0].geometry.vertices.len(), before_verts);
        assert_eq!(
            bto[0]
                .segments
                .iter()
                .map(|segment| segment.num_triangles as usize)
                .sum::<usize>(),
            before_tris
        );
        assert_eq!(telemetry.simplify.shapes_simplified, 0);
    }

    #[test]
    fn build_bto_caps_root_children_below_fo4_sentinel_index() {
        let settings = LodSettings::fo4_default().objects;
        let mut q = quad();
        q.quad_level = 32;
        let total = FO4_SAFE_BTO_ROOT_CHILDREN + 16;
        let mut shapes = Vec::with_capacity(total);

        for index in 0..total {
            let mut s = shape([0.0, 0.0, 0.0]);
            s.static_model = format!("shape_{index}.nif");
            s.enable_parent = index as u32 + 1;
            let size = if index == total - 1 { 4096.0 } else { 8.0 };
            s.geometry.vertices[1][0] = size;
            s.geometry.vertices[2][1] = size;
            shapes.push(s);
        }

        let (bto, telemetry) = build_bto_with_telemetry(&mut q, shapes, &settings);

        assert_eq!(bto.len(), FO4_SAFE_BTO_ROOT_CHILDREN);
        assert_eq!(telemetry.simplify.shapes_skipped, 16);
        assert_eq!(
            telemetry.simplify.triangles_after,
            FO4_SAFE_BTO_ROOT_CHILDREN as u64
        );
        assert!(
            bto.iter().any(|shape| shape.radius > 1000.0),
            "the cap should keep high-radius structures before small clutter"
        );
    }

    #[test]
    fn generated_bto_merges_compatible_shapes() {
        let settings = LodSettings::fo4_default().objects;
        let mut q = quad();
        q.quad_level = 16;
        let mut s1 = shape([0.0, 0.0, 0.0]);
        let mut s2 = shape([10.0, 10.0, 0.0]);
        s1.segments = generate_segments(&q, s1.x, s1.y, s1.geometry.num_triangles() as u16);
        s2.segments = generate_segments(&q, s2.x, s2.y, s2.geometry.num_triangles() as u16);

        let (bto, telemetry) = build_bto_with_telemetry(&mut q, vec![s1, s2], &settings);

        assert_eq!(bto.len(), 1);
        assert_eq!(bto[0].geometry.num_triangles(), 2);
        assert_eq!(bto[0].segments.len(), 1);
        assert_eq!(bto[0].segments[0].num_triangles, 2);
        assert_eq!(telemetry.simplify.triangles_after, 2);
    }

    #[test]
    fn source_bto_builder_keeps_compatible_shapes_separate_when_merge_disabled() {
        let settings = LodSettings::fo4_default().objects;
        let mut q = quad();
        q.quad_level = 16;
        let mut s1 = shape([0.0, 0.0, 0.0]);
        let mut s2 = shape([10.0, 10.0, 0.0]);
        s1.segments = generate_segments(&q, s1.x, s1.y, s1.geometry.num_triangles() as u16);
        s2.segments = generate_segments(&q, s2.x, s2.y, s2.geometry.num_triangles() as u16);

        let (bto, telemetry) = build_source_bto_with_telemetry(&mut q, vec![s1, s2], &settings);

        assert_eq!(bto.len(), 2);
        assert_eq!(telemetry.simplify.triangles_after, 2);
    }

    #[test]
    fn source_bto_builder_merges_compatible_shapes_when_enabled() {
        let mut settings = LodSettings::fo4_default().objects;
        settings.fo76_bto_merge_atlassed_shapes = true;
        let mut q = quad();
        let mut s1 = shape([0.0, 0.0, 0.0]);
        let mut s2 = shape([10.0, 10.0, 0.0]);
        s2.x = 5000.0;
        s1.segments = generate_segments(&q, s1.x, s1.y, s1.geometry.num_triangles() as u16);
        s2.segments = generate_segments(&q, s2.x, s2.y, s2.geometry.num_triangles() as u16);

        let (bto, telemetry) = build_source_bto_with_telemetry(&mut q, vec![s1, s2], &settings);

        assert_eq!(bto.len(), 1);
        assert_eq!(bto[0].geometry.num_triangles(), 2);
        assert_eq!(
            bto[0]
                .segments
                .iter()
                .map(|segment| segment.id)
                .collect::<Vec<_>>(),
            vec![0, 4]
        );
        assert_eq!(telemetry.simplify.triangles_after, 2);
    }

    #[test]
    fn source_bto_builder_preserves_explicit_output_transform() {
        let settings = LodSettings::fo4_default().objects;
        let mut q = quad();
        q.x = -10;
        q.y = -1;
        q.quad_level = 4;

        let mut s = shape([10.0, 20.0, 30.0]);
        s.bto_translation = Some([-40960.0, -4096.0, 0.0]);
        s.bto_scale = Some(1.0);
        s.segments = generate_segments(&q, s.x, s.y, s.geometry.num_triangles() as u16);

        let (bto, _) = build_source_bto_with_telemetry(&mut q, vec![s], &settings);

        assert_eq!(bto.len(), 1);
        assert_eq!(bto[0].translation, [-40960.0, -4096.0, 0.0]);
        assert_eq!(bto[0].scale, 1.0);
        assert_eq!(bto[0].multibound.position, [-40896.0, -4032.0, 20.0]);
        assert_eq!(bto[0].multibound.extent, [64.0, 64.0, 10.0]);
    }

    #[test]
    fn source_bto_tile_multibound_inflates_culling_bounds() {
        let mut settings = LodSettings::fo4_default().objects;
        settings.fo76_bto_multibound_mode = Fo76BtoMultiboundMode::Tile;
        let mut q = quad();
        q.x = -10;
        q.y = -1;
        q.quad_level = 4;
        q.quad_offset = 16_384.0;

        let mut s = shape([10.0, 20.0, 30.0]);
        s.bto_translation = Some([-40960.0, -4096.0, 0.0]);
        s.bto_scale = Some(1.0);
        s.segments = generate_segments(&q, s.x, s.y, s.geometry.num_triangles() as u16);

        let (bto, _) = build_source_bto_with_telemetry(&mut q, vec![s], &settings);

        assert_eq!(bto.len(), 1);
        assert_eq!(bto[0].translation, [-40960.0, -4096.0, 0.0]);
        assert_eq!(bto[0].scale, 1.0);
        assert_eq!(bto[0].multibound.position, [-32768.0, 4096.0, 20.0]);
        assert_eq!(bto[0].multibound.extent, [12288.0, 12288.0, 65536.0]);
    }

    #[test]
    fn remove_unseen_faces_drops_fully_buried_triangle() {
        let world = WorldspaceInput::from_cells("W", vec![flat_cell(10.0)]);
        let settings = LodSettings::fo4_default();
        let mut shape = shape([0.0, 0.0, 0.0]);

        let kept = transform_shape_with_world(
            &quad(),
            &stat(),
            &mut shape,
            &AtlasList::new(),
            &settings.objects,
            &world,
        );

        assert!(!kept);
    }

    #[test]
    fn remove_unseen_faces_keeps_triangle_with_visible_vertex() {
        let world = WorldspaceInput::from_cells("W", vec![flat_cell(10.0)]);
        let settings = LodSettings::fo4_default();
        let mut shape = shape([0.0, 0.0, 20.0]);

        let kept = transform_shape_with_world(
            &quad(),
            &stat(),
            &mut shape,
            &AtlasList::new(),
            &settings.objects,
            &world,
        );

        assert!(kept);
        assert_eq!(shape.geometry.num_triangles(), 1);
    }
}
