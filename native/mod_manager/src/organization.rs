use crate::{
    check_cancel,
    database::{Database, error},
    encoding, invalid, relative_path,
};
use pyo3::prelude::*;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use unicode_casefold::UnicodeCaseFold;

pub fn entries(connection: &Connection, profile: &str) -> PyResult<Vec<Value>> {
    let mut statement = connection.prepare("SELECT mod_id,priority,enabled,group_name FROM entries WHERE profile_id=? ORDER BY priority,mod_id").map_err(error)?;
    statement.query_map([profile], |row| Ok(json!({"mod_id": row.get::<_,String>(0)?, "priority": row.get::<_,i64>(1)?, "enabled": row.get::<_,i64>(2)?, "group_name": row.get::<_,String>(3)?})))
        .map_err(error)?.collect::<rusqlite::Result<Vec<_>>>().map_err(error)
}

pub fn history(
    connection: &Connection,
    profile: &str,
    before: &[Value],
    kind: &str,
) -> PyResult<()> {
    let after = entries(connection, profile)?;
    if before != after {
        connection.execute("INSERT INTO organization_history(id,profile_id,kind,snapshot,expected) VALUES (?,?,?,?,?)",
            params![uuid::Uuid::new_v4().simple().to_string(), profile, kind, serde_json::to_string(before).unwrap(), encoding::digest(&json!(after), true, true)]).map_err(error)?;
    }
    Ok(())
}

#[pyfunction]
fn organization_entries(db: &Database, profile: &str) -> PyResult<String> {
    Ok(json!(entries(&*db.connection()?, profile)?).to_string())
}

#[pyfunction]
fn organization_digest(rows: &str) -> PyResult<String> {
    Ok(encoding::digest(&encoding::parse(rows)?, true, true))
}

#[pyfunction]
fn organization_history(db: &Database, profile: &str, before: &str, kind: &str) -> PyResult<()> {
    let value = encoding::parse(before)?;
    history(
        &*db.connection()?,
        profile,
        value
            .as_array()
            .ok_or_else(|| invalid("Invalid organization snapshot."))?,
        kind,
    )
}

#[pyfunction]
#[pyo3(signature = (db, profile, selected, enabled=None, group=None, priority=None, expected=None, cancel=None, checkpoint=None))]
fn organization_bulk(
    db: &Database,
    profile: &str,
    selected: Vec<String>,
    enabled: Option<bool>,
    group: Option<&str>,
    priority: Option<i64>,
    expected: Option<&str>,
    cancel: Option<&Bound<'_, PyAny>>,
    checkpoint: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let selected: BTreeSet<_> = selected.into_iter().collect();
    if selected.is_empty() {
        return Err(invalid("Select one or more mods first."));
    }
    let group = group
        .map(|value| relative_path(value.trim(), true))
        .transpose()?;
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    let before = entries(&connection, profile)?;
    if expected.is_some_and(|value| value != encoding::digest(&json!(before), true, true)) {
        return Err(invalid(
            "Profile entries changed during the reviewed operation. Refresh and try again.",
        ));
    }
    let known: BTreeSet<_> = before
        .iter()
        .map(|row| row["mod_id"].as_str().unwrap().to_string())
        .collect();
    if !selected.is_subset(&known) {
        return Err(invalid("Selected mods no longer belong to this profile."));
    }
    check_cancel(cancel)?;
    for id in &selected {
        if let Some(enabled) = enabled {
            connection.execute("UPDATE entries SET enabled=? WHERE profile_id=? AND mod_id=? AND mod_id IN (SELECT id FROM mods WHERE kind!='separator')", params![enabled,profile,id]).map_err(error)?;
        }
        if let Some(group) = &group {
            connection
                .execute(
                    "UPDATE entries SET group_name=? WHERE profile_id=? AND mod_id=?",
                    params![group, profile, id],
                )
                .map_err(error)?;
        }
    }
    if let Some(priority) = priority {
        let (moving, mut remaining): (Vec<_>, Vec<_>) = before
            .iter()
            .map(|row| row["mod_id"].as_str().unwrap())
            .partition(|id| selected.contains(*id));
        let target = priority.clamp(0, remaining.len() as i64) as usize;
        remaining.splice(target..target, moving);
        for (index, id) in remaining.iter().enumerate() {
            connection
                .execute(
                    "UPDATE entries SET priority=? WHERE profile_id=? AND mod_id=?",
                    params![index as i64, profile, id],
                )
                .map_err(error)?;
        }
    }
    history(&connection, profile, &before, "bulk")?;
    drop(connection);
    if let Some(checkpoint) = checkpoint {
        checkpoint.call0()?;
    }
    check_cancel(cancel)
}

