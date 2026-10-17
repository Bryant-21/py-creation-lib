use crate::{check_cancel, invalid, nexus::failure};
use pyo3::{
    prelude::*,
    types::{PyDict, PySet, PyTuple},
};

fn dto<'py>(py: Python<'py>, module: &str, name: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import(format!("creation_lib.mod_manager.{module}"))?
        .getattr(name)
}

fn action(value: &str) -> bool {
    matches!(value, "endorse" | "unendorse" | "track" | "untrack")
}

fn endorsement(value: &str) -> bool {
    matches!(value, "endorse" | "unendorse")
}

pub fn distinct<'py>(values: &Bound<'py, PyAny>) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let seen = PySet::empty(values.py())?;
    let mut result = Vec::new();
    for value in values.try_iter()? {
        let value = value?;
        if !seen.contains(&value)? {
            seen.add(&value)?;
            result.push(value);
        }
    }
    Ok(result)
}

#[pyfunction]
fn nexus_destination<'py>(manager: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyDict>> {
    let py = manager.py();
    let mut profile = None;
    for candidate in manager.getattr("profiles")?.try_iter()? {
        let candidate = candidate?;
        if candidate
            .getattr("id")?
            .eq(manager.getattr("profile_id")?)?
        {
            profile = Some(candidate);
            break;
        }
    }
    let mut target = None;
    for candidate in manager.getattr("targets")?.try_iter()? {
        let candidate = candidate?;
        if candidate.getattr("id")?.eq(manager.getattr("target_id")?)? {
            target = Some(candidate);
            break;
        }
    }
    let library = manager.getattr("library")?;
    let result = PyDict::new(py);
    for key in [
        "library_id",
        "library_root",
        "profile_id",
        "profile_name",
        "target_id",
        "target_name",
        "target_root",
        "game",
    ] {
        result.set_item(key, "")?;
    }
    let mut game = String::new();
    if library.is_truthy()? {
        result.set_item("library_id", library.getattr("library_id")?)?;
        result.set_item("library_root", library.getattr("root")?.str()?)?;
    }
    if let Some(profile) = &profile {
        result.set_item("profile_id", profile.getattr("id")?)?;
        result.set_item("profile_name", profile.getattr("name")?)?;
        game = crate::nexus::nexus_game_domain(
            &profile
                .getattr("files")?
                .call_method1("get", ("_modbox_game", ""))?
                .extract::<String>()?,
        );
        if game.is_empty() && library.is_truthy()? {
            let config = manager.getattr("instance_configuration")?;
            let section = config.call_method1("get", ("game", PyDict::new(py)))?;
            game = crate::nexus::nexus_game_domain(
                &section
                    .call_method1("get", ("gamename", ""))?
                    .extract::<String>()?,
            );
        }
    }
    if let Some(target) = target {
        for (key, field) in [
            ("target_id", "id"),
            ("target_name", "name"),
            ("target_root", "game_root"),
        ] {
            result.set_item(key, target.getattr(field)?)?;
        }
        game = "fallout4".into();
    }
    result.set_item(
        "ready",
        library.is_truthy()? && profile.is_some() && !game.is_empty(),
    )?;
    result.set_item("game", game)?;
    Ok(result)
}

#[pyfunction]
fn nexus_destination_pin<'py>(value: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    PyTuple::new(
        value.py(),
        [
            "library_id",
            "profile_id",
            "target_id",
            "target_root",
            "game",
        ]
        .into_iter()
        .map(|k| value.get_item(k))
        .collect::<PyResult<Vec<_>>>()?,
    )
}

#[pyfunction]
fn nexus_destination_validate<'py>(
    manager: &Bound<'py, PyAny>,
    expected: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyDict>> {
    let current = nexus_destination(manager)?;
    if !current.get_item("ready")?.unwrap().is_truthy()? || !current.eq(expected)? {
        return Err(invalid(
            "Browser destination changed. Refresh and choose a current library/profile/target.",
        ));
    }
    Ok(current)
}

