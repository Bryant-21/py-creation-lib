use std::path::PathBuf;

use havok_native::collision::tagged_writer::{PatchEntry, TaggedBlobBuilder, TaggedItem};

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

// ---------------------------------------------------------------------------
// Minimal blob structure
// ---------------------------------------------------------------------------

#[test]
fn build_minimal_blob_has_tag0_magic_and_required_sections() {
    let mut builder = TaggedBlobBuilder::new("20190200");
    builder.set_type_section(vec![0u8; 16]);
    builder.set_data(vec![]);
    builder.set_items(vec![]);
    builder.set_patches(vec![]);
    let blob = builder.build();

    // TAG0 container: first 4 bytes are the size header, bytes 4-8 are "TAG0"
    assert_eq!(&blob[4..8], b"TAG0");
    assert!(contains_tag(&blob, b"SDKV"));
    assert!(contains_tag(&blob, b"DATA"));
    assert!(contains_tag(&blob, b"ITEM"));
}

#[test]
fn sdkv_section_contains_sdk_version_string() {
    let mut builder = TaggedBlobBuilder::new("20190200");
    builder.set_type_section(vec![0u8; 16]);
    builder.set_data(vec![]);
    builder.set_items(vec![]);
    builder.set_patches(vec![]);
    let blob = builder.build();

    // Find SDKV tag name (4 bytes), then 4-byte content follows immediately
    let idx = find_bytes(&blob, b"SDKV").expect("SDKV tag not found");
    assert_eq!(&blob[idx + 4..idx + 12], b"20190200");
}

