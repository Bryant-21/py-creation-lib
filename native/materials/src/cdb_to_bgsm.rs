// CE2Material (Starfield .mat / MaterialsDB.cdb) -> BGSM translation.
//
// Native counterpart of `creation_lib.material_tools.cdb_to_bgsm.cdb_to_bgsm`.
// CDB parse, CE2 projection, PBR -> spec-gloss math and the BGSM write live in
// this crate's `cdb`, `ce2`, `pbr` and `bgsm` modules; this module holds the
// field-population logic that glues them together.
//
// Emits an FO76-shaped BGSM at `TARGET_BGSM_VERSION`, even for a Starfield
// source. The caller downgrades to FO4 via `downgrade_bgsm`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::base::BaseHeader;
use crate::bgsm::{self, BGSM_SIGNATURE, BgsmData};
use crate::cdb::{self, CdbPayload, ResourceId};
use crate::ce2::{self, Ce2MaterialPayload};
use crate::pbr::{self, PbrToSpecGlossParams};

const TARGET_BGSM_VERSION: u32 = 22;

struct CachedCdb {
    payload: CdbPayload,
    by_resource_id: HashMap<ResourceId, u32>,
}

fn cdb_cache() -> &'static Mutex<HashMap<PathBuf, Arc<CachedCdb>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<CachedCdb>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn load_cdb(cdb_path: &Path) -> Result<Arc<CachedCdb>, String> {
    let cache = cdb_cache();
    if let Some(cached) = cache
        .lock()
        .map_err(|_| "cdb cache lock poisoned".to_string())?
        .get(cdb_path)
    {
        return Ok(Arc::clone(cached));
    }

    let bytes = std::fs::read(cdb_path)
        .map_err(|e| format!("failed to read {}: {e}", cdb_path.display()))?;
    let payload = cdb::parse_cdb(&bytes).map_err(|e| e.to_string())?;
    let mut by_resource_id = HashMap::with_capacity(payload.objects.len());
    for obj in &payload.objects {
        let rid = obj.persistent_id;
        if rid.dir != 0 || rid.file != 0 || rid.ext != 0 {
            by_resource_id.insert(rid, obj.db_id);
        }
    }
    let cached = Arc::new(CachedCdb {
        payload,
        by_resource_id,
    });

    let mut guard = cache
        .lock()
        .map_err(|_| "cdb cache lock poisoned".to_string())?;
    Ok(Arc::clone(
        guard
            .entry(cdb_path.to_path_buf())
            .or_insert_with(|| cached),
    ))
}

/// Translate a Starfield `.mat` reference into FO76-shaped BGSM bytes.
///
/// `mat_path` is the game-relative material path (e.g.
/// `materials\architecture\...\foo.mat`), resolved against `cdb_path` the
/// same way `MaterialsCDB.lookup_by_path` does.
pub fn cdb_to_bgsm(cdb_path: &Path, mat_path: &str) -> Result<Vec<u8>, String> {
    let cached = load_cdb(cdb_path)?;
    let canonical_path = canonical_material_path(mat_path);
    let db_id = [mat_path, canonical_path.as_str()]
        .into_iter()
        .find_map(|candidate| {
            cached
                .by_resource_id
                .get(&cdb::resource_id_from_path(candidate))
                .copied()
        })
        .ok_or_else(|| {
            format!("material not found in cdb: {mat_path} (canonical: {canonical_path})")
        })?;
    let material = ce2::project(&cached.payload, db_id)
        .ok_or_else(|| format!("failed to project CE2 material for {mat_path}"))?;
    let bgsm = build_bgsm(&material, mat_path)?;
    Ok(bgsm::write(&bgsm))
}

fn canonical_material_path(path: &str) -> String {
    let normalized = path
        .trim_matches(|c: char| c.is_ascii_whitespace() || c == '\0')
        .replace('/', "\\");
    let without_data = if normalized
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data\\"))
    {
        &normalized[5..]
    } else {
        &normalized
    };
    if without_data
        .get(..10)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("materials\\"))
    {
        without_data.to_owned()
    } else {
        format!("Materials\\{without_data}")
    }
}

