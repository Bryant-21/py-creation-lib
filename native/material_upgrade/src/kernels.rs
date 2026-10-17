//! Per-texel FO4 spec/gloss -> BGSM v22+ `_l` / `_r` formulas.
//!
//! Fitted on 180 shared FO4/reference texture sets whose `_d`/`_n` are
//! byte-identical in both games (so UVs match exactly). The reference upgrade
//! copies gloss into smoothness with a per-asset scale whose median is 1.0, bakes
//! the BGSM `Smoothness` scalar in about half the time, keeps F0 at 0.04 on
//! almost every texel and authors no material AO (`_d` already carries it).

/// Dielectric F0; the reference `_r` stores 0.039 in a 4x4 BC4.
pub const DIELECTRIC_F0: f32 = 0.04;
/// Median reference F0 on metal texels of shared assets (p10 0.21, p90 0.39), grey.
pub const METAL_F0: f32 = 0.25;
/// `_s` whose gloss varies less than this (std, 0..1) is treated as flat/generic.
pub const FLAT_GLOSS_STD: f32 = 0.03;
/// `_s.R` ramp used by the opt-in metal mask.
pub const METAL_MASK_LO: f32 = 0.6;
pub const METAL_MASK_HI: f32 = 0.8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SmoothnessCurve {
    pub scalar: f32,
    pub gain: f32,
    pub knee: Option<f32>,
}

impl SmoothnessCurve {
    pub fn apply(&self, gloss: f32) -> f32 {
        let mut value = gloss * self.scalar.clamp(0.0, 1.0) * self.gain;
        if let Some(knee) = self.knee {
            if value > knee {
                value = knee + 0.3 * (value - knee);
            }
        }
        value.clamp(0.0, 1.0)
    }
}

fn to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// True when the gloss channel (G) of an RGBA8 `_s` carries no detail.
pub fn gloss_is_flat(spec_rgba: &[u8]) -> bool {
    let count = spec_rgba.len() / 4;
    if count == 0 {
        return true;
    }
    let (mut sum, mut sum_sq) = (0f64, 0f64);
    for px in spec_rgba.chunks_exact(4) {
        let g = px[1] as f64 / 255.0;
        sum += g;
        sum_sq += g * g;
    }
    let mean = sum / count as f64;
    let var = (sum_sq / count as f64 - mean * mean).max(0.0);
    (var.sqrt() as f32) < FLAT_GLOSS_STD
}

/// `_l` RGBA8: R smoothness, G AO (1), B subsurface, A 255 (written as BC1, so the
/// alpha never reaches the shader as an emissive mask).
pub fn lighting_from_specgloss(spec_rgba: &[u8], curve: SmoothnessCurve, sss: f32) -> Vec<u8> {
    let sss = to_u8(sss);
    let mut out = Vec::with_capacity(spec_rgba.len());
    for px in spec_rgba.chunks_exact(4) {
        out.extend_from_slice(&[to_u8(curve.apply(px[1] as f32 / 255.0)), 255, sss, 255]);
    }
    out
}

pub fn constant_lighting(smoothness: f32, sss: f32, width: u32, height: u32) -> Vec<u8> {
    let px = [to_u8(smoothness), 255, to_u8(sss), 255];
    px.repeat((width * height) as usize)
}

pub fn constant_reflectance(f0: f32, width: u32, height: u32) -> Vec<u8> {
    let v = to_u8(f0);
    [v, v, v, 255].repeat((width * height) as usize)
}

/// Grey F0 from a metal mask in [0,1].
pub fn reflectance_from_mask(mask: impl Iterator<Item = f32>) -> Vec<u8> {
    let mut out = Vec::new();
    for m in mask {
        let v = to_u8(DIELECTRIC_F0 + (METAL_F0 - DIELECTRIC_F0) * m.clamp(0.0, 1.0));
        out.extend_from_slice(&[v, v, v, 255]);
    }
    out
}

/// Linear RGB tint of luminance 1. Neutral ([1,1,1]) means the metal stays grey.
pub type Tint = [f32; 3];
pub const NEUTRAL: Tint = [1.0, 1.0, 1.0];
/// Channel spread below which a cube counts as grey (keeps the one-channel BC4 `_r`);
/// wide enough to absorb the BC1 5:6:5 cast of a grey cube.
const NEUTRAL_SPREAD: f32 = 0.1;

