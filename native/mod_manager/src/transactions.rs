use crate::python_io::with_database;
use crate::{
    check_cancel,
    database::{Database, records},
    encoding,
    executables::text,
    invalid, reject_links,
};
use pyo3::prelude::*;
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use unicode_casefold::UnicodeCaseFold;

pyo3::create_exception!(
    mod_manager_native,
    TransactionCommitted,
    pyo3::exceptions::PyValueError
);

fn verify(
    py: Python<'_>,
    db: &Database,
    root: &Path,
    review: &Value,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<Value> {
    let result = encoding::parse(&transaction_review(
        py,
        db,
        root.to_path_buf(),
        text(review, "mod_id"),
        cancel,
    )?)?;
    if result[0]["token"] != review["token"] {
        return Err(invalid(
            "Owned files, metadata, or profile references changed after review. Review this operation again.",
        ));
    }
    Ok(result[1].clone())
}

fn record_operation(db: &Database, record: &Value, snapshot: &Value) -> PyResult<()> {
    db.connection()?
        .execute(
            "INSERT INTO operations(id,kind,mod_id,snapshot) VALUES (?,?,?,?)",
            params![
                text(record, "id"),
                text(record, "kind"),
                text(record, "mod_id"),
                snapshot.to_string()
            ],
        )
        .map_err(crate::database::error)?;
    Ok(())
}

fn abort(factory: &Bound<'_, PyAny>, root: &Path, library: &str, record: &Value) -> PyResult<bool> {
    with_database(factory, |_, db| {
        let committed = !records(
            &*db.connection()?,
            "SELECT id FROM operations WHERE id=?",
            [text(record, "id")],
        )?
        .is_empty();
        if !committed {
            rollback(db, root, library, record)?;
        }
        Ok(committed)
    })
}

fn restore_entries(db: &Database, entries: &Value, mod_id: &str) -> PyResult<()> {
    let connection = db.connection()?;
    for entry in entries
        .as_array()
        .ok_or_else(|| invalid("Invalid recovery profile entries."))?
    {
        let profile = text(entry, "profile_id");
        if records(&connection, "SELECT id FROM profiles WHERE id=?", [profile])?.is_empty() {
            continue;
        }
        let mut order: Vec<_> = records(
            &connection,
            "SELECT mod_id FROM entries WHERE profile_id=? ORDER BY priority,mod_id",
            [profile],
        )?
        .into_iter()
        .map(|v| text(&v, "mod_id").to_string())
        .collect();
        let priority = entry["priority"]
            .as_i64()
            .unwrap_or(0)
            .max(0)
            .min(order.len() as i64) as usize;
        order.insert(priority, mod_id.into());
        connection
            .execute(
                "INSERT INTO entries VALUES (?,?,?,?,?)",
                params![
                    profile,
                    mod_id,
                    priority,
                    entry["enabled"].as_i64().unwrap_or(0),
                    text(entry, "group_name")
                ],
            )
            .map_err(crate::database::error)?;
        for (index, id) in order.iter().enumerate() {
            connection
                .execute(
                    "UPDATE entries SET priority=? WHERE profile_id=? AND mod_id=?",
                    params![index, profile, id],
                )
                .map_err(crate::database::error)?;
        }
    }
    Ok(())
}

fn restore_links(db: &Database, snapshot: &Value) -> PyResult<()> {
    let Some(links) = snapshot.get("overlays") else {
        return Ok(());
    };
    let keys: BTreeSet<_> = ["overlay_id", "profile_id", "target_id", "base_id", "source"]
        .into_iter()
        .collect();
    for link in links
        .as_array()
        .ok_or_else(|| invalid("Recovery additive-update association is invalid."))?
    {
        if link
            .as_object()
            .map(|v| v.keys().map(String::as_str).collect::<BTreeSet<_>>())
            != Some(keys.clone())
            || link["overlay_id"] != snapshot["mod"]["id"]
        {
            return Err(invalid("Recovery additive-update association is invalid."));
        }
        db.connection()?
            .execute(
                "INSERT OR REPLACE INTO reference_overlays VALUES (?,?,?,?,?)",
                params![
                    text(link, "overlay_id"),
                    text(link, "profile_id"),
                    text(link, "target_id"),
                    text(link, "base_id"),
                    text(link, "source")
                ],
            )
            .map_err(crate::database::error)?;
    }
    Ok(())
}

#[pyfunction]
#[pyo3(signature = (library, checkpoint, package, options, cancel=None, progress=None, validate_before_commit=None, attach_metadata=None))]
fn transaction_install(
    py: Python<'_>,
    library: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
    package: &Bound<'_, PyAny>,
    options: &str,
    cancel: Option<&Bound<'_, PyAny>>,
    progress: Option<&Bound<'_, PyAny>>,
    validate_before_commit: Option<&Bound<'_, PyAny>>,
    attach_metadata: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let options = encoding::parse(options)?;
    let name = text(&options, "name").trim();
    let plan = options["plan"]
        .as_object()
        .ok_or_else(|| invalid("Invalid install plan."))?;
    let review = options.get("review").filter(|v| !v.is_null());
    let restored = options.get("restored").filter(|v| !v.is_null());
    let empty_mod = options["empty_mod"].as_bool().unwrap_or(false);
    if name.is_empty() || (plan.is_empty() && restored.is_none() && !empty_mod) {
        return Err(invalid("Install needs a name and at least one file."));
    }
    crate::paths::validate_destinations(plan.keys().cloned().collect())?;
    let mod_id = if let Some(review) = review {
        transaction_key(text(review, "mod_id"))?
    } else if let Some(restored) = restored {
        transaction_key(text(&restored["mod"], "id"))?
    } else {
        uuid::Uuid::new_v4().simple().to_string()
    };
    let kind = if restored.is_some() {
        "restore"
    } else if review.is_some() {
        "reinstall"
    } else {
        "install"
    };
    let root: PathBuf = library.getattr("root")?.extract()?;
    let library_id: String = library.getattr("library_id")?.extract()?;
    let factory = library.getattr("_db")?;
    with_database(&factory, |_, db| {
        transaction_recover(db, root.clone(), &library_id)
    })?;
    let destination = root.join("mods").join(&mod_id);
    reject_links(&destination)?;
    if review.is_none() && destination.exists() {
        return Err(invalid(
            "Owned destination is occupied; no content was overwritten.",
        ));
    }
    if review.is_some() {
        with_database(&factory, |_, db| snapshot(db, &root, &mod_id))?;
    }
    let record = encoding::parse(&transaction_begin(
        root.clone(),
        &library_id,
        kind,
        &mod_id,
        review.is_some(),
        None,
    )?)?;
    checkpoint.call1(("journal",))?;
    let [_, stage, destination, backup] = paths(&root, &library_id, &record)?;
    let mut committed = false;
    let result = (|| -> PyResult<String> {
        std::fs::create_dir(&stage)?;
        let sources: Vec<_> = plan
            .iter()
            .map(|(target, source)| {
                (
                    target.clone(),
                    package.clone().unbind(),
                    source.as_str().unwrap_or("").to_string(),
                )
            })
            .collect();
        let copied =
            library.call_method1("_copy_plan", (sources, stage.clone(), cancel, progress))?;
        let manifest: Value = encoding::parse(
            &py.import("json")?
                .call_method1("dumps", (copied,))?
                .extract::<String>()?,
        )?;
        let mut directories: Vec<_> = options["restore_directories"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| v.as_str().unwrap_or(""))
            .collect();
        directories.sort_by_key(|v| (v.matches('/').count(), *v));
        for directory in directories {
            check_cancel(cancel)?;
            let path = stage.join(crate::relative_path(directory, false)?);
            reject_links(&path)?;
            std::fs::create_dir_all(path)?;
        }
        if let Some(token) = options["expected_manifest_token"].as_str() {
            if encoding::digest(&manifest, true, false) != token {
                return Err(invalid(
                    "Recovery backup changed while copying; review restoration again.",
                ));
            }
        }
        checkpoint.call1(("prepared",))?;
        check_cancel(cancel)?;
        with_database(&factory, |wrapper, db| {
            db.connection()?
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(crate::database::error)?;
            if let Some(validate) = validate_before_commit {
                validate.call0()?;
            }
            let old = if let Some(review) = review {
                verify(py, db, &root, review, cancel)?
            } else {
                json!({"mod":null,"entries":[]})
            };
            let profile = text(&options, "profile_id");
            if restored.is_none()
                && review.is_none()
                && records(
                    &*db.connection()?,
                    "SELECT id FROM profiles WHERE id=?",
                    [profile],
                )?
                .is_empty()
            {
                return Err(invalid(
                    "Choose an existing target profile before installing.",
                ));
            }
            if review.is_none()
                && records(
                    &*db.connection()?,
                    "SELECT name FROM mods WHERE kind='owned'",
                    [],
                )?
                .iter()
                .any(|row| text(row, "name").case_fold().eq(name.case_fold()))
            {
                return Err(invalid(
                    "An installed mod already has this name. Review a reinstall instead; replacement is not automatic.",
                ));
            }
            if restored.is_some()
                && review.is_none()
                && !records(
                    &*db.connection()?,
                    "SELECT id FROM mods WHERE id=?",
                    [&mod_id],
                )?
                .is_empty()
            {
                return Err(invalid(
                    "This mod already exists; restoration cannot overwrite it without review.",
                ));
            }
            check_cancel(cancel)?;
            if let Some(review) = review {
                std::fs::create_dir(backup.parent().unwrap())?;
                std::fs::rename(&destination, &backup)?;
                checkpoint.call1(("old-moved",))?;
                if encoding::digest(
                    &json!({"snapshot":old,"files":manifest_backup(py,&backup,cancel)?}),
                    true,
                    false,
                ) != text(review, "token")
                {
                    return Err(invalid(
                        "Owned content changed during replacement; the original content will be restored.",
                    ));
                }
            }
            std::fs::rename(&stage, &destination)?;
            checkpoint.call1(("placed",))?;
            let origin = &options["nexus"];
            if review.is_some() {
                db.connection()?.execute("UPDATE mods SET manifest=?,version=?,nexus_game=?,nexus_mod_id=?,nexus_file_id=? WHERE id=?",params![manifest.to_string(),text(origin,"version"),text(origin,"game"),origin["mod_id"].as_i64().unwrap_or(0),origin["file_id"].as_i64().unwrap_or(0),mod_id]).map_err(crate::database::error)?;
            } else if let Some(restored) = restored {
                let mut mod_record = restored["mod"].clone();
                mod_record["source"] = json!(destination.to_string_lossy());
                mod_record["manifest"] = json!(manifest.to_string());
                insert_restored(db, &mod_record)?;
                restore_entries(db, &restored["entries"], &mod_id)?;
                restore_links(db, restored)?;
            } else {
                let priority: i64 = db
                    .connection()?
                    .query_row(
                        "SELECT COALESCE(MAX(priority),-1)+1 FROM entries WHERE profile_id=?",
                        [profile],
                        |r| r.get(0),
                    )
                    .map_err(crate::database::error)?;
                db.connection()?.execute("INSERT INTO mods(id,name,source,kind,version,nexus_game,nexus_mod_id,nexus_file_id,manifest) VALUES (?,?,?,'owned',?,?,?,?,?)",params![mod_id,name,destination.to_string_lossy(),text(origin,"version"),text(origin,"game"),origin["mod_id"].as_i64().unwrap_or(0),origin["file_id"].as_i64().unwrap_or(0),manifest.to_string()]).map_err(crate::database::error)?;
                db.connection()?
                    .execute(
                        "INSERT INTO entries VALUES (?,?,?,?, '')",
                        params![
                            profile,
                            mod_id,
                            priority,
                            options["enabled"].as_bool().unwrap_or(true)
                        ],
                    )
                    .map_err(crate::database::error)?;
            }
            if let Some(attach) = attach_metadata {
                attach.call1((wrapper, &mod_id))?;
            }
            if empty_mod {
                let after = text(&options, "after");
                if !after.is_empty() {
                    let mut rows = crate::organization::entries(&*db.connection()?, profile)?;
                    rows.retain(|row| text(row, "mod_id") != mod_id);
                    let position = rows.iter().position(|row| text(row, "mod_id") == after)
                        .ok_or_else(|| invalid("The insertion mod is no longer in this profile. Refresh and try again."))?;
                    let group = text(&rows[position], "group_name").to_string();
                    rows.insert(position + 1, json!({"mod_id":mod_id}));
                    for (priority, row) in rows.iter().enumerate() {
                        db.connection()?
                            .execute(
                                "UPDATE entries SET priority=? WHERE profile_id=? AND mod_id=?",
                                params![priority as i64, profile, text(row, "mod_id")],
                            )
                            .map_err(crate::database::error)?;
                    }
                    db.connection()?
                        .execute(
                            "UPDATE entries SET group_name=? WHERE profile_id=? AND mod_id=?",
                            params![group, profile, mod_id],
                        )
                        .map_err(crate::database::error)?;
                }
                library.call_method0("_runtime_write_guard")?;
            }
            record_operation(db, &record, &old)?;
            checkpoint.call1(("before-commit",))?;
            check_cancel(cancel)
        })?;
        committed = true;
        checkpoint.call1(("committed",))?;
        finish(&root, &library_id, &record)?;
        Ok(mod_id.clone())
    })();
    if let Err(error) = result {
        if committed || abort(&factory, &root, &library_id, &record)? {
            let committed = TransactionCommitted::new_err(record.to_string());
            committed.set_cause(py, Some(error));
            return Err(committed);
        }
        return Err(error);
    }
    result
}

fn manifest_backup(
    py: Python<'_>,
    path: &Path,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<Value> {
    manifest(py, path, cancel, None)
}

fn insert_restored(db: &Database, value: &Value) -> PyResult<()> {
    let columns = [
        "id",
        "name",
        "source",
        "kind",
        "version",
        "category",
        "notes",
        "tags",
        "nexus_game",
        "nexus_mod_id",
        "nexus_file_id",
        "manifest",
        "categories_json",
    ];
    let values: Vec<_> = columns
        .iter()
        .map(|key| match &value[*key] {
            Value::String(v) => rusqlite::types::Value::Text(v.clone()),
            Value::Number(v) => rusqlite::types::Value::Integer(v.as_i64().unwrap_or(0)),
            _ => rusqlite::types::Value::Null,
        })
        .collect();
    db.connection()?
        .execute(
            &format!(
                "INSERT INTO mods ({}) VALUES ({})",
                columns.join(","),
                vec!["?"; columns.len()].join(",")
            ),
            rusqlite::params_from_iter(values),
        )
        .map_err(crate::database::error)?;
    Ok(())
}

#[pyfunction]
#[pyo3(signature = (library, checkpoint, review, cancel=None))]
fn transaction_uninstall(
    py: Python<'_>,
    library: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
    review: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let review = encoding::parse(review)?;
    let root: PathBuf = library.getattr("root")?.extract()?;
    let library_id: String = library.getattr("library_id")?.extract()?;
    let factory = library.getattr("_db")?;
    with_database(&factory, |_, db| {
        transaction_recover(db, root.clone(), &library_id)?;
        snapshot(db, &root, text(&review, "mod_id"))
    })?;
    let record = encoding::parse(&transaction_begin(
        root.clone(),
        &library_id,
        "uninstall",
        text(&review, "mod_id"),
        true,
        None,
    )?)?;
    checkpoint.call1(("journal",))?;
    let [_, _, destination, backup] = paths(&root, &library_id, &record)?;
    let mut committed = false;
    let result = (|| -> PyResult<String> {
        with_database(&factory, |_, db| {
            db.connection()?
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(crate::database::error)?;
            let snapshot = verify(py, db, &root, &review, cancel)?;
            check_cancel(cancel)?;
            std::fs::create_dir(backup.parent().unwrap())?;
            std::fs::rename(&destination, &backup)?;
            checkpoint.call1(("old-moved",))?;
            if encoding::digest(
                &json!({"snapshot":snapshot,"files":manifest(py,&backup,cancel,None)?}),
                true,
                false,
            ) != text(&review, "token")
            {
                return Err(invalid(
                    "Owned content changed during uninstall; the original content will be restored.",
                ));
            }
            db.connection()?
                .execute(
                    "DELETE FROM entries WHERE mod_id=?",
                    [text(&review, "mod_id")],
                )
                .map_err(crate::database::error)?;
            db.connection()?
                .execute("DELETE FROM mods WHERE id=?", [text(&review, "mod_id")])
                .map_err(crate::database::error)?;
            record_operation(db, &record, &snapshot)?;
            checkpoint.call1(("before-commit",))?;
            check_cancel(cancel)
        })?;
        committed = true;
        checkpoint.call1(("committed",))?;
        finish(&root, &library_id, &record)?;
        Ok(text(&record, "id").into())
    })();
    if let Err(error) = result {
        if committed || abort(&factory, &root, &library_id, &record)? {
            let committed = TransactionCommitted::new_err(record.to_string());
            committed.set_cause(py, Some(error));
            return Err(committed);
        }
        return Err(error);
    }
    result
}

#[pyfunction]
pub fn transaction_key(value: &str) -> PyResult<String> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("Invalid owned transaction identifier."));
    }
    Ok(value.into())
}

