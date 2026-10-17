use crate::controller::{callback, submit_call};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyString, PyTuple},
};
fn function<'py>(py: Python<'py>, module: &str, name: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import(format!("creation_lib.mod_manager.{module}"))?
        .getattr(name)
}
fn error(py: Python<'_>, message: &str) -> PyErr {
    match function(py, "nexus", "NexusError").and_then(|t| t.call1((message,))) {
        Ok(e) => PyErr::from_value(e),
        Err(e) => e,
    }
}
fn now(py: Python<'_>) -> PyResult<f64> {
    py.import("time")?.call_method0("monotonic")?.extract()
}
fn revision(manager: &Bound<'_, PyAny>) -> PyResult<()> {
    manager.setattr(
        "_status_revision",
        manager.getattr("_status_revision")?.extract::<u64>()? + 1,
    )
}
fn identity<'py>(row: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    (row.getattr("nexus_game")?, row.getattr("nexus_mod_id")?).into_pyobject(row.py())
}
#[pyfunction]
fn controller_nexus_context(manager: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = manager.py();
    let client = manager.getattr("nexus_client")?;
    let library = manager.getattr("library")?;
    let user = if client.is_truthy()? {
        client
            .getattr("user")
            .unwrap_or_else(|_| py.None().into_bound(py))
    } else {
        py.None().into_bound(py)
    };
    let user = if user.is_truthy()? {
        user.call_method1("get", ("user_id",))?
    } else {
        py.None().into_bound(py)
    };
    let mut rows = vec![];
    for row in manager.getattr("rows")?.try_iter()? {
        let row = row?;
        let mut values = vec![];
        for key in [
            "id",
            "source",
            "version",
            "nexus_game",
            "nexus_mod_id",
            "nexus_file_id",
        ] {
            values.push(
                row.getattr(key)
                    .unwrap_or_else(|_| "".into_pyobject(py).unwrap().into_any()),
            );
        }
        rows.push(PyTuple::new(py, values)?);
    }
    Ok((
        py.import("builtins")?.call_method1("id", (&client,))?,
        user,
        if library.is_truthy()? {
            library.getattr("library_id")?
        } else {
            "".into_pyobject(py)?.into_any()
        },
        manager.getattr("profile_id")?,
        manager.getattr("target_id")?,
        PyTuple::new(py, rows)?,
    )
        .into_pyobject(py)?
        .into_any()
        .unbind())
}
#[pyfunction]
fn controller_nexus_action(
    manager: &Bound<'_, PyAny>,
    action: &str,
    args: &Bound<'_, PyTuple>,
) -> PyResult<()> {
    let py = manager.py();
    let client = manager.getattr("nexus_client")?;
    if action == "tick_updates" {
        if !manager.getattr("_startup_auth_checked")?.is_truthy()?
            && manager.getattr("library")?.is_truthy()?
            && !manager.getattr("busy")?.is_truthy()?
        {
            manager.setattr("_startup_auth_checked", true)?;
            if !client.is_truthy()?
                && manager
                    .getattr("credential_store")?
                    .call_method0("read")?
                    .is_truthy()?
            {
                manager.call_method0("connect_nexus")?;
                return Ok(());
            }
        }
        if client.is_truthy()? && !manager.getattr("busy")?.is_truthy()? {
            crate::update_schedule::prepare(
                manager,
                manager.getattr("auto_file_checks")?.is_truthy()?,
            )?;
        }
        let time = args.get_item(0)?;
        let time = if time.is_none() {
            now(py)?
        } else {
            time.extract()?
        };
        if manager.getattr("auto_updates")?.is_truthy()?
            && client.is_truthy()?
            && manager.getattr("rows")?.is_truthy()?
            && !manager.getattr("busy")?.is_truthy()?
            && !manager.getattr("downloads_active")?.is_truthy()?
            && time >= manager.getattr("_next_update")?.extract::<f64>()?
        {
            for row in manager.getattr("rows")?.try_iter()? {
                let row = row?;
                if row.getattr("nexus_game")?.is_truthy()?
                    && row.getattr("nexus_mod_id")?.is_truthy()?
                {
                    let options = PyDict::new(py);
                    options.set_item("manual", false)?;
                    manager
                        .getattr(if manager.getattr("auto_file_checks")?.is_truthy()? {
                            "check_nexus_batch"
                        } else {
                            "check_updates"
                        })?
                        .call((), Some(&options))?;
                    break;
                }
            }
        }
        return Ok(());
    }
    let context = controller_nexus_context(manager)?;
    if ["inspect_nexus_mod", "nexus_account_action"].contains(&action) {
        let row = manager
            .getattr("rows_by_id")?
            .call_method1("get", (args.get_item(0)?,))?;
        if !client.is_truthy()?
            || !row.is_truthy()?
            || !row.getattr("nexus_game")?.is_truthy()?
            || !row.getattr("nexus_mod_id")?.is_truthy()?
        {
            return Err(error(
                py,
                "Connect a Nexus account and select a mod with a verified Nexus identity.",
            ));
        }
        let key = identity(&row)?;
        if action == "inspect_nexus_mod" {
            let installed = PyList::empty(py);
            for item in manager.getattr("rows")?.try_iter()? {
                let item = item?;
                let file = item.getattr("nexus_file_id")?;
                if identity(&item)?.as_any().eq(&key)?
                    && file.is_truthy()?
                    && !installed.contains(&file)?
                    && installed.len() < 64
                {
                    installed.append(file)?;
                }
            }
            let source = client.unbind();
            let selected = row.getattr("nexus_file_id")?.unbind();
            let files = installed.unbind();
            let id = key.clone().unbind();
            let work = callback(py, move |py, args, _| {
                let cancel = args.get_item(0)?;
                let id = id.bind(py);
                let state = function(py, "nexus_state", "inspect")?.call1((
                    source.bind(py),
                    id.get_item(0)?,
                    id.get_item(1)?,
                    &cancel,
                ))?;
                state.call_method1("has_replacement", (selected.bind(py),))?;
                for file in files.bind(py).iter() {
                    crate::check_cancel(Some(&cancel))?;
                    state.call_method1("replacements", (file,))?;
                }
                Ok(state.unbind())
            })?;
            let owner = manager.clone().unbind();
            let key = key.unbind();
            let complete = callback(py, move |py, args, _| {
                let manager = owner.bind(py);
                if !context
                    .bind(py)
                    .eq(controller_nexus_context(manager)?.bind(py))?
                {
                    manager.setattr(
                        "status",
                        "Nexus read discarded because the session or local context changed.",
                    )?;
                    return Ok(py.None());
                }
                manager
                    .getattr("nexus_states")?
                    .set_item(key.bind(py), args.get_item(0)?)?;
                revision(manager)?;
                manager.setattr(
                    "status",
                    "Refreshed published file replacements and connected-account state.",
                )?;
                Ok(py.None())
            })?;
            manager.call_method1(
                "submit",
                ("Reading Nexus files and account state", work, complete),
            )?;
        } else {
            let change = args.get_item(1)?.extract::<String>()?;
            if !["endorse", "unendorse", "track", "untrack"].contains(&change.as_str()) {
                return Err(error(py, "Unsupported Nexus account action."));
            }
            manager
                .getattr("nexus_states")?
                .call_method1("pop", (&key, py.None()))?;
            revision(manager)?;
            let owner = manager.clone().unbind();
            let source = client.unbind();
            let key = key.unbind();
            let version = row.getattr("version")?.unbind();
            let work = callback(py, move |py, args, _| {
                crate::check_cancel(Some(&args.get_item(0)?))?;
                if !context
                    .bind(py)
                    .eq(controller_nexus_context(owner.bind(py))?.bind(py))?
                {
                    return Err(error(
                        py,
                        "Nexus account or installed context changed before sending.",
                    ));
                }
                let id = key.bind(py);
                let client = source.bind(py);
                let result = if ["endorse", "unendorse"].contains(&change.as_str()) {
                    client.call_method1(
                        change.as_str(),
                        (id.get_item(0)?, id.get_item(1)?, version.bind(py)),
                    )
                } else {
                    let options = PyDict::new(py);
                    options.set_item("enabled", change == "track")?;
                    client.getattr("track")?.call(id, Some(&options))
                };
                let message=match result {Ok(_)=>"Nexus accepted the account request. Refresh account state to verify the result.".into(),Err(e)=>{
                    if !e.matches(py,function(py,"nexus","NexusError")?)? {return Err(e);}
                    format!("Nexus account request outcome is unconfirmed. Refresh account state before retrying. {}",e.value(py).str()?)
                }};
                Ok(message.into_pyobject(py)?.into_any().unbind())
            })?;
            let owner = manager.clone().unbind();
            let complete = callback(py, move |py, args, _| {
                owner.bind(py).setattr("status", args.get_item(0)?)?;
                Ok(py.None())
            })?;
            manager.call_method1("submit", ("Sending Nexus account request", work, complete))?;
            manager.setattr("_cancel_status","Cancellation requested. A Nexus account write already sent cannot be undone; refresh account state afterward.")?;
        }
        return Ok(());
    }
    let batch_read = action == "check_nexus_batch";
    if !batch_read && action != "check_updates" {
        return Err(error(py, "Unknown Nexus controller operation."));
    }
    let manual = args.get_item(0)?.is_truthy()?;
    if !client.is_truthy()? {
        return Err(error(
            py,
            if batch_read {
                "Connect an account and wait for any rate-limit backoff before checking."
            } else {
                "Connect a Nexus account for this session first."
            },
        ));
    }
    let identities = crate::update_schedule::prepare(manager, batch_read)?;
    if now(py)? < manager.getattr("_rate_blocked_until")?.extract::<f64>()?
        || !manual && now(py)? < manager.getattr("_next_update")?.extract::<f64>()?
    {
        return Err(error(
            py,
            if batch_read {
                "Connect an account and wait for any rate-limit backoff before checking."
            } else {
                "Update checks are waiting for the configured interval or rate-limit backoff."
            },
        ));
    }
    if identities.is_empty() {
        return Err(error(
            py,
            if batch_read {
                "This profile has no mapped Nexus identities."
            } else {
                "This profile has no verified Nexus mod identities to check."
            },
        ));
    }
    let batch = crate::update_schedule::select(manager, &identities, manual)?;
    if batch.is_empty() {
        return Ok(());
    }
    let installed = PyDict::new(py);
    for id in batch.iter() {
        installed.set_item(&id, PyList::empty(py))?;
    }
    if batch_read {
        for row in manager.getattr("rows")?.try_iter()? {
            let row = row?;
            if let Some(values) = installed.get_item(identity(&row)?)? {
                let file = row.getattr("nexus_file_id")?;
                if file.is_truthy()? && values.len()? < 64 && !values.contains(&file)? {
                    values.call_method1("append", (file,))?;
                }
            }
        }
    }
    for id in batch.iter() {
        manager
            .getattr("updates")?
            .call_method1("pop", (&id, py.None()))?;
        if batch_read {
            manager
                .getattr("nexus_states")?
                .call_method1("pop", (&id, py.None()))?;
        }
    }
    revision(manager)?;
    let owner = manager.clone().unbind();
    let captured = batch.clone().unbind();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let batch = captured.bind(py);
        let result = args.get_item(0)?;
        if !context
            .bind(py)
            .eq(controller_nexus_context(manager)?.bind(py))?
        {
            manager.setattr(
                "status",
                if batch_read {
                    "Batch discarded because the session, target or installed metadata changed."
                } else {
                    "Version check discarded because the session or installed metadata changed."
                },
            )?;
            return Ok(py.None());
        }
        if batch_read {
            for id in batch.iter() {
                for (field, result_field) in [("nexus_states", "states"), ("updates", "versions")] {
                    let target = manager.getattr(field)?;
                    target.call_method1("pop", (&id, py.None()))?;
                    let value = result.getattr(result_field)?.get_item(&id)?;
                    if !value.is_none() {
                        target.set_item(&id, value)?;
                    }
                }
            }
        } else {
            manager
                .getattr("updates")?
                .call_method1("update", (&result,))?;
        }
        revision(manager)?;
        manager.setattr("_update_failures", 0)?;
        crate::update_schedule::complete(manager, batch, &result, batch_read)?;
        manager.setattr("status",if batch_read {format!("Checked {} mapped identities; {} unavailable reads remain unknown. No account writes or downloads were requested.",batch.len(),result.getattr("issues")?.len()?)} else {format!("Checked {} Nexus mod versions. Unmapped mods and individual file-update relationships remain unverified.",result.len()?)})?;
        Ok(py.None())
    })?;
    if batch_read {
        let options = PyDict::new(py);
        options.set_item("installed_files", installed)?;
        submit_call(
            manager,
            "Reading bounded Nexus metadata batch",
            &function(py, "nexus_batch", "read")?,
            &PyTuple::new(py, [client, batch.into_any()])?,
            &options,
            true,
            true,
            complete.bind(py),
        )?;
    } else {
        let client = client.unbind();
        let batch = batch.unbind();
        let work = callback(py, move |py, args, _| {
            let cancel = args.get_item(0)?;
            let progress = args.get_item(1)?;
            let batch = batch.bind(py);
            let result = PyDict::new(py);
            for (i, id) in batch.iter().enumerate() {
                crate::check_cancel(Some(&cancel))?;
                let info = client
                    .bind(py)
                    .getattr("mod_info")?
                    .call(id.cast::<PyTuple>()?, None)?;
                let value = if info.is_instance_of::<PyDict>() {
                    info.call_method1("get", ("version",))?
                } else {
                    py.None().into_bound(py)
                };
                if !value.is_instance_of::<PyString>() {
                    return Err(error(
                        py,
                        "Nexus mod version is unknown; no update is inferred.",
                    ));
                }
                let version = value.extract::<String>()?;
                if version.chars().count() > 256 || version.chars().any(|c| (c as u32) < 32) {
                    return Err(error(
                        py,
                        "Nexus mod version is unknown; no update is inferred.",
                    ));
                }
                result.set_item(id, value)?;
                progress.call1((
                    (i + 1) as f64 / batch.len() as f64,
                    format!("Checked {} of {} mapped mods", i + 1, batch.len()),
                ))?;
            }
            Ok(result.into_any().unbind())
        })?;
        manager.call_method1("submit", ("Checking Nexus mod versions", work, complete))?;
    }
    manager.setattr("_update_pending", true)?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(controller_nexus_action, m)?)?;
    m.add_function(wrap_pyfunction!(controller_nexus_context, m)?)?;
    Ok(())
}
