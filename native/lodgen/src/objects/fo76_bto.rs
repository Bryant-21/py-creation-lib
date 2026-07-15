use anyhow::Context;
use nif_core_native::model::{NifBlock, NifFile, NifValue};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::atlas::{AtlasMapRow, AtlasRect, AtlasResult};
use crate::billboards::BillboardManifest;
use crate::descriptors::{BBox, OutDesc, QuadDesc};
use crate::game::Game;
use crate::input::{StaticDesc, WorldspaceInput, identity_part_transform};
use crate::objects::geometry::LodGeometry;
use crate::objects::object_lod::{generate_segments, transform_shape_with_world};
use crate::objects::static_desc::{ShaderKind, ShapeDesc, ShapeFlags, atlas_build_key};
use crate::progress::{LodGenStats, LodPaths, Progress, QuadCtx};
use crate::settings::{LodSettings, ObjectSource};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceBtoTile {
    pub world: String,
    pub level: i32,
    pub x: i32,
    pub y: i32,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceId {
    pub dir: u32,
    pub file: u32,
    pub ext: u32,
}

struct TextureSink<'a> {
    source_root: &'a Path,
    output_dir: &'a Path,
    world: &'a str,
    copied: &'a Mutex<HashSet<PathBuf>>,
    dds_written: u32,
}

impl<'a> TextureSink<'a> {
    fn new(
        source_root: &'a Path,
        output_dir: &'a Path,
        world: &'a str,
        copied: &'a Mutex<HashSet<PathBuf>>,
    ) -> Self {
        Self {
            source_root,
            output_dir,
            world,
            copied,
            dds_written: 0,
        }
    }

    fn rewrite_slot(&mut self, texture: &str, specular: bool) -> String {
        rewrite_texture_path(
            self.source_root,
            self.output_dir,
            self.world,
            texture,
            specular,
            self.copied,
            &mut self.dds_written,
        )
    }
}

struct ResourceResolver {
    by_id: HashMap<ResourceId, String>,
}

impl ResourceResolver {
    fn build(source_root: &Path) -> anyhow::Result<Self> {
        let meshes =
            find_child_ci(source_root, "Meshes").unwrap_or_else(|| source_root.join("Meshes"));
        let mut by_id = HashMap::new();
        for lod_root in collect_lod_dirs(&meshes) {
            for path in collect_files_recursive(&lod_root, "nif") {
                let Ok(rel) = path.strip_prefix(source_root) else {
                    continue;
                };
                let rel = rel.to_string_lossy().replace('/', "\\");
                let rid = resource_id_from_path(&rel);
                by_id.entry(rid).or_insert(rel);
            }
        }
        Ok(Self { by_id })
    }

