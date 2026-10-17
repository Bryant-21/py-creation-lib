use crate::{
    check_cancel, encoding,
    executables::{identity, text},
    invalid,
};
use pyo3::{prelude::*, types::PyDict};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use unicode_casefold::UnicodeCaseFold;

fn fold(value: &str) -> String {
    value.case_fold().collect()
}
fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
fn path_object<'py>(py: Python<'py>, path: &Path) -> PyResult<Bound<'py, PyAny>> {
    py.import("pathlib")?.getattr("Path")?.call1((path,))
}
fn resolve(py: Python<'_>, path: &Path) -> PyResult<PathBuf> {
    path_object(py, path)?.call_method0("resolve")?.extract()
}
fn read(path: &Path, reader: &Bound<'_, PyAny>) -> PyResult<String> {
    reader.call1((path_object(reader.py(), path)?,))?.extract()
}

#[pyfunction]
pub fn mo2_qt_string(raw: &str) -> String {
    let mut raw = raw.trim().trim_matches('"').to_string();
    if raw.starts_with("@ByteArray(") && raw.ends_with(')') {
        let chars: Vec<char> = raw[11..raw.len() - 1].chars().collect();
        let mut decoded = String::new();
        let mut index = 0;
        while index < chars.len() {
            if chars[index] == '\\'
                && chars.get(index + 1) == Some(&'x')
                && chars.get(index + 2).is_some_and(|c| c.is_ascii_hexdigit())
            {
                let mut value = chars[index + 2].to_digit(16).unwrap();
                index += 3;
                if chars.get(index).is_some_and(|c| c.is_ascii_hexdigit()) {
                    value = value * 16 + chars[index].to_digit(16).unwrap();
                    index += 1;
                }
                decoded.push(char::from_u32(value).unwrap());
            } else {
                decoded.push(chars[index]);
                index += 1;
            }
        }
        raw = if decoded.chars().all(|c| c as u32 <= 255) {
            String::from_utf8(decoded.chars().map(|c| c as u8).collect()).unwrap_or(decoded)
        } else {
            decoded
        };
    }
    raw.replace("\\\\", "\\")
}

#[pyfunction]
fn mo2_read_ini<'py>(
    path: &Bound<'py, PyAny>,
    reader: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = path.py();
    let module = py.import("configparser")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("interpolation", py.None())?;
    kwargs.set_item("strict", false)?;
    let parser = module.getattr("ConfigParser")?.call((), Some(&kwargs))?;
    crate::reject_links(&path.extract::<PathBuf>()?)?;
    if path.call_method0("is_file")?.is_truthy()? {
        if let Err(error) = parser.call_method1("read_string", (reader.call1((path,))?,)) {
            if error.matches(py, module.getattr("Error")?)? {
                return Err(invalid(format!(
                    "Cannot parse MO2 INI {path}: {}",
                    error.value(py).str()?
                )));
            }
            return Err(error);
        }
    }
    Ok(parser)
}

pub fn section(config: &Bound<'_, PyAny>, name: &str) -> PyResult<BTreeMap<String, String>> {
    if !config.call_method1("has_section", (name,))?.is_truthy()? {
        return Ok(BTreeMap::new());
    }
    config
        .get_item(name)?
        .call_method0("items")?
        .try_iter()?
        .map(|v| {
            let v = v?;
            Ok((v.get_item(0)?.extract()?, v.get_item(1)?.extract()?))
        })
        .collect()
}

