//! Tagfile2014 (binary tagfile v13) reader tests.
//!
//! Bethesda ships this format for FO4 inline animation blobs nested in NIFs
//! (BSBound / cloth setup) and Skyrim SE .hkt skeleton files. There is no
//! vendored fixture (a Skyrim install is not guaranteed in CI), so these tests
//! synthesize v13 buffers byte-by-byte from the SDK-documented stream encoding.

use havok_native::hkx::tagfile2014::{
    BINARY_MAGIC_0, BINARY_MAGIC_1, TAGFILE_VERSION_2014_2, is_binary_tagfile_magic, parse_header,
    read_tagfile2014, write_tagfile2014,
};
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, HkxObject};

fn synthesize_header_only(magic_le: bool, tag: u32, version: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(16);
    if magic_le {
        buf.extend_from_slice(&BINARY_MAGIC_0.to_le_bytes());
        buf.extend_from_slice(&BINARY_MAGIC_1.to_le_bytes());
    } else {
        buf.extend_from_slice(&BINARY_MAGIC_0.to_be_bytes());
        buf.extend_from_slice(&BINARY_MAGIC_1.to_be_bytes());
    }
    buf.extend_from_slice(&tag.to_le_bytes());
    buf.extend_from_slice(&version.to_le_bytes());
    buf
}

#[test]
fn detects_binary_tagfile_magic_in_either_endianness() {
    let le = synthesize_header_only(true, 1, 13);
    assert!(is_binary_tagfile_magic(&le));

    let be = synthesize_header_only(false, 1, 13);
    assert!(is_binary_tagfile_magic(&be));

    let mut bogus = vec![0u8; 16];
    bogus[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
    assert!(!is_binary_tagfile_magic(&bogus));
}

#[test]
fn rejects_buffer_too_small_for_magic() {
    assert!(!is_binary_tagfile_magic(&[]));
    assert!(!is_binary_tagfile_magic(&[0x1E]));
    assert!(!is_binary_tagfile_magic(&[
        0x1E, 0x0D, 0xB0, 0xCA, 0xCE, 0xFA, 0x11
    ]));
}

// VLE-encoded signed int helper, mirrors the stream codec in tagfile2014.rs.
// Used only by the synthetic-stream integration tests below.
fn vle_signed(value: i64) -> Vec<u8> {
    let neg = value < 0;
    let mag = if neg { (-value) as u64 } else { value as u64 };
    let mut out = Vec::new();
    let first_mag = (mag & 0x3F) as u8;
    let mut remaining = mag >> 6;
    let cont = remaining != 0;
    let mut first = (first_mag << 1) | u8::from(neg);
    if cont {
        first |= 0x80;
    }
    out.push(first);
    while remaining != 0 {
        let chunk = (remaining & 0x7F) as u8;
        remaining >>= 7;
        let cont = remaining != 0;
        out.push(if cont { chunk | 0x80 } else { chunk });
    }
    out
}

fn vle_string(s: &str) -> Vec<u8> {
    let mut out = vle_signed(s.len() as i64);
    out.extend_from_slice(s.as_bytes());
    out
}

const SYNTH_TAG_FILE_INFO: i64 = 1;
const SYNTH_TAG_METADATA: i64 = 2;
const SYNTH_TAG_FILE_END: i64 = 7;

#[test]
fn read_with_only_header_errors_on_missing_tag_file_info() {
    // No content past the 16-byte header: the reader should fail trying to
    // read the first tag, not silently succeed.
    let header = synthesize_header_only(true, 1, 13);
    let err = read_tagfile2014(&header).unwrap_err();
    assert!(
        err.to_string().contains("truncated") || err.to_string().contains("VLE"),
        "expected truncation/VLE error past header, got: {err}"
    );
}

#[test]
fn read_with_metadata_before_file_info_is_rejected() {
    let mut buf = synthesize_header_only(true, 1, 13);
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_METADATA));
    let err = read_tagfile2014(&buf).unwrap_err();
    assert!(
        err.to_string().contains("starts with tag 2")
            && err.to_string().contains("expected TAG_FILE_INFO"),
        "unexpected error: {err}"
    );
}

