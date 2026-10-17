use crate::{
    check_cancel,
    database::{Database, error, records},
    encoding,
    executables::text,
    invalid,
    nexus::nexus_game_domain,
    python_io::{with_context, with_database},
    reject_links, relative_path,
};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict},
};
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
};
use unicode_casefold::UnicodeCaseFold;

const PUBLIC: [&str; 4] = [
    "plugins.txt",
    "loadorder.txt",
    "lockedorder.txt",
    "archives.txt",
];
fn fold(value: &str) -> String {
    value.case_fold().collect()
}
fn names(game: &str) -> PyResult<BTreeMap<String, String>> {
    let game = crate::instances::games()
        .into_iter()
        .find(|g| nexus_game_domain(text(g, "name")) == game)
        .ok_or_else(|| invalid("Recipe game is unsupported or unknown."))?;
    Ok(game["ini_names"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let s = v.as_str().unwrap();
            (fold(s), s.into())
        })
        .collect())
}
fn identifier(value: &Value) -> PyResult<&str> {
    value
        .as_str()
        .filter(|v| {
            !v.is_empty()
                && v.len() <= 128
                && v.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        })
        .ok_or_else(|| invalid("Invalid recipe metadata identity."))
}
fn label<'a>(value: &'a Value, label: &str) -> PyResult<&'a str> {
    value
        .as_str()
        .filter(|v| !v.trim().is_empty() && v.chars().count() <= 200 && !v.chars().any(|c| c < ' '))
        .ok_or_else(|| invalid(format!("Invalid recipe {label}.")))
}
fn fields(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|v| v.len() == keys.len() && keys.iter().all(|k| v.contains_key(*k)))
}
fn to_value(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    encoding::parse(
        &value
            .py()
            .import("json")?
            .call_method1("dumps", (value,))?
            .extract::<String>()?,
    )
}
fn dataclass(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    to_value(
        &value
            .py()
            .import("dataclasses")?
            .call_method1("asdict", (value,))?,
    )
}

#[pyfunction]
fn unique_pairs(pairs: &Bound<'_, PyAny>) -> PyResult<Py<PyDict>> {
    let result = PyDict::new(pairs.py());
    for pair in pairs.try_iter()? {
        let pair = pair?;
        let key = pair.get_item(0)?;
        if result.contains(&key)? {
            return Err(invalid("Recipe JSON contains duplicate keys."));
        }
        result.set_item(key, pair.get_item(1)?)?;
    }
    Ok(result.unbind())
}

