use pyo3::prelude::*;

pub mod asset_source;
pub mod atlas;
pub mod billboards;
pub mod descriptors;
pub mod driver;
pub mod game;
pub mod input;
pub mod naming;
pub mod objects;
pub mod output;
pub mod progress;
pub mod settings;
pub mod terrain;
pub mod trees;

pub fn crate_marker() -> &'static str {
    "lodgen_native"
}

// ---------------------------------------------------------------------------
// Rust-level entry points (test seam + real run)
// ---------------------------------------------------------------------------

/// Run full LOD generation (terrain + objects + trees) from a pre-built
/// `WorldspaceInput` (bypassing ESP). `run` calls it after enumeration; tests call
/// it directly. Object/tree LOD is skipped when `world.refs` is empty (terrain-only
/// worlds).
pub fn run_with_world(
    world: &input::WorldspaceInput,
    settings: &settings::LodSettings,
    paths: &progress::LodPaths,
    progress: &mut dyn progress::Progress,
) -> anyhow::Result<progress::LodGenStats> {
    let mut resolved_settings = settings.clone();
    let has_layout_override = settings.global.southwest_cell.is_some()
        || settings.global.bounds.is_some()
        || settings.global.stride.is_some()
        || settings.global.align != 0;
    if settings.global.use_source_lodsettings && !has_layout_override {
        if let Some(source_data_dir) = paths.source_data_dir.as_deref() {
            if let Some(source) =
                output::lodsettings::read_source(source_data_dir, &world.editor_id)?
            {
                if source.starfield_cell_grid {
                    progress.report(
                        "ignoring Starfield source LOD settings: they count 100 m cells, not FO4 cells",
                        0.0,
                    );
                } else {
                    resolved_settings.global.southwest_cell =
                        Some([source.southwest.0, source.southwest.1]);
                    resolved_settings.global.stride = Some(source.stride);
                    if let Some((min, max)) = source.levels {
                        resolved_settings.global.lod_min = min;
                        resolved_settings.global.lod_max = max;
                    }
                    progress.report(
                        &format!(
                            "using source LOD settings: southwest=({},{}) stride={} levels={}..{}",
                            source.southwest.0,
                            source.southwest.1,
                            source.stride,
                            resolved_settings.global.lod_min,
                            resolved_settings.global.lod_max
                        ),
                        0.0,
                    );
                }
            }
        }
    }
    let settings = &resolved_settings;
    if paths.data_dirs.iter().any(|path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("ba2"))
    }) {
        progress.report("indexing LOD assets from BA2 archives", 0.0);
    }
    let _asset_sources = asset_source::prepare(&paths.data_dirs)?;
    let game = game::Game::fo4();
    let mut stats = if settings.global.generate_terrain {
        driver::run_terrain(world, settings, &game, paths, progress)?
    } else {
        progress::LodGenStats::default()
    };
    if !settings.global.generate_objects {
        return Ok(stats);
    }
    match settings.objects.source {
        settings::ObjectSource::Records => {
            if !world.refs.is_empty() {
                let obj = driver::build_object_lod(world, settings, &game, paths, progress)?;
                stats.bto += obj.bto;
                stats.btt += obj.btt;
                stats.dds += obj.dds;
                stats.warnings.extend(obj.warnings);
            }
        }
        settings::ObjectSource::Fo76Bto => {
            let obj = objects::fo76_bto::build_object_lod(world, settings, &game, paths, progress)?;
            stats.bto += obj.bto;
            stats.btt += obj.btt;
            stats.dds += obj.dds;
            stats.warnings.extend(obj.warnings);
        }
        settings::ObjectSource::Fo76BtoAtlas => {
            let obj = objects::fo76_bto::build_object_lod(world, settings, &game, paths, progress)?;
            stats.bto += obj.bto;
            stats.btt += obj.btt;
            stats.dds += obj.dds;
            stats.warnings.extend(obj.warnings);
        }
    }
    Ok(stats)
}

/// Full entry: enumerate the named WRLD into a `WorldspaceInput`, then run. The
/// plugin is `working_esm` when given, else the first plugin in `paths.data_dirs`
/// that contains the WRLD.
pub fn run(
    world_id: &str,
    settings: &settings::LodSettings,
    working_esm: Option<&std::path::Path>,
    paths: &progress::LodPaths,
    progress: &mut dyn progress::Progress,
) -> anyhow::Result<progress::LodGenStats> {
    run_with_object_lod_overlay(world_id, settings, working_esm, None, paths, progress)
}

