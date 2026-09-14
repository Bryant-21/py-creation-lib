/// 2D billboard placement → `.btt` writer.
///
/// Port source: TwbLodTES5TreeBlock.AddReference (wbLOD.pas:963-998).
///
/// For each tree ref in the quad: look up its species in the manifest,
/// bucket by tree-list index, push a `TreeRef` with a deterministic rotation.
///
/// Determinism: xLODGen uses `2*Pi*Random` for billboard rotation (wbLOD.pas:994);
/// this uses an FNV-1a hash of the ref_id instead so runs produce identical `.btt`
/// bytes.
use std::collections::BTreeMap;
use std::f32::consts::PI;
use std::path::PathBuf;

use crate::atlas::AtlasResult;
use crate::billboards::BillboardManifest;
use crate::descriptors::QuadDesc;
use crate::input::StaticDesc;
use crate::naming;
use crate::output::btt::{TreeRef, TreeType, write_tree_block};
use crate::progress::{QuadCtx, QuadOutputs};

// ---------------------------------------------------------------------------
// deterministic_rotation — deterministic stand-in for 2*Pi*Random
// ---------------------------------------------------------------------------

/// Deterministic stand-in for xLODGen's `2*Pi*Random` (wbLOD.pas:994): FNV-1a
/// 32-bit over the UTF-8 bytes of `ref_id`, mapped uniformly into `[0, 2π)`.
pub fn deterministic_rotation(ref_id: &str) -> f32 {
    // FNV-1a 32-bit
    const FNV_PRIME: u32 = 16777619;
    const FNV_OFFSET: u32 = 2166136261;
    let mut h = FNV_OFFSET;
    for b in ref_id.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(FNV_PRIME);
    }
    // Map hash uniformly into [0, 2π)
    (h as f64 / u32::MAX as f64) as f32 * 2.0 * PI
}

// ---------------------------------------------------------------------------
// generate_quad — per-quad 2D billboard placement
// ---------------------------------------------------------------------------

/// Per-quad 2D billboard placement.
///
/// For each tree ref in `trees` (sorted by `ref_id`): look up its species in
/// `manifest`, bucket by tree-list index (`BTreeMap` for deterministic ordering), and
/// push a `TreeRef` with position/scale from the `StaticDesc` and a deterministic
/// rotation. Writes the `.btt` block to `ctx.paths.output_dir / naming::btt(...)`.
///
/// Refs whose model is not in the manifest are skipped silently. When `manifest` is
/// `None` this returns empty outputs; the driver warns about the missing manifest.
///
/// port: TwbLodTES5TreeBlock.AddReference (wbLOD.pas:963-998)
pub fn generate_quad(
    quad: &QuadDesc,
    ctx: &QuadCtx<'_>,
    _atlas: &AtlasResult,
    trees: &[&StaticDesc],
    manifest: Option<&BillboardManifest>,
) -> anyhow::Result<QuadOutputs> {
    let manifest = match manifest {
        Some(m) => m,
        None => {
            // Billboard mode needs the generator to have run first.
            // Return empty; the driver warns about the missing manifest.
            return Ok(QuadOutputs::default());
        }
    };

    // Bucket refs by tree-list index (BTreeMap → sorted by index, deterministic).
    // port: TwbLodTES5TreeBlock.AddReference (wbLOD.pas:963-998)
    let mut buckets: BTreeMap<i32, Vec<TreeRef>> = BTreeMap::new();

    // Sort refs by ref_id for deterministic insertion order within each bucket.
    let mut sorted_trees: Vec<&StaticDesc> = trees.to_vec();
    sorted_trees.sort_by(|a, b| a.ref_id.cmp(&b.ref_id));

    for stat in &sorted_trees {
        // Resolve model: the billboard LOD model at level index, or fall back to full_model.
        let level_idx = {
            // Convert quad level to lod_models index: [4,8,16,32] → [0,1,2,3]
            match ctx.level {
                4 => 0,
                8 => 1,
                16 => 2,
                32 => 3,
                _ => 0,
            }
        };
        let model = stat.lod_models[level_idx]
            .as_deref()
            .unwrap_or(&stat.full_model);

        let entry = match manifest.by_model(model) {
            Some(e) => e,
            None => {
                // Unknown species: skip silently.
                continue;
            }
        };

        // Parse form_id from the ref_id hex string; fall back to 0 on parse error.
        let form_id = u32::from_str_radix(stat.ref_id.trim_start_matches("0x"), 16).unwrap_or(0);

        let tree_ref = TreeRef {
            form_id,
            x: stat.pos[0],
            y: stat.pos[1],
            z: stat.pos[2],
            scale: stat.scale,
            // port: wbLOD.pas:994 `Rotation := 2*Pi*Random;` — deterministic hash replacement
            rotation: deterministic_rotation(&stat.ref_id),
        };

        buckets.entry(entry.index).or_default().push(tree_ref);
    }

    // Nothing placed — return empty (all refs had unknown species).
    if buckets.is_empty() {
        return Ok(QuadOutputs::default());
    }

    // Build the ordered type list from the sorted BTreeMap.
    let types: Vec<TreeType> = buckets
        .into_iter()
        .map(|(index, refs)| TreeType { index, refs })
        .collect();

    // Write the .btt block.
    // port: TwbLodTES5TreeBlock.SaveToFile (wbLOD.pas:939-961)
    let btt_rel = naming::btt(&ctx.world.editor_id, ctx.level, quad.x, quad.y);
    let btt_path = ctx.paths.output_dir.join(btt_rel.replace('\\', "/"));
    write_tree_block(&btt_path, &types)?;

    Ok(QuadOutputs {
        meshes: vec![btt_path],
        textures: vec![],
        object_lod: None,
    })
}
