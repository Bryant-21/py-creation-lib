use crate::{
    agent_library::{library, profile},
    encoding, invalid,
    plugin_inventory::{object, value},
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use serde_json::json;

pub fn finish(manager: &Bound<'_, PyAny>) -> PyResult<()> {
    while manager.getattr("busy")?.is_truthy()? {
        let result = manager.getattr("_future")?.call_method0("result");
        manager.call_method0("poll")?;
        result?;
    }
    let error = manager.getattr("error")?.extract::<String>()?;
    if !error.is_empty() {
        return Err(invalid(error));
    }
    Ok(())
}
pub fn with_manager<T>(
    root: &Bound<'_, PyAny>,
    wanted: &str,
    body: impl FnOnce(&Bound<'_, PyAny>) -> PyResult<T>,
) -> PyResult<T> {
    let py = root.py();
    let library = library(py, root)?;
    let manager = py
        .import("creation_lib.mod_manager.controller")?
        .call_method0("ManagerController")?;
    let result = (|| {
        manager.setattr("library", &library)?;
        if !wanted.is_empty() {
            manager.setattr(
                "profile_id",
                profile(&library, wanted.into_pyobject(py)?.as_any())?.getattr("id")?,
            )?;
        }
        manager.call_method0("refresh")?;
        body(&manager)
    })();
    let cleanup = manager.call_method0("cleanup");
    match result {
        Ok(value) => {
            cleanup?;
            Ok(value)
        }
        Err(e) => Err(e),
    }
}
#[pyfunction]
fn agent_create_instance(
    root: &Bound<'_, PyAny>,
    game: &str,
    game_root: &str,
    name: &str,
) -> PyResult<Py<PyAny>> {
    let py = root.py();
    let kwargs = PyDict::new(py);
    kwargs.set_item("create", true)?;
    let library = py
        .import("creation_lib.mod_manager.library")?
        .getattr("Library")?
        .call((root,), Some(&kwargs))?;
    let id = py
        .import("creation_lib.mod_manager.instances")?
        .call_method1("create", (&library, game, game_root, name))?;
    Ok(object(py, &json!({"profile_id":id.extract::<String>()?,"name":name,"library":library.getattr("root")?.str()?.to_string()}))?.unbind())
}
#[pyfunction]
fn agent_install(
    root: &Bound<'_, PyAny>,
    wanted: &str,
    package: &str,
    options: &str,
) -> PyResult<Py<PyAny>> {
    if options.len() > 1024 * 1024 {
        return Err(invalid("Installer options exceed 1 MiB."));
    }
    let opts = encoding::parse(options)?;
    let manual_root = opts["root"].as_str().unwrap_or("");
    let name = opts["name"].as_str().unwrap_or("");
    let apply = opts["apply"].as_bool().unwrap_or(false);
    let replace = opts["replace"].as_str().unwrap_or("");
    let mode = opts["mode"].as_str().unwrap_or("replace");
    if !["replace", "merge"].contains(&mode) {
        return Err(invalid("Install mode must be replace or merge."));
    }
    with_manager(root, wanted, |manager| {
        let py = manager.py();
        let target = opts["target"].as_str().unwrap_or("");
        if !target.is_empty() {
            manager.call_method1("select_target", (target,))?;
        }
        let kwargs = PyDict::new(py);
        kwargs.set_item("manual", opts["manual"].as_bool().unwrap_or(false))?;
        kwargs.set_item("overlay_id", opts["overlay"].as_str().unwrap_or(""))?;
        manager.call_method("prepare_install", (package, replace), Some(&kwargs))?;
        finish(manager)?;
        let policy = opts["policy"].as_str().unwrap_or("mo2");
        if policy != "mo2" {
            manager.call_method1("change_fomod_policy", (policy,))?;
            finish(manager)?;
        }
        let adjustments = opts
            .get("adjustments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !adjustments.is_object() {
            return Err(invalid(
                "Installer adjustments must be an object with mapping, additions or repairs objects.",
            ));
        }
        for (key, field) in [
            ("mapping", "install_mapping"),
            ("additions", "install_additions"),
            ("repairs", "install_repairs"),
        ] {
            let value = adjustments.get(key).cloned().unwrap_or_else(|| json!({}));
            if !value.is_object() {
                return Err(invalid(format!("Installer {key} must be an object.")));
            }
            manager.setattr(field, object(py, &value)?)?;
        }
        manager.setattr(
            "enable_on_install",
            !opts["disabled"].as_bool().unwrap_or(false),
        )?;
        let choices = opts.get("choices").cloned().unwrap_or_else(|| json!({}));
        if !choices.is_object() || choices.to_string().len() > 256 * 1024 {
            return Err(invalid(
                "Installer choices must be a JSON object within 256 KiB.",
            ));
        }
        manager.setattr("choices", object(py, &choices)?)?;
        manager.setattr("install_mode", mode)?;
        let name = if name.is_empty() {
            let row = manager
                .getattr("rows_by_id")?
                .call_method1("get", (replace,))?;
            if row.is_none() {
                std::path::Path::new(package)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            } else {
                row.getattr("name")?.extract::<String>()?
            }
        } else {
            name.to_owned()
        };
        let prepared = manager.getattr("prepared")?;
        let fomod = prepared.getattr("fomod")?;
        let steps = PyList::empty(py);
        let mut errors = Vec::<String>::new();
        if !fomod.is_none() {
            let evaluation = fomod.call_method1(
                "evaluate",
                (
                    manager.getattr("choices")?,
                    prepared.getattr("environment")?,
                ),
            )?;
            errors = evaluation.getattr("errors")?.extract()?;
            for step in evaluation.getattr("steps")?.try_iter()? {
                steps.append(py.import("dataclasses")?.call_method1("asdict", (step?,))?)?;
            }
        }
        let mut plan = json!({});
        if errors.is_empty() {
            let reviewed = manager
                .call_method1("review_install", (manual_root,))
                .and_then(|_| finish(manager));
            match reviewed {
                Ok(_) => plan = value(&manager.getattr("install_plan")?)?,
                Err(e) if !apply => {
                    errors.push(e.value(py).str()?.to_string());
                    manager.setattr("error", "")?;
                }
                Err(e) => return Err(e),
            }
        }
        let token = encoding::digest(
            &json!([
                crate::package::signature(std::path::Path::new(package))?,
                value(
                    &prepared
                        .getattr("package")?
                        .getattr("_index")?
                        .getattr("signatures")?
                )?,
                if prepared.getattr("replacement_review")?.is_none() {
                    serde_json::Value::Null
                } else {
                    value(
                        &py.import("dataclasses")?
                            .call_method1("asdict", (prepared.getattr("replacement_review")?,))?,
                    )?
                },
                plan,
                choices,
                package,
                replace,
                mode,
                manual_root,
                target,
                policy,
                adjustments,
                opts["disabled"].as_bool().unwrap_or(false),
                crate::agent_workflow::serial(&prepared.getattr("overlay_review")?, 0)?,
                value(
                    &py.import("dataclasses")?
                        .call_method1("asdict", (prepared.getattr("environment")?,))?
                )?
            ]),
            true,
            true,
        );
        let expected = opts["expect"].as_str().unwrap_or("");
        if !expected.is_empty() && token != expected {
            return Err(invalid(
                "Installer plan changed since the supplied review token.",
            ));
        }
        let result = object(
            py,
            &json!({"package":package,"fomod":!fomod.is_none(),"steps":value(steps.as_any())?,
            "errors":errors,"plan":plan,"review_token":token,"applied":false}),
        )?;
        if apply {
            if !errors.is_empty() {
                return Err(invalid(errors.join("\n")));
            }
            manager.call_method1("install", (name, manual_root))?;
            finish(manager)?;
            result.set_item("applied", true)?;
            result.set_item("mod_id", manager.getattr("selected_id")?)?;
        }
        Ok(result.unbind())
    })
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(agent_create_instance, m)?)?;
    m.add_function(wrap_pyfunction!(agent_install, m)?)?;
    Ok(())
}
