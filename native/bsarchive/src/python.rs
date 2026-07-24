use crate::{
    FileFormat, Reader as _, fo4, guess_format,
    mod_pack::{self, PackArchivePlan, PackModConfig, PackProgress},
    pack::{self, PackEntrySpec, PackFilters},
    tes4,
};
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyAny, PyBytes, PyDict, PyList},
};
use rayon::prelude::*;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

type ProgressCallback = Arc<Mutex<Py<PyAny>>>;

struct ArchiveInfo {
    format: &'static str,
    version: u32,
    file_count: usize,
    compressed: bool,
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(list_archive, m)?)?;
    m.add_function(wrap_pyfunction!(extract_archive, m)?)?;
    m.add_function(wrap_pyfunction!(extract_one, m)?)?;
    m.add_function(wrap_pyfunction!(archive_info, m)?)?;
    m.add_function(wrap_pyfunction!(pack_archive, m)?)?;
    m.add_function(wrap_pyfunction!(pack_archive_entries, m)?)?;
    m.add_function(wrap_pyfunction!(pack_archive_plans, m)?)?;
    m.add_function(wrap_pyfunction!(pack_mod_archives, m)?)?;
    m.add_function(wrap_pyfunction!(plan_archives, m)?)?;
    m.add_class::<FsIndex>()?;
    Ok(())
}

#[pymodule]
fn bsarchive_native(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_module(m)
}

/// Fast case-insensitive loose-file index built via a parallel directory walk.
///
/// The build runs with the GIL released so a large texture tree never stalls
/// the calling (load) thread. Lookups are pure in-memory map reads.
#[pyclass(name = "FsIndex")]
pub(crate) struct FsIndex {
    inner: crate::fs_index::FsIndex,
}

#[pymethods]
impl FsIndex {
    #[new]
    fn new(py: Python<'_>, root: &str) -> Self {
        let root = PathBuf::from(root);
        let inner = py.detach(move || crate::fs_index::FsIndex::build(&root));
        Self { inner }
    }

    fn resolve(&self, rel_path: &str) -> Option<String> {
        self.inner.resolve(rel_path)
    }

    fn contains(&self, rel_path: &str) -> bool {
        self.inner.contains(rel_path)
    }

    #[getter]
    fn file_count(&self) -> usize {
        self.inner.len()
    }

    #[pyo3(signature = (prefix="", suffix=""))]
    fn list_files(&self, prefix: &str, suffix: &str) -> Vec<String> {
        self.inner.list(prefix, suffix)
    }
}

