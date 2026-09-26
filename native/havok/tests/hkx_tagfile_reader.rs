use std::collections::HashSet;
use std::path::PathBuf;

use havok_native::api;
use havok_native::hkx::HkxFile;
use havok_native::hkx::tagfile::{
    TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection, parse_tagfile,
    read_vle, read_vle_signed,
};
use havok_native::hkx::types::HkxValue;

fn fo76_tag0_fixture() -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../python/creation_lib/hkxpack/tests/fixtures/fo76_snallygastercharacter.hkx");
    std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn hff_leaf(tag: &[u8; 4], content: &[u8]) -> Vec<u8> {
    let size = (8 + content.len()) as u32;
    let mut data = Vec::new();
    data.extend_from_slice(&(0x4000_0000 | size).to_be_bytes());
    data.extend_from_slice(tag);
    data.extend_from_slice(content);
    data
}

fn synthetic_tagfile(children: Vec<Vec<u8>>) -> Vec<u8> {
    let children = children.concat();
    let size = (8 + children.len()) as u32;
    let mut data = Vec::new();
    data.extend_from_slice(&size.to_be_bytes());
    data.extend_from_slice(b"TAG0");
    data.extend_from_slice(&children);
    data
}

fn vle_21(value: u32) -> [u8; 3] {
    [
        0xC0 | (((value >> 16) as u8) & 0x1F),
        (value >> 8) as u8,
        value as u8,
    ]
}

fn tag_type(
    id: usize,
    name: &str,
    kind: u8,
    subtype_id: usize,
    size: usize,
    fields: Vec<TagField>,
) -> TagType {
    TagType {
        id,
        name: name.into(),
        parent_id: 0,
        kind,
        subtype_id,
        size,
        align: size.clamp(1, 16),
        version: i64::from(kind == 7 || kind == 6 && !fields.is_empty()),
        format_value: 0,
        signed: false,
        fields,
    }
}

fn field(name: &str, type_id: usize) -> TagField {
    TagField {
        name: name.into(),
        type_id,
        offset: 0,
        flags: 0,
    }
}

fn item(kind: u8, type_id: usize, offset: usize, count: usize) -> TagfileItem {
    TagfileItem {
        kind,
        flags: 0,
        type_id,
        offset,
        count,
    }
}

fn root_sentinel() -> TagType {
    tag_type(0, "hkRootLevelContainer", 0, 0, 0, vec![])
}

fn synthetic_model(types: Vec<TagType>, items: Vec<TagfileItem>, data: Vec<u8>) -> Tagfile {
    Tagfile::from_synthetic_parts(
        "20150100",
        "hk_2015.1.0-r1",
        vec![TagfileSection {
            tag: "DATA".into(),
            offset: 0,
            size: data.len(),
            content_offset: 0,
            content_size: data.len(),
            scope: 1,
        }],
        TagTypeRegistry { types },
        items,
        data,
    )
}

fn member_value(tagfile: &Tagfile, class: Option<&str>, name: &str) -> HkxValue {
    let hkx = tagfile
        .materialize_hkx()
        .unwrap_or_else(|error| panic!("materialize {name}: {error}"));
    let object = hkx
        .objects()
        .iter()
        .find(|o| class.is_none_or(|class| o.class_name == class))
        .expect("object");
    object
        .members
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("missing member {name}"))
        .value
        .clone()
}

