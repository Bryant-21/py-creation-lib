use crate::{
    check_cancel, encoding, executables::text, history_prune::py_path, invalid,
    python_io::with_context, reject_links, transactions::transaction_key,
};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict},
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
fn library_root(library: &Bound<'_, PyAny>) -> PyResult<PathBuf> {
    library.getattr("root")?.extract()
}
fn root(library: &Bound<'_, PyAny>, profile: &str) -> PyResult<PathBuf> {
    library.call_method1("profile_snapshot", (profile,))?;
    let path = library_root(library)?
        .join("runtime")
        .join("profiles")
        .join(transaction_key(profile)?)
        .join("overwrite");
    reject_links(&path)?;
    Ok(path)
}
fn snapshot(
    library: &Bound<'_, PyAny>,
    path: &Path,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<(Value, Vec<String>)> {
    reject_links(path)?;
    if !path.is_dir() {
        return Ok((json!({}), Vec::new()));
    }
    crate::history_prune::snapshot(library.py(), path, cancel, progress)
}
fn verify(
    library: &Bound<'_, PyAny>,
    path: &Path,
    plan: &Value,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<(Value, Vec<String>)> {
    let (files, dirs) = snapshot(library, path, cancel, progress)?;
    if encoding::digest(&json!([files, dirs]), true, false) != text(plan, "token") {
        return Err(invalid(
            "Owned overwrite or backup changed after review. Review it again; no content was replaced.",
        ));
    }
    Ok((files, dirs))
}
fn paths(library: &Bound<'_, PyAny>, record: &Value) -> PyResult<[PathBuf; 4]> {
    let token = text(record, "token");
    if record["version"] != 1
        || text(record, "library") != library.getattr("library_id")?.extract::<String>()?
        || !["clear", "restore"].contains(&text(record, "kind"))
        || token.len() != 64
        || !token
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
    {
        return Err(invalid(
            "Invalid or foreign overwrite journal; preserve it for manual review.",
        ));
    }
    let profile = transaction_key(text(record, "profile"))?;
    let id = transaction_key(text(record, "id"))?;
    let base = library_root(library)?
        .join(".overwrite-history")
        .join(&profile)
        .join(&id);
    let result = [
        root(library, &profile)?,
        base.join("payload"),
        base.join("receipt.json"),
        library_root(library)?
            .join(".overwrite-transactions")
            .join(format!("{id}.json")),
    ];
    for path in &result {
        reject_links(path)?;
    }
    Ok(result)
}
fn write(py: Python<'_>, path: &Path, record: &Value) -> PyResult<()> {
    std::fs::create_dir_all(path.parent().unwrap())?;
    reject_links(path)?;
    let temporary = path.with_file_name(format!(
        "{}.{}.tmp",
        path.file_name().unwrap().to_string_lossy(),
        uuid::Uuid::new_v4().simple()
    ));
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
    py_path(py, &temporary)?.call_method1("replace", (path.to_path_buf(),))?;
    Ok(())
}
fn record(py: Python<'_>, path: &Path) -> PyResult<Value> {
    let value = crate::paths::metadata_read_text(&py_path(py, path)?, 8192)
        .and_then(|v| encoding::parse(&v))
        .map_err(|_| invalid("Unreadable owned overwrite receipt; no content was replaced."))?;
    if !value.is_object() {
        return Err(invalid("Invalid owned overwrite receipt."));
    }
    Ok(value)
}
fn history(library: &Bound<'_, PyAny>, profile: &str) -> PyResult<Vec<Value>> {
    root(library, profile)?;
    let base = library_root(library)?
        .join(".overwrite-history")
        .join(transaction_key(profile)?);
    reject_links(&base)?;
    if !base.is_dir() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for (count, folder) in std::fs::read_dir(base)?.enumerate() {
        let folder = folder?.path();
        reject_links(&folder)?;
        if count >= 2000 {
            return Err(invalid(
                "Overwrite history exceeds its 2,000-record inspection limit.",
            ));
        }
        let record = record(library.py(), &folder.join("receipt.json"))?;
        if paths(library, &record)?[2].parent() != Some(folder.as_path())
            || record["profile"] != profile
        {
            return Err(invalid(
                "Overwrite backup identity differs from its owned folder.",
            ));
        }
        if ["complete", "restored"].contains(&text(&record, "state")) {
            result.push(record);
        }
    }
    result.sort_by(|a, b| text(b, "created").cmp(text(a, "created")));
    Ok(result)
}
fn finish(library: &Bound<'_, PyAny>, record: &Value) -> PyResult<()> {
    let [overwrite, payload, receipt, journal] = paths(library, record)?;
    let clear = record["kind"] == "clear";
    let (placed, source) = if clear {
        (&payload, &overwrite)
    } else {
        (&overwrite, &payload)
    };
    let py = library.py();
    if placed.is_dir() && (clear || !source.exists()) {
        let (files, dirs) = snapshot(library, placed, py.None().bind(py), py.None().bind(py))?;
        if encoding::digest(&json!([files, dirs]), true, false) != text(record, "token") {
            return Err(invalid(
                "Interrupted overwrite operation has changed placed content. Both locations were preserved for manual review.",
            ));
        }
        std::fs::create_dir_all(&overwrite)?;
        let mut saved = record.clone();
        saved["state"] = json!(if clear { "complete" } else { "restored" });
        write(py, &receipt, &saved)?;
    } else if source.is_dir() {
        std::fs::create_dir_all(&overwrite)?;
        if clear {
            let mut saved = record.clone();
            saved["state"] = json!("aborted");
            write(py, &receipt, &saved)?;
        }
    } else {
        return Err(invalid(
            "Interrupted overwrite operation has no intact source or destination. Manual review is required.",
        ));
    }
    std::fs::remove_file(journal)?;
    Ok(())
}
fn recover(library: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    library.call_method0("_runtime_write_guard")?;
    let base = library_root(library)?.join(".overwrite-transactions");
    reject_links(&base)?;
    if !base.is_dir() {
        return Ok(Vec::new());
    }
    let count = with_context(&library.getattr("owned")?.call_method0("lock")?, |_| {
        let mut count = 0;
        for path in std::fs::read_dir(base)? {
            let path = path?.path();
            reject_links(&path)?;
            if path.extension().is_none_or(|v| v != "json") {
                continue;
            }
            count += 1;
            if count > 100 {
                return Err(invalid(
                    "Overwrite recovery exceeds its 100-operation limit.",
                ));
            }
            let record = record(library.py(), &path)?;
            if paths(library, &record)?[3] != path {
                return Err(invalid(
                    "Overwrite journal filename does not match its identity.",
                ));
            }
            finish(library, &record)?;
        }
        Ok(count)
    })?;
    Ok(if count > 0 {
        vec!["Recovered reviewed owned overwrite changes.".into()]
    } else {
        Vec::new()
    })
}
fn review_restore(
    library: &Bound<'_, PyAny>,
    profile: &str,
    id: &str,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<Value> {
    let record = history(library, profile)?
        .into_iter()
        .find(|v| v["id"] == id && v["state"] == "complete")
        .ok_or_else(|| invalid("Select an intact owned overwrite backup."))?;
    let plan = json!({"profile_id":profile,"token":record["token"],"files":record["files"],"bytes":record["bytes"],"directories":[]});
    verify(
        library,
        &paths(library, &record)?[1],
        &plan,
        cancel,
        progress,
    )?;
    Ok(plan)
}
#[pyfunction]
fn overwrite_root(library: &Bound<'_, PyAny>, profile: &str) -> PyResult<PathBuf> {
    root(library, profile)
}
#[pyfunction]
fn overwrite_paths(library: &Bound<'_, PyAny>, record: &str) -> PyResult<Vec<PathBuf>> {
    Ok(paths(library, &encoding::parse(record)?)?.to_vec())
}
#[pyfunction]
fn overwrite_verify(
    library: &Bound<'_, PyAny>,
    path: PathBuf,
    plan: &str,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let (files, dirs) = verify(library, &path, &encoding::parse(plan)?, cancel, progress)?;
    Ok(json!([files, dirs]).to_string())
}
#[pyfunction]
fn overwrite_operation(
    library: &Bound<'_, PyAny>,
    action: &str,
    profile: &str,
    id: &str,
    plan: &str,
    name: &str,
    checkpoint: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let py = library.py();
    if action == "history" {
        return Ok(json!(history(library, profile)?).to_string());
    }
    if action == "recover" {
        return Ok(json!(recover(library)?).to_string());
    }
    if action == "review_restore" {
        return Ok(review_restore(library, profile, id, cancel, progress)?.to_string());
    }
    library.call_method0("_runtime_write_guard")?;
    if action == "review" {
        return with_context(&library.getattr("owned")?.call_method0("lock")?, |_| {
            let (files, dirs) = snapshot(library, &root(library, profile)?, cancel, progress)?;
            let bytes: u64 = files
                .as_object()
                .unwrap()
                .values()
                .map(|v| v["size"].as_u64().unwrap())
                .sum();
            Ok(json!({"profile_id":profile,"token":encoding::digest(&json!([files,dirs]),true,false),"files":files.as_object().unwrap().len(),"bytes":bytes,"directories":dirs}).to_string())
        });
    }
    let plan = encoding::parse(plan)?;
    let profile = text(&plan, "profile_id");
    if action == "copy_mod" {
        let path = root(library, profile)?;
        let (files, dirs) = verify(library, &path, &plan, cancel, progress)?;
        let module = py.import("creation_lib.mod_manager.package")?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("inspect_installer", false)?;
        let package = module.getattr("Package")?.call(
            (&path, cancel, module.getattr("OWNED_LIMITS")?),
            Some(&kwargs),
        )?;
        let result = (|| -> PyResult<String> {
            let mapping = PyDict::new(py);
            for name in package
                .getattr("files")?
                .call_method0("values")?
                .try_iter()?
            {
                let name = name?;
                mapping.set_item(&name, &name)?;
            }
            let kwargs = PyDict::new(py);
            kwargs.set_item(
                "expected_manifest_token",
                encoding::digest(&files, true, false),
            )?;
            kwargs.set_item("restore_directories", dirs)?;
            let id = library
                .getattr("owned")?
                .call_method(
                    "install",
                    (&package, mapping, name, profile, cancel, progress),
                    Some(&kwargs),
                )?
                .extract::<String>()?;
            Ok(json!(id).to_string())
        })();
        package.call_method0("close")?;
        return result;
    }
    recover(library)?;
    with_context(&library.getattr("owned")?.call_method0("lock")?, |_| {
        if action == "clear" {
            let overwrite = root(library, profile)?;
            verify(library, &overwrite, &plan, cancel, progress)?;
            if plan["files"].as_u64().unwrap_or(0) == 0
                && plan["directories"].as_array().is_none_or(Vec::is_empty)
            {
                return Err(invalid("Owned runtime overwrite is empty."));
            }
            if history(library, profile)?.len() >= 1000 {
                return Err(invalid(
                    "Retained overwrite history has reached its 1,000-backup limit.",
                ));
            }
            let datetime = py.import("datetime")?;
            let created = datetime
                .getattr("datetime")?
                .call_method1("now", (datetime.getattr("UTC")?,))?
                .call_method0("isoformat")?
                .extract::<String>()?;
            let record = json!({"version":1,"library":library.getattr("library_id")?.extract::<String>()?,"profile":profile,"id":uuid::Uuid::new_v4().simple().to_string(),"kind":"clear","token":plan["token"],"files":plan["files"],"bytes":plan["bytes"],"created":created,"state":"pending"});
            let [_, payload, receipt, journal] = paths(library, &record)?;
            check_cancel(Some(cancel))?;
            write(py, &receipt, &record)?;
            write(py, &journal, &record)?;
            checkpoint.call1(("journal",))?;
            let result = (|| -> PyResult<()> {
                check_cancel(Some(cancel))?;
                library.call_method0("_runtime_write_guard")?;
                py_path(py, &overwrite)?.call_method1("rename", (&payload,))?;
                checkpoint.call1(("placed",))?;
                finish(library, &record)
            })();
            if let Err(error) = result {
                if !payload.exists() {
                    finish(library, &record)?;
                    return Err(error);
                }
                return Err(invalid(
                    "Owned overwrite was retained in backup, but finalization was interrupted. Reopen this library to recover it.",
                ));
            }
            Ok(record["id"].to_string())
        } else {
            let mut record = history(library, profile)?
                .into_iter()
                .find(|v| v["id"] == id && v["state"] == "complete" && v["token"] == plan["token"])
                .ok_or_else(|| {
                    invalid("Overwrite backup changed or was restored already. Review it again.")
                })?;
            let [overwrite, payload, _, journal] = paths(library, &record)?;
            verify(library, &payload, &plan, cancel, progress)?;
            if overwrite.exists()
                && (!overwrite.is_dir() || std::fs::read_dir(&overwrite)?.next().is_some())
            {
                return Err(invalid(
                    "Owned overwrite must be empty before restoration. Back up current output first; no folders are merged.",
                ));
            }
            record["kind"] = json!("restore");
            record["state"] = json!("pending");
            write(py, &journal, &record)?;
            let result = (|| -> PyResult<()> {
                check_cancel(Some(cancel))?;
                library.call_method0("_runtime_write_guard")?;
                if overwrite.exists() {
                    std::fs::remove_dir(&overwrite)?;
                }
                py_path(py, &payload)?.call_method1("rename", (&overwrite,))?;
                checkpoint.call1(("restored",))?;
                finish(library, &record)
            })();
            if let Err(error) = result {
                if payload.exists() {
                    finish(library, &record)?;
                    return Err(error);
                }
                return Err(invalid(
                    "Owned overwrite was restored, but finalization was interrupted. Reopen this library to recover it.",
                ));
            }
            Ok("null".into())
        }
    })
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(overwrite_root, m)?)?;
    m.add_function(wrap_pyfunction!(overwrite_paths, m)?)?;
    m.add_function(wrap_pyfunction!(overwrite_verify, m)?)?;
    m.add_function(wrap_pyfunction!(overwrite_operation, m)?)?;
    Ok(())
}
