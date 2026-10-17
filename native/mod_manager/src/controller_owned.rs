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
fn reviewed<'py>(
    manager: &Bound<'py, PyAny>,
    field: &str,
    message: &str,
    profile: bool,
    library: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let plan = manager.getattr(field)?;
    if !plan.is_truthy()?
        || library && !manager.getattr("library")?.is_truthy()?
        || profile
            && !plan
                .getattr("profile_id")?
                .eq(manager.getattr("profile_id")?)?
    {
        return Err(invalid(message));
    }
    Ok(plan)
}
#[pyfunction]
fn controller_owned_action(
    manager: &Bound<'_, PyAny>,
    action: &str,
    args: &Bound<'_, PyTuple>,
) -> PyResult<()> {
    let py = manager.py();
    let library = manager.getattr("library")?;
    let profile = manager.getattr("profile_id")?;
    let options = PyDict::new(py);
    let mut progress = false;
    let mut output = "";
    let mut refresh = false;
    let (label, message, operation, inputs) = match action {
        "review_owned" => {
            if !library.is_truthy()? {
                return Err(invalid("Open a library first."));
            }
            for key in ["lifecycle_review", "restore_preview"] {
                manager.setattr(key, py.None())?;
            }
            output = "lifecycle_review";
            (
                "Reviewing owned content and profile references",
                "Review affected profiles and file changes before uninstalling. All content will be retained in recovery history.",
                library.getattr("review_owned")?,
                vec![args.get_item(0)?],
            )
        }
        "inspect_mod_name" => {
            output = "name_review";
            (
                "Reviewing mod display title",
                "Display-title target reviewed. Source folders will remain intact.",
                function(py, "mod_names", "review")?,
                vec![library.clone(), profile.clone(), args.get_item(0)?],
            )
        }
        "review_reference_source" => {
            output = "relink_review";
            (
                "Reviewing referenced source inventory",
                "Source reviewed. Confirm mod identity, apply or cancel.",
                function(py, "reference_sources", "review")?,
                vec![
                    library.clone(),
                    profile.clone(),
                    args.get_item(0)?,
                    args.get_item(1)?,
                ],
            )
        }
        "relink_source" => {
            let plan = reviewed(
                manager,
                "relink_review",
                "Review the referenced source for this profile first.",
                true,
                false,
            )?;
            refresh = true;
            options.set_item("accept_identity", args.get_item(0)?)?;
            (
                "Relinking shared source metadata",
                "Shared reference relinked. Source files remain intact; metadata Undo is available.",
                function(py, "reference_sources", "apply")?,
                vec![library.clone(), plan],
            )
        }
        "undo_source_relink" => {
            refresh = true;
            (
                "Undoing reference source metadata",
                "Restored previous reference source metadata. Source folders remain intact.",
                function(py, "reference_sources", "undo")?,
                vec![library.clone(), profile.clone(), args.get_item(0)?],
            )
        }
        "inspect_mod_field" => {
            output = "field_review";
            (
                "Reviewing local mod field",
                "Local field reviewed. Save or cancel its edit.",
                function(py, "row_edit", "review")?,
                vec![
                    library.clone(),
                    profile.clone(),
                    args.get_item(0)?,
                    args.get_item(1)?,
                ],
            )
        }
        "review_source_metadata" => {
            output = "source_metadata_review";
            (
                "Reviewing referenced source metadata",
                "Source metadata reviewed. Choose individual fields to refresh.",
                function(py, "source_metadata", "review")?,
                vec![
                    library.clone(),
                    profile.clone(),
                    args.get_item(0)?,
                    args.get_item(1)?,
                ],
            )
        }
        "refresh_source_metadata" => {
            let plan = reviewed(
                manager,
                "source_metadata_review",
                "Review source metadata for this local profile first.",
                true,
                false,
            )?;
            refresh = true;
            let selected = py
                .import("builtins")?
                .call_method1("tuple", (args.get_item(0)?,))?;
            (
                "Refreshing selected source metadata",
                "Selected source fields refreshed across local profiles. Metadata Undo is available.",
                function(py, "source_metadata", "apply")?,
                vec![library.clone(), plan, selected],
            )
        }
        "undo_source_metadata" => {
            refresh = true;
            (
                "Undoing source metadata refresh",
                "Restored local values from before the source metadata refresh.",
                function(py, "source_metadata", "undo")?,
                vec![library.clone(), profile.clone(), args.get_item(0)?],
            )
        }
        "edit_mod_field" => {
            let plan = reviewed(
                manager,
                "field_review",
                "Review this local mod field first.",
                true,
                false,
            )?;
            refresh = true;
            (
                "Saving local mod field",
                "Local field saved. Source files remain intact.",
                function(py, "row_edit", "apply")?,
                vec![library.clone(), plan, args.get_item(0)?],
            )
        }
        "inspect_group_name" => {
            output = "group_review";
            (
                "Reviewing local hierarchy",
                "Local hierarchy reviewed. Rename or cancel.",
                function(py, "group_names", "review")?,
                vec![library.clone(), profile.clone(), args.get_item(0)?],
            )
        }
        "rename_group" => {
            let plan = reviewed(
                manager,
                "group_review",
                "Review this profile's group first.",
                true,
                false,
            )?;
            refresh = true;
            (
                "Renaming local hierarchy",
                "Local group and descendants renamed. Undo is available.",
                function(py, "group_names", "apply")?,
                vec![library.clone(), plan, args.get_item(0)?],
            )
        }
        "rename_mod_name" => {
            let plan = reviewed(
                manager,
                "name_review",
                "Review this local mod title first.",
                true,
                false,
            )?;
            refresh = true;
            (
                "Saving mod display title",
                "Shared display title saved. Source identity and folders are intact.",
                function(py, "mod_names", "apply")?,
                vec![library.clone(), plan, args.get_item(0)?],
            )
        }
        "uninstall" => {
            let review = reviewed(
                manager,
                "lifecycle_review",
                "Review the owned mod before uninstalling.",
                false,
                true,
            )?;
            refresh = true;
            (
                "Uninstalling owned mod into recovery history",
                "Uninstalled from all profiles. A recoverable copy is available in Recovery.",
                library.getattr("uninstall")?,
                vec![review],
            )
        }
        "review_restore" => {
            if !library.is_truthy()? {
                return Err(invalid("Open a library first."));
            }
            for key in ["restore_preview", "lifecycle_review", "prune_review"] {
                manager.setattr(key, py.None())?;
            }
            (
                "Reviewing recovery entry",
                "Review restoration. An existing installed version will also be backed up.",
                library.getattr("review_restore")?,
                vec![args.get_item(0)?],
            )
        }
        "restore" => {
            let entry = reviewed(
                manager,
                "restore_preview",
                "Review a recovery entry before restoring.",
                false,
                true,
            )?;
            refresh = true;
            progress = true;
            options.set_item("backup_token", entry.get_item("backup_token")?)?;
            (
                "Restoring owned content from recovery history",
                "Restored owned content. Existing backups were retained.",
                library.getattr("restore")?,
                vec![entry.get_item("id")?, manager.getattr("lifecycle_review")?],
            )
        }
        "review_prune" => {
            if !library.is_truthy()? {
                return Err(invalid("Open a library first."));
            }
            for key in ["prune_review", "restore_preview", "lifecycle_review"] {
                manager.setattr(key, py.None())?;
            }
            progress = true;
            output = "prune_review";
            (
                "Reviewing recovery-backup deletion",
                "Review permanent deletion of this one owned recovery backup. Installed mods stay intact.",
                function(py, "history_prune", "review")?,
                vec![library.clone(), args.get_item(0)?],
            )
        }
        "prune_backup" => {
            let plan = reviewed(
                manager,
                "prune_review",
                "Review a recovery backup before deleting it.",
                false,
                true,
            )?;
            refresh = true;
            progress = true;
            (
                "Deleting one reviewed recovery backup",
                "Deleted the reviewed recovery backup. Installed mods and other retained content remain intact.",
                function(py, "history_prune", "remove")?,
                vec![library.clone(), plan],
            )
        }
        _ => return Err(invalid("Unknown owned-content controller operation.")),
    };
    if !output.is_empty() {
        manager.setattr(output, py.None())?;
    }
    let owner = manager.clone().unbind();
    let action = action.to_string();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let result = args.get_item(0)?;
        if !output.is_empty() {
            manager.setattr(output, &result)?;
        }
        match action.as_str() {
            "uninstall" => manager.setattr("lifecycle_review", py.None())?,
            "review_restore" => {
                manager.setattr("restore_preview", result.get_item(0)?)?;
                manager.setattr("lifecycle_review", result.get_item(1)?)?;
            }
            "restore" => {
                manager.setattr("selected_id", result)?;
                for key in ["restore_preview", "lifecycle_review"] {
                    manager.setattr(key, py.None())?;
                }
            }
            "prune_backup" => {
                for key in ["prune_review", "restore_preview", "lifecycle_review"] {
                    manager.setattr(key, py.None())?;
                }
            }
            _ => (),
        }
        if refresh {
            manager.call_method0("refresh")?;
        }
        if ["uninstall", "restore"].contains(&action.as_str()) {
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
        true,
        progress,
        complete.bind(py),
    )
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(controller_owned_action, m)?)?;
    Ok(())
}
