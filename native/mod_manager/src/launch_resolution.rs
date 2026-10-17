use crate::{check_cancel, history_prune::py_path, invalid, saves::fold};
use pyo3::{prelude::*, types::PyDict};
use std::{collections::BTreeSet, path::PathBuf};
fn same(py: Python<'_>, first: &Bound<'_, PyAny>, second: &Bound<'_, PyAny>) -> PyResult<bool> {
    py_path(py, &first.extract::<PathBuf>()?)?
        .call_method0("resolve")?
        .eq(py_path(py, &second.extract::<PathBuf>()?)?.call_method0("resolve")?)
}
fn managed(target: &Bound<'_, PyAny>) -> PyResult<bool> {
    target
        .getattr("f4se")?
        .call_method1("get", ("managed",))?
        .is_truthy()
}
fn resolution(
    executable: &Bound<'_, PyAny>,
    target: &Bound<'_, PyAny>,
    game: &Bound<'_, PyAny>,
    working: bool,
) -> PyResult<Py<PyAny>> {
    let py = executable.py();
    let kwargs = PyDict::new(py);
    kwargs.set_item("id", target.getattr("executable")?.getattr("id")?)?;
    if working {
        let value = executable.getattr("working_directory")?;
        kwargs.set_item(
            "working_directory",
            if value.is_truthy()? {
                value
            } else {
                target.getattr("working_directory")?
            },
        )?;
    }
    let executable = py
        .import("dataclasses")?
        .getattr("replace")?
        .call((executable,), Some(&kwargs))?;
    Ok(py
        .import("creation_lib.mod_manager.launch_resolution")?
        .getattr("Resolution")?
        .call1((target.getattr("id")?, game.str()?, executable))?
        .unbind())
}
#[pyfunction]
fn launch_resolve(
    library: &Bound<'_, PyAny>,
    executable: &Bound<'_, PyAny>,
    selected_target: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    check_cancel(Some(cancel))?;
    let launcher = py.import("creation_lib.mod_manager.launcher")?;
    let record = executable.call_method0("record")?;
    let executable = launcher
        .getattr("definition")?
        .call((), Some(record.cast::<PyDict>()?))?;
    let binary =
        launcher.call_method1("local_path", (executable.getattr("binary")?, "Executable"))?;
    let binary_name = fold(&binary.getattr("name")?.extract::<String>()?);
    if !["fallout4.exe", "f4se_loader.exe"].contains(&binary_name.as_str()) {
        return Err(invalid(
            "The current modded adapter supports Fallout4.exe and f4se_loader.exe.",
        ));
    }
    let api = py.import("creation_lib.mod_manager.launch_targets")?;
    let targets = api
        .call_method1("records", (library,))?
        .try_iter()?
        .collect::<PyResult<Vec<_>>>()?;
    let mut associated = None;
    for target in &targets {
        if target
            .getattr("executable")?
            .getattr("id")?
            .eq(executable.getattr("id")?)?
        {
            associated = Some(target);
            break;
        }
    }
    if associated.is_none() {
        for target in &targets {
            if managed(target)? && same(py, &target.getattr("binary")?, &binary)? {
                associated = Some(target);
                break;
            }
        }
    }
    let validate = py
        .import("creation_lib.mod_manager.instances")?
        .getattr("validate_root")?;
    if let Some(target) = associated {
        if managed(target)? {
            if !same(py, &target.getattr("binary")?, &binary)? {
                return Err(invalid(
                    "The staged loader differs from its explicit launch target association.",
                ));
            }
            validate.call1((
                "fo4",
                target.getattr("game_root")?,
                library.getattr("root")?,
            ))?;
            return resolution(&executable, target, &target.getattr("game_root")?, true);
        }
    }
    if binary
        .call_method0("resolve")?
        .call_method1(
            "is_relative_to",
            (library.getattr("root")?.call_method0("resolve")?,),
        )?
        .is_truthy()?
    {
        return Err(invalid(
            "A manager-staged loader requires its registered target association; choose that target.",
        ));
    }
    let game = validate
        .call1(("fo4", binary.getattr("parent")?, library.getattr("root")?))?
        .get_item(1)?;
    let pe = py
        .import("creation_lib.mod_manager.runtime")?
        .getattr("pe_machine")?;
    for path in [
        binary.clone(),
        game.call_method1("joinpath", ("Fallout4.exe",))?,
    ] {
        if pe.call1((path,))?.extract::<u32>()? != 0x8664 {
            return Err(invalid(
                "The selected loader's installation must contain x64 Fallout4.exe and Data.",
            ));
        }
    }
    let mut candidates = Vec::new();
    for target in &targets {
        if !managed(target)? && same(py, &target.getattr("game_root")?, &game)? {
            candidates.push(target.clone());
        }
    }
    let mut target = None;
    for candidate in &candidates {
        if candidate.getattr("id")?.extract::<String>()? == selected_target {
            target = Some(candidate.clone());
            break;
        }
    }
    if target.is_none() {
        target = candidates.first().cloned();
    }
    let target = if let Some(target) = target {
        target
    } else {
        let stem = format!("Fallout 4 — {}", game.getattr("name")?.extract::<String>()?)
            .chars()
            .take(180)
            .collect::<String>();
        let mut name = stem.clone();
        let mut suffix = 2;
        let mut names = BTreeSet::new();
        for target in &targets {
            names.insert(fold(&target.getattr("name")?.extract::<String>()?));
        }
        while names.contains(&fold(&name)) {
            name = format!("{stem} ({suffix})");
            suffix += 1;
        }
        let kwargs = PyDict::new(py);
        kwargs.set_item("cancel", cancel)?;
        let plan = api.getattr("review")?.call(
            (library, name, game.str()?, binary.str()?, game.str()?),
            Some(&kwargs),
        )?;
        if binary_name == "f4se_loader.exe" {
            py.import("creation_lib.mod_manager.f4se_setup")?
                .call_method1("validate_launch", (plan.getattr("target")?, cancel))?;
        }
        api.call_method1("apply", (library, &plan, cancel))?;
        plan.getattr("target")?
    };
    check_cancel(Some(cancel))?;
    resolution(&executable, &target, &game, false)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(launch_resolve, m)?)?;
    Ok(())
}
