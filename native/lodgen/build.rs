// Test binaries built with `--features real-esp` link both our `directxtex_native`
// (used by the terrain texture code) and the external `directxtex` crate that
// `esp_authoring_core` pulls in via `bsarchive_native`. Both bundle the DirectXTex
// C++ FFI (`DirectXTexFFI_*`) in their rlib, so an exe-type link reports each
// symbol twice (LNK2005). The umbrella cdylib (`_native.pyd`) links the pair
// without complaint.
//
// `/FORCE:MULTIPLE` keeps the first definition. `tests/golden_terrain_e2e.rs`
// never calls the FFI (it validates `.btr` via nif_core and `.lod` bytes), and
// DDS fidelity is covered by the default-feature texture tests, which never link
// esp. The generic link arg (not `rustc-link-arg-tests`) also covers the lib
// unit-test harness that `cargo test -p lodgen_native --features real-esp`
// builds; the rlib is unaffected.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/FORCE:MULTIPLE");
    }
}