#[pyfunction]
fn transaction_digest(value: &str) -> PyResult<String> {
    Ok(encoding::digest(&encoding::parse(value)?, true, false))
}

fn paths(root: &Path, library: &str, record: &Value) -> PyResult<[PathBuf; 4]> {
    let operation = transaction_key(text(record, "id"))?;
    let identifier = transaction_key(text(record, "mod_id"))?;
    let kind = text(record, "kind");
    let origin = text(record, "origin");
    if record["format"] != 1
        || record["library"] != library
        || !matches!(
            kind,
            "install" | "reinstall" | "uninstall" | "restore" | "migrate"
        )
        || !record["old_present"].is_boolean()
        || (matches!(kind, "install" | "migrate") && record["old_present"] == true)
        || (kind == "migrate"
            && (origin.len() != 64
                || !origin
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))))
        || (matches!(kind, "reinstall" | "uninstall") && record["old_present"] != true)
    {
        return Err(invalid(
            "Invalid or foreign owned transaction journal; manual review is required.",
        ));
    }
    let result = [
        root.join(".transactions").join(format!("{operation}.json")),
        root.join(".staging")
            .join(format!("transaction-{operation}")),
        root.join("mods").join(identifier),
        root.join(".history").join(operation).join("payload"),
    ];
    for path in &result {
        reject_links(path)?;
    }
    Ok(result)
}

