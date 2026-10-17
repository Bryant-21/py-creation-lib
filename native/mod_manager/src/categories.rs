use crate::{
    check_cancel,
    database::{Database, error},
    encoding, invalid, relative_path,
};
use pyo3::prelude::*;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use unicode_casefold::UnicodeCaseFold;

pub fn fold(value: &str) -> String {
    value.case_fold().collect()
}

pub fn name(value: &Value) -> PyResult<String> {
    let value = value
        .as_str()
        .filter(|s| s.chars().count() <= 200)
        .ok_or_else(|| invalid("Category names must be at most 200 characters."))?;
    relative_path(value.trim(), false)
}

pub fn memberships(primary: &str, encoded: &str) -> PyResult<Vec<String>> {
    if encoded.is_empty() {
        return Ok(if primary.is_empty() {
            vec![]
        } else {
            vec![primary.into()]
        });
    }
    let value: Value = serde_json::from_str(encoded)
        .map_err(|e| invalid(format!("Local category memberships are invalid: {e}")))?;
    let values=value.as_array().filter(|a|a.len()<=32 && a.iter().all(|v|v.as_str().is_some_and(|s|!s.is_empty() && s.chars().count()<=200)))
        .ok_or_else(||invalid("Local category memberships are invalid: Expected at most 32 nonempty category names of 200 characters."))?;
    let mut seen = BTreeSet::new();
    let result: Vec<String> = values
        .iter()
        .map(|v| v.as_str().unwrap())
        .filter(|v| seen.insert(fold(v)))
        .map(str::to_owned)
        .collect();
    if result.first().map(String::as_str).unwrap_or("") != primary {
        return Err(invalid(
            "Local category memberships are invalid: Primary category differs from the stored first membership.",
        ));
    }
    Ok(result)
}

pub fn normalized(value: Value) -> PyResult<Vec<String>> {
    let values = match value {
        Value::String(value) => {
            if value.is_empty() {
                vec![]
            } else {
                vec![Value::String(value)]
            }
        }
        Value::Array(values) if values.len() <= 32 => values,
        _ => {
            return Err(invalid(
                "Choose at most 32 category paths; the first is primary.",
            ));
        }
    };
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for value in values {
        let value = name(&value)?;
        if seen.insert(fold(&value)) {
            result.push(value);
        }
    }
    Ok(result)
}

type Snapshot = (Vec<(String, i64)>, Vec<(String, Vec<String>)>);
fn snapshot(connection: &Connection) -> PyResult<Snapshot> {
    let configured = connection
        .prepare("SELECT name,nexus_id FROM categories ORDER BY name COLLATE NOCASE")
        .map_err(error)?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(error)?;
    let raw=connection.prepare("SELECT id,category,categories_json FROM mods WHERE category!='' OR categories_json NOT IN ('','[]') ORDER BY id").map_err(error)?
        .query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(error)?.collect::<rusqlite::Result<Vec<_>>>().map_err(error)?;
    let assigned = raw
        .into_iter()
        .map(|(id, primary, encoded)| Ok((id, memberships(&primary, &encoded)?)))
        .collect::<PyResult<Vec<_>>>()?;
    Ok((configured, assigned))
}

fn token(connection: &Connection) -> PyResult<String> {
    Ok(encoding::digest(&json!(snapshot(connection)?), true, true))
}

#[pyfunction]
fn category_name(value: &str) -> PyResult<String> {
    name(&encoding::parse(value)?)
}
#[pyfunction]
fn category_memberships(primary: &str, encoded: &str) -> PyResult<Vec<String>> {
    memberships(primary, encoded)
}
#[pyfunction]
fn category_normalized(value: &str) -> PyResult<Vec<String>> {
    normalized(encoding::parse(value)?)
}
#[pyfunction]
fn category_matches(values: Vec<String>, category: Option<&str>) -> bool {
    match category {
        None => true,
        Some("") => values.is_empty(),
        Some(category) => values.iter().any(|v| {
            let v = fold(v);
            v == category || v.starts_with(&format!("{category}/"))
        }),
    }
}
#[pyfunction]
fn category_snapshot(db: &Database) -> PyResult<Snapshot> {
    snapshot(&*db.connection()?)
}
#[pyfunction]
fn category_token(db: &Database) -> PyResult<String> {
    token(&*db.connection()?)
}

