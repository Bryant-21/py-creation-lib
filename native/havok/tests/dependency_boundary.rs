use std::path::PathBuf;

#[test]
fn havok_native_does_not_depend_on_nif_core_native() {
    let manifest =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .expect("read havok Cargo.toml");

    assert!(
        !manifest.contains("nif_core_native"),
        "havok_native must stay independent from NIF parsing; NIF-side crates own embedded Havok blobs"
    );
}
