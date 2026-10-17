use crate::{
    check_cancel,
    database::{Database, error, records},
    encoding,
    executables::{definition, text},
    invalid, reject_links,
};
use pyo3::prelude::*;
use rusqlite::params;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use unicode_casefold::UnicodeCaseFold;

pub fn games() -> Vec<Value> {
    [
        ("fo4","Fallout 4","Fallout4.exe","f4se_loader.exe",vec!["Fallout4.ini","Fallout4Prefs.ini","Fallout4Custom.ini"]),
        ("skyrimse","Skyrim Special Edition","SkyrimSE.exe","skse64_loader.exe",vec!["Skyrim.ini","SkyrimPrefs.ini"]),
        ("skyrim","Skyrim","TESV.exe","skse_loader.exe",vec!["Skyrim.ini","SkyrimPrefs.ini"]),
        ("fnv","Fallout New Vegas","FalloutNV.exe","nvse_loader.exe",vec!["Fallout.ini","FalloutPrefs.ini"]),
        ("fo3","Fallout 3","Fallout3.exe","fose_loader.exe",vec!["Fallout.ini","FalloutPrefs.ini"]),
        ("starfield","Starfield","Starfield.exe","sfse_loader.exe",vec!["Starfield.ini","StarfieldPrefs.ini","StarfieldCustom.ini"]),
    ].into_iter().map(|(id,name,binary,extender,inis)| json!({"id":id,"name":name,"binary":binary,"extender":extender,"ini_names":inis,"native":id=="fo4"})).collect()
}
#[pyfunction]
fn manager_games() -> String {
    json!(games()).to_string()
}

fn game(id: &str) -> PyResult<Value> {
    games()
        .into_iter()
        .find(|v| text(v, "id") == id)
        .ok_or_else(|| invalid("Choose a supported game for this instance."))
}
#[pyfunction]
fn manager_game(id: &str) -> PyResult<String> {
    Ok(game(id)?.to_string())
}

fn resolve(py: Python<'_>, path: &Path) -> PyResult<PathBuf> {
    py.import("pathlib")?
        .getattr("Path")?
        .call1((path,))?
        .call_method0("resolve")?
        .extract()
}

pub fn validate_root(
    py: Python<'_>,
    id: &str,
    root: &Path,
    library: &Path,
) -> PyResult<(Value, PathBuf)> {
    let game = game(id)?;
    if !root.is_absolute()
        || ["\\\\", "//"]
            .iter()
            .any(|prefix| root.to_string_lossy().starts_with(prefix))
    {
        return Err(invalid(
            "Choose an absolute local game installation directory.",
        ));
    }
    reject_links(root)?;
    if !crate::executables::local_mod_target(root.into())? {
        return Err(invalid(
            "Choose a game installation on an ordinary local drive.",
        ));
    }
    let owned = std::path::absolute(library)?;
    reject_links(&owned)?;
    let root = resolve(py, root)?;
    let owned = resolve(py, &owned)?;
    if root.starts_with(&owned) || owned.starts_with(&root) {
        return Err(invalid(
            "The manager library must be separate from the game installation.",
        ));
    }
    let binary = root.join(text(&game, "binary"));
    let data = root.join("Data");
    reject_links(&binary)?;
    reject_links(&data)?;
    if !binary.is_file() || !data.is_dir() {
        return Err(invalid(format!(
            "Select the folder containing {} and Data.",
            text(&game, "binary")
        )));
    }
    Ok((game, root))
}

#[pyfunction]
fn instance_validate_root(
    py: Python<'_>,
    id: &str,
    root: PathBuf,
    library: PathBuf,
) -> PyResult<(String, PathBuf)> {
    let (game, root) = validate_root(py, id, &root, &library)?;
    Ok((game.to_string(), root))
}

