// Material conversion: BGSM/BGEM downgrade and output writing.
//
// Params shape (JSON):
// {
//   "materials": [
//     {
//       "source_path":   "Materials/Weapons/Foo.bgsm",  // relative game path
//       "resolved_path": "/abs/path/to/Foo.bgsm",
//       "is_cdb_ref":    false
//     },
//     ...
//   ],
//   "source_game":             "fo76",     // optional, overrides run.source
//   "target_game":             "fo4",      // optional, overrides run.target
//   "asset_prefix":            "fo76",     // accepted for compatibility; output is unprefixed
//   "source_materialsdb":      "/abs/path/to/MaterialsDB.cdb",  // optional
//   "overwrite_existing":      false,
//   "bgsm_default_overrides":  { "bCastShadows": true, ... }    // optional
// }
//
// Output: writes BGSM/BGEM files under `mod_path/data/Materials/...`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Deserialize;
use serde_json::Value as JsonValue;

// ---------------------------------------------------------------------------
// Param types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct MaterialEntry {
    pub source_path: String,
    #[serde(default)]
    pub resolved_path: String,
    #[serde(default)]
    pub is_cdb_ref: bool,
    #[serde(default)]
    pub output_subpath: Option<String>,
    #[serde(default)]
    pub texture_namespace: Option<String>,
    #[serde(default)]
    pub texture_namespace_paths: HashSet<String>,
}

pub struct ConvertMaterialsRequest {
    pub materials: Vec<MaterialEntry>,
    pub source_game: Option<Game>,
    pub target_game: Option<Game>,
    pub asset_prefix: String,
    pub source_materialsdb: Option<PathBuf>,
    pub overwrite_existing: bool,
    pub bgsm_default_overrides: Vec<(String, JsonValue)>,
    pub convert_all: bool,
    pub pbr_carry: bool,
    /// Source-path overrides: normalized material key (lowercase, `/`-separated,
    /// `materials/`-prefixed) -> data-relative replacement source path. Applied to
    /// EVERY converted entry (including the ones `convert_all` enumerates here),
    /// so a placeholder material like `TEMP_GroundTexture01.bgsm` is emitted from
    /// its real replacement (`forestrocks01.bgsm`) no matter which path discovered
    /// it. Populated by the conversion crate from its embedded override table.
    pub source_path_overrides: HashMap<String, String>,
    pub target_asset_paths: HashSet<String>,
}

fn normalize_target_asset_key(value: &str) -> String {
    let normalized = value.replace('\\', "/");
    let trimmed = normalized.trim().trim_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    lower.strip_prefix("data/").unwrap_or(&lower).to_string()
}

impl ConvertMaterialsRequest {
    pub fn from_json(v: &JsonValue) -> Result<Self, String> {
        let materials: Vec<MaterialEntry> = serde_json::from_value(
            v.get("materials")
                .cloned()
                .unwrap_or(JsonValue::Array(vec![])),
        )
        .map_err(|e| format!("materials: {e}"))?;

        let source_game = v
            .get("source_game")
            .and_then(|g| g.as_str())
            .and_then(Game::from_str);
        let target_game = v
            .get("target_game")
            .and_then(|g| g.as_str())
            .and_then(Game::from_str);
        let asset_prefix = v
            .get("asset_prefix")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_owned();
        let source_materialsdb = v
            .get("source_materialsdb")
            .and_then(|p| p.as_str())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        let overwrite_existing = v
            .get("overwrite_existing")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let bgsm_default_overrides: Vec<(String, JsonValue)> = v
            .get("bgsm_default_overrides")
            .and_then(|o| o.as_object())
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        let convert_all = v
            .get("convert_all")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let pbr_carry = v
            .get("pbr_carry")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);

