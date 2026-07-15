use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use pyo3::exceptions::{PyIOError, PyKeyError, PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyModule, PyString};
use rusqlite::types::Value as SqlValue;

use crate::bulk::BulkInserter;
use crate::database::Database;
use crate::dir_index::DirectoryIndex;
use crate::embedder::Embedder;
use crate::error::DbError;
use crate::nif_indexer::{NifIndexTask, index_nifs_to_bulk};
use crate::query::RowMap;
use crate::records_indexer::index_records_to_bulk;

type DatabaseHandle = Arc<Mutex<Database>>;
type BulkHandle = Arc<Mutex<BulkInserter>>;
type DirectoryHandle = Arc<Mutex<DirectoryIndex>>;
type EmbedderHandle = Arc<Embedder>;

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
static DATABASES: Lazy<Mutex<HashMap<u64, DatabaseHandle>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static BULK_INSERTS: Lazy<Mutex<HashMap<u64, BulkHandle>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static DIRECTORY_INDEXES: Lazy<Mutex<HashMap<u64, DirectoryHandle>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static EMBEDDERS: Lazy<Mutex<HashMap<u64, EmbedderHandle>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn db_err_to_py(err: DbError) -> PyErr {
    match err {
        DbError::NotFound(msg) => PyRuntimeError::new_err(msg),
        DbError::BadFilter { .. } | DbError::BadKey { .. } | DbError::BadGroup { .. } => {
            PyValueError::new_err(err.to_string())
        }
        DbError::UnknownTable(_) => PyKeyError::new_err(err.to_string()),
        DbError::ColumnLengthMismatch { .. } => PyValueError::new_err(err.to_string()),
        DbError::BulkState(_) => PyRuntimeError::new_err(err.to_string()),
        DbError::Schema(_) => PyValueError::new_err(err.to_string()),
        DbError::Io(_) => PyIOError::new_err(err.to_string()),
        DbError::Json(_) => PyValueError::new_err(err.to_string()),
        DbError::Sqlite(_) => PyRuntimeError::new_err(err.to_string()),
        DbError::Other(_) => PyRuntimeError::new_err(err.to_string()),
    }
}

fn handle_id() -> u64 {
    NEXT_HANDLE.fetch_add(1, Ordering::Relaxed)
}

fn store_database(db: Database) -> PyResult<u64> {
    let id = handle_id();
    let mut guard = DATABASES
        .lock()
        .map_err(|_| PyRuntimeError::new_err("database registry poisoned"))?;
    guard.insert(id, Arc::new(Mutex::new(db)));
    Ok(id)
}

fn store_bulk(bulk: BulkInserter) -> PyResult<u64> {
    let id = handle_id();
    let mut guard = BULK_INSERTS
        .lock()
        .map_err(|_| PyRuntimeError::new_err("bulk registry poisoned"))?;
    guard.insert(id, Arc::new(Mutex::new(bulk)));
    Ok(id)
}

fn store_directory(index: DirectoryIndex) -> PyResult<u64> {
    let id = handle_id();
    let mut guard = DIRECTORY_INDEXES
        .lock()
        .map_err(|_| PyRuntimeError::new_err("directory registry poisoned"))?;
    guard.insert(id, Arc::new(Mutex::new(index)));
    Ok(id)
}

fn get_database(handle: u64) -> PyResult<DatabaseHandle> {
    DATABASES
        .lock()
        .map_err(|_| PyRuntimeError::new_err("database registry poisoned"))?
        .get(&handle)
        .cloned()
        .ok_or_else(|| PyRuntimeError::new_err(format!("unknown database handle {handle}")))
}

fn get_bulk(handle: u64) -> PyResult<BulkHandle> {
    BULK_INSERTS
        .lock()
        .map_err(|_| PyRuntimeError::new_err("bulk registry poisoned"))?
        .get(&handle)
        .cloned()
        .ok_or_else(|| PyRuntimeError::new_err(format!("unknown bulk handle {handle}")))
}

fn get_directory(handle: u64) -> PyResult<DirectoryHandle> {
    DIRECTORY_INDEXES
        .lock()
        .map_err(|_| PyRuntimeError::new_err("directory registry poisoned"))?
        .get(&handle)
        .cloned()
        .ok_or_else(|| PyRuntimeError::new_err(format!("unknown directory handle {handle}")))
}

fn store_embedder(embedder: Embedder) -> PyResult<u64> {
    let id = handle_id();
    let mut guard = EMBEDDERS
        .lock()
        .map_err(|_| PyRuntimeError::new_err("embedder registry poisoned"))?;
    guard.insert(id, Arc::new(embedder));
    Ok(id)
}

