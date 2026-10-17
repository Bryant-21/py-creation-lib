use crate::{
    check_cancel,
    executables::text,
    history_prune::{py_path, stamp},
    invalid, reject_links, relative_path,
    transactions::transaction_key,
};
use pyo3::prelude::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::PathBuf};
use unicode_casefold::UnicodeCaseFold;
fn fold(value: &str) -> String {
    value.case_fold().collect()
}
#[pyfunction]
fn save_backup_root(library: &Bound<'_, PyAny>, profile: &str, id: &str) -> PyResult<PathBuf> {
    let path = library
        .getattr("root")?
        .extract::<PathBuf>()?
        .join(".save-history")
        .join(transaction_key(profile)?)
        .join(transaction_key(id)?);
    reject_links(&path)?;
    Ok(path)
}
fn read(library: &Bound<'_, PyAny>, profile: &str, id: &str) -> PyResult<Value> {
    let py = library.py();
    let directory = save_backup_root(library, profile, id)?;
    let receipt = directory.join("receipt.json");
    reject_links(&receipt)?;
    if !receipt.is_file() {
        return Err(invalid("Save backup has no ordinary owned receipt."));
    }
    let receipt_py = py_path(py, &receipt)?;
    let before = stamp(&receipt_py.call_method0("stat")?)?;
    let data =
        crate::python_io::with_context(&receipt_py.call_method1("open", ("rb",))?, |stream| {
            stream.call_method1("read", (65537,))?.extract::<Vec<u8>>()
        })?;
    let after = stamp(&receipt_py.call_method0("stat")?)?;
    reject_links(&receipt)?;
    if data.len() > 65536 || (before.2, before.3, before.1) != (after.2, after.3, after.1) {
        return Err(invalid(
            "Save backup receipt is oversized or changed while reading.",
        ));
    }
    let inspected = (|| -> PyResult<Value> {
        let record: Value = serde_json::from_slice(&data).map_err(|e| invalid(e.to_string()))?;
        if record["version"] != 1
            || record["profile"] != profile
            || text(&record, "game").is_empty()
        {
            return Err(invalid("ownership or game"));
        }
        let files = record["files"]
            .as_array()
            .filter(|v| (1..=2).contains(&v.len()))
            .ok_or_else(|| invalid("file count"))?;
        let mut names = Vec::new();
        let mut sizes = Vec::new();
        let mut hashes = Vec::new();
        for item in files {
            let name = relative_path(text(item, "name"), false)?;
            let size = item["size"]
                .as_u64()
                .filter(|v| *v <= 2 * 1024 * 1024 * 1024)
                .ok_or_else(|| invalid("fingerprint"))?;
            let hash = text(item, "checksum");
            if name.contains('/')
                || hash.len() != 64
                || !hash
                    .bytes()
                    .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
            {
                return Err(invalid("fingerprint"));
            }
            names.push(name);
            sizes.push(size);
            hashes.push(hash.to_string());
        }
        let save = PathBuf::from(&names[0]);
        let extension = fold(save.extension().and_then(|v| v.to_str()).unwrap_or(""));
        if !["fos", "ess", "sfs"].contains(&extension.as_str()) {
            return Err(invalid("save extension"));
        }
        if names.len() == 2
            && (extension == "sfs"
                || fold(&names[1])
                    != fold(
                        &save
                            .with_extension(if extension == "fos" { "f4se" } else { "skse" })
                            .to_string_lossy(),
                    ))
        {
            return Err(invalid("companion identity"));
        }
        let mut allowed = names.iter().map(|v| fold(v)).collect::<BTreeSet<_>>();
        allowed.insert("receipt.json".into());
        let mut seen = BTreeSet::new();
        for (count, entry) in std::fs::read_dir(&directory)?.enumerate() {
            let entry = entry?;
            let name = fold(&entry.file_name().to_string_lossy());
            if count >= 4 || !allowed.contains(&name) || !seen.insert(name) {
                return Err(invalid("unexpected or ambiguous bundle member"));
            }
            reject_links(&entry.path())?;
            if !entry.file_type()?.is_file() {
                return Err(invalid("non-file bundle member"));
            }
        }
        if seen != allowed {
            return Err(invalid("missing bundle member"));
        }
        Ok(
            json!({"id":id,"profile":profile,"game":record["game"],"names":names,"sizes":sizes,"checksums":hashes,"modified":before.3,"token":format!("{:x}",Sha256::digest(&data))}),
        )
    })();
    inspected.map_err(|error| {
        invalid(format!(
            "Save backup receipt is invalid: {}.",
            error
                .value(py)
                .str()
                .map(|v| v.to_string())
                .unwrap_or_else(|_| error.to_string())
        ))
    })
}
#[pyfunction]
fn save_backup_read(library: &Bound<'_, PyAny>, profile: &str, id: &str) -> PyResult<String> {
    Ok(read(library, profile, id)?.to_string())
}
#[pyfunction]
fn save_backup_inventory(
    library: &Bound<'_, PyAny>,
    profile: &str,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let directory = library
        .getattr("root")?
        .extract::<PathBuf>()?
        .join(".save-history")
        .join(transaction_key(profile)?);
    reject_links(&directory)?;
    let mut result = Vec::new();
    if directory.exists() {
        for (count, entry) in std::fs::read_dir(directory)?.enumerate() {
            check_cancel(Some(cancel))?;
            if count >= 1000 {
                return Err(invalid(
                    "Save backup inventory exceeds its 1,000-bundle limit.",
                ));
            }
            let entry = entry?;
            reject_links(&entry.path())?;
            if !entry.file_type()?.is_dir() {
                return Err(invalid("Unexpected member in owned save backup history."));
            }
            result.push(read(
                library,
                profile,
                &transaction_key(&entry.file_name().to_string_lossy())?,
            )?);
        }
    }
    check_cancel(Some(cancel))?;
    if !progress.is_none() {
        progress.call1((
            1,
            format!("Read {} owned save backup receipts", result.len()),
        ))?;
    }
    result.sort_by(|a, b| {
        b["modified"]
            .as_i64()
            .cmp(&a["modified"].as_i64())
            .then_with(|| text(a, "id").cmp(text(b, "id")))
    });
    Ok(json!(result).to_string())
}
fn review<'py>(
    library: &Bound<'py, PyAny>,
    profile: &str,
    id: &str,
    target: &str,
    cancel: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = library.py();
    let backup = read(library, profile, id)?;
    let snapshot = library.call_method1("profile_snapshot", (target,))?;
    let game = snapshot
        .get_item(0)?
        .getattr("files")?
        .call_method1("get", ("_modbox_game", ""))?
        .extract::<String>()?;
    if crate::nexus::nexus_game_domain(&game) != text(&backup, "game") {
        return Err(invalid(
            "Save backup restores require a local profile for the same game. Compatibility is unverified.",
        ));
    }
    let directory = save_backup_root(library, profile, id)?;
    let transfer = py.import("creation_lib.mod_manager.save_transfer")?;
    let mut files = Vec::new();
    for (index, name) in backup["names"].as_array().unwrap().iter().enumerate() {
        let file = transfer.call_method1(
            "checksum",
            (
                py_path(py, &directory.join(name.as_str().unwrap()))?,
                cancel,
            ),
        )?;
        if file.getattr("size")?.extract::<u64>()? != backup["sizes"][index].as_u64().unwrap()
            || file.getattr("checksum")?.extract::<String>()?
                != backup["checksums"][index].as_str().unwrap()
        {
            return Err(invalid(
                "Retained save backup bytes no longer match its receipt.",
            ));
        }
        files.push(file);
    }
    if read(library, profile, id)? != backup {
        return Err(invalid("Save backup changed while reviewing its restore."));
    }
    let files = pyo3::types::PyTuple::new(py, files)?;
    transfer.call_method1(
        "_available",
        (transfer.call_method1("root", (library, target))?, &files),
    )?;
    check_cancel(Some(cancel))?;
    transfer.getattr("Plan")?.call1((
        profile,
        target,
        text(&backup, "game"),
        files,
        id,
        text(&backup, "token"),
    ))
}
#[pyfunction]
fn save_backup_review<'py>(
    library: &Bound<'py, PyAny>,
    profile: &str,
    id: &str,
    target: &str,
    cancel: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    review(library, profile, id, target, cancel)
}
#[pyfunction]
fn save_backup_validate(
    library: &Bound<'_, PyAny>,
    plan: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let target = plan.getattr("target_profile")?.extract::<String>()?;
    if target.is_empty() {
        return Err(invalid("Choose a local profile for backup restoration."));
    }
    let current = review(
        library,
        &plan.getattr("source_profile")?.extract::<String>()?,
        &plan.getattr("source_backup")?.extract::<String>()?,
        &target,
        cancel,
    )?;
    if !current.eq(plan)? {
        return Err(invalid(
            "Save backup changed after restore review. Review it again.",
        ));
    }
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(save_backup_root, m)?)?;
    m.add_function(wrap_pyfunction!(save_backup_read, m)?)?;
    m.add_function(wrap_pyfunction!(save_backup_inventory, m)?)?;
    m.add_function(wrap_pyfunction!(save_backup_review, m)?)?;
    m.add_function(wrap_pyfunction!(save_backup_validate, m)?)?;
    Ok(())
}
