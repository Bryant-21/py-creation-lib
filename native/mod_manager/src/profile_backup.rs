use crate::{
    check_cancel,
    database::{Database, error, records},
    encoding,
    executables::text,
    invalid,
};
use pyo3::prelude::*;
use rusqlite::params;
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[pyfunction]
fn metadata_encode(value: &str) -> PyResult<String> {
    Ok(encoding::encode(&encoding::parse(value)?, false, false))
}
#[pyfunction]
fn metadata_digest(value: &str) -> PyResult<String> {
    Ok(digest(&encoding::parse(value)?))
}
fn digest(value: &Value) -> String {
    encoding::digest(value, false, false)
}

pub fn snapshot(db: &Database, profile: &str) -> PyResult<Value> {
    let connection = db.connection()?;
    let rows = records(
        &connection,
        "SELECT name,source,files FROM profiles WHERE id=?",
        [profile],
    )?;
    let row = rows
        .first()
        .ok_or_else(|| invalid("The local profile no longer exists."))?;
    Ok(
        json!({"version":1,"profile_id":profile,"name":row["name"],"source":row["source"],"files":encoding::parse(text(row,"files"))?,"entries":crate::organization::entries(&connection,profile)?}),
    )
}

#[pyfunction]
fn metadata_snapshot(db: &Database, profile: &str) -> PyResult<String> {
    Ok(snapshot(db, profile)?.to_string())
}

fn scoped_snapshot(
    db: &Database,
    wrapper: &Bound<'_, PyAny>,
    library: &Bound<'_, PyAny>,
    profile: &str,
) -> PyResult<Value> {
    let mut data = snapshot(db, profile)?;
    if library.hasattr("profile_files")? {
        let json = library.py().import("json")?;
        let files = library.call_method1(
            "profile_files",
            (
                wrapper,
                profile,
                json.call_method1("loads", (data["files"].to_string(),))?,
            ),
        )?;
        data["files"] =
            encoding::parse(&json.call_method1("dumps", (files,))?.extract::<String>()?)?;
    }
    Ok(data)
}

