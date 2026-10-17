use crate::{check_cancel, invalid, python_io::with_context};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
use unicode_casefold::UnicodeCaseFold;

#[pyfunction]
fn scheduler_init(
    state: &Bound<'_, PyAny>,
    library: &Bound<'_, PyAny>,
    slots: &Bound<'_, PyAny>,
    observer: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = state.py();
    if slots.is_instance_of::<pyo3::types::PyBool>()
        || !slots.is_instance_of::<pyo3::types::PyInt>()
        || !slots.extract::<i64>().is_ok_and(|v| (1..=4).contains(&v))
    {
        return Err(invalid(
            "Choose between one and four simultaneous downloads.",
        ));
    }
    state.setattr("library", library)?;
    state.setattr("slots", slots)?;
    state.setattr("observer", observer)?;
    state.setattr("transfers", PyDict::new(py))?;
    let executor = py
        .import("concurrent.futures")?
        .getattr("ThreadPoolExecutor")?;
    for (name, count, prefix) in [
        ("_pool", slots.extract::<i64>()?, "mod-download"),
        ("_enqueue_pool", 1, "mod-download-queue"),
    ] {
        let kwargs = PyDict::new(py);
        kwargs.set_item("max_workers", count)?;
        kwargs.set_item("thread_name_prefix", prefix)?;
        state.setattr(name, executor.call((), Some(&kwargs))?)?;
    }
    state.setattr(
        "_resolve_lock",
        py.import("threading")?.getattr("Lock")?.call0()?,
    )?;
    state.setattr("_blocked_until", 0.0)?;
    state.setattr("_closed", false)
}

