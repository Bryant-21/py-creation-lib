//! FO4-frame assembly over BTD source data.
//!
//! BTD cell (x,y) data covers world [x*4096-2048, x*4096+2048) per axis
//! (a half-cell offset). Under the identity frame (FO4 world == FO76 world), FO4
//! cell (x,y):
//!   - heights: global source sample shifted by +HALF_CELL_SAMPLES per axis
//!   - quadrant data (texture sets, alphas, gcvr): FO4 quadrant (qx,qy) is
//!     BTD cell (x+qx, y+qy) quadrant (1-qx, 1-qy).
//!
//! Under the Starfield frame BTD cells are 100m, so there is no opposite-corner
//! stitch; each FO4 cell/sample maps through `sf_frame` into the BTD's own
//! sample lattice.

use crate::btd::{BtdError, BtdFile, CellTextureSet, QuadrantTextureSet};

pub const HALF_CELL_SAMPLES: usize = 64;
pub const CELL_SAMPLES: usize = 128;

/// BTD neighbor cell delta + source quadrant for an FO4 quadrant (qx, qy in 0..2).
pub fn source_quadrant(qx: usize, qy: usize) -> ((i32, i32), (usize, usize)) {
    ((qx as i32, qy as i32), (1 - qx, 1 - qy))
}

/// SF (100m) cell + which half (0 = low, 1 = high) of that cell's own 50m
/// span an FO4-unit position falls in. Shared by
/// `assemble_cell_texture_set`'s Starfield branch and
/// `global_blend::quadrant_base_source_ltex_object_ids`'s tie-break.
pub(crate) fn sf_cell_and_half_for_units(units: f64) -> (i32, u8) {
    let meters = crate::sf_frame::fo4_units_to_meters(units);
    let cell = (meters / crate::sf_frame::SF_CELL_METERS).floor() as i32;
    let within_cell = meters - cell as f64 * crate::sf_frame::SF_CELL_METERS;
    let half = u8::from(within_cell >= crate::sf_frame::SF_CELL_METERS / 2.0);
    (cell, half)
}

/// Nearest-neighbour global BTD sample index (relative to `btd_cell_min`,
/// non-negative) for a per-sample position `local` (0..CELL_SAMPLES) within
/// FO4 cell `cell`. Shared by `assemble_cell_grid`'s Starfield branch and
/// `authoring_emit::dense_cell_texture_object_ids`'s Starfield addressing.
pub(crate) fn starfield_global_sample_index(cell: i32, local: usize, btd_cell_min: i32) -> i32 {
    let stride_units = crate::sf_frame::FO4_CELL_UNITS / CELL_SAMPLES as f64;
    let units = cell as f64 * crate::sf_frame::FO4_CELL_UNITS + local as f64 * stride_units;
    crate::sf_frame::fo4_units_to_btd_sample(units, btd_cell_min)
        .round()
        .max(0.0) as i32
}

/// FO76 identity: taken from four BTD neighbors' opposite quadrants (see module
/// docs). Starfield: each FO4 quadrant takes the SF (cell, quadrant) containing
/// its centre. Source cells are clamped to BTD bounds, so the worldspace edge
/// reuses the nearest in-range cell instead of erroring.
pub fn assemble_cell_texture_set(
    btd: &BtdFile,
    cell_x: i32,
    cell_y: i32,
) -> Result<CellTextureSet, BtdError> {
    let header = btd.header();
    let (min_x, min_y) = (header.cell_min_x, header.cell_min_y);
    let (max_x, max_y) = (header.cell_max_x, header.cell_max_y);
    if header.is_starfield_layout {
        let mut quadrants: Vec<QuadrantTextureSet> = Vec::with_capacity(4);
        for q in 0..4usize {
            let qx = q & 1;
            let qy = (q >> 1) & 1;
            let centre_x_units = crate::sf_frame::fo4_land_vertex_units(cell_x, qx * 16 + 8);
            let centre_y_units = crate::sf_frame::fo4_land_vertex_units(cell_y, qy * 16 + 8);
            let (sf_cell_x, half_x) = sf_cell_and_half_for_units(centre_x_units);
            let (sf_cell_y, half_y) = sf_cell_and_half_for_units(centre_y_units);
            let sf_quadrant = ((half_y << 1) | half_x) as usize;
            let nx = sf_cell_x.clamp(min_x, max_x);
            let ny = sf_cell_y.clamp(min_y, max_y);
            let neighbor = btd.cell_texture_set(nx, ny)?;
            quadrants.push(neighbor.quadrants[sf_quadrant].clone());
        }
        return Ok(CellTextureSet { quadrants });
    }
    let mut quadrants: Vec<QuadrantTextureSet> = Vec::with_capacity(4);
    for q in 0..4usize {
        let (qx, qy) = (q & 1, q >> 1);
        let ((dx, dy), (sqx, sqy)) = source_quadrant(qx, qy);
        let nx = (cell_x + dx).clamp(min_x, max_x);
        let ny = (cell_y + dy).clamp(min_y, max_y);
        let neighbor = btd.cell_texture_set(nx, ny)?;
        quadrants.push(neighbor.quadrants[(sqy << 1) | sqx].clone());
    }
    Ok(CellTextureSet { quadrants })
}

