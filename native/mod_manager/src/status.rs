use crate::{check_cancel, conflicts::conflict_rows_signature, invalid};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
use std::collections::{BTreeMap, BTreeSet};
fn attr(v: &Bound<'_, PyAny>, name: &str) -> PyResult<String> {
    v.getattr(name)?.extract()
}
fn flag(v: &Bound<'_, PyAny>, name: &str) -> PyResult<bool> {
    Ok(v.hasattr(name)? && v.getattr(name)?.is_truthy()?)
}
pub fn file_states(index: &Bound<'_, PyAny>) -> PyResult<BTreeMap<String, BTreeSet<String>>> {
    if index.is_truthy()? && index.hasattr("states")? {
        return index.getattr("states")?.extract();
    }
    let mut result = BTreeMap::<String, BTreeSet<String>>::new();
    if index.is_truthy()? {
        for entry in index.getattr("files")?.try_iter()? {
            let entry = entry?;
            let winner = entry.getattr("winner")?;
            let count = entry.getattr("active")?.len()?;
            for provider in entry.getattr("providers")?.try_iter()? {
                let provider = provider?;
                let state = if flag(&provider, "hidden")? {
                    "hidden"
                } else if !flag(&provider, "enabled")? {
                    "disabled"
                } else if count == 1 {
                    "uncontested"
                } else if provider.is(&winner) {
                    "wins"
                } else {
                    "loses"
                };
                result
                    .entry(attr(&provider, "mod_id")?)
                    .or_default()
                    .insert(state.into());
            }
        }
    }
    Ok(result)
}
fn incomplete(index: &Bound<'_, PyAny>) -> PyResult<bool> {
    if !index.is_truthy()? || flag(index, "measured")? {
        return Ok(false);
    }
    for issue in index.getattr("issues")?.try_iter()? {
        if !issue?
            .extract::<String>()?
            .starts_with("Unmanaged game content is not indexed:")
        {
            return Ok(true);
        }
    }
    Ok(false)
}
fn unavailable(row: &Bound<'_, PyAny>, index: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(!row.getattr("source")?.is_truthy()?
        || (index.is_truthy()?
            && index
                .getattr("unavailable_mod_ids")?
                .contains(row.getattr("id")?)?))
}
fn conflict(
    row: &Bound<'_, PyAny>,
    index: &Bound<'_, PyAny>,
    states: &BTreeMap<String, BTreeSet<String>>,
) -> PyResult<String> {
    if unavailable(row, index)? {
        return Ok("unavailable".into());
    }
    if index.is_none() || incomplete(index)? {
        return Ok("unknown".into());
    }
    let empty = BTreeSet::new();
    let values = states.get(&attr(row, "id")?).unwrap_or(&empty);
    Ok(if values.contains("wins") && values.contains("loses") {
        "mixed"
    } else if values.contains("loses") {
        "loses"
    } else if values.contains("wins") {
        "wins"
    } else if values.contains("unresolved") {
        "unknown"
    } else if values.contains("uncontested") {
        "uncontested"
    } else if !values.is_empty() {
        "inactive"
    } else {
        "empty"
    }
    .into())
}
#[pyfunction]
fn status_file_states(index: &Bound<'_, PyAny>) -> PyResult<BTreeMap<String, BTreeSet<String>>> {
    file_states(index)
}
#[pyfunction]
fn status_conflict(
    row: &Bound<'_, PyAny>,
    index: &Bound<'_, PyAny>,
    states: BTreeMap<String, BTreeSet<String>>,
) -> PyResult<String> {
    conflict(row, index, &states)
}
#[pyfunction]
fn status_badges(
    row: &Bound<'_, PyAny>,
    index: &Bound<'_, PyAny>,
    states: BTreeMap<String, BTreeSet<String>>,
    updates: &Bound<'_, PyAny>,
    accounts: &Bound<'_, PyAny>,
    scan_state: &str,
) -> PyResult<Py<PyAny>> {
    let py = row.py();
    let badge = py
        .import("creation_lib.mod_manager.status")?
        .getattr("Badge")?;
    let empty = BTreeSet::new();
    let values = states.get(&attr(row, "id")?).unwrap_or(&empty);
    let scanning = ["queued", "scanning"].contains(&scan_state);
    let (mut text, mut icon, description, mut role) = if unavailable(row, index)? {
        (
            "!",
            "ICON_FA_TRIANGLE_EXCLAMATION",
            "Source folder unavailable; file conflicts unknown.",
            "warning",
        )
    } else if index.is_none() {
        (
            if scanning { "…" } else { "?" },
            "ICON_FA_CIRCLE_QUESTION",
            if scanning {
                "Scanning local file providers in the background."
            } else {
                "Local provider scan is unavailable. Use Refresh files to retry; see the scan state in the footer."
            },
            "muted",
        )
    } else if values.contains("wins") && values.contains("loses") {
        (
            "W/L",
            "ICON_FA_BOLT",
            "Wins and loses files among indexed active providers.",
            "warning",
        )
    } else if values.contains("loses") {
        (
            "L",
            "ICON_FA_ARROW_DOWN",
            "Loses files to other indexed active providers.",
            "warning",
        )
    } else if values.contains("wins") {
        (
            "W",
            "ICON_FA_ARROW_UP",
            "Overrides other indexed active providers.",
            "success",
        )
    } else if values.contains("unresolved") {
        (
            "?",
            "ICON_FA_TRIANGLE_EXCLAMATION",
            "Archive attachment or loose/archive precedence is unresolved for this profile.",
            "warning",
        )
    } else if values.contains("uncontested") {
        (
            "=",
            "ICON_FA_CHECK",
            "Indexed active providers are uncontested.",
            "muted",
        )
    } else if !values.is_empty() {
        (
            "off",
            "ICON_FA_EYE_SLASH",
            "Only disabled/hidden indexed providers; none can win.",
            "muted",
        )
    } else {
        (
            "—",
            "ICON_FA_MINUS",
            "No indexed loose files for this mod; an incomplete scan may omit unavailable sources.",
            "muted",
        )
    };
    let mut description = description.to_string();
    if index.is_truthy()? && flag(index, "measured")? {
        description.push_str(&format!("\n{}", attr(index, "scope")?));
        if flag(index, "partial")? {
            description.push_str("\nPartial inventory; observed statuses may change when unavailable providers or archive precedence are resolved.");
        }
        if values.contains("unresolved") {
            description.push_str("\nSome member paths have unresolved precedence.");
        }
        for issue in index.getattr("issues")?.try_iter()?.take(4) {
            description.push('\n');
            description.push_str(&issue?.extract::<String>()?);
        }
    }
    if incomplete(index)? && !["!", "?"].contains(&text) {
        text = "?";
        icon = "ICON_FA_TRIANGLE_EXCLAMATION";
        role = "warning";
        description = format!(
            "Partial file inventory. Observed status: {description} Refresh unavailable sources before relying on precedence."
        );
    }
    let files = badge.call1((text, icon, description, role))?;
    let identity = (row.getattr("nexus_game")?, row.getattr("nexus_mod_id")?);
    let version = updates.call_method1("get", (&identity,))?;
    let installed = row.getattr("version")?;
    let mut update = if !row.getattr("nexus_mod_id")?.is_truthy()? {
        badge.call1((
            "?",
            "ICON_FA_CIRCLE_QUESTION",
            "No Nexus mod mapping; update availability unknown.",
        ))?
    } else if !version.is_truthy()? || !installed.is_truthy()? {
        badge.call1((
            "?",
            "ICON_FA_CIRCLE_QUESTION",
            "Update availability unknown/offline; no verified current version comparison.",
        ))?
    } else if !version.eq(&installed)? {
        badge.call1(("≠","ICON_FA_NOT_EQUAL",format!("Nexus mod version differs: {}; installed metadata: {}. This does not establish a newer version or verify a replacement file.",version.str()?,installed.str()?),"accent"))?
    } else {
        badge.call1(("=","ICON_FA_CHECK","Session Nexus mod version matches installed metadata; file-specific updates are not verified."))?
    };
    let account = if accounts.is_none() {
        py.None().into_bound(py)
    } else {
        accounts.call_method1("get", (identity,))?
    };
    if account.is_truthy()? {
        let replacements =
            account.call_method1("replacements", (row.getattr("nexus_file_id")?,))?;
        if replacements.is_truthy()? {
            let names = replacements
                .try_iter()?
                .take(10)
                .map(|v| Ok(v?.getattr("id")?.str()?.to_string()))
                .collect::<PyResult<Vec<_>>>()?
                .join(", ");
            update=badge.call1(("update","ICON_FA_ARROW_UP",format!("Author-published replacement file(s) for this installed Nexus file: {names}. Review Nexus files in Information before downloading."),"accent"))?;
        }
    }
    let endorsement = if account.is_truthy()? {
        attr(&account, "endorsement")?
    } else {
        String::new()
    };
    let endorse = if endorsement == "Endorsed" {
        badge.call1((
            "endorsed",
            "ICON_FA_CHECK",
            "The connected account's fetched state is Endorsed.",
            "success",
        ))?
    } else if ["Undecided", "Abstained"].contains(&endorsement.as_str()) {
        badge.call1(("unendorsed","ICON_FA_EYE_SLASH",format!("The connected account's fetched endorsement state is {endorsement}. Use Information for the explicit account action.")))?
    } else {
        badge.call1(("?","ICON_FA_CIRCLE_QUESTION","Connected-account endorsement state is unknown. Refresh Nexus files and account in Information."))?
    };
    Ok((files, update, endorse)
        .into_pyobject(py)?
        .into_any()
        .unbind())
}
const FIELDS: [&str; 4] = ["conflicts", "updates", "endorsement", "tracking"];
fn ranks(field: &str) -> &'static [&'static str] {
    match field {
        "conflicts" => &[
            "unavailable",
            "unknown",
            "mixed",
            "loses",
            "wins",
            "uncontested",
            "inactive",
            "empty",
        ],
        "updates" => &["published", "different", "unknown", "matching"],
        "endorsement" => &["unendorsed", "unknown", "endorsed"],
        "tracking" => &["untracked", "unknown", "tracked"],
        _ => &[],
    }
}
#[pyfunction]
fn status_normalized(
    filters: Option<BTreeMap<String, String>>,
) -> PyResult<BTreeMap<String, String>> {
    let mut result = FIELDS
        .iter()
        .map(|k| (k.to_string(), "all".into()))
        .collect::<BTreeMap<_, _>>();
    for (name, value) in filters.unwrap_or_default() {
        if !FIELDS.contains(&name.as_str())
            || !(value == "all"
                || ranks(&name).contains(&value.as_str())
                || (name == "conflicts" && value == "conflicting"))
        {
            return Err(invalid("Unsupported mod status filter."));
        }
        result.insert(name, value);
    }
    Ok(result)
}
#[pyfunction]
fn status_inventory(
    rows: &Bound<'_, PyAny>,
    index: &Bound<'_, PyAny>,
    updates: &Bound<'_, PyAny>,
    accounts: &Bound<'_, PyAny>,
) -> PyResult<BTreeMap<String, BTreeMap<String, String>>> {
    let py = rows.py();
    let index = if index.is_truthy()?
        && !index
            .getattr("signature")?
            .eq(conflict_rows_signature(rows)?.bind(py))?
    {
        py.None().into_bound(py)
    } else {
        index.clone()
    };
    let states = file_states(&index)?;
    let mut result = BTreeMap::new();
    for row in rows.try_iter()? {
        let row = row?;
        if ["separator", "foreign"].contains(&attr(&row, "kind")?.as_str()) {
            continue;
        }
        let conflict = conflict(&row, &index, &states)?;
        let identity = (row.getattr("nexus_game")?, row.getattr("nexus_mod_id")?);
        let version = updates.call_method1("get", (&identity,))?;
        let account = accounts.call_method1("get", (identity,))?;
        let installed = row.getattr("version")?;
        let update = if account.is_truthy()?
            && account
                .call_method1("has_replacement", (row.getattr("nexus_file_id")?,))?
                .is_truthy()?
        {
            "published"
        } else if version.is_truthy()? && installed.is_truthy()? {
            if version.eq(&installed)? {
                "matching"
            } else {
                "different"
            }
        } else {
            "unknown"
        };
        let endorsement = if account.is_truthy()? {
            attr(&account, "endorsement")?
        } else {
            String::new()
        };
        let endorse = if endorsement == "Endorsed" {
            "endorsed"
        } else if ["Undecided", "Abstained"].contains(&endorsement.as_str()) {
            "unendorsed"
        } else {
            "unknown"
        };
        let tracked = if account.is_truthy()? {
            account.getattr("tracked")?.extract::<Option<bool>>()?
        } else {
            None
        };
        let tracking = match tracked {
            Some(true) => "tracked",
            Some(false) => "untracked",
            None => "unknown",
        };
        result.insert(
            attr(&row, "id")?,
            BTreeMap::from([
                ("conflicts".into(), conflict),
                ("updates".into(), update.into()),
                ("endorsement".into(), endorse.into()),
                ("tracking".into(), tracking.into()),
            ]),
        );
    }
    Ok(result)
}
#[pyfunction]
fn status_matches(
    id: &str,
    states: BTreeMap<String, BTreeMap<String, String>>,
    filters: BTreeMap<String, String>,
) -> PyResult<bool> {
    for (name, expected) in filters {
        if expected == "all" {
            continue;
        }
        let Some(status) = states.get(id) else {
            return Ok(false);
        };
        let actual = status
            .get(&name)
            .ok_or_else(|| invalid("Unknown status field"))?;
        let matches = if name == "conflicts" {
            match expected.as_str() {
                "conflicting" => ["mixed", "wins", "loses"].contains(&actual.as_str()),
                "wins" => ["mixed", "wins"].contains(&actual.as_str()),
                "loses" => ["mixed", "loses"].contains(&actual.as_str()),
                _ => actual == &expected,
            }
        } else {
            actual == &expected
        };
        if !matches {
            return Ok(false);
        }
    }
    Ok(true)
}
#[pyfunction]
fn status_sort_key(
    py: Python<'_>,
    id: &str,
    states: BTreeMap<String, BTreeMap<String, String>>,
    field: &str,
) -> PyResult<Py<PyAny>> {
    let keys = if field == "status" {
        FIELDS.to_vec()
    } else {
        vec![field]
    };
    let mut result = Vec::new();
    for field in keys {
        let actual = states
            .get(id)
            .and_then(|v| v.get(field))
            .map(String::as_str)
            .unwrap_or("unknown");
        result.push(
            ranks(field)
                .iter()
                .position(|v| *v == actual)
                .ok_or_else(|| invalid("Unknown status rank"))?,
        );
    }
    if field == "status" {
        Ok(PyTuple::new(py, result)?.into_any().unbind())
    } else {
        Ok(result[0].into_pyobject(py)?.into_any().unbind())
    }
}
fn summarize<'py>(
    loose: &Bound<'py, PyAny>,
    combined: &Bound<'py, PyAny>,
    archive_error: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let py = loose.py();
    let mut states = file_states(loose)?;
    let mut unresolved = 0;
    if combined.is_truthy()? {
        for entry in combined.getattr("files")?.try_iter()? {
            let entry = entry?;
            if !entry.getattr("archives")?.is_truthy()? {
                continue;
            }
            let active = entry
                .getattr("active")?
                .try_iter()?
                .collect::<PyResult<Vec<_>>>()?;
            let mut ids = BTreeSet::new();
            for candidate in &active {
                ids.insert(attr(&candidate.getattr("provider")?, "mod_id")?);
            }
            let winner = entry.getattr("winner")?;
            for candidate in &active {
                let state = if ids.len() <= 1 {
                    "uncontested"
                } else if winner.is_none() {
                    "unresolved"
                } else if candidate.eq(&winner)? {
                    "wins"
                } else {
                    "loses"
                };
                states
                    .entry(attr(&candidate.getattr("provider")?, "mod_id")?)
                    .or_default()
                    .insert(state.into());
            }
            if winner.is_none() && ids.len() > 1 {
                unresolved += 1;
            }
        }
    }
    let mut visible = Vec::new();
    for entry in loose.getattr("signature")?.try_iter()? {
        let entry = entry?;
        if !entry
            .get_item(0)?
            .extract::<String>()?
            .starts_with("f4se-target-")
        {
            visible.push(entry);
        }
    }
    let mut scope = "Observed loose-file providers".to_string();
    if combined.is_truthy()? {
        scope.push_str(
            " and applicable archive members (local FO4 INI/plugin load-plan predictions)",
        );
    }
    let partial = loose.getattr("unavailable_mod_ids")?.is_truthy()?
        || !archive_error.is_empty()
        || unresolved > 0;
    let mut issues = loose.getattr("issues")?.extract::<Vec<String>>()?;
    if !archive_error.is_empty() {
        issues.push(archive_error.into());
    }
    if unresolved > 0 {
        let count = py
            .import("builtins")?
            .call_method1("format", (unresolved, ","))?
            .extract::<String>()?;
        issues.push(format!(
            "{count} member paths have unresolved loose/archive precedence or archive attachment."
        ));
    }
    py.import("creation_lib.mod_manager.local_status")?
        .getattr("StatusIndex")?
        .call1((
            loose,
            states,
            PyTuple::new(py, visible)?,
            loose.getattr("unavailable_mod_ids")?,
            PyTuple::new(py, issues)?,
            scope,
            partial,
        ))
}
#[pyfunction]
fn local_status_summarize(
    loose: &Bound<'_, PyAny>,
    combined: &Bound<'_, PyAny>,
    archive_error: &str,
) -> PyResult<Py<PyAny>> {
    Ok(summarize(loose, combined, archive_error)?.unbind())
}
#[pyfunction]
fn local_status_scan(
    library: &Bound<'_, PyAny>,
    profile: &str,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
    publish: &Bound<'_, PyAny>,
    resolver: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let loose = py
        .import("creation_lib.mod_manager.conflicts")?
        .call_method1("build_index", (library, profile, cancel, progress))?;
    publish.call1((summarize(&loose, py.None().bind(py), "")?, py.None()))?;
    check_cancel(Some(cancel))?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("resolver", resolver)?;
    kwargs.set_item("loose", &loose)?;
    let result = (|| -> PyResult<Py<PyAny>> {
        let combined = py
            .import("creation_lib.mod_manager.combined_data")?
            .getattr("build")?
            .call((library, profile, cancel, progress), Some(&kwargs))?;
        Ok((summarize(&loose, &combined, "")?, combined)
            .into_pyobject(py)?
            .into_any()
            .unbind())
    })();
    match result {
        Ok(v) => Ok(v),
        Err(e) => {
            if e.is_instance(
                py,
                &py.import("creation_lib.mod_manager.paths")?
                    .getattr("Cancelled")?,
            ) {
                return Err(e);
            }
            if e.is_instance_of::<pyo3::exceptions::PyOSError>(py)
                || e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
            {
                Ok((
                    summarize(
                        &loose,
                        py.None().bind(py),
                        &format!("Archive member status unavailable: {}", e.value(py).str()?),
                    )?,
                    py.None(),
                )
                    .into_pyobject(py)?
                    .into_any()
                    .unbind())
            } else {
                Err(e)
            }
        }
    }
}
#[pyfunction]
fn local_status_matches(index: &Bound<'_, PyAny>, rows: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(index.is_truthy()?
        && index
            .getattr("signature")?
            .eq(conflict_rows_signature(rows)?.bind(index.py()))?)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(status_file_states, m)?)?;
    m.add_function(wrap_pyfunction!(status_conflict, m)?)?;
    m.add_function(wrap_pyfunction!(status_badges, m)?)?;
    m.add_function(wrap_pyfunction!(status_normalized, m)?)?;
    m.add_function(wrap_pyfunction!(status_inventory, m)?)?;
    m.add_function(wrap_pyfunction!(status_matches, m)?)?;
    m.add_function(wrap_pyfunction!(status_sort_key, m)?)?;
    m.add_function(wrap_pyfunction!(local_status_summarize, m)?)?;
    m.add_function(wrap_pyfunction!(local_status_scan, m)?)?;
    m.add_function(wrap_pyfunction!(local_status_matches, m)?)?;
    Ok(())
}
