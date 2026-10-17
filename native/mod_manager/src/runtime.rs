use crate::{
    check_cancel, encoding, history_prune::py_path, invalid, plugin_inventory::value,
    python_io::with_context, reject_links,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use serde_json::json;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
pub fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("creation_lib.mod_manager.runtime")
}
pub fn local<'py>(
    py: Python<'py>,
    path: &Bound<'py, PyAny>,
    label: &str,
    directory: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let kwargs = PyDict::new(py);
    kwargs.set_item("directory", directory)?;
    py.import("creation_lib.mod_manager.launcher")?.call_method(
        "local_path",
        (path, label),
        Some(&kwargs),
    )
}
#[pyfunction]
fn runtime_bundle(path: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = path.py();
    let raw = path.str()?.to_str()?.to_owned();
    let chosen = if raw.trim().is_empty() || !PathBuf::from(&raw).is_absolute() {
        py.import("creation_lib.mod_manager.runtime_setup")?
            .call_method1("resolve", (path,))?
    } else {
        path.clone()
    };
    let root = local(py, &chosen, "USVFS bundle", true)?;
    for (name, machine) in [
        ("usvfs_x64.dll", 0x8664),
        ("usvfs_x86.dll", 0x14c),
        ("usvfs_proxy_x64.exe", 0x8664),
        ("usvfs_proxy_x86.exe", 0x14c),
    ] {
        let candidate = local(
            py,
            &root.call_method1("joinpath", (name,))?,
            "USVFS bundle component",
            false,
        )?;
        if module(py)?
            .call_method1("pe_machine", (candidate,))?
            .extract::<u32>()?
            != machine
        {
            return Err(invalid(format!(
                "Wrong architecture for {name}; select a complete compatible USVFS bundle."
            )));
        }
    }
    Ok(root.unbind())
}
#[pyfunction]
fn runtime_bundle_resolve(value: &Bound<'_, PyAny>, app_root: PathBuf) -> PyResult<Py<PyAny>> {
    let py = value.py();
    let raw = value.str()?.to_str()?.trim().to_owned();
    let version = module(py)?
        .getattr("SUPPORTED_VERSION")?
        .extract::<String>()?;
    if raw.is_empty() {
        return Err(invalid(format!(
            "Choose a trusted USVFS {version} bundle directory in Runtime setup, or check the imported MO2 folder for a supported bundle."
        )));
    }
    let path = py_path(py, &PathBuf::from(&raw))?;
    let path = if !path.call_method0("is_absolute")?.is_truthy()? {
        if path.getattr("drive")?.is_truthy()? || raw.starts_with(['/', '\\']) {
            return Err(invalid(
                "Use a complete local bundle path, or a path relative to this application's directory.",
            ));
        }
        py_path(py, &app_root)?.call_method1("joinpath", (path,))?
    } else {
        path
    };
    Ok(local(py, &path, "USVFS bundle", true)?.unbind())
}
#[pyfunction]
fn runtime_supported_bundle(value: &Bound<'_, PyAny>, app_root: PathBuf) -> PyResult<Py<PyAny>> {
    let py = value.py();
    let root = runtime_bundle(runtime_bundle_resolve(value, app_root)?.bind(py))?;
    let version = module(py)?
        .getattr("SUPPORTED_VERSION")?
        .extract::<String>()?;
    let expected: Vec<u32> = version
        .split('.')
        .map(|v| {
            v.parse()
                .map_err(|_| invalid("Invalid supported runtime version."))
        })
        .collect::<PyResult<_>>()?;
    for name in [
        "usvfs_x64.dll",
        "usvfs_x86.dll",
        "usvfs_proxy_x64.exe",
        "usvfs_proxy_x86.exe",
    ] {
        let observed = py
            .import("creation_lib.mod_manager.pe_metadata")?
            .call_method1(
                "file_version",
                (root.bind(py).call_method1("joinpath", (name,))?,),
            )?;
        if observed.is_none() || observed.extract::<Vec<u32>>()? != expected {
            let found = if observed.is_truthy()? {
                observed
                    .extract::<Vec<u32>>()?
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(".")
            } else {
                "unknown (missing version metadata)".to_owned()
            };
            return Err(invalid(format!(
                "{name} reports {found}; this adapter supports {version}. Select a complete supported bundle. No candidate DLL was loaded."
            )));
        }
    }
    Ok(root)
}
#[pyfunction]
fn runtime_discover_bundle(
    configuration: &Bound<'_, PyAny>,
    app_root: PathBuf,
) -> PyResult<(Vec<String>, Vec<String>)> {
    let py = configuration.py();
    let config = value(configuration)?;
    let version = module(py)?
        .getattr("SUPPORTED_VERSION")?
        .extract::<String>()?;
    let candidates = [
        config["paths"]["instance"]
            .as_str()
            .unwrap_or("")
            .to_owned(),
        config["paths"]["base_directory"]
            .as_str()
            .unwrap_or("")
            .to_owned(),
        app_root
            .join("resource")
            .join("usvfs")
            .to_string_lossy()
            .into_owned(),
        app_root
            .join("resource")
            .join("usvfs")
            .join(version)
            .to_string_lossy()
            .into_owned(),
    ];
    let mut seen = std::collections::HashSet::new();
    let mut accepted = Vec::new();
    let mut issues = Vec::new();
    for raw in candidates {
        if raw.is_empty() || !seen.insert(crate::saves::fold(&raw)) {
            continue;
        }
        let result = (|| -> PyResult<Option<String>> {
            let root =
                runtime_bundle_resolve(raw.clone().into_pyobject(py)?.as_any(), app_root.clone())?;
            let path: PathBuf = root.extract(py)?;
            if !path.join("usvfs_x64.dll").exists() {
                return Ok(None);
            }
            let root = runtime_supported_bundle(root.bind(py), app_root.clone())?;
            Ok(Some(root.bind(py).str()?.to_str()?.to_owned()))
        })();
        match result {
            Ok(Some(root)) => accepted.push(root),
            Ok(None) => {}
            Err(e)
                if e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
                    || e.is_instance_of::<pyo3::exceptions::PyOSError>(py) =>
            {
                let message = e.value(py).str()?.to_str()?.to_owned();
                if !message.contains("missing or is not a directory") {
                    issues.push(format!("{raw}: {message}"));
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok((accepted, issues))
}
#[pyfunction]
fn runtime_captured_signature(
    profile: &Bound<'_, PyAny>,
    rows: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let mut content = Vec::new();
    for row in rows.try_iter()? {
        let row = row?;
        let mut fields = Vec::new();
        for field in ["id", "source", "priority", "enabled", "kind"] {
            fields.push(value(&row.getattr(field)?)?);
        }
        content.push(fields);
    }
    Ok(encoding::digest(
        &json!([value(&profile.getattr("files")?)?, content]),
        true,
        true,
    ))
}
#[pyfunction]
fn runtime_profile_signature(library: &Bound<'_, PyAny>, profile_id: &str) -> PyResult<String> {
    let snapshot = library.call_method1("profile_snapshot", (profile_id,))?;
    runtime_captured_signature(&snapshot.get_item(0)?, &snapshot.get_item(1)?)
}
#[pyfunction]
fn runtime_validate_f4se_configuration(
    game_root: PathBuf,
    library_root: PathBuf,
    profile_id: &str,
    files: &Bound<'_, PyAny>,
) -> PyResult<Option<PathBuf>> {
    let py = files.py();
    let overwrite = library_root
        .join("runtime")
        .join("profiles")
        .join(profile_id)
        .join("overwrite")
        .join("F4SE")
        .join("f4se.ini");
    reject_links(&overwrite)?;
    let mut configured = game_root.join("Data").join("F4SE").join("f4se.ini");
    for item in files.try_iter()? {
        let item = item?;
        if crate::saves::fold(&item.get_item(0)?.extract::<String>()?) == "f4se/f4se.ini" {
            configured = item.get_item(1)?.extract()?;
            break;
        }
    }
    if overwrite.is_file() {
        configured = overwrite;
    }
    reject_links(&configured)?;
    if !configured.is_file() {
        return Ok(None);
    }
    let configparser = py.import("configparser")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("interpolation", py.None())?;
    kwargs.set_item("strict", true)?;
    let parser = configparser
        .getattr("ConfigParser")?
        .call((), Some(&kwargs))?;
    let text = py
        .import("creation_lib.mod_manager.mo2")?
        .call_method1("read_text", (py_path(py, &configured)?,))?;
    if let Err(e) = parser.call_method1("read_string", (text,)) {
        if e.is_instance(py, &configparser.getattr("Error")?) {
            return Err(invalid(
                "Ambiguous or invalid effective F4SE INI; correct it before launching.",
            ));
        }
        return Err(e);
    }
    let sections: Vec<String> = parser.call_method0("sections")?.extract()?;
    let sections: Vec<_> = sections
        .into_iter()
        .filter(|s| crate::saves::fold(s) == "loader")
        .collect();
    if sections.len() > 1 {
        return Err(invalid(
            "Ambiguous Loader sections in the effective F4SE INI.",
        ));
    }
    if let Some(section) = sections.first() {
        let kwargs = PyDict::new(py);
        kwargs.set_item("fallback", "")?;
        let target = parser
            .call_method("get", (section, "RuntimeName"), Some(&kwargs))?
            .extract::<String>()?;
        let target = target.trim().trim_matches('"');
        if !target.is_empty() && crate::saves::fold(target) != "fallout4.exe" {
            return Err(invalid(
                "Effective F4SE RuntimeName selects another executable; this adapter supports the reviewed game-root Fallout4.exe only.",
            ));
        }
    }
    Ok(Some(configured))
}
#[pyfunction]
fn runtime_session_status(py: Python<'_>, session: PathBuf) -> PyResult<Py<PyAny>> {
    let path = session.join("status.json");
    reject_links(&path)?;
    let status = if !path.is_file() {
        json!({"state":"starting"})
    } else {
        if std::fs::metadata(&path)?.len() > 65536 {
            return Err(invalid("Runtime status exceeds its safety limit."));
        }
        encoding::parse(&std::fs::read_to_string(path)?)?
    };
    Ok(crate::plugin_inventory::object(py, &status)?.unbind())
}
#[pyfunction]
fn runtime_helper_command(
    request: &Bound<'_, PyAny>,
    python_root: PathBuf,
) -> PyResult<Vec<String>> {
    let py = request.py();
    let sys = py.import("sys")?;
    let frozen = sys.hasattr("frozen")? && sys.getattr("frozen")?.is_truthy()?;
    let executable = if frozen {
        sys.getattr("executable")?
    } else if sys.hasattr("_base_executable")? && sys.getattr("_base_executable")?.is_truthy()? {
        sys.getattr("_base_executable")?
    } else {
        sys.getattr("executable")?
    };
    let executable = local(
        py,
        &executable,
        if frozen {
            "Application runtime helper"
        } else {
            "Runtime helper interpreter"
        },
        false,
    )?;
    if module(py)?
        .call_method1("pe_machine", (&executable,))?
        .extract::<u32>()?
        != 0x8664
    {
        return Err(invalid(if frozen {
            "Application runtime helper must be x64."
        } else {
            "Runtime helper interpreter must be x64."
        }));
    }
    let executable = executable.str()?.to_str()?.to_owned();
    let request = request.str()?.to_str()?.to_owned();
    if frozen {
        return Ok(vec![
            executable,
            py.import("creation_lib.mod_manager.runtime_entry")?
                .getattr("FLAG")?
                .extract()?,
            request,
        ]);
    }
    let root = py
        .import("app.paths")?
        .call_method0("get_code_root")?
        .str()?
        .to_str()?
        .to_owned();
    let paths = PyList::new(py, [python_root.to_string_lossy().into_owned(), root])?
        .repr()?
        .to_str()?
        .to_owned();
    Ok(vec![
        executable,
        "-c".into(),
        format!(
            "import sys,runpy; sys.path[:0] = {paths}; runpy.run_module('creation_lib.mod_manager.runtime_helper', run_name='__main__')"
        ),
        request,
    ])
}
#[pyfunction]
fn runtime_authorize_helper(
    process: &Bound<'_, PyAny>,
    session: PathBuf,
    profile_id: &str,
) -> PyResult<Py<PyAny>> {
    let py = process.py();
    let identity = py
        .import("creation_lib.mod_manager.process_identity")?
        .call_method1(
            "from_handle",
            (process.getattr("pid")?, process.getattr("_handle")?),
        )?;
    let temporary = session.join("owner.tmp");
    let owner = session.join("owner.json");
    reject_links(&temporary)?;
    reject_links(&owner)?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("encoding", "utf-8")?;
    py_path(py, &temporary)?.call_method("write_text", (json!({"helper_pid":value(&process.getattr("pid")?)?,"helper_identity":value(&identity)?,"profile_id":profile_id}).to_string(),), Some(&kwargs))?;
    py_path(py, &temporary)?.call_method1("replace", (py_path(py, &owner)?,))?;
    Ok(identity.unbind())
}
#[pyfunction]
fn runtime_active_sessions(root: PathBuf, py: Python<'_>) -> PyResult<Py<PyAny>> {
    let identities = py.import("creation_lib.mod_manager.process_identity")?;
    let result = PyList::empty(py);
    for (path, record) in crate::session_records(root)? {
        let mut status = encoding::parse(&record)?;
        let liveness = identities
            .call_method1(
                "session_liveness",
                (crate::plugin_inventory::object(py, &status)?,),
            )?
            .extract::<String>()?;
        if liveness == "unverified" {
            status["state"] = json!("identity-unverified");
            status["error"] = json!(
                "Runtime creation identity cannot be verified; library changes remain locked. No process will be terminated by PID."
            );
        }
        if liveness != "exited" {
            result.append((
                py_path(py, &path)?,
                crate::plugin_inventory::object(py, &status)?,
            ))?;
        }
    }
    Ok(result.into_any().unbind())
}
#[pyfunction]
fn runtime_steam_running(py: Python<'_>) -> PyResult<bool> {
    Ok(cfg!(windows)
        && module(py)?
            .call_method0("process_names")?
            .contains("steam.exe")?)
}
#[pyfunction]
fn runtime_start(
    library: &Bound<'_, PyAny>,
    plan: &Bound<'_, PyAny>,
    accept_source_writes: bool,
    accept_minimal_ini: bool,
    accept_experimental: bool,
    cancel: &Bound<'_, PyAny>,
    spawn: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let runtime = module(py)?;
    crate::validate_mcp_options(
        plan.getattr("mcp_port")?.extract()?,
        plan.getattr("disable_mcp")?.is_truthy()?,
    )?;
    if plan.getattr("experimental")?.is_truthy()? && !accept_experimental {
        return Err(invalid(
            "Managed external F4SE staging is experimental. Acknowledge its unverified real-game launch route before starting.",
        ));
    }
    let project = plan.getattr("runtime")?.extract::<String>()? == "modbox-vfs-fo4-x64";
    if !project && !accept_source_writes {
        return Err(invalid(
            "Review and acknowledge USVFS's source-write behavior before a modded launch.",
        ));
    }
    if plan.getattr("minimal_ini")?.is_truthy()? && !accept_minimal_ini {
        return Err(invalid(
            "Acknowledge minimal local settings for missing profile INIs before launch.",
        ));
    }
    check_cancel(Some(cancel))?;
    let launch = plan.getattr("launch")?;
    let profile_id: String = launch.getattr("profile_id")?.extract()?;
    if plan.getattr("profile_signature")?.extract::<String>()?
        != runtime
            .call_method1("profile_signature", (library, &profile_id))?
            .extract::<String>()?
    {
        return Err(invalid(
            "Profile changed after runtime review; review it again.",
        ));
    }
    let record = plan.call_method0("record")?;
    py.import("creation_lib.mod_manager.runtime_helper")?
        .call_method1("validate_files", (&record,))?;
    if project {
        py.import("creation_lib.mod_manager.native_builder")?
            .call_method1(
                "validate",
                (
                    py_path(py, &plan.getattr("bundle_root")?.extract::<PathBuf>()?)?,
                    cancel,
                ),
            )?;
    } else {
        runtime.call_method1("bundle", (plan.getattr("bundle_root")?,))?;
    }
    if !runtime.call_method0("steam_running")?.is_truthy()? {
        return Err(invalid(
            "Start Steam and sign in before launching Fallout 4/F4SE. Steam bootstrap/restart outside this VFS is not supported; no process was started.",
        ));
    }
    let session_id: String = plan.getattr("session_id")?.extract()?;
    let session_id = if session_id.is_empty() {
        uuid::Uuid::new_v4().simple().to_string()
    } else {
        session_id
    };
    crate::transactions::transaction_key(&session_id)?;
    let session = library
        .getattr("root")?
        .extract::<PathBuf>()?
        .join("runtime")
        .join("sessions")
        .join(session_id);
    reject_links(&session)?;
    std::fs::create_dir_all(&session)?;
    let request = session.join("request.json");
    let path = py_path(py, &request)?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("encoding", "utf-8")?;
    with_context(
        &path.call_method("open", ("x",), Some(&kwargs))?,
        |stream| {
            py.import("json")?.call_method1("dump", (&record, stream))?;
            Ok(())
        },
    )?;
    check_cancel(Some(cancel))?;
    let subprocess = py.import("subprocess")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("cwd", session.to_string_lossy().as_ref())?;
    kwargs.set_item("shell", false)?;
    for key in ["stdin", "stdout", "stderr"] {
        kwargs.set_item(key, subprocess.getattr("DEVNULL")?)?;
    }
    kwargs.set_item(
        "creationflags",
        if cfg!(windows) { 0x08000000u32 } else { 0 },
    )?;
    let factory = if spawn.is_truthy()? {
        spawn.clone()
    } else {
        subprocess.getattr("Popen")?
    };
    let child = factory
        .call(
            (runtime.call_method1("helper_command", (&path,))?,),
            Some(&kwargs),
        )
        .map_err(|e| {
            if e.is_instance_of::<pyo3::exceptions::PyOSError>(py) {
                invalid(format!(
                    "Could not start the isolated filespace helper: {e}"
                ))
            } else {
                e
            }
        })?;
    let identities = py.import("creation_lib.mod_manager.process_identity")?;
    let mut helper_identity = None;
    let result = (|| -> PyResult<Py<PyAny>> {
        let identity = identities.call_method1(
            "from_handle",
            (child.getattr("pid")?, child.getattr("_handle")?),
        )?;
        helper_identity = Some(identity.clone());
        runtime.call_method1(
            "authorize_helper",
            (&child, py_path(py, &session)?, &profile_id),
        )?;
        let start = Instant::now();
        loop {
            let status = runtime.call_method1("session_status", (py_path(py, &session)?,))?;
            let state: String = status.get_item("state")?.extract()?;
            if matches!(state.as_str(), "running" | "completed") {
                if !status
                    .call_method1("get", ("helper_identity",))?
                    .eq(&identity)?
                {
                    return Err(invalid(
                        "Runtime acknowledgement does not match the spawned helper's creation identity.",
                    ));
                }
                return Ok((py_path(py, &session)?, child.clone(), status)
                    .into_pyobject(py)?
                    .into_any()
                    .unbind());
            }
            if state == "cancelled" {
                return Err(crate::OperationCancelled::new_err(
                    status
                        .call_method1(
                            "get",
                            ("error", "Launch cancelled before process creation."),
                        )?
                        .extract::<String>()?,
                ));
            }
            if matches!(state.as_str(), "failed" | "supervision-failed") {
                return Err(invalid(
                    status
                        .call_method1("get", ("error", "Native runtime failed before launch."))?
                        .extract::<String>()?,
                ));
            }
            if !child.call_method0("poll")?.is_none() {
                return Err(invalid(
                    "Filespace helper exited without confirming a hooked process. No direct-launch fallback was attempted.",
                ));
            }
            if start.elapsed() >= Duration::from_secs(180) {
                return Err(invalid(
                    "Native runtime preparation exceeded 180 seconds; the helper was stopped. Review the session before retrying.",
                ));
            }
            if !cancel.is_none() && cancel.call_method0("is_set")?.is_truthy()? {
                let marker = session.join("cancel");
                reject_links(&marker)?;
                py_path(py, &marker)?.call_method0("touch")?;
            }
            py.detach(|| std::thread::sleep(Duration::from_millis(50)));
        }
    })();
    if result.is_err() {
        let marker = session.join("cancel");
        let marked = (|| -> PyResult<()> {
            reject_links(&marker)?;
            py_path(py, &marker)?.call_method0("touch")?;
            Ok(())
        })();
        if child.call_method0("poll")?.is_none() {
            if let Some(identity) = helper_identity {
                identities.call_method1("terminate_owned", (&child, identity))?;
            }
        }
        child.call_method1("wait", (10,))?;
        marked?;
    }
    result
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(runtime_bundle, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_bundle_resolve, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_supported_bundle, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_discover_bundle, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_captured_signature, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_profile_signature, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_validate_f4se_configuration, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_session_status, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_helper_command, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_authorize_helper, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_active_sessions, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_steam_running, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_start, m)?)?;
    Ok(())
}
