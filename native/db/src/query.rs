use std::collections::HashMap;

use rusqlite::types::{Value as SqlVal, ValueRef};
use rusqlite::{Row, Statement};

use crate::error::{DbError, DbResult};
use crate::fts5::{cleaned_words, column_scope_prefix, fts5_escape_str};
use crate::registry::get_read;
use crate::schema;

pub type RowMap = Vec<(String, SqlVal)>;

fn row_to_map(row: &Row<'_>, col_names: &[String]) -> rusqlite::Result<RowMap> {
    let mut out = Vec::with_capacity(col_names.len());
    for (i, name) in col_names.iter().enumerate() {
        let v = match row.get_ref(i)? {
            ValueRef::Null => SqlVal::Null,
            ValueRef::Integer(i) => SqlVal::Integer(i),
            ValueRef::Real(f) => SqlVal::Real(f),
            ValueRef::Text(t) => SqlVal::Text(String::from_utf8_lossy(t).into_owned()),
            ValueRef::Blob(b) => SqlVal::Blob(b.to_vec()),
        };
        out.push((name.clone(), v));
    }
    Ok(out)
}

fn stmt_col_names(stmt: &Statement<'_>) -> Vec<String> {
    stmt.column_names().iter().map(|s| s.to_string()).collect()
}

pub fn fts_search(
    db_path: &str,
    table: &str,
    fts_table: &str,
    query: &str,
    filters: Option<HashMap<String, String>>,
    max_results: i64,
    columns: &str,
    search_columns: Option<Vec<String>>,
    offset: i64,
) -> DbResult<Vec<RowMap>> {
    schema::validate_ident(table)?;
    schema::validate_ident(fts_table)?;
    // `columns` is either "t.*" or a comma-separated column list like "t.form_key, t.editor_id".
    validate_columns_expr(columns)?;

    let filter_pairs = validated_filter_pairs(table, filters.as_ref())?;
    let match_expr = build_match_expr(query, search_columns.as_deref());

    let conn_h = get_read(db_path)?;
    let conn = conn_h
        .lock()
        .map_err(|_| DbError::BulkState("registry lock poisoned".into()))?;
    let mut sql = format!(
        "SELECT {columns} FROM {table} t JOIN {fts_table} f ON t.rowid = f.rowid WHERE {fts_table} MATCH ?"
    );
    let mut params: Vec<SqlVal> = vec![SqlVal::Text(match_expr)];
    for (col, val) in &filter_pairs {
        sql.push_str(&format!(" AND t.{col} = ?"));
        params.push(SqlVal::Text(val.clone()));
    }
    sql.push_str(" ORDER BY rank LIMIT ?");
    params.push(SqlVal::Integer(max_results));
    if offset > 0 {
        sql.push_str(" OFFSET ?");
        params.push(SqlVal::Integer(offset));
    }

    let mut stmt = conn.prepare(&sql)?;
    let col_names = stmt_col_names(&stmt);
    let refs: Vec<&dyn rusqlite::ToSql> =
        params.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
    let mut out = Vec::new();
    let mut rows = stmt.query(rusqlite::params_from_iter(refs.iter().copied()))?;
    while let Some(row) = rows.next()? {
        out.push(row_to_map(row, &col_names)?);
    }
    Ok(out)
}

pub fn exact_lookup(
    db_path: &str,
    table: &str,
    key_column: &str,
    key_value: &str,
) -> DbResult<Option<RowMap>> {
    schema::validate_ident(table)?;
    schema::validate_ident(key_column)?;
    schema::check_key(table, key_column)?;

    let conn_h = get_read(db_path)?;
    let conn = conn_h
        .lock()
        .map_err(|_| DbError::BulkState("registry lock poisoned".into()))?;
    let sql = format!("SELECT * FROM {table} WHERE {key_column} = ? LIMIT 1");
    let mut stmt = conn.prepare(&sql)?;
    let col_names = stmt_col_names(&stmt);
    let mut rows = stmt.query([key_value])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_map(row, &col_names)?))
    } else {
        Ok(None)
    }
}

pub fn batch_lookup(
    db_path: &str,
    table: &str,
    key_column: &str,
    key_values: Vec<String>,
    columns: &str,
) -> DbResult<HashMap<String, RowMap>> {
    let mut out = HashMap::new();
    if key_values.is_empty() {
        return Ok(out);
    }
    schema::validate_ident(table)?;
    schema::validate_ident(key_column)?;
    schema::check_key(table, key_column)?;
    validate_columns_expr(columns)?;

    let conn_h = get_read(db_path)?;
    let conn = conn_h
        .lock()
        .map_err(|_| DbError::BulkState("registry lock poisoned".into()))?;
    let placeholders = std::iter::repeat("?")
        .take(key_values.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("SELECT {columns} FROM {table} WHERE {key_column} IN ({placeholders})");
    let mut stmt = conn.prepare(&sql)?;
    let col_names = stmt_col_names(&stmt);
    let refs: Vec<&dyn rusqlite::ToSql> = key_values
        .iter()
        .map(|v| v as &dyn rusqlite::ToSql)
        .collect();
    let mut rows = stmt.query(rusqlite::params_from_iter(refs.iter().copied()))?;
    while let Some(row_ref) = rows.next()? {
        let row = row_to_map(row_ref, &col_names)?;
        if let Some((_, key_value)) = row.iter().find(|(k, _)| k == key_column) {
            if let Some(key) = sql_value_to_key(key_value) {
                out.insert(key, row);
            }
        }
    }
    Ok(out)
}

pub fn count_by_column(db_path: &str, table: &str, column: &str) -> DbResult<Vec<(SqlVal, i64)>> {
    schema::validate_ident(table)?;
    schema::validate_ident(column)?;
    schema::check_group(table, column)?;

    let conn_h = get_read(db_path)?;
    let conn = conn_h
        .lock()
        .map_err(|_| DbError::BulkState("registry lock poisoned".into()))?;
    let sql = format!(
        "SELECT {column}, COUNT(*) as cnt FROM {table} GROUP BY {column} ORDER BY cnt DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut out = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let key = match row.get_ref(0)? {
            ValueRef::Null => SqlVal::Null,
            ValueRef::Integer(i) => SqlVal::Integer(i),
            ValueRef::Real(f) => SqlVal::Real(f),
            ValueRef::Text(t) => SqlVal::Text(String::from_utf8_lossy(t).into_owned()),
            ValueRef::Blob(b) => SqlVal::Blob(b.to_vec()),
        };
        let cnt: i64 = row.get(1)?;
        out.push((key, cnt));
    }
    Ok(out)
}

