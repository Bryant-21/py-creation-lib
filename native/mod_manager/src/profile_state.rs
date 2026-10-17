use crate::{
    check_cancel,
    database::{Database, error, records},
    encoding,
    executables::text,
    invalid, relative_path,
};
use pyo3::{prelude::*, types::PyDict};
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::PathBuf};
use unicode_casefold::UnicodeCaseFold;

fn fold(value: &str) -> String {
    value.case_fold().collect()
}

fn parse_ini<'py>(
    py: Python<'py>,
    name: &str,
    text: &str,
    strict: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let kwargs = PyDict::new(py);
    kwargs.set_item("interpolation", py.None())?;
    kwargs.set_item("strict", strict)?;
    let parser = py
        .import("configparser")?
        .getattr("ConfigParser")?
        .call((), Some(&kwargs))?;
    if !strict {
        parser.setattr("optionxform", py.get_type::<pyo3::types::PyString>())?;
    }
    parser.call_method1("read_string", (text,)).map_err(|e| {
        invalid(if strict {
            format!("Invalid profile INI: {e}")
        } else {
            format!("Cannot prepare profile INI {name}: {e}")
        })
    })?;
    Ok(parser)
}

fn check_test_files(parser: &Bound<'_, PyAny>, sections: &[String], message: &str) -> PyResult<()> {
    for section in sections {
        let values = parser.get_item(section)?;
        for key in values.try_iter()? {
            let key = key?.extract::<String>()?;
            if fold(&key).starts_with("stestfile")
                && !values
                    .get_item(&key)?
                    .extract::<String>()?
                    .trim()
                    .is_empty()
            {
                return Err(invalid(message));
            }
        }
    }
    Ok(())
}

#[pyfunction]
pub fn profile_validate_ini(py: Python<'_>, name: &str, text: &str) -> PyResult<String> {
    let name = relative_path(name, false)?;
    if name.contains('/')
        || !fold(&name).ends_with(".ini")
        || text.len() > 2 * 1024 * 1024
        || text.contains('\0')
    {
        return Err(invalid(
            "Choose a root-level profile INI within the 2 MiB text limit.",
        ));
    }
    let parser = parse_ini(py, &name, text, true)?;
    let sections: Vec<String> = parser.call_method0("sections")?.extract()?;
    if sections
        .iter()
        .map(|s| fold(s))
        .collect::<BTreeSet<_>>()
        .len()
        != sections.len()
    {
        return Err(invalid("INI section names must be unique ignoring case."));
    }
    check_test_files(
        &parser,
        &sections,
        "sTestFile overrides bypass reviewed plugin activation and cannot be saved.",
    )?;
    Ok(name)
}

#[pyfunction]
fn profile_runtime_ini(py: Python<'_>, name: &str, text: &str, profile: &str) -> PyResult<String> {
    let parser = parse_ini(py, name, text, false)?;
    let sections: Vec<String> = parser.call_method0("sections")?.extract()?;
    let general: Vec<_> = sections.iter().filter(|s| fold(s) == "general").collect();
    if general.len() > 1 {
        return Err(invalid(format!(
            "Ambiguous General sections in {name}; correct the profile INI before launching."
        )));
    }
    check_test_files(
        &parser,
        &sections,
        &format!(
            "{name} contains sTestFile overrides that bypass normal plugin activation. Correct the profile INI before launching."
        ),
    )?;
    if let Some(section) = general.first() {
        if section.as_str() != "General" {
            let values = py
                .get_type::<PyDict>()
                .call1((parser.get_item(*section)?,))?;
            parser.call_method1("remove_section", (*section,))?;
            parser.set_item("General", values)?;
        }
    }
    if !parser
        .call_method1("has_section", ("General",))?
        .extract::<bool>()?
    {
        parser.call_method1("add_section", ("General",))?;
    }
    let keys: Vec<String> = parser
        .get_item("General")?
        .try_iter()?
        .map(|v| v?.extract())
        .collect::<PyResult<_>>()?;
    let mut save_path = "Saves\\".to_owned();
    for key in keys {
        if fold(&key) == "slocalsavepath" {
            let existing: String = parser.call_method1("get", ("General", &key))?.extract()?;
            if !existing.trim().is_empty() && !fold(&existing).starts_with("__modbox21saves_") {
                save_path = existing;
            }
            parser.call_method1("remove_option", ("General", key))?;
        }
    }
    parser.call_method1(
        "set",
        (
            "General",
            "SLocalSavePath",
            if profile.is_empty() {
                save_path
            } else {
                format!("__ModBox21Saves_{}\\", relative_path(profile, false)?)
            },
        ),
    )?;
    let stream = py.import("io")?.getattr("StringIO")?.call0()?;
    parser.call_method1("write", (&stream,))?;
    stream.call_method0("getvalue")?.extract()
}

