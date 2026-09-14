//! Import-path source resolver for the compiler.
//!
//! Resolves cross-script and inherited references (callee signatures incl.
//! defaults, parent chains, property types) the way `PapyrusCompiler.exe` does:
//! find `<Name>.psc` on the import path (the `-i` dirs: the script's own dir plus
//! the FO4 Base source tree), parse it, and walk the `Extends` chain. It does
//! not use `script_db`, which is the UI/LSP index, not a compiler dependency.
//!
//! Parsed ASTs are cached process-wide: callee sources don't change during a
//! run, and large API scripts (Actor/Form/ObjectReference) would otherwise be
//! re-parsed for every compile.

use crate::ast::{EventDef, FunctionDef, PropertyDef, ScriptNode};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

pub struct SourceResolver {
    dirs: Arc<[PathBuf]>,
    /// AST of the script currently being compiled. It lives in memory (the
    /// caller may pass source text that is on no import dir, or that is newer
    /// than the on-disk copy), so `parsed` must answer from it rather than
    /// from the import path — otherwise the script's own hierarchy is empty
    /// and every self/inherited call resolves to no return type.
    self_ast: Option<Arc<ScriptNode>>,
}

impl SourceResolver {
    pub fn new(import_dirs: &[String]) -> Self {
        let cwd = import_dirs
            .iter()
            .any(|dir| Path::new(dir).is_relative())
            .then(std::env::current_dir)
            .and_then(|result| result.ok());
        Self {
            dirs: import_roots(import_dirs, cwd.as_deref()),
            self_ast: None,
        }
    }

    pub fn with_self_ast(import_dirs: &[String], ast: &ScriptNode) -> Self {
        let cwd = import_dirs
            .iter()
            .any(|dir| Path::new(dir).is_relative())
            .then(std::env::current_dir)
            .and_then(|result| result.ok());
        Self {
            dirs: import_roots(import_dirs, cwd.as_deref()),
            self_ast: Some(Arc::new(ast.clone())),
        }
    }

    fn self_ast_for(&self, name: &str) -> Option<Arc<ScriptNode>> {
        let ast = self.self_ast.as_ref()?;
        ast.name.eq_ignore_ascii_case(name).then(|| ast.clone())
    }

