//! Starfield `.btd` terrain writer. Layout reference:
//! `bacup/docs/starfield_target/R4-btd-layout.md`. Field names mirror `BtdHeader`
//! in `btd.rs`; when they disagree the reader wins.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use flate2::Compression;
use flate2::write::ZlibEncoder;

const CELL_SAMPLES: usize = 128;
const BLOCK_LEN: usize = 65536;
const ZLIB_ENTRY_LEN: usize = 8;

/// Input to [`write_starfield_btd`]. Grid dimensions are in cells. The grid is
/// always origin-centred (`cell_min_x == -(cells_x >> 1)`; header cell bounds are
/// written as zero), so callers pad arbitrary cell windows. Cells absent from
/// `cells` are written flat (height = the header's minimum, no texture layers).
pub struct BtdWriteInput {
    pub cells_x: u32,
    pub cells_y: u32,
    pub ltex_form_ids: Vec<u32>,
    pub cells: Vec<BtdWriteCell>,
}

/// One cell's terrain data, row-major (`heights[y * 128 + x]`).
pub struct BtdWriteCell {
    pub cell_x: i32,
    pub cell_y: i32,
    /// 128*128 row-major world-Z heights, in the same units as the file's
    /// world_height_min/max (metres).
    pub heights: Vec<f32>,
    /// Per-quadrant LTEX slots: `[0]` = base, `[1..5]` = up to 4 additional
    /// blend layers. Each is an index into `ltex_form_ids`, or `0xFF` for
    /// unused.
    pub quad_layers: [[u8; 6]; 4],
    /// 128*128 row-major packed alpha words, already in on-disk FILE order
    /// (bits `[j*3..j*3+2]` = opacity of the texture named by
    /// `quad_layers[quadrant][j+1]`). Bit 15 is masked off on write.
    pub quad_alphas: Vec<u16>,
}

/// Header values the writer chose, so callers can build the WRLD's SFBK record
/// without re-parsing the file. `cell_min_max` is the BTD's per-cell (min, max)
/// height table (f32 in the file), row-major over the centred grid, rounded and
/// saturated to `i16`.
#[derive(Debug, Clone)]
pub struct BtdWriteReport {
    pub res_x: u32,
    pub res_y: u32,
    pub height_min: f32,
    pub height_max: f32,
    pub cell_min_max: Vec<(i16, i16)>,
}

