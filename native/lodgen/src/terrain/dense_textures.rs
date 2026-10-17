//! Terrain LOD textures composited from the data FO4CS dense terrain draws, so distant terrain looks like the
//! dense land that replaces it when its cell loads.
//!
//! Active with `terrain.source = btd` plus `terrain.btd4_path`. Per FO76 sample the inputs are the BTD's raw
//! selector words, per-quadrant LTEX palettes and RGB555 terrain colours (read through `BtdFile`, the decoder
//! BACUP's BTD4 writer copies into the sidecar's RAWT patches), and the BTD4's raw material bank (blend rows
//! and the FO76 `_d`/`_n`/`_l`/`_r` textures).
//!
//! `shade` is `SampleRawTerrain` from FO4CS `res/Lighting/landscape_raw.hlsli` (the non-native raw path dense
//! cells use, including the `_r` read the FO4CS spec adds), followed by the `* (1 - F0)` and the
//! physical-output inputs `deferred_prepass_landscape.hlsl` derives from it. Each LOD texel averages that result over sub-samples spread across its footprint, with
//! every source texture read at the mip that matches the sub-sample spacing.
//!
//! Outputs per quad, all at the level's diffuse size:
//! - diffuse: mean linear albedo, stored `albedo^(1/2.2)`, which the physical LOD shader linearises with
//!   `pow(x, 2.2)`;
//! - `_msn`: mean world normal from the BTD heights perturbed by the blended `_n` slope, green-up;
//! - `_rmaos`: perceptual roughness (alpha² averaged, plus Toksvig for the normal variance the `_msn` loses),
//!   metalness derived from F0, AO, F0;
//! - `_g` (only when something emits): blended emission × `_l` emissive mask, and the exposure row's luminous
//!   emittance, which FO4CS scales by scene exposure at draw time.

use std::collections::HashMap;
use std::path::PathBuf;

use terrain_native::btd::BtdFile;

use crate::descriptors::QuadDesc;
use crate::settings::{LodSettings, TerrainSource};
use crate::terrain::terrain_lod::TerrainBlock;

const CELL_SAMPLES: i64 = 128;
const CELL_UNITS: f64 = 4096.0;
/// World units between FO76 height/selector samples.
const SAMPLE_UNITS: f64 = CELL_UNITS / CELL_SAMPLES as f64;
const COLOR_STEP: i64 = 4;
const COLOR_SAMPLES: i64 = CELL_SAMPLES / COLOR_STEP;
/// Sub-samples per texel axis stop here; textures are already read at the sub-sample footprint's mip.
const MAX_SUBSAMPLES: usize = 4;
/// Source mips above this edge are never needed: a sub-sample spans at least one FO76 sample (32 units), and
/// terrain materials tile every few hundred units.
const KEPT_MIP_EDGE: usize = 64;
/// Cell rows per strip. Each strip reopens the BTD so its never-evicting tile cache holds at most a band of
/// tiles, not the whole quad.
const STRIP_CELLS: i32 = 8;
const GAMMA: f32 = 2.2;
const DEFAULT_ROUGHNESS: f32 = 0.75;
/// Encodes luminous emittance as `log2(x) / 16` in the glow alpha.
const EXPOSURE_LOG2_RANGE: f32 = 16.0;

// ---------------------------------------------------------------------------
// Source textures
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MapKind {
    /// `_d`: BC3, viewed as sRGB by FO4CS whatever the file says.
    Diffuse,
    /// `_n`: BC5_SNORM.
    Normal,
    /// `_l`: BC3_UNORM (x gloss, y AO, w emissive mask).
    Lighting,
    /// `_r`: BC4_UNORM grey F0.
    Reflectance,
}

struct Mip {
    w: usize,
    h: usize,
    texels: Vec<[f32; 4]>,
}

/// The small end of a texture's mip chain, decoded to linear floats (`Normal` keeps signed xy).
pub(crate) struct MipChain {
    base_edge: f32,
    first: usize,
    mips: Vec<Mip>,
}

impl MipChain {
    /// One texel, for synthetic materials in tests.
    #[cfg(test)]
    pub(crate) fn solid(texel: [f32; 4]) -> Self {
        MipChain { base_edge: 1.0, first: 0, mips: vec![Mip { w: 1, h: 1, texels: vec![texel] }] }
    }

    /// Trilinear, wrapped, at mip `lod` of the original chain (`log2` of texels per sample step).
    fn sample(&self, uv: [f32; 2], lod: f32) -> [f32; 4] {
        let level = (lod - self.first as f32).max(0.0);
        let low = level.floor() as usize;
        if low + 1 >= self.mips.len() {
            return bilinear(&self.mips[self.mips.len() - 1], uv);
        }
        let t = level - low as f32;
        let a = bilinear(&self.mips[low], uv);
        let b = bilinear(&self.mips[low + 1], uv);
        std::array::from_fn(|c| a[c] + (b[c] - a[c]) * t)
    }
}

fn bilinear(mip: &Mip, uv: [f32; 2]) -> [f32; 4] {
    let x = uv[0] * mip.w as f32 - 0.5;
    let y = uv[1] * mip.h as f32 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |dx: i64, dy: i64| {
        let px = (x0 as i64 + dx).rem_euclid(mip.w as i64) as usize;
        let py = (y0 as i64 + dy).rem_euclid(mip.h as i64) as usize;
        mip.texels[py * mip.w + px]
    };
    let (a, b, c, d) = (at(0, 0), at(1, 0), at(0, 1), at(1, 1));
    std::array::from_fn(|i| {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bottom = c[i] + (d[i] - c[i]) * fx;
        top + (bottom - top) * fy
    })
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

#[derive(Clone, Copy, PartialEq)]
enum BlockFormat {
    Bc3,
    Bc4,
    Bc5Snorm,
}

fn dds_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| "DDS header is truncated".to_string())
}

