use crate::{
    categories, check_cancel,
    database::{Database, error, records},
    encoding,
    executables::text,
    invalid,
    mo2_inspection::section,
};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict},
};
use rusqlite::{params, params_from_iter};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const LIMIT: usize = 1024 * 1024;
const FIELDS: &[&str] = &["version", "notes", "tags", "categories", "nexus"];
const COLUMNS: &[&str] = &[
    "version",
    "notes",
    "tags",
    "category",
    "categories_json",
    "nexus_game",
    "nexus_mod_id",
    "nexus_file_id",
];

fn state(db: &Database, profile: &str, id: &str) -> PyResult<Value> {
    let row = records(
        &*db.connection()?,
        "SELECT m.* FROM mods m JOIN entries e ON e.mod_id=m.id WHERE e.profile_id=? AND m.id=?",
        params![profile, id],
    )?
    .into_iter()
    .next()
    .filter(|v| v["kind"] == "external")
    .ok_or_else(|| invalid("Choose a referenced mod in this local profile."))?;
    let memberships:Vec<_>=records(&*db.connection()?,"SELECT profile_id,priority,enabled,group_name FROM entries WHERE mod_id=? ORDER BY profile_id",[id])?.iter().map(|v|json!([v["profile_id"],v["priority"],v["enabled"],v["group_name"]])).collect();
    let taxonomy: Vec<_> = records(
        &*db.connection()?,
        "SELECT name,nexus_id FROM categories ORDER BY name COLLATE NOCASE",
        [],
    )?
    .iter()
    .map(|v| json!([v["name"], v["nexus_id"]]))
    .collect();
    Ok(json!({"mod":row,"memberships":memberships,"taxonomy":taxonomy}))
}

fn stamp(info: &Bound<'_, PyAny>) -> PyResult<Value> {
    Ok(json!([
        info.getattr("st_dev")?.extract::<u64>()?,
        info.getattr("st_ino")?.extract::<u64>()?,
        info.getattr("st_size")?.extract::<u64>()?,
        info.getattr("st_mtime_ns")?.extract::<i64>()?
    ]))
}

fn file(py: Python<'_>, path: &Path, cancel: &Bound<'_, PyAny>) -> PyResult<(String, Value)> {
    let object = py
        .import("pathlib")?
        .getattr("Path")?
        .call1((path,))?
        .call_method0("absolute")?;
    let path = object.extract::<PathBuf>()?;
    if !crate::executables::local_mod_target(path.clone())? {
        return Err(invalid("Metadata must be an ordinary local file."));
    }
    crate::reject_links(&path)?;
    let initial = object.call_method0("stat")?;
    if initial.getattr("st_mode")?.extract::<u32>()? & 0xf000 != 0x8000
        || initial.getattr("st_size")?.extract::<u64>()? > LIMIT as u64
    {
        return Err(invalid(
            "Metadata requires an ordinary file of at most 1 MiB.",
        ));
    }
    check_cancel(Some(cancel))?;
    let raw = crate::python_io::with_context(&object.call_method1("open", ("rb",))?, |stream| {
        Ok(stream.call_method1("read", (LIMIT + 1,))?.unbind())
    })?;
    let raw = raw.bind(py);
    crate::reject_links(&path)?;
    let current = stamp(&object.call_method0("stat")?)?;
    let bytes = raw.cast::<PyBytes>()?.as_bytes();
    if bytes.len() > LIMIT || stamp(&initial)? != current {
        return Err(invalid(
            "Metadata changed during reading or exceeds 1 MiB. Review again.",
        ));
    }
    check_cancel(Some(cancel))?;
    let decoded = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        raw.call_method1("decode", ("utf-16",))?
    } else {
        match raw.call_method1("decode", ("utf-8-sig",)) {
            Ok(value) => value,
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyUnicodeDecodeError>(py) => {
                raw.call_method1("decode", ("cp1252",))?
            }
            Err(e) => return Err(e),
        }
    };
    Ok((
        decoded.extract()?,
        json!([
            path.to_string_lossy(),
            current,
            format!("{:x}", Sha256::digest(bytes))
        ]),
    ))
}