fn run_with_object_lod_overlay(
    world_id: &str,
    settings: &settings::LodSettings,
    working_esm: Option<&std::path::Path>,
    object_lod_overlay: Option<&std::path::Path>,
    paths: &progress::LodPaths,
    progress: &mut dyn progress::Progress,
) -> anyhow::Result<progress::LodGenStats> {
    if object_lod_overlay.is_some() && working_esm.is_none() {
        anyhow::bail!("object LOD overlay requires an explicit working_esm");
    }
    if source_bto_object_only_without_world(settings) {
        progress.report(
            "source-BTO object-only: using synthetic world input, skipping ESM enumeration",
            0.0,
        );
        let world = input::WorldspaceInput::from_cells(world_id, Vec::new());
        return run_with_world(&world, settings, paths, progress);
    }

    // Pinned source (conversion path): an explicit working ESM (the FO76→FO4 output,
    // mods/<out>/<out>.esm) is the sole source of the worldspace and every record
    // lodgen reads (WRLD/CELL/LAND/REFR + base MNAM). `paths.data_dirs` is asset-only
    // here, so a stale plugin copy in the game install cannot shadow the fresh output
    // (which carries the synthesized DistantLOD/MNAM object LOD depends on). A missing
    // ESM or one without the worldspace is a hard error, with no fallback to the game
    // data dir.
    if let Some(esm) = working_esm {
        progress.report(
            &format!("enumerating worldspace {world_id} from {}", esm.display()),
            0.0,
        );
        let world = try_enumerate(world_id, esm, settings, object_lod_overlay, progress)?;
        return run_with_world(&world, settings, paths, progress);
    }

    // Two-phase plugin discovery (standalone / UI use, no explicit working ESM):
    //
    // Phase A (fast path): named candidates per data_dir ("<world_id>.esm",
    //   "<world_id>.esp", "Fallout4.esm", "SeventySix.esm").
    //
    // Phase B (scan fallback): scan every .esm/.esp in each data_dir (sorted) and
    //   take the last one that contains a WRLD with the requested editor id. Handles
    //   DLC worldspaces (e.g. DLC03FarHarbor in DLCCoast.esm) and arbitrarily named
    //   converted-mod plugins.
    let mut esp_error: Option<anyhow::Error> = None;

    // Phase A — named fast-path candidates.
    for dir in &paths.data_dirs {
        let plugin_candidates = [
            format!("{}.esm", world_id),
            format!("{}.esp", world_id),
            "Fallout4.esm".to_string(),
            // FO76→FO4 converted-mod plugin: worldspace editor ID is APPALACHIA but
            // the plugin is SeventySix.esm. Without this entry every APPALACHIA run
            // falls through to the full-directory Phase-B scan, which is expensive.
            "SeventySix.esm".to_string(),
        ];
        for candidate in &plugin_candidates {
            let plugin_path = dir.join(candidate);
            if plugin_path.is_file() {
                match try_enumerate(world_id, &plugin_path, settings, None, progress) {
                    Ok(world) => {
                        return run_with_world(&world, settings, paths, progress);
                    }
                    Err(e) => {
                        esp_error = Some(e);
                    }
                }
            }
        }
    }

    // Phase B: full directory scan. Reaching here means Phase A found no named
    // candidate, found one without the WRLD (a soft miss, e.g. Fallout4.esm for a
    // DLC worldspace), or hit a parse error. In every case the worldspace may live in
    // another plugin, so the scan always runs. `already_tried` prevents re-opens.
    {
        // Collect all candidate paths from the scan (one winner per dir, last
        // sorted = load-order winner) then try them.
        #[cfg(feature = "real-esp")]
        for dir in &paths.data_dirs {
            if let Some(plugin_path) = input::scan_for_wrld_plugin(dir, world_id) {
                // Skip if this path was already tried in Phase A.
                let stem = plugin_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                let already_tried = stem.eq_ignore_ascii_case(world_id)
                    || stem.eq_ignore_ascii_case("Fallout4")
                    || stem.eq_ignore_ascii_case("SeventySix");
                if already_tried {
                    continue;
                }
                match try_enumerate(world_id, &plugin_path, settings, None, progress) {
                    Ok(world) => {
                        return run_with_world(&world, settings, paths, progress);
                    }
                    Err(e) => {
                        esp_error = Some(e);
                    }
                }
            }
        }
    }

    Err(esp_error.unwrap_or_else(|| {
        anyhow::anyhow!(
            "no plugin found containing WRLD '{}' in data_dirs {:?}",
            world_id,
            paths.data_dirs
        )
    }))
}

