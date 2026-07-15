/// Object/static LOD generation modules.
pub mod fo76_bto;
pub mod geometry;
pub mod hybrid;
pub mod meshopt_decimator;
pub mod object_lod;
pub mod parse_nif;
pub mod qem;
pub mod static_desc;

use crate::atlas::AtlasResult;
use crate::descriptors::QuadDesc;
use crate::objects::static_desc::ShapeDesc;
use crate::progress::{QuadCtx, QuadOutputs};

/// Generate one `.bto` quad from the filtered static refs in `quad`.
///
/// Port: `DoLOD` FO4 object path (LODApp.cs:2919-3138).
///
/// For each ref in `quad` that has a LOD model at `ctx.level`, loads the
/// geometry via `parse_nif`, transforms into quad space via `transform_shape`,
/// then merges and writes a single `.bto` via `output::write_bto`.
///
/// Per-ref errors are logged to stderr and skipped (partial-failure isolation —
/// plan spec §6). If no shapes survive, no `.bto` is written.
///
/// Shapes are processed in ref-index order for determinism (mirrors C# lock-ordered
/// insertion into the shared shape list; noted deviation: C# `Parallel.For` has
/// nondeterministic insertion order but we pick stable ref-index order — see report).
pub fn generate_quad(
    quad: &QuadDesc,
    ctx: &QuadCtx<'_>,
    atlas: &AtlasResult,
) -> anyhow::Result<QuadOutputs> {
    let all_shapes = collect_quad_shapes(quad, ctx, atlas);
    write_quad_shapes(quad, ctx, all_shapes)
}

/// Collect transformed object-LOD shapes for one quad without writing the `.bto`.
pub fn collect_quad_shapes(
    quad: &QuadDesc,
    ctx: &QuadCtx<'_>,
    atlas: &AtlasResult,
) -> Vec<ShapeDesc> {
    use crate::objects::object_lod::transform_shape_with_world;
    use crate::objects::parse_nif::parse_nif_for_object_lod;
    // quad_index maps to the lod_models slot index: 0=L4, 1=L8, 2=L16, 3=L32.
    // port: DoLOD:3044 — skip refs with no model at this level.
    let level_index: usize = match ctx.level {
        4 => 0,
        8 => 1,
        16 => 2,
        32 => 3,
        _ => 0,
    };

    let mut all_shapes = Vec::new();

    for stat in quad.static_refs(ctx.world) {
        // port: DoLOD:3044 — skip if no model for this level
        let Some(model) = stat.lod_models.get(level_index).and_then(|m| m.as_ref()) else {
            continue;
        };

        if model.to_lowercase().ends_with(".dds") && crate::trees::is_tree(stat) {
            let fd = crate::trees::tree3d::load_flat_desc(model, ctx);
            for mut shape in crate::trees::tree3d::build_flat_trunk(&fd, model) {
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
            continue;
        }

        // port: DoLOD:3055 — parse the NIF; on error skip+warn (partial-failure isolation)
        let shapes = match parse_nif_for_object_lod(stat, level_index, ctx) {
            Ok(s) => s,
            Err(e) => {
                // partial-failure: skip this ref and log to stderr
                eprintln!("[lodgen] generate_quad: skipping ref {} - {e}", stat.ref_id);
                continue;
            }
        };

        // port: DoLOD:3070 — transform each shape; drop shapes where transform returns false
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

    all_shapes
}

/// Build and write a FO4-native BTO from already-transformed quad-local shapes.
pub fn write_quad_shapes(
    quad: &QuadDesc,
    ctx: &QuadCtx<'_>,
    all_shapes: Vec<ShapeDesc>,
) -> anyhow::Result<QuadOutputs> {
    write_quad_shapes_impl(quad, ctx, all_shapes, SourceBtoWriteMode::Generated)
}

/// Build and write a BTO from source-BTO geometry without generated-LOD cleanup.
pub fn write_source_bto_shapes(
    quad: &QuadDesc,
    ctx: &QuadCtx<'_>,
    all_shapes: Vec<ShapeDesc>,
) -> anyhow::Result<QuadOutputs> {
    write_quad_shapes_impl(quad, ctx, all_shapes, SourceBtoWriteMode::Raw)
}

/// Build and write atlas-remapped source-BTO geometry without geometry optimization.
pub fn write_atlassed_source_bto_shapes(
    quad: &QuadDesc,
    ctx: &QuadCtx<'_>,
    all_shapes: Vec<ShapeDesc>,
) -> anyhow::Result<QuadOutputs> {
    write_quad_shapes_impl(quad, ctx, all_shapes, SourceBtoWriteMode::Atlassed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceBtoWriteMode {
    Generated,
    Raw,
    Atlassed,
}

fn write_quad_shapes_impl(
    quad: &QuadDesc,
    ctx: &QuadCtx<'_>,
    all_shapes: Vec<ShapeDesc>,
    source_mode: SourceBtoWriteMode,
) -> anyhow::Result<QuadOutputs> {
    use crate::objects::object_lod::{
        build_atlassed_source_bto_with_telemetry, build_bto_with_telemetry,
        build_source_bto_with_telemetry,
    };
    use crate::output::bto::{write_bto, write_bto_with_layout};

    // port: DoLOD:3088 — if no shapes survived, write nothing
    if all_shapes.is_empty() {
        return Ok(QuadOutputs::default());
    }

    // port: CreateLODNodesFO4:2455 — build the BtoShape list
    let mut mut_quad = quad.clone();
    let (bto_shapes, mut telemetry) = match source_mode {
        SourceBtoWriteMode::Generated => {
            build_bto_with_telemetry(&mut mut_quad, all_shapes, &ctx.settings.objects)
        }
        SourceBtoWriteMode::Raw => {
            build_source_bto_with_telemetry(&mut mut_quad, all_shapes, &ctx.settings.objects)
        }
        SourceBtoWriteMode::Atlassed => build_atlassed_source_bto_with_telemetry(
            &mut mut_quad,
            all_shapes,
            &ctx.settings.objects,
        ),
    };

    if bto_shapes.is_empty() {
        return Ok(QuadOutputs::default());
    }
    telemetry.output_shape_count = bto_shapes.len() as u64;

    // Determine season tag from settings (empty string = no season).
    let season = ctx.settings.global.season.as_deref().unwrap_or("");

    // Compute the output path via naming::bto — port: DoLOD writes to the Objects subfolder.
    let bto_rel = crate::naming::bto(&ctx.world.editor_id, ctx.level, quad.x, quad.y, season);
    let bto_path = ctx.paths.output_dir.join(bto_rel.replace('\\', "/"));
    if let Some(parent) = bto_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if source_mode != SourceBtoWriteMode::Generated {
        write_bto_with_layout(
            &bto_path,
            &bto_shapes,
            ctx.settings.objects.fo76_bto_node_layout,
        )?;
    } else {
        write_bto(&bto_path, &bto_shapes)?;
    }
    telemetry.bto_path = Some(bto_path.clone());
    telemetry.bto_bytes = std::fs::metadata(&bto_path).map(|m| m.len()).unwrap_or(0);

    Ok(QuadOutputs {
        meshes: vec![bto_path],
        textures: vec![],
        object_lod: Some(telemetry),
    })
}
