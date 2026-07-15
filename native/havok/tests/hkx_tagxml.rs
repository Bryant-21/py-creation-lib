use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use havok_native::hkx::descriptors::{
    ClassDescriptor, ClassKind, DescriptorRegistry, MemberTemplate,
};
use havok_native::hkx::tagxml::{read_tagxml_string, write_tagxml_string};
use havok_native::hkx::types::HkxValue;
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

#[test]
fn parses_basic_tagxml_file() {
    let hkx = read_tagxml_string(SAMPLE_XML).expect("parse sample XML");

    assert_eq!(hkx.class_version(), 11);
    assert_eq!(hkx.contents_version(), "hk_2014.1.0-r1");
    assert_eq!(hkx.objects().len(), 2);
}

#[test]
fn parses_object_names_and_classes() {
    let hkx = read_tagxml_string(SAMPLE_XML).expect("parse sample XML");

    let names: Vec<_> = hkx
        .objects()
        .iter()
        .map(|object| object.name.as_deref())
        .collect();
    let classes: Vec<_> = hkx
        .objects()
        .iter()
        .map(|object| object.class_name.as_str())
        .collect();

    assert!(names.contains(&Some("#0001")));
    assert!(names.contains(&Some("#0002")));
    assert!(classes.contains(&"hkRootLevelContainer"));
    assert!(classes.contains(&"TestClass"));
}

#[test]
fn writes_empty_tagxml_file() {
    let hkx = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", Vec::new());

    let xml = write_tagxml_string(&hkx).expect("write XML");

    assert!(xml.contains("<hkpackfile"));
    assert!(xml.contains("classversion=\"11\""));
    assert!(xml.contains("contentsversion=\"hk_2014.1.0-r1\""));
    assert!(xml.contains("<hksection name=\"__data__\">"));
}

#[test]
fn writes_direct_string_and_array_members() {
    let hkx = HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![HkxObject {
            name: Some("#0001".to_string()),
            offset: 0,
            signature: 0,
            class_name: "TestClass".to_string(),
            members: vec![
                HkxMember {
                    name: "value".to_string(),
                    value: HkxValue::I32(42),
                },
                HkxMember {
                    name: "scale".to_string(),
                    value: HkxValue::F32(1.5),
                },
                HkxMember {
                    name: "name".to_string(),
                    value: HkxValue::String {
                        value: "Hello & Goodbye".to_string(),
                        is_null: false,
                    },
                },
                HkxMember {
                    name: "flags".to_string(),
                    value: HkxValue::Array(vec![
                        HkxValue::I32(1),
                        HkxValue::I32(2),
                        HkxValue::I32(3),
                    ]),
                },
            ],
        }],
    );

    let xml = write_tagxml_string(&hkx).expect("write XML");

    assert!(xml.contains("class=\"TestClass\""));
    assert!(xml.contains("<hkparam name=\"value\">42</hkparam>"));
    assert!(xml.contains("<hkparam name=\"scale\">1.500000</hkparam>"));
    assert!(xml.contains("Hello &amp; Goodbye"));
    assert!(xml.contains("<hkparam name=\"flags\" numelements=\"3\">1 2 3</hkparam>"));
}

#[test]
fn round_trips_tagxml_through_model_structurally() {
    let first = read_tagxml_string(SAMPLE_XML).expect("parse sample XML");

    let xml = write_tagxml_string(&first).expect("write XML");
    let second = read_tagxml_string(&xml).expect("parse written XML");

    assert_eq!(second.class_version(), first.class_version());
    assert_eq!(second.contents_version(), first.contents_version());
    assert_eq!(second.objects(), first.objects());
}

#[test]
fn preserves_object_signatures_across_parse_write_parse() {
    let first = read_tagxml_string(SAMPLE_XML).expect("parse sample XML");
    assert_eq!(first.objects()[0].signature, 0x2772c11e);

    let xml = write_tagxml_string(&first).expect("write XML");
    assert!(xml.contains("signature=\"0x2772c11e\""));
    let second = read_tagxml_string(&xml).expect("parse written XML");

    assert_eq!(second.objects()[0].signature, 0x2772c11e);
}