fn scope(target: &str, data: &Value) -> PyResult<()> {
    if data["files"]
        .get("_modbox_target_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        != target
    {
        return Err(invalid(
            "This metadata backup belongs to another launch target.",
        ));
    }
    Ok(())
}
fn count(db: &Database, target: &str, profile: &str) -> PyResult<i64> {
    db.connection()?.query_row("SELECT count(*) FROM profile_backups WHERE profile_id=? AND coalesce(json_extract(snapshot,'$.files._modbox_target_id'),'')=?",params![profile,target],|r|r.get(0)).map_err(error)
}
fn validate_sources(db: &Database, data: &Value, profile: &str) -> PyResult<()> {
    if data["version"] != 1 || data["profile_id"] != profile || !data["files"].is_object() {
        return Err(invalid("Unsupported profile backup format or identity."));
    }
    let entries = data["entries"]
        .as_array()
        .ok_or_else(|| invalid("Unsupported profile backup format or identity."))?;
    let ids: BTreeSet<_> = entries.iter().map(|entry| text(entry, "mod_id")).collect();
    let available: BTreeSet<_> = records(&*db.connection()?, "SELECT id FROM mods", [])?
        .into_iter()
        .map(|row| text(&row, "id").to_string())
        .collect();
    if ids.len() != entries.len() || ids.iter().any(|id| !available.contains(*id)) {
        return Err(invalid(
            "A mod needed by this backup is missing. Restore its owned content before restoring the profile.",
        ));
    }
    Ok(())
}

#[pyfunction]
#[pyo3(signature = (db, wrapper, library, action, profile, backup="", plan="null", cancel=None))]
fn profile_backup(
    db: &Database,
    wrapper: &Bound<'_, PyAny>,
    library: &Bound<'_, PyAny>,
    action: &str,
    profile: &str,
    backup: &str,
    plan: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let target = library
        .getattr("target_id")
        .map(|v| v.extract::<String>())
        .unwrap_or(Ok(String::new()))?;
    if action == "history" {
        return Ok(json!(records(&*db.connection()?,"SELECT id,created,length(snapshot) AS characters FROM profile_backups WHERE profile_id=? AND coalesce(json_extract(snapshot,'$.files._modbox_target_id'),'')=? ORDER BY created DESC,rowid DESC",params![profile,target])?).to_string());
    }
    db.connection()?
        .execute_batch(if action == "review" {
            "BEGIN"
        } else {
            "BEGIN IMMEDIATE"
        })
        .map_err(error)?;
    check_cancel(cancel)?;
    if action == "capture" {
        if count(db, &target, profile)? >= 100 {
            return Err(invalid(
                "This profile has 100 metadata backups. Remove an old backup before creating another.",
            ));
        }
        let data = encoding::encode(
            &scoped_snapshot(db, wrapper, library, profile)?,
            false,
            false,
        );
        if data.len() > 64 * 1024 * 1024 {
            return Err(invalid("Profile backup exceeds the 64 MiB metadata limit."));
        }
        check_cancel(cancel)?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        db.connection()?
            .execute(
                "INSERT INTO profile_backups(id,profile_id,snapshot) VALUES (?,?,?)",
                params![id, profile, data],
            )
            .map_err(error)?;
        check_cancel(cancel)?;
        return Ok(json!(id).to_string());
    }
    let saved = records(
        &*db.connection()?,
        "SELECT snapshot FROM profile_backups WHERE id=? AND profile_id=?",
        params![backup, profile],
    )?
    .first()
    .map(|row| encoding::parse(text(row, "snapshot")))
    .transpose()?;
    if action == "remove" {
        if let Some(saved) = &saved {
            scope(&target, saved)?;
        }
        check_cancel(cancel)?;
        if db
            .connection()?
            .execute(
                "DELETE FROM profile_backups WHERE id=? AND profile_id=?",
                params![backup, profile],
            )
            .map_err(error)?
            == 0
        {
            return Err(invalid("This local metadata backup is unavailable."));
        }
        check_cancel(cancel)?;
        return Ok("null".into());
    }
    if action == "review" {
        let saved =
            saved.ok_or_else(|| invalid("Choose an existing backup of this local profile."))?;
        scope(&target, &saved)?;
        validate_sources(db, &saved, profile)?;
        let current = scoped_snapshot(db, wrapper, library, profile)?;
        check_cancel(cancel)?;
        return Ok(json!({"profile_id":profile,"backup_id":backup,"expected":digest(&current),"backup_digest":digest(&saved),"name":saved["name"],"entries":saved["entries"].as_array().unwrap().len(),"files":saved["files"].as_object().unwrap().len()}).to_string());
    }
    if action != "apply" {
        return Err(invalid("Unsupported profile backup operation."));
    }
    let plan = encoding::parse(plan)?;
    let current = scoped_snapshot(db, wrapper, library, profile)?;
    if digest(&current) != text(&plan, "expected") {
        return Err(invalid(
            "Profile changed after backup restore review. Review it again before restoring.",
        ));
    }
    let saved = saved.ok_or_else(|| invalid("The reviewed profile backup is unavailable."))?;
    scope(&target, &saved)?;
    validate_sources(db, &saved, profile)?;
    if digest(&saved) != text(&plan, "backup_digest") {
        return Err(invalid("The profile backup changed after review."));
    }
    if saved["files"]["_modbox_instance_id"] != current["files"]["_modbox_instance_id"] {
        return Err(invalid(
            "Profile backup game/instance context differs from the current profile.",
        ));
    }
    if count(db, &target, profile)? >= 100 {
        return Err(invalid(
            "Remove an old backup first; restoring retains a backup of the current state.",
        ));
    }
    check_cancel(cancel)?;
    let encoded = encoding::encode(&current, false, false);
    if encoded.len() > 64 * 1024 * 1024 {
        return Err(invalid(
            "Current profile backup exceeds the 64 MiB safety limit.",
        ));
    }
    db.connection()?
        .execute(
            "INSERT INTO profile_backups(id,profile_id,snapshot) VALUES (?,?,?)",
            params![uuid::Uuid::new_v4().simple().to_string(), profile, encoded],
        )
        .map_err(error)?;
    if library.hasattr("write_profile_fields")? {
        let files = library
            .py()
            .import("json")?
            .call_method1("loads", (saved["files"].to_string(),))?;
        library.call_method1("write_profile_fields", (wrapper, profile, files))?;
        db.connection()?
            .execute(
                "UPDATE profiles SET name=? WHERE id=?",
                params![text(&saved, "name"), profile],
            )
            .map_err(error)?;
    } else {
        db.connection()?
            .execute(
                "UPDATE profiles SET name=?,files=? WHERE id=?",
                params![
                    text(&saved, "name"),
                    encoding::encode(&saved["files"], false, false),
                    profile
                ],
            )
            .map_err(error)?;
    }
    db.connection()?
        .execute("DELETE FROM entries WHERE profile_id=?", [profile])
        .map_err(error)?;
    for row in saved["entries"].as_array().unwrap() {
        db.connection()?
            .execute(
                "INSERT INTO entries VALUES (?,?,?,?,?)",
                params![
                    profile,
                    text(row, "mod_id"),
                    row["priority"].as_i64().unwrap_or(0),
                    row["enabled"].as_i64().unwrap_or(0),
                    text(row, "group_name")
                ],
            )
            .map_err(error)?;
    }
    check_cancel(cancel)?;
    Ok("null".into())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(metadata_encode, m)?)?;
    m.add_function(wrap_pyfunction!(metadata_digest, m)?)?;
    m.add_function(wrap_pyfunction!(metadata_snapshot, m)?)?;
    m.add_function(wrap_pyfunction!(profile_backup, m)?)?;
    Ok(())
}
