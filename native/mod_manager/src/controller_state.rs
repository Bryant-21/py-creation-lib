use crate::invalid;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
};
use std::collections::BTreeSet;
fn module<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyModule>> {
    py.import(format!("creation_lib.mod_manager.{name}"))
}
#[pyfunction]
fn controller_context(manager: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = manager.py();
    let library = manager.getattr("library")?;
    let target = manager.getattr("target_id")?;
    if !library.is_truthy()? || !target.is_truthy()? {
        return Ok(library.unbind());
    }
    let view = manager.getattr("_context_view")?;
    if !view.is_truthy()?
        || !view.getattr("base")?.is(&library)
        || !view.getattr("target_id")?.eq(&target)?
    {
        manager.setattr(
            "_context_view",
            module(py, "launch_targets")?
                .getattr("TargetLibrary")?
                .call1((library, target))?,
        )?;
    }
    Ok(manager.getattr("_context_view")?.unbind())
}
#[pyfunction]
fn controller_save_context(manager: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = manager.py();
    let context = manager.getattr("context_library")?;
    if !manager.getattr("library")?.is_truthy()?
        || !manager.getattr("target_id")?.is_truthy()?
        || !manager
            .getattr("target_scopes")?
            .get_item("saves")?
            .eq("shared")?
    {
        return Ok(context.unbind());
    }
    let cached = py
        .import("builtins")?
        .call_method1("getattr", (manager, "_save_context_view", py.None()))?;
    if cached.is_none() || !cached.getattr("context")?.is(&context) {
        manager.setattr(
            "_save_context_view",
            module(py, "target_preferences")?
                .getattr("SaveLibrary")?
                .call1((context,))?,
        )?;
    }
    Ok(manager.getattr("_save_context_view")?.unbind())
}
#[pyfunction]
fn controller_runtime_active(manager: &Bound<'_, PyAny>) -> PyResult<bool> {
    if manager.getattr("runtime_sessions")?.is_truthy()? {
        return Ok(true);
    }
    let state: Option<String> = manager
        .getattr("runtime_status")?
        .call_method1("get", ("state",))?
        .extract()?;
    Ok([
        "starting",
        "mapping",
        "running",
        "supervision-failed",
        "identity-unverified",
    ]
    .contains(&state.as_deref().unwrap_or("")))
}
#[pyfunction]
fn controller_select_profile(
    manager: &Bound<'_, PyAny>,
    profile: &Bound<'_, PyAny>,
) -> PyResult<()> {
    if manager.getattr("busy")?.is_truthy()? {
        return Err(invalid(
            "Wait for the current operation before switching profiles.",
        ));
    }
    manager.call_method0("discard_install")?;
    manager.setattr("profile_id", profile)?;
    manager.setattr("export_preview", manager.py().None())?;
    manager.call_method0("refresh")?;
    Ok(())
}
#[pyfunction]
fn controller_select_target(
    manager: &Bound<'_, PyAny>,
    identifier: &Bound<'_, PyAny>,
) -> PyResult<()> {
    manager.call_method0("_assert_transfer_idle")?;
    if manager.getattr("busy")?.is_truthy()? {
        return Err(invalid(
            "Wait for the current worker before switching launch targets.",
        ));
    }
    if identifier.is_truthy()? {
        let mut found = false;
        for item in manager.getattr("targets")?.try_iter()? {
            if item?.getattr("id")?.eq(identifier)? {
                found = true;
                break;
            }
        }
        if !found {
            return Err(invalid("Select a configured launch target."));
        }
    }
    manager.call_method0("discard_install")?;
    manager.setattr("target_id", identifier)?;
    manager.setattr("_context_view", manager.py().None())?;
    manager.setattr("target_review", manager.py().None())?;
    manager.call_method0("refresh")?;
    manager.setattr(
        "status",
        "Launch target selected. Runtime reviews and cached provider/save views were cleared.",
    )?;
    Ok(())
}
#[pyfunction]
fn controller_refresh(manager: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = manager.py();
    manager
        .getattr("metadata_observer")?
        .call_method0("invalidate")?;
    for key in [
        "save_transfer_plan",
        "overwrite_review",
        "overwrite_restore",
        "launch_review",
        "plugin_catalog",
        "archive_index",
        "combined_index",
        "save_index",
        "profile_restore",
        "file_index",
        "recipe_plan",
        "name_review",
        "reference_review",
        "field_review",
        "prune_review",
        "group_review",
        "retained_review",
        "relink_review",
        "source_metadata_review",
        "root_setup_review",
    ] {
        manager.setattr(key, py.None())?;
    }
    manager.setattr("save_backups", PyTuple::empty(py))?;
    manager.setattr("overwrite_history", PyList::empty(py))?;
    for key in [
        "save_inspections",
        "save_comparisons",
        "save_images",
        "save_details",
    ] {
        manager.setattr(key, PyDict::new(py))?;
    }
    let library = manager.getattr("library")?;
    let has_library = library.is_truthy()?;
    if has_library {
        let sessions =
            module(py, "runtime")?.call_method1("active_sessions", (library.getattr("root")?,))?;
        manager.setattr("runtime_sessions", &sessions)?;
        if sessions.is_truthy()? {
            let first = sessions.get_item(0)?;
            manager.setattr("runtime_session", first.get_item(0)?)?;
            manager.setattr("runtime_status", first.get_item(1)?)?;
        }
    }
    manager.setattr(
        "origins",
        if has_library {
            library.call_method0("origins")?
        } else {
            PyDict::new(py).into_any()
        },
    )?;
    for (attr, method) in [
        ("migration_runs", "migration_runs"),
        ("deleted_profiles", "deleted_profiles"),
    ] {
        manager.setattr(
            attr,
            if has_library {
                library.call_method0(method)?
            } else {
                PyList::empty(py).into_any()
            },
        )?;
    }
    let targets = if has_library {
        module(py, "launch_targets")?.call_method1("records", (&library,))?
    } else {
        PyList::empty(py).into_any()
    };
    manager.setattr("targets", &targets)?;
    let target_id = manager.getattr("target_id")?;
    let mut selected_target = None;
    for target in targets.try_iter()? {
        let target = target?;
        if target.getattr("id")?.eq(&target_id)? {
            selected_target = Some(target);
            break;
        }
    }
    if target_id.is_truthy()? && selected_target.is_none() {
        manager.setattr("target_id", "")?;
        manager.setattr("_context_view", py.None())?;
    }
    if let Some(target) = selected_target {
        manager.setattr(
            "target_executable",
            module(py, "target_options")?.call_method1("executable", (&library, target))?,
        )?;
        manager.setattr(
            "target_scopes",
            module(py, "target_preferences")?
                .call_method1("value", (&library, manager.getattr("target_id")?))?,
        )?;
    } else {
        manager.setattr("target_executable", py.None())?;
        let scopes = PyDict::new(py);
        scopes.set_item("ini", "target")?;
        scopes.set_item("saves", "target")?;
        manager.setattr("target_scopes", scopes)?;
    }
    let context = manager.getattr("context_library")?;
    let profiles = if has_library {
        context.call_method0("profiles")?
    } else {
        PyList::empty(py).into_any()
    };
    manager.setattr("profiles", &profiles)?;
    let mut profile_id = manager.getattr("profile_id")?;
    let mut selected_profile = None;
    for profile in profiles.try_iter()? {
        let profile = profile?;
        if profile.getattr("id")?.eq(&profile_id)? {
            selected_profile = Some(profile);
            break;
        }
    }
    if selected_profile.is_none() {
        if profiles.is_truthy()? {
            let profile = profiles.get_item(0)?;
            profile_id = profile.getattr("id")?;
            selected_profile = Some(profile);
        } else {
            profile_id = "".into_pyobject(py)?.into_any();
        }
        manager.setattr("profile_id", &profile_id)?;
    }
    let selected = has_library && profile_id.is_truthy()?;
    let targeted = manager.getattr("target_id")?.is_truthy()? && profile_id.is_truthy()?;
    let rows = if targeted {
        let rows = PyList::empty(py);
        for row in context
            .call_method1("profile_snapshot", (&profile_id,))?
            .get_item(1)?
            .try_iter()?
        {
            let row = row?;
            if !row
                .getattr("id")?
                .extract::<String>()?
                .starts_with("f4se-target-")
            {
                rows.append(row)?;
            }
        }
        rows.into_any()
    } else if selected {
        library.call_method1("rows", (&profile_id,))?
    } else {
        PyList::empty(py).into_any()
    };
    manager.setattr("rows", &rows)?;
    let by_id = PyDict::new(py);
    let mut groups = BTreeSet::from([String::new()]);
    let mut enabled = 0;
    let mut count = 0;
    for row in rows.try_iter()? {
        let row = row?;
        by_id.set_item(row.getattr("id")?, &row)?;
        let group: String = row.getattr("group")?.extract()?;
        if !group.is_empty() {
            let parts: Vec<_> = group.split('/').collect();
            for i in 1..=parts.len() {
                groups.insert(parts[..i].join("/"));
            }
        }
        let kind: String = row.getattr("kind")?.extract()?;
        if !["separator", "foreign"].contains(&kind.as_str()) {
            count += 1;
            if row.getattr("enabled")?.is_truthy()? {
                enabled += 1;
            }
        }
    }
    manager.setattr("rows_by_id", &by_id)?;
    manager.setattr("groups", PyList::new(py, groups)?)?;
    manager.setattr("enabled_count", enabled)?;
    manager.setattr("mod_count", count)?;
    for (attr, source) in [
        ("reference_undo", "reference_sources"),
        ("source_metadata_undo", "source_metadata"),
    ] {
        manager.setattr(
            attr,
            if has_library {
                module(py, source)?.call_method1("undoable", (&library, &profile_id))?
            } else {
                PySet::empty(py)?.into_any()
            },
        )?;
    }
    for (attr, method) in [
        ("download_records", "downloads"),
        ("recovery_history", "history"),
    ] {
        manager.setattr(
            attr,
            if has_library {
                library.call_method0(method)?
            } else {
                PyList::empty(py).into_any()
            },
        )?;
    }
    manager.setattr(
        "download_jobs",
        if has_library {
            module(py, "download_queue")?
                .getattr("DownloadQueue")?
                .call1((&library,))?
                .call_method0("records")?
        } else {
            PyList::empty(py).into_any()
        },
    )?;
    let instance_id = if let Some(profile) = &selected_profile {
        profile
            .getattr("files")?
            .call_method1("get", ("_modbox_instance_id", ""))?
    } else {
        "".into_pyobject(py)?.into_any()
    };
    let mut configuration = PyDict::new(py).into_any();
    if has_library && instance_id.is_truthy()? {
        for record in library.call_method0("configurations")?.try_iter()? {
            let record = record?;
            if record.get_item("id")?.eq(&instance_id)? {
                configuration = record.get_item("configuration")?;
                break;
            }
        }
    }
    if targeted {
        configuration = context
            .call_method1("profile_snapshot", (&profile_id,))?
            .get_item(2)?;
    }
    manager.setattr("instance_configuration", configuration)?;
    let executables = if has_library {
        library.call_method1("executables", (&instance_id,))?
    } else {
        PyList::empty(py).into_any()
    };
    manager.setattr(
        "executables",
        if has_library {
            module(py, "executable_catalog")?
                .call_method1("ordered", (&library, executables, instance_id))?
        } else {
            executables
        },
    )?;
    manager.setattr(
        "profile_backups",
        if selected {
            module(py, "profile_backup")?.call_method1("history", (&context, &profile_id))?
        } else {
            PyList::empty(py).into_any()
        },
    )?;
    if let Some(profile) = selected_profile {
        let domain = module(py, "nexus")?.call_method1(
            "game_domain",
            (profile
                .getattr("files")?
                .call_method1("get", ("_modbox_game", ""))?,),
        )?;
        if domain.is_truthy()? {
            manager.setattr("nexus_game", domain)?;
        }
    }
    if !by_id.contains(manager.getattr("selected_id")?)? {
        manager.setattr("selected_id", "")?;
    }
    manager.setattr("_filter_key", py.None())?;
    manager.call_method0("refresh_conflicts")?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(controller_context, m)?)?;
    m.add_function(wrap_pyfunction!(controller_save_context, m)?)?;
    m.add_function(wrap_pyfunction!(controller_runtime_active, m)?)?;
    m.add_function(wrap_pyfunction!(controller_select_profile, m)?)?;
    m.add_function(wrap_pyfunction!(controller_select_target, m)?)?;
    m.add_function(wrap_pyfunction!(controller_refresh, m)?)?;
    Ok(())
}
