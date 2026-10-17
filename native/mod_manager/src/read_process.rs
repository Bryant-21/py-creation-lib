use crate::{invalid, python_io::with_context};
use pyo3::{prelude::*, types::PyList};

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
#[pyfunction]
fn reader_initialize(
    reader: &Bound<'_, PyAny>,
    argv: &Bound<'_, PyAny>,
    output: &Bound<'_, PyAny>,
    errors: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = reader.py();
    if !cfg!(windows) {
        return Err(invalid(
            "Supervised metadata inspection currently requires Windows 10 or later.",
        ));
    }
    reader.call_method0("_bind")?;
    for name in ["job", "process", "returncode"] {
        reader.setattr(name, py.None())?;
    }
    let kernel = reader.getattr("kernel")?;
    let ct = py.import("ctypes")?;
    let w = py.import("ctypes.wintypes")?;
    let types = py.import("creation_lib.mod_manager.native_runtime")?;
    let inherited = PyList::empty(py);
    let mut attributes = None;
    let mut initialized = false;
    let result = (|| {
        let job = kernel.call_method1("CreateJobObjectW", (py.None(), py.None()))?;
        reader.setattr("job", &job)?;
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
            return Err(invalid(
                "Could not create the metadata reader's supervised process job.",
            ));
        }
        let empty = py
            .import("builtins")?
            .call_method1("open", (py.import("os")?.getattr("devnull")?, "rb"))?;
        with_context(&empty, |empty| {
            let owner = kernel.call_method0("GetCurrentProcess")?;
            for stream in [empty, output, errors] {
                let duplicate = w.call_method0("HANDLE")?;
                let original = py
                    .import("msvcrt")?
                    .call_method1("get_osfhandle", (stream.call_method0("fileno")?,))?;
                if !kernel
                    .call_method1(
                        "DuplicateHandle",
                        (&owner, original, &owner, byref(&duplicate)?, 0, true, 2),
                    )?
                    .is_truthy()?
                {
                    return Err(invalid(
                        "Could not duplicate a metadata reader standard handle.",
                    ));
                }
                inherited.append(duplicate.getattr("value")?)?;
            }
            Ok(())
        })?;
        let size = ct.call_method0("c_size_t")?;
        kernel.call_method1(
            "InitializeProcThreadAttributeList",
            (py.None(), 2, 0, byref(&size)?),
        )?;
        let attrs = ct.call_method1("create_string_buffer", (size.getattr("value")?,))?;
        attributes = Some(attrs.clone());
        if !kernel
            .call_method1(
                "InitializeProcThreadAttributeList",
                (&attrs, 2, 0, byref(&size)?),
            )?
            .is_truthy()?
        {
            return Err(invalid(
                "Could not initialize metadata reader process attributes.",
            ));
        }
        initialized = true;
        let handles = w
            .getattr("HANDLE")?
            .call_method1("__mul__", (3,))?
            .call(inherited.to_tuple(), None)?;
        let job_handle = w.call_method1("HANDLE", (&job,))?;
        for (key, value) in [(0x20002, &handles), (0x2000D, &job_handle)] {
            if !kernel
                .call_method1(
                    "UpdateProcThreadAttribute",
                    (
                        &attrs,
                        0,
                        key,
                        byref(value)?,
                        sizeof(value)?,
                        py.None(),
                        py.None(),
                    ),
                )?
                .is_truthy()?
            {
                return Err(invalid(
                    "Windows cannot supervise the metadata reader at process creation.",
                ));
            }
        }
        let startup = types.call_method0("StartupInfoEx")?;
        let info = types.call_method0("ProcessInfo")?;
        let base = startup.getattr("startup")?;
        base.setattr("cb", sizeof(&startup)?)?;
        base.setattr("flags", 0x100)?;
        for (index, name) in ["stdin", "stdout", "stderr"].iter().enumerate() {
            base.setattr(*name, inherited.get_item(index)?)?;
        }
        startup.setattr(
            "attributes",
            ct.call_method1("cast", (&attrs, ct.getattr("c_void_p")?))?,
        )?;
        let command = ct.call_method1(
            "create_unicode_buffer",
            (py.import("subprocess")?
                .call_method1("list2cmdline", (argv,))?,),
        )?;
        let startup_ptr = ct.call_method1(
            "cast",
            (
                byref(&startup)?,
                ct.call_method1("POINTER", (types.getattr("StartupInfo")?,))?,
            ),
        )?;
        if !kernel
            .call_method1(
                "CreateProcessW",
                (
                    argv.get_item(0)?,
                    command,
                    py.None(),
                    py.None(),
                    true,
                    0x80000 | 0x8000000,
                    py.None(),
                    py.None(),
                    startup_ptr,
                    byref(&info)?,
                ),
            )?
            .is_truthy()?
        {
            return Err(invalid(format!(
                "Could not start supervised metadata inspection (Windows error {}).",
                ct.call_method0("get_last_error")?.str()?
            )));
        }
        reader.setattr("process", info.getattr("process")?)?;
        kernel.call_method1("CloseHandle", (info.getattr("thread")?,))?;
        Ok(())
    })();
    let result = if result.is_err() {
        match reader_close(reader) {
            Ok(()) => result,
            Err(e) => Err(e),
        }
    } else {
        result
    };
    if initialized {
        kernel.call_method1("DeleteProcThreadAttributeList", (attributes.unwrap(),))?;
    }
    for handle in inherited.iter() {
        kernel.call_method1("CloseHandle", (handle,))?;
    }
    result
}
#[pyfunction]
fn reader_poll(reader: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = reader.py();
    let kernel = reader.getattr("kernel")?;
    let accounting = py
        .import("creation_lib.mod_manager.read_process")?
        .call_method0("Accounting")?;
    if !kernel
        .call_method1(
            "QueryInformationJobObject",
            (
                reader.getattr("job")?,
                1,
                byref(&accounting)?,
                sizeof(&accounting)?,
                py.None(),
            ),
        )?
        .is_truthy()?
    {
        return Err(invalid(
            "Could not query metadata reader process ownership.",
        ));
    }
    if accounting.getattr("active")?.is_truthy()? {
        return Ok(py.None());
    }
    let code = py.import("ctypes.wintypes")?.call_method0("DWORD")?;
    if !kernel
        .call_method1(
            "GetExitCodeProcess",
            (reader.getattr("process")?, byref(&code)?),
        )?
        .is_truthy()?
    {
        return Err(invalid("Could not read metadata reader exit status."));
    }
    let code = code.getattr("value")?;
    reader.setattr("returncode", &code)?;
    Ok(code.unbind())
}
fn cleanup_error(reader: &Bound<'_, PyAny>, message: &str) -> PyResult<()> {
    let error = reader
        .py()
        .import("creation_lib.mod_manager.paths")?
        .getattr("ManagerError")?
        .call1((message,))?;
    error.setattr("reader", reader)?;
    Err(PyErr::from_value(error))
}
#[pyfunction]
fn reader_close(reader: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = reader.py();
    let kernel = reader.getattr("kernel")?;
    let job = reader.getattr("job")?;
    if job.is_truthy()? {
        if !kernel.call_method1("CloseHandle", (job,))?.is_truthy()? {
            return cleanup_error(
                reader,
                "Metadata process job cleanup failed; descendant exit could not be confirmed.",
            );
        }
        reader.setattr("job", py.None())?;
    }
    let process = reader.getattr("process")?;
    if process.is_truthy()? {
        if !kernel
            .call_method1("WaitForSingleObject", (&process, 2000))?
            .eq(0)?
        {
            return cleanup_error(
                reader,
                "Metadata process cleanup could not confirm the root process exited within two seconds.",
            );
        }
        if !kernel
            .call_method1("CloseHandle", (process,))?
            .is_truthy()?
        {
            return cleanup_error(reader, "Metadata process handle cleanup failed after exit.");
        }
        reader.setattr("process", py.None())?;
    }
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(reader_initialize, m)?)?;
    m.add_function(wrap_pyfunction!(reader_poll, m)?)?;
    m.add_function(wrap_pyfunction!(reader_close, m)?)?;
    Ok(())
}