        Ok(ConvertMaterialsRequest {
            materials,
            source_game,
            target_game,
            asset_prefix,
            source_materialsdb,
            overwrite_existing,
            bgsm_default_overrides,
            convert_all,
            pbr_carry,
            source_path_overrides: HashMap::new(),
            target_asset_paths: v
                .get("target_asset_paths")
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
                .filter_map(JsonValue::as_str)
                .map(normalize_target_asset_key)
                .collect(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Game {
    Fo3,
    Fnv,
    Fo4,
    Fo76,
    Skyrim,
    SkyrimSe,
    Starfield,
    Oblivion,
}

impl Game {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "fo3" => Some(Self::Fo3),
            "fnv" => Some(Self::Fnv),
            "fo4" => Some(Self::Fo4),
            "fo76" => Some(Self::Fo76),
            "skyrim" => Some(Self::Skyrim),
            "skyrimse" => Some(Self::SkyrimSe),
            "starfield" => Some(Self::Starfield),
            "oblivion" => Some(Self::Oblivion),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Game material model helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum MaterialModel {
    SpecGloss,     // FO4, SkyrimSE, FO3, FNV, Oblivion, Skyrim
    MetallicRough, // FO76, Starfield
}

fn material_model(game: Game) -> MaterialModel {
    match game {
        Game::Fo76 | Game::Starfield => MaterialModel::MetallicRough,
        _ => MaterialModel::SpecGloss,
    }
}

// ---------------------------------------------------------------------------
// Texture suffix renaming: FO76 → FO4
// ---------------------------------------------------------------------------

/// Rename a texture basename from FO76 suffix conventions to FO4 conventions.
/// Returns a new basename only when a rename occurred; None otherwise.
fn rename_texture_basename_fo76_to_fo4(basename: &str) -> Option<String> {
    let (stem, ext) = if let Some(dot) = basename.rfind('.') {
        (&basename[..dot], &basename[dot..])
    } else {
        (basename, "")
    };
    let stem_lower = stem.to_lowercase();
    // FO76 _r (reflectivity) → FO4 _s (smoothspec/specular)
    if stem_lower.ends_with("_r") {
        return Some(format!("{}_s{}", &stem[..stem.len() - 2], ext));
    }
    // FO76 _l (lighting/emissive rolloff) → FO4 _g (glow)
    if stem_lower.ends_with("_l") {
        return Some(format!("{}_g{}", &stem[..stem.len() - 2], ext));
    }
    None
}

/// Some FO76 actor head materials point texture slots at
/// `Actors/Customization/Character/...`, a directory that ships no matching
/// loose textures. Redirect those stale slot paths to the directories where the
/// texture actually ships or where FO4 already provides the stock head texture.
/// Case-insensitive on the segment; preserves the surrounding path.
fn replace_case_insensitive(haystack: &str, needle: &str, replacement: &str) -> Option<String> {
    let lower = haystack.to_ascii_lowercase();
    let needle_lower = needle.to_ascii_lowercase();
    match lower.find(&needle_lower) {
        Some(idx) => Some(format!(
            "{}{}{}",
            &haystack[..idx],
            replacement,
            &haystack[idx + needle.len()..]
        )),
        None => None,
    }
}

fn repair_stock_character_head_texture_dir(norm: &str) -> String {
    const REPAIRS: &[(&str, &str)] = &[
        (
            "actors/character/basehumanfemale/midagefemalehead_",
            "Actors/Character/MidAgedFemale/MidAgeFemaleHead_",
        ),
        (
            "actors/character/basehumanfemale/oldhumanfemalehead_",
            "Actors/Character/OldHumanFemale/OldHumanFemaleHead_",
        ),
        (
            "actors/character/basehumanmale/oldhumanmalehead_",
            "Actors/Character/OldHumanMale/OldHumanMaleHead_",
        ),
        (
            "actors/character/basehumanmale/mayor_",
            "Actors/Character/Mayor/Mayor_",
        ),
    ];
    let mut repaired = norm.to_owned();
    for (dangling, replacement) in REPAIRS {
        if let Some(next) = replace_case_insensitive(&repaired, dangling, replacement) {
            repaired = next;
        }
    }
    repaired
}

fn repair_dangling_fo76_texture_dir(norm: &str) -> String {
    if let Some(repaired) = replace_case_insensitive(
        norm,
        "actors/customization/character/corpse/",
        "Actors/Corpse/",
    ) {
        return repaired;
    }
    let repaired =
        replace_case_insensitive(norm, "actors/customization/character/", "Actors/Character/")
            .unwrap_or_else(|| norm.to_owned());
    repair_stock_character_head_texture_dir(&repaired)
}

fn rewrite_texture_path_fo76_to_fo4(path: &str) -> String {
    let clean = path.trim_end_matches('\0');
    if clean.is_empty() {
        return path.to_owned();
    }
    let norm = repair_dangling_fo76_texture_dir(&clean.replace('\\', "/"));
    let (dir_part, basename) = if let Some(idx) = norm.rfind('/') {
        (&norm[..=idx], &norm[idx + 1..])
    } else {
        ("", norm.as_str())
    };
    if let Some(new_base) = rename_texture_basename_fo76_to_fo4(basename) {
        return format!("{}{}", dir_part, new_base);
    }
    norm
}

fn material_texture_slot_path(path: &str) -> String {
    let clean = path.trim_end_matches('\0').trim();
    if clean.is_empty() {
        return String::new();
    }
    let mut rel = clean.replace('\\', "/");
    let lower = rel.to_lowercase();
    if lower.starts_with("data/textures/") {
        rel = rel[14..].to_owned();
    } else if lower.starts_with("textures/") || lower.starts_with("data/") {
        rel = rel
            .split_once('/')
            .map(|(_, rest)| rest)
            .unwrap_or("")
            .to_owned();
    }
    strip_known_asset_prefix(&rel)
        .trim_start_matches(|c| c == '/' || c == '\\')
        .replace('\\', "/")
}

fn strip_known_asset_prefix(path: &str) -> &str {
    let mut parts = path.splitn(2, '/');
    let first = parts.next().unwrap_or_default();
    if is_known_asset_prefix(first) {
        parts.next().unwrap_or_default()
    } else {
        path
    }
}

fn is_known_asset_prefix(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "fo4" | "fo76" | "fnv" | "fo3" | "skyrim" | "skyrimse" | "starfield" | "oblivion"
    )
}

fn expected_material_signature(source_path: &str) -> Option<u32> {
    let lower = source_path.to_ascii_lowercase();
    if lower.ends_with(".bgsm") {
        Some(crate::bgsm::BGSM_SIGNATURE)
    } else if lower.ends_with(".bgem") {
        Some(crate::bgem::BGEM_SIGNATURE)
    } else {
        None
    }
}

fn existing_output_matches_signature(
    path: &Path,
    source_path: &str,
    source_game: Game,
    target_game: Game,
) -> bool {
    let Some(expected) = expected_material_signature(source_path) else {
        return true;
    };
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    if bytes.len() < 4 {
        return false;
    }
    if u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) != expected {
        return false;
    }
    if expected == crate::bgsm::BGSM_SIGNATURE
        && source_game == Game::Fo76
        && material_model(target_game) == MaterialModel::SpecGloss
        && suppress_fo76_bgsm_emittance(source_path, BGSM_VERSION_FO4 + 1)
    {
        return crate::bgsm::parse(&bytes)
            .map(|bgsm| !bgsm.EmitEnabled)
            .unwrap_or(false);
    }
    true
}

fn remove_invalid_output(path: &Path) {
    if path.is_file() {
        let _ = fs::remove_file(path);
    }
}

fn json_bool(v: &JsonValue, key: &str, default: bool) -> bool {
    v.get(key).and_then(JsonValue::as_bool).unwrap_or(default)
}

fn json_bool_any(v: &JsonValue, keys: &[&str], default: bool) -> bool {
    keys.iter()
        .find_map(|key| v.get(*key).and_then(JsonValue::as_bool))
        .unwrap_or(default)
}

fn json_f32(v: &JsonValue, key: &str, default: f32) -> f32 {
    v.get(key)
        .and_then(JsonValue::as_f64)
        .map(|value| value as f32)
        .unwrap_or(default)
}

fn json_u8_from_f32(v: &JsonValue, key: &str, default: u8) -> u8 {
    json_f32(v, key, default as f32).clamp(0.0, 255.0).round() as u8
}

fn json_u8(v: &JsonValue, key: &str, default: u8) -> u8 {
    v.get(key)
        .and_then(JsonValue::as_u64)
        .map(|value| value.min(u8::MAX as u64) as u8)
        .unwrap_or(default)
}

fn json_string(v: &JsonValue, key: &str) -> String {
    v.get(key)
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .to_owned()
}

fn json_string_any(v: &JsonValue, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| v.get(*key).and_then(JsonValue::as_str))
        .unwrap_or("")
        .to_owned()
}

fn json_optional_string(v: &JsonValue, key: &str) -> Option<String> {
    Some(json_string(v, key))
}

fn json_color3(v: &JsonValue, key: &str, default: [f32; 3]) -> [f32; 3] {
    let Some(value) = v.get(key) else {
        return default;
    };
    if let Some(items) = value.as_array() {
        if items.len() >= 3 {
            return [
                items[0].as_f64().unwrap_or(default[0] as f64) as f32,
                items[1].as_f64().unwrap_or(default[1] as f64) as f32,
                items[2].as_f64().unwrap_or(default[2] as f64) as f32,
            ];
        }
    }
    let Some(text) = value.as_str() else {
        return default;
    };
    let hex = text.trim().trim_start_matches('#');
    if hex.len() < 6 {
        return default;
    }
    let parse = |range: std::ops::Range<usize>| {
        u8::from_str_radix(&hex[range], 16)
            .map(|component| component as f32 / 255.0)
            .ok()
    };
    match (parse(0..2), parse(2..4), parse(4..6)) {
        (Some(r), Some(g), Some(b)) => [r, g, b],
        _ => default,
    }
}

fn json_alpha_blend_modes(v: &JsonValue) -> (u8, u32, u32) {
    match json_string(v, "eAlphaBlendMode")
        .to_ascii_lowercase()
        .as_str()
    {
        "standard" => (1, 6, 7),
        "additive" => (2, 6, 7),
        "multiplicative" | "multiply" => (3, 6, 7),
        _ => (0, 6, 7),
    }
}

fn json_material_header(v: &JsonValue, signature: u32, version: u32) -> crate::base::BaseHeader {
    let (alpha_blend_mode0, alpha_blend_mode1, alpha_blend_mode2) = json_alpha_blend_modes(v);
    crate::base::BaseHeader {
        signature,
        version,
        tile_u: json_bool(v, "bTileU", false),
        tile_v: json_bool(v, "bTileV", false),
        u_offset: json_f32(v, "fUOffset", 0.0),
        v_offset: json_f32(v, "fVOffset", 0.0),
        u_scale: json_f32(v, "fUScale", 1.0),
        v_scale: json_f32(v, "fVScale", 1.0),
        alpha: json_f32(v, "fAlpha", 1.0),
        alpha_blend_mode0,
        alpha_blend_mode1,
        alpha_blend_mode2,
        alpha_test_ref: json_u8_from_f32(v, "fAlphaTestRef", 128),
        alpha_test: json_bool(v, "bAlphaTest", false),
        zbuffer_write: json_bool(v, "bZBufferWrite", true),
        zbuffer_test: json_bool(v, "bZBufferTest", true),
        ssr: json_bool(v, "bScreenSpaceReflections", false),
        wet_ssr: false,
        decal: json_bool(v, "bDecal", false),
        two_sided: json_bool(v, "bTwoSided", false),
        decal_nofade: json_bool(v, "bDecalNoFade", false),
        non_occluder: json_bool(v, "bNonOccluder", false),
        refraction: json_bool(v, "bRefraction", false),
        refraction_falloff: json_bool(v, "bRefractionFalloff", false),
        refraction_power: json_f32(v, "fRefractionPower", 0.0),
        env_mapping: None,
        env_mapping_mask_scale: None,
        depth_bias: Some(json_bool(v, "bDepthBias", false)),
        grayscale_to_palette_color: json_bool(v, "bGrayscaleToPaletteColor", false),
        mask_writes: Some(0),
    }
}

fn parse_json_bgsm(data: &[u8]) -> Result<crate::bgsm::BgsmData, String> {
    let v: JsonValue = serde_json::from_slice(data).map_err(|e| e.to_string())?;
    if !v.is_object() {
        return Err("expected JSON object".to_owned());
    }
    Ok(crate::bgsm::BgsmData {
        header: json_material_header(&v, crate::bgsm::BGSM_SIGNATURE, 20),
        DiffuseTexture: json_string(&v, "sDiffuseTexture"),
        NormalTexture: json_string(&v, "sNormalTexture"),
        SmoothSpecTexture: json_string(&v, "sSmoothSpecTexture"),
        GreyscaleTexture: json_string(&v, "sGreyscaleTexture"),
        EnvmapTexture: None,
        GlowTexture: json_optional_string(&v, "sGlowTexture"),
        InnerLayerTexture: None,
        WrinklesTexture: json_optional_string(&v, "sWrinklesTexture"),
        DisplacementTexture: None,
        SpecularTexture: json_optional_string(&v, "sSpecularTexture"),
        LightingTexture: json_optional_string(&v, "sLightingTexture"),
        FlowTexture: json_optional_string(&v, "sFlowTexture"),
        DistanceFieldAlphaTexture: json_optional_string(&v, "sDistanceFieldAlphaTexture"),
        EnableEditorAlphaRef: json_bool(&v, "bEnableEditorAlphaRef", false),
        RimLighting: None,
        RimPower: None,
        BackLightPower: None,
        SubsurfaceLighting: None,
        SubsurfaceLightingRolloff: None,
        Translucency: Some(json_bool(&v, "bTranslucency", false)),
        TranslucencyThickObject: Some(json_bool(&v, "bTranslucencyThickObject", false)),
        TranslucencyMixAlbedoWithSubsurfaceColor: Some(json_bool(
            &v,
            "bTranslucencyMixAlbedoWithSubsurfaceColor",
            false,
        )),
        TranslucencySubsurfaceColor: Some(json_color3(
            &v,
            "cTranslucencySubsurfaceColor",
            [1.0, 1.0, 1.0],
        )),
        TranslucencyTransmissiveScale: Some(json_f32(&v, "fTranslucencyTransmissiveScale", 0.0)),
        TranslucencyTurbulence: Some(json_f32(&v, "fTranslucencyTurbulence", 0.0)),
        SpecularEnabled: json_bool_any(&v, &["bSpecularEnabled", "bSpecularEnable"], true),
        SpecularColor: json_color3(&v, "cSpecularColor", [1.0, 1.0, 1.0]),
        SpecularMult: json_f32(&v, "fSpecularMult", 1.0),
        Smoothness: json_f32(&v, "fSmoothness", 0.5),
        FresnelPower: json_f32(&v, "fFresnelPower", 5.0),
        WetnessControlSpecScale: json_f32(&v, "fWetnessControlSpecScale", -1.0),
        WetnessControlSpecPowerScale: json_f32(&v, "fWetnessControlSpecPowerScale", -1.0),
        WetnessControlSpecMinvar: json_f32(&v, "fWetnessControlSpecMinvar", -1.0),
        WetnessControlEnvMapScale: None,
        WetnessControlFresnelPower: json_f32(&v, "fWetnessControlFresnelPower", -1.0),
        WetnessControlMetalness: json_f32(&v, "fWetnessControlMetalness", -1.0),
        PBR: Some(json_bool(&v, "bPBR", false)),
        CustomPorosity: Some(json_bool(&v, "bCustomPorosity", false)),
        PorosityValue: Some(json_f32(&v, "fPorosityValue", 0.0)),
        RootMaterialPath: json_string(&v, "sRootMaterialPath"),
        AnisoLighting: json_bool(&v, "bAnisoLighting", false),
        EmitEnabled: json_bool(&v, "bEmitEnabled", false),
        EmittanceColor: Some(json_color3(&v, "cEmittanceColor", [1.0, 1.0, 1.0])),
        EmittanceMult: json_f32(&v, "fEmittanceMult", 1.0),
        ModelSpaceNormals: json_bool(&v, "bModelSpaceNormals", false),
        ExternalEmittance: json_bool(&v, "bExternalEmittance", false),
        LumEmittance: Some(json_f32(&v, "fLumEmittance", 0.0)),
        UseAdaptativeEmissive: Some(json_bool(&v, "bUseAdaptativeEmissive", false)),
        AdaptativeEmissive_ExposureOffset: Some(json_f32(
            &v,
            "fAdaptativeEmissive_ExposureOffset",
            0.0,
        )),
        AdaptativeEmissive_FinalExposureMin: Some(json_f32(
            &v,
            "fAdaptativeEmissive_FinalExposureMin",
            0.0,
        )),
        AdaptativeEmissive_FinalExposureMax: Some(json_f32(
            &v,
            "fAdaptativeEmissive_FinalExposureMax",
            0.0,
        )),
        BackLighting: None,
        ReceiveShadows: json_bool(&v, "bReceiveShadows", true),
        HideSecret: json_bool(&v, "bHideSecret", false),
        CastShadows: json_bool(&v, "bCastShadows", true),
        DissolveFade: json_bool(&v, "bDissolveFade", false),
        AssumeShadowmask: json_bool(&v, "bAssumeShadowmask", false),
        Glowmap: json_bool(&v, "bGlowmap", false),
        EnvironmentMappingWindow: None,
        EnvironmentMappingEye: None,
        Hair: json_bool(&v, "bHair", false),
        HairTintColor: json_color3(&v, "cHairTintColor", [0.0, 0.0, 0.0]),
        Tree: json_bool(&v, "bTree", false),
        Facegen: json_bool(&v, "bFacegen", false),
        SkinTint: json_bool(&v, "bSkinTint", false),
        Tessellate: json_bool(&v, "bTessellate", false),
        DisplacementTextureBias: None,
        DisplacementTextureScale: None,
        TessellationPnScale: None,
        TessellationBaseFactor: None,
        TessellationFadeDistance: None,
        GrayscaleToPaletteScale: json_f32(&v, "fGrayscaleToPaletteScale", 1.0),
        SkewSpecularAlpha: Some(json_bool(&v, "bSkewSpecularAlpha", false)),
        Terrain: Some(json_bool(&v, "bTerrain", false)),
        UnkInt1: None,
        TerrainThresholdFalloff: Some(json_f32(&v, "fTerrainThresholdFalloff", 0.0)),
        TerrainTilingDistance: Some(json_f32(&v, "fTerrainTilingDistance", 0.0)),
        TerrainRotationAngle: Some(json_f32(&v, "fTerrainRotationAngle", 0.0)),
    })
}

fn parse_json_bgem(data: &[u8]) -> Result<crate::bgem::BgemData, String> {
    let v: JsonValue = serde_json::from_slice(data).map_err(|e| e.to_string())?;
    if !v.is_object() {
        return Err("expected JSON object".to_owned());
    }
    Ok(crate::bgem::BgemData {
        header: json_material_header(&v, crate::bgem::BGEM_SIGNATURE, 20),
        BaseTexture: json_string(&v, "sBaseTexture"),
        GrayscaleTexture: json_string_any(&v, &["sGrayscaleTexture", "sGreyscaleTexture"]),
        EnvmapTexture: json_string(&v, "sEnvmapTexture"),
        NormalTexture: json_string(&v, "sNormalTexture"),
        EnvmapMaskTexture: json_string(&v, "sEnvmapMaskTexture"),
        SpecularTexture: json_optional_string(&v, "sSpecularTexture"),
        LightingTexture: json_optional_string(&v, "sLightingTexture"),
        GlowTexture: json_optional_string(&v, "sGlowTexture"),
        GlassRoughnessScratch: None,
        GlassDirtOverlay: None,
        GlassEnabled: None,
        GlassFresnelColor: None,
        GlassBlurScaleBase: None,
        GlassBlurScaleFactor: None,
        GlassRefractionScaleBase: None,
        EnvironmentMapping: Some(json_bool(&v, "bEnvironmentMapping", false)),
        EnvironmentMappingMaskScale: Some(json_f32(&v, "fEnvironmentMappingMaskScale", 1.0)),
        BloodEnabled: json_bool(&v, "bBloodEnabled", false),
        EffectLightingEnabled: json_bool(&v, "bEffectLightingEnabled", false),
        FalloffEnabled: json_bool(&v, "bFalloffEnabled", false),
        FalloffColorEnabled: json_bool(&v, "bFalloffColorEnabled", false),
        GrayscaleToPaletteAlpha: json_bool(&v, "bGrayscaleToPaletteAlpha", false),
        SoftEnabled: json_bool(&v, "bSoftEnabled", false),
        BaseColor: json_color3(&v, "cBaseColor", [1.0, 1.0, 1.0]),
        BaseColorScale: json_f32(&v, "fBaseColorScale", 1.0),
        FalloffStartAngle: json_f32(&v, "fFalloffStartAngle", 0.0),
        FalloffStopAngle: json_f32(&v, "fFalloffStopAngle", 0.0),
        FalloffStartOpacity: json_f32(&v, "fFalloffStartOpacity", 1.0),
        FalloffStopOpacity: json_f32(&v, "fFalloffStopOpacity", 0.0),
        LightingInfluence: json_f32(&v, "fLightingInfluence", 1.0),
        EnvmapMinLOD: json_u8(&v, "iEnvmapMinLOD", 0),
        SoftDepth: json_f32(&v, "fSoftDepth", 100.0),
        EmittanceColor: Some(json_color3(&v, "cEmittanceColor", [1.0, 1.0, 1.0])),
        AdaptativeEmissive_ExposureOffset: Some(json_f32(
            &v,
            "fAdaptativeEmissive_ExposureOffset",
            0.0,
        )),
        AdaptativeEmissive_FinalExposureMin: Some(json_f32(
            &v,
            "fAdaptativeEmissive_FinalExposureMin",
            0.0,
        )),
        AdaptativeEmissive_FinalExposureMax: Some(json_f32(
            &v,
            "fAdaptativeEmissive_FinalExposureMax",
            0.0,
        )),
        Glowmap: Some(json_bool(&v, "bGlowmap", false)),
        EffectPbrSpecular: Some(json_bool(&v, "bEffectPbrSpecular", false)),
    })
}

// ---------------------------------------------------------------------------
// Cubemap heuristic (simplified port of cubemap_heuristics.py)
// ---------------------------------------------------------------------------

const DEFAULT_OUTSIDE: &str = "Shared/Cubemaps/mipblur_DefaultOutside1.dds";
const DEFAULT_DIELECTRIC: &str = "Shared/Cubemaps/mipblur_DefaultOutside1_dielectric.dds";
const CUBEMAP_COPPER: &str = "Shared/Cubemaps/Copper_e.dds";
const ORE_CORUNDUM: &str = "Shared/Cubemaps/Ore_Corun_e.dds";
const ORE_EBONY: &str = "Shared/Cubemaps/Ore_Ebony_e.dds";
const ORE_GOLD: &str = "Shared/Cubemaps/Ore_Gold_e.dds";
const ORE_IRON: &str = "Shared/Cubemaps/Ore_Iron_e.dds";
const ORE_MOONSTONE: &str = "Shared/Cubemaps/Ore_Moonstone_e.dds";
const ORE_OBSIDIAN: &str = "Shared/Cubemaps/Ore_Obsidian_e.dds";
const ORE_ORICHALCUM: &str = "Shared/Cubemaps/Ore_Orich_e.dds";
const ORE_QUICKSILVER: &str = "Shared/Cubemaps/Ore_Quicksilver_e.dds";
const ORE_SILVER: &str = "Shared/Cubemaps/Ore_Silver_e.dds";
const ORE_STEEL: &str = "Shared/Cubemaps/Ore_Steel_e.dds";

// Tinted / metal cubemaps for named non-ore metals (soft outdoor variants so
// world statics reflect a blurred sky, not a sharp mirror).
const OUT_BRONZE: &str = "Shared/Cubemaps/mipblur_DefaultOutside1_bronze.dds";
const OUT_COPPER: &str = "Shared/Cubemaps/mipblur_DefaultOutside1_copper.dds";
const METAL_CHROME: &str = "Shared/Cubemaps/MetalChrome01Cube_e.dds";

// Basename-matched hints for generic (untinted) metal. Matched against the
// filename only — folder names like "Iron_Mountain", "carnival", or
// "quonsethutINTfloor" would otherwise false-match on the full path. A false
// positive here is mild (a subtle 0.3 reflection, never full chrome).
const GENERIC_METAL_HINTS: &[&str] = &[
    "metal",
    "steel",
    "iron",
    "rebar",
    "girder",
    "guardrail",
    "railing",
    "railroad",
    "monorail",
    "scaffold",
    "ductwork",
    "pipe",
    "sheetmetal",
    "corrugated",
    "chainlink",
    "rusted",
    "rusty",
    "rust",
    "aluminum",
    "aluminium",
    "traincar",
    "boxcar",
    "locomotive",
    "flatbed",
    "trailer",
    "truck",
    "tanker",
    "hubcap",
    "wroughtiron",
    "fence",
    "gutter",
    "manhole",
];

fn source_path_uses_ore_material(path: &str) -> bool {
    path_has_any(
        path,
        &[
            "/ore/",
            "/ingotandore/",
            "/minerals/",
            "mineral_",
            "mineralwall_",
            "irradiatedore",
            "irradiated_ore",
        ],
    )
}

fn select_ore_cubemap(path: &str) -> Option<&'static str> {
    if !source_path_uses_ore_material(path) {
        return None;
    }
    if path_has_any(path, &["gold"]) {
        return Some(ORE_GOLD);
    }
    if path_has_any(path, &["silver"]) {
        return Some(ORE_SILVER);
    }
    if path_has_any(path, &["copper"]) {
        return Some(CUBEMAP_COPPER);
    }
    if path_has_any(path, &["iron"]) {
        return Some(ORE_IRON);
    }
    if path_has_any(path, &["steel"]) {
        return Some(ORE_STEEL);
    }
    if path_has_any(path, &["ebony"]) {
        return Some(ORE_EBONY);
    }
    if path_has_any(path, &["obsidian", "blacktitanium", "coal"]) {
        return Some(ORE_OBSIDIAN);
    }
    if path_has_any(path, &["moonstone"]) {
        return Some(ORE_MOONSTONE);
    }
    if path_has_any(path, &["orichalcum", "orich"]) {
        return Some(ORE_ORICHALCUM);
    }
    if path_has_any(path, &["quicksilver"]) {
        return Some(ORE_QUICKSILVER);
    }
    if path_has_any(path, &["corundum", "corun"]) {
        return Some(ORE_CORUNDUM);
    }
    Some(ORE_STEEL)
}

// Cubemap selection for BGSM (lit) materials. Environment mapping is OPT-IN:
// only metal-bearing surfaces get a cubemap. Everything else — rock, concrete,
// wood, plastic, ceramic, cloth, toys — returns None so it is not forced to
// reflect the sky (which reads as chrome). FO4 vanilla likewise leaves the vast
// majority of world/structural/dielectric materials with env mapping off.
fn select_cubemap(source_path: &str) -> Option<(&'static str, f32)> {
    let lower = source_path.to_lowercase().replace('\\', "/");
    // Exclusions: no cubemap for effects / UI / sky / decals.
    for seg in &["effects/", "interface/", "menu/", "sky/", "decals/"] {
        if lower.contains(seg) {
            return None;
        }
    }
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    if let Some(cubemap) = select_ore_cubemap(&lower) {
        return Some((cubemap, 1.0));
    }
    // Weapons / armor are genuinely metal — keep them fully reflective.
    if lower.contains("/weapons/") || base.contains("weapon") {
        return Some((DEFAULT_OUTSIDE, 1.0));
    }
    if lower.contains("/armor/") || base.contains("armor") {
        return Some((DEFAULT_OUTSIDE, 1.0));
    }
    if lower.contains("/actors/") || base.contains("creature") {
        return Some((DEFAULT_DIELECTRIC, 0.3));
    }
    if base.contains("carpet") {
        return Some((DEFAULT_DIELECTRIC, 0.3));
    }
    // Named tinted metals → matching soft outdoor cubemap.
    if base.contains("bronze") || base.contains("brass") {
        return Some((OUT_BRONZE, 0.5));
    }
    if base.contains("copper") {
        return Some((OUT_COPPER, 0.5));
    }
    if base.contains("chrome") {
        return Some((METAL_CHROME, 0.5));
    }
    // Generic metal (incl. vehicles) → subtle outdoor reflection.
    if lower.contains("/vehicles/") || GENERIC_METAL_HINTS.iter().any(|h| base.contains(h)) {
        return Some((DEFAULT_OUTSIDE, 0.3));
    }
    // Dielectric / matte (rock, concrete, wood, plastic, toys, …) → no cubemap.
    None
}

// Cubemap selection for BGEM (effect / glass) materials. Effect shaders such as
// glass legitimately want a reflection, so BGEM keeps the legacy always-on
// default for anything the hard exclusions don't drop — only the metal-specific
// tinting from `select_cubemap` is layered on top.
fn select_cubemap_bgem(source_path: &str) -> Option<(&'static str, f32)> {
    let lower = source_path.to_lowercase().replace('\\', "/");
    for seg in &["effects/", "interface/", "menu/", "sky/", "decals/"] {
        if lower.contains(seg) {
            return None;
        }
    }
    select_cubemap(source_path).or(Some((DEFAULT_OUTSIDE, 1.0)))
}

// ---------------------------------------------------------------------------
// RootMaterialPath synthesis (heuristic)
// ---------------------------------------------------------------------------

fn path_has_any(path: &str, hints: &[&str]) -> bool {
    hints.iter().any(|hint| path.contains(hint))
}

fn structural_root_material_template(path: &str) -> &'static str {
    if path_has_any(path, &["basicrough", "roughbasic"]) {
        return "template/basicrough.bgsm";
    }
    if path_has_any(path, &["basicsmooth", "smoothbasic"]) {
        return "template/basicsmooth.bgsm";
    }
    if path_has_any(path, &["defaultdim", " dim", "_dim", "dark"]) {
        return "template/DefaultTemplateDim_Wet.bgsm";
    }
    if path_has_any(path, &["vehiclebus", "/bus/", "bus_", "bus."]) {
        return "template/VehicleBusTemplate_Wet.bgsm";
    }
    if path_has_any(
        path,
        &["vehicle", "/vehicles/", "/cars/", "/truck", "/car/"],
    ) {
        return "template/VehicleTemplate_Wet.bgsm";
    }
    if path_has_any(path, &["capmetal", "capdetail", "/capsules/", "capsule"]) {
        return "template/CapMetalTemplate_Wet.bgsm";
    }
    if path_has_any(
        path,
        &["wrought", "railing", "standpipe", "streetlamp", "ironfence"],
    ) {
        return "template/WroughtIronMetalTemplate_Wet.bgsm";
    }
    if path_has_any(path, &["chrome", "polishedmetal"]) {
        return "template/MetalTemplate_Chrome.bgsm";
    }
    if path_has_any(path, &["brushed", "baremetal"]) {
        return "template/MetalTemplate_Brushed.bgsm";
    }
    if path_has_any(
        path,
        &["asphalt", "ashphalt", "parking", "road", "pavement"],
    ) {
        return "template/AsphaltTemplate_Wet.bgsm";
    }
    if path_has_any(path, &["crackedmud", "cracked_mud"]) {
        return "template/CrackedMudTemplate_Wet.bgsm";
    }
    if path_has_any(path, &["rockslab", "rock_slab", "fakerockslab"]) {
        return "template/RockSlabTemplate_Wet.bgsm";
    }
    if path_has_any(path, &["rock", "rocks/", "stone", "cliff", "boulder"]) {
        return "template/RockTemplate_Wet.bgsm";
    }
    if path_has_any(path, &["dirt", "mud", "soil", "gravel", "forestfloor"]) {
        return "template/DirtTemplate_Wet.bgsm";
    }
    if path_has_any(path, &["rubber", "tire", "tyre"]) {
        return "template/RubberTemplate_Wet.bgsm";
    }
    if path_has_any(path, &["felt", "hide", "leatherhide"]) {
        return "template/ClothTemplate_Felt_Wet.bgsm";
    }
    if path_has_any(path, &["wool"]) {
        return "template/ClothTemplate_Wool_Wet.bgsm";
    }
    if path_has_any(
        path,
        &["cloth", "fabric", "awning", "canvas", "cotton", "tarp"],
    ) {
        return "template/ClothTemplate_Cotton_Wet.bgsm";
    }
    if path_has_any(path, &["wood", "timber", "log", "bark", "stump"]) {
        return "template/WoodTemplate_Wet.bgsm";
    }
    if path_has_any(path, &["metal", "steel", "iron", "aluminum", "aluminium"]) {
        return "template/MetalTemplate_Wet.bgsm";
    }
    if path_has_any(path, &["brick"]) {
        return "template/BrickTemplate_Wet.bgsm";
    }
    if path_has_any(path, &["concrete", "cement", "plaster", "tile", "linoleum"]) {
        return "template/ConcreteTemplate_Wet.bgsm";
    }
    "template/defaultTemplate_wet.bgsm"
}

