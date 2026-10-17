use crate::invalid;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};

type Body = dyn for<'py> Fn(
        Python<'py>,
        &Bound<'py, PyTuple>,
        Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Py<PyAny>>
    + Send
    + Sync;
#[pyclass]
struct Callback {
    body: Box<Body>,
}
#[pymethods]
impl Callback {
    #[pyo3(signature=(*args, **kwargs))]
    fn __call__(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        (self.body)(py, args, kwargs)
    }
}
pub fn callback<F>(py: Python<'_>, body: F) -> PyResult<Py<PyAny>>
where
    F: for<'py> Fn(
            Python<'py>,
            &Bound<'py, PyTuple>,
            Option<&Bound<'py, PyDict>>,
        ) -> PyResult<Py<PyAny>>
        + Send
        + Sync
        + 'static,
{
    let native = Py::new(
        py,
        Callback {
            body: Box::new(body),
        },
    )?;
    Ok(py
        .import("functools")?
        .getattr("partial")?
        .call1((
            py.import("creation_lib.mod_manager.backend")?
                .getattr("invoke")?,
            native,
        ))?
        .unbind())
}
fn diagnostics(manager: &Bound<'_, PyAny>) -> PyResult<()> {
    manager
        .py()
        .import("creation_lib.mod_manager.diagnostics")?
        .call_method1("emit", (manager,))?;
    Ok(())
}

