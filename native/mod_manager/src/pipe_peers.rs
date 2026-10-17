use crate::invalid;
use pyo3::prelude::*;
fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("creation_lib.mod_manager.pipe_peers")
}
fn sid(handle: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = handle.py();
    let api = module(py)?.call_method0("_apis")?;
    let kernel = api.get_item(0)?;
    let security = api.get_item(1)?;
    let ct = py.import("ctypes")?;
    let w = py.import("ctypes.wintypes")?;
    let token = w.call_method0("HANDLE")?;
    let size = w.call_method0("DWORD")?;
    if !security
        .call_method1(
            "OpenProcessToken",
            (handle, 8, ct.call_method1("byref", (&token,))?),
        )?
        .is_truthy()?
    {
        return Err(invalid("Browser IPC peer account could not be verified."));
    }
    let result = (|| {
        security.call_method1(
            "GetTokenInformation",
            (&token, 1, py.None(), 0, ct.call_method1("byref", (&size,))?),
        )?;
        let length: usize = size.getattr("value")?.extract()?;
        if length == 0 || length > 65536 {
            return Err(invalid("Browser IPC peer identity is unavailable."));
        }
        let buffer = ct.call_method1("create_string_buffer", (length,))?;
        if !security
            .call_method1(
                "GetTokenInformation",
                (
                    &token,
                    1,
                    &buffer,
                    &size,
                    ct.call_method1("byref", (&size,))?,
                ),
            )?
            .is_truthy()?
        {
            return Err(invalid("Browser IPC peer identity is unavailable."));
        }
        let pointer = ct.call_method1("POINTER", (ct.getattr("c_void_p")?,))?;
        let sid = ct.call_method1("cast", (buffer, pointer))?.get_item(0)?;
        let text = w.call_method0("LPWSTR")?;
        if !security
            .call_method1(
                "ConvertSidToStringSidW",
                (sid, ct.call_method1("byref", (&text,))?),
            )?
            .is_truthy()?
        {
            return Err(invalid("Browser IPC peer account is unavailable."));
        }
        let result = text.getattr("value").map(Bound::unbind);
        kernel.call_method1(
            "LocalFree",
            (ct.call_method1("cast", (text, ct.getattr("c_void_p")?))?,),
        )?;
        result
    })();
    kernel.call_method1("CloseHandle", (token,))?;
    result
}
#[pyfunction]
fn pipe_current_sid(py: Python<'_>) -> PyResult<Py<PyAny>> {
    let kernel = py
        .import("creation_lib.mod_manager.process_identity")?
        .call_method0("kernel_api")?;
    sid(&kernel.call_method0("GetCurrentProcess")?)
}
#[pyfunction]
fn pipe_process(py: Python<'_>, pid: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let identity = py.import("creation_lib.mod_manager.process_identity")?;
    let kernel = identity.call_method0("kernel_api")?;
    let handle = kernel.call_method1("OpenProcess", (0x1000, false, pid))?;
    if !handle.is_truthy()? {
        return Err(invalid("Browser destination process is unavailable."));
    }
    let result = (|| {
        if !sid(&handle)?
            .bind(py)
            .eq(module(py)?.call_method0("_current_sid")?)?
        {
            return Err(invalid(
                "Browser IPC peer belongs to another Windows account.",
            ));
        }
        let kwargs = pyo3::types::PyDict::new(py);
        kwargs.set_item("kernel", &kernel)?;
        Ok(identity
            .getattr("from_handle")?
            .call((pid, &handle), Some(&kwargs))?
            .unbind())
    })();
    kernel.call_method1("CloseHandle", (handle,))?;
    result
}
#[pyfunction]
fn pipe_peer(connection: &Bound<'_, PyAny>, server: bool) -> PyResult<Py<PyAny>> {
    let py = connection.py();
    let kernel = module(py)?.call_method0("_apis")?.get_item(0)?;
    let pid = py.import("ctypes.wintypes")?.call_method0("ULONG")?;
    let operation = if server {
        "GetNamedPipeServerProcessId"
    } else {
        "GetNamedPipeClientProcessId"
    };
    if !kernel
        .call_method1(
            operation,
            (
                connection.call_method0("fileno")?,
                py.import("ctypes")?.call_method1("byref", (&pid,))?,
            ),
        )?
        .is_truthy()?
    {
        return Err(invalid(
            "Browser IPC endpoint identity could not be verified.",
        ));
    }
    pipe_process(py, &pid.getattr("value")?)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(pipe_current_sid, m)?)?;
    m.add_function(wrap_pyfunction!(pipe_process, m)?)?;
    m.add_function(wrap_pyfunction!(pipe_peer, m)?)?;
    Ok(())
}