#[pyfunction]
fn profile_effective_ini(
    py: Python<'_>,
    root: PathBuf,
    profile: &str,
    target: &str,
    name: &str,
    files: &Bound<'_, PyAny>,
    reader: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let name = relative_path(name, false)?;
    if name.contains('/') {
        return Err(invalid("Profile INIs must be root-level files."));
    }
    let original = files.get_item(&name)?.extract::<String>()?;
    if !["fallout4.ini", "fallout4prefs.ini", "fallout4custom.ini"].contains(&fold(&name).as_str())
    {
        return Ok(original.into());
    }
    let root = root
        .join("runtime/profiles")
        .join(crate::transactions::transaction_key(profile)?);
    let baseline = root.join("ini_baseline.json");
    let path = root.join("ini").join(&name);
    crate::reject_links(&baseline)?;
    crate::reject_links(&path)?;
    if !baseline.is_file() || !path.is_file() {
        return Ok(original.into());
    }
    if std::fs::metadata(&baseline)?.len() > 65536 {
        return Err(invalid("Runtime INI baseline exceeds its safety limit."));
    }
    let recorded: Value = serde_json::from_slice(&std::fs::read(baseline)?)
        .map_err(|_| invalid("Runtime INI baseline is invalid; review local runtime files."))?;
    if !recorded.is_object() {
        return Err(invalid(
            "Runtime INI baseline must be a name-to-digest record.",
        ));
    }
    let namespace = if target.is_empty() {
        profile.into()
    } else {
        format!("{profile}_{target}")
    };
    let digest = format!(
        "{:x}",
        Sha256::digest(profile_runtime_ini(py, &name, &original, &namespace)?.as_bytes())
    );
    if recorded[fold(&name)] == digest {
        reader.call1((path, 2 * 1024 * 1024))?.extract()
    } else {
        Ok(original.into())
    }
}

