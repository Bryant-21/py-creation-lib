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

#[test]
fn loads_hka_skeleton_descriptor() {
    let mut registry = DescriptorRegistry::new();

    let desc = registry
        .get("hkaSkeleton")
        .expect("descriptor parse succeeds")
        .expect("hkaSkeleton descriptor");

    assert_eq!(desc.name, "hkaSkeleton");
    assert_eq!(desc.parent.as_deref(), Some("hkReferencedObject"));
    let names: Vec<&str> = desc
        .members
        .iter()
        .map(|member| member.name.as_str())
        .collect();
    assert!(names.contains(&"name"));
    assert!(names.contains(&"parentIndices"));
    assert!(names.contains(&"bones"));
    assert!(names.contains(&"referencePose"));
}

#[test]
fn parses_member_types_and_offsets() {
    let mut registry = DescriptorRegistry::new();
    let desc = registry
        .get("hkaSkeleton")
        .expect("descriptor parse succeeds")
        .expect("hkaSkeleton descriptor");

    let name = member(desc, "name");
    assert_eq!(name.vtype, HkxType::StringPtr);
    assert_eq!(name.offset, 16);

    let bones = member(desc, "bones");
    assert_eq!(bones.vtype, HkxType::Array);
    assert_eq!(bones.vsubtype, HkxType::Struct);
    assert_eq!(bones.ctype, "hkaBone");
    assert_eq!(bones.offset, 40);
}