fn synthesize_root_material_path(
    source_path: &str,
    hair: bool,
    tree: bool,
    skin_tint: bool,
) -> String {
    let lower = source_path.to_lowercase().replace('\\', "/");
    if hair {
        return "template/defaultTemplate_wet.bgsm".to_owned();
    }
    if skin_tint {
        return "template/SkinTemplate_Wet.bgsm".to_owned();
    }
    if source_path_uses_ore_material(&lower) {
        return "template/defaultTemplate_wet.bgsm".to_owned();
    }
    if lower.contains("landscape/grass/") {
        return "Template/GrassTemplate_Wet.BGSM".to_owned();
    }
    if tree || lower.contains("landscape/plants/") || lower.contains("landscape/trees/") {
        return "Template/LeafTemplate_Wet.bgsm".to_owned();
    }

    let fname = lower.rsplit('/').next().unwrap_or(&lower);

    if lower.contains("/weapons/") || lower.contains("weapon") {
        if fname.contains("wood")
            || fname.contains("stock")
            || fname.contains("grip")
            || fname.contains("handle")
        {
            return "template/WeaponWoodTemplate_Wet.bgsm".to_owned();
        }
        if fname.contains("plastic") || fname.contains("polymer") || fname.contains("rubber") {
            return "template/WeaponPlasticTemplate_Wet.bgsm".to_owned();
        }
        return "template/WeaponMetalTemplate_Wet.bgsm".to_owned();
    }
    if lower.contains("/armor/") || lower.contains("armor") {
        return "template/ArmorTemplate_Wet.bgsm".to_owned();
    }
    if lower.contains("/clothes/") || lower.contains("/clothing/") {
        return "template/OutfitTemplate_Wet.bgsm".to_owned();
    }
    if lower.contains("/actors/") || lower.contains("creature") {
        return "template/CreatureTemplate_Wet.bgsm".to_owned();
    }
    let structural_cats = [
        "/architecture/",
        "/setdressing/",
        "/landscape/",
        "/construction/",
        "/furniture/",
        "/props/",
        "/vehicles/",
    ];
    if structural_cats.iter().any(|seg| lower.contains(seg)) {
        return structural_root_material_template(&lower).to_owned();
    }
    "template/defaultTemplate_wet.bgsm".to_owned()
}

fn source_path_uses_empty_root_material(source_path: &str) -> bool {
    source_path
        .to_lowercase()
        .replace('\\', "/")
        .contains("carpet")
}

fn apply_vegetation_material_defaults(bgsm: &mut crate::bgsm::BgsmData) {
    let root = bgsm
        .RootMaterialPath
        .replace('\0', "")
        .trim()
        .to_ascii_lowercase();
    if root.contains("leaftemplate_wet") {
        bgsm.BackLighting = Some(true);
        bgsm.BackLightPower = Some(0.25);
        bgsm.SubsurfaceLighting = Some(true);
        bgsm.SubsurfaceLightingRolloff = Some(2.0);
        bgsm.EnvmapTexture = Some(String::new());
        bgsm.header.env_mapping = Some(false);
    } else if root.contains("grasstemplate_wet") {
        bgsm.SubsurfaceLighting = Some(true);
        bgsm.EnvmapTexture = Some(String::new());
        bgsm.header.env_mapping = Some(false);
    }
}

// ---------------------------------------------------------------------------
// BGSM downgrade: FO76 (v>2) → FO4 (v2)
// ---------------------------------------------------------------------------

const BGSM_VERSION_FO4: u32 = 2;
const BGEM_VERSION_FO4: u32 = 2;

pub fn downgrade_bgsm(
    mut bgsm: crate::bgsm::BgsmData,
    source_path: &str,
    source_game: Game,
    target_game: Game,
) -> crate::bgsm::BgsmData {
    if material_model(target_game) != MaterialModel::SpecGloss {
        return bgsm;
    }
    if bgsm.header.version <= BGSM_VERSION_FO4 {
        normalize_bgsm_texture_slots(&mut bgsm);
        return bgsm;
    }
    let src_v = bgsm.header.version;

    // ---- Texture slot remapping (v>2 → v2) ----
    if src_v > 2 {
        let clean_s = |s: &str| s.replace('\0', "").trim().to_owned();
        let clean_opt = |s: Option<&str>| s.unwrap_or("").replace('\0', "").trim().to_owned();

        let smoothspec_clean = clean_s(&bgsm.SmoothSpecTexture);
        let specular_clean = clean_opt(bgsm.SpecularTexture.as_deref());

        // Promote FO76 SpecularTexture (PBR roughness) → FO4 SmoothSpecTexture only if empty.
        bgsm.SmoothSpecTexture = if smoothspec_clean.is_empty() && !specular_clean.is_empty() {
            rewrite_texture_path_fo76_to_fo4(&specular_clean)
        } else {
            smoothspec_clean
        };

        let lighting_clean = clean_opt(bgsm.LightingTexture.as_deref());
        let glow_clean = clean_opt(bgsm.GlowTexture.as_deref());
        if suppress_fo76_bgsm_emittance(source_path, src_v) && bgsm.EmitEnabled {
            bgsm.EmitEnabled = false;
            bgsm.Glowmap = false;
            bgsm.GlowTexture = None;
            bgsm.EmittanceColor = None;
            bgsm.EmittanceMult = 1.0;
            bgsm.ExternalEmittance = false;
        } else if bgsm.EmitEnabled && !lighting_clean.is_empty() && glow_clean.is_empty() {
            bgsm.GlowTexture = Some(rewrite_texture_path_fo76_to_fo4(&lighting_clean));
            bgsm.Glowmap = true;
            if bgsm.EmittanceMult > 1.0 {
                bgsm.EmittanceMult = 1.0;
            }
        }

        // Inject cubemap heuristic (FO76 has no EnvmapTexture slot).
        let cubemap = select_cubemap(source_path);
        bgsm.EnvmapTexture = Some(cubemap.map(|(c, _)| c.to_owned()).unwrap_or_default());
        bgsm.header.env_mapping = Some(cubemap.is_some());
        bgsm.header.env_mapping_mask_scale = Some(cubemap.map(|(_, scale)| scale).unwrap_or(1.0));
        bgsm.InnerLayerTexture = Some(String::new());
        bgsm.DisplacementTexture = Some(String::new());
    }

    // ---- Lighting block: Translucency (v>=8) → RimLighting ----
    if src_v >= 8 {
        let had_translucency = bgsm.Translucency.unwrap_or(false);
        bgsm.RimLighting = Some(false);
        bgsm.RimPower = Some(2.0);
        bgsm.BackLightPower = Some(0.0);
        bgsm.SubsurfaceLighting = Some(had_translucency);
        bgsm.SubsurfaceLightingRolloff = Some(bgsm.TranslucencyTransmissiveScale.unwrap_or(0.3));
    }

    // ---- Clear FO76-only fields ----
    // PBR texture slots (not present at v<=2):
    bgsm.SpecularTexture = None;
    bgsm.LightingTexture = None;
    bgsm.FlowTexture = None;
    bgsm.DistanceFieldAlphaTexture = None;
    // Translucency block (v<8):
    bgsm.Translucency = None;
    bgsm.TranslucencyThickObject = None;
    bgsm.TranslucencyMixAlbedoWithSubsurfaceColor = None;
    bgsm.TranslucencySubsurfaceColor = None;
    bgsm.TranslucencyTransmissiveScale = None;
    bgsm.TranslucencyTurbulence = None;

    // BackLighting field (present in v<8 only).
    if bgsm.BackLighting.is_none() {
        bgsm.BackLighting = Some(false);
    }

    // WetnessControlEnvMapScale: removed at v>=10, needed at v<10. The other
    // Wetness fields use -1.0 as the "disabled" sentinel — match that so the
    // downgraded BGSM is consistent with vanilla FO4 (which writes -1.0 across
    // the wetness block when wetness is disabled).
    if src_v >= 10 && bgsm.WetnessControlEnvMapScale.is_none() {
        bgsm.WetnessControlEnvMapScale = Some(-1.0);
    }

    // env_mapping / env_mapping_mask_scale: removed at header v>=10, needed at
    // v<10. FO4 vanilla writes mask_scale=1.0 even when env_mapping is off;
    // 0.0 is treated as "fully masked out" by the shader and changes the look.
    if bgsm.header.env_mapping.is_none() {
        bgsm.header.env_mapping = Some(false);
    }
    if bgsm.header.env_mapping_mask_scale.is_none() {
        bgsm.header.env_mapping_mask_scale = Some(1.0);
    }

    // RootMaterialPath synthesis.
    let current_root = bgsm.RootMaterialPath.replace('\0', "").trim().to_owned();
    if source_path_uses_empty_root_material(source_path) {
        bgsm.RootMaterialPath.clear();
    } else if current_root.is_empty() {
        bgsm.RootMaterialPath =
            synthesize_root_material_path(source_path, bgsm.Hair, bgsm.Tree, bgsm.SkinTint);
    }
    apply_vegetation_material_defaults(&mut bgsm);

    // Rewrite remaining texture paths to FO4 naming if source is FO76.
    if source_game == Game::Fo76 {
        bgsm.DiffuseTexture = rewrite_texture_path_fo76_to_fo4(&bgsm.DiffuseTexture);
        bgsm.NormalTexture = rewrite_texture_path_fo76_to_fo4(&bgsm.NormalTexture);
        bgsm.GreyscaleTexture = rewrite_texture_path_fo76_to_fo4(&bgsm.GreyscaleTexture);
        if let Some(t) = bgsm.GlowTexture.take() {
            bgsm.GlowTexture = Some(rewrite_texture_path_fo76_to_fo4(&t));
        }
        if let Some(t) = bgsm.WrinklesTexture.take() {
            bgsm.WrinklesTexture = Some(rewrite_texture_path_fo76_to_fo4(&t));
        }
    }

    normalize_bgsm_texture_slots(&mut bgsm);

    bgsm.header.version = BGSM_VERSION_FO4;
    bgsm
}

