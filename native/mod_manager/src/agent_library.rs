use crate::{
    database::{error, records},
    history_prune::py_path,
    invalid,
    plugin_inventory::{object, value},
    python_io::with_database,
    reject_links,
    saves::fold,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use serde_json::json;
use std::{collections::HashSet, path::PathBuf};

pub fn library<'py>(py: Python<'py>, root: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let root = py
        .import("pathlib")?
        .getattr("Path")?
        .call1((root,))?
        .call_method0("absolute")?;
    py.import("creation_lib.mod_manager.library")?
        .getattr("Library")?
        .call1((root,))
}
pub fn profile<'py>(
    library: &Bound<'py, PyAny>,
    wanted: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let mut exact = Vec::new();
    let mut named = Vec::new();
    let mut choices = Vec::new();
    let wanted_name = fold(&wanted.str()?.extract::<String>()?);
    for p in library.call_method0("profiles")?.try_iter()? {
        let p = p?;
        let name: String = p.getattr("name")?.extract()?;
        choices.push(format!("{name} ({})", p.getattr("id")?.str()?));
        if p.getattr("id")?.eq(wanted)? {
            exact.push(p.clone());
        }
        if fold(&name) == wanted_name {
            named.push(p);
        }
    }
    let mut matches = if exact.is_empty() { named } else { exact };
    if matches.len() != 1 {
        return Err(invalid(format!(
            "No profile uniquely matches {}. Choose one of: {}",
            wanted.repr()?,
            choices.join(", ")
        )));
    }
    Ok(matches.remove(0))
}
#[pyfunction]
fn agent_default_library(py: Python<'_>) -> PyResult<Py<PyAny>> {
    Ok(py
        .import("creation_lib.mod_manager.locations")?
        .call_method0("default_library_path")?
        .getattr("parent")?
        .call_method1("joinpath", ("agent-library",))?
        .unbind())
}
fn qt(config: &Bound<'_, PyAny>, key: &str, default: &str) -> PyResult<String> {
    config
        .py()
        .import("creation_lib.mod_manager.mo2")?
        .call_method1("qt_string", (config.call_method1("get", (key, default))?,))?
        .extract()
}
#[pyfunction]
fn agent_instances(py: Python<'_>, root: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let root = if root.is_truthy()? {
        root.extract::<PathBuf>()?
    } else {
        py.import("creation_lib.mod_manager.runtime")?
            .call_method1("known_folder", ("F1B32785-6FBA-4FCF-9D55-7B8E7F157091",))?
            .extract::<PathBuf>()?
            .join("ModOrganizer")
    };
    let mut folders = if root.is_dir() {
        std::fs::read_dir(&root)?
            .map(|p| p.map(|p| p.path()))
            .collect::<std::io::Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    folders.sort_by_key(|p| fold(&p.file_name().unwrap_or_default().to_string_lossy()));
    let found = PyList::empty(py);
    for folder in folders {
        let ini = folder.join("ModOrganizer.ini");
        reject_links(&ini)?;
        if !ini.is_file() {
            continue;
        }
        let config = py
            .import("creation_lib.mod_manager.mo2")?
            .call_method1("read_ini", (py_path(py, &ini)?,))?;
        let general = if config
            .call_method1("has_section", ("General",))?
            .is_truthy()?
        {
            config.get_item("General")?
        } else {
            PyDict::new(py).into_any()
        };
        let settings = if config
            .call_method1("has_section", ("Settings",))?
            .is_truthy()?
        {
            config.get_item("Settings")?
        } else {
            PyDict::new(py).into_any()
        };
        let base = PathBuf::from(qt(&settings, "base_directory", &folder.to_string_lossy())?);
        let resolve = |key, default| -> PyResult<PathBuf> {
            Ok(PathBuf::from(
                qt(&settings, key, default)?
                    .replace("%BASE_DIR%", &base.to_string_lossy())
                    .replace('\\', "/"),
            ))
        };
        let profiles = resolve("profiles_directory", "%BASE_DIR%/profiles")?;
        let mut names = Vec::new();
        if profiles.is_dir() {
            for p in std::fs::read_dir(&profiles)? {
                let p = p?;
                if p.path().join("modlist.txt").is_file() {
                    names.push(p.file_name().to_string_lossy().into_owned());
                }
            }
        }
        names.sort_by_key(|n| fold(n));
        found.append(object(py, &json!({"name":folder.file_name().unwrap_or_default().to_string_lossy(), "ini":ini.to_string_lossy(), "game_name":qt(&general, "gameName", "")?, "game_path":qt(&general, "gamePath", "")?, "base_directory":py_path(py, &base)?.str()?.extract::<String>()?, "mods":py_path(py, &resolve("mod_directory", "%BASE_DIR%/mods")?)?.str()?.extract::<String>()?, "profiles_directory":py_path(py, &profiles)?.str()?.extract::<String>()?, "selected_profile":qt(&general, "selected_profile", "")?, "profiles":names}))?)?;
    }
    Ok(found.into_any().unbind())
}
#[pyclass]
struct Validate;
#[pymethods]
impl Validate {
    fn __call__(&self, destination: &Bound<'_, PyAny>, preview: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = destination.py();
        let target = py
            .import("pathlib")?
            .getattr("Path")?
            .call1((destination,))?
            .call_method0("resolve")?;
        let mut paths = vec![
            preview.getattr("root")?,
            preview.getattr("mods_root")?,
            preview.getattr("profiles_root")?,
        ];
        let config = value(&preview.getattr("configuration")?)?;
        if let Some(mapping) = config["source_mappings"].as_object() {
            for source in mapping.values() {
                paths.push(object(py, source)?);
            }
        }
        for source in [
            &config["paths"]["overwrite_directory"],
            &config["game"]["gamepath"],
        ] {
            if source.as_str().is_some_and(|s| !s.is_empty()) {
                paths.push(object(py, source)?);
            }
        }
        for source in paths {
            let source = py
                .import("pathlib")?
                .getattr("Path")?
                .call1((source,))?
                .call_method0("resolve")?;
            if target
                .call_method1("is_relative_to", (&source,))?
                .is_truthy()?
                || source
                    .call_method1("is_relative_to", (&target,))?
                    .is_truthy()?
            {
                return Err(invalid(
                    "Choose an agent library outside the MO2 instance, mod/profile folders and game folder. No library files were created.",
                ));
            }
        }
        Ok(())
    }
}
#[pyclass]
struct Prepare;
#[pymethods]
impl Prepare {
    #[pyo3(signature=(target, *, create))]
    fn __call__(&self, target: &Bound<'_, PyAny>, create: bool) -> PyResult<Py<PyAny>> {
        let py = target.py();
        let kwargs = PyDict::new(py);
        kwargs.set_item("create", create)?;
        Ok(py
            .import("creation_lib.mod_manager.library")?
            .getattr("Library")?
            .call((target,), Some(&kwargs))?
            .unbind())
    }
}
#[pyclass]
struct Progress;
#[pymethods]
impl Progress {
    #[pyo3(signature=(*_args))]
    fn __call__(&self, _args: &Bound<'_, pyo3::types::PyTuple>) {}
}
fn mirror(library: &Bound<'_, PyAny>, preview: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = library.py();
    library.call_method0("_runtime_write_guard")?;
    let result = PyList::empty(py);
    with_database(&library.getattr("_db")?, |_, db| {
        let db = db.connection()?;
        db.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
        for profile in preview.getattr("profiles")?.try_iter()? {
            let profile = profile?;
            let id: String = profile.getattr("id")?.extract()?;
            let rows = records(&db, "SELECT files FROM profiles WHERE id=?", [&id])?;
            if !profile.getattr("source")?.is_truthy()? || rows.is_empty() {
                continue;
            }
            let existing = crate::encoding::parse(rows[0]["files"].as_str().unwrap_or("{}"))?;
            let mut files = value(&profile.getattr("files")?)?;
            for (k, v) in existing
                .as_object()
                .ok_or_else(|| invalid("Invalid profile files."))?
            {
                if k.starts_with("_modbox_") && k != "_modbox_plugin_validation" {
                    files[k] = v.clone();
                }
            }
            let mut foreign = Vec::new();
            let mut managed = Vec::new();
            for row in profile.getattr("mods")?.try_iter()? {
                let row = row?;
                if row.getattr("kind")?.eq("foreign")? {
                    foreign.push(row.getattr("name")?.extract::<String>()?);
                } else {
                    managed.push(row);
                }
            }
            files["_modbox_game_content"] =
                json!(crate::encoding::encode(&json!(foreign), true, true));
            db.execute(
                "UPDATE profiles SET files=? WHERE id=?",
                rusqlite::params![files.to_string(), id],
            )
            .map_err(error)?;
            let mut present = HashSet::new();
            let mut enabled = 0;
            for (priority, row) in managed.iter().enumerate() {
                let mod_id: String = row.getattr("id")?.extract()?;
                let active = row.getattr("enabled")?.is_truthy()?;
                let group: String = row.getattr("group")?.extract()?;
                db.execute("INSERT INTO entries VALUES (?,?,?,?,?) ON CONFLICT(profile_id, mod_id) DO UPDATE SET priority=excluded.priority, enabled=excluded.enabled, group_name=excluded.group_name", rusqlite::params![id, mod_id, priority as i64, active, group]).map_err(error)?;
                present.insert(mod_id);
                if active {
                    enabled += 1;
                }
            }
            let mut priority = managed.len();
            for row in records(
                &db,
                "SELECT mod_id FROM entries WHERE profile_id=? ORDER BY priority, mod_id",
                [&id],
            )? {
                let mod_id = row["mod_id"].as_str().unwrap_or("");
                if !present.contains(mod_id) {
                    db.execute(
                        "UPDATE entries SET enabled=0, priority=? WHERE profile_id=? AND mod_id=?",
                        rusqlite::params![priority as i64, id, mod_id],
                    )
                    .map_err(error)?;
                    priority += 1;
                }
            }
            result.append(object(py, &json!({"id":id, "name":profile.getattr("name")?.extract::<String>()?, "mods":managed.len(), "enabled":enabled}))?)?;
        }
        Ok(())
    })?;
    Ok(result.into_any().unbind())
}
#[pyfunction]
fn agent_sync(instance: &Bound<'_, PyAny>, library_root: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = instance.py();
    let root = py
        .import("pathlib")?
        .getattr("Path")?
        .call1((library_root,))?
        .call_method0("absolute")?;
    let result = py
        .import("creation_lib.mod_manager.mo2_migration")?
        .call_method1(
            "migrate",
            (
                instance,
                root,
                Py::new(py, Validate)?,
                Py::new(py, Prepare)?,
                py.None(),
                Py::new(py, Progress)?,
            ),
        )?;
    let library = result.getattr("library")?;
    let preview = result.getattr("preview")?;
    let output = PyDict::new(py);
    output.set_item("library", library.getattr("root")?.str()?)?;
    output.set_item("instance", preview.getattr("root")?)?;
    output.set_item(
        "game_root",
        object(
            py,
            &value(&preview.getattr("configuration")?)?["game"]["gamepath"]
                .as_str()
                .unwrap_or("")
                .into(),
        )?,
    )?;
    output.set_item("profiles", mirror(&library, &preview)?)?;
    for key in ["state", "unavailable", "issues"] {
        output.set_item(key, result.getattr(key)?)?;
    }
    Ok(output.into_any().unbind())
}
#[pyfunction]
fn agent_import_saves(
    library_root: &Bound<'_, PyAny>,
    wanted: &Bound<'_, PyAny>,
    names: &Bound<'_, PyAny>,
    latest: bool,
    folder: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library_root.py();
    let library = library(py, library_root)?;
    let selected = profile(&library, wanted)?;
    let id = selected.getattr("id")?;
    let source = if folder.is_truthy()? {
        py_path(py, &folder.extract::<PathBuf>()?)?
    } else {
        py.import("creation_lib.mod_manager.global_saves")?
            .call_method1("folder", (&library, &id))?
    };
    let index = py
        .import("creation_lib.mod_manager.saves")?
        .call_method1("inventory_folder", (&library, &id, &source))?;
    let files = index.getattr("files")?;
    let by_name = PyDict::new(py);
    for save in files.try_iter()? {
        let save = save?;
        by_name.set_item(fold(&save.getattr("name")?.extract::<String>()?), save)?;
    }
    let mut wanted: Vec<String> = names.extract()?;
    if latest && files.len()? > 0 {
        wanted.push(files.get_item(0)?.getattr("name")?.extract()?);
    }
    if wanted.is_empty() {
        return Err(invalid(
            "Choose save names or --latest; no save was copied.",
        ));
    }
    let mut missing = Vec::new();
    for name in &wanted {
        if !by_name.contains(fold(name))? {
            missing.push(name.as_str());
        }
    }
    if !missing.is_empty() {
        return Err(invalid(format!(
            "Not in {}: {}. No save was copied.",
            source.str()?,
            missing.join(", ")
        )));
    }
    let transfer = py.import("creation_lib.mod_manager.save_transfer")?;
    let destination = transfer.call_method1("root", (&library, &id))?;
    let mut seen = HashSet::new();
    let mut copied = Vec::new();
    let mut skipped = Vec::new();
    for name in wanted {
        let save = by_name.get_item(fold(&name))?.unwrap();
        let name: String = save.getattr("name")?.extract()?;
        if !seen.insert(name.clone()) {
            continue;
        }
        if destination.extract::<PathBuf>()?.join(&name).exists() {
            skipped.push(json!({"save":name, "reason":"already in owned saves"}));
            continue;
        }
        let review =
            transfer.call_method1("review", (&library, &index, save.getattr("path")?, &id))?;
        transfer.call_method1("apply", (&library, review))?;
        copied.push(name);
    }
    Ok(object(py, &json!({"source":source.str()?.extract::<String>()?, "saves":destination.str()?.extract::<String>()?, "copied":copied, "skipped":skipped}))?.unbind())
}
#[pyfunction]
fn agent_empty_mod(
    root: &Bound<'_, PyAny>,
    wanted: &Bound<'_, PyAny>,
    name: &str,
    after: &str,
) -> PyResult<Py<PyAny>> {
    let py = root.py();
    let library = library(py, root)?;
    let profile = profile(&library, wanted)?.getattr("id")?;
    let id = py
        .import("creation_lib.mod_manager.organization")?
        .call_method1("empty_mod", (&library, profile, name, after))?;
    let id: String = id.extract()?;
    let path = library
        .getattr("root")?
        .extract::<PathBuf>()?
        .join("mods")
        .join(&id);
    Ok(object(py, &json!({"mod_id":id,"name":name.trim(),"path":path}))?.unbind())
}

#[pyfunction]
fn agent_launch_options(
    root: &Bound<'_, PyAny>,
    wanted: &Bound<'_, PyAny>,
    isolate_documents: Option<bool>,
) -> PyResult<Py<PyAny>> {
    let py = root.py();
    let library = library(py, root)?;
    let id = profile(&library, wanted)?.getattr("id")?;
    let isolated: bool = py
        .import("creation_lib.mod_manager.profile_state")?
        .call_method1("isolate_documents", (&library, &id, isolate_documents))?
        .extract()?;
    Ok(object(
        py,
        &json!({"profile_id":id.extract::<String>()?, "isolate_documents":isolated}),
    )?
    .unbind())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(agent_launch_options, m)?)?;
    m.add_function(wrap_pyfunction!(agent_empty_mod, m)?)?;
    m.add_function(wrap_pyfunction!(agent_default_library, m)?)?;
    m.add_function(wrap_pyfunction!(agent_instances, m)?)?;
    m.add_function(wrap_pyfunction!(agent_sync, m)?)?;
    m.add_function(wrap_pyfunction!(agent_import_saves, m)?)?;
    Ok(())
}