#[test]
fn read_v3_file_info_only_then_end_yields_empty_hkx_file() {
    // Minimal valid stream: TAG_FILE_INFO version=3 (no sdk version,
    // no predicates, single precision), then TAG_FILE_END. This
    // materializes successfully with zero objects.
    let mut buf = synthesize_header_only(true, 1, 13);
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_INFO));
    buf.extend_from_slice(&vle_signed(3));
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_END));
    let hkx = read_tagfile2014(&buf).expect("v13 file-info-only stream materializes");
    assert!(
        hkx.objects().is_empty(),
        "expected zero objects, got {}",
        hkx.objects().len()
    );
}

#[test]
fn write_minimal_v13_object_graph_round_trips_through_reader() {
    let hkx = HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![
            HkxObject {
                name: Some("#0001".to_string()),
                offset: 0,
                signature: 0,
                class_name: "Container".to_string(),
                members: vec![
                    HkxMember {
                        name: "name".to_string(),
                        value: HkxValue::String {
                            value: "root".to_string(),
                            is_null: false,
                        },
                    },
                    HkxMember {
                        name: "child".to_string(),
                        value: HkxValue::Pointer(Some(1)),
                    },
                ],
            },
            HkxObject {
                name: Some("#0002".to_string()),
                offset: 0,
                signature: 0,
                class_name: "Child".to_string(),
                members: vec![HkxMember {
                    name: "value".to_string(),
                    value: HkxValue::I64(42),
                }],
            },
        ],
    );

    let bytes = write_tagfile2014(&hkx).expect("minimal v13 writer should accept simple graph");
    assert!(is_binary_tagfile_magic(&bytes));

    let reparsed = read_tagfile2014(&bytes).expect("writer output should parse");
    assert_eq!(reparsed.objects(), hkx.objects());
}

#[test]
fn read_v3_with_one_metadata_class_then_end_yields_empty_hkx_file() {
    // Class table is populated by TAG_METADATA but no TAG_OBJECT* records
    // follow, so the materialized HkxFile still has zero objects.
    let mut buf = synthesize_header_only(true, 1, 13);
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_INFO));
    buf.extend_from_slice(&vle_signed(3));

    buf.extend_from_slice(&vle_signed(SYNTH_TAG_METADATA));
    buf.extend_from_slice(&vle_string("TestClass")); // record name
    buf.extend_from_slice(&vle_signed(0)); // version
    buf.extend_from_slice(&vle_signed(-1)); // parent index
    buf.extend_from_slice(&vle_signed(1)); // numFields
    buf.extend_from_slice(&vle_string("x")); // field name
    buf.extend_from_slice(&vle_signed(3)); // legacyType TYPE_REAL

    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_END));
    let hkx = read_tagfile2014(&buf).expect("v13 stream with one class materializes");
    assert!(hkx.objects().is_empty());
}

#[test]
fn read_metadata_with_struct_field_consumes_class_name_string() {
    // STRUCT field-type bits require an extra string after the legacy type
    // — the class name. Verify the codec advances the stream past it
    // rather than getting out of sync.
    const LT_TYPE_STRUCT: u32 = 9;
    let mut buf = synthesize_header_only(true, 1, 13);
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_INFO));
    buf.extend_from_slice(&vle_signed(3));

    // Class A (the struct payload type, declared first so its name index
    // can be used).
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_METADATA));
    buf.extend_from_slice(&vle_string("StructPayload"));
    buf.extend_from_slice(&vle_signed(0));
    buf.extend_from_slice(&vle_signed(-1));
    buf.extend_from_slice(&vle_signed(0)); // numFields=0

    // Class B with a STRUCT-typed field whose class name = "StructPayload".
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_METADATA));
    buf.extend_from_slice(&vle_string("Container"));
    buf.extend_from_slice(&vle_signed(0));
    buf.extend_from_slice(&vle_signed(-1));
    buf.extend_from_slice(&vle_signed(1));
    buf.extend_from_slice(&vle_string("payload"));
    buf.extend_from_slice(&vle_signed(LT_TYPE_STRUCT as i64));
    // STRUCT triggers an extra class-name string; backref -2 references
    // "StructPayload" which was pushed at slot 2 of prev_strings.
    buf.extend_from_slice(&vle_signed(-2));

    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_END));
    let hkx = read_tagfile2014(&buf).expect("two-metadata stream materializes");
    assert!(hkx.objects().is_empty());
}

