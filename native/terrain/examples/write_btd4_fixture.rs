// Synthetic BTD4 fixture for the FO4CS dense-terrain reader tests: every channel and a
// four-map raw material bank. Values are invented; nothing here comes from FO76 data.
//
//   cargo run -p terrain_native --example write_btd4_fixture -- <out.btd4>
use std::path::Path;
use terrain_native::btd4::{
    self, ALPH_PLANE_COUNT, ALPH_PLANE_LEN, Btd4Header, Btd4Writer, CellChannels, GcvrChunk,
    GcvrEntry, GrassPointsChunk, GrassPointsGrass, GrassPointsRun, LAYER_COUNT, LayerRef,
};
use terrain_native::{btd, fo76_grass::GrassDnam, raw_terrain};

const GRASS_MASK_LEN: usize = 128 * 128;

/// 12 materials: source ordinal 11 repeats ordinal 0's LTEX id, and no palette names id 11
/// (ordinal 1), so readers must key on the ordinal and accept an unused material.
fn bank() -> Vec<u8> {
    let materials: Vec<_> = (0..12)
        .map(|i| raw_terrain::MaterialSource {
            source_ltex_form_id: 0x1000 + if i == 11 { 0 } else { i },
            rows: [[0.25 + i as f32 / 32.0, 0.0, 1.0, 1.0 / 128.0], [0.0; 4], [i as f32, 0.0, -1.0, 1.0]],
            textures: ["d", "n", "l", "r"].map(|map| format!("textures/terrain/fixture/material{i}_source_{map}.dds")),
        })
        .collect();
    raw_terrain::encode_bank(&materials).unwrap()
}

fn raw_cell(cell_x: i32, cell_y: i32) -> Vec<u8> {
    let mut out = Vec::new();
    for quadrant in 0..4i32 {
        let origin = [cell_x * 128 + 64 + (quadrant & 1) * 64, cell_y * 128 + 64 + (quadrant >> 1) * 64];
        let mut patch = btd::RawTerrainPatch {
            storage_origin: origin,
            native_grid_origin: [origin[0] as f32 - 64.0, origin[1] as f32 - 64.0],
            selectors: Vec::new(),
            palettes: [[0; 8]; 9],
            colors: Vec::new(),
        };
        for row in 0..67i32 {
            for column in 0..67i32 {
                let (x, y) = (origin[0] + column - 1, origin[1] + row - 1);
                patch.selectors.push(0x8000 | ((x.wrapping_mul(31) ^ y.wrapping_mul(17)) as u16 & 0x7fff));
            }
        }
        for row in 0..3i32 {
            for column in 0..3i32 {
                let x = origin[0].div_euclid(64) + column - 1;
                let y = origin[1].div_euclid(64) + row - 1;
                patch.palettes[(row * 3 + column) as usize] =
                    if (x + y).rem_euclid(2) == 0 { [12, 12, 10, 9, 8, 0, 7, 0] } else { [6, 5, 4, 3, 2, 0, 1, 0] };
            }
        }
        for row in 0..19i32 {
            for column in 0..19i32 {
                let x = origin[0].div_euclid(4) + column - 1;
                let y = origin[1].div_euclid(4) + row - 1;
                patch.colors.push(((x.rem_euclid(32) as u16) << 10) | ((y.rem_euclid(32) as u16) << 5) | 16);
            }
        }
        out.extend(raw_terrain::encode_patch(&patch).unwrap());
    }
    out
}

fn grass_points() -> GrassPointsChunk {
    let first = GrassDnam {
        sharp: 60.0,
        freq_base: 0.2,
        height_range: 0.25,
        color_range: 0.3,
        wave_period: 50.0,
        water_above: 0xFFFF,
        water_below: 0xFFFF,
        density: 80,
        min_slope: 0,
        max_slope: 40,
        flags: 0x04,
        cover: 90.0,
        lod_blend: 0.5,
    };
    let second = GrassDnam {
        height_range: 0.5,
        water_above: 64,
        water_below: 32,
        density: 40,
        min_slope: 10,
        max_slope: 90,
        flags: 0x08,
        ..first
    };
    GrassPointsChunk {
        grasses: vec![
            GrassPointsGrass { plugin_index: 0, object_id: 0x000A01, dnam: first },
            GrassPointsGrass { plugin_index: 1, object_id: 0x0B0B02, dnam: second },
        ],
        runs: vec![
            GrassPointsRun { block_x: 0, block_y: 0, param_index: 0, grass: 0, seed_after: 1234, points: vec![[-512, 100], [4096, 4096], [2048, 16]] },
            GrassPointsRun { block_x: 0, block_y: 0, param_index: 5, grass: 1, seed_after: -9, points: vec![[0, 0]] },
            GrassPointsRun { block_x: 9, block_y: 12, param_index: 23, grass: 0, seed_after: i32::MAX, points: vec![[100, 200], [300, 400]] },
        ],
    }
}

