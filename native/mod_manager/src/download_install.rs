use crate::{
    check_cancel, encoding, invalid,
    plugin_inventory::{object, value},
    reject_links,
    saves::fold,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};
fn base<'py>(library: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    if library.hasattr("base")? {
        library.getattr("base")
    } else {
        Ok(library.clone())
    }
}
fn target(library: &Bound<'_, PyAny>) -> PyResult<String> {
    if library.hasattr("target_id")? {
        library.getattr("target_id")?.extract()
    } else {
        Ok(String::new())
    }
}
fn context<'py>(
    library: &Bound<'py, PyAny>,
    profile: &str,
) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyAny>, Value, String)> {
    let snapshot = library.call_method1("profile_snapshot", (profile,))?;
    let rows = snapshot.get_item(1)?;
    let config = value(&snapshot.get_item(2)?)?;
    let logical = base(library)?.call_method1("rows", (profile,))?;
    let dataclasses = library.py().import("dataclasses")?;
    let row_values = |values: &Bound<'_, PyAny>| -> PyResult<Vec<Value>> {
        values
            .try_iter()?
            .map(|row| value(&dataclasses.call_method1("asdict", (row?,))?))
            .collect()
    };
    let mut settings = config.clone();
    settings
        .as_object_mut()
        .ok_or_else(|| invalid("Invalid profile configuration."))?
        .remove("download_inventory");
    let token = encoding::digest(
        &json!([
            value(&snapshot.get_item(0)?.getattr("files")?)?,
            row_values(&rows)?,
            row_values(&logical)?,
            settings
        ]),
        true,
        false,
    );
    Ok((rows, logical, config, token))
}
fn absolute(py: Python<'_>, path: &str) -> PyResult<PathBuf> {
    py.import("pathlib")?
        .getattr("Path")?
        .call1((path,))?
        .call_method0("absolute")?
        .extract()
}
fn text<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field].as_str().unwrap_or("")
}
fn record(
    library: &Bound<'_, PyAny>,
    config: &Value,
    path: &PathBuf,
    imported: bool,
) -> PyResult<Value> {
    let records = if imported {
        config
            .get("download_inventory")
            .cloned()
            .unwrap_or(json!([]))
    } else {
        value(&library.call_method0("downloads")?)?
    };
    let mut matching = Vec::new();
    for record in records
        .as_array()
        .ok_or_else(|| invalid("Invalid download inventory."))?
    {
        if absolute(
            library.py(),
            text(record, if imported { "source" } else { "path" }),
        )? == *path
        {
            matching.push(record);
        }
    }
    if matching.len() != 1 {
        return Err(invalid(
            "Download reference changed or is ambiguous. Refresh and review it again.",
        ));
    }
    let record = matching[0];
    reject_links(path)?;
    if !path.is_file()
        || !matches!(
            path.extension()
                .and_then(|s| s.to_str())
                .map(fold)
                .as_deref(),
            Some("zip" | "7z" | "rar" | "fomod")
        )
    {
        return Err(invalid(
            "Download is missing or is not a supported mod archive.",
        ));
    }
    if imported {
        let root = PathBuf::from(config["paths"]["download_directory"].as_str().unwrap_or(""));
        if !root.is_absolute()
            || path.parent() != Some(absolute(library.py(), &root.to_string_lossy())?.as_path())
        {
            return Err(invalid(
                "Imported archive is outside its original MO2 download directory.",
            ));
        }
        if Some(std::fs::metadata(path)?.len()) != record["bytes"].as_u64() {
            return Err(invalid(
                "Imported archive size changed. Repeat the metadata import before installing it.",
            ));
        }
    } else {
        let root: PathBuf = base(library)?.getattr("root")?.extract()?;
        let parent = path
            .parent()
            .ok_or_else(|| invalid("Completed archive is outside an owned download folder."))?;
        if parent.parent() != Some(root.join("downloads").as_path()) {
            return Err(invalid(
                "Completed archive is outside an owned download folder.",
            ));
        }
        crate::transactions::transaction_key(
            parent.file_name().and_then(|v| v.to_str()).unwrap_or(""),
        )?;
    }
    Ok(record.clone())
}
fn metadata(record: &Value, config: &Value, imported: bool) -> Value {
    if !imported {
        return json!({"game":record["game"], "mod_id":record["mod_id"], "file_id":record["file_id"], "version":record["version"]});
    }
    let fields = &record["metadata"];
    let mut game = crate::nexus::nexus_game_domain(text(fields, "gamename"));
    if game.is_empty() {
        game = crate::nexus::nexus_game_domain(text(&config["game"], "id"));
    }
    let number = |name| {
        let v = &fields[name];
        let raw = v
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| v.to_string());
        if raw.bytes().all(|c| c.is_ascii_digit()) {
            raw.parse::<u64>()
                .ok()
                .filter(|n| *n > 0 && *n < 1_000_000_000_000)
                .unwrap_or(0)
        } else {
            0
        }
    };
    let mod_id = number("modid");
    let file_id = number("fileid");
    let version = fields
        .get("version")
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_string())
        })
        .unwrap_or_default();
    json!({"game":if mod_id != 0 {game.clone()} else {String::new()}, "mod_id":if game.is_empty() {0} else {mod_id}, "file_id":if !game.is_empty() && mod_id != 0 {file_id} else {0}, "version":version})
}
#[pyfunction]
fn download_review_capture(
    library: &Bound<'_, PyAny>,
    profile: &str,
    path: &Bound<'_, PyAny>,
    imported: bool,
    states: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let path = absolute(py, path.str()?.to_str()?)?;
    let (rows, logical, config, token) = context(library, profile)?;
    let record = record(library, &config, &path, imported)?;
    let metadata = metadata(&record, &config, imported);
    if !imported {
        let queue = py
            .import("creation_lib.mod_manager.download_queue")?
            .getattr("DownloadQueue")?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("cancel", cancel)?;
        let checksum = queue
            .getattr("_hash")?
            .call((&path,), Some(&kwargs))?
            .get_item(0)?
            .call_method0("hexdigest")?
            .extract::<String>()?;
        if text(&record, "checksum").is_empty() || checksum != text(&record, "checksum") {
            return Err(invalid(
                "Completed archive checksum changed. Download or recover it again before installing.",
            ));
        }
    }
    let root: PathBuf = base(library)?.getattr("root")?.extract()?;
    let mut originals = BTreeMap::new();
    for row in logical.try_iter()? {
        let row = row?;
        originals.insert(row.getattr("id")?.extract::<String>()?, row);
    }
    let game = text(&metadata, "game");
    let mod_id = metadata["mod_id"].as_u64().unwrap_or(0);
    let file_id = metadata["file_id"].as_u64().unwrap_or(0);
    let state = if states.is_truthy()? {
        states.call_method1("get", ((game, mod_id),))?
    } else {
        py.None().into_bound(py)
    };
    let module = py.import("creation_lib.mod_manager.download_install")?;
    let mut candidates = Vec::new();
    for row in rows.try_iter()? {
        check_cancel(Some(cancel))?;
        let row = row?;
        let id: String = row.getattr("id")?.extract()?;
        let kind: String = row.getattr("kind")?.extract()?;
        let Some(original) = originals.get(&id) else {
            continue;
        };
        if matches!(kind.as_str(), "separator" | "foreign") {
            continue;
        }
        if mod_id != 0
            && (row.getattr("nexus_game")?.extract::<String>()? != game
                || row.getattr("nexus_mod_id")?.extract::<u64>()? != mod_id)
        {
            continue;
        }
        let source: String = row.getattr("source")?.extract()?;
        let eligible = kind == "owned"
            && original.getattr("kind")?.extract::<String>()? == "owned"
            && PathBuf::from(&source) == root.join("mods").join(&id)
            && source == original.getattr("source")?.extract::<String>()?;
        let installed: u64 = row.getattr("nexus_file_id")?.extract()?;
        let mut published = false;
        if state.is_truthy()? {
            for file in state
                .call_method1("replacements", (installed,))?
                .try_iter()?
            {
                if file?.getattr("id")?.extract::<u64>()? == file_id {
                    published = true;
                    break;
                }
            }
        }
        let mut reason = if published {
            "Published replacement"
        } else if installed != 0 && installed == file_id {
            "Same installed file"
        } else if mod_id != 0 {
            "Matching mod identity; file replacement unverified"
        } else {
            "Explicit manual destination; no Nexus identity"
        };
        let overlay = !eligible
            && matches!(kind.as_str(), "external" | "reference")
            && PathBuf::from(&source).is_dir();
        if !eligible {
            reason = if overlay {
                "Referenced source: additive owned update available; source remains read only"
            } else {
                "Source cannot be replaced; choose a new owned mod"
            };
        }
        candidates.push(module.getattr("Candidate")?.call1((
            id,
            row.getattr("name")?,
            source,
            row.getattr("version")?,
            installed,
            eligible,
            published,
            reason,
            overlay,
        ))?);
    }
    check_cancel(Some(cancel))?;
    let fields = object(py, &metadata)?;
    let result = module.getattr("DownloadReview")?.call1((
        library.getattr("library_id")?,
        profile,
        target(library)?,
        path.to_string_lossy().as_ref(),
        crate::package::signature(&path)?,
        imported,
        encoding::digest(&record, true, false),
        token,
        PyTuple::new(
            py,
            fields
                .call_method0("items")?
                .try_iter()?
                .collect::<PyResult<Vec<_>>>()?,
        )?,
        PyTuple::new(py, candidates)?,
    ))?;
    result.getattr("by_id")?;
    Ok(result.unbind())
}
#[pyfunction]
fn download_review_validate(
    library: &Bound<'_, PyAny>,
    profile: &str,
    review: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<()> {
    check_cancel(Some(cancel))?;
    if !library
        .getattr("library_id")?
        .eq(review.getattr("library_id")?)?
        || profile != review.getattr("profile_id")?.extract::<String>()?
        || target(library)? != review.getattr("target_id")?.extract::<String>()?
    {
        return Err(invalid(
            "Download target profile/folder changed. Review the destination again.",
        ));
    }
    let (_, _, config, token) = context(library, profile)?;
    let path: PathBuf = review.getattr("path")?.extract()?;
    let record = record(
        library,
        &config,
        &path,
        review.getattr("imported")?.is_truthy()?,
    )?;
    if token != review.getattr("context_token")?.extract::<String>()?
        || encoding::digest(&record, true, false)
            != review.getattr("record_token")?.extract::<String>()?
    {
        return Err(invalid(
            "Download or destination metadata changed. Review the destination again.",
        ));
    }
    if crate::package::signature(&path)? != review.getattr("signature")?.extract::<(u64, i128)>()? {
        return Err(invalid(
            "Archive changed after download review. Reopen its destination review.",
        ));
    }
    Ok(())
}
#[pyfunction]
fn download_review_destination(
    review: &Bound<'_, PyAny>,
    mod_id: &str,
    overlay: bool,
) -> PyResult<String> {
    if mod_id.is_empty() {
        return Ok(String::new());
    }
    let candidate = review.getattr("by_id")?.call_method1("get", (mod_id,))?;
    if !candidate.is_truthy()?
        || !candidate
            .getattr(if overlay {
                "overlay_eligible"
            } else {
                "eligible"
            })?
            .is_truthy()?
    {
        return Err(invalid(
            "Select a reviewed manager-owned destination; original references cannot be replaced.",
        ));
    }
    candidate.getattr("id")?.extract()
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(download_review_capture, m)?)?;
    m.add_function(wrap_pyfunction!(download_review_validate, m)?)?;
    m.add_function(wrap_pyfunction!(download_review_destination, m)?)?;
    Ok(())
}