#[test]
fn parses_pointer_and_null_scalars_without_descriptors() {
    let hkx = read_tagxml_string(SAMPLE_XML).expect("parse sample XML");
    let HkxValue::Array(variants) = &hkx.objects()[0].members[0].value else {
        panic!("namedVariants should parse as an array");
    };
    let HkxValue::Object(members) = &variants[0] else {
        panic!("namedVariants element should parse as an object");
    };
    let variant = members
        .iter()
        .find(|member| member.name == "variant")
        .unwrap();
    assert_eq!(variant.value, HkxValue::Pointer(Some(1)));

    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="Unknown" signature="0x00000000"><hkparam name="target">null</hkparam></hkobject></hksection></hkpackfile>"##;
    let hkx = read_tagxml_string(xml).expect("parse null pointer XML");
    assert_eq!(hkx.objects()[0].members[0].value, HkxValue::Pointer(None));
}

#[test]
fn round_trips_pointer_members() {
    let first = read_tagxml_string(SAMPLE_XML).expect("parse sample XML");

    let xml = write_tagxml_string(&first).expect("write XML");
    assert!(xml.contains("<hkparam name=\"variant\">#0002</hkparam>"));
    let second = read_tagxml_string(&xml).expect("parse written XML");

    assert_eq!(
        second.objects()[0].members[0],
        first.objects()[0].members[0]
    );
}

#[test]
fn descriptor_backed_parser_uses_declared_scalar_types() {
    let dir = temp_classxml_dir("descriptor_scalars");
    write_temp_classxml(
        &dir,
        "TestDescriptor_0.xml",
        "<class name='TestDescriptor' version='0' signature='0x12345678'><members><member name='enabled' offset='0' vtype='TYPE_BOOL' vsubtype='TYPE_VOID'/><member name='count' offset='1' vtype='TYPE_UINT32' vsubtype='TYPE_VOID'/><member name='ratio' offset='5' vtype='TYPE_REAL' vsubtype='TYPE_VOID'/><member name='label' offset='9' vtype='TYPE_STRINGPTR' vsubtype='TYPE_VOID'/><member name='target' offset='17' vtype='TYPE_POINTER' vsubtype='TYPE_VOID'/></members></class>",
    );
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="TestDescriptor" signature="0x12345678"><hkparam name="enabled">true</hkparam><hkparam name="count">4294967295</hkparam><hkparam name="ratio">1.25</hkparam><hkparam name="label">null</hkparam><hkparam name="target">#0001</hkparam></hkobject></hksection></hkpackfile>"##;

    let hkx = havok_native::hkx::tagxml::read_tagxml_string_with_registry(xml, &mut registry)
        .expect("parse descriptor-backed XML");

    assert_eq!(hkx.objects()[0].members[0].value, HkxValue::Bool(true));
    assert_eq!(hkx.objects()[0].members[1].value, HkxValue::U32(u32::MAX));
    assert_eq!(hkx.objects()[0].members[2].value, HkxValue::F32(1.25));
    assert_eq!(
        hkx.objects()[0].members[3].value,
        HkxValue::String {
            value: String::new(),
            is_null: true,
        }
    );
    assert_eq!(
        hkx.objects()[0].members[4].value,
        HkxValue::Pointer(Some(0))
    );
}

#[test]
fn rejects_mixed_object_and_scalar_arrays() {
    let hkx = HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![HkxObject {
            name: Some("#0001".to_string()),
            offset: 0,
            signature: 0,
            class_name: "TestClass".to_string(),
            members: vec![HkxMember {
                name: "mixed".to_string(),
                value: HkxValue::Array(vec![
                    HkxValue::Object(vec![HkxMember {
                        name: "name".to_string(),
                        value: HkxValue::String {
                            value: "nested".to_string(),
                            is_null: false,
                        },
                    }]),
                    HkxValue::I32(7),
                ]),
            }],
        }],
    );

    let error = write_tagxml_string(&hkx).expect_err("mixed arrays should be rejected");

    assert!(error.to_string().contains("mixed object/scalar array"));
}

