//! Checks `terrain.source = btd` against real data (plan Task 18):
//! 1. which BTD sample offset matches today's LAND-built `.btr` border posts (expected: half a cell, 64),
//! 2. that `assemble_block_from_btd` agrees with that sampler,
//! 3. the cost of opening the BTD per quad against reusing one instance, and the reused cache's size.
//! Usage: btd_terrain_check <Appalachia.btd> <terrain dir with L4 .btr files> <tile count>
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Context;
use lodgen_native::audit::{CELL, parse_tile_name, terrain_height_at};
use lodgen_native::descriptors::QuadDesc;
use lodgen_native::input::WorldspaceInput;
use lodgen_native::settings::TerrainSource;
use lodgen_native::terrain::btd_source::assemble_block_from_btd;
use lodgen_native::terrain::terrain_lod::build_terrain_mesh;
use terrain_native::btd::BtdFile;

const SHIFTS: [i64; 3] = [0, 64, 128];

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(args.len() == 3, "usage: btd_terrain_check <btd> <terrain dir> <tile count>");
    let (btd_path, dir, count) = (&args[0], PathBuf::from(&args[1]), args[2].parse::<usize>()?);
    let mut tiles: Vec<(PathBuf, i32, i32)> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?.to_string();
            let (_, level, x, y) = parse_tile_name(&format!("{}.bto", name.strip_suffix(".btr")?))?;
            (level == 4).then_some((p, x, y))
        })
        .collect();
    tiles.sort();
    let step = (tiles.len() / count).max(1);
    let tiles: Vec<_> = tiles.into_iter().step_by(step).take(count).collect();

    let mut btd = BtdFile::open(btd_path).map_err(|e| anyhow::anyhow!("{e}"))?;
    let header = btd.header().clone();
    println!(
        "BTD cells x {}..{} y {}..{}, heights {}..{}",
        header.cell_min_x, header.cell_max_x, header.cell_min_y, header.cell_max_y,
        header.world_height_min, header.world_height_max
    );
    let scale = (header.world_height_max - header.world_height_min) / f32::from(u16::MAX);
    let sample = |btd: &mut BtdFile, wx: f32, wy: f32, shift: i64| -> anyhow::Result<Option<f32>> {
        let global = |w: f32, min: i32| (w / 32.0).round() as i64 - i64::from(min) * 128 + shift;
        let (gx, gy) = (global(wx, header.cell_min_x), global(wy, header.cell_min_y));
        let (cx, cy) = (header.cell_min_x + gx.div_euclid(128) as i32, header.cell_min_y + gy.div_euclid(128) as i32);
        if cx < header.cell_min_x || cx > header.cell_max_x || cy < header.cell_min_y || cy > header.cell_max_y {
            return Ok(None);
        }
        let raw = btd.cell_height_map_u16(cx, cy, 0).map_err(|e| anyhow::anyhow!("{e}"))?
            [gx.rem_euclid(128) as usize + gy.rem_euclid(128) as usize * 128];
        Ok(Some(header.world_height_min + f32::from(raw) * scale))
    };

    let mut errors = [Vec::new(), Vec::new(), Vec::new()];
    let mut block_vs_sampler = 0f32;
    let mut worst = String::new();
    let world = WorldspaceInput::from_cells("APPALACHIA", Vec::new());
    for (path, x, y) in &tiles {
        let terrain = std::fs::read(path)?;
        let origin = [*x as f32 * CELL, *y as f32 * CELL];
        let edge = (0..=16).map(|k| k as f32 * 1024.0);
        let border: Vec<[f32; 2]> = edge
            .clone()
            .flat_map(|k| [[k, 0.0], [k, 4.0 * CELL], [0.0, k], [4.0 * CELL, k]])
            .collect();
        for local in &border {
            let (wx, wy) = (origin[0] + local[0], origin[1] + local[1]);
            let Some(land) = terrain_height_at(&terrain, origin, wx, wy)? else { continue };
            for (i, shift) in SHIFTS.iter().enumerate() {
                if let Some(h) = sample(&mut btd, wx, wy, *shift)? {
                    errors[i].push((h - land).abs());
                }
            }
        }
        let quad = QuadDesc {
            z_order: 0,
            x: *x,
            y: *y,
            quad_level: 4,
            quad_index: 0,
            quad_offset: 4.0 * CELL,
            static_indices: Vec::new(),
            statics: Vec::new(),
            out_values: Default::default(),
        };
        let block = assemble_block_from_btd(&mut btd, &world, &quad)?;
        for by in (0..block.size).step_by(37) {
            for bx in (0..block.size).step_by(41) {
                let (wx, wy) = (origin[0] + bx as f32 * 32.0, origin[1] + by as f32 * 32.0);
                if let Some(h) = sample(&mut btd, wx, wy, 64)? {
                    let diff = (block.heights[bx + by * block.size] * 4.0 - h).abs();
                    if diff > block_vs_sampler {
                        block_vs_sampler = diff;
                        worst = format!("tile {x},{y} post {bx},{by} world {wx},{wy} cell {},{}", (wx / CELL).floor(), (wy / CELL).floor());
                    }
                }
            }
        }
    }
    for (i, shift) in SHIFTS.iter().enumerate() {
        let e = &mut errors[i];
        e.sort_by(f32::total_cmp);
        let mean = e.iter().sum::<f32>() / e.len().max(1) as f32;
        println!(
            "shift {shift:>3}: {} border posts, mean |BTD - LAND| {mean:.1}, median {:.1}, p95 {:.1}, p99 {:.1}, over 100: {}",
            e.len(),
            e.get(e.len() / 2).copied().unwrap_or(0.0),
            e.get(e.len() * 95 / 100).copied().unwrap_or(0.0),
            e.get(e.len() * 99 / 100).copied().unwrap_or(0.0),
            e.iter().filter(|v| **v > 100.0).count()
        );
    }
    println!("assemble_block_from_btd vs the shift-64 sampler: max diff {block_vs_sampler} at {worst}");

    let quads: Vec<QuadDesc> = tiles
        .iter()
        .map(|(_, x, y)| QuadDesc {
            z_order: 0,
            x: *x,
            y: *y,
            quad_level: 4,
            quad_index: 0,
            quad_offset: 4.0 * CELL,
            static_indices: Vec::new(),
            statics: Vec::new(),
            out_values: Default::default(),
        })
        .collect();
    let started = Instant::now();
    for quad in &quads {
        let mut fresh = BtdFile::open(btd_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        assemble_block_from_btd(&mut fresh, &world, quad).context("per-quad open")?;
    }
    let per_quad = started.elapsed().as_secs_f64() / quads.len() as f64;
    let mut shared = BtdFile::open(btd_path).map_err(|e| anyhow::anyhow!("{e}"))?;
    let started = Instant::now();
    for quad in &quads {
        assemble_block_from_btd(&mut shared, &world, quad).context("shared")?;
    }
    let reused = started.elapsed().as_secs_f64() / quads.len() as f64;
    let stats = shared.tile_cache_stats();
    println!(
        "per-quad open {:.1} ms/quad; reused instance {:.1} ms/quad, cache {} tiles / {:.0} MB after {} quads",
        per_quad * 1000.0,
        reused * 1000.0,
        stats.cached_tiles,
        stats.cached_payload_bytes as f64 / 1_048_576.0,
        quads.len()
    );

    for (level, n) in [(4, 10usize), (8, 5), (16, 3), (32, 2)] {
        let mut settings = lodgen_native::settings::LodSettings::fo4_default();
        let inputs = quads
            .iter()
            .take(n)
            .map(|quad| {
                let quad = QuadDesc { quad_level: level, ..quad.clone() };
                Ok((land_world(&mut btd, &quad)?, quad))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        for source in [TerrainSource::Land, TerrainSource::Btd] {
            settings.terrain.source = source;
            settings.terrain.btd_path = Some(btd_path.clone());
            let started = Instant::now();
            let (mut verts, mut tris) = (0, 0);
            for (land, quad) in &inputs {
                let mesh = build_terrain_mesh(land, quad, &settings)?;
                verts += mesh.verts.len();
                tris += mesh.tris.len();
            }
            println!(
                "L{level} {source:?}: {:.0} ms/quad, {} verts, {} tris per quad",
                started.elapsed().as_secs_f64() * 1000.0 / n as f64,
                verts / n,
                tris / n
            );
        }
    }
    Ok(())
}

/// LAND posts for a quad's cells, sampled from the BTD the way the converter does (post i = sample 4i).
fn land_world(btd: &mut BtdFile, quad: &QuadDesc) -> anyhow::Result<WorldspaceInput> {
    let level = quad.quad_level;
    let mut one = QuadDesc { quad_level: 4, ..quad.clone() };
    let mut cells = Vec::new();
    for cy in 0..level / 4 {
        for cx in 0..level / 4 {
            one.x = quad.x + cx * 4;
            one.y = quad.y + cy * 4;
            let block = assemble_block_from_btd(btd, &WorldspaceInput::from_cells("W", Vec::new()), &one)?;
            for y in 0..4 {
                for x in 0..4 {
                    let heights = (0..33 * 33)
                        .map(|i| block.heights[(x * 128 + (i % 33) * 4) + (y * 128 + (i / 33) * 4) * block.size] * 4.0)
                        .collect();
                    cells.push(lodgen_native::input::CellInput {
                        x: one.x + x as i32,
                        y: one.y + y as i32,
                        heights,
                        vertex_colors: Vec::new(),
                        layers: Vec::new(),
                        hidden_quadrants: [false; 4],
                        water_height: f32::MIN,
                    });
                }
            }
        }
    }
    Ok(WorldspaceInput::from_cells("APPALACHIA", cells))
}
