/// Tree LOD — dispatcher and classifier.
///
/// Port sources: LODApp.cs (billboard/tree routing), ShapeDesc.cs (flags).
pub mod billboard_place;
pub mod tree3d;

use crate::atlas::AtlasResult;
use crate::descriptors::QuadDesc;
use crate::input::StaticDesc;
use crate::progress::{QuadCtx, QuadOutputs};

/// A `StaticDesc` is a "tree" for LOD purposes if:
/// - Its `is_billboard` flag is set (LOD model is a billboard `.dds`, LODApp.cs:1391), OR
/// - Its `base_flags` has the IS_TREE shape flag (0x1000, ShapeDesc.cs / ShapeFlags.cs).
///
/// TREE base records route through the same static path as 3D objects, per the decisive corpus
/// fact: FO4 default tree LOD is 3D and folded into `.bto`. Billboard-model trees set
/// `is_billboard=true` and build FlatTrunk quads inside the same `.bto`.
pub fn is_tree(stat: &StaticDesc) -> bool {
    // LODApp.cs:1391 — .dds LOD model → isBillboard=true on the ShapeDesc
    if stat.is_billboard {
        return true;
    }
    // ShapeFlags::IS_TREE = 0x1000 (ShapeFlags.cs); mapped from base record type
    if stat.base_flags & 0x1000 != 0 {
        return true;
    }
    false
}

/// Contract per-type generator. Dispatches 3D vs billboard by `settings.trees.trees_3d`.
///
/// Collects tree refs from `quad`, returns `QuadOutputs::default()` if none.
/// Delegates to `tree3d::generate_quad` (3D default) or `billboard_place::generate_quad` (2D).
///
/// In billboard mode (`trees_3d=false`), loads the `BillboardManifest` from
/// `naming::billboard_manifest(world)` resolved against `ctx.paths` (output_dir first,
/// then data_dirs). If absent, emits a warning and returns empty outputs.
pub fn generate_quad(
    quad: &QuadDesc,
    ctx: &QuadCtx<'_>,
    atlas: &AtlasResult,
) -> anyhow::Result<QuadOutputs> {
    let trees: Vec<&StaticDesc> = quad.static_refs(ctx.world).filter(|s| is_tree(s)).collect();
    if trees.is_empty() {
        return Ok(QuadOutputs::default());
    }
    if ctx.settings.trees.trees_3d {
        tree3d::generate_quad(quad, ctx, atlas, &trees)
    } else {
        // Billboard mode: load the manifest once per quad (driver pre-loads; this is the
        // per-quad fallback path).
        let manifest = load_billboard_manifest(ctx);
        if manifest.is_none() {
            // Manifest absent — billboard mode needs the Python generator to have run first.
            // Return empty, let the driver surface the warning.
            return Ok(QuadOutputs {
                meshes: vec![],
                textures: vec![],
                object_lod: None,
            });
        }
        billboard_place::generate_quad(quad, ctx, atlas, &trees, manifest.as_ref())
    }
}

/// Try to load the `BillboardManifest` for the current world.
///
/// Searches `ctx.paths.output_dir` first, then each `data_dirs` entry.
/// The manifest file is named `{world}_billboard_manifest.json`.
///
/// Returns `None` (and logs nothing — caller decides on the warning) if absent.
pub fn load_billboard_manifest(ctx: &QuadCtx<'_>) -> Option<crate::billboards::BillboardManifest> {
    let filename = crate::naming::billboard_manifest(&ctx.world.editor_id);
    // output_dir first, then data_dirs
    let search_dirs = std::iter::once(&ctx.paths.output_dir).chain(ctx.paths.data_dirs.iter());
    for dir in search_dirs {
        let path = dir.join(filename.replace('\\', "/"));
        if path.is_file() {
            match crate::billboards::BillboardManifest::load(&path) {
                Ok(m) => return Some(m),
                Err(_) => continue,
            }
        }
    }
    None
}
