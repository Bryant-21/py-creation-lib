use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::prelude::*;
use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
mod agent_ini;
mod agent_library;
mod agent_operations;
mod agent_runtime;
mod agent_workflow;
mod archive_index;
mod backup_prune;
mod browser_links;
mod categories;
mod cli_reader;
mod columns;
mod combined_data;
mod conflicts;
mod controller;
mod controller_categories;
mod controller_conflicts;
mod controller_downloads;
mod controller_filter;
mod controller_import;
mod controller_install;
mod controller_inventory;
mod controller_launch;
mod controller_lifecycle;
mod controller_nexus;
mod controller_overwrite;
mod controller_owned;
mod controller_profiles;
mod controller_saves;
mod controller_state;
mod controller_targets;
mod database;
mod download_install;
mod download_scheduler;
mod download_transfer;
mod downloads;
mod encoding;
mod executable_catalog;
mod executables;
mod extensions;
mod f4se_root_setup;
mod f4se_setup;
mod file_actions;
mod fomod;
mod history_prune;
mod installation;
mod installer_archives;
mod installer_context;
mod installer_preview;
mod installer_versions;
mod instances;
mod interaction;
mod launch_resolution;
mod library;
mod library_runtime;
mod metadata;
mod metadata_observer;
mod migration;
mod mo2_import;
mod mo2_inspection;
mod mo2_migration;
mod native_builder;
mod nexus;
mod nexus_auth;
mod nexus_batch;
mod nexus_categories;
mod nexus_links;
mod nexus_reviews;
mod nexus_state;
mod nexus_transport;
mod nxm_instances;
mod organization;
mod overwrite;
mod package;
mod paths;
mod payload;
mod pe_metadata;
mod pipe_peers;
mod plugin_files;
mod plugin_inventory;
mod plugin_priority;
mod plugin_state;
mod process_identity;
mod process_supervision;
mod profile_backup;
mod profile_recipes;
mod profile_state;
mod python_io;
mod read_process;
mod reference_overlay;
mod reference_removal;
mod reference_sources;
mod runtime;
mod runtime_helper;
mod runtime_plan;
mod runtime_windows;
mod save_backups;
mod save_inspection;
mod save_transfer;
mod saves;
mod service_adapters;
mod sevenzip;
mod source_metadata;
mod status;
mod target_profiles;
mod target_removal;
mod target_review;
mod target_settings;
mod target_sources;
mod transactions;
mod trees;
mod update_schedule;
pyo3::create_exception!(
    mod_manager_native,
    OperationCancelled,
    pyo3::exceptions::PyValueError
);

fn check_cancel(cancel: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
    if let Some(cancel) = cancel {
        if !cancel.is_none() && cancel.call_method0("is_set")?.is_truthy()? {
            return Err(OperationCancelled::new_err(
                "Operation cancelled; no changes committed.",
            ));
        }
    }
    Ok(())
}

fn json_record(path: &Path) -> PyResult<serde_json::Value> {
    reject_links(path)?;
    let file = File::open(path)?;
    if file.metadata()?.len() > 65536 {
        return Err(invalid(
            "Runtime status or active-session pointer exceeds its safety limit.",
        ));
    }
    use std::io::Read;
    let mut contents = Vec::new();
    file.take(65537).read_to_end(&mut contents)?;
    if contents.len() > 65536 {
        return Err(invalid("Runtime status exceeds its safety limit."));
    }
    let record: serde_json::Value = serde_json::from_slice(&contents)
        .map_err(|_| invalid("Invalid runtime session record."))?;
    if !record.is_object() {
        return Err(invalid("Runtime session record must be an object."));
    }
    Ok(record)
}

