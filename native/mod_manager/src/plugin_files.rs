use crate::{
    check_cancel, encoding,
    executables::text,
    history_prune::{py_path, stamp},
    invalid,
    plugin_inventory::value,
    plugin_state::{lines, plugin_name, snapshot},
    python_io::with_context,
    reject_links,
    saves::fold,
    transactions::transaction_key,
};
use pyo3::{prelude::*, types::PyDict};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn digest(v: &Value) -> String {
    encoding::digest(v, false, false)
}
fn file(py: Python<'_>, path: &Path, cancel: &Bound<'_, PyAny>) -> PyResult<Value> {
    reject_links(path)?;
    let target = py_path(py, path)?;
    let before = stamp(&target.call_method0("stat")?)?;
    if !path.is_file() || before.2 > 512 * 1024 * 1024 {
        return Err(invalid(
            "Owned plugin action requires an ordinary file of at most 512 MiB.",
        ));
    }
    let hash = with_context(&target.call_method1("open", ("rb",))?, |stream| {
        let mut hash = Sha256::new();
        loop {
            let bytes: Vec<u8> = stream.call_method1("read", (1024 * 1024,))?.extract()?;
            if bytes.is_empty() {
                break;
            }
            check_cancel(Some(cancel))?;
            hash.update(bytes);
        }
        Ok(format!("{:x}", hash.finalize()))
    })?;
    let after = stamp(&target.call_method0("stat")?)?;
    if (before.1, before.2, before.3) != (after.1, after.2, after.3) {
        return Err(invalid("Owned plugin changed during review."));
    }
    Ok(json!({"size":after.2,"sha256":hash}))
}
fn rows(db: &Bound<'_, PyAny>, sql: &str) -> PyResult<Value> {
    let mut rows = Vec::new();
    for row in db.call_method1("execute", (sql,))?.try_iter()? {
        rows.push(value(
            &db.py()
                .import("builtins")?
                .getattr("dict")?
                .call1((row?,))?,
        )?);
    }
    Ok(json!(rows))
}
fn state(db: &Bound<'_, PyAny>) -> PyResult<Value> {
    let mut result = json!({});
    for table in ["mods", "entries", "profiles", "instances"] {
        result[table] = rows(db, &format!("SELECT * FROM {table} ORDER BY 1,2"))?;
    }
    if !db
        .call_method1(
            "execute",
            ("SELECT 1 FROM sqlite_master WHERE name='launch_target_profiles'",),
        )?
        .call_method0("fetchone")?
        .is_none()
    {
        result["launch_target_profiles"] = rows(
            db,
            "SELECT * FROM launch_target_profiles ORDER BY target_id,profile_id",
        )?;
    }
    if encoding::encode(&result, true, true).len() > 64 * 1024 * 1024 {
        return Err(invalid("Owned plugin metadata review exceeds 64 MiB."));
    }
    Ok(result)
}
fn absolute(py: Python<'_>, path: &Path) -> PyResult<PathBuf> {
    py_path(py, path)?.call_method0("absolute")?.extract()
}
fn owned(library: &Bound<'_, PyAny>, mod_id: &str, source: &Path) -> PyResult<bool> {
    with_context(&library.call_method0("_db")?, |db| {
        let row = db
            .call_method1(
                "execute",
                ("SELECT source,kind FROM mods WHERE id=?", (mod_id,)),
            )?
            .call_method0("fetchone")?;
        Ok(!row.is_none()
            && row.get_item("kind")?.extract::<String>()? == "owned"
            && absolute(library.py(), &row.get_item("source")?.extract::<PathBuf>()?)?
                == absolute(library.py(), source)?)
    })
}
fn unavailable(library: &Bound<'_, PyAny>, catalog: &Value, plugin: &Value) -> PyResult<String> {
    if plugin.is_null()
        || text(plugin, "path").is_empty()
        || plugin["forced"] == true
        || text(plugin, "mod_id").is_empty()
    {
        return Ok("Select an ordinary manager-owned plugin; game masters and overwrite files cannot be changed here.".into());
    }
    let root = library
        .getattr("root")?
        .extract::<PathBuf>()?
        .join("mods")
        .join(transaction_key(text(plugin, "mod_id"))?);
    if !owned(library, text(plugin, "mod_id"), &root)?
        || absolute(library.py(), Path::new(text(plugin, "path")))?.parent()
            != Some(absolute(library.py(), &root)?.as_path())
    {
        return Ok("Referenced MO2/source plugins are read-only. These controls require a canonical manager-owned file.".into());
    }
    Ok(if text(catalog, "game") != "fo4" {
        "Owned plugin file actions currently cover the Fallout 4 adapter only."
    } else {
        ""
    }
    .into())
}
fn sidecars(root: &Path, name: &str) -> PyResult<()> {
    let stem = fold(
        &Path::new(name)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy(),
    );
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        let suffix = fold(&path.extension().unwrap_or_default().to_string_lossy());
        let other = fold(&path.file_stem().unwrap_or_default().to_string_lossy());
        if path.is_file()
            && ["ini", "ba2", "bsa"].contains(&suffix.as_str())
            && (other == stem || other.starts_with(&format!("{stem} - ")))
        {
            return Err(invalid(
                "Matching INI/archive sidecars require a reviewed multi-file rename; this action is disabled.",
            ));
        }
    }
    Ok(())
}
#[pyfunction]
fn plugin_file_review(
    library: &Bound<'_, PyAny>,
    catalog: &str,
    plugin: &str,
    action: &str,
    new_name: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<String> {
    library.call_method0("_runtime_write_guard")?;
    let catalog = encoding::parse(catalog)?;
    let plugin = encoding::parse(plugin)?;
    let reason = unavailable(library, &catalog, &plugin)?;
    if !reason.is_empty() {
        return Err(invalid(reason));
    }
    if !["rename", "ghost", "unghost", "esm", "no-esm"].contains(&action) {
        return Err(invalid(
            "Unsupported owned plugin action. ESL conversion and record compaction require a separate authoring review.",
        ));
    }
    let old = Path::new(text(&plugin, "path"))
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let name = text(&plugin, "name");
    let new = match action {
        "rename" => {
            let new = plugin_name(new_name)?;
            if fold(
                &Path::new(&new)
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy(),
            ) != fold(
                &Path::new(name)
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy(),
            ) || fold(&new) == fold(name)
            {
                return Err(invalid(
                    "Rename preserves the plugin extension and requires a different Windows filename.",
                ));
            }
            new
        }
        "ghost" => format!("{name}.ghost"),
        "unghost" => name.to_string(),
        _ => old.clone(),
    };
    let root = library
        .getattr("root")?
        .extract::<PathBuf>()?
        .join("mods")
        .join(transaction_key(text(&plugin, "mod_id"))?);
    reject_links(&root.join(&new))?;
    if new != old && root.join(&new).exists() {
        return Err(invalid(
            "Owned plugin destination already exists; files are never replaced by rename/ghost.",
        ));
    }
    if (action == "ghost" && fold(&old).ends_with(".ghost"))
        || (action == "unghost" && !fold(&old).ends_with(".ghost"))
    {
        return Err(invalid(
            "Plugin ghost state changed. Refresh this inventory.",
        ));
    }
    if ["esm", "no-esm"].contains(&action)
        && (!fold(&old).ends_with(".esp")
            || plugin["header"].as_object().is_none_or(|v| v.is_empty())
            || plugin["header"]["flags"]["esl"] == true)
    {
        return Err(invalid(
            "Master flag editing is limited to inspected, unghosted, non-light FO4 ESP files.",
        ));
    }
    let state = with_context(&library.call_method0("_db")?, state)?;
    for row in state["profiles"].as_array().into_iter().flatten().chain(
        state["launch_target_profiles"]
            .as_array()
            .into_iter()
            .flatten(),
    ) {
        let files = encoding::parse(text(row, "files"))?;
        if lines(&snapshot(&files, "plugins.txt"), true)?
            .iter()
            .any(|(n, a)| *a && fold(n) == fold(name))
        {
            return Err(invalid(
                "Deactivate this plugin in every local profile and launch target before changing its owned file.",
            ));
        }
    }
    if state["entries"].as_array().into_iter().flatten().any(|r| {
        r["mod_id"] == plugin["mod_id"]
            && r["enabled"].as_i64().unwrap_or(0) != 0
            && r["profile_id"] != catalog["profile_id"]
    }) {
        return Err(invalid(
            "This owned provider is enabled in another profile. Cross-profile plugin refactoring is unsupported; disable that provider there first.",
        ));
    }
    let mut inputs = Vec::new();
    for other in catalog["plugins"].as_array().into_iter().flatten() {
        check_cancel(Some(cancel))?;
        if !text(other, "path").is_empty() {
            if other["header"].is_null() || other["masters"].is_null() {
                return Err(invalid(
                    "Inspect all plugin details before reviewing an owned file action.",
                ));
            }
            let path = Path::new(text(other, "path"));
            reject_links(path)?;
            let info = stamp(&py_path(library.py(), path)?.call_method0("stat")?)?;
            if json!([info.2, info.3]) != other["fingerprint"] {
                return Err(invalid(
                    "Plugin inventory changed. Refresh details before this action.",
                ));
            }
            inputs.push(json!([path, info.2, info.3]));
        }
        if ["rename", "ghost"].contains(&action)
            && fold(text(other, "name")) != fold(name)
            && other["masters"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|m| fold(m.as_str().unwrap_or("")) == fold(name))
        {
            return Err(invalid(
                "Another available plugin names this master. Automatic dependent/master-reference rewriting is unsupported.",
            ));
        }
        if action == "rename" && fold(text(other, "name")) == fold(&new) {
            return Err(invalid(
                "Rename would collide with an existing plugin in this profile.",
            ));
        }
    }
    if action == "rename" {
        sidecars(&root, name)?;
    }
    Ok(json!({"library_id":library.getattr("library_id")?.extract::<String>()?,"mod_id":plugin["mod_id"],"profile_id":catalog["profile_id"],"old":old,"new":new,"action":action,"expected":digest(&state),"original":file(library.py(),&root.join(&old),cancel)?,"inputs":inputs,"state":state}).to_string())
}
fn schema(db: &Bound<'_, PyAny>) -> PyResult<()> {
    db.call_method1("execute",("CREATE TABLE IF NOT EXISTS plugin_file_operations(id TEXT PRIMARY KEY,mod_id TEXT,old TEXT,new TEXT,snapshot TEXT,expected TEXT,original TEXT,replacement TEXT)",))?;
    Ok(())
}
fn paths(library: &Bound<'_, PyAny>, record: &Value) -> PyResult<[PathBuf; 6]> {
    if record["version"] != 1
        || record["library"] != library.getattr("library_id")?.extract::<String>()?
    {
        return Err(invalid(
            "Invalid or foreign owned plugin journal; manual review is required.",
        ));
    }
    let root = library.getattr("root")?.extract::<PathBuf>()?;
    let folder = root
        .join(".plugin-actions")
        .join(transaction_key(text(record, "id"))?);
    let root = root
        .join("mods")
        .join(transaction_key(text(record, "mod_id"))?);
    for field in ["old", "new"] {
        let name = text(record, field);
        plugin_name(if fold(name).ends_with(".ghost") {
            &name[..name.len() - 6]
        } else {
            name
        })?;
    }
    let paths = [
        folder.clone(),
        root.join(text(record, "old")),
        root.join(text(record, "new")),
        folder.join("original"),
        folder.join("replacement"),
        folder.join("journal.json"),
    ];
    for path in &paths {
        reject_links(path)?;
    }
    Ok(paths)
}
fn recover_one(library: &Bound<'_, PyAny>, record: &Value) -> PyResult<Value> {
    let py = library.py();
    let [_folder, source, target, backup, stage, journal] = paths(library, record)?;
    let committed = with_context(&library.call_method0("_db")?, |db| {
        schema(db)?;
        Ok(!db
            .call_method1(
                "execute",
                (
                    "SELECT 1 FROM plugin_file_operations WHERE id=?",
                    (text(record, "id"),),
                ),
            )?
            .call_method0("fetchone")?
            .is_none())
    })?;
    if !owned(library, text(record, "mod_id"), source.parent().unwrap())? {
        return Err(invalid(
            "Owned plugin journal no longer matches canonical mod ownership; files were preserved.",
        ));
    }
    let none = py.None();
    let cancel = none.bind(py);
    if committed {
        if !target.is_file() || file(py, &target, cancel)? != record["replacement"] {
            return Err(invalid(
                "Committed owned plugin changed before journal finalization; files and backup were preserved.",
            ));
        }
    } else if backup.exists() {
        if file(py, &backup, cancel)? != record["original"] {
            return Err(invalid(
                "Owned plugin backup changed; recovery preserved all files.",
            ));
        }
        if target.exists() {
            if record["replacement"].is_null()
                || file(py, &target, cancel)? != record["replacement"]
            {
                return Err(invalid(
                    "Uncommitted owned plugin destination changed; recovery preserved all files.",
                ));
            }
            py_path(py, &target)?.call_method0("unlink")?;
        }
        if source.exists() {
            return Err(invalid(
                "Owned plugin rollback source is occupied; recovery preserved all files.",
            ));
        }
        py_path(py, &backup)?.call_method1("replace", (&source,))?;
    } else if !source.is_file() || file(py, &source, cancel)? != record["original"] {
        return Err(invalid(
            "Uncommitted owned plugin original is unavailable; manual recovery is required.",
        ));
    }
    let cleanup = (|| -> PyResult<()> {
        if stage.exists() {
            py_path(py, &stage)?.call_method0("unlink")?;
        }
        py_path(py, &journal)?.call_method0("unlink")?;
        Ok(())
    })();
    if let Err(e) = cleanup {
        if committed && e.is_instance_of::<pyo3::exceptions::PyOSError>(py) {
            return Err(invalid(format!(
                "Owned plugin action {} committed, but journal finalization failed. Its current file and backup were preserved; reopen the library to retry recovery.",
                text(record, "id")
            )));
        }
        return Err(e);
    }
    Ok(json!({"kind":"plugin-file","id":record["id"],"committed":committed}))
}
fn write_journal(py: Python<'_>, journal: &Path, record: &Value) -> PyResult<()> {
    let temporary = journal.with_extension("tmp");
    reject_links(&temporary)?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("encoding", "utf-8")?;
    with_context(
        &py_path(py, &temporary)?.call_method("open", ("w",), Some(&kwargs))?,
        |stream| {
            stream.call_method1("write", (record.to_string(),))?;
            stream.call_method0("flush")?;
            py.import("os")?
                .call_method1("fsync", (stream.call_method0("fileno")?,))?;
            Ok(())
        },
    )?;
    py_path(py, &temporary)?.call_method1("replace", (journal,))?;
    Ok(())
}
fn update_profiles(
    db: &Bound<'_, PyAny>,
    state: &Value,
    rename: Option<(&str, &str)>,
) -> PyResult<()> {
    for table in ["profiles", "launch_target_profiles"] {
        for row in state[table].as_array().into_iter().flatten() {
            let files = if let Some((old, new)) = rename {
                let mut files = encoding::parse(text(row, "files"))?;
                let fields = files
                    .as_object_mut()
                    .ok_or_else(|| invalid("Invalid profile files"))?;
                fields.remove("_modbox_plugin_validation");
                fields.remove("_modbox_plugin_backup");
                if !old.is_empty() {
                    for (field, data) in fields.iter_mut() {
                        if ["plugins.txt", "loadorder.txt", "lockedorder.txt"]
                            .contains(&fold(field).as_str())
                        {
                            let mut updated = String::new();
                            for line in data.as_str().unwrap_or("").split_inclusive('\n') {
                                let stripped = line.trim().trim_start_matches('*');
                                let logical = stripped.split('|').next().unwrap_or("").trim();
                                if fold(logical) == fold(old) {
                                    updated.push_str(&line.replacen(logical, new, 1));
                                } else {
                                    updated.push_str(line);
                                }
                            }
                            *data = json!(updated);
                        }
                    }
                }
                files.to_string()
            } else {
                text(row, "files").to_string()
            };
            if table == "profiles" {
                db.call_method1(
                    "execute",
                    (
                        "UPDATE profiles SET files=? WHERE id=?",
                        (files, text(row, "id")),
                    ),
                )?;
            } else {
                db.call_method1("execute",("UPDATE launch_target_profiles SET files=? WHERE target_id=? AND profile_id=?",(files,text(row,"target_id"),text(row,"profile_id"))))?;
            }
        }
    }
    db.call_method1("execute", ("DELETE FROM plugin_metadata",))?;
    Ok(())
}
fn apply(
    library: &Bound<'_, PyAny>,
    plan: &Value,
    cancel: &Bound<'_, PyAny>,
    writer: &Bound<'_, PyAny>,
    restore: &Value,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let py = library.py();
    library.call_method0("_runtime_write_guard")?;
    if plan["library_id"] != library.getattr("library_id")?.extract::<String>()? {
        return Err(invalid("Owned plugin review belongs to another library."));
    }
    with_context(&library.getattr("owned")?.call_method0("lock")?, |_| {
        let operation = py
            .import("uuid")?
            .call_method0("uuid4")?
            .getattr("hex")?
            .extract::<String>()?;
        let mut record = json!({"version":1,"library":plan["library_id"],"id":operation,"mod_id":plan["mod_id"],"old":plan["old"],"new":plan["new"],"original":plan["original"]});
        let [folder, source, target, backup, stage, journal] = paths(library, &record)?;
        if folder.exists() {
            return Err(invalid("Owned plugin transaction folder is occupied."));
        }
        if !owned(library, text(plan, "mod_id"), source.parent().unwrap())? {
            return Err(invalid(
                "Owned plugin review no longer matches canonical mod ownership.",
            ));
        }
        with_context(&library.call_method0("_db")?, |db| {
            if digest(&state(db)?) != text(plan, "expected")
                || file(py, &source, cancel)? != plan["original"]
            {
                return Err(invalid(
                    "Owned plugin or local metadata changed after review. Review again.",
                ));
            }
            Ok(())
        })?;
        if source != target && target.exists() {
            return Err(invalid("Owned plugin destination became occupied."));
        }
        std::fs::create_dir_all(folder.parent().unwrap())?;
        std::fs::create_dir(&folder)?;
        write_journal(py, &journal, &record)?;
        let result = (|| -> PyResult<String> {
            checkpoint.call1(("journal",))?;
            if !restore.is_null() {
                let original = Path::new(text(restore, "path"));
                if file(py, original, cancel)? != restore["file"] {
                    return Err(invalid("Owned plugin undo backup changed."));
                }
                py.import("shutil")?
                    .call_method1("copyfile", (original, &stage))?;
            } else if ["esm", "no-esm"].contains(&text(plan, "action")) {
                let cli = writer.is_none();
                let writer = if cli {
                    py.import("creation_lib.mod_manager.plugin_flag_writer")?
                        .getattr("write")?
                } else {
                    writer.clone()
                };
                writer.call1((
                    py_path(py, &source)?,
                    py_path(py, &stage)?,
                    text(plan, "action"),
                    cancel,
                ))?;
                if cli {
                    let kwargs = PyDict::new(py);
                    kwargs.set_item("game", "fo4")?;
                    kwargs.set_item("cancel", cancel)?;
                    let header = py
                        .import("creation_lib.mod_manager.cli_reader")?
                        .getattr("run_json")?
                        .call(
                            (("esp", "header", stage.to_string_lossy().as_ref()),),
                            Some(&kwargs),
                        )?;
                    if header
                        .get_item("flags")?
                        .call_method1("get", ("esm",))?
                        .is_truthy()?
                        != (text(plan, "action") == "esm")
                    {
                        return Err(invalid(
                            "Staged master flag failed CLI verification; the owned original was preserved.",
                        ));
                    }
                }
            } else {
                py.import("shutil")?
                    .call_method1("copyfile", (&source, &stage))?;
            }
            record["replacement"] = file(py, &stage, cancel)?;
            write_journal(py, &journal, &record)?;
            checkpoint.call1(("prepared",))?;
            check_cancel(Some(cancel))?;
            with_context(&library.call_method0("_db")?, |db| {
                db.call_method1("execute", ("BEGIN IMMEDIATE",))?;
                if digest(&state(db)?) != text(plan, "expected")
                    || file(py, &source, cancel)? != plan["original"]
                {
                    return Err(invalid(
                        "Owned plugin or local metadata changed during preparation. Review again.",
                    ));
                }
                for input in plan["inputs"].as_array().into_iter().flatten() {
                    let path = Path::new(input[0].as_str().unwrap_or(""));
                    reject_links(path)?;
                    let info = stamp(&py_path(py, path)?.call_method0("stat")?)?;
                    if json!(info.2) != input[1] || json!(info.3) != input[2] {
                        return Err(invalid("Plugin dependency/provider changed after review."));
                    }
                }
                if source != target && target.exists() {
                    return Err(invalid(
                        "Owned plugin destination became occupied during preparation.",
                    ));
                }
                if text(plan, "action") == "rename" {
                    sidecars(source.parent().unwrap(), text(plan, "old"))?;
                }
                py_path(py, &source)?.call_method1("replace", (&backup,))?;
                checkpoint.call1(("old-moved",))?;
                py_path(py, &stage)?.call_method1("replace", (&target,))?;
                checkpoint.call1(("placed",))?;
                check_cancel(Some(cancel))?;
                let state = if restore.is_null() {
                    &plan["state"]
                } else {
                    &restore["state"]
                };
                let mod_row = state["mods"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|r| r["id"] == plan["mod_id"])
                    .ok_or_else(|| invalid("Owned mod metadata missing"))?;
                let manifest = if restore.is_null() {
                    let raw = text(mod_row, "manifest");
                    let mut manifest = encoding::parse(if raw.is_empty() { "{}" } else { raw })?;
                    manifest
                        .as_object_mut()
                        .ok_or_else(|| invalid("Invalid owned manifest"))?
                        .retain(|k, _| fold(k) != fold(text(plan, "old")));
                    manifest[text(plan, "new")] = record["replacement"].clone();
                    manifest.to_string()
                } else {
                    text(mod_row, "manifest").to_string()
                };
                db.call_method1(
                    "execute",
                    (
                        "UPDATE mods SET manifest=? WHERE id=?",
                        (manifest, text(plan, "mod_id")),
                    ),
                )?;
                update_profiles(
                    db,
                    state,
                    if restore.is_null() {
                        Some((
                            if text(plan, "action") == "rename" {
                                text(plan, "old")
                            } else {
                                ""
                            },
                            text(plan, "new"),
                        ))
                    } else {
                        None
                    },
                )?;
                schema(db)?;
                let expected = digest(&self::state(db)?);
                db.call_method1(
                    "execute",
                    (
                        "INSERT INTO plugin_file_operations VALUES (?,?,?,?,?,?,?,?)",
                        (
                            &operation,
                            text(plan, "mod_id"),
                            text(plan, "old"),
                            text(plan, "new"),
                            plan["state"].to_string(),
                            expected,
                            plan["original"].to_string(),
                            record["replacement"].to_string(),
                        ),
                    ),
                )?;
                checkpoint.call1(("before-commit",))?;
                check_cancel(Some(cancel))
            })?;
            checkpoint.call1(("committed",))?;
            recover_one(library, &record)?;
            Ok(operation.clone())
        })();
        match result {
            Ok(v) => Ok(v),
            Err(e) => {
                let recovered = recover_one(library, &record)?;
                if recovered["committed"] == true
                    && e.is_instance(py, &py.import("builtins")?.getattr("Exception")?)
                {
                    Ok(operation)
                } else {
                    Err(e)
                }
            }
        }
    })
}
#[pyfunction]
fn plugin_file_apply(
    library: &Bound<'_, PyAny>,
    plan: &str,
    cancel: &Bound<'_, PyAny>,
    writer: &Bound<'_, PyAny>,
    restore: &str,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<String> {
    apply(
        library,
        &encoding::parse(plan)?,
        cancel,
        writer,
        &encoding::parse(restore)?,
        checkpoint,
    )
}
#[pyfunction]
fn plugin_file_unavailable(
    library: &Bound<'_, PyAny>,
    catalog: &str,
    plugin: &str,
) -> PyResult<String> {
    unavailable(
        library,
        &encoding::parse(catalog)?,
        &encoding::parse(plugin)?,
    )
}
#[pyfunction]
fn plugin_file_fingerprint(
    py: Python<'_>,
    path: PathBuf,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<String> {
    Ok(file(py, &path, cancel)?.to_string())
}
#[pyfunction]
fn plugin_file_state(db: &Bound<'_, PyAny>) -> PyResult<String> {
    Ok(state(db)?.to_string())
}
#[pyfunction]
fn plugin_file_paths(library: &Bound<'_, PyAny>, record: &str) -> PyResult<Vec<PathBuf>> {
    Ok(paths(library, &encoding::parse(record)?)?.to_vec())
}
#[pyfunction]
fn plugin_file_recover_one(library: &Bound<'_, PyAny>, record: &str) -> PyResult<String> {
    Ok(recover_one(library, &encoding::parse(record)?)?.to_string())
}
#[pyfunction]
fn plugin_file_recover(library: &Bound<'_, PyAny>) -> PyResult<String> {
    let root = library
        .getattr("root")?
        .extract::<PathBuf>()?
        .join(".plugin-actions");
    reject_links(&root)?;
    if !root.exists() {
        return Ok("[]".into());
    }
    with_context(&library.getattr("owned")?.call_method0("lock")?, |_| {
        let mut results = Vec::new();
        for entry in std::fs::read_dir(&root)? {
            let folder = entry?.path();
            transaction_key(&folder.file_name().unwrap_or_default().to_string_lossy())?;
            reject_links(&folder)?;
            let journal = folder.join("journal.json");
            reject_links(&journal)?;
            if journal.exists() {
                if journal.metadata()?.len() > 4096 {
                    return Err(invalid("Owned plugin recovery journal exceeds 4 KiB."));
                }
                let record = encoding::parse(&std::fs::read_to_string(journal)?)?;
                results.push(recover_one(library, &record)?);
            }
        }
        Ok(json!(results).to_string())
    })
}
#[pyfunction]
fn plugin_file_undo(
    library: &Bound<'_, PyAny>,
    identifier: &str,
    cancel: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let (row, current) = with_context(&library.call_method0("_db")?, |db| {
        schema(db)?;
        let row = db
            .call_method1(
                "execute",
                (
                    "SELECT * FROM plugin_file_operations WHERE id=?",
                    (transaction_key(identifier)?,),
                ),
            )?
            .call_method0("fetchone")?;
        let row = if row.is_none() {
            Value::Null
        } else {
            value(&db.py().import("builtins")?.getattr("dict")?.call1((row,))?)?
        };
        Ok((row, state(db)?))
    })?;
    if row.is_null() || digest(&current) != text(&row, "expected") {
        return Err(invalid(
            "Owned plugin metadata changed since that action; Undo is blocked to preserve later changes.",
        ));
    }
    let plan = json!({"library_id":library.getattr("library_id")?.extract::<String>()?,"mod_id":row["mod_id"],"profile_id":"","old":row["new"],"new":row["old"],"action":"undo","expected":row["expected"],"original":encoding::parse(text(&row,"replacement"))?,"inputs":[],"state":current});
    let restore = json!({"state":encoding::parse(text(&row,"snapshot"))?,"file":encoding::parse(text(&row,"original"))?,"path":library.getattr("root")?.extract::<PathBuf>()?.join(".plugin-actions").join(identifier).join("original")});
    apply(
        library,
        &plan,
        cancel,
        library.py().None().bind(library.py()),
        &restore,
        checkpoint,
    )
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(plugin_file_review, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_file_apply, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_file_unavailable, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_file_fingerprint, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_file_state, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_file_paths, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_file_recover_one, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_file_recover, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_file_undo, m)?)?;
    Ok(())
}
