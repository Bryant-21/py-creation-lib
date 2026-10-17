use crate::{
    check_cancel,
    database::{Database, error, records},
    encoding,
    executables::text,
    invalid,
    profile_state::profile_validate_ini,
    target_settings::{options, preferences, targets},
};
use pyo3::prelude::*;
use rusqlite::params;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use unicode_casefold::UnicodeCaseFold;

pub const INIS: &[&str] = &["Fallout4.ini", "Fallout4Prefs.ini", "Fallout4Custom.ini"];
const FIELDS: &[&str] = &[
    "Fallout4.ini",
    "Fallout4Prefs.ini",
    "Fallout4Custom.ini",
    "plugins.txt",
    "loadorder.txt",
    "lockedorder.txt",
    "archives.txt",
    "_modbox_plugin_validation",
    "_modbox_plugin_backup",
    "_modbox_minimal_inis",
    "_modbox_ini_backup",
    "_modbox_ini_backup_minimal",
];
fn fold(value: &str) -> String {
    value.case_fold().collect()
}
fn target(db: &Database, id: &str) -> PyResult<Value> {
    targets(db)?
        .into_iter()
        .find(|v| v["id"] == id)
        .ok_or_else(|| invalid("This launch target no longer exists."))
}
fn overlay(db: &Database, id: &str, profile: &str) -> PyResult<Value> {
    let row = records(
        &*db.connection()?,
        "SELECT files FROM launch_target_profiles WHERE target_id=? AND profile_id=?",
        params![id, profile],
    )?;
    row.first()
        .map(|v| encoding::parse(text(v, "files")))
        .unwrap_or_else(|| Ok(json!({})))
}
pub fn files(
    py: Python<'_>,
    db: &Database,
    id: &str,
    profile: &str,
    base: &Value,
) -> PyResult<Value> {
    let game = fold(text(base, "_modbox_game"));
    if !matches!(game.as_str(), "" | "fo4" | "fallout 4" | "fallout4") {
        return Err(invalid(
            "FO4 launch targets require a Fallout 4 logical profile; other game profiles retain their original executables.",
        ));
    }
    let target = target(db, id)?;
    let overlay = overlay(db, id, profile)?;
    if !overlay
        .as_object()
        .is_some_and(|v| v.keys().all(|k| FIELDS.contains(&k.as_str())))
    {
        return Err(invalid(
            "Target profile settings contain unsupported fields.",
        ));
    }
    let mut files = base
        .as_object()
        .ok_or_else(|| invalid("Invalid profile file metadata."))?
        .clone();
    for key in [
        "_modbox_plugin_validation",
        "_modbox_plugin_backup",
        "_modbox_game_content",
        "_modbox_ini_backup",
        "_modbox_ini_backup_minimal",
    ] {
        files.remove(key);
    }
    files.retain(|k, _| !INIS.iter().any(|name| fold(k) == fold(name)));
    files.extend(target["defaults"].as_object().unwrap().clone());
    files.extend(overlay.as_object().unwrap().clone());
    let scopes = preferences(db, id)?;
    if scopes["ini"] == "shared" {
        let mut found = BTreeSet::new();
        for name in INIS {
            if let Some(value) = base
                .as_object()
                .unwrap()
                .iter()
                .find(|(key, _)| fold(key) == fold(name))
                .map(|(_, v)| v)
                .filter(|v| !v.is_null())
            {
                profile_validate_ini(
                    py,
                    name,
                    value
                        .as_str()
                        .ok_or_else(|| invalid("Invalid profile INI content."))?,
                )?;
                files.insert((*name).into(), value.clone());
                found.insert(*name);
            }
        }
        let minimal = encoding::parse(
            target["defaults"]["_modbox_minimal_inis"]
                .as_str()
                .unwrap_or("[]"),
        )?;
        let minimal: Vec<_> = minimal
            .as_array()
            .ok_or_else(|| invalid("Invalid minimal INI metadata."))?
            .iter()
            .filter(|v| !found.contains(v.as_str().unwrap_or_default()))
            .cloned()
            .collect();
        files.insert(
            "_modbox_minimal_inis".into(),
            encoding::encode(&json!(minimal), true, true).into(),
        );
    }
    files.insert("_modbox_game".into(), "Fallout 4".into());
    files.insert("_modbox_target_id".into(), id.into());
    files.insert(
        "_modbox_target_signature".into(),
        encoding::digest(&json!([target, overlay]), false, false).into(),
    );
    files.insert(
        "_modbox_target_options".into(),
        encoding::encode(&options(py, db, id)?, true, true).into(),
    );
    files.insert(
        "_modbox_target_preferences".into(),
        encoding::encode(&scopes, true, true).into(),
    );
    Ok(Value::Object(files))
}

