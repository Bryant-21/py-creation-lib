/// Object/static LOD generation modules.
pub mod fo76_bto;
pub mod geometry;
pub mod hybrid;
pub mod instance_array_index;
pub mod instance_layers;
pub mod instance_transform;
pub mod instanced;
pub mod lodtex_index;
pub mod meshopt_decimator;
pub mod object_lod;
pub mod parse_nif;
pub mod qem;
pub mod static_desc;

use crate::atlas::AtlasResult;
use crate::descriptors::QuadDesc;
use crate::input::StaticDesc;
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
/// Per-ref errors are logged to stderr and skipped. If no shapes survive, no
/// `.bto` is written.
///
/// Shapes are processed in ref-index order for determinism; the C# `Parallel.For`
/// inserts into the shared shape list in nondeterministic order.
pub fn generate_quad(
    quad: &QuadDesc,
    ctx: &QuadCtx<'_>,
    atlas: &AtlasResult,
    instancing: Option<&RecordInstancing>,
) -> anyhow::Result<QuadOutputs> {
    let Some(instancing) = instancing else {
        let all_shapes = collect_quad_shapes(quad, ctx, atlas);
        return write_quad_shapes(quad, ctx, all_shapes);
    };
    let refs: Vec<&StaticDesc> = quad.static_refs(ctx.world).collect();
    let level_index = lod_model_index(ctx.level);
    let min_count = ctx.settings.objects.fo76_bto_instanced_min_count as usize;
    let (record_groups, mut merged, _) = instanced::group_record_instances(&refs, level_index, min_count);
    let mut groups = Vec::new();
    for group in record_groups {
        let representative = refs[group.refs[0]];
        match record_group_shape(representative, &group.model, level_index, ctx, atlas, instancing) {
            Some((object, shape, layer)) => groups.push(instanced::InstanceGroup {
                object,
                model: group.model,
                shape,
                layer,
                instances: group.instances,
                ids: group.ids,
            }),
            None => merged.extend(group.refs),
        }
    }
    merged.sort_unstable();
    let all_shapes = collect_shapes(merged.iter().map(|&i| refs[i]), quad, ctx, atlas);
    let mut outputs = write_quad_shapes(quad, ctx, all_shapes)?;
    if !groups.is_empty() {
        let written = outputs.object_lod.as_ref().and_then(|t| t.bto_path.clone());
        let (path, _, created) =
            instanced::append_instance_groups(ctx, quad, written, &groups, Some(&instancing.layers))?;
        if created {
            outputs.meshes.push(path);
        }
    }
    Ok(outputs)
}

/// World-wide state for records-path instancing (`objects.instance_records`).
pub struct RecordInstancing {
    object_ids: instanced::ObjectIds<String>,
    atlas_diffuse: String,
    layers: instance_layers::InstanceLayers,
}

impl RecordInstancing {
    pub fn new(world: &str, atlas: &AtlasResult) -> Self {
        Self {
            object_ids: instanced::ObjectIds::new(world),
            atlas_diffuse: crate::atlas::atlas::data_relative_path(&atlas.diffuse),
            layers: Default::default(),
        }
    }

    pub fn layers(&self) -> &instance_layers::InstanceLayers {
        &self.layers
    }

    /// The world's texture arrays, in the formats of the records atlas, whose diffuse stores plain texels.
    pub fn finish(
        &self,
        world: &str,
        settings: &crate::settings::LodSettings,
        paths: &crate::progress::LodPaths,
    ) -> anyhow::Result<crate::progress::LodGenStats> {
        let objects = &settings.objects;
        self.layers.finish(
            &paths.output_dir,
            world,
            &instance_layers::ArrayFormats::new(objects, false),
            settings.global.workers,
            |tile| {
                let (dirs, size) = (&paths.data_dirs, instance_array_index::SLICE_SIZE);
                crate::atlas::atlas::load_atlas_slice(dirs, &tile.key, objects.atlas_size, objects.max_tile_size, size)
            },
        )
    }
}

