//! SQLite wrapper for `fo4_scripts.db` — script symbol lookups with disk fallback.
//!
//! Pure Rust over `rusqlite` and the parser; no PyO3 (the bindings layer wraps
//! this).
//!
//! The DB schema is whatever `modkit index build --domain scripts` produces:
//! a `scripts` table with `script_name`, `extends`, `functions`, `events`,
//! `properties`, `script_path`, `source` columns. Those columns hold format
//! strings such as `"RetType FuncName(params); ..."` for functions.

use crate::parser::parse_script;
use rusqlite::{Connection, OpenFlags, params};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Default)]
pub struct DbFunction {
    pub return_type: String,
    pub name: String,
    pub params: String,
}

#[derive(Debug, Clone, Default)]
pub struct DbEvent {
    pub name: String,
    pub params: String,
}

#[derive(Debug, Clone, Default)]
pub struct DbProperty {
    pub ty: String,
    pub name: String,
}

#[derive(Debug, Clone, Default)]
struct DiskScript {
    name: String,
    extends: Option<String>,
    functions: Vec<DbFunction>,
    properties: Vec<DbProperty>,
    events: Vec<DbEvent>,
    #[allow(dead_code)]
    path: Option<PathBuf>,
}

pub struct ScriptDB {
    conn: Mutex<Option<Connection>>,
    source_dirs: Mutex<Vec<PathBuf>>,
    disk_cache: Mutex<HashMap<String, Option<DiskScript>>>,
}

