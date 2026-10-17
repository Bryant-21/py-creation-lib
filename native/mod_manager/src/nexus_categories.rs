use crate::{
    categories::{fold, name},
    check_cancel,
    database::error,
    invalid,
    plugin_inventory::{object, value},
    python_io::with_database,
};
use pyo3::prelude::*;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn catalog(data: Value) -> PyResult<Vec<Value>> {
    let rows = data
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 5000)
        .ok_or_else(|| invalid("Nexus category catalog must contain 1–5,000 categories."))?;
    let mut by_id = BTreeMap::new();
    for row in rows {
        let id = row["category_id"]
            .as_i64()
            .filter(|v| *v > 0 && *v <= 1_000_000)
            .ok_or_else(|| invalid("Nexus category has an invalid ID."))?;
        let parent = if row["parent_category"] == false || row["parent_category"].is_null() {
            0
        } else {
            row["parent_category"]
                .as_i64()
                .filter(|v| *v >= 0)
                .ok_or_else(|| invalid("Invalid category parent."))?
        };
        let label = row["name"]
            .as_str()
            .filter(|v| {
                !v.trim().is_empty() && v.chars().count() <= 200 && !v.chars().any(char::is_control)
            })
            .ok_or_else(|| invalid("Invalid category name."))?;
        let segment: String = label
            .chars()
            .map(|c| if "<>:\"/\\|?*".contains(c) { ' ' } else { c })
            .collect();
        let segment = name(&json!(segment.trim().trim_end_matches('.')))?;
        if by_id
            .insert(id, (parent, label.to_owned(), segment))
            .is_some()
        {
            return Err(invalid("Nexus catalog contains duplicate category IDs."));
        }
    }
    let mut result = Vec::new();
    let mut paths = BTreeSet::new();
    for (&id, (parent, label, _)) in &by_id {
        let mut cursor = id;
        let mut seen = BTreeSet::new();
        let mut parts = Vec::new();
        while cursor != 0 {
            if !seen.insert(cursor) || seen.len() > 32 {
                return Err(invalid("Nexus category hierarchy is cyclic or too deep."));
            }
            let (parent, _, segment) = by_id
                .get(&cursor)
                .ok_or_else(|| invalid("Nexus category parent is missing."))?;
            parts.push(segment.clone());
            cursor = *parent;
        }
        parts.reverse();
        let mut path = name(&json!(parts.join("/")))?;
        if !paths.insert(fold(&path)) {
            path = name(&json!(format!("{path} [{id}]")))?;
        }
        result.push(json!({"id":id,"parent_id":parent,"name":label,"path":path}));
    }
    Ok(result)
}

#[pyfunction]
fn nexus_categories(
    library: &Bound<'_, PyAny>,
    client: &Bound<'_, PyAny>,
    game: &str,
    refresh: bool,
    merge: bool,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    crate::nexus::mod_identity(game, &json!(1))?;
    check_cancel(Some(cancel))?;
    let cached = with_database(&library.getattr("_db")?, |_, db| {
        db.connection()?
            .query_row(
                "SELECT data,fetched FROM nexus_category_catalog WHERE game=?",
                [game],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(error)
    })?;
    let fetched = py.import("time")?.call_method0("time")?.extract::<f64>()? as i64;
    let (mut rows, fetched) = if refresh {
        if client.is_none() {
            return Err(invalid(
                "Connect a Nexus account before refreshing categories.",
            ));
        }
        (
            catalog(value(&client.call_method1("categories", (game,))?)?)?,
            fetched,
        )
    } else if let Some((data, fetched)) = cached {
        (
            serde_json::from_str::<Vec<Value>>(&data).map_err(|e| invalid(e.to_string()))?,
            fetched,
        )
    } else {
        return Err(invalid(
            "No cached Nexus categories for this game. Refresh while connected first.",
        ));
    };
    check_cancel(Some(cancel))?;
    if merge {
        library.call_method0("_runtime_write_guard")?;
    }
    let imported = with_database(&library.getattr("_db")?, |_, db| {
        let db = db.connection()?;
        db.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
        if refresh {
            db.execute("INSERT INTO nexus_category_catalog VALUES (?,?,?) ON CONFLICT(game) DO UPDATE SET data=excluded.data,fetched=excluded.fetched",
                params![game, json!(rows).to_string(), fetched]).map_err(error)?;
        }
        let mut imported = 0;
        for row in &mut rows {
            check_cancel(Some(cancel))?;
            let id = row["id"].as_i64().unwrap();
            let mapped: Option<String> = db
                .query_row(
                    "SELECT local_name FROM nexus_category_mapping WHERE game=? AND category_id=?",
                    params![game, id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(error)?;
            let mut local = mapped.unwrap_or_else(|| row["path"].as_str().unwrap().to_string());
            if merge {
                let existing: Option<i64> = db
                    .query_row(
                        "SELECT nexus_id FROM categories WHERE name=?",
                        [&local],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(error)?;
                if existing.is_some_and(|v| v != 0 && v != id) {
                    local = name(&json!(format!("{local} [{game} {id}]")))?;
                }
                imported += db
                    .execute(
                        "INSERT OR IGNORE INTO categories(name,nexus_id) VALUES (?,?)",
                        params![local, id],
                    )
                    .map_err(error)?;
                db.execute(
                    "UPDATE categories SET nexus_id=? WHERE name=? AND nexus_id=0",
                    params![id, local],
                )
                .map_err(error)?;
                db.execute("INSERT INTO nexus_category_mapping VALUES (?,?,?) ON CONFLICT(game,category_id) DO UPDATE SET local_name=excluded.local_name",
                    params![game,id,local]).map_err(error)?;
            }
            row["local_name"] = json!(local);
        }
        let count: i64 = db
            .query_row("SELECT count(*) FROM categories", [], |r| r.get(0))
            .map_err(error)?;
        if count > 5000 {
            return Err(invalid("Merged category taxonomy exceeds 5,000 names."));
        }
        check_cancel(Some(cancel))?;
        Ok(imported)
    })?;
    Ok(object(
        py,
        &json!({"game":game,"fetched":fetched,"categories":rows,"imported":imported}),
    )?
    .unbind())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(nexus_categories, m)?)?;
    m.add_function(wrap_pyfunction!(agent_categories, m)?)?;
    Ok(())
}

#[pyfunction]
fn agent_categories(
    root: &Bound<'_, PyAny>,
    game: &str,
    refresh: bool,
    merge: bool,
    api_key: &str,
) -> PyResult<Py<PyAny>> {
    let py = root.py();
    let library = crate::agent_library::library(py, root)?;
    let client = if refresh {
        py.import("creation_lib.mod_manager.nexus_auth")?
            .call_method1("authenticate", (api_key, false))?
    } else {
        py.None().into_bound(py)
    };
    nexus_categories(&library, &client, game, refresh, merge, py.None().bind(py))
}
