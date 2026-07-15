use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rusqlite::types::{Value as SqlVal, ValueRef};
use rusqlite::{Connection, OpenFlags};

use crate::embeddings::ensure_sqlite_vec_registered;
use crate::error::{DbError, DbResult};
use crate::pragmas;
use crate::query::RowMap;

pub struct Database {
    pub(crate) inner: Arc<Mutex<Connection>>,
    pub(crate) path: PathBuf,
    pub(crate) write: bool,
}

impl Database {
    /// Open a SQLite database. `mode` is "ro" or "rw". When `load_vec` is true,
    /// the sqlite-vec extension is registered globally before opening the
    /// connection so vec0 virtual tables are available.
    pub fn open(path: &str, mode: &str, load_vec: bool) -> DbResult<Self> {
        Self::open_impl(path, mode, load_vec)
    }

    pub fn path(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }

    pub fn is_write(&self) -> bool {
        self.write
    }

    /// Run a multi-statement SQL script (CREATE TABLE, CREATE INDEX, etc.).
    pub fn execute(&self, sql: &str) -> DbResult<()> {
        let conn = self.inner.clone();
        let sql_owned = sql.to_string();
        let guard = conn
            .lock()
            .map_err(|_| DbError::BulkState("poisoned connection mutex".into()))?;
        guard.execute_batch(&sql_owned)?;
        Ok(())
    }

    /// Execute a parameterized statement. `params` are positional strings/ints/floats/None/bytes.
    pub fn execute_one(&self, sql: &str, params: Vec<crate::bulk::SqlValue>) -> DbResult<usize> {
        let conn = self.inner.clone();
        let sql_owned = sql.to_string();
        let guard = conn
            .lock()
            .map_err(|_| DbError::BulkState("poisoned connection mutex".into()))?;
        let mut stmt = guard.prepare(&sql_owned)?;
        let refs: Vec<&dyn rusqlite::ToSql> =
            params.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        let n = stmt.execute(rusqlite::params_from_iter(refs.iter().copied()))?;
        Ok(n)
    }

    /// Run a SELECT and return every row as a `{column_name: value}` map.
    pub fn query_all(
        &self,
        sql: &str,
        params: Vec<crate::bulk::SqlValue>,
    ) -> DbResult<Vec<RowMap>> {
        let conn = self.inner.clone();
        let guard = conn
            .lock()
            .map_err(|_| DbError::BulkState("poisoned connection mutex".into()))?;
        let mut stmt = guard.prepare(sql)?;
        let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let ncols = col_names.len();
        let refs: Vec<&dyn rusqlite::ToSql> =
            params.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        let mut out = Vec::new();
        let mut rows = stmt.query(rusqlite::params_from_iter(refs.iter().copied()))?;
        while let Some(row) = rows.next()? {
            let mut mapped: RowMap = Vec::with_capacity(ncols);
            for (i, name) in col_names.iter().enumerate() {
                let v = match row.get_ref(i)? {
                    ValueRef::Null => SqlVal::Null,
                    ValueRef::Integer(v) => SqlVal::Integer(v),
                    ValueRef::Real(v) => SqlVal::Real(v),
                    ValueRef::Text(t) => SqlVal::Text(String::from_utf8_lossy(t).into_owned()),
                    ValueRef::Blob(b) => SqlVal::Blob(b.to_vec()),
                };
                mapped.push((name.clone(), v));
            }
            out.push(mapped);
        }
        Ok(out)
    }

    pub fn close(&mut self) -> DbResult<()> {
        // Dropping the last Arc clone closes the connection. We don't force it
        // here; callers can rely on GC or explicit __exit__.
        Ok(())
    }

    fn open_impl(path: &str, mode: &str, load_vec: bool) -> DbResult<Self> {
        let path_buf = PathBuf::from(path);
        let write = match mode {
            "ro" => false,
            "rw" => true,
            other => {
                return Err(DbError::Other(format!(
                    "unknown mode '{other}' (expected 'ro' or 'rw')"
                )));
            }
        };
        if !write && !path_buf.exists() {
            return Err(DbError::NotFound(path.to_string()));
        }
        if load_vec {
            ensure_sqlite_vec_registered();
        }
        let flags = if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_URI
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_URI
        };
        let conn = Connection::open_with_flags(&path_buf, flags)?;
        if write {
            pragmas::apply_write(&conn)?;
        } else {
            pragmas::apply_read(&conn)?;
        }
        Ok(Self {
            inner: Arc::new(Mutex::new(conn)),
            path: path_buf,
            write,
        })
    }
}
