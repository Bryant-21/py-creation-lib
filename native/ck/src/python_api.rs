use std::path::PathBuf;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyModule;

use crate::anim_text_data::emit::{AnimTextDataInputs, generate_anim_text_data_with_progress};
use crate::anim_text_data::race_decode::subgraph_inputs_from_plugin;

#[pyfunction]
fn ck_ping() -> u32 {
    1
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
    progress_callback=None
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
    });
    result.map_err(PyRuntimeError::new_err)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(ck_ping, m)?)?;
    m.add_function(wrap_pyfunction!(generate_anim_text_data_py, m)?)?;
    Ok(())
}