fn source_bto_object_only_without_world(settings: &settings::LodSettings) -> bool {
    let source_bto = matches!(
        settings.objects.source,
        settings::ObjectSource::Fo76Bto | settings::ObjectSource::Fo76BtoAtlas
    ) || settings.objects.fo76_bto_atlas_pages;
    let atlas_mode = matches!(
        settings.objects.source,
        settings::ObjectSource::Fo76BtoAtlas
    ) || settings.objects.fo76_bto_atlas_pages;
    source_bto
        && settings.global.generate_objects
        && !settings.global.generate_terrain
        && !settings.global.generate_trees
        && !(atlas_mode && settings.trees.trees_3d)
}

fn try_enumerate(
    world_id: &str,
    plugin_path: &std::path::Path,
    settings: &settings::LodSettings,
    object_lod_overlay: Option<&std::path::Path>,
    progress: &mut dyn progress::Progress,
) -> anyhow::Result<input::WorldspaceInput> {
    // Load the plugin (+ masters from the same dir) and enumerate the WRLD.
    // If this plugin does not contain `world_id`, enumerate_worldspace errors and
    // the caller (`run`) advances to the next plugin candidate.
    let started = std::time::Instant::now();
    let handle = input::EspHandle::load_with_overlay(plugin_path, "fo4", object_lod_overlay)?;
    progress.report(
        &format!(
            "LOD plugin load and index: {:.3}s ({})",
            started.elapsed().as_secs_f64(),
            plugin_path.display()
        ),
        0.0,
    );
    let started = std::time::Instant::now();
    let world = input::enumerate_worldspace(&handle, world_id, settings)?;
    progress.report(
        &format!(
            "LOD world enumeration: {:.3}s ({world_id})",
            started.elapsed().as_secs_f64()
        ),
        0.0,
    );
    Ok(world)
}

// ---------------------------------------------------------------------------
// PyO3 shims
// ---------------------------------------------------------------------------

/// Python-visible paths struct passed to `generate_lod`.
///
/// `data_dirs` are ASSET-ONLY loose roots or BA2 files (LOD meshes/textures),
/// searched after `output_dir`. `working_esm`, when set, is the SOLE plugin lodgen parses for the
/// worldspace + records (WRLD/CELL/LAND/REFR + base MNAM) — `data_dirs` is never
/// scanned for the plugin, so a stale copy in the game install cannot shadow the
/// freshly-built conversion output.
#[pyclass]
pub struct PyLodPaths {
    #[pyo3(get, set)]
    pub data_dirs: Vec<String>,
    #[pyo3(get, set)]
    pub output_dir: String,
    #[pyo3(get, set)]
    pub working_esm: Option<String>,
    #[pyo3(get, set)]
    pub source_data_dir: Option<String>,
    #[pyo3(get, set)]
    pub object_lod_overlay: Option<String>,
}

#[pymethods]
impl PyLodPaths {
    #[new]
    #[pyo3(signature = (data_dirs, output_dir, working_esm=None, source_data_dir=None, object_lod_overlay=None))]
    pub fn new(
        data_dirs: Vec<String>,
        output_dir: String,
        working_esm: Option<String>,
        source_data_dir: Option<String>,
        object_lod_overlay: Option<String>,
    ) -> Self {
        PyLodPaths {
            data_dirs,
            output_dir,
            working_esm,
            source_data_dir,
            object_lod_overlay,
        }
    }
}

/// Python-visible stats returned from `generate_lod`.
#[pyclass]
pub struct PyLodGenStats {
    #[pyo3(get)]
    pub btr: u32,
    #[pyo3(get)]
    pub bto: u32,
    #[pyo3(get)]
    pub btt: u32,
    #[pyo3(get)]
    pub dds: u32,
    #[pyo3(get)]
    pub lod_written: bool,
    #[pyo3(get)]
    pub warnings: Vec<String>,
}

/// Python progress wrapper: wraps a Python callable `(msg: str, frac: float) -> None`.
struct PyProgress {
    callback: Option<pyo3::Py<pyo3::PyAny>>,
}