#[test]
fn header_parse_accepts_canonical_little_endian_v13() {
    let mut buf = synthesize_header_only(true, 1, TAGFILE_VERSION_2014_2);
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_INFO));
    let header = parse_header(&buf).expect("canonical LE header parses");
    assert!(!header.swap_bytes);
    assert_eq!(header.stream_offset, 16);
}

#[test]
fn header_parse_detects_byte_swap_from_big_endian_magic() {
    // BE-magic file: magic words are written big-endian, but the rest of
    // the file (tag, version) is also stored in the BE byte order.
    // parse_header normalizes by swapping every u32 it reads after detect.
    let mut buf = Vec::new();
    buf.extend_from_slice(&BINARY_MAGIC_0.to_be_bytes());
    buf.extend_from_slice(&BINARY_MAGIC_1.to_be_bytes());
    buf.extend_from_slice(&1u32.to_be_bytes());
    buf.extend_from_slice(&TAGFILE_VERSION_2014_2.to_be_bytes());
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_INFO));
    let header = parse_header(&buf).expect("BE header parses");
    assert!(header.swap_bytes);
    assert_eq!(header.stream_offset, 16);
}

#[test]
fn header_parse_rejects_truncated_buffer() {
    let err = parse_header(&[0u8; 7]).unwrap_err();
    assert!(
        err.to_string().contains("Tagfile2014 magic requires"),
        "unexpected error: {err}"
    );
}

