use serde::{Deserialize, Serialize};

use crate::base::{
    BaseHeader, Reader, Writer, read_color3, read_header, write_color3, write_header,
};
use crate::error::Result;

pub const BGSM_SIGNATURE: u32 = 0x4D534742;

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BgsmData {
    pub header: BaseHeader,
    pub DiffuseTexture: String,
    pub NormalTexture: String,
    pub SmoothSpecTexture: String,
    pub GreyscaleTexture: String,
    pub EnvmapTexture: Option<String>,
    pub GlowTexture: Option<String>,
    pub InnerLayerTexture: Option<String>,
    pub WrinklesTexture: Option<String>,
    pub DisplacementTexture: Option<String>,
    pub SpecularTexture: Option<String>,
    pub LightingTexture: Option<String>,
    pub FlowTexture: Option<String>,
    pub DistanceFieldAlphaTexture: Option<String>,
    pub EnableEditorAlphaRef: bool,
    pub RimLighting: Option<bool>,
    pub RimPower: Option<f32>,
    pub BackLightPower: Option<f32>,
    pub SubsurfaceLighting: Option<bool>,
    pub SubsurfaceLightingRolloff: Option<f32>,
    pub Translucency: Option<bool>,
    pub TranslucencyThickObject: Option<bool>,
    pub TranslucencyMixAlbedoWithSubsurfaceColor: Option<bool>,
    pub TranslucencySubsurfaceColor: Option<[f32; 3]>,
    pub TranslucencyTransmissiveScale: Option<f32>,
    pub TranslucencyTurbulence: Option<f32>,
    pub SpecularEnabled: bool,
    pub SpecularColor: [f32; 3],
    pub SpecularMult: f32,
    pub Smoothness: f32,
    pub FresnelPower: f32,
    pub WetnessControlSpecScale: f32,
    pub WetnessControlSpecPowerScale: f32,
    pub WetnessControlSpecMinvar: f32,
    pub WetnessControlEnvMapScale: Option<f32>,
    pub WetnessControlFresnelPower: f32,
    pub WetnessControlMetalness: f32,
    pub PBR: Option<bool>,
    pub CustomPorosity: Option<bool>,
    pub PorosityValue: Option<f32>,
    pub RootMaterialPath: String,
    pub AnisoLighting: bool,
    pub EmitEnabled: bool,
    pub EmittanceColor: Option<[f32; 3]>,
    pub EmittanceMult: f32,
    pub ModelSpaceNormals: bool,
    pub ExternalEmittance: bool,
    pub LumEmittance: Option<f32>,
    pub UseAdaptativeEmissive: Option<bool>,
    pub AdaptativeEmissive_ExposureOffset: Option<f32>,
    pub AdaptativeEmissive_FinalExposureMin: Option<f32>,
    pub AdaptativeEmissive_FinalExposureMax: Option<f32>,
    pub BackLighting: Option<bool>,
    pub ReceiveShadows: bool,
    pub HideSecret: bool,
    pub CastShadows: bool,
    pub DissolveFade: bool,
    pub AssumeShadowmask: bool,
    pub Glowmap: bool,
    pub EnvironmentMappingWindow: Option<bool>,
    pub EnvironmentMappingEye: Option<bool>,
    pub Hair: bool,
    pub HairTintColor: [f32; 3],
    pub Tree: bool,
    pub Facegen: bool,
    pub SkinTint: bool,
    pub Tessellate: bool,
    pub DisplacementTextureBias: Option<f32>,
    pub DisplacementTextureScale: Option<f32>,
    pub TessellationPnScale: Option<f32>,
    pub TessellationBaseFactor: Option<f32>,
    pub TessellationFadeDistance: Option<f32>,
    pub GrayscaleToPaletteScale: f32,
    pub SkewSpecularAlpha: Option<bool>,
    pub Terrain: Option<bool>,
    pub UnkInt1: Option<u32>,
    pub TerrainThresholdFalloff: Option<f32>,
    pub TerrainTilingDistance: Option<f32>,
    pub TerrainRotationAngle: Option<f32>,
}

fn color_or(value: Option<[f32; 3]>, default: [f32; 3]) -> [f32; 3] {
    value.unwrap_or(default)
}

fn write_optional_string(writer: &mut Writer, value: &Option<String>) {
    write_bgsm_string(writer, value.as_deref().unwrap_or(""));
}

