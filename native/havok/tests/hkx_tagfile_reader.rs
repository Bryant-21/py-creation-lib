use std::collections::HashSet;
use std::path::PathBuf;

use havok_native::api;
use havok_native::hkx::HkxFile;
use havok_native::hkx::tagfile::{parse_tagfile, read_vle, read_vle_signed};

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn fixture_bytes(relative: &str) -> Vec<u8> {
    std::fs::read(repo_path(relative)).unwrap_or_else(|error| {
        panic!("failed to read fixture {relative}: {error}");
    })
}

fn fo76_tag0_fixture() -> Vec<u8> {
    fixture_bytes(
        "python/creation_lib/hkxpack/tests/fixtures/fo76_snallygastercharacter.hkx",
    )
}

fn hff_leaf(tag: &[u8; 4], content: &[u8]) -> Vec<u8> {
    let size = (8 + content.len()) as u32;
    let mut data = Vec::new();
    data.extend_from_slice(&(0x4000_0000 | size).to_be_bytes());
    data.extend_from_slice(tag);
    data.extend_from_slice(content);
    data
}

fn hff_branch(tag: &[u8; 4], children: &[u8]) -> Vec<u8> {
    let size = (8 + children.len()) as u32;
    let mut data = Vec::new();
    data.extend_from_slice(&size.to_be_bytes());
    data.extend_from_slice(tag);
    data.extend_from_slice(children);
    data
}

fn synthetic_tagfile(children: Vec<Vec<u8>>) -> Vec<u8> {
    let children = children.concat();
    hff_branch(b"TAG0", &children)
}

fn vle_21(value: u32) -> [u8; 3] {
    [
        0xC0 | (((value >> 16) as u8) & 0x1F),
        (value >> 8) as u8,
        value as u8,
    ]
}

#[test]
fn detects_real_tag0_magic_at_bytes_4_to_8() {
    let data = fo76_tag0_fixture();

    assert_ne!(&data[0..4], b"TAG0");
    assert_eq!(&data[4..8], b"TAG0");
    assert_eq!(api::hkx_detect_format(&data).unwrap(), "tagfile");
}

#[test]
fn hkx_file_read_materializes_tag0_fixture() {
    let data = fo76_tag0_fixture();

    let hkx = HkxFile::read(&data).expect("HkxFile::read should accept TAG0 tagfiles");

    assert_eq!(hkx.contents_version(), "hk_2015.1.0-r1");
    assert!(!hkx.objects().is_empty());
}

#[test]
fn rejects_packfile_first_four_byte_false_positive() {
    let mut data = Vec::from(*b"\x57\xE0\xE0\x57");
    data.extend_from_slice(b"nope");

    let error = api::hkx_detect_format(&data).unwrap_err();

    assert!(
        error.to_string().contains("unsupported Havok format"),
        "unexpected error: {error}"
    );
}

#[test]
fn decodes_unsigned_and_signed_vle_basics() {
    assert_eq!(read_vle(&[0x00], 0).unwrap(), (0, 1));
    assert_eq!(read_vle(&[0x7F], 0).unwrap(), (0x7F, 1));
    assert_eq!(read_vle(&[0x80, 0x00], 0).unwrap(), (0, 2));
    assert_eq!(read_vle(&[0xBF, 0xFF], 0).unwrap(), (0x3FFF, 2));
    assert_eq!(read_vle(&[0xDF, 0xFF, 0xFF], 0).unwrap(), (0x1F_FFFF, 3));

    assert_eq!(read_vle_signed(&[0x00], 0).unwrap(), (0, 1));
    assert_eq!(read_vle_signed(&[0x01], 0).unwrap(), (-1, 1));
    assert_eq!(read_vle_signed(&[0x02], 0).unwrap(), (1, 1));
}

#[test]
fn decodes_wide_vle_widths_and_rejects_invalid_or_truncated_inputs() {
    assert_eq!(read_vle(&[0xE0, 0x00, 0x00, 0x00], 0).unwrap(), (0, 4));
    assert_eq!(
        read_vle(&[0xE8, 0x00, 0x00, 0x00, 0x00], 0).unwrap(),
        (0, 5)
    );
    assert_eq!(
        read_vle(&[0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], 0).unwrap(),
        (0, 8)
    );
    assert_eq!(
        read_vle(&[0xF8, 0x00, 0x00, 0x00, 0x00, 0x00], 0).unwrap(),
        (0, 6)
    );
    assert_eq!(
        read_vle(&[0xF9, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2A], 0).unwrap(),
        (42, 9)
    );

    for prefix in 0xFA..=0xFF {
        let error = read_vle(&[prefix, 0, 0, 0, 0, 0, 0, 0, 0], 0).unwrap_err();
        assert!(
            error.to_string().contains("invalid VLE prefix"),
            "unexpected error for {prefix:#04X}: {error}"
        );
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
        assert!(
            error.to_string().contains("needs"),
            "unexpected truncation error: {error}"
        );
    }
}

