use crate::{
    categories, check_cancel,
    database::{Database, error},
    encoding, invalid, organization,
};
use pyo3::prelude::*;
use rusqlite::{Connection, params, types::Value as SqlValue};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const FIELDS: [&str; 5] = ["notes", "category", "version", "nexus", "priority"];

fn state(connection: &Connection, profile: &str, id: &str, field: Option<&str>) -> PyResult<Value> {
    let sql = if field.is_some() {
        "SELECT m.*,e.priority,e.enabled,e.group_name FROM mods m JOIN entries e ON m.id=e.mod_id WHERE e.profile_id=? AND m.id=?"
    } else {
        "SELECT m.id,m.name,m.source,m.kind FROM mods m JOIN entries e ON e.mod_id=m.id WHERE e.profile_id=? AND m.id=?"
    };
    let mut statement = connection.prepare(sql).map_err(error)?;
    let names: Vec<_> = statement
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut rows = statement.query(params![profile, id]).map_err(error)?;
    let missing = if field.is_some() {
        "Choose an owned or referenced mod in this local profile."
    } else {
        "Select an owned or referenced mod in this local profile."
    };
    let row = rows
        .next()
        .map_err(error)?
        .ok_or_else(|| invalid(missing))?;
    let mut result = serde_json::Map::new();
    for (index, name) in names.into_iter().enumerate() {
        let value = match row.get::<_, SqlValue>(index).map_err(error)? {
            SqlValue::Text(v) => json!(v),
            SqlValue::Integer(v) => json!(v),
            SqlValue::Real(v) => json!(v),
            SqlValue::Null => Value::Null,
            SqlValue::Blob(_) => return Err(invalid("Unexpected binary mod metadata.")),
        };
        result.insert(name, value);
    }
    if !matches!(
        result.get("kind").and_then(Value::as_str),
        Some("owned" | "external")
    ) {
        return Err(invalid(missing));
    }
    if field == Some("priority") {
        result.insert(
            "entries".into(),
            json!(organization::entries(connection, profile)?),
        );
    }
    Ok(Value::Object(result))
}

#[pyfunction]
#[pyo3(signature = (db,profile,id,field=None))]
fn metadata_state(db: &Database, profile: &str, id: &str, field: Option<&str>) -> PyResult<String> {
    Ok(state(&*db.connection()?, profile, id, field)?.to_string())
}

