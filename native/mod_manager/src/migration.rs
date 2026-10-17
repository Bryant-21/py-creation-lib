use crate::{
    check_cancel,
    database::{Database, error, records},
    encoding,
    executables::text,
    invalid,
    python_io::{with_context, with_database},
    reject_links,
    transactions::transaction_key,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use rusqlite::params;
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf};
use unicode_casefold::UnicodeCaseFold;

#[pyclass]
struct Progress {
    callback: Py<PyAny>,
    offset: f64,
    scale: f64,
    prefix: String,
}
#[pymethods]
impl Progress {
    fn __call__(&self, py: Python<'_>, fraction: f64, name: &str) -> PyResult<()> {
        if !self.callback.bind(py).is_none() {
            self.callback.call1(
                py,
                (
                    self.offset + fraction * self.scale,
                    format!("{}{name}", self.prefix),
                ),
            )?;
        }
        Ok(())
    }
}
fn progress<'py>(
    callback: &Bound<'py, PyAny>,
    offset: f64,
    scale: f64,
    prefix: String,
) -> PyResult<Bound<'py, PyAny>> {
    Ok(Py::new(
        callback.py(),
        Progress {
            callback: callback.clone().unbind(),
            offset,
            scale,
            prefix,
        },
    )?
    .into_bound(callback.py())
    .into_any())
}
fn value(object: &Bound<'_, PyAny>) -> PyResult<Value> {
    encoding::parse(
        &object
            .py()
            .import("json")?
            .call_method1("dumps", (object,))?
            .extract::<String>()?,
    )
}
fn object<'py>(py: Python<'py>, value: &Value) -> PyResult<Bound<'py, PyAny>> {
    py.import("json")?
        .call_method1("loads", (value.to_string(),))
}
fn path<'py>(py: Python<'py>, value: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import("pathlib")?.getattr("Path")?.call1((value,))
}
fn snapshot(db: &Database, library: &Bound<'_, PyAny>, id: &str) -> PyResult<Value> {
    transaction_key(id)?;
    let row = records(&*db.connection()?, "SELECT * FROM mods WHERE id=?", [id])?.into_iter().next()
        .filter(|v| v["kind"] == "external" && !text(v, "source").is_empty())
        .ok_or_else(|| invalid("Only available imported mod sources can be copied. Unmanaged game content, separators, and owned mods cannot be migrated."))?;
    let source = path(library.py(), text(&row, "source"))?.call_method0("absolute")?;
    reject_links(&source.extract::<PathBuf>()?)?;
    if !source.call_method0("is_dir")?.is_truthy()? {
        return Err(invalid(format!(
            "Imported source folder is missing: {source}"
        )));
    }
    let source = source.call_method0("resolve")?;
    let root = library.getattr("root")?.call_method0("resolve")?;
    if source
        .call_method1("is_relative_to", (&root,))?
        .is_truthy()?
        || root
            .call_method1("is_relative_to", (&source,))?
            .is_truthy()?
    {
        return Err(invalid(
            "Imported source and owned library must be separate.",
        ));
    }
    let entries = records(
        &*db.connection()?,
        "SELECT * FROM entries WHERE mod_id=? ORDER BY profile_id",
        [id],
    )?;
    Ok(json!({"mod":row,"entries":entries}))
}
fn package<'py>(
    py: Python<'py>,
    source: &str,
    cancel: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let module = py.import("creation_lib.mod_manager.package")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("inspect_installer", false)?;
    module.getattr("Package")?.call(
        (source, cancel, module.getattr("OWNED_LIMITS")?),
        Some(&kwargs),
    )
}
fn review(
    db: &Database,
    library: &Bound<'_, PyAny>,
    id: &str,
    cancel: &Bound<'_, PyAny>,
    report: &Bound<'_, PyAny>,
) -> PyResult<(Value, Value)> {
    let py = library.py();
    let state = snapshot(db, library, id)?;
    let source = text(&state["mod"], "source");
    let manifest = value(
        &library
            .getattr("owned")?
            .call_method1("_manifest", (source, cancel, report))?,
    )?;
    let package = package(py, source, cancel)?;
    let inspected = (|| -> PyResult<Vec<String>> {
        let mut directories = package
            .getattr("directories")?
            .extract::<BTreeSet<String>>()?
            .into_iter()
            .collect::<Vec<_>>();
        directories.sort();
        let files = package
            .getattr("files")?
            .call_method0("values")?
            .try_iter()?
            .map(|value| value?.extract::<String>())
            .collect::<PyResult<Vec<_>>>()?;
        let sizes = package.getattr("sizes")?;
        if files.len() != manifest.as_object().unwrap().len() {
            return Err(invalid(
                "Source changed while reviewing; review the copy again.",
            ));
        }
        for name in files {
            let key: String = name.case_fold().collect();
            if sizes.get_item(key)?.extract::<u64>()?
                != manifest[&name]["size"].as_u64().unwrap_or(u64::MAX)
            {
                return Err(invalid(
                    "Source changed while reviewing; review the copy again.",
                ));
            }
        }
        Ok(directories)
    })();
    package.call_method0("close")?;
    let directories = inspected?;
    let names: Vec<_> = records(&*db.connection()?, "SELECT p.name FROM profiles p JOIN entries e ON e.profile_id=p.id WHERE e.mod_id=? ORDER BY p.id", [id])?.iter().map(|v| text(v,"name").to_owned()).collect();
    let sizes: Vec<_> = manifest
        .as_object()
        .unwrap()
        .values()
        .map(|v| v["size"].as_u64().unwrap_or(0))
        .collect();
    let bytes: u64 = sizes.iter().sum();
    let allocated: u64 = sizes
        .iter()
        .map(|size| 4096.max(size.div_ceil(4096) * 4096))
        .sum::<u64>()
        + 4096 * directories.len() as u64;
    let token = encoding::digest(
        &json!({"snapshot":state,"files":manifest,"directories":directories}),
        true,
        false,
    );
    Ok((
        json!({"mod_id":id,"name":state["mod"]["name"],"source":source,"token":token,"manifest":manifest,"directories":directories,"profiles":names,"bytes":bytes,"allocated":allocated,"required":allocated + (64*1024*1024).max(allocated/20)}),
        state,
    ))
}
fn free(library: &Bound<'_, PyAny>, usage: &Bound<'_, PyAny>) -> PyResult<u64> {
    usage
        .call1((library.getattr("root")?,))?
        .getattr("free")?
        .extract()
}

