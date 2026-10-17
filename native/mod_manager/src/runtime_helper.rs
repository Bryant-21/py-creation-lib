use crate::{
    encoding,
    history_prune::py_path,
    invalid,
    plugin_inventory::{object, value},
    process_identity::valid_process_identity,
    reject_links, relative_path,
    saves::fold,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("creation_lib.mod_manager.runtime_helper")
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn items<'a>(v: &'a Value, key: &str) -> PyResult<&'a [Value]> {
    v.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| invalid(format!("Invalid runtime {key}.")))
}
fn read(path: &Path) -> PyResult<Value> {
    encoding::parse(&std::fs::read_to_string(path)?)
}
fn write(py: Python<'_>, path: &Path, text: &str, raw: bool) -> PyResult<()> {
    let kwargs = PyDict::new(py);
    kwargs.set_item("encoding", "utf-8")?;
    if raw {
        kwargs.set_item("newline", "")?;
    }
    py_path(py, path)?.call_method("write_text", (text,), Some(&kwargs))?;
    Ok(())
}
fn replace(py: Python<'_>, source: &Path, target: &Path) -> PyResult<()> {
    py_path(py, source)?.call_method1("replace", (py_path(py, target)?,))?;
    Ok(())
}
fn current(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("creation_lib.mod_manager.process_identity")?
        .call_method0("current_identity")
}
fn now(py: Python<'_>) -> PyResult<f64> {
    py.import("time")?.call_method0("monotonic")?.extract()
}
#[pyfunction]
fn helper_write_status(path: PathBuf, fields: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = fields.py();
    let temporary = path.with_extension("tmp");
    reject_links(&path)?;
    reject_links(&temporary)?;
    let mut status =
        json!({"helper_pid":std::process::id(),"helper_identity":value(&current(py)?)?});
    status.as_object_mut().unwrap().extend(
        value(fields)?
            .as_object()
            .ok_or_else(|| invalid("Invalid runtime status fields."))?
            .clone(),
    );
    write(py, &temporary, &status.to_string(), false)?;
    for attempt in 0..20 {
        match replace(py, &temporary, &path) {
            Ok(()) => return Ok(()),
            Err(error) if error.is_instance_of::<pyo3::exceptions::PyPermissionError>(py) => {
                let code = error
                    .value(py)
                    .getattr("winerror")
                    .ok()
                    .and_then(|v| v.extract::<u32>().ok());
                if !cfg!(windows) || !matches!(code, Some(5 | 32 | 33)) || attempt == 19 {
                    return Err(error);
                }
                py.import("time")?.call_method1("sleep", (0.01,))?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
fn signature(py: Python<'_>, path: &Path) -> PyResult<Value> {
    value(
        &py.import("creation_lib.mod_manager.launcher")?
            .call_method1("fingerprint", (py_path(py, path)?,))?,
    )
}
fn validate(py: Python<'_>, plan: &Value) -> PyResult<()> {
    for item in plan
        .get("root_files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = item[0].as_str().unwrap_or("");
        let lower = fold(name);
        let version = lower
            .strip_prefix("f4se_")
            .and_then(|s| s.strip_suffix(".dll"))
            .is_some_and(|s| {
                let parts: Vec<_> = s.split('_').collect();
                parts.len() == 3
                    && parts
                        .iter()
                        .all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()))
            });
        if lower != "f4se_steam_loader.dll" && lower != "f4se_loader.exe" && !version {
            return Err(invalid("Unsupported F4SE root mapping."));
        }
        let path = PathBuf::from(item[1].as_str().unwrap_or(""));
        reject_links(&path)?;
        if signature(py, &path)? != item[2] {
            return Err(invalid("Managed F4SE changed after launch review."));
        }
    }
    for item in plan
        .get("target_signatures")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let path = PathBuf::from(item[0].as_str().unwrap_or(""));
        reject_links(&path)?;
        if signature(py, &path)? != item[1] {
            return Err(invalid(
                "F4SE target or configuration changed after review; no process was started.",
            ));
        }
    }
    for item in items(plan, "files")? {
        let name = item[0].as_str().unwrap_or("");
        relative_path(name, false)?;
        let path = PathBuf::from(item[1].as_str().unwrap_or(""));
        reject_links(&path)?;
        if signature(py, &path)? != item[2] {
            return Err(invalid(format!(
                "Mod file changed after launch review: {name}. Review the profile again."
            )));
        }
    }
    let binary = PathBuf::from(plan["launch"]["argv"][0].as_str().unwrap_or(""));
    reject_links(&binary)?;
    if signature(py, &binary)? != plan["launch"]["signature"] {
        return Err(invalid(
            "Executable changed after review; no process was started.",
        ));
    }
    Ok(())
}
#[pyfunction]
fn helper_validate_files(plan: &Bound<'_, PyAny>) -> PyResult<()> {
    validate(plan.py(), &value(plan)?)
}
fn checkpoint(cancel: &Path) -> PyResult<()> {
    reject_links(cancel)?;
    if cancel.exists() {
        return Err(crate::OperationCancelled::new_err(
            "Launch cancelled before process creation; no game was started.",
        ));
    }
    Ok(())
}
fn report(py: Python<'_>, path: &Path, extra: &Bound<'_, PyDict>, fields: &Value) -> PyResult<()> {
    let kwargs = extra.copy()?;
    kwargs.update(object(py, fields)?.cast::<PyDict>()?.as_mapping())?;
    module(py)?.call_method("write_status", (py_path(py, path)?,), Some(&kwargs))?;
    Ok(())
}
#[pyclass]
struct BeforeResume {
    status: PathBuf,
    cancel: PathBuf,
    profile: String,
    extra: Py<PyDict>,
    state: Py<PyDict>,
}
#[pymethods]
impl BeforeResume {
    fn __call__(&self, pid: &Bound<'_, PyAny>, identity: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = pid.py();
        let state = self.state.bind(py);
        state.set_item("children", PyList::new(py, [pid])?)?;
        state.set_item("process_identity", identity)?;
        let valid = valid_process_identity(identity)?;
        state.set_item(
            "child_identities",
            if valid {
                PyList::new(py, [identity])?
            } else {
                PyList::empty(py)
            },
        )?;
        report(
            py,
            &self.status,
            self.extra.bind(py),
            &json!({"state":"starting","pid":value(pid)?,"process_identity":value(identity)?,"children":value(&state.get_item("children")?.unwrap())?,"child_identities":value(&state.get_item("child_identities")?.unwrap())?,"profile_id":self.profile}),
        )?;
        if !valid || !identity.get_item("pid")?.eq(pid)? {
            return Err(invalid(
                "Created process identity could not be verified; it was not resumed.",
            ));
        }
        checkpoint(&self.cancel)
    }
}
fn request(path: &Path) -> PyResult<Value> {
    reject_links(path)?;
    if std::fs::metadata(path)?.len() > 100 * 1024 * 1024 {
        return Err(invalid("Runtime request exceeds its safety limit."));
    }
    read(path)
}
fn mount(adapter: &Bound<'_, PyAny>, source: &Path, target: &Path) -> PyResult<()> {
    let py = adapter.py();
    let kwargs = PyDict::new(py);
    kwargs.set_item("create", true)?;
    adapter.call_method(
        "link_directory",
        (py_path(py, source)?, py_path(py, target)?),
        Some(&kwargs),
    )?;
    Ok(())
}
#[pyfunction]
fn helper_run(
    request_path: PathBuf,
    native_factory: &Bound<'_, PyAny>,
    wait: &Bound<'_, PyAny>,
) -> PyResult<i32> {
    let py = wait.py();
    let request_path: PathBuf = py_path(py, &request_path)?
        .call_method0("absolute")?
        .extract()?;
    let plan = request(&request_path)?;
    let state = PathBuf::from(text(&plan, "library_root"));
    let owner = if text(&plan, "metadata_root").is_empty() {
        state.clone()
    } else {
        PathBuf::from(text(&plan, "metadata_root"))
    };
    reject_links(&owner)?;
    reject_links(&owner.join(".modbox21-library.json"))?;
    if !owner.join(".modbox21-library.json").is_file() {
        return Err(invalid("Runtime metadata owner is unavailable."));
    }
    if !text(&plan, "target_id").is_empty() {
        if state
            != owner
                .join(".launch-targets")
                .join(crate::transactions::transaction_key(text(
                    &plan,
                    "target_id",
                ))?)
        {
            return Err(invalid("Target runtime owner differs."));
        }
    } else if owner != state {
        return Err(invalid("Unexpected runtime metadata owner."));
    }
    let _lease = crate::RuntimeLease::new(owner.join(".library.lock"), true)?;
    run(&request_path, native_factory, wait)
}
fn run(
    request_path: &Path,
    native_factory: &Bound<'_, PyAny>,
    wait: &Bound<'_, PyAny>,
) -> PyResult<i32> {
    let py = wait.py();
    let plan = request(request_path)?;
    let plan_object = object(py, &plan)?;
    let root = PathBuf::from(text(&plan, "library_root"));
    reject_links(&root)?;
    if !root.join(".modbox21-library.json").is_file()
        || !request_path.starts_with(root.join("runtime").join("sessions"))
    {
        return Err(invalid(
            "Runtime requests must belong to a manager-owned library session.",
        ));
    }
    let session = request_path
        .parent()
        .ok_or_else(|| invalid("Invalid runtime request path."))?;
    let status = session.join("status.json");
    let cancel = session.join("cancel");
    let profile_key = relative_path(text(&plan["launch"], "profile_id"), false)?;
    if profile_key.contains('/') {
        return Err(invalid("Invalid local profile identity."));
    }
    helper_write_status(
        status.clone(),
        &object(py, &json!({"state":"starting","profile_id":profile_key}))?,
    )?;
    let baseline_profile = root.join("runtime").join("profiles").join(&profile_key);
    let profile = session.join("output");
    reject_links(&profile)?;
    std::fs::create_dir_all(&profile)?;
    let lock_path = profile.join("active.lock");
    reject_links(&lock_path)?;
    let active_path = session.join("active.json");
    reject_links(&active_path)?;
    let mut adapter: Option<Bound<'_, PyAny>> = None;
    let mut started = false;
    let mut completed = false;
    let mut process_failed = false;
    let mut lock = None;
    let mut port_lease = None;
    let mut pid = Value::Null;
    let state = PyDict::new(py);
    state.set_item("children", PyList::empty(py))?;
    state.set_item("process_identity", py.None())?;
    state.set_item("child_identities", PyList::empty(py))?;
    let extra = PyDict::new(py);
    extra.set_item("output_root", profile.to_string_lossy().as_ref())?;
    let isolate_documents = plan["isolate_documents"].as_bool().unwrap_or(false);
    extra.set_item("isolate_documents", isolate_documents)?;
    extra.set_item(
        "documents_root",
        (if isolate_documents {
            profile.join("ini")
        } else {
            PathBuf::from(text(&plan, "ini_target"))
        })
        .to_string_lossy()
        .as_ref(),
    )?;
    extra.set_item(
        "f4se_logs",
        (if isolate_documents {
            profile.join("f4se-logs")
        } else {
            PathBuf::from(text(&plan, "ini_target")).join("F4SE")
        })
        .to_string_lossy()
        .as_ref(),
    )?;
    let result = (|| -> PyResult<()> {
        lock = Some(crate::RuntimeLease::new(lock_path, false)?);
        write(
            py,
            &active_path,
            &json!({"session":session.file_name().unwrap_or_default().to_string_lossy()})
                .to_string(),
            false,
        )?;
        checkpoint(&cancel)?;
        let owner_path = session.join("owner.json");
        reject_links(&owner_path)?;
        report(
            py,
            &status,
            &extra,
            &json!({"state":"starting","phase":"awaiting-owner","profile_id":profile_key}),
        )?;
        let deadline = now(py)? + 30.0;
        loop {
            checkpoint(&cancel)?;
            if owner_path.is_file() {
                if std::fs::metadata(&owner_path)?.len() > 65536 {
                    return Err(invalid(
                        "Runtime owner acknowledgement exceeds its safety limit.",
                    ));
                }
                let owner = read(&owner_path)?;
                let identity = object(py, &owner["helper_identity"])?;
                if owner["profile_id"] != profile_key
                    || owner["helper_pid"] != std::process::id()
                    || !valid_process_identity(&identity)?
                    || !identity.eq(current(py)?)?
                {
                    return Err(invalid(
                        "Runtime owner acknowledgement does not match this helper's creation identity.",
                    ));
                }
                break;
            }
            if now(py)? >= deadline {
                return Err(invalid(
                    "No validated runtime owner acknowledgement; no native process was started.",
                ));
            }
            wait.call1((0.05,))?;
        }
        validate(py, &plan)?;
        let runtime = py.import("creation_lib.mod_manager.runtime")?;
        let f4se = PathBuf::from(text(&plan["launch"]["executable"], "binary"))
            .file_name()
            .is_some_and(|v| fold(&v.to_string_lossy()) == "f4se_loader.exe");
        let validate_ini = || -> PyResult<()> {
            if f4se {
                runtime.call_method1(
                    "validate_f4se_configuration",
                    (
                        text(&plan, "game_root"),
                        py_path(py, &root)?,
                        &profile_key,
                        plan_object.get_item("files")?,
                    ),
                )?;
            }
            Ok(())
        };
        validate_ini()?;
        let library_class = py
            .import("creation_lib.mod_manager.library")?
            .getattr("Library")?;
        let mut metadata = py
            .import("builtins")?
            .getattr("object")?
            .getattr("__new__")?
            .call1((&library_class,))?;
        let metadata_root = if text(&plan, "metadata_root").is_empty() {
            root.clone()
        } else {
            PathBuf::from(text(&plan, "metadata_root"))
        };
        metadata.setattr("root", py_path(py, &metadata_root)?)?;
        metadata.setattr(
            "database",
            py_path(py, &metadata_root.join("library.sqlite3"))?,
        )?;
        let target = !text(&plan, "target_id").is_empty();
        let setup = py.import("creation_lib.mod_manager.f4se_setup")?;
        if target {
            let marker = metadata_root.join(library_class.getattr("MARKER")?.extract::<String>()?);
            reject_links(&marker)?;
            metadata.setattr("library_id", text(&read(&marker)?, "id"))?;
            metadata = py
                .import("creation_lib.mod_manager.launch_targets")?
                .getattr("TargetLibrary")?
                .call1((metadata, text(&plan, "target_id")))?;
            if metadata.getattr("root")?.extract::<PathBuf>()? != root {
                return Err(invalid(
                    "Target runtime state does not match its registered owner.",
                ));
            }
            let kwargs = PyDict::new(py);
            kwargs.set_item("owner_root", py_path(py, &root)?)?;
            setup.call_method(
                "validate_launch",
                (metadata.call_method0("target")?,),
                Some(&kwargs),
            )?;
        } else if metadata_root != root {
            return Err(invalid("Unexpected runtime metadata root."));
        }
        if !runtime
            .call_method1("profile_signature", (&metadata, &profile_key))?
            .eq(text(&plan, "profile_signature"))?
        {
            return Err(invalid(
                "Profile changed after runtime review; no process was started.",
            ));
        }
        let project = text(&plan, "runtime") == "modbox-vfs-fo4-x64";
        let bundle = if project {
            py.import("creation_lib.mod_manager.native_builder")?
                .call_method1("validate", (text(&plan, "bundle_root"),))?
        } else {
            runtime.call_method1("bundle", (text(&plan, "bundle_root"),))?
        };
        let directories: BTreeMap<_, _> = [
            "overwrite",
            "plugins",
            "ini",
            "saves",
            "f4se-logs",
            "temp",
            "game-root",
        ]
        .into_iter()
        .map(|name| (name, profile.join(name)))
        .collect();
        let saves = if target {
            metadata
                .call_method1("owned_save_root", (&profile_key,))?
                .extract::<PathBuf>()?
        } else {
            baseline_profile.join("saves")
        };
        if !text(&plan, "save_root").is_empty() && PathBuf::from(text(&plan, "save_root")) != saves
        {
            return Err(invalid("Owned save scope changed after runtime review."));
        }
        let namespace = if target {
            metadata
                .call_method1("save_namespace", (&profile_key,))?
                .extract::<String>()?
        } else {
            profile_key.clone()
        };
        if text(&plan, "save_subpath") != format!("__ModBox21Saves_{namespace}") {
            return Err(invalid(
                "Owned save namespace changed after runtime review.",
            ));
        }
        for name in ["overwrite", "ini", "saves"] {
            if name == "saves" && !isolate_documents {
                continue;
            }
            crate::snapshot_directory(
                if name == "saves" {
                    saves.clone()
                } else {
                    baseline_profile.join(name)
                },
                directories[name].clone(),
            )?;
        }
        let temp: PathBuf = py
            .import("tempfile")?
            .call_method0("gettempdir")?
            .extract()?;
        let port = plan
            .get("mcp_port")
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u16::try_from(n).ok())
                    .ok_or_else(|| invalid("MCP port must be between 1 and 65535."))
            })
            .transpose()?;
        let lease = crate::McpSession::new(
            temp.join("modbox21-mcp-ports"),
            port,
            plan["disable_mcp"].as_bool().unwrap_or(false),
        )?;
        extra.set_item("mcp_port", lease.port())?;
        extra.set_item("mcp_endpoint", lease.endpoint())?;
        extra.set_item("mcp_disabled", lease.disabled)?;
        port_lease = Some(lease);
        for path in directories.values() {
            reject_links(path)?;
            std::fs::create_dir_all(path)?;
        }
        let baseline_path = baseline_profile.join("ini_baseline.json");
        reject_links(&baseline_path)?;
        let baseline = if baseline_path.is_file() {
            read(&baseline_path)?
        } else {
            json!({})
        };
        let mut new_baseline = json!({});
        for item in items(&plan, "snapshots")? {
            if item[2] == "ini" {
                let name = item[0].as_str().unwrap_or("");
                let text = item[1].as_str().unwrap_or("");
                new_baseline[fold(name)] = json!(format!("{:x}", Sha256::digest(text.as_bytes())));
            }
        }
        for item in items(&plan, "snapshots")? {
            let name = item[0].as_str().unwrap_or("");
            let text = item[1].as_str().unwrap_or("");
            let kind = item[2].as_str().unwrap_or("");
            relative_path(name, false)?;
            if name.contains('/') || !matches!(kind, "plugins" | "ini") {
                return Err(invalid("Unsupported runtime snapshot path."));
            }
            let path = directories[kind].join(name);
            reject_links(&path)?;
            if kind == "plugins"
                || !path.exists()
                || baseline[fold(name)] != new_baseline[fold(name)]
            {
                let temporary = path.with_extension(format!(
                    "{}.new",
                    path.extension().unwrap_or_default().to_string_lossy()
                ));
                reject_links(&temporary)?;
                write(py, &temporary, text, true)?;
                replace(py, &temporary, &path)?;
            }
        }
        write(
            py,
            &profile.join("ini_baseline.json"),
            &new_baseline.to_string(),
            false,
        )?;
        for item in items(&plan, "snapshots")? {
            if item[2] != "ini" {
                continue;
            }
            let name = item[0].as_str().unwrap_or("");
            let path = directories["ini"].join(name);
            let text = py
                .import("creation_lib.mod_manager.mo2")?
                .call_method1("read_text", (py_path(py, &path)?,))?;
            let rendered = py
                .import("creation_lib.mod_manager.profile_state")?
                .call_method1(
                    "fo4_runtime_ini",
                    (
                        name,
                        text,
                        if isolate_documents {
                            namespace.as_str()
                        } else {
                            ""
                        },
                    ),
                )?
                .extract::<String>()?;
            let temporary = path.with_extension(format!(
                "{}.new",
                path.extension().unwrap_or_default().to_string_lossy()
            ));
            reject_links(&temporary)?;
            write(py, &temporary, &rendered, false)?;
            replace(py, &temporary, &path)?;
        }
        for item in items(&plan, "files")? {
            let path = PathBuf::from(item[0].as_str().unwrap_or(""));
            let parent = directories["overwrite"].join(path.parent().unwrap_or(Path::new("")));
            reject_links(&parent)?;
            std::fs::create_dir_all(parent)?;
        }
        for path in directories.values() {
            let tree = py
                .import("creation_lib.mod_manager.paths")?
                .call_method1("tree_files", (py_path(py, path)?,))?;
            for item in tree.try_iter()? {
                item?;
            }
        }
        checkpoint(&cancel)?;
        let factory = if native_factory.is_none() {
            py.import(if project {
                "creation_lib.mod_manager.project_vfs"
            } else {
                "creation_lib.mod_manager.native_runtime"
            })?
            .getattr(if project {
                "NativeFilespace"
            } else {
                "NativeVFS"
            })?
        } else {
            native_factory.clone()
        };
        adapter = Some(factory.call1((bundle, format!("b21_{}", uuid::Uuid::new_v4().simple())))?);
        let active = adapter.as_ref().unwrap();
        if project {
            active.call_method1(
                "configure",
                (
                    py_path(py, session)?,
                    py_path(py, &session.join("filespace.txt"))?,
                    object(py, plan.get("protected_roots").unwrap_or(&json!([])))?,
                ),
            )?;
        }
        let game = PathBuf::from(text(&plan, "game_root"));
        for item in plan
            .get("root_files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            checkpoint(&cancel)?;
            active.call_method1(
                "link_file",
                (
                    item[1].as_str().unwrap_or(""),
                    py_path(py, &game.join(item[0].as_str().unwrap_or("")))?,
                ),
            )?;
        }
        let data = game.join("Data");
        reject_links(&data)?;
        mount(active, &directories["overwrite"], &data)?;
        for (position, item) in items(&plan, "files")?.iter().enumerate() {
            checkpoint(&cancel)?;
            active.call_method1(
                "link_file",
                (
                    item[1].as_str().unwrap_or(""),
                    py_path(py, &data.join(item[0].as_str().unwrap_or("")))?,
                ),
            )?;
            if position % 256 == 0 {
                report(
                    py,
                    &status,
                    &extra,
                    &json!({"state":"mapping","files":position,"total":items(&plan,"files")?.len()}),
                )?;
            }
        }
        mount(active, &directories["overwrite"], &data)?;
        mount(
            active,
            &directories["plugins"],
            &PathBuf::from(text(&plan, "plugin_target")),
        )?;
        if isolate_documents {
            mount(
                active,
                &directories["ini"],
                &PathBuf::from(text(&plan, "ini_target")),
            )?;
            mount(
                active,
                &directories["f4se-logs"],
                &PathBuf::from(text(&plan, "ini_target")).join("F4SE"),
            )?;
        } else {
            for item in items(&plan, "snapshots")? {
                if item[2] == "ini" {
                    let name = item[0].as_str().unwrap_or("");
                    active.call_method1(
                        "link_file",
                        (
                            py_path(py, &directories["ini"].join(name))?,
                            py_path(py, &PathBuf::from(text(&plan, "ini_target")).join(name))?,
                        ),
                    )?;
                }
            }
        }
        if project {
            mount(active, &directories["game-root"], &game)?;
        }
        let save_target = PathBuf::from(text(&plan, "ini_target"))
            .join(relative_path(text(&plan, "save_subpath"), false)?);
        reject_links(&save_target)?;
        if isolate_documents && save_target.exists() {
            return Err(invalid(
                "Reserved virtual save directory exists physically; launch refused.",
            ));
        }
        if isolate_documents {
            mount(active, &directories["saves"], &save_target)?;
        }
        if project {
            mount(active, &directories["temp"], &directories["temp"])?;
        }
        checkpoint(&cancel)?;
        validate(py, &plan)?;
        validate_ini()?;
        if !runtime
            .call_method1("profile_signature", (&metadata, &profile_key))?
            .eq(text(&plan, "profile_signature"))?
        {
            return Err(invalid(
                "Profile changed during native mapping; no process was started.",
            ));
        }
        if target {
            let kwargs = PyDict::new(py);
            kwargs.set_item("owner_root", py_path(py, &root)?)?;
            setup.call_method(
                "validate_launch",
                (metadata.call_method0("target")?,),
                Some(&kwargs),
            )?;
        }
        let mut profile_files = None;
        for profile in metadata.call_method0("profiles")?.try_iter()? {
            let profile = profile?;
            if profile.getattr("id")?.eq(&profile_key)? {
                profile_files = Some(profile.getattr("files")?);
                break;
            }
        }
        let winners = PyDict::new(py);
        for item in items(&plan, "files")? {
            winners.set_item(fold(item[0].as_str().unwrap_or("")), object(py, item)?)?;
        }
        py.import("creation_lib.mod_manager.plugin_state")?
            .call_method1(
                "validate_runtime",
                (
                    profile_files.ok_or_else(|| invalid("Runtime profile is unavailable."))?,
                    winners,
                    text(&plan, "game_root"),
                    py_path(py, &directories["overwrite"])?,
                ),
            )?;
        checkpoint(&cancel)?;
        let callback = Py::new(
            py,
            BeforeResume {
                status: status.clone(),
                cancel: cancel.clone(),
                profile: profile_key.clone(),
                extra: extra.clone().unbind(),
                state: state.clone().unbind(),
            },
        )?;
        let environment = PyDict::new(py);
        environment.set_item("SteamAppId", "377160")?;
        environment.set_item("SteamGameId", "377160")?;
        for (key, value) in port_lease.as_ref().unwrap().environment() {
            environment.set_item(key, value)?;
        }
        if project {
            environment.set_item("TEMP", directories["temp"].to_string_lossy().as_ref())?;
            environment.set_item("TMP", directories["temp"].to_string_lossy().as_ref())?;
        }
        let kwargs = PyDict::new(py);
        kwargs.set_item("before_resume", callback)?;
        kwargs.set_item("environment", environment)?;
        pid = value(&active.call_method(
            "start",
            (
                plan_object.get_item("launch")?.get_item("argv")?,
                text(&plan["launch"], "cwd"),
            ),
            Some(&kwargs),
        )?)?;
        started = true;
        let title = text(&plan["launch"]["executable"], "title");
        report(
            py,
            &status,
            &extra,
            &json!({"state":"running","pid":pid,"process_identity":value(&state.get_item("process_identity")?.unwrap())?,"profile_id":profile_key,"title":title}),
        )?;
        let mut empty = 0;
        while empty < 2 {
            let identities = active.call_method0("running_processes")?;
            let pids = PyList::empty(py);
            for identity in identities.try_iter()? {
                let identity = identity?;
                if !valid_process_identity(&identity)? {
                    return Err(invalid(
                        "Runtime child creation identities could not be verified.",
                    ));
                }
                pids.append(identity.get_item("pid")?)?;
            }
            if pids.is_empty() {
                empty += 1;
            } else {
                empty = 0;
                state.set_item("children", &pids)?;
                state.set_item("child_identities", &identities)?;
                report(
                    py,
                    &status,
                    &extra,
                    &json!({"state":"running","pid":pid,"process_identity":value(&state.get_item("process_identity")?.unwrap())?,"children":value(pids.as_any())?,"child_identities":value(&identities)?,"profile_id":profile_key,"title":title}),
                )?;
            }
            wait.call1((0.2,))?;
        }
        let exit_code = value(&active.call_method0("exit_code")?)?;
        process_failed = exit_code.as_u64().is_some_and(|code| code != 0);
        report(
            py,
            &status,
            &extra,
            &json!({"state":if process_failed { "failed" } else { "completed" },"pid":pid,"process_identity":value(&state.get_item("process_identity")?.unwrap())?,"exit_code":exit_code,"profile_id":profile_key}),
        )?;
        completed = true;
        Ok(())
    })();
    let result = (|| -> PyResult<i32> {
        match result {
            Ok(()) => Ok(i32::from(process_failed)),
            Err(error) => {
                let children = value(&state.get_item("children")?.unwrap())?;
                let cancelled = error.is_instance_of::<crate::OperationCancelled>(py)
                    || error.is_instance(
                        py,
                        &py.import("creation_lib.mod_manager.paths")?
                            .getattr("Cancelled")?,
                    );
                let failed = if started || children.as_array().is_some_and(|v| !v.is_empty()) {
                    "supervision-failed"
                } else if cancelled {
                    "cancelled"
                } else {
                    "failed"
                };
                let unverified = if let Some(adapter) = &adapter {
                    adapter.hasattr("identity_unverified")?
                        && adapter.getattr("identity_unverified")?.is_truthy()?
                } else {
                    false
                };
                report(py,&status,&extra,&json!({"state":failed,"error":error.value(py).str()?.to_str()?,"pid":pid,"process_identity":value(&state.get_item("process_identity")?.unwrap())?,"children":children,"child_identities":value(&state.get_item("child_identities")?.unwrap())?,"identity_unverified":unverified,"profile_id":profile_key})).map(|_|1)
            }
        }
    })();
    let closed = if let Some(adapter) = adapter {
        adapter.call_method0("close").map(|_| ())
    } else {
        Ok(())
    };
    let unlocked = (|| -> PyResult<()> {
        if lock.is_some()
            && (completed || (!started && state.get_item("children")?.unwrap().is_empty()?))
            && active_path.is_file()
        {
            std::fs::remove_file(&active_path)?;
        }
        Ok(())
    })();
    if let Some(mut lease) = port_lease {
        lease.close();
    }
    if let Some(mut lease) = lock {
        lease.close();
    }
    unlocked?;
    closed?;
    result
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(helper_write_status, m)?)?;
    m.add_function(wrap_pyfunction!(helper_validate_files, m)?)?;
    m.add_function(wrap_pyfunction!(helper_run, m)?)?;
    Ok(())
}
