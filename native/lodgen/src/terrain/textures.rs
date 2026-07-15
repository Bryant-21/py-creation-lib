//! Per-quad terrain LOD texture compositing (diffuse + `_msn` normal).
//!
//! REPRODUCE node: no source code exists in any corpus for FO4 terrain texture
//! compositing — only the behavioral contract in
//! `tmp/lod_research/findings/R3_textures_settings_contract.md` §3 and the
//! xLODGen readme. The compositing math below is reproduced from that contract.
//! Places where the exact xLODGen blend is unknown are marked `APPROXIMATION`.
//!
//! Pipeline (R3 §3):
//!   1. Target res = `settings.terrain.levels[lod].diffuse_size` (256/512/1024/2048).
//!      The golden corpus confirms output size is governed by the *settings* size,
//!      not raw 64-px/cell native res (higher settings upscale).
//!   2. For each of the `level×level` cells in the quad, composite the cell's LTEX
//!      layers (base + alpha layers blended by per-layer alpha) into that cell's
//!      sub-region of the tile.
//!   3. Multiply by `Textures\Terrain\Noise.dds` (skip + warn if absent).
//!   4. Apply brightness / contrast / gamma to diffuse only.
//!   5. Overlay VCLR vertex colors at `vertex_color_intensity`.
//!   6. Build `_msn` from layer normals (Rise via `normal_rise`).
//!
//! Source textures and Noise.dds are loaded via `directxtex_native`; any miss is
//! handled with deterministic fallbacks so unit tests run without the FO4 install.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::descriptors::QuadDesc;
use crate::input::{CellInput, WorldspaceInput};
use crate::progress::LodPaths;
use crate::settings::{Format, LodSettings};

/// RGBA8 composite at the per-level target resolution, ready for BCn encode.
pub struct CompositeTile {
    pub width: u32,
    pub height: u32,
    /// `_msn` normal dimensions (may differ from diffuse for default/landless
    /// tiles, which use `default_normal_size`).
    pub normal_width: u32,
    pub normal_height: u32,
    pub diffuse_rgba: Vec<u8>,
    pub normal_rgba: Vec<u8>,
    /// Non-fatal notes (e.g. missing Noise.dds, missing source textures).
    pub warnings: Vec<String>,
}

/// Flat FO4 `_msn` (model-space) normal for level terrain, RGBA8. The FO4
/// terrain `_msn` convention encodes WORLD-UP in GREEN (not blue): a flat post
/// decodes to ~(128, 255, 128) — confirmed against the xLODGen golden corpus,
/// whose flat tiles decode to (126, 253, 126). Used where a cell has no usable
/// heightmap.
const FLAT_MSN_NORMAL: [u8; 4] = [128, 255, 128, 255];

/// Fallback diffuse color for an empty region / unresolved texture (mid-gray).
/// xLODGen uses the worldspace default land texture for empty regions; when even
/// that is unavailable we fall back to this neutral gray so the buffer is still
/// produced (spec §6 edge handling, R3 §3).
const FALLBACK_DIFFUSE: [u8; 4] = [128, 128, 128, 255];

fn lod_index(level: i32) -> usize {
    match level {
        4 => 0,
        8 => 1,
        16 => 2,
        32 => 3,
        // Defensive: clamp unexpected levels to the nearest in-range slot.
        l if l < 4 => 0,
        _ => 3,
    }
}

/// A source RGBA8 texture loaded from disk (or a synthesized 1x1 fallback).
#[derive(Clone)]
struct SourceTexture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl SourceTexture {
    fn solid(color: [u8; 4]) -> Self {
        SourceTexture {
            width: 1,
            height: 1,
            rgba: color.to_vec(),
        }
    }

    /// Nearest-neighbour sample in [0,1] UV. Used 1:1 AFTER the source has been
    /// box-downscaled to the cell's pixel size (see `downscaled_to`), so the
    /// source and the region it fills are the same resolution and there is
    /// nothing left to alias. (Sampling a full-res source here directly is what
    /// produced the olive/black grid noise.)
    fn sample(&self, u: f32, v: f32) -> [u8; 4] {
        let w = self.width.max(1);
        let h = self.height.max(1);
        let px = ((u.clamp(0.0, 1.0) * w as f32) as u32).min(w - 1);
        let py = ((v.clamp(0.0, 1.0) * h as f32) as u32).min(h - 1);
        let idx = ((py * w + px) * 4) as usize;
        [
            self.rgba[idx],
            self.rgba[idx + 1],
            self.rgba[idx + 2],
            self.rgba[idx + 3],
        ]
    }

    /// Box/area-average downscale to `target`×`target` px. Averaging over each
    /// output texel's source footprint is what kills the aliasing that
    /// nearest-neighbour point-sampling produced when a 512–1024px LTEX diffuse
    /// was crushed into a ~64px cell region (the olive/black grid noise). xLODGen
    /// Lanczos-resizes the source before compositing; a box average is the minimum
    /// faithful equivalent. Per-axis: if the source already fits within `target`
    /// that axis is left unchanged (upscaling is the 1:1 UV sampler's job). When
    /// both axes already fit, the texture is returned unchanged.
    fn downscaled_to(&self, target: u32) -> SourceTexture {
        let target = target.max(1);
        if self.width <= target && self.height <= target {
            return self.clone();
        }
        let sw = self.width as usize;
        let sh = self.height as usize;
        let tw = target.min(self.width) as usize;
        let th = target.min(self.height) as usize;
        let mut out = vec![0u8; tw * th * 4];
        for oy in 0..th {
            let sy0 = oy * sh / th;
            let sy1 = ((oy + 1) * sh / th).max(sy0 + 1).min(sh);
            for ox in 0..tw {
                let sx0 = ox * sw / tw;
                let sx1 = ((ox + 1) * sw / tw).max(sx0 + 1).min(sw);
                let mut acc = [0u32; 4];
                let mut count = 0u32;
                for sy in sy0..sy1 {
                    let row = sy * sw;
                    for sx in sx0..sx1 {
                        let i = (row + sx) * 4;
                        acc[0] += self.rgba[i] as u32;
                        acc[1] += self.rgba[i + 1] as u32;
                        acc[2] += self.rgba[i + 2] as u32;
                        acc[3] += self.rgba[i + 3] as u32;
                        count += 1;
                    }
                }
                let count = count.max(1);
                let oi = (oy * tw + ox) * 4;
                out[oi] = (acc[0] / count) as u8;
                out[oi + 1] = (acc[1] / count) as u8;
                out[oi + 2] = (acc[2] / count) as u8;
                out[oi + 3] = (acc[3] / count) as u8;
            }
        }
        SourceTexture {
            width: tw as u32,
            height: th as u32,
            rgba: out,
        }
    }
}

