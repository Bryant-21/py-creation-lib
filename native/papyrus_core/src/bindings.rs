//! PyO3 surface for `papyrus_core`.
//!
//! Every entry point clones owned data out of GIL-bound arguments, then runs
//! pure-Rust code under `py.detach(...)` (PyO3 0.28's name for `allow_threads`).
//! Results are JSON strings built inside `detach` and parsed by Python with
//! `json.loads`. No `#[pyclass]` AST types, no Python callbacks, no GIL held
//! during parse, resolve, or DB work.
//!
//! Function names mirror `py_creation_lib/python/creation_lib/esp/native_runtime.py`
//! so the facade in `py_creation_lib/python/creation_lib/papyrus_lsp/native_runtime.py`
//! can dispatch uniformly.

use pyo3::prelude::*;
use pyo3::types::PyModule;
use serde_json::json;

use crate::emitter;
use crate::parser;
use crate::resolver;
use crate::session;

#[pyfunction]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// ---------------------------------------------------------------------------
// Stateless entry points
// ---------------------------------------------------------------------------

#[pyfunction]
fn parse_text(py: Python<'_>, text: &str) -> PyResult<String> {
    let owned = text.to_owned();
    let json_str = py.detach(move || {
        let r = parser::parse_script(&owned);
        json!({
            "ast": r.ast,
            "errors": r.errors.iter().map(|e| json!({
                "line": e.line,
                "col": e.col,
                "message": e.message,
            })).collect::<Vec<_>>(),
        })
        .to_string()
    });
    Ok(json_str)
}

#[pyfunction]
fn validate_filename(py: Python<'_>, path: &str, script_name: &str) -> PyResult<String> {
    let p = path.to_owned();
    let n = script_name.to_owned();
    let json_str = py.detach(move || match parser::validate_filename(&p, &n) {
        Some(e) => json!({
            "line": e.line,
            "col": e.col,
            "message": e.message,
        })
        .to_string(),
        None => "null".to_string(),
    });
    Ok(json_str)
}

/// Render a serialized AST (the JSON shape produced by `parse_text`) back to
/// Papyrus source. Used by the PEX decompiler.
#[pyfunction]
fn emit_script_json(py: Python<'_>, ast_json: &str) -> PyResult<String> {
    let owned = ast_json.to_owned();
    py.detach(
        move || match serde_json::from_str::<crate::ast::ScriptNode>(&owned) {
            Ok(ast) => Ok(emitter::emit_script(&ast)),
            Err(e) => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "invalid script AST JSON: {e}"
            ))),
        },
    )
}

#[pyfunction]
fn parse_pex_bytes(py: Python<'_>, data: &[u8]) -> PyResult<String> {
    let owned = data.to_vec();
    py.detach(move || match crate::pex::parse_pex_bytes(&owned) {
        Ok(payload) => serde_json::to_string(&payload).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("failed to encode PEX JSON: {e}"))
        }),
        Err(e) => Err(pyo3::exceptions::PyValueError::new_err(e)),
    })
}

#[pyfunction]
fn parse_pex_file(py: Python<'_>, path: &str) -> PyResult<String> {
    let owned = path.to_owned();
    py.detach(move || match crate::pex::parse_pex_file(&owned) {
        Ok(payload) => serde_json::to_string(&payload).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("failed to encode PEX JSON: {e}"))
        }),
        Err(e) => Err(pyo3::exceptions::PyValueError::new_err(e)),
    })
}

#[pyfunction]
fn write_pex_bytes(py: Python<'_>, payload_json: &str) -> PyResult<Vec<u8>> {
    let owned = payload_json.to_owned();
    py.detach(move || {
        let payload: crate::pex::PexFilePayload = serde_json::from_str(&owned).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("invalid PEX payload JSON: {e}"))
        })?;
        crate::pex_writer::write_pex_bytes(&payload)
            .map_err(pyo3::exceptions::PyValueError::new_err)
    })
}