#[test]
fn resolves_inherited_members_in_offset_order() {
    let mut registry = DescriptorRegistry::new();

    let all_members = registry
        .get_all_members("hkaSkeleton")
        .expect("inheritance resolves");
    let names: Vec<&str> = all_members
        .iter()
        .map(|member| member.name.as_str())
        .collect();

    assert!(names.contains(&"memSizeAndRefCount"));
    assert!(
        names
            .iter()
            .position(|name| *name == "memSizeAndRefCount")
            .unwrap()
            < names.iter().position(|name| *name == "name").unwrap()
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
fn loads_animation_inheritance_and_enum_lookup() {
    let mut registry = DescriptorRegistry::new();

    let desc = registry
        .get("hkaSplineCompressedAnimation")
        .expect("descriptor parse succeeds")
        .expect("hkaSplineCompressedAnimation descriptor");
    assert_eq!(desc.parent.as_deref(), Some("hkaAnimation"));
    let names: Vec<&str> = desc
        .members
        .iter()
        .map(|member| member.name.as_str())
        .collect();
    assert!(names.contains(&"numFrames"));
    assert!(names.contains(&"data"));

    let animation = registry
        .get("hkaAnimation")
        .expect("descriptor parse succeeds")
        .expect("hkaAnimation descriptor");
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
}

#[test]
fn loads_struct_class_without_parent() {
    let mut registry = DescriptorRegistry::new();

    let desc = registry
        .get("hkaBone")
        .expect("descriptor parse succeeds")
        .expect("hkaBone descriptor");

    assert!(desc.is_struct);
    assert!(desc.parent.as_deref().unwrap_or_default().is_empty());
    let names: Vec<&str> = desc
        .members
        .iter()
        .map(|member| member.name.as_str())
        .collect();
    assert!(names.contains(&"name"));
    assert!(names.contains(&"lockTranslation"));
}

#[test]
fn unknown_class_returns_none() {
    let mut registry = DescriptorRegistry::new();

    assert!(
        registry
            .get("hkNonExistentClass")
            .expect("unknown class is not a parse error")
            .is_none()
    );
}

#[test]
fn loads_versioned_classxml_roots() {
    let mut registry = DescriptorRegistry::for_version("2015").expect("2015 classxml exists");

    let desc = registry
        .get("hkaSkeleton")
        .expect("descriptor parse succeeds")
        .expect("2015 hkaSkeleton descriptor");

    assert_eq!(desc.name, "hkaSkeleton");
    assert_eq!(desc.version, 6);
}

#[test]
fn missing_versioned_classxml_root_is_an_error() {
    let err = DescriptorRegistry::for_version("definitely_missing")
        .expect_err("missing version should not silently fall back");

    assert!(err.to_string().contains("classxml_definitely_missing"));
}

#[test]
fn falls_back_to_scanning_xml_when_index_is_absent() {
    let dir = temp_classxml_dir("scan");
    fs::write(
        dir.join("hkTempThing_2.xml"),
        "<class name='hkTempThing' version='2' signature='0x1'><members><member name='value' type='hkInt32' offset='0' vtype='TYPE_INT32' vsubtype='TYPE_VOID'/></members></class>",
    )
    .expect("write temp classxml");

    let mut registry = DescriptorRegistry::from_dir(&dir).expect("load scanned registry");
    let desc = registry
        .get("hkTempThing")
        .expect("descriptor parse succeeds")
        .expect("scanned descriptor");

    assert_eq!(desc.version, 2);
    assert_eq!(member(desc, "value").vtype, HkxType::Int32);

    fs::remove_dir_all(dir).ok();
}

#[test]
fn hkx_types_expose_sizes_families_and_classxml_names() {
    assert_eq!(HkxType::Int16.size(), 2);
    assert_eq!(HkxType::Array.size(), 16);
    assert_eq!(HkxType::Transform.size(), 64);
    assert_eq!(HkxType::StringPtr.family(), HkxTypeFamily::String);
    assert_eq!(
        HkxType::from_classxml_name("TYPE_QSTRANSFORM"),
        Some(HkxType::QsTransform)
    );
    assert_eq!(HkxType::Struct.classxml_name(), "TYPE_STRUCT");
}

#[test]
fn deserializes_primitive_and_complex_values() {
    assert_eq!(
        HkxType::Bool.deserialize(&[1]).expect("bool"),
        HkxValue::Bool(true)
    );
    assert_eq!(
        HkxType::Int16.deserialize(&[0x34, 0x12]).expect("int16"),
        HkxValue::I16(0x1234)
    );
    assert_eq!(
        HkxType::Uint32
            .deserialize(&[0x78, 0x56, 0x34, 0x12])
            .expect("uint32"),
        HkxValue::U32(0x12345678)
    );

    let mut vector_bytes = Vec::new();
    for value in [1.0_f32, 2.0, 3.0, 4.0] {
        vector_bytes.extend_from_slice(&value.to_le_bytes());
    }
    assert_eq!(
        HkxType::Vector4
            .deserialize(&vector_bytes)
            .expect("vector4"),
        HkxValue::F32List(vec![1.0, 2.0, 3.0, 4.0])
    );

    assert!(HkxType::Pointer.deserialize(&[0; 8]).is_none());
}

#[test]
fn deserializes_enum_and_flags_with_member_subtype_width_and_signedness() {
    assert_eq!(
        deserialize_member_value(HkxType::Enum, HkxType::Uint8, &[0xFE]).expect("enum uint8"),
        HkxValue::U8(0xFE)
    );
    assert_eq!(
        deserialize_member_value(HkxType::Enum, HkxType::Int8, &[0xFE]).expect("enum int8"),
        HkxValue::I8(-2)
    );
    assert_eq!(
        deserialize_member_value(HkxType::Flags, HkxType::Uint16, &[0x34, 0x12])
            .expect("flags uint16"),
        HkxValue::U16(0x1234)
    );
    assert_eq!(
        deserialize_member_value(HkxType::Flags, HkxType::Uint32, &[0x78, 0x56, 0x34, 0x12])
            .expect("flags uint32"),
        HkxValue::U32(0x12345678)
    );
}

#[test]
fn rejects_malformed_classxml_attributes() {
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
}

#[test]
fn detects_inheritance_cycles() {
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
}

#[test]
fn rejects_unsafe_index_paths() {
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
fn for_contents_version_known_versions_load_without_panic() {
    // verify all known versions map to a valid registry.
    for version in &["hk_2014.1.0-r1", "hk_2015.1.0-r1", "hk_2012.1.0-r1", ""] {
        let _registry = DescriptorRegistry::for_contents_version(version);
        // Implicit assertion: no panic.
    }
}

#[test]
fn for_contents_version_unknown_version_falls_back_to_fo4() {
    // completely unknown prefix falls back to FO4 (doesn't panic),
    // but the warning log captures the unknown version. We can't easily
    // intercept tracing warnings in unit tests, so we just verify the
    // fallback produces a usable registry (can look up hkaSkeleton).
    let mut registry = DescriptorRegistry::for_contents_version("hk_2009.1.0");
    // hkaSkeleton must resolve (it's in the default FO4 classxml).
    let result = registry.get("hkaSkeleton");
    assert!(
        result.is_ok(),
        "for_contents_version fallback must produce a usable registry"
    );
}

#[test]
fn class_kind_setup_classes_tagged_correctly() {
    // classes whose names contain "Setup" must be tagged ClassKind::Setup;
    // ordinary runtime classes must default to ClassKind::Runtime.
    let mut registry = DescriptorRegistry::new();

    // hkaSkeleton is a pure runtime class.
    let skeleton = registry
        .get("hkaSkeleton")
        .expect("parse ok")
        .expect("hkaSkeleton exists");
    assert_eq!(skeleton.kind, ClassKind::Runtime);

    // class_kind() fallback: a name containing "Setup" that has no classxml.
    let kind = registry.class_kind("hclSimClothSetup");
    assert_eq!(kind, ClassKind::Setup);

    // class_kind() fallback: ordinary unknown name → Runtime.
    let kind = registry.class_kind("hkNonExistentClass");
    assert_eq!(kind, ClassKind::Runtime);
}