/// Whether the FO4 downgrade of this source BGSM keeps or synthesizes a glow map.
/// FO76 static/object BGSM emittance is intentionally suppressed because FO4
/// applies it across the whole surface; effect/decal material paths keep their
/// emissive behavior.
pub fn source_bgsm_enables_fo4_glowmap(bgsm: &crate::bgsm::BgsmData, source_path: &str) -> bool {
    if suppress_fo76_bgsm_emittance(source_path, bgsm.header.version) {
        return false;
    }
    if bgsm.Glowmap {
        return true;
    }
    let has_text = |slot: &Option<String>| {
        slot.as_deref()
            .is_some_and(|t| !t.replace('\0', "").trim().is_empty())
    };
    bgsm.EmitEnabled && has_text(&bgsm.LightingTexture) && !has_text(&bgsm.GlowTexture)
}

fn suppress_fo76_bgsm_emittance(source_path: &str, source_version: u32) -> bool {
    if source_version <= BGSM_VERSION_FO4 {
        return false;
    }
    let path = source_path
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase();
    let relative = path.strip_prefix("materials/").unwrap_or(&path);
    !(relative.starts_with("effects/") || relative.starts_with("decals/"))
}

fn normalize_bgsm_texture_slots(bgsm: &mut crate::bgsm::BgsmData) {
    bgsm.DiffuseTexture = material_texture_slot_path(&bgsm.DiffuseTexture);
    bgsm.NormalTexture = material_texture_slot_path(&bgsm.NormalTexture);
    bgsm.SmoothSpecTexture = material_texture_slot_path(&bgsm.SmoothSpecTexture);
    bgsm.GreyscaleTexture = material_texture_slot_path(&bgsm.GreyscaleTexture);
    if let Some(t) = bgsm.GlowTexture.take() {
        bgsm.GlowTexture = Some(material_texture_slot_path(&t));
    }
    if let Some(t) = bgsm.EnvmapTexture.take() {
        bgsm.EnvmapTexture = Some(material_texture_slot_path(&t));
    }
    if let Some(t) = bgsm.WrinklesTexture.take() {
        bgsm.WrinklesTexture = Some(material_texture_slot_path(&t));
    }
}

