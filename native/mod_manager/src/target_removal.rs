use crate::{
    check_cancel,
    database::{Database, error, records},
    encoding,
    executables::text,
    invalid,
    target_settings::targets,
    transactions::transaction_key,
};
use pyo3::prelude::*;
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use unicode_casefold::UnicodeCaseFold;

fn removed(db: &Database) -> PyResult<Vec<Value>> {
    if records(
        &*db.connection()?,
        "SELECT 1 FROM sqlite_master WHERE name='removed_launch_targets'",
        [],
    )?
    .is_empty()
    {
        return Ok(Vec::new());
    }
    let mut values = Vec::new();
    for row in records(
        &*db.connection()?,
        "SELECT id,record FROM removed_launch_targets ORDER BY name COLLATE NOCASE,id LIMIT 257",
        [],
    )? {
        let mut value = encoding::parse(text(&row, "record"))?;
        if transaction_key(text(&value, "id"))? != text(&row, "id")
            || text(&value, "name").chars().count() > 200
            || !value["defaults"].is_object()
        {
            return Err(invalid("Removed target metadata is invalid."));
        }
        if value.get("f4se").is_none() {
            value["f4se"] = json!({});
        }
        values.push(value);
    }
    if values.len() > 256 {
        return Err(invalid(
            "Retained target metadata exceeds its 256-target limit.",
        ));
    }
    Ok(values)
}

#[pyfunction]
fn target_removed(db: &Database) -> PyResult<String> {
    Ok(json!(removed(db)?).to_string())
}

fn state(db: &Database, id: &str, restore: bool) -> PyResult<(Value, String)> {
    let active = targets(db)?;
    let retired = removed(db)?;
    let target = (if restore { &retired } else { &active })
        .iter()
        .find(|v| v["id"] == id)
        .cloned()
        .ok_or_else(|| invalid("This active/removed target changed; review it again."))?;
    let mut overlays = json!({});
    for (table, fields) in [
        (
            "launch_target_profiles",
            &["target_id", "profile_id", "files"][..],
        ),
        ("launch_target_options", &["target_id", "arguments"]),
        (
            "launch_target_mod_sources",
            &["target_id", "mod_id", "record"],
        ),
        ("launch_target_preferences", &["target_id", "record"]),
    ] {
        if !records(
            &*db.connection()?,
            "SELECT 1 FROM sqlite_master WHERE name=?",
            [table],
        )?
        .is_empty()
        {
            let rows = records(
                &*db.connection()?,
                &format!("SELECT * FROM {table} WHERE target_id=?"),
                [id],
            )?;
            let mut rows: Vec<Vec<String>> = rows
                .iter()
                .map(|row| fields.iter().map(|k| text(row, k).into()).collect())
                .collect();
            rows.sort();
            overlays[table] = json!(rows);
        }
    }
    Ok((
        target,
        encoding::digest(&json!([active, retired, overlays]), false, false),
    ))
}

fn history_digest(library: &Bound<'_, PyAny>, id: &str) -> PyResult<String> {
    let root = library
        .getattr("root")?
        .extract::<PathBuf>()?
        .join(".launch-targets")
        .join(transaction_key(id)?)
        .join("f4se-root-history");
    crate::reject_links(&root)?;
    let mut values = Vec::new();
    if root.is_dir() {
        for (index, folder) in std::fs::read_dir(root)?.enumerate() {
            if index >= 64 {
                return Err(invalid("Retained root history exceeds its limit."));
            }
            let folder = folder?;
            let receipt = folder.path().join("receipt.json");
            crate::reject_links(&receipt)?;
            if !receipt.is_file() || std::fs::metadata(&receipt)?.len() > 128 * 1024 {
                return Err(invalid("Retained root receipt changed or is invalid."));
            }
            values.push((
                folder.file_name().to_string_lossy().into_owned(),
                format!("{:x}", Sha256::digest(std::fs::read(receipt)?)),
            ));
        }
    }
    values.sort();
    Ok(encoding::digest(&json!(values), false, false))
}

fn root_lock(py: Python<'_>, target: &Value) -> PyResult<()> {
    let name = py
        .import("creation_lib.mod_manager.f4se_root_setup")?
        .getattr("LOCK")?
        .extract::<String>()?;
    let lock = PathBuf::from(text(target, "game_root")).join(name);
    crate::reject_links(&lock)?;
    if lock.exists() {
        return Err(invalid(
            "Resolve pending game-root ownership before removing/restoring target metadata.",
        ));
    }
    Ok(())
}