#[test]
fn header_parse_rejects_wrong_magic0() {
    let mut buf = synthesize_header_only(true, 1, TAGFILE_VERSION_2014_2);
    buf[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
    let err = parse_header(&buf).unwrap_err();
    assert!(
        err.to_string().contains("magic0 mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn header_parse_rejects_wrong_magic1() {
    let mut buf = synthesize_header_only(true, 1, TAGFILE_VERSION_2014_2);
    buf[4..8].copy_from_slice(&0u32.to_le_bytes());
    let err = parse_header(&buf).unwrap_err();
    assert!(
        err.to_string().contains("magic1 mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn header_parse_rejects_non_v13_layout() {
    let buf = synthesize_header_only(true, 1, 11);
    let err = parse_header(&buf).unwrap_err();
    assert!(
        err.to_string().contains("starts with tag"),
        "unexpected error: {err}"
    );
}

#[test]
fn header_parse_rejects_non_one_tag_word() {
    let buf = synthesize_header_only(true, 7, TAGFILE_VERSION_2014_2);
    let err = parse_header(&buf).unwrap_err();
    assert!(
        err.to_string().contains("starts with tag"),
        "unexpected error: {err}"
    );
}

// hkLegacyType bits used by the synthesized hkRootLevelContainer fixture.
// Mirrors SDK hkLegacyType.h.
const LT_TYPE_OBJECT: u32 = 8;
const LT_TYPE_STRUCT: u32 = 9;
const LT_TYPE_CSTRING: u32 = 10;
const LT_TYPE_ARRAY: u32 = 0x10;

const SYNTH_TAG_OBJECT_REMEMBER: i64 = 4;

/// Build a presence bitfield from a slice of bools. Bit `i` (LSB-first
/// within byte `i/8`) is the presence flag for member `i`. Padding bits
/// must be zero per SDK hkTagfileReadFormat2014.cpp:381-384.
fn presence_bytes(flags: &[bool]) -> Vec<u8> {
    let n = flags.len();
    let num_bytes = n.div_ceil(8).max(1);
    let mut out = vec![0u8; num_bytes];
    for (i, &flag) in flags.iter().enumerate() {
        if flag {
            out[i / 8] |= 1 << (i % 8);
        }
    }
    out
}

/// Synthesizes a minimal v13 stream:
///
///   classes:
///     1. hkaSkeleton                 (one CSTRING field "name")
///     2. hkRootLevelContainerNamedVariant (CSTRING name, CSTRING className,
///                                          OBJECT variant)
///     3. hkRootLevelContainer        (one ARRAY-of-STRUCT field
///                                     "namedVariants")
///   objects:
///     #1 (REMEMBER, id=1) hkRootLevelContainer with namedVariants[0]
///        = { name="myskel", className="hkaSkeleton", variant=id 2 }
///     #2 (REMEMBER, id=2) hkaSkeleton with name="MySkeletonName"
///
/// Verifies: the array reader, struct recursion, CSTRING decode, OBJECT
/// pointer forward-ref resolution via remembered_objects, and the
/// member-presence bitfield path all execute end-to-end.
#[test]
fn sub_commit_d_root_level_container_with_skeleton_pointer_materializes() {
    let mut buf = synthesize_header_only(true, 1, 13);
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_INFO));
    buf.extend_from_slice(&vle_signed(3));

    // Class index 1: hkaSkeleton
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_METADATA));
    buf.extend_from_slice(&vle_string("hkaSkeleton"));
    buf.extend_from_slice(&vle_signed(0)); // version
    buf.extend_from_slice(&vle_signed(-1)); // parent index = none
    buf.extend_from_slice(&vle_signed(1)); // numFields
    buf.extend_from_slice(&vle_string("name"));
    buf.extend_from_slice(&vle_signed(LT_TYPE_CSTRING as i64));

    // Class index 2: hkRootLevelContainerNamedVariant
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_METADATA));
    buf.extend_from_slice(&vle_string("hkRootLevelContainerNamedVariant"));
    buf.extend_from_slice(&vle_signed(0)); // version
    buf.extend_from_slice(&vle_signed(-1));
    buf.extend_from_slice(&vle_signed(3)); // numFields
    buf.extend_from_slice(&vle_string("name"));
    buf.extend_from_slice(&vle_signed(LT_TYPE_CSTRING as i64));
    buf.extend_from_slice(&vle_string("className"));
    buf.extend_from_slice(&vle_signed(LT_TYPE_CSTRING as i64));
    buf.extend_from_slice(&vle_string("variant"));
    buf.extend_from_slice(&vle_signed(LT_TYPE_OBJECT as i64));
    buf.extend_from_slice(&vle_signed(0)); // OBJECT class name = "" (HK_NULL backref)

    // Class index 3: hkRootLevelContainer
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_METADATA));
    buf.extend_from_slice(&vle_string("hkRootLevelContainer"));
    buf.extend_from_slice(&vle_signed(0));
    buf.extend_from_slice(&vle_signed(-1));
    buf.extend_from_slice(&vle_signed(1));
    buf.extend_from_slice(&vle_string("namedVariants"));
    buf.extend_from_slice(&vle_signed((LT_TYPE_ARRAY | LT_TYPE_STRUCT) as i64));
    // ARRAY-of-STRUCT: SDK reads the element class name immediately after
    // the legacy type. Backref into prev_strings: slot for
    // "hkRootLevelContainerNamedVariant" was pushed when class 2's name
    // was read — counting all pushes: "hkaSkeleton" (slot 2), "name"
    // (slot 3), "hkRootLevelContainerNamedVariant" (slot 4), "name" was
    // a backref (no push), "className" (slot 5), "variant" (slot 6),
    // "hkRootLevelContainer" (slot 7), "namedVariants" (slot 8). So
    // backref -4 references slot 4 = "hkRootLevelContainerNamedVariant".
    buf.extend_from_slice(&vle_signed(-4));

    // Object #1: TAG_OBJECT_REMEMBER hkRootLevelContainer (remembered_id=1)
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_OBJECT_REMEMBER));
    buf.extend_from_slice(&vle_signed(3)); // class id = hkRootLevelContainer
    // Bitfield: 1 member ("namedVariants"), present.
    buf.extend_from_slice(&presence_bytes(&[true]));
    // namedVariants array: length 1
    buf.extend_from_slice(&vle_signed(1));
    // Element 0 (struct hkRootLevelContainerNamedVariant): bitfield over
    // 3 members (name, className, variant), all present.
    buf.extend_from_slice(&presence_bytes(&[true, true, true]));
    buf.extend_from_slice(&vle_string("myskel")); // name
    // className: backref into prev_strings — "hkaSkeleton" was the first
    // pushed user string (slot 2).
    buf.extend_from_slice(&vle_signed(-2));
    // variant: forward-ref pointer to remembered_id 2 (the skeleton, read
    // next).
    buf.extend_from_slice(&vle_signed(2));

    // Object #2: TAG_OBJECT_REMEMBER hkaSkeleton (remembered_id=2)
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_OBJECT_REMEMBER));
    buf.extend_from_slice(&vle_signed(1)); // class id = hkaSkeleton
    buf.extend_from_slice(&presence_bytes(&[true])); // 1 member present
    buf.extend_from_slice(&vle_string("MySkeletonName"));

    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_END));

    let hkx = read_tagfile2014(&buf).expect("synthesized hkRootLevelContainer fixture parses");
    assert_eq!(
        hkx.objects().len(),
        2,
        "two REMEMBER objects materialize: container + skeleton"
    );

    // Object 0 should be hkRootLevelContainer with one namedVariants entry
    // pointing to object_index 1 (the skeleton).
    let container = &hkx.objects()[0];
    assert_eq!(container.class_name, "hkRootLevelContainer");
    let variants_member = container
        .members
        .iter()
        .find(|m| m.name == "namedVariants")
        .expect("namedVariants member present");
    let variant_array = match &variants_member.value {
        HkxValue::Array(elements) => elements,
        other => panic!("expected Array for namedVariants, got {other:?}"),
    };
    assert_eq!(variant_array.len(), 1, "exactly one variant");

    let variant_struct = match &variant_array[0] {
        HkxValue::TypedObject {
            class_name,
            members,
        } => {
            assert_eq!(class_name, "hkRootLevelContainerNamedVariant");
            members
        }
        other => panic!("expected TypedObject for variant, got {other:?}"),
    };
    let name = variant_struct
        .iter()
        .find(|m| m.name == "name")
        .expect("variant has name");
    let class_name = variant_struct
        .iter()
        .find(|m| m.name == "className")
        .expect("variant has className");
    let pointer = variant_struct
        .iter()
        .find(|m| m.name == "variant")
        .expect("variant has variant pointer");
    match &name.value {
        HkxValue::String { value, .. } => assert_eq!(value, "myskel"),
        other => panic!("expected String for name, got {other:?}"),
    }
    match &class_name.value {
        HkxValue::String { value, .. } => assert_eq!(value, "hkaSkeleton"),
        other => panic!("expected String for className, got {other:?}"),
    }
    // Pointer must have been remapped from remembered_id=2 to
    // object_index=1 (the skeleton).
    match &pointer.value {
        HkxValue::Pointer(Some(target)) => assert_eq!(
            *target, 1,
            "variant pointer should resolve to skeleton object_index 1"
        ),
        other => panic!("expected Pointer(Some(1)), got {other:?}"),
    }

    // Object 1: hkaSkeleton with name="MySkeletonName".
    let skeleton = &hkx.objects()[1];
    assert_eq!(skeleton.class_name, "hkaSkeleton");
    let skel_name = skeleton
        .members
        .iter()
        .find(|m| m.name == "name")
        .expect("skeleton has name");
    match &skel_name.value {
        HkxValue::String { value, .. } => assert_eq!(value, "MySkeletonName"),
        other => panic!("expected String for skeleton name, got {other:?}"),
    }
}