pub fn repair_missing_fo76_smoothspec_from_specular(
    bgsm: &mut crate::bgsm::BgsmData,
    resolved_material_path: &Path,
    source_game: Game,
    target_game: Game,
) {
    if source_game != Game::Fo76 || target_game != Game::Fo4 {
        return;
    }
    let smooth = bgsm.SmoothSpecTexture.replace('\0', "").trim().to_owned();
    if smooth.is_empty() || texture_ref_exists_for_material(resolved_material_path, &smooth) {
        return;
    }
    let Some(specular) = bgsm
        .SpecularTexture
        .as_deref()
        .map(|value| value.replace('\0', "").trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    if !texture_ref_exists_for_material(resolved_material_path, &specular) {
        return;
    }
    bgsm.SmoothSpecTexture = rewrite_texture_path_fo76_to_fo4(&specular);
}

fn texture_ref_exists_for_material(resolved_material_path: &Path, texture_ref: &str) -> bool {
    let slot = material_texture_slot_path(texture_ref);
    if slot.is_empty() {
        return false;
    }
    let Some(source_root) = source_root_for_resolved_material(resolved_material_path) else {
        return false;
    };
    source_root
        .join("textures")
        .join(slot.replace('/', std::path::MAIN_SEPARATOR_STR))
        .is_file()
}

fn source_root_for_resolved_material(resolved_material_path: &Path) -> Option<PathBuf> {
    for ancestor in resolved_material_path.ancestors() {
        if ancestor
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("materials"))
        {
            return ancestor.parent().map(Path::to_path_buf);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// BGEM downgrade: FO76 (v>2) → FO4 (v2)
// ---------------------------------------------------------------------------

pub fn downgrade_bgem(
    mut bgem: crate::bgem::BgemData,
    source_path: &str,
    source_game: Game,
    target_game: Game,
) -> crate::bgem::BgemData {
    if material_model(target_game) != MaterialModel::SpecGloss {
        return bgem;
    }
    if bgem.header.version <= BGEM_VERSION_FO4 {
        normalize_bgem_texture_slots(&mut bgem);
        return bgem;
    }

    // Clear FO76-only glass block fields (v >= 21).
    bgem.GlassRoughnessScratch = None;
    bgem.GlassDirtOverlay = None;
    bgem.GlassEnabled = None;
    bgem.GlassFresnelColor = None;
    bgem.GlassBlurScaleBase = None;
    bgem.GlassBlurScaleFactor = None;
    bgem.GlassRefractionScaleBase = None;

    // Inject cubemap heuristic when slot is empty.
    if let Some((cubemap, scale)) = select_cubemap_bgem(source_path) {
        let existing = bgem.EnvmapTexture.replace('\0', "").trim().to_owned();
        if existing.is_empty() {
            bgem.EnvmapTexture = cubemap.to_owned();
        }
        if bgem.EnvironmentMapping != Some(true) {
            bgem.EnvironmentMapping = Some(true);
        }
        if bgem.EnvironmentMappingMaskScale.unwrap_or(0.0) == 0.0 {
            bgem.EnvironmentMappingMaskScale = Some(scale);
        }
    }

    // Rewrite texture suffixes for FO76 → FO4.
    if source_game == Game::Fo76 {
        bgem.BaseTexture = rewrite_texture_path_fo76_to_fo4(&bgem.BaseTexture);
        bgem.GrayscaleTexture = rewrite_texture_path_fo76_to_fo4(&bgem.GrayscaleTexture);
        bgem.NormalTexture = rewrite_texture_path_fo76_to_fo4(&bgem.NormalTexture);
        bgem.EnvmapMaskTexture = rewrite_texture_path_fo76_to_fo4(&bgem.EnvmapMaskTexture);
        if let Some(t) = bgem.SpecularTexture.take() {
            bgem.SpecularTexture = Some(rewrite_texture_path_fo76_to_fo4(&t));
        }
        if let Some(t) = bgem.LightingTexture.take() {
            bgem.LightingTexture = Some(rewrite_texture_path_fo76_to_fo4(&t));
        }
        if let Some(t) = bgem.GlowTexture.take() {
            bgem.GlowTexture = Some(rewrite_texture_path_fo76_to_fo4(&t));
        }
    }

    normalize_bgem_texture_slots(&mut bgem);

    bgem.header.version = BGEM_VERSION_FO4;
    bgem
}

fn normalize_bgem_texture_slots(bgem: &mut crate::bgem::BgemData) {
    bgem.BaseTexture = material_texture_slot_path(&bgem.BaseTexture);
    bgem.GrayscaleTexture = material_texture_slot_path(&bgem.GrayscaleTexture);
    bgem.EnvmapTexture = material_texture_slot_path(&bgem.EnvmapTexture);
    bgem.NormalTexture = material_texture_slot_path(&bgem.NormalTexture);
    bgem.EnvmapMaskTexture = material_texture_slot_path(&bgem.EnvmapMaskTexture);
    if let Some(t) = bgem.SpecularTexture.take() {
        bgem.SpecularTexture = Some(material_texture_slot_path(&t));
    }
    if let Some(t) = bgem.LightingTexture.take() {
        bgem.LightingTexture = Some(material_texture_slot_path(&t));
    }
    if let Some(t) = bgem.GlowTexture.take() {
        bgem.GlowTexture = Some(material_texture_slot_path(&t));
    }
}

fn namespace_texture_slot_path(path: &str, namespace: &str) -> String {
    if path.trim_end_matches('\0').trim().is_empty() {
        return String::new();
    }
    let namespace = namespace.trim().trim_matches(|c| c == '/' || c == '\\');
    if namespace.is_empty() {
        return path.to_owned();
    }
    let mut slot = material_texture_slot_path(path);
    let lower = slot.to_ascii_lowercase();
    let namespace_lower = namespace.to_ascii_lowercase();
    if is_shared_cubemap_texture_slot(&lower) {
        return slot;
    }
    if lower == namespace_lower || lower.starts_with(&format!("{namespace_lower}/")) {
        return slot;
    }
    slot = slot.trim_start_matches('/').to_owned();
    if slot.is_empty() {
        return slot;
    }
    format!("{namespace}/{slot}")
}

fn is_shared_cubemap_texture_slot(lower_slot: &str) -> bool {
    lower_slot
        .trim_start_matches(|c| c == '/' || c == '\\')
        .replace('\\', "/")
        .starts_with("shared/cubemaps/")
}

fn texture_namespace_key(path: &str) -> Option<String> {
    let slot = material_texture_slot_path(path);
    (!slot.is_empty()).then(|| format!("textures/{}", slot.to_ascii_lowercase()))
}

fn should_namespace_texture_slot(path: &str, texture_namespace_paths: &HashSet<String>) -> bool {
    texture_namespace_paths.is_empty()
        || texture_namespace_key(path).is_some_and(|key| texture_namespace_paths.contains(&key))
}

fn namespace_texture_slot_path_if_selected(
    path: &str,
    namespace: &str,
    texture_namespace_paths: &HashSet<String>,
) -> String {
    if should_namespace_texture_slot(path, texture_namespace_paths) {
        namespace_texture_slot_path(path, namespace)
    } else {
        material_texture_slot_path(path)
    }
}

fn namespace_bgsm_texture_slots(
    bgsm: &mut crate::bgsm::BgsmData,
    namespace: Option<&str>,
    texture_namespace_paths: &HashSet<String>,
) {
    let Some(namespace) = namespace.filter(|value| !value.trim().is_empty()) else {
        return;
    };
    bgsm.DiffuseTexture = namespace_texture_slot_path_if_selected(
        &bgsm.DiffuseTexture,
        namespace,
        texture_namespace_paths,
    );
    bgsm.NormalTexture = namespace_texture_slot_path_if_selected(
        &bgsm.NormalTexture,
        namespace,
        texture_namespace_paths,
    );
    bgsm.SmoothSpecTexture = namespace_texture_slot_path_if_selected(
        &bgsm.SmoothSpecTexture,
        namespace,
        texture_namespace_paths,
    );
    bgsm.GreyscaleTexture = namespace_texture_slot_path_if_selected(
        &bgsm.GreyscaleTexture,
        namespace,
        texture_namespace_paths,
    );
    if let Some(t) = bgsm.GlowTexture.take() {
        bgsm.GlowTexture = Some(namespace_texture_slot_path_if_selected(
            &t,
            namespace,
            texture_namespace_paths,
        ));
    }
    if let Some(t) = bgsm.EnvmapTexture.take() {
        bgsm.EnvmapTexture = Some(namespace_texture_slot_path_if_selected(
            &t,
            namespace,
            texture_namespace_paths,
        ));
    }
    if let Some(t) = bgsm.WrinklesTexture.take() {
        bgsm.WrinklesTexture = Some(namespace_texture_slot_path_if_selected(
            &t,
            namespace,
            texture_namespace_paths,
        ));
    }
}

fn namespace_bgem_texture_slots(
    bgem: &mut crate::bgem::BgemData,
    namespace: Option<&str>,
    texture_namespace_paths: &HashSet<String>,
) {
    let Some(namespace) = namespace.filter(|value| !value.trim().is_empty()) else {
        return;
    };
    bgem.BaseTexture = namespace_texture_slot_path_if_selected(
        &bgem.BaseTexture,
        namespace,
        texture_namespace_paths,
    );
    bgem.GrayscaleTexture = namespace_texture_slot_path_if_selected(
        &bgem.GrayscaleTexture,
        namespace,
        texture_namespace_paths,
    );
    bgem.EnvmapTexture = namespace_texture_slot_path_if_selected(
        &bgem.EnvmapTexture,
        namespace,
        texture_namespace_paths,
    );
    bgem.NormalTexture = namespace_texture_slot_path_if_selected(
        &bgem.NormalTexture,
        namespace,
        texture_namespace_paths,
    );
    bgem.EnvmapMaskTexture = namespace_texture_slot_path_if_selected(
        &bgem.EnvmapMaskTexture,
        namespace,
        texture_namespace_paths,
    );
    if let Some(t) = bgem.SpecularTexture.take() {
        bgem.SpecularTexture = Some(namespace_texture_slot_path_if_selected(
            &t,
            namespace,
            texture_namespace_paths,
        ));
    }
    if let Some(t) = bgem.LightingTexture.take() {
        bgem.LightingTexture = Some(namespace_texture_slot_path_if_selected(
            &t,
            namespace,
            texture_namespace_paths,
        ));
    }
    if let Some(t) = bgem.GlowTexture.take() {
        bgem.GlowTexture = Some(namespace_texture_slot_path_if_selected(
            &t,
            namespace,
            texture_namespace_paths,
        ));
    }
}

// ---------------------------------------------------------------------------
// Asset output path construction
// ---------------------------------------------------------------------------

/// Build the Data-relative subpath for a material asset.
/// Mirrors Python's `_asset_data_subpath` + unprefixed output path normalization.
fn asset_output_subpath(
    source_path: &str,
    _asset_prefix: &str,
    output_subpath: Option<&str>,
) -> String {
    if let Some(subpath) = output_subpath {
        let clean = subpath.replace('\\', "/").trim_matches('/').to_owned();
        if !clean.is_empty() {
            return clean;
        }
    }
    let norm = source_path.replace('\\', "/");
    let lower = norm.to_lowercase();

    // Ensure Materials/ root prefix.
    let with_root = if lower.starts_with("materials/") {
        norm.clone()
    } else {
        format!("Materials/{}", norm)
    };

    // Split: root / [maybe-source-prefix] / rest
    let mut parts = with_root.splitn(3, '/');
    let root = match parts.next() {
        Some(r) => r,
        None => return with_root,
    };
    let second = match parts.next() {
        Some(s) => s,
        None => return with_root,
    };

    if is_known_asset_prefix(second) {
        return match parts.next() {
            Some(rest) => format!("{root}/{rest}"),
            None => root.to_owned(),
        };
    }

    with_root
}

// ---------------------------------------------------------------------------
// BGSM field overrides (bgsm_default_overrides)
// ---------------------------------------------------------------------------

fn apply_bgsm_overrides(bgsm: &mut crate::bgsm::BgsmData, overrides: &[(String, JsonValue)]) {
    for (key, val) in overrides {
        match key.as_str() {
            "bCastShadows" | "CastShadows" => {
                if let Some(b) = val.as_bool() {
                    bgsm.CastShadows = b;
                }
            }
            "bReceiveShadows" | "ReceiveShadows" => {
                if let Some(b) = val.as_bool() {
                    bgsm.ReceiveShadows = b;
                }
            }
            "bTwoSided" | "TwoSided" => {
                if let Some(b) = val.as_bool() {
                    bgsm.header.two_sided = b;
                }
            }
            "bDecal" | "Decal" => {
                if let Some(b) = val.as_bool() {
                    bgsm.header.decal = b;
                }
            }
            "bGlowmap" | "Glowmap" => {
                if let Some(b) = val.as_bool() {
                    bgsm.Glowmap = b;
                }
            }
            "bEmitEnabled" | "EmitEnabled" => {
                if let Some(b) = val.as_bool() {
                    bgsm.EmitEnabled = b;
                }
            }
            _ => {} // unknown key: ignore
        }
    }
}

// ---------------------------------------------------------------------------
// CDB → BGSM translation (stub: best-effort, falls back to copy-as-is)
// ---------------------------------------------------------------------------

/// Try to translate a .mat / CDB-ref asset into BGSM bytes via the CDB file.
///
/// The full CE2-material → BGSM pipeline is implemented in Python
/// (`creation_lib.material_tools.materials_cdb` + `cdb_to_bgsm`).
/// This stub returns `None` so callers fall back to copy-as-is.
///
/// TODO: port `cdb_to_bgsm` translation to Rust for native execution.
fn cdb_to_bgsm_bytes(_cdb_path: &Path, _source_path: &str) -> Option<Vec<u8>> {
    None
}

// ---------------------------------------------------------------------------
// Per-asset conversion result
// ---------------------------------------------------------------------------

enum MatOutcome {
    Converted,
    Skipped,
    Warning(String),
}

struct ConvertResult {
    outcome: MatOutcome,
    log: Option<String>,
}

// ---------------------------------------------------------------------------
// Per-asset conversion logic
// ---------------------------------------------------------------------------

/// Normalize a material entry's `source_path` into the lookup key used by
/// `ConvertMaterialsRequest::source_path_overrides` (lowercase, `/`-separated,
/// `materials/`-prefixed, `.bgsm`/`.bgem` only). Mirrors the conversion crate's
/// override-table key normalization so a raw `C:\...\Data\Materials\...` NIF path
/// and an enumerated `Materials/...` rel path both match the same key.
fn normalize_material_source_key(value: &str) -> Option<String> {
    let raw = value.trim().trim_matches('\0').replace('\\', "/");
    if raw.is_empty() || raw.starts_with("//") {
        return None;
    }
    let mut path = raw.trim_start_matches('/').to_ascii_lowercase();
    if let Some((_, rest)) = path.split_once("/data/") {
        path = rest.to_string();
    } else if let Some(rest) = path.strip_prefix("data/") {
        path = rest.to_string();
    } else {
        let bytes = path.as_bytes();
        let win_abs = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'/';
        if win_abs || path.contains(':') {
            return None;
        }
    }
    let parts: Vec<&str> = path
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.is_empty() || parts.iter().any(|p| *p == "..") {
        return None;
    }
    let rel = parts.join("/");
    let rel = if rel.starts_with("materials/") {
        rel
    } else {
        format!("materials/{rel}")
    };
    (rel.ends_with(".bgsm") || rel.ends_with(".bgem")).then_some(rel)
}

/// Resolve a data-relative override replacement against the source tree.
fn resolve_override_source(source_extracted: &Path, data_relative: &str) -> Option<PathBuf> {
    let rel = data_relative.replace('/', std::path::MAIN_SEPARATOR_STR);
    let candidates = [
        source_extracted.join(&rel),
        source_extracted.join("Data").join(&rel),
    ];
    candidates.into_iter().find(|c| c.is_file())
}

/// Repoint each matching entry's `resolved_path` at its override replacement.
/// `source_path` (and thus the output location) is left unchanged, so the
/// placeholder path stays but its bytes come from the real material.
fn apply_source_path_overrides(
    entries: &mut [MaterialEntry],
    overrides: &HashMap<String, String>,
    source_extracted: &Path,
) {
    for entry in entries.iter_mut() {
        let Some(key) = normalize_material_source_key(&entry.source_path) else {
            continue;
        };
        let Some(replacement) = overrides.get(&key) else {
            continue;
        };
        if let Some(resolved) = resolve_override_source(source_extracted, replacement) {
            entry.resolved_path = resolved.to_string_lossy().to_string();
        }
    }
}

fn enumerate_source_materials(source_extracted: &Path) -> Vec<MaterialEntry> {
    let mut out = Vec::new();
    collect_material_files(
        &source_extracted.join("Materials"),
        source_extracted,
        &mut out,
    );
    out.sort_by(|a, b| {
        a.source_path
            .to_lowercase()
            .cmp(&b.source_path.to_lowercase())
    });
    out
}

fn collect_material_files(dir: &Path, source_root: &Path, out: &mut Vec<MaterialEntry>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_material_files(&path, source_root, out);
            continue;
        }
        let is_material = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| {
                let e = e.to_ascii_lowercase();
                e == "bgsm" || e == "bgem"
            })
            .unwrap_or(false);
        if !is_material {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(source_root) {
            out.push(MaterialEntry {
                source_path: rel.to_string_lossy().replace('\\', "/"),
                resolved_path: path.to_string_lossy().to_string(),
                is_cdb_ref: false,
                output_subpath: None,
                texture_namespace: None,
                texture_namespace_paths: HashSet::new(),
            });
        }
    }
}

fn has_explicit_output_subpath(entry: &MaterialEntry) -> bool {
    entry
        .output_subpath
        .as_deref()
        .map(|subpath| {
            !subpath
                .replace('\\', "/")
                .trim_matches('/')
                .trim()
                .is_empty()
        })
        .unwrap_or(false)
}

fn convert_one(
    entry: &MaterialEntry,
    mod_path: &Path,
    source_game: Game,
    target_game: Game,
    asset_prefix: &str,
    cdb_path: Option<&Path>,
    overwrite_existing: bool,
    bgsm_default_overrides: &[(String, JsonValue)],
    target_dirs: &[PathBuf],
    target_asset_paths: &HashSet<String>,
) -> ConvertResult {
    // Validate resolved path.
    if entry.resolved_path.is_empty() || !Path::new(&entry.resolved_path).is_file() {
        return ConvertResult {
            outcome: MatOutcome::Warning(format!(
                "[ERROR] Material not found: {}",
                entry.source_path
            )),
            log: None,
        };
    }
    let resolved = Path::new(&entry.resolved_path);

    let subpath = asset_output_subpath(
        &entry.source_path,
        asset_prefix,
        entry.output_subpath.as_deref(),
    );
    let out_path = mod_path.join("data").join(&subpath);

    if target_asset_paths.contains(&normalize_target_asset_key(&subpath))
        || (!target_dirs.is_empty() && target_dirs.iter().any(|t| t.join(&subpath).is_file()))
    {
        return ConvertResult {
            outcome: MatOutcome::Skipped,
            log: None,
        };
    }

    // Overwrite check. Reprocess stale material outputs that were copied from
    // JSON/text pseudo-materials before the converter knew how to handle them.
    if !overwrite_existing
        && out_path.exists()
        && existing_output_matches_signature(
            &out_path,
            &entry.source_path,
            source_game,
            target_game,
        )
    {
        return ConvertResult {
            outcome: MatOutcome::Skipped,
            log: None,
        };
    }

    let lower = entry.source_path.to_lowercase();
    let is_mat = entry.is_cdb_ref || lower.ends_with(".mat");

    // ---- .mat / CDB-ref dispatch ----
    if is_mat {
        if let Some(cdb) = cdb_path {
            if let Some(bgsm_bytes) = cdb_to_bgsm_bytes(cdb, &entry.source_path) {
                match crate::bgsm::parse(&bgsm_bytes) {
                    Ok(bgsm) => {
                        let bgsm =
                            downgrade_bgsm(bgsm, &entry.source_path, source_game, target_game);
                        return write_bgsm(
                            &bgsm,
                            &out_path,
                            bgsm_default_overrides,
                            entry.texture_namespace.as_deref(),
                            &entry.texture_namespace_paths,
                        );
                    }
                    Err(e) => {
                        return copy_as_is(
                            resolved,
                            &out_path,
                            &format!(
                                "[WARN] CDB BGSM parse error, copying as-is: {}: {e}",
                                entry.source_path
                            ),
                        );
                    }
                }
            }
        }
        // No CDB or lookup miss → copy-as-is.
        return copy_as_is(
            resolved,
            &out_path,
            &format!("[INFO] .mat copied as-is (no CDB): {}", entry.source_path),
        );
    }

    // ---- BGSM ----
    if lower.ends_with(".bgsm") {
        let data = match fs::read(resolved) {
            Ok(d) => d,
            Err(e) => {
                return ConvertResult {
                    outcome: MatOutcome::Warning(format!(
                        "[ERROR] Read failed {}: {e}",
                        entry.source_path
                    )),
                    log: None,
                };
            }
        };
        match crate::bgsm::parse(&data) {
            Ok(mut bgsm) => {
                repair_missing_fo76_smoothspec_from_specular(
                    &mut bgsm,
                    resolved,
                    source_game,
                    target_game,
                );
                let bgsm = downgrade_bgsm(bgsm, &entry.source_path, source_game, target_game);
                return write_bgsm(
                    &bgsm,
                    &out_path,
                    bgsm_default_overrides,
                    entry.texture_namespace.as_deref(),
                    &entry.texture_namespace_paths,
                );
            }
            Err(e) => match parse_json_bgsm(&data) {
                Ok(mut bgsm) => {
                    repair_missing_fo76_smoothspec_from_specular(
                        &mut bgsm,
                        resolved,
                        source_game,
                        target_game,
                    );
                    let bgsm = downgrade_bgsm(bgsm, &entry.source_path, source_game, target_game);
                    let mut result = write_bgsm(
                        &bgsm,
                        &out_path,
                        bgsm_default_overrides,
                        entry.texture_namespace.as_deref(),
                        &entry.texture_namespace_paths,
                    );
                    if matches!(result.outcome, MatOutcome::Converted) {
                        result.log =
                            Some(format!("[INFO] JSON BGSM converted: {}", entry.source_path));
                    }
                    return result;
                }
                Err(json_error) => {
                    remove_invalid_output(&out_path);
                    return ConvertResult {
                        outcome: MatOutcome::Warning(format!(
                            "[ERROR] BGSM parse failed: {}: {e}; JSON fallback failed: {json_error}",
                            entry.source_path
                        )),
                        log: None,
                    };
                }
            },
        }
    }

    // ---- BGEM ----
    if lower.ends_with(".bgem") {
        let data = match fs::read(resolved) {
            Ok(d) => d,
            Err(e) => {
                return ConvertResult {
                    outcome: MatOutcome::Warning(format!(
                        "[ERROR] Read failed {}: {e}",
                        entry.source_path
                    )),
                    log: None,
                };
            }
        };
        match crate::bgem::parse(&data) {
            Ok(bgem) => {
                let bgem = downgrade_bgem(bgem, &entry.source_path, source_game, target_game);
                return write_bgem(
                    &bgem,
                    &out_path,
                    entry.texture_namespace.as_deref(),
                    &entry.texture_namespace_paths,
                );
            }
            Err(e) => match parse_json_bgem(&data) {
                Ok(bgem) => {
                    let bgem = downgrade_bgem(bgem, &entry.source_path, source_game, target_game);
                    let mut result = write_bgem(
                        &bgem,
                        &out_path,
                        entry.texture_namespace.as_deref(),
                        &entry.texture_namespace_paths,
                    );
                    if matches!(result.outcome, MatOutcome::Converted) {
                        result.log =
                            Some(format!("[INFO] JSON BGEM converted: {}", entry.source_path));
                    }
                    return result;
                }
                Err(json_error) => {
                    remove_invalid_output(&out_path);
                    return ConvertResult {
                        outcome: MatOutcome::Warning(format!(
                            "[ERROR] BGEM parse failed: {}: {e}; JSON fallback failed: {json_error}",
                            entry.source_path
                        )),
                        log: None,
                    };
                }
            },
        }
    }

    // ---- Unknown extension: copy as-is ----
    copy_as_is(
        resolved,
        &out_path,
        &format!(
            "[INFO] Material copied as-is (unknown format): {}",
            entry.source_path
        ),
    )
}

// ---------------------------------------------------------------------------
// Write helpers
// ---------------------------------------------------------------------------

fn write_bgsm(
    bgsm: &crate::bgsm::BgsmData,
    out_path: &Path,
    overrides: &[(String, JsonValue)],
    texture_namespace: Option<&str>,
    texture_namespace_paths: &HashSet<String>,
) -> ConvertResult {
    let mut bgsm = bgsm.clone();
    apply_bgsm_overrides(&mut bgsm, overrides);
    namespace_bgsm_texture_slots(&mut bgsm, texture_namespace, texture_namespace_paths);
    let bytes = crate::bgsm::write(&bgsm);
    match write_bytes(out_path, &bytes) {
        Ok(()) => ConvertResult {
            outcome: MatOutcome::Converted,
            log: None,
        },
        Err(e) => ConvertResult {
            outcome: MatOutcome::Warning(format!(
                "[ERROR] Write failed {}: {e}",
                out_path.display()
            )),
            log: None,
        },
    }
}

fn write_bgem(
    bgem: &crate::bgem::BgemData,
    out_path: &Path,
    texture_namespace: Option<&str>,
    texture_namespace_paths: &HashSet<String>,
) -> ConvertResult {
    let mut bgem = bgem.clone();
    namespace_bgem_texture_slots(&mut bgem, texture_namespace, texture_namespace_paths);
    let bytes = crate::bgem::write(&bgem);
    match write_bytes(out_path, &bytes) {
        Ok(()) => ConvertResult {
            outcome: MatOutcome::Converted,
            log: None,
        },
        Err(e) => ConvertResult {
            outcome: MatOutcome::Warning(format!(
                "[ERROR] Write failed {}: {e}",
                out_path.display()
            )),
            log: None,
        },
    }
}

fn copy_as_is(src: &Path, out_path: &Path, log_msg: &str) -> ConvertResult {
    match copy_file(src, out_path) {
        Ok(()) => ConvertResult {
            outcome: MatOutcome::Converted,
            log: Some(log_msg.to_owned()),
        },
        Err(e) => ConvertResult {
            outcome: MatOutcome::Warning(format!(
                "[ERROR] Copy failed {} → {}: {e}",
                src.display(),
                out_path.display()
            )),
            log: None,
        },
    }
}

fn write_bytes(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, data)
}

fn copy_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dst).map(|_| ())
}

