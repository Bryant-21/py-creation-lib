use crate::{check_cancel, invalid};
use pyo3::prelude::*;

#[pyfunction]
fn nexus_credential(action: &str, target: &str, secret: &str) -> PyResult<String> {
    if !target.starts_with("ModBox21/Nexus/") || target.len() > 200 || target.contains('\0') {
        return Err(invalid("Invalid ModBox21 credential target."));
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::{ERROR_NOT_FOUND, GetLastError},
            Security::Credentials::*,
        };
        let mut target: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
        match action {
            "read" => unsafe {
                let mut credential = std::ptr::null_mut();
                if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) == 0 {
                    return if GetLastError() == ERROR_NOT_FOUND {
                        Ok(String::new())
                    } else {
                        Err(invalid(
                            "Windows Credential Manager could not read the Nexus credential.",
                        ))
                    };
                }
                let result = if (*credential).CredentialBlobSize > 4096 {
                    Err(invalid(
                        "Stored Nexus credential exceeds the supported limit.",
                    ))
                } else if (*credential).CredentialBlobSize == 0 {
                    Ok(String::new())
                } else {
                    String::from_utf8(
                        std::slice::from_raw_parts(
                            (*credential).CredentialBlob,
                            (*credential).CredentialBlobSize as usize,
                        )
                        .to_vec(),
                    )
                    .map_err(|_| invalid("Stored Nexus credential is invalid."))
                };
                CredFree(credential.cast());
                result
            },
            "write" => {
                if secret.is_empty() || secret.len() > 4096 || secret.chars().any(char::is_control)
                {
                    return Err(invalid(
                        "Nexus key must contain 1–4,096 characters without control characters.",
                    ));
                }
                let credential = CREDENTIALW {
                    Type: CRED_TYPE_GENERIC,
                    TargetName: target.as_mut_ptr(),
                    CredentialBlobSize: secret.len() as u32,
                    CredentialBlob: secret.as_ptr() as *mut u8,
                    Persist: CRED_PERSIST_LOCAL_MACHINE,
                    ..Default::default()
                };
                if unsafe { CredWriteW(&credential, 0) } == 0 {
                    return Err(invalid(
                        "Windows Credential Manager could not save the Nexus credential.",
                    ));
                }
                Ok(String::new())
            }
            "delete" => {
                if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0
                    && unsafe { GetLastError() } != ERROR_NOT_FOUND
                {
                    return Err(invalid(
                        "Windows Credential Manager could not remove the Nexus credential.",
                    ));
                }
                Ok(String::new())
            }
            _ => Err(invalid("Unknown credential operation.")),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = secret;
        if action == "read" {
            Ok(String::new())
        } else {
            Err(invalid(
                "Persistent Nexus credentials require Windows Credential Manager.",
            ))
        }
    }
}

#[pyfunction]
fn nexus_authenticate(
    api_key: &str,
    remember: bool,
    store: &Bound<'_, PyAny>,
    factory: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    check_cancel(Some(cancel))?;
    let key = if api_key.is_empty() {
        store.call_method0("read")?.extract::<String>()?
    } else {
        api_key.to_string()
    };
    if key.is_empty() {
        return Err(invalid(
            "No saved Nexus credential. Sign in with a personal API key first.",
        ));
    }
    let client = factory.call1((key,))?;
    let result = (|| {
        client.call_method0("connect")?;
        check_cancel(Some(cancel))?;
        if remember {
            store.call_method1("write", (client.getattr("_api_key")?,))?;
        }
        Ok(client.clone().unbind())
    })();
    if result.is_err() {
        client.call_method0("forget")?;
    }
    result
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(nexus_credential, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_authenticate, m)?)?;
    m.add_function(wrap_pyfunction!(agent_nexus_auth, m)?)?;
    Ok(())
}

#[pyfunction]
fn agent_nexus_auth(
    py: Python<'_>,
    action: &str,
    api_key: &str,
    remember: bool,
) -> PyResult<Py<PyAny>> {
    let module = py.import("creation_lib.mod_manager.nexus_auth")?;
    let store = module.call_method0("CredentialStore")?;
    match action {
        "login" => {
            let client = module.call_method1("authenticate", (api_key, remember))?;
            let user = client.getattr("user")?.unbind();
            client.call_method0("forget")?;
            Ok(user)
        }
        "logout" => {
            store.call_method0("delete")?;
            Ok(crate::plugin_inventory::object(py, &serde_json::json!({"saved":false}))?.unbind())
        }
        "status" => Ok(crate::plugin_inventory::object(
            py,
            &serde_json::json!({"saved":store.call_method0("read")?.is_truthy()?}),
        )?
        .unbind()),
        _ => Err(invalid("Unknown Nexus authentication action.")),
    }
}
