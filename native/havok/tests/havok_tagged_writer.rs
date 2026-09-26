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

fn le_u32(blob: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(blob[offset..offset + 4].try_into().unwrap())
}

fn section_content_size(blob: &[u8], tag: &[u8; 4]) -> usize {
    let idx = find_bytes(blob, tag).expect("section tag");
    let raw = u32::from_be_bytes(blob[idx - 4..idx].try_into().unwrap());
    (raw & 0x00FF_FFFF) as usize - 8
}

#[test]
fn tagged_blob_builder_emits_tag0_sections_items_and_aligned_patches() {
    let type_bytes = b"FAKE_TYPE_CONTAINER_BYTES_ABCDEF".to_vec();
    let mut data = vec![0u8; 64];
    data[0..4].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    let mut builder = TaggedBlobBuilder::new("20190200");
    builder.set_type_section(type_bytes.clone());
    builder.set_data(data.clone());
    builder.set_items(vec![
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
    ]);
    // One 12-byte patch entry needs 4 bytes of padding to reach 16.
    builder.set_patches(vec![PatchEntry {
        src: 0,
        flag: 0,
        target: 0,
    }]);
    let blob = builder.build();

    assert_eq!(&blob[4..8], b"TAG0");
    let header = u32::from_be_bytes(blob[0..4].try_into().unwrap());
    assert_eq!((header & 0x00FF_FFFF) as usize, blob.len());

    let sdkv = find_bytes(&blob, b"SDKV").expect("SDKV tag");
    assert_eq!(&blob[sdkv + 4..sdkv + 12], b"20190200");
    let data_at = find_bytes(&blob, b"DATA").expect("DATA tag") + 4;
    assert_eq!(&blob[data_at..data_at + data.len()], data.as_slice());
    assert!(
        find_bytes(&blob, &type_bytes).is_some(),
        "type bytes verbatim"
    );

    // ITEM records: (kind << 24 | type_idx, offset, count) little-endian.
    let item = find_bytes(&blob, b"ITEM").expect("ITEM tag") + 4;
    let records: Vec<u32> = (0..6).map(|i| le_u32(&blob, item + i * 4)).collect();
    assert_eq!(records, [0x1000_0001, 0, 1, 0x2000_0004, 16, 2]);

    assert_eq!(section_content_size(&blob, b"PTCH") % 8, 0);
}

#[test]
fn novablast_round_trip_is_byte_identical_via_tagged_blob_builder() {
    use havok_native::collision::payload::{parse_tagged_collision, rebuild_tag0_collision};

    let blob = fixture_bytes("python/creation_lib/havok/tests/novablast_reference.bin");
    let parsed = parse_tagged_collision(&blob).expect("parse novablast TAG0 collision");
    let rebuilt = rebuild_tag0_collision(&parsed).expect("rebuild TAG0 collision");
    assert_eq!(
        rebuilt, blob,
        "rebuilt blob differs from original (byte-identity gate)"
    );
}

fn find_bytes(blob: &[u8], pattern: &[u8]) -> Option<usize> {
    blob.windows(pattern.len()).position(|w| w == pattern)
}
