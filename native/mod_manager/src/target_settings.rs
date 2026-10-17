use crate::{
    check_cancel,
    database::{Database, error, records},
    encoding,
    executables::{definition, identity, text},
    invalid,
};
use pyo3::prelude::*;
use rusqlite::params;
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf};
use unicode_casefold::UnicodeCaseFold;

pub fn targets(db: &Database) -> PyResult<Vec<Value>> {
    if records(
        &*db.connection()?,
        "SELECT 1 FROM sqlite_master WHERE name='launch_targets' AND type='table'",
        [],
    )?
    .is_empty()
    {
        return Ok(vec![]);
    }
    let mut result = vec![];
    for row in records(
        &*db.connection()?,
        "SELECT id,record FROM launch_targets ORDER BY name COLLATE NOCASE,id LIMIT 33",
        [],
    )? {
        let mut value = encoding::parse(text(&row, "record"))?;
        if crate::transactions::transaction_key(text(&value, "id"))? != text(&row, "id")
            || text(&value, "name").chars().count() > 200
            || !value["defaults"].is_object()
        {
            return Err(invalid("Launch target metadata is invalid."));
        }
        if value.get("f4se").is_none() {
            value["f4se"] = json!({});
        }
        result.push(value);
    }
    if result.len() > 32 {
        return Err(invalid(
            "Launch target metadata exceeds its 32-target limit.",
        ));
    }
    Ok(result)
}

#[pyfunction]
fn target_records(db: &Database) -> PyResult<String> {
    Ok(json!(targets(db)?).to_string())
}

fn target(db: &Database, id: &str, message: &str) -> PyResult<Value> {
    targets(db)?
        .into_iter()
        .find(|v| text(v, "id") == id)
        .ok_or_else(|| invalid(message))
}

fn valid_scopes(value: &Value) -> bool {
    value.as_object().is_some_and(|v| {
        v.len() == 2
            && ["ini", "saves"].iter().all(|key| {
                matches!(
                    v.get(*key).and_then(Value::as_str),
                    Some("target" | "shared")
                )
            })
    })
}

pub fn preferences(db: &Database, id: &str) -> PyResult<Value> {
    let mut value = json!({"ini":"target","saves":"target"});
    if !records(
        &*db.connection()?,
        "SELECT 1 FROM sqlite_master WHERE name='launch_target_preferences'",
        [],
    )?
    .is_empty()
    {
        if let Some(row) = records(
            &*db.connection()?,
            "SELECT record FROM launch_target_preferences WHERE target_id=?",
            [id],
        )?
        .first()
        {
            value = encoding::parse(text(row, "record"))?;
        }
    }
    if !valid_scopes(&value) {
        return Err(invalid(
            "Target INI/save preferences contain unsupported scopes.",
        ));
    }
    Ok(value)
}

#[pyfunction]
fn target_arguments(py: Python<'_>, arguments: &Bound<'_, PyAny>) -> PyResult<String> {
    let arguments = arguments
        .extract::<String>()
        .map_err(|_| invalid("Target arguments exceed the bounded command-line limit."))?;
    validate_arguments(py, &arguments)
}
fn validate_arguments(py: Python<'_>, arguments: &str) -> PyResult<String> {
    if arguments.chars().count() > 32700 {
        return Err(invalid(
            "Target arguments exceed the bounded command-line limit.",
        ));
    }
    let tokens = crate::windows_arguments(arguments)?;
    let mut seen = BTreeSet::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].case_fold().collect::<String>();
        if !seen.insert(token.clone()) {
            return Err(invalid("Use each target launch option at most once."));
        }
        let next = tokens.get(index + 1);
        if matches!(token.as_str(), "-v" | "-waitforclose") {
            index += 1;
        } else if token == "-priority"
            && next.is_some_and(|v| {
                ["above_normal", "below_normal", "normal", "idle", "high"]
                    .contains(&v.to_lowercase().as_str())
            })
        {
            index += 2;
        } else if token == "-affinity" && next.is_some() {
            let value = py
                .get_type::<pyo3::types::PyInt>()
                .call1((next.unwrap(), 0))
                .map_err(|_| invalid("Affinity must be a positive decimal or 0x-prefixed mask."))?;
            if !value.extract::<i64>().is_ok_and(|v| v > 0) {
                return Err(invalid(
                    "Affinity mask must fit a positive signed 64-bit value.",
                ));
            }
            index += 2;
        } else {
            return Err(invalid(
                "Supported target options: -v, -waitforclose, -priority above_normal/below_normal/normal/idle/high, -affinity <positive mask>. Other bootstrap/Steam/editor/debugger overrides are blocked.",
            ));
        }
    }
    Ok(arguments.into())
}

