use crate::{
    check_cancel,
    database::{Database, error, records},
    encoding,
    executables::text,
    invalid,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PySet},
};
use rusqlite::params;
use serde_json::{Value, json};
use std::path::PathBuf;

pub fn directory<'py>(
    library: &Bound<'py, PyAny>,
    source: &str,
) -> PyResult<(Bound<'py, PyAny>, Value)> {
    if source.trim().is_empty() {
        return Err(invalid("Choose an existing source folder first."));
    }
    let py = library.py();
    let root = py
        .import("pathlib")?
        .getattr("Path")?
        .call1((source,))?
        .call_method0("absolute")?;
    let path = root.extract::<PathBuf>()?;
    if !crate::executables::local_mod_target(path.clone())? {
        return Err(invalid(
            "Choose an ordinary local source folder; network/drive roots are unsupported.",
        ));
    }
    crate::reject_links(&path)?;
    if !path.is_dir() {
        return Err(invalid("The selected source folder is unavailable."));
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item("strict", true)?;
    let root = root.call_method("resolve", (), Some(&kwargs))?;
    let owned = library.getattr("root")?;
    if root.eq(&owned)?
        || root
            .call_method1("is_relative_to", (&owned,))?
            .is_truthy()?
        || owned
            .call_method1("is_relative_to", (&root,))?
            .is_truthy()?
    {
        return Err(invalid(
            "Reference folders must be separate from the manager-owned library.",
        ));
    }
    let path = root.extract::<PathBuf>()?;
    let marker = library.getattr("MARKER")?.extract::<String>()?;
    if path
        .ancestors()
        .any(|parent| parent.join(&marker).is_file())
    {
        return Err(invalid(
            "Reference folders must be outside manager-owned libraries.",
        ));
    }
    let info = root.call_method0("stat")?;
    let identity = json!([
        info.getattr("st_dev")?.extract::<u64>()?,
        info.getattr("st_ino")?.extract::<u64>()?
    ]);
    Ok((root, identity))
}

#[pyfunction]
fn reference_directory(library: &Bound<'_, PyAny>, source: &str) -> PyResult<String> {
    let (root, identity) = directory(library, source)?;
    Ok(json!([root.str()?.to_string(), identity]).to_string())
}

pub fn inventory(
    root: &Bound<'_, PyAny>,
    cancel: Option<&Bound<'_, PyAny>>,
    validate: bool,
) -> PyResult<(usize, u64, String)> {
    let py = root.py();
    let directories = PySet::empty(py)?;
    let mut tree = crate::paths::FileTree::new(
        root.extract()?,
        cancel.map(|v| v.clone().unbind()),
        Some(directories.clone().into_any().unbind()),
        200000,
    )?;
    let path_type = py.import("pathlib")?.getattr("Path")?;
    let mut files = Vec::new();
    let mut size = 0;
    let mut names = Vec::new();
    while let Some((name, path)) = tree.__next__(py)? {
        let info = path_type.call1((path,))?.call_method0("lstat")?;
        let length = info.getattr("st_size")?.extract::<u64>()?;
        size += length;
        names.push(name.clone());
        files.push(json!([
            name,
            info.getattr("st_dev")?.extract::<u64>()?,
            info.getattr("st_ino")?.extract::<u64>()?,
            length,
            info.getattr("st_mtime_ns")?.extract::<i64>()?
        ]));
        if files.len() > 200000 {
            return Err(invalid(if validate {
                "Target source review exceeds 200,000 files."
            } else {
                "Source review exceeds its 200,000-file limit."
            }));
        }
    }
    if files.is_empty() {
        return Err(invalid("Choose a source folder containing mod files."));
    }
    check_cancel(cancel)?;
    if validate {
        crate::paths::validate_destinations(names)?;
    }
    files.sort_by(|a, b| a[0].as_str().cmp(&b[0].as_str()));
    let mut directories = directories
        .iter()
        .map(|v| v.extract::<String>())
        .collect::<PyResult<Vec<_>>>()?;
    directories.sort();
    Ok((
        files.len(),
        size,
        encoding::digest(&json!([files, directories]), false, false),
    ))
}

fn state(db: &Database, profile: &str, id: &str) -> PyResult<Value> {
    records(&*db.connection()?, "SELECT m.id,m.name,m.source,m.kind FROM mods m JOIN entries e ON e.mod_id=m.id WHERE e.profile_id=? AND m.id=?", params![profile, id])?.into_iter().next().filter(|v| v["kind"] == "external").ok_or_else(|| invalid("Choose a referenced source mod in this local profile."))
}

#[pyfunction]
fn reference_review(
    db: &Database,
    library: &Bound<'_, PyAny>,
    profile: &str,
    id: &str,
    source: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let (root, identity) = directory(library, source)?;
    let data = state(db, profile, id)?;
    let profiles = db
        .connection()?
        .query_row("SELECT count(*) FROM entries WHERE mod_id=?", [id], |row| {
            row.get::<_, u64>(0)
        })
        .map_err(error)?;
    let (count, size, fingerprint) = inventory(&root, Some(cancel), false)?;
    let source = root.str()?.to_string();
    if directory(library, &source)?.1 != identity {
        return Err(invalid(
            "Source folder changed during review. Review it again.",
        ));
    }
    Ok(json!({"library_id":library.getattr("library_id")?.extract::<String>()?, "profile_id":profile,"mod_id":id,"name":data["name"],"old_source":data["source"],"source":source,"files":count,"bytes":size,"profiles":profiles,"expected":encoding::digest(&data,false,false),"fingerprint":fingerprint,"directory_identity":identity}).to_string())
}

#[pyfunction]
fn reference_apply(
    db: &Database,
    library: &Bound<'_, PyAny>,
    plan: &str,
    accept: bool,
    cancel: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let plan = encoding::parse(plan)?;
    if !accept {
        return Err(invalid(
            "Confirm that this folder is the same referenced mod before relinking it.",
        ));
    }
    if plan["library_id"] != library.getattr("library_id")?.extract::<String>()? {
        return Err(invalid("Library changed; review the source again."));
    }
    library.call_method0("_runtime_write_guard")?;
    let source = text(&plan, "source");
    let (root, identity) = directory(library, source)?;
    if identity != plan["directory_identity"]
        || inventory(&root, Some(cancel), false)?.2 != text(&plan, "fingerprint")
    {
        return Err(invalid(
            "Source files changed after review. Review this folder again.",
        ));
    }
    db.connection()?
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(error)?;
    let profile = text(&plan, "profile_id");
    let id = text(&plan, "mod_id");
    let old = state(db, profile, id)?;
    if encoding::digest(&old, false, false) != text(&plan, "expected")
        || directory(library, source)?.1 != identity
    {
        return Err(invalid(
            "Reference metadata or source identity changed. Review it again.",
        ));
    }
    check_cancel(Some(cancel))?;
    db.connection()?.execute_batch("CREATE TABLE IF NOT EXISTS reference_relinks(sequence INTEGER PRIMARY KEY AUTOINCREMENT, profile_id TEXT NOT NULL,mod_id TEXT NOT NULL,before_state TEXT NOT NULL,expected TEXT NOT NULL)").map_err(error)?;
    db.connection()?
        .execute("UPDATE mods SET source=? WHERE id=?", params![source, id])
        .map_err(error)?;
    let expected = encoding::digest(&state(db, profile, id)?, false, false);
    db.connection()?.execute("INSERT INTO reference_relinks(profile_id,mod_id,before_state,expected) VALUES (?,?,?,?)", params![profile,id,old.to_string(),expected]).map_err(error)?;
    checkpoint.call0()?;
    check_cancel(Some(cancel))?;
    if directory(library, source)?.1 != identity {
        return Err(invalid("Source directory changed before commit."));
    }
    library.call_method0("_runtime_write_guard")?;
    Ok(())
}

fn has_history(db: &Database) -> PyResult<bool> {
    Ok(!records(
        &*db.connection()?,
        "SELECT 1 FROM sqlite_master WHERE name='reference_relinks' AND type='table'",
        [],
    )?
    .is_empty())
}

#[pyfunction]
fn reference_undoable(db: &Database, profile: &str) -> PyResult<Vec<String>> {
    if !has_history(db)? {
        return Ok(Vec::new());
    }
    Ok(records(&*db.connection()?, "SELECT DISTINCT r.mod_id FROM reference_relinks r JOIN entries e ON r.mod_id=e.mod_id WHERE r.profile_id=? AND e.profile_id=?", params![profile,profile])?.iter().map(|v| text(v,"mod_id").into()).collect())
}

#[pyfunction]
fn reference_undo(
    db: &Database,
    library: &Bound<'_, PyAny>,
    profile: &str,
    id: &str,
    cancel: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<()> {
    library.call_method0("_runtime_write_guard")?;
    db.connection()?
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(error)?;
    if !has_history(db)? {
        return Err(invalid("This reference has no source relink to undo."));
    }
    let saved = records(&*db.connection()?, "SELECT * FROM reference_relinks WHERE profile_id=? AND mod_id=? ORDER BY sequence DESC LIMIT 1", params![profile,id])?.into_iter().next().ok_or_else(|| invalid("Reference changed since relinking, or no relink exists. Undo is blocked."))?;
    if encoding::digest(&state(db, profile, id)?, false, false) != text(&saved, "expected") {
        return Err(invalid(
            "Reference changed since relinking, or no relink exists. Undo is blocked.",
        ));
    }
    let old = encoding::parse(text(&saved, "before_state"))?;
    check_cancel(Some(cancel))?;
    db.connection()?
        .execute(
            "UPDATE mods SET source=? WHERE id=?",
            params![text(&old, "source"), id],
        )
        .map_err(error)?;
    db.connection()?
        .execute(
            "DELETE FROM reference_relinks WHERE sequence=?",
            [saved["sequence"].as_i64().unwrap_or_default()],
        )
        .map_err(error)?;
    checkpoint.call0()?;
    check_cancel(Some(cancel))?;
    library.call_method0("_runtime_write_guard")?;
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(reference_directory, m)?)?;
    m.add_function(wrap_pyfunction!(reference_review, m)?)?;
    m.add_function(wrap_pyfunction!(reference_apply, m)?)?;
    m.add_function(wrap_pyfunction!(reference_undoable, m)?)?;
    m.add_function(wrap_pyfunction!(reference_undo, m)?)?;
    Ok(())
}
