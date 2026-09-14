//! Loose Starfield `.mat` (JSON `BSComponentDB2` object graph) writer.
//!
//! Format contract: `bacup/docs/starfield_target/R5-mat-format.md`. Do NOT use
//! `creation_lib/material_tools/mat_writer.py` as a reference: it emits a
//! shape that does not exist in the 48,745-file vanilla corpus (R5 SD-1).
//!
//! Vanilla ships materials only in a compiled `materialsbeta.cdb` (R5 section
//! 1); BACUP emits loose `.mat` under `<mod>/Data/materials/...` (R5 1.4).

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::bgsm;
use crate::cdb::bethesda_crc32;

pub struct MatTextures {
    pub color: String,
    pub normal: String,
    pub rough: String,
    pub metal: String,
    pub ao: Option<String>,
    pub emissive: Option<String>,
}

pub struct MatSettings {
    pub two_sided: bool,
    pub alpha_test: Option<f32>,
    pub emissive_scale: f32,
}

/// Shader-model dependency for the opaque `1LayerStandard` template (R5 5.1).
const SHADER_MODEL_IMPORT: &str = "Data\\MATERIALS\\Layered\\ShaderModels\\1LayerStandard.mat";

/// Sub-object `Parent` ids, read directly out of vanilla's
/// `1LayerStandard.mat` (R5 5.1). 6,547-6,671 of 6,727 vanilla one-layer
/// materials use exactly these four.
const LAYER_PARENT: &str = "res:F316D5F5:0005A4F1:A487E721";
const MATERIAL_PARENT: &str = "res:F316D64C:0005A4F1:A487E721";
const TEXTURESET_PARENT: &str = "res:F316D666:0005A4F1:A487E721";
const UVSTREAM_PARENT: &str = "res:20150940:0005A709:A487E721";

/// "FO4SF" as ASCII bytes reinterpreted big-endian-hex. Outside vanilla's
/// entire 3rd `res:` component range (`0xA0000000..0xA8000000`), so ids this
/// crate mints can never collide with a base-game or Creation id (R5 2.3).
const ID_MAGIC: &str = "F04F5346";

/// FO4's `EmittanceMult` is a unitless multiplier; Starfield's
/// `LuminousEmittance` is physical cd/m^2. Starting calibration from R5
/// section 7: FO4's default mult of 1.0 lands on 250 cd/m^2, vanilla's
/// "bright glow" exemplar. Tune this if emissive intensity reads wrong in-game.
const EMISSIVE_CALIBRATION: f32 = 250.0;

fn object_id(crc: u32, slot: u8) -> String {
    format!("res:{crc:08X}:00F0000{slot}:{ID_MAGIC}")
}

fn material_name(mat_rel: &str) -> String {
    let file = mat_rel.rsplit(['/', '\\']).next().unwrap_or(mat_rel);
    if file.len() >= 4 && file[file.len() - 4..].eq_ignore_ascii_case(".mat") {
        file[..file.len() - 4].to_owned()
    } else {
        file.to_owned()
    }
}

/// Texture `FileName` values are `Data\`-prefixed, backslash-separated — the
/// opposite convention from the NIF-side material path (R5 section 3).
fn to_data_backslash_path(path: &str) -> String {
    let backslashed = path.replace('/', "\\");
    if backslashed.len() >= 5 && backslashed[..5].eq_ignore_ascii_case("data\\") {
        backslashed
    } else {
        format!("Data\\{backslashed}")
    }
}

/// Formats an f32 as the shortest decimal string that still reads back as
/// the intended value, matching vanilla's mixed integer/decimal scalar style
/// (`"250"`, `"0.333333"`).
fn format_scalar(value: f32) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.6}")
    }
}

fn texture_slot(index: u8, path: &str) -> Value {
    json!({
        "Data": { "FileName": to_data_backslash_path(path) },
        "Index": index,
        "Type": "BSMaterial::MRTextureFile",
        "Version": 1
    })
}

