use std::path::{Component, Path, PathBuf};

use materials_native::base::BaseHeader;
use materials_native::{bgem, bgsm};

use crate::model::{NifBlock, NifFile, NifValue};

const SLSF1_SPECULAR: u64 = 1 << 0;
const SLSF1_GREYSCALE_COLOR: u64 = 1 << 4;
const SLSF1_GREYSCALE_ALPHA: u64 = 1 << 5;
const SLSF1_ENVIRONMENT_MAPPING: u64 = 1 << 7;
const SLSF1_RECEIVE_SHADOWS: u64 = 1 << 8;
const SLSF1_CAST_SHADOWS: u64 = 1 << 9;
const SLSF1_MODEL_SPACE_NORMALS: u64 = 1 << 12;
const SLSF1_OWN_EMIT: u64 = 1 << 22;
const SLSF1_EXTERNAL_EMIT: u64 = 1 << 29;
const SLSF1_ZBUFFER_TEST: u64 = 1 << 31;
const SLSF2_ZBUFFER_WRITE: u64 = 1 << 0;
const SLSF2_DOUBLE_SIDED: u64 = 1 << 4;
const SLSF2_GLOW_MAP: u64 = 1 << 6;
const SLSF2_ASSUME_SHADOWMASK: u64 = 1 << 7;
const SLSF2_PREMULT_ALPHA: u64 = 1 << 19;
const SLSF2_ANISOTROPIC_LIGHTING: u64 = 1 << 21;
const SLSF2_BACK_LIGHTING: u64 = 1 << 27;
const SLSF2_TREE_ANIM: u64 = 1 << 29;
const VF_SKINNED: i64 = 0x0040;

const FO4_FLAGS_1_STATIC_MASK: u64 = (1 << 0)
    | (1 << 3)
    | (1 << 4)
    | (1 << 5)
    | (1 << 6)
    | (1 << 7)
    | (1 << 9)
    | (1 << 12)
    | (1 << 13)
    | (1 << 14)
    | (1 << 15)
    | (1 << 16)
    | (1 << 20)
    | (1 << 22)
    | (1 << 23)
    | (1 << 24)
    | (1 << 25)
    | (1 << 26)
    | (1 << 27)
    | (1 << 29)
    | (1 << 30)
    | (1 << 31);
const FO4_FLAGS_2_STATIC_MASK: u64 = (1 << 0)
    | (1 << 3)
    | (1 << 4)
    | (1 << 5)
    | (1 << 6)
    | (1 << 7)
    | (1 << 10)
    | (1 << 11)
    | (1 << 12)
    | (1 << 13)
    | (1 << 14)
    | (1 << 15)
    | (1 << 16)
    | (1 << 18)
    | (1 << 19)
    | (1 << 20)
    | (1 << 22)
    | (1 << 24)
    | (1 << 25)
    | (1 << 26)
    | (1 << 27)
    | (1 << 29)
    | (1 << 30)
    | (1 << 31);