enum OpenArchive {
    Tes4(tes4::Archive<'static>, tes4::ArchiveOptions),
    Fo4(fo4::Archive<'static>, fo4::ArchiveOptions),
}

pub struct ArchiveReader {
    inner: OpenArchive,
}

impl ArchiveReader {
    pub fn open(path: &Path) -> Result<Self, String> {
        Ok(Self {
            inner: open_archive(path)?,
        })
    }

    pub fn visit_files(
        &self,
        file_paths: &[String],
        mut visitor: impl FnMut(&str, &[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        for file_path in file_paths {
            let bytes = self.inner.extract_file(file_path)?;
            visitor(file_path, &bytes)?;
        }
        Ok(())
    }

    pub fn list_files(&self) -> Vec<String> {
        self.inner.list_files()
    }

    pub fn read_file(&self, file_path: &str) -> Result<Vec<u8>, String> {
        self.inner.extract_file(file_path)
    }
}

impl OpenArchive {
    fn file_count(&self) -> usize {
        match self {
            Self::Tes4(archive, _) => archive.values().map(|dir| dir.len()).sum(),
            Self::Fo4(archive, _) => archive.len(),
        }
    }

    fn format_name(&self) -> &'static str {
        match self {
            Self::Tes4(_, _) => "tes4",
            Self::Fo4(_, options) => match options.format() {
                fo4::Format::GNRL => "fo4_gnrl",
                fo4::Format::DX10 => "fo4_dx10",
                fo4::Format::GNMF => "fo4_gnmf",
            },
        }
    }

    fn version(&self) -> u32 {
        match self {
            Self::Tes4(_, options) => options.version() as u32,
            Self::Fo4(_, options) => options.version() as u32,
        }
    }

    fn compressed(&self) -> bool {
        match self {
            Self::Tes4(_, options) => options.flags().compressed(),
            Self::Fo4(archive, _) => archive
                .values()
                .flat_map(|file| file.iter())
                .any(fo4::Chunk::is_compressed),
        }
    }

    fn list_files(&self) -> Vec<String> {
        match self {
            Self::Tes4(archive, _) => archive
                .iter()
                .flat_map(|(dir_key, directory)| {
                    directory.iter().map(move |(file_key, _)| {
                        join_tes4_path(dir_key.name().to_string(), file_key.name().to_string())
                    })
                })
                .collect(),
            Self::Fo4(archive, _) => archive
                .keys()
                .map(|key| normalize_lookup(&key.name().to_string()))
                .collect(),
        }
    }

    fn extract_file(&self, requested_path: &str) -> Result<Vec<u8>, String> {
        let requested = normalize_lookup(requested_path);
        match self {
            Self::Tes4(archive, options) => {
                let write_options: tes4::FileCompressionOptions = (*options).into();
                for (dir_key, directory) in archive {
                    for (file_key, file) in directory {
                        let rel =
                            join_tes4_path(dir_key.name().to_string(), file_key.name().to_string());
                        if normalize_lookup(&rel) == requested {
                            let mut bytes = Vec::new();
                            file.write(&mut bytes, &write_options)
                                .map_err(|err| err.to_string())?;
                            return Ok(bytes);
                        }
                    }
                }
                Err(format!("file not found in archive: {requested_path}"))
            }
            Self::Fo4(archive, options) => {
                let write_options: fo4::FileWriteOptions = (*options).into();
                let key: fo4::ArchiveKey = requested.as_bytes().into();
                if let Some(file) = archive.get(&key) {
                    let mut bytes = Vec::new();
                    file.write(&mut bytes, &write_options)
                        .map_err(|err| err.to_string())?;
                    return Ok(bytes);
                }
                for (key, file) in archive {
                    if normalize_lookup(&key.name().to_string()) == requested {
                        let mut bytes = Vec::new();
                        file.write(&mut bytes, &write_options)
                            .map_err(|err| err.to_string())?;
                        return Ok(bytes);
                    }
                }
                Err(format!("file not found in archive: {requested_path}"))
            }
        }
    }
}

fn open_archive(path: &Path) -> Result<OpenArchive, String> {
    let mut file = fs::File::open(path).map_err(|err| err.to_string())?;
    let format = guess_format(&mut file).ok_or_else(|| "unsupported archive format".to_string())?;
    match format {
        FileFormat::TES4 => {
            let (archive, options) = tes4::Archive::read(path).map_err(|err| err.to_string())?;
            Ok(OpenArchive::Tes4(archive, options))
        }
        FileFormat::FO4 => {
            let (archive, options) = fo4::Archive::read(path).map_err(|err| err.to_string())?;
            Ok(OpenArchive::Fo4(archive, options))
        }
    }
}

fn write_extracted_file(output_dir: &Path, rel: &str, bytes: &[u8]) -> Result<(), String> {
    let out_path = output_dir.join(rel.replace('/', "\\"));
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    fs::write(&out_path, bytes).map_err(|err| err.to_string())
}

fn report_extract_progress(
    progress: Option<&ProgressCallback>,
    archive_name: &str,
    completed: usize,
    total: usize,
    rel_path: &str,
) -> Result<(), String> {
    let Some(progress) = progress else {
        return Ok(());
    };
    if completed != total && completed != 1 && completed % 1000 != 0 {
        return Ok(());
    }
    Python::attach(|py| -> Result<(), String> {
        let event = PyDict::new(py);
        event
            .set_item("archive", archive_name)
            .map_err(|err| err.to_string())?;
        event
            .set_item("completed", completed)
            .map_err(|err| err.to_string())?;
        event
            .set_item("total", total)
            .map_err(|err| err.to_string())?;
        event
            .set_item("path", rel_path)
            .map_err(|err| err.to_string())?;
        let keep_going = progress
            .lock()
            .map_err(|err| err.to_string())?
            .call1(py, (event,))
            .and_then(|result| result.extract::<bool>(py).or(Ok(true)))
            .map_err(|err| err.to_string())?;
        if keep_going {
            Ok(())
        } else {
            Err("extraction cancelled".to_string())
        }
    })
}

fn extract_tes4_archive(
    archive: &tes4::Archive<'static>,
    options: &tes4::ArchiveOptions,
    archive_name: &str,
    output_dir: &Path,
    workers: usize,
    progress: Option<&ProgressCallback>,
) -> Result<usize, String> {
    let write_options: tes4::FileCompressionOptions = (*options).into();
    let entries: Vec<(String, tes4::File<'static>)> = archive
        .iter()
        .flat_map(|(dir_key, directory)| {
            directory.iter().map(move |(file_key, file)| {
                (
                    join_tes4_path(dir_key.name().to_string(), file_key.name().to_string()),
                    file.clone(),
                )
            })
        })
        .collect();
    let total = entries.len();
    let completed = AtomicUsize::new(0);

    let extract_entry = |(rel, file): &(String, tes4::File<'static>)| -> Result<(), String> {
        let mut bytes = Vec::new();
        file.write(&mut bytes, &write_options)
            .map_err(|err| err.to_string())?;
        write_extracted_file(output_dir, rel, &bytes)?;
        let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
        report_extract_progress(progress, archive_name, done, total, rel)
    };

    if workers > 1 && total > 1 {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .map_err(|err| format!("rayon pool error: {err}"))?;
        pool.install(|| entries.par_iter().try_for_each(extract_entry))?;
    } else {
        for entry in &entries {
            extract_entry(entry)?;
        }
    }
    Ok(total)
}

fn extract_fo4_archive(
    archive: &fo4::Archive<'static>,
    options: &fo4::ArchiveOptions,
    archive_name: &str,
    output_dir: &Path,
    workers: usize,
    progress: Option<&ProgressCallback>,
) -> Result<usize, String> {
    let write_options: fo4::FileWriteOptions = (*options).into();
    let entries: Vec<(String, fo4::File<'static>)> = archive
        .iter()
        .map(|(key, file)| (normalize_lookup(&key.name().to_string()), file.clone()))
        .collect();
    let total = entries.len();
    let completed = AtomicUsize::new(0);

    let extract_entry = |(rel, file): &(String, fo4::File<'static>)| -> Result<(), String> {
        let mut bytes = Vec::new();
        file.write(&mut bytes, &write_options)
            .map_err(|err| err.to_string())?;
        write_extracted_file(output_dir, rel, &bytes)?;
        let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
        report_extract_progress(progress, archive_name, done, total, rel)
    };

    if workers > 1 && total > 1 {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .map_err(|err| format!("rayon pool error: {err}"))?;
        pool.install(|| entries.par_iter().try_for_each(extract_entry))?;
    } else {
        for entry in &entries {
            extract_entry(entry)?;
        }
    }
    Ok(total)
}

fn normalize_lookup(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .to_ascii_lowercase()
}

fn join_tes4_path(dir: String, file: String) -> String {
    let dir = normalize_lookup(&dir);
    let file = normalize_lookup(&file);
    if dir.is_empty() || dir == "." {
        file
    } else {
        format!("{dir}/{file}")
    }
}

fn list_archive_impl(path: &Path) -> Result<Vec<String>, String> {
    let archive = open_archive(path)?;
    Ok(archive.list_files())
}

pub fn list_archive_files(path: &Path) -> Result<Vec<String>, String> {
    list_archive_impl(path)
}

fn extract_archive_impl(
    archive: &Path,
    output_dir: &Path,
    format: Option<&str>,
    workers: usize,
    progress: Option<&ProgressCallback>,
) -> Result<usize, String> {
    let _ = format;
    let opened = open_archive(archive)?;
    fs::create_dir_all(output_dir).map_err(|err| err.to_string())?;
    let workers = workers.max(1);
    let archive_name = archive
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("archive");
    match opened {
        OpenArchive::Tes4(archive, options) => extract_tes4_archive(
            &archive,
            &options,
            archive_name,
            output_dir,
            workers,
            progress,
        ),
        OpenArchive::Fo4(archive, options) => extract_fo4_archive(
            &archive,
            &options,
            archive_name,
            output_dir,
            workers,
            progress,
        ),
    }
}

pub fn extract_one_impl(archive: &Path, file_path: &str) -> Result<Vec<u8>, String> {
    let archive = open_archive(archive)?;
    archive.extract_file(file_path)
}

/// Open an archive once and visit the requested members in order.
///
/// This is the non-Python batch seam used by conversion's sparse target-asset
/// cache. Keeping the parsed archive alive across the batch avoids reopening a
/// multi-gigabyte BA2 for every on-demand member.
pub fn visit_archive_files(
    archive_path: &Path,
    file_paths: &[String],
    mut visitor: impl FnMut(&str, &[u8]) -> Result<(), String>,
) -> Result<(), String> {
    ArchiveReader::open(archive_path)?.visit_files(file_paths, |path, bytes| visitor(path, bytes))
}

fn archive_info_impl(path: &Path) -> Result<ArchiveInfo, String> {
    let archive = open_archive(path)?;
    Ok(ArchiveInfo {
        format: archive.format_name(),
        version: archive.version(),
        file_count: archive.file_count(),
        compressed: archive.compressed(),
    })
}

fn required_config_item<'py>(
    config: &Bound<'py, PyDict>,
    key: &str,
) -> PyResult<Bound<'py, PyAny>> {
    config
        .get_item(key)?
        .ok_or_else(|| PyValueError::new_err(format!("pack_mod_archives config missing {key}")))
}

fn optional_config_string(config: &Bound<'_, PyDict>, key: &str) -> PyResult<Option<String>> {
    let Some(value) = config.get_item(key)? else {
        return Ok(None);
    };
    if value.is_none() {
        return Ok(None);
    }
    Ok(Some(value.extract::<String>()?))
}

fn optional_config_bool(config: &Bound<'_, PyDict>, key: &str, default: bool) -> PyResult<bool> {
    let Some(value) = config.get_item(key)? else {
        return Ok(default);
    };
    if value.is_none() {
        return Ok(default);
    }
    value.extract::<bool>()
}

fn parse_pack_mod_config(config: &Bound<'_, PyDict>) -> PyResult<PackModConfig> {
    let manifest_path = optional_config_string(config, "manifest_path")?.map(PathBuf::from);
    Ok(PackModConfig {
        mod_name: required_config_item(config, "mod_name")?.extract::<String>()?,
        mod_dir: PathBuf::from(required_config_item(config, "mod_dir")?.extract::<String>()?),
        data_dir: PathBuf::from(required_config_item(config, "data_dir")?.extract::<String>()?),
        strings_dir: PathBuf::from(
            required_config_item(config, "strings_dir")?.extract::<String>()?,
        ),
        game: required_config_item(config, "game")?.extract::<String>()?,
        archive_ext: required_config_item(config, "archive_ext")?.extract::<String>()?,
        archive_cap: required_config_item(config, "archive_max_bytes")?.extract::<u64>()?,
        expanded_archives: required_config_item(config, "expanded_archives")?.extract::<bool>()?,
        pc: required_config_item(config, "pc")?.extract::<bool>()?,
        xbox: required_config_item(config, "xbox")?.extract::<bool>()?,
        archive_workers: required_config_item(config, "archive_workers")?.extract::<usize>()?,
        manifest_path,
        dry_run: optional_config_bool(config, "dry_run", false)?,
    })
}

fn report_pack_progress(
    progress: Option<&ProgressCallback>,
    event: &PackProgress,
) -> Result<(), String> {
    let Some(progress) = progress else {
        return Ok(());
    };
    Python::attach(|py| -> Result<(), String> {
        let payload = PyDict::new(py);
        payload
            .set_item("phase", event.phase)
            .map_err(|err| err.to_string())?;
        payload
            .set_item("platform", event.platform.as_str())
            .map_err(|err| err.to_string())?;
        payload
            .set_item("message", event.message.as_str())
            .map_err(|err| err.to_string())?;
        payload
            .set_item("completed", event.completed)
            .map_err(|err| err.to_string())?;
        payload
            .set_item("total", event.total)
            .map_err(|err| err.to_string())?;
        let keep_going = progress
            .lock()
            .map_err(|err| err.to_string())?
            .call1(py, (payload,))
            .and_then(|result| result.extract::<bool>(py).or(Ok(true)))
            .map_err(|err| err.to_string())?;
        if keep_going {
            Ok(())
        } else {
            Err("packing cancelled".to_string())
        }
    })
}

fn pack_archive_impl(
    source_dir: &Path,
    output_path: &Path,
    archive_type: &str,
    compress: bool,
    compression_level: u32,
    share_data: bool,
    manifest_path: Option<&Path>,
    jobs: Option<usize>,
    include_prefixes: Vec<String>,
    exclude_prefixes: Vec<String>,
) -> Result<usize, String> {
    let filters = PackFilters::new(include_prefixes, exclude_prefixes);
    pack::pack_archive(
        source_dir,
        output_path,
        archive_type,
        compress,
        compression_level,
        share_data,
        manifest_path,
        jobs,
        filters,
    )
    .map_err(|err| err.to_string())
}

fn pack_archive_entries_impl(
    entries: Vec<(String, String)>,
    output_path: &Path,
    archive_type: &str,
    compress: bool,
    compression_level: u32,
    share_data: bool,
    manifest_path: Option<&Path>,
    jobs: Option<usize>,
) -> Result<usize, String> {
    let entries: Vec<PackEntrySpec> = entries
        .into_iter()
        .map(|(source_path, archive_path)| PackEntrySpec {
            source_path: PathBuf::from(source_path),
            archive_path,
            source_size: None,
        })
        .collect();
    pack::pack_archive_entries(
        &entries,
        output_path,
        archive_type,
        compress,
        compression_level,
        share_data,
        manifest_path,
        jobs,
    )
    .map_err(|err| err.to_string())
}

#[pyfunction]
pub(crate) fn list_archive(py: Python<'_>, path: &str) -> PyResult<Vec<String>> {
    let path = PathBuf::from(path);
    py.detach(|| list_archive_impl(&path))
        .map_err(PyRuntimeError::new_err)
}

#[pyfunction(signature = (archive, output_dir, format=None, workers=0, progress=None))]
pub(crate) fn extract_archive(
    py: Python<'_>,
    archive: &str,
    output_dir: &str,
    format: Option<&str>,
    workers: usize,
    progress: Option<Py<PyAny>>,
) -> PyResult<usize> {
    let archive = PathBuf::from(archive);
    let output_dir = PathBuf::from(output_dir);
    let format = format.map(str::to_owned);
    let progress = progress.map(|callback| Arc::new(Mutex::new(callback)));
    py.detach(|| {
        extract_archive_impl(
            &archive,
            &output_dir,
            format.as_deref(),
            workers,
            progress.as_ref(),
        )
    })
    .map_err(PyRuntimeError::new_err)
}

#[pyfunction]
pub(crate) fn extract_one(py: Python<'_>, archive: &str, file_path: &str) -> PyResult<Py<PyAny>> {
    let archive = PathBuf::from(archive);
    let file_path = file_path.to_owned();
    let bytes = py
        .detach(|| extract_one_impl(&archive, &file_path))
        .map_err(PyRuntimeError::new_err)?;
    Ok(PyBytes::new(py, &bytes).into_any().unbind())
}

#[pyfunction]
pub(crate) fn archive_info(py: Python<'_>, path: &str) -> PyResult<Py<PyAny>> {
    let path = PathBuf::from(path);
    let archive = py
        .detach(|| archive_info_impl(&path))
        .map_err(PyRuntimeError::new_err)?;
    let info = PyDict::new(py);
    info.set_item("format", archive.format)?;
    info.set_item("version", archive.version)?;
    info.set_item("file_count", archive.file_count)?;
    info.set_item("compressed", archive.compressed)?;
    Ok(info.into_any().unbind())
}

#[pyfunction(signature = (source_dir, output_path, archive_type, compress=true, compression_level=None, share_data=false, manifest_path=None, jobs=0, include_prefixes=None, exclude_prefixes=None))]
pub(crate) fn pack_archive(
    py: Python<'_>,
    source_dir: &str,
    output_path: &str,
    archive_type: &str,
    compress: bool,
    compression_level: Option<u32>,
    share_data: bool,
    manifest_path: Option<&str>,
    jobs: usize,
    include_prefixes: Option<Vec<String>>,
    exclude_prefixes: Option<Vec<String>>,
) -> PyResult<usize> {
    if archive_type.trim().is_empty() {
        return Err(PyValueError::new_err("archive_type is required"));
    }
    let source_dir = PathBuf::from(source_dir);
    let output_path = PathBuf::from(output_path);
    let archive_type = archive_type.to_owned();
    let manifest_path = manifest_path.map(PathBuf::from);
    let jobs = if jobs == 0 { None } else { Some(jobs) };
    let include_prefixes = include_prefixes.unwrap_or_default();
    let exclude_prefixes = exclude_prefixes.unwrap_or_default();
    let compression_level =
        compression_level.unwrap_or_else(|| crate::pack::archive_type_default_level(&archive_type));
    py.detach(|| {
        pack_archive_impl(
            &source_dir,
            &output_path,
            &archive_type,
            compress,
            compression_level,
            share_data,
            manifest_path.as_deref(),
            jobs,
            include_prefixes,
            exclude_prefixes,
        )
    })
    .map_err(PyRuntimeError::new_err)
}

#[pyfunction(signature = (entries, output_path, archive_type, compress=true, compression_level=None, share_data=false, manifest_path=None, jobs=0))]
pub(crate) fn pack_archive_entries(
    py: Python<'_>,
    entries: Vec<(String, String)>,
    output_path: &str,
    archive_type: &str,
    compress: bool,
    compression_level: Option<u32>,
    share_data: bool,
    manifest_path: Option<&str>,
    jobs: usize,
) -> PyResult<usize> {
    if archive_type.trim().is_empty() {
        return Err(PyValueError::new_err("archive_type is required"));
    }
    let output_path = PathBuf::from(output_path);
    let archive_type = archive_type.to_owned();
    let manifest_path = manifest_path.map(PathBuf::from);
    let jobs = if jobs == 0 { None } else { Some(jobs) };
    let compression_level =
        compression_level.unwrap_or_else(|| crate::pack::archive_type_default_level(&archive_type));
    py.detach(|| {
        pack_archive_entries_impl(
            entries,
            &output_path,
            &archive_type,
            compress,
            compression_level,
            share_data,
            manifest_path.as_deref(),
            jobs,
        )
    })
    .map_err(PyRuntimeError::new_err)
}

#[pyfunction(signature = (plans, total_workers=0, progress=None))]
pub(crate) fn pack_archive_plans(
    py: Python<'_>,
    plans: Vec<(String, String, bool, Vec<(String, String, u64)>)>,
    total_workers: usize,
    progress: Option<Py<PyAny>>,
) -> PyResult<usize> {
    let mut native_plans = Vec::with_capacity(plans.len());
    for (output_path, archive_type, texture_archive, entries) in plans {
        if archive_type.trim().is_empty() {
            return Err(PyValueError::new_err("archive_type is required"));
        }
        let output_path = PathBuf::from(output_path);
        let output_name = output_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| PyValueError::new_err("output_path must have a file name"))?
            .to_owned();
        let input_bytes = entries.iter().map(|entry| entry.2).sum();
        let entries = entries
            .into_iter()
            .map(|(source_path, archive_path, source_size)| PackEntrySpec {
                source_path: PathBuf::from(source_path),
                archive_path,
                source_size: Some(source_size),
            })
            .collect();
        native_plans.push(PackArchivePlan {
            output_path,
            output_name,
            archive_type,
            entries,
            input_bytes,
            texture_archive,
        });
    }
    let progress = progress.map(|callback| Arc::new(Mutex::new(callback)));
    py.detach(|| {
        mod_pack::pack_archive_plans(&native_plans, total_workers, |event| {
            report_pack_progress(progress.as_ref(), &event)
        })
        .map(|summaries| summaries.len())
    })
    .map_err(PyRuntimeError::new_err)
}

#[pyfunction(signature = (config, progress=None))]
pub(crate) fn pack_mod_archives(
    py: Python<'_>,
    config: &Bound<'_, PyDict>,
    progress: Option<Py<PyAny>>,
) -> PyResult<Py<PyAny>> {
    let config = parse_pack_mod_config(config)?;
    let progress = progress.map(|callback| Arc::new(Mutex::new(callback)));
    let result = py
        .detach(|| {
            mod_pack::pack_mod_archives(&config, |event| {
                report_pack_progress(progress.as_ref(), &event)
            })
        })
        .map_err(PyRuntimeError::new_err)?;

    let out = PyDict::new(py);
    out.set_item("inventory_elapsed_secs", result.inventory_elapsed_secs)?;
    out.set_item("planning_elapsed_secs", result.planning_elapsed_secs)?;
    let archives = PyList::empty(py);
    for archive in result.archives {
        let item = PyDict::new(py);
        item.set_item("platform", archive.platform)?;
        item.set_item("name", archive.name)?;
        item.set_item("file_count", archive.file_count)?;
        item.set_item("bytes", archive.bytes)?;
        item.set_item("elapsed_secs", archive.elapsed_secs)?;
        archives.append(item)?;
    }
    out.set_item("archives", archives)?;
    Ok(out.into_any().unbind())
}

#[pyfunction(signature = (mod_name, entries, archive_ext, platform_suffix="", archive_max_bytes=16 * 1024 * 1024 * 1024, game=None, expanded_archives=false))]
pub(crate) fn plan_archives(
    py: Python<'_>,
    mod_name: &str,
    entries: Vec<(String, String, u64)>,
    archive_ext: &str,
    platform_suffix: &str,
    archive_max_bytes: u64,
    game: Option<&str>,
    expanded_archives: bool,
) -> PyResult<Py<PyAny>> {
    let game = game.unwrap_or("").to_owned();
    let plans = py
        .detach(|| {
            mod_pack::plan_archives_public(
                mod_name,
                &entries,
                archive_ext,
                platform_suffix,
                archive_max_bytes,
                &game,
                expanded_archives,
            )
        })
        .map_err(PyValueError::new_err)?;

    let out = PyList::empty(py);
    for plan in plans {
        let item = PyDict::new(py);
        item.set_item("family", plan.family)?;
        item.set_item("label", plan.label)?;
        item.set_item("output_name", plan.output_name)?;
        item.set_item("texture_archive", plan.texture_archive)?;
        let entries_list = PyList::empty(py);
        for (rel, src, size) in plan.entries {
            entries_list.append((rel, src, size))?;
        }
        item.set_item("entries", entries_list)?;
        out.append(item)?;
    }
    Ok(out.into_any().unbind())
}
