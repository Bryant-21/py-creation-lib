use std::sync::Once;

use rusqlite::ffi;
use rusqlite::{Connection, OpenFlags};

use crate::error::{DbError, DbResult};
use crate::pragmas;

static INIT: Once = Once::new();
static mut INIT_RESULT: i32 = 0;

type VecInitFn = unsafe extern "C" fn(
    db: *mut ffi::sqlite3,
    pz_err_msg: *mut *mut std::os::raw::c_char,
    p_api: *const ffi::sqlite3_api_routines,
) -> std::os::raw::c_int;

/// Register the sqlite-vec extension globally via `sqlite3_auto_extension`.
/// Safe to call multiple times — only the first call registers.
pub fn ensure_sqlite_vec_registered() {
    INIT.call_once(|| unsafe {
        let raw = sqlite_vec::sqlite3_vec_init as *const ();
        let cast: VecInitFn = std::mem::transmute(raw);
        INIT_RESULT = ffi::sqlite3_auto_extension(Some(cast));
    });
}

fn open_write_with_vec(path: &str) -> DbResult<Connection> {
    ensure_sqlite_vec_registered();
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_URI;
    let conn = Connection::open_with_flags(path, flags)?;
    pragmas::apply_write(&conn)?;
    Ok(conn)
}

fn open_read_with_vec(path: &str) -> DbResult<Connection> {
    ensure_sqlite_vec_registered();
    if !std::path::Path::new(path).exists() {
        return Err(DbError::NotFound(path.to_string()));
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_URI;
    let conn = Connection::open_with_flags(path, flags)?;
    pragmas::apply_read(&conn)?;
    Ok(conn)
}

/// Create a vec0 virtual table with `dim` float columns. Drops any existing
/// table of the same name first so `build_vec_index` is idempotent.
pub fn vec_create_table(db_path: &str, table_name: &str, dim: usize) -> DbResult<()> {
    crate::schema::validate_ident(table_name)?;
    let conn = open_write_with_vec(db_path)?;
    conn.execute_batch(&format!("DROP TABLE IF EXISTS {table_name}"))?;
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE {table_name} USING vec0(doc_id TEXT PRIMARY KEY, embedding float[{dim}])"
    ))?;
    Ok(())
}

pub fn vec_drop_table(db_path: &str, table_name: &str) -> DbResult<()> {
    crate::schema::validate_ident(table_name)?;
    let conn = open_write_with_vec(db_path)?;
    conn.execute_batch(&format!("DROP TABLE IF EXISTS {table_name}"))?;
    Ok(())
}

/// Bulk-insert vector rows into a vec0 table.
///
/// `vectors_bytes` is the raw little-endian float32 payload — shape [N, dim],
/// row-major. `doc_ids.len() * dim * 4` must equal `vectors_bytes.len()`.
/// Runs in a single transaction; returns rows inserted.
pub fn vec_bulk_insert(
    db_path: &str,
    table_name: &str,
    doc_ids: Vec<String>,
    vectors_bytes: Vec<u8>,
    dim: usize,
) -> DbResult<usize> {
    crate::schema::validate_ident(table_name)?;
    let expected = doc_ids.len() * dim * 4;
    if vectors_bytes.len() != expected {
        return Err(DbError::Other(format!(
            "vector byte length {} != expected {} ({} ids * {} dim * 4)",
            vectors_bytes.len(),
            expected,
            doc_ids.len(),
            dim
        )));
    }
    let stride = dim * 4;

    let mut conn = open_write_with_vec(db_path)?;
    let tx = conn.transaction()?;
    {
        let sql = format!("INSERT INTO {table_name}(doc_id, embedding) VALUES (?, ?)");
        let mut stmt = tx.prepare_cached(&sql)?;
        for (i, id) in doc_ids.iter().enumerate() {
            let start = i * stride;
            let end = start + stride;
            let slice = &vectors_bytes[start..end];
            stmt.execute(rusqlite::params![id, slice])?;
        }
    }
    let count = doc_ids.len();
    tx.commit()?;
    Ok(count)
}

/// KNN search against a vec0 table. `query_bytes` is a single float32 vector
/// (little-endian). Returns list of (doc_id, similarity) where similarity is
/// `1.0 - distance` matching the Python helper.
pub fn vec_knn_search(
    db_path: &str,
    table_name: &str,
    query_bytes: Vec<u8>,
    k: usize,
) -> DbResult<Vec<(String, f64)>> {
    crate::schema::validate_ident(table_name)?;

    let conn = open_read_with_vec(db_path)?;
    let sql =
        format!("SELECT doc_id, distance FROM {table_name} WHERE embedding MATCH ? AND k = ?");
    let mut stmt = conn.prepare(&sql)?;
    let mut out = Vec::new();
    let mut rows = stmt.query(rusqlite::params![query_bytes.as_slice(), k as i64])?;
    while let Some(row) = rows.next()? {
        let doc_id: String = row.get(0)?;
        let distance: f64 = row.get(1)?;
        // Match Python: round(1.0 - distance, 4)
        let sim = ((1.0 - distance) * 10000.0).round() / 10000.0;
        out.push((doc_id, sim));
    }
    Ok(out)
}
