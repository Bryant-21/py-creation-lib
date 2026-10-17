use crate::{controller::callback, invalid};
use pyo3::{prelude::*, types::PyDict};
fn module<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyModule>> {
    py.import(format!("creation_lib.mod_manager.{name}"))
}
#[pyfunction]
fn controller_review_target(
    manager: &Bound<'_, PyAny>,
    name: &str,
    game_root: &Bound<'_, PyAny>,
    identifier: &str,
) -> PyResult<()> {
    let py = manager.py();
    let library = manager.getattr("library")?;
    if !library.is_truthy()? {
        return Err(invalid("Open a manager library first."));
    }
    for key in ["target_review", "root_setup_review"] {
        manager.setattr(key, py.None())?;
    }
    let library = library.unbind();
    let name = name.to_string();
    let root = game_root.clone().unbind();
    let identifier = identifier.to_string();
    let work = callback(py, move |py, args, _| {
        let kwargs = PyDict::new(py);
        kwargs.set_item("identifier", &identifier)?;
        kwargs.set_item("cancel", args.get_item(0)?)?;
        Ok(module(py, "launch_targets")?
            .getattr("review")?
            .call((library.bind(py), &name, root.bind(py)), Some(&kwargs))?
            .unbind())
    })?;
    let owner = manager.clone().unbind();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        manager.setattr("target_review", args.get_item(0)?)?;
        manager.setattr(
            "status",
            "FO4 folder reviewed. Its exact build and F4SE findings are ready.",
        )?;
        Ok(py.None())
    })?;
    manager.call_method1("submit", ("Inspecting FO4 folder", work, complete))?;
    Ok(())
}
#[pyfunction]
fn controller_apply_target(
    manager: &Bound<'_, PyAny>,
    automatic: bool,
    archive: &Bound<'_, PyAny>,
    nxm: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = manager.py();
    manager.call_method0("_assert_transfer_idle")?;
    let plan = manager.getattr("target_review")?;
    let library = manager.getattr("library")?;
    if !plan.is_truthy()? || !library.is_truthy()? {
        return Err(invalid("Review the FO4 folder first."));
    }
    let captured = plan.clone().unbind();
    let library = library.unbind();
    let work = callback(py, move |py, args, _| {
        Ok(module(py, "launch_targets")?
            .call_method1(
                "apply",
                (library.bind(py), captured.bind(py), args.get_item(0)?),
            )?
            .unbind())
    })?;
    let owner = manager.clone().unbind();
    let plan = plan.unbind();
    let archive = archive.clone().unbind();
    let nxm = nxm.clone().unbind();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        for key in ["target_review", "_context_view"] {
            manager.setattr(key, py.None())?;
        }
        manager.setattr("target_id", args.get_item(0)?)?;
        manager.call_method0("refresh")?;
        manager.setattr("status", "Launch target saved.")?;
        if automatic
            && !plan
                .bind(py)
                .getattr("target")?
                .getattr("f4se")?
                .call_method1("get", ("state",))?
                .eq("ready")?
        {
            let kwargs = PyDict::new(py);
            kwargs.set_item("archive", archive.bind(py))?;
            kwargs.set_item("nxm", nxm.bind(py))?;
            manager.call_method("prepare_target_f4se", (), Some(&kwargs))?;
        }
        Ok(py.None())
    })?;
    manager.call_method1("submit", ("Saving reviewed launch target", work, complete))?;
    Ok(())
}
#[pyfunction]
fn controller_prepare_target_f4se(
    manager: &Bound<'_, PyAny>,
    archive: &Bound<'_, PyAny>,
    nxm: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = manager.py();
    manager.call_method0("_assert_transfer_idle")?;
    let library = manager.getattr("library")?;
    let identifier = manager.getattr("target_id")?;
    if !library.is_truthy()? || !identifier.is_truthy()? {
        return Err(invalid("Select a launch target first."));
    }
    let library = library.unbind();
    let identifier = identifier.unbind();
    let client = manager.getattr("nexus_client")?.unbind();
    let archive = archive.clone().unbind();
    let nxm = nxm.clone().unbind();
    let work = callback(py, move |py, args, _| {
        let kwargs = PyDict::new(py);
        kwargs.set_item("archive", archive.bind(py))?;
        kwargs.set_item("client", client.bind(py))?;
        kwargs.set_item("nxm", nxm.bind(py))?;
        kwargs.set_item("cancel", args.get_item(0)?)?;
        kwargs.set_item("progress", args.get_item(1)?)?;
        Ok(module(py, "f4se_setup")?
            .getattr("prepare")?
            .call((library.bind(py), identifier.bind(py)), Some(&kwargs))?
            .unbind())
    })?;
    let owner = manager.clone().unbind();
    let complete = callback(py, move |py, _, _| {
        let manager = owner.bind(py);
        manager.setattr("_context_view", py.None())?;
        manager.call_method0("refresh")?;
        manager.setattr(
            "status",
            "Matching F4SE prepared in target-owned storage. Run opens a native launch review.",
        )?;
        Ok(py.None())
    })?;
    manager.call_method1("submit", ("Preparing matching F4SE", work, complete))?;
    Ok(())
}
#[pyfunction]
fn controller_review_root_setup(manager: &Bound<'_, PyAny>, restore: bool) -> PyResult<()> {
    let py = manager.py();
    manager.call_method0("_assert_transfer_idle")?;
    let library = manager.getattr("library")?;
    let identifier = manager.getattr("target_id")?;
    if !library.is_truthy()? || !identifier.is_truthy()? {
        return Err(invalid("Select a FO4 launch target first."));
    }
    for key in ["root_setup_review", "target_review"] {
        manager.setattr(key, py.None())?;
    }
    let library = library.unbind();
    let identifier = identifier.unbind();
    let work = callback(py, move |py, args, _| {
        let kwargs = PyDict::new(py);
        kwargs.set_item("cancel", args.get_item(0)?)?;
        Ok(module(py, "f4se_root_setup")?
            .getattr(if restore { "review_restore" } else { "review" })?
            .call((library.bind(py), identifier.bind(py)), Some(&kwargs))?
            .unbind())
    })?;
    let owner = manager.clone().unbind();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        manager.setattr("root_setup_review", args.get_item(0)?)?;
        manager.setattr(
            "status",
            if restore {
                "Game-root restore reviewed."
            } else {
                "Game-root F4SE bootstrap setup reviewed. No files have been replaced."
            },
        )?;
        Ok(py.None())
    })?;
    manager.call_method1(
        "submit",
        (
            if restore {
                "Reviewing game-root F4SE restore"
            } else {
                "Reviewing game-root F4SE setup"
            },
            work,
            complete,
        ),
    )?;
    Ok(())
}
#[pyfunction]
fn controller_apply_root_setup(
    manager: &Bound<'_, PyAny>,
    accept_root_writes: bool,
) -> PyResult<()> {
    let py = manager.py();
    manager.call_method0("_assert_transfer_idle")?;
    let plan = manager.getattr("root_setup_review")?;
    let library = manager.getattr("library")?;
    if !plan.is_truthy()?
        || !library.is_truthy()?
        || !plan
            .getattr("target_id")?
            .eq(manager.getattr("target_id")?)?
    {
        return Err(invalid("Review this target's root setup/restore first."));
    }
    let restore = plan.getattr("restore")?.is_truthy()?;
    let library = library.unbind();
    let plan = plan.unbind();
    let work = callback(py, move |py, args, _| {
        let kwargs = PyDict::new(py);
        kwargs.set_item("accept_root_writes", accept_root_writes)?;
        kwargs.set_item("cancel", args.get_item(0)?)?;
        Ok(module(py, "f4se_root_setup")?
            .getattr(if restore { "restore" } else { "apply" })?
            .call((library.bind(py), plan.bind(py)), Some(&kwargs))?
            .unbind())
    })?;
    let owner = manager.clone().unbind();
    let complete = callback(py, move |py, _, _| {
        let manager = owner.bind(py);
        manager.setattr("root_setup_review", py.None())?;
        manager.call_method0("discard_install")?;
        manager.setattr("_context_view", py.None())?;
        manager.call_method0("refresh")?;
        manager.setattr("status", if restore { "Root originals restored; private runtime remains available." } else { "F4SE bootstrap uses recoverable game-root placement. Run still requires a separate launch review." })?;
        Ok(py.None())
    })?;
    manager.call_method1(
        "submit",
        (
            if restore {
                "Restoring game-root F4SE"
            } else {
                "Applying reviewed game-root F4SE"
            },
            work,
            complete,
        ),
    )?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(controller_review_target, m)?)?;
    m.add_function(wrap_pyfunction!(controller_apply_target, m)?)?;
    m.add_function(wrap_pyfunction!(controller_prepare_target_f4se, m)?)?;
    m.add_function(wrap_pyfunction!(controller_review_root_setup, m)?)?;
    m.add_function(wrap_pyfunction!(controller_apply_root_setup, m)?)?;
    Ok(())
}
