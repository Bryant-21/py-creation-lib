use crate::{history_prune::py_path, invalid, python_io::with_context, reject_links};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict, PyTuple},
};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("creation_lib.mod_manager.nxm_instances")
}
fn decode(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    let text: String = value
        .py()
        .import("json")?
        .call_method1("dumps", (value,))?
        .extract()?;
    crate::encoding::parse(&text)
}
fn object(py: Python<'_>, value: &Value) -> PyResult<Py<PyAny>> {
    Ok(py
        .import("json")?
        .call_method1("loads", (value.to_string(),))?
        .unbind())
}
fn identifier(value: &Value) -> PyResult<&str> {
    value
        .as_str()
        .filter(|v| {
            v.len() == 32
                && v.bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        })
        .ok_or_else(|| invalid("Browser routing identity is invalid."))
}
fn context(value: &Value) -> PyResult<Value> {
    let fields = [
        "library_id",
        "library_root",
        "profile_id",
        "profile_name",
        "target_id",
        "target_name",
        "target_root",
        "game",
    ];
    let valid = value.as_object().is_some_and(|v| v.len() == 9)
        && value["ready"].is_boolean()
        && fields.iter().all(|k| {
            value[k]
                .as_str()
                .is_some_and(|s| s.chars().count() <= 2048 && !s.chars().any(|c| c < ' '))
        });
    if !valid
        || value["ready"] == true
            && (value["library_id"] == ""
                || value["profile_id"] == ""
                || !crate::nexus::GAMES.contains(&value["game"].as_str().unwrap_or(""))
                || !Path::new(value["library_root"].as_str().unwrap_or("")).is_absolute())
    {
        return Err(invalid("Browser destination context is invalid."));
    }
    if fields.iter().any(|k| {
        value[k].as_str().unwrap().chars().count()
            > if matches!(*k, "library_root" | "target_root") {
                2048
            } else {
                200
            }
    }) {
        return Err(invalid(
            "Browser destination labels exceed their safety bound.",
        ));
    }
    Ok(value.clone())
}
fn read(path: &Path) -> PyResult<Value> {
    reject_links(path)?;
    let file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(16385).read_to_end(&mut bytes)?;
    if bytes.len() > 16384 {
        return Err(invalid("Browser routing record exceeds its safety bound."));
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid("Browser routing record is invalid."))
}
fn write(path: &Path, value: &Value) -> PyResult<()> {
    reject_links(path)?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(value.to_string().as_bytes())?;
        file.flush()?;
        drop(file);
        // Path.replace supplies Windows replace-existing semantics.
        Python::attach(|py| {
            py_path(py, &temporary)?
                .call_method1("replace", (py_path(py, path)?,))
                .map(|_| ())
        })?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}
