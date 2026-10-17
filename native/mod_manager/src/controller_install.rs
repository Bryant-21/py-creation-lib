use crate::{check_cancel, controller::callback, invalid};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
fn module<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyModule>> {
    py.import(format!("creation_lib.mod_manager.{name}"))
}
fn cancelled(py: Python<'_>, e: &PyErr) -> PyResult<bool> {
    Ok(e.is_instance_of::<crate::OperationCancelled>(py)
        || e.matches(py, &module(py, "paths")?.getattr("Cancelled")?)?)
}
#[pyfunction]
fn controller_prepare_install(
    manager: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    replace_id: &str,
    manual: bool,
    download_review: &Bound<'_, PyAny>,
    overlay_id: &str,
) -> PyResult<()> {
    let py = manager.py();
    if !overlay_id.is_empty() && !replace_id.is_empty() {
        return Err(invalid(
            "Choose an owned reinstall or a referenced additive update, separately.",
        ));
    }
    manager.call_method0("discard_install")?;
    let library = manager.getattr("context_library")?;
    let mut profile = py.None().into_bound(py);
    for item in manager.getattr("profiles")?.try_iter()? {
        let item = item?;
        if item.getattr("id")?.eq(manager.getattr("profile_id")?)? {
            profile = item;
            break;
        }
    }
    if !download_review.is_none() && (!library.is_truthy()? || !profile.is_truthy()?) {
        return Err(invalid(
            "Choose the reviewed local profile before installing this download.",
        ));
    }
    let library = library.unbind();
    let profile = profile.unbind();
    let path = path.clone().unbind();
    let download = download_review.clone().unbind();
    let replace_id = replace_id.to_string();
    let overlay_id = overlay_id.to_string();
    let work = callback(py, move |py, args, _| {
        let cancel = args.get_item(0)?;
        let progress = args.get_item(1)?;
        let library = library.bind(py);
        let profile = profile.bind(py);
        let path = path.bind(py);
        let download = download.bind(py);
        if !download.is_none() {
            let absolute = py
                .import("pathlib")?
                .getattr("Path")?
                .call1((path,))?
                .call_method0("absolute")?;
            if !absolute.str()?.as_any().eq(download.getattr("path")?)? {
                return Err(invalid(
                    "Download archive differs from its destination review.",
                ));
            }
            let module = module(py, "download_install")?;
            let kwargs = PyDict::new(py);
            kwargs.set_item("overlay", !overlay_id.is_empty())?;
            module.getattr("destination")?.call(
                (
                    download,
                    if overlay_id.is_empty() {
                        &replace_id
                    } else {
                        &overlay_id
                    },
                ),
                Some(&kwargs),
            )?;
            module.call_method1(
                "validate",
                (library, profile.getattr("id")?, download, &cancel),
            )?;
        }
        let kwargs = PyDict::new(py);
        kwargs.set_item("inspect_installer", !manual)?;
        let package = module(py, "package")?
            .getattr("Package")?
            .call((path, &cancel), Some(&kwargs))?;
        let result = (|| {
            let fomod_module = module(py, "fomod")?;
            let fomod = if package.getattr("config_path")?.is_truthy()? {
                let kwargs = PyDict::new(py);
                kwargs.set_item("compatibility", "mo2")?;
                fomod_module
                    .getattr("Fomod")?
                    .call((package.call_method0("read_config")?,), Some(&kwargs))?
            } else {
                py.None().into_bound(py)
            };
            let mut states = PyDict::new(py).into_any();
            let mut context = py.None().into_bound(py);
            if fomod.is_truthy()? && library.is_truthy()? && profile.is_truthy()? {
                let captured = module(py, "installer_context")?.call_method1(
                    "capture",
                    (library, profile.getattr("id")?, &fomod, &cancel, &progress),
                )?;
                context = captured.get_item(0)?;
                states = captured.get_item(1)?;
            }
            let kwargs = PyDict::new(py);
            kwargs.set_item("complete", false)?;
            let environment = fomod_module
                .getattr("Environment")?
                .call((states,), Some(&kwargs))?;
            if context.is_truthy()? && context.getattr("versions")?.is_truthy()? {
                let versions = py
                    .import("builtins")?
                    .call_method1("dict", (context.getattr("versions")?.getattr("versions")?,))?;
                environment.setattr("game_version", versions.call_method1("pop", ("game", ""))?)?;
                environment
                    .getattr("extender_versions")?
                    .call_method1("update", (versions,))?;
            }
            check_cancel(Some(&cancel))?;
            let review = if !replace_id.is_empty() && library.is_truthy()? {
                library.call_method1("review_owned", (&replace_id, &cancel))?
            } else {
                py.None().into_bound(py)
            };
            let overlay = if !overlay_id.is_empty() {
                module(py, "reference_overlay")?.call_method1(
                    "capture",
                    (library, profile.getattr("id")?, &overlay_id, &cancel),
                )?
            } else {
                py.None().into_bound(py)
            };
            let preview = module(py, "installer_preview")?;
            let info = match preview.call_method1("metadata", (&package, &cancel)) {
                Ok(info) => info,
                Err(e)
                    if e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
                        && !cancelled(py, &e)? =>
                {
                    let info = PyDict::new(py);
                    info.set_item("_error", e.value(py).str()?)?;
                    info.into_any()
                }
                Err(e) => return Err(e),
            };
            let mut header = py.None().into_bound(py);
            if fomod.is_truthy()? && fomod.getattr("decoration")?.getattr("image")?.is_truthy()? {
                match preview.call_method1(
                    "image",
                    (
                        &package,
                        fomod.getattr("decoration")?.getattr("image")?,
                        &cancel,
                    ),
                ) {
                    Ok(image) => header = image,
                    Err(e)
                        if e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
                            && !cancelled(py, &e)? =>
                    {
                        info.set_item("Header artwork", e.value(py).str()?)?
                    }
                    Err(e) => return Err(e),
                }
            }
            let kwargs = PyDict::new(py);
            kwargs.set_item("replacement_review", review)?;
            kwargs.set_item(
                "source_tree",
                module(py, "installation")?.call_method1("source_tree", (&package, &cancel))?,
            )?;
            kwargs.set_item("metadata", info)?;
            kwargs.set_item("manual", manual)?;
            kwargs.set_item("context", context)?;
            kwargs.set_item("download_review", download)?;
            kwargs.set_item("header_image", header)?;
            kwargs.set_item("overlay_review", overlay)?;
            Ok(module(py, "controller")?
                .getattr("PreparedInstall")?
                .call((&package, fomod, environment), Some(&kwargs))?
                .unbind())
        })();
        if result.is_err() {
            package.call_method0("close")?;
        }
        result
    })?;
    let owner = manager.clone().unbind();
    let download = download_review.clone().unbind();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let prepared = args.get_item(0)?;
        manager.setattr("prepared", &prepared)?;
        let nexus = if !download.bind(py).is_none() {
            download.bind(py).call_method0("metadata")?
        } else {
            let mut value = py.None().into_bound(py);
            for record in manager.getattr("download_records")?.try_iter()? {
                let record = record?;
                let path = py
                    .import("pathlib")?
                    .getattr("Path")?
                    .call1((record.get_item("path")?,))?
                    .call_method0("absolute")?;
                if path.eq(prepared.getattr("package")?.getattr("path")?)? {
                    value = record;
                    break;
                }
            }
            value
        };
        prepared.setattr("nexus", nexus)?;
        manager.setattr("choices", PyDict::new(py))?;
        manager.setattr("status", if prepared.getattr("manual")?.is_truthy()? { "Manual package layout ready. Choose payload roots/variants and review destinations; FOMOD rules were skipped." } else { "Package ready. Review the installer choices and file plan before installing." })?;
        Ok(py.None())
    })?;
    manager.call_method1(
        "submit",
        (
            "Reading mod package and profile dependencies",
            work,
            complete,
        ),
    )?;
    Ok(())
}
fn sorted_tuple<'py>(py: Python<'py>, values: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let builtins = py.import("builtins")?;
    builtins.call_method1("tuple", (builtins.call_method1("sorted", (values,))?,))
}
#[pyfunction]
fn controller_install_review_state(manager: &Bound<'_, PyAny>, root: &str) -> PyResult<Py<PyAny>> {
    let py = manager.py();
    let prepared = manager.getattr("prepared")?;
    let environment = if prepared.is_truthy()? {
        prepared.getattr("environment")?
    } else {
        py.None().into_bound(py)
    };
    let choices = PyList::empty(py);
    for item in manager
        .getattr("choices")?
        .call_method0("items")?
        .try_iter()?
    {
        let item = item?;
        choices.append((item.get_item(0)?, sorted_tuple(py, &item.get_item(1)?)?))?;
    }
    let state = PyList::empty(py);
    state.append(py.import("builtins")?.call_method1("id", (prepared,))?)?;
    for name in ["profile_id", "target_id"] {
        state.append(manager.getattr(name)?)?;
    }
    state.append(root)?;
    state.append(manager.getattr("install_mode")?)?;
    state.append(sorted_tuple(py, choices.as_any())?)?;
    for name in ["install_mapping", "install_additions", "install_repairs"] {
        state.append(sorted_tuple(
            py,
            &manager.getattr(name)?.call_method0("items")?,
        )?)?;
    }
    if environment.is_truthy()? {
        let files = environment.getattr("files")?;
        let details = PyList::empty(py);
        for name in ["game_version", "fomm_version", "complete"] {
            details.append(environment.getattr(name)?)?;
        }
        details.append(py.import("builtins")?.call_method1("id", (&files,))?)?;
        details.append(if files.hasattr("revision")? {
            files.getattr("revision")?
        } else {
            sorted_tuple(py, &files.call_method0("items")?)?
        })?;
        details.append(sorted_tuple(
            py,
            &environment
                .getattr("extender_versions")?
                .call_method0("items")?,
        )?)?;
        state.append(details.to_tuple())?;
    } else {
        state.append(py.None())?;
    }
    Ok(state.to_tuple().into_any().unbind())
}
#[pyfunction]
fn controller_discard_install(manager: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = manager.py();
    for key in ["install_repairs", "install_additions"] {
        manager.setattr(key, PyDict::new(py))?;
    }
    for key in [
        "install_review_state",
        "install_review_plan",
        "installer_image",
    ] {
        manager.setattr(key, py.None())?;
    }
    manager.setattr("install_addition_pairs", PyList::empty(py))?;
    manager.setattr("installer_image_source", "")?;
    if manager.getattr("busy")?.is_truthy()? {
        return Err(invalid(
            "Cancel and wait for the current operation before closing the installer.",
        ));
    }
    let prepared = manager.getattr("prepared")?;
    if prepared.is_truthy()? {
        prepared.getattr("package")?.call_method0("close")?;
    }
    for key in ["prepared", "install_plan"] {
        manager.setattr(key, py.None())?;
    }
    manager.setattr("install_base_pairs", PyList::empty(py))?;
    for key in ["install_mapping", "choices"] {
        manager.setattr(key, PyDict::new(py))?;
    }
    manager.setattr("install_mode", "replace")?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(controller_review_install, m)?)?;
    m.add_function(wrap_pyfunction!(controller_install, m)?)?;
    m.add_function(wrap_pyfunction!(controller_fomod_policy, m)?)?;
    m.add_function(wrap_pyfunction!(controller_installer_image, m)?)?;
    m.add_function(wrap_pyfunction!(controller_prepare_install, m)?)?;
    m.add_function(wrap_pyfunction!(controller_install_review_state, m)?)?;
    m.add_function(wrap_pyfunction!(controller_discard_install, m)?)?;
    Ok(())
}