fn main() {
    let output = std::env::args().nth(1).expect("output .btd4 path");
    let output = Path::new(&output);
    let bank = bank();
    let mut writer = Btd4Writer::new(
        Btd4Header {
            density: 128,
            height_min: 100.0,
            height_scale: 0.5,
            worldspace_editor_id: "DenseFixture".into(),
            plugin_names: vec!["DenseFixture.esm".into(), "Fallout4.esm".into()],
            cell_min_x: -1,
            cell_min_y: 0,
            cell_max_x: 0,
            cell_max_y: 0,
        },
        bank.clone(),
    )
    .unwrap();

    // Cell (-1,0) carries every optional channel; cell (0,0) only the required HGTS + RAWT.
    let heights = |cell_x: i32| -> Vec<u16> {
        (0..129 * 129).map(|i| (1000 + 3 * (cell_x * 128 + i % 129) + 2 * (i / 129)) as u16).collect()
    };
    let mut alphas = vec![vec![0; ALPH_PLANE_LEN]; ALPH_PLANE_COUNT];
    for quadrant in 0..4 {
        for overlay in 0..7 {
            alphas[quadrant * 7 + overlay][overlay + 1] = 255;
            alphas[quadrant * 7 + overlay][8] = 32;
        }
    }
    let layers = (0..LAYER_COUNT)
        .map(|slot| LayerRef { plugin_index: 0, object_id: 0x800 + (slot % 8) as u32, kind: 0 })
        .collect();
    let mut mask = vec![0u8; GRASS_MASK_LEN];
    mask[..GRASS_MASK_LEN / 2].fill(255);
    let colors: Vec<u8> = (0..129 * 129).flat_map(|i| [(i % 129) as u8, (i / 129) as u8, 128]).collect();
    let full = CellChannels {
        raw_terrain: Some(raw_cell(-1, 0)),
        heights: Some(heights(-1)),
        alphas: Some(alphas),
        layers: Some(layers),
        gcvr: Some(GcvrChunk { entries: vec![GcvrEntry { plugin_index: 0, object_id: 0x000900, mask }] }),
        colors: Some(colors),
        grass_points: Some(grass_points()),
    };
    writer.add_cell(-1, 0, full.clone()).unwrap();
    let minimal = CellChannels {
        raw_terrain: Some(raw_cell(0, 0)),
        heights: Some(heights(0)),
        alphas: None,
        layers: None,
        gcvr: None,
        colors: None,
        grass_points: None,
    };
    writer.add_cell(0, 0, minimal.clone()).unwrap();
    writer.finish(output).unwrap();

    let read = btd4::Btd4Reader::open(output).unwrap();
    assert_eq!(read.raw_material_bank(), bank);
    let cell = read.cell(-1, 0).unwrap();
    assert_eq!((cell.raw_terrain, cell.grass_points, cell.colors), (full.raw_terrain, full.grass_points, full.colors));
    let cell = read.cell(0, 0).unwrap();
    assert_eq!((cell.raw_terrain, cell.alphas, cell.grass_points), (minimal.raw_terrain, None, None));
    let verified = terrain_native::btd4_verify::verify_btd4(output, &Default::default()).unwrap();
    assert_eq!(verified.error_count, 0, "{:?}", verified.errors);
    println!(
        "BTD4 fixture: 12-material d/n/l/r bank; cell (-1,0) with every channel (2 grasses / 3 runs / 6 points), \
         cell (0,0) with HGTS + RAWT only; Rust reader and verifier passed"
    );
}
