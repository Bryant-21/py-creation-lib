use crate::{
    agent_operations::{finish, with_manager},
    encoding, invalid,
    plugin_inventory::object,
};
use pyo3::{
    prelude::*,
    types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple},
};
use serde_json::{Value, json};

// Explicit exports keep controller internals and credentials out of the command protocol.
const ACTIONS: &[(&str, bool, &str)] = &[
    (
        "profiles",
        false,
        "profiles deleted_profiles profile_backups",
    ),
    ("mods", false, "rows"),
    ("targets", false, "targets target_id"),
    ("executables", false, "executables"),
    ("downloads", false, "download_jobs download_records"),
    ("history", false, "recovery_history"),
    ("ini", false, ""),
    ("select_profile", false, "profile_id"),
    ("select_target", false, "target_id"),
    ("create_profile", true, "profiles profile_id"),
    ("rename_profile", true, "profiles"),
    ("delete_profile", true, "profiles deleted_profiles"),
    ("restore_profile", true, "profiles profile_id"),
    ("save_profile_ini", true, ""),
    ("restore_profile_ini", true, ""),
    ("backup_profile", true, "profile_backups"),
    ("review_profile_backup", false, "profile_restore"),
    ("restore_profile_backup", true, "profile_backups"),
    ("remove_profile_backup", true, "profile_backups"),
    ("export_recipe", true, "recipe_export"),
    (
        "inspect_recipe",
        false,
        "recipe recipe_candidates recipe_suggestions",
    ),
    ("review_recipe", false, "recipe_plan"),
    ("apply_recipe", true, "profiles profile_id"),
    ("create_empty_mod", true, "selected_id rows"),
    ("create_group", true, "selected_id rows"),
    ("customize", true, "rows"),
    ("bulk_organize", true, "rows"),
    ("reorder", true, "rows"),
    ("set_target_enabled", true, "rows"),
    ("undo_organization", true, "rows"),
    ("inspect_mod_name", false, "name_review"),
    ("rename_mod_name", true, "rows"),
    ("inspect_mod_field", false, "field_review"),
    ("edit_mod_field", true, "rows"),
    ("inspect_group_name", false, "group_review"),
    ("rename_group", true, "rows"),
    ("review_reference_source", false, "relink_review"),
    ("relink_source", true, "rows"),
    ("undo_source_relink", true, "rows"),
    ("review_source_metadata", false, "source_metadata_review"),
    ("refresh_source_metadata", true, "rows"),
    ("undo_source_metadata", true, "rows"),
    ("review_owned", false, "lifecycle_review"),
    ("uninstall", true, "rows recovery_history"),
    ("review_restore", false, "restore_preview lifecycle_review"),
    ("restore", true, "selected_id rows recovery_history"),
    ("review_prune", false, "prune_review"),
    ("prune_backup", true, "recovery_history"),
    ("review_migration", false, "migration_review"),
    ("migrate", true, "rows"),
    ("review_reference_removal", false, "reference_review"),
    ("remove_references", true, "rows"),
    ("preview_export", false, "export_preview"),
    ("export", true, ""),
    ("inspect_files", false, "file_index"),
    ("inspect_plugins", false, "plugin_catalog"),
    ("review_plugins", false, ""),
    ("apply_plugins", true, "plugin_catalog"),
    ("inspect_archives", false, "archive_index"),
    ("inspect_combined_data", false, "combined_index"),
    (
        "inspect_overwrite",
        false,
        "overwrite_review overwrite_history",
    ),
    ("change_overwrite", true, "overwrite_history rows"),
    ("review_overwrite_restore", false, "overwrite_restore"),
    ("restore_overwrite", true, "overwrite_history"),
    ("review_retained_backup", false, "retained_review"),
    ("remove_retained_backup", true, ""),
    ("inspect_categories", false, "categories"),
    ("import_nexus_categories", true, "categories"),
    ("edit_category", true, "categories"),
    ("assign_category", true, "rows"),
    ("inspect_saves", false, "save_index"),
    ("inspect_source_saves", false, "save_index"),
    ("inspect_global_saves", false, "save_index"),
    ("review_save_copy", false, "save_transfer_plan"),
    ("apply_save_copy", true, ""),
    ("inspect_save_backups", false, "save_backups"),
    ("review_save_backup", false, "save_transfer_plan"),
    ("inspect_save_header", false, "save_details"),
    ("inspect_save_plugins", false, "save_inspections"),
    ("preview_save_screenshot", false, "save_images"),
    ("compare_save_profile", false, "save_comparisons"),
    ("compare_save_to", false, "save_comparisons"),
    ("review_target", false, "target_review"),
    ("apply_target", true, "targets target_id"),
    ("prepare_target_f4se", true, "targets"),
    ("review_target_root_setup", false, "root_setup_review"),
    ("apply_target_root_setup", true, "targets"),
    ("save_executable", true, "executables"),
    ("remove_executable", true, "executables"),
    ("inspect_nexus_mod", false, "nexus_states"),
    ("nexus_account_action", true, "nexus_states"),
    ("review_download", false, "download_review"),
    ("download", true, "download_jobs download_records"),
    ("resume_download", true, "download_jobs download_records"),
    ("remove_download_job", true, "download_jobs"),
    ("updates", false, "updates nexus_states"),
];