pub fn write_starfield_btd(
    out_path: &Path,
    input: &BtdWriteInput,
) -> Result<BtdWriteReport, String> {
    let ltex_count = input.ltex_form_ids.len();
    if ltex_count > 255 {
        return Err(format!(
            "ltex_form_ids has {ltex_count} entries; a Starfield quadrant record \
             encodes `ltex_count - index` in a u8, so at most 255 land textures \
             are representable (R4-btd-layout.md RISK-3). Reduce the palette \
             before calling write_starfield_btd."
        ));
    }
    if input.cells_x == 0 || input.cells_y == 0 {
        return Err("cells_x and cells_y must both be nonzero".to_owned());
    }

    let cells_x = input.cells_x as i32;
    let cells_y = input.cells_y as i32;
    // Centred-grid invariant; holds for every vanilla file.
    let cell_min_x = -(cells_x >> 1);
    let cell_min_y = -(cells_y >> 1);
    let cell_max_x = cell_min_x + cells_x - 1;
    let cell_max_y = cell_min_y + cells_y - 1;
    let cell_count = (input.cells_x as usize) * (input.cells_y as usize);

    let mut cell_map: HashMap<(i32, i32), &BtdWriteCell> =
        HashMap::with_capacity(input.cells.len());
    for cell in &input.cells {
        if cell.heights.len() != CELL_SAMPLES * CELL_SAMPLES {
            return Err(format!(
                "cell ({}, {}) heights.len() = {}, expected {}",
                cell.cell_x,
                cell.cell_y,
                cell.heights.len(),
                CELL_SAMPLES * CELL_SAMPLES
            ));
        }
        if cell.quad_alphas.len() != CELL_SAMPLES * CELL_SAMPLES {
            return Err(format!(
                "cell ({}, {}) quad_alphas.len() = {}, expected {}",
                cell.cell_x,
                cell.cell_y,
                cell.quad_alphas.len(),
                CELL_SAMPLES * CELL_SAMPLES
            ));
        }
        if cell.cell_x < cell_min_x
            || cell.cell_x > cell_max_x
            || cell.cell_y < cell_min_y
            || cell.cell_y > cell_max_y
        {
            return Err(format!(
                "cell ({}, {}) is outside the centred {}x{} grid [{cell_min_x}..{cell_max_x}]x[{cell_min_y}..{cell_max_y}]",
                cell.cell_x, cell.cell_y, input.cells_x, input.cells_y
            ));
        }
        for quad in &cell.quad_layers {
            for &slot in quad {
                if slot != 0xFF && slot as usize >= ltex_count {
                    return Err(format!(
                        "cell ({}, {}) references ltex index {slot} but only {ltex_count} textures are registered",
                        cell.cell_x, cell.cell_y
                    ));
                }
            }
        }
        if cell_map.insert((cell.cell_x, cell.cell_y), cell).is_some() {
            return Err(format!(
                "cell ({}, {}) supplied more than once",
                cell.cell_x, cell.cell_y
            ));
        }
    }

    // Header range is the worldspace's Z extent, widened for a flat or empty world.
    let (mut hdr_min, mut hdr_max) = (f32::MAX, f32::MIN);
    for cell in &input.cells {
        for &z in &cell.heights {
            hdr_min = hdr_min.min(z);
            hdr_max = hdr_max.max(z);
        }
    }
    if input.cells.is_empty() {
        hdr_min = 0.0;
        hdr_max = 1.0;
    } else if hdr_max - hdr_min < 1e-3 {
        hdr_max = hdr_min + 1.0;
    }

    let file = File::create(out_path).map_err(|e| format!("create {}: {e}", out_path.display()))?;
    let mut w = BufWriter::new(file);

    let resolution_x = input.cells_x * 128;
    let resolution_y = input.cells_y * 128;

    // --- header ---
    write_all(&mut w, b"BTDB")?;
    write_all(&mut w, &6u32.to_le_bytes())?;
    write_all(&mut w, &hdr_min.to_le_bytes())?;
    write_all(&mut w, &hdr_max.to_le_bytes())?;
    write_all(&mut w, &resolution_x.to_le_bytes())?;
    write_all(&mut w, &resolution_y.to_le_bytes())?;
    write_all(&mut w, &0i32.to_le_bytes())?; // cell_min_x sentinel
    write_all(&mut w, &0i32.to_le_bytes())?; // cell_min_y sentinel
    write_all(&mut w, &0i32.to_le_bytes())?; // cell_max_x sentinel
    write_all(&mut w, &0i32.to_le_bytes())?; // cell_max_y sentinel
    write_all(&mut w, &(ltex_count as u32).to_le_bytes())?;

    // --- ltex_form_ids ---
    for &id in &input.ltex_form_ids {
        write_all(&mut w, &id.to_le_bytes())?;
    }

    // --- cell_height_minmax ---
    // Must equal the dequantized min/max of that cell's own LOD0 heights
    // (holds 377/377 on vanilla data), not the raw f32 extent.
    let mut cell_min_max = Vec::with_capacity(cell_count);
    for cy in cell_min_y..=cell_max_y {
        for cx in cell_min_x..=cell_max_x {
            let (min_f, max_f) = match cell_map.get(&(cx, cy)) {
                Some(cell) => {
                    let mut min_q = u16::MAX;
                    let mut max_q = 0u16;
                    for &z in &cell.heights {
                        let q = quantize_height(z, hdr_min, hdr_max);
                        min_q = min_q.min(q);
                        max_q = max_q.max(q);
                    }
                    (
                        dequantize_height(min_q, hdr_min, hdr_max),
                        dequantize_height(max_q, hdr_min, hdr_max),
                    )
                }
                // Default-flat cell: every sample quantizes to 0 (hdr_min).
                None => (hdr_min, hdr_min),
            };
            write_all(&mut w, &min_f.to_le_bytes())?;
            write_all(&mut w, &max_f.to_le_bytes())?;
            cell_min_max.push((saturate_i16(min_f), saturate_i16(max_f)));
        }
    }

    // --- ltex_map ---
    for cy in cell_min_y..=cell_max_y {
        for cx in cell_min_x..=cell_max_x {
            let quad_layers = cell_map
                .get(&(cx, cy))
                .map(|c| c.quad_layers)
                .unwrap_or([[0xFFu8; 6]; 4]);
            for quad in &quad_layers {
                write_all(&mut w, &encode_quadrant_record(quad, ltex_count))?;
            }
        }
    }

    // Starfield BTD has no GCVR section; even an empty one would shift every
    // downstream offset.

    // --- LOD4 height and alpha rasters ---
    let lod4_width = (input.cells_x as usize) * 8;
    let mut height_lod4 = vec![0u16; cell_count * 64];
    let mut alpha_lod4 = vec![0u16; cell_count * 64];
    for (cell_idx_y, cy) in (cell_min_y..=cell_max_y).enumerate() {
        for (cell_idx_x, cx) in (cell_min_x..=cell_max_x).enumerate() {
            let cell = cell_map.get(&(cx, cy));
            for j in 0..8usize {
                for i in 0..8usize {
                    let src = (j * 16) * CELL_SAMPLES + (i * 16);
                    let (h, a) = match cell {
                        Some(cell) => (
                            quantize_height(cell.heights[src], hdr_min, hdr_max),
                            cell.quad_alphas[src] & 0x7FFF,
                        ),
                        None => (0u16, 0u16),
                    };
                    let dest = (cell_idx_y * 8 + j) * lod4_width + (cell_idx_x * 8 + i);
                    height_lod4[dest] = h;
                    alpha_lod4[dest] = a;
                }
            }
        }
    }
    for &v in &height_lod4 {
        write_all(&mut w, &v.to_le_bytes())?;
    }
    for &v in &alpha_lod4 {
        write_all(&mut w, &v.to_le_bytes())?;
    }

    // --- LOD3..LOD0 zlib tables, reserved (backpatched after the data below) ---
    let lod3_cols = (input.cells_x as usize).div_ceil(8);
    let lod3_rows = (input.cells_y as usize).div_ceil(8);
    let lod2_cols = (input.cells_x as usize).div_ceil(4);
    let lod2_rows = (input.cells_y as usize).div_ceil(4);
    let lod1_cols = (input.cells_x as usize).div_ceil(2);
    let lod1_rows = (input.cells_y as usize).div_ceil(2);
    let lod0_cols = input.cells_x as usize;
    let lod0_rows = input.cells_y as usize;

    let table_lod3_pos = stream_pos(&mut w)?;
    write_zeros(&mut w, lod3_cols * lod3_rows * ZLIB_ENTRY_LEN)?;
    let table_lod2_pos = stream_pos(&mut w)?;
    write_zeros(&mut w, lod2_cols * lod2_rows * ZLIB_ENTRY_LEN)?;
    let table_lod1_pos = stream_pos(&mut w)?;
    write_zeros(&mut w, lod1_cols * lod1_rows * ZLIB_ENTRY_LEN)?;
    let table_lod0_pos = stream_pos(&mut w)?;
    write_zeros(&mut w, lod0_cols * lod0_rows * ZLIB_ENTRY_LEN)?;

    // --- zlib_data: streamed, offsets are relative to here ---
    let zlib_data_start = stream_pos(&mut w)?;

    let mut table_lod3 = Vec::with_capacity(lod3_cols * lod3_rows);
    let mut table_lod2 = Vec::with_capacity(lod2_cols * lod2_rows);
    let mut table_lod1 = Vec::with_capacity(lod1_cols * lod1_rows);
    let mut table_lod0 = Vec::with_capacity(lod0_cols * lod0_rows);

    // Data region order: LOD3, LOD2, LOD1, LOD0, contiguous, no gaps.
    for gy in 0..lod3_rows {
        for gx in 0..lod3_cols {
            let block = build_block(
                &cell_map, cell_min_x, cell_min_y, cell_max_x, cell_max_y, gx as i32, gy as i32, 3,
                hdr_min, hdr_max,
            );
            table_lod3.push(write_compressed_block(&mut w, zlib_data_start, &block)?);
        }
    }
    for gy in 0..lod2_rows {
        for gx in 0..lod2_cols {
            let block = build_block(
                &cell_map, cell_min_x, cell_min_y, cell_max_x, cell_max_y, gx as i32, gy as i32, 2,
                hdr_min, hdr_max,
            );
            table_lod2.push(write_compressed_block(&mut w, zlib_data_start, &block)?);
        }
    }
    for gy in 0..lod1_rows {
        for gx in 0..lod1_cols {
            let block = build_block(
                &cell_map, cell_min_x, cell_min_y, cell_max_x, cell_max_y, gx as i32, gy as i32, 1,
                hdr_min, hdr_max,
            );
            table_lod1.push(write_compressed_block(&mut w, zlib_data_start, &block)?);
        }
    }
    for gy in 0..lod0_rows {
        for gx in 0..lod0_cols {
            let block = build_block(
                &cell_map, cell_min_x, cell_min_y, cell_max_x, cell_max_y, gx as i32, gy as i32, 0,
                hdr_min, hdr_max,
            );
            table_lod0.push(write_compressed_block(&mut w, zlib_data_start, &block)?);
        }
    }

    // --- backpatch the four tables now that offsets/sizes are known ---
    backpatch_table(&mut w, table_lod3_pos, &table_lod3)?;
    backpatch_table(&mut w, table_lod2_pos, &table_lod2)?;
    backpatch_table(&mut w, table_lod1_pos, &table_lod1)?;
    backpatch_table(&mut w, table_lod0_pos, &table_lod0)?;

    w.flush().map_err(io_err)?;

    Ok(BtdWriteReport {
        res_x: resolution_x,
        res_y: resolution_y,
        height_min: hdr_min,
        height_max: hdr_max,
        cell_min_max,
    })
}

