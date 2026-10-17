use crate::{check_cancel, history_prune::py_path, invalid, python_io::with_context, reject_links};
use pyo3::prelude::*;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
fn executable(which: &Bound<'_, PyAny>, message: &str) -> PyResult<String> {
    for name in ["modkit.exe", "modkit"] {
        let found = which.call1((name,))?;
        if found.is_truthy()? {
            return found.extract();
        }
    }
    Err(invalid(message))
}
fn size(stream: &Bound<'_, PyAny>) -> PyResult<u64> {
    stream
        .py()
        .import("os")?
        .call_method1("fstat", (stream.call_method0("fileno")?,))?
        .getattr("st_size")?
        .extract()
}
fn error_text(stream: &Bound<'_, PyAny>) -> PyResult<String> {
    stream.call_method1("seek", (size(stream)?.saturating_sub(2000),))?;
    let bytes: Vec<u8> = stream.call_method1("read", (2000,))?.extract()?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
#[pyfunction]
fn cli_read_json(
    command: Vec<String>,
    game: &str,
    cancel: &Bound<'_, PyAny>,
    max_bytes: u64,
    timeout: f64,
    which: &Bound<'_, PyAny>,
    factory: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = cancel.py();
    let allowed = (command.len() == 3
        && (command[..2] == ["esp", "header"] || command[..2] == ["archive", "list"]))
        || (command.len() == 4 && command[..3] == ["esp", "masters", "list"]);
    if !allowed {
        return Err(invalid(
            "This reader accepts only supported read-only metadata commands.",
        ));
    }
    let mut argv = vec![
        executable(
            which,
            "The supported modkit CLI is required for metadata inspection.",
        )?,
        "--format".into(),
        "json".into(),
    ];
    if !game.is_empty() {
        argv.extend(["--game".into(), game.into()]);
    }
    argv.extend(command);
    check_cancel(Some(cancel))?;
    let tempfile = py.import("tempfile")?;
    with_context(&tempfile.call_method0("TemporaryFile")?, |output| {
        with_context(&tempfile.call_method0("TemporaryFile")?, |errors| {
            let process = factory.call1((argv, output, errors)).map_err(|e| {
                if e.is_instance_of::<pyo3::exceptions::PyOSError>(py) {
                    invalid(format!("Could not start modkit metadata inspection: {e}"))
                } else {
                    e
                }
            })?;
            let result = (|| -> PyResult<Py<PyAny>> {
                let started = Instant::now();
                while process.call_method0("poll")?.is_none() {
                    check_cancel(Some(cancel))?;
                    if started.elapsed().as_secs_f64() > timeout {
                        return Err(invalid(
                            "modkit metadata inspection exceeded its time limit.",
                        ));
                    }
                    if size(output)? > max_bytes || size(errors)? > 1024 * 1024 {
                        return Err(invalid("modkit metadata output exceeded its safety limit."));
                    }
                    py.detach(|| std::thread::sleep(Duration::from_millis(25)));
                }
                check_cancel(Some(cancel))?;
                if size(output)? > max_bytes || size(errors)? > 1024 * 1024 {
                    return Err(invalid("modkit metadata output exceeded its safety limit."));
                }
                if process.getattr("returncode")?.is_truthy()? {
                    return Err(invalid(format!(
                        "modkit could not inspect this file: {}",
                        error_text(errors)?
                    )));
                }
                output.call_method1("seek", (0,))?;
                let bytes = output.call_method1("read", (max_bytes,))?;
                let parsed = (|| -> PyResult<Py<PyAny>> {
                    let text = bytes.call_method1("decode", ("utf-8-sig",))?;
                    Ok(py.import("json")?.call_method1("loads", (text,))?.unbind())
                })();
                parsed.map_err(|e| {
                    if e.is_instance_of::<pyo3::exceptions::PyValueError>(py) {
                        invalid("modkit returned invalid JSON metadata.")
                    } else {
                        e
                    }
                })
            })();
            process.call_method0("close")?;
            result
        })
    })
}
#[pyfunction]
fn plugin_write_flag(
    source: PathBuf,
    output: PathBuf,
    action: &str,
    cancel: &Bound<'_, PyAny>,
    which: &Bound<'_, PyAny>,
    factory: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = cancel.py();
    if !["esm", "no-esm"].contains(&action) || source == output {
        return Err(invalid(
            "Only staged ESM-flag changes are supported by this writer.",
        ));
    }
    reject_links(&source)?;
    reject_links(&output)?;
    if output.exists() {
        return Err(invalid("Plugin flag stage is occupied."));
    }
    let argv = vec![
        executable(
            which,
            "The supported modkit CLI is required to edit the master flag.",
        )?,
        "--game".into(),
        "fo4".into(),
        "--format".into(),
        "json".into(),
        "esp".into(),
        "header".into(),
        source.to_string_lossy().into_owned(),
        format!("--{action}"),
        "--output".into(),
        output.to_string_lossy().into_owned(),
    ];
    check_cancel(Some(cancel))?;
    let tempfile = py.import("tempfile")?;
    with_context(&tempfile.call_method0("TemporaryFile")?, |stdout| {
        with_context(&tempfile.call_method0("TemporaryFile")?, |stderr| {
            let process = factory.call1((argv, stdout, stderr))?;
            let result = (|| -> PyResult<()> {
                let started = Instant::now();
                while process.call_method0("poll")?.is_none() {
                    check_cancel(Some(cancel))?;
                    if started.elapsed().as_secs_f64() > 60.
                        || size(stdout)? > 2 * 1024 * 1024
                        || size(stderr)? > 2 * 1024 * 1024
                    {
                        return Err(invalid("Plugin flag CLI exceeded its time/output bound."));
                    }
                    py.detach(|| std::thread::sleep(Duration::from_millis(25)));
                }
                check_cancel(Some(cancel))?;
                if size(stdout)? > 2 * 1024 * 1024 || size(stderr)? > 2 * 1024 * 1024 {
                    return Err(invalid("Plugin flag CLI exceeded its time/output bound."));
                }
                if process.getattr("returncode")?.is_truthy()? {
                    return Err(invalid(format!(
                        "Master flag edit failed: {}",
                        error_text(stderr)?
                    )));
                }
                if !py_path(py, &output)?.call_method0("is_file")?.is_truthy()? {
                    return Err(invalid("Master flag CLI produced no staged plugin."));
                }
                Ok(())
            })();
            process.call_method0("close")?;
            result
        })
    })
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(cli_read_json, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_write_flag, m)?)?;
    Ok(())
}