fn decode(py: Python<'_>, data: &[u8], path: &str, max_entries: usize) -> PyResult<Value> {
    let raw = PyBytes::new(py, data)
        .call_method1("decode", ("utf-8-sig",))
        .map_err(|e| invalid(format!("Invalid UTF-8 profile recipe: {e}")))?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("object_pairs_hook", wrap_pyfunction!(unique_pairs, py)?)?;
    let document = py
        .import("json")?
        .call_method("loads", (raw,), Some(&kwargs))
        .map_err(|e| invalid(format!("Invalid UTF-8 profile recipe: {e}")))?;
    let document =
        to_value(&document).map_err(|_| invalid("Recipe text contains invalid Unicode."))?;
    if !fields(
        &document,
        &[
            "format",
            "version",
            "library",
            "profile",
            "name",
            "game",
            "entries",
            "files",
            "minimal_inis",
        ],
    ) || document["format"] != "modbox21-profile-recipe"
        || document["version"].as_i64() != Some(1)
    {
        return Err(invalid("Unsupported profile recipe format or fields."));
    }
    let game = text(&document, "game");
    let names = names(game)?;
    let entries = document["entries"]
        .as_array()
        .filter(|v| v.len() <= max_entries)
        .ok_or_else(|| invalid("Recipe needs at most 100,000 ordered entries."))?;
    let mut seen = BTreeSet::new();
    for entry in entries {
        if !fields(
            entry,
            &["id", "name", "kind", "enabled", "group", "version"],
        ) {
            return Err(invalid("Invalid recipe mod entry fields."));
        }
        let id = identifier(&entry["id"])?;
        if !seen.insert(id)
            || !matches!(text(entry, "kind"), "external" | "owned" | "separator")
            || !entry["enabled"].is_boolean()
        {
            return Err(invalid(
                "Duplicate identity or invalid recipe mod kind/activation.",
            ));
        }
        if entry["kind"] == "separator" && entry["enabled"] == true {
            return Err(invalid("Recipe separators cannot be enabled payloads."));
        }
        label(&entry["name"], "mod name")?;
        let group = entry["group"]
            .as_str()
            .filter(|v| v.chars().count() <= 500)
            .ok_or_else(|| invalid("Invalid recipe organization group."))?;
        relative_path(group, true)?;
        if !entry["version"]
            .as_str()
            .is_some_and(|v| v.chars().count() <= 500 && !v.contains('\0'))
        {
            return Err(invalid("Invalid recipe version metadata."));
        }
    }
    let files = document["files"]
        .as_object()
        .filter(|v| v.len() <= PUBLIC.len() + names.len())
        .ok_or_else(|| invalid("Invalid recipe configuration inventory."))?;
    let mut normalized = BTreeMap::new();
    for (name, value) in files {
        let key = fold(name);
        if (!PUBLIC.contains(&key.as_str()) && !names.contains_key(&key))
            || normalized.contains_key(&key)
            || !value
                .as_str()
                .is_some_and(|v| !v.contains('\0') && v.len() <= 2 * 1024 * 1024)
        {
            return Err(invalid(
                "Recipe configuration needs allowlisted root-level names and bounded text.",
            ));
        }
        if names.contains_key(&key) {
            crate::profile_state::profile_validate_ini(py, name, value.as_str().unwrap())?;
        }
        normalized.insert(key.clone(), json!([names.get(&key).unwrap_or(&key), value]));
    }
    let minimal = document["minimal_inis"]
        .as_array()
        .filter(|v| {
            v.len() <= names.len()
                && v.iter()
                    .all(|v| v.as_str().is_some_and(|s| names.contains_key(&fold(s))))
        })
        .ok_or_else(|| invalid("Invalid recipe minimal-INI declarations."))?;
    let minimal: BTreeSet<_> = minimal
        .iter()
        .map(|v| names[&fold(v.as_str().unwrap())].clone())
        .collect();
    Ok(
        json!({"path":path,"checksum":format!("{:x}",Sha256::digest(data)),"library":identifier(&document["library"] )?,"profile":identifier(&document["profile"] )?,"name":label(&document["name"],"profile name")?,"game":game,"entries":entries,"files":normalized.into_values().collect::<Vec<_>>(),"minimal_inis":minimal}),
    )
}

#[pyfunction]
fn recipe_decode(py: Python<'_>, data: &[u8], path: &str, max_entries: usize) -> PyResult<String> {
    Ok(decode(py, data, path, max_entries)?.to_string())
}

