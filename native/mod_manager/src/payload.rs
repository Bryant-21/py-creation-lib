use crate::{check_cancel, invalid, reject_links, relative_path};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use unicode_casefold::UnicodeCaseFold;

#[pyfunction]
#[pyo3(signature = (library, client, link, cancel=None, progress=None))]
fn library_download_nexus(
    library: &Bound<'_, PyAny>,
    client: &Bound<'_, PyAny>,
    link: &Bound<'_, PyAny>,
    cancel: Option<&Bound<'_, PyAny>>,
    progress: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    use crate::database::{error, records};
    use crate::python_io::with_database;
    use rusqlite::params;
    let root: PathBuf = library.getattr("root")?.extract()?;
    let factory = library.getattr("_db")?;
    let (game, mod_id, file_id): (String, i64, i64) = link.getattr("identity")?.extract()?;
    let existing = with_database(&factory, |_, db| {
        records(
            &*db.connection()?,
            "SELECT path FROM downloads WHERE game=? AND mod_id=? AND file_id=?",
            params![game, mod_id, file_id],
        )
    })?;
    for item in existing {
        let path = PathBuf::from(item["path"].as_str().unwrap_or(""));
        if path.is_file() {
            reject_links(&path)?;
            return Ok(path.to_string_lossy().into());
        }
    }
    check_cancel(cancel)?;
    let (metadata, url): (Bound<'_, PyAny>, Bound<'_, PyAny>) =
        client.call_method1("resolve", (link,))?.extract()?;
    let filename = relative_path(&metadata.get_item("file_name")?.extract::<String>()?, false)?;
    let stage = library_stage(root.clone())?;
    let destination = root
        .join("downloads")
        .join(uuid::Uuid::new_v4().simple().to_string());
    reject_links(&destination)?;
    let mut moved = false;
    let result = (|| {
        let kwargs = pyo3::types::PyDict::new(library.py());
        let md5 = metadata.call_method1("get", ("md5", ""))?;
        kwargs.set_item(
            "md5",
            if md5.is_truthy()? {
                md5.extract::<String>()?
            } else {
                String::new()
            },
        )?;
        let downloaded = client.call_method(
            "download",
            (url, stage.join(&filename), cancel, progress),
            Some(&kwargs),
        )?;
        let checksum: String = downloaded.get_item(1)?.extract()?;
        check_cancel(cancel)?;
        with_database(&factory, |_, db| {
            db.connection()?
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(error)?;
            std::fs::rename(&stage, &destination)?;
            moved = true;
            let version = metadata
                .call_method1("get", ("version", ""))?
                .str()?
                .to_string();
            db.connection()?
                .execute(
                    "INSERT OR REPLACE INTO downloads VALUES (?,?,?,?,?,?)",
                    params![
                        game,
                        mod_id,
                        file_id,
                        destination.join(&filename).to_string_lossy(),
                        version,
                        checksum
                    ],
                )
                .map_err(error)?;
            check_cancel(cancel)
        })?;
        Ok(destination.join(&filename).to_string_lossy().to_string())
    })();
    if result.is_err() {
        library_discard(root, if moved { destination } else { stage })?;
    }
    result
}