#[pyfunction]
fn transaction_paths(root: PathBuf, library: &str, record: &str) -> PyResult<Vec<PathBuf>> {
    Ok(paths(&root, library, &encoding::parse(record)?)?.to_vec())
}

#[pyfunction]
#[pyo3(signature = (root, library, kind, mod_id, old_present, origin=None))]
fn transaction_begin(
    root: PathBuf,
    library: &str,
    kind: &str,
    mod_id: &str,
    old_present: bool,
    origin: Option<&str>,
) -> PyResult<String> {
    let mut record = json!({"format":1,"library":library,"id":uuid::Uuid::new_v4().simple().to_string(),"kind":kind,"mod_id":transaction_key(mod_id)?,"old_present":old_present});
    if kind == "migrate" {
        record["origin"] = json!(origin);
    }
    let [journal, stage, _, backup] = paths(&root, library, &record)?;
    if journal.exists() || stage.exists() || backup.parent().unwrap().exists() {
        return Err(invalid("Transaction path is already occupied."));
    }
    let temporary = journal.with_extension("tmp");
    reject_links(&temporary)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(record.to_string().as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, journal)?;
    Ok(record.to_string())
}

fn finish(root: &Path, library: &str, record: &Value) -> PyResult<()> {
    let [journal, _, _, _] = paths(root, library, record)?;
    match std::fs::remove_file(journal) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[pyfunction]
fn transaction_finish(root: PathBuf, library: &str, record: &str) -> PyResult<()> {
    finish(&root, library, &encoding::parse(record)?)
}

fn quarantine(root: &Path, path: &Path, record: &Value, label: &str) -> PyResult<()> {
    if !path.exists() {
        return Ok(());
    }
    reject_links(path)?;
    let target = root
        .join(".history")
        .join(transaction_key(text(record, "id"))?)
        .join(crate::relative_path(label, false)?);
    reject_links(&target)?;
    std::fs::create_dir_all(target.parent().unwrap())?;
    if target.exists() {
        return Err(invalid(
            "Recovery destination is occupied; no content was overwritten.",
        ));
    }
    std::fs::rename(path, target)?;
    Ok(())
}

#[pyfunction]
fn transaction_quarantine(root: PathBuf, path: PathBuf, record: &str, label: &str) -> PyResult<()> {
    quarantine(&root, &path, &encoding::parse(record)?, label)
}

fn rollback(db: &Database, root: &Path, library: &str, record: &Value) -> PyResult<()> {
    let [_, stage, destination, backup] = paths(root, library, record)?;
    let rows = records(
        &*db.connection()?,
        "SELECT kind,source FROM mods WHERE id=?",
        [text(record, "mod_id")],
    )?;
    let row = rows.first();
    if record["old_present"] == true {
        if !row.is_some_and(|row| {
            row["kind"] == "owned"
                && std::path::absolute(text(row, "source")).ok().as_ref() == Some(&destination)
        }) {
            return Err(invalid(
                "Original owned metadata does not match the recovery journal; no payload was moved.",
            ));
        }
    } else if record["kind"] == "migrate" {
        if !row.is_some_and(|row| {
            row["kind"] == "external"
                && encoding::digest(&row["source"], true, false) == text(record, "origin")
        }) {
            return Err(invalid(
                "Imported source identity does not match the migration journal; no source files were changed.",
            ));
        }
    } else if row.is_some() {
        return Err(invalid(
            "Recovery would conflict with an existing mod identity; no payload was moved.",
        ));
    }
    if record["old_present"] == true {
        if backup.exists() {
            if destination.exists() {
                quarantine(root, &destination, record, "uncommitted-new")?;
            }
            std::fs::rename(backup, destination)?;
        } else if !destination.is_dir() {
            return Err(invalid(
                "Original payload and backup are missing; transaction recovery requires manual review.",
            ));
        }
    } else if destination.exists() {
        quarantine(root, &destination, record, "uncommitted-new")?;
    }
    quarantine(root, &stage, record, "abandoned-stage")?;
    finish(root, library, record)
}

#[pyfunction]
fn transaction_rollback(db: &Database, root: PathBuf, library: &str, record: &str) -> PyResult<()> {
    rollback(db, &root, library, &encoding::parse(record)?)
}

#[pyfunction]
fn transaction_recover(db: &Database, root: PathBuf, library: &str) -> PyResult<Vec<String>> {
    let directory = root.join(".transactions");
    reject_links(&directory)?;
    let mut journals = std::fs::read_dir(directory)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    journals.sort();
    let mut messages = Vec::new();
    for journal in journals {
        reject_links(&journal)?;
        if journal.extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }
        transaction_key(journal.file_stem().and_then(|v| v.to_str()).unwrap_or(""))?;
        let file = std::fs::File::open(&journal)?;
        if file.metadata()?.len() > 4096 {
            return Err(invalid(
                "Oversized transaction journal; manual review is required.",
            ));
        }
        let mut contents = String::new();
        file.take(4097).read_to_string(&mut contents)?;
        let malformed = || {
            invalid(
                "Malformed transaction journal; no payload was moved. Manual review is required.",
            )
        };
        let record = encoding::parse(&contents).map_err(|_| malformed())?;
        let [expected, stage, destination, backup] =
            paths(&root, library, &record).map_err(|_| malformed())?;
        if expected != journal {
            return Err(invalid(
                "Transaction journal identity mismatch; manual review is required.",
            ));
        }
        let committed = records(
            &*db.connection()?,
            "SELECT kind,mod_id FROM operations WHERE id=?",
            [text(&record, "id")],
        )?;
        if let Some(committed) = committed.first() {
            if committed["kind"] != record["kind"] || committed["mod_id"] != record["mod_id"] {
                return Err(invalid(
                    "Transaction commit identity mismatch; manual review is required.",
                ));
            }
            let present = if record["kind"] == "uninstall" {
                backup.is_dir()
            } else {
                destination.is_dir()
            };
            if !present {
                return Err(invalid(
                    "Committed transaction payload is missing; backups were preserved for manual review.",
                ));
            }
            quarantine(&root, &stage, &record, "abandoned-stage")?;
            finish(&root, library, &record)?;
            messages.push(format!(
                "Completed interrupted {} transaction {}.",
                text(&record, "kind"),
                text(&record, "id")
            ));
        } else {
            rollback(db, &root, library, &record)?;
            messages.push(format!("Rolled back interrupted {}; any new partial files were retained in recovery history.",text(&record,"kind")));
        }
    }
    Ok(messages)
}