/// Resolve a Data-relative texture path against the search dirs and load it as
/// RGBA8. Backslash / forward-slash agnostic. Returns None on miss.
///
/// FO4 TXST diffuse/normal slots (TX00/TX01) are stored relative to
/// `Data\Textures\` — the `Textures\` component is stripped. So a landscape
/// path arrives here as e.g. `terrain/appalachia/foo_d.dds`, which lives on disk
/// at `<data_dir>/textures/terrain/appalachia/foo_d.dds`. We probe the raw path
/// first (so already-`textures/`-prefixed callers — Noise.dds, test fixtures —
/// keep matching) and then a `textures/`-prefixed variant.
/// Largest source-texture edge we will decode. A malformed/corrupt DDS whose
/// header claims absurd dimensions makes the DirectXTex C++ decoder allocate
/// gigabytes and ABORT the whole native process (no Rust panic to catch). The
/// cheap header probe rejects such files before the expensive/decoding read.
const MAX_SOURCE_DDS_EDGE: u32 = 16384;

/// Load + VALIDATE a single DDS file as RGBA8, or `None` if it is missing,
/// not-a-DDS, truncated, absurdly large, undecodable, or produced a buffer whose
/// length disagrees with its dimensions. A miss falls back to the grey base in
/// `composite_pass`, so a bad source DDS can never crash the run or feed garbage
/// to `SourceTexture::sample`.
fn load_valid_dds(path: &Path) -> Option<SourceTexture> {
    // Cheap header probe (≤148 bytes, pure-Rust, no decode) guards the C++
    // decoder from absurd / truncated headers that would OOM-abort the process.
    match directxtex_native::read_dds_probe(path) {
        Ok(p) => {
            if p.width == 0
                || p.height == 0
                || p.width > MAX_SOURCE_DDS_EDGE
                || p.height > MAX_SOURCE_DDS_EDGE
            {
                if std::env::var_os("LODGEN_TRACE_TEX").is_some() {
                    eprintln!(
                        "[tex] REJECT (bad dims {}x{}) {}",
                        p.width,
                        p.height,
                        path.display()
                    );
                }
                return None;
            }
        }
        Err(e) => {
            if std::env::var_os("LODGEN_TRACE_TEX").is_some() {
                eprintln!("[tex] REJECT (probe: {e}) {}", path.display());
            }
            return None;
        }
    }
    let img = directxtex_native::read_dds_rgba_image(path).ok()?;
    let expected = (img.width as usize)
        .checked_mul(img.height as usize)
        .and_then(|n| n.checked_mul(4));
    if img.width == 0 || img.height == 0 || expected != Some(img.rgba.len()) {
        if std::env::var_os("LODGEN_TRACE_TEX").is_some() {
            eprintln!(
                "[tex] REJECT (buffer {} != {}x{}*4) {}",
                img.rgba.len(),
                img.width,
                img.height,
                path.display()
            );
        }
        return None;
    }
    if std::env::var_os("LODGEN_TRACE_TEX").is_some() {
        eprintln!("[tex] OK {}x{} {}", img.width, img.height, path.display());
    }
    Some(SourceTexture {
        width: img.width,
        height: img.height,
        rgba: img.rgba,
    })
}

/// Decode + box-downscale memoization cache, keyed by `(resolved absolute path,
/// target square edge px)` → the decoded-and-resized texture (or `None` if that
/// file is missing/invalid). The compositor samples the same ~100 LTEX diffuse
/// textures across every cell of every quad; without this each access re-decoded
/// the DDS from disk (a full worldspace run exceeded 10 min). The expensive
/// decode+resize runs OUTSIDE the lock, so the rayon-parallel quad loop never
/// blocks all workers on a single decode; a racing duplicate decode is harmless
/// (idempotent) and far cheaper than holding a global lock across a decode. The
/// `Arc` lets readers clone the entry out cheaply.
type TexCache = HashMap<(PathBuf, u32), Option<Arc<SourceTexture>>>;

fn texture_cache() -> &'static Mutex<TexCache> {
    static CACHE: OnceLock<Mutex<TexCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Decode `path` and box-downscale it to `target`² px, memoized by `(path,
/// target)`. Returns `None` (cached) when the DDS is missing/invalid so callers
/// can fall through to the next candidate without re-probing it.
fn cached_decode_resize(path: PathBuf, target: u32) -> Option<Arc<SourceTexture>> {
    let key = (path, target);
    {
        let cache = texture_cache().lock().unwrap();
        if let Some(entry) = cache.get(&key) {
            return entry.clone();
        }
    }
    // Decode + resize OUTSIDE the lock (the expensive step).
    let result = load_valid_dds(&key.0).map(|t| Arc::new(t.downscaled_to(target)));
    let mut cache = texture_cache().lock().unwrap();
    // A racing thread may have inserted first; keep that entry (idempotent).
    cache.entry(key.clone()).or_insert_with(|| result.clone());
    cache.get(&key).cloned().flatten()
}

