//! Fallout 4 LAND texture layers -> raw BTD4 selector patches.
//!
//! FO4 blends a quadrant's base texture and its ATXT overlays sequentially: each
//! overlay of opacity `a` covers `a` and scales everything below by `1 - a`. The raw
//! route walks a quadrant's palette front to back in descending bank id, giving each
//! material `coverage * (1 - covered so far)`, with the last id terminal. Converting
//! the FO4 final weights into those per-step coverages reproduces the FO4 mixture in
//! any palette order, so palettes stay sorted the way neighbouring queues merge.
//!
//! Native grid unit = 32 world units (64 per quadrant), so a quadrant at global
//! quadrant coordinates (gx, gy) has native-grid origin (64 gx, 64 gy) and the
//! runtime derives the grid from world position.

use crate::btd::{RAW_COLOR_SIDE, RAW_SELECTOR_SIDE, RawTerrainPatch};

pub const QUAD_VERTS: usize = 17;
pub const QUAD_VERTEX_COUNT: usize = QUAD_VERTS * QUAD_VERTS;
pub const LATTICE: usize = 64;
/// Palette physical slots the runtime reads, in queue order.
pub const PALETTE_SLOTS: [usize; 6] = [0, 1, 2, 3, 4, 6];
/// RGB555 mid grey: the colour overlay is neutral; FO4 vertex colours stay on the vertex.
pub const NEUTRAL_COLOR: u16 = (16 << 10) | (16 << 5) | 16;

/// One LAND quadrant. Ids are 1-based raw bank ids.
#[derive(Debug, Clone, Default)]
pub struct Fo4Quadrant {
    pub base: Option<u8>,
    /// ATXT overlays in layer order with 17x17 row-major opacities (row 0 = south).
    pub overlays: Vec<(u8, Vec<f32>)>,
}

