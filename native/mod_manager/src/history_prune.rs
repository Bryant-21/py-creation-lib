use crate::{
    check_cancel,
    database::{Database, error, records},
    encoding,
    executables::text,
    invalid,
    python_io::{with_context, with_database},
    reject_links, relative_path,
    transactions::{manifest, transaction_key},
};
use pyo3::{prelude::*, types::PyDict};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub fn py_path<'py>(py: Python<'py>, path: &Path) -> PyResult<Bound<'py, PyAny>> {
    py.import("pathlib")?
        .getattr("Path")?
        .call1((path.to_path_buf(),))
}
fn root(library: &Bound<'_, PyAny>) -> PyResult<PathBuf> {
    library.getattr("root")?.extract()
}
fn paths(library: &Bound<'_, PyAny>, id: &str) -> PyResult<[PathBuf; 3]> {
    transaction_key(id)?;
    let root = root(library)?;
    let result = [
        root.join(".history").join(id).join("payload"),
        root.join(".history-prune").join(id).join("payload"),
        root.join(".history-prune").join(format!("{id}.json")),
    ];
    for path in &result {
        reject_links(path)?;
    }
    Ok(result)
}
pub fn snapshot(
    py: Python<'_>,
    path: &Path,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<(Value, Vec<String>)> {
    let files = manifest(
        py,
        path,
        Some(cancel),
        if progress.is_none() {
            None
        } else {
            Some(progress)
        },
    )?;
    let (package, directories) =
        crate::package::directory_inventory(py, path.to_path_buf(), Some(cancel))?;
    if files.as_object().unwrap().keys().collect::<BTreeSet<_>>()
        != package.files().values().collect::<BTreeSet<_>>()
    {
        return Err(invalid("Recovery backup changed while reviewing it."));
    }
    Ok((files, directories.into_iter().collect()))
}
fn operation(db: &Database, id: &str) -> PyResult<(Value, String)> {
    let record = records(
        &*db.connection()?,
        "SELECT * FROM operations WHERE id=?",
        [id],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| invalid("Recovery entry no longer exists."))?;
    let state = encoding::parse(text(&record, "snapshot"))
        .map_err(|_| invalid("Recovery metadata is malformed; preserve it for manual review."))?;
    if !state.is_object() {
        return Err(invalid(
            "Recovery metadata is malformed; preserve it for manual review.",
        ));
    }
    let item = &state["mod"];
    if !item.is_object()
        || item["kind"] != "owned"
        || transaction_key(text(item, "id"))? != transaction_key(text(&record, "mod_id"))?
        || !["reinstall", "uninstall", "restore"].contains(&text(&record, "kind"))
        || !item["name"].is_string()
    {
        return Err(invalid(
            "This entry is not a removable owned recovery payload.",
        ));
    }
    Ok((record, text(item, "name").into()))
}
fn review(
    library: &Bound<'_, PyAny>,
    db: &Database,
    id: &str,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<Value> {
    let (operation, name) = operation(db, id)?;
    let (files, dirs) = snapshot(library.py(), &paths(library, id)?[0], cancel, progress)?;
    let token = encoding::digest(&json!([operation, files, dirs]), true, false);
    let bytes: u64 = files
        .as_object()
        .unwrap()
        .values()
        .map(|v| v["size"].as_u64().unwrap())
        .sum();
    Ok(
        json!([{"operation_id":id,"mod_id":operation["mod_id"],"name":name,"kind":operation["kind"],"token":token,"files":files.as_object().unwrap().len(),"bytes":bytes},operation,files,dirs]),
    )
}
fn mkdir(path: &Bound<'_, PyAny>, existing: bool) -> PyResult<()> {
    let kwargs = PyDict::new(path.py());
    kwargs.set_item("exist_ok", existing)?;
    path.call_method("mkdir", (), Some(&kwargs))?;
    Ok(())
}
pub fn remove_empty(path: &Path) -> PyResult<()> {
    reject_links(path)?;
    if path.is_dir() && std::fs::read_dir(path)?.next().is_none() {
        std::fs::remove_dir(path)?;
    }
    Ok(())
}
pub fn write(py: Python<'_>, path: &Path, record: &Value, limit: usize) -> PyResult<()> {
    let encoded = encoding::encode(record, true, true);
    if encoded.len() > limit {
        return Err(invalid(
            "Recovery deletion receipt exceeds its 64 MiB safety limit.",
        ));
    }
    let parent = path.parent().unwrap();
    reject_links(parent)?;
    mkdir(&py_path(py, parent)?, true)?;
    let temporary = path.with_file_name(format!(
        "{}.{}.tmp",
        path.file_stem().unwrap().to_string_lossy(),
        uuid::Uuid::new_v4().simple()
    ));
    let temporary_py = py_path(py, &temporary)?;
    let result = (|| -> PyResult<()> {
        with_context(&temporary_py.call_method1("open", ("xb",))?, |stream| {
            stream.call_method1(
                "write",
                (pyo3::types::PyBytes::new(py, encoded.as_bytes()),),
            )?;
            stream.call_method0("flush")?;
            py.import("os")?
                .call_method1("fsync", (stream.call_method0("fileno")?,))?;
            Ok(())
        })?;
        #[cfg(windows)]
        temporary_py.call_method1("rename", (py_path(py, path)?,))?;
        #[cfg(not(windows))]
        {
            std::fs::hard_link(&temporary, path)?;
            std::fs::remove_file(&temporary)?;
        }
        Ok(())
    })();
    let kwargs = PyDict::new(py);
    kwargs.set_item("missing_ok", true)?;
    temporary_py.call_method("unlink", (), Some(&kwargs))?;
    result
}
fn record(library: &Bound<'_, PyAny>, path: &Path, limit: usize) -> PyResult<Value> {
    let result = (|| -> PyResult<Value> {
        let record = encoding::parse(&crate::paths::metadata_read_text(
            &py_path(library.py(), path)?,
            limit,
        )?)?;
        let fields: BTreeSet<_> = record
            .as_object()
            .ok_or_else(|| invalid("structure"))?
            .keys()
            .map(String::as_str)
            .collect();
        if fields
            != BTreeSet::from([
                "version",
                "library",
                "operation",
                "manifest",
                "directories",
                "token",
            ])
            || record["version"] != 1
            || text(&record, "library") != library.getattr("library_id")?.extract::<String>()?
        {
            return Err(invalid("identity or structure"));
        }
        let op = &record["operation"];
        if op
            .as_object()
            .ok_or_else(|| invalid("operation"))?
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != BTreeSet::from(["id", "kind", "mod_id", "snapshot", "created"])
        {
            return Err(invalid("operation structure"));
        }
        let id = transaction_key(text(op, "id"))?;
        let mod_id = transaction_key(text(op, "mod_id"))?;
        let state = encoding::parse(text(op, "snapshot"))?;
        if state["mod"]["kind"] != "owned"
            || transaction_key(text(&state["mod"], "id"))? != mod_id
            || !["reinstall", "uninstall", "restore"].contains(&text(op, "kind"))
            || paths(library, &id)?[2] != path
        {
            return Err(invalid("operation identity"));
        }
        let files = record["manifest"]
            .as_object()
            .filter(|v| v.len() <= 500_000)
            .ok_or_else(|| invalid("file manifest"))?;
        for (name, item) in files {
            let fields = item.as_object().ok_or_else(|| invalid("file manifest"))?;
            if relative_path(name, false)? != *name
                || fields.len() != 2
                || !fields.contains_key("size")
                || !fields.contains_key("sha256")
                || item["size"]
                    .as_u64()
                    .is_none_or(|v| v > 64 * 1024 * 1024 * 1024)
                || text(item, "sha256").len() != 64
                || !text(item, "sha256")
                    .bytes()
                    .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
            {
                return Err(invalid("file manifest"));
            }
        }
        let dirs = record["directories"]
            .as_array()
            .filter(|v| v.len() <= 1_000_000)
            .ok_or_else(|| invalid("directory manifest"))?;
        let mut seen = BTreeSet::new();
        for dir in dirs {
            let name = dir.as_str().ok_or_else(|| invalid("directory manifest"))?;
            if !seen.insert(name) || relative_path(name, false)? != name {
                return Err(invalid("directory manifest"));
            }
        }
        crate::paths::validate_destinations(files.keys().cloned().collect())?;
        if encoding::digest(&json!([op, files, dirs]), true, false) != text(&record, "token") {
            return Err(invalid("receipt digest"));
        }
        Ok(record)
    })();
    result.map_err(|_| invalid("Invalid recovery deletion receipt; preserve it for manual review."))
}
fn rollback(library: &Bound<'_, PyAny>, record: &Value) -> PyResult<()> {
    let [source, trash, journal] = paths(library, text(&record["operation"], "id"))?;
    if trash.exists() {
        if source.exists() || !trash.is_dir() {
            return Err(invalid(
                "Recovery deletion rollback has conflicting locations; both were preserved.",
            ));
        }
        mkdir(&py_path(library.py(), source.parent().unwrap())?, true)?;
        py_path(library.py(), &trash)?.call_method1("rename", (source,))?;
    } else if !source.is_dir() {
        return Err(invalid(
            "Recovery deletion rollback has no intact payload; preserve the receipt.",
        ));
    }
    std::fs::remove_file(journal)?;
    remove_empty(trash.parent().unwrap())
}
pub fn stamp(info: &Bound<'_, PyAny>) -> PyResult<(u64, u64, u64, i64)> {
    Ok((
        info.getattr("st_dev")?.extract()?,
        info.getattr("st_ino")?.extract()?,
        info.getattr("st_size")?.extract()?,
        info.getattr("st_mtime_ns")?.extract()?,
    ))
}
fn purge(
    library: &Bound<'_, PyAny>,
    record: &Value,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<()> {
    purge_at(
        library,
        record,
        checkpoint,
        paths(library, text(&record["operation"], "id"))?,
    )
}

pub fn purge_at(
    library: &Bound<'_, PyAny>,
    record: &Value,
    checkpoint: &Bound<'_, PyAny>,
    [source, trash, journal]: [PathBuf; 3],
) -> PyResult<()> {
    let py = library.py();
    if source.exists() {
        return Err(invalid(
            "Committed recovery deletion has an unexpected original payload; preserve both locations.",
        ));
    }
    if trash.exists() {
        let (files, mut dirs) = snapshot(py, &trash, py.None().bind(py), py.None().bind(py))?;
        let approved = record["directories"].as_array().unwrap();
        if dirs.iter().any(|v| !approved.contains(&json!(v)))
            || files
                .as_object()
                .unwrap()
                .iter()
                .any(|(name, value)| record["manifest"].get(name) != Some(value))
        {
            return Err(invalid(
                "Pending recovery cleanup contains changed or unexpected content. It was preserved for manual review.",
            ));
        }
        let mut names: Vec<_> = files.as_object().unwrap().keys().collect();
        names.sort();
        for name in names {
            let path = trash.join(name);
            reject_links(&path)?;
            let path_py = py_path(py, &path)?;
            let (before, after, checksum) =
                with_context(&path_py.call_method1("open", ("rb",))?, |stream| {
                    let os = py.import("os")?;
                    let fd = stream.call_method0("fileno")?;
                    let before = stamp(&os.call_method1("fstat", (&fd,))?)?;
                    let mut digest = Sha256::new();
                    loop {
                        let block = stream
                            .call_method1("read", (1024 * 1024,))?
                            .extract::<Vec<u8>>()?;
                        if block.is_empty() {
                            break;
                        }
                        digest.update(block);
                    }
                    Ok((
                        before,
                        stamp(&os.call_method1("fstat", (&fd,))?)?,
                        format!("{:x}", digest.finalize()),
                    ))
                })?;
            reject_links(&path)?;
            let current = stamp(&path_py.call_method0("stat")?)?;
            if before != after
                || current != before
                || current.2 != files[name]["size"].as_u64().unwrap()
                || checksum != text(&files[name], "sha256")
            {
                return Err(invalid(
                    "Pending recovery file changed during cleanup. It was preserved.",
                ));
            }
            path_py.call_method0("unlink")?;
            checkpoint.call1(("file-removed",))?;
        }
        dirs.sort_by(|a, b| (b.matches('/').count(), b).cmp(&(a.matches('/').count(), a)));
        for dir in dirs {
            let path = trash.join(dir);
            reject_links(&path)?;
            std::fs::remove_dir(path)?;
        }
        reject_links(&trash)?;
        std::fs::remove_dir(&trash)?;
    }
    remove_empty(trash.parent().unwrap())?;
    remove_empty(source.parent().unwrap())?;
    std::fs::remove_file(journal)?;
    Ok(())
}
fn recover_locked(
    library: &Bound<'_, PyAny>,
    limit: usize,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<Vec<String>> {
    let base = root(library)?.join(".history-prune");
    reject_links(&base)?;
    if !base.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = std::fs::read_dir(base)?
        .map(|v| v.map(|v| v.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let mut count = 0;
    let mut messages = Vec::new();
    for path in paths {
        reject_links(&path)?;
        if path.extension().is_none_or(|v| v != "json") {
            continue;
        }
        count += 1;
        if count > 100 {
            return Err(invalid(
                "Recovery deletion exceeds its 100-receipt recovery limit.",
            ));
        }
        let record = record(library, &path, limit)?;
        let row = with_database(&library.getattr("_db")?, |_, db| {
            Ok(records(
                &*db.connection()?,
                "SELECT * FROM operations WHERE id=?",
                [text(&record["operation"], "id")],
            )?
            .into_iter()
            .next())
        })?;
        if let Some(row) = row {
            if row != record["operation"] {
                return Err(invalid(
                    "Recovery deletion metadata changed; preserve both locations for manual review.",
                ));
            }
            rollback(library, &record)?;
            messages.push("Restored an interrupted, uncommitted recovery-backup deletion.".into());
        } else {
            purge(library, &record, checkpoint)?;
            messages.push("Finished a committed deletion of reviewed recovery content.".into());
        }
    }
    Ok(messages)
}

#[pyfunction]
fn history_prune_paths(library: &Bound<'_, PyAny>, id: &str) -> PyResult<Vec<PathBuf>> {
    Ok(paths(library, id)?.to_vec())
}
#[pyfunction]
fn history_prune_write(py: Python<'_>, path: PathBuf, record: &str, limit: usize) -> PyResult<()> {
    write(py, &path, &encoding::parse(record)?, limit)
}
#[pyfunction]
fn history_prune_review(
    library: &Bound<'_, PyAny>,
    db: &Database,
    id: &str,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<String> {
    Ok(review(library, db, id, cancel, progress)?.to_string())
}
#[pyfunction]
fn history_prune_operation(
    library: &Bound<'_, PyAny>,
    action: &str,
    id: &str,
    plan: &str,
    limit: usize,
    checkpoint: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<String> {
    if action == "recover_locked" {
        return Ok(json!(recover_locked(library, limit, checkpoint)?).to_string());
    }
    library.call_method0("_runtime_write_guard")?;
    let owned = library.getattr("owned")?;
    with_context(&owned.call_method0("lock")?, |_| {
        if action == "recover" {
            return Ok(json!(recover_locked(library, limit, checkpoint)?).to_string());
        }
        if action == "remove" {
            library.call_method0("_runtime_write_guard")?;
        }
        owned.call_method0("_recover_locked")?;
        recover_locked(library, limit, checkpoint)?;
        with_database(&library.getattr("_db")?, |_, db| {
            if action == "review" {
                return Ok(
                    review(library, db, &transaction_key(id)?, cancel, progress)?[0].to_string(),
                );
            }
            let plan = encoding::parse(plan)?;
            let id = transaction_key(text(&plan, "operation_id"))?;
            db.connection()?
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(error)?;
            let current = review(library, db, &id, cancel, progress)?;
            if current[0] != plan {
                return Err(invalid(
                    "Recovery entry or payload changed after review. Review deletion again.",
                ));
            }
            let [source, trash, journal] = paths(library, &id)?;
            if trash.parent().unwrap().exists() || journal.exists() {
                return Err(invalid(
                    "Recovery deletion destination is already occupied.",
                ));
            }
            let record = json!({"version":1,"library":library.getattr("library_id")?.extract::<String>()?,"operation":current[1],"manifest":current[2],"directories":current[3],"token":plan["token"]});
            check_cancel(Some(cancel))?;
            let result = (|| -> PyResult<()> {
                write(library.py(), &journal, &record, limit)?;
                checkpoint.call1(("receipt",))?;
                check_cancel(Some(cancel))?;
                mkdir(&py_path(library.py(), trash.parent().unwrap())?, false)?;
                py_path(library.py(), &source)?.call_method1("rename", (&trash,))?;
                checkpoint.call1(("moved",))?;
                check_cancel(Some(cancel))?;
                db.connection()?
                    .execute("DELETE FROM operations WHERE id=?", [&id])
                    .map_err(error)?;
                checkpoint.call1(("metadata",))?;
                check_cancel(Some(cancel))?;
                library.call_method0("_runtime_write_guard")?;
                db.connection()?.execute_batch("COMMIT").map_err(error)?;
                checkpoint.call1(("committed",))?;
                purge(library, &record, checkpoint)
            })();
            if let Err(original) = result {
                {
                    let connection = db.connection()?;
                    if !connection.is_autocommit() {
                        connection.execute_batch("ROLLBACK").map_err(error)?;
                    }
                }
                let committed = with_database(&library.getattr("_db")?, |_, check| {
                    Ok(records(
                        &*check.connection()?,
                        "SELECT 1 FROM operations WHERE id=?",
                        [&id],
                    )?
                    .is_empty())
                })?;
                if committed {
                    let py = library.py();
                    let exception = py
                        .import("creation_lib.mod_manager.transactions")?
                        .getattr("CommittedTransaction")?
                        .call1((py.import("json")?.call_method1(
                            "loads",
                            (json!({"mod_id":plan["mod_id"],"kind":"prune"}).to_string(),),
                        )?,))?;
                    exception.setattr("args",("Recovery deletion committed, but cleanup is pending. Reopen this library; changed or unexpected content will be preserved for manual review.",))?;
                    return Err(PyErr::from_value(exception));
                }
                if journal.exists() {
                    rollback(library, &record)?;
                }
                return Err(original);
            }
            Ok(json!(id).to_string())
        })
    })
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(history_prune_paths, m)?)?;
    m.add_function(wrap_pyfunction!(history_prune_write, m)?)?;
    m.add_function(wrap_pyfunction!(history_prune_review, m)?)?;
    m.add_function(wrap_pyfunction!(history_prune_operation, m)?)?;
    Ok(())
}
