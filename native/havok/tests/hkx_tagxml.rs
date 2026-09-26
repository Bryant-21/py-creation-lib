use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use havok_native::hkx::descriptors::{
    ClassDescriptor, ClassKind, DescriptorRegistry, MemberTemplate,
};
use havok_native::hkx::tagxml::{
    read_tagxml_string, read_tagxml_string_with_registry, write_tagxml_string,
    write_tagxml_string_with_registry,
};
use havok_native::hkx::types::{HkxType, HkxValue};
use havok_native::hkx::{HkxFile, HkxMember, HkxObject};

const SAMPLE_XML: &str = r##"<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
    <hksection name="__data__">
        <hkobject name="#0001" class="hkRootLevelContainer" signature="0x2772c11e">
            <hkparam name="namedVariants" numelements="1">
                <hkobject>
                    <hkparam name="name">TestVariant</hkparam>
                    <hkparam name="className">hkbBehaviorGraph</hkparam>
                    <hkparam name="variant">#0002</hkparam>
                </hkobject>
            </hkparam>
        </hkobject>
        <hkobject name="#0002" class="TestClass" signature="0x00000000">
            <hkparam name="value">42</hkparam>
            <hkparam name="scale">1.500000</hkparam>
            <hkparam name="name">Hello &amp; Goodbye</hkparam>
            <hkparam name="flags" numelements="3">1 2 3</hkparam>
        </hkobject>
    </hksection>
</hkpackfile>
"##;

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn temp_classxml_dir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("havok_native_tagxml_{name}_{stamp}"));
    std::fs::create_dir_all(&dir).expect("create temp classxml dir");
    dir
}

fn write_temp_classxml(dir: &Path, filename: &str, xml: &str) {
    std::fs::write(dir.join(filename), xml).expect("write temp classxml");
}

fn registry_with(name: &str, files: &[(&str, &str)]) -> DescriptorRegistry {
    let dir = temp_classxml_dir(name);
    for (file, xml) in files {
        write_temp_classxml(&dir, file, xml);
    }
    DescriptorRegistry::from_dir(&dir).expect("temp descriptors")
}

fn single_object_packfile(class: &str, signature: &str, params: &str) -> String {
    format!(
        r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="{class}" signature="{signature}">{params}</hkobject></hksection></hkpackfile>"##
    )
}

fn single_object_file(class: &str, signature: u32, members: Vec<HkxMember>) -> HkxFile {
    HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![HkxObject {
            name: Some("#0001".to_string()),
            offset: 0,
            signature,
            class_name: class.to_string(),
            members,
        }],
    )
}

fn member(name: &str, value: HkxValue) -> HkxMember {
    HkxMember {
        name: name.to_string(),
        value,
    }
}

fn string(value: &str) -> HkxValue {
    HkxValue::String {
        value: value.to_string(),
        is_null: false,
    }
}

#[test]
fn sample_xml_parses_and_round_trips_structurally() {
    let first = read_tagxml_string(SAMPLE_XML).expect("parse sample XML");
    assert_eq!(first.class_version(), 11);
    assert_eq!(first.contents_version(), "hk_2014.1.0-r1");
    let names: Vec<_> = first.objects().iter().map(|o| o.name.as_deref()).collect();
    let classes: Vec<_> = first
        .objects()
        .iter()
        .map(|o| o.class_name.as_str())
        .collect();
    assert_eq!(names, [Some("#0001"), Some("#0002")]);
    assert_eq!(classes, ["hkRootLevelContainer", "TestClass"]);
    assert_eq!(first.objects()[0].signature, 0x2772c11e);

    let HkxValue::Array(variants) = &first.objects()[0].members[0].value else {
        panic!("namedVariants should parse as an array");
    };
    let HkxValue::Object(members) = &variants[0] else {
        panic!("namedVariants element should parse as an object");
    };
    let variant = members.iter().find(|m| m.name == "variant").unwrap();
    assert_eq!(variant.value, HkxValue::Pointer(Some(1)));

    let xml = write_tagxml_string(&first).expect("write XML");
    assert!(xml.contains("signature=\"0x2772c11e\""));
    assert!(xml.contains("<hkparam name=\"variant\">#0002</hkparam>"));
    let second = read_tagxml_string(&xml).expect("parse written XML");
    assert_eq!(second.class_version(), first.class_version());
    assert_eq!(second.contents_version(), first.contents_version());
    assert_eq!(second.objects(), first.objects());

    let null_target = single_object_packfile(
        "Unknown",
        "0x00000000",
        r#"<hkparam name="target">null</hkparam>"#,
    );
    let hkx = read_tagxml_string(&null_target).expect("parse null pointer XML");
    assert_eq!(hkx.objects()[0].members[0].value, HkxValue::Pointer(None));
}