/// TUPLE(3)+REAL field (VEC_4 partial-3 analogue): a C-array of 3 floats
/// encoded with the TUPLE bit set and basic type LT_TYPE_REAL. The reader
/// must emit an Array of 3 F32 values rather than a single scalar.
#[test]
fn tuple_real_3_decodes_as_array_of_three_f32_values() {
    const LT_TYPE_REAL: u32 = 3;
    const LT_TYPE_TUPLE: u32 = 0x20;
    let tuple_real_3 = LT_TYPE_REAL | LT_TYPE_TUPLE;

    let mut buf = synthesize_header_only(true, 1, 13);
    // TAG_FILE_INFO version=3 (single precision)
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_INFO));
    buf.extend_from_slice(&vle_signed(3));

    // Class with one TUPLE(3)+REAL field.
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_METADATA));
    buf.extend_from_slice(&vle_string("Vec3Class"));
    buf.extend_from_slice(&vle_signed(0)); // version
    buf.extend_from_slice(&vle_signed(-1)); // no parent
    buf.extend_from_slice(&vle_signed(1)); // 1 field
    buf.extend_from_slice(&vle_string("pos"));
    buf.extend_from_slice(&vle_signed(tuple_real_3 as i64));
    buf.extend_from_slice(&vle_signed(3)); // tuple_count = 3

    // TAG_OBJECT_REMEMBER class_id=1 (Vec3Class is class index 1)
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_OBJECT_REMEMBER));
    buf.extend_from_slice(&vle_signed(1)); // class index
    // presence bitfield: 1 field present
    buf.extend_from_slice(&presence_bytes(&[true]));
    // 3 f32 values: 1.0, 2.0, 3.0
    buf.extend_from_slice(&1.0f32.to_le_bytes());
    buf.extend_from_slice(&2.0f32.to_le_bytes());
    buf.extend_from_slice(&3.0f32.to_le_bytes());

    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_END));

    let hkx = read_tagfile2014(&buf).expect("TUPLE+REAL stream materializes");
    assert_eq!(hkx.objects().len(), 1);
    let pos = hkx.objects()[0]
        .members
        .iter()
        .find(|m| m.name == "pos")
        .expect("pos member present");
    match &pos.value {
        HkxValue::Array(elems) => {
            assert_eq!(elems.len(), 3, "expected 3 elements");
            assert_eq!(elems[0], HkxValue::F32(1.0));
            assert_eq!(elems[1], HkxValue::F32(2.0));
            assert_eq!(elems[2], HkxValue::F32(3.0));
        }
        other => panic!("expected Array for TUPLE+REAL, got {other:?}"),
    }
}