#[test]
fn descriptor_backed_parser_preserves_direct_inline_structs() {
    let dir = temp_classxml_dir("inline_struct");
    write_temp_classxml(
        &dir,
        "NestedStruct_0.xml",
        "<struct name='NestedStruct' version='0' signature='0x00000002'><members><member name='id' offset='0' vtype='TYPE_INT32' vsubtype='TYPE_VOID'/></members></struct>",
    );
    write_temp_classxml(
        &dir,
        "ContainerClass_0.xml",
        "<class name='ContainerClass' version='0' signature='0x00000001'><members><member name='nested' offset='0' vtype='TYPE_STRUCT' vsubtype='TYPE_VOID' ctype='NestedStruct'/></members></class>",
    );
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="ContainerClass" signature="0x00000001"><hkparam name="nested"><hkobject><hkparam name="id">7</hkparam></hkobject></hkparam></hkobject></hksection></hkpackfile>"##;

    let first = havok_native::hkx::tagxml::read_tagxml_string_with_registry(xml, &mut registry)
        .expect("parse inline struct XML");
    let HkxValue::Object(members) = &first.objects()[0].members[0].value else {
        panic!("direct inline struct should parse as object value");
    };
    assert_eq!(members[0].name, "id");
    assert_eq!(members[0].value, HkxValue::I32(7));

    let written = write_tagxml_string(&first).expect("write inline struct XML");
    assert!(written.contains("<hkparam name=\"nested\">"));
    assert!(written.contains("<hkobject>"));
    assert!(written.contains("<hkparam name=\"id\">7</hkparam>"));

    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    let second =
        havok_native::hkx::tagxml::read_tagxml_string_with_registry(&written, &mut registry)
            .expect("parse written inline struct XML");
    assert_eq!(
        second.objects()[0].members[0],
        first.objects()[0].members[0]
    );
}

#[test]
fn descriptor_backed_round_trip_preserves_textual_enum_names() {
    // Verify that an enum member whose tagxml value is a textual enumitem
    // name (e.g. HK_SPLINE_COMPRESSED_ANIMATION) round-trips with the name
    // preserved, not collapsed to its integer value. Mirrors Python
    // `HKXEnumMember.value`.
    let dir = temp_classxml_dir("textual_enum");
    write_temp_classxml(
        &dir,
        "EnumOwner_0.xml",
        "<class name='EnumOwner' version='0' signature='0xdeadbeef'>\
            <enums>\
                <enum name='AnimationType'>\
                    <enumitem name='HK_UNKNOWN_ANIMATION' value='0'/>\
                    <enumitem name='HK_INTERLEAVED_ANIMATION' value='1'/>\
                    <enumitem name='HK_DELTA_COMPRESSED_ANIMATION' value='2'/>\
                    <enumitem name='HK_WAVELET_COMPRESSED_ANIMATION' value='3'/>\
                    <enumitem name='HK_MIRRORED_ANIMATION' value='4'/>\
                    <enumitem name='HK_SPLINE_COMPRESSED_ANIMATION' value='5'/>\
                </enum>\
            </enums>\
            <members>\
                <member name='kind' offset='0' vtype='TYPE_ENUM' vsubtype='TYPE_INT32' etype='AnimationType'/>\
            </members>\
        </class>",
    );
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="EnumOwner" signature="0xdeadbeef"><hkparam name="kind">HK_SPLINE_COMPRESSED_ANIMATION</hkparam></hkobject></hksection></hkpackfile>"##;

    let first = havok_native::hkx::tagxml::read_tagxml_string_with_registry(xml, &mut registry)
        .expect("parse textual enum");
    // Stored internally as the integer.
    assert_eq!(first.objects()[0].members[0].value, HkxValue::I32(5));

    // Round-trip writer must emit the textual name, not "5".
    let written =
        havok_native::hkx::tagxml::write_tagxml_string_with_registry(&first, &mut registry)
            .expect("write textual enum");
    assert!(
        written.contains("HK_SPLINE_COMPRESSED_ANIMATION"),
        "writer did not preserve textual enum name; xml=\n{}",
        written
    );
    assert!(
        !written.contains(">5</hkparam>"),
        "writer collapsed enum to integer; xml=\n{}",
        written
    );

    // And re-parsing the written XML yields the same value.
    let second =
        havok_native::hkx::tagxml::read_tagxml_string_with_registry(&written, &mut registry)
            .expect("re-parse textual enum");
    assert_eq!(second.objects()[0].members[0].value, HkxValue::I32(5));
}