fn quantize_height(z: f32, hdr_min: f32, hdr_max: f32) -> u16 {
    let scaled = (z - hdr_min) * 65535.0 / (hdr_max - hdr_min);
    scaled.round().clamp(0.0, 65535.0) as u16
}

fn dequantize_height(q: u16, hdr_min: f32, hdr_max: f32) -> f32 {
    hdr_min + f32::from(q) * (hdr_max - hdr_min) / 65535.0
}

fn saturate_i16(v: f32) -> i16 {
    v.round().clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
}

/// `raw[j] = ltex_count - quad_layers[j+1]` for the five blend slots and
/// `raw[6] = ltex_count - quad_layers[0]` for the base (`0` if unused); bytes 5
/// and 7 are always `0`. The base goes in byte 6 because the reader always
/// visits that index last, so the round trip holds.
fn encode_quadrant_record(quad: &[u8; 6], ltex_count: usize) -> [u8; 8] {
    let mut raw = [0u8; 8];
    for j in 0..5usize {
        let slot = quad[j + 1];
        raw[j] = if slot == 0xFF {
            0
        } else {
            (ltex_count - slot as usize) as u8
        };
    }
    let base = quad[0];
    raw[6] = if base == 0xFF {
        0
    } else {
        (ltex_count - base as usize) as u8
    };
    raw
}

