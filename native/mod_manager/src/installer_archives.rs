use crate::{check_cancel, saves::fold};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
use std::collections::{BTreeMap, BTreeSet};
#[pyfunction]
fn installer_archive_capture(
    library: &Bound<'_, PyAny>,
    profile: &str,
    requested: Vec<String>,
    loose: &Bound<'_, PyAny>,
    game: &str,
    game_available: bool,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let class = py
        .import("creation_lib.mod_manager.installer_archives")?
        .getattr("Snapshot")?;
    let mut present = BTreeSet::new();
    for entry in loose.getattr("files")?.try_iter()? {
        let entry = entry?;
        if entry.getattr("winner")?.is_truthy()? {
            present.insert(fold(&entry.getattr("path")?.extract::<String>()?));
        }
    }
    let assets = requested
        .into_iter()
        .filter(|n| {
            ![".esp", ".esm", ".esl"]
                .iter()
                .any(|suffix| n.ends_with(suffix))
                && !present.contains(n)
        })
        .collect::<Vec<_>>();
    if assets.is_empty() || !game_available {
        return Ok(class.call0()?.unbind());
    }
    check_cancel(Some(cancel))?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("loose", loose)?;
    let index = match py
        .import("creation_lib.mod_manager.archive_index")?
        .getattr("build")?
        .call((library, profile, cancel, progress), Some(&kwargs))
    {
        Ok(v) => v,
        Err(e) => {
            let paths = py.import("creation_lib.mod_manager.paths")?;
            if e.is_instance(py, &paths.getattr("Cancelled")?) {
                return Err(e);
            }
            if e.is_instance_of::<pyo3::exceptions::PyOSError>(py)
                || e.is_instance(py, &paths.getattr("ManagerError")?)
            {
                let kwargs = PyDict::new(py);
                kwargs.set_item(
                    "issues",
                    (format!(
                        "Archive dependency inventory is unavailable: {}",
                        e.value(py).str()?
                    ),),
                )?;
                return Ok(class.call((), Some(&kwargs))?.unbind());
            }
            return Err(e);
        }
    };
    let mut records = Vec::new();
    for archive in index.getattr("archives")?.try_iter()? {
        let archive = archive?;
        let provider = archive.getattr("provider")?;
        records.push(
            (
                archive.getattr("name")?,
                provider.getattr("path")?,
                provider.getattr("size")?,
                provider.getattr("modified")?,
                archive.getattr("selected")?,
                archive.getattr("activation")?,
                archive.getattr("rank")?,
            )
                .into_pyobject(py)?,
        );
    }
    let mut members = BTreeMap::new();
    for member in index.getattr("members")?.try_iter()? {
        let member = member?;
        members.insert(
            fold(&member.getattr("path")?.extract::<String>()?),
            member.getattr("archives")?,
        );
    }
    let mut states = Vec::new();
    let mut issues = Vec::new();
    for name in assets {
        check_cancel(Some(cancel))?;
        let mut active = false;
        let mut unresolved = game != "fo4";
        if let Some(archives) = members.get(&name) {
            for archive in archives.try_iter()? {
                let archive = archive?;
                if !archive.getattr("selected")?.is_truthy()? {
                    continue;
                }
                let activation = archive.getattr("activation")?.extract::<String>()?;
                let standard = [" - main.ba2", " - textures.ba2"].iter().any(|s| {
                    fold(
                        &archive
                            .getattr("name")
                            .and_then(|v| v.extract::<String>())
                            .unwrap_or_default(),
                    )
                    .ends_with(s)
                });
                let accepted =
                    activation == "listed" || (activation == "plugin candidate" && standard);
                active |= accepted;
                unresolved |=
                    activation == "unknown" || (activation == "plugin candidate" && !accepted);
            }
        }
        if active {
            states.push((name, "Active"));
        } else if unresolved {
            issues.push(format!(
                "{name}: archive attachment is unresolved; confirm its dependency state explicitly."
            ));
        } else {
            states.push((name, "Missing"));
        }
    }
    Ok(class
        .call1((
            PyTuple::new(py, states)?,
            PyTuple::new(py, records)?,
            PyTuple::new(py, issues)?,
        ))?
        .unbind())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(installer_archive_capture, m)?)?;
    Ok(())
}
