use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use havok_native::collision::extract_preview_meshes_from_blob;
use indexmap::IndexMap;
use nif_core_native::convert_file::{ConvertFileOptions, convert_nif_file};
use nif_core_native::model::{NifFile, NifValue};
use nif_core_native::skin::pack::vertex_desc_skinned;

fn temp_dir(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "nif_core_native_{name}_{}_{}",
        std::process::id(),
        suffix
    ))
}

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(relative)
}

fn fixture_bytes(relative: &str) -> Vec<u8> {
    std::fs::read(repo_path(relative)).unwrap_or_else(|error| {
        panic!("failed to read fixture {relative}: {error}");
    })
}

fn embedded_havok_blob(nif: &NifFile) -> Option<Vec<u8>> {
    for block in &nif.blocks {
        if block.type_name != "bhkPhysicsSystem" && block.type_name != "bhkRagdollSystem" {
            continue;
        }
        let Some(NifValue::Struct(binary_data)) = block.get_field("Binary Data") else {
            continue;
        };
        let Some(data) = binary_data.get("Data") else {
            continue;
        };
        let bytes = match data {
            NifValue::Bytes(bytes) => bytes.clone(),
            NifValue::Array(values) => values.iter().map(|value| value.as_i64() as u8).collect(),
            _ => Vec::new(),
        };
        if !bytes.is_empty() {
            return Some(bytes);
        }
    }
    None
}

