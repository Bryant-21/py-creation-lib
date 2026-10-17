//! Terrain LOD heights from FO76's BTD height pyramid (`terrain.source = btd`), up to 4× denser per axis
//! than LAND's 33 posts per cell.
//!
//! FO4 cell (x, y) starts half a BTD cell into BTD cell (x, y) (`terrain_native::fo4_frame`): its post i
//! is global BTD sample `i + samples/2`, counted from the BTD's minimum cell.

use std::collections::HashMap;

use terrain_native::btd::BtdFile;

use crate::descriptors::QuadDesc;
use crate::input::WorldspaceInput;
use crate::settings::{LodSettings, TerrainSource};
use crate::terrain::terrain_lod::{LAND_POSTS_PER_CELL, TerrainBlock, block_size};

const BTD_CELL_SAMPLES: usize = 128;

/// The part of a BTD header the block assembly needs.
pub struct BtdGrid {
    pub cell_min: [i32; 2],
    pub cell_max: [i32; 2],
    pub height_min: f32,
    /// World units per raw u16 step.
    pub height_scale: f32,
}

/// Posts per cell for a level and the BTD lod that supplies them, keeping each block within 1025 posts.
pub fn posts_per_cell_for_level(level: usize) -> (usize, u8) {
    let ppc = (1024 / level).min(BTD_CELL_SAMPLES);
    (ppc, (BTD_CELL_SAMPLES / ppc).trailing_zeros() as u8)
}

/// Why a `terrain.source = btd` run can't use its BTD, if it can't: no path, an unreadable file, the Starfield
/// layout, or corner cells that don't decode. Checked once per run, so a bad file costs one warning instead of
/// every terrain quad.
pub fn unusable_btd(settings: &LodSettings) -> Option<String> {
    if settings.terrain.source != TerrainSource::Btd {
        return None;
    }
    let Some(path) = settings.terrain.btd_path.as_deref() else {
        return Some("terrain.source=btd has no BTD path".to_string());
    };
    let mut btd = match BtdFile::open(path) {
        Ok(btd) => btd,
        Err(e) => return Some(format!("cannot read BTD {path}: {e}")),
    };
    let header = btd.header();
    if header.is_starfield_layout {
        return Some(format!("BTD {path} has the Starfield layout"));
    }
    let corners = [(header.cell_min_x, header.cell_min_y), (header.cell_max_x, header.cell_max_y)];
    for (x, y) in corners {
        if let Err(e) = btd.cell_height_map_u16(x, y, 0) {
            return Some(format!("BTD {path}: cell {x},{y} does not decode: {e}"));
        }
    }
    None
}

pub fn assemble_block_from_btd(
    btd: &mut BtdFile,
    world: &WorldspaceInput,
    quad: &QuadDesc,
) -> anyhow::Result<TerrainBlock> {
    let header = btd.header();
    anyhow::ensure!(
        !header.is_starfield_layout,
        "terrain.source=btd reads FO76 BTDs; this one has the Starfield layout"
    );
    let grid = BtdGrid {
        cell_min: [header.cell_min_x, header.cell_min_y],
        cell_max: [header.cell_max_x, header.cell_max_y],
        height_min: header.world_height_min,
        height_scale: (header.world_height_max - header.world_height_min) / f32::from(u16::MAX),
    };
    assemble_block_with(&grid, |x, y, lod| Ok(btd.cell_height_map_u16(x, y, lod)?), world, quad)
}

