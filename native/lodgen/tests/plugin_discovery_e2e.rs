//! End-to-end plugin-discovery gate: `run()` must find plugins whose filename differs
//! from the worldspace editor id (`DLC03FarHarbor` lives in `DLCCoast.esm`), while
//! base-game worldspaces (`DiamondCity` in `Fallout4.esm`) resolve via the fast path.
//!
//! Requires the `real-esp` feature. Skips when the FO4 install is absent.
#![cfg(feature = "real-esp")]

use std::path::PathBuf;

use lodgen_native::input::{EspHandle, enumerate_worldspace, scan_for_wrld_plugin};
use lodgen_native::progress::{LodPaths, Progress};
use lodgen_native::settings::LodSettings;

struct NullProgress;
impl Progress for NullProgress {
    fn report(&mut self, _m: &str, _f: f32) {}
}

const FARHARBOR_PLUGIN: &str = "DLCCoast.esm";
const FARHARBOR_EDID: &str = "DLC03FarHarbor";

fn fo4_data() -> Option<PathBuf> {
    let Ok(data) = std::env::var("FO4_DATA") else {
        eprintln!("SKIP plugin_discovery_e2e: FO4_DATA unset");
        return None;
    };
    let p = PathBuf::from(data);
    if p.join(FARHARBOR_PLUGIN).is_file() {
        Some(p)
    } else {
        eprintln!(
            "SKIP plugin_discovery_e2e: {}\\{FARHARBOR_PLUGIN} not found",
            p.display()
        );
        None
    }
}

// ---------------------------------------------------------------------------
// Discovery: scan_for_wrld_plugin finds DLCCoast.esm for DLC03FarHarbor
// ---------------------------------------------------------------------------

/// `scan_for_wrld_plugin` must return `DLCCoast.esm` (not a file named
/// `DLC03FarHarbor.esm` which does not exist) when asked for `DLC03FarHarbor`.
/// This is the core proof that the scan path works.
#[test]
fn scan_finds_dlccoast_for_farharbor() {
    let Some(data) = fo4_data() else { return };

    let found = scan_for_wrld_plugin(&data, FARHARBOR_EDID);
    assert!(
        found.is_some(),
        "scan_for_wrld_plugin failed to find a plugin containing '{FARHARBOR_EDID}' in {}",
        data.display()
    );
    let found = found.unwrap();
    let stem = found.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    assert!(
        stem.eq_ignore_ascii_case("DLCCoast"),
        "expected DLCCoast.esm, got: {found:?}"
    );
    eprintln!("scan_for_wrld_plugin({FARHARBOR_EDID}) → {found:?}");
}

/// The scan must NOT find a file named `DLC03FarHarbor.esm` in the data dir
/// (that file does not exist; the fast-path candidate is a red herring). This
/// confirms the scan selects the *correct* plugin, not a phantom.
#[test]
fn fast_path_candidate_does_not_exist_for_farharbor() {
    let Some(data) = fo4_data() else { return };
    let phantom = data.join("DLC03FarHarbor.esm");
    assert!(
        !phantom.is_file(),
        "DLC03FarHarbor.esm unexpectedly exists at {phantom:?} — fast-path would have won; \
         test premise is invalid"
    );
    eprintln!("confirmed: DLC03FarHarbor.esm does not exist (fast-path misses, scan required)");
}

// ---------------------------------------------------------------------------
// Enumeration via scanned plugin: DLC03FarHarbor is discoverable and non-empty
// ---------------------------------------------------------------------------

/// Load `DLCCoast.esm` (found via scan) and enumerate `DLC03FarHarbor`: the
/// worldspace must be reachable and populated (>100 cells, some placed refs).
#[test]
fn farharbor_enumerated_via_scanned_plugin() {
    let Some(data) = fo4_data() else { return };

    let found = scan_for_wrld_plugin(&data, FARHARBOR_EDID).expect("scan must find DLC03FarHarbor");

    let handle = EspHandle::load(&found, "fo4").expect("load scanned plugin");
    let settings = LodSettings::fo4_default();
    let world =
        enumerate_worldspace(&handle, FARHARBOR_EDID, &settings).expect("enumerate DLC03FarHarbor");

    assert_eq!(world.editor_id, FARHARBOR_EDID);
    assert!(
        world.cells.len() > 100,
        "DLC03FarHarbor must have many exterior cells, got {}",
        world.cells.len()
    );
    assert!(
        !world.refs.is_empty(),
        "DLC03FarHarbor must have placed refs (object LOD), got 0"
    );
    eprintln!(
        "DLC03FarHarbor via scan: {} cells, {} refs, sw={:?} ne={:?}",
        world.cells.len(),
        world.refs.len(),
        world.sw_cell,
        world.ne_cell,
    );
}

// ---------------------------------------------------------------------------
// DiamondCity (Fallout4.esm) resolves via the fast path
// ---------------------------------------------------------------------------

