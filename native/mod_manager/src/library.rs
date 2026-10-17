use crate::{
    categories, check_cancel,
    database::{Database, error, records},
    encoding, invalid, organization, relative_path,
};
use pyo3::prelude::*;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn decode(row: &mut Value, key: &str) -> PyResult<()> {
    row[key] = encoding::parse(
        row[key]
            .as_str()
            .ok_or_else(|| invalid("Invalid encoded library metadata."))?,
    )?;
    Ok(())
}

pub fn overlays(connection: &Connection) -> PyResult<Vec<Value>> {
    if !connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='reference_overlays')",[],|r|r.get::<_,bool>(0)).map_err(error)?{return Ok(vec![]);}
    records(
        connection,
        "SELECT * FROM reference_overlays ORDER BY overlay_id",
        [],
    )
}

pub fn effective(rows: &[Value], links: &[Value], profile: &str, target: &str) -> Vec<bool> {
    let providers: BTreeMap<_, _> = rows
        .iter()
        .map(|row| (row["id"].as_str().unwrap_or(""), row))
        .collect();
    rows.iter()
        .map(|row| {
            let enabled = row["enabled"].as_bool().unwrap_or(false);
            let Some(link) = links.iter().find(|link| link["overlay_id"] == row["id"]) else {
                return enabled;
            };
            let base = providers.get(link["base_id"].as_str().unwrap_or(""));
            enabled
                && link["profile_id"] == profile
                && link["target_id"] == target
                && base.is_some_and(|base| {
                    matches!(base["kind"].as_str(), Some("external" | "reference"))
                        && base["source"] == link["source"]
                        && base["enabled"].as_bool() == Some(true)
                })
        })
        .collect()
}

pub fn raw_mod_rows(connection: &Connection, profile: &str) -> PyResult<Vec<Value>> {
    let mut rows = records(
        connection,
        "SELECT m.id,m.name,m.source,m.kind,m.version,m.category,m.notes,m.tags,e.priority,e.enabled,e.group_name,m.nexus_game,m.nexus_mod_id,m.nexus_file_id,m.categories_json FROM mods m JOIN entries e ON e.mod_id=m.id WHERE e.profile_id=? AND m.kind!='foreign' ORDER BY e.priority,m.id",
        [profile],
    )?;
    for row in &mut rows {
        let memberships = categories::memberships(
            row["category"].as_str().unwrap(),
            row["categories_json"].as_str().unwrap(),
        )?;
        let object = row.as_object_mut().unwrap();
        object.remove("categories_json");
        let group = object.remove("group_name").unwrap();
        object.insert("group".into(), group);
        object.insert("categories".into(), json!(memberships));
        let enabled = object["enabled"].as_i64() != Some(0);
        object.insert("enabled".into(), json!(enabled));
    }
    Ok(rows)
}

pub fn mod_rows(connection: &Connection, profile: &str) -> PyResult<Vec<Value>> {
    let mut rows = raw_mod_rows(connection, profile)?;
    let enabled = effective(&rows, &overlays(connection)?, profile, "");
    for (row, enabled) in rows.iter_mut().zip(enabled) {
        row["enabled"] = json!(enabled);
    }
    Ok(rows)
}

