use crate::{
    check_cancel,
    database::{Database, error},
    encoding,
    executables::{definition, launch_local_path, text},
    invalid,
    target_settings::targets,
};
use pyo3::prelude::*;
use rusqlite::params;
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf};
use unicode_casefold::UnicodeCaseFold;

fn fold(value: &str) -> String {
    value.case_fold().collect()
}
fn value(object: &Bound<'_, PyAny>) -> PyResult<Value> {
    encoding::parse(
        &object
            .py()
            .import("json")?
            .call_method1("dumps", (object,))?
            .extract::<String>()?,
    )
}
fn stamp(path: &std::path::Path, fingerprint: &Bound<'_, PyAny>) -> PyResult<Value> {
    value(&fingerprint.call1((path,))?)
}

#[pyfunction]
fn target_review(
    db: &Database,
    library: &Bound<'_, PyAny>,
    request: &str,
    fingerprint: &Bound<'_, PyAny>,
    reader: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let py = library.py();
    let request = encoding::parse(request)?;
    let id = text(&request, "identifier");
    let automatic = text(&request, "binary").is_empty();
    let (_, game) = crate::instances::validate_root(
        py,
        "fo4",
        &PathBuf::from(text(&request, "game_root")),
        &library.getattr("root")?.extract::<PathBuf>()?,
    )?;
    let binary = if automatic {
        game.join(if game.join("f4se_loader.exe").is_file() {
            "f4se_loader.exe"
        } else {
            "Fallout4.exe"
        })
        .to_string_lossy()
        .into_owned()
    } else {
        text(&request, "binary").into()
    };
    let cwd = if text(&request, "working_directory").is_empty() {
        game.to_string_lossy().into_owned()
    } else {
        text(&request, "working_directory").into()
    };
    let executable = definition(
        json!({"id":id,"title":request["name"],"binary":binary,"arguments":request["arguments"],"working_directory":cwd,"steam_app_id":"377160"}),
    )?;
    let mut chosen = launch_local_path(text(&executable, "binary"), "Target executable", false)?;
    let path_type = py.import("pathlib")?.getattr("Path")?;
    let parent = path_type
        .call1((chosen.parent().unwrap(),))?
        .call_method0("resolve")?;
    if !parent.eq(path_type.call1((&game,))?)?
        || !matches!(
            fold(&chosen.file_name().unwrap().to_string_lossy()).as_str(),
            "fallout4.exe" | "f4se_loader.exe"
        )
    {
        return Err(invalid(
            "Each target must use its own Fallout4.exe or f4se_loader.exe.",
        ));
    }
    let cwd = launch_local_path(
        text(&executable, "working_directory"),
        "Target working directory",
        true,
    )?;
    let info = path_type.call1((&cwd,))?.call_method0("stat")?;
    let directory = json!([
        cwd.to_string_lossy(),
        [
            info.getattr("st_dev")?.extract::<u64>()?,
            info.getattr("st_ino")?.extract::<u64>()?
        ]
    ]);
    for path in [&chosen, &game.join("Fallout4.exe")] {
        if crate::pe_metadata::pe_machine(path.to_owned())? != 0x8664 {
            return Err(invalid(
                "FO4 launch targets require x64 game/loader executables.",
            ));
        }
    }
    let mut defaults = json!({"Fallout4.ini":"[General]\n","Fallout4Prefs.ini":"[General]\n","Fallout4Custom.ini":"[Archive]\nbInvalidateOlderFiles=1\nsResourceDataDirsFinal=\n"});
    let mut minimal = vec!["Fallout4.ini", "Fallout4Prefs.ini"];
    let mut inputs = Vec::new();
    let default = game.join("Fallout4_default.ini");
    crate::reject_links(&default)?;
    if default.is_file() {
        let before = stamp(&default, fingerprint)?;
        let text = reader
            .call1((path_type.call1((&default,))?, 2 * 1024 * 1024))?
            .extract::<String>()?;
        if stamp(&default, fingerprint)? != before {
            return Err(invalid("Target default INI changed while reading it."));
        }
        crate::profile_state::profile_validate_ini(py, "Fallout4.ini", &text)?;
        defaults["Fallout4.ini"] = text.into();
        minimal.remove(0);
        inputs.push(json!([default.to_string_lossy(), before]));
    }
    defaults["_modbox_minimal_inis"] = encoding::encode(&json!(minimal), true, true).into();
    for path in BTreeSet::from([chosen.clone(), game.join("Fallout4.exe")]) {
        inputs.push(json!([path.to_string_lossy(), stamp(&path, fingerprint)?]));
    }
    inputs.push(directory);
    let mut runtime = value(
        &py.import("creation_lib.mod_manager.f4se_setup")?
            .call_method1("inspect_folder", (path_type.call1((&game,))?, cancel))?,
    )?;
    check_cancel(Some(cancel))?;
    let existing = targets(db)?;
    let duplicate = existing
        .iter()
        .find(|v| fold(text(v, "name")) == fold(text(&executable, "title")));
    let review = |target: &Value| -> PyResult<String> {
        Ok(json!({"library_id":library.getattr("library_id")?.extract::<String>()?,"target":target,"expected":encoding::digest(&json!(existing),false,false),"inputs":inputs}).to_string())
    };
    if id.is_empty() {
        if let Some(duplicate) = duplicate {
            let same_binary = fold(text(duplicate, "binary")) == fold(&chosen.to_string_lossy())
                || automatic && duplicate["f4se"]["managed"].as_bool().unwrap_or_default();
            if !same_binary
                || fold(text(duplicate, "game_root")) != fold(&game.to_string_lossy())
                || fold(text(duplicate, "working_directory")) != fold(&cwd.to_string_lossy())
                || duplicate["arguments"] != request["arguments"]
            {
                return Err(invalid(
                    "This target name already refers to a different configuration. Edit it or use another name.",
                ));
            }
            if duplicate["f4se"]["game_signature"] != runtime["game_signature"] {
                return Err(invalid(
                    "This target game executable changed. Edit the selected folder and prepare its matching F4SE again.",
                ));
            }
            return review(duplicate);
        }
    }
    if !id.is_empty() && !existing.iter().any(|v| v["id"] == id) {
        return Err(invalid("This launch target no longer exists."));
    }
    if duplicate.is_some_and(|v| v["id"] != id) {
        return Err(invalid("Choose a unique launch target name."));
    }
    if id.is_empty() && existing.len() >= 32 {
        return Err(invalid("This library already has 32 launch targets."));
    }
    if let Some(previous) = existing.iter().find(|v| v["id"] == id) {
        if previous["f4se"]
            .get("root_setup")
            .is_some_and(|v| !v.is_null() && v != false && v != "" && v != &json!({}))
        {
            return Err(invalid(
                "Restore this target's game-root setup before editing its folder/runtime definition.",
            ));
        }
        if fold(text(previous, "game_root")) == fold(&game.to_string_lossy())
            && previous["f4se"]["managed"].as_bool().unwrap_or_default()
            && previous["f4se"]["game_signature"] == runtime["game_signature"]
        {
            runtime = previous["f4se"].clone();
            chosen = PathBuf::from(text(previous, "binary"));
        }
    }
    review(
        &json!({"id":executable["id"],"name":executable["title"],"game_root":game.to_string_lossy(),"binary":chosen.to_string_lossy(),"working_directory":cwd.to_string_lossy(),"arguments":executable["arguments"],"defaults":defaults,"f4se":runtime}),
    )
}