#[pyfunction]
fn nexus_review<'py>(
    manager: &Bound<'py, PyAny>,
    mod_id: &Bound<'py, PyAny>,
    requested: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let row = manager
        .getattr("rows_by_id")?
        .call_method1("get", (mod_id,))?;
    let client = manager.getattr("nexus_client")?;
    if !client.is_truthy()?
        || !client.getattr("user")?.is_truthy()?
        || !row.is_truthy()?
        || !row.getattr("nexus_game")?.is_truthy()?
        || !row.getattr("nexus_mod_id")?.is_truthy()?
    {
        return Err(failure(
            "Connect a session account and select a mapped mod.",
        ));
    }
    if !action(requested) {
        return Err(failure("Unsupported Nexus account request."));
    }
    if endorsement(requested) && !row.getattr("version")?.is_truthy()? {
        return Err(failure(
            "Set the installed published version before reviewing an endorsement request.",
        ));
    }
    dto(manager.py(), "nexus_account", "Review")?.call1((
        mod_id,
        requested,
        row.getattr("name")?,
        row.getattr("nexus_game")?,
        row.getattr("nexus_mod_id")?,
        row.getattr("nexus_file_id")?,
        row.getattr("version")?,
        client.getattr("user")?.get_item("name")?,
        manager.call_method0("_nexus_context")?,
    ))
}

#[pyfunction]
fn nexus_review_validate(manager: &Bound<'_, PyAny>, plan: &Bound<'_, PyAny>) -> PyResult<()> {
    if !plan
        .getattr("context")?
        .eq(manager.call_method0("_nexus_context")?)?
        || !nexus_review(
            manager,
            &plan.getattr("mod_id")?,
            &plan.getattr("action")?.extract::<String>()?,
        )?
        .eq(plan)?
    {
        return Err(failure(
            "Account, target, profile or installed metadata changed. Review the account request again.",
        ));
    }
    Ok(())
}

#[pyfunction]
fn nexus_bulk_account<'py>(
    manager: &Bound<'py, PyAny>,
    ids: &Bound<'py, PyAny>,
    requested: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let py = manager.py();
    let client = manager.getattr("nexus_client")?;
    if !client.is_truthy()? || !client.getattr("user")?.is_truthy()? || !action(requested) {
        return Err(failure(
            "Connect an account and choose a supported account action.",
        ));
    }
    let selected = distinct(ids)?;
    if selected.is_empty() || selected.len() > 20 {
        return Err(invalid(
            "Select between one and 20 mods for an account batch.",
        ));
    }
    let mut items = Vec::new();
    for id in selected {
        let row = manager.getattr("rows_by_id")?.call_method1("get", (&id,))?;
        if !row.is_truthy()? {
            return Err(invalid(
                "Selected profile mods changed. Review the selection again.",
            ));
        }
        let game = row.getattr("nexus_game")?;
        let nexus_id = row.getattr("nexus_mod_id")?;
        let mut issue = if !game.is_truthy()? || !nexus_id.is_truthy()? {
            "No known Nexus identity"
        } else {
            ""
        };
        if matches!(
            row.getattr("kind")?.extract::<String>()?.as_str(),
            "separator" | "foreign"
        ) {
            issue = "Select an installed mod";
        }
        if issue.is_empty() {
            let parsed = if nexus_id.is_instance_of::<pyo3::types::PyBool>() {
                serde_json::Value::Null
            } else {
                nexus_id
                    .extract::<u64>()
                    .ok()
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null)
            };
            if crate::nexus::mod_identity(&game.extract::<String>()?, &parsed).is_err() {
                issue = "Unsupported Nexus identity";
            }
        }
        if endorsement(requested) && !row.getattr("version")?.is_truthy()? {
            issue = "Installed published version is unknown";
        }
        let kwargs = PyDict::new(py);
        kwargs.set_item("file_id", row.getattr("nexus_file_id")?)?;
        items.push(dto(py, "nexus_bulk", "Item")?.call(
            (
                row.getattr("id")?,
                row.getattr("name")?,
                (game, nexus_id),
                row.getattr("version")?,
                issue,
            ),
            Some(&kwargs),
        )?);
    }
    dto(py, "nexus_bulk", "Review")?.call1((
        "account",
        requested,
        client.getattr("user")?.get_item("name")?,
        manager.call_method0("_nexus_context")?,
        nexus_destination(manager)?,
        PyTuple::new(py, items)?,
    ))
}

fn download_issue(
    manager: &Bound<'_, PyAny>,
    identity: &Bound<'_, PyAny>,
    link: &Bound<'_, PyAny>,
    destination: &Bound<'_, PyDict>,
    user: &Bound<'_, PyAny>,
) -> PyResult<()> {
    link.call_method0("check_expiry")?;
    if !link
        .getattr("game")?
        .eq(destination.get_item("game")?.unwrap())?
    {
        return Err(failure("Request game differs from the selected profile"));
    }
    let pending = manager
        .getattr("pending_destinations")?
        .call_method1("get", (identity,))?;
    if !pending.is_none() && !pending.eq(nexus_destination_pin(destination.as_any())?)? {
        return Err(failure(
            "Request is bound to another library/profile/target",
        ));
    }
    if manager
        .call_method1("transfer_active", (identity,))?
        .is_truthy()?
    {
        return Err(failure("Already active or queued"));
    }
    if link.getattr("user_id")?.is_truthy()?
        && !link.getattr("user_id")?.eq(user.get_item("user_id")?)?
    {
        return Err(failure("Browser link belongs to another account"));
    }
    if !user.get_item("premium")?.is_truthy()?
        && !(link.getattr("key")?.is_truthy()?
            && link.getattr("expires")?.is_truthy()?
            && link.getattr("user_id")?.is_truthy()?)
    {
        return Err(failure(
            "Free account needs a fresh authenticated browser link",
        ));
    }
    Ok(())
}

