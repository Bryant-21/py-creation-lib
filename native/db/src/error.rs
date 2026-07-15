use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("database not found: {0}")]
    NotFound(String),

    #[error("invalid filter column '{column}' for table '{table}'")]
    BadFilter { table: String, column: String },

    #[error("invalid key column '{column}' for table '{table}'")]
    BadKey { table: String, column: String },

    #[error("invalid group column '{column}' for table '{table}'")]
    BadGroup { table: String, column: String },

    #[error("unknown table '{0}'")]
    UnknownTable(String),

    #[error(
        "column length mismatch in bulk insert: table '{table}', column '{column}' has {actual} values, expected {expected}"
    )]
    ColumnLengthMismatch {
        table: String,
        column: String,
        expected: usize,
        actual: usize,
    },

    #[error("bulk insert in invalid state: {0}")]
    BulkState(String),

    #[error("schema error: {0}")]
    Schema(String),

    #[error("{0}")]
    Other(String),
}

pub type DbResult<T> = Result<T, DbError>;