#[test]
fn descriptor_backed_parser_decodes_packed_hex_uint8_array() {
    // hkxpack-cli emits small-int arrays (hkArray<hkUint8> etc.) as a packed
    // hex blob with no whitespace separators. Verify our reader falls back to
    // that format when the standard whitespace-separated decode fails.
    let dir = temp_classxml_dir("packed_hex_uint8");
    write_temp_classxml(
        &dir,
        "PackedHexClass_0.xml",
        "<class name='PackedHexClass' version='0' signature='0xdeadbeef'><members><member name='blob' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_UINT8'/></members></class>",
    );
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    // Bytes: 0x00, 0xFF, 0x10, 0x80, 0x7F — packed hex string is "00ff10807f".
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="PackedHexClass" signature="0xdeadbeef"><hkparam name="blob" numelements="5">00ff10807f</hkparam></hkobject></hksection></hkpackfile>"##;

    let hkx = havok_native::hkx::tagxml::read_tagxml_string_with_registry(xml, &mut registry)
        .expect("parse packed-hex uint8 array");

    let HkxValue::Array(values) = &hkx.objects()[0].members[0].value else {
        panic!("expected array");
    };
    assert_eq!(values.len(), 5);
    assert_eq!(values[0], HkxValue::U8(0x00));
    assert_eq!(values[1], HkxValue::U8(0xFF));
    assert_eq!(values[2], HkxValue::U8(0x10));
    assert_eq!(values[3], HkxValue::U8(0x80));
    assert_eq!(values[4], HkxValue::U8(0x7F));
}

#[test]
fn descriptor_backed_parser_decodes_packed_hex_int8_signed_values() {
    // Verify two's-complement decoding for signed small ints. Bytes:
    //   0xFF -> -1, 0x80 -> -128, 0x7F -> 127, 0x00 -> 0.
    let dir = temp_classxml_dir("packed_hex_int8");
    write_temp_classxml(
        &dir,
        "PackedHexI8Class_0.xml",
        "<class name='PackedHexI8Class' version='0' signature='0xdeadbeef'><members><member name='blob' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_INT8'/></members></class>",
    );
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="PackedHexI8Class" signature="0xdeadbeef"><hkparam name="blob" numelements="4">ff807f00</hkparam></hkobject></hksection></hkpackfile>"##;

    let hkx = havok_native::hkx::tagxml::read_tagxml_string_with_registry(xml, &mut registry)
        .expect("parse packed-hex int8 array");

    let HkxValue::Array(values) = &hkx.objects()[0].members[0].value else {
        panic!("expected array");
    };
    assert_eq!(
        values,
        &vec![
            HkxValue::I8(-1),
            HkxValue::I8(-128),
            HkxValue::I8(127),
            HkxValue::I8(0),
        ]
    );
}

#[test]
fn descriptor_backed_parser_falls_back_to_packed_hex_when_decimal_fails() {
    // The legacy whitespace-decimal form should still parse (control case).
    let dir = temp_classxml_dir("packed_hex_decimal_form");
    write_temp_classxml(
        &dir,
        "DecArr_0.xml",
        "<class name='DecArr' version='0' signature='0xdeadbeef'><members><member name='blob' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_UINT8'/></members></class>",
    );
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="DecArr" signature="0xdeadbeef"><hkparam name="blob" numelements="3">1 2 3</hkparam></hkobject></hksection></hkpackfile>"##;

    let hkx = havok_native::hkx::tagxml::read_tagxml_string_with_registry(xml, &mut registry)
        .expect("parse decimal uint8 array");

    let HkxValue::Array(values) = &hkx.objects()[0].members[0].value else {
        panic!("expected array");
    };
    assert_eq!(
        values,
        &vec![HkxValue::U8(1), HkxValue::U8(2), HkxValue::U8(3)]
    );
}

#[test]
fn binary_model_tagxml_roundtrip_preserves_representative_shape() {
    let data =
        std::fs::read(repo_path("native/havok/tests/fixtures/skeleton.hkx")).expect("read fixture");
    let first = havok_native::hkx::read_packfile(&data).expect("read binary HKX");
    let first_object = first.objects().first().expect("fixture has objects");
    let first_member_kinds: Vec<_> = first_object.members.iter().map(member_kind).collect();

    let xml = write_tagxml_string(&first).expect("write XML");
    let second = read_tagxml_string(&xml).expect("parse written XML");
    let second_object = second.objects().first().expect("roundtrip has objects");
    let second_member_kinds: Vec<_> = second_object.members.iter().map(member_kind).collect();

    assert_eq!(second.objects().len(), first.objects().len());
    assert_eq!(
        second_object.name.as_deref(),
        first_object.name.as_deref().or(Some("#0001"))
    );
    assert_eq!(second_object.class_name, first_object.class_name);
    assert_eq!(second_object.signature, first_object.signature);
    assert_eq!(second_member_kinds, first_member_kinds);
}