#[pyfunction]
#[pyo3(signature = (db,profile,id,cancel=None))]
fn mod_name_review(
    db: &Database,
    profile: &str,
    id: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<(String, String)> {
    let value = state(&*db.connection()?, profile, id, None)?;
    check_cancel(cancel)?;
    Ok((
        value["name"].as_str().unwrap().into(),
        encoding::digest(&value, false, false),
    ))
}

#[pyfunction]
#[pyo3(signature = (db,profile,id,name,expected,runtime_active,cancel=None,checkpoint=None))]
fn mod_name_apply(
    db: &Database,
    profile: &str,
    id: &str,
    name: &str,
    expected: &str,
    runtime_active: bool,
    cancel: Option<&Bound<'_, PyAny>>,
    checkpoint: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let value = encoding::parse(name)?;
    let name = value
        .as_str()
        .filter(|s| {
            !s.trim().is_empty() && s.trim().chars().count() <= 200 && !s.chars().any(|c| c < ' ')
        })
        .ok_or_else(|| {
            invalid("Give the display title 1-200 characters without control characters.")
        })?
        .trim();
    if runtime_active {
        return Err(invalid(
            "Close the recorded runtime before editing mod metadata.",
        ));
    }
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    if encoding::digest(&state(&connection, profile, id, None)?, false, false) != expected {
        return Err(invalid(
            "Selected mod metadata or profile membership changed. Review its title again.",
        ));
    }
    check_cancel(cancel)?;
    connection
        .execute("UPDATE mods SET name=? WHERE id=?", params![name, id])
        .map_err(error)?;
    drop(connection);
    if let Some(checkpoint) = checkpoint {
        checkpoint.call0()?;
    }
    check_cancel(cancel)
}

pub fn field_value(field: &str, value: Value) -> PyResult<Value> {
    let value = value
        .as_str()
        .ok_or_else(|| invalid("Enter a text value for this field."))?;
    if matches!(field, "priority" | "nexus") {
        let text = value.trim();
        if text.is_empty() || text.len() > 10 || !text.bytes().all(|c| c.is_ascii_digit()) {
            return Err(invalid("Enter a nonnegative whole number."));
        }
        let value = text
            .parse::<u64>()
            .map_err(|_| invalid("Enter a nonnegative whole number."))?;
        if value > 2_147_483_647 {
            return Err(invalid("Value exceeds the supported integer range."));
        }
        return Ok(json!(value));
    }
    if field == "category" {
        return Ok(json!(categories::normalized(json!(
            value
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        ))?));
    }
    if value.chars().count() > if field == "notes" { 16384 } else { 200 }
        || value
            .chars()
            .any(|c| c < ' ' && (field != "notes" || !matches!(c, '\n' | '\r' | '\t')))
    {
        return Err(invalid(
            "Notes allow 16,384 characters; versions allow 200 without control characters.",
        ));
    }
    Ok(json!(if field == "notes" {
        value
    } else {
        value.trim()
    }))
}

#[pyfunction]
fn metadata_value(field: &str, value: &str) -> PyResult<String> {
    Ok(field_value(field, encoding::parse(value)?)?.to_string())
}

#[pyfunction]
#[pyo3(signature = (db,profile,id,field,cancel=None))]
fn metadata_review(
    db: &Database,
    profile: &str,
    id: &str,
    field: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<(String, String)> {
    if !FIELDS.contains(&field) {
        return Err(invalid("This field is read-only."));
    }
    let state = state(&*db.connection()?, profile, id, Some(field))?;
    let value = if field == "category" {
        categories::memberships(
            state["category"].as_str().unwrap(),
            state["categories_json"].as_str().unwrap(),
        )?
        .join("\n")
    } else if field == "nexus" {
        state["nexus_mod_id"].to_string()
    } else if let Some(value) = state[field].as_str() {
        value.into()
    } else {
        state[field].to_string()
    };
    check_cancel(cancel)?;
    Ok((value, encoding::digest(&state, false, false)))
}

#[pyfunction]
#[pyo3(signature = (db,profile,id,field,value,expected,library_id,reviewed_library_id,cancel=None,checkpoint=None))]
fn metadata_apply(
    db: &Database,
    profile: &str,
    id: &str,
    field: &str,
    value: &str,
    expected: &str,
    library_id: &str,
    reviewed_library_id: &str,
    cancel: Option<&Bound<'_, PyAny>>,
    checkpoint: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    if library_id != reviewed_library_id || !FIELDS.contains(&field) {
        return Err(invalid("Library or editable field changed. Review again."));
    }
    let value = field_value(field, encoding::parse(value)?)?;
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    let state = state(&connection, profile, id, Some(field))?;
    if encoding::digest(&state, false, false) != expected {
        return Err(invalid(
            "Mod metadata or profile entries changed. Review this field again.",
        ));
    }
    check_cancel(cancel)?;
    match field {
        "priority" => {
            let before = state["entries"].as_array().unwrap();
            let mut order: Vec<_> = before
                .iter()
                .filter_map(|r| r["mod_id"].as_str())
                .filter(|m| *m != id)
                .collect();
            let priority = value.as_u64().unwrap() as usize;
            if priority > order.len() {
                return Err(invalid(format!(
                    "Priority must be between 0 and {}.",
                    order.len()
                )));
            }
            order.insert(priority, id);
            for (index, id) in order.into_iter().enumerate() {
                connection
                    .execute(
                        "UPDATE entries SET priority=? WHERE profile_id=? AND mod_id=?",
                        params![index as i64, profile, id],
                    )
                    .map_err(error)?;
            }
            organization::history(&connection, profile, before, "field-priority")?;
        }
        "category" => {
            let names = connection
                .prepare("SELECT name FROM categories")
                .map_err(error)?
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(error)?;
            let names: BTreeMap<_, _> = names
                .into_iter()
                .map(|v| (categories::fold(&v), v))
                .collect();
            let values: Vec<_> = value
                .as_array()
                .unwrap()
                .iter()
                .map(|v| {
                    let v = v.as_str().unwrap();
                    names
                        .get(&categories::fold(v))
                        .cloned()
                        .unwrap_or_else(|| v.into())
                })
                .collect();
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
        "nexus" => {
            let value = value.as_i64().unwrap();
            if value != 0 && state["nexus_game"].as_str().unwrap().is_empty() {
                return Err(invalid(
                    "Set this mod's Nexus game context before assigning a mod ID.",
                ));
            }
            let file = if Some(value) == state["nexus_mod_id"].as_i64() {
                state["nexus_file_id"].as_i64().unwrap()
            } else {
                0
            };
            connection
                .execute(
                    "UPDATE mods SET nexus_mod_id=?,nexus_file_id=? WHERE id=?",
                    params![value, file, id],
                )
                .map_err(error)?;
        }
        _ => {
            connection
                .execute(
                    &format!("UPDATE mods SET {field}=? WHERE id=?"),
                    params![value.as_str().unwrap(), id],
                )
                .map_err(error)?;
        }
    }
    drop(connection);
    if let Some(checkpoint) = checkpoint {
        checkpoint.call0()?;
    }
    check_cancel(cancel)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(metadata_state, m)?)?;
    m.add_function(wrap_pyfunction!(mod_name_review, m)?)?;
    m.add_function(wrap_pyfunction!(mod_name_apply, m)?)?;
    m.add_function(wrap_pyfunction!(metadata_value, m)?)?;
    m.add_function(wrap_pyfunction!(metadata_review, m)?)?;
    m.add_function(wrap_pyfunction!(metadata_apply, m)?)?;
    Ok(())
}