#[pyfunction]
#[pyo3(signature = (db, profile, name, group="", cancel=None))]
fn organization_separator(
    db: &Database,
    profile: &str,
    name: &str,
    group: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 200 || name.chars().any(|c| c < ' ') {
        return Err(invalid(
            "Give the group a short name without control characters.",
        ));
    }
    let group = relative_path(
        if group.trim().is_empty() {
            name
        } else {
            group.trim()
        },
        false,
    )?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    if !connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM profiles WHERE id=?)",
            [profile],
            |row| row.get::<_, bool>(0),
        )
        .map_err(error)?
    {
        return Err(invalid("Select a local profile first."));
    }
    let before = entries(&connection, profile)?;
    connection
        .execute(
            "INSERT INTO mods(id,name,source,kind) VALUES (?,?,'','separator')",
            params![id, name],
        )
        .map_err(error)?;
    connection
        .execute(
            "INSERT INTO entries VALUES (?,?,?,0,?)",
            params![profile, id, before.len() as i64, group],
        )
        .map_err(error)?;
    history(&connection, profile, &before, "group")?;
    check_cancel(cancel)?;
    Ok(id)
}

#[pyfunction]
#[pyo3(signature = (db, profile, cancel=None))]
fn organization_undo(
    db: &Database,
    profile: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    let (id,kind,snapshot,expected)=connection.query_row("SELECT id,kind,snapshot,expected FROM organization_history WHERE profile_id=? ORDER BY sequence DESC LIMIT 1",[profile],|r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional().map_err(error)?.ok_or_else(|| invalid("This profile has no organization operation to undo."))?;
    let current = entries(&connection, profile)?;
    if encoding::digest(&json!(current), true, true) != expected {
        return Err(invalid(
            "Profile entries changed after this operation. Undo is blocked to preserve later installs/imports/edits.",
        ));
    }
    let saved = encoding::parse(&snapshot)?;
    let saved = saved
        .as_array()
        .ok_or_else(|| invalid("Invalid organization snapshot."))?;
    for row in saved {
        let metadata = connection
            .query_row(
                "SELECT source,kind FROM mods WHERE id=?",
                [row["mod_id"].as_str().unwrap_or("")],
                |r| Ok(json!({"source":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?})),
            )
            .optional()
            .map_err(error)?
            .ok_or_else(|| {
                invalid("A mod needed by this undo is unavailable. Restore its content first.")
            })?;
        if let Some(removed) = row.get("removed_metadata") {
            if removed != &metadata {
                return Err(invalid(
                    "Removed reference source or ownership changed. Undo is blocked to preserve the new target.",
                ));
            }
        }
    }
    check_cancel(cancel)?;
    connection
        .execute("DELETE FROM entries WHERE profile_id=?", [profile])
        .map_err(error)?;
    for row in saved {
        connection
            .execute(
                "INSERT INTO entries VALUES (?,?,?,?,?)",
                params![
                    profile,
                    row["mod_id"].as_str(),
                    row["priority"].as_i64(),
                    row["enabled"].as_i64(),
                    row["group_name"].as_str()
                ],
            )
            .map_err(error)?;
        if kind == "reference-removal" && !current.iter().any(|r| r["mod_id"] == row["mod_id"]) {
            connection
                .execute(
                    "DELETE FROM reference_exclusions WHERE profile_id=? AND mod_id=?",
                    params![profile, row["mod_id"].as_str()],
                )
                .map_err(error)?;
        }
    }
    connection
        .execute("DELETE FROM organization_history WHERE id=?", [id])
        .map_err(error)?;
    check_cancel(cancel)
}

fn groups(rows: &[Value]) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    for row in rows {
        let parts: Vec<_> = row["group_name"]
            .as_str()
            .unwrap_or("")
            .split('/')
            .collect();
        for length in 1..=parts.len() {
            let value = parts[..length].join("/");
            if !value.is_empty() {
                result.insert(value);
            }
        }
    }
    result
}

