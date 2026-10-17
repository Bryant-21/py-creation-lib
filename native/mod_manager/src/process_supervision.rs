use crate::{invalid, reject_links};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

fn structures(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("creation_lib.mod_manager.native_runtime")
}
fn byref<'py>(value: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    value.py().import("ctypes")?.call_method1("byref", (value,))
}
fn sizeof(value: &Bound<'_, PyAny>) -> PyResult<usize> {
    value
        .py()
        .import("ctypes")?
        .call_method1("sizeof", (value,))?
        .extract()
}
fn absolute<'py>(value: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    value
        .py()
        .import("pathlib")?
        .getattr("Path")?
        .call1((value,))?
        .call_method0("absolute")
}
fn last_error(py: Python<'_>) -> PyResult<u32> {
    py.import("ctypes")?
        .call_method0("get_last_error")?
        .extract()
}
fn identity<'py>(
    pid: &Bound<'py, PyAny>,
    handle: &Bound<'py, PyAny>,
    kernel: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = pid.py();
    let kwargs = PyDict::new(py);
    kwargs.set_item("kernel", kernel)?;
    py.import("creation_lib.mod_manager.process_identity")?
        .getattr("from_handle")?
        .call((pid, handle), Some(&kwargs))
}
#[pyfunction]
fn vfs_initialize(
    adapter: &Bound<'_, PyAny>,
    root: &Bound<'_, PyAny>,
    session: &str,
    project: bool,
) -> PyResult<()> {
    let py = adapter.py();
    let ct = py.import("ctypes")?;
    if !cfg!(windows) || sizeof(&ct.getattr("c_void_p")?)? != 8 {
        return Err(invalid(if project {
            "Project virtual filespace requires x64 Windows."
        } else {
            "USVFS requires a 64-bit Windows helper."
        }));
    }
    for name in ["job", "process"] {
        adapter.setattr(name, py.None())?;
    }
    for name in ["connected", "identity_unverified"] {
        adapter.setattr(name, false)?;
    }
    adapter.setattr("observed_processes", PyDict::new(py))?;
    if project {
        let root = absolute(root)?;
        let path = root.extract::<PathBuf>()?.join("modbox_vfs.dll");
        reject_links(&path)?;
        adapter.setattr("root", root)?;
        adapter.call_method1("_load", (path.to_string_lossy().as_ref(),))?;
        if adapter
            .getattr("library")?
            .call_method0("MBVFS_Version")?
            .extract::<u32>()?
            != 1
        {
            return Err(invalid("Unsupported project virtual filespace ABI."));
        }
        adapter.setattr("session", session.strip_prefix("b21_").unwrap_or(session))?;
        for name in ["files", "mounts"] {
            adapter.setattr(name, PyDict::new(py))?;
        }
        adapter.setattr("protected", PySet::empty(py)?)?;
        for name in ["owner", "config"] {
            adapter.setattr(name, py.None())?;
        }
        return Ok(());
    }
    let root = py
        .import("creation_lib.mod_manager.runtime_setup")?
        .call_method1("supported_bundle", (root.str()?,))?;
    let path = root.extract::<PathBuf>()?.join("usvfs_x64.dll");
    if let Err(e) = adapter.call_method1("_load", (path.to_string_lossy().as_ref(),)) {
        if e.is_instance_of::<pyo3::exceptions::PyOSError>(py) {
            return Err(invalid(format!(
                "The verified USVFS runtime could not load. Repair the app dependency or Microsoft's x64 Visual C++ v14 prerequisite. {}",
                e.value(py)
            )));
        }
        return Err(e);
    }
    let api = adapter.getattr("api")?;
    let version: String = api
        .call_method0("usvfsVersionString")?
        .call_method1("decode", ("ascii",))?
        .extract()?;
    let expected: String = py
        .import("creation_lib.mod_manager.runtime")?
        .getattr("SUPPORTED_VERSION")?
        .extract()?;
    if version != expected {
        return Err(invalid(format!(
            "Unsupported USVFS version {version}; this adapter was tested with {expected}."
        )));
    }
    let parameters = adapter.getattr("parameters")?;
    let value = parameters.call_method0("usvfsCreateParameters")?;
    if !value.is_truthy()? {
        return Err(invalid("USVFS could not allocate parameters."));
    }
    let result = (|| {
        parameters.call_method1(
            "usvfsSetInstanceName",
            (
                &value,
                session
                    .into_pyobject(py)?
                    .call_method1("encode", ("ascii",))?,
            ),
        )?;
        parameters.call_method1("usvfsSetDebugMode", (&value, false))?;
        parameters.call_method1("usvfsSetLogLevel", (&value, 3))?;
        parameters.call_method1("usvfsSetCrashDumpType", (&value, 0))?;
        if !api.call_method1("usvfsCreateVFS", (&value,))?.is_truthy()? {
            return Err(invalid(
                "USVFS could not create the isolated runtime session.",
            ));
        }
        adapter.setattr("connected", true)?;
        Ok(())
    })();
    parameters.call_method1("usvfsFreeParameters", (value,))?;
    result
}
#[pyfunction]
fn filespace_configure(
    adapter: &Bound<'_, PyAny>,
    owner: &Bound<'_, PyAny>,
    config: &Bound<'_, PyAny>,
    protected: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let owner = absolute(owner)?;
    let config = absolute(config)?;
    reject_links(&owner.extract::<PathBuf>()?)?;
    reject_links(&config.extract::<PathBuf>()?)?;
    adapter.setattr("owner", &owner)?;
    adapter.setattr("config", &config)?;
    if !config
        .call_method1("is_relative_to", (&owner,))?
        .is_truthy()?
    {
        return Err(invalid("Filespace configuration must be manager-owned."));
    }
    for path in protected.try_iter()? {
        let path = path?;
        if path.is_truthy()? {
            adapter
                .getattr("protected")?
                .call_method1("add", (absolute(&path)?.str()?,))?;
        }
    }
    Ok(())
}
#[pyfunction]
fn vfs_link(
    adapter: &Bound<'_, PyAny>,
    source: &Bound<'_, PyAny>,
    target: &Bound<'_, PyAny>,
    project: bool,
    directory: bool,
    create: bool,
) -> PyResult<()> {
    if !project {
        let api = adapter.getattr("api")?;
        let method = if directory {
            "usvfsVirtualLinkDirectoryStatic"
        } else {
            "usvfsVirtualLinkFile"
        };
        let flags = if directory {
            8 | if create { 4 } else { 0 }
        } else {
            0
        };
        if !api
            .call_method1(method, (source.str()?, target.str()?, flags))?
            .is_truthy()?
        {
            return Err(invalid(format!(
                "USVFS refused {} mapping: {}",
                if directory { "directory" } else { "file" },
                target.str()?
            )));
        }
        return Ok(());
    }
    if directory && !create {
        return Err(invalid(
            "Filespace directory mounts currently require owned output.",
        ));
    }
    let source = absolute(source)?;
    let target = absolute(target)?;
    reject_links(&source.extract::<PathBuf>()?)?;
    reject_links(&target.extract::<PathBuf>()?)?;
    let owner = adapter.getattr("owner")?;
    if directory {
        if owner.is_none()
            || !source
                .call_method1("is_relative_to", (&owner,))?
                .is_truthy()?
        {
            return Err(invalid(
                "Virtual output must reside inside the manager owner.",
            ));
        }
        adapter
            .getattr("mounts")?
            .set_item(target.str()?, source.str()?)?;
    } else {
        adapter
            .getattr("files")?
            .set_item(target.str()?, source.str()?)?;
        if owner.is_none()
            || !source
                .call_method1(
                    "is_relative_to",
                    (owner.call_method1("joinpath", ("runtime",))?,),
                )?
                .is_truthy()?
        {
            adapter
                .getattr("protected")?
                .call_method1("add", (source.getattr("parent")?.str()?,))?;
        }
    }
    Ok(())
}
#[pyfunction]
fn filespace_write_config(adapter: &Bound<'_, PyAny>) -> PyResult<()> {
    let owner = adapter.getattr("owner")?;
    let config = adapter.getattr("config")?;
    if owner.is_none() || config.is_none() {
        return Err(invalid("Configure filespace ownership before starting it."));
    }
    let session: String = adapter.getattr("session")?.extract()?;
    let mut lines = vec![
        "MBVFS1".into(),
        format!("session\t{session}"),
        format!("owner\t{}", owner.str()?),
    ];
    let mut protected: Vec<String> = adapter
        .getattr("protected")?
        .try_iter()?
        .map(|p| p?.extract())
        .collect::<PyResult<_>>()?;
    protected.sort();
    let valid = |s: &str| -> PyResult<()> {
        if s.contains(['\n', '\r', '\t', '\0']) {
            Err(invalid(
                "Filespace paths cannot contain control characters.",
            ))
        } else {
            Ok(())
        }
    };
    valid(&session)?;
    valid(&owner.str()?.extract::<String>()?)?;
    for path in protected {
        valid(&path)?;
        lines.push(format!("protect\t{path}"));
    }
    for (attr, prefix) in [("files", "file"), ("mounts", "mount")] {
        for pair in adapter.getattr(attr)?.call_method0("items")?.try_iter()? {
            let pair = pair?;
            let target: String = pair.get_item(0)?.extract()?;
            let source: String = pair.get_item(1)?.extract()?;
            valid(&target)?;
            valid(&source)?;
            lines.push(format!("{prefix}\t{target}\t{source}"));
        }
    }
    let text = lines.join("\n") + "\n";
    let content: Vec<_> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    if content.len() > 100 * 1024 * 1024 {
        return Err(invalid("Filespace mapping exceeds its bound."));
    }
    let path: PathBuf = config.extract()?;
    reject_links(&path)?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(path, content)?;
    Ok(())
}
fn environment_block<'py>(
    py: Python<'py>,
    overrides: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let mut values: BTreeMap<String, String> = std::env::vars_os()
        .map(|(k, v)| {
            (
                k.to_string_lossy().to_uppercase(),
                v.to_string_lossy().into_owned(),
            )
        })
        .collect();
    if !overrides.is_none() {
        for pair in overrides.call_method0("items")?.try_iter()? {
            let pair = pair?;
            let key: String = pair.get_item(0)?.extract()?;
            values.insert(key.to_uppercase(), pair.get_item(1)?.extract()?);
        }
    }
    let text = values
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\0")
        + "\0\0";
    py.import("ctypes")?
        .call_method1("create_unicode_buffer", (text,))
}
#[pyfunction]
fn vfs_start(
    adapter: &Bound<'_, PyAny>,
    argv: &Bound<'_, PyAny>,
    cwd: &Bound<'_, PyAny>,
    before_resume: &Bound<'_, PyAny>,
    environment: &Bound<'_, PyAny>,
    project: bool,
) -> PyResult<Py<PyAny>> {
    let py = adapter.py();
    let ct = py.import("ctypes")?;
    let w = py.import("ctypes.wintypes")?;
    let types = structures(py)?;
    if project {
        adapter.call_method0("_write_config")?;
    }
    let kernel = adapter.getattr("kernel")?;
    let job = kernel.call_method1("CreateJobObjectW", (py.None(), py.None()))?;
    adapter.setattr("job", &job)?;
    let limit = types.call_method0("JobExtended")?;
    limit.getattr("basic")?.setattr("flags", 0x2000)?;
    if !job.is_truthy()?
        || !kernel
            .call_method1(
                "SetInformationJobObject",
                (&job, 9, byref(&limit)?, sizeof(&limit)?),
            )?
            .is_truthy()?
    {
        return Err(invalid(if project {
            "Could not create the supervised filespace job."
        } else {
            "Could not create the supervised runtime process job."
        }));
    }
    let size = ct.call_method0("c_size_t")?;
    kernel.call_method1(
        "InitializeProcThreadAttributeList",
        (py.None(), 1, 0, byref(&size)?),
    )?;
    let attributes = ct.call_method1("create_string_buffer", (size.getattr("value")?,))?;
    if !kernel
        .call_method1(
            "InitializeProcThreadAttributeList",
            (&attributes, 1, 0, byref(&size)?),
        )?
        .is_truthy()?
    {
        return Err(invalid(if project {
            "Windows could not initialize filespace process attributes."
        } else {
            "Windows could not initialize runtime process attributes."
        }));
    }
    let result = (|| {
        let info = types.call_method0("ProcessInfo")?;
        let startup = types.call_method0("StartupInfoEx")?;
        startup
            .getattr("startup")?
            .setattr("cb", sizeof(&startup)?)?;
        startup.setattr(
            "attributes",
            ct.call_method1("cast", (&attributes, ct.getattr("c_void_p")?))?,
        )?;
        let job_handle = w.call_method1("HANDLE", (&job,))?;
        let command = ct.call_method1(
            "create_unicode_buffer",
            (py.import("subprocess")?
                .call_method1("list2cmdline", (argv,))?,),
        )?;
        let environment = if project || environment.is_truthy()? {
            environment_block(py, environment)?
        } else {
            py.None().into_bound(py)
        };
        if !kernel
            .call_method1(
                "UpdateProcThreadAttribute",
                (
                    &attributes,
                    0,
                    0x2000D,
                    byref(&job_handle)?,
                    sizeof(&job_handle)?,
                    py.None(),
                    py.None(),
                ),
            )?
            .is_truthy()?
        {
            return Err(invalid(if project {
                "Windows cannot attach the filespace job at creation."
            } else {
                "Windows cannot attach the runtime job at process creation."
            }));
        }
        let flags = 4
            | 0x80000
            | if environment.is_none() { 0 } else { 0x400 }
            | if project { 0x08000000 } else { 0 };
        let api = if project {
            kernel.clone()
        } else {
            adapter.getattr("api")?
        };
        let operation = if project {
            "CreateProcessW"
        } else {
            "usvfsCreateProcessHooked"
        };
        let startup_ptr = ct.call_method1(
            "cast",
            (
                byref(&startup)?,
                ct.call_method1("POINTER", (types.getattr("StartupInfo")?,))?,
            ),
        )?;
        if !api
            .call_method1(
                operation,
                (
                    argv.get_item(0)?,
                    command,
                    py.None(),
                    py.None(),
                    false,
                    flags,
                    environment,
                    cwd,
                    startup_ptr,
                    byref(&info)?,
                ),
            )?
            .is_truthy()?
        {
            if !project {
                for name in ["thread", "process"] {
                    let handle = info.getattr(name)?;
                    if handle.is_truthy()? {
                        kernel.call_method1("CloseHandle", (handle,))?;
                    }
                }
            }
            return Err(invalid(if project {
                format!(
                    "Filespace process creation failed (Windows {}).",
                    last_error(py)?
                )
            } else {
                format!(
                    "USVFS process creation/injection failed (Windows error {}). No direct-launch fallback was attempted.",
                    last_error(py)?
                )
            }));
        }
        adapter.setattr("process", &info)?;
        let result = (|| {
            let pid = info.getattr("pid")?;
            let token = match identity(&pid, &info.getattr("process")?, &kernel) {
                Ok(token) => token,
                Err(e) => {
                    if !project && e.is_instance_of::<pyo3::exceptions::PyValueError>(py) {
                        adapter.setattr("identity_unverified", true)?;
                        if before_resume.is_truthy()? {
                            before_resume.call1((&pid, py.None()))?;
                        }
                    }
                    return Err(e);
                }
            };
            if before_resume.is_truthy()? {
                before_resume.call1((&pid, token))?;
            }
            if project
                && !adapter
                    .getattr("library")?
                    .call_method1(
                        "MBVFS_Inject",
                        (info.getattr("process")?, adapter.getattr("config")?.str()?),
                    )?
                    .is_truthy()?
            {
                return Err(invalid(format!(
                    "Project filespace injection/initialization failed (Windows {}); no unhooked fallback.",
                    last_error(py)?
                )));
            }
            if kernel
                .call_method1("ResumeThread", (info.getattr("thread")?,))?
                .extract::<u64>()?
                == 0xFFFFFFFF
            {
                return Err(invalid(if project {
                    "Filespace process could not be resumed."
                } else {
                    "Injected runtime process could not be resumed."
                }));
            }
            Ok(pid.unbind())
        })();
        kernel.call_method1("CloseHandle", (info.getattr("thread")?,))?;
        result
    })();
    kernel.call_method1("DeleteProcThreadAttributeList", (attributes,))?;
    result
}
#[pyfunction]
fn vfs_running(adapter: &Bound<'_, PyAny>, project: bool) -> PyResult<Py<PyAny>> {
    let py = adapter.py();
    let result = running(adapter, project);
    if let Err(e) = &result {
        if e.is_instance_of::<pyo3::exceptions::PyValueError>(py) {
            adapter.setattr("identity_unverified", true)?;
        }
    }
    result
}
fn running(adapter: &Bound<'_, PyAny>, project: bool) -> PyResult<Py<PyAny>> {
    let py = adapter.py();
    let ct = py.import("ctypes")?;
    let w = py.import("ctypes.wintypes")?;
    let kernel = adapter.getattr("kernel")?;
    let job = adapter.getattr("job")?;
    let mut pids = BTreeSet::new();
    if !project {
        let count = ct.call_method1("c_size_t", (4096,))?;
        let buffer = w
            .getattr("DWORD")?
            .call_method1("__mul__", (4096,))?
            .call0()?;
        if !adapter
            .getattr("api")?
            .call_method1("usvfsGetVFSProcessList", (byref(&count)?, &buffer))?
            .is_truthy()?
        {
            return Err(invalid(
                "Could not verify the USVFS process list; runtime supervision failed.",
            ));
        }
        let count: usize = count.getattr("value")?.extract()?;
        if count > 4096 {
            return Err(invalid(
                "USVFS process list exceeded the supervision limit.",
            ));
        }
        for i in 0..count {
            pids.insert(buffer.get_item(i)?.extract::<u32>()?);
        }
    }
    let buffer = ct.call_method1(
        "create_string_buffer",
        (8 + 4096 * sizeof(&ct.getattr("c_size_t")?)?,),
    )?;
    let returned = w.call_method0("DWORD")?;
    if !kernel
        .call_method1(
            "QueryInformationJobObject",
            (&job, 3, &buffer, sizeof(&buffer)?, byref(&returned)?),
        )?
        .is_truthy()?
    {
        return Err(invalid(if project {
            "Could not enumerate the filespace job."
        } else {
            "Windows could not enumerate the supervised runtime job."
        }));
    }
    let header = w
        .getattr("DWORD")?
        .call_method1("__mul__", (2,))?
        .call_method1("from_buffer", (&buffer,))?;
    let assigned: usize = header.get_item(0)?.extract()?;
    let size: usize = header.get_item(1)?.extract()?;
    if size > 4096 || assigned > size {
        return Err(invalid(if project {
            "Filespace descendants exceed the supervision bound."
        } else {
            "Runtime child process count exceeds its safety limit."
        }));
    }
    let children = ct
        .getattr("c_size_t")?
        .call_method1("__mul__", (size,))?
        .call_method1("from_buffer", (&buffer, 8))?;
    for i in 0..size {
        pids.insert(children.get_item(i)?.extract::<u32>()?);
    }
    let own = std::process::id();
    let result = PyList::empty(py);
    let observed = adapter.getattr("observed_processes")?;
    for pid in pids {
        if !project && pid == own {
            continue;
        }
        let handle = kernel.call_method1("OpenProcess", (0x1000 | 0x100000, false, pid))?;
        if !handle.is_truthy()? {
            if last_error(py)? == 87 {
                continue;
            }
            return Err(invalid(if project {
                "Could not inspect an owned filespace process."
            } else {
                "Windows could not inspect a registered runtime child process."
            }));
        }
        let mut retained = false;
        let check = (|| {
            let state: u64 = kernel
                .call_method1("WaitForSingleObject", (&handle, 0))?
                .extract()?;
            if state == 0xFFFFFFFF {
                return Err(invalid("Windows could not verify a runtime child process."));
            }
            if state == 258 {
                let member = w.call_method0("BOOL")?;
                if !kernel
                    .call_method1("IsProcessInJob", (&handle, &job, byref(&member)?))?
                    .is_truthy()?
                {
                    return Err(invalid(
                        "Windows could not verify runtime child job ownership.",
                    ));
                }
                if member.getattr("value")?.is_truthy()? {
                    let token = identity(pid.into_pyobject(py)?.as_any(), &handle, &kernel)?;
                    let key = (pid, token.get_item("created")?.extract::<String>()?);
                    if !observed.contains(&key)? {
                        if observed.len()? >= 4096 {
                            return Err(invalid(
                                "Runtime process history exceeds the supervision bound.",
                            ));
                        }
                        observed.set_item(key, &handle)?;
                        retained = true;
                    }
                    result.append(token)?;
                }
            }
            Ok(())
        })();
        if !retained {
            kernel.call_method1("CloseHandle", (handle,))?;
        }
        check?;
    }
    let process = adapter.getattr("process")?;
    if !project && process.is_truthy()? {
        let state: u64 = kernel
            .call_method1("WaitForSingleObject", (process.getattr("process")?, 0))?
            .extract()?;
        if state == 0xFFFFFFFF {
            return Err(invalid(
                "Windows could not verify the retained runtime process handle.",
            ));
        }
        let pid = process.getattr("pid")?;
        let mut found = false;
        for item in result.iter() {
            if item.get_item("pid")?.eq(&pid)? {
                found = true;
                break;
            }
        }
        if state == 258 && !found {
            result.append(identity(&pid, &process.getattr("process")?, &kernel)?)?;
        }
    }
    Ok(result.into_any().unbind())
}
#[pyfunction]
fn vfs_exit_code(adapter: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = adapter.py();
    let value = py.import("ctypes.wintypes")?.call_method0("DWORD")?;
    let kernel = adapter.getattr("kernel")?;
    for handle in adapter
        .getattr("observed_processes")?
        .call_method0("values")?
        .try_iter()?
    {
        if !kernel
            .call_method1("GetExitCodeProcess", (handle?, byref(&value)?))?
            .is_truthy()?
        {
            return Err(invalid(
                "Could not read a retained runtime child exit code.",
            ));
        }
        if value.getattr("value")?.extract::<u32>()? != 0 {
            return Ok(value.getattr("value")?.unbind());
        }
    }
    let process = adapter.getattr("process")?;
    if process.is_truthy()?
        && adapter
            .getattr("kernel")?
            .call_method1(
                "GetExitCodeProcess",
                (process.getattr("process")?, byref(&value)?),
            )?
            .is_truthy()?
    {
        Ok(value.getattr("value")?.unbind())
    } else {
        Ok(py.None())
    }
}
#[pyfunction]
fn vfs_close(adapter: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = adapter.py();
    let kernel = adapter.getattr("kernel")?;
    let observed = adapter.getattr("observed_processes")?;
    for handle in observed.call_method0("values")?.try_iter()? {
        kernel.call_method1("CloseHandle", (handle?,))?;
    }
    observed.call_method0("clear")?;
    let job = adapter.getattr("job")?;
    if job.is_truthy()? {
        kernel.call_method1("CloseHandle", (job,))?;
        adapter.setattr("job", py.None())?;
    }
    let process = adapter.getattr("process")?;
    if process.is_truthy()? {
        kernel.call_method1("CloseHandle", (process.getattr("process")?,))?;
        adapter.setattr("process", py.None())?;
    }
    if adapter.getattr("connected")?.is_truthy()? {
        adapter.getattr("api")?.call_method0("usvfsDisconnectVFS")?;
        adapter.setattr("connected", false)?;
    }
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(vfs_initialize, m)?)?;
    m.add_function(wrap_pyfunction!(filespace_configure, m)?)?;
    m.add_function(wrap_pyfunction!(vfs_link, m)?)?;
    m.add_function(wrap_pyfunction!(filespace_write_config, m)?)?;
    m.add_function(wrap_pyfunction!(vfs_start, m)?)?;
    m.add_function(wrap_pyfunction!(vfs_running, m)?)?;
    m.add_function(wrap_pyfunction!(vfs_exit_code, m)?)?;
    m.add_function(wrap_pyfunction!(vfs_close, m)?)?;
    Ok(())
}
