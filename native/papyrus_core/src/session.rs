//! Session store — handle-based document state for the LSP.
//!
//! Same pattern as `py_creation_lib/native/esp/src/plugin_runtime.rs`: a static
//! `OnceLock<Mutex<HashMap<u64, Session>>>` plus an atomic id allocator. Each
//! session owns its document text (a `ropey::Rope` for cheap edits) and caches
//! the parsed AST + diagnostics so an unchanged buffer isn't re-parsed.
//!
//! All public methods take `&self` / `&Self::Pool`, so concurrent calls from
//! different LSP requests (or threads under `py.detach`) are safe.

use crate::ast::ScriptNode;
use crate::parser::{ParseError, parse_script};
use crate::resolver::{Diagnostic, resolve};
use crate::script_db::ScriptDB;
use ropey::Rope;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

// ---------------------------------------------------------------------------
// DB pool
// ---------------------------------------------------------------------------

static DB_POOL: OnceLock<Mutex<HashMap<u64, Arc<ScriptDB>>>> = OnceLock::new();
static NEXT_DB_ID: AtomicU64 = AtomicU64::new(1);

fn db_pool() -> &'static Mutex<HashMap<u64, Arc<ScriptDB>>> {
    DB_POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn db_open(path: &str, source_dirs: &[String]) -> u64 {
    let id = NEXT_DB_ID.fetch_add(1, Ordering::Relaxed);
    let db = Arc::new(ScriptDB::open(path, source_dirs));
    db_pool().lock().unwrap().insert(id, db);
    id
}

pub fn db_close(id: u64) -> bool {
    db_pool().lock().unwrap().remove(&id).is_some()
}

pub fn db_get(id: u64) -> Option<Arc<ScriptDB>> {
    db_pool().lock().unwrap().get(&id).cloned()
}

// ---------------------------------------------------------------------------
// Session pool
// ---------------------------------------------------------------------------

static SESSION_POOL: OnceLock<Mutex<HashMap<u64, Arc<Mutex<Session>>>>> = OnceLock::new();
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

fn session_pool() -> &'static Mutex<HashMap<u64, Arc<Mutex<Session>>>> {
    SESSION_POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Default)]
struct CachedAnalysis {
    ast: Option<ScriptNode>,
    parse_errors: Vec<ParseError>,
    diagnostics: Vec<Diagnostic>,
    /// Bumped each time the rope changes; cache is valid when it matches.
    text_revision: u64,
}

pub struct Session {
    pub uri: String,
    pub rope: Rope,
    pub db_id: Option<u64>,
    revision: u64,
    cached: CachedAnalysis,
}

impl Session {
    fn new(uri: String, text: String, db_id: Option<u64>) -> Self {
        Self {
            uri,
            rope: Rope::from_str(&text),
            db_id,
            revision: 1,
            cached: CachedAnalysis::default(),
        }
    }

