use crate::{
    agent_library::{library, profile},
    history_prune::py_path,
    invalid,
    plugin_inventory::{object, value},
    reject_links, relative_path,
    saves::fold,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use serde_json::{Value, json};
use std::{collections::HashSet, path::PathBuf};

fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("creation_lib.mod_manager.agent_launch")
}
fn runtime(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("creation_lib.mod_manager.runtime")
}
fn identity(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("creation_lib.mod_manager.process_identity")
}
#[pyfunction]
fn agent_detached_spawn(
    args: &Bound<'_, PyAny>,
    popen: &Bound<'_, PyAny>,
    kwargs: &Bound<'_, PyDict>,
) -> PyResult<Py<PyAny>> {
    let py = args.py();
    let kwargs = kwargs.copy()?;
    let flags = kwargs
        .get_item("creationflags")?
        .map(|v| v.extract::<u32>())
        .transpose()?
        .unwrap_or(0)
        | 0x08000000;
    kwargs.set_item("creationflags", flags | 0x01000000)?;
    match popen.call((args,), Some(&kwargs)) {
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyPermissionError>(py) => {
            kwargs.set_item("creationflags", flags)?;
            Ok(popen.call((args,), Some(&kwargs))?.unbind())
        }
        result => result.map(Bound::unbind),
    }
}
fn executable<'py>(
    library: &Bound<'py, PyAny>,
    instance: &Bound<'py, PyAny>,
    game_root: &str,
    wanted: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = library.py();
    let mut choices = Vec::new();
    let mut matches = Vec::new();
    let mut f4se = Vec::new();
    let folded = fold(&wanted.str()?.extract::<String>()?);
    let game = py_path(py, &PathBuf::from(game_root))?.call_method0("resolve")?;
    for e in library
        .call_method1("executables", (instance,))?
        .try_iter()?
    {
        let e = e?;
        let title: String = e.getattr("title")?.extract()?;
        choices.push(title.clone());
        if e.getattr("id")?.eq(wanted)? || fold(&title) == folded {
            matches.push(e.clone());
        }
        if folded == "f4se" {
            let binary: PathBuf = e.getattr("binary")?.extract()?;
            if fold(&binary.file_name().unwrap_or_default().to_string_lossy()) == "f4se_loader.exe"
                && py_path(py, binary.parent().unwrap_or(&binary))?
                    .call_method0("resolve")?
                    .eq(&game)?
            {
                f4se.push(e);
            }
        }
    }
    if matches.is_empty() {
        matches = f4se;
    }
    if matches.len() != 1 {
        return Err(invalid(format!(
            "No executable uniquely matches {}. Choose one of: {}",
            wanted.repr()?,
            choices.join(", ")
        )));
    }
    Ok(matches.remove(0))
}
fn write_mappings(
    library: &Bound<'_, PyAny>,
    plan: &Bound<'_, PyAny>,
) -> PyResult<(PathBuf, Value, usize)> {
    let py = library.py();
    let root: PathBuf = library.getattr("root")?.extract()?;
    let session: String = plan.getattr("session_id")?.extract()?;
    let output_root = root
        .join("runtime")
        .join("sessions")
        .join(relative_path(&session, false)?)
        .join("output");
    let isolated = plan.getattr("isolate_documents")?.is_truthy()?;
    let mut output = json!({"overwrite":output_root.join("overwrite").to_string_lossy(), "saves":output_root.join("saves").to_string_lossy(), "ini":output_root.join("ini").to_string_lossy(), "plugins":output_root.join("plugins").to_string_lossy(), "f4se_logs":output_root.join("f4se-logs").to_string_lossy(), "game_root":output_root.join("game-root").to_string_lossy()});
    let game: PathBuf = plan.getattr("game_root")?.extract()?;
    let ini: PathBuf = plan.getattr("ini_target")?.extract()?;
    output["documents"] = json!(if isolated {
        output_root.join("ini")
    } else {
        ini.clone()
    });
    if !isolated {
        output["saves"] = json!(ini.join("Saves"));
        output["f4se_logs"] = json!(ini.join("F4SE"));
    }
    let mut mounts = serde_json::Map::new();
    for (target, key) in [
        (game.join("Data"), "overwrite"),
        (
            plan.getattr("plugin_target")?.extract::<PathBuf>()?,
            "plugins",
        ),
        (ini.clone(), "ini"),
        (ini.join("F4SE"), "f4se_logs"),
        (
            ini.join(plan.getattr("save_subpath")?.extract::<String>()?),
            "saves",
        ),
        (game, "game_root"),
    ] {
        if !isolated && ["ini", "saves", "f4se_logs"].contains(&key) {
            continue;
        }
        mounts.insert(target.to_string_lossy().into_owned(), output[key].clone());
    }
    let mut files = Vec::new();
    for (attribute, prefix) in [("files", "Data/"), ("root_files", "")] {
        for file in plan.getattr(attribute)?.try_iter()? {
            let file = file?;
            files.push(json!({"path":format!("{prefix}{}", file.get_item(0)?.extract::<String>()?), "source":file.get_item(1)?.extract::<String>()?}));
        }
    }
    let path = output_root.join("agent-mappings.json");
    reject_links(&path)?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let temporary = path.with_file_name("agent-mappings.json.new");
    reject_links(&temporary)?;
    std::fs::write(
        &temporary,
        json!({"files":files, "mounts":mounts}).to_string(),
    )?;
    py_path(py, &temporary)?.call_method1("replace", (py_path(py, &path)?,))?;
    Ok((path, output, files.len()))
}
fn open_verified(py: Python<'_>, expected: &Bound<'_, PyAny>, access: u32) -> PyResult<Py<PyAny>> {
    if !crate::process_identity::valid_process_identity(expected)? {
        return Ok(py.None());
    }
    let identity = identity(py)?;
    let api = identity.call_method0("kernel_api")?;
    let handle = api.call_method1(
        "OpenProcess",
        (access | 0x1000, false, expected.get_item("pid")?),
    )?;
    if !handle.is_truthy()? {
        return Ok(py.None());
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item("kernel", &api)?;
    let result = identity
        .getattr("from_handle")?
        .call((expected.get_item("pid")?, &handle), Some(&kwargs));
    match result {
        Ok(found) if found.eq(expected)? => return Ok(handle.unbind()),
        Err(e) if !e.is_instance_of::<pyo3::exceptions::PyValueError>(py) => {
            api.call_method1("CloseHandle", (handle,))?;
            return Err(e);
        }
        _ => (),
    }
    api.call_method1("CloseHandle", (handle,))?;
    Ok(py.None())
}
#[pyfunction]
fn agent_processes(status: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = status.py();
    let status = value(status)?;
    let mut pairs = vec![(status["pid"].clone(), status["process_identity"].clone())];
    if let (Some(children), Some(identities)) = (
        status["children"].as_array(),
        status["child_identities"].as_array(),
    ) {
        pairs.extend(children.iter().cloned().zip(identities.iter().cloned()));
    }
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for (pid, expected) in pairs {
        if pid.is_null() || !seen.insert(pid.to_string()) {
            continue;
        }
        let expected = object(py, &expected)?;
        let handle = open_verified(py, &expected, 0)?;
        let mut image = Value::Null;
        if handle.bind(py).is_truthy()? {
            let query = (|| -> PyResult<Value> {
                let ct = py.import("ctypes")?;
                let buffer = ct.call_method1("create_unicode_buffer", (32768,))?;
                let size = py
                    .import("ctypes.wintypes")?
                    .call_method1("DWORD", (32768,))?;
                if module(py)?
                    .call_method0("_kernel")?
                    .call_method1(
                        "QueryFullProcessImageNameW",
                        (
                            handle.bind(py),
                            0,
                            &buffer,
                            ct.call_method1("byref", (size,))?,
                        ),
                    )?
                    .is_truthy()?
                {
                    value(&buffer.getattr("value")?)
                } else {
                    Ok(Value::Null)
                }
            })();
            identity(py)?
                .call_method0("kernel_api")?
                .call_method1("CloseHandle", (handle,))?;
            image = query?;
        }
        let state: String = identity(py)?
            .call_method1("process_state", (expected,))?
            .extract()?;
        result.push(json!({"pid":pid, "image":image, "state":state}));
    }
    Ok(object(py, &json!(result))?.unbind())
}
fn game_pid(found: &Value) -> Value {
    found
        .as_array()
        .and_then(|values| {
            values.iter().find(|item| {
                item["state"] == "alive"
                    && item["image"].as_str().is_some_and(|s| {
                        fold(
                            &PathBuf::from(s)
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy(),
                        ) == "fallout4.exe"
                    })
            })
        })
        .map(|item| item["pid"].clone())
        .unwrap_or(Value::Null)
}
#[pyfunction]
fn agent_launch(
    root: &Bound<'_, PyAny>,
    wanted: &Bound<'_, PyAny>,
    exe: &Bound<'_, PyAny>,
    options: &Bound<'_, PyDict>,
) -> PyResult<Py<PyAny>> {
    let py = root.py();
    let options = options.as_any();
    let mcp = options.get_item("mcp_port")?;
    let disabled: bool = options.get_item("disable_mcp")?.extract()?;
    crate::validate_mcp_options(mcp.extract::<Option<u16>>()?, disabled)?;
    let library = library(py, root)?;
    let selected = profile(&library, wanted)?;
    let id = selected.getattr("id")?;
    let configuration = value(
        &library
            .call_method1("profile_snapshot", (&id,))?
            .get_item(2)?,
    )?;
    let game = configuration["game"]["gamepath"].as_str().unwrap_or("");
    let chosen = executable(
        &library,
        &selected
            .getattr("files")?
            .call_method1("get", ("_modbox_instance_id", ""))?,
        game,
        exe,
    )?;
    let cancel = options.get_item("cancel")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("user_targets", options.get_item("user_targets")?)?;
    kwargs.set_item("isolate_documents", options.get_item("isolate_documents")?)?;
    let plan = runtime(py)?
        .getattr("prepare")?
        .call((&library, &id, &chosen, game, "", &cancel), Some(&kwargs))?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("mcp_port", mcp)?;
    kwargs.set_item("disable_mcp", disabled)?;
    let plan = py
        .import("dataclasses")?
        .getattr("replace")?
        .call((plan,), Some(&kwargs))?;
    let specs = PyList::empty(py);
    if options.get_item("agent_defaults")?.is_truthy()? {
        for spec in module(py)?.getattr("AGENT_DEFAULTS")?.try_iter()? {
            specs.append(crate::agent_ini::agent_parse_override(&spec?)?)?;
        }
    }
    for spec in options.get_item("overrides")?.try_iter()? {
        specs.append(crate::agent_ini::agent_parse_override(&spec?)?)?;
    }
    let prepared = crate::agent_ini::agent_apply_launch_settings(
        &library,
        &plan,
        &selected.getattr("files")?,
        specs.as_any(),
        &options.get_item("ini_source")?.extract::<String>()?,
    )?;
    let prepared = prepared.bind(py);
    let plan = prepared.get_item(0)?;
    let (mappings, output, count) = write_mappings(&library, &plan)?;
    let mut result = json!({"library":library.getattr("root")?.str()?.extract::<String>()?, "profile":{"id":id.extract::<String>()?, "name":selected.getattr("name")?.extract::<String>()?}, "executable":{"title":chosen.getattr("title")?.extract::<String>()?, "argv":value(&plan.getattr("launch")?.getattr("argv")?)?}, "game_root":plan.getattr("game_root")?.extract::<String>()?, "state":"planned", "mapped_files":count, "mappings":mappings.to_string_lossy(), "output":output, "mcp_disabled":disabled, "minimal_ini":value(&plan.getattr("minimal_ini")?)?, "compatibility":value(&plan.getattr("compatibility")?)?});
    let report = value(&prepared.get_item(1)?)?;
    result["isolate_documents"] = json!(plan.getattr("isolate_documents")?.is_truthy()?);
    result
        .as_object_mut()
        .unwrap()
        .extend(report.as_object().unwrap().clone());
    if options.get_item("dry_run")?.is_truthy()? {
        return Ok(object(py, &result)?.unbind());
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item(
        "accept_minimal_ini",
        options.get_item("accept_minimal_ini")?,
    )?;
    kwargs.set_item("cancel", &cancel)?;
    kwargs.set_item("spawn", module(py)?.getattr("detached_spawn")?)?;
    let started = runtime(py)?
        .getattr("start")?
        .call((&library, &plan), Some(&kwargs))?;
    let session = started.get_item(0)?;
    let child = started.get_item(1)?;
    let mut status = started.get_item(2)?;
    let wait: f64 = options.get_item("wait_game")?.extract()?;
    let time = py.import("time")?;
    let deadline = time.call_method0("monotonic")?.extract::<f64>()? + wait.max(0.0);
    let mut found = if wait != 0.0 {
        value(agent_processes(&status)?.bind(py))?
    } else {
        json!([])
    };
    while time.call_method0("monotonic")?.extract::<f64>()? < deadline
        && status.call_method1("get", ("state",))?.eq("running")?
        && game_pid(&found).is_null()
    {
        time.call_method1("sleep", (0.25,))?;
        status = runtime(py)?.call_method1("session_status", (&session,))?;
        found = value(agent_processes(&status)?.bind(py))?;
    }
    let status = value(&status)?;
    result["session"] = json!(session.getattr("name")?.extract::<String>()?);
    result["session_dir"] = json!(session.str()?.extract::<String>()?);
    for key in ["state", "pid", "mcp_port", "mcp_endpoint"] {
        result[key] = status[key].clone();
    }
    result["children"] = status.get("children").cloned().unwrap_or(json!([]));
    result["helper_pid"] = status
        .get("helper_pid")
        .cloned()
        .unwrap_or(value(&child.getattr("pid")?)?);
    result["mcp_disabled"] = status
        .get("mcp_disabled")
        .cloned()
        .unwrap_or(json!(disabled));
    result["game_pid"] = game_pid(&found);
    result["processes"] = found;
    Ok(object(py, &result)?.unbind())
}
#[pyfunction]
fn agent_status(root: &Bound<'_, PyAny>, session: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = root.py();
    let root = py
        .import("pathlib")?
        .getattr("Path")?
        .call1((root,))?
        .call_method0("absolute")?;
    if session.is_truthy()? {
        let name: String = session.extract()?;
        let path = crate::session_path(root.extract()?, &name)?;
        let entry = py_path(py, &PathBuf::from(path))?;
        let status = runtime(py)?.call_method1("session_status", (&entry,))?;
        let found = value(agent_processes(&status)?.bind(py))?;
        let mut current = value(&status)?;
        let current = current
            .as_object_mut()
            .ok_or_else(|| invalid("Invalid session status."))?;
        current.insert(
            "session".into(),
            json!(entry.getattr("name")?.extract::<String>()?),
        );
        current.insert(
            "liveness".into(),
            value(&identity(py)?.call_method1("session_liveness", (status,))?)?,
        );
        current.insert("game_pid".into(), game_pid(&found));
        current.insert("processes".into(), found);
        return Ok(object(py, &json!(current))?.unbind());
    }
    let mut sessions = Vec::new();
    for item in runtime(py)?
        .call_method1("active_sessions", (&root,))?
        .try_iter()?
    {
        let item = item?;
        let entry = item.get_item(0)?;
        let current = item.get_item(1)?;
        let found = value(agent_processes(&current)?.bind(py))?;
        let current = value(&current)?;
        let mut record = json!({"session":entry.getattr("name")?.extract::<String>()?, "session_dir":entry.str()?.extract::<String>()?, "processes":found, "game_pid":game_pid(&found), "children":current.get("children").cloned().unwrap_or(json!([])), "mcp_disabled":current.get("mcp_disabled").cloned().unwrap_or(json!(false))});
        for key in [
            "state",
            "helper_pid",
            "pid",
            "profile_id",
            "mcp_port",
            "mcp_endpoint",
            "output_root",
        ] {
            record[key] = current[key].clone();
        }
        sessions.push(record);
    }
    Ok(object(
        py,
        &json!({"library":root.str()?.extract::<String>()?, "sessions":sessions}),
    )?
    .unbind())
}
#[pyfunction]
fn agent_stop(root: &Bound<'_, PyAny>, pid: i64, timeout: f64) -> PyResult<Py<PyAny>> {
    let py = root.py();
    let root = py
        .import("pathlib")?
        .getattr("Path")?
        .call1((root,))?
        .call_method0("absolute")?;
    for item in runtime(py)?
        .call_method1("active_sessions", (root,))?
        .try_iter()?
    {
        let item = item?;
        let entry = item.get_item(0)?;
        let current = item.get_item(1)?;
        let data = value(&current)?;
        if data["helper_pid"] != pid
            && data["pid"] != pid
            && !data["children"]
                .as_array()
                .is_some_and(|v| v.contains(&json!(pid)))
        {
            continue;
        }
        let handle = open_verified(
            py,
            &object(py, &data["helper_identity"])?,
            0x0001 | 0x100000,
        )?;
        if !handle.bind(py).is_truthy()? {
            return Err(invalid(
                "The session's helper creation identity cannot be verified; nothing was terminated.",
            ));
        }
        let result = module(py)?
            .call_method0("_kernel")?
            .call_method1("TerminateProcess", (handle.bind(py), 1));
        identity(py)?
            .call_method0("kernel_api")?
            .call_method1("CloseHandle", (handle,))?;
        if !result?.is_truthy()? {
            return Err(invalid(format!(
                "Windows refused to stop the session helper ({}).",
                py.import("ctypes")?.call_method0("get_last_error")?.str()?
            )));
        }
        let time = py.import("time")?;
        let deadline = time.call_method0("monotonic")?.extract::<f64>()? + timeout;
        let mut liveness: String = identity(py)?
            .call_method1("session_liveness", (&current,))?
            .extract()?;
        while liveness != "exited" && time.call_method0("monotonic")?.extract::<f64>()? < deadline {
            time.call_method1("sleep", (0.1,))?;
            liveness = identity(py)?
                .call_method1("session_liveness", (&current,))?
                .extract()?;
        }
        return Ok(object(py, &json!({"stopped":true, "session":entry.getattr("name")?.extract::<String>()?, "helper_pid":data["helper_pid"], "liveness":liveness}))?.unbind());
    }
    Err(invalid(format!(
        "No live session of this library owns PID {pid}; nothing was terminated."
    )))
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(agent_detached_spawn, m)?)?;
    m.add_function(wrap_pyfunction!(agent_processes, m)?)?;
    m.add_function(wrap_pyfunction!(agent_launch, m)?)?;
    m.add_function(wrap_pyfunction!(agent_status, m)?)?;
    m.add_function(wrap_pyfunction!(agent_stop, m)?)?;
    Ok(())
}