fn read(
    py: Python<'_>,
    path: &Path,
    max_bytes: usize,
    max_entries: usize,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<Value> {
    let path =
        crate::executables::launch_local_path(&path.to_string_lossy(), "Profile recipe", false)?;
    let object = py.import("pathlib")?.getattr("Path")?.call1((&path,))?;
    let before = object.call_method0("stat")?;
    if before.getattr("st_size")?.extract::<u64>()? > max_bytes as u64 {
        return Err(invalid("Profile recipe exceeds the 64 MiB metadata limit."));
    }
    check_cancel(cancel)?;
    let data = with_context(&object.call_method1("open", ("rb",))?, |stream| {
        Ok(stream
            .call_method1("read", (max_bytes.saturating_add(1),))?
            .unbind())
    })?;
    let data = data.bind(py).cast::<PyBytes>()?.as_bytes();
    reject_links(&path)?;
    let after = object.call_method0("stat")?;
    let mut changed = data.len() > max_bytes;
    for key in ["st_dev", "st_ino", "st_size", "st_mtime_ns"] {
        changed |= !before.getattr(key)?.eq(after.getattr(key)?)?;
    }
    if changed {
        return Err(invalid(
            "Profile recipe changed while reading or exceeds its limit.",
        ));
    }
    check_cancel(cancel)?;
    decode(py, data, &path.to_string_lossy(), max_entries)
}

#[pyfunction]
#[pyo3(signature=(path,max_bytes,max_entries,cancel=None))]
fn recipe_read(
    py: Python<'_>,
    path: PathBuf,
    max_bytes: usize,
    max_entries: usize,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    Ok(read(py, &path, max_bytes, max_entries, cancel)?.to_string())
}

#[pyfunction]
#[pyo3(signature=(library,profile,token,max_bytes,max_entries,effective_ini,checkpoint,cancel=None,progress=None))]
fn recipe_export(
    library: &Bound<'_, PyAny>,
    profile: &str,
    token: &str,
    max_bytes: usize,
    max_entries: usize,
    effective_ini: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
    cancel: Option<&Bound<'_, PyAny>>,
    progress: Option<&Bound<'_, PyAny>>,
) -> PyResult<PathBuf> {
    let py = library.py();
    let snapshot = library.call_method1("profile_snapshot", (profile,))?;
    let profile_object = snapshot.get_item(0)?;
    let profile = dataclass(&profile_object)?;
    let game = nexus_game_domain(text(&profile["files"], "_modbox_game"));
    let names = names(&game)?;
    let rows = snapshot.get_item(1)?;
    if rows.len()? > max_entries {
        return Err(invalid("Profile recipe exceeds the 100,000-entry limit."));
    }
    let mut files = BTreeMap::new();
    for (name, value) in profile["files"].as_object().unwrap() {
        check_cancel(cancel)?;
        let key = fold(name);
        if !PUBLIC.contains(&key.as_str()) && !names.contains_key(&key) {
            continue;
        }
        let value = if names.contains_key(&key) {
            json!(
                effective_ini
                    .call1((library, &profile_object, name))?
                    .extract::<String>()?
            )
        } else {
            value.clone()
        };
        let canonical = names.get(&key).unwrap_or(&key).clone();
        if files.insert(canonical, value).is_some() {
            return Err(invalid(
                "Profile configuration names are ambiguous by case.",
            ));
        }
    }
    let mut entries = vec![];
    for row in rows.try_iter()? {
        let row = dataclass(&row?)?;
        entries.push(json!({"id":row["id"],"name":row["name"],"kind":row["kind"],"enabled":row["kind"]!="separator" && row["enabled"]==true,"group":row["group"],"version":row["version"]}));
    }
    let library_id: String = library.getattr("library_id")?.extract()?;
    let document = json!({"format":"modbox21-profile-recipe","version":1,"library":library_id,"profile":profile["id"],"name":profile["name"],"game":game,"entries":entries,"files":files,"minimal_inis":encoding::parse(profile["files"]["_modbox_minimal_inis"].as_str().unwrap_or("[]"))?});
    let data = encoding::encode(&document, false, false);
    if data.len() > max_bytes {
        return Err(invalid("Profile recipe exceeds the 64 MiB metadata limit."));
    }
    decode(py, data.as_bytes(), "", max_entries)?;
    let root: PathBuf = library.getattr("root")?.extract()?;
    crate::transactions::transaction_key(token)?;
    let stage = root.join(".staging").join(format!("recipe-{token}"));
    let destination = root
        .join("exports")
        .join(format!("{token}.modbox-profile.json"));
    reject_links(&stage)?;
    reject_links(&destination)?;
    let result = (|| {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stage)?
            .write_all(data.as_bytes())?;
        checkpoint.call1(("export-staged",))?;
        if let Some(progress) = progress {
            progress.call1((1, "Profile recipe ready; no mod payloads copied"))?;
        }
        check_cancel(cancel)?;
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn MoveFileW(source: *const u16, destination: *const u16) -> i32;
            }
            let source: Vec<u16> = stage.as_os_str().encode_wide().chain(Some(0)).collect();
            let target: Vec<u16> = destination
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            if unsafe { MoveFileW(source.as_ptr(), target.as_ptr()) } == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        #[cfg(not(windows))]
        {
            std::fs::hard_link(&stage, &destination)?;
            std::fs::remove_file(&stage)?;
        }
        Ok(destination)
    })();
    reject_links(&stage)?;
    if stage.exists() {
        std::fs::remove_file(stage)?;
    }
    result
}

