use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags, params};

use crate::error::{DbError, DbResult};

const BATCH_SIZE: usize = 50_000;

fn default_cache_dir() -> PathBuf {
    if let Some(dirs) = directories::ProjectDirs::from("", "", "modkit21") {
        return dirs.cache_dir().to_path_buf();
    }
    if cfg!(windows) {
        if let Some(v) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(v).join("modkit21");
        }
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    if cfg!(windows) {
        home.join("modkit21")
    } else {
        home.join(".cache").join("modkit21")
    }
}

fn max_mtime(root: &Path) -> f64 {
    let mut best = 0.0f64;
    if let Ok(meta) = std::fs::metadata(root) {
        if let Ok(m) = meta.modified() {
            if let Ok(d) = m.duration_since(UNIX_EPOCH) {
                best = d.as_secs_f64();
            }
        }
    } else {
        return 0.0;
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if let Ok(ft) = entry.file_type() {
                if ft.is_dir() {
                    if let Ok(meta) = entry.metadata() {
                        if let Ok(m) = meta.modified() {
                            if let Ok(d) = m.duration_since(UNIX_EPOCH) {
                                let t = d.as_secs_f64();
                                if t > best {
                                    best = t;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    best
}

struct Inner {
    conn: Connection,
    root: PathBuf,
    root_str: String,
    lookup: HashMap<String, String>,
    missing: HashSet<String>,
    file_count: usize,
    fully_loaded: bool,
}

pub struct DirectoryIndex {
    inner: Arc<Mutex<Inner>>,
}

impl DirectoryIndex {
    pub fn new(root: &str, cache_dir: Option<&str>) -> DbResult<Self> {
        let root_path = PathBuf::from(root);
        let cache_dir = cache_dir
            .map(PathBuf::from)
            .unwrap_or_else(default_cache_dir);
        Ok(Self {
            inner: Arc::new(Mutex::new(Self::build(root_path, cache_dir)?)),
        })
    }

    pub fn resolve(&self, rel_path: &str) -> DbResult<Option<String>> {
        let key = rel_path.replace('\\', "/").to_lowercase();
        let inner = self.inner.clone();
        let mut g = inner
            .lock()
            .map_err(|_| DbError::BulkState("dir-index poisoned".into()))?;
        if let Some(v) = g.lookup.get(&key) {
            return Ok(Some(v.clone()));
        }
        if g.missing.contains(&key) {
            return Ok(None);
        }
        if g.fully_loaded {
            return Ok(None);
        }
        let row: Option<String> = g
            .conn
            .query_row(
                "SELECT abs_path FROM dir_files WHERE root_path = ? AND rel_lower = ?",
                params![&g.root_str, &key],
                |row| row.get(0),
            )
            .ok();
        match row {
            Some(p) => {
                g.lookup.insert(key, p.clone());
                Ok(Some(p))
            }
            None => {
                g.missing.insert(key);
                Ok(None)
            }
        }
    }

    pub fn contains(&self, rel_path: &str) -> DbResult<bool> {
        Ok(self.resolve(rel_path)?.is_some())
    }

    pub fn file_count(&self) -> DbResult<usize> {
        let g = self
            .inner
            .lock()
            .map_err(|_| DbError::BulkState("dir-index poisoned".into()))?;
        if g.fully_loaded {
            Ok(g.lookup.len())
        } else {
            Ok(g.file_count)
        }
    }

    pub fn lookup_snapshot(&self) -> DbResult<HashMap<String, String>> {
        let g = self
            .inner
            .lock()
            .map_err(|_| DbError::BulkState("dir-index poisoned".into()))?;
        Ok(g.lookup.clone())
    }

    pub fn close(&self) -> DbResult<()> {
        // Connection is owned by Inner; dropping the last Arc closes it.
        Ok(())
    }

    fn build(root: PathBuf, cache_dir: PathBuf) -> DbResult<Inner> {
        std::fs::create_dir_all(&cache_dir)?;
        let db_path = cache_dir.join("dir_index.sqlite");
        let resolved_root = dunce::canonicalize(&root).unwrap_or(root);
        let root_str = resolved_root.to_string_lossy().into_owned();
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI;
        let conn = Connection::open_with_flags(&db_path, flags)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS dir_meta (
                 root_path  TEXT PRIMARY KEY,
                 scan_time  REAL,
                 file_count INTEGER
             );
             CREATE TABLE IF NOT EXISTS dir_files (
                 root_path TEXT,
                 rel_lower TEXT,
                 abs_path  TEXT,
                 PRIMARY KEY (root_path, rel_lower)
             );",
        )?;

        let mut inner = Inner {
            conn,
            root: resolved_root.clone(),
            root_str: root_str.clone(),
            lookup: HashMap::new(),
            missing: HashSet::new(),
            file_count: 0,
            fully_loaded: false,
        };
        if Self::is_fresh(&mut inner)? {
            return Ok(inner);
        }
        Self::scan_and_store(&mut inner)?;
        Ok(inner)
    }

    fn is_fresh(inner: &mut Inner) -> DbResult<bool> {
        let row: Option<(f64, i64)> = inner
            .conn
            .query_row(
                "SELECT scan_time, file_count FROM dir_meta WHERE root_path = ?",
                params![&inner.root_str],
                |row| Ok((row.get::<_, f64>(0)?, row.get::<_, i64>(1)?)),
            )
            .ok();
        let Some((scan_time, count)) = row else {
            return Ok(false);
        };
        inner.file_count = count.max(0) as usize;
        let current = max_mtime(&inner.root);
        if current > scan_time {
            return Ok(false);
        }
        Ok(true)
    }

    fn scan_and_store(inner: &mut Inner) -> DbResult<()> {
        let root_str = inner.root_str.clone();
        let prefix_len = root_str.len() + 1;
        let mut lookup: HashMap<String, String> = HashMap::new();
        let mut entries: Vec<(String, String, String)> = Vec::new();

        let mut stack: Vec<PathBuf> = vec![inner.root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in rd.flatten() {
                let Ok(ft) = entry.file_type() else { continue };
                if ft.is_dir() {
                    stack.push(entry.path());
                } else if ft.is_file() {
                    let abs = entry.path();
                    let abs_str = abs.to_string_lossy().into_owned();
                    if abs_str.len() > prefix_len {
                        let rel = abs_str[prefix_len..].replace('\\', "/");
                        let rel_lower = rel.to_lowercase();
                        lookup.insert(rel_lower.clone(), abs_str.clone());
                        entries.push((root_str.clone(), rel_lower, abs_str));
                    }
                }
            }
        }

        let tx = inner.conn.transaction()?;
        tx.execute(
            "DELETE FROM dir_files WHERE root_path = ?",
            params![&root_str],
        )?;
        tx.execute(
            "DELETE FROM dir_meta WHERE root_path = ?",
            params![&root_str],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO dir_files (root_path, rel_lower, abs_path) VALUES (?, ?, ?)",
            )?;
            for chunk in entries.chunks(BATCH_SIZE) {
                for row in chunk {
                    stmt.execute(params![&row.0, &row.1, &row.2])?;
                }
            }
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        tx.execute(
            "INSERT INTO dir_meta (root_path, scan_time, file_count) VALUES (?, ?, ?)",
            params![&root_str, now, entries.len() as i64],
        )?;
        tx.commit()?;

        inner.file_count = entries.len();
        inner.lookup = lookup;
        inner.missing.clear();
        inner.fully_loaded = true;
        Ok(())
    }
}
