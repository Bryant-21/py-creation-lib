use crate::{database::records, history_prune::py_path, invalid, python_io::with_database};
use pyo3::{
    prelude::*,
    types::{PyDict, PyInt, PyTuple},
};
use rusqlite::params;
use std::path::PathBuf;

#[pyfunction]
fn metadata_remember(
    cache: &Bound<'_, PyAny>,
    key: &Bound<'_, PyAny>,
    value: &Bound<'_, PyAny>,
    capacity: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = cache.py();
    if !capacity.is_instance_of::<PyInt>() || capacity.lt(1)? || capacity.gt(64)? {
        return Err(invalid("Metadata cache capacity must be between 1 and 64."));
    }
    let capacity = capacity.extract::<usize>()?;
    cache.call_method1("pop", (key, py.None()))?;
    cache.set_item(key, value)?;
    let mut removed = vec![];
    while cache.len()? > capacity {
        let oldest = cache
            .try_iter()?
            .next()
            .transpose()?
            .ok_or_else(|| invalid("Metadata cache changed while trimming."))?;
        cache.call_method1("pop", (&oldest,))?;
        removed.push(oldest);
    }
    Ok(PyTuple::new(py, removed)?.into_any().unbind())
}
#[pyfunction]
fn folder_reveal(
    library: &Bound<'_, PyAny>,
    profile: Option<&str>,
    mod_id: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let (source, label) = if let Some(mod_id) = mod_id {
        let row=with_database(&library.getattr("_db")?,|_,db| {
            Ok(records(&*db.connection()?,"SELECT mods.* FROM mods JOIN entries ON mods.id=entries.mod_id WHERE entries.profile_id=? AND mods.id=?",params![profile.unwrap_or(""),mod_id])?.into_iter().next())
        })?.ok_or_else(||invalid("Select an available mod folder in this local profile."))?;
        let kind = row["kind"].as_str().unwrap_or("");
        let source = row["source"].as_str().unwrap_or("");
        if !["owned", "external"].contains(&kind) {
            return Err(invalid(
                "Select an available mod folder in this local profile.",
            ));
        }
        if kind == "owned" {
            if crate::relative_path(mod_id, false)? != mod_id || mod_id.contains('/') {
                return Err(invalid("Owned mod folder identity is invalid."));
            }
            let expected = library
                .getattr("root")?
                .call_method1("joinpath", ("mods", mod_id))?;
            if !py_path(py, &PathBuf::from(source))?
                .call_method0("absolute")?
                .eq(expected)?
            {
                return Err(invalid(
                    "Owned mod folder metadata no longer matches its library location.",
                ));
            }
        }
        (source.to_string(), "Selected mod folder")
    } else {
        (
            library.getattr("root")?.str()?.to_str()?.to_owned(),
            "Manager library",
        )
    };
    let options = PyDict::new(py);
    options.set_item("directory", true)?;
    let path = py
        .import("creation_lib.mod_manager.launcher")?
        .getattr("local_path")?
        .call((source, label), Some(&options))?;
    let options = PyDict::new(py);
    options.set_item("shell", false)?;
    Ok(py
        .import("subprocess")?
        .getattr("Popen")?
        .call(
            (vec![
                "explorer.exe".to_string(),
                path.str()?.to_str()?.to_owned(),
            ],),
            Some(&options),
        )?
        .unbind())
}
#[pyfunction]
fn runtime_user_targets(py: Python<'_>) -> PyResult<Py<PyAny>> {
    let module = py.import("creation_lib.mod_manager.runtime")?;
    let local = module.call_method1("known_folder", ("F1B32785-6FBA-4FCF-9D55-7B8E7F157091",))?;
    let documents =
        module.call_method1("known_folder", ("FDD39AD0-238F-46AF-ADB4-6C85480369C7",))?;
    Ok((
        local.call_method1("joinpath", ("Fallout4",))?,
        documents.call_method1("joinpath", ("My Games", "Fallout4"))?,
    )
        .into_pyobject(py)?
        .into_any()
        .unbind())
}
#[pyfunction]
fn nexus_receive(workspace: &Bound<'_, PyAny>, request: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = workspace.py();
    let inbox = workspace.getattr("_link_inbox")?;
    let manager = workspace.getattr("manager")?;
    let result: PyResult<()> = (|| {
        let value = py
            .import("creation_lib.mod_manager.nxm_routing")?
            .call_method1(
                "validate",
                (
                    &manager,
                    request.getattr("destination")?.getattr("context")?,
                ),
            )?;
        manager.setattr("nexus_game", value.get_item("game")?)?;
        let options = PyDict::new(py);
        options.set_item("destination", value)?;
        manager
            .getattr("accept_nxm")?
            .call((request.getattr("link")?,), Some(&options))?;
        Ok(())
    })();
    if let Err(error) = result {
        if !error.matches(
            py,
            py.import("creation_lib.mod_manager.paths")?
                .getattr("ManagerError")?,
        )? {
            return Err(error);
        }
        inbox.call_method1("mark", (request, "stale"))?;
        let receipt = inbox.call_method1("_receipt", (request.getattr("id")?,))?;
        let options = PyDict::new(py);
        options.set_item("missing_ok", true)?;
        receipt.getattr("unlink")?.call((), Some(&options))?;
        let request = py
            .import("creation_lib.mod_manager.nxm_instances")?
            .getattr("Request")?
            .call1((request.getattr("id")?, request.getattr("link")?))?;
        workspace
            .getattr("nxm_router")?
            .call_method1("open", (request,))?;
        return Ok(());
    }
    workspace.setattr("_tab", "Nexus")?;
    if let Err(error) = inbox.call_method1("mark", (request, "delivered")) {
        if !error.is_instance_of::<pyo3::exceptions::PyOSError>(py)
            && !error.is_instance_of::<pyo3::exceptions::PyValueError>(py)
        {
            return Err(error);
        }
        manager.setattr("status", "Browser link is queued here, but its delivery receipt could not be updated. Review this queue before retrying; no download was started.")?;
    }
    Ok(())
}

