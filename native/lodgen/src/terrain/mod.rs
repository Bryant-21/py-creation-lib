pub mod btd_source;
pub mod delaunay;
pub mod dense_textures;
pub mod terra;
pub mod terrain_lod;
pub mod textures;
pub mod water;

use crate::descriptors::QuadDesc;
use crate::naming;
use crate::output::btr::{build_btr_nif, build_btr_nif_with_water};
use crate::progress::{QuadCtx, QuadOutputs};
use crate::terrain::terrain_lod::{build_terrain_mesh_from_block, terrain_block};
use crate::terrain::textures::{composite_quad, composite_quad_at, lod_index, write_tile_dds};
use crate::terrain::water::build_water_mesh;

/// Generate one terrain LOD quad: mesh + textures.
///
/// Port of `TerrainLOD.GenerateBTR` (TerrainLOD.cs:1319-1500).
pub fn generate_quad(quad: &QuadDesc, ctx: &QuadCtx) -> anyhow::Result<QuadOutputs> {
    generate_quad_with(quad, ctx, None)
}

/// `generate_quad`, compositing the textures from the dense terrain data when `dense` is given.
pub fn generate_quad_with(
    quad: &QuadDesc,
    ctx: &QuadCtx,
    dense: Option<&dense_textures::DenseTerrainSource>,
) -> anyhow::Result<QuadOutputs> {
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

    // TODO(P1-FIDELITY): optimize_unseen and hide_quads are not ported; see
    // build_terrain_mesh in terrain_lod.rs and the one-time warning in driver.rs.
    let block = terrain_block(ctx.world, quad, ctx.settings)?;
    let dense_block = dense.map(|_| block.clone());
    let mesh = build_terrain_mesh_from_block(ctx.world, quad, ctx.settings, block)?;

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
    // (lodLevel * bbox z-center) is applied inside build_btr_nif; the per-level
    // zShift is 0.0 (ZSHIFTLOD* settings are not supported).
    let z_shift = 0.0f32;
    // Landless/ocean water sheet (TerrainLOD.cs GenerateWater): emit a 2nd
    // BSTriShape water block in the same .btr when any cell in the quad sits
    // below water. Shares the terrain block's scale/zShift so the shapes register.
    //
    // WRLD.DATA.NoLODWater overrides the global terrain setting. Emitting a WATER
    // subtree for such a world makes FO4 resolve a surface the WRLD explicitly
    // declares absent, which can crash background terrain loading.
    let water = if ctx.settings.terrain.emit_water && !ctx.world.no_lod_water {
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

    let dense_tile = match (dense, &dense_block) {
        (Some(source), Some(block)) => {
            let size = ctx.settings.terrain.levels[lod_index(level)].diffuse_size.max(1);
            Some(dense_textures::composite_quad_dense(source, quad, block, size)?)
        }
        _ => None,
    };
    let mut textures = vec![diff_path.clone(), msn_path.clone()];
    match dense_tile.filter(|tile| tile.uncovered.iter().any(|&u| !u)) {
        Some(tile) => {
            let land = if tile.uncovered.iter().any(|&u| u) {
                Some(composite_quad_at(ctx.world, quad, ctx.settings, ctx.paths, tile.size, tile.size)?)
            } else {
                None
            };
            let rmaos_rel = naming::terrain_rmaos(world_id, level, quad.x, quad.y, &season);
            let glow_rel = naming::terrain_glow(world_id, level, quad.x, quad.y, &season);
            let rmaos_path = ctx.paths.output_dir.join(rmaos_rel.replace('\\', "/"));
            let glow_path = ctx.paths.output_dir.join(glow_rel.replace('\\', "/"));
            let paths = dense_textures::DenseTilePaths { diffuse: &diff_path, msn: &msn_path, rmaos: &rmaos_path, glow: &glow_path };
            let glows = dense_textures::write_dense_tile(&tile, land.as_ref(), &paths, ctx.settings, level)?;
            textures.push(rmaos_path);
            if glows {
                textures.push(glow_path);
            }
        }
        // Nothing dense covers this quad: FO4CS draws its engine layers there, which the LAND composite matches.
        None => {
            let tile = composite_quad(ctx.world, quad, ctx.settings, ctx.paths)?;
            write_tile_dds(&tile, &diff_path, &msn_path, ctx.settings, level)?;
        }
    }

    Ok(QuadOutputs {
        meshes: vec![btr_path],
        textures,
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
        let mut w = crate::input::WorldspaceInput::from_cells(
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
                    water_height: 100.0,
                })
                .collect(),
        );
        w.no_lod_water = true;
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
        let btr = nif_core_native::model::NifFile::load(&outputs.meshes[0]).unwrap();
        assert!(btr.blocks.iter().all(|block| {
            !matches!(block.fields.get("Name"), Some(nif_core_native::model::NifValue::String(name)) if name == "WATER")
        }));
    }
}
