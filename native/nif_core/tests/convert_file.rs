use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use havok_native::collision::{
    PreviewMesh, RawCompressedMeshData, SourcePrimitiveShape,
    extract_direct_source_primitive_from_blob, extract_preview_meshes_from_blob,
    extract_raw_compressed_meshes_from_blob, extract_source_polytopes_from_blob,
    parse_fo4_compressed_mesh,
};
use havok_native::hkx::{HkxMember, parse_tagfile, types::HkxValue};
use indexmap::IndexMap;
use nif_core_native::convert_file::{ConvertFileError, ConvertFileOptions, convert_nif_file};
use nif_core_native::model::{NifBlock, NifFile, NifValue};
use nif_core_native::schema::NifSchema;
use nif_core_native::skin::LegacySkinPolicy;
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

fn assert_all_nif_refs_resolve(nif: &NifFile) {
    let schema = NifSchema::from_generated();
    for block in &nif.blocks {
        for (field, refs) in block.get_all_ref_fields(&schema) {
            for reference in refs {
                assert!(
                    reference < 0 || (reference as usize) < nif.blocks.len(),
                    "block {} {} field {field} has invalid reference {reference}",
                    block.block_id,
                    block.type_name
                );
            }
        }
    }
}

fn assert_animation_palette_entries_resolve(nif: &NifFile) {
    for entry in nif
        .blocks
        .iter()
        .filter(|block| block.type_name == "NiDefaultAVObjectPalette")
        .flat_map(|palette| match palette.get_field("Objs") {
            Some(NifValue::Array(entries)) => entries.as_slice(),
            _ => &[],
        })
    {
        let NifValue::Struct(fields) = entry else {
            panic!("animation palette has a non-struct entry");
        };
        let name = match fields.get("Name") {
            Some(NifValue::String(name)) => name.as_str(),
            _ => "<unnamed>",
        };
        let reference = fields.get("AV Object").map(NifValue::as_i64).unwrap_or(-1);
        assert!(
            reference >= 0 && (reference as usize) < nif.blocks.len(),
            "palette entry {name:?} has invalid AV Object {reference}"
        );
    }
}

fn assert_no_pre_fo4_av_flags(nif: &NifFile) {
    let schema = NifSchema::from_generated();
    for block in &nif.blocks {
        if !schema.is_subtype_of(&block.type_name, "NiAVObject") {
            continue;
        }
        let flags = block
            .get_field("Flags")
            .map(NifValue::as_i64)
            .unwrap_or_default();
        assert_eq!(
            flags & 0x80000,
            0,
            "block {} {} retains pre-FO4 AV flag in {flags:#x}",
            block.block_id,
            block.type_name
        );
    }
}

