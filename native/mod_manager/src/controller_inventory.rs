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
fn number(value: &Bound<'_, PyAny>) -> PyResult<String> {
    value.call_method1("__format__", (",",))?.extract()
}
fn count(value: &Bound<'_, PyAny>, field: &str) -> PyResult<String> {
    number(
        value
            .getattr(field)?
            .len()?
            .into_pyobject(value.py())?
            .as_any(),
    )
}

#[pyfunction]
fn controller_inventory_action(
    manager: &Bound<'_, PyAny>,
    action: &str,
    args: &Bound<'_, PyTuple>,
) -> PyResult<()> {
    let py = manager.py();
    let library = manager.getattr(
        if [
            "inspect_files",
            "inspect_plugins",
            "inspect_archives",
            "inspect_combined_data",
            "apply_plugins",
        ]
        .contains(&action)
        {
            "context_library"
        } else {
            "library"
        },
    )?;
    let profile = manager.getattr("profile_id")?;
    if !library.is_truthy()? {
        return Err(invalid("Open a library first."));
    }
    if [
        "inspect_plugins",
        "inspect_archives",
        "inspect_combined_data",
    ]
    .contains(&action)
        && !profile.is_truthy()?
    {
        return Err(invalid("Select a local game profile first."));
    }
    let options = PyDict::new(py);
    let mut progress = false;
    let mut metadata = false;
    let (label, operation, inputs) = match action {
        "preview_export" => {
            manager.setattr("export_preview", py.None())?;
            (
                "Indexing enabled mods and file conflicts",
                library.getattr("export_plan")?,
                vec![profile.clone()],
            )
        }
        "review_migration" => {
            manager.setattr("migration_review", py.None())?;
            progress = true;
            (
                "Hashing imported sources for owned copy",
                library.getattr("review_migration")?,
                vec![
                    py.import("builtins")?
                        .call_method1("tuple", (args.get_item(0)?,))?,
                ],
            )
        }
        "migrate" | "export" => {
            let plan = manager.getattr(if action == "migrate" {
                "migration_review"
            } else {
                "export_preview"
            })?;
            if !plan.is_truthy()? {
                return Err(invalid(if action == "migrate" {
                    "Review selected imported sources before copying."
                } else {
                    "Preview the profile's file conflicts first."
                }));
            }
            progress = true;
            (
                if action == "migrate" {
                    "Copying reviewed sources into library ownership"
                } else {
                    "Staging profile export"
                },
                library.getattr(action)?,
                vec![plan],
            )
        }
        "inspect_files" => {
            manager.setattr("file_index", py.None())?;
            options.set_item("include_disabled", args.get_item(0)?)?;
            progress = true;
            (
                "Indexing file providers",
                function(py, "conflicts", "build_index")?,
                vec![library.clone(), profile.clone()],
            )
        }
        "inspect_plugins" => {
            metadata = args.get_item(0)?.is_truthy()?;
            progress = true;
            let owner = library.clone().unbind();
            let selected = profile.clone().unbind();
            let operation = callback(py, move |py, _, kwargs| {
                let catalog = function(py, "plugin_state", "inventory")?
                    .call((owner.bind(py), selected.bind(py)), kwargs)?;
                if metadata {
                    Ok(function(py, "plugin_state", "inspect")?
                        .call(
                            (
                                catalog,
                                function(py, "plugin_state", "CachedMetadata")?
                                    .call1((owner.bind(py),))?,
                            ),
                            kwargs,
                        )?
                        .unbind())
                } else {
                    Ok(catalog.unbind())
                }
            })?;
            (
                if metadata {
                    "Inspecting plugin details"
                } else {
                    "Refreshing plugin inventory"
                },
                operation.into_bound(py),
                vec![],
            )
        }
        "inspect_archives" | "inspect_combined_data" => {
            progress = true;
            if action == "inspect_combined_data" {
                options.set_item(
                    "resolver",
                    manager.getattr("extensions")?.getattr("resolve_archive")?,
                )?;
            }
            (
                if action == "inspect_archives" {
                    "Inspecting archive member providers"
                } else {
                    "Inspecting loose and archive providers"
                },
                function(
                    py,
                    if action == "inspect_archives" {
                        "archive_index"
                    } else {
                        "combined_data"
                    },
                    "build",
                )?,
                vec![library.clone(), profile.clone()],
            )
        }
        "review_reference_removal" => {
            manager.setattr("reference_review", py.None())?;
            (
                "Reviewing local entry removal",
                function(py, "reference_removal", "review")?,
                vec![
                    library.clone(),
                    profile.clone(),
                    py.import("builtins")?
                        .call_method1("tuple", (args.get_item(0)?,))?,
                ],
            )
        }
        "remove_references" => {
            let plan = manager.getattr("reference_review")?;
            if !plan.is_truthy()? || !plan.getattr("profile_id")?.eq(&profile)? {
                return Err(invalid("Review this profile's local entry removal first."));
            }
            (
                "Removing reviewed local references",
                function(py, "reference_removal", "apply")?,
                vec![library.clone(), plan],
            )
        }
        "apply_plugins" => {
            let owner = library.clone().unbind();
            let plan = args.get_item(0)?.unbind();
            let operation = callback(py, move |py, _, kwargs| {
                let library = owner.bind(py);
                let plan = plan.bind(py);
                function(py, "plugin_state", "apply")?.call((library, plan), kwargs)?;
                let catalog = plan.getattr("catalog")?;
                let known = PyDict::new(py);
                for plugin in catalog.getattr("plugins")?.try_iter()? {
                    let plugin = plugin?;
                    known.set_item(plugin.getattr("name")?.call_method0("casefold")?, &plugin)?;
                }
                let replace = py.import("dataclasses")?.getattr("replace")?;
                let plugins = PyList::empty(py);
                for (priority, name) in plan.getattr("order")?.try_iter()?.enumerate() {
                    let key = name?.call_method0("casefold")?;
                    let fields = PyDict::new(py);
                    fields.set_item("priority", priority)?;
                    fields.set_item("enabled", plan.getattr("enabled")?.contains(&key)?)?;
                    plugins.append(
                        replace.call(
                            (known
                                .get_item(&key)?
                                .ok_or_else(|| invalid("Reviewed plugin is missing."))?,),
                            Some(&fields),
                        )?,
                    )?;
                }
                let fields = PyDict::new(py);
                fields.set_item(
                    "signature",
                    function(py, "plugin_state", "signature")?
                        .call1((library, catalog.getattr("profile_id")?))?,
                )?;
                fields.set_item("locked", plan.getattr("locked")?)?;
                fields.set_item("plugins", plugins.to_tuple())?;
                Ok(replace.call((catalog,), Some(&fields))?.unbind())
            })?;
            (
                "Saving reviewed local plugin state",
                operation.into_bound(py),
                vec![],
            )
        }
        _ => return Err(invalid("Unknown inventory controller operation.")),
    };
    let owner = manager.clone().unbind();
    let action = action.to_string();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let result = args.get_item(0)?;
        let status = match action.as_str() {
            "preview_export" => {
                manager.setattr("export_preview", &result)?;
                manager.setattr(
                    "conflict_pairs",
                    py.import("builtins")?.call_method1(
                        "list",
                        (result.getattr("conflicts")?.call_method0("items")?,),
                    )?,
                )?;
                format!(
                    "Ready to stage {} files; {} file conflicts resolved by priority.",
                    count(&result, "files")?,
                    count(&result, "conflicts")?
                )
            }
            "review_migration" => {
                manager.setattr("migration_review", &result)?;
                format!(
                    "Reviewed {} sources ({} bytes). Confirm the copy; source folders will remain unchanged.",
                    count(&result, "sources")?,
                    number(&result.getattr("bytes")?)?
                )
            }
            "migrate" => {
                manager.setattr("migration_review", py.None())?;
                manager.call_method0("refresh")?;
                manager.setattr("export_preview", py.None())?;
                format!(
                    "Copied {} sources into owned library folders. All profile references and original sources were preserved.",
                    number(result.len()?.into_pyobject(py)?.as_any())?
                )
            }
            "export" => format!("Profile staged successfully: {}", result.str()?),
            "inspect_files" => {
                manager.setattr("file_index", &result)?;
                manager.setattr("combined_index", py.None())?;
                format!(
                    "Indexed {} loose-file paths. {} unavailable or unmanaged sources need review.",
                    count(&result, "files")?,
                    count(&result, "issues")?
                )
            }
            "inspect_plugins" => {
                manager.setattr("plugin_catalog", &result)?;
                format!(
                    "{} plugins inventoried{}",
                    count(&result, "plugins")?,
                    if metadata {
                        " with header/dependency details."
                    } else {
                        ". Inspect details before applying plugin changes."
                    }
                )
            }
            "inspect_archives" => {
                manager.setattr("archive_index", &result)?;
                manager.setattr("combined_index", py.None())?;
                format!(
                    "Inventoried {} archive containers and {} member paths. See archive activation/precedence limitations.",
                    count(&result, "archives")?,
                    count(&result, "members")?
                )
            }
            "inspect_combined_data" => {
                manager.setattr("combined_index", &result)?;
                manager.setattr("file_index", result.getattr("loose")?)?;
                manager.setattr("archive_index", result.getattr("archives")?)?;
                format!(
                    "Read {} combined paths. Winners are profile load-plan predictions; unresolved precedence is marked.",
                    count(&result, "files")?
                )
            }
            "review_reference_removal" => {
                manager.setattr("reference_review", result)?;
                "Reviewed profile-local reference/separator removal. Source content is retained."
                    .into()
            }
            "remove_references" => {
                manager.call_method0("refresh")?;
                "Removed reviewed references/separators from this profile. Undo restores them; repeat imports preserve removal.".into()
            }
            "apply_plugins" => {
                manager.call_method0("refresh")?;
                manager.setattr("plugin_catalog", result)?;
                "Plugin activation and order saved in this local profile. Game and MO2 configuration files stay unchanged.".into()
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
    m.add_function(wrap_pyfunction!(controller_inventory_action, m)?)?;
    Ok(())
}
