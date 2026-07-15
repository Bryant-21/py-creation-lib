pub mod delaunay;
pub mod terra;
pub mod terrain_lod;
pub mod textures;
pub mod water;

use crate::descriptors::QuadDesc;
use crate::naming;
use crate::output::btr::{build_btr_nif, build_btr_nif_with_water};
use crate::progress::{QuadCtx, QuadOutputs};
use crate::terrain::terrain_lod::build_terrain_mesh;
use crate::terrain::textures::{composite_quad, write_tile_dds};
use crate::terrain::water::build_water_mesh;

/// Generate one terrain LOD quad: mesh + textures.
///
/// Contract §Per-type generators: `terrain::generate_quad(quad, ctx) -> QuadOutputs`.
/// Port of `TerrainLOD.GenerateBTR` (TerrainLOD.cs:1319-1500).
pub fn generate_quad(quad: &QuadDesc, ctx: &QuadCtx) -> anyhow::Result<QuadOutputs> {
    let level = quad.quad_level;
    let world_id = &ctx.world.editor_id;
    let season = ctx
        .settings
        .global
        .season
        .as_deref()
        .map(|s| {
            if s.is_empty() {
                "".to_string()
            } else {
                format!(".{s}")
            }
        })
        .unwrap_or_default();

    // TODO(P1-FIDELITY): per-quad fidelity passes (protect_cell_borders / skirts
    // geometry / optimize_unseen / hide_quads) are not run here — see the marker
    // in terrain_lod.rs and the one-time stats.warnings entry in driver.rs.
    // Build terrain mesh.
    let mesh = build_terrain_mesh(ctx.world, quad, ctx.settings)?;

    // Early-return if no geometry (TerrainLOD.cs:1322-1325).
    if mesh.verts.is_empty() {
        return Ok(QuadOutputs::default());
    }

    // Resolve output paths (Data-relative names joined onto output_dir).
    let btr_rel = naming::btr(world_id, level, quad.x, quad.y);
    let diff_rel = naming::terrain_diffuse(world_id, level, quad.x, quad.y, &season);
    let msn_rel = naming::terrain_msn(world_id, level, quad.x, quad.y, &season);

    let btr_path = ctx.paths.output_dir.join(btr_rel.replace('\\', "/"));
    let diff_path = ctx.paths.output_dir.join(diff_rel.replace('\\', "/"));
    let msn_path = ctx.paths.output_dir.join(msn_rel.replace('\\', "/"));

    // Write .btr mesh (scale = lodLevel per TerrainLOD.cs:1433). The ShiftZ term
    // (lodLevel * bbox z-center) is applied inside build_btr_nif; we pass the
    // per-level zShift, which is 0.0 by default (Phase-1: ZSHIFTLOD* unsupported,
    // see TODO(P1-FIDELITY) in terrain_lod.rs).
    let z_shift = 0.0f32;
    // Landless/ocean water sheet (TerrainLOD.cs GenerateWater): emit a 2nd
    // BSTriShape water block in the same .btr when any cell in the quad sits
    // below water. Shares the terrain block's scale/zShift so the shapes register.
    //
    // Gated behind `terrain.emit_water` (default OFF): the WATER block is
    // byte-faithful to the golden LODGen .btr, but FO4 resolves the water-LOD
    // surface shader from the worldspace water type, which a raw FO76→FO4
    // converted worldspace does not synthesize — so emitting it crashes the
    // BSBatchRenderer (null-deref walking the "WATER" node). See settings.rs.
    let water = if ctx.settings.terrain.emit_water {
        build_water_mesh(ctx.world, quad)
    } else {
        None
    };
    let mut nif = match &water {
        Some(w) => build_btr_nif_with_water(
            &mesh.verts,
            &mesh.uvs,
            &mesh.tris,
            &diff_rel,
            &msn_rel,
            &mesh.bbox,
            level as f32,
            z_shift,
            w,
        )?,
        None => build_btr_nif(
            &mesh.verts,
            &mesh.uvs,
            &mesh.tris,
            &diff_rel,
            &msn_rel,
            &mesh.bbox,
            level as f32,
            z_shift,
        )?,
    };
    if let Some(parent) = btr_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    nif.save(Some(btr_path.clone()))
        .map_err(|e| anyhow::anyhow!("btr save: {e}"))?;

    // Composite + write diffuse/_msn tiles.
    let tile = composite_quad(ctx.world, quad, ctx.settings, ctx.paths)?;
    write_tile_dds(&tile, &diff_path, &msn_path, ctx.settings, level)?;

    Ok(QuadOutputs {
        meshes: vec![btr_path],
        textures: vec![diff_path, msn_path],
        object_lod: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::quads_for;
    use crate::game::Game;
    use crate::progress::LodPaths;
    use crate::settings::LodSettings;

    #[test]
    fn generate_quad_emits_btr_and_dds() {
        let w = crate::input::WorldspaceInput::from_cells(
            "W",
            (0..4)
                .flat_map(|y| (0..4).map(move |x| (x, y)))
                .map(|(x, y)| crate::input::CellInput {
                    x,
                    y,
                    heights: vec![0.0; 33 * 33],
                    vertex_colors: vec![[255, 255, 255]; 33 * 33],
                    layers: Vec::new(),
                    hidden_quadrants: [false; 4],
                    water_height: f32::MIN,
                })
                .collect(),
        );
        let s = LodSettings::fo4_default();
        let g = Game::fo4();
        let out_dir = std::env::temp_dir().join("lodgen_genquad_test");
        std::fs::create_dir_all(&out_dir).unwrap();
        let paths = LodPaths {
            data_dirs: vec![std::path::PathBuf::from(".")],
            output_dir: out_dir.clone(),
            source_data_dir: None,
        };
        let quad = quads_for(&w, 4, &s)
            .into_iter()
            .find(|q| q.x == 0 && q.y == 0)
            .unwrap();
        let ctx = QuadCtx {
            world: &w,
            settings: &s,
            game: &g,
            paths: &paths,
            level: 4,
        };

        let outputs = generate_quad(&quad, &ctx).unwrap();
        assert_eq!(outputs.meshes.len(), 1);
        assert_eq!(outputs.textures.len(), 2); // diffuse + _msn
        assert!(outputs.meshes[0].exists());
        assert!(outputs.meshes[0].to_string_lossy().ends_with("W.4.0.0.btr"));
        assert!(outputs.textures.iter().all(|t| t.exists()));
    }
}
