use crate::{
    check_cancel, encoding, history_prune::py_path, invalid, python_io::with_context, reject_links,
};
use pyo3::prelude::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const PAYLOAD: [&str; 3] = [
    "modbox_vfs.dll",
    "MinHook-LICENSE.txt",
    "Project-LICENSE.txt",
];
#[pyclass]
struct BuildLock {
    file: Option<File>,
}
#[pymethods]
impl BuildLock {
    fn __enter__(&self) {}
    fn __exit__(
        &mut self,
        _kind: &Bound<'_, PyAny>,
        _error: &Bound<'_, PyAny>,
        _trace: &Bound<'_, PyAny>,
    ) {
        self.file.take();
    }
}
#[pyfunction]
fn native_build_lock(root: PathBuf, cancel: &Bound<'_, PyAny>) -> PyResult<BuildLock> {
    reject_links(&root)?;
    std::fs::create_dir_all(&root)?;
    let path = root.join("build.lock");
    reject_links(&path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(&path)?;
    if file.metadata()?.len() == 0 {
        file.write_all(b"0")?;
        file.flush()?;
    }
    let start = Instant::now();
    loop {
        check_cancel(Some(cancel))?;
        match file.try_lock() {
            Ok(()) => break,
            Err(_) => {
                if start.elapsed() > Duration::from_secs(60) {
                    return Err(invalid(
                        "Another app instance is building the project filespace. Retry shortly.",
                    ));
                }
                cancel
                    .py()
                    .detach(|| std::thread::sleep(Duration::from_millis(20)));
            }
        }
    }
    Ok(BuildLock { file: Some(file) })
}
#[pyfunction]
fn native_source_fingerprint(source: PathBuf, py: Python<'_>) -> PyResult<String> {
    let source = py_path(py, &source)?;
    let paths = py
        .import("builtins")?
        .getattr("sorted")?
        .call1((source.call_method1("rglob", ("*",))?,))?;
    let mut digest = Sha256::new();
    digest.update(b"ModBox21 native filespace ABI1 /MT x64\n");
    for path in paths.try_iter()? {
        let path = path?;
        if path.call_method0("is_file")?.is_truthy()? {
            digest.update(
                path.call_method1("relative_to", (&source,))?
                    .str()?
                    .to_str()?
                    .as_bytes(),
            );
            let bytes: Vec<u8> = path.call_method0("read_bytes")?.extract()?;
            digest.update(bytes);
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn fingerprint(py: Python<'_>) -> PyResult<String> {
    py.import("creation_lib.mod_manager.native_builder")?
        .call_method0("source_fingerprint")?
        .extract()
}
fn read_json(path: &Path) -> PyResult<Value> {
    encoding::parse(&std::fs::read_to_string(path)?)
}
#[pyfunction]
fn native_build_validate(root: PathBuf, cancel: &Bound<'_, PyAny>) -> PyResult<PathBuf> {
    let py = cancel.py();
    let root: PathBuf = py_path(py, &root)?.call_method0("absolute")?.extract()?;
    reject_links(&root)?;
    reject_links(&root.join("native-receipt.json"))?;
    for name in PAYLOAD {
        reject_links(&root.join(name))?;
    }
    let receipt = root.join("native-receipt.json");
    if !receipt.is_file() || std::fs::metadata(&receipt)?.len() > 4096 {
        return Err(invalid(
            "The app's project filespace payload is missing its build receipt.",
        ));
    }
    let record = read_json(&receipt)?;
    if record["abi"] != 1 || record["source"] != fingerprint(py)? {
        return Err(invalid(
            "The app's native filespace build differs from its source version. Repair/rebuild this app.",
        ));
    }
    for name in PAYLOAD {
        check_cancel(Some(cancel))?;
        let path = root.join(name);
        if !path.is_file()
            || std::fs::metadata(&path)?.len() > 16 * 1024 * 1024
            || record[name] != format!("{:x}", Sha256::digest(std::fs::read(path)?))
        {
            return Err(invalid(format!(
                "The project's native filespace payload is missing/corrupt: {name}"
            )));
        }
    }
    if py
        .import("creation_lib.mod_manager.runtime")?
        .call_method1("pe_machine", (py_path(py, &root.join("modbox_vfs.dll"))?,))?
        .extract::<u32>()?
        != 0x8664
    {
        return Err(invalid("The project's native filespace must be x64."));
    }
    Ok(root)
}
#[pyfunction]
fn native_build_command(
    argv: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    _progress: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = argv.py();
    let temp = py.import("tempfile")?;
    with_context(&temp.call_method0("TemporaryFile")?, |output| {
        with_context(&temp.call_method0("TemporaryFile")?, |errors| {
            let child = py
                .import("creation_lib.mod_manager.read_process")?
                .getattr("ReadProcess")?
                .call1((argv, output, errors))?;
            let result = (|| -> PyResult<()> {
                let start = Instant::now();
                while child.call_method0("poll")?.is_none() {
                    check_cancel(Some(cancel))?;
                    if start.elapsed() > Duration::from_secs(240) {
                        return Err(invalid("Native filespace build exceeded four minutes."));
                    }
                    py.detach(|| std::thread::sleep(Duration::from_millis(50)));
                }
                if child.getattr("returncode")?.is_truthy()? {
                    errors.call_method1("seek", (0,))?;
                    output.call_method1("seek", (0,))?;
                    let mut bytes: Vec<u8> = errors.call_method1("read", (32000,))?.extract()?;
                    bytes.extend(
                        output
                            .call_method1("read", (32000,))?
                            .extract::<Vec<u8>>()?,
                    );
                    let text = String::from_utf8_lossy(&bytes);
                    let detail = text
                        .chars()
                        .skip(text.chars().count().saturating_sub(8000))
                        .collect::<String>();
                    return Err(invalid(format!("Project filespace build failed: {detail}")));
                }
                Ok(())
            })();
            child.call_method0("close")?;
            result
        })
    })
}
#[pyfunction]
fn native_build_ensure(
    source: PathBuf,
    packaged: PathBuf,
    cache: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<PathBuf> {
    let py = cache.py();
    if !cfg!(windows) {
        return Err(invalid("Project filespace requires x64 Windows."));
    }
    let app = py.import("app.paths")?;
    if app.call_method0("is_frozen")?.is_truthy()? {
        return native_build_validate(packaged, cancel);
    }
    let cache = if cache.is_truthy()? {
        cache.extract::<PathBuf>()?
    } else {
        app.call_method0("get_db_dir")?
            .extract::<PathBuf>()?
            .join("mod_manager")
            .join("dependencies")
            .join("project-vfs")
    };
    let cache: PathBuf = py_path(py, &cache)?.call_method0("absolute")?.extract()?;
    let fingerprint = fingerprint(py)?;
    let _lock = native_build_lock(cache.clone(), cancel)?;
    let pointer = cache.join(format!("{fingerprint}.json"));
    reject_links(&pointer)?;
    let cached = (|| -> PyResult<PathBuf> {
        if !pointer.is_file() || std::fs::metadata(&pointer)?.len() > 4096 {
            return Err(invalid("No current native build."));
        }
        let record = read_json(&pointer)?;
        let generation = record["generation"].as_str().unwrap_or("");
        if generation.len() != 32
            || !generation
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(invalid("Invalid native build pointer."));
        }
        native_build_validate(cache.join(generation), cancel)
    })();
    match cached {
        Ok(root) => return Ok(root),
        Err(e)
            if e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
                || e.is_instance_of::<pyo3::exceptions::PyOSError>(py)
                || e.is_instance_of::<pyo3::exceptions::PyKeyError>(py) =>
        {
            check_cancel(Some(cancel))?
        }
        Err(e) => return Err(e),
    }
    let cmake = py.import("shutil")?.call_method1("which", ("cmake",))?;
    if !cmake.is_truthy()? {
        return Err(invalid(
            "Development needs CMake and MSVC x64 BuildTools to build the project filespace. Packaged apps include it.",
        ));
    }
    let cmake: String = cmake.extract()?;
    let build = cache.join("b").join(&fingerprint[..16]);
    if progress.is_truthy()? {
        progress.call1((
            py.None(),
            "Building this project's native virtual filespace (offline)",
        ))?;
    }
    let module = py.import("creation_lib.mod_manager.native_builder")?;
    module.call_method1(
        "_command",
        (
            vec![
                cmake.clone(),
                "-S".into(),
                source.to_string_lossy().into_owned(),
                "-B".into(),
                build.to_string_lossy().into_owned(),
                "-G".into(),
                "Visual Studio 18 2026".into(),
                "-A".into(),
                "x64".into(),
                "-DMODBOX_VFS_TESTING=OFF".into(),
            ],
            cancel,
            progress,
        ),
    )?;
    module.call_method1(
        "_command",
        (
            vec![
                cmake,
                "--build".into(),
                build.to_string_lossy().into_owned(),
                "--config".into(),
                "Release".into(),
            ],
            cancel,
            progress,
        ),
    )?;
    let generation = uuid::Uuid::new_v4().simple().to_string();
    let root = cache.join(&generation);
    std::fs::create_dir(&root)?;
    for (input, name) in [
        (build.join("Release").join("modbox_vfs.dll"), PAYLOAD[0]),
        (
            source.join("vendor").join("minhook").join("LICENSE.txt"),
            PAYLOAD[1],
        ),
        (source.join("LICENSE.txt"), PAYLOAD[2]),
    ] {
        check_cancel(Some(cancel))?;
        reject_links(&input)?;
        reject_links(&root.join(name))?;
        std::fs::copy(input, root.join(name))?;
    }
    let mut receipt = json!({"abi":1,"source":fingerprint});
    for name in PAYLOAD {
        receipt[name] = json!(format!(
            "{:x}",
            Sha256::digest(std::fs::read(root.join(name))?)
        ));
    }
    std::fs::write(root.join("native-receipt.json"), receipt.to_string())?;
    native_build_validate(root.clone(), cancel)?;
    check_cancel(Some(cancel))?;
    let temporary = cache.join(format!("{generation}.json"));
    std::fs::write(&temporary, json!({"generation":generation}).to_string())?;
    py_path(py, &temporary)?.call_method1("replace", (py_path(py, &pointer)?,))?;
    Ok(root)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(native_build_lock, m)?)?;
    m.add_function(wrap_pyfunction!(native_source_fingerprint, m)?)?;
    m.add_function(wrap_pyfunction!(native_build_validate, m)?)?;
    m.add_function(wrap_pyfunction!(native_build_command, m)?)?;
    m.add_function(wrap_pyfunction!(native_build_ensure, m)?)?;
    Ok(())
}
