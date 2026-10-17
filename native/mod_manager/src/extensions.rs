use crate::{check_cancel, invalid, saves::fold};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
use std::collections::HashSet;
fn module<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyModule>> {
    py.import(format!("creation_lib.mod_manager.{name}"))
}
fn text(value: &Bound<'_, PyAny>) -> Option<String> {
    value.extract().ok()
}
fn valid_id(value: &str, limit: usize, punctuation: bool) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && value.as_bytes()[0].is_ascii_lowercase()
        && value.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || (punctuation && b"_.-".contains(&c))
        })
}
#[pyfunction]
fn extension_register(registry: &Bound<'_, PyAny>, adapter: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = registry.py();
    if !adapter.is_instance(&module(py, "extensions")?.getattr("Adapter")?)?
        || !adapter.getattr("api_version")?.eq(1)?
    {
        return Err(invalid("Metadata adapter API version is unsupported."));
    }
    let id = text(&adapter.getattr("id")?).filter(|s| valid_id(s, 64, true));
    let kind = text(&adapter.getattr("kind")?);
    if id.is_none() || !matches!(kind.as_deref(), Some("save" | "archive")) {
        return Err(invalid("Metadata adapter identity or kind is invalid."));
    }
    let game = text(&adapter.getattr("game")?).filter(|s| valid_id(s, 32, false));
    if game.is_none() || !adapter.getattr("callback")?.is_callable() {
        return Err(invalid(
            "Metadata adapter needs a canonical game identity and callable.",
        ));
    }
    let name = text(&adapter.getattr("name")?);
    let summary = text(&adapter.getattr("summary")?);
    if name
        .as_ref()
        .is_none_or(|s| s.is_empty() || s.chars().count() > 128)
        || summary.as_ref().is_none_or(|s| s.chars().count() > 1024)
    {
        return Err(invalid("Metadata adapter display text exceeds its bounds."));
    }
    let adapters = registry.getattr("_adapters")?;
    let identity = (kind.unwrap(), game.unwrap());
    let mut duplicate = adapters.contains(&identity)?;
    for item in adapters.call_method0("values")?.try_iter()? {
        duplicate |= item?.getattr("id")?.extract::<String>()? == id.as_ref().unwrap().as_str();
    }
    if duplicate {
        return Err(invalid(
            "An adapter for this game/kind or identity is already registered.",
        ));
    }
    if adapters.len()? >= 64 {
        return Err(invalid(
            "Metadata adapter registry exceeds its 64-adapter limit.",
        ));
    }
    adapters.set_item(identity, adapter)
}
#[pyfunction]
fn extension_adapter(registry: &Bound<'_, PyAny>, kind: &str, game: &str) -> PyResult<Py<PyAny>> {
    let canonical = if game == "fallout4" { "fo4" } else { game };
    let adapters = registry.getattr("_adapters")?;
    if !adapters.contains((kind, canonical))? {
        return Err(invalid(format!(
            "No registered {kind} metadata adapter for {canonical}."
        )));
    }
    Ok(adapters.get_item((kind, canonical))?.unbind())
}
#[pyfunction]
fn extension_inspect_save(
    registry: &Bound<'_, PyAny>,
    game: &str,
    save: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = registry.py();
    check_cancel(Some(cancel))?;
    let result = extension_adapter(registry, "save", game)?
        .bind(py)
        .getattr("callback")?
        .call1((save, cancel))?;
    check_cancel(Some(cancel))?;
    let inspection = module(py, "save_inspection")?;
    if !result.is_instance(&inspection.getattr("Inspection")?)?
        || !result.getattr("path")?.eq(save.getattr("path")?)?
    {
        return Err(invalid(
            "Save adapter returned an invalid result or a different source.",
        ));
    }
    if result.getattr("regular")?.len()? > 255
        || result.getattr("light")?.len()? > 4096
        || result.getattr("notices")?.len()? > 64
    {
        return Err(invalid("Save adapter result exceeds metadata bounds."));
    }
    let mut names = HashSet::new();
    for field in ["regular", "light"] {
        for name in result.getattr(field)?.try_iter()? {
            let name = name?.extract::<String>()?;
            module(py, "plugin_state")?.call_method1("plugin_name", (&name,))?;
            if !names.insert(fold(&name)) {
                return Err(invalid("Save adapter returned ambiguous plugin names."));
            }
        }
    }
    inspection.call_method1("validate", (&result, cancel))?;
    let stamp = result.getattr("stamp")?;
    let snapshot = PyTuple::new(py, [save.getattr("size")?, save.getattr("modified")?])?;
    let tail = PyTuple::new(py, [stamp.get_item(2)?, stamp.get_item(3)?])?;
    let identity = save.getattr("identity")?;
    if !tail.eq(snapshot)?
        || (!identity.is_none()
            && !PyTuple::new(py, [stamp.get_item(0)?, stamp.get_item(1)?])?.eq(identity)?)
    {
        return Err(invalid(
            "Save adapter result does not match the inventory snapshot.",
        ));
    }
    Ok(result.unbind())
}
#[pyfunction]
fn extension_resolve_archive(
    registry: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    loose: &Bound<'_, PyAny>,
    archives: &Bound<'_, PyAny>,
    settings: &Bound<'_, PyAny>,
    game: &str,
) -> PyResult<Py<PyAny>> {
    let py = registry.py();
    let kwargs = PyDict::new(py);
    kwargs.set_item("game", game)?;
    let result = extension_adapter(registry, "archive", game)?
        .bind(py)
        .getattr("callback")?
        .call((path, loose, archives, settings), Some(&kwargs))?;
    if !result.is_instance(&module(py, "combined_data")?.getattr("Entry")?)?
        || !result.getattr("path")?.eq(path)?
        || !result.getattr("loose")?.eq(loose)?
        || !result.getattr("archives")?.eq(archives)?
    {
        return Err(invalid("Archive adapter returned mismatched provenance."));
    }
    let winner = result.getattr("winner")?;
    if !winner.is_none() && !result.getattr("active")?.contains(&winner)? {
        return Err(invalid(
            "Archive adapter selected an inactive or unknown winner.",
        ));
    }
    if text(&result.getattr("reason")?).is_none_or(|s| s.chars().count() > 4096) {
        return Err(invalid("Archive adapter explanation exceeds its bound."));
    }
    Ok(result.unbind())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(extension_register, m)?)?;
    m.add_function(wrap_pyfunction!(extension_adapter, m)?)?;
    m.add_function(wrap_pyfunction!(extension_inspect_save, m)?)?;
    m.add_function(wrap_pyfunction!(extension_resolve_archive, m)?)?;
    Ok(())
}