fn inputs(plan: &Value, fingerprint: &Bound<'_, PyAny>) -> PyResult<()> {
    let path_type = fingerprint.py().import("pathlib")?.getattr("Path")?;
    for input in plan["inputs"]
        .as_array()
        .ok_or_else(|| invalid("Invalid target input review."))?
    {
        let path = PathBuf::from(input[0].as_str().unwrap_or_default());
        crate::reject_links(&path)?;
        let info = path_type.call1((&path,))?.call_method0("stat")?;
        let actual = if input[1].as_array().is_some_and(|v| v.len() == 2) {
            json!([
                info.getattr("st_dev")?.extract::<u64>()?,
                info.getattr("st_ino")?.extract::<u64>()?
            ])
        } else {
            stamp(&path, fingerprint)?
        };
        if actual != input[1] {
            return Err(invalid(
                "Target executable/default INI or working directory changed after review.",
            ));
        }
    }
    Ok(())
}

#[pyfunction]
fn target_apply(
    db: &Database,
    library: &Bound<'_, PyAny>,
    plan: &str,
    fingerprint: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let plan = encoding::parse(plan)?;
    if plan["library_id"] != library.getattr("library_id")?.extract::<String>()? {
        return Err(invalid("Library changed after target review."));
    }
    library.call_method0("_runtime_write_guard")?;
    inputs(&plan, fingerprint)?;
    db.connection()?
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(error)?;
    if encoding::digest(&json!(targets(db)?), false, false) != text(&plan, "expected") {
        return Err(invalid(
            "Launch target configurations changed after review.",
        ));
    }
    check_cancel(Some(cancel))?;
    db.connection()?.execute_batch("CREATE TABLE IF NOT EXISTS launch_targets(id TEXT PRIMARY KEY,name TEXT NOT NULL,record TEXT NOT NULL); CREATE TABLE IF NOT EXISTS launch_target_profiles(target_id TEXT NOT NULL,profile_id TEXT NOT NULL,files TEXT NOT NULL,PRIMARY KEY(target_id,profile_id))").map_err(error)?;
    let target = &plan["target"];
    db.connection()?.execute("INSERT INTO launch_targets VALUES (?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,record=excluded.record",params![text(target,"id"),text(target,"name"),target.to_string()]).map_err(error)?;
    checkpoint.call0()?;
    check_cancel(Some(cancel))?;
    inputs(&plan, fingerprint)?;
    library.call_method0("_runtime_write_guard")?;
    Ok(text(target, "id").into())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(target_executable, m)?)?;
    m.add_function(wrap_pyfunction!(target_recover, m)?)?;
    m.add_function(wrap_pyfunction!(target_review, m)?)?;
    m.add_function(wrap_pyfunction!(target_apply, m)?)?;
    Ok(())
}

