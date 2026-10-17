use crate::{
    controller::{callback, submit_call},
    invalid,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};

fn function<'py>(py: Python<'py>, module: &str, name: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import(format!("creation_lib.mod_manager.{module}"))?
        .getattr(name)
}
#[pyfunction]
fn controller_selected_save(
    manager: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    header: bool,
) -> PyResult<Py<PyAny>> {
    let index = manager.getattr("save_index")?;
    if !index.is_truthy()?
        || !index
            .getattr("profile_id")?
            .eq(manager.getattr("profile_id")?)?
        || index.getattr("game")?.extract::<String>()? != "fallout4"
    {
        return Err(invalid(if header {
            "Refresh a Fallout 4 profile's owned save inventory first. Other header adapters remain unavailable."
        } else {
            "Refresh this Fallout 4 profile's save inventory first."
        }));
    }
    for save in index.getattr("files")?.try_iter()? {
        let save = save?;
        if save.getattr("path")?.eq(path)? {
            return Ok(save.unbind());
        }
    }
    Err(invalid(if header {
        "Select an owned save from the current inventory."
    } else {
        "Select a save from the current inventory."
    }))
}
fn remember(
    manager: &Bound<'_, PyAny>,
    field: &str,
    path: &Bound<'_, PyAny>,
    result: &Bound<'_, PyAny>,
    limit: usize,
) -> PyResult<Py<PyAny>> {
    Ok(function(manager.py(), "metadata_cache", "remember")?
        .call1((manager.getattr(field)?, path, result, limit))?
        .unbind())
}