fn guard(db: &Database, library: &Bound<'_, PyAny>, target: &Value, restore: bool) -> PyResult<()> {
    root_lock(library.py(), target)?;
    let id = text(target, "id");
    if restore {
        let stored = records(
            &*db.connection()?,
            "SELECT root_history FROM removed_launch_targets WHERE id=?",
            [id],
        )?;
        let current = history_digest(library, id)?;
        if stored.first().is_none_or(|v| v["root_history"] != current) {
            return Err(invalid(
                "Retained root history changed after target removal; preserve it for review.",
            ));
        }
    } else {
        let setup = &target["f4se"]["root_setup"];
        let configured = match setup {
            Value::Null => false,
            Value::Bool(v) => *v,
            Value::String(v) => !v.is_empty(),
            Value::Object(v) => !v.is_empty(),
            Value::Array(v) => !v.is_empty(),
            Value::Number(v) => v.as_f64() != Some(0.0),
        };
        if configured {
            return Err(invalid(
                "Restore/recover this target's managed game-root setup before removing its metadata.",
            ));
        }
        for item in library
            .py()
            .import("creation_lib.mod_manager.f4se_root_setup")?
            .call_method1("history", (library, id))?
            .try_iter()?
        {
            if !matches!(
                item?.get_item("state")?.extract::<String>()?.as_str(),
                "restored" | "rolled-back"
            ) {
                return Err(invalid(
                    "Restore/recover this target's managed game-root setup before removing its metadata.",
                ));
            }
        }
    }
    Ok(())
}

#[pyfunction]
fn target_removal_review(
    db: &Database,
    library: &Bound<'_, PyAny>,
    id: &str,
    restore: bool,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let (target, signature) = state(db, id, restore)?;
    guard(db, library, &target, restore)?;
    check_cancel(Some(cancel))?;
    let root = library
        .getattr("root")?
        .extract::<PathBuf>()?
        .join(".launch-targets")
        .join(id);
    Ok(json!({"library_id":library.getattr("library_id")?.extract::<String>()?,"target_id":id,"name":target["name"],"retained_root":root.to_string_lossy(),"expected":signature,"restore":restore,"root_history":history_digest(library,id)?}).to_string())
}

#[pyfunction]
fn target_removal_apply(
    db: &Database,
    library: &Bound<'_, PyAny>,
    plan: &str,
    cancel: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let plan = encoding::parse(plan)?;
    if plan["library_id"] != library.getattr("library_id")?.extract::<String>()? {
        return Err(invalid(
            "Library changed after target removal/restore review.",
        ));
    }
    library.call_method0("_runtime_write_guard")?;
    db.connection()?
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(error)?;
    let id = text(&plan, "target_id");
    let restore = plan["restore"].as_bool().unwrap_or_default();
    let (target, signature) = state(db, id, restore)?;
    if signature != text(&plan, "expected") {
        return Err(invalid(
            "Target configuration/overlays changed after review.",
        ));
    }
    guard(db, library, &target, restore)?;
    if history_digest(library, id)? != text(&plan, "root_history") {
        return Err(invalid(
            "Root history changed after target removal/restore review.",
        ));
    }
    db.connection()?.execute_batch("CREATE TABLE IF NOT EXISTS removed_launch_targets(id TEXT PRIMARY KEY,name TEXT NOT NULL,record TEXT NOT NULL,root_history TEXT NOT NULL)").map_err(error)?;
    check_cancel(Some(cancel))?;
    if restore {
        let active = targets(db)?;
        let name = text(&target, "name").case_fold().collect::<String>();
        if active.len() >= 32
            || active
                .iter()
                .any(|v| v["id"] == id || text(v, "name").case_fold().collect::<String>() == name)
        {
            return Err(invalid(
                "The active target limit or a conflicting target name prevents restore.",
            ));
        }
        db.connection()?
            .execute(
                "INSERT INTO launch_targets VALUES (?,?,?)",
                params![id, text(&target, "name"), target.to_string()],
            )
            .map_err(error)?;
        db.connection()?
            .execute("DELETE FROM removed_launch_targets WHERE id=?", [id])
            .map_err(error)?;
    } else {
        if removed(db)?.len() >= 256 {
            return Err(invalid(
                "The retained target limit prevents further removal; no files were deleted.",
            ));
        }
        let history = history_digest(library, id)?;
        db.connection()?
            .execute(
                "INSERT INTO removed_launch_targets VALUES (?,?,?,?)",
                params![id, text(&target, "name"), target.to_string(), history],
            )
            .map_err(error)?;
        db.connection()?
            .execute("DELETE FROM launch_targets WHERE id=?", [id])
            .map_err(error)?;
    }
    checkpoint.call0()?;
    check_cancel(Some(cancel))?;
    root_lock(library.py(), &target)?;
    if history_digest(library, id)? != text(&plan, "root_history") {
        return Err(invalid("Root history changed before metadata commit."));
    }
    library.call_method0("_runtime_write_guard")?;
    Ok(id.into())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(target_removed, m)?)?;
    m.add_function(wrap_pyfunction!(target_removal_review, m)?)?;
    m.add_function(wrap_pyfunction!(target_removal_apply, m)?)?;
    Ok(())
}
