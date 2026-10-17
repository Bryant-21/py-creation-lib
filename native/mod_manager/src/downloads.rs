use crate::{
    check_cancel,
    database::{Database, error, records},
    encoding,
    executables::text,
    invalid,
    nexus::failure,
    python_io::with_database,
    reject_links, relative_path,
    transactions::transaction_key,
};
use md5::Md5;
use pyo3::prelude::*;
use rusqlite::params;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::PathBuf;

const LIMIT: u64 = 8 * 1024u64.pow(3);

#[pyclass(skip_from_py_object)]
#[derive(Clone)]
pub struct DownloadHash {
    digest: Sha256,
}
#[pymethods]
impl DownloadHash {
    #[new]
    pub fn new() -> Self {
        Self {
            digest: Sha256::new(),
        }
    }
    pub fn update(&mut self, data: &[u8]) {
        self.digest.update(data);
    }
    pub fn hexdigest(&self) -> String {
        format!("{:x}", self.digest.clone().finalize())
    }
    fn copy(&self) -> Self {
        self.clone()
    }
}

#[pyfunction]
pub fn download_etag(value: &str) -> PyResult<String> {
    let value = encoding::parse(value)?;
    let Some(value) = value.as_str() else {
        return Ok(String::new());
    };
    if !(3..=256).contains(&value.len())
        || !value.starts_with('"')
        || !value.ends_with('"')
        || !value.as_bytes()[1..value.len() - 1]
            .iter()
            .all(|b| *b == 0x21 || (0x23..=0x7e).contains(b))
    {
        return Ok(String::new());
    }
    Ok(value.into())
}

#[pyfunction]
#[pyo3(signature = (db, id=None))]
fn download_records(db: &Database, id: Option<&str>) -> PyResult<String> {
    let connection = db.connection()?;
    let mut result = if let Some(id) = id {
        transaction_key(id)?;
        let rows = records(&connection, "SELECT * FROM download_jobs WHERE id=?", [id])?;
        if rows.is_empty() {
            return Err(invalid("This queued download is unavailable."));
        }
        rows
    } else {
        if records(
            &connection,
            "SELECT name FROM sqlite_master WHERE type='table' AND name='download_jobs'",
            [],
        )?
        .is_empty()
        {
            return Ok("[]".into());
        }
        records(
            &connection,
            "SELECT * FROM download_jobs ORDER BY rowid DESC",
            [],
        )?
    };
    for row in &mut result {
        row["record"] = encoding::parse(text(row, "record"))?;
    }
    Ok(if id.is_some() {
        result.remove(0).to_string()
    } else {
        json!(result).to_string()
    })
}

#[pyfunction]
#[pyo3(signature = (path, size=None, cancel=None))]
pub fn download_hash(
    path: PathBuf,
    size: Option<u64>,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<(DownloadHash, u64)> {
    let mut digest = DownloadHash::new();
    let mut read = 0u64;
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; 1024 * 1024];
    while size.is_none_or(|size| read < size) {
        check_cancel(cancel)?;
        let length = size
            .map(|size| (size - read).min(buffer.len() as u64) as usize)
            .unwrap_or(buffer.len());
        let count = file.read(&mut buffer[..length])?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        read += count as u64;
        if read > LIMIT {
            return Err(invalid("Owned download exceeds the 8 GiB safety limit."));
        }
    }
    if size.is_some_and(|size| read != size) {
        return Err(invalid(
            "Owned partial data is shorter than its saved checkpoint.",
        ));
    }
    Ok((digest, read))
}

