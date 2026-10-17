use crate::{
    database::error,
    encoding,
    plugin_inventory::{object, value},
    python_io::with_database,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn clock(py: Python<'_>, name: &str) -> PyResult<f64> {
    py.import("time")?.call_method0(name)?.extract()
}
fn key(id: &Bound<'_, PyAny>) -> PyResult<String> {
    Ok(format!(
        "{}:{}",
        id.get_item(0)?.extract::<String>()?,
        id.get_item(1)?.extract::<u64>()?
    ))
}
fn identity(key: &str) -> Option<(&str, u64)> {
    let (game, id) = key.split_once(':')?;
    Some((game, id.parse().ok()?))
}
fn state(manager: &Bound<'_, PyAny>) -> PyResult<Value> {
    value(&manager.getattr("_update_schedule")?)
}
fn persist(manager: &Bound<'_, PyAny>, state: &Value) -> PyResult<()> {
    let library = manager.getattr("library")?;
    let encoded = state.to_string();
    if library.is_truthy()? {
        with_database(&library.getattr("_db")?, |_, db| {
            db.connection()?.execute("INSERT INTO nexus_update_schedule VALUES (?,?) ON CONFLICT(scope) DO UPDATE SET state=excluded.state",
                params![manager.getattr("_update_schedule_scope")?.extract::<String>()?, encoded]).map_err(error)?;
            db.connection()?.execute("INSERT INTO nexus_update_limits VALUES (?,?) ON CONFLICT(account) DO UPDATE SET blocked=excluded.blocked",
                params![manager.getattr("_update_schedule_account")?.extract::<u64>()?, state["blocked"].as_f64().unwrap_or(0.0)]).map_err(error)?;
            Ok(())
        })?;
    }
    manager.setattr("_update_schedule", object(manager.py(), state)?)
}
fn due(manager: &Bound<'_, PyAny>, state: &Value, ids: &Bound<'_, PyList>) -> PyResult<()> {
    let py = manager.py();
    let wall = clock(py, "time")?;
    let mono = clock(py, "monotonic")?;
    let interval = manager
        .getattr("update_hours")?
        .extract::<f64>()?
        .clamp(1.0, 24.0)
        * 3600.0;
    let mut next = wall + interval;
    for id in ids.iter() {
        let checked = state["records"][key(&id)?]["checked"]
            .as_f64()
            .unwrap_or(0.0);
        next = next.min(checked + interval);
    }
    let blocked = state["blocked"].as_f64().unwrap_or(0.0).min(wall + 86400.0);
    manager.setattr("_next_update", mono + (next.max(blocked) - wall).max(0.0))?;
    manager.setattr("_rate_blocked_until", mono + (blocked - wall).max(0.0))?;
    Ok(())
}
pub fn prepare<'py>(manager: &Bound<'py, PyAny>, files: bool) -> PyResult<Bound<'py, PyList>> {
    let py = manager.py();
    let library = manager.getattr("library")?;
    let client = manager.getattr("nexus_client")?;
    let user = client.getattr("user").ok().filter(|v| !v.is_none());
    let account = user
        .and_then(|v| v.get_item("user_id").ok())
        .and_then(|v| v.extract::<u64>().ok())
        .unwrap_or(0);
    let scope = json!([
        if library.is_truthy()? {
            library.getattr("library_id")?.extract::<String>()?
        } else {
            String::new()
        },
        manager.getattr("profile_id")?.extract::<String>()?,
        manager.getattr("target_id")?.extract::<String>()?,
        account,
        files
    ])
    .to_string();
    let context = (
        scope.clone(),
        manager.getattr("rows")?.as_ptr() as usize,
        manager.getattr("update_hours")?,
    )
        .into_pyobject(py)?;
    if !manager.getattr("_update_schedule_context")?.eq(&context)? {
        let rows = manager.getattr("rows")?;
        let ids = PyList::empty(py);
        let mut seen = BTreeSet::new();
        for row in rows.try_iter()? {
            let row = row?;
            let game = row.getattr("nexus_game")?;
            let id = row.getattr("nexus_mod_id")?;
            if game.is_truthy()? && id.is_truthy()? {
                let identity = (game.extract::<String>()?, id.extract::<i64>()?);
                if seen.insert(identity) {
                    ids.append((game, id))?;
                }
            }
        }
        let loaded = if library.is_truthy()? {
            with_database(&library.getattr("_db")?, |_, db| {
                db.connection()?
                    .query_row(
                        "SELECT state FROM nexus_update_schedule WHERE scope=?",
                        [&scope],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(error)
            })?
        } else if manager.getattr("_update_schedule_scope")?.eq(&scope)? {
            Some(state(manager)?.to_string())
        } else {
            None
        };
        let mut state = loaded
            .map(|v| encoding::parse(&v))
            .transpose()?
            .unwrap_or_else(|| json!({"records":{},"blocked":0}));
        if library.is_truthy()? {
            let blocked = with_database(&library.getattr("_db")?, |_, db| {
                db.connection()?
                    .query_row(
                        "SELECT blocked FROM nexus_update_limits WHERE account=?",
                        [account],
                        |r| r.get::<_, f64>(0),
                    )
                    .optional()
                    .map_err(error)
            })?
            .unwrap_or(0.0);
            state["blocked"] = json!(state["blocked"].as_f64().unwrap_or(0.0).max(blocked));
        }
        manager.setattr("_update_schedule_account", account)?;
        manager.setattr("_update_schedule_scope", scope)?;
        manager.setattr("_update_schedule_context", context)?;
        manager.setattr("_update_schedule_rows", rows)?;
        manager.setattr("_update_schedule_ids", &ids)?;
        manager.setattr("_update_schedule", object(py, &state)?)?;
        let updates = PyDict::new(py);
        let states = PyDict::new(py);
        let module = py.import("creation_lib.mod_manager.nexus_state")?;
        if let Some(records) = state["records"].as_object() {
            for (id, record) in records {
                if let Some(id) = identity(id) {
                    if let Some(version) = record["version"].as_str() {
                        updates.set_item(id, version)?;
                    }
                    if let Some(saved) = record.get("state").filter(|v| v.is_object()) {
                        let kwargs = object(py, saved)?.cast_into::<PyDict>()?;
                        let files = PyList::empty(py);
                        for file in kwargs.get_item("files")?.unwrap().try_iter()? {
                            files.append(
                                module
                                    .getattr("File")?
                                    .call((), Some(file?.cast::<PyDict>()?))?,
                            )?;
                        }
                        kwargs.set_item("files", PyTuple::new(py, files.iter())?)?;
                        let state = module.getattr("State")?.call((), Some(&kwargs))?;
                        states.set_item(id, state)?;
                    }
                }
            }
        }
        manager.setattr("updates", updates)?;
        if files {
            manager.setattr("nexus_states", states)?;
        }
        manager.setattr(
            "_status_revision",
            manager.getattr("_status_revision")?.extract::<u64>()? + 1,
        )?;
        due(manager, &state, &ids)?;
        Ok(ids)
    } else {
        Ok(manager
            .getattr("_update_schedule_ids")?
            .cast_into::<PyList>()?)
    }
}
pub fn select<'py>(
    manager: &Bound<'py, PyAny>,
    ids: &Bound<'py, PyList>,
    manual: bool,
) -> PyResult<Bound<'py, PyList>> {
    let state = state(manager)?;
    let cutoff = clock(manager.py(), "time")?
        - manager
            .getattr("update_hours")?
            .extract::<f64>()?
            .clamp(1.0, 24.0)
            * 3600.0;
    let mut ordered = ids
        .iter()
        .map(|id| {
            Ok((
                state["records"][key(&id)?]["checked"]
                    .as_f64()
                    .unwrap_or(0.0),
                id,
            ))
        })
        .collect::<PyResult<Vec<_>>>()?;
    ordered.sort_by(|a, b| a.0.total_cmp(&b.0));
    PyList::new(
        manager.py(),
        ordered
            .into_iter()
            .filter(|(time, _)| manual || *time <= cutoff)
            .take(20)
            .map(|(_, id)| id)
            .collect::<Vec<_>>(),
    )
}
pub fn complete(
    manager: &Bound<'_, PyAny>,
    batch: &Bound<'_, PyAny>,
    result: &Bound<'_, PyAny>,
    files: bool,
) -> PyResult<()> {
    let py = manager.py();
    let wall = clock(py, "time")?;
    let mut saved = state(manager)?;
    let retry = if files {
        result.getattr("retry_after")?.extract::<f64>()?.max(60.0)
    } else {
        60.0
    };
    let versions = if files {
        result.getattr("versions")?
    } else {
        result.clone()
    };
    for id in batch.try_iter()? {
        let id = id?;
        let version = versions.call_method1("get", (&id,))?;
        if retry > 60.0 && version.is_none() {
            continue;
        }
        let mut record = json!({"checked":wall,"version":value(&version)?});
        if files {
            let state = result.getattr("states")?.call_method1("get", (&id,))?;
            if !state.is_none() {
                record["state"] =
                    value(&py.import("dataclasses")?.call_method1("asdict", (state,))?)?;
            } else if retry > 60.0 {
                continue;
            }
        }
        saved["records"][key(&id)?] = record;
    }
    saved["blocked"] = json!(wall + retry);
    persist(manager, &saved)?;
    let ids = prepare(manager, files)?;
    due(manager, &saved, &ids)
}
pub fn failed(manager: &Bound<'_, PyAny>, delay: f64) -> PyResult<()> {
    let mut saved = state(manager)?;
    saved["blocked"] = json!(clock(manager.py(), "time")? + delay);
    persist(manager, &saved)
}
