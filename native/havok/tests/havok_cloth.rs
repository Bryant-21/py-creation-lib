use std::path::PathBuf;

use havok_native::cloth::{cloth_metadata_from_blob, load_cloth_hkx, validate_cloth_blob};
use havok_native::hkx::read_packfile;

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

fn optional_fixture(relative: &str) -> Option<Vec<u8>> {
    let path = repo_path(relative);
    if !path.exists() {
        return None;
    }
    Some(std::fs::read(&path).unwrap_or_else(|error| {
        panic!("failed to read fixture {relative}: {error}");
    }))
}

#[test]
fn parses_runtime_metadata_inventory_from_tracked_hkx_blob() {
    let blob = fixture_bytes("native/havok/tests/fixtures/skeleton.hkx");

    let metadata = cloth_metadata_from_blob(&blob).expect("parse cloth metadata scaffold");

    assert!(metadata.object_count > 0);
    assert!(
        metadata
            .class_inventory
            .iter()
            .any(|entry| entry.class_name == "hkRootLevelContainer")
    );
    assert!(!metadata.has_cloth_data);
    assert!(!metadata.has_setup_data);
    assert!(!metadata.has_runtime_data);
}

#[test]
fn semantic_validation_reports_no_cloth_data_for_animation_blob() {
    let blob = fixture_bytes("native/havok/tests/fixtures/skeleton.hkx");

    let summary = validate_cloth_blob(&blob).expect("validation should succeed for parseable blob");

    // The skeleton is an animation HKX with no hclClothData — expect a NO_CLOTH_DATA error.
    let issues = summary
        .get("issues")
        .and_then(|v| v.as_array())
        .expect("issues array");
    assert!(
        issues
            .iter()
            .any(|i| i.get("code").and_then(|c| c.as_str()) == Some("NO_CLOTH_DATA")),
        "expected NO_CLOTH_DATA error in: {summary:?}",
    );
}

#[test]
fn extracts_bathrobe_cloth_blob_when_fixture_present() {
    let blob = match optional_fixture("../tests/fixtures/cloth/bathrobe_outfitm_cloth.hkx") {
        Some(bytes) => bytes,
        None => return, // skip when fixture not prepared
    };

    assert!(!blob.is_empty(), "bathrobe blob fixture must be non-empty");
    assert!(
        blob.len() > 1024,
        "bathrobe blob is unexpectedly small: {} bytes",
        blob.len()
    );
}

#[test]
fn bathrobe_metadata_reports_cloth_data_present() {
    let blob = match optional_fixture("../tests/fixtures/cloth/bathrobe_outfitm_cloth.hkx") {
        Some(bytes) => bytes,
        None => return,
    };

    let metadata = cloth_metadata_from_blob(&blob).expect("parse bathrobe cloth metadata");

    assert!(
        metadata.has_cloth_data,
        "bathrobe must report has_cloth_data=true"
    );
    assert!(
        metadata.has_runtime_data,
        "bathrobe must report has_runtime_data=true"
    );
    assert!(
        metadata
            .class_inventory
            .iter()
            .any(|entry| entry.class_name == "hclClothData"),
        "bathrobe inventory must include hclClothData",
    );
}

#[test]
fn load_cloth_hkx_from_bathrobe_returns_non_empty_object_graph() {
    let blob = match optional_fixture("../tests/fixtures/cloth/bathrobe_outfitm_cloth.hkx") {
        Some(bytes) => bytes,
        None => return, // skip when fixture not prepared
    };

    let hkx = load_cloth_hkx(&blob).expect("load cloth HKX from bathrobe blob");

    assert!(
        hkx.objects().len() > 0,
        "bathrobe HKX must have at least one object"
    );
}

/// A vanilla FO4 cape cloth blob must round-trip byte-exact through
/// `read_packfile` -> writer.
#[test]
fn test_vanilla_cape_roundtrip() {
    let original =
        fixture_bytes("native/havok/tests/fixtures/cloth/vanilla_cape_outfitm.bin");

    let mut parsed = read_packfile(&original).expect("parse vanilla cape blob");
    // Force the writer path (otherwise save() short-circuits to source bytes).
    let _ = parsed.objects_mut();
    let reemitted = parsed.save();

    if original != reemitted {
        let divergence = original
            .iter()
            .zip(reemitted.iter())
            .position(|(a, b)| a != b);
        let len_msg = format!(
            "len(original)={} len(reemitted)={}",
            original.len(),
            reemitted.len()
        );
        let div_msg = match divergence {
            Some(off) => {
                let lo = off.saturating_sub(8);
                let hi_o = (off + 24).min(original.len());
                let hi_r = (off + 24).min(reemitted.len());
                format!(
                    "first divergence at offset 0x{off:x} ({off}); \
                     original[{lo:#x}..{hi_o:#x}]={:02x?}; \
                     reemitted[{lo:#x}..{hi_r:#x}]={:02x?}",
                    &original[lo..hi_o],
                    &reemitted[lo..hi_r],
                )
            }
            None => format!(
                "no byte differs but len mismatch (truncation at {})",
                original.len().min(reemitted.len())
            ),
        };
        panic!("vanilla cape blob must round-trip byte-exact; {len_msg}; {div_msg}");
    }
}