fn get_embedder(handle: u64) -> PyResult<EmbedderHandle> {
    EMBEDDERS
        .lock()
        .map_err(|_| PyRuntimeError::new_err("embedder registry poisoned"))?
        .get(&handle)
        .cloned()
        .ok_or_else(|| PyRuntimeError::new_err(format!("unknown embedder handle {handle}")))
}

fn sqlvalue_from_py(obj: &Bound<'_, PyAny>) -> PyResult<SqlValue> {
    if obj.is_none() {
        return Ok(SqlValue::Null);
    }
    if let Ok(b) = obj.cast::<PyBool>() {
        return Ok(SqlValue::Integer(if b.is_true() { 1 } else { 0 }));
    }
    if let Ok(i) = obj.cast::<PyInt>() {
        let v: i64 = i.extract()?;
        return Ok(SqlValue::Integer(v));
    }
    if let Ok(f) = obj.cast::<PyFloat>() {
        let v: f64 = f.extract()?;
        return Ok(SqlValue::Real(v));
    }
    if let Ok(s) = obj.cast::<PyString>() {
        return Ok(SqlValue::Text(s.to_str()?.to_owned()));
    }
    if let Ok(b) = obj.cast::<PyBytes>() {
        return Ok(SqlValue::Blob(b.as_bytes().to_vec()));
    }
    if let Ok(s) = obj.extract::<String>() {
        return Ok(SqlValue::Text(s));
    }
    Err(PyTypeError::new_err(format!(
        "unsupported value type for db native binding: {}",
        obj.get_type().name()?
    )))
}

fn params_from_py(py: Python<'_>, params: Option<Vec<Py<PyAny>>>) -> PyResult<Vec<SqlValue>> {
    let Some(params) = params else {
        return Ok(Vec::new());
    };
    params
        .iter()
        .map(|obj| sqlvalue_from_py(obj.bind(py)))
        .collect()
}

fn columnar_from_py(columns: &Bound<'_, PyDict>) -> PyResult<HashMap<String, Vec<SqlValue>>> {
    let mut out = HashMap::new();
    for (key, value) in columns.iter() {
        let col_name: String = key.extract()?;
        let list = value
            .cast::<PyList>()
            .map_err(|_| PyTypeError::new_err(format!("column '{col_name}' must be a list")))?;
        let values = list
            .iter()
            .map(|item| sqlvalue_from_py(&item))
            .collect::<PyResult<Vec<_>>>()?;
        out.insert(col_name, values);
    }
    Ok(out)
}

fn rowdicts_from_py(
    py: Python<'_>,
    rows: Vec<Py<PyDict>>,
) -> PyResult<Vec<HashMap<String, SqlValue>>> {
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let dict = row.bind(py);
        let mut mapped = HashMap::new();
        for (key, value) in dict.iter() {
            mapped.insert(key.extract()?, sqlvalue_from_py(&value)?);
        }
        out.push(mapped);
    }
    Ok(out)
}

fn rowmaps_from_py(py: Python<'_>, rows: Vec<Py<PyDict>>) -> PyResult<Vec<RowMap>> {
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let dict = row.bind(py);
        let mut mapped = RowMap::new();
        for (key, value) in dict.iter() {
            mapped.push((key.extract()?, sqlvalue_from_py(&value)?));
        }
        out.push(mapped);
    }
    Ok(out)
}

fn sqlval_to_py<'py>(py: Python<'py>, v: &SqlValue) -> Py<PyAny> {
    match v {
        SqlValue::Null => py.None(),
        SqlValue::Integer(i) => (*i).into_pyobject(py).unwrap().into_any().unbind(),
        SqlValue::Real(f) => (*f).into_pyobject(py).unwrap().into_any().unbind(),
        SqlValue::Text(t) => t.as_str().into_pyobject(py).unwrap().into_any().unbind(),
        SqlValue::Blob(b) => PyBytes::new(py, b).into_any().unbind(),
    }
}

fn rowmap_to_pydict<'py>(py: Python<'py>, row: RowMap) -> PyResult<Py<PyDict>> {
    let dict = PyDict::new(py);
    for (key, value) in row {
        dict.set_item(key, sqlval_to_py(py, &value))?;
    }
    Ok(dict.unbind())
}

fn rowmaps_to_pylist<'py>(py: Python<'py>, rows: Vec<RowMap>) -> PyResult<Py<PyList>> {
    let list = PyList::empty(py);
    for row in rows {
        list.append(rowmap_to_pydict(py, row)?)?;
    }
    Ok(list.unbind())
}