/// v6+ FILE_INFO sets real_is_double; VEC_4 fields then read 4 f64 values
/// and narrow them to f32 for the HkxValue::F32List public model.
#[test]
fn v6_file_info_double_precision_vec4_narrows_to_f32_list() {
    const LT_TYPE_VEC_4: u32 = 4;

    let mut buf = synthesize_header_only(true, 1, 13);
    // TAG_FILE_INFO version=6 → sets real_is_double
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_INFO));
    buf.extend_from_slice(&vle_signed(6));
    buf.extend_from_slice(&vle_string("sdk_ver")); // sdk version string (v4+)
    buf.extend_from_slice(&0u16.to_le_bytes()); // max_predicate (v5+)
    buf.extend_from_slice(&0u16.to_le_bytes()); // num_verified (v5+)
    // v6: real_is_double is set implicitly (no extra stream bytes)

    // Class with one VEC_4 field.
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_METADATA));
    buf.extend_from_slice(&vle_string("PosClass"));
    buf.extend_from_slice(&vle_signed(0));
    buf.extend_from_slice(&vle_signed(-1));
    buf.extend_from_slice(&vle_signed(1));
    buf.extend_from_slice(&vle_string("q"));
    buf.extend_from_slice(&vle_signed(LT_TYPE_VEC_4 as i64));

    // TAG_OBJECT_REMEMBER class_id=1 (PosClass is class index 1)
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_OBJECT_REMEMBER));
    buf.extend_from_slice(&vle_signed(1)); // class index
    buf.extend_from_slice(&presence_bytes(&[true]));
    // 4 f64 values (x=1.5, y=2.5, z=3.5, w=0.0)
    buf.extend_from_slice(&1.5f64.to_le_bytes());
    buf.extend_from_slice(&2.5f64.to_le_bytes());
    buf.extend_from_slice(&3.5f64.to_le_bytes());
    buf.extend_from_slice(&0.0f64.to_le_bytes());

    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_END));

    let hkx = read_tagfile2014(&buf).expect("v6 double-precision stream materializes");
    assert_eq!(hkx.objects().len(), 1);
    let q = hkx.objects()[0]
        .members
        .iter()
        .find(|m| m.name == "q")
        .expect("q member present");
    match &q.value {
        HkxValue::F32List(floats) => {
            assert_eq!(floats.len(), 4);
            assert!((floats[0] - 1.5f32).abs() < 1e-6);
            assert!((floats[1] - 2.5f32).abs() < 1e-6);
            assert!((floats[2] - 3.5f32).abs() < 1e-6);
            assert!(floats[3].abs() < 1e-6);
        }
        other => panic!("expected F32List for VEC_4 double-precision, got {other:?}"),
    }
}

