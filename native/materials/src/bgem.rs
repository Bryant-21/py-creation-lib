use serde::{Deserialize, Serialize};

use crate::base::{
    BaseHeader, Reader, Writer, read_color3, read_header, write_color3, write_header,
};
use crate::error::Result;

pub const BGEM_SIGNATURE: u32 = 0x4D454742;

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BgemData {
    pub header: BaseHeader,
    pub BaseTexture: String,
    pub GrayscaleTexture: String,
    pub EnvmapTexture: String,
    pub NormalTexture: String,
    pub EnvmapMaskTexture: String,
    pub SpecularTexture: Option<String>,
    pub LightingTexture: Option<String>,
    pub GlowTexture: Option<String>,
    pub GlassRoughnessScratch: Option<String>,
    pub GlassDirtOverlay: Option<String>,
    pub GlassEnabled: Option<bool>,
    pub GlassFresnelColor: Option<[f32; 3]>,
    pub GlassBlurScaleBase: Option<f32>,
    pub GlassBlurScaleFactor: Option<f32>,
    pub GlassRefractionScaleBase: Option<f32>,
    pub EnvironmentMapping: Option<bool>,
    pub EnvironmentMappingMaskScale: Option<f32>,
    pub BloodEnabled: bool,
    pub EffectLightingEnabled: bool,
    pub FalloffEnabled: bool,
    pub FalloffColorEnabled: bool,
    pub GrayscaleToPaletteAlpha: bool,
    pub SoftEnabled: bool,
    pub BaseColor: [f32; 3],
    pub BaseColorScale: f32,
    pub FalloffStartAngle: f32,
    pub FalloffStopAngle: f32,
    pub FalloffStartOpacity: f32,
    pub FalloffStopOpacity: f32,
    pub LightingInfluence: f32,
    pub EnvmapMinLOD: u8,
    pub SoftDepth: f32,
    pub EmittanceColor: Option<[f32; 3]>,
    pub AdaptativeEmissive_ExposureOffset: Option<f32>,
    pub AdaptativeEmissive_FinalExposureMin: Option<f32>,
    pub AdaptativeEmissive_FinalExposureMax: Option<f32>,
    pub Glowmap: Option<bool>,
    pub EffectPbrSpecular: Option<bool>,
}

fn color_or(value: Option<[f32; 3]>, default: [f32; 3]) -> [f32; 3] {
    value.unwrap_or(default)
}

fn write_optional_string(writer: &mut Writer, value: &Option<String>) {
    write_bgem_string(writer, value.as_deref().unwrap_or(""));
}

fn write_bgem_string(writer: &mut Writer, value: &str) {
    // BGEM strings are length-prefixed AND null-terminated, like BGSM. An
    // empty slot must be `len=1, byte=0x00`; emitting `len=0` misaligns the
    // FO4 parser and corrupts every field that follows (manifests as pink
    // materials in-game).
    if value.is_empty() {
        writer.write_string("\0");
        return;
    }
    if value.ends_with('\0') {
        writer.write_string(value);
        return;
    }
    let mut terminated = String::with_capacity(value.len() + 1);
    terminated.push_str(value);
    terminated.push('\0');
    writer.write_string(&terminated);
}

