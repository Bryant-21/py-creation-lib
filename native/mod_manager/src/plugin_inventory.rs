use crate::{
    check_cancel, encoding,
    history_prune::py_path,
    invalid,
    plugin_state::{lines, locked_order, plugin_name, snapshot},
    python_io::with_context,
    reject_links,
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

pub fn value(object: &Bound<'_, PyAny>) -> PyResult<Value> {
    encoding::parse(
        &object
            .py()
            .import("json")?
            .call_method1("dumps", (object,))?
            .extract::<String>()?,
    )
}
pub fn object<'py>(py: Python<'py>, value: &Value) -> PyResult<Bound<'py, PyAny>> {
    py.import("json")?
        .call_method1("loads", (value.to_string(),))
}
pub fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("creation_lib.mod_manager.plugin_state")
}
pub fn signature_for(
    library: &Bound<'_, PyAny>,
    db: &Bound<'_, PyAny>,
    profile: &str,
) -> PyResult<String> {
    if library.hasattr("database_signature")? {
        library
            .call_method1("database_signature", (db, profile))?
            .extract()
    } else {
        let native = db.getattr("native")?;
        let db = native.extract::<PyRef<'_, crate::database::Database>>()?;
        crate::library::signature(&*db.connection()?, profile)
    }
}
#[pyfunction]
pub fn plugin_inventory_signature(library: &Bound<'_, PyAny>, profile: &str) -> PyResult<String> {
    with_context(&library.call_method0("_db")?, |db| {
        signature_for(library, db, profile)
    })
}
#[pyfunction]
pub fn plugin_inventory_signature_for(
    library: &Bound<'_, PyAny>,
    db: &Bound<'_, PyAny>,
    profile: &str,
) -> PyResult<String> {
    signature_for(library, db, profile)
}
#[pyfunction]
pub fn plugin_provider_files(root: PathBuf, cancel: &Bound<'_, PyAny>) -> PyResult<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(root)? {
        check_cancel(Some(cancel))?;
        if paths.len() >= 100_000 {
            return Err(invalid("Plugin provider exceeds its 100,000-entry limit."));
        }
        paths.push(entry?.path());
    }
    Ok(paths)
}
fn fingerprint(py: Python<'_>, path: &Path) -> PyResult<(u64, i128)> {
    let stat = py_path(py, path)?.call_method0("stat")?;
    Ok((
        stat.getattr("st_size")?.extract()?,
        stat.getattr("st_mtime_ns")?.extract()?,
    ))
}
#[pyfunction]
pub fn plugin_inventory(
    library: &Bound<'_, PyAny>,
    profile: &str,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let py = library.py();
    let kwargs = PyDict::new(py);
    kwargs.set_item("include_signature", true)?;
    let data = library.call_method("profile_snapshot", (profile,), Some(&kwargs))?;
    let files = value(&data.get_item(0)?.getattr("files")?)?;
    let config = value(&data.get_item(2)?)?;
    let initial: String = data.get_item(3)?.extract()?;
    let domain: String = py
        .import("creation_lib.mod_manager.nexus")?
        .call_method1(
            "game_domain",
            (files["_modbox_game"].as_str().unwrap_or(""),),
        )?
        .extract()?;
    let game = match domain.as_str() {
        "fallout4" => "fo4",
        "skyrimspecialedition" => "skyrimse",
        "skyrim" => "skyrim",
        "fallout3" => "fo3",
        "newvegas" => "fnv",
        "starfield" => "starfield",
        _ => {
            return Err(invalid(
                "This profile needs a supported game identity before plugin management.",
            ));
        }
    };
    let mut sources = Vec::<(PathBuf, String, String)>::new();
    let game_root = config["game"]["gamepath"].as_str().unwrap_or("");
    if !game_root.is_empty() {
        let args = PyDict::new(py);
        args.set_item("directory", true)?;
        let root: PathBuf = py
            .import("creation_lib.mod_manager.launcher")?
            .getattr("local_path")?
            .call((game_root, "Plugin game directory"), Some(&args))?
            .extract()?;
        sources.push((root.join("Data"), "Game Data".into(), "".into()));
    }
    for row in data.get_item(1)?.try_iter()? {
        check_cancel(Some(cancel))?;
        let row = row?;
        let source: String = row.getattr("source")?.extract()?;
        if row.getattr("enabled")?.is_truthy()?
            && row.getattr("kind")?.extract::<String>()? != "separator"
            && !source.is_empty()
        {
            let root = PathBuf::from(source);
            if !root.is_absolute()
                || !py
                    .import("creation_lib.mod_manager.mo2")?
                    .call_method1("local_mod_target", (py_path(py, &root)?,))?
                    .is_truthy()?
            {
                return Err(invalid(
                    "Plugin provider must use an absolute path on a local drive.",
                ));
            }
            reject_links(&root)?;
            sources.push((
                root,
                row.getattr("name")?.extract()?,
                row.getattr("id")?.extract()?,
            ));
        }
    }
    if sources.len() > 100_000 {
        return Err(invalid(
            "Plugin inventory exceeds its 100,000-provider limit.",
        ));
    }
    let overwrite = library
        .getattr("root")?
        .extract::<PathBuf>()?
        .join("runtime")
        .join("profiles")
        .join(crate::transactions::transaction_key(profile)?)
        .join("overwrite");
    reject_links(&overwrite)?;
    if overwrite.is_dir() {
        sources.push((overwrite, "Runtime overwrite".into(), "".into()));
    }
    let mut found = BTreeMap::<String, Value>::new();
    let mut ghosts = BTreeMap::new();
    let mut issues = Vec::new();
    for (index, (root, provider, mod_id)) in sources.iter().enumerate() {
        check_cancel(Some(cancel))?;
        reject_links(root)?;
        if !root.is_dir() {
            issues.push(format!("Plugin provider unavailable: {provider}"));
            continue;
        }
        let mut seen = BTreeSet::new();
        for path in plugin_provider_files(root.clone(), cancel)? {
            check_cancel(Some(cancel))?;
            let physical = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let lower = fold(&physical);
            let ghosted = [".esp.ghost", ".esm.ghost", ".esl.ghost"]
                .iter()
                .any(|s| lower.ends_with(s));
            if !ghosted && ![".esp", ".esm", ".esl"].iter().any(|s| lower.ends_with(s)) {
                continue;
            }
            reject_links(&path)?;
            if !path.is_file() {
                continue;
            }
            let name = plugin_name(if ghosted {
                &physical[..physical.len() - 6]
            } else {
                &physical
            })?;
            if !seen.insert(lower) {
                return Err(invalid(format!(
                    "Ambiguous case-insensitive plugin names in {provider}: {name}"
                )));
            }
            let entry = json!({"name":name,"path":path,"provider":provider,"mod_id":mod_id,"library_priority":index,"fingerprint":fingerprint(py,&path)?,"ghosted":ghosted,"header":null,"masters":null});
            if ghosted {
                ghosts.insert(fold(&name), entry);
            } else {
                found.insert(fold(&name), entry);
            }
            if found.len() > 10_000 {
                return Err(invalid("Plugin inventory exceeds its 10,000-name limit."));
            }
        }
        if !progress.is_none() {
            progress.call1((
                (index + 1) as f64 / sources.len().max(1) as f64,
                format!("Plugins: {provider}"),
            ))?;
        }
    }
    for (key, entry) in ghosts {
        found.entry(key).or_insert(entry);
    }
    if found.len() > 10_000 {
        return Err(invalid("Plugin inventory exceeds its 10,000-name limit."));
    }
    let states = lines(
        &snapshot(&files, "plugins.txt"),
        !["fo3", "fnv", "skyrim"].contains(&game),
    )?;
    let mut ordered = lines(&snapshot(&files, "loadorder.txt"), false)?
        .into_iter()
        .map(|v| v.0)
        .collect::<Vec<_>>();
    let custom_order = !ordered.is_empty();
    let mut library_plugins = found.values().collect::<Vec<_>>();
    library_plugins.sort_by_key(|v| {
        (
            v["library_priority"].as_u64().unwrap_or(0),
            fold(v["name"].as_str().unwrap()),
        )
    });
    let mut active = states
        .iter()
        .filter(|v| v.1)
        .map(|v| fold(&v.0))
        .collect::<BTreeSet<_>>();
    let mut seen = ordered.iter().map(|v| fold(v)).collect::<BTreeSet<_>>();
    for name in library_plugins
        .iter()
        .map(|v| v["name"].as_str().unwrap().to_string())
        .chain(states.iter().map(|v| v.0.clone()))
    {
        if seen.insert(fold(&name)) {
            ordered.push(name);
        }
    }
    let base: &[&str] = match game {
        "fo4" => &[
            "Fallout4.esm",
            "DLCRobot.esm",
            "DLCworkshop01.esm",
            "DLCCoast.esm",
            "DLCworkshop02.esm",
            "DLCworkshop03.esm",
            "DLCNukaWorld.esm",
            "DLCUltraHighResolution.esm",
        ],
        "skyrim" | "skyrimse" => &["Skyrim.esm"],
        "fo3" => &["Fallout3.esm"],
        "fnv" => &["FalloutNV.esm"],
        _ => &["Starfield.esm"],
    };
    let mut primary = base
        .iter()
        .filter(|v| found.contains_key(&fold(v)))
        .map(|v| v.to_string())
        .collect::<Vec<_>>();
    if game == "fo4" && !game_root.is_empty() {
        let ccc = PathBuf::from(game_root).join("Fallout4.ccc");
        reject_links(&ccc)?;
        if ccc.is_file() {
            let args = PyDict::new(py);
            args.set_item("limit", 1024 * 1024)?;
            let content: String = py
                .import("creation_lib.mod_manager.mo2")?
                .getattr("read_text")?
                .call((py_path(py, &ccc)?,), Some(&args))?
                .extract()?;
            for (name, _) in lines(&content, false)? {
                if !found.contains_key(&fold(&name)) {
                    issues.push(format!("Creation Club entry is unavailable: {name}"));
                } else if !primary.iter().any(|v| fold(v) == fold(&name)) {
                    primary.push(name);
                }
            }
        }
    }
    let forced = primary.iter().map(|v| fold(v)).collect::<BTreeSet<_>>();
    primary.extend(ordered.into_iter().filter(|v| !forced.contains(&fold(v))));
    ordered = primary;
    active.extend(forced.clone());
    let mut plugins = Vec::new();
    for (priority, name) in ordered.iter().enumerate() {
        let key = fold(name);
        let mut entry = found.get(&key).cloned().unwrap_or_else(||json!({"name":name,"path":"","provider":"Missing","mod_id":"","fingerprint":null,"ghosted":false,"header":null,"masters":null}));
        if entry["ghosted"] == true && active.contains(&key) {
            issues.push(format!("Ghosted plugin is marked active in local metadata: {name}. Deactivate it before review."));
        }
        entry["enabled"] = json!(active.contains(&key));
        entry["priority"] = json!(priority);
        entry["forced"] = json!(forced.contains(&key));
        plugins.push(entry);
    }
    if initial != plugin_inventory_signature(library, profile)? {
        return Err(invalid(
            "Local profile changed during plugin inventory; refresh it.",
        ));
    }
    let locks = locked_order(&snapshot(&files, "lockedorder.txt"))?
        .into_iter()
        .filter(|v| !forced.contains(&fold(&v.0)))
        .collect::<Vec<_>>();
    Ok(json!({"profile_id":profile,"signature":initial,"game":game,"plugins":plugins,"issues":issues,"locked":locks,"custom_order":custom_order}).to_string())
}
#[pyfunction]
fn plugin_metadata(path: PathBuf, game: &str, cancel: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = cancel.py();
    let reader = py
        .import("creation_lib.mod_manager.cli_reader")?
        .getattr("run_json")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("game", game)?;
    kwargs.set_item("cancel", cancel)?;
    let path = path.to_string_lossy();
    let header = reader.call((("esp", "header", path.as_ref()),), Some(&kwargs))?;
    let masters = reader.call((("esp", "masters", "list", path.as_ref()),), Some(&kwargs))?;
    let mut names = Vec::new();
    for master in masters.get_item("masters")?.try_iter()? {
        names.push(plugin_name(
            &master?.get_item("name")?.extract::<String>()?,
        )?);
    }
    Ok((header, PyTuple::new(py, names)?)
        .into_pyobject(py)?
        .into_any()
        .unbind())
}
#[pyfunction]
fn plugin_cached_metadata(
    library: &Bound<'_, PyAny>,
    reader: &Bound<'_, PyAny>,
    path: PathBuf,
    game: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    reject_links(&path)?;
    let target = py_path(py, &path)?;
    let canonical = fold(&target.call_method0("resolve")?.str()?.to_string());
    let before = fingerprint(py, &path)?;
    let cached = with_context(&library.call_method0("_db")?, |db| {
        db.call_method1("execute",("SELECT metadata FROM plugin_metadata WHERE path=? AND game=? AND size=? AND modified=?",(&canonical,game,before.0,before.1)))?.call_method0("fetchone").map(Bound::unbind)
    })?;
    let cached = cached.bind(py);
    check_cancel(Some(cancel))?;
    if !cached.is_none() {
        if let Ok(data) = encoding::parse(&cached.get_item(0)?.extract::<String>()?) {
            if data["version"] == 1 && data["header"].is_object() {
                if let Some(masters) = data["masters"].as_array() {
                    let names = masters
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .ok_or_else(|| invalid("Invalid master name"))
                                .and_then(plugin_name)
                        })
                        .collect::<PyResult<Vec<_>>>();
                    if let Ok(names) = names {
                        check_cancel(Some(cancel))?;
                        return Ok((object(py, &data["header"])?, PyTuple::new(py, names)?)
                            .into_pyobject(py)?
                            .into_any()
                            .unbind());
                    }
                }
            }
        }
    }
    let result = reader.call1((&target, game, cancel))?;
    check_cancel(Some(cancel))?;
    if before != fingerprint(py, &path)? {
        return Err(invalid(
            "Plugin changed while inspecting metadata; no cache was saved.",
        ));
    }
    let data=json!({"version":1,"header":value(&result.get_item(0)?)?,"masters":value(&result.get_item(1)?)?}).to_string();
    if data.len() > 2 * 1024 * 1024 {
        return Err(invalid(
            "Plugin metadata exceeds its 2 MiB cache safety limit.",
        ));
    }
    with_context(&library.call_method0("_db")?, |db| {
        check_cancel(Some(cancel))?;
        db.call_method1("execute",("INSERT INTO plugin_metadata VALUES (?,?,?,?,?) ON CONFLICT(path,game) DO UPDATE SET size=excluded.size,modified=excluded.modified,metadata=excluded.metadata",(&canonical,game,before.0,before.1,data)))?;
        check_cancel(Some(cancel))
    })?;
    Ok(result.unbind())
}
#[pyfunction]
fn plugin_inspect(
    catalog: &Bound<'_, PyAny>,
    reader: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = catalog.py();
    let source = catalog.getattr("plugins")?;
    let total = source.len()?.max(1);
    let mut plugins = Vec::new();
    let mut inspection_issues: Vec<String> = catalog.getattr("issues")?.extract()?;
    for (index, plugin) in source.try_iter()?.enumerate() {
        check_cancel(Some(cancel))?;
        let plugin = plugin?;
        let path: String = plugin.getattr("path")?.extract()?;
        if path.is_empty() {
            plugins.push(plugin);
            continue;
        }
        let path = PathBuf::from(path);
        reject_links(&path)?;
        let expected: (u64, i128) = plugin.getattr("fingerprint")?.extract()?;
        if fingerprint(py, &path)? != expected {
            return Err(invalid(
                "Plugin source changed. Refresh inventory before inspection.",
            ));
        }
        let result = match reader.call1((py_path(py, &path)?, catalog.getattr("game")?, cancel)) {
            Ok(result) => result,
            Err(error) => {
                check_cancel(Some(cancel))?;
                let paths = py.import("creation_lib.mod_manager.paths")?;
                if error.is_instance(py, &paths.getattr("Cancelled")?)
                    || !error.is_instance(py, &paths.getattr("ManagerError")?)
                {
                    return Err(error);
                }
                inspection_issues.push(format!(
                    "Cannot read {}: {error}",
                    plugin.getattr("name")?.extract::<String>()?
                ));
                let fields = PyDict::new(py);
                fields.set_item("header", py.None())?;
                fields.set_item("masters", py.None())?;
                plugins.push(
                    py.import("dataclasses")?
                        .getattr("replace")?
                        .call((&plugin,), Some(&fields))?,
                );
                continue;
            }
        };
        if fingerprint(py, &path)? != expected {
            return Err(invalid("Plugin changed during metadata inspection."));
        }
        let kwargs = PyDict::new(py);
        kwargs.set_item("header", result.get_item(0)?)?;
        kwargs.set_item(
            "masters",
            py.import("builtins")?
                .getattr("tuple")?
                .call1((result.get_item(1)?,))?,
        )?;
        plugins.push(
            py.import("dataclasses")?
                .getattr("replace")?
                .call((&plugin,), Some(&kwargs))?,
        );
        if !progress.is_none() {
            progress.call1((
                (index + 1) as f64 / total as f64,
                format!("Header: {}", plugin.getattr("name")?.extract::<String>()?),
            ))?;
        }
    }
    check_cancel(Some(cancel))?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("plugins", PyTuple::new(py, plugins)?)?;
    kwargs.set_item("issues", PyTuple::new(py, inspection_issues)?)?;
    let inspected = py
        .import("dataclasses")?
        .getattr("replace")?
        .call((catalog,), Some(&kwargs))?;
    if !catalog.getattr("custom_order")?.is_truthy()? && catalog.getattr("locked")?.len()? == 0 {
        let serialized: String = py
            .import("creation_lib.mod_manager.plugin_state")?
            .call_method1("_catalog_json", (&inspected,))?
            .extract()?;
        match crate::plugin_state::plugin_state_default_order(&serialized) {
            Ok(order) => {
                let by_name = inspected
                    .getattr("plugins")?
                    .try_iter()?
                    .map(|p| {
                        let p = p?;
                        Ok((p.getattr("name")?.extract::<String>()?, p))
                    })
                    .collect::<PyResult<BTreeMap<_, _>>>()?;
                let ordered = order
                    .iter()
                    .enumerate()
                    .map(|(priority, name)| {
                        let fields = PyDict::new(py);
                        fields.set_item("priority", priority)?;
                        py.import("dataclasses")?
                            .getattr("replace")?
                            .call((&by_name[name],), Some(&fields))
                    })
                    .collect::<PyResult<Vec<_>>>()?;
                kwargs.set_item("plugins", PyTuple::new(py, ordered)?)?;
            }
            Err(error) => {
                let mut issues: Vec<String> = inspected.getattr("issues")?.extract()?;
                issues.push(error.to_string());
                kwargs.set_item("issues", PyTuple::new(py, issues)?)?;
            }
        }
        return Ok(py
            .import("dataclasses")?
            .getattr("replace")?
            .call((&inspected,), Some(&kwargs))?
            .unbind());
    }
    Ok(inspected.unbind())
}
fn providers(catalog: &Value) -> Vec<String> {
    let mut result = catalog["plugins"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            json!([
                fold(p["name"].as_str().unwrap_or("")),
                fold(p["path"].as_str().unwrap_or("")),
                p["fingerprint"]
            ])
            .to_string()
        })
        .collect::<Vec<_>>();
    result.sort();
    result
}
#[pyfunction]
fn plugin_apply(
    library: &Bound<'_, PyAny>,
    plan: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let api = module(py)?;
    let catalog_obj = plan.getattr("catalog")?;
    let catalog = encoding::parse(
        &api.call_method1("_catalog_json", (&catalog_obj,))?
            .extract::<String>()?,
    )?;
    let profile = catalog["profile_id"].as_str().unwrap_or("");
    let signature = catalog["signature"].as_str().unwrap_or("");
    if plugin_inventory_signature(library, profile)? != signature {
        return Err(invalid(
            "Profile changed after plugin review. Refresh and review again.",
        ));
    }
    let checked = api.call_method1(
        "review",
        (
            &catalog_obj,
            plan.getattr("order")?,
            plan.getattr("enabled")?,
            plan.getattr("locked")?,
        ),
    )?;
    let current = encoding::parse(&plugin_inventory(
        library,
        profile,
        cancel,
        py.None().bind(py),
    )?)?;
    if providers(&current) != providers(&catalog) {
        return Err(invalid(
            "Plugin inventory/provider changed after review. Refresh before saving.",
        ));
    }
    for plugin in catalog["plugins"].as_array().into_iter().flatten() {
        check_cancel(Some(cancel))?;
        let path = plugin["path"].as_str().unwrap_or("");
        if !path.is_empty() {
            let path = PathBuf::from(path);
            reject_links(&path)?;
            if json!(fingerprint(py, &path)?) != plugin["fingerprint"] {
                return Err(invalid(
                    "Plugin changed after review; no local state was saved.",
                ));
            }
        }
    }
    let order: Vec<String> = checked.getattr("order")?.extract()?;
    let active: BTreeSet<String> = checked.getattr("enabled")?.extract()?;
    let locks: Vec<(String, usize)> = checked.getattr("locked")?.extract()?;
    with_context(&library.call_method0("_db")?, |db| {
        db.call_method1("execute", ("BEGIN IMMEDIATE",))?;
        if signature_for(library, db, profile)? != signature {
            return Err(invalid(
                "Profile changed after plugin review. No local state was saved.",
            ));
        }
        let record = db
            .call_method1(
                "execute",
                ("SELECT files FROM profiles WHERE id=?", (profile,)),
            )?
            .call_method0("fetchone")?;
        let mut files = encoding::parse(&record.get_item(0)?.extract::<String>()?)?;
        if library.hasattr("profile_files")? {
            files =
                value(&library.call_method1("profile_files", (db, profile, object(py, &files)?))?)?;
        }
        files["_modbox_plugin_backup"]=json!(json!({"plugins.txt":snapshot(&files,"plugins.txt"),"loadorder.txt":snapshot(&files,"loadorder.txt"),"lockedorder.txt":snapshot(&files,"lockedorder.txt")}).to_string());
        files
            .as_object_mut()
            .ok_or_else(|| invalid("Invalid profile files"))?
            .retain(|k, _| {
                !["plugins.txt", "loadorder.txt", "lockedorder.txt"].contains(&fold(k).as_str())
            });
        let modern = !["fo3", "fnv", "skyrim"].contains(&catalog["game"].as_str().unwrap_or(""));
        files["plugins.txt"] = json!(
            order
                .iter()
                .filter(|name| modern || active.contains(&fold(name)))
                .map(|name| format!(
                    "{}{name}\n",
                    if active.contains(&fold(name)) {
                        "*"
                    } else {
                        ""
                    }
                ))
                .collect::<String>()
        );
        files["loadorder.txt"] = json!(
            order
                .iter()
                .map(|name| format!("{name}\n"))
                .collect::<String>()
        );
        files["lockedorder.txt"] = json!(
            locks
                .iter()
                .map(|(name, position)| format!("{name}|{position}\n"))
                .collect::<String>()
        );
        let records=catalog["plugins"].as_array().unwrap().iter().map(|p|json!({"name":p["name"],"path":p["path"],"fingerprint":p["fingerprint"],"header":p["header"],"masters":p["masters"],"forced":p["forced"]})).collect::<Vec<_>>();
        files["_modbox_plugin_validation"]=json!(json!({"version":2,"game":catalog["game"],"order":order,"locked":locks,"enabled":active,"plugins":records}).to_string());
        check_cancel(Some(cancel))?;
        if library.hasattr("write_profile_fields")? {
            library.call_method1("write_profile_fields", (db, profile, object(py, &files)?))?;
        } else {
            db.call_method1(
                "execute",
                (
                    "UPDATE profiles SET files=? WHERE id=?",
                    (files.to_string(), profile),
                ),
            )?;
        }
        check_cancel(Some(cancel))
    })?;
    Ok(checked.unbind())
}
#[pyfunction]
#[pyo3(signature = (files, winners, game_root, overwrite, original_game_root=""))]
fn plugin_validate_runtime(
    py: Python<'_>,
    files: &str,
    winners: &Bound<'_, PyAny>,
    game_root: PathBuf,
    overwrite: PathBuf,
    original_game_root: &str,
) -> PyResult<()> {
    let files = encoding::parse(files)?;
    let Some(encoded) = files.get("_modbox_plugin_validation") else {
        return Ok(());
    };
    let result = (|| -> PyResult<()> {
        let validated = encoding::parse(
            encoded
                .as_str()
                .ok_or_else(|| invalid("Invalid plugin validation format"))?,
        )?;
        if validated["version"] != 1 && validated["version"] != 2 {
            return Err(invalid("Unsupported local plugin validation format."));
        }
        let records = validated["plugins"]
            .as_array()
            .ok_or_else(|| invalid("Invalid reviewed plugin records"))?;
        let states = lines(&snapshot(&files, "plugins.txt"), true)?;
        let names = states
            .iter()
            .filter(|v| v.1)
            .map(|v| fold(&v.0))
            .collect::<BTreeSet<_>>();
        let order = lines(&snapshot(&files, "loadorder.txt"), false)?
            .iter()
            .map(|v| fold(&v.0))
            .collect::<Vec<_>>();
        let enabled = validated["enabled"]
            .as_array()
            .ok_or_else(|| invalid("Invalid reviewed activation"))?
            .iter()
            .map(|v| v.as_str().unwrap_or("").to_string())
            .collect::<BTreeSet<_>>();
        let expected = validated["order"]
            .as_array()
            .ok_or_else(|| invalid("Invalid reviewed order"))?
            .iter()
            .map(|v| fold(v.as_str().unwrap_or("")))
            .collect::<Vec<_>>();
        if names != enabled
            || order != expected
            || states.iter().map(|v| fold(&v.0)).collect::<Vec<_>>() != order
        {
            return Err(invalid(
                "Local plugin activation/order changed outside the plugin review workflow.",
            ));
        }
        let locks = locked_order(&snapshot(&files, "lockedorder.txt"))?;
        if validated["version"] == 2 && json!(locks) != validated["locked"] {
            return Err(invalid(
                "Local plugin locks changed outside the plugin review workflow.",
            ));
        }
        let mut plugins = Vec::new();
        for record in records {
            let mut record = record.clone();
            let name = fold(record["name"].as_str().unwrap_or(""));
            let position = order
                .iter()
                .position(|v| v == &name)
                .ok_or_else(|| invalid("Reviewed plugin missing from order"))?;
            record["priority"] = json!(position);
            record["enabled"] = json!(names.contains(&name));
            plugins.push(record);
        }
        plugins.sort_by_key(|p| p["priority"].as_u64());
        let catalog = json!({"game":validated["game"],"plugins":plugins,"locked":locks});
        crate::plugin_state::review(
            &catalog,
            &json!(states.iter().map(|v| &v.0).collect::<Vec<_>>()),
            &json!(names),
            &Value::Null,
        )?;
        for record in records {
            let name = plugin_name(record["name"].as_str().unwrap_or(""))?;
            if !names.contains(&fold(&name)) {
                continue;
            }
            let owned = overwrite.join(&name);
            reject_links(&owned)?;
            let mapped = winners.call_method1("get", (fold(&name),))?;
            let path = if owned.is_file() {
                owned
            } else if !mapped.is_none() {
                mapped.get_item(1)?.extract::<PathBuf>()?
            } else {
                game_root.join("Data").join(&name)
            };
            reject_links(&path)?;
            if !path.is_file() {
                continue;
            }
            let actual = py_path(py, &path)?
                .call_method0("absolute")?
                .str()?
                .to_string();
            let expected = py_path(py, Path::new(record["path"].as_str().unwrap_or("")))?
                .call_method0("absolute")?
                .str()?
                .to_string();
            if fold(&actual) != fold(&expected) {
                let original_data = PathBuf::from(original_game_root).join("Data");
                let expected_path = PathBuf::from(&expected);
                if !original_game_root.is_empty()
                    && expected_path.parent().is_some_and(|p| {
                        fold(&p.to_string_lossy()) == fold(&original_data.to_string_lossy())
                    })
                    && path.parent().is_some_and(|p| {
                        fold(&p.to_string_lossy())
                            == fold(&game_root.join("Data").to_string_lossy())
                    })
                {
                    continue;
                }
                return Err(invalid(format!(
                    "{name} now uses another provider. Refresh plugin details and review again."
                )));
            }
            if json!(fingerprint(py, &path)?) != record["fingerprint"] {
                return Err(invalid(format!(
                    "{name} changed after its dependency review. Refresh plugin details."
                )));
            }
        }
        Ok(())
    })();
    result.map_err(|e| invalid(format!("Cannot validate reviewed local plugin state: {e}")))
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(plugin_inventory_signature, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_inventory_signature_for, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_provider_files, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_inventory, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_metadata, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_cached_metadata, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_inspect, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_apply, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_validate_runtime, m)?)?;
    Ok(())
}