#[pyfunction]
fn category_inventory(db: &Database) -> PyResult<(Vec<(String, i64, usize)>, String)> {
    let connection = db.connection()?;
    let (configured, assigned) = snapshot(&connection)?;
    let mut rows: BTreeMap<String, (String, i64, usize)> = configured
        .into_iter()
        .map(|(name, id)| (fold(&name), (name, id, 0)))
        .collect();
    for (_, values) in assigned {
        for value in values {
            rows.entry(fold(&value)).or_insert((value, 0, 0)).2 += 1;
        }
    }
    Ok((rows.into_values().collect(), token(&connection)?))
}

#[pyfunction]
#[pyo3(signature = (db,value,nexus_id,cancel=None))]
fn category_save(
    db: &Database,
    value: &str,
    nexus_id: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let mut value = name(&encoding::parse(value)?)?;
    let id = encoding::parse(nexus_id)?
        .as_i64()
        .filter(|v| (0..=1_000_000).contains(v))
        .ok_or_else(|| {
            invalid(
                "Nexus category ID must be between zero and 1,000,000; zero leaves it unmapped.",
            )
        })?;
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    check_cancel(cancel)?;
    let (configured, _) = snapshot(&connection)?;
    if let Some((existing, _)) = configured
        .iter()
        .find(|(original, _)| fold(original) == fold(&value))
    {
        value = existing.clone();
    } else if configured.len() >= 5000 {
        return Err(invalid("Category taxonomy exceeds its 5,000-name limit."));
    }
    connection.execute("INSERT INTO categories(name,nexus_id) VALUES (?,?) ON CONFLICT(name) DO UPDATE SET nexus_id=excluded.nexus_id",params![value,id]).map_err(error)?;
    check_cancel(cancel)
}

#[pyfunction]
#[pyo3(signature = (db,original,replacement,expected,cancel=None))]
fn category_rename(
    db: &Database,
    original: &str,
    replacement: &str,
    expected: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let replacement = name(&encoding::parse(replacement)?)?;
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    if token(&connection)? != expected {
        return Err(invalid(
            "Categories or shared mod assignments changed after review. Refresh before renaming.",
        ));
    }
    let (configured, assigned) = snapshot(&connection)?;
    let mut names: BTreeMap<_, _> = configured
        .iter()
        .map(|(v, _)| (fold(v), v.clone()))
        .collect();
    for (_, values) in &assigned {
        for value in values {
            names.insert(fold(value), value.clone());
        }
    }
    let original_key = fold(original);
    if !names.contains_key(&original_key) {
        return Err(invalid("Select an existing category first."));
    }
    let included = |value: &str| {
        let value = fold(value);
        value == original_key || value.starts_with(&format!("{original_key}/"))
    };
    let mapping: BTreeMap<_, _> = names
        .iter()
        .filter(|(_, v)| included(v))
        .map(|(k, v)| {
            (
                k.clone(),
                format!(
                    "{replacement}{}",
                    v.chars().skip(original.chars().count()).collect::<String>()
                ),
            )
        })
        .collect();
    if mapping
        .values()
        .any(|v| names.contains_key(&fold(v)) && !mapping.contains_key(&fold(v)))
    {
        return Err(invalid(
            "Renamed category would collide with another category. Assign or move those mods explicitly first.",
        ));
    }
    if mapping
        .values()
        .map(|v| fold(v))
        .collect::<BTreeSet<_>>()
        .len()
        != mapping.len()
    {
        return Err(invalid("Renamed category would create ambiguous names."));
    }
    for value in mapping.values() {
        name(&json!(value))?;
    }
    check_cancel(cancel)?;
    for (value, _) in &configured {
        if included(value) {
            connection
                .execute("DELETE FROM categories WHERE name=?", [value])
                .map_err(error)?;
        }
    }
    for (value, id) in &configured {
        if included(value) {
            connection.execute("UPDATE nexus_category_mapping SET local_name=? WHERE local_name=? COLLATE NOCASE",
                params![mapping[&fold(value)], value]).map_err(error)?;
            connection
                .execute(
                    "INSERT INTO categories(name,nexus_id) VALUES (?,?)",
                    params![mapping[&fold(value)], id],
                )
                .map_err(error)?;
        }
    }
    for (id, values) in assigned {
        check_cancel(cancel)?;
        if values.iter().any(|v| included(v)) {
            let values: Vec<_> = values
                .into_iter()
                .map(|v| mapping.get(&fold(&v)).cloned().unwrap_or(v))
                .collect();
            connection
                .execute(
                    "UPDATE mods SET category=?,categories_json=? WHERE id=?",
                    params![
                        values.first(),
                        encoding::encode(&json!(values), true, true),
                        id
                    ],
                )
                .map_err(error)?;
        }
    }
    check_cancel(cancel)
}

