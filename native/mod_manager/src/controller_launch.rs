use crate::{check_cancel, controller::callback, invalid};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
fn module<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyModule>> {
    py.import(format!("creation_lib.mod_manager.{name}"))
}
#[pyfunction]
fn controller_poll_runtime(manager: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = manager.py();
    let library = manager.getattr("library")?;
    let time = py.import("time")?;
    if !library.is_truthy()?
        || time.call_method0("monotonic")?.extract::<f64>()?
            < manager.getattr("_runtime_poll_at")?.extract::<f64>()?
    {
        return Ok(());
    }
    manager.setattr(
        "_runtime_poll_at",
        time.call_method0("monotonic")?.extract::<f64>()? + 0.3,
    )?;
    let runtime = module(py, "runtime")?;
    let sessions = runtime.call_method1("active_sessions", (library.getattr("root")?,))?;
    manager.setattr("runtime_sessions", &sessions)?;
    if sessions.is_truthy()? {
        let latest = sessions.get_item(sessions.len()? - 1)?;
        manager.setattr("runtime_session", latest.get_item(0)?)?;
        manager.setattr("runtime_status", latest.get_item(1)?)?;
        for session in sessions.try_iter()? {
            if session?
                .get_item(1)?
                .call_method1("get", ("state",))?
                .eq("identity-unverified")?
            {
                manager.setattr(
                    "error",
                    "Runtime identity could not be verified; library changes remain locked.",
                )?;
                break;
            }
        }
    } else if manager.getattr("runtime_session")?.is_truthy()? {
        let status =
            runtime.call_method1("session_status", (manager.getattr("runtime_session")?,))?;
        manager.setattr("runtime_status", &status)?;
        let state: Option<String> = status.call_method1("get", ("state",))?.extract()?;
        if [
            "starting",
            "mapping",
            "running",
            "supervision-failed",
            "identity-unverified",
        ]
        .contains(&state.as_deref().unwrap_or(""))
        {
            let status = status.call_method0("copy")?;
            status.set_item("state", "interrupted")?;
            manager.setattr("runtime_status", status)?;
        }
    }
    Ok(())
}
#[pyfunction]
fn controller_save_executable(
    manager: &Bound<'_, PyAny>,
    executable: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    if manager.getattr("busy")?.is_truthy()?
        || manager.getattr("runtime_active")?.is_truthy()?
        || !manager.getattr("library")?.is_truthy()?
    {
        return Err(invalid(
            "Open a library and finish the current operation/runtime before editing executables.",
        ));
    }
    let result = manager
        .getattr("library")?
        .call_method1("save_executable", (executable,))?;
    manager.call_method0("refresh")?;
    manager.setattr(
        "status",
        "Executable saved locally; its MO2 definition stays unchanged.",
    )?;
    Ok(result.unbind())
}
#[pyfunction]
fn controller_remove_executable(
    manager: &Bound<'_, PyAny>,
    identifier: &Bound<'_, PyAny>,
) -> PyResult<()> {
    if manager.getattr("busy")?.is_truthy()?
        || manager.getattr("runtime_active")?.is_truthy()?
        || !manager.getattr("library")?.is_truthy()?
    {
        return Err(invalid(
            "Finish the current operation/runtime before removing an executable.",
        ));
    }
    manager
        .getattr("library")?
        .call_method1("remove_executable", (identifier,))?;
    manager.call_method0("refresh")?;
    manager.setattr(
        "status",
        "Executable removed locally; repeat import will keep it removed.",
    )?;
    Ok(())
}
#[pyfunction]
fn controller_save_executables(
    manager: &Bound<'_, PyAny>,
    instance: &Bound<'_, PyAny>,
    expected: &Bound<'_, PyAny>,
    items: &Bound<'_, PyAny>,
    default: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    if manager.getattr("busy")?.is_truthy()?
        || manager.getattr("runtime_active")?.is_truthy()?
        || !manager.getattr("library")?.is_truthy()?
    {
        return Err(invalid(
            "Finish the current worker/runtime before editing executables.",
        ));
    }
    let result = module(manager.py(), "executable_catalog")?.call_method1(
        "save",
        (
            manager.getattr("library")?,
            instance,
            expected,
            items,
            default,
        ),
    )?;
    manager.call_method0("refresh")?;
    manager.setattr(
        "status",
        "Executable definitions, order and default saved locally.",
    )?;
    Ok(result.unbind())
}
#[pyfunction]
fn controller_prepare_launch(
    manager: &Bound<'_, PyAny>,
    executable_id: &Bound<'_, PyAny>,
    bundle_root: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = manager.py();
    let mut executable = py.None().into_bound(py);
    for item in manager.getattr("executables")?.try_iter()? {
        let item = item?;
        if item.getattr("id")?.eq(executable_id)? {
            executable = item;
            break;
        }
    }
    if manager.getattr("target_id")?.is_truthy()? {
        let target = manager.getattr("context_library")?.call_method0("target")?;
        if executable_id.eq(target.getattr("executable")?.getattr("id")?)? {
            executable = module(py, "target_options")?
                .call_method1("executable", (manager.getattr("library")?, target))?;
        }
    }
    if !executable.is_truthy()? {
        return Err(invalid("Select a configured executable first."));
    }
    manager.setattr("launch_review", py.None())?;
    let profile_id = manager.getattr("profile_id")?;
    let mut profile = py.None().into_bound(py);
    for item in manager.getattr("profiles")?.try_iter()? {
        let item = item?;
        if item.getattr("id")?.eq(&profile_id)? {
            profile = item;
            break;
        }
    }
    let library = manager.getattr("library")?.unbind();
    let profile_id = profile_id.unbind();
    let profile = profile.unbind();
    let target = manager.getattr("target_id")?.unbind();
    let bundle = bundle_root.clone().unbind();
    let executable = executable.unbind();
    let work = callback(py, move |py, args, _| {
        let cancel = args.get_item(0)?;
        let progress = args.get_item(1)?;
        let executable = executable.bind(py);
        let profile_id = profile_id.bind(py);
        let profile = profile.bind(py);
        let library = library.bind(py);
        if executable.getattr("mode")?.eq("direct")? {
            let name = if profile.is_truthy()? {
                profile.getattr("name")?
            } else {
                "".into_pyobject(py)?.into_any()
            };
            let plan =
                module(py, "launcher")?.call_method1("review", (executable, profile_id, name))?;
            return Ok(PyTuple::new(py, [plan, py.None().into_bound(py)])?
                .into_any()
                .unbind());
        }
        let kwargs = PyDict::new(py);
        kwargs.set_item("selected_target", target.bind(py))?;
        kwargs.set_item("cancel", &cancel)?;
        let resolution = module(py, "launch_resolution")?
            .getattr("resolve")?
            .call((library, executable), Some(&kwargs))?;
        let context = module(py, "launch_targets")?
            .getattr("TargetLibrary")?
            .call1((library, resolution.getattr("target_id")?))?;
        let plan = module(py, "runtime")?.call_method1(
            "prepare",
            (
                context,
                profile_id,
                resolution.getattr("executable")?,
                resolution.getattr("game_root")?,
                bundle.bind(py),
                cancel,
                progress,
            ),
        )?;
        Ok(PyTuple::new(py, [plan, resolution])?.into_any().unbind())
    })?;
    let owner = manager.clone().unbind();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let result = args.get_item(0)?;
        let plan = result.get_item(0)?;
        let resolution = result.get_item(1)?;
        let mut status = "Launch ready for review.".to_string();
        if resolution.is_truthy()? {
            manager.call_method0("discard_install")?;
            manager.setattr("target_id", resolution.getattr("target_id")?)?;
            for name in ["_context_view", "target_review"] {
                manager.setattr(name, py.None())?;
            }
            manager.call_method0("refresh")?;
            status.push_str(&format!(
                " Using the selected executable's installation: {}",
                resolution.getattr("game_root")?.str()?
            ));
        }
        manager.setattr("launch_review", plan)?;
        manager.setattr("status", status)?;
        Ok(py.None())
    })?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("allow_runtime", true)?;
    manager.call_method(
        "submit",
        ("Reviewing executable launch", work, complete),
        Some(&kwargs),
    )?;
    Ok(())
}
#[pyfunction]
fn controller_start_launch(
    manager: &Bound<'_, PyAny>,
    accept_source_writes: bool,
    accept_minimal_ini: bool,
    accept_experimental: bool,
    disable_mcp: bool,
) -> PyResult<()> {
    let py = manager.py();
    manager.call_method0("_assert_transfer_idle")?;
    let plan = manager.getattr("launch_review")?;
    if !plan.is_truthy()? {
        return Err(invalid("Review the executable before running it."));
    }
    let library = manager.getattr("context_library")?.unbind();
    let captured = plan.clone().unbind();
    let work = callback(py, move |py, args, _| {
        let cancel = args.get_item(0)?;
        check_cancel(Some(&cancel))?;
        let plan = captured.bind(py);
        let runtime = module(py, "runtime")?;
        if plan.is_instance(&runtime.getattr("RuntimePlan")?)? {
            let kwargs = PyDict::new(py);
            kwargs.set_item("disable_mcp", disable_mcp)?;
            let updated = py
                .import("dataclasses")?
                .getattr("replace")?
                .call((plan,), Some(&kwargs))?;
            let kwargs = PyDict::new(py);
            kwargs.set_item("accept_source_writes", accept_source_writes)?;
            kwargs.set_item("accept_minimal_ini", accept_minimal_ini)?;
            kwargs.set_item("accept_experimental", accept_experimental)?;
            kwargs.set_item("cancel", cancel)?;
            Ok(runtime
                .getattr("start")?
                .call((library.bind(py), updated), Some(&kwargs))?
                .unbind())
        } else {
            let kwargs = PyDict::new(py);
            kwargs.set_item("disable_mcp", disable_mcp)?;
            Ok(module(py, "launcher")?
                .getattr("start")?
                .call((plan,), Some(&kwargs))?
                .unbind())
        }
    })?;
    let owner = manager.clone().unbind();
    let plan = plan.unbind();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let plan = plan.bind(py);
        let result = args.get_item(0)?;
        manager.setattr("launch_review", py.None())?;
        let runtime = module(py, "runtime")?;
        if plan.is_instance(&runtime.getattr("RuntimePlan")?)? {
            for (index, name) in ["runtime_session", "runtime_process", "runtime_status"]
                .iter()
                .enumerate()
            {
                manager.setattr(*name, result.get_item(index)?)?;
            }
            manager.setattr(
                "runtime_sessions",
                runtime.call_method1(
                    "active_sessions",
                    (manager.getattr("library")?.getattr("root")?,),
                )?,
            )?;
            let backend = if plan.getattr("runtime")?.eq("modbox-vfs-fo4-x64")? {
                "Project filespace"
            } else {
                "Legacy USVFS"
            };
            let launch = plan.getattr("launch")?;
            manager.setattr(
                "status",
                format!(
                    "{backend} runtime {}: {} | {}",
                    manager
                        .getattr("runtime_status")?
                        .get_item("state")?
                        .str()?,
                    launch.getattr("executable")?.getattr("title")?.str()?,
                    launch.getattr("profile_name")?.str()?
                ),
            )?;
        } else {
            manager.setattr(
                "status",
                format!(
                    "Started {} directly. The selected mod profile is not virtualized.",
                    plan.getattr("executable")?.getattr("title")?.str()?
                ),
            )?;
        }
        Ok(py.None())
    })?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("allow_runtime", true)?;
    manager.call_method(
        "submit",
        ("Starting reviewed executable", work, complete),
        Some(&kwargs),
    )?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(controller_poll_runtime, m)?)?;
    m.add_function(wrap_pyfunction!(controller_save_executable, m)?)?;
    m.add_function(wrap_pyfunction!(controller_remove_executable, m)?)?;
    m.add_function(wrap_pyfunction!(controller_save_executables, m)?)?;
    m.add_function(wrap_pyfunction!(controller_prepare_launch, m)?)?;
    m.add_function(wrap_pyfunction!(controller_start_launch, m)?)?;
    Ok(())
}