fn rowmap_hash_to_pydict<'py>(
    py: Python<'py>,
    rows: HashMap<String, RowMap>,
) -> PyResult<Py<PyDict>> {
    let dict = PyDict::new(py);
    for (key, row) in rows {
        dict.set_item(key, rowmap_to_pydict(py, row)?)?;
    }
    Ok(dict.unbind())
}

fn counted_to_pydict<'py>(py: Python<'py>, rows: Vec<(SqlValue, i64)>) -> PyResult<Py<PyDict>> {
    let dict = PyDict::new(py);
    for (key, count) in rows {
        dict.set_item(sqlval_to_py(py, &key), count)?;
    }
    Ok(dict.unbind())
}

fn value_rows_to_pylist<'py>(py: Python<'py>, rows: Vec<Vec<SqlValue>>) -> PyResult<Py<PyList>> {
    let list = PyList::empty(py);
    for row in rows {
        let tuple = pyo3::types::PyTuple::new(py, row.iter().map(|v| sqlval_to_py(py, v)))?;
        list.append(tuple)?;
    }
    Ok(list.unbind())
}

#[pyfunction]
#[pyo3(signature = (path, mode="rw", load_vec=false))]
fn database_open(py: Python<'_>, path: &str, mode: &str, load_vec: bool) -> PyResult<u64> {
    let path = path.to_string();
    let mode = mode.to_string();
    let db = py
        .detach(move || Database::open(&path, &mode, load_vec))
        .map_err(db_err_to_py)?;
    store_database(db)
}

#[pyfunction]
fn database_path(handle: u64) -> PyResult<String> {
    let db = get_database(handle)?;
    let guard = db
        .lock()
        .map_err(|_| PyRuntimeError::new_err("database handle poisoned"))?;
    Ok(guard.path())
}

#[pyfunction]
fn database_is_write(handle: u64) -> PyResult<bool> {
    let db = get_database(handle)?;
    let guard = db
        .lock()
        .map_err(|_| PyRuntimeError::new_err("database handle poisoned"))?;
    Ok(guard.is_write())
}

#[pyfunction]
fn database_execute(py: Python<'_>, handle: u64, sql: &str) -> PyResult<()> {
    let db = get_database(handle)?;
    let sql = sql.to_string();
    py.detach(move || {
        let guard = db
            .lock()
            .map_err(|_| DbError::BulkState("database handle poisoned".into()))?;
        guard.execute(&sql)
    })
    .map_err(db_err_to_py)
}

#[pyfunction]
#[pyo3(signature = (handle, sql, params=None))]
fn database_execute_one(
    py: Python<'_>,
    handle: u64,
    sql: &str,
    params: Option<Vec<Py<PyAny>>>,
) -> PyResult<usize> {
    let db = get_database(handle)?;
    let sql = sql.to_string();
    let params = params_from_py(py, params)?;
    py.detach(move || {
        let guard = db
            .lock()
            .map_err(|_| DbError::BulkState("database handle poisoned".into()))?;
        guard.execute_one(&sql, params)
    })
    .map_err(db_err_to_py)
}

#[pyfunction]
#[pyo3(signature = (handle, sql, params=None))]
fn database_query_all(
    py: Python<'_>,
    handle: u64,
    sql: &str,
    params: Option<Vec<Py<PyAny>>>,
) -> PyResult<Py<PyList>> {
    let db = get_database(handle)?;
    let sql = sql.to_string();
    let params = params_from_py(py, params)?;
    let rows = py
        .detach(move || {
            let guard = db
                .lock()
                .map_err(|_| DbError::BulkState("database handle poisoned".into()))?;
            guard.query_all(&sql, params)
        })
        .map_err(db_err_to_py)?;
    rowmaps_to_pylist(py, rows)
}

#[pyfunction]
fn database_close(py: Python<'_>, handle: u64) -> PyResult<()> {
    let db = DATABASES
        .lock()
        .map_err(|_| PyRuntimeError::new_err("database registry poisoned"))?
        .remove(&handle);
    if let Some(db) = db {
        py.detach(move || {
            let mut guard = db
                .lock()
                .map_err(|_| DbError::BulkState("database handle poisoned".into()))?;
            guard.close()
        })
        .map_err(db_err_to_py)?;
    }
    Ok(())
}

