use crate::{check_cancel, nexus::failure};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};

struct Reads<'py> {
    cancel: &'py Bound<'py, PyAny>,
    blocked: bool,
    retry: i64,
    issues: Vec<String>,
}

impl<'py> Reads<'py> {
    fn attempt(
        &mut self,
        label: &str,
        operation: impl FnOnce() -> PyResult<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let py = self.cancel.py();
        check_cancel(Some(self.cancel))?;
        if self.blocked {
            return Ok(py.None().into_bound(py));
        }
        let result = operation().and_then(|value| {
            check_cancel(Some(self.cancel))?;
            Ok(value)
        });
        match result {
            Ok(value) => Ok(value),
            Err(error)
                if error.is_instance_of::<crate::OperationCancelled>(py)
                    || error.matches(
                        py,
                        py.import("creation_lib.mod_manager.paths")?
                            .getattr("Cancelled")?,
                    )? =>
            {
                Err(error)
            }
            Err(error)
                if error.matches(
                    py,
                    py.import("creation_lib.mod_manager.paths")?
                        .getattr("ManagerError")?,
                )? || error.is_instance_of::<crate::nexus::NexusFailure>(py) =>
            {
                self.issues
                    .push(format!("{label}: unavailable; state remains unknown."));
                let value = error.value(py);
                if let Ok(status) = value.getattr("http_status") {
                    if matches!(status.extract::<u16>(), Ok(401 | 403 | 429)) {
                        self.retry = value.getattr("retry_after")?.extract::<i64>()?.max(60);
                        self.blocked = true;
                    }
                }
                Ok(py.None().into_bound(py))
            }
            Err(error) => Err(error),
        }
    }
}

fn account_state<'py>(
    state: &Bound<'py, PyAny>,
    identity: &Bound<'py, PyAny>,
    endorsements: &Bound<'py, PyAny>,
    tracked: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = state.py();
    let kwargs = PyDict::new(py);
    kwargs.set_item(
        "endorsement",
        if endorsements.is_none() {
            "Unknown".into_pyobject(py)?.into_any()
        } else {
            endorsements.call_method1("get", (identity, "Undecided"))?
        },
    )?;
    kwargs.set_item(
        "tracked",
        if tracked.is_none() {
            py.None().into_bound(py)
        } else {
            tracked
                .contains(identity)?
                .into_pyobject(py)?
                .to_owned()
                .into_any()
        },
    )?;
    py.import("dataclasses")?
        .getattr("replace")?
        .call((state,), Some(&kwargs))
}

#[pyfunction]
fn nexus_inspect<'py>(
    client: &Bound<'py, PyAny>,
    game: &str,
    mod_id: &Bound<'py, PyAny>,
    cancel: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = client.py();
    let module = py.import("creation_lib.mod_manager.nexus_state")?;
    let state = module.call_method1("file_state", (client, game, mod_id, cancel))?;
    let endorsements = module.call_method1(
        "endorsement_snapshot",
        (client.call_method0("endorsements")?, cancel),
    )?;
    let tracked = module.call_method1(
        "tracking_snapshot",
        (client.call_method0("tracked_mods")?, cancel),
    )?;
    check_cancel(Some(cancel))?;
    account_state(
        &state,
        (game, mod_id).into_pyobject(py)?.as_any(),
        &endorsements,
        &tracked,
    )
}

#[pyfunction]
fn nexus_batch_read<'py>(
    client: &Bound<'py, PyAny>,
    identities: &Bound<'py, PyAny>,
    cancel: &'py Bound<'py, PyAny>,
    progress: &Bound<'py, PyAny>,
    installed: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = client.py();
    let identities = crate::nexus_reviews::distinct(identities)?;
    if identities.is_empty() || identities.len() > 20 {
        return Err(failure(
            "Choose between one and 20 distinct Nexus mod identities.",
        ));
    }
    for identity in &identities {
        let game = identity.get_item(0)?.extract::<String>()?;
        let id = identity.get_item(1)?;
        let value = if id.is_instance_of::<pyo3::types::PyBool>() {
            serde_json::Value::Null
        } else {
            id.extract::<u64>()
                .ok()
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null)
        };
        crate::nexus::mod_identity(&game, &value)?;
    }
    let module = py.import("creation_lib.mod_manager.nexus_state")?;
    let states = PyDict::new(py);
    let versions = PyDict::new(py);
    let mut reads = Reads {
        cancel,
        blocked: false,
        retry: 60,
        issues: Vec::new(),
    };
    let endorsements = reads.attempt("Endorsements", || {
        module.call_method1(
            "endorsement_snapshot",
            (client.call_method0("endorsements")?, cancel),
        )
    })?;
    let tracked = reads.attempt("Tracking", || {
        module.call_method1(
            "tracking_snapshot",
            (client.call_method0("tracked_mods")?, cancel),
        )
    })?;
    for (index, identity) in identities.iter().enumerate() {
        let game = identity.get_item(0)?;
        let id = identity.get_item(1)?;
        let version = reads.attempt(&format!("Mod {id} version"), || {
            let info = client.call_method1("mod_info", (&game, &id))?;
            let value = if info.is_instance_of::<PyDict>() {
                info.call_method1("get", ("version",))?
            } else {
                py.None().into_bound(py)
            };
            if !value.extract::<String>().is_ok_and(|v| {
                !v.is_empty() && v.chars().count() <= 256 && !v.chars().any(|c| c < ' ')
            }) {
                return Err(failure("Nexus mod version is unknown."));
            }
            Ok(value)
        })?;
        versions.set_item(identity, version)?;
        let mut state = reads.attempt(&format!("Mod {id} files"), || {
            module.call_method1("file_state", (client, &game, &id, cancel))
        })?;
        if state.is_truthy()? {
            state = account_state(&state, identity, &endorsements, &tracked)?;
            state.getattr("_updatable_files")?;
            if installed.is_truthy()? {
                let files = installed.call_method1("get", (identity, PyTuple::empty(py)))?;
                for file in crate::nexus_reviews::distinct(&files)?.into_iter().take(64) {
                    check_cancel(Some(cancel))?;
                    state.call_method1("replacements", (file,))?;
                }
            }
        }
        states.set_item(identity, state)?;
        progress.call1((
            (index + 1) as f64 / identities.len() as f64,
            format!(
                "Read {} of {} Nexus mod identities",
                index + 1,
                identities.len()
            ),
        ))?;
    }
    py.import("creation_lib.mod_manager.nexus_batch")?
        .getattr("Batch")?
        .call1((
            states,
            versions,
            PyTuple::new(py, reads.issues)?,
            reads.retry,
        ))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(nexus_inspect, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_batch_read, m)?)?;
    Ok(())
}
