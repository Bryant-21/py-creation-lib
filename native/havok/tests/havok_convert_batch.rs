use std::path::{Path, PathBuf};

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn clean_temp(name: &str) -> PathBuf {
    let temp = std::env::temp_dir().join(format!("{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    temp
}

fn copy_fixture(relative: &str, destination: &Path) {
    std::fs::copy(repo_path(relative), destination).unwrap_or_else(|error| {
        panic!(
            "failed to copy fixture {relative} to {}: {error}",
            destination.display()
        );
    });
}

#[test]
fn convert_file_creates_parent_dirs_and_writes_structurally_valid_output() {
    let temp = clean_temp("havok_convert_file");
    let src = repo_path("native/havok/tests/fixtures/skeleton.hkx");
    let dst = temp.join("nested/out.hkx");

    havok_native::api::havok_convert_file(&src, &dst, "fo4").unwrap();

    let original = std::fs::read(&src).unwrap();
    let converted = std::fs::read(&dst).unwrap();
    assert_eq!(converted, original);
    havok_native::hkx::read_packfile(&converted).unwrap();
}

#[test]
fn convert_batch_runs_mixed_fixture_tree_with_deterministic_results() {
    let temp = clean_temp("havok_convert_batch");
    let src_root = temp.join("src");
    let dst_root = temp.join("dst");
    std::fs::create_dir_all(src_root.join("a")).unwrap();
    std::fs::create_dir_all(src_root.join("b")).unwrap();
    copy_fixture("native/havok/tests/fixtures/skeleton.hkx", &src_root.join("a/skeleton.hkx"));
    copy_fixture(
        "../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/creatures/deathclaw/expected/character.hkx",
        &src_root.join("b/character.hkx"),
    );
    copy_fixture(
        "python/creation_lib/hkxpack/tests/fixtures/fo76_snallygastercharacter.hkx",
        &src_root.join("b/fo76.hkx"),
    );

    let result = havok_native::api::havok_convert_batch(&src_root, &dst_root, "fo4", true).unwrap();

    // FO76 file converts successfully — all three files should be converted.
    assert_eq!(result.converted, 3);
    assert_eq!(result.skipped, 0);
    assert_eq!(result.errors.len(), 0);
    havok_native::hkx::read_packfile(&std::fs::read(dst_root.join("a/skeleton.hkx")).unwrap())
        .unwrap();
    havok_native::hkx::read_packfile(&std::fs::read(dst_root.join("b/character.hkx")).unwrap())
        .unwrap();
    // The FO76→FO4 converted file should also be a valid packfile.
    havok_native::hkx::read_packfile(&std::fs::read(dst_root.join("b/fo76.hkx")).unwrap()).unwrap();
}

#[test]
fn convert_batch_rejects_flattened_duplicate_destinations() {
    let temp = clean_temp("havok_convert_batch_duplicates");
    let src_root = temp.join("src");
    let dst_root = temp.join("dst");
    std::fs::create_dir_all(src_root.join("a")).unwrap();
    std::fs::create_dir_all(src_root.join("b")).unwrap();
    copy_fixture("native/havok/tests/fixtures/skeleton.hkx", &src_root.join("a/same.hkx"));
    copy_fixture(
        "../bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/creatures/deathclaw/expected/character.hkx",
        &src_root.join("b/same.hkx"),
    );

    let error =
        havok_native::api::havok_convert_batch(&src_root, &dst_root, "fo4", false).unwrap_err();

    assert!(
        error.to_string().contains("duplicate batch destination"),
        "unexpected error: {error}"
    );
    assert!(!dst_root.join("same.hkx").exists());
}
