use crate::{
    check_cancel, history_prune::py_path, invalid, python_io::with_context, relative_path,
    saves::fold,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
fn validate(py: Python<'_>, names: impl IntoIterator<Item = String>) -> PyResult<()> {
    py.import("creation_lib.mod_manager.paths")?.call_method1(
        "validate_destinations",
        (names.into_iter().collect::<Vec<_>>(),),
    )?;
    Ok(())
}
#[pyfunction]
fn installation_remap(
    plan: BTreeMap<String, String>,
    changes: BTreeMap<String, Option<String>>,
    cancel: &Bound<'_, PyAny>,
    allow_empty: bool,
) -> PyResult<BTreeMap<String, String>> {
    let mut base = BTreeMap::new();
    for (target, source) in &plan {
        base.insert(fold(&relative_path(target, false)?), (target, source));
    }
    if base.len() != plan.len() {
        return Err(invalid(
            "Installer has ambiguous case-insensitive destination paths.",
        ));
    }
    let mut edits = BTreeMap::new();
    for (target, destination) in &changes {
        edits.insert(fold(&relative_path(target, false)?), destination);
    }
    if edits.len() != changes.len() || edits.keys().any(|k| !base.contains_key(k)) {
        return Err(invalid(
            "Custom mapping refers to changed installer destinations. Reset mappings and review again.",
        ));
    }
    let mut result = BTreeMap::new();
    for (key, (target, source)) in base {
        check_cancel(Some(cancel))?;
        let destination = match edits.get(&key) {
            Some(Some(v)) => v.as_str(),
            Some(None) => continue,
            None => target,
        };
        let destination = relative_path(destination, false)?;
        if result.insert(destination, source.clone()).is_some() {
            return Err(invalid("Custom mappings collide at a destination."));
        }
    }
    if result.is_empty() && !allow_empty {
        return Err(invalid("Custom mapping excludes every installable file."));
    }
    validate(cancel.py(), result.keys().cloned())?;
    check_cancel(Some(cancel))?;
    Ok(result)
}
fn payload(name: &str, prefix: &str) -> bool {
    let logical = if !prefix.is_empty() && fold(name).starts_with(&fold(prefix)) {
        name.chars()
            .skip(prefix.chars().count())
            .collect::<String>()
    } else {
        name.into()
    };
    !crate::package::installer_metadata(&logical)
        && !name.split('/').any(|p| {
            let p = fold(p);
            p.ends_with(".mohidden") || p.ends_with(".ghost")
        })
}
#[pyfunction]
fn installation_source_tree(
    package: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = package.py();
    let prefix: String = package.getattr("prefix")?.extract()?;
    let source = py
        .import("creation_lib.mod_manager.installation")?
        .getattr("Source")?;
    let mut entries = Vec::new();
    for name in package
        .getattr("files")?
        .call_method0("values")?
        .try_iter()?
    {
        check_cancel(Some(cancel))?;
        let name = name?.extract::<String>()?;
        if payload(&name, &prefix) {
            entries.push(source.call1((name,))?);
        }
    }
    Ok(py
        .import("creation_lib.mod_manager.data_tree")?
        .getattr("Tree")?
        .call1((PyTuple::new(py, entries)?, cancel))?
        .unbind())
}
#[pyfunction]
fn installation_include_sources(
    package: &Bound<'_, PyAny>,
    plan: BTreeMap<String, String>,
    additions: BTreeMap<String, String>,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<BTreeMap<String, String>> {
    let mut result = plan;
    let mut known = result.keys().map(|n| fold(n)).collect::<BTreeSet<_>>();
    let files = package.getattr("files")?;
    let prefix: String = package.getattr("prefix")?.extract()?;
    for (target, source) in additions {
        check_cancel(Some(cancel))?;
        let target = relative_path(&target, false)?;
        let source = relative_path(&source, false)?;
        if !files.contains(fold(&source))? {
            return Err(invalid(
                "Additional installer source is absent from the reviewed package.",
            ));
        }
        let source = files.get_item(fold(&source))?.extract::<String>()?;
        if !payload(&source, &prefix) {
            return Err(invalid(
                "Installer metadata and hidden sources cannot be added as payloads.",
            ));
        }
        if !known.insert(fold(&target)) {
            return Err(invalid(
                "Additional source collides with an installer destination. Exclude or relocate that destination first.",
            ));
        }
        result.insert(target, source);
    }
    if result.is_empty() {
        return Err(invalid("Select at least one installable destination."));
    }
    validate(package.py(), result.keys().cloned())?;
    Ok(result)
}
#[pyfunction]
fn installation_combined_init(
    combined: &Bound<'_, PyAny>,
    existing: &Bound<'_, PyAny>,
    incoming: &Bound<'_, PyAny>,
    plan: BTreeMap<String, String>,
) -> PyResult<()> {
    let py = combined.py();
    validate(py, plan.keys().cloned())?;
    let sources = PyDict::new(py);
    let sizes = PyDict::new(py);
    let mut merged = BTreeMap::new();
    let mut mapping = BTreeMap::new();
    for name in existing
        .getattr("files")?
        .call_method0("values")?
        .try_iter()?
    {
        let name = name?.extract::<String>()?;
        mapping.insert(name.clone(), name);
    }
    for (prefix, package, mapping) in [
        ("existing", existing, mapping),
        ("incoming", incoming, plan),
    ] {
        for (target, source) in mapping {
            check_cancel(Some(&existing.getattr("_cancel")?))?;
            let key = format!("{prefix}/{source}");
            merged.insert(fold(&target), (target, key.clone()));
            sources.set_item(fold(&key), (package, &source))?;
            sizes.set_item(
                fold(&key),
                package.getattr("sizes")?.get_item(fold(&source))?,
            )?;
        }
    }
    let limits = py
        .import("creation_lib.mod_manager.package")?
        .getattr("OWNED_LIMITS")?;
    let mut total = 0u128;
    for (_, source) in merged.values() {
        total += sizes.get_item(fold(source))?.unwrap().extract::<u64>()? as u128;
    }
    if merged.len() > limits.getattr("files")?.extract::<usize>()?
        || total > limits.getattr("total_bytes")?.extract::<u64>()? as u128
    {
        return Err(invalid(
            "Merged payload exceeds the owned-mod file/byte limits.",
        ));
    }
    let plan = merged.into_values().collect::<BTreeMap<_, _>>();
    validate(py, plan.keys().cloned())?;
    combined.setattr("sources", sources)?;
    combined.setattr("sizes", sizes)?;
    combined.setattr("plan", plan)?;
    combined.setattr("_cancel", py.None())?;
    Ok(())
}
#[pyfunction]
fn installation_combined_open(combined: &Bound<'_, PyAny>, source: &str) -> PyResult<Py<PyAny>> {
    let cancel = combined.getattr("_cancel")?;
    check_cancel(Some(&cancel))?;
    let item = combined.getattr("sources")?.get_item(fold(source))?;
    let package = item.get_item(0)?;
    package.setattr("_cancel", cancel)?;
    Ok(package.call_method1("open", (item.get_item(1)?,))?.unbind())
}
#[pyfunction]
fn installation_merge(
    library: &Bound<'_, PyAny>,
    review: &Bound<'_, PyAny>,
    incoming: &Bound<'_, PyAny>,
    plan: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
    nexus: &Bound<'_, PyAny>,
    validate_before_commit: &Bound<'_, PyAny>,
    preview: bool,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let expected = library
        .getattr("root")?
        .extract::<PathBuf>()?
        .join("mods")
        .join(review.getattr("mod_id")?.extract::<String>()?);
    let mut nexus = nexus.clone();
    let source = if preview {
        expected.clone()
    } else {
        library.call_method0("_runtime_write_guard")?;
        let row = with_context(&library.call_method0("_db")?, |db| {
            Ok(db.call_method1("execute",("SELECT source,version,nexus_game,nexus_mod_id,nexus_file_id FROM mods WHERE id=? AND kind='owned'",(review.getattr("mod_id")?,)))?.call_method0("fetchone")?.unbind())
        })?;
        let row = row.bind(py);
        if row.is_none() {
            return Err(invalid("Merge requires an existing manager-owned mod."));
        }
        if nexus.is_none() {
            let data = PyDict::new(py);
            for (key, column) in [
                ("version", "version"),
                ("game", "nexus_game"),
                ("mod_id", "nexus_mod_id"),
                ("file_id", "nexus_file_id"),
            ] {
                data.set_item(key, row.get_item(column)?)?;
            }
            nexus = data.into_any();
        }
        let source = row.get_item("source")?.extract::<PathBuf>()?;
        if !py_path(py, &source)?
            .call_method0("absolute")?
            .eq(py_path(py, &expected)?)?
        {
            return Err(invalid(
                "Merge target is outside its manager-owned mod directory.",
            ));
        }
        source
    };
    let package = py.import("creation_lib.mod_manager.package")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("inspect_installer", false)?;
    let existing = package.getattr("Package")?.call(
        (source, cancel, package.getattr("OWNED_LIMITS")?),
        Some(&kwargs),
    )?;
    with_context(
        &py.import("contextlib")?
            .getattr("closing")?
            .call1((existing,))?,
        |existing| {
            let combined = py
                .import("creation_lib.mod_manager.installation")?
                .getattr("CombinedPackage")?
                .call1((existing, incoming, plan))?;
            let plan = combined.getattr("plan")?;
            if preview {
                check_cancel(Some(cancel))?;
                return Ok(py
                    .import("builtins")?
                    .getattr("list")?
                    .call1((plan.call_method0("items")?,))?
                    .unbind());
            }
            let destinations = plan
                .try_iter()?
                .map(|n| Ok(fold(&n?.extract::<String>()?)))
                .collect::<PyResult<BTreeSet<_>>>()?;
            for directory in existing.getattr("directories")?.try_iter()? {
                if destinations.contains(&fold(&directory?.extract::<String>()?)) {
                    return Err(invalid(
                        "Merged file conflicts with an existing directory. Review a Replace install instead.",
                    ));
                }
            }
            check_cancel(Some(cancel))?;
            let kwargs = PyDict::new(py);
            kwargs.set_item("review", review)?;
            kwargs.set_item("nexus", &nexus)?;
            kwargs.set_item("restore_directories", existing.getattr("directories")?)?;
            kwargs.set_item("validate_before_commit", validate_before_commit)?;
            Ok(library
                .getattr("owned")?
                .call_method(
                    "install",
                    (
                        combined,
                        plan,
                        review.getattr("name")?,
                        "",
                        cancel,
                        progress,
                    ),
                    Some(&kwargs),
                )?
                .unbind())
        },
    )
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(installation_remap, m)?)?;
    m.add_function(wrap_pyfunction!(installation_source_tree, m)?)?;
    m.add_function(wrap_pyfunction!(installation_include_sources, m)?)?;
    m.add_function(wrap_pyfunction!(installation_combined_init, m)?)?;
    m.add_function(wrap_pyfunction!(installation_combined_open, m)?)?;
    m.add_function(wrap_pyfunction!(installation_merge, m)?)?;
    Ok(())
}