pub fn parse(data: &[u8]) -> Result<BgemData> {
    let mut reader = Reader::new(data);
    let header = read_header(&mut reader, BGEM_SIGNATURE)?;
    let base_texture = reader.read_string()?;
    let grayscale_texture = reader.read_string()?;
    let envmap_texture = reader.read_string()?;
    let normal_texture = reader.read_string()?;
    let envmap_mask_texture = reader.read_string()?;

    let mut specular_texture = None;
    let mut lighting_texture = None;
    let mut glow_texture = None;
    if header.version >= 11 {
        specular_texture = Some(reader.read_string()?);
        lighting_texture = Some(reader.read_string()?);
        glow_texture = Some(reader.read_string()?);
    }

    let mut glass_roughness_scratch = None;
    let mut glass_dirt_overlay = None;
    let mut glass_enabled_payload = None;
    let mut glass_fresnel_color = None;
    let mut glass_blur_scale_base = None;
    let mut glass_blur_scale_factor = None;
    let mut glass_refraction_scale_base = None;
    if header.version >= 21 {
        glass_roughness_scratch = Some(reader.read_string()?);
        glass_dirt_overlay = Some(reader.read_string()?);
        let glass_enabled = reader.read_bool()?;
        glass_enabled_payload = Some(glass_enabled);
        if glass_enabled {
            glass_fresnel_color = Some(read_color3(&mut reader)?);
            glass_blur_scale_base = Some(reader.read_f32()?);
            if header.version >= 22 {
                glass_blur_scale_factor = Some(reader.read_f32()?);
            }
            glass_refraction_scale_base = Some(reader.read_f32()?);
        }
    }

    let mut environment_mapping = None;
    let mut environment_mapping_mask_scale = None;
    if header.version >= 10 {
        environment_mapping = Some(reader.read_bool()?);
        environment_mapping_mask_scale = Some(reader.read_f32()?);
    }

    let blood_enabled = reader.read_bool()?;
    let effect_lighting_enabled = reader.read_bool()?;
    let falloff_enabled = reader.read_bool()?;
    let falloff_color_enabled = reader.read_bool()?;
    let grayscale_to_palette_alpha = reader.read_bool()?;
    let soft_enabled = reader.read_bool()?;
    let base_color = read_color3(&mut reader)?;
    let base_color_scale = reader.read_f32()?;
    let falloff_start_angle = reader.read_f32()?;
    let falloff_stop_angle = reader.read_f32()?;
    let falloff_start_opacity = reader.read_f32()?;
    let falloff_stop_opacity = reader.read_f32()?;
    let lighting_influence = reader.read_f32()?;
    let envmap_min_lod = reader.read_u8()?;
    let soft_depth = reader.read_f32()?;

    let emittance_color = if header.version >= 11 {
        Some(read_color3(&mut reader)?)
    } else {
        None
    };

    let mut adaptative_emissive_exposure_offset = None;
    let mut adaptative_emissive_final_exposure_min = None;
    let mut adaptative_emissive_final_exposure_max = None;
    if header.version >= 15 {
        adaptative_emissive_exposure_offset = Some(reader.read_f32()?);
        adaptative_emissive_final_exposure_min = Some(reader.read_f32()?);
        adaptative_emissive_final_exposure_max = Some(reader.read_f32()?);
    }

    let glowmap = if header.version >= 16 {
        Some(reader.read_bool()?)
    } else {
        None
    };
    let effect_pbr_specular = if header.version >= 20 {
        Some(reader.read_bool()?)
    } else {
        None
    };

    Ok(BgemData {
        header,
        BaseTexture: base_texture,
        GrayscaleTexture: grayscale_texture,
        EnvmapTexture: envmap_texture,
        NormalTexture: normal_texture,
        EnvmapMaskTexture: envmap_mask_texture,
        SpecularTexture: specular_texture,
        LightingTexture: lighting_texture,
        GlowTexture: glow_texture,
        GlassRoughnessScratch: glass_roughness_scratch,
        GlassDirtOverlay: glass_dirt_overlay,
        GlassEnabled: glass_enabled_payload,
        GlassFresnelColor: glass_fresnel_color,
        GlassBlurScaleBase: glass_blur_scale_base,
        GlassBlurScaleFactor: glass_blur_scale_factor,
        GlassRefractionScaleBase: glass_refraction_scale_base,
        EnvironmentMapping: environment_mapping,
        EnvironmentMappingMaskScale: environment_mapping_mask_scale,
        BloodEnabled: blood_enabled,
        EffectLightingEnabled: effect_lighting_enabled,
        FalloffEnabled: falloff_enabled,
        FalloffColorEnabled: falloff_color_enabled,
        GrayscaleToPaletteAlpha: grayscale_to_palette_alpha,
        SoftEnabled: soft_enabled,
        BaseColor: base_color,
        BaseColorScale: base_color_scale,
        FalloffStartAngle: falloff_start_angle,
        FalloffStopAngle: falloff_stop_angle,
        FalloffStartOpacity: falloff_start_opacity,
        FalloffStopOpacity: falloff_stop_opacity,
        LightingInfluence: lighting_influence,
        EnvmapMinLOD: envmap_min_lod,
        SoftDepth: soft_depth,
        EmittanceColor: emittance_color,
        AdaptativeEmissive_ExposureOffset: adaptative_emissive_exposure_offset,
        AdaptativeEmissive_FinalExposureMin: adaptative_emissive_final_exposure_min,
        AdaptativeEmissive_FinalExposureMax: adaptative_emissive_final_exposure_max,
        Glowmap: glowmap,
        EffectPbrSpecular: effect_pbr_specular,
    })
}

