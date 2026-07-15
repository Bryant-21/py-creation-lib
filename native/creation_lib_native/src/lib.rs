use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule};

fn register_submodule(
    py: Python<'_>,
    parent: &Bound<'_, PyModule>,
    name: &str,
    register: impl FnOnce(&Bound<'_, PyModule>) -> PyResult<()>,
) -> PyResult<()> {
    let submodule = PyModule::new(py, name)?;
    register(&submodule)?;
    parent.add_submodule(&submodule)?;

    let full_name = format!("creation_lib._native.{name}");
    let sys = py.import("sys")?;
    let modules = sys.getattr("modules")?;
    let sys_modules = modules.cast::<PyDict>()?;
    sys_modules.set_item(full_name, &submodule)?;
    Ok(())
}

#[pymodule]
fn _native(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    #[cfg(feature = "audio")]
    register_submodule(py, m, "audio_native", audio_native::register_module)?;
    #[cfg(feature = "palette")]
    register_submodule(py, m, "palette_native", palette_native::register_module)?;
    #[cfg(feature = "esp")]
    register_submodule(
        py,
        m,
        "esp_authoring_core",
        esp_authoring_core::register_module,
    )?;
    #[cfg(feature = "ck")]
    register_submodule(py, m, "ck_native", ck_native::register_module)?;
    #[cfg(feature = "bsarchive")]
    register_submodule(py, m, "bsarchive_native", bsarchive_native::register_module)?;
    #[cfg(feature = "directxtex")]
    register_submodule(
        py,
        m,
        "directxtex_native",
        directxtex_native::register_module,
    )?;
    #[cfg(feature = "fnv_script")]
    register_submodule(
        py,
        m,
        "fnv_script_native",
        fnv_script_native::register_module,
    )?;
    #[cfg(feature = "havok")]
    register_submodule(py, m, "havok_native", havok_native::register_module)?;
    #[cfg(feature = "materials")]
    register_submodule(py, m, "materials_native", materials_native::register_module)?;
    #[cfg(feature = "nif")]
    register_submodule(py, m, "nif_core_native", nif_core_native::register_module)?;
    #[cfg(feature = "db")]
    register_submodule(py, m, "db_native", db_native::register_module)?;
    #[cfg(feature = "papyrus_core")]
    register_submodule(py, m, "papyrus_core", papyrus_core::register_module)?;
    #[cfg(feature = "scientific")]
    register_submodule(
        py,
        m,
        "scientific_native",
        scientific_native::register_module,
    )?;
    #[cfg(feature = "swf")]
    register_submodule(py, m, "swf_native", swf_native::register_module)?;
    #[cfg(feature = "terrain")]
    register_submodule(py, m, "terrain_native", terrain_native::register_module)?;
    #[cfg(feature = "lodgen")]
    register_submodule(py, m, "lodgen_native", lodgen_native::register_module)?;
    #[cfg(feature = "world_renderer")]
    register_submodule(
        py,
        m,
        "world_renderer_native",
        world_renderer_native::register_module,
    )?;
    Ok(())
}