fn session_candidates(root: &Path, targets: bool, result: &mut BTreeSet<PathBuf>) -> PyResult<()> {
    reject_links(root)?;
    let profiles = root.join("runtime/profiles");
    reject_links(&profiles)?;
    if profiles.is_dir() {
        for profile in std::fs::read_dir(profiles)? {
            let pointer = profile?.path().join("active.json");
            reject_links(&pointer)?;
            if !pointer.is_file() {
                continue;
            }
            let record = json_record(&pointer)?;
            let name = record
                .get("session")
                .and_then(|v| v.as_str())
                .ok_or_else(|| invalid("Invalid runtime session identity."))?;
            let name = relative_path(name, false)?;
            if name.contains('/') {
                return Err(invalid("Invalid runtime session identity."));
            }
            result.insert(root.join("runtime/sessions").join(name));
        }
    }
    let sessions = root.join("runtime/sessions");
    reject_links(&sessions)?;
    if sessions.is_dir() {
        for session in std::fs::read_dir(sessions)? {
            let path = session?.path();
            reject_links(&path)?;
            if path.is_dir()
                && (path.join("status.json").is_file() || path.join("owner.json").is_file())
            {
                result.insert(path);
            }
        }
    }
    let target_root = root.join(".launch-targets");
    reject_links(&target_root)?;
    if target_root.exists() {
        if !targets {
            return Err(invalid("Nested launch target state is invalid."));
        }
        for (index, target) in std::fs::read_dir(target_root)?.enumerate() {
            if index >= 288 {
                return Err(invalid(
                    "Runtime target state exceeds its 32-active/256-retained target limit.",
                ));
            }
            let path = target?.path();
            reject_links(&path)?;
            if !path.is_dir() {
                continue;
            }
            let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
            if name.len() != 32
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            {
                return Err(invalid("Invalid owned transaction identifier."));
            }
            session_candidates(&path, false, result)?;
        }
    }
    Ok(())
}

#[pyfunction]
fn session_records(root: PathBuf) -> PyResult<Vec<(PathBuf, String)>> {
    let mut candidates = BTreeSet::new();
    session_candidates(&root, true, &mut candidates)?;
    let mut result = Vec::new();
    for path in candidates {
        reject_links(&path)?;
        let status_path = path.join("status.json");
        let mut status = if status_path.is_file() {
            json_record(&status_path)?
        } else {
            serde_json::json!({"state": "starting"})
        };
        if status["state"] == "starting" && path.join("owner.json").is_file() {
            let owner = json_record(&path.join("owner.json"))?;
            status
                .as_object_mut()
                .unwrap()
                .extend(owner.as_object().unwrap().clone());
        }
        if matches!(
            status["state"].as_str(),
            Some("starting" | "mapping" | "running" | "supervision-failed" | "identity-unverified")
        ) {
            result.push((path, status.to_string()));
        }
    }
    Ok(result)
}

#[pyfunction]
fn session_path(root: PathBuf, identifier: &str) -> PyResult<PathBuf> {
    let name = relative_path(identifier, false)?;
    if name.contains('/') {
        return Err(invalid("Session must be a session identifier."));
    }
    let mut candidates = BTreeSet::new();
    session_candidates(&root, true, &mut candidates)?;
    let matches: Vec<_> = candidates
        .into_iter()
        .filter(|p| p.file_name().and_then(|v| v.to_str()) == Some(&name))
        .collect();
    if matches.len() != 1 {
        return Err(invalid(
            "Session was not found or its identity is ambiguous.",
        ));
    }
    Ok(matches.into_iter().next().unwrap())
}

fn invalid(message: impl Into<String>) -> PyErr {
    PyValueError::new_err(message.into())
}

#[pyfunction]
#[pyo3(signature = (raw, empty=false))]
fn relative_path(raw: &str, empty: bool) -> PyResult<String> {
    let raw = raw.replace('\\', "/");
    if raw.is_empty() && empty {
        return Ok(raw);
    }
    let normalized = raw.trim_end_matches('/');
    if normalized.is_empty() || normalized.starts_with('/') {
        return Err(invalid(format!("Unsafe relative path: {raw:?}")));
    }
    for part in normalized.split('/') {
        let stem = part.split('.').next().unwrap_or("").to_ascii_lowercase();
        let device = matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
            || (stem.len() == 4
                && (stem.starts_with("com") || stem.starts_with("lpt"))
                && stem.as_bytes()[3].is_ascii_digit());
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with([' ', '.'])
            || part.chars().any(|c| c < ' ' || "<>:\"|?*".contains(c))
            || device
        {
            return Err(invalid(format!("Unsafe Windows path: {raw:?}")));
        }
    }
    Ok(normalized.to_owned())
}