/// Builds one 65536-byte block (heights then alphas, each row-major 128x128) for
/// the LOD-`lod` group at `(gx, gy)`: the stride-`2^l` point decimation (no
/// filtering) of the `2^l x 2^l` cell region starting at cell `cell_min + g*2^l`.
/// Cells past the grid edge clamp to the nearest in-range cell.
#[allow(clippy::too_many_arguments)]
fn build_block(
    cell_map: &HashMap<(i32, i32), &BtdWriteCell>,
    cell_min_x: i32,
    cell_min_y: i32,
    cell_max_x: i32,
    cell_max_y: i32,
    gx: i32,
    gy: i32,
    lod: u8,
    hdr_min: f32,
    hdr_max: f32,
) -> [u8; BLOCK_LEN] {
    let mut buf = [0u8; BLOCK_LEN];
    let stride = 1i64 << lod;
    for j in 0..CELL_SAMPLES {
        let abs_row = i64::from(gy) * stride * 128 + (j as i64) * stride;
        let cy = i64::from(cell_min_y) + abs_row.div_euclid(128);
        let ly = abs_row.rem_euclid(128) as usize;
        let ccy = cy.clamp(i64::from(cell_min_y), i64::from(cell_max_y)) as i32;
        for i in 0..CELL_SAMPLES {
            let abs_col = i64::from(gx) * stride * 128 + (i as i64) * stride;
            let cx = i64::from(cell_min_x) + abs_col.div_euclid(128);
            let lx = abs_col.rem_euclid(128) as usize;
            let ccx = cx.clamp(i64::from(cell_min_x), i64::from(cell_max_x)) as i32;

            let src = ly * CELL_SAMPLES + lx;
            let (h, a) = match cell_map.get(&(ccx, ccy)) {
                Some(cell) => (
                    quantize_height(cell.heights[src], hdr_min, hdr_max),
                    cell.quad_alphas[src] & 0x7FFF,
                ),
                None => (0u16, 0u16),
            };
            let hi = (j * CELL_SAMPLES + i) * 2;
            buf[hi..hi + 2].copy_from_slice(&h.to_le_bytes());
            let ai = 0x8000 + (j * CELL_SAMPLES + i) * 2;
            buf[ai..ai + 2].copy_from_slice(&a.to_le_bytes());
        }
    }
    buf
}