    fn resolve(&self, rid: &ResourceId) -> Option<&str> {
        self.by_id.get(rid).map(String::as_str)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceBtoMode {
    Raw,
    Atlas,
}

impl SourceBtoMode {
    fn is_atlas(self) -> bool {
        self == SourceBtoMode::Atlas
    }
}

fn atlas_applies_to_level(settings: &LodSettings, level: i32) -> bool {
    settings
        .objects
        .fo76_bto_atlas_from_lod
        .map(|min_level| level >= min_level)
        .unwrap_or(true)
}

fn source_bto_mode_for_tile(
    global_mode: SourceBtoMode,
    settings: &LodSettings,
    tile: &SourceBtoTile,
) -> SourceBtoMode {
    if global_mode.is_atlas() && atlas_applies_to_level(settings, tile.level) {
        SourceBtoMode::Atlas
    } else {
        SourceBtoMode::Raw
    }
}

#[derive(Default)]
struct SourceTileStats {
    stats: LodGenStats,
    input_shapes: u64,
    output_shapes: u64,
    load_secs: f64,
    tree_secs: f64,
    extract_secs: f64,
    write_secs: f64,
    btt_secs: f64,
    total_secs: f64,
    tree_instances_to_btt: u64,
    far_tree_3d_fallback: u64,
    skipped_missing_billboard: BTreeMap<String, u64>,
    placed_tree_indices: std::collections::BTreeSet<i32>,
}

#[derive(Default)]
struct SourceAtlasStats {
    texture_sets: usize,
    pages: usize,
    dds_written: u32,
    scan_warnings: Vec<String>,
}

struct SourceBtoAtlasScanResult {
    index: usize,
    texture_sets: BTreeMap<String, HybridAtlasTexture>,
    instance_models: BTreeMap<(String, usize), String>,
    warnings: Vec<String>,
}

type TreeTileIndex = HashMap<(i32, i32, i32), Vec<usize>>;

const HYBRID_ATLAS_CACHE_MISSING_TOLERANCE: usize = 16;
const HYBRID_ATLAS_CACHE_VERSION: u32 = 7;

#[derive(Clone)]
struct HybridAtlasTexture {
    diffuse_rel: String,
    normal_rel: String,
    lighting_rel: String,
    specular_rel: String,
    diffuse_path: PathBuf,
    normal_path: Option<PathBuf>,
    lighting_path: Option<PathBuf>,
    specular_path: Option<PathBuf>,
}

impl HybridAtlasTexture {
    fn atlas_key(&self) -> String {
        if !self.lighting_rel.is_empty() {
            format!(
                "{},{},{}",
                self.diffuse_rel, self.normal_rel, self.lighting_rel
            )
        } else if !self.normal_rel.is_empty() {
            format!("{},{}", self.diffuse_rel, self.normal_rel)
        } else {
            self.diffuse_rel.clone()
        }
    }
}

pub fn build_object_lod(
    world: &WorldspaceInput,
    settings: &LodSettings,
    game: &Game,
    paths: &LodPaths,
    progress: &mut dyn Progress,
) -> anyhow::Result<LodGenStats> {
    let source_root = paths
        .source_data_dir
        .as_ref()
        .context("objects.source=fo76_bto requires source_data_dir")?;
    let all_tiles = enumerate_source_bto_tiles(source_root, &world.editor_id)?;
    if all_tiles.is_empty() {
        anyhow::bail!(
            "objects.source=fo76_bto found no source BTOs under {} for world {}",
            source_root.display(),
            world.editor_id
        );
    }

    let tiles: Vec<SourceBtoTile> = all_tiles
        .into_iter()
        .filter(|tile| tile_allowed(tile, settings))
        .collect();
    if tiles.is_empty() {
        anyhow::bail!(
            "objects.source=fo76_bto found source BTOs for {} but none matched lod settings",
            world.editor_id
        );
    }

    let mode = if settings.objects.source == ObjectSource::Fo76BtoAtlas
        || settings.objects.fo76_bto_atlas_pages
    {
        SourceBtoMode::Atlas
    } else {
        SourceBtoMode::Raw
    };
    let standard_tree_tiles = if mode.is_atlas() && settings.trees.trees_3d {
        index_standard_tree_refs_by_tile(world, settings)
    } else {
        TreeTileIndex::new()
    };
    let resolver = ResourceResolver::build(source_root)?;
    let atlas_tiles: Vec<SourceBtoTile> = tiles
        .iter()
        .filter(|tile| source_bto_mode_for_tile(mode, settings, tile).is_atlas())
        .cloned()
        .collect();
    let (atlas, atlas_stats) = if mode.is_atlas() && !atlas_tiles.is_empty() {
        build_source_bto_atlas(
            world,
            settings,
            game,
            paths,
            source_root,
            &resolver,
            &atlas_tiles,
            &standard_tree_tiles,
            progress,
        )?
    } else {
        (
            empty_atlas_result(),
            SourceAtlasStats {
                texture_sets: 0,
                pages: 0,
                dds_written: 0,
                scan_warnings: Vec::new(),
            },
        )
    };
    let atlas = Arc::new(atlas);
    let billboard_manifest =
        if mode.is_atlas() && settings.objects.fo76_bto_tree_billboard_from_lod.is_some() {
            let local_paths = paths_with_source_root(paths, source_root);
            let ctx = QuadCtx {
                world,
                settings,
                game,
                paths: &local_paths,
                level: settings.global.lod_min,
            };
            crate::trees::load_billboard_manifest(&ctx)
        } else {
            None
        };
    let copied_textures = Arc::new(Mutex::new(HashSet::new()));
    let worker_count = crate::driver::effective_worker_count(settings.global.workers);
    let mut stats = LodGenStats::default();
    stats.dds += atlas_stats.dds_written;
    stats.warnings.extend(atlas_stats.scan_warnings);
    if mode.is_atlas()
        && settings.objects.fo76_bto_tree_billboard_from_lod.is_some()
        && billboard_manifest.is_none()
    {
        stats.warnings.push(format!(
            "hybrid-atlas objects: billboard manifest not found for '{}'; far tree instances will stay as 3D BTO geometry",
            world.editor_id
        ));
    }

    progress.report(
        &format!(
            "{} objects: converting {} source BTOs workers={}",
            if mode.is_atlas() {
                "hybrid-atlas"
            } else {
                "hybrid"
            },
            tiles.len(),
            worker_count
        ),
        0.0,
    );

    let convert_one = |idx: usize, tile: &SourceBtoTile| match convert_tile(
        world,
        settings,
        game,
        paths,
        source_root,
        &resolver,
        tile,
        copied_textures.as_ref(),
        source_bto_mode_for_tile(mode, settings, tile),
        atlas.as_ref(),
        billboard_manifest.as_ref(),
        &standard_tree_tiles,
    ) {
        Ok(tile_stats) => (idx, tile_stats, None),
        Err(err) => (
            idx,
            SourceTileStats::default(),
            Some(format!(
                "hybrid source BTO skipped {}: {err:#}",
                tile.path.display()
            )),
        ),
    };

    let mut results: Vec<(usize, SourceTileStats, Option<String>)> =
        if worker_count > 1 && tiles.len() > 1 {
            if let Some(pool) = crate::driver::build_worker_pool(settings.global.workers) {
                pool.install(|| {
                    tiles
                        .par_iter()
                        .enumerate()
                        .map(|(idx, tile)| convert_one(idx, tile))
                        .collect()
                })
            } else {
                tiles
                    .par_iter()
                    .enumerate()
                    .map(|(idx, tile)| convert_one(idx, tile))
                    .collect()
            }
        } else {
            tiles
                .iter()
                .enumerate()
                .map(|(idx, tile)| convert_one(idx, tile))
                .collect()
        };
    results.sort_by_key(|(idx, _, _)| *idx);
    let mut input_shapes = 0u64;
    let mut output_shapes = 0u64;
    let mut load_secs = 0.0;
    let mut tree_secs = 0.0;
    let mut extract_secs = 0.0;
    let mut write_secs = 0.0;
    let mut btt_secs = 0.0;
    let mut total_secs = 0.0;
    let mut tree_instances_to_btt = 0u64;
    let mut far_tree_3d_fallback = 0u64;
    let mut skipped_missing_billboard = BTreeMap::<String, u64>::new();
    let mut placed_tree_indices = std::collections::BTreeSet::new();
    for (_, tile_stats, warning) in results {
        stats.bto += tile_stats.stats.bto;
        stats.btt += tile_stats.stats.btt;
        stats.dds += tile_stats.stats.dds;
        stats.warnings.extend(tile_stats.stats.warnings);
        input_shapes += tile_stats.input_shapes;
        output_shapes += tile_stats.output_shapes;
        load_secs += tile_stats.load_secs;
        tree_secs += tile_stats.tree_secs;
        extract_secs += tile_stats.extract_secs;
        write_secs += tile_stats.write_secs;
        btt_secs += tile_stats.btt_secs;
        total_secs += tile_stats.total_secs;
        tree_instances_to_btt += tile_stats.tree_instances_to_btt;
        far_tree_3d_fallback += tile_stats.far_tree_3d_fallback;
        placed_tree_indices.extend(tile_stats.placed_tree_indices);
        for (model, count) in tile_stats.skipped_missing_billboard {
            *skipped_missing_billboard.entry(model).or_default() += count;
        }
        if let Some(warning) = warning {
            stats.warnings.push(warning);
        }
    }
    if mode.is_atlas() {
        if let Some(manifest) = billboard_manifest.as_ref() {
            if !placed_tree_indices.is_empty() {
                write_source_bto_tree_list(world, paths, manifest, &placed_tree_indices)?;
            }
        }
        if !skipped_missing_billboard.is_empty() {
            let skipped_total = skipped_missing_billboard.values().copied().sum::<u64>();
            let species = skipped_missing_billboard.len();
            stats.warnings.push(format!(
                "hybrid-atlas objects: skipped {skipped_total} far tree instance(s) across {species} model(s) with no billboard manifest entry"
            ));
        }
        if far_tree_3d_fallback > 0 {
            stats.warnings.push(format!(
                "hybrid-atlas objects: kept {far_tree_3d_fallback} far tree instance(s) as 3D LOD because billboard data was missing"
            ));
        }
        progress.report(
            &format!(
                "hybrid-atlas objects complete: atlas_pages={} atlas_textures={} source_shapes={} bto_shapes={} tree_btt={} far_tree_3d_fallback={} dds={} tile_secs(load={:.1},tree={:.1},extract={:.1},write={:.1},btt={:.1},total={:.1})",
                atlas_stats.pages,
                atlas_stats.texture_sets,
                input_shapes,
                output_shapes,
                tree_instances_to_btt,
                far_tree_3d_fallback,
                stats.dds,
                load_secs,
                tree_secs,
                extract_secs,
                write_secs,
                btt_secs,
                total_secs
            ),
            1.0,
        );
    } else {
        progress.report("hybrid objects complete", 1.0);
    }

    Ok(stats)
}

fn empty_atlas_result() -> AtlasResult {
    AtlasResult {
        map_path: PathBuf::new(),
        diffuse: PathBuf::new(),
        normal: PathBuf::new(),
        specular: PathBuf::new(),
        atlas_size: (0, 0),
        uv: HashMap::new(),
        list: crate::atlas::AtlasList::new(),
        dds_written: 0,
    }
}

fn build_source_bto_atlas(
    world: &WorldspaceInput,
    settings: &LodSettings,
    game: &Game,
    paths: &LodPaths,
    source_root: &Path,
    resolver: &ResourceResolver,
    tiles: &[SourceBtoTile],
    standard_tree_tiles: &TreeTileIndex,
    progress: &mut dyn Progress,
) -> anyhow::Result<(AtlasResult, SourceAtlasStats)> {
    let started = Instant::now();
    if let Some(cached) =
        try_load_cached_hybrid_atlas_without_scan(&world.editor_id, settings, paths, progress)
    {
        return Ok(cached);
    }
    if let Some(rebuilt) =
        try_rebuild_hybrid_atlas_from_existing_map(world, settings, paths, source_root, progress)
    {
        return Ok(rebuilt);
    }

    let worker_count = crate::driver::effective_worker_count(settings.global.workers);
    progress.report(
        &format!(
            "hybrid-atlas objects: scanning {} source BTOs for texture atlas workers={worker_count}",
            tiles.len(),
        ),
        0.0,
    );
    let mut texture_sets = BTreeMap::<String, HybridAtlasTexture>::new();
    let mut instance_models = BTreeMap::<(String, usize), String>::new();
    let mut warnings = Vec::new();
    let local_paths = paths_with_source_root(paths, source_root);
    let report_every = (tiles.len() / 20).max(1);

    let mut scan_results = Vec::<SourceBtoAtlasScanResult>::with_capacity(tiles.len());
    if worker_count > 1 && tiles.len() > 1 {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| -> anyhow::Result<()> {
            let handle = scope.spawn(|| {
                let run = || {
                    tiles.par_iter().enumerate().for_each_with(
                        tx.clone(),
                        |sender, (idx, tile)| {
                            let result = scan_one_source_bto_for_atlas(
                                idx,
                                tile,
                                world,
                                settings,
                                game,
                                &local_paths,
                                source_root,
                                resolver,
                            );
                            let _ = sender.send(result);
                        },
                    );
                };
                if let Some(pool) = crate::driver::build_worker_pool(settings.global.workers) {
                    pool.install(run);
                } else {
                    run();
                }
            });

            for done in 1..=tiles.len() {
                let result = rx
                    .recv()
                    .context("hybrid-atlas source BTO scan worker stopped early")?;
                scan_results.push(result);
                if done == tiles.len() || done % report_every == 0 {
                    let partial_texture_sets: usize =
                        scan_results.iter().map(|r| r.texture_sets.len()).sum();
                    progress.report(
                        &format!(
                            "hybrid-atlas objects: scanned {done}/{} BTOs, up to {} texture set(s)",
                            tiles.len(),
                            partial_texture_sets
                        ),
                        0.05 + (done as f32 / tiles.len().max(1) as f32) * 0.25,
                    );
                }
            }

            handle
                .join()
                .map_err(|_| anyhow::anyhow!("hybrid-atlas source BTO scan worker panicked"))?;
            Ok(())
        })?;
        scan_results.sort_by_key(|result| result.index);
    } else {
        for (idx, tile) in tiles.iter().enumerate() {
            scan_results.push(scan_one_source_bto_for_atlas(
                idx,
                tile,
                world,
                settings,
                game,
                &local_paths,
                source_root,
                resolver,
            ));
            let done = idx + 1;
            if done == tiles.len() || done % report_every == 0 {
                let partial_texture_sets: usize =
                    scan_results.iter().map(|r| r.texture_sets.len()).sum();
                progress.report(
                    &format!(
                        "hybrid-atlas objects: scanned {done}/{} BTOs, up to {} texture set(s)",
                        tiles.len(),
                        partial_texture_sets
                    ),
                    0.05 + (done as f32 / tiles.len().max(1) as f32) * 0.25,
                );
            }
        }
    }

    for result in scan_results {
        warnings.extend(result.warnings);
        for (key, texture) in result.texture_sets {
            texture_sets.entry(key).or_insert(texture);
        }
        for (key, model) in result.instance_models {
            instance_models.entry(key).or_insert(model);
        }
    }
    if settings.trees.trees_3d && !standard_tree_tiles.is_empty() {
        scan_standard_tree_atlas_textures(
            world,
            settings,
            game,
            paths,
            standard_tree_tiles,
            &mut texture_sets,
        );
    }
    progress.report(
        &format!(
            "hybrid-atlas objects: scanned {}/{} BTOs, {} baked texture set(s), {} instance model(s), elapsed={:.1}s",
            tiles.len(),
            tiles.len(),
            texture_sets.len(),
            instance_models.len(),
            started.elapsed().as_secs_f32()
        ),
        0.3,
    );

    if !instance_models.is_empty() {
        progress.report(
            &format!(
                "hybrid-atlas objects: scanning {} unique instance model(s) for atlas textures workers={worker_count}",
                instance_models.len()
            ),
            0.31,
        );
        let models: Vec<(String, usize)> = instance_models
            .into_iter()
            .map(|((_key, level_index), model)| (model, level_index))
            .collect();
        let scan_model = |(model, level_index): &(String, usize)| {
            scan_instance_model_atlas_textures(
                source_root,
                model,
                *level_index,
                world,
                settings,
                game,
                &local_paths,
            )
        };
        let model_texture_sets: Vec<BTreeMap<String, HybridAtlasTexture>> =
            if worker_count > 1 && models.len() > 1 {
                if let Some(pool) = crate::driver::build_worker_pool(settings.global.workers) {
                    pool.install(|| models.par_iter().map(scan_model).collect())
                } else {
                    models.par_iter().map(scan_model).collect()
                }
            } else {
                models.iter().map(scan_model).collect()
            };
        for model_sets in model_texture_sets {
            for (key, texture) in model_sets {
                texture_sets.entry(key).or_insert(texture);
            }
        }
        progress.report(
            &format!(
                "hybrid-atlas objects: scanned instance models, {} total texture set(s), elapsed={:.1}s",
                texture_sets.len(),
                started.elapsed().as_secs_f32()
            ),
            0.35,
        );
    }

    let texture_count = texture_sets.len();
    let (atlas, mut atlas_stats) = build_paged_hybrid_atlas(
        world,
        settings,
        paths,
        texture_sets.into_values().collect(),
        progress,
        true,
    )?;
    atlas_stats.texture_sets = texture_count;
    atlas_stats.scan_warnings.extend(warnings);
    progress.report(
        &format!(
            "hybrid-atlas objects: source atlas pass complete pages={} textures={} dds={} elapsed={:.1}s",
            atlas_stats.pages,
            atlas_stats.texture_sets,
            atlas_stats.dds_written,
            started.elapsed().as_secs_f32()
        ),
        0.88,
    );
    Ok((atlas, atlas_stats))
}

fn scan_one_source_bto_for_atlas(
    index: usize,
    tile: &SourceBtoTile,
    world: &WorldspaceInput,
    settings: &LodSettings,
    game: &Game,
    local_paths: &LodPaths,
    source_root: &Path,
    resolver: &ResourceResolver,
) -> SourceBtoAtlasScanResult {
    let mut result = SourceBtoAtlasScanResult {
        index,
        texture_sets: BTreeMap::new(),
        instance_models: BTreeMap::new(),
        warnings: Vec::new(),
    };
    let nif = match NifFile::load(&tile.path) {
        Ok(nif) => nif,
        Err(err) => {
            result.warnings.push(format!(
                "hybrid-atlas objects: failed to scan {} for atlas textures: {err:?}",
                tile.path.display()
            ));
            return result;
        }
    };
    let ctx = QuadCtx {
        world,
        settings,
        game,
        paths: local_paths,
        level: tile.level,
    };
    scan_source_bto_atlas_textures(
        &nif,
        tile,
        source_root,
        resolver,
        &ctx,
        &mut result.texture_sets,
        &mut result.instance_models,
    );
    result
}

fn scan_source_bto_atlas_textures(
    nif: &NifFile,
    tile: &SourceBtoTile,
    source_root: &Path,
    resolver: &ResourceResolver,
    ctx: &QuadCtx<'_>,
    texture_sets: &mut BTreeMap<String, HybridAtlasTexture>,
    instance_models: &mut BTreeMap<(String, usize), String>,
) {
    for block in &nif.blocks {
        match block.type_name.as_str() {
            "BSSubIndexTriShape" | "BSTriShape"
                if should_extract_baked_block(ctx.settings, block, SourceBtoMode::Atlas, false) =>
            {
                scan_baked_block_atlas_textures(nif, block, source_root, ctx.paths, texture_sets);
            }
            "BSDistantObjectInstancedNode" if ctx.settings.objects.fo76_bto_include_instances => {
                scan_instance_node_atlas_textures(block, tile, resolver, ctx, instance_models);
            }
            _ => {}
        }
    }
}

fn scan_baked_block_atlas_textures(
    nif: &NifFile,
    geom_block: &NifBlock,
    source_root: &Path,
    paths: &LodPaths,
    texture_sets: &mut BTreeMap<String, HybridAtlasTexture>,
) {
    let shader_ref = val_ref(geom_block.get_field("Shader Property")).unwrap_or(-1);
    let Some(shader) = block_of(nif, shader_ref) else {
        return;
    };
    if shader.type_name != "BSLightingShaderProperty" {
        return;
    }
    if let Some(arrays) = shader_texture_arrays(shader) {
        let slice_count = arrays.iter().map(Vec::len).max().unwrap_or(0);
        for slice in 0..slice_count {
            let Some(texture_set) = atlas_texture_set_for_array_slice(&arrays, slice) else {
                continue;
            };
            insert_hybrid_atlas_texture_set(paths, source_root, &texture_set, texture_sets);
        }
        return;
    }
    if let Some(texture_set) = shader_texture_set(nif, shader) {
        insert_hybrid_atlas_texture_set(paths, source_root, &texture_set, texture_sets);
    }
}

fn scan_instance_node_atlas_textures(
    node: &NifBlock,
    tile: &SourceBtoTile,
    resolver: &ResourceResolver,
    ctx: &QuadCtx<'_>,
    instance_models: &mut BTreeMap<(String, usize), String>,
) {
    let level_index = level_index(tile.level);
    for inst_value in val_array(node.get_field("Instances")) {
        let Some(inst) = as_struct(Some(inst_value)) else {
            continue;
        };
        let Some(rid) = inst.get("Resource ID").and_then(resource_id_from_value) else {
            continue;
        };
        let Some(model) = resolver.resolve(&rid) else {
            continue;
        };
        let is_tree = is_tree_instance_model(model);
        if !ctx.settings.objects.fo76_bto_include_tree_instances && is_tree {
            continue;
        }
        if is_tree
            && ctx
                .settings
                .objects
                .fo76_bto_tree_billboard_from_lod
                .map(|min_lod| tile.level >= min_lod)
                .unwrap_or(false)
        {
            continue;
        }
        let model_key = (model.to_ascii_lowercase(), level_index);
        instance_models
            .entry(model_key)
            .or_insert_with(|| model.to_string());
    }
}

fn scan_instance_model_atlas_textures(
    source_root: &Path,
    model: &str,
    level_index: usize,
    world: &WorldspaceInput,
    settings: &LodSettings,
    game: &Game,
    paths: &LodPaths,
) -> BTreeMap<String, HybridAtlasTexture> {
    let level = match level_index {
        0 => 4,
        1 => 8,
        2 => 16,
        3 => 32,
        _ => 4,
    };
    let ctx = QuadCtx {
        world,
        settings,
        game,
        paths,
        level,
    };
    let stat = instance_static_desc(
        model,
        level_index,
        [0.0, 0.0, 0.0],
        identity_part_transform(),
        0,
        0,
    );
    let mut texture_sets = BTreeMap::new();
    let Ok(shapes) = crate::objects::parse_nif::parse_nif(&stat, level_index, &ctx) else {
        return texture_sets;
    };
    for shape in shapes {
        insert_hybrid_atlas_texture_set(paths, source_root, &shape.textures, &mut texture_sets);
    }
    texture_sets
}

fn scan_standard_tree_atlas_textures(
    world: &WorldspaceInput,
    settings: &LodSettings,
    game: &Game,
    paths: &LodPaths,
    standard_tree_tiles: &TreeTileIndex,
    texture_sets: &mut BTreeMap<String, HybridAtlasTexture>,
) {
    let mut seen_models = HashSet::<String>::new();
    for (&(level, _x, _y), indices) in standard_tree_tiles {
        if !atlas_applies_to_level(settings, level) {
            continue;
        }
        let level_index = level_index(level);
        let ctx = QuadCtx {
            world,
            settings,
            game,
            paths,
            level,
        };
        for &idx in indices {
            let Some(stat) = world.refs.get(idx) else {
                continue;
            };
            let Some(model) = stat.lod_models.get(level_index).and_then(|m| m.as_ref()) else {
                continue;
            };
            let key = format!("{level_index}:{}", model.to_ascii_lowercase());
            if !seen_models.insert(key) {
                continue;
            }
            let shapes = if model.to_ascii_lowercase().ends_with(".dds") {
                let fd = crate::trees::tree3d::load_flat_desc(model, &ctx);
                crate::trees::tree3d::build_flat_trunk(&fd, model)
            } else {
                match crate::objects::parse_nif::parse_nif(stat, level_index, &ctx) {
                    Ok(shapes) => shapes,
                    Err(err) => {
                        eprintln!(
                            "[lodgen] hybrid-atlas objects: skipping standard tree atlas scan {}: {err}",
                            model
                        );
                        Vec::new()
                    }
                }
            };
            for shape in shapes {
                insert_hybrid_atlas_texture_set_from_data_paths(
                    paths,
                    &shape.textures,
                    texture_sets,
                );
            }
        }
    }
}

fn atlas_texture_set_for_array_slice(arrays: &[Vec<String>], slice: usize) -> Option<[String; 10]> {
    let diffuse = arrays.get(0)?.get(slice)?;
    if diffuse.is_empty() {
        return None;
    }
    let mut texture_set: [String; 10] = Default::default();
    texture_set[0] = normalize_texture_rel(diffuse);
    if let Some(normal) = arrays.get(1).and_then(|v| v.get(slice)) {
        texture_set[1] = normalize_texture_rel(normal);
    }
    if let Some(lighting) = arrays.get(10).and_then(|v| v.get(slice)) {
        texture_set[2] = normalize_texture_rel(lighting);
    }
    if let Some(specular) = arrays.get(9).and_then(|v| v.get(slice)) {
        texture_set[7] = normalize_texture_rel(specular);
    }
    Some(texture_set)
}

fn insert_hybrid_atlas_texture_set(
    paths: &LodPaths,
    source_root: &Path,
    texture_set: &[String; 10],
    texture_sets: &mut BTreeMap<String, HybridAtlasTexture>,
) {
    let Some(source_diffuse) = resolve_atlas_texture(source_root, &texture_set[0]) else {
        return;
    };
    let (diffuse_rel, diffuse_path) =
        resolve_atlas_texture_from_data_paths(paths, &texture_set[0]).unwrap_or(source_diffuse);
    let normal = resolve_atlas_texture_from_data_paths(paths, &texture_set[1])
        .or_else(|| resolve_atlas_texture(source_root, &texture_set[1]));
    let converted_specular = converted_fo4_specular_for_diffuse(paths, &texture_set[0]);
    let specular = converted_specular
        .clone()
        .or_else(|| resolve_atlas_texture_from_data_paths(paths, &texture_set[7]))
        .or_else(|| resolve_atlas_texture(source_root, &texture_set[7]));
    let lighting = if converted_specular.is_some() {
        None
    } else {
        resolve_atlas_texture(source_root, &texture_set[2])
    };
    let (normal_rel, normal_path) = normal
        .map(|(rel, path)| (rel, Some(path)))
        .unwrap_or_default();
    let (lighting_rel, lighting_path) = lighting
        .map(|(rel, path)| (rel, Some(path)))
        .unwrap_or_default();
    let (specular_rel, specular_path) = specular
        .map(|(rel, path)| (rel, Some(path)))
        .unwrap_or_default();
    let texture = HybridAtlasTexture {
        diffuse_rel,
        normal_rel,
        lighting_rel,
        specular_rel,
        diffuse_path,
        normal_path,
        lighting_path,
        specular_path,
    };
    texture_sets
        .entry(texture.atlas_key().to_ascii_lowercase())
        .or_insert(texture);
}

fn converted_fo4_specular_for_diffuse(
    paths: &LodPaths,
    diffuse: &str,
) -> Option<(String, PathBuf)> {
    let specular = replace_texture_suffix(diffuse, "_d.dds", "_s.dds")?;
    resolve_atlas_texture_from_data_paths(paths, &specular)
}

fn replace_texture_suffix(texture: &str, from: &str, to: &str) -> Option<String> {
    let normalized = normalize_texture_rel(texture);
    let lower = normalized.to_ascii_lowercase();
    lower
        .ends_with(from)
        .then(|| format!("{}{}", &normalized[..normalized.len() - from.len()], to))
}

fn insert_hybrid_atlas_texture_set_from_data_paths(
    paths: &LodPaths,
    texture_set: &[String; 10],
    texture_sets: &mut BTreeMap<String, HybridAtlasTexture>,
) {
    let Some((diffuse_rel, diffuse_path)) =
        resolve_atlas_texture_from_data_paths(paths, &texture_set[0])
    else {
        return;
    };
    let normal = resolve_atlas_texture_from_data_paths(paths, &texture_set[1]);
    let lighting = resolve_atlas_texture_from_data_paths(paths, &texture_set[2]);
    let specular = resolve_atlas_texture_from_data_paths(paths, &texture_set[7]);
    let (normal_rel, normal_path) = normal
        .map(|(rel, path)| (rel, Some(path)))
        .unwrap_or_default();
    let (lighting_rel, lighting_path) = lighting
        .map(|(rel, path)| (rel, Some(path)))
        .unwrap_or_default();
    let (specular_rel, specular_path) = specular
        .map(|(rel, path)| (rel, Some(path)))
        .unwrap_or_default();
    let texture = HybridAtlasTexture {
        diffuse_rel,
        normal_rel,
        lighting_rel,
        specular_rel,
        diffuse_path,
        normal_path,
        lighting_path,
        specular_path,
    };
    texture_sets
        .entry(texture.atlas_key().to_ascii_lowercase())
        .or_insert(texture);
}

fn resolve_atlas_texture(source_root: &Path, texture: &str) -> Option<(String, PathBuf)> {
    let normalized = normalize_texture_rel(texture);
    if normalized.is_empty()
        || !normalized.to_ascii_lowercase().ends_with(".dds")
        || is_fo4_shared_texture(&normalized)
    {
        return None;
    }
    let path = resolve_source_data_path_ci(source_root, &normalized)?;
    Some((normalized, path))
}

fn resolve_atlas_texture_from_data_paths(
    paths: &LodPaths,
    texture: &str,
) -> Option<(String, PathBuf)> {
    let normalized = normalize_texture_rel(texture);
    if normalized.is_empty()
        || !normalized.to_ascii_lowercase().ends_with(".dds")
        || is_fo4_shared_texture(&normalized)
    {
        return None;
    }
    if let Some(path) = resolve_source_data_path_ci(&paths.output_dir, &normalized) {
        return Some((normalized, path));
    }
    for dir in &paths.data_dirs {
        if let Some(path) = resolve_source_data_path_ci(dir, &normalized) {
            return Some((normalized, path));
        }
    }
    None
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct HybridAtlasCacheMeta {
    version: u32,
    atlas_size: u32,
    mip_flooding: bool,
    max_tile_size: u32,
    min_tile_size: u32,
    min_foliage_tile_size: u32,
    foliage_page_size: u32,
    foliage_max_tile_size: u32,
    min_alpha_tested_tile_size: u32,
    alpha_tested_page_size: u32,
    alpha_tested_max_tile_size: u32,
    include_baked: bool,
    include_global_atlas_baked: bool,
    include_instances: bool,
    include_tree_instances: bool,
    atlas_from_lod: Option<i32>,
    tree_billboard_from_lod: Option<i32>,
    trees_3d: bool,
}

fn hybrid_atlas_cache_meta(settings: &LodSettings) -> HybridAtlasCacheMeta {
    HybridAtlasCacheMeta {
        version: HYBRID_ATLAS_CACHE_VERSION,
        atlas_size: settings.objects.atlas_size,
        mip_flooding: settings.objects.atlas_mip_flooding,
        max_tile_size: settings.objects.max_tile_size,
        min_tile_size: settings.objects.fo76_bto_atlas_min_tile_size,
        min_foliage_tile_size: settings.objects.fo76_bto_atlas_min_foliage_tile_size,
        foliage_page_size: settings.objects.fo76_bto_atlas_foliage_page_size,
        foliage_max_tile_size: settings.objects.fo76_bto_atlas_foliage_max_tile_size,
        min_alpha_tested_tile_size: settings.objects.fo76_bto_atlas_min_alpha_tested_tile_size,
        alpha_tested_page_size: settings.objects.fo76_bto_atlas_alpha_tested_page_size,
        alpha_tested_max_tile_size: settings.objects.fo76_bto_atlas_alpha_tested_max_tile_size,
        include_baked: settings.objects.fo76_bto_include_baked,
        include_global_atlas_baked: settings.objects.fo76_bto_include_global_atlas_baked,
        include_instances: settings.objects.fo76_bto_include_instances,
        include_tree_instances: settings.objects.fo76_bto_include_tree_instances,
        atlas_from_lod: settings.objects.fo76_bto_atlas_from_lod,
        tree_billboard_from_lod: settings.objects.fo76_bto_tree_billboard_from_lod,
        trees_3d: settings.trees.trees_3d,
    }
}

fn hybrid_atlas_meta_path(map_path: &Path) -> PathBuf {
    map_path.with_extension("meta.json")
}

fn read_hybrid_atlas_cache_meta(map_path: &Path) -> Option<HybridAtlasCacheMeta> {
    let meta_path = hybrid_atlas_meta_path(map_path);
    let text = std::fs::read_to_string(&meta_path).ok()?;
    serde_json::from_str::<HybridAtlasCacheMeta>(&text).ok()
}

fn hybrid_atlas_cache_meta_matches(
    map_path: &Path,
    settings: &LodSettings,
    require_meta: bool,
) -> bool {
    let Some(actual) = read_hybrid_atlas_cache_meta(map_path) else {
        return !require_meta;
    };
    actual == hybrid_atlas_cache_meta(settings)
}

fn hybrid_atlas_cache_source_selection_matches(
    actual: &HybridAtlasCacheMeta,
    settings: &LodSettings,
) -> bool {
    let expected = hybrid_atlas_cache_meta(settings);
    actual.version == expected.version
        && actual.include_baked == expected.include_baked
        && actual.include_global_atlas_baked == expected.include_global_atlas_baked
        && actual.include_instances == expected.include_instances
        && actual.include_tree_instances == expected.include_tree_instances
        && actual.atlas_from_lod == expected.atlas_from_lod
        && actual.tree_billboard_from_lod == expected.tree_billboard_from_lod
        && actual.trees_3d == expected.trees_3d
}

fn write_hybrid_atlas_cache_meta(map_path: &Path, settings: &LodSettings) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(&hybrid_atlas_cache_meta(settings))?;
    std::fs::write(hybrid_atlas_meta_path(map_path), text)?;
    Ok(())
}

fn try_load_cached_hybrid_atlas_without_scan(
    world: &str,
    settings: &LodSettings,
    paths: &LodPaths,
    progress: &mut dyn Progress,
) -> Option<(AtlasResult, SourceAtlasStats)> {
    let (atlas, stats, _row_sources) = load_cached_hybrid_atlas(world, settings, paths, true)?;
    progress.report(
        &format!(
            "hybrid-atlas objects: reused cached atlas pages={} textures={} dds=0; skipped source BTO atlas scan",
            stats.pages, stats.texture_sets
        ),
        0.88,
    );
    Some((atlas, stats))
}

fn load_cached_hybrid_atlas(
    world: &str,
    settings: &LodSettings,
    paths: &LodPaths,
    require_meta: bool,
) -> Option<(AtlasResult, SourceAtlasStats, BTreeSet<String>)> {
    let map_path = paths
        .output_dir
        .join(source_bto_atlas_page_rel(world, 0).replace('\\', "/"))
        .with_extension("txt");
    if !map_path.is_file() {
        return None;
    }
    if !hybrid_atlas_cache_meta_matches(&map_path, settings, require_meta) {
        return None;
    }

    let text = std::fs::read_to_string(&map_path).ok()?;
    let rows = crate::atlas::parse_atlas_map(&text);
    if rows.is_empty() {
        return None;
    }

    let mut row_sources = BTreeSet::new();
    let mut pages = BTreeSet::new();
    for row in &rows {
        row_sources.insert(row.source.to_ascii_lowercase());
        pages.insert(row.atlas.clone());
    }

    for atlas_rel in &pages {
        let atlas_path = paths.output_dir.join(atlas_rel.replace('\\', "/"));
        if !atlas_path.is_file()
            || !crate::atlas::atlas::sibling_dds(&atlas_path, "_n").is_file()
            || !crate::atlas::atlas::sibling_dds(&atlas_path, "_s").is_file()
        {
            return None;
        }
    }

    let first_diffuse = paths.output_dir.join(rows[0].atlas.replace('\\', "/"));
    let first_normal = crate::atlas::atlas::sibling_dds(&first_diffuse, "_n");
    let first_specular = crate::atlas::atlas::sibling_dds(&first_diffuse, "_s");
    let mut uv = HashMap::<String, AtlasRect>::new();
    let mut list = crate::atlas::AtlasList::new();
    for row in &rows {
        let rect = AtlasRect::from_map_row(
            row.tile_w,
            row.tile_h,
            row.x,
            row.y,
            row.atlas_w,
            row.atlas_h,
            &row.atlas,
            false,
        );
        let key = row.source.to_ascii_lowercase();
        uv.insert(key.clone(), rect.clone());
        list.insert(key, rect);
    }

    let texture_sets = row_sources.len();
    Some((
        AtlasResult {
            map_path,
            diffuse: first_diffuse,
            normal: first_normal,
            specular: first_specular,
            atlas_size: (rows[0].atlas_w, rows[0].atlas_h),
            uv,
            list,
            dds_written: 0,
        },
        SourceAtlasStats {
            texture_sets,
            pages: pages.len(),
            dds_written: 0,
            scan_warnings: Vec::new(),
        },
        row_sources,
    ))
}

fn try_load_cached_hybrid_atlas(
    world: &str,
    settings: &LodSettings,
    paths: &LodPaths,
    textures: &[HybridAtlasTexture],
    progress: &mut dyn Progress,
) -> Option<(AtlasResult, SourceAtlasStats)> {
    let (atlas, mut stats, row_sources) = load_cached_hybrid_atlas(world, settings, paths, false)?;
    let expected: BTreeSet<String> = textures
        .iter()
        .map(|texture| texture.atlas_key().to_ascii_lowercase())
        .collect();
    let missing = expected
        .iter()
        .filter(|source| !row_sources.contains(*source))
        .count();
    if missing > HYBRID_ATLAS_CACHE_MISSING_TOLERANCE {
        return None;
    }
    stats.texture_sets = expected.len();
    let _ = write_hybrid_atlas_cache_meta(&atlas.map_path, settings);

    progress.report(
        &format!(
            "hybrid-atlas objects: reused cached atlas pages={} textures={} missing_skipped={} dds=0",
            stats.pages,
            stats.texture_sets,
            missing
        ),
        0.88,
    );

    Some((atlas, stats))
}

fn try_rebuild_hybrid_atlas_from_existing_map(
    world: &WorldspaceInput,
    settings: &LodSettings,
    paths: &LodPaths,
    source_root: &Path,
    progress: &mut dyn Progress,
) -> Option<(AtlasResult, SourceAtlasStats)> {
    let map_path = paths
        .output_dir
        .join(source_bto_atlas_page_rel(&world.editor_id, 0).replace('\\', "/"))
        .with_extension("txt");
    if !map_path.is_file() || hybrid_atlas_cache_meta_matches(&map_path, settings, true) {
        return None;
    }
    let Some(cache_meta) = read_hybrid_atlas_cache_meta(&map_path) else {
        progress.report(
            "hybrid-atlas objects: existing atlas map has no compatible metadata; source BTO scan required",
            0.01,
        );
        return None;
    };
    if !hybrid_atlas_cache_source_selection_matches(&cache_meta, settings) {
        progress.report(
            "hybrid-atlas objects: existing atlas source selection changed; source BTO scan required",
            0.01,
        );
        return None;
    }
    let text = std::fs::read_to_string(&map_path).ok()?;
    let rows = crate::atlas::parse_atlas_map(&text);
    if rows.is_empty() {
        return None;
    }

    let mut textures = BTreeMap::<String, HybridAtlasTexture>::new();
    let mut skipped = 0usize;
    for row in rows {
        let row_key = row.source.to_ascii_lowercase();
        if textures.contains_key(&row_key) {
            continue;
        }
        if let Some(texture) =
            hybrid_atlas_texture_from_existing_map_key(paths, source_root, &row.source)
        {
            textures
                .entry(texture.atlas_key().to_ascii_lowercase())
                .or_insert(texture);
        } else {
            skipped += 1;
        }
    }
    if textures.is_empty() {
        return None;
    }

    let texture_count = textures.len();
    progress.report(
        &format!(
            "hybrid-atlas objects: rebuilding atlas from existing map texture list textures={} skipped={} (source BTO scan skipped)",
            texture_count, skipped
        ),
        0.01,
    );
    let (atlas, mut stats) = match build_paged_hybrid_atlas(
        world,
        settings,
        paths,
        textures.into_values().collect(),
        progress,
        false,
    ) {
        Ok(result) => result,
        Err(err) => {
            progress.report(
                &format!(
                    "hybrid-atlas objects: existing map atlas rebuild failed, falling back to source scan: {err:#}"
                ),
                0.01,
            );
            return None;
        }
    };
    stats.texture_sets = texture_count;
    if skipped > 0 {
        stats.scan_warnings.push(format!(
            "hybrid-atlas objects: existing atlas map rebuild skipped {skipped} unresolved texture row(s)"
        ));
    }
    progress.report(
        &format!(
            "hybrid-atlas objects: rebuilt atlas from existing map pages={} textures={} dds={} skipped={} (source BTO scan skipped)",
            stats.pages, stats.texture_sets, stats.dds_written, skipped
        ),
        0.88,
    );
    Some((atlas, stats))
}

fn hybrid_atlas_texture_from_existing_map_key(
    paths: &LodPaths,
    source_root: &Path,
    key: &str,
) -> Option<HybridAtlasTexture> {
    let parts: Vec<String> = key
        .split(',')
        .map(normalize_texture_rel)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }

