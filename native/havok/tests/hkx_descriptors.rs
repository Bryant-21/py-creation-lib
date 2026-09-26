use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use havok_native::hkx::descriptors::{ClassKind, DescriptorRegistry, StructureLayout};
use havok_native::hkx::types::{HkxType, HkxTypeFamily, HkxValue, deserialize_member_value};

fn member<'a>(
    desc: &'a havok_native::hkx::descriptors::ClassDescriptor,
    name: &str,
) -> &'a havok_native::hkx::descriptors::MemberTemplate {
    desc.members
        .iter()
        .find(|member| member.name == name)
        .expect("member exists")
}

fn temp_classxml_dir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("havok_native_{name}_{stamp}"));
    fs::create_dir_all(&dir).expect("create temp classxml dir");
    dir
}

fn member_names(desc: &havok_native::hkx::descriptors::ClassDescriptor) -> Vec<&str> {
    desc.members.iter().map(|m| m.name.as_str()).collect()
}

#[test]
fn bundled_fo4_descriptors_expose_members_offsets_inheritance_and_enums() {
    let mut registry = DescriptorRegistry::new();

    let skeleton = registry.get("hkaSkeleton").unwrap().expect("hkaSkeleton");
    assert_eq!(skeleton.parent.as_deref(), Some("hkReferencedObject"));
    assert_eq!(skeleton.kind, ClassKind::Runtime);
    let names = member_names(skeleton);
    for name in ["name", "parentIndices", "bones", "referencePose"] {
        assert!(names.contains(&name), "{name}");
    }
    let name = member(skeleton, "name");
    assert_eq!((name.vtype, name.offset), (HkxType::StringPtr, 16));
    let bones = member(skeleton, "bones");
    assert_eq!(
        (
            bones.vtype,
            bones.vsubtype,
            bones.ctype.as_str(),
            bones.offset
        ),
        (HkxType::Array, HkxType::Struct, "hkaBone", 40)
    );

    let all_members = registry.get_all_members("hkaSkeleton").unwrap();
    let names: Vec<&str> = all_members.iter().map(|m| m.name.as_str()).collect();
    let position = |needle: &str| names.iter().position(|name| *name == needle).unwrap();
    assert!(position("memSizeAndRefCount") < position("name"));

    let bone = registry.get("hkaBone").unwrap().expect("hkaBone");
    assert!(bone.is_struct);
    assert!(bone.parent.as_deref().unwrap_or_default().is_empty());
    assert!(member_names(bone).contains(&"lockTranslation"));

    let spline = registry
        .get("hkaSplineCompressedAnimation")
        .unwrap()
        .expect("hkaSplineCompressedAnimation");
    assert_eq!(spline.parent.as_deref(), Some("hkaAnimation"));
    assert!(member_names(spline).contains(&"numFrames"));
    let animation = registry.get("hkaAnimation").unwrap().expect("hkaAnimation");
    assert_eq!(
        animation.enums["AnimationType"].name_to_value("HK_SPLINE_COMPRESSED_ANIMATION"),
        Some(3)
    );
    assert_eq!(
        registry.get_enum_value("hkaAnimation", "AnimationType", 3),
        "HK_SPLINE_COMPRESSED_ANIMATION"
    );
    assert_eq!(
        registry.get_enum_int(
            "hkaAnimation",
            "AnimationType",
            "HK_SPLINE_COMPRESSED_ANIMATION"
        ),
        3
    );

    assert!(registry.get("hkNonExistentClass").unwrap().is_none());
    assert_eq!(registry.class_kind("hclSimClothSetup"), ClassKind::Setup);
    assert_eq!(
        registry.class_kind("hkNonExistentClass"),
        ClassKind::Runtime
    );
}