fn member_kind(member: &HkxMember) -> &'static str {
    match member.value {
        HkxValue::Void => "void",
        HkxValue::Bool(_) => "bool",
        HkxValue::I8(_) => "i8",
        HkxValue::U8(_) => "u8",
        HkxValue::I16(_) => "i16",
        HkxValue::U16(_) => "u16",
        HkxValue::I32(_) => "i32",
        HkxValue::U32(_) => "u32",
        HkxValue::I64(_) => "i64",
        HkxValue::U64(_) => "u64",
        HkxValue::F32(_) => "f32",
        HkxValue::Half(_) => "half",
        HkxValue::F32List(_) => "f32list",
        HkxValue::String { .. } => "string",
        HkxValue::Pointer(_) => "pointer",
        HkxValue::Array(_) => "array",
        HkxValue::Object(_) => "object",
        HkxValue::TypedObject { .. } => "typed_object",
        HkxValue::PendingPtr(_) => "pending_ptr",
    }
}

// ---------------------------------------------------------------------------
// per-field defaulted-init metadata
// ---------------------------------------------------------------------------

#[test]
fn writer_omits_members_matching_template_default() {
    // Build a registry with a class descriptor that has a default value.
    let mut registry = DescriptorRegistry::new();
    registry.insert(ClassDescriptor {
        name: "DefaultedClass".to_string(),
        version: 0,
        signature: "0xaabbccdd".to_string(),
        parent: None,
        is_struct: false,
        members: vec![
            MemberTemplate {
                name: "required".to_string(),
                offset: 0,
                vtype: havok_native::hkx::types::HkxType::Int32,
                vsubtype: havok_native::hkx::types::HkxType::Void,
                ctype: String::new(),
                arrsize: 0,
                flags: "FLAGS_NONE".to_string(),
                etype: String::new(),
                default: None,
            },
            MemberTemplate {
                name: "optional".to_string(),
                offset: 4,
                vtype: havok_native::hkx::types::HkxType::Int32,
                vsubtype: havok_native::hkx::types::HkxType::Void,
                ctype: String::new(),
                arrsize: 0,
                flags: "FLAGS_NONE".to_string(),
                etype: String::new(),
                default: Some(HkxValue::I32(0)),
            },
        ],
        enums: std::collections::HashMap::new(),
        kind: ClassKind::Runtime,
    });

    let hkx = havok_native::hkx::HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![havok_native::hkx::HkxObject {
            name: Some("#0001".to_string()),
            offset: 0,
            signature: 0xaabbccdd,
            class_name: "DefaultedClass".to_string(),
            members: vec![
                havok_native::hkx::HkxMember {
                    name: "required".to_string(),
                    value: HkxValue::I32(42),
                },
                havok_native::hkx::HkxMember {
                    name: "optional".to_string(),
                    value: HkxValue::I32(0), // matches default
                },
            ],
        }],
    );

    let xml = havok_native::hkx::tagxml::write_tagxml_string_with_registry(&hkx, &mut registry)
        .expect("write with defaults");

    // required=42 must appear
    assert!(
        xml.contains("<hkparam name=\"required\">42</hkparam>"),
        "required member must appear; xml=\n{xml}"
    );
    // optional=0 matches the template default and must be omitted
    assert!(
        !xml.contains("optional"),
        "defaulted member must be omitted; xml=\n{xml}"
    );
}