    let mut source_texture_set: [String; 10] = Default::default();
    source_texture_set[0] = parts[0].clone();
    if let Some(normal) = parts.get(1) {
        source_texture_set[1] = normal.clone();
    }
    if let Some(lighting) = parts.get(2) {
        source_texture_set[2] = lighting.clone();
    }
    if let Some(reflectivity) = replace_texture_suffix(&source_texture_set[0], "_d.dds", "_r.dds") {
        source_texture_set[7] = reflectivity;
    }

    let mut one = BTreeMap::new();
    insert_hybrid_atlas_texture_set(paths, source_root, &source_texture_set, &mut one);
    if let Some(texture) = one.into_values().next() {
        return Some(texture);
    }

    let mut data_texture_set = source_texture_set;
    if let Some(specular) = replace_texture_suffix(&data_texture_set[0], "_d.dds", "_s.dds") {
        data_texture_set[7] = specular;
    }
    let mut one = BTreeMap::new();
    insert_hybrid_atlas_texture_set_from_data_paths(paths, &data_texture_set, &mut one);
    one.into_values().next()
}

fn build_paged_hybrid_atlas(
    world: &WorldspaceInput,
    settings: &LodSettings,
    paths: &LodPaths,
    textures: Vec<HybridAtlasTexture>,
    progress: &mut dyn Progress,
    allow_cache: bool,
) -> anyhow::Result<(AtlasResult, SourceAtlasStats)> {
    let started = Instant::now();
    let mut stats = SourceAtlasStats::default();
    if textures.is_empty() {
        progress.report("hybrid-atlas objects: no atlasable source textures", 0.35);
        return Ok((empty_atlas_result(), stats));
    }
    if allow_cache
        && let Some(cached) =
            try_load_cached_hybrid_atlas(&world.editor_id, settings, paths, &textures, progress)
    {
        return Ok(cached);
    }
    let worker_count = crate::driver::effective_worker_count(settings.global.workers);

    progress.report(
        &format!(
            "hybrid-atlas objects: loading {} atlas texture set(s) workers={worker_count}",
            textures.len(),
        ),
        0.35,
    );
    let load_started = Instant::now();
    let loaded = load_hybrid_atlas_tiles(settings, textures, progress, &mut stats)?;
    progress.report(
        &format!(
            "hybrid-atlas objects: atlas texture load complete kept={} elapsed={:.1}s total={:.1}s",
            loaded.len(),
            load_started.elapsed().as_secs_f32(),
            started.elapsed().as_secs_f32()
        ),
        0.6,
    );

    if loaded.is_empty() {
        progress.report("hybrid-atlas objects: no loadable atlas textures", 0.6);
        return Ok((empty_atlas_result(), stats));
    }

    let format_diffuse = object_format_to_str(&settings.objects.diffuse_format);
    let format_normal = object_format_to_str(&settings.objects.normal_format);
    let format_specular = object_format_to_str(&settings.objects.specular_format);
    let mut rows = Vec::<AtlasMapRow>::new();
    let mut uv = HashMap::<String, AtlasRect>::new();
    let mut list = crate::atlas::AtlasList::new();
    let mut first_diffuse = PathBuf::new();
    let mut first_normal = PathBuf::new();
    let mut first_specular = PathBuf::new();
    let mut first_map = PathBuf::new();
    let mut first_size = (0, 0);
    let mut remaining: Vec<usize> = (0..loaded.len()).collect();
    let mut page = 0usize;

    while !remaining.is_empty() {
        let page_started = Instant::now();
        let page_size = loaded[remaining[0]].page_size.max(1);
        let page_group = loaded[remaining[0]].group;
        let candidate_indices: Vec<usize> = remaining
            .iter()
            .copied()
            .filter(|index| {
                loaded[*index].page_size == page_size && loaded[*index].group == page_group
            })
            .collect();
        let mut blocks: Vec<crate::atlas::BinBlock> = candidate_indices
            .iter()
            .copied()
            .map(|index| crate::atlas::BinBlock {
                index,
                w: loaded[index].pack_w,
                h: loaded[index].pack_h,
                x: 0,
                y: 0,
                fit: false,
            })
            .collect();
        let packer = crate::atlas::BinPacker::new(page_size, page_size);
        let _ = packer.fit(&mut blocks);
        let fitted: Vec<crate::atlas::BinBlock> = blocks.into_iter().filter(|b| b.fit).collect();
        let fitted = if fitted.is_empty() {
            let index = candidate_indices[0];
            vec![crate::atlas::BinBlock {
                index,
                w: loaded[index].pack_w,
                h: loaded[index].pack_h,
                x: 0,
                y: 0,
                fit: true,
            }]
        } else {
            fitted
        };

        let (atlas_w, atlas_h) = fitted.iter().fold((1u32, 1u32), |(w, h), b| {
            (w.max(b.x + b.w), h.max(b.y + b.h))
        });
        let atlas_w = crate::atlas::atlas::next_pow2(atlas_w);
        let atlas_h = crate::atlas::atlas::next_pow2(atlas_h);
        let mut buf_d = vec![0u8; (atlas_w * atlas_h * 4) as usize];
        let mut buf_n = vec![128u8; (atlas_w * atlas_h * 4) as usize];
        let mut buf_s = missing_specular_rgba(atlas_w, atlas_h);

        let atlas_rel = source_bto_atlas_page_rel(&world.editor_id, page);
        let atlas_path = paths.output_dir.join(atlas_rel.replace('\\', "/"));
        let atlas_normal_path = crate::atlas::atlas::sibling_dds(&atlas_path, "_n");
        let atlas_specular_path = crate::atlas::atlas::sibling_dds(&atlas_path, "_s");
        let atlas_data_rel = crate::atlas::atlas::data_relative_path(&atlas_path);

        for block in &fitted {
            let tile = &loaded[block.index];
            crate::atlas::atlas::blit_rgba(
                &tile.rgba_d,
                tile.w,
                tile.h,
                &mut buf_d,
                atlas_w,
                block.x,
                block.y,
            );
            crate::atlas::atlas::blit_rgba(
                &tile.rgba_n,
                tile.w,
                tile.h,
                &mut buf_n,
                atlas_w,
                block.x,
                block.y,
            );
            crate::atlas::atlas::blit_rgba(
                &tile.rgba_s,
                tile.w,
                tile.h,
                &mut buf_s,
                atlas_w,
                block.x,
                block.y,
            );
            let source = tile.texture.atlas_key();
            let row = AtlasMapRow {
                source: source.clone(),
                tile_w: tile.w,
                tile_h: tile.h,
                x: block.x,
                y: block.y,
                atlas: atlas_data_rel.clone(),
                atlas_w,
                atlas_h,
            };
            let rect = AtlasRect::from_map_row(
                tile.w,
                tile.h,
                block.x,
                block.y,
                atlas_w,
                atlas_h,
                &atlas_data_rel,
                false,
            );
            let key = source.to_ascii_lowercase();
            uv.insert(key.clone(), rect.clone());
            list.insert(key, rect);
            rows.push(row);
        }

        if let Some(parent) = atlas_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        for (index, (path, buf, fmt)) in [
            (atlas_path.as_path(), &buf_d, format_diffuse),
            (atlas_normal_path.as_path(), &buf_n, format_normal),
            (atlas_specular_path.as_path(), &buf_s, format_specular),
        ]
        .into_iter()
        .enumerate()
        {
            match crate::atlas::atlas::write_atlas_dds(
                path,
                atlas_w,
                atlas_h,
                buf,
                fmt,
                index == 0 && settings.objects.atlas_mip_flooding,
            ) {
                Ok(()) => stats.dds_written += 1,
                Err(err) => eprintln!(
                    "[lodgen] hybrid-atlas objects: DDS write skipped ({}): {err}",
                    path.display()
                ),
            }
        }

        if page == 0 {
            first_diffuse = atlas_path.clone();
            first_normal = atlas_normal_path.clone();
            first_specular = atlas_specular_path.clone();
            first_map = atlas_path.with_extension("txt");
            first_size = (atlas_w, atlas_h);
        }
        stats.pages += 1;
        let fitted_set: HashSet<usize> = fitted.iter().map(|b| b.index).collect();
        remaining.retain(|index| !fitted_set.contains(index));
        page += 1;
        progress.report(
            &format!(
                "hybrid-atlas objects: wrote atlas page {} {}x{} limit ({} texture set(s), remaining={}) elapsed={:.1}s total={:.1}s",
                page,
                page_size,
                page_size,
                fitted_set.len(),
                remaining.len(),
                page_started.elapsed().as_secs_f32(),
                started.elapsed().as_secs_f32()
            ),
            0.6 + ((loaded.len() - remaining.len()) as f32 / loaded.len().max(1) as f32) * 0.25,
        );
    }

    if !first_map.as_os_str().is_empty() {
        crate::atlas::write_atlas_map(&first_map, &rows)?;
        write_hybrid_atlas_cache_meta(&first_map, settings)?;
    }
    progress.report(
        &format!(
            "hybrid-atlas objects: atlas ready pages={} textures={} dds={} elapsed={:.1}s",
            stats.pages,
            list.len(),
            stats.dds_written,
            started.elapsed().as_secs_f32()
        ),
        0.88,
    );

    Ok((
        AtlasResult {
            map_path: first_map,
            diffuse: first_diffuse,
            normal: first_normal,
            specular: first_specular,
            atlas_size: first_size,
            uv,
            list,
            dds_written: stats.dds_written,
        },
        stats,
    ))
}

struct LoadedHybridAtlasTile {
    texture: HybridAtlasTexture,
    group: HybridAtlasGroup,
    w: u32,
    h: u32,
    pack_w: u32,
    pack_h: u32,
    page_size: u32,
    rgba_d: Vec<u8>,
    rgba_n: Vec<u8>,
    rgba_s: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HybridAtlasGroup {
    Tree,
    Translucent,
    AlphaTested,
    Other,
}

struct HybridAtlasLoadResult {
    index: usize,
    tile: Option<LoadedHybridAtlasTile>,
    warning: Option<String>,
}

fn load_hybrid_atlas_tiles(
    settings: &LodSettings,
    textures: Vec<HybridAtlasTexture>,
    progress: &mut dyn Progress,
    stats: &mut SourceAtlasStats,
) -> anyhow::Result<Vec<LoadedHybridAtlasTile>> {
    let total = textures.len();
    let worker_count = crate::driver::effective_worker_count(settings.global.workers);
    let report_every = (total / 20).max(1);
    let mut results = Vec::<HybridAtlasLoadResult>::with_capacity(total);

    if worker_count > 1 && total > 1 {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| -> anyhow::Result<()> {
            let handle = scope.spawn(move || {
                let run = || {
                    textures.into_par_iter().enumerate().for_each_with(
                        tx.clone(),
                        |sender, (index, texture)| {
                            let result = load_one_hybrid_atlas_texture(settings, index, texture);
                            let _ = sender.send(result);
                        },
                    );
                };
                if let Some(pool) = crate::driver::build_worker_pool(settings.global.workers) {
                    pool.install(run);
                } else {
                    run();
                }
            });

            for done in 1..=total {
                let result = rx
                    .recv()
                    .context("hybrid-atlas texture loader worker stopped early")?;
                results.push(result);
                if done == total || done % report_every == 0 {
                    let kept = results.iter().filter(|r| r.tile.is_some()).count();
                    progress.report(
                        &format!(
                            "hybrid-atlas objects: loaded {done}/{total} atlas texture set(s), kept {kept}"
                        ),
                        0.35 + (done as f32 / total.max(1) as f32) * 0.25,
                    );
                }
            }

            handle
                .join()
                .map_err(|_| anyhow::anyhow!("hybrid-atlas texture loader worker panicked"))?;
            Ok(())
        })?;
    } else {
        for (index, texture) in textures.into_iter().enumerate() {
            results.push(load_one_hybrid_atlas_texture(settings, index, texture));
            let done = index + 1;
            if done == total || done % report_every == 0 {
                let kept = results.iter().filter(|r| r.tile.is_some()).count();
                progress.report(
                    &format!(
                        "hybrid-atlas objects: loaded {done}/{total} atlas texture set(s), kept {kept}"
                    ),
                    0.35 + (done as f32 / total.max(1) as f32) * 0.25,
                );
            }
        }
    }

    results.sort_by_key(|result| result.index);
    let mut loaded = Vec::<LoadedHybridAtlasTile>::new();
    for result in results {
        if let Some(warning) = result.warning {
            stats.scan_warnings.push(warning);
        }
        if let Some(tile) = result.tile {
            loaded.push(tile);
        }
    }
    Ok(loaded)
}

fn load_one_hybrid_atlas_texture(
    settings: &LodSettings,
    index: usize,
    texture: HybridAtlasTexture,
) -> HybridAtlasLoadResult {
    let diffuse_path = texture.diffuse_path.clone();
    let converted_bundle = load_converted_fo76_bundle_for_hybrid_atlas(&texture);
    let (source_w, source_h, source_rgba_d, converted_specular) =
        if let Some((w, h, rgba_d, rgba_s)) = converted_bundle {
            (w, h, rgba_d, Some(rgba_s))
        } else {
            let Ok(img) = directxtex_native::read_dds_rgba_image(&diffuse_path) else {
                return HybridAtlasLoadResult {
                    index,
                    tile: None,
                    warning: None,
                };
            };
            (img.width, img.height, img.rgba, None)
        };
    let page_size = hybrid_atlas_effective_page_size(settings, &texture);
    if source_w > page_size || source_h > page_size {
        return HybridAtlasLoadResult {
            index,
            tile: None,
            warning: Some(format!(
                "hybrid-atlas objects: skipping oversized atlas texture {} ({}x{})",
                diffuse_path.display(),
                source_w,
                source_h
            )),
        };
    }
    let max_tile_size = hybrid_atlas_effective_max_tile_size(settings, &texture);
    let (w, h, rgba_d) = if source_w > max_tile_size || source_h > max_tile_size {
        let w = source_w.min(max_tile_size);
        let h = source_h.min(max_tile_size);
        (
            w,
            h,
            crate::atlas::atlas::resize_rgba(&source_rgba_d, source_w, source_h, w, h),
        )
    } else {
        (source_w, source_h, source_rgba_d)
    };
    let effective_min_tile_size = hybrid_atlas_effective_min_tile_size(settings, &texture);
    let (target_w, target_h) = hybrid_atlas_content_size(w, h, effective_min_tile_size, page_size);
    let rgba_d = resize_rgba_buffer_if_needed(rgba_d, w, h, target_w, target_h);
    let w = target_w;
    let h = target_h;
    let (pack_w, pack_h) = hybrid_atlas_pack_size(w, h, effective_min_tile_size, page_size);
    let rgba_n = texture
        .normal_path
        .as_ref()
        .and_then(|p| directxtex_native::read_dds_rgba_image(p).ok())
        .map(|img| crate::atlas::atlas::maybe_resize_rgba(img, w, h))
        .unwrap_or_else(|| crate::atlas::atlas::flat_normal_rgba(w, h));
    let rgba_s = converted_specular
        .map(|rgba| resize_rgba_buffer_if_needed(rgba, source_w, source_h, w, h))
        .or_else(|| {
            texture
                .specular_path
                .as_ref()
                .and_then(|p| directxtex_native::read_dds_rgba_image(p).ok())
                .map(|img| crate::atlas::atlas::maybe_resize_rgba(img, w, h))
        })
        .unwrap_or_else(|| missing_specular_rgba(w, h));

    HybridAtlasLoadResult {
        index,
        tile: Some(LoadedHybridAtlasTile {
            group: hybrid_atlas_texture_group(&texture),
            texture,
            w,
            h,
            pack_w,
            pack_h,
            page_size,
            rgba_d,
            rgba_n,
            rgba_s,
        }),
        warning: None,
    }
}

