use crate::{
    nexus::{
        failure, nexus_cdn_url, nexus_download, nexus_http_error, nexus_json_request,
        nexus_json_response, nexus_key,
    },
    plugin_inventory::{object, value},
    python_io::with_context,
};
use pyo3::{
    exceptions::{PyOSError, PyTimeoutError},
    prelude::*,
    types::{PyBytes, PyDict},
};
use std::path::PathBuf;

#[pyfunction]
fn nexus_client_initialize(
    client: &Bound<'_, PyAny>,
    key: &str,
    version: &str,
    transport: &Bound<'_, PyAny>,
) -> PyResult<()> {
    nexus_key(key)?;
    let py = client.py();
    client.setattr("_api_key", key)?;
    client.setattr("version", version)?;
    client.setattr("user", py.None())?;
    client.setattr("_transport", transport)?;
    let redirect = py
        .import("creation_lib.mod_manager.nexus")?
        .getattr("_Redirects")?;
    let build = py.import("urllib.request")?.getattr("build_opener")?;
    client.setattr("_api", build.call1((redirect.call1((false,))?,))?)?;
    client.setattr("_cdn", build.call1((redirect.call1((true,))?,))?)?;
    Ok(())
}
#[pyfunction]
fn nexus_transfer_client(client: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = client.py();
    let options = PyDict::new(py);
    options.set_item("version", client.getattr("version")?)?;
    options.set_item("transport", client.getattr("_transport")?)?;
    let copy = py
        .import("creation_lib.mod_manager.nexus")?
        .getattr("NexusClient")?
        .call((client.getattr("_api_key")?,), Some(&options))?;
    let user = client.getattr("user")?;
    copy.setattr(
        "user",
        if user.is_truthy()? {
            py.import("builtins")?.call_method1("dict", (user,))?
        } else {
            py.None().into_bound(py)
        },
    )?;
    Ok(copy.unbind())
}
#[pyfunction]
fn nexus_redirect(allowed: bool, newurl: &str) -> PyResult<()> {
    if !allowed {
        return Err(failure(
            "Unexpected redirect from the Nexus API; credentials were not forwarded.",
        ));
    }
    nexus_cdn_url(newurl)?;
    Ok(())
}
#[pyfunction]
fn nexus_open(
    client: &Bound<'_, PyAny>,
    request: &Bound<'_, PyAny>,
    api: bool,
    accepted: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = client.py();
    let options = PyDict::new(py);
    options.set_item("timeout", 15)?;
    let transport = client.getattr("_transport")?;
    let result = if transport.is_truthy()? {
        transport.call((request,), Some(&options))
    } else {
        client
            .getattr(if api { "_api" } else { "_cdn" })?
            .getattr("open")?
            .call((request,), Some(&options))
    };
    match result {
        Ok(response) => Ok(response.unbind()),
        Err(error) => {
            let errors = py.import("urllib.error")?;
            if error.matches(py, errors.getattr("HTTPError")?)? {
                let error = error.value(py);
                let code = error.getattr("code")?;
                if !api && accepted.contains(&code)? {
                    return Ok(error.clone().into_any().unbind());
                }
                let headers = error.getattr("headers")?;
                let retry = if headers.is_truthy()? {
                    headers
                        .call_method1("get", ("Retry-After", "3600"))?
                        .extract()?
                } else {
                    "3600".to_string()
                };
                nexus_http_error(code.extract()?, &retry)?;
                Ok(py.None())
            } else if error.matches(py, errors.getattr("URLError")?)?
                || error.is_instance_of::<PyOSError>(py)
                || error.is_instance_of::<PyTimeoutError>(py)
            {
                Err(failure(
                    "Nexus network request failed. Check the connection and retry.",
                ))
            } else {
                Err(error)
            }
        }
    }
}
#[pyfunction]
fn nexus_read(response: &Bound<'_, PyAny>, size: usize) -> PyResult<Py<PyAny>> {
    let py = response.py();
    match response.call_method1("read", (size,)) {
        Ok(data) => Ok(data.unbind()),
        Err(error) => {
            if error.is_instance_of::<PyOSError>(py)
                || error.is_instance_of::<PyTimeoutError>(py)
                || error.matches(py, py.import("urllib.error")?.getattr("URLError")?)?
                || error.matches(py, py.import("http.client")?.getattr("HTTPException")?)?
            {
                Err(failure(
                    "Nexus response was interrupted. Check the connection and retry.",
                ))
            } else {
                Err(error)
            }
        }
    }
}
#[pyfunction]
fn nexus_json(
    client: &Bound<'_, PyAny>,
    endpoint: &str,
    parameters: &Bound<'_, PyAny>,
    method: &str,
    body: &Bound<'_, PyAny>,
    allow_empty: bool,
) -> PyResult<Py<PyAny>> {
    let py = client.py();
    let spec: serde_json::Value = serde_json::from_str(&nexus_json_request(
        &client.getattr("_api_key")?.extract::<String>()?,
        &client.getattr("version")?.extract::<String>()?,
        endpoint,
        &value(parameters)?.to_string(),
        method,
        &value(body)?.to_string(),
    )?)
    .map_err(|e| failure(e.to_string()))?;
    let options = PyDict::new(py);
    options.set_item("headers", object(py, &spec["headers"])?)?;
    options.set_item("method", method)?;
    options.set_item(
        "data",
        if let Some(data) = spec["data"].as_str() {
            PyBytes::new(py, data.as_bytes()).into_any()
        } else {
            py.None().into_bound(py)
        },
    )?;
    let request = py
        .import("urllib.request")?
        .getattr("Request")?
        .call((spec["url"].as_str().unwrap_or(""),), Some(&options))?;
    let context = client.call_method1("_open", (request,))?;
    let (data, status) = with_context(&context, |response| {
        let status = response
            .getattr("status")
            .unwrap_or_else(|_| 200u16.into_pyobject(py).unwrap().into_any())
            .extract::<u16>()?;
        Ok((
            client
                .call_method1("_read", (response, 2 * 1024 * 1024 + 1))?
                .extract::<Vec<u8>>()?,
            status,
        ))
    })?;
    let data = serde_json::from_str(&nexus_json_response(&data, status, allow_empty)?)
        .map_err(|e| failure(e.to_string()))?;
    Ok(object(py, &data)?.unbind())
}
#[pyfunction]
fn nexus_client_download(
    client: &Bound<'_, PyAny>,
    url: &str,
    destination: PathBuf,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
    md5: &str,
) -> PyResult<(u64, String)> {
    nexus_cdn_url(url)?;
    crate::check_cancel(Some(cancel))?;
    let py = client.py();
    let headers = PyDict::new(py);
    headers.set_item(
        "User-Agent",
        format!("ModBox21/{}", client.getattr("version")?.str()?),
    )?;
    let options = PyDict::new(py);
    options.set_item("headers", headers)?;
    let request = py
        .import("urllib.request")?
        .getattr("Request")?
        .call((url,), Some(&options))?;
    let options = PyDict::new(py);
    options.set_item("api", false)?;
    let context = client.getattr("_open")?.call((request,), Some(&options))?;
    with_context(&context, |response| {
        nexus_download(
            response,
            destination,
            md5,
            Some(cancel).filter(|v| !v.is_none()),
            Some(progress).filter(|v| !v.is_none()),
        )
    })
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(nexus_client_initialize, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_transfer_client, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_redirect, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_open, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_read, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_json, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_client_download, m)?)?;
    Ok(())
}