/// Block assembly over any source of BTD cells; `cell_samples(x, y, lod)` returns that cell's row-major
/// `(128 >> lod)²` raw heights. A cell that overlaps the BTD repeats the edge sample where the BTD ends;
/// cells outside it fall back to their LAND posts, upsampled, or 0.
pub fn assemble_block_with(
    grid: &BtdGrid,
    mut cell_samples: impl FnMut(i32, i32, u8) -> anyhow::Result<Vec<u16>>,
    world: &WorldspaceInput,
    quad: &QuadDesc,
) -> anyhow::Result<TerrainBlock> {
    let level = quad.quad_level as usize;
    let (ppc, lod) = posts_per_cell_for_level(level);
    let size = block_size(level, ppc);
    let div = level as f32;
    let quad_cell = [quad.x, quad.y];
    let span = [0, 1].map(|a| (grid.cell_max[a] - grid.cell_min[a] + 1) as i64 * ppc as i64);
    let land: HashMap<(i32, i32), &crate::input::CellInput> =
        world.cells.iter().map(|c| ((c.x, c.y), c)).collect();
    let mut samples: HashMap<(i32, i32), Vec<u16>> = HashMap::new();
    let mut heights = vec![0.0f32; size * size];
    let mut water = vec![0.0f32; size * size];

    for by in 0..size {
        for bx in 0..size {
            let post = [bx, by];
            // Shared edge posts belong to the later cell, the block's last post to the last cell (as LAND).
            let cell = [0, 1].map(|a| quad_cell[a] + (post[a] / ppc).min(level - 1) as i32);
            // FO4 cell c reads BTD cells c and c+1, so the cell before the BTD still overlaps it.
            let in_btd = (0..2).all(|a| (grid.cell_min[a] - 1..=grid.cell_max[a]).contains(&cell[a]));
            let index = bx + by * size;
            let land_cell = land.get(&(cell[0], cell[1]));
            water[index] = land_cell.map_or(world.water_height, |c| c.water_height) / div;
            heights[index] = if in_btd {
                let global = [0, 1].map(|a| {
                    let frame = quad_cell[a] as i64 * ppc as i64 + post[a] as i64;
                    (frame - grid.cell_min[a] as i64 * ppc as i64 + (ppc / 2) as i64).clamp(0, span[a] - 1)
                });
                let source = [0, 1].map(|a| grid.cell_min[a] + (global[a] / ppc as i64) as i32);
                let local = [0, 1].map(|a| (global[a] % ppc as i64) as usize);
                let raw = match samples.entry((source[0], source[1])) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => e.insert(cell_samples(source[0], source[1], lod)?),
                }[local[0] + local[1] * ppc];
                (grid.height_min + f32::from(raw) * grid.height_scale) / div
            } else if let Some(c) = land_cell {
                let within = [0, 1].map(|a| (post[a] - (cell[a] - quad_cell[a]) as usize * ppc) as f32);
                land_height(&c.heights, within.map(|p| p * LAND_POSTS_PER_CELL as f32 / ppc as f32)) / div
            } else {
                0.0
            };
        }
    }
    Ok(TerrainBlock { size, posts_per_cell: ppc, heights, water })
}