#[pyfunction]
fn profile_save_scoped(
    library: &Bound<'_, PyAny>,
    profile: &str,
    name: &str,
    text: &str,
    expected: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let scoped = library
        .getattr("save_ini")
        .unwrap_or_else(|_| py.None().into_bound(py));
    if scoped.is_truthy()? {
        return Ok(scoped
            .call1((profile, name, text, expected, cancel))?
            .unbind());
    }
    Ok(py
        .import("creation_lib.mod_manager.profile_state")?
        .call_method1(
            "_edit",
            (library, "save", profile, name, text, expected, cancel),
        )?
        .unbind())
}

#[pyfunction]
fn agent_dispatch(args: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = args.py();
    let module = py.import("creation_lib.mod_manager.agent_launch")?;
    let command = args.getattr("command")?.extract::<String>()?;
    if command == "instances" {
        let root = args.getattr("root")?;
        return Ok(module
            .call_method1(
                "instances",
                (if root.is_truthy()? {
                    root
                } else {
                    py.None().into_bound(py)
                },),
            )?
            .unbind());
    }
    let root = args.getattr("library")?;
    let library = if root.is_truthy()? {
        py.import("pathlib")?.call_method1("Path", (root,))?
    } else {
        module.call_method0("default_agent_library")?
    };
    let result = match command.as_str() {
        "sync" => module.call_method1("sync", (args.getattr("mo2")?, library))?,
        "launch" => {
            let mo2 = args.getattr("mo2")?;
            let synced = if mo2.is_truthy()? {
                module.call_method1("sync", (mo2, &library))?
            } else {
                py.None().into_bound(py)
            };
            let options = PyDict::new(py);
            options.set_item("overrides", args.getattr("ini")?)?;
            for key in [
                "agent_defaults",
                "ini_source",
                "accept_minimal_ini",
                "wait_game",
                "dry_run",
                "mcp_port",
                "disable_mcp",
                "isolate_documents",
            ] {
                options.set_item(key, args.getattr(key)?)?;
            }
            let result = module.getattr("launch")?.call(
                (library, args.getattr("profile")?, args.getattr("exe")?),
                Some(&options),
            )?;
            if synced.is_truthy()? {
                let summary = PyDict::new(py);
                for key in ["state", "unavailable", "issues"] {
                    summary.set_item(key, synced.get_item(key)?)?;
                }
                result.set_item("sync", summary)?;
            }
            result
        }
        "import-saves" => {
            let options = PyDict::new(py);
            options.set_item("latest", args.getattr("latest")?)?;
            let folder = args.getattr("folder")?;
            options.set_item(
                "folder",
                if folder.is_truthy()? {
                    folder
                } else {
                    py.None().into_bound(py)
                },
            )?;
            module.getattr("import_saves")?.call(
                (library, args.getattr("profile")?, args.getattr("save")?),
                Some(&options),
            )?
        }
        "status" => {
            let session = args.getattr("session")?;
            module.call_method1(
                "status",
                (
                    library,
                    if session.is_truthy()? {
                        session
                    } else {
                        py.None().into_bound(py)
                    },
                ),
            )?
        }
        "stop" => {
            let options = PyDict::new(py);
            options.set_item("timeout", args.getattr("timeout")?)?;
            module
                .getattr("stop")?
                .call((library, args.getattr("pid")?), Some(&options))?
        }
        _ => return Err(invalid("Unknown manager command.")),
    };
    Ok(result.unbind())
}