#[pyfunction]
#[pyo3(signature = (db, root, link, cancel=None))]
fn download_enqueue(
    db: &Database,
    root: PathBuf,
    link: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let link = encoding::parse(link)?;
    crate::nexus::mod_identity(text(&link, "game"), &link["mod_id"])?;
    let file_id = link["file_id"]
        .as_u64()
        .filter(|v| *v > 0 && *v < 1_000_000_000_000)
        .ok_or_else(|| failure("This download has no supported Nexus file identity."))?;
    let connection = db.connection()?;
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    let game = text(&link, "game");
    let mod_id = link["mod_id"].as_u64().unwrap();
    if let Some(row) = records(
        &connection,
        "SELECT id FROM download_jobs WHERE game=? AND mod_id=? AND file_id=?",
        params![game, mod_id, file_id],
    )?
    .first()
    {
        return Ok(text(row, "id").into());
    }
    if connection
        .query_row("SELECT count(*) FROM download_jobs", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(error)?
        >= 1000
    {
        return Err(invalid(
            "Download history is full. Remove a queued/history entry before adding another.",
        ));
    }
    let mut identifier = uuid::Uuid::new_v4().simple().to_string();
    let mut state = "queued";
    let mut record = json!({});
    if let Some(cached) = records(
        &connection,
        "SELECT path,version,checksum FROM downloads WHERE game=? AND mod_id=? AND file_id=?",
        params![game, mod_id, file_id],
    )?
    .first()
    {
        let path = PathBuf::from(text(cached, "path"));
        reject_links(&path)?;
        if path.is_file() {
            let folder = path.parent().unwrap();
            let key = folder.file_name().and_then(|v| v.to_str()).unwrap_or("");
            if folder.parent() != Some(root.join("downloads").as_path())
                || transaction_key(key).is_err()
            {
                return Err(invalid(
                    "The cached Nexus archive is outside an owned download folder.",
                ));
            }
            let (digest, size) = download_hash(path.clone(), None, cancel)?;
            if digest.hexdigest() != text(cached, "checksum") {
                return Err(invalid(
                    "Cached Nexus archive changed. Review the owned download before requesting it again.",
                ));
            }
            identifier = key.into();
            state = "complete";
            record = json!({"filename":relative_path(path.file_name().unwrap().to_str().unwrap(),false)?,"version":cached["version"],"bytes":size,"total":size,"sha256":digest.hexdigest(),"md5":"","etag":""});
        }
    }
    connection
        .execute(
            "INSERT INTO download_jobs VALUES (?,?,?,?,?,?)",
            params![identifier, game, mod_id, file_id, state, record.to_string()],
        )
        .map_err(error)?;
    check_cancel(cancel)?;
    Ok(identifier)
}

#[pyfunction]
fn download_store(db: &Database, id: &str, state: &str, record: &str) -> PyResult<String> {
    let record = encoding::parse(record)?;
    let mut allowed = json!({});
    for name in [
        "filename", "version", "bytes", "total", "etag", "sha256", "md5",
    ] {
        if let Some(value) = record.get(name) {
            allowed[name] = value.clone();
        }
    }
    let connection = db.connection()?;
    if connection.is_autocommit() {
        connection.execute_batch("BEGIN").map_err(error)?;
    }
    let changed = connection
        .execute(
            "UPDATE download_jobs SET state=?,record=? WHERE id=?",
            params![state, allowed.to_string(), id],
        )
        .map_err(error)?;
    if changed == 0 {
        return Err(invalid(
            "The queued download was removed during its operation.",
        ));
    }
    Ok(allowed.to_string())
}

#[pyfunction]
fn download_paths(root: PathBuf, id: &str) -> PyResult<(PathBuf, PathBuf)> {
    let id = transaction_key(id)?;
    let partials = root.join("downloads/.partials");
    reject_links(&partials)?;
    std::fs::create_dir_all(&partials)?;
    let partial = partials.join(format!("{id}.part"));
    let folder = root.join("downloads").join(id);
    reject_links(&partial)?;
    reject_links(&folder)?;
    Ok((partial, folder))
}

#[pyfunction]
#[pyo3(signature = (factory, job, partial, folder, cancel=None, checkpoint=None))]
fn download_finish(
    factory: &Bound<'_, PyAny>,
    job: &str,
    partial: PathBuf,
    folder: PathBuf,
    cancel: Option<&Bound<'_, PyAny>>,
    checkpoint: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let job = encoding::parse(job)?;
    let record = &job["record"];
    let filename = relative_path(text(record, "filename"), false)?;
    if filename.contains('/')
        || !["zip", "7z", "rar", "fomod"].contains(
            &filename
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str(),
        )
    {
        return Err(invalid("Invalid queued archive filename."));
    }
    let target = folder.join(filename);
    reject_links(&target)?;
    let source = if target.is_file() { &target } else { &partial };
    let (digest, size) = download_hash(source.clone(), None, cancel)?;
    if record["bytes"].as_u64() != Some(size) || digest.hexdigest() != text(record, "sha256") {
        return Err(invalid(
            "Finalizing download differs from its committed checkpoint. Retained data needs review.",
        ));
    }
    std::fs::create_dir_all(folder)?;
    check_cancel(cancel)?;
    if source == &partial {
        if target.exists() {
            return Err(invalid(
                "The final download path is occupied; no file was replaced.",
            ));
        }
        std::fs::rename(partial, &target)?;
    }
    with_database(factory, |_, db| {
        let connection = db.connection()?;
        connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
        connection
            .execute(
                "INSERT OR REPLACE INTO downloads VALUES (?,?,?,?,?,?)",
                params![
                    text(&job, "game"),
                    job["mod_id"].as_i64().unwrap(),
                    job["file_id"].as_i64().unwrap(),
                    target.to_string_lossy(),
                    text(record, "version"),
                    text(record, "sha256")
                ],
            )
            .map_err(error)?;
        connection
            .execute(
                "UPDATE download_jobs SET state='complete' WHERE id=?",
                [text(&job, "id")],
            )
            .map_err(error)?;
        drop(connection);
        if let Some(checkpoint) = checkpoint {
            checkpoint.call1(("before-commit",))?;
        }
        check_cancel(cancel)
    })?;
    Ok(target.to_string_lossy().into())
}

#[pyfunction]
#[pyo3(signature = (partial, md5, cancel=None))]
pub fn download_verify_md5(
    partial: PathBuf,
    md5: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    if md5.is_empty() {
        return Ok(());
    }
    let mut file = std::fs::File::open(partial)?;
    let mut digest = Md5::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        check_cancel(cancel)?;
        digest.update(&buffer[..count]);
    }
    if format!("{:x}", digest.finalize()) != md5.to_ascii_lowercase() {
        return Err(failure(
            "Downloaded archive failed its Nexus checksum. Remove the queued download and retry.",
        ));
    }
    Ok(())
}

