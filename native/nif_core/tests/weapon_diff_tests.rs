use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use nif_core_native::model::NifFile;
use nif_core_native::weapon_attachment::extract_attachment;
use nif_core_native::weapon_diff::weapon_block_diff;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/nif/fnv/weapons")
        .join(name)
}

fn temp_dir(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "nif_core_native_{name}_{}_{}",
        std::process::id(),
        suffix
    ))
}

#[test]
fn weapon_block_diff_returns_attachment_blocks() {
    let base = NifFile::load(fixture_path("m2_min_base.nif")).expect("load base");
    let sibling = NifFile::load(fixture_path("m2_min_with_attachment.nif")).expect("load sibling");
    let diff = weapon_block_diff(&base, &sibling);
    assert!(!diff.is_empty(), "expected attachment diff");
}

#[test]
fn extract_attachment_writes_attachment_and_patches_base() {
    let dir = temp_dir("extract_attachment");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let base_copy = dir.join("base.nif");
    let sibling_copy = dir.join("sibling.nif");
    let attachment_path = dir.join("out").join("attach.nif");
    std::fs::copy(fixture_path("m2_min_base.nif"), &base_copy).expect("copy base");
    std::fs::copy(fixture_path("m2_min_with_attachment.nif"), &sibling_copy).expect("copy sibling");

    let report = extract_attachment(&base_copy, &sibling_copy, 1, &attachment_path, "Weapon")
        .expect("extract attachment");

    assert!(report.blocks_copied > 0);
    assert!(attachment_path.exists());

    let base_after = NifFile::load(base_copy).expect("load patched base");
    assert!(
        base_after
            .blocks
            .iter()
            .any(|block| block.type_name == "BSConnectPoint::Parents"),
        "patched base must include BSConnectPoint::Parents"
    );

    let _ = std::fs::remove_dir_all(dir);
}
