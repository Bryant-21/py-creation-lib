use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyTuple};
use rusqlite::{Connection, types::Value};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

pub fn error(error: rusqlite::Error) -> PyErr {
    Python::attach(|py| {
        let kind = match &error {
            rusqlite::Error::SqliteFailure(code, _)
                if code.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                "IntegrityError"
            }
            _ => "OperationalError",
        };
        match py
            .import("sqlite3")
            .and_then(|module| module.getattr(kind))
            .and_then(|class| class.call1((error.to_string(),)))
        {
            Ok(value) => PyErr::from_value(value),
            Err(error) => error,
        }
    })
}

pub fn records(
    connection: &Connection,
    sql: &str,
    parameters: impl rusqlite::Params,
) -> PyResult<Vec<serde_json::Value>> {
    let mut statement = connection.prepare(sql).map_err(error)?;
    let names: Vec<_> = statement
        .column_names()
        .iter()
        .map(|name| name.to_string())
        .collect();
    let mut rows = statement.query(parameters).map_err(error)?;
    let mut result = Vec::new();
    while let Some(row) = rows.next().map_err(error)? {
        let mut fields = serde_json::Map::new();
        for (index, name) in names.iter().enumerate() {
            let value = match row.get::<_, Value>(index).map_err(error)? {
                Value::Null => serde_json::Value::Null,
                Value::Integer(v) => serde_json::json!(v),
                Value::Real(v) => serde_json::json!(v),
                Value::Text(v) => serde_json::json!(v),
                Value::Blob(_) => {
                    return Err(crate::invalid("Unexpected binary library metadata."));
                }
            };
            fields.insert(name.clone(), value);
        }
        result.push(serde_json::Value::Object(fields));
    }
    Ok(result)
}

#[pyclass]
pub struct Database {
    connection: Mutex<Option<Connection>>,
}

pub struct ConnectionGuard<'a>(MutexGuard<'a, Option<Connection>>);

impl std::ops::Deref for ConnectionGuard<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.0
            .as_ref()
            .expect("connection checked before constructing guard")
    }
}

impl Database {
    pub fn connection(&self) -> PyResult<ConnectionGuard<'_>> {
        let guard = self
            .connection
            .try_lock()
            .map_err(|_| crate::invalid("Database connection is already in use."))?;
        if guard.is_none() {
            return Err(crate::invalid("Database connection is closed."));
        }
        Ok(ConnectionGuard(guard))
    }
}

fn sql_value(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    Ok(if value.is_none() {
        Value::Null
    } else if let Ok(value) = value.extract::<i64>() {
        Value::Integer(value)
    } else if let Ok(value) = value.extract::<f64>() {
        Value::Real(value)
    } else if let Ok(value) = value.extract::<String>() {
        Value::Text(value)
    } else if let Ok(value) = value.cast::<PyBytes>() {
        Value::Blob(value.as_bytes().to_vec())
    } else {
        return Err(pyo3::exceptions::PyTypeError::new_err(
            "Unsupported SQL parameter type.",
        ));
    })
}

fn python_value(py: Python<'_>, value: Value) -> PyResult<Py<PyAny>> {
    Ok(match value {
        Value::Null => py.None(),
        Value::Integer(value) => value.into_pyobject(py)?.into_any().unbind(),
        Value::Real(value) => value.into_pyobject(py)?.into_any().unbind(),
        Value::Text(value) => value.into_pyobject(py)?.into_any().unbind(),
        Value::Blob(value) => PyBytes::new(py, &value).into_any().unbind(),
    })
}

#[pymethods]
impl Database {
    #[new]
    #[pyo3(signature = (path, readonly=false))]
    fn new(path: PathBuf, readonly: bool) -> PyResult<Self> {
        crate::reject_links(&path)?;
        for suffix in ["-journal", "-wal", "-shm"] {
            let mut sibling = path.as_os_str().to_os_string();
            sibling.push(suffix);
            crate::reject_links(&PathBuf::from(sibling))?;
        }
        let connection = Connection::open(path).map_err(error)?;
        connection
            .busy_handler(Some(|attempt| {
                if attempt >= 1000 {
                    return false;
                }
                // The lock owner may need Python callbacks before committing.
                Python::attach(|py| {
                    py.detach(|| std::thread::sleep(std::time::Duration::from_millis(10)))
                });
                true
            }))
            .map_err(error)?;
        connection
            .execute_batch("PRAGMA foreign_keys=ON")
            .map_err(error)?;
        if readonly {
            connection
                .execute_batch("PRAGMA query_only=ON")
                .map_err(error)?;
        }
        Ok(Self {
            connection: Mutex::new(Some(connection)),
        })
    }