/// Resolve a Data-relative texture path against the search dirs, decode it, and
/// box-downscale to `target`² px — memoized per resolved path + target size.
/// Backslash/forward-slash agnostic; probes the raw path first then a
/// `textures/`-prefixed variant (FO4 TXST slots strip the `Textures\` component).
/// Falls through to the next candidate if a resolved file is missing/invalid.
fn load_texture_scaled(
    rel: &str,
    data_dirs: &[PathBuf],
    target: u32,
) -> Option<Arc<SourceTexture>> {
    if rel.trim().is_empty() {
        return None;
    }
    let norm = rel.replace('\\', "/");
    let mut candidates: Vec<PathBuf> = Vec::with_capacity(2);
    candidates.push(norm.split('/').collect());
    if !norm.to_ascii_lowercase().starts_with("textures/") {
        let prefixed = format!("textures/{norm}");
        candidates.push(prefixed.split('/').collect());
    }
    let target = target.max(1);
    for dir in data_dirs {
        for cand in &candidates {
            let candidate = dir.join(cand);
            if !candidate.is_file() {
                continue;
            }
            if let Some(tex) = cached_decode_resize(candidate, target) {
                return Some(tex);
            }
        }
    }
    None
}

/// Resolve a layer diffuse texture downscaled to the cell's pixel size:
/// explicit path -> worldspace default -> gray.
fn resolve_diffuse(
    rel: &str,
    world: &WorldspaceInput,
    data_dirs: &[PathBuf],
    target: u32,
) -> Arc<SourceTexture> {
    load_texture_scaled(rel, data_dirs, target)
        .or_else(|| load_texture_scaled(&world.default_diffuse, data_dirs, target))
        .unwrap_or_else(|| Arc::new(SourceTexture::solid(FALLBACK_DIFFUSE)))
}

/// FO4 per-quadrant ATXT alpha grid edge length (17x17 covers one cell quadrant).
const QUADRANT_ALPHA_EDGE: usize = 17;

/// Sample a layer's per-quadrant alpha at cell-local UV (0..1 over the whole
/// cell). The 17x17 quadrant alpha grid covers half the cell per axis, so the
/// quadrant id selects which half. APPROXIMATION: xLODGen blends the 5 ATXT
/// alpha layers per quadrant; we treat each `LayerTexture.alpha` as the opacity
/// field for its quadrant and sample it with clamping. Layers whose alpha vec is
/// not 17x17 are treated as uniformly opaque inside their quadrant only.
fn sample_layer_alpha(alpha: &[f32], quadrant: u8, cu: f32, cv: f32) -> f32 {
    // Map cell UV into the quadrant the layer owns.
    // Quadrant layout matches decode_hidden_quadrants: 0=SW,1=SE,2=NW,3=NE.
    let (in_u, in_v) = match quadrant {
        0 => (cu * 2.0, cv * 2.0),                 // SW
        1 => ((cu - 0.5) * 2.0, cv * 2.0),         // SE
        2 => (cu * 2.0, (cv - 0.5) * 2.0),         // NW
        3 => ((cu - 0.5) * 2.0, (cv - 0.5) * 2.0), // NE
        _ => (cu, cv),
    };
    if !(0.0..=1.0).contains(&in_u) || !(0.0..=1.0).contains(&in_v) {
        return 0.0; // outside this layer's quadrant
    }
    if alpha.len() != QUADRANT_ALPHA_EDGE * QUADRANT_ALPHA_EDGE {
        return 1.0;
    }

    let fx = in_u.clamp(0.0, 1.0) * (QUADRANT_ALPHA_EDGE - 1) as f32;
    let fy = in_v.clamp(0.0, 1.0) * (QUADRANT_ALPHA_EDGE - 1) as f32;
    let x0 = fx.floor() as usize;
    let y0 = fy.floor() as usize;
    let x1 = (x0 + 1).min(QUADRANT_ALPHA_EDGE - 1);
    let y1 = (y0 + 1).min(QUADRANT_ALPHA_EDGE - 1);
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;
    let get = |x: usize, y: usize| alpha[y * QUADRANT_ALPHA_EDGE + x].clamp(0.0, 1.0);
    let top = get(x0, y0) * (1.0 - tx) + get(x1, y0) * tx;
    let bot = get(x0, y1) * (1.0 - tx) + get(x1, y1) * tx;
    (top * (1.0 - ty) + bot * ty).clamp(0.0, 1.0)
}

fn blend(dst: &mut [u8; 4], src: [u8; 4], a: f32) {
    let a = a.clamp(0.0, 1.0);
    for c in 0..3 {
        dst[c] = (src[c] as f32 * a + dst[c] as f32 * (1.0 - a)).round() as u8;
    }
    dst[3] = 255;
}