/// Decodes the mips at or below `KEPT_MIP_EDGE` (or the smallest stored one, box-reduced) and completes the
/// chain down to 1×1. Accepts what FO4CS's raw bank accepts for the map: BC3 for `_d`/`_l`, BC5_SNORM for `_n`,
/// BC4_UNORM for `_r`.
pub(crate) fn decode_mip_chain(bytes: &[u8], kind: MapKind) -> Result<MipChain, String> {
    if bytes.get(0..4) != Some(b"DDS ") {
        return Err("not a DDS file".into());
    }
    let height = dds_u32(bytes, 12)? as usize;
    let width = dds_u32(bytes, 16)? as usize;
    let mip_count = (dds_u32(bytes, 28)? as usize).max(1);
    let four_cc = bytes.get(84..88).ok_or("DDS header is truncated")?;
    let (dxgi, data_start) = if four_cc == b"DX10" { (Some(dds_u32(bytes, 128)?), 148) } else { (None, 128) };
    let format = match (dxgi, four_cc) {
        (Some(76 | 77 | 78), _) | (None, b"DXT5") => BlockFormat::Bc3,
        (Some(82 | 84), _) | (None, b"BC5S") => BlockFormat::Bc5Snorm,
        (Some(79 | 80), _) | (None, b"BC4U" | b"ATI1") => BlockFormat::Bc4,
        _ => return Err(format!("unsupported raw terrain texture format {dxgi:?}/{four_cc:?}")),
    };
    let expected = match kind {
        MapKind::Normal => BlockFormat::Bc5Snorm,
        MapKind::Reflectance => BlockFormat::Bc4,
        MapKind::Diffuse | MapKind::Lighting => BlockFormat::Bc3,
    };
    if format != expected || width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(format!("raw terrain {kind:?} texture has the wrong format or size"));
    }
    let mut offset = data_start;
    let mut mips = Vec::new();
    let mut first = 0;
    for level in 0..mip_count {
        let (w, h) = ((width >> level).max(1), (height >> level).max(1));
        let block_bytes = if format == BlockFormat::Bc4 { 8 } else { 16 };
        let len = w.div_ceil(4) * h.div_ceil(4) * block_bytes;
        let blocks = bytes.get(offset..offset + len).ok_or("DDS mip data is truncated")?;
        offset += len;
        if w.max(h) <= KEPT_MIP_EDGE || level + 1 == mip_count {
            if mips.is_empty() {
                first = level;
            }
            mips.push(Mip { w, h, texels: decode_blocks(blocks, w, h, format, kind) });
        }
    }
    // A chain that stops above KEPT_MIP_EDGE kept only its last mip; reduce it, then finish the chain.
    while mips[0].w.max(mips[0].h) > KEPT_MIP_EDGE {
        mips = vec![box_reduce(&mips[0])];
        first += 1;
    }
    while let Some(last) = mips.last().filter(|m| m.w > 1 || m.h > 1) {
        let reduced = box_reduce(last);
        mips.push(reduced);
    }
    Ok(MipChain { base_edge: width.max(height) as f32, first, mips })
}

fn box_reduce(mip: &Mip) -> Mip {
    let (w, h) = ((mip.w / 2).max(1), (mip.h / 2).max(1));
    let texels = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let xs = [(2 * x).min(mip.w - 1), (2 * x + 1).min(mip.w - 1)];
            let ys = [(2 * y).min(mip.h - 1), (2 * y + 1).min(mip.h - 1)];
            let mut sum = [0.0f32; 4];
            for yy in ys {
                for xx in xs {
                    let t = mip.texels[yy * mip.w + xx];
                    sum.iter_mut().zip(t).for_each(|(s, v)| *s += v * 0.25);
                }
            }
            sum
        })
        .collect();
    Mip { w, h, texels }
}

fn decode_blocks(blocks: &[u8], w: usize, h: usize, format: BlockFormat, kind: MapKind) -> Vec<[f32; 4]> {
    let mut texels = vec![[0.0f32; 4]; w * h];
    let bw = w.div_ceil(4);
    let block_bytes = if format == BlockFormat::Bc4 { 8 } else { 16 };
    for (index, block) in blocks.chunks_exact(block_bytes).enumerate() {
        let (bx, by) = (index % bw * 4, index / bw * 4);
        let decoded: [[f32; 4]; 16] = match format {
            BlockFormat::Bc3 => {
                let alpha = bc4_unorm(&block[0..8]);
                let color = bc1_color(&block[8..16]);
                std::array::from_fn(|i| {
                    let rgb = color[i].map(|c| if kind == MapKind::Diffuse { srgb_to_linear(c) } else { c });
                    [rgb[0], rgb[1], rgb[2], alpha[i]]
                })
            }
            BlockFormat::Bc4 => bc4_unorm(block).map(|v| [v, v, v, 1.0]),
            BlockFormat::Bc5Snorm => {
                let (x, y) = (bc4_snorm(&block[0..8]), bc4_snorm(&block[8..16]));
                std::array::from_fn(|i| [x[i], y[i], 0.0, 1.0])
            }
        };
        for (i, texel) in decoded.into_iter().enumerate() {
            let (px, py) = (bx + i % 4, by + i / 4);
            if px < w && py < h {
                texels[py * w + px] = texel;
            }
        }
    }
    texels
}

fn bc1_color(block: &[u8]) -> [[f32; 3]; 16] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let expand = |c: u16| {
        let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
        [((r << 3) | (r >> 2)) as f32 / 255.0, ((g << 2) | (g >> 4)) as f32 / 255.0, ((b << 3) | (b >> 2)) as f32 / 255.0]
    };
    let (a, b) = (expand(c0), expand(c1));
    // BC3 colour blocks always use the four-colour palette.
    let palette = [a, b, std::array::from_fn(|i| (2.0 * a[i] + b[i]) / 3.0), std::array::from_fn(|i| (a[i] + 2.0 * b[i]) / 3.0)];
    let bits = u32::from_le_bytes(block[4..8].try_into().unwrap());
    std::array::from_fn(|i| palette[((bits >> (2 * i)) & 3) as usize])
}

fn bc4_indices(block: &[u8]) -> [usize; 16] {
    let bits = block[2..8].iter().rev().fold(0u64, |acc, b| (acc << 8) | u64::from(*b));
    std::array::from_fn(|i| ((bits >> (3 * i)) & 7) as usize)
}

fn bc4_palette(e0: f32, e1: f32, six: bool, low: f32, high: f32) -> [f32; 8] {
    if six {
        std::array::from_fn(|i| match i {
            0 => e0,
            1 => e1,
            _ => ((8 - i) as f32 * e0 + (i - 1) as f32 * e1) / 7.0,
        })
    } else {
        std::array::from_fn(|i| match i {
            0 => e0,
            1 => e1,
            6 => low,
            7 => high,
            _ => ((6 - i) as f32 * e0 + (i - 1) as f32 * e1) / 5.0,
        })
    }
}

fn bc4_unorm(block: &[u8]) -> [f32; 16] {
    let (e0, e1) = (block[0], block[1]);
    let palette = bc4_palette(f32::from(e0) / 255.0, f32::from(e1) / 255.0, e0 > e1, 0.0, 1.0);
    bc4_indices(block).map(|i| palette[i])
}

fn bc4_snorm(block: &[u8]) -> [f32; 16] {
    let (e0, e1) = (block[0] as i8, block[1] as i8);
    let norm = |v: i8| (f32::from(v) / 127.0).max(-1.0);
    let palette = bc4_palette(norm(e0), norm(e1), e0 > e1, -1.0, 1.0);
    bc4_indices(block).map(|i| palette[i])
}

// ---------------------------------------------------------------------------
// Material bank
// ---------------------------------------------------------------------------

