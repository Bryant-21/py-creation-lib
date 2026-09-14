use std::path::PathBuf;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyModule;

use crate::anim_text_data::creature_closure::generate_creature_anim_text_closure;
use crate::anim_text_data::emit::{AnimTextDataInputs, generate_anim_text_data_with_progress};
use crate::anim_text_data::race_decode::subgraph_inputs_from_plugin;

#[pyfunction]
fn ck_ping() -> u32 {
    1
}

#[pyfunction(name = "ck_race_subgraphs")]
#[pyo3(signature = (plugin_path, game, form_id=None))]
fn race_subgraphs_py(py: Python<'_>, plugin_path: String, game: String, form_id: Option<u32>) -> PyResult<String> {
    py.detach(move || crate::anim_text_data::race_decode::inspect_race_subgraphs(
        &PathBuf::from(plugin_path), &game, form_id,
    )).map_err(PyRuntimeError::new_err)
}

#[pyfunction(name = "ck_subgraph_id")]
fn subgraph_id_py(behavior: &str, paths: Vec<String>) -> u64 {
    crate::anim_text_data::core::subgraph_id(behavior, &paths.iter().map(String::as_str).collect::<Vec<_>>())
}

#[pyfunction(name = "ck_generate_anim_text_data")]
#[pyo3(signature = (
    plugin_path,
    game,
    src_meshes_root,
    out_meshes_root,
    base_meshes_root=None,
    base_plugin_paths=Vec::new(),
    mod_prefix=None,
    progress_callback=None,
    workers=None
))]
#[allow(clippy::too_many_arguments)]
pub fn generate_anim_text_data_py(
    py: Python<'_>,
    plugin_path: &str,
    game: &str,
    src_meshes_root: &str,
    out_meshes_root: &str,
    base_meshes_root: Option<&str>,
    base_plugin_paths: Vec<String>,
    mod_prefix: Option<&str>,
    progress_callback: Option<Py<PyAny>>,
    workers: Option<usize>,
) -> PyResult<u32> {
    let plugin_path = PathBuf::from(plugin_path);
    let game = game.to_string();
    let src_meshes_root = PathBuf::from(src_meshes_root);
    let out_meshes_root = PathBuf::from(out_meshes_root);
    let base_meshes_root = base_meshes_root.map(PathBuf::from);
    let base_plugin_paths = base_plugin_paths
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let mod_prefix = mod_prefix.map(str::to_string);

    let result = py.detach(move || {
        let run = || {
            let decoded = subgraph_inputs_from_plugin(&plugin_path, &game, &base_plugin_paths)?;
            let inputs = AnimTextDataInputs::from(decoded);
            let mut progress = |message: &str| {
                if let Some(callback) = progress_callback.as_ref() {
                    Python::attach(|py| {
                        let _ = callback.call1(py, (message,));
                    });
                }
            };
            generate_anim_text_data_with_progress(
                &inputs,
                &src_meshes_root,
                &out_meshes_root,
                base_meshes_root.as_deref(),
                mod_prefix.as_deref(),
                &mut progress,
            )
            .map(|report| report.written)
        };
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers.filter(|workers| *workers > 0).unwrap_or(1))
            .build()
            .map_err(|error| format!("AnimTextData rayon pool: {error}"))?
            .install(run)
    });
    result.map_err(PyRuntimeError::new_err)
}

#[pyfunction(name = "ck_generate_creature_anim_text_closure")]
#[pyo3(signature = (contract_json, progress_callback=None, workers=None))]
pub fn generate_creature_anim_text_closure_py(
    py: Python<'_>,
    contract_json: &str,
    progress_callback: Option<Py<PyAny>>,
    workers: Option<usize>,
) -> PyResult<String> {
    let contract_json = contract_json.to_string();
    let result = py.detach(move || {
        let run = || {
            let mut progress = |message: &str| {
                if let Some(callback) = progress_callback.as_ref() {
                    Python::attach(|py| {
                        let _ = callback.call1(py, (message,));
                    });
                }
            };
            generate_creature_anim_text_closure(&contract_json, &mut progress)
        };
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers.filter(|workers| *workers > 0).unwrap_or(1))
            .build()
            .map_err(|error| format!("creature AnimText rayon pool: {error}"))?
            .install(run)
    });
    result.map_err(PyRuntimeError::new_err)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(ck_ping, m)?)?;
    m.add_function(wrap_pyfunction!(race_subgraphs_py, m)?)?;
    m.add_function(wrap_pyfunction!(subgraph_id_py, m)?)?;
    m.add_function(wrap_pyfunction!(generate_anim_text_data_py, m)?)?;
    m.add_function(wrap_pyfunction!(generate_creature_anim_text_closure_py, m)?)?;
    Ok(())
}