#[pyfunction]
#[pyo3(signature=(db,library,id,root,name,reader,cancel=None))]
fn instance_create(
    db: &Database,
    library: PathBuf,
    id: &str,
    root: PathBuf,
    name: &str,
    reader: &Bound<'_, PyAny>,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let py = reader.py();
    let (game, path) = validate_root(py, id, &root, &library)?;
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 200 || name.chars().any(|c| c < ' ') {
        return Err(invalid(
            "Give the initial profile a short name without control characters.",
        ));
    }
    let mut executables = vec![definition(
        json!({"title":game["name"],"binary":path.join(text(&game,"binary")),"working_directory":path}),
    )?];
    let extender = path.join(text(&game, "extender"));
    if !text(&game, "extender").is_empty() && extender.is_file() {
        reject_links(&extender)?;
        let title = extender
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_uppercase();
        executables.push(definition(json!({"title":title.strip_suffix("_LOADER").unwrap_or(&title),"binary":extender,"working_directory":path}))?);
    }
    check_cancel(cancel)?;
    db.connection()?
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(error)?;
    let mut instance_id = String::new();
    let instances = records(
        &*db.connection()?,
        "SELECT id,configuration FROM instances",
        [],
    )?;
    for instance in instances {
        let existing = encoding::parse(text(&instance, "configuration"))?;
        if existing["kind"] == "standalone"
            && existing["game"]["id"] == id
            && text(&existing["game"], "gamepath")
                .case_fold()
                .eq(path.to_string_lossy().case_fold())
        {
            let profiles = records(
                &*db.connection()?,
                "SELECT id FROM profiles WHERE json_extract(files,'$._modbox_instance_id')=? ORDER BY name COLLATE NOCASE,id",
                [text(&instance, "id")],
            )?;
            if let Some(profile) = profiles.first() {
                return Ok(text(profile, "id").into());
            }
            instance_id = text(&instance, "id").into();
            break;
        }
    }
    if instance_id.is_empty() {
        instance_id = uuid::Uuid::new_v4().simple().to_string();
    }
    let profile_id = uuid::Uuid::new_v4().simple().to_string();
    let configuration = json!({"kind":"standalone","game":{"id":id,"name":game["name"],"gamepath":path},"paths":{"mods":library.join("mods"),"downloads":library.join("downloads")}});
    let mut files = json!({"_modbox_instance_id":instance_id,"_modbox_game":game["name"],"plugins.txt":"","loadorder.txt":"","lockedorder.txt":"","_modbox_minimal_inis":encoding::encode(&game["ini_names"],true,true)});
    for ini in game["ini_names"].as_array().unwrap() {
        files[ini.as_str().unwrap()] = json!("[General]\n");
    }
    if id == "fo4" {
        let default = path.join("Fallout4_default.ini");
        reject_links(&default)?;
        let mut minimal = game["ini_names"].as_array().unwrap().clone();
        if default.is_file() {
            let text: String = reader.call1((default, 2 * 1024 * 1024))?.extract()?;
            crate::profile_state::profile_validate_ini(py, "Fallout4.ini", &text)?;
            files["Fallout4.ini"] = json!(text);
            minimal.retain(|v| v != "Fallout4.ini");
        }
        files["Fallout4Custom.ini"] =
            json!("[Archive]\nbInvalidateOlderFiles=1\nsResourceDataDirsFinal=\n");
        minimal.retain(|v| v != "Fallout4Custom.ini");
        files["_modbox_minimal_inis"] = json!(encoding::encode(&json!(minimal), true, true));
    }
    db.connection()?
        .execute(
            "INSERT OR IGNORE INTO instances VALUES (?,?,?)",
            params![
                instance_id,
                "",
                encoding::encode(&configuration, true, true)
            ],
        )
        .map_err(error)?;
    db.connection()?
        .execute(
            "INSERT INTO profiles VALUES (?,?,?,?)",
            params![profile_id, name, "", encoding::encode(&files, true, true)],
        )
        .map_err(error)?;
    if records(
        &*db.connection()?,
        "SELECT 1 FROM executables WHERE instance_id=?",
        [&instance_id],
    )?
    .is_empty()
    {
        for mut item in executables {
            item["instance_id"] = json!(instance_id);
            db.connection()?
                .execute(
                    "INSERT INTO executables VALUES (?,?,?,0)",
                    params![
                        text(&item, "id"),
                        instance_id,
                        encoding::encode(&item, true, true)
                    ],
                )
                .map_err(error)?;
        }
    }
    check_cancel(cancel)?;
    Ok(profile_id)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(manager_games, m)?)?;
    m.add_function(wrap_pyfunction!(manager_game, m)?)?;
    m.add_function(wrap_pyfunction!(instance_validate_root, m)?)?;
    m.add_function(wrap_pyfunction!(instance_create, m)?)?;
    Ok(())
}