pub fn fallback_word_search(
    db_path: &str,
    table: &str,
    fts_table: &str,
    query: &str,
    existing_hits: Vec<RowMap>,
    id_column: &str,
    filters: Option<HashMap<String, String>>,
    max_results: i64,
    search_columns: Option<Vec<String>>,
) -> DbResult<Vec<RowMap>> {
    if existing_hits.len() >= 3 {
        return Ok(existing_hits);
    }
    schema::validate_ident(table)?;
    schema::validate_ident(fts_table)?;
    schema::validate_ident(id_column)?;

    let words = cleaned_words(query, 3);
    if words.len() <= 1 {
        return Ok(existing_hits);
    }

    let mut or_query = words
        .iter()
        .map(|w| format!("\"{w}\""))
        .collect::<Vec<_>>()
        .join(" OR ");
    if let Some(cols) = &search_columns {
        or_query = format!("{}{}", column_scope_prefix(cols), or_query);
    }

    let filter_pairs = validated_filter_pairs(table, filters.as_ref())?;
    let conn_h = get_read(db_path)?;
    let conn = conn_h
        .lock()
        .map_err(|_| DbError::BulkState("registry lock poisoned".into()))?;
    let mut sql = format!(
        "SELECT t.* FROM {table} t JOIN {fts_table} f ON t.rowid = f.rowid WHERE {fts_table} MATCH ?"
    );
    let mut params: Vec<SqlVal> = vec![SqlVal::Text(or_query)];
    for (col, val) in &filter_pairs {
        sql.push_str(&format!(" AND t.{col} = ?"));
        params.push(SqlVal::Text(val.clone()));
    }
    sql.push_str(" ORDER BY rank LIMIT ?");
    params.push(SqlVal::Integer(max_results));

    let mut stmt = conn.prepare(&sql)?;
    let col_names = stmt_col_names(&stmt);
    let refs: Vec<&dyn rusqlite::ToSql> =
        params.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
    let mut candidate_rows = Vec::new();
    let mut rows = stmt.query(rusqlite::params_from_iter(refs.iter().copied()))?;
    while let Some(row) = rows.next()? {
        candidate_rows.push(row_to_map(row, &col_names)?);
    }

    let mut out = Vec::with_capacity(max_results.max(0) as usize);
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for h in &existing_hits {
        if let Some((_, v)) = h.iter().find(|(k, _)| k == id_column) {
            if let Some(s) = sql_value_to_key(v) {
                seen.insert(s);
            }
        }
        out.push(h.clone());
    }
    let remaining = (max_results as usize).saturating_sub(existing_hits.len());
    let mut added = 0usize;
    for r in candidate_rows {
        if added >= remaining {
            break;
        }
        let id_val = r
            .iter()
            .find(|(k, _)| k == id_column)
            .and_then(|(_, v)| sql_value_to_key(v));
        if let Some(key) = id_val {
            if seen.contains(&key) {
                continue;
            }
            seen.insert(key);
        }
        out.push(r);
        added += 1;
    }
    Ok(out)
}

fn build_match_expr(query: &str, search_columns: Option<&[String]>) -> String {
    let escaped = fts5_escape_str(query);
    match search_columns {
        Some(cols) if !cols.is_empty() => format!("{}{}", column_scope_prefix(cols), escaped),
        _ => escaped,
    }
}

fn validated_filter_pairs(
    table: &str,
    filters: Option<&HashMap<String, String>>,
) -> DbResult<Vec<(String, String)>> {
    let Some(filters) = filters else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (col, val) in filters {
        if val.is_empty() {
            continue;
        }
        schema::check_filter(table, col)?;
        schema::validate_ident(col)?;
        out.push((col.clone(), val.clone()));
    }
    Ok(out)
}

fn validate_columns_expr(expr: &str) -> DbResult<()> {
    // Accepts "t.*", "*", or comma-separated qualified columns.
    for part in expr.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(DbError::Schema("empty columns expression".into()));
        }
        if part == "*" || part == "t.*" {
            continue;
        }
        // Allow "t.colname" or "colname".
        let rest = part.strip_prefix("t.").unwrap_or(part);
        if !rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(DbError::Schema(format!(
                "invalid columns expression '{expr}'"
            )));
        }
    }
    Ok(())
}

fn sql_value_to_key(v: &SqlVal) -> Option<String> {
    match v {
        SqlVal::Text(t) => Some(t.clone()),
        SqlVal::Integer(i) => Some(i.to_string()),
        _ => None,
    }
}
