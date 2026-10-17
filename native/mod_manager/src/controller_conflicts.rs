use crate::controller::callback;
use pyo3::{prelude::*, types::PyDict};
fn now(py: Python<'_>) -> PyResult<f64> {
    py.import("time")?.call_method0("monotonic")?.extract()
}
#[pyfunction]
fn controller_conflict_index(manager: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let value = manager.getattr("_conflict_index")?;
    let files = manager.getattr("file_index")?;
    if value.is_truthy()?
        && value.getattr("loose")?.is(&files)
        && manager
            .getattr("_conflict_rows")?
            .is(&manager.getattr("rows")?)
    {
        Ok(value.unbind())
    } else {
        Ok(files.unbind())
    }
}
#[pyfunction]
fn controller_refresh_conflicts(manager: &Bound<'_, PyAny>, delay: f64) -> PyResult<()> {
    let py = manager.py();
    manager.getattr("_conflict_cancel")?.call_method0("set")?;
    manager.setattr(
        "_conflict_generation",
        manager.getattr("_conflict_generation")?.extract::<u64>()? + 1,
    )?;
    for key in [
        "_conflict_index",
        "file_index",
        "archive_index",
        "combined_index",
    ] {
        manager.setattr(key, py.None())?;
    }
    manager.setattr("conflict_scan_error", "")?;
    manager.setattr(
        "conflict_scan_progress",
        (py.None(), "Waiting for local provider scan"),
    )?;
    let ready =
        manager.getattr("library")?.is_truthy()? && manager.getattr("profile_id")?.is_truthy()?;
    manager.setattr("conflict_scan_state", if ready { "queued" } else { "idle" })?;
    manager.setattr(
        "_conflict_due",
        if ready {
            now(py)? + delay
        } else {
            f64::INFINITY
        },
    )?;
    Ok(())
}
#[pyfunction]
fn controller_accept_conflicts(
    manager: &Bound<'_, PyAny>,
    generation: &Bound<'_, PyAny>,
    result: &Bound<'_, PyAny>,
    final_result: bool,
) -> PyResult<()> {
    let py = manager.py();
    if !generation.eq(manager.getattr("_conflict_generation")?)? {
        return Ok(());
    }
    let status = result.get_item(0)?;
    let combined = result.get_item(1)?;
    if !py
        .import("creation_lib.mod_manager.local_status")?
        .call_method1("matches", (&status, manager.getattr("rows")?))?
        .is_truthy()?
    {
        manager.call_method0("refresh_conflicts")?;
        return Ok(());
    }
    manager.setattr("_conflict_index", &status)?;
    manager.setattr("file_index", status.getattr("loose")?)?;
    manager.setattr("_conflict_rows", manager.getattr("rows")?)?;
    if combined.is_truthy()? {
        manager.setattr("combined_index", &combined)?;
        manager.setattr("archive_index", combined.getattr("archives")?)?;
    }
    if final_result {
        manager.setattr(
            "conflict_scan_state",
            if status.getattr("partial")?.is_truthy()? {
                "partial"
            } else {
                "ready"
            },
        )?;
        manager.setattr(
            "conflict_scan_error",
            "\n".into_pyobject(py)?
                .call_method1("join", (status.getattr("issues")?,))?,
        )?;
        manager.setattr("_conflict_due", now(py)? + 60.0)?;
    }
    for key in ["_status_key", "_filter_key"] {
        manager.setattr(key, py.None())?;
    }
    Ok(())
}
#[pyfunction]
fn controller_poll_conflicts(manager: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = manager.py();
    loop {
        let event = match manager
            .getattr("_conflict_events")?
            .call_method0("get_nowait")
        {
            Ok(v) => v,
            Err(e) if e.matches(py, &py.import("queue")?.getattr("Empty")?)? => break,
            Err(e) => return Err(e),
        };
        manager.call_method1(
            "_accept_conflicts",
            (event.get_item(0)?, event.get_item(1)?),
        )?;
    }
    let future = manager.getattr("_conflict_future")?;
    if future.is_truthy()? && future.call_method0("done")?.is_truthy()? {
        let generation = manager.getattr("_conflict_running_generation")?;
        manager.setattr("_conflict_future", py.None())?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("final", true)?;
        let result = future.call_method0("result").and_then(|v| {
            manager.call_method("_accept_conflicts", (&generation, v), Some(&kwargs))
        });
        if let Err(e) = result {
            if !e.matches(
                py,
                &py.import("creation_lib.mod_manager.paths")?
                    .getattr("Cancelled")?,
            )? {
                if !e.is_instance_of::<pyo3::exceptions::PyException>(py) {
                    return Err(e);
                }
                if generation.eq(manager.getattr("_conflict_generation")?)? {
                    manager.setattr("conflict_scan_state", "error")?;
                    manager.setattr("conflict_scan_error", e.value(py).str()?)?;
                    manager.setattr("_conflict_due", now(py)? + 60.0)?;
                }
            }
        }
    }
    if !manager.getattr("_conflict_future")?.is_none()
        || !manager.getattr("library")?.is_truthy()?
        || !manager.getattr("profile_id")?.is_truthy()?
        || now(py)? < manager.getattr("_conflict_due")?.extract::<f64>()?
        || manager.getattr("runtime_active")?.is_truthy()?
        || manager.getattr("busy")?.is_truthy()?
    {
        return Ok(());
    }
    let generation: u64 = manager.getattr("_conflict_generation")?.extract()?;
    let library = manager.getattr("context_library")?;
    let profile = manager.getattr("profile_id")?;
    let cancel = py.import("threading")?.call_method0("Event")?;
    manager.setattr("_conflict_cancel", &cancel)?;
    manager.setattr("_conflict_running_generation", generation)?;
    manager.setattr("conflict_scan_state", "scanning")?;
    manager.setattr("_conflict_due", f64::INFINITY)?;
    let owner = manager.clone().unbind();
    let progress = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        if manager.getattr("_conflict_generation")?.extract::<u64>()? == generation {
            manager.setattr(
                "conflict_scan_progress",
                (args.get_item(0)?, args.get_item(1)?),
            )?;
        }
        Ok(py.None())
    })?;
    let owner = manager.clone().unbind();
    let publish = callback(py, move |py, args, _| {
        owner.bind(py).getattr("_conflict_events")?.call_method1(
            "put",
            ((generation, (args.get_item(0)?, args.get_item(1)?)),),
        )?;
        Ok(py.None())
    })?;
    let scan = py
        .import("creation_lib.mod_manager.local_status")?
        .getattr("scan")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item(
        "resolver",
        manager.getattr("extensions")?.getattr("resolve_archive")?,
    )?;
    let future = manager.getattr("_conflict_executor")?.call_method(
        "submit",
        (scan, library, profile, cancel, progress, publish),
        Some(&kwargs),
    )?;
    manager.setattr("_conflict_future", future)?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(controller_conflict_index, m)?)?;
    m.add_function(wrap_pyfunction!(controller_refresh_conflicts, m)?)?;
    m.add_function(wrap_pyfunction!(controller_accept_conflicts, m)?)?;
    m.add_function(wrap_pyfunction!(controller_poll_conflicts, m)?)?;
    Ok(())
}
