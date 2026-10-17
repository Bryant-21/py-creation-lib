use crate::{
    check_cancel, encoding,
    history_prune::{py_path, stamp},
    invalid,
    plugin_inventory::{object, value},
    python_io::with_context,
    reject_links, relative_path,
    saves::fold,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
pub fn source_stamp(py: Python<'_>, path: &Path) -> PyResult<(u64, i64)> {
    reject_links(path)?;
    if !path.is_file() {
        return Err(invalid(
            "Archive provider is missing or is not an ordinary file.",
        ));
    }
    let info = stamp(&py_path(py, path)?.call_method0("stat")?)?;
    Ok((info.2, info.3))
}
#[pyfunction]
fn archive_source_stamp(py: Python<'_>, path: PathBuf) -> PyResult<(u64, i64)> {
    source_stamp(py, &path)
}
pub fn plugin_signature(catalog: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = catalog.py();
    let mut result = Vec::new();
    for plugin in catalog.getattr("plugins")?.try_iter()? {
        let p = plugin?;
        result.push(
            (
                fold(&p.getattr("name")?.extract::<String>()?),
                p.getattr("path")?,
                p.getattr("enabled")?,
                p.getattr("priority")?,
                p.getattr("forced")?,
                p.getattr("fingerprint")?,
            )
                .into_pyobject(py)?,
        );
    }
    Ok(PyTuple::new(py, result)?.into_any().unbind())
}
#[pyfunction]
fn archive_plugin_signature(catalog: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    plugin_signature(catalog)
}
fn validated(py: Python<'_>, data: &Value) -> PyResult<Value> {
    let files=data["files"].as_array().filter(|v|v.len()<=250_000 && data["file_count"].as_u64()==Some(v.len() as u64)).ok_or_else(||invalid("Unsupported or oversized archive metadata; expected at most 250,000 named members."))?;
    let mut seen = BTreeSet::new();
    let mut names = Vec::new();
    for name in files {
        let name = name
            .as_str()
            .ok_or_else(|| invalid("Archive member names must be strings."))?;
        let safe = relative_path(name, false)?;
        if !seen.insert(fold(&safe)) {
            return Err(invalid(
                "Archive has ambiguous duplicate Windows member paths.",
            ));
        }
        names.push(safe);
    }
    names.sort_by_key(|v| fold(v));
    let format = if let Some(v) = data.get("format") {
        object(py, v)?.str()?.to_string()
    } else {
        "Unknown".into()
    };
    Ok(json!({"format":format,"file_count":names.len(),"files":names}))
}
#[pyfunction]
fn archive_validate(py: Python<'_>, data: &str) -> PyResult<String> {
    Ok(validated(py, &encoding::parse(data)?)?.to_string())
}
#[pyfunction]
fn archive_cached(
    library: &Bound<'_, PyAny>,
    reader: &Bound<'_, PyAny>,
    path: PathBuf,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let py = library.py();
    let before = source_stamp(py, &path)?;
    let canonical = fold(
        &py_path(py, &path)?
            .call_method0("resolve")?
            .str()?
            .to_string(),
    );
    let record = with_context(&library.call_method0("_db")?, |db| {
        Ok(db
            .call_method1(
                "execute",
                (
                    "SELECT metadata FROM archive_metadata WHERE path=? AND size=? AND modified=?",
                    (&canonical, before.0, before.1),
                ),
            )?
            .call_method0("fetchone")?
            .unbind())
    })?;
    check_cancel(Some(cancel))?;
    if !record.is_none(py) {
        if let Ok(data) = encoding::parse(&record.bind(py).get_item(0)?.extract::<String>()?)
            .and_then(|v| validated(py, &v))
        {
            if source_stamp(py, &path)? != before {
                return Err(invalid(
                    "Archive changed while reading its cached inventory.",
                ));
            }
            return Ok(data.to_string());
        }
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item("cancel", cancel)?;
    kwargs.set_item("max_bytes", 32 * 1024 * 1024)?;
    kwargs.set_item("timeout", 60)?;
    let data = validated(
        py,
        &value(&reader.call(
            (("archive", "list", path.to_string_lossy().as_ref()),),
            Some(&kwargs),
        )?)?,
    )?;
    check_cancel(Some(cancel))?;
    if source_stamp(py, &path)? != before {
        return Err(invalid(
            "Archive changed during inspection; no inventory was cached.",
        ));
    }
    let encoded = encoding::encode(&data, true, true);
    if encoded.len() > 32 * 1024 * 1024 {
        return Err(invalid("Archive metadata exceeds its 32 MiB cache limit."));
    }
    with_context(&library.call_method0("_db")?, |db| {
        check_cancel(Some(cancel))?;
        db.call_method1("execute",("INSERT INTO archive_metadata VALUES (?,?,?,?) ON CONFLICT(path) DO UPDATE SET size=excluded.size,modified=excluded.modified,metadata=excluded.metadata",(canonical,before.0,before.1,encoded)))?;
        check_cancel(Some(cancel))
    })?;
    Ok(data.to_string())
}
pub fn ini_settings(py: Python<'_>, files: &Value) -> PyResult<BTreeMap<String, String>> {
    let mut settings = BTreeMap::new();
    let config = py.import("configparser")?;
    let error = config.getattr("Error")?;
    for filename in ["fallout4.ini", "fallout4custom.ini"] {
        let text = crate::plugin_state::snapshot(files, filename);
        let kwargs = PyDict::new(py);
        kwargs.set_item("interpolation", py.None())?;
        kwargs.set_item("strict", false)?;
        let parser = config.getattr("ConfigParser")?.call((), Some(&kwargs))?;
        if let Err(e) = parser.call_method1(
            "read_file",
            (py.import("io")?.getattr("StringIO")?.call1((text,))?,),
        ) {
            if e.is_instance(py, &error) {
                return Err(invalid(format!(
                    "Cannot inspect archive activation from {filename}: {}",
                    e.value(py).str()?
                )));
            }
            return Err(e);
        }
        for section in parser.call_method0("sections")?.try_iter()? {
            let section = section?.extract::<String>()?;
            if fold(&section) == "archive" {
                for item in parser.call_method1("items", (section,))?.try_iter()? {
                    let item = item?;
                    settings.insert(
                        fold(&item.get_item(0)?.extract::<String>()?),
                        item.get_item(1)?.extract::<String>()?.trim().into(),
                    );
                }
                break;
            }
        }
    }
    Ok(settings)
}
#[pyfunction]
fn archive_ini_settings(py: Python<'_>, files: &str) -> PyResult<BTreeMap<String, String>> {
    ini_settings(py, &encoding::parse(files)?)
}
fn ini_archives(py: Python<'_>, files: &Value) -> PyResult<Vec<String>> {
    let settings = ini_settings(py, files)?;
    let mut names = Vec::new();
    for key in ["sresourcearchivelist", "sresourcearchivelist2"] {
        for name in settings
            .get(key)
            .map(String::as_str)
            .unwrap_or("")
            .split(',')
        {
            if !name.trim().is_empty() {
                names.push(relative_path(name.trim(), false)?);
            }
        }
    }
    Ok(names)
}
#[pyfunction]
fn archive_ini_archives(py: Python<'_>, files: &str) -> PyResult<Vec<String>> {
    ini_archives(py, &encoding::parse(files)?)
}
#[pyfunction]
fn archive_member_candidates(member: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let mut result = Vec::new();
    for archive in member.getattr("archives")?.try_iter()? {
        let archive = archive?;
        if archive.getattr("selected")?.is_truthy()?
            && ["listed", "plugin candidate"]
                .contains(&archive.getattr("activation")?.extract::<String>()?.as_str())
        {
            result.push(archive);
        }
    }
    Ok(PyTuple::new(member.py(), result)?.into_any().unbind())
}
pub fn profile_signature(snapshot: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = snapshot.py();
    Ok((
        crate::conflicts::conflict_rows_signature(&snapshot.get_item(1)?)?,
        encoding::encode(
            &value(&snapshot.get_item(0)?.getattr("files")?)?,
            true,
            true,
        ),
        encoding::encode(&value(&snapshot.get_item(2)?)?, true, true),
    )
        .into_pyobject(py)?
        .into_any()
        .unbind())
}
pub fn effective_ini(
    library: &Bound<'_, PyAny>,
    profile: &Bound<'_, PyAny>,
    names: Option<&[String]>,
) -> PyResult<Value> {
    let py = library.py();
    let keys = if let Some(names) = names {
        names.to_vec()
    } else {
        profile
            .getattr("files")?
            .try_iter()?
            .map(|n| n?.extract::<String>())
            .collect::<PyResult<Vec<_>>>()?
            .into_iter()
            .filter(|n| ["fallout4.ini", "fallout4custom.ini"].contains(&fold(n).as_str()))
            .collect()
    };
    let mut files = json!({});
    for name in keys {
        files[&name] = value(
            &py.import("creation_lib.mod_manager.profile_state")?
                .call_method1("effective_ini", (library, profile, &name))?,
        )?;
    }
    Ok(files)
}
fn container(name: &str) -> bool {
    let name = fold(name);
    name.ends_with(".ba2") || name.ends_with(".bsa")
}
fn shallow<'py>(
    py: Python<'py>,
    root: &Path,
    id: &str,
    label: &str,
    priority: i64,
    cancel: &Bound<'py, PyAny>,
    message: &str,
) -> PyResult<Vec<(String, Bound<'py, PyAny>)>> {
    let class = py
        .import("creation_lib.mod_manager.conflicts")?
        .getattr("Provider")?;
    let mut result = Vec::new();
    for (count, entry) in std::fs::read_dir(root)?.enumerate() {
        check_cancel(Some(cancel))?;
        if count >= 100_000 {
            return Err(invalid(message));
        }
        let path = entry?.path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if container(&name) {
            let stamp = source_stamp(py, &path)?;
            result.push((
                name,
                class.call1((
                    id,
                    label,
                    priority,
                    true,
                    false,
                    path.to_string_lossy().as_ref(),
                    stamp.0,
                    stamp.1,
                ))?,
            ));
        }
    }
    Ok(result)
}
#[pyfunction]
fn archive_build(
    library: &Bound<'_, PyAny>,
    profile_id: &str,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
    reader: &Bound<'_, PyAny>,
    loose: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let api = py.import("creation_lib.mod_manager.archive_index")?;
    let snapshot = library.call_method1("profile_snapshot", (profile_id,))?;
    let profile = snapshot.get_item(0)?;
    let rows = snapshot.get_item(1)?;
    let config = value(&snapshot.get_item(2)?)?;
    let signature = profile_signature(&snapshot)?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("directory", true)?;
    let root = py
        .import("creation_lib.mod_manager.launcher")?
        .getattr("local_path")?
        .call(
            (
                config["game"]["gamepath"].as_str().unwrap_or(""),
                "Game installation",
            ),
            Some(&kwargs),
        )?
        .extract::<PathBuf>()?;
    let data = root.join("Data");
    reject_links(&data)?;
    if !data.is_dir() {
        return Err(invalid(
            "The configured game installation has no Data directory.",
        ));
    }
    let catalog = py
        .import("creation_lib.mod_manager.plugin_state")?
        .call_method1("inventory", (library, profile_id, cancel))?;
    let mut containers = shallow(
        py,
        &data,
        "game",
        "Game Data",
        -1,
        cancel,
        "Game Data root exceeds the shallow inventory limit.",
    )?;
    let loose = if loose.is_truthy()? {
        loose.clone()
    } else {
        let kwargs = PyDict::new(py);
        kwargs.set_item("include_context", false)?;
        kwargs.set_item("build_tree", false)?;
        py.import("creation_lib.mod_manager.conflicts")?
            .getattr("build_index")?
            .call((library, profile_id, cancel, progress), Some(&kwargs))?
    };
    for entry in loose.getattr("files")?.try_iter()? {
        let entry = entry?;
        let name = entry.getattr("path")?.extract::<String>()?;
        if !name.contains('/') && container(&name) {
            for provider in entry.getattr("providers")?.try_iter()? {
                let provider = provider?;
                if !["game", "overwrite"]
                    .contains(&provider.getattr("mod_id")?.extract::<String>()?.as_str())
                {
                    containers.push((name.clone(), provider));
                }
            }
        }
    }
    let overwrite = library
        .getattr("root")?
        .extract::<PathBuf>()?
        .join("runtime")
        .join("profiles")
        .join(profile_id)
        .join("overwrite");
    reject_links(&overwrite)?;
    if overwrite.is_dir() {
        containers.extend(shallow(
            py,
            &overwrite,
            "overwrite",
            "Owned overwrite",
            rows.len()? as i64 + 1,
            cancel,
            "Owned overwrite root exceeds its inventory limit.",
        )?);
    }
    let mut ordered = Vec::new();
    for (name, provider) in containers {
        ordered.push((
            name,
            provider.getattr("priority")?.extract::<i64>()?,
            provider.getattr("mod_id")?.extract::<String>()?,
            provider,
        ));
    }
    ordered.sort_by(|a, b| (a.1, &a.2).cmp(&(b.1, &b.2)));
    let mut identities = BTreeSet::new();
    let mut selected = BTreeMap::new();
    for (name, _, id, provider) in &ordered {
        if !identities.insert((id.clone(), fold(name))) {
            return Err(invalid(
                "Archive container names are ambiguous ignoring case within a provider.",
            ));
        }
        if provider.getattr("enabled")?.is_truthy()? && !provider.getattr("hidden")?.is_truthy()? {
            selected.insert(fold(name), provider.clone());
        }
    }
    let effective = effective_ini(library, &profile, None)?;
    let game = catalog.getattr("game")?.extract::<String>()?;
    let listed = if game == "fo4" {
        ini_archives(py, &effective)?
    } else {
        Vec::new()
    };
    let ranks = listed
        .iter()
        .enumerate()
        .map(|(i, n)| (fold(n), i as i64))
        .collect::<BTreeMap<_, _>>();
    let mut active = Vec::new();
    for plugin in catalog.getattr("plugins")?.try_iter()? {
        let plugin = plugin?;
        if plugin.getattr("enabled")?.is_truthy()? {
            let name = plugin.getattr("name")?.extract::<String>()?;
            active.push((
                fold(
                    &Path::new(&name)
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy(),
                ),
                plugin.getattr("priority")?.extract::<i64>()?,
            ));
        }
    }
    let cache = api.getattr("CachedArchives")?.call1((library, reader))?;
    let mut archives = Vec::new();
    let mut members = BTreeMap::<String, (String, Vec<Bound<'_, PyAny>>)>::new();
    let mut issues = loose.getattr("issues")?.extract::<Vec<String>>()?;
    issues.push("Member order is a profile load-plan preview. Actual engine attachment/precedence and non-FO4 adapters remain unverified.".into());
    ordered.sort_by(|a, b| (fold(&a.0), a.1, &a.2).cmp(&(fold(&b.0), b.1, &b.2)));
    for (index, (name, _, _, provider)) in ordered.iter().enumerate() {
        check_cancel(Some(cancel))?;
        if !progress.is_none() {
            progress.call1((
                index as f64 / ordered.len().max(1) as f64,
                format!("Inspecting archive {name}"),
            ))?;
        }
        let chosen = selected
            .get(&fold(name))
            .map(|v| v.eq(provider))
            .transpose()?
            .unwrap_or(false);
        let (mut activation, mut reason, mut rank) = (
            "inactive",
            "Mod disabled, hidden, or container overridden",
            -1i64,
        );
        if chosen {
            if game != "fo4" {
                activation = "unknown";
                reason = "This game's archive activation adapter has not been validated";
            } else if let Some(position) = ranks.get(&fold(name)) {
                activation = "listed";
                reason = "Listed by local profile Archive INI settings";
                rank = *position;
            } else {
                let stem = fold(
                    &Path::new(name)
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy(),
                );
                if let Some((_, order)) = active.iter().rev().find(|(plugin, _)| {
                    stem == *plugin || stem.starts_with(&format!("{plugin} - "))
                }) {
                    activation = "plugin candidate";
                    reason = "Name matches active plugin; engine attachment needs validation";
                    rank = listed.len() as i64 + order;
                } else {
                    activation = "unlisted";
                    reason = "Not listed by this profile and no matching active plugin";
                }
            }
        }
        let result = if chosen {
            let result = value(&cache.call1((provider.getattr("path")?, cancel))?)?;
            if source_stamp(py, &provider.getattr("path")?.extract::<PathBuf>()?)?
                != (
                    provider.getattr("size")?.extract()?,
                    provider.getattr("modified")?.extract()?,
                )
            {
                return Err(invalid(
                    "Archive source changed after its container inventory. Refresh archives.",
                ));
            }
            result
        } else {
            json!({"files":[],"format":""})
        };
        let names = result["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        let archive = api.getattr("Archive")?.call1((
            name,
            provider,
            chosen,
            activation,
            reason,
            rank,
            PyTuple::new(py, &names)?,
            result["format"].as_str().unwrap(),
        ))?;
        archives.push(archive.clone());
        for name in names {
            members
                .entry(fold(&name))
                .or_insert((name, Vec::new()))
                .1
                .push(archive.clone());
            if members.len() > 1_000_000 {
                return Err(invalid(
                    "Combined archive inventory exceeds 1,000,000 member paths.",
                ));
            }
        }
    }
    check_cancel(Some(cancel))?;
    let current = library.call_method1("profile_snapshot", (profile_id,))?;
    if !signature
        .bind(py)
        .eq(profile_signature(&current)?.bind(py))?
    {
        return Err(invalid(
            "Profile changed while inspecting archives; refresh its inventory.",
        ));
    }
    for archive in &archives {
        check_cancel(Some(cancel))?;
        if archive.getattr("selected")?.is_truthy()? {
            let p = archive.getattr("provider")?;
            if source_stamp(py, &p.getattr("path")?.extract::<PathBuf>()?)?
                != (
                    p.getattr("size")?.extract()?,
                    p.getattr("modified")?.extract()?,
                )
            {
                return Err(invalid(
                    "Archive source changed during member inspection. Refresh archives.",
                ));
            }
        }
    }
    let names = effective
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    if effective != effective_ini(library, &current.get_item(0)?, Some(&names))? {
        return Err(invalid(
            "Runtime INI settings changed during archive inspection. Refresh archives.",
        ));
    }
    let mut members = members.into_values().collect::<Vec<_>>();
    members.sort_by(|a, b| a.0.cmp(&b.0));
    let mut results = Vec::new();
    for (name, providers) in members {
        let mut providers = providers
            .into_iter()
            .map(|a| {
                Ok((
                    a.getattr("rank")?.extract::<i64>()?,
                    fold(&a.getattr("name")?.extract::<String>()?),
                    a,
                ))
            })
            .collect::<PyResult<Vec<_>>>()?;
        providers.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        results.push(
            api.getattr("Member")?
                .call1((name, PyTuple::new(py, providers.into_iter().map(|v| v.2))?))?,
        );
    }
    Ok(api
        .getattr("ArchiveIndex")?
        .call1((
            profile_id,
            PyTuple::new(py, archives)?,
            PyTuple::new(py, results)?,
            PyTuple::new(py, issues)?,
            signature,
            object(py, &effective)?,
            plugin_signature(&catalog)?,
        ))?
        .unbind())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(agent_archives, m)?)?;
    m.add_function(wrap_pyfunction!(archive_preview, m)?)?;
    m.add_function(wrap_pyfunction!(archive_source_stamp, m)?)?;
    m.add_function(wrap_pyfunction!(archive_plugin_signature, m)?)?;
    m.add_function(wrap_pyfunction!(archive_validate, m)?)?;
    m.add_function(wrap_pyfunction!(archive_cached, m)?)?;
    m.add_function(wrap_pyfunction!(archive_ini_settings, m)?)?;
    m.add_function(wrap_pyfunction!(archive_ini_archives, m)?)?;
    m.add_function(wrap_pyfunction!(archive_member_candidates, m)?)?;
    m.add_function(wrap_pyfunction!(archive_build, m)?)?;
    Ok(())
}