fn argument_text(record: &Value) -> PyResult<&str> {
    record["arguments"]
        .as_str()
        .ok_or_else(|| invalid("Target arguments exceed the bounded command-line limit."))
}

pub fn options(py: Python<'_>, db: &Database, id: &str) -> PyResult<Value> {
    let target = target(db, id, "Select an active FO4 target first.")?;
    let mut arguments = text(&target, "arguments").to_string();
    if !records(
        &*db.connection()?,
        "SELECT 1 FROM sqlite_master WHERE name='launch_target_options'",
        [],
    )?
    .is_empty()
    {
        if let Some(row) = records(
            &*db.connection()?,
            "SELECT arguments FROM launch_target_options WHERE target_id=?",
            [id],
        )?
        .first()
        {
            arguments = validate_arguments(py, text(row, "arguments"))?;
        }
    }
    Ok(json!({"arguments":arguments}))
}

fn state(py: Python<'_>, db: &Database, id: &str, profile: &str, kind: &str) -> PyResult<Value> {
    if kind == "options" {
        return Ok(json!([
            target(db, id, "This target no longer exists.")?,
            options(py, db, id)?
        ]));
    }
    let target = target(db, id, "Choose an active target and logical profile.")?;
    let profile_rows = records(
        &*db.connection()?,
        "SELECT files FROM profiles WHERE id=?",
        [profile],
    )?;
    let profile_row = profile_rows
        .first()
        .ok_or_else(|| invalid("Choose an active target and logical profile."))?;
    let overlay = records(
        &*db.connection()?,
        "SELECT files FROM launch_target_profiles WHERE target_id=? AND profile_id=?",
        params![id, profile],
    )?;
    Ok(json!([
        target,
        preferences(db, id)?,
        profile_row["files"],
        overlay.first().map(|r| r["files"].clone())
    ]))
}

#[pyfunction]
fn target_setting_value(py: Python<'_>, db: &Database, id: &str, kind: &str) -> PyResult<String> {
    Ok(if kind == "options" {
        options(py, db, id)?
    } else {
        preferences(db, id)?
    }
    .to_string())
}

fn fingerprints(inputs: &Value, fingerprint: &Bound<'_, PyAny>) -> PyResult<()> {
    let json = fingerprint.py().import("json")?;
    for input in inputs
        .as_array()
        .ok_or_else(|| invalid("Invalid target input review."))?
    {
        let path = PathBuf::from(input[0].as_str().unwrap_or(""));
        crate::reject_links(&path)?;
        let stamp = fingerprint.call1((path,))?;
        let stamp = encoding::parse(&json.call_method1("dumps", (stamp,))?.extract::<String>()?)?;
        if stamp != input[1] {
            return Err(invalid("Target executable changed after arguments review."));
        }
    }
    Ok(())
}