fn hybrid_atlas_effective_min_tile_size(
    settings: &LodSettings,
    texture: &HybridAtlasTexture,
) -> u32 {
    let global_min = settings.objects.fo76_bto_atlas_min_tile_size;
    let foliage_min = settings.objects.fo76_bto_atlas_min_foliage_tile_size;
    let alpha_tested_min = settings.objects.fo76_bto_atlas_min_alpha_tested_tile_size;
    if alpha_tested_min > global_min && hybrid_atlas_texture_is_alpha_tested(texture) {
        alpha_tested_min
    } else if foliage_min > global_min && hybrid_atlas_texture_is_foliage(texture) {
        foliage_min
    } else {
        global_min
    }
}

fn hybrid_atlas_effective_page_size(settings: &LodSettings, texture: &HybridAtlasTexture) -> u32 {
    let global_page_size = settings.objects.atlas_size.max(1);
    let alpha_tested_page_size = settings.objects.fo76_bto_atlas_alpha_tested_page_size;
    if alpha_tested_page_size > 0 && hybrid_atlas_texture_is_alpha_tested(texture) {
        return alpha_tested_page_size.max(1);
    }
    let foliage_page_size = settings.objects.fo76_bto_atlas_foliage_page_size;
    if foliage_page_size > 0 && hybrid_atlas_texture_is_foliage(texture) {
        foliage_page_size.max(1)
    } else {
        global_page_size
    }
}

fn hybrid_atlas_effective_max_tile_size(
    settings: &LodSettings,
    texture: &HybridAtlasTexture,
) -> u32 {
    let global_max = settings.objects.max_tile_size.max(1);
    let alpha_tested_max = settings.objects.fo76_bto_atlas_alpha_tested_max_tile_size;
    if alpha_tested_max > 0 && hybrid_atlas_texture_is_alpha_tested(texture) {
        return alpha_tested_max.max(1);
    }
    let foliage_max = settings.objects.fo76_bto_atlas_foliage_max_tile_size;
    if foliage_max > 0 && hybrid_atlas_texture_is_foliage(texture) {
        foliage_max.max(1)
    } else {
        global_max
    }
}

fn hybrid_atlas_texture_is_foliage(texture: &HybridAtlasTexture) -> bool {
    matches!(
        hybrid_atlas_texture_group(texture),
        HybridAtlasGroup::Tree | HybridAtlasGroup::Translucent
    )
}

fn hybrid_atlas_texture_is_alpha_tested(texture: &HybridAtlasTexture) -> bool {
    hybrid_atlas_texture_group(texture) == HybridAtlasGroup::AlphaTested
}

fn hybrid_atlas_texture_group(texture: &HybridAtlasTexture) -> HybridAtlasGroup {
    let lower = texture.diffuse_rel.to_ascii_lowercase();
    if lower.contains(r"\lod\landscape\trees\") || lower.contains(r"\landscape\trees\") {
        HybridAtlasGroup::Tree
    } else if lower.contains(r"\texturearrays\translucent\") {
        HybridAtlasGroup::Translucent
    } else if lower.contains(r"\texturearrays\alphatested\") {
        HybridAtlasGroup::AlphaTested
    } else {
        HybridAtlasGroup::Other
    }
}

fn hybrid_atlas_content_size(w: u32, h: u32, min_tile_size: u32, atlas_size: u32) -> (u32, u32) {
    let min_tile_size = min_tile_size.min(atlas_size).max(1);
    if min_tile_size <= 1 || w.max(h) >= min_tile_size {
        return (w, h);
    }
    let max_dim = w.max(h).max(1);
    let new_w = scale_dim_to_min(w, max_dim, min_tile_size).min(atlas_size.max(1));
    let new_h = scale_dim_to_min(h, max_dim, min_tile_size).min(atlas_size.max(1));
    (new_w.max(1), new_h.max(1))
}

fn scale_dim_to_min(dim: u32, max_dim: u32, min_tile_size: u32) -> u32 {
    (((dim as u64) * (min_tile_size as u64) + (max_dim as u64 / 2)) / max_dim as u64) as u32
}

fn hybrid_atlas_pack_size(w: u32, h: u32, min_tile_size: u32, atlas_size: u32) -> (u32, u32) {
    let min_tile_size = min_tile_size.min(atlas_size).max(1);
    if min_tile_size <= 1 {
        return (w, h);
    }
    (w.max(min_tile_size), h.max(min_tile_size))
}

fn load_converted_fo76_bundle_for_hybrid_atlas(
    texture: &HybridAtlasTexture,
) -> Option<(u32, u32, Vec<u8>, Vec<u8>)> {
    if !has_fo76_bundle_suffixes(texture) {
        return None;
    }
    let reflectivity_path = texture.specular_path.as_ref()?;
    let lighting_path = texture.lighting_path.as_ref()?;
    let diffuse = directxtex_native::read_dds_float_rgba_image(&texture.diffuse_path).ok()?;
    let reflectivity = directxtex_native::read_dds_float_rgba_image(reflectivity_path).ok()?;
    let lighting = directxtex_native::read_dds_float_rgba_image(lighting_path).ok()?;
    let outputs = materials_native::texture_convert::fo76_bundle_to_fo4_buffers(
        &materials_native::texture_convert::f32_vec_to_bytes(&diffuse.rgba),
        &materials_native::texture_convert::f32_vec_to_bytes(&reflectivity.rgba),
        &materials_native::texture_convert::f32_vec_to_bytes(&lighting.rgba),
        diffuse.width as usize,
        diffuse.height as usize,
        reflectivity.width as usize,
        reflectivity.height as usize,
        lighting.width as usize,
        lighting.height as usize,
        materials_native::texture_convert::TextureConversionParams::default(),
        false,
    )
    .ok()?;
    Some((
        diffuse.width,
        diffuse.height,
        rgba_f32_to_u8(&outputs.diffuse),
        rgba_f32_to_u8(&outputs.specgloss),
    ))
}

fn has_fo76_bundle_suffixes(texture: &HybridAtlasTexture) -> bool {
    texture.diffuse_rel.to_ascii_lowercase().ends_with("_d.dds")
        && texture
            .specular_rel
            .to_ascii_lowercase()
            .ends_with("_r.dds")
        && texture
            .lighting_rel
            .to_ascii_lowercase()
            .ends_with("_l.dds")
}

fn rgba_f32_to_u8(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .map(|value| ((*value).clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect()
}

fn resize_rgba_buffer_if_needed(
    rgba: Vec<u8>,
    source_w: u32,
    source_h: u32,
    target_w: u32,
    target_h: u32,
) -> Vec<u8> {
    if source_w == target_w && source_h == target_h {
        rgba
    } else {
        crate::atlas::atlas::resize_rgba(&rgba, source_w, source_h, target_w, target_h)
    }
}

fn write_source_bto_tree_list(
    world: &WorldspaceInput,
    paths: &LodPaths,
    manifest: &BillboardManifest,
    placed_indices: &std::collections::BTreeSet<i32>,
) -> anyhow::Result<()> {
    use crate::output::btt::{LstEntry, write_tree_list};
    let entries: Vec<LstEntry> = manifest
        .entries
        .iter()
        .filter(|entry| placed_indices.contains(&entry.index))
        .map(|entry| LstEntry {
            index: entry.index,
            width: entry.width,
            height: entry.height,
            uv_min_x: entry.uv_min_x,
            uv_max_x: entry.uv_max_x,
            uv_min_y: entry.uv_min_y,
            uv_max_y: entry.uv_max_y,
        })
        .collect();
    if entries.is_empty() {
        return Ok(());
    }
    let rel = crate::naming::tree_list(&world.editor_id);
    let path = paths.output_dir.join(rel.replace('\\', "/"));
    write_tree_list(&path, &entries)
}

fn missing_specular_rgba(w: u32, h: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for px in rgba.chunks_exact_mut(4) {
        px[3] = 255;
    }
    rgba
}

fn source_bto_atlas_page_rel(world: &str, page: usize) -> String {
    let rel = crate::naming::object_atlas(world);
    if page == 0 {
        return rel;
    }
    let lower = rel.to_ascii_lowercase();
    if let Some(pos) = lower.rfind(".dds") {
        format!("{}.{page:03}{}", &rel[..pos], &rel[pos..])
    } else {
        format!("{rel}.{page:03}.dds")
    }
}

fn object_format_to_str(f: &crate::settings::Format) -> &'static str {
    use crate::settings::Format;
    match f {
        Format::Bc1 => "BC1_UNORM",
        Format::Bc2 => "BC2_UNORM",
        Format::Bc3 => "BC3_UNORM",
        Format::Bc5 => "BC5_UNORM",
        Format::Bc7 => "BC7_UNORM",
        Format::Rgba8 => "R8G8B8A8_UNORM",
        Format::Bgr565 => "BC1_UNORM",
    }
}

fn paths_with_source_root(paths: &LodPaths, source_root: &Path) -> LodPaths {
    let mut local_paths = paths.clone();
    if !local_paths
        .data_dirs
        .iter()
        .any(|p| same_path_text(p, source_root))
    {
        local_paths.data_dirs.insert(0, source_root.to_path_buf());
    }
    local_paths
}

fn extension_eq(path: &Path, ext: &str) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(ext))
}

pub fn enumerate_source_bto_tiles(
    source_root: &Path,
    world_editor_id: &str,
) -> anyhow::Result<Vec<SourceBtoTile>> {
    let objects_dir = source_objects_dir(source_root, world_editor_id);
    if !objects_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut tiles = Vec::new();
    for entry in std::fs::read_dir(&objects_dir)
        .with_context(|| format!("read source BTO dir {}", objects_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file()
            || !path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("bto"))
        {
            continue;
        }
        if let Some(tile) = parse_source_bto_filename(&path) {
            if tile.world.eq_ignore_ascii_case(world_editor_id) {
                tiles.push(tile);
            }
        }
    }
    tiles.sort_by(|a, b| {
        a.level
            .cmp(&b.level)
            .then_with(|| a.x.cmp(&b.x))
            .then_with(|| a.y.cmp(&b.y))
    });
    Ok(tiles)
}

pub fn parse_source_bto_filename(path: &Path) -> Option<SourceBtoTile> {
    let stem = path.file_stem()?.to_string_lossy();
    let parts: Vec<&str> = stem.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    Some(SourceBtoTile {
        world: parts[0].to_string(),
        level: parts[1].parse().ok()?,
        x: parts[2].parse().ok()?,
        y: parts[3].parse().ok()?,
        path: path.to_path_buf(),
    })
}

