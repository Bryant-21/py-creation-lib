use crate::{check_cancel, controller::callback, invalid};
use pyo3::{
    prelude::*,
    types::{PyDict, PySet, PyTuple},
};
fn module<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyModule>> {
    py.import(format!("creation_lib.mod_manager.{name}"))
}
fn mapping<'py>(value: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    if value.is_truthy()? {
        value
            .py()
            .import("builtins")?
            .call_method1("dict", (value,))
    } else {
        Ok(PyDict::new(value.py()).into_any())
    }
}
#[pyfunction]
fn controller_inspect_mo2(
    manager: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    source_mappings: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = manager.py();
    manager.setattr("import_preview", py.None())?;
    let path = path.clone().unbind();
    let mappings = mapping(source_mappings)?.unbind();
    let work = callback(py, move |py, args, _| {
        Ok(module(py, "mo2")?
            .call_method1(
                "inspect_instance",
                (path.bind(py), args.get_item(0)?, mappings.bind(py)),
            )?
            .unbind())
    })?;
    let owner = manager.clone().unbind();
    let complete = callback(py, move |py, args, _| {
        let preview = args.get_item(0)?;
        let manager = owner.bind(py);
        manager.setattr("import_preview", &preview)?;
        let count: String = py
            .import("builtins")?
            .call_method1("format", (preview.getattr("profiles")?.len()?, ","))?
            .extract()?;
        manager.setattr(
            "status",
            format!("Found {count} profiles. Review the import before adding them."),
        )?;
        Ok(py.None())
    })?;
    manager.call_method1("submit", ("Inspecting MO2 instance", work, complete))?;
    Ok(())
}
fn preferred(preview: &Bound<'_, PyAny>) -> PyResult<String> {
    let config = crate::plugin_inventory::value(&preview.getattr("configuration")?)?;
    Ok(config["game"]["selected_profile"]
        .as_str()
        .unwrap_or("")
        .to_string())
}
#[pyfunction]
fn controller_accept_migration(
    manager: &Bound<'_, PyAny>,
    result: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = manager.py();
    manager.setattr("instance_migration", result)?;
    let preview = result.getattr("preview")?;
    if preview.is_truthy()? {
        manager.setattr("import_preview", &preview)?;
    }
    let library = result.getattr("library")?;
    if !library.is_truthy()? {
        return Ok(());
    }
    manager.call_method0("discard_install")?;
    manager.setattr("library", &library)?;
    let mut name = String::new();
    let ids = PySet::empty(py)?;
    if preview.is_truthy()? {
        name = preferred(&preview)?;
        for profile in preview.getattr("profiles")?.try_iter()? {
            ids.add(profile?.getattr("id")?)?;
        }
    }
    let mut selected = String::new();
    let mut matched = false;
    for profile in library.call_method0("profiles")?.try_iter()? {
        let profile = profile?;
        if ids.contains(profile.getattr("id")?)? {
            let id: String = profile.getattr("id")?.extract()?;
            if selected.is_empty() {
                selected = id.clone();
            }
            if !matched && profile.getattr("name")?.eq(&name)? {
                selected = id;
                matched = true;
            }
        }
    }
    manager.setattr("profile_id", selected)?;
    manager.setattr("export_preview", py.None())?;
    manager.call_method0("refresh")?;
    Ok(())
}
#[pyfunction]
fn controller_migrate_mo2(
    manager: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    destination: &Bound<'_, PyAny>,
    source_mappings: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = manager.py();
    manager.call_method0("_assert_transfer_idle")?;
    if path.str()?.extract::<String>()?.trim().is_empty() {
        return Err(invalid("Choose an MO2 instance directory first."));
    }
    let target = manager
        .call_method1("resolve_destination", (destination,))?
        .unbind();
    let path = path.clone().unbind();
    let mappings = mapping(source_mappings)?.unbind();
    let owner = manager.clone().unbind();
    let work = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        Ok(module(py, "mo2_migration")?
            .call_method1(
                "migrate",
                (
                    path.bind(py),
                    target.bind(py),
                    manager.getattr("_validate_destination")?,
                    manager.getattr("_prepare_library")?,
                    args.get_item(0)?,
                    args.get_item(1)?,
                    mappings.bind(py),
                ),
            )?
            .unbind())
    })?;
    let owner = manager.clone().unbind();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let result = args.get_item(0)?;
        manager.call_method1("_accept_migration", (&result,))?;
        manager.setattr(
            "status",
            format!(
                "Import {}: {} source profiles, {} mod-folder references, {} unavailable.",
                result.getattr("state")?.str()?,
                result.getattr("original_profiles")?.str()?,
                result.getattr("referenced")?.len()?,
                result.getattr("unavailable")?.len()?
            ),
        )?;
        Ok(py.None())
    })?;
    manager.call_method1("submit", ("Migrating MO2 instance", work, complete))?;
    Ok(())
}
#[pyfunction]
fn controller_import_profiles(
    manager: &Bound<'_, PyAny>,
    selected: &Bound<'_, PyAny>,
    destination: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = manager.py();
    let library = manager.getattr("library")?;
    let preview = manager.getattr("import_preview")?;
    if !preview.is_truthy()? {
        return Err(invalid("Inspect an MO2 instance first."));
    }
    let selected = py.import("builtins")?.call_method1("set", (selected,))?;
    let ids = PySet::empty(py)?;
    for profile in preview.getattr("profiles")?.try_iter()? {
        ids.add(profile?.getattr("id")?)?;
    }
    if !selected.is_truthy()? || !selected.call_method1("issubset", (ids,))?.is_truthy()? {
        return Err(invalid(
            "Select at least one profile from the current import preview.",
        ));
    }
    let target = manager.call_method1("resolve_destination", (destination,))?;
    if !library.is_truthy()? || !target.eq(library.getattr("root")?)? {
        manager.call_method0("_assert_transfer_idle")?;
    }
    manager.call_method1("_validate_destination", (&target, &preview))?;
    let reuse = !library.is_none() && library.getattr("root")?.eq(&target)?;
    let owner = manager.clone().unbind();
    let captured_library = library.unbind();
    let captured_preview = preview.clone().unbind();
    let captured_selected = selected.clone().unbind();
    let captured_target = target.clone().unbind();
    let work = callback(py, move |py, args, _| {
        let cancel = args.get_item(0)?;
        let progress = args.get_item(1)?;
        let target = captured_target.bind(py);
        check_cancel(Some(&cancel))?;
        progress.call1((py.None(), format!("Preparing library at {}", target.str()?)))?;
        let library = if reuse {
            captured_library.bind(py).clone()
        } else {
            let kwargs = PyDict::new(py);
            kwargs.set_item("create", true)?;
            owner
                .bind(py)
                .call_method("_prepare_library", (target,), Some(&kwargs))?
        };
        check_cancel(Some(&cancel))?;
        let count = match library.call_method1(
            "import_mo2",
            (
                captured_preview.bind(py),
                captured_selected.bind(py),
                cancel,
                progress,
            ),
        ) {
            Ok(count) => count,
            Err(e)
                if e.is_instance_of::<pyo3::exceptions::PyOSError>(py)
                    || e.matches(py, &py.import("sqlite3")?.getattr("Error")?)? =>
            {
                return Err(invalid(format!(
                    "Import into '{}' failed: {}. Check permissions and available disk space. Uncommitted profile changes were rolled back; retry this destination or choose another.",
                    target.str()?,
                    e.value(py)
                )));
            }
            Err(e) => return Err(e),
        };
        Ok(PyTuple::new(py, [library, count])?.into_any().unbind())
    })?;
    let owner = manager.clone().unbind();
    let preview = preview.unbind();
    let selected = selected.unbind();
    let target = target.unbind();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let result = args.get_item(0)?;
        let library = result.get_item(0)?;
        let count = result.get_item(1)?;
        manager.setattr("library", &library)?;
        if !reuse {
            manager.call_method0("discard_install")?;
            manager.setattr("profile_id", "")?;
            manager.setattr("export_preview", py.None())?;
        }
        if count.is_truthy()? || library.getattr("last_import_repaired")?.is_truthy()? || !reuse {
            let preferred = preferred(preview.bind(py))?;
            let mut choice = None;
            let mut matched = false;
            for profile in preview.bind(py).getattr("profiles")?.try_iter()? {
                let profile = profile?;
                if selected.bind(py).contains(profile.getattr("id")?)? {
                    if choice.is_none() {
                        choice = Some(profile.getattr("id")?);
                    }
                    if !matched && profile.getattr("name")?.eq(&preferred)? {
                        choice = Some(profile.getattr("id")?);
                        matched = true;
                    }
                }
            }
            if let Some(choice) = choice {
                manager.setattr("profile_id", choice)?;
            }
        }
        manager.call_method0("refresh")?;
        let fmt = py.import("builtins")?.getattr("format")?;
        manager.setattr("status", format!("Imported {} new profiles; {} entries in the selected profile. Updated {} source mappings. Destination: {}. Mod files remain read-only references; existing organization was preserved.", fmt.call1((count, ","))?.str()?, fmt.call1((manager.getattr("rows")?.len()?, ","))?.str()?, fmt.call1((library.getattr("last_import_repaired")?, ","))?.str()?, target.bind(py).str()?))?;
        Ok(py.None())
    })?;
    manager.call_method1(
        "submit",
        (
            "Preparing destination and importing profiles",
            work,
            complete,
        ),
    )?;
    Ok(())
}
#[pyfunction]
fn controller_create_instance(
    manager: &Bound<'_, PyAny>,
    game_id: &str,
    game_root: &Bound<'_, PyAny>,
    destination: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<()> {
    let py = manager.py();
    manager.call_method0("_assert_transfer_idle")?;
    let target = manager.call_method1("resolve_destination", (destination,))?;
    module(py, "instances")?.call_method1("validate_root", (game_id, game_root, &target))?;
    let owner = manager.clone().unbind();
    let target = target.unbind();
    let root = game_root.clone().unbind();
    let game = game_id.to_string();
    let name = name.to_string();
    let work = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let library = manager.getattr("library")?;
        let target = target.bind(py);
        let library = if library.is_truthy()? && library.getattr("root")?.eq(target)? {
            library
        } else {
            let kwargs = PyDict::new(py);
            kwargs.set_item("create", true)?;
            manager.call_method("_prepare_library", (target,), Some(&kwargs))?
        };
        let profile = module(py, "instances")?.call_method1(
            "create",
            (&library, &game, root.bind(py), &name, args.get_item(0)?),
        )?;
        Ok(PyTuple::new(py, [library, profile])?.into_any().unbind())
    })?;
    let owner = manager.clone().unbind();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let result = args.get_item(0)?;
        manager.setattr("library", result.get_item(0)?)?;
        manager.setattr("profile_id", result.get_item(1)?)?;
        manager.call_method0("refresh")?;
        manager.setattr(
            "status",
            "Standalone instance ready. Install a package or configure your local profile.",
        )?;
        Ok(py.None())
    })?;
    manager.call_method1("submit", ("Creating standalone instance", work, complete))?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(controller_inspect_mo2, m)?)?;
    m.add_function(wrap_pyfunction!(controller_accept_migration, m)?)?;
    m.add_function(wrap_pyfunction!(controller_migrate_mo2, m)?)?;
    m.add_function(wrap_pyfunction!(controller_import_profiles, m)?)?;
    m.add_function(wrap_pyfunction!(controller_create_instance, m)?)?;
    Ok(())
}