/// One raw bank entry: the rows FO4CS uploads to `rawTerrainMaterials` and its `_d`/`_n`/`_l`/`_r` maps.
pub(crate) struct RawMaterial {
    /// x blend softness, y sin, z cos of the rotation, w texture repeats per world unit.
    pub(crate) blend: [f32; 4],
    /// rgb linear emittance colour, w > 0.1 when that colour overrides the diffuse.
    pub(crate) emission: [f32; 4],
    /// x luminous emittance (the only lane the runtime scales by).
    pub(crate) exposure: [f32; 4],
    pub(crate) maps: [MipChain; 4],
}

/// The bank in runtime order (palette id `n` is `materials[n - 1]`) with its textures decoded.
pub struct DenseTerrainSource {
    btd_path: String,
    pub(crate) materials: Vec<RawMaterial>,
}

impl DenseTerrainSource {
    /// `Ok(None)` when the run doesn't composite from the BTD; `Err` names why it can't, so the caller warns
    /// once and keeps LAND textures. Any missing or malformed texture disables the whole bank, as FO4CS
    /// refuses a bank it cannot fully load and dense terrain is then not drawn either.
    pub fn load(settings: &LodSettings, data_dirs: &[PathBuf]) -> Result<Option<Self>, String> {
        let terrain = &settings.terrain;
        if terrain.source != TerrainSource::Btd {
            return Ok(None);
        }
        let btd_path = terrain.btd_path.clone().ok_or("terrain.source=btd has no BTD path")?;
        let btd4 = terrain.btd4_path.as_deref().ok_or("no dense terrain sidecar (terrain.btd4_path)")?;
        let bank = terrain_native::btd4::read_raw_material_bank(std::path::Path::new(btd4))
            .map_err(|e| format!("dense terrain sidecar {btd4}: {e}"))?;
        let entries = terrain_native::raw_terrain::decode_bank(&bank)?;
        let kinds = [MapKind::Diffuse, MapKind::Normal, MapKind::Lighting, MapKind::Reflectance];
        let materials = entries
            .iter()
            .map(|entry| {
                let maps = kinds.iter().zip(&entry.textures).map(|(&kind, path)| {
                    let asset = crate::asset_source::resolve(data_dirs, path)
                        .ok_or_else(|| format!("raw terrain texture {path} is not in the LOD asset sources"))?;
                    let bytes = crate::asset_source::read(data_dirs, &asset).map_err(|e| format!("{path}: {e}"))?;
                    decode_mip_chain(&bytes, kind).map_err(|e| format!("{path}: {e}"))
                });
                let maps: Vec<MipChain> = maps.collect::<Result<_, String>>()?;
                Ok(RawMaterial {
                    blend: entry.rows[0],
                    emission: entry.rows[1],
                    exposure: entry.rows[2],
                    maps: maps.try_into().ok().unwrap(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Some(DenseTerrainSource { btd_path, materials }))
    }

    #[cfg(test)]
    pub(crate) fn from_materials(materials: Vec<RawMaterial>) -> Self {
        DenseTerrainSource { btd_path: String::new(), materials }
    }
}

// ---------------------------------------------------------------------------
// FO76 sample data
// ---------------------------------------------------------------------------

/// Storage-sample frame of the BTD: cell bounds, and the shift from storage coordinates to the shader's
/// `nativeGrid` (`raw_terrain_patch`'s `native_grid_origin` rule).
#[derive(Clone, Copy)]
pub(crate) struct DenseGrid {
    pub(crate) cell_min: [i32; 2],
    pub(crate) cell_max: [i32; 2],
    pub(crate) native_offset: [f64; 2],
}

/// One FO76 cell as the RAWT patch carries it: raw selector words (lod 0, 128²), the four quadrant
/// palettes, RGB555 colours (lod 2, 32²).
pub(crate) struct CellData {
    pub(crate) selectors: Vec<u16>,
    pub(crate) palettes: [[u8; 8]; 4],
    pub(crate) colors: Vec<u16>,
}

pub(crate) trait DenseCells {
    fn grid(&self) -> DenseGrid;
    fn load_cell(&mut self, x: i32, y: i32) -> anyhow::Result<CellData>;
}

impl DenseCells for BtdFile {
    fn grid(&self) -> DenseGrid {
        let h = self.header();
        DenseGrid {
            cell_min: [h.cell_min_x, h.cell_min_y],
            cell_max: [h.cell_max_x, h.cell_max_y],
            native_offset: [
                f64::from(h.cell_min_x) * CELL_SAMPLES as f64 + f64::from(h.resolution_x) * 0.5,
                f64::from(h.cell_min_y) * CELL_SAMPLES as f64 + f64::from(h.resolution_y) * 0.5,
            ],
        }
    }

    fn load_cell(&mut self, x: i32, y: i32) -> anyhow::Result<CellData> {
        Ok(CellData {
            selectors: self.cell_raw_land_alpha_u16(x, y, 0)?,
            palettes: self.cell_raw_ltex_palettes(x, y)?,
            colors: self.cell_terrain_color_u16(x, y, 2)?,
        })
    }
}

/// Per-strip cell cache over a source. Samples outside the BTD read zero, as `raw_terrain_patch` pads them.
struct Cells<S: DenseCells> {
    source: S,
    grid: DenseGrid,
    cells: HashMap<(i32, i32), Option<CellData>>,
    errors: u32,
}

impl<S: DenseCells> Cells<S> {
    fn new(source: S) -> Self {
        let grid = source.grid();
        Cells { source, grid, cells: HashMap::new(), errors: 0 }
    }

    fn cell(&mut self, x: i32, y: i32) -> Option<&CellData> {
        let g = self.grid;
        if x < g.cell_min[0] || x > g.cell_max[0] || y < g.cell_min[1] || y > g.cell_max[1] {
            return None;
        }
        if !self.cells.contains_key(&(x, y)) {
            let loaded = match self.source.load_cell(x, y) {
                Ok(cell) => Some(cell),
                Err(_) => {
                    self.errors += 1;
                    None
                }
            };
            self.cells.insert((x, y), loaded);
        }
        self.cells.get(&(x, y)).and_then(Option::as_ref)
    }

    fn split(sample: i64, per_cell: i64) -> (i32, usize) {
        (sample.div_euclid(per_cell) as i32, sample.rem_euclid(per_cell) as usize)
    }

    fn selector(&mut self, x: i64, y: i64) -> u16 {
        let ((cx, lx), (cy, ly)) = (Self::split(x, CELL_SAMPLES), Self::split(y, CELL_SAMPLES));
        self.cell(cx, cy).map_or(0, |c| c.selectors[ly * CELL_SAMPLES as usize + lx])
    }

    fn palette(&mut self, x: i64, y: i64) -> [u8; 8] {
        let ((cx, lx), (cy, ly)) = (Self::split(x, CELL_SAMPLES), Self::split(y, CELL_SAMPLES));
        self.cell(cx, cy).map_or([0; 8], |c| c.palettes[ly / 64 * 2 + lx / 64])
    }

    /// The RGB555 colour at colour-grid index (x, y), as FO4CS uploads it: each channel `(v5 << 3) / 255`.
    fn color(&mut self, x: i64, y: i64) -> [f32; 3] {
        let ((cx, lx), (cy, ly)) = (Self::split(x, COLOR_SAMPLES), Self::split(y, COLOR_SAMPLES));
        let packed = self.cell(cx, cy).map_or(0, |c| c.colors[ly * COLOR_SAMPLES as usize + lx]);
        [(packed >> 10) & 31, (packed >> 5) & 31, packed & 31].map(|v| f32::from(v << 3) / 255.0)
    }
}

// ---------------------------------------------------------------------------
// The dense blend (landscape_raw.hlsli)
// ---------------------------------------------------------------------------

/// What the dense pixel shader resolves at one point, before lighting.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Shade {
    /// The physical output's albedo input: colour overlay applied, times `1 - f0`. Linear.
    pub(crate) albedo: [f32; 3],
    /// Blended tangent-space slope; the normal is `normalize(slope, 1)`.
    pub(crate) slope: [f32; 2],
    /// Coverage-blended `_l` x (gloss) and y (AO).
    pub(crate) gloss: f32,
    pub(crate) ao: f32,
    /// Coverage-blended `_r` F0.
    pub(crate) f0: f32,
    /// `material.emission * material.lighting.z`, zero where `RawTerrainEmission` returns zero.
    pub(crate) emission: [f32; 3],
    /// Blended luminous emittance, the exposure lane the runtime scales emission with.
    pub(crate) emittance: f32,
}

/// Metalness the object shader derives from `_r` for its IBL lane; terrain uses the same rule.
pub(crate) fn metal_from_f0(f0: f32) -> f32 {
    let t = ((f0 - 0.16) / (0.34 - 0.16)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn palette_words(p: [u8; 8]) -> [u32; 2] {
    let words = [0, 2, 4, 6].map(|i| u32::from(p[i]) | (u32::from(p[i + 1]) << 8));
    [words[0] | (words[1] << 16), words[2] | (words[3] << 8)]
}

/// `RawTerrainColorOverlay`.
pub(crate) fn color_overlay(albedo: [f32; 3], color: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|c| {
        let encoded = ((color[c] * 31.875 - 1.0) / 30.0).clamp(0.0, 1.0);
        let overlay = ((encoded / (1.0 - encoded)).log2() * 0.071543 + 0.5).clamp(0.0, 1.0);
        let gamma = albedo[c].max(0.0).powf(1.0 / GAMMA);
        let mixed = if overlay < 0.5 { 2.0 * gamma * overlay } else { 1.0 - 2.0 * (1.0 - gamma) * (1.0 - overlay) };
        mixed.powf(GAMMA)
    })
}

/// `SampleRawTerrain` at absolute storage-sample position `at`, reading textures at the mip of a
/// `step_units`-wide footprint. `None` where nothing covers the point (FO4CS then draws engine layers).
fn shade<S: DenseCells>(source: &DenseTerrainSource, cells: &mut Cells<S>, at: [f64; 2], step_units: f32) -> Option<Shade> {
    let base = [at[0].floor() as i64, at[1].floor() as i64];
    let fraction = [(at[0] - base[0] as f64) as f32, (at[1] - base[1] as f64) as f32];
    let corners = [(0, 0), (1, 0), (0, 1), (1, 1)];
    let mut selectors = corners.map(|(dx, dy)| u32::from(cells.selector(base[0] + dx, base[1] + dy)) | 0x38000);
    let mut palettes = corners.map(|(dx, dy)| palette_words(cells.palette(base[0] + dx, base[1] + dy)));
    let edge = [base[0].rem_euclid(64) == 63, base[1].rem_euclid(64) == 63];
    let advance = (if edge[0] { 1 } else { 2 }) * (if edge[1] { 1 } else { 2 });
    let native = [at[0] - cells.grid.native_offset[0], at[1] - cells.grid.native_offset[1]];
    let world_uv = [(native[0] * 32.0) as f32, (native[1] * -32.0) as f32];

    let mut albedo = [0.0f32; 3];
    let mut slope = [0.0f32; 2];
    let mut lighting = [0.0f32; 3];
    let mut emission = [0.0f32; 3];
    let mut exposure = [0.0f32; 4];
    let mut f0 = 0.0f32;
    let mut covered = 0.0f32;
    let mut step = 0;
    while step < 24 {
        step += advance;
        let id = palettes.iter().map(|p| p[0] & 255).max().unwrap();
        if id == 0 {
            break;
        }
        let mut weights = [0.0f32; 4];
        for corner in 0..4 {
            if palettes[corner][0] & 255 == id {
                weights[corner] = (selectors[corner] & 7) as f32 / 7.0;
                selectors[corner] >>= 3;
                palettes[corner][0] = (palettes[corner][0] >> 8) | (palettes[corner][1] << 24);
                palettes[corner][1] >>= 8;
            }
        }
        let terminal = palettes.iter().all(|p| p[0] & 255 == 0);
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let selector = lerp(lerp(weights[0], weights[1], fraction[0]), lerp(weights[2], weights[3], fraction[0]), fraction[1]);
        if !terminal && selector <= 0.0 {
            continue;
        }
        let Some(material) = source.materials.get(id as usize - 1) else {
            continue;
        };
        let [softness, sin, cos, scale] = material.blend;
        let rotate = |v: [f32; 2]| [cos * v[0] - sin * v[1], sin * v[0] + cos * v[1]];
        let uv = rotate(world_uv).map(|v| v * scale);
        let lod = |map: &MipChain| (step_units * scale.abs() * map.base_edge).max(1e-6).log2();
        let diffuse = material.maps[0].sample(uv, lod(&material.maps[0]));
        let coverage = if terminal { 1.0 } else { (((softness + 1.0) * selector + diffuse[3] - 1.0) / softness).clamp(0.0, 1.0) };
        if coverage <= 0.0 {
            continue;
        }
        let contribution = coverage * (1.0 - covered);
        let light = material.maps[2].sample(uv, lod(&material.maps[2]));
        let normal = material.maps[1].sample(uv, lod(&material.maps[1]));
        let reflectance = material.maps[3].sample(uv, lod(&material.maps[3]));
        let z = (1.0 - (normal[0] * normal[0] + normal[1] * normal[1]).min(1.0)).sqrt().max(0.1);
        let tilted = rotate([normal[0], -normal[1]]);
        let emitted = if material.emission[3] > 0.1 { &material.emission[..3] } else { &diffuse[..3] };
        for c in 0..3 {
            albedo[c] += contribution * diffuse[c];
            emission[c] += contribution * emitted[c];
        }
        for (l, v) in lighting.iter_mut().zip([light[0], light[1], light[3]]) {
            *l += contribution * v;
        }
        for c in 0..2 {
            slope[c] += contribution * tilted[c] / z;
        }
        f0 += contribution * reflectance[0];
        for c in 0..4 {
            exposure[c] += contribution * material.exposure[c];
        }
        covered = (covered + contribution).min(1.0);
        if coverage >= 1.0 || covered >= 1.0 {
            break;
        }
    }
    if covered <= 0.0 {
        return None;
    }

    // Colour texels sit every 4 samples; FO4CS samples them bilinearly.
    let grid = [at[0] / COLOR_STEP as f64, at[1] / COLOR_STEP as f64];
    let c0 = [grid[0].floor() as i64, grid[1].floor() as i64];
    let t = [(grid[0] - c0[0] as f64) as f32, (grid[1] - c0[1] as f64) as f32];
    let taps = corners.map(|(dx, dy)| cells.color(c0[0] + dx, c0[1] + dy));
    let color: [f32; 3] = std::array::from_fn(|c| {
        let top = taps[0][c] + (taps[1][c] - taps[0][c]) * t[0];
        let bottom = taps[2][c] + (taps[3][c] - taps[2][c]) * t[0];
        top + (bottom - top) * t[1]
    });
    let f0 = (f0 / covered.max(1e-4)).clamp(0.0, 1.0);
    let albedo = color_overlay(albedo, color).map(|v| v * (1.0 - f0));
    let emits = (exposure[0] + exposure[1]) * lighting[2] > 0.0 && exposure[0] > 0.0;
    Some(Shade {
        albedo,
        slope,
        gloss: lighting[0],
        ao: lighting[1],
        f0,
        emission: if emits { emission.map(|v| v * lighting[2]) } else { [0.0; 3] },
        emittance: if emits { exposure[0] } else { 0.0 },
    })
}

// ---------------------------------------------------------------------------
// Per-quad composite
// ---------------------------------------------------------------------------

/// One quad's dense-derived maps, RGBA8 at `size`², row 0 north.
pub struct DenseTile {
    pub size: u32,
    /// `albedo^(1/2.2)`; texels no sub-sample covered are left for the LAND fallback.
    pub diffuse_rgba: Vec<u8>,
    pub normal_rgba: Vec<u8>,
    pub rmaos_rgba: Vec<u8>,
    /// Present only when some texel emits.
    pub glow_rgba: Option<Vec<u8>>,
    /// Per texel: no sub-sample was covered.
    pub uncovered: Vec<bool>,
    pub warnings: Vec<String>,
}

struct Accumulator {
    albedo: [f32; 3],
    /// Unit world normals of every sub-sample, covered or not.
    geometric: [f32; 3],
    alpha2: f32,
    ao: f32,
    metal: f32,
    f0: f32,
    emission: [f32; 3],
    emittance: f32,
    covered: u32,
}

fn encode(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The block's height at fractional block-post coordinates (heights are stored divided by `div`).
fn block_height(block: &TerrainBlock, div: f32, at: [f64; 2]) -> f32 {
    let last = (block.size - 1) as f64;
    let [x, y] = at.map(|v| v.clamp(0.0, last));
    let (x0, y0) = (x.floor().min(last - 1.0).max(0.0) as usize, y.floor().min(last - 1.0).max(0.0) as usize);
    let (x1, y1) = ((x0 + 1).min(block.size - 1), (y0 + 1).min(block.size - 1));
    let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
    let h = |px: usize, py: usize| block.heights[px + py * block.size] * div;
    let top = h(x0, y0) + (h(x1, y0) - h(x0, y0)) * fx;
    let bottom = h(x0, y1) + (h(x1, y1) - h(x0, y1)) * fx;
    top + (bottom - top) * fy
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(f32::EPSILON);
    v.map(|c| c / len)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Composites one quad from the BTD at `source.btd_path`, one strip of cell rows at a time.
pub fn composite_quad_dense(
    source: &DenseTerrainSource,
    quad: &QuadDesc,
    block: &TerrainBlock,
    size: u32,
) -> anyhow::Result<DenseTile> {
    let path = source.btd_path.clone();
    composite_with(source, quad, block, size, || {
        BtdFile::open(&path).map_err(|e| anyhow::anyhow!("open BTD {path}: {e}"))
    })
}

pub(crate) fn composite_with<S: DenseCells>(
    source: &DenseTerrainSource,
    quad: &QuadDesc,
    block: &TerrainBlock,
    size: u32,
    mut open: impl FnMut() -> anyhow::Result<S>,
) -> anyhow::Result<DenseTile> {
    let level = quad.quad_level.max(1);
    let n = size.max(1) as usize;
    let quad_units = CELL_UNITS * f64::from(level);
    let texel_units = quad_units / n as f64;
    let sub = ((texel_units / SAMPLE_UNITS).round() as usize).clamp(1, MAX_SUBSAMPLES);
    let step_units = (texel_units / sub as f64) as f32;
    let post_units = CELL_UNITS / block.posts_per_cell as f64;
    let div = level as f32;
    let origin = [f64::from(quad.x) * CELL_UNITS, f64::from(quad.y) * CELL_UNITS];

    let mut diffuse = vec![0u8; n * n * 4];
    let mut normal = vec![0u8; n * n * 4];
    let mut rmaos = vec![0u8; n * n * 4];
    let mut glow = vec![0u8; n * n * 4];
    let mut glows = false;
    let mut uncovered = vec![false; n * n];
    let mut errors = 0u32;

    // Image row r covers world y from origin + (n - 1 - r) * texel; process south to north in strips.
    let rows_per_strip = ((n as f64) * f64::from(STRIP_CELLS) / f64::from(level)).ceil().max(1.0) as usize;
    for strip in (0..n).step_by(rows_per_strip) {
        let mut cells = Cells::new(open()?);
        for south_row in strip..(strip + rows_per_strip).min(n) {
            let row = n - 1 - south_row;
            for col in 0..n {
                let mut acc = Accumulator {
                    albedo: [0.0; 3],
                    geometric: [0.0; 3],
                    alpha2: 0.0,
                    ao: 0.0,
                    metal: 0.0,
                    f0: 0.0,
                    emission: [0.0; 3],
                    emittance: 0.0,
                    covered: 0,
                };
                for sy in 0..sub {
                    for sx in 0..sub {
                        let world = [
                            origin[0] + (col as f64 + (sx as f64 + 0.5) / sub as f64) * texel_units,
                            origin[1] + (south_row as f64 + (sy as f64 + 0.5) / sub as f64) * texel_units,
                        ];
                        let post = [(world[0] - origin[0]) / post_units, (world[1] - origin[1]) / post_units];
                        let span = 2.0 * post_units as f32;
                        let dzdx = (block_height(block, div, [post[0] + 1.0, post[1]])
                            - block_height(block, div, [post[0] - 1.0, post[1]]))
                            / span;
                        let dzdy = (block_height(block, div, [post[0], post[1] + 1.0])
                            - block_height(block, div, [post[0], post[1] - 1.0]))
                            / span;
                        let geometric = normalize([-dzdx, -dzdy, 1.0]);
                        let Some(s) = shade(source, &mut cells, world.map(|w| w / SAMPLE_UNITS + 64.0), step_units)
                        else {
                            for c in 0..3 {
                                acc.geometric[c] += geometric[c];
                            }
                            continue;
                        };
                        let tangent = normalize([1.0, 0.0, dzdx]);
                        let bitangent = cross(geometric, tangent);
                        let ts = normalize([s.slope[0], s.slope[1], 1.0]);
                        let world_normal =
                            normalize(std::array::from_fn(|c| tangent[c] * ts[0] + bitangent[c] * ts[1] + geometric[c] * ts[2]));
                        for c in 0..3 {
                            acc.geometric[c] += world_normal[c];
                        }
                        let roughness = 1.0 - (0.25 + 0.74 * s.gloss.clamp(0.0, 1.0));
                        acc.alpha2 += roughness.powi(4);
                        for c in 0..3 {
                            acc.albedo[c] += s.albedo[c];
                            acc.emission[c] += s.emission[c];
                        }
                        acc.ao += s.ao.clamp(0.0, 1.0);
                        acc.metal += metal_from_f0(s.f0);
                        acc.f0 += s.f0;
                        acc.emittance += s.emittance * (s.emission[0] + s.emission[1] + s.emission[2]);
                        acc.covered += 1;
                    }
                }
                let i = (row * n + col) * 4;
                let mean = acc.geometric.map(|v| v / (sub * sub) as f32);
                let length = (mean[0] * mean[0] + mean[1] * mean[1] + mean[2] * mean[2]).sqrt().clamp(1e-3, 1.0);
                let unit = mean.map(|v| v / length);
                let enc = |v: f32| encode(v * 0.5 + 0.5);
                normal[i..i + 4].copy_from_slice(&[enc(unit[0]), enc(unit[2]), enc(unit[1]), 255]);
                if acc.covered == 0 {
                    uncovered[row * n + col] = true;
                    rmaos[i..i + 4].copy_from_slice(&[encode(DEFAULT_ROUGHNESS), 0, 255, encode(0.04)]);
                    continue;
                }
                let inv = 1.0 / acc.covered as f32;
                for c in 0..3 {
                    diffuse[i + c] = encode((acc.albedo[c] * inv).max(0.0).powf(1.0 / GAMMA));
                }
                diffuse[i + 3] = 255;
                // Toksvig: the spread of the averaged normals widens the lobe the `_msn` can no longer show.
                let alpha2 = (acc.alpha2 * inv + (1.0 - length) / length).min(1.0);
                rmaos[i..i + 4].copy_from_slice(&[
                    encode(alpha2.sqrt().sqrt()),
                    encode(acc.metal * inv),
                    encode(acc.ao * inv),
                    encode(acc.f0 * inv),
                ]);
                let emitted = acc.emission.map(|v| v * inv);
                let weight = acc.emission.iter().sum::<f32>();
                if weight > 0.0 {
                    glows = true;
                    let emittance = acc.emittance / weight;
                    for c in 0..3 {
                        glow[i + c] = encode(emitted[c].powf(1.0 / GAMMA));
                    }
                    glow[i + 3] = encode(emittance.max(1.0).log2() / EXPOSURE_LOG2_RANGE);
                }
            }
        }
        errors += cells.errors;
    }
    let mut warnings = Vec::new();
    if errors > 0 {
        warnings.push(format!("{errors} BTD cell(s) failed to decode; their texels use LAND"));
    }
    Ok(DenseTile {
        size: n as u32,
        diffuse_rgba: diffuse,
        normal_rgba: normal,
        rmaos_rgba: rmaos,
        glow_rgba: glows.then_some(glow),
        uncovered,
        warnings,
    })
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Where one quad's tiles go. `glow` is written only when the tile emits.
pub struct DenseTilePaths<'a> {
    pub diffuse: &'a std::path::Path,
    pub msn: &'a std::path::Path,
    pub rmaos: &'a std::path::Path,
    pub glow: &'a std::path::Path,
}

/// Companion maps keep four independent channels, so they use BC3 whatever the diffuse format is.
const COMPANION_FORMAT: &str = "BC3_UNORM";

fn gamma_to_linear_table() -> [f32; 256] {
    std::array::from_fn(|v| (v as f32 / 255.0).powf(GAMMA))
}

/// Mips averaged in linear light and stored `x^(1/2.2)`, the encoding the physical LOD shader decodes.
fn gamma_mip_chain(size: u32, rgba: Vec<u8>, mips: bool) -> Vec<(u32, u32, Vec<u8>)> {
    let table = gamma_to_linear_table();
    let mut chain = vec![(size, size, rgba)];
    while mips && chain.last().unwrap().0 > 1 {
        let (w, _, src) = chain.last().unwrap();
        let (w, half) = (*w as usize, (*w as usize / 2).max(1));
        let mut out = vec![0u8; half * half * 4];
        for y in 0..half {
            for x in 0..half {
                for c in 0..4 {
                    let at = |dx: usize, dy: usize| src[((2 * y + dy) * w + 2 * x + dx) * 4 + c];
                    let taps = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
                    out[(y * half + x) * 4 + c] = if c == 3 {
                        ((taps.iter().map(|&v| u32::from(v)).sum::<u32>() + 2) / 4) as u8
                    } else {
                        encode((taps.iter().map(|&v| table[v as usize]).sum::<f32>() * 0.25).powf(1.0 / GAMMA))
                    };
                }
            }
        }
        chain.push((half as u32, half as u32, out));
    }
    chain
}

/// Bilinear resample of an encoded `_msn`, renormalised.
fn resample_normals(rgba: &[u8], from: usize, to: usize) -> Vec<u8> {
    let decode = |i: usize| -> [f32; 3] { std::array::from_fn(|c| f32::from(rgba[i * 4 + c]) / 127.5 - 1.0) };
    let mut out = vec![0u8; to * to * 4];
    for y in 0..to {
        for x in 0..to {
            let u = ((x as f32 + 0.5) * from as f32 / to as f32 - 0.5).clamp(0.0, (from - 1) as f32);
            let v = ((y as f32 + 0.5) * from as f32 / to as f32 - 0.5).clamp(0.0, (from - 1) as f32);
            let (x0, y0) = (u.floor() as usize, v.floor() as usize);
            let (x1, y1) = ((x0 + 1).min(from - 1), (y0 + 1).min(from - 1));
            let (fx, fy) = (u - x0 as f32, v - y0 as f32);
            let (a, b, c, d) = (decode(y0 * from + x0), decode(y0 * from + x1), decode(y1 * from + x0), decode(y1 * from + x1));
            let n = normalize(std::array::from_fn(|i| {
                let top = a[i] + (b[i] - a[i]) * fx;
                top + (c[i] + (d[i] - c[i]) * fx - top) * fy
            }));
            let o = (y * to + x) * 4;
            out[o..o + 4].copy_from_slice(&[encode(n[0] * 0.5 + 0.5), encode(n[1] * 0.5 + 0.5), encode(n[2] * 0.5 + 0.5), 255]);
        }
    }
    out
}

/// Writes the quad's diffuse, `_msn`, `_rmaos` and (when it emits) `_g`. Texels no dense sample covered take
/// `land`'s diffuse (linear, as `composite_quad` builds it), which must then be at the tile's size. Returns
/// whether `_g` was written.
pub fn write_dense_tile(
    tile: &DenseTile,
    land: Option<&crate::terrain::textures::CompositeTile>,
    paths: &DenseTilePaths<'_>,
    settings: &LodSettings,
    level: i32,
) -> anyhow::Result<bool> {
    use crate::terrain::textures::{format_to_dxgi, lod_index};
    let lvl = &settings.terrain.levels[lod_index(level)];
    let n = tile.size as usize;
    let mut diffuse = tile.diffuse_rgba.clone();
    if tile.uncovered.iter().any(|&u| u) {
        let land = land
            .filter(|land| land.width == tile.size && land.height == tile.size)
            .ok_or_else(|| anyhow::anyhow!("dense tile has uncovered texels but no LAND tile of its size"))?;
        for (texel, _) in tile.uncovered.iter().enumerate().filter(|(_, u)| **u) {
            for c in 0..3 {
                diffuse[texel * 4 + c] = encode((f32::from(land.diffuse_rgba[texel * 4 + c]) / 255.0).powf(1.0 / GAMMA));
            }
            diffuse[texel * 4 + 3] = 255;
        }
    }
    for path in [paths.diffuse, paths.msn, paths.rmaos, paths.glow] {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let encoded = directxtex_native::encode_dds_from_rgba8_chain(
        &gamma_mip_chain(tile.size, diffuse, lvl.diffuse_mipmap),
        format_to_dxgi(&lvl.diffuse_format),
        true,
        None,
    )
    .map_err(|e| anyhow::anyhow!("diffuse encode failed: {e}"))?;
    std::fs::write(paths.diffuse, encoded)?;

    let normal_size = lvl.normal_size.max(1) as usize;
    let normal = if normal_size == n { tile.normal_rgba.clone() } else { resample_normals(&tile.normal_rgba, n, normal_size) };
    directxtex_native::write_dds_rgba_image(
        paths.msn,
        normal_size as u32,
        normal_size as u32,
        &normal,
        format_to_dxgi(&lvl.normal_format),
        lvl.normal_mipmap,
    )
    .map_err(|e| anyhow::anyhow!("normal (_msn) encode failed: {e}"))?;

    directxtex_native::write_dds_rgba_image(paths.rmaos, tile.size, tile.size, &tile.rmaos_rgba, COMPANION_FORMAT, lvl.diffuse_mipmap)
        .map_err(|e| anyhow::anyhow!("_rmaos encode failed: {e}"))?;
    if let Some(glow) = &tile.glow_rgba {
        directxtex_native::write_dds_rgba_image(paths.glow, tile.size, tile.size, glow, COMPANION_FORMAT, lvl.diffuse_mipmap)
            .map_err(|e| anyhow::anyhow!("_g encode failed: {e}"))?;
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BTD stand-in: every sample of every cell carries the same selector word, palette and colour. Storage
    /// samples sit half a cell north-east of FO4 world (`world / 32 + 64`), so a level-4 quad at the origin
    /// reads cells 0..=4.
    #[derive(Clone, Copy)]
    struct Uniform {
        selector: u16,
        palette: [u8; 8],
        color: u16,
        cells: i32,
    }

    impl DenseCells for Uniform {
        fn grid(&self) -> DenseGrid {
            DenseGrid { cell_min: [0, 0], cell_max: [self.cells - 1, self.cells - 1], native_offset: [0.0, 0.0] }
        }

        fn load_cell(&mut self, _x: i32, _y: i32) -> anyhow::Result<CellData> {
            Ok(CellData {
                selectors: vec![self.selector; (CELL_SAMPLES * CELL_SAMPLES) as usize],
                palettes: [self.palette; 4],
                colors: vec![self.color; (COLOR_SAMPLES * COLOR_SAMPLES) as usize],
            })
        }
    }

    fn rgb555(v: u16) -> u16 {
        (v << 10) | (v << 5) | v
    }

    /// `v << 3 = 128` makes the overlay exactly 0.5, the soft-light identity.
    const NEUTRAL_COLOR: u16 = 16;

    fn material(albedo: [f32; 3], height: f32, softness: f32, gloss: f32, ao: f32, f0: f32) -> RawMaterial {
        RawMaterial {
            blend: [softness, 0.0, 1.0, 1.0 / 128.0],
            emission: [0.0; 4],
            exposure: [0.0; 4],
            maps: [
                MipChain::solid([albedo[0], albedo[1], albedo[2], height]),
                MipChain::solid([0.0, 0.0, 0.0, 1.0]),
                MipChain::solid([gloss, ao, 0.0, 0.0]),
                MipChain::solid([f0, f0, f0, 1.0]),
            ],
        }
    }

    fn quad(level: i32) -> QuadDesc {
        QuadDesc {
            z_order: 0,
            x: 0,
            y: 0,
            quad_level: level,
            quad_index: 0,
            quad_offset: 16384.0,
            static_indices: Vec::new(),
            statics: Vec::new(),
            out_values: crate::descriptors::OutDesc::default(),
        }
    }

    fn flat_block(level: usize) -> TerrainBlock {
        let size = crate::terrain::terrain_lod::block_size(level, 32);
        TerrainBlock { size, posts_per_cell: 32, heights: vec![0.0; size * size], water: vec![0.0; size * size] }
    }

    fn composite(source: &DenseTerrainSource, cells: Uniform, size: u32) -> DenseTile {
        composite_with(source, &quad(4), &flat_block(4), size, || Ok(cells)).unwrap()
    }

    #[test]
    fn one_terminal_layer_is_its_material_not_land() {
        let source = DenseTerrainSource::from_materials(vec![material([0.25, 0.5, 0.1], 1.0, 0.5, 0.5, 0.8, 0.04)]);
        let cells = Uniform { selector: 7, palette: [1, 0, 0, 0, 0, 0, 0, 0], color: rgb555(NEUTRAL_COLOR), cells: 5 };
        let tile = composite(&source, cells, 16);
        assert!(tile.uncovered.iter().all(|&u| !u), "every texel is dense");
        let expected = [0.25f32, 0.5, 0.1].map(|v| encode((v * 0.96).powf(1.0 / GAMMA)));
        for texel in tile.diffuse_rgba.chunks_exact(4) {
            assert_eq!(texel, [expected[0], expected[1], expected[2], 255]);
        }
        let roughness = 1.0 - (0.25 + 0.74 * 0.5);
        for texel in tile.rmaos_rgba.chunks_exact(4) {
            assert_eq!(texel, [encode(roughness), encode(metal_from_f0(0.04)), encode(0.8), encode(0.04)]);
        }
        for texel in tile.normal_rgba.chunks_exact(4) {
            assert_eq!(texel, [128, 255, 128, 255], "flat ground, flat `_n`: green-up");
        }
        assert!(tile.glow_rgba.is_none());
    }

    #[test]
    fn two_layer_blend_and_colour_overlay_follow_the_dense_shader() {
        // Palette [2, 1]: material 2 first with weight 4/7, material 1 terminal underneath.
        let (softness, height) = (0.5f32, 0.5f32);
        let top = material([0.6, 0.2, 0.1], height, softness, 0.9, 0.5, 0.3);
        let bottom = material([0.1, 0.3, 0.5], 1.0, 1.0, 0.1, 1.0, 0.04);
        let source = DenseTerrainSource::from_materials(vec![bottom, top]);
        let v5 = 24u16;
        let mut cells = Cells::new(Uniform { selector: 4 | (7 << 3), palette: [2, 1, 0, 0, 0, 0, 0, 0], color: rgb555(v5), cells: 1 });
        let shade = shade(&source, &mut cells, [40.25, 70.5], 32.0).unwrap();

        let coverage = (((softness + 1.0) * (4.0 / 7.0) + height - 1.0) / softness).clamp(0.0, 1.0);
        let mix = |a: f32, b: f32| coverage * a + (1.0 - coverage) * b;
        let f0 = mix(0.3, 0.04);
        let c = f32::from(v5 << 3) / 255.0;
        let e = ((c * 31.875 - 1.0) / 30.0).clamp(0.0, 1.0);
        let overlay = ((e / (1.0 - e)).log2() * 0.071543 + 0.5).clamp(0.0, 1.0);
        let soft_light = |albedo: f32| {
            let g = albedo.powf(1.0 / 2.2);
            let mixed = if overlay < 0.5 { 2.0 * g * overlay } else { 1.0 - 2.0 * (1.0 - g) * (1.0 - overlay) };
            mixed.powf(2.2) * (1.0 - f0)
        };
        let expected = [mix(0.6, 0.1), mix(0.2, 0.3), mix(0.1, 0.5)].map(soft_light);
        for c in 0..3 {
            assert!((shade.albedo[c] - expected[c]).abs() < 1e-5, "{:?} vs {expected:?}", shade.albedo);
        }
        assert!((shade.f0 - f0).abs() < 1e-6);
        assert!((shade.gloss - mix(0.9, 0.1)).abs() < 1e-6);
        assert!((shade.ao - mix(0.5, 1.0)).abs() < 1e-6);
        assert!(coverage > 0.0 && coverage < 1.0, "the case must exercise a partial layer");
    }

    #[test]
    fn uncovered_texels_take_the_land_composite() {
        let source = DenseTerrainSource::from_materials(vec![material([0.5; 3], 1.0, 0.5, 0.5, 1.0, 0.04)]);
        let cells = Uniform { selector: 0, palette: [0; 8], color: 0, cells: 4 };
        let tile = composite(&source, cells, 8);
        assert!(tile.uncovered.iter().all(|&u| u), "no palette entry, no dense coverage");

        let mut settings = LodSettings::fo4_default();
        let level = &mut settings.terrain.levels[0];
        level.diffuse_format = crate::settings::Format::Rgba8;
        level.diffuse_mipmap = false;
        level.normal_size = 8;
        let land = crate::terrain::textures::CompositeTile {
            width: 8,
            height: 8,
            normal_width: 8,
            normal_height: 8,
            diffuse_rgba: [55u8, 90, 20, 255].repeat(64),
            normal_rgba: [128u8, 255, 128, 255].repeat(64),
            warnings: Vec::new(),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = |name: &str| dir.path().join(name);
        let (diffuse, msn, rmaos, glow) = (path("t.dds"), path("t_msn.dds"), path("t_rmaos.dds"), path("t_g.dds"));
        let paths = DenseTilePaths { diffuse: &diffuse, msn: &msn, rmaos: &rmaos, glow: &glow };
        assert!(!write_dense_tile(&tile, Some(&land), &paths, &settings, 4).unwrap());
        assert!(write_dense_tile(&tile, None, &paths, &settings, 4).is_err(), "uncovered texels need LAND");
        let written = directxtex_native::read_dds_rgba_image(&diffuse).unwrap();
        let expected = [55u8, 90, 20].map(|v| encode((f32::from(v) / 255.0).powf(1.0 / GAMMA)));
        assert!(written.rgba.chunks_exact(4).all(|t| t[..3] == expected));
        assert!(rmaos.is_file() && msn.is_file() && !glow.exists());
    }

    #[test]
    fn missing_sidecar_means_land_and_land_source_means_no_dense() {
        let mut settings = LodSettings::fo4_default();
        assert!(DenseTerrainSource::load(&settings, &[]).unwrap().is_none(), "LAND source composites LAND");
        settings.terrain.source = TerrainSource::Btd;
        settings.terrain.btd_path = Some("w.btd".into());
        let error = DenseTerrainSource::load(&settings, &[]).err().unwrap();
        assert!(error.contains("no dense terrain sidecar"), "{error}");
        settings.terrain.btd4_path = Some("missing.btd4".into());
        assert!(DenseTerrainSource::load(&settings, &[]).is_err());
    }

    #[test]
    fn composite_is_deterministic() {
        let source = DenseTerrainSource::from_materials(vec![
            material([0.2, 0.4, 0.3], 0.7, 0.3, 0.6, 0.9, 0.04),
            material([0.7, 0.6, 0.5], 0.4, 0.8, 0.2, 0.7, 0.5),
        ]);
        let cells = Uniform { selector: 3 | (7 << 3), palette: [2, 1, 0, 0, 0, 0, 0, 0], color: rgb555(20), cells: 5 };
        let (a, b) = (composite(&source, cells, 32), composite(&source, cells, 32));
        assert_eq!(a.diffuse_rgba, b.diffuse_rgba);
        assert_eq!(a.normal_rgba, b.normal_rgba);
        assert_eq!(a.rmaos_rgba, b.rmaos_rgba);
    }

    #[test]
    fn bc4_reflectance_decodes_to_grey_f0() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.dds");
        directxtex_native::write_dds_rgba_image(&path, 256, 256, &[64u8, 0, 0, 255].repeat(256 * 256), "BC4_UNORM", true).unwrap();
        let chain = decode_mip_chain(&std::fs::read(&path).unwrap(), MapKind::Reflectance).unwrap();
        let texel = chain.sample([0.3, 0.7], 8.0);
        assert!((texel[0] - 64.0 / 255.0).abs() < 2.0 / 255.0 && texel[0] == texel[2], "{texel:?}");
        assert!(decode_mip_chain(&std::fs::read(&path).unwrap(), MapKind::Diffuse).is_err());
    }
}