#[pyfunction]
fn runtime_freeze_datas(
    project_root: PathBuf,
    public: bool,
    onefile: bool,
    builder: &Bound<'_, PyAny>,
) -> PyResult<Vec<(String, String)>> {
    if public {
        return Err(invalid(
            "Public toolkit packaging remains blocked pending a distribution audit, including proprietary Autodesk FBX and optional legacy runtime dependencies. The project's original filespace uses BSD MinHook notices; it does not require upstream USVFS. Private execution remains supported.",
        ));
    }
    if onefile {
        return Err(invalid(
            "Project filespace requires an onedir application package so helper ownership identifies the actual process. Onefile is not certified.",
        ));
    }
    let py = builder.py();
    let options = PyDict::new(py);
    options.set_item(
        "cache",
        py_path(
            py,
            &project_root
                .join("cache")
                .join("mod-manager-project-vfs-package"),
        )?,
    )?;
    let root = builder.getattr("ensure")?.call((), Some(&options))?;
    builder.call_method1("validate", (&root,))?;
    let mut data = vec![];
    for name in builder.getattr("PAYLOAD_FILES")?.try_iter()? {
        data.push((
            root.call_method1("joinpath", (name?,))?
                .str()?
                .to_str()?
                .to_owned(),
            "creation_lib/mod_manager/_runtime/project-vfs".to_string(),
        ));
    }
    data.push((
        root.call_method1("joinpath", ("native-receipt.json",))?
            .str()?
            .to_str()?
            .to_owned(),
        "creation_lib/mod_manager/_runtime/project-vfs".to_string(),
    ));
    let source = builder.getattr("SOURCE")?;
    for path in source.call_method1("rglob", ("*",))?.try_iter()? {
        let path = path?;
        if path.call_method0("is_file")?.is_truthy()? {
            data.push((
                path.str()?.to_str()?.to_owned(),
                format!(
                    "py_creation_lib/native/mod_manager_vfs/{}",
                    path.call_method1("relative_to", (&source,))?
                        .getattr("parent")?
                        .str()?
                        .to_str()?
                        .replace('\\', "/")
                ),
            ));
        }
    }
    let icon = project_root.join("resource").join("icon.ico");
    if icon.is_file() {
        data.push((
            py_path(py, &icon)?.str()?.to_str()?.to_owned(),
            "resource".to_string(),
        ));
    }
    Ok(data)
}

#[pyfunction]
fn runtime_dispatch(arguments: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = arguments.py();
    if arguments.len()? == 0
        || arguments.get_item(0)?.extract::<String>()? != "--modbox21-runtime-helper"
    {
        return Ok(false);
    }
    if arguments.len()? != 2 {
        return Err(pyo3::exceptions::PySystemExit::new_err(
            "Runtime helper requires one owned request file.",
        ));
    }
    let result = py
        .import("creation_lib.mod_manager.runtime_helper")?
        .call_method1("run", (arguments.get_item(1)?,))?;
    Err(PyErr::from_value(
        py.get_type::<pyo3::exceptions::PySystemExit>()
            .call1((result,))?,
    ))
}

#[pyfunction]
fn download_store_observed(
    queue: &Bound<'_, PyAny>,
    job: &Bound<'_, PyAny>,
    state: &str,
    record: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = queue.py();
    let library = queue.getattr("library")?;
    let allowed = crate::python_io::with_context(&library.call_method0("_db")?, |db| {
        let raw = py
            .import("creation_lib.mod_manager.backend")?
            .call_method1(
                "call",
                (
                    "download_store",
                    db.getattr("native")?,
                    job.get_item("id")?,
                    state,
                    py.import("json")?.call_method1("dumps", (record,))?,
                ),
            )?;
        Ok(py.import("json")?.call_method1("loads", (raw,))?.unbind())
    })?;
    job.set_item("state", state)?;
    job.set_item("record", allowed.bind(py))?;
    let observer = queue.getattr("observer")?;
    if observer.is_truthy()? {
        let snapshot = py.import("builtins")?.call_method1("dict", (job,))?;
        snapshot.set_item(
            "record",
            py.import("builtins")?
                .call_method1("dict", (allowed.bind(py),))?,
        )?;
        observer.call1((snapshot,))?;
    }
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(runtime_freeze_datas, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_dispatch, m)?)?;
    m.add_function(wrap_pyfunction!(download_store_observed, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_receive, m)?)?;
    m.add_function(wrap_pyfunction!(profile_save_scoped, m)?)?;
    m.add_function(wrap_pyfunction!(agent_dispatch, m)?)?;
    m.add_function(wrap_pyfunction!(metadata_remember, m)?)?;
    m.add_function(wrap_pyfunction!(folder_reveal, m)?)?;
    m.add_function(wrap_pyfunction!(runtime_user_targets, m)?)?;
    Ok(())
}
