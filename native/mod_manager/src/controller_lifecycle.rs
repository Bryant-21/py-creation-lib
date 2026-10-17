use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
};
#[pyfunction]
fn controller_initialize(
    manager: &Bound<'_, PyAny>,
    extensions: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = manager.py();
    let scopes = PyDict::new(py);
    scopes.set_item("ini", "target")?;
    scopes.set_item("saves", "target")?;
    manager.setattr("target_scopes", scopes)?;
    manager.setattr("library", py.None())?;
    manager.setattr("target_id", "")?;
    manager.setattr("targets", PyList::empty(py))?;
    manager.setattr("_context_view", py.None())?;
    manager.setattr("target_executable", py.None())?;
    manager.setattr("target_review", py.None())?;
    manager.setattr("root_setup_review", py.None())?;
    manager.setattr("profiles", PyList::empty(py))?;
    manager.setattr("profile_id", "")?;
    manager.setattr("rows", PyList::empty(py))?;
    manager.setattr("rows_by_id", PyDict::new(py))?;
    manager.setattr("groups", PyList::new(py, [""])?)?;
    manager.setattr("enabled_count", 0)?;
    manager.setattr("selected_id", "")?;
    manager.setattr("import_preview", py.None())?;
    manager.setattr("instance_configuration", PyDict::new(py))?;
    manager.setattr("origins", PyDict::new(py))?;
    manager.setattr("migration_review", py.None())?;
    manager.setattr("file_index", py.None())?;
    manager.setattr("plugin_catalog", py.None())?;
    manager.setattr("archive_index", py.None())?;
    manager.setattr("combined_index", py.None())?;
    manager.setattr("save_inspections", PyDict::new(py))?;
    manager.setattr("save_comparisons", PyDict::new(py))?;
    manager.setattr("save_images", PyDict::new(py))?;
    manager.setattr("profile_backups", PyList::empty(py))?;
    manager.setattr("recipe", py.None())?;
    manager.setattr("recipe_plan", py.None())?;
    manager.setattr("recipe_candidates", PyTuple::empty(py))?;
    manager.setattr("recipe_suggestions", PyDict::new(py))?;
    manager.setattr("recipe_export", "")?;
    manager.setattr("name_review", py.None())?;
    manager.setattr("field_review", py.None())?;
    manager.setattr("group_review", py.None())?;
    manager.setattr("relink_review", py.None())?;
    manager.setattr("reference_undo", PySet::empty(py)?)?;
    manager.setattr("source_metadata_review", py.None())?;
    manager.setattr("source_metadata_undo", PySet::empty(py)?)?;
    manager.setattr("reference_review", py.None())?;
    manager.setattr("profile_restore", py.None())?;
    manager.setattr("instance_migration", py.None())?;
    manager.setattr("migration_runs", PyList::empty(py))?;
    manager.setattr("deleted_profiles", PyList::empty(py))?;
    manager.setattr("executables", PyList::empty(py))?;
    manager.setattr("launch_review", py.None())?;
    manager.setattr("runtime_sessions", PyList::empty(py))?;
    manager.setattr("runtime_session", py.None())?;
    manager.setattr("runtime_process", py.None())?;
    manager.setattr("runtime_status", PyDict::new(py))?;
    manager.setattr("_runtime_poll_at", 0)?;
    manager.setattr("mod_count", 0)?;
    manager.setattr("prepared", py.None())?;
    manager.setattr("install_plan", py.None())?;
    manager.setattr("install_repairs", PyDict::new(py))?;
    manager.setattr("install_review_state", py.None())?;
    manager.setattr("install_review_plan", py.None())?;
    manager.setattr("install_pairs", PyList::empty(py))?;
    manager.setattr("install_base_pairs", PyList::empty(py))?;
    manager.setattr("install_mapping", PyDict::new(py))?;
    manager.setattr("install_additions", PyDict::new(py))?;
    manager.setattr("install_addition_pairs", PyList::empty(py))?;
    manager.setattr("installer_image", py.None())?;
    manager.setattr("installer_image_source", "")?;
    manager.setattr("install_mode", "replace")?;
    manager.setattr("export_preview", py.None())?;
    manager.setattr("conflict_pairs", PyList::empty(py))?;
    manager.setattr("lifecycle_review", py.None())?;
    manager.setattr("restore_preview", py.None())?;
    manager.setattr("recovery_history", PyList::empty(py))?;
    manager.setattr("prune_review", py.None())?;
    manager.setattr("retained_review", py.None())?;
    manager.setattr("choices", PyDict::new(py))?;
    manager.setattr(
        "status",
        "Create a game instance to begin. MO2 import is optional in the Mods menu.",
    )?;
    manager.setattr("error", "")?;
    manager.setattr("progress", (py.None(), ""))?;
    manager.setattr("job_state", "idle")?;
    manager.setattr("_future", py.None())?;
    manager.setattr("_complete", py.None())?;
    manager.setattr("_filter_key", py.None())?;
    manager.setattr("_filtered", PyList::empty(py))?;
    manager.setattr("_status_revision", 0)?;
    manager.setattr("_status_key", py.None())?;
    manager.setattr("_status_rows", PyDict::new(py))?;
    manager.setattr("_conflict_index", py.None())?;
    manager.setattr("conflict_scan_state", "idle")?;
    manager.setattr("conflict_scan_error", "")?;
    manager.setattr("conflict_scan_progress", (py.None(), ""))?;
    manager.setattr("_conflict_future", py.None())?;
    manager.setattr("_conflict_generation", 0)?;
    manager.setattr("_conflict_due", f64::INFINITY)?;
    manager.setattr("enable_on_install", true)?;
    manager.setattr("nexus_game", "fallout4")?;
    manager.setattr("nexus_client", py.None())?;
    manager.setattr(
        "credential_store",
        py.import("creation_lib.mod_manager.nexus_auth")?
            .call_method0("CredentialStore")?,
    )?;
    manager.setattr("_startup_auth_checked", false)?;
    manager.setattr("pending_links", PyDict::new(py))?;
    manager.setattr("pending_destinations", PyDict::new(py))?;
    manager.setattr("download_records", PyList::empty(py))?;
    manager.setattr("download_review", py.None())?;
    manager.setattr("download_jobs", PyList::empty(py))?;
    manager.setattr("download_slots", 1)?;
    manager.setattr("_download_scheduler", py.None())?;
    manager.setattr("save_index", py.None())?;
    manager.setattr("save_details", PyDict::new(py))?;
    manager.setattr("save_transfer_plan", py.None())?;
    manager.setattr("save_backups", PyTuple::empty(py))?;
    manager.setattr("categories", PyTuple::empty(py))?;
    manager.setattr("category_token", "")?;
    manager.setattr("overwrite_review", py.None())?;
    manager.setattr("overwrite_restore", py.None())?;
    manager.setattr("overwrite_history", PyList::empty(py))?;
    manager.setattr("updates", PyDict::new(py))?;
    manager.setattr("nexus_states", PyDict::new(py))?;
    manager.setattr("auto_updates", true)?;
    manager.setattr("auto_file_checks", false)?;
    manager.setattr("update_hours", 6)?;
    manager.setattr("_next_update", 0)?;
    manager.setattr("_update_schedule", PyDict::new(py))?;
    manager.setattr("_update_schedule_context", py.None())?;
    manager.setattr("_update_schedule_rows", py.None())?;
    manager.setattr("_update_schedule_ids", PyList::empty(py))?;
    manager.setattr("_update_schedule_scope", "")?;
    manager.setattr("_update_pending", false)?;
    manager.setattr("_update_failures", 0)?;
    manager.setattr("_rate_blocked_until", 0)?;
    manager.setattr(
        "extensions",
        if extensions.is_none() {
            py.import("creation_lib.mod_manager.extensions")?
                .call_method0("builtin_registry")?
        } else {
            extensions.clone()
        },
    )?;
    for (field, prefix) in [
        ("_executor", "mod-manager"),
        ("_conflict_executor", "local-conflicts"),
    ] {
        let options = PyDict::new(py);
        options.set_item("max_workers", 1)?;
        options.set_item("thread_name_prefix", prefix)?;
        manager.setattr(
            field,
            py.import("concurrent.futures")?
                .getattr("ThreadPoolExecutor")?
                .call((), Some(&options))?,
        )?;
    }
    for field in ["_cancel", "_conflict_cancel"] {
        manager.setattr(field, py.import("threading")?.call_method0("Event")?)?;
    }
    for field in ["_conflict_events", "_download_events"] {
        manager.setattr(field, py.import("queue")?.call_method0("SimpleQueue")?)?;
    }
    manager.setattr(
        "metadata_observer",
        py.import("creation_lib.mod_manager.metadata_observer")?
            .call_method0("Observer")?,
    )?;
    Ok(())
}
#[pyfunction]
fn controller_cleanup(manager: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = manager.py();
    manager.getattr("_cancel")?.call_method0("set")?;
    manager
        .getattr("metadata_observer")?
        .call_method0("close")?;
    manager.getattr("_conflict_cancel")?.call_method0("set")?;
    let options = PyDict::new(py);
    options.set_item("wait", true)?;
    options.set_item("cancel_futures", true)?;
    manager
        .getattr("_conflict_executor")?
        .getattr("shutdown")?
        .call((), Some(&options))?;
    let scheduler = manager.getattr("_download_scheduler")?;
    if scheduler.is_truthy()? {
        scheduler.call_method0("close")?;
        manager.setattr("_download_scheduler", py.None())?;
    }
    manager
        .getattr("_executor")?
        .getattr("shutdown")?
        .call((), Some(&options))?;
    let future = manager.getattr("_future")?;
    if !future.is_none() && !future.call_method0("cancelled")?.is_truthy()? {
        let result: PyResult<()> = (|| {
            let result = future.call_method0("result")?;
            if result.is_instance(
                &py.import("creation_lib.mod_manager.controller")?
                    .getattr("PreparedInstall")?,
            )? {
                result.getattr("package")?.call_method0("close")?;
            } else if result.is_instance(
                &py.import("creation_lib.mod_manager.nexus")?
                    .getattr("NexusClient")?,
            )? {
                result.call_method0("forget")?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            if !error.is_instance_of::<pyo3::exceptions::PyException>(py) {
                return Err(error);
            }
        }
    }
    manager.setattr("_future", py.None())?;
    manager.call_method0("discard_install")?;
    let client = manager.getattr("nexus_client")?;
    if client.is_truthy()? {
        client.call_method0("forget")?;
    }
    manager.getattr("pending_links")?.call_method0("clear")?;
    manager
        .getattr("pending_destinations")?
        .call_method0("clear")?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(controller_initialize, m)?)?;
    m.add_function(wrap_pyfunction!(controller_cleanup, m)?)?;
    Ok(())
}
