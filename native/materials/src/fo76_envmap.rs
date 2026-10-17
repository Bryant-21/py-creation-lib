//! FO76 metal colour -> FO4 envmap cube.
//!
//! Vanilla FO4 colours metal only through the envmap cube (and `_d`):
//! reflection = cube(r, mip (1 - gloss) * 6) * 3 * `_s.R` * min(sqrt(`_s.G` - 0.3), 1)
//! * SpecularMult * EnvmapMaskScale. FO76 carries the metal colour in `_r` F0 instead, and FO4's
//! stock tinted cubes rarely match it, so a metal material gets a neutral stock cube rescaled
//! per channel (in linear light) to its mean metal F0 hue. Tints are quantised so materials
//! share cubes; the quantised colour is the file name, so the cube can be rebuilt from it.

use crate::error::{MaterialError, Result};
use directxtex_native::{
    DDS_FLAGS_NONE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8X8_UNORM,
    DXGI_FORMAT_R8G8B8A8_UNORM, ScratchImage, TEX_FILTER_FORCE_NON_WIC, TexMetadata,
};
use std::path::Path;

/// FO4's neutral brushed-metal cube: the shape every generated cube is tinted from.
pub const STOCK_METAL_CUBEMAP: &str = "Shared/Cubemaps/MetalBrushed01Cube_e.dds";
/// Generated cubes live here; materials never namespace and relocation never moves `Shared/Cubemaps`.
pub const GENERATED_CUBEMAP_DIR: &str = "Shared/Cubemaps/BACUP";
/// FO4CS ramps a texel from dielectric to metal across this F0 range.
const METAL_F0_LO: f32 = 0.16;
const METAL_F0_HI: f32 = 0.34;
/// Share of metal texels (weighted) above which a material samples a metal cube.
pub const METAL_MATERIAL_FRACTION: f32 = 0.02;
/// EnvmapMaskScale that makes a generated cube's mean strength match Bethesda's own
/// FO4 cube + `_s` on the shared FO76/FO4 sets (see the material guide).
pub const METAL_ENVMAP_MASK_SCALE: f32 = 0.65;
/// Hue spread (max - min of the luma-1 tint) below which a metal stays on the stock cube.
const NEUTRAL_SPREAD: f32 = 0.1;
/// The tint is named by the sRGB bytes of a 0.25 F0 metal of that hue, rounded to this step.
const NAME_F0: f32 = 0.25;
pub const TINT_QUANT_STEP: u8 = 16;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetalStats {
    /// Mean metal weight over all texels, 0..1.
    pub metal_fraction: f32,
    /// Linear RGB of luma 1: the metal texels' mean F0 hue.
    pub tint: [f32; 3],
}

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// `rgba` is linear F0 (as decoded from FO76 `_r`; a one-channel `_r` decodes splatted).
pub fn metal_stats_from_pixels(rgba: &[f32]) -> MetalStats {
    let (mut weight, mut sum, mut count) = (0f64, [0f64; 3], 0usize);
    for px in rgba.chunks_exact(4) {
        let w = smoothstep(METAL_F0_LO, METAL_F0_HI, px[0].max(px[1]).max(px[2])) as f64;
        weight += w;
        for c in 0..3 {
            sum[c] += w * px[c] as f64;
        }
        count += 1;
    }
    let mean = sum.map(|s| (s / weight.max(1e-9)) as f32);
    let l = luma(mean);
    MetalStats {
        metal_fraction: if count == 0 { 0.0 } else { (weight / count as f64) as f32 },
        tint: if l > 1e-6 { mean.map(|c| c / l) } else { [1.0; 3] },
    }
}

