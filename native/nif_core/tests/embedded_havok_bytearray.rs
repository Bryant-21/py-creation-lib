use std::fs;
use std::path::PathBuf;

use nif_core_native::io::{NifReader, NifWriter};
use nif_core_native::model::{NifFile, NifValue};
use nif_core_native::schema::NifSchema;

fn snallygaster_skeleton_fixture() -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/creatures/snallygaster/source/skeleton.nif");
    path.exists().then_some(path)
}

fn load_fixture(path: &PathBuf) -> NifFile {
    let bytes = fs::read(path).expect("read NIF fixture");
    let schema = NifSchema::from_generated();
    NifReader::read(&bytes, &schema).expect("read NIF")
}

fn embedded_havok_blobs(nif: &NifFile) -> Vec<(usize, Vec<u8>)> {
    let mut blobs = Vec::new();
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
            blobs.push((block.block_id, bytes));
        }
    }
    blobs
}

fn first_embedded_havok_data_len(nif: &NifFile) -> Option<(usize, usize)> {
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
        let len = match data {
            NifValue::Bytes(bytes) => bytes.len(),
            NifValue::Array(values) => values.len(),
            _ => 0,
        };
        if len > 0 {
            return Some((block.block_id, len));
        }
    }
    None
}

fn set_embedded_havok_data_size(nif: &mut NifFile, block_id: usize, size: usize) {
    let block = nif.blocks.get_mut(block_id).expect("block exists");
    let Some(NifValue::Struct(binary_data)) = block.fields.get_mut("Binary Data") else {
        panic!("embedded Havok block has Binary Data");
    };
    binary_data.insert("Data Size".to_string(), NifValue::UInt(size as u64));
}

#[test]
fn snallygaster_embedded_havok_blobs_roundtrip_byte_identically() {
    let Some(path) = snallygaster_skeleton_fixture() else {
        eprintln!("skip: Snallygaster skeleton fixture not available");
        return;
    };
    let schema = NifSchema::from_generated();
    let mut nif = load_fixture(&path);
    let before = embedded_havok_blobs(&nif);
    assert!(
        !before.is_empty(),
        "expected embedded Havok blobs in fixture"
    );

    let written = NifWriter::write_to_bytes(&mut nif, &schema).expect("write NIF");
    let reparsed = NifReader::read(&written, &schema).expect("re-read written NIF");
    let after = embedded_havok_blobs(&reparsed);

    assert_eq!(
        before, after,
        "embedded Havok blobs changed after NIF roundtrip"
    );
}

#[test]
fn writer_rejects_embedded_havok_bytearray_data_size_mismatch() {
    let Some(path) = snallygaster_skeleton_fixture() else {
        eprintln!("skip: Snallygaster skeleton fixture not available");
        return;
    };
    let schema = NifSchema::from_generated();
    let mut nif = load_fixture(&path);
    let (block_id, data_len) =
        first_embedded_havok_data_len(&nif).expect("fixture has embedded Havok data");
    set_embedded_havok_data_size(&mut nif, block_id, data_len + 1);

    let err = NifWriter::write_to_bytes(&mut nif, &schema)
        .expect_err("stale ByteArray Data Size must not be padded or truncated");
    let message = err.to_string();
    assert!(
        message.contains("ByteArray") && message.contains("Data Size"),
        "unexpected error: {message}"
    );
}