#[test]
fn writer_emits_header_and_direct_members_and_rejects_mixed_arrays() {
    let empty = write_tagxml_string(&HkxFile::from_tagxml(11, "hk_2014.1.0-r1", Vec::new()))
        .expect("write empty XML");
    for needle in [
        "<hkpackfile",
        "classversion=\"11\"",
        "contentsversion=\"hk_2014.1.0-r1\"",
        "<hksection name=\"__data__\">",
    ] {
        assert!(empty.contains(needle), "{needle}");
    }

    let hkx = single_object_file(
        "TestClass",
        0,
        vec![
            member("value", HkxValue::I32(42)),
            member("scale", HkxValue::F32(1.5)),
            member("name", string("Hello & Goodbye")),
            member(
                "flags",
                HkxValue::Array(vec![HkxValue::I32(1), HkxValue::I32(2), HkxValue::I32(3)]),
            ),
        ],
    );
    let xml = write_tagxml_string(&hkx).expect("write XML");
    for needle in [
        "class=\"TestClass\"",
        "<hkparam name=\"value\">42</hkparam>",
        "<hkparam name=\"scale\">1.500000</hkparam>",
        "Hello &amp; Goodbye",
        "<hkparam name=\"flags\" numelements=\"3\">1 2 3</hkparam>",
    ] {
        assert!(xml.contains(needle), "{needle}");
    }

    let mixed = single_object_file(
        "TestClass",
        0,
        vec![member(
            "mixed",
            HkxValue::Array(vec![
                HkxValue::Object(vec![member("name", string("nested"))]),
                HkxValue::I32(7),
            ]),
        )],
    );
    let error = write_tagxml_string(&mixed).expect_err("mixed arrays should be rejected");
    assert!(error.to_string().contains("mixed object/scalar array"));
}

#[test]
fn descriptor_backed_parser_uses_declared_types_and_resolves_object_labels() {
    let mut registry = registry_with(
        "descriptor_scalars",
        &[
            (
                "TestDescriptor_0.xml",
                "<class name='TestDescriptor' version='0' signature='0x12345678'><members><member name='enabled' offset='0' vtype='TYPE_BOOL' vsubtype='TYPE_VOID'/><member name='count' offset='1' vtype='TYPE_UINT32' vsubtype='TYPE_VOID'/><member name='ratio' offset='5' vtype='TYPE_REAL' vsubtype='TYPE_VOID'/><member name='label' offset='9' vtype='TYPE_STRINGPTR' vsubtype='TYPE_VOID'/><member name='target' offset='17' vtype='TYPE_POINTER' vsubtype='TYPE_VOID'/></members></class>",
            ),
            (
                "PointerOwner_0.xml",
                "<class name='PointerOwner' version='0' signature='0x12345678'><members><member name='target' offset='0' vtype='TYPE_POINTER' vsubtype='TYPE_STRUCT' ctype='PointerTarget'/></members></class>",
            ),
        ],
    );
    let xml = single_object_packfile(
        "TestDescriptor",
        "0x12345678",
        r#"<hkparam name="enabled">true</hkparam><hkparam name="count">4294967295</hkparam><hkparam name="ratio">1.25</hkparam><hkparam name="label">null</hkparam><hkparam name="target">#0001</hkparam>"#,
    );
    let hkx =
        read_tagxml_string_with_registry(&xml, &mut registry).expect("parse descriptor-backed XML");
    let values: Vec<_> = hkx.objects()[0]
        .members
        .iter()
        .map(|m| m.value.clone())
        .collect();
    assert_eq!(
        values,
        vec![
            HkxValue::Bool(true),
            HkxValue::U32(u32::MAX),
            HkxValue::F32(1.25),
            HkxValue::String {
                value: String::new(),
                is_null: true,
            },
            HkxValue::Pointer(Some(0)),
        ]
    );

    let non_positional = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0090" class="PointerOwner" signature="0x12345678"><hkparam name="target">#0100</hkparam></hkobject><hkobject name="#0100" class="PointerTarget" signature="0x00000000"/></hksection></hkpackfile>"##;
    let hkx = read_tagxml_string_with_registry(non_positional, &mut registry)
        .expect("parse non-positional object labels");
    assert_eq!(
        hkx.objects()[0].members[0].value,
        HkxValue::Pointer(Some(1))
    );
}