/// Emits the loose `.mat` JSON text for one material.
///
/// `mat_rel` is the on-disk output path in the `mat_out_rel` convention
/// (lowercase, forward slashes, e.g. `"materials/fo4sf/architecture/bldconcrete01.mat"`).
/// It seeds the `res:` id CRC (R5 2.3) and is the same string lowercased into
/// the NIF's `MaterialID` CRC (R5 section 4), so the two CRCs agree.
/// `seen_crcs` is threaded by the caller across a writer session to reject
/// id collisions.
pub fn write_starfield_mat(
    mat_rel: &str,
    tex: &MatTextures,
    settings: &MatSettings,
    seen_crcs: &mut HashSet<u32>,
) -> Result<String, String> {
    let crc = bethesda_crc32(mat_rel.to_ascii_lowercase().as_bytes());
    if !seen_crcs.insert(crc) {
        return Err(format!(
            "CRC32 collision writing '{mat_rel}': id 0x{crc:08X} was already minted by \
             another material in this writer session (R5 section 2.3 requires unique \
             res: ids)"
        ));
    }

    let layer_id = object_id(crc, 1);
    let material_id = object_id(crc, 2);
    let textureset_id = object_id(crc, 3);
    let uvstream_id = object_id(crc, 4);
    let name = material_name(mat_rel);

    // --- root object: the material itself (BSMaterial::LayeredMaterial, no ID) ---
    let mut root_components = vec![
        json!({ "Data": { "Name": name }, "Index": 0, "Type": "BSComponentDB::CTName" }),
        json!({ "Data": { "ID": layer_id }, "Index": 0, "Type": "BSMaterial::LayerID" }),
    ];
    if settings.two_sided {
        // R5 section 6: `BSMaterial::ParamBool` Index:0 on the root == TwoSided
        // (proven against the stock TwoSided1Layer/1LayerStandard shader
        // models). Only 30/48,745 vanilla files set this directly; most pick
        // the TwoSided1Layer shader model instead. If this renders one-sided
        // in-game, switch `Import`/root `Parent` to
        // `Data\MATERIALS\Layered\ShaderModels\TwoSided1Layer.mat` and its
        // own four sub-object ids (`res:C191080D:0005B0ED:A06346EA` Layer,
        // `res:C1910B21:0005B0ED:A06346EA` Material,
        // `res:C1910DB3:0005B0ED:A06346EA` TextureSet,
        // `res:C19109A9:0005B0ED:A06346EA` UVStream) instead of this flag.
        root_components.push(json!({
            "Data": { "Value": "true" },
            "Index": 0,
            "Type": "BSMaterial::ParamBool"
        }));
    }
    if let Some(threshold) = settings.alpha_test {
        // HasOpacity is the master switch (R5 section 6) — setting only
        // AlphaTestThreshold renders fully opaque, so both are always
        // emitted together here.
        root_components.push(json!({
            "Data": {
                "AlphaTestThreshold": format_scalar(threshold),
                "HasOpacity": "true"
            },
            "Index": 0,
            "Type": "BSMaterial::AlphaSettingsComponent"
        }));
    }
    if tex.emissive.is_some() {
        root_components.push(json!({
            "Data": {
                "Enabled": "true",
                "Settings": {
                    "Data": {
                        // No BGSM EmittanceColor field flows through
                        // MatSettings (R5 section 7 pins only the
                        // LuminousEmittance calibration); white tint lets
                        // the emissive texture's own color carry through
                        // unmodified.
                        "EmissiveTint": {
                            "Data": {
                                "Value": {
                                    "Data": { "w": "1", "x": "1", "y": "1", "z": "1" },
                                    "Type": "XMFLOAT4"
                                }
                            },
                            "Type": "BSMaterial::Color",
                            "Version": 1
                        },
                        "LuminousEmittance": format_scalar(settings.emissive_scale)
                    },
                    "Type": "BSMaterial::EmittanceSettings"
                }
            },
            "Index": 0,
            "Type": "BSMaterial::EmissiveSettingsComponent",
            "Version": 1
        }));
    }
    let root = json!({
        "Components": root_components,
        "Parent": SHADER_MODEL_IMPORT
    });

    // --- Layer object ---
    let layer = json!({
        "Components": [
            { "Data": { "Name": format!("{name}_Layer1") }, "Index": 0, "Type": "BSComponentDB::CTName" },
            { "Data": { "ID": material_id }, "Index": 0, "Type": "BSMaterial::MaterialID" },
            { "Data": { "ID": uvstream_id }, "Index": 0, "Type": "BSMaterial::UVStreamID" }
        ],
        "Edges": [ { "EdgeIndex": 0, "To": "<this>", "Type": "BSComponentDB2::OuterEdge" } ],
        "ID": layer_id,
        "Parent": LAYER_PARENT
    });

    // --- Material object ---
    let material = json!({
        "Components": [
            { "Data": { "Name": format!("{name}_Material1") }, "Index": 0, "Type": "BSComponentDB::CTName" },
            { "Data": { "ID": textureset_id }, "Index": 0, "Type": "BSMaterial::TextureSetID" }
        ],
        "Edges": [ { "EdgeIndex": 0, "To": layer_id, "Type": "BSComponentDB2::OuterEdge" } ],
        "ID": material_id,
        "Parent": MATERIAL_PARENT
    });

    // --- TextureSet object ---
    // Slots 0/1/3/4 always emitted; slot 7 only when an emissive texture
    // exists. Slot 5 (AO) is never emitted: FO4 has no AO source and the
    // shader model's inherited default is white (R5 9.4). Slot 2 (Opacity) is
    // not emitted either; MatTextures has no alpha-channel path, and alpha
    // testing goes through AlphaSettingsComponent above.
    let mut ts_components = vec![
        json!({ "Data": { "Name": format!("{name}_TextureSet1") }, "Index": 0, "Type": "BSComponentDB::CTName" }),
        texture_slot(0, &tex.color),
        texture_slot(1, &tex.normal),
        texture_slot(3, &tex.rough),
        texture_slot(4, &tex.metal),
    ];
    if let Some(emissive) = &tex.emissive {
        ts_components.push(texture_slot(7, emissive));
    }
    let textureset = json!({
        "Components": ts_components,
        "Edges": [ { "EdgeIndex": 0, "To": material_id, "Type": "BSComponentDB2::OuterEdge" } ],
        "ID": textureset_id,
        "Parent": TEXTURESET_PARENT
    });

    // --- UVStream object (empty = identity UV: channel 0, scale 1, offset 0) ---
    let uvstream = json!({
        "Components": [
            { "Data": { "Name": format!("{name}_UVStream1") }, "Index": 0, "Type": "BSComponentDB::CTName" }
        ],
        "Edges": [ { "EdgeIndex": 0, "To": layer_id, "Type": "BSComponentDB2::OuterEdge" } ],
        "ID": uvstream_id,
        "Parent": UVSTREAM_PARENT
    });

    let doc = json!({
        "Import": [SHADER_MODEL_IMPORT],
        "Objects": [root, layer, material, textureset, uvstream],
        "Version": 1
    });

    serde_json::to_string_pretty(&doc).map_err(|err| err.to_string())
}