fn context(
    db: &Database,
    template: &str,
    recipe: &Value,
    mapping: &Value,
    max_entries: usize,
) -> PyResult<(Value, String, String, bool)> {
    let current = crate::profile_backup::snapshot(db, template)?;
    if nexus_game_domain(text(&current["files"], "_modbox_game")) != text(recipe, "game")
        || text(&current["files"], "_modbox_instance_id").is_empty()
    {
        return Err(invalid(
            "Choose a configured local template profile for the same game.",
        ));
    }
    let instance = records(
        &*db.connection()?,
        "SELECT configuration FROM instances WHERE id=?",
        [text(&current["files"], "_modbox_instance_id")],
    )?;
    let instance = instance
        .first()
        .ok_or_else(|| invalid("Template game instance metadata is unavailable."))?;
    let payloads: BTreeSet<_> = recipe["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] != "separator")
        .map(|e| text(e, "id"))
        .collect();
    let mapping = mapping
        .as_object()
        .filter(|v| v.values().all(Value::is_string))
        .ok_or_else(|| invalid("Recipe mappings must contain local metadata identities."))?;
    if mapping.keys().map(String::as_str).collect::<BTreeSet<_>>() != payloads
        || mapping
            .values()
            .map(|v| v.as_str().unwrap())
            .collect::<BTreeSet<_>>()
            .len()
            != mapping.len()
    {
        return Err(invalid(
            "Map every payload entry to one distinct existing local mod. Names are never guessed.",
        ));
    }
    let available = records(
        &*db.connection()?,
        "SELECT id,name,source,kind,version,nexus_game FROM mods LIMIT ?",
        [(max_entries + 1) as i64],
    )?;
    if available.len() > max_entries {
        return Err(invalid(
            "Local recipe mapping inventory exceeds the 100,000-mod limit.",
        ));
    }
    let mut mapped = vec![];
    for (source, target) in mapping {
        let row = available
            .iter()
            .find(|v| v["id"] == *target)
            .filter(|v| {
                matches!(text(v, "kind"), "external" | "owned")
                    && (text(v, "nexus_game").is_empty()
                        || nexus_game_domain(text(v, "nexus_game")) == text(recipe, "game"))
            })
            .ok_or_else(|| {
                invalid("A mapped local mod is missing, unsupported or belongs to another game.")
            })?;
        mapped.push(json!([source, row]));
    }
    mapped.sort_by(|a, b| a[0].as_str().cmp(&b[0].as_str()));
    let expected = encoding::digest(
        &json!({"template":current,"instance":instance["configuration"],"mapped":mapped}),
        false,
        false,
    );
    let origin = format!("{}:{}", text(recipe, "library"), text(recipe, "profile"));
    let target = format!(
        "{:x}",
        Sha256::digest(
            format!(
                "{origin}:{}",
                text(&current["files"], "_modbox_instance_id")
            )
            .as_bytes()
        )
    )[..32]
        .to_string();
    if !records(
        &*db.connection()?,
        "SELECT 1 FROM deleted_profiles WHERE id=?",
        [&target],
    )?
    .is_empty()
    {
        return Err(invalid(
            "This transferred profile was removed. Restore it from Removed profiles; import will not resurrect it.",
        ));
    }
    let existing = records(
        &*db.connection()?,
        "SELECT files FROM profiles WHERE id=?",
        [&target],
    )?;
    if let Some(row) = existing.first() {
        if encoding::parse(text(row, "files"))?["_modbox_recipe_origin"] != origin {
            return Err(invalid(
                "Recipe destination identity is occupied by another local profile.",
            ));
        }
    }
    Ok((current, expected, target, !existing.is_empty()))
}

#[pyfunction]
#[pyo3(signature=(library,action,template,recipe,mapping,name,max_bytes,max_entries,checkpoint,active_sessions,cancel=None))]
fn recipe_operation(
    library: &Bound<'_, PyAny>,
    action: &str,
    template: &str,
    recipe: &str,
    mapping: &str,
    name: &str,
    max_bytes: usize,
    max_entries: usize,
    checkpoint: &Bound<'_, PyAny>,
    active_sessions: &Bound<'_, PyAny>,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let py = library.py();
    let recipe = encoding::parse(recipe)?;
    let mapping = encoding::parse(mapping)?;
    let factory = library.getattr("_db")?;
    if action == "candidates" {
        return with_database(&factory, |_, db| {
            let rows = records(
                &*db.connection()?,
                "SELECT id,name,source,kind,nexus_game FROM mods WHERE kind IN ('owned','external') ORDER BY name,id LIMIT ?",
                [(max_entries + 1) as i64],
            )?;
            if rows.len() > max_entries {
                return Err(invalid(
                    "Local recipe mapping inventory exceeds the 100,000-mod limit.",
                ));
            }
            Ok(json!(
                rows.into_iter()
                    .filter(|v| text(v, "nexus_game").is_empty()
                        || nexus_game_domain(text(v, "nexus_game")) == name)
                    .map(|mut v| {
                        v.as_object_mut().unwrap().remove("nexus_game");
                        v
                    })
                    .collect::<Vec<_>>()
            )
            .to_string())
        });
    }
    if action == "suggestions" {
        let snapshot = library.call_method1("profile_snapshot", (template,))?;
        let profile = dataclass(&snapshot.get_item(0)?)?;
        let available: BTreeSet<String> = snapshot
            .get_item(1)?
            .try_iter()?
            .map(|row| row?.getattr("id")?.extract())
            .collect::<PyResult<_>>()?;
        let library_id: String = library.getattr("library_id")?.extract()?;
        let mut known = BTreeMap::new();
        for entry in recipe["entries"].as_array().unwrap() {
            let id = text(entry, "id");
            if recipe["library"] == library_id && available.contains(id) {
                known.insert(id.to_string(), json!(id));
            }
        }
        if profile["files"]["_modbox_recipe_origin"]
            == format!("{}:{}", text(&recipe, "library"), text(&recipe, "profile"))
        {
            let saved = encoding::parse(
                profile["files"]["_modbox_recipe_mapping"]
                    .as_str()
                    .unwrap_or("{}"),
            )?;
            if let Some(saved) = saved.as_object() {
                for (source, target) in saved {
                    if target.as_str().is_some_and(|v| available.contains(v)) {
                        known.insert(source.clone(), target.clone());
                    }
                }
            }
        }
        let result: BTreeMap<_, _> = recipe["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] != "separator")
            .filter_map(|e| {
                let id = text(e, "id");
                known.get(id).map(|v| (id, v))
            })
            .collect();
        return Ok(json!(result).to_string());
    }
    if action == "review" {
        if read(
            py,
            Path::new(text(&recipe, "path")),
            max_bytes,
            max_entries,
            cancel,
        )? != recipe
        {
            return Err(invalid("Recipe changed after preview. Read it again."));
        }
        let name = json!(name);
        let name = label(&name, "new profile name")?.trim();
        return with_database(&factory, |_, db| {
            db.connection()?.execute_batch("BEGIN").map_err(error)?;
            let (_, expected, target, existing) =
                context(db, template, &recipe, &mapping, max_entries)?;
            check_cancel(cancel)?;
            let mut mapping: Vec<_> = mapping
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| json!([k, v]))
                .collect();
            mapping.sort_by(|a, b| a[0].as_str().cmp(&b[0].as_str()));
            Ok(json!({"recipe":recipe,"template":template,"name":name,"mapping":mapping,"expected":expected,"target":target,"existing":existing}).to_string())
        });
    }
    let plan = mapping;
    let guard = || -> PyResult<()> {
        if active_sessions
            .call1((library.getattr("root")?,))?
            .is_truthy()?
        {
            return Err(invalid(
                "Close the recorded runtime before importing a profile recipe.",
            ));
        }
        Ok(())
    };
    guard()?;
    if read(
        py,
        Path::new(text(&recipe, "path")),
        max_bytes,
        max_entries,
        cancel,
    )? != recipe
    {
        return Err(invalid(
            "Recipe changed after import review. Review it again.",
        ));
    }
    with_context(&library.getattr("owned")?.call_method0("lock")?, |_| {
        with_database(&factory, |_, db| {
            guard()?;
            db.connection()?
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(error)?;
            let mapping: serde_json::Map<String, Value> = plan["mapping"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| (v[0].as_str().unwrap().into(), v[1].clone()))
                .collect();
            let mapping = Value::Object(mapping);
            let (current, expected, target, existing) =
                context(db, template, &recipe, &mapping, max_entries)?;
            if expected != text(&plan, "expected")
                || target != text(&plan, "target")
                || json!(existing) != plan["existing"]
            {
                return Err(invalid(
                    "Template, mod mapping or destination changed after review. Review the transfer again.",
                ));
            }
            if read(
                py,
                Path::new(text(&recipe, "path")),
                max_bytes,
                max_entries,
                cancel,
            )? != recipe
            {
                return Err(invalid("Recipe changed during import. Review it again."));
            }
            check_cancel(cancel)?;
            if existing {
                return Ok(json!([target, false]).to_string());
            }
            let mut files = json!({"_modbox_instance_id":current["files"]["_modbox_instance_id"],"_modbox_game":current["files"]["_modbox_game"]});
            let mut present = BTreeSet::new();
            for file in recipe["files"].as_array().unwrap() {
                let name = file[0].as_str().unwrap();
                files[name] = file[1].clone();
                present.insert(fold(name));
            }
            let mut minimal: BTreeSet<String> = recipe["minimal_inis"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().into())
                .collect();
            for (key, name) in names(text(&recipe, "game"))? {
                if !present.contains(&key) {
                    minimal.insert(name);
                }
            }
            files["_modbox_minimal_inis"] = json!(encoding::encode(&json!(minimal), true, true));
            files["_modbox_recipe_origin"] = json!(format!(
                "{}:{}",
                text(&recipe, "library"),
                text(&recipe, "profile")
            ));
            files["_modbox_recipe_mapping"] = json!(encoding::encode(&mapping, true, true));
            db.connection()?
                .execute(
                    "INSERT INTO profiles VALUES (?,?,'',?)",
                    params![
                        target,
                        text(&plan, "name"),
                        encoding::encode(&files, true, true)
                    ],
                )
                .map_err(error)?;
            for (index, entry) in recipe["entries"].as_array().unwrap().iter().enumerate() {
                check_cancel(cancel)?;
                let mut id = text(&mapping, text(entry, "id")).to_string();
                if entry["kind"] == "separator" {
                    id = format!(
                        "{:x}",
                        Sha256::digest(
                            format!("{target}:separator:{}", text(entry, "id")).as_bytes()
                        )
                    )[..32]
                        .into();
                    db.connection()?
                        .execute(
                            "INSERT INTO mods(id,name,source,kind) VALUES (?,?,'','separator')",
                            params![id, text(entry, "name")],
                        )
                        .map_err(error)?;
                }
                db.connection()?
                    .execute(
                        "INSERT INTO entries VALUES (?,?,?,?,?)",
                        params![
                            target,
                            id,
                            index as i64,
                            entry["enabled"].as_bool().unwrap(),
                            text(entry, "group")
                        ],
                    )
                    .map_err(error)?;
            }
            checkpoint.call1(("import-inserted",))?;
            check_cancel(cancel)?;
            Ok(json!([target, true]).to_string())
        })
    })
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(recipe_decode, m)?)?;
    m.add_function(wrap_pyfunction!(recipe_read, m)?)?;
    m.add_function(wrap_pyfunction!(recipe_export, m)?)?;
    m.add_function(wrap_pyfunction!(recipe_operation, m)?)?;
    Ok(())
}
