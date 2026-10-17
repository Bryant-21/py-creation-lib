use crate::{
    check_cancel,
    database::{error, records},
    encoding,
    history_prune::{py_path, stamp},
    invalid,
    python_io::{with_context, with_database},
    reject_links,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("creation_lib.mod_manager.metadata_observer")
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn schema(db: &rusqlite::Connection) -> PyResult<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS source_metadata_observations (owner_id TEXT,kind TEXT,path TEXT,name TEXT,baseline TEXT,observed TEXT,issue TEXT,changed INTEGER,PRIMARY KEY(owner_id,kind,path))").map_err(error)
}
#[pyfunction]
fn observer_targets(library: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let class = module(py)?.getattr("Target")?;
    let result = with_database(&library.getattr("_db")?, |_, db| {
        let connection = db.connection()?;
        let mut result = Vec::new();
        for row in records(
            &connection,
            "SELECT id,name,source FROM mods WHERE kind='external' AND source!='' ORDER BY id",
            [],
        )? {
            result.push(
                class
                    .call1((
                        text(&row, "id"),
                        "mod",
                        text(&row, "name"),
                        PathBuf::from(text(&row, "source"))
                            .join("meta.ini")
                            .to_string_lossy()
                            .as_ref(),
                        text(&row, "source"),
                    ))?
                    .unbind(),
            );
        }
        for row in records(
            &connection,
            "SELECT id,source,configuration FROM instances ORDER BY id",
            [],
        )? {
            let config = encoding::parse(text(&row, "configuration"))?;
            let root = config["paths"]["instance"].as_str().unwrap_or("");
            if !root.is_empty() {
                let path = PathBuf::from(root);
                result.push(
                    class
                        .call1((
                            text(&row, "id"),
                            "categories",
                            path.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .as_ref(),
                            path.join("categories.dat").to_string_lossy().as_ref(),
                            root,
                        ))?
                        .unbind(),
                );
            }
        }
        Ok(result)
    })?;
    if result.len() > 100_000 {
        return Err(invalid(
            "Source metadata observation exceeds its 100,000-file bound.",
        ));
    }
    Ok(PyTuple::new(py, result)?.into_any().unbind())
}
#[pyfunction]
fn observer_fingerprint(
    target: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<(String, String, usize)> {
    let py = target.py();
    let path =
        py_path(py, &target.getattr("path")?.extract::<PathBuf>()?)?.call_method0("absolute")?;
    let root: PathBuf = path.extract()?;
    let result = (|| -> PyResult<(String, String, usize)> {
        if !py
            .import("creation_lib.mod_manager.mo2")?
            .call_method1("local_mod_target", (&path,))?
            .is_truthy()?
        {
            return Err(invalid("Source metadata requires an ordinary local path."));
        }
        reject_links(&root)?;
        let before = match path.call_method0("stat") {
            Ok(v) => v,
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyFileNotFoundError>(py) => {
                return Ok((
                    encoding::encode(&json!({"state":"missing"}), true, false),
                    String::new(),
                    0,
                ));
            }
            Err(e) => return Err(e),
        };
        if before.getattr("st_mode")?.extract::<u32>()? & 0o170000 != 0o100000
            || before.getattr("st_size")?.extract::<u64>()? > 1024 * 1024
        {
            return Err(invalid(
                "Observed metadata must be an ordinary file of at most 1 MiB.",
            ));
        }
        check_cancel(Some(cancel))?;
        let data: Vec<u8> = with_context(&path.call_method1("open", ("rb",))?, |stream| {
            stream.call_method1("read", (1024 * 1024 + 1,))?.extract()
        })?;
        check_cancel(Some(cancel))?;
        reject_links(&root)?;
        let after = path.call_method0("stat")?;
        if data.len() > 1024 * 1024 || stamp(&before)? != stamp(&after)? {
            return Err(invalid(
                "Metadata changed during observation; retry without importing fields.",
            ));
        }
        Ok((
            encoding::encode(
                &json!({"state":"file","stamp":stamp(&after)?,"sha256":format!("{:x}",Sha256::digest(&data))}),
                true,
                false,
            ),
            String::new(),
            data.len(),
        ))
    })();
    match result {
        Err(e)
            if e.is_instance_of::<pyo3::exceptions::PyOSError>(py)
                || (e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
                    && !e.is_instance_of::<crate::OperationCancelled>(py)
                    && !e.is_instance(
                        py,
                        &py.import("creation_lib.mod_manager.paths")?
                            .getattr("Cancelled")?,
                    )) =>
        {
            let message = e.value(py).str()?.to_str()?.to_owned();
            Ok((
                encoding::encode(
                    &json!({"state":"unavailable","reason":message}),
                    true,
                    false,
                ),
                message,
                0,
            ))
        }
        other => other,
    }
}
fn current(db: &rusqlite::Connection, target: &Bound<'_, PyAny>) -> PyResult<bool> {
    let kind: String = target.getattr("kind")?.extract()?;
    let id: String = target.getattr("owner_id")?.extract()?;
    let source: String = target.getattr("source")?.extract()?;
    if kind == "mod" {
        let rows = records(db, "SELECT source,kind FROM mods WHERE id=?", [id])?;
        Ok(rows
            .first()
            .is_some_and(|v| text(v, "kind") == "external" && text(v, "source") == source))
    } else {
        let rows = records(db, "SELECT configuration FROM instances WHERE id=?", [id])?;
        match rows.first() {
            None => Ok(false),
            Some(v) => Ok(
                encoding::parse(text(v, "configuration"))?["paths"]["instance"]
                    .as_str()
                    .unwrap_or("")
                    == source,
            ),
        }
    }
}
fn notices(py: Python<'_>, db: &rusqlite::Connection) -> PyResult<(Py<PyAny>, usize)> {
    let condition = "changed=1 AND ((kind='mod' AND EXISTS(SELECT 1 FROM mods m WHERE m.id=o.owner_id AND m.kind='external')) OR (kind='categories' AND EXISTS(SELECT 1 FROM instances i WHERE i.id=o.owner_id)))";
    let count: usize = db
        .query_row(
            &format!("SELECT count(*) FROM source_metadata_observations o WHERE {condition}"),
            [],
            |row| row.get(0),
        )
        .map_err(error)?;
    let class = module(py)?.getattr("Notice")?;
    let mut notices = Vec::new();
    for row in records(
        db,
        &format!(
            "SELECT * FROM source_metadata_observations o WHERE {condition} ORDER BY kind,owner_id,path LIMIT 200"
        ),
        [],
    )? {
        let old = encoding::parse(text(&row, "baseline"))?;
        let new = encoding::parse(text(&row, "observed"))?;
        let change = if !text(&row, "issue").is_empty() {
            "unavailable"
        } else if text(&new, "state") == "missing" {
            "removed"
        } else if text(&old, "state") == "missing" {
            "created"
        } else {
            "changed"
        };
        notices.push(class.call1((
            text(&row, "owner_id"),
            text(&row, "kind"),
            text(&row, "name"),
            text(&row, "path"),
            change,
            text(&row, "observed"),
            text(&row, "issue"),
        ))?);
    }
    Ok((PyTuple::new(py, notices)?.into_any().unbind(), count))
}
#[pyfunction]
fn observer_scan(
    library: &Bound<'_, PyAny>,
    watched: &Bound<'_, PyAny>,
    cursor: usize,
    visited: usize,
    cancel: &Bound<'_, PyAny>,
    selected: &str,
    batch_files: usize,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let watched = if watched.is_none() {
        observer_targets(library)?.into_bound(py)
    } else {
        watched.clone()
    };
    let length = watched.len()?;
    let mut batch = Vec::new();
    for step in 0..batch_files.min(length) {
        batch.push(watched.get_item((cursor + step) % length)?);
    }
    let mut inserted = false;
    for target in watched.try_iter()? {
        let target = target?;
        if target.getattr("kind")?.extract::<String>()? == "mod"
            && target.getattr("owner_id")?.extract::<String>()? == selected
        {
            let mut contains = false;
            for item in &batch {
                contains |= item.eq(&target)?;
            }
            if !contains {
                batch.insert(0, target);
                inserted = true;
            }
            break;
        }
    }
    let mut samples = Vec::new();
    let mut bytes = 0;
    let mut consumed = 0;
    for (index, target) in batch.into_iter().enumerate() {
        check_cancel(Some(cancel))?;
        let (value, issue, size) = observer_fingerprint(&target, cancel)?;
        samples.push((target, value, issue));
        bytes += size;
        if !inserted || index > 0 {
            consumed += 1;
        }
        if bytes >= 4 * 1024 * 1024 {
            break;
        }
    }
    library.call_method0("_runtime_write_guard")?;
    let (notices, count, changed) = with_database(&library.getattr("_db")?, |_, db| {
        let connection = db.connection()?;
        connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
        schema(&connection)?;
        let mut changed = Vec::new();
        for (target, value, issue) in samples {
            check_cancel(Some(cancel))?;
            if !current(&connection, &target)? {
                continue;
            }
            let id: String = target.getattr("owner_id")?.extract()?;
            let kind: String = target.getattr("kind")?.extract()?;
            let path: String = target.getattr("path")?.extract()?;
            let name: String = target.getattr("name")?.extract()?;
            connection.execute("DELETE FROM source_metadata_observations WHERE owner_id=? AND kind=? AND path!=?",rusqlite::params![id,kind,path]).map_err(error)?;
            let previous = records(
                &connection,
                "SELECT baseline,observed FROM source_metadata_observations WHERE owner_id=? AND kind=? AND path=?",
                rusqlite::params![id, kind, path],
            )?;
            let baseline = if let Some(previous) = previous.first() {
                if text(previous, "observed") != value {
                    changed.push(path.clone());
                }
                text(previous, "baseline").to_owned()
            } else if !issue.is_empty() {
                encoding::encode(&json!({"state":"missing"}), true, false)
            } else {
                value.clone()
            };
            connection.execute("INSERT INTO source_metadata_observations VALUES (?,?,?,?,?,?,?,?) ON CONFLICT(owner_id,kind,path) DO UPDATE SET name=excluded.name,observed=excluded.observed,issue=excluded.issue,changed=excluded.changed",rusqlite::params![id,kind,path,name,baseline,value,issue,baseline!=value]).map_err(error)?;
        }
        check_cancel(Some(cancel))?;
        let (notices, count) = notices(py, &connection)?;
        Ok((notices, count, changed))
    })?;
    Ok(module(py)?
        .getattr("Index")?
        .call1((
            watched,
            if length > 0 {
                (cursor + consumed) % length
            } else {
                0
            },
            length.min(visited + consumed),
            notices,
            count,
            PyTuple::new(py, changed)?,
        ))?
        .unbind())
}
#[pyfunction]
fn observer_dismiss(library: &Bound<'_, PyAny>, notice: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = library.py();
    library.call_method0("_runtime_write_guard")?;
    let path: PathBuf = notice.getattr("path")?.extract()?;
    let target = module(py)?.getattr("Target")?.call1((
        notice.getattr("owner_id")?,
        notice.getattr("kind")?,
        notice.getattr("name")?,
        notice.getattr("path")?,
        path.parent()
            .unwrap_or(std::path::Path::new(""))
            .to_string_lossy()
            .as_ref(),
    ))?;
    let observed: String = notice.getattr("observed")?.extract()?;
    if observer_fingerprint(&target, py.None().bind(py))?.0 != observed {
        return Err(invalid(
            "Source metadata changed again. Refresh notices before dismissing it.",
        ));
    }
    with_database(&library.getattr("_db")?, |_, db| {
        let connection = db.connection()?;
        connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
        schema(&connection)?;
        if !current(&connection, &target)? {
            return Err(invalid(
                "The source reference changed. Refresh notices before dismissal.",
            ));
        }
        let id: String = notice.getattr("owner_id")?.extract()?;
        let kind: String = notice.getattr("kind")?.extract()?;
        let path = path.to_string_lossy();
        let rows = records(
            &connection,
            "SELECT observed FROM source_metadata_observations WHERE owner_id=? AND kind=? AND path=?",
            rusqlite::params![id, kind, path.as_ref()],
        )?;
        if !rows
            .first()
            .is_some_and(|v| text(v, "observed") == observed)
        {
            return Err(invalid(
                "Metadata notice changed; refresh it before dismissal.",
            ));
        }
        connection.execute("UPDATE source_metadata_observations SET baseline=observed,changed=0 WHERE owner_id=? AND kind=? AND path=?",rusqlite::params![id,kind,path.as_ref()]).map_err(error)?;
        Ok(())
    })
}
fn now(py: Python<'_>) -> PyResult<f64> {
    py.import("time")?.call_method0("monotonic")?.extract()
}
#[pyfunction]
fn observer_initialize(observer: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = observer.py();
    let kwargs = PyDict::new(py);
    kwargs.set_item("max_workers", 1)?;
    kwargs.set_item("thread_name_prefix", "source-metadata")?;
    observer.setattr(
        "executor",
        py.import("concurrent.futures")?
            .getattr("ThreadPoolExecutor")?
            .call((), Some(&kwargs))?,
    )?;
    for name in ["future", "index", "library"] {
        observer.setattr(name, py.None())?;
    }
    observer.setattr("cancel", py.import("threading")?.getattr("Event")?.call0()?)?;
    observer.setattr("generation", 0)?;
    observer.setattr("due", f64::INFINITY)?;
    observer.setattr("error", "")?;
    Ok(())
}
#[pyfunction]
fn observer_invalidate(observer: &Bound<'_, PyAny>) -> PyResult<()> {
    observer.getattr("cancel")?.call_method0("set")?;
    observer.setattr(
        "generation",
        observer.getattr("generation")?.extract::<u64>()? + 1,
    )?;
    observer.setattr("index", observer.py().None())?;
    observer.setattr("due", now(observer.py())? + 0.2)?;
    Ok(())
}
fn same(observer: &Bound<'_, PyAny>, manager: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(observer
        .getattr("running_generation")?
        .eq(observer.getattr("generation")?)?
        && observer
            .getattr("library")?
            .is(&manager.getattr("library")?))
}
#[pyfunction]
fn observer_poll(observer: &Bound<'_, PyAny>, manager: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = observer.py();
    let future = observer.getattr("future")?;
    if future.is_truthy()? && future.call_method0("done")?.is_truthy()? {
        observer.setattr("future", py.None())?;
        let result = (|| -> PyResult<()> {
            let result = future.call_method0("result")?;
            if same(observer, manager)? {
                observer.setattr("index", &result)?;
                observer.setattr("error", "")?;
                let plan = manager.getattr("source_metadata_review")?;
                let changed = result.getattr("changed_paths")?;
                if plan.is_truthy()? {
                    for item in plan.getattr("inputs")?.try_iter()? {
                        if changed.contains(item?.get_item(0)?)? {
                            manager.setattr("source_metadata_review", py.None())?;
                            manager.setattr("status","Source metadata changed while its review was open. Review it again before selecting fields.")?;
                            break;
                        }
                    }
                }
                if changed.is_truthy()? {
                    manager.setattr("_filter_key", py.None())?;
                    manager.setattr("_status_key", py.None())?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            if error.is_instance(
                py,
                &py.import("creation_lib.mod_manager.paths")?
                    .getattr("Cancelled")?,
            ) {
            } else if error.is_instance_of::<pyo3::exceptions::PyException>(py) {
                if same(observer, manager)? {
                    observer.setattr("error", error.value(py).str()?)?;
                }
            } else {
                return Err(error);
            }
        }
        observer.setattr(
            "due",
            observer
                .getattr("due")?
                .extract::<f64>()?
                .min(now(py)? + 2.0),
        )?;
    }
    if observer.getattr("future")?.is_none()
        && manager.getattr("library")?.is_truthy()?
        && !manager.getattr("busy")?.is_truthy()?
        && !manager.getattr("runtime_active")?.is_truthy()?
        && now(py)? >= observer.getattr("due")?.extract::<f64>()?
    {
        observer.setattr("library", manager.getattr("library")?)?;
        observer.setattr("cancel", py.import("threading")?.getattr("Event")?.call0()?)?;
        observer.setattr("running_generation", observer.getattr("generation")?)?;
        let previous = observer.getattr("index")?;
        let (targets, cursor, visited) = if previous.is_truthy()? {
            (
                previous.getattr("targets")?,
                previous.getattr("cursor")?.extract::<usize>()?,
                previous.getattr("visited")?.extract::<usize>()?,
            )
        } else {
            (py.None().into_bound(py), 0, 0)
        };
        let future = observer.getattr("executor")?.call_method1(
            "submit",
            (
                module(py)?.getattr("scan")?,
                observer.getattr("library")?,
                targets,
                cursor,
                visited,
                observer.getattr("cancel")?,
                manager.getattr("selected_id")?,
            ),
        )?;
        observer.setattr("future", future)?;
        observer.setattr("due", f64::INFINITY)?;
    }
    Ok(())
}
#[pyfunction]
fn observer_close(observer: &Bound<'_, PyAny>) -> PyResult<()> {
    observer.getattr("cancel")?.call_method0("set")?;
    let kwargs = PyDict::new(observer.py());
    kwargs.set_item("wait", true)?;
    kwargs.set_item("cancel_futures", true)?;
    observer
        .getattr("executor")?
        .call_method("shutdown", (), Some(&kwargs))?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(observer_targets, m)?)?;
    m.add_function(wrap_pyfunction!(observer_fingerprint, m)?)?;
    m.add_function(wrap_pyfunction!(observer_scan, m)?)?;
    m.add_function(wrap_pyfunction!(observer_dismiss, m)?)?;
    m.add_function(wrap_pyfunction!(observer_initialize, m)?)?;
    m.add_function(wrap_pyfunction!(observer_invalidate, m)?)?;
    m.add_function(wrap_pyfunction!(observer_poll, m)?)?;
    m.add_function(wrap_pyfunction!(observer_close, m)?)?;
    Ok(())
}
