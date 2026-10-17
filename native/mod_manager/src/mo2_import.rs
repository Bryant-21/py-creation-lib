use crate::{
    categories::fold,
    check_cancel,
    database::{Database, error},
    encoding,
    executables::{identity, import_into, text},
    invalid,
};
use pyo3::prelude::*;
use rusqlite::params;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn resolved(path: &Path) -> std::io::Result<PathBuf> {
    if path.exists() {
        return std::fs::canonicalize(path);
    }
    let absolute = std::path::absolute(path)?;
    match (absolute.parent(), absolute.file_name()) {
        (Some(parent), Some(name)) => Ok(resolved(parent)?.join(name)),
        _ => Ok(absolute),
    }
}
fn within(child: &Path, parent: &Path) -> bool {
    let child = fold(&child.to_string_lossy()).replace('\\', "/");
    let parent = fold(&parent.to_string_lossy()).replace('\\', "/");
    child == parent || child.starts_with(&format!("{}/", parent.trim_end_matches('/')))
}
fn array<'a>(value: &'a Value, key: &str) -> PyResult<&'a Vec<Value>> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(format!("Invalid MO2 {key} metadata.")))
}
fn count(value: usize) -> String {
    let digits = value.to_string();
    let mut result = String::new();
    for (index, c) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result
}