/// Gather a 128x128 per-cell array (alphas u16 or gcvr u8) from BTD source data.
/// FO76 identity (`starfield_btd_cell_min: None`): stitched from four neighbors'
/// opposite quadrants. Starfield: an FO4 cell spans ~74.908 BTD samples, so each
/// target sample is a nearest-neighbour pick via `sf_frame`; palette indices and
/// packed 3-bit alphas must never be interpolated. `fetch` must clamp its own
/// cell coordinates to BTD bounds.
pub fn assemble_cell_grid<T: Copy + Default>(
    mut fetch: impl FnMut(i32, i32) -> Result<Vec<T>, BtdError>,
    cell_x: i32,
    cell_y: i32,
    starfield_btd_cell_min: Option<(i32, i32)>,
) -> Result<Vec<T>, BtdError> {
    let Some((btd_cell_min_x, btd_cell_min_y)) = starfield_btd_cell_min else {
        let half = HALF_CELL_SAMPLES;
        let mut out = vec![T::default(); CELL_SAMPLES * CELL_SAMPLES];
        for qy in 0..2usize {
            for qx in 0..2usize {
                let ((dx, dy), (sqx, sqy)) = source_quadrant(qx, qy);
                let src = fetch(cell_x + dx, cell_y + dy)?;
                for ly in 0..half {
                    for lx in 0..half {
                        let sy = sqy * half + ly;
                        let sx = sqx * half + lx;
                        let ty = qy * half + ly;
                        let tx = qx * half + lx;
                        out[ty * CELL_SAMPLES + tx] = src[sy * CELL_SAMPLES + sx];
                    }
                }
            }
        }
        return Ok(out);
    };

    let mut cache: std::collections::HashMap<(i32, i32), Vec<T>> = std::collections::HashMap::new();
    let mut out = vec![T::default(); CELL_SAMPLES * CELL_SAMPLES];
    for ly in 0..CELL_SAMPLES {
        let sample_y = starfield_global_sample_index(cell_y, ly, btd_cell_min_y);
        let sf_cell_y = sample_y.div_euclid(CELL_SAMPLES as i32);
        let local_y = sample_y.rem_euclid(CELL_SAMPLES as i32) as usize;
        for lx in 0..CELL_SAMPLES {
            let sample_x = starfield_global_sample_index(cell_x, lx, btd_cell_min_x);
            let sf_cell_x = sample_x.div_euclid(CELL_SAMPLES as i32);
            let local_x = sample_x.rem_euclid(CELL_SAMPLES as i32) as usize;
            if !cache.contains_key(&(sf_cell_x, sf_cell_y)) {
                let fetched = fetch(sf_cell_x, sf_cell_y)?;
                cache.insert((sf_cell_x, sf_cell_y), fetched);
            }
            let src = &cache[&(sf_cell_x, sf_cell_y)];
            out[ly * CELL_SAMPLES + lx] = src[local_y * CELL_SAMPLES + local_x];
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_quadrant_is_opposite_corner_of_positive_neighbor() {
        assert_eq!(source_quadrant(0, 0), ((0, 0), (1, 1)));
        assert_eq!(source_quadrant(1, 0), ((1, 0), (0, 1)));
        assert_eq!(source_quadrant(0, 1), ((0, 1), (1, 0)));
        assert_eq!(source_quadrant(1, 1), ((1, 1), (0, 0)));
    }

    #[test]
    fn assemble_grid_stitches_opposite_quadrants() {
        // Each synthetic neighbor cell is filled with a marker encoding
        // (cell dx, cell dy, quadrant) so we can assert exact provenance.
        let fetch = |cx: i32, cy: i32| -> Result<Vec<u16>, BtdError> {
            let mut v = vec![0u16; CELL_SAMPLES * CELL_SAMPLES];
            for y in 0..CELL_SAMPLES {
                for x in 0..CELL_SAMPLES {
                    let q = ((y / HALF_CELL_SAMPLES) << 1) | (x / HALF_CELL_SAMPLES);
                    v[y * CELL_SAMPLES + x] = ((cx as u16) << 8) | ((cy as u16) << 4) | q as u16;
                }
            }
            Ok(v)
        };
        let out = assemble_cell_grid(fetch, 0, 0, None).unwrap();
        // FO4 quadrant (0,0) = neighbor (0,0) quadrant (1,1) => marker q=3.
        assert_eq!(out[0], 0x0003);
        // FO4 quadrant (1,0) = neighbor (1,0) quadrant (0,1) => cx=1, q=2.
        assert_eq!(out[HALF_CELL_SAMPLES], 0x0102);
        // FO4 quadrant (0,1) = neighbor (0,1) quadrant (1,0) => cy=1, q=1.
        assert_eq!(out[HALF_CELL_SAMPLES * CELL_SAMPLES], 0x0011);
        // FO4 quadrant (1,1) = neighbor (1,1) quadrant (0,0) => cx=1, cy=1, q=0.
        assert_eq!(
            out[HALF_CELL_SAMPLES * CELL_SAMPLES + HALF_CELL_SAMPLES],
            0x0110
        );
    }
}