fn srgb_to_linear(v: u8) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb_u8(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    to_u8(if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 })
}

/// FO4 colours metal only through its envmap cube; FO76/FO4CS take that colour from `_r` F0.
pub fn cube_tint(cube_rgba: &[u8]) -> Tint {
    let mut sum = [0f64; 3];
    for px in cube_rgba.chunks_exact(4) {
        for c in 0..3 {
            sum[c] += srgb_to_linear(px[c]) as f64;
        }
    }
    let luma = 0.2126 * sum[0] + 0.7152 * sum[1] + 0.0722 * sum[2];
    if luma <= 0.0 {
        return NEUTRAL;
    }
    let tint = sum.map(|s| (s / luma) as f32);
    let (lo, hi) = tint.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    if hi - lo < NEUTRAL_SPREAD { NEUTRAL } else { tint }
}

pub fn is_neutral(tint: Tint) -> bool {
    tint == NEUTRAL
}

/// sRGB-encoded bytes of the metal F0 colour, also used to name shared `_r` files.
pub fn tinted_metal_srgb(tint: Tint) -> [u8; 3] {
    tint.map(|t| linear_to_srgb_u8(METAL_F0 * t))
}

/// Coloured F0 (sRGB payload for a BC1_UNORM_SRGB `_r`) from a metal mask in [0,1].
pub fn tinted_reflectance_from_mask(mask: impl Iterator<Item = f32>, tint: Tint) -> Vec<u8> {
    let mut out = Vec::new();
    for m in mask {
        let m = m.clamp(0.0, 1.0);
        let px = tint.map(|t| linear_to_srgb_u8(DIELECTRIC_F0 + (METAL_F0 * t - DIELECTRIC_F0) * m));
        out.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    out
}

/// FO4 tints its envmap by the diffuse texel, so metal F0 takes the hue of cube tint × `_d`
/// (luminance kept at METAL_F0 × mask). `diffuse_rgba` is sRGB-encoded and matches the mask size.
pub fn blended_reflectance_from_mask(mask: impl Iterator<Item = f32>, tint: Tint, diffuse_rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (m, d) in mask.zip(diffuse_rgba.chunks_exact(4)) {
        let m = m.clamp(0.0, 1.0);
        let c = [0, 1, 2].map(|i| tint[i] * srgb_to_linear(d[i]));
        let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let hue = if luma > 1e-4 { c.map(|v| v / luma) } else { tint };
        let px = hue.map(|h| linear_to_srgb_u8(DIELECTRIC_F0 + (METAL_F0 * h - DIELECTRIC_F0) * m));
        out.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    out
}

/// Opt-in per-texel mask from FO4 specular intensity (`_s.R`).
pub fn spec_metal_mask(spec_rgba: &[u8]) -> impl Iterator<Item = f32> + '_ {
    spec_rgba
        .chunks_exact(4)
        .map(|px| smoothstep(METAL_MASK_LO, METAL_MASK_HI, px[0] as f32 / 255.0))
}

/// How much envmap FO4 shows per texel (vanilla BSLightingShader, 1.0 = pure envmap):
/// `3 * _s.R * min(sqrt(_s.G - 0.3), 1) * EnvironmentMappingMaskScale`.
pub fn envmap_metal_mask(spec_rgba: &[u8], mask_scale: f32) -> impl Iterator<Item = f32> + '_ {
    spec_rgba.chunks_exact(4).map(move |px| {
        let (spec, gloss) = (px[0] as f32 / 255.0, px[1] as f32 / 255.0);
        (3.0 * spec * (gloss - 0.3).clamp(0.0, 1.0).sqrt().min(1.0) * mask_scale).clamp(0.0, 1.0)
    })
}

