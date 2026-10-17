use crate::{
    check_cancel, encoding,
    history_prune::{py_path, stamp},
    invalid,
    plugin_inventory::{object, plugin_inventory_signature, value},
    reject_links, relative_path,
    saves::fold,
    transactions::transaction_key,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
fn digest(v: &Value) -> String {
    encoding::digest(v, true, false)
}
fn read(py: Python<'_>, path: &Path, limit: usize) -> PyResult<String> {
    let kwargs = PyDict::new(py);
    kwargs.set_item("limit", limit)?;
    py.import("creation_lib.mod_manager.mo2")?
        .getattr("read_text")?
        .call((py_path(py, path)?,), Some(&kwargs))?
        .extract()
}
fn identity(library: &Bound<'_, PyAny>) -> PyResult<(String, String)> {
    let py = library.py();
    let root = library
        .getattr("root")?
        .call_method0("absolute")?
        .extract::<PathBuf>()?;
    let marker = root.join(library.getattr("MARKER")?.extract::<String>()?);
    let ownership = (|| -> PyResult<Value> {
        reject_links(&root)?;
        reject_links(&marker)?;
        encoding::parse(&read(py, &marker, 65536)?)
    })();
    let ownership = ownership.map_err(|e| {
        if e.is_instance_of::<pyo3::exceptions::PyOSError>(py)
            || e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
        {
            invalid("Installer library ownership became unavailable. Prepare the package again.")
        } else {
            e
        }
    })?;
    let id = library.getattr("library_id")?.extract::<String>()?;
    if !ownership.is_object() || ownership["format"] != 1 || ownership["id"] != id {
        return Err(invalid(
            "Installer library identity changed. Prepare the package again.",
        ));
    }
    Ok((
        fold(
            &py_path(py, &root)?
                .call_method0("resolve")?
                .str()?
                .to_string(),
        ),
        id,
    ))
}
fn database(snapshot: &Bound<'_, PyAny>) -> PyResult<String> {
    let py = snapshot.py();
    let asdict = py.import("dataclasses")?.getattr("asdict")?;
    let profile = value(&asdict.call1((snapshot.get_item(0)?,))?)?;
    let mut rows = Vec::new();
    for row in snapshot.get_item(1)?.try_iter()? {
        rows.push(value(&asdict.call1((row?,))?)?);
    }
    Ok(digest(&json!([
        profile,
        rows,
        value(&snapshot.get_item(2)?)?
    ])))
}
fn directory(py: Python<'_>, path: &Path) -> PyResult<Value> {
    let path_py = py_path(py, path)?.call_method0("absolute")?;
    let path = path_py.extract::<PathBuf>()?;
    reject_links(&path)?;
    let info = match path_py.call_method0("stat") {
        Ok(v) => v,
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyFileNotFoundError>(py) => {
            return Ok(json!([fold(&path.to_string_lossy()), false, null]));
        }
        Err(e) => return Err(e),
    };
    if !path.is_dir() {
        return Err(invalid(
            "An installer Data provider is no longer an ordinary directory.",
        ));
    }
    let stamp = stamp(&info)?;
    Ok(json!([
        fold(&path.to_string_lossy()),
        true,
        [stamp.0, stamp.1, stamp.3]
    ]))
}
fn providers(
    library: &Bound<'_, PyAny>,
    profile: &str,
    rows: &Bound<'_, PyAny>,
    config: &Value,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<(Value, bool)> {
    let py = library.py();
    let mut result = Vec::new();
    let raw = config["game"]["gamepath"].as_str().unwrap_or("");
    let mut available = false;
    let local = py
        .import("creation_lib.mod_manager.mo2")?
        .getattr("local_mod_target")?;
    if !raw.is_empty() {
        check_cancel(Some(cancel))?;
        let game = PathBuf::from(raw);
        if !game.is_absolute() || !local.call1((py_path(py, &game)?,))?.is_truthy()? {
            return Err(invalid(
                "Installer game context requires an absolute ordinary local directory.",
            ));
        }
        result.push(json!(["game-root", directory(py, &game)?]));
        if library.hasattr("target_id")? && library.getattr("target_id")?.is_truthy()? {
            let executable = game.join("Fallout4.exe");
            reject_links(&executable)?;
            let actual = value(
                &py.import("creation_lib.mod_manager.launcher")?
                    .call_method1("fingerprint", (py_path(py, &executable)?,))?,
            )?;
            if actual != config["target"]["f4se"]["game_signature"] {
                return Err(invalid(
                    "Target game build changed; review the FO4 folder before preparing its installer context.",
                ));
            }
            result.push(json!([
                "target-executable",
                [fold(&executable.to_string_lossy()), actual]
            ]));
        }
        let data = directory(py, &game.join("Data"))?;
        available = data[1] == true;
        result.push(json!(["game-data", data]));
    }
    for row in rows.try_iter()? {
        check_cancel(Some(cancel))?;
        let row = row?;
        if row.getattr("enabled")?.is_truthy()? && !row.getattr("kind")?.eq("separator")? {
            let source = row.getattr("source")?.extract::<String>()?;
            if source.is_empty() {
                return Err(invalid(
                    "Resolve unavailable enabled mod sources before preparing the installer.",
                ));
            }
            let path = PathBuf::from(source);
            if !path.is_absolute() || !local.call1((py_path(py, &path)?,))?.is_truthy()? {
                return Err(invalid(
                    "Installer mod sources require absolute ordinary local directories.",
                ));
            }
            let state = directory(py, &path)?;
            if state[1] != true {
                return Err(invalid(
                    "Resolve unavailable enabled mod sources before preparing the installer.",
                ));
            }
            result.push(json!([row.getattr("id")?.extract::<String>()?, state]));
        }
    }
    let overwrite = library
        .getattr("root")?
        .extract::<PathBuf>()?
        .join("runtime")
        .join("profiles")
        .join(transaction_key(profile)?)
        .join("overwrite");
    check_cancel(Some(cancel))?;
    result.push(json!(["runtime-overwrite", directory(py, &overwrite)?]));
    Ok((json!(result), available))
}
fn ccc(py: Python<'_>, config: &Value, game: &str, cancel: &Bound<'_, PyAny>) -> PyResult<Value> {
    let root = config["game"]["gamepath"].as_str().unwrap_or("");
    if game != "fo4" || root.is_empty() {
        return Ok(Value::Null);
    }
    let path = PathBuf::from(root).join("Fallout4.ccc");
    reject_links(&path)?;
    check_cancel(Some(cancel))?;
    let target = py_path(py, &path)?;
    let before = match target.call_method0("stat") {
        Ok(v) => stamp(&v)?,
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyFileNotFoundError>(py) => {
            return Ok(json!([fold(&path.to_string_lossy()), false, null, ""]));
        }
        Err(e) => return Err(e),
    };
    if !path.is_file() {
        return Err(invalid(
            "Creation Club plugin context must be an ordinary file.",
        ));
    }
    let content = read(py, &path, 1024 * 1024)?;
    check_cancel(Some(cancel))?;
    reject_links(&path)?;
    if before != stamp(&target.call_method0("stat")?)? {
        return Err(invalid(
            "Creation Club context changed during installer preparation.",
        ));
    }
    Ok(json!([
        fold(&path.to_string_lossy()),
        true,
        before,
        content
    ]))
}
fn plugin_context(
    library: &Bound<'_, PyAny>,
    profile: &Bound<'_, PyAny>,
    config: &Value,
    winners: &BTreeMap<String, Value>,
    game: &str,
    available: bool,
    ccc: &Value,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Value> {
    let py = library.py();
    let root = config["game"]["gamepath"].as_str().unwrap_or("");
    let mut result = Vec::new();
    let api = py.import("creation_lib.mod_manager.plugin_state")?;
    if !game.is_empty() && (root.is_empty() || Path::new(root).is_dir()) {
        let catalog = api.call_method1("inventory", (library, profile.getattr("id")?, cancel))?;
        for plugin in catalog.getattr("plugins")?.try_iter()? {
            check_cancel(Some(cancel))?;
            let plugin = plugin?;
            let name = fold(&plugin.getattr("name")?.extract::<String>()?);
            let path = plugin.getattr("path")?.extract::<String>()?;
            let fingerprint = value(&plugin.getattr("fingerprint")?)?;
            let state = if plugin.getattr("ghosted")?.is_truthy()? {
                "Missing"
            } else if !path.is_empty() {
                let winner = winners.get(&name).ok_or_else(|| {
                    invalid("Plugin providers changed during installer preparation. Prepare again.")
                })?;
                if fold(winner[1].as_str().unwrap_or("")) != fold(&path)
                    || json!([winner[2], winner[3]]) != fingerprint
                {
                    return Err(invalid(
                        "Plugin providers changed during installer preparation. Prepare again.",
                    ));
                }
                if plugin.getattr("enabled")?.is_truthy()?
                    || plugin.getattr("forced")?.is_truthy()?
                {
                    "Active"
                } else {
                    "Inactive"
                }
            } else if available {
                "Missing"
            } else {
                "Unknown"
            };
            result.push(json!([
                name,
                fold(&path),
                fingerprint,
                state,
                plugin.getattr("forced")?.is_truthy()?
            ]));
        }
        return Ok(json!(result));
    }
    let files = value(&profile.getattr("files")?)?;
    let states =
        crate::plugin_state::lines(&crate::plugin_state::snapshot(&files, "plugins.txt"), true)?;
    let classic = ["fo3", "fnv", "skyrim"].contains(&game);
    let active = states
        .iter()
        .filter(|v| classic || v.1)
        .map(|v| fold(&v.0))
        .collect::<BTreeSet<_>>();
    let primary = if game == "fo4" {
        api.getattr("FO4_PRIMARY")?.extract::<Vec<String>>()?
    } else {
        vec![
            api.getattr("BASE")?
                .call_method1("get", (game, ""))?
                .extract()?,
        ]
    };
    let mut forced = primary
        .iter()
        .map(|v| fold(v))
        .filter(|v| winners.contains_key(v))
        .collect::<BTreeSet<_>>();
    if ccc[1] == true {
        for (name, _) in crate::plugin_state::lines(ccc[3].as_str().unwrap_or(""), false)? {
            if winners.contains_key(&fold(&name)) {
                forced.insert(fold(&name));
            }
        }
    }
    for (name, record) in winners {
        if name.contains('/') || ![".esp", ".esm", ".esl"].iter().any(|s| name.ends_with(s)) {
            continue;
        }
        let state = if active.contains(name) || forced.contains(name) {
            "Active"
        } else if !game.is_empty() {
            "Inactive"
        } else {
            "Unknown"
        };
        result.push(json!([
            name,
            fold(record[1].as_str().unwrap_or("")),
            [record[2], record[3]],
            state,
            forced.contains(name)
        ]));
    }
    Ok(json!(result))
}
fn verify(
    winners: &BTreeMap<String, Value>,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = cancel.py();
    let mut directories = BTreeMap::new();
    for (count, record) in winners.values().enumerate() {
        check_cancel(Some(cancel))?;
        let path = PathBuf::from(record[1].as_str().unwrap_or(""));
        let directory = path
            .parent()
            .ok_or_else(|| invalid("Invalid installer provider"))?
            .to_path_buf();
        if !directories.contains_key(&directory) {
            reject_links(&directory)?;
            let info = py_path(py, &directory)?.call_method0("lstat")?;
            if info.getattr("st_mode")?.extract::<u32>()? & 0o170000 != 0o040000 {
                return Err(invalid(
                    "Installer provider directory became unavailable. Prepare again.",
                ));
            }
            let info = stamp(&info)?;
            directories.insert(directory, (info.0, info.1));
        }
        let info = py_path(py, &path)?.call_method0("lstat")?;
        let flags = if info.hasattr("st_file_attributes")? {
            info.getattr("st_file_attributes")?.extract::<u32>()?
        } else {
            0
        };
        let stat = stamp(&info)?;
        if info.getattr("st_mode")?.extract::<u32>()? & 0o170000 != 0o100000
            || flags & 0x400 != 0
            || json!(stat.2) != record[2]
            || json!(stat.3) != record[3]
        {
            return Err(invalid(
                "Installer files changed during dependency preparation. Prepare again.",
            ));
        }
        if !progress.is_none() && count % 1024 == 0 {
            let count = py
                .import("builtins")?
                .call_method1("format", (winners.len(), ","))?
                .extract::<String>()?;
            progress.call1((
                py.None(),
                format!("Checking {count} installer dependency file fingerprints"),
            ))?;
        }
    }
    for (directory, identity) in directories {
        check_cancel(Some(cancel))?;
        reject_links(&directory)?;
        let info = py_path(py, &directory)?.call_method0("lstat")?;
        let stat = stamp(&info)?;
        if (stat.0, stat.1) != identity
            || info.getattr("st_mode")?.extract::<u32>()? & 0o170000 != 0o040000
        {
            return Err(invalid(
                "Installer provider directory changed during dependency preparation. Prepare again.",
            ));
        }
    }
    Ok(())
}
#[pyfunction]
fn installer_verify_winners(
    winners: &str,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let data = encoding::parse(winners)?;
    let winners = data
        .as_object()
        .ok_or_else(|| invalid("Invalid dependency files"))?
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    verify(&winners, cancel, progress)
}
fn tuple_value<'py>(py: Python<'py>, value: &Value) -> PyResult<Bound<'py, PyAny>> {
    if let Some(items) = value.as_array() {
        Ok(PyTuple::new(
            py,
            items
                .iter()
                .map(|v| tuple_value(py, v))
                .collect::<PyResult<Vec<_>>>()?,
        )?
        .into_any())
    } else {
        object(py, value)
    }
}
fn scan(
    library: &Bound<'_, PyAny>,
    profile_id: &str,
    requested: Vec<String>,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
    version_requests: Vec<String>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    check_cancel(Some(cancel))?;
    let initial_identity = identity(library)?;
    let snapshot = library.call_method1("profile_snapshot", (profile_id,))?;
    let profile = snapshot.get_item(0)?;
    let rows = snapshot.get_item(1)?;
    let config = value(&snapshot.get_item(2)?)?;
    let database = database(&snapshot)?;
    let signature = plugin_inventory_signature(library, profile_id)?;
    let (providers, available) = providers(library, profile_id, &rows, &config, cancel)?;
    let domain = py.import("creation_lib.mod_manager.nexus")?.call_method1(
        "game_domain",
        (profile
            .getattr("files")?
            .call_method1("get", ("_modbox_game", ""))?,),
    )?;
    let game = py
        .import("creation_lib.mod_manager.plugin_state")?
        .getattr("DOMAINS")?
        .call_method1("get", (domain, ""))?
        .extract::<String>()?;
    let ccc = ccc(py, &config, &game, cancel)?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("include_disabled", false)?;
    kwargs.set_item("build_tree", false)?;
    let index = py
        .import("creation_lib.mod_manager.conflicts")?
        .getattr("build_index")?
        .call((library, profile_id, cancel, progress), Some(&kwargs))?;
    if index.getattr("unavailable_mod_ids")?.is_truthy()? {
        return Err(invalid(
            "Resolve unavailable enabled Data providers before preparing the installer.",
        ));
    }
    let mut hash = Sha256::new();
    let mut winners = BTreeMap::new();
    let mut states = BTreeMap::new();
    let mut count = 0;
    for entry in index.getattr("files")?.try_iter()? {
        check_cancel(Some(cancel))?;
        let entry = entry?;
        let winner = entry.getattr("winner")?;
        let path = entry.getattr("path")?.extract::<String>()?;
        if !winner.is_truthy()?
            || path.split('/').any(|p| {
                let p = fold(p);
                p.ends_with(".ghost") || p.ends_with(".mohidden")
            })
        {
            continue;
        }
        let name = fold(&path);
        let source = winner.getattr("path")?.extract::<String>()?;
        let size = value(&winner.getattr("size")?)?;
        let modified = value(&winner.getattr("modified")?)?;
        winners.insert(name.clone(), json!([name, source, size, modified]));
        states.insert(name.clone(), "Active".to_string());
        hash.update(
            encoding::encode(&json!([name, fold(&source), size, modified]), true, false).as_bytes(),
        );
        hash.update(b"\n");
        count += 1;
    }
    let plugins = plugin_context(
        library, &profile, &config, &winners, &game, available, &ccc, cancel,
    )?;
    verify(&winners, cancel, progress)?;
    for plugin in plugins.as_array().unwrap() {
        let name = plugin[0].as_str().unwrap();
        let state = plugin[3].as_str().unwrap();
        if state == "Unknown" {
            states.remove(name);
        } else {
            states.insert(name.into(), state.into());
        }
    }
    for name in &requested {
        if !states.contains_key(name)
            && available
            && [".esp", ".esm", ".esl"].iter().any(|s| name.ends_with(s))
        {
            states.insert(name.clone(), "Missing".into());
        }
    }
    let archives = py
        .import("creation_lib.mod_manager.installer_archives")?
        .call_method1(
            "capture",
            (
                library, profile_id, &requested, index, &game, available, cancel, progress,
            ),
        )?;
    for item in archives.getattr("states")?.try_iter()? {
        let item = item?;
        states.insert(item.get_item(0)?.extract()?, item.get_item(1)?.extract()?);
    }
    let versions = py
        .import("creation_lib.mod_manager.installer_versions")?
        .call_method1(
            "capture",
            (
                library,
                object(py, &config)?,
                &game,
                version_requests,
                cancel,
            ),
        )?;
    check_cancel(Some(cancel))?;
    let current = library.call_method1("profile_snapshot", (profile_id,))?;
    let current_config = value(&current.get_item(2)?)?;
    if database != self::database(&current)?
        || signature != plugin_inventory_signature(library, profile_id)?
        || initial_identity != identity(library)?
        || providers
            != self::providers(
                library,
                profile_id,
                &current.get_item(1)?,
                &current_config,
                cancel,
            )?
            .0
        || ccc != self::ccc(py, &current_config, &game, cancel)?
    {
        return Err(invalid(
            "Installer profile or provider context changed during preparation. Prepare again.",
        ));
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item("library_root", initial_identity.0)?;
    kwargs.set_item("library_id", initial_identity.1)?;
    kwargs.set_item("profile_id", profile_id)?;
    kwargs.set_item("plugin_signature", signature)?;
    kwargs.set_item("database_signature", database)?;
    kwargs.set_item("file_signature", format!("{:x}", hash.finalize()))?;
    kwargs.set_item("file_count", count)?;
    for (name, value) in [
        ("providers", providers),
        ("ccc", ccc),
        ("plugins", plugins),
        ("requested", json!(requested)),
    ] {
        kwargs.set_item(name, tuple_value(py, &value)?)?;
    }
    kwargs.set_item("archives", archives)?;
    kwargs.set_item("versions", versions)?;
    let context = py
        .import("creation_lib.mod_manager.installer_context")?
        .getattr("Context")?
        .call((), Some(&kwargs))?;
    Ok((context, states).into_pyobject(py)?.into_any().unbind())
}
#[pyfunction]
fn installer_context_scan(
    library: &Bound<'_, PyAny>,
    profile: &str,
    requested: Vec<String>,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
    versions: Vec<String>,
) -> PyResult<Py<PyAny>> {
    scan(library, profile, requested, cancel, progress, versions)
}
#[pyfunction]
fn installer_context_capture(
    library: &Bound<'_, PyAny>,
    profile: &str,
    fomod: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    check_cancel(Some(cancel))?;
    let root = fomod.getattr("root")?;
    let mut requested = BTreeSet::new();
    for node in root.call_method1("iter", ("fileDependency",))?.try_iter()? {
        requested.insert(fold(&relative_path(
            &node?
                .call_method1("get", ("file", ""))?
                .extract::<String>()?,
            false,
        )?));
    }
    let extenders = py
        .import("creation_lib.mod_manager.fomod")?
        .getattr("EXTENDERS")?;
    let mut versions = BTreeSet::new();
    for node in root.call_method0("iter")?.try_iter()? {
        let tag = node?.getattr("tag")?.extract::<String>()?;
        if tag == "gameDependency" || extenders.contains(&tag)? {
            versions.insert(tag);
        }
    }
    if library.hasattr("target_id")? && library.getattr("target_id")?.is_truthy()? {
        versions.extend(["gameDependency".into(), "f4seDependency".into()]);
    }
    let requested = requested.into_iter().collect::<Vec<_>>();
    let versions = versions.into_iter().collect::<Vec<_>>();
    (|| -> PyResult<Py<PyAny>> {
        let first = scan(
            library,
            profile,
            requested.clone(),
            cancel,
            progress,
            versions.clone(),
        )?;
        let second = scan(library, profile, requested, cancel, progress, versions)?;
        if !first.bind(py).eq(second.bind(py))? {
            return Err(invalid(
                "Installer Data changed during preparation. Prepare the package again.",
            ));
        }
        Ok(first)
    })()
    .map_err(|e| {
        if e.is_instance_of::<pyo3::exceptions::PyOSError>(py) {
            invalid("Installer dependency context became unavailable. Prepare the package again.")
        } else {
            e
        }
    })
}
#[pyfunction]
fn installer_context_validate(
    library: &Bound<'_, PyAny>,
    context: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = library.py();
    check_cancel(Some(cancel))?;
    (|| -> PyResult<()> {
        if !context.is_instance(
            &py.import("creation_lib.mod_manager.installer_context")?
                .getattr("Context")?,
        )? || identity(library)?
            != (
                context.getattr("library_root")?.extract()?,
                context.getattr("library_id")?.extract()?,
            )
        {
            return Err(invalid(
                "Installer context belongs to a different library. Prepare the package again.",
            ));
        }
        let versions = context.getattr("versions")?;
        let versions = if versions.is_truthy()? {
            versions.getattr("requested")?.extract::<Vec<String>>()?
        } else {
            Vec::new()
        };
        let current = scan(
            library,
            &context.getattr("profile_id")?.extract::<String>()?,
            context.getattr("requested")?.extract()?,
            cancel,
            py.None().bind(py),
            versions,
        )?;
        if !current.bind(py).get_item(0)?.eq(context)? {
            return Err(invalid(
                "Installer dependency context changed. Prepare and review the package again.",
            ));
        }
        Ok(())
    })()
    .map_err(|e| {
        if e.is_instance_of::<pyo3::exceptions::PyOSError>(py) {
            invalid("Installer dependency context became unavailable. Prepare and review again.")
        } else {
            e
        }
    })
}
#[pyfunction]
fn known_game_files(
    library: &Bound<'_, PyAny>,
    profile_id: &str,
    fomod: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<std::collections::BTreeMap<String, String>> {
    let py = library.py();
    let catalog = py
        .import("creation_lib.mod_manager.plugin_state")?
        .call_method1("inventory", (library, profile_id, cancel))?;
    let mut result = std::collections::BTreeMap::new();
    for plugin in catalog.getattr("plugins")?.try_iter()? {
        let plugin = plugin?;
        if plugin.getattr("path")?.is_truthy()? {
            result.insert(
                plugin.getattr("name")?.extract::<String>()?,
                if plugin.getattr("enabled")?.is_truthy()? {
                    "Active"
                } else {
                    "Inactive"
                }
                .into(),
            );
        }
    }
    let snapshot = library.call_method1("profile_snapshot", (profile_id,))?;
    let config = snapshot.get_item(2)?;
    let empty = pyo3::types::PyDict::new(py);
    let root: String = config
        .call_method1("get", ("game", empty))?
        .call_method1("get", ("gamepath", ""))?
        .extract()?;
    if root.is_empty() {
        return Ok(result);
    }
    let data = std::path::PathBuf::from(root).join("Data");
    crate::reject_links(&data)?;
    if !data.is_dir() {
        return Ok(result);
    }
    let known: std::collections::HashSet<_> =
        result.keys().map(|name| crate::saves::fold(name)).collect();
    for dependency in fomod
        .getattr("root")?
        .call_method1("iter", ("fileDependency",))?
        .try_iter()?
    {
        crate::check_cancel(Some(cancel))?;
        let name = crate::relative_path(
            &dependency?
                .call_method1("get", ("file", ""))?
                .extract::<String>()?,
            false,
        )?;
        if known.contains(&crate::saves::fold(&name)) {
            continue;
        }
        let target = data.join(&name);
        crate::reject_links(&target)?;
        if target.is_file() {
            result.insert(name, "Active".into());
        } else if matches!(
            target
                .extension()
                .and_then(|v| v.to_str())
                .map(crate::saves::fold)
                .as_deref(),
            Some("esp" | "esm" | "esl")
        ) {
            result.insert(name, "Missing".into());
        }
    }
    Ok(result)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(known_game_files, m)?)?;
    m.add_function(wrap_pyfunction!(installer_verify_winners, m)?)?;
    m.add_function(wrap_pyfunction!(installer_context_scan, m)?)?;
    m.add_function(wrap_pyfunction!(installer_context_capture, m)?)?;
    m.add_function(wrap_pyfunction!(installer_context_validate, m)?)?;
    Ok(())
}