#[pyfunction]
#[pyo3(signature = (db_path, schema_json, *, fresh=false, load_vec=false))]
fn bulk_inserter_new(
    py: Python<'_>,
    db_path: &str,
    schema_json: &str,
    fresh: bool,
    load_vec: bool,
) -> PyResult<u64> {
    let db_path = db_path.to_string();
    let schema_json = schema_json.to_string();
    let bulk = py
        .detach(move || BulkInserter::new(&db_path, &schema_json, fresh, load_vec))
        .map_err(db_err_to_py)?;
    store_bulk(bulk)
}

#[pyfunction]
fn bulk_execute(py: Python<'_>, handle: u64, sql: &str) -> PyResult<()> {
    let bulk = get_bulk(handle)?;
    let sql = sql.to_string();
    py.detach(move || {
        let mut guard = bulk
            .lock()
            .map_err(|_| DbError::BulkState("bulk handle poisoned".into()))?;
        guard.execute(&sql)
    })
    .map_err(db_err_to_py)
}

#[pyfunction]
#[pyo3(signature = (handle, sql, params=None))]
fn bulk_execute_params(
    py: Python<'_>,
    handle: u64,
    sql: &str,
    params: Option<Vec<Py<PyAny>>>,
) -> PyResult<usize> {
    let bulk = get_bulk(handle)?;
    let sql = sql.to_string();
    let params = params_from_py(py, params)?;
    py.detach(move || {
        let mut guard = bulk
            .lock()
            .map_err(|_| DbError::BulkState("bulk handle poisoned".into()))?;
        guard.execute_params(&sql, params)
    })
    .map_err(db_err_to_py)
}

#[pyfunction]
fn bulk_add_chunk(
    py: Python<'_>,
    handle: u64,
    table: &str,
    columns: &Bound<'_, PyDict>,
) -> PyResult<usize> {
    let bulk = get_bulk(handle)?;
    let table = table.to_string();
    let columns = columnar_from_py(columns)?;
    py.detach(move || {
        let mut guard = bulk
            .lock()
            .map_err(|_| DbError::BulkState("bulk handle poisoned".into()))?;
        guard.add_chunk(&table, columns)
    })
    .map_err(db_err_to_py)
}

#[pyfunction]
#[pyo3(signature = (handle, tasks, source, source_path, max_size=5_242_880, workers=0, timing_log_path=None))]
fn bulk_index_nifs(
    py: Python<'_>,
    handle: u64,
    tasks: Vec<(String, String)>,
    source: &str,
    source_path: &str,
    max_size: u64,
    workers: usize,
    timing_log_path: Option<String>,
) -> PyResult<Py<PyAny>> {
    let bulk = get_bulk(handle)?;
    let tasks = tasks
        .into_iter()
        .map(|(abs_path, rel_path)| NifIndexTask { abs_path, rel_path })
        .collect::<Vec<_>>();
    let source = source.to_string();
    let source_path = source_path.to_string();
    let summary = py
        .detach(move || {
            let mut guard = bulk
                .lock()
                .map_err(|_| DbError::BulkState("bulk handle poisoned".into()))?;
            index_nifs_to_bulk(
                &mut guard,
                tasks,
                &source,
                &source_path,
                max_size,
                workers,
                timing_log_path.as_deref(),
            )
        })
        .map_err(db_err_to_py)?;

    let d = PyDict::new(py);
    d.set_item("indexed", summary.indexed)?;
    d.set_item("errors", summary.errors)?;
    d.set_item("skipped", summary.skipped)?;
    d.set_item("elapsed_seconds", summary.elapsed_seconds)?;
    d.set_item("category_counts", summary.category_counts)?;
    Ok(d.into_any().unbind())
}

#[pyfunction]
#[pyo3(signature = (handle, yaml_root, sources=None, workers=0))]
fn bulk_index_records(
    py: Python<'_>,
    handle: u64,
    yaml_root: &str,
    sources: Option<Vec<String>>,
    workers: usize,
) -> PyResult<Py<PyAny>> {
    let bulk = get_bulk(handle)?;
    let yaml_root = std::path::PathBuf::from(yaml_root);
    let sources_filter: Option<std::collections::HashSet<String>> =
        sources.map(|v| v.into_iter().collect());
    let summary = py
        .detach(move || {
            let mut guard = bulk
                .lock()
                .map_err(|_| DbError::BulkState("bulk handle poisoned".into()))?;
            index_records_to_bulk(&mut guard, &yaml_root, sources_filter.as_ref(), workers)
        })
        .map_err(db_err_to_py)?;

    let d = PyDict::new(py);
    d.set_item("indexed", summary.indexed)?;
    d.set_item("refs", summary.refs)?;
    d.set_item("elapsed_seconds", summary.elapsed_seconds)?;
    d.set_item("type_counts", summary.type_counts)?;
    d.set_item("source_counts", summary.source_counts)?;
    Ok(d.into_any().unbind())
}

