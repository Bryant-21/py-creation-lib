use crate::{history_prune::py_path, invalid, python_io::with_context, reject_links};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::path::PathBuf;
fn runtime_active(library: &Bound<'_, PyAny>) -> PyResult<bool> {
    library
        .py()
        .import("creation_lib.mod_manager.runtime")?
        .call_method1("active_sessions", (library.getattr("root")?,))?
        .is_truthy()
}
#[pyclass]
struct LibraryDatabase {
    database: Py<PyAny>,
    runtime_locked: bool,
}
#[pymethods]
impl LibraryDatabase {
    #[new]
    fn new(library: &Bound<'_, PyAny>) -> PyResult<Self> {
        let py = library.py();
        let locked = runtime_active(library)?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("readonly", locked)?;
        let database = py
            .import("creation_lib.mod_manager.database")?
            .getattr("Database")?
            .call((library.getattr("database")?,), Some(&kwargs))?;
        Ok(Self {
            database: database.unbind(),
            runtime_locked: locked,
        })
    }
    fn __enter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(self.database.bind(py).call_method0("__enter__")?.unbind())
    }
    fn __exit__(
        &self,
        py: Python<'_>,
        kind: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
        traceback: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let db = self.database.bind(py);
        let result = db.call_method1("__exit__", (kind, value, traceback));
        db.call_method0("close")?;
        let database_error = py.import("sqlite3")?.getattr("DatabaseError")?;
        let failure = value.is_instance(&database_error)?
            || match &result {
                Err(e) => e.matches(py, &database_error)?,
                Ok(_) => false,
            };
        if self.runtime_locked && failure {
            return Err(invalid(
                "This library has a running native runtime. Close it before changing library metadata or payloads.",
            ));
        }
        result?;
        Ok(false)
    }
}
#[pyfunction]
fn library_initialize(
    library: &Bound<'_, PyAny>,
    root: &Bound<'_, PyAny>,
    create: bool,
) -> PyResult<()> {
    let py = library.py();
    let root = py
        .import("pathlib")?
        .getattr("Path")?
        .call1((root,))?
        .call_method0("absolute")?;
    let native = py
        .import("creation_lib._native")?
        .getattr("mod_manager_native")?;
    library.setattr("root", &root)?;
    library.setattr(
        "library_id",
        native.call_method1("library_open", (&root, create))?,
    )?;
    let path = root.extract::<PathBuf>()?.join("library.sqlite3");
    reject_links(&path)?;
    library.setattr("database", py_path(py, &path)?)?;
    library.setattr("last_import_repaired", 0)?;
    let owned_type = py
        .import("creation_lib.mod_manager.transactions")?
        .getattr("OwnedTransactions")?;
    if runtime_active(library)? {
        library.setattr("owned", owned_type.call1((library,))?)?;
        library.setattr("recovery_report", PyList::empty(py))?;
        return Ok(());
    }
    with_context(&library.call_method0("_db")?, |db| {
        native.call_method1("library_schema", (db.getattr("native")?,))?;
        native.call_method1("executable_migrate", (db.getattr("native")?,))?;
        Ok(())
    })?;
    let owned = owned_type.call1((library,))?;
    library.setattr("owned", &owned)?;
    let report = owned.call_method0("recover")?;
    library.setattr("recovery_report", &report)?;
    for source in [
        "plugin_file_actions",
        "save_transfer",
        "overwrite",
        "history_prune",
        "backup_prune",
        "launch_targets",
    ] {
        report.call_method1(
            "extend",
            (py.import(format!("creation_lib.mod_manager.{source}"))?
                .call_method1("recover", (library,))?,),
        )?;
    }
    Ok(())
}
#[pyfunction]
fn library_runtime_write_guard(library: &Bound<'_, PyAny>) -> PyResult<()> {
    if runtime_active(library)? {
        return Err(invalid(
            "Close this library's running runtime before changing installed payloads.",
        ));
    }
    Ok(())
}
#[pyfunction]
fn transactions_initialize(
    transactions: &Bound<'_, PyAny>,
    library: &Bound<'_, PyAny>,
) -> PyResult<()> {
    transactions.setattr("library", library)?;
    transactions.setattr("root", library.getattr("root")?)?;
    let root: PathBuf = library.getattr("root")?.extract()?;
    for name in [".transactions", ".history"] {
        let path = root.join(name);
        reject_links(&path)?;
        match std::fs::create_dir(&path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
#[pyfunction]
fn library_mutation(
    library: &Bound<'_, PyAny>,
    action: &str,
    args: &Bound<'_, pyo3::types::PyTuple>,
    options: &Bound<'_, PyDict>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    library_runtime_write_guard(library)?;
    let owned = library.getattr("owned")?;
    let result = match action {
        "install" | "uninstall" | "restore" => owned.getattr(action)?.call(args, Some(options))?,
        "reinstall" => {
            let review = args.get_item(0)?;
            let fields = options.copy()?;
            fields.set_item("review", &review)?;
            owned.getattr("install")?.call(
                (
                    args.get_item(1)?,
                    args.get_item(2)?,
                    review.getattr("name")?,
                    "",
                    args.get_item(3)?,
                    args.get_item(4)?,
                ),
                Some(&fields),
            )?
        }
        "migrate" => py
            .import("creation_lib.mod_manager.migration")?
            .getattr("SourceMigration")?
            .call1((library,))?
            .getattr("copy")?
            .call(args, Some(options))?,
        _ => return Err(invalid("Unknown library mutation.")),
    };
    Ok(result.unbind())
}

#[pyfunction]
fn library_create_profile_guarded(
    library: &Bound<'_, PyAny>,
    name: &str,
    clone: &str,
    template: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    with_context(&library.call_method0("_db")?, |db| {
        let result = py
            .import("creation_lib._native")?
            .getattr("mod_manager_native")?
            .call_method1(
                "library_create_profile",
                (
                    db.getattr("native")?,
                    name,
                    clone,
                    template,
                    cancel,
                    library.getattr("_profile_create_checkpoint")?,
                ),
            )?;
        library_runtime_write_guard(library)?;
        Ok(result.unbind())
    })
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(library_mutation, m)?)?;
    m.add_function(wrap_pyfunction!(library_create_profile_guarded, m)?)?;
    m.add_class::<LibraryDatabase>()?;
    m.add_function(wrap_pyfunction!(library_initialize, m)?)?;
    m.add_function(wrap_pyfunction!(library_runtime_write_guard, m)?)?;
    m.add_function(wrap_pyfunction!(transactions_initialize, m)?)?;
    Ok(())
}