impl progress::Progress for PyProgress {
    fn report(&mut self, msg: &str, frac: f32) {
        if let Some(ref cb) = self.callback {
            pyo3::Python::attach(|py| {
                let _ = cb.call1(py, (msg, frac));
            });
        }
    }
}

/// Return the default FO4 LOD settings as a compact JSON string.
///
/// Python callers can pass the result directly to `generate_lod` as `settings_json`.
/// This keeps the schema in sync with the Rust default instead of duplicating it
/// in Python.
#[pyfunction]
fn default_settings_json() -> String {
    settings::default_settings_json()
}

/// Return every LOD-eligible top-level WRLD EditorID in `plugin_path`.
///
/// A worldspace is eligible when it has a matching World Children group with
/// at least one exterior CELL+LAND pair. Nested records and empty shells are
/// excluded because the generator cannot produce terrain LOD for them.
#[pyfunction]
#[pyo3(signature = (plugin_path, game = "fo4"))]
fn discover_worldspaces(
    py: pyo3::Python<'_>,
    plugin_path: &str,
    game: &str,
) -> PyResult<Vec<String>> {
    let plugin_path = std::path::PathBuf::from(plugin_path);
    let game = game.to_string();
    py.detach(move || input::discover_worldspaces(&plugin_path, &game))
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("{e}")))
}

/// Count source FO76 object-LOD tiles available for one worldspace.
#[pyfunction]
fn count_fo76_bto_tiles(
    py: pyo3::Python<'_>,
    source_data_dir: &str,
    world_editor_id: &str,
) -> PyResult<usize> {
    let source_data_dir = std::path::PathBuf::from(source_data_dir);
    let world_editor_id = world_editor_id.to_string();
    py.detach(move || {
        objects::fo76_bto::enumerate_source_bto_tiles(&source_data_dir, &world_editor_id)
            .map(|tiles| tiles.len())
    })
    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("{e}")))
}

#[pyfunction]
fn collect_fo76_bto_tree_billboard_species(
    py: pyo3::Python<'_>,
    world_editor_id: &str,
    settings_json: &str,
    source_data_dir: &str,
) -> PyResult<Vec<(String, String, u64)>> {
    let settings: settings::LodSettings = serde_json::from_str(settings_json)
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(format!("settings JSON: {e}")))?;
    let source_root = std::path::PathBuf::from(source_data_dir);

    let species = py
        .detach(|| {
            objects::fo76_bto::collect_tree_billboard_species(
                &source_root,
                world_editor_id,
                &settings,
            )
        })
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("{e}")))?;

    Ok(species
        .into_iter()
        .map(|entry| {
            (
                entry.model,
                entry.resolved_path.to_string_lossy().to_string(),
                entry.instance_count,
            )
        })
        .collect())
}

/// PyO3 entry point.
///
/// `settings_json` is a JSON-serialized `LodSettings`; `paths` supplies data and output
/// dirs; `progress` is an optional Python callable `(msg: str, frac: float)`.
///
/// The GIL is released during generation so other Python threads (e.g. the `ui/lodgen`
/// UI) stay responsive; `PyProgress::report` re-acquires it via `Python::attach` for
/// the callback.
#[pyfunction]
fn generate_lod(
    py: pyo3::Python<'_>,
    world_editor_id: &str,
    settings_json: &str,
    paths: &PyLodPaths,
    progress: Option<pyo3::Py<pyo3::PyAny>>,
) -> PyResult<PyLodGenStats> {
    let settings: settings::LodSettings = serde_json::from_str(settings_json)
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(format!("settings JSON: {e}")))?;

    let lod_paths = progress::LodPaths {
        data_dirs: paths
            .data_dirs
            .iter()
            .map(std::path::PathBuf::from)
            .collect(),
        output_dir: std::path::PathBuf::from(&paths.output_dir),
        source_data_dir: paths.source_data_dir.as_ref().map(std::path::PathBuf::from),
    };
    let working_esm = paths.working_esm.as_ref().map(std::path::PathBuf::from);
    let object_lod_overlay = paths
        .object_lod_overlay
        .as_ref()
        .map(std::path::PathBuf::from);

    let mut prog = PyProgress { callback: progress };

    // Release the GIL for the duration of the multi-second/minute generation.
    // `Py<PyAny>` (held inside `PyProgress`) is `Send`; `PyProgress::report`
    // re-acquires via `Python::attach` before each callback invocation.
    let stats = py
        .detach(|| {
            run_with_object_lod_overlay(
                world_editor_id,
                &settings,
                working_esm.as_deref(),
                object_lod_overlay.as_deref(),
                &lod_paths,
                &mut prog,
            )
        })
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("{e}")))?;

    Ok(PyLodGenStats {
        btr: stats.btr,
        bto: stats.bto,
        btt: stats.btt,
        dds: stats.dds,
        lod_written: stats.lod_written,
        warnings: stats.warnings,
    })
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyLodPaths>()?;
    m.add_class::<PyLodGenStats>()?;
    m.add_function(wrap_pyfunction!(generate_lod, m)?)?;
    m.add_function(wrap_pyfunction!(default_settings_json, m)?)?;
    m.add_function(wrap_pyfunction!(discover_worldspaces, m)?)?;
    m.add_function(wrap_pyfunction!(count_fo76_bto_tiles, m)?)?;
    m.add_function(wrap_pyfunction!(
        collect_fo76_bto_tree_billboard_species,
        m
    )?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_marker_is_lodgen() {
        assert_eq!(crate_marker(), "lodgen_native");
    }
}