pub fn write(data: &BgemData) -> Vec<u8> {
    let mut writer = Writer::new();
    write_header(&mut writer, &data.header);
    write_bgem_string(&mut writer, &data.BaseTexture);
    write_bgem_string(&mut writer, &data.GrayscaleTexture);
    write_bgem_string(&mut writer, &data.EnvmapTexture);
    write_bgem_string(&mut writer, &data.NormalTexture);
    write_bgem_string(&mut writer, &data.EnvmapMaskTexture);
    if data.header.version >= 11 {
        write_optional_string(&mut writer, &data.SpecularTexture);
        write_optional_string(&mut writer, &data.LightingTexture);
        write_optional_string(&mut writer, &data.GlowTexture);
    }
    if data.header.version >= 21 {
        write_optional_string(&mut writer, &data.GlassRoughnessScratch);
        write_optional_string(&mut writer, &data.GlassDirtOverlay);
        let glass_enabled = data.GlassEnabled.unwrap_or(false);
        writer.write_bool(glass_enabled);
        if glass_enabled {
            write_color3(
                &mut writer,
                color_or(data.GlassFresnelColor, [1.0, 1.0, 1.0]),
            );
            writer.write_f32(data.GlassBlurScaleBase.unwrap_or(0.0));
            if data.header.version >= 22 {
                writer.write_f32(data.GlassBlurScaleFactor.unwrap_or(0.0));
            }
            writer.write_f32(data.GlassRefractionScaleBase.unwrap_or(0.0));
        }
    }
    if data.header.version >= 10 {
        writer.write_bool(data.EnvironmentMapping.unwrap_or(false));
        writer.write_f32(data.EnvironmentMappingMaskScale.unwrap_or(0.0));
    }
    writer.write_bool(data.BloodEnabled);
    writer.write_bool(data.EffectLightingEnabled);
    writer.write_bool(data.FalloffEnabled);
    writer.write_bool(data.FalloffColorEnabled);
    writer.write_bool(data.GrayscaleToPaletteAlpha);
    writer.write_bool(data.SoftEnabled);
    write_color3(&mut writer, data.BaseColor);
    writer.write_f32(data.BaseColorScale);
    writer.write_f32(data.FalloffStartAngle);
    writer.write_f32(data.FalloffStopAngle);
    writer.write_f32(data.FalloffStartOpacity);
    writer.write_f32(data.FalloffStopOpacity);
    writer.write_f32(data.LightingInfluence);
    writer.write_u8(data.EnvmapMinLOD);
    writer.write_f32(data.SoftDepth);
    if data.header.version >= 11 {
        write_color3(&mut writer, color_or(data.EmittanceColor, [1.0, 1.0, 1.0]));
    }
    if data.header.version >= 15 {
        writer.write_f32(data.AdaptativeEmissive_ExposureOffset.unwrap_or(0.0));
        writer.write_f32(data.AdaptativeEmissive_FinalExposureMin.unwrap_or(0.0));
        writer.write_f32(data.AdaptativeEmissive_FinalExposureMax.unwrap_or(0.0));
    }
    if data.header.version >= 16 {
        writer.write_bool(data.Glowmap.unwrap_or(false));
    }
    if data.header.version >= 20 {
        writer.write_bool(data.EffectPbrSpecular.unwrap_or(false));
    }
    writer.into_bytes()
}