#[pyfunction]
#[pyo3(signature = (db, profile, original, cancel=None))]
fn group_review(
    db: &Database,
    profile: &str,
    original: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<(usize, String)> {
    let rows = entries(&*db.connection()?, profile)?;
    if !groups(&rows).contains(original) {
        return Err(invalid("Choose an existing local organization group."));
    }
    check_cancel(cancel)?;
    Ok((
        rows.iter()
            .filter(|row| {
                let value = row["group_name"].as_str().unwrap();
                value == original || value.starts_with(&format!("{original}/"))
            })
            .count(),
        encoding::digest(&json!(rows), true, true),
    ))
}

#[pyfunction]
#[pyo3(signature = (db, profile, original, replacement, expected, library_id, reviewed_library_id, cancel=None, checkpoint=None))]
fn group_apply(
    db: &Database,
    profile: &str,
    original: &str,
    replacement: &str,
    expected: &str,
    library_id: &str,
    reviewed_library_id: &str,
    cancel: Option<&Bound<'_, PyAny>>,
    checkpoint: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    if replacement.chars().count() > 200 {
        return Err(invalid("Use a group path of at most 200 characters."));
    }
    let replacement = relative_path(replacement.trim(), false)?;
    if library_id != reviewed_library_id {
        return Err(invalid("Library changed; review the group again."));
    }
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    let before = entries(&connection, profile)?;
    if encoding::digest(&json!(before), true, true) != expected {
        return Err(invalid("Profile entries changed; review this group again."));
    }
    let all = groups(&before);
    let included = |value: &str| value == original || value.starts_with(&format!("{original}/"));
    let mapping: BTreeMap<_, _> = all
        .iter()
        .filter(|v| included(v))
        .map(|v| (v.clone(), format!("{replacement}{}", &v[original.len()..])))
        .collect();
    if mapping.is_empty() {
        return Err(invalid("The reviewed group no longer exists."));
    }
    let remaining: BTreeSet<String> = all
        .iter()
        .filter(|v| !included(v))
        .map(|v| v.case_fold().collect())
        .collect();
    let new_names: BTreeSet<String> = mapping.values().map(|v| v.case_fold().collect()).collect();
    if !remaining.is_disjoint(&new_names) || new_names.len() != mapping.len() {
        return Err(invalid(
            "Renamed hierarchy would collide with another group. Choose a distinct path.",
        ));
    }
    if mapping.values().any(|v| v.chars().count() > 200) {
        return Err(invalid(
            "A renamed descendant path would exceed 200 characters.",
        ));
    }
    check_cancel(cancel)?;
    for row in &before {
        if let Some(replacement) = mapping.get(row["group_name"].as_str().unwrap()) {
            connection
                .execute(
                    "UPDATE entries SET group_name=? WHERE profile_id=? AND mod_id=?",
                    params![replacement, profile, row["mod_id"].as_str()],
                )
                .map_err(error)?;
        }
    }
    history(&connection, profile, &before, "group-rename")?;
    drop(connection);
    if let Some(checkpoint) = checkpoint {
        checkpoint.call0()?;
    }
    check_cancel(cancel)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(organization_entries, m)?)?;
    m.add_function(wrap_pyfunction!(organization_digest, m)?)?;
    m.add_function(wrap_pyfunction!(organization_history, m)?)?;
    m.add_function(wrap_pyfunction!(organization_bulk, m)?)?;
    m.add_function(wrap_pyfunction!(organization_separator, m)?)?;
    m.add_function(wrap_pyfunction!(organization_undo, m)?)?;
    m.add_function(wrap_pyfunction!(group_review, m)?)?;
    m.add_function(wrap_pyfunction!(group_apply, m)?)?;
    Ok(())
}