#[derive(Debug, Default)]
pub(crate) struct MaterialSynthesisReport {
    pub emitted: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

pub(crate) fn validate_static_only(nif: &NifFile) -> Result<(), String> {
    for block in &nif.blocks {
        if matches!(
            block.type_name.as_str(),
            "BSDynamicTriShape"
                | "NiSkinInstance"
                | "BSDismemberSkinInstance"
                | "NiControllerManager"
                | "NiControllerSequence"
        ) || crate::schema::SCHEMA.is_subtype_of(&block.type_name, "NiTimeController")
        {
            return Err(format!(
                "Skyrim static NIF conversion excludes animated/skinned block {} ({})",
                block.block_id, block.type_name
            ));
        }
        if is_geometry(block) && shape_is_skinned(block) {
            return Err(format!(
                "Skyrim static NIF conversion excludes skinned geometry block {} ({})",
                block.block_id, block.type_name
            ));
        }
    }
    Ok(())
}

pub(crate) fn normalize_static_geometry(nif: &mut NifFile) -> usize {
    let mut normalized = 0;
    for block in &mut nif.blocks {
        if !is_geometry(block) {
            continue;
        }
        let triangle_count = array_len(block.get_field("Triangles"));
        let vertex_count = array_len(block.get_field("Vertex Data"));
        block.set_field("Num Triangles", NifValue::UInt(triangle_count as u64));
        block.set_field("Num Vertices", NifValue::UInt(vertex_count as u64));
        for key in [
            "Particle Data Size",
            "Particle Vertices",
            "Particle Normals",
            "Particle Triangles",
        ] {
            remove_bare_field(block, key);
        }
        if block.type_name == "BSSubIndexTriShape" {
            let segments = array_len(block.get_field("Segment"));
            block.set_field("Num Segments", NifValue::UInt(segments as u64));
            block.set_field("Total Segments", NifValue::UInt(segments as u64));
        }
        normalized += 1;
    }
    normalized
}

pub(crate) fn synthesize_fo4_materials(
    nif: &mut NifFile,
    source_path: &Path,
    output_root: &Path,
) -> Result<MaterialSynthesisReport, std::io::Error> {
    let mut result = MaterialSynthesisReport::default();
    let shader_ids = nif
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.type_name.as_str(),
                "BSLightingShaderProperty" | "BSEffectShaderProperty"
            )
        })
        .map(|block| block.block_id)
        .collect::<Vec<_>>();

    for shader_id in shader_ids {
        let Some(shader) = nif.get_block(shader_id).cloned() else {
            continue;
        };
        let extension = if shader.type_name == "BSEffectShaderProperty" {
            "bgem"
        } else {
            "bgsm"
        };
        let output_path = material_output_path(output_root, source_path, shader_id, extension);
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let bytes = if extension == "bgem" {
            bgem::write(&effect_material(&shader))
        } else {
            let textures = shader_texture_paths(nif, &shader);
            if textures.is_empty() {
                result.warnings.push(format!(
                    "Skyrim shader block {shader_id} has no readable BSShaderTextureSet; emitted a default BGSM"
                ));
            }
            bgsm::write(&lighting_material(&shader, &textures))
        };
        std::fs::write(&output_path, bytes)?;

        let material_name = material_game_path(&output_path);
        if let Some(target) = nif.blocks.get_mut(shader_id) {
            let skyrim_flags_1 = numeric(target.get_field("Shader Flags 1")).unwrap_or(0);
            let skyrim_flags_2 = numeric(target.get_field("Shader Flags 2")).unwrap_or(0);
            target.set_field("Name", NifValue::String(material_name));
            target.fields.shift_remove("Shader Flags 1:SK");
            target.fields.shift_remove("Shader Flags 2:SK");
            target.fields.insert(
                "Shader Flags 1:FO4".to_string(),
                NifValue::UInt(skyrim_flags_1 & FO4_FLAGS_1_STATIC_MASK),
            );
            target.fields.insert(
                "Shader Flags 2:FO4".to_string(),
                NifValue::UInt(skyrim_flags_2 & FO4_FLAGS_2_STATIC_MASK),
            );
        }
        result.emitted.push(output_path);
    }
    Ok(result)
}

fn is_geometry(block: &NifBlock) -> bool {
    matches!(
        block.type_name.as_str(),
        "BSTriShape" | "BSSubIndexTriShape"
    )
}

fn shape_is_skinned(block: &NifBlock) -> bool {
    for field in ["Skin", "Skin Instance"] {
        if block
            .get_field(field)
            .is_some_and(|value| value.as_i64() >= 0)
        {
            return true;
        }
    }
    block
        .get_field("Vertex Desc")
        .is_some_and(|value| ((value.as_i64() >> 44) & VF_SKINNED) != 0)
}

fn array_len(value: Option<&NifValue>) -> usize {
    match value {
        Some(NifValue::Array(values)) => values.len(),
        _ => 0,
    }
}

