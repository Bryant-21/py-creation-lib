use crate::{check_cancel, invalid};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
use std::{collections::BTreeMap, path::PathBuf};
use unicode_casefold::UnicodeCaseFold;

#[pyfunction]
fn installer_versions<'py>(
    library: &Bound<'py, PyAny>,
    configuration: &Bound<'py, PyAny>,
    game: &str,
    mut requested: Vec<String>,
    cancel: &Bound<'py, PyAny>,
    fingerprint: &Bound<'py, PyAny>,
    file_version: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = library.py();
    requested.sort();
    let snapshot = py
        .import("creation_lib.mod_manager.installer_versions")?
        .getattr("Snapshot")?;
    if requested.is_empty() {
        return snapshot.call0();
    }
    let root = configuration
        .call_method1("get", ("game", PyDict::new(py)))?
        .call_method1("get", ("gamepath", ""))?
        .extract::<String>()?;
    let executable = match game {
        "fo4" => "Fallout4.exe",
        "fo3" => "Fallout3.exe",
        "fnv" => "FalloutNV.exe",
        "skyrim" => "TESV.exe",
        "skyrimse" => "SkyrimSE.exe",
        _ => "",
    };
    let mut versions = BTreeMap::<String, String>::new();
    let mut inputs = Vec::new();
    let mut issues = Vec::<String>::new();
    if root.is_empty() || executable.is_empty() {
        issues.push("Version discovery needs a supported configured game folder.".into());
    } else {
        let path = PathBuf::from(&root).join(executable);
        let path_type = py.import("pathlib")?.getattr("Path")?;
        check_cancel(Some(cancel))?;
        let result = (|| -> PyResult<()> {
            crate::reject_links(&path)?;
            if !path.is_file() {
                issues.push(
                    "The configured game executable is unavailable for version discovery.".into(),
                );
                return Ok(());
            }
            let path_object = path_type.call1((&path,))?;
            let before = fingerprint.call1((&path_object,))?;
            let version = file_version.call1((&path_object,))?;
            if !fingerprint.call1((&path_object,))?.eq(&before)? {
                return Err(invalid(
                    "Game executable changed during installer version discovery.",
                ));
            }
            inputs.push((
                path.to_string_lossy().case_fold().collect::<String>(),
                before,
            ));
            if version.is_truthy()? {
                let parts = version
                    .try_iter()?
                    .map(|v| Ok(v?.str()?.to_string()))
                    .collect::<PyResult<Vec<_>>>()?;
                versions.insert("game".into(), parts.join("."));
            } else if requested.iter().any(|v| v == "gameDependency") {
                issues.push(
                    "Game fixed version metadata is unavailable; supply an explicit version."
                        .into(),
                );
            }
            if requested.iter().any(|v| v == "f4seDependency") && game == "fo4" {
                let module = py.import("creation_lib.mod_manager.f4se_setup")?;
                let target = if library
                    .getattr("target_id")
                    .is_ok_and(|v| v.is_truthy().unwrap_or(false))
                {
                    library.call_method0("target")?
                } else {
                    py.None().into_bound(py)
                };
                let state = if target.is_truthy()?
                    && target
                        .getattr("f4se")?
                        .call_method1("get", ("state",))?
                        .eq("ready")?
                {
                    target.getattr("f4se")?
                } else {
                    module.call_method1("inspect_folder", (&root, cancel))?
                };
                if state.call_method1("get", ("state",))?.eq("ready")? {
                    if target.is_truthy()? {
                        let kwargs = PyDict::new(py);
                        kwargs.set_item("owner_root", library.getattr("root")?)?;
                        module
                            .getattr("validate_ready")?
                            .call((&target, cancel), Some(&kwargs))?;
                    }
                    versions.insert("f4se".into(), state.get_item("version")?.extract()?);
                    for entry in state.get_item("root_files")?.try_iter()? {
                        let entry = entry?;
                        check_cancel(Some(cancel))?;
                        let source = path_type.call1((entry.get_item(1)?,))?;
                        let actual = fingerprint.call1((&source,))?;
                        let expected = py.get_type::<PyTuple>().call1((entry.get_item(2)?,))?;
                        if !actual.eq(expected)? {
                            return Err(invalid(
                                "F4SE component changed during installer dependency discovery.",
                            ));
                        }
                        inputs.push((
                            source.str()?.to_string().case_fold().collect::<String>(),
                            actual,
                        ));
                    }
                } else {
                    let reason = state
                        .call_method1(
                            "get",
                            ("reason", state.call_method1("get", ("state", "unknown"))?),
                        )?
                        .extract::<String>()?;
                    issues.push(format!("Matching F4SE was not discovered: {reason}"));
                }
            }
            for tag in &requested {
                if !matches!(tag.as_str(), "gameDependency" | "f4seDependency") {
                    issues.push(format!("{tag}: automatic version discovery is not implemented; supply an explicit version."));
                }
            }
            if requested.iter().any(|v| v == "f4seDependency") && game != "fo4" {
                issues.push("F4SE dependency does not match this game's adapter.".into());
            }
            Ok(())
        })();
        match result {
            Ok(()) => {}
            Err(error)
                if error.is_instance_of::<crate::OperationCancelled>(py)
                    || error.matches(
                        py,
                        py.import("creation_lib.mod_manager.paths")?
                            .getattr("Cancelled")?,
                    )? =>
            {
                return Err(error);
            }
            Err(error)
                if error.is_instance_of::<pyo3::exceptions::PyValueError>(py)
                    || error.is_instance_of::<pyo3::exceptions::PyOSError>(py) =>
            {
                versions.clear();
                issues.push(format!(
                    "Version discovery is unavailable: {}",
                    error.value(py).str()?
                ));
            }
            Err(error) => return Err(error),
        }
        check_cancel(Some(cancel))?;
    }
    snapshot.call1((
        PyTuple::new(py, requested)?,
        PyTuple::new(py, versions)?,
        PyTuple::new(py, inputs)?,
        PyTuple::new(py, issues)?,
    ))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(installer_versions, m)?)?;
    Ok(())
}