fn snapshot(db: &Database, root: &Path, mod_id: &str) -> PyResult<Value> {
    transaction_key(mod_id)?;
    let connection = db.connection()?;
    let rows = records(&connection, "SELECT * FROM mods WHERE id=?", [mod_id])?;
    let Some(row) = rows.first().filter(|row| row["kind"] == "owned") else {
        return Err(invalid(
            "This operation only supports manager-owned installed mods; imported sources are read-only.",
        ));
    };
    let destination = root.join("mods").join(mod_id);
    if std::path::absolute(text(row, "source"))? != destination {
        return Err(invalid(
            "Owned mod path does not match its library identity; no files were changed.",
        ));
    }
    reject_links(&destination)?;
    if !destination.is_dir() {
        return Err(invalid(
            "Owned mod folder is missing. Restore it or review recovery history first.",
        ));
    }
    let entries = records(
        &connection,
        "SELECT * FROM entries WHERE mod_id=? ORDER BY profile_id",
        [mod_id],
    )?;
    let overlays: Vec<_> = crate::library::overlays(&connection)?
        .into_iter()
        .filter(|v| v["overlay_id"] == mod_id)
        .collect();
    let mut result = json!({"mod":row,"entries":entries});
    if !overlays.is_empty() {
        result["overlays"] = json!(overlays);
    }
    Ok(result)
}