fn write_bgsm_string(writer: &mut Writer, value: &str) {
    // BGSM strings are length-prefixed AND null-terminated. An empty slot is
    // serialized as `len=1, byte=0x00` (a single null), never `len=0`. FO4's
    // BGSM parser misaligns the rest of the stream when it encounters a
    // zero-length string in a texture slot, which manifests as pink materials
    // in-game.
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

pub fn parse(data: &[u8]) -> Result<BgsmData> {
    let mut reader = Reader::new(data);
    let header = read_header(&mut reader, BGSM_SIGNATURE)?;
    let diffuse_texture = reader.read_string()?;
    let normal_texture = reader.read_string()?;
    let smooth_spec_texture = reader.read_string()?;
    let greyscale_texture = reader.read_string()?;
    let (
        envmap_texture,
        glow_texture,
        inner_layer_texture,
        wrinkles_texture,
        displacement_texture,
        specular_texture,
        lighting_texture,
        flow_texture,
        distance_field_alpha_texture,
    ) = if header.version > 2 {
        let glow_texture = Some(reader.read_string()?);
        let wrinkles_texture = Some(reader.read_string()?);
        let specular_texture = Some(reader.read_string()?);
        let lighting_texture = Some(reader.read_string()?);
        let flow_texture = Some(reader.read_string()?);
        let distance_field_alpha_texture = if header.version >= 17 {
            Some(reader.read_string()?)
        } else {
            None
        };
        (
            None,
            glow_texture,
            None,
            wrinkles_texture,
            None,
            specular_texture,
            lighting_texture,
            flow_texture,
            distance_field_alpha_texture,
        )
    } else {
        (
            Some(reader.read_string()?),
            Some(reader.read_string()?),
            Some(reader.read_string()?),
            Some(reader.read_string()?),
            Some(reader.read_string()?),
            None,
            None,
            None,
            None,
        )
    };

    let enable_editor_alpha_ref = reader.read_bool()?;
    let (
        rim_lighting,
        rim_power,
        back_light_power,
        subsurface_lighting,
        subsurface_lighting_rolloff,
        translucency,
        translucency_thick_object,
        translucency_mix_albedo_with_subsurface_color,
        translucency_subsurface_color,
        translucency_transmissive_scale,
        translucency_turbulence,
    ) = if header.version >= 8 {
        (
            None,
            None,
            None,
            None,
            None,
            Some(reader.read_bool()?),
            Some(reader.read_bool()?),
            Some(reader.read_bool()?),
            Some(read_color3(&mut reader)?),
            Some(reader.read_f32()?),
            Some(reader.read_f32()?),
        )
    } else {
        (
            Some(reader.read_bool()?),
            Some(reader.read_f32()?),
            Some(reader.read_f32()?),
            Some(reader.read_bool()?),
            Some(reader.read_f32()?),
            None,
            None,
            None,
            None,
            None,
            None,
        )
    };

    let specular_enabled = reader.read_bool()?;
    let specular_color = read_color3(&mut reader)?;
    let specular_mult = reader.read_f32()?;
    let smoothness = reader.read_f32()?;
    let fresnel_power = reader.read_f32()?;
    let wetness_control_spec_scale = reader.read_f32()?;
    let wetness_control_spec_power_scale = reader.read_f32()?;
    let wetness_control_spec_minvar = reader.read_f32()?;
    let wetness_control_env_map_scale = if header.version < 10 {
        Some(reader.read_f32()?)
    } else {
        None
    };
    let wetness_control_fresnel_power = reader.read_f32()?;
    let wetness_control_metalness = reader.read_f32()?;

    let mut pbr = None;
    let mut custom_porosity = None;
    let mut porosity_value = None;
    if header.version > 2 {
        pbr = Some(reader.read_bool()?);
        if header.version >= 9 {
            custom_porosity = Some(reader.read_bool()?);
            porosity_value = Some(reader.read_f32()?);
        }
    }

    let root_material_path = reader.read_string()?;
    let aniso_lighting = reader.read_bool()?;
    let emit_enabled = reader.read_bool()?;
    let emittance_color = if emit_enabled {
        Some(read_color3(&mut reader)?)
    } else {
        None
    };
    let emittance_mult = reader.read_f32()?;
    let model_space_normals = reader.read_bool()?;
    let external_emittance = reader.read_bool()?;
    let lum_emittance = if header.version >= 12 {
        Some(reader.read_f32()?)
    } else {
        None
    };

    let mut use_adaptative_emissive = None;
    let mut adaptative_emissive_exposure_offset = None;
    let mut adaptative_emissive_final_exposure_min = None;
    let mut adaptative_emissive_final_exposure_max = None;
    if header.version >= 13 {
        use_adaptative_emissive = Some(reader.read_bool()?);
        adaptative_emissive_exposure_offset = Some(reader.read_f32()?);
        adaptative_emissive_final_exposure_min = Some(reader.read_f32()?);
        adaptative_emissive_final_exposure_max = Some(reader.read_f32()?);
    }

    let back_lighting = if header.version < 8 {
        Some(reader.read_bool()?)
    } else {
        None
    };
    let receive_shadows = reader.read_bool()?;
    let hide_secret = reader.read_bool()?;
    let cast_shadows = reader.read_bool()?;
    let dissolve_fade = reader.read_bool()?;
    let assume_shadowmask = reader.read_bool()?;
    let glowmap = reader.read_bool()?;
    let mut environment_mapping_window = None;
    let mut environment_mapping_eye = None;
    if header.version < 7 {
        environment_mapping_window = Some(reader.read_bool()?);
        environment_mapping_eye = Some(reader.read_bool()?);
    }
    let hair = reader.read_bool()?;
    let hair_tint_color = read_color3(&mut reader)?;
    let tree = reader.read_bool()?;
    let facegen = reader.read_bool()?;
    let skin_tint = reader.read_bool()?;
    let tessellate = reader.read_bool()?;

    let mut displacement_texture_bias = None;
    let mut displacement_texture_scale = None;
    let mut tessellation_pn_scale = None;
    let mut tessellation_base_factor = None;
    let mut tessellation_fade_distance = None;
    if header.version < 3 {
        displacement_texture_bias = Some(reader.read_f32()?);
        displacement_texture_scale = Some(reader.read_f32()?);
        tessellation_pn_scale = Some(reader.read_f32()?);
        tessellation_base_factor = Some(reader.read_f32()?);
        tessellation_fade_distance = Some(reader.read_f32()?);
    }
    let grayscale_to_palette_scale = reader.read_f32()?;
    let skew_specular_alpha = if header.version >= 1 {
        Some(reader.read_bool()?)
    } else {
        None
    };

    let mut terrain = None;
    let mut unk_int1 = None;
    let mut terrain_threshold_falloff = None;
    let mut terrain_tiling_distance = None;
    let mut terrain_rotation_angle = None;
    if header.version >= 3 {
        let terrain_enabled = reader.read_bool()?;
        terrain = Some(terrain_enabled);
        if terrain_enabled {
            if header.version == 3 {
                unk_int1 = Some(reader.read_u32()?);
            }
            terrain_threshold_falloff = Some(reader.read_f32()?);
            terrain_tiling_distance = Some(reader.read_f32()?);
            terrain_rotation_angle = Some(reader.read_f32()?);
        }
    }

    Ok(BgsmData {
        header,
        DiffuseTexture: diffuse_texture,
        NormalTexture: normal_texture,
        SmoothSpecTexture: smooth_spec_texture,
        GreyscaleTexture: greyscale_texture,
        EnvmapTexture: envmap_texture,
        GlowTexture: glow_texture,
        InnerLayerTexture: inner_layer_texture,
        WrinklesTexture: wrinkles_texture,
        DisplacementTexture: displacement_texture,
        SpecularTexture: specular_texture,
        LightingTexture: lighting_texture,
        FlowTexture: flow_texture,
        DistanceFieldAlphaTexture: distance_field_alpha_texture,
        EnableEditorAlphaRef: enable_editor_alpha_ref,
        RimLighting: rim_lighting,
        RimPower: rim_power,
        BackLightPower: back_light_power,
        SubsurfaceLighting: subsurface_lighting,
        SubsurfaceLightingRolloff: subsurface_lighting_rolloff,
        Translucency: translucency,
        TranslucencyThickObject: translucency_thick_object,
        TranslucencyMixAlbedoWithSubsurfaceColor: translucency_mix_albedo_with_subsurface_color,
        TranslucencySubsurfaceColor: translucency_subsurface_color,
        TranslucencyTransmissiveScale: translucency_transmissive_scale,
        TranslucencyTurbulence: translucency_turbulence,
        SpecularEnabled: specular_enabled,
        SpecularColor: specular_color,
        SpecularMult: specular_mult,
        Smoothness: smoothness,
        FresnelPower: fresnel_power,
        WetnessControlSpecScale: wetness_control_spec_scale,
        WetnessControlSpecPowerScale: wetness_control_spec_power_scale,
        WetnessControlSpecMinvar: wetness_control_spec_minvar,
        WetnessControlEnvMapScale: wetness_control_env_map_scale,
        WetnessControlFresnelPower: wetness_control_fresnel_power,
        WetnessControlMetalness: wetness_control_metalness,
        PBR: pbr,
        CustomPorosity: custom_porosity,
        PorosityValue: porosity_value,
        RootMaterialPath: root_material_path,
        AnisoLighting: aniso_lighting,
        EmitEnabled: emit_enabled,
        EmittanceColor: emittance_color,
        EmittanceMult: emittance_mult,
        ModelSpaceNormals: model_space_normals,
        ExternalEmittance: external_emittance,
        LumEmittance: lum_emittance,
        UseAdaptativeEmissive: use_adaptative_emissive,
        AdaptativeEmissive_ExposureOffset: adaptative_emissive_exposure_offset,
        AdaptativeEmissive_FinalExposureMin: adaptative_emissive_final_exposure_min,
        AdaptativeEmissive_FinalExposureMax: adaptative_emissive_final_exposure_max,
        BackLighting: back_lighting,
        ReceiveShadows: receive_shadows,
        HideSecret: hide_secret,
        CastShadows: cast_shadows,
        DissolveFade: dissolve_fade,
        AssumeShadowmask: assume_shadowmask,
        Glowmap: glowmap,
        EnvironmentMappingWindow: environment_mapping_window,
        EnvironmentMappingEye: environment_mapping_eye,
        Hair: hair,
        HairTintColor: hair_tint_color,
        Tree: tree,
        Facegen: facegen,
        SkinTint: skin_tint,
        Tessellate: tessellate,
        DisplacementTextureBias: displacement_texture_bias,
        DisplacementTextureScale: displacement_texture_scale,
        TessellationPnScale: tessellation_pn_scale,
        TessellationBaseFactor: tessellation_base_factor,
        TessellationFadeDistance: tessellation_fade_distance,
        GrayscaleToPaletteScale: grayscale_to_palette_scale,
        SkewSpecularAlpha: skew_specular_alpha,
        Terrain: terrain,
        UnkInt1: unk_int1,
        TerrainThresholdFalloff: terrain_threshold_falloff,
        TerrainTilingDistance: terrain_tiling_distance,
        TerrainRotationAngle: terrain_rotation_angle,
    })
}

pub fn write(data: &BgsmData) -> Vec<u8> {
    let mut writer = Writer::new();
    write_header(&mut writer, &data.header);
    write_bgsm_string(&mut writer, &data.DiffuseTexture);
    write_bgsm_string(&mut writer, &data.NormalTexture);
    write_bgsm_string(&mut writer, &data.SmoothSpecTexture);
    write_bgsm_string(&mut writer, &data.GreyscaleTexture);
    if data.header.version > 2 {
        write_optional_string(&mut writer, &data.GlowTexture);
        write_optional_string(&mut writer, &data.WrinklesTexture);
        write_optional_string(&mut writer, &data.SpecularTexture);
        write_optional_string(&mut writer, &data.LightingTexture);
        write_optional_string(&mut writer, &data.FlowTexture);
        if data.header.version >= 17 {
            write_optional_string(&mut writer, &data.DistanceFieldAlphaTexture);
        }
    } else {
        write_optional_string(&mut writer, &data.EnvmapTexture);
        write_optional_string(&mut writer, &data.GlowTexture);
        write_optional_string(&mut writer, &data.InnerLayerTexture);
        write_optional_string(&mut writer, &data.WrinklesTexture);
        write_optional_string(&mut writer, &data.DisplacementTexture);
    }

    writer.write_bool(data.EnableEditorAlphaRef);
    if data.header.version >= 8 {
        writer.write_bool(data.Translucency.unwrap_or(false));
        writer.write_bool(data.TranslucencyThickObject.unwrap_or(false));
        writer.write_bool(
            data.TranslucencyMixAlbedoWithSubsurfaceColor
                .unwrap_or(false),
        );
        write_color3(
            &mut writer,
            color_or(data.TranslucencySubsurfaceColor, [1.0, 1.0, 1.0]),
        );
        writer.write_f32(data.TranslucencyTransmissiveScale.unwrap_or(0.0));
        writer.write_f32(data.TranslucencyTurbulence.unwrap_or(0.0));
    } else {
        writer.write_bool(data.RimLighting.unwrap_or(false));
        writer.write_f32(data.RimPower.unwrap_or(0.0));
        writer.write_f32(data.BackLightPower.unwrap_or(0.0));
        writer.write_bool(data.SubsurfaceLighting.unwrap_or(false));
        writer.write_f32(data.SubsurfaceLightingRolloff.unwrap_or(0.0));
    }

    writer.write_bool(data.SpecularEnabled);
    write_color3(&mut writer, data.SpecularColor);
    writer.write_f32(data.SpecularMult);
    writer.write_f32(data.Smoothness);
    writer.write_f32(data.FresnelPower);
    writer.write_f32(data.WetnessControlSpecScale);
    writer.write_f32(data.WetnessControlSpecPowerScale);
    writer.write_f32(data.WetnessControlSpecMinvar);
    if data.header.version < 10 {
        writer.write_f32(data.WetnessControlEnvMapScale.unwrap_or(0.0));
    }
    writer.write_f32(data.WetnessControlFresnelPower);
    writer.write_f32(data.WetnessControlMetalness);
    if data.header.version > 2 {
        writer.write_bool(data.PBR.unwrap_or(false));
        if data.header.version >= 9 {
            writer.write_bool(data.CustomPorosity.unwrap_or(false));
            writer.write_f32(data.PorosityValue.unwrap_or(0.0));
        }
    }
    write_bgsm_string(&mut writer, &data.RootMaterialPath);
    writer.write_bool(data.AnisoLighting);
    writer.write_bool(data.EmitEnabled);
    if data.EmitEnabled {
        write_color3(&mut writer, color_or(data.EmittanceColor, [1.0, 1.0, 1.0]));
    }
    writer.write_f32(data.EmittanceMult);
    writer.write_bool(data.ModelSpaceNormals);
    writer.write_bool(data.ExternalEmittance);
    if data.header.version >= 12 {
        writer.write_f32(data.LumEmittance.unwrap_or(0.0));
    }
    if data.header.version >= 13 {
        writer.write_bool(data.UseAdaptativeEmissive.unwrap_or(false));
        writer.write_f32(data.AdaptativeEmissive_ExposureOffset.unwrap_or(0.0));
        writer.write_f32(data.AdaptativeEmissive_FinalExposureMin.unwrap_or(0.0));
        writer.write_f32(data.AdaptativeEmissive_FinalExposureMax.unwrap_or(0.0));
    }
    if data.header.version < 8 {
        writer.write_bool(data.BackLighting.unwrap_or(false));
    }
    writer.write_bool(data.ReceiveShadows);
    writer.write_bool(data.HideSecret);
    writer.write_bool(data.CastShadows);
    writer.write_bool(data.DissolveFade);
    writer.write_bool(data.AssumeShadowmask);
    writer.write_bool(data.Glowmap);
    if data.header.version < 7 {
        writer.write_bool(data.EnvironmentMappingWindow.unwrap_or(false));
        writer.write_bool(data.EnvironmentMappingEye.unwrap_or(false));
    }
    writer.write_bool(data.Hair);
    write_color3(&mut writer, data.HairTintColor);
    writer.write_bool(data.Tree);
    writer.write_bool(data.Facegen);
    writer.write_bool(data.SkinTint);
    writer.write_bool(data.Tessellate);
    if data.header.version < 3 {
        writer.write_f32(data.DisplacementTextureBias.unwrap_or(0.0));
        writer.write_f32(data.DisplacementTextureScale.unwrap_or(0.0));
        writer.write_f32(data.TessellationPnScale.unwrap_or(0.0));
        writer.write_f32(data.TessellationBaseFactor.unwrap_or(0.0));
        writer.write_f32(data.TessellationFadeDistance.unwrap_or(0.0));
    }
    writer.write_f32(data.GrayscaleToPaletteScale);
    if data.header.version >= 1 {
        writer.write_bool(data.SkewSpecularAlpha.unwrap_or(false));
    }
    if data.header.version >= 3 {
        let terrain = data.Terrain.unwrap_or(false);
        writer.write_bool(terrain);
        if terrain {
            if data.header.version == 3 {
                writer.write_u32(data.UnkInt1.unwrap_or(0));
            }
            writer.write_f32(data.TerrainThresholdFalloff.unwrap_or(0.0));
            writer.write_f32(data.TerrainTilingDistance.unwrap_or(0.0));
            writer.write_f32(data.TerrainRotationAngle.unwrap_or(0.0));
        }
    }
    writer.into_bytes()
}