#[pyfunction]
fn agent_archives(
    root: &Bound<'_, PyAny>,
    wanted: &Bound<'_, PyAny>,
    archive: &str,
    member: &str,
    target: &str,
) -> PyResult<Py<PyAny>> {
    let py = root.py();
    let library = crate::agent_library::library(py, root)?;
    let id = crate::agent_library::profile(&library, wanted)?.getattr("id")?;
    let library = if target.is_empty() {
        library
    } else {
        py.import("creation_lib.mod_manager.launch_targets")?
            .call_method1("TargetLibrary", (library, target))?
    };
    let index = py
        .import("creation_lib.mod_manager.archive_index")?
        .call_method1("build", (&library, &id))?;
    if member.is_empty() {
        return Ok(py
            .import("dataclasses")?
            .call_method1("asdict", (index,))?
            .unbind());
    }
    let mut matches = Vec::new();
    for item in index.getattr("archives")?.try_iter()? {
        let item = item?;
        if fold(
            &item
                .getattr("provider")?
                .getattr("path")?
                .extract::<String>()?,
        ) == fold(archive)
            || fold(&item.getattr("name")?.extract::<String>()?) == fold(archive)
        {
            matches.push(item);
        }
    }
    if matches.len() != 1 {
        return Err(invalid(
            "Select one archive by its exact provider path (names must be unique).",
        ));
    }
    let selected = &matches[0];
    let preview = archive_preview(selected, member, py.None().bind(py))?;
    let result = py
        .import("dataclasses")?
        .call_method1("asdict", (preview,))?;
    let pixels = result.call_method1("pop", ("pixels",))?;
    if !pixels.is_none() {
        result.set_item("image_shape", pixels.getattr("shape")?)?;
    }
    result.set_item(
        "provider",
        py.import("dataclasses")?
            .call_method1("asdict", (selected.getattr("provider")?,))?,
    )?;
    Ok(result.unbind())
}