#[pyfunction]
fn bulk_add_rows(
    py: Python<'_>,
    handle: u64,
    table: &str,
    rows: Vec<Py<PyDict>>,
) -> PyResult<usize> {
    let bulk = get_bulk(handle)?;
    let table = table.to_string();
    let rows = rowdicts_from_py(py, rows)?;
    py.detach(move || {
        let mut guard = bulk
            .lock()
            .map_err(|_| DbError::BulkState("bulk handle poisoned".into()))?;
        guard.add_rows(&table, rows)
    })
    .map_err(db_err_to_py)
}

#[pyfunction]
#[pyo3(signature = (handle, sql, params=None))]
fn bulk_query_all(
    py: Python<'_>,
    handle: u64,
    sql: &str,
    params: Option<Vec<Py<PyAny>>>,
) -> PyResult<Py<PyList>> {
    let bulk = get_bulk(handle)?;
    let sql = sql.to_string();
    let params = params_from_py(py, params)?;
    let rows = py
        .detach(move || {
            let mut guard = bulk
                .lock()
                .map_err(|_| DbError::BulkState("bulk handle poisoned".into()))?;
            guard.query_all(&sql, params)
        })
        .map_err(db_err_to_py)?;
    value_rows_to_pylist(py, rows)
}

#[pyfunction]
fn bulk_rebuild_fts(py: Python<'_>, handle: u64, fts_table: &str) -> PyResult<()> {
    let bulk = get_bulk(handle)?;
    let fts_table = fts_table.to_string();
    py.detach(move || {
        let mut guard = bulk
            .lock()
            .map_err(|_| DbError::BulkState("bulk handle poisoned".into()))?;
        guard.rebuild_fts(&fts_table)
    })
    .map_err(db_err_to_py)
}

#[pyfunction]
fn bulk_rebuild_records_fts(py: Python<'_>, handle: u64) -> PyResult<()> {
    let bulk = get_bulk(handle)?;
    py.detach(move || {
        let mut guard = bulk
            .lock()
            .map_err(|_| DbError::BulkState("bulk handle poisoned".into()))?;
        guard.rebuild_records_fts()
    })
    .map_err(db_err_to_py)
}

#[pyfunction]
fn bulk_create_indexes(py: Python<'_>, handle: u64, sql: &str) -> PyResult<()> {
    let bulk = get_bulk(handle)?;
    let sql = sql.to_string();
    py.detach(move || {
        let mut guard = bulk
            .lock()
            .map_err(|_| DbError::BulkState("bulk handle poisoned".into()))?;
        guard.create_indexes(&sql)
    })
    .map_err(db_err_to_py)
}

#[pyfunction]
fn bulk_commit(py: Python<'_>, handle: u64) -> PyResult<()> {
    let bulk = get_bulk(handle)?;
    py.detach(move || {
        let mut guard = bulk
            .lock()
            .map_err(|_| DbError::BulkState("bulk handle poisoned".into()))?;
        guard.commit()
    })
    .map_err(db_err_to_py)
}

#[pyfunction]
fn bulk_finalize(py: Python<'_>, handle: u64) -> PyResult<usize> {
    let bulk = get_bulk(handle)?;
    py.detach(move || {
        let mut guard = bulk
            .lock()
            .map_err(|_| DbError::BulkState("bulk handle poisoned".into()))?;
        guard.finalize()
    })
    .map_err(db_err_to_py)
}

#[pyfunction]
fn bulk_rollback(py: Python<'_>, handle: u64) -> PyResult<()> {
    let bulk = get_bulk(handle)?;
    py.detach(move || {
        let mut guard = bulk
            .lock()
            .map_err(|_| DbError::BulkState("bulk handle poisoned".into()))?;
        guard.rollback()
    })
    .map_err(db_err_to_py)
}

#[pyfunction]
fn bulk_rows_inserted(handle: u64) -> PyResult<usize> {
    let bulk = get_bulk(handle)?;
    let guard = bulk
        .lock()
        .map_err(|_| PyRuntimeError::new_err("bulk handle poisoned"))?;
    Ok(guard.rows_inserted())
}