#[test]
fn fo76_fixture_parses_sections_registry_and_items() {
    let data = fo76_tag0_fixture();
    assert_ne!(&data[0..4], b"TAG0");
    assert_eq!(&data[4..8], b"TAG0");
    assert_eq!(api::hkx_detect_format(&data).unwrap(), "tagfile");

    let tagfile = parse_tagfile(&data).unwrap();
    assert_eq!(tagfile.sdk_version, "20150100");
    assert_eq!(tagfile.contents_version, "hk_2015.1.0-r1");
    for section in ["TAG0", "SDKV", "DATA", "TSTR", "FSTR", "ITEM"] {
        assert!(tagfile.section(section).is_some(), "{section}");
    }
    assert!(tagfile.section("TNAM").is_some() || tagfile.section("TNA1").is_some());
    assert!(tagfile.section("TBOD").is_some() || tagfile.section("TBDY").is_some());
    for name in ["hkRootLevelContainer", "hkbCharacterData"] {
        assert!(tagfile.type_strings.iter().any(|s| s == name), "{name}");
    }
    for name in ["name", "characterControllerSetup"] {
        assert!(tagfile.field_strings.iter().any(|s| s == name), "{name}");
    }

    assert!(tagfile.type_registry.types.len() >= 100);
    let names: HashSet<&str> = tagfile
        .type_registry
        .types
        .iter()
        .map(|ty| ty.name.as_str())
        .collect();
    for name in [
        "hkRootLevelContainer",
        "hkbCharacterData",
        "hkbCharacterControllerSetup",
    ] {
        assert!(names.contains(name), "{name}");
    }
    let character = tagfile
        .type_registry
        .types
        .iter()
        .find(|ty| ty.name == "hkbCharacterData")
        .unwrap();
    assert!(character.size > 0 && !character.fields.is_empty());

    let alias = &tagfile.type_registry.types[18];
    assert_eq!(
        (alias.id, alias.name.as_str(), alias.kind, alias.size, alias.parent_id),
        (18, "hkVector4", 0, 0, 30)
    );
    let parent = &tagfile.type_registry.types[30];
    assert_eq!(
        (parent.id, parent.name.as_str(), parent.kind, parent.size),
        (30, "hkVector4f", 8, 16)
    );

    assert!(tagfile.items.len() >= 32);
    assert!(tagfile.items.iter().any(|item| item.kind == 1));
    assert!(tagfile.items.iter().any(|item| item.kind == 2));
    assert!(tagfile.items.iter().any(|item| item.count > 1));
    assert!(tagfile.items.iter().any(|item| item.offset > 0));
    if tagfile.section("PTCH").is_some() {
        assert!(!tagfile.pointer_offsets.is_empty());
    }
}

#[test]
fn fo76_fixture_round_trips_unchanged_and_saves_through_writer_once_dirty() {
    let data = fo76_tag0_fixture();
    assert_eq!(api::hkx_roundtrip_bytes(&data).unwrap(), data);

    let hkx = HkxFile::read(&data).expect("HkxFile::read should accept TAG0 tagfiles");
    assert_eq!(hkx.contents_version(), "hk_2015.1.0-r1");
    assert!(!hkx.objects().is_empty());

    let mut hkx = parse_tagfile(&data)
        .unwrap()
        .materialize_hkx()
        .expect("materialize FO76 fixture");
    let _ = hkx.objects_mut();
    let saved = hkx.save();
    assert_ne!(saved, data);
    assert_eq!(
        &saved[0..8],
        &[0x57, 0xE0, 0xE0, 0x57, 0x10, 0xC0, 0xC0, 0x10],
        "save() output should start with packfile magic"
    );
}

#[test]
fn vle_decodes_every_width_and_rejects_invalid_or_truncated_inputs() {
    for (bytes, expected) in [
        (&[0x00][..], (0, 1)),
        (&[0x7F][..], (0x7F, 1)),
        (&[0x80, 0x00][..], (0, 2)),
        (&[0xBF, 0xFF][..], (0x3FFF, 2)),
        (&[0xDF, 0xFF, 0xFF][..], (0x1F_FFFF, 3)),
        (&[0xE0, 0x00, 0x00, 0x00][..], (0, 4)),
        (&[0xE8, 0x00, 0x00, 0x00, 0x00][..], (0, 5)),
        (&[0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00][..], (0, 8)),
        (&[0xF8, 0x00, 0x00, 0x00, 0x00, 0x00][..], (0, 6)),
        (
            &[0xF9, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2A][..],
            (42, 9),
        ),
    ] {
        assert_eq!(read_vle(bytes, 0).unwrap(), expected, "{bytes:02X?}");
    }
    assert_eq!(read_vle_signed(&[0x00], 0).unwrap(), (0, 1));
    assert_eq!(read_vle_signed(&[0x01], 0).unwrap(), (-1, 1));
    assert_eq!(read_vle_signed(&[0x02], 0).unwrap(), (1, 1));

    for prefix in 0xFA..=0xFF {
        let error = read_vle(&[prefix, 0, 0, 0, 0, 0, 0, 0, 0], 0).unwrap_err();
        assert!(error.to_string().contains("invalid VLE prefix"), "{error}");
    }
    for data in [
        &[0x80][..],
        &[0xC0, 0x00][..],
        &[0xE0, 0x00, 0x00][..],
        &[0xE8, 0x00, 0x00, 0x00][..],
        &[0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00][..],
        &[0xF8, 0x00, 0x00, 0x00, 0x00][..],
        &[0xF9, 0][..],
    ] {
        let error = read_vle(data, 0).unwrap_err();
        assert!(error.to_string().contains("needs"), "{error}");
    }
}