fn has(state: &Bound<'_, PyAny>, identity: &Bound<'_, PyAny>) -> PyResult<bool> {
    for transfer in state
        .getattr("transfers")?
        .call_method0("values")?
        .try_iter()?
    {
        let transfer = transfer?;
        if transfer.getattr("identity")?.eq(identity)?
            || transfer.getattr("identifier")?.eq(identity)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[pyfunction]
fn scheduler_has(state: &Bound<'_, PyAny>, identity: &Bound<'_, PyAny>) -> PyResult<bool> {
    has(state, identity)
}

#[pyclass(frozen)]
struct RateLimitedClient {
    client: Py<PyAny>,
    state: Py<PyAny>,
    cancel: Py<PyAny>,
}

#[pymethods]
impl RateLimitedClient {
    fn __traverse__(
        &self,
        visit: pyo3::class::gc::PyVisit<'_>,
    ) -> Result<(), pyo3::class::gc::PyTraverseError> {
        visit.call(&self.client)?;
        visit.call(&self.state)?;
        visit.call(&self.cancel)
    }
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        Ok(self.client.bind(py).getattr(name)?.unbind())
    }

    fn resolve(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let state = self.state.bind(py);
        with_context(&state.getattr("_resolve_lock")?, |_| {
            check_cancel(Some(self.cancel.bind(py)))?;
            let time = py.import("time")?;
            let now = time.call_method0("monotonic")?.extract::<f64>()?;
            let nexus_error = py
                .import("creation_lib.mod_manager.nexus")?
                .getattr("NexusError")?;
            if now < state.getattr("_blocked_until")?.extract::<f64>()? {
                return Err(PyErr::from_value(nexus_error.call1((
                    "Nexus transfer requests are waiting for rate-limit backoff.",
                ))?));
            }
            match self.client.bind(py).call_method1("resolve", (value,)) {
                Ok(value) => Ok(value.unbind()),
                Err(error) => {
                    if error.matches(py, &nexus_error)?
                        && error
                            .value(py)
                            .str()?
                            .to_str()?
                            .case_fold()
                            .collect::<String>()
                            .contains("rate limit")
                    {
                        let retry = error
                            .value(py)
                            .getattr("retry_after")
                            .map(|v| v.extract::<f64>())
                            .unwrap_or(Ok(3600.0))?
                            .clamp(60.0, 86400.0);
                        state.setattr(
                            "_blocked_until",
                            time.call_method0("monotonic")?.extract::<f64>()? + retry,
                        )?;
                    }
                    Err(error)
                }
            }
        })
    }
}

#[pyclass(frozen)]
struct TransferJob {
    state: Py<PyAny>,
    queue: Py<PyAny>,
    client: Py<PyAny>,
    link: Py<PyAny>,
    transfer: Py<PyAny>,
}

#[pymethods]
impl TransferJob {
    fn __traverse__(
        &self,
        visit: pyo3::class::gc::PyVisit<'_>,
    ) -> Result<(), pyo3::class::gc::PyTraverseError> {
        visit.call(&self.state)?;
        visit.call(&self.queue)?;
        visit.call(&self.client)?;
        visit.call(&self.link)?;
        visit.call(&self.transfer)
    }
    fn progress(
        &self,
        py: Python<'_>,
        fraction: &Bound<'_, PyAny>,
        detail: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        self.transfer
            .bind(py)
            .setattr("progress", (fraction, detail))
    }

    fn work(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let private = if slf.client.bind(py).is_truthy()? {
            slf.client.bind(py).call_method0("for_transfer")?
        } else {
            py.None().into_bound(py)
        };
        let transfer = slf.transfer.bind(py);
        let result = (|| {
            let client = if private.is_truthy()? {
                Py::new(
                    py,
                    RateLimitedClient {
                        client: private.clone().unbind(),
                        state: slf.state.clone_ref(py),
                        cancel: transfer.getattr("cancel")?.unbind(),
                    },
                )?
                .into_bound(py)
                .into_any()
            } else {
                py.None().into_bound(py)
            };
            let job = (&slf).into_pyobject(py)?;
            Ok(slf
                .queue
                .bind(py)
                .call_method1(
                    "run",
                    (
                        transfer.getattr("identifier")?,
                        client,
                        slf.link.bind(py),
                        transfer.getattr("cancel")?,
                        job.getattr("progress")?,
                    ),
                )?
                .unbind())
        })();
        if private.is_truthy()? {
            private.call_method0("forget")?;
        }
        result
    }

    fn finish(&self, py: Python<'_>, future: &Bound<'_, PyAny>) -> PyResult<()> {
        let output = self.transfer.bind(py).getattr("future")?;
        match future.call_method0("result") {
            Ok(result) => {
                output.call_method1("set_result", (result,))?;
            }
            Err(error) => {
                output.call_method1("set_exception", (error.value(py),))?;
            }
        }
        Ok(())
    }

    fn prepare(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<()> {
        let transfer = slf.transfer.bind(py);
        if !transfer.getattr("identifier")?.is_truthy()? {
            transfer.setattr(
                "identifier",
                slf.queue
                    .bind(py)
                    .call_method1("enqueue", (slf.link.bind(py),))?,
            )?;
        }
        let job = (&slf).into_pyobject(py)?;
        let running = slf
            .state
            .bind(py)
            .getattr("_pool")?
            .call_method1("submit", (job.getattr("work")?,))?;
        running.call_method1("add_done_callback", (job.getattr("finish")?,))?;
        Ok(())
    }

    fn enqueue_finished(&self, py: Python<'_>, future: &Bound<'_, PyAny>) -> PyResult<()> {
        if let Err(error) = future.call_method0("result") {
            self.transfer
                .bind(py)
                .getattr("future")?
                .call_method1("set_exception", (error.value(py),))?;
        }
        Ok(())
    }
}

#[pyfunction]
fn scheduler_add(
    state: &Bound<'_, PyAny>,
    client: &Bound<'_, PyAny>,
    link: &Bound<'_, PyAny>,
    identifier: &str,
) -> PyResult<Py<PyAny>> {
    let py = state.py();
    if state.getattr("_closed")?.is_truthy()? || (!link.is_truthy()? && identifier.is_empty()) {
        return Err(invalid("Open the transfer queue and choose a Nexus file."));
    }
    let queue = py
        .import("creation_lib.mod_manager.download_queue")?
        .getattr("DownloadQueue")?
        .call1((state.getattr("library")?, state.getattr("observer")?))?;
    let job = if identifier.is_empty() {
        py.None().into_bound(py)
    } else {
        queue.call_method1("get", (identifier,))?
    };
    let identity = if job.is_truthy()? {
        PyTuple::new(
            py,
            [
                job.get_item("game")?,
                job.get_item("mod_id")?,
                job.get_item("file_id")?,
            ],
        )?
        .into_any()
    } else {
        link.getattr("identity")?
    };
    if has(state, &identity)? {
        return Err(invalid(
            "This file already has an active or queued transfer.",
        ));
    }
    let transfers = state.getattr("transfers")?;
    if transfers.len()? >= 100 {
        return Err(invalid(
            "Background transfer requests reached their 100-file limit.",
        ));
    }
    let future = py
        .import("concurrent.futures")?
        .getattr("Future")?
        .call0()?;
    let cancel = py.import("threading")?.getattr("Event")?.call0()?;
    let transfer = py
        .import("creation_lib.mod_manager.download_scheduler")?
        .getattr("Transfer")?
        .call1((&identity, future, cancel, identifier))?;
    let job = Py::new(
        py,
        TransferJob {
            state: state.clone().unbind(),
            queue: queue.unbind(),
            client: client.clone().unbind(),
            link: link.clone().unbind(),
            transfer: transfer.clone().unbind(),
        },
    )?
    .into_bound(py);
    transfers.set_item(&identity, &transfer)?;
    let result = (|| -> PyResult<()> {
        let queued = state
            .getattr("_enqueue_pool")?
            .call_method1("submit", (job.getattr("prepare")?,))?;
        queued.call_method1("add_done_callback", (job.getattr("enqueue_finished")?,))?;
        Ok(())
    })();
    if let Err(error) = result {
        transfers.call_method1("pop", (&identity, py.None()))?;
        return Err(error);
    }
    Ok(transfer.unbind())
}

#[pyfunction]
fn scheduler_pause(state: &Bound<'_, PyAny>, identity: &Bound<'_, PyAny>) -> PyResult<()> {
    for transfer in state
        .getattr("transfers")?
        .call_method0("values")?
        .try_iter()?
    {
        let transfer = transfer?;
        if identity.is_none()
            || transfer.getattr("identity")?.eq(identity)?
            || transfer.getattr("identifier")?.eq(identity)?
        {
            transfer.getattr("cancel")?.call_method0("set")?;
        }
    }
    Ok(())
}

#[pyfunction]
fn scheduler_poll(state: &Bound<'_, PyAny>) -> PyResult<Vec<Py<PyAny>>> {
    let py = state.py();
    let mut outcomes = vec![];
    let transfers = state.getattr("transfers")?;
    let items: Vec<_> = transfers
        .call_method0("items")?
        .try_iter()?
        .collect::<PyResult<_>>()?;
    let paths = py.import("creation_lib.mod_manager.paths")?;
    let cancelled = paths.getattr("Cancelled")?;
    let manager_error = paths.getattr("ManagerError")?;
    let nexus_error = py
        .import("creation_lib.mod_manager.nexus")?
        .getattr("NexusError")?;
    for item in items {
        let identity = item.get_item(0)?;
        let future = item.get_item(1)?.getattr("future")?;
        if !future.call_method0("done")?.is_truthy()? {
            continue;
        }
        transfers.del_item(&identity)?;
        let (state, detail) = match future.call_method0("result") {
            Ok(value) => ("complete", value),
            Err(error)
                if error.matches(py, &cancelled)?
                    || error.is_instance_of::<crate::OperationCancelled>(py) =>
            {
                (
                    "paused",
                    "Background download paused; its checkpoint is retained."
                        .into_pyobject(py)?
                        .into_any(),
                )
            }
            Err(error)
                if error.matches(py, &manager_error)? || error.matches(py, &nexus_error)? =>
            {
                ("failed", error.value(py).str()?.into_any())
            }
            Err(error) if error.is_instance_of::<pyo3::exceptions::PyException>(py) => (
                "failed",
                "Background transfer failed. Retained data needs review/resume."
                    .into_pyobject(py)?
                    .into_any(),
            ),
            Err(error) => return Err(error),
        };
        outcomes.push(
            (identity, state, detail)
                .into_pyobject(py)?
                .into_any()
                .unbind(),
        );
    }
    Ok(outcomes)
}

#[pyfunction]
fn scheduler_close(state: &Bound<'_, PyAny>) -> PyResult<()> {
    state.setattr("_closed", true)?;
    scheduler_pause(state, &state.py().None().into_bound(state.py()))?;
    state
        .getattr("_enqueue_pool")?
        .call_method1("shutdown", (true,))?;
    state.getattr("_pool")?.call_method1("shutdown", (true,))?;
    scheduler_poll(state)?;
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(scheduler_init, m)?)?;
    m.add_function(wrap_pyfunction!(scheduler_has, m)?)?;
    m.add_function(wrap_pyfunction!(scheduler_add, m)?)?;
    m.add_function(wrap_pyfunction!(scheduler_pause, m)?)?;
    m.add_function(wrap_pyfunction!(scheduler_poll, m)?)?;
    m.add_function(wrap_pyfunction!(scheduler_close, m)?)?;
    Ok(())
}