#[pyfunction]
fn transaction_snapshot(db: &Database, root: PathBuf, mod_id: &str) -> PyResult<String> {
    Ok(snapshot(db, &root, mod_id)?.to_string())
}

pub fn manifest(
    py: Python<'_>,
    source: &Path,
    cancel: Option<&Bound<'_, PyAny>>,
    progress: Option<&Bound<'_, PyAny>>,
) -> PyResult<Value> {
    let package = crate::package::directory(py, source.to_path_buf(), cancel)?;
    let sizes = package.sizes();
    let total_bytes = sizes.values().sum::<u64>().max(1);
    let mut read_bytes = 0u64;
    let mut result = BTreeMap::new();
    for name in package.files().values() {
        check_cancel(cancel)?;
        package.source(name, false)?;
        let mut reader = std::fs::File::open(source.join(name))?;
        let mut checksum = Sha256::new();
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            check_cancel(cancel)?;
            checksum.update(&buffer[..count]);
            read_bytes += count as u64;
            if let Some(progress) = progress {
                progress.call1((
                    read_bytes as f64 / total_bytes as f64,
                    format!("Hashing {name}"),
                ))?;
            }
        }
        package.source(name, true)?;
        result.insert(name.clone(),json!({"size":sizes[&name.case_fold().collect::<String>()],"sha256":format!("{:x}",checksum.finalize())}));
    }
    let latest = crate::package::directory(py, source.to_path_buf(), cancel)?;
    if latest.signatures() != package.signatures() || latest.files() != package.files() {
        return Err(invalid(
            "Owned files changed during review; review them again.",
        ));
    }
    Ok(json!(result))
}