#[test]
fn malformed_containers_are_rejected_before_allocation() {
    let mut false_positive = Vec::from(*b"\x57\xE0\xE0\x57");
    false_positive.extend_from_slice(b"nope");
    let error = api::hkx_detect_format(&false_positive).unwrap_err();
    assert!(
        error.to_string().contains("unsupported Havok format"),
        "{error}"
    );

    let mut trailing = synthetic_tagfile(vec![hff_leaf(b"SDKV", b"20150100")]);
    trailing.extend_from_slice(b"junk");
    let mut incomplete_ptch = Vec::new();
    for value in [1u32, 2, 8] {
        incomplete_ptch.extend_from_slice(&value.to_le_bytes());
    }
    let huge_type_id = [vle_21(100_001).as_slice(), &[0x00, 0x00]].concat();
    for (data, message) in [
        (trailing, "trailing HFF bytes"),
        (
            synthetic_tagfile(vec![hff_leaf(b"TNAM", &vle_21(100_001))]),
            "type count",
        ),
        (
            synthetic_tagfile(vec![hff_leaf(b"TBOD", &huge_type_id)]),
            "type id",
        ),
        (
            synthetic_tagfile(vec![hff_leaf(b"ITEM", &[0; 11])]),
            "ITEM section length",
        ),
        (
            synthetic_tagfile(vec![hff_leaf(b"PTCH", &[0; 3])]),
            "PTCH section length",
        ),
        (
            synthetic_tagfile(vec![hff_leaf(b"PTCH", &incomplete_ptch)]),
            "incomplete PTCH group",
        ),
    ] {
        let error = parse_tagfile(&data).unwrap_err();
        assert!(error.to_string().contains(message), "{message}: {error}");
    }
}

#[test]
fn named_complex_types_materialize_as_flat_float_lists() {
    for (type_name, byte_size, float_count) in [
        ("hkQuaternion", 16, 4),
        ("hkQsTransform", 48, 12),
        ("hkMatrix3", 48, 12),
        ("hkMatrix4", 64, 16),
        ("hkTransform", 64, 16),
    ] {
        let tagfile = synthetic_model(
            vec![
                root_sentinel(),
                tag_type(1, type_name, 0, 0, byte_size, vec![]),
                tag_type(2, "TestObject", 6, 0, byte_size, vec![field("value", 1)]),
            ],
            vec![item(0, 0, 0, 0), item(1, 2, 0, 1)],
            vec![0u8; byte_size],
        );
        let HkxValue::F32List(floats) = member_value(&tagfile, None, "value") else {
            panic!("expected F32List for {type_name}");
        };
        assert_eq!(floats.len(), float_count, "{type_name}");
    }
}

#[test]
fn string_fields_decode_tolerantly_and_strip_xml_invalid_controls() {
    // Matches hkxpack/tagfile_reader.py: ascii decode with errors="replace",
    // then drop C0 controls other than \n, \r, \t (hkStringPtr doubles as a
    // scratch field whose bytes may not be text).
    for (payload, expected) in [
        (&b"name_\xE9_end"[..], "name_\u{FFFD}_end"),
        (&b"a\x01b\tc\x07d\ne"[..], "ab\tcd\ne"),
        (&b"hkbStateMachine"[..], "hkbStateMachine"),
    ] {
        let object_size = 4usize;
        let payload_count = payload.len() + 1;
        let mut data = vec![0u8; object_size + payload_count];
        data[0..4].copy_from_slice(&2u32.to_le_bytes());
        data[object_size..object_size + payload.len()].copy_from_slice(payload);
        let mut tagfile = synthetic_model(
            vec![
                root_sentinel(),
                tag_type(1, "hkStringPtr", 3, 0, 4, vec![]),
                tag_type(2, "TestObject", 7, 0, object_size, vec![field("name", 1)]),
            ],
            vec![
                item(0, 0, 0, 0),
                item(1, 2, 0, 1),
                item(2, 1, object_size, payload_count),
            ],
            data,
        );
        // Registering the field in PTCH selects the item-indexed string path.
        tagfile.set_pointer_offsets_for_test(vec![0]);
        let HkxValue::String { value, .. } = member_value(&tagfile, None, "name") else {
            panic!("expected String for {expected:?}");
        };
        assert_eq!(value, expected);
    }
}