pub fn resource_id_from_path(path: &str) -> ResourceId {
    let rid = materials_native::cdb::resource_id_from_path(path);
    ResourceId {
        dir: rid.dir,
        file: rid.file,
        ext: rid.ext,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceBtoTreeBillboardSpecies {
    pub model: String,
    pub resolved_path: PathBuf,
    pub instance_count: u64,
}

pub fn collect_tree_billboard_species(
    source_root: &Path,
    world_editor_id: &str,
    settings: &LodSettings,
) -> anyhow::Result<Vec<SourceBtoTreeBillboardSpecies>> {
    let Some(min_lod) = settings.objects.fo76_bto_tree_billboard_from_lod else {
        return Ok(Vec::new());
    };
    let tiles: Vec<SourceBtoTile> = enumerate_source_bto_tiles(source_root, world_editor_id)?
        .into_iter()
        .filter(|tile| tile.level >= min_lod && tile_allowed(tile, settings))
        .collect();
    if tiles.is_empty() {
        return Ok(Vec::new());
    }

    let resolver = ResourceResolver::build(source_root)?;
    let scan_one =
        |tile: &SourceBtoTile| source_bto_tree_instance_models(tile, &resolver).unwrap_or_default();
    let maps: Vec<BTreeMap<String, u64>> =
        if let Some(pool) = crate::driver::build_worker_pool(settings.global.workers) {
            pool.install(|| tiles.par_iter().map(scan_one).collect())
        } else {
            tiles.par_iter().map(scan_one).collect()
        };

    let mut counts = BTreeMap::<String, u64>::new();
    for map in maps {
        for (model, count) in map {
            *counts.entry(model).or_default() += count;
        }
    }

    let mut species = Vec::new();
    for (model, instance_count) in counts {
        let Some(resolved_path) = resolve_source_data_path_ci(source_root, &model) else {
            continue;
        };
        species.push(SourceBtoTreeBillboardSpecies {
            model,
            resolved_path,
            instance_count,
        });
    }
    species.sort_by(|a, b| {
        a.model
            .to_ascii_lowercase()
            .cmp(&b.model.to_ascii_lowercase())
    });
    Ok(species)
}

fn source_bto_tree_instance_models(
    tile: &SourceBtoTile,
    resolver: &ResourceResolver,
) -> anyhow::Result<BTreeMap<String, u64>> {
    let nif = NifFile::load(&tile.path).map_err(|e| anyhow::anyhow!("read source BTO: {e:?}"))?;
    let mut out = BTreeMap::<String, u64>::new();
    for block in &nif.blocks {
        if block.type_name != "BSDistantObjectInstancedNode" {
            continue;
        }
        for inst_value in val_array(block.get_field("Instances")) {
            let Some(inst) = as_struct(Some(inst_value)) else {
                continue;
            };
            let Some(rid) = inst.get("Resource ID").and_then(resource_id_from_value) else {
                continue;
            };
            let Some(model) = resolver.resolve(&rid) else {
                continue;
            };
            if !is_tree_instance_model(model) {
                continue;
            }
            let count = val_array(inst.get("Transforms")).len() as u64;
            if count > 0 {
                *out.entry(model.to_string()).or_default() += count;
            }
        }
    }
    Ok(out)
}

pub fn split_texture_array_triangles(geometry: &LodGeometry) -> BTreeMap<i32, LodGeometry> {
    let mut grouped: BTreeMap<i32, Vec<[u32; 3]>> = BTreeMap::new();
    for tri in &geometry.triangles {
        let slices = [
            vertex_slice(geometry, tri[0]),
            vertex_slice(geometry, tri[1]),
            vertex_slice(geometry, tri[2]),
        ];
        let slice = if slices[0] == slices[1] && slices[1] == slices[2] {
            slices[0]
        } else {
            ((slices[0] + slices[1] + slices[2]) as f32 / 3.0).round() as i32
        };
        grouped.entry(slice).or_default().push(*tri);
    }

    grouped
        .into_iter()
        .filter_map(|(slice, tris)| {
            let mut geom = remap_geometry_for_triangles(geometry, &tris);
            geom.tangents.clear();
            geom.bitangents.clear();
            (geom.num_triangles() > 0).then_some((slice, geom))
        })
        .collect()
}

fn convert_tile(
    world: &WorldspaceInput,
    settings: &LodSettings,
    game: &Game,
    paths: &LodPaths,
    source_root: &Path,
    resolver: &ResourceResolver,
    tile: &SourceBtoTile,
    copied_textures: &Mutex<HashSet<PathBuf>>,
    mode: SourceBtoMode,
    atlas: &AtlasResult,
    billboard_manifest: Option<&BillboardManifest>,
    standard_tree_tiles: &TreeTileIndex,
) -> anyhow::Result<SourceTileStats> {
    let total_started = Instant::now();
    let load_started = Instant::now();
    let nif = NifFile::load(&tile.path).map_err(|e| anyhow::anyhow!("read source BTO: {e:?}"))?;
    let load_secs = load_started.elapsed().as_secs_f64();
    let quad = source_quad(tile);
    let local_paths = paths_with_source_root(paths, source_root);
    let ctx = QuadCtx {
        world,
        settings,
        game,
        paths: &local_paths,
        level: tile.level,
    };

    let mut copied = TextureSink::new(
        source_root,
        &paths.output_dir,
        &world.editor_id,
        copied_textures,
    );
    let mut shapes = Vec::new();
    let mut tree_refs = Vec::<StaticDesc>::new();
    let mut tile_stats = SourceTileStats::default();
    tile_stats.load_secs = load_secs;
    let standard_tree_refs = if mode.is_atlas() && settings.trees.trees_3d {
        standard_tree_refs_for_tile(world, standard_tree_tiles, tile)
    } else {
        Vec::new()
    };
    let tree_ctx = QuadCtx {
        world,
        settings,
        game,
        paths,
        level: tile.level,
    };
    let tree_started = Instant::now();
    let generated_tree_shapes = if !standard_tree_refs.is_empty() {
        crate::trees::tree3d::collect_quad_shapes(&quad, &tree_ctx, atlas, &standard_tree_refs)
    } else {
        Vec::new()
    };
    tile_stats.tree_secs = tree_started.elapsed().as_secs_f64();
    let replace_source_translucent_foliage = !generated_tree_shapes.is_empty();

    let extract_started = Instant::now();
    for block in &nif.blocks {
        match block.type_name.as_str() {
            "BSSubIndexTriShape" | "BSTriShape"
                if should_extract_baked_block(
                    settings,
                    block,
                    mode,
                    replace_source_translucent_foliage,
                ) =>
            {
                shapes.extend(extract_baked_shapes(
                    &nif,
                    block,
                    &quad,
                    tile,
                    mode,
                    atlas,
                    &mut copied,
                ));
            }
            "BSDistantObjectInstancedNode" if settings.objects.fo76_bto_include_instances => {
                let extracted = extract_instance_shapes(
                    &nif,
                    block,
                    &quad,
                    tile,
                    resolver,
                    &ctx,
                    mode,
                    atlas,
                    billboard_manifest,
                    &mut copied,
                );
                shapes.extend(extracted.shapes);
                tree_refs.extend(extracted.tree_refs);
                tile_stats.tree_instances_to_btt += extracted.tree_instances_to_btt;
                tile_stats.far_tree_3d_fallback += extracted.far_tree_3d_fallback;
                for (model, count) in extracted.skipped_missing_billboard {
                    *tile_stats
                        .skipped_missing_billboard
                        .entry(model)
                        .or_default() += count;
                }
            }
            _ => {}
        }
    }
    shapes.extend(generated_tree_shapes);
    tile_stats.extract_secs = extract_started.elapsed().as_secs_f64();

    tile_stats.input_shapes = shapes.len() as u64;
    let write_started = Instant::now();
    let outputs = if mode.is_atlas() {
        crate::objects::write_atlassed_source_bto_shapes(&quad, &ctx, shapes)?
    } else {
        crate::objects::write_source_bto_shapes(&quad, &ctx, shapes)?
    };
    tile_stats.write_secs = write_started.elapsed().as_secs_f64();
    tile_stats.output_shapes = outputs
        .object_lod
        .as_ref()
        .map(|t| t.output_shape_count)
        .unwrap_or(0);
    tile_stats.stats.bto += outputs
        .meshes
        .iter()
        .filter(|p| extension_eq(p, "bto"))
        .count() as u32;
    tile_stats.stats.dds += copied.dds_written;

    if !tree_refs.is_empty() {
        let btt_started = Instant::now();
        let tree_ref_ptrs: Vec<&StaticDesc> = tree_refs.iter().collect();
        let tree_outputs = crate::trees::billboard_place::generate_quad(
            &quad,
            &ctx,
            atlas,
            &tree_ref_ptrs,
            billboard_manifest,
        )?;
        tile_stats.stats.btt += tree_outputs
            .meshes
            .iter()
            .filter(|p| extension_eq(p, "btt"))
            .count() as u32;
        if let Some(manifest) = billboard_manifest {
            for stat in tree_refs {
                let model = stat.lod_models[level_index(tile.level)]
                    .as_deref()
                    .unwrap_or(&stat.full_model);
                if let Some(entry) = manifest.by_model(model) {
                    tile_stats.placed_tree_indices.insert(entry.index);
                }
            }
        }
        tile_stats.btt_secs = btt_started.elapsed().as_secs_f64();
    }

    tile_stats.total_secs = total_started.elapsed().as_secs_f64();
    Ok(tile_stats)
}

fn should_extract_baked_block(
    settings: &LodSettings,
    block: &NifBlock,
    mode: SourceBtoMode,
    replace_source_translucent_foliage: bool,
) -> bool {
    if !settings.objects.fo76_bto_include_baked {
        return false;
    }
    let name = block_name(block);
    if mode.is_atlas()
        && replace_source_translucent_foliage
        && name.eq_ignore_ascii_case("GlobalAtlasShape_Translucent")
    {
        return false;
    }
    if name.starts_with("GlobalAtlasShape") {
        return settings.objects.fo76_bto_include_global_atlas_baked;
    }
    if name.starts_with("RemeshedShape") {
        return settings.objects.fo76_bto_include_remeshed_baked;
    }
    true
}

#[derive(Default)]
struct InstanceExtract {
    shapes: Vec<ShapeDesc>,
    tree_refs: Vec<StaticDesc>,
    tree_instances_to_btt: u64,
    far_tree_3d_fallback: u64,
    skipped_missing_billboard: BTreeMap<String, u64>,
}

fn extract_instance_shapes(
    nif: &NifFile,
    node: &NifBlock,
    quad: &QuadDesc,
    tile: &SourceBtoTile,
    resolver: &ResourceResolver,
    ctx: &QuadCtx<'_>,
    mode: SourceBtoMode,
    atlas: &AtlasResult,
    billboard_manifest: Option<&BillboardManifest>,
    textures: &mut TextureSink<'_>,
) -> InstanceExtract {
    let node_translation = vec3(node.get_field("Translation")).unwrap_or([0.0, 0.0, 0.0]);
    let level_index = level_index(tile.level);
    let mut out = InstanceExtract::default();
    let instances = val_array(node.get_field("Instances"));
    for (inst_idx, inst_value) in instances.iter().enumerate() {
        let Some(inst) = as_struct(Some(inst_value)) else {
            continue;
        };
        let Some(rid) = inst.get("Resource ID").and_then(resource_id_from_value) else {
            continue;
        };
        let Some(model) = resolver.resolve(&rid) else {
            eprintln!(
                "[lodgen] hybrid source BTO: unresolved resource id dir={} file={} ext={} in {}",
                rid.dir,
                rid.file,
                rid.ext,
                tile.path.display()
            );
            continue;
        };
        let is_tree = is_tree_instance_model(model);
        if !ctx.settings.objects.fo76_bto_include_tree_instances && is_tree {
            continue;
        }
        let far_tree_billboard = is_tree
            && mode.is_atlas()
            && ctx
                .settings
                .objects
                .fo76_bto_tree_billboard_from_lod
                .map(|min_lod| tile.level >= min_lod)
                .unwrap_or(false);
        for (tx_idx, matrix_value) in val_array(inst.get("Transforms")).iter().enumerate() {
            let Some(matrix) = mat4(matrix_value) else {
                continue;
            };
            let stat = instance_static_desc(
                model,
                level_index,
                node_translation,
                matrix,
                inst_idx,
                tx_idx,
            );
            if far_tree_billboard {
                if billboard_manifest
                    .and_then(|manifest| manifest.by_model(model))
                    .is_some()
                {
                    let mut tree_stat = stat.clone();
                    tree_stat.scale = stat.part_scale;
                    out.tree_refs.push(tree_stat);
                    out.tree_instances_to_btt += 1;
                    continue;
                } else {
                    out.far_tree_3d_fallback += 1;
                }
            }
            let parsed = match crate::objects::parse_nif::parse_nif(&stat, level_index, ctx) {
                Ok(shapes) => shapes,
                Err(err) => {
                    eprintln!(
                        "[lodgen] hybrid source BTO: skipping resource {} in {}: {err}",
                        model,
                        tile.path.display()
                    );
                    continue;
                }
            };
            for mut shape in parsed {
                if mode.is_atlas() {
                    wrap_source_bto_repeating_uvs_for_atlas(&mut shape);
                }
                if transform_shape_with_world(
                    quad,
                    &stat,
                    &mut shape,
                    &atlas.list,
                    &ctx.settings.objects,
                    ctx.world,
                ) {
                    if mode.is_atlas() {
                        if shape_uses_hybrid_atlas_or_safe(&shape, &ctx.world.editor_id) {
                            out.shapes.push(shape);
                        }
                    } else {
                        rewrite_shape_textures(&mut shape, textures);
                        out.shapes.push(shape);
                    }
                }
            }
        }
    }
    let _ = nif;
    out
}

fn extract_baked_shapes(
    nif: &NifFile,
    geom_block: &NifBlock,
    quad: &QuadDesc,
    tile: &SourceBtoTile,
    mode: SourceBtoMode,
    atlas: &AtlasResult,
    textures: &mut TextureSink<'_>,
) -> Vec<ShapeDesc> {
    let Some(source_geometry) = extract_inline_geometry(geom_block) else {
        return Vec::new();
    };
    if source_geometry.uvcoords.is_empty() || source_geometry.triangles.is_empty() {
        return Vec::new();
    }
    let name = block_name(geom_block);
    if name.is_empty() {
        return Vec::new();
    }

    let shader_ref = val_ref(geom_block.get_field("Shader Property")).unwrap_or(-1);
    let Some(shader) = block_of(nif, shader_ref) else {
        return Vec::new();
    };
    if shader.type_name != "BSLightingShaderProperty" {
        return Vec::new();
    }

    let alpha = alpha_info(nif, geom_block);
    let clamp = shader_clamp_mode(shader);
    let source_segment_id = source_segment_id(geom_block);
    let bto_translation = vec3(geom_block.get_field("Translation")).unwrap_or([
        tile.x as f32 * 4096.0,
        tile.y as f32 * 4096.0,
        0.0,
    ]);
    let bto_scale = val_f32(geom_block.get_field("Scale")).unwrap_or(tile.level as f32);
    let mut out = Vec::new();

    if let Some(arrays) = shader_texture_arrays(shader) {
        for (slice, geometry) in split_texture_array_triangles(&source_geometry) {
            let Some(texture_set) = texture_set_for_array_slice(&arrays, slice, mode, textures)
            else {
                continue;
            };
            let mut shape = make_baked_shape(
                name.clone(),
                tile,
                quad,
                geometry,
                texture_set,
                alpha,
                clamp,
                source_segment_id,
                bto_translation,
                bto_scale,
            );
            if !mode.is_atlas() || remap_baked_shape_to_atlas(&mut shape, atlas) {
                out.push(shape);
            }
        }
        return out;
    }

    if let Some(mut texture_set) = shader_texture_set(nif, shader) {
        rewrite_texture_set(&mut texture_set, mode, textures);
        let mut shape = make_baked_shape(
            name,
            tile,
            quad,
            source_geometry,
            texture_set,
            alpha,
            clamp,
            source_segment_id,
            bto_translation,
            bto_scale,
        );
        if !mode.is_atlas() || remap_baked_shape_to_atlas(&mut shape, atlas) {
            out.push(shape);
        }
    }

    out
}

fn make_baked_shape(
    name: String,
    tile: &SourceBtoTile,
    quad: &QuadDesc,
    mut geometry: LodGeometry,
    textures: [String; 10],
    alpha: Option<(u16, u8)>,
    texture_clamp_mode: u32,
    source_segment_id: Option<i32>,
    bto_translation: [f32; 3],
    bto_scale: f32,
) -> ShapeDesc {
    geometry.update_bbox();
    let center = geometry.bbox.center(false);
    let (segment_x, segment_y) = source_segment_id
        .and_then(|id| segment_position_from_source_id(quad, id))
        .unwrap_or((center[0] * bto_scale, center[1] * bto_scale));
    let mut flags = ShapeFlags::HAS_LOD_FLAG;
    if geometry.has_vertex_colors() {
        flags |= ShapeFlags::HAS_VERTEX_COLOR;
    }
    if alpha.is_some() || baked_shape_name_implies_alpha(&name) {
        flags |= ShapeFlags::IS_ALPHA;
    }
    let num_triangles = geometry.num_triangles().min(u16::MAX as usize) as u16;
    ShapeDesc {
        name,
        static_model: tile.path.to_string_lossy().to_ascii_lowercase(),
        geometry,
        flags,
        textures,
        source_materials: Vec::new(),
        textures_key: String::new(),
        texture_clamp_mode,
        alpha_threshold: alpha.map(|(_, threshold)| threshold).unwrap_or(128),
        alpha_flags: alpha.map(|(flags, _)| flags).unwrap_or(4844),
        backlight_power: 0.0,
        grayscale_to_palette_scale: 1.0,
        enable_parent: 0,
        shader_type: ShaderKind::Lighting,
        x: segment_x,
        y: segment_y,
        bounding_box: BBox::empty(),
        segments: generate_segments(quad, segment_x, segment_y, num_triangles),
        uv_scale: [1.0, 1.0],
        uv_offset: [0.0, 0.0],
        ref_flags: 0,
        node_transform: identity_part_transform(),
        node_scale: 1.0,
        translation: [0.0, 0.0, 0.0],
        rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        bto_translation: Some(bto_translation),
        bto_scale: Some(bto_scale),
    }
}

fn source_segment_id(geom_block: &NifBlock) -> Option<i32> {
    let mut found = None;
    for (idx, segment) in val_array(geom_block.get_field("Segment"))
        .iter()
        .enumerate()
    {
        let Some(fields) = as_struct(Some(segment)) else {
            continue;
        };
        if val_u64(fields.get("Num Primitives")).unwrap_or(0) == 0 {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(idx as i32);
    }
    found
}

fn segment_position_from_source_id(quad: &QuadDesc, id: i32) -> Option<(f32, f32)> {
    if id < 0 || quad.quad_level <= 0 {
        return None;
    }
    let count = quad.quad_level;
    if id >= count * count {
        return None;
    }
    let cell_size = quad.quad_offset / count as f32;
    Some((
        (id / count) as f32 * cell_size + cell_size * 0.5,
        (id % count) as f32 * cell_size + cell_size * 0.5,
    ))
}

fn baked_shape_name_implies_alpha(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    normalized.contains("translucent")
        || (normalized.contains("alphatested") && !normalized.contains("notalphatested"))
}

fn instance_static_desc(
    model: &str,
    level_index: usize,
    node_translation: [f32; 3],
    matrix: [[f32; 4]; 4],
    inst_idx: usize,
    tx_idx: usize,
) -> StaticDesc {
    let mut lod_models: [Option<String>; 4] = [None, None, None, None];
    lod_models[level_index] = Some(model.to_string());
    let instance_pos = [
        node_translation[0] + matrix[0][3],
        node_translation[1] + matrix[1][3],
        node_translation[2] + matrix[2][3],
    ];
    let mut part_transform = matrix;
    part_transform[0][3] = 0.0;
    part_transform[1][3] = 0.0;
    part_transform[2][3] = 0.0;
    part_transform[3] = [0.0, 0.0, 0.0, 1.0];
    let part_scale = matrix[3][3].abs().max(0.001);
    let is_tree = is_tree_instance_model(model);
    let ref_hash = fnv1a_32(
        format!(
            "{}:{inst_idx}:{tx_idx}:{:.3}:{:.3}:{:.3}",
            model, instance_pos[0], instance_pos[1], instance_pos[2]
        )
        .as_bytes(),
    );
    StaticDesc {
        ref_id: format!("{ref_hash:08X}"),
        ref_flags: 0,
        enable_parent: 0,
        cell: (
            (instance_pos[0] / 4096.0).floor() as i32,
            (instance_pos[1] / 4096.0).floor() as i32,
        ),
        pos: instance_pos,
        rot: [0.0, 0.0, 0.0],
        scale: 1.0,
        color: 1.0,
        alpha_threshold: 128,
        is_billboard: false,
        is_grass: false,
        base_name: String::new(),
        base_flags: if is_tree { 0x40 } else { 0 },
        material_name: String::new(),
        full_model: String::new(),
        lod_models,
        part_transform,
        part_scale,
        material_swap: Default::default(),
    }
}

fn is_tree_instance_model(model: &str) -> bool {
    let normalized = model.to_ascii_lowercase().replace('/', "\\");
    let in_tree_lod_dir = normalized.contains("\\lod\\landscape\\trees\\")
        || normalized.contains("\\lod\\landscape\\swamp\\")
        || normalized.contains("\\landscape\\trees\\");
    if !in_tree_lod_dir {
        return false;
    }

    let stem = normalized
        .rsplit('\\')
        .next()
        .unwrap_or(&normalized)
        .split('.')
        .next()
        .unwrap_or(&normalized);

    const NON_TREE_OBJECT_TOKENS: &[&str] = &[
        "pole",
        "power",
        "wire",
        "utility",
        "telephone",
        "pylon",
        "streetlamp",
        "lamp",
        "cable",
        "tower",
        "antenna",
        "sign",
        "fence",
    ];
    if NON_TREE_OBJECT_TOKENS
        .iter()
        .any(|token| normalized.contains(token) || stem.contains(token))
    {
        return false;
    }

    const NON_UPRIGHT_TREE_TOKENS: &[&str] = &[
        "stump",
        "log",
        "fallen",
        "deadfall",
        "branch",
        "roots",
        "root",
        "driftwood",
        "cutlog",
        "pile",
        "marker",
    ];
    !NON_UPRIGHT_TREE_TOKENS
        .iter()
        .any(|token| stem.contains(token))
}

fn rewrite_shape_textures(shape: &mut ShapeDesc, textures: &mut TextureSink<'_>) {
    let mut rewritten: [String; 10] = Default::default();
    for (idx, texture) in shape.textures.iter().enumerate() {
        if texture.is_empty() {
            continue;
        }
        let specular = idx == 7 || idx == 9;
        let target_idx = if idx == 9 { 7 } else { idx };
        if target_idx >= rewritten.len() {
            continue;
        }
        rewritten[target_idx] = textures.rewrite_slot(texture, specular);
    }
    shape.textures = rewritten;
}

fn rewrite_texture_set(
    texture_set: &mut [String; 10],
    mode: SourceBtoMode,
    textures: &mut TextureSink<'_>,
) {
    let source = texture_set.clone();
    *texture_set = Default::default();
    for (idx, texture) in source.iter().enumerate() {
        if texture.is_empty() {
            continue;
        }
        let specular = idx == 7 || idx == 9;
        let target_idx = if idx == 9 { 7 } else { idx };
        if target_idx >= texture_set.len() {
            continue;
        }
        texture_set[target_idx] = if mode.is_atlas() {
            normalize_texture_rel(texture)
        } else {
            textures.rewrite_slot(texture, specular)
        };
    }
}

fn texture_set_for_array_slice(
    arrays: &[Vec<String>],
    slice: i32,
    mode: SourceBtoMode,
    textures: &mut TextureSink<'_>,
) -> Option<[String; 10]> {
    let slice = usize::try_from(slice).ok()?;
    let diffuse = arrays.get(0)?.get(slice)?;
    if diffuse.is_empty() {
        return None;
    }
    let mut texture_set: [String; 10] = Default::default();
    texture_set[0] = if mode.is_atlas() {
        normalize_texture_rel(diffuse)
    } else {
        textures.rewrite_slot(diffuse, false)
    };
    if let Some(normal) = arrays.get(1).and_then(|v| v.get(slice)) {
        texture_set[1] = if mode.is_atlas() {
            normalize_texture_rel(normal)
        } else {
            textures.rewrite_slot(normal, false)
        };
    }
    if mode.is_atlas()
        && let Some(lighting) = arrays.get(10).and_then(|v| v.get(slice))
    {
        texture_set[2] = normalize_texture_rel(lighting);
    }
    if let Some(specular) = arrays.get(9).and_then(|v| v.get(slice)) {
        texture_set[7] = if mode.is_atlas() {
            normalize_texture_rel(specular)
        } else {
            textures.rewrite_slot(specular, true)
        };
    }
    Some(texture_set)
}

fn remap_baked_shape_to_atlas(shape: &mut ShapeDesc, atlas: &AtlasResult) -> bool {
    let textures_key = atlas_build_key(&atlas.list, &shape.textures[..3], shape.alpha_threshold);
    shape.textures_key = textures_key.clone();
    let Some(rect) = atlas.list.get(&textures_key).cloned() else {
        return shape_uses_safe_non_atlas_texture(shape);
    };

    wrap_source_bto_repeating_uvs_for_atlas(shape);

    if !shape.geometry.uvcoords.is_empty() {
        let uvcoords = shape
            .geometry
            .uvcoords
            .iter()
            .map(|uv| {
                let (u, v) = rect.uv_atlas(uv[0], uv[1]);
                [u, v]
            })
            .collect();
        shape.geometry.set_uvcoords(uvcoords);
    }

    let diffuse = shape.textures[0].clone();
    let normal = shape.textures[1].clone();
    let specular = shape.textures[7].clone();
    let mut new_slots: [String; 10] = Default::default();
    for (idx, texture) in shape.textures.iter().enumerate() {
        if texture.is_empty() {
            continue;
        }
        if texture.eq_ignore_ascii_case(&diffuse) {
            new_slots[idx] = rect.atlas_diffuse.clone();
        } else if !normal.is_empty() && texture.eq_ignore_ascii_case(&normal) {
            new_slots[idx] = rect.atlas_normal.clone();
        } else if !specular.is_empty() && texture.eq_ignore_ascii_case(&specular) {
            new_slots[idx] = rect.atlas_specular.clone();
        } else if is_atlas_sentinel_texture(texture) {
            new_slots[idx] = texture.clone();
        }
    }
    if shape.flags.contains(ShapeFlags::IS_GREYSCALE_TO_PALETTE)
        || shape.flags.contains(ShapeFlags::IS_GREYSCALE_TO_ALPHA)
    {
        new_slots[3] = shape.textures[3].clone();
    }
    shape.textures = new_slots;
    shape.textures_key = atlas_build_key(&atlas.list, &shape.textures[..3], shape.alpha_threshold);
    shape.texture_clamp_mode = 0;
    true
}

fn wrap_source_bto_repeating_uvs_for_atlas(shape: &mut ShapeDesc) {
    if shape.texture_clamp_mode != 3 || shape.geometry.uvcoords.is_empty() {
        return;
    }
    let uvcoords = shape
        .geometry
        .uvcoords
        .iter()
        .map(|uv| [wrap_repeating_uv(uv[0]), wrap_repeating_uv(uv[1])])
        .collect();
    shape.geometry.set_uvcoords(uvcoords);
}

fn wrap_repeating_uv(value: f32) -> f32 {
    if (0.0..=1.0).contains(&value) {
        value
    } else {
        value.rem_euclid(1.0)
    }
}

fn shape_uses_hybrid_atlas_or_safe(shape: &ShapeDesc, world: &str) -> bool {
    let diffuse = normalize_texture_rel(&shape.textures[0]);
    if diffuse.is_empty() {
        return true;
    }
    let lower = diffuse.to_ascii_lowercase();
    let world = world.to_ascii_lowercase();
    lower.starts_with(&format!("textures\\terrain\\{world}\\objects\\"))
        || is_fo4_shared_texture(&lower)
        || is_atlas_sentinel_texture(&diffuse)
}

fn shape_uses_safe_non_atlas_texture(shape: &ShapeDesc) -> bool {
    let diffuse = normalize_texture_rel(&shape.textures[0]);
    diffuse.is_empty() || is_fo4_shared_texture(&diffuse) || is_atlas_sentinel_texture(&diffuse)
}

fn is_atlas_sentinel_texture(texture: &str) -> bool {
    const SENTINELS: [&str; 9] = [
        "textures\\white.dds",
        "textures\\gray.dds",
        "textures\\grey.dds",
        "textures\\black.dds",
        "textures\\brightyellow.dds",
        "textures\\default_n.dds",
        "textures\\shared\\flatwhite01_d.dds",
        "textures\\shared\\flatflat_n.dds",
        "textures\\shared\\white01_s.dds",
    ];
    let normalized = normalize_texture_rel(texture).to_ascii_lowercase();
    SENTINELS.iter().any(|s| normalized.eq_ignore_ascii_case(s))
}

fn shader_texture_arrays(shader: &NifBlock) -> Option<Vec<Vec<String>>> {
    let data = as_struct(shader.get_field("Shader Property Data"))?;
    if val_u64(data.get("Has Texture Arrays")).unwrap_or(0) == 0 {
        return None;
    }
    let mut arrays = Vec::new();
    for entry in val_array(data.get("Texture Arrays")) {
        let Some(entry_fields) = as_struct(Some(entry)) else {
            arrays.push(Vec::new());
            continue;
        };
        arrays.push(
            val_array(entry_fields.get("Texture Array"))
                .iter()
                .filter_map(|v| val_string(Some(v)))
                .collect(),
        );
    }
    Some(arrays)
}

fn shader_texture_set(nif: &NifFile, shader: &NifBlock) -> Option<[String; 10]> {
    let data_ref = as_struct(shader.get_field("Shader Property Data"))
        .and_then(|data| val_ref(data.get("Texture Set")))
        .or_else(|| val_ref(shader.get_field("Texture Set")))?;
    let texture_block = block_of(nif, data_ref)?;
    if texture_block.type_name != "BSShaderTextureSet" {
        return None;
    }
    let mut out: [String; 10] = Default::default();
    for (idx, value) in val_array(texture_block.get_field("Textures"))
        .iter()
        .enumerate()
    {
        if idx >= 10 {
            break;
        }
        out[idx] = val_string(Some(value))
            .unwrap_or_default()
            .replace('/', "\\")
            .to_ascii_lowercase();
    }
    if out[7].is_empty() {
        if let Some(spec) = val_array(texture_block.get_field("Textures"))
            .get(9)
            .and_then(|v| val_string(Some(v)))
        {
            out[7] = spec.replace('/', "\\").to_ascii_lowercase();
        }
    }
    Some(out)
}

fn alpha_info(nif: &NifFile, geom_block: &NifBlock) -> Option<(u16, u8)> {
    let alpha_ref = val_ref(geom_block.get_field("Alpha Property")).unwrap_or(-1);
    let alpha = block_of(nif, alpha_ref)?;
    if alpha.type_name != "NiAlphaProperty" {
        return None;
    }
    Some((
        val_u64(alpha.get_field("Flags")).unwrap_or(4844) as u16,
        val_u64(alpha.get_field("Threshold")).unwrap_or(128) as u8,
    ))
}

fn shader_clamp_mode(shader: &NifBlock) -> u32 {
    if let Some(data) = shader
        .get_field("Shader Property Data")
        .and_then(|value| as_struct(Some(value)))
        .and_then(|data| data.get("Texture Clamp Mode"))
    {
        return clamp_mode(Some(data));
    }
    clamp_mode(shader.get_field("Texture Clamp Mode"))
}

fn extract_inline_geometry(block: &NifBlock) -> Option<LodGeometry> {
    let mut geometry = LodGeometry::new();
    for vertex in val_array(block.get_field("Vertex Data")) {
        let fields = as_struct(Some(vertex))?;
        geometry.vertices.push(vec3(fields.get("Vertex"))?);
        if let Some(uv) = uv2(fields.get("UV")) {
            geometry.uvcoords.push(uv);
        }
        if let Some(normal) = vec3(fields.get("Normal")) {
            geometry.normals.push(normal);
        }
        if let Some(tangent) = vec3(fields.get("Tangent")) {
            geometry.tangents.push(tangent);
        }
        let bitangent = if let Some(b) = vec3(fields.get("Bitangent")) {
            Some(b)
        } else {
            Some([
                val_f32(fields.get("Bitangent X")).unwrap_or(0.0),
                val_f32(fields.get("Bitangent Y")).unwrap_or(0.0),
                val_f32(fields.get("Bitangent Z")).unwrap_or(0.0),
            ])
        };
        if let Some(bitangent) = bitangent {
            geometry.bitangents.push(bitangent);
        }
        if let Some(color) = color4(fields.get("Vertex Colors"))
            .or_else(|| color4(fields.get("Vertex Color")))
            .or_else(|| color4(fields.get("Color")))
        {
            geometry.vertex_colors.push(color);
        }
    }
    for tri in val_array(block.get_field("Triangles")) {
        let Some(fields) = as_struct(Some(tri)) else {
            continue;
        };
        let v1 = val_u64(fields.get("v1")).unwrap_or(0) as u32;
        let v2 = val_u64(fields.get("v2")).unwrap_or(0) as u32;
        let v3 = val_u64(fields.get("v3")).unwrap_or(0) as u32;
        geometry.triangles.push([v1, v2, v3]);
    }
    if geometry.uvcoords.len() != geometry.vertices.len() {
        geometry.uvcoords.clear();
    }
    if geometry.normals.len() != geometry.vertices.len() {
        geometry.normals.clear();
    }
    if geometry.tangents.len() != geometry.vertices.len()
        || geometry.bitangents.len() != geometry.vertices.len()
    {
        geometry.tangents.clear();
        geometry.bitangents.clear();
    }
    if geometry.vertex_colors.len() != geometry.vertices.len() {
        geometry.vertex_colors.clear();
    }
    geometry.update_bbox();
    Some(geometry)
}

fn remap_geometry_for_triangles(source: &LodGeometry, triangles: &[[u32; 3]]) -> LodGeometry {
    let mut out = LodGeometry::new();
    let mut remap = HashMap::<u32, u32>::new();
    for tri in triangles {
        let mut out_tri = [0u32; 3];
        for (i, old_idx) in tri.iter().copied().enumerate() {
            let new_idx = if let Some(existing) = remap.get(&old_idx) {
                *existing
            } else {
                let idx = out.vertices.len() as u32;
                copy_vertex(source, old_idx as usize, &mut out);
                remap.insert(old_idx, idx);
                idx
            };
            out_tri[i] = new_idx;
        }
        out.triangles.push(out_tri);
    }
    out.update_bbox();
    out
}

fn copy_vertex(source: &LodGeometry, idx: usize, out: &mut LodGeometry) {
    out.vertices.push(source.vertices[idx]);
    if !source.uvcoords.is_empty() {
        out.uvcoords.push(source.uvcoords[idx]);
    }
    if !source.normals.is_empty() {
        out.normals.push(source.normals[idx]);
    }
    if !source.tangents.is_empty() {
        out.tangents.push(source.tangents[idx]);
    }
    if !source.bitangents.is_empty() {
        out.bitangents.push(source.bitangents[idx]);
    }
    if !source.vertex_colors.is_empty() {
        out.vertex_colors.push(source.vertex_colors[idx]);
    }
}

fn vertex_slice(geometry: &LodGeometry, idx: u32) -> i32 {
    geometry
        .bitangents
        .get(idx as usize)
        .map(|b| b[0].round() as i32)
        .unwrap_or(0)
}

fn rewrite_texture_path(
    source_root: &Path,
    output_dir: &Path,
    world: &str,
    texture: &str,
    specular: bool,
    copied: &Mutex<HashSet<PathBuf>>,
    dds_written: &mut u32,
) -> String {
    let mut normalized = normalize_texture_rel(texture);
    if normalized.is_empty() || !normalized.to_ascii_lowercase().ends_with(".dds") {
        return normalized;
    }
    let mut src_path = source_root.join(normalized.replace('\\', "/"));
    if !src_path.is_file() && !normalized.starts_with("textures\\") {
        let prefixed = format!("textures\\{normalized}");
        let prefixed_path = source_root.join(prefixed.replace('\\', "/"));
        if prefixed_path.is_file() {
            normalized = prefixed;
            src_path = prefixed_path;
        }
    }
    if is_fo4_shared_texture(&normalized) {
        return normalized;
    }
    let dest_rel = hybrid_texture_rel(world, &normalized, specular);
    let dst_path = output_dir.join(dest_rel.replace('\\', "/"));
    if src_path.is_file() {
        let should_write = {
            let mut copied = copied
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            copied.insert(dst_path.clone())
                && texture_needs_write(&src_path, &dst_path, &normalized, specular)
        };
        if should_write {
            if let Some(parent) = dst_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if write_fo4_lod_texture(&src_path, &dst_path, &normalized, specular).is_ok() {
                *dds_written += 1;
            }
        }
        return dest_rel;
    }
    normalized
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fo4DdsTarget {
    Dxt1,
    Dxt5,
    Bc5Unorm,
}

impl Fo4DdsTarget {
    fn format(self) -> &'static str {
        match self {
            Self::Dxt1 => "DXT1",
            Self::Dxt5 => "DXT5",
            Self::Bc5Unorm => "BC5_UNORM",
        }
    }

    fn matches_fourcc(self, fourcc: &[u8; 4]) -> bool {
        match self {
            Self::Dxt1 => fourcc == b"DXT1",
            Self::Dxt5 => fourcc == b"DXT5",
            Self::Bc5Unorm => fourcc == b"ATI2" || fourcc == b"BC5U",
        }
    }
}

fn texture_needs_write(src_path: &Path, dst_path: &Path, normalized: &str, specular: bool) -> bool {
    if !dst_path.is_file() {
        return true;
    }
    let Some(target) = fo4_lod_texture_target(src_path, normalized, specular) else {
        return false;
    };
    !texture_matches_target(dst_path, target)
}

fn write_fo4_lod_texture(
    src_path: &Path,
    dst_path: &Path,
    normalized: &str,
    specular: bool,
) -> std::io::Result<u64> {
    if let Some(target) = fo4_lod_texture_target(src_path, normalized, specular) {
        if texture_matches_target(dst_path, target) {
            return Ok(0);
        }
        match transcode_fo4_lod_texture(src_path, dst_path, target) {
            Ok(()) => return Ok(std::fs::metadata(dst_path).map(|m| m.len()).unwrap_or(0)),
            Err(err) => {
                eprintln!(
                    "[lodgen] hybrid source BTO: failed to transcode {} to {}: {err}",
                    src_path.display(),
                    target.format()
                );
            }
        }
    }
    std::fs::copy(src_path, dst_path)
}

fn transcode_fo4_lod_texture(
    src_path: &Path,
    dst_path: &Path,
    target: Fo4DdsTarget,
) -> Result<(), String> {
    let mips = match directxtex_native::read_dds_mips_rgba8(src_path) {
        Ok(mips) => mips,
        Err(_) => {
            let image = directxtex_native::read_dds_rgba_image(src_path)?;
            directxtex_native::DdsMipsRgba8 {
                width: image.width,
                height: image.height,
                dxgi_format: image.dxgi_format,
                mips: vec![(image.width, image.height, image.rgba)],
            }
        }
    };
    let bytes =
        directxtex_native::encode_dds_from_rgba8_chain(&mips.mips, target.format(), true, None)?;
    std::fs::write(dst_path, bytes).map_err(|err| err.to_string())
}

fn fo4_lod_texture_target(
    src_path: &Path,
    normalized: &str,
    specular: bool,
) -> Option<Fo4DdsTarget> {
    if texture_is_normal_map(normalized) || specular {
        if texture_matches_target(src_path, Fo4DdsTarget::Bc5Unorm) {
            return None;
        }
        return Some(Fo4DdsTarget::Bc5Unorm);
    }

    let probe = directxtex_native::read_dds_probe(src_path).ok()?;
    let fourcc = dds_fourcc(src_path).ok()?;
    match probe.dxgi_format {
        // Legacy DXT1/DXT5 are FO4-safe. DX10 BC1/BC3 should be rewritten to
        // the matching legacy header because object LOD texture streaming is
        // much less tolerant than normal NIF material texture loading.
        71 if &fourcc != b"DX10" => None,
        77 if &fourcc != b"DX10" => None,
        71 | 72 => Some(Fo4DdsTarget::Dxt1),
        77 | 78 => Some(Fo4DdsTarget::Dxt5),
        98 | 99 => Some(Fo4DdsTarget::Dxt5),
        28 | 29 if &fourcc == b"DX10" => Some(Fo4DdsTarget::Dxt5),
        _ if &fourcc == b"DX10" => Some(Fo4DdsTarget::Dxt5),
        _ => None,
    }
}

fn texture_is_normal_map(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with("_n.dds") || lower.ends_with(".n.dds")
}

fn texture_matches_target(path: &Path, target: Fo4DdsTarget) -> bool {
    dds_fourcc(path)
        .map(|fourcc| target.matches_fourcc(&fourcc))
        .unwrap_or(false)
}

fn dds_fourcc(path: &Path) -> std::io::Result<[u8; 4]> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut header = [0u8; 88];
    file.read_exact(&mut header)?;
    let mut fourcc = [0u8; 4];
    fourcc.copy_from_slice(&header[84..88]);
    Ok(fourcc)
}

fn normalize_texture_rel(texture: &str) -> String {
    let mut t = texture
        .trim_matches('\0')
        .trim()
        .replace('/', "\\")
        .to_ascii_lowercase();
    while t.starts_with('\\') {
        t.remove(0);
    }
    if let Some(rest) = t.strip_prefix("data\\") {
        t = rest.to_string();
    }
    if t.starts_with("lod\\") {
        t = format!("textures\\{t}");
    } else {
        let has_known_prefix = t.starts_with("textures\\")
            || t.starts_with("materials\\")
            || t.starts_with("meshes\\")
            || t.starts_with("interface\\")
            || t.starts_with("strings\\")
            || t.starts_with("shadersfx\\")
            || t.starts_with("vis\\");
        if !has_known_prefix && t.contains('\\') {
            t = format!("textures\\{t}");
        }
    }
    t
}

fn hybrid_texture_rel(world: &str, normalized: &str, specular: bool) -> String {
    let mut rest = normalized.to_string();
    for prefix in [
        "textures\\lod\\generated\\texturearrays\\",
        "textures\\lod\\generated\\merged\\",
        "textures\\",
    ] {
        if let Some(stripped) = rest.strip_prefix(prefix) {
            rest = stripped.to_string();
            break;
        }
    }
    if specular {
        rest = fo4_specular_name(&rest);
    }
    rest = shorten_long_path_components(&rest);
    format!(
        "textures\\terrain\\{}\\objects\\hybrid\\{}",
        world.to_ascii_lowercase(),
        rest
    )
}

fn shorten_long_path_components(path: &str) -> String {
    path.split('\\')
        .map(|component| shorten_path_component(component, 48))
        .collect::<Vec<_>>()
        .join("\\")
}

fn shorten_path_component(component: &str, max_len: usize) -> String {
    if component.len() <= max_len {
        return component.to_string();
    }

    let hash = fnv1a_32(component.as_bytes());
    let suffix = format!("_{hash:08x}");
    if let Some(dot) = component.rfind('.') {
        let ext = &component[dot..];
        let keep = max_len.saturating_sub(suffix.len() + ext.len());
        if keep > 0 {
            return format!("{}{}{}", &component[..keep], suffix, ext);
        }
    }

    let keep = max_len.saturating_sub(suffix.len());
    format!("{}{}", &component[..keep], suffix)
}

fn fnv1a_32(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c9dc5u32;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

fn fo4_specular_name(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with("_r.dds") {
        format!("{}_s.dds", &path[..path.len() - 6])
    } else if lower.ends_with(".r.dds") {
        format!("{}.s.dds", &path[..path.len() - 6])
    } else {
        path.to_string()
    }
}

fn is_fo4_shared_texture(texture: &str) -> bool {
    texture.starts_with("textures\\shared\\")
}

fn tile_allowed(tile: &SourceBtoTile, settings: &LodSettings) -> bool {
    if tile.level < settings.global.lod_min || tile.level > settings.global.lod_max {
        return false;
    }
    if let Some([sw_x, sw_y]) = settings.global.southwest_cell {
        if let Some(stride) = settings.global.stride.filter(|s| *s > 0) {
            let ne_x = sw_x.saturating_add(stride).saturating_sub(1);
            let ne_y = sw_y.saturating_add(stride).saturating_sub(1);
            if tile.x < sw_x || tile.x > ne_x || tile.y < sw_y || tile.y > ne_y {
                return false;
            }
        }
    }
    if let Some(bounds) = settings.global.bounds {
        if tile.x < bounds.w || tile.x > bounds.e || tile.y < bounds.s || tile.y > bounds.n {
            return false;
        }
    }
    if let Some(chunk) = settings.global.chunk.as_ref() {
        return chunk.level == tile.level
            && tile.x >= chunk.w
            && tile.x <= chunk.e
            && tile.y >= chunk.s
            && tile.y <= chunk.n;
    }
    true
}

fn source_quad(tile: &SourceBtoTile) -> QuadDesc {
    QuadDesc {
        z_order: 0,
        x: tile.x,
        y: tile.y,
        quad_level: tile.level,
        quad_index: level_index(tile.level) as i32,
        quad_offset: (tile.level * 4096) as f32,
        static_indices: Vec::new(),
        statics: Vec::new(),
        out_values: OutDesc::default(),
    }
}

fn index_standard_tree_refs_by_tile(
    world: &WorldspaceInput,
    settings: &LodSettings,
) -> TreeTileIndex {
    let mut out = TreeTileIndex::new();
    let levels = [4, 8, 16, 32]
        .into_iter()
        .filter(|level| *level >= settings.global.lod_min && *level <= settings.global.lod_max);
    let (lod_sw, stride) = crate::driver::lod_settings_window(world, settings);
    let (emit_w, emit_s, emit_e, emit_n) =
        crate::driver::object_emit_bounds(settings, lod_sw, stride);
    for level in levels {
        let quad_offset = (level * 4096) as f32;
        let level_index = level_index(level);
        let chunk_filter = settings.global.chunk.as_ref().filter(|c| c.level == level);
        for (idx, stat) in world.refs.iter().enumerate() {
            let Some(model) = stat
                .lod_models
                .get(level_index)
                .and_then(|model| model.as_ref())
            else {
                continue;
            };
            if !standard_tree_replacement_ref(stat, model) {
                continue;
            }
            let (cell_x, cell_y) = crate::driver::ref_lod_cell(stat);
            if cell_x < emit_w || cell_x > emit_e || cell_y < emit_s || cell_y > emit_n {
                continue;
            }
            let x = crate::driver::object_quad_origin(stat.pos[0], lod_sw.0, level, quad_offset);
            let y = crate::driver::object_quad_origin(stat.pos[1], lod_sw.1, level, quad_offset);
            if x < emit_w || x > emit_e || y < emit_s || y > emit_n {
                continue;
            }
            if chunk_filter
                .map(|c| x < c.w || x > c.e || y < c.s || y > c.n)
                .unwrap_or(false)
            {
                continue;
            }
            out.entry((level, x, y)).or_default().push(idx);
        }
    }
    out
}

fn standard_tree_replacement_ref(stat: &StaticDesc, model: &str) -> bool {
    crate::trees::is_tree(stat) || is_tree_instance_model(model)
}

fn standard_tree_refs_for_tile<'a>(
    world: &'a WorldspaceInput,
    standard_tree_tiles: &TreeTileIndex,
    tile: &SourceBtoTile,
) -> Vec<&'a StaticDesc> {
    standard_tree_tiles
        .get(&(tile.level, tile.x, tile.y))
        .into_iter()
        .flat_map(|indices| indices.iter())
        .filter_map(|idx| world.refs.get(*idx))
        .collect()
}

fn level_index(level: i32) -> usize {
    match level {
        4 => 0,
        8 => 1,
        16 => 2,
        32 => 3,
        _ => 0,
    }
}

fn source_objects_dir(source_root: &Path, world: &str) -> PathBuf {
    let meshes = find_child_ci(source_root, "Meshes").unwrap_or_else(|| source_root.join("Meshes"));
    let terrain = find_child_ci(&meshes, "Terrain").unwrap_or_else(|| meshes.join("Terrain"));
    let world_dir = find_child_ci(&terrain, world).unwrap_or_else(|| terrain.join(world));
    find_child_ci(&world_dir, "Objects").unwrap_or_else(|| world_dir.join("Objects"))
}

fn collect_files_recursive(root: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case(ext))
            {
                out.push(path);
            }
        }
    }
    out
}

