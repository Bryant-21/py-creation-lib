use crate::{
    history_prune::py_path, invalid, plugin_inventory::object, reject_links, relative_path,
    saves::fold,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
};

fn game_ini(name: &str) -> Option<&'static str> {
    match fold(name).as_str() {
        "fallout4.ini" => Some("Fallout4.ini"),
        "fallout4prefs.ini" => Some("Fallout4Prefs.ini"),
        "fallout4custom.ini" => Some("Fallout4Custom.ini"),
        _ => None,
    }
}
#[pyfunction]
pub fn agent_parse_override(spec: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = spec.py();
    let text: String = spec.str()?.extract()?;
    let parts: Vec<_> = text.splitn(3, ':').collect();
    if parts.len() != 3 || !parts[2].contains('=') {
        return Err(invalid(format!(
            "INI override must be FILE:SECTION:KEY=VALUE: {}",
            spec.repr()?
        )));
    }
    let (key, val) = parts[2].split_once('=').unwrap();
    let (key, val, section) = (key.trim(), val.trim(), parts[1].trim());
    if section.is_empty()
        || key.is_empty()
        || section.contains(['[', ']'])
        || [section, key, val]
            .iter()
            .any(|s| s.contains(['\r', '\n', '\0']))
    {
        return Err(invalid(format!(
            "INI override needs a section and key without control characters: {}",
            spec.repr()?
        )));
    }
    if fold(key) == "slocalsavepath" || fold(key).starts_with("stestfile") {
        return Err(invalid(
            "Save isolation and plugin activation keys cannot be overridden at launch.",
        ));
    }
    let name = relative_path(parts[0].trim(), false)?;
    if !fold(&name).ends_with(".ini") {
        return Err(invalid(format!("Only INI files can be overridden: {name}")));
    }
    let known = game_ini(&name);
    Ok(object(py, &json!({"file":known.unwrap_or(&name), "section":section, "key":key, "value":val, "scope":if known.is_some() {"game"} else {"data"}}))?.unbind())
}
struct Section {
    folded: String,
    name: String,
    values: Vec<(String, String)>,
}
fn sections(value: &Bound<'_, PyAny>) -> PyResult<Vec<Section>> {
    let mut result = Vec::new();
    for pair in value.call_method0("items")?.try_iter()? {
        let pair = pair?;
        let val = pair.get_item(1)?;
        let mut values = Vec::new();
        for item in val.get_item(1)?.call_method0("items")?.try_iter()? {
            let item = item?;
            values.push((item.get_item(0)?.extract()?, item.get_item(1)?.extract()?));
        }
        result.push(Section {
            folded: pair.get_item(0)?.extract()?,
            name: val.get_item(0)?.extract()?,
            values,
        });
    }
    Ok(result)
}
fn finish(
    output: &mut String,
    done: &mut HashSet<(String, String)>,
    current: &str,
    sections: &[Section],
    newline: &str,
) {
    if let Some(section) = sections.iter().find(|s| s.folded == current) {
        for (key, value) in &section.values {
            if done.insert((current.into(), fold(key))) {
                output.push_str(&format!("{key}={value}{newline}"));
            }
        }
    }
}
#[pyfunction]
pub fn agent_set_ini_values(text: &str, value: &Bound<'_, PyAny>) -> PyResult<String> {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let sections = sections(value)?;
    let mut done = HashSet::new();
    let mut output = String::new();
    let mut current = String::new();
    let lines = text
        .into_pyobject(value.py())?
        .call_method1("splitlines", (true,))?;
    for line in lines.try_iter()? {
        let mut line: String = line?.extract()?;
        if !line.ends_with(['\n', '\r']) {
            line.push_str(newline);
        }
        let stripped = line.trim();
        if stripped.starts_with('[') && stripped.ends_with(']') {
            finish(&mut output, &mut done, &current, &sections, newline);
            current = fold(stripped[1..stripped.len() - 1].trim());
        } else if !stripped.starts_with([';', '#']) {
            if let Some((key, _)) = stripped.split_once('=') {
                if let Some(section) = sections.iter().find(|s| s.folded == current) {
                    let key = key.trim();
                    if let Some((_, val)) =
                        section.values.iter().find(|(k, _)| fold(k) == fold(key))
                    {
                        output.push_str(&format!("{key}={val}{newline}"));
                        done.insert((current.clone(), fold(key)));
                        continue;
                    }
                }
            }
        }
        output.push_str(&line);
    }
    finish(&mut output, &mut done, &current, &sections, newline);
    for section in &sections {
        if section
            .values
            .iter()
            .any(|(k, _)| !done.contains(&(section.folded.clone(), fold(k))))
        {
            output.push_str(&format!("[{}]{newline}", section.name));
            for (key, value) in &section.values {
                output.push_str(&format!("{key}={value}{newline}"));
            }
        }
    }
    Ok(output)
}
fn local_settings(files: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = files.py();
    let mut text = String::new();
    for pair in files.call_method0("items")?.try_iter()? {
        let pair = pair?;
        if fold(&pair.get_item(0)?.extract::<String>()?) == "settings.ini" {
            text = pair.get_item(1)?.extract()?;
            break;
        }
    }
    let config = py.import("configparser")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("interpolation", py.None())?;
    kwargs.set_item("strict", false)?;
    let parser = config.getattr("ConfigParser")?.call((), Some(&kwargs))?;
    if let Err(e) = parser.call_method1("read_string", (text,)) {
        if e.matches(py, &config.getattr("Error")?)? {
            return Err(invalid(format!(
                "Cannot read the MO2 profile settings.ini: {}",
                e.value(py)
            )));
        }
        return Err(e);
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item("fallback", "false")?;
    let setting: String = parser
        .call_method("get", ("General", "LocalSettings"), Some(&kwargs))?
        .extract()?;
    Ok(fold(setting.trim()) == "true")
}
fn read_text<'py>(py: Python<'py>, path: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let kwargs = PyDict::new(py);
    kwargs.set_item("limit", 2 * 1024 * 1024)?;
    py.import("creation_lib.mod_manager.mo2")?
        .getattr("read_text")?
        .call((path,), Some(&kwargs))
}
#[pyfunction]
pub fn agent_apply_launch_settings(
    library: &Bound<'_, PyAny>,
    plan: &Bound<'_, PyAny>,
    profile_files: &Bound<'_, PyAny>,
    specs: &Bound<'_, PyAny>,
    ini_source: &str,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    if !["auto", "profile", "global"].contains(&ini_source) {
        return Err(invalid("INI source must be auto, profile or global."));
    }
    let grouped = PyDict::new(py);
    for spec in specs.try_iter()? {
        let spec = spec?;
        let name: String = spec.get_item("file")?.extract()?;
        let key = fold(&name);
        if !grouped.contains(&key)? {
            grouped.set_item(&key, (&name, PyDict::new(py)))?;
        }
        let sections = grouped.get_item(&key)?.unwrap().get_item(1)?;
        let section: String = spec.get_item("section")?.extract()?;
        let section_key = fold(&section);
        if !sections.contains(&section_key)? {
            sections.set_item(&section_key, (&section, PyDict::new(py)))?;
        }
        sections
            .get_item(&section_key)?
            .get_item(1)?
            .set_item(spec.get_item("key")?, spec.get_item("value")?)?;
    }
    let use_global =
        ini_source == "global" || ini_source == "auto" && !local_settings(profile_files)?;
    let save_subpath: String = plan.getattr("save_subpath")?.extract()?;
    let namespace = save_subpath
        .strip_prefix("__ModBox21Saves_")
        .unwrap_or(&save_subpath);
    let namespace = if plan.getattr("isolate_documents")?.is_truthy()? {
        namespace
    } else {
        ""
    };
    let mut minimal = BTreeMap::new();
    for item in plan.getattr("minimal_ini")?.try_iter()? {
        let name: String = item?.extract()?;
        minimal.insert(fold(&name), name);
    }
    let snapshots = PyList::empty(py);
    let sources = PyDict::new(py);
    for snapshot in plan.getattr("snapshots")?.try_iter()? {
        let snapshot = snapshot?;
        let name: String = snapshot.get_item(0)?.extract()?;
        let mut text: String = snapshot.get_item(1)?.extract()?;
        let kind: String = snapshot.get_item(2)?.extract()?;
        let folded = fold(&name);
        if kind == "ini" {
            let mut source = if minimal.contains_key(&folded) {
                "minimal"
            } else {
                "profile"
            };
            let canonical = game_ini(&name).ok_or_else(|| invalid("Unknown game INI snapshot."))?;
            let physical = plan
                .getattr("ini_target")?
                .extract::<PathBuf>()?
                .join(canonical);
            reject_links(&physical)?;
            if use_global && physical.is_file() {
                text = py
                    .import("creation_lib.mod_manager.profile_state")?
                    .call_method1(
                        "fo4_runtime_ini",
                        (&name, read_text(py, &py_path(py, &physical)?)?, namespace),
                    )?
                    .extract()?;
                source = "global";
                minimal.remove(&folded);
            }
            if let Some(group) = grouped.get_item(&folded)? {
                text = agent_set_ini_values(&text, &group.get_item(1)?)?;
            }
            sources.set_item(canonical, source)?;
        }
        snapshots.append((name, text, kind))?;
    }
    let files = PyList::new(
        py,
        plan.getattr("files")?
            .try_iter()?
            .collect::<PyResult<Vec<_>>>()?,
    )?;
    let root: PathBuf = library.getattr("root")?.extract()?;
    let session: String = plan.getattr("session_id")?.extract()?;
    let profile_root = root
        .join("runtime")
        .join("sessions")
        .join(relative_path(&session, false)?);
    let skipped = PyList::empty(py);
    let mut skipped_names = HashSet::new();
    for (folded, group) in grouped.iter() {
        let folded: String = folded.extract()?;
        let name: String = group.get_item(0)?.extract()?;
        if game_ini(&folded).is_some() {
            continue;
        }
        let mut position = None;
        for (index, file) in files.iter().enumerate() {
            if fold(&file.get_item(0)?.extract::<String>()?) == folded {
                position = Some(index);
                break;
            }
        }
        let base = match position {
            Some(p) => files.get_item(p)?.get_item(1)?.extract::<PathBuf>()?,
            None => plan
                .getattr("game_root")?
                .extract::<PathBuf>()?
                .join("Data")
                .join(&name),
        };
        reject_links(&base)?;
        if !base.is_file() {
            skipped.append(object(py, &json!({"file":name, "reason":"not present in mods or game Data (paths are relative to Data)"}))?)?;
            skipped_names.insert(folded);
            continue;
        }
        let profile: String = plan.getattr("launch")?.getattr("profile_id")?.extract()?;
        let shadow = root
            .join("runtime")
            .join("profiles")
            .join(relative_path(&profile, false)?)
            .join("overwrite")
            .join(&name);
        reject_links(&shadow)?;
        if shadow.exists() {
            return Err(invalid(format!(
                "{name} exists in this profile's owned overwrite and would shadow the launch override. Move or edit it there first."
            )));
        }
        let target = profile_root.join("launch-overrides").join(&name);
        reject_links(&target)?;
        std::fs::create_dir_all(target.parent().unwrap())?;
        let temporary = target.with_file_name(format!(
            "{}.new",
            target.file_name().unwrap().to_string_lossy()
        ));
        reject_links(&temporary)?;
        let text: String = read_text(py, &py_path(py, &base)?)?.extract()?;
        std::fs::write(
            &temporary,
            agent_set_ini_values(&text, &group.get_item(1)?)?,
        )?;
        py_path(py, &temporary)?.call_method1("replace", (py_path(py, &target)?,))?;
        let entry_name = match position {
            Some(p) => files.get_item(p)?.get_item(0)?.extract::<String>()?,
            None => name,
        };
        let fingerprint = py
            .import("creation_lib.mod_manager.launcher")?
            .call_method1("fingerprint", (py_path(py, &target)?,))?;
        let entry = (
            entry_name,
            py_path(py, &target)?.str()?.extract::<String>()?,
            fingerprint,
        );
        match position {
            Some(p) => files.set_item(p, entry)?,
            None => files.append(entry)?,
        }
    }
    let applied = PyList::empty(py);
    for spec in specs.try_iter()? {
        let spec = spec?;
        if spec.get_item("scope")?.eq("game")?
            || !skipped_names.contains(&fold(&spec.get_item("file")?.extract::<String>()?))
        {
            applied.append(spec)?;
        }
    }
    let minimal_ini = PyList::empty(py);
    for name in plan.getattr("minimal_ini")?.try_iter()? {
        let name = name?;
        if minimal.contains_key(&fold(&name.extract::<String>()?)) {
            minimal_ini.append(name)?;
        }
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item("snapshots", snapshots.to_tuple())?;
    kwargs.set_item("files", files.to_tuple())?;
    kwargs.set_item("minimal_ini", minimal_ini.to_tuple())?;
    let updated = py
        .import("dataclasses")?
        .getattr("replace")?
        .call((plan,), Some(&kwargs))?;
    let report = PyDict::new(py);
    report.set_item("ini_sources", sources)?;
    report.set_item("overrides", applied)?;
    report.set_item("skipped_overrides", skipped)?;
    Ok(PyTuple::new(py, [updated, report.into_any()])?
        .into_any()
        .unbind())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(agent_parse_override, m)?)?;
    m.add_function(wrap_pyfunction!(agent_set_ini_values, m)?)?;
    m.add_function(wrap_pyfunction!(agent_apply_launch_settings, m)?)?;
    Ok(())
}