fn split_ext(path: &str) -> (&str, &str) {
    let name_start = path.rfind(['/', '\\']).map_or(0, |pos| pos + 1);
    match path[name_start..].rfind('.') {
        Some(rel_pos) => path.split_at(name_start + rel_pos),
        None => (path, ""),
    }
}

fn strip_suffix_ci<'a>(stem: &'a str, suffix: &str) -> Option<&'a str> {
    if stem.len() > suffix.len() && stem[stem.len() - suffix.len()..].eq_ignore_ascii_case(suffix) {
        Some(&stem[..stem.len() - suffix.len()])
    } else {
        None
    }
}

/// Strips a known FO4 texture-role suffix (e.g. `_d`, `_n`, `_s`, `_g`) and
/// extension off a texture path, yielding the shared "family" base name that
/// per-slot output suffixes (`_color`, `_normal`, `_rough`, ...) attach to.
fn family_base(path: &str, known_suffixes: &[&str]) -> String {
    let (stem, _ext) = split_ext(path);
    for suffix in known_suffixes {
        if let Some(base) = strip_suffix_ci(stem, suffix) {
            return base.to_owned();
        }
    }
    stem.to_owned()
}

/// Maps a parsed BGSM to the writer's texture/setting inputs using the
/// FO4->Starfield suffix convention (`_d`->`_color`, `_n`->`_normal`, derived
/// `_rough`/`_metal` from the packed spec/gloss map, `_g`->`_emissive`).
/// Output paths carry no `Data\` prefix and use forward slashes, matching
/// the `mat_out_rel` convention (R5 section 4).
pub fn map_bgsm_to_mat_inputs(bgsm_bytes: &[u8]) -> Result<(MatTextures, MatSettings), String> {
    let data = bgsm::parse(bgsm_bytes).map_err(|err| err.to_string())?;

    if data.DiffuseTexture.is_empty() {
        return Err("BGSM has no DiffuseTexture; cannot derive a Starfield texture set".into());
    }
    let diffuse_base = family_base(&data.DiffuseTexture, &["_d"]);
    let color = format!("{diffuse_base}_color.dds");

    let normal_base = if data.NormalTexture.is_empty() {
        diffuse_base.clone()
    } else {
        family_base(&data.NormalTexture, &["_n"])
    };
    let normal = format!("{normal_base}_normal.dds");

    let spec_base = if data.SmoothSpecTexture.is_empty() {
        diffuse_base.clone()
    } else {
        family_base(&data.SmoothSpecTexture, &["_s"])
    };
    let rough = format!("{spec_base}_rough.dds");
    let metal = format!("{spec_base}_metal.dds");

    // No AO source exists in BGSM; slot 5 is never emitted anyway (R5 9.4).
    let ao = None;

    let emissive = data
        .GlowTexture
        .as_deref()
        .filter(|glow| !glow.is_empty())
        .map(|glow| format!("{}_emissive.dds", family_base(glow, &["_g"])));

    let tex = MatTextures {
        color,
        normal,
        rough,
        metal,
        ao,
        emissive,
    };

    let alpha_test = if data.header.alpha_test {
        Some(f32::from(data.header.alpha_test_ref) / 255.0)
    } else {
        None
    };
    let emissive_scale = data.EmittanceMult.clamp(0.0, 100.0) * EMISSIVE_CALIBRATION;

    let settings = MatSettings {
        two_sided: data.header.two_sided,
        alpha_test,
        emissive_scale,
    };

    Ok((tex, settings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::BaseHeader;
    use crate::bgsm::BgsmData;

    fn opaque_textures() -> MatTextures {
        MatTextures {
            color: "textures/fo4sf/bldconcrete01_color.dds".to_owned(),
            normal: "textures/fo4sf/bldconcrete01_normal.dds".to_owned(),
            rough: "textures/fo4sf/bldconcrete01_rough.dds".to_owned(),
            metal: "textures/fo4sf/bldconcrete01_metal.dds".to_owned(),
            ao: None,
            emissive: None,
        }
    }

    fn opaque_settings() -> MatSettings {
        MatSettings {
            two_sided: false,
            alpha_test: None,
            emissive_scale: 0.0,
        }
    }

    fn parent_constants() -> [&'static str; 4] {
        [
            LAYER_PARENT,
            MATERIAL_PARENT,
            TEXTURESET_PARENT,
            UVSTREAM_PARENT,
        ]
    }

    #[test]
    fn opaque_output_parses_and_matches_r5_5_3_structure() {
        let mut seen = HashSet::new();
        let text = write_starfield_mat(
            "materials/fo4sf/architecture/buildings/bldconcrete01.mat",
            &opaque_textures(),
            &opaque_settings(),
            &mut seen,
        )
        .expect("write succeeds");

        let doc: Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(doc["Version"], 1);
        assert_eq!(doc["Import"][0], SHADER_MODEL_IMPORT);

        let objects = doc["Objects"].as_array().expect("Objects array");
        assert_eq!(
            objects.len(),
            5,
            "root + Layer/Material/TextureSet/UVStream"
        );

        let roots: Vec<_> = objects.iter().filter(|o| o.get("ID").is_none()).collect();
        assert_eq!(roots.len(), 1, "exactly one root object without an ID");
        assert_eq!(roots[0]["Parent"], SHADER_MODEL_IMPORT);

        let sub_objects: Vec<_> = objects.iter().filter(|o| o.get("ID").is_some()).collect();
        assert_eq!(sub_objects.len(), 4);

        let mut ids = HashSet::new();
        let mut parents = HashSet::new();
        for object in &sub_objects {
            ids.insert(object["ID"].as_str().unwrap().to_owned());
            parents.insert(object["Parent"].as_str().unwrap().to_owned());
        }
        assert_eq!(ids.len(), 4, "four distinct object ids");

        let expected_parents: HashSet<String> =
            parent_constants().iter().map(|s| s.to_string()).collect();
        assert_eq!(
            parents, expected_parents,
            "exactly the four R5 5.1 parent constants"
        );

        // Every Edges[].To other than "<this>" resolves to one of the ids.
        for object in objects {
            if let Some(edges) = object.get("Edges").and_then(Value::as_array) {
                for edge in edges {
                    let to = edge["To"].as_str().unwrap();
                    if to != "<this>" {
                        assert!(ids.contains(to), "dangling edge target {to}");
                    }
                }
            }
        }
    }

    #[test]
    fn scalars_inside_components_data_are_json_strings() {
        let mut seen = HashSet::new();
        let text = write_starfield_mat(
            "materials/fo4sf/scalars_probe.mat",
            &opaque_textures(),
            &MatSettings {
                two_sided: true,
                alpha_test: Some(0.333_333),
                emissive_scale: 250.0,
            },
            &mut seen,
        )
        .expect("write succeeds");
        let doc: Value = serde_json::from_str(&text).expect("valid JSON");

        // Index/Version are real numbers everywhere.
        for object in doc["Objects"].as_array().unwrap() {
            for component in object["Components"].as_array().unwrap() {
                assert!(component["Index"].is_number(), "Index must be a number");
                if let Some(version) = component.get("Version") {
                    assert!(version.is_number(), "Version must be a number");
                }
            }
        }

        let root = doc["Objects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o.get("ID").is_none())
            .unwrap();
        let mut saw_alpha = false;
        let mut saw_two_sided = false;
        for component in root["Components"].as_array().unwrap() {
            if component["Type"] == "BSMaterial::AlphaSettingsComponent" {
                saw_alpha = true;
                assert!(component["Data"]["HasOpacity"].is_string());
                assert!(component["Data"]["AlphaTestThreshold"].is_string());
                assert_eq!(component["Data"]["AlphaTestThreshold"], "0.333333");
                assert_eq!(component["Data"]["HasOpacity"], "true");
            }
            if component["Type"] == "BSMaterial::ParamBool" {
                saw_two_sided = true;
                assert!(component["Data"]["Value"].is_string());
                assert_eq!(component["Data"]["Value"], "true");
            }
        }
        assert!(saw_alpha && saw_two_sided);
    }

    #[test]
    fn crc_collision_guard_trips() {
        let mut seen = HashSet::new();
        write_starfield_mat(
            "materials/fo4sf/dup.mat",
            &opaque_textures(),
            &opaque_settings(),
            &mut seen,
        )
        .expect("first write succeeds");

        let err = write_starfield_mat(
            "materials/fo4sf/dup.mat",
            &opaque_textures(),
            &opaque_settings(),
            &mut seen,
        )
        .expect_err("second write with the same path must fail loudly");
        assert!(err.contains("collision"), "{err}");
    }

    fn fixture_bgsm_bytes() -> Vec<u8> {
        let header = BaseHeader {
            signature: bgsm::BGSM_SIGNATURE,
            version: 20,
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
            alpha_test: true,
            zbuffer_write: true,
            zbuffer_test: true,
            ssr: false,
            wet_ssr: false,
            decal: false,
            two_sided: true,
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
        };
        let data = BgsmData {
            header,
            DiffuseTexture: "foo_d.dds".to_owned(),
            NormalTexture: "foo_n.dds".to_owned(),
            SmoothSpecTexture: "foo_s.dds".to_owned(),
            GreyscaleTexture: String::new(),
            EnvmapTexture: None,
            GlowTexture: Some("foo_g.dds".to_owned()),
            InnerLayerTexture: None,
            WrinklesTexture: Some(String::new()),
            DisplacementTexture: None,
            SpecularTexture: Some(String::new()),
            LightingTexture: Some(String::new()),
            FlowTexture: Some(String::new()),
            DistanceFieldAlphaTexture: None,
            EnableEditorAlphaRef: false,
            RimLighting: None,
            RimPower: None,
            BackLightPower: None,
            SubsurfaceLighting: None,
            SubsurfaceLightingRolloff: None,
            Translucency: Some(false),
            TranslucencyThickObject: Some(false),
            TranslucencyMixAlbedoWithSubsurfaceColor: Some(false),
            TranslucencySubsurfaceColor: Some([1.0, 1.0, 1.0]),
            TranslucencyTransmissiveScale: Some(0.0),
            TranslucencyTurbulence: Some(0.0),
            SpecularEnabled: true,
            SpecularColor: [1.0, 1.0, 1.0],
            SpecularMult: 1.0,
            Smoothness: 0.5,
            FresnelPower: 5.0,
            WetnessControlSpecScale: 1.0,
            WetnessControlSpecPowerScale: 1.0,
            WetnessControlSpecMinvar: 0.0,
            WetnessControlEnvMapScale: None,
            WetnessControlFresnelPower: 1.0,
            WetnessControlMetalness: 0.0,
            PBR: Some(true),
            CustomPorosity: Some(false),
            PorosityValue: Some(0.0),
            RootMaterialPath: String::new(),
            AnisoLighting: false,
            EmitEnabled: true,
            EmittanceColor: Some([1.0, 1.0, 1.0]),
            EmittanceMult: 1.0,
            ModelSpaceNormals: false,
            ExternalEmittance: false,
            LumEmittance: Some(0.0),
            UseAdaptativeEmissive: None,
            AdaptativeEmissive_ExposureOffset: None,
            AdaptativeEmissive_FinalExposureMin: None,
            AdaptativeEmissive_FinalExposureMax: None,
            BackLighting: None,
            ReceiveShadows: true,
            HideSecret: false,
            CastShadows: true,
            DissolveFade: false,
            AssumeShadowmask: false,
            Glowmap: false,
            EnvironmentMappingWindow: None,
            EnvironmentMappingEye: None,
            Hair: false,
            HairTintColor: [1.0, 1.0, 1.0],
            Tree: false,
            Facegen: false,
            SkinTint: false,
            Tessellate: false,
            DisplacementTextureBias: None,
            DisplacementTextureScale: None,
            TessellationPnScale: None,
            TessellationBaseFactor: None,
            TessellationFadeDistance: None,
            GrayscaleToPaletteScale: 0.0,
            SkewSpecularAlpha: Some(false),
            Terrain: Some(false),
            UnkInt1: None,
            TerrainThresholdFalloff: None,
            TerrainTilingDistance: None,
            TerrainRotationAngle: None,
        };
        bgsm::write(&data)
    }

    #[test]
    fn map_bgsm_to_mat_inputs_derives_suffixes_and_flags() {
        let bytes = fixture_bgsm_bytes();
        let (tex, settings) = map_bgsm_to_mat_inputs(&bytes).expect("maps fixture BGSM");

        assert_eq!(tex.color, "foo_color.dds");
        assert_eq!(tex.normal, "foo_normal.dds");
        assert_eq!(tex.rough, "foo_rough.dds");
        assert_eq!(tex.metal, "foo_metal.dds");
        assert_eq!(tex.emissive.as_deref(), Some("foo_emissive.dds"));
        assert!(tex.ao.is_none());

        assert!(settings.two_sided);
        let threshold = settings.alpha_test.expect("alpha test enabled in fixture");
        assert!((threshold - 128.0 / 255.0).abs() < 1e-6);
        assert!((settings.emissive_scale - 250.0).abs() < 1e-6);
    }

    #[test]
    fn alpha_tested_variant_sets_has_opacity_and_threshold_string() {
        let mut seen = HashSet::new();
        let settings = MatSettings {
            two_sided: false,
            alpha_test: Some(0.333_333),
            emissive_scale: 0.0,
        };
        let text = write_starfield_mat(
            "materials/fo4sf/landscape/trees/treecanopy01.mat",
            &opaque_textures(),
            &settings,
            &mut seen,
        )
        .expect("write succeeds");
        let doc: Value = serde_json::from_str(&text).unwrap();

        let root = doc["Objects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o.get("ID").is_none())
            .unwrap();
        let alpha_component = root["Components"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["Type"] == "BSMaterial::AlphaSettingsComponent")
            .expect("AlphaSettingsComponent present");

        assert_eq!(alpha_component["Data"]["HasOpacity"], "true");
        assert_eq!(alpha_component["Data"]["AlphaTestThreshold"], "0.333333");
    }

    #[test]
    fn emissive_variant_includes_emittance_settings_and_slot_seven() {
        let mut seen = HashSet::new();
        let mut tex = opaque_textures();
        tex.emissive = Some("textures/fo4sf/neonsign01_emissive.dds".to_owned());
        let settings = MatSettings {
            two_sided: false,
            alpha_test: None,
            emissive_scale: 250.0,
        };
        let text = write_starfield_mat(
            "materials/fo4sf/setdressing/signage/neonsign01.mat",
            &tex,
            &settings,
            &mut seen,
        )
        .expect("write succeeds");
        let doc: Value = serde_json::from_str(&text).unwrap();

        let root = doc["Objects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o.get("ID").is_none())
            .unwrap();
        let emissive_component = root["Components"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["Type"] == "BSMaterial::EmissiveSettingsComponent")
            .expect("EmissiveSettingsComponent present");
        assert_eq!(emissive_component["Data"]["Enabled"], "true");
        assert_eq!(
            emissive_component["Data"]["Settings"]["Data"]["LuminousEmittance"],
            "250"
        );

        let textureset = doc["Objects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["Parent"] == TEXTURESET_PARENT)
            .expect("TextureSet object present");
        let has_slot_seven = textureset["Components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["Type"] == "BSMaterial::MRTextureFile" && c["Index"] == 7);
        assert!(has_slot_seven, "slot 7 emissive texture must be present");
    }

    #[test]
    fn never_emits_ao_slot_five() {
        let mut seen = HashSet::new();
        let mut tex = opaque_textures();
        tex.ao = Some("textures/fo4sf/white_ao.dds".to_owned());
        let text = write_starfield_mat(
            "materials/fo4sf/ao_probe.mat",
            &tex,
            &opaque_settings(),
            &mut seen,
        )
        .expect("write succeeds");
        assert!(
            !text.contains("\"Index\": 5"),
            "slot 5 (AO) must never be emitted"
        );
    }

    #[test]
    fn texture_slots_use_mr_texture_file_never_texture_file() {
        let mut seen = HashSet::new();
        let text = write_starfield_mat(
            "materials/fo4sf/slot_probe.mat",
            &opaque_textures(),
            &opaque_settings(),
            &mut seen,
        )
        .expect("write succeeds");
        assert!(text.contains("BSMaterial::MRTextureFile"));
        assert!(!text.contains("\"BSMaterial::TextureFile\""));
    }
}