#[test]
fn note_kind_pointer_follows_indirection() {
    // SDK hkTagfileReadFormat.cpp:540: a KIND_NOTE item's `count` is the index
    // of the real object item. The parent pointer stores NOTE item 3, which
    // aliases VAR0 item 2 (object index 1).
    let mut data = vec![0u8; 12];
    data[0..4].copy_from_slice(&3u32.to_le_bytes());
    let mut int32 = tag_type(3, "int32", 4, 0, 4, vec![]);
    int32.signed = true;
    let tagfile = synthetic_model(
        vec![
            root_sentinel(),
            tag_type(1, "TargetClass", 7, 0, 4, vec![field("scratch", 3)]),
            tag_type(2, "TargetClass*", 6, 1, 4, vec![]),
            int32,
            tag_type(4, "ParentClass", 7, 0, 4, vec![field("child", 2)]),
        ],
        vec![
            item(0, 0, 0, 0),
            item(1, 1, 4, 1),
            item(1, 1, 8, 1),
            item(3, 1, 0, 2),
            item(1, 4, 0, 1),
        ],
        data,
    );
    assert_eq!(tagfile.materialize_hkx().unwrap().objects().len(), 3);
    assert_eq!(
        member_value(&tagfile, Some("ParentClass"), "child"),
        HkxValue::Pointer(Some(1))
    );
}

#[test]
fn rel_and_inline_fixed_arrays_materialize_in_full() {
    // hkRelArray uses a bare 4-byte header (the VARN item index); reading it
    // as empty collapses hknpConvexPolytopeShape hulls to an AABB fallback.
    // hkVector4f elements appear as kind=8 in FO76 type tables.
    let floats = [1.5f32, -2.0, 3.25, 4.0];
    let vectors = [
        HkxValue::F32List(vec![1.0, 2.0, 3.0, 4.0]),
        HkxValue::F32List(vec![-5.0, 6.5, -7.25, 8.0]),
    ];
    for (element, count, expected) in [
        (
            tag_type(1, "hkReal", 5, 0, 4, vec![]),
            4,
            floats.map(HkxValue::F32).to_vec(),
        ),
        (
            tag_type(1, "hkVector4f", 8, 0, 16, vec![]),
            2,
            vectors.to_vec(),
        ),
    ] {
        let payload: Vec<f32> = expected
            .iter()
            .flat_map(|value| match value {
                HkxValue::F32(f) => vec![*f],
                HkxValue::F32List(list) => list.clone(),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        let mut data = 2u32.to_le_bytes().to_vec();
        data.extend(payload.iter().flat_map(|f| f.to_le_bytes()));
        let element_name = element.name.clone();
        let tagfile = synthetic_model(
            vec![
                root_sentinel(),
                element,
                tag_type(2, "hkRelArray", 8, 1, 4, vec![]),
                tag_type(3, "TestPolytope", 7, 0, 4, vec![field("vertices", 2)]),
            ],
            vec![item(0, 0, 0, 0), item(1, 3, 0, 1), item(2, 1, 4, count)],
            data,
        );
        assert_eq!(
            member_value(&tagfile, None, "vertices"),
            HkxValue::Array(expected),
            "{element_name}"
        );
    }

    // T[N] stores N elements inline at the field offset with no header
    // (hkbGeneratorPartitionInfo.boneMask is hkUint32[8]).
    let mut data = Vec::new();
    for value in [7u32, 9] {
        data.extend_from_slice(&value.to_le_bytes());
    }
    let tagfile = synthetic_model(
        vec![
            root_sentinel(),
            tag_type(1, "hkUint32", 4, 0, 4, vec![]),
            tag_type(2, "T[N]", 8, 1, 8, vec![]),
            tag_type(3, "PartitionInfo", 7, 0, 8, vec![field("boneMask", 2)]),
        ],
        vec![item(0, 0, 0, 0), item(1, 3, 0, 1)],
        data,
    );
    assert_eq!(
        member_value(&tagfile, None, "boneMask"),
        HkxValue::Array(vec![HkxValue::U32(7), HkxValue::U32(9)])
    );
}
