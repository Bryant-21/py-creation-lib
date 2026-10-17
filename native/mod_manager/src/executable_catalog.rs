use crate::{
    check_cancel, encoding, invalid, plugin_inventory::value, python_io::with_context, saves::fold,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
fn preferences(db: &Bound<'_, PyAny>, instance: &str) -> PyResult<Value> {
    if !db
        .call_method1(
            "execute",
            ("SELECT 1 FROM sqlite_master WHERE name='executable_preferences'",),
        )?
        .call_method0("fetchone")?
        .is_none()
    {
        let row = db
            .call_method1(
                "execute",
                (
                    "SELECT configuration FROM executable_preferences WHERE instance_id=?",
                    (instance,),
                ),
            )?
            .call_method0("fetchone")?;
        if !row.is_none() {
            return encoding::parse(&row.get_item(0)?.extract::<String>()?);
        }
    }
    Ok(json!({"order":[],"default":""}))
}
fn snapshot(db: &Bound<'_, PyAny>, instance: &str) -> PyResult<String> {
    let py = db.py();
    let mut records = Vec::new();
    for row in db.call_method1("execute",("SELECT id,instance_id,configuration,removed FROM executables WHERE instance_id='' OR instance_id=? ORDER BY id",(instance,)))?.try_iter()? {records.push(value(&py.import("builtins")?.getattr("tuple")?.call1((row?,))?)?);}
    Ok(encoding::digest(
        &json!([records, preferences(db, instance)?]),
        false,
        false,
    ))
}
#[pyfunction]
fn executable_catalog_preferences(
    library: &Bound<'_, PyAny>,
    instance: &str,
    db: &Bound<'_, PyAny>,
) -> PyResult<String> {
    Ok(if db.is_none() {
        with_context(&library.call_method0("_db")?, |db| {
            preferences(db, instance)
        })?
    } else {
        preferences(db, instance)?
    }
    .to_string())
}
#[pyfunction]
fn executable_catalog_snapshot(
    library: &Bound<'_, PyAny>,
    instance: &str,
    db: &Bound<'_, PyAny>,
) -> PyResult<String> {
    if db.is_none() {
        with_context(&library.call_method0("_db")?, |db| snapshot(db, instance))
    } else {
        snapshot(db, instance)
    }
}
#[pyfunction]
fn executable_catalog_ordered(
    library: &Bound<'_, PyAny>,
    items: &Bound<'_, PyAny>,
    instance: &str,
) -> PyResult<Py<PyAny>> {
    let prefs = with_context(&library.call_method0("_db")?, |db| {
        preferences(db, instance)
    })?;
    let ranks = prefs["order"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, v)| (v.as_str().unwrap_or("").to_string(), i))
        .collect::<BTreeMap<_, _>>();
    let mut result = Vec::new();
    for item in items.try_iter()? {
        let item = item?;
        let id: String = item.getattr("id")?.extract()?;
        let title = fold(&item.getattr("title")?.extract::<String>()?);
        result.push((*ranks.get(&id).unwrap_or(&ranks.len()), title, id, item));
    }
    result.sort_by(|a, b| (&a.0, &a.1, &a.2).cmp(&(&b.0, &b.1, &b.2)));
    Ok(PyList::new(library.py(), result.into_iter().map(|v| v.3))?
        .into_any()
        .unbind())
}
#[pyfunction]
fn executable_catalog_save(
    library: &Bound<'_, PyAny>,
    instance: &str,
    expected: &str,
    items: &Bound<'_, PyAny>,
    default: &str,
    cancel: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let definition = py
        .import("creation_lib.mod_manager.launcher")?
        .getattr("definition")?;
    let mut values = Vec::new();
    let mut ids = BTreeSet::new();
    let mut visible = false;
    for item in items.try_iter()? {
        let record = item?.call_method0("record")?;
        let item = definition.call((), Some(record.cast::<PyDict>()?))?;
        let id: String = item.getattr("id")?.extract()?;
        ids.insert(id.clone());
        visible |= id == default && !item.getattr("hidden")?.is_truthy()?;
        values.push(item);
    }
    if values.len() > 256 || ids.len() != values.len() {
        return Err(invalid("Use at most 256 unique executable definitions."));
    }
    for item in &values {
        let owner: String = item.getattr("instance_id")?.extract()?;
        if !owner.is_empty() && owner != instance {
            return Err(invalid(
                "An executable belongs to a different imported instance.",
            ));
        }
    }
    if !default.is_empty() && !visible {
        return Err(invalid(
            "The default executable must be a visible entry in this list.",
        ));
    }
    library.call_method0("_runtime_write_guard")?;
    with_context(&library.call_method0("_db")?, |db| {
        db.call_method1("execute", ("BEGIN IMMEDIATE",))?;
        if snapshot(db, instance)? != expected {
            return Err(invalid(
                "Executable definitions changed while this dialog was open. Reopen it before saving.",
            ));
        }
        db.call_method1(
            "execute",
            (
                "UPDATE executables SET removed=1 WHERE instance_id='' OR instance_id=?",
                (instance,),
            ),
        )?;
        let mut order = Vec::new();
        for item in &values {
            let id: String = item.getattr("id")?.extract()?;
            let other = db
                .call_method1(
                    "execute",
                    ("SELECT instance_id FROM executables WHERE id=?", (&id,)),
                )?
                .call_method0("fetchone")?;
            if !other.is_none() {
                let owner: String = other.get_item(0)?.extract()?;
                if !owner.is_empty() && owner != instance {
                    return Err(invalid("Executable identity is owned by another instance."));
                }
            }
            db.call_method1("execute",("INSERT INTO executables VALUES (?,?,?,0) ON CONFLICT(id) DO UPDATE SET configuration=excluded.configuration,removed=0",(&id,item.getattr("instance_id")?,encoding::encode(&value(&item.call_method0("record")?)?,true,true))))?;
            order.push(id);
        }
        db.call_method1("execute",("CREATE TABLE IF NOT EXISTS executable_preferences(instance_id TEXT PRIMARY KEY,configuration TEXT NOT NULL)",))?;
        db.call_method1("execute",("INSERT INTO executable_preferences VALUES (?,?) ON CONFLICT(instance_id) DO UPDATE SET configuration=excluded.configuration",(instance,json!({"order":order,"default":default}).to_string())))?;
        checkpoint.call0()?;
        check_cancel(Some(cancel))?;
        library.call_method0("_runtime_write_guard")?;
        Ok(())
    })?;
    Ok(PyList::new(py, values)?.into_any().unbind())
}
#[pyfunction]
fn executable_catalog_instance(manager: &Bound<'_, PyAny>) -> PyResult<String> {
    for profile in manager.getattr("profiles")?.try_iter()? {
        let profile = profile?;
        if profile.getattr("id")?.eq(manager.getattr("profile_id")?)? {
            return profile
                .getattr("files")?
                .call_method1("get", ("_modbox_instance_id", ""))?
                .extract();
        }
    }
    Ok("".into())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(executable_catalog_preferences, m)?)?;
    m.add_function(wrap_pyfunction!(executable_catalog_snapshot, m)?)?;
    m.add_function(wrap_pyfunction!(executable_catalog_ordered, m)?)?;
    m.add_function(wrap_pyfunction!(executable_catalog_save, m)?)?;
    m.add_function(wrap_pyfunction!(executable_catalog_instance, m)?)?;
    Ok(())
}
