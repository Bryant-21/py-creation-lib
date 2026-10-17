//! Offline port of FO76's BTD ground-cover placement (`AddCellGrass` → per-block
//! `GetGroundCoverParameters` → `CreateGrassPoissonDiskSample`), as reverse engineered in
//! `mods/B21_TalesFromAppalachia/docs/re/FO76_GROUND_COVER.md` §3–§4.
//!
//! Everything here is terrain-height independent, so the converter runs it once and the
//! runtime only applies §5 (height, water, slope, rotation/scale draws) to the stored points.
//! Arithmetic is `f32` throughout because the game computes in single precision.
//!
//! Frame: FO76 grass cells are world cells. Sample `lx` of cell `X` sits at world
//! `X*4096 + lx*32`; the BTD storage cell is offset by half a cell (see `fo4_frame`), so
//! the inputs here are the same FO4-frame assembled grids the GCVR masks use, and the seeds
//! use the world (= FO4) cell coordinates and cell-local sample offsets.

use std::sync::OnceLock;

pub const CELL_SAMPLES: usize = 128;
pub const BLOCK_SAMPLES: usize = 8;
pub const BLOCKS_PER_AXIS: usize = CELL_SAMPLES / BLOCK_SAMPLES;
pub const SAMPLE_UNITS: f32 = 32.0;
pub const CELL_UNITS: f32 = 4096.0;
pub const MAX_PARAMS_PER_BLOCK: usize = 24;
pub const MIN_GRASS_DISTANCE: f32 = 25.0;
pub const MAX_GRASS_DISTANCE: f32 = 175.0;
pub const GCVR_DEFAULT_YNAM: f32 = 0.286;
/// GCVR per-grass DNAM value meaning "use the GRAS density byte".
pub const GCVR_DENSITY_FROM_GRASS: u16 = 0xFFFF;
const TRIES_PER_POINT: usize = 16;
const RANDOM_TABLE_LEN: usize = 2048;
/// The texture list reads quadrant bytes 0,1,2,3,4,6 as layers 0..5; byte 5 is skipped.
const LAYER_BYTES: [usize; 6] = [0, 1, 2, 3, 4, 6];

/// FO76 GRAS DNAM values in file order (36 bytes; spec §2.1). Raw, untransformed.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GrassDnam {
    /// File f0 (`+236`): clump mask sharpness, used as `min(x*0.01, 0.9999)`.
    pub sharp: f32,
    /// File f4 (`+216`): noise feature size; frequency `0.001/max(x, 0.01)`.
    pub freq_base: f32,
    pub height_range: f32,
    pub color_range: f32,
    pub wave_period: f32,
    /// Water distance above / below (`0xFFFF` = none).
    pub water_above: u16,
    pub water_below: u16,
    pub density: u8,
    pub min_slope: u8,
    pub max_slope: u8,
    pub flags: u8,
    /// File f28 (`+240`, form version >= 137): clump coverage, used as `x*0.01`.
    pub cover: f32,
    /// File f32 (`+228`, form version >= 175): LOD land blend.
    pub lod_blend: f32,
}

impl GrassDnam {
    /// Same defaults as `TESGrass::InitializeData`, then the file fields that are present.
    pub fn from_file_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 28 {
            return None;
        }
        let f32_at = |offset: usize| f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let u16_at = |offset: usize| u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        Some(Self {
            sharp: f32_at(0),
            freq_base: f32_at(4),
            height_range: f32_at(8),
            color_range: f32_at(12),
            wave_period: f32_at(16),
            water_above: u16_at(20),
            water_below: u16_at(22),
            density: bytes[24],
            min_slope: bytes[25],
            max_slope: bytes[26],
            flags: bytes[27],
            cover: if bytes.len() >= 32 { f32_at(28) } else { 50.0 },
            lod_blend: if bytes.len() >= 36 { f32_at(32) } else { 1.0 },
        })
    }
}

/// The values `CreateGrassPoissonDiskSample` derives from one GRAS.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClumpMask {
    pub frequency: f32,
    pub sharp: f32,
    pub cover: f32,
}