fn spec(name: &str) -> PyResult<&'static (&'static str, bool, &'static str)> {
    ACTIONS.iter().find(|s| s.0 == name).ok_or_else(|| {
        invalid(format!(
            "Unknown manager action: {name}. Use manager actions to list the contract."
        ))
    })
}

pub(crate) fn serial(value: &Bound<'_, PyAny>, depth: usize) -> PyResult<Value> {
    if depth > 24 {
        return Err(invalid("Manager response exceeds its nesting limit."));
    }
    if value.is_none() {
        return Ok(Value::Null);
    }
    if value.is_instance_of::<PyString>() {
        return Ok(json!(value.extract::<String>()?));
    }
    if value.is_instance_of::<PyBool>() {
        return Ok(json!(value.extract::<bool>()?));
    }
    if value.is_instance_of::<PyInt>() {
        return Ok(json!(value.extract::<i128>()?));
    }
    if value.is_instance_of::<PyFloat>() {
        return Ok(json!(value.extract::<f64>()?));
    }
    if let Ok(dict) = value.cast::<PyDict>() {
        let mut result = serde_json::Map::new();
        for (key, item) in dict.iter() {
            let key = if key.is_instance_of::<PyString>() {
                key.extract()?
            } else {
                serial(&key, depth + 1)?.to_string()
            };
            result.insert(key, serial(&item, depth + 1)?);
        }
        return Ok(Value::Object(result));
    }
    if value.hasattr("__fspath__")? {
        return Ok(json!(value.str()?.to_string()));
    }
    let py = value.py();
    if py
        .import("dataclasses")?
        .call_method1("is_dataclass", (value,))?
        .is_truthy()?
    {
        let mut result = serde_json::Map::new();
        for field in py
            .import("dataclasses")?
            .call_method1("fields", (value,))?
            .try_iter()?
        {
            let name = field?.getattr("name")?.extract::<String>()?;
            if name.starts_with('_') || name == "tree" {
                continue;
            }
            result.insert(
                name.clone(),
                serial(&value.getattr(name.as_str())?, depth + 1)?,
            );
        }
        return Ok(Value::Object(result));
    }
    if value.hasattr("shape")? && value.hasattr("dtype")? {
        return Ok(
            json!({"shape":serial(&value.getattr("shape")?, depth + 1)?,"dtype":value.getattr("dtype")?.str()?.to_string()}),
        );
    }
    let builtins = py.import("builtins")?;
    if value.is_instance_of::<PyList>()
        || value.is_instance_of::<PyTuple>()
        || value.is_instance(&builtins.getattr("set")?)?
        || value.is_instance(&builtins.getattr("frozenset")?)?
    {
        let mut result = Vec::new();
        for item in value.try_iter()? {
            result.push(serial(&item?, depth + 1)?);
        }
        return Ok(Value::Array(result));
    }
    Err(invalid(format!(
        "Unsupported manager response type: {}",
        value.get_type().name()?
    )))
}