/// FO76 `_d` is ~black where `_r` is metal (FO4CS scales diffuse by 1 - F0 and expects it);
/// FO4 painted the metal colour into `_d` too. Linear RGB × (1 - mask), alpha kept.
pub fn darken_metal_diffuse(diffuse_rgba: &[u8], mask: impl Iterator<Item = f32>) -> Vec<u8> {
    diffuse_rgba
        .chunks_exact(4)
        .zip(mask)
        .flat_map(|(px, m)| {
            let keep = 1.0 - m.clamp(0.0, 1.0);
            [0, 1, 2].map(|c| linear_to_srgb_u8(srgb_to_linear(px[c]) * keep)).into_iter().chain([px[3]])
        })
        .collect()
}

/// User-painted metal mask: R channel.
pub fn user_metal_mask(mask_rgba: &[u8]) -> impl Iterator<Item = f32> + '_ {
    mask_rgba.chunks_exact(4).map(|px| px[0] as f32 / 255.0)
}

/// Bilinear RGBA8 resize (used to fit layer textures to one bank size).
pub fn resize_rgba8(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    if sw == dw && sh == dh {
        return src.to_vec();
    }
    let (sw_us, sh_us) = (sw as usize, sh as usize);
    let mut out = vec![0u8; (dw * dh * 4) as usize];
    for y in 0..dh as usize {
        let fy = ((y as f32 + 0.5) * sh as f32 / dh as f32 - 0.5).max(0.0);
        let y0 = (fy as usize).min(sh_us - 1);
        let y1 = (y0 + 1).min(sh_us - 1);
        let ty = fy - y0 as f32;
        for x in 0..dw as usize {
            let fx = ((x as f32 + 0.5) * sw as f32 / dw as f32 - 0.5).max(0.0);
            let x0 = (fx as usize).min(sw_us - 1);
            let x1 = (x0 + 1).min(sw_us - 1);
            let tx = fx - x0 as f32;
            for c in 0..4 {
                let at = |xx: usize, yy: usize| src[(yy * sw_us + xx) * 4 + c] as f32;
                let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
                let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
                out[(y * dw as usize + x) * 4 + c] = (top * (1.0 - ty) + bottom * ty + 0.5) as u8;
            }
        }
    }
    out
}

/// Normal-map AO for `_l.G`. FO76's AO is near white with dark crevices that exist only in the
/// high-poly bake, i.e. in `_n` (the low-poly mesh barely predicts it). Fitted on 120 shared FO4/FO76
/// sets at full resolution: crevice = height minus its Gaussian blur (radius 6 px per 256 px of
/// texture), standardised per texture, darkened only on the concave side. Holdout MAE vs FO76
/// 0.036, against 0.049 for a flat AO of 1.
pub const CREVICE_SIGMA_AT_256: f32 = 6.0;
pub const CREVICE_STEPS: usize = 8;
pub const CREVICE_DARKEN: f32 = 0.096;

/// h - G_S * h == -integral_0^S G_s * div(slope) * s ds, evaluated as a midpoint sum of blurs.
pub fn crevice_scales(width: u32, height: u32) -> Vec<(f32, f32)> {
    let total = CREVICE_SIGMA_AT_256 * width.max(height) as f32 / 256.0;
    let step = total / CREVICE_STEPS as f32;
    (0..CREVICE_STEPS).map(|k| (k as f32 + 0.5) * step).map(|s| (s, s * step)).collect()
}

/// AO in [0, 1] from the blurred-divergence sum the GPU returns.
pub fn ao_from_crevice_sum(sum: &[f32]) -> Vec<f32> {
    let n = sum.len().max(1) as f64;
    let mean = sum.iter().map(|&v| f64::from(-v)).sum::<f64>() / n;
    let std = (sum.iter().map(|&v| (f64::from(-v) - mean).powi(2)).sum::<f64>() / n).sqrt().max(1e-9);
    sum.iter()
        .map(|&v| {
            let z = ((f64::from(-v) - mean) / std) as f32;
            (1.0 - CREVICE_DARKEN * (-z).max(0.0)).clamp(0.0, 1.0)
        })
        .collect()
}

pub fn normal_map_ao(normal_rgba: &[u8], width: u32, height: u32) -> Result<Vec<f32>, String> {
    let sum = directxtex_native::normal_crevice_gpu(normal_rgba, width, height, &crevice_scales(width, height))?;
    Ok(ao_from_crevice_sum(&sum))
}