fn build_bgsm(material: &Ce2MaterialPayload, mat_path: &str) -> Result<BgsmData, String> {
    // Highest-index layer == topmost == last painted; multi-layer materials
    // collapse to it, blenders and LOD materials are dropped (FO4 has no
    // equivalent of either).
    let layer = material
        .layers
        .last()
        .ok_or_else(|| format!("CE2Material {mat_path:?} has no layers"))?;

    let params = PbrToSpecGlossParams {
        ao_multiplier: 1.0,
        specular_multiplier: 1.0,
        gloss_multiplier: 1.0,
        spec_offset: 0.0,
    };
    let albedo_bytes = pbr::f32_vec_to_bytes(&[1.0, 1.0, 1.0]);
    let metallic_bytes = pbr::f32_vec_to_bytes(&[layer.material.metalness]);
    let roughness_bytes = pbr::f32_vec_to_bytes(&[1.0 - layer.material.smoothness]);
    let converted = pbr::convert_buffers(
        &albedo_bytes,
        &metallic_bytes,
        &roughness_bytes,
        None,
        1,
        params,
    )
    .map_err(|e| e.to_string())?;
    let spec_color = [
        converted.specular[0],
        converted.specular[1],
        converted.specular[2],
    ];
    let smoothness = converted.gloss[0];

    let header = default_header(TARGET_BGSM_VERSION);
    let mut bgsm = default_bgsm(header);

    bgsm.DiffuseTexture = layer.texture_set.diffuse.clone();
    bgsm.NormalTexture = layer.texture_set.normal.clone();

    bgsm.SpecularColor = spec_color;
    bgsm.Smoothness = smoothness;

    if layer.material.emissive_multiplier > 0.0 && layer.material.emissive_color != [0.0, 0.0, 0.0]
    {
        bgsm.EmitEnabled = true;
        bgsm.EmittanceColor = Some(layer.material.emissive_color);
        bgsm.EmittanceMult = layer.material.emissive_multiplier;
    }

    bgsm.header.alpha = layer.material.alpha;

    Ok(bgsm)
}

fn default_header(version: u32) -> BaseHeader {
    BaseHeader {
        signature: BGSM_SIGNATURE,
        version,
        tile_u: true,
        tile_v: true,
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
        // version (22) >= 10, so env_mapping/env_mapping_mask_scale are dead
        // and depth_bias is the live field.
        env_mapping: None,
        env_mapping_mask_scale: None,
        depth_bias: Some(false),
        grayscale_to_palette_color: false,
        mask_writes: Some(63),
    }
}

