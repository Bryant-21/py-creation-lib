use crate::{
    check_cancel,
    history_prune::py_path,
    invalid,
    plugin_inventory::{object, value},
    reject_links,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use serde_json::json;
use std::{
    collections::{BTreeSet, HashSet},
    path::PathBuf,
};
fn absolute<'py>(path: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    path.py()
        .import("pathlib")?
        .getattr("Path")?
        .call1((path,))?
        .call_method0("absolute")
}
#[pyfunction]
fn mo2_migration_discover(
    path: &Bound<'_, PyAny>,
    destination: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    source_mappings: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = path.py();
    let chosen = absolute(path)?;
    let root: PathBuf = chosen.extract()?;
    reject_links(&root)?;
    let input = if root.is_dir() && root.join("modlist.txt").is_file() {
        let instance = chosen.getattr("parent")?.getattr("parent")?;
        let instance_path: PathBuf = instance.extract()?;
        if !instance_path.join("ModOrganizer.ini").is_file() {
            return Err(invalid(
                "Choose the MO2 instance directory or ModOrganizer.ini to import all profiles. This profile folder cannot identify a redirected instance safely.",
            ));
        }
        instance
    } else {
        path.clone()
    };
    let mo2 = py.import("creation_lib.mod_manager.mo2")?;
    let mut preview = mo2.call_method1("inspect_instance", (&input, cancel, source_mappings))?;
    let mappings = if source_mappings.is_truthy()? {
        py.import("builtins")?
            .getattr("dict")?
            .call1((source_mappings,))?
    } else {
        PyDict::new(py).into_any()
    };
    let accepted = PyList::empty(py);
    let mut rejected = Vec::new();
    let config = value(&preview.getattr("configuration")?)?;
    let game = config["game"]["gamepath"].as_str().unwrap_or("");
    let preview_root = py_path(py, &preview.getattr("root")?.extract::<PathBuf>()?)?;
    for link in preview.getattr("linked_sources")?.try_iter()? {
        check_cancel(Some(cancel))?;
        let link = link?;
        if link.getattr("source")?.is_truthy()? {
            continue;
        }
        let target = link.getattr("target")?;
        if target.is_truthy()? && link.getattr("can_use_target")?.is_truthy()? {
            let target = absolute(&target)?;
            if mo2
                .call_method1("local_mod_target", (&target,))?
                .is_truthy()?
            {
                let target = target.call_method0("resolve")?;
                let mut forbidden = vec![
                    preview_root.clone(),
                    py.import("pathlib")?
                        .getattr("Path")?
                        .call1((destination,))?,
                ];
                if !game.is_empty() {
                    forbidden.push(absolute(&game.into_pyobject(py)?.into_any())?);
                }
                let mut allowed = true;
                for root in forbidden {
                    if target.eq(&root)?
                        || root
                            .call_method1("is_relative_to", (&target,))?
                            .is_truthy()?
                        || (target
                            .call_method1("is_relative_to", (&root,))?
                            .is_truthy()?
                            && !root.eq(&preview_root)?)
                    {
                        allowed = false;
                        break;
                    }
                }
                if allowed {
                    let source = target.str()?;
                    mappings.set_item(link.getattr("alias")?, &source)?;
                    let item = PyDict::new(py);
                    item.set_item("name", link.getattr("name")?)?;
                    item.set_item("source", source)?;
                    accepted.append(item)?;
                    continue;
                }
            }
        }
        rejected.push(format!(
            "{}: linked source requires an explicit ordinary local replacement ({}).",
            link.getattr("name")?.str()?.to_str()?,
            link.getattr("status")?.str()?.to_str()?
        ));
    }
    if !accepted.is_empty() {
        preview = mo2.call_method1("inspect_instance", (&input, cancel, mappings))?;
    }
    Ok((preview, accepted, rejected)
        .into_pyobject(py)?
        .into_any()
        .unbind())
}
#[pyfunction]
fn mo2_migration_augment(
    preview: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = preview.py();
    let mo2 = py.import("creation_lib.mod_manager.mo2")?;
    let profiles = preview.getattr("profiles")?;
    let mut referenced = HashSet::new();
    for profile in profiles.try_iter()? {
        for item in profile?.getattr("mods")?.try_iter()? {
            referenced.insert(item?.getattr("id")?.extract::<String>()?);
        }
    }
    let warnings: Vec<String> = preview.getattr("warnings")?.extract()?;
    preview.setattr(
        "warnings",
        warnings
            .into_iter()
            .filter(|s| !s.starts_with("Executables, downloads, saves"))
            .collect::<Vec<_>>(),
    )?;
    let unassigned = PyList::empty(py);
    for item in preview.getattr("inventory")?.try_iter()? {
        let item = item?;
        if !referenced.contains(&item.getattr("id")?.extract::<String>()?) {
            unassigned.append(item)?;
        }
    }
    if !unassigned.is_empty() {
        let id = mo2.call_method1(
            "identity",
            (format!(
                "{}/migration/unassigned",
                preview.getattr("root")?.extract::<String>()?
            ),),
        )?;
        profiles.call_method1(
            "append",
            (mo2.getattr("ImportedProfile")?.call1((
                id,
                "Unassigned mods (MO2)",
                "",
                unassigned,
                object(
                    py,
                    &json!({"_modbox_auxiliary":"Unreferenced source mod folders"}),
                )?,
            ))?,),
        )?;
    }
    let config = preview.getattr("configuration")?;
    let downloads: PathBuf = config
        .get_item("paths")?
        .get_item("download_directory")?
        .extract()?;
    let inventory = PyList::empty(py);
    let mut issues = Vec::new();
    if downloads.is_dir() {
        reject_links(&downloads)?;
        for entry in py_path(py, &downloads)?
            .call_method0("iterdir")?
            .try_iter()?
        {
            check_cancel(Some(cancel))?;
            if inventory.len() >= 50_000 {
                issues
                    .push("Download metadata exceeds the 50,000-item inventory limit.".to_owned());
                break;
            }
            let entry = entry?;
            let path: PathBuf = entry.extract()?;
            let name: String = entry.getattr("name")?.extract()?;
            let result = (|| -> PyResult<()> {
                reject_links(&path)?;
                if !path.is_file() || crate::saves::fold(&name).ends_with(".meta") {
                    return Ok(());
                }
                let metadata = mo2.call_method1(
                    "read_ini",
                    (py_path(
                        py,
                        &PathBuf::from(format!("{}.meta", path.display())),
                    )?,),
                )?;
                let fields = PyDict::new(py);
                if metadata
                    .call_method1("has_section", ("General",))?
                    .is_truthy()?
                {
                    let general = metadata.get_item("General")?;
                    for key in [
                        "gamename",
                        "modid",
                        "fileid",
                        "name",
                        "version",
                        "filetime",
                        "installed",
                        "removed",
                    ] {
                        if general.contains(key)? {
                            fields.set_item(key, general.get_item(key)?)?;
                        }
                    }
                }
                let item = PyDict::new(py);
                item.set_item("name", &name)?;
                item.set_item("bytes", std::fs::metadata(&path)?.len())?;
                item.set_item("metadata", fields)?;
                item.set_item("source", path.to_string_lossy().as_ref())?;
                inventory.append(item)?;
                Ok(())
            })();
            if let Err(error) = result {
                if error.is_instance_of::<pyo3::exceptions::PyValueError>(py)
                    || error.is_instance_of::<pyo3::exceptions::PyOSError>(py)
                {
                    issues.push(format!(
                        "Skipped download metadata {name}: {}",
                        error.value(py).str()?.to_str()?
                    ));
                } else {
                    return Err(error);
                }
            }
        }
    }
    config.set_item("download_inventory", &inventory)?;
    Ok((inventory, issues).into_pyobject(py)?.into_any().unbind())
}
fn save(result: &Bound<'_, PyAny>) -> PyResult<()> {
    result.getattr("library")?.call_method1(
        "save_migration_run",
        (
            result.getattr("preview")?.getattr("root")?,
            result.getattr("state")?,
            result.call_method0("report")?,
        ),
    )?;
    Ok(())
}
#[pyfunction]
fn mo2_migrate(
    path: &Bound<'_, PyAny>,
    destination: &Bound<'_, PyAny>,
    validate: &Bound<'_, PyAny>,
    prepare: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
    source_mappings: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = path.py();
    let target = absolute(destination)?;
    let root: PathBuf = target.extract()?;
    let module = py.import("creation_lib.mod_manager.mo2_migration")?;
    let result = module.getattr("MigrationResult")?.call0()?;
    let run = (|| -> PyResult<()> {
        progress.call1((
            py.None(),
            "Discovering MO2 profiles and original mod-folder references",
        ))?;
        let discovery = mo2_migration_discover(path, &target, cancel, source_mappings)?;
        let preview = discovery.bind(py).get_item(0)?;
        result.setattr("preview", &preview)?;
        result.setattr("auto_links", discovery.bind(py).get_item(1)?)?;
        result.setattr("issues", discovery.bind(py).get_item(2)?)?;
        let profiles = preview.getattr("profiles")?;
        result.setattr("original_profiles", profiles.len()?)?;
        let issues = result.getattr("issues")?;
        for warning in preview.getattr("warnings")?.try_iter()? {
            let warning = warning?.extract::<String>()?;
            if warning.starts_with("Skipped") && warning.contains("profile folder") {
                issues.call_method1("append", (warning,))?;
            }
        }
        let augmented = mo2_migration_augment(&preview, cancel)?;
        result.setattr("downloads", augmented.bind(py).get_item(0)?.len()?)?;
        issues.call_method1("extend", (augmented.bind(py).get_item(1)?,))?;
        validate.call1((&target, &preview))?;
        reject_links(&root)?;
        let marker: String = py
            .import("creation_lib.mod_manager.library")?
            .getattr("Library")?
            .getattr("MARKER")?
            .extract()?;
        if root.exists()
            && (!root.is_dir()
                || (!root.join(marker).is_file() && std::fs::read_dir(&root)?.next().is_some()))
        {
            return Err(invalid(format!(
                "Destination {} contains unexpected files; choose an empty or manager-owned metadata library.",
                root.display()
            )));
        }
        check_cancel(Some(cancel))?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("create", true)?;
        let library = prepare.call((&target,), Some(&kwargs))?;
        result.setattr("library", &library)?;
        result.setattr("state", "importing")?;
        save(&result)?;
        let mut retired = HashSet::new();
        for profile in library.call_method0("deleted_profiles")?.try_iter()? {
            retired.insert(profile?.get_item("id")?.extract::<String>()?);
        }
        let mut game_content = BTreeSet::new();
        let retired_profiles = PyList::empty(py);
        let mods = PyDict::new(py);
        for profile in profiles.try_iter()? {
            let profile = profile?;
            let deleted = retired.contains(&profile.getattr("id")?.extract::<String>()?);
            if deleted {
                retired_profiles.append(profile.getattr("name")?)?;
            }
            for item in profile.getattr("mods")?.try_iter()? {
                let item = item?;
                let kind: String = item.getattr("kind")?.extract()?;
                if kind == "foreign" {
                    game_content.insert(item.getattr("name")?.extract::<String>()?);
                }
                if !deleted && kind == "external" {
                    mods.set_item(item.getattr("id")?, item)?;
                }
            }
        }
        result.setattr("game_content", game_content.into_iter().collect::<Vec<_>>())?;
        result.setattr("retired_profiles", retired_profiles)?;
        let referenced = PyList::empty(py);
        let unavailable = PyList::empty(py);
        for item in mods.values().iter() {
            if item.getattr("source")?.is_truthy()? {
                referenced.append(item.getattr("name")?)?;
            } else {
                unavailable.append(item.getattr("name")?)?;
            }
        }
        result.setattr("referenced", referenced)?;
        result.setattr("unavailable", &unavailable)?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("cancel", cancel)?;
        kwargs.set_item("progress", progress)?;
        kwargs.set_item("reconcile", true)?;
        kwargs.set_item("checkpoint", checkpoint)?;
        result.setattr(
            "new_profiles",
            library.call_method("import_mo2", (&preview,), Some(&kwargs))?,
        )?;
        if checkpoint.is_truthy()? {
            checkpoint.call1(("profiles-imported",))?;
        }
        result.setattr(
            "state",
            if !unavailable.is_empty() || issues.is_truthy()? {
                "partial"
            } else {
                "complete"
            },
        )?;
        save(&result)?;
        progress.call1((
            1.0,
            "Profiles imported; original MO2 mod folders remain in place",
        ))?;
        Ok(())
    })();
    if let Err(error) = run {
        if error.is_instance_of::<pyo3::exceptions::PyKeyboardInterrupt>(py)
            || error.is_instance_of::<pyo3::exceptions::PySystemExit>(py)
        {
            return Err(error);
        }
        let stopped = error.is_instance_of::<crate::OperationCancelled>(py)
            || error.is_instance(
                py,
                &py.import("creation_lib.mod_manager.paths")?
                    .getattr("Cancelled")?,
            );
        result.setattr("state", if stopped { "cancelled" } else { "failed" })?;
        result
            .getattr("issues")?
            .call_method1("append", (error.value(py).str()?,))?;
        if result.getattr("library")?.is_truthy()? && result.getattr("preview")?.is_truthy()? {
            if let Err(error) = save(&result) {
                if error.is_instance_of::<pyo3::exceptions::PyException>(py) {
                    result.getattr("issues")?.call_method1(
                        "append",
                        ("Migration report needs review when the metadata library reopens.",),
                    )?;
                } else {
                    return Err(error);
                }
            }
        }
        let failure = if stopped {
            module.getattr("MigrationStopped")?.call1((&result,))?
        } else {
            module
                .getattr("MigrationFailure")?
                .call1((&result, error.value(py)))?
        };
        let raised = PyErr::from_value(failure);
        raised.set_cause(py, Some(error));
        return Err(raised);
    }
    Ok(result.unbind())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(mo2_migration_discover, m)?)?;
    m.add_function(wrap_pyfunction!(mo2_migration_augment, m)?)?;
    m.add_function(wrap_pyfunction!(mo2_migrate, m)?)?;
    Ok(())
}