#[pyfunction]
#[pyo3(signature = (db,value,expected,cancel=None))]
fn category_remove(
    db: &Database,
    value: &str,
    expected: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    if token(&connection)? != expected {
        return Err(invalid(
            "Category assignments changed. Refresh before removing an unused category.",
        ));
    }
    let (configured, assigned) = snapshot(&connection)?;
    let key = fold(value);
    let prefix = format!("{key}/");
    if assigned.iter().flat_map(|(_, v)| v).any(|v| {
        let v = fold(v);
        v == key || v.starts_with(&prefix)
    }) {
        return Err(invalid(
            "This category or its descendants have assigned mods. Assign those mods elsewhere before removing it.",
        ));
    }
    if configured.iter().any(|(v, _)| fold(v).starts_with(&prefix)) {
        return Err(invalid("Remove or rename child categories first."));
    }
    check_cancel(cancel)?;
    if connection
        .execute("DELETE FROM categories WHERE name=?", [value])
        .map_err(error)?
        == 0
    {
        return Err(invalid("Only a configured unused category can be removed."));
    }
    check_cancel(cancel)
}

#[pyfunction]
#[pyo3(signature = (db,profile,selected,category,cancel=None))]
fn category_assign(
    db: &Database,
    profile: &str,
    selected: Vec<String>,
    category: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let values = normalized(encoding::parse(category)?)?;
    let selected: BTreeSet<_> = selected.into_iter().collect();
    if selected.is_empty() {
        return Err(invalid(
            "Select one or more mods before assigning a category.",
        ));
    }
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    let known = connection
        .prepare("SELECT mod_id FROM entries WHERE profile_id=?")
        .map_err(error)?
        .query_map([profile], |r| r.get::<_, String>(0))
        .map_err(error)?
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .map_err(error)?;
    if !selected.is_subset(&known) {
        return Err(invalid(
            "Selected mods no longer belong to this local profile.",
        ));
    }
    check_cancel(cancel)?;
    let configured: BTreeMap<_, _> = snapshot(&connection)?
        .0
        .into_iter()
        .map(|(v, _)| (fold(&v), v))
        .collect();
    let values: Vec<_> = values
        .into_iter()
        .map(|v| configured.get(&fold(&v)).cloned().unwrap_or(v))
        .collect();
    for id in selected {
        connection
            .execute(
                "UPDATE mods SET category=?,categories_json=? WHERE id=?",
                params![
                    values.first().map(String::as_str).unwrap_or(""),
                    encoding::encode(&json!(values), true, true),
                    id
                ],
            )
            .map_err(error)?;
    }
    check_cancel(cancel)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(category_name, m)?)?;
    m.add_function(wrap_pyfunction!(category_memberships, m)?)?;
    m.add_function(wrap_pyfunction!(category_normalized, m)?)?;
    m.add_function(wrap_pyfunction!(category_matches, m)?)?;
    m.add_function(wrap_pyfunction!(category_snapshot, m)?)?;
    m.add_function(wrap_pyfunction!(category_token, m)?)?;
    m.add_function(wrap_pyfunction!(category_inventory, m)?)?;
    m.add_function(wrap_pyfunction!(category_save, m)?)?;
    m.add_function(wrap_pyfunction!(category_rename, m)?)?;
    m.add_function(wrap_pyfunction!(category_remove, m)?)?;
    m.add_function(wrap_pyfunction!(category_assign, m)?)?;
    Ok(())
}
