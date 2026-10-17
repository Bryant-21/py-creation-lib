use crate::{check_cancel, invalid, reject_links, relative_path};
use pyo3::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use unicode_casefold::UnicodeCaseFold;

fn folded(value: &str) -> String {
    value.case_fold().collect()
}

#[pyfunction]
pub fn installer_metadata(name: &str) -> bool {
    let name = folded(name);
    name == "meta.ini" || name.split('/').any(|part| part == "fomod")
}

#[pyfunction]
fn package_signature(path: PathBuf) -> PyResult<(u64, i128)> {
    signature(&path)
}

pub fn signature(path: &Path) -> PyResult<(u64, i128)> {
    let info = std::fs::metadata(path)?;
    let time = info.modified()?;
    let nanos = match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos() as i128,
        Err(error) => -(error.duration().as_nanos() as i128),
    };
    Ok((info.len(), nanos))
}

#[pyclass]
pub struct PackageIndex {
    path: PathBuf,
    files: BTreeMap<String, String>,
    sizes: BTreeMap<String, u64>,
    signatures: BTreeMap<String, (u64, i128)>,
    archive_signature: Option<(u64, i128)>,
    count_limit: usize,
    file_limit: u64,
    total_limit: u64,
    ratio: u64,
    total: u64,
    #[pyo3(get)]
    config_path: String,
    #[pyo3(get)]
    prefix: String,
}

