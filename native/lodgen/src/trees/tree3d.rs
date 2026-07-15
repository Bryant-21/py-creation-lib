/// 3D tree LOD path — FlatDesc, dimension parser, build_flat_trunk, and generate_quad.
///
/// Port sources:
/// - `FlatDesc`                   → FlatDesc.cs
/// - `parse_billboard_dimensions` → Utils.GetBillboardDimensions (Utils.cs:536-707)
/// - `build_flat_trunk`           → LODApp.cs ParseNif billboard branch :1442-1505
/// - `generate_quad`              → DoLOD FO4 object path via objects::*
use crate::atlas::AtlasResult;
use crate::descriptors::QuadDesc;
use crate::input::StaticDesc;
use crate::objects::static_desc::ShapeDesc;
use crate::progress::{QuadCtx, QuadOutputs};

// ---------------------------------------------------------------------------
// FlatDesc — port: FlatDesc.cs
// ---------------------------------------------------------------------------

/// Billboard descriptor for a tree species whose LOD model is a `.dds`.
///
/// Port: `FlatDesc` (FlatDesc.cs).
#[derive(Clone, Debug, Default)]
pub struct FlatDesc {
    /// Half-width of the billboard in game units (Utils.cs:617 — WIDTH halved).
    pub width: f32,
    /// Half-depth of the billboard in game units (Utils.cs:618 — initially == width;
    /// overridden by DEPTH line, also halved per Utils.cs:626).
    pub depth: f32,
    /// Full height of the billboard in game units.
    pub height: f32,
    /// X shift of the billboard origin.
    pub shift_x: f32,
    /// Y shift of the billboard origin.
    pub shift_y: f32,
    /// Z shift of the billboard origin (lift off the ground).
    pub shift_z: f32,
    /// Scale multiplier.
    pub scale: f32,
    /// Whether the billboard is "complex" (multi-plane, COMPLEX=true in the .txt).
    pub complex: bool,
    /// Dimensions per plane: `[[w, h, z_adjust], ...]`.
    ///
    /// Seeded as `[[width, height, 0], [depth, height, 0]]` from Utils.cs:538-542;
    /// updated by WIDTH/DEPTH/HEIGHT/ADJUST lines (:619-646).
    pub dimensions: Vec<[f32; 3]>,
}

// ---------------------------------------------------------------------------
// parse_billboard_dimensions — port: Utils.GetBillboardDimensions (Utils.cs:536-707)
// ---------------------------------------------------------------------------