fn collect_lod_dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case("LOD")
            {
                out.push(path);
            } else {
                stack.push(path);
            }
        }
    }
    out
}

fn find_child_ci(parent: &Path, name: &str) -> Option<PathBuf> {
    let read_dir = std::fs::read_dir(parent).ok()?;
    for entry in read_dir.flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
        {
            return Some(entry.path());
        }
    }
    None
}

fn resolve_source_data_path_ci(source_root: &Path, rel: &str) -> Option<PathBuf> {
    let normalized = normalize_texture_rel(rel).replace('\\', "/");
    let direct = source_root.join(&normalized);
    if direct.is_file() {
        return Some(direct);
    }
    let mut cur = source_root.to_path_buf();
    for part in normalized.split('/').filter(|part| !part.is_empty()) {
        let entries = std::fs::read_dir(&cur).ok()?;
        let mut matched = None;
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(part)
            {
                matched = Some(entry.path());
                break;
            }
        }
        cur = matched?;
    }
    cur.is_file().then_some(cur)
}

fn same_path_text(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}

fn resource_id_from_value(value: &NifValue) -> Option<ResourceId> {
    let fields = as_struct(Some(value))?;
    let dir = val_u64(fields.get("Directory Hash"))? as u32;
    let file = val_u64(fields.get("File Hash"))? as u32;
    let ext = match fields.get("Extension") {
        Some(NifValue::Array(chars)) => chars
            .iter()
            .filter_map(|v| match v {
                NifValue::Char(s) | NifValue::String(s) => s.as_bytes().first().copied(),
                NifValue::UInt(v) => Some(*v as u8),
                NifValue::Int(v) => Some(*v as u8),
                _ => None,
            })
            .enumerate()
            .fold(0u32, |acc, (idx, byte)| acc | ((byte as u32) << (idx * 8))),
        Some(NifValue::String(s)) => s
            .as_bytes()
            .iter()
            .copied()
            .enumerate()
            .fold(0u32, |acc, (idx, byte)| acc | ((byte as u32) << (idx * 8))),
        other => val_u64(other).unwrap_or(0) as u32,
    };
    Some(ResourceId { dir, file, ext })
}