fn scalar(raw: &str) -> PyResult<String> {
    let mut raw = raw.trim();
    if raw.starts_with("@ByteArray(") && raw.ends_with(')') {
        return Ok(crate::mo2_inspection::mo2_qt_string(raw));
    }
    if raw.starts_with("@@") {
        raw = &raw[1..];
    } else if raw.starts_with('@') {
        return Err(invalid(
            "This encoded QSettings value needs an unsupported Qt type decoder.",
        ));
    }
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        raw = &raw[1..raw.len() - 1];
    }
    let chars: Vec<_> = raw.chars().collect();
    let mut result = String::new();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        index += 1;
        if c != '\\' || index == chars.len() || chars[index] == '\n' {
            result.push(c);
            continue;
        }
        let token = chars[index];
        index += 1;
        if token == 'x' && chars.get(index).is_some_and(|c| c.is_ascii_hexdigit()) {
            let mut value = 0;
            let mut count = 0;
            while count < 4 && chars.get(index).is_some_and(|c| c.is_ascii_hexdigit()) {
                value = value * 16 + chars[index].to_digit(16).unwrap();
                index += 1;
                count += 1;
            }
            result.push(
                char::from_u32(value)
                    .ok_or_else(|| invalid("Metadata contains invalid Unicode."))?,
            );
        } else {
            result.push(match token {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                _ => {
                    return Err(invalid(
                        "Metadata contains an unsupported QSettings escape.",
                    ));
                }
            });
        }
    }
    Ok(result)
}

fn identifier(raw: &str) -> PyResult<u64> {
    let value = scalar(raw)?;
    if value == "-1" {
        return Ok(0);
    }
    if value.is_empty() || value.len() > 10 || !value.bytes().all(|v| v.is_ascii_digit()) {
        return Err(invalid(
            "Nexus IDs must be within the supported nonnegative integer range.",
        ));
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|v| *v <= 2_147_483_647)
        .ok_or_else(|| invalid("Nexus IDs must be within the supported nonnegative integer range."))
}

fn association(
    parser: &Bound<'_, PyAny>,
    general: &BTreeMap<String, String>,
    old: &Value,
) -> PyResult<(Value, String)> {
    let id = identifier(&general["modid"])?;
    let domain = if let Some(raw) = general.get("gamename") {
        crate::nexus::nexus_game_domain(&scalar(raw)?)
    } else {
        text(old, "nexus_game").into()
    };
    if id != 0 && domain.is_empty() {
        return Err(invalid(
            "A nonzero Nexus mod ID needs an explicit supported game context.",
        ));
    }
    if id != 0
        && categories::fold(&scalar(
            general
                .get("repository")
                .map(String::as_str)
                .unwrap_or("Nexus"),
        )?) != "nexus"
    {
        return Err(invalid("This repository is not a Nexus association."));
    }
    let mut warning = String::new();
    let mut file_id = 0;
    if parser
        .call_method1("has_section", ("installedFiles",))?
        .is_truthy()?
    {
        let installed = section(parser, "installedFiles")?;
        let count = identifier(installed.get("size").map(String::as_str).unwrap_or("0"))?;
        if count > 128 {
            return Err(invalid(
                "Installed-file metadata exceeds its 128-association limit.",
            ));
        }
        for key in installed.keys() {
            if let Some((index, key)) = key.split_once('\\') {
                if matches!(key, "modid" | "fileid")
                    && !index.is_empty()
                    && !index.starts_with('0')
                    && index.bytes().all(|v| v.is_ascii_digit())
                    && index.parse::<u64>().map_or(true, |v| v > count)
                {
                    return Err(invalid(
                        "The installedFiles array differs from its declared size.",
                    ));
                }
            }
        }
        let mut pairs = BTreeSet::new();
        for index in 1..=count {
            let mod_key = format!("{index}\\modid");
            let file_key = format!("{index}\\fileid");
            let (Some(mod_id), Some(file)) = (installed.get(&mod_key), installed.get(&file_key))
            else {
                return Err(invalid(
                    "The installedFiles array has an incomplete association.",
                ));
            };
            let pair = (identifier(mod_id)?, identifier(file)?);
            if pair.0 != 0 && pair.1 != 0 {
                pairs.insert(pair);
            }
        }
        if pairs.len() == 1 && pairs.first().unwrap().0 == id {
            file_id = pairs.first().unwrap().1;
        } else if !pairs.is_empty() {
            warning="Multiple or foreign installed-file identities cannot be represented by one file ID; it will be cleared.".into();
        }
    } else if let Some(raw) = general.get("fileid") {
        file_id = identifier(raw)?;
    }
    Ok((
        json!([domain, id, if id != 0 { file_id } else { 0 }]),
        warning,
    ))
}