#[test]
fn descriptor_backed_parser_preserves_direct_inline_structs() {
    let files = [
        (
            "NestedStruct_0.xml",
            "<struct name='NestedStruct' version='0' signature='0x00000002'><members><member name='id' offset='0' vtype='TYPE_INT32' vsubtype='TYPE_VOID'/></members></struct>",
        ),
        (
            "ContainerClass_0.xml",
            "<class name='ContainerClass' version='0' signature='0x00000001'><members><member name='nested' offset='0' vtype='TYPE_STRUCT' vsubtype='TYPE_VOID' ctype='NestedStruct'/></members></class>",
        ),
    ];
    let mut registry = registry_with("inline_struct", &files);
    let xml = single_object_packfile(
        "ContainerClass",
        "0x00000001",
        r#"<hkparam name="nested"><hkobject><hkparam name="id">7</hkparam></hkobject></hkparam>"#,
    );

    let first =
        read_tagxml_string_with_registry(&xml, &mut registry).expect("parse inline struct XML");
    let HkxValue::Object(members) = &first.objects()[0].members[0].value else {
        panic!("direct inline struct should parse as object value");
    };
    assert_eq!(members[0].name, "id");
    assert_eq!(members[0].value, HkxValue::I32(7));

    let written = write_tagxml_string(&first).expect("write inline struct XML");
    assert!(written.contains("<hkparam name=\"nested\">"));
    assert!(written.contains("<hkobject>"));
    assert!(written.contains("<hkparam name=\"id\">7</hkparam>"));

    let mut registry = registry_with("inline_struct_reread", &files);
    let second = read_tagxml_string_with_registry(&written, &mut registry)
        .expect("parse written inline struct XML");
    assert_eq!(
        second.objects()[0].members[0],
        first.objects()[0].members[0]
    );
}

#[test]
fn descriptor_backed_round_trip_preserves_textual_enum_names() {
    let mut registry = registry_with(
        "textual_enum",
        &[(
            "EnumOwner_0.xml",
            "<class name='EnumOwner' version='0' signature='0xdeadbeef'>\
                <enums><enum name='AnimationType'>\
                    <enumitem name='HK_UNKNOWN_ANIMATION' value='0'/>\
                    <enumitem name='HK_SPLINE_COMPRESSED_ANIMATION' value='5'/>\
                </enum></enums>\
                <members><member name='kind' offset='0' vtype='TYPE_ENUM' vsubtype='TYPE_INT32' etype='AnimationType'/></members>\
            </class>",
        )],
    );
    let xml = single_object_packfile(
        "EnumOwner",
        "0xdeadbeef",
        r#"<hkparam name="kind">HK_SPLINE_COMPRESSED_ANIMATION</hkparam>"#,
    );

    let first = read_tagxml_string_with_registry(&xml, &mut registry).expect("parse textual enum");
    assert_eq!(first.objects()[0].members[0].value, HkxValue::I32(5));

    let written =
        write_tagxml_string_with_registry(&first, &mut registry).expect("write textual enum");
    assert!(written.contains("HK_SPLINE_COMPRESSED_ANIMATION"), "{written}");
    assert!(!written.contains(">5</hkparam>"), "{written}");

    let second =
        read_tagxml_string_with_registry(&written, &mut registry).expect("re-parse textual enum");
    assert_eq!(second.objects()[0].members[0].value, HkxValue::I32(5));
}