/// Compile Papyrus source to `.pex` bytes with header identity fields zeroed.
/// Returns `(meta_json, pex_bytes_or_none)` where `meta_json` carries
/// `{"ok", "diagnostics"}`; raw bytes are returned out-of-band (no base64), as
/// `write_pex_bytes` does.
#[pyfunction]
#[pyo3(signature = (text, imports, game, flags = None, source_path = None))]
fn compile_source(
    py: Python<'_>,
    text: &str,
    imports: Vec<String>,
    game: &str,
    flags: Option<String>,
    source_path: Option<String>,
) -> PyResult<(String, Option<Vec<u8>>)> {
    let t = text.to_owned();
    let g = match game {
        "fo4" => crate::profile::Game::Fo4,
        "skyrimse" => crate::profile::Game::SkyrimSe,
        "starfield" => crate::profile::Game::Starfield,
        other => {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "unknown game {other}"
            )));
        }
    };
    py.detach(move || {
        let r = crate::compiler::compile_source_with_path(
            &t,
            &imports,
            g,
            flags.as_deref(),
            source_path.as_deref(),
        );
        let meta = json!({ "ok": r.ok, "diagnostics": r.diagnostics }).to_string();
        Ok((meta, r.pex_bytes))
    })
}

// ---------------------------------------------------------------------------
// Script DB handles
// ---------------------------------------------------------------------------

#[pyfunction]
#[pyo3(signature = (path, source_dirs = None))]
fn db_open(py: Python<'_>, path: &str, source_dirs: Option<Vec<String>>) -> PyResult<u64> {
    let p = path.to_owned();
    let dirs = source_dirs.unwrap_or_default();
    Ok(py.detach(move || session::db_open(&p, &dirs)))
}

#[pyfunction]
fn db_close(py: Python<'_>, db_id: u64) -> PyResult<bool> {
    Ok(py.detach(move || session::db_close(db_id)))
}

#[pyfunction]
fn db_add_source_dir(py: Python<'_>, db_id: u64, path: &str) -> PyResult<bool> {
    let p = path.to_owned();
    Ok(py.detach(move || match session::db_get(db_id) {
        Some(db) => {
            db.add_source_dir(&p);
            true
        }
        None => false,
    }))
}

#[pyfunction]
fn db_register_session_ast(py: Python<'_>, db_id: u64, session_id: u64) -> PyResult<bool> {
    Ok(py.detach(move || {
        let Some(db) = session::db_get(db_id) else {
            return false;
        };
        let Some(ast) = session::session_ast(session_id) else {
            return false;
        };
        db.register_ast(&ast);
        true
    }))
}

#[pyfunction]
fn db_script_exists(py: Python<'_>, db_id: u64, name: &str) -> PyResult<bool> {
    let n = name.to_owned();
    Ok(py.detach(move || match session::db_get(db_id) {
        Some(db) => db.script_exists(&n),
        None => false,
    }))
}

#[pyfunction]
fn db_get_hierarchy(py: Python<'_>, db_id: u64, name: &str) -> PyResult<String> {
    let n = name.to_owned();
    Ok(py.detach(move || {
        let chain = match session::db_get(db_id) {
            Some(db) => db.get_hierarchy(&n),
            None => Vec::new(),
        };
        json!(chain).to_string()
    }))
}

#[pyfunction]
fn db_register_ast_json(py: Python<'_>, db_id: u64, ast_json: &str) -> PyResult<bool> {
    let owned = ast_json.to_owned();
    py.detach(
        move || match serde_json::from_str::<crate::ast::ScriptNode>(&owned) {
            Ok(ast) => match session::db_get(db_id) {
                Some(db) => {
                    db.register_ast(&ast);
                    Ok(true)
                }
                None => Ok(false),
            },
            Err(e) => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "invalid script AST JSON: {e}"
            ))),
        },
    )
}

#[pyfunction]
fn db_get_extends(py: Python<'_>, db_id: u64, name: &str) -> PyResult<Option<String>> {
    let n = name.to_owned();
    Ok(py.detach(move || session::db_get(db_id).and_then(|db| db.get_extends(&n))))
}