#[pyfunction]
fn bulk_close(py: Python<'_>, handle: u64) -> PyResult<()> {
    let bulk = BULK_INSERTS
        .lock()
        .map_err(|_| PyRuntimeError::new_err("bulk registry poisoned"))?
        .remove(&handle);
    if let Some(bulk) = bulk {
        py.detach(move || {
            let mut guard = bulk
                .lock()
                .map_err(|_| DbError::BulkState("bulk handle poisoned".into()))?;
            guard.rollback()
        })
        .map_err(db_err_to_py)?;
    }
    Ok(())
}

#[pyfunction]
#[pyo3(signature = (root, *, cache_dir=None))]
fn dir_index_new(py: Python<'_>, root: &str, cache_dir: Option<&str>) -> PyResult<u64> {
    let root = root.to_string();
    let cache_dir = cache_dir.map(str::to_string);
    let index = py
        .detach(move || DirectoryIndex::new(&root, cache_dir.as_deref()))
        .map_err(db_err_to_py)?;
    store_directory(index)
}

#[pyfunction]
fn dir_index_resolve(py: Python<'_>, handle: u64, rel_path: &str) -> PyResult<Option<String>> {
    let index = get_directory(handle)?;
    let rel_path = rel_path.to_string();
    py.detach(move || {
        let guard = index
            .lock()
            .map_err(|_| DbError::BulkState("directory handle poisoned".into()))?;
        guard.resolve(&rel_path)
    })
    .map_err(db_err_to_py)
}

#[pyfunction]
fn dir_index_contains(py: Python<'_>, handle: u64, rel_path: &str) -> PyResult<bool> {
    let index = get_directory(handle)?;
    let rel_path = rel_path.to_string();
    py.detach(move || {
        let guard = index
            .lock()
            .map_err(|_| DbError::BulkState("directory handle poisoned".into()))?;
        guard.contains(&rel_path)
    })
    .map_err(db_err_to_py)
}

#[pyfunction]
fn dir_index_file_count(handle: u64) -> PyResult<usize> {
    let index = get_directory(handle)?;
    let guard = index
        .lock()
        .map_err(|_| PyRuntimeError::new_err("directory handle poisoned"))?;
    guard.file_count().map_err(db_err_to_py)
}

#[pyfunction]
fn dir_index_lookup(handle: u64) -> PyResult<HashMap<String, String>> {
    let index = get_directory(handle)?;
    let guard = index
        .lock()
        .map_err(|_| PyRuntimeError::new_err("directory handle poisoned"))?;
    guard.lookup_snapshot().map_err(db_err_to_py)
}

#[pyfunction]
fn dir_index_close(handle: u64) -> PyResult<()> {
    DIRECTORY_INDEXES
        .lock()
        .map_err(|_| PyRuntimeError::new_err("directory registry poisoned"))?
        .remove(&handle);
    Ok(())
}

#[pyfunction]
#[pyo3(signature = (db_path, table, fts_table, query, filters=None, max_results=10, columns="t.*", search_columns=None, offset=0))]
fn fts_search(
    py: Python<'_>,
    db_path: &str,
    table: &str,
    fts_table: &str,
    query: &str,
    filters: Option<HashMap<String, String>>,
    max_results: i64,
    columns: &str,
    search_columns: Option<Vec<String>>,
    offset: i64,
) -> PyResult<Py<PyList>> {
    let db_path = db_path.to_string();
    let table = table.to_string();
    let fts_table = fts_table.to_string();
    let query = query.to_string();
    let columns = columns.to_string();
    let rows = py
        .detach(move || {
            crate::query::fts_search(
                &db_path,
                &table,
                &fts_table,
                &query,
                filters,
                max_results,
                &columns,
                search_columns,
                offset,
            )
        })
        .map_err(db_err_to_py)?;
    rowmaps_to_pylist(py, rows)
}

#[pyfunction]
fn exact_lookup(
    py: Python<'_>,
    db_path: &str,
    table: &str,
    key_column: &str,
    key_value: &str,
) -> PyResult<Option<Py<PyDict>>> {
    let db_path = db_path.to_string();
    let table = table.to_string();
    let key_column = key_column.to_string();
    let key_value = key_value.to_string();
    let row = py
        .detach(move || crate::query::exact_lookup(&db_path, &table, &key_column, &key_value))
        .map_err(db_err_to_py)?;
    row.map(|r| rowmap_to_pydict(py, r)).transpose()
}