fn taxonomy(raw: &str) -> PyResult<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for line in raw.lines().filter(|v| !v.trim().is_empty()) {
        let parts: Vec<_> = line.split('|').collect();
        let id = parts[0].trim();
        if parts.len() < 2
            || id.is_empty()
            || id.len() > 10
            || id.starts_with('0')
            || !id.bytes().all(|v| v.is_ascii_digit())
        {
            return Err(invalid(
                "Category mapping needs MO2 categories.dat ID|name records.",
            ));
        }
        let name = categories::name(&json!(parts[1]))?;
        if values.get(id).is_some_and(|v| v != &name) {
            return Err(invalid(
                "Category mapping contains conflicting duplicate IDs.",
            ));
        }
        values.insert(id.into(), name);
        if values.len() > 5000 {
            return Err(invalid("Category mapping exceeds its 5,000-record limit."));
        }
    }
    Ok(values)
}

#[pyfunction]
fn source_metadata_review(
    db: &Database,
    library: &Bound<'_, PyAny>,
    profile: &str,
    id: &str,
    category_file: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let py = library.py();
    db.connection()?.execute_batch("BEGIN").map_err(error)?;
    let state = state(db, profile, id)?;
    db.connection()?.execute_batch("COMMIT").map_err(error)?;
    let old = &state["mod"];
    if text(old, "source").is_empty() {
        return Err(invalid(
            "This reference has no available source folder. Relink it first.",
        ));
    }
    let (raw, stamp) = file(
        py,
        &PathBuf::from(text(old, "source")).join("meta.ini"),
        cancel,
    )?;
    let mut inputs = vec![stamp];
    let module = py.import("configparser")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("interpolation", py.None())?;
    kwargs.set_item("strict", true)?;
    let parser = module.getattr("ConfigParser")?.call((), Some(&kwargs))?;
    if let Err(e) = parser.call_method1("read_string", (raw,)) {
        if e.matches(py, module.getattr("Error")?)? {
            return Err(invalid(format!(
                "Cannot parse source meta.ini: {}",
                e.value(py).str()?
            )));
        }
        return Err(e);
    }
    if !parser
        .call_method1("has_section", ("General",))?
        .is_truthy()?
    {
        return Err(invalid("Source metadata needs a [General] section."));
    }
    let general = section(&parser, "General")?;
    let mut mapping = BTreeMap::new();
    let mut mapping_error = String::new();
    if !category_file.is_empty() {
        let (raw, stamp) = file(py, Path::new(category_file), cancel)?;
        inputs.push(stamp);
        match taxonomy(&raw) {
            Ok(value) => mapping = value,
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyValueError>(py) => {
                mapping_error = e.value(py).str()?.to_string()
            }
            Err(e) => return Err(e),
        }
    }
    let mut current = json!({"version":old["version"],"notes":old["notes"],"tags":old["tags"]});
    current["categories"] = json!(categories::memberships(
        text(old, "category"),
        text(old, "categories_json")
    )?);
    current["nexus"] = json!([old["nexus_game"], old["nexus_mod_id"], old["nexus_file_id"]]);
    let mut fields = Vec::new();
    for name in FIELDS {
        check_cancel(Some(cancel))?;
        let key = match *name {
            "categories" => "category",
            "nexus" => "modid",
            name => name,
        };
        let mut value = current[*name].clone();
        let mut issue = String::new();
        let mut warning = String::new();
        let result = (|| -> PyResult<()> {
            let raw = general.get(key).ok_or_else(|| {
                invalid("Field is absent from source metadata; its local value is preserved.")
            })?;
            match *name {
                "version" | "notes" => {
                    value = crate::metadata::field_value(name, json!(scalar(raw)?))?
                }
                "tags" => {
                    value = scalar(raw)?.into();
                    let content = value.as_str().unwrap();
                    if content.chars().count() > 4096 || content.chars().any(|c| c < ' ') {
                        return Err(invalid(
                            "Tags allow 4,096 characters without control characters.",
                        ));
                    }
                }
                "nexus" => {
                    (value, warning) = association(&parser, &general, old)?;
                }
                _ => {
                    let decoded = scalar(raw)?;
                    let ids: Vec<_> = decoded
                        .split(',')
                        .map(str::trim)
                        .filter(|v| !v.is_empty() && *v != "0")
                        .collect();
                    if !mapping_error.is_empty() {
                        return Err(invalid(&mapping_error));
                    }
                    let unknown: Vec<_> = ids
                        .iter()
                        .filter(|v| !mapping.contains_key(**v))
                        .copied()
                        .take(32)
                        .collect();
                    if !unknown.is_empty() {
                        return Err(invalid(format!(
                            "Choose categories.dat containing source category IDs: {}",
                            unknown.join(", ")
                        )));
                    }
                    let configured: BTreeMap<_, _> = state["taxonomy"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| {
                            (
                                categories::fold(v[0].as_str().unwrap()),
                                v[0].as_str().unwrap(),
                            )
                        })
                        .collect();
                    let names = categories::normalized(json!(
                        ids.iter().map(|v| &mapping[*v]).collect::<Vec<_>>()
                    ))?;
                    value = json!(
                        names
                            .iter()
                            .map(|v| configured.get(&categories::fold(v)).copied().unwrap_or(v))
                            .collect::<Vec<_>>()
                    );
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            if e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
                || e.is_instance_of::<pyo3::exceptions::PyUnicodeError>(py)
            {
                issue = e.value(py).str()?.to_string();
            } else {
                return Err(e);
            }
        }
        fields.push(json!({"name":name,"current":current[*name],"value":value,"issue":issue,"warning":warning}));
    }
    check_cancel(Some(cancel))?;
    Ok(json!({"library_id":library.getattr("library_id")?.extract::<String>()?,"profile_id":profile,"mod_id":id,"name":old["name"],"profiles":state["memberships"].as_array().unwrap().len(),"source":old["source"],"expected":encoding::digest(&state,false,false),"inputs":inputs,"fields":fields}).to_string())
}

fn inputs(py: Python<'_>, plan: &Value, cancel: &Bound<'_, PyAny>, message: &str) -> PyResult<()> {
    for item in plan["inputs"]
        .as_array()
        .ok_or_else(|| invalid("Invalid metadata input review."))?
    {
        if file(py, Path::new(item[0].as_str().unwrap_or_default()), cancel)?.1 != *item {
            return Err(invalid(message));
        }
    }
    Ok(())
}

fn update(db: &Database, id: &str, changes: &Value) -> PyResult<()> {
    let changes = changes
        .as_object()
        .filter(|v| !v.is_empty() && v.keys().all(|k| COLUMNS.contains(&k.as_str())))
        .ok_or_else(|| invalid("The metadata Undo record is invalid."))?;
    let assignments = changes
        .keys()
        .map(|v| format!("{v}=?"))
        .collect::<Vec<_>>()
        .join(",");
    let mut values = changes
        .values()
        .map(|v| match v {
            Value::String(v) => Ok(rusqlite::types::Value::Text(v.clone())),
            Value::Number(v) => v
                .as_i64()
                .map(rusqlite::types::Value::Integer)
                .ok_or_else(|| invalid("Invalid metadata integer.")),
            _ => Err(invalid("Invalid metadata value.")),
        })
        .collect::<PyResult<Vec<_>>>()?;
    values.push(rusqlite::types::Value::Text(id.into()));
    db.connection()?
        .execute(
            &format!("UPDATE mods SET {assignments} WHERE id=?"),
            params_from_iter(values),
        )
        .map_err(error)?;
    Ok(())
}

#[pyfunction]
fn source_metadata_apply(
    db: &Database,
    library: &Bound<'_, PyAny>,
    plan: &str,
    selected: Vec<String>,
    cancel: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = library.py();
    let plan = encoding::parse(plan)?;
    let selected: BTreeSet<_> = selected.into_iter().collect();
    let fields: BTreeMap<_, _> = plan["fields"]
        .as_array()
        .ok_or_else(|| invalid("Choose reviewed metadata fields for this library."))?
        .iter()
        .map(|v| (text(v, "name"), v))
        .collect();
    if plan["library_id"] != library.getattr("library_id")?.extract::<String>()?
        || selected.is_empty()
        || !selected
            .iter()
            .all(|v| fields.contains_key(v.as_str()) && FIELDS.contains(&v.as_str()))
    {
        return Err(invalid("Choose reviewed metadata fields for this library."));
    }
    if selected
        .iter()
        .any(|v| !text(fields[v.as_str()], "issue").is_empty())
    {
        return Err(invalid(
            "A selected source field is unavailable or invalid. Review its explanation.",
        ));
    }
    library.call_method0("_runtime_write_guard")?;
    inputs(
        py,
        &plan,
        cancel,
        "Source metadata changed after review. Review it again.",
    )?;
    let mut changes = json!({});
    for name in selected {
        let value = &fields[name.as_str()]["value"];
        match name.as_str() {
            "categories" => {
                changes["category"] = value
                    .as_array()
                    .and_then(|v| v.first())
                    .cloned()
                    .unwrap_or(json!(""));
                changes["categories_json"] = encoding::encode(value, true, true).into();
            }
            "nexus" => {
                for (index, key) in ["nexus_game", "nexus_mod_id", "nexus_file_id"]
                    .iter()
                    .enumerate()
                {
                    changes[*key] = value[index].clone();
                }
            }
            _ => changes[name] = value.clone(),
        }
    }
    db.connection()?
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(error)?;
    let profile = text(&plan, "profile_id");
    let id = text(&plan, "mod_id");
    let state = state(db, profile, id)?;
    if encoding::digest(&state, false, false) != text(&plan, "expected") {
        return Err(invalid(
            "Local metadata, memberships or category taxonomy changed. Review again.",
        ));
    }
    check_cancel(Some(cancel))?;
    let mut before = json!({});
    for key in changes.as_object().unwrap().keys() {
        before[key] = state["mod"][key].clone();
    }
    db.connection()?.execute_batch("CREATE TABLE IF NOT EXISTS metadata_refreshes(sequence INTEGER PRIMARY KEY AUTOINCREMENT,profile_id TEXT NOT NULL,mod_id TEXT NOT NULL,before_state TEXT NOT NULL,expected TEXT NOT NULL)").map_err(error)?;
    update(db, id, &changes)?;
    let expected = encoding::digest(&self::state(db, profile, id)?, false, false);
    db.connection()?.execute("INSERT INTO metadata_refreshes(profile_id,mod_id,before_state,expected) VALUES (?,?,?,?)",params![profile,id,before.to_string(),expected]).map_err(error)?;
    checkpoint.call0()?;
    check_cancel(Some(cancel))?;
    inputs(
        py,
        &plan,
        cancel,
        "Source metadata changed before commit. Review again.",
    )?;
    library.call_method0("_runtime_write_guard")?;
    Ok(())
}

fn history(db: &Database) -> PyResult<bool> {
    Ok(!records(
        &*db.connection()?,
        "SELECT 1 FROM sqlite_master WHERE name='metadata_refreshes' AND type='table'",
        [],
    )?
    .is_empty())
}
#[pyfunction]
fn source_metadata_undoable(db: &Database, profile: &str) -> PyResult<Vec<String>> {
    if !history(db)? {
        return Ok(Vec::new());
    }
    Ok(records(&*db.connection()?,"SELECT DISTINCT r.mod_id FROM metadata_refreshes r JOIN entries e ON r.mod_id=e.mod_id WHERE r.profile_id=? AND e.profile_id=?",params![profile,profile])?.iter().map(|v|text(v,"mod_id").into()).collect())
}
#[pyfunction]
fn source_metadata_undo(
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
    if !history(db)? {
        return Err(invalid("This reference has no metadata refresh to undo."));
    }
    let saved=records(&*db.connection()?,"SELECT * FROM metadata_refreshes WHERE profile_id=? AND mod_id=? ORDER BY sequence DESC LIMIT 1",params![profile,id])?.into_iter().next().ok_or_else(||invalid("The reference changed after refresh. Undo is blocked."))?;
    if encoding::digest(&state(db, profile, id)?, false, false) != text(&saved, "expected") {
        return Err(invalid(
            "The reference changed after refresh. Undo is blocked.",
        ));
    }
    let before = encoding::parse(text(&saved, "before_state"))?;
    check_cancel(Some(cancel))?;
    update(db, id, &before)?;
    db.connection()?
        .execute(
            "DELETE FROM metadata_refreshes WHERE sequence=?",
            [saved["sequence"].as_i64().unwrap_or_default()],
        )
        .map_err(error)?;
    checkpoint.call0()?;
    check_cancel(Some(cancel))?;
    library.call_method0("_runtime_write_guard")?;
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(source_metadata_review, m)?)?;
    m.add_function(wrap_pyfunction!(source_metadata_apply, m)?)?;
    m.add_function(wrap_pyfunction!(source_metadata_undoable, m)?)?;
    m.add_function(wrap_pyfunction!(source_metadata_undo, m)?)?;
    Ok(())
}
