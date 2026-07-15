use std::collections::HashMap;

use rusqlite::types::Value as SqlVal;
use rusqlite::{Connection, OpenFlags, ToSql};

use crate::bulk_schema::{SchemaDoc, TableSpec};
use crate::embeddings::ensure_sqlite_vec_registered;
use crate::error::{DbError, DbResult};
use crate::pragmas;

pub type SqlValue = SqlVal;

pub struct BulkInserter {
    conn: Option<Connection>,
    schema: SchemaDoc,
    in_txn: bool,
    rows_inserted: usize,
}

impl BulkInserter {
    pub fn new(db_path: &str, schema_json: &str, fresh: bool, load_vec: bool) -> DbResult<Self> {
        let schema = SchemaDoc::parse(schema_json)?;
        if load_vec {
            ensure_sqlite_vec_registered();
        }
        if fresh && std::path::Path::new(db_path).exists() {
            std::fs::remove_file(db_path)?;
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI;
        let conn = Connection::open_with_flags(db_path, flags)?;
        pragmas::apply_write(&conn)?;
        conn.execute_batch("BEGIN")?;

        Ok(Self {
            conn: Some(conn),
            schema,
            in_txn: true,
            rows_inserted: 0,
        })
    }

    /// Execute DDL or other SQL against the inserter's connection (multi-statement OK).
    pub fn execute(&mut self, sql: &str) -> DbResult<()> {
        let sql = sql.to_string();
        self.with_conn(move |conn| {
            conn.execute_batch(&sql)?;
            Ok(())
        })
    }

    /// Execute a single parameterized statement. Returns affected row count.
    pub fn execute_params(&mut self, sql: &str, params: Vec<SqlValue>) -> DbResult<usize> {
        let sql_owned = sql.to_string();
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(&sql_owned)?;
            let refs: Vec<&dyn ToSql> = params.iter().map(|v| v as &dyn ToSql).collect();
            let n = stmt.execute(rusqlite::params_from_iter(refs.iter().copied()))?;
            Ok(n)
        })
    }

    /// Append a columnar chunk: `columns` is {col_name: [values...]} with
    /// every list the same length. Returns number of rows inserted.
    pub fn add_chunk(
        &mut self,
        table: &str,
        columns: HashMap<String, Vec<SqlValue>>,
    ) -> DbResult<usize> {
        let spec = self.table_spec(table)?;
        let cols = spec.columns.clone();
        let insert_sql = spec.insert_sql();

        let row_count = columnar_row_count(&columns, &cols);
        if row_count == 0 {
            return Ok(0);
        }
        let mut rows: Vec<Vec<SqlValue>> = (0..row_count)
            .map(|_| Vec::with_capacity(cols.len()))
            .collect();
        for col_name in &cols {
            let list = columns.get(col_name).ok_or_else(|| {
                DbError::Other(format!("missing column '{col_name}' for table '{table}'"))
            })?;
            if list.len() != row_count {
                return Err(DbError::ColumnLengthMismatch {
                    table: table.to_string(),
                    column: col_name.clone(),
                    expected: row_count,
                    actual: list.len(),
                });
            }
            for (i, item) in list.iter().enumerate() {
                rows[i].push(item.clone());
            }
        }

        let n = self.with_conn(move |conn| {
            let mut stmt = conn.prepare_cached(&insert_sql)?;
            let mut count = 0usize;
            for row in &rows {
                let refs: Vec<&dyn ToSql> = row.iter().map(|v| v as &dyn ToSql).collect();
                stmt.execute(rusqlite::params_from_iter(refs.iter().copied()))?;
                count += 1;
            }
            Ok(count)
        })?;
        self.rows_inserted += n;
        Ok(n)
    }

    /// Append row-dict rows. Less efficient than add_chunk; use for small batches.
    pub fn add_rows(
        &mut self,
        table: &str,
        rows_in: Vec<HashMap<String, SqlValue>>,
    ) -> DbResult<usize> {
        let spec = self.table_spec(table)?;
        let cols = spec.columns.clone();
        let insert_sql = spec.insert_sql();

        let mut rows: Vec<Vec<SqlValue>> = Vec::with_capacity(rows_in.len());
        for row_map in &rows_in {
            let mut row = Vec::with_capacity(cols.len());
            for c in &cols {
                row.push(row_map.get(c).cloned().unwrap_or(SqlValue::Null));
            }
            rows.push(row);
        }

        let n = self.with_conn(move |conn| {
            let mut stmt = conn.prepare_cached(&insert_sql)?;
            let mut count = 0usize;
            for row in &rows {
                let refs: Vec<&dyn ToSql> = row.iter().map(|v| v as &dyn ToSql).collect();
                stmt.execute(rusqlite::params_from_iter(refs.iter().copied()))?;
                count += 1;
            }
            Ok(count)
        })?;
        self.rows_inserted += n;
        Ok(n)
    }

    /// Run a SELECT query against the inserter's connection. Rows are
    /// returned as row-major SQL values so callers can convert them cheaply.
    pub fn query_all(&mut self, sql: &str, params: Vec<SqlValue>) -> DbResult<Vec<Vec<SqlValue>>> {
        let sql_owned = sql.to_string();
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(&sql_owned)?;
            let ncols = stmt.column_count();
            let refs: Vec<&dyn ToSql> = params.iter().map(|v| v as &dyn ToSql).collect();
            let mut out = Vec::new();
            let mut rows = stmt.query(rusqlite::params_from_iter(refs.iter().copied()))?;
            while let Some(row) = rows.next()? {
                let mut tup = Vec::with_capacity(ncols);
                for i in 0..ncols {
                    let v = match row.get_ref(i)? {
                        rusqlite::types::ValueRef::Null => SqlValue::Null,
                        rusqlite::types::ValueRef::Integer(i) => SqlValue::Integer(i),
                        rusqlite::types::ValueRef::Real(f) => SqlValue::Real(f),
                        rusqlite::types::ValueRef::Text(t) => {
                            SqlValue::Text(String::from_utf8_lossy(t).into_owned())
                        }
                        rusqlite::types::ValueRef::Blob(b) => SqlValue::Blob(b.to_vec()),
                    };
                    tup.push(v);
                }
                out.push(tup);
            }
            Ok(out)
        })
    }

    /// `INSERT INTO <fts_table>(<fts_table>) VALUES ('rebuild')`.
    pub fn rebuild_fts(&mut self, fts_table: &str) -> DbResult<()> {
        crate::schema::validate_ident(fts_table)?;
        let sql = format!("INSERT INTO {fts_table}({fts_table}) VALUES('rebuild')");
        self.with_conn(move |conn| {
            conn.execute_batch(&sql)?;
            Ok(())
        })
    }

    /// Rebuild `records_fts` after `records.content` switched to a zstd BLOB.
    /// See `records_indexer::rebuild_records_fts_decompressed` for details.
    pub fn rebuild_records_fts(&mut self) -> DbResult<()> {
        self.with_conn(|conn| crate::records_indexer::rebuild_records_fts_decompressed(conn))
    }

    /// Execute a batch of CREATE INDEX / CREATE TABLE / etc.
    pub fn create_indexes(&mut self, sql: &str) -> DbResult<()> {
        let sql = sql.to_string();
        self.with_conn(move |conn| {
            conn.execute_batch(&sql)?;
            Ok(())
        })
    }

    /// Commit the current transaction and open a new one so the inserter
    /// remains usable.
    pub fn commit(&mut self) -> DbResult<()> {
        let was_in_txn = self.in_txn;
        self.with_conn(move |conn| {
            if was_in_txn {
                conn.execute_batch("COMMIT")?;
            }
            conn.execute_batch("BEGIN")?;
            Ok(())
        })?;
        self.in_txn = true;
        Ok(())
    }

    /// Commit the transaction and release the connection. After this, the
    /// inserter is unusable.
    pub fn finalize(&mut self) -> DbResult<usize> {
        let inserted = self.rows_inserted;
        if let Some(conn) = self.conn.take() {
            if self.in_txn {
                conn.execute_batch("COMMIT")?;
            }
            self.in_txn = false;
        }
        Ok(inserted)
    }

    /// Roll back and release.
    pub fn rollback(&mut self) -> DbResult<()> {
        if let Some(conn) = self.conn.take() {
            if self.in_txn {
                // Ignore errors during rollback because this is a cleanup path.
                let _ = conn.execute_batch("ROLLBACK");
            }
            self.in_txn = false;
        }
        Ok(())
    }

    pub fn rows_inserted(&self) -> usize {
        self.rows_inserted
    }

    fn table_spec(&self, name: &str) -> DbResult<TableSpec> {
        self.schema
            .tables
            .iter()
            .find(|t| t.name == name)
            .cloned()
            .ok_or_else(|| DbError::UnknownTable(name.to_string()))
    }

    fn with_conn<F, R>(&mut self, f: F) -> DbResult<R>
    where
        F: FnOnce(&mut Connection) -> DbResult<R>,
    {
        let mut conn = self
            .conn
            .take()
            .ok_or_else(|| DbError::BulkState("BulkInserter already finalized".into()))?;
        let result = f(&mut conn);
        self.conn = Some(conn);
        result
    }
}

fn columnar_row_count(columns: &HashMap<String, Vec<SqlValue>>, expected_cols: &[String]) -> usize {
    expected_cols
        .first()
        .and_then(|first_col| columns.get(first_col))
        .map(|values| values.len())
        .unwrap_or(0)
}