#[pyfunction]
fn target_profile_files(
    py: Python<'_>,
    db: &Database,
    id: &str,
    profile: &str,
    base: &str,
) -> PyResult<String> {
    Ok(files(py, db, id, profile, &encoding::parse(base)?)?.to_string())
}

pub fn write_files(
    py: Python<'_>,
    db: &Database,
    id: &str,
    profile: &str,
    value: &Value,
) -> PyResult<()> {
    target(db, id)?;
    let mut stored = value
        .as_object()
        .ok_or_else(|| invalid("Invalid target profile metadata."))?
        .clone();
    stored.retain(|name, _| FIELDS.contains(&name.as_str()));
    if preferences(db, id)?["ini"] == "shared" {
        let row = records(
            &*db.connection()?,
            "SELECT files FROM profiles WHERE id=?",
            [profile],
        )?
        .into_iter()
        .next()
        .ok_or_else(|| invalid("This local profile no longer exists."))?;
        let current = files(py, db, id, profile, &encoding::parse(text(&row, "files"))?)?;
        if INIS.iter().any(|name| value[*name] != current[*name]) {
            return Err(invalid(
                "Shared manager INIs are read-only in target view. Select Original executables to edit/restore them, or select target-local INIs first.",
            ));
        }
        let previous = overlay(db, id, profile)?;
        for name in INIS.iter().copied().chain([
            "_modbox_ini_backup",
            "_modbox_ini_backup_minimal",
            "_modbox_minimal_inis",
        ]) {
            stored.remove(name);
            if let Some(value) = previous.get(name) {
                stored.insert(name.into(), value.clone());
            }
        }
    }
    db.connection()?.execute("INSERT INTO launch_target_profiles VALUES (?,?,?) ON CONFLICT(target_id,profile_id) DO UPDATE SET files=excluded.files",params![id,profile,Value::Object(stored).to_string()]).map_err(error)?;
    Ok(())
}

#[pyfunction]
fn target_write_profile(
    py: Python<'_>,
    db: &Database,
    id: &str,
    profile: &str,
    fields: &str,
) -> PyResult<()> {
    write_files(py, db, id, profile, &encoding::parse(fields)?)
}

#[pyfunction]
fn target_save_ini(
    db: &Database,
    library: &Bound<'_, PyAny>,
    id: &str,
    profile: &str,
    name: &str,
    content: &str,
    expected: &str,
    cancel: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let py = library.py();
    if preferences(db, id)?["ini"] == "shared" {
        return Err(invalid(
            "Shared manager INIs are read-only in target view. Select Original executables to edit the logical profile INI, or choose target-local INIs.",
        ));
    }
    let name = profile_validate_ini(py, name, content)?;
    let name = INIS
        .iter()
        .copied()
        .find(|v| fold(v) == fold(&name))
        .ok_or_else(|| invalid("Choose a supported target-local FO4 INI."))?;
    library.call_method0("_runtime_write_guard")?;
    db.connection()?
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(error)?;
    let row = records(
        &*db.connection()?,
        "SELECT files FROM profiles WHERE id=?",
        [profile],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| invalid("This local profile no longer exists."))?;
    let mut fields = files(py, db, id, profile, &encoding::parse(text(&row, "files"))?)?;
    if fields[name] != encoding::parse(expected)? {
        return Err(invalid(
            "Target profile configuration changed after review.",
        ));
    }
    let mut backups = encoding::parse(fields["_modbox_ini_backup"].as_str().unwrap_or("{}"))?;
    backups[name] = fields.get(name).cloned().unwrap_or(json!(""));
    fields["_modbox_ini_backup"] = encoding::encode(&backups, true, true).into();
    fields[name] = content.into();
    let minimal = encoding::parse(fields["_modbox_minimal_inis"].as_str().unwrap_or("[]"))?;
    let minimal = minimal
        .as_array()
        .ok_or_else(|| invalid("Invalid minimal INI metadata."))?;
    let mut backup_minimal = encoding::parse(
        fields["_modbox_ini_backup_minimal"]
            .as_str()
            .unwrap_or("{}"),
    )?;
    backup_minimal[name] = minimal.iter().any(|v| v == name).into();
    fields["_modbox_ini_backup_minimal"] = encoding::encode(&backup_minimal, true, true).into();
    let minimal: Vec<_> = minimal
        .iter()
        .filter(|v| fold(v.as_str().unwrap_or_default()) != fold(name))
        .cloned()
        .collect();
    fields["_modbox_minimal_inis"] = encoding::encode(&json!(minimal), true, true).into();
    check_cancel(Some(cancel))?;
    write_files(py, db, id, profile, &fields)?;
    checkpoint.call0()?;
    check_cancel(Some(cancel))?;
    library.call_method0("_runtime_write_guard")?;
    Ok(content.into())
}