    #[pyo3(signature = (sql, parameters=None))]
    fn execute(
        &self,
        py: Python<'_>,
        sql: &str,
        parameters: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<(Vec<String>, Vec<Py<PyAny>>, i64, i64)> {
        let values = match parameters {
            Some(parameters) => parameters
                .try_iter()?
                .map(|value| sql_value(&value?))
                .collect::<PyResult<Vec<_>>>()?,
            None => Vec::new(),
        };
        let connection = self.connection()?;
        let command = sql
            .trim_start()
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        let dml = matches!(command.as_str(), "INSERT" | "UPDATE" | "DELETE" | "REPLACE");
        if dml && connection.is_autocommit() {
            connection.execute_batch("BEGIN").map_err(error)?;
        }
        let mut statement = connection.prepare(sql).map_err(error)?;
        let names = statement
            .column_names()
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        let mut output = Vec::new();
        if names.is_empty() {
            let changed = statement
                .execute(rusqlite::params_from_iter(values))
                .map_err(error)?;
            return Ok((
                names,
                output,
                if dml { changed as i64 } else { -1 },
                connection.last_insert_rowid(),
            ));
        }
        let mut rows = statement
            .query(rusqlite::params_from_iter(values))
            .map_err(error)?;
        while let Some(row) = rows.next().map_err(error)? {
            let values = (0..names.len())
                .map(|index| python_value(py, row.get::<_, Value>(index).map_err(error)?))
                .collect::<PyResult<Vec<_>>>()?;
            output.push(PyTuple::new(py, values)?.into_any().unbind());
        }
        Ok((names, output, -1, connection.last_insert_rowid()))
    }

    fn executescript(&self, sql: &str) -> PyResult<()> {
        self.commit()?;
        self.connection()?.execute_batch(sql).map_err(error)
    }

    fn commit(&self) -> PyResult<()> {
        let connection = self.connection()?;
        if !connection.is_autocommit() {
            connection.execute_batch("COMMIT").map_err(error)?;
        }
        Ok(())
    }

    fn rollback(&self) -> PyResult<()> {
        let connection = self.connection()?;
        if !connection.is_autocommit() {
            connection.execute_batch("ROLLBACK").map_err(error)?;
        }
        Ok(())
    }

    fn close(&self) -> PyResult<()> {
        self.connection
            .try_lock()
            .map_err(|_| crate::invalid("Database connection is already in use."))?
            .take();
        Ok(())
    }
}

pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Database>()?;
    module.add_function(wrap_pyfunction!(library_open, module)?)?;
    module.add_function(wrap_pyfunction!(library_schema, module)?)?;
    Ok(())
}

#[pyfunction]
fn library_open(root: PathBuf, create: bool) -> PyResult<String> {
    crate::reject_links(&root)?;
    let marker = root.join(".modbox21-library.json");
    crate::reject_links(&marker)?;
    if !marker.is_file() {
        if !create {
            return Err(crate::invalid(
                "Choose a manager-owned library, or create one in an empty folder.",
            ));
        }
        if root.exists() && std::fs::read_dir(&root)?.next().transpose()?.is_some() {
            return Err(crate::invalid(
                "A new library requires an empty folder; existing folders cannot be adopted.",
            ));
        }
        std::fs::create_dir_all(&root)?;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)?;
        write!(
            file,
            "{}",
            serde_json::json!({"format":1,"id":uuid::Uuid::new_v4().simple().to_string()})
        )?;
    }
    let ownership: serde_json::Value = serde_json::from_slice(&std::fs::read(&marker)?)
        .map_err(|e| crate::invalid(format!("Invalid library ownership marker: {e}")))?;
    if ownership.get("format").and_then(|v| v.as_i64()) != Some(1) {
        return Err(crate::invalid(
            "Invalid library ownership marker: Unsupported manager library version.",
        ));
    }
    let id = ownership.get("id").and_then(|v| v.as_str()).unwrap_or("");
    if id.len() != 32
        || !id
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(crate::invalid(
            "Invalid library ownership marker: Invalid owned transaction identifier.",
        ));
    }
    for name in ["mods", ".staging", "exports", "downloads"] {
        let path = root.join(name);
        crate::reject_links(&path)?;
        std::fs::create_dir_all(path)?;
    }
    Ok(id.to_string())
}

#[pyfunction]
fn library_schema(db: &Database) -> PyResult<()> {
    let connection = db.connection()?;
    connection
        .execute_batch(include_str!("library_schema.sql"))
        .map_err(error)?;
    let mut query = connection
        .prepare("PRAGMA table_info(mods)")
        .map_err(error)?;
    let columns = query
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(error)?
        .collect::<rusqlite::Result<std::collections::BTreeSet<_>>>()
        .map_err(error)?;
    for (name, definition) in [
        ("nexus_game", "TEXT NOT NULL DEFAULT ''"),
        ("nexus_mod_id", "INTEGER NOT NULL DEFAULT 0"),
        ("nexus_file_id", "INTEGER NOT NULL DEFAULT 0"),
        ("manifest", "TEXT NOT NULL DEFAULT ''"),
        ("categories_json", "TEXT NOT NULL DEFAULT ''"),
    ] {
        if !columns.contains(name) {
            connection
                .execute_batch(&format!("ALTER TABLE mods ADD COLUMN {name} {definition}"))
                .map_err(error)?;
        }
    }
    Ok(())
}