fn mat4(value: &NifValue) -> Option<[[f32; 4]; 4]> {
    match value {
        NifValue::Matrix44(m) => Some(*m),
        NifValue::Struct(fields) => Some([
            [
                val_f32(fields.get("m11")).unwrap_or(1.0),
                val_f32(fields.get("m12")).unwrap_or(0.0),
                val_f32(fields.get("m13")).unwrap_or(0.0),
                val_f32(fields.get("m14")).unwrap_or(0.0),
            ],
            [
                val_f32(fields.get("m21")).unwrap_or(0.0),
                val_f32(fields.get("m22")).unwrap_or(1.0),
                val_f32(fields.get("m23")).unwrap_or(0.0),
                val_f32(fields.get("m24")).unwrap_or(0.0),
            ],
            [
                val_f32(fields.get("m31")).unwrap_or(0.0),
                val_f32(fields.get("m32")).unwrap_or(0.0),
                val_f32(fields.get("m33")).unwrap_or(1.0),
                val_f32(fields.get("m34")).unwrap_or(0.0),
            ],
            [
                val_f32(fields.get("m41")).unwrap_or(0.0),
                val_f32(fields.get("m42")).unwrap_or(0.0),
                val_f32(fields.get("m43")).unwrap_or(0.0),
                val_f32(fields.get("m44")).unwrap_or(1.0),
            ],
        ]),
        _ => None,
    }
}

fn block_of(nif: &NifFile, id: i32) -> Option<&NifBlock> {
    if id < 0 {
        return None;
    }
    nif.get_block(id as usize)
}

fn block_name(block: &NifBlock) -> String {
    val_string(block.get_field("Name"))
        .unwrap_or_default()
        .replace("\\n", "")
        .replace("\\r", "")
        .trim()
        .to_string()
}

fn as_struct(value: Option<&NifValue>) -> Option<&indexmap::IndexMap<String, NifValue>> {
    match value {
        Some(NifValue::Struct(fields)) => Some(fields),
        _ => None,
    }
}

fn val_array(value: Option<&NifValue>) -> &[NifValue] {
    match value {
        Some(NifValue::Array(items)) => items,
        _ => &[],
    }
}

fn val_string(value: Option<&NifValue>) -> Option<String> {
    match value {
        Some(NifValue::String(s)) | Some(NifValue::Char(s)) => {
            Some(s.trim_end_matches('\0').to_string())
        }
        _ => None,
    }
}

fn val_ref(value: Option<&NifValue>) -> Option<i32> {
    match value {
        Some(NifValue::Ref(v)) => Some(*v),
        Some(NifValue::Int(v)) => Some(*v as i32),
        Some(NifValue::UInt(v)) => Some(*v as i32),
        _ => None,
    }
}

fn val_u64(value: Option<&NifValue>) -> Option<u64> {
    match value {
        Some(NifValue::UInt(v)) => Some(*v),
        Some(NifValue::Int(v)) if *v >= 0 => Some(*v as u64),
        Some(NifValue::Bool(v)) => Some(u64::from(*v)),
        Some(NifValue::Ref(v)) if *v >= 0 => Some(*v as u64),
        _ => None,
    }
}

fn val_f32(value: Option<&NifValue>) -> Option<f32> {
    match value {
        Some(NifValue::Float(v)) => Some(*v as f32),
        Some(NifValue::Int(v)) => Some(*v as f32),
        Some(NifValue::UInt(v)) => Some(*v as f32),
        _ => None,
    }
}

fn vec3(value: Option<&NifValue>) -> Option<[f32; 3]> {
    match value {
        Some(NifValue::Vec3(v)) => Some(*v),
        Some(NifValue::Struct(fields)) => Some([
            val_f32(fields.get("x")).unwrap_or(0.0),
            val_f32(fields.get("y")).unwrap_or(0.0),
            val_f32(fields.get("z")).unwrap_or(0.0),
        ]),
        _ => None,
    }
}

fn uv2(value: Option<&NifValue>) -> Option<[f32; 2]> {
    match value {
        Some(NifValue::Struct(fields)) => Some([
            val_f32(fields.get("u")).unwrap_or(0.0),
            val_f32(fields.get("v")).unwrap_or(0.0),
        ]),
        Some(NifValue::Vec3(v)) => Some([v[0], v[1]]),
        _ => None,
    }
}

fn color4(value: Option<&NifValue>) -> Option<[f32; 4]> {
    match value {
        Some(NifValue::Color4(v)) | Some(NifValue::Vec4(v)) => Some(*v),
        Some(NifValue::Color3(v)) => Some([v[0], v[1], v[2], 1.0]),
        Some(NifValue::Struct(fields)) => Some([
            val_f32(fields.get("r")).unwrap_or(1.0),
            val_f32(fields.get("g")).unwrap_or(1.0),
            val_f32(fields.get("b")).unwrap_or(1.0),
            val_f32(fields.get("a")).unwrap_or(1.0),
        ]),
        _ => None,
    }
}

