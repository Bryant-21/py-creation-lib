use crate::{
    check_cancel, encoding,
    history_prune::py_path,
    invalid,
    plugin_inventory::{object, value},
    python_io::{with_context, with_database},
    reject_links,
    saves::fold,
    transactions::transaction_key,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
const LOCK: &str = ".modbox21-f4se-root.lock";
const MAX_FILE: u64 = 16 * 1024 * 1024;
fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("creation_lib.mod_manager.f4se_root_setup")
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn fields(v: &Value) -> PyResult<&Map<String, Value>> {
    v["files"]
        .as_object()
        .ok_or_else(|| invalid("Invalid root receipt files."))
}
fn dataclass(item: &Bound<'_, PyAny>) -> PyResult<Value> {
    value(
        &item
            .py()
            .import("dataclasses")?
            .call_method1("asdict", (item,))?,
    )
}
fn digest(py: Python<'_>, v: &Value) -> PyResult<String> {
    py.import("creation_lib.mod_manager.profile_backup")?
        .call_method1("digest", (object(py, v)?,))?
        .extract()
}
fn signature(py: Python<'_>, path: &Path) -> PyResult<Value> {
    value(
        &py.import("creation_lib.mod_manager.launcher")?
            .call_method1("fingerprint", (py_path(py, path)?,))?,
    )
}
fn checkpoint(py: Python<'_>, phase: &str) -> PyResult<()> {
    module(py)?.call_method1("_checkpoint", (phase,))?;
    Ok(())
}
fn stopped(py: Python<'_>) -> PyResult<()> {
    let names = py
        .import("creation_lib.mod_manager.runtime")?
        .call_method0("process_names")?;
    for name in ["fallout4.exe", "fallout4launcher.exe", "f4se_loader.exe"] {
        if names.contains(name)? {
            return Err(invalid(
                "Close all Fallout 4 launchers, games and F4SE processes before root setup/restore.",
            ));
        }
    }
    Ok(())
}
fn snapshot(py: Python<'_>, path: &Path) -> PyResult<Value> {
    reject_links(path)?;
    if !path.exists() {
        return Ok(Value::Null);
    }
    if !path.is_file() || std::fs::metadata(path)?.len() > MAX_FILE {
        return Err(invalid(
            "F4SE root components must be regular files no larger than 16 MiB.",
        ));
    }
    let before = signature(py, path)?;
    let bytes = std::fs::read(path)?;
    if bytes.len() as u64 > MAX_FILE || signature(py, path)? != before {
        return Err(invalid("F4SE root file changed while reading it."));
    }
    Ok(json!({"sha256":format!("{:x}",Sha256::digest(&bytes)),"size":bytes.len(),"stamp":before}))
}
#[pyfunction]
fn f4se_root_snapshot(py: Python<'_>, path: PathBuf) -> PyResult<Py<PyAny>> {
    Ok(object(py, &snapshot(py, &path)?)?.unbind())
}
fn target<'py>(
    library: &Bound<'py, PyAny>,
    id: &str,
    db: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    let module = library
        .py()
        .import("creation_lib.mod_manager.launch_targets")?;
    let records = if let Some(db) = db {
        module.call_method1("records", (library, db))?
    } else {
        module.call_method1("records", (library,))?
    };
    for item in records.try_iter()? {
        let item = item?;
        if item.getattr("id")?.eq(id)? {
            return Ok(item);
        }
    }
    Err(invalid("Select a configured FO4 target first."))
}
fn target_root(library: &Bound<'_, PyAny>, id: &str) -> PyResult<PathBuf> {
    library
        .py()
        .import("creation_lib.mod_manager.launch_targets")?
        .getattr("TargetLibrary")?
        .call1((library, id))?
        .getattr("root")?
        .extract()
}
#[pyfunction]
fn f4se_root_base(library: &Bound<'_, PyAny>, id: &str) -> PyResult<PathBuf> {
    let path = target_root(library, id)?.join("f4se-root-history");
    reject_links(&path)?;
    Ok(path)
}
fn write_record(py: Python<'_>, path: &Path, record: &Value) -> PyResult<()> {
    reject_links(path)?;
    let temporary = path.with_extension("new");
    reject_links(&temporary)?;
    let mut file = std::fs::File::create(&temporary)?;
    file.write_all(record.to_string().as_bytes())?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    py_path(py, &temporary)?.call_method1("replace", (py_path(py, path)?,))?;
    Ok(())
}
fn read_record(
    library: &Bound<'_, PyAny>,
    target_id: &str,
    receipt_id: &str,
) -> PyResult<(PathBuf, Value)> {
    let py = library.py();
    let path = f4se_root_base(library, target_id)?
        .join(transaction_key(receipt_id)?)
        .join("receipt.json");
    reject_links(&path)?;
    if !path.is_file() || std::fs::metadata(&path)?.len() > 128 * 1024 {
        return Err(invalid(
            "Root setup receipt is missing or exceeds its size limit; preserve it for review.",
        ));
    }
    let read = (|| -> PyResult<Value> {
        let value = encoding::parse(&std::fs::read_to_string(&path)?)?;
        let before = &value["before"];
        let setup = py.import("creation_lib.mod_manager.f4se_setup")?;
        let targets = py.import("creation_lib.mod_manager.launch_targets")?;
        targets
            .getattr("Target")?
            .call((), Some(object(py, before)?.cast::<PyDict>()?))?;
        let build = before["f4se"]["build"]
            .as_array()
            .ok_or_else(|| invalid("build"))?;
        let release = setup.getattr("CATALOG")?.get_item(PyTuple::new(
            py,
            build
                .iter()
                .take(3)
                .map(|v| object(py, v))
                .collect::<PyResult<Vec<_>>>()?,
        )?)?;
        let runtime = setup
            .call_method1("runtime_name", (object(py, &before["f4se"]["build"])?,))?
            .extract::<String>()?;
        let mut names = BTreeSet::from(["f4se_loader.exe".to_owned(), runtime]);
        if release.get_item("steam_loader")?.is_truthy()? {
            names.insert("f4se_steam_loader.dll".into());
        }
        let files = fields(&value)?;
        let actual: BTreeSet<_> = files.keys().cloned().collect();
        if value["format"] != 1
            || text(&value, "library") != library.getattr("library_id")?.extract::<String>()?
            || text(&value, "target") != transaction_key(target_id)?
            || text(&value, "id") != transaction_key(receipt_id)?
            || value["game"] != before["game_root"]
            || text(before, "id") != target_id
            || !matches!(
                text(&value, "state"),
                "pending" | "complete" | "restoring" | "restored" | "rolled-back"
            )
            || before["f4se"]["managed"] != true
            || before["f4se"]["root_setup"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
            || actual != names
            || files.len() != names.len()
        {
            return Err(invalid("identity"));
        }
        for (name, item) in files {
            let expected = before["f4se"]["manifest"]
                .as_object()
                .and_then(|v| v.iter().find(|(path, _)| fold(path) == fold(name)))
                .map(|(_, v)| v);
            if item["new"].is_null() || expected != Some(&item["new"]["sha256"]) {
                return Err(invalid("package identity"));
            }
            for info in [&item["old"], &item["new"]] {
                if !info.is_null() {
                    let hash = text(info, "sha256");
                    if hash.len() != 64
                        || !hash
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                        || !info["size"].as_u64().is_some_and(|v| v <= MAX_FILE)
                    {
                        return Err(invalid("file"));
                    }
                }
            }
        }
        let stage = PathBuf::from(text(&before["f4se"], "stage"));
        transaction_key(stage.file_name().and_then(|v| v.to_str()).unwrap_or(""))?;
        if stage.parent() != Some(target_root(library, target_id)?.join("f4se").as_path()) {
            return Err(invalid("stage"));
        }
        if text(before, "binary") != stage.join("f4se_loader.exe").to_string_lossy()
            || text(&before["f4se"], "data") != stage.join("Data").to_string_lossy()
        {
            return Err(invalid("private runtime"));
        }
        if let Some(after) = value.get("after") {
            targets
                .getattr("Target")?
                .call((), Some(object(py, after)?.cast::<PyDict>()?))?;
            let mut changed = after.clone();
            changed["binary"] = before["binary"].clone();
            changed["f4se"] = before["f4se"].clone();
            let mut normalized = after["f4se"].clone();
            let mut expected = before["f4se"].clone();
            for field in ["route", "root_setup", "root_files", "reason"] {
                normalized
                    .as_object_mut()
                    .ok_or_else(|| invalid("published target"))?
                    .remove(field);
                expected
                    .as_object_mut()
                    .ok_or_else(|| invalid("published target"))?
                    .remove(field);
            }
            let root_files = after["f4se"]["root_files"]
                .as_array()
                .ok_or_else(|| invalid("published target"))?;
            let game = PathBuf::from(text(&value, "game"));
            let listed: BTreeSet<_> = root_files
                .iter()
                .map(|v| v[0].as_str().unwrap_or("").to_owned())
                .collect();
            if changed != *before
                || normalized != expected
                || after["game_root"] != value["game"]
                || after["f4se"]["route"] != "game-root"
                || after["f4se"]["root_setup"] != value["id"]
                || text(after, "binary") != game.join("f4se_loader.exe").to_string_lossy()
                || listed != names
                || root_files.len() != names.len()
                || root_files.iter().any(|v| {
                    PathBuf::from(v[1].as_str().unwrap_or(""))
                        != game.join(v[0].as_str().unwrap_or(""))
                })
            {
                return Err(invalid("published target"));
            }
        }
        reject_links(&PathBuf::from(text(&value, "game")))?;
        Ok(value)
    })();
    match read {
        Ok(value) => Ok((path, value)),
        Err(e)
            if e.is_instance_of::<pyo3::exceptions::PyKeyError>(py)
                || e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
                || e.is_instance_of::<pyo3::exceptions::PyTypeError>(py)
                || e.is_instance_of::<pyo3::exceptions::PyOSError>(py) =>
        {
            Err(invalid(
                "Invalid/foreign F4SE root receipt; no game files were changed.",
            ))
        }
        Err(e) => Err(e),
    }
}
#[pyfunction]
fn f4se_root_read(
    library: &Bound<'_, PyAny>,
    target_id: &str,
    receipt_id: &str,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let (path, record) = read_record(library, target_id, receipt_id)?;
    Ok((py_path(py, &path)?, object(py, &record)?)
        .into_pyobject(py)?
        .into_any()
        .unbind())
}
fn history(library: &Bound<'_, PyAny>, id: &str) -> PyResult<Vec<Value>> {
    let base = f4se_root_base(library, id)?;
    if !base.is_dir() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for (index, folder) in std::fs::read_dir(base)?.enumerate() {
        if index >= 64 {
            return Err(invalid(
                "Root setup history exceeds 64 records; preserve it for review.",
            ));
        }
        let folder = folder?;
        reject_links(&folder.path())?;
        result.push(read_record(library, id, &folder.file_name().to_string_lossy())?.1);
    }
    Ok(result)
}
#[pyfunction]
fn f4se_root_history(library: &Bound<'_, PyAny>, id: &str) -> PyResult<Py<PyAny>> {
    Ok(object(library.py(), &json!(history(library, id)?))?.unbind())
}
fn validate_ready(
    target: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    owner: &Path,
) -> PyResult<()> {
    let py = target.py();
    let kwargs = PyDict::new(py);
    kwargs.set_item("owner_root", py_path(py, owner)?)?;
    py.import("creation_lib.mod_manager.f4se_setup")?
        .call_method("validate_ready", (target, cancel), Some(&kwargs))?;
    Ok(())
}
fn review_object(
    library: &Bound<'_, PyAny>,
    id: &str,
    target: &Value,
    game: &Path,
    files: Vec<(String, Value, Value)>,
    receipt_id: &str,
    receipt: Option<&Value>,
    restore: bool,
    temporaries: Vec<(String, Value)>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let files = files
        .into_iter()
        .map(|(name, old, new)| Ok((name, object(py, &old)?, object(py, &new)?)))
        .collect::<PyResult<Vec<_>>>()?;
    let temporaries = temporaries
        .into_iter()
        .map(|(name, info)| Ok((name, object(py, &info)?)))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(module(py)?
        .getattr("Review")?
        .call1((
            library.getattr("library_id")?,
            id,
            digest(py, target)?,
            py.import("builtins")?
                .getattr("tuple")?
                .call1((object(py, &signature(py, &game.join("Fallout4.exe"))?)?,))?,
            PyTuple::new(py, files)?,
            receipt_id,
            if let Some(receipt) = receipt {
                digest(py, receipt)?
            } else {
                String::new()
            },
            restore,
            PyTuple::new(py, temporaries)?,
        ))?
        .unbind())
}
#[pyfunction]
fn f4se_root_review(
    library: &Bound<'_, PyAny>,
    id: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    library.call_method0("_runtime_write_guard")?;
    stopped(py)?;
    let target = target(library, id, None)?;
    let data = dataclass(&target)?;
    let game = PathBuf::from(text(&data, "game_root"));
    for other in py
        .import("creation_lib.mod_manager.launch_targets")?
        .call_method1("records", (library,))?
        .try_iter()?
    {
        let other = dataclass(&other?)?;
        if text(&other, "id") != id
            && fold(text(&other, "game_root")) == fold(text(&data, "game_root"))
            && other["f4se"]["root_setup"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
        {
            return Err(invalid(
                "Another target owns root setup for this same physical folder; restore that setup first.",
            ));
        }
    }
    let receipt_id = text(&data["f4se"], "root_setup");
    if !receipt_id.is_empty() {
        let (path, receipt) = read_record(library, id, receipt_id)?;
        if receipt["state"] != "complete" {
            return Err(invalid(
                "Review recovery for this unfinished root setup first.",
            ));
        }
        validate_ready(
            &target,
            cancel,
            path.parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .unwrap(),
        )?;
        return review_object(
            library,
            id,
            &data,
            &game,
            Vec::new(),
            text(&receipt, "id"),
            Some(&receipt),
            false,
            Vec::new(),
        );
    }
    if data["f4se"]["managed"] != true {
        return Err(invalid(
            "Prepare an official matching private F4SE package before reviewing root setup.",
        ));
    }
    reject_links(&game.join(LOCK))?;
    if game.join(LOCK).exists()
        || history(library, id)?
            .iter()
            .any(|v| matches!(text(v, "state"), "pending" | "complete" | "restoring"))
    {
        return Err(invalid(
            "Root setup/recovery already owns this game folder; review its restore first.",
        ));
    }
    validate_ready(&target, cancel, &target_root(library, id)?)?;
    let mut files = Vec::new();
    for item in data["f4se"]["root_files"]
        .as_array()
        .ok_or_else(|| invalid("Invalid F4SE root files."))?
    {
        check_cancel(Some(cancel))?;
        let name = item[0].as_str().unwrap_or("");
        files.push((
            name.to_owned(),
            snapshot(py, &game.join(name))?,
            snapshot(py, &PathBuf::from(item[1].as_str().unwrap_or("")))?,
        ));
    }
    review_object(
        library,
        id,
        &data,
        &game,
        files,
        "",
        None,
        false,
        Vec::new(),
    )
}
fn game_lock<T>(
    game: &Path,
    record: &Value,
    recovery: bool,
    body: impl FnOnce(&mut bool) -> PyResult<T>,
) -> PyResult<T> {
    let path = game.join(LOCK);
    reject_links(&path)?;
    let identity = json!({"library":record["library"],"target":record["target"],"id":record["id"]});
    let mut file = match OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            file.write_all(b"0")?;
            file.write_all(identity.to_string().as_bytes())?;
            file.flush()?;
            file.sync_all()?;
            file
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if !recovery {
                return Err(invalid(
                    "Another root operation/recovery owns this game folder.",
                ));
            }
            OpenOptions::new().read(true).write(true).open(&path)?
        }
        Err(error) => return Err(error.into()),
    };
    file.try_lock().map_err(|_| {
        invalid("The root setup lock is busy or unavailable; no competing operation was performed.")
    })?;
    file.seek(SeekFrom::Start(1))?;
    let mut bytes = Vec::new();
    let read = std::io::Read::by_ref(&mut file)
        .take(8193)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            invalid("The retained root lock is unreadable; preserve it for manual review.")
        });
    read?;
    let lock_identity: Value = serde_json::from_slice(&bytes).map_err(|_| {
        invalid("The retained root lock is unreadable; preserve it for manual review.")
    })?;
    if lock_identity != identity {
        return Err(invalid(
            "The root operation lock belongs to another receipt; preserve it for review.",
        ));
    }
    let mut release = false;
    let result = body(&mut release);
    drop(file);
    if release {
        reject_links(&path)?;
        let bytes = std::fs::read(&path)?;
        if bytes.len() > 8193
            || bytes.is_empty()
            || serde_json::from_slice::<Value>(&bytes[1..]).map_err(|e| invalid(e.to_string()))?
                != identity
        {
            return Err(invalid(
                "Root lock identity changed; it was retained for manual review.",
            ));
        }
        std::fs::remove_file(path)?;
    }
    result
}
fn matches(info: &Value, expected: &Value) -> bool {
    (info.is_null() && expected.is_null())
        || (!info.is_null()
            && !expected.is_null()
            && info["sha256"] == expected["sha256"]
            && info["size"] == expected["size"])
}
#[pyfunction]
fn f4se_root_copy(
    source: PathBuf,
    destination: PathBuf,
    identifier: &str,
    expected: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = expected.py();
    let temporary = destination
        .parent()
        .ok_or_else(|| invalid("Invalid root destination."))?
        .join(format!(
            ".modbox21-f4se-{}-{}.tmp",
            transaction_key(identifier)?,
            destination
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        ));
    reject_links(&temporary)?;
    if temporary.exists() {
        return Err(invalid(
            "A retained root temporary needs reviewed recovery before retrying.",
        ));
    }
    with_context(
        &py_path(py, &source)?.call_method1("open", ("rb",))?,
        |reader| {
            with_context(
                &py_path(py, &temporary)?.call_method1("open", ("xb",))?,
                |writer| {
                    py.import("shutil")?
                        .call_method1("copyfileobj", (reader, writer, 1024 * 1024))?;
                    writer.call_method0("flush")?;
                    py.import("os")?
                        .call_method1("fsync", (writer.call_method0("fileno")?,))?;
                    Ok(())
                },
            )
        },
    )?;
    py.import("shutil")?.call_method1(
        "copystat",
        (py_path(py, &source)?, py_path(py, &temporary)?),
    )?;
    if !matches(&snapshot(py, &temporary)?, &value(expected)?) {
        return Err(invalid(
            "Root copy changed during transfer; its temporary was retained for review.",
        ));
    }
    py_path(py, &temporary)?.call_method1("replace", (py_path(py, &destination)?,))?;
    Ok(())
}
fn copy(
    py: Python<'_>,
    source: &Path,
    destination: &Path,
    id: &str,
    expected: &Value,
) -> PyResult<()> {
    module(py)?.call_method1(
        "_copy",
        (
            py_path(py, source)?,
            py_path(py, destination)?,
            id,
            object(py, expected)?,
        ),
    )?;
    Ok(())
}
fn temporary(game: &Path, record: &Value, name: &str) -> PathBuf {
    game.join(format!(".modbox21-f4se-{}-{name}.tmp", text(record, "id")))
}
fn rollback(
    library: &Bound<'_, PyAny>,
    path: &Path,
    record: &mut Value,
    completed: bool,
    approved: &Value,
) -> PyResult<()> {
    let py = library.py();
    let game = PathBuf::from(text(record, "game"));
    let current = dataclass(&target(library, text(record, "target"), None)?)?;
    let current_digest = digest(py, &current)?;
    if current_digest != digest(py, &record["before"])?
        && (!record.get("after").is_some() || current_digest != digest(py, &record["after"])?)
    {
        return Err(invalid(
            "Target changed outside this root transaction; recovery needs manual review.",
        ));
    }
    for (name, item) in fields(record)? {
        let actual = snapshot(py, &game.join(name))?;
        if !matches(&actual, &item["old"]) && !matches(&actual, &item["new"]) {
            return Err(invalid(format!(
                "{name} changed outside this setup. It and the backup were preserved; recovery needs manual review."
            )));
        }
        if !item["old"].is_null()
            && !matches(&actual, &item["old"])
            && !matches(
                &snapshot(py, &path.parent().unwrap().join("backup").join(name))?,
                &item["old"],
            )
        {
            return Err(invalid(
                "Root backup changed/missing; no destination was restored.",
            ));
        }
        let temporary = temporary(&game, record, name);
        let info = snapshot(py, &temporary)?;
        let approved = &approved[temporary
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .as_ref()];
        if !info.is_null()
            && !matches(&info, &item["old"])
            && !matches(&info, &item["new"])
            && info != *approved
        {
            return Err(invalid(
                "Root temporary changed/incomplete; preserve it for manual review.",
            ));
        }
    }
    with_database(&library.getattr("_db")?, |db, native| {
        native
            .connection()?
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(crate::database::error)?;
        if digest(
            py,
            &dataclass(&target(library, text(record, "target"), Some(db))?)?,
        )? != current_digest
        {
            return Err(invalid(
                "Target changed before restore; review recovery again.",
            ));
        }
        record["state"] = json!("restoring");
        write_record(py, path, record)?;
        for (name, item) in fields(record)? {
            let temp = temporary(&game, record, name);
            if temp.exists() {
                std::fs::remove_file(&temp)?;
            }
            if !matches(&snapshot(py, &game.join(name))?, &item["old"]) {
                if item["old"].is_null() {
                    std::fs::remove_file(game.join(name))?;
                } else {
                    copy(
                        py,
                        &path.parent().unwrap().join("backup").join(name),
                        &game.join(name),
                        text(record, "id"),
                        &item["old"],
                    )?;
                }
            }
            checkpoint(py, "restore-file")?;
        }
        library.call_method0("_runtime_write_guard")?;
        native
            .connection()?
            .execute(
                "UPDATE launch_targets SET record=? WHERE id=?",
                rusqlite::params![record["before"].to_string(), text(record, "target")],
            )
            .map_err(crate::database::error)?;
        Ok(())
    })?;
    record["state"] = json!(if completed { "restored" } else { "rolled-back" });
    write_record(py, path, record)
}
#[pyfunction]
fn f4se_root_apply(
    library: &Bound<'_, PyAny>,
    plan: &Bound<'_, PyAny>,
    accept_root_writes: bool,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    if !accept_root_writes || plan.getattr("restore")?.is_truthy()? {
        return Err(invalid(
            "Acknowledge the reviewed game-root replacements/backups before applying setup.",
        ));
    }
    library.call_method0("_runtime_write_guard")?;
    stopped(py)?;
    with_context(&library.getattr("owned")?.call_method0("lock")?, |_| {
        let id = plan.getattr("target_id")?.extract::<String>()?;
        if !f4se_root_review(library, &id, cancel)?.bind(py).eq(plan)?
            || !plan
                .getattr("library_id")?
                .eq(library.getattr("library_id")?)?
        {
            return Err(invalid(
                "Game/target/root files changed after review; no root files were replaced.",
            ));
        }
        let target = target(library, &id, None)?;
        if plan.getattr("receipt_id")?.is_truthy()? {
            return Ok(target.unbind());
        }
        let before = dataclass(&target)?;
        let base = f4se_root_base(library, &id)?;
        if !base.exists() {
            std::fs::create_dir(&base)?;
        }
        if history(library, &id)?.len() >= 64 {
            return Err(invalid(
                "Root setup history is full; no game files were changed.",
            ));
        }
        let identifier = uuid::Uuid::new_v4().simple().to_string();
        let folder = base.join(&identifier);
        std::fs::create_dir(&folder)?;
        let backup = folder.join("backup");
        std::fs::create_dir(&backup)?;
        let path = folder.join("receipt.json");
        let game = PathBuf::from(text(&before, "game_root"));
        let mut entries = Map::new();
        for item in plan.getattr("files")?.try_iter()? {
            let item = item?;
            entries.insert(
                item.get_item(0)?.extract::<String>()?,
                json!({"old":value(&item.get_item(1)?)?,"new":value(&item.get_item(2)?)?}),
            );
        }
        let mut record = json!({"format":1,"library":library.getattr("library_id")?.extract::<String>()?,"target":id,"id":identifier,"game":game.to_string_lossy(),"before":before,"state":"pending","files":entries});
        write_record(py, &path, &record)?;
        let identity = record.clone();
        game_lock(&game, &identity, false, |release| {
            let mut published = false;
            let result = (|| -> PyResult<Py<PyAny>> {
                for (name, item) in fields(&record)? {
                    check_cancel(Some(cancel))?;
                    if !item["old"].is_null() {
                        py.import("shutil")?.call_method1(
                            "copy2",
                            (
                                py_path(py, &game.join(name))?,
                                py_path(py, &backup.join(name))?,
                            ),
                        )?;
                        if !matches(&snapshot(py, &backup.join(name))?, &item["old"]) {
                            return Err(invalid(
                                "Game-root original changed during backup; no root files were replaced.",
                            ));
                        }
                    }
                }
                let target_signature = plan.getattr("target_signature")?.extract::<String>()?;
                let game_signature = value(&plan.getattr("game_signature")?)?;
                if digest(py, &dataclass(&target_lookup(library, &id, None)?)?)? != target_signature
                    || signature(py, &game.join("Fallout4.exe"))? != game_signature
                {
                    return Err(invalid(
                        "Target/game changed during backup; review recovery again.",
                    ));
                }
                checkpoint(py, "before-copy")?;
                check_cancel(Some(cancel))?;
                for (name, item) in fields(&record)? {
                    check_cancel(Some(cancel))?;
                    if snapshot(py, &game.join(name))? != item["old"] {
                        return Err(invalid(
                            "Root destination changed immediately before replacement.",
                        ));
                    }
                    if !matches(&item["old"], &item["new"]) {
                        copy(
                            py,
                            &PathBuf::from(text(&before["f4se"], "stage")).join(name),
                            &game.join(name),
                            &identifier,
                            &item["new"],
                        )?;
                    }
                    checkpoint(py, "installed-file")?;
                }
                let updated = with_database(&library.getattr("_db")?, |db, native| {
                    native
                        .connection()?
                        .execute_batch("BEGIN IMMEDIATE")
                        .map_err(crate::database::error)?;
                    if digest(py, &dataclass(&target_lookup(library, &id, Some(db))?)?)?
                        != target_signature
                        || signature(py, &game.join("Fallout4.exe"))? != game_signature
                    {
                        return Err(invalid(
                            "Game/target changed before root setup publication.",
                        ));
                    }
                    validate_ready(&target, cancel, &target_root(library, &id)?)?;
                    let mut after = before.clone();
                    after["binary"] = json!(game.join("f4se_loader.exe").to_string_lossy());
                    after["f4se"]["route"] = json!("game-root");
                    after["f4se"]["root_setup"] = json!(identifier);
                    let mut files = Vec::new();
                    for name in fields(&record)?.keys() {
                        files.push(json!([
                            name,
                            game.join(name).to_string_lossy(),
                            signature(py, &game.join(name))?
                        ]));
                    }
                    after["f4se"]["root_files"] = json!(files);
                    after["f4se"]["reason"] = json!(
                        "F4SE bootstrap files use documented game-root placement with recoverable backups. Scripts stay target-owned/virtual; real game boot remains unverified."
                    );
                    record["after"] = after.clone();
                    write_record(py, &path, &record)?;
                    checkpoint(py, "before-publish")?;
                    check_cancel(Some(cancel))?;
                    library.call_method0("_runtime_write_guard")?;
                    native
                        .connection()?
                        .execute(
                            "UPDATE launch_targets SET record=? WHERE id=?",
                            rusqlite::params![after.to_string(), id],
                        )
                        .map_err(crate::database::error)?;
                    Ok(py
                        .import("creation_lib.mod_manager.launch_targets")?
                        .getattr("Target")?
                        .call((), Some(object(py, &after)?.cast::<PyDict>()?))?
                        .unbind())
                })?;
                published = true;
                record["state"] = json!("complete");
                write_record(py, &path, &record)?;
                *release = true;
                Ok(updated)
            })();
            match result {
                Err(error) if error.is_instance_of::<pyo3::exceptions::PyException>(py) => {
                    if published {
                        let raised = invalid(
                            "Root setup committed but finalization failed. Review recovery; originals and transaction ownership were retained.",
                        );
                        raised.set_cause(py, Some(error));
                        return Err(raised);
                    }
                    if let Err(rollback) = rollback(library, &path, &mut record, false, &json!({}))
                    {
                        if !rollback.is_instance_of::<pyo3::exceptions::PyException>(py) {
                            return Err(rollback);
                        }
                        let raised = invalid(format!(
                            "Setup failed and guarded rollback needs review: {}. Retained receipt: {}",
                            rollback.value(py).str()?.to_str()?,
                            path.display()
                        ));
                        raised.set_cause(py, Some(error));
                        return Err(raised);
                    }
                    *release = true;
                    Err(error)
                }
                other => other,
            }
        })
    })
}
fn target_lookup<'py>(
    library: &Bound<'py, PyAny>,
    id: &str,
    db: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    target(library, id, db)
}
#[pyfunction]
fn f4se_root_review_restore(
    library: &Bound<'_, PyAny>,
    id: &str,
    receipt_id: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    library.call_method0("_runtime_write_guard")?;
    stopped(py)?;
    let current = dataclass(&target(library, id, None)?)?;
    let eligible: Vec<_> = history(library, id)?
        .into_iter()
        .filter(|v| matches!(text(v, "state"), "pending" | "complete" | "restoring"))
        .collect();
    let receipt_id = if receipt_id.is_empty() {
        if eligible.len() != 1 {
            return Err(invalid(
                "Select the single active root setup receipt for restore/recovery.",
            ));
        }
        text(&eligible[0], "id")
    } else {
        receipt_id
    };
    let (_, record) = read_record(library, id, receipt_id)?;
    if !matches!(text(&record, "state"), "pending" | "complete" | "restoring") {
        return Err(invalid("This root setup was already restored/rolled back."));
    }
    let game = PathBuf::from(text(&record, "game"));
    let mut files = Vec::new();
    let mut temporaries = Vec::new();
    for (name, item) in fields(&record)? {
        check_cancel(Some(cancel))?;
        let actual = snapshot(py, &game.join(name))?;
        if !matches(&actual, &item["old"]) && !matches(&actual, &item["new"]) {
            return Err(invalid(format!(
                "{name} was changed outside this setup. Restore will not overwrite it."
            )));
        }
        files.push((name.to_owned(), actual, item["old"].clone()));
        let temp = temporary(&game, &record, name);
        let info = snapshot(py, &temp)?;
        if !info.is_null() {
            temporaries.push((
                temp.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                info,
            ));
        }
    }
    review_object(
        library,
        id,
        &current,
        &game,
        files,
        receipt_id,
        Some(&record),
        true,
        temporaries,
    )
}
#[pyfunction]
fn f4se_root_restore(
    library: &Bound<'_, PyAny>,
    plan: &Bound<'_, PyAny>,
    accept_root_writes: bool,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    if !accept_root_writes
        || !plan.getattr("restore")?.is_truthy()?
        || !plan
            .getattr("library_id")?
            .eq(library.getattr("library_id")?)?
    {
        return Err(invalid(
            "Acknowledge the reviewed game-root restore before applying it.",
        ));
    }
    library.call_method0("_runtime_write_guard")?;
    stopped(py)?;
    let id = plan.getattr("target_id")?.extract::<String>()?;
    let receipt = plan.getattr("receipt_id")?.extract::<String>()?;
    with_context(&library.getattr("owned")?.call_method0("lock")?, |_| {
        if !f4se_root_review_restore(library, &id, &receipt, cancel)?
            .bind(py)
            .eq(plan)?
        {
            return Err(invalid(
                "Root restore inputs changed after review; no files were restored.",
            ));
        }
        let (path, mut record) = read_record(library, &id, &receipt)?;
        let identity = record.clone();
        let game = PathBuf::from(text(&record, "game"));
        game_lock(&game, &identity, true, |release| {
            let preflight =
                checkpoint(py, "before-restore").and_then(|_| check_cancel(Some(cancel)));
            if let Err(error) = preflight {
                if error.is_instance_of::<pyo3::exceptions::PyException>(py) {
                    *release = record["state"] == "complete";
                }
                return Err(error);
            }
            let approved = value(
                &py.import("builtins")?
                    .getattr("dict")?
                    .call1((plan.getattr("temporaries")?,))?,
            )?;
            let completed = record["state"] == "complete";
            rollback(library, &path, &mut record, completed, &approved)?;
            *release = true;
            Ok(())
        })
    })?;
    Ok(target(library, &id, None)?.unbind())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(f4se_root_snapshot, m)?)?;
    m.add_function(wrap_pyfunction!(f4se_root_base, m)?)?;
    m.add_function(wrap_pyfunction!(f4se_root_read, m)?)?;
    m.add_function(wrap_pyfunction!(f4se_root_history, m)?)?;
    m.add_function(wrap_pyfunction!(f4se_root_review, m)?)?;
    m.add_function(wrap_pyfunction!(f4se_root_copy, m)?)?;
    m.add_function(wrap_pyfunction!(f4se_root_apply, m)?)?;
    m.add_function(wrap_pyfunction!(f4se_root_review_restore, m)?)?;
    m.add_function(wrap_pyfunction!(f4se_root_restore, m)?)?;
    Ok(())
}