#[test]
fn generic_layout_reuses_base_class_tail_padding() {
    let mut registry = DescriptorRegistry::new();
    registry.set_structure_layout(StructureLayout::Generic);

    let animation = registry
        .get_all_members("hkaSplineCompressedAnimation")
        .expect("generic animation layout");
    assert_eq!(
        animation
            .iter()
            .find(|member| member.name == "type")
            .unwrap()
            .offset,
        12
    );
    assert_eq!(
        animation
            .iter()
            .find(|member| member.name == "extractedMotion")
            .unwrap()
            .offset,
        32
    );

    let reference_frame = registry
        .get_all_members("hkaDefaultAnimatedReferenceFrame")
        .expect("generic reference-frame layout");
    assert_eq!(
        reference_frame
            .iter()
            .find(|member| member.name == "up")
            .unwrap()
            .offset,
        16
    );
}

#[test]
fn versioned_registries_load_known_roots_and_fall_back_only_by_contents_version() {
    let mut registry = DescriptorRegistry::for_version("2015").expect("2015 classxml exists");
    let desc = registry
        .get("hkaSkeleton")
        .unwrap()
        .expect("2015 hkaSkeleton");
    assert_eq!(desc.version, 6);

    let err = DescriptorRegistry::for_version("definitely_missing")
        .expect_err("missing version should not silently fall back");
    assert!(err.to_string().contains("classxml_definitely_missing"));

    for version in [
        "hk_2014.1.0-r1",
        "hk_2015.1.0-r1",
        "hk_2012.1.0-r1",
        "",
        "hk_2009.1.0",
    ] {
        let mut registry = DescriptorRegistry::for_contents_version(version);
        assert!(registry.get("hkaSkeleton").is_ok(), "{version:?}");
    }
}