#[test]
fn reader_fills_in_defaulted_member_absent_from_xml() {
    // Same descriptor with a default on "optional".
    let mut registry = DescriptorRegistry::new();
    registry.insert(ClassDescriptor {
        name: "DefaultedClass".to_string(),
        version: 0,
        signature: "0xaabbccdd".to_string(),
        parent: None,
        is_struct: false,
        members: vec![
            MemberTemplate {
                name: "required".to_string(),
                offset: 0,
                vtype: havok_native::hkx::types::HkxType::Int32,
                vsubtype: havok_native::hkx::types::HkxType::Void,
                ctype: String::new(),
                arrsize: 0,
                flags: "FLAGS_NONE".to_string(),
                etype: String::new(),
                default: None,
            },
            MemberTemplate {
                name: "optional".to_string(),
                offset: 4,
                vtype: havok_native::hkx::types::HkxType::Int32,
                vsubtype: havok_native::hkx::types::HkxType::Void,
                ctype: String::new(),
                arrsize: 0,
                flags: "FLAGS_NONE".to_string(),
                etype: String::new(),
                default: Some(HkxValue::I32(99)),
            },
        ],
        enums: std::collections::HashMap::new(),
        kind: ClassKind::Runtime,
    });

    // XML that omits "optional"
    let xml = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="DefaultedClass" signature="0xaabbccdd"><hkparam name="required">7</hkparam></hkobject></hksection></hkpackfile>"##;

    let hkx = havok_native::hkx::tagxml::read_tagxml_string_with_registry(xml, &mut registry)
        .expect("parse xml with omitted defaulted member");

    let members = &hkx.objects()[0].members;
    assert_eq!(members.len(), 2, "reader must inject the defaulted member");

    let required = members.iter().find(|m| m.name == "required").unwrap();
    assert_eq!(required.value, HkxValue::I32(7));

    let optional = members.iter().find(|m| m.name == "optional").unwrap();
    assert_eq!(
        optional.value,
        HkxValue::I32(99),
        "defaulted member must be filled in with the template default"
    );
}

// ---------------------------------------------------------------------------
// COMPLEX (Vector4 / QsTransform / Matrix4) Python-compatible format
// ---------------------------------------------------------------------------

#[test]
fn writes_vector4_in_parenthesized_form_matching_python_tagwriter() {
    let dir = temp_classxml_dir("vector4");
    write_temp_classxml(
        &dir,
        "VecHolder_0.xml",
        "<class name='VecHolder' version='0' signature='0x00000010'><members><member name='translation' offset='0' vtype='TYPE_VECTOR4' vsubtype='TYPE_VOID'/></members></class>",
    );
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    let hkx = HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![HkxObject {
            name: Some("#0001".to_string()),
            offset: 0,
            signature: 0x00000010,
            class_name: "VecHolder".to_string(),
            members: vec![HkxMember {
                name: "translation".to_string(),
                value: HkxValue::F32List(vec![1.0, 2.0, 3.0, 1.0]),
            }],
        }],
    );

    let xml = havok_native::hkx::tagxml::write_tagxml_string_with_registry(&hkx, &mut registry)
        .expect("write XML");
    assert!(
        xml.contains("(1.000000 2.000000 3.000000 1.000000)"),
        "Vector4 should emit in parenthesized form, got:\n{xml}"
    );
}

#[test]
fn writes_qstransform_array_with_per_element_parens_matching_python_tagwriter() {
    // Mirrors `py_creation_lib/python/creation_lib/hkxpack/tagwriter.py:164-169`: arrays of COMPLEX subtype
    // wrap each element (which is a flat 12-float list for QsTransform) in
    // ONE pair of parens. NOT three groups of four. Python's tagreader
    // regex `\(([^)]+)\)` then yields one element per parenthesized group,
    // recovering the original element count.
    let dir = temp_classxml_dir("qst_array");
    write_temp_classxml(
        &dir,
        "Pose_0.xml",
        "<class name='Pose' version='0' signature='0x00000020'><members><member name='referencePose' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_QSTRANSFORM'/></members></class>",
    );
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    let hkx = HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![HkxObject {
            name: Some("#0001".to_string()),
            offset: 0,
            signature: 0x00000020,
            class_name: "Pose".to_string(),
            members: vec![HkxMember {
                name: "referencePose".to_string(),
                value: HkxValue::Array(vec![
                    HkxValue::F32List(vec![
                        0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0,
                    ]),
                    HkxValue::F32List(vec![
                        2.0, 2.0, 2.0, 1.0, 0.5, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0,
                    ]),
                ]),
            }],
        }],
    );

    let xml = havok_native::hkx::tagxml::write_tagxml_string_with_registry(&hkx, &mut registry)
        .expect("write XML");
    let expected_first = "(0.000000 0.000000 0.000000 1.000000 0.000000 0.000000 0.000000 1.000000 1.000000 1.000000 1.000000 1.000000)";
    let expected_second = "(2.000000 2.000000 2.000000 1.000000 0.500000 0.500000 0.500000 0.500000 1.000000 1.000000 1.000000 1.000000)";
    assert!(
        xml.contains(expected_first),
        "first QsTransform should be one paren group, got:\n{xml}"
    );
    assert!(
        xml.contains(expected_second),
        "second QsTransform should be one paren group, got:\n{xml}"
    );
    // numelements="2" — the array has two elements, not six.
    assert!(xml.contains("numelements=\"2\""));
}