/// KIND_RECORD field-major encoding: TUPLE(2)+STRUCT with 2 fields each. The
/// SDK writes one presence bit per struct field, then all values for each
/// present field (x[0], x[1], y[0], y[1]).
#[test]
fn tuple_struct_uses_shared_field_bitmap_then_column_major_data() {
    const LT_TYPE_REAL: u32 = 3;
    const LT_TYPE_STRUCT_CODE: u32 = 9;
    const LT_TYPE_TUPLE: u32 = 0x20;
    let tuple_struct_2 = LT_TYPE_STRUCT_CODE | LT_TYPE_TUPLE;

    let mut buf = synthesize_header_only(true, 1, 13);
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_INFO));
    buf.extend_from_slice(&vle_signed(3));

    // Class index 1: "Pair" — two REAL fields: "x" and "y".
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_METADATA));
    buf.extend_from_slice(&vle_string("Pair"));
    buf.extend_from_slice(&vle_signed(0));
    buf.extend_from_slice(&vle_signed(-1));
    buf.extend_from_slice(&vle_signed(2)); // 2 fields
    buf.extend_from_slice(&vle_string("x"));
    buf.extend_from_slice(&vle_signed(LT_TYPE_REAL as i64));
    buf.extend_from_slice(&vle_string("y"));
    buf.extend_from_slice(&vle_signed(LT_TYPE_REAL as i64));

    // Class index 2: "Container" — one TUPLE(2)+STRUCT field "pairs".
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_METADATA));
    buf.extend_from_slice(&vle_string("Container"));
    buf.extend_from_slice(&vle_signed(0));
    buf.extend_from_slice(&vle_signed(-1));
    buf.extend_from_slice(&vle_signed(1)); // 1 field
    buf.extend_from_slice(&vle_string("pairs"));
    buf.extend_from_slice(&vle_signed(tuple_struct_2 as i64));
    buf.extend_from_slice(&vle_signed(2)); // tuple_count = 2
    // STRUCT class name = "Pair" (backref: "Pair" was slot 2 in prev_strings)
    buf.extend_from_slice(&vle_signed(-2));

    // TAG_OBJECT_REMEMBER class_id=2 (Container)
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_OBJECT_REMEMBER));
    buf.extend_from_slice(&vle_signed(2)); // class index
    // Outer presence bitfield: 1 field ("pairs"), present.
    buf.extend_from_slice(&presence_bytes(&[true]));
    // TUPLE(2)+STRUCT "pairs": one shared bit per Pair field; x and y present.
    buf.extend_from_slice(&presence_bytes(&[true, true]));
    // x column: element[0].x = 1.0, element[1].x = 3.0
    buf.extend_from_slice(&1.0f32.to_le_bytes());
    buf.extend_from_slice(&3.0f32.to_le_bytes());
    // y column: element[0].y = 2.0, element[1].y = 4.0
    buf.extend_from_slice(&2.0f32.to_le_bytes());
    buf.extend_from_slice(&4.0f32.to_le_bytes());

    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_END));

    let hkx = read_tagfile2014(&buf).expect("TUPLE+STRUCT stream materializes");
    assert_eq!(hkx.objects().len(), 1);
    let pairs = hkx.objects()[0]
        .members
        .iter()
        .find(|m| m.name == "pairs")
        .expect("pairs member present");
    let elems = match &pairs.value {
        HkxValue::Array(e) => e,
        other => panic!("expected Array for TUPLE+STRUCT, got {other:?}"),
    };
    assert_eq!(elems.len(), 2, "two tuple elements");

    let check = |elem: &HkxValue, fx: f32, fy: f32| {
        let members = match elem {
            HkxValue::TypedObject { members, .. } => members,
            other => panic!("expected TypedObject element, got {other:?}"),
        };
        let x_val = members.iter().find(|m| m.name == "x").expect("x present");
        let y_val = members.iter().find(|m| m.name == "y").expect("y present");
        assert_eq!(x_val.value, HkxValue::F32(fx), "element x");
        assert_eq!(y_val.value, HkxValue::F32(fy), "element y");
    };
    check(&elems[0], 1.0, 2.0);
    check(&elems[1], 3.0, 4.0);
}