#[pyfunction]
fn archive_preview(
    archive: &Bound<'_, PyAny>,
    member: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = archive.py();
    let member = relative_path(member, false)?;
    let provider = archive.getattr("provider")?;
    let path: PathBuf = provider.getattr("path")?.extract()?;
    let expected = (
        provider.getattr("size")?.extract::<u64>()?,
        provider.getattr("modified")?.extract::<i64>()?,
    );
    if source_stamp(py, &path)? != expected {
        return Err(invalid(
            "Archive provider changed; refresh its inventory before previewing.",
        ));
    }
    check_cancel(Some(cancel))?;
    let data: Vec<u8> = py
        .import("creation_lib.ba2.native_runtime")?
        .call_method1(
            "extract_one_bounded",
            (py_path(py, &path)?, &member, 8 * 1024 * 1024),
        )
        .map_err(|e| invalid(format!("Cannot preview archive member: {}", e.value(py))))?
        .extract()?;
    check_cancel(Some(cancel))?;
    if source_stamp(py, &path)? != expected {
        return Err(invalid(
            "Archive changed during preview; refresh its inventory.",
        ));
    }
    let extension = Path::new(&member)
        .extension()
        .and_then(|v| v.to_str())
        .map(fold)
        .unwrap_or_default();
    if ["png", "jpg", "jpeg", "bmp", "gif", "webp", "dds"].contains(&extension.as_str()) {
        let pixels = crate::installer_preview::decode_image(py, &data)?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("pixels", pixels)?;
        Ok(py
            .import("creation_lib.mod_manager.package_preview")?
            .getattr("Preview")?
            .call((&member, data.len(), "image"), Some(&kwargs))?
            .unbind())
    } else {
        crate::installer_preview::preview_text(
            py,
            &member,
            data.len(),
            &data[..data.len().min(65536)],
        )
    }
}