/// Returns the fast-path plugin candidate names for the given `world_id`.
/// This mirrors the inline array in `run()` and is exposed only for testing.
#[cfg(test)]
pub fn fast_path_candidates(world_id: &str) -> Vec<String> {
    vec![
        format!("{}.esm", world_id),
        format!("{}.esp", world_id),
        "Fallout4.esm".to_string(),
        "SeventySix.esm".to_string(),
    ]
}

#[cfg(test)]
mod run_tests {
    use super::*;
    use crate::progress::{LodPaths, Progress};
    use crate::settings::{LodSettings, ObjectSource};

    struct NullProgress;
    impl Progress for NullProgress {
        fn report(&mut self, _m: &str, _f: f32) {}
    }

    /// The fast-path candidate list must include SeventySix.esm so that
    /// APPALACHIA (FO76→FO4 converted-mod) is found without a full Data dir scan.
    #[test]
    fn fast_path_candidates_include_seventysix_esm() {
        let candidates = fast_path_candidates("APPALACHIA");
        assert!(
            candidates
                .iter()
                .any(|c| c.eq_ignore_ascii_case("SeventySix.esm")),
            "SeventySix.esm must be in the fast-path candidates; got: {:?}",
            candidates
        );
        // Verify world_id.esm/esp are also present (regression guard).
        assert!(candidates.contains(&"APPALACHIA.esm".to_string()));
        assert!(candidates.contains(&"APPALACHIA.esp".to_string()));
        assert!(candidates.contains(&"Fallout4.esm".to_string()));
    }

    /// With an explicit working ESM, `run()` must read the worldspace + records
    /// SOLELY from that plugin and must NOT fall through to the `data_dirs` scan
    /// (which could resolve a stale plugin from the FO4 game install). The default
    /// build has no `real-esp`, so enumeration bails with the stub error — the
    /// invariant under test is that the failure is NOT the data_dirs-scan error.
    #[test]
    fn run_with_working_esm_does_not_scan_data_dirs() {
        let s = LodSettings::fo4_default();
        let working = std::path::PathBuf::from("definitely/missing/Working.esm");
        let paths = LodPaths {
            // A non-empty data_dirs that would be scanned if the pin were ignored.
            data_dirs: vec![std::path::PathBuf::from(".")],
            output_dir: std::env::temp_dir().join("lodgen_pin_test"),
            source_data_dir: None,
        };
        let mut p = NullProgress;
        let err = run("APPALACHIA", &s, Some(working.as_path()), &paths, &mut p)
            .expect_err("default build has no real-esp → enumeration must error");
        let msg = err.to_string();
        assert!(
            !msg.contains("data_dirs"),
            "run() with an explicit working ESM must not scan data_dirs; got: {msg}"
        );
    }