/// Reads the first mip no larger than 256 px, decoded to linear F0.
pub fn read_metal_stats(reflectivity_dds: &Path) -> Option<MetalStats> {
    let bytes = std::fs::read(reflectivity_dds).ok()?;
    let mut metadata = TexMetadata::default();
    let image = ScratchImage::load_dds(&bytes, DDS_FLAGS_NONE, Some(&mut metadata), None).ok()?;
    let mip = (0..metadata.mip_levels)
        .find(|&m| (metadata.width >> m).max(1) <= 256 && (metadata.height >> m).max(1) <= 256)
        .unwrap_or(metadata.mip_levels.saturating_sub(1));
    let level = image.image(mip, 0, 0)?;
    // DirectXTex's Convert rejects a same-format conversion, so RGBA8 is read as it is.
    let decoded = if metadata.format == DXGI_FORMAT_R8G8B8A8_UNORM {
        None
    } else if metadata.format.is_compressed() {
        Some(level.decompress(DXGI_FORMAT_R8G8B8A8_UNORM).ok()?)
    } else {
        Some(level.convert(DXGI_FORMAT_R8G8B8A8_UNORM, TEX_FILTER_FORCE_NON_WIC, 0.0).ok()?)
    };
    let out = match &decoded {
        Some(scratch) => scratch.image(0, 0, 0)?,
        None => level,
    };
    let mut rgba = Vec::with_capacity(out.width * out.height * 4);
    for row in 0..out.height {
        // SAFETY: DirectXTex owns `row_pitch * height` bytes for this image.
        let line = unsafe { std::slice::from_raw_parts(out.pixels.add(row * out.row_pitch), out.width * 4) };
        rgba.extend(line.iter().map(|&v| v as f32 / 255.0));
    }
    if metadata.format == directxtex_native::DXGI_FORMAT_BC4_UNORM {
        for px in rgba.chunks_exact_mut(4) {
            px[1] = px[0];
            px[2] = px[0];
        }
    }
    Some(metal_stats_from_pixels(&rgba))
}

fn quantised_name(tint: [f32; 3]) -> [u8; 3] {
    let step = TINT_QUANT_STEP as f32;
    tint.map(|t| {
        let v = linear_to_srgb(NAME_F0 * t) * 255.0;
        ((v / step).round() * step).clamp(0.0, 255.0) as u8
    })
}

fn tint_from_name(rgb: [u8; 3]) -> [f32; 3] {
    let lin = rgb.map(|v| srgb_to_linear(v as f32 / 255.0));
    let l = luma(lin);
    if l > 1e-6 { lin.map(|c| c / l) } else { [1.0; 3] }
}

/// The envmap (slot path, EnvmapMaskScale) for a metal material, or None for a dielectric one.
pub fn metal_envmap(stats: &MetalStats) -> Option<(String, f32)> {
    if stats.metal_fraction < METAL_MATERIAL_FRACTION {
        return None;
    }
    let (lo, hi) = stats.tint.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    if hi - lo < NEUTRAL_SPREAD {
        return Some((STOCK_METAL_CUBEMAP.to_owned(), METAL_ENVMAP_MASK_SCALE));
    }
    let [r, g, b] = quantised_name(stats.tint);
    if tint_spread(tint_from_name([r, g, b])) < NEUTRAL_SPREAD {
        return Some((STOCK_METAL_CUBEMAP.to_owned(), METAL_ENVMAP_MASK_SCALE));
    }
    Some((format!("{GENERATED_CUBEMAP_DIR}/Metal_{r:02x}{g:02x}{b:02x}_e.dds"), METAL_ENVMAP_MASK_SCALE))
}

fn tint_spread(t: [f32; 3]) -> f32 {
    t.iter().cloned().fold(f32::MIN, f32::max) - t.iter().cloned().fold(f32::MAX, f32::min)
}

/// The tint a generated cube slot names, or None when the slot is not a generated cube.
pub fn generated_cubemap_tint(slot: &str) -> Option<[f32; 3]> {
    let lower = slot.replace('\\', "/").to_ascii_lowercase();
    let lower = lower.trim_start_matches('/');
    let lower = lower.strip_prefix("textures/").unwrap_or(lower);
    let hex = lower
        .strip_prefix(&format!("{}/metal_", GENERATED_CUBEMAP_DIR.to_ascii_lowercase()))?
        .strip_suffix("_e.dds")?;
    if hex.len() != 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(tint_from_name([byte(0)?, byte(2)?, byte(4)?]))
}