/// A group's LOD model in model space, remapped onto the world atlas, with its object id and layer. None keeps the
/// group merged: an id collision, a part off the atlas or alpha-tested, or a model that fails to parse.
fn record_group_shape(
    representative: &StaticDesc,
    model: &str,
    level_index: usize,
    ctx: &QuadCtx<'_>,
    atlas: &AtlasResult,
    instancing: &RecordInstancing,
) -> Option<(u32, object_lod::BtoShape, u32)> {
    use crate::objects::static_desc::ShapeFlags;
    let object = instancing.object_ids.claim_record(model, &representative.material_swap)?;
    let neutral = StaticDesc {
        pos: [0.0; 3],
        rot: [0.0; 3],
        scale: 1.0,
        part_transform: crate::input::identity_part_transform(),
        part_scale: 1.0,
        ..representative.clone()
    };
    let shapes = parse_nif::parse_nif_for_object_lod(&neutral, level_index, ctx).ok()?;
    // Level 1 at the origin: the transform leaves model space as it is and only applies the atlas remap.
    let model_space = QuadDesc {
        z_order: 0,
        x: 0,
        y: 0,
        quad_level: 1,
        quad_index: 0,
        quad_offset: 4096.0,
        static_indices: Vec::new(),
        statics: Vec::new(),
        out_values: Default::default(),
    };
    let mut parts = Vec::new();
    let mut texture_set = None;
    for mut shape in shapes {
        let tile = instance_layers::AtlasTile::lookup(&atlas.list, &shape.textures[..3], shape.alpha_threshold);
        if !object_lod::transform_shape_with_world(
            &model_space,
            &neutral,
            &mut shape,
            &atlas.list,
            &ctx.settings.objects,
            ctx.world,
        ) {
            continue;
        }
        texture_set.get_or_insert_with(|| shape.textures.clone());
        parts.push(instanced::InstancePart {
            alpha: shape.flags.contains(ShapeFlags::IS_ALPHA),
            diffuse: shape.textures[0].clone(),
            geometry: shape.geometry,
            tile,
        });
    }
    let tile = instance_layers::single_tile(parts.iter().map(|part| part.tile.clone()));
    let shape = instanced::instance_shape(parts, &instancing.atlas_diffuse, texture_set?)?;
    Some((object, shape, instancing.layers.assign(tile.as_ref())))
}

/// The `lod_models` slot for a level: 0=L4, 1=L8, 2=L16, 3=L32.
fn lod_model_index(level: i32) -> usize {
    match level {
        8 => 1,
        16 => 2,
        32 => 3,
        _ => 0,
    }
}

/// Collect transformed object-LOD shapes for one quad without writing the `.bto`.
pub fn collect_quad_shapes(
    quad: &QuadDesc,
    ctx: &QuadCtx<'_>,
    atlas: &AtlasResult,
) -> Vec<ShapeDesc> {
    collect_shapes(quad.static_refs(ctx.world), quad, ctx, atlas)
}

fn collect_shapes<'a>(
    refs: impl Iterator<Item = &'a StaticDesc>,
    quad: &QuadDesc,
    ctx: &QuadCtx<'_>,
    atlas: &AtlasResult,
) -> Vec<ShapeDesc> {
    use crate::objects::object_lod::transform_shape_with_world;
    use crate::objects::parse_nif::parse_nif_for_object_lod;
    // port: DoLOD:3044 — skip refs with no model at this level.
    let level_index = lod_model_index(ctx.level);

    let mut all_shapes = Vec::new();

    for stat in refs {
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
    use crate::output::bto::write_bto_with_layout_timed;
    use crate::settings::Fo76BtoNodeLayout;

    // port: DoLOD:3088 — if no shapes survived, write nothing
    if all_shapes.is_empty() {
        return Ok(QuadOutputs::default());
    }

    // port: CreateLODNodesFO4:2455 — build the BtoShape list
    let shape_started = std::time::Instant::now();
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
    telemetry.shape_build_secs = shape_started.elapsed().as_secs_f64();

    if bto_shapes.is_empty() {
        return Ok(QuadOutputs::default());
    }
    telemetry.output_shape_count = bto_shapes.len() as u64;

    // Determine season tag from settings (empty string = no season).
    let season = ctx.settings.global.season.as_deref().unwrap_or("");

    // Compute the output path via naming::bto — port: DoLOD writes to the Objects subfolder.
    let bto_rel = crate::naming::bto(&ctx.world.editor_id, ctx.level, quad.x, quad.y, season);
    let bto_path = ctx.paths.output_dir.join(bto_rel.replace('\\', "/"));
    let layout = if source_mode != SourceBtoWriteMode::Generated {
        ctx.settings.objects.fo76_bto_node_layout
    } else {
        Fo76BtoNodeLayout::Fo4PerShape
    };
    telemetry.write_report = write_bto_with_layout_timed(&bto_path, &bto_shapes, layout)?;
    telemetry.bto_path = Some(bto_path.clone());
    telemetry.bto_bytes = telemetry.write_report.bytes;

    Ok(QuadOutputs {
        meshes: vec![bto_path],
        textures: vec![],
        object_lod: Some(telemetry),
    })
}