fn connect(manager: &Bound<'_, PyAny>) -> PyResult<()> {
    if manager.getattr("nexus_client")?.is_none() {
        manager.call_method0("connect_nexus")?;
        finish(manager)?;
    }
    Ok(())
}

fn execute(
    manager: &Bound<'_, PyAny>,
    step: &Value,
    plugin_plan: &mut Option<Py<PyAny>>,
) -> PyResult<Value> {
    let py = manager.py();
    let name = step["action"].as_str().unwrap_or("");
    let (_, _, fields) = spec(name)?;
    let args = object(py, step.get("args").unwrap_or(&json!([])))?;
    let args = PyTuple::new(py, args.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
    let opts = object(py, step.get("options").unwrap_or(&json!({})))?;
    let opts = opts.cast::<PyDict>()?;
    let mut extra = json!({});
    match name {
        "profiles" | "mods" | "targets" | "executables" | "downloads" | "history" => (),
        "ini" => {
            let library = manager.getattr("context_library")?;
            let selected =
                crate::agent_library::profile(&library, &manager.getattr("profile_id")?)?;
            extra = serial(
                &py.import("creation_lib.mod_manager.profile_state")?
                    .call_method1("effective_ini", (library, selected, args.get_item(0)?))?,
                0,
            )?;
        }
        "review_plugins" => {
            let options = PyDict::new(py);
            options.set_item("metadata", true)?;
            manager.call_method("inspect_plugins", (), Some(&options))?;
            finish(manager)?;
            let module = py.import("creation_lib.mod_manager.plugin_state")?;
            let catalog = manager.getattr("plugin_catalog")?;
            let plan = module.getattr("review")?.call((&catalog,), Some(opts))?;
            extra = json!({"plan":serial(&plan, 0)?,"indices":serial(&module.call_method1("load_indices", (&catalog,plan.getattr("order")?,plan.getattr("enabled")?))?,0)?});
            *plugin_plan = Some(plan.unbind());
        }
        "apply_plugins" => {
            let plan = plugin_plan.take().ok_or_else(|| {
                invalid("Use review_plugins before apply_plugins in the same workflow.")
            })?;
            manager.call_method1("apply_plugins", (plan,))?;
            finish(manager)?;
        }
        "save_executable" => {
            let executable = py
                .import("creation_lib.mod_manager.launcher")?
                .getattr("definition")?
                .call((), Some(opts))?;
            manager.call_method1(name, (executable,))?;
            finish(manager)?;
        }
        "download" => {
            connect(manager)?;
            let link = py
                .import("creation_lib.mod_manager.nexus")?
                .call_method1("parse_nxm", (args.get_item(0)?,))?;
            manager.call_method1("accept_nxm", (args.get_item(0)?,))?;
            manager.call_method1("download_nxm", (link.getattr("identity")?,))?;
            finish(manager)?;
        }
        "resume_download" => {
            let queue = py
                .import("creation_lib.mod_manager.download_queue")?
                .call_method1("DownloadQueue", (manager.getattr("library")?,))?;
            let job = queue.call_method1("get", (args.get_item(0)?,))?;
            if !manager
                .call_method1("download_local_recovery", (job,))?
                .is_truthy()?
            {
                connect(manager)?;
            }
            manager.call_method("resume_download", args, Some(opts))?;
            finish(manager)?;
        }
        "updates" => {
            connect(manager)?;
            let files = opts
                .get_item("files")?
                .is_some_and(|v| v.is_truthy().unwrap_or(false));
            let check = opts
                .get_item("check")?
                .is_some_and(|v| v.is_truthy().unwrap_or(false));
            manager.setattr("auto_file_checks", files)?;
            crate::update_schedule::prepare(manager, files)?;
            let now: f64 = py.import("time")?.call_method0("monotonic")?.extract()?;
            if check && now >= manager.getattr("_next_update")?.extract::<f64>()? {
                let kwargs = PyDict::new(py);
                kwargs.set_item("manual", false)?;
                manager.call_method(
                    if files {
                        "check_nexus_batch"
                    } else {
                        "check_updates"
                    },
                    (),
                    Some(&kwargs),
                )?;
                finish(manager)?;
            }
            let now: f64 = py.import("time")?.call_method0("monotonic")?.extract()?;
            extra = json!({"schedule":serial(&manager.getattr("_update_schedule")?,0)?, "next_check_seconds":(manager.getattr("_next_update")?.extract::<f64>()? - now).max(0.0)});
        }
        _ => {
            if ["inspect_nexus_mod", "nexus_account_action"].contains(&name)
                || name == "import_nexus_categories"
                    && (args.is_empty() || args.get_item(0)?.is_truthy()?)
            {
                connect(manager)?;
            }
            manager.call_method(name, args, Some(opts))?;
            finish(manager)?;
        }
    }
    let mut result = json!({"action":name,"status":manager.getattr("status")?.extract::<String>()?,"result":extra});
    for field in fields.split_whitespace() {
        result[field] = serial(&manager.getattr(field)?, 0)?;
    }
    Ok(result)
}

#[pyfunction]
fn agent_actions(py: Python<'_>) -> PyResult<Py<PyAny>> {
    let controller = py
        .import("creation_lib.mod_manager.controller")?
        .getattr("ManagerController")?;
    let inspect = py.import("inspect")?;
    let actions = ACTIONS.iter().map(|(name, mutate, fields)| {
        let signature = match *name {
            "review_plugins" => "(*, order=None, enabled=None, locked=None)".into(),
            "apply_plugins" => "() — consumes the preceding review_plugins plan".into(),
            "save_executable" => "(**Executable fields)".into(),
            "updates" => "(*, check=False, files=False)".into(),
            "download" => "(nxm_url)".into(),
            "ini" => "(filename)".into(),
            _ if controller.hasattr(*name)? => inspect.call_method1("signature", (controller.getattr(*name)?,))?.str()?.to_string(),
            _ => "()".into(),
        };
        Ok(json!({"action":name,"requires_apply":mutate,"signature":signature,"outputs":fields.split_whitespace().collect::<Vec<_>>()}))
    }).collect::<PyResult<Vec<_>>>()?;
    Ok(object(py, &json!({"version":1,"actions":actions,"atomic":false}))?.unbind())
}

#[pyfunction]
fn agent_workflow(
    root: &Bound<'_, PyAny>,
    wanted: &str,
    target: &str,
    request: &str,
    apply: bool,
) -> PyResult<Py<PyAny>> {
    if request.len() > 1024 * 1024 {
        return Err(invalid("Manager workflow exceeds 1 MiB."));
    }
    let request = encoding::parse(request)?;
    let steps = request
        .as_array()
        .ok_or_else(|| invalid("A workflow must be an array of action objects."))?;
    if steps.is_empty() || steps.len() > 64 {
        return Err(invalid("Use 1 to 64 workflow steps."));
    }
    for step in steps {
        let (_, mutate, _) = spec(step["action"].as_str().unwrap_or(""))?;
        if *mutate && !apply {
            return Err(invalid(
                "This workflow changes library state. Pass --apply to execute it.",
            ));
        }
        if step.get("args").is_some_and(|v| !v.is_array())
            || step.get("options").is_some_and(|v| !v.is_object())
        {
            return Err(invalid(
                "Action args must be an array and options must be an object.",
            ));
        }
    }
    with_manager(root, wanted, |manager| {
        if !target.is_empty() {
            manager.call_method1("select_target", (target,))?;
        }
        let mut plugin_plan = None;
        let mut results = Vec::new();
        for (index, step) in steps.iter().enumerate() {
            match execute(manager, step, &mut plugin_plan) {
                Ok(result) => results.push(result),
                Err(error) => {
                    return Err(invalid(format!(
                        "Workflow step {} ({}) failed: {}. {} earlier steps completed; each mutation is its own transaction.",
                        index + 1,
                        step["action"].as_str().unwrap_or(""),
                        error,
                        results.len()
                    )));
                }
            }
        }
        Ok(object(root.py(), &json!({"profile_id":manager.getattr("profile_id")?.extract::<String>()?,"target_id":manager.getattr("target_id")?.extract::<String>()?,"results":results}))?.unbind())
    })
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(agent_actions, m)?)?;
    m.add_function(wrap_pyfunction!(agent_workflow, m)?)?;
    Ok(())
}
