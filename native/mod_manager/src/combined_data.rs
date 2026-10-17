use crate::{
    archive_index::{
        effective_ini, ini_settings, plugin_signature, profile_signature, source_stamp,
    },
    check_cancel,
    conflicts::conflict_rows_signature,
    invalid,
    plugin_inventory::value,
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
#[pyfunction]
fn combined_candidates(entry: &Bound<'_, PyAny>, kind: &str) -> PyResult<Py<PyAny>> {
    let py = entry.py();
    let class = py
        .import("creation_lib.mod_manager.combined_data")?
        .getattr("Candidate")?;
    let loose = entry.getattr("loose")?;
    let mut result = Vec::new();
    if loose.is_truthy()? {
        for provider in loose
            .getattr(if kind == "providers" {
                "providers"
            } else {
                "active"
            })?
            .try_iter()?
        {
            let provider = provider?;
            result.push(if kind == "providers" {
                provider
            } else {
                class.call1(("loose", provider))?
            });
        }
    }
    if kind != "candidates" {
        for archive in entry.getattr("archives")?.try_iter()? {
            let archive = archive?;
            if kind == "providers" {
                result.push(archive.getattr("provider")?);
            } else if archive.getattr("selected")?.is_truthy()?
                && ["listed", "plugin candidate"]
                    .contains(&archive.getattr("activation")?.extract::<String>()?.as_str())
            {
                result.push(class.call1(("archive", archive.getattr("provider")?, archive))?);
            }
        }
    }
    Ok(PyTuple::new(py, result)?.into_any().unbind())
}
#[pyfunction]
fn combined_resolve(
    path: &str,
    loose: &Bound<'_, PyAny>,
    archives: &Bound<'_, PyAny>,
    settings: &Bound<'_, PyAny>,
    game: &str,
) -> PyResult<Py<PyAny>> {
    let py = loose.py();
    let api = py.import("creation_lib.mod_manager.combined_data")?;
    let mut active = Vec::new();
    for archive in archives.try_iter()? {
        let archive = archive?;
        if archive.getattr("selected")?.is_truthy()?
            && ["listed", "plugin candidate"]
                .contains(&archive.getattr("activation")?.extract::<String>()?.as_str())
        {
            active.push(archive);
        }
    }
    let loose_winner = if loose.is_truthy()? {
        loose.getattr("winner")?
    } else {
        py.None().into_bound(py)
    };
    let mut winner = py.None().into_bound(py);
    let reason;
    if active.is_empty() {
        if loose_winner.is_truthy()? {
            winner = api.getattr("Candidate")?.call1(("loose", loose_winner))?;
            reason = "Loose provider order; no active archive candidates";
        } else {
            reason = "No active providers";
        }
    } else if game != "fo4" {
        reason = "Combined precedence adapter is unavailable for this game";
    } else if loose_winner.is_truthy()? {
        if settings
            .call_method1("get", ("binvalidateolderfiles",))?
            .eq("1")?
            && settings
                .call_method1("get", ("sresourcedatadirsfinal",))?
                .eq("")?
        {
            winner = api.getattr("Candidate")?.call1(("loose", loose_winner))?;
            reason = "FO4 loose override predicted from bInvalidateOlderFiles=1 and empty sResourceDataDirsFinal";
        } else {
            reason = "Loose/archive precedence is unresolved: profile does not explicitly enable unrestricted loose overrides";
        }
    } else {
        let mut ranked = Vec::new();
        let mut nonstandard = false;
        for archive in active {
            let name = fold(&archive.getattr("name")?.extract::<String>()?);
            nonstandard |= archive.getattr("activation")?.eq("plugin candidate")?
                && ![" - main.ba2", " - textures.ba2"]
                    .iter()
                    .any(|s| name.ends_with(s));
            ranked.push((archive.getattr("rank")?.extract::<i64>()?, archive));
        }
        ranked.sort_by_key(|v| v.0);
        let last = ranked.last().unwrap();
        if nonstandard || ranked[..ranked.len() - 1].iter().any(|v| v.0 == last.0) {
            reason = "Archive attachment/order is unresolved: equal load ranks or nonstandard plugin archive names";
        } else {
            winner = api.getattr("Candidate")?.call1((
                "archive",
                last.1.getattr("provider")?,
                &last.1,
            ))?;
            reason = "Archive winner predicted from INI list order, then active plugin order; engine attachment remains unverified";
        }
    }
    Ok(api
        .getattr("Entry")?
        .call1((path, loose, archives, winner, reason))?
        .unbind())
}
#[pyfunction]
fn combined_build(
    library: &Bound<'_, PyAny>,
    profile_id: &str,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
    reader: &Bound<'_, PyAny>,
    resolver: &Bound<'_, PyAny>,
    loose: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = library.py();
    let api = py.import("creation_lib.mod_manager.combined_data")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("loose", loose)?;
    let archives = py
        .import("creation_lib.mod_manager.archive_index")?
        .getattr("build")?
        .call(
            (library, profile_id, cancel, progress, reader),
            Some(&kwargs),
        )?;
    let loose = if loose.is_truthy()? {
        loose.clone()
    } else {
        let kwargs = PyDict::new(py);
        kwargs.set_item("build_tree", false)?;
        py.import("creation_lib.mod_manager.conflicts")?
            .getattr("build_index")?
            .call((library, profile_id, cancel, progress), Some(&kwargs))?
    };
    let snapshot = library.call_method1("profile_snapshot", (profile_id,))?;
    let profile = snapshot.get_item(0)?;
    let signature = conflict_rows_signature(&snapshot.get_item(1)?)?;
    if !archives
        .getattr("signature")?
        .get_item(0)?
        .eq(signature.bind(py))?
        || !loose.getattr("signature")?.eq(signature.bind(py))?
    {
        return Err(invalid(
            "Profile changed between loose and archive inventories. Refresh combined Data.",
        ));
    }
    let effective = effective_ini(library, &profile, None)?;
    if effective != value(&archives.getattr("effective_ini")?)? {
        return Err(invalid(
            "Runtime INI settings changed between archive and loose inventories. Refresh combined Data.",
        ));
    }
    let settings = ini_settings(py, &effective)?;
    let domain = py.import("creation_lib.mod_manager.nexus")?.call_method1(
        "game_domain",
        (profile
            .getattr("files")?
            .call_method1("get", ("_modbox_game", ""))?,),
    )?;
    let game = py
        .import("creation_lib.mod_manager.plugin_state")?
        .getattr("DOMAINS")?
        .call_method1("get", (domain, ""))?;
    let mut by_path = BTreeMap::new();
    for entry in loose.getattr("files")?.try_iter()? {
        let entry = entry?;
        let name = entry.getattr("path")?.extract::<String>()?;
        by_path.insert(fold(&name), (name, entry, PyTuple::empty(py).into_any()));
    }
    for member in archives.getattr("members")?.try_iter()? {
        check_cancel(Some(cancel))?;
        let member = member?;
        let name = member.getattr("path")?.extract::<String>()?;
        by_path
            .entry(fold(&name))
            .or_insert((
                name,
                py.None().into_bound(py),
                PyTuple::empty(py).into_any(),
            ))
            .2 = member.getattr("archives")?;
        if by_path.len() > 1_000_000 {
            return Err(invalid("Combined Data exceeds its 1,000,000-path limit."));
        }
    }
    let resolver = if resolver.is_none() {
        api.getattr("resolve")?
    } else {
        resolver.clone()
    };
    let kwargs = PyDict::new(py);
    kwargs.set_item("game", game)?;
    let mut entries = Vec::new();
    for (path, entry, members) in by_path.into_values() {
        check_cancel(Some(cancel))?;
        entries.push(resolver.call((path, entry, members, &settings), Some(&kwargs))?);
    }
    let entries = PyTuple::new(py, entries)?;
    let tree = py
        .import("creation_lib.mod_manager.data_tree")?
        .getattr("Tree")?
        .call1((&entries, cancel))?;
    let current = library.call_method1("profile_snapshot", (profile_id,))?;
    if !archives
        .getattr("signature")?
        .eq(profile_signature(&current)?.bind(py))?
    {
        return Err(invalid(
            "Profile changed during combined Data inspection. Refresh it.",
        ));
    }
    let names = effective
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    if effective != effective_ini(library, &current.get_item(0)?, Some(&names))? {
        return Err(invalid(
            "Runtime INI settings changed during combined Data inspection.",
        ));
    }
    let catalog = py
        .import("creation_lib.mod_manager.plugin_state")?
        .call_method1("inventory", (library, profile_id, cancel))?;
    if !plugin_signature(&catalog)?
        .bind(py)
        .eq(archives.getattr("plugin_signature")?)?
    {
        return Err(invalid(
            "Plugin providers or activation changed during combined Data inspection. Refresh it.",
        ));
    }
    for entry in entries.iter() {
        check_cancel(Some(cancel))?;
        let loose = entry.getattr("loose")?;
        if loose.is_truthy()? {
            for provider in loose.getattr("providers")?.try_iter()? {
                let p = provider?;
                if source_stamp(py, &p.getattr("path")?.extract::<PathBuf>()?)?
                    != (
                        p.getattr("size")?.extract()?,
                        p.getattr("modified")?.extract()?,
                    )
                {
                    return Err(invalid(
                        "Loose source changed during combined Data inspection. Refresh it.",
                    ));
                }
            }
        }
    }
    for archive in archives.getattr("archives")?.try_iter()? {
        check_cancel(Some(cancel))?;
        let archive = archive?;
        if archive.getattr("selected")?.is_truthy()? {
            let p = archive.getattr("provider")?;
            if source_stamp(py, &p.getattr("path")?.extract::<PathBuf>()?)?
                != (
                    p.getattr("size")?.extract()?,
                    p.getattr("modified")?.extract()?,
                )
            {
                return Err(invalid(
                    "Archive source changed during combined Data inspection. Refresh it.",
                ));
            }
        }
    }
    check_cancel(Some(cancel))?;
    let mut issues = Vec::new();
    let mut seen = BTreeSet::new();
    for source in [&loose, &archives] {
        for issue in source.getattr("issues")?.try_iter()? {
            let issue = issue?.extract::<String>()?;
            if seen.insert(issue.clone()) {
                issues.push(issue);
            }
        }
    }
    let notice =
        "Combined winners are metadata predictions; no game has been launched.".to_string();
    if seen.insert(notice.clone()) {
        issues.push(notice);
    }
    Ok(api
        .getattr("Index")?
        .call1((
            profile_id,
            entries,
            loose,
            archives,
            settings,
            PyTuple::new(py, issues)?,
            tree,
        ))?
        .unbind())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(combined_candidates, m)?)?;
    m.add_function(wrap_pyfunction!(combined_resolve, m)?)?;
    m.add_function(wrap_pyfunction!(combined_build, m)?)?;
    Ok(())
}