#[pyfunction]
fn db_get_functions(py: Python<'_>, db_id: u64, name: &str) -> PyResult<String> {
    let n = name.to_owned();
    Ok(py.detach(move || {
        let funcs = match session::db_get(db_id) {
            Some(db) => db.get_functions(&n),
            None => Vec::new(),
        };
        let payload: Vec<_> = funcs
            .into_iter()
            .map(|f| json!({"return_type": f.return_type, "name": f.name, "params": f.params}))
            .collect();
        json!(payload).to_string()
    }))
}

#[pyfunction]
fn db_get_events(py: Python<'_>, db_id: u64, name: &str) -> PyResult<String> {
    let n = name.to_owned();
    Ok(py.detach(move || {
        let events = match session::db_get(db_id) {
            Some(db) => db.get_events(&n),
            None => Vec::new(),
        };
        let payload: Vec<_> = events
            .into_iter()
            .map(|e| json!({"name": e.name, "params": e.params}))
            .collect();
        json!(payload).to_string()
    }))
}

#[pyfunction]
fn db_get_properties(py: Python<'_>, db_id: u64, name: &str) -> PyResult<String> {
    let n = name.to_owned();
    Ok(py.detach(move || {
        let props = match session::db_get(db_id) {
            Some(db) => db.get_properties(&n),
            None => Vec::new(),
        };
        let payload: Vec<_> = props
            .into_iter()
            .map(|p| json!({"type": p.ty, "name": p.name}))
            .collect();
        json!(payload).to_string()
    }))
}

#[pyfunction]
fn db_has_function(
    py: Python<'_>,
    db_id: u64,
    script_name: &str,
    func_name: &str,
) -> PyResult<bool> {
    let s = script_name.to_owned();
    let f = func_name.to_owned();
    Ok(py.detach(move || match session::db_get(db_id) {
        Some(db) => db.has_function(&s, &f),
        None => false,
    }))
}

#[pyfunction]
fn db_has_event(py: Python<'_>, db_id: u64, script_name: &str, event_name: &str) -> PyResult<bool> {
    let s = script_name.to_owned();
    let e = event_name.to_owned();
    Ok(py.detach(move || match session::db_get(db_id) {
        Some(db) => db.has_event(&s, &e),
        None => false,
    }))
}

#[pyfunction]
fn db_has_property(
    py: Python<'_>,
    db_id: u64,
    script_name: &str,
    prop_name: &str,
) -> PyResult<bool> {
    let s = script_name.to_owned();
    let p = prop_name.to_owned();
    Ok(py.detach(move || match session::db_get(db_id) {
        Some(db) => db.has_property(&s, &p),
        None => false,
    }))
}

#[pyfunction]
fn db_get_function_return_type(
    py: Python<'_>,
    db_id: u64,
    script_name: &str,
    func_name: &str,
) -> PyResult<Option<String>> {
    let s = script_name.to_owned();
    let f = func_name.to_owned();
    Ok(
        py.detach(move || {
            session::db_get(db_id).and_then(|db| db.get_function_return_type(&s, &f))
        }),
    )
}

#[pyfunction]
fn db_get_script_path(py: Python<'_>, db_id: u64, name: &str) -> PyResult<Option<String>> {
    let n = name.to_owned();
    Ok(py.detach(move || session::db_get(db_id).and_then(|db| db.get_script_path(&n))))
}

#[pyfunction]
fn db_get_source(py: Python<'_>, db_id: u64, name: &str) -> PyResult<Option<String>> {
    let n = name.to_owned();
    Ok(py.detach(move || session::db_get(db_id).and_then(|db| db.get_source(&n))))
}

#[pyfunction]
fn db_search_scripts(py: Python<'_>, db_id: u64, prefix: &str) -> PyResult<String> {
    let p = prefix.to_owned();
    Ok(py.detach(move || {
        let names = match session::db_get(db_id) {
            Some(db) => db.search_scripts(&p),
            None => Vec::new(),
        };
        json!(names).to_string()
    }))
}