pub fn signature(connection: &Connection, profile: &str) -> PyResult<String> {
    let files = connection
        .query_row("SELECT files FROM profiles WHERE id=?", [profile], |r| {
            r.get::<_, String>(0)
        })
        .optional()
        .map_err(error)?
        .ok_or_else(|| invalid("Select a local profile first."))?;
    let files = encoding::parse(&files)?;
    let mut statement=connection.prepare("SELECT m.id,m.source,e.priority,e.enabled,m.kind FROM mods m JOIN entries e ON e.mod_id=m.id WHERE e.profile_id=? AND m.kind!='foreign' ORDER BY e.priority,m.id").map_err(error)?;
    let rows = statement
        .query_map([profile], |r| {
            Ok(json!([
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?
            ]))
        })
        .map_err(error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(error)?;
    let instance = connection
        .query_row(
            "SELECT configuration FROM instances WHERE id=?",
            [files
                .get("_modbox_instance_id")
                .and_then(Value::as_str)
                .unwrap_or("")],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(error)?;
    let configuration = instance
        .map(|v| encoding::parse(&v))
        .transpose()?
        .unwrap_or(json!({}));
    Ok(encoding::digest(
        &json!([files, rows, configuration, overlays(connection)?]),
        true,
        true,
    ))
}

#[pyfunction]
fn library_signature(db: &Database, profile: &str) -> PyResult<String> {
    signature(&*db.connection()?, profile)
}
#[pyfunction]
fn overlay_records(db: &Database) -> PyResult<String> {
    Ok(json!(overlays(&*db.connection()?)?).to_string())
}
#[pyfunction]
fn overlay_effective(rows: &str, links: &str, profile: &str, target: &str) -> PyResult<Vec<bool>> {
    let rows = encoding::parse(rows)?;
    let links = encoding::parse(links)?;
    Ok(effective(
        rows.as_array()
            .ok_or_else(|| invalid("Invalid mod rows."))?,
        links
            .as_array()
            .ok_or_else(|| invalid("Invalid overlay records."))?,
        profile,
        target,
    ))
}

#[pyfunction]
fn library_records(db: &Database, kind: &str) -> PyResult<String> {
    let (sql, encoded) = match kind {
        "profiles" => (
            "SELECT * FROM profiles ORDER BY name COLLATE NOCASE,id",
            Some("files"),
        ),
        "migrations" => (
            "SELECT * FROM mo2_migrations ORDER BY updated DESC,id",
            Some("report"),
        ),
        "deleted_profiles" => (
            "SELECT id,name,removed FROM deleted_profiles ORDER BY removed DESC,id",
            None,
        ),
        "configurations" => ("SELECT * FROM instances", Some("configuration")),
        "origins" => ("SELECT * FROM origins", None),
        "downloads" => ("SELECT * FROM downloads ORDER BY game,mod_id,file_id", None),
        _ => return Err(invalid("Unknown library record collection.")),
    };
    let mut rows = records(&*db.connection()?, sql, [])?;
    if let Some(key) = encoded {
        for row in &mut rows {
            decode(row, key)?;
        }
    }
    Ok(json!(rows).to_string())
}

#[pyfunction]
fn library_rows(db: &Database, profile: &str) -> PyResult<String> {
    Ok(json!(mod_rows(&*db.connection()?, profile)?).to_string())
}

#[pyfunction]
fn library_unmanaged_names(db: &Database, profile: &str) -> PyResult<Vec<String>> {
    let connection = db.connection()?;
    let files = connection
        .query_row("SELECT files FROM profiles WHERE id=?", [profile], |r| {
            r.get::<_, String>(0)
        })
        .optional()
        .map_err(error)?;
    let files = files
        .map(|v| encoding::parse(&v))
        .transpose()?
        .unwrap_or(json!({}));
    let names = encoding::parse(
        files
            .get("_modbox_game_content")
            .and_then(Value::as_str)
            .unwrap_or("[]"),
    )?;
    let mut names: BTreeSet<String> = names
        .as_array()
        .ok_or_else(|| invalid("Invalid unmanaged game content."))?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let mut statement=connection.prepare("SELECT m.name FROM mods m JOIN entries e ON e.mod_id=m.id WHERE e.profile_id=? AND m.kind='foreign'").map_err(error)?;
    for name in statement
        .query_map([profile], |r| r.get::<_, String>(0))
        .map_err(error)?
    {
        names.insert(name.map_err(error)?);
    }
    Ok(names.into_iter().collect())
}

#[pyfunction]
fn library_profile_snapshot(
    db: &Database,
    profile: &str,
    include_signature: bool,
) -> PyResult<String> {
    let connection = db.connection()?;
    connection.execute_batch("BEGIN").map_err(error)?;
    let mut record = records(&connection, "SELECT * FROM profiles WHERE id=?", [profile])?
        .into_iter()
        .next()
        .ok_or_else(|| invalid("Select an existing local profile first."))?;
    decode(&mut record, "files")?;
    let configuration = connection
        .query_row(
            "SELECT configuration FROM instances WHERE id=?",
            [record["files"]
                .get("_modbox_instance_id")
                .and_then(Value::as_str)
                .unwrap_or("")],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(error)?;
    let configuration = configuration
        .map(|v| encoding::parse(&v))
        .transpose()?
        .unwrap_or(json!({}));
    let mut result = vec![
        record,
        json!(mod_rows(&connection, profile)?),
        configuration,
    ];
    if include_signature {
        result.push(json!(signature(&connection, profile)?));
    }
    Ok(json!(result).to_string())
}

#[pyfunction]
#[pyo3(signature = (db,name,clone,template,cancel=None,checkpoint=None))]
fn library_create_profile(
    db: &Database,
    name: &str,
    clone: &str,
    template: &str,
    cancel: Option<&Bound<'_, PyAny>>,
    checkpoint: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(invalid("Profile name cannot be empty."));
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    if !clone.is_empty() {
        let source = connection
            .query_row("SELECT files FROM profiles WHERE id=?", [clone], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .map_err(error)?
            .ok_or_else(|| invalid("Profile to clone no longer exists."))?;
        connection
            .execute(
                "INSERT INTO profiles VALUES (?,?,?,?)",
                params![id, name, "", source],
            )
            .map_err(error)?;
        connection.execute("INSERT INTO entries SELECT ?,mod_id,priority,enabled,group_name FROM entries WHERE profile_id=?",params![id,clone]).map_err(error)?;
        if connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='launch_target_profiles')",
                [],
                |r| r.get::<_, bool>(0),
            )
            .map_err(error)?
        {
            connection.execute("INSERT INTO launch_target_profiles SELECT target_id,?,files FROM launch_target_profiles WHERE profile_id=?",params![id,clone]).map_err(error)?;
        }
    } else {
        let source = if template.is_empty() {
            None
        } else {
            connection
                .query_row("SELECT files FROM profiles WHERE id=?", [template], |r| {
                    r.get::<_, String>(0)
                })
                .optional()
                .map_err(error)?
        };
        let mut files = source
            .map(|v| encoding::parse(&v))
            .transpose()?
            .unwrap_or(json!({}));
        let object = files
            .as_object_mut()
            .ok_or_else(|| invalid("Invalid profile files."))?;
        for (key, value) in object.iter_mut() {
            if ["plugins.txt", "loadorder.txt", "lockedorder.txt"]
                .contains(&categories::fold(key).as_str())
            {
                *value = json!("");
            }
        }
        object.remove("_modbox_plugin_validation");
        object.remove("_modbox_plugin_backup");
        connection
            .execute(
                "INSERT INTO profiles(id,name,files) VALUES (?,?,?)",
                params![id, name, files.to_string()],
            )
            .map_err(error)?;
        connection.execute("INSERT INTO entries SELECT ?,id,ROW_NUMBER() OVER (ORDER BY name)-1,0,'' FROM mods WHERE kind IN ('owned','external')",[&id]).map_err(error)?;
    }
    drop(connection);
    if let Some(checkpoint) = checkpoint {
        checkpoint.call0()?;
    }
    check_cancel(cancel)?;
    Ok(id)
}

#[pyfunction]
#[pyo3(signature = (db,profile,cancel=None))]
fn library_delete_profile(
    db: &Database,
    profile: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    let record = records(&connection, "SELECT * FROM profiles WHERE id=?", [profile])?
        .into_iter()
        .next()
        .ok_or_else(|| invalid("Profile no longer exists in this library."))?;
    let entries = records(
        &connection,
        "SELECT * FROM entries WHERE profile_id=? ORDER BY priority,mod_id",
        [profile],
    )?;
    check_cancel(cancel)?;
    connection
        .execute(
            "INSERT INTO deleted_profiles(id,name,snapshot) VALUES (?,?,?)",
            params![
                profile,
                record["name"].as_str(),
                json!({"profile":record,"entries":entries}).to_string()
            ],
        )
        .map_err(error)?;
    connection
        .execute("DELETE FROM entries WHERE profile_id=?", [profile])
        .map_err(error)?;
    connection
        .execute("DELETE FROM profiles WHERE id=?", [profile])
        .map_err(error)?;
    check_cancel(cancel)?;
    Ok(profile.into())
}

#[pyfunction]
#[pyo3(signature = (db,profile,cancel=None))]
fn library_restore_profile(
    db: &Database,
    profile: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    let snapshot = connection
        .query_row(
            "SELECT snapshot FROM deleted_profiles WHERE id=?",
            [profile],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(error)?
        .ok_or_else(|| invalid("Removed profile is no longer available to restore."))?;
    let snapshot = encoding::parse(&snapshot)?;
    let entries = snapshot["entries"]
        .as_array()
        .ok_or_else(|| invalid("Invalid removed profile snapshot."))?;
    for row in entries {
        if !connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM mods WHERE id=?)",
                [row["mod_id"].as_str()],
                |r| r.get::<_, bool>(0),
            )
            .map_err(error)?
        {
            return Err(invalid(
                "Restore removed mod content from Recovery before restoring this profile. Its saved organization remains retained.",
            ));
        }
    }
    let saved = &snapshot["profile"];
    connection
        .execute(
            "INSERT INTO profiles VALUES (?,?,?,?)",
            params![
                saved["id"].as_str(),
                saved["name"].as_str(),
                saved["source"].as_str(),
                saved["files"].as_str()
            ],
        )
        .map_err(error)?;
    for row in entries {
        connection
            .execute(
                "INSERT INTO entries VALUES (?,?,?,?,?)",
                params![
                    row["profile_id"].as_str(),
                    row["mod_id"].as_str(),
                    row["priority"].as_i64(),
                    row["enabled"].as_i64(),
                    row["group_name"].as_str()
                ],
            )
            .map_err(error)?;
    }
    check_cancel(cancel)?;
    connection
        .execute("DELETE FROM deleted_profiles WHERE id=?", [profile])
        .map_err(error)?;
    Ok(profile.into())
}

#[pyfunction]
#[pyo3(signature = (db,profile,id,enabled=None,group=None,tags=None,notes=None))]
fn library_customize(
    db: &Database,
    profile: &str,
    id: &str,
    enabled: Option<bool>,
    group: Option<&str>,
    tags: Option<&str>,
    notes: Option<&str>,
) -> PyResult<()> {
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    let before = organization::entries(&connection, profile)?;
    if let Some(enabled) = enabled {
        connection.execute("UPDATE entries SET enabled=? WHERE profile_id=? AND mod_id=? AND mod_id IN (SELECT id FROM mods WHERE kind!='separator')",params![enabled,profile,id]).map_err(error)?;
    }
    if let Some(group) = group {
        connection
            .execute(
                "UPDATE entries SET group_name=? WHERE profile_id=? AND mod_id=?",
                params![relative_path(group.trim(), true)?, profile, id],
            )
            .map_err(error)?;
    }
    if let Some(tags) = tags {
        connection
            .execute(
                "UPDATE mods SET tags=? WHERE id=?",
                params![tags.trim(), id],
            )
            .map_err(error)?;
    }
    if let Some(notes) = notes {
        connection
            .execute("UPDATE mods SET notes=? WHERE id=?", params![notes, id])
            .map_err(error)?;
    }
    organization::history(&connection, profile, &before, "organization")
}

#[pyfunction]
fn library_reorder(db: &Database, profile: &str, id: &str, priority: i64) -> PyResult<()> {
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    let rows = organization::entries(&connection, profile)?;
    if !rows.iter().any(|r| r["mod_id"] == id) {
        return Err(invalid("Mod no longer belongs to this profile."));
    }
    let mut order: Vec<_> = rows
        .iter()
        .filter_map(|r| r["mod_id"].as_str())
        .filter(|v| *v != id)
        .collect();
    order.insert(priority.clamp(0, order.len() as i64) as usize, id);
    for (index, id) in order.into_iter().enumerate() {
        connection
            .execute(
                "UPDATE entries SET priority=? WHERE profile_id=? AND mod_id=?",
                params![index as i64, profile, id],
            )
            .map_err(error)?;
    }
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(library_signature, m)?)?;
    m.add_function(wrap_pyfunction!(overlay_records, m)?)?;
    m.add_function(wrap_pyfunction!(overlay_effective, m)?)?;
    m.add_function(wrap_pyfunction!(library_records, m)?)?;
    m.add_function(wrap_pyfunction!(library_rows, m)?)?;
    m.add_function(wrap_pyfunction!(library_unmanaged_names, m)?)?;
    m.add_function(wrap_pyfunction!(library_profile_snapshot, m)?)?;
    m.add_function(wrap_pyfunction!(library_create_profile, m)?)?;
    m.add_function(wrap_pyfunction!(library_delete_profile, m)?)?;
    m.add_function(wrap_pyfunction!(library_restore_profile, m)?)?;
    m.add_function(wrap_pyfunction!(library_customize, m)?)?;
    m.add_function(wrap_pyfunction!(library_reorder, m)?)?;
    Ok(())
}