#[pyfunction]
#[pyo3(signature = (source, cancel=None, progress=None))]
fn transaction_manifest(
    py: Python<'_>,
    source: PathBuf,
    cancel: Option<&Bound<'_, PyAny>>,
    progress: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    Ok(manifest(py, &source, cancel, progress)?.to_string())
}

#[pyfunction]
#[pyo3(signature = (db, root, mod_id, cancel=None))]
fn transaction_review(
    py: Python<'_>,
    db: &Database,
    root: PathBuf,
    mod_id: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let snapshot = snapshot(db, &root, mod_id)?;
    let manifest = manifest(
        py,
        Path::new(text(&snapshot["mod"], "source")),
        cancel,
        None,
    )?;
    let raw = text(&snapshot["mod"], "manifest");
    let baseline = encoding::parse(if raw.is_empty() { "{}" } else { raw })?;
    let mut changes = Vec::new();
    if baseline.as_object().is_none_or(|v| v.is_empty()) {
        changes.push("Baseline unavailable; all current files will be backed up.".into());
    } else {
        let actual: BTreeMap<_, _> = manifest
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.case_fold().collect::<String>(), v))
            .collect();
        let original: BTreeMap<_, _> = baseline
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.case_fold().collect::<String>(), v))
            .collect();
        for (name, value) in &actual {
            match original.get(name) {
                None => changes.push(format!("Added: {name}")),
                Some(old) if old != value => changes.push(format!("Modified: {name}")),
                _ => (),
            }
        }
        for name in original.keys().filter(|k| !actual.contains_key(*k)) {
            changes.push(format!("Removed: {name}"));
        }
    }
    changes.sort();
    let profiles = records(&*db.connection()?,"SELECT p.name FROM profiles p JOIN entries e ON e.profile_id=p.id WHERE e.mod_id=? ORDER BY p.id",[mod_id])?.into_iter().map(|v|v["name"].clone()).collect::<Vec<_>>();
    let review = json!({"mod_id":mod_id,"name":snapshot["mod"]["name"],"token":encoding::digest(&json!({"snapshot":snapshot,"files":manifest}),true,false),"profiles":profiles,"changed":changes,"file_count":manifest.as_object().unwrap().len(),"bytes":manifest.as_object().unwrap().values().map(|v|v["size"].as_u64().unwrap()).sum::<u64>()});
    Ok(json!([review, snapshot]).to_string())
}

