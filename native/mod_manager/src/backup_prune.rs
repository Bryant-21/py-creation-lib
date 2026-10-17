use crate::{
    check_cancel,
    database::{error, records},
    encoding,
    executables::text,
    history_prune::{purge_at, py_path, remove_empty, snapshot, stamp, write},
    invalid,
    python_io::{with_context, with_database},
    reject_links, relative_path,
    transactions::transaction_key,
};
use pyo3::{prelude::*, types::PyDict};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

fn root(library: &Bound<'_, PyAny>) -> PyResult<PathBuf> {
    library.getattr("root")?.extract()
}
fn source(library: &Bound<'_, PyAny>, kind: &str, profile: &str, id: &str) -> PyResult<PathBuf> {
    if !["save", "overwrite"].contains(&kind) {
        return Err(invalid("Choose an owned save or overwrite backup."));
    }
    let path = root(library)?
        .join(if kind == "save" {
            ".save-history"
        } else {
            ".overwrite-history"
        })
        .join(transaction_key(profile)?)
        .join(transaction_key(id)?);
    reject_links(&path)?;
    Ok(path)
}
fn paths(library: &Bound<'_, PyAny>, record: &Value) -> PyResult<[PathBuf; 3]> {
    let id = transaction_key(text(record, "id"))?;
    let source = source(
        library,
        text(record, "kind"),
        text(record, "profile"),
        text(record, "backup"),
    )?;
    let trash = root(library)?
        .join(".backup-prune")
        .join(&id)
        .join("bundle");
    let receipt = root(library)?
        .join(".backup-prune")
        .join(format!("{id}.json"));
    reject_links(&trash)?;
    reject_links(&receipt)?;
    Ok([source, trash, receipt])
}
fn review(
    library: &Bound<'_, PyAny>,
    kind: &str,
    profile: &str,
    id: &str,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<Value> {
    let py = library.py();
    let source = source(library, kind, profile, id)?;
    if !source.is_dir() {
        return Err(invalid("This retained backup no longer exists."));
    }
    let before = stamp(&py_path(py, &source)?.call_method0("stat")?)?;
    let backup = if kind == "save" {
        Some(
            py.import("creation_lib.mod_manager.save_backups")?
                .call_method1("_read", (library, profile, id))?,
        )
    } else {
        None
    };
    let name = if let Some(backup) = &backup {
        backup.getattr("names")?.get_item(0)?.extract::<String>()?
    } else {
        let overwrite = py.import("creation_lib.mod_manager.overwrite")?;
        let mut found = None;
        for item in overwrite
            .call_method1("history", (library, profile))?
            .try_iter()?
        {
            let item = item?;
            if item.get_item("id")?.extract::<String>()? == id {
                found = Some(item);
                break;
            }
        }
        let record = found
            .ok_or_else(|| invalid("Choose a completed or restored owned overwrite backup."))?;
        let payload = overwrite
            .call_method1("_paths", (library, &record))?
            .get_item(1)?;
        let complete = record.get_item("state")?.extract::<String>()? == "complete";
        if complete {
            let review = overwrite.getattr("Review")?.call1((
                profile,
                record.get_item("token")?,
                record.get_item("files")?,
                record.get_item("bytes")?,
                pyo3::types::PyTuple::empty(py),
            ))?;
            overwrite.call_method1("_verify", (library, &payload, review, cancel, progress))?;
        } else if payload.call_method0("exists")?.is_truthy()? {
            return Err(invalid(
                "A restored overwrite receipt has unexpected retained payload; preserve it for review.",
            ));
        }
        let allowed = if complete {
            BTreeSet::from(["receipt.json".to_string(), "payload".into()])
        } else {
            BTreeSet::from(["receipt.json".to_string()])
        };
        let present = std::fs::read_dir(&source)?
            .map(|v| v.map(|v| v.file_name().to_string_lossy().into_owned()))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if present != allowed {
            return Err(invalid(
                "Overwrite backup has unexpected bundle members; preserve them for review.",
            ));
        }
        format!(
            "Overwrite {}",
            record.get_item("created")?.extract::<String>()?
        )
    };
    let (files, dirs) = snapshot(py, &source, cancel, progress)?;
    if let Some(backup) = backup {
        let names = backup.getattr("names")?.extract::<Vec<String>>()?;
        let sizes = backup.getattr("sizes")?.extract::<Vec<u64>>()?;
        let checksums = backup.getattr("checksums")?.extract::<Vec<String>>()?;
        if names.len() != sizes.len() || names.len() != checksums.len() {
            return Err(invalid("Invalid owned save receipt."));
        }
        for (index, name) in names.iter().enumerate() {
            if files[name] != json!({"size":sizes[index],"sha256":checksums[index]}) {
                return Err(invalid(
                    "Save backup bytes no longer match its owned receipt.",
                ));
            }
        }
        if !py
            .import("creation_lib.mod_manager.save_backups")?
            .call_method1("_read", (library, profile, id))?
            .eq(&backup)?
        {
            return Err(invalid(
                "Save backup receipt changed while reviewing deletion.",
            ));
        }
    }
    reject_links(&source)?;
    let after = stamp(&py_path(py, &source)?.call_method0("stat")?)?;
    let identity = json!([before.0, before.1]);
    if (before.0, before.1) != (after.0, after.1) {
        return Err(invalid(
            "Retained backup identity changed while reviewing it.",
        ));
    }
    let token = encoding::digest(
        &json!([
            library.getattr("library_id")?.extract::<String>()?,
            kind,
            profile,
            id,
            files,
            dirs,
            identity
        ]),
        true,
        false,
    );
    let bytes: u64 = files
        .as_object()
        .unwrap()
        .values()
        .map(|v| v["size"].as_u64().unwrap())
        .sum();
    Ok(
        json!([{"kind":kind,"profile_id":profile,"backup_id":id,"name":name,"token":token,"files":files.as_object().unwrap().len(),"bytes":bytes},files,dirs,identity]),
    )
}
fn record(library: &Bound<'_, PyAny>, path: &Path) -> PyResult<Value> {
    let result = (|| -> PyResult<Value> {
        let value = encoding::parse(&crate::paths::metadata_read_text(
            &py_path(library.py(), path)?,
            64 * 1024 * 1024,
        )?)?;
        if value
            .as_object()
            .ok_or_else(|| invalid("structure"))?
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != BTreeSet::from([
                "version",
                "library",
                "id",
                "kind",
                "profile",
                "backup",
                "manifest",
                "directories",
                "identity",
                "token",
            ])
            || value["version"] != 1
            || text(&value, "library") != library.getattr("library_id")?.extract::<String>()?
            || paths(library, &value)?[2] != path
        {
            return Err(invalid("identity or structure"));
        }
        let files = value["manifest"]
            .as_object()
            .filter(|v| v.len() <= 500_000)
            .ok_or_else(|| invalid("manifest"))?;
        let dirs = value["directories"]
            .as_array()
            .filter(|v| v.len() <= 1_000_000)
            .ok_or_else(|| invalid("directories"))?;
        let identity = value["identity"]
            .as_array()
            .filter(|v| v.len() == 2 && v.iter().all(|n| n.as_u64().is_some()))
            .ok_or_else(|| invalid("identity"))?;
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
        let mut seen = BTreeSet::new();
        for dir in dirs {
            let name = dir.as_str().ok_or_else(|| invalid("directory"))?;
            if !seen.insert(name) || relative_path(name, false)? != name {
                return Err(invalid("directory manifest"));
            }
        }
        crate::paths::validate_destinations(files.keys().cloned().collect())?;
        if encoding::digest(
            &json!([
                value["library"],
                value["kind"],
                value["profile"],
                value["backup"],
                files,
                dirs,
                identity
            ]),
            true,
            false,
        ) != text(&value, "token")
        {
            return Err(invalid("receipt digest"));
        }
        Ok(value)
    })();
    result.map_err(|_| {
        invalid("Invalid retained-backup deletion receipt; preserve it for manual review.")
    })
}
fn rollback(library: &Bound<'_, PyAny>, record: &Value) -> PyResult<()> {
    let [source, trash, receipt] = paths(library, record)?;
    if trash.exists() {
        if source.exists() || !trash.is_dir() {
            return Err(invalid(
                "Backup deletion rollback has conflicting locations; both were preserved.",
            ));
        }
        std::fs::create_dir_all(source.parent().unwrap())?;
        py_path(library.py(), &trash)?.call_method1("rename", (source,))?;
    } else if !source.is_dir() {
        return Err(invalid(
            "Backup deletion rollback has no intact bundle; preserve the receipt.",
        ));
    }
    std::fs::remove_file(receipt)?;
    remove_empty(trash.parent().unwrap())
}
fn purge(
    library: &Bound<'_, PyAny>,
    record: &Value,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let locations = paths(library, record)?;
    if locations[0].exists() {
        return Err(invalid(
            "Committed backup deletion has an unexpected original bundle; preserve both locations.",
        ));
    }
    if locations[1].exists() {
        let info = stamp(&py_path(library.py(), &locations[1])?.call_method0("stat")?)?;
        if json!([info.0, info.1]) != record["identity"] {
            return Err(invalid(
                "Pending backup cleanup bundle identity changed; preserve it for review.",
            ));
        }
    }
    purge_at(library, record, checkpoint, locations)?;
    checkpoint.call1(("receipt-removed",))?;
    with_database(&library.getattr("_db")?, |_, db| {
        db.connection()?
            .execute(
                "DELETE FROM backup_prunes WHERE id=? AND token=?",
                [text(record, "id"), text(record, "token")],
            )
            .map_err(error)?;
        Ok(())
    })
}
fn recover_locked(
    library: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<Vec<String>> {
    let base = root(library)?.join(".backup-prune");
    reject_links(&base)?;
    if !base.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries = std::fs::read_dir(&base)?
        .map(|v| v.map(|v| v.path()))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    let mut count = 0;
    let mut messages = Vec::new();
    for path in entries {
        reject_links(&path)?;
        if path.extension().is_none_or(|v| v != "json") {
            continue;
        }
        count += 1;
        if count > 100 {
            return Err(invalid(
                "Retained-backup cleanup exceeds its 100-receipt recovery limit.",
            ));
        }
        let record = record(library, &path)?;
        let marker = with_database(&library.getattr("_db")?, |_, db| {
            Ok(records(
                &*db.connection()?,
                "SELECT token FROM backup_prunes WHERE id=?",
                [text(&record, "id")],
            )?
            .into_iter()
            .next())
        })?;
        if let Some(marker) = marker {
            if marker["token"] != record["token"] {
                return Err(invalid(
                    "Backup deletion commit marker differs from its receipt; preserve it for review.",
                ));
            }
            purge(library, &record, checkpoint)?;
            messages.push("Finished a committed retained-backup deletion.".into());
        } else {
            rollback(library, &record)?;
            messages.push("Restored an uncommitted retained-backup deletion.".into());
        }
    }
    with_database(&library.getattr("_db")?, |_, db| {
        let markers = records(&*db.connection()?, "SELECT id FROM backup_prunes", [])?;
        for marker in markers {
            let id = transaction_key(text(&marker, "id"))?;
            if !base.join(format!("{id}.json")).exists() && !base.join(&id).exists() {
                db.connection()?
                    .execute("DELETE FROM backup_prunes WHERE id=?", [id])
                    .map_err(error)?;
            }
        }
        Ok(())
    })?;
    Ok(messages)
}
#[pyfunction]
fn backup_prune_operation(
    library: &Bound<'_, PyAny>,
    action: &str,
    kind: &str,
    profile: &str,
    id: &str,
    plan: &str,
    checkpoint: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<String> {
    if action == "recover_locked" {
        return Ok(json!(recover_locked(library, checkpoint)?).to_string());
    }
    library.call_method0("_runtime_write_guard")?;
    with_context(&library.getattr("owned")?.call_method0("lock")?, |_| {
        let messages = recover_locked(library, checkpoint)?;
        if action == "recover" {
            return Ok(json!(messages).to_string());
        }
        if action == "review" {
            return Ok(review(
                library,
                kind,
                &transaction_key(profile)?,
                &transaction_key(id)?,
                cancel,
                progress,
            )?[0]
                .to_string());
        }
        let plan = encoding::parse(plan)?;
        let current = review(
            library,
            text(&plan, "kind"),
            text(&plan, "profile_id"),
            text(&plan, "backup_id"),
            cancel,
            progress,
        )?;
        if current[0] != plan {
            return Err(invalid(
                "Retained backup changed after deletion review. Review it again.",
            ));
        }
        let record = json!({"version":1,"library":library.getattr("library_id")?.extract::<String>()?,"id":uuid::Uuid::new_v4().simple().to_string(),"kind":plan["kind"],"profile":plan["profile_id"],"backup":plan["backup_id"],"manifest":current[1],"directories":current[2],"identity":current[3],"token":plan["token"]});
        let [source, trash, receipt] = paths(library, &record)?;
        let mut did_commit = false;
        with_database(&library.getattr("_db")?, |_, db| {
            db.connection()?
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(error)?;
            let result = (|| -> PyResult<()> {
                check_cancel(Some(cancel))?;
                write(library.py(), &receipt, &record, 64 * 1024 * 1024)?;
                checkpoint.call1(("receipt",))?;
                check_cancel(Some(cancel))?;
                std::fs::create_dir(trash.parent().unwrap())?;
                py_path(library.py(), &source)?.call_method1("rename", (&trash,))?;
                checkpoint.call1(("moved",))?;
                check_cancel(Some(cancel))?;
                let (files, dirs) = snapshot(library.py(), &trash, cancel, progress)?;
                if files != current[1] || json!(dirs) != current[2] {
                    return Err(invalid(
                        "Retained backup changed while staging deletion. Its complete bundle was preserved.",
                    ));
                }
                db.connection()?
                    .execute(
                        "INSERT INTO backup_prunes(id,token) VALUES (?,?)",
                        [text(&record, "id"), text(&record, "token")],
                    )
                    .map_err(error)?;
                checkpoint.call1(("metadata",))?;
                check_cancel(Some(cancel))?;
                library.call_method0("_runtime_write_guard")?;
                db.connection()?.execute_batch("COMMIT").map_err(error)?;
                did_commit = true;
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
                    Ok(!records(
                        &*check.connection()?,
                        "SELECT token FROM backup_prunes WHERE id=?",
                        [text(&record, "id")],
                    )?
                    .is_empty())
                })?;
                if did_commit || committed {
                    let py = library.py();
                    let fields = PyDict::new(py);
                    fields.set_item("mod_id", "")?;
                    fields.set_item("kind", "backup-prune")?;
                    let exception = py
                        .import("creation_lib.mod_manager.transactions")?
                        .getattr("CommittedTransaction")?
                        .call1((fields,))?;
                    exception.setattr("args",("Retained-backup deletion committed; cleanup is pending. Reopen the library. Changed or unexpected content will be preserved.",))?;
                    return Err(PyErr::from_value(exception));
                }
                if receipt.exists() {
                    rollback(library, &record)?;
                }
                return Err(original);
            }
            Ok(())
        })?;
        Ok(plan["backup_id"].to_string())
    })
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(backup_prune_operation, m)?)?;
    Ok(())
}