#[test]
fn from_dir_scans_without_index_and_rejects_malformed_cyclic_or_unsafe_input() {
    let dir = temp_classxml_dir("scan");
    fs::write(
        dir.join("hkTempThing_2.xml"),
        "<class name='hkTempThing' version='2' signature='0x1'><members><member name='value' type='hkInt32' offset='0' vtype='TYPE_INT32' vsubtype='TYPE_VOID'/></members></class>",
    )
    .expect("write temp classxml");
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("load scanned registry");
    let desc = registry
        .get("hkTempThing")
        .unwrap()
        .expect("scanned descriptor");
    assert_eq!(desc.version, 2);
    assert_eq!(member(desc, "value").vtype, HkxType::Int32);
    fs::remove_dir_all(dir).ok();

    for (name, xml) in [
        (
            "missing_name",
            "<class version='0'><members><member name='value' offset='0' vtype='TYPE_INT32' vsubtype='TYPE_VOID'/></members></class>",
        ),
        (
            "bad_vtype",
            "<class name='hkBad' version='0'><members><member name='value' offset='0' vtype='TYPE_NOT_REAL' vsubtype='TYPE_VOID'/></members></class>",
        ),
        (
            "bad_vsubtype",
            "<class name='hkBad' version='0'><members><member name='value' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_NOT_REAL'/></members></class>",
        ),
        (
            "bad_offset",
            "<class name='hkBad' version='0'><members><member name='value' offset='nope' vtype='TYPE_INT32' vsubtype='TYPE_VOID'/></members></class>",
        ),
        (
            "bad_arrsize",
            "<class name='hkBad' version='0'><members><member name='value' offset='0' arrsize='nope' vtype='TYPE_INT32' vsubtype='TYPE_VOID'/></members></class>",
        ),
        (
            "bad_enum",
            "<class name='hkBad' version='0'><enums><enum name='Bad'><enumitem name='BAD' value='nope'/></enum></enums></class>",
        ),
    ] {
        let dir = temp_classxml_dir(name);
        fs::write(dir.join("hkBad_0.xml"), xml).expect("write bad classxml");
        fs::write(dir.join("_index.json"), r#"{"hkBad":"hkBad_0.xml"}"#).expect("write index");

        let mut registry = DescriptorRegistry::from_dir(&dir).expect("load registry");
        assert!(registry.get("hkBad").is_err(), "case {name} should fail");

        fs::remove_dir_all(dir).ok();
    }

    let dir = temp_classxml_dir("cycle");
    fs::write(
        dir.join("hkA_0.xml"),
        "<class name='hkA' version='0' parent='hkB'><members><member name='a' offset='0' vtype='TYPE_INT32' vsubtype='TYPE_VOID'/></members></class>",
    )
    .expect("write hkA");
    fs::write(
        dir.join("hkB_0.xml"),
        "<class name='hkB' version='0' parent='hkA'><members><member name='b' offset='4' vtype='TYPE_INT32' vsubtype='TYPE_VOID'/></members></class>",
    )
    .expect("write hkB");
    fs::write(
        dir.join("_index.json"),
        r#"{"hkA":"hkA_0.xml","hkB":"hkB_0.xml"}"#,
    )
    .expect("write index");

    let mut registry = DescriptorRegistry::from_dir(&dir).expect("load registry");
    let err = registry
        .get_all_members("hkA")
        .expect_err("inheritance cycle should fail");
    assert!(err.to_string().contains("cycle"));

    fs::remove_dir_all(dir).ok();

    for (name, filename) in [
        ("absolute", "C:/outside.xml"),
        ("parent", "../outside.xml"),
        ("separator", "nested/hkBad_0.xml"),
        ("extension", "hkBad_0.txt"),
    ] {
        let dir = temp_classxml_dir(name);
        fs::write(
            dir.join("_index.json"),
            format!(r#"{{"hkBad":"{filename}"}}"#),
        )
        .expect("write unsafe index");

        assert!(
            DescriptorRegistry::from_dir(&dir).is_err(),
            "case {name} should fail"
        );

        fs::remove_dir_all(dir).ok();
    }
}

#[test]
fn hkx_types_expose_sizes_names_and_deserialize_by_member_subtype() {
    assert_eq!(HkxType::Int16.size(), 2);
    assert_eq!(HkxType::Array.size(), 16);
    assert_eq!(HkxType::Transform.size(), 64);
    assert_eq!(HkxType::StringPtr.family(), HkxTypeFamily::String);
    assert_eq!(
        HkxType::from_classxml_name("TYPE_QSTRANSFORM"),
        Some(HkxType::QsTransform)
    );
    assert_eq!(HkxType::Struct.classxml_name(), "TYPE_STRUCT");

    let vector_bytes: Vec<u8> = [1.0_f32, 2.0, 3.0, 4.0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    for (ty, bytes, expected) in [
        (HkxType::Bool, &[1][..], HkxValue::Bool(true)),
        (HkxType::Int16, &[0x34, 0x12][..], HkxValue::I16(0x1234)),
        (
            HkxType::Uint32,
            &[0x78, 0x56, 0x34, 0x12][..],
            HkxValue::U32(0x12345678),
        ),
        (
            HkxType::Vector4,
            &vector_bytes[..],
            HkxValue::F32List(vec![1.0, 2.0, 3.0, 4.0]),
        ),
    ] {
        assert_eq!(ty.deserialize(bytes), Some(expected), "{ty:?}");
    }
    assert!(HkxType::Pointer.deserialize(&[0; 8]).is_none());

    for (ty, subtype, bytes, expected) in [
        (
            HkxType::Enum,
            HkxType::Uint8,
            &[0xFE][..],
            HkxValue::U8(0xFE),
        ),
        (HkxType::Enum, HkxType::Int8, &[0xFE][..], HkxValue::I8(-2)),
        (
            HkxType::Flags,
            HkxType::Uint16,
            &[0x34, 0x12][..],
            HkxValue::U16(0x1234),
        ),
        (
            HkxType::Flags,
            HkxType::Uint32,
            &[0x78, 0x56, 0x34, 0x12][..],
            HkxValue::U32(0x12345678),
        ),
    ] {
        assert_eq!(
            deserialize_member_value(ty, subtype, bytes),
            Some(expected),
            "{ty:?}/{subtype:?}"
        );
    }
}
