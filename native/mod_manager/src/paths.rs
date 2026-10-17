use crate::{check_cancel, invalid, relative_path};
use pyo3::prelude::*;
use std::collections::BTreeSet;
use std::path::PathBuf;
use unicode_casefold::UnicodeCaseFold;

#[pyfunction]
#[pyo3(signature=(path, limit=8*1024*1024))]
pub fn metadata_read_text(path: &Bound<'_, PyAny>, limit: usize) -> PyResult<String> {
    let raw: PathBuf = path.extract()?;
    crate::reject_links(&raw)?;
    let initial = path.call_method0("stat")?;
    let exceeded = || {
        invalid(format!(
            "Metadata file exceeds {limit} bytes: {}",
            raw.display()
        ))
    };
    if initial.getattr("st_size")?.extract::<u64>()? > limit as u64 {
        return Err(exceeded());
    }
    let data = crate::python_io::with_context(&path.call_method1("open", ("rb",))?, |stream| {
        Ok(stream
            .call_method1("read", (limit.saturating_add(1),))?
            .unbind())
    })?;
    let data = data.bind(path.py());
    crate::reject_links(&raw)?;
    let current = path.call_method0("stat")?;
    let bytes = data.cast::<pyo3::types::PyBytes>()?.as_bytes();
    if bytes.len() > limit {
        return Err(exceeded());
    }
    for field in ["st_size", "st_mtime_ns", "st_ino"] {
        if !initial.getattr(field)?.eq(current.getattr(field)?)? {
            return Err(invalid(format!(
                "Metadata file changed while reading: {}",
                raw.display()
            )));
        }
    }
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        return data.call_method1("decode", ("utf-16",))?.extract();
    }
    match data.call_method1("decode", ("utf-8-sig",)) {
        Ok(text) => text.extract(),
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyUnicodeDecodeError>(path.py()) => {
            data.call_method1("decode", ("cp1252",))?.extract()
        }
        Err(e) => Err(e),
    }
}

#[pyfunction]
fn reject_links(path: PathBuf) -> PyResult<()> {
    crate::reject_links(&std::path::absolute(path)?)
}

#[pyfunction]
pub fn validate_destinations(paths: Vec<String>) -> PyResult<()> {
    let mut known = BTreeSet::new();
    for raw in paths {
        let path = relative_path(&raw, false)?.case_fold().collect::<String>();
        if !known.insert(path) {
            return Err(invalid(format!("Duplicate Windows destination: {raw}")));
        }
    }
    for path in &known {
        for (index, _) in path.match_indices('/') {
            if known.contains(&path[..index]) {
                return Err(invalid(format!(
                    "A file is also used as a directory: {path}"
                )));
            }
        }
    }
    Ok(())
}

#[pyclass]
pub struct FileTree {
    root: PathBuf,
    pending: Vec<PathBuf>,
    current: Option<std::fs::ReadDir>,
    directories: Option<Py<PyAny>>,
    cancel: Option<Py<PyAny>>,
    directory_limit: usize,
}

#[pymethods]
impl FileTree {
    #[new]
    #[pyo3(signature = (root, cancel=None, directories=None, directory_limit=200_000))]
    pub fn new(
        root: PathBuf,
        cancel: Option<Py<PyAny>>,
        directories: Option<Py<PyAny>>,
        directory_limit: usize,
    ) -> PyResult<Self> {
        crate::reject_links(&root)?;
        if !root.is_dir() {
            return Err(invalid(format!("Directory is missing: {}", root.display())));
        }
        Ok(Self {
            pending: vec![root.clone()],
            root,
            current: None,
            directories,
            cancel,
            directory_limit,
        })
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    pub fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<(String, PathBuf)>> {
        loop {
            check_cancel(self.cancel.as_ref().map(|v| v.bind(py)))?;
            if self.current.is_none() {
                let Some(path) = self.pending.pop() else {
                    return Ok(None);
                };
                crate::reject_links(&path)?;
                self.current = Some(std::fs::read_dir(path)?);
            }
            let Some(entry) = self.current.as_mut().unwrap().next() else {
                self.current = None;
                continue;
            };
            let path = entry?.path();
            crate::reject_links(&path)?;
            let info = std::fs::symlink_metadata(&path)?;
            let name = relative_path(
                &path.strip_prefix(&self.root).unwrap().to_string_lossy(),
                false,
            )?;
            if info.is_dir() {
                if let Some(directories) = &self.directories {
                    let directories = directories.bind(py);
                    directories.call_method1("add", (&name,))?;
                    if directories.len()? > self.directory_limit {
                        return Err(invalid("Directory count exceeds the package safety limit."));
                    }
                }
                self.pending.push(path);
            } else if info.is_file() {
                return Ok(Some((name, path)));
            } else {
                return Err(invalid(format!("Non-regular file: {}", path.display())));
            }
        }
    }
}

#[pyfunction]
fn manager_check_cancel(cancel: &Bound<'_, PyAny>) -> PyResult<()> {
    crate::check_cancel(Some(cancel))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(manager_check_cancel, m)?)?;
    m.add_function(wrap_pyfunction!(metadata_read_text, m)?)?;
    m.add_function(wrap_pyfunction!(reject_links, m)?)?;
    m.add_function(wrap_pyfunction!(validate_destinations, m)?)?;
    m.add_class::<FileTree>()?;
    Ok(())
}