    /// Regression guard for the legacy (UI/standalone) path: with no working ESM,
    /// `run()` still discovers the plugin by scanning `data_dirs`. An empty data
    /// dir yields the data_dirs-scan error, proving the scan path is taken.
    #[test]
    fn run_without_working_esm_scans_data_dirs() {
        let s = LodSettings::fo4_default();
        let empty = std::env::temp_dir().join("lodgen_pin_test_empty_datadir");
        std::fs::create_dir_all(&empty).unwrap();
        let paths = LodPaths {
            data_dirs: vec![empty],
            output_dir: std::env::temp_dir().join("lodgen_pin_test_out"),
            source_data_dir: None,
        };
        let mut p = NullProgress;
        let err = run("APPALACHIA", &s, None, &paths, &mut p)
            .expect_err("empty data dir → no plugin found");
        assert!(
            err.to_string().contains("data_dirs"),
            "legacy path must scan data_dirs; got: {err}"
        );
    }

    #[test]
    fn source_bto_object_only_can_skip_world_without_standard_tree_refs() {
        let mut s = LodSettings::fo4_default();
        s.global.generate_terrain = false;
        s.global.generate_objects = true;
        s.global.generate_trees = false;
        s.objects.source = ObjectSource::Fo76BtoAtlas;
        s.objects.fo76_bto_atlas_pages = true;
        s.trees.trees_3d = false;

        assert!(source_bto_object_only_without_world(&s));

        s.trees.trees_3d = true;
        assert!(!source_bto_object_only_without_world(&s));
    }

    #[test]
    fn run_with_world_terrain_only() {
        let w = crate::input::WorldspaceInput::from_cells(
            "W",
            (0..4)
                .flat_map(|y| (0..4).map(move |x| (x, y)))
                .map(|(x, y)| crate::input::CellInput {
                    x,
                    y,
                    heights: vec![0.0; 33 * 33],
                    vertex_colors: vec![[255, 255, 255]; 33 * 33],
                    layers: Vec::new(),
                    hidden_quadrants: [false; 4],
                    water_height: f32::MIN,
                })
                .collect(),
        );
        let s = LodSettings::fo4_default();
        let out = std::env::temp_dir().join("lodgen_run_test");
        std::fs::create_dir_all(&out).unwrap();
        let paths = LodPaths {
            data_dirs: vec![std::path::PathBuf::from(".")],
            output_dir: out,
            source_data_dir: None,
        };
        let mut p = NullProgress;
        let stats = run_with_world(&w, &s, &paths, &mut p).unwrap();
        // 4x4: L4->1, L8->1, L16->1, L32->1 = 4 btr
        assert_eq!(stats.btr, 4);
        assert!(stats.lod_written);
    }