#[pyfunction]
fn nexus_bulk_downloads<'py>(manager: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = manager.py();
    let client = manager.getattr("nexus_client")?;
    let destination = nexus_destination(manager)?;
    if !client.is_truthy()?
        || !client.getattr("user")?.is_truthy()?
        || !destination.get_item("ready")?.unwrap().is_truthy()?
    {
        return Err(failure(
            "Open a known-game profile and connect a session account before reviewing downloads.",
        ));
    }
    let user = client.getattr("user")?;
    let mut items = Vec::new();
    for pair in manager
        .getattr("pending_links")?
        .call_method0("items")?
        .try_iter()?
        .take(20)
    {
        let pair = pair?;
        let identity = pair.get_item(0)?;
        let link = pair.get_item(1)?;
        let issue = match download_issue(manager, &identity, &link, &destination, &user) {
            Ok(()) => String::new(),
            Err(error) if error.is_instance_of::<crate::nexus::NexusFailure>(py) => error
                .value(py)
                .getattr("args")?
                .get_item(0)?
                .extract::<String>()?,
            Err(error) if error.matches(py, dto(py, "nexus", "NexusError")?)? => {
                error.value(py).str()?.to_string()
            }
            Err(error) => return Err(error),
        };
        let kwargs = PyDict::new(py);
        kwargs.set_item("issue", issue)?;
        kwargs.set_item("link", &link)?;
        items.push(dto(py, "nexus_bulk", "Item")?.call(
            (&identity, link.getattr("label")?, &identity),
            Some(&kwargs),
        )?);
    }
    if items.is_empty() {
        return Err(failure("No pending browser requests to review."));
    }
    dto(py, "nexus_bulk", "Review")?.call1((
        "downloads",
        "queue",
        user.get_item("name")?,
        manager.call_method0("_nexus_context")?,
        destination,
        PyTuple::new(py, items)?,
    ))
}

#[pyfunction]
fn nexus_bulk_validate<'py>(
    manager: &Bound<'py, PyAny>,
    plan: &Bound<'py, PyAny>,
    selected: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyTuple>> {
    let py = manager.py();
    if !plan
        .getattr("context")?
        .eq(manager.call_method0("_nexus_context")?)?
        || !plan
            .getattr("destination")?
            .eq(nexus_destination(manager)?)?
    {
        return Err(failure(
            "Account or destination context changed. Review this batch again.",
        ));
    }
    let account = plan.getattr("kind")?.extract::<String>()? == "account";
    if account {
        if manager.getattr("downloads_active")?.is_truthy()? {
            return Err(failure(
                "Pause or finish transfers before sending a reviewed account batch.",
            ));
        }
        if py
            .import("time")?
            .call_method0("monotonic")?
            .extract::<f64>()?
            < manager.getattr("_rate_blocked_until")?.extract::<f64>()?
        {
            return Err(failure(
                "Account batches are waiting for Nexus rate-limit backoff.",
            ));
        }
    }
    let selected = PySet::new(py, distinct(selected)?)?;
    let ids = PySet::empty(py)?;
    let mut items = Vec::new();
    let mut all_ids = Vec::new();
    for item in plan.getattr("items")?.try_iter()? {
        let item = item?;
        let id = item.getattr("id")?;
        all_ids.push(id.clone());
        if selected.contains(&id)? {
            ids.add(&id)?;
            items.push(item);
        }
    }
    if items.is_empty()
        || !selected.eq(&ids)?
        || items.iter().any(|item| {
            item.getattr("issue")
                .and_then(|v| v.is_truthy())
                .unwrap_or(true)
        })
    {
        return Err(invalid("Choose available items from the reviewed batch."));
    }
    let requested = plan.getattr("action")?.extract::<String>()?;
    let fresh = if account {
        nexus_bulk_account(manager, PyTuple::new(py, all_ids)?.as_any(), &requested)?
    } else {
        nexus_bulk_downloads(manager)?
    };
    if !fresh.eq(plan)? {
        return Err(failure(
            "Reviewed requests or installed versions changed. Review the batch again.",
        ));
    }
    let versions = PyDict::new(py);
    for item in &items {
        let identity = item.getattr("identity")?;
        let version = item.getattr("version")?;
        if endorsement(&requested) {
            if let Some(previous) = versions.get_item(&identity)? {
                if !previous.eq(&version)? {
                    return Err(invalid(
                        "Choose one installed version per Nexus mod identity for an endorsement batch.",
                    ));
                }
            }
        }
        versions.set_item(identity, version)?;
    }
    PyTuple::new(py, items)
}