fn default_bgsm(header: BaseHeader) -> BgsmData {
    let v = header.version;
    let is_fo76 = v > 2;
    BgsmData {
        header,
        DiffuseTexture: String::new(),
        NormalTexture: String::new(),
        SmoothSpecTexture: String::new(),
        GreyscaleTexture: String::new(),
        EnvmapTexture: if is_fo76 { None } else { Some(String::new()) },
        GlowTexture: if is_fo76 { Some(String::new()) } else { None },
        InnerLayerTexture: if is_fo76 { None } else { Some(String::new()) },
        WrinklesTexture: if is_fo76 { Some(String::new()) } else { None },
        DisplacementTexture: if is_fo76 { None } else { Some(String::new()) },
        SpecularTexture: if is_fo76 { Some(String::new()) } else { None },
        LightingTexture: if is_fo76 { Some(String::new()) } else { None },
        FlowTexture: if is_fo76 { Some(String::new()) } else { None },
        DistanceFieldAlphaTexture: if v >= 17 { Some(String::new()) } else { None },
        EnableEditorAlphaRef: false,
        RimLighting: if v >= 8 { None } else { Some(false) },
        RimPower: if v >= 8 { None } else { Some(2.0) },
        BackLightPower: if v >= 8 { None } else { Some(0.0) },
        SubsurfaceLighting: if v >= 8 { None } else { Some(false) },
        SubsurfaceLightingRolloff: if v >= 8 { None } else { Some(0.3) },
        Translucency: if v >= 8 { Some(false) } else { None },
        TranslucencyThickObject: if v >= 8 { Some(false) } else { None },
        TranslucencyMixAlbedoWithSubsurfaceColor: if v >= 8 { Some(false) } else { None },
        TranslucencySubsurfaceColor: if v >= 8 { Some([1.0, 1.0, 1.0]) } else { None },
        TranslucencyTransmissiveScale: if v >= 8 { Some(0.0) } else { None },
        TranslucencyTurbulence: if v >= 8 { Some(0.0) } else { None },
        SpecularEnabled: true,
        SpecularColor: [1.0, 1.0, 1.0],
        SpecularMult: 1.0,
        Smoothness: 0.5,
        FresnelPower: 5.0,
        WetnessControlSpecScale: -0.95,
        WetnessControlSpecPowerScale: 0.5,
        WetnessControlSpecMinvar: 0.2,
        WetnessControlEnvMapScale: if v >= 10 { None } else { Some(1.0) },
        WetnessControlFresnelPower: 1.6,
        WetnessControlMetalness: 0.0,
        PBR: if is_fo76 { Some(true) } else { None },
        CustomPorosity: if v >= 9 { Some(false) } else { None },
        PorosityValue: if v >= 9 { Some(0.0) } else { None },
        RootMaterialPath: String::new(),
        AnisoLighting: false,
        EmitEnabled: false,
        EmittanceColor: None,
        EmittanceMult: 1.0,
        ModelSpaceNormals: false,
        ExternalEmittance: false,
        LumEmittance: if v >= 12 { Some(0.0) } else { None },
        UseAdaptativeEmissive: if v >= 13 { Some(false) } else { None },
        AdaptativeEmissive_ExposureOffset: if v >= 13 { Some(0.0) } else { None },
        AdaptativeEmissive_FinalExposureMin: if v >= 13 { Some(0.0) } else { None },
        AdaptativeEmissive_FinalExposureMax: if v >= 13 { Some(0.0) } else { None },
        BackLighting: if v >= 8 { None } else { Some(false) },
        ReceiveShadows: true,
        HideSecret: false,
        CastShadows: true,
        DissolveFade: false,
        AssumeShadowmask: false,
        Glowmap: false,
        EnvironmentMappingWindow: if v < 7 { Some(false) } else { None },
        EnvironmentMappingEye: if v < 7 { Some(false) } else { None },
        Hair: false,
        HairTintColor: [0.0, 0.0, 0.0],
        Tree: false,
        Facegen: false,
        SkinTint: false,
        Tessellate: false,
        DisplacementTextureBias: if v < 3 { Some(0.0) } else { None },
        DisplacementTextureScale: if v < 3 { Some(0.0) } else { None },
        TessellationPnScale: if v < 3 { Some(0.0) } else { None },
        TessellationBaseFactor: if v < 3 { Some(0.0) } else { None },
        TessellationFadeDistance: if v < 3 { Some(0.0) } else { None },
        GrayscaleToPaletteScale: 1.0,
        SkewSpecularAlpha: if v >= 1 { Some(false) } else { None },
        Terrain: if v >= 3 { Some(false) } else { None },
        UnkInt1: None,
        TerrainThresholdFalloff: None,
        TerrainTilingDistance: None,
        TerrainRotationAngle: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starfield_materials_cdb() -> PathBuf {
        let root = std::env::var_os("STARFIELD_EXTRACTED_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/starfield")
            });
        root.join("materials/materialsbeta.cdb")
    }
    use crate::ce2::{
        Ce2LayerPayload, Ce2MaterialPropsPayload, Ce2TextureSetPayload, Ce2UvStreamPayload,
    };

    fn layer(smoothness: f32, metalness: f32) -> Ce2LayerPayload {
        Ce2LayerPayload {
            material: Ce2MaterialPropsPayload {
                smoothness,
                metalness,
                emissive_color: [0.0, 0.0, 0.0],
                emissive_multiplier: 0.0,
                alpha: 1.0,
            },
            texture_set: Ce2TextureSetPayload {
                diffuse: "tex_d.dds".to_owned(),
                normal: "tex_n.dds".to_owned(),
                opacity: String::new(),
                rough: String::new(),
                metal: String::new(),
                ao: String::new(),
                emissive: String::new(),
            },
            uv_stream: Ce2UvStreamPayload {
                scale_u: 1.0,
                scale_v: 1.0,
                offset_u: 0.0,
                offset_v: 0.0,
                channel: 0,
            },
            name: String::new(),
        }
    }

    fn material(layers: Vec<Ce2LayerPayload>) -> Ce2MaterialPayload {
        Ce2MaterialPayload {
            name: "test".to_owned(),
            object_path: String::new(),
            layers,
            blenders: Vec::new(),
            lod_materials: Vec::new(),
        }
    }

    #[test]
    fn empty_layers_errors() {
        let mat = material(Vec::new());
        let err = build_bgsm(&mat, "materials\\bad.mat").expect_err("no layers");
        assert!(err.contains("has no layers"), "{err}");
    }

    #[test]
    fn single_layer_lands_at_fo76_v22() {
        let mat = material(vec![layer(0.6, 0.0)]);
        let bgsm = build_bgsm(&mat, "materials\\gun.mat").expect("builds");
        assert_eq!(bgsm.header.version, 22);
        assert_eq!(bgsm.DiffuseTexture, "tex_d.dds");
        assert_eq!(bgsm.NormalTexture, "tex_n.dds");
        // roughness = 1 - 0.6 = 0.4 -> gloss = 0.6 at neutral multipliers.
        assert!((bgsm.Smoothness - 0.6).abs() < 1e-5, "{}", bgsm.Smoothness);
    }

    #[test]
    fn top_layer_wins_on_multi_layer_material() {
        let mat = material(vec![layer(0.2, 0.0), layer(0.9, 0.0)]);
        let bgsm = build_bgsm(&mat, "materials\\multi.mat").expect("builds");
        // Only the last (highest-index) layer's smoothness feeds the gloss
        // conversion: roughness = 1 - 0.9 = 0.1 -> gloss = 0.9.
        assert!((bgsm.Smoothness - 0.9).abs() < 1e-5, "{}", bgsm.Smoothness);
    }

    #[test]
    fn emissive_layer_enables_emittance() {
        let mut l = layer(0.5, 0.0);
        l.material.emissive_multiplier = 2.0;
        l.material.emissive_color = [1.0, 0.5, 0.0];
        let mat = material(vec![l]);
        let bgsm = build_bgsm(&mat, "materials\\lamp.mat").expect("builds");
        assert!(bgsm.EmitEnabled);
        assert_eq!(bgsm.EmittanceColor, Some([1.0, 0.5, 0.0]));
        assert_eq!(bgsm.EmittanceMult, 2.0);
    }

    #[test]
    fn canonical_material_path_supplies_starfield_material_root() {
        assert_eq!(
            canonical_material_path(r"TERRAIN\MossClumpy01_Yellow.mat"),
            r"Materials\TERRAIN\MossClumpy01_Yellow.mat"
        );
        assert_eq!(
            canonical_material_path(r"materials/terrain/Default001Solid.mat"),
            r"materials\terrain\Default001Solid.mat"
        );
        assert_eq!(
            canonical_material_path(r"DATA\MATERIALS\Terrain\DirtCracked01.mat"),
            r"MATERIALS\Terrain\DirtCracked01.mat"
        );
    }

    #[test]
    fn matches_python_cdb_to_bgsm_output() {
        let cdb_path = starfield_materials_cdb();
        if !cdb_path.exists() {
            eprintln!("skip: starfield extracted data not present");
            return;
        }
        let cases: [(&str, &[u8]); 3] = [
            (
                r"materials\architecture\catwalks\barescuffedmetal01_base01.mat",
                include_bytes!("../tests/fixtures/barescuffedmetal01_base01.bgsm").as_slice(),
            ),
            (
                r"materials\terrain\default001solid.mat",
                include_bytes!("../tests/fixtures/default001solid.bgsm").as_slice(),
            ),
            (
                r"materials\items\animalgenericingredients\animalgenericingredients_bone.mat",
                include_bytes!("../tests/fixtures/animalgenericingredients_bone.bgsm").as_slice(),
            ),
        ];
        for (mat_path, fixture) in cases {
            let got =
                cdb_to_bgsm(&cdb_path, mat_path).unwrap_or_else(|e| panic!("{mat_path}: {e}"));
            assert_eq!(got, fixture, "{mat_path}");
        }
    }

    #[test]
    fn resolves_starfield_ltex_material_without_materials_prefix() {
        let cdb_path = starfield_materials_cdb();
        if !cdb_path.exists() {
            eprintln!("skip: starfield extracted data not present");
            return;
        }

        let got = cdb_to_bgsm(&cdb_path, r"TERRAIN\MossClumpy01_Yellow.mat")
            .expect("LTEX-relative material path resolves through canonical Materials root");
        bgsm::parse(&got).expect("resolved material converts to parseable BGSM");
    }
}