pub fn submit_call(
    manager: &Bound<'_, PyAny>,
    label: &str,
    function: &Bound<'_, PyAny>,
    arguments: &Bound<'_, PyTuple>,
    options: &Bound<'_, PyDict>,
    cancel: bool,
    progress: bool,
    complete: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = manager.py();
    let function = function.clone().unbind();
    let arguments = arguments.clone().unbind();
    let options = options.copy()?.unbind();
    let work = callback(py, move |py, args, _| {
        let options = options.bind(py).copy()?;
        if cancel {
            options.set_item("cancel", args.get_item(0)?)?;
        }
        if progress {
            options.set_item("progress", args.get_item(1)?)?;
        }
        Ok(function
            .bind(py)
            .call(arguments.bind(py), Some(&options))?
            .unbind())
    })?;
    manager.call_method1("submit", (label, work, complete))?;
    Ok(())
}
#[pyfunction]
fn controller_submit(
    manager: &Bound<'_, PyAny>,
    label: &str,
    work: &Bound<'_, PyAny>,
    complete: &Bound<'_, PyAny>,
    allow_runtime: bool,
) -> PyResult<()> {
    let py = manager.py();
    if manager.getattr("busy")?.is_truthy()? {
        return Err(invalid("Wait for the current operation to finish."));
    }
    if manager.getattr("runtime_active")?.is_truthy()? && !allow_runtime {
        return Err(invalid(
            "Close the running native runtime before changing this library or starting another operation.",
        ));
    }
    manager.setattr("error", "")?;
    manager.setattr("status", label)?;
    manager.setattr("progress", (py.None(), ""))?;
    manager.setattr("job_state", "running")?;
    let cancel = py.import("threading")?.call_method0("Event")?;
    manager.setattr("_cancel", &cancel)?;
    manager.setattr("_complete", complete)?;
    manager.setattr(
        "_cancel_status",
        "Cancelling; waiting for the operation to roll back.",
    )?;
    manager.setattr(
        "_future",
        manager
            .getattr("_executor")?
            .call_method1("submit", (work, cancel, manager.getattr("_report")?))?,
    )?;
    diagnostics(manager)
}
fn matches(py: Python<'_>, error: &PyErr, module: &str, name: &str) -> PyResult<bool> {
    Ok(error.matches(py, &py.import(module)?.getattr(name)?)?)
}
fn download_records(manager: &Bound<'_, PyAny>) -> PyResult<()> {
    manager.setattr(
        "download_jobs",
        manager
            .py()
            .import("creation_lib.mod_manager.download_queue")?
            .getattr("DownloadQueue")?
            .call1((manager.getattr("library")?,))?
            .call_method0("records")?,
    )
}
#[pyfunction]
fn controller_poll(manager: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = manager.py();
    diagnostics(manager)?;
    manager.call_method0("poll_runtime")?;
    manager.call_method0("_poll_conflicts")?;
    manager
        .getattr("metadata_observer")?
        .call_method1("poll", (manager,))?;
    loop {
        let event = match manager
            .getattr("_download_events")?
            .call_method0("get_nowait")
        {
            Ok(event) => event,
            Err(e) if matches(py, &e, "queue", "Empty")? => break,
            Err(e) => return Err(e),
        };
        let jobs = PyList::empty(py);
        let mut found = false;
        for job in manager.getattr("download_jobs")?.try_iter()? {
            let job = job?;
            if job.get_item("id")?.eq(event.get_item("id")?)? {
                jobs.append(&event)?;
                found = true;
            } else {
                jobs.append(job)?;
            }
        }
        if !found {
            jobs.insert(0, event)?;
        }
        manager.setattr("download_jobs", jobs)?;
    }
    manager.call_method0("_poll_downloads")?;
    if !manager.getattr("busy")?.is_truthy()?
        || !manager
            .getattr("_future")?
            .call_method0("done")?
            .is_truthy()?
    {
        return Ok(());
    }
    let future = manager.getattr("_future")?;
    let complete = manager.getattr("_complete")?;
    let update = manager.getattr("_update_pending")?.is_truthy()?;
    for key in ["_future", "_complete"] {
        manager.setattr(key, py.None())?;
    }
    manager.setattr("_update_pending", false)?;
    let result = future
        .call_method0("result")
        .and_then(|v| complete.call1((v,)));
    match result {
        Ok(_) => manager.setattr("job_state", "completed")?,
        Err(e)
            if matches(
                py,
                &e,
                "creation_lib.mod_manager.transactions",
                "CommittedTransaction",
            )? =>
        {
            manager.setattr("job_state", "committed")?;
            manager.setattr(
                "status",
                "Change committed; reopening the library will complete recovery.",
            )?;
            manager.setattr("error", e.value(py).str()?)?;
            for key in ["lifecycle_review", "restore_preview", "migration_review"] {
                manager.setattr(key, py.None())?;
            }
            manager.call_method0("discard_install")?;
            let kind = e.value(py).getattr("kind")?;
            if !kind.eq("backup-prune")? {
                manager.setattr(
                    "selected_id",
                    if !kind.eq("uninstall")? {
                        e.value(py).getattr("mod_id")?
                    } else {
                        "".into_pyobject(py)?.into_any()
                    },
                )?;
            }
            if let Err(e) = manager.call_method0("refresh") {
                if !e.is_instance_of::<pyo3::exceptions::PyOSError>(py)
                    && !matches(py, &e, "creation_lib.mod_manager.paths", "ManagerError")?
                {
                    return Err(e);
                }
                let message: String = manager.getattr("error")?.extract()?;
                manager.setattr(
                    "error",
                    message + " Metadata refresh also requires reopening the library.",
                )?;
            }
        }
        Err(e) if matches(py, &e, "creation_lib.mod_manager.paths", "Cancelled")? => {
            manager.setattr("status", e.value(py).str()?)?;
            manager.setattr("job_state", "cancelled")?;
            if matches(
                py,
                &e,
                "creation_lib.mod_manager.mo2_migration",
                "MigrationStopped",
            )? {
                manager.call_method1("_accept_migration", (e.value(py).getattr("result")?,))?;
                manager.setattr("status", e.value(py).str()?)?;
            }
            if matches(
                py,
                &e,
                "creation_lib.mod_manager.migration",
                "MigrationCancelled",
            )? {
                manager.setattr("migration_review", py.None())?;
                manager.call_method0("refresh")?;
            }
            if matches(
                py,
                &e,
                "creation_lib.mod_manager.download_queue",
                "DownloadPaused",
            )? {
                download_records(manager)?;
            }
        }
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => {
            manager.setattr("error", e.value(py).str()?)?;
            manager.setattr("status", "Operation failed; review the error below.")?;
            manager.setattr("job_state", "failed")?;
            if matches(
                py,
                &e,
                "creation_lib.mod_manager.migration",
                "MigrationInterrupted",
            )? {
                manager.setattr("migration_review", py.None())?;
                manager.call_method0("refresh")?;
            }
            if matches(
                py,
                &e,
                "creation_lib.mod_manager.mo2_migration",
                "MigrationFailure",
            )? {
                manager.call_method1("_accept_migration", (e.value(py).getattr("result")?,))?;
            }
            if matches(
                py,
                &e,
                "creation_lib.mod_manager.download_queue",
                "DownloadFailure",
            )? {
                download_records(manager)?;
            }
            if update {
                let failures = manager.getattr("_update_failures")?.extract::<u64>()? + 1;
                manager.setattr("_update_failures", failures)?;
                let retry: f64 = py
                    .import("builtins")?
                    .call_method1("getattr", (e.value(py), "retry_after", 3600.0))?
                    .extract()?;
                let backoff = retry.max((3600.0 * 2_f64.powi(failures.min(5) as i32)).min(86400.0));
                let next = py
                    .import("time")?
                    .call_method0("monotonic")?
                    .extract::<f64>()?
                    + backoff;
                manager.setattr("_next_update", next)?;
                manager.setattr("_rate_blocked_until", next)?;
                crate::update_schedule::failed(manager, backoff)?;
            }
        }
        Err(e) => return Err(e),
    }
    diagnostics(manager)
}
#[pyfunction]
fn controller_cancel(manager: &Bound<'_, PyAny>) -> PyResult<()> {
    manager.getattr("_cancel")?.call_method0("set")?;
    let status = manager.py().import("builtins")?.call_method1(
        "getattr",
        (
            manager,
            "_cancel_status",
            "Cancelling; waiting for the operation to roll back.",
        ),
    )?;
    manager.setattr("status", status)?;
    diagnostics(manager)
}
#[pyfunction]
fn prepared_install_plan(
    prepared: &Bound<'_, PyAny>,
    choices: &Bound<'_, PyAny>,
    root: &str,
    cancel: &Bound<'_, PyAny>,
    repairs: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = prepared.py();
    let fomod = prepared.getattr("fomod")?;
    let package = prepared.getattr("package")?;
    if fomod.is_truthy()? {
        let kwargs = PyDict::new(py);
        kwargs.set_item("repairs", repairs)?;
        Ok(fomod
            .call_method(
                "plan",
                (package, choices, prepared.getattr("environment")?, cancel),
                Some(&kwargs),
            )?
            .unbind())
    } else {
        Ok(package.call_method1("plain_plan", (root, cancel))?.unbind())
    }
}
#[pyfunction]
fn prepared_install_validate(
    prepared: &Bound<'_, PyAny>,
    library: &Bound<'_, PyAny>,
    profile: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = prepared.py();
    for (attribute, module) in [
        ("overlay_review", "reference_overlay"),
        ("download_review", "download_install"),
    ] {
        let review = prepared.getattr(attribute)?;
        if !review.is_none() {
            py.import(format!("creation_lib.mod_manager.{module}"))?
                .call_method1("validate", (library, profile, review, cancel))?;
        }
    }
    let context = prepared.getattr("context")?;
    if !context.is_none() {
        if !context.getattr("profile_id")?.eq(profile)? {
            return Err(invalid(
                "Installer target profile changed. Reopen the package for this profile.",
            ));
        }
        py.import("creation_lib.mod_manager.installer_context")?
            .call_method1("validate", (library, context, cancel))?;
    }
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(controller_destination, m)?)?;
    m.add_function(wrap_pyfunction!(controller_validate_destination, m)?)?;
    m.add_function(wrap_pyfunction!(controller_prepare_library, m)?)?;
    m.add_function(wrap_pyfunction!(controller_open_library, m)?)?;
    m.add_function(wrap_pyfunction!(controller_submit, m)?)?;
    m.add_function(wrap_pyfunction!(controller_poll, m)?)?;
    m.add_function(wrap_pyfunction!(controller_cancel, m)?)?;
    m.add_function(wrap_pyfunction!(prepared_install_plan, m)?)?;
    m.add_function(wrap_pyfunction!(prepared_install_validate, m)?)?;
    Ok(())
}