    #[test]
    fn run_with_world_uses_source_lodsettings_unless_layout_is_explicit() {
        let world = crate::input::WorldspaceInput::from_cells(
            "Tamriel",
            vec![crate::input::CellInput {
                x: -57,
                y: -43,
                heights: vec![0.0; 33 * 33],
                vertex_colors: vec![[255, 255, 255]; 33 * 33],
                layers: Vec::new(),
                hidden_quadrants: [false; 4],
                water_height: f32::MIN,
            }],
        );
        let root =
            std::env::temp_dir().join(format!("lodgen_source_window_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let source_dir = root.join("source");
        let settings_dir = source_dir.join("LODSettings");
        std::fs::create_dir_all(&settings_dir).unwrap();
        std::fs::write(
            settings_dir.join("Tamriel.lod"),
            crate::output::lodsettings::encode((-96, -96), 256, 4, 32),
        )
        .unwrap();

        let mut settings = LodSettings::fo4_default();
        settings.global.generate_objects = false;
        let source_output = root.join("source_output");
        let source_paths = LodPaths {
            data_dirs: vec![root.clone()],
            output_dir: source_output.clone(),
            source_data_dir: Some(source_dir.clone()),
        };
        run_with_world(&world, &settings, &source_paths, &mut NullProgress).unwrap();
        assert_eq!(
            std::fs::read(source_output.join("LODSettings/Tamriel.lod")).unwrap(),
            crate::output::lodsettings::encode((-96, -96), 256, 4, 32)
        );
        for (level, x, y) in [(4, -60, -44), (8, -64, -48), (16, -64, -48), (32, -64, -64)] {
            assert!(
                source_output
                    .join(format!(
                        "Meshes/Terrain/Tamriel/Tamriel.{level}.{x}.{y}.btr"
                    ))
                    .is_file()
            );
        }

        settings.global.southwest_cell = Some([-8, -8]);
        settings.global.stride = Some(64);
        let override_output = root.join("override_output");
        let override_paths = LodPaths {
            data_dirs: vec![root.clone()],
            output_dir: override_output.clone(),
            source_data_dir: Some(source_dir.clone()),
        };
        run_with_world(&world, &settings, &override_paths, &mut NullProgress).unwrap();
        assert_eq!(
            std::fs::read(override_output.join("LODSettings/Tamriel.lod")).unwrap(),
            crate::output::lodsettings::encode((-8, -8), 64, 4, 32)
        );

        settings.global.southwest_cell = None;
        settings.global.stride = None;
        settings.global.align = 4;
        let aligned_output = root.join("aligned_output");
        let aligned_paths = LodPaths {
            data_dirs: vec![root.clone()],
            output_dir: aligned_output.clone(),
            source_data_dir: Some(source_dir.clone()),
        };
        run_with_world(&world, &settings, &aligned_paths, &mut NullProgress).unwrap();
        assert_eq!(
            std::fs::read(aligned_output.join("LODSettings/Tamriel.lod")).unwrap(),
            crate::output::lodsettings::encode((-60, -44), 32, 4, 32)
        );

        settings.global.align = 0;
        settings.global.use_source_lodsettings = false;
        let disabled_output = root.join("disabled_output");
        let disabled_paths = LodPaths {
            data_dirs: vec![root.clone()],
            output_dir: disabled_output.clone(),
            source_data_dir: Some(source_dir),
        };
        run_with_world(&world, &settings, &disabled_paths, &mut NullProgress).unwrap();
        assert_eq!(
            std::fs::read(disabled_output.join("LODSettings/Tamriel.lod")).unwrap(),
            crate::output::lodsettings::encode((-57, -43), 32, 4, 32)
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_with_world_ignores_starfield_source_lodsettings() {
        let world = crate::input::WorldspaceInput::from_cells(
            "AkilaCity",
            vec![crate::input::CellInput {
                x: -57,
                y: -43,
                heights: vec![0.0; 33 * 33],
                vertex_colors: vec![[255, 255, 255]; 33 * 33],
                layers: Vec::new(),
                hidden_quadrants: [false; 4],
                water_height: f32::MIN,
            }],
        );
        let root = std::env::temp_dir().join(format!(
            "lodgen_starfield_source_window_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let source_dir = root.join("source");
        let settings_dir = source_dir.join("LODSettings");
        std::fs::create_dir_all(&settings_dir).unwrap();
        let mut starfield_lod = Vec::with_capacity(20);
        starfield_lod.extend_from_slice(&(-35i32).to_le_bytes());
        starfield_lod.extend_from_slice(&(-59i32).to_le_bytes());
        starfield_lod.extend_from_slice(&72i32.to_le_bytes());
        starfield_lod.extend_from_slice(&0i32.to_le_bytes());
        starfield_lod.extend_from_slice(&0i32.to_le_bytes());
        std::fs::write(settings_dir.join("AkilaCity.lod"), starfield_lod).unwrap();

        let mut settings = LodSettings::fo4_default();
        settings.global.generate_objects = false;
        let output = root.join("output");
        let paths = LodPaths {
            data_dirs: vec![root.clone()],
            output_dir: output.clone(),
            source_data_dir: Some(source_dir),
        };
        run_with_world(&world, &settings, &paths, &mut NullProgress).unwrap();
        assert_eq!(
            std::fs::read(output.join("LODSettings/AkilaCity.lod")).unwrap(),
            crate::output::lodsettings::encode((-57, -43), 32, 4, 32)
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_with_world_can_skip_terrain_phase() {
        let w = crate::input::WorldspaceInput::from_cells(
            "W",
            vec![crate::input::CellInput {
                x: 0,
                y: 0,
                heights: vec![0.0; 33 * 33],
                vertex_colors: vec![[255, 255, 255]; 33 * 33],
                layers: Vec::new(),
                hidden_quadrants: [false; 4],
                water_height: f32::MIN,
            }],
        );
        let mut s = LodSettings::fo4_default();
        s.global.generate_terrain = false;
        s.global.generate_objects = false;
        let out = std::env::temp_dir().join("lodgen_run_skip_terrain_test");
        std::fs::create_dir_all(&out).unwrap();
        let paths = LodPaths {
            data_dirs: vec![std::path::PathBuf::from(".")],
            output_dir: out,
            source_data_dir: None,
        };
        let mut p = NullProgress;
        let stats = run_with_world(&w, &s, &paths, &mut p).unwrap();
        assert_eq!(stats.btr, 0);
        assert!(!stats.lod_written);
    }
}
