use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use rusqlite::{Connection, OpenFlags};

use crate::error::{DbError, DbResult};
use crate::pragmas;

type ConnHandle = Arc<Mutex<Connection>>;

static REGISTRY: Lazy<Mutex<HashMap<PathBuf, ConnHandle>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn canonical(path: &str) -> PathBuf {
    let p = Path::new(path);
    dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Fetch (opening if needed) a shared read-only connection for `path`.
/// Callers lock the returned handle to run queries.
pub fn get_read(path: &str) -> DbResult<ConnHandle> {
    let key = canonical(path);
    {
        let guard = REGISTRY
            .lock()
            .map_err(|_| DbError::BulkState("registry poisoned".into()))?;
        if let Some(h) = guard.get(&key) {
            return Ok(h.clone());
        }
    }
    if !key.exists() {
        return Err(DbError::NotFound(path.to_string()));
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_URI;
    let conn = Connection::open_with_flags(&key, flags)?;
    pragmas::apply_read(&conn)?;
    let handle = Arc::new(Mutex::new(conn));
    let mut guard = REGISTRY
        .lock()
        .map_err(|_| DbError::BulkState("registry poisoned".into()))?;
    // Double-check in case a racing thread opened it.
    if let Some(existing) = guard.get(&key) {
        return Ok(existing.clone());
    }
    guard.insert(key, handle.clone());
    Ok(handle)
}

/// Drop all cached read-only connections. Useful when the caller knows a
/// .db file was rewritten and stale handles must be purged.
pub fn clear_registry() -> DbResult<()> {
    let mut guard = REGISTRY
        .lock()
        .map_err(|_| DbError::BulkState("registry poisoned".into()))?;
    guard.clear();
    Ok(())
}

/// Drop a single cached connection by path.
pub fn drop_path(path: &str) {
    let key = canonical(path);
    if let Ok(mut guard) = REGISTRY.lock() {
        guard.remove(&key);
    }
}
