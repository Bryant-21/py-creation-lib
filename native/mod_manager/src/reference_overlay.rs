use crate::{check_cancel, encoding, invalid, plugin_inventory::value, python_io::with_context};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
use serde_json::json;
#[pyfunction]
fn overlay_capture(
    library: &Bound<'_, PyAny>,
    profile: &str,
    base_id: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let snapshot = library.call_method1("profile_snapshot", (profile,))?;
    let mut selected = None;
    for row in snapshot.get_item(1)?.try_iter()? {
        let row = row?;
        if row.getattr("id")?.extract::<String>()? == base_id {
            selected = Some(row);
            break;
        }
    }
    let row = selected.ok_or_else(|| {
        invalid("An additive update requires an available referenced mod in this profile.")
    })?;
    if !matches!(
        row.getattr("kind")?.extract::<String>()?.as_str(),
        "external" | "reference"
    ) || !row.getattr("source")?.is_truthy()?
    {
        return Err(invalid(
            "An additive update requires an available referenced mod in this profile.",
        ));
    }
    check_cancel(Some(cancel))?;
    let package_module = py.import("creation_lib.mod_manager.package")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("inspect_installer", false)?;
    let package = package_module.getattr("Package")?.call(
        (
            row.getattr("source")?,
            cancel,
            package_module.getattr("OWNED_LIMITS")?,
        ),
        Some(&kwargs),
    )?;
    let context = py
        .import("contextlib")?
        .getattr("closing")?
        .call1((package,))?;
    let token = with_context(&context, |package| {
        let sorted = py
            .import("builtins")?
            .getattr("sorted")?
            .call1((package.getattr("_signatures")?.call_method0("items")?,))?;
        Ok(encoding::digest(
            &json!([
                value(&row.getattr("id")?)?,
                value(&row.getattr("source")?)?,
                value(&row.getattr("priority")?)?,
                value(&row.getattr("enabled")?)?,
                value(&sorted)?
            ]),
            true,
            false,
        ))
    })?;
    let target = if library.hasattr("target_id")? {
        library.getattr("target_id")?.extract::<String>()?
    } else {
        String::new()
    };
    Ok(py
        .import("creation_lib.mod_manager.reference_overlay")?
        .getattr("Review")?
        .call1((
            library.getattr("library_id")?,
            profile,
            target,
            row.getattr("id")?,
            row.getattr("name")?,
            row.getattr("source")?,
            token,
        ))?
        .unbind())
}
#[pyfunction]
fn overlay_validate(
    library: &Bound<'_, PyAny>,
    profile: &str,
    review: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<()> {
    if !overlay_capture(
        library,
        profile,
        &review.getattr("base_id")?.extract::<String>()?,
        cancel,
    )?
    .bind(library.py())
    .eq(review)?
    {
        return Err(invalid(
            "Referenced update provider or profile changed. Reopen and review the additive update.",
        ));
    }
    Ok(())
}
#[pyclass]
struct ValidateOverlay {
    library: Py<PyAny>,
    review: Py<PyAny>,
    cancel: Py<PyAny>,
    validation: Py<PyAny>,
}
#[pymethods]
impl ValidateOverlay {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        let review = self.review.bind(py);
        overlay_validate(
            self.library.bind(py),
            &review.getattr("profile_id")?.extract::<String>()?,
            review,
            self.cancel.bind(py),
        )?;
        if self.validation.bind(py).is_truthy()? {
            self.validation.bind(py).call0()?;
        }
        Ok(())
    }
}
#[pyclass]
struct AttachOverlay {
    review: Py<PyAny>,
}
#[pymethods]
impl AttachOverlay {
    fn __call__(&self, db: &Bound<'_, PyAny>, mod_id: &str) -> PyResult<()> {
        let py = db.py();
        let review = self.review.bind(py);
        let native = db.getattr("native")?;
        let native = native.extract::<PyRef<'_, crate::database::Database>>()?;
        let connection = native.connection()?;
        let profile: String = review.getattr("profile_id")?.extract()?;
        let target: String = review.getattr("target_id")?.extract()?;
        let base_id: String = review.getattr("base_id")?.extract()?;
        let source: String = review.getattr("source")?.extract()?;
        connection
            .execute(
                "INSERT INTO reference_overlays VALUES (?,?,?,?,?)",
                rusqlite::params![mod_id, profile, target, base_id, source],
            )
            .map_err(crate::database::error)?;
        let notes = format!(
            "Additive update above {}. Keeps unmatched source files. Follows the reference's enabled state only in the profile and target used for installation. Review this patch's priority after reorganizing mods.",
            review.getattr("name")?.extract::<String>()?
        );
        connection
            .execute(
                "UPDATE mods SET notes=? WHERE id=?",
                rusqlite::params![notes, mod_id],
            )
            .map_err(crate::database::error)?;
        let mut order: Vec<String> = crate::database::records(
            &connection,
            "SELECT mod_id FROM entries WHERE profile_id=? AND mod_id!=? ORDER BY priority,mod_id",
            rusqlite::params![profile, mod_id],
        )?
        .iter()
        .map(|v| v["mod_id"].as_str().unwrap_or("").to_owned())
        .collect();
        let position = order
            .iter()
            .position(|id| id == &base_id)
            .ok_or_else(|| invalid("Referenced update provider was removed before publication."))?;
        order.insert(position + 1, mod_id.to_owned());
        for (i, id) in order.iter().enumerate() {
            connection
                .execute(
                    "UPDATE entries SET priority=? WHERE profile_id=? AND mod_id=?",
                    rusqlite::params![i as i64, profile, id],
                )
                .map_err(crate::database::error)?;
        }
        Ok(())
    }
}
#[pyfunction]
fn overlay_install(
    library: &Bound<'_, PyAny>,
    review: &Bound<'_, PyAny>,
    package: &Bound<'_, PyAny>,
    plan: &Bound<'_, PyAny>,
    name: &str,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
    nexus: &Bound<'_, PyAny>,
    enabled: bool,
    validation: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let base = if library.hasattr("base")? {
        library.getattr("base")?
    } else {
        library.clone()
    };
    base.call_method0("_runtime_write_guard")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("nexus", nexus)?;
    kwargs.set_item("enabled", enabled)?;
    kwargs.set_item(
        "validate_before_commit",
        Py::new(
            py,
            ValidateOverlay {
                library: library.clone().unbind(),
                review: review.clone().unbind(),
                cancel: cancel.clone().unbind(),
                validation: validation.clone().unbind(),
            },
        )?,
    )?;
    kwargs.set_item(
        "attach_metadata",
        Py::new(
            py,
            AttachOverlay {
                review: review.clone().unbind(),
            },
        )?,
    )?;
    Ok(base
        .getattr("owned")?
        .call_method(
            "install",
            (
                package,
                plan,
                name,
                review.getattr("profile_id")?,
                cancel,
                progress,
            ),
            Some(&kwargs),
        )?
        .unbind())
}
#[pyfunction]
fn overlay_restore_links(db: &Bound<'_, PyAny>, snapshot: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = db.py();
    let data = value(snapshot)?;
    let fields = ["overlay_id", "profile_id", "target_id", "base_id", "source"];
    if let Some(links) = data.get("overlays").and_then(|v| v.as_array()) {
        for link in links {
            if !link.as_object().is_some_and(|v| {
                v.len() == fields.len() && fields.iter().all(|k| v.contains_key(*k))
            }) || link["overlay_id"] != data["mod"]["id"]
            {
                return Err(invalid("Recovery additive-update association is invalid."));
            }
            let values = fields
                .iter()
                .map(|k| crate::plugin_inventory::object(py, &link[*k]))
                .collect::<PyResult<Vec<_>>>()?;
            db.call_method1(
                "execute",
                (
                    "INSERT OR REPLACE INTO reference_overlays VALUES (?,?,?,?,?)",
                    PyTuple::new(py, values)?,
                ),
            )?;
        }
    }
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(overlay_capture, m)?)?;
    m.add_function(wrap_pyfunction!(overlay_validate, m)?)?;
    m.add_function(wrap_pyfunction!(overlay_install, m)?)?;
    m.add_function(wrap_pyfunction!(overlay_restore_links, m)?)?;
    Ok(())
}
