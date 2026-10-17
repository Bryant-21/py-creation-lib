use crate::{
    check_cancel, invalid, python_io::with_context, reject_links, relative_path, saves::fold,
};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict},
};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    time::Instant,
};
#[pyfunction]
fn sevenzip_executable(py: Python<'_>) -> PyResult<String> {
    let shutil = py.import("shutil")?;
    for name in ["7z", "7zz", "7za"] {
        let found = shutil.call_method1("which", (name,))?;
        if found.is_truthy()? {
            return found.extract();
        }
    }
    Err(invalid(
        "7z/RAR support requires an installed 7-Zip or NanaZip command-line tool on PATH.",
    ))
}
fn mode(attributes: &str, kinds: &str) -> bool {
    attributes.split_whitespace().any(|s| {
        s.len() >= 10
            && kinds.as_bytes().contains(&s.as_bytes()[0])
            && s.as_bytes()[1..10].iter().all(|c| b"rwx-".contains(c))
    })
}
#[pyfunction]
fn sevenzip_listing(
    data: &[u8],
    limits: &Bound<'_, PyAny>,
    archive_bytes: u64,
) -> PyResult<Vec<(String, u64, String)>> {
    let text = std::str::from_utf8(data)
        .map_err(|_| invalid("Archive listing is not valid UTF-8."))?
        .trim_start_matches('\u{feff}');
    let text = text
        .lines()
        .map(|s| s.trim_end_matches('\r'))
        .collect::<Vec<_>>()
        .join("\n");
    let (header, body) = text
        .split_once("----------\n")
        .ok_or_else(|| invalid("Archive reader returned an unsupported listing format."))?;
    for line in header.lines() {
        if let Some((key, value)) = line.split_once(" = ") {
            if (matches!(
                key.to_ascii_lowercase().as_str(),
                "volumes" | "volume index"
            ) && !value.is_empty()
                && value.bytes().all(|c| c.is_ascii_digit())
                && value != "0"
                && value != "1")
                || (key.eq_ignore_ascii_case("Multivolume") && value == "+")
            {
                return Err(invalid(
                    "Multi-volume archives are unsupported; choose a complete single archive.",
                ));
            }
        }
    }
    let files: usize = limits.getattr("files")?.extract()?;
    let file_bytes: u64 = limits.getattr("file_bytes")?.extract()?;
    let total_bytes: u64 = limits.getattr("total_bytes")?.extract()?;
    let ratio: u64 = limits.getattr("ratio")?.extract()?;
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut expanded = 0u64;
    for block in body.trim_matches('\n').split("\n\n") {
        if block.trim().is_empty() {
            continue;
        }
        let mut properties = BTreeMap::new();
        for line in block.lines() {
            let (key, value) = line.split_once(" = ").ok_or_else(|| {
                invalid("Ambiguous archive metadata or control characters in a filename.")
            })?;
            if properties.insert(key, value).is_some() {
                return Err(invalid("Duplicate archive metadata fields are unsafe."));
            }
        }
        let get = |key| *properties.get(key).unwrap_or(&"");
        let name = relative_path(get("Path"), false)?;
        if !seen.insert(fold(&name)) {
            return Err(invalid(format!(
                "Duplicate archive path on Windows: {name}"
            )));
        }
        if seen.len() > files.saturating_mul(2) {
            return Err(invalid("Archive entry count exceeds the safety limit."));
        }
        let attributes = get("Attributes");
        if properties
            .iter()
            .any(|(key, value)| fold(key).contains("link") && !value.is_empty())
            || mode(attributes, "lbcps")
            || fold(attributes).contains("reparse")
            || get("Anti") == "+"
            || get("Alternate Stream") == "+"
        {
            return Err(invalid(format!(
                "Archive contains a link, stream, anti-item, or special file: {name}"
            )));
        }
        if get("Encrypted") == "+" {
            return Err(invalid("Encrypted archives are unsupported."));
        }
        if get("Split Before") == "+" || get("Split After") == "+" {
            return Err(invalid(
                "Split archive members are unsupported; choose a complete single archive.",
            ));
        }
        if get("Folder") == "+" || attributes.starts_with('D') || mode(attributes, "d") {
            continue;
        }
        let size = get("Size");
        let packed = get("Packed Size");
        if size.is_empty()
            || !size.bytes().all(|c| c.is_ascii_digit())
            || !packed.bytes().all(|c| c.is_ascii_digit())
        {
            return Err(invalid("Archive has invalid file sizes."));
        }
        let size: u64 = size
            .parse()
            .map_err(|_| invalid("Archive exceeds file-count or expanded-size limits."))?;
        let packed: u64 = if packed.is_empty() {
            0
        } else {
            packed
                .parse()
                .map_err(|_| invalid("Archive has invalid file sizes."))?
        };
        expanded = expanded
            .checked_add(size)
            .ok_or_else(|| invalid("Archive exceeds file-count or expanded-size limits."))?;
        if entries.len() >= files || size > file_bytes || expanded > total_bytes {
            return Err(invalid(
                "Archive exceeds file-count or expanded-size limits.",
            ));
        }
        if packed != 0 && size > packed.saturating_mul(ratio) {
            return Err(invalid(format!(
                "Suspicious archive expansion ratio: {name}"
            )));
        }
        entries.push((name.clone(), size, name));
    }
    if expanded > archive_bytes.max(1).saturating_mul(ratio) {
        return Err(invalid("Suspicious total archive expansion ratio."));
    }
    Ok(entries)
}
#[pyclass]
struct ArchiveWatch {
    child: Py<PyAny>,
    cancel: Py<PyAny>,
    stop: Py<PyAny>,
    expired: Py<PyAny>,
    timeout: f64,
}
#[pymethods]
impl ArchiveWatch {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        let start = Instant::now();
        while !self
            .stop
            .bind(py)
            .call_method1("wait", (0.05,))?
            .is_truthy()?
        {
            let cancel = self.cancel.bind(py);
            if (!cancel.is_none() && cancel.call_method0("is_set")?.is_truthy()?)
                || start.elapsed().as_secs_f64() >= self.timeout
            {
                self.expired.bind(py).call_method0("set")?;
                if let Err(error) = self.child.bind(py).call_method0("kill") {
                    if !error.is_instance_of::<pyo3::exceptions::PyOSError>(py) {
                        return Err(error);
                    }
                }
                return Ok(());
            }
        }
        Ok(())
    }
}
#[pyclass]
struct ArchiveProcess {
    child: Py<PyAny>,
    cancel: Py<PyAny>,
    stop: Py<PyAny>,
    expired: Py<PyAny>,
    watcher: Py<PyAny>,
}
#[pymethods]
impl ArchiveProcess {
    fn __enter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(self.child.bind(py).getattr("stdout")?.unbind())
    }
    fn __exit__(
        &self,
        py: Python<'_>,
        kind: &Bound<'_, PyAny>,
        _error: &Bound<'_, PyAny>,
        _trace: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let child = self.child.bind(py);
        let result = (|| -> PyResult<()> {
            if !kind.is_none() {
                return Ok(());
            }
            child.getattr("stdout")?.call_method0("close")?;
            let status = child.call_method1("wait", (3,))?.is_truthy()?;
            check_cancel(Some(self.cancel.bind(py)))?;
            if self.expired.bind(py).call_method0("is_set")?.is_truthy()? {
                return Err(invalid("Archive operation exceeded its time limit."));
            }
            if status {
                return Err(invalid(
                    "Archive reader failed. The archive may be corrupt, encrypted, or unsupported.",
                ));
            }
            Ok(())
        })();
        self.stop.bind(py).call_method0("set")?;
        if child.call_method0("poll")?.is_none() {
            child.call_method0("kill")?;
        }
        child.getattr("stdout")?.call_method0("close")?;
        child.call_method1("wait", (3,))?;
        self.watcher.bind(py).call_method1("join", (1,))?;
        result
    }
}
#[pyfunction]
fn archive_process(
    arguments: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    timeout: f64,
) -> PyResult<ArchiveProcess> {
    let py = arguments.py();
    check_cancel(Some(cancel))?;
    let subprocess = py.import("subprocess")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("stdin", subprocess.getattr("DEVNULL")?)?;
    kwargs.set_item("stdout", subprocess.getattr("PIPE")?)?;
    kwargs.set_item("stderr", subprocess.getattr("DEVNULL")?)?;
    kwargs.set_item("shell", false)?;
    kwargs.set_item(
        "creationflags",
        if cfg!(windows) { 0x08000000u32 } else { 0 },
    )?;
    let child = subprocess
        .getattr("Popen")?
        .call((arguments,), Some(&kwargs))?;
    let threading = py.import("threading")?;
    let stop = threading.getattr("Event")?.call0()?;
    let expired = threading.getattr("Event")?.call0()?;
    let watch = Py::new(
        py,
        ArchiveWatch {
            child: child.clone().unbind(),
            cancel: cancel.clone().unbind(),
            stop: stop.clone().unbind(),
            expired: expired.clone().unbind(),
            timeout,
        },
    )?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("target", watch)?;
    kwargs.set_item("name", "archive-watch")?;
    kwargs.set_item("daemon", true)?;
    let watcher = threading.getattr("Thread")?.call((), Some(&kwargs))?;
    if let Err(error) = watcher.call_method0("start") {
        child.call_method0("kill")?;
        child.getattr("stdout")?.call_method0("close")?;
        child.call_method1("wait", (3,))?;
        return Err(error);
    }
    Ok(ArchiveProcess {
        child: child.unbind(),
        cancel: cancel.clone().unbind(),
        stop: stop.unbind(),
        expired: expired.unbind(),
        watcher: watcher.unbind(),
    })
}
#[pyfunction]
fn sevenzip_initialize(
    seven: &Bound<'_, PyAny>,
    archive: &Bound<'_, PyAny>,
    limits: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<()> {
    use std::io::Read;
    let py = seven.py();
    let path = py.import("pathlib")?.getattr("Path")?.call1((archive,))?;
    let root: PathBuf = path.extract()?;
    reject_links(&root)?;
    seven.setattr("archive", &path)?;
    seven.setattr("cancel", cancel)?;
    let mut signature = [0; 8];
    let count = std::fs::File::open(&root)?.read(&mut signature)?;
    if !signature[..count].starts_with(b"7z\xbc\xaf\x27\x1c")
        && !signature[..count].starts_with(b"Rar!\x1a\x07")
    {
        return Err(invalid(
            "Expected a 7z or RAR archive; executable/self-extracting packages are unsupported.",
        ));
    }
    let tool = sevenzip_executable(py)?;
    seven.setattr("tool", &tool)?;
    let args = vec![
        tool,
        "l".into(),
        "-slt".into(),
        "-sccUTF-8".into(),
        "-p-".into(),
        "--".into(),
        root.to_string_lossy().into_owned(),
    ];
    let context = Py::new(
        py,
        archive_process(pyo3::types::PyList::new(py, args)?.as_any(), cancel, 30.0)?,
    )?;
    let data: Vec<u8> = with_context(context.bind(py).as_any(), |stream| {
        let data: Vec<u8> = stream
            .call_method1("read", (16 * 1024 * 1024 + 1,))?
            .extract()?;
        if data.len() > 16 * 1024 * 1024 {
            return Err(invalid("Archive metadata exceeds the 16-MiB limit."));
        }
        Ok(data)
    })?;
    seven.setattr(
        "entries",
        sevenzip_listing(&data, limits, std::fs::metadata(root)?.len())?,
    )?;
    Ok(())
}
#[pyfunction]
fn sevenzip_open(
    seven: &Bound<'_, PyAny>,
    source: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<ArchiveProcess> {
    let py = seven.py();
    let name = relative_path(source, false)?;
    let args = vec![
        seven.getattr("tool")?.extract::<String>()?,
        "x".into(),
        "-so".into(),
        "-y".into(),
        "-bd".into(),
        "-bsp0".into(),
        "-spd".into(),
        "-r-".into(),
        "-p-".into(),
        format!("-i!{name}"),
        "--".into(),
        seven.getattr("archive")?.str()?.to_str()?.into(),
    ];
    let cancel = if cancel.is_truthy()? {
        cancel.clone()
    } else {
        seven.getattr("cancel")?
    };
    archive_process(pyo3::types::PyList::new(py, args)?.as_any(), &cancel, 120.0)
}
#[pyfunction]
fn sevenzip_read_config(seven: &Bound<'_, PyAny>, source: &str) -> PyResult<Py<PyAny>> {
    let py = seven.py();
    let data: Vec<u8> = with_context(&seven.call_method1("open", (source,))?, |stream| {
        let data: Vec<u8> = stream
            .call_method1("read", (2 * 1024 * 1024 + 1,))?
            .extract()?;
        if data.len() > 2 * 1024 * 1024 {
            return Err(invalid("FOMOD XML exceeds the 2-MiB limit."));
        }
        Ok(data)
    })?;
    Ok(py
        .import("io")?
        .getattr("BytesIO")?
        .call1((PyBytes::new(py, &data),))?
        .unbind())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(sevenzip_executable, m)?)?;
    m.add_function(wrap_pyfunction!(sevenzip_listing, m)?)?;
    m.add_function(wrap_pyfunction!(archive_process, m)?)?;
    m.add_function(wrap_pyfunction!(sevenzip_initialize, m)?)?;
    m.add_function(wrap_pyfunction!(sevenzip_open, m)?)?;
    m.add_function(wrap_pyfunction!(sevenzip_read_config, m)?)?;
    Ok(())
}