    /// Read `<Name>.psc` from the first import dir that contains it. A `:`
    /// namespace separator maps to a path separator (e.g. `Foo:Bar`).
    pub fn get_source(&self, name: &str) -> Option<String> {
        let rel = name.replace(':', std::path::MAIN_SEPARATOR_STR);
        let rel_path = PathBuf::from(format!("{rel}.psc"));
        for dir in self.dirs.iter() {
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
        // Never routed through the process-global cache: the in-memory AST is
        // per-compile, unlike immutable on-disk callee sources.
        if let Some(ast) = self.self_ast_for(name) {
            return Some(ast);
        }
        let key = CacheKey {
            dirs: Arc::clone(&self.dirs),
            name: name.to_ascii_lowercase(),
        };
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

fn import_roots(import_dirs: &[String], cwd: Option<&std::path::Path>) -> Arc<[PathBuf]> {
    import_dirs
        .iter()
        .map(|dir| {
            let path = PathBuf::from(dir);
            if path.is_absolute() {
                path
            } else if let Some(cwd) = cwd {
                cwd.join(path)
            } else {
                path
            }
        })
        .collect()
}

#[derive(Hash, Eq, PartialEq)]
struct CacheKey {
    dirs: Arc<[PathBuf]>,
    name: String,
}

fn cache() -> &'static Mutex<HashMap<CacheKey, Option<Arc<ScriptNode>>>> {
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, Option<Arc<ScriptNode>>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver_for(import_dirs: &[String], cwd: &std::path::Path) -> SourceResolver {
        SourceResolver {
            dirs: import_roots(import_dirs, Some(cwd)),
            self_ast: None,
        }
    }

    fn property_names(resolver: &SourceResolver, script_name: &str) -> Vec<String> {
        resolver
            .get_properties(script_name)
            .into_iter()
            .map(|property| property.name)
            .collect()
    }

    #[test]
    fn cache_context_isolates_roots_order_and_misses() {
        let nonce = format!(
            "{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock before Unix epoch")
                .as_nanos()
        );
        let base = std::env::temp_dir().join(format!("papyrus-resolver-cache-{nonce}"));
        let root_a = base.join("a");
        let root_b = base.join("b");
        std::fs::create_dir_all(&root_a).expect("create first import root");
        std::fs::create_dir_all(&root_b).expect("create second import root");

        let script_name = format!("ResolverCacheSentinel{nonce}");
        std::fs::write(
            root_a.join(format!("{script_name}.psc")),
            format!("ScriptName {script_name}\nInt Property FirstOnly Auto\n"),
        )
        .expect("write first imported script");
        std::fs::write(
            root_b.join(format!("{script_name}.psc")),
            format!("ScriptName {script_name}\nInt Property SecondOnly Auto\n"),
        )
        .expect("write second imported script");

        let a = resolver_for(&[root_a.to_string_lossy().into_owned()], &base);
        assert_eq!(
            property_names(&a, &script_name),
            vec!["FirstOnly".to_string()]
        );
        let b = resolver_for(&[root_b.to_string_lossy().into_owned()], &base);
        assert_eq!(
            property_names(&b, &script_name),
            vec!["SecondOnly".to_string()]
        );
        let mut legacy_name_only_cache = HashMap::new();
        legacy_name_only_cache.insert(
            script_name.to_ascii_lowercase(),
            property_names(&a, &script_name),
        );
        let legacy_b_properties = legacy_name_only_cache
            .entry(script_name.to_ascii_lowercase())
            .or_insert_with(|| property_names(&b, &script_name));
        assert_eq!(legacy_b_properties, &vec!["FirstOnly".to_string()]);

        let first_order = resolver_for(
            &[
                root_a.to_string_lossy().into_owned(),
                root_b.to_string_lossy().into_owned(),
            ],
            &base,
        );
        let second_order = resolver_for(
            &[
                root_b.to_string_lossy().into_owned(),
                root_a.to_string_lossy().into_owned(),
            ],
            &base,
        );
        assert_eq!(
            property_names(&first_order, &script_name),
            vec!["FirstOnly".to_string()]
        );
        assert_eq!(
            property_names(&second_order, &script_name),
            vec!["SecondOnly".to_string()]
        );

        let relative_root = "relative".to_string();
        let relative_a = root_a.join(&relative_root);
        let relative_b = root_b.join(&relative_root);
        std::fs::create_dir_all(&relative_a).expect("create first relative root");
        std::fs::create_dir_all(&relative_b).expect("create second relative root");
        let relative_script = format!("ResolverRelativeSentinel{nonce}");
        std::fs::write(
            relative_a.join(format!("{relative_script}.psc")),
            format!("ScriptName {relative_script}\nInt Property RelativeA Auto\n"),
        )
        .expect("write first relative script");
        std::fs::write(
            relative_b.join(format!("{relative_script}.psc")),
            format!("ScriptName {relative_script}\nInt Property RelativeB Auto\n"),
        )
        .expect("write second relative script");
        let relative_a_resolver = resolver_for(&[relative_root.clone()], &root_a);
        let relative_b_resolver = resolver_for(&[relative_root], &root_b);
        assert_eq!(
            property_names(&relative_a_resolver, &relative_script),
            vec!["RelativeA".to_string()]
        );
        assert_eq!(
            property_names(&relative_b_resolver, &relative_script),
            vec!["RelativeB".to_string()]
        );

        let missing_script = format!("ResolverMissingSentinel{nonce}");
        assert!(!a.script_exists(&missing_script));
        std::fs::write(
            root_b.join(format!("{missing_script}.psc")),
            format!("ScriptName {missing_script}\n"),
        )
        .expect("write script after first-root miss");
        assert!(b.script_exists(&missing_script));
        std::fs::write(
            root_a.join(format!("{missing_script}.psc")),
            format!("ScriptName {missing_script}\n"),
        )
        .expect("write script after cached miss");
        assert!(!a.script_exists(&missing_script));

        let reuse_script = format!("ResolverReuseSentinel{nonce}");
        let reuse_path = root_a.join(format!("{reuse_script}.psc"));
        std::fs::write(&reuse_path, format!("ScriptName {reuse_script}\n"))
            .expect("write reusable script");
        assert!(a.script_exists(&reuse_script));
        std::fs::remove_file(reuse_path).expect("remove reusable script");
        let a_again = resolver_for(&[root_a.to_string_lossy().into_owned()], &base);
        assert!(a_again.script_exists(&reuse_script));

        std::fs::remove_dir_all(base).expect("remove import roots");
    }
}