fn configuration(
    config: &Bound<'_, PyAny>,
    root: &Path,
    base: &Path,
    mods: &Path,
    profiles: &Path,
) -> PyResult<Value> {
    let general = section(config, "General")?;
    let settings = section(config, "Settings")?;
    let mut paths = json!({"instance":root.to_string_lossy(),"base_directory":base.to_string_lossy(),"mod_directory":mods.to_string_lossy(),"profiles_directory":profiles.to_string_lossy()});
    for key in [
        "download_directory",
        "overwrite_directory",
        "cache_directory",
    ] {
        if let Some(value) = settings.get(key) {
            paths[key] = mo2_qt_string(value)
                .replace("%BASE_DIR%", &base.to_string_lossy())
                .into();
        }
    }
    let mut game = json!({});
    for key in ["gamename", "gamepath", "game_edition", "selected_profile"] {
        if let Some(value) = general.get(key) {
            game[key] = mo2_qt_string(value).into();
        }
    }
    let mut preferences = json!({});
    for key in [
        "enable_mods_after_install",
        "skip_file_suffixes",
        "skip_directories",
        "archive_parsing_experimental",
        "force_enable_core_files",
    ] {
        if let Some(value) = settings.get(key) {
            preferences[key] = mo2_qt_string(value).into();
        }
    }
    let mut executables = BTreeMap::<u32, Value>::new();
    for (name, raw) in section(config, "customExecutables")? {
        let Some((index, key)) = name.split_once(['\\', '/']) else {
            continue;
        };
        if index.is_empty()
            || index.len() > 4
            || index.starts_with('0')
            || !index.bytes().all(|v| v.is_ascii_digit())
            || ![
                "title",
                "binary",
                "arguments",
                "workingdirectory",
                "hide",
                "toolbar",
                "steamappid",
            ]
            .contains(&key)
        {
            continue;
        }
        let mut value = mo2_qt_string(&raw);
        if !config
            .py()
            .import("re")?
            .call_method1(
                "search",
                ("(?i)(api[_-]?key|password|access[_-]?token|nxm://)", &value),
            )?
            .is_none()
        {
            value = "[credential-bearing command field omitted]".into();
        }
        executables
            .entry(index.parse().unwrap())
            .or_insert_with(|| json!({}))[key] = value.into();
    }
    Ok(
        json!({"game":game,"paths":paths,"preferences":preferences,"executables":executables.into_values().collect::<Vec<_>>(),"executable_launching":false}),
    )
}

#[pyfunction]
fn mo2_configuration(
    config: &Bound<'_, PyAny>,
    root: PathBuf,
    base: PathBuf,
    mods: PathBuf,
    profiles: PathBuf,
) -> PyResult<String> {
    Ok(configuration(config, &root, &base, &mods, &profiles)?.to_string())
}

#[pyfunction]
fn mo2_ordinary_source(py: Python<'_>, path: PathBuf) -> PyResult<PathBuf> {
    let path = path_object(py, &path)?
        .call_method0("absolute")?
        .extract::<PathBuf>()?;
    crate::reject_links(&path)?;
    if !path.exists() {
        return Err(invalid(format!(
            "Approved source folder is missing: {}",
            path.display()
        )));
    }
    if !path.is_dir() {
        return Err(invalid(format!(
            "Approved source path is not a directory: {}",
            path.display()
        )));
    }
    resolve(py, &path)
}

fn metadata(
    folder: Option<&PathBuf>,
    config: &Bound<'_, PyAny>,
    categories: &BTreeMap<String, String>,
    reader: &Bound<'_, PyAny>,
) -> PyResult<Value> {
    let general = if let Some(folder) = folder {
        section(
            &mo2_read_ini(&path_object(reader.py(), &folder.join("meta.ini"))?, reader)?,
            "General",
        )?
    } else {
        BTreeMap::new()
    };
    let get = |name: &str| general.get(name).map(String::as_str).unwrap_or_default();
    let identifier = |key: &str| {
        let value = get(key);
        if !value.is_empty()
            && value.len() <= 12
            && !value.starts_with('0')
            && value.bytes().all(|v| v.is_ascii_digit())
        {
            value.parse::<u64>().unwrap()
        } else {
            0
        }
    };
    let mut seen = BTreeSet::new();
    let mut values = Vec::new();
    for raw in get("category")
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "0")
    {
        let value = categories.get(raw).map(String::as_str).unwrap_or(raw);
        if seen.insert(value) {
            values.push(value);
        }
    }
    if values.len() > 32 || values.iter().any(|v| v.chars().count() > 200) {
        return Err(invalid(
            "Mod metadata exceeds its 32-category assignment limit.",
        ));
    }
    let instance = section(config, "General")?;
    let game = general
        .get("gamename")
        .or_else(|| instance.get("gamename"))
        .map(String::as_str)
        .unwrap_or_default();
    Ok(
        json!({"version":get("version"),"category":values.first().copied().unwrap_or_default(),"notes":get("notes"),"nexus_game":crate::nexus::nexus_game_domain(game),"nexus_mod_id":identifier("modid"),"nexus_file_id":identifier("fileid"),"tags":get("tags"),"categories":values}),
    )
}