/// `DiamondCity` lives in `Fallout4.esm`, one of the named fast-path candidates,
/// and must enumerate from it directly.
#[test]
fn diamond_city_fast_path_still_works() {
    let Some(data) = fo4_data() else { return };

    // Fast path: Fallout4.esm is in the candidate list.
    let fo4_esm = data.join("Fallout4.esm");
    assert!(fo4_esm.is_file(), "Fallout4.esm not found (prerequisite)");

    let handle = EspHandle::load(&fo4_esm, "fo4").expect("load Fallout4.esm");
    let settings = LodSettings::fo4_default();
    let world = enumerate_worldspace(&handle, "DiamondCity", &settings)
        .expect("enumerate DiamondCity via fast path");

    assert_eq!(world.editor_id, "DiamondCity");
    assert!(
        world.cells.len() > 0,
        "DiamondCity must have at least one exterior cell"
    );
    eprintln!(
        "DiamondCity fast path: {} cells, {} refs, sw={:?} ne={:?}",
        world.cells.len(),
        world.refs.len(),
        world.sw_cell,
        world.ne_cell,
    );
}

// ---------------------------------------------------------------------------
// run() must resolve DLC worldspaces via the Phase-B scan
//
// With the FO4 Data dir in data_dirs, Phase A finds Fallout4.esm and fails to
// enumerate DLC03FarHarbor from it. That "WRLD not in this plugin" miss is soft:
// Phase B must run whenever Phase A did not enumerate the target world. Gating
// the scan on `!fast_path_hit || esp_error.is_none()` skips it here, and run()
// returns "worldspace not found" for every DLC world.
// ---------------------------------------------------------------------------

/// `run()` with data_dirs = [FO4 Data]: Phase A opens Fallout4.esm but cannot
/// enumerate DLC03FarHarbor from it, so Phase B must scan and find DLCCoast.esm.
/// Output goes to a temp dir; only worldspace resolution matters here.
#[test]
fn run_resolves_dlc_worldspace_via_scan_when_fallout4_esm_hits_first() {
    let Some(data) = fo4_data() else { return };

    // Fallout4.esm must be present: it is the Phase-A candidate that hits first.
    assert!(
        data.join("Fallout4.esm").is_file(),
        "Fallout4.esm not found; prerequisite for bug reproduction"
    );

    let out = std::env::temp_dir().join("lodgen_run_gate_test");
    std::fs::create_dir_all(&out).unwrap();

    let settings = LodSettings::fo4_default();
    let paths = LodPaths {
        data_dirs: vec![data.clone()],
        output_dir: out,
        source_data_dir: None,
    };
    let mut progress = NullProgress;

    // No explicit working ESM → exercise the legacy data_dirs discovery path.
    let result = lodgen_native::run(FARHARBOR_EDID, &settings, None, &paths, &mut progress);
    match &result {
        Err(e) => panic!(
            "run() failed for DLC03FarHarbor (gate bug not fixed): {e}\n\
             Expected: Phase-B scan finds DLCCoast.esm and enumerates the worldspace.\n\
             Got error instead — Phase-B scan was suppressed by the fast-path gate."
        ),
        Ok(stats) => {
            // run() produced LOD output — the worldspace was resolved.
            // Terrain cells > 0 proves enumeration succeeded.
            eprintln!(
                "run() gate test PASS: DLC03FarHarbor resolved via Phase-B scan. \
                 btr={} lod_written={}",
                stats.btr, stats.lod_written
            );
            // We don't assert specific counts (run() produces terrain LOD files,
            // counts vary), just that it didn't fail.
            assert!(
                stats.lod_written || stats.btr > 0,
                "run() succeeded but produced no LOD output (btr=0, lod_written=false) — \
                 enumeration may have returned empty cells"
            );
        }
    }
}

/// Control: run() resolves DiamondCity via the Phase-A fast path. Fallout4.esm is a
/// named candidate and contains DiamondCity, so Phase A must short-circuit.
#[test]
fn run_resolves_base_game_worldspace_via_fast_path() {
    let Some(data) = fo4_data() else { return };

    let out = std::env::temp_dir().join("lodgen_run_gate_test_diamondcity");
    std::fs::create_dir_all(&out).unwrap();

    let settings = LodSettings::fo4_default();
    let paths = LodPaths {
        data_dirs: vec![data],
        output_dir: out,
        source_data_dir: None,
    };
    let mut progress = NullProgress;

    // DiamondCity is in Fallout4.esm → Phase A fast-path must enumerate it.
    let result = lodgen_native::run("DiamondCity", &settings, None, &paths, &mut progress);
    match &result {
        Err(e) => panic!("run() failed for DiamondCity (fast-path regression): {e}"),
        Ok(stats) => {
            eprintln!(
                "run() fast-path control PASS: DiamondCity resolved. \
                 btr={} lod_written={}",
                stats.btr, stats.lod_written
            );
            assert!(
                stats.lod_written || stats.btr > 0,
                "run() succeeded but produced no LOD output for DiamondCity"
            );
        }
    }
}
