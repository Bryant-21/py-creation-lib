use crate::{
    check_cancel, encoding,
    history_prune::py_path,
    invalid,
    plugin_inventory::{object, value},
    python_io::{with_context, with_database},
    reject_links,
    saves::fold,
};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict, PyTuple},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("creation_lib.mod_manager.f4se_setup")
}
fn pe(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("creation_lib.mod_manager.pe_metadata")
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn signature(py: Python<'_>, path: &Path) -> PyResult<Value> {
    value(
        &py.import("creation_lib.mod_manager.launcher")?
            .call_method1("fingerprint", (py_path(py, path)?,))?,
    )
}
fn dataclass(item: &Bound<'_, PyAny>) -> PyResult<Value> {
    value(
        &item
            .py()
            .import("dataclasses")?
            .call_method1("asdict", (item,))?,
    )
}
fn digest(py: Python<'_>, value: &Value) -> PyResult<String> {
    py.import("creation_lib.mod_manager.profile_backup")?
        .call_method1("digest", (object(py, value)?,))?
        .extract()
}
fn release(py: Python<'_>, build: &Value) -> PyResult<Value> {
    let values = build
        .as_array()
        .ok_or_else(|| invalid("Invalid F4SE build."))?;
    let key = PyTuple::new(
        py,
        values
            .iter()
            .take(3)
            .map(|v| object(py, v))
            .collect::<PyResult<Vec<_>>>()?,
    )?;
    value(
        &module(py)?
            .getattr("CATALOG")?
            .call_method1("get", (key,))?,
    )
}
#[pyfunction]
fn f4se_runtime_name(build: &Bound<'_, PyAny>) -> PyResult<String> {
    let mut parts = Vec::new();
    for part in build.try_iter()?.take(3) {
        parts.push(part?.str()?.to_str()?.to_owned());
    }
    Ok(format!("f4se_{}.dll", parts.join("_")))
}
fn runtime_name(py: Python<'_>, build: &Value) -> PyResult<String> {
    f4se_runtime_name(&object(py, build)?)
}
fn required(py: Python<'_>, build: &Value, release: &Value) -> PyResult<BTreeSet<String>> {
    let mut required = BTreeSet::from(["f4se_loader.exe".to_owned(), runtime_name(py, build)?]);
    if release["steam_loader"].as_bool().unwrap_or(false) {
        required.insert("f4se_steam_loader.dll".into());
    }
    Ok(required)
}
fn machine(py: Python<'_>, path: &Path) -> PyResult<u32> {
    py.import("creation_lib.mod_manager.runtime")?
        .call_method1("pe_machine", (py_path(py, path)?,))?
        .extract()
}
fn version(py: Python<'_>, path: &Path) -> PyResult<Option<Vec<u32>>> {
    pe(py)?
        .call_method1("file_version", (py_path(py, path)?,))?
        .extract()
}
fn core_matches(py: Python<'_>, path: &Path, build: &Value) -> PyResult<bool> {
    let raw = pe(py)?.call_method1("exported_data", (py_path(py, path)?, "F4SECore_Version", 8))?;
    if !raw.is_truthy()? {
        return Ok(true);
    }
    let bytes: Vec<u8> = raw.extract()?;
    let current = pe(py)?
        .call_method1("packed_runtime", (object(py, build)?,))?
        .extract::<u32>()?;
    Ok(bytes.len() == 8
        && u32::from_le_bytes(bytes[..4].try_into().unwrap()) == 1
        && u32::from_le_bytes(bytes[4..8].try_into().unwrap()) == current)
}
fn release_version(release: &Value) -> PyResult<Vec<u32>> {
    text(release, "version")
        .split('.')
        .map(|v| {
            v.parse()
                .map_err(|_| invalid("Invalid F4SE release version."))
        })
        .collect()
}
#[pyfunction]
fn f4se_inspect_folder(game: PathBuf, cancel: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = cancel.py();
    check_cancel(Some(cancel))?;
    let exe = game.join("Fallout4.exe");
    let game_version = version(py, &exe)?;
    let mut result = json!({"build":game_version.clone().unwrap_or_default(),"state":"missing","managed":false,"reason":"Matching F4SE has not been prepared.","game_signature":signature(py,&exe)?});
    if game_version.is_none() {
        result["state"] = json!("unknown");
        result["reason"] = json!(
            "Fallout4.exe has no readable fixed version resource; automatic matching is blocked."
        );
        return Ok(object(py, &result)?.unbind());
    }
    let build = result["build"].clone();
    let release = release(py, &build)?;
    if release.is_null() {
        result["state"] = json!("unsupported");
        result["reason"] = json!(
            "This build has no verified F4SE entry in our bounded catalog. Check the official site; no nearest-version fallback is used."
        );
        return Ok(object(py, &result)?.unbind());
    }
    result
        .as_object_mut()
        .unwrap()
        .extend(release.as_object().unwrap().clone());
    let mut storefront = false;
    for (index, path) in std::fs::read_dir(&game)?.enumerate() {
        if index >= 10000 {
            return Err(invalid(
                "Game-root inventory exceeds its 10,000-entry limit.",
            ));
        }
        let name = fold(&path?.file_name().to_string_lossy());
        if (name.starts_with("goggame-") && name.ends_with(".info"))
            || matches!(name.as_str(), "microsoftgame.config" | ".egstore")
        {
            storefront = true;
        }
    }
    if storefront {
        result["state"] = json!("unsupported");
        result["reason"] = json!(
            "The native launch adapter currently supports Steam FO4 only; detected GOG/Store/Epic metadata needs another runtime adapter."
        );
        return Ok(object(py, &result)?.unbind());
    }
    let names = required(py, &build, &release)?;
    let scripts = game.join("Data").join("Scripts");
    for name in &names {
        reject_links(&game.join(name))?;
    }
    reject_links(&scripts)?;
    if names.iter().any(|name| !game.join(name).is_file()) {
        return Ok(object(py, &result)?.unbind());
    }
    let checked = (|| -> PyResult<()> {
        for name in &names {
            if machine(py, &game.join(name))? != 0x8664 {
                return Err(invalid("Existing F4SE contains a non-x64 component."));
            }
        }
        if !version(py, &game.join("f4se_loader.exe"))?
            .is_some_and(|v| v[..3] == release_version(&release).unwrap_or_default())
        {
            return Err(invalid(
                "Existing F4SE loader version is unknown or does not match the selected build; it is preserved.",
            ));
        }
        if !core_matches(py, &game.join(runtime_name(py, &build)?), &build)? {
            return Err(invalid(
                "Existing F4SE core metadata does not match this runtime.",
            ));
        }
        if !scripts.is_dir() || !scripts.join("F4SE.pex").is_file() {
            return Err(invalid(
                "Existing F4SE script coverage is unavailable; prepare the official package privately.",
            ));
        }
        reject_links(&scripts.join("F4SE.pex"))?;
        Ok(())
    })();
    if let Err(error) = checked {
        if error.is_instance_of::<crate::OperationCancelled>(py)
            || error.is_instance(
                py,
                &py.import("creation_lib.mod_manager.paths")?
                    .getattr("Cancelled")?,
            )
        {
            return Err(error);
        }
        if !error.is_instance_of::<pyo3::exceptions::PyValueError>(py) {
            return Err(error);
        }
        result["state"] = json!("mismatch");
        result["reason"] = json!(error.value(py).str()?.to_str()?);
        return Ok(object(py, &result)?.unbind());
    }
    let mut root_files = Vec::new();
    let mut order = vec!["f4se_loader.exe".to_owned(), runtime_name(py, &build)?];
    if release["steam_loader"] == true {
        order.push("f4se_steam_loader.dll".into());
    }
    for name in order {
        let path = game.join(&name);
        root_files.push(json!([name, path.to_string_lossy(), signature(py, &path)?]));
    }
    result["state"] = json!("ready");
    result["reason"] =
        json!("Existing matching F4SE metadata found; custom files are retained in place.");
    result["root_files"] = json!(root_files);
    result["data"] = json!("");
    Ok(object(py, &result)?.unbind())
}
#[pyfunction]
fn f4se_official(raw: &str) -> PyResult<String> {
    let url =
        url::Url::parse(raw).map_err(|_| invalid("F4SE download left the official HTTPS host."))?;
    if url.scheme() != "https"
        || url.host_str() != Some("f4se.silverlock.org")
        || !matches!(url.port(), None | Some(443))
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid("F4SE download left the official HTTPS host."));
    }
    Ok(raw.to_owned())
}
#[pyfunction]
fn f4se_download(
    url: &str,
    destination: PathBuf,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
    opener: &Bound<'_, PyAny>,
    validator: &Bound<'_, PyAny>,
    md5: &str,
) -> PyResult<()> {
    let py = cancel.py();
    validator.call1((url,))?;
    let urllib = py.import("urllib.request")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item(
        "headers",
        object(py, &json!({"User-Agent":"ModBox21/1.0"}))?,
    )?;
    let request = urllib.getattr("Request")?.call((url,), Some(&kwargs))?;
    let result = (|| -> PyResult<()> {
        let response = if opener.is_truthy()? {
            opener.call1((request,))?
        } else {
            let kwargs = PyDict::new(py);
            kwargs.set_item("timeout", 15)?;
            urllib
                .call_method1(
                    "build_opener",
                    (module(py)?.getattr("_Redirects")?.call0()?,),
                )?
                .call_method("open", (request,), Some(&kwargs))?
        };
        with_context(&response, |response| {
            with_context(
                &py_path(py, &destination)?.call_method1("open", ("xb",))?,
                |stream| {
                    validator.call1((if response.hasattr("geturl")? {
                        response.call_method0("geturl")?
                    } else {
                        url.into_pyobject(py)?.into_any()
                    },))?;
                    let length = response
                        .getattr("headers")?
                        .call_method1("get", ("Content-Length", ""))?
                        .extract::<String>()?;
                    let total = if !length.is_empty() && length.bytes().all(|c| c.is_ascii_digit())
                    {
                        length.parse::<u64>().map_err(|_| {
                            invalid("F4SE archive exceeds the 64 MiB transfer limit.")
                        })?
                    } else {
                        0
                    };
                    if total > 64 * 1024 * 1024 {
                        return Err(invalid("F4SE archive exceeds the 64 MiB transfer limit."));
                    }
                    let mut size = 0;
                    let mut checksum = md5::Md5::new();
                    loop {
                        check_cancel(Some(cancel))?;
                        let chunk: Vec<u8> =
                            response.call_method1("read", (1024 * 1024,))?.extract()?;
                        if chunk.is_empty() {
                            break;
                        }
                        size += chunk.len() as u64;
                        if size > 64 * 1024 * 1024 || (total != 0 && size > total) {
                            return Err(invalid("F4SE archive exceeded its transfer limit."));
                        }
                        stream.call_method1("write", (PyBytes::new(py, &chunk),))?;
                        checksum.update(&chunk);
                        if progress.is_truthy()? {
                            progress.call1((
                                if total == 0 {
                                    None
                                } else {
                                    Some(size as f64 / total as f64)
                                },
                                "Preparing matching F4SE",
                            ))?;
                        }
                    }
                    if size == 0 || (total != 0 && size != total) {
                        return Err(invalid("F4SE transfer was incomplete."));
                    }
                    if !md5.is_empty()
                        && (md5.len() != 32
                            || !md5.bytes().all(|c| c.is_ascii_hexdigit())
                            || format!("{:x}", checksum.finalize()) != md5.to_ascii_lowercase())
                    {
                        return Err(invalid(
                            "F4SE archive failed the publisher's Nexus checksum.",
                        ));
                    }
                    Ok(())
                },
            )
        })
    })();
    result.map_err(|e|if e.is_instance_of::<pyo3::exceptions::PyOSError>(py){invalid("F4SE acquisition failed; check connectivity or choose a cached official archive. Existing files are preserved.")}else{e})
}
fn target<'py>(
    library: &Bound<'py, PyAny>,
    id: &str,
    db: Option<&Bound<'py, PyAny>>,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let module = library
        .py()
        .import("creation_lib.mod_manager.launch_targets")?;
    let records = if let Some(db) = db {
        module.call_method1("records", (library, db))?
    } else {
        module.call_method1("records", (library,))?
    };
    for item in records.try_iter()? {
        let item = item?;
        if item.getattr("id")?.eq(id)? {
            return Ok(Some(item));
        }
    }
    Ok(None)
}
#[pyclass]
struct TransferOpener {
    transfer: Py<PyAny>,
}
#[pymethods]
impl TransferOpener {
    fn __call__(&self, request: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let py = request.py();
        let kwargs = PyDict::new(py);
        kwargs.set_item("api", false)?;
        Ok(self
            .transfer
            .bind(py)
            .call_method("_open", (request,), Some(&kwargs))?
            .unbind())
    }
}
#[pyfunction]
fn f4se_prepare(
    library: &Bound<'_, PyAny>,
    target_id: &str,
    archive: &Bound<'_, PyAny>,
    client: &Bound<'_, PyAny>,
    nxm: &str,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
    opener: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    library.call_method0("_runtime_write_guard")?;
    let target = target(library, target_id, None)?
        .ok_or_else(|| invalid("Select a configured launch target first."))?;
    let before = dataclass(&target)?;
    if before["f4se"]["root_setup"]
        .as_str()
        .is_some_and(|s| !s.is_empty())
    {
        return Err(invalid(
            "Restore this target's game-root setup before replacing its private runtime package.",
        ));
    }
    let expected = digest(py, &before)?;
    let view = py
        .import("creation_lib.mod_manager.launch_targets")?
        .getattr("TargetLibrary")?
        .call1((library, target_id))?;
    let game = PathBuf::from(text(&before, "game_root"));
    let lock = game.join(
        py.import("creation_lib.mod_manager.f4se_root_setup")?
            .getattr("LOCK")?
            .extract::<String>()?,
    );
    reject_links(&lock)?;
    if lock.exists() {
        return Err(invalid(
            "Review retained game-root recovery before preparing another runtime.",
        ));
    }
    let inspected = value(f4se_inspect_folder(game.clone(), cancel)?.bind(py))?;
    let build = inspected["build"].clone();
    let release = release(py, &build)?;
    if release.is_null() || matches!(text(&inspected, "state"), "unknown" | "unsupported") {
        return Err(invalid(text(&inspected, "reason")));
    }
    check_cancel(Some(cancel))?;
    let staging = view.getattr("root")?.extract::<PathBuf>()?.join("f4se");
    reject_links(&staging)?;
    if !staging.exists() {
        std::fs::create_dir(&staging)?;
    }
    for (index, entry) in std::fs::read_dir(&staging)?.enumerate() {
        reject_links(&entry?.path())?;
        if index >= 256 {
            return Err(invalid(
                "Target F4SE retention exceeds 256 folders. Review retained stages before retrying; nothing was removed.",
            ));
        }
    }
    let temporary = staging.join(format!("pending-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&temporary)?;
    let mut final_path = None;
    let mut published = false;
    let mut transfer: Option<Bound<'_, PyAny>> = None;
    let result = (|| -> PyResult<Py<PyAny>> {
        let archive = if archive.is_truthy()? {
            archive.extract::<PathBuf>()?
        } else if release.get("url").is_some() {
            let archive = temporary.join("official.7z");
            f4se_download(
                text(&release, "url"),
                archive.clone(),
                cancel,
                progress,
                opener,
                &module(py)?.getattr("_official")?,
                "",
            )?;
            archive
        } else {
            if !client.is_truthy()? {
                return Err(invalid(
                    "This official release is hosted on Nexus. Connect the session account or choose a cached official archive; free accounts need its browser download link.",
                ));
            }
            let nexus = py.import("creation_lib.mod_manager.nexus")?;
            let id = release["file_id"].as_u64().unwrap_or(0);
            let link = if nxm.is_empty() {
                nexus.getattr("NxmLink")?.call1(("fallout4", 42147, id))?
            } else {
                nexus.call_method1("parse_nxm", (nxm,))?
            };
            if !link.getattr("identity")?.eq(("fallout4", 42147, id))? {
                return Err(invalid(
                    "The browser link is not the exact official F4SE file for this build.",
                ));
            }
            transfer = Some(client.call_method0("for_transfer")?);
            let active = transfer.as_ref().unwrap();
            let resolution = active.call_method1("resolve", (link,))?;
            let metadata = resolution.get_item(0)?;
            let url = resolution.get_item(1)?.extract::<String>()?;
            let filename = py_path(py, &metadata.get_item("file_name")?.extract::<PathBuf>()?)?;
            let archive = temporary.join(format!(
                "official{}",
                filename.getattr("suffix")?.extract::<String>()?
            ));
            let opener = Py::new(
                py,
                TransferOpener {
                    transfer: active.clone().unbind(),
                },
            )?;
            f4se_download(
                &url,
                archive.clone(),
                cancel,
                progress,
                opener.bind(py).as_any(),
                &nexus.getattr("validate_cdn_url")?,
                &metadata
                    .call_method1("get", ("md5", ""))?
                    .extract::<String>()?,
            )?;
            archive
        };
        reject_links(&archive)?;
        let payload = temporary.join("payload");
        std::fs::create_dir(&payload)?;
        let required = required(py, &build, &release)?;
        let package_module = py.import("creation_lib.mod_manager.package")?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("inspect_installer", false)?;
        let package = package_module.getattr("Package")?.call(
            (
                py_path(py, &archive)?,
                cancel,
                module(py)?.getattr("LIMITS")?,
            ),
            Some(&kwargs),
        )?;
        with_context(
            &py.import("contextlib")?
                .getattr("closing")?
                .call1((package,))?,
            |package| {
                let names: Vec<String> = package
                    .getattr("files")?
                    .call_method0("values")?
                    .try_iter()?
                    .map(|v| v?.extract())
                    .collect::<PyResult<_>>()?;
                let loaders: Vec<_> = names
                    .iter()
                    .filter(|name| {
                        Path::new(name)
                            .file_name()
                            .is_some_and(|v| fold(&v.to_string_lossy()) == "f4se_loader.exe")
                    })
                    .collect();
                if loaders.len() != 1 {
                    return Err(invalid(
                        "Official F4SE package must contain exactly one loader.",
                    ));
                }
                let prefix = &loaders[0][..loaders[0].len() - "f4se_loader.exe".len()];
                let count = prefix.chars().count();
                let mut selected = BTreeMap::new();
                for name in &names {
                    if !fold(name).starts_with(&fold(prefix)) {
                        continue;
                    }
                    let relative = name.chars().skip(count).collect::<String>();
                    let folded = fold(&relative);
                    if required.contains(&folded)
                        || (folded.starts_with("data/scripts/") && folded.ends_with(".pex"))
                    {
                        selected.insert(relative, name);
                    }
                }
                let folded: BTreeSet<_> = selected.keys().map(|name| fold(name)).collect();
                if !required.is_subset(&folded) || !folded.contains("data/scripts/f4se.pex") {
                    return Err(invalid(
                        "F4SE package lacks required matching runtime files or scripts.",
                    ));
                }
                for (name, source) in selected {
                    check_cancel(Some(cancel))?;
                    let destination = payload.join(name);
                    std::fs::create_dir_all(destination.parent().unwrap())?;
                    with_context(&package.call_method1("open", (source,))?, |reader| {
                        with_context(
                            &py_path(py, &destination)?.call_method1("open", ("xb",))?,
                            |writer| {
                                py.import("shutil")?
                                    .call_method1("copyfileobj", (reader, writer, 1024 * 1024))?;
                                Ok(())
                            },
                        )
                    })?;
                }
                Ok(())
            },
        )?;
        for name in &required {
            if machine(py, &payload.join(name))? != 0x8664 {
                return Err(invalid("F4SE package contains a non-x64 component."));
            }
        }
        if !version(py, &payload.join("f4se_loader.exe"))?
            .is_some_and(|v| v[..3] == release_version(&release).unwrap_or_default())
        {
            return Err(invalid(
                "F4SE package loader version does not match this game's catalog entry.",
            ));
        }
        if !core_matches(py, &payload.join(runtime_name(py, &build)?), &build)? {
            return Err(invalid(
                "F4SE package core export selects a different game runtime.",
            ));
        }
        let mut manifest = BTreeMap::new();
        for item in py
            .import("creation_lib.mod_manager.paths")?
            .call_method1("tree_files", (py_path(py, &payload)?, cancel))?
            .try_iter()?
        {
            let item = item?;
            let name: String = item.get_item(0)?.extract()?;
            let path: PathBuf = item.get_item(1)?.extract()?;
            manifest.insert(name, format!("{:x}", Sha256::digest(std::fs::read(path)?)));
        }
        let final_root = staging.join(uuid::Uuid::new_v4().simple().to_string());
        final_path = Some(final_root.clone());
        std::fs::rename(payload, &final_root)?;
        let mut root_files = Vec::new();
        for name in required {
            let path = final_root.join(&name);
            root_files.push(json!([name, path.to_string_lossy(), signature(py, &path)?]));
        }
        let result = json!({"build":inspected["build"],"version":release["version"],"state":"ready","managed":true,"game_signature":inspected["game_signature"],"data":final_root.join("Data").to_string_lossy(),"root_files":root_files,"manifest":manifest,"stage":final_root.to_string_lossy(),"reason":"Matching F4SE privately staged. Game-root files are virtual mappings; real boot and script/runtime behavior remain unverified."});
        let kwargs = PyDict::new(py);
        kwargs.set_item(
            "binary",
            final_root
                .join("f4se_loader.exe")
                .to_string_lossy()
                .as_ref(),
        )?;
        kwargs.set_item("f4se", object(py, &result)?)?;
        let updated =
            py.import("dataclasses")?
                .call_method("replace", (&target,), Some(&kwargs))?;
        with_database(&library.getattr("_db")?, |db, native| {
            native
                .connection()?
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(crate::database::error)?;
            let current = target_record(library, target_id, db)?;
            if current
                .as_ref()
                .map(|v| digest(py, &dataclass(v)?))
                .transpose()?
                .as_deref()
                != Some(&expected)
            {
                return Err(invalid("Launch target changed during F4SE preparation."));
            }
            module(py)?.call_method0("_checkpoint")?;
            check_cancel(Some(cancel))?;
            reject_links(&game.join("Fallout4.exe"))?;
            if signature(py, &game.join("Fallout4.exe"))? != inspected["game_signature"] {
                return Err(invalid("Game executable changed during F4SE preparation."));
            }
            library.call_method0("_runtime_write_guard")?;
            native
                .connection()?
                .execute(
                    "UPDATE launch_targets SET record=? WHERE id=?",
                    rusqlite::params![dataclass(&updated)?.to_string(), target_id],
                )
                .map_err(crate::database::error)?;
            Ok(())
        })?;
        published = true;
        Ok(updated.unbind())
    })();
    let forgotten = if let Some(transfer) = transfer {
        transfer.call_method0("forget").map(|_| ())
    } else {
        Ok(())
    };
    for path in std::iter::once(Some(temporary))
        .chain(std::iter::once(if published { None } else { final_path }))
        .flatten()
    {
        reject_links(&path)?;
        if path.starts_with(&staging) && path != staging && path.exists() {
            py.import("shutil")?
                .call_method1("rmtree", (py_path(py, &path)?,))?;
        }
    }
    forgotten?;
    result
}
fn target_record<'py>(
    library: &Bound<'py, PyAny>,
    id: &str,
    db: &Bound<'py, PyAny>,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    target(library, id, Some(db))
}
#[pyfunction]
fn f4se_validate_launch(
    target: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    owner_root: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = target.py();
    let data = value(&target.getattr("f4se")?)?;
    let game = target.getattr("game_root")?.extract::<PathBuf>()?;
    if !data["managed"].as_bool().unwrap_or(false) {
        check_cancel(Some(cancel))?;
        let binary = target.getattr("binary")?.extract::<PathBuf>()?;
        if text(&data, "state") == "missing"
            && fold(&binary.file_name().unwrap_or_default().to_string_lossy()) != "f4se_loader.exe"
        {
            return Err(invalid(
                "This F4SE target has no loader. Select an existing F4SE loader or prepare F4SE first.",
            ));
        }
        if py_path(py, &binary)?
            .getattr("parent")?
            .call_method0("resolve")?
            .ne(py_path(py, &game)?.call_method0("resolve")?)?
        {
            return Err(invalid(
                "The existing loader must belong to its selected game directory.",
            ));
        }
        reject_links(&game.join("Data"))?;
        if !game.join("Data").is_dir() {
            return Err(invalid("The selected game directory has no Data folder."));
        }
        for path in [&binary, &game.join("Fallout4.exe")] {
            reject_links(path)?;
            if machine(py, path)? != 0x8664 {
                return Err(invalid(
                    "The selected installation requires x64 game/loader executables.",
                ));
            }
        }
        let build = version(py, &game.join("Fallout4.exe"))?.unwrap_or_default();
        let loader_version = version(py, &binary)?.unwrap_or_default();
        let existing = json!({"managed":false,"state":"ready","build":build,
            "version":loader_version.iter().take(3).map(u32::to_string).collect::<Vec<_>>().join("."),
            "reason":"Using the selected existing executable; package catalog matching is not required."});
        return Ok(object(py, &existing)?.unbind());
    }
    f4se_validate_ready(target, cancel, owner_root)
}
#[pyfunction]
fn f4se_validate_ready(
    target: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    owner_root: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = target.py();
    let data = value(&target.getattr("f4se")?)?;
    let game = target.getattr("game_root")?.extract::<PathBuf>()?;
    if text(&data, "state") != "ready" {
        return Err(invalid(
            data.get("reason")
                .and_then(Value::as_str)
                .unwrap_or("Prepare matching F4SE before Run."),
        ));
    }
    if signature(py, &game.join("Fallout4.exe"))? != data["game_signature"] {
        return Err(invalid(
            "Target game build changed. Review the folder and prepare matching F4SE again.",
        ));
    }
    let release = release(py, &data["build"])?;
    if release.is_null() || data["version"] != release["version"] {
        return Err(invalid(
            "Target F4SE receipt has no matching catalog identity.",
        ));
    }
    let names = required(py, &data["build"], &release)?;
    let components = data["root_files"]
        .as_array()
        .ok_or_else(|| invalid("Target F4SE receipt lacks its exact required components."))?;
    let present: BTreeSet<_> = components
        .iter()
        .map(|v| v[0].as_str().unwrap_or("").to_owned())
        .collect();
    if present != names || components.len() != names.len() {
        return Err(invalid(
            "Target F4SE receipt lacks its exact required components.",
        ));
    }
    let managed = data["managed"].as_bool().unwrap_or(false);
    let root_route = text(&data, "route") == "game-root";
    let stage = if managed {
        PathBuf::from(text(&data, "stage"))
    } else {
        game.clone()
    };
    let stage_components;
    if managed {
        crate::transactions::transaction_key(
            stage.file_name().and_then(|v| v.to_str()).unwrap_or(""),
        )?;
        if owner_root.is_truthy()?
            && stage.parent() != Some(owner_root.extract::<PathBuf>()?.join("f4se").as_path())
        {
            return Err(invalid(
                "F4SE stage is outside this target's private storage.",
            ));
        }
        let loader = if root_route {
            game.join("f4se_loader.exe")
        } else {
            stage.join("f4se_loader.exe")
        };
        if text(&data, "data") != stage.join("Data").to_string_lossy()
            || target.getattr("binary")?.extract::<String>()? != loader.to_string_lossy()
        {
            return Err(invalid(
                "F4SE script/launcher storage differs from its staged receipt.",
            ));
        }
        if root_route {
            let roots = py.import("creation_lib.mod_manager.f4se_root_setup")?;
            let lock = game.join(roots.getattr("LOCK")?.extract::<String>()?);
            reject_links(&lock)?;
            if lock.exists() {
                return Err(invalid(
                    "Game-root setup/recovery is unfinished. Review root restore before Run.",
                ));
            }
            if !owner_root.is_truthy()? {
                return Err(invalid(
                    "Managed game-root validation requires its owned target context.",
                ));
            }
            let library = py
                .import("creation_lib.mod_manager.library")?
                .getattr("Library")?;
            let owner = py
                .import("builtins")?
                .getattr("object")?
                .getattr("__new__")?
                .call1((&library,))?;
            let root = owner_root
                .extract::<PathBuf>()?
                .parent()
                .and_then(Path::parent)
                .ok_or_else(|| invalid("Invalid F4SE owned target context."))?
                .to_path_buf();
            owner.setattr("root", py_path(py, &root)?)?;
            owner.setattr("database", py_path(py, &root.join("library.sqlite3"))?)?;
            let marker = root.join(library.getattr("MARKER")?.extract::<String>()?);
            reject_links(&marker)?;
            let record = encoding::parse(&std::fs::read_to_string(marker)?)?;
            owner.setattr("library_id", text(&record, "id"))?;
            let receipt = value(
                &roots
                    .call_method1(
                        "_read",
                        (owner, target.getattr("id")?, text(&data, "root_setup")),
                    )?
                    .get_item(1)?,
            )?;
            if text(&receipt, "state") != "complete"
                || digest(py, &dataclass(target)?)?
                    != digest(py, receipt.get("after").unwrap_or(&json!({})))?
            {
                return Err(invalid(
                    "Managed root setup receipt differs; review recovery before Run.",
                ));
            }
            stage_components = game.clone();
        } else {
            stage_components = stage.clone();
        }
    } else {
        stage_components = stage.clone();
    }
    for item in components {
        check_cancel(Some(cancel))?;
        let name = item[0].as_str().unwrap_or("");
        let source = PathBuf::from(item[1].as_str().unwrap_or(""));
        reject_links(&source)?;
        if source.parent() != Some(&stage_components)
            || fold(&source.file_name().unwrap_or_default().to_string_lossy()) != fold(name)
            || signature(py, &source)? != item[2]
            || machine(py, &source)? != 0x8664
        {
            return Err(invalid(
                "Target F4SE components changed; prepare/review them again.",
            ));
        }
        if root_route {
            let checksum = format!("{:x}", Sha256::digest(std::fs::read(&source)?));
            let expected = data["manifest"]
                .as_object()
                .and_then(|v| v.iter().find(|(path, _)| fold(path) == fold(name)))
                .map(|(_, v)| v.as_str().unwrap_or(""));
            if expected != Some(&checksum) {
                return Err(invalid(
                    "Managed game-root component bytes differ from their private package receipt.",
                ));
            }
        }
    }
    if managed {
        let kwargs = PyDict::new(py);
        kwargs.set_item("directory_limit", 4000)?;
        kwargs.set_item("with_info", true)?;
        let tree = py.import("creation_lib.mod_manager.paths")?.call_method(
            "tree_files",
            (py_path(py, &stage)?, cancel),
            Some(&kwargs),
        )?;
        let mut actual = BTreeMap::new();
        for item in tree.try_iter()? {
            let item = item?;
            if actual.len() >= 4000
                || item.get_item(2)?.getattr("st_size")?.extract::<u64>()? > 16 * 1024 * 1024
            {
                return Err(invalid(
                    "Managed F4SE payload exceeds its bounded receipt limits.",
                ));
            }
            let name: String = item.get_item(0)?.extract()?;
            let path: PathBuf = item.get_item(1)?.extract()?;
            actual.insert(name, format!("{:x}", Sha256::digest(std::fs::read(path)?)));
        }
        if json!(actual) != data["manifest"] {
            return Err(invalid("Managed F4SE payload changed after preparation."));
        }
    }
    Ok(target.getattr("f4se")?.unbind())
}
#[pyfunction]
fn f4se_plugin_findings(
    files: &Bound<'_, PyAny>,
    build: Vec<u32>,
    f4se_version: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Vec<String>> {
    let py = files.py();
    let files: Vec<_> = files.try_iter()?.collect::<PyResult<_>>()?;
    let mut available = BTreeSet::new();
    for item in &files {
        available.insert(fold(&item.get_item(0)?.extract::<String>()?));
    }
    let mut findings = Vec::new();
    let mut count = 0;
    for item in files {
        check_cancel(Some(cancel))?;
        let name = item.get_item(0)?.extract::<String>()?;
        let folded = fold(&name);
        if !folded.starts_with("f4se/plugins/") || !folded.ends_with(".dll") {
            continue;
        }
        if count >= 512 {
            return Err(invalid(
                "Native plugin inventory exceeds its 512-DLL limit.",
            ));
        }
        count += 1;
        let source = item.get_item(1)?.extract::<PathBuf>()?;
        if machine(py, &source)? != 0x8664 {
            return Err(invalid(format!("{name}: native plugin is not x64.")));
        }
        let raw = pe(py)?.call_method1(
            "exported_data",
            (py_path(py, &source)?, "F4SEPlugin_Version", 604),
        )?;
        if !raw.is_truthy()? {
            findings.push(format!("{name}: compatibility unknown; no static version declaration (legacy plugins may query at runtime)."));
            continue;
        }
        let bytes: Vec<u8> = raw.extract()?;
        if bytes.len() < 604 {
            return Err(invalid("Native plugin version declaration is truncated."));
        }
        let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let version = word(0);
        let addresses = word(520);
        let structures = word(524);
        let required = word(592);
        if version != 1 {
            findings.push(format!(
                "{name}: compatibility unknown; unsupported declaration version {version}."
            ));
            continue;
        }
        if build.len() < 3 || f4se_version.is_empty() {
            findings.push(format!("{name}: compatibility unknown; selected game/loader version metadata is unavailable."));
            continue;
        }
        let current = pe(py)?
            .call_method1("packed_runtime", (&build,))?
            .extract::<u32>()?;
        let mut listed = BTreeSet::new();
        for i in 0..16 {
            let value = word(528 + i * 4);
            if value == 0 {
                break;
            }
            listed.insert(value);
        }
        let generation = if build[..3] >= [1, 11, 137][..] {
            4
        } else if build[..3] >= [1, 10, 980][..] {
            2
        } else {
            0
        };
        let independent = addresses & (1 | generation) != 0
            && structures & (1 | generation) != 0
            && word(600) == 0;
        if !independent && !listed.contains(&current) {
            return Err(invalid(format!(
                "{name}: declared runtime list does not support {}.",
                build
                    .iter()
                    .take(3)
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(".")
            )));
        }
        if generation != 0 && addresses & generation != 0 {
            let path = format!(
                "f4se/plugins/version-{}-0.bin",
                build
                    .iter()
                    .take(3)
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join("-")
            );
            if !available.contains(&path) {
                return Err(invalid(format!(
                    "{name}: declared Address Library dependency is missing {path} for this target."
                )));
            }
        }
        let f4se: Vec<u32> = f4se_version
            .split('.')
            .map(|v| v.parse().map_err(|_| invalid("Invalid F4SE version.")))
            .collect::<PyResult<_>>()?;
        if required
            > pe(py)?
                .call_method1("packed_runtime", (f4se,))?
                .extract::<u32>()?
        {
            return Err(invalid(format!(
                "{name}: requires a newer F4SE than this target."
            )));
        }
        findings.push(if addresses!=0 || structures!=0{format!("{name}: declares version-independent flags; exact build/address-library behavior remains unverified.")}else{format!("{name}: static runtime declaration includes this build; successful loading remains unverified.")});
    }
    Ok(findings)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(f4se_runtime_name, m)?)?;
    m.add_function(wrap_pyfunction!(f4se_inspect_folder, m)?)?;
    m.add_function(wrap_pyfunction!(f4se_official, m)?)?;
    m.add_function(wrap_pyfunction!(f4se_download, m)?)?;
    m.add_function(wrap_pyfunction!(f4se_prepare, m)?)?;
    m.add_function(wrap_pyfunction!(f4se_validate_ready, m)?)?;
    m.add_function(wrap_pyfunction!(f4se_validate_launch, m)?)?;
    m.add_function(wrap_pyfunction!(f4se_plugin_findings, m)?)?;
    Ok(())
}