fn linked(
    folder: &Path,
    mappings: &Value,
    local_probe: &Bound<'_, PyAny>,
    ordinary: &Bound<'_, PyAny>,
) -> PyResult<Value> {
    let py = local_probe.py();
    let mut target = String::new();
    let mut available = false;
    let mut status = "Cannot read link target".to_string();
    let result = (|| -> PyResult<()> {
        let mut raw = py
            .import("os")?
            .call_method1("readlink", (path_object(py, folder)?,))?
            .extract::<String>()?;
        if raw.starts_with("\\\\?\\UNC\\") {
            raw = format!("\\\\{}", &raw[8..]);
        } else if raw.starts_with("\\\\?\\") {
            raw = raw[4..].into();
        }
        let value = PathBuf::from(raw);
        let value = if value.is_absolute() {
            value
        } else {
            folder.parent().unwrap().join(value)
        };
        target = path_object(py, &value)?
            .call_method0("absolute")?
            .str()?
            .to_string();
        if target.starts_with("\\\\") || !local_probe.call1((&target,))?.is_truthy()? {
            status = "Network target requires an explicit source mapping".into();
        } else {
            match ordinary.call1((&target,)) {
                Ok(_) => {
                    available = true;
                    status = "Source approval required".into();
                }
                Err(error)
                    if error.matches(
                        py,
                        py.import("creation_lib.mod_manager.paths")?
                            .getattr("ManagerError")?,
                    )? =>
                {
                    status = error.value(py).str()?.to_string();
                    if status.starts_with("Approved source folder is missing:") {
                        status = "Missing target".into();
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        if error.is_instance_of::<pyo3::exceptions::PyOSError>(py) {
            status = format!(
                "Cannot read link target: {}",
                error.value(py).getattr("strerror")?.str()?
            );
        } else {
            return Err(error);
        }
    }
    let alias = path_object(py, folder)?
        .call_method0("absolute")?
        .str()?
        .to_string();
    let source = if let Some(mapped) = mappings
        .get(fold(&alias))
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
    {
        ordinary.call1((mapped,))?.str()?.to_string()
    } else {
        String::new()
    };
    if !source.is_empty() {
        status = "Approved read-only source mapping".into();
    }
    Ok(
        json!({"name":name(folder),"alias":alias,"target":target,"source":source,"status":status,"can_use_target":available}),
    )
}

fn setting(py: Python<'_>, raw: &str, base: &Path, root: &Path) -> PyResult<PathBuf> {
    let mut raw = mo2_qt_string(raw).replace('\\', "/");
    let replacement = base.to_string_lossy().replace('\\', "/");
    let mut cursor = 0;
    while let Some(offset) = raw[cursor..].to_ascii_lowercase().find("%base_dir%") {
        let index = cursor + offset;
        raw.replace_range(index..index + 10, &replacement);
        cursor = index + replacement.len();
    }
    if raw.contains('%') || raw.starts_with('@') {
        return Err(invalid(format!("Unresolved MO2 path setting: {raw}")));
    }
    let path = PathBuf::from(raw);
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    crate::reject_links(&path)?;
    resolve(py, &path)
}

#[pyfunction]
fn mo2_inspect(
    path: PathBuf,
    cancel: &Bound<'_, PyAny>,
    mappings: &str,
    reader: &Bound<'_, PyAny>,
    local_probe: &Bound<'_, PyAny>,
    ordinary: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let py = reader.py();
    let chosen = path_object(py, &path)?
        .call_method0("absolute")?
        .extract::<PathBuf>()?;
    crate::reject_links(&chosen)?;
    if !chosen.exists() {
        return Err(invalid(format!(
            "MO2 instance or profile path does not exist: {}",
            chosen.display()
        )));
    }
    let single = chosen.is_dir() && chosen.join("modlist.txt").is_file();
    let root = if single {
        let parent = chosen.parent().unwrap_or(&chosen);
        parent.parent().unwrap_or(parent).to_owned()
    } else if chosen.is_file() {
        chosen.parent().unwrap().to_owned()
    } else {
        chosen.clone()
    };
    let ini_path = if chosen.is_file() && !single {
        chosen.clone()
    } else {
        root.join("ModOrganizer.ini")
    };
    let config = mo2_read_ini(&path_object(py, &ini_path)?, reader)?;
    let settings = section(&config, "Settings")?;
    let get = |key: &str, fallback: &str| {
        settings
            .get(key)
            .cloned()
            .unwrap_or_else(|| fallback.into())
    };
    let base = setting(
        py,
        &get("base_directory", &root.to_string_lossy()),
        &root,
        &root,
    )?;
    let mods_root = setting(py, &get("mod_directory", "%BASE_DIR%/mods"), &base, &root)?;
    let profiles_root = setting(
        py,
        &get("profiles_directory", "%BASE_DIR%/profiles"),
        &base,
        &root,
    )?;
    if mods_root.exists() && !mods_root.is_dir() {
        return Err(invalid(format!(
            "MO2 mods path is not a directory: {}",
            mods_root.display()
        )));
    }
    if !single && !profiles_root.is_dir() {
        return Err(invalid(format!(
            "MO2 profiles directory is missing: {}",
            profiles_root.display()
        )));
    }
    let mut categories = BTreeMap::new();
    let category_file = root.join("categories.dat");
    crate::reject_links(&category_file)?;
    if category_file.is_file() {
        for line in read(&category_file, reader)?.lines() {
            let parts: Vec<_> = line.split('|').collect();
            if parts.len() >= 2 {
                categories.insert(parts[0].into(), parts[1].into());
            }
        }
    }
    let mut warnings = Vec::new();
    if !mods_root.is_dir() {
        warnings.push(format!("MO2 mods directory is missing: {}. Profiles can be imported as metadata; enabled missing content blocks staging.",mods_root.display()));
    }
    let mut mappings = encoding::parse(mappings)?;
    let mut normalized = json!({});
    for (alias, target) in mappings
        .as_object()
        .ok_or_else(|| invalid("Invalid source mappings."))?
    {
        normalized[fold(
            &path_object(py, Path::new(alias))?
                .call_method0("absolute")?
                .str()?
                .to_string(),
        )] = target.clone();
    }
    mappings = normalized;
    let mut folders = BTreeMap::<String, PathBuf>::new();
    let mut unsupported = BTreeSet::new();
    let mut links = Vec::new();
    if mods_root.is_dir() {
        for entry in std::fs::read_dir(&mods_root)? {
            check_cancel(Some(cancel))?;
            let folder = entry?.path();
            let info = std::fs::symlink_metadata(&folder)?;
            let mut link = info.file_type().is_symlink();
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                link |= info.file_attributes() & 0x400 != 0;
            }
            let actual = name(&folder);
            let key = fold(&actual);
            if link {
                let linked = linked(&folder, &mappings, local_probe, ordinary)?;
                if !text(&linked, "source").is_empty() {
                    folders.insert(key, PathBuf::from(text(&linked, "source")));
                    warnings.push(format!("Using an explicitly approved read-only source for {actual}: {}. The original link is unchanged.",text(&linked,"source")));
                } else {
                    unsupported.insert(key);
                    warnings.push(format!("Skipped linked or reparse-point mod folder: {actual}. {}; approve a source mapping to make its files available.",text(&linked,"status")));
                }
                links.push(linked);
                continue;
            }
            if let Err(error) = crate::reject_links(&folder) {
                if !error.is_instance_of::<pyo3::exceptions::PyValueError>(py) {
                    return Err(error);
                }
                unsupported.insert(key);
                warnings.push(format!("Skipped linked or reparse-point mod folder: {actual}. Its content was not read; profile entries are retained as unavailable metadata."));
                continue;
            }
            if folder.is_dir() && folders.insert(key, folder).is_some() {
                return Err(invalid(format!(
                    "Ambiguous MO2 mod directory name: {actual}"
                )));
            }
        }
    }
    let mut directories = if single {
        vec![chosen]
    } else {
        std::fs::read_dir(&profiles_root)?
            .map(|v| v.map(|v| v.path()))
            .collect::<std::io::Result<Vec<_>>>()?
    };
    directories.sort_by_key(|v| fold(&name(v)));
    let mut profiles = Vec::new();
    let mut cache = BTreeMap::<String, Value>::new();
    for directory in directories {
        check_cancel(Some(cancel))?;
        let profile_name = name(&directory);
        if let Err(error) = crate::reject_links(&directory) {
            if !error.is_instance_of::<pyo3::exceptions::PyValueError>(py) {
                return Err(error);
            }
            warnings.push(format!(
                "Skipped linked or reparse-point profile folder: {profile_name}."
            ));
            continue;
        }
        if !directory.is_dir() {
            continue;
        }
        let modlist = directory.join("modlist.txt");
        crate::reject_links(&modlist)?;
        if !modlist.is_file() {
            warnings.push(format!(
                "Skipped profile folder without modlist.txt: {profile_name}."
            ));
            continue;
        }
        let raw = read(&modlist, reader)?;
        let lines: Vec<_> = raw
            .lines()
            .map(str::trim)
            .filter(|v| !v.is_empty() && !v.starts_with('#'))
            .collect();
        let mut mods = Vec::new();
        let mut seen = BTreeSet::new();
        let mut group = String::new();
        for line in lines.into_iter().rev() {
            check_cancel(Some(cancel))?;
            let first = line.as_bytes()[0];
            let marked = matches!(first, b'+' | b'-' | b'*');
            let marker = if marked { first } else { b'+' };
            let name = if marked { line[1..].trim() } else { line };
            if name.is_empty() {
                warnings.push(format!("{profile_name}: ignored an empty mod name."));
                continue;
            }
            if marker != b'*' {
                crate::relative_path(name, false)?;
                if name.contains(['\\', '/']) {
                    return Err(invalid(format!(
                        "MO2 mod name must be one directory component: {name}"
                    )));
                }
            }
            let key = fold(name);
            if !seen.insert(key.clone()) {
                warnings.push(format!(
                    "{profile_name}: duplicate mod entry {name} ignored."
                ));
                continue;
            }
            let separator = marker != b'*' && key.ends_with("_separator");
            let kind = if separator {
                "separator"
            } else if marker == b'*' {
                "foreign"
            } else {
                "external"
            };
            let folder = if kind == "foreign" {
                None
            } else {
                folders.get(&key)
            };
            if separator {
                group = name[..name.len() - 10].into();
            }
            let source = if !separator && kind != "foreign" {
                folder
                    .map(|v| v.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            if source.is_empty() && kind == "external" {
                let reason = if unsupported.contains(&key) {
                    "unsupported linked mod folder"
                } else {
                    "missing mod folder"
                };
                warnings.push(format!("{profile_name}: {reason} {name}; disable this entry before staging or use an ordinary source folder."));
            }
            if kind == "foreign" {
                warnings.push(format!(
                    "{profile_name}: unmanaged game content {name} is retained as metadata only."
                ));
            }
            if !cache.contains_key(&key) {
                cache.insert(key.clone(), metadata(folder, &config, &categories, reader)?);
            }
            let mut row = cache[&key].clone();
            row["id"] = identity(&if kind == "external" {
                format!("{}/{name}", mods_root.display())
            } else {
                format!("{}/{kind}/{name}", root.display())
            })
            .into();
            row["name"] = name.into();
            row["source"] = source.into();
            row["kind"] = kind.into();
            row["enabled"] = (marker != b'-' && !separator).into();
            row["group"] = group.clone().into();
            mods.push(row);
        }
        let mut files = json!({"modlist.txt":raw});
        for entry in std::fs::read_dir(&directory)? {
            check_cancel(Some(cancel))?;
            let file = entry?.path();
            let actual = name(&file);
            let lower = fold(&actual);
            if [
                "plugins.txt",
                "loadorder.txt",
                "lockedorder.txt",
                "archives.txt",
                "settings.ini",
                "initweaks.ini",
            ]
            .contains(&lower.as_str())
                || file
                    .extension()
                    .is_some_and(|v| fold(&v.to_string_lossy()) == "ini")
            {
                crate::reject_links(&file)?;
                if file.is_file() {
                    files[actual] = read(&file, reader)?.into();
                }
            }
        }
        let source = resolve(py, &directory)?.to_string_lossy().into_owned();
        profiles.push(json!({"id":identity(&source),"name":profile_name,"source":source,"mods":mods,"files":files}));
    }
    if profiles.is_empty() {
        return Err(invalid(
            "No MO2 profiles containing modlist.txt were found. Choose the instance INI, instance folder, or profile folder.",
        ));
    }
    let overwrite = setting(
        py,
        &get("overwrite_directory", "%BASE_DIR%/overwrite"),
        &base,
        &root,
    )?;
    warnings.push("MO2 overwrite content is not imported or scanned; make it a named mod in MO2 first if needed.".into());
    warnings.push("Executables, downloads, saves, game files, and MO2 plugins are not imported. Source mod folders remain read-only references.".into());
    let mut configuration = configuration(&config, &root, &base, &mods_root, &profiles_root)?;
    let mut inventory = Vec::new();
    for key in folders
        .keys()
        .cloned()
        .chain(unsupported)
        .collect::<BTreeSet<_>>()
    {
        check_cancel(Some(cancel))?;
        let folder = folders.get(&key);
        let link = links.iter().find(|v| fold(text(v, "name")) == key);
        let actual = if let Some(link) = link {
            text(link, "name").into()
        } else {
            folder.map(|v| name(v)).unwrap_or_else(|| key.clone())
        };
        crate::relative_path(&actual, false)?;
        let separator = key.ends_with("_separator");
        let mut row = cache
            .get(&key)
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| metadata(folder, &config, &categories, reader))?;
        row["id"] = identity(&if separator {
            format!("{}/separator/{actual}", root.display())
        } else {
            format!("{}/{actual}", mods_root.display())
        })
        .into();
        row["name"] = actual.into();
        row["source"] = if separator {
            json!("")
        } else {
            json!(
                folder
                    .map(|v| v.to_string_lossy().into_owned())
                    .unwrap_or_default()
            )
        };
        row["kind"] = if separator { "separator" } else { "external" }.into();
        row["enabled"] = false.into();
        row["group"] = "".into();
        inventory.push(row);
    }
    configuration["paths"]["overwrite_directory"] = overwrite.to_string_lossy().into_owned().into();
    configuration["paths"]["download_directory"] = setting(
        py,
        &get("download_directory", "%BASE_DIR%/downloads"),
        &base,
        &root,
    )?
    .to_string_lossy()
    .into_owned()
    .into();
    let mut sources = json!({});
    for link in &links {
        if !text(link, "source").is_empty() {
            sources[text(link, "alias")] = link["source"].clone();
        }
    }
    configuration["source_mappings"] = sources;
    warnings.push("Instance paths and executable definitions are retained for review; imported commands are never launched. Authentication, Qt geometry, MO2 plugins, and VFS settings are omitted.".into());
    Ok(json!({"root":resolve(py,&root)?.to_string_lossy(),"mods_root":mods_root.to_string_lossy(),"profiles_root":profiles_root.to_string_lossy(),"game":mo2_qt_string(section(&config,"General")?.get("gamename").map(String::as_str).unwrap_or_default()),"profiles":profiles,"warnings":warnings,"configuration":configuration,"linked_sources":links,"inventory":inventory}).to_string())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(mo2_qt_string, m)?)?;
    m.add_function(wrap_pyfunction!(mo2_read_ini, m)?)?;
    m.add_function(wrap_pyfunction!(mo2_configuration, m)?)?;
    m.add_function(wrap_pyfunction!(mo2_ordinary_source, m)?)?;
    m.add_function(wrap_pyfunction!(mo2_inspect, m)?)?;
    Ok(())
}