/// A quadrant's palette and its 64x64 selector words (row-major, row 0 = south).
#[derive(Debug, Clone, PartialEq)]
pub struct QuadrantSelectors {
    pub palette: [u8; 8],
    pub words: Vec<u16>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EncodeStats {
    /// Distinct materials beyond the six palette slots, folded into the kept ones.
    pub dropped_materials: usize,
}

/// FO4 final per-vertex weights, base first, duplicate ids summed.
pub fn final_weights(quadrant: &Fo4Quadrant) -> Option<Vec<(u8, Vec<f32>)>> {
    let base = quadrant.base?;
    let mut layers: Vec<(u8, Vec<f32>)> = vec![(base, vec![1.0; QUAD_VERTEX_COUNT])];
    for (id, opacity) in &quadrant.overlays {
        if opacity.len() != QUAD_VERTEX_COUNT {
            return None;
        }
        for (_, weights) in layers.iter_mut() {
            for (weight, alpha) in weights.iter_mut().zip(opacity) {
                *weight *= 1.0 - alpha.clamp(0.0, 1.0);
            }
        }
        layers.push((*id, opacity.iter().map(|a| a.clamp(0.0, 1.0)).collect()));
    }
    let mut merged: Vec<(u8, Vec<f32>)> = Vec::new();
    for (id, weights) in layers {
        match merged.iter_mut().find(|(existing, _)| *existing == id) {
            Some((_, sum)) => sum.iter_mut().zip(&weights).for_each(|(s, w)| *s += w),
            None => merged.push((id, weights)),
        }
    }
    Some(merged)
}

fn bilinear(grid: &[f32], x: f32, y: f32) -> f32 {
    let x0 = (x.floor() as usize).min(QUAD_VERTS - 1);
    let y0 = (y.floor() as usize).min(QUAD_VERTS - 1);
    let x1 = (x0 + 1).min(QUAD_VERTS - 1);
    let y1 = (y0 + 1).min(QUAD_VERTS - 1);
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |cx: usize, cy: usize| grid[cy * QUAD_VERTS + cx];
    let south = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
    let north = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
    south * (1.0 - fy) + north * fy
}

/// Palette (descending ids, at most six) and selector words for one quadrant.
/// `None` when the quadrant has no base texture: it then keeps native shading.
pub fn encode_quadrant(quadrant: &Fo4Quadrant) -> Option<(QuadrantSelectors, EncodeStats)> {
    let mut layers = final_weights(quadrant)?;
    let mut stats = EncodeStats::default();
    layers.retain(|(_, weights)| weights.iter().any(|w| *w > 0.0));
    if layers.is_empty() {
        return None;
    }
    if layers.len() > PALETTE_SLOTS.len() {
        layers.sort_by(|a, b| b.1.iter().sum::<f32>().total_cmp(&a.1.iter().sum::<f32>()));
        stats.dropped_materials = layers.len() - PALETTE_SLOTS.len();
        layers.truncate(PALETTE_SLOTS.len());
        for vertex in 0..QUAD_VERTEX_COUNT {
            let kept: f32 = layers.iter().map(|(_, w)| w[vertex]).sum();
            if kept > 0.0 {
                layers.iter_mut().for_each(|(_, w)| w[vertex] /= kept);
            } else {
                layers[0].1[vertex] = 1.0;
            }
        }
    }
    layers.sort_by(|a, b| b.0.cmp(&a.0));
    let mut palette = [0u8; 8];
    for (slot, (id, _)) in PALETTE_SLOTS.iter().zip(&layers) {
        palette[*slot] = *id;
    }
    let mut words = vec![0u16; LATTICE * LATTICE];
    for row in 0..LATTICE {
        for column in 0..LATTICE {
            let (x, y) = (column as f32 / 4.0, row as f32 / 4.0);
            let mut remaining = 1.0f32;
            let mut word = 0u16;
            for (step, (_, weights)) in layers.iter().enumerate().take(layers.len().saturating_sub(1).min(5)) {
                let weight = bilinear(weights, x, y).max(0.0);
                let coverage = if remaining > 1e-4 { (weight / remaining).clamp(0.0, 1.0) } else { 0.0 };
                word |= ((coverage * 7.0).round() as u16) << (3 * step);
                remaining = (remaining - weight).max(0.0);
            }
            words[row * LATTICE + column] = word;
        }
    }
    Some((QuadrantSelectors { palette, words }, stats))
}

/// The v4 patch for global quadrant (gx, gy) of cell (gx >> 1, gy >> 1). `lookup`
/// returns a neighbouring quadrant's selectors; selector words and palettes on the far
/// side of a quadrant edge always come from the quadrant that owns them.
pub fn assemble_patch<'a>(gx: i32, gy: i32, lookup: impl Fn(i32, i32) -> Option<&'a QuadrantSelectors>) -> RawTerrainPatch {
    let (cell_x, cell_y) = (gx.div_euclid(2), gy.div_euclid(2));
    let storage_origin = [cell_x * 128 + 64 + gx.rem_euclid(2) * 64, cell_y * 128 + 64 + gy.rem_euclid(2) * 64];
    let mut palettes = [[0u8; 8]; 9];
    for dy in -1..=1 {
        for dx in -1..=1 {
            if let Some(owner) = lookup(gx + dx, gy + dy) {
                palettes[((dy + 1) * 3 + dx + 1) as usize] = owner.palette;
            }
        }
    }
    let lattice = LATTICE as i32;
    let mut selectors = vec![0u16; RAW_SELECTOR_SIDE * RAW_SELECTOR_SIDE];
    for row in 0..RAW_SELECTOR_SIDE as i32 {
        for column in 0..RAW_SELECTOR_SIDE as i32 {
            let (px, py) = (gx * lattice + column - 1, gy * lattice + row - 1);
            if let Some(owner) = lookup(px.div_euclid(lattice), py.div_euclid(lattice)) {
                let local = (py.rem_euclid(lattice) * lattice + px.rem_euclid(lattice)) as usize;
                selectors[row as usize * RAW_SELECTOR_SIDE + column as usize] = owner.words[local];
            }
        }
    }
    RawTerrainPatch {
        storage_origin,
        native_grid_origin: [(gx * lattice) as f32, (gy * lattice) as f32],
        selectors,
        palettes,
        colors: vec![NEUTRAL_COLOR; RAW_COLOR_SIDE * RAW_COLOR_SIDE],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(value: f32) -> Vec<f32> {
        vec![value; QUAD_VERTEX_COUNT]
    }

    /// The runtime's palette queue walk at one lattice point (all four corners equal).
    fn shade(selectors: &QuadrantSelectors, word: u16) -> Vec<(u8, f32)> {
        let ids: Vec<u8> = PALETTE_SLOTS.iter().map(|s| selectors.palette[*s]).take_while(|id| *id != 0).collect();
        let mut bits = u32::from(word) | 0x38000;
        let mut covered = 0.0f32;
        let mut out = Vec::new();
        for (index, id) in ids.iter().enumerate() {
            let selector = (bits & 7) as f32 / 7.0;
            bits >>= 3;
            let terminal = index + 1 == ids.len();
            let coverage = if terminal { 1.0 } else { selector };
            let contribution = coverage * (1.0 - covered);
            out.push((*id, contribution));
            covered = (covered + contribution).min(1.0);
            if covered >= 1.0 { break; }
        }
        out
    }

    #[test]
    fn sequential_overlays_become_fo4_final_weights() {
        let quadrant = Fo4Quadrant { base: Some(3), overlays: vec![(7, flat(0.5)), (5, flat(0.5))] };
        let weights = final_weights(&quadrant).unwrap();
        let at = |id: u8| weights.iter().find(|(i, _)| *i == id).unwrap().1[0];
        assert!((at(3) - 0.25).abs() < 1e-6 && (at(7) - 0.25).abs() < 1e-6 && (at(5) - 0.5).abs() < 1e-6);
        let repeated = Fo4Quadrant { base: Some(3), overlays: vec![(3, flat(0.5))] };
        assert_eq!(final_weights(&repeated).unwrap().len(), 1, "duplicate ids merge");
        assert!(final_weights(&Fo4Quadrant { base: None, overlays: vec![] }).is_none());
    }

    #[test]
    fn descending_queue_reproduces_the_mixture_in_any_layer_order() {
        let quadrant = Fo4Quadrant { base: Some(3), overlays: vec![(7, flat(0.5)), (5, flat(0.5))] };
        let (selectors, stats) = encode_quadrant(&quadrant).unwrap();
        assert_eq!(stats.dropped_materials, 0);
        assert_eq!(selectors.palette, [7, 5, 3, 0, 0, 0, 0, 0]);
        let shaded = shade(&selectors, selectors.words[0]);
        let at = |id: u8| shaded.iter().find(|(i, _)| *i == id).map_or(0.0, |(_, w)| *w);
        // 7: 0.25 -> 2/7; 5: 0.5 of the remaining 0.75 -> 5/7 (quantised).
        assert!((at(7) - 2.0 / 7.0).abs() < 1e-6);
        assert!((at(5) - 5.0 / 7.0 * 5.0 / 7.0).abs() < 1e-6);
        assert!((shaded.iter().map(|(_, w)| w).sum::<f32>() - 1.0).abs() < 1e-6, "the terminal base closes the sum");
    }

    #[test]
    fn opacity_gradients_interpolate_across_the_lattice() {
        let mut ramp = vec![0.0; QUAD_VERTEX_COUNT];
        for row in 0..QUAD_VERTS { for column in 0..QUAD_VERTS { ramp[row * QUAD_VERTS + column] = column as f32 / 16.0; } }
        let (selectors, _) = encode_quadrant(&Fo4Quadrant { base: Some(1), overlays: vec![(2, ramp)] }).unwrap();
        assert_eq!(selectors.palette[..2], [2, 1]);
        assert_eq!(selectors.words[0], 0, "west edge: no overlay");
        assert_eq!(selectors.words[32], 4, "lattice x 32 = vertex 8: 0.5 -> round(3.5)");
        assert_eq!(selectors.words[63], 7, "east lattice point is 63/64 of the way");
    }

    #[test]
    fn a_pure_base_or_missing_base_quadrant() {
        let (selectors, _) = encode_quadrant(&Fo4Quadrant { base: Some(9), overlays: vec![(4, flat(0.0))] }).unwrap();
        assert_eq!(selectors.palette, [9, 0, 0, 0, 0, 0, 0, 0], "zero-coverage overlays leave the palette");
        assert!(selectors.words.iter().all(|w| *w == 0));
        assert!(encode_quadrant(&Fo4Quadrant { base: None, overlays: vec![(4, flat(1.0))] }).is_none());
    }

    #[test]
    fn more_than_six_materials_keep_the_heaviest_six() {
        let overlays = (2..=8).map(|id| (id as u8, flat(1.0 / id as f32))).collect();
        let (selectors, stats) = encode_quadrant(&Fo4Quadrant { base: Some(1), overlays }).unwrap();
        assert_eq!(stats.dropped_materials, 2);
        let ids: Vec<u8> = PALETTE_SLOTS.iter().map(|s| selectors.palette[*s]).collect();
        assert!(ids.windows(2).all(|pair| pair[0] > pair[1]), "palette ids descend: {ids:?}");
        assert_eq!(selectors.palette[5], 0);
        assert_eq!(selectors.palette[7], 0);
    }

    #[test]
    fn patch_takes_far_edge_words_and_palettes_from_their_owner() {
        let own = QuadrantSelectors { palette: [5, 0, 0, 0, 0, 0, 0, 0], words: vec![1; LATTICE * LATTICE] };
        let east = QuadrantSelectors { palette: [6, 0, 0, 0, 0, 0, 0, 0], words: vec![2; LATTICE * LATTICE] };
        let patch = assemble_patch(-1, 3, |x, y| match (x, y) { (-1, 3) => Some(&own), (0, 3) => Some(&east), _ => None });
        assert_eq!(patch.native_grid_origin, [-64.0, 192.0]);
        assert_eq!(patch.storage_origin, [-128 + 64 + 64, 128 + 64 + 64]);
        assert_eq!(patch.palettes[4][0], 5);
        assert_eq!(patch.palettes[5][0], 6);
        assert_eq!(patch.palettes[3], [0; 8], "missing neighbours are empty queues");
        let at = |row: usize, column: usize| patch.selectors[row * RAW_SELECTOR_SIDE + column];
        assert_eq!(at(1, 1), 1, "local (0,0)");
        assert_eq!(at(1, 64), 1, "local (63,0)");
        assert_eq!(at(1, 65), 2, "local (64,0) is the east quadrant's (0,0)");
        assert_eq!(at(1, 0), 0, "west apron has no owner");
        assert!(patch.colors.iter().all(|c| *c == NEUTRAL_COLOR));
    }

    #[test]
    fn assembled_patch_passes_the_cell_contract() {
        let selectors = encode_quadrant(&Fo4Quadrant { base: Some(1), overlays: vec![] }).unwrap().0;
        let bank = crate::raw_terrain::encode_bank(&[crate::raw_terrain::MaterialSource {
            source_ltex_form_id: 0x1234,
            rows: [[256.0, 0.0, 1.0, 1.0], [0.0; 4], [0.0; 4]],
            textures: ["d", "n", "l", "r"].map(|m| format!("textures/landscape/dirt01_source_{m}.dds")),
        }]).unwrap();
        let mut cell = Vec::new();
        for quadrant in 0..4 {
            let patch = assemble_patch(2 * -3 + (quadrant & 1), 2 * 5 + (quadrant >> 1), |_, _| Some(&selectors));
            cell.extend(crate::raw_terrain::encode_patch(&patch).unwrap());
        }
        crate::raw_terrain::validate_cell(&bank, &cell, -3, 5).unwrap();
    }
}