#[test]
fn convert_static_skyrim_to_fo4_emits_material_and_rewrites_vertex_stream() {
    let dir = temp_dir("convert_static_skyrim");
    let materials_dir = dir.join("data").join("Materials").join("Skyrim");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("data").join("Meshes").join("converted.nif");

    let mut nif = NifFile::new("skyrimse");
    let texture_set_id = nif.add_block(
        "BSShaderTextureSet",
        Some(fields([
            ("Num Textures", NifValue::UInt(9)),
            (
                "Textures",
                NifValue::Array(vec![
                    NifValue::String(r"textures\architecture\wall_d.dds".to_string()),
                    NifValue::String(r"textures\architecture\wall_n.dds".to_string()),
                    NifValue::String(r"textures\architecture\wall_g.dds".to_string()),
                    NifValue::String(String::new()),
                    NifValue::String(r"textures\architecture\wall_cube.dds".to_string()),
                    NifValue::String(r"textures\architecture\wall_em.dds".to_string()),
                    NifValue::String(String::new()),
                    NifValue::String(r"textures\architecture\wall_bl.dds".to_string()),
                    NifValue::String(String::new()),
                ]),
            ),
        ])),
    );
    let shader_id = nif.add_block(
        "BSLightingShaderProperty",
        Some(fields([
            ("Name", NifValue::String(String::new())),
            (
                "Shader Flags 1:SK",
                NifValue::UInt((1 << 0) | (1 << 7) | (1 << 9) | (1 << 22) | (1 << 31)),
            ),
            ("Shader Flags 2:SK", NifValue::UInt((1 << 0) | (1 << 6))),
            ("Texture Set", NifValue::Ref(texture_set_id as i32)),
            ("Texture Clamp Mode", NifValue::UInt(3)),
            ("Alpha", NifValue::Float(1.0)),
            ("Refraction Strength", NifValue::Float(0.0)),
            ("Glossiness", NifValue::Float(75.0)),
            ("Specular Color", NifValue::Color3([1.0, 1.0, 1.0])),
            ("Specular Strength", NifValue::Float(1.5)),
        ])),
    );
    let shape_id = nif.add_block(
        "BSTriShape",
        Some(fields([
            ("Name", NifValue::String("Wall:0".to_string())),
            ("Skin", NifValue::Ref(-1)),
            ("Shader Property", NifValue::Ref(shader_id as i32)),
            ("Alpha Property", NifValue::Ref(-1)),
            ("Vertex Desc", NifValue::Int(skyrim_vertex_desc(false))),
            ("Num Triangles", NifValue::UInt(1)),
            ("Num Vertices", NifValue::UInt(3)),
            (
                "Vertex Data",
                NifValue::Array(vec![
                    basic_vertex([0.0, 0.0, 0.0], false),
                    basic_vertex([1.0, 0.0, 0.0], false),
                    basic_vertex([0.0, 1.0, 0.0], false),
                ]),
            ),
            ("Triangles", NifValue::Array(vec![triangle(0, 1, 2)])),
        ])),
    );
    nif.blocks[0].set_field("Num Children", NifValue::UInt(1));
    nif.blocks[0].set_field(
        "Children",
        NifValue::Array(vec![NifValue::Ref(shape_id as i32)]),
    );
    nif.save(Some(src.clone())).expect("write Skyrim source");

    let report = convert_nif_file(
        &src,
        &dst,
        "skyrimse",
        "fo4",
        Some(&materials_dir),
        &ConvertFileOptions {
            asset_prefix: Some("Skyrim".to_string()),
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert Skyrim static");
    assert!(report.supported, "{:?}", report.errors);
    assert!(report.errors.is_empty());
    assert_eq!(report.emitted_bgsms.len(), 1);
    let material_path = PathBuf::from(&report.emitted_bgsms[0]);
    assert!(material_path.is_file());
    let material =
        materials_native::bgsm::parse(&std::fs::read(&material_path).expect("read emitted BGSM"))
            .expect("parse emitted BGSM");
    assert_eq!(
        material.DiffuseTexture.trim_end_matches('\0'),
        "Skyrim/architecture/wall_d.dds"
    );
    assert_eq!(
        material.NormalTexture.trim_end_matches('\0'),
        "Skyrim/architecture/wall_n.dds"
    );
    assert_eq!(
        material
            .GlowTexture
            .as_deref()
            .map(|value| value.trim_end_matches('\0')),
        Some("Skyrim/architecture/wall_g.dds")
    );
    // The texture conversion path synthesizes `_s` from the normal's alpha and
    // the `_em` mask, so the material must name it up front.
    assert_eq!(
        material.SmoothSpecTexture.trim_end_matches('\0'),
        "Skyrim/architecture/wall_s.dds"
    );
    assert_eq!(
        material
            .EnvmapTexture
            .as_deref()
            .map(|value| value.trim_end_matches('\0')),
        Some("Skyrim/architecture/wall_cube.dds")
    );
    assert!(material.SpecularEnabled);
    assert!(material.header.env_mapping.unwrap_or(false));
    assert!(
        material
            .DisplacementTexture
            .as_deref()
            .unwrap_or("")
            .trim_end_matches('\0')
            .is_empty()
    );

    let converted = NifFile::load(dst).expect("load converted Skyrim static");
    assert_eq!(converted.header.bs_version, 130);
    let converted_shape = converted
        .blocks
        .iter()
        .find(|block| block.type_name == "BSTriShape")
        .expect("converted shape");
    assert_eq!(
        converted_shape
            .get_field("Vertex Data")
            .and_then(|value| match value {
                NifValue::Array(values) => Some(values.len()),
                _ => None,
            }),
        Some(3)
    );
    assert_eq!(
        converted_shape
            .get_field("Vertex Desc")
            .map(NifValue::as_i64),
        Some(basic_vertex_desc(false))
    );
    assert_eq!(
        converted_shape.get_field("Data Size").map(NifValue::as_i64),
        Some(66)
    );
    let converted_shader = converted
        .blocks
        .iter()
        .find(|block| block.type_name == "BSLightingShaderProperty")
        .expect("converted shader");
    assert!(
        matches!(
            converted_shader.get_field("Name"),
            Some(NifValue::String(name)) if name.starts_with(r"Materials\Skyrim")
        ),
        "shader name: {:?}",
        converted_shader.get_field("Name")
    );
    assert!(converted_shader.fields.contains_key("Shader Flags 1:FO4"));
    assert!(matches!(
        converted_shader.get_field("Root Material"),
        Some(NifValue::String(value)) if value.is_empty()
    ));
    assert!(
        matches!(
            converted_shader.get_field("Texture Clamp Mode"),
            Some(NifValue::UInt(3))
        ),
        "shader fields: {:?}",
        converted_shader.fields
    );
    assert!(matches!(
        converted_shader.get_field("Alpha"),
        Some(NifValue::Float(value)) if (*value - 1.0).abs() < f64::EPSILON
    ));
    assert!(matches!(
        converted_shader.get_field("Refraction Strength"),
        Some(NifValue::Float(value)) if value.abs() < f64::EPSILON
    ));
    assert!(matches!(
        converted_shader.get_field("Smoothness"),
        Some(NifValue::Float(value)) if (*value - 1.0).abs() < f64::EPSILON
    ));
    assert!(converted_shader.get_field("Wetness").is_some());

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_skyrim_rigid_transform_animation_is_preserved() {
    let dir = temp_dir("convert_skyrim_rigid_animation");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("converted.nif");
    let mut nif = NifFile::new("skyrimse");
    let data_id = nif.add_block("NiTransformData", None);
    let interpolator_id = nif.add_block(
        "NiTransformInterpolator",
        Some(fields([("Data", NifValue::Ref(data_id as i32))])),
    );
    let controller_id = nif.add_block(
        "NiTransformController",
        Some(fields([
            ("Flags", NifValue::UInt(72)),
            ("Stop Time", NifValue::Float(8.0)),
            ("Target", NifValue::Ref(0)),
            ("Interpolator", NifValue::Ref(interpolator_id as i32)),
        ])),
    );
    nif.blocks[0].set_field("Controller", NifValue::Ref(controller_id as i32));
    nif.save(Some(src.clone()))
        .expect("write rigid animated source");

    let report = convert_nif_file(
        &src,
        &dst,
        "skyrimse",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert rigid animated Skyrim NIF");

    assert!(report.supported, "errors: {:?}", report.errors);
    let converted = NifFile::load(dst).expect("load converted rigid animation");
    assert_eq!(converted.header.bs_version, 130);
    let controller = converted
        .blocks
        .iter()
        .find(|block| block.type_name == "NiTransformController")
        .expect("preserved transform controller");
    assert_eq!(
        controller.get_field("Interpolator").map(NifValue::as_i64),
        Some(interpolator_id as i64)
    );
    assert!(
        converted
            .blocks
            .iter()
            .any(|block| block.type_name == "NiTransformData")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_skyrim_skinned_nif_is_rejected_without_output() {
    let dir = temp_dir("reject_skyrim_skin");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("converted.nif");
    let mut nif = NifFile::new("skyrimse");
    let skin_id = nif.add_block("NiSkinInstance", None);
    nif.add_block(
        "BSTriShape",
        Some(fields([("Skin", NifValue::Ref(skin_id as i32))])),
    );
    nif.save(Some(src.clone())).expect("write skinned source");

    let report = convert_nif_file(
        &src,
        &dst,
        "skyrimse",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("reject report");
    assert!(!report.supported);
    assert!(
        report.errors[0].contains("requires translation_maps_dir"),
        "{:?}",
        report.errors
    );
    assert!(!dst.exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_minimal_fnv_to_fo4_updates_header_and_writes_output() {
    let dir = temp_dir("convert_file");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = NifFile::new("fnv");
    nif.save(Some(src.clone())).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fnv",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert nif");
    assert!(report.supported);
    assert!(dst.exists());
    assert!(report.timings_ms.iter().any(|(step, _)| step == "load"));
    assert!(report.timings_ms.iter().any(|(step, _)| step == "save"));
    assert!(
        report
            .timings_ms
            .iter()
            .any(|(step, _)| step == "save_encode")
    );
    assert!(
        report
            .timings_ms
            .iter()
            .any(|(step, _)| step == "save_write")
    );
    assert!(report.timings_ms.iter().any(|(step, _)| step == "total"));
    assert!(report.timings_ms.iter().any(|(step, _)| step == "cleanup"));

    let converted = NifFile::load(dst).expect("load converted nif");
    assert_eq!(converted.header.user_version, 12);
    assert_eq!(converted.header.bs_version, 130);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_to_fo4_flattens_shader_data_and_remaps_texture_slots() {
    let dir = temp_dir("convert_fo76");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = fo76_inline_shader_nif();
    nif.save(Some(src.clone())).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions {
            asset_prefix: Some("fo76".to_string()),
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert nif");

    assert!(report.supported, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("FO76 texture slot remap")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    assert_eq!(converted.header.user_version, 12);
    assert_eq!(converted.header.bs_version, 130);

    let shader = converted
        .blocks
        .iter()
        .find(|block| block.type_name == "BSLightingShaderProperty")
        .expect("lighting shader");
    assert!(shader.get_field("Shader Property Data").is_none());
    let flags1 = shader
        .get_field("Shader Flags 1")
        .map(NifValue::as_i64)
        .unwrap_or_default();
    assert_eq!(flags1 & (1 << 7), 0);

    let texset = converted
        .blocks
        .iter()
        .find(|block| block.type_name == "BSShaderTextureSet")
        .expect("texture set");
    let textures = match texset.get_field("Textures") {
        Some(NifValue::Array(textures)) => textures,
        other => panic!("expected texture array, got {other:?}"),
    };
    assert_eq!(textures.len(), 10);
    assert!(matches!(
        textures.get(0),
        Some(NifValue::String(path)) if path == "textures\\fo76\\weapons\\rifle_d.dds"
    ));
    assert!(matches!(
        textures.get(2),
        Some(NifValue::String(path)) if path == "textures\\fo76\\weapons\\rifle_g.dds"
    ));
    assert!(matches!(
        textures.get(7),
        Some(NifValue::String(path)) if path == "textures\\fo76\\weapons\\rifle_s.dds"
    ));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_static_shape_clears_skinned_shader_flag() {
    let dir = temp_dir("convert_fo76_static_shader_skin_flag");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = fo76_static_shape_with_skinned_shader_flag_nif();
    nif.save(Some(src.clone())).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert nif");

    assert!(report.supported, "{:?}", report.errors);
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("cleared Skinned")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let shape = converted
        .blocks
        .iter()
        .find(|block| {
            block.type_name == "BSTriShape"
                && matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "StaticPanel")
        })
        .expect("static shape");
    let shader_id = match shape.get_field("Shader Property") {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
        other => panic!("expected shader ref, got {other:?}"),
    };
    let shader = converted.get_block(shader_id).expect("shader");
    let flags1 = shader
        .get_field("Shader Flags 1")
        .map(NifValue::as_i64)
        .unwrap_or_default();
    assert_eq!(flags1 & 0x02, 0, "{flags1}");
    assert_eq!(flags1 & (1 << 7), 0, "{flags1}");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_flatwoods_skeleton_prunes_hand_helper_shapes() {
    let dir = temp_dir("convert_fo76_flatwoods_skeleton_helpers");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = NifFile::new("fo76");
    nif.blocks[0].set_field(
        "Name",
        NifValue::String("FlatwoodsMonsterExportRoot".to_string()),
    );

    let helper_texset = nif.add_block(
        "BSShaderTextureSet",
        Some(texture_set_fields(
            "textures\\Actors\\FlatwoodsMonster\\FlatwoodsMonster_GlowHands_d.dds",
        )),
    );
    let float_data = nif.add_block("NiFloatData", None);
    let interpolator = nif.add_block(
        "NiFloatInterpolator",
        Some(fields([("Data", NifValue::Ref(float_data as i32))])),
    );
    let controller = nif.add_block(
        "BSLightingShaderPropertyFloatController",
        Some(fields([
            ("Next Controller", NifValue::Ref(-1)),
            ("Target", NifValue::Ref(-1)),
            ("Interpolator", NifValue::Ref(interpolator as i32)),
            (
                "Controlled Variable",
                NifValue::String("Emissive Multiple (F76)".to_string()),
            ),
        ])),
    );
    let helper_shader = nif.add_block(
        "BSLightingShaderProperty",
        Some(fields([
            ("Controller", NifValue::Ref(controller as i32)),
            (
                "Shader Property Data",
                NifValue::Struct(fields([
                    ("Shader Type", NifValue::UInt(0)),
                    ("Texture Set", NifValue::Ref(helper_texset as i32)),
                    ("Num SF1", NifValue::UInt(0)),
                    ("SF1", NifValue::Array(Vec::new())),
                    ("Num SF2", NifValue::UInt(0)),
                    ("SF2", NifValue::Array(Vec::new())),
                ])),
            ),
        ])),
    );
    nif.blocks[controller].set_field("Target", NifValue::Ref(helper_shader as i32));
    let helper_shape = nif.add_block(
        "BSTriShape",
        Some(vegetation_shape_fields("R_Hand:0", helper_shader, false)),
    );

    let keep_texset = nif.add_block(
        "BSShaderTextureSet",
        Some(texture_set_fields(
            "textures\\Actors\\FlatwoodsMonster\\FlatwoodsMonster_d.dds",
        )),
    );
    let keep_shader = nif.add_block(
        "BSLightingShaderProperty",
        Some(fields([(
            "Shader Property Data",
            NifValue::Struct(fields([
                ("Shader Type", NifValue::UInt(0)),
                ("Texture Set", NifValue::Ref(keep_texset as i32)),
                ("Num SF1", NifValue::UInt(0)),
                ("SF1", NifValue::Array(Vec::new())),
                ("Num SF2", NifValue::UInt(0)),
                ("SF2", NifValue::Array(Vec::new())),
            ])),
        )])),
    );
    let keep_shape = nif.add_block(
        "BSTriShape",
        Some(vegetation_shape_fields(
            "flatwoodsmonster_body:0",
            keep_shader,
            false,
        )),
    );
    let hand = nif.add_block(
        "NiNode",
        Some(fields([("Name", NifValue::String("R_Hand".to_string()))])),
    );
    nif.blocks[hand].set_field("Num Children", NifValue::UInt(2));
    nif.blocks[hand].set_field(
        "Children",
        NifValue::Array(vec![
            NifValue::Ref(helper_shape as i32),
            NifValue::Ref(keep_shape as i32),
        ]),
    );
    nif.blocks[0].set_field("Num Children", NifValue::UInt(1));
    nif.blocks[0].set_field(
        "Children",
        NifValue::Array(vec![NifValue::Ref(hand as i32)]),
    );
    nif.save(Some(src.clone())).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert nif");

    assert!(report.supported, "{:?}", report.errors);
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("Flatwoods skeleton hand helper")),
        "{:?}",
        report.changes
    );
    let converted = NifFile::load(dst).expect("load converted nif");
    assert!(!converted.blocks.iter().any(|block| {
        matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "R_Hand:0")
    }));
    assert!(
        !converted
            .blocks
            .iter()
            .any(|block| block.type_name == "BSLightingShaderPropertyFloatController")
    );
    assert!(converted.blocks.iter().any(|block| {
        matches!(
            block.get_field("Name"),
            Some(NifValue::String(name)) if name == "flatwoodsmonster_body:0"
        )
    }));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_named_bgem_effect_shader_writes_fo4_fields() {
    let dir = temp_dir("convert_fo76_named_bgem_effect_shader");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = fo76_named_effect_material_shader_nif();
    nif.save(Some(src.clone())).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions {
            asset_prefix: Some("fo76".to_string()),
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert nif");

    assert!(report.supported, "{:?}", report.errors);
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("BSEffectShaderProperty: filled FO4 defaults")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let shader = converted
        .blocks
        .iter()
        .find(|block| block.type_name == "BSEffectShaderProperty")
        .expect("effect shader");
    assert!(matches!(
        shader.get_field("Name"),
        Some(NifValue::String(path)) if path == "Materials\\Shared\\EditorMarker01.BGEM"
    ));
    assert!(shader.get_field("Shader Flags 1").is_some());
    assert!(shader.get_field("Shader Flags 2").is_some());
    assert!(shader.get_field("UV Scale").is_some());
    assert!(shader.get_field("Source Texture").is_some());
    assert!(shader.get_field("Base Color").is_some());
    assert!(shader.get_field("Environment Map Scale").is_some());
    assert!(
        converted.header.block_sizes[shader.block_id] > 80,
        "FO4 effect shader block should include effect fields, got {} bytes",
        converted.header.block_sizes[shader.block_id]
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_shared_unreadable_np_collision_preserves_per_binding_fallback_report() {
    let dir = temp_dir("convert_fo76_unreadable_np_collision_minimal_fallback");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = NifFile::new("fo76");
    let physics = nif.add_block(
        "bhkPhysicsSystem",
        Some(fields([(
            "Binary Data",
            NifValue::Struct(fields([
                ("Data Size", NifValue::UInt(4)),
                ("Data", NifValue::Bytes(vec![1, 2, 3, 4])),
            ])),
        )])),
    );
    let collision = nif.add_block(
        "bhkNPCollisionObject",
        Some(fields([
            ("Target", NifValue::Ref(0)),
            ("Flags", NifValue::UInt(0x80)),
            ("Data", NifValue::Ref(physics as i32)),
            ("Body ID", NifValue::UInt(0)),
        ])),
    );
    nif.add_block(
        "bhkNPCollisionObject",
        Some(fields([
            ("Target", NifValue::Ref(0)),
            ("Flags", NifValue::UInt(0x80)),
            ("Data", NifValue::Ref(physics as i32)),
            ("Body ID", NifValue::UInt(1)),
        ])),
    );
    nif.blocks[0].set_field("Collision Object", NifValue::Ref(collision as i32));
    nif.save(Some(src.clone())).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert nif");

    assert!(report.supported, "{:?}", report.errors);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("using minimal AABB fallback")),
        "{:?}",
        report.warnings
    );
    assert!(
        report.changes.iter().any(|change| change
            .contains("replaced 2 bhkNPCollisionObject chain(s) (2 degenerate)")
            && change.contains("visible-mesh-aabb-fallback=2")
            && change.contains("stripped-unrecoverable=0")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    assert_eq!(
        converted
            .blocks
            .iter()
            .filter(|block| block.type_name == "bhkNPCollisionObject")
            .count(),
        2
    );
    let blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let meshes =
        extract_preview_meshes_from_blob(&blob, 69.99125, Some(0)).expect("preview collision");
    assert_eq!(meshes.len(), 1, "{meshes:?}");
    assert_eq!(meshes[0].vertices.len(), 8, "{:?}", meshes[0]);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_bscloth_extra_data_blob_preserves_valid_fo4_cloth() {
    let dir = temp_dir("convert_fo76_bscloth_extra_data_blob");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");
    let blob = fixture_bytes("py_creation_lib/tests/fixtures/cloth/bathrobe_outfitm_cloth.hkx");
    let source_format = havok_native::api::hkx_detect_format_full(&blob).unwrap();
    assert_eq!(source_format.kind, "packfile");

    let mut nif = NifFile::new("fo76");
    let nif_bytes = nif.to_bytes().expect("blank NIF serializes");
    let packed =
        nif_core_native::cloth::pack_cloth_blob(&nif_bytes, &blob).expect("pack cloth blob");
    std::fs::write(&src, packed).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert nif");

    assert!(report.supported, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("converted 1 embedded FO76 cloth blob")),
        "{:?}",
        report.changes
    );

    let converted_bytes = std::fs::read(&dst).expect("read converted nif");
    let converted = NifFile::from_bytes(&converted_bytes, None).expect("load converted nif");
    assert!(
        converted
            .blocks
            .iter()
            .any(|block| block.type_name == "BSClothExtraData")
    );
    let converted_blob =
        nif_core_native::cloth::extract_cloth_blob(&converted_bytes).expect("converted cloth blob");
    let format = havok_native::api::hkx_detect_format_full(&converted_blob).unwrap();
    assert_eq!(format.kind, "packfile");
    assert_eq!(format.version, "hk_2014.1.0-r1");
    assert!(
        havok_native::api::hkx_class_summary(&converted_blob)
            .expect("cloth class summary")
            .has_cloth_data
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn invalid_bscloth_extra_data_warns_without_failing_nif_conversion() {
    let dir = temp_dir("invalid_bscloth_extra_data_warns");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = NifFile::new("fo76");
    nif.add_block(
        "BSClothExtraData",
        Some(fields([(
            "Binary Data",
            NifValue::Struct(fields([
                ("Data Size", NifValue::UInt(4)),
                ("Data", NifValue::Bytes(vec![1, 2, 3, 4])),
            ])),
        )])),
    );
    nif.save(Some(src.clone())).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert nif");

    assert!(report.supported, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("Havok cloth: stripped 1 FO76 BSClothExtraData")),
        "{:?}",
        report.warnings
    );
    assert!(
        dst.exists(),
        "invalid embedded cloth must not abort NIF output"
    );
    let converted = NifFile::load(dst).expect("load converted nif");
    assert!(
        converted
            .blocks
            .iter()
            .all(|block| block.type_name != "BSClothExtraData")
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_vegetation_tree_anim_requires_vertex_colors() {
    let dir = temp_dir("convert_fo76_vegetation_tree_anim");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = NifFile::new("fo76");
    let no_color_texset = nif.add_block(
        "BSShaderTextureSet",
        Some(texture_set_fields(
            "textures\\landscape\\plants\\blackberrybush01_d.dds",
        )),
    );
    let color_texset = nif.add_block(
        "BSShaderTextureSet",
        Some(texture_set_fields(
            "textures\\landscape\\grass\\forestgrass01_d.dds",
        )),
    );
    let no_color_shader = nif.add_block(
        "BSLightingShaderProperty",
        Some(fields([
            (
                "Name",
                NifValue::String("Materials\\Landscape\\Plants\\BlackberryBush01.BGSM".to_string()),
            ),
            ("Texture Set", NifValue::Ref(no_color_texset as i32)),
        ])),
    );
    let color_shader = nif.add_block(
        "BSLightingShaderProperty",
        Some(fields([
            (
                "Name",
                NifValue::String("Materials\\Landscape\\Grass\\ForestGrass01.BGSM".to_string()),
            ),
            ("Texture Set", NifValue::Ref(color_texset as i32)),
        ])),
    );
    let no_color_shape = nif.add_block(
        "BSTriShape",
        Some(vegetation_shape_fields(
            "BlackberryBush01:0",
            no_color_shader,
            false,
        )),
    );
    let color_shape = nif.add_block(
        "BSTriShape",
        Some(vegetation_shape_fields(
            "ForestGrass01:0",
            color_shader,
            true,
        )),
    );
    nif.blocks[0].set_field("Num Children", NifValue::UInt(2));
    nif.blocks[0].set_field(
        "Children",
        NifValue::Array(vec![
            NifValue::Ref(no_color_shape as i32),
            NifValue::Ref(color_shape as i32),
        ]),
    );
    nif.save(Some(src.clone())).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert nif");

    assert!(report.supported, "{:?}", report.errors);
    let converted = NifFile::load(dst).expect("load converted nif");
    let no_color_flags = shader_flags_2_for_shape(&converted, "BlackberryBush01:0");
    assert_ne!(no_color_flags & (1 << 0), 0, "{no_color_flags:#x}");
    assert_ne!(no_color_flags & (1 << 4), 0, "{no_color_flags:#x}");
    assert_eq!(no_color_flags & (1 << 5), 0, "{no_color_flags:#x}");
    assert_eq!(no_color_flags & (1 << 29), 0, "{no_color_flags:#x}");

    let color_flags = shader_flags_2_for_shape(&converted, "ForestGrass01:0");
    assert_ne!(color_flags & (1 << 5), 0, "{color_flags:#x}");
    assert_ne!(color_flags & (1 << 29), 0, "{color_flags:#x}");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_file_patches_addon_node_indices_in_native_path() {
    let dir = temp_dir("convert_addon_nodes");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = NifFile::new("fo4");
    let node = nif.add_block(
        "BSValueNode",
        Some(fields([
            ("Name", NifValue::String("AddOnNode20000".to_string())),
            ("Value", NifValue::Int(20000)),
        ])),
    );
    nif.blocks[0].set_field("Num Children", NifValue::UInt(1));
    nif.blocks[0].set_field(
        "Children",
        NifValue::Array(vec![NifValue::Ref(node as i32)]),
    );
    nif.save(Some(src.clone())).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fo4",
        "fo4",
        None,
        &ConvertFileOptions {
            addon_index_map: HashMap::from([(20000, 21001)]),
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert nif");

    assert!(report.supported, "{:?}", report.errors);
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("AddOnNode20000 -> AddOnNode21001")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let block = converted.get_block(node).expect("addon node");
    assert!(matches!(
        block.get_field("Name"),
        Some(NifValue::String(name)) if name == "AddOnNode21001"
    ));
    assert_eq!(block.get_field("Value").map(NifValue::as_i64), Some(21001));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_file_weapon_role_gun_renames_root_to_weapon() {
    let dir = temp_dir("convert_weapon_role_gun");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = NifFile::new("fnv");
    nif.blocks[0].set_field("Name", NifValue::String("OldRoot".to_string()));
    nif.save(Some(src.clone())).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fnv",
        "fo4",
        None,
        &ConvertFileOptions {
            weapon_role: Some("gun".to_string()),
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert nif");

    assert!(report.supported, "{:?}", report.errors);
    let converted = NifFile::load(dst).expect("load converted nif");
    assert!(matches!(
        converted.blocks[0].get_field("Name"),
        Some(NifValue::String(name)) if name == "Weapon"
    ));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_file_fo76_to_fo4_normalizes_controller_root_flags() {
    let dir = temp_dir("convert_fo76_controller_root_flags");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = NifFile::new("fo76");
    let manager = nif.add_block("NiControllerManager", None);
    nif.blocks[0].set_field("Name", NifValue::String("CivWarDoor01".to_string()));
    nif.blocks[0].set_field("Flags", NifValue::UInt(0x400E));
    nif.blocks[0].set_field("Controller", NifValue::Ref(manager as i32));
    nif.save(Some(src.clone())).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert nif");

    assert!(report.supported, "{:?}", report.errors);
    let converted = NifFile::load(dst).expect("load converted nif");
    assert_eq!(
        converted.blocks[0].get_field("Flags").map(NifValue::as_i64),
        Some(14)
    );
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("Normalized FO4 root data")),
        "{:?}",
        report.changes
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_file_fo76_to_fo4_preserves_static_scol_root_flags() {
    let dir = temp_dir("convert_fo76_static_scol_root_flags");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = NifFile::new("fo76");
    nif.blocks[0].set_field(
        "Name",
        NifValue::String("Fishing_IntroBarrel_SCOL".to_string()),
    );
    nif.blocks[0].set_field("Flags", NifValue::UInt(0x400E));
    nif.blocks[0].set_field("Controller", NifValue::Ref(-1));
    nif.save(Some(src.clone())).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert nif");

    assert!(report.supported, "{:?}", report.errors);
    let converted = NifFile::load(dst).expect("load converted nif");
    assert_eq!(
        converted.blocks[0].get_field("Flags").map(NifValue::as_i64),
        Some(0x400E)
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_file_weapon_role_melee_attaches_root_weapon_marker() {
    let dir = temp_dir("convert_weapon_role_melee");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = NifFile::new("fnv");
    let source_marker_id = nif.add_block(
        "NiStringExtraData",
        Some(IndexMap::from([
            ("Name".to_string(), NifValue::String("Prn".to_string())),
            (
                "String Data".to_string(),
                NifValue::String("WeaponBack".to_string()),
            ),
        ])),
    );
    let wrong_marker_id = nif.add_block(
        "NiStringExtraData",
        Some(IndexMap::from([
            ("Name".to_string(), NifValue::String("WEAPON".to_string())),
            (
                "String Data".to_string(),
                NifValue::String("WEAPON".to_string()),
            ),
        ])),
    );
    nif.blocks[0].set_field("Num Extra Data List", NifValue::UInt(2));
    nif.blocks[0].set_field(
        "Extra Data List",
        NifValue::Array(vec![
            NifValue::Ref(source_marker_id as i32),
            NifValue::Ref(wrong_marker_id as i32),
        ]),
    );
    nif.save(Some(src.clone())).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fnv",
        "fo4",
        None,
        &ConvertFileOptions {
            weapon_role: Some("melee".to_string()),
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert nif");

    assert!(report.supported, "{:?}", report.errors);
    let converted = NifFile::load(dst).expect("load converted nif");
    let root = converted.get_block(0).expect("root");
    let extra_ids: Vec<usize> = match root.get_field("Extra Data List") {
        Some(NifValue::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                NifValue::Ref(id) if *id >= 0 => Some(*id as usize),
                _ => None,
            })
            .collect(),
        other => panic!("expected extra data list, got {other:?}"),
    };
    let weapon_marker_ids: Vec<usize> = converted
        .blocks
        .iter()
        .filter(|block| {
            block.type_name == "NiStringExtraData"
                && matches!(
                    block.get_field("Name"),
                    Some(NifValue::String(name)) if name == "Prn"
                )
                && matches!(
                    block.get_field("String Data"),
                    Some(NifValue::String(value)) if value == "WEAPON"
                )
        })
        .map(|block| block.block_id)
        .collect();

    assert_eq!(
        root.get_field("Num Extra Data List").map(NifValue::as_i64),
        Some(1)
    );
    assert_eq!(extra_ids, weapon_marker_ids);
    assert_eq!(extra_ids.len(), 1);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_file_melee_marker_does_not_steal_or_mutate_child_extra_data() {
    let dir = temp_dir("convert_melee_marker_parentage");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out/converted.nif");

    let mut nif = NifFile::new("skyrimse");
    let shared_child_id = nif.add_block(
        "NiNode",
        Some(IndexMap::from([(
            "Name".to_string(),
            NifValue::String("SharedMarkerChild".to_string()),
        )])),
    );
    let own_child_id = nif.add_block(
        "NiNode",
        Some(IndexMap::from([(
            "Name".to_string(),
            NifValue::String("OwnMarkerChild".to_string()),
        )])),
    );
    let shared_marker_id = nif.add_block(
        "NiStringExtraData",
        Some(IndexMap::from([
            ("Name".to_string(), NifValue::String("Prn".to_string())),
            (
                "String Data".to_string(),
                NifValue::String("WeaponBack".to_string()),
            ),
        ])),
    );
    let child_marker_id = nif.add_block(
        "NiStringExtraData",
        Some(IndexMap::from([
            ("Name".to_string(), NifValue::String("WEAPON".to_string())),
            (
                "String Data".to_string(),
                NifValue::String("CHILD_ONLY".to_string()),
            ),
        ])),
    );
    nif.blocks[0].set_field("Num Children", NifValue::UInt(2));
    nif.blocks[0].set_field(
        "Children",
        NifValue::Array(vec![
            NifValue::Ref(shared_child_id as i32),
            NifValue::Ref(own_child_id as i32),
        ]),
    );
    for block_id in [0, shared_child_id] {
        nif.blocks[block_id].set_field("Num Extra Data List", NifValue::UInt(1));
        nif.blocks[block_id].set_field(
            "Extra Data List",
            NifValue::Array(vec![NifValue::Ref(shared_marker_id as i32)]),
        );
    }
    nif.blocks[own_child_id].set_field("Num Extra Data List", NifValue::UInt(1));
    nif.blocks[own_child_id].set_field(
        "Extra Data List",
        NifValue::Array(vec![NifValue::Ref(child_marker_id as i32)]),
    );
    nif.save(Some(src.clone())).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "skyrimse",
        "fo4",
        None,
        &ConvertFileOptions {
            weapon_role: Some("melee".to_string()),
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert nif");
    assert!(report.supported, "{:?}", report.errors);

    let converted = NifFile::load(dst).expect("load converted nif");
    let extra_ids =
        |block: &nif_core_native::model::NifBlock| match block.get_field("Extra Data List") {
            Some(NifValue::Array(values)) => values
                .iter()
                .filter_map(|value| match value {
                    NifValue::Ref(id) if *id >= 0 => Some(*id as usize),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
    let root_extra_ids = extra_ids(&converted.blocks[0]);
    let root_markers = root_extra_ids
        .iter()
        .filter_map(|id| converted.get_block(*id))
        .filter(|block| {
            block.type_name == "NiStringExtraData"
                && matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "Prn")
                && matches!(block.get_field("String Data"), Some(NifValue::String(value)) if value == "WEAPON")
        })
        .collect::<Vec<_>>();
    assert_eq!(root_markers.len(), 1);

    let shared_child = converted
        .blocks
        .iter()
        .find(|block| {
            matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "SharedMarkerChild")
        })
        .expect("shared-marker child");
    let own_child = converted
        .blocks
        .iter()
        .find(|block| {
            matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "OwnMarkerChild")
        })
        .expect("own-marker child");
    let shared_child_extra_ids = extra_ids(shared_child);
    let own_child_extra_ids = extra_ids(own_child);
    assert_eq!(shared_child_extra_ids.len(), 1);
    assert_eq!(own_child_extra_ids.len(), 1);

    let shared_marker = converted
        .get_block(shared_child_extra_ids[0])
        .expect("preserved shared marker");
    assert!(matches!(
        shared_marker.get_field("Name"),
        Some(NifValue::String(name)) if name == "Prn"
    ));
    assert!(matches!(
        shared_marker.get_field("String Data"),
        Some(NifValue::String(value)) if value == "WeaponBack"
    ));
    let child_marker = converted
        .get_block(own_child_extra_ids[0])
        .expect("preserved child-only marker");
    assert!(matches!(
        child_marker.get_field("Name"),
        Some(NifValue::String(name)) if name == "WEAPON"
    ));
    assert!(matches!(
        child_marker.get_field("String Data"),
        Some(NifValue::String(value)) if value == "CHILD_ONLY"
    ));

    let root_marker_id = root_markers[0].block_id;
    assert!(!shared_child_extra_ids.contains(&root_marker_id));
    assert!(!own_child_extra_ids.contains(&root_marker_id));
    assert!(!root_extra_ids.contains(&shared_child_extra_ids[0]));
    assert!(!root_extra_ids.contains(&own_child_extra_ids[0]));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_file_preserves_skinned_flag_after_legacy_shader_conversion() {
    let dir = temp_dir("convert_skinned_shader");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = fnv_legacy_skinned_shader_nif(false);
    nif.save(Some(src.clone())).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fnv",
        "fo4",
        None,
        &ConvertFileOptions {
            translation_maps_dir: Some(translation_maps_dir()),
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert nif");

    assert!(report.supported, "{:?}", report.errors);
    assert_eq!(report.shapes_skinned, 1);
    let converted = NifFile::load(dst).expect("load converted nif");
    let shape = converted
        .blocks
        .iter()
        .find(|block| block.type_name == "BSSubIndexTriShape")
        .expect("converted shape");
    let shader_id = match shape.get_field("Shader Property") {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
        other => panic!("expected shader ref, got {other:?}"),
    };
    let shader = converted.get_block(shader_id).expect("shader");
    assert_eq!(shader.type_name, "BSLightingShaderProperty");
    let flags = shader
        .get_field("Shader Flags 1")
        .map(NifValue::as_i64)
        .unwrap_or_default();
    assert!(flags & 0x02 != 0, "{flags}");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_file_fails_without_writing_on_legacy_skin_conversion_error() {
    let dir = temp_dir("convert_skin_error");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = fnv_legacy_skinned_shader_nif(true);
    nif.save(Some(src.clone())).expect("write source nif");

    let result = convert_nif_file(
        &src,
        &dst,
        "fnv",
        "fo4",
        None,
        &ConvertFileOptions {
            translation_maps_dir: Some(translation_maps_dir()),
            ..ConvertFileOptions::default()
        },
    );

    let error = result.expect_err("skin conversion should hard-fail");
    assert!(
        error.to_string().contains("legacy skin conversion"),
        "{error}"
    );
    assert!(!dst.exists(), "partial output was written");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_file_emits_first_person_sibling_for_fo4_skinned_shape() {
    let dir = temp_dir("convert_first_person");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("armor.nif");

    let mut nif = fo4_arm_skinned_nif();
    nif.save(Some(src.clone())).expect("write source nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fo4",
        "fo4",
        None,
        &ConvertFileOptions {
            emit_first_person: true,
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert nif");

    assert!(report.supported, "{:?}", report.errors);
    let emitted = report
        .emitted_first_person
        .as_deref()
        .expect("first-person sibling");
    assert!(PathBuf::from(emitted).exists(), "{emitted}");

    let first_person = NifFile::load(emitted).expect("load emitted first-person nif");
    let shape = first_person
        .blocks
        .iter()
        .find(|block| block.type_name == "BSSubIndexTriShape")
        .expect("first-person shape");
    assert_eq!(
        shape.get_field("Num Triangles").map(NifValue::as_i64),
        Some(1)
    );

    let _ = std::fs::remove_dir_all(dir);
}

fn fo76_inline_shader_nif() -> NifFile {
    let mut nif = NifFile::new("fo76");
    let texset = nif.add_block(
        "BSShaderTextureSet",
        Some(fields([
            ("Num Textures", NifValue::UInt(11)),
            (
                "Textures",
                NifValue::Array(vec![
                    NifValue::String("weapons/rifle_d.dds".to_string()),
                    NifValue::String("weapons/rifle_n.dds".to_string()),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                    NifValue::String("weapons/rifle_r.dds".to_string()),
                    NifValue::String("weapons/rifle_l.dds".to_string()),
                ]),
            ),
        ])),
    );
    let shader = nif.add_block(
        "BSLightingShaderProperty",
        Some(fields([
            ("Texture Set", NifValue::Ref(texset as i32)),
            (
                "Shader Property Data",
                NifValue::Struct(fields([
                    ("Shader Type", NifValue::UInt(0)),
                    ("Texture Set", NifValue::Ref(texset as i32)),
                    ("Num SF1", NifValue::UInt(2)),
                    (
                        "SF1",
                        NifValue::Array(vec![
                            NifValue::UInt(2893749418),
                            NifValue::UInt(2262553490),
                        ]),
                    ),
                    ("Num SF2", NifValue::UInt(0)),
                    ("SF2", NifValue::Array(Vec::new())),
                    ("Smoothness", NifValue::Float(0.45)),
                ])),
            ),
        ])),
    );
    nif.blocks[0].set_field("Num Children", NifValue::UInt(1));
    nif.blocks[0].set_field(
        "Children",
        NifValue::Array(vec![NifValue::Ref(shader as i32)]),
    );
    nif
}

fn fo76_static_shape_with_skinned_shader_flag_nif() -> NifFile {
    let mut nif = NifFile::new("fo76");
    let texset = nif.add_block(
        "BSShaderTextureSet",
        Some(texture_set_fields(
            "textures\\setdressing\\signage\\billboardstructure01_d.dds",
        )),
    );
    let shader = nif.add_block(
        "BSLightingShaderProperty",
        Some(fields([
            ("Texture Set", NifValue::Ref(texset as i32)),
            (
                "Shader Property Data",
                NifValue::Struct(fields([
                    ("Shader Type", NifValue::UInt(0)),
                    ("Texture Set", NifValue::Ref(texset as i32)),
                    ("Num SF1", NifValue::UInt(0)),
                    ("SF1", NifValue::Array(Vec::new())),
                    ("Num SF2", NifValue::UInt(2)),
                    (
                        "SF2",
                        NifValue::Array(vec![
                            NifValue::UInt(3744563888),
                            NifValue::UInt(2893749418),
                        ]),
                    ),
                    ("Smoothness", NifValue::Float(0.45)),
                ])),
            ),
        ])),
    );
    let shape = nif.add_block(
        "BSTriShape",
        Some(vegetation_shape_fields("StaticPanel", shader, false)),
    );
    nif.blocks[0].set_field("Num Children", NifValue::UInt(1));
    nif.blocks[0].set_field(
        "Children",
        NifValue::Array(vec![NifValue::Ref(shape as i32)]),
    );
    nif
}

fn fo76_named_effect_material_shader_nif() -> NifFile {
    let mut nif = NifFile::new("fo76");
    let shader = nif.add_block(
        "BSEffectShaderProperty",
        Some(fields([(
            "Name",
            NifValue::String(
                "C:\\Projects\\76\\Build\\PC\\Data\\Materials\\Shared\\EditorMarker01.BGEM"
                    .to_string(),
            ),
        )])),
    );
    nif.blocks[0].set_field("Num Children", NifValue::UInt(1));
    nif.blocks[0].set_field(
        "Children",
        NifValue::Array(vec![NifValue::Ref(shader as i32)]),
    );
    nif
}

fn fnv_legacy_skinned_shader_nif(invalid_skin: bool) -> NifFile {
    let mut nif = NifFile::new("fo4");
    let pelvis = nif.add_block(
        "NiNode",
        Some(fields([(
            "Name",
            NifValue::String("Bip01 Pelvis".to_string()),
        )])),
    );
    let shader = nif.add_block(
        "BSShaderPPLightingProperty",
        Some(fields([
            ("Shader Flags", NifValue::UInt(0)),
            ("Shader Flags 2", NifValue::UInt(0)),
        ])),
    );
    let skin_data = nif.add_block(
        "NiSkinData",
        Some(fields([
            ("Num Bones", NifValue::UInt(1)),
            ("Has Vertex Weights", NifValue::Bool(true)),
        ])),
    );
    let mut skin_fields = fields([
        ("Skin Partition", NifValue::Ref(-1)),
        ("Skeleton Root", NifValue::Ref(0)),
        ("Num Bones", NifValue::UInt(1)),
        ("Bones", NifValue::Array(vec![NifValue::Ref(pelvis as i32)])),
    ]);
    if !invalid_skin {
        skin_fields.insert("Data".to_string(), NifValue::Ref(skin_data as i32));
    }
    let skin = nif.add_block("NiSkinInstance", Some(skin_fields));
    let shape = nif.add_block(
        "BSTriShape",
        Some(fields([
            ("Name", NifValue::String("Body".to_string())),
            ("Skin", NifValue::Ref(skin as i32)),
            ("Shader Property", NifValue::Ref(shader as i32)),
            ("Alpha Property", NifValue::Ref(-1)),
            ("Vertex Desc", NifValue::Int(vertex_desc_skinned(false))),
            (
                "Vertex Data",
                NifValue::Array(vec![
                    skinned_vertex([0.0, 0.0, 0.0]),
                    skinned_vertex([1.0, 0.0, 0.0]),
                    skinned_vertex([0.0, 1.0, 0.0]),
                ]),
            ),
            ("Triangles", NifValue::Array(vec![triangle(0, 1, 2)])),
            ("Num Vertices", NifValue::UInt(3)),
            ("Num Triangles", NifValue::UInt(1)),
            ("Data Size", NifValue::UInt(90)),
        ])),
    );
    nif.blocks[0].set_field("Num Children", NifValue::UInt(2));
    nif.blocks[0].set_field(
        "Children",
        NifValue::Array(vec![
            NifValue::Ref(pelvis as i32),
            NifValue::Ref(shape as i32),
        ]),
    );
    nif
}

fn fo4_arm_skinned_nif() -> NifFile {
    let mut nif = NifFile::new("fo4");
    let bone = nif.add_block(
        "NiNode",
        Some(fields([(
            "Name",
            NifValue::String("LArm_ForeArm1".to_string()),
        )])),
    );
    let skin = nif.add_block(
        "BSSkin::Instance",
        Some(fields([
            ("Skeleton Root", NifValue::Ref(0)),
            ("Data", NifValue::Ref(-1)),
            ("Num Bones", NifValue::UInt(1)),
            ("Bones", NifValue::Array(vec![NifValue::Ref(bone as i32)])),
            ("Num Scales", NifValue::UInt(0)),
            ("Scales", NifValue::Array(Vec::new())),
        ])),
    );
    let shape = nif.add_block(
        "BSSubIndexTriShape",
        Some(fields([
            ("Name", NifValue::String("Sleeve:0".to_string())),
            ("Skin", NifValue::Ref(skin as i32)),
            ("Shader Property", NifValue::Ref(-1)),
            ("Alpha Property", NifValue::Ref(-1)),
            ("Vertex Desc", NifValue::Int(vertex_desc_skinned(false))),
            ("Num Triangles", NifValue::UInt(1)),
            ("Num Vertices", NifValue::UInt(3)),
            ("Data Size", NifValue::UInt(90)),
            (
                "Vertex Data",
                NifValue::Array(vec![
                    skinned_vertex([0.0, 0.0, 0.0]),
                    skinned_vertex([1.0, 0.0, 0.0]),
                    skinned_vertex([0.0, 1.0, 0.0]),
                ]),
            ),
            ("Triangles", NifValue::Array(vec![triangle(0, 1, 2)])),
            ("Num Primitives", NifValue::UInt(1)),
            ("Num Segments", NifValue::UInt(1)),
            ("Total Segments", NifValue::UInt(1)),
            ("Segment", NifValue::Array(vec![segment(0, 1)])),
        ])),
    );
    nif.blocks[0].set_field("Num Children", NifValue::UInt(2));
    nif.blocks[0].set_field(
        "Children",
        NifValue::Array(vec![
            NifValue::Ref(bone as i32),
            NifValue::Ref(shape as i32),
        ]),
    );
    nif
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn translation_maps_dir() -> PathBuf {
    repo_root().join("bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps")
}

fn skinned_vertex(position: [f32; 3]) -> NifValue {
    NifValue::Struct(fields([
        ("Vertex", NifValue::Vec3(position)),
        ("Bitangent X", NifValue::Float(0.0)),
        ("UV", tex_coord([0.0, 0.0])),
        ("Normal", NifValue::Vec3([0.0, 0.0, 1.0])),
        ("Bitangent Y", NifValue::Float(1.0)),
        ("Tangent", NifValue::Vec3([1.0, 0.0, 0.0])),
        ("Bitangent Z", NifValue::Float(0.0)),
        (
            "Bone Weights",
            NifValue::Array(vec![
                NifValue::Float(1.0),
                NifValue::Float(0.0),
                NifValue::Float(0.0),
                NifValue::Float(0.0),
            ]),
        ),
        (
            "Bone Indices",
            NifValue::Array(vec![
                NifValue::UInt(0),
                NifValue::UInt(0),
                NifValue::UInt(0),
                NifValue::UInt(0),
            ]),
        ),
    ]))
}

fn vegetation_shape_fields(
    name: &str,
    shader_id: usize,
    has_vertex_colors: bool,
) -> IndexMap<String, NifValue> {
    fields([
        ("Name", NifValue::String(name.to_string())),
        ("Shader Property", NifValue::Ref(shader_id as i32)),
        ("Alpha Property", NifValue::Ref(-1)),
        (
            "Vertex Desc",
            NifValue::Int(basic_vertex_desc(has_vertex_colors)),
        ),
        (
            "Vertex Data",
            NifValue::Array(vec![
                basic_vertex([0.0, 0.0, 0.0], has_vertex_colors),
                basic_vertex([1.0, 0.0, 0.0], has_vertex_colors),
                basic_vertex([0.0, 1.0, 0.0], has_vertex_colors),
            ]),
        ),
        ("Triangles", NifValue::Array(vec![triangle(0, 1, 2)])),
        ("Num Vertices", NifValue::UInt(3)),
        ("Num Triangles", NifValue::UInt(1)),
        (
            "Data Size",
            NifValue::UInt(if has_vertex_colors { 93 } else { 81 }),
        ),
    ])
}

fn basic_vertex_desc(has_vertex_colors: bool) -> i64 {
    let stride = if has_vertex_colors { 6 } else { 5 };
    let mut flags = 0x0001 | 0x0002 | 0x0008 | 0x0010;
    let mut color_offset = 0;
    if has_vertex_colors {
        flags |= 0x0020;
        color_offset = 5;
    }
    stride | (2 << 8) | (3 << 16) | (4 << 20) | (color_offset << 24) | (flags << 44)
}

fn skyrim_vertex_desc(has_vertex_colors: bool) -> i64 {
    let stride = if has_vertex_colors { 8 } else { 7 };
    let mut flags = 0x0001 | 0x0002 | 0x0008 | 0x0010;
    let mut color_offset = 0;
    if has_vertex_colors {
        flags |= 0x0020;
        color_offset = 7;
    }
    stride | (4 << 8) | (5 << 16) | (6 << 20) | (color_offset << 24) | (flags << 44)
}

fn basic_vertex(position: [f32; 3], has_vertex_colors: bool) -> NifValue {
    let mut data = fields([
        ("Vertex", NifValue::Vec3(position)),
        ("Bitangent X", NifValue::Float(0.0)),
        ("UV", tex_coord([0.0, 0.0])),
        ("Normal", NifValue::Vec3([0.0, 0.0, 1.0])),
        ("Bitangent Y", NifValue::Float(1.0)),
        ("Tangent", NifValue::Vec3([1.0, 0.0, 0.0])),
        ("Bitangent Z", NifValue::Float(0.0)),
    ]);
    if has_vertex_colors {
        data.insert(
            "Vertex Colors".to_string(),
            NifValue::Color4([1.0, 1.0, 1.0, 1.0]),
        );
    }
    NifValue::Struct(data)
}

fn texture_set_fields(texture_path: &str) -> IndexMap<String, NifValue> {
    fields([
        ("Num Textures", NifValue::UInt(10)),
        (
            "Textures",
            NifValue::Array(vec![
                NifValue::String(texture_path.to_string()),
                NifValue::String(String::new()),
                NifValue::String(String::new()),
                NifValue::String(String::new()),
                NifValue::String(String::new()),
                NifValue::String(String::new()),
                NifValue::String(String::new()),
                NifValue::String(String::new()),
                NifValue::String(String::new()),
                NifValue::String(String::new()),
            ]),
        ),
    ])
}

fn shader_flags_2_for_shape(nif: &NifFile, shape_name: &str) -> i64 {
    let shape = nif
        .blocks
        .iter()
        .find(|block| {
            block.type_name == "BSTriShape"
                && matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == shape_name)
        })
        .unwrap_or_else(|| panic!("shape {shape_name}"));
    let shader_id = match shape.get_field("Shader Property") {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
        other => panic!("expected shader ref, got {other:?}"),
    };
    nif.get_block(shader_id)
        .and_then(|shader| shader.get_field("Shader Flags 2"))
        .map(NifValue::as_i64)
        .unwrap_or_default()
}

fn segment(start: u64, count: u64) -> NifValue {
    NifValue::Struct(fields([
        ("Start Index", NifValue::UInt(start)),
        ("Num Primitives", NifValue::UInt(count)),
        ("Parent Array Index", NifValue::UInt(u32::MAX as u64)),
        ("Num Sub Segments", NifValue::UInt(0)),
        ("Sub Segment", NifValue::Array(Vec::new())),
    ]))
}

fn triangle(v1: u64, v2: u64, v3: u64) -> NifValue {
    NifValue::Struct(fields([
        ("v1", NifValue::UInt(v1)),
        ("v2", NifValue::UInt(v2)),
        ("v3", NifValue::UInt(v3)),
    ]))
}

fn tex_coord(uv: [f32; 2]) -> NifValue {
    NifValue::Struct(fields([
        ("u", NifValue::Float(uv[0] as f64)),
        ("v", NifValue::Float(uv[1] as f64)),
    ]))
}

fn fields<const N: usize>(entries: [(&str, NifValue); N]) -> IndexMap<String, NifValue> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}