fn remove_bare_field(block: &mut NifBlock, name: &str) {
    let keys = block
        .fields
        .keys()
        .filter(|key| key.split(':').next() == Some(name))
        .cloned()
        .collect::<Vec<_>>();
    for key in keys {
        block.fields.shift_remove(&key);
    }
}

fn shader_texture_paths(nif: &NifFile, shader: &NifBlock) -> Vec<String> {
    let Some(texture_set_id) = shader
        .get_field("Texture Set")
        .map(NifValue::as_i64)
        .filter(|id| *id >= 0)
        .map(|id| id as usize)
    else {
        return Vec::new();
    };
    let Some(NifValue::Array(values)) = nif
        .get_block(texture_set_id)
        .and_then(|block| block.get_field("Textures"))
    else {
        return Vec::new();
    };
    values
        .iter()
        .map(|value| match value {
            NifValue::String(path) => normalize_texture(path),
            _ => String::new(),
        })
        .collect()
}

fn lighting_material(shader: &NifBlock, textures: &[String]) -> bgsm::BgsmData {
    let flags_1 = numeric(shader.get_field("Shader Flags 1")).unwrap_or(0);
    let flags_2 = numeric(shader.get_field("Shader Flags 2")).unwrap_or(0);
    let mut header = fo4_material_header(flags_1, flags_2, shader);
    header.signature = bgsm::BGSM_SIGNATURE;
    let mut material = bgsm::BgsmData {
        header,
        DiffuseTexture: texture(textures, 0),
        NormalTexture: texture(textures, 1),
        SpecularEnabled: flags_1 & SLSF1_SPECULAR != 0,
        SpecularColor: color3(shader.get_field("Specular Color")).unwrap_or([1.0; 3]),
        SpecularMult: float(shader.get_field("Specular Strength")).unwrap_or(1.0),
        Smoothness: (float(shader.get_field("Glossiness")).unwrap_or(50.0) / 100.0).clamp(0.0, 1.0),
        FresnelPower: 5.0,
        WetnessControlSpecScale: -1.0,
        WetnessControlSpecPowerScale: -1.0,
        WetnessControlSpecMinvar: -1.0,
        WetnessControlEnvMapScale: Some(-1.0),
        WetnessControlFresnelPower: -1.0,
        WetnessControlMetalness: -1.0,
        EmitEnabled: flags_1 & SLSF1_OWN_EMIT != 0 || flags_2 & SLSF2_GLOW_MAP != 0,
        EmittanceColor: color3(shader.get_field("Emissive Color")),
        EmittanceMult: float(shader.get_field("Emissive Multiple")).unwrap_or(1.0),
        ModelSpaceNormals: flags_1 & SLSF1_MODEL_SPACE_NORMALS != 0,
        ExternalEmittance: flags_1 & SLSF1_EXTERNAL_EMIT != 0,
        ReceiveShadows: flags_1 & SLSF1_RECEIVE_SHADOWS != 0,
        CastShadows: flags_1 & SLSF1_CAST_SHADOWS != 0,
        AssumeShadowmask: flags_2 & SLSF2_ASSUME_SHADOWMASK != 0,
        Glowmap: flags_2 & SLSF2_GLOW_MAP != 0,
        AnisoLighting: flags_2 & SLSF2_ANISOTROPIC_LIGHTING != 0,
        Tree: flags_2 & SLSF2_TREE_ANIM != 0,
        GrayscaleToPaletteScale: 1.0,
        BackLighting: Some(flags_2 & SLSF2_BACK_LIGHTING != 0),
        BackLightPower: Some(if flags_2 & SLSF2_BACK_LIGHTING != 0 {
            float(shader.get_field("Lighting Effect 2")).unwrap_or(2.0)
        } else {
            0.0
        }),
        ..bgsm::BgsmData::default()
    };
    let detail = texture(textures, 2);
    if material.Glowmap || material.EmitEnabled {
        material.GlowTexture = nonempty(detail);
    } else if flags_1 & (SLSF1_GREYSCALE_COLOR | SLSF1_GREYSCALE_ALPHA) != 0 {
        material.GreyscaleTexture = detail;
    }
    if flags_1 & SLSF1_ENVIRONMENT_MAPPING != 0 {
        material.header.env_mapping = Some(true);
        material.EnvmapTexture = nonempty(texture(textures, 4));
        if material.GlowTexture.is_none() {
            material.GlowTexture = nonempty(texture(textures, 5));
        }
    }
    material
}

