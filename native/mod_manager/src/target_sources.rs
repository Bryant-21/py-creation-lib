use crate::{
    check_cancel,
    database::{Database, error, records},
    encoding,
    executables::text,
    invalid,
    reference_sources::{directory, inventory},
    target_settings::targets,
};
use pyo3::prelude::*;
use rusqlite::params;
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};

pub fn sources(db: &Database, target: &str) -> PyResult<Value> {
    if records(
        &*db.connection()?,
        "SELECT 1 FROM sqlite_master WHERE name='launch_target_mod_sources'",
        [],
    )?
    .is_empty()
    {
        return Ok(json!({}));
    }
    let mut result = json!({});
    for row in records(
        &*db.connection()?,
        "SELECT mod_id,record FROM launch_target_mod_sources WHERE target_id=? ORDER BY mod_id LIMIT 50001",
        [target],
    )? {
        let value = encoding::parse(text(&row, "record"))?;
        let keys = value
            .as_object()
            .map(|v| v.keys().map(String::as_str).collect::<BTreeSet<_>>())
            .unwrap_or_default();
        let basic: BTreeSet<_> = ["source", "version", "enabled"].into_iter().collect();
        let extended: BTreeSet<_> = [
            "source",
            "version",
            "enabled",
            "nexus_file_id",
            "nexus_game",
            "nexus_mod_id",
        ]
        .into_iter()
        .collect();
        if (keys != basic && keys != extended)
            || !value["source"].is_string()
            || (!text(&value, "source").is_empty()
                && !Path::new(text(&value, "source")).is_absolute())
            || !value["version"].is_string()
            || text(&value, "version").chars().count() > 200
            || !(value["enabled"].is_null() || value["enabled"].is_boolean())
        {
            return Err(invalid("Target mod source metadata is invalid."));
        }
        if value.get("nexus_file_id").is_some() {
            let file = &value["nexus_file_id"];
            if (!file.is_null() && !file.as_u64().is_some_and(|v| v < 1_000_000_000_000))
                || !value["nexus_game"].is_string()
                || !(value["nexus_mod_id"].is_i64() || value["nexus_mod_id"].is_u64())
                || (file.as_u64().unwrap_or_default() != 0
                    && (text(&value, "nexus_game").is_empty()
                        || !value["nexus_mod_id"]
                            .as_u64()
                            .is_some_and(|v| v > 0 && v < 1_000_000_000_000)))
            {
                return Err(invalid("Target Nexus file identity is invalid."));
            }
        }
        result[text(&row, "mod_id")] = value;
    }
    if result.as_object().unwrap().len() > 50000 {
        return Err(invalid(
            "Target source overrides exceed the 50,000-mod limit.",
        ));
    }
    Ok(result)
}

#[pyfunction]
fn target_source_records(db: &Database, target: &str) -> PyResult<String> {
    Ok(sources(db, target)?.to_string())
}

#[pyfunction]
pub fn target_source_effective(row: &str, value: &str) -> PyResult<String> {
    let mut row = encoding::parse(row)?;
    let value = encoding::parse(value)?;
    if value.is_null() || value.as_object().is_some_and(|v| v.is_empty()) {
        return Ok(row.to_string());
    }
    if !value["enabled"].is_null() {
        row["enabled"] = value["enabled"].clone();
    }
    let source = text(&value, "source");
    let file = &value["nexus_file_id"];
    row["nexus_file_id"] = if file.is_null() {
        if source.is_empty() {
            row["nexus_file_id"].clone()
        } else {
            json!(0)
        }
    } else if file.as_u64().unwrap_or_default() != 0
        && (value["nexus_game"] != row["nexus_game"]
            || value["nexus_mod_id"] != row["nexus_mod_id"])
    {
        json!(0)
    } else {
        file.clone()
    };
    if !source.is_empty() {
        row["source"] = source.into();
        row["kind"] = "reference".into();
        row["version"] = if text(&value, "version").is_empty() {
            json!("Target source")
        } else {
            value["version"].clone()
        };
    } else if !text(&value, "version").is_empty() {
        row["version"] = value["version"].clone();
    }
    Ok(row.to_string())
}

fn state(db: &Database, target_id: &str, profile: &str, id: &str) -> PyResult<Value> {
    let target = targets(db)?.into_iter().find(|v| v["id"] == target_id);
    let row = records(
        &*db.connection()?,
        "SELECT m.* FROM mods m JOIN entries e ON m.id=e.mod_id WHERE e.profile_id=? AND m.id=?",
        params![profile, id],
    )?
    .into_iter()
    .next();
    let message = || invalid("Choose a real mod in this profile and an active FO4 target.");
    let target = target.ok_or_else(message)?;
    let row = row
        .filter(|v| !matches!(text(v, "kind"), "separator" | "foreign"))
        .ok_or_else(message)?;
    let links: Vec<_> = records(
        &*db.connection()?,
        "SELECT * FROM entries WHERE mod_id=? ORDER BY profile_id",
        [id],
    )?
    .iter()
    .map(|v| {
        json!([
            v["profile_id"],
            v["mod_id"],
            v["priority"],
            v["enabled"],
            v["group_name"]
        ])
    })
    .collect();
    Ok(json!([target, row, links, sources(db, target_id)?[id]]))
}