/// Parse a billboard `.txt` sidecar into `fd`.
///
/// The caller supplies the file contents as `txt: &str`; the file read / BSA lookup
/// in the C# source is replaced by having the caller resolve the path via `LodPaths`.
///
/// Seeds `fd.dimensions` = `[[width, height, 0], [depth, height, 0]]` first, then
/// applies each KEY=VALUE line. Numeric values strip non-`[\d.-]` characters before
/// parsing (Utils.cs:598). COMPLEX is the exception and is passed through as-is.
///
/// port: Utils.cs:536-707
pub fn parse_billboard_dimensions(txt: &str, fd: &mut FlatDesc) {
    // Utils.cs:538-542 — seed dimensions
    fd.dimensions = vec![[fd.width, fd.height, 0.0], [fd.depth, fd.height, 0.0]];

    let known = [
        "WIDTH", "DEPTH", "HEIGHT", "SHIFTX", "SHIFTY", "SHIFTZ", "SCALE", "ADJUST", "COMPLEX",
    ];

    for raw_line in txt.lines() {
        let parts: Vec<&str> = raw_line.splitn(2, '=').collect();
        if parts.len() != 2 {
            continue;
        }
        let setting = parts[0].trim().to_uppercase();
        let raw_val = parts[1];

        // Check if setting contains any known keyword
        let is_known = known.iter().any(|&k| setting.contains(k));
        if !is_known {
            continue;
        }

        // COMPLEX is not numeric-stripped (Utils.cs:596)
        let text = if setting.contains("COMPLEX") {
            raw_val.trim().to_string()
        } else {
            // Strip non-numeric chars (Utils.cs:598): keep digits, '.', '-'
            raw_val
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                .collect()
        };

        if text.is_empty() {
            continue;
        }

        // Handle indexed variants: WIDTH_N, HEIGHT_N, ADJUST_N (Utils.cs:687-706)
        if let Some(sep_pos) = setting.rfind('_') {
            let base = &setting[..sep_pos];
            let idx_str = &setting[sep_pos + 1..];
            if let Ok(n) = idx_str.parse::<usize>() {
                // 1-based index → 0-based
                let idx = n.saturating_sub(1);
                // Grow dimensions if needed
                while fd.dimensions.len() <= idx {
                    fd.dimensions.push([1.0, 1.0, 0.0]);
                }
                match base {
                    "WIDTH" => {
                        if let Ok(v) = text.parse::<f32>() {
                            fd.dimensions[idx][0] = v / 2.0;
                        }
                    }
                    "HEIGHT" => {
                        if let Ok(v) = text.parse::<f32>() {
                            fd.dimensions[idx][1] = v;
                        }
                    }
                    "ADJUST" => {
                        if let Ok(v) = text.parse::<f32>() {
                            fd.dimensions[idx][2] = v;
                        }
                    }
                    _ => {}
                }
                continue;
            }
        }

        // Plain keyword dispatch (Utils.cs:606-686)
        match setting.as_str() {
            "WIDTH" => {
                if let Ok(v) = text.parse::<f32>() {
                    // Utils.cs:617-620 — halved; depth also set; both dimensions[*].x updated
                    fd.width = v / 2.0;
                    fd.depth = fd.width;
                    if fd.dimensions.len() > 0 {
                        fd.dimensions[0][0] = fd.width;
                    }
                    if fd.dimensions.len() > 1 {
                        fd.dimensions[1][0] = fd.width;
                    }
                }
            }
            "DEPTH" => {
                if let Ok(v) = text.parse::<f32>() {
                    // Utils.cs:626-627 — halved; only dimensions[1].x updated
                    fd.depth = v / 2.0;
                    if fd.dimensions.len() > 1 {
                        fd.dimensions[1][0] = fd.depth;
                    }
                }
            }
            "HEIGHT" => {
                if let Ok(v) = text.parse::<f32>() {
                    // Utils.cs:643-646 — both dimensions[*].y updated
                    fd.height = v;
                    if fd.dimensions.len() > 0 {
                        fd.dimensions[0][1] = fd.height;
                    }
                    if fd.dimensions.len() > 1 {
                        fd.dimensions[1][1] = fd.height;
                    }
                }
            }
            "SHIFTX" => {
                if let Ok(v) = text.parse::<f32>() {
                    fd.shift_x = v;
                }
            }
            "SHIFTY" => {
                if let Ok(v) = text.parse::<f32>() {
                    fd.shift_y = v;
                }
            }
            "SHIFTZ" => {
                if let Ok(v) = text.parse::<f32>() {
                    fd.shift_z = v;
                }
            }
            "SCALE" => {
                if let Ok(v) = text.parse::<f32>() {
                    fd.scale = v;
                }
            }
            "COMPLEX" => {
                // Utils.cs:675-677 — not numeric-stripped; parse as bool
                if let Ok(b) = text.to_lowercase().parse::<bool>() {
                    fd.complex = b;
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// build_flat_trunk — port: LODApp.cs ParseNif billboard branch :1442-1505
// ---------------------------------------------------------------------------

/// Build the FlatTrunk billboard geometry (2 crossed quads) for a tree whose LOD model is a `.dds`.
///
/// Returns two `ShapeDesc`s — one per plane of the cross. Each has:
/// - 4 vertices, 2 triangles
/// - `IS_BILLBOARD` flag set
/// - `billboard_diffuse` in `textures[0]`
/// - White vertex colors (grass brightness branches deferred — see port note below)
///
/// Port: LODApp.cs ParseNif billboard branch :1442-1505 (the `for i in 0..2` two-cross loop).
///
/// DEVIATION/DEFERRED: vertex-color brightness (`:1458-1502`) — `colorVariance`,
/// `flatDesc.grassBrightnessTop/Bottom`, `vertexColorsMuliplier` — requires
/// grass-specific fields and a `Random` source. For FO4 3D-tree default (non-grass)
/// the vertex color path sets all to white when `num` (vertexColor) is in [0,1].
/// Grass billboard brightness is a later sub-feature. Flat white vertex colors are
/// correct for the non-grass FO4 tree path.
///
/// The 90° Z rotation (`:1449`) on the 2nd quad is applied to the vertices directly
/// (cos90=0, sin90=1) rather than via a NiTriShape rotation matrix, since our
/// ShapeDesc is geometry-only (no NIF transform hierarchy at this level).
pub fn build_flat_trunk(
    fd: &FlatDesc,
    billboard_diffuse: &str,
) -> Vec<crate::objects::static_desc::ShapeDesc> {
    use crate::descriptors::BBox;
    use crate::objects::geometry::LodGeometry;
    use crate::objects::parse_nif::mat4_identity;
    use crate::objects::static_desc::{ShaderKind, ShapeDesc, ShapeFlags};

    // port: LODApp.cs:1421-1422
    // array[i]  = [shiftX, shiftY]  — center XY offset per quad plane
    // array2[i] = [shiftY, shiftX]  — transposed Y offset per quad plane
    let array = [fd.shift_x, fd.shift_y];
    let array2 = [fd.shift_y, fd.shift_x];

    let mut shapes = Vec::with_capacity(2);

    for i in 0..2 {
        // Ensure we have dimension data for this plane
        let dim = if i < fd.dimensions.len() {
            fd.dimensions[i]
        } else {
            [fd.width, fd.height, 0.0]
        };
        // dim[0] = half-width (x), dim[1] = height (y), dim[2] = z-adjust (center x offset)
        let half_w = dim[0];
        let h = dim[1];
        let z_adj = dim[2];

        // Center offsets for this quad plane
        let cx = array[i] + z_adj; // LODApp.cs:1454: array[i] + flatDesc.dimensions[i].z
        let cy = array2[i]; // LODApp.cs:1454: array2[i]

        // 4 vertices per quad — LODApp.cs:1454-1457:
        //   v0 = (cx - half_w, cy, shiftZ)
        //   v1 = (cx + half_w, cy, shiftZ)
        //   v2 = (cx + half_w, cy, shiftZ + h)
        //   v3 = (cx - half_w, cy, shiftZ + h)
        //
        // For i=1 the 2nd quad is rotated 90° around Z (LODApp.cs:1449):
        //   Rz(90°): (x,y) → (-y, x)
        // Applied to the local (cx ± half_w, cy) coordinates.
        let verts: Vec<[f32; 3]> = if i == 0 {
            // Quad 0: no rotation
            vec![
                [cx - half_w, cy, fd.shift_z],
                [cx + half_w, cy, fd.shift_z],
                [cx + half_w, cy, fd.shift_z + h],
                [cx - half_w, cy, fd.shift_z + h],
            ]
        } else {
            // Quad 1: rotate 90° around Z
            // Original corners in local XY: (cx±half_w, cy)
            // After Rz(90°): (x,y) → (-y, x)
            vec![
                [-cy, cx - half_w, fd.shift_z],
                [-cy, cx + half_w, fd.shift_z],
                [-cy, cx + half_w, fd.shift_z + h],
                [-cy, cx - half_w, fd.shift_z + h],
            ]
        };

        // UV coords — LODApp.cs:1504-1507:
        //   (1,1), (0,1), (0,0), (1,0)
        let uvs = vec![[1.0_f32, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]];

        // Triangles — LODApp.cs:1508-1509: (0,1,2) and (2,3,0)
        let tris = vec![[0u32, 1, 2], [2, 3, 0]];

        // White vertex colors (flat white — non-grass FO4 tree path, LODApp.cs:1461-1464)
        // Deferred: grass brightness / colorVariance (LODApp.cs:1458-1502)
        let vcolors = vec![
            [1.0_f32, 1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
        ];

        let mut geom = LodGeometry::new();
        geom.vertices = verts;
        geom.uvcoords = uvs;
        geom.triangles = tris;
        geom.vertex_colors = vcolors;
        geom.update_bbox();

        // Textures: billboard diffuse in slot 0 (LODApp.cs sets the billboard texture on the shape)
        let mut textures: [String; 10] = Default::default();
        textures[0] = billboard_diffuse.to_string();

        // Flags: IS_BILLBOARD set — LODApp.cs:1391 isBillboard=true
        let flags = ShapeFlags::IS_BILLBOARD | ShapeFlags::HAS_VERTEX_COLOR;

        shapes.push(ShapeDesc {
            name: format!("Internal Billboard {i}"),
            static_model: billboard_diffuse.to_string(),
            geometry: geom,
            flags,
            textures,
            source_materials: Vec::new(),
            textures_key: String::new(),
            texture_clamp_mode: 3, // WRAP_S_WRAP_T — typical for billboards
            alpha_threshold: 0,
            alpha_flags: 4844, // standard alpha-test flags (LODApp default)
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
            node_transform: mat4_identity(),
            // port: LODApp.cs:1451 niTriShape.SetScale(flatDesc.scale) — transform_shape
            // applies node_scale to every vertex (matrix7 step), scaling the quad.
            node_scale: fd.scale,
            translation: [0.0; 3],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            bto_translation: None,
            bto_scale: None,
        });
    }

    shapes
}

// ---------------------------------------------------------------------------
// load_flat_desc — resolve the .txt sidecar and parse dimensions
// ---------------------------------------------------------------------------

/// Try to resolve and parse the `.txt` billboard sidecar for a `.dds` LOD model.
///
/// Replaces the file-path portion of the `.dds` model with `.txt`, then searches
/// `ctx.paths.data_dirs` for the file. If not found, returns a default `FlatDesc`
/// with unit dimensions so the billboard quad is still emitted.
pub(crate) fn load_flat_desc(dds_model: &str, ctx: &QuadCtx<'_>) -> FlatDesc {
    // Derive the .txt path: replace .dds extension with .txt
    let txt_model = if let Some(stem) = dds_model.to_lowercase().strip_suffix(".dds") {
        format!("{stem}.txt")
    } else {
        format!("{dds_model}.txt")
    };

    // Seed FlatDesc from defaults (will be overridden by the .txt if found)
    let mut fd = FlatDesc {
        width: 1.0,
        depth: 1.0,
        height: 1.0,
        scale: 1.0,
        ..Default::default()
    };

    // Search data_dirs for the .txt sidecar
    for dir in &ctx.paths.data_dirs {
        let candidate = dir.join(txt_model.replace('\\', "/"));
        if let Ok(txt) = std::fs::read_to_string(&candidate) {
            parse_billboard_dimensions(&txt, &mut fd);
            return fd;
        }
        // Also try with the original (possibly backslash) path form
        let candidate2 = dir.join(&txt_model);
        if let Ok(txt) = std::fs::read_to_string(&candidate2) {
            parse_billboard_dimensions(&txt, &mut fd);
            return fd;
        }
    }

    // Not found — return the default FlatDesc (unit dimensions)
    fd
}

// ---------------------------------------------------------------------------
// generate_quad — 3D-tree per-quad generator (FO4 default, trees_3d=true)
// ---------------------------------------------------------------------------

/// 3D-tree per-quad generator (default `trees_3d=true`). Returns the same `.bto`
/// the object path writes. Trees with a 3D NIF LOD model flow through the object path
/// (`parse_nif` → `transform_shape` → `build_bto`). Trees whose LOD model is a
/// billboard `.dds` build FlatTrunk shapes (`build_flat_trunk`) and fold them into
/// the same `.bto`.
///
/// Port: DoLOD FO4 object path (LODApp.cs) + ParseNif billboard branch :1389-1505.
/// Deviation from C# (noted): refs are sorted by `ref_id` before processing for
/// determinism (C# `Parallel.For` has nondeterministic insertion order).
///
/// `atlas` is the REAL object `AtlasResult` (the driver `driver::run_trees`
/// now threads it in from `build_object_lod`), so tree UVs ARE remapped onto the
/// shared object atlas via `transform_shape`.
pub fn generate_quad(
    quad: &QuadDesc,
    ctx: &QuadCtx<'_>,
    atlas: &AtlasResult,
    trees: &[&StaticDesc],
) -> anyhow::Result<QuadOutputs> {
    use crate::objects::object_lod::build_bto;
    use crate::output::bto::write_bto;

    let all_shapes = collect_quad_shapes(quad, ctx, atlas, trees);
    if all_shapes.is_empty() {
        return Ok(QuadOutputs::default());
    }

    // port: CreateLODNodesFO4:2455 — build the BtoShape list
    let mut mut_quad = quad.clone();
    let bto_shapes = build_bto(&mut mut_quad, all_shapes, &ctx.settings.objects);

    if bto_shapes.is_empty() {
        return Ok(QuadOutputs::default());
    }

    let season = ctx.settings.global.season.as_deref().unwrap_or("");
    let bto_rel = crate::naming::bto(&ctx.world.editor_id, ctx.level, quad.x, quad.y, season);
    let bto_path = ctx.paths.output_dir.join(bto_rel.replace('\\', "/"));
    if let Some(parent) = bto_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    write_bto(&bto_path, &bto_shapes)?;

    Ok(QuadOutputs {
        meshes: vec![bto_path],
        textures: vec![],
        object_lod: None,
    })
}

pub fn collect_quad_shapes(
    quad: &QuadDesc,
    ctx: &QuadCtx<'_>,
    atlas: &AtlasResult,
    trees: &[&StaticDesc],
) -> Vec<ShapeDesc> {
    use crate::objects::object_lod::transform_shape_with_world;
    use crate::objects::parse_nif::parse_nif;

    // Level index: 0=L4, 1=L8, 2=L16, 3=L32 — mirrors objects::generate_quad
    let level_index: usize = match ctx.level {
        4 => 0,
        8 => 1,
        16 => 2,
        32 => 3,
        _ => 0,
    };

    // Sort by ref_id for determinism (deviation: C# Parallel.For is non-deterministic)
    let mut sorted_trees: Vec<&StaticDesc> = trees.to_vec();
    sorted_trees.sort_by(|a, b| a.ref_id.cmp(&b.ref_id));

    let mut all_shapes = Vec::new();

    for stat in &sorted_trees {
        let model_opt = stat.lod_models.get(level_index).and_then(|m| m.as_ref());
        let model = match model_opt {
            Some(m) => m,
            None => continue,
        };

        if model.to_lowercase().ends_with(".dds") {
            // Billboard .dds path — build FlatTrunk crossed quads
            // port: LODApp.cs:1389-1505 (the isBillboard branch)
            let fd = load_flat_desc(model, ctx);
            let flat_shapes = build_flat_trunk(&fd, model);
            for mut shape in flat_shapes {
                // Skip RemoveDuplicate for billboard/FlatTrunk shapes
                // port: LODApp.cs:1348-1355 — billboard refs skip RemoveDuplicate
                if transform_shape_with_world(
                    quad,
                    stat,
                    &mut shape,
                    &atlas.list,
                    &ctx.settings.objects,
                    ctx.world,
                ) {
                    all_shapes.push(shape);
                }
            }
        } else {
            // 3D NIF model — same path as objects::generate_quad
            // port: LODApp.cs:3055-3070
            let shapes = match parse_nif(stat, level_index, ctx) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!(
                        "[lodgen] tree3d::generate_quad: skipping ref {} - {e}",
                        stat.ref_id
                    );
                    continue;
                }
            };
            for mut shape in shapes {
                if transform_shape_with_world(
                    quad,
                    stat,
                    &mut shape,
                    &atlas.list,
                    &ctx.settings.objects,
                    ctx.world,
                ) {
                    all_shapes.push(shape);
                }
            }
        }
    }

    all_shapes
}