#[test]
fn small_int_arrays_decode_packed_hex_or_decimal_forms() {
    // hkxpack-cli emits small-int arrays as a packed hex blob with no
    // separators; the reader falls back to it when whitespace decode fails.
    let mut registry = registry_with(
        "packed_hex",
        &[
            (
                "U8Array_0.xml",
                "<class name='U8Array' version='0' signature='0xdeadbeef'><members><member name='blob' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_UINT8'/></members></class>",
            ),
            (
                "I8Array_0.xml",
                "<class name='I8Array' version='0' signature='0xdeadbeef'><members><member name='blob' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_INT8'/></members></class>",
            ),
        ],
    );
    for (class, count, text, expected) in [
        (
            "U8Array",
            5,
            "00ff10807f",
            [0x00, 0xFF, 0x10, 0x80, 0x7F].map(HkxValue::U8).to_vec(),
        ),
        (
            "I8Array",
            4,
            "ff807f00",
            [-1, -128, 127, 0].map(HkxValue::I8).to_vec(),
        ),
        ("U8Array", 3, "1 2 3", [1, 2, 3].map(HkxValue::U8).to_vec()),
    ] {
        let xml = single_object_packfile(
            class,
            "0xdeadbeef",
            &format!(r#"<hkparam name="blob" numelements="{count}">{text}</hkparam>"#),
        );
        let hkx = read_tagxml_string_with_registry(&xml, &mut registry)
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        assert_eq!(
            hkx.objects()[0].members[0].value,
            HkxValue::Array(expected),
            "{text}"
        );
    }
}

#[test]
fn binary_model_tagxml_roundtrip_preserves_representative_shape() {
    let data =
        std::fs::read(repo_path("native/havok/tests/fixtures/skeleton.hkx")).expect("read fixture");
    let first = havok_native::hkx::read_packfile(&data).expect("read binary HKX");
    let first_object = first.objects().first().expect("fixture has objects");
    let first_member_kinds: Vec<_> = first_object
        .members
        .iter()
        .map(|m| std::mem::discriminant(&m.value))
        .collect();

    let xml = write_tagxml_string(&first).expect("write XML");
    let second = read_tagxml_string(&xml).expect("parse written XML");
    let second_object = second.objects().first().expect("roundtrip has objects");
    let second_member_kinds: Vec<_> = second_object
        .members
        .iter()
        .map(|m| std::mem::discriminant(&m.value))
        .collect();

    assert_eq!(second.objects().len(), first.objects().len());
    assert_eq!(
        second_object.name.as_deref(),
        first_object.name.as_deref().or(Some("#0001"))
    );
    assert_eq!(second_object.class_name, first_object.class_name);
    assert_eq!(second_object.signature, first_object.signature);
    assert_eq!(second_member_kinds, first_member_kinds);
}

fn defaulted_registry(optional_default: i32) -> DescriptorRegistry {
    let template = |name: &str, offset, default| MemberTemplate {
        name: name.to_string(),
        offset,
        vtype: HkxType::Int32,
        vsubtype: HkxType::Void,
        ctype: String::new(),
        arrsize: 0,
        flags: "FLAGS_NONE".to_string(),
        etype: String::new(),
        default,
    };
    let mut registry = DescriptorRegistry::new();
    registry.insert(ClassDescriptor {
        name: "DefaultedClass".to_string(),
        version: 0,
        signature: "0xaabbccdd".to_string(),
        parent: None,
        is_struct: false,
        members: vec![
            template("required", 0, None),
            template("optional", 4, Some(HkxValue::I32(optional_default))),
        ],
        enums: std::collections::HashMap::new(),
        kind: ClassKind::Runtime,
    });
    registry
}

#[test]
fn template_defaults_are_omitted_on_write_and_filled_on_read() {
    let mut registry = defaulted_registry(0);
    let hkx = single_object_file(
        "DefaultedClass",
        0xaabbccdd,
        vec![
            member("required", HkxValue::I32(42)),
            member("optional", HkxValue::I32(0)),
        ],
    );
    let xml = write_tagxml_string_with_registry(&hkx, &mut registry).expect("write with defaults");
    assert!(
        xml.contains("<hkparam name=\"required\">42</hkparam>"),
        "{xml}"
    );
    assert!(!xml.contains("optional"), "{xml}");

    let mut registry = defaulted_registry(99);
    let xml = single_object_packfile(
        "DefaultedClass",
        "0xaabbccdd",
        r#"<hkparam name="required">7</hkparam>"#,
    );
    let hkx = read_tagxml_string_with_registry(&xml, &mut registry)
        .expect("parse xml with omitted defaulted member");
    assert_eq!(
        hkx.objects()[0].members,
        vec![
            member("required", HkxValue::I32(7)),
            member("optional", HkxValue::I32(99)),
        ]
    );
}

#[test]
fn complex_values_use_one_paren_group_per_element() {
    // Mirrors hkxpack/tagwriter.py: each COMPLEX element (a flat 12-float list
    // for QsTransform) is ONE paren group, so the reader recovers the count.
    let mut registry = registry_with(
        "complex",
        &[
            (
                "VecHolder_0.xml",
                "<class name='VecHolder' version='0' signature='0x00000010'><members><member name='translation' offset='0' vtype='TYPE_VECTOR4' vsubtype='TYPE_VOID'/></members></class>",
            ),
            (
                "Pose_0.xml",
                "<class name='Pose' version='0' signature='0x00000020'><members><member name='pose' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_QSTRANSFORM'/></members></class>",
            ),
            (
                "VecArray_0.xml",
                "<class name='VecArray' version='0' signature='0x00000030'><members><member name='vectors' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_VECTOR4'/></members></class>",
            ),
        ],
    );

    let vector = "(1.500000 -2.250000 3.875000 1.000000)";
    let qs_a = "(0.000000 0.000000 0.000000 1.000000 0.000000 0.000000 0.000000 1.000000 1.000000 1.000000 1.000000 1.000000)";
    let qs_b = "(2.000000 2.000000 2.000000 1.000000 0.500000 0.500000 0.500000 0.500000 1.000000 1.000000 1.000000 1.000000)";
    for (class, signature, param, expected) in [
        (
            "VecHolder",
            "0x00000010",
            format!(r#"<hkparam name="translation">{vector}</hkparam>"#),
            HkxValue::F32List(vec![1.5, -2.25, 3.875, 1.0]),
        ),
        (
            "Pose",
            "0x00000020",
            format!("<hkparam name=\"pose\" numelements=\"2\">{qs_a}\n{qs_b}</hkparam>"),
            HkxValue::Array(vec![
                HkxValue::F32List(vec![
                    0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0,
                ]),
                HkxValue::F32List(vec![
                    2.0, 2.0, 2.0, 1.0, 0.5, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0,
                ]),
            ]),
        ),
    ] {
        let xml = single_object_packfile(class, signature, &param);
        let parsed = read_tagxml_string_with_registry(&xml, &mut registry)
            .unwrap_or_else(|error| panic!("{class}: {error}"));
        assert_eq!(parsed.objects()[0].members[0].value, expected, "{class}");
        let written = write_tagxml_string_with_registry(&parsed, &mut registry)
            .unwrap_or_else(|error| panic!("{class} write: {error}"));
        if class == "Pose" {
            assert!(written.contains(qs_a) && written.contains(qs_b), "{written}");
            assert!(written.contains("numelements=\"2\""), "{written}");
        } else {
            assert!(written.contains(vector), "{written}");
        }
    }

    // Flat COMPLEX arrays from non-Python emitters are rebundled per element.
    let flat = single_object_packfile(
        "VecArray",
        "0x00000030",
        r#"<hkparam name="vectors" numelements="2">1.000000 2.000000 3.000000 4.000000 5.000000 6.000000 7.000000 8.000000</hkparam>"#,
    );
    let parsed =
        read_tagxml_string_with_registry(&flat, &mut registry).expect("parse flat Vector4 array");
    assert_eq!(
        parsed.objects()[0].members[0].value,
        HkxValue::Array(vec![
            HkxValue::F32List(vec![1.0, 2.0, 3.0, 4.0]),
            HkxValue::F32List(vec![5.0, 6.0, 7.0, 8.0]),
        ])
    );
}

#[test]
fn strings_round_trip_as_hkcstring_arrays_and_with_xml_special_characters() {
    let mut registry = registry_with(
        "strings",
        &[
            (
                "StringArrayHolder_0.xml",
                "<class name='StringArrayHolder' version='0' signature='0x00000030'><members><member name='names' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_STRINGPTR'/></members></class>",
            ),
            (
                "StrHolder_0.xml",
                "<class name='StrHolder' version='0' signature='0x00000040'><members><member name='label' offset='0' vtype='TYPE_STRINGPTR' vsubtype='TYPE_VOID'/></members></class>",
            ),
        ],
    );
    let xml = single_object_packfile(
        "StringArrayHolder",
        "0x00000030",
        r#"<hkparam name="names" numelements="2"><hkcstring>first value</hkcstring><hkcstring>second value</hkcstring></hkparam>"#,
    );
    let parsed = read_tagxml_string_with_registry(&xml, &mut registry).expect("parse string array");
    assert_eq!(
        parsed.objects()[0].members[0].value,
        HkxValue::Array(vec![string("first value"), string("second value")])
    );
    let written = write_tagxml_string_with_registry(&parsed, &mut registry).expect("write XML");
    assert!(written.contains("<hkcstring>first value</hkcstring>"));
    assert!(written.contains("<hkcstring>second value</hkcstring>"));

    let special = "a < b & c > d \" e ' f\ng";
    let hkx = single_object_file(
        "StrHolder",
        0x00000040,
        vec![member("label", string(special))],
    );
    let written = write_tagxml_string_with_registry(&hkx, &mut registry)
        .expect("write must succeed with special chars");
    assert!(!written.contains(" < "), "unescaped < in xml");
    assert!(!written.contains(" > "), "unescaped > in xml");
    let parsed =
        read_tagxml_string_with_registry(&written, &mut registry).expect("re-parse must succeed");
    assert_eq!(parsed.objects()[0].members[0].value, string(special));
}