fn effect_material(shader: &NifBlock) -> bgem::BgemData {
    let flags_1 = numeric(shader.get_field("Shader Flags 1")).unwrap_or(0);
    let flags_2 = numeric(shader.get_field("Shader Flags 2")).unwrap_or(0);
    let mut header = fo4_material_header(flags_1, flags_2, shader);
    header.signature = bgem::BGEM_SIGNATURE;
    bgem::BgemData {
        header,
        BaseTexture: string(shader.get_field("Source Texture"))
            .map(normalize_texture)
            .unwrap_or_default(),
        GrayscaleTexture: string(shader.get_field("Greyscale Texture"))
            .map(normalize_texture)
            .unwrap_or_default(),
        BloodEnabled: false,
        EffectLightingEnabled: flags_2 & (1 << 30) != 0,
        FalloffEnabled: flags_1 & (1 << 6) != 0,
        FalloffColorEnabled: false,
        GrayscaleToPaletteAlpha: flags_1 & SLSF1_GREYSCALE_ALPHA != 0,
        SoftEnabled: flags_1 & (1 << 30) != 0,
        BaseColor: color4(shader.get_field("Base Color"))
            .map(|color| [color[0], color[1], color[2]])
            .unwrap_or([1.0; 3]),
        BaseColorScale: float(shader.get_field("Base Color Scale")).unwrap_or(1.0),
        FalloffStartAngle: float(shader.get_field("Falloff Start Angle")).unwrap_or(1.0),
        FalloffStopAngle: float(shader.get_field("Falloff Stop Angle")).unwrap_or(1.0),
        FalloffStartOpacity: float(shader.get_field("Falloff Start Opacity")).unwrap_or(1.0),
        FalloffStopOpacity: float(shader.get_field("Falloff Stop Opacity")).unwrap_or(1.0),
        LightingInfluence: numeric(shader.get_field("Lighting Influence")).unwrap_or(255) as f32
            / 255.0,
        EnvmapMinLOD: numeric(shader.get_field("Env Map Min LOD")).unwrap_or(0) as u8,
        SoftDepth: float(shader.get_field("Soft Falloff Depth")).unwrap_or(100.0),
        ..bgem::BgemData::default()
    }
}

fn fo4_material_header(flags_1: u64, flags_2: u64, shader: &NifBlock) -> BaseHeader {
    let uv_offset = tex_coord(shader.get_field("UV Offset")).unwrap_or([0.0, 0.0]);
    let uv_scale = tex_coord(shader.get_field("UV Scale")).unwrap_or([1.0, 1.0]);
    BaseHeader {
        signature: 0,
        version: 2,
        tile_u: true,
        tile_v: true,
        u_offset: uv_offset[0],
        v_offset: uv_offset[1],
        u_scale: uv_scale[0],
        v_scale: uv_scale[1],
        alpha: float(shader.get_field("Alpha")).unwrap_or(1.0),
        alpha_blend_mode0: 0,
        alpha_blend_mode1: 6,
        alpha_blend_mode2: 7,
        alpha_test_ref: 128,
        alpha_test: false,
        zbuffer_write: flags_2 & SLSF2_ZBUFFER_WRITE != 0,
        zbuffer_test: flags_1 & SLSF1_ZBUFFER_TEST != 0,
        ssr: false,
        wet_ssr: false,
        decal: flags_1 & (1 << 26) != 0,
        two_sided: flags_2 & SLSF2_DOUBLE_SIDED != 0,
        decal_nofade: false,
        non_occluder: false,
        refraction: flags_1 & (1 << 15) != 0,
        refraction_falloff: false,
        refraction_power: float(shader.get_field("Refraction Strength")).unwrap_or(0.0),
        env_mapping: Some(flags_1 & SLSF1_ENVIRONMENT_MAPPING != 0),
        env_mapping_mask_scale: Some(
            float(shader.get_field("Environment Map Scale")).unwrap_or(1.0),
        ),
        depth_bias: Some(false),
        grayscale_to_palette_color: flags_1 & SLSF1_GREYSCALE_COLOR != 0,
        mask_writes: Some(if flags_2 & SLSF2_PREMULT_ALPHA != 0 {
            1
        } else {
            0
        }),
    }
}