fn copy_one(
    library: &Bound<'_, PyAny>,
    plan: &Value,
    usage: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    report: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let py = library.py();
    let owned = library.getattr("owned")?;
    let factory = library.getattr("_db")?;
    with_context(&owned.call_method0("lock")?, |_| {
        owned.call_method0("_recover_locked")?;
        let (current, state) = with_database(&factory, |_, db| {
            review(
                db,
                library,
                text(plan, "mod_id"),
                cancel,
                &progress(report, 0., 0.25, String::new())?,
            )
        })?;
        if current != *plan {
            return Err(invalid(
                "Source files, metadata, directories, or profile references changed after review. Review the copy again.",
            ));
        }
        if free(library, usage)? < plan["required"].as_u64().unwrap_or(u64::MAX) {
            return Err(invalid(
                "Insufficient free space for this copy and its safety headroom. No destination was created.",
            ));
        }
        let root = library.getattr("root")?.extract::<PathBuf>()?;
        let destination = root
            .join("mods")
            .join(transaction_key(text(plan, "mod_id"))?);
        reject_links(&destination)?;
        if destination.exists() {
            return Err(invalid(
                "Owned destination is occupied; no existing content was overwritten.",
            ));
        }
        let kwargs = PyDict::new(py);
        kwargs.set_item("origin", encoding::digest(&plan["source"], true, false))?;
        let record = owned.call_method(
            "_begin",
            ("migrate", text(plan, "mod_id"), false),
            Some(&kwargs),
        )?;
        let paths = owned.call_method1("_paths", (&record,))?;
        let stage = paths.get_item(1)?;
        let destination = paths.get_item(2)?;
        let mut committed = false;
        let result = (|| -> PyResult<String> {
            stage.call_method0("mkdir")?;
            let package = package(py, text(plan, "source"), cancel)?;
            let copied = (|| -> PyResult<Value> {
                let sources = PyList::empty(py);
                for entry in package
                    .getattr("files")?
                    .call_method0("values")?
                    .try_iter()?
                {
                    let entry = entry?;
                    sources.append((&entry, &package, &entry))?;
                }
                let kwargs = PyDict::new(py);
                kwargs.set_item("byte_progress", true)?;
                let manifest = value(&library.call_method(
                    "_copy_plan",
                    (
                        &sources,
                        &stage,
                        cancel,
                        progress(report, 0.25, 0.5, "Copying ".into())?,
                    ),
                    Some(&kwargs),
                )?)?;
                for name in plan["directories"].as_array().unwrap() {
                    check_cancel(Some(cancel))?;
                    let directory = stage.call_method1(
                        "joinpath",
                        (crate::relative_path(name.as_str().unwrap(), false)?,),
                    )?;
                    reject_links(&directory.extract::<PathBuf>()?)?;
                    let kwargs = PyDict::new(py);
                    kwargs.set_item("parents", true)?;
                    kwargs.set_item("exist_ok", true)?;
                    directory.call_method("mkdir", (), Some(&kwargs))?;
                }
                Ok(manifest)
            })();
            package.call_method0("close")?;
            let manifest = copied?;
            if manifest != plan["manifest"] {
                return Err(invalid(
                    "Source bytes changed during copying; review the copy again.",
                ));
            }
            owned.call_method1("_checkpoint", ("prepared",))?;
            with_database(&factory, |wrapper, db| {
                db.connection()?
                    .execute_batch("BEGIN IMMEDIATE")
                    .map_err(error)?;
                let (latest, _) = review(
                    db,
                    library,
                    text(plan, "mod_id"),
                    cancel,
                    &progress(report, 0.75, 0.24, String::new())?,
                )?;
                if latest != *plan {
                    return Err(invalid(
                        "Source or organization changed during copying; review the copy again.",
                    ));
                }
                check_cancel(Some(cancel))?;
                stage.call_method1("rename", (&destination,))?;
                owned.call_method1("_checkpoint", ("placed",))?;
                db.connection()?
                    .execute(
                        "UPDATE mods SET kind='owned',source=?,manifest=? WHERE id=?",
                        params![
                            destination.str()?.to_str()?,
                            manifest.to_string(),
                            text(plan, "mod_id")
                        ],
                    )
                    .map_err(error)?;
                db.connection()?.execute("INSERT INTO origins(mod_id,source,identity) VALUES (?,?,?) ON CONFLICT(mod_id) DO UPDATE SET source=excluded.source,identity=excluded.identity,copied=CURRENT_TIMESTAMP",params![text(plan,"mod_id"),text(plan,"source"),encoding::digest(&json!({"source":plan["source"],"files":plan["manifest"]}),true,false)]).map_err(error)?;
                owned.call_method1("_record", (wrapper, &record, object(py, &state)?))?;
                owned.call_method1("_checkpoint", ("before-commit",))?;
                check_cancel(Some(cancel))
            })?;
            committed = true;
            owned.call_method1("_checkpoint", ("committed",))?;
            if !report.is_none() {
                report.call1((1., "Copy committed"))?;
            }
            owned.call_method1("_finish", (&record,))?;
            Ok(text(plan, "mod_id").into())
        })();
        match result {
            Ok(value) => Ok(value),
            Err(original) => {
                let confirmed = if committed {
                    Ok(true)
                } else {
                    owned
                        .call_method1("_abort", (&record,))
                        .and_then(|v| v.is_truthy())
                };
                match confirmed {
                    Err(recovery) => Err(invalid(format!(
                        "Copy interrupted; rollback needs recovery on library reopen. Retained journal: {}. {recovery}",
                        record.get_item("id")?
                    ))),
                    Ok(true) => Err(crate::transactions::TransactionCommitted::new_err(
                        value(&record)?.to_string(),
                    )),
                    Ok(false) => Err(original),
                }
            }
        }
    })
}