#[pyfunction]
fn windows_arguments(text: &str) -> PyResult<Vec<String>> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if matches!(chars[index], ' ' | '\t') {
            index += 1;
            continue;
        }
        let mut token = String::new();
        let mut quoted = false;
        while index < chars.len() && (quoted || !matches!(chars[index], ' ' | '\t')) {
            if chars[index] == '\\' {
                let start = index;
                while index < chars.len() && chars[index] == '\\' {
                    index += 1;
                }
                let count = index - start;
                if index < chars.len() && chars[index] == '"' {
                    token.extend(std::iter::repeat_n('\\', count / 2));
                    if count % 2 != 0 {
                        token.push('"');
                        index += 1;
                        continue;
                    }
                } else {
                    token.extend(std::iter::repeat_n('\\', count));
                    continue;
                }
            }
            if chars[index] == '"' {
                if quoted && index + 1 < chars.len() && chars[index + 1] == '"' {
                    token.push('"');
                    index += 2;
                    continue;
                }
                quoted = !quoted;
            } else {
                token.push(chars[index]);
            }
            index += 1;
        }
        if quoted {
            return Err(invalid(
                "Arguments contain an unclosed quote. Check the imported command before running it.",
            ));
        }
        tokens.push(token);
    }
    Ok(tokens)
}

#[pyclass]
struct RuntimeLease {
    file: Option<File>,
}