pub fn directory(
    py: Python<'_>,
    path: PathBuf,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<PackageIndex> {
    Ok(directory_inventory(py, path, cancel)?.0)
}

pub fn directory_inventory(
    py: Python<'_>,
    path: PathBuf,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<(PackageIndex, BTreeSet<String>)> {
    let mut package = PackageIndex::new(
        path.clone(),
        500_000,
        64 * 1024u64.pow(3),
        256 * 1024u64.pow(3),
        2000,
        false,
    )?;
    let directories = pyo3::types::PySet::empty(py)?;
    let mut tree = crate::paths::FileTree::new(
        path,
        cancel.map(|v| v.clone().unbind()),
        Some(directories.clone().into_any().unbind()),
        1_000_000,
    )?;
    while let Some((name, path)) = tree.__next__(py)? {
        package.add(&name, std::fs::metadata(path)?.len())?;
    }
    package.finish(false)?;
    Ok((package, directories.extract()?))
}

#[pymethods]
impl PackageIndex {
    #[new]
    pub fn new(
        path: PathBuf,
        count_limit: usize,
        file_limit: u64,
        total_limit: u64,
        ratio: u64,
        archive: bool,
    ) -> PyResult<Self> {
        reject_links(&path)?;
        let archive_signature = if archive {
            Some(signature(&path)?)
        } else {
            None
        };
        Ok(Self {
            path,
            files: BTreeMap::new(),
            sizes: BTreeMap::new(),
            signatures: BTreeMap::new(),
            archive_signature,
            count_limit,
            file_limit,
            total_limit,
            ratio,
            total: 0,
            config_path: String::new(),
            prefix: String::new(),
        })
    }

    pub fn add(&mut self, raw: &str, size: u64) -> PyResult<String> {
        let name = relative_path(raw, false)?;
        let key = folded(&name);
        if self.files.contains_key(&key) {
            return Err(invalid(format!(
                "Duplicate archive/folder path on Windows: {name}"
            )));
        }
        let total = self.total.checked_add(size).ok_or_else(|| {
            invalid("Package exceeds the configured file-count or expanded-size limit.")
        })?;
        if self.files.len() >= self.count_limit
            || size > self.file_limit
            || total > self.total_limit
        {
            return Err(invalid(
                "Package exceeds the configured file-count or expanded-size limit.",
            ));
        }
        if self.archive_signature.is_none() {
            self.signatures
                .insert(key.clone(), signature(&self.path.join(&name))?);
        }
        self.total = total;
        self.files.insert(key.clone(), name);
        self.sizes.insert(key.clone(), size);
        Ok(key)
    }

    fn zip_entry(
        &self,
        name: &str,
        size: u64,
        compressed: u64,
        external: u32,
        flags: u16,
        directory: bool,
        count: usize,
    ) -> PyResult<bool> {
        if count > self.count_limit.saturating_mul(2) {
            return Err(invalid("Archive entry count exceeds the safety limit."));
        }
        relative_path(name, false)?;
        let kind = (external >> 16) & 0o170000;
        if !matches!(kind, 0 | 0o100000 | 0o040000) {
            return Err(invalid(format!(
                "Archive contains a link or special file: {name}"
            )));
        }
        if flags & 1 != 0 {
            return Err(invalid("Encrypted archives are unsupported."));
        }
        if directory {
            return Ok(false);
        }
        if size > compressed.max(1).saturating_mul(self.ratio) {
            return Err(invalid(format!(
                "Suspicious archive expansion ratio: {name}"
            )));
        }
        Ok(true)
    }

    pub fn finish(&mut self, inspect_installer: bool) -> PyResult<()> {
        crate::paths::validate_destinations(self.files.values().cloned().collect())?;
        if !inspect_installer {
            return Ok(());
        }
        let configs: Vec<_> = self
            .files
            .values()
            .filter(|name| {
                let name = folded(name);
                name.ends_with("fomod/moduleconfig.xml")
                    && (name == "fomod/moduleconfig.xml" || name.contains("/fomod/"))
            })
            .collect();
        if configs.len() > 1 {
            return Err(invalid(
                "Multiple FOMOD configurations found; choose a single package root.",
            ));
        }
        if let Some(name) = configs.first() {
            self.config_path = (*name).clone();
            self.prefix = name[..name.len() - "fomod/ModuleConfig.xml".len()].into();
        }
        if self.files.values().any(|name| {
            let name = folded(name);
            name.ends_with("fomod/script.cs") || name.ends_with("fomod/script.py")
        }) {
            return Err(invalid(
                "Executable FOMOD scripts require a separate sandboxed backend and cannot run here.",
            ));
        }
        Ok(())
    }

    #[getter]
    pub fn files(&self) -> BTreeMap<String, String> {
        self.files.clone()
    }

    pub fn available(&self, source: &str, folder: bool) -> bool {
        let source = folded(source);
        if !folder {
            return self.files.contains_key(&source);
        }
        let prefix = format!("{}/", source.trim_end_matches('/'));
        self.files
            .range(prefix.clone()..)
            .next()
            .is_some_and(|(name, _)| name.starts_with(&prefix))
    }
    #[getter]
    pub fn sizes(&self) -> BTreeMap<String, u64> {
        self.sizes.clone()
    }
    #[getter]
    pub fn signatures(&self) -> BTreeMap<String, (u64, i128)> {
        self.signatures.clone()
    }

    #[pyo3(signature = (root="", cancel=None, allow_empty=false))]
    pub fn plain_plan(
        &self,
        root: &str,
        cancel: Option<&Bound<'_, PyAny>>,
        allow_empty: bool,
    ) -> PyResult<BTreeMap<String, String>> {
        let root = relative_path(root, true)?;
        let prefix = if root.is_empty() {
            root
        } else {
            format!("{root}/")
        };
        let mut result = BTreeMap::new();
        for name in self.files.values() {
            check_cancel(cancel)?;
            if !folded(name).starts_with(&folded(&prefix)) {
                continue;
            }
            // Character counts preserve the Python path-slicing contract for Unicode roots.
            let destination: String = name.chars().skip(prefix.chars().count()).collect();
            if installer_metadata(&destination)
                || destination
                    .split('/')
                    .any(|part| folded(part).ends_with(".mohidden"))
            {
                continue;
            }
            result.insert(destination, name.clone());
        }
        if result.is_empty() && !allow_empty {
            return Err(invalid(
                "No installable files at the selected content root.",
            ));
        }
        Ok(result)
    }

    pub fn source(&self, source: &str, after: bool) -> PyResult<String> {
        let key = folded(&relative_path(source, false)?);
        let Some(name) = self.files.get(&key) else {
            return Err(invalid(format!("Installer source is missing: {source}")));
        };
        if let Some(expected) = self.archive_signature {
            reject_links(&self.path)?;
            if signature(&self.path)? != expected {
                return Err(invalid(if after {
                    "Archive changed during copy; reopen it before installing."
                } else {
                    "Archive changed after the package preview; reopen it before installing."
                }));
            }
        } else {
            let path = self.path.join(name);
            reject_links(&path)?;
            if signature(&path)? != self.signatures[&key] {
                return Err(invalid(format!(
                    "Source changed {}: {source}",
                    if after {
                        "during copy"
                    } else {
                        "after the package preview"
                    }
                )));
            }
        }
        Ok(key)
    }

    #[pyo3(signature = (sources, cancel=None))]
    fn validate_sources(
        &self,
        sources: Vec<String>,
        cancel: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        reject_links(&self.path)?;
        let result = (|| {
            if let Some(expected) = self.archive_signature {
                if signature(&self.path)? != expected {
                    return Err(invalid(
                        "Archive changed after preview; reopen it before reviewing or installing.",
                    ));
                }
                return Ok(());
            }
            let mut checked: BTreeSet<_> = sources.into_iter().collect();
            if !self.config_path.is_empty() {
                checked.insert(self.config_path.clone());
            }
            for source in checked {
                check_cancel(cancel)?;
                let key = folded(&relative_path(&source, false)?);
                let Some(name) = self.files.get(&key) else {
                    return Err(invalid(
                        "Reviewed package source is unavailable; reopen the package.",
                    ));
                };
                let file = self.path.join(name);
                reject_links(&file)?;
                if !file.is_file() || signature(&file)? != self.signatures[&key] {
                    return Err(invalid(format!(
                        "Source changed after preview: {source}. Reopen and review the package."
                    )));
                }
            }
            Ok(())
        })();
        result.map_err(|error: PyErr| Python::attach(|py| {
            if error.is_instance_of::<pyo3::exceptions::PyOSError>(py) { invalid("Package source disappeared or became unreadable. Reopen and review the package.") } else { error }
        }))
    }
}

#[pyfunction]
fn package_initialize(
    package: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    limits: &Bound<'_, PyAny>,
    inspect_installer: bool,
) -> PyResult<()> {
    let py = package.py();
    let path = py
        .import("pathlib")?
        .getattr("Path")?
        .call1((path,))?
        .call_method0("absolute")?;
    let root: PathBuf = path.extract()?;
    reject_links(&root)?;
    package.setattr("path", &path)?;
    package.setattr("_cancel", cancel)?;
    package.setattr("_zip", py.None())?;
    package.setattr("_seven", py.None())?;
    let entries = pyo3::types::PyDict::new(py);
    package.setattr("_zip_entries", &entries)?;
    let directories = pyo3::types::PySet::empty(py)?;
    package.setattr("directories", &directories)?;
    let count: usize = limits.getattr("files")?.extract()?;
    let mut index = PackageIndex::new(
        root.clone(),
        count,
        limits.getattr("file_bytes")?.extract()?,
        limits.getattr("total_bytes")?.extract()?,
        limits.getattr("ratio")?.extract()?,
        !root.is_dir(),
    )?;
    let load = (|| -> PyResult<()> {
        if root.is_dir() {
            let mut tree = crate::paths::FileTree::new(
                root.clone(),
                Some(cancel.clone().unbind()),
                Some(directories.clone().into_any().unbind()),
                count.saturating_mul(2),
            )?;
            while let Some((name, file)) = tree.__next__(py)? {
                check_cancel(Some(cancel))?;
                let key = index.add(&name, std::fs::metadata(&file)?.len())?;
                entries.set_item(key, file)?;
            }
            reject_links(&root)?;
            for directory in directories.iter() {
                check_cancel(Some(cancel))?;
                reject_links(&root.join(directory.extract::<String>()?))?;
            }
        } else {
            let suffix = root
                .extension()
                .and_then(|v| v.to_str())
                .map(folded)
                .unwrap_or_default();
            let zipfile = py.import("zipfile")?;
            if matches!(suffix.as_str(), "zip" | "fomod")
                && zipfile.call_method1("is_zipfile", (&path,))?.is_truthy()?
            {
                let zip = zipfile.getattr("ZipFile")?.call1((&path,))?;
                package.setattr("_zip", &zip)?;
                let infos = zip.call_method0("infolist")?;
                for info in infos.try_iter()? {
                    check_cancel(Some(cancel))?;
                    let info = info?;
                    let name: String = info.getattr("filename")?.extract()?;
                    let size: u64 = info.getattr("file_size")?.extract()?;
                    if index.zip_entry(
                        &name,
                        size,
                        info.getattr("compress_size")?.extract()?,
                        info.getattr("external_attr")?.extract()?,
                        info.getattr("flag_bits")?.extract()?,
                        info.call_method0("is_dir")?.is_truthy()?,
                        infos.len()?,
                    )? {
                        entries.set_item(index.add(&name, size)?, info)?;
                    }
                }
            } else if matches!(suffix.as_str(), "7z" | "rar" | "fomod") {
                let seven = py
                    .import("creation_lib.mod_manager.sevenzip")?
                    .getattr("SevenZip")?
                    .call1((&path, limits, cancel))?;
                package.setattr("_seven", &seven)?;
                for entry in seven.getattr("entries")?.try_iter()? {
                    check_cancel(Some(cancel))?;
                    let entry = entry?;
                    let name: String = entry.get_item(0)?.extract()?;
                    let key = index.add(&name, entry.get_item(1)?.extract()?)?;
                    entries.set_item(key, entry.get_item(2)?)?;
                }
            } else {
                return Err(invalid(
                    "Choose an unpacked mod folder, ZIP, 7z, RAR, or FOMOD archive.",
                ));
            }
        }
        index.finish(inspect_installer)?;
        package.setattr("files", &index.files)?;
        package.setattr("sizes", &index.sizes)?;
        package.setattr("_signatures", &index.signatures)?;
        package.setattr("config_path", &index.config_path)?;
        package.setattr("prefix", &index.prefix)?;
        Ok(())
    })();
    if let Err(error) = load {
        package_close(package)?;
        return Err(error);
    }
    package.setattr("_index", Py::new(py, index)?)?;
    Ok(())
}
#[pyclass]
struct PackageReader {
    package: Py<PyAny>,
    source: String,
    reader: Py<PyAny>,
}
#[pymethods]
impl PackageReader {
    fn __enter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(self.reader.bind(py).call_method0("__enter__")?.unbind())
    }
    fn __exit__(
        &self,
        py: Python<'_>,
        kind: &Bound<'_, PyAny>,
        error: &Bound<'_, PyAny>,
        trace: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        self.reader
            .bind(py)
            .call_method1("__exit__", (kind, error, trace))?;
        if kind.is_none() {
            self.package
                .bind(py)
                .getattr("_index")?
                .call_method1("source", (&self.source, true))?;
        }
        Ok(())
    }
}
#[pyfunction]
fn package_open(package: &Bound<'_, PyAny>, source: &str) -> PyResult<PackageReader> {
    let key = package
        .getattr("_index")?
        .call_method1("source", (source, false))?;
    let zip = package.getattr("_zip")?;
    let seven = package.getattr("_seven")?;
    let reader = if zip.is_truthy()? {
        zip.call_method1("open", (package.getattr("_zip_entries")?.get_item(&key)?,))?
    } else if seven.is_truthy()? {
        seven.call_method1(
            "open",
            (
                package.getattr("files")?.get_item(&key)?,
                package.getattr("_cancel")?,
            ),
        )?
    } else {
        package
            .getattr("path")?
            .call_method1("joinpath", (package.getattr("files")?.get_item(&key)?,))?
            .call_method1("open", ("rb",))?
    };
    Ok(PackageReader {
        package: package.clone().unbind(),
        source: source.into(),
        reader: reader.unbind(),
    })
}
#[pyfunction]
fn package_close(package: &Bound<'_, PyAny>) -> PyResult<()> {
    let zip = package.getattr("_zip")?;
    if zip.is_truthy()? {
        zip.call_method0("close")?;
        package.setattr("_zip", package.py().None())?;
    }
    Ok(())
}
#[pyfunction]
fn package_read_config(package: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    crate::python_io::with_context(
        &package.call_method1("open", (package.getattr("config_path")?,))?,
        |stream| {
            Ok(stream
                .call_method1("read", (2 * 1024 * 1024 + 1,))?
                .unbind())
        },
    )
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(package_initialize, m)?)?;
    m.add_function(wrap_pyfunction!(package_open, m)?)?;
    m.add_function(wrap_pyfunction!(package_close, m)?)?;
    m.add_function(wrap_pyfunction!(package_read_config, m)?)?;
    m.add_class::<PackageIndex>()?;
    m.add_function(wrap_pyfunction!(installer_metadata, m)?)?;
    m.add_function(wrap_pyfunction!(package_signature, m)?)?;
    Ok(())
}
