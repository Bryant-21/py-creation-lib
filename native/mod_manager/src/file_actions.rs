use crate::{
    check_cancel,
    history_prune::{py_path, stamp},
    invalid,
    python_io::with_context,
    reject_links, relative_path,
    saves::fold,
    transactions::transaction_key,
};
use pyo3::{prelude::*, types::PyDict};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
fn owned_path(
    library: &Bound<'_, PyAny>,
    provider: &Bound<'_, PyAny>,
    parent: bool,
) -> PyResult<(PathBuf, PathBuf, String)> {
    let py = library.py();
    let root = library
        .getattr("root")?
        .extract::<PathBuf>()?
        .join("mods")
        .join(transaction_key(
            &provider.getattr("mod_id")?.extract::<String>()?,
        )?);
    let source =
        py_path(py, &provider.getattr("path")?.extract::<PathBuf>()?)?.call_method0("absolute")?;
    if !parent {
        reject_links(&source.extract::<PathBuf>()?)?;
    }
    if !source
        .call_method1("is_relative_to", (&root,))?
        .is_truthy()?
    {
        return Err(invalid(if parent {
            "Only canonical manager-owned content has a hideable parent."
        } else {
            "Only files in this library's owned mod folders can be hidden or unhidden."
        }));
    }
    let name = relative_path(
        &source
            .call_method1("relative_to", (&root,))?
            .call_method0("as_posix")?
            .extract::<String>()?,
        false,
    )?;
    Ok((root, source.extract()?, name))
}
fn folder(name: &str) -> String {
    let mut parts = name.split('/').collect::<Vec<_>>();
    parts.pop();
    if let Some(index) = parts.iter().rposition(|p| fold(p).ends_with(".mohidden")) {
        parts.truncate(index + 1);
    }
    parts.join("/")
}
#[pyfunction]
fn file_parent_folder(library: &Bound<'_, PyAny>, provider: &Bound<'_, PyAny>) -> PyResult<String> {
    Ok(folder(&owned_path(library, provider, true)?.2))
}
fn unchanged(
    source: &Bound<'_, PyAny>,
    provider: &Bound<'_, PyAny>,
    message: &str,
) -> PyResult<()> {
    let info = stamp(&source.call_method0("stat")?)?;
    if info.2 != provider.getattr("size")?.extract::<u64>()?
        || info.3 != provider.getattr("modified")?.extract::<i64>()?
    {
        return Err(invalid(message));
    }
    Ok(())
}
#[pyfunction]
fn file_toggle_hidden(
    library: &Bound<'_, PyAny>,
    provider: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
    parent: bool,
) -> PyResult<String> {
    let py = library.py();
    library.call_method0("_runtime_write_guard")?;
    let (root, source, name) = owned_path(library, provider, parent)?;
    let name = if parent {
        let folder = folder(&name);
        if folder.is_empty() {
            return Err(invalid("A root-level file has no mod subfolder to hide."));
        }
        folder
    } else {
        let mut parts = name.split('/').collect::<Vec<_>>();
        parts.pop();
        if parts.iter().any(|p| fold(p).ends_with(".mohidden")) {
            return Err(invalid(
                "Unhide the parent folder before changing a file beneath it.",
            ));
        }
        name
    };
    reject_links(&source)?;
    let source = py_path(py, &source)?;
    unchanged(
        &source,
        provider,
        if parent {
            "File changed after inspection. Refresh Files before changing its parent."
        } else {
            "File changed after inspection. Refresh Files before hiding it."
        },
    )?;
    let destination = if fold(&name).ends_with(".mohidden") {
        name[..name.len() - 9].to_string()
    } else {
        format!("{name}.mohidden")
    };
    relative_path(&destination, false)?;
    if parent {
        let target = destination.split('/').fold(root.clone(), |p, c| p.join(c));
        reject_links(&target)?;
        if target.exists() {
            return Err(invalid(
                "Parent-folder hide/unhide destination already exists; no folders were merged or replaced.",
            ));
        }
    }
    let mod_id = provider.getattr("mod_id")?;
    let review = library.call_method1("review_owned", (&mod_id, cancel))?;
    let nexus = with_context(&library.call_method0("_db")?, |db| {
        let record = db
            .call_method1(
                "execute",
                (
                    "SELECT version,nexus_game,nexus_mod_id,nexus_file_id FROM mods WHERE id=?",
                    (&mod_id,),
                ),
            )?
            .call_method0("fetchone")?;
        let result = PyDict::new(py);
        for (target, source) in [
            ("version", "version"),
            ("game", "nexus_game"),
            ("mod_id", "nexus_mod_id"),
            ("file_id", "nexus_file_id"),
        ] {
            result.set_item(target, record.get_item(source)?)?;
        }
        Ok(result.unbind())
    })?;
    let api = py.import("creation_lib.mod_manager.package")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("inspect_installer", false)?;
    let package = api
        .getattr("Package")?
        .call((&root, cancel, api.getattr("OWNED_LIMITS")?), Some(&kwargs))?;
    with_context(
        &py.import("contextlib")?
            .getattr("closing")?
            .call1((&package,))?,
        |package| {
            unchanged(
                &source,
                provider,
                if parent {
                    "File changed while preparing parent-folder replacement. Refresh Files first."
                } else {
                    "File changed while preparing hidden-content replacement. Refresh Files first."
                },
            )?;
            let rename = |value: &str| -> String {
                if fold(value) == fold(&name)
                    || (parent && fold(value).starts_with(&format!("{}/", fold(&name))))
                {
                    format!(
                        "{destination}{}",
                        value.chars().skip(name.chars().count()).collect::<String>()
                    )
                } else {
                    value.to_string()
                }
            };
            let files = package.getattr("files")?;
            let mut plan = BTreeMap::new();
            for value in files.call_method0("values")?.try_iter()? {
                let value = value?.extract::<String>()?;
                plan.insert(rename(&value), value);
            }
            let directories = package.getattr("directories")?;
            let mut output = BTreeSet::new();
            for value in directories.try_iter()? {
                output.insert(rename(&value?.extract::<String>()?));
            }
            if plan.len() != files.len()?
                || (parent
                    && (output.len() != directories.len()?
                        || output
                            .iter()
                            .map(|p| fold(p))
                            .collect::<BTreeSet<_>>()
                            .len()
                            != output.len()))
            {
                return Err(invalid(if parent {
                    "Parent-folder rename creates a Windows destination collision."
                } else {
                    "Hide/unhide destination already exists; no file was replaced."
                }));
            }
            py.import("creation_lib.mod_manager.paths")?.call_method1(
                "validate_destinations",
                (plan.keys().cloned().collect::<Vec<_>>(),),
            )?;
            if parent
                && plan
                    .keys()
                    .map(|p| fold(p))
                    .collect::<BTreeSet<_>>()
                    .intersection(&output.iter().map(|p| fold(p)).collect())
                    .next()
                    .is_some()
            {
                return Err(invalid(
                    "Parent-folder rename creates a file/directory collision.",
                ));
            }
            check_cancel(Some(cancel))?;
            let kwargs = PyDict::new(py);
            kwargs.set_item("nexus", nexus)?;
            if parent {
                kwargs.set_item("restore_directories", output)?;
            } else {
                kwargs.set_item("restore_directories", directories)?;
            }
            library.call_method(
                "reinstall",
                (review, package, plan, cancel, progress),
                Some(&kwargs),
            )?;
            Ok(())
        },
    )?;
    Ok(destination)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(file_parent_folder, m)?)?;
    m.add_function(wrap_pyfunction!(file_toggle_hidden, m)?)?;
    Ok(())
}