fn assert_all_blocks_reachable(nif: &NifFile) {
    let schema = NifSchema::from_generated();
    let mut reachable = std::collections::HashSet::new();
    let mut pending = nif
        .header
        .footer_roots
        .iter()
        .copied()
        .filter_map(|root| usize::try_from(root).ok())
        .collect::<Vec<_>>();
    while let Some(block_id) = pending.pop() {
        if block_id >= nif.blocks.len() || !reachable.insert(block_id) {
            continue;
        }
        pending.extend(
            nif.blocks[block_id]
                .get_refs(&schema)
                .into_iter()
                .filter_map(|reference| usize::try_from(reference).ok()),
        );
    }
    let detached = nif
        .blocks
        .iter()
        .filter(|block| !reachable.contains(&block.block_id))
        .map(|block| (block.block_id, block.type_name.as_str()))
        .collect::<Vec<_>>();
    assert!(
        detached.is_empty(),
        "converted NIF has detached blocks: {detached:?}"
    );
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

fn texture_string(value: &NifValue) -> &str {
    match value {
        NifValue::String(path) => path,
        _ => "",
    }
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

fn embedded_havok_blobs(nif: &NifFile) -> Vec<Vec<u8>> {
    nif.blocks
        .iter()
        .filter(|block| block.type_name == "bhkPhysicsSystem")
        .filter_map(|block| {
            let Some(NifValue::Struct(binary_data)) = block.get_field("Binary Data") else {
                return None;
            };
            let Some(data) = binary_data.get("Data") else {
                return None;
            };
            let bytes = match data {
                NifValue::Bytes(bytes) => bytes.clone(),
                NifValue::Array(values) => {
                    values.iter().map(|value| value.as_i64() as u8).collect()
                }
                _ => Vec::new(),
            };
            (!bytes.is_empty()).then_some(bytes)
        })
        .collect()
}

fn member_value<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a HkxValue> {
    members
        .iter()
        .find(|member| member.name == name)
        .map(|member| &member.value)
}

fn hkx_int(value: &HkxValue) -> Option<i64> {
    match value {
        HkxValue::I8(value) => Some(i64::from(*value)),
        HkxValue::U8(value) => Some(i64::from(*value)),
        HkxValue::I16(value) => Some(i64::from(*value)),
        HkxValue::U16(value) => Some(i64::from(*value)),
        HkxValue::I32(value) => Some(i64::from(*value)),
        HkxValue::U32(value) => Some(i64::from(*value)),
        HkxValue::I64(value) => Some(*value),
        HkxValue::U64(value) => Some(*value as i64),
        _ => None,
    }
}

fn compound_instance_counts(blob: &[u8]) -> Vec<usize> {
    let tagfile = parse_tagfile(blob).expect("parse TAG0 source collision");
    let hkx = tagfile
        .materialize_hkx()
        .expect("materialize TAG0 source collision");
    hkx.objects()
        .iter()
        .filter(|object| object.class_name == "hknpCompoundShape")
        .filter_map(|object| {
            let instances = member_value(&object.members, "instances")?.as_object_members()?;
            let elements = member_value(instances, "elements")?;
            match elements {
                HkxValue::Array(values) => Some(values.len()),
                _ => None,
            }
        })
        .collect()
}

fn physics_motion_ids(blob: &[u8]) -> (Vec<i64>, usize) {
    use havok_native::hkx::types::HkxValue;

    let hkx = havok_native::hkx::model::HkxFile::read(blob).expect("parse physics blob");
    let psd = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpPhysicsSystemData")
        .expect("physics system data");
    let body_cinfos = psd
        .members
        .iter()
        .find(|member| member.name == "bodyCinfos")
        .and_then(|member| match &member.value {
            HkxValue::Array(values) => Some(values.as_slice()),
            _ => None,
        })
        .expect("bodyCinfos array");
    let motion_cinfo_count = psd
        .members
        .iter()
        .find(|member| member.name == "motionCinfos")
        .and_then(|member| match &member.value {
            HkxValue::Array(values) => Some(values.len()),
            _ => None,
        })
        .expect("motionCinfos array");
    let motion_ids = body_cinfos
        .iter()
        .map(|body| {
            let members = body.as_object_members().expect("body cinfo object");
            let value = &members
                .iter()
                .find(|member| member.name == "motionId")
                .expect("body motionId")
                .value;
            match value {
                HkxValue::I32(value) => i64::from(*value),
                HkxValue::U32(value) => i64::from(*value),
                HkxValue::I64(value) => *value,
                HkxValue::U64(value) => *value as i64,
                other => panic!("unsupported motionId value: {other:?}"),
            }
        })
        .collect();
    (motion_ids, motion_cinfo_count)
}

fn hkx_object_array<'a>(
    object: &'a havok_native::hkx::model::HkxObject,
    name: &str,
) -> &'a [havok_native::hkx::types::HkxValue] {
    use havok_native::hkx::types::HkxValue;

    object
        .members
        .iter()
        .find(|member| member.name == name)
        .and_then(|member| match &member.value {
            HkxValue::Array(values) => Some(values.as_slice()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{name} array missing"))
}

fn hkx_member_i64(value: &havok_native::hkx::types::HkxValue, name: &str) -> i64 {
    use havok_native::hkx::types::HkxValue;

    let members = value.as_object_members().expect("HKX object members");
    match &members
        .iter()
        .find(|member| member.name == name)
        .unwrap_or_else(|| panic!("{name} member missing"))
        .value
    {
        HkxValue::I32(value) => i64::from(*value),
        HkxValue::U32(value) => i64::from(*value),
        HkxValue::I64(value) => *value,
        HkxValue::U64(value) => *value as i64,
        HkxValue::U16(value) => i64::from(*value),
        HkxValue::U8(value) => i64::from(*value),
        other => panic!("unsupported {name} value: {other:?}"),
    }
}

fn hkx_member_f32(value: &havok_native::hkx::types::HkxValue, name: &str) -> f32 {
    use havok_native::hkx::types::HkxValue;

    let members = value.as_object_members().expect("HKX object members");
    match &members
        .iter()
        .find(|member| member.name == name)
        .unwrap_or_else(|| panic!("{name} member missing"))
        .value
    {
        HkxValue::F32(value) => *value,
        HkxValue::Half(value) => *value,
        other => panic!("unsupported {name} value: {other:?}"),
    }
}

fn hkx_member_vec4(value: &havok_native::hkx::types::HkxValue, name: &str) -> [f32; 4] {
    let members = value.as_object_members().expect("HKX object members");
    let HkxValue::F32List(values) = &members
        .iter()
        .find(|member| member.name == name)
        .unwrap_or_else(|| panic!("{name} member missing"))
        .value
    else {
        panic!("unsupported {name} value");
    };
    assert!(values.len() >= 4, "{name} must contain four floats");
    [values[0], values[1], values[2], values[3]]
}

fn quat_mul_xyzw(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

fn quat_rotate_vector_xyzw(quaternion: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
    let inverse_norm = quaternion
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt()
        .recip();
    let q = [
        quaternion[0] * inverse_norm,
        quaternion[1] * inverse_norm,
        quaternion[2] * inverse_norm,
        quaternion[3] * inverse_norm,
    ];
    let t = [
        2.0 * (q[1] * vector[2] - q[2] * vector[1]),
        2.0 * (q[2] * vector[0] - q[0] * vector[2]),
        2.0 * (q[0] * vector[1] - q[1] * vector[0]),
    ];
    [
        vector[0] + q[3] * t[0] + q[1] * t[2] - q[2] * t[1],
        vector[1] + q[3] * t[1] + q[2] * t[0] - q[0] * t[2],
        vector[2] + q[3] * t[2] + q[0] * t[1] - q[1] * t[0],
    ]
}

fn assert_quaternion_equivalent(actual: [f32; 4], expected: [f32; 4], context: &str) {
    let dot = actual
        .iter()
        .zip(expected.iter())
        .map(|(left, right)| left * right)
        .sum::<f32>()
        .abs();
    assert!((dot - 1.0).abs() < 1e-4, "{context}: quaternion dot={dot}");
}

fn np_collision_blob_for_target(nif: &NifFile, target_id: i32) -> Option<(Vec<u8>, usize)> {
    for block in &nif.blocks {
        if block.type_name != "bhkNPCollisionObject" {
            continue;
        }
        let target = match block.get_field("Target") {
            Some(NifValue::Ref(value)) => *value,
            Some(NifValue::Int(value)) => *value as i32,
            Some(NifValue::UInt(value)) => *value as i32,
            _ => continue,
        };
        if target != target_id {
            continue;
        }
        let data_ref = match block.get_field("Data") {
            Some(NifValue::Ref(value)) if *value >= 0 => *value as usize,
            Some(NifValue::Int(value)) if *value >= 0 => *value as usize,
            Some(NifValue::UInt(value)) => *value as usize,
            _ => continue,
        };
        let body_id = match block.get_field("Body ID") {
            Some(NifValue::Int(value)) if *value >= 0 => *value as usize,
            Some(NifValue::UInt(value)) => *value as usize,
            Some(NifValue::Ref(value)) if *value >= 0 => *value as usize,
            _ => 0,
        };
        let physics = nif.get_block(data_ref)?;
        let Some(NifValue::Struct(binary_data)) = physics.get_field("Binary Data") else {
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
            return Some((bytes, body_id));
        }
    }
    None
}

fn ref_usize(value: Option<&NifValue>) -> Option<usize> {
    match value {
        Some(NifValue::Ref(value)) if *value >= 0 => Some(*value as usize),
        Some(NifValue::Int(value)) if *value >= 0 => Some(*value as usize),
        Some(NifValue::UInt(value)) => Some(*value as usize),
        _ => None,
    }
}

#[test]
fn converts_real_fo76_whitespring_bunker_compressed_mesh_header_to_fo4() {
    let src = repo_path(
        "extracted/fo76/meshes/architecture/neoclassical/whitespring/WhiteSpring_BunkerEntrance_Interior.nif",
    );
    if !src.exists() {
        eprintln!("skip: FO76 Whitespring bunker entrance fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load FO76 Whitespring bunker entrance");
    let source_blob = embedded_havok_blob(&source).expect("source collision blob");
    let source_hkx =
        havok_native::hkx::model::HkxFile::read(&source_blob).expect("parse source collision");
    let source_shape = source_hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpCompressedMeshShape")
        .expect("source compressed mesh");
    assert_eq!(
        member_value(&source_shape.members, "flags").and_then(hkx_int),
        Some(4)
    );
    assert_eq!(
        member_value(&source_shape.members, "dispatchType").and_then(hkx_int),
        Some(3)
    );
    let source_shape_key_bits =
        member_value(&source_shape.members, "numShapeKeyBits").and_then(hkx_int);

    let dir = temp_dir("convert_fo76_whitespring_bunker_compressed_mesh_header");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir
        .join("out")
        .join("WhiteSpring_BunkerEntrance_Interior.nif");
    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert Whitespring bunker entrance");
    assert!(report.supported, "{:?}", report.errors);

    let converted = NifFile::load(dst).expect("load converted bunker entrance");
    let converted_blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let converted_hkx = havok_native::hkx::model::HkxFile::read(&converted_blob)
        .expect("parse converted collision");
    let converted_shape = converted_hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpCompressedMeshShape")
        .expect("converted compressed mesh");
    assert_eq!(
        member_value(&converted_shape.members, "flags").and_then(hkx_int),
        Some(0x0204)
    );
    assert_eq!(
        member_value(&converted_shape.members, "dispatchType").and_then(hkx_int),
        Some(2)
    );
    assert_eq!(
        member_value(&converted_shape.members, "numShapeKeyBits").and_then(hkx_int),
        source_shape_key_bits
    );
}

#[test]
fn vanilla_fo4_capsule_layout_uses_target_radius_encoding() {
    let path = repo_path("extracted/fo4/Meshes/Ammo/44/44Ammo.nif");
    if !path.exists() {
        return;
    }
    let nif = NifFile::load(path).expect("load vanilla FO4 capsule fixture");
    let blob = embedded_havok_blob(&nif).expect("embedded Havok blob");
    let hkx = havok_native::hkx::model::HkxFile::read(&blob).expect("parse Havok blob");
    let capsule = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpCapsuleShape")
        .expect("capsule shape");
    assert_eq!(
        member_value(&capsule.members, "flags"),
        Some(&HkxValue::U16(451))
    );
    assert_eq!(
        member_value(&capsule.members, "dispatchType"),
        Some(&HkxValue::U8(1))
    );
    let vec4 = |name| match member_value(&capsule.members, name) {
        Some(HkxValue::F32List(values)) if values.len() >= 4 => values[3],
        other => panic!("missing {name}: {other:?}"),
    };
    assert!((vec4("a") - 1.0).abs() < 1e-6);
    assert!((vec4("b") - 1.0).abs() < 1e-6);
    assert_eq!(hkx_object_array(capsule, "vertices").len(), 8);
    assert_eq!(hkx_object_array(capsule, "planes").len(), 8);
    assert_eq!(hkx_object_array(capsule, "faces").len(), 6);
    assert_eq!(hkx_object_array(capsule, "indices").len(), 24);
}

#[test]
fn converts_real_fo76_flatwoods_projectile_without_new_animation_orphans() {
    let src = repo_path(
        "extracted/fo76/meshes/actors/flatwoodsmonster/characterassets/flatwoodsprojectile.nif",
    );
    if !src.exists() {
        eprintln!("skip: FO76 flatwoods projectile fixture not available");
        return;
    }
    let dir = temp_dir("convert_fo76_flatwoods_projectile_animation_refs");
    let dst = dir.join("flatwoodsprojectile.nif");
    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert flatwoods projectile");
    assert!(report.supported, "conversion failed: {:?}", report.errors);

    let converted = NifFile::load(dst).expect("load converted flatwoods projectile");
    let schema = NifSchema::from_generated();
    let lightning = converted
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSProceduralLightningController")
        .collect::<Vec<_>>();
    assert_eq!(lightning.len(), 3);
    for controller in lightning {
        for field in ["Interpolator 1", "Interpolator 9"] {
            let reference = controller
                .get_field(field)
                .map(NifValue::as_i64)
                .unwrap_or(-1);
            assert!(
                reference >= 0,
                "controller {} lost {field}",
                controller.block_id
            );
            let interpolator = &converted.blocks[reference as usize];
            assert!(
                schema.is_subtype_of(&interpolator.type_name, "NiInterpolator"),
                "controller {} {field} points to block {reference} {}",
                controller.block_id,
                interpolator.type_name
            );
        }
    }
    assert_all_blocks_reachable(&converted);
}

#[test]
fn converts_real_fo76_external_emittance_and_inline_texture_contracts() {
    let external_src = repo_path(
        "extracted/fo76/meshes/actors/atx/turret/characterassets/enclave/atx_enclaveturretdamage_fx.nif",
    );
    let texture_src = repo_path(
        "extracted/fo76/meshes/actors/character/facegendata/facegeom/seventysix.esm/00003101.nif",
    );
    let skeleton_src =
        repo_path("extracted/fo76/meshes/actors/antiairturret/characterassets/skeleton.nif");
    if !external_src.exists() || !texture_src.exists() || !skeleton_src.exists() {
        eprintln!("skip: FO76 shader contract fixtures not available");
        return;
    }
    let dir = temp_dir("convert_fo76_shader_contracts");
    let external_dst = dir.join("external.nif");
    let texture_dst = dir.join("textures.nif");
    let skeleton_dst = dir.join("skeleton.nif");
    for (src, dst) in [
        (&external_src, &external_dst),
        (&texture_src, &texture_dst),
        (&skeleton_src, &skeleton_dst),
    ] {
        let report = convert_nif_file(
            src,
            dst,
            "fo76",
            "fo4",
            None,
            &ConvertFileOptions::default(),
        )
        .expect("convert FO76 shader fixture");
        assert!(report.supported, "conversion failed: {:?}", report.errors);
    }

    let external = NifFile::load(external_dst).expect("load converted external-emittance NIF");
    let schema = NifSchema::from_generated();
    assert!(external.blocks.iter().any(|block| {
        schema.is_subtype_of(&block.type_name, "BSShaderProperty")
            && block
                .get_field("Shader Flags 1")
                .map(NifValue::as_i64)
                .unwrap_or_default()
                & (1 << 29)
                != 0
    }));
    let bsx = external
        .blocks
        .iter()
        .find(|block| block.type_name == "BSXFlags")
        .expect("external-emittance NIF has BSXFlags");
    assert_ne!(
        bsx.get_field("Integer Data")
            .map(NifValue::as_i64)
            .unwrap_or_default()
            & 0x200,
        0,
        "External_Emittance requires the FO4 BSX External Emit bit"
    );

    let skeleton = NifFile::load(skeleton_dst).expect("load converted anti-air skeleton");
    let skeleton_bsx = skeleton
        .blocks
        .iter()
        .find(|block| block.type_name == "BSXFlags")
        .expect("skeleton has BSXFlags");
    assert_eq!(
        skeleton_bsx.get_field("Integer Data").map(NifValue::as_i64),
        Some(194),
        "FO4 target structure requires Havok | Dynamic | Articulated, not source Ragdoll"
    );

    let textures = NifFile::load(texture_dst).expect("load converted inline-texture NIF");
    for block in &textures.blocks {
        let fields: &[&str] = match block.type_name.as_str() {
            "BSShaderTextureSet" => &["Textures"],
            "BSEffectShaderProperty" => &[
                "Source Texture",
                "Grayscale Texture",
                "Greyscale Texture",
                "Env Map Texture",
                "Normal Texture",
                "Env Mask Texture",
            ],
            _ => &[],
        };
        for field in fields {
            for path in texture_strings(block.get_field(field)) {
                if path.is_empty() {
                    continue;
                }
                assert!(
                    path.to_ascii_lowercase().starts_with("textures\\"),
                    "block {} {} {field} retained non-FO4 texture path {path:?}",
                    block.block_id,
                    block.type_name
                );
            }
        }
    }
}

fn texture_strings(value: Option<&NifValue>) -> Vec<&str> {
    match value {
        Some(NifValue::String(value)) => vec![value.as_str()],
        Some(NifValue::Array(values)) => values
            .iter()
            .flat_map(|value| texture_strings(Some(value)))
            .collect(),
        _ => Vec::new(),
    }
}

#[test]
fn extracts_real_fo76_native_primitives_without_preview_reconstruction() {
    for (relative, expected) in [
        ("extracted/fo76/Meshes/ammo/50cal/50calball.nif", "sphere"),
        (
            "extracted/fo76/Meshes/weapons/huntingrifle/308casing.nif",
            "capsule",
        ),
        (
            "extracted/fo76/Meshes/babylon/zaxframemodular/zaxfloor_d_half.nif",
            "convex",
        ),
    ] {
        let path = repo_path(relative);
        if !path.exists() {
            continue;
        }
        let nif = NifFile::load(path).expect("load FO76 primitive fixture");
        let blob = embedded_havok_blob(&nif).expect("embedded Havok blob");
        let primitive = extract_direct_source_primitive_from_blob(&blob, 0)
            .expect("extract primitive")
            .expect("direct primitive");
        match (expected, primitive) {
            ("sphere", SourcePrimitiveShape::Sphere { center, radius }) => {
                assert_eq!(center, [0.0; 3]);
                assert!((radius - 0.0108109405).abs() < 1e-7);
            }
            ("capsule", SourcePrimitiveShape::Capsule(shape)) => {
                assert_eq!(shape.hull.vertices.len(), 8);
                assert_eq!(shape.hull.planes.len(), 6);
                assert_eq!(shape.hull.faces.len(), 6);
                assert_eq!(shape.hull.indices.len(), 24);
                assert!((shape.a[3] - 0.0067894207).abs() < 1e-7);
                assert!((shape.convex_radius - 0.0067215264).abs() < 1e-7);
            }
            ("convex", SourcePrimitiveShape::Convex(shape)) => {
                assert_eq!(shape.vertices.len(), 8);
                assert_eq!(shape.convex_radius, 0.0);
            }
            (expected, actual) => panic!("expected {expected}, got {actual:?}"),
        }
    }
}

#[test]
fn converts_real_fo76_direct_primitives_to_native_fo4_shapes() {
    for (name, relative, route, output_class) in [
        (
            "sphere",
            "extracted/fo76/Meshes/ammo/50cal/50calball.nif",
            "source-sphere=1",
            "hknpConvexShape",
        ),
        (
            "capsule",
            "extracted/fo76/Meshes/weapons/huntingrifle/308casing.nif",
            "source-capsule=1",
            "hknpCapsuleShape",
        ),
        (
            "convex",
            "extracted/fo76/Meshes/babylon/zaxframemodular/zaxfloor_d_half.nif",
            "source-convex=1",
            "hknpConvexShape",
        ),
    ] {
        let source_path = repo_path(relative);
        if !source_path.exists() {
            continue;
        }
        let source_nif = NifFile::load(&source_path).expect("load source primitive NIF");
        let source_blob = embedded_havok_blob(&source_nif).expect("source primitive blob");
        let source_shape = extract_direct_source_primitive_from_blob(&source_blob, 0)
            .expect("extract source primitive")
            .expect("direct source primitive");

        let dir = temp_dir(&format!("native_primitive_{name}"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let output_path = dir.join("converted.nif");
        let report = convert_nif_file(
            &source_path,
            &output_path,
            "fo76",
            "fo4",
            None,
            &ConvertFileOptions::default(),
        )
        .expect("convert native primitive NIF");
        assert!(report.supported, "{name}: {:?}", report.errors);
        assert!(
            report.changes.iter().any(|change| change.contains(route)),
            "{name}: expected {route} in {:?}; warnings={:?}",
            report.changes,
            report.warnings
        );

        let output_nif = NifFile::load(&output_path).expect("load converted primitive NIF");
        let output_blob = embedded_havok_blob(&output_nif).expect("output primitive blob");
        let output_hkx =
            havok_native::hkx::model::HkxFile::read(&output_blob).expect("parse output primitive");
        let output_summary: serde_json::Value = serde_json::from_str(
            &havok_native::api::havok_collision_summary(&output_blob)
                .expect("summarize output primitive"),
        )
        .expect("parse output primitive summary");
        let output_object = output_hkx
            .objects()
            .iter()
            .find(|object| object.class_name == output_class)
            .unwrap_or_else(|| panic!("{name}: missing {output_class}"));

        match source_shape {
            SourcePrimitiveShape::Sphere { radius, .. } => {
                assert_eq!(
                    member_value(&output_object.members, "convexRadius"),
                    Some(&HkxValue::F32(radius))
                );
                assert_eq!(
                    member_value(&output_object.members, "flags"),
                    Some(&HkxValue::U16(17))
                );
                let vertices = hkx_object_array(output_object, "vertices");
                assert_eq!(
                    vertices.len(),
                    4,
                    "single support point must be SIMD padded"
                );
                assert!(vertices.windows(2).all(|pair| pair[0] == pair[1]));
                let body = &output_summary["bodies"][0];
                assert_eq!(body["flags"].as_u64(), Some(128));
                assert_eq!(body["motion_id"].as_u64(), Some(0));
                assert!(body["inverse_mass"].as_f64().is_some_and(|mass| mass > 0.0));
            }
            SourcePrimitiveShape::Capsule(shape) => {
                assert_eq!(hkx_object_array(output_object, "vertices").len(), 8);
                assert_eq!(hkx_object_array(output_object, "planes").len(), 8);
                assert_eq!(hkx_object_array(output_object, "faces").len(), 6);
                assert_eq!(hkx_object_array(output_object, "indices").len(), 24);
                assert_eq!(
                    member_value(&output_object.members, "convexRadius"),
                    Some(&HkxValue::F32(shape.convex_radius))
                );
                for endpoint in ["a", "b"] {
                    let Some(HkxValue::F32List(values)) =
                        member_value(&output_object.members, endpoint)
                    else {
                        panic!("{name}: missing endpoint {endpoint}");
                    };
                    assert!((values[3] - 1.0).abs() < 1e-6);
                }
                let body = &output_summary["bodies"][0];
                assert_eq!(body["flags"].as_u64(), Some(128));
                assert_eq!(body["motion_id"].as_u64(), Some(0));
                assert!(body["inverse_mass"].as_f64().is_some_and(|mass| mass > 0.0));
            }
            SourcePrimitiveShape::Convex(shape) => {
                let output_vertices = hkx_object_array(output_object, "vertices");
                assert_eq!(output_vertices.len(), shape.vertices.len());
                assert_eq!(
                    output_vertices[0],
                    HkxValue::F32List(shape.vertices[0].to_vec())
                );
                assert!(member_value(&output_object.members, "faces").is_none());
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn converts_real_fo76_static_sphere_with_valid_support_and_material() {
    let source_path =
        repo_path("extracted/fo76/Meshes/atx/setdressing/atx_ballbuddies/ATX_Basketball01.nif");
    if !source_path.exists() {
        eprintln!("skip: reported FO76 basketball fixture not available");
        return;
    }

    let source_nif = NifFile::load(&source_path).expect("load source basketball NIF");
    let source_blob = embedded_havok_blob(&source_nif).expect("source basketball collision");
    let source_hkx =
        havok_native::hkx::model::HkxFile::read(&source_blob).expect("parse source collision");
    let expected_material_crc = source_hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpSphereShape")
        .and_then(|shape| member_value(&shape.members, "userData"))
        .and_then(hkx_int)
        .expect("source sphere material CRC");

    let dir = temp_dir("static_sphere_material");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let output_path = dir.join("converted.nif");
    let report = convert_nif_file(
        &source_path,
        &output_path,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert basketball NIF");
    assert!(report.supported, "{:?}", report.errors);
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("source-sphere=1")),
        "{:?}",
        report.changes
    );

    let output_nif = NifFile::load(&output_path).expect("load converted basketball NIF");
    let output_blob = embedded_havok_blob(&output_nif).expect("converted basketball collision");
    let output_hkx =
        havok_native::hkx::model::HkxFile::read(&output_blob).expect("parse converted collision");
    let output_sphere = output_hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpSphereShape")
        .expect("converted sphere");
    assert_eq!(
        member_value(&output_sphere.members, "userData").and_then(hkx_int),
        Some(expected_material_crc)
    );

    let vertices = hkx_object_array(output_sphere, "vertices");
    assert_eq!(vertices.len(), 4);
    assert!(vertices.windows(2).all(|pair| pair[0] == pair[1]));
    assert!(vertices.iter().all(|vertex| {
        matches!(vertex, HkxValue::F32List(values) if values.iter().all(|value| value.is_finite()))
    }));

    let _ = std::fs::remove_dir_all(dir);
}

fn collision_blobs_for_named_nodes(nif: &NifFile, name: &str) -> Vec<(Vec<u8>, usize)> {
    nif.blocks
        .iter()
        .filter(|block| {
            block.type_name == "NiNode"
                && matches!(block.get_field("Name"), Some(NifValue::String(value)) if value == name)
        })
        .filter_map(|node| {
            let collision_id = ref_usize(node.get_field("Collision Object"))?;
            let collision = nif.get_block(collision_id)?;
            let data_ref = ref_usize(collision.get_field("Data"))?;
            let body_id = ref_usize(collision.get_field("Body ID")).unwrap_or(0);
            let physics = nif.get_block(data_ref)?;
            let Some(NifValue::Struct(binary_data)) = physics.get_field("Binary Data") else {
                return None;
            };
            let Some(data) = binary_data.get("Data") else {
                return None;
            };
            let bytes = match data {
                NifValue::Bytes(bytes) => bytes.clone(),
                NifValue::Array(values) => {
                    values.iter().map(|value| value.as_i64() as u8).collect()
                }
                _ => Vec::new(),
            };
            (!bytes.is_empty()).then_some((bytes, body_id))
        })
        .collect()
}

fn valid_compressed_triangle_count(meshes: &[PreviewMesh]) -> usize {
    meshes
        .iter()
        .flat_map(|mesh| {
            mesh.triangles
                .iter()
                .filter(move |tri| tri.iter().all(|idx| (*idx as usize) < mesh.vertices.len()))
        })
        .count()
}

fn hkx_member<'a>(
    members: &'a [havok_native::hkx::model::HkxMember],
    name: &str,
) -> &'a havok_native::hkx::types::HkxValue {
    &members
        .iter()
        .find(|member| member.name == name)
        .unwrap_or_else(|| panic!("{name} missing"))
        .value
}

fn hkx_object_members<'a>(
    value: &'a havok_native::hkx::types::HkxValue,
    name: &str,
) -> &'a [havok_native::hkx::model::HkxMember] {
    match value {
        havok_native::hkx::types::HkxValue::Object(members)
        | havok_native::hkx::types::HkxValue::TypedObject { members, .. } => members,
        other => panic!("{name} must be an object, got {}", other.variant_name()),
    }
}

fn bitfield_words(members: &[havok_native::hkx::model::HkxMember], name: &str) -> Vec<u32> {
    let bitfield = hkx_object_members(hkx_member(members, name), name);
    let storage = hkx_object_members(hkx_member(bitfield, "storage"), "storage");
    let words = match hkx_member(storage, "words") {
        havok_native::hkx::types::HkxValue::Array(values) => values,
        other => panic!("{name}.storage.words must be an array, got {other:?}"),
    };
    words
        .iter()
        .map(|word| match word {
            havok_native::hkx::types::HkxValue::U32(value) => *value,
            havok_native::hkx::types::HkxValue::I32(value) => *value as u32,
            other => panic!("{name}.storage.words entry must be an int, got {other:?}"),
        })
        .collect()
}

fn assert_valid_compressed_mesh_blob(blob: &[u8], body_id: usize) {
    let raw = extract_raw_compressed_meshes_from_blob(blob, Some(body_id))
        .expect("converted raw compressed mesh data");
    assert!(!raw.is_empty(), "converted compressed mesh missing");
    assert!(
        raw.iter().all(|mesh| mesh.sections.iter().all(|section| {
            !section.primitive_bytes.is_empty() && !section.primitive_data_runs.is_empty()
        })),
        "converted compressed mesh has incomplete section topology"
    );

    let hkx = havok_native::hkx::model::HkxFile::read(blob).expect("parse converted physics blob");
    let shapes: Vec<_> = hkx
        .objects()
        .iter()
        .filter(|object| object.class_name == "hknpCompressedMeshShape")
        .collect();
    assert!(
        !shapes.is_empty(),
        "converted compressed mesh shape missing"
    );
    for shape in shapes {
        let _ = bitfield_words(&shape.members, "quadIsFlat");
        let _ = bitfield_words(&shape.members, "triangleIsInterior");
    }
}

fn assert_authored_compressed_mesh_preserved(
    source: &RawCompressedMeshData,
    converted: &RawCompressedMeshData,
) {
    assert_eq!(converted.user_data, source.user_data);
    assert_eq!(converted.edge_welding_map, source.edge_welding_map);
    assert_eq!(converted.triangle_is_interior, source.triangle_is_interior);
    assert_eq!(
        converted.primitive_stores_is_flat_convex,
        source.primitive_stores_is_flat_convex
    );
    assert_eq!(converted.object_aabb_min, source.object_aabb_min);
    assert_eq!(converted.object_aabb_max, source.object_aabb_max);
    assert_eq!(converted.num_primitive_keys, source.num_primitive_keys);
    assert_eq!(converted.bits_per_key, source.bits_per_key);
    assert_eq!(converted.max_key_value, source.max_key_value);
    assert_eq!(converted.master_tree_nodes, source.master_tree_nodes);
    assert_eq!(converted.sections, source.sections);
    assert_eq!(converted.shared_vertices, source.shared_vertices);
    assert_eq!(converted.materials, source.materials);
}

fn preview_aabb(meshes: &[PreviewMesh]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let mut seen = false;
    for vertex in meshes.iter().flat_map(|mesh| mesh.vertices.iter()) {
        seen = true;
        for axis in 0..3 {
            min[axis] = min[axis].min(vertex[axis]);
            max[axis] = max[axis].max(vertex[axis]);
        }
    }
    assert!(seen, "preview has no vertices");
    (min, max)
}

fn preview_mesh_extent(mesh: &PreviewMesh) -> [f32; 3] {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for vertex in &mesh.vertices {
        for axis in 0..3 {
            min[axis] = min[axis].min(vertex[axis]);
            max[axis] = max[axis].max(vertex[axis]);
        }
    }
    [max[0] - min[0], max[1] - min[1], max[2] - min[2]]
}

fn assert_same_preview_aabb(left: &[PreviewMesh], right: &[PreviewMesh], tolerance: f32) {
    let (left_min, left_max) = preview_aabb(left);
    let (right_min, right_max) = preview_aabb(right);
    for axis in 0..3 {
        assert!(
            (left_min[axis] - right_min[axis]).abs() <= tolerance,
            "min axis {axis} differs: {} vs {}",
            left_min[axis],
            right_min[axis]
        );
        assert!(
            (left_max[axis] - right_max[axis]).abs() <= tolerance,
            "max axis {axis} differs: {} vs {}",
            left_max[axis],
            right_max[axis]
        );
    }
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
fn convert_real_skyrim_rigid_body_t_statics_when_available() {
    let extracted =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/skyrimse/Meshes");
    let sources = [
        "landscape/roads/roadscurver01.nif",
        "clutter/exteriorwoodenfurniture/exteriorwoodentable01.nif",
        "dungeons/imperial/exterior/impextstairs02.nif",
        "clutter/stockade/stockadestairs01.nif",
    ]
    .map(|relative| extracted.join(relative));
    if sources.iter().any(|source| !source.is_file()) {
        eprintln!("skip: reported Skyrim collision fixtures are unavailable");
        return;
    }

    for source in sources {
        let dir = temp_dir("convert_real_skyrim_rigid_body_t_static");
        let destination = dir.join("data/Meshes/converted.nif");
        let materials = dir.join("data/Materials/Skyrim");
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let report = convert_nif_file(
            &source,
            &destination,
            "skyrimse",
            "fo4",
            Some(&materials),
            &ConvertFileOptions {
                asset_prefix: Some("Skyrim".to_string()),
                ..ConvertFileOptions::default()
            },
        )
        .expect("convert Skyrim rigid-body-T static");
        assert!(
            report.supported,
            "{}: {:?}",
            source.display(),
            report.errors
        );
        assert!(
            report.errors.is_empty(),
            "{}: {:?}",
            source.display(),
            report.errors
        );

        let converted = NifFile::load(destination).expect("load converted Skyrim static");
        assert!(
            converted
                .blocks
                .iter()
                .any(|block| block.type_name == "bhkNPCollisionObject"),
            "{} lost its FO4 collision object",
            source.display()
        );
        assert!(
            converted
                .blocks
                .iter()
                .any(|block| block.type_name == "bhkPhysicsSystem"),
            "{} lost its FO4 physics data",
            source.display()
        );
        assert!(converted.blocks.iter().all(|block| !matches!(
            block.type_name.as_str(),
            "bhkCollisionObject" | "bhkRigidBody" | "bhkRigidBodyT"
        )));
        let bsx = converted
            .blocks
            .iter()
            .find(|block| block.type_name == "BSXFlags")
            .expect("converted static BSXFlags");
        assert_ne!(
            bsx.get_field("Integer Data")
                .map(NifValue::as_i64)
                .unwrap_or(0)
                & 2,
            0
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn convert_real_skyrim_whiterun_gate_preserves_animstatic_leaf_collision_when_available() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/skyrimse/Meshes/architecture/whiterun/wrdoormaingate01.nif");
    if !source.is_file() {
        eprintln!("skip: Whiterun gate collision fixture is unavailable");
        return;
    }

    let dir = temp_dir("convert_real_skyrim_whiterun_gate");
    let destination = dir.join("data/Meshes/converted.nif");
    let materials = dir.join("data/Materials/Skyrim");
    std::fs::create_dir_all(&dir).expect("create temp dir");

    let report = convert_nif_file(
        &source,
        &destination,
        "skyrimse",
        "fo4",
        Some(&materials),
        &ConvertFileOptions {
            asset_prefix: Some("Skyrim".to_string()),
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert Whiterun gate");
    assert!(report.supported, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let converted = NifFile::load(destination).expect("load converted Whiterun gate");
    assert_eq!(
        converted.find_blocks("bhkNPCollisionObject").len(),
        3,
        "housing and both animated leaves need collision objects; changes={:?}; warnings={:?}",
        report.changes,
        report.warnings
    );
    let blobs = embedded_havok_blobs(&converted);
    assert_eq!(
        blobs.len(),
        3,
        "each gate collision owner needs physics data"
    );

    let mut static_bodies = 0;
    let mut animstatic_bodies = 0;
    for blob in &blobs {
        let summary: serde_json::Value = serde_json::from_str(
            &havok_native::api::havok_collision_summary(blob).expect("collision summary"),
        )
        .expect("collision summary JSON");
        let layer = summary["bodies"][0]["layer"]
            .as_u64()
            .expect("collision layer");
        let (motion_ids, motion_cinfo_count) = physics_motion_ids(blob);
        match layer {
            1 => {
                static_bodies += 1;
                assert_eq!(motion_cinfo_count, 0);
                assert_eq!(motion_ids, vec![0x7FFF_FFFF]);
            }
            2 => {
                animstatic_bodies += 1;
                assert_eq!(motion_cinfo_count, 1);
                assert_eq!(motion_ids, vec![0]);
            }
            other => panic!("unexpected converted gate collision layer {other}"),
        }
    }
    assert_eq!(static_bodies, 1, "gate housing should remain static");
    assert_eq!(
        animstatic_bodies, 2,
        "both gate leaves should remain animated"
    );

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
fn convert_skyrim_dwemer_rigid_animation_fixture_when_available() {
    let src = repo_path(
        "extracted/skyrimse/Meshes/Dungeons/Dwemer/Pipes/DwePipeGearAssemblyExtraCCW01.nif",
    );
    if !src.exists() {
        eprintln!("skip: Skyrim Dwemer animation fixture is unavailable");
        return;
    }
    let dir = temp_dir("convert_skyrim_dwemer_rigid_animation");
    let materials = dir.join("Materials").join("Skyrim");
    std::fs::create_dir_all(&materials).expect("create material output");
    let dst = dir.join("converted.nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "skyrimse",
        "fo4",
        Some(&materials),
        &ConvertFileOptions::default(),
    )
    .expect("convert Dwemer animated static");

    assert!(report.supported, "errors: {:?}", report.errors);
    let converted = NifFile::load(dst).expect("load converted Dwemer animation");
    assert_eq!(converted.header.bs_version, 130);
    for block_type in [
        "NiTransformController",
        "NiTransformInterpolator",
        "NiTransformData",
    ] {
        assert!(
            converted
                .blocks
                .iter()
                .any(|block| block.type_name == block_type),
            "missing {block_type}"
        );
    }
    let transform_data = converted
        .blocks
        .iter()
        .find(|block| block.type_name == "NiTransformData")
        .expect("preserved transform data");
    assert_eq!(
        transform_data
            .get_field("Num Rotation Keys")
            .map(NifValue::as_i64),
        Some(1)
    );
    let NifValue::Array(rotations) = transform_data
        .get_field("XYZ Rotations")
        .expect("preserved XYZ rotations")
    else {
        panic!("XYZ rotations are not an array");
    };
    assert_eq!(rotations.len(), 3);
    let NifValue::Struct(y_rotation) = &rotations[1] else {
        panic!("Y rotation is not a key group");
    };
    assert_eq!(y_rotation.get("Num Keys").map(NifValue::as_i64), Some(3));
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
    assert!(report.errors[0].contains("excludes dynamic/skinned block"));
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
fn convert_fo76_to_fo4_clears_marker_flags_from_non_marker_scene_nodes() {
    let dir = temp_dir("convert_fo76_scene_node_flags");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = NifFile::new("fo76");
    nif.blocks[0].set_field("Name", NifValue::String("ToxicStone01".to_string()));
    nif.blocks[0].set_field("Flags", NifValue::UInt(0x2000_500E));

    let child_id = nif.add_block(
        "NiNode",
        Some(IndexMap::from([
            (
                "Name".to_string(),
                NifValue::String("L1_ToxicStone01".to_string()),
            ),
            ("Flags".to_string(), NifValue::UInt(0x2000_000E)),
        ])),
    );
    nif.blocks[0].set_field("Num Children", NifValue::UInt(1));
    nif.blocks[0].set_field(
        "Children",
        NifValue::Array(vec![NifValue::Ref(child_id as i32)]),
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

    assert!(report.supported);
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("Normalized FO76 scene node flags for FO4"))
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    assert_eq!(
        converted.blocks[0]
            .get_field("Flags")
            .and_then(|value| match value {
                NifValue::UInt(flags) => Some(flags),
                _ => None,
            })
            .copied(),
        Some(0x500E)
    );
    assert_eq!(
        converted.blocks[child_id]
            .get_field("Flags")
            .and_then(|value| match value {
                NifValue::UInt(flags) => Some(flags),
                _ => None,
            })
            .copied(),
        Some(0x000E)
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_vault76_stairs_preserves_root_compressed_collision() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/hardscape/unique/hard_unique_vault76_stairslg01.nif");
    if !src.exists() {
        return;
    }

    let source = NifFile::load(&src).expect("load source nif");
    let (source_blob, source_body_id) =
        np_collision_blob_for_target(&source, 0).expect("source root collision");
    let source_preview =
        extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(source_body_id))
            .expect("source root preview");
    let source_raw_triangles = source_preview
        .iter()
        .map(|mesh| mesh.triangles.len())
        .sum::<usize>();
    let source_triangles = valid_compressed_triangle_count(&source_preview);
    assert!(source_triangles > 128);
    assert_eq!(source_triangles, source_raw_triangles);
    assert!(
        source_preview
            .iter()
            .all(|mesh| mesh.shape_type == "compressed_mesh")
    );
    assert!(!source_preview.is_empty());

    let dir = temp_dir("convert_fo76_vault76_stairs");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("Hard_Unique_Vault76_StairsLG01.nif");
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

    let converted = NifFile::load(&dst).expect("load converted nif");
    let (converted_blob, converted_body_id) = np_collision_blob_for_target(&converted, 0)
        .unwrap_or_else(|| {
            panic!(
                "converted root collision; changes={:?}; warnings={:?}",
                report.changes, report.warnings
            )
        });
    let converted_preview =
        extract_preview_meshes_from_blob(&converted_blob, 69.99125, Some(converted_body_id))
            .expect("converted root preview");
    let converted_triangles = converted_preview
        .iter()
        .map(|mesh| mesh.triangles.len())
        .sum::<usize>();
    assert_eq!(converted_triangles, source_triangles);
    assert_valid_compressed_mesh_blob(&converted_blob, converted_body_id);
    assert!(
        converted_preview
            .iter()
            .all(|mesh| mesh.shape_type == "compressed_mesh")
    );
    assert_same_preview_aabb(&source_preview, &converted_preview, 0.25);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_workshop_trigger_preserves_source_body_flags() {
    let upper_src = repo_path(
        "extracted/fo76/Meshes/Workshop/WorkshopCapturePointBorderCylinderHalf512Trigger.nif",
    );
    let lower_src = repo_path(
        "extracted/fo76/meshes/workshop/WorkshopCapturePointBorderCylinderHalf512Trigger.nif",
    );
    let src = if upper_src.exists() {
        upper_src
    } else if lower_src.exists() {
        lower_src
    } else {
        return;
    };

    let source = NifFile::load(&src).expect("load source nif");
    let source_blob = embedded_havok_blob(&source).expect("source collision blob");
    let source_summary_json =
        havok_native::api::havok_collision_summary(&source_blob).expect("source summary");
    let source_summary: serde_json::Value =
        serde_json::from_str(&source_summary_json).expect("source summary json");
    let source_body = source_summary
        .get("bodies")
        .and_then(serde_json::Value::as_array)
        .and_then(|bodies| bodies.first())
        .expect("source body summary");
    assert_eq!(
        source_body.get("layer").and_then(serde_json::Value::as_i64),
        Some(12)
    );
    assert_eq!(
        source_body
            .get("shape_class")
            .and_then(serde_json::Value::as_str),
        Some("hknpConvexPolytopeShape")
    );
    assert_eq!(
        source_body.get("flags").and_then(serde_json::Value::as_i64),
        Some(16)
    );

    let dir = temp_dir("convert_fo76_workshop_trigger_flags");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir
        .join("out")
        .join("WorkshopCapturePointBorderCylinderHalf512Trigger.nif");
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

    let converted = NifFile::load(&dst).expect("load converted nif");
    let converted_blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let converted_summary_json =
        havok_native::api::havok_collision_summary(&converted_blob).expect("converted summary");
    let converted_summary: serde_json::Value =
        serde_json::from_str(&converted_summary_json).expect("converted summary json");
    let converted_body = converted_summary
        .get("bodies")
        .and_then(serde_json::Value::as_array)
        .and_then(|bodies| bodies.first())
        .expect("converted body summary");

    assert_eq!(
        converted_body
            .get("collision_filter_info")
            .and_then(serde_json::Value::as_i64),
        Some(12)
    );
    assert_eq!(
        converted_body
            .get("shape_class")
            .and_then(serde_json::Value::as_str),
        Some("hknpConvexPolytopeShape")
    );
    assert_eq!(
        converted_body
            .get("flags")
            .and_then(serde_json::Value::as_i64),
        Some(16)
    );
    let (motion_ids, motion_cinfo_count) = physics_motion_ids(&converted_blob);
    assert_eq!(motion_ids, vec![0x7FFF_FFFF]);
    assert_eq!(motion_cinfo_count, 0);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_vault76_stairslg04_uses_shared_multi_body_collision() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/hardscape/unique/hard_unique_vault76_stairslg04.nif");
    if !src.exists() {
        return;
    }

    let dir = temp_dir("convert_fo76_vault76_stairslg04");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("Hard_Unique_Vault76_StairsLG04.nif");
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
        !report
            .warnings
            .iter()
            .any(|warning| warning.contains("separate FO4 physics system")),
        "LG04 must not fall back to per-body physics systems: {:?}",
        report.warnings
    );

    let converted = NifFile::load(&dst).expect("load converted nif");
    let physics_ids = converted
        .blocks
        .iter()
        .filter(|block| block.type_name == "bhkPhysicsSystem")
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    assert_eq!(
        physics_ids.len(),
        1,
        "LG04 should have one shared bhkPhysicsSystem"
    );

    let mut body_ids = Vec::new();
    let mut collision_count = 0usize;
    for block in converted
        .blocks
        .iter()
        .filter(|block| block.type_name == "bhkNPCollisionObject")
    {
        let data_ref = match block.get_field("Data") {
            Some(NifValue::Ref(value)) if *value >= 0 => *value as usize,
            Some(NifValue::Int(value)) if *value >= 0 => *value as usize,
            Some(NifValue::UInt(value)) => *value as usize,
            _ => panic!("collision block {} has no Data ref", block.block_id),
        };
        assert_eq!(
            data_ref, physics_ids[0],
            "collision block {} should point at the shared physics system",
            block.block_id
        );
        let body_id = match block.get_field("Body ID") {
            Some(NifValue::Int(value)) if *value >= 0 => *value as usize,
            Some(NifValue::UInt(value)) => *value as usize,
            Some(NifValue::Ref(value)) if *value >= 0 => *value as usize,
            _ => panic!("collision block {} has no Body ID", block.block_id),
        };
        body_ids.push(body_id);
        collision_count += 1;
    }
    body_ids.sort_unstable();
    assert_eq!(
        collision_count, 9,
        "LG04 should keep all nine collision objects"
    );
    assert_eq!(body_ids, (0usize..9).collect::<Vec<_>>());

    let blobs = embedded_havok_blobs(&converted);
    assert_eq!(blobs.len(), 1, "LG04 should embed one shared physics blob");
    let blob = &blobs[0];
    let summary_json = havok_native::api::havok_collision_summary(&blob).expect("summary");
    let summary: serde_json::Value = serde_json::from_str(&summary_json).expect("summary JSON");
    assert_eq!(
        summary
            .get("bodies")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(9),
        "shared physics blob should expose nine bodies"
    );
    let (motion_ids, motion_cinfo_count) = physics_motion_ids(blob);
    assert_eq!(
        motion_cinfo_count, 9,
        "LG04 shared compressed-mesh collision must emit one motionCinfo per body"
    );
    assert_eq!(
        motion_ids,
        (0i64..9).collect::<Vec<_>>(),
        "LG04 shared compressed-mesh bodies must all use valid sequential motionIds"
    );
    for body_id in 0..9 {
        let preview = extract_preview_meshes_from_blob(&blob, 69.99125, Some(body_id))
            .unwrap_or_else(|error| panic!("body {body_id} preview failed: {error}"));
        assert!(
            !preview.is_empty(),
            "shared physics body {body_id} should be previewable"
        );
    }

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_siding_interior_stairs_uses_shared_mixed_collision() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../../extracted/fo76/meshes/Architecture/BLDKIT/Siding/Interior/BLD_Siding_Interior_Stairs_01.nif",
    );
    if !src.exists() {
        return;
    }

    let dir = temp_dir("convert_fo76_siding_interior_stairs");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("BLD_Siding_Interior_Stairs_01.nif");
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
        !report
            .warnings
            .iter()
            .any(|warning| warning.contains("separate FO4 physics system")),
        "mixed stairs collision must not fall back to per-body physics systems: {:?}",
        report.warnings
    );

    let converted = NifFile::load(&dst).expect("load converted nif");
    let physics_ids = converted
        .blocks
        .iter()
        .filter(|block| block.type_name == "bhkPhysicsSystem")
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    assert_eq!(
        physics_ids.len(),
        1,
        "mixed stairs should have one shared bhkPhysicsSystem"
    );

    let blobs = embedded_havok_blobs(&converted);
    assert_eq!(
        blobs.len(),
        1,
        "mixed stairs should embed one shared physics blob"
    );
    let blob = &blobs[0];
    let summary_json = havok_native::api::havok_collision_summary(&blob).expect("summary");
    let summary: serde_json::Value = serde_json::from_str(&summary_json).expect("summary JSON");
    let bodies = summary
        .get("bodies")
        .and_then(serde_json::Value::as_array)
        .expect("bodies");
    assert_eq!(
        bodies.len(),
        3,
        "shared physics blob should expose three bodies"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_scol_cm005627d3_preserves_compressed_collision() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/scol/seventysix.esm/cm005627d3.nif");
    if !src.exists() {
        return;
    }

    let source = NifFile::load(&src).expect("load source nif");
    let (source_blob, source_body_id) =
        np_collision_blob_for_target(&source, 11).expect("source physics-node collision");
    let source_preview =
        extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(source_body_id))
            .expect("source collision preview");
    let source_triangles = valid_compressed_triangle_count(&source_preview);
    assert_eq!(source_triangles, 552);
    assert!(!source_preview.is_empty());

    let dir = temp_dir("convert_fo76_scol_cm005627d3");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("cm005627d3.nif");
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

    let converted = NifFile::load(&dst).expect("load converted nif");
    let (converted_blob, converted_body_id) = np_collision_blob_for_target(&converted, 11)
        .unwrap_or_else(|| {
            panic!(
                "converted physics-node collision; changes={:?}; warnings={:?}",
                report.changes, report.warnings
            )
        });
    let converted_preview =
        extract_preview_meshes_from_blob(&converted_blob, 69.99125, Some(converted_body_id))
            .expect("converted collision preview");
    assert_valid_compressed_mesh_blob(&converted_blob, converted_body_id);
    assert_eq!(
        valid_compressed_triangle_count(&converted_preview),
        source_triangles
    );
    assert_same_preview_aabb(&source_preview, &converted_preview, 0.25);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_scol_cm00013b0b_rebuilds_compressed_collision() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/scol/seventysix.esm/cm00013b0b.nif");
    if !src.exists() {
        return;
    }

    let source = NifFile::load(&src).expect("load source nif");
    let (source_blob, source_body_id) =
        np_collision_blob_for_target(&source, 7).expect("source physics-node collision");
    let source_preview =
        extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(source_body_id))
            .expect("source collision preview");
    let source_triangles = valid_compressed_triangle_count(&source_preview);
    assert!(source_triangles > 0);
    assert!(!source_preview.is_empty());

    let dir = temp_dir("convert_fo76_scol_cm00013b0b");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("cm00013b0b.nif");
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

    let converted = NifFile::load(&dst).expect("load converted nif");
    let (converted_blob, converted_body_id) = np_collision_blob_for_target(&converted, 7)
        .unwrap_or_else(|| {
            panic!(
                "converted physics-node collision; changes={:?}; warnings={:?}",
                report.changes, report.warnings
            )
        });
    let converted_preview =
        extract_preview_meshes_from_blob(&converted_blob, 69.99125, Some(converted_body_id))
            .expect("converted collision preview");
    assert_valid_compressed_mesh_blob(&converted_blob, converted_body_id);
    assert_eq!(
        valid_compressed_triangle_count(&converted_preview),
        source_triangles
    );
    assert_same_preview_aabb(&source_preview, &converted_preview, 0.25);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_scol_cm002a74bc_decodes_full_custom_flat_convex_coverage() {
    // CUSTOM flat-convex shared-vertex decode: the record ref `m_indices[0]` is in
    // section vertex-index space, so it must be offset by the packed-vertex count
    // before indexing `sharedVerticesIndex` (like the triangle path). Without the
    // offset, 16 of this body's 28 custom primitives fall out of bounds and 12
    // read the wrong record, leaving ~half the SCOL hull uncollided: 308 decoded
    // triangles instead of ~676. The assertion sits well above 308 because the
    // exact count shifts with hull triangulation.
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/scol/seventysix.esm/cm002a74bc.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted CM002A74BC fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load source nif");
    let (source_blob, source_body_id) =
        np_collision_blob_for_target(&source, 30).expect("source physics-node collision");
    let source_preview =
        extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(source_body_id))
            .expect("source collision preview");
    let source_triangles = valid_compressed_triangle_count(&source_preview);
    assert!(
        source_triangles >= 550,
        "custom flat-convex coverage regressed: {source_triangles} triangles (buggy floor was 308)"
    );

    let dir = temp_dir("convert_fo76_scol_cm002a74bc");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("cm002a74bc.nif");
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

    let converted = NifFile::load(&dst).expect("load converted nif");
    let (converted_blob, converted_body_id) = np_collision_blob_for_target(&converted, 30)
        .unwrap_or_else(|| {
            panic!(
                "converted physics-node collision; changes={:?}; warnings={:?}",
                report.changes, report.warnings
            )
        });
    let converted_preview =
        extract_preview_meshes_from_blob(&converted_blob, 69.99125, Some(converted_body_id))
            .expect("converted collision preview");
    assert_valid_compressed_mesh_blob(&converted_blob, converted_body_id);
    // The FO4 re-encode round-trips this hull with a small triangle delta, so
    // assert the converted collision is likewise well above the buggy floor (the
    // fix has to survive the decode→re-encode path, not just the raw decode) and
    // that its spatial coverage still matches the source AABB.
    let converted_triangles = valid_compressed_triangle_count(&converted_preview);
    assert!(
        converted_triangles >= 550,
        "converted custom flat-convex coverage regressed: {converted_triangles} triangles"
    );
    assert_same_preview_aabb(&source_preview, &converted_preview, 0.25);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_scol_cm004521e1_preserves_authored_compressed_mesh_topology() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/scol/seventysix.esm/cm004521e1.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted CM004521E1 fixture not available");
        return;
    }

    const MAIN_NODE: &str = "004521E1_PhysicsMerged_L01";
    let source = NifFile::load(&src).expect("load source nif");
    let source_collisions = collision_blobs_for_named_nodes(&source, MAIN_NODE);
    assert_eq!(source_collisions.len(), 1, "source main collision body");
    let (source_blob, source_body_id) = &source_collisions[0];
    let source_raw = extract_raw_compressed_meshes_from_blob(source_blob, Some(*source_body_id))
        .expect("source raw compressed mesh")
        .into_iter()
        .next()
        .expect("source main compressed mesh");
    assert_eq!(source_raw.primitive_stores_is_flat_convex, u8::MAX);
    assert_eq!(source_raw.sections.len(), 3);
    assert_eq!(
        source_raw.edge_welding_map.value_and_secondary_keys.len(),
        74
    );
    assert!(
        source_raw
            .triangle_is_interior
            .words
            .iter()
            .any(|word| *word != 0)
    );
    assert_eq!(source_raw.materials.len(), 7);

    let dir = temp_dir("convert_fo76_scol_cm004521e1");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("cm004521e1.nif");
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

    let converted = NifFile::load(&dst).expect("load converted nif");
    let converted_collisions = collision_blobs_for_named_nodes(&converted, MAIN_NODE);
    assert_eq!(
        converted_collisions.len(),
        1,
        "converted main collision body; changes={:?}; warnings={:?}",
        report.changes,
        report.warnings
    );
    let (converted_blob, converted_body_id) = &converted_collisions[0];
    assert_valid_compressed_mesh_blob(converted_blob, *converted_body_id);
    let converted_raw =
        extract_raw_compressed_meshes_from_blob(converted_blob, Some(*converted_body_id))
            .expect("converted raw compressed mesh")
            .into_iter()
            .next()
            .expect("converted main compressed mesh");

    assert_authored_compressed_mesh_preserved(&source_raw, &converted_raw);

    let source_preview =
        extract_preview_meshes_from_blob(source_blob, 69.99125, Some(*source_body_id))
            .expect("source main preview");
    let converted_preview =
        extract_preview_meshes_from_blob(converted_blob, 69.99125, Some(*converted_body_id))
            .expect("converted main preview");
    assert_eq!(
        valid_compressed_triangle_count(&converted_preview),
        valid_compressed_triangle_count(&source_preview)
    );
    assert_same_preview_aabb(&source_preview, &converted_preview, 0.001);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_scol_cm004521e2_preserves_authored_compressed_mesh_topology() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/scol/seventysix.esm/cm004521e2.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted CM004521E2 fixture not available");
        return;
    }

    const MAIN_NODE: &str = "004521E2_PhysicsMerged_L01";
    let source = NifFile::load(&src).expect("load source nif");
    let source_collisions = collision_blobs_for_named_nodes(&source, MAIN_NODE);
    assert_eq!(source_collisions.len(), 1, "source main collision body");
    let (source_blob, source_body_id) = &source_collisions[0];
    let source_raw = extract_raw_compressed_meshes_from_blob(source_blob, Some(*source_body_id))
        .expect("source raw compressed mesh")
        .into_iter()
        .next()
        .expect("source main compressed mesh");

    let dir = temp_dir("convert_fo76_scol_cm004521e2");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("cm004521e2.nif");
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

    let converted = NifFile::load(&dst).expect("load converted nif");
    let converted_collisions = collision_blobs_for_named_nodes(&converted, MAIN_NODE);
    assert_eq!(
        converted_collisions.len(),
        1,
        "converted main collision body; changes={:?}; warnings={:?}",
        report.changes,
        report.warnings
    );
    let (converted_blob, converted_body_id) = &converted_collisions[0];
    assert_valid_compressed_mesh_blob(converted_blob, *converted_body_id);
    let converted_raw =
        extract_raw_compressed_meshes_from_blob(converted_blob, Some(*converted_body_id))
            .expect("converted raw compressed mesh")
            .into_iter()
            .next()
            .expect("converted main compressed mesh");
    assert_authored_compressed_mesh_preserved(&source_raw, &converted_raw);

    let source_preview =
        extract_preview_meshes_from_blob(source_blob, 69.99125, Some(*source_body_id))
            .expect("source main preview");
    let converted_preview =
        extract_preview_meshes_from_blob(converted_blob, 69.99125, Some(*converted_body_id))
            .expect("converted main preview");
    assert_eq!(
        valid_compressed_triangle_count(&converted_preview),
        valid_compressed_triangle_count(&source_preview)
    );
    assert_same_preview_aabb(&source_preview, &converted_preview, 0.001);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_scol_cm00844bcb_preserves_stairhelper_convex_collision() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/scol/seventysix.esm/cm00844bcb.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted CM00844BCB fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load source nif");
    let source_helpers = collision_blobs_for_named_nodes(&source, "STAIRHELPER");
    assert_eq!(source_helpers.len(), 2, "source STAIRHELPER nodes");
    let source_preview =
        extract_preview_meshes_from_blob(&source_helpers[0].0, 69.99125, Some(source_helpers[0].1))
            .expect("source STAIRHELPER preview");
    assert_eq!(source_preview.len(), 1, "{source_preview:?}");
    assert_eq!(source_preview[0].shape_type, "convex_hull");
    let (source_min, source_max) = preview_aabb(&source_preview);
    assert!(
        source_max
            .iter()
            .zip(source_min.iter())
            .any(|(max, min)| max - min > 100.0),
        "source STAIRHELPER should not be a tiny fallback: {source_preview:?}"
    );

    let dir = temp_dir("convert_fo76_scol_cm00844bcb_stairhelper");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("cm00844bcb.nif");
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
        report.warnings.iter().all(|warning| {
            !(warning.contains("STAIRHELPER")
                && (warning.contains("source collision unavailable")
                    || warning.contains("minimal AABB fallback")))
        }),
        "{:?}",
        report.warnings
    );

    let converted = NifFile::load(&dst).expect("load converted nif");
    let converted_helpers = collision_blobs_for_named_nodes(&converted, "STAIRHELPER");
    assert_eq!(
        converted_helpers.len(),
        2,
        "converted STAIRHELPER nodes; changes={:?}; warnings={:?}",
        report.changes,
        report.warnings
    );
    for (blob, body_id) in converted_helpers {
        let converted_preview = extract_preview_meshes_from_blob(&blob, 69.99125, Some(body_id))
            .expect("converted STAIRHELPER preview");
        assert_same_preview_aabb(&source_preview, &converted_preview, 0.25);
    }

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_whitespring_trashcan_preserves_collision_aabb() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/furniture/whitespring/whitespring_trashcan_01.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted whitespring_trashcan_01 fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load source nif");
    let source_collisions = collision_blobs_for_named_nodes(&source, "WhiteSpring_Trashcan_01");
    assert_eq!(source_collisions.len(), 1, "source trashcan collision");
    let source_preview = extract_preview_meshes_from_blob(
        &source_collisions[0].0,
        69.99125,
        Some(source_collisions[0].1),
    )
    .expect("source collision preview");
    let (source_min, source_max) = preview_aabb(&source_preview);
    assert!(
        source_min[2] > -1.0 && source_max[2] > 89.0,
        "trashcan source collision must be decoded in body space, got min={source_min:?} max={source_max:?}"
    );

    let dir = temp_dir("convert_fo76_whitespring_trashcan_collision_aabb");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("WhiteSpring_Trashcan_01.nif");
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

    let converted = NifFile::load(&dst).expect("load converted nif");
    let converted_collisions =
        collision_blobs_for_named_nodes(&converted, "WhiteSpring_Trashcan_01");
    assert_eq!(
        converted_collisions.len(),
        1,
        "converted trashcan collision; changes={:?}; warnings={:?}",
        report.changes,
        report.warnings
    );
    let converted_preview = extract_preview_meshes_from_blob(
        &converted_collisions[0].0,
        69.99125,
        Some(converted_collisions[0].1),
    )
    .expect("converted collision preview");
    assert_same_preview_aabb(&source_preview, &converted_preview, 0.25);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fnv_geometry_shader_and_collision_in_rust() {
    let dir = temp_dir("convert_legacy");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/nif/fnv/weapons/gaussrifle.nif");
    assert!(src.exists(), "missing fixture {}", src.display());

    let report = convert_nif_file(
        &src,
        &dst,
        "fnv",
        "fo4",
        None,
        &ConvertFileOptions {
            asset_prefix: Some("fnv".to_string()),
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert nif");

    assert!(report.supported);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("NiTriStrips -> BSTriShape")),
        "{:?}",
        report.changes
    );
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("Legacy shader properties")),
        "{:?}",
        report.changes
    );
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("Legacy collision")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    assert!(
        converted
            .blocks
            .iter()
            .any(|block| block.type_name == "BSTriShape")
    );
    assert!(
        has_nonzero_vertex_data(&converted),
        "converted BSTriShape vertex positions were all zero"
    );
    assert!(
        converted
            .blocks
            .iter()
            .any(|block| block.type_name == "BSLightingShaderProperty")
    );
    assert!(
        converted
            .blocks
            .iter()
            .any(|block| block.type_name == "bhkNPCollisionObject")
    );
    assert!(
        converted
            .blocks
            .iter()
            .any(|block| block.type_name == "bhkPhysicsSystem")
    );
    assert!(converted.blocks.iter().all(|block| !matches!(
        block.type_name.as_str(),
        "bhkCollisionObject" | "bhkRigidBody" | "bhkRigidBodyT" | "bhkBoxShape"
    )));
    let collision_blob = embedded_havok_blob(&converted).expect("FO4 collision blob");
    let collision_summary =
        havok_native::api::havok_collision_summary(&collision_blob).expect("collision summary");
    assert!(!collision_summary.contains("\"n_vertices\":0"));
    assert!(
        converted
            .blocks
            .iter()
            .all(|block| block.type_name != "NiTriStrips"
                && block.type_name != "NiTriStripsData"
                && block.type_name != "BSShaderPPLightingProperty"
                && block.type_name != "NiMaterialProperty")
    );

    let texset = converted
        .blocks
        .iter()
        .find(|block| block.type_name == "BSShaderTextureSet")
        .expect("texture set");
    let textures = match texset.get_field("Textures") {
        Some(NifValue::Array(textures)) => textures,
        other => panic!("expected texture array, got {other:?}"),
    };
    let first_texture = textures
        .iter()
        .find_map(|value| match value {
            NifValue::String(path) if !path.is_empty() => Some(path),
            _ => None,
        })
        .expect("non-empty texture path");
    assert!(
        first_texture.to_ascii_lowercase().starts_with("textures\\"),
        "{first_texture}"
    );
    assert!(
        !first_texture
            .to_ascii_lowercase()
            .starts_with("textures\\fnv\\"),
        "{first_texture}"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fnv_swamp_gas_particle_fixture_to_fo4_layout_when_available() {
    let src = repo_root().join("extracted/fnv/Meshes/effects/ambient/fxswampgas01.nif");
    if !src.exists() {
        eprintln!("skipping missing fixture {}", src.display());
        return;
    }
    let dir = temp_dir("convert_fnv_swamp_gas");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("FXSwampGas01.nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fnv",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert swamp gas");

    assert!(report.supported, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let converted = NifFile::load(dst).expect("load converted swamp gas");
    assert_all_nif_refs_resolve(&converted);
    assert_no_pre_fo4_av_flags(&converted);
    assert!(converted.blocks.iter().all(|block| {
        block.type_name != "BSShaderNoLightingProperty" && block.type_name != "NiMaterialProperty"
    }));

    let particles = converted
        .blocks
        .iter()
        .filter(|block| block.type_name == "NiParticleSystem")
        .collect::<Vec<_>>();
    assert_eq!(particles.len(), 2);
    for particle in particles {
        assert_eq!(particle.get_field("Flags").map(NifValue::as_i64), Some(14));
        assert_eq!(
            particle.get_field("Vertex Desc").map(NifValue::as_i64),
            Some(594_510_335_218_548_785_i64)
        );
        let shader_id = ref_usize(particle.get_field("Shader Property"))
            .expect("FO4 particle shader reference");
        assert_eq!(
            converted.blocks[shader_id].type_name,
            "BSEffectShaderProperty"
        );
        let data_id = ref_usize(particle.get_field("Data")).expect("FO4 particle data reference");
        assert_eq!(converted.blocks[data_id].type_name, "NiPSysData");
        let modifiers = match particle.get_field("Modifiers") {
            Some(NifValue::Array(modifiers)) => modifiers,
            other => panic!("expected particle modifier array, got {other:?}"),
        };
        assert!(modifiers.iter().all(|modifier| {
            ref_usize(Some(modifier)).is_some_and(|id| id < converted.blocks.len())
        }));
    }

    for data in converted
        .blocks
        .iter()
        .filter(|block| block.type_name == "NiPSysData")
    {
        assert_eq!(
            data.get_field("Material CRC").map(NifValue::as_i64),
            Some(0)
        );
        assert!(matches!(
            data.get_field("Additional Data"),
            Some(NifValue::Ref(-1))
        ));
        assert_eq!(
            data.get_field("Aspect Ratio").map(NifValue::as_i64),
            Some(1)
        );
    }

    for shape in converted
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSTriShape")
    {
        let shader_id = ref_usize(shape.get_field("Shader Property"))
            .expect("converted shape shader reference");
        assert_eq!(
            converted.blocks[shader_id].type_name,
            "BSEffectShaderProperty"
        );
    }

    let texture_refs = converted.referenced_asset_paths().textures;
    assert!(
        texture_refs
            .iter()
            .any(|path| path.ends_with("effects/fxdustsmallgen01.dds")),
        "{texture_refs:?}"
    );
    assert!(
        texture_refs
            .iter()
            .any(|path| path.ends_with("effects/fxdustsmoke01.dds")),
        "{texture_refs:?}"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fnv_custom_glow_fixture_clears_pre_fo4_av_flags_when_available() {
    let src = repo_root().join("extracted/fnv/Meshes/effects/ambient/fxllcustomglow.nif");
    if !src.exists() {
        eprintln!("skipping missing fixture {}", src.display());
        return;
    }
    let dir = temp_dir("convert_fnv_custom_glow");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("FXLLcustomGlow.nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fnv",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert custom glow");

    assert!(report.supported, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let converted = NifFile::load(dst).expect("load converted custom glow");
    assert_all_nif_refs_resolve(&converted);
    assert_no_pre_fo4_av_flags(&converted);
    assert!(converted.blocks.iter().all(|block| {
        block.type_name != "BSShaderNoLightingProperty" && block.type_name != "NiMaterialProperty"
    }));

    let texture_refs = converted.referenced_asset_paths().textures;
    assert!(
        texture_refs
            .iter()
            .any(|path| path.ends_with("effects/fxparttinyglowy.dds")),
        "{texture_refs:?}"
    );
    assert!(
        texture_refs
            .iter()
            .any(|path| path.ends_with("effects/smokewispstile.dds")),
        "{texture_refs:?}"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fnv_hotel_chair_fixture_uses_fo4_furniture_marker_when_available() {
    let src = repo_root().join("extracted/fnv/Meshes/furniture/hotel/hotelchair02.nif");
    if !src.exists() {
        eprintln!("skipping missing fixture {}", src.display());
        return;
    }
    let dir = temp_dir("convert_fnv_hotel_chair");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("HotelChair02.nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fnv",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert hotel chair");

    assert!(report.supported, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("Legacy furniture markers")),
        "{:?}",
        report.changes
    );
    let converted = NifFile::load(dst).expect("load converted hotel chair");
    assert_all_nif_refs_resolve(&converted);
    assert_no_pre_fo4_av_flags(&converted);
    assert!(
        converted
            .blocks
            .iter()
            .all(|block| block.type_name != "BSFurnitureMarker")
    );
    let marker = converted
        .blocks
        .iter()
        .find(|block| block.type_name == "BSFurnitureMarkerNode")
        .expect("FO4 furniture marker node");
    let positions = match marker.get_field("Positions") {
        Some(NifValue::Array(positions)) => positions,
        other => panic!("expected furniture positions, got {other:?}"),
    };
    assert_eq!(positions.len(), 3);
    let headings = positions
        .iter()
        .map(|position| match position {
            NifValue::Struct(fields) => {
                assert_eq!(numeric_value(fields.get("Animation Type")), Some(0.0));
                assert_eq!(numeric_value(fields.get("Entry Properties")), Some(0.0));
                numeric_value(fields.get("Heading")).expect("furniture heading")
            }
            other => panic!("expected furniture position struct, got {other:?}"),
        })
        .collect::<Vec<_>>();
    assert!((headings[0] - 1.570).abs() < 0.001);
    assert!((headings[1] - 4.712).abs() < 0.001);
    assert!((headings[2] - 3.141).abs() < 0.001);
    assert!(
        converted
            .blocks
            .iter()
            .any(|block| block.type_name == "bhkNPCollisionObject")
    );
    assert!(
        converted
            .blocks
            .iter()
            .any(|block| block.type_name == "bhkPhysicsSystem")
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fnv_prospector_neon_fixture_clears_nested_legacy_flags_when_available() {
    let src = repo_root()
        .join("extracted/fnv/Meshes/architecture/goodsprings/nv_prospectorsaloon-neon_lights.nif");
    if !src.exists() {
        eprintln!("skipping missing fixture {}", src.display());
        return;
    }
    let dir = temp_dir("convert_fnv_prospector_neon");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("NV_ProspectorSaloon-Neon_Lights.nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fnv",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert Prospector Saloon neon lights");

    assert!(report.supported, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("pre-FO4 0x80000 flag")),
        "{:?}",
        report.changes
    );
    let converted = NifFile::load(dst).expect("load converted Prospector Saloon neon lights");
    assert_all_nif_refs_resolve(&converted);
    assert_no_pre_fo4_av_flags(&converted);
    assert!(converted.blocks.iter().all(|block| !matches!(
        block.type_name.as_str(),
        "NiTriStrips"
            | "NiTriStripsData"
            | "BSShaderNoLightingProperty"
            | "BSShaderPPLightingProperty"
            | "NiMaterialProperty"
    )));
    assert!(
        converted
            .blocks
            .iter()
            .any(|block| block.type_name == "BSEffectShaderProperty")
    );
    assert!(
        converted
            .blocks
            .iter()
            .any(|block| block.type_name == "BSLightingShaderProperty")
    );
    let texture_refs = converted.referenced_asset_paths().textures;
    assert!(
        texture_refs
            .iter()
            .any(|path| path.ends_with("architecture/strip/nv_neon-lights.dds")),
        "{texture_refs:?}"
    );
    assert!(
        texture_refs
            .iter()
            .any(|path| path.ends_with("architecture/strip/nv_thetops-sign04.dds")),
        "{texture_refs:?}"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fnv_tumbleweed_fixture_preserves_dynamic_collision_when_available() {
    let src = repo_root().join("extracted/fnv/Meshes/clutter/TumbleweedNV.NIF");
    if !src.exists() {
        eprintln!("skipping missing fixture {}", src.display());
        return;
    }
    let dir = temp_dir("convert_fnv_tumbleweed");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("TumbleweedNV.NIF");
    let report = convert_nif_file(
        &src,
        &dst,
        "fnv",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert tumbleweed");
    assert!(report.supported, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("(1 dynamic)")),
        "{:?}",
        report.changes
    );
    let converted = NifFile::load(dst).expect("load converted tumbleweed");
    assert_all_nif_refs_resolve(&converted);
    assert_no_pre_fo4_av_flags(&converted);
    assert_eq!(
        converted
            .blocks
            .iter()
            .find(|block| block.type_name == "BSXFlags")
            .and_then(|block| block.get_field("Integer Data"))
            .map(NifValue::as_i64),
        Some(0x42)
    );
    let blob = embedded_havok_blob(&converted).expect("converted tumbleweed collision");
    let summary = havok_native::api::havok_collision_summary(&blob)
        .expect("summarize converted tumbleweed collision");
    let summary: serde_json::Value =
        serde_json::from_str(&summary).expect("parse collision summary");
    let body = &summary["bodies"][0];
    assert_eq!(body["layer"].as_u64(), Some(4));
    assert_eq!(body["flags"].as_u64(), Some(128));
    assert_eq!(body["motion_id"].as_u64(), Some(0));
    assert_eq!(
        body["shape_class"].as_str(),
        Some("hknpConvexPolytopeShape")
    );
    assert!(body["inverse_mass"].as_f64().is_some_and(|mass| mass > 0.0));
    assert!(body["inverse_inertia"].as_array().is_some_and(|values| {
        values
            .iter()
            .all(|value| value.as_f64().is_some_and(|value| value > 0.0))
    }));
    assert!(
        summary["objects"][1]["n_vertices"]
            .as_u64()
            .is_some_and(|count| count >= 20)
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_reported_legacy_fixtures_remove_dead_properties_and_preserve_palettes() {
    let fixtures = [
        (
            "fnv",
            "extracted/fnv/Meshes/Effects/Ambient/FXDustMeshSky01.NIF",
            "FXDustMeshSky01",
        ),
        (
            "fnv",
            "extracted/fnv/Meshes/Effects/Ambient/FXRadiationDust1x2.NIF",
            "FXRadiationDust1x2",
        ),
        (
            "fnv",
            "extracted/fnv/Meshes/Terminals/TerminalDesk01.NIF",
            "TerminalDesk01",
        ),
        (
            "fnv",
            "extracted/fnv/Meshes/Dungeons/Vault/RoomU/VGearDoor01.NIF",
            "VGearDoor01",
        ),
        (
            "fnv",
            "extracted/fnv/Meshes/Dungeons/Vault/Halls/VDoor01.NIF",
            "VDoor01",
        ),
    ];
    let dir = temp_dir("convert_reported_legacy_fixtures");
    std::fs::create_dir_all(&dir).expect("create temp dir");

    for (source_game, relative_path, name) in fixtures {
        let src = repo_root().join(relative_path);
        if !src.exists() {
            eprintln!("skipping missing fixture {}", src.display());
            continue;
        }
        let dst = dir.join(format!("{name}.NIF"));
        let report = convert_nif_file(
            &src,
            &dst,
            source_game,
            "fo4",
            None,
            &ConvertFileOptions::default(),
        )
        .unwrap_or_else(|error| panic!("convert {name}: {error}"));
        assert!(report.supported, "{name}: {:?}", report.errors);
        assert!(report.errors.is_empty(), "{name}: {:?}", report.errors);

        let converted = NifFile::load(&dst).unwrap_or_else(|error| {
            panic!("load converted {name}: {error}");
        });
        assert_all_nif_refs_resolve(&converted);
        assert_animation_palette_entries_resolve(&converted);
        assert!(
            converted.blocks.iter().all(|block| !matches!(
                block.type_name.as_str(),
                "NiTriStrips"
                    | "NiTriStripsData"
                    | "NiTexturingProperty"
                    | "NiSourceTexture"
                    | "NiMaterialProperty"
                    | "BSShaderNoLightingProperty"
                    | "BSShaderPPLightingProperty"
                    | "NiAlphaController"
                    | "NiMaterialColorController"
                    | "NiTextureTransformController"
            )),
            "{name}: retained legacy-only blocks"
        );
    }

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fnv_metal_box_preserves_unlit_shape_shader_when_available() {
    let src = repo_root().join("extracted/fnv/Meshes/clutter/office/metalbox01.nif");
    if !src.exists() {
        eprintln!("skipping missing fixture {}", src.display());
        return;
    }
    let dir = temp_dir("convert_fnv_metal_box");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("MetalBox01.nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fnv",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert metal box");

    assert!(report.supported, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let converted = NifFile::load(dst).expect("load converted metal box");
    assert_all_nif_refs_resolve(&converted);
    assert!(
        converted
            .blocks
            .iter()
            .all(|block| block.type_name != "BSShaderNoLightingProperty")
    );
    let shapes = converted
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSTriShape")
        .collect::<Vec<_>>();
    assert_eq!(shapes.len(), 2);
    for shape in shapes {
        let shader_id = ref_usize(shape.get_field("Shader Property"))
            .expect("converted metal-box shape shader reference");
        assert!(matches!(
            converted.blocks[shader_id].type_name.as_str(),
            "BSLightingShaderProperty" | "BSEffectShaderProperty"
        ));
    }

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fnv_woodbeam_static_fixture_when_available() {
    let src = repo_root().join("extracted/fnv/Meshes/architecture/Wasteland/WoodBeam01.NIF");
    if !src.exists() {
        eprintln!("skipping missing fixture {}", src.display());
        return;
    }
    let dir = temp_dir("convert_fnv_woodbeam");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("WoodBeam01.nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fnv",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert wood beam");

    assert!(report.supported, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let converted = NifFile::load(dst).expect("load converted wood beam");
    let root = converted.get_block(0).expect("root");
    assert_eq!(root.get_field("Flags").map(NifValue::as_i64), Some(14));
    assert_eq!(converted.header.footer_roots, vec![0]);

    let shader = converted
        .blocks
        .iter()
        .find(|block| block.type_name == "BSLightingShaderProperty")
        .expect("lighting shader");
    assert_eq!(
        shader.get_field("Shader Flags 1").map(NifValue::as_i64),
        Some(0x8000_0001)
    );
    assert_eq!(
        shader.get_field("Shader Flags 2").map(NifValue::as_i64),
        Some(1)
    );
    let Some(NifValue::Struct(uv_scale)) = shader.get_field("UV Scale") else {
        panic!("expected UV Scale");
    };
    assert!(matches!(uv_scale.get("u"), Some(NifValue::Float(1.0))));
    assert!(matches!(uv_scale.get("v"), Some(NifValue::Float(1.0))));

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
        Some(NifValue::String(path)) if path == "textures\\weapons\\rifle_d.dds"
    ));
    assert!(matches!(
        textures.get(2),
        Some(NifValue::String(path)) if path == "textures\\weapons\\rifle_g.dds"
    ));
    assert!(matches!(
        textures.get(7),
        Some(NifValue::String(path)) if path == "textures\\weapons\\rifle_s.dds"
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
fn convert_fo76_named_bgsm_shader_keeps_nif_texture_set() {
    let dir = temp_dir("convert_fo76_named_bgsm_shader");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");

    let mut nif = fo76_named_material_shader_nif();
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
            .any(|change| change.contains("filled FO4 defaults")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let shader = converted
        .blocks
        .iter()
        .find(|block| block.type_name == "BSLightingShaderProperty")
        .expect("lighting shader");
    let texset_id = match shader.get_field("Texture Set") {
        Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
        other => panic!("expected linked texture set, got {other:?}"),
    };
    assert!(matches!(
        shader.get_field("Name"),
        Some(NifValue::String(path))
            if path == "Materials\\Landscape\\Grass\\ForestGrass01.BGSM"
    ));
    assert_eq!(
        shader.get_field("Shader Type").map(NifValue::as_i64),
        Some(0)
    );
    assert!(
        shader
            .get_field("Shader Flags 1")
            .map(NifValue::as_i64)
            .is_some_and(|flags| flags & (1 << 7) == 0)
    );
    let texset = converted.get_block(texset_id).expect("texture set");
    assert_eq!(texset.type_name, "BSShaderTextureSet");
    let textures = match texset.get_field("Textures") {
        Some(NifValue::Array(textures)) => textures,
        other => panic!("expected texture array, got {other:?}"),
    };
    assert_eq!(textures.len(), 10);
    assert_eq!(
        texture_string(&textures[0]).to_ascii_lowercase(),
        "textures\\landscape\\grass\\forestgrass01_d.dds"
    );
    assert_eq!(
        texture_string(&textures[1]).to_ascii_lowercase(),
        "textures\\landscape\\grass\\forestgrass01_n.dds"
    );
    assert_eq!(
        texture_string(&textures[7]).to_ascii_lowercase(),
        "textures\\shared\\default_s.dds"
    );
    assert_eq!(texture_string(&textures[2]), "");
    assert!(shader.get_field("Shader Flags 1").is_some());
    assert!(shader.get_field("Wetness").is_some());

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
fn convert_real_fo76_vault_glass_hydrates_external_bgem_shader() {
    let source = repo_path(
        "extracted/fo76/Meshes/interiors/vault/freewall/vltwinoverseer02wideglassclear.nif",
    );
    let source_material_dir = repo_path("extracted/fo76");
    if !source.exists() {
        eprintln!("skip: FO76 vault glass fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_vault_glass_bgem");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let destination = dir.join("converted.nif");
    let report = convert_nif_file(
        &source,
        &destination,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions {
            source_material_dir: Some(source_material_dir),
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert vault glass");

    assert!(report.supported, "{:?}", report.errors);
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("normalized 1 external BGEM shader")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(destination).expect("load converted vault glass");
    let shader = converted
        .blocks
        .iter()
        .find(|block| block.type_name == "BSEffectShaderProperty")
        .expect("effect shader");
    let flags1 = shader
        .get_field("Shader Flags 1")
        .map(NifValue::as_i64)
        .unwrap_or_default();
    let flags2 = shader
        .get_field("Shader Flags 2")
        .map(NifValue::as_i64)
        .unwrap_or_default();
    assert_ne!(flags1 & (1 << 7), 0, "environment mapping");
    assert_ne!(flags1 & (1 << 31), 0, "z-buffer test");
    assert_eq!(flags1 & (1 << 6), 0, "source BGEM disables falloff");
    assert_ne!(flags2 & (1 << 30), 0, "effect lighting");
    assert!(matches!(
        shader.get_field("Source Texture"),
        Some(NifValue::String(path))
            if path.eq_ignore_ascii_case(
                r"textures\Architecture\Capsules\CapGlassTile01_d.dds"
            )
    ));
    assert!(matches!(
        shader.get_field("Normal Texture"),
        Some(NifValue::String(path))
            if path.eq_ignore_ascii_case(
                r"textures\Architecture\Capsules\CapGlassTile01_n.dds"
            )
    ));
    assert!(matches!(
        shader.get_field("Env Mask Texture"),
        Some(NifValue::String(path))
            if path.eq_ignore_ascii_case(
                r"textures\Architecture\Capsules\CapGlassTile01_m.dds"
            )
    ));
    assert!(matches!(
        shader.get_field("Env Map Texture"),
        Some(NifValue::String(path)) if !path.is_empty()
    ));
    let base_alpha = match shader.get_field("Base Color") {
        Some(NifValue::Color4(color)) => color[3] as f64,
        Some(NifValue::Struct(fields)) => match fields.get("a") {
            Some(NifValue::Float(value)) => *value,
            other => panic!("expected base-color alpha, got {other:?}"),
        },
        other => panic!("expected base color, got {other:?}"),
    };
    assert!((base_alpha - 0.5).abs() < f64::EPSILON);
    assert!(matches!(
        shader.get_field("Base Color Scale"),
        Some(NifValue::Float(value)) if (*value - 0.5).abs() < f64::EPSILON
    ));
    assert!(matches!(
        shader.get_field("Environment Map Scale"),
        Some(NifValue::Float(value)) if (*value - 0.6).abs() < 0.0001
    ));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_18d409_ultracite_vein_preserves_masked_glow() {
    let source = repo_path("extracted/fo76/Meshes/landscape/plants/mineralwallultracite01.nif");
    if !source.exists() {
        eprintln!("skip: FO76 18D409 ultracite vein fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_18d409_ultracite_vein_glow");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let destination = dir.join("converted.nif");
    let report = convert_nif_file(
        &source,
        &destination,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions {
            source_material_dir: Some(repo_path("extracted/fo76")),
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert 18D409 ultracite vein");

    assert!(report.supported, "{:?}", report.errors);
    let converted = NifFile::load(destination).expect("load converted ultracite vein");
    let find_shader = |material_name: &str| {
        converted
            .blocks
            .iter()
            .filter(|block| block.type_name == "BSLightingShaderProperty")
            .find(|shader| {
                matches!(
                    shader.get_field("Name"),
                    Some(NifValue::String(path)) if path.eq_ignore_ascii_case(material_name)
                )
            })
            .unwrap_or_else(|| panic!("missing shader for {material_name}"))
    };
    let texture_set = |shader: &nif_core_native::model::NifBlock| {
        let texture_set_id = match shader.get_field("Texture Set") {
            Some(NifValue::Ref(reference)) if *reference >= 0 => *reference as usize,
            other => panic!("expected texture set, got {other:?}"),
        };
        converted
            .get_block(texture_set_id)
            .expect("ultracite texture set")
    };

    let crystal_shader = find_shader(r"Materials\Landscape\Plants\Mineral_Ultracite01.bgsm");
    assert_eq!(
        crystal_shader
            .get_field("Shader Type")
            .map(NifValue::as_i64),
        Some(2)
    );
    assert!(
        crystal_shader
            .get_field("Shader Flags 2")
            .map(NifValue::as_i64)
            .is_some_and(|flags| flags & (1 << 6) != 0)
    );
    let crystal_textures = match texture_set(crystal_shader).get_field("Textures") {
        Some(NifValue::Array(textures)) => textures,
        other => panic!("expected crystal texture array, got {other:?}"),
    };
    assert_eq!(
        texture_string(&crystal_textures[2]).to_ascii_lowercase(),
        r"textures\landscape\plants\mineral_ultracite01_g.dds"
    );

    let wall_shader = find_shader(r"Materials\Landscape\Plants\MineralWall_Ultracite01.bgsm");
    assert_eq!(
        wall_shader.get_field("Shader Type").map(NifValue::as_i64),
        Some(0)
    );
    assert!(
        wall_shader
            .get_field("Shader Flags 2")
            .map(NifValue::as_i64)
            .is_some_and(|flags| flags & (1 << 6) == 0)
    );
    let wall_textures = match texture_set(wall_shader).get_field("Textures") {
        Some(NifValue::Array(textures)) => textures,
        other => panic!("expected wall texture array, got {other:?}"),
    };
    assert_eq!(texture_string(&wall_textures[2]), "");

    assert_eq!(
        converted
            .blocks
            .iter()
            .filter(|block| block.type_name == "BSLightingShaderProperty")
            .filter(|shader| {
                shader
                    .get_field("Shader Flags 2")
                    .map(NifValue::as_i64)
                    .is_some_and(|flags| flags & (1 << 6) != 0)
            })
            .count(),
        1
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_1e3947_glass_matches_fo4_transparency_contract() {
    let source =
        repo_path("extracted/fo76/Meshes/interiors/vault/freewall/vltwinoverseer02wideglass.nif");
    let fo4 =
        repo_path("extracted/fo4/Meshes/Interiors/Vault/FreeWall/VltWinOverseer02WideGlass.nif");
    if !source.exists() || !fo4.exists() {
        eprintln!("skip: FO76/FO4 1E3947 vault glass fixtures not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_1e3947_vault_glass");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let destination = dir.join("converted.nif");
    convert_nif_file(
        &source,
        &destination,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions {
            source_material_dir: Some(repo_path("extracted/fo76")),
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert 1E3947 vault glass");

    let converted = NifFile::load(destination).expect("load converted 1E3947 vault glass");
    let fo4 = NifFile::load(fo4).expect("load FO4 1E3947 vault glass");
    let converted_shader = converted
        .blocks
        .iter()
        .find(|block| block.type_name == "BSEffectShaderProperty")
        .expect("converted glass shader");
    let fo4_shader = fo4
        .blocks
        .iter()
        .find(|block| block.type_name == "BSEffectShaderProperty")
        .expect("FO4 glass shader");
    let transparency_flags = (1 << 6) | (1 << 7) | (1 << 31);
    assert_eq!(
        converted_shader
            .get_field("Shader Flags 1")
            .map(NifValue::as_i64)
            .unwrap_or_default()
            & transparency_flags,
        fo4_shader
            .get_field("Shader Flags 1")
            .map(NifValue::as_i64)
            .unwrap_or_default()
            & transparency_flags
    );
    assert!(matches!(
        (
            converted_shader.get_field("Source Texture"),
            fo4_shader.get_field("Source Texture")
        ),
        (Some(NifValue::String(converted)), Some(NifValue::String(fo4)))
            if converted.eq_ignore_ascii_case(fo4)
    ));
    assert!(matches!(
        converted_shader.get_field("Env Map Texture"),
        Some(NifValue::String(path)) if !path.is_empty()
    ));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_waterflow00_writes_fo4_water_shader_and_collision_order() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/water/waterflows/waterflow00.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted waterflow fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_waterflow00");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("WaterFlow00.nif");

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

    let converted = NifFile::load(dst).expect("load converted nif");
    let shader = converted
        .blocks
        .iter()
        .find(|block| block.type_name == "BSWaterShaderProperty")
        .expect("water shader");
    assert_eq!(
        shader.get_field("Shader Flags 1").map(NifValue::as_i64),
        Some(0x8000_0000)
    );
    assert_eq!(
        shader.get_field("Shader Flags 2").map(NifValue::as_i64),
        Some(1)
    );
    assert_eq!(
        shader.get_field("Water Shader Flags").map(NifValue::as_i64),
        Some(0xC4)
    );
    assert!(shader.get_field("SF1").is_none());
    assert!(shader.get_field("SF2").is_none());
    assert!(shader.get_field("UV Offset").is_some());
    assert!(shader.get_field("UV Scale").is_some());

    assert_eq!(
        converted
            .blocks
            .get(8)
            .map(|block| block.type_name.as_str()),
        Some("bhkNPCollisionObject")
    );
    assert_eq!(
        converted
            .blocks
            .get(9)
            .map(|block| block.type_name.as_str()),
        Some("bhkPhysicsSystem")
    );
    let collision = converted.get_block(8).expect("collision block");
    assert_eq!(collision.get_field("Data").map(NifValue::as_i64), Some(9));

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
fn convert_real_fo76_hightech_bookshelf_window_preserves_thin_static_collision() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../../extracted/fo76/meshes/setdressing/hightech/shelves/hightechbookshelf02window01static.nif",
    );
    if !src.exists() {
        eprintln!("skip: FO76 extracted bookshelf fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load source nif");
    let source_blob = embedded_havok_blob(&source).expect("source collision blob");
    let source_preview = extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(0))
        .expect("source collision preview");
    let (source_min, source_max) = preview_aabb(&source_preview);
    let source_y_extent = source_max[1] - source_min[1];
    assert!(
        source_y_extent < 1.0,
        "fixture should have thin source collision on Y, got {source_y_extent}"
    );

    let dir = temp_dir("convert_real_fo76_hightech_bookshelf_window");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir
        .join("out")
        .join("HighTechBookshelf02Window01Static.nif");
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
            .all(|warning| !warning.contains("AABB fallback")),
        "{:?}",
        report.warnings
    );
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("source-compressed-mesh=1")
                && change.contains("visible-mesh-aabb-fallback=0")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(&dst).expect("load converted nif");
    let converted_blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let converted_preview = extract_preview_meshes_from_blob(&converted_blob, 69.99125, Some(0))
        .expect("converted collision preview");
    assert_same_preview_aabb(&source_preview, &converted_preview, 0.05);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_hightech_bookshelf_merges_convex_children_to_compressed_mesh() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../../extracted/fo76/meshes/setdressing/hightech/shelves/hightechbookshelf02c.nif",
    );
    if !src.exists() {
        eprintln!("skip: FO76 extracted bookshelf fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load source nif");
    let source_blob = embedded_havok_blob(&source).expect("source collision blob");
    let source_preview = extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(0))
        .expect("source collision preview");
    assert_eq!(source_preview.len(), 9, "{source_preview:?}");
    assert!(
        source_preview
            .iter()
            .all(|mesh| mesh.shape_type == "convex_hull"),
        "{source_preview:?}"
    );

    let dir = temp_dir("convert_real_fo76_hightech_bookshelf_compound");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("HighTechBookshelf02c.nif");
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
            .any(|change| change.contains("source-compressed-mesh=1")
                && change.contains("source-compound=0")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(&dst).expect("load converted nif");
    let converted_blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let converted_preview = extract_preview_meshes_from_blob(&converted_blob, 69.99125, Some(0))
        .expect("converted collision preview");
    assert_eq!(converted_preview.len(), 1);
    assert!(
        converted_preview[0].shape_type == "compressed_mesh",
        "{converted_preview:?}"
    );
    assert_same_preview_aabb(&source_preview, &converted_preview, 0.05);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_atx_logcabin_preserves_body_local_roof_collision() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/atx/architecture/logcabin/atx_logcabin.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted ATX_LogCabin fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load source nif");
    let source_blob = embedded_havok_blob(&source).expect("source collision blob");
    let instance_counts = compound_instance_counts(&source_blob);
    assert!(
        instance_counts.iter().any(|count| *count == 6),
        "LogCabin TAG0 compound instances must materialize, including the six-child roof compound: {instance_counts:?}"
    );
    assert!(
        instance_counts.iter().filter(|count| **count > 0).count() >= 5,
        "LogCabin TAG0 compound instance arrays were dropped: {instance_counts:?}"
    );
    let wall_preview = extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(0))
        .expect("wall body preview");
    let roof_preview = extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(7))
        .expect("roof body preview");
    let rotated_roof_preview = extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(9))
        .expect("rotated roof body preview");
    assert_eq!(wall_preview.len(), 2);
    assert_eq!(roof_preview.len(), 6);
    assert_eq!(rotated_roof_preview.len(), 2);
    for mesh in &wall_preview {
        let extent = preview_mesh_extent(mesh);
        assert!(
            extent[1] > extent[0] * 4.0,
            "LogCabin wall compound leaves must retain their 90-degree source orientation: {extent:?}"
        );
    }

    let dir = temp_dir("convert_real_fo76_atx_logcabin");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("ATX_LogCabin.nif");
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
            .all(|warning| !warning.contains("source collision unavailable")
                && !warning.contains("minimal AABB fallback")),
        "{:?}",
        report.warnings
    );
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("visible-mesh-aabb-fallback=0")
                && change.contains("stripped-unrecoverable=0")),
        "{:?}",
        report.changes
    );
    assert!(
        report.changes.iter().any(|change| change
            .contains("block=36 name=\"CabinNewRoofACornerOut01\"")
            && change.contains("shape compound(6 children:compressed_meshx6)")
            && change.contains("compressed_mesh")
            && change.contains("route=source-compressed-mesh")),
        "{:?}",
        report.changes
    );
    let converted = NifFile::load(&dst).expect("load converted nif");
    for node_name in [
        "CabinNewWallA01",
        "CabinNewRoofACornerOut01",
        "CabinNewRoofACornerOut003",
    ] {
        let source_collisions = collision_blobs_for_named_nodes(&source, node_name);
        assert_eq!(source_collisions.len(), 1, "source {node_name} collision");
        let converted_collisions = collision_blobs_for_named_nodes(&converted, node_name);
        assert_eq!(
            converted_collisions.len(),
            1,
            "converted {node_name} collision; changes={:?}; warnings={:?}",
            report.changes,
            report.warnings
        );
        let source_preview = extract_preview_meshes_from_blob(
            &source_collisions[0].0,
            69.99125,
            Some(source_collisions[0].1),
        )
        .expect("source collision preview");
        let converted_preview = extract_preview_meshes_from_blob(
            &converted_collisions[0].0,
            69.99125,
            Some(converted_collisions[0].1),
        )
        .expect("converted collision preview");
        assert_same_preview_aabb(&source_preview, &converted_preview, 0.25);
    }

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_facegeom_skin_tint_shaders_get_conditional_fields() {
    // Regression: FO76 facegen heads carry Shader Type 5 (Skin Tint) shaders.
    // FO4 reads Skin Tint Color + Alpha (16 bytes) for type 5; if the converter
    // omits them the engine over-reads the next block and CTDs in BSStringPool.
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../../extracted/fo76/Meshes/Actors/Character/FaceGenData/FaceGeom/SeventySix.esm/000A0E32.nif",
    );
    if !src.exists() {
        eprintln!("skip: FO76 extracted facegeom fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_facegeom_skin_tint");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("000A0E32.nif");
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

    let converted = NifFile::load(&dst).expect("load converted nif");
    let mut skin_tint_shaders = 0usize;
    for block in &converted.blocks {
        if block.type_name != "BSLightingShaderProperty"
            || block.get_field("Shader Type").map(|v| v.as_i64()) != Some(5)
        {
            continue;
        }
        skin_tint_shaders += 1;
        // After serialize+reload a color reads back as a Struct{r,g,b}; the
        // crash-relevant invariant is simply that the 16 type-5 trailing bytes
        // (Skin Tint Color + Alpha) are present.
        assert!(
            block.get_field("Skin Tint Color").is_some(),
            "type 5 shader missing Skin Tint Color"
        );
        assert!(
            block.get_field("Skin Tint Alpha").is_some(),
            "type 5 shader missing Skin Tint Alpha"
        );
    }
    assert!(
        skin_tint_shaders >= 3,
        "expected >=3 Skin Tint shaders in facegeom, found {skin_tint_shaders}"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_facegeom_preserves_root_flags() {
    let src = repo_path(
        "extracted/fo76/meshes/actors/character/facegendata/facegeom/seventysix.esm/006fd088.nif",
    );
    if !src.is_file() {
        eprintln!("skip: FO76 extracted Lane facegeom fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load source facegeom");
    assert_eq!(
        source.blocks[0].get_field("Flags").map(NifValue::as_i64),
        Some(0x400E)
    );

    let dir = temp_dir("convert_real_fo76_facegeom_preserves_root_flags");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("006fd088.nif");
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
    let converted = NifFile::load(dst).expect("load converted facegeom");
    assert_eq!(
        converted.blocks[0].get_field("Flags").map(NifValue::as_i64),
        Some(0x400E)
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_teddy_valid_polytope_preserves_dynamic_clutter_collision() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/setdressing/teddybear/toyteddybeardirty01_green.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted teddy fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_teddy_valid_polytope");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");

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
            .all(|warning| !warning.contains("AABB fallback")),
        "{:?}",
        report.warnings
    );
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("source-polytope=1")
                && change.contains("clutter-convex=0")
                && change.contains("visible-mesh-aabb-fallback=0")
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
        1
    );
    let blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let meshes =
        extract_preview_meshes_from_blob(&blob, 69.99125, Some(0)).expect("preview collision");
    assert_eq!(meshes.len(), 1, "{meshes:?}");
    assert_eq!(meshes[0].vertices.len(), 12, "{:?}", meshes[0]);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_boslp_part_is_byte_deterministic() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../../extracted/fo76/meshes/architecture/airport/boslpparts/boslpleftarmpart07.nif",
    );
    if !src.exists() {
        eprintln!("skip: FO76 extracted boslp fixture not available");
        return;
    }
    let dir = temp_dir("convert_real_fo76_boslp_determinism");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let mut hashes = std::collections::HashSet::new();
    for i in 0..8 {
        let dst = dir.join(format!("out_{i}")).join("converted.nif");
        convert_nif_file(
            &src,
            &dst,
            "fo76",
            "fo4",
            None,
            &ConvertFileOptions::default(),
        )
        .expect("convert nif");
        let bytes = std::fs::read(&dst).unwrap();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&bytes, &mut hasher);
        hashes.insert(std::hash::Hasher::finish(&hasher));
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        hashes.len(),
        1,
        "boslp part output not byte-deterministic across 8 runs"
    );
}

#[test]
fn convert_real_fo76_firecracker_valid_polytopes_preserve_source_collision() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/landscape/plants/firecrackertrap01.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted firecracker fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_firecracker_valid_polytopes");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");

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
            .all(|warning| !warning.contains("AABB fallback")),
        "{:?}",
        report.warnings
    );
    assert!(
        report.changes.iter().any(
            |change| change.contains("regenerated 2 FO4 collision object")
                && change.contains("source-compressed-mesh=2")
                && change.contains("visible-mesh-aabb-fallback=0")
                && change.contains("stripped-unrecoverable=0")
        ),
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
    let blobs = embedded_havok_blobs(&converted);
    assert!(!blobs.is_empty(), "missing converted collision blob");
    let preview_count = blobs
        .iter()
        .flat_map(|blob| {
            extract_preview_meshes_from_blob(blob, 69.99125, None)
                .expect("preview collision")
                .into_iter()
        })
        .filter(|mesh| !mesh.vertices.is_empty())
        .count();
    assert!(preview_count > 0, "converted collision preview was empty");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_oil_lamp_does_not_reconvert_rebuilt_np_collision() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../../extracted/fo76/meshes/setdressing/lightfixtures/lightoillampoff_handleup.nif",
    );
    if !src.exists() {
        eprintln!("skip: FO76 extracted oil lamp fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_oil_lamp_collision");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");

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
            .any(|change| change.contains("source-polytope=1")
                && change.contains("clutter-convex=0")
                && change.contains("visible-mesh-aabb-fallback=0")),
        "{:?}",
        report.changes
    );
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("skipped 1 regenerated NP collision blob")),
        "{:?}",
        report.changes
    );
    assert!(
        !report
            .changes
            .iter()
            .any(|change| change.contains("converted 1 embedded FO76 blob")),
        "{:?}",
        report.changes
    );
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("cleared Environment_Mapping")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let shader = converted
        .blocks
        .iter()
        .find(|block| block.type_name == "BSLightingShaderProperty")
        .expect("lighting shader");
    let flags = shader
        .get_field("Shader Flags 1")
        .map(NifValue::as_i64)
        .unwrap_or_default();
    assert_eq!(
        flags & (1 << 7),
        0,
        "Environment_Mapping must be cleared when FO4 cubemap slot is empty"
    );
    let blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let meshes =
        extract_preview_meshes_from_blob(&blob, 69.99125, Some(0)).expect("preview collision");
    assert_eq!(meshes.len(), 1, "{meshes:?}");
    assert_eq!(meshes[0].vertices.len(), 8, "{:?}", meshes[0]);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_miner_lamp_preserves_dynamic_clutter_collision() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/Props/Miner/Miner_Lamp_03.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted miner lamp fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_miner_lamp_collision");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");

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
            .any(|change| change.contains("source-polytope=2")
                && change.contains("clutter-convex=0")),
        "{:?}",
        report.changes
    );
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("dynamic-clutter+motionCinfo+clutter-mass")),
        "{:?}",
        report.changes
    );
    assert!(
        !report
            .changes
            .iter()
            .any(|change| change.contains("static+motionCinfo+clutter-mass")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let blobs = embedded_havok_blobs(&converted);
    assert_eq!(
        blobs.len(),
        1,
        "miner lamp should embed one shared physics blob"
    );

    let hkx = havok_native::hkx::model::HkxFile::read(&blobs[0]).expect("parse physics blob");
    let psd = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpPhysicsSystemData")
        .expect("physics system data");
    let body_cinfos = hkx_object_array(psd, "bodyCinfos");
    let motion_cinfos = hkx_object_array(psd, "motionCinfos");
    let motion_properties = hkx_object_array(psd, "motionProperties");

    assert_eq!(body_cinfos.len(), 2, "both source bodies should be rebuilt");
    assert_eq!(
        motion_cinfos.len(),
        2,
        "dynamic clutter bodies need one motionCinfo each"
    );
    assert_eq!(
        motion_properties.len(),
        1,
        "dynamic clutter bodies must index a valid motionProperties entry"
    );

    for (idx, body) in body_cinfos.iter().enumerate() {
        assert_eq!(
            hkx_member_i64(body, "collisionFilterInfo") & 0xFF,
            4,
            "body {idx} must stay on FO4 CLUTTER layer"
        );
        assert_eq!(
            hkx_member_i64(body, "flags"),
            128,
            "body {idx} must be flagged dynamic"
        );
        assert_eq!(
            hkx_member_i64(body, "motionId"),
            idx as i64,
            "body {idx} must point at its dynamic motionCinfo"
        );
    }

    for (idx, motion) in motion_cinfos.iter().enumerate() {
        assert_eq!(
            hkx_member_i64(motion, "motionPropertiesId"),
            0,
            "motion {idx} must reference motionProperties[0]"
        );
        assert!(
            hkx_member_f32(motion, "inverseMass") > 0.0,
            "motion {idx} must carry nonzero inverse mass"
        );
    }

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_nukacola_machine_static_compound_uses_static_compressed_motion_cinfos() {
    let src = repo_path(
        "extracted/fo76/meshes/setdressing/nukacolamachine/nukacolamachine01_baseonly.nif",
    );
    if !src.exists() {
        eprintln!("skip: FO76 extracted Nuka-Cola machine fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_nukacola_machine_static_compound");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("NukaColaMachine01_BaseOnly.nif");

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
        !report
            .changes
            .iter()
            .any(|change| change.contains("direct-transcoded")),
        "NavCut collision systems must stay on the reconstruction path: {:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let (motion_ids, motion_cinfo_count) = physics_motion_ids(&blob);
    assert_eq!(
        motion_cinfo_count, 2,
        "shared static compressed-mesh systems use motionCinfos for FO4 collision parity"
    );
    assert_eq!(
        motion_ids,
        vec![0, 1],
        "shared compressed-mesh bodies must reference their motionCinfos"
    );

    let summary_json = havok_native::api::havok_collision_summary(&blob).expect("summary");
    let summary: serde_json::Value = serde_json::from_str(&summary_json).expect("summary JSON");
    let bodies = summary
        .get("bodies")
        .and_then(serde_json::Value::as_array)
        .expect("bodies");
    assert!(
        bodies.iter().all(|body| {
            body.get("shape_class").and_then(serde_json::Value::as_str)
                != Some("hknpDynamicCompoundShape")
        }),
        "static vending-machine collision must not emit dynamic compound shapes"
    );
    assert!(
        bodies.iter().all(|body| {
            body.get("collision_filter_info")
                .and_then(serde_json::Value::as_i64)
                .map(|filter| filter & 0xFF)
                != Some(4)
        }),
        "static vending-machine bodies must not be remapped to dynamic CLUTTER"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_tire_swing_keyframed_refmass_is_not_dynamic_clutter() {
    let src = repo_path("extracted/fo76/meshes/setdressing/tireswing/tireswing01.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted TireSwing fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load source nif");
    let source_blob = embedded_havok_blob(&source).expect("source collision blob");
    let source_summary_json =
        havok_native::api::havok_collision_summary(&source_blob).expect("source summary");
    let source_summary: serde_json::Value =
        serde_json::from_str(&source_summary_json).expect("source summary JSON");
    let source_bodies = source_summary
        .get("bodies")
        .and_then(serde_json::Value::as_array)
        .expect("source bodies");
    assert_eq!(
        source_bodies.len(),
        7,
        "fixture should have 7 source bodies"
    );
    assert!(
        source_bodies.iter().skip(1).all(|body| {
            body.get("motion_type").and_then(serde_json::Value::as_i64) == Some(2)
                && body
                    .get("collision_filter_info")
                    .and_then(serde_json::Value::as_i64)
                    .map(|filter| filter & 0xFF)
                    == Some(4)
        }),
        "fixture should exercise keyframed layer-4 bodies"
    );

    let dir = temp_dir("convert_real_fo76_tire_swing_keyframed");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("TireSwing01.nif");
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
            .any(|change| change.contains("keyframed-refmass -> keyframed+motionCinfo")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let blobs = embedded_havok_blobs(&converted);
    assert_eq!(
        blobs.len(),
        1,
        "tire swing should use one shared physics blob"
    );

    let hkx = havok_native::hkx::model::HkxFile::read(&blobs[0]).expect("parse physics blob");
    assert!(
        hkx.objects()
            .iter()
            .all(|object| object.class_name != "hknpDynamicCompoundShape"),
        "keyframed tire swing must not emit the dynamic compound shape that crashes FO4 narrowphase"
    );

    let psd = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpPhysicsSystemData")
        .expect("physics system data");
    let body_cinfos = hkx_object_array(psd, "bodyCinfos");
    let motion_cinfos = hkx_object_array(psd, "motionCinfos");

    assert_eq!(body_cinfos.len(), 7, "all source bodies should be rebuilt");
    assert_eq!(
        motion_cinfos.len(),
        6,
        "six source keyframed bodies should emit keyframed motionCinfos"
    );

    let invalid_motion_count = body_cinfos
        .iter()
        .filter(|body| hkx_member_i64(body, "motionId") == 0x7FFF_FFFF)
        .count();
    assert_eq!(
        invalid_motion_count, 1,
        "one static anchor body should keep HK_INVALID motion"
    );
    let mut keyframed_motion_ids = Vec::new();
    for (idx, body) in body_cinfos.iter().enumerate() {
        let motion_id = hkx_member_i64(body, "motionId");
        if motion_id == 0x7FFF_FFFF {
            assert_eq!(
                hkx_member_i64(body, "flags"),
                0,
                "static anchor body {idx} must not be flagged dynamic"
            );
            continue;
        }
        assert_ne!(
            hkx_member_i64(body, "collisionFilterInfo") & 0xFF,
            4,
            "keyframed body {idx} must not remain on FO4 CLUTTER"
        );
        assert_eq!(
            hkx_member_i64(body, "flags"),
            0,
            "keyframed body {idx} must not be flagged dynamic"
        );
        keyframed_motion_ids.push(motion_id);
    }
    keyframed_motion_ids.sort_unstable();
    assert_eq!(
        keyframed_motion_ids,
        vec![0, 1, 2, 3, 4, 5],
        "keyframed bodies must point at the six keyframed motionCinfos"
    );
    for (idx, motion) in motion_cinfos.iter().enumerate() {
        assert_eq!(
            hkx_member_i64(motion, "motionPropertiesId"),
            0xFFFF,
            "keyframed motion {idx} must not index dynamic motionProperties"
        );
        assert_eq!(
            hkx_member_f32(motion, "inverseMass"),
            0.0,
            "keyframed motion {idx} should be infinite-mass, not dynamic clutter"
        );
    }

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_offroad_vehicle_dynamic_motion_without_bsx_is_static() {
    let src = repo_path(
        "extracted/fo76/meshes/vehicles/offroadvehicle/vehicle_offroadvehicle01_destroyed.nif",
    );
    if !src.exists() {
        eprintln!("skip: FO76 extracted offroad vehicle fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load source nif");
    assert!(
        source
            .blocks
            .iter()
            .any(|block| block.type_name == "BSXFlags"
                && matches!(block.get_field("Integer Data"), Some(NifValue::UInt(130)))),
        "fixture should keep the crash-relevant BSX=130 signal"
    );
    let source_blob = embedded_havok_blob(&source).expect("source collision blob");
    let source_summary_json =
        havok_native::api::havok_collision_summary(&source_blob).expect("source summary");
    let source_summary: serde_json::Value =
        serde_json::from_str(&source_summary_json).expect("source summary JSON");
    let source_body = source_summary
        .get("bodies")
        .and_then(serde_json::Value::as_array)
        .and_then(|bodies| bodies.first())
        .expect("source body");
    assert_eq!(
        source_body
            .get("shape_class")
            .and_then(serde_json::Value::as_str),
        Some("hknpCompoundShape")
    );
    assert_eq!(
        source_body
            .get("motion_type")
            .and_then(serde_json::Value::as_i64),
        Some(1),
        "fixture should exercise the dynamic-motion/no-BSX crash path"
    );
    assert_eq!(
        source_body
            .get("collision_filter_info")
            .and_then(serde_json::Value::as_i64)
            .map(|filter| filter & 0xFF),
        Some(2)
    );

    let dir = temp_dir("convert_real_fo76_offroad_vehicle_static");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir
        .join("out")
        .join("Vehicle_OffRoadVehicle01_Destroyed.nif");
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
            .any(|change| change.contains("source-compressed-mesh=1")),
        "{:?}",
        report.changes
    );
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("dynamic-motion-without-bsx-kept-static")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let (motion_ids, motion_cinfo_count) = physics_motion_ids(&blob);
    assert_eq!(
        motion_cinfo_count, 1,
        "layer-2 compressed mesh uses one static motionCinfo"
    );
    assert_eq!(
        motion_ids,
        vec![0],
        "layer-2 vehicle body should not be remapped to dynamic clutter motion"
    );

    let summary_json = havok_native::api::havok_collision_summary(&blob).expect("summary");
    let summary: serde_json::Value = serde_json::from_str(&summary_json).expect("summary JSON");
    let bodies = summary
        .get("bodies")
        .and_then(serde_json::Value::as_array)
        .expect("bodies");
    assert!(
        bodies.iter().all(|body| {
            body.get("shape_class").and_then(serde_json::Value::as_str)
                != Some("hknpDynamicCompoundShape")
        }),
        "BSX=130 vehicle must not emit FO4 dynamic compound collision"
    );
    assert_eq!(
        bodies
            .first()
            .and_then(|body| body.get("collision_filter_info"))
            .and_then(serde_json::Value::as_i64)
            .map(|filter| filter & 0xFF),
        Some(2),
        "source layer 2 should not be remapped to dynamic CLUTTER"
    );

    let compressed_meshes = havok_native::hkx::model::HkxFile::read(&blob)
        .expect("parse physics blob")
        .objects()
        .iter()
        .filter(|object| object.class_name == "hknpCompressedMeshShape")
        .count();
    assert_eq!(
        compressed_meshes, 1,
        "static vehicle compound should merge to one compressed mesh"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_whitespring_lamp_refmass_without_complex_bsx_is_static() {
    let src = repo_path("extracted/fo76/meshes/setdressing/whitespring/whitespringlamp03off.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted Whitespring lamp fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load source nif");
    let source_blob = embedded_havok_blob(&source).expect("source collision blob");
    assert!(
        source
            .blocks
            .iter()
            .any(|block| block.type_name == "BSXFlags"
                && matches!(block.get_field("Integer Data"), Some(NifValue::UInt(194)))),
        "fixture should keep the crash-relevant BSX=194 signal"
    );
    let source_summary =
        havok_native::api::havok_collision_summary(&source_blob).expect("source collision summary");
    assert!(
        source_summary.contains("hknpCompoundShape"),
        "fixture should start as a FO76 compound shape"
    );

    let dir = temp_dir("convert_real_fo76_whitespring_lamp_mass");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("WhitespringLamp03Off.nif");
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
            .any(|change| change.contains("source-compressed-mesh=1")),
        "{:?}",
        report.changes
    );
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("refmass-without-dynamic-complex-bsx-kept-static")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let (motion_ids, motion_cinfo_count) = physics_motion_ids(&blob);
    assert_eq!(
        motion_cinfo_count, 0,
        "static lamp must not get motionCinfos"
    );
    assert_eq!(
        motion_ids,
        vec![0x7FFF_FFFF],
        "static lamp body must use HK_INVALID motion id"
    );

    let summary_json = havok_native::api::havok_collision_summary(&blob).expect("summary");
    let summary: serde_json::Value = serde_json::from_str(&summary_json).expect("summary JSON");
    let bodies = summary
        .get("bodies")
        .and_then(serde_json::Value::as_array)
        .expect("bodies");
    assert!(
        bodies.iter().all(|body| {
            body.get("shape_class").and_then(serde_json::Value::as_str)
                != Some("hknpDynamicCompoundShape")
        }),
        "BSX=194 wall lamps must not emit FO4 dynamic compound collision"
    );
    assert_eq!(
        bodies
            .first()
            .and_then(|body| body.get("collision_filter_info"))
            .and_then(serde_json::Value::as_i64)
            .map(|filter| filter & 0xFF),
        Some(1),
        "lamp layer 4 should be demoted to FO4 STATIC"
    );

    let hkx = havok_native::hkx::model::HkxFile::read(&blob).expect("parse physics blob");
    let psd = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpPhysicsSystemData")
        .expect("physics system data");
    let body_cinfos = hkx_object_array(psd, "bodyCinfos");
    let motion_cinfos = hkx_object_array(psd, "motionCinfos");

    assert_eq!(body_cinfos.len(), 1);
    assert_eq!(motion_cinfos.len(), 0);
    assert_eq!(hkx_member_i64(&body_cinfos[0], "flags"), 0);
    let compressed_meshes = hkx
        .objects()
        .iter()
        .filter(|object| object.class_name == "hknpCompressedMeshShape")
        .collect::<Vec<_>>();
    assert_eq!(
        compressed_meshes.len(),
        1,
        "static wall lamp collision should be one compressed mesh"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_soot_flower_keeps_external_emit_extra_data_valid() {
    let src = repo_path(
        "extracted/fo76/Meshes/landscape/plants/switchnodechildren/florasootflower01_1.nif",
    );
    if !src.exists() {
        eprintln!("skip: FO76 extracted soot-flower fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load source nif");
    assert!(source.blocks.iter().any(|block| {
        block.type_name == "BSXFlags"
            && matches!(block.get_field("Integer Data"), Some(NifValue::UInt(512)))
    }));

    let dir = temp_dir("convert_real_fo76_soot_flower_external_emit");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("FloraSootFlower01_1.nif");
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

    let converted = NifFile::load(dst).expect("load converted nif");
    for block in &converted.blocks {
        let Some(NifValue::Array(extra_refs)) = block.get_field("Extra Data List") else {
            continue;
        };
        assert_eq!(
            block.get_field("Num Extra Data List").map(NifValue::as_i64),
            Some(extra_refs.len() as i64),
            "block {} extra-data count must match its array",
            block.block_id
        );
        assert!(
            extra_refs.iter().all(|value| match value {
                NifValue::Ref(id) => *id >= 0 && (*id as usize) < converted.blocks.len(),
                _ => false,
            }),
            "block {} has an invalid extra-data ref: {:?}",
            block.block_id,
            extra_refs
        );
    }
    assert!(converted.blocks.iter().any(|block| {
        block.type_name == "BSXFlags"
            && matches!(block.get_field("Integer Data"), Some(NifValue::UInt(512)))
    }));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_reported_fo76_protest_sign_keeps_dynamic_collision() {
    let src = repo_path("extracted/fo76/Meshes/setdressing/protestsigns/weapprotestsign007.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted protest-sign fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load source nif");
    let source_blob = embedded_havok_blob(&source).expect("source collision blob");
    let source_meshes = extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(0))
        .expect("source collision preview");
    assert_eq!(
        source_meshes
            .iter()
            .map(|mesh| (
                mesh.shape_type.as_str(),
                mesh.vertices.len(),
                mesh.triangles.len()
            ))
            .collect::<Vec<_>>(),
        vec![("convex_hull", 8, 12), ("convex_hull", 8, 12)]
    );

    let dir = temp_dir("convert_reported_fo76_protest_sign_collision");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("WeapProtestSign007.nif");
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

    let converted = NifFile::load(dst).expect("load converted nif");
    let blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let converted_meshes = extract_preview_meshes_from_blob(&blob, 69.99125, Some(0))
        .expect("converted collision preview");
    assert_same_preview_aabb(&source_meshes, &converted_meshes, 0.001);

    let summary_json = havok_native::api::havok_collision_summary(&blob).expect("summary");
    let summary: serde_json::Value = serde_json::from_str(&summary_json).expect("summary JSON");
    let body = summary
        .get("bodies")
        .and_then(serde_json::Value::as_array)
        .and_then(|bodies| bodies.first())
        .expect("collision body");
    assert_eq!(
        body.get("collision_filter_info")
            .and_then(serde_json::Value::as_i64)
            .map(|filter| filter & 0xFF),
        Some(4),
        "weapon collision must use FO4 CLUTTER"
    );
    assert_ne!(
        body.get("motion_id").and_then(serde_json::Value::as_i64),
        Some(0x7FFF_FFFF),
        "weapon collision must reference a motion cinfo"
    );
    assert!(
        body.get("inverse_mass")
            .and_then(serde_json::Value::as_f64)
            .is_some_and(|mass| mass > 0.0),
        "weapon collision must have finite non-zero mass"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_tincanister_static_compound_does_not_emit_dynamic_compound() {
    let src = repo_path("extracted/fo76/meshes/setdressing/oldtimeassets/tincanister01_empty.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted TinCanister fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load source nif");
    let source_blob = embedded_havok_blob(&source).expect("source collision blob");
    let source_summary =
        havok_native::api::havok_collision_summary(&source_blob).expect("source collision summary");
    assert!(
        source_summary.contains("hknpCompoundShape"),
        "fixture should start as a FO76 compound shape"
    );

    let dir = temp_dir("convert_real_fo76_tincanister_static_compound");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("TinCanister01_Empty.nif");
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

    let converted = NifFile::load(dst).expect("load converted nif");
    let blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let summary_json = havok_native::api::havok_collision_summary(&blob).expect("summary");
    let summary: serde_json::Value = serde_json::from_str(&summary_json).expect("summary JSON");
    let bodies = summary
        .get("bodies")
        .and_then(serde_json::Value::as_array)
        .expect("bodies");
    assert!(
        bodies.iter().all(|body| {
            body.get("shape_class").and_then(serde_json::Value::as_str)
                != Some("hknpDynamicCompoundShape")
        }),
        "static TinCanister collision must not emit hknpDynamicCompoundShape"
    );
    assert_eq!(
        bodies
            .first()
            .and_then(|body| body.get("collision_filter_info"))
            .and_then(serde_json::Value::as_i64)
            .map(|filter| filter & 0xFF),
        Some(1),
        "TinCanister layer 4 should be demoted to FO4 STATIC"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_bscloth_extra_data_blob_preserves_valid_fo4_cloth() {
    let dir = temp_dir("convert_fo76_bscloth_extra_data_blob");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let src = dir.join("source.nif");
    let dst = dir.join("out").join("converted.nif");
    let blob = fixture_bytes("tests/fixtures/cloth/bathrobe_outfitm_cloth.hkx");
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
fn convert_real_fo76_cloth_flair_removes_dependency_graph_and_preserves_cloth() {
    let src = repo_path(
        "extracted/fo76/meshes/atx/backpack_flair/flair_mothmanlogo/atx_mothmanlogo_flair.nif",
    );
    if !src.is_file() {
        eprintln!("skip: FO76 extracted Mothman flair fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_cloth_flair_dependency_graph");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");

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
    let hkx = havok_native::hkx::model::HkxFile::read(&converted_blob).expect("parse cloth blob");
    assert!(
        hkx.objects()
            .iter()
            .all(|object| object.class_name != "hclStateDependencyGraph")
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_film_reel_static_variant_strips_cloth() {
    let src =
        repo_path("extracted/fo76/meshes/atx/backpack_flair/flair_filmreel/atx_filmreel_flair.nif");
    if !src.is_file() {
        eprintln!("skip: FO76 extracted Film Reel flair fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_film_reel_static_variant");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions {
            strip_cloth: true,
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert Film Reel static variant");

    assert!(report.supported, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("stripped 1 BSClothExtraData")),
        "{:?}",
        report.changes
    );
    let converted = NifFile::load(dst).expect("load converted Film Reel static variant");
    assert!(
        converted
            .blocks
            .iter()
            .all(|block| block.type_name != "BSClothExtraData")
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_whitetuxf_preserves_valid_fo4_cloth() {
    let src = repo_path("extracted/fo76/meshes/atx/clothes/whitetux/whitetuxf.nif");
    if !src.is_file() {
        eprintln!("skip: FO76 extracted WhiteTuxF fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_whitetuxf_strips_unsupported_cloth");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");

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

    let converted = NifFile::load(dst).expect("load converted nif");
    assert!(
        converted
            .blocks
            .iter()
            .any(|block| block.type_name == "BSClothExtraData")
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_ranger_outfits_preserve_valid_fo4_cloth() {
    for filename in [
        "atx_rangerarmor_advanced_outfit_m.nif",
        "atx_rangerarmor_advanced_outfit_f.nif",
        "atx_rangerarmor_elite_outfit_m.nif",
        "atx_rangerarmor_elite_outfit_f.nif",
    ] {
        let src = repo_path(&format!(
            "extracted/fo76/meshes/atx/clothes/rangerarmor/{filename}"
        ));
        if !src.is_file() {
            eprintln!("skip: FO76 extracted Ranger outfit fixture not available: {filename}");
            continue;
        }

        let dir = temp_dir(&format!("convert_real_fo76_{filename}"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let dst = dir.join("out").join(filename);
        let report = convert_nif_file(
            &src,
            &dst,
            "fo76",
            "fo4",
            None,
            &ConvertFileOptions::default(),
        )
        .expect("convert Ranger outfit");

        assert!(report.supported, "{:?}", report.errors);
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("converted 1 embedded FO76 cloth blob")),
            "changes={:?}; warnings={:?}",
            report.changes,
            report.warnings
        );
        let converted_bytes = std::fs::read(&dst).expect("read converted Ranger outfit");
        let converted_blob = nif_core_native::cloth::extract_cloth_blob(&converted_bytes)
            .expect("converted Ranger cloth blob");
        let format = havok_native::api::hkx_detect_format_full(&converted_blob).unwrap();
        assert_eq!(format.kind, "packfile");
        assert_eq!(format.version, "hk_2014.1.0-r1");
        let validation = havok_native::api::cloth_validate(&converted_blob)
            .expect("validate converted Ranger cloth");
        let validation: serde_json::Value =
            serde_json::from_str(&validation).expect("cloth validation JSON");
        assert_eq!(
            validation.get("valid").and_then(serde_json::Value::as_bool),
            Some(true),
            "{validation}"
        );

        let cloth = havok_native::hkx::read_packfile(&converted_blob)
            .expect("parse converted Ranger cloth packfile");
        let skin = cloth
            .objects()
            .iter()
            .find(|object| object.class_name == "hclObjectSpaceSkinPNOperator")
            .expect("converted Ranger hclObjectSpaceSkinPNOperator");
        let local_pns = member_value(&skin.members, "localPNs")
            .and_then(|value| match value {
                HkxValue::Array(values) => Some(values),
                _ => None,
            })
            .expect("converted Ranger localPNs");
        assert!(
            !local_pns.is_empty(),
            "{filename}: localPNs must not be empty"
        );
        let mut has_nonzero_packed_value = false;
        for block in local_pns {
            let members = block.as_object_members().expect("localPN block members");
            for component_name in ["localPosition", "localNormal"] {
                let values = member_value(members, component_name)
                    .and_then(|value| match value {
                        HkxValue::Array(values) => Some(values),
                        _ => None,
                    })
                    .expect("packed local component");
                assert_eq!(
                    values.len(),
                    64,
                    "{filename}: {component_name} must contain 64 packed values"
                );
                assert!(
                    values.iter().all(|value| hkx_int(value).is_some()),
                    "{filename}: {component_name} must contain only packed integers"
                );
                has_nonzero_packed_value |= values
                    .iter()
                    .any(|value| hkx_int(value).is_some_and(|value| value != 0));
            }
        }
        assert!(
            has_nonzero_packed_value,
            "{filename}: packed skinning output must preserve authored nonzero values"
        );

        let simulate = cloth
            .objects()
            .iter()
            .find(|object| object.class_name == "hclSimulateOperator")
            .expect("converted Ranger hclSimulateOperator");
        assert_eq!(
            member_value(&simulate.members, "subSteps").and_then(hkx_int),
            Some(6),
            "{filename}: authored cloth substeps must survive conversion"
        );
        assert_eq!(
            member_value(&simulate.members, "numberOfSolveIterations").and_then(hkx_int),
            Some(1),
            "{filename}: authored solve iterations must survive conversion"
        );
        let constraint_execution = member_value(&simulate.members, "constraintExecution")
            .and_then(|value| match value {
                HkxValue::Array(values) => Some(values),
                _ => None,
            })
            .expect("converted Ranger constraintExecution")
            .iter()
            .map(|value| hkx_int(value).expect("constraint index"))
            .collect::<Vec<_>>();
        assert_eq!(
            constraint_execution,
            vec![0, 2, 1, -1],
            "{filename}: authored constraint execution order must survive conversion"
        );

        let sim_cloth = cloth
            .objects()
            .iter()
            .find(|object| object.class_name == "hclSimClothData")
            .expect("converted Ranger hclSimClothData");
        let simulation_info = member_value(&sim_cloth.members, "simulationInfo")
            .and_then(HkxValue::as_object_members)
            .expect("converted Ranger simulationInfo");
        let collision_tolerance = member_value(simulation_info, "collisionTolerance")
            .and_then(|value| match value {
                HkxValue::F32(value) => Some(*value),
                _ => None,
            })
            .expect("converted Ranger collisionTolerance");
        assert!(
            (collision_tolerance - 13.99825).abs() < 0.0001,
            "{filename}: authored collision tolerance must survive conversion; got {collision_tolerance}"
        );
        assert_eq!(
            member_value(simulation_info, "transferMotionEnabled"),
            Some(&HkxValue::Bool(true)),
            "{filename}: transfer motion must survive conversion"
        );

        let buffer_definitions = cloth
            .objects()
            .iter()
            .filter(|object| {
                matches!(
                    object.class_name.as_str(),
                    "hclBufferDefinition" | "hclScratchBufferDefinition"
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            buffer_definitions.len(),
            2,
            "{filename}: expected scratch and simulation buffers"
        );
        for buffer in buffer_definitions {
            let layout = member_value(&buffer.members, "bufferLayout")
                .and_then(HkxValue::as_object_members)
                .expect("converted Ranger bufferLayout");
            let elements = member_value(layout, "elementsLayout")
                .and_then(|value| match value {
                    HkxValue::Array(values) => Some(values),
                    _ => None,
                })
                .expect("converted Ranger elementsLayout");
            let slots = member_value(layout, "slots")
                .and_then(|value| match value {
                    HkxValue::Array(values) => Some(values),
                    _ => None,
                })
                .expect("converted Ranger slots");
            assert_eq!(elements.len(), 4, "{filename}: four buffer elements");
            assert_eq!(slots.len(), 4, "{filename}: four buffer slots");

            let element_values = elements
                .iter()
                .map(|element| {
                    let members = element
                        .as_object_members()
                        .expect("buffer element object members");
                    (
                        member_value(members, "vectorConversion")
                            .and_then(hkx_int)
                            .expect("vectorConversion"),
                        member_value(members, "vectorSize")
                            .and_then(hkx_int)
                            .expect("vectorSize"),
                        member_value(members, "slotId")
                            .and_then(hkx_int)
                            .expect("slotId"),
                        member_value(members, "slotStart")
                            .and_then(hkx_int)
                            .expect("slotStart"),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(
                element_values,
                vec![(0, 16, 0, 0), (0, 16, 1, 0), (250, 0, 0, 0), (250, 0, 0, 0)],
                "{filename}: position/normal buffer layout must survive TAG0 materialization"
            );
            let slot_values = slots
                .iter()
                .map(|slot| {
                    let members = slot
                        .as_object_members()
                        .expect("buffer slot object members");
                    (
                        member_value(members, "flags")
                            .and_then(hkx_int)
                            .expect("slot flags"),
                        member_value(members, "stride")
                            .and_then(hkx_int)
                            .expect("slot stride"),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(
                slot_values,
                vec![(1, 16), (1, 16), (0, 0), (0, 0)],
                "{filename}: aligned buffer slot strides must survive TAG0 materialization"
            );
        }

        let deformer = cloth
            .objects()
            .iter()
            .find_map(|object| {
                member_value(&object.members, "objectSpaceDeformer")
                    .and_then(HkxValue::as_object_members)
            })
            .expect("converted Ranger object-space deformer");
        assert_eq!(
            member_value(deformer, "batchSizeSpu").and_then(hkx_int),
            Some(512),
            "{filename}: FO4 deformer batch size must be restored"
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn convert_real_fo76_collisionless_ranger_ground_models_get_clutter_collision() {
    for filename in [
        "atx_rangerarmor_advanced_go.nif",
        "atx_rangerarmor_standard_helmet_go.nif",
    ] {
        let src = repo_path(&format!(
            "extracted/fo76/meshes/atx/clothes/rangerarmor/{filename}"
        ));
        if !src.is_file() {
            eprintln!("skip: FO76 extracted Ranger ground model not available: {filename}");
            continue;
        }
        let source = NifFile::load(&src).expect("load source ground model");
        assert!(
            source
                .blocks
                .iter()
                .all(|block| block.type_name != "bhkNPCollisionObject"),
            "test fixture must be collisionless"
        );

        let dir = temp_dir(&format!("convert_real_fo76_{filename}"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let dst = dir.join("out").join(filename);
        let report = convert_nif_file(
            &src,
            &dst,
            "fo76",
            "fo4",
            None,
            &ConvertFileOptions::default(),
        )
        .expect("convert Ranger ground model");

        assert!(report.supported, "{:?}", report.errors);
        assert!(
            report.changes.iter().any(|change| change.contains(
                "ground object collision: synthesized 1 dynamic FO4 clutter AABB collision"
            )),
            "changes={:?}; warnings={:?}",
            report.changes,
            report.warnings
        );
        let converted = NifFile::load(&dst).expect("load converted ground model");
        assert!(
            converted
                .blocks
                .iter()
                .any(|block| block.type_name == "bhkNPCollisionObject")
        );
        let bsx = converted
            .blocks
            .iter()
            .find(|block| block.type_name == "BSXFlags")
            .expect("ground model BSXFlags");
        assert_eq!(
            bsx.get_field("Integer Data").map(NifValue::as_i64),
            Some(194)
        );
        let blob = embedded_havok_blob(&converted).expect("ground model collision blob");
        let summary = havok_native::api::havok_collision_summary(&blob).expect("summary");
        let summary: serde_json::Value = serde_json::from_str(&summary).expect("summary JSON");
        let layer = summary
            .get("bodies")
            .and_then(serde_json::Value::as_array)
            .and_then(|bodies| bodies.first())
            .and_then(|body| body.get("collision_filter_info"))
            .and_then(serde_json::Value::as_i64)
            .map(|filter| filter & 0xFF);
        assert_eq!(
            layer,
            Some(4),
            "ground model must use FO4 CLUTTER collision"
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn convert_real_fo76_ranger_elite_ground_model_keeps_source_collision_path() {
    let filename = "atx_rangerarmor_elite_go.nif";
    let src = repo_path(&format!(
        "extracted/fo76/meshes/atx/clothes/rangerarmor/{filename}"
    ));
    if !src.is_file() {
        eprintln!("skip: FO76 extracted Ranger Elite ground model not available");
        return;
    }
    let source = NifFile::load(&src).expect("load source ground model");
    assert!(
        source
            .blocks
            .iter()
            .any(|block| block.type_name == "bhkNPCollisionObject"),
        "Elite control fixture must contain source collision"
    );

    let dir = temp_dir("convert_real_fo76_ranger_elite_go");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join(filename);
    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert Ranger Elite ground model");

    assert!(report.supported, "{:?}", report.errors);
    assert!(
        report
            .changes
            .iter()
            .all(|change| !change.contains("ground object collision: synthesized")),
        "source collision must use the normal conversion path: {:?}",
        report.changes
    );
    let converted = NifFile::load(&dst).expect("load converted ground model");
    assert!(
        converted
            .blocks
            .iter()
            .any(|block| block.type_name == "bhkNPCollisionObject")
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_facegen_strips_unsupported_cloth() {
    let src = repo_path(
        "extracted/fo76/meshes/actors/character/facegendata/facegeom/seventysix.esm/0062f60a.nif",
    );
    if !src.is_file() {
        eprintln!("skip: FO76 extracted Rucker facegen fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_facegen_strips_unsupported_cloth");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");

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
            .any(|warning| warning.contains("FO76 BSClothExtraData")),
        "{:?}",
        report.warnings
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    assert!(
        converted
            .blocks
            .iter()
            .all(|block| block.type_name != "BSClothExtraData")
    );
    for block in &converted.blocks {
        let Some(NifValue::Array(extra_refs)) = block.get_field("Extra Data List") else {
            continue;
        };
        assert!(
            extra_refs.iter().all(|value| match value {
                NifValue::Ref(id) => *id >= 0,
                _ => true,
            }),
            "block {} has a removed extra-data ref: {:?}",
            block.block_id,
            extra_refs
        );
    }

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
fn convert_real_fo76_tree_uses_source_compressed_mesh_collision() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/landscape/trees/mtntopredpinelg01.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted tree fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_tree_collision");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");

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
            .any(|change| change.contains("source-compressed-mesh=1")
                && change.contains("visible-mesh-aabb-fallback=0")
                && change.contains("stripped-unrecoverable=0")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let meshes =
        extract_preview_meshes_from_blob(&blob, 69.99125, Some(0)).expect("preview collision");
    assert_eq!(meshes.len(), 1, "{meshes:?}");
    assert_eq!(meshes[0].shape_type, "compressed_mesh");
    assert!(
        meshes[0].vertices.len() > 8,
        "tree collision should not collapse to an AABB box: {:?}",
        meshes[0]
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_reported_fo76_scol_preserves_collision_bodies_and_materials() {
    let src = repo_path("extracted/fo76/Meshes/SCOL/SeventySix.esm/CM007745B9.NIF");
    if !src.exists() {
        eprintln!("skip: reported FO76 SCOL fixture not available");
        return;
    }

    let dir = temp_dir("convert_reported_fo76_scol");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");
    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert reported SCOL");
    assert!(report.supported, "{:?}", report.errors);
    assert!(
        report.changes.iter().any(|change| change
            .contains("direct-transcoded 2 static physics system")
            && change.contains("preserved 5 bhkNPCollisionObject binding")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted SCOL");
    let mut body_count = 0usize;
    let mut triangle_count = 0usize;
    for blob in embedded_havok_blobs(&converted) {
        let summary_json =
            havok_native::api::havok_collision_summary(&blob).expect("collision summary");
        let summary: serde_json::Value = serde_json::from_str(&summary_json).expect("summary JSON");
        let bodies = summary
            .get("bodies")
            .and_then(serde_json::Value::as_array)
            .expect("summary bodies");
        body_count += bodies.len();
        for (body_index, body) in bodies.iter().enumerate() {
            let materials = body
                .get("bs_materials")
                .and_then(serde_json::Value::as_array)
                .expect("BS material table");
            assert!(
                materials.iter().all(|material| {
                    material
                        .get("material_crc")
                        .and_then(serde_json::Value::as_u64)
                        .is_some_and(|crc| crc != 0)
                }),
                "body {body_index} retained an invalid collision material: {materials:?}"
            );
            triangle_count += extract_preview_meshes_from_blob(&blob, 69.99125, Some(body_index))
                .expect("collision preview")
                .iter()
                .map(|mesh| mesh.triangles.len())
                .sum::<usize>();
        }
    }
    assert_eq!(body_count, 5);
    assert_eq!(triangle_count, 1020);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_reported_fo76_mixed_compounds_preserves_all_children() {
    fn preview_bounds(meshes: &[PreviewMesh]) -> ([f32; 3], [f32; 3]) {
        let mut vertices = meshes.iter().flat_map(|mesh| mesh.vertices.iter());
        let first = *vertices.next().expect("preview vertices");
        vertices.fold((first, first), |(mut min, mut max), vertex| {
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex[axis]);
                max[axis] = max[axis].max(vertex[axis]);
            }
            (min, max)
        })
    }

    for (name, source_relative, source_mesh_count, source_convex_count, source_compressed_count) in [
        (
            "37E6B0-biplane-tail",
            "extracted/fo76/Meshes/setdressing/airplanedebris/airplanedebris_biplane_tail_01b_static.nif",
            8,
            4,
            4,
        ),
        (
            "67BC8D-mascot",
            "extracted/fo76/Meshes/setdressing/nukaworldprops/e09b_defend_bnc_red.nif",
            7,
            6,
            1,
        ),
        (
            "67BC8D-mascot-damaged",
            "extracted/fo76/Meshes/setdressing/nukaworldprops/e09b_defend_bnc_damaged.nif",
            7,
            6,
            1,
        ),
    ] {
        let source_path = repo_path(source_relative);
        if !source_path.exists() {
            eprintln!("skip: reported FO76 fixture not available: {source_relative}");
            continue;
        }

        let source = NifFile::load(&source_path).expect("load source NIF");
        let source_blob = embedded_havok_blob(&source).expect("source collision blob");
        let source_meshes = extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(0))
            .expect("source collision preview");
        assert_eq!(source_meshes.len(), source_mesh_count, "{name}");
        assert_eq!(
            source_meshes
                .iter()
                .filter(|mesh| mesh.shape_type == "convex_hull")
                .count(),
            source_convex_count,
            "{name}"
        );
        assert_eq!(
            source_meshes
                .iter()
                .filter(|mesh| mesh.shape_type == "compressed_mesh")
                .count(),
            source_compressed_count,
            "{name}"
        );
        let source_bounds = preview_bounds(&source_meshes);
        let source_triangle_count = source_meshes
            .iter()
            .map(|mesh| mesh.triangles.len())
            .sum::<usize>();

        let dir = temp_dir(&format!("convert_{name}"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let converted_path = dir.join("converted.nif");
        let report = convert_nif_file(
            &source_path,
            &converted_path,
            "fo76",
            "fo4",
            None,
            &ConvertFileOptions::default(),
        )
        .expect("convert reported collision asset");
        assert!(report.supported, "{name}: {:?}", report.errors);
        assert!(
            report
                .changes
                .iter()
                .any(|change| change.contains("direct-transcoded 1 static physics system")),
            "{name}: {:?}",
            report.changes
        );

        let converted = NifFile::load(converted_path).expect("load converted NIF");
        let converted_blob = embedded_havok_blob(&converted).expect("converted collision blob");
        let converted_meshes = extract_preview_meshes_from_blob(&converted_blob, 69.99125, Some(0))
            .expect("converted collision preview");
        assert_eq!(
            converted_meshes.len(),
            source_mesh_count,
            "{name}: {converted_meshes:?}"
        );
        assert_eq!(
            converted_meshes
                .iter()
                .filter(|mesh| mesh.shape_type == "convex_hull")
                .count(),
            source_convex_count,
            "{name}"
        );
        assert_eq!(
            converted_meshes
                .iter()
                .filter(|mesh| mesh.shape_type == "compressed_mesh")
                .count(),
            source_compressed_count,
            "{name}"
        );
        assert_eq!(
            converted_meshes
                .iter()
                .map(|mesh| mesh.triangles.len())
                .sum::<usize>(),
            source_triangle_count,
            "{name}"
        );
        let converted_bounds = preview_bounds(&converted_meshes);
        for axis in 0..3 {
            for (source_value, converted_value) in [
                (source_bounds.0[axis], converted_bounds.0[axis]),
                (source_bounds.1[axis], converted_bounds.1[axis]),
            ] {
                assert!(
                    (source_value - converted_value).abs() < 0.001,
                    "{name}: collision bounds changed on axis {axis}: source={source_bounds:?}, converted={converted_bounds:?}"
                );
            }
        }

        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn convert_real_fo76_catwalk_preserves_all_compound_collision_children() {
    let src =
        repo_path("extracted/fo76/Meshes/architecture/brickfactory/factory_add_catwalkcor_01.nif");
    if !src.exists() {
        eprintln!("skip: FO76 catwalk fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load FO76 catwalk fixture");
    let source_blob = embedded_havok_blob(&source).expect("source collision blob");
    let source_meshes =
        extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(0)).expect("source preview");
    assert_eq!(source_meshes.len(), 5, "{source_meshes:?}");
    assert_eq!(
        source_meshes
            .iter()
            .filter(|mesh| mesh.shape_type == "convex_hull")
            .count(),
        4
    );
    assert_eq!(
        source_meshes
            .iter()
            .filter(|mesh| mesh.shape_type == "compressed_mesh")
            .count(),
        1
    );

    let dir = temp_dir("convert_real_fo76_catwalk_collision");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");
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
            .any(|change| change.contains("source-compressed-mesh=1")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let converted_blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let converted_meshes = extract_preview_meshes_from_blob(&converted_blob, 69.99125, Some(0))
        .expect("converted preview");
    assert_eq!(converted_meshes.len(), 1, "{converted_meshes:?}");
    assert_eq!(converted_meshes[0].shape_type, "compressed_mesh");
    assert_eq!(converted_meshes[0].vertices.len(), 48);
    assert_eq!(converted_meshes[0].triangles.len(), 72);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_scol_remaps_raw_compressed_mesh_materials() {
    let src = repo_path("extracted/fo76/Meshes/SCOL/SeventySix.esm/CM00051F89.NIF");
    if !src.exists() {
        eprintln!("skip: FO76 extracted SCOL fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_scol_collision_materials");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");

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
            .any(|change| change.contains("source-compressed-mesh=1")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let summary_json = havok_native::api::havok_collision_summary(&blob).expect("summary");
    let summary: serde_json::Value = serde_json::from_str(&summary_json).expect("summary JSON");
    let body = summary
        .get("bodies")
        .and_then(serde_json::Value::as_array)
        .and_then(|bodies| bodies.first())
        .expect("converted body summary");
    let material_crcs = body
        .get("bs_materials")
        .and_then(serde_json::Value::as_array)
        .expect("BS material table")
        .iter()
        .map(|material| {
            material
                .get("material_crc")
                .and_then(serde_json::Value::as_u64)
                .expect("material CRC") as u32
        })
        .collect::<Vec<_>>();

    assert_eq!(
        body.get("material_crc").and_then(serde_json::Value::as_u64),
        Some(u64::from(0x1DD9C611u32))
    );
    assert_eq!(material_crcs, vec![0xBDD69D70, 0x1DD9C611]);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_reported_fo76_wrangler_shelter_uses_direct_static_collision_transcode() {
    let src = repo_path(
        "extracted/fo76/Meshes/setdressing/shelters/shelters_wranglercasino/shelters_wranglercasino_floor1.nif",
    );
    if !src.exists() {
        eprintln!("skip: FO76 Wrangler shelter fixture not available");
        return;
    }

    let dir = temp_dir("convert_reported_fo76_wrangler_shelter_collision");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("converted.nif");
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
        report.changes.iter().any(|change| {
            change.contains("direct-transcoded 1 static physics system")
                && change.contains("preserved 3 bhkNPCollisionObject binding")
        }),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let collisions = converted
        .blocks
        .iter()
        .filter(|block| block.type_name == "bhkNPCollisionObject")
        .collect::<Vec<_>>();
    assert_eq!(collisions.len(), 3);
    let mut body_ids = collisions
        .iter()
        .map(|collision| {
            collision
                .get_field("Body ID")
                .expect("collision Body ID")
                .as_i64()
        })
        .collect::<Vec<_>>();
    body_ids.sort_unstable();
    assert_eq!(body_ids, vec![0, 1, 2]);
    let physics_system_id = collisions[0]
        .get_field("Data")
        .expect("collision Data")
        .as_i64();
    assert!(
        collisions.iter().all(|collision| {
            collision
                .get_field("Data")
                .is_some_and(|data| data.as_i64() == physics_system_id)
        }),
        "Wrangler collision objects must keep their shared physics-system binding"
    );

    let converted_blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let hkx =
        havok_native::hkx::model::HkxFile::read(&converted_blob).expect("parse converted blob");
    assert_eq!(hkx.class_version(), 11);
    assert_eq!(hkx.contents_version(), "hk_2014.1.0-r1");
    assert_eq!(
        hkx.objects()
            .iter()
            .filter(|object| object.class_name == "hknpCompressedMeshShape")
            .count(),
        10
    );
    assert_eq!(
        hkx.objects()
            .iter()
            .filter(|object| object.class_name == "hknpDynamicCompoundShape")
            .count(),
        2
    );
    assert_eq!(
        hkx.objects()
            .iter()
            .filter(|object| object.class_name == "hknpConvexPolytopeShape")
            .count(),
        3
    );

    let triangle_counts = (0..3)
        .map(|body_id| {
            extract_preview_meshes_from_blob(&converted_blob, 69.99125, Some(body_id))
                .expect("converted preview")
                .iter()
                .map(|mesh| mesh.triangles.len())
                .sum::<usize>()
        })
        .collect::<Vec<_>>();
    assert_eq!(triangle_counts, vec![695, 3_398, 359]);
    let (motion_ids, motion_cinfo_count) = physics_motion_ids(&converted_blob);
    assert_eq!(motion_ids, vec![0x7FFF_FFFF; 3]);
    assert_eq!(motion_cinfo_count, 0);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_reported_fo76_haunted_bell_tower_preserves_stair_collision_graph() {
    let src = repo_path(
        "extracted/fo76/Meshes/atx/architecture/prefabs/atx_haunted_belltower/ATX_Haunted_Belltower_BellwithButton.nif",
    );
    if !src.exists() {
        eprintln!("skip: FO76 Haunted Bell Tower fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load FO76 Haunted Bell Tower fixture");
    let source_blob = embedded_havok_blob(&source).expect("source collision blob");
    let source_meshes = (0..11)
        .flat_map(|body_id| {
            extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(body_id))
                .expect("source preview")
        })
        .collect::<Vec<_>>();

    let dir = temp_dir("convert_reported_fo76_haunted_bell_tower_collision");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("converted.nif");
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
        report.changes.iter().any(|change| {
            change.contains("direct-transcoded 1 static physics system")
                && change.contains("preserved 11 bhkNPCollisionObject binding")
        }),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let collisions = converted
        .blocks
        .iter()
        .filter(|block| block.type_name == "bhkNPCollisionObject")
        .collect::<Vec<_>>();
    assert_eq!(collisions.len(), 11);
    let stair_collision = collisions
        .iter()
        .find(|collision| {
            collision
                .get_field("Body ID")
                .is_some_and(|value| value.as_i64() == 2)
        })
        .expect("stair collision binding");
    let stair_target_id = stair_collision
        .get_field("Target")
        .expect("stair collision target")
        .as_i64() as usize;
    assert_eq!(
        converted
            .get_block(stair_target_id)
            .and_then(|target| target.get_field("Name"))
            .and_then(|name| match name {
                NifValue::String(name) => Some(name.as_str()),
                _ => None,
            }),
        Some("Stairs")
    );

    let converted_blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let hkx =
        havok_native::hkx::model::HkxFile::read(&converted_blob).expect("parse converted blob");
    assert_eq!(
        hkx.objects()
            .iter()
            .filter(|object| object.class_name == "hknpDynamicCompoundShape")
            .count(),
        6
    );
    assert_eq!(
        hkx.objects()
            .iter()
            .filter(|object| object.class_name == "hknpConvexPolytopeShape")
            .count(),
        14
    );
    assert_eq!(
        hkx.objects()
            .iter()
            .filter(|object| object.class_name == "hknpCapsuleShape")
            .count(),
        2
    );

    let converted_meshes = (0..11)
        .flat_map(|body_id| {
            extract_preview_meshes_from_blob(&converted_blob, 69.99125, Some(body_id))
                .expect("converted preview")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        converted_meshes
            .iter()
            .map(|mesh| mesh.triangles.len())
            .sum::<usize>(),
        source_meshes
            .iter()
            .map(|mesh| mesh.triangles.len())
            .sum::<usize>()
    );
    assert_same_preview_aabb(&source_meshes, &converted_meshes, 0.001);
    let (motion_ids, motion_cinfo_count) = physics_motion_ids(&converted_blob);
    assert_eq!(motion_ids, vec![0x7FFF_FFFF; 11]);
    assert_eq!(motion_cinfo_count, 0);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_reported_fo76_restricted_area_main_floor_keeps_static_wall_bodies() {
    let src = repo_path(
        "extracted/fo76/Meshes/setdressing/shelters/Shelters_RestrictedArea/Shelters_RestrictedArea_MainFloor.nif",
    );
    if !src.exists() {
        eprintln!("skip: FO76 restricted-area main-floor fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load FO76 restricted-area main-floor fixture");
    let source_blob = embedded_havok_blob(&source).expect("source collision blob");
    let mut source_meshes = Vec::new();
    for body_id in 0..7 {
        let body_meshes = extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(body_id))
            .expect("source preview");
        source_meshes.extend(body_meshes);
    }
    assert_eq!(source_meshes.len(), 47);
    assert_eq!(
        (0..7)
            .map(|body_id| {
                extract_source_polytopes_from_blob(&source_blob, body_id)
                    .expect("source polytopes")
                    .len()
            })
            .collect::<Vec<_>>(),
        vec![7, 0, 1, 1, 1, 1, 20]
    );
    let source_triangle_count = source_meshes
        .iter()
        .map(|mesh| mesh.triangles.len())
        .sum::<usize>();
    assert_eq!(source_triangle_count, 6169);

    let dir = temp_dir("convert_reported_fo76_restricted_area_main_floor_collision");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("converted.nif");
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
        report.changes.iter().any(|change| {
            change.contains("direct-transcoded 1 static physics system")
                && change.contains("preserved 7 bhkNPCollisionObject binding")
        }),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let mut converted_bindings = converted
        .blocks
        .iter()
        .filter(|block| block.type_name == "bhkNPCollisionObject")
        .map(|collision| {
            let body_id = collision
                .get_field("Body ID")
                .expect("collision Body ID")
                .as_i64();
            let target_id = collision
                .get_field("Target")
                .expect("collision Target")
                .as_i64() as usize;
            let target_name = converted
                .get_block(target_id)
                .and_then(|target| target.get_field("Name"))
                .and_then(|name| match name {
                    NifValue::String(name) => Some(name.as_str()),
                    _ => None,
                })
                .unwrap_or_default()
                .to_string();
            (body_id, target_name)
        })
        .collect::<Vec<_>>();
    converted_bindings.sort_by_key(|(body_id, _)| *body_id);
    assert_eq!(
        converted_bindings,
        vec![
            (
                0,
                "Shelters_RestrictedArea_MainFloor_Scaffolding".to_string()
            ),
            (1, "Shelters_RestrictedArea_MainFloor_Concrete".to_string()),
            (2, "C_Stairhelper00".to_string()),
            (3, "C_Stairhelper03".to_string()),
            (4, "C_Stairhelper02".to_string()),
            (5, "C_Stairhelper01".to_string()),
            (6, "Shelters_RestrictedArea_MainFloor_Metal".to_string()),
        ]
    );

    let converted_blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let summary_json =
        havok_native::api::havok_collision_summary(&converted_blob).expect("collision summary");
    let summary: serde_json::Value = serde_json::from_str(&summary_json).expect("summary JSON");
    let bodies = summary
        .get("bodies")
        .and_then(serde_json::Value::as_array)
        .expect("summary bodies");
    assert_eq!(bodies.len(), 7);
    assert_eq!(
        bodies
            .iter()
            .filter(|body| {
                body.get("layer").and_then(serde_json::Value::as_u64) == Some(31)
                    && body.get("shape_class").and_then(serde_json::Value::as_str)
                        == Some("hknpConvexPolytopeShape")
            })
            .count(),
        4,
        "{bodies:?}"
    );
    assert_eq!(
        bodies
            .iter()
            .filter(|body| {
                body.get("shape_class").and_then(serde_json::Value::as_str)
                    == Some("hknpCompressedMeshShape")
            })
            .count(),
        0,
        "{bodies:?}"
    );
    assert_eq!(
        bodies
            .iter()
            .filter(|body| {
                body.get("shape_class").and_then(serde_json::Value::as_str)
                    == Some("hknpDynamicCompoundShape")
            })
            .count(),
        3,
        "{bodies:?}"
    );

    let (motion_ids, motion_cinfo_count) = physics_motion_ids(&converted_blob);
    assert!(
        motion_ids.iter().all(|motion_id| *motion_id == 0x7FFF_FFFF),
        "static wall bodies must not link live motion frames: {motion_ids:?}"
    );
    assert_eq!(motion_cinfo_count, 0);

    let mut converted_meshes = Vec::new();
    for body_id in 0..7 {
        let body_meshes =
            extract_preview_meshes_from_blob(&converted_blob, 69.99125, Some(body_id))
                .expect("converted preview");
        converted_meshes.extend(body_meshes);
    }
    assert_eq!(
        converted_meshes
            .iter()
            .map(|mesh| mesh.triangles.len())
            .sum::<usize>(),
        source_triangle_count
    );
    assert_same_preview_aabb(&source_meshes, &converted_meshes, 0.25);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_verified_fo76_statics_use_direct_collision_transcode() {
    let fixtures: [(&str, &str, &[&str]); 4] = [
        (
            "extracted/fo76/Meshes/vehicles/dragline/vehicle_dragline.nif",
            "convert_reported_fo76_dragline_collision",
            &["Vehicle_Dragline", "StairRamp"],
        ),
        (
            "extracted/fo76/Meshes/setdressing/guntherswildwestshowprops/gwws_entrancesign.nif",
            "convert_reported_fo76_entrance_sign_collision",
            &["GWWS_EntranceSign"],
        ),
        (
            "extracted/fo76/Meshes/dlc04/setdressing/vendorcart/vendorcart_04.nif",
            "convert_reported_fo76_vendor_cart_collision",
            &["VendorCart_04"],
        ),
        (
            "extracted/fo76/Meshes/Architecture/RangerLookoutTower/RangerTowerBase03.nif",
            "convert_reported_fo76_ranger_tower_collision",
            &[
                "RangerTowerBaseA3Floors01",
                "RangerTowerBaseA3",
                "StairsHelper06",
                "StairsHelper05",
                "StairsHelper006",
                "StairsHelper04",
                "StairsHelper002",
                "StairsHelper003",
                "StairsHelper001",
            ],
        ),
    ];

    for (relative_path, temp_name, expected_targets) in fixtures {
        let src = repo_path(relative_path);
        if !src.exists() {
            eprintln!("skip: FO76 static fixture not available: {relative_path}");
            continue;
        }

        let body_count = expected_targets.len();
        let source = NifFile::load(&src).expect("load FO76 static fixture");
        let source_blob = embedded_havok_blob(&source).expect("source collision blob");
        let source_meshes = (0..body_count)
            .flat_map(|body_id| {
                extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(body_id))
                    .expect("source preview")
            })
            .collect::<Vec<_>>();

        let dir = temp_dir(temp_name);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let dst = dir.join("converted.nif");
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
            report.changes.iter().any(|change| {
                change.contains("direct-transcoded 1 static physics system")
                    && change.contains(&format!(
                        "preserved {body_count} bhkNPCollisionObject binding"
                    ))
            }),
            "{:?}",
            report.changes
        );

        let converted = NifFile::load(dst).expect("load converted nif");
        let mut converted_bindings = converted
            .blocks
            .iter()
            .filter(|block| block.type_name == "bhkNPCollisionObject")
            .map(|collision| {
                let body_id = collision
                    .get_field("Body ID")
                    .expect("collision Body ID")
                    .as_i64();
                let target_id = collision
                    .get_field("Target")
                    .expect("collision Target")
                    .as_i64() as usize;
                let target_name = converted
                    .get_block(target_id)
                    .and_then(|target| target.get_field("Name"))
                    .and_then(|name| match name {
                        NifValue::String(name) => Some(name.as_str()),
                        _ => None,
                    })
                    .unwrap_or_default()
                    .to_string();
                (body_id, target_name)
            })
            .collect::<Vec<_>>();
        converted_bindings.sort_by_key(|(body_id, _)| *body_id);
        assert_eq!(
            converted_bindings,
            expected_targets
                .iter()
                .enumerate()
                .map(|(body_id, target)| (body_id as i64, (*target).to_string()))
                .collect::<Vec<_>>()
        );

        let converted_blob = embedded_havok_blob(&converted).expect("converted collision blob");
        let converted_meshes = (0..body_count)
            .flat_map(|body_id| {
                extract_preview_meshes_from_blob(&converted_blob, 69.99125, Some(body_id))
                    .expect("converted preview")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            converted_meshes
                .iter()
                .map(|mesh| mesh.triangles.len())
                .sum::<usize>(),
            source_meshes
                .iter()
                .map(|mesh| mesh.triangles.len())
                .sum::<usize>()
        );
        assert_same_preview_aabb(&source_meshes, &converted_meshes, 0.001);

        let (motion_ids, motion_cinfo_count) = physics_motion_ids(&converted_blob);
        assert_eq!(motion_ids, vec![0x7FFF_FFFF; body_count]);
        assert_eq!(motion_cinfo_count, 0);

        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn convert_reported_fo76_golf_cart_uses_direct_collision_with_fo4_instance_metadata() {
    let src = repo_path("extracted/fo76/Meshes/vehicles/golf_cart/vehicle_golf_cart01.nif");
    if !src.exists() {
        eprintln!("skip: FO76 golf-cart fixture not available");
        return;
    }

    let dir = temp_dir("convert_reported_fo76_golf_cart_collision");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("converted.nif");
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
            .any(|change| change.contains("direct-transcoded 1 static physics system")),
        "golf-cart collision must use the structurally eligible direct route: {:?}",
        report.changes
    );
    assert!(
        !report
            .changes
            .iter()
            .any(|change| change.contains("source-compound=1")),
        "golf-cart collision must not fall through to reconstruction: {:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let converted_blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let hkx =
        havok_native::hkx::model::HkxFile::read(&converted_blob).expect("parse converted blob");
    assert_eq!(
        hkx.objects()
            .iter()
            .filter(|object| object.class_name == "hknpDynamicCompoundShape")
            .count(),
        1
    );
    assert_eq!(
        hkx.objects()
            .iter()
            .filter(|object| object.class_name == "hknpConvexPolytopeShape")
            .count(),
        20
    );
    let compound = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpDynamicCompoundShape")
        .expect("golf-cart compound");
    assert_fo4_compound_instance_metadata(&hkx, compound, 20);

    let (motion_ids, motion_cinfo_count) = physics_motion_ids(&converted_blob);
    assert_eq!(motion_ids, vec![0x7FFF_FFFF]);
    assert_eq!(motion_cinfo_count, 0);

    let _ = std::fs::remove_dir_all(dir);
}

fn assert_fo4_compound_instance_metadata(
    hkx: &havok_native::hkx::model::HkxFile,
    shape: &havok_native::hkx::model::HkxObject,
    expected_instances: usize,
) {
    let instances_container = member_value(&shape.members, "instances")
        .and_then(HkxValue::as_object_members)
        .expect("compound instances");
    assert_eq!(
        member_value(instances_container, "firstFree").and_then(hkx_int),
        Some(-1),
        "used instance slots must not form a free list"
    );
    let instances = member_value(instances_container, "elements")
        .and_then(|value| match value {
            HkxValue::Array(values) => Some(values),
            _ => None,
        })
        .expect("compound instance elements");
    assert_eq!(instances.len(), expected_instances);

    let data_index = member_value(&shape.members, "boundingVolumeData")
        .and_then(|value| match value {
            HkxValue::Pointer(Some(index)) => Some(*index),
            _ => None,
        })
        .expect("compound data pointer");
    let data = &hkx.objects()[data_index];
    let tree = member_value(&data.members, "aabbTree")
        .and_then(HkxValue::as_object_members)
        .expect("compound AABB tree");
    let nodes = member_value(tree, "nodes")
        .and_then(|value| match value {
            HkxValue::Array(values) => Some(values),
            _ => None,
        })
        .expect("compound AABB tree nodes");
    assert_eq!(
        member_value(tree, "numLeaves").and_then(hkx_int),
        Some(expected_instances as i64)
    );

    for (instance_index, instance) in instances.iter().enumerate() {
        let members = instance.as_object_members().expect("shape instance");
        let transform = member_value(members, "transform")
            .and_then(|value| match value {
                HkxValue::F32List(values) => Some(values),
                _ => None,
            })
            .expect("instance transform");
        assert_eq!(transform.len(), 16);
        assert_eq!(transform[3].to_bits() & 0xff00_0000, 0x3f00_0000);
        assert_ne!(transform[3].to_bits() & 0x40, 0);
        assert_eq!(transform[7].to_bits(), 0);
        assert_eq!(transform[11].to_bits() & 0xff00_0000, 0x3f00_0000);
        assert_ne!(
            transform[11].to_bits() & 0x00ff_ffff,
            0,
            "FO4 child shape size must be populated"
        );

        let leaf_index = (transform[15].to_bits() & 0x00ff_ffff) as usize;
        let leaf_aabb = nodes[leaf_index]
            .as_object_members()
            .and_then(|members| member_value(members, "aabb"))
            .and_then(HkxValue::as_object_members)
            .expect("tree node AABB");
        let leaf_max = member_value(leaf_aabb, "max")
            .and_then(|value| match value {
                HkxValue::F32List(values) => Some(values),
                _ => None,
            })
            .expect("tree node max");
        let leaf_data = leaf_max[3].to_bits();
        assert_eq!(leaf_data & 0xffff, 0);
        assert_eq!(leaf_data >> 16, instance_index as u32);
    }
}

#[test]
fn convert_reported_fo76_root_cellar_stairs_keeps_mixed_collision_children() {
    let src = repo_path(
        "extracted/fo76/Meshes/setdressing/shelters/Shelters_RootCellar/Shelter_RootCellar_Stairs.nif",
    );
    if !src.exists() {
        eprintln!("skip: FO76 root-cellar stairs fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load FO76 root-cellar stairs fixture");
    let source_blob = embedded_havok_blob(&source).expect("source collision blob");
    let source_meshes =
        extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(0)).expect("source preview");
    assert_eq!(
        source_meshes
            .iter()
            .map(|mesh| (
                mesh.shape_type.as_str(),
                mesh.vertices.len(),
                mesh.triangles.len()
            ))
            .collect::<Vec<_>>(),
        vec![("convex_hull", 8, 12), ("compressed_mesh", 56, 108)]
    );
    assert_eq!(
        extract_source_polytopes_from_blob(&source_blob, 0)
            .expect("source polytopes")
            .len(),
        1
    );

    let dir = temp_dir("convert_reported_fo76_root_cellar_stairs_collision");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("converted.nif");
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
            .any(|change| change.contains("direct-transcoded 1 static physics system")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let converted_blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let converted_meshes = extract_preview_meshes_from_blob(&converted_blob, 69.99125, Some(0))
        .expect("converted preview");
    assert_eq!(
        converted_meshes
            .iter()
            .map(|mesh| (
                mesh.shape_type.as_str(),
                mesh.vertices.len(),
                mesh.triangles.len()
            ))
            .collect::<Vec<_>>(),
        vec![("convex_hull", 8, 12), ("compressed_mesh", 56, 108)]
    );
    assert_same_preview_aabb(&source_meshes, &converted_meshes, 0.001);

    let (motion_ids, motion_cinfo_count) = physics_motion_ids(&converted_blob);
    assert_eq!(motion_ids, vec![0x7FFF_FFFF]);
    assert_eq!(motion_cinfo_count, 0);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_county_sign_restores_decal_shader_flags() {
    let src = repo_path("extracted/fo76/Meshes/SCOL/SeventySix.esm/CM0034C71E.NIF");
    let source_material_dir = repo_path("extracted/fo76");
    if !src.exists() {
        eprintln!("skip: FO76 extracted county-sign fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_county_sign_decal");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");
    let options = ConvertFileOptions {
        source_material_dir: Some(source_material_dir),
        ..ConvertFileOptions::default()
    };

    let report = convert_nif_file(&src, &dst, "fo76", "fo4", None, &options).expect("convert nif");
    assert!(report.supported, "{:?}", report.errors);

    let converted = NifFile::load(dst).expect("load converted nif");
    let shader = converted
        .blocks
        .iter()
        .find(|block| {
            block.type_name == "BSLightingShaderProperty"
                && matches!(
                    block.get_field("Name"),
                    Some(NifValue::String(path))
                        if path.eq_ignore_ascii_case(
                            r"Materials\SetDressing\Signage\HighwaySignLetters_Black.BGSM"
                        )
                )
        })
        .expect("county-sign lettering shader");
    let flags = shader
        .get_field("Shader Flags 1")
        .map(NifValue::as_i64)
        .unwrap_or_default();
    assert_ne!(flags & (1 << 26), 0, "decal flag missing: {flags:#x}");
    assert_ne!(
        flags & (1 << 27),
        0,
        "dynamic decal flag missing: {flags:#x}"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_multi_compressed_collision_uses_shared_blob() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/SCOL/SeventySix.esm/CM0000F6FD.NIF");
    if !src.exists() {
        eprintln!("skip: FO76 extracted SCOL fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_multi_compressed_collision");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");

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
            .any(|change| change.contains("regenerated 2 FO4 collision object")),
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
    let blobs = embedded_havok_blobs(&converted);
    assert_eq!(blobs.len(), 1);
    for blob in blobs {
        let meshes =
            extract_preview_meshes_from_blob(&blob, 69.99125, Some(0)).expect("preview collision");
        assert!(!meshes.is_empty(), "{meshes:?}");
    }

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_scaled_convex_body_does_not_use_aabb_fallback() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/SCOL/SeventySix.esm/CM0058DD26.NIF");
    if !src.exists() {
        eprintln!("skip: FO76 extracted scaled-convex SCOL fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_scaled_convex_collision");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");

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
            .all(|warning| !warning.contains("minimal AABB fallback")),
        "{:?}",
        report.warnings
    );
    assert!(
        report
            .changes
            .iter()
            .any(|change| change.contains("visible-mesh-aabb-fallback=0")
                && change.contains("stripped-unrecoverable=0")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    let mut previewed = 0usize;
    for block in converted
        .blocks
        .iter()
        .filter(|block| block.type_name == "bhkNPCollisionObject")
    {
        let data_ref = match block.get_field("Data") {
            Some(NifValue::Ref(value)) if *value >= 0 => *value as usize,
            Some(NifValue::Int(value)) if *value >= 0 => *value as usize,
            Some(NifValue::UInt(value)) => *value as usize,
            _ => continue,
        };
        let body_id = match block.get_field("Body ID") {
            Some(NifValue::Int(value)) if *value >= 0 => *value as usize,
            Some(NifValue::UInt(value)) => *value as usize,
            Some(NifValue::Ref(value)) if *value >= 0 => *value as usize,
            _ => 0,
        };
        let physics = converted.get_block(data_ref).expect("collision physics");
        let Some(NifValue::Struct(binary_data)) = physics.get_field("Binary Data") else {
            panic!("missing binary data for collision block {}", block.block_id);
        };
        let Some(data) = binary_data.get("Data") else {
            panic!("missing blob data for collision block {}", block.block_id);
        };
        let blob = match data {
            NifValue::Bytes(bytes) => bytes.clone(),
            NifValue::Array(values) => values.iter().map(|value| value.as_i64() as u8).collect(),
            _ => Vec::new(),
        };
        let meshes = extract_preview_meshes_from_blob(&blob, 69.99125, Some(body_id))
            .expect("preview collision");
        assert!(
            !meshes.is_empty(),
            "collision block {} body {body_id} should be previewable",
            block.block_id
        );
        previewed += 1;
    }
    assert_eq!(previewed, 5);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_dirtcliff_prunes_temp_ground_decal_overlay() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/landscape/dirtcliffs/ECliffCurved02.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted dirtcliff fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_dirtcliff_temp_decal");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");

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
            .any(|change| change.contains("ECliffCurved02:1")),
        "{:?}",
        report.changes
    );

    let converted = NifFile::load(dst).expect("load converted nif");
    assert!(converted.blocks.iter().any(|block| {
        block.type_name == "BSTriShape"
            && matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "ECliffCurved02:0")
    }));
    assert!(!converted.blocks.iter().any(|block| {
        block.type_name == "BSTriShape"
            && matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "ECliffCurved02:1")
    }));
    assert!(!converted.blocks.iter().any(|block| {
        matches!(
            block.get_field("Name"),
            Some(NifValue::String(name))
                if name.to_ascii_lowercase().contains("temp_groundtexture01decal")
        )
    }));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_previously_invalid_compressed_collision_preserves_mesh() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/SCOL/SeventySix.esm/CM00112FDC.NIF");
    if !src.exists() {
        eprintln!("skip: FO76 extracted SCOL fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_invalid_compressed_collision");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");

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
            .all(|warning| !warning.contains("triangle")),
        "{:?}",
        report.warnings
    );
    let converted = NifFile::load(dst).expect("load converted nif");
    let blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let meshes =
        extract_preview_meshes_from_blob(&blob, 69.99125, Some(0)).expect("preview collision");
    assert_eq!(meshes.len(), 1, "{meshes:?}");
    assert_eq!(meshes[0].shape_type, "compressed_mesh");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_compound_mesh_collision_preserves_compressed_children() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extracted/fo76/meshes/Architecture/BLDKIT/Siding/Roof/BLD_Siding_RoofResortC_WallPeak_01.nif");
    if !src.exists() {
        eprintln!("skip: FO76 extracted roof fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_compound_mesh_collision");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("converted.nif");

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
            .all(|warning| !warning.contains("MaterialA")),
        "{:?}",
        report.warnings
    );
    let converted = NifFile::load(&dst).expect("load converted nif");
    assert!(
        converted
            .blocks
            .iter()
            .any(|block| block.type_name == "bhkNPCollisionObject")
    );

    // Source is a compound of two concave compressed meshes; the converter merges
    // them into a single compressed mesh rather than collapsing each to a convex
    // hull.
    let converted_blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let converted_preview = extract_preview_meshes_from_blob(&converted_blob, 69.99125, Some(0))
        .expect("converted collision preview");
    assert!(!converted_preview.is_empty(), "{converted_preview:?}");
    assert!(
        converted_preview
            .iter()
            .all(|mesh| mesh.shape_type == "compressed_mesh"),
        "roof collision must stay compressed mesh, not convex hull: {:?}",
        converted_preview
            .iter()
            .map(|mesh| mesh.shape_type.clone())
            .collect::<Vec<_>>()
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_vault76_railing_merges_concave_bars_into_sectioned_mesh() {
    // Vault 76 railing source = hknpCompoundShape of two concave
    // hknpCompressedMeshShape children (thin bars). The converter merges them into
    // a single multi-section hknpCompressedMeshShape so the collision follows the
    // bars; collapsing them to convex hulls "fills in" the railing into a solid
    // slab, and a compound of separate CM children crashes FO4's narrowphase.
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../../extracted/fo76/meshes/hardscape/railings/hard_railinga_halfcirc_02_vault76.nif",
    );
    if !src.exists() {
        eprintln!("skip: FO76 extracted railing fixture not available");
        return;
    }

    let source = NifFile::load(&src).expect("load source nif");
    let source_blob = embedded_havok_blob(&source).expect("source collision blob");
    let source_preview = extract_preview_meshes_from_blob(&source_blob, 69.99125, Some(0))
        .expect("source collision preview");
    assert!(source_preview.len() >= 2, "{source_preview:?}");
    assert!(
        source_preview
            .iter()
            .all(|mesh| mesh.shape_type == "compressed_mesh"),
        "source railing children should be compressed meshes: {:?}",
        source_preview
            .iter()
            .map(|mesh| mesh.shape_type.clone())
            .collect::<Vec<_>>()
    );
    let source_triangles = valid_compressed_triangle_count(&source_preview);

    let dir = temp_dir("convert_fo76_vault76_railing");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir
        .join("out")
        .join("Hard_RailingA_HalfCirC_02_VAULT76.nif");
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

    let converted = NifFile::load(&dst).expect("load converted nif");
    let converted_blob = embedded_havok_blob(&converted).expect("converted collision blob");
    let converted_preview = extract_preview_meshes_from_blob(&converted_blob, 69.99125, Some(0))
        .expect("converted collision preview");
    // The two concave bars merge into ONE multi-section hknpCompressedMeshShape —
    // the vanilla representation for multi-part concave static collision.
    assert_eq!(
        converted_preview.len(),
        1,
        "railing must convert to a single merged compressed mesh: {:?}",
        converted_preview
            .iter()
            .map(|mesh| mesh.shape_type.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(converted_preview[0].shape_type, "compressed_mesh");
    assert_eq!(
        valid_compressed_triangle_count(&converted_preview),
        source_triangles,
        "railing triangle count must be preserved through the merge"
    );

    // Engine-safety guard: every section must stay within the codec limits the
    // FO4 narrowphase assumes (<=128 triangles, <=255 vertices). The pre-fix
    // compound writer crammed all triangles into one oversized section and the
    // narrowphase read past the section node array (CTD).
    let parsed = parse_fo4_compressed_mesh(&converted_blob).expect("parse converted CM");
    assert!(
        parsed.sections.len() > 1,
        "merged railing (> 128 triangles) must split into multiple sections"
    );
    for section in &parsed.sections {
        assert!(
            section.triangles.len() <= 128,
            "section has {} triangles (> 128 codec max)",
            section.triangles.len()
        );
        assert!(
            section.vertices.len() <= 255,
            "section has {} vertices (> 255 codec max)",
            section.vertices.len()
        );
    }

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
fn convert_fo76_static_component_clears_scol_only_root_flag() {
    let src = repo_root().join("extracted/fo76/Meshes/SetDressing/Signage/SignStreet76_Post01.nif");
    if !src.exists() {
        eprintln!("skipping missing fixture {}", src.display());
        return;
    }
    let dir = temp_dir("convert_fo76_static_component_root_flags");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("SignStreet76_Post01.nif");

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

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_fo76_civwar_door_fixture_normalizes_root_flags_when_available() {
    let src = repo_root().join("extracted/fo76/Meshes/Architecture/CivilWarForts/CivWarDoor01.nif");
    if !src.exists() {
        eprintln!("skipping missing fixture {}", src.display());
        return;
    }
    let dir = temp_dir("convert_fo76_civwar_door");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("CivWarDoor01.nif");

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
    assert_eq!(
        converted.blocks[0]
            .get_field("Controller")
            .map(NifValue::as_i64),
        Some(2)
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
fn convert_real_fnv_hatchet_to_fo4_melee_when_available() {
    let source = repo_root().join("extracted/fnv/meshes/weapons/1handmelee/Hatchet.NIF");
    if !source.is_file() {
        eprintln!("skipping missing FNV Hatchet fixture {}", source.display());
        return;
    }

    let dir = temp_dir("convert_fnv_hatchet_melee");
    let destination = dir.join("data/Meshes/Weapons/1HandMelee/Hatchet.nif");
    let report = convert_nif_file(
        &source,
        &destination,
        "fnv",
        "fo4",
        None,
        &ConvertFileOptions {
            asset_prefix: Some("FNV".to_string()),
            weapon_role: Some("melee".to_string()),
            skin_policy: LegacySkinPolicy::PreserveSourceRig,
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert FNV Hatchet");

    assert!(report.supported, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let converted = NifFile::load(&destination).expect("load converted FNV Hatchet");
    assert_eq!(converted.header.version, (20, 2, 0, 7));
    assert_eq!(converted.header.user_version, 12);
    assert_eq!(converted.header.bs_version, 130);
    assert_eq!(converted.blocks[0].type_name, "NiNode");

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
    let root_marker_ids = root_extra_ids
        .iter()
        .copied()
        .filter(|id| {
            converted.get_block(*id).is_some_and(|block| {
                block.type_name == "NiStringExtraData"
                    && matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "Prn")
                    && matches!(block.get_field("String Data"), Some(NifValue::String(value)) if value == "WEAPON")
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(root_marker_ids.len(), 1);
    assert!(root_extra_ids.iter().copied().all(|id| {
        converted.get_block(id).is_none_or(|block| {
            block.type_name != "NiStringExtraData"
                || (!matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "WEAPON")
                    && !matches!(block.get_field("String Data"), Some(NifValue::String(value)) if value == "WeaponBack"))
        })
    }));
    let root_marker_id = root_marker_ids[0];
    let marker_parents = converted
        .blocks
        .iter()
        .filter(|block| extra_ids(block).contains(&root_marker_id))
        .map(|block| block.block_id)
        .collect::<Vec<_>>();
    assert_eq!(marker_parents, vec![0]);

    assert!(converted.blocks.iter().all(|block| !matches!(
        block.type_name.as_str(),
        "NiTriStrips"
            | "NiTriStripsData"
            | "BSShaderPPLightingProperty"
            | "NiMaterialProperty"
            | "BSDecalPlacementVectorExtraData"
            | "bhkCollisionObject"
            | "bhkRigidBody"
            | "bhkRigidBodyT"
            | "bhkConvexVerticesShape"
    )));
    assert!(converted.blocks.iter().all(|block| {
        !matches!(
            block.get_field("Name"),
            Some(NifValue::String(name)) if name.starts_with("DecalPlacementVector")
        )
    }));

    let mut shaders = 0usize;
    let mut texture_paths = Vec::new();
    for shader in converted
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSLightingShaderProperty")
    {
        shaders += 1;
        let texture_set_id = match shader.get_field("Texture Set") {
            Some(NifValue::Ref(id)) if *id >= 0 => *id as usize,
            other => panic!("Hatchet shader texture-set ref is {other:?}"),
        };
        let texture_set = converted
            .get_block(texture_set_id)
            .filter(|block| block.type_name == "BSShaderTextureSet")
            .expect("Hatchet shader resolves to a texture set");
        if let Some(NifValue::Array(textures)) = texture_set.get_field("Textures") {
            texture_paths.extend(textures.iter().filter_map(|texture| match texture {
                NifValue::String(path) if !path.is_empty() => Some(path.clone()),
                _ => None,
            }));
        }
    }
    assert!(shaders > 0);
    assert!(!texture_paths.is_empty());
    for output_path in texture_paths {
        let canonical = output_path.replace('/', "\\");
        assert!(canonical.to_ascii_lowercase().starts_with("textures\\"));
        assert!(!canonical.contains(".."));
        let source_relative = canonical
            .strip_prefix("Textures\\FNV\\")
            .or_else(|| canonical.strip_prefix("textures\\FNV\\"))
            .map(|relative| format!("textures\\{relative}"))
            .unwrap_or(canonical);
        assert!(
            repo_root()
                .join("extracted/fnv")
                .join(source_relative)
                .is_file(),
            "converted texture path does not resolve to source: {output_path}"
        );
    }

    let has_np_collision = converted
        .blocks
        .iter()
        .any(|block| block.type_name == "bhkNPCollisionObject");
    assert!(has_np_collision);
    let bsx_flags = converted
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSXFlags")
        .filter_map(|block| block.get_field("Integer Data"))
        .map(NifValue::as_i64)
        .collect::<Vec<_>>();
    assert!(
        converted
            .blocks
            .iter()
            .any(|block| block.type_name == "bhkPhysicsSystem")
    );
    assert!(bsx_flags.iter().any(|flags| flags & 2 != 0));
    let collision_blob = embedded_havok_blob(&converted).expect("FO4 Hatchet collision blob");
    let summary = havok_native::api::havok_collision_summary(&collision_blob)
        .expect("FO4 Hatchet collision summary");
    assert!(!summary.contains("\"n_vertices\":0"));
    assert_all_nif_refs_resolve(&converted);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fnv_gecko_body_and_skeleton_defer_legacy_ragdoll_when_available() {
    let source_dir = repo_root().join("extracted/fnv/meshes/creatures/nvgecko");
    let body_source = source_dir.join("nvgecko.nif");
    let skeleton_source = source_dir.join("skeleton.nif");
    if !body_source.is_file() || !skeleton_source.is_file() {
        eprintln!(
            "skipping missing FNV Gecko fixtures {} and {}",
            body_source.display(),
            skeleton_source.display()
        );
        return;
    }

    let source_body = NifFile::load(&body_source).expect("load source Gecko body");
    let source_skeleton = NifFile::load(&skeleton_source).expect("load source Gecko skeleton");
    assert!(source_body.blocks.iter().any(|block| matches!(
        block.type_name.as_str(),
        "NiSkinInstance" | "BSDismemberSkinInstance"
    )));
    assert!(source_skeleton.blocks.iter().any(|block| {
        matches!(
            block.type_name.as_str(),
            "bhkBlendCollisionObject" | "bhkRagdollConstraint"
        )
    }));

    let dir = temp_dir("convert_fnv_gecko_source_rig");
    let body_destination = dir.join("Meshes/Creatures/NVGecko/NVGecko.nif");
    let skeleton_destination = dir.join("Meshes/Creatures/NVGecko/Skeleton.nif");
    let options = ConvertFileOptions {
        asset_prefix: Some("FNV".to_string()),
        skin_policy: LegacySkinPolicy::PreserveSourceRig,
        ..ConvertFileOptions::default()
    };

    let body_report = convert_nif_file(
        &body_source,
        &body_destination,
        "fnv",
        "fo4",
        None,
        &options,
    )
    .expect("convert FNV Gecko body");
    assert!(body_report.supported, "{:?}", body_report.errors);
    assert!(body_report.errors.is_empty(), "{:?}", body_report.errors);
    assert_eq!(body_report.shapes_skinned, 6);
    let body = NifFile::load(&body_destination).expect("load converted Gecko body");
    assert_eq!(
        body.blocks
            .iter()
            .filter(|block| block.type_name == "BSSubIndexTriShape")
            .count(),
        6
    );
    assert_eq!(
        body.blocks
            .iter()
            .filter(|block| block.type_name == "BSSkin::Instance")
            .count(),
        6
    );
    const SOURCE_RIG_ROOT_ONLY_BPTD_SEGMENT: i64 = 32;
    let mut emitted_segment_ids = Vec::new();
    for shape in body
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSSubIndexTriShape")
    {
        assert_eq!(
            shape.get_field("Num Segments").map(NifValue::as_i64),
            Some(33)
        );
        assert_eq!(
            shape.get_field("Total Segments").map(NifValue::as_i64),
            Some(33)
        );
        assert!(shape.get_field("Segment Data").is_none());
        let segments = match shape.get_field("Segment") {
            Some(NifValue::Array(segments)) => segments,
            other => panic!("Gecko shape {} segments are {other:?}", shape.block_id),
        };
        assert_eq!(segments.len(), 33);
        let mut nonempty_segments = 0usize;
        for (segment_index, segment) in segments.iter().enumerate() {
            let segment = match segment {
                NifValue::Struct(fields) => fields,
                other => panic!("Gecko shape {} segment is {other:?}", shape.block_id),
            };
            assert_eq!(
                segment.get("Num Sub Segments").map(NifValue::as_i64),
                Some(0)
            );
            let primitive_count = segment
                .get("Num Primitives")
                .map(NifValue::as_i64)
                .expect("Gecko segment primitive count");
            if primitive_count == 0 {
                continue;
            }
            nonempty_segments += 1;
            assert_eq!(segment.get("Start Index").map(NifValue::as_i64), Some(0));
            assert_eq!(
                Some(primitive_count),
                shape.get_field("Num Triangles").map(NifValue::as_i64)
            );
            emitted_segment_ids.push(segment_index as i64);
        }
        assert_eq!(nonempty_segments, 1);
    }
    emitted_segment_ids.sort_unstable();
    emitted_segment_ids.dedup();
    assert_eq!(emitted_segment_ids, vec![SOURCE_RIG_ROOT_ONLY_BPTD_SEGMENT]);
    assert!(body.blocks.iter().all(|block| {
        !block.type_name.starts_with("bhk")
            && !matches!(
                block.type_name.as_str(),
                "NiSkinInstance" | "BSDismemberSkinInstance" | "NiSkinData" | "NiSkinPartition"
            )
    }));
    assert_all_nif_refs_resolve(&body);

    let skeleton_report = convert_nif_file(
        &skeleton_source,
        &skeleton_destination,
        "fnv",
        "fo4",
        None,
        &options,
    )
    .expect("convert FNV Gecko skeleton");
    assert!(skeleton_report.supported, "{:?}", skeleton_report.errors);
    assert!(
        skeleton_report.errors.is_empty(),
        "{:?}",
        skeleton_report.errors
    );
    assert!(
        skeleton_report
            .warnings
            .iter()
            .any(|warning| { warning.contains("ragdoll conversion is a separate gate") })
    );
    let skeleton = NifFile::load(&skeleton_destination).expect("load converted Gecko skeleton");
    assert!(skeleton.blocks.iter().all(|block| {
        !block.type_name.starts_with("bhk")
            && !block.type_name.to_ascii_lowercase().contains("constraint")
    }));
    let skeleton_bsx_flags = skeleton
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSXFlags")
        .filter_map(|block| block.get_field("Integer Data"))
        .map(NifValue::as_i64)
        .collect::<Vec<_>>();
    assert!(!skeleton_bsx_flags.is_empty());
    assert!(skeleton_bsx_flags.iter().all(|flags| flags & 2 == 0));
    assert!(
        skeleton
            .blocks
            .iter()
            .filter(|block| block.type_name == "NiNode")
            .count()
            > 80
    );
    assert_all_nif_refs_resolve(&skeleton);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_skyrim_steel_battleaxe_pair_to_fo4_melee_when_available() {
    let source_dir = repo_path("extracted/skyrimse/meshes/weapons/steel");
    let sources = [
        source_dir.join("steelbattleaxe.nif"),
        source_dir.join("1stpersonsteelbattleaxe.nif"),
    ];
    if sources.iter().any(|source| !source.is_file()) {
        eprintln!("skipping missing Skyrim steel battleaxe fixtures");
        return;
    }

    let dir = temp_dir("convert_skyrim_steel_battleaxe_pair");
    let materials = dir.join("data/Materials/Skyrim");
    for source in sources {
        let destination = dir
            .join("data/Meshes/Weapons/Steel")
            .join(source.file_name().expect("battleaxe filename"));
        let report = convert_nif_file(
            &source,
            &destination,
            "skyrimse",
            "fo4",
            Some(&materials),
            &ConvertFileOptions {
                asset_prefix: Some("Skyrim".to_string()),
                weapon_role: Some("melee".to_string()),
                ..ConvertFileOptions::default()
            },
        )
        .expect("convert Skyrim steel battleaxe");

        assert!(
            report.supported,
            "{}: {:?}",
            source.display(),
            report.errors
        );
        assert!(
            report.errors.is_empty(),
            "{}: {:?}",
            source.display(),
            report.errors
        );
        assert!(!report.emitted_bgsms.is_empty(), "{}", source.display());

        let converted = NifFile::load(&destination).expect("load converted battleaxe");
        assert_eq!(
            converted.header.version,
            (20, 2, 0, 7),
            "{}",
            source.display()
        );
        assert_eq!(converted.header.user_version, 12, "{}", source.display());
        assert_eq!(converted.header.bs_version, 130, "{}", source.display());
        assert_eq!(
            converted.blocks[0].type_name,
            "NiNode",
            "{}",
            source.display()
        );
        let markers = converted
            .blocks
            .iter()
            .filter(|block| {
                block.type_name == "NiStringExtraData"
                    && matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "Prn")
                    && matches!(block.get_field("String Data"), Some(NifValue::String(value)) if value == "WEAPON")
            })
            .collect::<Vec<_>>();
        assert_eq!(markers.len(), 1, "{}", source.display());
        let root_extra_ids = match converted.blocks[0].get_field("Extra Data List") {
            Some(NifValue::Array(values)) => values
                .iter()
                .filter_map(|value| match value {
                    NifValue::Ref(id) if *id >= 0 => Some(*id as usize),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            other => panic!("{} root extra data is {other:?}", source.display()),
        };
        assert!(root_extra_ids.contains(&markers[0].block_id));
        assert!(converted.blocks.iter().all(|block| {
            block.type_name != "NiStringExtraData"
                || (!matches!(block.get_field("Name"), Some(NifValue::String(name)) if name == "WEAPON")
                    && !matches!(block.get_field("String Data"), Some(NifValue::String(value)) if value == "WeaponBack"))
        }));
        for shader in converted.blocks.iter().filter(|block| {
            matches!(
                block.type_name.as_str(),
                "BSLightingShaderProperty" | "BSEffectShaderProperty"
            )
        }) {
            let material = match shader.get_field("Name") {
                Some(NifValue::String(material)) => material.to_ascii_lowercase(),
                other => panic!("{} shader material is {other:?}", source.display()),
            };
            assert!(
                material.starts_with("materials\\skyrim\\")
                    && (matches!(
                        shader.type_name.as_str(),
                        "BSLightingShaderProperty" if material.ends_with(".bgsm")
                    ) || matches!(
                        shader.type_name.as_str(),
                        "BSEffectShaderProperty" if material.ends_with(".bgem")
                    )),
                "{} {} material {material}",
                source.display(),
                shader.type_name
            );
        }
        assert!(converted.blocks.iter().all(|block| !matches!(
            block.type_name.as_str(),
            "bhkCollisionObject" | "bhkRigidBody" | "bhkRigidBodyT"
        )));

        let has_np_collision = converted
            .blocks
            .iter()
            .any(|block| block.type_name == "bhkNPCollisionObject");
        if !has_np_collision {
            let bsx_flags = converted
                .blocks
                .iter()
                .filter(|block| block.type_name == "BSXFlags")
                .filter_map(|block| block.get_field("Integer Data"))
                .map(NifValue::as_i64)
                .collect::<Vec<_>>();
            assert!(bsx_flags.iter().all(|flags| flags & 2 == 0));
            assert!(report.warnings.iter().any(|warning| {
                warning.contains("Skyrim melee weapon active collision is not FO4-compatible")
            }));
        }
        assert_all_nif_refs_resolve(&converted);
    }

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_skyrim_wolf_preserves_its_source_rig_when_available() {
    let src = repo_path("extracted/skyrimse/meshes/actors/canine/character assets wolf/wolf.nif");
    if !src.is_file() {
        eprintln!("skipping missing Skyrim wolf fixture");
        return;
    }
    let dir = temp_dir("convert_skyrim_wolf_source_rig");
    let dst = dir.join("Meshes/Actors/Wolf/CharacterAssets/Wolf.nif");
    let materials = dir.join("Materials/Skyrim");
    let source = NifFile::load(&src).expect("load source wolf");

    let report = convert_nif_file(
        &src,
        &dst,
        "skyrimse",
        "fo4",
        Some(&materials),
        &ConvertFileOptions {
            asset_prefix: Some("Skyrim".to_string()),
            skin_policy: LegacySkinPolicy::PreserveSourceRig,
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert Skyrim wolf with its own rig");

    assert!(report.supported, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.shapes_skinned, 2, "{:?}", report.changes);
    assert_eq!(report.vertices_repacked, 3154 + 628);
    assert_eq!(report.bones_remapped, 0);
    assert_eq!(report.bones_dropped_unmapped, 0);
    assert_eq!(report.weights_redistributed, 0);

    let converted = NifFile::load(&dst).expect("load converted wolf");
    assert_eq!(converted.header.version, (20, 2, 0, 7));
    assert_eq!(converted.header.user_version, 12);
    assert_eq!(converted.header.bs_version, 130);
    assert_all_nif_refs_resolve(&converted);
    assert!(converted.blocks.iter().all(|block| {
        !matches!(
            block.type_name.as_str(),
            "NiSkinInstance" | "BSDismemberSkinInstance" | "NiSkinData" | "NiSkinPartition"
        ) && !block.type_name.starts_with("bhk")
    }));

    let palettes = [
        (
            "WolfREDUCED",
            3154usize,
            5058usize,
            vec![
                "Canine_Pelvis",
                "Canine_LBackLeg1",
                "Canine_LBackLeg2",
                "Canine_LBackLegPalm",
                "Canine_LBackLegToe",
                "Canine_RBackLeg1",
                "Canine_RBackLeg2",
                "Canine_RBackLegPalm",
                "Canine_RBackLegToe",
                "Canine_Spine1",
                "Canine_Spine2",
                "Canine_Spine3",
                "Canine_LEar_Wolf",
                "Canine_LFrontLegShoulderblade",
                "Canine_LFrontLeg1",
                "Canine_LFrontLeg2",
                "Canine_LFrontLegPalm",
                "Canine_LFrontLegToe",
                "Canine_Neck1",
                "Canine_Neck2",
                "Canine_Head",
                "Canine_REar_Wolf",
                "Canine_Dog_LEyelid",
                "Canine_Dog_REyelid",
                "Canine_LUpperLip",
                "Canine_RUpperLip",
                "Canine_JawBone",
                "Canine_Tongue01",
                "Canine_Tongue02",
                "Canine_Tongue03",
                "Canine_LEye",
                "Canine_REye",
                "Canine_RFrontLegShoulderblade",
                "Canine_RFrontLeg1",
                "Canine_RFrontLeg2",
                "Canine_RFrontLegPalm",
                "Canine_RFrontLegToe",
                "Canine_Tail1",
                "Canine_Tail2",
                "Canine_Tail3",
            ],
        ),
        (
            "Fur",
            628usize,
            720usize,
            vec![
                "Canine_Pelvis",
                "Canine_LBackLeg1",
                "Canine_LBackLeg2",
                "Canine_RBackLeg1",
                "Canine_RBackLeg2",
                "Canine_Spine3",
                "Canine_LEar_Wolf",
                "Canine_LFrontLegShoulderblade",
                "Canine_LFrontLeg1",
                "Canine_LFrontLeg2",
                "Canine_Neck1",
                "Canine_Neck2",
                "Canine_Head",
                "Canine_REar_Wolf",
                "Canine_JawBone",
                "Canine_RFrontLegShoulderblade",
                "Canine_RFrontLeg1",
                "Canine_RFrontLeg2",
                "Canine_Tail1",
            ],
        ),
    ];

    for (shape_name, vertex_count, triangle_count, expected_palette) in palettes {
        assert_wolf_shape_contract(
            &source,
            &converted,
            shape_name,
            vertex_count,
            triangle_count,
            &expected_palette,
        );
    }

    let _ = std::fs::remove_dir_all(dir);
}

const SKYRIM_CREATURE_BODY_CORPUS: &[&str] = &[
    "Actors/Ambient/Chicken/Character Assets/Chicken.nif",
    "Actors/Ambient/Hare/Character Assets/Rabbit.nif",
    "Actors/AtronachFlame/Character Assets/AtronachFlame.nif",
    "Actors/AtronachFrost/Character Assets/AtronachFrost.nif",
    "Actors/AtronachStorm/Character Assets/AtronachStorm.nif",
    "Actors/Bear/Character Assets/bear.NIF",
    "Actors/Canine/Character Assets Dog/dog.nif",
    "Actors/Canine/Character Assets Wolf/wolf.nif",
    "Actors/Character/Character Assets/1stPersonMaleBody_1.NIF",
    "Actors/Chaurus/Character Assets/ChauurusSkin.nif",
    "Actors/Cow/Character Assets/HighlandCow.nif",
    "Actors/Deer/Character Assets/Elk_MaleSkin.nif",
    "Actors/Deer/Character Assets/ReinDeer_Skin.nif",
    "Actors/DLC02/BenthicLurker/Character Assets/BenthicLurker.nif",
    "Actors/DLC02/BoarRiekling/Character Assets/Boar.nif",
    "Actors/DLC02/BoarRiekling/Character Assets/BoarRiekling_VariantA.nif",
    "Actors/DLC02/DLC2FrostGiant/DLC2FrostGiant.nif",
    "Actors/DLC02/DwarvenBallistaCenturion/Character Assets/DwarvenBallista.nif",
    "Actors/DLC02/HMDaedra/Character Assets/HMDaedra.nif",
    "Actors/DLC02/HulkingDraugr/HulkingDraugr.nif",
    "Actors/DLC02/Netch/CharacterAssets/Netch.nif",
    "Actors/DLC02/Netch/CharacterAssets/NetchCalf.nif",
    "Actors/DLC02/Riekling/Character Assets/Riekling01.nif",
    "Actors/DLC02/Riekling/Character Assets/RieklingChief.nif",
    "Actors/DLC02/Scrib/Character Assets/Scrib.nif",
    "Actors/DLC02/SpectralDragonHead/SpectralDragonHead.nif",
    "Actors/DLC02/Spider_poison/CharacterAssets/Spider_poison.nif",
    "Actors/DLC02/SprigganBurnt/SprigganBurnt.nif",
    "Actors/DLC02/StormAtronachAsh/StormAtronachAsh.nif",
    "Actors/DLC02/Werebear/werebear.nif",
    "Actors/Dragon/Character Assets/Dragon.nif",
    "Actors/DragonPriest/Character Assets/Dragon_Priest.nif",
    "Actors/Draugr/Character Assets/DraugrMale.nif",
    "Actors/Draugr/Character Assets/DraugrSkeleton.nif",
    "Actors/DwarvenSphereCenturion/Character Assets/SphereCenturion.nif",
    "Actors/DwarvenSpider/Character Assets/DwarvenSpiderCenturion.nif",
    "Actors/DwarvenSteamCenturion/Character Assets/SteamCenturion.nif",
    "Actors/Falmer/Character Assets/Falmer_Male.nif",
    "Actors/FrostbiteSpider/Character Assets/FrostbiteSpider.nif",
    "Actors/Giant/Character Assets/Giant02.nif",
    "Actors/Goat/Character Assets/Goat.nif",
    "Actors/Hagraven/Character Assets/Hagraven.nif",
    "Actors/Horker/Character Assets/Horker.nif",
    "Actors/Horse/Character Assets/Horse.nif",
    "Actors/IceWraith/Character Assets/IceWraith.nif",
    "Actors/Mammoth/Character Assets/MammothWild.nif",
    "Actors/Mudcrab/Character Assets/MCrab1.nif",
    "Actors/SabreCat/Character Assets/SabreCat.nif",
    "Actors/SabreCat/Character Assets/SabreCatSnow.nif",
    "Actors/Skeever/Character Assets/Skeever.nif",
    "Actors/Slaughterfish/Character Assets/SlaughterFish.nif",
    "Actors/Spriggan/Character Assets/spriggan.nif",
    "Actors/Troll/Character Assets/BlackTroll.nif",
    "Actors/Troll/Character Assets/Troll.nif",
    "Actors/WerewolfBeast/Character Assets/MaleBodyWereWolf_1.NIF",
    "Actors/Wisp/Character Assets/Wisp.nif",
    "Actors/Witchlight/Character Assets/Witchlight.nif",
    "DLC02/Clutter/DLC02DetectLifeBox.nif",
];

#[test]
fn convert_canonical_skyrim_creature_body_corpus_with_source_rigs_when_available() {
    assert_eq!(SKYRIM_CREATURE_BODY_CORPUS.len(), 58);
    let source_root = repo_path("extracted/skyrimse/meshes");
    if !source_root.is_dir() {
        eprintln!(
            "skipping missing Skyrim creature corpus {}",
            source_root.display()
        );
        return;
    }

    let dir = temp_dir("convert_skyrim_creature_source_rig_corpus");
    let mut converted = Vec::new();
    let mut rejected = Vec::new();
    let mut failures = Vec::new();
    for relative in SKYRIM_CREATURE_BODY_CORPUS {
        let source_path = source_root.join(relative);
        if !source_path.is_file() {
            failures.push(format!("{relative}: canonical fixture is missing"));
            continue;
        }
        let destination = dir.join("Meshes").join(relative);
        let material_dir = dir.join("Materials/Skyrim");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            classify_source_rig_corpus_entry(
                &source_path,
                &destination,
                "skyrimse",
                Some(&material_dir),
                relative,
            )
        }));

        match result {
            Err(_) => failures.push(format!("{relative}: conversion panicked")),
            Ok(Err(error)) => failures.push(error),
            Ok(Ok(SourceRigCorpusOutcome::Rejected(reason))) => {
                rejected.push(format!("{relative}: {reason}"));
            }
            Ok(Ok(SourceRigCorpusOutcome::Converted)) => converted.push(relative.to_string()),
        }
    }

    eprintln!(
        "Skyrim PreserveSourceRig corpus: candidates={} converted={} rejected={}",
        SKYRIM_CREATURE_BODY_CORPUS.len(),
        converted.len(),
        rejected.len()
    );
    for rejection in &rejected {
        eprintln!("  rejected: {rejection}");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(
        converted.len() + rejected.len(),
        SKYRIM_CREATURE_BODY_CORPUS.len()
    );
    assert!(
        converted
            .iter()
            .any(|path| path.ends_with("Character Assets Wolf/wolf.nif")),
        "the exact wolf fixture must remain in the converted set"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_direct_fnv_and_fo3_creature_body_and_skeleton_corpus_when_available() {
    let dir = temp_dir("convert_gamebryo_creature_source_rig_corpus");
    let mut total_candidates = 0usize;
    let mut total_converted = 0usize;
    let mut total_rejected = 0usize;
    let mut failures = Vec::new();
    let mut saw_gecko = false;

    for source_game in ["fnv", "fo3"] {
        let source_root = repo_path(&format!("extracted/{source_game}/meshes/creatures"));
        if !source_root.is_dir() {
            eprintln!(
                "skipping missing {source_game} creature corpus {}",
                source_root.display()
            );
            continue;
        }
        let candidates = direct_creature_body_and_skeleton_candidates(&source_root)
            .unwrap_or_else(|error| panic!("discover {source_game} creature corpus: {error}"));
        assert!(
            !candidates.is_empty(),
            "{source_game} direct creature corpus is empty"
        );
        let mut converted = Vec::new();
        let mut rejected = Vec::new();
        for (relative, source_path) in &candidates {
            let destination = dir.join(source_game).join("Meshes").join(relative);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                classify_source_rig_corpus_entry(
                    source_path,
                    &destination,
                    source_game,
                    None,
                    relative,
                )
            }));
            match result {
                Err(_) => failures.push(format!("{source_game}/{relative}: conversion panicked")),
                Ok(Err(error)) => failures.push(error),
                Ok(Ok(SourceRigCorpusOutcome::Rejected(reason))) => {
                    rejected.push(format!("{relative}: {reason}"));
                }
                Ok(Ok(SourceRigCorpusOutcome::Converted)) => {
                    saw_gecko |= source_game == "fnv"
                        && relative.eq_ignore_ascii_case("nvgecko/nvgecko.nif");
                    converted.push(relative.clone());
                }
            }
        }
        eprintln!(
            "{source_game} PreserveSourceRig direct creature corpus: candidates={} converted={} rejected={}",
            candidates.len(),
            converted.len(),
            rejected.len()
        );
        for rejection in &rejected {
            eprintln!("  rejected: {rejection}");
        }
        total_candidates += candidates.len();
        total_converted += converted.len();
        total_rejected += rejected.len();
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(total_converted + total_rejected, total_candidates);
    if repo_path("extracted/fnv/meshes/creatures/nvgecko/nvgecko.nif").is_file() {
        assert!(saw_gecko, "the exact FNV Gecko body must remain converted");
    }
    let _ = std::fs::remove_dir_all(dir);
}

fn direct_creature_body_and_skeleton_candidates(
    source_root: &std::path::Path,
) -> Result<Vec<(String, PathBuf)>, String> {
    let mut candidates = Vec::new();
    let families = std::fs::read_dir(source_root)
        .map_err(|error| format!("read {}: {error}", source_root.display()))?;
    for family in families {
        let family = family.map_err(|error| error.to_string())?;
        let file_type = family.file_type().map_err(|error| error.to_string())?;
        if !file_type.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(family.path()).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if !entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_file()
                || entry.path().extension().is_none_or(|extension| {
                    !extension.to_string_lossy().eq_ignore_ascii_case("nif")
                })
            {
                continue;
            }
            let filename = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let source = NifFile::load(entry.path())
                .map_err(|error| format!("load {}: {error}", entry.path().display()))?;
            let is_skinned_body = source.blocks.iter().any(|block| {
                matches!(
                    block.type_name.as_str(),
                    "NiSkinInstance" | "BSDismemberSkinInstance"
                )
            });
            if !is_skinned_body && !filename.starts_with("skeleton") {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(source_root)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            candidates.push((relative, entry.path()));
        }
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(candidates)
}

#[derive(Debug)]
enum SourceRigCorpusOutcome {
    Converted,
    Rejected(String),
}

fn classify_source_rig_corpus_entry(
    source_path: &std::path::Path,
    destination: &std::path::Path,
    source_game: &str,
    material_dir: Option<&std::path::Path>,
    label: &str,
) -> Result<SourceRigCorpusOutcome, String> {
    let source = NifFile::load(source_path)
        .map_err(|error| format!("{label}: source read failed: {error}"))?;
    let legacy_skin_count = source
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.type_name.as_str(),
                "NiSkinInstance" | "BSDismemberSkinInstance"
            )
        })
        .count();
    let report = convert_nif_file(
        source_path,
        destination,
        source_game,
        "fo4",
        material_dir,
        &ConvertFileOptions {
            asset_prefix: Some(
                match source_game {
                    "skyrimse" => "Skyrim",
                    "fo3" => "FO3",
                    _ => "FNV",
                }
                .to_string(),
            ),
            skin_policy: LegacySkinPolicy::PreserveSourceRig,
            ..ConvertFileOptions::default()
        },
    );
    match report {
        Err(ConvertFileError::PreserveSourceRig { path, reason }) => {
            if destination.exists() {
                return Err(format!("{label}: typed rejection left a partial output"));
            }
            if path != source_path {
                return Err(format!(
                    "{label}: typed rejection path {} does not match {}",
                    path.display(),
                    source_path.display()
                ));
            }
            if reason.trim().is_empty() {
                return Err(format!("{label}: typed rejection has no reason"));
            }
            Ok(SourceRigCorpusOutcome::Rejected(reason))
        }
        Err(other) => Err(format!(
            "{label}: conversion returned an untyped source-rig rejection: {other}"
        )),
        Ok(report) if !report.supported || !report.errors.is_empty() => {
            if destination.exists() {
                return Err(format!("{label}: report rejection left a partial output"));
            }
            Err(format!(
                "{label}: source-rig rejection was not typed: {}",
                report.errors.join("; ")
            ))
        }
        Ok(report) => {
            let output = NifFile::load(destination)
                .map_err(|error| format!("{label}: load converted output failed: {error}"))?;
            assert_source_rig_corpus_output(
                label,
                source_game,
                &source,
                legacy_skin_count,
                &output,
                &report,
            );
            Ok(SourceRigCorpusOutcome::Converted)
        }
    }
}

fn assert_source_rig_corpus_output(
    relative: &str,
    source_game: &str,
    source: &NifFile,
    legacy_skin_count: usize,
    output: &NifFile,
    report: &nif_core_native::convert_file::ConvertFileReport,
) {
    assert_eq!(output.header.version, (20, 2, 0, 7), "{relative}");
    assert_eq!(output.header.user_version, 12, "{relative}");
    assert_eq!(output.header.bs_version, 130, "{relative}");
    assert_all_nif_refs_resolve(output);
    for material in &report.emitted_bgsms {
        assert!(
            PathBuf::from(material).is_file(),
            "{relative}: emitted material is missing: {material}"
        );
    }
    assert!(
        output.blocks.iter().all(|block| {
            !matches!(
                block.type_name.as_str(),
                "NiSkinInstance" | "BSDismemberSkinInstance" | "NiSkinData" | "NiSkinPartition"
            ) && !block.type_name.starts_with("bhk")
        }),
        "{relative}: legacy skin or creature collision remains"
    );

    if legacy_skin_count == 0 {
        assert_eq!(report.shapes_skinned, 0, "{relative}");
        return;
    }
    assert_eq!(report.shapes_skinned, legacy_skin_count, "{relative}");
    let skinned_shapes = output
        .blocks
        .iter()
        .filter(|shape| {
            shape.type_name == "BSSubIndexTriShape"
                && shape
                    .get_field("Skin")
                    .is_some_and(|skin| skin.as_i64() >= 0)
        })
        .collect::<Vec<_>>();
    assert_eq!(skinned_shapes.len(), legacy_skin_count, "{relative}");
    let source_skinned_shapes = source
        .blocks
        .iter()
        .filter(|shape| {
            nif_core_native::skin::source::parse_skin_chain(source, shape.block_id)
                .is_ok_and(|skin| skin.is_some())
        })
        .collect::<Vec<_>>();
    assert_eq!(source_skinned_shapes.len(), legacy_skin_count, "{relative}");

    for (shape, source_shape) in skinned_shapes.into_iter().zip(source_skinned_shapes) {
        let shape_name = block_name(shape);
        let parsed = nif_core_native::skin::source::parse_skin_chain(source, source_shape.block_id)
            .unwrap_or_else(|error| panic!("{relative}: parse {shape_name:?}: {error}"))
            .unwrap_or_else(|| panic!("{relative}: {shape_name:?} lost its source skin"));
        let mut expected_triangles = array_field(source_shape, "Triangles")
            .iter()
            .map(triangle_indices)
            .collect::<Vec<_>>();
        if expected_triangles.is_empty() {
            expected_triangles = source_shape
                .get_field("Data")
                .map(NifValue::as_i64)
                .filter(|data| *data >= 0)
                .and_then(|data| source.get_block(data as usize))
                .map(|data| {
                    array_field(data, "Triangles")
                        .iter()
                        .map(triangle_indices)
                        .collect()
                })
                .unwrap_or_default();
        }
        if expected_triangles.is_empty() {
            let streamed_partition = parsed
                .skin_partition_block_id
                .and_then(|id| source.get_block(id))
                .is_some_and(|block| !array_field(block, "Vertex Data").is_empty());
            expected_triangles = parsed
                .partitions
                .iter()
                .flat_map(|partition| {
                    partition.triangles.iter().filter_map(|triangle| {
                        if streamed_partition {
                            Some(*triangle)
                        } else {
                            Some([
                                *partition.vertex_map.get(triangle[0] as usize)?,
                                *partition.vertex_map.get(triangle[1] as usize)?,
                                *partition.vertex_map.get(triangle[2] as usize)?,
                            ])
                        }
                    })
                })
                .collect();
        }
        let actual_triangles = array_field(shape, "Triangles")
            .iter()
            .map(triangle_indices)
            .collect::<Vec<_>>();
        assert_eq!(
            actual_triangles.len(),
            expected_triangles.len(),
            "{relative}: {shape_name}"
        );
        for (triangle_index, (actual, expected)) in
            actual_triangles.iter().zip(&expected_triangles).enumerate()
        {
            assert_eq!(
                actual, expected,
                "{relative}: {shape_name} triangle {triangle_index}"
            );
        }

        let skin = referenced_block(output, shape, "Skin");
        assert_eq!(skin.type_name, "BSSkin::Instance", "{relative}");
        let bone_refs = ref_values(skin, "Bones");
        assert_eq!(
            bone_refs.len(),
            parsed.bones.len(),
            "{relative}: {shape_name}"
        );
        let actual_palette = bone_refs
            .iter()
            .map(|bone| block_name(output.get_block(*bone).expect("palette bone")))
            .collect::<Vec<_>>();
        let expected_palette = parsed
            .bones
            .iter()
            .map(|bone| bone.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(actual_palette, expected_palette, "{relative}: {shape_name}");

        let segments = array_field(shape, "Segment");
        assert_eq!(
            field_usize(shape, "Num Segments"),
            33,
            "{relative}: {shape_name}"
        );
        assert_eq!(
            field_usize(shape, "Total Segments"),
            33,
            "{relative}: {shape_name}"
        );
        assert_eq!(segments.len(), 33, "{relative}: {shape_name}");
        assert!(
            shape.get_field("Segment Data").is_none(),
            "{relative}: {shape_name}"
        );
        for (index, segment) in segments.iter().enumerate() {
            let NifValue::Struct(fields) = segment else {
                panic!("{relative}: {shape_name} segment {index} is not structured");
            };
            let primitives = fields
                .get("Num Primitives")
                .map(NifValue::as_i64)
                .unwrap_or(-1);
            assert_eq!(
                primitives,
                if index == 32 {
                    actual_triangles.len() as i64
                } else {
                    0
                },
                "{relative}: {shape_name} segment {index}"
            );
        }

        for (vertex_index, vertex) in array_field(shape, "Vertex Data").iter().enumerate() {
            let NifValue::Struct(fields) = vertex else {
                panic!("{relative}: {shape_name} vertex {vertex_index} is not structured");
            };
            let indices = numeric_values(fields.get("Bone Indices"));
            let weights = float_values(fields.get("Bone Weights"));
            assert_eq!(
                indices.len(),
                4,
                "{relative}: {shape_name} vertex {vertex_index}"
            );
            assert_eq!(
                weights.len(),
                4,
                "{relative}: {shape_name} vertex {vertex_index}"
            );
            assert!(
                indices
                    .iter()
                    .all(|bone| (*bone as usize) < bone_refs.len()),
                "{relative}: {shape_name} vertex {vertex_index} palette index"
            );
            assert!(
                (weights.iter().sum::<f32>() - 1.0).abs() <= 0.001,
                "{relative}: {shape_name} vertex {vertex_index} weights"
            );
        }
    }

    if source_game == "skyrimse" {
        for shader in output.blocks.iter().filter(|block| {
            matches!(
                block.type_name.as_str(),
                "BSLightingShaderProperty" | "BSEffectShaderProperty"
            )
        }) {
            let material = block_name(shader).to_ascii_lowercase();
            assert!(
                material.starts_with("materials\\skyrim\\")
                    && (material.ends_with(".bgsm") || material.ends_with(".bgem")),
                "{relative}: invalid FO4 material path {material:?}"
            );
        }
    }
    for texture_set in output
        .blocks
        .iter()
        .filter(|block| block.type_name == "BSShaderTextureSet")
    {
        for texture in array_field(texture_set, "Textures") {
            let texture = texture_string(texture);
            if texture.is_empty() {
                continue;
            }
            let normalized = texture.replace('/', "\\").to_ascii_lowercase();
            assert!(
                normalized.starts_with("textures\\")
                    && normalized.ends_with(".dds")
                    && !normalized.contains("..")
                    && !PathBuf::from(texture).is_absolute(),
                "{relative}: invalid texture path {texture:?}"
            );
        }
    }
}

fn assert_wolf_shape_contract(
    source: &NifFile,
    converted: &NifFile,
    shape_name: &str,
    vertex_count: usize,
    triangle_count: usize,
    expected_palette: &[&str],
) {
    let source_shape = named_block(source, shape_name);
    let source_skin =
        nif_core_native::skin::source::parse_skin_chain(source, source_shape.block_id)
            .expect("parse source wolf skin")
            .expect("source wolf shape is skinned");
    assert_eq!(source_skin.partitions.len(), 1);
    let partition = &source_skin.partitions[0];
    assert_eq!(partition.vertex_map.len(), vertex_count);
    assert_eq!(partition.triangles.len(), triangle_count);
    assert_eq!(
        partition.vertex_map,
        (0..vertex_count as u32).collect::<Vec<_>>(),
        "{shape_name} vertex map"
    );
    assert_eq!(
        source_skin
            .bones
            .iter()
            .map(|bone| bone.name.as_str())
            .collect::<Vec<_>>(),
        expected_palette
    );

    let target_shape = named_block(converted, shape_name);
    assert_eq!(target_shape.type_name, "BSSubIndexTriShape");
    assert_eq!(field_usize(target_shape, "Num Vertices"), vertex_count);
    assert_eq!(field_usize(target_shape, "Num Triangles"), triangle_count);
    assert_eq!(field_usize(target_shape, "Num Segments"), 33);
    assert_eq!(field_usize(target_shape, "Total Segments"), 33);
    assert!(target_shape.get_field("Segment Data").is_none());
    let segments = array_field(target_shape, "Segment");
    assert_eq!(segments.len(), 33);
    let nonempty_segment_indices = segments
        .iter()
        .enumerate()
        .filter_map(|(segment_index, segment)| match segment {
            NifValue::Struct(fields)
                if fields
                    .get("Num Primitives")
                    .is_some_and(|count| count.as_i64() > 0) =>
            {
                Some(segment_index)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(nonempty_segment_indices, vec![32], "{shape_name} segments");
    for segment in segments {
        let NifValue::Struct(fields) = segment else {
            panic!("{shape_name} segment is {segment:?}");
        };
        assert_eq!(
            fields.get("Num Sub Segments").map(NifValue::as_i64),
            Some(0)
        );
    }
    let NifValue::Struct(root_segment) = &segments[32] else {
        unreachable!();
    };
    assert_eq!(
        root_segment.get("Start Index").map(NifValue::as_i64),
        Some(0)
    );
    assert_eq!(
        root_segment.get("Num Primitives").map(NifValue::as_i64),
        Some(triangle_count as i64)
    );
    assert_sphere_near(
        target_shape
            .get_field("Bounding Sphere")
            .expect("target shape bound"),
        source_shape
            .get_field("Bounding Sphere")
            .expect("source shape bound"),
    );
    let target_triangles = array_field(target_shape, "Triangles")
        .iter()
        .map(triangle_indices)
        .collect::<Vec<_>>();
    assert_eq!(
        target_triangles, partition.triangles,
        "{shape_name} triangles"
    );

    let target_skin = referenced_block(converted, target_shape, "Skin");
    assert_eq!(target_skin.type_name, "BSSkin::Instance");
    let target_bone_refs = ref_values(target_skin, "Bones");
    assert_eq!(target_bone_refs.len(), expected_palette.len());
    let target_palette = target_bone_refs
        .iter()
        .map(|id| block_name(converted.get_block(*id).expect("palette bone")))
        .collect::<Vec<_>>();
    assert_eq!(target_palette, expected_palette);
    assert_eq!(
        named_parent_map(source, expected_palette),
        named_parent_map(converted, expected_palette),
        "{shape_name} bone hierarchy"
    );

    let target_bone_data = referenced_block(converted, target_skin, "Data");
    assert_eq!(target_bone_data.type_name, "BSSkin::BoneData");
    let target_binds = array_field(target_bone_data, "Bone List");
    assert_eq!(target_binds.len(), expected_palette.len());
    for (bone_index, target_bind) in target_binds.iter().enumerate() {
        let NifValue::Struct(fields) = target_bind else {
            panic!("{shape_name} bind {bone_index} is not structured");
        };
        let source_transform = source_skin.bone_transforms[bone_index];
        assert_matrix_near(
            fields.get("Rotation").expect("target bind rotation"),
            source_transform.rotation,
        );
        assert_vec3_near(
            fields.get("Translation").expect("target bind translation"),
            source_transform.translation,
        );
        assert!(
            (fields.get("Scale").and_then(numeric_f64).unwrap_or(0.0)
                - source_transform.scale as f64)
                .abs()
                <= 1e-6
        );
        assert_sphere_near(
            fields.get("Bounding Sphere").expect("target bind bound"),
            source_skin.bone_bounds[bone_index]
                .as_ref()
                .expect("source bind bound"),
        );
    }

    let target_vertices = array_field(target_shape, "Vertex Data");
    assert_eq!(target_vertices.len(), vertex_count);
    let source_partition = source
        .get_block(source_skin.skin_partition_block_id.expect("partition ref"))
        .expect("source partition");
    let source_vertices = array_field(source_partition, "Vertex Data");
    assert_eq!(source_vertices.len(), vertex_count);
    for vertex_index in 0..vertex_count {
        let NifValue::Struct(source_fields) = &source_vertices[vertex_index] else {
            panic!("source {shape_name} vertex {vertex_index}");
        };
        let NifValue::Struct(target_fields) = &target_vertices[vertex_index] else {
            panic!("target {shape_name} vertex {vertex_index}");
        };
        assert_vec3_near_with_tolerance(
            target_fields.get("Vertex").expect("target position"),
            vec3_components(source_fields.get("Vertex").expect("source position")),
            0.04,
        );
        for field in ["Normal", "Tangent"] {
            assert_vec3_near_with_tolerance(
                target_fields.get(field).expect("target direction"),
                vec3_components(source_fields.get(field).expect("source direction")),
                0.01,
            );
        }
        assert_vec2_near(
            target_fields.get("UV").expect("target UV"),
            source_fields.get("UV").expect("source UV"),
            0.001,
        );
        for field in ["Bitangent X", "Bitangent Y", "Bitangent Z"] {
            let source_value = source_fields.get(field).and_then(numeric_f64).unwrap();
            let target_value = target_fields.get(field).and_then(numeric_f64).unwrap();
            assert!(
                (target_value - source_value).abs() <= 0.01,
                "{shape_name} vertex {vertex_index} {field}: {target_value} != {source_value}"
            );
        }
        if let Some(source_color) = source_fields.get("Vertex Colors") {
            assert_color4_near(
                target_fields
                    .get("Vertex Colors")
                    .expect("target vertex color"),
                source_color,
                0.01,
            );
        }
        let source_row = &partition.influences[vertex_index];
        let mut expected_indices = [0u32; 4];
        let mut expected_weights = [0.0f32; 4];
        let total = source_row.iter().map(|(_, weight)| *weight).sum::<f32>();
        for (lane, (local_bone, weight)) in source_row.iter().enumerate() {
            expected_indices[lane] = partition.bones[*local_bone as usize];
            expected_weights[lane] = *weight / total;
        }
        let target_indices = numeric_values(target_fields.get("Bone Indices"));
        let target_weights = float_values(target_fields.get("Bone Weights"));
        assert_eq!(
            target_indices,
            expected_indices.to_vec(),
            "{shape_name} vertex {vertex_index}"
        );
        assert_eq!(target_weights.len(), 4);
        for lane in 0..4 {
            assert!(
                (target_weights[lane] - expected_weights[lane]).abs() <= 0.001,
                "{shape_name} vertex {vertex_index} lane {lane}: {} != {}",
                target_weights[lane],
                expected_weights[lane]
            );
        }
        assert!((target_weights.iter().sum::<f32>() - 1.0).abs() <= 0.001);
    }

    let source_alpha = referenced_block(source, source_shape, "Alpha Property");
    let target_alpha = referenced_block(converted, target_shape, "Alpha Property");
    assert_eq!(target_alpha.type_name, "NiAlphaProperty");
    assert_eq!(
        target_alpha.get_field("Flags").map(NifValue::as_i64),
        source_alpha.get_field("Flags").map(NifValue::as_i64)
    );
    assert_eq!(
        target_alpha.get_field("Threshold").map(NifValue::as_i64),
        source_alpha.get_field("Threshold").map(NifValue::as_i64)
    );
    let shader = referenced_block(converted, target_shape, "Shader Property");
    assert_eq!(shader.type_name, "BSLightingShaderProperty");
    assert!(
        block_name(shader)
            .to_ascii_lowercase()
            .starts_with("materials\\skyrim\\")
    );
}

fn named_block<'a>(nif: &'a NifFile, name: &str) -> &'a NifBlock {
    nif.blocks
        .iter()
        .find(|block| block_name(block) == name)
        .unwrap_or_else(|| panic!("missing block {name:?}"))
}

fn block_name(block: &NifBlock) -> &str {
    match block.get_field("Name") {
        Some(NifValue::String(name)) => name.trim_end_matches('\0'),
        _ => "",
    }
}

fn field_usize(block: &NifBlock, field: &str) -> usize {
    block.get_field(field).map(NifValue::as_usize).unwrap_or(0)
}

fn array_field<'a>(block: &'a NifBlock, field: &str) -> &'a [NifValue] {
    match block.get_field(field) {
        Some(NifValue::Array(values)) => values,
        _ => &[],
    }
}

fn referenced_block<'a>(nif: &'a NifFile, block: &NifBlock, field: &str) -> &'a NifBlock {
    let reference = block
        .get_field(field)
        .map(NifValue::as_i64)
        .filter(|reference| *reference >= 0)
        .unwrap_or_else(|| panic!("block {} has no {field} ref", block.block_id));
    nif.get_block(reference as usize).unwrap_or_else(|| {
        panic!(
            "block {} {field} ref {reference} is missing",
            block.block_id
        )
    })
}

fn ref_values(block: &NifBlock, field: &str) -> Vec<usize> {
    array_field(block, field)
        .iter()
        .filter_map(|value| {
            let reference = value.as_i64();
            (reference >= 0).then_some(reference as usize)
        })
        .collect()
}

fn triangle_indices(value: &NifValue) -> [u32; 3] {
    let NifValue::Struct(fields) = value else {
        panic!("triangle is not structured");
    };
    ["v1", "v2", "v3"].map(|field| fields.get(field).map(NifValue::as_i64).unwrap_or(-1) as u32)
}

fn named_parent_map(nif: &NifFile, names: &[&str]) -> HashMap<String, String> {
    let names = names
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let mut parents = HashMap::new();
    for parent in &nif.blocks {
        for child in ref_values(parent, "Children") {
            let Some(child) = nif.get_block(child) else {
                continue;
            };
            let child_name = block_name(child);
            if names.contains(child_name) {
                parents.insert(child_name.to_string(), block_name(parent).to_string());
            }
        }
    }
    parents
}

fn assert_matrix_near(value: &NifValue, expected: [[f32; 3]; 3]) {
    let actual = match value {
        NifValue::Matrix33(matrix) => *matrix,
        NifValue::Struct(fields) => [
            ["m11", "m21", "m31"],
            ["m12", "m22", "m32"],
            ["m13", "m23", "m33"],
        ]
        .map(|row| {
            row.map(|field| fields.get(field).and_then(numeric_f64).unwrap_or(f64::NAN) as f32)
        }),
        other => panic!("matrix is {other:?}"),
    };
    for row in 0..3 {
        for column in 0..3 {
            assert!((actual[row][column] - expected[row][column]).abs() <= 1e-5);
        }
    }
}

fn assert_vec3_near(value: &NifValue, expected: [f32; 3]) {
    assert_vec3_near_with_tolerance(value, expected, 1e-4);
}

fn assert_vec3_near_with_tolerance(value: &NifValue, expected: [f32; 3], tolerance: f32) {
    let actual = vec3_components(value);
    for component in 0..3 {
        assert!(
            (actual[component] - expected[component]).abs() <= tolerance,
            "component {component}: actual={actual:?} expected={expected:?}"
        );
    }
}

fn assert_vec2_near(actual: &NifValue, expected: &NifValue, tolerance: f64) {
    let components = |value: &NifValue| match value {
        NifValue::Struct(fields) => [
            fields.get("u").and_then(numeric_f64).unwrap_or(f64::NAN),
            fields.get("v").and_then(numeric_f64).unwrap_or(f64::NAN),
        ],
        other => panic!("UV is {other:?}"),
    };
    for (actual, expected) in components(actual).into_iter().zip(components(expected)) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "UV {actual} != {expected}"
        );
    }
}

fn assert_color4_near(actual: &NifValue, expected: &NifValue, tolerance: f32) {
    let components = |value: &NifValue| match value {
        NifValue::Color4(color) => *color,
        NifValue::Struct(fields) => ["r", "g", "b", "a"]
            .map(|field| fields.get(field).and_then(numeric_f64).unwrap_or(f64::NAN) as f32),
        other => panic!("color is {other:?}"),
    };
    for (actual, expected) in components(actual).into_iter().zip(components(expected)) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "color {actual} != {expected}"
        );
    }
}

fn assert_sphere_near(actual: &NifValue, expected: &NifValue) {
    let NifValue::Struct(actual) = actual else {
        panic!("target sphere is not structured");
    };
    let NifValue::Struct(expected) = expected else {
        panic!("source sphere is not structured");
    };
    assert_vec3_near(
        actual.get("Center").expect("target sphere center"),
        vec3_components(expected.get("Center").expect("source sphere center")),
    );
    let actual_radius = actual.get("Radius").and_then(numeric_f64).unwrap();
    let expected_radius = expected.get("Radius").and_then(numeric_f64).unwrap();
    assert!((actual_radius - expected_radius).abs() <= 1e-5);
}

fn vec3_components(value: &NifValue) -> [f32; 3] {
    match value {
        NifValue::Vec3(vector) => *vector,
        NifValue::Struct(fields) => ["x", "y", "z"]
            .map(|field| fields.get(field).and_then(numeric_f64).unwrap_or(f64::NAN) as f32),
        other => panic!("vector is {other:?}"),
    }
}

fn numeric_values(value: Option<&NifValue>) -> Vec<u32> {
    match value {
        Some(NifValue::Array(values)) => values
            .iter()
            .map(|value| value.as_i64().max(0) as u32)
            .collect(),
        _ => Vec::new(),
    }
}

fn float_values(value: Option<&NifValue>) -> Vec<f32> {
    match value {
        Some(NifValue::Array(values)) => values
            .iter()
            .filter_map(numeric_f64)
            .map(|value| value as f32)
            .collect(),
        _ => Vec::new(),
    }
}

fn numeric_f64(value: &NifValue) -> Option<f64> {
    match value {
        NifValue::Float(value) => Some(*value),
        NifValue::Int(value) => Some(*value as f64),
        NifValue::UInt(value) => Some(*value as f64),
        _ => None,
    }
}

#[test]
fn convert_fnv_vault_suit_skin_fixture_when_available() {
    let src = repo_root().join("extracted/fnv/meshes/armor/vaultsuit/m/outfit.nif");
    if !src.exists() {
        eprintln!("skipping missing fixture {}", src.display());
        return;
    }
    let dir = temp_dir("convert_skin_fixture");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dst = dir.join("out").join("vaultsuit.nif");

    let report = convert_nif_file(
        &src,
        &dst,
        "fnv",
        "fo4",
        None,
        &ConvertFileOptions {
            asset_prefix: Some("fnv".to_string()),
            translation_maps_dir: Some(translation_maps_dir()),
            ..ConvertFileOptions::default()
        },
    )
    .expect("convert nif");

    assert!(report.supported, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.shapes_skinned > 0, "{:?}", report);
    assert!(report.vertices_repacked > 0, "{:?}", report);

    let converted = NifFile::load(dst).expect("load converted nif");
    assert!(
        converted
            .blocks
            .iter()
            .any(|block| block.type_name == "BSSubIndexTriShape")
    );
    assert!(converted.blocks.iter().all(|block| !matches!(
        block.type_name.as_str(),
        "NiSkinInstance" | "NiSkinData" | "NiSkinPartition"
    )));

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

#[test]
fn convert_real_fo76_turret_ragdoll_populates_motion_properties() {
    let src =
        repo_path("extracted/fo76/Meshes/actors/Turret/CharacterAssets/SkeletonTurretMilitary.nif");
    if !src.exists() {
        eprintln!("skip: FO76 military turret skeleton fixture not available");
        return;
    }

    let dir = temp_dir("convert_real_fo76_turret_ragdoll");
    let dst = dir.join("out").join("SkeletonTurretMilitary.nif");
    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert military turret skeleton");

    assert!(report.supported, "{:?}", report.errors);
    let converted = NifFile::load(dst).expect("load converted turret skeleton");
    let blob = embedded_havok_blob(&converted).expect("converted ragdoll blob");
    let hkx = havok_native::hkx::model::HkxFile::read(&blob).expect("parse ragdoll blob");
    let ragdoll = hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpRagdollData")
        .expect("ragdoll data");
    let bodies = hkx_object_array(ragdoll, "bodyCinfos");
    let motions = hkx_object_array(ragdoll, "motionCinfos");
    let motion_properties = hkx_object_array(ragdoll, "motionProperties");

    assert_eq!(bodies.len(), 3);
    assert_eq!(motions.len(), bodies.len());
    assert_eq!(motion_properties.len(), 1);
    for motion in motions {
        let motion_properties_id = hkx_member_i64(motion, "motionPropertiesId");
        assert!(
            motion_properties_id >= 0 && (motion_properties_id as usize) < motion_properties.len(),
            "motionPropertiesId {motion_properties_id} is out of range"
        );
    }

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_radbeaver_ragdoll_uses_fo4_solver_frames() {
    let src = repo_path("extracted/fo76/Meshes/actors/RadBeaver/CharacterAssets/Skeleton.nif");
    if !src.exists() {
        eprintln!("skip: FO76 RadBeaver skeleton fixture not available");
        return;
    }

    let source_nif = NifFile::load(&src).expect("load source RadBeaver skeleton");
    let source_blob = embedded_havok_blob(&source_nif).expect("source RadBeaver ragdoll blob");
    let source_hkx =
        havok_native::hkx::model::HkxFile::read(&source_blob).expect("parse source ragdoll");
    let source_ragdoll = source_hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpRagdollData")
        .expect("source ragdoll data");
    assert!(
        hkx_object_array(source_ragdoll, "bodyCinfos")
            .iter()
            .any(|body| hkx_member_vec4(body, "position")[3].abs() > 1e-6),
        "fixture must exercise nonzero FO76 body-position lanes"
    );
    let source_bodies = hkx_object_array(source_ragdoll, "bodyCinfos");
    let source_bodies_by_material: HashMap<i64, &HkxValue> = source_bodies
        .iter()
        .map(|body| (hkx_member_i64(body, "materialId"), body))
        .collect();
    assert_eq!(source_bodies_by_material.len(), source_bodies.len());

    let dir = temp_dir("convert_real_fo76_radbeaver_ragdoll");
    let dst = dir.join("out").join("Skeleton.nif");
    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert RadBeaver skeleton");
    assert!(report.supported, "{:?}", report.errors);

    let converted = NifFile::load(dst).expect("load converted RadBeaver skeleton");
    let output_blob = embedded_havok_blob(&converted).expect("converted RadBeaver ragdoll blob");
    let output_hkx =
        havok_native::hkx::model::HkxFile::read(&output_blob).expect("parse converted ragdoll");
    let output_ragdoll = output_hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpRagdollData")
        .expect("converted ragdoll data");
    let output_bodies = hkx_object_array(output_ragdoll, "bodyCinfos");
    let output_motions = hkx_object_array(output_ragdoll, "motionCinfos");

    assert_eq!(output_bodies.len(), 22);
    assert_eq!(output_motions.len(), output_bodies.len());
    let output_spheres: Vec<_> = output_hkx
        .objects()
        .iter()
        .filter(|object| object.class_name == "hknpSphereShape")
        .collect();
    assert_eq!(output_spheres.len(), 1);
    let sphere_vertices = member_value(&output_spheres[0].members, "vertices")
        .and_then(|value| match value {
            HkxValue::Array(values) => Some(values),
            _ => None,
        })
        .expect("sphere support vertices");
    assert_eq!(sphere_vertices.len(), 4, "FO4 sphere support width");
    assert!(
        sphere_vertices.windows(2).all(|pair| pair[0] == pair[1]),
        "FO4 sphere support lanes must describe the same center"
    );
    let output_shape_mass_properties: Vec<_> = output_hkx
        .objects()
        .iter()
        .filter(|object| object.class_name == "hknpShapeMassProperties")
        .collect();
    assert_eq!(output_shape_mass_properties.len(), 5);
    for mass_properties in output_shape_mass_properties {
        let compressed = member_value(&mass_properties.members, "compressedMassProperties")
            .and_then(HkxValue::as_object_members)
            .expect("compressed shape mass properties");
        for field_name in ["centerOfMass", "inertia"] {
            let values = member_value(compressed, field_name)
                .and_then(|value| match value {
                    HkxValue::Array(values) => Some(values),
                    _ => None,
                })
                .expect("packed shape mass vector");
            assert_eq!(values.len(), 4, "{field_name} packed width");
            assert!(
                values.iter().any(|value| hkx_int(value) != Some(0)),
                "{field_name} must retain the FO76 packed values"
            );
        }
    }
    for (index, body) in output_bodies.iter().enumerate() {
        assert_eq!(
            hkx_member_vec4(body, "position")[3],
            0.0,
            "body {index} position.w"
        );

        let material_id = hkx_member_i64(body, "materialId");
        let source_body = source_bodies_by_material
            .get(&material_id)
            .unwrap_or_else(|| panic!("source body for material {material_id}"));
        let source_orientation = hkx_member_vec4(source_body, "orientation");
        let output_orientation = hkx_member_vec4(body, "orientation");
        assert_quaternion_equivalent(
            output_orientation,
            source_orientation,
            &format!("body {index} authored orientation"),
        );

        let source_body_members = source_body.as_object_members().expect("source body object");
        let mass_distribution_index = match member_value(source_body_members, "massDistribution") {
            Some(HkxValue::Pointer(Some(index))) => *index,
            other => panic!("body {index} massDistribution pointer: {other:?}"),
        };
        let mass_distribution = &source_hkx.objects()[mass_distribution_index];
        let distribution_members = member_value(&mass_distribution.members, "massDistribution")
            .and_then(HkxValue::as_object_members)
            .expect("source mass distribution object");
        let source_distribution = HkxValue::Object(distribution_members.to_vec());
        let center_and_volume = hkx_member_vec4(&source_distribution, "centerOfMassAndVolume");
        let inertia = hkx_member_vec4(&source_distribution, "inertiaTensor");
        let major_axis = hkx_member_vec4(&source_distribution, "majorAxisSpace");

        let motion_id = hkx_member_i64(body, "motionId") as usize;
        let motion = &output_motions[motion_id];
        let output_position = hkx_member_vec4(body, "position");
        let rotated_com = quat_rotate_vector_xyzw(
            output_orientation,
            [
                center_and_volume[0],
                center_and_volume[1],
                center_and_volume[2],
            ],
        );
        let center_of_mass_world = hkx_member_vec4(motion, "centerOfMassWorld");
        for axis in 0..3 {
            assert!(
                (center_of_mass_world[axis] - (output_position[axis] + rotated_com[axis])).abs()
                    < 1e-5,
                "body {index} centerOfMassWorld axis {axis}"
            );
        }

        let body_mass = hkx_member_f32(source_body, "mass");
        let mass_factor = hkx_member_f32(motion, "massFactor");
        let expected_mass_factor = body_mass / center_and_volume[3];
        assert!(
            (mass_factor - expected_mass_factor).abs() / expected_mass_factor < 1e-5,
            "body {index} massFactor {mass_factor}, expected {expected_mass_factor}"
        );

        let inverse_inertia = hkx_member_vec4(motion, "inverseInertiaLocal");
        for axis in 0..3 {
            let expected_inverse_inertia = 1.0 / (inertia[axis] * body_mass);
            assert!(
                (inverse_inertia[axis] - expected_inverse_inertia).abs() / expected_inverse_inertia
                    < 1e-5,
                "body {index} inverse inertia axis {axis}"
            );
        }
        assert_quaternion_equivalent(
            hkx_member_vec4(motion, "orientation"),
            quat_mul_xyzw(output_orientation, major_axis),
            &format!("body {index} motion orientation"),
        );
    }
    assert!(
        output_motions
            .iter()
            .any(|motion| hkx_member_vec4(motion, "centerOfMassWorld")[3].abs() > 1e-6),
        "motion center lanes should retain the source values"
    );

    let ragdoll_constraints: Vec<_> = output_hkx
        .objects()
        .iter()
        .filter(|object| object.class_name == "hkpRagdollConstraintData")
        .collect();
    assert_eq!(ragdoll_constraints.len(), 13);
    for (index, constraint) in ragdoll_constraints.iter().enumerate() {
        let atoms = member_value(&constraint.members, "atoms")
            .and_then(HkxValue::as_object_members)
            .expect("ragdoll atoms");
        let cone_limit = member_value(atoms, "coneLimit")
            .and_then(HkxValue::as_object_members)
            .expect("ragdoll cone limit");
        let offset = member_value(cone_limit, "memOffsetToAngleOffset")
            .and_then(hkx_int)
            .expect("ragdoll cone offset");
        assert_eq!(offset, 56, "constraint {index} cone runtime offset");
    }

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn convert_real_fo76_dismembered_skeleton_preserves_constrained_body_frames() {
    let src = repo_path("extracted/fo76/Meshes/SetDressing/Skeletons/SkeletonDismembered08.nif");
    if !src.exists() {
        eprintln!("skip: FO76 dismembered skeleton fixture not available");
        return;
    }

    let source_nif = NifFile::load(&src).expect("load source skeleton");
    let source_blob = embedded_havok_blob(&source_nif).expect("source physics blob");
    let source_hkx =
        havok_native::hkx::model::HkxFile::read(&source_blob).expect("parse source physics blob");
    let source_system = source_hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpPhysicsSystemData")
        .expect("source physics system data");

    let dir = temp_dir("convert_real_fo76_dismembered_skeleton");
    let dst = dir.join("out").join("SkeletonDismembered08.nif");
    let report = convert_nif_file(
        &src,
        &dst,
        "fo76",
        "fo4",
        None,
        &ConvertFileOptions::default(),
    )
    .expect("convert dismembered skeleton");
    assert!(report.supported, "{:?}", report.errors);

    let converted = NifFile::load(dst).expect("load converted skeleton");
    let output_blob = embedded_havok_blob(&converted).expect("converted physics blob");
    let output_hkx = havok_native::hkx::model::HkxFile::read(&output_blob)
        .expect("parse converted physics blob");
    let output_system = output_hkx
        .objects()
        .iter()
        .find(|object| object.class_name == "hknpPhysicsSystemData")
        .expect("converted physics system data");

    let source_bodies = hkx_object_array(source_system, "bodyCinfos");
    let output_bodies = hkx_object_array(output_system, "bodyCinfos");
    let output_motions = hkx_object_array(output_system, "motionCinfos");
    assert_eq!(source_bodies.len(), 13);
    assert_eq!(output_bodies.len(), source_bodies.len());
    assert_eq!(output_motions.len(), source_bodies.len());
    assert_eq!(
        hkx_object_array(source_system, "constraintCinfos").len(),
        12
    );
    assert_eq!(
        hkx_object_array(output_system, "constraintCinfos").len(),
        12
    );

    for (index, (source_body, output_body)) in source_bodies.iter().zip(output_bodies).enumerate() {
        assert_eq!(
            hkx_member_vec4(output_body, "position"),
            hkx_member_vec4(source_body, "position"),
            "body {index} position"
        );
        assert_eq!(
            hkx_member_vec4(output_body, "orientation"),
            hkx_member_vec4(source_body, "orientation"),
            "body {index} orientation"
        );
        assert_eq!(hkx_member_i64(output_body, "motionId"), index as i64);
    }

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

fn fo76_named_material_shader_nif() -> NifFile {
    let mut nif = NifFile::new("fo76");
    let shader = nif.add_block(
        "BSLightingShaderProperty",
        Some(fields([(
            "Name",
            NifValue::String(
                "C:\\Projects\\76\\Build\\PC\\Data\\Materials\\Landscape\\Grass\\ForestGrass01.BGSM"
                    .to_string(),
            ),
        )])),
    );
    let texset = nif.add_block(
        "BSShaderTextureSet",
        Some(fields([
            ("Num Textures", NifValue::UInt(15)),
            (
                "Textures",
                NifValue::Array(vec![
                    NifValue::String(
                        "C:\\Projects\\76\\Build\\PC\\Data\\Textures\\landscape\\grass\\forestgrass01_d.dds"
                            .to_string(),
                    ),
                    NifValue::String(
                        "C:\\Projects\\76\\Build\\PC\\Data\\Textures\\landscape\\grass\\forestgrass01_n.dds"
                            .to_string(),
                    ),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                    NifValue::String(
                        "C:\\Projects\\76\\Build\\PC\\Data\\Textures\\Shared\\Default_r.DDS"
                            .to_string(),
                    ),
                    NifValue::String(
                        "C:\\Projects\\76\\Build\\PC\\Data\\Textures\\Shared\\Default_l.DDS"
                            .to_string(),
                    ),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                    NifValue::String(String::new()),
                ]),
            ),
        ])),
    );
    nif.blocks[0].set_field("Num Children", NifValue::UInt(1));
    nif.blocks[0].set_field(
        "Children",
        NifValue::Array(vec![NifValue::Ref(shader as i32)]),
    );
    assert_eq!(texset, shader + 1);
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

fn has_nonzero_vertex_data(nif: &NifFile) -> bool {
    nif.blocks
        .iter()
        .filter(|block| block.type_name == "BSTriShape" || block.type_name == "BSSubIndexTriShape")
        .filter_map(|block| block.get_field("Vertex Data"))
        .flat_map(|value| match value {
            NifValue::Array(vertices) => vertices.iter().collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .filter_map(|value| match value {
            NifValue::Struct(fields) => fields.get("Vertex").and_then(vertex_position),
            _ => None,
        })
        .any(|position| position.iter().any(|component| component.abs() > 0.001))
}

fn vertex_position(value: &NifValue) -> Option<[f32; 3]> {
    match value {
        NifValue::Vec3(position) => Some(*position),
        NifValue::Struct(fields) => Some([
            numeric_value(fields.get("x"))? as f32,
            numeric_value(fields.get("y"))? as f32,
            numeric_value(fields.get("z"))? as f32,
        ]),
        _ => None,
    }
}

fn numeric_value(value: Option<&NifValue>) -> Option<f64> {
    match value? {
        NifValue::Float(value) => Some(*value),
        NifValue::Int(value) => Some(*value as f64),
        NifValue::UInt(value) => Some(*value as f64),
        _ => None,
    }
}

fn fields<const N: usize>(entries: [(&str, NifValue); N]) -> IndexMap<String, NifValue> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}