#[pyfunction]
fn transaction_history(db: &Database, root: PathBuf) -> PyResult<String> {
    let connection = db.connection()?;
    let current: BTreeSet<_> = records(&connection, "SELECT id FROM mods WHERE kind='owned'", [])?
        .into_iter()
        .map(|v| text(&v, "id").to_string())
        .collect();
    let mut result = Vec::new();
    for row in records(
        &connection,
        "SELECT * FROM operations ORDER BY created DESC, rowid DESC",
        [],
    )? {
        let backup = root
            .join(".history")
            .join(transaction_key(text(&row, "id"))?)
            .join("payload");
        reject_links(&backup)?;
        let snapshot = encoding::parse(text(&row, "snapshot"))?;
        if backup.is_dir() && snapshot["mod"].is_object() {
            result.push(json!({"id":row["id"],"kind":row["kind"],"name":snapshot["mod"]["name"],"mod_id":row["mod_id"],"path":backup.to_string_lossy(),"current":current.contains(text(&row,"mod_id"))}));
        }
    }
    Ok(json!(result).to_string())
}

#[pyfunction]
fn transaction_record(db: &Database, record: &str, snapshot: &str) -> PyResult<()> {
    record_operation(db, &encoding::parse(record)?, &encoding::parse(snapshot)?)
}

#[pyfunction]
fn transaction_abort(
    factory: &Bound<'_, PyAny>,
    root: PathBuf,
    library: &str,
    record: &str,
) -> PyResult<bool> {
    abort(factory, &root, library, &encoding::parse(record)?)
}

fn operation(db: &Database, id: &str) -> PyResult<(Value, Value)> {
    transaction_key(id)?;
    let rows = records(
        &*db.connection()?,
        "SELECT * FROM operations WHERE id=?",
        [id],
    )?;
    let row = rows
        .into_iter()
        .next()
        .ok_or_else(|| invalid("Recovery entry no longer exists."))?;
    let snapshot = encoding::parse(text(&row, "snapshot"))?;
    Ok((row, snapshot))
}

#[pyfunction]
#[pyo3(signature = (db, root, operation_id, cancel=None))]
fn transaction_review_restore(
    py: Python<'_>,
    db: &Database,
    root: PathBuf,
    operation_id: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let (operation, snapshot) = operation(db, operation_id)?;
    if !snapshot["mod"].is_object() {
        return Err(invalid("This entry has no restorable payload."));
    }
    let backup = root.join(".history").join(operation_id).join("payload");
    reject_links(&backup)?;
    let manifest = manifest(py, &backup, cancel, None)?;
    let current = !records(
        &*db.connection()?,
        "SELECT id FROM mods WHERE id=?",
        [text(&operation, "mod_id")],
    )?
    .is_empty();
    let review = if current {
        encoding::parse(&transaction_review(
            py,
            db,
            root,
            text(&operation, "mod_id"),
            cancel,
        )?)?[0]
            .clone()
    } else {
        Value::Null
    };
    let entry = json!({"id":operation_id,"name":snapshot["mod"]["name"],"kind":operation["kind"],"mod_id":operation["mod_id"],"path":backup.to_string_lossy(),"backup_token":encoding::digest(&manifest,true,false),"file_count":manifest.as_object().unwrap().len(),"bytes":manifest.as_object().unwrap().values().map(|v|v["size"].as_u64().unwrap()).sum::<u64>()});
    Ok(json!([entry, review]).to_string())
}