fn write_compressed_block<W: Write + Seek>(
    w: &mut W,
    zlib_data_start: u64,
    block: &[u8; BLOCK_LEN],
) -> Result<(u32, u32), String> {
    let pos_before = stream_pos(w)?;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(block).map_err(io_err)?;
    let compressed = encoder.finish().map_err(io_err)?;
    write_all(w, &compressed)?;
    let offset = u32::try_from(pos_before - zlib_data_start)
        .map_err(|_| "compressed block offset exceeds u32 range".to_owned())?;
    let size = u32::try_from(compressed.len())
        .map_err(|_| "compressed block size exceeds u32 range".to_owned())?;
    Ok((offset, size))
}

fn backpatch_table<W: Write + Seek>(
    w: &mut W,
    table_pos: u64,
    entries: &[(u32, u32)],
) -> Result<(), String> {
    w.seek(SeekFrom::Start(table_pos)).map_err(io_err)?;
    for &(offset, size) in entries {
        write_all(w, &offset.to_le_bytes())?;
        write_all(w, &size.to_le_bytes())?;
    }
    Ok(())
}

fn stream_pos<W: Seek>(w: &mut W) -> Result<u64, String> {
    w.stream_position().map_err(io_err)
}

fn write_zeros<W: Write>(w: &mut W, len: usize) -> Result<(), String> {
    const CHUNK: [u8; 4096] = [0u8; 4096];
    let mut remaining = len;
    while remaining > 0 {
        let n = remaining.min(CHUNK.len());
        write_all(w, &CHUNK[..n])?;
        remaining -= n;
    }
    Ok(())
}

fn write_all<W: Write>(w: &mut W, bytes: &[u8]) -> Result<(), String> {
    w.write_all(bytes).map_err(io_err)
}

