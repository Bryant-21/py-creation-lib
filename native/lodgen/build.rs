// Integration-test binaries built with `--features real-esp` link BOTH our
// `directxtex_native` (referenced by the lodgen lib's terrain texture code) and
// the external `directxtex` crate that `esp_authoring_core` pulls in via
// `bsarchive_native`. Both compile the same DirectXTex C++ FFI (`DirectXTexFFI_*`)
// and bundle the object into their rlib, so an exe-type link reports each symbol
// twice (LNK2005). The umbrella cdylib (`_native.pyd`) links the identical pair
// without complaint — only the strict exe/test link trips.
//
// The end-to-end enumeration golden test (`tests/golden_terrain_e2e.rs`) never
// CALLS the DirectXTex FFI (it validates `.btr` via nif_core and `.lod` bytes,
// not `.dds` encoding), so it does not matter which of the two identical FFI
// copies the linker keeps. `/FORCE:MULTIPLE` lets the link proceed by keeping the
// first definition. DDS encoding fidelity is covered by the default-feature
// texture tests, which never link esp and so never hit this path.
//
// Scoped to executable-style test links. `cargo:rustc-link-arg-tests` only
// reaches integration-test binaries; `cargo test -p lodgen_native --features
// real-esp ...` also builds the lib unit-test harness, so the generic link arg
// is needed for that harness as well. The rlib itself is unaffected.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/FORCE:MULTIPLE");
    }
}