#[pyfunction]
fn controller_review_install(manager: &Bound<'_, PyAny>, root: &str) -> PyResult<()> {
    let py = manager.py();
    let prepared = manager.getattr("prepared")?;
    if !prepared.is_truthy()? {
        return Err(invalid("Prepare a package first."));
    }
    let library = manager.getattr("context_library")?.unbind();
    let profile = manager.getattr("profile_id")?.unbind();
    let choices = PyDict::new(py);
    for item in manager
        .getattr("choices")?
        .call_method0("items")?
        .try_iter()?
    {
        let item = item?;
        choices.set_item(
            item.get_item(0)?,
            py.import("builtins")?
                .call_method1("set", (item.get_item(1)?,))?,
        )?;
    }
    let mapping = manager
        .getattr("install_mapping")?
        .call_method0("copy")?
        .unbind();
    let additions = manager
        .getattr("install_additions")?
        .call_method0("copy")?
        .unbind();
    let repairs = manager
        .getattr("install_repairs")?
        .call_method0("copy")?
        .unbind();
    let mode: String = manager.getattr("install_mode")?.extract()?;
    let state = manager.call_method1("_install_review_state", (root,))?;
    let owner = manager.clone().unbind();
    let captured = prepared.clone().unbind();
    let frozen = state.clone().unbind();
    let root = root.to_string();
    let choices = choices.unbind();
    let work = callback(py, move |py, args, _| {
        let cancel = args.get_item(0)?;
        let prepared = captured.bind(py);
        let library = library.bind(py);
        let profile = profile.bind(py);
        prepared.call_method1("validate", (library, profile, &cancel))?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("repairs", repairs.bind(py))?;
        let base =
            prepared.call_method("plan", (choices.bind(py), &root, &cancel), Some(&kwargs))?;
        let installation = module(py, "installation")?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("allow_empty", additions.bind(py).is_truthy()?)?;
        let mapped = installation
            .getattr("remap")?
            .call((&base, mapping.bind(py), &cancel), Some(&kwargs))?;
        let package = prepared.getattr("package")?;
        let plan = installation.call_method1(
            "include_sources",
            (&package, mapped, additions.bind(py), &cancel),
        )?;
        package.call_method1("validate_sources", (plan.call_method0("values")?, &cancel))?;
        if !frozen.bind(py).eq(owner
            .bind(py)
            .call_method1("_install_review_state", (&root,))?)?
        {
            return Err(invalid(
                "Installer configuration changed during file review. Review again.",
            ));
        }
        let replacement = prepared.getattr("replacement_review")?;
        let review = if replacement.is_truthy()? {
            library.call_method1("review_owned", (replacement.getattr("mod_id")?, &cancel))?
        } else {
            py.None().into_bound(py)
        };
        let pairs = if review.is_truthy()? && mode == "merge" {
            let base_library = py
                .import("builtins")?
                .call_method1("getattr", (library, "base", library))?;
            installation.call_method1(
                "merge_preview",
                (base_library, &review, package, &plan, &cancel),
            )?
        } else {
            py.import("builtins")?
                .call_method1("list", (plan.call_method0("items")?,))?
        };
        let base = py
            .import("builtins")?
            .call_method1("list", (base.call_method0("items")?,))?;
        Ok(pyo3::types::PyTuple::new(py, [plan, review, base, pairs])?
            .into_any()
            .unbind())
    })?;
    let owner = manager.clone().unbind();
    let prepared = prepared.unbind();
    let state = state.unbind();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let result = args.get_item(0)?;
        let plan = result.get_item(0)?;
        let pairs = result.get_item(3)?;
        prepared
            .bind(py)
            .setattr("replacement_review", result.get_item(1)?)?;
        manager.setattr("install_plan", &plan)?;
        manager.setattr("install_review_plan", plan.call_method0("copy")?)?;
        manager.setattr("install_review_state", state.bind(py))?;
        manager.setattr("install_base_pairs", result.get_item(2)?)?;
        manager.setattr("install_pairs", &pairs)?;
        let count: String = py
            .import("builtins")?
            .call_method1("format", (pairs.len()?, ","))?
            .extract()?;
        manager.setattr(
            "status",
            format!("Reviewed {count} destination files. Ready to install."),
        )?;
        Ok(py.None())
    })?;
    manager.setattr("install_plan", py.None())?;
    manager.call_method1(
        "submit",
        (
            "Building installer file plan and owned-content review",
            work,
            complete,
        ),
    )?;
    Ok(())
}
#[pyfunction]
fn controller_install(manager: &Bound<'_, PyAny>, name: &str, root: &str) -> PyResult<()> {
    let py = manager.py();
    let prepared = manager.getattr("prepared")?;
    if !prepared.is_truthy()? || !manager.getattr("library")?.is_truthy()? {
        return Err(invalid("Prepare a package and open a library first."));
    }
    let plan = manager.getattr("install_plan")?;
    if plan.is_none() {
        return Err(invalid(
            "Review the file plan after changing installer choices.",
        ));
    }
    let reviewed = manager.getattr("install_review_state")?;
    let root = if root.is_empty() && reviewed.is_truthy()? {
        reviewed.get_item(3)?.extract::<String>()?
    } else {
        root.to_string()
    };
    if !reviewed.eq(manager.call_method1("_install_review_state", (&root,))?)?
        || !plan.eq(manager.getattr("install_review_plan")?)?
    {
        manager.setattr("install_plan", py.None())?;
        return Err(invalid(
            "Installer choices, environment, source repairs or destination edits changed after review. Review the files again.",
        ));
    }
    let library = manager.getattr("context_library")?;
    let profile = manager.getattr("profile_id")?;
    let plan = plan.call_method0("copy")?.unbind();
    let owner = manager.clone().unbind();
    let label_name = name.to_string();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        manager.setattr("selected_id", args.get_item(0)?)?;
        manager.call_method0("discard_install")?;
        manager.call_method0("refresh")?;
        manager.setattr("export_preview", py.None())?;
        manager.setattr(
            "status",
            format!("Installed {label_name} into your library."),
        )?;
        Ok(py.None())
    })?;
    let label = if prepared.getattr("replacement_review")?.is_truthy()? {
        if manager.getattr("install_mode")?.eq("merge")? {
            "Merging into owned mod with recoverable backup"
        } else {
            "Reinstalling owned mod with recoverable backup"
        }
    } else {
        "Installing mod"
    };
    let enabled = manager.getattr("enable_on_install")?.is_truthy()?;
    let owner = manager.clone().unbind();
    let prepared = prepared.unbind();
    let reviewed = reviewed.unbind();
    let library = library.unbind();
    let profile = profile.unbind();
    let name = name.to_string();
    let work = callback(py, move |py, args, _| {
        let cancel = args.get_item(0)?;
        let progress = args.get_item(1)?;
        prepared
            .bind(py)
            .call_method1("validate", (library.bind(py), profile.bind(py), &cancel))?;
        let validation_owner = owner.clone_ref(py);
        let validation_prepared = prepared.clone_ref(py);
        let validation_review = reviewed.clone_ref(py);
        let validation_library = library.clone_ref(py);
        let validation_profile = profile.clone_ref(py);
        let validation_cancel = cancel.clone().unbind();
        let validation_root = root.clone();
        let validation = callback(py, move |py, _, _| {
            if !validation_review.bind(py).eq(validation_owner
                .bind(py)
                .call_method1("_install_review_state", (&validation_root,))?)?
            {
                return Err(invalid(
                    "Installer configuration changed during staging. Review again; no owned destination was published.",
                ));
            }
            validation_prepared.bind(py).call_method1(
                "validate",
                (
                    validation_library.bind(py),
                    validation_profile.bind(py),
                    validation_cancel.bind(py),
                ),
            )?;
            Ok(py.None())
        })?;
        validation.bind(py).call0()?;
        let prepared = prepared.bind(py);
        let library = library.bind(py);
        let profile = profile.bind(py);
        let package = prepared.getattr("package")?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("nexus", prepared.getattr("nexus")?)?;
        let replacement = prepared.getattr("replacement_review")?;
        if replacement.is_truthy()? {
            kwargs.set_item("validate_before_commit", validation)?;
            if owner.bind(py).getattr("install_mode")?.eq("merge")? {
                let base = py
                    .import("builtins")?
                    .call_method1("getattr", (library, "base", library))?;
                return Ok(module(py, "installation")?
                    .getattr("merge")?
                    .call(
                        (base, replacement, package, plan.bind(py), cancel, progress),
                        Some(&kwargs),
                    )?
                    .unbind());
            }
            return Ok(library
                .call_method(
                    "reinstall",
                    (replacement, package, plan.bind(py), cancel, progress),
                    Some(&kwargs),
                )?
                .unbind());
        }
        let overlay = prepared.getattr("overlay_review")?;
        kwargs.set_item("enabled", enabled)?;
        if overlay.is_truthy()? {
            kwargs.set_item("validation", validation)?;
            Ok(module(py, "reference_overlay")?
                .getattr("install")?
                .call(
                    (
                        library,
                        overlay,
                        package,
                        plan.bind(py),
                        &name,
                        cancel,
                        progress,
                    ),
                    Some(&kwargs),
                )?
                .unbind())
        } else {
            kwargs.set_item("validate_before_commit", validation)?;
            Ok(library
                .call_method(
                    "install",
                    (package, plan.bind(py), &name, profile, cancel, progress),
                    Some(&kwargs),
                )?
                .unbind())
        }
    })?;
    manager.call_method1("submit", (label, work, complete))?;
    Ok(())
}
#[pyfunction]
fn controller_fomod_policy(manager: &Bound<'_, PyAny>, compatibility: &str) -> PyResult<()> {
    let py = manager.py();
    let prepared = manager.getattr("prepared")?;
    if !prepared.is_truthy()? || !prepared.getattr("fomod")?.is_truthy()? {
        return Err(invalid("Prepare a declarative FOMOD package first."));
    }
    let captured = prepared.clone().unbind();
    let compatibility = compatibility.to_string();
    let work = callback(py, move |py, args, _| {
        let cancel = args.get_item(0)?;
        check_cancel(Some(&cancel))?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("compatibility", &compatibility)?;
        let result = module(py, "fomod")?.getattr("Fomod")?.call(
            (captured
                .bind(py)
                .getattr("package")?
                .call_method0("read_config")?,),
            Some(&kwargs),
        )?;
        check_cancel(Some(&cancel))?;
        Ok(result.unbind())
    })?;
    let owner = manager.clone().unbind();
    let prepared = prepared.unbind();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let kwargs = PyDict::new(py);
        kwargs.set_item("fomod", args.get_item(0)?)?;
        manager.setattr(
            "prepared",
            py.import("dataclasses")?
                .getattr("replace")?
                .call((prepared.bind(py),), Some(&kwargs))?,
        )?;
        for name in [
            "choices",
            "install_mapping",
            "install_additions",
            "install_repairs",
        ] {
            manager.setattr(name, PyDict::new(py))?;
        }
        for name in ["install_review_state", "install_plan"] {
            manager.setattr(name, py.None())?;
        }
        for name in [
            "install_pairs",
            "install_base_pairs",
            "install_addition_pairs",
        ] {
            manager.setattr(name, PyList::empty(py))?;
        }
        manager.setattr("status", "Changed installer compatibility. Choices and destination edits reset; review the new file plan.")?;
        Ok(py.None())
    })?;
    manager.call_method1(
        "submit",
        ("Reparsing installer compatibility", work, complete),
    )?;
    Ok(())
}
#[pyfunction]
fn controller_installer_image(
    manager: &Bound<'_, PyAny>,
    source: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = manager.py();
    let prepared = manager.getattr("prepared")?;
    if !prepared.is_truthy()? {
        return Err(invalid("Prepare an installer package first."));
    }
    let captured_source = source.clone().unbind();
    let captured_prepared = prepared.unbind();
    let work = callback(py, move |py, args, _| {
        Ok(module(py, "installer_preview")?
            .call_method1(
                "image",
                (
                    captured_prepared.bind(py).getattr("package")?,
                    captured_source.bind(py),
                    args.get_item(0)?,
                ),
            )?
            .unbind())
    })?;
    let owner = manager.clone().unbind();
    let source = source.clone().unbind();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        manager.setattr("installer_image", args.get_item(0)?)?;
        manager.setattr("installer_image_source", source.bind(py))?;
        manager.setattr(
            "status",
            "Decoded bounded installer artwork. File choices are unchanged.",
        )?;
        Ok(py.None())
    })?;
    manager.call_method1("submit", ("Decoding installer artwork", work, complete))?;
    Ok(())
}