fn process_state(py: Python<'_>, value: &Value) -> PyResult<String> {
    py.import("creation_lib.mod_manager.process_identity")?
        .call_method1("process_state", (object(py, value)?,))?
        .extract()
}
fn valid_process(py: Python<'_>, value: &Value) -> PyResult<bool> {
    crate::process_identity::valid_process_identity(object(py, value)?.bind(py))
}
fn handled(py: Python<'_>, e: &PyErr) -> bool {
    e.is_instance_of::<pyo3::exceptions::PyOSError>(py)
        || e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
        || e.is_instance_of::<pyo3::exceptions::PyTypeError>(py)
        || e.is_instance_of::<pyo3::exceptions::PyKeyError>(py)
}
fn transport_error(py: Python<'_>, e: &PyErr) -> bool {
    e.is_instance_of::<pyo3::exceptions::PyOSError>(py)
        || e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
        || e.is_instance_of::<pyo3::exceptions::PyEOFError>(py)
}
fn destination(py: Python<'_>, value: &Value) -> PyResult<Py<PyAny>> {
    Ok(module(py)?
        .getattr("Destination")?
        .call1((
            value["instance_id"].as_str().unwrap_or(""),
            value["address"].as_str().unwrap_or(""),
            object(py, &value["process"])?,
            object(py, &value["context"])?,
        ))?
        .unbind())
}
#[pyfunction]
fn nxm_destination_label(value: &Bound<'_, PyAny>) -> PyResult<String> {
    let value = decode(&value.getattr("context")?)?;
    let root = Path::new(value["library_root"].as_str().unwrap_or(""));
    let name = root.file_name().unwrap_or_default().to_string_lossy();
    let target = value["target_name"]
        .as_str()
        .filter(|v| !v.is_empty())
        .unwrap_or("No native target");
    Ok(format!(
        "{name} / {} / {target}",
        value["profile_name"].as_str().unwrap_or("")
    ))
}
#[pyfunction]
fn nxm_discover(py: Python<'_>, directory: PathBuf, game: &str) -> PyResult<Py<PyAny>> {
    let folder = directory.join("nxm-instances");
    reject_links(&folder)?;
    let mut result = Vec::new();
    let mut count = 0;
    let mut live = 0;
    if folder.exists() {
        for entry in fs::read_dir(folder)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name
                .strip_suffix(".json")
                .is_some_and(|s| identifier(&json!(s)).is_ok())
            {
                continue;
            }
            count += 1;
            if count > 256 {
                return Err(invalid(
                    "Browser discovery exceeds 256 app-owned records. Review stale routing metadata before continuing.",
                ));
            }
            let found = (|| -> PyResult<Option<Py<PyAny>>> {
                let data = read(&entry.path())?;
                if !data.as_object().is_some_and(|v| {
                    v.len() == 5
                        && ["format", "instance_id", "address", "process", "context"]
                            .iter()
                            .all(|k| v.contains_key(*k))
                }) || data["format"] != 1
                {
                    return Ok(None);
                }
                let id = identifier(&data["instance_id"])?;
                if name != format!("{id}.json")
                    || data["address"] != format!(r"\\.\pipe\ModBox21-Nexus-{id}")
                    || !valid_process(py, &data["process"])?
                {
                    return Ok(None);
                }
                let state = process_state(py, &data["process"])?;
                if matches!(state.as_str(), "exited" | "reused") {
                    if read(&entry.path())? == data {
                        fs::remove_file(entry.path())?;
                    }
                    return Ok(None);
                }
                if state != "alive" {
                    return Ok(None);
                }
                let peer = py
                    .import("creation_lib.mod_manager.pipe_peers")?
                    .call_method1("process", (data["process"]["pid"].as_u64().unwrap_or(0),))?;
                if decode(&peer)? != data["process"] {
                    return Ok(None);
                }
                live += 1;
                if live > 64 {
                    return Ok(None);
                }
                let c = context(&data["context"])?;
                if c["ready"] != true || !game.is_empty() && c["game"] != game {
                    return Ok(None);
                }
                let root = Path::new(c["library_root"].as_str().unwrap());
                reject_links(root)?;
                if !root.is_dir() {
                    return Ok(None);
                }
                Ok(Some(destination(py, &data)?))
            })();
            match found {
                Ok(Some(value)) => result.push(value),
                Ok(None) => (),
                Err(e) if handled(py, &e) => (),
                Err(e) => return Err(e),
            }
        }
    }
    if live > 64 {
        return Err(invalid(
            "Browser discovery exceeds 64 live instances; no automatic destination is chosen.",
        ));
    }
    let mut keyed = Vec::new();
    for item in result {
        let key: String = item
            .bind(py)
            .getattr("label")?
            .call_method0("casefold")?
            .extract()?;
        let id: String = item.bind(py).getattr("instance_id")?.extract()?;
        keyed.push((key, id, item));
    }
    keyed.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    Ok(PyTuple::new(py, keyed.into_iter().map(|(_, _, v)| v))?
        .into_any()
        .unbind())
}
fn thread(py: Python<'_>, target: &Bound<'_, PyAny>, name: &str) -> PyResult<Py<PyAny>> {
    let kwargs = PyDict::new(py);
    kwargs.set_item("target", target)?;
    kwargs.set_item("name", name)?;
    kwargs.set_item("daemon", true)?;
    let thread = py
        .import("threading")?
        .getattr("Thread")?
        .call((), Some(&kwargs))?;
    thread.call_method0("start")?;
    Ok(thread.unbind())
}
#[pyfunction]
fn nxm_inbox_initialize(
    inbox: &Bound<'_, PyAny>,
    directory: PathBuf,
    family: &Bound<'_, PyAny>,
    value: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = inbox.py();
    if !cfg!(windows) || !family.is_none() && family.extract::<String>()? != "AF_PIPE" {
        return Err(invalid(
            "Browser instance routing requires the Windows named-pipe adapter.",
        ));
    }
    let c = if value.is_truthy()? {
        context(&decode(value)?)?
    } else {
        decode(&module(py)?.getattr("EMPTY")?)?
    };
    let folder = directory.join("nxm-instances");
    reject_links(&folder)?;
    fs::create_dir_all(&folder)?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    inbox.setattr("directory", py_path(py, &directory)?)?;
    inbox.setattr("instance_id", &id)?;
    inbox.setattr("state", py_path(py, &folder.join(format!("{id}.json")))?)?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("family", "AF_PIPE")?;
    let listener = py
        .import("multiprocessing.connection")?
        .getattr("Listener")?
        .call((format!(r"\\.\pipe\ModBox21-Nexus-{id}"),), Some(&kwargs))?;
    inbox.setattr("listener", &listener)?;
    inbox.setattr(
        "requests",
        py.import("queue")?.call_method1("Queue", (100,))?,
    )?;
    inbox.setattr("stop", py.import("threading")?.call_method0("Event")?)?;
    inbox.setattr("_lock", py.import("threading")?.call_method0("RLock")?)?;
    let process = py
        .import("creation_lib.mod_manager.process_identity")?
        .call_method0("current_identity")?;
    let data = json!({"format":1, "instance_id":id, "address":listener.getattr("address")?.extract::<String>()?, "process":decode(&process)?, "context":c});
    inbox.setattr("_state_data", object(py, &data)?)?;
    nxm_inbox_publish(inbox, object(py, &c)?.bind(py), true)?;
    inbox.setattr("thread", thread(py, &inbox.getattr("_run")?, "nxm-inbox")?)?;
    Ok(())
}
#[pyfunction]
fn nxm_inbox_destination(inbox: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    with_context(&inbox.getattr("_lock")?, |_| {
        destination(inbox.py(), &decode(&inbox.getattr("_state_data")?)?)
    })
}
#[pyfunction]
fn nxm_inbox_publish(
    inbox: &Bound<'_, PyAny>,
    value: &Bound<'_, PyAny>,
    force: bool,
) -> PyResult<()> {
    let c = context(&decode(value)?)?;
    with_context(&inbox.getattr("_lock")?, |_| {
        let mut data = decode(&inbox.getattr("_state_data")?)?;
        if force || data["context"] != c {
            data["context"] = c;
            inbox.setattr("_state_data", object(inbox.py(), &data)?)?;
            write(&inbox.getattr("state")?.extract::<PathBuf>()?, &data)?;
        }
        Ok(())
    })
}
fn receipt(inbox: &Bound<'_, PyAny>, id: &Value) -> PyResult<PathBuf> {
    let path = inbox
        .getattr("directory")?
        .extract::<PathBuf>()?
        .join("nxm-receipts")
        .join(format!("{}.json", identifier(id)?));
    reject_links(&path)?;
    Ok(path)
}
#[pyfunction]
fn nxm_inbox_mark(
    inbox: &Bound<'_, PyAny>,
    request: &Bound<'_, PyAny>,
    state: &str,
) -> PyResult<()> {
    let path = receipt(inbox, &decode(&request.getattr("id")?)?)?;
    with_context(&inbox.getattr("_lock")?, |_| {
        let mut value = read(&path)?;
        if value["instance_id"] != inbox.getattr("instance_id")?.extract::<String>()? {
            return Err(invalid("This window does not own that browser receipt."));
        }
        value["state"] = json!(state);
        write(&path, &value)
    })
}
#[pyfunction]
fn nxm_inbox_dispatch(inbox: &Bound<'_, PyAny>, raw: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = inbox.py();
    let data = decode(raw)?;
    if !data.is_object() {
        return object(py, &json!({"state":"rejected"}));
    }
    let id = identifier(&data["id"])?;
    let path = receipt(inbox, &data["id"])?;
    if data["command"] == "status" {
        return object(
            py,
            &if path.exists() {
                read(&path)?
            } else {
                json!({"state":"unknown"})
            },
        );
    }
    if data["command"] != "deliver"
        || data.as_object().unwrap().len() != 4
        || !data.get("context").is_some()
        || !data.get("url").is_some()
    {
        return object(py, &json!({"state":"rejected"}));
    }
    let link = py
        .import("creation_lib.mod_manager.nexus")?
        .call_method1("parse_nxm", (object(py, &data["url"])?,))?;
    link.call_method0("check_expiry")?;
    let identity = decode(&link.getattr("identity")?)?;
    if path.exists() {
        let mut previous = read(&path)?;
        if previous["identity"] != identity {
            return object(py, &json!({"state":"rejected"}));
        }
        if matches!(
            process_state(py, &previous["process"])?.as_str(),
            "exited" | "reused"
        ) {
            if previous["state"] == "accepted" {
                if read(&path)? == previous {
                    fs::remove_file(&path)?;
                }
            } else {
                previous["state"] = json!("delivered_closed");
                return object(py, &previous);
            }
        } else {
            return object(py, &previous);
        }
    }
    with_context(&inbox.getattr("_lock")?, |_| {
        let c = context(&data["context"])?;
        let current = decode(&inbox.getattr("_state_data")?)?;
        if c != current["context"]
            || c["ready"] != true
            || c["game"] != link.getattr("game")?.extract::<String>()?
        {
            return object(py, &json!({"state":"stale"}));
        }
        fs::create_dir_all(path.parent().unwrap())?;
        let value = json!({"state":"accepted", "id":id, "instance_id":current["instance_id"], "process":current["process"], "identity":identity, "context":c});
        let mut file = match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return object(py, &read(&path)?);
            }
            Err(e) => return Err(e.into()),
        };
        file.write_all(value.to_string().as_bytes())?;
        drop(file);
        let request =
            module(py)?
                .getattr("Request")?
                .call1((id, &link, nxm_inbox_destination(inbox)?))?;
        if let Err(e) = inbox
            .getattr("requests")?
            .call_method1("put_nowait", (request,))
        {
            if e.matches(py, &py.import("queue")?.getattr("Full")?)? {
                fs::remove_file(path)?;
                return object(py, &json!({"state":"full"}));
            }
            return Err(e);
        }
        object(py, &value)
    })
}
#[pyfunction]
fn nxm_inbox_run(inbox: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = inbox.py();
    let result = (|| {
        while !inbox.getattr("stop")?.call_method0("is_set")?.is_truthy()? {
            let result = (|| {
                with_context(
                    &inbox.getattr("listener")?.call_method0("accept")?,
                    |connection| {
                        let kwargs = PyDict::new(py);
                        kwargs.set_item("server", false)?;
                        py.import("creation_lib.mod_manager.pipe_peers")?
                            .getattr("peer")?
                            .call((connection,), Some(&kwargs))?;
                        if !connection.call_method1("poll", (2,))?.is_truthy()? {
                            return Ok(());
                        }
                        let raw = connection.call_method1("recv_bytes", (16384,))?;
                        if inbox.getattr("stop")?.call_method0("is_set")?.is_truthy()? {
                            return Ok(());
                        }
                        let response = py
                            .import("json")?
                            .call_method1("loads", (raw,))
                            .and_then(|v| inbox.call_method1("_dispatch", (v,)));
                        let response = match response {
                            Ok(v) => v.unbind(),
                            Err(e) if handled(py, &e) => object(py, &json!({"state":"rejected"}))?,
                            Err(e) => return Err(e),
                        };
                        let bytes: String = py
                            .import("json")?
                            .call_method1("dumps", (response,))?
                            .extract()?;
                        connection
                            .call_method1("send_bytes", (PyBytes::new(py, bytes.as_bytes()),))?;
                        Ok(())
                    },
                )
            })();
            if let Err(e) = result {
                if !transport_error(py, &e) {
                    return Err(e);
                }
            }
        }
        Ok(())
    })();
    inbox.getattr("listener")?.call_method0("close")?;
    result
}
#[pyclass]
struct Wake {
    address: Py<PyAny>,
}
#[pymethods]
impl Wake {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        let result = (|| {
            let kwargs = PyDict::new(py);
            kwargs.set_item("family", "AF_PIPE")?;
            let connection = py
                .import("multiprocessing.connection")?
                .getattr("Client")?
                .call((self.address.bind(py),), Some(&kwargs))?;
            with_context(&connection, |c| {
                c.call_method1("send_bytes", (PyBytes::new(py, b"{}"),))?;
                Ok(())
            })
        })();
        match result {
            Err(e)
                if e.is_instance_of::<pyo3::exceptions::PyOSError>(py)
                    || e.is_instance_of::<pyo3::exceptions::PyEOFError>(py) =>
            {
                Ok(())
            }
            result => result,
        }
    }
}
#[pyfunction]
fn nxm_inbox_close(inbox: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = inbox.py();
    inbox.getattr("stop")?.call_method0("set")?;
    let wake = Py::new(
        py,
        Wake {
            address: inbox.getattr("listener")?.getattr("address")?.unbind(),
        },
    )?;
    thread(py, wake.bind(py).as_any(), "nxm-close")?;
    inbox.getattr("thread")?.call_method1("join", (3,))?;
    let path: PathBuf = inbox.getattr("state")?.extract()?;
    reject_links(&path)?;
    let result = (|| -> PyResult<()> {
        if path.exists()
            && read(&path)?["instance_id"] == inbox.getattr("instance_id")?.extract::<String>()?
        {
            fs::remove_file(path)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        if !transport_error(py, &e) {
            return Err(e);
        }
    }
    loop {
        match inbox.getattr("requests")?.call_method0("get_nowait") {
            Ok(_) => (),
            Err(e) if e.matches(py, &py.import("queue")?.getattr("Empty")?)? => break,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
fn response(py: Python<'_>, value: &Value, request: &Bound<'_, PyAny>) -> PyResult<()> {
    let state = value["state"].as_str().unwrap_or("");
    if !value.is_object()
        || ![
            "accepted",
            "delivered",
            "delivered_closed",
            "stale",
            "full",
            "rejected",
            "unknown",
            "uncertain",
        ]
        .contains(&state)
    {
        return Err(invalid("Browser response is invalid."));
    }
    if ["accepted", "delivered", "delivered_closed"].contains(&state)
        || value.as_object().unwrap().len() != 1
    {
        if value.as_object().unwrap().len() != 6
            || value["id"] != request.getattr("id")?.extract::<String>()?
            || value["identity"] != decode(&request.getattr("link")?.getattr("identity")?)?
            || !valid_process(py, &value["process"])?
        {
            return Err(invalid("Browser receipt does not match the request."));
        }
        identifier(&value["instance_id"])?;
        context(&value["context"])?;
    }
    Ok(())
}
fn url(link: &Bound<'_, PyAny>) -> PyResult<String> {
    let py = link.py();
    let params = PyDict::new(py);
    if link.getattr("key")?.is_truthy()? {
        params.set_item("key", link.getattr("key")?)?;
        params.set_item("expires", link.getattr("expires")?)?;
    }
    if link.getattr("user_id")?.is_truthy()? {
        params.set_item("user_id", link.getattr("user_id")?)?;
    }
    let mut base = format!(
        "nxm://{}/mods/{}/files/{}",
        link.getattr("game")?.str()?,
        link.getattr("mod_id")?.str()?,
        link.getattr("file_id")?.str()?
    );
    if !params.is_empty() {
        base.push('?');
        base.push_str(
            &py.import("urllib.parse")?
                .call_method1("urlencode", (params,))?
                .extract::<String>()?,
        );
    }
    Ok(base)
}
#[pyclass]
struct Send {
    destination: Py<PyAny>,
    request: Py<PyAny>,
    result: Py<PyAny>,
    status_only: bool,
}
#[pymethods]
impl Send {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        let destination = self.destination.bind(py);
        let request = self.request.bind(py);
        let result = (|| -> PyResult<Value> {
            let kwargs = PyDict::new(py);
            kwargs.set_item("family", "AF_PIPE")?;
            let connection = py
                .import("multiprocessing.connection")?
                .getattr("Client")?
                .call((destination.getattr("address")?,), Some(&kwargs))?;
            with_context(&connection, |connection| {
                let kwargs = PyDict::new(py);
                kwargs.set_item("server", true)?;
                let peer = py
                    .import("creation_lib.mod_manager.pipe_peers")?
                    .getattr("peer")?
                    .call((connection,), Some(&kwargs))?;
                if !peer.eq(destination.getattr("process")?)? {
                    return Err(invalid("Browser pipe process changed."));
                }
                let mut data = json!({"command":if self.status_only {"status"} else {"deliver"}, "id":request.getattr("id")?.extract::<String>()?});
                if !self.status_only {
                    data["context"] = decode(&destination.getattr("context")?)?;
                    data["url"] = json!(url(&request.getattr("link")?)?);
                }
                connection.call_method1(
                    "send_bytes",
                    (PyBytes::new(py, data.to_string().as_bytes()),),
                )?;
                let value = if connection.call_method1("poll", (2,))?.is_truthy()? {
                    let raw = connection.call_method1("recv_bytes", (16384,))?;
                    decode(&py.import("json")?.call_method1("loads", (raw,))?)?
                } else {
                    json!({"state":"uncertain"})
                };
                response(py, &value, request)?;
                Ok(value)
            })
        })();
        let value = match result {
            Ok(v) => v,
            Err(e) if transport_error(py, &e) => json!({"state":"uncertain"}),
            Err(e) => return Err(e),
        };
        self.result
            .bind(py)
            .call_method1("put", (object(py, &value)?,))?;
        Ok(())
    }
}
#[pyfunction]
fn nxm_send(
    destination: &Bound<'_, PyAny>,
    request: &Bound<'_, PyAny>,
    status_only: bool,
) -> PyResult<Py<PyAny>> {
    let py = destination.py();
    let result = py.import("queue")?.call_method1("Queue", (1,))?;
    let target = Py::new(
        py,
        Send {
            destination: destination.clone().unbind(),
            request: request.clone().unbind(),
            result: result.clone().unbind(),
            status_only,
        },
    )?;
    thread(py, target.bind(py).as_any(), "nxm-route")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("timeout", 3)?;
    match result.call_method("get", (), Some(&kwargs)) {
        Ok(v) => Ok(v.unbind()),
        Err(e) if e.matches(py, &py.import("queue")?.getattr("Empty")?)? => {
            object(py, &json!({"state":"uncertain"}))
        }
        Err(e) => Err(e),
    }
}
#[pyfunction]
fn nxm_forward_link(py: Python<'_>, directory: PathBuf, raw: &str) -> PyResult<bool> {
    let m = module(py)?;
    let link = py
        .import("creation_lib.mod_manager.nexus")?
        .call_method1("parse_nxm", (raw,))?;
    link.call_method0("check_expiry")?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let mut request = m.getattr("Request")?.call1((&id, &link))?;
    let destinations = m.call_method1(
        "discover",
        (py_path(py, &directory)?, link.getattr("game")?),
    );
    let destinations = match destinations {
        Ok(v) => v,
        Err(e) if transport_error(py, &e) => PyTuple::empty(py).into_any(),
        Err(e) => return Err(e),
    };
    if destinations.len()? == 1 {
        let destination = destinations.get_item(0)?;
        let response = m.call_method1("send", (&destination, &request))?;
        let state = response.call_method1("get", ("state",))?;
        if state.eq("accepted")? || state.eq("delivered")? {
            m.setattr("_pending_startup", py.None())?;
            return Ok(true);
        }
        request = m
            .getattr("Request")?
            .call1((&id, &link, destination, state.eq("uncertain")?))?;
    }
    m.setattr("_pending_startup", request)?;
    Ok(false)
}
#[pyfunction]
fn nxm_take_startup(link: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = link.py();
    let m = module(py)?;
    let value = m.getattr("_pending_startup")?;
    m.setattr("_pending_startup", py.None())?;
    if value.is_truthy()? && value.getattr("link")?.eq(link)? {
        Ok(value.unbind())
    } else {
        Ok(m.getattr("Request")?
            .call1((uuid::Uuid::new_v4().simple().to_string(), link))?
            .unbind())
    }
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(nxm_destination_label, m)?)?;
    m.add_function(wrap_pyfunction!(nxm_discover, m)?)?;
    m.add_function(wrap_pyfunction!(nxm_inbox_initialize, m)?)?;
    m.add_function(wrap_pyfunction!(nxm_inbox_destination, m)?)?;
    m.add_function(wrap_pyfunction!(nxm_inbox_publish, m)?)?;
    m.add_function(wrap_pyfunction!(nxm_inbox_mark, m)?)?;
    m.add_function(wrap_pyfunction!(nxm_inbox_dispatch, m)?)?;
    m.add_function(wrap_pyfunction!(nxm_inbox_run, m)?)?;
    m.add_function(wrap_pyfunction!(nxm_inbox_close, m)?)?;
    m.add_function(wrap_pyfunction!(nxm_send, m)?)?;
    m.add_function(wrap_pyfunction!(nxm_forward_link, m)?)?;
    m.add_function(wrap_pyfunction!(nxm_take_startup, m)?)?;
    Ok(())
}
