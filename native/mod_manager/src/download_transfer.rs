use crate::{
    check_cancel,
    database::error,
    downloads::{DownloadHash, download_etag, download_hash, download_verify_md5},
    encoding,
    executables::text,
    invalid,
    nexus::{NexusFailure, failure, nexus_cdn_url},
    python_io::{with_context, with_database},
    reject_links, relative_path,
};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use serde_json::{Value, json};
use std::io::Write;
use std::path::PathBuf;

const LIMIT: u64 = 8 * 1024u64.pow(3);
pyo3::create_exception!(
    mod_manager_native,
    DownloadInterrupted,
    pyo3::exceptions::PyValueError
);

fn decode(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    encoding::parse(
        &value
            .py()
            .import("json")?
            .call_method1("dumps", (value,))?
            .extract::<String>()?,
    )
}
fn encode<'py>(py: Python<'py>, value: &Value) -> PyResult<Bound<'py, PyAny>> {
    py.import("json")?
        .call_method1("loads", (value.to_string(),))
}
fn store(
    queue: &Bound<'_, PyAny>,
    job: &Bound<'_, PyAny>,
    state: &str,
    record: &Value,
) -> PyResult<()> {
    queue.call_method1("_store", (job, state, encode(queue.py(), record)?))?;
    Ok(())
}
fn finish(
    queue: &Bound<'_, PyAny>,
    job: &Bound<'_, PyAny>,
    partial: &Bound<'_, PyAny>,
    folder: &Bound<'_, PyAny>,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    queue
        .call_method1("_finish", (job, partial, folder, cancel))?
        .extract()
}
fn cancelled(py: Python<'_>, error: &PyErr) -> PyResult<bool> {
    Ok(error.is_instance_of::<crate::OperationCancelled>(py)
        || error.is_instance(
            py,
            &py.import("creation_lib.mod_manager.paths")?
                .getattr("Cancelled")?,
        ))
}
fn header(response: &Bound<'_, PyAny>, name: &str, default: &str) -> PyResult<String> {
    response
        .getattr("headers")?
        .call_method1("get", (name, default))?
        .extract()
}
fn bytes_message(count: u64) -> String {
    let digits = count.to_string();
    let mut result = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    format!("Downloading {result} bytes")
}