/// hkInt64 field in a v3+ stream is VLE-encoded and materializes as I64.
/// Tests both positive and large-negative values to cover the sign-bit path.
#[test]
fn int_field_materializes_as_i64_in_v3_stream() {
    const LT_TYPE_INT: u32 = 2;

    let mut buf = synthesize_header_only(true, 1, 13);
    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_INFO));
    buf.extend_from_slice(&vle_signed(3));

    buf.extend_from_slice(&vle_signed(SYNTH_TAG_METADATA));
    buf.extend_from_slice(&vle_string("IntClass"));
    buf.extend_from_slice(&vle_signed(0));
    buf.extend_from_slice(&vle_signed(-1));
    buf.extend_from_slice(&vle_signed(2)); // 2 fields
    buf.extend_from_slice(&vle_string("pos"));
    buf.extend_from_slice(&vle_signed(LT_TYPE_INT as i64));
    buf.extend_from_slice(&vle_string("neg"));
    buf.extend_from_slice(&vle_signed(LT_TYPE_INT as i64));

    buf.extend_from_slice(&vle_signed(SYNTH_TAG_OBJECT_REMEMBER));
    buf.extend_from_slice(&vle_signed(1)); // class index
    buf.extend_from_slice(&presence_bytes(&[true, true]));
    buf.extend_from_slice(&vle_signed(12345678_i64));
    buf.extend_from_slice(&vle_signed(-9876543_i64));

    buf.extend_from_slice(&vle_signed(SYNTH_TAG_FILE_END));

    let hkx = read_tagfile2014(&buf).expect("INT field stream materializes");
    assert_eq!(hkx.objects().len(), 1);
    let obj = &hkx.objects()[0];
    let pos = obj.members.iter().find(|m| m.name == "pos").expect("pos");
    let neg = obj.members.iter().find(|m| m.name == "neg").expect("neg");
    assert_eq!(pos.value, HkxValue::I64(12345678));
    assert_eq!(neg.value, HkxValue::I64(-9876543));
}