    fn invalidate(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn ensure_analyzed(&mut self) {
        if self.cached.text_revision == self.revision && self.cached.ast.is_some() {
            return;
        }
        let text = self.rope.to_string();
        let result = parse_script(&text);
        let diagnostics = match (&result.ast, self.db_id) {
            (Some(ast), Some(db_id)) => match db_get(db_id) {
                Some(db) => resolve(ast, &db),
                None => Vec::new(),
            },
            (Some(ast), None) => {
                // Run resolver with empty DB so we still get extends/import
                // unknown-symbol diagnostics for user scripts that haven't been
                // indexed yet.
                let db = ScriptDB::empty();
                resolve(ast, &db)
            }
            _ => Vec::new(),
        };
        self.cached = CachedAnalysis {
            ast: result.ast,
            parse_errors: result.errors,
            diagnostics,
            text_revision: self.revision,
        };
    }
}

pub fn session_open(uri: String, text: String, db_id: Option<u64>) -> u64 {
    let id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    let s = Arc::new(Mutex::new(Session::new(uri, text, db_id)));
    session_pool().lock().unwrap().insert(id, s);
    id
}

pub fn session_close(id: u64) -> bool {
    session_pool().lock().unwrap().remove(&id).is_some()
}

pub fn session_get(id: u64) -> Option<Arc<Mutex<Session>>> {
    session_pool().lock().unwrap().get(&id).cloned()
}

pub fn session_replace_text(id: u64, text: String) -> bool {
    let Some(s) = session_get(id) else {
        return false;
    };
    let mut s = s.lock().unwrap();
    s.rope = Rope::from_str(&text);
    s.invalidate();
    true
}

/// Apply an incremental LSP edit: replace the byte range [start_offset..end_offset)
/// with `replacement`. Offsets are byte offsets into the rope.
pub fn session_apply_edit(
    id: u64,
    start_offset: usize,
    end_offset: usize,
    replacement: &str,
) -> bool {
    let Some(s) = session_get(id) else {
        return false;
    };
    let mut s = s.lock().unwrap();
    let total = s.rope.len_chars();
    let start = char_offset_from_byte(&s.rope, start_offset).min(total);
    let end = char_offset_from_byte(&s.rope, end_offset)
        .min(total)
        .max(start);
    s.rope.remove(start..end);
    s.rope.insert(start, replacement);
    s.invalidate();
    true
}

fn char_offset_from_byte(rope: &Rope, byte: usize) -> usize {
    if byte >= rope.len_bytes() {
        return rope.len_chars();
    }
    rope.byte_to_char(byte)
}

pub fn session_text(id: u64) -> Option<String> {
    let s = session_get(id)?;
    Some(s.lock().unwrap().rope.to_string())
}

pub fn session_diagnostics(id: u64) -> Option<Vec<Diagnostic>> {
    let s = session_get(id)?;
    let mut s = s.lock().unwrap();
    s.ensure_analyzed();
    let mut all: Vec<Diagnostic> = s
        .cached
        .parse_errors
        .iter()
        .map(|e| Diagnostic {
            line: e.line,
            col: e.col,
            end_line: e.line,
            end_col: e.col,
            message: e.message.clone(),
            severity: crate::resolver::DiagnosticSeverity::Error,
        })
        .collect();
    all.extend(s.cached.diagnostics.clone());
    Some(all)
}

pub fn session_ast(id: u64) -> Option<ScriptNode> {
    let s = session_get(id)?;
    let mut s = s.lock().unwrap();
    s.ensure_analyzed();
    s.cached.ast.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_close_lifecycle() {
        let sid = session_open("file:///x.psc".into(), "ScriptName Foo\n".into(), None);
        assert!(session_get(sid).is_some());
        assert!(session_close(sid));
        assert!(session_get(sid).is_none());
    }

    #[test]
    fn diagnostics_after_open() {
        let sid = session_open(
            "file:///x.psc".into(),
            "ScriptName Foo extends NoSuchScript\n".into(),
            None,
        );
        let diags = session_diagnostics(sid).unwrap();
        assert!(diags.iter().any(|d| d.message.contains("NoSuchScript")));
        session_close(sid);
    }

    #[test]
    fn replace_text_invalidates_cache() {
        let sid = session_open(
            "file:///x.psc".into(),
            "ScriptName Foo extends Bad\n".into(),
            None,
        );
        let d1 = session_diagnostics(sid).unwrap();
        assert!(d1.iter().any(|d| d.message.contains("Bad")));
        session_replace_text(sid, "ScriptName Foo\n".into());
        let d2 = session_diagnostics(sid).unwrap();
        assert!(d2.iter().all(|d| !d.message.contains("Bad")));
        session_close(sid);
    }

    #[test]
    fn ast_is_cached_until_edit() {
        let sid = session_open(
            "file:///x.psc".into(),
            "ScriptName Foo\nInt Property X Auto\n".into(),
            None,
        );
        let a1 = session_ast(sid).unwrap();
        assert_eq!(a1.properties.len(), 1);
        session_replace_text(sid, "ScriptName Foo\n".into());
        let a2 = session_ast(sid).unwrap();
        assert_eq!(a2.properties.len(), 0);
        session_close(sid);
    }
}