#[pyfunction]
#[pyo3(signature = (db,root,preview,selected=None,reconcile=false,cancel=None,progress=None,checkpoint=None))]
fn library_import_mo2(
    db: &Database,
    root: PathBuf,
    preview: &str,
    selected: Option<Vec<String>>,
    reconcile: bool,
    cancel: Option<&Bound<'_, PyAny>>,
    progress: Option<&Bound<'_, PyAny>>,
    checkpoint: Option<&Bound<'_, PyAny>>,
) -> PyResult<(usize, usize)> {
    let preview = encoding::parse(preview)?;
    let profiles = array(&preview, "profiles")?;
    let selected: BTreeSet<_> = selected
        .unwrap_or_else(|| profiles.iter().map(|p| text(p, "id").to_string()).collect())
        .into_iter()
        .collect();
    if selected.is_empty() {
        return Err(invalid("Select at least one profile to import."));
    }
    let empty = json!({});
    let configuration = preview.get("configuration").unwrap_or(&empty);
    let approved = configuration
        .get("source_mappings")
        .and_then(Value::as_object);
    let root = resolved(&root)?;
    let sources = [
        text(&preview, "root"),
        text(&preview, "mods_root"),
        text(&preview, "profiles_root"),
    ]
    .into_iter()
    .chain(
        approved
            .into_iter()
            .flat_map(|m| m.values().filter_map(Value::as_str)),
    );
    for source in sources {
        let path = resolved(Path::new(source))?;
        if within(&root, &path) || within(&path, &root) {
            return Err(invalid("Library and MO2 source folders must be separate."));
        }
    }
    let approved_aliases: BTreeSet<_> = approved
        .into_iter()
        .flat_map(|m| m.keys().map(|s| fold(s)))
        .collect();
    let total = profiles
        .iter()
        .filter(|p| selected.contains(text(p, "id")))
        .map(|p| p["mods"].as_array().map_or(0, Vec::len) + 1)
        .sum::<usize>()
        .max(1);
    let (mut imported, mut repaired, mut completed) = (0, 0, 0);
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    let instance = identity(text(&preview, "root"));
    connection
        .execute(
            "INSERT OR IGNORE INTO instances VALUES (?,?,?)",
            params![instance, text(&preview, "root"), configuration.to_string()],
        )
        .map_err(error)?;
    if reconcile {
        connection
            .execute(
                "UPDATE instances SET configuration=? WHERE id=?",
                params![configuration.to_string(), instance],
            )
            .map_err(error)?;
    }
    if let Some(approved) = approved.filter(|m| !m.is_empty()) {
        let mut config = encoding::parse(
            &connection
                .query_row(
                    "SELECT configuration FROM instances WHERE id=?",
                    [&instance],
                    |r| r.get::<_, String>(0),
                )
                .map_err(error)?,
        )?;
        if config.get("source_mappings").is_none() {
            config["source_mappings"] = json!({});
        }
        config["source_mappings"]
            .as_object_mut()
            .ok_or_else(|| invalid("Invalid source mappings."))?
            .extend(approved.clone());
        connection
            .execute(
                "UPDATE instances SET configuration=? WHERE id=?",
                params![config.to_string(), instance],
            )
            .map_err(error)?;
    }
    import_into(&connection, configuration, &instance)?;
    for profile in profiles {
        check_cancel(cancel)?;
        let id = text(profile, "id");
        if !selected.contains(id) {
            continue;
        }
        if connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM deleted_profiles WHERE id=?)",
                [id],
                |r| r.get::<_, bool>(0),
            )
            .map_err(error)?
        {
            continue;
        }
        let mods = array(profile, "mods")?;
        let managed: Vec<_> = mods
            .iter()
            .filter(|m| text(m, "kind") != "foreign")
            .collect();
        for item in &managed {
            check_cancel(cancel)?;
            if text(item, "kind") == "external" && !text(item, "source").is_empty() {
                let alias = std::path::absolute(
                    Path::new(text(&preview, "mods_root")).join(text(item, "name")),
                )?;
                repaired += if approved_aliases.contains(&fold(&alias.to_string_lossy())) {
                    connection
                        .execute(
                            "UPDATE mods SET source=? WHERE id=? AND kind='external' AND source<>?",
                            params![text(item, "source"), text(item, "id"), text(item, "source")],
                        )
                        .map_err(error)?
                } else {
                    connection
                        .execute(
                            "UPDATE mods SET source=? WHERE id=? AND kind='external' AND source=''",
                            params![text(item, "source"), text(item, "id")],
                        )
                        .map_err(error)?
                };
            }
        }
        let existing = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM profiles WHERE id=?)",
                [id],
                |r| r.get::<_, bool>(0),
            )
            .map_err(error)?;
        if existing && !reconcile {
            continue;
        }
        if !existing {
            let mut files = profile["files"].clone();
            if !files.is_object() {
                return Err(invalid("Invalid MO2 profile files."));
            }
            files["_modbox_instance_id"] = json!(instance);
            files["_modbox_game"] = preview["game"].clone();
            files["_modbox_game_content"] = json!(encoding::encode(
                &json!(
                    mods.iter()
                        .filter(|m| text(m, "kind") == "foreign")
                        .map(|m| text(m, "name"))
                        .collect::<Vec<_>>()
                ),
                true,
                true
            ));
            connection
                .execute(
                    "INSERT INTO profiles VALUES (?,?,?,?)",
                    params![
                        id,
                        text(profile, "name"),
                        text(profile, "source"),
                        files.to_string()
                    ],
                )
                .map_err(error)?;
            imported += 1;
        }
        completed += 1;
        for (position, item) in managed.iter().enumerate() {
            check_cancel(cancel)?;
            let mut priority = position as i64;
            let categories = item
                .get("categories")
                .and_then(Value::as_array)
                .filter(|v| !v.is_empty())
                .cloned()
                .unwrap_or_else(|| {
                    if text(item, "category").is_empty() {
                        vec![]
                    } else {
                        vec![item["category"].clone()]
                    }
                });
            connection.execute("INSERT OR IGNORE INTO mods (id,name,source,kind,version,category,notes,nexus_game,nexus_mod_id,nexus_file_id,tags,categories_json) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",params![text(item,"id"),text(item,"name"),text(item,"source"),text(item,"kind"),text(item,"version"),text(item,"category"),text(item,"notes"),text(item,"nexus_game"),item["nexus_mod_id"].as_i64().unwrap_or(0),item["nexus_file_id"].as_i64().unwrap_or(0),text(item,"tags"),encoding::encode(&json!(categories),true,true)]).map_err(error)?;
            if existing {
                let skip=connection.query_row("SELECT EXISTS(SELECT 1 FROM reference_exclusions WHERE profile_id=?1 AND mod_id=?2) OR EXISTS(SELECT 1 FROM entries WHERE profile_id=?1 AND mod_id=?2)",params![id,text(item,"id")],|r|r.get::<_,bool>(0)).map_err(error)?;
                if skip {
                    continue;
                }
                priority = connection
                    .query_row(
                        "SELECT COALESCE(MAX(priority),-1)+1 FROM entries WHERE profile_id=?",
                        [id],
                        |r| r.get(0),
                    )
                    .map_err(error)?;
            }
            connection
                .execute(
                    "INSERT INTO entries VALUES (?,?,?,?,?)",
                    params![
                        id,
                        text(item, "id"),
                        priority,
                        item["enabled"].as_bool().unwrap_or(false),
                        text(item, "group")
                    ],
                )
                .map_err(error)?;
            completed += 1;
            if let Some(progress) = progress {
                if completed % 128 == 0 || priority == mods.len() as i64 - 1 {
                    progress.call1((
                        completed as f64 / total as f64,
                        format!(
                            "{}: {} / {} entries",
                            text(profile, "name"),
                            count(priority as usize + 1),
                            count(mods.len())
                        ),
                    ))?;
                }
            }
        }
    }
    drop(connection);
    check_cancel(cancel)?;
    if let Some(checkpoint) = checkpoint {
        checkpoint.call1(("before-profile-commit",))?;
    }
    check_cancel(cancel)?;
    Ok((imported, repaired))
}

#[pyfunction]
fn library_save_migration(db: &Database, source: &str, state: &str, report: &str) -> PyResult<()> {
    let report = encoding::parse(report)?;
    db.connection()?.execute("INSERT INTO mo2_migrations(id,source,state,report) VALUES (?,?,?,?) ON CONFLICT(id) DO UPDATE SET state=excluded.state,report=excluded.report,updated=CURRENT_TIMESTAMP",params![identity(source),source,state,report.to_string()]).map_err(error)?;
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(library_import_mo2, m)?)?;
    m.add_function(wrap_pyfunction!(library_save_migration, m)?)?;
    Ok(())
}