#[pyfunction]
fn download_lock(root: PathBuf, id: &str) -> PyResult<crate::RuntimeLease> {
    let directory = root.join("downloads/.locks");
    reject_links(&directory)?;
    std::fs::create_dir_all(&directory)?;
    crate::RuntimeLease::new(
        directory.join(format!("{}.lock", transaction_key(id)?)),
        false,
    )
    .map_err(|error| {
        Python::attach(|py| {
            if error.is_instance_of::<pyo3::exceptions::PyValueError>(py) {
                invalid("Another transfer owns this download. Wait or pause it before retrying.")
            } else {
                error
            }
        })
    })
}

#[pyfunction]
fn download_library_lock(library: &Bound<'_, PyAny>, id: &str) -> PyResult<crate::RuntimeLease> {
    library.call_method0("_runtime_write_guard")?;
    download_lock(library.getattr("root")?.extract()?, id)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(download_library_lock, m)?)?;
    m.add_class::<DownloadHash>()?;
    m.add_function(wrap_pyfunction!(download_etag, m)?)?;
    m.add_function(wrap_pyfunction!(download_records, m)?)?;
    m.add_function(wrap_pyfunction!(download_hash, m)?)?;
    m.add_function(wrap_pyfunction!(download_enqueue, m)?)?;
    m.add_function(wrap_pyfunction!(download_store, m)?)?;
    m.add_function(wrap_pyfunction!(download_paths, m)?)?;
    m.add_function(wrap_pyfunction!(download_finish, m)?)?;
    m.add_function(wrap_pyfunction!(download_verify_md5, m)?)?;
    m.add_function(wrap_pyfunction!(download_lock, m)?)?;
    Ok(())
}