#[pyfunction]
#[pyo3(signature = (db, root, operation_id, review, backup_token=None, cancel=None))]
fn transaction_restore_options(
    py: Python<'_>,
    db: &Database,
    root: PathBuf,
    operation_id: &str,
    review: &str,
    backup_token: Option<&str>,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let review = encoding::parse(review)?;
    let (operation, mut snapshot) = operation(db, operation_id)?;
    if snapshot["mod"]["kind"] != "owned" {
        return Err(invalid("This transaction has no restorable owned payload."));
    }
    let columns: BTreeSet<_> = [
        "id",
        "name",
        "source",
        "kind",
        "version",
        "category",
        "notes",
        "tags",
        "nexus_game",
        "nexus_mod_id",
        "nexus_file_id",
        "manifest",
        "categories_json",
    ]
    .into_iter()
    .collect();
    let keys: BTreeSet<_> = snapshot["mod"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    if keys.len() == columns.len() - 1
        && !keys.contains("categories_json")
        && keys.is_subset(&columns)
    {
        snapshot["mod"]["categories_json"] = json!("");
    }
    let keys: BTreeSet<_> = snapshot["mod"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    if keys != columns
        || transaction_key(text(&snapshot["mod"], "id"))? != text(&operation, "mod_id")
    {
        return Err(invalid(
            "Recovery metadata identity is invalid; no payload was changed.",
        ));
    }
    let current = !records(
        &*db.connection()?,
        "SELECT id FROM mods WHERE id=?",
        [text(&operation, "mod_id")],
    )?
    .is_empty();
    if current && (review.is_null() || review["mod_id"] != operation["mod_id"]) {
        return Err(invalid(
            "Review the current owned mod before restoring an older version over it.",
        ));
    }
    if !current && !review.is_null() {
        return Err(invalid(
            "The reviewed mod was removed; review restoration again.",
        ));
    }
    let backup = root.join(".history").join(operation_id).join("payload");
    reject_links(&backup)?;
    if let Some(token) = backup_token {
        if encoding::digest(&manifest(py, &backup, cancel, None)?, true, false) != token {
            return Err(invalid(
                "Recovery backup changed after review. Review restoration again.",
            ));
        }
    }
    let origin = &snapshot["mod"];
    let nexus = json!({"version":origin["version"],"game":origin["nexus_game"],"mod_id":origin["nexus_mod_id"],"file_id":origin["nexus_file_id"]});
    let files = crate::package::directory(py, backup.clone(), cancel)?.files();
    let plan: BTreeMap<_, _> = files
        .values()
        .map(|name| (name.clone(), name.clone()))
        .collect();
    Ok(json!({"backup":backup.to_string_lossy(),"name":origin["name"],"plan":plan,"nexus":nexus,"restored":snapshot,"review":review,"expected_manifest_token":backup_token,"profile_id":"","enabled":true}).to_string())
}

#[pyfunction]
fn transaction_lock(owned: &Bound<'_, PyAny>, shared: bool) -> PyResult<crate::RuntimeLease> {
    crate::RuntimeLease::new(
        owned
            .getattr("root")?
            .extract::<PathBuf>()?
            .join(".library.lock"),
        shared,
    )
}

#[pyfunction]
#[pyo3(signature = (library, profile, name, after="", cancel=None))]
fn transaction_empty_mod(
    library: &Bound<'_, PyAny>,
    profile: &str,
    name: &str,
    after: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 200 || name.chars().any(|c| c < ' ') {
        return Err(invalid(
            "Give the mod a short name without control characters.",
        ));
    }
    library.call_method0("_runtime_write_guard")?;
    let py = library.py();
    let owned = py
        .import("creation_lib.mod_manager.transactions")?
        .getattr("OwnedTransactions")?
        .call1((library,))?;
    crate::python_io::with_context(&owned.call_method0("lock")?, |_| {
        let options = json!({"empty_mod":true,"plan":{},"name":name,"profile_id":profile,"after":after,"enabled":true});
        transaction_install(
            py,
            library,
            &owned.getattr("_checkpoint")?,
            py.None().bind(py),
            &options.to_string(),
            cancel,
            None,
            None,
            None,
        )
    })
}

#[pyfunction]
fn transaction_owned_review(
    owned: &Bound<'_, PyAny>,
    identifier: &str,
    restore: bool,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let py = owned.py();
    let library = owned.getattr("library")?;
    crate::python_io::with_context(&owned.call_method0("lock")?, |_| {
        owned.call_method0("_recover_locked")?;
        with_database(&library.getattr("_db")?, |_, db| {
            let root = owned.getattr("root")?.extract()?;
            if restore {
                transaction_review_restore(py, db, root, identifier, Some(cancel))
            } else {
                transaction_review(py, db, root, identifier, Some(cancel))
            }
        })
    })
}

#[pyfunction]
fn transaction_owned_recover(owned: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    crate::python_io::with_context(&owned.call_method0("lock")?, |_| {
        Ok(owned.call_method0("_recover_locked")?.unbind())
    })
}

#[pyfunction]
fn transaction_owned_restore(
    owned: &Bound<'_, PyAny>,
    operation_id: &str,
    review: &str,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
    backup_token: Option<&str>,
) -> PyResult<String> {
    let py = owned.py();
    let library = owned.getattr("library")?;
    let root = owned.getattr("root")?.extract()?;
    let mut options = encoding::parse(&with_database(&library.getattr("_db")?, |_, db| {
        transaction_restore_options(
            py,
            db,
            root,
            operation_id,
            review,
            backup_token,
            Some(cancel),
        )
    })?)?;
    let backup = options.as_object_mut().unwrap().remove("backup").unwrap();
    let module = py.import("creation_lib.mod_manager.package")?;
    let kwargs = pyo3::types::PyDict::new(py);
    kwargs.set_item("inspect_installer", false)?;
    let package = module.getattr("Package")?.call(
        (
            backup.as_str().unwrap(),
            cancel,
            module.getattr("OWNED_LIMITS")?,
        ),
        Some(&kwargs),
    )?;
    let result = (|| {
        options["restore_directories"] = crate::plugin_inventory::value(
            &py.import("builtins")?
                .call_method1("list", (package.getattr("directories")?,))?,
        )?;
        crate::python_io::with_context(&owned.call_method0("lock")?, |_| {
            transaction_install(
                py,
                &library,
                &owned.getattr("_checkpoint")?,
                &package,
                &options.to_string(),
                Some(cancel),
                Some(progress).filter(|v| !v.is_none()),
                None,
                None,
            )
        })
    })();
    package.call_method0("close")?;
    result
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(transaction_empty_mod, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_lock, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_owned_review, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_owned_recover, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_owned_restore, m)?)?;
    m.add(
        "TransactionCommitted",
        m.py().get_type::<TransactionCommitted>(),
    )?;
    m.add_function(wrap_pyfunction!(transaction_install, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_uninstall, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_record, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_abort, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_review_restore, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_restore_options, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_key, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_digest, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_paths, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_begin, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_finish, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_quarantine, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_rollback, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_recover, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_snapshot, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_manifest, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_review, m)?)?;
    m.add_function(wrap_pyfunction!(transaction_history, m)?)?;
    Ok(())
}
