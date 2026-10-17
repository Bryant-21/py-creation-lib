use crate::{
    controller::{callback, submit_call},
    invalid,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
fn function<'py>(py: Python<'py>, module: &str, name: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import(format!("creation_lib.mod_manager.{module}"))?
        .getattr(name)
}
#[pyfunction]
fn controller_profile_action(
    manager: &Bound<'_, PyAny>,
    action: &str,
    args: &Bound<'_, PyTuple>,
    changes: &Bound<'_, PyDict>,
) -> PyResult<Py<PyAny>> {
    let py = manager.py();
    let library = manager.getattr("library")?;
    let profile = manager.getattr("profile_id")?;
    let context = manager.getattr("context_library")?;
    let options = changes.copy()?;
    let mut progress = false;
    let mut cancel = true;
    let mut count = 0;
    if ["customize", "bulk_organize"].contains(&action)
        && manager.getattr("target_id")?.is_truthy()?
        && changes.len() == 1
        && changes.contains("enabled")?
    {
        let selected = if action == "customize" {
            PyTuple::new(py, [args.get_item(0)?])?.into_any()
        } else {
            py.import("builtins")?
                .call_method1("tuple", (args.get_item(0)?,))?
        };
        return Ok(manager
            .call_method1(
                "set_target_enabled",
                (selected, changes.get_item("enabled")?.unwrap()),
            )?
            .unbind());
    }
    let (label, status, operation, inputs) = match action {
        "create_profile" => {
            if !library.is_truthy()? {
                return Err(invalid("Open a library first."));
            }
            let source = if args.get_item(1)?.is_truthy()? {
                profile.clone()
            } else {
                "".into_pyobject(py)?.into_any()
            };
            options.set_item("template", &profile)?;
            (
                "Creating profile",
                "Profile created.",
                library.getattr("create_profile")?,
                vec![args.get_item(0)?, source],
            )
        }
        "delete_profile" => {
            let wanted = args.get_item(0)?;
            let mut found = false;
            if library.is_truthy()? {
                for p in manager.getattr("profiles")?.try_iter()? {
                    if p?.getattr("id")?.eq(&wanted)? {
                        found = true;
                        break;
                    }
                }
            }
            if !found {
                return Err(invalid("Select a local library profile to remove."));
            }
            (
                "Removing local profile metadata",
                "Local profile removed. Restore it from Profiles; MO2 folders stay unchanged.",
                library.getattr("delete_profile")?,
                vec![wanted],
            )
        }
        "restore_profile" => {
            if !library.is_truthy()? {
                return Err(invalid("Open a library first."));
            }
            (
                "Restoring local profile metadata",
                "Local profile restored.",
                library.getattr("restore_profile")?,
                vec![args.get_item(0)?],
            )
        }
        "rename_profile" => {
            if !library.is_truthy()? || !profile.is_truthy()? {
                return Err(invalid("Select a local profile first."));
            }
            (
                "Renaming local profile",
                "Local profile renamed. Its stable identity and source folders stay unchanged.",
                function(py, "profile_state", "rename")?,
                vec![library.clone(), profile.clone(), args.get_item(0)?],
            )
        }
        "save_profile_ini" => (
            "Saving local profile INI",
            "Profile INI saved with a local metadata backup. Native launch will use the edited local settings.",
            function(py, "profile_state", "save_ini")?,
            vec![
                context.clone(),
                profile.clone(),
                args.get_item(0)?,
                args.get_item(1)?,
                args.get_item(2)?,
            ],
        ),
        "restore_profile_ini" => (
            "Restoring local INI snapshot",
            "Previous local INI restored; the replaced snapshot remains available for recovery.",
            function(py, "profile_state", "restore_ini")?,
            vec![
                context.clone(),
                profile.clone(),
                args.get_item(0)?,
                args.get_item(1)?,
            ],
        ),
        "backup_profile" => (
            "Backing up local profile metadata",
            "Local profile metadata backed up. Mod payloads were not scanned or copied.",
            function(py, "profile_backup", "capture")?,
            vec![context.clone(), profile.clone()],
        ),
        "export_recipe" => {
            progress = true;
            (
                "Exporting profile recipe",
                "Portable profile recipe exported into the owned exports folder. No payloads copied.",
                function(py, "profile_recipes", "export")?,
                vec![library.clone(), profile.clone()],
            )
        }
        "review_recipe" => {
            let recipe = manager.getattr("recipe")?;
            if !recipe.is_truthy()? {
                return Err(invalid("Read a profile recipe first."));
            }
            (
                "Reviewing profile recipe mapping",
                "",
                function(py, "profile_recipes", "review")?,
                vec![
                    library.clone(),
                    profile.clone(),
                    recipe,
                    py.import("builtins")?
                        .call_method1("dict", (args.get_item(0)?,))?,
                    args.get_item(1)?,
                ],
            )
        }
        "apply_recipe" => {
            let plan = manager.getattr("recipe_plan")?;
            if !plan.is_truthy()? || !plan.getattr("template")?.eq(&profile)? {
                return Err(invalid("Review this profile transfer first."));
            }
            (
                "Importing reviewed profile recipe",
                "",
                function(py, "profile_recipes", "apply")?,
                vec![library.clone(), plan],
            )
        }
        "review_profile_backup" => (
            "Reviewing profile metadata backup",
            "Review the local profile state before restoring this backup.",
            function(py, "profile_backup", "review")?,
            vec![context.clone(), profile.clone(), args.get_item(0)?],
        ),
        "restore_profile_backup" => {
            let plan = manager.getattr("profile_restore")?;
            if !plan.is_truthy()? {
                return Err(invalid("Review a local profile backup first."));
            }
            (
                "Restoring reviewed profile metadata",
                "Local profile metadata restored. The previous state is retained as another backup.",
                function(py, "profile_backup", "apply")?,
                vec![context.clone(), plan],
            )
        }
        "remove_profile_backup" => (
            "Removing local metadata backup",
            "Selected local metadata backup removed.",
            function(py, "profile_backup", "remove")?,
            vec![context.clone(), profile.clone(), args.get_item(0)?],
        ),
        "customize" => {
            cancel = false;
            (
                "Saving organization",
                "Organization saved to your library.",
                library.getattr("customize")?,
                vec![profile.clone(), args.get_item(0)?],
            )
        }
        "reorder" => {
            options.set_item("priority", args.get_item(1)?)?;
            (
                "Saving priority",
                "Priority saved; higher priority wins file conflicts.",
                function(py, "organization", "bulk")?,
                vec![
                    library.clone(),
                    profile.clone(),
                    PyList::new(py, [args.get_item(0)?])?.into_any(),
                ],
            )
        }
        "bulk_organize" => {
            let selected = py
                .import("builtins")?
                .call_method1("tuple", (args.get_item(0)?,))?;
            count = selected.len()?;
            (
                "Saving selected mod organization",
                "",
                function(py, "organization", "bulk")?,
                vec![library.clone(), profile.clone(), selected],
            )
        }
        "set_target_enabled" => (
            "Saving target-local enable overrides",
            "Target-local enable overrides saved; shared profile priorities and other targets are unchanged. Reset through Mod source for this target.",
            function(py, "target_mod_sources", "set_enabled")?,
            vec![
                library.clone(),
                manager.getattr("target_id")?,
                profile.clone(),
                args.get_item(0)?,
                args.get_item(1)?,
            ],
        ),
        "undo_organization" => (
            "Undoing local organization",
            "Previous local organization restored.",
            function(py, "organization", "undo")?,
            vec![library.clone(), profile.clone()],
        ),
        "create_empty_mod" => (
            "Creating empty mod",
            "Empty mod created. Open its folder to add files.",
            function(py, "organization", "empty_mod")?,
            vec![
                library.clone(),
                profile.clone(),
                args.get_item(0)?,
                args.get_item(1)?,
            ],
        ),
        "create_group" => (
            "Creating organization group",
            "Organization group created.",
            function(py, "organization", "separator")?,
            vec![
                library.clone(),
                profile.clone(),
                args.get_item(0)?,
                args.get_item(1)?,
            ],
        ),
        "review_download" => {
            if !context.is_truthy()? || !profile.is_truthy()? {
                return Err(invalid(
                    "Choose a local profile before installing a download.",
                ));
            }
            manager.setattr("download_review", py.None())?;
            (
                "Reviewing downloaded archive and local destinations",
                "Choose a new mod or an explicit owned reinstall destination. Original MO2 archives and source mods stay in place.",
                function(py, "download_install", "capture")?,
                vec![
                    context.clone(),
                    profile.clone(),
                    args.get_item(0)?,
                    args.get_item(1)?,
                    manager.getattr("nexus_states")?.call_method0("copy")?,
                ],
            )
        }
        _ => return Err(invalid("Unknown profile controller operation.")),
    };
    let owner = manager.clone().unbind();
    let action = action.to_string();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let result = args.get_item(0)?;
        let mut message = status.to_string();
        let mut refresh = true;
        match action.as_str() {
            "create_profile" | "restore_profile" => manager.setattr("profile_id", &result)?,
            "delete_profile" => {
                manager.call_method0("discard_install")?;
                manager.setattr("export_preview", py.None())?;
            }
            "export_recipe" => {
                manager.setattr("recipe_export", result.str()?)?;
                refresh = false;
            }
            "review_recipe" => {
                manager.setattr("recipe_plan", &result)?;
                refresh = false;
                message = if result.getattr("existing")?.is_truthy()? {
                    "Existing transferred profile will be preserved."
                } else {
                    "Profile transfer reviewed. Confirm to create local metadata."
                }
                .into();
            }
            "apply_recipe" => {
                manager.setattr("profile_id", result.get_item(0)?)?;
                message = if result.get_item(1)?.is_truthy()? {
                    "Transferred profile created; source profiles and mods remain intact."
                } else {
                    "Repeated transfer preserved the existing local profile and its edits."
                }
                .into();
            }
            "review_profile_backup" => {
                manager.setattr("profile_restore", result)?;
                refresh = false;
            }
            "bulk_organize" => {
                let count: String = py
                    .import("builtins")?
                    .call_method1("format", (count, ","))?
                    .extract()?;
                message =
                    format!("Saved organization for {count} selected mods. Undo is available.");
            }
            "create_group" | "create_empty_mod" => manager.setattr("selected_id", result)?,
            "review_download" => {
                manager.setattr("download_review", result)?;
                refresh = false;
            }
            _ => (),
        }
        if refresh {
            manager.call_method0("refresh")?;
        }
        if ["customize", "reorder"].contains(&action.as_str()) {
            manager.setattr("export_preview", py.None())?;
        }
        manager.setattr("status", message)?;
        Ok(py.None())
    })?;
    submit_call(
        manager,
        label,
        &operation,
        &PyTuple::new(py, inputs)?,
        &options,
        cancel,
        progress,
        complete.bind(py),
    )?;
    Ok(py.None())
}
#[pyfunction]
fn controller_inspect_recipe(manager: &Bound<'_, PyAny>, path: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = manager.py();
    let library = manager.getattr("library")?.unbind();
    let profile = manager.getattr("profile_id")?.unbind();
    let path = path.clone().unbind();
    for key in ["recipe", "recipe_plan"] {
        manager.setattr(key, py.None())?;
    }
    manager.setattr("recipe_candidates", PyTuple::empty(py))?;
    let work = callback(py, move |py, args, _| {
        let recipe =
            function(py, "profile_recipes", "read")?.call1((path.bind(py), args.get_item(0)?))?;
        let suggestions = function(py, "profile_recipes", "suggestions")?.call1((
            library.bind(py),
            profile.bind(py),
            &recipe,
        ))?;
        let candidates = function(py, "profile_recipes", "candidates")?
            .call1((library.bind(py), recipe.getattr("game")?))?;
        Ok(PyTuple::new(py, [recipe, suggestions, candidates])?
            .into_any()
            .unbind())
    })?;
    let owner = manager.clone().unbind();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let result = args.get_item(0)?;
        for (index, key) in ["recipe", "recipe_suggestions", "recipe_candidates"]
            .iter()
            .enumerate()
        {
            manager.setattr(*key, result.get_item(index)?)?;
        }
        manager.setattr(
            "status",
            "Recipe read. Map every payload to an existing local mod, then review a new profile.",
        )?;
        Ok(py.None())
    })?;
    manager.call_method1("submit", ("Reading profile recipe", work, complete))?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(controller_profile_action, m)?)?;
    m.add_function(wrap_pyfunction!(controller_inspect_recipe, m)?)?;
    Ok(())
}