#[pyfunction]
fn target_executable(record: &str) -> PyResult<String> {
    let target = encoding::parse(record)?;
    Ok(json!({"id":crate::executables::identity(&format!("{}/executable",text(&target,"id"))),"title":target["name"],"binary":target["binary"],"arguments":target["arguments"],"working_directory":target["working_directory"],"mode":"profile","instance_id":"","hidden":false,"steam_app_id":"377160"}).to_string())
}

#[pyfunction]
fn target_recover(library: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    let py = library.py();
    let root = library.getattr("root")?.extract::<PathBuf>()?;
    let targets = crate::python_io::with_database(&library.getattr("_db")?, |_, db| targets(db))?;
    let mut report = Vec::new();
    for target in targets {
        let path = root.join(".launch-targets").join(text(&target, "id"));
        crate::reject_links(&path)?;
        if !path.is_dir() {
            continue;
        }
        let view = py
            .import("creation_lib.mod_manager.launch_targets")?
            .getattr("TargetLibrary")?
            .call1((library, text(&target, "id")))?;
        for module in ["save_transfer", "overwrite", "backup_prune"] {
            report.extend(
                py.import(format!("creation_lib.mod_manager.{module}"))?
                    .call_method1("recover", (&view,))?
                    .extract::<Vec<String>>()?,
            );
        }
        let setup = py.import("creation_lib.mod_manager.f4se_root_setup")?;
        let lock = setup.getattr("LOCK")?.extract::<String>()?;
        for receipt in setup
            .call_method1("history", (library, text(&target, "id")))?
            .try_iter()?
        {
            let receipt = receipt?;
            let state = receipt.get_item("state")?.extract::<String>()?;
            if matches!(state.as_str(), "pending" | "restoring")
                || state == "complete"
                    && PathBuf::from(receipt.get_item("game")?.extract::<String>()?)
                        .join(&lock)
                        .exists()
            {
                report.push(format!("{}: game-root F4SE setup needs explicit reviewed restore/recovery ({}). Reopening did not change game files.",text(&target,"name"),receipt.get_item("id")?.extract::<String>()?));
            }
        }
        let stages = path.join("f4se");
        crate::reject_links(&stages)?;
        if stages.is_dir() {
            for (index, entry) in std::fs::read_dir(stages)?.enumerate() {
                if index >= 256 {
                    return Err(invalid(
                        "Target F4SE retention exceeds 256 folders; review retained stages before retrying.",
                    ));
                }
                let entry = entry?;
                crate::reject_links(&entry.path())?;
                if entry.file_name().to_string_lossy().starts_with("pending-") {
                    report.push(format!("{}: interrupted F4SE preparation retained at {}. It is ignored by Run; retry prepares a new stage.",text(&target,"name"),entry.path().display()));
                }
            }
        }
    }
    Ok(report)
}