fn material_output_path(
    output_root: &Path,
    source_path: &Path,
    shader_id: usize,
    extension: &str,
) -> PathBuf {
    let components = source_path.components().collect::<Vec<_>>();
    let mesh_index = components.iter().rposition(|component| match component {
        Component::Normal(value) => value.to_string_lossy().eq_ignore_ascii_case("meshes"),
        _ => false,
    });
    let relative = mesh_index
        .map(|index| components[index + 1..].iter().collect::<PathBuf>())
        .unwrap_or_else(|| {
            source_path
                .file_name()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("skyrim_static.nif"))
        });
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let stem = relative
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("skyrim_static");
    output_root
        .join(parent)
        .join(format!("{stem}_{shader_id}.{extension}"))
}

fn material_game_path(path: &Path) -> String {
    let components = path.components().collect::<Vec<_>>();
    let material_index = components.iter().rposition(|component| match component {
        Component::Normal(value) => value.to_string_lossy().eq_ignore_ascii_case("materials"),
        _ => false,
    });
    let relative = material_index
        .map(|index| components[index..].iter().collect::<PathBuf>())
        .unwrap_or_else(|| {
            PathBuf::from("Materials").join(
                path.file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("skyrim_static.bgsm")),
            )
        });
    relative.to_string_lossy().replace('/', "\\")
}

fn normalize_texture(path: &str) -> String {
    let normalized = path
        .trim_end_matches('\0')
        .trim()
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string();
    if normalized
        .get(..9)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("textures/"))
    {
        normalized[9..].to_string()
    } else {
        normalized
    }
}

fn texture(textures: &[String], index: usize) -> String {
    textures.get(index).cloned().unwrap_or_default()
}

fn nonempty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn numeric(value: Option<&NifValue>) -> Option<u64> {
    match value {
        Some(NifValue::UInt(value)) => Some(*value),
        Some(NifValue::Int(value)) if *value >= 0 => Some(*value as u64),
        Some(NifValue::Ref(value)) if *value >= 0 => Some(*value as u64),
        _ => None,
    }
}

fn float(value: Option<&NifValue>) -> Option<f32> {
    match value {
        Some(NifValue::Float(value)) => Some(*value as f32),
        Some(NifValue::Int(value)) => Some(*value as f32),
        Some(NifValue::UInt(value)) => Some(*value as f32),
        _ => None,
    }
}

fn string(value: Option<&NifValue>) -> Option<&str> {
    match value {
        Some(NifValue::String(value)) => Some(value),
        _ => None,
    }
}

fn color3(value: Option<&NifValue>) -> Option<[f32; 3]> {
    match value {
        Some(NifValue::Color3(value)) => Some(*value),
        Some(NifValue::Struct(fields)) => Some([
            float(fields.get("r"))?,
            float(fields.get("g"))?,
            float(fields.get("b"))?,
        ]),
        _ => None,
    }
}

fn color4(value: Option<&NifValue>) -> Option<[f32; 4]> {
    match value {
        Some(NifValue::Color4(value)) => Some(*value),
        Some(NifValue::Struct(fields)) => Some([
            float(fields.get("r"))?,
            float(fields.get("g"))?,
            float(fields.get("b"))?,
            float(fields.get("a"))?,
        ]),
        _ => None,
    }
}