#[pyfunction]
fn db_get_all_members(py: Python<'_>, db_id: u64, name: &str) -> PyResult<String> {
    let n = name.to_owned();
    Ok(py.detach(move || {
        let (functions, properties, events) = match session::db_get(db_id) {
            Some(db) => db.get_all_members(&n),
            None => (Vec::new(), Vec::new(), Vec::new()),
        };
        let f_json: Vec<_> = functions
            .into_iter()
            .map(|f| json!({"return_type": f.return_type, "name": f.name, "params": f.params}))
            .collect();
        let p_json: Vec<_> = properties
            .into_iter()
            .map(|p| json!({"type": p.ty, "name": p.name}))
            .collect();
        let e_json: Vec<_> = events
            .into_iter()
            .map(|e| json!({"name": e.name, "params": e.params}))
            .collect();
        json!({"functions": f_json, "properties": p_json, "events": e_json}).to_string()
    }))
}

/// Run the resolver on a serialized AST against a DB handle. Returns a JSON
/// list of diagnostics (same shape as `session_diagnostics`).
#[pyfunction]
#[pyo3(signature = (ast_json, db_id = None))]
fn resolve_ast(py: Python<'_>, ast_json: &str, db_id: Option<u64>) -> PyResult<String> {
    let owned = ast_json.to_owned();
    py.detach(
        move || match serde_json::from_str::<crate::ast::ScriptNode>(&owned) {
            Ok(ast) => {
                let diags = match db_id.and_then(session::db_get) {
                    Some(db) => resolver::resolve(&ast, &db),
                    None => {
                        let empty = crate::script_db::ScriptDB::empty();
                        resolver::resolve(&ast, &empty)
                    }
                };
                Ok(json!(diags).to_string())
            }
            Err(e) => Err(pyo3::exceptions::PyValueError::new_err(format!(
                "invalid script AST JSON: {e}"
            ))),
        },
    )
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

#[pyfunction]
#[pyo3(signature = (uri, text, db_id = None))]
fn session_open(py: Python<'_>, uri: &str, text: &str, db_id: Option<u64>) -> PyResult<u64> {
    let u = uri.to_owned();
    let t = text.to_owned();
    Ok(py.detach(move || session::session_open(u, t, db_id)))
}

#[pyfunction]
fn session_close(py: Python<'_>, sid: u64) -> PyResult<bool> {
    Ok(py.detach(move || session::session_close(sid)))
}

#[pyfunction]
fn session_replace_text(py: Python<'_>, sid: u64, text: &str) -> PyResult<bool> {
    let t = text.to_owned();
    Ok(py.detach(move || session::session_replace_text(sid, t)))
}

#[pyfunction]
fn session_apply_edit(
    py: Python<'_>,
    sid: u64,
    start: usize,
    end: usize,
    replacement: &str,
) -> PyResult<bool> {
    let r = replacement.to_owned();
    Ok(py.detach(move || session::session_apply_edit(sid, start, end, &r)))
}

#[pyfunction]
fn session_text(py: Python<'_>, sid: u64) -> PyResult<Option<String>> {
    Ok(py.detach(move || session::session_text(sid)))
}

#[pyfunction]
fn session_diagnostics(py: Python<'_>, sid: u64) -> PyResult<String> {
    Ok(py.detach(move || {
        let diags = session::session_diagnostics(sid).unwrap_or_default();
        json!(diags).to_string()
    }))
}

#[pyfunction]
fn session_ast(py: Python<'_>, sid: u64) -> PyResult<String> {
    Ok(py.detach(move || {
        let ast = session::session_ast(sid);
        json!(ast).to_string()
    }))
}

#[pyfunction]
fn session_document_symbols(py: Python<'_>, sid: u64) -> PyResult<String> {
    Ok(py.detach(move || {
        let Some(ast) = session::session_ast(sid) else {
            return "[]".into();
        };
        let mut symbols: Vec<serde_json::Value> = Vec::new();
        symbols.push(json!({
            "name": ast.name,
            "kind": "Script",
            "pos": ast.pos,
        }));
        for p in &ast.properties {
            symbols.push(json!({
                "name": p.name,
                "kind": "Property",
                "type": p.ty,
                "pos": p.pos,
            }));
        }
        for v in &ast.variables {
            symbols.push(json!({
                "name": v.name,
                "kind": "Variable",
                "type": v.ty,
                "pos": v.pos,
            }));
        }
        for f in &ast.functions {
            symbols.push(json!({
                "name": f.name,
                "kind": "Function",
                "return_type": f.return_type,
                "pos": f.pos,
            }));
        }
        for e in &ast.events {
            symbols.push(json!({
                "name": e.name,
                "kind": "Event",
                "pos": e.pos,
            }));
        }
        for s in &ast.states {
            symbols.push(json!({
                "name": s.name,
                "kind": "State",
                "is_auto": s.is_auto,
                "pos": s.pos,
                "children": {
                    "functions": s.functions.iter().map(|f| json!({
                        "name": f.name, "kind": "Function", "return_type": f.return_type, "pos": f.pos,
                    })).collect::<Vec<_>>(),
                    "events": s.events.iter().map(|e| json!({
                        "name": e.name, "kind": "Event", "pos": e.pos,
                    })).collect::<Vec<_>>(),
                },
            }));
        }
        json!(symbols).to_string()
    }))
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(version, m)?)?;

    // Stateless
    m.add_function(wrap_pyfunction!(parse_text, m)?)?;
    m.add_function(wrap_pyfunction!(validate_filename, m)?)?;
    m.add_function(wrap_pyfunction!(emit_script_json, m)?)?;
    m.add_function(wrap_pyfunction!(parse_pex_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(parse_pex_file, m)?)?;
    m.add_function(wrap_pyfunction!(write_pex_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(compile_source, m)?)?;

    // DB
    m.add_function(wrap_pyfunction!(db_open, m)?)?;
    m.add_function(wrap_pyfunction!(db_close, m)?)?;
    m.add_function(wrap_pyfunction!(db_add_source_dir, m)?)?;
    m.add_function(wrap_pyfunction!(db_register_session_ast, m)?)?;
    m.add_function(wrap_pyfunction!(db_register_ast_json, m)?)?;
    m.add_function(wrap_pyfunction!(db_script_exists, m)?)?;
    m.add_function(wrap_pyfunction!(db_get_hierarchy, m)?)?;
    m.add_function(wrap_pyfunction!(db_get_extends, m)?)?;
    m.add_function(wrap_pyfunction!(db_get_functions, m)?)?;
    m.add_function(wrap_pyfunction!(db_get_events, m)?)?;
    m.add_function(wrap_pyfunction!(db_get_properties, m)?)?;
    m.add_function(wrap_pyfunction!(db_has_function, m)?)?;
    m.add_function(wrap_pyfunction!(db_has_event, m)?)?;
    m.add_function(wrap_pyfunction!(db_has_property, m)?)?;
    m.add_function(wrap_pyfunction!(db_get_function_return_type, m)?)?;
    m.add_function(wrap_pyfunction!(db_get_script_path, m)?)?;
    m.add_function(wrap_pyfunction!(db_get_source, m)?)?;
    m.add_function(wrap_pyfunction!(db_search_scripts, m)?)?;
    m.add_function(wrap_pyfunction!(db_get_all_members, m)?)?;
    m.add_function(wrap_pyfunction!(resolve_ast, m)?)?;

    // Sessions
    m.add_function(wrap_pyfunction!(session_open, m)?)?;
    m.add_function(wrap_pyfunction!(session_close, m)?)?;
    m.add_function(wrap_pyfunction!(session_replace_text, m)?)?;
    m.add_function(wrap_pyfunction!(session_apply_edit, m)?)?;
    m.add_function(wrap_pyfunction!(session_text, m)?)?;
    m.add_function(wrap_pyfunction!(session_diagnostics, m)?)?;
    m.add_function(wrap_pyfunction!(session_ast, m)?)?;
    m.add_function(wrap_pyfunction!(session_document_symbols, m)?)?;
    Ok(())
}