#[pyfunction]
#[pyo3(signature=(db, wrapper, library, action, profile, name, content, expected="null", cancel=None))]
fn profile_edit(
    db: &Database,
    wrapper: &Bound<'_, PyAny>,
    library: &Bound<'_, PyAny>,
    action: &str,
    profile: &str,
    name: &str,
    content: &str,
    expected: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let py = library.py();
    if action == "rename" {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 200 || name.chars().any(|c| c < ' ') {
            return Err(invalid(
                "Give the profile a short name without control characters.",
            ));
        }
        check_cancel(cancel)?;
        db.connection()?
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(error)?;
        if db
            .connection()?
            .execute(
                "UPDATE profiles SET name=? WHERE id=?",
                params![name, profile],
            )
            .map_err(error)?
            == 0
        {
            return Err(invalid("The local profile no longer exists."));
        }
        return check_cancel(cancel);
    }
    let mut name = if action == "save" {
        profile_validate_ini(py, name, content)?
    } else {
        name.into()
    };
    db.connection()?
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(error)?;
    let rows = records(
        &*db.connection()?,
        "SELECT files FROM profiles WHERE id=?",
        [profile],
    )?;
    let row = rows
        .first()
        .ok_or_else(|| invalid("The local profile no longer exists."))?;
    let mut files = encoding::parse(text(row, "files"))?;
    let json = py.import("json")?;
    if action == "restore" && library.hasattr("profile_files")? {
        let result = library.call_method1(
            "profile_files",
            (
                wrapper,
                profile,
                json.call_method1("loads", (files.to_string(),))?,
            ),
        )?;
        files = encoding::parse(&json.call_method1("dumps", (result,))?.extract::<String>()?)?;
    }
    name = files
        .as_object()
        .unwrap()
        .keys()
        .find(|key| fold(key) == fold(&name))
        .cloned()
        .unwrap_or(name);
    let mut backups = encoding::parse(files["_modbox_ini_backup"].as_str().unwrap_or("{}"))?;
    if action == "restore" && backups.get(&name).is_none() {
        return Err(invalid("This INI has no previous local snapshot."));
    }
    if files.get(&name).unwrap_or(&Value::Null) != &encoding::parse(expected)? {
        return Err(invalid(if action == "restore" {
            "Profile INI changed after restore was requested. Reload it first."
        } else {
            "Profile INI changed after editing began. Reload it before saving."
        }));
    }
    let minimal = encoding::parse(files["_modbox_minimal_inis"].as_str().unwrap_or("[]"))?;
    let mut backup_minimal =
        encoding::parse(files["_modbox_ini_backup_minimal"].as_str().unwrap_or("{}"))?;
    let previous_minimal = backup_minimal[&name].as_bool().unwrap_or(false);
    let minimal = minimal
        .as_array()
        .ok_or_else(|| invalid("Invalid minimal INI metadata."))?;
    backup_minimal[&name] = json!(
        minimal
            .iter()
            .any(|v| fold(v.as_str().unwrap_or("")) == fold(&name))
    );
    let mut minimal: Vec<_> = minimal
        .iter()
        .filter(|v| fold(v.as_str().unwrap_or("")) != fold(&name))
        .cloned()
        .collect();
    let next = if action == "restore" {
        let next = backups[&name].as_str().unwrap_or("");
        profile_validate_ini(py, &name, next)?;
        if previous_minimal {
            minimal.push(json!(name));
        }
        next.to_string()
    } else {
        content.into()
    };
    backups[&name] = files.get(&name).cloned().unwrap_or(json!(""));
    files[&name] = json!(next);
    files["_modbox_ini_backup"] = json!(encoding::encode(&backups, true, true));
    files["_modbox_ini_backup_minimal"] = json!(encoding::encode(&backup_minimal, true, true));
    files["_modbox_minimal_inis"] = json!(encoding::encode(&json!(minimal), true, true));
    check_cancel(cancel)?;
    if action == "restore" && library.hasattr("write_profile_fields")? {
        library.call_method1(
            "write_profile_fields",
            (
                wrapper,
                profile,
                json.call_method1("loads", (files.to_string(),))?,
            ),
        )?;
    } else {
        db.connection()?
            .execute(
                "UPDATE profiles SET files=? WHERE id=?",
                params![encoding::encode(&files, true, true), profile],
            )
            .map_err(error)?;
    }
    check_cancel(cancel)
}

#[pyfunction]
pub fn profile_isolate_documents(
    db: &Database,
    profile: &str,
    enabled: Option<bool>,
) -> PyResult<bool> {
    let connection = db.connection()?;
    if enabled.is_some() {
        connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    }
    let rows = records(
        &connection,
        "SELECT files FROM profiles WHERE id=?",
        [profile],
    )?;
    let row = rows
        .first()
        .ok_or_else(|| invalid("The local profile no longer exists."))?;
    let mut files = encoding::parse(text(row, "files"))?;
    if let Some(enabled) = enabled {
        files["_modbox_isolate_documents"] = json!(if enabled { "true" } else { "false" });
        connection
            .execute(
                "UPDATE profiles SET files=? WHERE id=?",
                params![encoding::encode(&files, true, true), profile],
            )
            .map_err(error)?;
    }
    Ok(files["_modbox_isolate_documents"].as_str() == Some("true"))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(profile_isolate_documents, m)?)?;
    m.add_function(wrap_pyfunction!(profile_validate_ini, m)?)?;
    m.add_function(wrap_pyfunction!(profile_runtime_ini, m)?)?;
    m.add_function(wrap_pyfunction!(profile_effective_ini, m)?)?;
    m.add_function(wrap_pyfunction!(profile_edit, m)?)?;
    Ok(())
}
