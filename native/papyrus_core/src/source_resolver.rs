//! Import-path source resolver for the compiler.
//!
//! Cross-script and inherited references (callee signatures incl. defaults,
//! parent chains, property types) are resolved exactly the way
//! `PapyrusCompiler.exe` does it: by locating `<Name>.psc` on the import path
//! (the `-i` dirs — the script's own dir plus the FO4 Base source tree),
//! lexing+parsing it, and walking the `Extends` chain. This deliberately does
//! NOT use `script_db` (that is the UI/LSP index, not a compiler dependency).
//!
//! Parsed ASTs are cached process-globally: callee sources are immutable during
//! a run and are referenced by nearly every compiled script, so caching avoids
//! re-parsing large API scripts (Actor/Form/ObjectReference) once per compile.

use crate::ast::{EventDef, FunctionDef, PropertyDef, ScriptNode};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

pub struct SourceResolver {
    dirs: Vec<PathBuf>,
}

impl SourceResolver {
    pub fn new(import_dirs: &[String]) -> Self {
        Self {
            dirs: import_dirs.iter().map(PathBuf::from).collect(),
        }
    }

    /// Read `<Name>.psc` from the first import dir that contains it. A `:`
    /// namespace separator maps to a path separator (e.g. `Foo:Bar`).
    pub fn get_source(&self, name: &str) -> Option<String> {
        let rel = name.replace(':', std::path::MAIN_SEPARATOR_STR);
        let rel_path = PathBuf::from(format!("{rel}.psc"));
        for dir in &self.dirs {
            if let Ok(s) = std::fs::read_to_string(dir.join(&rel_path)) {
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
        None
    }

    /// Parse a script's source, memoized for the process lifetime.
    pub fn parsed(&self, name: &str) -> Option<Arc<ScriptNode>> {
        let key = name.to_ascii_lowercase();
        if let Some(cached) = cache().lock().unwrap().get(&key) {
            return cached.clone();
        }
        let parsed = self
            .get_source(name)
            .and_then(|src| crate::parser::parse_script(&src).ast.map(Arc::new));
        cache().lock().unwrap().insert(key, parsed.clone());
        parsed
    }

    /// A script "exists" iff its source is locatable on the import path.
    pub fn script_exists(&self, name: &str) -> bool {
        self.parsed(name).is_some()
    }

    /// `name` followed by its transitive `Extends` ancestors (canonical-cased
    /// from each parsed source). Mirrors `ScriptDB::get_hierarchy`: includes the
    /// starting name, caps at 20, and guards against cycles.
    pub fn get_hierarchy(&self, name: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut current = name.to_owned();
        let mut seen: HashSet<String> = HashSet::new();
        for _ in 0..20 {
            if !seen.insert(current.to_ascii_lowercase()) {
                break;
            }
            let Some(ast) = self.parsed(&current) else {
                break;
            };
            chain.push(ast.name.clone());
            match ast.parent.as_deref().filter(|s| !s.is_empty()) {
                Some(e) => current = e.to_string(),
                None => break,
            }
        }
        chain
    }

    /// Return type of `func` resolved up `script`'s hierarchy (the first
    /// declaration wins). Mirrors `ScriptDB::get_function_return_type`.
    pub fn get_function_return_type(&self, script: &str, func: &str) -> Option<String> {
        for ancestor in self.get_hierarchy(script) {
            if let Some(ast) = self.parsed(&ancestor) {
                if let Some(f) = ast
                    .functions
                    .iter()
                    .find(|f| f.name.eq_ignore_ascii_case(func))
                {
                    return Some(f.return_type.clone());
                }
            }
        }
        None
    }

    pub fn get_event(&self, script: &str, event: &str) -> Option<(String, EventDef)> {
        for ancestor in self.get_hierarchy(script) {
            if let Some(ast) = self.parsed(&ancestor) {
                if let Some(e) = ast
                    .events
                    .iter()
                    .find(|e| e.name.eq_ignore_ascii_case(event))
                {
                    return Some((ast.name.clone(), e.clone()));
                }
            }
        }
        None
    }

    /// A script's own declared properties (not hierarchy-walked), matching
    /// `ScriptDB::get_properties`.
    pub fn get_properties(&self, script: &str) -> Vec<PropertyDef> {
        self.parsed(script)
            .map(|a| a.properties.clone())
            .unwrap_or_default()
    }

    pub fn has_struct(&self, script: &str, struct_name: &str) -> bool {
        self.parsed(script)
            .map(|a| {
                a.structs
                    .iter()
                    .any(|s| s.name.eq_ignore_ascii_case(struct_name))
            })
            .unwrap_or(false)
    }

    /// Resolve a bare call against `Import`ed scripts: the first imported script
    /// declaring a GLOBAL function `func`. Returns its canonical script name and
    /// the matching `FunctionDef` (for return type + params/defaults).
    pub fn find_imported_global(
        &self,
        imports: &[String],
        func: &str,
    ) -> Option<(String, FunctionDef)> {
        for imp in imports {
            if let Some(ast) = self.parsed(imp) {
                if let Some(f) = ast
                    .functions
                    .iter()
                    .find(|f| f.is_global && f.name.eq_ignore_ascii_case(func))
                {
                    return Some((ast.name.clone(), f.clone()));
                }
            }
        }
        None
    }
}

fn cache() -> &'static Mutex<HashMap<String, Option<Arc<ScriptNode>>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<ScriptNode>>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}