fn io_err(e: std::io::Error) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_input_2x2() -> BtdWriteInput {
        let ltex_form_ids = vec![0x0001_1234u32, 0x0001_5678u32];
        let mut cells = Vec::new();
        for cell_y in -1..=0i32 {
            for cell_x in -1..=0i32 {
                let mut heights = vec![0f32; CELL_SAMPLES * CELL_SAMPLES];
                let mut quad_alphas = vec![0u16; CELL_SAMPLES * CELL_SAMPLES];
                for j in 0..CELL_SAMPLES {
                    for i in 0..CELL_SAMPLES {
                        let idx = j * CELL_SAMPLES + i;
                        // A smooth ramp spanning all four cells so hdr_min/max
                        // are nontrivial and vary sample-to-sample.
                        heights[idx] = 10.0 * (cell_x as f32 + 1.0)
                            + 20.0 * (cell_y as f32 + 1.0)
                            + (i as f32) * 0.1
                            + (j as f32) * 0.05;
                        // File bit-group 0 (bits 0..2) carries a gradient;
                        // groups 1..4 stay zero.
                        quad_alphas[idx] = ((i + j) % 8) as u16;
                    }
                }
                let mut quad_layers = [[0xFFu8; 6]; 4];
                for quad in &mut quad_layers {
                    quad[0] = 0; // base -> ltex_form_ids[0]
                    quad[1] = 1; // one additional layer -> ltex_form_ids[1]
                }
                cells.push(BtdWriteCell {
                    cell_x,
                    cell_y,
                    heights,
                    quad_layers,
                    quad_alphas,
                });
            }
        }
        BtdWriteInput {
            cells_x: 2,
            cells_y: 2,
            ltex_form_ids,
            cells,
        }
    }

    #[test]
    fn roundtrip_two_by_two_cells_through_reader() {
        let input = synthetic_input_2x2();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("TESTWORLD.btd");

        let report = write_starfield_btd(&path, &input).expect("write starfield btd");
        let path_str = path.to_str().unwrap();

        let header = crate::btd::BtdFile::open_header(path_str).expect("decode header");
        assert!(header.is_starfield_layout);
        assert_eq!(header.version, 6);
        assert_eq!(header.cells_x, 2);
        assert_eq!(header.cells_y, 2);
        assert_eq!(header.cell_min_x, -1);
        assert_eq!(header.cell_max_x, 0);
        assert_eq!(header.cell_min_y, -1);
        assert_eq!(header.cell_max_y, 0);
        assert_eq!(header.ltex_count, 2);
        assert_eq!(header.gcvr_count, 0);

        // BtdWriteReport must match what the reader derives from the file, since
        // SFBK synthesis uses the report instead of re-parsing it.
        assert_eq!(report.res_x, header.resolution_x);
        assert_eq!(report.res_y, header.resolution_y);
        assert!((report.height_min - header.world_height_min).abs() < 1e-6);
        assert!((report.height_max - header.world_height_max).abs() < 1e-6);
        assert_eq!(
            report.cell_min_max.len(),
            (input.cells_x * input.cells_y) as usize
        );

        let mut btd = crate::btd::BtdFile::open(path_str).expect("open starfield btd");
        let step = (report.height_max - report.height_min) / 65535.0;

        for cell in &input.cells {
            let heights_u16 = btd
                .cell_height_map_u16(cell.cell_x, cell.cell_y, 0)
                .expect("decode heights");
            assert_eq!(heights_u16.len(), CELL_SAMPLES * CELL_SAMPLES);
            for (i, &true_z) in cell.heights.iter().enumerate() {
                let decoded_z =
                    dequantize_height(heights_u16[i], report.height_min, report.height_max);
                assert!(
                    (decoded_z - true_z).abs() <= step + 1e-4,
                    "cell ({}, {}) sample {i}: true={true_z} decoded={decoded_z} step={step}",
                    cell.cell_x,
                    cell.cell_y
                );
            }

            let alphas_u16 = btd
                .cell_land_alpha_u16(cell.cell_x, cell.cell_y, 0)
                .expect("decode alphas");
            for (i, &packed_file_order) in cell.quad_alphas.iter().enumerate() {
                let expected_file_group0 = packed_file_order & 0x7;
                // reorder_land_alpha_bits reverses the five 3-bit groups (an
                // involution over all 65536 inputs): file group 0 lands in
                // post-reorder group 4.
                let decoded_group4 = (alphas_u16[i] >> 12) & 0x7;
                assert_eq!(
                    decoded_group4, expected_file_group0,
                    "cell ({}, {}) sample {i}",
                    cell.cell_x, cell.cell_y
                );
            }

            let texset = btd
                .cell_texture_set(cell.cell_x, cell.cell_y)
                .expect("decode texture set");
            for (q, quadrant) in texset.quadrants.iter().enumerate() {
                assert_eq!(quadrant.base, Some(cell.quad_layers[q][0]));
                assert_eq!(quadrant.additional[4], Some(cell.quad_layers[q][1]));
            }
        }
    }

    #[test]
    fn rejects_too_many_ltex_form_ids() {
        let input = BtdWriteInput {
            cells_x: 1,
            cells_y: 1,
            ltex_form_ids: vec![0u32; 256],
            cells: Vec::new(),
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("TOOMANY.btd");
        let err = write_starfield_btd(&path, &input).expect_err("must reject > 255 ltex ids");
        assert!(err.contains("255"), "unexpected error message: {err}");
    }
}