#[pyfunction]
fn controller_save_action(
    manager: &Bound<'_, PyAny>,
    action: &str,
    args: &Bound<'_, PyTuple>,
) -> PyResult<()> {
    let py = manager.py();
    let library = manager.getattr(
        if ["inspect_global_saves", "compare_save_profile"].contains(&action) {
            "context_library"
        } else {
            "save_library"
        },
    )?;
    let profile = manager.getattr("profile_id")?;
    let options = PyDict::new(py);
    let mut progress = false;
    let path = if args.is_empty() {
        py.None()
    } else {
        args.get_item(0)?.unbind()
    };
    let (label, operation, inputs) = match action {
        "inspect_saves"
        | "inspect_source_saves"
        | "inspect_global_saves"
        | "inspect_save_backups" => {
            if !library.is_truthy()? || !profile.is_truthy()? {
                return Err(invalid(if action == "inspect_global_saves" {
                    "Choose a local Fallout 4 profile first."
                } else {
                    "Choose a local profile first."
                }));
            }
            progress = true;
            let mut inputs = vec![library.clone(), profile.clone()];
            if action == "inspect_source_saves" {
                inputs.push(args.get_item(0)?);
            }
            let (label, module, name) = match action {
                "inspect_saves" => ("Reading owned save inventory", "saves", "inventory"),
                "inspect_source_saves" => (
                    "Reading selected source save folder",
                    "saves",
                    "inventory_folder",
                ),
                "inspect_global_saves" => (
                    "Reading default global FO4 saves",
                    "global_saves",
                    "inventory",
                ),
                _ => ("Reading owned save backups", "save_backups", "inventory"),
            };
            (label, function(py, module, name)?, inputs)
        }
        "review_save_copy" => {
            let index = manager.getattr("save_index")?;
            if !library.is_truthy()?
                || !index.is_truthy()?
                || !index.getattr("profile_id")?.eq(&profile)?
            {
                return Err(invalid(
                    "Refresh this local profile's displayed save inventory first.",
                ));
            }
            manager.setattr("save_transfer_plan", py.None())?;
            (
                "Reviewing save and companion",
                function(py, "save_transfer", "review")?,
                vec![library.clone(), index, args.get_item(0)?, args.get_item(1)?],
            )
        }
        "apply_save_copy" => {
            let plan = manager.getattr("save_transfer_plan")?;
            if !library.is_truthy()?
                || !plan.is_truthy()?
                || !plan.getattr("source_profile")?.eq(&profile)?
            {
                return Err(invalid("Review this profile's owned save copy first."));
            }
            progress = true;
            (
                "Copying reviewed owned save pair",
                function(py, "save_transfer", "apply")?,
                vec![library.clone(), plan],
            )
        }
        "review_save_backup" => {
            let mut available = false;
            for item in manager.getattr("save_backups")?.try_iter()? {
                let item = item?;
                if item.getattr("id")?.eq(args.get_item(0)?)?
                    && item.getattr("profile")?.eq(&profile)?
                {
                    available = true;
                    break;
                }
            }
            if !library.is_truthy()? || !available {
                return Err(invalid(
                    "Refresh and select an owned save backup for this profile first.",
                ));
            }
            manager.setattr("save_transfer_plan", py.None())?;
            (
                "Reviewing save backup restore",
                function(py, "save_backups", "review")?,
                vec![
                    library.clone(),
                    profile.clone(),
                    args.get_item(0)?,
                    profile.clone(),
                ],
            )
        }
        "inspect_save_header" | "inspect_save_plugins" => {
            let save =
                controller_selected_save(manager, path.bind(py), action == "inspect_save_header")?
                    .into_bound(py);
            if action == "inspect_save_header" {
                (
                    "Inspecting owned save header",
                    function(py, "saves", "fo4_header")?,
                    vec![save],
                )
            } else {
                (
                    "Inspecting save plugin tables",
                    manager.getattr("extensions")?.getattr("inspect_save")?,
                    vec!["fo4".into_pyobject(py)?.into_any(), save],
                )
            }
        }
        "preview_save_screenshot" => {
            controller_selected_save(manager, path.bind(py), false)?;
            let saved = manager
                .getattr("save_inspections")?
                .call_method1("get", (path.bind(py),))?;
            if saved.is_none() {
                return Err(invalid(
                    "Inspect this save's metadata before previewing its screenshot.",
                ));
            }
            (
                "Reading inspected save screenshot",
                function(py, "save_inspection", "screenshot")?,
                vec![saved],
            )
        }
        "compare_save_to" | "compare_save_profile" => {
            controller_selected_save(manager, path.bind(py), false)?;
            let saved = manager
                .getattr("save_inspections")?
                .call_method1("get", (path.bind(py),))?;
            if action == "compare_save_to" {
                let other = controller_selected_save(manager, &args.get_item(1)?, false)?;
                if saved.is_none() || path.bind(py).eq(args.get_item(1)?)? {
                    return Err(invalid(
                        "Inspect this save and choose a different comparison save.",
                    ));
                }
                let saved = saved.unbind();
                let extensions = manager.getattr("extensions")?.unbind();
                let work = callback(py, move |py, _, kwargs| {
                    let other = extensions
                        .bind(py)
                        .getattr("inspect_save")?
                        .call(("fo4", other.bind(py)), kwargs)?;
                    Ok(function(py, "save_inspection", "compare_saved")?
                        .call((saved.bind(py), other), kwargs)?
                        .unbind())
                })?;
                (
                    "Comparing two saved plugin tables",
                    work.into_bound(py),
                    vec![],
                )
            } else {
                if saved.is_none() {
                    return Err(invalid("Inspect this save's plugin table first."));
                }
                progress = true;
                let saved = saved.unbind();
                let source = library.clone().unbind();
                let selected = profile.clone().unbind();
                let work = callback(py, move |py, _, kwargs| {
                    let cancel = kwargs
                        .and_then(|k| k.get_item("cancel").transpose())
                        .transpose()?
                        .unwrap_or_else(|| py.None().into_bound(py));
                    let library = source.bind(py);
                    let profile = selected.bind(py);
                    function(py, "save_inspection", "validate")?
                        .call1((saved.bind(py), &cancel))?;
                    let inventory = function(py, "plugin_state", "inventory")?;
                    let catalog = function(py, "plugin_state", "inspect")?.call(
                        (
                            inventory.call((library, profile), kwargs)?,
                            function(py, "plugin_state", "CachedMetadata")?.call1((library,))?,
                        ),
                        kwargs,
                    )?;
                    let signature = function(py, "archive_index", "plugin_signature")?;
                    if !signature
                        .call1((inventory.call1((library, profile, &cancel))?,))?
                        .eq(signature.call1((&catalog,))?)?
                    {
                        return Err(invalid(
                            "Plugin providers or activation changed during save comparison. Refresh it.",
                        ));
                    }
                    if !function(py, "plugin_state", "signature")?
                        .call1((library, profile))?
                        .eq(catalog.getattr("signature")?)?
                    {
                        return Err(invalid(
                            "Profile changed during save comparison. Refresh and compare again.",
                        ));
                    }
                    for plugin in catalog.getattr("plugins")?.try_iter()? {
                        crate::check_cancel(Some(&cancel))?;
                        let plugin = plugin?;
                        let path = plugin.getattr("path")?;
                        if path.is_truthy()? {
                            let path = path.extract::<std::path::PathBuf>()?;
                            crate::reject_links(&path)?;
                            let info =
                                crate::history_prune::py_path(py, &path)?.call_method0("stat")?;
                            let fingerprint =
                                (info.getattr("st_size")?, info.getattr("st_mtime_ns")?)
                                    .into_pyobject(py)?;
                            if !fingerprint.as_any().eq(plugin.getattr("fingerprint")?)? {
                                return Err(invalid(
                                    "Plugin changed during save comparison. Refresh and compare again.",
                                ));
                            }
                        }
                    }
                    Ok(function(py, "save_inspection", "compare_profile")?
                        .call1((saved.bind(py), catalog, cancel))?
                        .unbind())
                })?;
                (
                    "Comparing save with local plugins",
                    work.into_bound(py),
                    vec![],
                )
            }
        }
        _ => return Err(invalid("Unknown save controller operation.")),
    };
    let owner = manager.clone().unbind();
    let source = library.unbind();
    let selected = profile.unbind();
    let action = action.to_string();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let result = args.get_item(0)?;
        let status = match action.as_str() {
            "inspect_saves" | "inspect_source_saves" | "inspect_global_saves" => {
                if action == "inspect_global_saves"
                    && (!manager.getattr("context_library")?.is(source.bind(py))
                        || !manager.getattr("profile_id")?.eq(selected.bind(py))?)
                {
                    return Err(invalid(
                        "The local profile changed during global save discovery. Select it again.",
                    ));
                }
                manager.setattr("save_index", &result)?;
                for key in [
                    "save_details",
                    "save_inspections",
                    "save_comparisons",
                    "save_images",
                ] {
                    manager.setattr(key, PyDict::new(py))?;
                }
                if action != "inspect_saves" {
                    manager.setattr("save_transfer_plan", py.None())?;
                }
                let count = result
                    .getattr("files")?
                    .len()?
                    .into_pyobject(py)?
                    .call_method1("__format__", (",",))?
                    .extract::<String>()?;
                match action.as_str() {
                    "inspect_saves" => {
                        format!("Indexed {count} owned saves. Header inspection is separate.")
                    }
                    "inspect_source_saves" => format!(
                        "Indexed {count} source saves read only. Copies go into owned library folders."
                    ),
                    _ => format!(
                        "Read {count} default global FO4 saves without changing them. Use Browse for customized locations."
                    ),
                }
            }
            "review_save_copy" | "review_save_backup" => {
                manager.setattr("save_transfer_plan", result)?;
                if action == "review_save_copy" {"Reviewed save copy. Confirm it before creating owned destination files."} else {"Reviewed retained save pair. Restore creates new owned files and preserves the backup."}.into()
            }
            "apply_save_copy" => {
                manager.setattr("save_transfer_plan", py.None())?;
                manager.setattr("save_index", py.None())?;
                manager.setattr("save_details", PyDict::new(py))?;
                format!(
                    "Copied owned save pair to {}. Plugin compatibility remains unverified.",
                    result.str()?
                )
            }
            "inspect_save_backups" => {
                manager.setattr("save_backups", &result)?;
                manager.setattr("save_transfer_plan", py.None())?;
                format!(
                    "Read {} retained owned save backup receipts. Restore checks their bytes separately.",
                    result
                        .len()?
                        .into_pyobject(py)?
                        .call_method1("__format__", (",",))?
                        .extract::<String>()?
                )
            }
            "inspect_save_header" => {
                remember(manager, "save_details", path.bind(py), &result, 32)?;
                "Read basic Fallout 4 save header. Plugin compatibility remains unverified.".into()
            }
            "inspect_save_plugins" => {
                for removed in remember(manager, "save_inspections", path.bind(py), &result, 8)?
                    .bind(py)
                    .try_iter()?
                {
                    let removed = removed?;
                    for key in ["save_comparisons", "save_images"] {
                        manager
                            .getattr(key)?
                            .call_method1("pop", (&removed, py.None()))?;
                    }
                }
                remember(
                    manager,
                    "save_details",
                    path.bind(py),
                    &result.getattr("basic")?,
                    32,
                )?;
                for key in ["save_comparisons", "save_images"] {
                    manager
                        .getattr(key)?
                        .call_method1("pop", (path.bind(py), py.None()))?;
                }
                "Read FO4 regular/light plugin tables. Compare explicitly; save-body validity remains unverified.".into()
            }
            "preview_save_screenshot" => {
                remember(manager, "save_images", path.bind(py), &result, 4)?;
                "Read bounded FO4 screenshot preview; save and companion were not changed.".into()
            }
            "compare_save_profile" | "compare_save_to" => {
                remember(manager, "save_comparisons", path.bind(py), &result, 8)?;
                if action == "compare_save_profile" {"Compared saved plugins with the current local profile. Differences are metadata warnings."} else {"Compared two save plugin tables. Neither file was changed."}.into()
            }
            _ => unreachable!(),
        };
        manager.setattr("status", status)?;
        Ok(py.None())
    })?;
    submit_call(
        manager,
        label,
        &operation,
        &PyTuple::new(py, inputs)?,
        &options,
        true,
        progress,
        complete.bind(py),
    )
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(controller_save_action, m)?)?;
    m.add_function(wrap_pyfunction!(controller_selected_save, m)?)?;
    Ok(())
}