#[pyfunction]
#[pyo3(signature = (db_path, table, key_column, key_values, columns="*"))]
fn batch_lookup(
    py: Python<'_>,
    db_path: &str,
    table: &str,
    key_column: &str,
    key_values: Vec<String>,
    columns: &str,
) -> PyResult<Py<PyDict>> {
    let db_path = db_path.to_string();
    let table = table.to_string();
    let key_column = key_column.to_string();
    let columns = columns.to_string();
    let rows = py
        .detach(move || {
            crate::query::batch_lookup(&db_path, &table, &key_column, key_values, &columns)
        })
        .map_err(db_err_to_py)?;
    rowmap_hash_to_pydict(py, rows)
}

#[pyfunction]
fn count_by_column(
    py: Python<'_>,
    db_path: &str,
    table: &str,
    column: &str,
) -> PyResult<Py<PyDict>> {
    let db_path = db_path.to_string();
    let table = table.to_string();
    let column = column.to_string();
    let rows = py
        .detach(move || crate::query::count_by_column(&db_path, &table, &column))
        .map_err(db_err_to_py)?;
    counted_to_pydict(py, rows)
}

#[pyfunction]
#[pyo3(signature = (db_path, table, fts_table, query, existing_hits, id_column, filters=None, max_results=10, search_columns=None))]
fn fallback_word_search(
    py: Python<'_>,
    db_path: &str,
    table: &str,
    fts_table: &str,
    query: &str,
    existing_hits: Vec<Py<PyDict>>,
    id_column: &str,
    filters: Option<HashMap<String, String>>,
    max_results: i64,
    search_columns: Option<Vec<String>>,
) -> PyResult<Py<PyList>> {
    let db_path = db_path.to_string();
    let table = table.to_string();
    let fts_table = fts_table.to_string();
    let query = query.to_string();
    let existing_hits = rowmaps_from_py(py, existing_hits)?;
    let id_column = id_column.to_string();
    let rows = py
        .detach(move || {
            crate::query::fallback_word_search(
                &db_path,
                &table,
                &fts_table,
                &query,
                existing_hits,
                &id_column,
                filters,
                max_results,
                search_columns,
            )
        })
        .map_err(db_err_to_py)?;
    rowmaps_to_pylist(py, rows)
}

#[pyfunction]
fn fts5_escape(query: &str) -> String {
    crate::fts5::fts5_escape(query)
}

#[pyfunction]
fn tokenize(text: &str) -> String {
    crate::tokenizer::tokenize(text)
}

#[pyfunction]
fn vec_create_table(py: Python<'_>, db_path: &str, table_name: &str, dim: usize) -> PyResult<()> {
    let db_path = db_path.to_string();
    let table_name = table_name.to_string();
    py.detach(move || crate::embeddings::vec_create_table(&db_path, &table_name, dim))
        .map_err(db_err_to_py)
}

#[pyfunction]
fn vec_drop_table(py: Python<'_>, db_path: &str, table_name: &str) -> PyResult<()> {
    let db_path = db_path.to_string();
    let table_name = table_name.to_string();
    py.detach(move || crate::embeddings::vec_drop_table(&db_path, &table_name))
        .map_err(db_err_to_py)
}

#[pyfunction]
fn vec_bulk_insert(
    py: Python<'_>,
    db_path: &str,
    table_name: &str,
    doc_ids: Vec<String>,
    vectors_bytes: &Bound<'_, PyBytes>,
    dim: usize,
) -> PyResult<usize> {
    let db_path = db_path.to_string();
    let table_name = table_name.to_string();
    let vectors_bytes = vectors_bytes.as_bytes().to_vec();
    py.detach(move || {
        crate::embeddings::vec_bulk_insert(&db_path, &table_name, doc_ids, vectors_bytes, dim)
    })
    .map_err(db_err_to_py)
}

#[pyfunction]
fn vec_knn_search(
    py: Python<'_>,
    db_path: &str,
    table_name: &str,
    query_bytes: &Bound<'_, PyBytes>,
    k: usize,
) -> PyResult<Vec<(String, f64)>> {
    let db_path = db_path.to_string();
    let table_name = table_name.to_string();
    let query_bytes = query_bytes.as_bytes().to_vec();
    py.detach(move || crate::embeddings::vec_knn_search(&db_path, &table_name, query_bytes, k))
        .map_err(db_err_to_py)
}

#[pyfunction]
fn embedder_new(py: Python<'_>, repo_or_path: &str) -> PyResult<u64> {
    let repo = repo_or_path.to_string();
    let embedder = py
        .detach(move || Embedder::load(&repo))
        .map_err(db_err_to_py)?;
    store_embedder(embedder)
}

#[pyfunction]
fn embedder_dim(handle: u64) -> PyResult<usize> {
    let embedder = get_embedder(handle)?;
    Ok(embedder.dim())
}