fn source<'py>(
    db: &Database,
    library: &Bound<'py, PyAny>,
    raw: &str,
) -> PyResult<(Bound<'py, PyAny>, Value)> {
    let (root, identity) = directory(library, raw)?;
    let mut games: Vec<String> = targets(db)?
        .iter()
        .map(|v| text(v, "game_root").into())
        .collect();
    for row in records(
        &*db.connection()?,
        "SELECT configuration FROM instances",
        [],
    )? {
        let config = encoding::parse(text(&row, "configuration"))?;
        games.push(text(&config["game"], "gamepath").into());
    }
    let path = library.py().import("pathlib")?.getattr("Path")?;
    for game in games {
        if game.is_empty() || !Path::new(&game).is_absolute() {
            continue;
        }
        let game = path.call1((game,))?.call_method0("resolve")?;
        if root.call_method1("is_relative_to", (&game,))?.is_truthy()?
            || game.call_method1("is_relative_to", (&root,))?.is_truthy()?
        {
            return Err(invalid(
                "A target source cannot contain or reside inside a game folder.",
            ));
        }
    }
    Ok((root, identity))
}

#[pyfunction]
fn target_source_review(
    db: &Database,
    library: &Bound<'_, PyAny>,
    target: &str,
    profile: &str,
    id: &str,
    request: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let request = encoding::parse(request)?;
    let enabled = &request["enabled"];
    let version = text(&request, "version");
    if !(enabled.is_boolean() || enabled.is_null())
        || !request["version"].is_string()
        || version.chars().count() > 200
        || version.chars().any(|c| c < ' ')
    {
        return Err(invalid(
            "Use a short version label and an explicit shared/enabled/disabled choice.",
        ));
    }
    let state = state(db, target, profile, id)?;
    let reset = request["reset"].as_bool().unwrap_or(false);
    let mut record = if reset {
        json!({})
    } else {
        json!({"source":text(&request,"source").trim(),"version":version.trim(),"enabled":enabled})
    };
    let provided = request.get("nexus_file_id").is_some();
    if !reset && provided {
        let file = &request["nexus_file_id"];
        if !file.is_null() && !file.as_u64().is_some_and(|v| v < 1_000_000_000_000) {
            return Err(invalid(
                "Use a positive Nexus file ID, 0 for unknown, or leave it blank to inherit.",
            ));
        }
        let game = &state[1]["nexus_game"];
        let id = &state[1]["nexus_mod_id"];
        if file.as_u64().unwrap_or_default() != 0 {
            crate::nexus::mod_identity(game.as_str().unwrap_or_default(), id)?;
        }
        record["nexus_file_id"] = file.clone();
        record["nexus_game"] = game.clone();
        record["nexus_mod_id"] = id.clone();
    }
    let mut identity = json!([]);
    let mut signature = String::new();
    let mut count = 0;
    let mut size = 0;
    if !reset && !text(&record, "source").is_empty() {
        let (root, stamp) = source(db, library, text(&record, "source"))?;
        identity = stamp;
        record["source"] = root.str()?.to_string().into();
        (count, size, signature) = inventory(&root, Some(cancel), true)?;
        if source(db, library, text(&record, "source"))?.1 != identity {
            return Err(invalid(
                "Target source directory changed while reviewing it.",
            ));
        }
    } else if !reset && !provided && (!text(&record, "version").is_empty() || enabled.is_null()) {
        return Err(invalid(
            "Choose a variant folder, an explicit enable override, or reset to shared defaults.",
        ));
    }
    check_cancel(Some(cancel))?;
    Ok(json!({"library_id":library.getattr("library_id")?.extract::<String>()?,"target_id":target,"profile_id":profile,"mod_id":id,"name":state[1]["name"],"shared_source":state[1]["source"],"record":record,"expected":encoding::digest(&state,false,false),"identity":identity,"inventory":signature,"files":count,"bytes":size}).to_string())
}