#[test]
fn rejects_trailing_non_padding_bytes_after_hff_sections() {
    let mut data = synthetic_tagfile(vec![hff_leaf(b"SDKV", b"20150100")]);
    data.extend_from_slice(b"junk");

    let error = parse_tagfile(&data).unwrap_err();

    assert!(
        error.to_string().contains("trailing HFF bytes"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_absurd_type_counts_and_type_ids_before_allocation() {
    let too_many = vle_21(100_001);
    let error = parse_tagfile(&synthetic_tagfile(vec![hff_leaf(b"TNAM", &too_many)])).unwrap_err();
    assert!(
        error.to_string().contains("type count"),
        "unexpected type count error: {error}"
    );

    let huge_type_id = [vle_21(100_001).as_slice(), &[0x00, 0x00]].concat();
    let error =
        parse_tagfile(&synthetic_tagfile(vec![hff_leaf(b"TBOD", &huge_type_id)])).unwrap_err();
    assert!(
        error.to_string().contains("type id"),
        "unexpected type id error: {error}"
    );
}

#[test]
fn rejects_truncated_item_and_ptch_sections() {
    let error = parse_tagfile(&synthetic_tagfile(vec![hff_leaf(b"ITEM", &[0; 11])])).unwrap_err();
    assert!(
        error.to_string().contains("ITEM section length"),
        "unexpected ITEM error: {error}"
    );

    let error = parse_tagfile(&synthetic_tagfile(vec![hff_leaf(b"PTCH", &[0; 3])])).unwrap_err();
    assert!(
        error.to_string().contains("PTCH section length"),
        "unexpected PTCH length error: {error}"
    );

    let mut ptch = Vec::new();
    ptch.extend_from_slice(&1u32.to_le_bytes());
    ptch.extend_from_slice(&2u32.to_le_bytes());
    ptch.extend_from_slice(&8u32.to_le_bytes());
    let error = parse_tagfile(&synthetic_tagfile(vec![hff_leaf(b"PTCH", &ptch)])).unwrap_err();
    assert!(
        error.to_string().contains("incomplete PTCH group"),
        "unexpected PTCH group error: {error}"
    );
}

#[test]
fn parses_hff_sections_and_string_tables_from_fixture() {
    let data = fo76_tag0_fixture();
    let tagfile = parse_tagfile(&data).unwrap();

    assert_eq!(tagfile.sdk_version, "20150100");
    assert_eq!(tagfile.contents_version, "hk_2015.1.0-r1");
    assert!(tagfile.section("TAG0").is_some());
    assert!(tagfile.section("SDKV").is_some());
    assert!(tagfile.section("DATA").is_some());
    assert!(tagfile.section("TSTR").is_some());
    assert!(tagfile.section("FSTR").is_some());
    assert!(tagfile.section("TNAM").is_some() || tagfile.section("TNA1").is_some());
    assert!(tagfile.section("TBOD").is_some() || tagfile.section("TBDY").is_some());
    assert!(tagfile.section("ITEM").is_some());

    assert!(
        tagfile
            .type_strings
            .iter()
            .any(|name| name == "hkRootLevelContainer")
    );
    assert!(
        tagfile
            .type_strings
            .iter()
            .any(|name| name == "hkbCharacterData")
    );
    assert!(tagfile.field_strings.iter().any(|name| name == "name"));
    assert!(
        tagfile
            .field_strings
            .iter()
            .any(|name| name == "characterControllerSetup")
    );
}

#[test]
fn parses_type_registry_metadata_from_fixture() {
    let data = fo76_tag0_fixture();
    let tagfile = parse_tagfile(&data).unwrap();

    assert!(tagfile.type_registry.types.len() >= 100);
    let names: HashSet<&str> = tagfile
        .type_registry
        .types
        .iter()
        .map(|ty| ty.name.as_str())
        .collect();
    assert!(names.contains("hkRootLevelContainer"));
    assert!(names.contains("hkbCharacterData"));
    assert!(names.contains("hkbCharacterControllerSetup"));

    let record_with_fields = tagfile
        .type_registry
        .types
        .iter()
        .find(|ty| ty.name == "hkbCharacterData")
        .expect("fixture should declare hkbCharacterData");
    assert!(record_with_fields.size > 0);
    assert!(!record_with_fields.fields.is_empty());
}

#[test]
fn parses_fo76_hkvector4_alias_metadata_from_fixture() {
    let data = fo76_tag0_fixture();
    let tagfile = parse_tagfile(&data).unwrap();

    let alias = &tagfile.type_registry.types[18];
    assert_eq!(alias.id, 18);
    assert_eq!(alias.name, "hkVector4");
    assert_eq!(alias.kind, 0);
    assert_eq!(alias.size, 0);
    assert_eq!(alias.parent_id, 30);

    let parent = &tagfile.type_registry.types[30];
    assert_eq!(parent.id, 30);
    assert_eq!(parent.name, "hkVector4f");
    assert_eq!(parent.kind, 8);
    assert_eq!(parent.size, 16);
}

#[test]
fn parses_item_table_and_optional_patch_offsets_from_fixture() {
    let data = fo76_tag0_fixture();
    let tagfile = parse_tagfile(&data).unwrap();

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
fn api_roundtrip_preserves_unchanged_tag0_fixture_bytes() {
    let data = fo76_tag0_fixture();

    let roundtripped = api::hkx_roundtrip_bytes(&data).unwrap();

    assert_eq!(roundtripped, data);
}

#[test]
fn materializes_fo76_weapon_behavior_with_t_n_inline_arrays() {
    // hkbBehaviorGraph.partitionInfo has type hkbGeneratorPartitionInfo whose
    // boneMask member is hkUint32[8] (32 bytes inline, no header).  TAG0
    // encodes that as the generic type "T[N]"; the reader must materialize
    // the 8 inline u32 elements directly from the field offset rather than
    // expecting a 16-byte hkArray header.  The base FO76 weaponbehavior file
    // is the canonical case from the FO76→FO4 parity test.
    let env_dir = match std::env::var("FO76_EXTRACTED_DIR") {
        Ok(value) if !value.is_empty() => value,
        _ => {
            eprintln!("FO76_EXTRACTED_DIR unset; skipping weapon behavior T[N] check");
            return;
        }
    };
    let path = PathBuf::from(env_dir).join("meshes/actors/character/behaviors/weaponbehavior.hkx");
    if !path.exists() {
        eprintln!(
            "weaponbehavior.hkx not present at {}; skipping",
            path.display()
        );
        return;
    }
    let data = std::fs::read(&path).expect("read weaponbehavior.hkx");
    let tagfile = parse_tagfile(&data).expect("parse_tagfile");

    // Confirm the registry has a T[N] entry referenced by hkbGeneratorPartitionInfo.
    let has_tn = tagfile
        .type_registry
        .types
        .iter()
        .any(|ty| ty.name == "T[N]");
    assert!(has_tn, "expected at least one T[N] type in FO76 fixture");

    // We do not assert full materialize_hkx success here because other
    // unrelated TAG0 gaps may still exist for this fixture. What we DO assert
    // is that whatever error arrives is not the T[N] header-size error.
    if let Err(error) = tagfile.materialize_hkx() {
        let message = error.to_string();
        assert!(
            !message.contains("hkArray type T[N] has unsupported header size"),
            "regression: T[N] inline-array path still rejected: {message}"
        );
    }
}

// Helper that builds a minimal synthetic tagfile with a single named complex
// type field (kind=0, explicit size) and returns the materialized float list.
fn materialize_named_complex_field(type_name: &str, byte_size: usize) -> Vec<f32> {
    use havok_native::hkx::tagfile::{
        TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection,
    };
    use havok_native::hkx::types::HkxValue;

    let data = vec![0u8; byte_size];
    let tagfile = Tagfile::from_synthetic_parts(
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
        TagTypeRegistry {
            types: vec![
                TagType {
                    id: 0,
                    name: "hkRootLevelContainer".into(),
                    parent_id: 0,
                    kind: 0,
                    subtype_id: 0,
                    size: 0,
                    align: 0,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 1,
                    name: type_name.into(),
                    parent_id: 0,
                    kind: 0,
                    subtype_id: 0,
                    size: byte_size,
                    align: 16,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 2,
                    name: "TestObject".into(),
                    parent_id: 0,
                    kind: 6,
                    subtype_id: 0,
                    size: byte_size,
                    align: 16,
                    version: 1,
                    format_value: 0,
                    signed: false,
                    fields: vec![TagField {
                        name: "value".into(),
                        type_id: 1,
                        offset: 0,
                        flags: 0,
                    }],
                },
            ],
        },
        vec![
            TagfileItem {
                kind: 0,
                flags: 0,
                type_id: 0,
                offset: 0,
                count: 0,
            },
            TagfileItem {
                kind: 1,
                flags: 0,
                type_id: 2,
                offset: 0,
                count: 1,
            },
        ],
        data,
    );

    let hkx = tagfile
        .materialize_hkx()
        .unwrap_or_else(|e| panic!("materialize failed for {type_name}: {e}"));
    let obj = hkx.objects().first().expect("at least one object");
    let member = obj
        .members
        .iter()
        .find(|m| m.name == "value")
        .expect("value member");
    match &member.value {
        HkxValue::F32List(floats) => floats.clone(),
        other => panic!("expected F32List for {type_name}, got {other:?}"),
    }
}

#[test]
fn named_complex_hkx_type_hkquaternion_16_bytes() {
    let floats = materialize_named_complex_field("hkQuaternion", 16);
    assert_eq!(floats.len(), 4);
}

#[test]
fn named_complex_hkx_type_hkqstransform_48_bytes() {
    let floats = materialize_named_complex_field("hkQsTransform", 48);
    assert_eq!(floats.len(), 12);
}

#[test]
fn named_complex_hkx_type_hkmatrix3_48_bytes() {
    let floats = materialize_named_complex_field("hkMatrix3", 48);
    assert_eq!(floats.len(), 12);
}

#[test]
fn named_complex_hkx_type_hkmatrix4_64_bytes() {
    let floats = materialize_named_complex_field("hkMatrix4", 64);
    assert_eq!(floats.len(), 16);
}

#[test]
fn named_complex_hkx_type_hktransform_64_bytes() {
    let floats = materialize_named_complex_field("hkTransform", 64);
    assert_eq!(floats.len(), 16);
}

// Build a minimal synthetic TAG0 in-memory model with a single hkStringPtr-like
// field at the start of the object and a VARN char payload at a configurable
// offset.  The payload bytes are written into DATA verbatim, so the test can
// inject non-UTF-8 sequences.  The field offset is registered in PTCH so the
// reader takes the item-indexed string path.  Returns the materialized string.
fn materialize_string_field_with_payload(payload: &[u8]) -> String {
    use havok_native::hkx::tagfile::{
        TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection,
    };
    use havok_native::hkx::types::HkxValue;

    let object_size = 4usize;
    let payload_offset = object_size;
    let payload_count = payload.len() + 1; // null terminator slot
    let mut data = vec![0u8; object_size + payload_count];
    let item_index = 2u32;
    data[0..4].copy_from_slice(&item_index.to_le_bytes());
    data[payload_offset..payload_offset + payload.len()].copy_from_slice(payload);

    let mut tagfile = Tagfile::from_synthetic_parts(
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
        TagTypeRegistry {
            types: vec![
                TagType {
                    id: 0,
                    name: "hkRootLevelContainer".into(),
                    parent_id: 0,
                    kind: 0,
                    subtype_id: 0,
                    size: 0,
                    align: 0,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 1,
                    name: "hkStringPtr".into(),
                    parent_id: 0,
                    kind: 3,
                    subtype_id: 0,
                    size: 4,
                    align: 4,
                    version: 0,
                    format_value: 0,
                    signed: false,
                    fields: vec![],
                },
                TagType {
                    id: 2,
                    name: "TestObject".into(),
                    parent_id: 0,
                    kind: 7,
                    subtype_id: 0,
                    size: object_size,
                    align: 4,
                    version: 1,
                    format_value: 0,
                    signed: false,
                    fields: vec![TagField {
                        name: "name".into(),
                        type_id: 1,
                        offset: 0,
                        flags: 0,
                    }],
                },
            ],
        },
        vec![
            // index 0: sentinel
            TagfileItem {
                kind: 0,
                flags: 0,
                type_id: 0,
                offset: 0,
                count: 0,
            },
            // index 1: VAR0 of the test object (gets materialized)
            TagfileItem {
                kind: 1,
                flags: 0,
                type_id: 2,
                offset: 0,
                count: 1,
            },
            // index 2: VARN char payload that the field's u32 indexes into
            TagfileItem {
                kind: 2,
                flags: 0,
                type_id: 1,
                offset: payload_offset,
                count: payload_count,
            },
        ],
        data,
    );
    // Field at object_offset(0) + field_offset(0) = 0 — register it in PTCH so
    // the reader takes the item-indexed string path rather than scratch-int.
    tagfile.set_pointer_offsets_for_test(vec![0]);

    let hkx = tagfile
        .materialize_hkx()
        .expect("materialize tolerant string fixture");
    let obj = hkx.objects().first().expect("at least one object");
    let member = obj
        .members
        .iter()
        .find(|m| m.name == "name")
        .expect("name member");
    match &member.value {
        HkxValue::String { value, .. } => value.clone(),
        other => panic!("expected String, got {other:?}"),
    }
}

#[test]
fn string_field_with_non_utf8_bytes_decodes_tolerantly() {
    // FO76 weapon behavior fixtures hit hkStringPtr fields whose VARN payload
    // contains non-UTF-8 bytes (e.g. 0xE9 in latin-1).  Python decodes these
    // tolerantly via `.decode("ascii", errors="replace")` at
    // py_creation_lib/python/creation_lib/hkxpack/tagfile_reader.py:1212; the Rust port must not reject them
    // with "TAG0 string is not UTF-8".
    let payload = b"name_\xE9_end";
    let value = materialize_string_field_with_payload(payload);
    // 0xE9 is replaced with U+FFFD (matches Python's `errors="replace"`).
    assert_eq!(value, "name_\u{FFFD}_end");
}

#[test]
fn string_field_strips_xml_invalid_control_bytes() {
    // Match Python's post-decode sanitization at
    // py_creation_lib/python/creation_lib/hkxpack/tagfile_reader.py:1215 — drop C0 control bytes other than
    // \n, \r, \t.  Required because hkStringPtr is reused as a scratch
    // 4/8-byte field whose contents may not actually be a string.
    let payload = b"a\x01b\tc\x07d\ne";
    let value = materialize_string_field_with_payload(payload);
    assert_eq!(value, "ab\tcd\ne");
}

#[test]
fn string_field_pure_ascii_passes_through_unchanged() {
    // Sanity check: the tolerant decode path must not corrupt valid ASCII.
    let payload = b"hkbStateMachine";
    let value = materialize_string_field_with_payload(payload);
    assert_eq!(value, "hkbStateMachine");
}

// Build a synthetic TAG0 in-memory model with two VAR0 target objects and a
// NOTE item that aliases the second target. A third VAR0 object holds a
// pointer field whose stored item index is the NOTE; the reader must follow
// the NOTE's `count` to land on the real VAR0 object instead of erroring.
//
// SDK reference: hkTagfileReadFormat.cpp:540, KIND_NOTE indirection where
// `count` is the index of the actual object item.
#[test]
fn note_kind_pointer_follows_indirection() {
    use havok_native::hkx::tagfile::{
        TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection,
    };
    use havok_native::hkx::types::HkxValue;

    // Layout in DATA:
    //   [0..4]     parent's pointer field (u32 = item index of the NOTE = 3)
    //   [4..8]     target A body (4-byte placeholder)
    //   [8..12]    target B body (4-byte placeholder)
    let parent_offset = 0usize;
    let target_a_offset = 4usize;
    let target_b_offset = 8usize;
    let mut data = vec![0u8; 12];
    let note_item_index: u32 = 3;
    data[parent_offset..parent_offset + 4].copy_from_slice(&note_item_index.to_le_bytes());

    let registry = TagTypeRegistry {
        types: vec![
            // id 0: sentinel hkRootLevelContainer
            TagType {
                id: 0,
                name: "hkRootLevelContainer".into(),
                parent_id: 0,
                kind: 0,
                subtype_id: 0,
                size: 0,
                align: 0,
                version: 0,
                format_value: 0,
                signed: false,
                fields: vec![],
            },
            // id 1: target class (real pointee) — small record with one int
            // member so collect_fields() produces a non-empty vec, which is
            // what materialized_object_item_indices() requires for VAR0 items
            // to be promoted to objects.
            TagType {
                id: 1,
                name: "TargetClass".into(),
                parent_id: 0,
                kind: 7,
                subtype_id: 0,
                size: 4,
                align: 4,
                version: 1,
                format_value: 0,
                signed: false,
                fields: vec![TagField {
                    name: "scratch".into(),
                    type_id: 3,
                    offset: 0,
                    flags: 0,
                }],
            },
            // id 2: pointer-to-TargetClass (kind=6, size 4)
            TagType {
                id: 2,
                name: "TargetClass*".into(),
                parent_id: 0,
                kind: 6,
                subtype_id: 1,
                size: 4,
                align: 4,
                version: 0,
                format_value: 0,
                signed: false,
                fields: vec![],
            },
            // id 3: int32 used as TargetClass.scratch
            TagType {
                id: 3,
                name: "int32".into(),
                parent_id: 0,
                kind: 4,
                subtype_id: 0,
                size: 4,
                align: 4,
                version: 0,
                format_value: 0,
                signed: true,
                fields: vec![],
            },
            // id 4: parent class with a single pointer field at offset 0
            TagType {
                id: 4,
                name: "ParentClass".into(),
                parent_id: 0,
                kind: 7,
                subtype_id: 0,
                size: 4,
                align: 4,
                version: 1,
                format_value: 0,
                signed: false,
                fields: vec![TagField {
                    name: "child".into(),
                    type_id: 2,
                    offset: 0,
                    flags: 0,
                }],
            },
        ],
    };

    let items = vec![
        // index 0: sentinel
        TagfileItem {
            kind: 0,
            flags: 0,
            type_id: 0,
            offset: 0,
            count: 0,
        },
        // index 1: VAR0 target A (becomes object_index 0)
        TagfileItem {
            kind: 1,
            flags: 0,
            type_id: 1,
            offset: target_a_offset,
            count: 1,
        },
        // index 2: VAR0 target B (becomes object_index 1) — NOTE will alias
        TagfileItem {
            kind: 1,
            flags: 0,
            type_id: 1,
            offset: target_b_offset,
            count: 1,
        },
        // index 3: NOTE indirection → count is the index of the real item (2)
        TagfileItem {
            kind: 3,
            flags: 0,
            type_id: 1,
            offset: 0,
            count: 2,
        },
        // index 4: VAR0 parent (becomes object_index 2) — pointer field
        // stored in DATA at offset 0 is item_index 3 (NOTE).
        TagfileItem {
            kind: 1,
            flags: 0,
            type_id: 4,
            offset: parent_offset,
            count: 1,
        },
    ];

    let tagfile = Tagfile::from_synthetic_parts(
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
        registry,
        items,
        data,
    );

    let hkx = tagfile
        .materialize_hkx()
        .expect("NOTE-indirected pointer must resolve to the aliased real item");

    // Three VAR0 objects: target A (index 0), target B (index 1), parent
    // (index 2). The parent's `child` pointer must end up referencing
    // object_index 1 (target B) — not erroring, not None, not pointing at A.
    assert_eq!(hkx.objects().len(), 3);
    let parent = hkx
        .objects()
        .iter()
        .find(|o| o.class_name == "ParentClass")
        .expect("parent object materialized");
    let child = parent
        .members
        .iter()
        .find(|m| m.name == "child")
        .expect("child member present");
    match &child.value {
        HkxValue::Pointer(Some(target_index)) => {
            assert_eq!(
                *target_index, 1,
                "NOTE indirection should resolve to the aliased VAR0 object_index 1, got {target_index}"
            );
        }
        other => panic!("expected resolved Pointer(Some(1)), got {other:?}"),
    }
}

#[test]
fn hkrelarray_field_materializes_from_varn_item() {
    // hknpConvexPolytopeShape stores its hull geometry (vertices/planes/faces/
    // indices) as hkRelArray fields. Unlike hkArray's 16-byte synthetic header,
    // hkRelArray uses a bare 4-byte header that is just the VARN item index.
    // Treating a non-16-byte array header as empty would silently drop all hull
    // geometry and collapse the shape to an AABB-box fallback. Fixture: a parent
    // object with a single hkRelArray<hkReal> field whose 4-byte header indexes
    // a VARN payload of 4 floats.
    use havok_native::hkx::tagfile::{
        TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection,
    };
    use havok_native::hkx::types::HkxValue;

    // DATA: [0..4] parent body = hkRelArray header (u32 item index = 2)
    //       [4..20] VARN payload = 4 little-endian f32 values
    let payload_offset = 4usize;
    let floats = [1.5f32, -2.0, 3.25, 4.0];
    let mut data = vec![0u8; payload_offset + floats.len() * 4];
    let varn_item_index: u32 = 2;
    data[0..4].copy_from_slice(&varn_item_index.to_le_bytes());
    for (i, f) in floats.iter().enumerate() {
        let o = payload_offset + i * 4;
        data[o..o + 4].copy_from_slice(&f.to_le_bytes());
    }

    let registry = TagTypeRegistry {
        types: vec![
            // id 0: sentinel
            TagType {
                id: 0,
                name: "hkRootLevelContainer".into(),
                parent_id: 0,
                kind: 0,
                subtype_id: 0,
                size: 0,
                align: 0,
                version: 0,
                format_value: 0,
                signed: false,
                fields: vec![],
            },
            // id 1: hkReal element subtype (kind 5, size 4 -> HkxType::Real)
            TagType {
                id: 1,
                name: "hkReal".into(),
                parent_id: 0,
                kind: 5,
                subtype_id: 0,
                size: 4,
                align: 4,
                version: 0,
                format_value: 0,
                signed: false,
                fields: vec![],
            },
            // id 2: hkRelArray<hkReal> (kind 8, 4-byte header -> not 16)
            TagType {
                id: 2,
                name: "hkRelArray".into(),
                parent_id: 0,
                kind: 8,
                subtype_id: 1,
                size: 4,
                align: 4,
                version: 0,
                format_value: 0,
                signed: false,
                fields: vec![],
            },
            // id 3: parent object holding the relarray field at offset 0
            TagType {
                id: 3,
                name: "TestPolytope".into(),
                parent_id: 0,
                kind: 7,
                subtype_id: 0,
                size: 4,
                align: 4,
                version: 1,
                format_value: 0,
                signed: false,
                fields: vec![TagField {
                    name: "vertices".into(),
                    type_id: 2,
                    offset: 0,
                    flags: 0,
                }],
            },
        ],
    };

    let items = vec![
        // index 0: sentinel
        TagfileItem {
            kind: 0,
            flags: 0,
            type_id: 0,
            offset: 0,
            count: 0,
        },
        // index 1: VAR0 parent object (becomes object 0)
        TagfileItem {
            kind: 1,
            flags: 0,
            type_id: 3,
            offset: 0,
            count: 1,
        },
        // index 2: VARN float payload that the relarray header indexes into
        TagfileItem {
            kind: 2,
            flags: 0,
            type_id: 1,
            offset: payload_offset,
            count: floats.len(),
        },
    ];

    let tagfile = Tagfile::from_synthetic_parts(
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
        registry,
        items,
        data,
    );

    let hkx = tagfile
        .materialize_hkx()
        .expect("materialize hkRelArray fixture");
    let obj = hkx.objects().first().expect("parent object");
    let member = obj
        .members
        .iter()
        .find(|m| m.name == "vertices")
        .expect("vertices member");
    match &member.value {
        HkxValue::Array(values) => {
            let got: Vec<f32> = values
                .iter()
                .map(|v| match v {
                    HkxValue::F32(f) => *f,
                    other => panic!("expected F32 element, got {other:?}"),
                })
                .collect();
            assert_eq!(got, floats, "hkRelArray payload should materialize in full");
        }
        other => panic!("expected populated Array for hkRelArray, got {other:?}"),
    }
}

#[test]
fn hkrelarray_vector4f_element_kind8_materializes() {
    // hknpConvexPolytopeShape declares its `vertices` hkRelArray element subtype
    // against hkVector4f directly — and in the FO76 type table hkVector4f is a
    // kind=8 (concrete float-vector layout) type, not the kind=0 alias used by
    // `planes`. The element-sizing/materialization path resolves vectors via
    // named_complex_hkx_type, which must accept kind=8 vectors; otherwise
    // `vertices` stays empty (every other hull array populated) and the shape
    // collapses to an AABB-box fallback. Fixture: an
    // hkRelArray<hkVector4f(kind=8)> of two vectors.
    use havok_native::hkx::tagfile::{
        TagField, TagType, TagTypeRegistry, Tagfile, TagfileItem, TagfileSection,
    };
    use havok_native::hkx::types::HkxValue;

    // DATA: [0..4] parent body = hkRelArray header (u32 item index = 2)
    //       [4..36] VARN payload = 2 hkVector4f (8 little-endian f32 values)
    let payload_offset = 4usize;
    let vectors = [[1.0f32, 2.0, 3.0, 4.0], [-5.0f32, 6.5, -7.25, 8.0]];
    let flat: Vec<f32> = vectors.iter().flatten().copied().collect();
    let mut data = vec![0u8; payload_offset + flat.len() * 4];
    let varn_item_index: u32 = 2;
    data[0..4].copy_from_slice(&varn_item_index.to_le_bytes());
    for (i, f) in flat.iter().enumerate() {
        let o = payload_offset + i * 4;
        data[o..o + 4].copy_from_slice(&f.to_le_bytes());
    }

    let registry = TagTypeRegistry {
        types: vec![
            TagType {
                id: 0,
                name: "hkRootLevelContainer".into(),
                parent_id: 0,
                kind: 0,
                subtype_id: 0,
                size: 0,
                align: 0,
                version: 0,
                format_value: 0,
                signed: false,
                fields: vec![],
            },
            // id 1: hkVector4f as the *concrete* float-vector type (kind 8).
            TagType {
                id: 1,
                name: "hkVector4f".into(),
                parent_id: 0,
                kind: 8,
                subtype_id: 0,
                size: 16,
                align: 16,
                version: 0,
                format_value: 0,
                signed: false,
                fields: vec![],
            },
            // id 2: hkRelArray<hkVector4f> (kind 8, 4-byte header)
            TagType {
                id: 2,
                name: "hkRelArray".into(),
                parent_id: 0,
                kind: 8,
                subtype_id: 1,
                size: 4,
                align: 4,
                version: 0,
                format_value: 0,
                signed: false,
                fields: vec![],
            },
            // id 3: parent object holding the relarray field at offset 0
            TagType {
                id: 3,
                name: "TestPolytope".into(),
                parent_id: 0,
                kind: 7,
                subtype_id: 0,
                size: 4,
                align: 4,
                version: 1,
                format_value: 0,
                signed: false,
                fields: vec![TagField {
                    name: "vertices".into(),
                    type_id: 2,
                    offset: 0,
                    flags: 0,
                }],
            },
        ],
    };

    let items = vec![
        TagfileItem {
            kind: 0,
            flags: 0,
            type_id: 0,
            offset: 0,
            count: 0,
        },
        TagfileItem {
            kind: 1,
            flags: 0,
            type_id: 3,
            offset: 0,
            count: 1,
        },
        TagfileItem {
            kind: 2,
            flags: 0,
            type_id: 1,
            offset: payload_offset,
            count: vectors.len(),
        },
    ];

    let tagfile = Tagfile::from_synthetic_parts(
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
        registry,
        items,
        data,
    );

    let hkx = tagfile
        .materialize_hkx()
        .expect("materialize hkRelArray<hkVector4f> fixture");
    let obj = hkx.objects().first().expect("parent object");
    let member = obj
        .members
        .iter()
        .find(|m| m.name == "vertices")
        .expect("vertices member");
    match &member.value {
        HkxValue::Array(values) => {
            assert_eq!(values.len(), 2, "both hull vertices should materialize");
            let got: Vec<Vec<f32>> = values
                .iter()
                .map(|v| match v {
                    HkxValue::F32List(f) => f.clone(),
                    other => panic!("expected F32List vector element, got {other:?}"),
                })
                .collect();
            assert_eq!(got[0], vectors[0]);
            assert_eq!(got[1], vectors[1]);
        }
        other => panic!("expected populated Array for hkRelArray<hkVector4f>, got {other:?}"),
    }
}

#[test]
fn materialize_then_save_runs_writer_not_source_bytes() {
    // verify that materializing a TAG0 file, mutating it, and calling save()
    // produces writer output rather than echoing the original TAG0 bytes.
    let data = fo76_tag0_fixture();
    let tagfile = parse_tagfile(&data).expect("parse FO76 fixture");
    let mut hkx = tagfile.materialize_hkx().expect("materialize FO76 fixture");

    // Mutation to mark the model dirty.
    let _ = hkx.objects_mut();
    let saved = hkx.save();

    // Saved bytes must not be the raw TAG0 source.
    assert_ne!(
        saved, data,
        "save() on a mutated TAG0-sourced HkxFile should run the packfile writer"
    );
    // The output must be a valid v11 packfile (magic bytes check).
    assert_eq!(
        &saved[0..8],
        &[0x57, 0xE0, 0xE0, 0x57, 0x10, 0xC0, 0xC0, 0x10],
        "save() output should start with packfile magic"
    );
}
