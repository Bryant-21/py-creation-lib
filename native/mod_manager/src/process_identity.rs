use crate::invalid;
use pyo3::{
    prelude::*,
    types::{PyDict, PyInt, PyList, PyString},
};
use std::collections::{BTreeMap, HashSet};
fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("creation_lib.mod_manager.process_identity")
}
#[pyfunction]
pub fn valid_process_identity(value: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = value.py();
    if !value.is_instance_of::<PyDict>()
        || value.len()? != 2
        || !value.contains("pid")?
        || !value.contains("created")?
    {
        return Ok(false);
    }
    let pid = value.get_item("pid")?;
    let created = value.get_item("created")?;
    if !pid.get_type().is(&py.get_type::<PyInt>()) || !created.is_instance_of::<PyString>() {
        return Ok(false);
    }
    let Ok(pid) = pid.extract::<u32>() else {
        return Ok(false);
    };
    if pid == 0 {
        return Ok(false);
    }
    let created: String = created.extract()?;
    Ok(!created.is_empty()
        && created.len() <= 20
        && !created.starts_with('0')
        && created.bytes().all(|c| c.is_ascii_digit())
        && created.parse::<u64>().is_ok())
}
fn api<'py>(py: Python<'py>, kernel: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    if kernel.is_truthy()? {
        Ok(kernel.clone())
    } else {
        module(py)?.call_method0("kernel_api")
    }
}
#[pyfunction]
fn process_from_handle(
    pid: &Bound<'_, PyAny>,
    handle: &Bound<'_, PyAny>,
    kernel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = pid.py();
    let api = api(py, kernel)?;
    if !pid.get_type().is(&py.get_type::<PyInt>())
        || pid.extract::<i64>().is_err()
        || pid.extract::<i64>()? <= 0
        || !api.call_method1("GetProcessId", (handle,))?.eq(pid)?
    {
        return Err(invalid(
            "Runtime process handle identity could not be verified.",
        ));
    }
    let ct = py.import("ctypes")?;
    let ft = py.import("ctypes.wintypes")?.getattr("FILETIME")?;
    let created = ft.call0()?;
    let exited = ft.call0()?;
    let used_kernel = ft.call0()?;
    let used_user = ft.call0()?;
    if !api
        .call_method1(
            "GetProcessTimes",
            (
                handle,
                ct.call_method1("byref", (&created,))?,
                ct.call_method1("byref", (exited,))?,
                ct.call_method1("byref", (used_kernel,))?,
                ct.call_method1("byref", (used_user,))?,
            ),
        )?
        .is_truthy()?
    {
        return Err(invalid(
            "Runtime process creation time could not be verified.",
        ));
    }
    let value = (created.getattr("dwHighDateTime")?.extract::<u64>()? << 32)
        | created.getattr("dwLowDateTime")?.extract::<u64>()?;
    if value == 0 {
        return Err(invalid("Runtime process creation time is invalid."));
    }
    let result = PyDict::new(py);
    result.set_item("pid", pid)?;
    result.set_item("created", value.to_string())?;
    Ok(result.into_any().unbind())
}
#[pyfunction]
fn current_process_identity(py: Python<'_>) -> PyResult<Py<PyAny>> {
    let api = module(py)?.call_method0("kernel_api")?;
    process_from_handle(
        &py.import("os")?.call_method0("getpid")?,
        &api.call_method0("GetCurrentProcess")?,
        &api,
    )
}
#[pyfunction]
fn process_identity_state(
    identity: &Bound<'_, PyAny>,
    kernel: &Bound<'_, PyAny>,
) -> PyResult<String> {
    if !valid_process_identity(identity)? {
        return Ok("unverified".into());
    }
    let py = identity.py();
    let api = api(py, kernel)?;
    let handle = api.call_method1("OpenProcess", (0x101000, false, identity.get_item("pid")?))?;
    if !handle.is_truthy()? {
        return Ok(if py
            .import("ctypes")?
            .call_method0("get_last_error")?
            .extract::<u32>()?
            == 87
        {
            "exited"
        } else {
            "unverified"
        }
        .into());
    }
    let result = (|| -> PyResult<String> {
        let actual = match process_from_handle(&identity.get_item("pid")?, &handle, &api) {
            Ok(v) => v,
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyValueError>(py) => {
                return Ok("unverified".into());
            }
            Err(e) => return Err(e),
        };
        if !actual.bind(py).eq(identity)? {
            return Ok("reused".into());
        }
        let state = api
            .call_method1("WaitForSingleObject", (&handle, 0))?
            .extract::<u32>()?;
        Ok(match state {
            258 => "alive",
            0 => "exited",
            _ => "unverified",
        }
        .into())
    })();
    api.call_method1("CloseHandle", (handle,))?;
    result
}
#[pyfunction]
fn session_process_liveness(status: &Bound<'_, PyAny>) -> PyResult<String> {
    let py = status.py();
    if status
        .call_method1("get", ("identity_unverified",))?
        .is_truthy()?
    {
        return Ok("unverified".into());
    }
    let mut pairs = vec![
        (
            status.call_method1("get", ("helper_pid",))?,
            status.call_method1("get", ("helper_identity",))?,
        ),
        (
            status.call_method1("get", ("pid",))?,
            status.call_method1("get", ("process_identity",))?,
        ),
    ];
    let children = status.call_method1("get", ("children", PyList::empty(py)))?;
    let identities = status.call_method1("get", ("child_identities", PyList::empty(py)))?;
    if !children.is_instance_of::<PyList>()
        || !identities.is_instance_of::<PyList>()
        || children.len()? != identities.len()?
        || children.len()? > 4096
    {
        return Ok("unverified".into());
    }
    let mut pids = HashSet::new();
    for pid in children.try_iter()? {
        let pid = pid?;
        if !pid.get_type().is(&py.get_type::<PyInt>()) {
            return Ok("unverified".into());
        }
        let Ok(pid) = pid.extract::<u32>() else {
            return Ok("unverified".into());
        };
        if pid == 0 || !pids.insert(pid) {
            return Ok("unverified".into());
        }
    }
    let mut mapped = BTreeMap::new();
    for identity in identities.try_iter()? {
        let identity = identity?;
        if !valid_process_identity(&identity)? {
            return Ok("unverified".into());
        }
        let pid = identity.get_item("pid")?.extract::<u32>()?;
        if mapped.insert(pid, identity).is_some() || !pids.contains(&pid) {
            return Ok("unverified".into());
        }
    }
    for pid in children.try_iter()? {
        let pid = pid?;
        pairs.push((pid.clone(), mapped[&pid.extract::<u32>()?].clone()));
    }
    let mut states = Vec::new();
    for (pid, identity) in pairs {
        if pid.is_none() {
            continue;
        }
        if !valid_process_identity(&identity)? || !identity.get_item("pid")?.eq(&pid)? {
            states.push("unverified".to_owned());
        } else {
            states.push(
                module(py)?
                    .call_method1("process_state", (identity,))?
                    .extract::<String>()?,
            );
        }
    }
    Ok(
        if states.is_empty() || states.iter().any(|v| v == "unverified") {
            "unverified"
        } else if states.iter().any(|v| v == "alive") {
            "alive"
        } else {
            "exited"
        }
        .into(),
    )
}
#[pyfunction]
fn terminate_owned_process(
    process: &Bound<'_, PyAny>,
    identity: &Bound<'_, PyAny>,
) -> PyResult<bool> {
    let py = process.py();
    if !process.call_method0("poll")?.is_none() {
        return Ok(false);
    }
    let handle = if process.hasattr("_handle")? {
        process.getattr("_handle")?
    } else {
        py.None().into_bound(py)
    };
    if handle.is_none()
        || !valid_process_identity(identity)?
        || !module(py)?
            .call_method1("from_handle", (process.getattr("pid")?, handle))?
            .eq(identity)?
    {
        return Err(invalid(
            "Cannot verify the spawned helper handle; termination refused. No numeric PID was terminated.",
        ));
    }
    process.call_method0("terminate")?;
    Ok(true)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(valid_process_identity, m)?)?;
    m.add_function(wrap_pyfunction!(process_from_handle, m)?)?;
    m.add_function(wrap_pyfunction!(current_process_identity, m)?)?;
    m.add_function(wrap_pyfunction!(process_identity_state, m)?)?;
    m.add_function(wrap_pyfunction!(session_process_liveness, m)?)?;
    m.add_function(wrap_pyfunction!(terminate_owned_process, m)?)?;
    Ok(())
}