#[pyfunction]
fn embedder_embed<'py>(
    py: Python<'py>,
    handle: u64,
    texts: Vec<String>,
) -> PyResult<Bound<'py, PyBytes>> {
    let embedder = get_embedder(handle)?;
    let bytes = py
        .detach(move || embedder.encode_bytes(&texts))
        .map_err(db_err_to_py)?;
    Ok(PyBytes::new(py, &bytes))
}

#[pyfunction]
fn embedder_close(handle: u64) -> PyResult<()> {
    EMBEDDERS
        .lock()
        .map_err(|_| PyRuntimeError::new_err("embedder registry poisoned"))?
        .remove(&handle);
    Ok(())
}

#[pyfunction]
fn clear_registry() -> PyResult<()> {
    crate::registry::clear_registry().map_err(db_err_to_py)
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(database_open, m)?)?;
    m.add_function(wrap_pyfunction!(database_path, m)?)?;
    m.add_function(wrap_pyfunction!(database_is_write, m)?)?;
    m.add_function(wrap_pyfunction!(database_execute, m)?)?;
    m.add_function(wrap_pyfunction!(database_execute_one, m)?)?;
    m.add_function(wrap_pyfunction!(database_query_all, m)?)?;
    m.add_function(wrap_pyfunction!(database_close, m)?)?;
    m.add_function(wrap_pyfunction!(bulk_inserter_new, m)?)?;
    m.add_function(wrap_pyfunction!(bulk_execute, m)?)?;
    m.add_function(wrap_pyfunction!(bulk_execute_params, m)?)?;
    m.add_function(wrap_pyfunction!(bulk_add_chunk, m)?)?;
    m.add_function(wrap_pyfunction!(bulk_index_nifs, m)?)?;
    m.add_function(wrap_pyfunction!(bulk_index_records, m)?)?;
    m.add_function(wrap_pyfunction!(bulk_add_rows, m)?)?;
    m.add_function(wrap_pyfunction!(bulk_query_all, m)?)?;
    m.add_function(wrap_pyfunction!(bulk_rebuild_fts, m)?)?;
    m.add_function(wrap_pyfunction!(bulk_rebuild_records_fts, m)?)?;
    m.add_function(wrap_pyfunction!(bulk_create_indexes, m)?)?;
    m.add_function(wrap_pyfunction!(bulk_commit, m)?)?;
    m.add_function(wrap_pyfunction!(bulk_finalize, m)?)?;
    m.add_function(wrap_pyfunction!(bulk_rollback, m)?)?;
    m.add_function(wrap_pyfunction!(bulk_rows_inserted, m)?)?;
    m.add_function(wrap_pyfunction!(bulk_close, m)?)?;
    m.add_function(wrap_pyfunction!(dir_index_new, m)?)?;
    m.add_function(wrap_pyfunction!(dir_index_resolve, m)?)?;
    m.add_function(wrap_pyfunction!(dir_index_contains, m)?)?;
    m.add_function(wrap_pyfunction!(dir_index_file_count, m)?)?;
    m.add_function(wrap_pyfunction!(dir_index_lookup, m)?)?;
    m.add_function(wrap_pyfunction!(dir_index_close, m)?)?;
    m.add_function(wrap_pyfunction!(fts_search, m)?)?;
    m.add_function(wrap_pyfunction!(exact_lookup, m)?)?;
    m.add_function(wrap_pyfunction!(batch_lookup, m)?)?;
    m.add_function(wrap_pyfunction!(count_by_column, m)?)?;
    m.add_function(wrap_pyfunction!(fallback_word_search, m)?)?;
    m.add_function(wrap_pyfunction!(fts5_escape, m)?)?;
    m.add_function(wrap_pyfunction!(tokenize, m)?)?;
    m.add_function(wrap_pyfunction!(vec_bulk_insert, m)?)?;
    m.add_function(wrap_pyfunction!(vec_knn_search, m)?)?;
    m.add_function(wrap_pyfunction!(vec_create_table, m)?)?;
    m.add_function(wrap_pyfunction!(vec_drop_table, m)?)?;
    m.add_function(wrap_pyfunction!(embedder_new, m)?)?;
    m.add_function(wrap_pyfunction!(embedder_dim, m)?)?;
    m.add_function(wrap_pyfunction!(embedder_embed, m)?)?;
    m.add_function(wrap_pyfunction!(embedder_close, m)?)?;
    m.add_function(wrap_pyfunction!(clear_registry, m)?)?;
    Ok(())
}

#[pymodule]
fn db_native(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_module(m)
}