#[pyfunction]
fn controller_destination(
    manager: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = manager.py();
    let library = manager.getattr("library")?;
    let root = if library.is_truthy()? {
        library.getattr("root")?
    } else {
        py.None().into_bound(py)
    };
    Ok(py
        .import("creation_lib.mod_manager.locations")?
        .call_method1("library_path", (path, root))?
        .unbind())
}
#[pyfunction]
fn controller_validate_destination(
    destination: &Bound<'_, PyAny>,
    preview: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = destination.py();
    crate::reject_links(&destination.extract::<std::path::PathBuf>()?)?;
    if !preview.is_truthy()? {
        return Ok(());
    }
    let mut protected = vec![
        preview.getattr("root")?,
        preview.getattr("mods_root")?,
        preview.getattr("profiles_root")?,
    ];
    let config = crate::plugin_inventory::value(&preview.getattr("configuration")?)?;
    if let Some(mapping) = config["source_mappings"].as_object() {
        for value in mapping.values() {
            protected.push(crate::plugin_inventory::object(py, value)?);
        }
    }
    for source in [
        &config["paths"]["overwrite_directory"],
        &config["game"]["gamepath"],
    ] {
        if source.as_str().is_some_and(|s| !s.is_empty()) {
            protected.push(crate::plugin_inventory::object(py, source)?);
        }
    }
    let target = destination.call_method0("resolve")?;
    for source in protected {
        let source = py
            .import("pathlib")?
            .getattr("Path")?
            .call1((source,))?
            .call_method0("resolve")?;
        if target
            .call_method1("is_relative_to", (&source,))?
            .is_truthy()?
            || source
                .call_method1("is_relative_to", (&target,))?
                .is_truthy()?
        {
            return Err(invalid(
                "Choose a destination library outside the MO2 instance, mod/profile folders, and game folder. No destination files were created.",
            ));
        }
    }
    Ok(())
}
#[pyfunction]
fn controller_prepare_library(destination: &Bound<'_, PyAny>, create: bool) -> PyResult<Py<PyAny>> {
    let py = destination.py();
    let kwargs = PyDict::new(py);
    kwargs.set_item("create", create)?;
    match py
        .import("creation_lib.mod_manager.library")?
        .getattr("Library")?
        .call((destination,), Some(&kwargs))
    {
        Ok(library) => Ok(library.unbind()),
        Err(e)
            if e.is_instance_of::<pyo3::exceptions::PyOSError>(py)
                || matches(py, &e, "creation_lib.mod_manager.paths", "ManagerError")?
                || matches(py, &e, "sqlite3", "Error")? =>
        {
            Err(invalid(format!(
                "Could not prepare import destination '{}': {}. Check the location, permissions and available disk space, or choose another destination. Existing files were not adopted.",
                destination.str()?,
                e.value(py)
            )))
        }
        Err(e) => Err(e),
    }
}
#[pyfunction]
fn controller_open_library(
    manager: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    create: bool,
) -> PyResult<()> {
    let py = manager.py();
    manager.call_method0("_assert_transfer_idle")?;
    let destination = manager.call_method1("resolve_destination", (path,))?;
    manager.call_method1(
        "_validate_destination",
        (&destination, manager.getattr("import_preview")?),
    )?;
    manager.call_method0("discard_install")?;
    let owner = manager.clone().unbind();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let library = args.get_item(0)?;
        manager.setattr("library", &library)?;
        manager.setattr("profile_id", "")?;
        for key in ["_context_view", "export_preview"] {
            manager.setattr(key, py.None())?;
        }
        manager.call_method0("refresh")?;
        manager.setattr(
            "status",
            if library.getattr("recovery_report")?.is_truthy()? {
                "Library recovered interrupted transactions. Review the Recovery tab."
            } else {
                "Library ready. Import MO2 profiles or create a profile to install mods."
            },
        )?;
        Ok(py.None())
    })?;
    let owner = manager.clone().unbind();
    let destination = destination.unbind();
    let work = callback(py, move |py, _, _| {
        let kwargs = PyDict::new(py);
        kwargs.set_item("create", create)?;
        Ok(owner
            .bind(py)
            .call_method("_prepare_library", (destination.bind(py),), Some(&kwargs))?
            .unbind())
    })?;
    manager.call_method1("submit", ("Opening library", work, complete))?;
    Ok(())
}