#[test]
fn data_section_contains_exact_bytes_provided() {
    let mut builder = TaggedBlobBuilder::new("20190200");
    builder.set_type_section(vec![0u8; 16]);
    let test_data: Vec<u8> = vec![0xDE, 0xAD, 0xBE, 0xEF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    builder.set_data(test_data.clone());
    builder.set_items(vec![]);
    builder.set_patches(vec![]);
    let blob = builder.build();

    let idx = find_bytes(&blob, b"DATA").expect("DATA tag not found");
    // After "DATA" tag name (4 bytes) comes the content
    let data_start = idx + 4;
    assert_eq!(
        &blob[data_start..data_start + test_data.len()],
        test_data.as_slice()
    );
}

// ---------------------------------------------------------------------------
// ITEM serialization
// ---------------------------------------------------------------------------

#[test]
fn item_entries_serialized_as_12_byte_le_records() {
    let mut builder = TaggedBlobBuilder::new("20190200");
    builder.set_type_section(vec![0u8; 16]);
    builder.set_data(vec![0u8; 32]);
    let items = vec![
        TaggedItem {
            kind: 0x10,
            type_idx: 1,
            data_offset: 0,
            count: 1,
        },
        TaggedItem {
            kind: 0x20,
            type_idx: 4,
            data_offset: 16,
            count: 2,
        },
    ];
    builder.set_items(items);
    builder.set_patches(vec![]);
    let blob = builder.build();

    let idx = find_bytes(&blob, b"ITEM").expect("ITEM tag not found");
    let content_start = idx + 4;

    let packed1 = u32::from_le_bytes(blob[content_start..content_start + 4].try_into().unwrap());
    let off1 = u32::from_le_bytes(
        blob[content_start + 4..content_start + 8]
            .try_into()
            .unwrap(),
    );
    let cnt1 = u32::from_le_bytes(
        blob[content_start + 8..content_start + 12]
            .try_into()
            .unwrap(),
    );
    // packed = (kind << 24) | type_idx
    assert_eq!(packed1, 0x10000001);
    assert_eq!(off1, 0);
    assert_eq!(cnt1, 1);

    let packed2 = u32::from_le_bytes(
        blob[content_start + 12..content_start + 16]
            .try_into()
            .unwrap(),
    );
    let off2 = u32::from_le_bytes(
        blob[content_start + 16..content_start + 20]
            .try_into()
            .unwrap(),
    );
    let cnt2 = u32::from_le_bytes(
        blob[content_start + 20..content_start + 24]
            .try_into()
            .unwrap(),
    );
    assert_eq!(packed2, 0x20000004);
    assert_eq!(off2, 16);
    assert_eq!(cnt2, 2);
}

// ---------------------------------------------------------------------------
// TAG0 size invariant
// ---------------------------------------------------------------------------

#[test]
fn tag0_header_size_field_matches_total_blob_size() {
    let mut builder = TaggedBlobBuilder::new("20190200");
    builder.set_type_section(vec![0u8; 16]);
    builder.set_data(vec![0u8; 64]);
    builder.set_items(vec![]);
    builder.set_patches(vec![]);
    let blob = builder.build();

    // First 4 bytes (big-endian): (type_byte << 24) | size
    let hdr = u32::from_be_bytes(blob[0..4].try_into().unwrap());
    let size = (hdr & 0x00FF_FFFF) as usize;
    assert_eq!(size, blob.len());
}

// ---------------------------------------------------------------------------
// PTCH alignment
// ---------------------------------------------------------------------------

#[test]
fn ptch_content_is_padded_to_8_byte_boundary() {
    let mut builder = TaggedBlobBuilder::new("20190200");
    builder.set_type_section(vec![0u8; 16]);
    builder.set_data(vec![0u8; 16]);
    // One patch entry (12 bytes) — needs 4 bytes of padding to reach 16
    let patches = vec![PatchEntry {
        src: 0,
        flag: 0,
        target: 0,
    }];
    builder.set_items(vec![]);
    builder.set_patches(patches);
    let blob = builder.build();

    let idx = find_bytes(&blob, b"PTCH").expect("PTCH tag not found");
    // Section header is 8 bytes: 4-byte size|type BE, then "PTCH"
    let hdr_offset = idx - 4; // the 4-byte size header precedes the tag name
    let raw = u32::from_be_bytes(blob[hdr_offset..hdr_offset + 4].try_into().unwrap());
    let section_size = (raw & 0x00FF_FFFF) as usize;
    // content_size = section_size - 8
    let content_size = section_size - 8;
    assert_eq!(
        content_size % 8,
        0,
        "PTCH content size {content_size} must be 8-byte aligned"
    );
}

// ---------------------------------------------------------------------------
// TYPE container passthrough
// ---------------------------------------------------------------------------

#[test]
fn type_section_bytes_appear_verbatim_in_blob() {
    let type_bytes: Vec<u8> = b"FAKE_TYPE_CONTAINER_BYTES_ABCDEF".to_vec();
    let mut builder = TaggedBlobBuilder::new("20190200");
    builder.set_type_section(type_bytes.clone());
    builder.set_data(vec![]);
    builder.set_items(vec![]);
    builder.set_patches(vec![]);
    let blob = builder.build();

    // The type section bytes should appear verbatim somewhere in the blob
    let found = blob
        .windows(type_bytes.len())
        .any(|w| w == type_bytes.as_slice());
    assert!(found, "type section bytes not found in blob");
}

// ---------------------------------------------------------------------------
// Round-trip: novablast fixture parse → rebuild → byte-identical
// ---------------------------------------------------------------------------

#[test]
fn novablast_round_trip_is_byte_identical_via_tagged_blob_builder() {
    use havok_native::collision::payload::{parse_tagged_collision, rebuild_tag0_collision};

    let blob =
        fixture_bytes("python/creation_lib/havok/tests/novablast_reference.bin");
    let parsed = parse_tagged_collision(&blob).expect("parse novablast TAG0 collision");
    let rebuilt = rebuild_tag0_collision(&parsed).expect("rebuild TAG0 collision");
    assert_eq!(
        rebuilt, blob,
        "rebuilt blob differs from original (byte-identity gate)"
    );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn contains_tag(blob: &[u8], tag: &[u8; 4]) -> bool {
    blob.windows(4).any(|w| w == tag.as_slice())
}

fn find_bytes(blob: &[u8], pattern: &[u8]) -> Option<usize> {
    blob.windows(pattern.len()).position(|w| w == pattern)
}