#[test]
fn round_trips_parenthesized_vector4_through_parse_and_write() {
    let dir = temp_classxml_dir("vector4_rt");
    write_temp_classxml(
        &dir,
        "VecHolder_0.xml",
        "<class name='VecHolder' version='0' signature='0x00000010'><members><member name='translation' offset='0' vtype='TYPE_VECTOR4' vsubtype='TYPE_VOID'/></members></class>",
    );
    let xml_in = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="VecHolder" signature="0x00000010"><hkparam name="translation">(1.500000 -2.250000 3.875000 1.000000)</hkparam></hkobject></hksection></hkpackfile>"##;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    let parsed = havok_native::hkx::tagxml::read_tagxml_string_with_registry(xml_in, &mut registry)
        .expect("parse parenthesized Vector4");
    let HkxValue::F32List(values) = &parsed.objects()[0].members[0].value else {
        panic!(
            "expected F32List, got {:?}",
            parsed.objects()[0].members[0].value
        );
    };
    assert_eq!(values.len(), 4);
    assert!((values[0] - 1.5).abs() < 1e-6);
    assert!((values[1] - -2.25).abs() < 1e-6);
    assert!((values[2] - 3.875).abs() < 1e-6);
    assert!((values[3] - 1.0).abs() < 1e-6);

    let xml_out =
        havok_native::hkx::tagxml::write_tagxml_string_with_registry(&parsed, &mut registry)
            .expect("write XML");
    assert!(xml_out.contains("(1.500000 -2.250000 3.875000 1.000000)"));
}

#[test]
fn round_trips_qstransform_array_through_parse_and_write() {
    let dir = temp_classxml_dir("qst_rt");
    write_temp_classxml(
        &dir,
        "Pose_0.xml",
        "<class name='Pose' version='0' signature='0x00000020'><members><member name='pose' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_QSTRANSFORM'/></members></class>",
    );
    // Each QsTransform element is wrapped in ONE paren group with all 12 floats.
    let xml_in = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="Pose" signature="0x00000020"><hkparam name="pose" numelements="2">(0.000000 0.000000 0.000000 1.000000 0.000000 0.000000 0.000000 1.000000 1.000000 1.000000 1.000000 1.000000)
(2.000000 2.000000 2.000000 1.000000 0.500000 0.500000 0.500000 0.500000 1.000000 1.000000 1.000000 1.000000)</hkparam></hkobject></hksection></hkpackfile>"##;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    let parsed = havok_native::hkx::tagxml::read_tagxml_string_with_registry(xml_in, &mut registry)
        .expect("parse QsTransform array");
    let HkxValue::Array(values) = &parsed.objects()[0].members[0].value else {
        panic!(
            "expected Array, got {:?}",
            parsed.objects()[0].members[0].value
        );
    };
    assert_eq!(values.len(), 2);
    for value in values {
        let HkxValue::F32List(floats) = value else {
            panic!("each QsTransform element should be F32List, got {value:?}");
        };
        assert_eq!(floats.len(), 12);
    }

    let xml_out =
        havok_native::hkx::tagxml::write_tagxml_string_with_registry(&parsed, &mut registry)
            .expect("write XML");
    assert!(xml_out.contains(
        "(2.000000 2.000000 2.000000 1.000000 0.500000 0.500000 0.500000 0.500000 1.000000 1.000000 1.000000 1.000000)"
    ));
}