#[pymethods]
impl RuntimeLease {
    #[new]
    fn new(path: PathBuf, shared: bool) -> PyResult<Self> {
        reject_links(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let result = if shared {
            file.try_lock_shared()
        } else {
            file.try_lock()
        };
        result.map_err(|_| invalid("Another operation owns this library or profile. Wait for it to finish before retrying."))?;
        Ok(Self { file: Some(file) })
    }

    fn close(&mut self) {
        self.file.take();
    }

    fn __enter__(&self) {}

    fn __exit__(
        &mut self,
        _kind: &Bound<'_, PyAny>,
        _error: &Bound<'_, PyAny>,
        _trace: &Bound<'_, PyAny>,
    ) {
        self.close();
    }
}

fn reject_links(path: &Path) -> PyResult<()> {
    for component in path.ancestors() {
        match std::fs::symlink_metadata(component) {
            Ok(info) => {
                #[cfg(windows)]
                let reparse = {
                    use std::os::windows::fs::MetadataExt;
                    info.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let reparse = false;
                if info.is_symlink() || reparse {
                    return Err(invalid(
                        "Links and Windows reparse points are not supported.",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[pyfunction]
fn snapshot_directory(source: PathBuf, destination: PathBuf) -> PyResult<()> {
    reject_links(&source)?;
    reject_links(&destination)?;
    if destination.starts_with(&source) {
        return Err(invalid("Snapshot destination must be outside its source."));
    }
    std::fs::create_dir_all(&destination)?;
    if !source.exists() {
        return Ok(());
    }
    let mut pending = vec![(source, destination)];
    let mut count = 0;
    while let Some((source, destination)) = pending.pop() {
        for entry in std::fs::read_dir(source)? {
            count += 1;
            if count > 200_000 {
                return Err(invalid("Session snapshot exceeds 200,000 entries."));
            }
            let entry = entry?;
            let source = entry.path();
            let destination = destination.join(entry.file_name());
            reject_links(&source)?;
            reject_links(&destination)?;
            let info = std::fs::symlink_metadata(&source)?;
            if info.is_dir() {
                std::fs::create_dir(&destination)?;
                pending.push((source, destination));
            } else if info.is_file() {
                let mut input = File::open(&source)?;
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(destination)?;
                std::io::copy(&mut input, &mut output)?;
                let after = input.metadata()?;
                if after.len() != info.len() || after.modified()? != info.modified()? {
                    return Err(invalid(
                        "Profile files changed while preparing the session.",
                    ));
                }
            } else {
                return Err(invalid("Session snapshots require regular files."));
            }
        }
    }
    Ok(())
}

#[pyclass]
struct PortLease {
    lease: Option<RuntimeLease>,
    #[pyo3(get)]
    port: u16,
}

#[pymethods]
impl PortLease {
    #[new]
    #[pyo3(signature = (directory, requested=None))]
    fn new(directory: PathBuf, requested: Option<u16>) -> PyResult<Self> {
        if requested == Some(0) {
            return Err(invalid("MCP port must be between 1 and 65535."));
        }
        reject_links(&directory)?;
        std::fs::create_dir_all(&directory)?;
        let ports = match requested {
            Some(port) => port..=port,
            None => 8922..=9021,
        };
        for port in ports {
            let path = directory.join(format!("{port}.lock"));
            let lease = match RuntimeLease::new(path, false) {
                Ok(lease) => lease,
                Err(error) if Python::attach(|py| error.is_instance_of::<PyValueError>(py)) => {
                    drop(error);
                    continue;
                }
                Err(error) => return Err(error),
            };
            // The lease serializes manager launches until the game exits. The
            // bind probe also excludes listeners started outside this manager.
            if TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok() {
                return Ok(Self {
                    lease: Some(lease),
                    port,
                });
            }
        }
        Err(PyOSError::new_err(
            "No available MCP port in the requested range.",
        ))
    }

    fn close(&mut self) {
        self.lease.take();
    }
}

#[pyclass]
struct McpSession {
    lease: Option<PortLease>,
    #[pyo3(get)]
    disabled: bool,
}

#[pyfunction]
#[pyo3(signature = (requested=None, disabled=false))]
fn validate_mcp_options(requested: Option<u16>, disabled: bool) -> PyResult<()> {
    if disabled && requested.is_some() {
        return Err(invalid("Cannot select an MCP port while MCP is disabled."));
    }
    if requested == Some(0) {
        return Err(invalid("MCP port must be between 1 and 65535."));
    }
    Ok(())
}

#[pyfunction]
fn launch_environment(
    steam_app_id: &str,
    disabled: bool,
) -> Option<std::collections::BTreeMap<String, String>> {
    if steam_app_id.is_empty() && !disabled {
        return None;
    }
    let mut environment: std::collections::BTreeMap<_, _> = std::env::vars().collect();
    if !steam_app_id.is_empty() {
        environment.insert("SteamAppId".into(), steam_app_id.into());
        environment.insert("SteamGameId".into(), steam_app_id.into());
    }
    if disabled {
        environment.insert("B21_MCP_DISABLED".into(), "1".into());
    }
    Some(environment)
}

#[pymethods]
impl McpSession {
    #[new]
    #[pyo3(signature = (directory, requested=None, disabled=false))]
    fn new(directory: PathBuf, requested: Option<u16>, disabled: bool) -> PyResult<Self> {
        validate_mcp_options(requested, disabled)?;
        Ok(Self {
            lease: if disabled {
                None
            } else {
                Some(PortLease::new(directory, requested)?)
            },
            disabled,
        })
    }

    #[getter]
    fn port(&self) -> Option<u16> {
        self.lease.as_ref().map(|lease| lease.port)
    }

    #[getter]
    fn endpoint(&self) -> Option<String> {
        self.port()
            .map(|port| format!("http://127.0.0.1:{port}/mcp"))
    }

    #[getter]
    fn environment(&self) -> std::collections::BTreeMap<String, String> {
        let mut values = std::collections::BTreeMap::from([(
            "B21_MCP_DISABLED".into(),
            if self.disabled { "1" } else { "0" }.into(),
        )]);
        if let Some(port) = self.port() {
            values.insert("B21_MCP_PORT".into(), port.to_string());
        }
        values
    }

    fn close(&mut self) {
        self.lease.take();
    }
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add(
        "OperationCancelled",
        m.py().get_type::<OperationCancelled>(),
    )?;
    database::register(m)?;
    profile_backup::register(m)?;
    profile_state::register(m)?;
    profile_recipes::register(m)?;
    target_settings::register(m)?;
    target_removal::register(m)?;
    target_review::register(m)?;
    target_profiles::register(m)?;
    target_sources::register(m)?;
    categories::register(m)?;
    metadata::register(m)?;
    library::register(m)?;
    instances::register(m)?;
    installer_versions::register(m)?;
    fomod::register(m)?;
    executables::register(m)?;
    mo2_import::register(m)?;
    mo2_inspection::register(m)?;
    organization::register(m)?;
    paths::register(m)?;
    payload::register(m)?;
    pe_metadata::register(m)?;
    transactions::register(m)?;
    nexus::register(m)?;
    nexus_batch::register(m)?;
    reference_sources::register(m)?;
    source_metadata::register(m)?;
    migration::register(m)?;
    history_prune::register(m)?;
    backup_prune::register(m)?;
    save_backups::register(m)?;
    save_transfer::register(m)?;
    saves::register(m)?;
    save_inspection::register(m)?;
    overwrite::register(m)?;
    plugin_state::register(m)?;
    plugin_inventory::register(m)?;
    plugin_priority::register(m)?;
    cli_reader::register(m)?;
    conflicts::register(m)?;
    controller::register(m)?;
    controller_conflicts::register(m)?;
    controller_filter::register(m)?;
    controller_install::register(m)?;
    controller_import::register(m)?;
    controller_launch::register(m)?;
    controller_owned::register(m)?;
    interaction::register(m)?;
    runtime_windows::register(m)?;
    nexus_transport::register(m)?;
    service_adapters::register(m)?;
    columns::register(m)?;
    controller_lifecycle::register(m)?;
    controller_nexus::register(m)?;
    controller_downloads::register(m)?;
    controller_categories::register(m)?;
    controller_saves::register(m)?;
    controller_overwrite::register(m)?;
    controller_inventory::register(m)?;
    controller_profiles::register(m)?;
    controller_state::register(m)?;
    controller_targets::register(m)?;
    library_runtime::register(m)?;
    agent_ini::register(m)?;
    agent_library::register(m)?;
    agent_runtime::register(m)?;
    browser_links::register(m)?;
    nxm_instances::register(m)?;
    pipe_peers::register(m)?;
    status::register(m)?;
    f4se_root_setup::register(m)?;
    f4se_setup::register(m)?;
    runtime_helper::register(m)?;
    runtime_plan::register(m)?;
    runtime::register(m)?;
    process_identity::register(m)?;
    process_supervision::register(m)?;
    read_process::register(m)?;
    native_builder::register(m)?;
    metadata_observer::register(m)?;
    reference_overlay::register(m)?;
    mo2_migration::register(m)?;
    download_install::register(m)?;
    sevenzip::register(m)?;
    extensions::register(m)?;
    installer_context::register(m)?;
    combined_data::register(m)?;
    archive_index::register(m)?;
    agent_operations::register(m)?;
    agent_workflow::register(m)?;
    installer_archives::register(m)?;
    installer_preview::register(m)?;
    installation::register(m)?;
    launch_resolution::register(m)?;
    executable_catalog::register(m)?;
    file_actions::register(m)?;
    trees::register(m)?;
    plugin_files::register(m)?;
    reference_removal::register(m)?;
    nexus_state::register(m)?;
    nexus_auth::register(m)?;
    nexus_categories::register(m)?;
    nexus_links::register(m)?;
    nexus_reviews::register(m)?;
    downloads::register(m)?;
    download_transfer::register(m)?;
    download_scheduler::register(m)?;
    package::register(m)?;
    m.add_function(wrap_pyfunction!(relative_path, m)?)?;
    m.add_function(wrap_pyfunction!(windows_arguments, m)?)?;
    m.add_function(wrap_pyfunction!(snapshot_directory, m)?)?;
    m.add_class::<RuntimeLease>()?;
    m.add_class::<PortLease>()?;
    m.add_class::<McpSession>()?;
    m.add_function(wrap_pyfunction!(validate_mcp_options, m)?)?;
    m.add_function(wrap_pyfunction!(launch_environment, m)?)?;
    m.add_function(wrap_pyfunction!(session_records, m)?)?;
    m.add_function(wrap_pyfunction!(session_path, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_quoting_and_unicode() {
        assert_eq!(
            windows_arguments(r#"-x "a b" "" α"#).unwrap(),
            vec!["-x", "a b", "", "α"]
        );
        assert_eq!(
            windows_arguments(r#""a""b" c\\d"#).unwrap(),
            vec!["a\"b", "c\\\\d"]
        );
    }

    #[test]
    fn path_normalization() {
        assert_eq!(
            relative_path("meshes\\thing.nif/", false).unwrap(),
            "meshes/thing.nif"
        );
    }

    #[test]
    fn readers_coexist_but_exclude_writer() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("library.lock");
        let a = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        let b = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        a.try_lock_shared().unwrap();
        b.try_lock_shared().unwrap();
        let c = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        assert!(c.try_lock().is_err());
        drop(a);
        drop(b);
        c.try_lock().unwrap();
    }
}