impl ScriptDB {
    /// Open the SQLite database read-only. If `db_path` doesn't exist or fails
    /// to open, returns a stub that has no DB but can still resolve user
    /// scripts via `add_source_dir`.
    pub fn open(db_path: &str, source_dirs: &[String]) -> Self {
        let conn = Connection::open_with_flags(
            db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .ok();
        Self {
            conn: Mutex::new(conn),
            source_dirs: Mutex::new(source_dirs.iter().map(|p| PathBuf::from(p)).collect()),
            disk_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Stub constructor for tests / disk-only operation.
    pub fn empty() -> Self {
        Self {
            conn: Mutex::new(None),
            source_dirs: Mutex::new(Vec::new()),
            disk_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn add_source_dir(&self, path: &str) {
        let p = PathBuf::from(path);
        let mut dirs = self.source_dirs.lock().unwrap();
        if !dirs.iter().any(|d| d == &p) {
            dirs.push(p);
        }
    }

    /// Register a live AST as a disk-script fallback (e.g., from an open
    /// editor buffer that hasn't been saved or indexed yet).
    pub fn register_ast(&self, ast: &crate::ast::ScriptNode) {
        let name_lower = ast.name.to_ascii_lowercase();
        let mut funcs: Vec<DbFunction> = ast
            .functions
            .iter()
            .map(|f| DbFunction {
                return_type: f.return_type.clone(),
                name: f.name.clone(),
                params: render_params(&f.params),
            })
            .collect();
        for state in &ast.states {
            for f in &state.functions {
                funcs.push(DbFunction {
                    return_type: f.return_type.clone(),
                    name: f.name.clone(),
                    params: render_params(&f.params),
                });
            }
        }
        let disk = DiskScript {
            name: ast.name.clone(),
            extends: ast.parent.clone(),
            functions: funcs,
            properties: ast
                .properties
                .iter()
                .map(|p| DbProperty {
                    ty: p.ty.clone(),
                    name: p.name.clone(),
                })
                .collect(),
            events: ast
                .events
                .iter()
                .map(|e| DbEvent {
                    name: e.name.clone(),
                    params: render_params(&e.params),
                })
                .collect(),
            path: None,
        };
        self.disk_cache
            .lock()
            .unwrap()
            .insert(name_lower, Some(disk));
    }

    fn query_one_string(
        &self,
        sql: &str,
        key: &str,
        col_count: usize,
    ) -> Option<Vec<Option<String>>> {
        let guard = self.conn.lock().unwrap();
        let conn = guard.as_ref()?;
        let mut stmt = conn.prepare(sql).ok()?;
        let mut rows = stmt.query(params![key]).ok()?;
        let row = rows.next().ok()??;
        let mut out = Vec::with_capacity(col_count);
        for i in 0..col_count {
            out.push(row.get::<_, Option<String>>(i).ok().flatten());
        }
        Some(out)
    }

    fn find_on_disk(&self, name: &str) -> Option<DiskScript> {
        let key = name.to_ascii_lowercase();
        {
            let cache = self.disk_cache.lock().unwrap();
            if let Some(entry) = cache.get(&key) {
                return entry.clone();
            }
        }
        let rel = name.replace(':', std::path::MAIN_SEPARATOR_STR);
        let rel_path = PathBuf::from(format!("{rel}.psc"));
        let dirs = self.source_dirs.lock().unwrap().clone();
        for dir in dirs {
            let full = dir.join(&rel_path);
            if full.is_file() {
                let disk = parse_to_disk_script(&full);
                if let Some(ref d) = disk {
                    self.disk_cache
                        .lock()
                        .unwrap()
                        .insert(key.clone(), Some(d.clone()));
                    return Some(d.clone());
                }
            }
        }
        self.disk_cache.lock().unwrap().insert(key, None);
        None
    }

    pub fn script_exists(&self, name: &str) -> bool {
        if let Some(cols) = self.query_one_string(
            "SELECT 1 FROM scripts WHERE script_name = ? COLLATE NOCASE LIMIT 1",
            name,
            1,
        ) {
            if !cols.is_empty() {
                return true;
            }
        }
        self.find_on_disk(name).is_some()
    }

    pub fn get_extends(&self, name: &str) -> Option<String> {
        if let Some(cols) = self.query_one_string(
            "SELECT extends FROM scripts WHERE script_name = ? COLLATE NOCASE LIMIT 1",
            name,
            1,
        ) {
            return cols.into_iter().next().flatten().filter(|s| !s.is_empty());
        }
        self.find_on_disk(name).and_then(|d| d.extends)
    }

    pub fn get_functions(&self, name: &str) -> Vec<DbFunction> {
        if let Some(cols) = self.query_one_string(
            "SELECT functions FROM scripts WHERE script_name = ? COLLATE NOCASE LIMIT 1",
            name,
            1,
        ) {
            let raw = cols.into_iter().next().flatten().unwrap_or_default();
            return parse_functions(&raw);
        }
        self.find_on_disk(name)
            .map(|d| d.functions)
            .unwrap_or_default()
    }

    pub fn get_events(&self, name: &str) -> Vec<DbEvent> {
        if let Some(cols) = self.query_one_string(
            "SELECT events FROM scripts WHERE script_name = ? COLLATE NOCASE LIMIT 1",
            name,
            1,
        ) {
            let raw = cols.into_iter().next().flatten().unwrap_or_default();
            return parse_events(&raw);
        }
        self.find_on_disk(name)
            .map(|d| d.events)
            .unwrap_or_default()
    }

    pub fn get_properties(&self, name: &str) -> Vec<DbProperty> {
        if let Some(cols) = self.query_one_string(
            "SELECT properties FROM scripts WHERE script_name = ? COLLATE NOCASE LIMIT 1",
            name,
            1,
        ) {
            let raw = cols.into_iter().next().flatten().unwrap_or_default();
            return parse_properties(&raw);
        }
        self.find_on_disk(name)
            .map(|d| d.properties)
            .unwrap_or_default()
    }

    pub fn get_hierarchy(&self, name: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut current = name.to_owned();
        let mut seen: HashSet<String> = HashSet::new();
        for _ in 0..20 {
            let lower = current.to_ascii_lowercase();
            if !seen.insert(lower) {
                break;
            }
            // Try DB first.
            let row = self.query_one_string(
                "SELECT script_name, extends FROM scripts WHERE script_name = ? COLLATE NOCASE LIMIT 1",
                &current,
                2,
            );
            if let Some(cols) = row {
                let script_name = cols.first().cloned().flatten().unwrap_or(current.clone());
                chain.push(script_name);
                let extends = cols.get(1).cloned().flatten().filter(|s| !s.is_empty());
                match extends {
                    Some(e) => current = e,
                    None => break,
                }
                continue;
            }
            // Disk fallback.
            if let Some(disk) = self.find_on_disk(&current) {
                chain.push(disk.name.clone());
                match disk.extends {
                    Some(e) => current = e,
                    None => break,
                }
                continue;
            }
            break;
        }
        chain
    }

    pub fn has_function(&self, script_name: &str, func_name: &str) -> bool {
        for ancestor in self.get_hierarchy(script_name) {
            for f in self.get_functions(&ancestor) {
                if f.name.eq_ignore_ascii_case(func_name) {
                    return true;
                }
            }
        }
        false
    }

    pub fn has_event(&self, script_name: &str, event_name: &str) -> bool {
        for ancestor in self.get_hierarchy(script_name) {
            for e in self.get_events(&ancestor) {
                if e.name.eq_ignore_ascii_case(event_name) {
                    return true;
                }
            }
        }
        false
    }

    pub fn has_property(&self, script_name: &str, prop_name: &str) -> bool {
        for ancestor in self.get_hierarchy(script_name) {
            for p in self.get_properties(&ancestor) {
                if p.name.eq_ignore_ascii_case(prop_name) {
                    return true;
                }
            }
        }
        false
    }

    pub fn get_function_return_type(&self, script_name: &str, func_name: &str) -> Option<String> {
        for ancestor in self.get_hierarchy(script_name) {
            for f in self.get_functions(&ancestor) {
                if f.name.eq_ignore_ascii_case(func_name) {
                    return Some(f.return_type);
                }
            }
        }
        None
    }

    pub fn get_script_path(&self, name: &str) -> Option<String> {
        if let Some(cols) = self.query_one_string(
            "SELECT script_path FROM scripts WHERE script_name = ? COLLATE NOCASE LIMIT 1",
            name,
            1,
        ) {
            return cols.into_iter().next().flatten().filter(|s| !s.is_empty());
        }
        self.find_on_disk(name)
            .and_then(|d| d.path.as_ref().map(|p| p.to_string_lossy().into_owned()))
    }

    pub fn get_source(&self, name: &str) -> Option<String> {
        if let Some(cols) = self.query_one_string(
            "SELECT source FROM scripts WHERE script_name = ? COLLATE NOCASE LIMIT 1",
            name,
            1,
        ) {
            return cols.into_iter().next().flatten().filter(|s| !s.is_empty());
        }
        None
    }

    pub fn search_scripts(&self, prefix: &str) -> Vec<String> {
        let guard = self.conn.lock().unwrap();
        let Some(conn) = guard.as_ref() else {
            return Vec::new();
        };
        let pattern = format!("{prefix}%");
        let Ok(mut stmt) = conn.prepare(
            "SELECT script_name FROM scripts WHERE script_name LIKE ? COLLATE NOCASE LIMIT 50",
        ) else {
            return Vec::new();
        };
        let Ok(rows) = stmt.query_map(params![pattern], |row| row.get::<_, String>(0)) else {
            return Vec::new();
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    pub fn get_all_members(&self, name: &str) -> (Vec<DbFunction>, Vec<DbProperty>, Vec<DbEvent>) {
        let mut functions: Vec<DbFunction> = Vec::new();
        let mut properties: Vec<DbProperty> = Vec::new();
        let mut events: Vec<DbEvent> = Vec::new();
        let mut seen_funcs: HashSet<String> = HashSet::new();
        let mut seen_props: HashSet<String> = HashSet::new();
        let mut seen_events: HashSet<String> = HashSet::new();
        for ancestor in self.get_hierarchy(name) {
            for f in self.get_functions(&ancestor) {
                let key = f.name.to_ascii_lowercase();
                if seen_funcs.insert(key) {
                    functions.push(f);
                }
            }
            for p in self.get_properties(&ancestor) {
                let key = p.name.to_ascii_lowercase();
                if seen_props.insert(key) {
                    properties.push(p);
                }
            }
            for e in self.get_events(&ancestor) {
                let key = e.name.to_ascii_lowercase();
                if seen_events.insert(key) {
                    events.push(e);
                }
            }
        }
        (functions, properties, events)
    }
}

fn render_params(ps: &[crate::ast::Parameter]) -> String {
    ps.iter()
        .map(|p| format!("{} {}", p.ty, p.name))
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_to_disk_script(path: &Path) -> Option<DiskScript> {
    let text = std::fs::read_to_string(path).ok()?;
    let result = parse_script(&text);
    let ast = result.ast?;
    let mut funcs: Vec<DbFunction> = ast
        .functions
        .iter()
        .map(|f| DbFunction {
            return_type: f.return_type.clone(),
            name: f.name.clone(),
            params: render_params(&f.params),
        })
        .collect();
    for state in &ast.states {
        for f in &state.functions {
            funcs.push(DbFunction {
                return_type: f.return_type.clone(),
                name: f.name.clone(),
                params: render_params(&f.params),
            });
        }
    }
    Some(DiskScript {
        name: ast.name.clone(),
        extends: ast.parent.clone(),
        functions: funcs,
        properties: ast
            .properties
            .iter()
            .map(|p| DbProperty {
                ty: p.ty.clone(),
                name: p.name.clone(),
            })
            .collect(),
        events: ast
            .events
            .iter()
            .map(|e| DbEvent {
                name: e.name.clone(),
                params: render_params(&e.params),
            })
            .collect(),
        path: Some(path.to_path_buf()),
    })
}

/// Parse `"RetType Func(params); RetType Func2(params); ..."`.
fn parse_functions(raw: &str) -> Vec<DbFunction> {
    let mut out = Vec::new();
    for entry in raw.split(';') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if let Some(f) = parse_function_entry(entry) {
            out.push(f);
        }
    }
    out
}

fn parse_function_entry(entry: &str) -> Option<DbFunction> {
    let lparen = entry.find('(')?;
    let rparen = entry.rfind(')')?;
    if rparen < lparen {
        return None;
    }
    let head = entry[..lparen].trim();
    let params = entry[lparen + 1..rparen].trim().to_owned();
    let mut split = head.splitn(2, char::is_whitespace);
    let return_type = split.next()?.trim().to_owned();
    let name = split.next()?.trim().to_owned();
    if return_type.is_empty() || name.is_empty() {
        return None;
    }
    Some(DbFunction {
        return_type,
        name,
        params,
    })
}

/// Parse `"EventName(params); EventName2(params); ..."`.
fn parse_events(raw: &str) -> Vec<DbEvent> {
    let mut out = Vec::new();
    for entry in raw.split(';') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let Some(lparen) = entry.find('(') else {
            continue;
        };
        let Some(rparen) = entry.rfind(')') else {
            continue;
        };
        if rparen < lparen {
            continue;
        }
        let name = entry[..lparen].trim().to_owned();
        let params = entry[lparen + 1..rparen].trim().to_owned();
        if !name.is_empty() {
            out.push(DbEvent { name, params });
        }
    }
    out
}

/// Parse `"Type Name; Type Name; ..."`.
fn parse_properties(raw: &str) -> Vec<DbProperty> {
    let mut out = Vec::new();
    for entry in raw.split(';') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let mut split = entry.splitn(2, char::is_whitespace);
        let ty = match split.next() {
            Some(t) => t.trim().to_owned(),
            None => continue,
        };
        let name = match split.next() {
            Some(n) => n.trim().to_owned(),
            None => continue,
        };
        if !ty.is_empty() && !name.is_empty() {
            out.push(DbProperty { ty, name });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_function_string() {
        let f = parse_functions("Int GetCount(); Bool IsValid(Int x, String y)");
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].return_type, "Int");
        assert_eq!(f[0].name, "GetCount");
        assert_eq!(f[1].name, "IsValid");
        assert_eq!(f[1].params, "Int x, String y");
    }

    #[test]
    fn parse_event_string() {
        let e = parse_events("OnInit(); OnActivate(ObjectReference akActionRef)");
        assert_eq!(e.len(), 2);
        assert_eq!(e[1].name, "OnActivate");
    }

    #[test]
    fn parse_property_string() {
        let p = parse_properties("Int Count; String Name; Float Threshold");
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].ty, "Int");
        assert_eq!(p[0].name, "Count");
        assert_eq!(p[2].name, "Threshold");
    }

    #[test]
    fn empty_db_supports_disk_fallback_only() {
        let db = ScriptDB::empty();
        assert!(!db.script_exists("Anything"));
        assert!(db.get_functions("Anything").is_empty());
        assert!(db.get_hierarchy("Anything").is_empty());
    }

    #[test]
    fn register_ast_makes_script_discoverable() {
        let db = ScriptDB::empty();
        let r = parse_script(
            "ScriptName Foo extends Bar\nInt Property X Auto\nFunction DoIt()\nEndFunction\n",
        );
        let ast = r.ast.unwrap();
        db.register_ast(&ast);
        assert!(db.script_exists("Foo"));
        assert!(db.has_function("Foo", "DoIt"));
        assert!(db.has_property("Foo", "X"));
        assert_eq!(db.get_extends("Foo").as_deref(), Some("Bar"));
    }
}
