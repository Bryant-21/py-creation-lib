use rusqlite::Connection;

use crate::error::DbResult;

/// PRAGMAs applied to every writable connection. Matches the settings the
/// Python preprocessors used (WAL + NORMAL sync + 128 MB cache + in-memory
/// temp + 512 MB mmap).
pub const WRITE_PRAGMAS: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA cache_size = -131072;
PRAGMA temp_store = MEMORY;
PRAGMA mmap_size = 536870912;
";

/// PRAGMAs applied to read-only connections. Same tuning without WAL since
/// we open with `mode=ro`.
pub const READ_PRAGMAS: &str = "
PRAGMA cache_size = -131072;
PRAGMA temp_store = MEMORY;
PRAGMA mmap_size = 536870912;
";

pub fn apply_write(conn: &Connection) -> DbResult<()> {
    conn.execute_batch(WRITE_PRAGMAS)?;
    Ok(())
}

pub fn apply_read(conn: &Connection) -> DbResult<()> {
    conn.execute_batch(READ_PRAGMAS)?;
    Ok(())
}