/// Bilinear height at fractional LAND post coordinates in a 33×33 row-major grid.
fn land_height(posts: &[f32], at: [f32; 2]) -> f32 {
    let n = LAND_POSTS_PER_CELL;
    let [x0, y0] = at.map(|v| (v.floor() as usize).min(n - 1));
    let [fx, fy] = [at[0] - x0 as f32, at[1] - y0 as f32];
    let h = |x: usize, y: usize| posts[x + y * (n + 1)];
    let top = h(x0, y0) * (1.0 - fx) + h(x0 + 1, y0) * fx;
    let bottom = h(x0, y0 + 1) * (1.0 - fx) + h(x0 + 1, y0 + 1) * fx;
    top * (1.0 - fy) + bottom * fy
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::QuadDesc;
    use crate::input::{CellInput, WorldspaceInput};

    fn quad(x: i32, y: i32, level: i32) -> QuadDesc {
        QuadDesc {
            z_order: 0,
            x,
            y,
            quad_level: level,
            quad_index: 0,
            quad_offset: level as f32 * 4096.0,
            static_indices: Vec::new(),
            statics: Vec::new(),
            out_values: Default::default(),
        }
    }

    /// Cells -2..=1 on both axes; every sample's raw value is its global BTD sample index along x.
    fn ramp_grid() -> BtdGrid {
        BtdGrid { cell_min: [-2, -2], cell_max: [1, 1], height_min: 0.0, height_scale: 1.0 }
    }

    fn ramp_cell(cell_x: i32, _cell_y: i32, lod: u8) -> anyhow::Result<Vec<u16>> {
        let samples = 128usize >> lod;
        let base = (cell_x + 2) as usize * samples;
        Ok((0..samples * samples).map(|i| (base + i % samples) as u16).collect())
    }

    fn empty_world() -> WorldspaceInput {
        WorldspaceInput::from_cells("W", Vec::new())
    }

    #[test]
    fn lod_choice_keeps_blocks_within_1025() {
        assert_eq!(posts_per_cell_for_level(4), (128, 0));
        assert_eq!(posts_per_cell_for_level(8), (128, 0));
        assert_eq!(posts_per_cell_for_level(16), (64, 1));
        assert_eq!(posts_per_cell_for_level(32), (32, 2));
    }

    #[test]
    fn fo4_cell_starts_half_a_btd_cell_in() {
        // FO4 cell -2's first post is BTD global sample 64: half a cell in (fo4_frame.rs).
        let block = assemble_block_with(&ramp_grid(), ramp_cell, &empty_world(), &quad(-2, -2, 4)).unwrap();
        assert_eq!(block.posts_per_cell, 128);
        assert_eq!(block.size, 513);
        // Block heights are divided by the level, like assemble_block.
        assert_eq!(block.heights[0] * 4.0, 64.0);
        assert_eq!(block.heights[1] * 4.0, 65.0);
        assert_eq!(block.heights[256] * 4.0, 320.0);
    }

    #[test]
    fn samples_past_the_btd_edge_repeat_the_last_one() {
        let block = assemble_block_with(&ramp_grid(), ramp_cell, &empty_world(), &quad(-2, -2, 4)).unwrap();
        // The quad's last post column is FO4-frame sample 512 → BTD 576, past the 512-sample grid.
        assert_eq!(block.heights[512] * 4.0, 511.0);
    }

    #[test]
    fn the_cell_before_the_btd_reads_its_overlap() {
        // FO4 cell -3's upper half lies over BTD cell -2's lower half; its lower half repeats sample 0.
        let block = assemble_block_with(&ramp_grid(), ramp_cell, &empty_world(), &quad(-3, -2, 4)).unwrap();
        assert_eq!(block.heights[100] * 4.0, 36.0);
        assert_eq!(block.heights[10] * 4.0, 0.0);
    }

    #[test]
    fn coarser_levels_read_the_btd_pyramid() {
        let block = assemble_block_with(&ramp_grid(), ramp_cell, &empty_world(), &quad(-2, -2, 32)).unwrap();
        assert_eq!(block.posts_per_cell, 32);
        assert_eq!(block.size, 1025);
        // lod 2: 32 samples per cell, half-cell shift 16.
        assert_eq!(block.heights[0] * 32.0, 16.0);
    }

    #[test]
    fn cells_outside_the_btd_fall_back_to_land_then_zero() {
        let land = CellInput {
            x: 2,
            y: -2,
            heights: (0..33 * 33).map(|i| (i % 33) as f32 * 10.0).collect(),
            vertex_colors: Vec::new(),
            layers: Vec::new(),
            hidden_quadrants: [false; 4],
            water_height: 50.0,
        };
        let world = WorldspaceInput::from_cells("W", vec![land]);
        let block = assemble_block_with(&ramp_grid(), ramp_cell, &world, &quad(-2, -2, 8)).unwrap();
        let size = block.size;
        // Cell x=2 is block cell column 4 (posts 512..640). LAND post 1 (10 units) sits at block post 516.
        assert_eq!(block.heights[516] * 8.0, 10.0);
        // Halfway between LAND posts 1 and 2: upsampled linearly.
        assert_eq!(block.heights[518] * 8.0, 15.0);
        assert_eq!(block.water[516], 50.0 / 8.0);
        // Cell x=3 has neither BTD nor LAND.
        assert_eq!(block.heights[5 * 128 + 10], 0.0);
        assert_eq!(block.heights[size - 1], 0.0);
    }
}