fn tex_coord(value: Option<&NifValue>) -> Option<[f32; 2]> {
    match value {
        Some(NifValue::Struct(fields)) => Some([float(fields.get("u"))?, float(fields.get("v"))?]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    #[test]
    fn material_path_preserves_mesh_relative_directories() {
        let path = material_output_path(
            Path::new(r"C:\mod\data\Materials\Skyrim"),
            Path::new(r"X:\extracted\skyrimse\Meshes\Architecture\Whiterun\Wall.nif"),
            7,
            "bgsm",
        );
        assert!(path.ends_with(r"Architecture\Whiterun\Wall_7.bgsm"));
        assert_eq!(
            material_game_path(&path),
            r"Materials\Skyrim\Architecture\Whiterun\Wall_7.bgsm"
        );
    }

    #[test]
    fn static_geometry_rejects_skin() {
        let mut nif = NifFile::new("skyrimse");
        let mut fields = IndexMap::new();
        fields.insert("Skin".to_string(), NifValue::Ref(0));
        nif.add_block("BSTriShape", Some(fields));
        assert!(validate_static_only(&nif).is_err());
    }

    #[test]
    fn lighting_material_maps_skyrim_texture_slots_and_flags() {
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field(
            "Shader Flags 1:SK",
            NifValue::UInt(SLSF1_SPECULAR | SLSF1_OWN_EMIT | SLSF1_CAST_SHADOWS),
        );
        shader.set_field("Glossiness", NifValue::Float(75.0));
        shader.set_field("Lighting Effect 2", NifValue::Float(3.5));
        shader.set_field(
            "Shader Flags 2:SK",
            NifValue::UInt(SLSF2_ZBUFFER_WRITE | SLSF2_GLOW_MAP | SLSF2_BACK_LIGHTING),
        );
        let material = lighting_material(
            &shader,
            &[
                "a_d.dds".into(),
                "a_n.dds".into(),
                "a_g.dds".into(),
                "a_h.dds".into(),
                "a_e.dds".into(),
                "a_em.dds".into(),
                String::new(),
                "a_s.dds".into(),
            ],
        );
        assert_eq!(material.DiffuseTexture, "a_d.dds");
        assert_eq!(material.NormalTexture, "a_n.dds");
        assert_eq!(material.GlowTexture.as_deref(), Some("a_g.dds"));
        assert!(material.SmoothSpecTexture.is_empty());
        assert_eq!(material.BackLighting, Some(true));
        assert_eq!(material.BackLightPower, Some(3.5));
        assert!(material.DisplacementTexture.is_none());
        assert!(material.Glowmap);
        assert!(material.EmitEnabled);
        assert!((material.Smoothness - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn environment_material_uses_skyrim_mask_slot() {
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field(
            "Shader Flags 1:SK",
            NifValue::UInt(SLSF1_ENVIRONMENT_MAPPING),
        );
        let material = lighting_material(
            &shader,
            &[
                "a_d.dds".into(),
                "a_n.dds".into(),
                String::new(),
                "a_h.dds".into(),
                "a_cube.dds".into(),
                "a_em.dds".into(),
            ],
        );
        assert_eq!(material.EnvmapTexture.as_deref(), Some("a_cube.dds"));
        assert_eq!(material.GlowTexture.as_deref(), Some("a_em.dds"));
        assert!(material.header.env_mapping.unwrap_or(false));
        assert!(material.DisplacementTexture.is_none());
        let parsed = bgsm::parse(&bgsm::write(&material)).expect("parse serialized BGSM");
        assert_eq!(
            parsed
                .EnvmapTexture
                .as_deref()
                .map(|value| value.trim_end_matches('\0')),
            Some("a_cube.dds")
        );
        assert_eq!(
            parsed
                .GlowTexture
                .as_deref()
                .map(|value| value.trim_end_matches('\0')),
            Some("a_em.dds")
        );
    }
}
