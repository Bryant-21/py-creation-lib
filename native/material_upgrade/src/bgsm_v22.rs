//! FO4 BGSM v2 -> BGSM v22+ (PBR) field mapping.
//!
//! Reproduces the reference upgrade of 6,609 same-path materials: identical in
//! 35.5 % of them field-for-field and 74.6 % ignoring texture paths and the
//! legacy scalars the PBR path never reads.

use materials_native::bgsm::BgsmData;

pub const V22: u32 = 22;
pub const FIRST_V2X: u32 = 20;

fn is_template_root(root: &str) -> bool {
    let root = root.trim_end_matches('\0').replace('\\', "/").to_ascii_lowercase();
    root.starts_with("template/") || root.starts_with("materials/template/")
}

pub fn is_already_v2x(data: &BgsmData) -> bool {
    data.header.version >= FIRST_V2X
}

pub fn upgrade_bgsm(source: &BgsmData, reflectance: &str, lighting: &str, translucent: bool) -> BgsmData {
    let mut v = source.clone();
    v.header.version = V22;
    v.header.depth_bias = Some(false);
    v.header.mask_writes = Some(63);
    v.header.env_mapping = None;
    v.header.env_mapping_mask_scale = None;

    v.SmoothSpecTexture = String::new();
    v.EnvmapTexture = None;
    v.InnerLayerTexture = None;
    v.DisplacementTexture = None;
    v.SpecularTexture = Some(reflectance.to_string());
    v.LightingTexture = Some(lighting.to_string());
    v.FlowTexture = Some(String::new());
    v.DistanceFieldAlphaTexture = Some(String::new());

    v.RimLighting = None;
    v.RimPower = None;
    v.BackLightPower = None;
    v.SubsurfaceLighting = None;
    v.SubsurfaceLightingRolloff = None;
    v.WetnessControlEnvMapScale = None;
    v.BackLighting = None;
    v.EnvironmentMappingWindow = None;
    v.EnvironmentMappingEye = None;
    v.DisplacementTextureBias = None;
    v.DisplacementTextureScale = None;
    v.TessellationPnScale = None;
    v.TessellationBaseFactor = None;
    v.TessellationFadeDistance = None;

    v.Translucency = Some(translucent);
    v.TranslucencyThickObject = Some(false);
    v.TranslucencyMixAlbedoWithSubsurfaceColor = Some(true);
    v.TranslucencySubsurfaceColor = Some([1.0, 1.0, 1.0]);
    v.TranslucencyTransmissiveScale = Some(1.0);
    v.TranslucencyTurbulence = Some(0.5);

    v.PBR = Some(true);
    // Baked into `_l.R`; FO4CS reads the scalar only when `_l` is missing.
    v.Smoothness = 1.0;
    v.CustomPorosity = Some(false);
    v.PorosityValue = Some(0.0);
    v.LumEmittance = Some(100.0);
    v.UseAdaptativeEmissive = Some(false);
    v.AdaptativeEmissive_ExposureOffset = Some(0.0);
    v.AdaptativeEmissive_FinalExposureMin = Some(0.0);
    v.AdaptativeEmissive_FinalExposureMax = Some(1.0);
    v.Terrain = Some(false);
    if v.EmitEnabled && v.EmittanceColor.is_none() {
        v.EmittanceColor = Some([1.0, 1.0, 1.0]);
    }
    // FO4 `Template/*_Wet.bgsm` roots are v2 files.
    if is_template_root(&v.RootMaterialPath) {
        v.RootMaterialPath = String::new();
    }
    v
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use materials_native::bgsm::{parse, write};

    pub(crate) fn fo4_bgsm() -> BgsmData {
        let mut data = BgsmData::default();
        data.header.signature = materials_native::bgsm::BGSM_SIGNATURE;
        data.header.version = 2;
        data.header.tile_u = true;
        data.header.tile_v = true;
        data.header.u_scale = 1.0;
        data.header.v_scale = 1.0;
        data.header.alpha = 1.0;
        data.header.alpha_blend_mode1 = 6;
        data.header.alpha_blend_mode2 = 7;
        data.header.alpha_test_ref = 128;
        data.header.zbuffer_write = true;
        data.header.zbuffer_test = true;
        data.header.env_mapping = Some(true);
        data.header.env_mapping_mask_scale = Some(1.0);
        data.DiffuseTexture = "SetDressing/Foo/Foo_d.dds".into();
        data.NormalTexture = "SetDressing/Foo/Foo_n.dds".into();
        data.SmoothSpecTexture = "SetDressing/Foo/Foo_s.dds".into();
        data.EnvmapTexture = Some("Shared/Cubemaps/mipblur_DefaultOutside1.dds".into());
        data.GlowTexture = Some(String::new());
        data.InnerLayerTexture = Some(String::new());
        data.WrinklesTexture = Some(String::new());
        data.DisplacementTexture = Some(String::new());
        data.RimLighting = Some(false);
        data.RimPower = Some(2.0);
        data.BackLightPower = Some(0.0);
        data.SubsurfaceLighting = Some(false);
        data.SubsurfaceLightingRolloff = Some(0.3);
        data.SpecularEnabled = true;
        data.SpecularColor = [1.0, 1.0, 1.0];
        data.SpecularMult = 1.0;
        data.Smoothness = 0.6;
        data.FresnelPower = 5.0;
        data.WetnessControlSpecScale = -1.0;
        data.WetnessControlSpecPowerScale = -1.0;
        data.WetnessControlSpecMinvar = -1.0;
        data.WetnessControlEnvMapScale = Some(-1.0);
        data.WetnessControlFresnelPower = -1.0;
        data.WetnessControlMetalness = -1.0;
        data.RootMaterialPath = "Template/MetalTemplate_Wet.bgsm".into();
        data.EmittanceMult = 1.0;
        data.BackLighting = Some(false);
        data.CastShadows = true;
        data.ReceiveShadows = true;
        data.EnvironmentMappingWindow = Some(false);
        data.EnvironmentMappingEye = Some(false);
        data.HairTintColor = [1.0, 1.0, 1.0];
        data.DisplacementTextureBias = Some(0.0);
        data.DisplacementTextureScale = Some(0.0);
        data.TessellationPnScale = Some(0.0);
        data.TessellationBaseFactor = Some(0.0);
        data.TessellationFadeDistance = Some(0.0);
        data.GrayscaleToPaletteScale = 1.0;
        data
    }

    #[test]
    fn v2_round_trips_through_the_writer() {
        let bytes = write(&fo4_bgsm());
        assert_eq!(parse(&bytes).unwrap().header.version, 2);
    }

    #[test]
    fn upgrade_writes_v22_pbr_and_round_trips() {
        let src = fo4_bgsm();
        let out = upgrade_bgsm(&src, "SetDressing/Foo/Foo_r.dds", "SetDressing/Foo/Foo_l.dds", false);
        assert_eq!(src.header.version, 2, "source untouched");
        let back = parse(&write(&out)).unwrap();
        assert_eq!(back.header.version, 22);
        assert_eq!(back.PBR, Some(true));
        assert_eq!(back.header.mask_writes, Some(63));
        assert_eq!(back.header.depth_bias, Some(false));
        let clean = |s: &str| s.trim_end_matches('\0').to_string();
        assert_eq!(clean(back.SpecularTexture.as_deref().unwrap()), "SetDressing/Foo/Foo_r.dds");
        assert_eq!(clean(back.LightingTexture.as_deref().unwrap()), "SetDressing/Foo/Foo_l.dds");
        assert_eq!(clean(&back.SmoothSpecTexture), "");
        assert_eq!(clean(&back.DiffuseTexture), "SetDressing/Foo/Foo_d.dds");
        assert_eq!(clean(&back.NormalTexture), "SetDressing/Foo/Foo_n.dds");
        assert_eq!(back.Smoothness, 1.0);
        assert_eq!(back.LumEmittance, Some(100.0));
        assert_eq!(clean(&back.RootMaterialPath), "");
        assert_eq!(back.Translucency, Some(false));
        assert!(back.EnvmapTexture.is_none() || clean(back.EnvmapTexture.as_deref().unwrap()).is_empty());
    }

    #[test]
    fn keeps_non_template_root_and_sets_translucency() {
        let mut src = fo4_bgsm();
        src.RootMaterialPath = "Landscape/Plants/Base.bgsm".into();
        let out = upgrade_bgsm(&src, "r.dds", "l.dds", true);
        assert_eq!(out.RootMaterialPath, "Landscape/Plants/Base.bgsm");
        assert_eq!(out.Translucency, Some(true));
        assert_eq!(out.TranslucencyTurbulence, Some(0.5));
        assert!(is_already_v2x(&out));
        assert!(!is_already_v2x(&src));
    }
}
