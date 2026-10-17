use crate::{invalid, python_io::with_context};
use pyo3::{prelude::*, types::PyList};

#[pyfunction]
fn browser_protocol_command(py: Python<'_>) -> PyResult<String> {
    let sys = py.import("sys")?;
    let args = PyList::empty(py);
    args.append(sys.getattr("executable")?)?;
    if sys
        .getattr("frozen")
        .is_ok_and(|v| v.is_truthy().unwrap_or(false))
    {
        args.append("--variant=mod_manager")?;
    } else {
        let root = py.import("app.paths")?.call_method0("get_code_root")?;
        args.append(
            root.call_method1("joinpath", ("ui", "mod_manager", "nxm_entry.py"))?
                .str()?,
        )?;
    }
    let command: String = py
        .import("subprocess")?
        .call_method1("list2cmdline", (args,))?
        .extract()?;
    Ok(format!("{command} \"%1\""))
}
fn registry<'py>(value: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    if value.is_none() {
        Ok(value.py().import("winreg")?.into_any())
    } else {
        Ok(value.clone())
    }
}
#[pyfunction]
fn browser_current_handler(value: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = value.py();
    let api = registry(value)?;
    let result = (|| {
        let context = api.call_method1(
            "OpenKey",
            (api.getattr("HKEY_CLASSES_ROOT")?, r"nxm\shell\open\command"),
        )?;
        with_context(&context, |key| {
            Ok(api
                .call_method1("QueryValueEx", (key, ""))?
                .get_item(0)?
                .unbind())
        })
    })();
    match result {
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyFileNotFoundError>(py) => {
            Ok("".into_pyobject(py)?.into_any().unbind())
        }
        result => result,
    }
}
#[pyfunction]
fn browser_register_handler(confirmed: bool, value: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    if !confirmed {
        return Err(invalid(
            "Confirm the browser protocol change in setup before registering it.",
        ));
    }
    let py = value.py();
    let api = registry(value)?;
    let command = browser_protocol_command(py)?;
    let previous = browser_current_handler(&api)?;
    let entries = [
        (
            r"Software\Classes\nxm",
            vec![
                ("", "URL:ModBox21 Nexus download link"),
                ("URL Protocol", ""),
            ],
        ),
        (
            r"Software\Classes\nxm\shell\open\command",
            vec![("", command.as_str())],
        ),
        (
            r"Software\Classes\ModBox21.nxm",
            vec![("", "ModBox21 Nexus download link"), ("URL Protocol", "")],
        ),
        (
            r"Software\Classes\ModBox21.nxm\shell\open\command",
            vec![("", command.as_str())],
        ),
        (
            r"Software\ModBox21\Capabilities",
            vec![("ApplicationName", "ModBox21")],
        ),
        (
            r"Software\ModBox21\Capabilities\URLAssociations",
            vec![("nxm", "ModBox21.nxm")],
        ),
        (
            r"Software\RegisteredApplications",
            vec![("ModBox21", r"Software\ModBox21\Capabilities")],
        ),
    ];
    for (path, values) in entries {
        let context = api.call_method1("CreateKey", (api.getattr("HKEY_CURRENT_USER")?, path))?;
        with_context(&context, |key| {
            for (name, value) in values {
                api.call_method1("SetValueEx", (key, name, 0, api.getattr("REG_SZ")?, value))?;
            }
            Ok(())
        })?;
    }
    Ok(previous)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(browser_protocol_command, m)?)?;
    m.add_function(wrap_pyfunction!(browser_current_handler, m)?)?;
    m.add_function(wrap_pyfunction!(browser_register_handler, m)?)?;
    Ok(())
}