#[pyfunction]
fn target_profiles(py: Python<'_>, db: &Database, id: &str) -> PyResult<String> {
    db.connection()?.execute_batch("BEGIN").map_err(error)?;
    let profiles = records(
        &*db.connection()?,
        "SELECT * FROM profiles ORDER BY name COLLATE NOCASE,id",
        [],
    )?;
    let mut result = Vec::new();
    for mut row in profiles {
        let base = encoding::parse(text(&row, "files"))?;
        if matches!(
            fold(text(&base, "_modbox_game")).as_str(),
            "" | "fo4" | "fallout 4" | "fallout4"
        ) {
            row["files"] = files(py, db, id, text(&row, "id"), &base)?;
            result.push(row);
        }
    }
    Ok(json!(result).to_string())
}

#[pyfunction]
fn target_save_namespace(db: &Database, id: &str, profile: &str) -> PyResult<String> {
    let profile = crate::transactions::transaction_key(profile)?;
    Ok(if preferences(db, id)?["saves"] == "shared" {
        profile
    } else {
        format!("{profile}_{id}")
    })
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(target_profile_fields, m)?)?;
    m.add_function(wrap_pyfunction!(target_profile_snapshot, m)?)?;
    m.add_function(wrap_pyfunction!(target_profile_signature, m)?)?;
    m.add_function(wrap_pyfunction!(target_state_root, m)?)?;
    m.add_function(wrap_pyfunction!(target_save_root, m)?)?;
    m.add_function(wrap_pyfunction!(target_record, m)?)?;
    m.add_function(wrap_pyfunction!(target_profile_files, m)?)?;
    m.add_function(wrap_pyfunction!(target_write_profile, m)?)?;
    m.add_function(wrap_pyfunction!(target_save_ini, m)?)?;
    m.add_function(wrap_pyfunction!(target_profiles, m)?)?;
    m.add_function(wrap_pyfunction!(target_save_namespace, m)?)?;
    Ok(())
}

#[pyfunction]
fn target_profile_fields() -> Vec<String> {
    FIELDS.iter().map(|v| (*v).into()).collect()
}

fn configuration(
    db: &Database,
    files: &Value,
    target: &Value,
    variants: &Value,
) -> PyResult<Value> {
    let row = records(
        &*db.connection()?,
        "SELECT configuration FROM instances WHERE id=?",
        [text(files, "_modbox_instance_id")],
    )?;
    let mut value = row
        .first()
        .map(|v| encoding::parse(text(v, "configuration")))
        .unwrap_or_else(|| Ok(json!({})))?;
    if value.get("game").is_none() {
        value["game"] = json!({});
    }
    value["game"]["id"] = "fo4".into();
    value["game"]["gamepath"] = target["game_root"].clone();
    value["target"] = target.clone();
    value["target_sources"] = variants.clone();
    Ok(value)
}

#[pyfunction]
fn target_profile_signature(
    py: Python<'_>,
    db: &Database,
    id: &str,
    profile: &str,
) -> PyResult<String> {
    let row = records(
        &*db.connection()?,
        "SELECT files FROM profiles WHERE id=?",
        [profile],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| invalid("Select a local profile first."))?;
    let files = files(py, db, id, profile, &encoding::parse(text(&row, "files"))?)?;
    let target = target(db, id)?;
    let variants = crate::target_sources::sources(db, id)?;
    let records = records(
        &*db.connection()?,
        "SELECT m.id,m.source,e.priority,e.enabled,m.kind FROM mods m JOIN entries e ON e.mod_id=m.id WHERE e.profile_id=? AND m.kind!='foreign' ORDER BY e.priority,m.id",
        [profile],
    )?;
    let mut rows = Vec::new();
    for row in records {
        let variant = &variants[text(&row, "id")];
        let source = text(variant, "source");
        let enabled = variant["enabled"]
            .as_bool()
            .map(|v| json!(if v { 1 } else { 0 }))
            .unwrap_or_else(|| row["enabled"].clone());
        rows.push(json!([
            row["id"],
            if source.is_empty() {
                &row["source"]
            } else {
                &variant["source"]
            },
            row["priority"],
            enabled,
            if source.is_empty() {
                text(&row, "kind")
            } else {
                "reference"
            }
        ]));
    }
    if target["f4se"]["managed"].as_bool().unwrap_or_default()
        && !text(&target["f4se"], "data").is_empty()
    {
        rows.insert(
            0,
            json!([
                format!("f4se-target-{id}"),
                target["f4se"]["data"],
                -1,
                1,
                "reference"
            ]),
        );
    }
    let config = configuration(db, &files, &target, &variants)?;
    Ok(encoding::digest(
        &json!([
            files,
            rows,
            config,
            crate::library::overlays(&*db.connection()?)?
        ]),
        true,
        true,
    ))
}

