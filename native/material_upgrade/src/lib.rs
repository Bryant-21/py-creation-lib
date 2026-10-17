//! FO4 material/texture-set upgrade to BGSM v22+ (PBR): `_s` + BGSM v2 ->
//! `_r`/`_l` + BGSM v22, NIF texture-set slots 8/9, and a staged raw-terrain
//! layer bank. Python, the CLI and the UI only parse options and show reports.

use pyo3::prelude::*;
use serde::Serialize;

pub mod bgsm_v22;
pub mod classify;
pub mod execute;
pub mod kernels;
pub mod options;
pub mod plan;
pub mod sources;
pub mod terrain;
pub mod terrain_btd4;

use crate::execute::{REPORT_PATH, RunResult};
use crate::options::UpgradeOptions;
use crate::plan::{Plan, Summary, Unit};
use crate::sources::AssetSources;

#[derive(Serialize)]
pub struct Report<'a> {
    pub dry_run: bool,
    pub output: String,
    pub summary: Summary,
    pub units: &'a [Unit],
    pub texture_outputs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<&'a RunResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terrain_sidecars: Option<&'a terrain_btd4::SidecarRun>,
}

pub fn report<'a>(
    plan: &'a Plan,
    opts: &UpgradeOptions,
    result: Option<&'a RunResult>,
    terrain_sidecars: Option<&'a terrain_btd4::SidecarRun>,
) -> Report<'a> {
    Report {
        dry_run: result.is_none(),
        output: opts.output.display().to_string(),
        summary: plan.summary(),
        units: &plan.units,
        texture_outputs: plan.textures.iter().flat_map(|t| t.outputs()).collect(),
        result,
        terrain_sidecars,
    }
}

pub fn plan_upgrade(opts: &UpgradeOptions) -> Result<(Plan, AssetSources), String> {
    let all: Vec<_> = opts.base_sources.iter().chain(&opts.sources).chain(&opts.overlay_sources).cloned().collect();
    let sources = AssetSources::open(&all)?;
    let plan = plan::build_plan(opts, &sources)?;
    Ok((plan, sources))
}

/// Plan, execute and write `materialupgrade/upgrade_report.json` into the output mod.
pub fn run_upgrade(
    opts: &UpgradeOptions,
    progress: &mut dyn FnMut(usize, usize, &str) -> bool,
) -> Result<String, String> {
    if opts.output.as_os_str().is_empty() {
        return Err("an output mod folder is required".into());
    }
    let (plan, sources) = plan_upgrade(opts)?;
    let result = execute::execute(&plan, opts, &sources, progress);
    let sidecars = (opts.include_terrain && opts.terrain_sidecars && !result.cancelled)
        .then(|| terrain_btd4::write_sidecars(&plan, opts));
    let text = serde_json::to_string_pretty(&report(&plan, opts, Some(&result), sidecars.as_ref()))
        .map_err(|err| err.to_string())?;
    if !result.cancelled {
        let path = opts.output.join(REPORT_PATH);
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|err| err.to_string())?;
        std::fs::write(&path, &text).map_err(|err| format!("{}: {err}", path.display()))?;
    }
    Ok(text)
}

fn to_py_err(err: String) -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(err)
}

/// Dry run: returns the JSON report (summary, per-unit decisions, outputs).
#[pyfunction]
fn plan_upgrade_v22(py: Python<'_>, options_json: &str) -> PyResult<String> {
    let options_json = options_json.to_owned();
    py.detach(move || {
        let opts = UpgradeOptions::from_json(&options_json)?;
        let (plan, _) = plan_upgrade(&opts)?;
        serde_json::to_string_pretty(&report(&plan, &opts, None, None)).map_err(|err| err.to_string())
    })
    .map_err(to_py_err)
}

/// Upgrade into the output mod. `progress(done, total, message)` may return False to cancel.
#[pyfunction]
#[pyo3(signature = (options_json, progress=None))]
fn run_upgrade_v22(py: Python<'_>, options_json: &str, progress: Option<Py<PyAny>>) -> PyResult<String> {
    let options_json = options_json.to_owned();
    py.detach(move || {
        let opts = UpgradeOptions::from_json(&options_json)?;
        let mut callback = |done: usize, total: usize, message: &str| -> bool {
            let Some(progress) = progress.as_ref() else { return true };
            Python::attach(|py| match progress.call1(py, (done, total, message)) {
                Ok(value) => value.is_none(py) || value.extract::<bool>(py).unwrap_or(true),
                Err(_) => true,
            })
        };
        run_upgrade(&opts, &mut callback)
    })
    .map_err(to_py_err)
}

/// Upgrade-vs-replace rule helper for callers that already have a plan report:
/// given the replace set, returns the material/texture-set keys still upgraded.
#[pyfunction]
fn select_upgrade_keys(py: Python<'_>, options_json: &str) -> PyResult<Vec<String>> {
    let options_json = options_json.to_owned();
    py.detach(move || {
        let opts = UpgradeOptions::from_json(&options_json)?;
        let (plan, _) = plan_upgrade(&opts)?;
        Ok(plan.units.iter().filter(|u| u.is_upgrade()).map(|u| u.key.clone()).collect())
    })
    .map_err(to_py_err)
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(plan_upgrade_v22, m)?)?;
    m.add_function(wrap_pyfunction!(run_upgrade_v22, m)?)?;
    m.add_function(wrap_pyfunction!(select_upgrade_keys, m)?)?;
    Ok(())
}
