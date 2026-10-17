use crate::{
    controller::{callback, submit_call},
    invalid,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyInt, PyString, PyTuple},
};

fn function<'py>(py: Python<'py>, module: &str, name: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import(format!("creation_lib.mod_manager.{module}"))?
        .getattr(name)
}
fn nexus_error(py: Python<'_>, message: &str) -> PyErr {
    match function(py, "nexus", "NexusError").and_then(|t| t.call1((message,))) {
        Ok(e) => PyErr::from_value(e),
        Err(e) => e,
    }
}
#[pyfunction]
fn controller_download_local_recovery(job: &Bound<'_, PyAny>) -> PyResult<bool> {
    let record = job.get_item("record")?;
    let bytes = record.call_method1("get", ("bytes",))?;
    Ok(job.get_item("state")?.extract::<String>()? == "finalizing"
        || (bytes.is_exact_instance_of::<PyInt>()
            && bytes.gt(0)?
            && bytes.eq(record.call_method1("get", ("total",))?)?))
}
#[pyfunction]
fn controller_download_action(
    manager: &Bound<'_, PyAny>,
    action: &str,
    args: &Bound<'_, PyTuple>,
) -> PyResult<Py<PyAny>> {
    let py = manager.py();
    let library = manager.getattr("library")?;
    let scheduler = manager.getattr("_download_scheduler")?;
    match action {
        "downloads_active" => {
            return Ok(
                (scheduler.is_truthy()? && scheduler.getattr("active")?.is_truthy()?)
                    .into_pyobject(py)?
                    .to_owned()
                    .into_any()
                    .unbind(),
            );
        }
        "transfer_active" => {
            return Ok((scheduler.is_truthy()?
                && scheduler
                    .call_method1("has", (args.get_item(0)?,))?
                    .is_truthy()?)
            .into_pyobject(py)?
            .to_owned()
            .into_any()
            .unbind());
        }
        "_assert_transfer_idle" => {
            if manager.getattr("downloads_active")?.is_truthy()? {
                return Err(invalid(
                    "Pause background downloads and wait for their checkpoints before changing the library, account, transfer limit or launching a runtime.",
                ));
            }
        }
        "accept_nxm" => {
            let raw = args.get_item(0)?;
            let link = if raw.is_instance_of::<PyString>() {
                function(py, "nexus", "parse_nxm")?.call1((raw,))?
            } else {
                raw
            };
            link.call_method0("check_expiry")?;
            let game = manager.getattr("nexus_game")?;
            if !link.getattr("game")?.eq(&game)? {
                return Err(nexus_error(
                    py,
                    &format!(
                        "Nexus game mismatch: this link is for {}; the selected library game is {}.",
                        link.getattr("game")?.str()?,
                        game.str()?
                    ),
                ));
            }
            let identity = link.getattr("identity")?;
            let pending = manager.getattr("pending_links")?;
            let duplicate = pending.contains(&identity)?;
            if !duplicate && pending.len()? >= 100 {
                return Err(nexus_error(
                    py,
                    "The download request queue is full. Remove or download an existing request first.",
                ));
            }
            let destination = args.get_item(1)?;
            let current = if destination.is_none() {
                function(py, "nxm_routing", "context")?.call1((manager,))?
            } else {
                function(py, "nxm_routing", "validate")?.call1((manager, destination))?
            };
            let binding = if current.get_item("ready")?.is_truthy()? {
                function(py, "nxm_routing", "pin")?.call1((current,))?
            } else {
                py.None().into_bound(py)
            };
            let destinations = manager.getattr("pending_destinations")?;
            if duplicate
                && manager
                    .call_method1("transfer_active", (&identity,))?
                    .is_truthy()?
                && !destinations
                    .call_method1("get", (&identity,))?
                    .eq(&binding)?
            {
                return Err(nexus_error(
                    py,
                    "This file has an active transfer for another destination. Pause it before routing a new request.",
                ));
            }
            pending.set_item(&identity, &link)?;
            destinations.set_item(&identity, binding)?;
            manager.setattr(
                "status",
                format!(
                    "{} Nexus request: {}. Connect and choose Download to continue.",
                    if duplicate { "Refreshed" } else { "Received" },
                    link.getattr("label")?.str()?
                ),
            )?;
            return Ok(link.unbind());
        }
        "connect_nexus" => {
            manager.call_method0("_assert_transfer_idle")?;
            let key = args.get_item(0)?.unbind();
            let factory = args.get_item(1)?.unbind();
            let remember = args.get_item(2)?.is_truthy()?;
            let store = manager.getattr("credential_store")?.unbind();
            manager.setattr("_startup_auth_checked", true)?;
            let work = callback(py, move |py, args, _| {
                let options = PyDict::new(py);
                options.set_item("store", store.bind(py))?;
                options.set_item("factory", factory.bind(py))?;
                options.set_item("cancel", args.get_item(0)?)?;
                Ok(py
                    .import("creation_lib.mod_manager.nexus_auth")?
                    .getattr("authenticate")?
                    .call((key.bind(py), remember), Some(&options))?
                    .unbind())
            })?;
            let owner = manager.clone().unbind();
            let complete = callback(py, move |py, args, _| {
                let manager = owner.bind(py);
                let previous = manager.getattr("nexus_client")?;
                if previous.is_truthy()? {
                    previous.call_method0("forget")?;
                }
                let client = args.get_item(0)?;
                manager.setattr("nexus_client", &client)?;
                manager.getattr("nexus_states")?.call_method0("clear")?;
                manager.setattr(
                    "_status_revision",
                    manager.getattr("_status_revision")?.extract::<u64>()? + 1,
                )?;
                manager.setattr(
                    "status",
                    format!(
                        "Connected to Nexus as {}.",
                        client.getattr("user")?.get_item("name")?.str()?
                    ),
                )?;
                Ok(py.None())
            })?;
            manager.call_method1("submit", ("Connecting to Nexus", work, complete))?;
        }
        "disconnect_nexus" => {
            manager.call_method0("_assert_transfer_idle")?;
            if manager.getattr("busy")?.is_truthy()? {
                return Err(invalid(
                    "Wait for or cancel the current operation before disconnecting.",
                ));
            }
            manager
                .getattr("credential_store")?
                .call_method0("delete")?;
            manager.setattr("_startup_auth_checked", true)?;
            let client = manager.getattr("nexus_client")?;
            if client.is_truthy()? {
                client.call_method0("forget")?;
            }
            manager.setattr("nexus_client", py.None())?;
            manager.getattr("nexus_states")?.call_method0("clear")?;
            manager.setattr(
                "_status_revision",
                manager.getattr("_status_revision")?.extract::<u64>()? + 1,
            )?;
            manager.setattr(
                "status",
                "Disconnected from Nexus; saved and session credentials removed.",
            )?;
        }
        "download_nxm" | "resume_download" => {
            let client = manager.getattr("nexus_client")?;
            let pending = manager.getattr("pending_links")?;
            let (identity, link, queue, identifier, local) = if action == "download_nxm" {
                let identity = args.get_item(0)?;
                let link = pending.get_item(&identity)?;
                let binding = manager
                    .getattr("pending_destinations")?
                    .call_method1("get", (&identity,))?;
                if !binding.is_none()
                    && !binding.eq(function(py, "nxm_routing", "pin")?.call1((function(
                        py,
                        "nxm_routing",
                        "context",
                    )?
                    .call1((manager,))?,))?)?
                {
                    return Err(nexus_error(
                        py,
                        "The request belongs to another library/profile/target. Return to that destination or explicitly review a fresh route.",
                    ));
                }
                if !library.is_truthy()? || !client.is_truthy()? {
                    return Err(nexus_error(
                        py,
                        "Open a library and connect a Nexus account for this session first.",
                    ));
                }
                if !link.getattr("game")?.eq(manager.getattr("nexus_game")?)? {
                    return Err(nexus_error(
                        py,
                        "Nexus game changed; select the matching game before downloading.",
                    ));
                }
                (
                    identity,
                    link,
                    py.None().into_bound(py),
                    "".into_pyobject(py)?.into_any(),
                    false,
                )
            } else {
                if !library.is_truthy()? {
                    return Err(nexus_error(
                        py,
                        "Open this download's manager library first.",
                    ));
                }
                let queue = function(py, "download_queue", "DownloadQueue")?.call1((
                    &library,
                    manager.getattr("_download_events")?.getattr("put")?,
                ))?;
                let identifier = args.get_item(0)?;
                let job = queue.call_method1("get", (&identifier,))?;
                let local = controller_download_local_recovery(&job)?;
                if !local && !client.is_truthy()? {
                    return Err(nexus_error(
                        py,
                        "Connect a Nexus account for this session before resuming a network transfer.",
                    ));
                }
                if !local && !job.get_item("game")?.eq(manager.getattr("nexus_game")?)? {
                    return Err(nexus_error(
                        py,
                        "Select the queued download's game before resuming.",
                    ));
                }
                let identity = (
                    job.get_item("game")?,
                    job.get_item("mod_id")?,
                    job.get_item("file_id")?,
                )
                    .into_pyobject(py)?
                    .into_any();
                let link = pending.call_method1("get", (&identity,))?;
                (identity, link, queue, identifier, local)
            };
            if manager.getattr("download_slots")?.extract::<usize>()? > 1 {
                manager.call_method1(
                    "_schedule_download",
                    (
                        if local {
                            py.None().into_bound(py)
                        } else {
                            client
                        },
                        link,
                        identifier,
                    ),
                )?;
                return Ok(py.None());
            }
            let owner = manager.clone().unbind();
            let key = identity.unbind();
            let complete = callback(py, move |py, args, _| {
                let manager = owner.bind(py);
                manager
                    .getattr("pending_links")?
                    .call_method1("pop", (key.bind(py), py.None()))?;
                manager.call_method0("refresh")?;
                manager.setattr("status",format!("Nexus archive ready: {}. Review it with Install before adding it to a profile.",args.get_item(0)?.str()?))?;
                Ok(py.None())
            })?;
            if action == "download_nxm" {
                let library = library.unbind();
                let event = manager
                    .getattr("_download_events")?
                    .getattr("put")?
                    .unbind();
                let client = client.unbind();
                let link = link.unbind();
                let work = callback(py, move |py, args, _| {
                    let queue = function(py, "download_queue", "DownloadQueue")?
                        .call1((library.bind(py), event.bind(py)))?;
                    let cancel = args.get_item(0)?;
                    let identifier = queue.call_method1("enqueue", (link.bind(py), &cancel))?;
                    Ok(queue
                        .call_method1(
                            "run",
                            (
                                identifier,
                                client.bind(py),
                                link.bind(py),
                                cancel,
                                args.get_item(1)?,
                            ),
                        )?
                        .unbind())
                })?;
                manager.call_method1("submit", ("Downloading Nexus archive", work, complete))?;
            } else {
                submit_call(
                    manager,
                    "Resuming owned download",
                    &queue.getattr("run")?,
                    &PyTuple::new(py, [identifier, client, link])?,
                    &PyDict::new(py),
                    true,
                    true,
                    complete.bind(py),
                )?;
            }
            manager.setattr(
                "_cancel_status",
                "Pausing download; waiting for the current network read and owned checkpoint.",
            )?;
        }
        "remove_download_job" => {
            let id = args.get_item(0)?;
            if manager
                .call_method1("transfer_active", (&id,))?
                .is_truthy()?
            {
                return Err(invalid(
                    "Pause this transfer and wait for its checkpoint before removing it.",
                ));
            }
            if manager.getattr("busy")?.is_truthy()?
                || manager.getattr("runtime_active")?.is_truthy()?
                || !library.is_truthy()?
            {
                return Err(invalid(
                    "Wait for the current operation/runtime before removing a queued download.",
                ));
            }
            function(py, "download_queue", "DownloadQueue")?
                .call1((&library,))?
                .call_method1("remove", (id,))?;
            manager.setattr(
                "download_jobs",
                function(py, "download_queue", "DownloadQueue")?
                    .call1((&library,))?
                    .call_method0("records")?,
            )?;
            manager.setattr("status","Removed queued/history metadata and any owned partial. Completed archives were retained.")?;
        }
        "set_download_slots" => {
            manager.call_method0("_assert_transfer_idle")?;
            let slots = args.get_item(0)?;
            if !slots.is_exact_instance_of::<PyInt>() || slots.lt(1)? || slots.gt(4)? {
                return Err(invalid(
                    "Choose between one and four simultaneous downloads.",
                ));
            }
            if scheduler.is_truthy()? {
                scheduler.call_method0("close")?;
                manager.setattr("_download_scheduler", py.None())?;
            }
            manager.setattr("download_slots", slots)?;
        }
        "_schedule_download" => {
            if manager.getattr("busy")?.is_truthy()?
                || manager.getattr("runtime_active")?.is_truthy()?
                || !library.is_truthy()?
            {
                return Err(invalid(
                    "Wait for the current operation/runtime before scheduling a transfer.",
                ));
            }
            let mut scheduler = scheduler;
            if scheduler.is_truthy()?
                && !scheduler
                    .getattr("library")?
                    .getattr("root")?
                    .eq(library.getattr("root")?)?
            {
                scheduler.call_method0("close")?;
                scheduler = py.None().into_bound(py);
                manager.setattr("_download_scheduler", py.None())?;
            }
            if !scheduler.is_truthy()? {
                scheduler = function(py, "download_scheduler", "Scheduler")?.call1((
                    &library,
                    manager.getattr("download_slots")?,
                    manager.getattr("_download_events")?.getattr("put")?,
                ))?;
                manager.setattr("_download_scheduler", &scheduler)?;
            }
            scheduler.getattr("add")?.call(args, None)?;
            manager.setattr(
                "status",
                "Background transfer queued. Completed archives require installer review.",
            )?;
            manager.setattr("error", "")?;
        }
        "pause_download" => {
            if scheduler.is_truthy()? {
                scheduler.call_method1("pause", (args.get_item(0)?,))?;
                manager.setattr("status","Pausing background transfer; waiting for its network read and owned checkpoint.")?;
            }
        }
        "_poll_downloads" => {
            if scheduler.is_truthy()? {
                let outcomes = scheduler.call_method0("poll")?;
                for outcome in outcomes.try_iter()? {
                    let outcome = outcome?;
                    match outcome.get_item(1)?.extract::<String>()?.as_str() {
                        "complete" => {
                            manager
                                .getattr("pending_links")?
                                .call_method1("pop", (outcome.get_item(0)?, py.None()))?;
                            manager.setattr("status","Background archive ready. Review it with Install before adding it to a profile.")?;
                        }
                        "failed" => manager.setattr("error", outcome.get_item(2)?)?,
                        _ => manager.setattr("status", outcome.get_item(2)?)?,
                    }
                }
                if outcomes.is_truthy()? && library.is_truthy()? {
                    manager.setattr("download_records", library.call_method0("downloads")?)?;
                    manager.setattr(
                        "download_jobs",
                        function(py, "download_queue", "DownloadQueue")?
                            .call1((library,))?
                            .call_method0("records")?,
                    )?;
                }
            }
        }
        _ => return Err(invalid("Unknown download controller operation.")),
    }
    Ok(py.None())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(controller_download_action, m)?)?;
    m.add_function(wrap_pyfunction!(controller_download_local_recovery, m)?)?;
    Ok(())
}
