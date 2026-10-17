use crate::{
    controller::{callback, submit_call},
    invalid,
};
use pyo3::{
    exceptions::PyOSError,
    prelude::*,
    types::{PyDict, PyTuple},
};

fn function<'py>(py: Python<'py>, module: &str, name: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import(format!("creation_lib.mod_manager.{module}"))?
        .getattr(name)
}

#[pyfunction]
fn controller_overwrite_action(
    manager: &Bound<'_, PyAny>,
    action: &str,
    args: &Bound<'_, PyTuple>,
) -> PyResult<()> {
    let py = manager.py();
    let profile = manager.getattr("profile_id")?;
    let retained = manager.getattr("retained_review")?;
    let kind = if action == "review_retained_backup" {
        args.get_item(0)?.extract::<String>()?
    } else if action == "remove_retained_backup" && retained.is_truthy()? {
        retained.getattr("kind")?.extract()?
    } else {
        String::new()
    };
    let library_field = if kind == "save" {
        "save_library"
    } else {
        "context_library"
    };
    let library = manager.getattr(library_field)?;
    if !library.is_truthy()? || !profile.is_truthy()? {
        return Err(invalid("Choose a local profile first."));
    }
    let mut identifier = py.None();
    let mut copy = false;
    let (label, operation, inputs) = match action {
        "inspect_overwrite" => {
            let owner = library.clone().unbind();
            let selected = profile.clone().unbind();
            let work = callback(py, move |py, _, kwargs| {
                let review = function(py, "overwrite", "review")?
                    .call((owner.bind(py), selected.bind(py)), kwargs)?;
                let history = function(py, "overwrite", "history")?
                    .call1((owner.bind(py), selected.bind(py)))?;
                Ok((review, history).into_pyobject(py)?.into_any().unbind())
            })?;
            (
                "Reviewing owned runtime overwrite",
                work.into_bound(py),
                vec![],
            )
        }
        "review_retained_backup" => {
            identifier = args.get_item(1)?.unbind();
            let mut available = false;
            if kind == "save" || kind == "overwrite" {
                for record in manager
                    .getattr(if kind == "save" {
                        "save_backups"
                    } else {
                        "overwrite_history"
                    })?
                    .try_iter()?
                {
                    let record = record?;
                    let id = if kind == "save" {
                        if !record.getattr("profile")?.eq(&profile)? {
                            continue;
                        }
                        record.getattr("id")?
                    } else {
                        record.get_item("id")?
                    };
                    if id.eq(identifier.bind(py))? {
                        available = true;
                        break;
                    }
                }
            }
            if !available {
                return Err(invalid(
                    "Refresh and select an owned retained backup first.",
                ));
            }
            manager.setattr("retained_review", py.None())?;
            (
                "Reviewing retained backup deletion",
                function(py, "backup_prune", "review")?,
                vec![
                    library.clone(),
                    kind.clone().into_pyobject(py)?.into_any(),
                    profile.clone(),
                    identifier.bind(py).clone(),
                ],
            )
        }
        "remove_retained_backup" => {
            if !retained.is_truthy()? || !retained.getattr("profile_id")?.eq(&profile)? {
                return Err(invalid(
                    "Review this local profile's retained backup deletion first.",
                ));
            }
            let owner = library.clone().unbind();
            let plan = retained.clone().unbind();
            let save = kind == "save";
            let work = callback(py, move |py, _, kwargs| {
                let library = owner.bind(py);
                let plan = plan.bind(py);
                function(py, "backup_prune", "remove")?.call((library, plan), kwargs)?;
                match function(
                    py,
                    if save { "save_backups" } else { "overwrite" },
                    if save { "inventory" } else { "history" },
                )?
                .call1((library, plan.getattr("profile_id")?))
                {
                    Ok(records) => Ok(records.unbind()),
                    Err(error) => {
                        if !error.is_instance_of::<PyOSError>(py)
                            && !error.matches(py, function(py, "paths", "ManagerError")?)?
                        {
                            return Err(error);
                        }
                        let record = PyDict::new(py);
                        record.set_item("mod_id", "")?;
                        record.set_item("kind", "backup-prune")?;
                        let committed = function(py, "transactions", "CommittedTransaction")?
                            .call1((record,))?;
                        committed.setattr("args", ("The reviewed backup was deleted, but the retained-backup inventory could not refresh. Reopen or refresh the library.",))?;
                        let committed = PyErr::from_value(committed);
                        committed.set_cause(py, Some(error));
                        Err(committed)
                    }
                }
            })?;
            (
                "Deleting reviewed retained backup",
                work.into_bound(py),
                vec![],
            )
        }
        "change_overwrite" => {
            let plan = manager.getattr("overwrite_review")?;
            if !plan.is_truthy()? || !plan.getattr("profile_id")?.eq(&profile)? {
                return Err(invalid("Review this profile's owned overwrite first."));
            }
            let change = args.get_item(0)?.extract::<String>()?;
            if !["clear", "copy"].contains(&change.as_str()) {
                return Err(invalid("Unsupported owned overwrite operation."));
            }
            copy = change == "copy";
            let mut inputs = vec![library.clone(), plan];
            if copy {
                inputs.push(args.get_item(1)?);
            }
            (
                if copy {
                    "Copying owned overwrite to a mod"
                } else {
                    "Backing up and clearing owned overwrite"
                },
                function(py, "overwrite", if copy { "copy_mod" } else { "clear" })?,
                inputs,
            )
        }
        "review_overwrite_restore" => {
            identifier = args.get_item(0)?.unbind();
            (
                "Reviewing owned overwrite backup",
                function(py, "overwrite", "review_restore")?,
                vec![
                    library.clone(),
                    profile.clone(),
                    identifier.bind(py).clone(),
                ],
            )
        }
        "restore_overwrite" => {
            let prepared = manager.getattr("overwrite_restore")?;
            if !prepared.is_truthy()?
                || !prepared.get_item(1)?.getattr("profile_id")?.eq(&profile)?
            {
                return Err(invalid("Review this local profile's output backup first."));
            }
            (
                "Restoring owned overwrite backup",
                function(py, "overwrite", "restore")?,
                vec![
                    library.clone(),
                    prepared.get_item(0)?,
                    prepared.get_item(1)?,
                ],
            )
        }
        _ => return Err(invalid("Unknown overwrite controller operation.")),
    };
    let owner = manager.clone().unbind();
    let selected = profile.unbind();
    let source = library.unbind();
    let action = action.to_string();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let result = args.get_item(0)?;
        let status = match action.as_str() {
            "inspect_overwrite" => {
                manager.setattr("overwrite_review", result.get_item(0)?)?;
                manager.setattr("overwrite_history", result.get_item(1)?)?;
                manager.setattr("overwrite_restore", py.None())?;
                "Reviewed manager-owned runtime output and retained backups. MO2 overwrite is excluded from imports."
            }
            "review_retained_backup" => {
                if !manager.getattr(library_field)?.is(source.bind(py))
                    || !manager.getattr("profile_id")?.eq(selected.bind(py))?
                {
                    return Err(invalid(
                        "The local profile changed during backup review. Review it again.",
                    ));
                }
                manager.setattr("retained_review", result)?;
                "Reviewed one owned retained backup for permanent deletion. Acknowledgement is required."
            }
            "remove_retained_backup" => {
                manager.setattr("retained_review", py.None())?;
                manager.setattr(
                    if kind == "save" {
                        "save_backups"
                    } else {
                        "overwrite_history"
                    },
                    result,
                )?;
                manager.setattr(
                    if kind == "save" {
                        "save_transfer_plan"
                    } else {
                        "overwrite_restore"
                    },
                    py.None(),
                )?;
                "Permanently deleted one reviewed owned retained backup. Current output, saves and other backups are intact."
            }
            "change_overwrite" => {
                manager.call_method0("refresh")?;
                if copy {
                    manager.setattr("selected_id", result)?;
                    "Copied generated output into an enabled owned mod; overwrite remains intact. Review it separately before clearing."
                } else {
                    "Owned runtime overwrite cleared into a retained complete backup."
                }
            }
            "review_overwrite_restore" => {
                manager.setattr("overwrite_restore", (identifier.bind(py), result))?;
                "Reviewed retained output. Restoration requires an empty owned overwrite folder."
            }
            "restore_overwrite" => {
                manager.call_method0("refresh")?;
                "Restored the complete owned overwrite backup into an empty folder."
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
        &PyDict::new(py),
        true,
        true,
        complete.bind(py),
    )
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(controller_overwrite_action, m)?)?;
    Ok(())
}