#[pyfunction]
#[pyo3(signature=(db, library, action, kind, id, profile="", record="null", plan="null", fingerprint=None, checkpoint=None, cancel=None))]
fn target_setting(
    db: &Database,
    library: &Bound<'_, PyAny>,
    action: &str,
    kind: &str,
    id: &str,
    profile: &str,
    record: &str,
    plan: &str,
    fingerprint: Option<&Bound<'_, PyAny>>,
    checkpoint: Option<&Bound<'_, PyAny>>,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let py = library.py();
    let record = encoding::parse(record)?;
    if action == "review" {
        if kind == "preferences" && !valid_scopes(&record) {
            return Err(invalid("Choose target-local or manager-shared state."));
        }
        if kind == "options" {
            validate_arguments(py, argument_text(&record)?)?;
        }
        let state = state(py, db, id, profile, kind)?;
        let expected = encoding::digest(&state, false, false);
        let library_id: String = library.getattr("library_id")?.extract()?;
        check_cancel(cancel)?;
        if kind == "preferences" {
            return Ok(json!({"library_id":library_id,"target_id":id,"profile_id":profile,"record":record,"expected":expected}).to_string());
        }
        let target = &state[0];
        definition(
            json!({"id":identity(&format!("{id}/executable")),"title":target["name"],"binary":target["binary"],"arguments":record["arguments"],"working_directory":target["working_directory"],"steam_app_id":"377160"}),
        )?;
        let mut inputs = vec![];
        let json = py.import("json")?;
        let game = PathBuf::from(text(target, "game_root")).join("Fallout4.exe");
        let paths = BTreeSet::from([
            text(target, "binary").to_string(),
            game.to_string_lossy().into_owned(),
        ]);
        for path in paths {
            crate::reject_links(std::path::Path::new(&path))?;
            let stamp = fingerprint.unwrap().call1((&path,))?;
            let stamp =
                encoding::parse(&json.call_method1("dumps", (stamp,))?.extract::<String>()?)?;
            inputs.push(json!([path, stamp]));
        }
        check_cancel(cancel)?;
        return Ok(json!({"library_id":library_id,"target_id":id,"arguments":record["arguments"],"expected":expected,"inputs":inputs}).to_string());
    }
    let plan = encoding::parse(plan)?;
    let library_id: String = library.getattr("library_id")?.extract()?;
    if text(&plan, "library_id") != library_id {
        return Err(invalid(if kind == "options" {
            "Library changed after arguments review."
        } else {
            "Library changed after preference review."
        }));
    }
    library.call_method0("_runtime_write_guard")?;
    db.connection()?
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(error)?;
    if encoding::digest(&state(py, db, id, profile, kind)?, false, false) != text(&plan, "expected")
    {
        return Err(invalid(if kind == "options" {
            "Target arguments/definition changed after review."
        } else {
            "Target/profile preferences changed after review."
        }));
    }
    if kind == "options" {
        let arguments = validate_arguments(py, argument_text(&plan)?)?;
        fingerprints(&plan["inputs"], fingerprint.unwrap())?;
        check_cancel(cancel)?;
        db.connection()?.execute_batch("CREATE TABLE IF NOT EXISTS launch_target_options(target_id TEXT PRIMARY KEY,arguments TEXT NOT NULL)").map_err(error)?;
        db.connection()?.execute("INSERT INTO launch_target_options VALUES (?,?) ON CONFLICT(target_id) DO UPDATE SET arguments=excluded.arguments",params![id,arguments]).map_err(error)?;
    } else {
        if !valid_scopes(&plan["record"]) {
            return Err(invalid("Unsupported preference scope."));
        }
        db.connection()?.execute_batch("CREATE TABLE IF NOT EXISTS launch_target_preferences(target_id TEXT PRIMARY KEY,record TEXT NOT NULL)").map_err(error)?;
        db.connection()?.execute("INSERT INTO launch_target_preferences VALUES (?,?) ON CONFLICT(target_id) DO UPDATE SET record=excluded.record",params![id,encoding::encode(&plan["record"],true,true)]).map_err(error)?;
    }
    if let Some(checkpoint) = checkpoint {
        checkpoint.call0()?;
    }
    check_cancel(cancel)?;
    if kind == "options" {
        fingerprints(&plan["inputs"], fingerprint.unwrap())?;
    }
    library.call_method0("_runtime_write_guard")?;
    Ok(json!(id).to_string())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(target_records, m)?)?;
    m.add_function(wrap_pyfunction!(target_arguments, m)?)?;
    m.add_function(wrap_pyfunction!(target_setting_value, m)?)?;
    m.add_function(wrap_pyfunction!(target_setting, m)?)?;
    Ok(())
}
