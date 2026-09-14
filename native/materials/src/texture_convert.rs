use crate::error::{MaterialError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub struct TextureConversionParams {
    pub ao_multiplier: f32,
    pub specular_multiplier: f32,
    pub gloss_multiplier: f32,
    pub spec_offset: f32,
    pub preserve_lighting_rgb_for_glow: bool,
}

pub struct Fo76BundleOutputs {
    pub diffuse: Vec<f32>,
    pub specgloss: Vec<f32>,
    pub glow: Option<Vec<f32>>,
}

pub struct StarfieldPbrOutputs {
    pub diffuse: Vec<f32>,
    pub specgloss: Vec<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TexturePathInput {
    pub role: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TexturePathOutput {
    pub role: String,
    pub path: PathBuf,
    pub format: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TextureSetPathRequest {
    pub source_game: String,
    pub target_game: String,
    pub inputs: Vec<TexturePathInput>,
    pub outputs: Vec<TexturePathOutput>,
    #[serde(default)]
    pub params: TextureConversionParamsPayload,
    #[serde(default = "default_use_gpu")]
    pub use_gpu: bool,
    #[serde(default)]
    pub gpu_min_pixels: u32,
    /// Whether each image is compressed with DirectXTex's internal thread fan
    /// (TEX_COMPRESS_PARALLEL). True for standalone single-image callers; the
    /// `convert_textures` phase sets this false because it already parallelizes
    /// across texture groups via its worker pool — nesting the two oversubscribes
    /// the CPU (16 workers x hardware_concurrency threads).
    #[serde(default = "default_parallel_compression")]
    pub parallel_compression: bool,
}

fn default_use_gpu() -> bool {
    true
}

fn default_parallel_compression() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct TextureConversionParamsPayload {
    #[serde(default = "default_ao_multiplier")]
    pub ao_multiplier: f32,
    #[serde(default = "default_specular_multiplier")]
    pub specular_multiplier: f32,
    #[serde(default = "default_gloss_multiplier")]
    pub gloss_multiplier: f32,
    #[serde(default = "default_spec_offset")]
    pub spec_offset: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextureSetPathResult {
    pub converted: Vec<TexturePathResultItem>,
    pub skipped: Vec<TexturePathSkipItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TexturePathResultItem {
    pub role: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TexturePathSkipItem {
    pub role: String,
    pub reason: String,
}

fn default_ao_multiplier() -> f32 {
    0.5
}

fn default_specular_multiplier() -> f32 {
    1.0
}

fn default_gloss_multiplier() -> f32 {
    1.0
}

fn default_spec_offset() -> f32 {
    0.8
}

impl Default for TextureConversionParams {
    fn default() -> Self {
        Self {
            ao_multiplier: 0.5,
            specular_multiplier: 1.0,
            gloss_multiplier: 1.0,
            spec_offset: 0.8,
            preserve_lighting_rgb_for_glow: false,
        }
    }
}

impl Default for TextureConversionParamsPayload {
    fn default() -> Self {
        Self {
            ao_multiplier: default_ao_multiplier(),
            specular_multiplier: default_specular_multiplier(),
            gloss_multiplier: default_gloss_multiplier(),
            spec_offset: default_spec_offset(),
        }
    }
}

impl From<TextureConversionParamsPayload> for TextureConversionParams {
    fn from(value: TextureConversionParamsPayload) -> Self {
        Self {
            ao_multiplier: value.ao_multiplier,
            specular_multiplier: value.specular_multiplier,
            gloss_multiplier: value.gloss_multiplier,
            spec_offset: value.spec_offset,
            preserve_lighting_rgb_for_glow: false,
        }
    }
}

pub fn fo76_bundle_to_fo4_buffers(
    diffuse_bytes: &[u8],
    reflectivity_bytes: &[u8],
    lighting_bytes: &[u8],
    width: usize,
    height: usize,
    reflectivity_width: usize,
    reflectivity_height: usize,
    lighting_width: usize,
    lighting_height: usize,
    params: TextureConversionParams,
    emit_lighting_alpha_glow: bool,
) -> Result<Fo76BundleOutputs> {
    let _timer = directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    check_params(params)?;
    let diffuse = read_rgba_f32_bytes(diffuse_bytes, width, height, "diffuse")?;
    let reflectivity = read_rgba_f32_bytes(
        reflectivity_bytes,
        reflectivity_width,
        reflectivity_height,
        "reflectivity",
    )?;
    let lighting =
        read_rgba_f32_bytes(lighting_bytes, lighting_width, lighting_height, "lighting")?;
    fo76_bundle_to_fo4_pixels(
        &diffuse,
        &reflectivity,
        &lighting,
        width,
        height,
        reflectivity_width,
        reflectivity_height,
        lighting_width,
        lighting_height,
        params,
        emit_lighting_alpha_glow,
    )
}

pub fn fo76_bundle_to_fo4_pixels(
    diffuse_pixels: &[f32],
    reflectivity_pixels: &[f32],
    lighting_pixels: &[f32],
    width: usize,
    height: usize,
    reflectivity_width: usize,
    reflectivity_height: usize,
    lighting_width: usize,
    lighting_height: usize,
    params: TextureConversionParams,
    emit_lighting_alpha_glow: bool,
) -> Result<Fo76BundleOutputs> {
    let _timer =
        directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    check_params(params)?;
    let diffuse = checked_rgba_pixels(diffuse_pixels, width, height, "diffuse")?;
    let reflectivity_raw = checked_rgba_pixels(
        reflectivity_pixels,
        reflectivity_width,
        reflectivity_height,
        "reflectivity",
    )?;
    let lighting_raw =
        checked_rgba_pixels(lighting_pixels, lighting_width, lighting_height, "lighting")?;
    let reflectivity = resize_rgba_bilinear(
        &reflectivity_raw,
        reflectivity_width,
        reflectivity_height,
        width,
        height,
    )?;
    let lighting = resize_rgba_bilinear(
        &lighting_raw,
        lighting_width,
        lighting_height,
        width,
        height,
    )?;

    let mut diffuse_out = diffuse.to_vec();
    let mut specgloss = vec![0.0; diffuse.len()];
    let mut glow = emit_lighting_alpha_glow.then(|| vec![0.0; diffuse.len()]);
    let threshold = 1.0 - params.spec_offset;
    let denom = params.spec_offset.max(1e-6);

    for idx in 0..pixel_count(width, height)? {
        let i = idx * 4;
        let d0 = diffuse[i].clamp(0.0, 1.0);
        let d1 = diffuse[i + 1].clamp(0.0, 1.0);
        let d2 = diffuse[i + 2].clamp(0.0, 1.0);
        let r0 = reflectivity[i].clamp(0.0, 1.0);
        let r1 = reflectivity[i + 1].clamp(0.0, 1.0);
        let r2 = reflectivity[i + 2].clamp(0.0, 1.0);
        let gloss = (lighting[i].clamp(0.0, 1.0) * params.gloss_multiplier).clamp(0.0, 1.0);
        let ao = lighting[i + 1].clamp(0.0, 1.0);
        let ao_term = (1.0 - params.ao_multiplier) + ao * params.ao_multiplier;

        let [metal0, metal1, metal2] =
            metal_contribution_preserving_hue(r0, r1, r2, threshold, denom);

        let base0 = d0 + metal0;
        let base1 = d1 + metal1;
        let base2 = d2 + metal2;

        diffuse_out[i] = (base0 * ao_term).clamp(0.0, 1.0);
        diffuse_out[i + 1] = (base1 * ao_term).clamp(0.0, 1.0);
        diffuse_out[i + 2] = (base2 * ao_term).clamp(0.0, 1.0);
        diffuse_out[i + 3] = diffuse[i + 3].clamp(0.0, 1.0);

        let spec0 = 0.22 * (1.0 - metal0) + base0 * metal0;
        let spec1 = 0.22 * (1.0 - metal1) + base1 * metal1;
        let spec2 = 0.22 * (1.0 - metal2) + base2 * metal2;
        let specular =
            (((spec0 + spec1 + spec2) / 3.0) * params.specular_multiplier).clamp(0.0, 1.0);

        specgloss[i] = specular;
        specgloss[i + 1] = gloss;
        specgloss[i + 2] = 0.0;
        specgloss[i + 3] = 1.0;

        if let Some(glow_values) = glow.as_mut() {
            // FO76 stores the emissive mask in the `_l` alpha channel; the `_l`
            // RGB normally holds packed material data. Explicitly glow-named
            // textures are authored with colour there, matching FO4's coloured
            // `_g` convention, so preserve it only for that named exception.
            let emissive_mask = lighting[i + 3].clamp(0.0, 1.0);
            if params.preserve_lighting_rgb_for_glow {
                glow_values[i] = lighting[i].clamp(0.0, 1.0) * emissive_mask;
                glow_values[i + 1] = lighting[i + 1].clamp(0.0, 1.0) * emissive_mask;
                glow_values[i + 2] = lighting[i + 2].clamp(0.0, 1.0) * emissive_mask;
            } else {
                glow_values[i] = emissive_mask;
                glow_values[i + 1] = emissive_mask;
                glow_values[i + 2] = emissive_mask;
            }
            glow_values[i + 3] = 1.0;
        }
    }

    Ok(Fo76BundleOutputs {
        diffuse: diffuse_out,
        specgloss,
        glow,
    })
}

/// FO76 packs the emissive mask in `_l` alpha while its RGB holds gloss/AO. A
/// `_l` with no `_d`/`_r` siblings never reaches `fo76_bundle_to_fo4_buffers`,
/// but the BGSM downgrade still promotes it into FO4's glow slot, so it needs
/// the same alpha-derived mask the bundle builds.
pub fn fo76_lighting_to_fo4_glow_buffer(
    lighting_bytes: &[u8],
    width: usize,
    height: usize,
    preserve_lighting_rgb: bool,
) -> Result<Vec<f32>> {
    let _timer = directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    let lighting = read_rgba_f32_bytes(lighting_bytes, width, height, "lighting")?;
    fo76_lighting_to_fo4_glow_pixels(&lighting, width, height, preserve_lighting_rgb)
}

pub fn fo76_lighting_to_fo4_glow_pixels(
    lighting_pixels: &[f32],
    width: usize,
    height: usize,
    preserve_lighting_rgb: bool,
) -> Result<Vec<f32>> {
    let _timer =
        directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    let lighting = checked_rgba_pixels(lighting_pixels, width, height, "lighting")?;
    let mut glow = vec![0.0; lighting.len()];
    for idx in 0..pixel_count(width, height)? {
        let i = idx * 4;
        let emissive_mask = lighting[i + 3].clamp(0.0, 1.0);
        if preserve_lighting_rgb {
            glow[i] = lighting[i].clamp(0.0, 1.0) * emissive_mask;
            glow[i + 1] = lighting[i + 1].clamp(0.0, 1.0) * emissive_mask;
            glow[i + 2] = lighting[i + 2].clamp(0.0, 1.0) * emissive_mask;
        } else {
            glow[i] = emissive_mask;
            glow[i + 1] = emissive_mask;
            glow[i + 2] = emissive_mask;
        }
        glow[i + 3] = 1.0;
    }
    Ok(glow)
}

#[allow(clippy::too_many_arguments)]
pub fn starfield_pbr_to_fo4_buffers(
    albedo_bytes: &[u8],
    metallic_bytes: &[u8],
    roughness_bytes: &[u8],
    ao_bytes: Option<&[u8]>,
    width: usize,
    height: usize,
    metallic_width: usize,
    metallic_height: usize,
    roughness_width: usize,
    roughness_height: usize,
    ao_width: usize,
    ao_height: usize,
    params: TextureConversionParams,
) -> Result<StarfieldPbrOutputs> {
    let _timer = directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    check_params(params)?;
    let albedo = read_rgba_f32_bytes(albedo_bytes, width, height, "albedo")?;
    let metallic =
        read_rgba_f32_bytes(metallic_bytes, metallic_width, metallic_height, "metallic")?;
    let roughness = read_rgba_f32_bytes(
        roughness_bytes,
        roughness_width,
        roughness_height,
        "roughness",
    )?;
    let ao = ao_bytes
        .map(|bytes| read_rgba_f32_bytes(bytes, ao_width, ao_height, "ao"))
        .transpose()?;
    starfield_pbr_to_fo4_pixels(
        &albedo,
        &metallic,
        &roughness,
        ao.as_deref(),
        width,
        height,
        metallic_width,
        metallic_height,
        roughness_width,
        roughness_height,
        ao_width,
        ao_height,
        params,
    )
}

pub fn starfield_pbr_to_fo4_pixels(
    albedo_pixels: &[f32],
    metallic_pixels: &[f32],
    roughness_pixels: &[f32],
    ao_pixels: Option<&[f32]>,
    width: usize,
    height: usize,
    metallic_width: usize,
    metallic_height: usize,
    roughness_width: usize,
    roughness_height: usize,
    ao_width: usize,
    ao_height: usize,
    params: TextureConversionParams,
) -> Result<StarfieldPbrOutputs> {
    let _timer =
        directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    check_params(params)?;
    let albedo = checked_rgba_pixels(albedo_pixels, width, height, "albedo")?;
    let metallic = resize_rgba_bilinear(
        &checked_rgba_pixels(metallic_pixels, metallic_width, metallic_height, "metallic")?,
        metallic_width,
        metallic_height,
        width,
        height,
    )?;
    let roughness = resize_rgba_bilinear(
        &checked_rgba_pixels(
            roughness_pixels,
            roughness_width,
            roughness_height,
            "roughness",
        )?,
        roughness_width,
        roughness_height,
        width,
        height,
    )?;
    let ao = match ao_pixels {
        Some(bytes) => Some(resize_rgba_bilinear(
            &checked_rgba_pixels(bytes, ao_width, ao_height, "ao")?,
            ao_width,
            ao_height,
            width,
            height,
        )?),
        None => None,
    };

    let albedo_rgb: Vec<f32> = albedo
        .chunks_exact(4)
        .flat_map(|pixel| pixel[..3].iter().copied())
        .collect();
    let metallic_values: Vec<f32> = metallic.chunks_exact(4).map(|pixel| pixel[0]).collect();
    let roughness_values: Vec<f32> = roughness.chunks_exact(4).map(|pixel| pixel[0]).collect();
    let ao_values: Option<Vec<f32>> = ao
        .as_ref()
        .map(|values| values.chunks_exact(4).map(|pixel| pixel[0]).collect());
    let pixel_count = pixel_count(width, height)?;
    let converted = crate::pbr::convert_pixels(
        &albedo_rgb,
        &metallic_values,
        &roughness_values,
        ao_values.as_deref(),
        pixel_count,
        crate::pbr::PbrToSpecGlossParams {
            ao_multiplier: params.ao_multiplier,
            specular_multiplier: params.specular_multiplier,
            gloss_multiplier: params.gloss_multiplier,
            spec_offset: params.spec_offset,
        },
    )?;

    let mut diffuse = Vec::with_capacity(pixel_count * 4);
    let mut specgloss = Vec::with_capacity(pixel_count * 4);
    for idx in 0..pixel_count {
        diffuse.extend_from_slice(&converted.diffuse[idx * 3..idx * 3 + 3]);
        diffuse.push(albedo[idx * 4 + 3].clamp(0.0, 1.0));
        specgloss.extend_from_slice(&[converted.specular[idx * 3], converted.gloss[idx], 0.0, 1.0]);
    }

    Ok(StarfieldPbrOutputs { diffuse, specgloss })
}

fn metal_contribution_preserving_hue(
    reflectivity_r: f32,
    reflectivity_g: f32,
    reflectivity_b: f32,
    threshold: f32,
    denominator: f32,
) -> [f32; 3] {
    let peak = reflectivity_r.max(reflectivity_g).max(reflectivity_b);
    let remapped_peak = ((peak - threshold).max(0.0) / denominator).clamp(0.0, 1.0);
    if peak <= 0.0 {
        return [remapped_peak; 3];
    }

    // spec_offset filters reflectivity magnitude. Applying its threshold to
    // each channel separately destroys colored-metal hue (gold/copper turn red).
    let scale = remapped_peak / peak;
    [
        reflectivity_r * scale,
        reflectivity_g * scale,
        reflectivity_b * scale,
    ]
}

pub fn fo76_normal_to_fo4_buffer(
    normal_bytes: &[u8],
    width: usize,
    height: usize,
) -> Result<Vec<f32>> {
    let _timer = directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    let normal = read_rgba_f32_bytes(normal_bytes, width, height, "normal")?;
    fo76_normal_to_fo4_pixels(&normal, width, height)
}

pub fn fo76_normal_to_fo4_pixels(
    normal_pixels: &[f32],
    width: usize,
    height: usize,
) -> Result<Vec<f32>> {
    let _timer =
        directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    let normal = checked_rgba_pixels(normal_pixels, width, height, "normal")?;
    Ok(normal
        .into_iter()
        .map(|value| (value * 0.5 + 0.5).clamp(0.0, 1.0))
        .collect())
}

pub fn fo76_normalized_normal_to_fo4_buffer(
    normal_bytes: &[u8],
    width: usize,
    height: usize,
) -> Result<Vec<f32>> {
    let _timer =
        directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    let normal = read_rgba_f32_bytes(normal_bytes, width, height, "normal")?;
    fo76_normalized_normal_to_fo4_pixels(&normal, width, height)
}

pub fn fo76_normalized_normal_to_fo4_pixels(
    normal_pixels: &[f32],
    width: usize,
    height: usize,
) -> Result<Vec<f32>> {
    let _timer =
        directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    let mut normal = checked_rgba_pixels(normal_pixels, width, height, "normal")?.to_vec();
    for idx in 0..pixel_count(width, height)? {
        let i = idx * 4;
        normal[i] = normal[i].clamp(0.0, 1.0);
        normal[i + 1] = normal[i + 1].clamp(0.0, 1.0);
        normal[i + 2] = 0.0;
        normal[i + 3] = normal[i + 3].clamp(0.0, 1.0);
    }
    Ok(normal)
}

pub fn fo76_reflectivity_lighting_to_fo4_specgloss_buffers(
    reflectivity_bytes: &[u8],
    lighting_bytes: &[u8],
    width: usize,
    height: usize,
    lighting_width: usize,
    lighting_height: usize,
) -> Result<Vec<f32>> {
    let _timer = directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    let reflectivity = read_rgba_f32_bytes(reflectivity_bytes, width, height, "reflectivity")?;
    let lighting =
        read_rgba_f32_bytes(lighting_bytes, lighting_width, lighting_height, "lighting")?;
    fo76_reflectivity_lighting_to_fo4_specgloss_pixels(
        &reflectivity,
        &lighting,
        width,
        height,
        lighting_width,
        lighting_height,
    )
}

pub fn fo76_reflectivity_lighting_to_fo4_specgloss_pixels(
    reflectivity_pixels: &[f32],
    lighting_pixels: &[f32],
    width: usize,
    height: usize,
    lighting_width: usize,
    lighting_height: usize,
) -> Result<Vec<f32>> {
    let _timer =
        directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    let reflectivity = checked_rgba_pixels(reflectivity_pixels, width, height, "reflectivity")?;
    let lighting_raw =
        checked_rgba_pixels(lighting_pixels, lighting_width, lighting_height, "lighting")?;
    let lighting = resize_rgba_bilinear(
        &lighting_raw,
        lighting_width,
        lighting_height,
        width,
        height,
    )?;
    let mut specgloss = vec![0.0; reflectivity.len()];
    for idx in 0..pixel_count(width, height)? {
        let i = idx * 4;
        specgloss[i] = reflectivity[i].clamp(0.0, 1.0);
        specgloss[i + 1] = lighting[i].clamp(0.0, 1.0);
        specgloss[i + 2] = 0.0;
        specgloss[i + 3] = 1.0;
    }
    Ok(specgloss)
}

/// Parameters for the Gamebryo/Skyrim → FO4 spec-gloss synthesis.
///
/// FNV and Skyrim store per-texel specular intensity in the normal map's alpha
/// and cubemap throughput in a separate `_m`/`_em` mask. FO4 has one channel for
/// both: `_s.R`. `_s.G` is glossiness and `_s.B` is unread.
#[derive(Debug, Clone, Copy)]
pub struct GamebryoSpecParams {
    /// Specular for non-metals when the source alpha carries no information.
    /// Matches the constant the FO76 bundle kernel uses for dielectrics.
    pub dielectric_baseline: f32,
    /// Alpha range below which the channel is treated as carrying no data.
    /// 73% of sampled Skyrim normals are uniformly opaque; without this guard
    /// they would all render fully specular.
    pub alpha_flat_epsilon: f32,
    pub envmask_weight: f32,
    pub gloss_baseline: f32,
    pub specular_multiplier: f32,
}

impl Default for GamebryoSpecParams {
    fn default() -> Self {
        Self {
            dielectric_baseline: 0.22,
            alpha_flat_epsilon: 0.02,
            envmask_weight: 1.0,
            gloss_baseline: 0.8,
            specular_multiplier: 1.0,
        }
    }
}

pub struct GamebryoSpecOutputs {
    pub normal: Vec<f32>,
    pub specgloss: Vec<f32>,
}

pub fn gamebryo_normal_envmask_to_fo4_specgloss_buffers(
    normal_bytes: &[u8],
    envmask_bytes: Option<&[u8]>,
    width: usize,
    height: usize,
    envmask_width: usize,
    envmask_height: usize,
    params: GamebryoSpecParams,
) -> Result<GamebryoSpecOutputs> {
    let _timer = directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    let normal = read_rgba_f32_bytes(normal_bytes, width, height, "normal")?;
    let envmask = envmask_bytes
        .map(|bytes| read_rgba_f32_bytes(bytes, envmask_width, envmask_height, "envmask"))
        .transpose()?;
    gamebryo_normal_envmask_to_fo4_specgloss_pixels(
        &normal,
        envmask.as_deref(),
        width,
        height,
        envmask_width,
        envmask_height,
        params,
    )
}

pub fn gamebryo_normal_envmask_to_fo4_specgloss_pixels(
    normal_pixels: &[f32],
    envmask_pixels: Option<&[f32]>,
    width: usize,
    height: usize,
    envmask_width: usize,
    envmask_height: usize,
    params: GamebryoSpecParams,
) -> Result<GamebryoSpecOutputs> {
    let _timer =
        directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    let normal = checked_rgba_pixels(normal_pixels, width, height, "normal")?;
    let count = pixel_count(width, height)?;

    let envmask = match envmask_pixels {
        Some(bytes) => {
            let raw = checked_rgba_pixels(bytes, envmask_width, envmask_height, "envmask")?;
            Some(resize_rgba_bilinear(
                &raw,
                envmask_width,
                envmask_height,
                width,
                height,
            )?)
        }
        None => None,
    };

    let mut alpha_min = f32::MAX;
    let mut alpha_max = f32::MIN;
    for idx in 0..count {
        let alpha = normal[idx * 4 + 3];
        alpha_min = alpha_min.min(alpha);
        alpha_max = alpha_max.max(alpha);
    }
    let alpha_informative = (alpha_max - alpha_min) > params.alpha_flat_epsilon;

    let mut normal_out = vec![0.0; normal.len()];
    let mut specgloss = vec![0.0; normal.len()];
    for idx in 0..count {
        let i = idx * 4;
        let spec_base = if alpha_informative {
            normal[i + 3].clamp(0.0, 1.0)
        } else {
            params.dielectric_baseline
        };
        let mask = envmask.as_ref().map_or(0.0, |values| {
            values[i].clamp(0.0, 1.0) * params.envmask_weight
        });

        normal_out[i] = normal[i].clamp(0.0, 1.0);
        normal_out[i + 1] = normal[i + 1].clamp(0.0, 1.0);
        // FO4 normals are two-channel: it reconstructs Z from X and Y, and
        // every vanilla BC5 normal decodes with blue at ~0 (measured mean 12.7
        // across 4000 `_n.dds`, 9.7 for terrain). Gamebryo sources carry a real
        // Z in blue, which is what makes a carried-over normal read blue
        // instead of FO4's yellow.
        normal_out[i + 2] = 0.0;
        normal_out[i + 3] = 1.0;

        specgloss[i] =
            (spec_base.max(mask).clamp(0.0, 1.0) * params.specular_multiplier).clamp(0.0, 1.0);
        specgloss[i + 1] = params.gloss_baseline.clamp(0.0, 1.0);
        specgloss[i + 2] = 0.0;
        specgloss[i + 3] = 1.0;
    }

    Ok(GamebryoSpecOutputs {
        normal: normal_out,
        specgloss,
    })
}

pub fn passthrough_rgba_buffer(rgba_bytes: &[u8], width: usize, height: usize) -> Result<Vec<f32>> {
    let _timer =
        directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    let rgba = read_rgba_f32_bytes(rgba_bytes, width, height, "rgba")?;
    passthrough_rgba_pixels(&rgba, width, height)
}

pub fn passthrough_rgba_pixels(
    rgba_pixels: &[f32],
    width: usize,
    height: usize,
) -> Result<Vec<f32>> {
    let _timer =
        directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    checked_rgba_pixels(rgba_pixels, width, height, "rgba").map(<[f32]>::to_vec)
}

fn input_path<'a>(request: &'a TextureSetPathRequest, role: &str) -> Option<&'a Path> {
    request
        .inputs
        .iter()
        .find(|input| input.role == role)
        .map(|input| input.path.as_path())
}

pub fn is_named_glow_lighting_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().contains("glow"))
}

fn output_for<'a>(request: &'a TextureSetPathRequest, role: &str) -> Option<&'a TexturePathOutput> {
    request.outputs.iter().find(|output| output.role == role)
}

fn push_converted(result: &mut TextureSetPathResult, output: &TexturePathOutput) {
    result.converted.push(TexturePathResultItem {
        role: output.role.clone(),
        path: output.path.to_string_lossy().replace('\\', "/"),
    });
}

/// sRGB-preserving source→output format map. Keep in sync with the triage in
/// `conversion_native::texture_engine`.
pub fn output_format_for_source(dxgi_format: u32, fallback_format: &str) -> String {
    match dxgi_format {
        71 => "BC1_UNORM",
        72 => "BC1_UNORM_SRGB",
        77 | 98 => "BC7_UNORM",
        78 | 99 => "BC7_UNORM_SRGB",
        28 => "R8G8B8A8_UNORM",
        29 => "R8G8B8A8_UNORM_SRGB",
        80 => "BC4_UNORM",
        83 => "BC5_UNORM",
        61 => "R8_UNORM",
        49 => "R8G8_UNORM",
        _ => fallback_format,
    }
    .to_owned()
}

pub fn output_format_for_role(role: &str, dxgi_format: u32, fallback_format: &str) -> String {
    if role == "specular" {
        return fallback_format.to_owned();
    }
    output_format_for_source(dxgi_format, fallback_format)
}

pub fn output_format_for_path(
    role: &str,
    dxgi_format: u32,
    fallback_format: &str,
    output_path: &Path,
) -> String {
    if matches!(role, "diffuse" | "glow")
        && output_path.components().any(|component| {
            component
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case("effects")
        })
    {
        match dxgi_format {
            77 => return "BC3_UNORM".to_owned(),
            78 => return "BC3_UNORM_SRGB".to_owned(),
            _ => {}
        }
    }
    output_format_for_role(role, dxgi_format, fallback_format)
}

/// A light gobo (projected-light mask) lives under a `Gobos/` (Effects) or
/// `GOBO/` directory. FO4 samples these as a linear mask — every vanilla FO4
/// gobo is `BC1_UNORM`, never sRGB — so a gobo carried over from FO76 with its
/// sRGB format leaves the projected light unmasked in FO4.
fn is_light_gobo_path(path: &Path) -> bool {
    path.components().any(|component| {
        let segment = component.as_os_str().to_string_lossy();
        segment.eq_ignore_ascii_case("gobos") || segment.eq_ignore_ascii_case("gobo")
    })
}

/// Linear (non-sRGB) counterpart of an sRGB DDS format string. Returns the
/// input unchanged when it is already linear.
fn linear_format_variant(format: &str) -> &str {
    match format {
        "BC1_UNORM_SRGB" => "BC1_UNORM",
        "BC2_UNORM_SRGB" => "BC2_UNORM",
        "BC3_UNORM_SRGB" => "BC3_UNORM",
        "BC7_UNORM_SRGB" => "BC7_UNORM",
        "R8G8B8A8_UNORM_SRGB" => "R8G8B8A8_UNORM",
        other => other,
    }
}

fn srgb_channel_to_linear(channel: f32) -> f32 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

/// Bake the sRGB→linear transfer into the RGB channels so the stored values
/// match what the source sampled as an sRGB texture. Alpha is left untouched.
fn linearize_rgb_in_place(rgba: &mut [f32]) {
    for pixel in rgba.chunks_mut(4) {
        if pixel.len() < 3 {
            continue;
        }
        pixel[0] = srgb_channel_to_linear(pixel[0]);
        pixel[1] = srgb_channel_to_linear(pixel[1]);
        pixel[2] = srgb_channel_to_linear(pixel[2]);
    }
}

fn output_uses_gpu(
    format: &str,
    width: u32,
    height: u32,
    use_gpu: bool,
    gpu_min_pixels: u32,
) -> bool {
    if !use_gpu || !matches!(format, "BC7_UNORM" | "BC7_UNORM_SRGB") {
        return false;
    }
    u64::from(width) * u64::from(height) >= u64::from(gpu_min_pixels)
}

fn write_float_output(
    output: &TexturePathOutput,
    width: u32,
    height: u32,
    rgba: &[f32],
    format: &str,
    use_gpu: bool,
    gpu_min_pixels: u32,
    parallel_compression: bool,
) -> Result<()> {
    if let Some(parent) = output.path.parent() {
        directxtex_native::profiling::create_dir_all(parent)
            .map_err(|error| MaterialError::runtime(error.to_string()))?;
    }
    let effective_use_gpu = output_uses_gpu(format, width, height, use_gpu, gpu_min_pixels);
    directxtex_native::write_dds_float_rgba_image_gpu(
        output.path.as_path(),
        width,
        height,
        rgba,
        format,
        true,
        parallel_compression,
        effective_use_gpu,
    )
    .map_err(MaterialError::runtime)
}

pub fn convert_texture_set_paths(request: TextureSetPathRequest) -> Result<TextureSetPathResult> {
    if request.source_game == request.target_game {
        return copy_same_game_paths(request);
    }
    if request.source_game == "fo76" && request.target_game == "fo4" {
        return convert_fo76_to_fo4_paths(request);
    }
    Err(MaterialError::invalid(format!(
        "No converter for {} to {}",
        request.source_game, request.target_game
    )))
}

fn copy_same_game_paths(request: TextureSetPathRequest) -> Result<TextureSetPathResult> {
    let mut result = TextureSetPathResult {
        converted: Vec::new(),
        skipped: Vec::new(),
    };
    for input in request.inputs {
        if let Some(output) = request
            .outputs
            .iter()
            .find(|candidate| candidate.role == input.role)
        {
            if let Some(parent) = output.path.parent() {
                directxtex_native::profiling::create_dir_all(parent)
                    .map_err(|error| MaterialError::runtime(error.to_string()))?;
            }
            std::fs::copy(&input.path, &output.path)
                .map_err(|error| MaterialError::runtime(error.to_string()))?;
            push_converted(&mut result, output);
        } else {
            result.skipped.push(TexturePathSkipItem {
                role: input.role,
                reason: "no matching output path".to_string(),
            });
        }
    }
    Ok(result)
}

// FO76 texture slots do NOT map 1:1 to FO4 (e.g. `_l` lighting is a packed
// smoothness/AO/SSS/emissive map, NOT the same as FO4 `_g` glow). See
// `materials/FO76_TEXTURE_FORMATS.md` for the channel reference.
fn convert_fo76_to_fo4_paths(request: TextureSetPathRequest) -> Result<TextureSetPathResult> {
    let mut result = TextureSetPathResult {
        converted: Vec::new(),
        skipped: Vec::new(),
    };
    let mut params: TextureConversionParams = request.params.into();

    let diffuse_path = input_path(&request, "diffuse");
    let reflectivity_path = input_path(&request, "reflectivity");
    let lighting_path = input_path(&request, "lighting");

    if let (Some(diffuse_path), Some(reflectivity_path), Some(lighting_path)) =
        (diffuse_path, reflectivity_path, lighting_path)
    {
        params.preserve_lighting_rgb_for_glow = is_named_glow_lighting_path(lighting_path);
        let diffuse = directxtex_native::read_dds_float_rgba_image(diffuse_path)
            .map_err(MaterialError::runtime)?;
        let reflectivity = directxtex_native::read_dds_float_rgba_image(reflectivity_path)
            .map_err(MaterialError::runtime)?;
        let lighting = directxtex_native::read_dds_float_rgba_image(lighting_path)
            .map_err(MaterialError::runtime)?;
        let outputs = fo76_bundle_to_fo4_pixels(
            &diffuse.rgba,
            &reflectivity.rgba,
            &lighting.rgba,
            diffuse.width as usize,
            diffuse.height as usize,
            reflectivity.width as usize,
            reflectivity.height as usize,
            lighting.width as usize,
            lighting.height as usize,
            params,
            output_for(&request, "glow").is_some(),
        )?;

        if let Some(output) = output_for(&request, "diffuse") {
            let format = output_format_for_path(
                &output.role,
                diffuse.dxgi_format,
                &output.format,
                &output.path,
            );
            write_float_output(
                output,
                diffuse.width,
                diffuse.height,
                &outputs.diffuse,
                &format,
                request.use_gpu,
                request.gpu_min_pixels,
                request.parallel_compression,
            )?;
            push_converted(&mut result, output);
        }
        if let Some(output) = output_for(&request, "specular") {
            let format = output_format_for_path(
                &output.role,
                reflectivity.dxgi_format,
                &output.format,
                &output.path,
            );
            write_float_output(
                output,
                diffuse.width,
                diffuse.height,
                &outputs.specgloss,
                &format,
                request.use_gpu,
                request.gpu_min_pixels,
                request.parallel_compression,
            )?;
            push_converted(&mut result, output);
        }
        if let (Some(output), Some(glow)) = (output_for(&request, "glow"), outputs.glow.as_ref()) {
            let format = output_format_for_path(
                &output.role,
                lighting.dxgi_format,
                &output.format,
                &output.path,
            );
            write_float_output(
                output,
                diffuse.width,
                diffuse.height,
                glow,
                &format,
                request.use_gpu,
                request.gpu_min_pixels,
                request.parallel_compression,
            )?;
            push_converted(&mut result, output);
        }
        // The bundle handles diffuse / specular / glow; remaining inputs
        // (notably `normal`) still need per-channel conversion or they get
        // silently dropped from the output set. Mirrors the 2-channel branch
        // below.
        convert_fo76_individual_paths_into(
            &request,
            &mut result,
            &["diffuse", "reflectivity", "lighting"],
        )?;
        return Ok(result);
    }

    if let (Some(reflectivity_path), Some(lighting_path), Some(output)) = (
        reflectivity_path,
        lighting_path,
        output_for(&request, "specular"),
    ) {
        let reflectivity = directxtex_native::read_dds_float_rgba_image(reflectivity_path)
            .map_err(MaterialError::runtime)?;
        let lighting = directxtex_native::read_dds_float_rgba_image(lighting_path)
            .map_err(MaterialError::runtime)?;
        let specgloss = fo76_reflectivity_lighting_to_fo4_specgloss_pixels(
            &reflectivity.rgba,
            &lighting.rgba,
            reflectivity.width as usize,
            reflectivity.height as usize,
            lighting.width as usize,
            lighting.height as usize,
        )?;
        let format = output_format_for_path(
            &output.role,
            reflectivity.dxgi_format,
            &output.format,
            &output.path,
        );
        write_float_output(
            output,
            reflectivity.width,
            reflectivity.height,
            &specgloss,
            &format,
            request.use_gpu,
            request.gpu_min_pixels,
            request.parallel_compression,
        )?;
        push_converted(&mut result, output);
        convert_fo76_individual_paths_into(&request, &mut result, &["reflectivity", "lighting"])?;
        return Ok(result);
    }

    // A lone `_l` bound to the glow slot: derive the mask from alpha instead of
    // letting the individual path pass the packed RGB through.
    if let (Some(lighting_path), Some(output)) = (lighting_path, output_for(&request, "glow")) {
        let lighting = directxtex_native::read_dds_float_rgba_image(lighting_path)
            .map_err(MaterialError::runtime)?;
        let glow = fo76_lighting_to_fo4_glow_pixels(
            &lighting.rgba,
            lighting.width as usize,
            lighting.height as usize,
            is_named_glow_lighting_path(lighting_path),
        )?;
        let format = output_format_for_path(
            &output.role,
            lighting.dxgi_format,
            &output.format,
            &output.path,
        );
        write_float_output(
            output,
            lighting.width,
            lighting.height,
            &glow,
            &format,
            request.use_gpu,
            request.gpu_min_pixels,
            request.parallel_compression,
        )?;
        push_converted(&mut result, output);
        convert_fo76_individual_paths_into(&request, &mut result, &["lighting"])?;
        return Ok(result);
    }

    convert_fo76_individual_paths(request)
}

fn convert_fo76_individual_paths(request: TextureSetPathRequest) -> Result<TextureSetPathResult> {
    let mut result = TextureSetPathResult {
        converted: Vec::new(),
        skipped: Vec::new(),
    };
    convert_fo76_individual_paths_into(&request, &mut result, &[])?;
    Ok(result)
}

fn convert_fo76_individual_paths_into(
    request: &TextureSetPathRequest,
    result: &mut TextureSetPathResult,
    skip_roles: &[&str],
) -> Result<()> {
    for input in &request.inputs {
        if skip_roles.iter().any(|role| input.role.as_str() == *role) {
            continue;
        }
        let Some(output_role) = mapped_fo76_output_role(input.role.as_str()) else {
            result.skipped.push(TexturePathSkipItem {
                role: input.role.clone(),
                reason: "unsupported texture role".to_string(),
            });
            continue;
        };
        let Some(output) = output_for(&request, output_role) else {
            result.skipped.push(TexturePathSkipItem {
                role: input.role.clone(),
                reason: "no matching output path".to_string(),
            });
            continue;
        };

        let image = directxtex_native::read_dds_float_rgba_image(input.path.as_path())
            .map_err(MaterialError::runtime)?;
        convert_fo76_individual_image(
            &input.role,
            output,
            image,
            request.use_gpu,
            request.gpu_min_pixels,
            request.parallel_compression,
        )?;
        push_converted(result, output);
    }
    Ok(())
}

pub fn convert_fo76_individual_image(
    input_role: &str,
    output: &TexturePathOutput,
    image: directxtex_native::DdsRgbaFloatImage,
    use_gpu: bool,
    gpu_min_pixels: u32,
    parallel_compression: bool,
) -> Result<()> {
    let mut rgba = if input_role == "normal" {
        fo76_normalized_normal_to_fo4_pixels(
            &image.rgba,
            image.width as usize,
            image.height as usize,
        )?
    } else {
        passthrough_rgba_pixels(&image.rgba, image.width as usize, image.height as usize)?
    };
    let mut format = output_format_for_path(
        &output.role,
        image.dxgi_format,
        &output.format,
        &output.path,
    );
    if is_light_gobo_path(output.path.as_path()) {
        let linear = linear_format_variant(&format);
        if linear != format {
            linearize_rgb_in_place(&mut rgba);
            format = linear.to_string();
        }
    }
    write_float_output(
        output,
        image.width,
        image.height,
        &rgba,
        &format,
        use_gpu,
        gpu_min_pixels,
        parallel_compression,
    )?;
    Ok(())
}

pub fn mapped_fo76_output_role(role: &str) -> Option<&'static str> {
    match role {
        "normal" => Some("normal"),
        "diffuse" => Some("diffuse"),
        "glow" => Some("glow"),
        "reflectivity" | "lighting" => Some("specular"),
        _ => None,
    }
}