/// `ModifyContrastBrightness(contrast, brightness)` + per-channel gamma (R3 §3).
/// brightness is additive in [-1,1]-ish units scaled to 0..255; contrast scales
/// around mid-gray. Matches the Imaging primitive shape used by the atlas path
/// (`wbLOD.pas:1664-1672`). APPROXIMATION: xLODGen's brightness is `b/10` in the
/// atlas path; here brightness/contrast/gamma come straight from settings and
/// the defaults (0.0 / 1.0 / 1.0) are identity.
fn apply_brightness_contrast_gamma(
    px: &mut [u8; 4],
    brightness: f32,
    contrast: f32,
    gamma: [f32; 3],
) {
    for c in 0..3 {
        let mut v = px[c] as f32 / 255.0;
        // contrast around 0.5
        v = (v - 0.5) * contrast + 0.5;
        // brightness additive
        v += brightness;
        v = v.clamp(0.0, 1.0);
        // per-channel gamma
        let g = gamma[c];
        if g > 0.0 && (g - 1.0).abs() > f32::EPSILON {
            v = v.powf(1.0 / g);
        }
        px[c] = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
}

/// Bilinear sample of the 33x33 per-post vertex-color grid at cell UV (0..1).
fn sample_vertex_color(colors: &[[u8; 3]], cu: f32, cv: f32) -> [f32; 3] {
    const N: usize = 33;
    if colors.len() != N * N {
        return [1.0, 1.0, 1.0];
    }
    let fx = cu.clamp(0.0, 1.0) * (N - 1) as f32;
    let fy = cv.clamp(0.0, 1.0) * (N - 1) as f32;
    let x0 = fx.floor() as usize;
    let y0 = fy.floor() as usize;
    let x1 = (x0 + 1).min(N - 1);
    let y1 = (y0 + 1).min(N - 1);
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;
    let get = |x: usize, y: usize| colors[y * N + x];
    let c00 = get(x0, y0);
    let c10 = get(x1, y0);
    let c01 = get(x0, y1);
    let c11 = get(x1, y1);
    let mut out = [0.0f32; 3];
    for c in 0..3 {
        let top = c00[c] as f32 * (1.0 - tx) + c10[c] as f32 * tx;
        let bot = c01[c] as f32 * (1.0 - tx) + c11[c] as f32 * tx;
        out[c] = (top * (1.0 - ty) + bot * ty) / 255.0;
    }
    out
}

/// 33x33 post grid edge and FO4 post spacing (4096 world units / 32 spans).
const POST_GRID: usize = 33;
const POST_SPACING: f32 = 4096.0 / 32.0;

/// Compute the FO4 `_msn` (model-space) normal for a pixel from the cell's 33x33
/// world-unit heightmap, at cell-local UV (cu = east 0..1, cv = north 0..1).
///
/// The surface normal of a heightfield is `n = normalize(-dz/dx, -dz/dy, 1)` in
/// world space (X=east, Y=north, Z=up). FO4's `_msn` encodes it green-up:
///   R = 0.5 + 0.5*n.x  (east-west)
///   G = 0.5 + 0.5*n.z  (UP — green-dominant)
///   B = 0.5 + 0.5*n.y  (north-south)
/// Channel mapping, sign and scale were derived empirically from the xLODGen
/// golden corpus (DLC03FarHarbor): flat tiles decode to ~(128,255,128); a
/// per-pixel reproduction from the corpus heightmap with this encoding
/// correlates +0.9 with the corpus `_msn` (R, B) — see the lodgen regression
/// test. Finite differences are clamped one-sided at the cell border.
fn heightmap_normal(heights: &[f32], cu: f32, cv: f32) -> [u8; 4] {
    if heights.len() != POST_GRID * POST_GRID {
        return FLAT_MSN_NORMAL;
    }
    let last = POST_GRID - 1;
    let col = (cu.clamp(0.0, 1.0) * last as f32).round() as usize;
    let row = (cv.clamp(0.0, 1.0) * last as f32).round() as usize;
    let cl = col.saturating_sub(1);
    let cr = (col + 1).min(last);
    let rd = row.saturating_sub(1);
    let ru = (row + 1).min(last);
    let h = |c: usize, r: usize| heights[c + r * POST_GRID];
    let dx_span = (cr - cl).max(1) as f32 * POST_SPACING;
    let dy_span = (ru - rd).max(1) as f32 * POST_SPACING;
    let dzdx = (h(cr, row) - h(cl, row)) / dx_span;
    let dzdy = (h(col, ru) - h(col, rd)) / dy_span;
    let (nx, ny, nz) = (-dzdx, -dzdy, 1.0);
    let len = (nx * nx + ny * ny + nz * nz).sqrt().max(f32::EPSILON);
    let enc = |v: f32| (((v / len) * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
    [enc(nx), enc(nz), enc(ny), 255]
}

/// Apply tangent-space "rise" steepness to a normal pixel: scales the XY
/// deviation from flat (R3 §3 "Rise steepness"). rise==1 is identity.
fn apply_normal_rise(px: &mut [u8; 4], rise: f32) {
    if (rise - 1.0).abs() <= f32::EPSILON {
        return;
    }
    for c in 0..2 {
        let centered = px[c] as f32 - 128.0;
        px[c] = (centered * rise + 128.0).clamp(0.0, 255.0).round() as u8;
    }
}

/// Composite the diffuse + `_msn` normal for one quad at the per-level target
/// resolution. Reproduced from R3 §3 (no source corpus for the exact blend).
pub fn composite_quad(
    world: &WorldspaceInput,
    quad: &QuadDesc,
    settings: &LodSettings,
    paths: &LodPaths,
) -> anyhow::Result<CompositeTile> {
    let level = quad.quad_level.max(1);
    let lod = lod_index(level);
    let normal_rise = settings.terrain.levels[lod].normal_rise;
    let warnings: Vec<String> = Vec::new();

    // Index cells by (x,y) for fast lookup; the quad covers
    // [quad.x .. quad.x+level) x [quad.y .. quad.y+level).
    let cell_at = |cx: i32, cy: i32| -> Option<&CellInput> {
        world.cells.iter().find(|c| c.x == cx && c.y == cy)
    };

    // Gap 5: default/landless tile sizing. A quad whose cells carry NO LTEX layer
    // (default-textured or landless) is emitted at default_diffuse_size /
    // default_normal_size (128) instead of the per-level 256. xLODGen sizes the
    // default-land tile by the worldspace default texture, which the golden corpus
    // shows as 128² for DLC03FarHarbor. When the default size is unset (None) the
    // per-level size is kept (opt-in override).
    let quad_has_layer = (0..level).any(|row| {
        (0..level).any(|col| {
            cell_at(quad.x + col, quad.y + row)
                .map(|c| !c.layers.is_empty())
                .unwrap_or(false)
        })
    });
    let diffuse_size = if !quad_has_layer {
        settings
            .terrain
            .default_diffuse_size
            .unwrap_or(settings.terrain.levels[lod].diffuse_size)
    } else {
        settings.terrain.levels[lod].diffuse_size
    }
    .max(1);
    let normal_size = if !quad_has_layer {
        settings
            .terrain
            .default_normal_size
            .unwrap_or(settings.terrain.levels[lod].normal_size)
    } else {
        settings.terrain.levels[lod].normal_size
    }
    .max(1);

    // --- diffuse pass (at diffuse_size) ---
    // The worldspace default land texture (used as the opaque base for cells with
    // no resolvable layer, R3 §3) is resolved per-cell inside composite_pass via
    // the decode+resize cache, so it is downscaled to the cell size like every
    // other layer and shared across cells/quads through the cache.
    let dw = diffuse_size as usize;
    let dh = diffuse_size as usize;
    let mut diffuse_rgba = vec![0u8; dw * dh * 4];
    composite_pass(
        world,
        quad,
        level,
        dw,
        dh,
        &paths.data_dirs,
        cell_at,
        Channel::Diffuse,
        normal_rise,
        &mut diffuse_rgba,
    );

    // Noise.dds multiply intentionally DISABLED: the prior full red-channel
    // multiply (tiled 4×) injected per-texel speckle and over-darkening that the
    // xLODGen golden corpus does not have (its terrain diffuse is smooth). The
    // exact xLODGen noise blend is unknown and near-identity at default
    // brightness (Noise.dds mean red ≈ 0.95), so omitting it matches the golden's
    // continuous, speckle-free surface.

    // brightness / contrast / gamma (diffuse only)
    let b = settings.terrain.brightness;
    let c = settings.terrain.contrast;
    let g = settings.terrain.gamma;
    if b != 0.0 || (c - 1.0).abs() > f32::EPSILON || g != [1.0, 1.0, 1.0] {
        for px in diffuse_rgba.chunks_exact_mut(4) {
            let mut p = [px[0], px[1], px[2], px[3]];
            apply_brightness_contrast_gamma(&mut p, b, c, g);
            px.copy_from_slice(&p);
        }
    }

    // VCLR vertex-color overlay
    let intensity = settings.terrain.vertex_color_intensity;
    if intensity != 0.0 {
        overlay_vertex_colors(
            world,
            quad,
            level,
            dw,
            dh,
            intensity,
            cell_at,
            &mut diffuse_rgba,
        );
    }

    // --- normal pass (at normal_size) ---
    let nw = normal_size as usize;
    let nh = normal_size as usize;
    let mut normal_rgba = vec![0u8; nw * nh * 4];
    composite_pass(
        world,
        quad,
        level,
        nw,
        nh,
        &paths.data_dirs,
        cell_at,
        Channel::Normal,
        normal_rise,
        &mut normal_rgba,
    );

    Ok(CompositeTile {
        width: diffuse_size,
        height: diffuse_size,
        normal_width: normal_size,
        normal_height: normal_size,
        diffuse_rgba,
        normal_rgba,
        warnings,
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Channel {
    Diffuse,
    Normal,
}

/// Fill one RGBA8 buffer (diffuse or normal) by compositing each cell's layers
/// into the cell's sub-region of the tile.
#[allow(clippy::too_many_arguments)]
fn composite_pass<'a, F>(
    world: &'a WorldspaceInput,
    quad: &QuadDesc,
    level: i32,
    width: usize,
    height: usize,
    data_dirs: &[PathBuf],
    cell_at: F,
    channel: Channel,
    normal_rise: f32,
    out: &mut [u8],
) where
    F: Fn(i32, i32) -> Option<&'a CellInput>,
{
    let level_usz = level as usize;
    // Each cell owns a (width/level) x (height/level) sub-block (integer math;
    // remainder rows/cols go to the last cell to avoid gaps).
    for cell_row in 0..level_usz {
        for cell_col in 0..level_usz {
            let cx = quad.x + cell_col as i32;
            let cy = quad.y + cell_row as i32;

            // Pixel bounds for this cell. Cell (0,0) is the SW corner of the
            // quad; image row 0 is the top (north). Flip cell_row -> image row
            // so north cells land at the top of the tile.
            let px0 = cell_col * width / level_usz;
            let px1 = (cell_col + 1) * width / level_usz;
            let img_row = level_usz - 1 - cell_row;
            let py0 = img_row * height / level_usz;
            let py1 = (img_row + 1) * height / level_usz;

            let cell = cell_at(cx, cy);

            // Box-downscale each source diffuse to the cell's pixel footprint
            // BEFORE compositing (xLODGen "resize source, then composite"): a
            // 512–1024px LTEX crushed into this ~64px region by point-sampling
            // aliased into olive/black grid noise. The cache keys on this edge so
            // the resize happens once per (texture, size) and is reused.
            let target_edge = (px1 - px0).max(py1 - py0) as u32;

            // Diffuse composites the cell's LTEX layers; `_msn` is derived from
            // the cell heightmap slopes (FO4 terrain `_msn` is the geometry
            // normal, green-up — the golden corpus bakes no source detail
            // normals: its flat tiles decode to a constant (126,253,126)).
            // Each layer remembers whether its diffuse genuinely RESOLVED on disk
            // (vs. a substituted fallback); the opaque full-cell base comes from
            // world default/fallback below, while LAND layers stay quadrant-scoped.
            let layers: Vec<(Arc<SourceTexture>, &crate::input::LayerTexture, bool)> =
                match (channel, cell) {
                    (Channel::Diffuse, Some(c)) => c
                        .layers
                        .iter()
                        .map(
                            |l| match load_texture_scaled(&l.diffuse, data_dirs, target_edge) {
                                Some(t) => (t, l, true),
                                None => (
                                    resolve_diffuse(&l.diffuse, world, data_dirs, target_edge),
                                    l,
                                    false,
                                ),
                            },
                        )
                        .collect(),
                    _ => Vec::new(),
                };

            // Opaque full-cell base (Diffuse only): worldspace default land
            // texture -> neutral grey. LAND layers are quadrant-scoped below,
            // including BTXT base layers with empty/non-17x17 alpha.
            let base: Option<Arc<SourceTexture>> = if channel == Channel::Diffuse {
                Some(
                    load_texture_scaled(&world.default_diffuse, data_dirs, target_edge)
                        .unwrap_or_else(|| Arc::new(SourceTexture::solid(FALLBACK_DIFFUSE))),
                )
            } else {
                None
            };
            let base_ref = base.as_deref();

            for py in py0..py1 {
                for px in px0..px1 {
                    // cell-local UV in [0,1]; v flipped so cell row 0 (south)
                    // maps to the bottom of the cell region.
                    let cu = if px1 > px0 {
                        (px - px0) as f32 / (px1 - px0) as f32
                    } else {
                        0.0
                    };
                    let cv_img = if py1 > py0 {
                        (py - py0) as f32 / (py1 - py0) as f32
                    } else {
                        0.0
                    };
                    let cv = 1.0 - cv_img; // north-up image -> south-up cell space

                    let acc = match channel {
                        Channel::Normal => {
                            let mut n = match cell {
                                Some(c) => heightmap_normal(&c.heights, cu, cv),
                                None => FLAT_MSN_NORMAL,
                            };
                            apply_normal_rise(&mut n, normal_rise);
                            n[3] = 255;
                            n
                        }
                        Channel::Diffuse => {
                            let mut acc = base_ref
                                .map(|b| b.sample(cu, cv))
                                .unwrap_or(FALLBACK_DIFFUSE);
                            // Composite the cell's layers over the opaque base.
                            for (tex, layer, _) in &layers {
                                let a = sample_layer_alpha(&layer.alpha, layer.quadrant, cu, cv);
                                if a <= 0.0 {
                                    continue;
                                }
                                blend(&mut acc, tex.sample(cu, cv), a);
                            }
                            acc
                        }
                    };

                    let idx = (py * width + px) * 4;
                    out[idx..idx + 4].copy_from_slice(&acc);
                }
            }
        }
    }
}

/// Overlay VCLR per-post vertex colors onto the diffuse at `intensity`
/// (multiplicative tint lerped toward the colored result by intensity).
fn overlay_vertex_colors<'a, F>(
    _world: &'a WorldspaceInput,
    quad: &QuadDesc,
    level: i32,
    width: usize,
    height: usize,
    intensity: f32,
    cell_at: F,
    diffuse: &mut [u8],
) where
    F: Fn(i32, i32) -> Option<&'a CellInput>,
{
    let level_usz = level as usize;
    for cell_row in 0..level_usz {
        for cell_col in 0..level_usz {
            let cx = quad.x + cell_col as i32;
            let cy = quad.y + cell_row as i32;
            let Some(cell) = cell_at(cx, cy) else {
                continue;
            };
            if cell.vertex_colors.len() != 33 * 33 {
                continue;
            }
            let px0 = cell_col * width / level_usz;
            let px1 = (cell_col + 1) * width / level_usz;
            let img_row = level_usz - 1 - cell_row;
            let py0 = img_row * height / level_usz;
            let py1 = (img_row + 1) * height / level_usz;

            for py in py0..py1 {
                for px in px0..px1 {
                    let cu = if px1 > px0 {
                        (px - px0) as f32 / (px1 - px0) as f32
                    } else {
                        0.0
                    };
                    let cv_img = if py1 > py0 {
                        (py - py0) as f32 / (py1 - py0) as f32
                    } else {
                        0.0
                    };
                    let cv = 1.0 - cv_img;
                    let vc = sample_vertex_color(&cell.vertex_colors, cu, cv);
                    let idx = (py * width + px) * 4;
                    for c in 0..3 {
                        let base = diffuse[idx + c] as f32 / 255.0;
                        // multiplicative tint; intensity lerps tint -> identity.
                        let tinted = base * vc[c];
                        let mixed = base * (1.0 - intensity) + tinted * intensity;
                        diffuse[idx + c] = (mixed.clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                }
            }
        }
    }
}

fn format_to_dxgi(format: &Format) -> &'static str {
    match format {
        Format::Bc1 => "BC1_UNORM",
        Format::Bc2 => "BC2_UNORM",
        Format::Bc3 => "BC3_UNORM",
        Format::Bc5 => "BC5_UNORM",
        Format::Bc7 => "BC7_UNORM",
        Format::Rgba8 => "R8G8B8A8_UNORM",
        // BGR565 has no directxtex_native string mapping here; fall back to BC1.
        Format::Bgr565 => "BC1_UNORM",
    }
}

/// Encode the composite to BCn DDS (with mips per settings) and write both the
/// diffuse and `_msn` tiles via `directxtex_native`.
pub fn write_tile_dds(
    tile: &CompositeTile,
    diffuse_path: &Path,
    msn_path: &Path,
    settings: &LodSettings,
    level: i32,
) -> anyhow::Result<()> {
    let lod = lod_index(level);
    let lvl = &settings.terrain.levels[lod];

    if let Some(parent) = diffuse_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = msn_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    directxtex_native::write_dds_rgba_image(
        diffuse_path,
        tile.width,
        tile.height,
        &tile.diffuse_rgba,
        format_to_dxgi(&lvl.diffuse_format),
        lvl.diffuse_mipmap,
    )
    .map_err(|e| anyhow::anyhow!("diffuse encode failed: {e}"))?;

    // Normal dims come from the tile (default/landless tiles use default_normal_size).
    directxtex_native::write_dds_rgba_image(
        msn_path,
        tile.normal_width.max(1),
        tile.normal_height.max(1),
        &tile.normal_rgba,
        format_to_dxgi(&lvl.normal_format),
        lvl.normal_mipmap,
    )
    .map_err(|e| anyhow::anyhow!("normal (_msn) encode failed: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::quads_for;
    use crate::settings::LodSettings;

    /// Regression guard for the olive/black grid-noise bug: box-downscaling a
    /// high-frequency source into a much smaller region must AVERAGE the footprint
    /// (near-uniform mid-tone, tiny variance), not point-sample it. The same
    /// source point-sampled at the small grid returns only extreme texels (the
    /// old aliasing behavior), proving the difference.
    #[test]
    fn downscale_averages_high_frequency_pattern() {
        // 64x64 black/white checkerboard — the worst-case high-frequency input.
        let n = 64u32;
        let mut rgba = vec![0u8; (n * n * 4) as usize];
        for y in 0..n as usize {
            for x in 0..n as usize {
                let v = if (x + y) % 2 == 0 { 255 } else { 0 };
                let i = (y * n as usize + x) * 4;
                rgba[i] = v;
                rgba[i + 1] = v;
                rgba[i + 2] = v;
                rgba[i + 3] = 255;
            }
        }
        let src = SourceTexture {
            width: n,
            height: n,
            rgba,
        };

        // Crush to 8x8: each output texel averages an 8x8 source block (32 black
        // + 32 white) -> ~127. Must be near mid-gray everywhere.
        let small = src.downscaled_to(8);
        assert_eq!(small.width, 8);
        assert_eq!(small.height, 8);
        for px in small.rgba.chunks_exact(4) {
            for &c in &px[..3] {
                assert!(
                    (c as i32 - 128).abs() <= 8,
                    "downscaled texel must be ~mid-gray, got {c}"
                );
            }
        }

        // Variance across the downscaled image must be tiny (uniform mid-tone),
        // proving the high-frequency pattern was averaged out, not point-sampled.
        let lum: Vec<f32> = small.rgba.chunks_exact(4).map(|p| p[0] as f32).collect();
        let mean = lum.iter().sum::<f32>() / lum.len() as f32;
        let var = lum.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / lum.len() as f32;
        assert!(
            var < 4.0,
            "downscaled checkerboard must be near-uniform, variance={var}"
        );

        // Contrast: NEAREST point-sampling the SAME source at the 8x8 grid (the
        // pre-fix behavior) returns only pure 0/255 texels — it never averages.
        let mut all_extreme = true;
        for oy in 0..8u32 {
            for ox in 0..8u32 {
                let s = src.sample(ox as f32 / 8.0, oy as f32 / 8.0)[0];
                if s > 1 && s < 254 {
                    all_extreme = false;
                }
            }
        }
        assert!(
            all_extreme,
            "sanity: nearest point-sampling returns extreme texels, never mid-tones"
        );
    }

    #[test]
    fn empty_alpha_base_layer_only_affects_its_quadrant() {
        let dir =
            std::env::temp_dir().join(format!("lodgen_tex_quadrant_base_{}", std::process::id()));
        let tex_dir = dir.join("textures").join("landscape");
        std::fs::create_dir_all(&tex_dir).unwrap();
        let red_path = tex_dir.join("red.dds");
        directxtex_native::write_dds_rgba_image(
            &red_path,
            1,
            1,
            &[255, 0, 0, 255],
            "R8G8B8A8_UNORM",
            false,
        )
        .unwrap();

        let cell = crate::input::CellInput {
            x: 0,
            y: 0,
            heights: vec![0.0; 33 * 33],
            vertex_colors: Vec::new(),
            layers: vec![crate::input::LayerTexture {
                diffuse: "textures/landscape/red.dds".into(),
                normal: String::new(),
                quadrant: 0,
                alpha: Vec::new(),
            }],
            hidden_quadrants: [false; 4],
            water_height: f32::MIN,
        };
        let w = crate::input::WorldspaceInput::from_cells("W", vec![cell]);
        let mut s = LodSettings::fo4_default();
        s.terrain.vertex_color_intensity = 0.0;
        let quad = quads_for(&w, 4, &s).into_iter().next().unwrap();
        let paths = crate::progress::LodPaths {
            data_dirs: vec![dir.clone()],
            output_dir: dir.clone(),
            source_data_dir: None,
        };
        let tile = composite_quad(&w, &quad, &s, &paths).unwrap();

        let pixel = |x: usize, y: usize| -> [u8; 4] {
            let i = (y * tile.width as usize + x) * 4;
            [
                tile.diffuse_rgba[i],
                tile.diffuse_rgba[i + 1],
                tile.diffuse_rgba[i + 2],
                tile.diffuse_rgba[i + 3],
            ]
        };

        // Cell (0,0) is the SW cell of the L4 tile. Its quadrant 0 is the
        // bottom-left quarter of the cell: x=0..31, y=224..255 in image space.
        assert_eq!(pixel(16, 240), [255, 0, 0, 255]);
        assert_eq!(pixel(48, 240), FALLBACK_DIFFUSE);
        assert_eq!(pixel(16, 208), FALLBACK_DIFFUSE);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quadrant_alpha_uses_bilinear_sampling() {
        let mut alpha = vec![0.0f32; QUADRANT_ALPHA_EDGE * QUADRANT_ALPHA_EDGE];
        for y in 0..QUADRANT_ALPHA_EDGE {
            alpha[y * QUADRANT_ALPHA_EDGE + 1] = 1.0;
        }

        let half_between_first_two_alpha_posts = 0.5 / (QUADRANT_ALPHA_EDGE - 1) as f32 / 2.0;
        let a = sample_layer_alpha(&alpha, 0, half_between_first_two_alpha_posts, 0.0);
        assert!(
            (a - 0.5).abs() <= 0.001,
            "bilinear alpha should interpolate a mid value, got {a}"
        );
    }

    fn one_layer_world() -> crate::input::WorldspaceInput {
        let cells = (0..4)
            .flat_map(|y| (0..4).map(move |x| (x, y)))
            .map(|(x, y)| crate::input::CellInput {
                x,
                y,
                heights: vec![0.0; 33 * 33],
                vertex_colors: vec![[255, 255, 255]; 33 * 33],
                layers: vec![crate::input::LayerTexture {
                    diffuse: "textures/landscape/dirt01.dds".into(),
                    normal: String::new(),
                    quadrant: 0,
                    alpha: vec![1.0; 17 * 17],
                }],
                hidden_quadrants: [false; 4],
                water_height: f32::MIN,
            })
            .collect();
        crate::input::WorldspaceInput::from_cells("W", cells)
    }

    #[test]
    fn composite_resolution_per_level() {
        let w = one_layer_world();
        let s = LodSettings::fo4_default();
        let quad = quads_for(&w, 4, &s)
            .into_iter()
            .find(|q| q.x == 0 && q.y == 0)
            .unwrap();
        let tile = composite_quad(&w, &quad, &s, &test_paths()).unwrap();
        // L4 native diffuse size = 256 (R3 appendix)
        assert_eq!(tile.width, 256);
        assert_eq!(tile.height, 256);
        assert_eq!(tile.diffuse_rgba.len(), 256 * 256 * 4);
    }

    /// Gap 5: a quad whose cells carry NO LTEX layers (default/landless) gets the
    /// default tile size (default_diffuse_size/default_normal_size = 128), not the
    /// per-level 256. Golden DLC03FarHarbor emits 128² for these default tiles.
    fn no_layer_world() -> crate::input::WorldspaceInput {
        let cells = (0..4)
            .flat_map(|y| (0..4).map(move |x| (x, y)))
            .map(|(x, y)| crate::input::CellInput {
                x,
                y,
                heights: vec![0.0; 33 * 33],
                vertex_colors: vec![[255, 255, 255]; 33 * 33],
                layers: Vec::new(), // NO LTEX layers
                hidden_quadrants: [false; 4],
                water_height: f32::MIN,
            })
            .collect();
        crate::input::WorldspaceInput::from_cells("W", cells)
    }

    #[test]
    fn default_cell_quad_uses_default_dds_size() {
        let w = no_layer_world();
        let s = LodSettings::fo4_default(); // default_*_size = Some(128)
        let quad = quads_for(&w, 4, &s)
            .into_iter()
            .find(|q| q.x == 0 && q.y == 0)
            .unwrap();
        let tile = composite_quad(&w, &quad, &s, &test_paths()).unwrap();
        assert_eq!(tile.width, 128, "default-cell diffuse must be 128²");
        assert_eq!(tile.height, 128);
        assert_eq!(tile.diffuse_rgba.len(), 128 * 128 * 4);
        // normal buffer also at default_normal_size
        assert_eq!(tile.normal_rgba.len(), 128 * 128 * 4);
    }

    /// A quad with at least one layered cell keeps the per-level size (256) — the
    /// default-size rule only applies when NO cell has a layer.
    #[test]
    fn partially_layered_quad_keeps_per_level_size() {
        // Build a 4x4 quad where only one cell has a layer.
        let mut cells: Vec<_> = (0..4)
            .flat_map(|y| (0..4).map(move |x| (x, y)))
            .map(|(x, y)| crate::input::CellInput {
                x,
                y,
                heights: vec![0.0; 33 * 33],
                vertex_colors: vec![[255, 255, 255]; 33 * 33],
                layers: Vec::new(),
                hidden_quadrants: [false; 4],
                water_height: f32::MIN,
            })
            .collect();
        cells[0].layers.push(crate::input::LayerTexture {
            diffuse: "textures/landscape/dirt01.dds".into(),
            normal: String::new(),
            quadrant: 0,
            alpha: vec![1.0; 17 * 17],
        });
        let w = crate::input::WorldspaceInput::from_cells("W", cells);
        let s = LodSettings::fo4_default();
        let quad = quads_for(&w, 4, &s)
            .into_iter()
            .find(|q| q.x == 0 && q.y == 0)
            .unwrap();
        let tile = composite_quad(&w, &quad, &s, &test_paths()).unwrap();
        assert_eq!(tile.width, 256, "layered quad keeps per-level 256");
    }

    /// When default_*_size is None, fall back to the per-level size even for a
    /// no-layer quad (the override is opt-in).
    #[test]
    fn no_default_size_falls_back_to_per_level() {
        let w = no_layer_world();
        let mut s = LodSettings::fo4_default();
        s.terrain.default_diffuse_size = None;
        s.terrain.default_normal_size = None;
        let quad = quads_for(&w, 4, &s)
            .into_iter()
            .find(|q| q.x == 0 && q.y == 0)
            .unwrap();
        let tile = composite_quad(&w, &quad, &s, &test_paths()).unwrap();
        assert_eq!(tile.width, 256, "None default size → per-level 256");
    }

    #[test]
    fn vertex_color_intensity_zero_leaves_base() {
        let w = one_layer_world();
        let mut s = LodSettings::fo4_default();
        s.terrain.vertex_color_intensity = 0.0; // no VCLR overlay
        let quad = quads_for(&w, 4, &s)
            .into_iter()
            .find(|q| q.x == 0 && q.y == 0)
            .unwrap();
        let tile = composite_quad(&w, &quad, &s, &test_paths()).unwrap();
        // alpha channel fully opaque
        assert_eq!(tile.diffuse_rgba[3], 255);
    }

    #[test]
    fn normal_buffer_sized_to_normal_size() {
        let w = one_layer_world();
        let s = LodSettings::fo4_default();
        let quad = quads_for(&w, 4, &s)
            .into_iter()
            .find(|q| q.x == 0 && q.y == 0)
            .unwrap();
        let tile = composite_quad(&w, &quad, &s, &test_paths()).unwrap();
        let n = s.terrain.levels[0].normal_size as usize;
        assert_eq!(tile.normal_rgba.len(), n * n * 4);
        // Flat cell heights -> flat FO4 `_msn` (model-space), which is GREEN-up:
        // R=128, G=255 (world up), B=128. (NOT the tangent-space blue-up (..,255).)
        assert_eq!(tile.normal_rgba[0], 128);
        assert_eq!(tile.normal_rgba[1], 255);
        assert_eq!(tile.normal_rgba[2], 128);
    }

    #[test]
    fn write_tile_dds_produces_bc1_msn() {
        let w = one_layer_world();
        let s = LodSettings::fo4_default();
        let quad = quads_for(&w, 4, &s)
            .into_iter()
            .find(|q| q.x == 0 && q.y == 0)
            .unwrap();
        let tile = composite_quad(&w, &quad, &s, &test_paths()).unwrap();

        let dir = std::env::temp_dir().join("lodgen_tex_write_test");
        std::fs::create_dir_all(&dir).unwrap();
        let diffuse = dir.join("W.4.0.0.dds");
        let msn = dir.join("W.4.0.0_msn.dds");
        write_tile_dds(&tile, &diffuse, &msn, &s, 4).unwrap();

        assert!(diffuse.is_file());
        assert!(msn.is_file());
        // _msn naming carried through
        assert!(msn.to_string_lossy().ends_with("_msn.dds"));

        // Validate the diffuse tile against the golden expectation:
        // 256x256, BC1_UNORM (DXT1). Mips depend on the diffuse_mipmap setting.
        let probe = directxtex_native::read_dds_probe(&diffuse).unwrap();
        assert_eq!(probe.width, 256);
        assert_eq!(probe.height, 256);
        // BC1_UNORM == dxgi 71 (golden corpus tiles are DXT1/BC1).
        assert_eq!(probe.dxgi_format, 71);

        let nprobe = directxtex_native::read_dds_probe(&msn).unwrap();
        assert_eq!(nprobe.width, s.terrain.levels[0].normal_size);
        assert_eq!(nprobe.dxgi_format, 71); // BC1 normal, matches golden _msn

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn test_paths() -> crate::progress::LodPaths {
        crate::progress::LodPaths {
            data_dirs: vec![std::path::PathBuf::from(".")],
            output_dir: std::env::temp_dir().join("lodgen_tex_test"),
            source_data_dir: None,
        }
    }
}