// ---------------------------------------------------------------------------
// Conversion runner
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertLogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct ConvertLog {
    pub level: ConvertLogLevel,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ConvertProgress {
    pub current: u32,
    pub total: u32,
}

#[derive(Debug, Default, Clone)]
pub struct ConvertMaterialsReport {
    pub assets_written: u32,
    pub warnings: u32,
    pub logs: Vec<ConvertLog>,
    pub progress: Vec<ConvertProgress>,
}

pub fn run_convert_materials(
    mod_path: &Path,
    params: &ConvertMaterialsRequest,
    default_source_game: Game,
    default_target_game: Game,
    source_extracted: &Path,
    target_extracted_dir: Option<&Path>,
    target_data_dir: Option<&Path>,
) -> ConvertMaterialsReport {
    let source_game = params.source_game.unwrap_or(default_source_game);
    let target_game = params.target_game.unwrap_or(default_target_game);
    let cdb_path = params.source_materialsdb.as_deref();

    let mut materials_owned: Vec<MaterialEntry> = if params.convert_all {
        let explicit_output_keys: HashSet<String> = params
            .materials
            .iter()
            .filter(|entry| has_explicit_output_subpath(entry))
            .filter_map(|entry| normalize_material_source_key(&entry.source_path))
            .collect();
        let mut m = enumerate_source_materials(source_extracted);
        if !explicit_output_keys.is_empty() {
            m.retain(|entry| {
                normalize_material_source_key(&entry.source_path)
                    .map(|key| !explicit_output_keys.contains(&key))
                    .unwrap_or(true)
            });
        }
        m.extend(params.materials.iter().cloned());
        m
    } else {
        params.materials.clone()
    };
    // Apply source-path overrides to the FULL list — including the entries
    // `convert_all` enumerated above, which never pass through the conversion
    // crate's per-entry override pass. Without this, placeholder materials like
    // TEMP_GroundTexture01.bgsm are re-emitted verbatim at the path NIFs
    // reference, defeating the override.
    if !params.source_path_overrides.is_empty() {
        apply_source_path_overrides(
            &mut materials_owned,
            &params.source_path_overrides,
            source_extracted,
        );
    }
    let materials: &[MaterialEntry] = &materials_owned;

    let target_dirs: Vec<PathBuf> = if params.convert_all {
        let mut v = Vec::new();
        if let Some(t) = target_extracted_dir {
            v.push(t.to_path_buf());
        }
        if let Some(t) = target_data_dir {
            v.push(t.to_path_buf());
        }
        v
    } else {
        Vec::new()
    };

    let total = materials.len() as u32;
    let mod_path = mod_path.to_path_buf();
    let asset_prefix = params.asset_prefix.clone();
    let overwrite = params.overwrite_existing;
    let overrides = params.bgsm_default_overrides.clone();

    // Run per-asset conversions in parallel for large batches.
    let results: Vec<ConvertResult> = if materials.len() > 64 {
        materials
            .par_iter()
            .map(|entry| {
                convert_one(
                    entry,
                    &mod_path,
                    source_game,
                    target_game,
                    &asset_prefix,
                    cdb_path,
                    overwrite,
                    &overrides,
                    &target_dirs,
                    &params.target_asset_paths,
                )
            })
            .collect()
    } else {
        materials
            .iter()
            .map(|entry| {
                convert_one(
                    entry,
                    &mod_path,
                    source_game,
                    target_game,
                    &asset_prefix,
                    cdb_path,
                    overwrite,
                    &overrides,
                    &target_dirs,
                    &params.target_asset_paths,
                )
            })
            .collect()
    };

    // Tally and report events sequentially.
    let mut assets_written: u32 = 0;
    let mut warnings: u32 = 0;
    let mut logs = Vec::new();
    let mut progress = Vec::new();

    for (i, r) in results.iter().enumerate() {
        match &r.outcome {
            MatOutcome::Converted => assets_written += 1,
            MatOutcome::Warning(msg) => {
                warnings += 1;
                let level = if msg.starts_with("[ERROR]") {
                    ConvertLogLevel::Error
                } else {
                    ConvertLogLevel::Warn
                };
                logs.push(ConvertLog {
                    level,
                    message: msg.clone(),
                });
            }
            MatOutcome::Skipped => {}
        }
        if let Some(ref msg) = r.log {
            logs.push(ConvertLog {
                level: ConvertLogLevel::Info,
                message: msg.clone(),
            });
        }
        // Progress pulse every 10 items or at the end.
        if i % 10 == 0 || i + 1 == materials.len() {
            progress.push(ConvertProgress {
                current: (i + 1) as u32,
                total,
            });
        }
    }

    ConvertMaterialsReport {
        assets_written,
        warnings,
        logs,
        progress,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- asset_output_subpath ----

    #[test]
    fn subpath_bare_path_injects_root_without_prefix() {
        let result = asset_output_subpath("Weapons/Foo.bgsm", "fo76", None);
        assert_eq!(result, "Materials/Weapons/Foo.bgsm");
    }

    #[test]
    fn subpath_already_has_materials_root_without_prefix() {
        let result = asset_output_subpath("Materials/Weapons/Bar.bgsm", "fo76", None);
        assert_eq!(result, "Materials/Weapons/Bar.bgsm");
    }

    #[test]
    fn subpath_no_prefix_is_identity() {
        let result = asset_output_subpath("Materials/Weapons/Bar.bgsm", "", None);
        assert_eq!(result, "Materials/Weapons/Bar.bgsm");
    }

    #[test]
    fn subpath_strips_known_prefix() {
        let result = asset_output_subpath("Materials/fo76/Weapons/Bar.bgsm", "fo76", None);
        assert_eq!(result, "Materials/Weapons/Bar.bgsm");
    }

    #[test]
    fn subpath_uses_explicit_output_subpath() {
        let result = asset_output_subpath(
            "Materials/fo76/Weapons/Bar.bgsm",
            "fo76",
            Some("Materials/FO76/Weapons/Bar.bgsm"),
        );
        assert_eq!(result, "Materials/FO76/Weapons/Bar.bgsm");
    }

    #[test]
    fn namespace_texture_slot_preserves_requested_namespace() {
        assert_eq!(
            namespace_texture_slot_path("Textures/Landscape/Rocks/Foo_d.dds", "FO76"),
            "FO76/Landscape/Rocks/Foo_d.dds"
        );
    }

    #[test]
    fn namespace_texture_slot_preserves_empty_slots() {
        assert_eq!(namespace_texture_slot_path("", "FO76"), "");
        assert_eq!(namespace_texture_slot_path("   \0\0", "FO76"), "");
    }

    #[test]
    fn namespace_texture_slot_leaves_shared_cubemap_unprefixed() {
        assert_eq!(
            namespace_texture_slot_path("Shared/Cubemaps/mipblur_DefaultOutside1.dds", "FO76"),
            "Shared/Cubemaps/mipblur_DefaultOutside1.dds"
        );
    }

    // ---- texture suffix renaming ----

    #[test]
    fn rename_reflectivity_suffix() {
        assert_eq!(
            rename_texture_basename_fo76_to_fo4("Foo_r.dds"),
            Some("Foo_s.dds".to_owned())
        );
    }

    #[test]
    fn rename_lighting_suffix() {
        assert_eq!(
            rename_texture_basename_fo76_to_fo4("Bar_l.dds"),
            Some("Bar_g.dds".to_owned())
        );
    }

    #[test]
    fn no_rename_for_diffuse_suffix() {
        assert_eq!(rename_texture_basename_fo76_to_fo4("Baz_d.dds"), None);
    }

    #[test]
    fn rewrite_preserves_path_prefix() {
        let result = rewrite_texture_path_fo76_to_fo4("Textures/Weapons/Gun_r.dds");
        assert_eq!(result, "Textures/Weapons/Gun_s.dds");
    }

    #[test]
    fn rewrite_repairs_dangling_corpse_customization_dir() {
        // FO76's corpse head BGSMs point diffuse/normal at a
        // `Actors/Customization/Character/Corpse/` directory that ships no
        // texture; the real corpse heads live under `Actors/Corpse/`. Left
        // as-is the FO4 output can't resolve them (CK "Could not queue
        // texture"). Repair to where the texture actually ships.
        assert_eq!(
            rewrite_texture_path_fo76_to_fo4(
                "Actors/Customization/Character/Corpse/Male/Head_d.dds"
            ),
            "Actors/Corpse/Male/Head_d.dds"
        );
        assert_eq!(
            rewrite_texture_path_fo76_to_fo4(
                "Actors\\Customization\\Character\\Corpse\\Female\\Head_n.dds"
            ),
            "Actors/Corpse/Female/Head_n.dds"
        );
    }

    #[test]
    fn rewrite_repairs_dangling_actor_customization_dir() {
        assert_eq!(
            rewrite_texture_path_fo76_to_fo4(
                "Actors/Customization/Character/GhoulFemale/PlayerGhoul/PlayerGhoulHeadSmoothFemale_d.dds"
            ),
            "Actors/Character/GhoulFemale/PlayerGhoul/PlayerGhoulHeadSmoothFemale_d.dds"
        );
        assert_eq!(
            rewrite_texture_path_fo76_to_fo4(
                "Actors/Customization/Character/GhoulMale/PlayerGhoul/PlayerGhoulHeadSmoothMale_r.dds"
            ),
            "Actors/Character/GhoulMale/PlayerGhoul/PlayerGhoulHeadSmoothMale_s.dds"
        );
        assert_eq!(
            rewrite_texture_path_fo76_to_fo4(
                "Actors/Customization/Character/Piper/PiperHead_l.dds"
            ),
            "Actors/Character/Piper/PiperHead_g.dds"
        );
    }

    #[test]
    fn rewrite_repairs_dangling_stock_head_dirs() {
        assert_eq!(
            rewrite_texture_path_fo76_to_fo4(
                "Actors/Customization/Character/BaseHumanFemale/MidAgeFemaleHead_d.dds"
            ),
            "Actors/Character/MidAgedFemale/MidAgeFemaleHead_d.dds"
        );
        assert_eq!(
            rewrite_texture_path_fo76_to_fo4(
                "Actors/Customization/Character/BaseHumanFemale/OldHumanFemaleHead_n.dds"
            ),
            "Actors/Character/OldHumanFemale/OldHumanFemaleHead_n.dds"
        );
        assert_eq!(
            rewrite_texture_path_fo76_to_fo4(
                "Actors/Customization/Character/BaseHumanMale/OldHumanMaleHead_r.dds"
            ),
            "Actors/Character/OldHumanMale/OldHumanMaleHead_s.dds"
        );
        assert_eq!(
            rewrite_texture_path_fo76_to_fo4(
                "Actors/Customization/Character/BaseHumanMale/Mayor_l.dds"
            ),
            "Actors/Character/Mayor/Mayor_g.dds"
        );
    }

    #[test]
    fn rewrite_leaves_real_actors_corpse_dir_untouched() {
        assert_eq!(
            rewrite_texture_path_fo76_to_fo4("Actors/Corpse/Male/Head_r.dds"),
            "Actors/Corpse/Male/Head_s.dds"
        );
    }

    #[test]
    fn material_texture_slot_path_strips_root_and_known_game_prefix() {
        assert_eq!(
            material_texture_slot_path("Textures/fo76/Landscape/Rocks/Foo_d.dds"),
            "Landscape/Rocks/Foo_d.dds"
        );
        assert_eq!(
            material_texture_slot_path("Data/Textures/fo76/Landscape/Rocks/Foo_d.dds"),
            "Landscape/Rocks/Foo_d.dds"
        );
        assert_eq!(
            material_texture_slot_path("Landscape/Rocks/Foo_d.dds"),
            "Landscape/Rocks/Foo_d.dds"
        );
    }

    // ---- BGSM downgrade ----

    #[test]
    fn bgsm_roundtrip_downgrade_v20_to_v2() {
        let bgsm = make_test_bgsm_v20();
        assert_eq!(bgsm.header.version, 20);

        let downgraded = downgrade_bgsm(
            bgsm,
            "Materials/Weapons/TestWeapon.bgsm",
            Game::Fo76,
            Game::Fo4,
        );

        assert_eq!(
            downgraded.header.version, BGSM_VERSION_FO4,
            "version not downgraded"
        );
        assert!(
            downgraded.SpecularTexture.is_none(),
            "SpecularTexture not cleared"
        );
        assert!(
            downgraded.LightingTexture.is_none(),
            "LightingTexture not cleared"
        );
        assert!(downgraded.FlowTexture.is_none(), "FlowTexture not cleared");
        // Cubemap injected for weapon path.
        let envmap = downgraded.EnvmapTexture.as_deref().unwrap_or("");
        assert!(!envmap.is_empty(), "cubemap not injected");
        assert_eq!(downgraded.header.env_mapping, Some(true));
        assert_eq!(downgraded.header.env_mapping_mask_scale, Some(1.0));
        // RootMaterialPath synthesised.
        assert!(
            !downgraded.RootMaterialPath.is_empty(),
            "RootMaterialPath not synthesised"
        );
        // WetnessControlEnvMapScale restored.
        assert!(
            downgraded.WetnessControlEnvMapScale.is_some(),
            "WetnessControlEnvMapScale missing"
        );
    }

    #[test]
    fn bgsm_no_downgrade_when_already_v2() {
        let bgsm = make_test_bgsm_v2();
        let result = downgrade_bgsm(bgsm, "Materials/Test.bgsm", Game::Fo76, Game::Fo4);
        assert_eq!(result.header.version, 2);
    }

    #[test]
    fn bgsm_no_downgrade_for_fo76_target() {
        let bgsm = make_test_bgsm_v20();
        let result = downgrade_bgsm(bgsm, "Materials/Test.bgsm", Game::Fo76, Game::Fo76);
        assert_eq!(
            result.header.version, 20,
            "should not downgrade for FO76 target"
        );
    }

    #[test]
    fn bgsm_lighting_emittance_is_disabled_for_static_fo4_bgsm() {
        let mut bgsm = make_test_bgsm_v20();
        bgsm.LightingTexture = Some("Interiors/Industrial/Foo_l.dds".to_owned());
        bgsm.GlowTexture = None;
        bgsm.EmitEnabled = true;
        bgsm.EmittanceColor = Some([1.0, 0.0, 0.0]);
        bgsm.EmittanceMult = 10.0;

        let result = downgrade_bgsm(
            bgsm,
            "Materials/Interiors/Industrial/Foo.bgsm",
            Game::Fo76,
            Game::Fo4,
        );

        assert!(result.GlowTexture.is_none());
        assert!(!result.Glowmap);
        assert!(!result.EmitEnabled);
        assert!(result.EmittanceColor.is_none());
        assert_eq!(result.EmittanceMult, 1.0);
    }

    #[test]
    fn bgsm_effect_lighting_emittance_is_preserved() {
        let mut bgsm = make_test_bgsm_v20();
        bgsm.LightingTexture = Some("Effects/Foo_l.dds".to_owned());
        bgsm.GlowTexture = None;
        bgsm.EmitEnabled = true;
        bgsm.EmittanceMult = 10.0;

        let result = downgrade_bgsm(bgsm, "Materials/Effects/Foo.bgsm", Game::Fo76, Game::Fo4);

        assert_eq!(result.GlowTexture.as_deref(), Some("Effects/Foo_g.dds"));
        assert!(result.Glowmap);
        assert!(result.EmitEnabled);
        assert_eq!(result.EmittanceMult, 1.0);
    }

    #[test]
    fn bgsm_tree_vegetation_uses_leaf_template_defaults() {
        let mut bgsm = make_test_bgsm_v20();
        bgsm.Tree = true;
        bgsm.Translucency = Some(true);
        bgsm.TranslucencyTransmissiveScale = Some(1.0);
        bgsm.SpecularTexture = Some("Landscape/Plants/Bramble01_r.dds".to_owned());
        bgsm.LightingTexture = Some("Landscape/Plants/Bramble01_l.dds".to_owned());

        let result = downgrade_bgsm(
            bgsm,
            "Materials/Landscape/Plants/Bramble.BGSM",
            Game::Fo76,
            Game::Fo4,
        );

        assert_eq!(result.RootMaterialPath, "Template/LeafTemplate_Wet.bgsm");
        assert_eq!(result.BackLighting, Some(true));
        assert_eq!(result.BackLightPower, Some(0.25));
        assert_eq!(result.SubsurfaceLighting, Some(true));
        assert_eq!(result.SubsurfaceLightingRolloff, Some(2.0));
        assert_eq!(result.EnvmapTexture.as_deref(), Some(""));
        assert_eq!(result.header.env_mapping, Some(false));
        assert!(result.GlowTexture.is_none());
        assert_eq!(result.SmoothSpecTexture, "Landscape/Plants/Bramble01_s.dds");
    }

    #[test]
    fn bgsm_grass_path_uses_grass_template_before_tree_flag() {
        let mut bgsm = make_test_bgsm_v20();
        bgsm.Tree = true;
        bgsm.Translucency = Some(true);

        let result = downgrade_bgsm(
            bgsm,
            "Materials/Landscape/Grass/MtnTop_Grass01.BGSM",
            Game::Fo76,
            Game::Fo4,
        );

        assert_eq!(result.RootMaterialPath, "Template/GrassTemplate_Wet.BGSM");
        assert_eq!(result.SubsurfaceLighting, Some(true));
        assert_eq!(result.EnvmapTexture.as_deref(), Some(""));
        assert_eq!(result.header.env_mapping, Some(false));
    }

    #[test]
    fn bgsm_landscape_rock_path_uses_rock_template() {
        let mut bgsm = make_test_bgsm_v20();
        bgsm.RootMaterialPath = String::new();
        bgsm.Tree = false;

        let result = downgrade_bgsm(
            bgsm,
            "Materials/Landscape/Rocks/RockBoulderForest_SingleDraw01.bgsm",
            Game::Fo76,
            Game::Fo4,
        );

        assert_eq!(result.RootMaterialPath, "template/RockTemplate_Wet.bgsm");
        // Rock is dielectric/matte — no cubemap, env mapping off (was chrome).
        assert_eq!(result.EnvmapTexture.as_deref(), Some(""));
        assert_eq!(result.header.env_mapping, Some(false));
    }

    #[test]
    fn select_cubemap_uses_ore_specific_cubemaps() {
        let cases = [
            ("Materials/Props/Ore/ore_gold.bgsm", ORE_GOLD),
            ("Materials/Props/Ore/ore_silver.bgsm", ORE_SILVER),
            ("Materials/Props/Ore/ore_copper.bgsm", CUBEMAP_COPPER),
            ("Materials/Props/Ore/ore_iron.bgsm", ORE_IRON),
            ("Materials/Props/Ore/ore_blacktitanium.bgsm", ORE_OBSIDIAN),
            ("Materials/Props/Ore/ore_coal.bgsm", ORE_OBSIDIAN),
            ("Materials/Props/Ore/ore_ultracite01_new.bgsm", ORE_STEEL),
            ("Materials/Props/Ore/ore_uranium.bgsm", ORE_STEEL),
            ("Materials/Props/IngotAndOre/titaniumore.bgsm", ORE_STEEL),
            (
                "Materials/Landscape/Minerals/mineral_hp_copper01.bgsm",
                CUBEMAP_COPPER,
            ),
            ("Materials/Landscape/Plants/mineral_gold01.bgsm", ORE_GOLD),
            (
                "Materials/Landscape/Plants/irradiatedore_01.bgsm",
                ORE_STEEL,
            ),
        ];

        for (path, expected) in cases {
            assert_eq!(
                select_cubemap(path),
                Some((expected, 1.0)),
                "unexpected cubemap for {path}"
            );
        }
    }

    #[test]
    fn select_cubemap_does_not_match_non_ore_words_containing_ore() {
        // "score"/"forest" contain the substring "ore" but are not ore, and are
        // dielectric — they must get no cubemap, not an ore/chrome one.
        assert_eq!(
            select_cubemap("Materials/Furniture/Ally/score_s13_signjoeybello.bgsm"),
            None
        );
        assert_eq!(
            select_cubemap("Materials/Landscape/Ground/forestfloor01.bgsm"),
            None
        );
    }

    #[test]
    fn select_cubemap_is_opt_in_for_metal() {
        // Dielectric / matte surfaces → no cubemap.
        for path in [
            "Materials/Landscape/Rocks/RockCliff76.bgsm",
            "Materials/SetDressing/Toys/TeddyBear01.bgsm",
            "Materials/Props/PlasticFruitBowl.bgsm",
            "Materials/SetDressing/CeramicTurkey.bgsm",
            "Materials/Architecture/Unique/BridgeConcrete01Details.bgsm",
            "Materials/Furniture/WoodChair01.bgsm",
        ] {
            assert_eq!(select_cubemap(path), None, "expected no cubemap for {path}");
        }

        // Named tinted metals → matching soft outdoor cubemap.
        assert_eq!(
            select_cubemap("Materials/SetDressing/BronzeStatue01.bgsm"),
            Some((OUT_BRONZE, 0.5))
        );
        assert_eq!(
            select_cubemap("Materials/SetDressing/CopperPipe01.bgsm"),
            Some((OUT_COPPER, 0.5))
        );
        assert_eq!(
            select_cubemap("Materials/SetDressing/ChromeBumper01.bgsm"),
            Some((METAL_CHROME, 0.5))
        );

        // Generic metal + vehicles → subtle outdoor reflection (not full chrome).
        for path in [
            "Materials/Landscape/Roads/RoadRailings01.bgsm",
            "Materials/Vehicles/FlatbedTrailerSmall/FlatbedTrailerSmall01.bgsm",
            "Materials/SetDressing/MetalBarrelRust01.bgsm",
        ] {
            assert_eq!(
                select_cubemap(path),
                Some((DEFAULT_OUTSIDE, 0.3)),
                "expected subtle metal reflection for {path}"
            );
        }

        // Folder-name false-match guard: "Iron_Mountain" folder must not force a
        // cubemap onto a stone wall whose basename has no metal token.
        assert_eq!(
            select_cubemap("Materials/Architecture/Iron_Mountain/StoneWallDirty01.bgsm"),
            None
        );

        // Weapons stay fully reflective.
        assert_eq!(
            select_cubemap("Materials/Weapons/10mm/Receiver.bgsm"),
            Some((DEFAULT_OUTSIDE, 1.0))
        );

        // BGEM (glass/effect) keeps a reflection by default even when dielectric.
        assert_eq!(
            select_cubemap_bgem("Materials/SetDressing/Glass/WindowGlass01.bgem"),
            Some((DEFAULT_OUTSIDE, 1.0))
        );
        // …but BGEM in an excluded dir still gets none.
        assert_eq!(select_cubemap_bgem("Materials/Effects/Smoke01.bgem"), None);
    }

    #[test]
    fn bgsm_landscape_mineral_path_keeps_ore_cubemap() {
        let mut bgsm = make_test_bgsm_v20();
        bgsm.RootMaterialPath = String::new();
        bgsm.Tree = false;

        let result = downgrade_bgsm(
            bgsm,
            "Materials/Landscape/Plants/Mineral_Gold01.bgsm",
            Game::Fo76,
            Game::Fo4,
        );

        assert_eq!(result.RootMaterialPath, "template/defaultTemplate_wet.bgsm");
        assert_eq!(result.EnvmapTexture.as_deref(), Some(ORE_GOLD));
        assert_eq!(result.header.env_mapping, Some(true));
        assert_eq!(result.header.env_mapping_mask_scale, Some(1.0));
    }

    #[test]
    fn bgsm_carpet_path_keeps_empty_root_material() {
        let mut bgsm = make_test_bgsm_v20();
        bgsm.RootMaterialPath = "template/defaultTemplate_wet.bgsm".to_owned();

        let result = downgrade_bgsm(
            bgsm,
            "Materials/SetDressing/Carpet/CarpetRunner01.bgsm",
            Game::Fo76,
            Game::Fo4,
        );

        assert_eq!(result.RootMaterialPath, "");
        assert_eq!(result.EnvmapTexture.as_deref(), Some(DEFAULT_DIELECTRIC));
        assert_eq!(result.header.env_mapping, Some(true));
        assert_eq!(result.header.env_mapping_mask_scale, Some(0.3));
    }

    #[test]
    fn bgsm_corpse_head_repairs_dangling_diffuse_and_normal() {
        let mut bgsm = make_test_bgsm_v20();
        bgsm.DiffuseTexture = "Actors/Customization/Character/Corpse/Male/Head_d.dds".to_owned();
        bgsm.NormalTexture = "Actors/Customization/Character/Corpse/Male/Head_n.dds".to_owned();

        let result = downgrade_bgsm(
            bgsm,
            "Materials/Actors/Corpse/MaleHead.bgsm",
            Game::Fo76,
            Game::Fo4,
        );

        assert_eq!(result.DiffuseTexture, "Actors/Corpse/Male/Head_d.dds");
        assert_eq!(result.NormalTexture, "Actors/Corpse/Male/Head_n.dds");
    }

    // ---- BGSM default overrides ----

    #[test]
    fn bgsm_override_cast_shadows() {
        let mut bgsm = make_test_bgsm_v2();
        bgsm.CastShadows = false;
        let overrides = vec![("bCastShadows".to_owned(), serde_json::json!(true))];
        apply_bgsm_overrides(&mut bgsm, &overrides);
        assert!(bgsm.CastShadows);
    }

    #[test]
    fn bgsm_override_receive_shadows() {
        let mut bgsm = make_test_bgsm_v2();
        bgsm.ReceiveShadows = true;
        let overrides = vec![("bReceiveShadows".to_owned(), serde_json::json!(false))];
        apply_bgsm_overrides(&mut bgsm, &overrides);
        assert!(!bgsm.ReceiveShadows);
    }

    #[test]
    fn json_bgsm_material_converts_to_binary_v2() {
        let data = br##"{
            "bCastShadows": true,
            "bPBR": true,
            "cSpecularColor": "#ffffff",
            "fSmoothness": 1.0,
            "sDiffuseTexture": "ATX\\SetDressing\\Foo\\Foo_d.dds",
            "sNormalTexture": "ATX\\SetDressing\\Foo\\Foo_n.dds",
            "sSpecularTexture": "ATX\\SetDressing\\Foo\\Foo_r.dds",
            "sLightingTexture": "ATX\\SetDressing\\Foo\\Foo_l.dds"
        }"##;
        let bgsm = parse_json_bgsm(data).expect("json bgsm");
        let downgraded = downgrade_bgsm(
            bgsm,
            "Materials/ATX/SetDressing/Foo/Foo.bgsm",
            Game::Fo76,
            Game::Fo4,
        );
        let bytes = crate::bgsm::write(&downgraded);

        assert_eq!(&bytes[..4], b"BGSM");
        assert_eq!(downgraded.header.version, BGSM_VERSION_FO4);
        assert_eq!(downgraded.DiffuseTexture, "ATX/SetDressing/Foo/Foo_d.dds");
        assert_eq!(downgraded.NormalTexture, "ATX/SetDressing/Foo/Foo_n.dds");
        assert_eq!(
            downgraded.SmoothSpecTexture,
            "ATX/SetDressing/Foo/Foo_s.dds"
        );
    }

    #[test]
    fn json_bgem_material_converts_to_binary_v2() {
        let data = br##"{
            "bZBufferWrite": false,
            "cBaseColor": "#ffffff",
            "fBaseColorScale": 1.0,
            "sBaseTexture": "Effects\\ColorBlackZeroAlphaUtility.dds",
            "sEnvmapMaskTexture": "",
            "sEnvmapTexture": "",
            "sGreyscaleTexture": "",
            "sNormalTexture": ""
        }"##;
        let bgem = parse_json_bgem(data).expect("json bgem");
        let downgraded = downgrade_bgem(
            bgem,
            "Materials/Interface/HUDGlassFlat.bgem",
            Game::Fo76,
            Game::Fo4,
        );
        let bytes = crate::bgem::write(&downgraded);

        assert_eq!(&bytes[..4], b"BGEM");
        assert_eq!(downgraded.header.version, BGEM_VERSION_FO4);
        assert_eq!(
            downgraded.BaseTexture,
            "Effects/ColorBlackZeroAlphaUtility.dds"
        );
    }

    // ---- request parsing ----

    #[test]
    fn enumerate_source_materials_walks_bgsm_and_bgem() {
        let tmp = std::env::temp_dir().join("enumerate_source_materials_walks");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("Materials").join("Sub")).unwrap();
        fs::write(tmp.join("Materials").join("a.bgsm"), b"x").unwrap();
        fs::write(tmp.join("Materials").join("Sub").join("b.bgem"), b"x").unwrap();
        fs::write(tmp.join("Materials").join("Sub").join("c.txt"), b"x").unwrap();

        let entries = enumerate_source_materials(&tmp);

        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|e| e.source_path.ends_with("a.bgsm")));
        assert!(
            entries
                .iter()
                .any(|e| e.source_path == "Materials/Sub/b.bgem")
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn from_json_parses_convert_all_flag() {
        let v = serde_json::json!({ "materials": [], "convert_all": true });
        let req = ConvertMaterialsRequest::from_json(&v).unwrap();
        assert!(req.convert_all);
        let v2 = serde_json::json!({ "materials": [] });
        assert!(!ConvertMaterialsRequest::from_json(&v2).unwrap().convert_all);
    }

    #[test]
    fn from_json_parses_pbr_carry_flag() {
        let enabled = serde_json::json!({ "pbr_carry": true });
        let defaulted = serde_json::json!({});

        assert!(
            ConvertMaterialsRequest::from_json(&enabled)
                .unwrap()
                .pbr_carry
        );
        assert!(
            !ConvertMaterialsRequest::from_json(&defaulted)
                .unwrap()
                .pbr_carry
        );
    }

    #[test]
    fn params_parse_empty_materials() {
        let v = serde_json::json!({});
        let p = ConvertMaterialsRequest::from_json(&v).expect("parse failed");
        assert!(p.materials.is_empty());
        assert!(p.bgsm_default_overrides.is_empty());
        assert!(!p.overwrite_existing);
    }

    #[test]
    fn params_parse_material_entry() {
        let v = serde_json::json!({
            "materials": [{
                "source_path": "Materials/Weapons/Gun.bgsm",
                "resolved_path": "/tmp/Gun.bgsm",
                "is_cdb_ref": false
            }],
            "asset_prefix": "fo76",
            "bgsm_default_overrides": { "bCastShadows": true }
        });
        let p = ConvertMaterialsRequest::from_json(&v).expect("parse failed");
        assert_eq!(p.materials.len(), 1);
        assert_eq!(p.materials[0].source_path, "Materials/Weapons/Gun.bgsm");
        assert_eq!(p.asset_prefix, "fo76");
        assert_eq!(p.bgsm_default_overrides.len(), 1);
    }

    #[test]
    fn source_bgsm_glowmap_predicate_matches_downgrade() {
        let mut bgsm = make_test_bgsm(20);
        bgsm.Glowmap = false;
        bgsm.EmitEnabled = false;
        bgsm.LightingTexture = None;
        bgsm.GlowTexture = None;
        assert!(
            !source_bgsm_enables_fo4_glowmap(&bgsm, "Materials/Props/Test.bgsm"),
            "non-emissive material yields no glow map"
        );

        // EmitEnabled + FO76 lighting mask + no existing glow texture is
        // disabled during BGSM downgrade because FO4 applies it to the whole
        // object surface.
        bgsm.EmitEnabled = true;
        bgsm.LightingTexture = Some("textures/x_l.dds".to_string());
        assert!(
            !source_bgsm_enables_fo4_glowmap(&bgsm, "Materials/Props/Test.bgsm"),
            "synthesized lighting-mask emission is not preserved"
        );
        assert!(
            source_bgsm_enables_fo4_glowmap(&bgsm, "Materials/Effects/Test.bgsm"),
            "effect materials preserve synthesized lighting-mask emission"
        );

        // An existing GlowTexture suppresses the lighting-mask synthesis branch.
        bgsm.GlowTexture = Some("textures/x_g.dds".to_string());
        assert!(
            !source_bgsm_enables_fo4_glowmap(&bgsm, "Materials/Effects/Test.bgsm"),
            "existing glow texture skips the synthesis branch"
        );

        // An explicit Glowmap flag only matters when emission remains enabled.
        bgsm.Glowmap = true;
        assert!(
            !source_bgsm_enables_fo4_glowmap(&bgsm, "Materials/Props/Test.bgsm"),
            "static object paths suppress explicit emissive glowmap"
        );
        assert!(
            source_bgsm_enables_fo4_glowmap(&bgsm, "Materials/Effects/Test.bgsm"),
            "explicit emissive glowmap yields a glow map"
        );
        bgsm.EmitEnabled = false;
        assert!(
            source_bgsm_enables_fo4_glowmap(&bgsm, "Materials/Effects/Test.bgsm"),
            "explicit Glowmap flag still controls true effect material glow"
        );
    }

    #[test]
    fn existing_static_fo76_bgsm_with_emit_enabled_is_stale() {
        let tmp = std::env::temp_dir().join(format!(
            "existing_static_fo76_bgsm_with_emit_enabled_is_stale_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("stale.bgsm");

        let mut bgsm = make_test_bgsm_v2();
        bgsm.EmitEnabled = true;
        std::fs::write(&path, crate::bgsm::write(&bgsm)).unwrap();

        assert!(
            !existing_output_matches_signature(
                &path,
                "Materials/Props/Test.bgsm",
                Game::Fo76,
                Game::Fo4
            ),
            "static FO76-derived v2 BGSMs with EmitEnabled must be regenerated"
        );
        assert!(
            existing_output_matches_signature(
                &path,
                "Materials/Effects/Test.bgsm",
                Game::Fo76,
                Game::Fo4
            ),
            "effect materials keep their emissive behavior"
        );
        assert!(
            existing_output_matches_signature(
                &path,
                "Materials/Props/Test.bgsm",
                Game::Fo4,
                Game::Fo4
            ),
            "non-FO76 source materials are not part of this repair policy"
        );

        bgsm.EmitEnabled = false;
        std::fs::write(&path, crate::bgsm::write(&bgsm)).unwrap();
        assert!(
            existing_output_matches_signature(
                &path,
                "Materials/Props/Test.bgsm",
                Game::Fo76,
                Game::Fo4
            ),
            "after regeneration the output can be skipped normally"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn convert_all_applies_source_path_override_to_enumerated_entry() {
        // Regression: in convert_all mode `run_convert_materials` enumerates the
        // source Materials/ tree itself; those entries must still honor the
        // source-path override (TEMP placeholder -> real material) that callers
        // pass, or the non-namespaced file NIFs reference keeps the placeholder.
        let tmp = std::env::temp_dir().join("convert_all_source_path_override");
        let source = tmp.join("source");
        let out = tmp.join("mod");
        let ground = source.join("Materials").join("Landscape").join("Ground");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&ground).unwrap();

        let mut temp = make_test_bgsm(20);
        temp.DiffuseTexture = "Landscape/Ground/TEMP_GroundTexture01_d.dds".to_string();
        temp.NormalTexture = "Landscape/Ground/TEMP_GroundTexture01_n.dds".to_string();
        std::fs::write(
            ground.join("temp_groundtexture01.bgsm"),
            crate::bgsm::write(&temp),
        )
        .unwrap();

        let mut forest = make_test_bgsm(20);
        forest.DiffuseTexture = "Landscape/Ground/ForestRocks01_d.dds".to_string();
        forest.NormalTexture = "Landscape/Ground/ForestRocks01_n.dds".to_string();
        std::fs::write(
            ground.join("forestrocks01.bgsm"),
            crate::bgsm::write(&forest),
        )
        .unwrap();

        let mut overrides = HashMap::new();
        overrides.insert(
            "materials/landscape/ground/temp_groundtexture01.bgsm".to_string(),
            "materials/landscape/ground/forestrocks01.bgsm".to_string(),
        );

        let request = ConvertMaterialsRequest {
            materials: vec![],
            source_game: Some(Game::Fo76),
            target_game: Some(Game::Fo4),
            asset_prefix: String::new(),
            source_materialsdb: None,
            overwrite_existing: false,
            bgsm_default_overrides: vec![],
            convert_all: true,
            pbr_carry: false,
            source_path_overrides: overrides,
            target_asset_paths: HashSet::new(),
        };

        run_convert_materials(&out, &request, Game::Fo76, Game::Fo4, &source, None, None);

        let written = out
            .join("data")
            .join("Materials")
            .join("Landscape")
            .join("Ground")
            .join("temp_groundtexture01.bgsm");
        assert!(
            written.is_file(),
            "expected output at {}",
            written.display()
        );
        let parsed = crate::bgsm::parse(&std::fs::read(&written).unwrap()).unwrap();
        assert!(
            parsed
                .DiffuseTexture
                .to_lowercase()
                .contains("forestrocks01"),
            "convert_all TEMP material must convert from the override source \
             (forestrocks01), got diffuse {}",
            parsed.DiffuseTexture
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn convert_all_drops_default_output_when_explicit_subpath_is_queued() {
        let tmp = std::env::temp_dir().join(format!(
            "convert_all_explicit_output_subpath_{}",
            std::process::id()
        ));
        let source = tmp.join("source");
        let out = tmp.join("mod");
        let grass = source.join("Materials").join("Landscape").join("Grass");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&grass).unwrap();

        let mut hemlock = make_test_bgsm(20);
        hemlock.DiffuseTexture = "Landscape/Grass/Forest76WaterHemlock01_D.dds".to_string();
        hemlock.NormalTexture = "Landscape/Grass/Forest76WaterHemlock01_n.dds".to_string();
        std::fs::write(
            grass.join("forest76waterhemlock01.bgsm"),
            crate::bgsm::write(&hemlock),
        )
        .unwrap();

        let request = ConvertMaterialsRequest {
            materials: vec![MaterialEntry {
                source_path: "Materials/Landscape/Grass/forest76waterhemlock01.bgsm".to_string(),
                resolved_path: grass
                    .join("forest76waterhemlock01.bgsm")
                    .to_string_lossy()
                    .to_string(),
                is_cdb_ref: false,
                output_subpath: Some(
                    "Materials/FO76/Landscape/Grass/forest76waterhemlock01.bgsm".to_string(),
                ),
                texture_namespace: Some("FO76".to_string()),
                texture_namespace_paths: HashSet::new(),
            }],
            source_game: Some(Game::Fo76),
            target_game: Some(Game::Fo4),
            asset_prefix: String::new(),
            source_materialsdb: None,
            overwrite_existing: true,
            bgsm_default_overrides: vec![],
            convert_all: true,
            pbr_carry: false,
            source_path_overrides: HashMap::new(),
            target_asset_paths: HashSet::new(),
        };

        run_convert_materials(&out, &request, Game::Fo76, Game::Fo4, &source, None, None);

        let root_output = out
            .join("data")
            .join("Materials")
            .join("Landscape")
            .join("Grass")
            .join("forest76waterhemlock01.bgsm");
        let namespaced_output = out
            .join("data")
            .join("Materials")
            .join("FO76")
            .join("Landscape")
            .join("Grass")
            .join("forest76waterhemlock01.bgsm");
        assert!(
            !root_output.exists(),
            "default output should be suppressed when explicit output_subpath is queued"
        );
        assert!(
            namespaced_output.is_file(),
            "expected namespaced output at {}",
            namespaced_output.display()
        );
        let parsed = crate::bgsm::parse(&std::fs::read(&namespaced_output).unwrap()).unwrap();
        assert_eq!(
            parsed.DiffuseTexture.trim_end_matches('\0'),
            "FO76/Landscape/Grass/Forest76WaterHemlock01_D.dds"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn bgsm_missing_smoothspec_uses_existing_fo76_specular_bundle() {
        let tmp = std::env::temp_dir().join(format!(
            "bgsm_missing_smoothspec_uses_existing_fo76_specular_bundle_{}",
            std::process::id()
        ));
        let source = tmp.join("source");
        let out = tmp.join("mod");
        let material_dir = source
            .join("Materials")
            .join("SetDressing")
            .join("PlayerHouse_Ruin");
        let texture_dir = source
            .join("textures")
            .join("SetDressing")
            .join("PlayerHouse_Ruin");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&material_dir).unwrap();
        std::fs::create_dir_all(&texture_dir).unwrap();
        std::fs::write(
            texture_dir.join("playerhouse_ruin_kitchenrefrigerator01_r.DDS"),
            b"r",
        )
        .unwrap();
        std::fs::write(
            texture_dir.join("playerhouse_ruin_kitchenrefrigerator01_l.DDS"),
            b"l",
        )
        .unwrap();

        let mut material = make_test_bgsm(20);
        material.DiffuseTexture =
            "SetDressing/PlayerHouse_Ruin/playerhouse_ruin_kitchenrefrigerator05_d.dds".to_string();
        material.NormalTexture =
            "SetDressing/PlayerHouse_Ruin/playerhouse_ruin_kitchenrefrigerator05_n.dds".to_string();
        material.SmoothSpecTexture =
            "SetDressing/PlayerHouse_Ruin/playerhouse_ruin_kitchenrefrigerator05_s.dds".to_string();
        material.SpecularTexture = Some(
            "SetDressing/PlayerHouse_Ruin/playerhouse_ruin_kitchenrefrigerator01_r.DDS".to_string(),
        );
        material.LightingTexture = Some(
            "SetDressing/PlayerHouse_Ruin/playerhouse_ruin_kitchenrefrigerator01_l.DDS".to_string(),
        );
        let source_material = material_dir.join("PlayerHouse_Ruin_KitchenRefrigerator05.BGSM");
        std::fs::write(&source_material, crate::bgsm::write(&material)).unwrap();

        let request = ConvertMaterialsRequest {
            materials: vec![MaterialEntry {
                source_path:
                    "Materials/SetDressing/PlayerHouse_Ruin/PlayerHouse_Ruin_KitchenRefrigerator05.BGSM"
                        .to_string(),
                resolved_path: source_material.to_string_lossy().to_string(),
                is_cdb_ref: false,
                output_subpath: Some(
                    "Materials/FO76/SetDressing/PlayerHouse_Ruin/PlayerHouse_Ruin_KitchenRefrigerator05.BGSM"
                        .to_string(),
                ),
                texture_namespace: Some("FO76".to_string()),
                texture_namespace_paths: HashSet::new(),
            }],
            source_game: Some(Game::Fo76),
            target_game: Some(Game::Fo4),
            asset_prefix: String::new(),
            source_materialsdb: None,
            overwrite_existing: true,
            bgsm_default_overrides: vec![],
            convert_all: false,
            pbr_carry: false,
            source_path_overrides: HashMap::new(),
            target_asset_paths: HashSet::new(),
        };

        run_convert_materials(&out, &request, Game::Fo76, Game::Fo4, &source, None, None);

        let output = out
            .join("data")
            .join("Materials")
            .join("FO76")
            .join("SetDressing")
            .join("PlayerHouse_Ruin")
            .join("PlayerHouse_Ruin_KitchenRefrigerator05.BGSM");
        let parsed = crate::bgsm::parse(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(
            parsed
                .SmoothSpecTexture
                .trim_end_matches('\0')
                .to_ascii_lowercase(),
            "fo76/setdressing/playerhouse_ruin/playerhouse_ruin_kitchenrefrigerator01_s.dds"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ---- Fixture helpers ----

    fn make_test_bgsm_v20() -> crate::bgsm::BgsmData {
        make_test_bgsm(20)
    }

    fn make_test_bgsm_v2() -> crate::bgsm::BgsmData {
        make_test_bgsm(2)
    }

    fn make_test_bgsm(version: u32) -> crate::bgsm::BgsmData {
        use crate::base::BaseHeader;
        crate::bgsm::BgsmData {
            header: BaseHeader {
                signature: 0x4D534742,
                version,
                tile_u: false,
                tile_v: false,
                u_offset: 0.0,
                v_offset: 0.0,
                u_scale: 1.0,
                v_scale: 1.0,
                alpha: 1.0,
                alpha_blend_mode0: 0,
                alpha_blend_mode1: 6,
                alpha_blend_mode2: 7,
                alpha_test_ref: 128,
                alpha_test: false,
                zbuffer_write: true,
                zbuffer_test: true,
                ssr: false,
                wet_ssr: false,
                decal: false,
                two_sided: false,
                decal_nofade: false,
                non_occluder: false,
                refraction: false,
                refraction_falloff: false,
                refraction_power: 0.0,
                env_mapping: None,
                env_mapping_mask_scale: None,
                depth_bias: Some(false),
                grayscale_to_palette_color: false,
                mask_writes: Some(0),
            },
            DiffuseTexture: String::new(),
            NormalTexture: String::new(),
            SmoothSpecTexture: String::new(),
            GreyscaleTexture: String::new(),
            EnvmapTexture: None,
            GlowTexture: None,
            InnerLayerTexture: None,
            WrinklesTexture: None,
            DisplacementTexture: None,
            SpecularTexture: if version > 2 {
                Some(String::new())
            } else {
                None
            },
            LightingTexture: if version > 2 {
                Some(String::new())
            } else {
                None
            },
            FlowTexture: if version > 2 {
                Some(String::new())
            } else {
                None
            },
            DistanceFieldAlphaTexture: None,
            EnableEditorAlphaRef: false,
            RimLighting: if version < 8 { Some(false) } else { None },
            RimPower: if version < 8 { Some(0.0) } else { None },
            BackLightPower: if version < 8 { Some(0.0) } else { None },
            SubsurfaceLighting: if version < 8 { Some(false) } else { None },
            SubsurfaceLightingRolloff: if version < 8 { Some(0.0) } else { None },
            Translucency: if version >= 8 { Some(false) } else { None },
            TranslucencyThickObject: if version >= 8 { Some(false) } else { None },
            TranslucencyMixAlbedoWithSubsurfaceColor: if version >= 8 { Some(false) } else { None },
            TranslucencySubsurfaceColor: if version >= 8 {
                Some([1.0, 1.0, 1.0])
            } else {
                None
            },
            TranslucencyTransmissiveScale: if version >= 8 { Some(0.0) } else { None },
            TranslucencyTurbulence: if version >= 8 { Some(0.0) } else { None },
            SpecularEnabled: true,
            SpecularColor: [1.0, 1.0, 1.0],
            SpecularMult: 1.0,
            Smoothness: 0.5,
            FresnelPower: 5.0,
            WetnessControlSpecScale: 1.0,
            WetnessControlSpecPowerScale: 1.0,
            WetnessControlSpecMinvar: 0.0,
            WetnessControlEnvMapScale: if version < 10 { Some(1.0) } else { None },
            WetnessControlFresnelPower: 1.0,
            WetnessControlMetalness: 0.0,
            PBR: if version > 2 { Some(false) } else { None },
            CustomPorosity: if version >= 9 { Some(false) } else { None },
            PorosityValue: if version >= 9 { Some(0.0) } else { None },
            RootMaterialPath: String::new(),
            AnisoLighting: false,
            EmitEnabled: false,
            EmittanceColor: None,
            EmittanceMult: 1.0,
            ModelSpaceNormals: false,
            ExternalEmittance: false,
            LumEmittance: if version >= 12 { Some(0.0) } else { None },
            UseAdaptativeEmissive: None,
            AdaptativeEmissive_ExposureOffset: None,
            AdaptativeEmissive_FinalExposureMin: None,
            AdaptativeEmissive_FinalExposureMax: None,
            BackLighting: if version < 8 { Some(false) } else { None },
            ReceiveShadows: true,
            HideSecret: false,
            CastShadows: true,
            DissolveFade: false,
            AssumeShadowmask: false,
            Glowmap: false,
            EnvironmentMappingWindow: None,
            EnvironmentMappingEye: None,
            Hair: false,
            HairTintColor: [0.0, 0.0, 0.0],
            Tree: false,
            Facegen: false,
            SkinTint: false,
            Tessellate: false,
            DisplacementTextureBias: None,
            DisplacementTextureScale: None,
            TessellationPnScale: None,
            TessellationBaseFactor: None,
            TessellationFadeDistance: None,
            GrayscaleToPaletteScale: 1.0,
            SkewSpecularAlpha: Some(false),
            Terrain: Some(false),
            UnkInt1: None,
            TerrainThresholdFalloff: None,
            TerrainTilingDistance: None,
            TerrainRotationAngle: None,
        }
    }
}