impl ClumpMask {
    pub fn from_dnam(dnam: &GrassDnam) -> Self {
        // The DNAM reader replaces a near-zero noise size with 1.0.
        let freq_base = if dnam.freq_base.abs() <= 1e-4 { 1.0 } else { dnam.freq_base };
        Self {
            frequency: 0.001 / freq_base.max(0.01),
            sharp: (dnam.sharp * 0.01).min(0.9999),
            cover: dnam.cover * 0.01,
        }
    }

    /// `0xd6b620`: 0 where grass may grow at full packing, >= 1 where it is rejected.
    pub fn value(&self, x: f32, y: f32) -> f32 {
        let (lo, hi) = self.thresholds();
        let n01 = (perlin2(x.abs() * self.frequency, y.abs() * self.frequency) + 1.0) * 0.5;
        1.0 - ((n01 - lo) / (hi - lo)).clamp(0.0, 1.0)
    }

    fn thresholds(&self) -> (f32, f32) {
        let c = 1.0 - self.cover;
        let v7 = c.max(0.5);
        let t = (self.sharp * 0.5).clamp(0.0, 1.0);
        let v9 = (2.0 * c).min(1.0);
        let v10 = 2.0 * v7 - 1.0;
        (v10 + (v9 - v10) * t, v9 + (v10 - v9) * t)
    }
}

/// `BSRandom::Init` table: MT19937 `init_genrand(1)`, then
/// `uniform_real_distribution<float>(0, 0.99999)` built on `generate_canonical<float, 24>`.
pub fn random_table() -> &'static [f32; RANDOM_TABLE_LEN] {
    static TABLE: OnceLock<[f32; RANDOM_TABLE_LEN]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut mt = Mt19937::new(1);
        let range = 0.99999f32 - 0.0;
        std::array::from_fn(|_| (mt.next_u32() as f32 / 4_294_967_296.0f32) * range + 0.0)
    })
}

/// `BSRandom` float-by-seed: `tbl[seed & 0x7FF]`.
pub fn random_by_seed(seed: i32) -> f32 {
    random_table()[(seed as u32 & (RANDOM_TABLE_LEN as u32 - 1)) as usize]
}

fn next_random(seed: &mut i32) -> f32 {
    let value = random_by_seed(*seed);
    *seed = seed.wrapping_add(1);
    value
}

struct Mt19937 {
    state: [u32; 624],
    index: usize,
}

