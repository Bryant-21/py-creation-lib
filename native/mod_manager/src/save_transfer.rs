use crate::{
    check_cancel, encoding,
    executables::text,
    history_prune::{py_path, stamp},
    invalid,
    python_io::with_context,
    reject_links, relative_path,
    transactions::transaction_key,
};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use unicode_casefold::UnicodeCaseFold;
fn fold(value: &str) -> String {
    value.case_fold().collect()
}
fn library_root(library: &Bound<'_, PyAny>) -> PyResult<PathBuf> {
    library.getattr("root")?.extract()
}
pub fn root(library: &Bound<'_, PyAny>, profile: &str) -> PyResult<PathBuf> {
    Ok(library_root(library)?
        .join("runtime")
        .join("profiles")
        .join(transaction_key(profile)?)
        .join("saves"))
}
pub fn checksum(py: Python<'_>, path: &Path, cancel: &Bound<'_, PyAny>) -> PyResult<Value> {
    reject_links(path)?;
    let path_py = py_path(py, path)?;
    if !path.is_file() || stamp(&path_py.call_method0("stat")?)?.2 > 2 * 1024 * 1024 * 1024 {
        return Err(invalid(
            "Save must be an ordinary file no larger than 2 GiB.",
        ));
    }
    let before = stamp(&path_py.call_method0("stat")?)?;
    let hash = with_context(&path_py.call_method1("open", ("rb",))?, |stream| {
        let mut digest = Sha256::new();
        let mut bytes = 0;
        loop {
            let block = stream
                .call_method1("read", (1024 * 1024,))?
                .extract::<Vec<u8>>()?;
            if block.is_empty() {
                break;
            }
            check_cancel(Some(cancel))?;
            bytes += block.len() as u64;
            if bytes > before.2 {
                return Err(invalid("Save grew beyond its reviewed size while reading."));
            }
            digest.update(block);
        }
        Ok(format!("{:x}", digest.finalize()))
    })?;
    check_cancel(Some(cancel))?;
    let after = stamp(&path_py.call_method0("stat")?)?;
    if (before.2, before.3) != (after.2, after.3) {
        return Err(invalid("Save changed while reviewing its copy."));
    }
    Ok(
        json!({"name":relative_path(&path.file_name().unwrap_or_default().to_string_lossy(),false)?,"source":path.to_string_lossy(),"size":before.2,"modified":before.3,"checksum":hash}),
    )
}
fn profile_game(library: &Bound<'_, PyAny>, id: &str) -> PyResult<String> {
    Ok(crate::nexus::nexus_game_domain(
        &library
            .call_method1("profile_snapshot", (id,))?
            .get_item(0)?
            .getattr("files")?
            .call_method1("get", ("_modbox_game", ""))?
            .extract::<String>()?,
    ))
}
fn profiles(library: &Bound<'_, PyAny>, plan: &Value) -> PyResult<()> {
    if text(plan, "source_backup").is_empty()
        && profile_game(library, text(plan, "source_profile"))? != text(plan, "game")
    {
        return Err(invalid("Source profile game changed after save review."));
    }
    let target = text(plan, "target_profile");
    if !target.is_empty() {
        if profile_game(library, target)? != text(plan, "game") {
            return Err(invalid(
                "Save copies require profiles for the same game. Compatibility remains unverified.",
            ));
        }
        if target == text(plan, "source_profile")
            && text(plan, "source_backup").is_empty()
            && text(plan, "source_folder").is_empty()
        {
            return Err(invalid("Choose another local profile for a save copy."));
        }
    }
    Ok(())
}
fn folder_identity(py: Python<'_>, path: &Path, expected: &Value) -> PyResult<()> {
    reject_links(path)?;
    let info = py_path(py, path)?
        .call_method0("lstat")
        .map_err(|_| invalid("Source save folder became unavailable. Refresh it."))?;
    let stamp = stamp(&info)?;
    let mode = info.getattr("st_mode")?.extract::<u32>()?;
    if mode & 0xf000 != 0x4000 || json!([stamp.0, stamp.1]) != *expected {
        return Err(invalid(
            "Source save folder changed identity after inventory. Refresh it.",
        ));
    }
    Ok(())
}
fn source_root(library: &Bound<'_, PyAny>, plan: &Value) -> PyResult<PathBuf> {
    let py = library.py();
    let backup = text(plan, "source_backup");
    let folder = text(plan, "source_folder");
    if !backup.is_empty() {
        if !folder.is_empty() {
            return Err(invalid(
                "Save copy cannot combine backup and external source identities.",
            ));
        }
        return Ok(library_root(library)?
            .join(".save-history")
            .join(transaction_key(text(plan, "source_profile"))?)
            .join(transaction_key(backup)?));
    }
    if !folder.is_empty() {
        let path = PathBuf::from(folder);
        let object = py_path(py, &path)?;
        if !path.is_absolute()
            || !object.eq(object.call_method0("absolute")?)?
            || !py
                .import("creation_lib.mod_manager.mo2")?
                .call_method1("local_mod_target", (&object,))?
                .is_truthy()?
        {
            return Err(invalid(
                "Source save folder must be the reviewed absolute path.",
            ));
        }
        folder_identity(py, &path, &plan["folder_identity"])?;
        return Ok(path);
    }
    root(library, text(plan, "source_profile"))
}
fn companion_set(directory: &Path, files: &[Value]) -> PyResult<()> {
    let save = PathBuf::from(text(&files[0], "name"));
    let extension = fold(&save.extension().unwrap_or_default().to_string_lossy());
    if extension == "sfs" {
        return Ok(());
    }
    let expected_name = fold(
        &save
            .with_extension(if extension == "fos" { "f4se" } else { "skse" })
            .to_string_lossy(),
    );
    let expected = files
        .iter()
        .skip(1)
        .map(|v| fold(text(v, "name")))
        .collect::<BTreeSet<_>>();
    let mut found = Vec::new();
    if directory.is_dir() {
        for (count, entry) in std::fs::read_dir(directory)?.enumerate() {
            if count >= 100_000 {
                return Err(invalid("Save source exceeds its directory limit."));
            }
            let name = fold(&entry?.file_name().to_string_lossy());
            if name == expected_name {
                found.push(name);
            }
        }
    }
    if found.len() != expected.len() || found.into_iter().collect::<BTreeSet<_>>() != expected {
        return Err(invalid(
            "Save companion appeared, disappeared or became ambiguous after review. Refresh it.",
        ));
    }
    Ok(())
}
pub fn available(destination: &Path, files: &[Value]) -> PyResult<()> {
    reject_links(destination)?;
    let names = files
        .iter()
        .map(|v| fold(text(v, "name")))
        .collect::<BTreeSet<_>>();
    if destination.exists() {
        for (count, entry) in std::fs::read_dir(destination)?.enumerate() {
            if count >= 100_000 {
                return Err(invalid("Save destination exceeds its directory limit."));
            }
            if names.contains(&fold(&entry?.file_name().to_string_lossy())) {
                return Err(invalid(
                    "A destination save or companion already exists. No file was replaced.",
                ));
            }
        }
    }
    Ok(())
}
fn write(py: Python<'_>, path: &Path, record: &Value) -> PyResult<()> {
    reject_links(path)?;
    let temporary = path.with_extension("tmp");
    reject_links(&temporary)?;
    with_context(
        &py_path(py, &temporary)?.call_method1("open", ("xb",))?,
        |stream| {
            stream.call_method1(
                "write",
                (PyBytes::new(
                    py,
                    encoding::encode(record, true, true).as_bytes(),
                ),),
            )?;
            stream.call_method0("flush")?;
            py.import("os")?
                .call_method1("fsync", (stream.call_method0("fileno")?,))?;
            Ok(())
        },
    )?;
    py_path(py, &temporary)?.call_method1("replace", (py_path(py, path)?,))?;
    Ok(())
}
fn paths(library: &Bound<'_, PyAny>, record: &Value) -> PyResult<[PathBuf; 3]> {
    if record["version"] != 1
        || text(record, "library") != library.getattr("library_id")?.extract::<String>()?
    {
        return Err(invalid(
            "Foreign save copy journal; manual review is required.",
        ));
    }
    let id = transaction_key(text(record, "id"))?;
    let target = transaction_key(text(record, "target"))?;
    let files = record["files"]
        .as_array()
        .filter(|v| (1..=2).contains(&v.len()))
        .ok_or_else(|| invalid("Invalid save copy journal file count."))?;
    let mut names = BTreeSet::new();
    for item in files {
        let name = relative_path(text(item, "name"), false)?;
        let extension = fold(
            &Path::new(&name)
                .extension()
                .unwrap_or_default()
                .to_string_lossy(),
        );
        if name.contains('/')
            || !["fos", "f4se", "ess", "skse", "sfs"].contains(&extension.as_str())
            || !names.insert(fold(&name))
        {
            return Err(invalid("Invalid save copy journal filename."));
        }
        if item["size"]
            .as_u64()
            .is_none_or(|v| v > 2 * 1024 * 1024 * 1024)
            || text(item, "checksum").len() != 64
            || !text(item, "checksum")
                .bytes()
                .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
        {
            return Err(invalid("Invalid save copy journal fingerprint."));
        }
    }
    let base = library_root(library)?;
    let result = [
        base.join(".save-transfers").join(format!("{id}.json")),
        base.join(".staging").join(format!("save-{id}")),
        root(library, &target)?,
    ];
    for path in &result {
        reject_links(path)?;
    }
    Ok(result)
}
fn matches(py: Python<'_>, path: &Path, item: &Value) -> PyResult<bool> {
    Ok(path.is_file()
        && std::fs::metadata(path)?.len() == item["size"].as_u64().unwrap_or(u64::MAX)
        && checksum(py, path, py.None().bind(py))?["checksum"] == item["checksum"])
}
fn place(
    library: &Bound<'_, PyAny>,
    record: &Value,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = library.py();
    let [journal, stage, destination] = paths(library, record)?;
    std::fs::create_dir_all(&destination)?;
    for item in record["files"].as_array().unwrap() {
        let source = stage.join(text(item, "name"));
        let target = destination.join(text(item, "name"));
        reject_links(&target)?;
        if target.exists() {
            if source.exists() || !matches(py, &target, item)? {
                return Err(invalid(
                    "Save recovery destination is occupied or changed; files and journal were retained for review.",
                ));
            }
        } else {
            available(&destination, &[item.clone()])?;
            if !matches(py, &source, item)? {
                return Err(invalid(
                    "Staged save is missing or changed; recovery requires manual review.",
                ));
            }
            #[cfg(windows)]
            py_path(py, &source)?.call_method1("rename", (&target,))?;
            #[cfg(not(windows))]
            {
                std::fs::hard_link(&source, &target)?;
                std::fs::remove_file(source)?;
            }
        }
        checkpoint.call1(("placed",))?;
    }
    if stage.exists() {
        std::fs::remove_dir(stage)?;
    }
    std::fs::remove_file(journal)?;
    Ok(())
}
fn active(library: &Bound<'_, PyAny>, sessions: &Bound<'_, PyAny>) -> PyResult<bool> {
    sessions.call1((library.getattr("root")?,))?.is_truthy()
}
fn recover(
    library: &Bound<'_, PyAny>,
    sessions: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<Vec<String>> {
    if active(library, sessions)? {
        return Ok(Vec::new());
    }
    let directory = library_root(library)?.join(".save-transfers");
    reject_links(&directory)?;
    if !directory.exists() {
        return Ok(Vec::new());
    }
    with_context(&library.getattr("owned")?.call_method0("lock")?, |_| {
        let mut messages = Vec::new();
        for (count, path) in std::fs::read_dir(&directory)?.enumerate() {
            if count >= 1000 {
                return Err(invalid("Save recovery exceeds its 1,000-journal limit."));
            }
            let path = path?.path();
            reject_links(&path)?;
            if path.extension().is_none_or(|v| v != "json") {
                continue;
            }
            if std::fs::metadata(&path)?.len() > 4096 {
                return Err(invalid(
                    "Oversized save copy journal; manual review is required.",
                ));
            }
            let result = (|| -> PyResult<()> {
                let record = encoding::parse(&std::fs::read_to_string(&path)?)?;
                if paths(library, &record)?[0] != path {
                    return Err(invalid(
                        "Save recovery journal identity differs from its filename.",
                    ));
                }
                library.call_method1("profile_snapshot", (text(&record, "target"),))?;
                place(library, &record, checkpoint)
            })();
            if let Err(error) = result {
                return Err(invalid(format!(
                    "Save copy recovery stopped without replacing files: {}",
                    error.value(library.py()).str()?
                )));
            }
            messages.push("Completed interrupted copy of an owned save and its companion.".into());
        }
        Ok(messages)
    })
}

#[pyfunction]
fn save_transfer_root(library: &Bound<'_, PyAny>, profile: &str) -> PyResult<PathBuf> {
    root(library, profile)
}
#[pyfunction]
fn save_transfer_checksum(
    py: Python<'_>,
    path: PathBuf,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<String> {
    Ok(checksum(py, &path, cancel)?.to_string())
}
#[pyfunction]
fn save_transfer_available(destination: PathBuf, files: &str) -> PyResult<()> {
    available(
        &destination,
        encoding::parse(files)?
            .as_array()
            .ok_or_else(|| invalid("Invalid save copy files."))?,
    )
}
#[pyfunction]
fn save_transfer_review(
    library: &Bound<'_, PyAny>,
    index: &Bound<'_, PyAny>,
    path: &str,
    target: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let py = library.py();
    let mut selected = None;
    for file in index.getattr("files")?.try_iter()? {
        let file = file?;
        if file.getattr("path")?.extract::<String>()? == path {
            selected = Some(file);
            break;
        }
    }
    let external = index.getattr("external")?.is_truthy()?;
    let profile = index.getattr("profile_id")?.extract::<String>()?;
    let expected_root = if external {
        PathBuf::from(index.getattr("root")?.extract::<String>()?)
    } else {
        root(library, &profile)?
    };
    let source = PathBuf::from(path);
    if selected.is_none()
        || !py_path(py, source.parent().unwrap_or(Path::new("")))?
            .eq(py_path(py, &expected_root)?)?
    {
        return Err(invalid(
            "Refresh and select a save in this profile's displayed inventory.",
        ));
    }
    let save = selected.unwrap();
    let identity = encoding::parse(
        &py.import("json")?
            .call_method1("dumps", (index.getattr("identity")?,))?
            .extract::<String>()?,
    )?;
    if external {
        folder_identity(py, &expected_root, &identity)?;
    }
    let item = checksum(py, &source, cancel)?;
    if item["size"].as_u64().unwrap() != save.getattr("size")?.extract::<u64>()?
        || item["modified"].as_i64().unwrap() != save.getattr("modified")?.extract::<i64>()?
    {
        return Err(invalid(
            "Save changed after inventory; refresh it before copying.",
        ));
    }
    let mut files = vec![item];
    let companion = save.getattr("companion")?.extract::<String>()?;
    if !companion.is_empty() {
        let companion = PathBuf::from(companion);
        let suffix = if fold(&source.extension().unwrap_or_default().to_string_lossy()) == "fos" {
            "f4se"
        } else {
            "skse"
        };
        if !py_path(py, companion.parent().unwrap_or(Path::new("")))?
            .eq(py_path(py, source.parent().unwrap_or(Path::new("")))?)?
            || fold(&companion.file_name().unwrap_or_default().to_string_lossy())
                != fold(
                    &source
                        .with_extension(suffix)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                )
        {
            return Err(invalid(
                "Save companion identity does not match its owned save.",
            ));
        }
        files.push(checksum(py, &companion, cancel)?);
    }
    let plan = json!({"source_profile":profile,"target_profile":target,"game":index.getattr("game")?.extract::<String>()?,"files":files,"source_backup":"","backup_token":"","source_folder":if external{expected_root.to_string_lossy().into_owned()}else{String::new()},"folder_identity":if external{identity}else{Value::Null}});
    profiles(library, &plan)?;
    if !target.is_empty() {
        available(&root(library, target)?, plan["files"].as_array().unwrap())?;
    }
    check_cancel(Some(cancel))?;
    Ok(plan.to_string())
}
#[pyfunction]
fn save_transfer_recover(
    library: &Bound<'_, PyAny>,
    sessions: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<Vec<String>> {
    recover(library, sessions, checkpoint)
}
#[pyfunction]
fn save_transfer_apply(
    library: &Bound<'_, PyAny>,
    plan: &str,
    sessions: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let py = library.py();
    let plan = encoding::parse(plan)?;
    if active(library, sessions)? {
        return Err(invalid(
            "Close the native runtime before copying owned saves.",
        ));
    }
    recover(library, sessions, checkpoint)?;
    with_context(&library.getattr("owned")?.call_method0("lock")?, |_| {
        profiles(library, &plan)?;
        if !text(&plan, "source_backup").is_empty() {
            let transfer = py.import("creation_lib.mod_manager.save_transfer")?;
            let dto = transfer.call_method1(
                "_plan",
                (py.import("json")?
                    .call_method1("loads", (plan.to_string(),))?,),
            )?;
            py.import("creation_lib.mod_manager.save_backups")?
                .call_method1("validate", (library, dto, cancel))?;
        }
        let expected_root = source_root(library, &plan)?;
        let files = plan["files"]
            .as_array()
            .filter(|v| (1..=2).contains(&v.len()))
            .ok_or_else(|| invalid("Choose one save and its optional extender companion."))?;
        let mut names = BTreeSet::new();
        for item in files {
            let name = relative_path(text(item, "name"), false)?;
            let extension = fold(
                &Path::new(&name)
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy(),
            );
            if name.contains('/')
                || !names.insert(fold(&name))
                || !["fos", "f4se", "ess", "skse", "sfs"].contains(&extension.as_str())
            {
                return Err(invalid(
                    "Choose ordinary owned save and companion filenames.",
                ));
            }
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let stage = library_root(library)?
            .join(".staging")
            .join(format!("save-{id}"));
        reject_links(&stage)?;
        let target = text(&plan, "target_profile");
        if !target.is_empty() {
            available(&root(library, target)?, files)?;
        }
        std::fs::create_dir(&stage)?;
        let mut committed = false;
        let mut journal: Option<PathBuf> = None;
        let result = (|| -> PyResult<String> {
            companion_set(&expected_root, files)?;
            for item in files {
                let source = PathBuf::from(text(item, "source"));
                if !py_path(py, source.parent().unwrap_or(Path::new("")))?
                    .eq(py_path(py, &expected_root)?)?
                    || text(item, "name")
                        != relative_path(
                            &source.file_name().unwrap_or_default().to_string_lossy(),
                            false,
                        )?
                {
                    return Err(invalid("Only canonical owned saves can be copied."));
                }
                if checksum(py, &source, cancel)? != *item {
                    return Err(invalid("Save or companion changed after copy review."));
                }
                let copied = with_context(
                    &py_path(py, &source)?.call_method1("open", ("rb",))?,
                    |incoming| {
                        with_context(
                            &py_path(py, &stage.join(text(item, "name")))?
                                .call_method1("open", ("xb",))?,
                            |output| {
                                let mut digest = Sha256::new();
                                let mut copied = 0;
                                loop {
                                    let chunk = incoming
                                        .call_method1("read", (1024 * 1024,))?
                                        .extract::<Vec<u8>>()?;
                                    if chunk.is_empty() {
                                        break;
                                    }
                                    check_cancel(Some(cancel))?;
                                    copied += chunk.len() as u64;
                                    if copied > item["size"].as_u64().unwrap_or(0) {
                                        return Err(invalid(
                                            "Save grew beyond its reviewed size while staging.",
                                        ));
                                    }
                                    digest.update(&chunk);
                                    output.call_method1("write", (PyBytes::new(py, &chunk),))?;
                                }
                                output.call_method0("flush")?;
                                py.import("os")?
                                    .call_method1("fsync", (output.call_method0("fileno")?,))?;
                                Ok(format!("{:x}", digest.finalize()))
                            },
                        )
                    },
                )?;
                if copied != text(item, "checksum") || checksum(py, &source, cancel)? != *item {
                    return Err(invalid("Save changed while staging its copy."));
                }
                checkpoint.call1(("staged",))?;
                if !progress.is_none() {
                    progress.call1((py.None(), format!("Copied {}", text(item, "name"))))?;
                }
            }
            check_cancel(Some(cancel))?;
            source_root(library, &plan)?;
            companion_set(&expected_root, files)?;
            if active(library, sessions)? {
                return Err(invalid("A native runtime started during save copy review."));
            }
            if target.is_empty() {
                let destination = library_root(library)?
                    .join(".save-history")
                    .join(transaction_key(text(&plan, "source_profile"))?)
                    .join(&id);
                reject_links(&destination)?;
                std::fs::create_dir_all(destination.parent().unwrap())?;
                write(
                    py,
                    &stage.join("receipt.json"),
                    &json!({"version":1,"game":plan["game"],"profile":plan["source_profile"],"files":files}),
                )?;
                check_cancel(Some(cancel))?;
                py_path(py, &stage)?.call_method1("rename", (&destination,))?;
                committed = true;
                return Ok(destination.to_string_lossy().into_owned());
            }
            let record = json!({"version":1,"library":library.getattr("library_id")?.extract::<String>()?,"id":id,"target":target,"files":files.iter().map(|item|json!({"name":item["name"],"size":item["size"],"checksum":item["checksum"]})).collect::<Vec<_>>()});
            let path = paths(library, &record)?[0].clone();
            journal = Some(path.clone());
            let kwargs = PyDict::new(py);
            kwargs.set_item("exist_ok", true)?;
            py_path(py, path.parent().unwrap())?.call_method("mkdir", (), Some(&kwargs))?;
            write(py, &path, &record)?;
            place(library, &record, checkpoint)?;
            committed = true;
            Ok(root(library, target)?.to_string_lossy().into_owned())
        })();
        let pending = journal.as_ref().is_some_and(|v| v.exists());
        let result = match result {
            Err(error) if pending && error.is_instance_of::<pyo3::exceptions::PyException>(py) => {
                Err(invalid(
                    "Save copy placement was interrupted. Reopen the library to finish the owned pair; source saves and existing destination files were preserved.",
                ))
            }
            other => other,
        };
        if !committed && !pending && stage.exists() {
            for path in std::fs::read_dir(&stage)? {
                let path = path?.path();
                reject_links(&path)?;
                std::fs::remove_file(path)?;
            }
            std::fs::remove_dir(stage)?;
        }
        result
    })
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(save_transfer_root, m)?)?;
    m.add_function(wrap_pyfunction!(save_transfer_checksum, m)?)?;
    m.add_function(wrap_pyfunction!(save_transfer_available, m)?)?;
    m.add_function(wrap_pyfunction!(save_transfer_review, m)?)?;
    m.add_function(wrap_pyfunction!(save_transfer_recover, m)?)?;
    m.add_function(wrap_pyfunction!(save_transfer_apply, m)?)?;
    Ok(())
}
