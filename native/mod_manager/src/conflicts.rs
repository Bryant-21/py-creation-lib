use crate::{
    check_cancel, history_prune::py_path, invalid, package, reject_links, relative_path,
    saves::fold,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
fn attr(v: &Bound<'_, PyAny>, name: &str) -> PyResult<String> {
    v.getattr(name)?.extract()
}
#[pyfunction]
fn conflict_providers(entry: &Bound<'_, PyAny>, active: bool) -> PyResult<Py<PyAny>> {
    let py = entry.py();
    let mut result = Vec::new();
    for item in entry.getattr("providers")?.try_iter()? {
        let item = item?;
        if item.getattr("enabled")?.is_truthy()? && !item.getattr("hidden")?.is_truthy()? {
            result.push(item);
        }
    }
    if active {
        Ok(PyTuple::new(py, result)?.into_any().unbind())
    } else {
        Ok(result.pop().map(Bound::unbind).unwrap_or_else(|| py.None()))
    }
}
#[pyfunction]
fn conflict_selected(index: &Bound<'_, PyAny>, path: &str) -> PyResult<Py<PyAny>> {
    if index.getattr("_visible")?.contains(path)? {
        Ok(index
            .getattr("_by_path")?
            .call_method1("get", (path,))?
            .unbind())
    } else {
        Ok(index.py().None())
    }
}
#[pyfunction]
fn conflict_filter(
    index: &Bound<'_, PyAny>,
    query: &str,
    mode: &str,
    mod_id: &str,
) -> PyResult<Py<PyAny>> {
    let py = index.py();
    let query_folded = fold(query);
    let key = (&query_folded, mode, mod_id).into_pyobject(py)?;
    if index.getattr("_key")?.eq(&key)? {
        return Ok(index.getattr("_filtered")?.unbind());
    }
    if ![
        "all",
        "conflicts",
        "winning",
        "losing",
        "uncontested",
        "inactive",
        "hidden",
    ]
    .contains(&mode)
    {
        return Err(invalid("Unknown file inspection filter."));
    }
    let mut result = Vec::new();
    let mut visible = BTreeSet::new();
    for entry in index.getattr("files")?.try_iter()? {
        let entry = entry?;
        let providers = entry
            .getattr("providers")?
            .try_iter()?
            .collect::<PyResult<Vec<_>>>()?;
        let selected = providers
            .iter()
            .filter_map(|p| match attr(p, "mod_id") {
                Ok(id) if mod_id.is_empty() || id == mod_id => Some(Ok(p)),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            })
            .collect::<PyResult<Vec<_>>>()?;
        if selected.is_empty() {
            continue;
        }
        let winner = entry.getattr("winner")?;
        let active = entry.getattr("active")?.len()?;
        let mut wins = false;
        let mut loses = false;
        let mut hidden = false;
        for item in selected {
            let is_hidden = item.getattr("hidden")?.is_truthy()?;
            hidden |= is_hidden;
            wins |= item.is(&winner);
            loses |= item.getattr("enabled")?.is_truthy()? && !is_hidden && !item.is(&winner);
        }
        let matches = match mode {
            "all" => true,
            "conflicts" => active > 1,
            "winning" => !winner.is_none() && wins,
            "losing" => !winner.is_none() && loses,
            "uncontested" => active == 1,
            "inactive" => winner.is_none(),
            _ => hidden,
        };
        let path = attr(&entry, "path")?;
        let mut label = path.clone();
        for p in &providers {
            label.push(' ');
            label.push_str(&attr(p, "name")?);
        }
        if matches && (query.is_empty() || fold(&label).contains(&query_folded)) {
            visible.insert(path);
            result.push(entry);
        }
    }
    let result = PyList::new(py, result)?;
    index.setattr("_key", key)?;
    index.setattr("_filtered", &result)?;
    index.setattr("_visible", visible)?;
    Ok(result.into_any().unbind())
}
#[pyfunction]
pub fn conflict_rows_signature(rows: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = rows.py();
    let mut result = Vec::new();
    for row in rows.try_iter()? {
        let row = row?;
        result.push(
            (
                row.getattr("id")?,
                row.getattr("source")?,
                row.getattr("kind")?,
                row.getattr("priority")?,
                row.getattr("enabled")?,
            )
                .into_pyobject(py)?,
        );
    }
    Ok(PyTuple::new(py, result)?.into_any().unbind())
}
struct Provider {
    id: String,
    name: String,
    priority: i64,
    enabled: bool,
    hidden: bool,
    path: PathBuf,
    size: u64,
    modified: i128,
}
#[pyfunction]
fn conflict_build_index(
    library: &Bound<'_, PyAny>,
    profile_id: &str,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
    include_disabled: bool,
    include_context: bool,
    build_tree: bool,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let api = py.import("creation_lib.mod_manager.conflicts")?;
    let snapshot = library.call_method1("profile_snapshot", (profile_id,))?;
    let profile = snapshot.get_item(0)?;
    let rows = snapshot.get_item(1)?;
    let config = snapshot.get_item(2)?;
    let mut sources = rows.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    let game: String = config
        .call_method1("get", ("game", PyDict::new(py)))?
        .call_method1("get", ("gamepath", ""))?
        .extract()?;
    let mut game_available = false;
    let local = py
        .import("creation_lib.mod_manager.mo2")?
        .getattr("local_mod_target")?;
    if !game.is_empty() && include_context {
        let root = PathBuf::from(&game);
        if !root.is_absolute() || !local.call1((py_path(py, &root)?,))?.is_truthy()? {
            return Err(invalid(
                "Data game directory must be an absolute path on a local drive.",
            ));
        }
        reject_links(&root)?;
        let data = root.join("Data");
        if data.is_dir() {
            sources.insert(
                0,
                api.getattr("DataSource")?.call1((
                    "game",
                    "Game Data",
                    data.to_string_lossy().as_ref(),
                    "game",
                    -1,
                    true,
                ))?,
            );
            game_available = true;
        }
    }
    let overwrite = library
        .getattr("root")?
        .extract::<PathBuf>()?
        .join("runtime")
        .join("profiles")
        .join(crate::transactions::transaction_key(profile_id)?)
        .join("overwrite");
    reject_links(&overwrite)?;
    if include_context && overwrite.is_dir() {
        let priority = rows
            .try_iter()?
            .map(|r| r?.getattr("priority")?.extract::<i64>())
            .collect::<PyResult<Vec<_>>>()?
            .into_iter()
            .max()
            .unwrap_or(0)
            + 1;
        sources.push(api.getattr("DataSource")?.call1((
            "overwrite",
            "Runtime overwrite",
            overwrite.to_string_lossy().as_ref(),
            "runtime",
            priority,
            true,
        ))?);
    }
    if sources.len() > 100_000 {
        return Err(invalid(
            "Data inventory exceeds its 100,000-provider limit.",
        ));
    }
    let mut providers = BTreeMap::<String, Vec<Provider>>::new();
    let mut display = BTreeMap::new();
    let mut issues = Vec::new();
    let mut count = 0;
    let mut unavailable = Vec::new();
    if !game_available {
        for name in library
            .call_method1("unmanaged_names", (profile_id,))?
            .try_iter()?
        {
            issues.push(format!(
                "Unmanaged game content is not indexed: {}",
                name?.extract::<String>()?
            ));
        }
    }
    for (index, source) in sources.iter().enumerate() {
        check_cancel(Some(cancel))?;
        let name = attr(source, "name")?;
        let id = attr(source, "id")?;
        let kind = attr(source, "kind")?;
        let enabled = source.getattr("enabled")?.is_truthy()?;
        if !progress.is_none() {
            progress.call1((
                index as f64 / sources.len().max(1) as f64,
                format!("Indexing {name}"),
            ))?;
        }
        if kind == "separator" || (!enabled && !include_disabled) {
            continue;
        }
        if kind == "foreign" {
            issues.push(format!("Unmanaged game content is not indexed: {name}"));
            continue;
        }
        let path = attr(source, "source")?;
        if path.is_empty() {
            issues.push(format!("Unavailable imported source: {name}"));
            unavailable.push(id);
            continue;
        }
        let root = PathBuf::from(path);
        let inspected = (|| -> PyResult<package::PackageIndex> {
            if !root.is_absolute() || !local.call1((py_path(py, &root)?,))?.is_truthy()? {
                return Err(invalid(
                    "Data providers require absolute ordinary local folders.",
                ));
            }
            reject_links(&root)?;
            if !root.is_dir() {
                return Err(invalid("The source is not an ordinary mod folder."));
            }
            package::directory(py, root.clone(), Some(cancel))
        })();
        let package = match inspected {
            Ok(v) => v,
            Err(e) => {
                check_cancel(Some(cancel))?;
                let paths = py.import("creation_lib.mod_manager.paths")?;
                if e.is_instance(py, &paths.getattr("Cancelled")?) {
                    return Err(e);
                }
                if e.is_instance_of::<pyo3::exceptions::PyOSError>(py)
                    || e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
                    || e.is_instance(py, &paths.getattr("ManagerError")?)
                {
                    issues.push(format!("Unindexed source {name}: {}", e.value(py).str()?));
                    unavailable.push(id);
                    continue;
                }
                return Err(e);
            }
        };
        let signatures = package.signatures();
        let priority = source.getattr("priority")?.extract()?;
        for source in package.files().values() {
            check_cancel(Some(cancel))?;
            let lower = fold(source);
            if lower == "meta.ini"
                || lower.starts_with("fomod/")
                || ["plugins.txt", "loadorder.txt", "archives.txt"].contains(&lower.as_str())
            {
                continue;
            }
            let hidden = source.split('/').any(|p| fold(p).ends_with(".mohidden"));
            let logical = source
                .split('/')
                .map(|p| {
                    if fold(p).ends_with(".mohidden") {
                        &p[..p.len() - 9]
                    } else {
                        p
                    }
                })
                .collect::<Vec<_>>()
                .join("/");
            if logical.is_empty() {
                continue;
            }
            relative_path(&logical, false)?;
            let canonical = fold(&logical);
            let (size, modified) = signatures[&lower];
            let path = source.split('/').fold(root.clone(), |p, c| p.join(c));
            providers
                .entry(canonical.clone())
                .or_default()
                .push(Provider {
                    id: id.clone(),
                    name: name.clone(),
                    priority,
                    enabled,
                    hidden,
                    path,
                    size,
                    modified,
                });
            display.entry(canonical).or_insert(logical);
            count += 1;
            if count > 1_000_000 || providers.len() > 500_000 {
                return Err(invalid(
                    "File inspection exceeds the 1,000,000-provider / 500,000-path limit.",
                ));
            }
        }
    }
    check_cancel(Some(cancel))?;
    let current = library.call_method1("profile_snapshot", (profile_id,))?;
    let signature = conflict_rows_signature(&rows)?;
    if !conflict_rows_signature(&current.get_item(1)?)?
        .bind(py)
        .eq(signature.bind(py))?
        || !current
            .get_item(0)?
            .getattr("files")?
            .eq(profile.getattr("files")?)?
        || !current.get_item(2)?.eq(&config)?
    {
        return Err(invalid(
            "Profile organization changed during indexing; refresh the file view.",
        ));
    }
    let mut files = Vec::new();
    for (name, mut items) in providers {
        items.sort_by(|a, b| (a.priority, &a.id, a.hidden).cmp(&(b.priority, &b.id, b.hidden)));
        let mut entries = Vec::new();
        for p in items {
            entries.push(api.getattr("Provider")?.call1((
                p.id,
                p.name,
                p.priority,
                p.enabled,
                p.hidden,
                p.path.to_string_lossy().as_ref(),
                p.size,
                p.modified,
            ))?);
        }
        files.push(
            api.getattr("FileEntry")?
                .call1((&display[&name], PyTuple::new(py, entries)?))?,
        );
    }
    let files = PyTuple::new(py, files)?;
    let tree = if build_tree {
        py.import("creation_lib.mod_manager.data_tree")?
            .getattr("Tree")?
            .call1((&files, cancel))?
    } else {
        py.None().into_bound(py)
    };
    check_cancel(Some(cancel))?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("tree", tree)?;
    Ok(api
        .getattr("FileIndex")?
        .call(
            (
                profile.getattr("id")?,
                profile.getattr("name")?,
                files,
                PyTuple::new(py, issues)?,
                signature,
                PyTuple::new(py, unavailable)?,
            ),
            Some(&kwargs),
        )?
        .unbind())
}
#[pyfunction]
fn conflict_reveal(provider: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = provider.py();
    let path = py_path(py, &provider.getattr("path")?.extract::<PathBuf>()?)?
        .call_method0("absolute")?
        .extract::<PathBuf>()?;
    reject_links(&path)?;
    if !path.is_file()
        || package::signature(&path)?
            != (
                provider.getattr("size")?.extract()?,
                provider.getattr("modified")?.extract()?,
            )
    {
        return Err(invalid(
            "Source file changed or disappeared. Refresh inspection before revealing it.",
        ));
    }
    Ok(py
        .import("subprocess")?
        .getattr("Popen")?
        .call1((vec![
            "explorer.exe".into(),
            format!("/select,{}", path.display()),
        ],))?
        .unbind())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(conflict_providers, m)?)?;
    m.add_function(wrap_pyfunction!(conflict_selected, m)?)?;
    m.add_function(wrap_pyfunction!(conflict_filter, m)?)?;
    m.add_function(wrap_pyfunction!(conflict_rows_signature, m)?)?;
    m.add_function(wrap_pyfunction!(conflict_build_index, m)?)?;
    m.add_function(wrap_pyfunction!(conflict_reveal, m)?)?;
    Ok(())
}
