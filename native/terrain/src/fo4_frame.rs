//! FO4-frame assembly over BTD source data.
//!
//! BTD cell (x,y) data covers world [x*4096-2048, x*4096+2048) per axis
//! (half-cell, H1). Under the identity frame (FO4 world == FO76 world), FO4
//! cell (x,y):
//!   - heights: global source sample shifted by +HALF_CELL_SAMPLES per axis
//!   - quadrant data (texture sets, alphas, gcvr): FO4 quadrant (qx,qy) is
//!     BTD cell (x+qx, y+qy) quadrant (1-qx, 1-qy).

use crate::btd::{BtdError, BtdFile, CellTextureSet, QuadrantTextureSet};

pub const HALF_CELL_SAMPLES: usize = 64;
pub const CELL_SAMPLES: usize = 128;

/// BTD neighbor cell delta + source quadrant for an FO4 quadrant (qx, qy in 0..2).
pub fn source_quadrant(qx: usize, qy: usize) -> ((i32, i32), (usize, usize)) {
    ((qx as i32, qy as i32), (1 - qx, 1 - qy))
}

/// Assemble the FO4 cell's texture set from four BTD neighbors. Neighbor
/// coordinates are clamped to BTD bounds so the worldspace edge degrades to
/// the in-range cell rather than erroring.
pub fn assemble_cell_texture_set(
    btd: &BtdFile,
    cell_x: i32,
    cell_y: i32,
) -> Result<CellTextureSet, BtdError> {
    let header = btd.header();
    let (min_x, min_y) = (header.cell_min_x, header.cell_min_y);
    let (max_x, max_y) = (header.cell_max_x, header.cell_max_y);
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

/// Stitch a 128x128 per-cell array (alphas u16 or gcvr u8) from the four BTD
/// neighbors' opposite quadrant blocks. The `fetch` closure is responsible for
/// clamping its own coordinates to BTD bounds (callers pass the clamp).
pub fn assemble_cell_grid<T: Copy + Default>(
    mut fetch: impl FnMut(i32, i32) -> Result<Vec<T>, BtdError>,
    cell_x: i32,
    cell_y: i32,
) -> Result<Vec<T>, BtdError> {
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
        let out = assemble_cell_grid(fetch, 0, 0).unwrap();
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