#[pyfunction]
#[pyo3(signature = (db, profile_id, cancel=None))]
fn library_export_plan(
    py: Python<'_>,
    db: &crate::database::Database,
    profile_id: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let connection = db.connection()?;
    let name: Option<String> = connection
        .query_row(
            "SELECT name FROM profiles WHERE id=?",
            [profile_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(crate::database::error)?;
    let Some(name) = name else {
        return Err(invalid("Select a profile first."));
    };
    let rows = crate::library::mod_rows(&connection, profile_id)?;
    drop(connection);
    let mut files: BTreeMap<String, Value> = BTreeMap::new();
    let mut providers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        check_cancel(cancel)?;
        if row["enabled"].as_bool() != Some(true) || row["kind"] == "separator" {
            continue;
        }
        let source = row["source"].as_str().unwrap_or("");
        let name = row["name"].as_str().unwrap_or("");
        if row["kind"] == "foreign" || source.is_empty() {
            return Err(invalid(format!(
                "Enabled mod {name:?} has no importable files. Disable metadata-only entries before staging."
            )));
        }
        let source = std::path::absolute(source)?;
        let package = crate::package::directory(py, source.clone(), cancel)?;
        for (target, relative) in package.plain_plan("", cancel, true)? {
            check_cancel(cancel)?;
            let key: String = target.case_fold().collect();
            if matches!(
                key.as_str(),
                "plugins.txt" | "loadorder.txt" | "archives.txt"
            ) {
                continue;
            }
            providers.entry(key.clone()).or_default().push(name.into());
            let path: PathBuf = source.join(relative).components().collect();
            let (size, modified) = crate::package::signature(&path)?;
            files.insert(key, json!([target, path.to_string_lossy(), size, modified]));
        }
    }
    if files.is_empty() {
        return Err(invalid(
            "No files from enabled mods are available for staging.",
        ));
    }
    crate::paths::validate_destinations(
        files
            .values()
            .map(|v| v[0].as_str().unwrap().to_string())
            .collect(),
    )?;
    if files.len() > 100_000
        || files.values().map(|v| v[2].as_u64().unwrap()).sum::<u64>() > 8 * 1024u64.pow(3)
    {
        return Err(invalid(
            "Profile export exceeds the 100,000-file or 8-GiB staging limit.",
        ));
    }
    providers.retain(|_, names| names.len() > 1);
    Ok(
        json!({"profile_id":profile_id,"profile_name":name,"files":files,"conflicts":providers})
            .to_string(),
    )
}

#[pyfunction]
pub fn library_stage(root: PathBuf) -> PyResult<PathBuf> {
    let directory = root.join(".staging");
    reject_links(&directory)?;
    let path = directory.join(format!("transaction-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&path)?;
    Ok(path)
}

#[pyfunction]
pub fn library_discard(root: PathBuf, path: PathBuf) -> PyResult<()> {
    let root = std::path::absolute(root)?.canonicalize()?;
    let path = std::path::absolute(path)?;
    reject_links(&path)?;
    let resolved = path.canonicalize()?;
    if resolved == root || !resolved.starts_with(&root) {
        return Err(invalid(
            "Refused to remove a path outside this owned library.",
        ));
    }
    std::fs::remove_dir_all(resolved)?;
    Ok(())
}

fn output(path: &Path) -> PyResult<std::fs::File> {
    reject_links(path)?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    Ok(std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?)
}

#[pyfunction]
#[pyo3(signature = (sources, destination, cancel=None, progress=None, byte_progress=false))]
fn library_copy_plan(
    py: Python<'_>,
    sources: Vec<(String, Py<PyAny>, String)>,
    destination: PathBuf,
    cancel: Option<&Bound<'_, PyAny>>,
    progress: Option<&Bound<'_, PyAny>>,
    byte_progress: bool,
) -> PyResult<String> {
    let mut total_bytes = 0u64;
    let sizes: Vec<u64> = sources
        .iter()
        .map(|(_, package, source)| {
            package
                .bind(py)
                .getattr("sizes")?
                .get_item(source.case_fold().collect::<String>())?
                .extract()
        })
        .collect::<PyResult<_>>()?;
    if byte_progress {
        for size in &sizes {
            total_bytes = total_bytes
                .checked_add(*size)
                .ok_or_else(|| invalid("Package size exceeds the supported limit."))?;
        }
    }
    let total_bytes = total_bytes.max(1);
    let mut completed_bytes = 0u64;
    let mut manifest = BTreeMap::new();
    for (index, (target, package, source)) in sources.iter().enumerate() {
        check_cancel(cancel)?;
        let package = package.bind(py);
        package.setattr("_cancel", cancel)?;
        let target = relative_path(target, false)?;
        let path = destination.join(&target);
        let context = package.call_method1("open", (source,))?;
        let stream = context.call_method0("__enter__")?;
        let copied = (|| -> PyResult<(u64, String)> {
            let mut writer = output(&path)?;
            let mut copied = 0;
            let mut checksum = Sha256::new();
            loop {
                check_cancel(cancel)?;
                let chunk = stream.call_method1("read", (1024 * 1024,))?;
                let chunk = chunk.cast::<PyBytes>()?.as_bytes();
                if chunk.is_empty() {
                    break;
                }
                copied += chunk.len() as u64;
                if copied > sizes[index] {
                    return Err(invalid(format!("Source changed during copy: {source}")));
                }
                writer.write_all(chunk)?;
                checksum.update(chunk);
                if byte_progress {
                    if let Some(progress) = progress {
                        progress.call1((
                            (completed_bytes + copied) as f64 / total_bytes as f64,
                            &target,
                        ))?;
                    }
                }
            }
            writer.sync_all()?;
            Ok((copied, format!("{:x}", checksum.finalize())))
        })();
        let (copied, digest) = match copied {
            Ok(value) => {
                context.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
                value
            }
            Err(error) => {
                context.call_method1(
                    "__exit__",
                    (error.get_type(py), error.value(py), error.traceback(py)),
                )?;
                return Err(error);
            }
        };
        if copied != sizes[index] {
            return Err(invalid(format!(
                "Source size changed during copy: {source}"
            )));
        }
        manifest.insert(target.clone(), json!({"size":copied,"sha256":digest}));
        completed_bytes += copied;
        if let Some(progress) = progress {
            let amount = if byte_progress {
                completed_bytes as f64 / total_bytes as f64
            } else {
                (index + 1) as f64 / sources.len().max(1) as f64
            };
            progress.call1((amount, target))?;
        }
    }
    Ok(json!(manifest).to_string())
}

#[pyfunction]
#[pyo3(signature = (root, files, cancel=None, progress=None))]
fn library_export(
    root: PathBuf,
    files: &str,
    cancel: Option<&Bound<'_, PyAny>>,
    progress: Option<&Bound<'_, PyAny>>,
) -> PyResult<PathBuf> {
    let files: Vec<(String, String, u64, i128)> =
        serde_json::from_str(files).map_err(|e| invalid(e.to_string()))?;
    let destination = root
        .join("exports")
        .join(uuid::Uuid::new_v4().simple().to_string());
    reject_links(&destination)?;
    let stage = library_stage(root.clone())?;
    let result = (|| {
        for (index, (target, source, size, modified)) in files.iter().enumerate() {
            check_cancel(cancel)?;
            let source = Path::new(source);
            reject_links(source)?;
            if crate::package::signature(source)? != (*size, *modified) {
                return Err(invalid(format!(
                    "Source changed after the staging preview: {}",
                    source.display()
                )));
            }
            let mut reader = std::fs::File::open(source)?;
            let mut writer = output(&stage.join(relative_path(target, false)?))?;
            let mut copied = 0u64;
            let mut buffer = vec![0u8; 1024 * 1024];
            loop {
                check_cancel(cancel)?;
                let count = reader.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                copied += count as u64;
                if copied > *size {
                    return Err(invalid(format!(
                        "Source changed during staging: {}",
                        source.display()
                    )));
                }
                writer.write_all(&buffer[..count])?;
            }
            writer.sync_all()?;
            if copied != *size || crate::package::signature(source)? != (*size, *modified) {
                return Err(invalid(format!(
                    "Source changed during staging: {}",
                    source.display()
                )));
            }
            if let Some(progress) = progress {
                progress.call1(((index + 1) as f64 / files.len() as f64, target))?;
            }
        }
        check_cancel(cancel)?;
        std::fs::rename(&stage, &destination)?;
        Ok(destination)
    })();
    if result.is_err() {
        library_discard(root, stage)?;
    }
    result
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(library_download_nexus, m)?)?;
    m.add_function(wrap_pyfunction!(library_stage, m)?)?;
    m.add_function(wrap_pyfunction!(library_discard, m)?)?;
    m.add_function(wrap_pyfunction!(library_copy_plan, m)?)?;
    m.add_function(wrap_pyfunction!(library_export, m)?)?;
    m.add_function(wrap_pyfunction!(library_export_plan, m)?)?;
    Ok(())
}