#[pyfunction]
#[pyo3(signature = (queue, id, client, link_factory, link=None, cancel=None, progress=None))]
fn download_run(
    py: Python<'_>,
    queue: &Bound<'_, PyAny>,
    id: &str,
    client: &Bound<'_, PyAny>,
    link_factory: &Bound<'_, PyAny>,
    link: Option<&Bound<'_, PyAny>>,
    cancel: Option<&Bound<'_, PyAny>>,
    progress: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let job = queue.call_method1("get", (id,))?;
    let value = decode(&job)?;
    let mut record = value["record"].clone();
    let (partial_py, folder_py): (Bound<'_, PyAny>, Bound<'_, PyAny>) =
        queue.call_method1("_paths", (&job,))?.extract()?;
    let partial: PathBuf = partial_py.extract()?;
    let folder: PathBuf = folder_py.extract()?;
    if value["state"] == "complete" {
        let target: PathBuf = folder
            .join(relative_path(text(&record, "filename"), false)?)
            .components()
            .collect();
        reject_links(&target)?;
        let (digest, size) = download_hash(target.clone(), None, cancel)?;
        if digest.hexdigest() != text(&record, "sha256") || record["bytes"].as_u64() != Some(size) {
            return Err(invalid(
                "Completed download changed. Review the owned archive before reusing it.",
            ));
        }
        return Ok(target.to_string_lossy().into());
    }
    if value["state"] == "finalizing" {
        return match finish(queue, &job, &partial_py, &folder_py, cancel) {
            Err(error) if cancelled(py, &error)? => {
                Err(DownloadInterrupted::new_err((true, String::new())))
            }
            result => result,
        };
    }
    let link = match link {
        Some(link) => link.clone(),
        None => link_factory.call1((
            text(&value, "game"),
            value["mod_id"].as_u64().unwrap(),
            value["file_id"].as_u64().unwrap(),
        ))?,
    };
    let identity: (String, u64, u64) = link.getattr("identity")?.extract()?;
    if json!([identity.0, identity.1, identity.2])
        != json!([value["game"], value["mod_id"], value["file_id"]])
    {
        return Err(failure(
            "The fresh browser link identifies a different queued file.",
        ));
    }
    let mut digest = DownloadHash::new();
    let mut count = 0u64;
    let result = (|| -> PyResult<String> {
        check_cancel(cancel)?;
        if partial.exists() {
            count = record
                .get("bytes")
                .unwrap_or(&json!(0))
                .as_u64()
                .filter(|v| *v <= LIMIT)
                .ok_or_else(|| invalid("Invalid owned partial checkpoint size."))?;
            digest = download_hash(partial.clone(), Some(count), cancel)?.0;
            if digest.hexdigest() != text(&record, "sha256") {
                return Err(invalid(
                    "Owned partial data changed. Remove this queued download before restarting it.",
                ));
            }
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&partial)?
                .set_len(count)?;
            if count != 0 && record["total"].as_u64() == Some(count) {
                download_verify_md5(partial.clone(), text(&record, "md5"), cancel)?;
                check_cancel(cancel)?;
                store(queue, &job, "finalizing", &record)?;
                return finish(queue, &job, &partial_py, &folder_py, cancel);
            }
        }
        let (metadata, url): (Bound<'_, PyAny>, String) =
            client.call_method1("resolve", (&link,))?.extract()?;
        nexus_cdn_url(&url)?;
        let metadata = decode(&metadata)?;
        let filename = relative_path(text(&metadata, "file_name"), false)?;
        let version = metadata
            .get("version")
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| v.to_string())
            })
            .unwrap_or_default();
        let md5 = metadata
            .get("md5")
            .filter(|v| !v.is_null())
            .unwrap_or(&Value::Null);
        let md5 = if md5.is_null() {
            ""
        } else {
            md5.as_str()
                .ok_or_else(|| failure("Nexus returned an invalid expected checksum."))?
        };
        if filename.contains('/')
            || !["zip", "7z", "rar", "fomod"].contains(
                &filename
                    .rsplit('.')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str(),
            )
            || version.chars().count() > 256
        {
            return Err(failure(
                "Nexus returned invalid archive filename/version metadata.",
            ));
        }
        if !md5.is_empty() && (md5.len() != 32 || !md5.bytes().all(|b| b.is_ascii_hexdigit())) {
            return Err(failure("Nexus returned an invalid expected checksum."));
        }
        let etag = download_etag(&record.get("etag").unwrap_or(&json!("")).to_string())?;
        if record["filename"] != filename || text(&record, "md5") != md5 || etag.is_empty() {
            count = 0;
            digest = DownloadHash::new();
        }
        let headers = PyDict::new(py);
        headers.set_item(
            "User-Agent",
            format!(
                "ModBox21/{}",
                client.getattr("version")?.extract::<String>()?
            ),
        )?;
        headers.set_item("Accept-Encoding", "identity")?;
        if count != 0 {
            headers.set_item("Range", format!("bytes={count}-"))?;
            headers.set_item("If-Range", &etag)?;
        }
        check_cancel(cancel)?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("headers", headers)?;
        let request = py
            .import("urllib.request")?
            .getattr("Request")?
            .call((url,), Some(&kwargs))?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("api", false)?;
        kwargs.set_item("accepted_statuses", (416,))?;
        let response = client.call_method("_open", (request,), Some(&kwargs))?;
        let completed = with_context(&response, |response| -> PyResult<Option<String>> {
            if !header(response, "Content-Encoding", "identity")?.eq_ignore_ascii_case("identity") {
                return Err(failure(
                    "Download content encoding cannot be matched to the saved archive bytes.",
                ));
            }
            let status = response
                .getattr("status")
                .map(|v| v.extract::<u16>())
                .unwrap_or(Ok(200))?;
            let response_etag = download_etag(&json!(header(response, "ETag", "")?).to_string())?;
            let expected_length = header(response, "Content-Length", "")?;
            let remaining = if expected_length.is_empty() {
                None
            } else {
                Some(
                    expected_length
                        .parse::<u64>()
                        .map_err(|_| failure("Download has an invalid Content-Length."))?,
                )
            };
            if status == 416 {
                let range = header(response, "Content-Range", "")?;
                let total = range
                    .strip_prefix("bytes */")
                    .and_then(|v| v.parse::<u64>().ok());
                if count == 0
                    || total != Some(count)
                    || response_etag != etag
                    || record["total"]
                        .as_u64()
                        .is_some_and(|v| v != 0 && v != count)
                {
                    return Err(failure(
                        "Download EOF response does not prove the saved prefix is complete. Partial data was retained.",
                    ));
                }
                download_verify_md5(partial.clone(), md5, cancel)?;
                record["total"] = json!(count);
                record["bytes"] = json!(count);
                record["sha256"] = json!(digest.hexdigest());
                check_cancel(cancel)?;
                store(queue, &job, "finalizing", &record)?;
                return Ok(Some(finish(queue, &job, &partial_py, &folder_py, cancel)?));
            }
            let total = if status == 206 {
                let range = header(response, "Content-Range", "")?;
                let parts = range
                    .strip_prefix("bytes ")
                    .and_then(|v| v.split_once('/'))
                    .and_then(|(span, total)| {
                        span.split_once('-').map(|(start, end)| (start, end, total))
                    })
                    .and_then(|(start, end, total)| {
                        Some((
                            start.parse::<u64>().ok()?,
                            end.parse::<u64>().ok()?,
                            total.parse::<u64>().ok()?,
                        ))
                    });
                let valid = parts.filter(|(start, end, total)| {
                    count != 0
                        && *start == count
                        && *end >= count
                        && *total > count
                        && *end == total - 1
                        && !record["total"]
                            .as_u64()
                            .is_some_and(|v| v != 0 && v != *total)
                        && response_etag == etag
                        && remaining.is_none_or(|v| v == total - count)
                });
                valid.ok_or_else(||failure("Download range/validator does not match the saved checkpoint. Partial data was retained."))?.2
            } else if status == 200 {
                count = 0;
                digest = DownloadHash::new();
                remaining.unwrap_or(0)
            } else {
                return Err(failure(
                    "Unsupported download response status; partial data was retained.",
                ));
            };
            if total > LIMIT || remaining.is_some_and(|v| v > LIMIT - count) {
                return Err(failure("Download exceeds the 8 GiB safety limit."));
            }
            let free: u64 = py
                .import("shutil")?
                .call_method1("disk_usage", (queue.getattr("library")?.getattr("root")?,))?
                .getattr("free")?
                .extract()?;
            if free < remaining.unwrap_or(0).max(1024 * 1024) {
                return Err(invalid("Insufficient free space for this download."));
            }
            record = json!({"filename":filename,"version":version,"md5":md5,"etag":response_etag,"bytes":count,"total":total,"sha256":digest.hexdigest()});
            store(queue, &job, "running", &record)?;
            let start = std::time::Instant::now();
            let mut checkpoint = count;
            let mut received = 0u64;
            let mut stream = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .append(count != 0)
                .truncate(count == 0)
                .open(&partial)?;
            let copied = (|| -> PyResult<()> {
                loop {
                    check_cancel(cancel)?;
                    if start.elapsed().as_secs() > 1800 {
                        return Err(failure(
                            "Download exceeded its 30 minute transfer limit; resume the retained checkpoint.",
                        ));
                    }
                    let data = response
                        .call_method1("read1", (1024 * 1024,))
                        .map_err(|_| {
                            failure("Download was interrupted; resume its owned checkpoint.")
                        })?;
                    let data = data.cast::<pyo3::types::PyBytes>()?.as_bytes();
                    if data.is_empty() {
                        break;
                    }
                    if count + data.len() as u64 > LIMIT
                        || (total != 0 && count + data.len() as u64 > total)
                    {
                        return Err(failure(
                            "Download bytes exceed the declared or supported size.",
                        ));
                    }
                    stream.write_all(data)?;
                    digest.update(data);
                    count += data.len() as u64;
                    received += data.len() as u64;
                    record["bytes"] = json!(count);
                    record["sha256"] = json!(digest.hexdigest());
                    if count - checkpoint >= 8 * 1024 * 1024 {
                        stream.sync_all()?;
                        store(queue, &job, "running", &record)?;
                        checkpoint = count;
                    }
                    if let Some(progress) = progress {
                        progress.call1((
                            if total == 0 {
                                None
                            } else {
                                Some(count as f64 / total as f64)
                            },
                            bytes_message(count),
                        ))?;
                    }
                }
                Ok(())
            })();
            stream.sync_all()?;
            store(queue, &job, "running", &record)?;
            drop(stream);
            copied?;
            if remaining.is_some_and(|v| received != v)
                || (total != 0 && count != total)
                || count == 0
            {
                return Err(failure(
                    "Download ended before its declared size; partial data was retained.",
                ));
            }
            Ok(None)
        })?;
        if let Some(path) = completed {
            return Ok(path);
        }
        download_verify_md5(partial.clone(), md5, cancel)?;
        check_cancel(cancel)?;
        store(queue, &job, "finalizing", &record)?;
        finish(queue, &job, &partial_py, &folder_py, cancel)
    })();
    match result {
        Err(error) if cancelled(py, &error)? => {
            if job.get_item("state")?.extract::<String>()? != "finalizing" {
                store(queue, &job, "paused", &decode(&job.get_item("record")?)?)?;
            }
            Err(DownloadInterrupted::new_err((true, String::new())))
        }
        Err(error)
            if error.is_instance_of::<pyo3::exceptions::PyValueError>(py)
                || error.is_instance_of::<pyo3::exceptions::PyOSError>(py) =>
        {
            if job.get_item("state")?.extract::<String>()? != "finalizing" {
                store(queue, &job, "failed", &decode(&job.get_item("record")?)?)?;
            }
            let message = if error.is_instance_of::<NexusFailure>(py) {
                error
                    .value(py)
                    .getattr("args")?
                    .get_item(0)?
                    .extract::<String>()?
            } else if error.is_instance_of::<pyo3::exceptions::PyValueError>(py) {
                error.value(py).str()?.to_string()
            } else {
                "Owned download I/O failed. Its checkpoint was retained for review/resume.".into()
            };
            Err(DownloadInterrupted::new_err((false, message)))
        }
        result => result,
    }
}

#[pyfunction]
fn download_remove(queue: &Bound<'_, PyAny>, id: &str) -> PyResult<()> {
    let job = queue.call_method1("get", (id,))?;
    if job.get_item("state")?.extract::<String>()? == "finalizing" {
        return Err(invalid(
            "Resume this download's finalization recovery before removing its queue entry.",
        ));
    }
    let partial = queue.call_method1("_paths", (&job,))?.get_item(0)?;
    with_database(&queue.getattr("library")?.getattr("_db")?, |_, db| {
        let connection = db.connection()?;
        connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
        connection
            .execute("DELETE FROM download_jobs WHERE id=?", [id])
            .map_err(error)?;
        if partial.call_method0("is_file")?.is_truthy()? {
            partial.call_method0("unlink")?;
        }
        Ok(())
    })
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add(
        "DownloadInterrupted",
        m.py().get_type::<DownloadInterrupted>(),
    )?;
    m.add_function(wrap_pyfunction!(download_run, m)?)?;
    m.add_function(wrap_pyfunction!(download_remove, m)?)?;
    Ok(())
}