fn tables(db: &Database) -> PyResult<()> {
    db.connection()?.execute_batch("CREATE TABLE IF NOT EXISTS launch_target_mod_sources(target_id TEXT NOT NULL,mod_id TEXT NOT NULL,record TEXT NOT NULL,PRIMARY KEY(target_id,mod_id))").map_err(error)
}
fn upsert(db: &Database, target: &str, id: &str, record: &Value) -> PyResult<()> {
    db.connection()?.execute("INSERT INTO launch_target_mod_sources VALUES (?,?,?) ON CONFLICT(target_id,mod_id) DO UPDATE SET record=excluded.record",params![target,id,record.to_string()]).map_err(error)?;
    Ok(())
}
fn inputs(
    db: &Database,
    library: &Bound<'_, PyAny>,
    plan: &Value,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<()> {
    if !text(&plan["record"], "source").is_empty() {
        let (root, identity) = source(db, library, text(&plan["record"], "source"))?;
        if identity != plan["identity"]
            || inventory(&root, Some(cancel), true)?.2 != text(plan, "inventory")
        {
            return Err(invalid("Target variant files changed after review."));
        }
    }
    Ok(())
}

#[pyfunction]
fn target_source_apply(
    db: &Database,
    library: &Bound<'_, PyAny>,
    plan: &str,
    accept: bool,
    cancel: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let plan = encoding::parse(plan)?;
    if plan["library_id"] != library.getattr("library_id")?.extract::<String>()?
        || (!text(&plan["record"], "source").is_empty() && !accept)
    {
        return Err(invalid(
            "Confirm that the selected folder is the intended version of this same mod for this target.",
        ));
    }
    library.call_method0("_runtime_write_guard")?;
    inputs(db, library, &plan, cancel)?;
    db.connection()?
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(error)?;
    let target = text(&plan, "target_id");
    let id = text(&plan, "mod_id");
    if encoding::digest(
        &state(db, target, text(&plan, "profile_id"), id)?,
        false,
        false,
    ) != text(&plan, "expected")
    {
        return Err(invalid(
            "Target/shared mod context changed after source review.",
        ));
    }
    check_cancel(Some(cancel))?;
    tables(db)?;
    if plan["record"].as_object().is_some_and(|v| !v.is_empty()) {
        upsert(db, target, id, &plan["record"])?;
    } else {
        db.connection()?
            .execute(
                "DELETE FROM launch_target_mod_sources WHERE target_id=? AND mod_id=?",
                params![target, id],
            )
            .map_err(error)?;
    }
    checkpoint.call0()?;
    check_cancel(Some(cancel))?;
    inputs(db, library, &plan, cancel)?;
    library.call_method0("_runtime_write_guard")?;
    Ok(id.into())
}

#[pyfunction]
fn target_source_set_enabled(
    db: &Database,
    library: &Bound<'_, PyAny>,
    target: &str,
    profile: &str,
    selected: &str,
    enabled: &str,
    cancel: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let enabled = encoding::parse(enabled)?
        .as_bool()
        .ok_or_else(|| invalid("Choose enabled or disabled."))?;
    let selected = encoding::parse(selected)?;
    let selected: BTreeSet<_> = selected
        .as_array()
        .ok_or_else(|| invalid("Select a bounded mod block."))?
        .iter()
        .map(|v| {
            v.as_str()
                .ok_or_else(|| invalid("Selected target/profile mods changed."))
        })
        .collect::<PyResult<_>>()?;
    if selected.is_empty() || selected.len() > 50000 {
        return Err(invalid("Select a bounded mod block."));
    }
    library.call_method0("_runtime_write_guard")?;
    db.connection()?
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(error)?;
    if !targets(db)?.iter().any(|v| v["id"] == target) {
        return Err(invalid("Select an active target."));
    }
    let allowed = records(
        &*db.connection()?,
        "SELECT m.id FROM mods m JOIN entries e ON e.mod_id=m.id WHERE e.profile_id=? AND m.kind NOT IN ('separator','foreign')",
        [profile],
    )?;
    let allowed: BTreeSet<_> = allowed.iter().map(|v| text(v, "id")).collect();
    if !selected.is_subset(&allowed) {
        return Err(invalid("Selected target/profile mods changed."));
    }
    let current = sources(db, target)?;
    tables(db)?;
    for id in selected {
        check_cancel(Some(cancel))?;
        let mut record = current
            .get(id)
            .cloned()
            .unwrap_or_else(|| json!({"source":"","version":""}));
        record["enabled"] = enabled.into();
        upsert(db, target, id, &record)?;
    }
    checkpoint.call0()?;
    check_cancel(Some(cancel))?;
    library.call_method0("_runtime_write_guard")?;
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(target_source_records, m)?)?;
    m.add_function(wrap_pyfunction!(target_source_effective, m)?)?;
    m.add_function(wrap_pyfunction!(target_source_review, m)?)?;
    m.add_function(wrap_pyfunction!(target_source_apply, m)?)?;
    m.add_function(wrap_pyfunction!(target_source_set_enabled, m)?)?;
    Ok(())
}
