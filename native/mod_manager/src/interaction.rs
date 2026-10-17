use crate::{controller::callback, history_prune::py_path, invalid, reject_links};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use std::path::PathBuf;

fn drag_rows<'py>(
    workspace: &Bound<'py, PyAny>,
    dropping: bool,
) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyAny>)> {
    let py = workspace.py();
    let manager = workspace.getattr("manager")?;
    let sorted = workspace
        .getattr("_sort_by")
        .map(|v| v.extract::<String>())
        .unwrap_or_else(|_| Ok("priority".into()))?;
    let descending = workspace
        .getattr("_descending")
        .map(|v| v.is_truthy())
        .unwrap_or(Ok(false))?;
    if sorted != "priority" || descending {
        return Err(invalid(if dropping {
            "Choose ascending Priority before dropping load order. Visual sorting does not change priorities."
        } else {
            "Choose ascending Priority before dragging load order. Visual sorting does not change priorities."
        }));
    }
    let library = manager.getattr("library")?;
    if manager.getattr("busy")?.is_truthy()?
        || manager.getattr("runtime_active")?.is_truthy()?
        || !library.is_truthy()?
    {
        return Err(invalid(if dropping {
            "Wait for the current worker and runtime before dropping mods."
        } else {
            "Wait for the current worker and runtime before dragging mods."
        }));
    }
    let rows = if manager
        .getattr("target_id")
        .map(|v| v.is_truthy())
        .unwrap_or(Ok(false))?
    {
        library.call_method1("rows", (manager.getattr("profile_id")?,))?
    } else {
        manager.getattr("rows")?
    };
    let key = callback(py, |py, args, _| {
        let row = args.get_item(0)?;
        Ok((row.getattr("priority")?, row.getattr("id")?)
            .into_pyobject(py)?
            .into_any()
            .unbind())
    })?;
    let options = PyDict::new(py);
    options.set_item("key", key)?;
    Ok((
        manager,
        py.import("builtins")?
            .getattr("sorted")?
            .call((rows,), Some(&options))?,
    ))
}
fn entries<'py>(rows: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    let py = rows.py();
    let mut entries = vec![];
    for row in rows.try_iter()? {
        let row = row?;
        entries.push(
            (
                row.getattr("id")?,
                row.getattr("priority")?,
                i32::from(row.getattr("enabled")?.is_truthy()?),
                row.getattr("group")?,
            )
                .into_pyobject(py)?,
        );
    }
    PyTuple::new(py, entries)
}
#[pyfunction]
fn manager_drag_begin(
    workspace: &Bound<'_, PyAny>,
    origin: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = workspace.py();
    let (manager, rows) = drag_rows(workspace, false)?;
    let selected = workspace.getattr("_selected_mods")?;
    let selected = if selected.contains(origin)? {
        selected
    } else {
        PyTuple::new(py, [origin])?.into_any()
    };
    let mut chosen = vec![];
    for row in rows.try_iter()? {
        let id = row?.getattr("id")?;
        if selected.contains(&id)? {
            chosen.push(id);
        }
    }
    if !manager.getattr("rows_by_id")?.contains(origin)?
        || chosen.is_empty()
        || chosen.len() != selected.len()?
    {
        return Err(invalid(
            "Selected profile entries changed; refresh before dragging them.",
        ));
    }
    let entries = entries(&rows)?;
    let snapshot = PyList::empty(py);
    for row in entries.iter() {
        let record = PyDict::new(py);
        for (i, key) in ["mod_id", "priority", "enabled", "group_name"]
            .iter()
            .enumerate()
        {
            record.set_item(key, row.get_item(i)?)?;
        }
        snapshot.append(record)?;
    }
    let digest = py
        .import("creation_lib.mod_manager.organization")?
        .call_method1("digest", (snapshot,))?;
    Ok(py
        .import("creation_lib.mod_manager.mod_drag")?
        .getattr("Drag")?
        .call1((
            manager.getattr("library")?.getattr("library_id")?,
            manager.getattr("profile_id")?,
            PyTuple::new(py, chosen)?,
            entries,
            digest,
        ))?
        .unbind())
}
#[pyfunction]
fn manager_drag_drop(
    workspace: &Bound<'_, PyAny>,
    gesture: &Bound<'_, PyAny>,
    target: &Bound<'_, PyAny>,
    group: &Bound<'_, PyAny>,
    after: bool,
) -> PyResult<()> {
    let py = workspace.py();
    let (manager, rows) = drag_rows(workspace, true)?;
    if !manager
        .getattr("library")?
        .getattr("library_id")?
        .eq(gesture.getattr("library_id")?)?
        || !manager
            .getattr("profile_id")?
            .eq(gesture.getattr("profile_id")?)?
        || !entries(&rows)?.as_any().eq(gesture.getattr("entries")?)?
    {
        return Err(invalid(
            "Profile entries changed during this drag. Drag the selection again.",
        ));
    }
    let selected = gesture.getattr("mod_ids")?;
    let options = PyDict::new(py);
    options.set_item("expected", gesture.getattr("expected")?)?;
    if !group.is_none() {
        options.set_item("group", group)?;
    } else if !selected.contains(target)? {
        let mut priority = 0;
        let mut found = false;
        for row in gesture.getattr("entries")?.try_iter()? {
            let id = row?.get_item(0)?;
            if !selected.contains(&id)? {
                if id.eq(target)? {
                    found = true;
                    break;
                }
                priority += 1;
            }
        }
        if !found {
            return Err(invalid(
                "The target profile entry changed; drag the selection again.",
            ));
        }
        options.set_item("priority", priority + i32::from(after))?;
    } else {
        return Ok(());
    }
    manager
        .getattr("bulk_organize")?
        .call((selected,), Some(&options))?;
    Ok(())
}
#[pyfunction]
fn manager_validate_intake(path: &Bound<'_, PyAny>) -> PyResult<String> {
    let py = path.py();
    let candidate = py
        .import("pathlib")?
        .call_method1("Path", (path,))?
        .call_method0("absolute")?;
    reject_links(&candidate.extract::<PathBuf>()?)?;
    if !py
        .import("creation_lib.mod_manager.mo2")?
        .call_method1("local_mod_target", (&candidate,))?
        .is_truthy()?
        || !candidate.call_method0("exists")?.is_truthy()?
    {
        return Err(invalid(
            "Choose an ordinary existing local mod archive or unpacked folder.",
        ));
    }
    let suffix = candidate
        .getattr("suffix")?
        .extract::<String>()?
        .to_lowercase();
    if !candidate.call_method0("is_dir")?.is_truthy()?
        && (!candidate.call_method0("is_file")?.is_truthy()?
            || ![".zip", ".7z", ".rar", ".fomod"].contains(&suffix.as_str()))
    {
        return Err(invalid(
            "Choose a ZIP, 7z, RAR, FOMOD archive or unpacked mod folder.",
        ));
    }
    candidate.str()?.extract()
}
#[pyfunction]
fn manager_preferred_game(
    manager: &Bound<'_, PyAny>,
    explicit: &str,
    settings: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let target = manager.getattr("target_id")?;
    if target.is_truthy()? {
        for item in manager.getattr("targets")?.try_iter()? {
            let item = item?;
            if item.getattr("id")?.eq(&target)? {
                return item.getattr("game_root")?.extract();
            }
        }
    }
    if !explicit.is_empty() {
        return Ok(explicit.into());
    }
    let py = manager.py();
    let imported = manager
        .getattr("instance_configuration")?
        .call_method1("get", ("game", PyDict::new(py)))?
        .call_method1("get", ("gamepath", ""))?
        .extract::<String>()?;
    if !imported.is_empty() {
        return Ok(imported);
    }
    if settings.is_truthy()? {
        settings
            .call_method1("get_game_paths", ("fo4",))?
            .call_method1("get", ("root_dir", ""))?
            .extract()
    } else {
        Ok(String::new())
    }
}
#[pyfunction]
fn manager_selected_executable(
    manager: &Bound<'_, PyAny>,
    selected: &str,
) -> PyResult<(String, Py<PyAny>)> {
    let py = manager.py();
    let target_id = manager.getattr("target_id")?;
    let executables = manager.getattr("executables")?;
    if target_id.is_truthy()? {
        let mut target = None;
        for item in manager.getattr("targets")?.try_iter()? {
            let item = item?;
            if item.getattr("id")?.eq(&target_id)? {
                target = Some(item);
                break;
            }
        }
        if let Some(target) = target {
            for item in executables.try_iter()? {
                let item = item?;
                if item.getattr("id")?.extract::<String>()? == selected
                    && !item.getattr("hidden")?.is_truthy()?
                {
                    let binary = py_path(py, &item.getattr("binary")?.extract::<PathBuf>()?)?;
                    let same = binary.call_method0("resolve")?.eq(py_path(
                        py,
                        &target.getattr("binary")?.extract::<PathBuf>()?,
                    )?
                    .call_method0("resolve")?)?;
                    let root = !target
                        .getattr("f4se")?
                        .call_method1("get", ("managed",))?
                        .is_truthy()?
                        && binary
                            .getattr("parent")?
                            .call_method0("resolve")?
                            .eq(py_path(
                                py,
                                &target.getattr("game_root")?.extract::<PathBuf>()?,
                            )?
                            .call_method0("resolve")?)?;
                    if same || root {
                        return Ok((selected.into(), item.unbind()));
                    }
                    break;
                }
            }
        }
        return Ok((
            selected.into(),
            manager.getattr("target_executable")?.unbind(),
        ));
    }
    let mut visible = vec![];
    for item in executables.try_iter()? {
        let item = item?;
        if !item.getattr("hidden")?.is_truthy()? {
            visible.push(item);
        }
    }
    for item in &visible {
        if item.getattr("id")?.extract::<String>()? == selected {
            return Ok((selected.into(), item.clone().unbind()));
        }
    }
    let library = manager.getattr("library")?;
    let catalog = py.import("creation_lib.mod_manager.executable_catalog")?;
    let default = if library.is_truthy()? {
        catalog
            .call_method1(
                "preferences",
                (library, catalog.call_method1("instance", (manager,))?),
            )?
            .get_item("default")?
            .extract::<String>()?
    } else {
        String::new()
    };
    let mut preferred = None;
    let mut loader = None;
    for (i, item) in visible.iter().enumerate() {
        if item.getattr("id")?.extract::<String>()? == default && preferred.is_none() {
            preferred = Some(i);
        }
        if loader.is_none()
            && py_path(py, &item.getattr("binary")?.extract::<PathBuf>()?)?
                .getattr("name")?
                .extract::<String>()?
                .to_lowercase()
                == "f4se_loader.exe"
        {
            loader = Some(i);
        }
    }
    let index = preferred.or(loader).unwrap_or(0);
    if let Some(item) = visible.get(index) {
        Ok((item.getattr("id")?.extract()?, item.clone().unbind()))
    } else {
        Ok((String::new(), py.None()))
    }
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(manager_drag_begin, m)?)?;
    m.add_function(wrap_pyfunction!(manager_drag_drop, m)?)?;
    m.add_function(wrap_pyfunction!(manager_validate_intake, m)?)?;
    m.add_function(wrap_pyfunction!(manager_preferred_game, m)?)?;
    m.add_function(wrap_pyfunction!(manager_selected_executable, m)?)?;
    Ok(())
}