/// The stock cube rescaled per channel in linear light by `tint`, every face and mip kept.
pub fn tinted_cube_bytes(stock_cube: &[u8], tint: [f32; 3]) -> Result<Vec<u8>> {
    let mut metadata = TexMetadata::default();
    let mut image = ScratchImage::load_dds(stock_cube, DDS_FLAGS_NONE, Some(&mut metadata), None)
        .map_err(|e| MaterialError::runtime(format!("load stock cube: {e}")))?;
    let bgra = match metadata.format {
        f if f == DXGI_FORMAT_B8G8R8A8_UNORM || f == DXGI_FORMAT_B8G8R8X8_UNORM => true,
        f if f == DXGI_FORMAT_R8G8B8A8_UNORM => false,
        other => return Err(MaterialError::invalid(format!("stock cube format {} is not 8-bit RGBA", other.bits()))),
    };
    if !metadata.is_cubemap() {
        return Err(MaterialError::invalid("stock cube is not a cubemap"));
    }
    let table: [[u8; 256]; 3] = std::array::from_fn(|c| {
        std::array::from_fn(|v| (linear_to_srgb(srgb_to_linear(v as f32 / 255.0) * tint[c]) * 255.0 + 0.5) as u8)
    });
    for px in image.pixels_mut().chunks_exact_mut(4) {
        let (r, b) = if bgra { (2, 0) } else { (0, 2) };
        px[r] = table[0][px[r] as usize];
        px[1] = table[1][px[1] as usize];
        px[b] = table[2][px[b] as usize];
    }
    let blob = image
        .save_dds(DDS_FLAGS_NONE)
        .map_err(|e| MaterialError::runtime(format!("save tinted cube: {e}")))?;
    Ok(blob.buffer().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(f0: [f32; 3], n: usize) -> Vec<f32> {
        [f0[0], f0[1], f0[2], 1.0].repeat(n)
    }

    #[test]
    fn dielectric_material_gets_no_metal_cube() {
        let stats = metal_stats_from_pixels(&px([0.04; 3], 64));
        assert_eq!(stats.metal_fraction, 0.0);
        assert!(metal_envmap(&stats).is_none());
    }

    #[test]
    fn grey_metal_uses_stock_cube_and_gold_gets_named_tint() {
        let grey = metal_stats_from_pixels(&px([0.3; 3], 64));
        assert_eq!(metal_envmap(&grey).unwrap().0, STOCK_METAL_CUBEMAP);

        let mut gold = px([0.6, 0.45, 0.15], 16);
        gold.extend(px([0.04; 3], 48));
        let stats = metal_stats_from_pixels(&gold);
        assert!((stats.metal_fraction - 0.25).abs() < 1e-4);
        let (slot, scale) = metal_envmap(&stats).unwrap();
        assert!(slot.starts_with("Shared/Cubemaps/BACUP/Metal_"), "{slot}");
        assert_eq!(scale, METAL_ENVMAP_MASK_SCALE);
        let tint = generated_cubemap_tint(&format!("Textures/{slot}")).unwrap();
        assert!((luma(tint) - 1.0).abs() < 1e-3);
        assert!(tint[0] > tint[1] && tint[1] > tint[2], "{tint:?}");
    }

    #[test]
    fn tinted_cube_keeps_layout_and_takes_the_hue() {
        let mut cube = ScratchImage::default();
        cube.initialize_cube(DXGI_FORMAT_B8G8R8A8_UNORM, 4, 4, 1, 3, directxtex_native::CP_FLAGS_NONE).unwrap();
        cube.pixels_mut().fill(128);
        let stock = cube.save_dds(DDS_FLAGS_NONE).unwrap().buffer().to_vec();
        let out = tinted_cube_bytes(&stock, [1.5, 1.0, 0.3]).unwrap();
        assert_eq!(out.len(), stock.len());
        let mut md = TexMetadata::default();
        let img = ScratchImage::load_dds(&out, DDS_FLAGS_NONE, Some(&mut md), None).unwrap();
        assert!(md.is_cubemap() && md.mip_levels == 3);
        let p = &img.pixels()[..4];
        // BGRA: red up, blue down, green unchanged.
        assert!(p[2] > 128 && p[1] == 128 && p[0] < 128, "{p:?}");
    }
}