#[pyfunction]
fn nexus_bulk_run<'py>(
    manager: &Bound<'py, PyAny>,
    plan: &Bound<'py, PyAny>,
    items: &Bound<'py, PyAny>,
    cancel: &Bound<'py, PyAny>,
    progress: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = manager.py();
    let client = manager.getattr("nexus_client")?;
    let mut rows = Vec::new();
    let seen = PySet::empty(py)?;
    let mut stopped = false;
    let mut retry = 0;
    let requested = plan.getattr("action")?.extract::<String>()?;
    for (index, item) in items.try_iter()?.enumerate() {
        let item = item?;
        let id = item.getattr("id")?;
        let identity = item.getattr("identity")?;
        if stopped {
            rows.push((id, "Not sent"));
            continue;
        }
        if seen.contains(&identity)? {
            rows.push((id, "Same Nexus identity: request already sent once"));
            continue;
        }
        let cancelled = match check_cancel(Some(cancel)) {
            Ok(()) => false,
            Err(error) if error.is_instance_of::<crate::OperationCancelled>(py) => true,
            Err(error) => return Err(error),
        };
        if cancelled
            || !plan
                .getattr("context")?
                .eq(manager.call_method0("_nexus_context")?)?
        {
            stopped = true;
            rows.push((id, "Not sent: batch stopped"));
            continue;
        }
        seen.add(&identity)?;
        let operation = if endorsement(&requested) {
            client.call_method1(
                requested.as_str(),
                (
                    identity.get_item(0)?,
                    identity.get_item(1)?,
                    item.getattr("version")?,
                ),
            )
        } else {
            let kwargs = PyDict::new(py);
            kwargs.set_item("enabled", requested == "track")?;
            client.call_method(
                "track",
                (identity.get_item(0)?, identity.get_item(1)?),
                Some(&kwargs),
            )
        };
        match operation {
            Ok(_) => rows.push((id, "Request accepted; refresh account state to verify")),
            Err(error) if error.matches(py, dto(py, "nexus", "NexusError")?)? => {
                rows.push((id, "Remote outcome unconfirmed; refresh before retrying"));
                let value = error.value(py);
                if matches!(
                    value.getattr("http_status")?.extract::<u16>(),
                    Ok(401 | 403 | 429)
                ) {
                    stopped = true;
                    retry = value
                        .getattr("retry_after")?
                        .extract::<i64>()?
                        .clamp(60, 86400);
                }
            }
            Err(error) => return Err(error),
        }
        progress.call1((
            (index + 1) as f64 / items.len()? as f64,
            format!(
                "Processed {} of {} reviewed account items",
                index + 1,
                items.len()?
            ),
        ))?;
    }
    dto(py, "nexus_bulk", "Result")?.call1((PyTuple::new(py, rows)?, stopped, retry))
}

#[pyfunction]
fn nexus_bulk_queue<'py>(
    manager: &Bound<'py, PyAny>,
    plan: &Bound<'py, PyAny>,
    selected: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = manager.py();
    let items = nexus_bulk_validate(manager, plan, selected)?;
    let mut rows = Vec::new();
    for item in items.iter() {
        let status = match manager.call_method1(
            "_schedule_download",
            (manager.getattr("nexus_client")?, item.getattr("link")?),
        ) {
            Ok(_) => "Queued for owned download; installation requires review".into(),
            Err(error) if error.matches(py, dto(py, "paths", "ManagerError")?)? => {
                format!("Not queued: {}", error.value(py).str()?)
            }
            Err(error) => return Err(error),
        };
        rows.push((item.getattr("id")?, status));
    }
    dto(py, "nexus_bulk", "Result")?.call1((PyTuple::new(py, rows)?,))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(nexus_destination, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_destination_pin, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_destination_validate, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_review, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_review_validate, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_bulk_account, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_bulk_downloads, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_bulk_validate, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_bulk_run, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_bulk_queue, m)?)?;
    Ok(())
}