pub fn f32_vec_to_bytes(values: &[f32]) -> Vec<u8> {
    let _timer = directxtex_native::profiling::Timer::new(directxtex_native::profiling::Stage::Material);
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn pixel_count(width: usize, height: usize) -> Result<usize> {
    if width == 0 || height == 0 {
        return Err(MaterialError::invalid(
            "texture dimensions must be non-zero",
        ));
    }
    width
        .checked_mul(height)
        .ok_or_else(|| MaterialError::invalid("texture dimensions overflow"))
}

fn expected_rgba_byte_len(width: usize, height: usize) -> Result<usize> {
    pixel_count(width, height)?
        .checked_mul(4)
        .and_then(|value_count| value_count.checked_mul(4))
        .ok_or_else(|| MaterialError::invalid("rgba buffer byte length overflow"))
}

fn checked_rgba_pixels<'a>(
    pixels: &'a [f32],
    width: usize,
    height: usize,
    name: &str,
) -> Result<&'a [f32]> {
    let expected = expected_rgba_byte_len(width, height)? / 4;
    if pixels.len() != expected {
        return Err(MaterialError::invalid(format!(
            "{name} buffer has {} pixel values; expected {expected}",
            pixels.len()
        )));
    }
    Ok(pixels)
}

fn read_rgba_f32_bytes(bytes: &[u8], width: usize, height: usize, name: &str) -> Result<Vec<f32>> {
    let expected = expected_rgba_byte_len(width, height)?;
    if bytes.len() != expected {
        return Err(MaterialError::invalid(format!(
            "{name} buffer has {} bytes; expected {expected}",
            bytes.len()
        )));
    }
    let mut values = Vec::with_capacity(pixel_count(width, height)? * 4);
    for chunk in bytes.chunks_exact(4) {
        values.push(f32::from_le_bytes(
            chunk.try_into().expect("chunks_exact(4) yields 4 bytes"),
        ));
    }
    Ok(values)
}