fn clamp_mode(value: Option<&NifValue>) -> u32 {
    match value {
        Some(NifValue::String(s)) => {
            let key: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            match key.to_ascii_lowercase().as_str() {
                "clampsclampt" => 0,
                "clampswrapt" => 1,
                "wrapsclampt" => 2,
                "wrapswrapt" => 3,
                _ => 3,
            }
        }
        other => val_u64(other).map(|v| v as u32).unwrap_or(3),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::static_desc::ShapeFlags;

    #[test]
    fn source_bto_enumerator_counts_only_matching_world_tiles() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "lodgen_source_bto_enumerator_{}_{}",
            std::process::id(),
            unique
        ));
        let objects = root.join("Meshes/Terrain/ThePitt/Objects");
        std::fs::create_dir_all(&objects).unwrap();
        std::fs::write(objects.join("ThePitt.4.0.0.bto"), []).unwrap();
        std::fs::write(objects.join("thepitt.8.-1.2.BTO"), []).unwrap();
        std::fs::write(objects.join("Appalachia.4.0.0.bto"), []).unwrap();
        std::fs::write(objects.join("not-a-tile.bto"), []).unwrap();

        let tiles = enumerate_source_bto_tiles(&root, "THEPITT").unwrap();
        assert_eq!(tiles.len(), 2);
        assert_eq!((tiles[0].level, tiles[0].x, tiles[0].y), (4, 0, 0));
        assert_eq!((tiles[1].level, tiles[1].x, tiles[1].y), (8, -1, 2));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_bto_filename_parser_extracts_world_level_and_tile() {
        let path = Path::new("Meshes/Terrain/Appalachia/Objects/Appalachia.16.-14.-13.bto");
        let tile = parse_source_bto_filename(path).unwrap();
        assert_eq!(tile.world, "Appalachia");
        assert_eq!(tile.level, 16);
        assert_eq!(tile.x, -14);
        assert_eq!(tile.y, -13);
    }

    #[test]
    fn hybrid_atlas_from_lod_routes_l4_raw_and_l8_atlas() {
        let mut settings = LodSettings::fo4_default();
        settings.objects.fo76_bto_atlas_from_lod = Some(8);

        let mut tile = SourceBtoTile {
            world: "APPALACHIA".to_string(),
            level: 4,
            x: 2,
            y: 3,
            path: PathBuf::from("APPALACHIA.4.2.3.bto"),
        };
        assert_eq!(
            source_bto_mode_for_tile(SourceBtoMode::Atlas, &settings, &tile),
            SourceBtoMode::Raw
        );

        tile.level = 8;
        tile.path = PathBuf::from("APPALACHIA.8.2.3.bto");
        assert_eq!(
            source_bto_mode_for_tile(SourceBtoMode::Atlas, &settings, &tile),
            SourceBtoMode::Atlas
        );
    }

    #[test]
    fn baked_shape_name_alpha_classifier_handles_negation_and_translucent() {
        assert!(!baked_shape_name_implies_alpha(
            "GlobalAtlasShape_NotAlphaTested"
        ));
        assert!(baked_shape_name_implies_alpha(
            "GlobalAtlasShape_AlphaTested"
        ));
        assert!(baked_shape_name_implies_alpha(
            "GlobalAtlasShape_Translucent"
        ));
    }

    #[test]
    fn make_baked_shape_keeps_not_alpha_tested_names_opaque_without_alpha_property() {
        let mut geometry = LodGeometry::new();
        geometry.vertices = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        geometry.uvcoords = vec![[0.0, 0.0]; 3];
        geometry.triangles = vec![[0, 1, 2]];

        let tile = SourceBtoTile {
            world: "APPALACHIA".to_string(),
            level: 16,
            x: -46,
            y: -29,
            path: PathBuf::from("Appalachia.16.-46.-29.bto"),
        };
        let quad = source_quad(&tile);

        let shape = make_baked_shape(
            "GlobalAtlasShape_NotAlphaTested".to_string(),
            &tile,
            &quad,
            geometry,
            Default::default(),
            None,
            3,
            None,
            [0.0, 0.0, 0.0],
            16.0,
        );

        assert!(!shape.flags.contains(ShapeFlags::IS_ALPHA));
    }

    #[test]
    fn make_baked_shape_preserves_alpha_property_override() {
        let mut geometry = LodGeometry::new();
        geometry.vertices = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        geometry.uvcoords = vec![[0.0, 0.0]; 3];
        geometry.triangles = vec![[0, 1, 2]];

        let tile = SourceBtoTile {
            world: "APPALACHIA".to_string(),
            level: 16,
            x: -46,
            y: -29,
            path: PathBuf::from("Appalachia.16.-46.-29.bto"),
        };
        let quad = source_quad(&tile);

        let shape = make_baked_shape(
            "GlobalAtlasShape_NotAlphaTested".to_string(),
            &tile,
            &quad,
            geometry,
            Default::default(),
            Some((4844, 128)),
            3,
            None,
            [0.0, 0.0, 0.0],
            16.0,
        );

        assert!(shape.flags.contains(ShapeFlags::IS_ALPHA));
    }

    #[test]
    fn make_baked_shape_sets_l4_segment_position_from_source_bbox() {
        let mut geometry = LodGeometry::new();
        geometry.vertices = vec![
            [1400.0, 2500.0, 0.0],
            [1460.0, 2500.0, 0.0],
            [1400.0, 2700.0, 0.0],
        ];
        geometry.uvcoords = vec![[0.0, 0.0]; 3];
        geometry.triangles = vec![[0, 1, 2]];

        let tile = SourceBtoTile {
            world: "APPALACHIA".to_string(),
            level: 4,
            x: -10,
            y: -1,
            path: PathBuf::from("Appalachia.4.-10.-1.bto"),
        };
        let quad = source_quad(&tile);

        let shape = make_baked_shape(
            "GlobalAtlasShape_NotAlphaTested".to_string(),
            &tile,
            &quad,
            geometry,
            Default::default(),
            None,
            3,
            Some(6),
            [-40960.0, -4096.0, 0.0],
            4.0,
        );
        let segments = crate::objects::object_lod::generate_segments(
            &quad,
            shape.x,
            shape.y,
            shape.geometry.num_triangles() as u16,
        );

        assert_eq!(segments[0].id, 6);
    }

    #[test]
    fn resource_id_matches_known_fo76_lod_mesh() {
        let rid =
            resource_id_from_path("meshes\\lod\\landscape\\trees\\mtntoppinetree_lg01_lod_2.nif");
        assert_eq!(rid.dir, 1420557209);
        assert_eq!(rid.file, 3550270850);
        assert_eq!(rid.ext, 0x0066_696e);
    }

    #[test]
    fn resource_resolver_indexes_nested_lod_dirs() {
        let unique = format!(
            "lodgen_fo76_bto_resolver_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let top_lod = root.join("Meshes/LOD/Landscape/Trees");
        let dlc_lod = root.join("Meshes/DLC03/LOD/Landscape/Trees");
        std::fs::create_dir_all(&top_lod).unwrap();
        std::fs::create_dir_all(&dlc_lod).unwrap();
        std::fs::write(top_lod.join("CranbogPineMed02_lod_2.nif"), b"nif").unwrap();
        std::fs::write(dlc_lod.join("TreeRedPineDead02_lod_0.nif"), b"nif").unwrap();

        let resolver = ResourceResolver::build(&root).unwrap();
        let top_rid =
            resource_id_from_path("meshes\\lod\\landscape\\trees\\cranbogpinemed02_lod_2.nif");
        let nested_rid = resource_id_from_path(
            "meshes\\dlc03\\lod\\landscape\\trees\\treeredpinedead02_lod_0.nif",
        );

        assert_eq!(
            resolver.resolve(&top_rid),
            Some("Meshes\\LOD\\Landscape\\Trees\\CranbogPineMed02_lod_2.nif")
        );
        assert_eq!(
            resolver.resolve(&nested_rid),
            Some("Meshes\\DLC03\\LOD\\Landscape\\Trees\\TreeRedPineDead02_lod_0.nif")
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hybrid_atlas_prefers_preconverted_fo4_texture_bundle() {
        let unique = format!(
            "lodgen_fo76_bto_atlas_convert_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let source = root.join("source");
        let output = root.join("output");
        let rel_dir = Path::new("textures/lod/generated/texturearrays/translucent");
        std::fs::create_dir_all(source.join(rel_dir)).unwrap();
        std::fs::create_dir_all(output.join(rel_dir)).unwrap();
        for suffix in ["d", "n", "r", "l"] {
            std::fs::write(
                source
                    .join(rel_dir)
                    .join(format!("translucent_55_{suffix}.dds")),
                b"source",
            )
            .unwrap();
        }
        for suffix in ["d", "n", "s"] {
            std::fs::write(
                output
                    .join(rel_dir)
                    .join(format!("translucent_55_{suffix}.dds")),
                b"output",
            )
            .unwrap();
        }

        let paths = LodPaths {
            output_dir: output.clone(),
            data_dirs: Vec::new(),
            source_data_dir: Some(source.clone()),
        };
        let mut texture_set: [String; 10] = Default::default();
        texture_set[0] =
            r"textures\lod\generated\texturearrays\translucent\translucent_55_d.dds".into();
        texture_set[1] =
            r"textures\lod\generated\texturearrays\translucent\translucent_55_n.dds".into();
        texture_set[2] =
            r"textures\lod\generated\texturearrays\translucent\translucent_55_l.dds".into();
        texture_set[7] =
            r"textures\lod\generated\texturearrays\translucent\translucent_55_r.dds".into();

        let mut texture_sets = BTreeMap::new();
        insert_hybrid_atlas_texture_set(&paths, &source, &texture_set, &mut texture_sets);
        let texture = texture_sets.values().next().unwrap();

        assert_eq!(
            texture.atlas_key(),
            r"textures\lod\generated\texturearrays\translucent\translucent_55_d.dds,textures\lod\generated\texturearrays\translucent\translucent_55_n.dds"
        );
        assert_eq!(texture.lighting_rel, "");
        assert!(texture.diffuse_path.starts_with(&output));
        assert!(texture.normal_path.as_ref().unwrap().starts_with(&output));
        assert!(texture.specular_path.as_ref().unwrap().starts_with(&output));
        assert!(texture.specular_rel.ends_with("_s.dds"));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hybrid_atlas_reuses_valid_cached_pages() {
        struct TestProgress {
            events: Vec<String>,
        }
        impl Progress for TestProgress {
            fn report(&mut self, msg: &str, _frac: f32) {
                self.events.push(msg.to_string());
            }
        }

        let unique = format!(
            "lodgen_fo76_bto_atlas_cache_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let output = root.join("output");
        let atlas_rel = source_bto_atlas_page_rel("APPALACHIA", 0);
        let atlas_path = output.join(atlas_rel.replace('\\', "/"));
        let atlas_normal = crate::atlas::atlas::sibling_dds(&atlas_path, "_n");
        let atlas_specular = crate::atlas::atlas::sibling_dds(&atlas_path, "_s");
        std::fs::create_dir_all(atlas_path.parent().unwrap()).unwrap();
        std::fs::write(&atlas_path, b"diffuse").unwrap();
        std::fs::write(&atlas_normal, b"normal").unwrap();
        std::fs::write(&atlas_specular, b"specular").unwrap();

        let texture = HybridAtlasTexture {
            diffuse_rel: r"textures\lod\generated\rock_d.dds".to_string(),
            normal_rel: r"textures\lod\generated\rock_n.dds".to_string(),
            lighting_rel: String::new(),
            specular_rel: r"textures\lod\generated\rock_s.dds".to_string(),
            diffuse_path: root.join("rock_d.dds"),
            normal_path: Some(root.join("rock_n.dds")),
            lighting_path: None,
            specular_path: Some(root.join("rock_s.dds")),
        };
        crate::atlas::write_atlas_map(
            &atlas_path.with_extension("txt"),
            &[
                AtlasMapRow {
                    source: texture.atlas_key(),
                    tile_w: 64,
                    tile_h: 64,
                    x: 128,
                    y: 256,
                    atlas: crate::atlas::atlas::data_relative_path(&atlas_path),
                    atlas_w: 1024,
                    atlas_h: 1024,
                },
                AtlasMapRow {
                    source: r"textures\lod\generated\unused_d.dds".to_string(),
                    tile_w: 32,
                    tile_h: 32,
                    x: 512,
                    y: 512,
                    atlas: crate::atlas::atlas::data_relative_path(&atlas_path),
                    atlas_w: 1024,
                    atlas_h: 1024,
                },
            ],
        )
        .unwrap();

        let paths = LodPaths {
            output_dir: output,
            data_dirs: Vec::new(),
            source_data_dir: None,
        };
        let settings = LodSettings::fo4_default();
        let mut progress = TestProgress { events: Vec::new() };
        let (atlas, stats) = try_load_cached_hybrid_atlas(
            "APPALACHIA",
            &settings,
            &paths,
            &[texture.clone()],
            &mut progress,
        )
        .unwrap();

        assert_eq!(stats.pages, 1);
        assert_eq!(stats.dds_written, 0);
        assert_eq!(atlas.dds_written, 0);
        assert_eq!(atlas.atlas_size, (1024, 1024));
        assert_eq!(atlas.list.len(), 2);
        assert!(atlas.list.get(&texture.atlas_key()).is_some());
        assert!(
            progress
                .events
                .iter()
                .any(|event| event.contains("reused cached atlas"))
        );

        let mut early_progress = TestProgress { events: Vec::new() };
        let (_early_atlas, early_stats) = try_load_cached_hybrid_atlas_without_scan(
            "APPALACHIA",
            &settings,
            &paths,
            &mut early_progress,
        )
        .unwrap();
        assert_eq!(early_stats.pages, 1);
        assert_eq!(early_stats.texture_sets, 2);
        assert!(
            early_progress
                .events
                .iter()
                .any(|event| event.contains("skipped source BTO atlas scan"))
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hybrid_atlas_existing_map_rebuild_rejects_changed_lod_cutoff() {
        let mut settings = LodSettings::fo4_default();
        settings.objects.fo76_bto_atlas_from_lod = Some(8);

        let mut stale_meta = hybrid_atlas_cache_meta(&settings);
        stale_meta.atlas_from_lod = None;
        assert!(!hybrid_atlas_cache_source_selection_matches(
            &stale_meta,
            &settings
        ));

        stale_meta.atlas_from_lod = Some(8);
        stale_meta.min_foliage_tile_size = 0;
        assert!(hybrid_atlas_cache_source_selection_matches(
            &stale_meta,
            &settings
        ));
    }

    #[test]
    fn hybrid_atlas_cache_changes_with_mip_flooding() {
        let mut settings = LodSettings::fo4_default();
        let without_flooding = hybrid_atlas_cache_meta(&settings);
        settings.objects.atlas_mip_flooding = true;

        assert!(without_flooding != hybrid_atlas_cache_meta(&settings));
    }

    #[test]
    fn hybrid_atlas_min_tile_reserves_eight_by_eight_pages_without_stretching() {
        assert_eq!(hybrid_atlas_content_size(256, 256, 512, 4096), (512, 512));
        assert_eq!(hybrid_atlas_pack_size(512, 512, 512, 4096), (512, 512));

        assert_eq!(hybrid_atlas_content_size(256, 128, 512, 4096), (512, 256));
        assert_eq!(hybrid_atlas_pack_size(512, 256, 512, 4096), (512, 512));

        assert_eq!(hybrid_atlas_content_size(512, 256, 512, 4096), (512, 256));
        assert_eq!(hybrid_atlas_pack_size(512, 256, 512, 4096), (512, 512));
    }

    #[test]
    fn hybrid_atlas_foliage_min_applies_only_to_foliage_textures() {
        let mut settings = LodSettings::fo4_default();
        settings.objects.fo76_bto_atlas_min_tile_size = 0;
        settings.objects.fo76_bto_atlas_min_foliage_tile_size = 1024;
        settings.objects.fo76_bto_atlas_foliage_page_size = 8192;
        settings.objects.fo76_bto_atlas_foliage_max_tile_size = 1024;
        settings.objects.fo76_bto_atlas_min_alpha_tested_tile_size = 1024;
        settings.objects.fo76_bto_atlas_alpha_tested_page_size = 4096;
        settings.objects.fo76_bto_atlas_alpha_tested_max_tile_size = 1024;

        let tree = HybridAtlasTexture {
            diffuse_rel: r"textures\lod\landscape\trees\cranbog\cranbogpine_d.dds".into(),
            normal_rel: String::new(),
            lighting_rel: String::new(),
            specular_rel: String::new(),
            diffuse_path: PathBuf::new(),
            normal_path: None,
            lighting_path: None,
            specular_path: None,
        };
        let translucent = HybridAtlasTexture {
            diffuse_rel: r"textures\lod\generated\texturearrays\translucent\translucent_55_d.dds"
                .into(),
            ..tree.clone()
        };
        let rock = HybridAtlasTexture {
            diffuse_rel: r"textures\fo76\landscape\rocks\mtntopcliff02_d.dds".into(),
            ..tree.clone()
        };
        let alpha_tested = HybridAtlasTexture {
            diffuse_rel: r"textures\lod\generated\texturearrays\alphatested\alphatested_34_d.dds"
                .into(),
            ..tree.clone()
        };

        assert_eq!(hybrid_atlas_effective_min_tile_size(&settings, &tree), 1024);
        assert_eq!(
            hybrid_atlas_effective_min_tile_size(&settings, &translucent),
            1024
        );
        assert_eq!(hybrid_atlas_effective_min_tile_size(&settings, &rock), 0);
        assert_eq!(
            hybrid_atlas_effective_min_tile_size(&settings, &alpha_tested),
            1024
        );
        assert_eq!(hybrid_atlas_effective_page_size(&settings, &tree), 8192);
        assert_eq!(
            hybrid_atlas_effective_page_size(&settings, &translucent),
            8192
        );
        assert_eq!(
            hybrid_atlas_effective_page_size(&settings, &rock),
            settings.objects.atlas_size
        );
        assert_eq!(
            hybrid_atlas_effective_page_size(&settings, &alpha_tested),
            4096
        );
        assert_eq!(hybrid_atlas_effective_max_tile_size(&settings, &tree), 1024);
        assert_eq!(
            hybrid_atlas_effective_max_tile_size(&settings, &translucent),
            1024
        );
        assert_eq!(
            hybrid_atlas_effective_max_tile_size(&settings, &rock),
            settings.objects.max_tile_size
        );
        assert_eq!(
            hybrid_atlas_effective_max_tile_size(&settings, &alpha_tested),
            1024
        );
        assert_eq!(hybrid_atlas_pack_size(1024, 1024, 1024, 8192), (1024, 1024));
    }

    #[test]
    fn tile_allowed_respects_explicit_lod_window() {
        let mut settings = LodSettings::fo4_default();
        settings.global.stride = Some(256);
        settings.global.southwest_cell = Some([-126, -125]);
        settings.global.bounds = Some(crate::settings::LodBounds {
            w: -126,
            s: -125,
            e: 129,
            n: 130,
        });

        let inside = SourceBtoTile {
            world: "APPALACHIA".to_string(),
            level: 4,
            x: 98,
            y: 91,
            path: PathBuf::from("inside.bto"),
        };
        let outside = SourceBtoTile {
            world: "APPALACHIA".to_string(),
            level: 4,
            x: 98,
            y: 131,
            path: PathBuf::from("outside.bto"),
        };

        assert!(tile_allowed(&inside, &settings));
        assert!(!tile_allowed(&outside, &settings));
    }

    #[test]
    fn instance_static_desc_promotes_matrix_translation_to_ref_position() {
        let node_translation = [4096.0, 8192.0, 10.0];
        let mut matrix = crate::input::identity_part_transform();
        matrix[0][3] = 100.0;
        matrix[1][3] = -200.0;
        matrix[2][3] = 300.0;
        matrix[3][3] = 1.25;

        let stat = instance_static_desc(
            "Meshes\\LOD\\Landscape\\Trees\\Tree_lod_0.nif",
            1,
            node_translation,
            matrix,
            2,
            3,
        );

        assert_eq!(stat.pos, [4196.0, 7992.0, 310.0]);
        assert_eq!(stat.cell, (1, 1));
        assert_eq!(stat.part_transform[0][3], 0.0);
        assert_eq!(stat.part_transform[1][3], 0.0);
        assert_eq!(stat.part_transform[2][3], 0.0);
        assert_eq!(stat.part_scale, 1.25);
    }

    #[test]
    fn tree_instance_classifier_keeps_only_upright_tree_lod_models() {
        assert!(is_tree_instance_model(
            r"Meshes\LOD\Landscape\Trees\MtnTopPineTree_LG01_lod_2.nif"
        ));
        assert!(is_tree_instance_model(
            r"Meshes\LOD\Landscape\Swamp\SwampTree17_lod_2.nif"
        ));
        assert!(is_tree_instance_model(
            r"Meshes\LOD\Landscape\Trees\TreeMapleForest4_lod.nif"
        ));
        assert!(is_tree_instance_model(
            r"Textures\DLC03\LOD\Landscape\Trees\RedPineHalfLODSetLvl2_d.dds"
        ));

        assert!(!is_tree_instance_model(
            r"Meshes\LOD\Landscape\Trees\MtnTopRedPineLog01_lod_0.nif"
        ));
        assert!(!is_tree_instance_model(
            r"Meshes\LOD\Landscape\Trees\TreeForestFallen01_lod_0.nif"
        ));
        assert!(!is_tree_instance_model(
            r"Meshes\LOD\Landscape\Trees\UtilityPowerPole01_lod.nif"
        ));
        assert!(!is_tree_instance_model(
            r"Meshes\LOD\SetDressing\StreetLamps\ResidentialStreetLamp01_lod.nif"
        ));
    }

    #[test]
    fn missing_source_bto_specular_is_neutral_black() {
        let rgba = missing_specular_rgba(2, 1);
        assert_eq!(rgba, vec![0, 0, 0, 255, 0, 0, 0, 255]);
    }

    #[test]
    fn source_bto_atlas_wraps_repeating_uvs_before_remap() {
        let mut geometry = LodGeometry::new();
        geometry.vertices = vec![[0.0, 0.0, 0.0]];
        geometry.set_uvcoords(vec![[1.796875, -0.025970459]]);
        let tile = SourceBtoTile {
            world: "APPALACHIA".to_string(),
            level: 32,
            x: 2,
            y: 3,
            path: PathBuf::from("Appalachia.32.2.3.bto"),
        };
        let quad = source_quad(&tile);
        let mut shape = make_baked_shape(
            "GlobalAtlasShape_AlphaTested".to_string(),
            &tile,
            &quad,
            geometry,
            Default::default(),
            Some((4844, 128)),
            3,
            None,
            [8192.0, 12288.0, 0.0],
            32.0,
        );

        wrap_source_bto_repeating_uvs_for_atlas(&mut shape);

        assert!((shape.geometry.uvcoords[0][0] - 0.796875).abs() < 0.0001);
        assert!((shape.geometry.uvcoords[0][1] - 0.97402954).abs() < 0.0001);
    }

    #[test]
    fn texture_array_split_keeps_one_slice_per_triangle() {
        let mut geom = LodGeometry::new();
        geom.vertices = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [2.0, 1.0, 0.0],
            [2.0, 2.0, 0.0],
        ];
        geom.uvcoords = vec![[0.0, 0.0]; 6];
        geom.bitangents = vec![
            [2.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [9.0, 0.0, 0.0],
            [9.0, 0.0, 0.0],
            [9.0, 0.0, 0.0],
        ];
        geom.triangles = vec![[0, 1, 2], [3, 4, 5]];

        let split = split_texture_array_triangles(&geom);
        assert_eq!(split.len(), 2);
        assert_eq!(split.get(&2).unwrap().num_triangles(), 1);
        assert_eq!(split.get(&9).unwrap().num_triangles(), 1);
    }

    #[test]
    fn baked_shape_remaps_to_hybrid_atlas_page() {
        let mut geometry = LodGeometry::new();
        geometry.vertices = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        geometry.uvcoords = vec![[0.25, 0.25], [0.5, 0.25], [0.25, 0.5]];
        geometry.triangles = vec![[0, 1, 2]];

        let tile = SourceBtoTile {
            world: "APPALACHIA".to_string(),
            level: 16,
            x: -46,
            y: -29,
            path: PathBuf::from("Appalachia.16.-46.-29.bto"),
        };
        let quad = source_quad(&tile);
        let mut textures: [String; 10] = Default::default();
        textures[0] = r"textures\lod\generated\rock_d.dds".to_string();
        textures[1] = r"textures\lod\generated\rock_n.dds".to_string();
        textures[7] = r"textures\lod\generated\rock_s.dds".to_string();
        let mut shape = make_baked_shape(
            "GlobalAtlasShape_NotAlphaTested".to_string(),
            &tile,
            &quad,
            geometry,
            textures,
            None,
            3,
            None,
            [-188416.0, -118784.0, 0.0],
            16.0,
        );

        let mut list = crate::atlas::AtlasList::new();
        let rect = AtlasRect::from_map_row(
            128,
            128,
            256,
            512,
            1024,
            1024,
            r"Textures\Terrain\APPALACHIA\Objects\APPALACHIA.Objects.001.dds",
            false,
        );
        list.insert(
            r"textures\lod\generated\rock_d.dds,textures\lod\generated\rock_n.dds".to_string(),
            rect,
        );
        let atlas = AtlasResult {
            map_path: PathBuf::new(),
            diffuse: PathBuf::new(),
            normal: PathBuf::new(),
            specular: PathBuf::new(),
            atlas_size: (1024, 1024),
            uv: HashMap::new(),
            list,
            dds_written: 3,
        };

        assert!(remap_baked_shape_to_atlas(&mut shape, &atlas));
        assert_eq!(
            shape.textures[0],
            r"Textures\Terrain\APPALACHIA\Objects\APPALACHIA.Objects.001.dds"
        );
        assert_eq!(
            shape.textures[1],
            r"Textures\Terrain\APPALACHIA\Objects\APPALACHIA.Objects.001_n.dds"
        );
        assert_eq!(
            shape.textures[7],
            r"Textures\Terrain\APPALACHIA\Objects\APPALACHIA.Objects.001_s.dds"
        );
        assert!(shape.geometry.uvcoords[0][0] > 0.25 && shape.geometry.uvcoords[0][0] < 0.5);
    }

    #[test]
    fn hybrid_texture_rel_shortens_engine_unsafe_components() {
        let rel = hybrid_texture_rel(
            "APPALACHIA",
            r"textures\lod\generated\scol\seventysix.esm\bld_brick_a_building_houseshotgun_02_open_v2noporchgrass_fixed\bld_brick_a_building_houseshotgun_02_open_v2noporchgrass_fixed_lod_0_d.dds",
            false,
        );

        assert!(rel.starts_with(r"textures\terrain\appalachia\objects\hybrid\lod\generated"));
        assert!(
            rel.split('\\').all(|component| component.len() <= 48),
            "{rel}"
        );
        assert!(rel.ends_with(".dds"));
    }

    #[test]
    fn rewrite_texture_path_promotes_existing_texture_root() {
        let unique = format!(
            "lodgen_fo76_bto_texture_root_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let source = root.join("source");
        let output = root.join("output");
        let texture = source.join("textures/dlc03/lod/landscape/trees/redpine_d.dds");
        std::fs::create_dir_all(texture.parent().unwrap()).unwrap();
        std::fs::write(&texture, b"dds").unwrap();

        let copied = Mutex::new(HashSet::new());
        let mut written = 0;
        let rel = rewrite_texture_path(
            &source,
            &output,
            "APPALACHIA",
            r"Data\dlc03\lod\landscape\trees\redpine_d.dds",
            false,
            &copied,
            &mut written,
        );

        assert_eq!(
            rel,
            r"textures\terrain\appalachia\objects\hybrid\dlc03\lod\landscape\trees\redpine_d.dds"
        );
        assert_eq!(written, 1);
        assert!(output.join(rel.replace('\\', "/")).is_file());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rewrite_texture_path_converts_bc3_srgb_diffuse_to_legacy_dxt5() {
        let unique = format!(
            "lodgen_fo76_bto_texture_bc3_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let source = root.join("source");
        let output = root.join("output");
        let texture = source.join("textures/lod/trees/tree_d.dds");
        std::fs::create_dir_all(texture.parent().unwrap()).unwrap();
        directxtex_native::write_dds_rgba_image(
            &texture,
            8,
            8,
            &vec![200u8, 100, 50, 128].repeat(8 * 8),
            "BC3_UNORM_SRGB",
            true,
        )
        .unwrap();

        let copied = Mutex::new(HashSet::new());
        let mut written = 0;
        let rel = rewrite_texture_path(
            &source,
            &output,
            "APPALACHIA",
            r"LOD\Trees\Tree_d.dds",
            false,
            &copied,
            &mut written,
        );

        let dst = output.join(rel.replace('\\', "/"));
        assert_eq!(written, 1);
        assert_eq!(dds_fourcc(&dst).unwrap(), *b"DXT5");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rewrite_texture_path_converts_bc5_snorm_normal_to_legacy_bc5u() {
        let unique = format!(
            "lodgen_fo76_bto_texture_bc5_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let source = root.join("source");
        let output = root.join("output");
        let texture = source.join("textures/lod/trees/tree_n.dds");
        std::fs::create_dir_all(texture.parent().unwrap()).unwrap();
        directxtex_native::write_dds_rgba_image(
            &texture,
            8,
            8,
            &vec![128u8, 128, 255, 255].repeat(8 * 8),
            "BC5_SNORM",
            true,
        )
        .unwrap();

        let copied = Mutex::new(HashSet::new());
        let mut written = 0;
        let rel = rewrite_texture_path(
            &source,
            &output,
            "APPALACHIA",
            r"LOD\Trees\Tree_n.dds",
            false,
            &copied,
            &mut written,
        );

        let fourcc = dds_fourcc(&output.join(rel.replace('\\', "/"))).unwrap();
        assert_eq!(written, 1);
        assert!(fourcc == *b"ATI2" || fourcc == *b"BC5U", "{fourcc:?}");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rewrite_texture_path_overwrites_existing_unsafe_lod_texture() {
        let unique = format!(
            "lodgen_fo76_bto_texture_overwrite_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let source = root.join("source");
        let output = root.join("output");
        let texture = source.join("textures/lod/trees/tree_d.dds");
        std::fs::create_dir_all(texture.parent().unwrap()).unwrap();
        directxtex_native::write_dds_rgba_image(
            &texture,
            8,
            8,
            &vec![64u8, 128, 192, 128].repeat(8 * 8),
            "BC3_UNORM_SRGB",
            true,
        )
        .unwrap();

        let dst = output.join("textures/terrain/appalachia/objects/hybrid/lod/trees/tree_d.dds");
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::copy(&texture, &dst).unwrap();
        assert_eq!(dds_fourcc(&dst).unwrap(), *b"DX10");

        let copied = Mutex::new(HashSet::new());
        let mut written = 0;
        let rel = rewrite_texture_path(
            &source,
            &output,
            "APPALACHIA",
            r"LOD\Trees\Tree_d.dds",
            false,
            &copied,
            &mut written,
        );

        assert_eq!(
            rel,
            r"textures\terrain\appalachia\objects\hybrid\lod\trees\tree_d.dds"
        );
        assert_eq!(written, 1);
        assert_eq!(dds_fourcc(&dst).unwrap(), *b"DXT5");

        std::fs::remove_dir_all(root).unwrap();
    }
}
