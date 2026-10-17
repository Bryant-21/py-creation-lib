use crate::{
    check_cancel,
    history_prune::py_path,
    invalid, reject_links, relative_path,
    runtime::{local, module},
    saves::fold,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
fn tuple<'py>(value: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    value
        .py()
        .import("builtins")?
        .getattr("tuple")?
        .call1((value,))
}
fn resolved(path: &Bound<'_, PyAny>) -> PyResult<PathBuf> {
    path.call_method0("resolve")?.extract()
}
fn fingerprint<'py>(py: Python<'py>, path: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    py.import("creation_lib.mod_manager.launcher")?
        .call_method1("fingerprint", (path,))
}
fn tree<'py>(
    py: Python<'py>,
    path: &Bound<'py, PyAny>,
    cancel: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    py.import("creation_lib.mod_manager.paths")?
        .call_method1("tree_files", (path, cancel))
}
fn textpath(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
fn filtered_plugin_snapshot(
    name: &str,
    text: &str,
    winners: &Bound<'_, PyDict>,
    game: &Path,
    overwrite: &Path,
) -> PyResult<String> {
    let mut filtered = String::new();
    for line in text.split_inclusive('\n') {
        let entry = line.trim().trim_start_matches('\u{feff}');
        if entry.is_empty() || entry.starts_with('#') {
            filtered.push_str(line);
            continue;
        }
        let entry = if name == "lockedorder.txt" {
            entry.split('|').next().unwrap_or("").trim()
        } else {
            entry.strip_prefix('*').unwrap_or(entry).trim()
        };
        let plugin = crate::plugin_state::plugin_name(entry)?;
        let base = game.join("Data").join(&plugin);
        let owned = overwrite.join(&plugin);
        reject_links(&base)?;
        reject_links(&owned)?;
        if winners.contains(fold(&plugin))? || base.is_file() || owned.is_file() {
            filtered.push_str(line);
        }
    }
    Ok(filtered)
}
#[pyfunction]
fn runtime_prepare(
    library: &Bound<'_, PyAny>,
    profile_id: &str,
    executable: &Bound<'_, PyAny>,
    game_root: &Bound<'_, PyAny>,
    bundle_root: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
    user_targets: &Bound<'_, PyAny>,
    isolate_documents: Option<bool>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let runtime = module(py)?;
    let isolate_documents = match isolate_documents {
        Some(value) => value,
        None => py
            .import("creation_lib.mod_manager.profile_state")?
            .call_method1("isolate_documents", (library, profile_id))?
            .extract()?,
    };
    let launcher = py.import("creation_lib.mod_manager.launcher")?;
    if !cfg!(all(windows, target_pointer_width = "64")) {
        return Err(invalid(
            "The project filespace adapter requires x64 Windows.",
        ));
    }
    let sys = py.import("sys")?;
    if sys.hasattr("frozen")?
        && sys.getattr("frozen")?.is_truthy()?
        && py_path(py, &sys.getattr("_MEIPASS")?.extract::<PathBuf>()?)?
            .getattr("name")?
            .extract::<String>()?
            != "_internal"
    {
        return Err(invalid(
            "Native runtime requires the application's onedir package; onefile helper identity is not supported.",
        ));
    }
    let managed_runtime = bundle_root.str()?.to_str()?.trim().is_empty();
    let record = executable.call_method0("record")?;
    let executable = launcher
        .getattr("definition")?
        .call((), Some(record.cast::<PyDict>()?))?;
    if !matches!(
        executable
            .getattr("steam_app_id")?
            .extract::<String>()?
            .as_str(),
        "" | "377160"
    ) {
        return Err(invalid(
            "This Fallout 4 adapter requires Steam application ID 377160.",
        ));
    }
    let binary = local(py, &executable.getattr("binary")?, "Executable", false)?;
    let binary_name = fold(&binary.getattr("name")?.extract::<String>()?);
    if !matches!(binary_name.as_str(), "fallout4.exe" | "f4se_loader.exe")
        || runtime
            .call_method1("pe_machine", (&binary,))?
            .extract::<u32>()?
            != 0x8664
    {
        return Err(invalid(
            "The initial modded runtime supports x64 Fallout4.exe and f4se_loader.exe. Imported editors and generators remain blocked because they can edit referenced source files.",
        ));
    }
    let staged = library.hasattr("target_id")?
        && library.getattr("target_id")?.is_truthy()?
        && library
            .call_method0("target")?
            .getattr("f4se")?
            .call_method1("get", ("managed",))?
            .is_truthy()?;
    let selected_game = if staged {
        game_root.clone()
    } else {
        binary.getattr("parent")?
    };
    let game = local(py, &selected_game, "Fallout 4 game directory", true)?;
    let game_path: PathBuf = game.extract()?;
    let library_root: PathBuf = library.getattr("root")?.extract()?;
    let lock = py
        .import("creation_lib.mod_manager.f4se_root_setup")?
        .getattr("LOCK")?
        .extract::<String>()?;
    reject_links(&game_path.join(&lock))?;
    if game_path.join(lock).exists() {
        return Err(invalid(
            "Game-root setup/recovery is unfinished. Review restore before launching any profile.",
        ));
    }
    let target_id = if library.hasattr("target_id")? {
        library.getattr("target_id")?.extract::<String>()?
    } else {
        String::new()
    };
    let mut launch_target = if !target_id.is_empty() {
        library.call_method0("target")?
    } else {
        py.None().into_bound(py)
    };
    let mut f4se = PyDict::new(py).into_any();
    if launch_target.is_truthy()? {
        let setup = py.import("creation_lib.mod_manager.f4se_setup")?;
        if !executable
            .getattr("id")?
            .eq(launch_target.getattr("executable")?.getattr("id")?)?
        {
            return Err(invalid(
                "Use this target's F4SE launcher, or select Original executables to run an imported definition.",
            ));
        }
        if resolved(&game)?
            != resolved(&py_path(
                py,
                &launch_target.getattr("game_root")?.extract::<PathBuf>()?,
            )?)?
        {
            return Err(invalid("Launch target game folder differs."));
        }
        let managed = launch_target
            .getattr("f4se")?
            .call_method1("get", ("managed",))?
            .is_truthy()?;
        if managed
            && resolved(&binary)?
                != resolved(&py_path(
                    py,
                    &launch_target.getattr("binary")?.extract::<PathBuf>()?,
                )?)?
        {
            return Err(invalid(
                "The selected staged executable differs from its explicit target receipt.",
            ));
        }
        if binary_name == "f4se_loader.exe" && !managed {
            let kwargs = PyDict::new(py);
            kwargs.set_item("binary", binary.str()?)?;
            launch_target = py.import("dataclasses")?.call_method(
                "replace",
                (launch_target,),
                Some(&kwargs),
            )?;
        }
        let kwargs = PyDict::new(py);
        kwargs.set_item("owner_root", library.getattr("root")?)?;
        f4se = setup.call_method("validate_launch", (&launch_target, cancel), Some(&kwargs))?;
    }
    let managed = f4se.call_method1("get", ("managed",))?.is_truthy()?;
    if resolved(&binary.getattr("parent")?)? != resolved(&game)? && !managed {
        return Err(invalid(
            "Executable and selected Fallout 4 game directory differ. Correct the executable or game path before launch.",
        ));
    }
    local(
        py,
        &py_path(py, &game_path.join("Data"))?,
        "Game Data directory",
        true,
    )?;
    let runtime_bundle = if managed_runtime {
        py.import("creation_lib.mod_manager.native_builder")?
            .call_method1("ensure", (cancel, progress))?
    } else {
        runtime.call_method1("bundle", (bundle_root,))?
    };
    let working = executable.getattr("working_directory")?;
    let cwd = local(
        py,
        &if working.is_truthy()? {
            working
        } else {
            binary.getattr("parent")?
        },
        "Working directory",
        true,
    )?;
    let mut argv = vec![binary.str()?.to_str()?.to_owned()];
    argv.extend(
        launcher
            .call_method1("windows_arguments", (executable.getattr("arguments")?,))?
            .extract::<Vec<String>>()?,
    );
    let targets = PyList::empty(py);
    let target_exe = py_path(py, &game_path.join("Fallout4.exe"))?;
    targets.append((target_exe.str()?, fingerprint(py, &target_exe)?))?;
    if binary_name == "f4se_loader.exe" {
        if argv.iter().skip(1).any(|s| {
            matches!(
                fold(s)
                    .split('=')
                    .next()
                    .unwrap_or("")
                    .split(':')
                    .next()
                    .unwrap_or(""),
                "-altexe" | "-altdll" | "-noskiplauncher" | "-launchsteam" | "-editor"
            )
        }) {
            return Err(invalid(
                "F4SE runtime/launcher overrides are unsupported by this bounded adapter; remove them before launch.",
            ));
        }
        let target = local(py, &target_exe, "F4SE target executable", false)?;
        if runtime
            .call_method1("pe_machine", (&target,))?
            .extract::<u32>()?
            != 0x8664
        {
            return Err(invalid("F4SE's Fallout4.exe target must be x64."));
        }
        targets.append((target.str()?, fingerprint(py, &target)?))?;
        if managed && !f4se.call_method1("get", ("route",))?.eq("game-root")? {
            let name = py
                .import("creation_lib.mod_manager.f4se_setup")?
                .call_method1("runtime_name", (f4se.get_item("build")?,))?
                .extract::<String>()?;
            argv.extend([
                "-altexe".to_owned(),
                target.str()?.to_str()?.to_owned(),
                "-altdll".into(),
                textpath(&f4se.get_item("stage")?.extract::<PathBuf>()?.join(name)),
            ]);
        }
    }
    if py
        .import("subprocess")?
        .call_method1("list2cmdline", (&argv,))?
        .len()?
        > 32760
    {
        return Err(invalid(
            "The resulting Windows command line exceeds its size limit.",
        ));
    }
    if profile_id.is_empty() {
        return Err(invalid("Select a local profile before preparing launch."));
    }
    let snapshot = library.call_method1("profile_snapshot", (profile_id,))?;
    let profile = snapshot.get_item(0)?;
    let rows = snapshot.get_item(1)?;
    let files = profile.getattr("files")?;
    let game_name = files
        .call_method1("get", ("_modbox_game", ""))?
        .extract::<String>()?;
    if !game_name.is_empty()
        && !matches!(fold(&game_name).as_str(), "fallout 4" | "fallout4" | "fo4")
    {
        return Err(invalid(
            "The selected profile is for another game; this adapter is Fallout 4 only.",
        ));
    }
    let signature = runtime.call_method1("captured_signature", (&profile, &rows))?;
    let mut plugin_snapshot = false;
    for name in files.try_iter()? {
        if fold(&name?.extract::<String>()?) == "plugins.txt" {
            plugin_snapshot = true;
            break;
        }
    }
    if !plugin_snapshot {
        return Err(invalid(
            "This profile has no plugin activation snapshot. Initialize its local game instance/profile before modded launch; global plugin settings will not be used silently.",
        ));
    }
    let namespace = if target_id.is_empty() {
        profile_id.to_owned()
    } else {
        library
            .call_method1("save_namespace", (profile_id,))?
            .extract::<String>()?
    };
    let save_subpath = format!("__ModBox21Saves_{}", relative_path(&namespace, false)?);
    let winners = PyDict::new(py);
    for (position, row) in rows.try_iter()?.enumerate() {
        check_cancel(Some(cancel))?;
        let row = row?;
        if !row.getattr("enabled")?.is_truthy()? || row.getattr("kind")?.eq("separator")? {
            continue;
        }
        let source = local(
            py,
            &row.getattr("source")?,
            &format!("Enabled mod {}", row.getattr("name")?.str()?.to_str()?),
            true,
        )?;
        let source_resolved = resolved(&source)?;
        let game_resolved = resolved(&game)?;
        if source_resolved.starts_with(&game_resolved)
            || game_resolved.starts_with(&source_resolved)
        {
            return Err(invalid(
                "Game-directory content cannot be mapped as a managed mod.",
            ));
        }
        for item in tree(py, &source, cancel)?.try_iter()? {
            let item = item?;
            let name: String = item.get_item(0)?.extract()?;
            let path = item.get_item(1)?;
            let folded = fold(&name);
            if matches!(
                folded.as_str(),
                "meta.ini" | "plugins.txt" | "loadorder.txt" | "archives.txt" | "lockedorder.txt"
            ) || folded.starts_with("fomod/")
                || folded.split('/').any(|part| part.ends_with(".mohidden"))
            {
                continue;
            }
            winners.set_item(folded, (name, path.str()?, fingerprint(py, &path)?))?;
            if winners.len() > 250_000 {
                return Err(invalid(
                    "Runtime mapping exceeds the 250,000-file safety limit.",
                ));
            }
        }
        if progress.is_truthy()? {
            progress.call1((
                (position + 1) as f64 / rows.len()?.max(1) as f64,
                format!(
                    "Preparing mappings: {}",
                    row.getattr("name")?.str()?.to_str()?
                ),
            ))?;
        }
    }
    let names = PyList::empty(py);
    for item in winners.values().iter() {
        names.append(item.get_item(0)?)?;
    }
    py.import("creation_lib.mod_manager.paths")?
        .call_method1("validate_destinations", (names,))?;
    let overwrite = library_root
        .join("runtime")
        .join("profiles")
        .join(profile_id)
        .join("overwrite");
    py.import("creation_lib.mod_manager.plugin_state")?
        .call_method1(
            "validate_runtime",
            (
                &files,
                &winners,
                &game,
                py_path(py, &overwrite)?,
                snapshot
                    .get_item(2)?
                    .call_method1("get", ("game", PyDict::new(py)))?
                    .call_method1("get", ("gamepath", ""))?,
            ),
        )?;
    if binary_name == "f4se_loader.exe" {
        let configured = runtime.call_method1(
            "validate_f4se_configuration",
            (
                &game,
                library.getattr("root")?,
                profile_id,
                winners.values(),
            ),
        )?;
        if configured.is_truthy()? {
            targets.append((configured.str()?, fingerprint(py, &configured)?))?;
        }
    }
    let mut findings = PyTuple::empty(py).into_any();
    if launch_target.is_truthy()? {
        let effective = PyDict::new(py);
        for root in [game_path.join("Data"), overwrite.clone()] {
            let plugins = root.join("F4SE").join("Plugins");
            reject_links(&plugins)?;
            if plugins.is_dir() {
                for item in tree(py, &py_path(py, &plugins)?, cancel)?.try_iter()? {
                    let item = item?;
                    let name = item.get_item(0)?.extract::<String>()?;
                    let path = item.get_item(1)?;
                    let folded = fold(&name);
                    if folded.ends_with(".dll")
                        || (folded.starts_with("version-") && folded.ends_with(".bin"))
                    {
                        let name = format!("F4SE/Plugins/{name}");
                        effective
                            .set_item(fold(&name), (name, path.str()?, fingerprint(py, &path)?))?;
                        if effective.len() > 512 {
                            return Err(invalid("Native plugin inventory exceeds 512 DLLs."));
                        }
                    }
                }
            }
        }
        let base = PyDict::new(py);
        for (name, item) in effective.iter() {
            if !item
                .get_item(1)?
                .extract::<PathBuf>()?
                .starts_with(&library_root)
            {
                base.set_item(name, item)?;
            }
        }
        for (name, item) in winners.iter() {
            if name.extract::<String>()?.starts_with("f4se/plugins/") {
                base.set_item(name, item)?;
            }
        }
        for (name, item) in effective.iter() {
            if item
                .get_item(1)?
                .extract::<PathBuf>()?
                .starts_with(&library_root)
            {
                base.set_item(name, item)?;
            }
        }
        findings = py
            .import("creation_lib.mod_manager.f4se_setup")?
            .call_method1(
                "plugin_findings",
                (
                    base.values(),
                    f4se.get_item("build")?,
                    f4se.get_item("version")?,
                    cancel,
                ),
            )?;
        for item in base.values().iter() {
            targets.append((item.get_item(1)?, item.get_item(2)?))?;
        }
    }
    let user_targets = if user_targets.is_truthy()? {
        user_targets.clone()
    } else {
        runtime.call_method0("fo4_user_targets")?
    };
    let plugin_target = user_targets.get_item(0)?;
    let ini_target = user_targets.get_item(1)?;
    for target in [&plugin_target, &ini_target] {
        let target = py_path(py, &target.extract::<PathBuf>()?)?;
        let target_path: PathBuf = target.extract()?;
        if !target_path.is_absolute()
            || !runtime
                .call_method1("local_mod_target_for_user", (&target,))?
                .is_truthy()?
        {
            return Err(invalid(
                "Game user paths must be absolute local directories.",
            ));
        }
        reject_links(&target_path)?;
        let target_resolved = resolved(&target)?;
        if target_resolved.starts_with(resolved(&game)?)
            || target_resolved.starts_with(resolved(&library.getattr("root")?)?)
        {
            return Err(invalid(
                "Runtime user targets must be outside the game and manager library.",
            ));
        }
    }
    let snapshot_files = py.import("builtins")?.getattr("dict")?.call1((&files,))?;
    let minimal = (|| -> PyResult<Vec<String>> {
        let raw = files.call_method1("get", ("_modbox_minimal_inis", "[]"))?;
        let value = py.import("json")?.call_method1("loads", (raw,))?;
        if !value.is_instance_of::<PyList>() {
            return Err(invalid("Invalid minimal INI metadata"));
        }
        let values: Vec<String> = value.extract()?;
        if values.iter().any(|v| {
            !matches!(
                v.as_str(),
                "Fallout4.ini" | "Fallout4Prefs.ini" | "Fallout4Custom.ini"
            )
        }) {
            return Err(invalid("Invalid minimal INI metadata"));
        }
        Ok(values)
    })();
    let mut minimal = match minimal {
        Ok(v) => v,
        Err(e)
            if e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
                || e.is_instance_of::<pyo3::exceptions::PyTypeError>(py) =>
        {
            return Err(invalid("Invalid profile INI baseline metadata."));
        }
        Err(e) => return Err(e),
    };
    for name in ["Fallout4.ini", "Fallout4Prefs.ini", "Fallout4Custom.ini"] {
        let mut present = false;
        for key in snapshot_files.try_iter()? {
            if fold(&key?.extract::<String>()?) == fold(name) {
                present = true;
                break;
            }
        }
        if !present {
            snapshot_files.set_item(name, "[General]\n")?;
            minimal.push(name.to_owned());
        }
    }
    let snapshots = PyList::empty(py);
    for item in snapshot_files.call_method0("items")?.try_iter()? {
        let item = item?;
        let name = item.get_item(0)?.extract::<String>()?;
        let text = item.get_item(1)?;
        let folded = fold(&name);
        if matches!(
            folded.as_str(),
            "plugins.txt" | "loadorder.txt" | "lockedorder.txt"
        ) {
            let filtered = filtered_plugin_snapshot(
                &folded,
                &text.extract::<String>()?,
                &winners,
                &game_path,
                &overwrite,
            )?;
            snapshots.append((name, filtered, "plugins"))?;
        } else if matches!(
            folded.as_str(),
            "fallout4.ini" | "fallout4prefs.ini" | "fallout4custom.ini"
        ) {
            snapshots.append((
                &name,
                py.import("creation_lib.mod_manager.profile_state")?
                    .call_method1(
                        "fo4_runtime_ini",
                        (
                            &name,
                            text,
                            if isolate_documents {
                                namespace.as_str()
                            } else {
                                ""
                            },
                        ),
                    )?,
                "ini",
            ))?;
        }
    }
    check_cancel(Some(cancel))?;
    if !runtime
        .call_method1("profile_signature", (library, profile_id))?
        .eq(&signature)?
    {
        return Err(invalid(
            "Profile changed while preparing runtime mappings; review it again.",
        ));
    }
    let backend = if managed_runtime {
        "modbox-vfs-fo4-x64"
    } else {
        "usvfs-fo4-x64"
    };
    let launch = launcher.getattr("LaunchPlan")?.call1((
        executable,
        PyTuple::new(py, argv)?,
        cwd.str()?,
        fingerprint(py, &binary)?,
        profile.getattr("id")?,
        profile.getattr("name")?,
        backend,
    ))?;
    let save_target = ini_target.extract::<PathBuf>()?.join(&save_subpath);
    reject_links(&save_target)?;
    if isolate_documents && save_target.exists() {
        return Err(invalid(
            "The reserved virtual save directory already exists physically. Choose a clean user-directory configuration before launching.",
        ));
    }
    let external = managed && !f4se.call_method1("get", ("route",))?.eq("game-root")?;
    let root_files = if external {
        let list = PyList::empty(py);
        for item in f4se
            .call_method1("get", ("root_files", PyList::empty(py)))?
            .try_iter()?
        {
            list.append(tuple(&item?)?)?;
        }
        tuple(list.as_any())?
    } else {
        PyTuple::empty(py).into_any()
    };
    let mut protected = BTreeSet::from([
        game.str()?.to_str()?.to_owned(),
        plugin_target.str()?.to_str()?.to_owned(),
    ]);
    if isolate_documents {
        protected.insert(ini_target.str()?.to_str()?.to_owned());
    }
    for row in rows.try_iter()? {
        let row = row?;
        let source = row.getattr("source")?;
        if source.is_truthy()?
            && !matches!(
                row.getattr("kind")?.extract::<String>()?.as_str(),
                "separator" | "foreign"
            )
        {
            protected.insert(source.extract::<String>()?);
        }
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item(
        "metadata_root",
        if launch_target.is_truthy()? {
            library
                .getattr("base")?
                .getattr("root")?
                .str()?
                .to_str()?
                .to_owned()
        } else {
            String::new()
        },
    )?;
    kwargs.set_item(
        "target_id",
        if launch_target.is_truthy()? {
            target_id
        } else {
            String::new()
        },
    )?;
    kwargs.set_item("managed_runtime", managed_runtime)?;
    kwargs.set_item("runtime", backend)?;
    kwargs.set_item("protected_roots", PyTuple::new(py, protected)?)?;
    kwargs.set_item("root_files", root_files)?;
    kwargs.set_item("compatibility", findings)?;
    kwargs.set_item("experimental", external)?;
    let saves = if launch_target.is_truthy()? {
        library
            .call_method1("owned_save_root", (profile_id,))?
            .str()?
            .to_str()?
            .to_owned()
    } else {
        textpath(
            &library_root
                .join("runtime")
                .join("profiles")
                .join(profile_id)
                .join("saves"),
        )
    };
    kwargs.set_item("save_root", saves)?;
    kwargs.set_item("isolate_documents", isolate_documents)?;
    kwargs.set_item("session_id", uuid::Uuid::new_v4().simple().to_string())?;
    Ok(runtime
        .getattr("RuntimePlan")?
        .call(
            (
                launch,
                textpath(&library_root),
                signature,
                game.str()?,
                runtime_bundle.str()?,
                tuple(winners.values().as_any())?,
                tuple(snapshots.as_any())?,
                plugin_target.str()?,
                ini_target.str()?,
                save_subpath,
                PyTuple::new(py, minimal)?,
                tuple(targets.as_any())?,
            ),
            Some(&kwargs),
        )?
        .unbind())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(runtime_prepare, m)?)?;
    Ok(())
}