#[pyfunction]
fn target_profile_snapshot(
    py: Python<'_>,
    db: &Database,
    id: &str,
    profile: &str,
    include_signature: bool,
) -> PyResult<String> {
    db.connection()?.execute_batch("BEGIN").map_err(error)?;
    let mut record = records(
        &*db.connection()?,
        "SELECT * FROM profiles WHERE id=?",
        [profile],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| invalid("Select an existing local profile first."))?;
    let base = encoding::parse(text(&record, "files"))?;
    let target = target(db, id)?;
    let variants = crate::target_sources::sources(db, id)?;
    record["files"] = files(py, db, id, profile, &base)?;
    let raw = crate::library::raw_mod_rows(&*db.connection()?, profile)?;
    let mut rows = Vec::new();
    for row in raw {
        rows.push(encoding::parse(
            &crate::target_sources::target_source_effective(
                &row.to_string(),
                &variants[text(&row, "id")].to_string(),
            )?,
        )?);
    }
    let enabled = crate::library::effective(
        &rows,
        &crate::library::overlays(&*db.connection()?)?,
        profile,
        id,
    );
    for (row, value) in rows.iter_mut().zip(enabled) {
        row["enabled"] = value.into();
    }
    if target["f4se"]["managed"].as_bool().unwrap_or_default()
        && !text(&target["f4se"], "data").is_empty()
    {
        rows.insert(0,json!({"id":format!("f4se-target-{id}"),"name":"F4SE target scripts","source":target["f4se"]["data"],"kind":"reference","version":text(&target["f4se"],"version"),"category":"","notes":"Target runtime baseline","tags":"","priority":-1,"enabled":true,"group":"","nexus_game":"","nexus_mod_id":0,"nexus_file_id":0,"categories":[]}));
    }
    let config = configuration(db, &base, &target, &variants)?;
    let mut result = vec![record, json!(rows), config];
    if include_signature {
        result.push(target_profile_signature(py, db, id, profile)?.into());
    }
    Ok(json!(result).to_string())
}

#[pyfunction]
fn target_record(db: &Database, id: &str) -> PyResult<String> {
    Ok(target(db, id)?.to_string())
}

#[pyfunction]
fn target_save_root(
    db: &Database,
    id: &str,
    profile: &str,
    base: std::path::PathBuf,
    root: std::path::PathBuf,
) -> PyResult<std::path::PathBuf> {
    let profile = crate::transactions::transaction_key(profile)?;
    let owner = if preferences(db, id)?["saves"] == "shared" {
        base
    } else {
        root
    };
    Ok(owner.join("runtime/profiles").join(profile).join("saves"))
}

#[pyfunction]
fn target_state_root(db: &Database, library: &Bound<'_, PyAny>, id: &str) -> PyResult<String> {
    use std::io::Write;
    let id = crate::transactions::transaction_key(id)?;
    target(db, &id)?;
    let library_id = crate::executables::identity(&format!(
        "{}/target/{id}",
        library.getattr("library_id")?.extract::<String>()?
    ));
    let root = library
        .getattr("root")?
        .extract::<std::path::PathBuf>()?
        .join(".launch-targets")
        .join(&id);
    crate::reject_links(&root)?;
    let marker = root.join(library.getattr("MARKER")?.extract::<String>()?);
    crate::reject_links(&marker)?;
    if root.exists() && !marker.is_file() && std::fs::read_dir(&root)?.next().is_some() {
        return Err(invalid(
            "Launch target state folder has unowned content; it cannot be adopted.",
        ));
    }
    std::fs::create_dir_all(&root)?;
    let expected = json!({"format":1,"id":library_id});
    if marker.exists() {
        if encoding::parse(&std::fs::read_to_string(&marker)?)? != expected {
            return Err(invalid("Launch target state ownership marker differs."));
        }
    } else {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)?
            .write_all(expected.to_string().as_bytes())?;
    }
    for name in [".staging", "exports"] {
        let path = root.join(name);
        crate::reject_links(&path)?;
        std::fs::create_dir_all(path)?;
    }
    Ok(json!({"root":root.to_string_lossy(),"library_id":library_id,"target_id":id}).to_string())
}