#[test]
fn flat_complex_array_input_decodes_into_per_element_groups() {
    // Flat space-separated COMPLEX arrays (from non-Python emitters) must still
    // parse: the parser rebundles flat float runs into per-element F32List
    // groups.
    let dir = temp_classxml_dir("flat_complex");
    write_temp_classxml(
        &dir,
        "Pose_0.xml",
        "<class name='Pose' version='0' signature='0x00000020'><members><member name='pose' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_VECTOR4'/></members></class>",
    );
    let xml_in = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="Pose" signature="0x00000020"><hkparam name="pose" numelements="2">1.000000 2.000000 3.000000 4.000000 5.000000 6.000000 7.000000 8.000000</hkparam></hkobject></hksection></hkpackfile>"##;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    let parsed = havok_native::hkx::tagxml::read_tagxml_string_with_registry(xml_in, &mut registry)
        .expect("parse flat Vector4 array");
    let HkxValue::Array(values) = &parsed.objects()[0].members[0].value else {
        panic!(
            "expected Array, got {:?}",
            parsed.objects()[0].members[0].value
        );
    };
    assert_eq!(values.len(), 2);
    let HkxValue::F32List(first) = &values[0] else {
        panic!("first element should be F32List");
    };
    assert_eq!(first, &vec![1.0, 2.0, 3.0, 4.0]);
    let HkxValue::F32List(second) = &values[1] else {
        panic!("second element should be F32List");
    };
    assert_eq!(second, &vec![5.0, 6.0, 7.0, 8.0]);
}

#[test]
fn round_trips_string_array_hkcstring_children() {
    let dir = temp_classxml_dir("string_array_hkcstring");
    write_temp_classxml(
        &dir,
        "StringArrayHolder_0.xml",
        "<class name='StringArrayHolder' version='0' signature='0x00000030'><members><member name='names' offset='0' vtype='TYPE_ARRAY' vsubtype='TYPE_STRINGPTR'/></members></class>",
    );
    let xml_in = r##"<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1"><hksection name="__data__"><hkobject name="#0001" class="StringArrayHolder" signature="0x00000030"><hkparam name="names" numelements="2"><hkcstring>first value</hkcstring><hkcstring>second value</hkcstring></hkparam></hkobject></hksection></hkpackfile>"##;
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    let parsed = havok_native::hkx::tagxml::read_tagxml_string_with_registry(xml_in, &mut registry)
        .expect("parse string array");
    let HkxValue::Array(values) = &parsed.objects()[0].members[0].value else {
        panic!("expected string array");
    };
    assert_eq!(values.len(), 2);
    assert!(matches!(
        &values[0],
        HkxValue::String { value, is_null: false } if value == "first value"
    ));
    assert!(matches!(
        &values[1],
        HkxValue::String { value, is_null: false } if value == "second value"
    ));

    let xml_out =
        havok_native::hkx::tagxml::write_tagxml_string_with_registry(&parsed, &mut registry)
            .expect("write XML");
    assert!(xml_out.contains("<hkcstring>first value</hkcstring>"));
    assert!(xml_out.contains("<hkcstring>second value</hkcstring>"));
}

#[test]
fn round_trips_string_with_xml_special_characters() {
    // strings containing XML special characters must survive a
    // write→read round-trip without corruption or parse failure.
    let special = "a < b & c > d \" e ' f\ng";
    let dir = temp_classxml_dir("special_chars");
    write_temp_classxml(
        &dir,
        "StrHolder_0.xml",
        "<class name='StrHolder' version='0' signature='0x00000040'><members><member name='label' offset='0' vtype='TYPE_STRINGPTR' vsubtype='TYPE_VOID'/></members></class>",
    );
    let mut registry = DescriptorRegistry::from_dir(&dir).expect("temp descriptors");
    let hkx = HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![HkxObject {
            name: Some("#0001".to_string()),
            offset: 0,
            signature: 0x00000040,
            class_name: "StrHolder".to_string(),
            members: vec![HkxMember {
                name: "label".to_string(),
                value: HkxValue::String {
                    value: special.to_string(),
                    is_null: false,
                },
            }],
        }],
    );

    let xml = havok_native::hkx::tagxml::write_tagxml_string_with_registry(&hkx, &mut registry)
        .expect("write must succeed with special chars");

    // The raw special characters must not appear unescaped in text content.
    // (The writer must escape them before emitting.)
    assert!(!xml.contains(" < "), "unescaped < in xml");
    assert!(!xml.contains(" > "), "unescaped > in xml");

    let parsed = havok_native::hkx::tagxml::read_tagxml_string_with_registry(&xml, &mut registry)
        .expect("re-parse must succeed");

    let HkxValue::String {
        value,
        is_null: false,
    } = &parsed.objects()[0].members[0].value
    else {
        panic!(
            "expected String, got {:?}",
            parsed.objects()[0].members[0].value
        );
    };
    assert_eq!(
        value, special,
        "round-trip must preserve all special chars exactly"
    );
}
