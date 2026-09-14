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
const NI_ALPHA_BLEND: u64 = 1 << 0;
const NI_ALPHA_TEST: u64 = 1 << 9;
const DEFAULT_NI_ALPHA_FLAGS: u64 = 4844;
const DEFAULT_ALPHA_THRESHOLD: u8 = 128;
const VF_VERTEX: i64 = 0x0001;
const VF_SKINNED: i64 = 0x0040;
const VF_FULL_PRECISION: i64 = 0x0400;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AlphaSettings {
    blend: bool,
    source_blend_mode: u32,
    destination_blend_mode: u32,
    test: bool,
    threshold: u8,
}

pub(crate) fn validate_unskinned_geometry(nif: &NifFile) -> Result<(), String> {
    for block in &nif.blocks {
        if matches!(
            block.type_name.as_str(),
            "BSDynamicTriShape" | "NiSkinInstance" | "BSDismemberSkinInstance"
        ) {
            return Err(format!(
                "Skyrim NIF conversion excludes dynamic/skinned block {} ({})",
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

pub(crate) fn validate_supported_geometry(nif: &NifFile) -> Result<(), String> {
    for block in &nif.blocks {
        if block.type_name == "BSDynamicTriShape" && !shape_is_skinned(block) {
            return Err(format!(
                "Skyrim NIF conversion excludes unskinned dynamic block {} ({})",
                block.block_id, block.type_name
            ));
        }
    }
    Ok(())
}

pub(crate) fn contains_skinned_geometry(nif: &NifFile) -> bool {
    nif.blocks.iter().any(|block| {
        matches!(
            block.type_name.as_str(),
            "NiSkinInstance" | "BSDismemberSkinInstance"
        ) || (is_geometry(block) && shape_is_skinned(block))
    })
}

pub(crate) fn load_static_tree_fallback(
    source_path: &Path,
    nif: &NifFile,
) -> Result<Option<(NifFile, PathBuf)>, crate::io::ReadError> {
    if !nif
        .blocks
        .iter()
        .any(|block| block.type_name == "BSTreeNode")
    {
        return Ok(None);
    }
    let Some(parent) = source_path.parent() else {
        return Ok(None);
    };
    let Some(stem) = source_path.file_stem().and_then(|stem| stem.to_str()) else {
        return Ok(None);
    };
    let fallback_path = parent
        .join("switchnodechildren")
        .join(format!("{stem}_1.nif"));
    if !fallback_path.is_file() {
        return Ok(None);
    }
    let fallback = NifFile::load(fallback_path.clone())?;
    if validate_unskinned_geometry(&fallback).is_err()
        || !fallback.blocks.iter().any(|block| is_geometry(block))
    {
        return Ok(None);
    }
    Ok(Some((fallback, fallback_path)))
}

pub(crate) fn normalize_static_geometry(nif: &mut NifFile) -> usize {
    let mut normalized = 0;
    for block in &mut nif.blocks {
        if !is_geometry(block) || shape_is_skinned(block) {
            continue;
        }
        let triangle_count = array_len(block.get_field("Triangles"));
        let vertex_count = array_len(block.get_field("Vertex Data"));
        block.set_field("Num Triangles", NifValue::UInt(triangle_count as u64));
        block.set_field("Num Vertices", NifValue::UInt(vertex_count as u64));
        retarget_static_vertex_desc(block);
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

fn retarget_static_vertex_desc(block: &mut NifBlock) {
    let Some(source_desc) = block.get_field("Vertex Desc").map(NifValue::as_i64) else {
        return;
    };
    let attributes = (source_desc >> 44) & 0xFFF;
    if attributes & VF_VERTEX == 0 || attributes & VF_FULL_PRECISION != 0 {
        return;
    }

    let mut target_desc = source_desc as u64;
    let source_stride = target_desc & 0xF;
    if source_stride < 2 {
        return;
    }
    target_desc = (target_desc & !0xF) | (source_stride - 2);
    for shift in (8..=36).step_by(4) {
        let source_offset = (target_desc >> shift) & 0xF;
        if source_offset >= 2 {
            target_desc = (target_desc & !(0xF << shift)) | ((source_offset - 2) << shift);
        }
    }
    block.set_field("Vertex Desc", NifValue::UInt(target_desc));
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

        let alpha = linked_alpha_settings(nif, shader_id);
        let bytes = if extension == "bgem" {
            bgem::write(&effect_material(&shader, alpha))
        } else {
            let textures = shader_texture_paths(nif, &shader);
            if textures.is_empty() {
                result.warnings.push(format!(
                    "Skyrim shader block {shader_id} has no readable BSShaderTextureSet; emitted a default BGSM"
                ));
            }
            bgsm::write(&lighting_material(&shader, &textures, alpha))
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

fn linked_alpha_settings(nif: &NifFile, shader_id: usize) -> Option<AlphaSettings> {
    nif.blocks
        .iter()
        .filter(|block| is_geometry(block))
        .filter(|shape| {
            linked_property_id(
                nif,
                shape,
                "Shader Property",
                &["BSLightingShaderProperty", "BSEffectShaderProperty"],
            ) == Some(shader_id)
        })
        .find_map(|shape| {
            let alpha_id = linked_property_id(nif, shape, "Alpha Property", &["NiAlphaProperty"])?;
            let alpha = nif.get_block(alpha_id)?;
            let flags = numeric(alpha.get_field("Flags")).unwrap_or(DEFAULT_NI_ALPHA_FLAGS);
            Some(AlphaSettings {
                blend: flags & NI_ALPHA_BLEND != 0,
                source_blend_mode: ((flags >> 1) & 0xF) as u32,
                destination_blend_mode: ((flags >> 5) & 0xF) as u32,
                test: flags & NI_ALPHA_TEST != 0,
                threshold: numeric(alpha.get_field("Threshold"))
                    .and_then(|value| u8::try_from(value).ok())
                    .unwrap_or(DEFAULT_ALPHA_THRESHOLD),
            })
        })
}

fn linked_property_id(
    nif: &NifFile,
    shape: &NifBlock,
    direct_field: &str,
    property_types: &[&str],
) -> Option<usize> {
    let has_type = |id: usize| {
        nif.get_block(id)
            .is_some_and(|block| property_types.contains(&block.type_name.as_str()))
    };
    if let Some(id) = block_ref(shape.get_field(direct_field)).filter(|id| has_type(*id)) {
        return Some(id);
    }
    let Some(NifValue::Array(properties)) = shape.get_field("Properties") else {
        return None;
    };
    properties
        .iter()
        .filter_map(|property| block_ref(Some(property)))
        .find(|id| has_type(*id))
}

fn block_ref(value: Option<&NifValue>) -> Option<usize> {
    numeric(value).and_then(|value| usize::try_from(value).ok())
}

fn lighting_material(
    shader: &NifBlock,
    textures: &[String],
    alpha: Option<AlphaSettings>,
) -> bgsm::BgsmData {
    let flags_1 = numeric(shader.get_field("Shader Flags 1")).unwrap_or(0);
    let flags_2 = numeric(shader.get_field("Shader Flags 2")).unwrap_or(0);
    let mut header = fo4_material_header(flags_1, flags_2, shader, alpha);
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
        CastShadows: true,
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
    material.SmoothSpecTexture = smooth_spec_path(&material.NormalTexture);
    let detail = texture(textures, 2);
    if material.Glowmap || material.EmitEnabled {
        material.GlowTexture = nonempty(detail);
    } else if flags_1 & (SLSF1_GREYSCALE_COLOR | SLSF1_GREYSCALE_ALPHA) != 0 {
        material.GreyscaleTexture = detail;
    }
    if flags_1 & SLSF1_ENVIRONMENT_MAPPING != 0 {
        material.header.env_mapping = Some(true);
        material.EnvmapTexture = nonempty(texture(textures, 4));
    }
    material
}

/// Derive the FO4 `_s` path from the normal's path.
///
/// The texture conversion engine folds Skyrim's normal-alpha gloss and its
/// `_em` environment mask into `<normal base>_s.dds`, keyed off the `_n` in the
/// normal's stem. A normal that does not carry `_n` produces no `_s`, so the
/// slot stays empty rather than pointing at a file nothing writes.
fn smooth_spec_path(normal: &str) -> String {
    let path = Path::new(normal);
    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return String::new();
    };
    let Some(index) = stem.to_ascii_lowercase().rfind("_n") else {
        return String::new();
    };
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    let parent = normal[..normal.len() - stem.len() - extension.len()].to_string();
    format!("{parent}{}_s{extension}", &stem[..index])
}

fn effect_material(shader: &NifBlock, alpha: Option<AlphaSettings>) -> bgem::BgemData {
    let flags_1 = numeric(shader.get_field("Shader Flags 1")).unwrap_or(0);
    let flags_2 = numeric(shader.get_field("Shader Flags 2")).unwrap_or(0);
    let mut header = fo4_material_header(flags_1, flags_2, shader, alpha);
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

fn fo4_material_header(
    flags_1: u64,
    flags_2: u64,
    shader: &NifBlock,
    alpha: Option<AlphaSettings>,
) -> BaseHeader {
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
        alpha_blend_mode0: alpha.is_some_and(|settings| settings.blend) as u8,
        alpha_blend_mode1: alpha.map_or(6, |settings| settings.source_blend_mode),
        alpha_blend_mode2: alpha.map_or(7, |settings| settings.destination_blend_mode),
        alpha_test_ref: alpha.map_or(DEFAULT_ALPHA_THRESHOLD, |settings| settings.threshold),
        alpha_test: alpha.is_some_and(|settings| settings.test),
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
        assert!(validate_unskinned_geometry(&nif).is_err());
        assert!(validate_supported_geometry(&nif).is_ok());
        assert!(contains_skinned_geometry(&nif));
    }

    #[test]
    fn dynamic_geometry_remains_unsupported() {
        let mut nif = NifFile::new("skyrimse");
        nif.add_block("BSDynamicTriShape", None);
        assert!(validate_supported_geometry(&nif).is_err());
    }

    #[test]
    fn rigid_transform_animation_is_supported() {
        let mut nif = NifFile::new("skyrimse");
        nif.add_block("NiTransformController", None);
        nif.add_block("NiTransformInterpolator", None);
        nif.add_block("NiTransformData", None);

        assert!(validate_unskinned_geometry(&nif).is_ok());
    }

    #[test]
    fn skinned_tree_loads_static_switch_child() {
        let temp = tempfile::tempdir().unwrap();
        let tree_dir = temp.path().join("Landscape").join("Trees");
        std::fs::create_dir_all(&tree_dir).unwrap();
        let source_path = tree_dir.join("TreePineForest01.nif");

        let mut source = NifFile::new("skyrimse");
        source.blocks[0].type_name = "BSTreeNode".to_string();
        let mut skinned_fields = IndexMap::new();
        skinned_fields.insert("Skin".to_string(), NifValue::Ref(0));
        source.add_block("BSTriShape", Some(skinned_fields));
        source.rebuild_header();
        source.save(Some(source_path.clone())).unwrap();

        let fallback_dir = tree_dir.join("switchnodechildren");
        std::fs::create_dir_all(&fallback_dir).unwrap();
        let fallback_path = fallback_dir.join("TreePineForest01_1.nif");
        let mut fallback = NifFile::new("skyrimse");
        fallback.blocks[0].type_name = "BSLeafAnimNode".to_string();
        let mut static_fields = IndexMap::new();
        static_fields.insert("Skin".to_string(), NifValue::Ref(-1));
        fallback.add_block("BSTriShape", Some(static_fields));
        fallback.rebuild_header();
        fallback.save(Some(fallback_path.clone())).unwrap();

        let source = NifFile::load(source_path.clone()).unwrap();
        let (loaded, loaded_path) = load_static_tree_fallback(&source_path, &source)
            .unwrap()
            .expect("static switch child");

        assert_eq!(loaded_path, fallback_path);
        assert_eq!(loaded.blocks[0].type_name, "BSLeafAnimNode");
        assert!(validate_unskinned_geometry(&loaded).is_ok());
    }

    #[test]
    fn static_geometry_retargets_rock_grass_vertex_stream() {
        let mut nif = NifFile::new("skyrimse");
        let mut fields = IndexMap::new();
        fields.insert(
            "Vertex Desc".to_string(),
            NifValue::UInt(0x0003_B000_0765_0408),
        );
        nif.add_block("BSTriShape", Some(fields));

        normalize_static_geometry(&mut nif);

        assert_eq!(
            nif.blocks[1].get_field("Vertex Desc").map(NifValue::as_i64),
            Some(0x0003_B000_0543_0206)
        );
    }

    #[test]
    fn static_geometry_normalization_skips_skinned_shapes() {
        let mut nif = NifFile::new("skyrimse");
        let mut fields = IndexMap::new();
        fields.insert(
            "Vertex Desc".to_string(),
            NifValue::UInt(0x0003_B000_0765_0408),
        );
        fields.insert("Skin".to_string(), NifValue::Ref(2));
        nif.add_block("BSTriShape", Some(fields));

        assert_eq!(normalize_static_geometry(&mut nif), 0);
        assert_eq!(
            nif.blocks[1].get_field("Vertex Desc").map(NifValue::as_i64),
            Some(0x0003_B000_0765_0408)
        );
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
            None,
        );
        assert_eq!(material.DiffuseTexture, "a_d.dds");
        assert_eq!(material.NormalTexture, "a_n.dds");
        assert_eq!(material.GlowTexture.as_deref(), Some("a_g.dds"));
        assert_eq!(material.SmoothSpecTexture, "a_s.dds");
        assert_eq!(material.BackLighting, Some(true));
        assert_eq!(material.BackLightPower, Some(3.5));
        assert!(material.DisplacementTexture.is_none());
        assert!(material.Glowmap);
        assert!(material.EmitEnabled);
        assert!((material.Smoothness - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn environment_material_folds_the_mask_into_the_spec_map() {
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
            None,
        );
        assert_eq!(material.EnvmapTexture.as_deref(), Some("a_cube.dds"));
        // Slot 5 is the environment mask; the texture engine bakes it into the
        // FO4 `_s` red channel, so it must not be misrouted to the glow slot.
        assert!(material.GlowTexture.is_none());
        assert_eq!(material.SmoothSpecTexture, "a_s.dds");
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
                .map(|value| value.trim_end_matches('\0'))
                .unwrap_or(""),
            ""
        );
    }

    #[test]
    fn smooth_spec_path_mirrors_what_the_texture_engine_writes() {
        assert_eq!(
            smooth_spec_path("clutter/common/crate01_n.dds"),
            "clutter/common/crate01_s.dds"
        );
        // No `_n` in the stem means no `_s` output, so the slot stays empty.
        assert_eq!(smooth_spec_path("clutter/common/crate01.dds"), "");
        assert_eq!(smooth_spec_path(""), "");
    }

    #[test]
    fn tree_tundra_shrub_08_alpha_property_is_propagated_to_bgsm() {
        let mut nif = NifFile::new("skyrimse");
        let shader_id = nif.add_block("BSLightingShaderProperty", None);
        let alpha_id = nif.add_block(
            "NiAlphaProperty",
            Some(IndexMap::from([
                ("Flags".to_string(), NifValue::UInt(4844)),
                ("Threshold".to_string(), NifValue::UInt(180)),
            ])),
        );
        nif.add_block(
            "BSTriShape",
            Some(IndexMap::from([
                (
                    "Shader Property".to_string(),
                    NifValue::Ref(shader_id as i32),
                ),
                ("Alpha Property".to_string(), NifValue::Ref(alpha_id as i32)),
            ])),
        );

        let output = tempfile::tempdir().expect("grass material output");
        let report = synthesize_fo4_materials(
            &mut nif,
            Path::new(r"Meshes\Landscape\Plants\TreeTundraShrub08.nif"),
            output.path(),
        )
        .expect("synthesize grass BGSM");
        assert_eq!(report.emitted.len(), 1);
        let parsed =
            bgsm::parse(&std::fs::read(&report.emitted[0]).expect("read synthesized grass BGSM"))
                .expect("parse synthesized grass BGSM");

        assert!(parsed.header.alpha_test);
        assert_eq!(parsed.header.alpha_test_ref, 180);
        assert_eq!(parsed.header.alpha_blend_mode0, 0);
        assert_eq!(parsed.header.alpha_blend_mode1, 6);
        assert_eq!(parsed.header.alpha_blend_mode2, 7);
    }

    #[test]
    fn opaque_material_without_alpha_property_keeps_alpha_disabled() {
        let shader = NifBlock::new(0, "BSLightingShaderProperty");
        let material = lighting_material(&shader, &[], None);

        assert!(material.CastShadows);
        assert!(!material.header.alpha_test);
        assert_eq!(material.header.alpha_test_ref, 128);
        assert_eq!(material.header.alpha_blend_mode0, 0);
    }

    #[test]
    fn alpha_blend_is_enabled_only_when_the_property_requests_it() {
        let settings = AlphaSettings {
            blend: true,
            source_blend_mode: 6,
            destination_blend_mode: 7,
            test: true,
            threshold: 90,
        };
        let shader = NifBlock::new(0, "BSLightingShaderProperty");
        let material = lighting_material(&shader, &[], Some(settings));

        assert_eq!(material.header.alpha_blend_mode0, 1);
        assert_eq!(material.header.alpha_blend_mode1, 6);
        assert_eq!(material.header.alpha_blend_mode2, 7);
    }
}