impl Mt19937 {
    fn new(seed: u32) -> Self {
        let mut state = [0u32; 624];
        state[0] = seed;
        for i in 1..624 {
            state[i] = 1_812_433_253u32
                .wrapping_mul(state[i - 1] ^ (state[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        Self { state, index: 624 }
    }

    fn next_u32(&mut self) -> u32 {
        if self.index >= 624 {
            for i in 0..624 {
                let y = (self.state[i] & 0x8000_0000) | (self.state[(i + 1) % 624] & 0x7FFF_FFFF);
                let mut next = self.state[(i + 397) % 624] ^ (y >> 1);
                if y & 1 != 0 {
                    next ^= 0x9908_B0DF;
                }
                self.state[i] = next;
            }
            self.index = 0;
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9D2C_5680;
        y ^= (y << 15) & 0xEFC6_0000;
        y ^ (y >> 18)
    }
}

const PERMUTATION: [u8; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69,
    142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219,
    203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
    74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230,
    220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76,
    132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173,
    186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206,
    59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163,
    70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232,
    178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162,
    241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204,
    176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141,
    128, 195, 78, 66, 215, 61, 156, 180,
];

fn perm(index: usize) -> usize {
    usize::from(PERMUTATION[index & 255])
}

fn fade(t: f32) -> f32 {
    ((6.0 * t - 15.0) * t + 10.0) * t * t * t
}

fn lerp(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

fn grad(hash: usize, x: f32, y: f32) -> f32 {
    match hash & 7 {
        0 => x + y,
        1 => x - y,
        2 => y - x,
        3 => -x - y,
        4 => x,
        5 => -x,
        6 => y,
        _ => -y,
    }
}

/// `0x4138ec0`: Perlin's improved noise in 2D. Inputs are non-negative here (the mask
/// passes absolute values), so `(int)` truncation equals floor.
pub fn perlin2(x: f32, y: f32) -> f32 {
    let ix = x as i32;
    let iy = y as i32;
    let fx = x - ix as f32;
    let fy = y - iy as f32;
    let xi = (ix & 255) as usize;
    let yi = (iy & 255) as usize;
    let u = fade(fx);
    let v = fade(fy);
    let a = perm(xi);
    let b = perm(xi + 1);
    let x0 = lerp(u, grad(perm(a + yi), fx, fy), grad(perm(b + yi), fx - 1.0, fy));
    let x1 = lerp(
        u,
        grad(perm(a + yi + 1), fx, fy - 1.0),
        grad(perm(b + yi + 1), fx - 1.0, fy - 1.0),
    );
    lerp(v, x0, x1)
}

/// One GCVR as `BGSGroundCover` holds it after load.
#[derive(Debug, Clone, PartialEq)]
pub struct GroundCover {
    /// LNAM source LTEX object ids (24-bit).
    pub land_textures: Vec<u32>,
    pub ynam: f32,
    /// (grass key, GCVR DNAM u16) in GNAM file order; the key indexes the caller's grass table.
    pub grasses: Vec<(usize, u16)>,
}

/// The BTD inputs of one world cell, in the FO4/world frame (row-major, 128x128).
pub struct CellSource<'a> {
    pub cell_x: i32,
    pub cell_y: i32,
    /// Ground-cover byte per sample, FO76 raw bit order: bit k = GCVR slot k.
    pub ground_cover: &'a [u8],
    /// LOD0 LTEX alpha word per sample (raw: 3-bit group i belongs to texture byte i).
    pub alpha_words: &'a [u16],
    /// Per quadrant (index `qx | qy << 1`): texture layers 0..5 as source LTEX ids, already
    /// cut at the first empty byte, and the eight GCVR slots.
    pub quadrant_layers: [Vec<u32>; 4],
    pub quadrant_ground_covers: [[Option<&'a GroundCover>; 8]; 4],
}

/// One emitted param of one block: its Poisson points, in world units, in list order.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamRun {
    pub block_x: u8,
    pub block_y: u8,
    pub param_index: u8,
    pub grass: usize,
    /// Seed after sampling; the runtime draws rotation and scale for kept points from it.
    pub seed_after: i32,
    pub points: Vec<[f32; 2]>,
}

/// Bytes 0,1,2,3,4,6 of a quadrant's 8-byte LTEX list as LTEX table indices, stopping at
/// the first 0 byte (`ltexIndex = ltexCount - byte`). A byte above the count also ends it.
pub fn quadrant_layer_indices(bytes: &[u8; 8], ltex_count: usize) -> Vec<usize> {
    LAYER_BYTES
        .iter()
        .map(|&index| usize::from(bytes[index]))
        .take_while(|&byte| byte != 0 && byte <= ltex_count)
        .map(|byte| ltex_count - byte)
        .collect()
}

/// `0xfb9fb0` + `0xfc0990`: visible fraction of each listed layer, composited front to back.
fn layer_weights(word: u16, layer_count: usize) -> [f32; 6] {
    let mut weights = [0.0f32; 6];
    let mut remaining = 1.0f32;
    for (i, weight) in weights.iter_mut().enumerate().take(layer_count) {
        if i + 1 == layer_count {
            *weight = remaining;
            break;
        }
        let alpha = if i < 5 { f32::from((word >> (3 * i)) & 7) / 7.0 } else { 1.0 };
        *weight = remaining * alpha;
        remaining -= *weight;
    }
    weights
}

/// `GetGroundCoverParameters` (`0xfc2d90` + `0xfc4cc0`) for the block whose first sample is
/// (`sx`, `sy`): up to 24 (grass key, 64 densities), slots 0..7 then GNAM order.
pub fn block_params(source: &CellSource, sx: usize, sy: usize, grass_density: &dyn Fn(usize) -> u8) -> Vec<(usize, [f32; 64])> {
    let quadrant = (sx / 64) | ((sy / 64) << 1);
    let layers = &source.quadrant_layers[quadrant];
    let covers = &source.quadrant_ground_covers[quadrant];
    let mut texture_weight = [[0.0f32; 8]; 64];
    for (s, slot_weights) in texture_weight.iter_mut().enumerate() {
        let index = (sy + s / 8) * CELL_SAMPLES + sx + s % 8;
        let bits = source.ground_cover[index];
        if bits == 0 {
            continue;
        }
        let weights = layer_weights(source.alpha_words[index], layers.len());
        for (k, cover) in covers.iter().enumerate() {
            let Some(cover) = cover.filter(|_| bits >> k & 1 != 0) else { continue };
            slot_weights[k] = layers
                .iter()
                .zip(weights)
                .filter(|(ltex, _)| cover.land_textures.contains(ltex))
                .map(|(_, weight)| weight)
                .sum();
        }
    }
    let mut params = Vec::new();
    for (k, cover) in covers.iter().enumerate() {
        let Some(cover) = cover else { continue };
        for &(grass, value) in &cover.grasses {
            if params.len() == MAX_PARAMS_PER_BLOCK {
                return params;
            }
            let pct = if value == GCVR_DENSITY_FROM_GRASS { f32::from(grass_density(grass)) } else { f32::from(value) };
            let densities = std::array::from_fn(|s| {
                let weight = texture_weight[s][k];
                if weight < cover.ynam { 0.0 } else { weight * pct * 0.01 }
            });
            params.push((grass, densities));
        }
    }
    params
}

#[derive(Clone, Copy)]
struct Candidate {
    x: f32,
    y: f32,
    noise: f32,
    weight: f32,
}

/// `CreateGrassPoissonDiskSample`, BTD branch, for one block centred at (`cx`, `cy`).
/// Returns the points in list order; `seed` is left where sampling stopped.
pub fn poisson_block(cx: f32, cy: f32, densities: &[f32; 64], mask: &ClumpMask, seed: &mut i32) -> Vec<[f32; 2]> {
    let (min_r, max_r) = (MIN_GRASS_DISTANCE, MAX_GRASS_DISTANCE);
    let (lo_x, lo_y, hi_x, hi_y) = (cx - 128.0, cy - 128.0, cx + 128.0, cy + 128.0);
    let mut points: Vec<Candidate> = Vec::with_capacity(121);
    for (s, &density) in densities.iter().enumerate() {
        if density <= 0.0 {
            continue;
        }
        let jx = 2.0 * next_random(seed) - 1.0;
        let jy = 2.0 * next_random(seed) - 1.0;
        let x = ((s & 7) as f32 + jx) * SAMPLE_UNITS + lo_x;
        let y = ((s >> 3) as f32 + jy) * SAMPLE_UNITS + lo_y;
        let noise = mask.value(x, y);
        if noise < 1.0 {
            points.push(Candidate { x, y, noise, weight: density });
            break;
        }
    }
    let mut i = 0;
    while i < points.len() {
        let p = points[i];
        let base = min_r + (1.0 - p.weight) * (max_r - min_r);
        let radius = base + (max_r - base) * p.noise;
        for _ in 0..TRIES_PER_POINT {
            let distance = next_random(seed) * (max_r - radius) + radius;
            let angle = next_random(seed) * std::f32::consts::TAU;
            let (sin, cos) = angle.sin_cos();
            let x = p.x + distance * cos;
            let y = p.y + distance * sin;
            if !(x > lo_x && y > lo_y && x < hi_x && y < hi_y) {
                continue;
            }
            let column = (((x - cx) / SAMPLE_UNITS + 4.0) as i32).clamp(0, 7) as usize;
            let row = (((y - cy) / SAMPLE_UNITS + 4.0) as i32).clamp(0, 7) as usize;
            let weight = densities[column + 8 * row].cbrt();
            let noise = mask.value(x, y);
            if noise >= 1.0 {
                continue;
            }
            let base = min_r + (1.0 - weight) * (max_r - min_r);
            let r = base + (max_r - base) * noise;
            let r2 = r * r;
            if points.iter().any(|q| (x - q.x) * (x - q.x) + (y - q.y) * (y - q.y) < r2) {
                continue;
            }
            points.push(Candidate { x, y, noise, weight });
        }
        i += 1;
    }
    points.into_iter().map(|p| [p.x, p.y]).collect()
}

/// The whole `AddCellGrass` BTD loop for one cell: blocks local x outer, y inner; params
/// whose densities are all <= 0 are skipped but still advance the param index (seed).
/// `grass_mask` is `None` for a grass the caller cannot place; its params count, unsampled.
/// Blocks are independent (own seeds, no cross-block distance test), so they run in parallel
/// and are concatenated in FO76's order.
pub fn cell_runs(
    source: &CellSource,
    grass_density: &(dyn Fn(usize) -> u8 + Sync),
    grass_mask: &(dyn Fn(usize) -> Option<ClumpMask> + Sync),
) -> Vec<ParamRun> {
    use rayon::prelude::*;
    (0..BLOCKS_PER_AXIS * BLOCKS_PER_AXIS)
        .into_par_iter()
        .map(|block| {
            let (block_x, block_y) = (block / BLOCKS_PER_AXIS, block % BLOCKS_PER_AXIS);
            let (lx, ly) = (block_x * BLOCK_SAMPLES, block_y * BLOCK_SAMPLES);
            let cx = source.cell_x as f32 * CELL_UNITS + (lx + 4) as f32 * SAMPLE_UNITS;
            let cy = source.cell_y as f32 * CELL_UNITS + (ly + 4) as f32 * SAMPLE_UNITS;
            let mut runs = Vec::new();
            for (k, (grass, densities)) in block_params(source, lx, ly, grass_density).into_iter().enumerate() {
                if densities.iter().all(|&d| d <= 0.0) {
                    continue;
                }
                let Some(mask) = grass_mask(grass) else { continue };
                let mut seed = block_seed(source.cell_x, source.cell_y, lx, ly, k);
                let points = poisson_block(cx, cy, &densities, &mask, &mut seed);
                if points.is_empty() {
                    continue;
                }
                runs.push(ParamRun {
                    block_x: block_x as u8,
                    block_y: block_y as u8,
                    param_index: k as u8,
                    grass,
                    seed_after: seed,
                    points,
                });
            }
            runs
        })
        .flatten()
        .collect()
}

pub fn block_seed(cell_x: i32, cell_y: i32, lx: usize, ly: usize, param_index: usize) -> i32 {
    31i32
        .wrapping_mul(cell_x)
        .wrapping_add(29i32.wrapping_mul(cell_y))
        .wrapping_add(23 * lx as i32)
        .wrapping_add(17 * ly as i32)
        .wrapping_add(13 * param_index as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forest_grass() -> GrassDnam {
        // Forest76GrassObj03A DNAM bytes (spec §2.1).
        let bytes = [
            0x00, 0x00, 0xbe, 0x42, 0xcd, 0xcc, 0xcc, 0x3d, 0x9a, 0x99, 0x99, 0x3e, 0xcd, 0xcc, 0x4c,
            0x3e, 0x00, 0x00, 0x11, 0x43, 0x00, 0x00, 0xff, 0xff, 0x5e, 0x00, 0x39, 0x15, 0x00, 0x00,
            0xc2, 0x42, 0xcd, 0xcc, 0xcc, 0x3d,
        ];
        GrassDnam::from_file_bytes(&bytes).unwrap()
    }

    #[test]
    fn random_table_matches_bsrandom_test_vectors() {
        // Spec §4.3 TBL[0..7]; also reproduced by handover/perlin_ref.py's independent MT19937.
        let expected = [0.4170178f32, 0.9971748, 0.7203173, 0.932548, 0.0001144, 0.1281232, 0.3023295, 0.9990305];
        for (i, value) in expected.iter().enumerate() {
            assert!((random_table()[i] - value).abs() < 5e-7, "TBL[{i}] = {}", random_table()[i]);
        }
        assert_eq!(random_by_seed(2048), random_table()[0]);
        assert_eq!(random_by_seed(-1), random_table()[2047]);
        assert!(random_table().iter().all(|&v| (0.0..1.0).contains(&v)));
    }

    #[test]
    fn forest_grass_dnam_reorders_file_fields() {
        let dnam = forest_grass();
        assert_eq!((dnam.sharp, dnam.cover), (95.0, 97.0));
        assert!((dnam.freq_base - 0.1).abs() < 1e-7 && (dnam.lod_blend - 0.1).abs() < 1e-7);
        assert!((dnam.height_range - 0.3).abs() < 1e-7 && (dnam.color_range - 0.2).abs() < 1e-7);
        assert_eq!(dnam.wave_period, 145.0);
        assert_eq!((dnam.water_above, dnam.water_below), (0, 0xFFFF));
        assert_eq!((dnam.density, dnam.min_slope, dnam.max_slope, dnam.flags), (94, 0, 57, 0x15));
        let older = GrassDnam::from_file_bytes(&[0; 28]).unwrap();
        assert_eq!((older.cover, older.lod_blend), (50.0, 1.0));
    }

    #[test]
    fn forest_clump_mask_thresholds_match_spec() {
        let mask = ClumpMask::from_dnam(&forest_grass());
        let (lo, hi) = mask.thresholds();
        assert!((lo - 0.0285).abs() < 1e-5 && (hi - 0.0315).abs() < 1e-5, "{lo} {hi}");
        assert!((mask.frequency - 0.01).abs() < 1e-6);
        assert!((mask.sharp - 0.95).abs() < 1e-6);
        let zero = ClumpMask::from_dnam(&GrassDnam { freq_base: 0.0, ..forest_grass() });
        assert!((zero.frequency - 0.001).abs() < 1e-9);
    }

    #[test]
    fn perlin_matches_reference_values() {
        // Expected values from handover/perlin_ref.py: an independent double-precision
        // implementation of Perlin's improved noise with the same table and gradients.
        assert_eq!(perlin2(3.0, 7.0), 0.0);
        for (x, y, expected) in [
            (0.5f32, 0.5f32, -0.5f32),
            (1.25, 2.75, -0.27780533),
            (12.3, 45.6, -0.19093455),
            (300.7, 0.1, 0.27178954),
        ] {
            let value = perlin2(x, y);
            // f32 inputs (12.3, 45.6, 300.7, 0.1) are not exact, so allow a few ulps of drift.
            assert!((value - expected).abs() < 1e-4, "perlin2({x},{y}) = {value}, want {expected}");
        }
    }

    #[test]
    fn layer_list_stops_at_first_empty_byte_and_skips_byte_five() {
        assert_eq!(quadrant_layer_indices(&[3, 2, 0, 1, 1, 1, 1, 1], 10), vec![7, 8]);
        assert_eq!(quadrant_layer_indices(&[1, 2, 3, 4, 5, 9, 6, 7], 10), vec![9, 8, 7, 6, 5, 4]);
        assert_eq!(quadrant_layer_indices(&[11, 1, 1, 1, 1, 1, 1, 1], 10), Vec::<usize>::new());
        // Front to back: w0 = a0, w1 = (1-a0)*a1, the last layer takes the rest.
        let w = layer_weights(7 | (3 << 3), 3);
        assert_eq!((w[0], w[1], w[2]), (1.0, 0.0, 0.0));
        let w = layer_weights(3 | (7 << 3), 3);
        assert!((w[0] - 3.0 / 7.0).abs() < 1e-6 && (w[1] - 4.0 / 7.0).abs() < 1e-6 && w[2].abs() < 1e-6);
        assert_eq!(layer_weights(0, 1)[0], 1.0);
    }

    fn uniform_source<'a>(ground_cover: &'a [u8], alpha_words: &'a [u16], cover: &'a GroundCover) -> CellSource<'a> {
        CellSource {
            cell_x: 0,
            cell_y: 0,
            ground_cover,
            alpha_words,
            quadrant_layers: std::array::from_fn(|_| vec![0x100]),
            quadrant_ground_covers: std::array::from_fn(|_| {
                let mut slots = [None; 8];
                slots[2] = Some(cover);
                slots
            }),
        }
    }

    #[test]
    fn params_follow_slot_then_gnam_order_with_threshold_and_sentinel() {
        let cover = GroundCover { land_textures: vec![0x100], ynam: 0.286, grasses: vec![(5, 50), (6, GCVR_DENSITY_FROM_GRASS), (7, 0)] };
        let bits = vec![1u8 << 2; CELL_SAMPLES * CELL_SAMPLES];
        let words = vec![0u16; CELL_SAMPLES * CELL_SAMPLES];
        let source = uniform_source(&bits, &words, &cover);
        let params = block_params(&source, 0, 0, &|_| 94);
        assert_eq!(params.iter().map(|p| p.0).collect::<Vec<_>>(), vec![5, 6, 7]);
        assert!(params[0].1.iter().all(|&d| (d - 0.5).abs() < 1e-6));
        assert!(params[1].1.iter().all(|&d| (d - 0.94).abs() < 1e-6));
        assert!(params[2].1.iter().all(|&d| d == 0.0));

        // A second layer on top with alpha 6/7 leaves 1/7 < YNAM to the GCVR's texture.
        let covered = vec![6u16; CELL_SAMPLES * CELL_SAMPLES];
        let mut source = uniform_source(&bits, &covered, &cover);
        source.quadrant_layers = std::array::from_fn(|_| vec![0x200, 0x100]);
        assert!(block_params(&source, 0, 0, &|_| 94)[0].1.iter().all(|&d| d == 0.0));

        // Without the slot bit, no weight.
        let other_bit = vec![1u8; CELL_SAMPLES * CELL_SAMPLES];
        let source = uniform_source(&other_bit, &words, &cover);
        assert!(block_params(&source, 0, 0, &|_| 94).iter().all(|p| p.1.iter().all(|&d| d == 0.0)));
    }

    #[test]
    fn params_cap_at_twenty_four() {
        let cover = GroundCover { land_textures: vec![0x100], ynam: 0.0, grasses: (0..30).map(|g| (g, 100)).collect() };
        let bits = vec![0xFFu8; CELL_SAMPLES * CELL_SAMPLES];
        let words = vec![0u16; CELL_SAMPLES * CELL_SAMPLES];
        let source = uniform_source(&bits, &words, &cover);
        assert_eq!(block_params(&source, 64, 64, &|_| 0).len(), MAX_PARAMS_PER_BLOCK);
    }

    #[test]
    fn seeds_follow_the_spec_formula() {
        assert_eq!(block_seed(0, 0, 0, 0, 0), 0);
        assert_eq!(block_seed(-3, 5, 8, 120, 2), 31 * -3 + 29 * 5 + 23 * 8 + 17 * 120 + 13 * 2);
    }

    /// cover 1 => c = 0, lo = hi = 0 => (n01-0)/0 = +inf clamps to 1 => mask 0 (always open).
    fn open_mask() -> ClumpMask {
        ClumpMask { frequency: 0.0, sharp: 0.0, cover: 1.0 }
    }

    #[test]
    fn poisson_block_golden_case() {
        assert_eq!(open_mask().value(1234.0, -77.0), 0.0);
        let densities = [1.0f32; 64];
        let mut seed = 0;
        let points = poisson_block(128.0, 128.0, &densities, &open_mask(), &mut seed);

        // Seeding: sample 0 jitters by (2*TBL[0]-1, 2*TBL[1]-1) samples from the block corner.
        let tbl = random_table();
        assert_eq!(points[0], [(2.0 * tbl[0] - 1.0) * 32.0, (2.0 * tbl[1] - 1.0) * 32.0]);
        // The seeding point is not bounds-tested, so its jitter can leave the block by up to
        // one sample; grown points stay strictly inside. All are at least minR apart.
        assert!(points[0][0] > -32.0 && points[0][1] > -32.0);
        for (i, p) in points.iter().enumerate() {
            if i > 0 {
                assert!(p[0] > 0.0 && p[0] < 256.0 && p[1] > 0.0 && p[1] < 256.0);
            }
            for q in &points[..i] {
                let d2 = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2);
                assert!(d2 >= MIN_GRASS_DISTANCE * MIN_GRASS_DISTANCE);
            }
        }
        // 2 seeding draws, then 16 tries x 2 draws for every point in the list.
        assert_eq!(seed, 2 + 32 * points.len() as i32);
        // 71 points (59% of the 121 hexagonal bound, §8), last at (253.82681, 53.85709): an
        // independent numpy-float32 port of spec §4 run on the same inputs gives the same count,
        // final seed (2274) and last point.
        assert_eq!(points.len(), 71);
        let last = points[70];
        assert!((last[0] - 253.82681).abs() < 1e-3 && (last[1] - 53.85709).abs() < 1e-3, "{last:?}");
        let mut again = 0;
        assert_eq!(poisson_block(128.0, 128.0, &densities, &open_mask(), &mut again), points);
    }

    #[test]
    fn poisson_block_seeding_skips_zero_density_without_drawing() {
        let mut densities = [0.0f32; 64];
        densities[9] = 0.125;
        let mut seed = 40;
        let points = poisson_block(0.0, 0.0, &densities, &open_mask(), &mut seed);
        let tbl = random_table();
        let first = [(1.0 + (2.0 * tbl[40] - 1.0)) * 32.0 - 128.0, (1.0 + (2.0 * tbl[41] - 1.0)) * 32.0 - 128.0];
        assert_eq!(points[0], first);
        // Seed radius 25 + 0.875*150; grown candidates elsewhere use cbrt(0) = 0 => radius 175.
        assert!(points.len() < 12, "{}", points.len());
    }

    #[test]
    fn closed_mask_rejects_every_point() {
        // cover 0 => c = 1 => lo = hi = 1, n01 <= 1 => mask 1 everywhere => no seeding point.
        let closed = ClumpMask { frequency: 0.01, sharp: 0.0, cover: 0.0 };
        let mut seed = 0;
        assert!(poisson_block(0.0, 0.0, &[1.0; 64], &closed, &mut seed).is_empty());
        assert_eq!(seed, 128, "every positive-density sample tries once");
    }

    #[test]
    fn cell_runs_skip_zero_params_but_keep_their_index() {
        let cover = GroundCover { land_textures: vec![0x100], ynam: 0.286, grasses: vec![(0, 0), (1, 100)] };
        let bits = vec![1u8 << 2; CELL_SAMPLES * CELL_SAMPLES];
        let words = vec![0u16; CELL_SAMPLES * CELL_SAMPLES];
        let source = uniform_source(&bits, &words, &cover);
        let runs = cell_runs(&source, &|_| 50, &|_| Some(open_mask()));
        assert_eq!(runs.len(), 256);
        assert!(runs.iter().all(|run| run.param_index == 1 && run.grass == 1));
        assert_eq!((runs[0].block_x, runs[0].block_y, runs[1].block_x, runs[1].block_y), (0, 0, 0, 1));
        let mut seed = block_seed(0, 0, 0, 8, 1);
        let expected = poisson_block(128.0, 384.0, &[1.0; 64], &open_mask(), &mut seed);
        assert_eq!((runs[1].points.clone(), runs[1].seed_after), (expected, seed));
    }
}