/// Raw-terrain lighting layer: R smoothness, G AO 1, B 0, A 0 (no emission).
pub fn terrain_layer_lighting(spec_rgba: Option<&[u8]>, curve: SmoothnessCurve, flat_value: f32, pixels: usize) -> Vec<u8> {
    match spec_rgba {
        Some(spec) => spec
            .chunks_exact(4)
            .flat_map(|px| [to_u8(curve.apply(px[1] as f32 / 255.0)), 255, 0, 0])
            .collect(),
        None => [to_u8(flat_value), 255, 0, 0].repeat(pixels),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(r: u8, g: u8, n: usize) -> Vec<u8> {
        [r, g, 0, 255].repeat(n)
    }

    const PLAIN: SmoothnessCurve = SmoothnessCurve { scalar: 1.0, gain: 1.0, knee: None };

    #[test]
    fn lighting_bakes_gloss_times_scalar_with_ao_one_and_opaque_alpha() {
        let out = lighting_from_specgloss(&spec(50, 204, 4), SmoothnessCurve { scalar: 0.5, ..PLAIN }, 0.0);
        assert_eq!(out.len(), 16);
        assert_eq!(out[0], 102);
        assert_eq!(&out[1..4], &[255, 0, 255]);
    }

    #[test]
    fn gain_and_knee_shape_the_curve() {
        assert!((SmoothnessCurve { gain: 1.2, ..PLAIN }.apply(0.5) - 0.6).abs() < 1e-6);
        assert!((SmoothnessCurve { knee: Some(0.5), ..PLAIN }.apply(0.9) - 0.62).abs() < 1e-6);
        assert_eq!(SmoothnessCurve { gain: 4.0, ..PLAIN }.apply(0.9), 1.0);
        assert_eq!(SmoothnessCurve { scalar: 7.0, ..PLAIN }.apply(0.5), 0.5, "scalar is clamped to 1");
    }

    #[test]
    fn translucent_sets_subsurface_channel() {
        assert_eq!(lighting_from_specgloss(&spec(0, 80, 1), PLAIN, 1.0)[2], 255);
    }

    #[test]
    fn flat_detection() {
        assert!(gloss_is_flat(&spec(128, 230, 64)));
        let ramp: Vec<u8> = (0..64u32).flat_map(|i| [0, (i * 4) as u8, 0, 255]).collect();
        assert!(!gloss_is_flat(&ramp));
        assert!(gloss_is_flat(&[]));
    }

    #[test]
    fn metal_mask_ramps_on_spec_red_only() {
        let s: Vec<u8> = [77u8, 179, 242].iter().flat_map(|&r| [r, 128, 0, 255]).collect();
        let f0 = reflectance_from_mask(spec_metal_mask(&s));
        assert_eq!(f0[0], to_u8(DIELECTRIC_F0));
        assert!(f0[4] > f0[0] && f0[4] < to_u8(METAL_F0));
        assert_eq!(f0[8], to_u8(METAL_F0));
        assert_eq!(f0[1], f0[0]);
    }

    #[test]
    fn envmap_mask_follows_the_fo4_shader() {
        let m: Vec<f32> = envmap_metal_mask(&[0, 255, 0, 255, 255, 0, 0, 255, 128, 255, 0, 255, 128, 255, 0, 255], 1.0).collect();
        assert_eq!(&m[..2], &[0.0, 0.0], "no spec or no gloss shows no envmap");
        assert_eq!(m[2], 1.0, "mid spec at full gloss is already pure envmap");
        let half: Vec<f32> = envmap_metal_mask(&[40, 255, 0, 255], 1.0).collect();
        assert!((half[0] - 3.0 * 40.0 / 255.0 * 0.7f32.sqrt()).abs() < 1e-5);
        let scaled: Vec<f32> = envmap_metal_mask(&[40, 255, 0, 255], 0.5).collect();
        assert!((scaled[0] - half[0] * 0.5).abs() < 1e-5);
    }

    #[test]
    fn metal_texels_darken_diffuse_in_linear_light() {
        let d = [200u8, 100, 50, 128].repeat(3);
        let out = darken_metal_diffuse(&d, [0.0, 1.0, 0.5].into_iter());
        assert_eq!(&out[0..4], &[200, 100, 50, 128], "dielectric texels untouched");
        assert_eq!(&out[4..8], &[0, 0, 0, 128], "pure metal is black, alpha kept");
        assert_eq!(out[8], linear_to_srgb_u8(srgb_to_linear(200) * 0.5));
        assert!(out[8] > 100, "half in linear light is brighter than half the sRGB byte");
    }

    /// A tangent-space normal map of a bowl (concave dent) in the middle of a flat w x w tile.
    fn dent(w: u32) -> Vec<u8> {
        let c = w as f32 / 2.0;
        let r = w as f32 / 6.0;
        let mut out = Vec::new();
        for y in 0..w {
            for x in 0..w {
                let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
                let d2 = dx * dx + dy * dy;
                // h = d^2 - r^2 inside the bowl; the shader reads slope = -(stored n.xy)/nz.
                let (sx, sy) = if d2 < r * r { (2.0 * dx / r, 2.0 * dy / r) } else { (0.0, 0.0) };
                let len = (sx * sx + sy * sy + 1.0).sqrt();
                let (nx, ny) = (-sx / len, -sy / len);
                out.extend_from_slice(&[to_u8(nx * 0.5 + 0.5), to_u8(ny * 0.5 + 0.5), 255, 255]);
            }
        }
        out
    }

    /// Single-threaded reference of the GPU passes (divergence, blur x, blur y + accumulate).
    fn crevice_sum_cpu(rgba: &[u8], w: usize, h: usize, scales: &[(f32, f32)]) -> Vec<f32> {
        let slope = |x: usize, y: usize| {
            let px = &rgba[(y * w + x) * 4..];
            let nx = px[0] as f32 / 255.0 * 2.0 - 1.0;
            let ny = px[1] as f32 / 255.0 * 2.0 - 1.0;
            let nz = (1.0 - nx * nx - ny * ny).clamp(1e-4, 1.0).sqrt();
            (-nx / nz, -ny / nz)
        };
        let wrap = |v: i64, n: usize| v.rem_euclid(n as i64) as usize;
        let div: Vec<f32> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| {
                let dp = slope(wrap(x as i64 + 1, w), y).0 - slope(wrap(x as i64 - 1, w), y).0;
                let dq = slope(x, wrap(y as i64 + 1, h)).1 - slope(x, wrap(y as i64 - 1, h)).1;
                0.5 * (dp + dq)
            })
            .collect();
        let mut acc = vec![0f32; w * h];
        for &(sigma, weight) in scales {
            let radius = (sigma * 3.0 + 1.0) as i64;
            let g = |i: i64| (-0.5 * (i * i) as f32 / (sigma * sigma)).exp();
            let norm: f32 = (-radius..=radius).map(g).sum();
            let mut tmp = vec![0f32; w * h];
            for y in 0..h {
                for x in 0..w {
                    tmp[y * w + x] = (-radius..=radius).map(|i| g(i) * div[y * w + wrap(x as i64 + i, w)]).sum::<f32>() / norm;
                }
            }
            for y in 0..h {
                for x in 0..w {
                    acc[y * w + x] += weight * (-radius..=radius).map(|i| g(i) * tmp[wrap(y as i64 + i, h) * w + x]).sum::<f32>() / norm;
                }
            }
        }
        acc
    }

    #[test]
    fn gpu_crevice_matches_the_cpu_reference_and_darkens_the_dent() {
        let w = 64u32;
        let normals = dent(w);
        let scales = crevice_scales(w, w);
        let gpu = match directxtex_native::normal_crevice_gpu(&normals, w, w, &scales) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("skipped, no GPU: {err}");
                return;
            }
        };
        let cpu = crevice_sum_cpu(&normals, w as usize, w as usize, &scales);
        let peak = cpu.iter().fold(0f32, |m, v| m.max(v.abs()));
        let worst = gpu.iter().zip(&cpu).fold(0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(peak > 0.0 && worst < peak * 1e-3, "gpu vs cpu worst {worst}, peak {peak}");

        let ao = ao_from_crevice_sum(&gpu);
        let at = |x: u32, y: u32| ao[(y * w + x) as usize];
        assert_eq!(at(2, 2), 1.0, "flat surface away from the dent stays white");
        assert!(at(w / 2, w / 2) < 0.9, "the bottom of the dent darkens, got {}", at(w / 2, w / 2));
        assert!(ao.iter().all(|&v| (0.0..=1.0).contains(&v)));
    }

    #[test]
    fn flat_normals_give_white_ao() {
        assert!(ao_from_crevice_sum(&[0.0; 16]).iter().all(|&v| v == 1.0));
        let scales = crevice_scales(256, 128);
        assert_eq!(scales.len(), CREVICE_STEPS);
        let total: f32 = scales.iter().map(|s| s.1).sum();
        assert!((total - 18.0).abs() < 1e-3, "weights integrate s ds to S^2/2, got {total}");
    }

    #[test]
    fn blended_f0_takes_the_diffuse_hue_at_metal_brightness() {
        let gold_d = [200u8, 160, 60, 255];
        let grey_d = [128u8, 128, 128, 255];
        let diffuse = [gold_d, grey_d, gold_d].concat();
        let f0 = blended_reflectance_from_mask([1.0, 1.0, 0.0].into_iter(), NEUTRAL, &diffuse);
        assert!(f0[0] > f0[1] && f0[1] > f0[2], "grey cube + gold _d gives gold F0: {:?}", &f0[0..3]);
        assert_eq!(&f0[4..7], &[linear_to_srgb_u8(METAL_F0); 3]);
        assert_eq!(&f0[8..11], &[linear_to_srgb_u8(DIELECTRIC_F0); 3]);
        let warm = blended_reflectance_from_mask([1.0].into_iter(), [1.3, 0.95, 0.6], &gold_d);
        assert!(warm[0] - warm[2] > f0[0] - f0[2], "a warm cube pushes the gold further");
    }

    #[test]
    fn cube_tint_keeps_hue_and_greys_stay_grey() {
        let copper = cube_tint(&[89u8, 37, 8, 255].repeat(16));
        assert!(copper[0] > 2.0 && copper[2] < 0.2, "{copper:?}");
        let luma = 0.2126 * copper[0] + 0.7152 * copper[1] + 0.0722 * copper[2];
        assert!((luma - 1.0).abs() < 1e-3);
        assert!(is_neutral(cube_tint(&[93u8, 93, 93, 255].repeat(16))));
        assert!(is_neutral(cube_tint(&[0u8, 0, 0, 255].repeat(4))));
        // a fully metal texel carries the tinted F0, a bare one stays dielectric
        let px = tinted_reflectance_from_mask([1.0, 0.0].into_iter(), copper);
        assert_eq!(&px[0..3], &tinted_metal_srgb(copper));
        assert!(px[0] > px[1] && px[1] > px[2]);
        assert_eq!(&px[4..7], &[linear_to_srgb_u8(DIELECTRIC_F0); 3]);
        assert_eq!(tinted_metal_srgb(NEUTRAL), [linear_to_srgb_u8(METAL_F0); 3]);
    }

    #[test]
    fn constants_and_resize() {
        assert_eq!(constant_reflectance(DIELECTRIC_F0, 4, 4), [10, 10, 10, 255].repeat(16));
        assert_eq!(constant_lighting(0.4, 0.0, 2, 1), vec![102, 255, 0, 255, 102, 255, 0, 255]);
        let src = [0u8, 0, 0, 255, 255, 255, 255, 255].repeat(2);
        let out = resize_rgba8(&src, 2, 2, 4, 4);
        assert_eq!(out.len(), 64);
        assert_eq!(out[0], 0);
        assert_eq!(out[3 * 4], 255);
    }

    #[test]
    fn terrain_lighting_has_no_emission() {
        let l = terrain_layer_lighting(Some(&spec(0, 255, 2)), PLAIN, 0.2, 2);
        assert_eq!(l, vec![255, 255, 0, 0, 255, 255, 0, 0]);
        assert_eq!(terrain_layer_lighting(None, PLAIN, 0.2, 1), vec![51, 255, 0, 0]);
    }
}