#[pyfunction]
fn migration_review(
    library: &Bound<'_, PyAny>,
    ids: Vec<String>,
    limit: usize,
    usage: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    report: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let mut seen = BTreeSet::new();
    let ids: Vec<_> = ids
        .into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect();
    if ids.is_empty() {
        return Err(invalid("Select at least one imported source to copy."));
    }
    let owned = library.getattr("owned")?;
    with_context(&owned.call_method0("lock")?, |_| {
        with_database(&library.getattr("_db")?, |_, db| {
            owned.call_method0("_recover_locked")?;
            let mut sources = Vec::new();
            let mut entries = 0;
            for (index, id) in ids.iter().enumerate() {
                check_cancel(Some(cancel))?;
                if !report.is_none() {
                    report.call1((
                        index as f64 / ids.len() as f64,
                        format!("Hashing source {} / {}", index + 1, ids.len()),
                    ))?;
                }
                let (source, _) = review(
                    db,
                    library,
                    id,
                    cancel,
                    &progress(
                        report,
                        index as f64 / ids.len() as f64,
                        1. / ids.len() as f64,
                        String::new(),
                    )?,
                )?;
                entries += source["manifest"].as_object().unwrap().len()
                    + source["directories"].as_array().unwrap().len();
                if entries > limit {
                    return Err(invalid(
                        "Copy review exceeds the 1,000,000-entry batch limit. Select a smaller batch; no copies were started.",
                    ));
                }
                sources.push(source);
            }
            let allocated: u64 = sources
                .iter()
                .map(|v| v["allocated"].as_u64().unwrap())
                .sum();
            let bytes: u64 = sources.iter().map(|v| v["bytes"].as_u64().unwrap()).sum();
            Ok(json!({"sources":sources,"bytes":bytes,"required":allocated+(64*1024*1024).max(allocated/20),"free":free(library,usage)?}).to_string())
        })
    })
}
#[pyfunction]
fn migration_copy(
    library: &Bound<'_, PyAny>,
    plan: &str,
    usage: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    report: &Bound<'_, PyAny>,
) -> PyResult<Vec<String>> {
    let py = library.py();
    let plan = encoding::parse(plan)?;
    if free(library, usage)? < plan["required"].as_u64().unwrap_or(u64::MAX) {
        return Err(invalid(
            "Insufficient free space for the reviewed batch and safety headroom. No copies were started.",
        ));
    }
    let sources = plan["sources"]
        .as_array()
        .ok_or_else(|| invalid("Invalid source copy review."))?;
    let mut completed = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let result = (|| {
            check_cancel(Some(cancel))?;
            copy_one(
                library,
                source,
                usage,
                cancel,
                &progress(
                    report,
                    index as f64 / sources.len() as f64,
                    1. / sources.len() as f64,
                    format!("{}: ", text(source, "name")),
                )?,
            )
        })();
        match result {
            Ok(id) => completed.push(id),
            Err(error) => {
                if error.is_instance_of::<crate::transactions::TransactionCommitted>(py) {
                    return Err(error);
                }
                let paths = py.import("creation_lib.mod_manager.paths")?;
                if !error.is_instance_of::<pyo3::exceptions::PyException>(py) {
                    return Err(error);
                }
                let cancelled = error.is_instance_of::<crate::OperationCancelled>(py)
                    || error.matches(py, paths.getattr("Cancelled")?)?;
                let module = py.import("creation_lib.mod_manager.migration")?;
                let exception = if cancelled {
                    module
                        .getattr("MigrationCancelled")?
                        .call1((&completed, sources.len()))?
                } else {
                    module.getattr("MigrationInterrupted")?.call1((
                        &completed,
                        sources.len(),
                        error.value(py).str()?.to_str()?,
                    ))?
                };
                return Err(PyErr::from_value(exception));
            }
        }
    }
    Ok(completed)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(migration_review, m)?)?;
    m.add_function(wrap_pyfunction!(migration_copy, m)?)?;
    Ok(())
}