fn check_params(params: TextureConversionParams) -> Result<()> {
    for (name, value) in [
        ("ao_multiplier", params.ao_multiplier),
        ("specular_multiplier", params.specular_multiplier),
        ("gloss_multiplier", params.gloss_multiplier),
        ("spec_offset", params.spec_offset),
    ] {
        if !value.is_finite() {
            return Err(MaterialError::invalid(format!("{name} must be finite")));
        }
    }
    if params.spec_offset < 0.0 {
        return Err(MaterialError::invalid("spec_offset must be non-negative"));
    }
    Ok(())
}

fn resize_rgba_bilinear(
    rgba: &[f32],
    src_width: usize,
    src_height: usize,
    dst_width: usize,
    dst_height: usize,
) -> Result<Vec<f32>> {
    let _ = expected_rgba_byte_len(src_width, src_height)?;
    let _ = expected_rgba_byte_len(dst_width, dst_height)?;
    if rgba.len() != pixel_count(src_width, src_height)? * 4 {
        return Err(MaterialError::invalid(
            "resize input length does not match dimensions",
        ));
    }
    if src_width == dst_width && src_height == dst_height {
        return Ok(rgba.to_vec());
    }

    let mut out = vec![0.0; pixel_count(dst_width, dst_height)? * 4];
    let x_scale = if dst_width > 1 {
        (src_width - 1) as f32 / (dst_width - 1) as f32
    } else {
        0.0
    };
    let y_scale = if dst_height > 1 {
        (src_height - 1) as f32 / (dst_height - 1) as f32
    } else {
        0.0
    };

    for y in 0..dst_height {
        let sy = y as f32 * y_scale;
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(src_height - 1);
        let fy = sy - y0 as f32;
        for x in 0..dst_width {
            let sx = x as f32 * x_scale;
            let x0 = sx.floor() as usize;
            let x1 = (x0 + 1).min(src_width - 1);
            let fx = sx - x0 as f32;
            for channel in 0..4 {
                let a = rgba[(y0 * src_width + x0) * 4 + channel];
                let b = rgba[(y0 * src_width + x1) * 4 + channel];
                let c = rgba[(y1 * src_width + x0) * 4 + channel];
                let d = rgba[(y1 * src_width + x1) * 4 + channel];
                let top = a + (b - a) * fx;
                let bottom = c + (d - c) * fx;
                out[(y * dst_width + x) * 4 + channel] = top + (bottom - top) * fy;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba_bytes(px: [f32; 4]) -> Vec<u8> {
        f32_vec_to_bytes(&px)
    }

    fn decode_rgba(bytes: Vec<f32>) -> [f32; 4] {
        [bytes[0], bytes[1], bytes[2], bytes[3]]
    }

    fn assert_rgba_close(actual: [f32; 4], expected: [f32; 4]) {
        for channel in 0..4 {
            assert!(
                (actual[channel] - expected[channel]).abs() < 1e-6,
                "channel {channel}: expected {}, got {}",
                expected[channel],
                actual[channel]
            );
        }
    }

    #[test]
    fn bundle_uses_source_dielectric_fill_for_non_metal() {
        let out = fo76_bundle_to_fo4_buffers(
            &rgba_bytes([1.0, 0.0, 0.0, 1.0]),
            &rgba_bytes([0.0, 0.0, 0.0, 1.0]),
            &rgba_bytes([0.5, 1.0, 0.0, 1.0]),
            1,
            1,
            1,
            1,
            1,
            1,
            TextureConversionParams::default(),
            false,
        )
        .unwrap();

        assert_eq!(decode_rgba(out.diffuse), [1.0, 0.0, 0.0, 1.0]);
        assert_rgba_close(decode_rgba(out.specgloss), [0.22, 0.5, 0.0, 1.0]);
        assert!(out.glow.is_none());
    }

    #[test]
    fn starfield_pbr_bundle_maps_metal_roughness_and_ao_to_fo4() {
        let out = starfield_pbr_to_fo4_buffers(
            &rgba_bytes([0.5, 0.25, 0.75, 0.4]),
            &rgba_bytes([0.0, 0.0, 0.0, 1.0]),
            &rgba_bytes([0.25, 0.25, 0.25, 1.0]),
            Some(&rgba_bytes([1.0, 1.0, 1.0, 1.0])),
            1,
            1,
            1,
            1,
            1,
            1,
            1,
            1,
            TextureConversionParams::default(),
        )
        .unwrap();

        assert_rgba_close(decode_rgba(out.diffuse), [0.5, 0.25, 0.75, 0.4]);
        assert_rgba_close(decode_rgba(out.specgloss), [0.22, 0.75, 0.0, 1.0]);
    }

    #[test]
    fn bundle_uses_lighting_r_for_gloss_and_lighting_g_for_ao() {
        let out = fo76_bundle_to_fo4_buffers(
            &rgba_bytes([0.5, 0.25, 0.75, 1.0]),
            &rgba_bytes([1.0, 1.0, 1.0, 1.0]),
            &rgba_bytes([0.25, 0.5, 0.0, 1.0]),
            1,
            1,
            1,
            1,
            1,
            1,
            TextureConversionParams::default(),
            false,
        )
        .unwrap();

        assert_eq!(decode_rgba(out.diffuse), [1.0, 0.9375, 1.0, 1.0]);
        assert_eq!(decode_rgba(out.specgloss), [1.0, 0.25, 0.0, 1.0]);
    }

    #[test]
    fn bundle_preserves_colored_metal_reflectivity_hue() {
        let out = fo76_bundle_to_fo4_buffers(
            &rgba_bytes([0.0, 0.0, 0.0, 1.0]),
            &rgba_bytes([0.4, 0.25, 0.1, 1.0]),
            &rgba_bytes([0.5, 1.0, 0.0, 1.0]),
            1,
            1,
            1,
            1,
            1,
            1,
            TextureConversionParams::default(),
            false,
        )
        .unwrap();

        assert_rgba_close(decode_rgba(out.diffuse), [0.25, 0.15625, 0.0625, 1.0]);
    }

    #[test]
    fn bundle_keeps_achromatic_reflectivity_remap() {
        let out = fo76_bundle_to_fo4_buffers(
            &rgba_bytes([0.0, 0.0, 0.0, 1.0]),
            &rgba_bytes([0.25, 0.25, 0.25, 1.0]),
            &rgba_bytes([0.5, 1.0, 0.0, 1.0]),
            1,
            1,
            1,
            1,
            1,
            1,
            TextureConversionParams::default(),
            false,
        )
        .unwrap();

        assert_rgba_close(decode_rgba(out.diffuse), [0.0625, 0.0625, 0.0625, 1.0]);
    }

    #[test]
    fn bundle_emits_white_glow_scaled_by_lighting_emissive_mask() {
        let out = fo76_bundle_to_fo4_buffers(
            &rgba_bytes([0.1, 0.2, 0.3, 0.4]),
            &rgba_bytes([0.0, 0.0, 0.0, 1.0]),
            // _l = gloss(R) / AO(G) / unused(B) / emissive-mask(A). The RGB is
            // NOT emissive colour, so it must not tint the glow.
            &rgba_bytes([0.5, 1.0, 0.0, 0.75]),
            1,
            1,
            1,
            1,
            1,
            1,
            TextureConversionParams::default(),
            true,
        )
        .unwrap();

        // White (grayscale) glow scaled by the emissive mask (alpha = 0.75),
        // not tinted green by the AO channel.
        assert_eq!(decode_rgba(out.glow.unwrap()), [0.75, 0.75, 0.75, 1.0]);
    }

    #[test]
    fn orphan_lighting_glow_matches_the_bundle_mask() {
        // RobCoDispenser02_l.dds has no _d/_r sibling, so it misses the bundle.
        // It must still yield the alpha-derived mask, not the packed RGB.
        let lighting = rgba_bytes([0.5, 1.0, 0.0, 0.75]);
        let orphan = fo76_lighting_to_fo4_glow_buffer(&lighting, 1, 1, false).unwrap();
        assert_eq!(decode_rgba(orphan), [0.75, 0.75, 0.75, 1.0]);

        let bundled = fo76_bundle_to_fo4_buffers(
            &rgba_bytes([0.1, 0.2, 0.3, 0.4]),
            &rgba_bytes([0.0, 0.0, 0.0, 1.0]),
            &lighting,
            1,
            1,
            1,
            1,
            1,
            1,
            TextureConversionParams::default(),
            true,
        )
        .unwrap();
        assert_eq!(
            decode_rgba(fo76_lighting_to_fo4_glow_buffer(&lighting, 1, 1, false).unwrap()),
            decode_rgba(bundled.glow.unwrap()),
            "orphan _l must produce the same glow map the bundle path would"
        );
    }

    #[test]
    fn orphan_lighting_glow_keeps_named_glow_colour() {
        // `*_glow_l.dds` is authored with real colour in RGB; the named-glow
        // exception must survive the orphan path too.
        let out = fo76_lighting_to_fo4_glow_buffer(&rgba_bytes([0.5, 1.0, 0.0, 0.75]), 1, 1, true)
            .unwrap();
        assert_rgba_close(decode_rgba(out), [0.375, 0.75, 0.0, 1.0]);
    }

    #[test]
    fn named_glow_rule_is_limited_to_glow_lighting_filenames() {
        assert!(is_named_glow_lighting_path(Path::new(
            "Actors/Wendigo/wendigo_glow_l.dds"
        )));
        assert!(!is_named_glow_lighting_path(Path::new(
            "Actors/Wendigo/wendigo_l.dds"
        )));
    }

    #[test]
    fn bundle_preserves_named_glow_color_from_lighting_rgb() {
        let out = fo76_bundle_to_fo4_buffers(
            &rgba_bytes([0.1, 0.2, 0.3, 0.4]),
            &rgba_bytes([0.0, 0.0, 0.0, 1.0]),
            &rgba_bytes([0.5, 1.0, 0.0, 0.75]),
            1,
            1,
            1,
            1,
            1,
            1,
            TextureConversionParams {
                preserve_lighting_rgb_for_glow: true,
                ..TextureConversionParams::default()
            },
            true,
        )
        .unwrap();

        assert_eq!(decode_rgba(out.glow.unwrap()), [0.375, 0.75, 0.0, 1.0]);
    }

    #[test]
    fn normal_conversion_matches_reference_signed_to_unsigned_transform() {
        let out =
            fo76_normal_to_fo4_buffer(&f32_vec_to_bytes(&[-1.0, 0.0, 1.0, 0.5]), 1, 1).unwrap();

        assert_eq!(decode_rgba(out), [0.0, 0.5, 1.0, 0.75]);
    }

    #[test]
    fn normalized_normal_conversion_zeroes_blue_and_preserves_other_channels() {
        let out =
            fo76_normalized_normal_to_fo4_buffer(&f32_vec_to_bytes(&[0.25, 0.5, 1.0, 0.75]), 1, 1)
                .unwrap();

        assert_eq!(decode_rgba(out), [0.25, 0.5, 0.0, 0.75]);
    }

    #[test]
    fn reflectivity_lighting_fallback_writes_specgloss_channels() {
        let out = fo76_reflectivity_lighting_to_fo4_specgloss_buffers(
            &f32_vec_to_bytes(&[1.0, 0.25, 0.25, 1.0]),
            &f32_vec_to_bytes(&[0.5, 0.125, 0.0, 1.0]),
            1,
            1,
            1,
            1,
        )
        .unwrap();

        assert_eq!(decode_rgba(out), [1.0, 0.5, 0.0, 1.0]);
    }

    #[test]
    fn invalid_buffer_length_returns_error() {
        let err = passthrough_rgba_buffer(&[0, 1, 2, 3], 1, 1).unwrap_err();
        assert!(
            err.to_string()
                .contains("rgba buffer has 4 bytes; expected 16")
        );
    }

    #[test]
    fn path_converter_writes_fo76_bundle_outputs() {
        let dir =
            std::env::temp_dir().join(format!("modbox21_materials_path_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let diffuse = dir.join("armor_d.dds");
        let reflectivity = dir.join("armor_r.dds");
        let lighting = dir.join("armor_l.dds");
        let diffuse_out = dir.join("out").join("armor_d.dds");
        let spec_out = dir.join("out").join("armor_s.dds");
        let glow_out = dir.join("out").join("armor_g.dds");

        let px = |rgba: [u8; 4]| -> Vec<u8> { vec![rgba[0], rgba[1], rgba[2], rgba[3]] };
        directxtex_native::write_dds_rgba_image(
            &diffuse,
            1,
            1,
            &px([255, 0, 0, 255]),
            "R8G8B8A8_UNORM",
            false,
        )
        .unwrap();
        directxtex_native::write_dds_rgba_image(
            &reflectivity,
            1,
            1,
            &px([0, 0, 0, 255]),
            "R8G8B8A8_UNORM",
            false,
        )
        .unwrap();
        directxtex_native::write_dds_rgba_image(
            &lighting,
            1,
            1,
            &px([128, 255, 0, 192]),
            "R8G8B8A8_UNORM",
            false,
        )
        .unwrap();

        let result = convert_texture_set_paths(TextureSetPathRequest {
            source_game: "fo76".to_string(),
            target_game: "fo4".to_string(),
            inputs: vec![
                TexturePathInput {
                    role: "diffuse".to_string(),
                    path: diffuse.clone(),
                },
                TexturePathInput {
                    role: "reflectivity".to_string(),
                    path: reflectivity.clone(),
                },
                TexturePathInput {
                    role: "lighting".to_string(),
                    path: lighting.clone(),
                },
            ],
            outputs: vec![
                TexturePathOutput {
                    role: "diffuse".to_string(),
                    path: diffuse_out.clone(),
                    format: "R8G8B8A8_UNORM".to_string(),
                },
                TexturePathOutput {
                    role: "specular".to_string(),
                    path: spec_out.clone(),
                    format: "R8G8B8A8_UNORM".to_string(),
                },
                TexturePathOutput {
                    role: "glow".to_string(),
                    path: glow_out.clone(),
                    format: "R8G8B8A8_UNORM".to_string(),
                },
            ],
            params: TextureConversionParamsPayload {
                ao_multiplier: 0.5,
                specular_multiplier: 1.0,
                gloss_multiplier: 1.0,
                spec_offset: 0.8,
            },
            use_gpu: false,
            gpu_min_pixels: 0,
            parallel_compression: false,
        })
        .unwrap();

        assert_eq!(result.converted.len(), 3);
        assert!(diffuse_out.exists());
        assert!(spec_out.exists());
        assert!(glow_out.exists());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn path_converter_keeps_specgloss_bc5_when_reflectivity_is_bc4() {
        let dir = std::env::temp_dir().join(format!(
            "modbox21_materials_spec_bc5_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let diffuse = dir.join("armor_d.dds");
        let reflectivity = dir.join("armor_r.dds");
        let lighting = dir.join("armor_l.dds");
        let spec_out = dir.join("out").join("armor_s.dds");

        directxtex_native::write_dds_rgba_image(
            &diffuse,
            4,
            4,
            &vec![128u8; 4 * 4 * 4],
            "R8G8B8A8_UNORM",
            false,
        )
        .unwrap();
        directxtex_native::write_dds_rgba_image(
            &reflectivity,
            4,
            4,
            &vec![0u8; 4 * 4 * 4],
            "BC4_UNORM",
            false,
        )
        .unwrap();
        directxtex_native::write_dds_rgba_image(
            &lighting,
            4,
            4,
            &vec![128u8; 4 * 4 * 4],
            "R8G8B8A8_UNORM",
            false,
        )
        .unwrap();

        let result = convert_texture_set_paths(TextureSetPathRequest {
            source_game: "fo76".to_string(),
            target_game: "fo4".to_string(),
            inputs: vec![
                TexturePathInput {
                    role: "diffuse".to_string(),
                    path: diffuse,
                },
                TexturePathInput {
                    role: "reflectivity".to_string(),
                    path: reflectivity,
                },
                TexturePathInput {
                    role: "lighting".to_string(),
                    path: lighting,
                },
            ],
            outputs: vec![TexturePathOutput {
                role: "specular".to_string(),
                path: spec_out.clone(),
                format: "BC5_UNORM".to_string(),
            }],
            params: TextureConversionParamsPayload::default(),
            use_gpu: false,
            gpu_min_pixels: 0,
            parallel_compression: false,
        })
        .unwrap();

        assert_eq!(result.converted.len(), 1);
        let image = directxtex_native::read_dds_float_rgba_image(&spec_out).unwrap();
        assert_eq!(image.dxgi_format, 83);
        assert_eq!(image.width, 4);
        assert_eq!(image.height, 4);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn request_defaults_use_gpu_true() {
        let json = serde_json::json!({
            "source_game": "fo76", "target_game": "fo4",
            "inputs": [], "outputs": []
        });
        let req: TextureSetPathRequest = serde_json::from_value(json).unwrap();
        assert!(req.use_gpu, "use_gpu must default to true when absent");
        assert_eq!(req.gpu_min_pixels, 0);
    }

    #[test]
    fn fo76_diffuse_outputs_preserve_source_storage_and_srgb_format() {
        let dir = std::env::temp_dir().join(format!(
            "modbox21_materials_source_formats_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let rgba = vec![128u8; 4 * 4 * 4];
        let cases = [
            ("BC1_UNORM", "bc1_linear_d.dds", 71),
            ("BC1_UNORM_SRGB", "bc1_srgb_d.dds", 72),
            ("BC3_UNORM", "bc3_linear_d.dds", 98),
            ("BC3_UNORM_SRGB", "bc3_srgb_d.dds", 99),
            ("BC7_UNORM", "bc7_linear_d.dds", 98),
            ("BC7_UNORM_SRGB", "bc7_srgb_d.dds", 99),
            ("R8G8B8A8_UNORM", "rgba_linear_d.dds", 28),
            ("R8G8B8A8_UNORM_SRGB", "rgba_srgb_d.dds", 29),
        ];

        for (source_format, source_name, expected_dxgi) in cases {
            let source = dir.join(source_name);
            let output = dir.join("out").join(source_name);
            directxtex_native::write_dds_rgba_image(&source, 4, 4, &rgba, source_format, false)
                .unwrap();

            let result = convert_texture_set_paths(TextureSetPathRequest {
                source_game: "fo76".to_string(),
                target_game: "fo4".to_string(),
                inputs: vec![TexturePathInput {
                    role: "diffuse".to_string(),
                    path: source,
                }],
                outputs: vec![TexturePathOutput {
                    role: "diffuse".to_string(),
                    path: output.clone(),
                    format: "BC7_UNORM".to_string(),
                }],
                params: TextureConversionParamsPayload::default(),
                use_gpu: false,
                gpu_min_pixels: 0,
                parallel_compression: false,
            })
            .unwrap_or_else(|error| panic!("{source_format}: {error:?}"));

            assert_eq!(result.converted.len(), 1);
            let image = directxtex_native::read_dds_float_rgba_image(&output).unwrap();
            assert_eq!(image.dxgi_format, expected_dxgi, "{source_format}");
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fo76_bc3_effect_diffuse_stays_bc3_for_fo4() {
        let dir = std::env::temp_dir().join(format!(
            "modbox21_materials_effect_bc3_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("SmokeNuke76PuffsTile_d.dds");
        let output = dir
            .join("out")
            .join("Textures")
            .join("Effects")
            .join("SmokeNuke76PuffsTile_d.dds");
        let rgba = vec![128u8; 4 * 4 * 4];
        directxtex_native::write_dds_rgba_image(&source, 4, 4, &rgba, "BC3_UNORM_SRGB", false)
            .unwrap();

        convert_texture_set_paths(TextureSetPathRequest {
            source_game: "fo76".to_string(),
            target_game: "fo4".to_string(),
            inputs: vec![TexturePathInput {
                role: "diffuse".to_string(),
                path: source,
            }],
            outputs: vec![TexturePathOutput {
                role: "diffuse".to_string(),
                path: output.clone(),
                format: "BC7_UNORM".to_string(),
            }],
            params: TextureConversionParamsPayload::default(),
            use_gpu: false,
            gpu_min_pixels: 0,
            parallel_compression: false,
        })
        .unwrap();

        let image = directxtex_native::read_dds_float_rgba_image(&output).unwrap();
        assert_eq!(image.dxgi_format, 78);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fo76_gobo_diffuse_converts_to_linear_not_srgb() {
        // Light gobos are sampled as a linear mask by FO4 (vanilla gobos are
        // BC1_UNORM). FO76 ships them sRGB; carrying the sRGB format through
        // leaves the projected light unmasked in FO4. A texture landing under a
        // Gobos/ directory must come out linear.
        let dir = std::env::temp_dir().join(format!(
            "modbox21_materials_gobo_linear_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let rgba = vec![128u8; 4 * 4 * 4];
        let source = dir.join("hemispheresoft_e.dds");
        let output = dir
            .join("out")
            .join("Textures")
            .join("Effects")
            .join("Gobos")
            .join("hemispheresoft_e.dds");
        directxtex_native::write_dds_rgba_image(&source, 4, 4, &rgba, "BC1_UNORM_SRGB", false)
            .unwrap();

        let result = convert_texture_set_paths(TextureSetPathRequest {
            source_game: "fo76".to_string(),
            target_game: "fo4".to_string(),
            inputs: vec![TexturePathInput {
                role: "diffuse".to_string(),
                path: source,
            }],
            outputs: vec![TexturePathOutput {
                role: "diffuse".to_string(),
                path: output.clone(),
                format: "BC7_UNORM".to_string(),
            }],
            params: TextureConversionParamsPayload::default(),
            use_gpu: false,
            gpu_min_pixels: 0,
            parallel_compression: false,
        })
        .unwrap();

        assert_eq!(result.converted.len(), 1);
        let image = directxtex_native::read_dds_float_rgba_image(&output).unwrap();
        assert_eq!(
            image.dxgi_format, 71,
            "gobo must convert to linear BC1_UNORM (71), got {}",
            image.dxgi_format
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bc7_gpu_cutoff_keeps_small_textures_on_cpu() {
        assert!(!output_uses_gpu("BC7_UNORM", 511, 511, true, 512 * 512));
        assert!(output_uses_gpu("BC7_UNORM", 512, 512, true, 512 * 512));
        assert!(!output_uses_gpu("BC1_UNORM", 1024, 1024, true, 512 * 512));
        assert!(!output_uses_gpu("BC7_UNORM", 1024, 1024, false, 512 * 512));
    }

    #[test]
    fn gamebryo_flat_normal_alpha_falls_back_to_dielectric_baseline() {
        // 2x1 normal whose alpha carries no variation: 73% of Skyrim looks like this.
        let normal = f32_vec_to_bytes(&[0.5, 0.5, 1.0, 1.0, 0.5, 0.5, 1.0, 1.0]);
        let out = gamebryo_normal_envmask_to_fo4_specgloss_buffers(
            &normal,
            None,
            2,
            1,
            0,
            0,
            GamebryoSpecParams::default(),
        )
        .unwrap();
        assert!((out.specgloss[0] - 0.22).abs() < 1e-6);
        assert!((out.specgloss[4] - 0.22).abs() < 1e-6);
    }

    #[test]
    fn gamebryo_normal_drops_blue_to_fo4s_two_channel_convention() {
        // Tangent-space source (blue ~1.0) and an object-space terrain source
        // (green ~1.0, blue mid) both keep R and G and lose blue: FO4
        // reconstructs Z, and its own normals decode with blue at ~0 in both
        // categories.
        let normal = f32_vec_to_bytes(&[0.5, 0.5, 0.99, 0.4, 0.48, 0.95, 0.48, 0.4]);
        let out = gamebryo_normal_envmask_to_fo4_specgloss_buffers(
            &normal,
            None,
            2,
            1,
            0,
            0,
            GamebryoSpecParams::default(),
        )
        .unwrap();

        assert!((out.normal[0] - 0.5).abs() < 1e-6);
        assert!((out.normal[1] - 0.5).abs() < 1e-6);
        assert_eq!(out.normal[2], 0.0, "tangent-space blue must be dropped");
        assert_eq!(out.normal[3], 1.0);

        assert!((out.normal[4] - 0.48).abs() < 1e-6);
        assert!(
            (out.normal[5] - 0.95).abs() < 1e-6,
            "terrain green survives"
        );
        assert_eq!(out.normal[6], 0.0, "object-space blue must be dropped too");
    }

    #[test]
    fn gamebryo_varying_normal_alpha_becomes_specular_red() {
        let normal = f32_vec_to_bytes(&[0.5, 0.5, 1.0, 0.0, 0.5, 0.5, 1.0, 1.0]);
        let out = gamebryo_normal_envmask_to_fo4_specgloss_buffers(
            &normal,
            None,
            2,
            1,
            0,
            0,
            GamebryoSpecParams::default(),
        )
        .unwrap();
        assert!((out.specgloss[0] - 0.0).abs() < 1e-6);
        assert!((out.specgloss[4] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn gamebryo_envmask_wins_where_brighter_than_normal_alpha() {
        let normal = f32_vec_to_bytes(&[0.5, 0.5, 1.0, 0.0, 0.5, 0.5, 1.0, 1.0]);
        let mask = f32_vec_to_bytes(&[0.6, 0.6, 0.6, 1.0, 0.1, 0.1, 0.1, 1.0]);
        let out = gamebryo_normal_envmask_to_fo4_specgloss_buffers(
            &normal,
            Some(&mask),
            2,
            1,
            2,
            1,
            GamebryoSpecParams::default(),
        )
        .unwrap();
        assert!(
            (out.specgloss[0] - 0.6).abs() < 1e-6,
            "mask should win texel 0"
        );
        assert!(
            (out.specgloss[4] - 1.0).abs() < 1e-6,
            "alpha should win texel 1"
        );
    }

    #[test]
    fn gamebryo_envmask_resizes_to_normal_dimensions() {
        // Flat 0.0 alpha is uninformative, so the baseline applies and the 1x1
        // mask must be upsampled to cover both texels.
        let normal = f32_vec_to_bytes(&[0.5, 0.5, 1.0, 0.0, 0.5, 0.5, 1.0, 0.0]);
        let mask = f32_vec_to_bytes(&[0.75, 0.75, 0.75, 1.0]);
        let out = gamebryo_normal_envmask_to_fo4_specgloss_buffers(
            &normal,
            Some(&mask),
            2,
            1,
            1,
            1,
            GamebryoSpecParams::default(),
        )
        .unwrap();
        assert!((out.specgloss[0] - 0.75).abs() < 1e-6);
        assert!((out.specgloss[4] - 0.75).abs() < 1e-6);
    }

    #[test]
    fn gamebryo_specgloss_zeroes_blue_and_strips_normal_alpha() {
        let normal = f32_vec_to_bytes(&[0.25, 0.75, 1.0, 0.4, 0.25, 0.75, 1.0, 0.9]);
        let out = gamebryo_normal_envmask_to_fo4_specgloss_buffers(
            &normal,
            None,
            2,
            1,
            0,
            0,
            GamebryoSpecParams::default(),
        )
        .unwrap();
        for idx in 0..2 {
            let i = idx * 4;
            assert_eq!(out.specgloss[i + 2], 0.0, "blue must be zero");
            assert_eq!(out.specgloss[i + 3], 1.0, "alpha must be opaque");
            assert!((out.specgloss[i + 1] - 0.8).abs() < 1e-6, "gloss baseline");
            assert_eq!(out.normal[i + 3], 1.0, "normal alpha must be discarded");
        }
        assert!((out.normal[0] - 0.25).abs() < 1e-6);
        assert!((out.normal[1] - 0.75).abs() < 1e-6);
    }

    #[test]
    fn gamebryo_gloss_baseline_is_configurable_per_game() {
        let normal = f32_vec_to_bytes(&[0.5, 0.5, 1.0, 0.5, 0.5, 0.5, 1.0, 0.5]);
        let params = GamebryoSpecParams {
            gloss_baseline: 0.1,
            ..GamebryoSpecParams::default()
        };
        let out =
            gamebryo_normal_envmask_to_fo4_specgloss_buffers(&normal, None, 2, 1, 0, 0, params)
                .unwrap();
        assert!((out.specgloss[1] - 0.1).abs() < 1e-6);
        assert!((out.specgloss[5] - 0.1).abs() < 1e-6);
    }
}

#[cfg(test)]
mod float_tests;
