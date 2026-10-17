use crate::{
    categories::fold,
    database::{Database, error},
    encoding, invalid,
};
use pyo3::prelude::*;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[pyfunction]
pub fn local_mod_target(path: PathBuf) -> PyResult<bool> {
    let path = std::path::absolute(path)?;
    if path.to_string_lossy().starts_with("\\\\") || path.parent().is_none() {
        return Ok(false);
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetDriveTypeW(root: *const u16) -> u32;
        }
        let root = path.ancestors().last().unwrap();
        let root: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
        if !matches!(unsafe { GetDriveTypeW(root.as_ptr()) }, 2 | 3) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[pyfunction]
#[pyo3(signature = (raw, label, directory=false))]
pub fn launch_local_path(raw: &str, label: &str, directory: bool) -> PyResult<PathBuf> {
    let path: PathBuf = PathBuf::from(raw).components().collect();
    if raw.is_empty() || !path.is_absolute() || !local_mod_target(path.clone())? {
        return Err(invalid(format!(
            "{label} must be an absolute path on a local drive."
        )));
    }
    crate::reject_links(&path)?;
    if !(if directory {
        path.is_dir()
    } else {
        path.is_file()
    }) {
        return Err(invalid(format!(
            "{label} is missing or is not a {}: {}",
            if directory { "directory" } else { "file" },
            path.display()
        )));
    }
    Ok(path)
}

fn launch_review(
    value: Value,
    profile_id: &str,
    profile_name: &str,
    fingerprint: &Bound<'_, PyAny>,
) -> PyResult<Value> {
    let executable = definition(value)?;
    let binary = launch_local_path(text(&executable, "binary"), "Executable", false)?;
    if binary
        .extension()
        .and_then(|v| v.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
        != Some("exe")
    {
        return Err(invalid(
            "Select an .exe file. Scripts and shell command strings are not executable paths.",
        ));
    }
    let working = text(&executable, "working_directory");
    let working = if working.is_empty() {
        binary.parent().unwrap().to_string_lossy().to_string()
    } else {
        working.into()
    };
    let cwd = launch_local_path(&working, "Working directory", true)?;
    let mut argv = vec![binary.to_string_lossy().to_string()];
    argv.extend(crate::windows_arguments(text(&executable, "arguments"))?);
    if argv
        .iter()
        .map(|arg| arg.chars().count() + 3)
        .sum::<usize>()
        > 32760
    {
        return Err(invalid(
            "The resulting Windows command line exceeds its size limit.",
        ));
    }
    if executable["mode"] == "profile" {
        if profile_id.is_empty() {
            return Err(invalid(
                "Select a mod profile before reviewing a modded launch.",
            ));
        }
        return Err(invalid(
            "Modded launch needs a configured VFS runtime. Imported definitions are ready to edit, but Direct mode cannot expose the selected mod profile. See the runtime choices in the manager documentation.",
        ));
    }
    let stamp = fingerprint.call1((binary,))?;
    let stamp = encoding::parse(
        &fingerprint
            .py()
            .import("json")?
            .call_method1("dumps", (stamp,))?
            .extract::<String>()?,
    )?;
    Ok(
        json!({"executable":executable,"argv":argv,"cwd":cwd.to_string_lossy(),"signature":stamp,"profile_id":profile_id,"profile_name":profile_name,"runtime":"direct"}),
    )
}

#[pyfunction]
fn executable_review(
    value: &str,
    profile_id: &str,
    profile_name: &str,
    fingerprint: &Bound<'_, PyAny>,
) -> PyResult<String> {
    Ok(launch_review(
        encoding::parse(value)?,
        profile_id,
        profile_name,
        fingerprint,
    )?
    .to_string())
}

#[pyfunction]
fn executable_validate_launch(plan: &str, fingerprint: &Bound<'_, PyAny>) -> PyResult<()> {
    let plan = encoding::parse(plan)?;
    if plan["runtime"] != "direct" || plan["executable"]["mode"] != "direct" {
        return Err(invalid(
            "This runtime has no verified process adapter; launch was refused.",
        ));
    }
    let current = launch_review(
        plan["executable"].clone(),
        text(&plan, "profile_id"),
        text(&plan, "profile_name"),
        fingerprint,
    )?;
    if current != plan {
        return Err(invalid(
            "Executable changed after review. Review it again before running.",
        ));
    }
    Ok(())
}

pub fn identity(value: &str) -> String {
    format!("{:x}", Sha256::digest(fold(value).as_bytes()))[..32].into()
}
pub fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn definition(mut value: Value) -> PyResult<Value> {
    if !value.is_object() {
        return Err(invalid("Invalid executable definition."));
    }
    for key in [
        "id",
        "title",
        "binary",
        "arguments",
        "working_directory",
        "instance_id",
        "steam_app_id",
    ] {
        if value.get(key).is_none() {
            value[key] = json!("");
        }
        if !value[key].is_string() {
            return Err(invalid("Executable text fields must be strings."));
        }
    }
    if text(&value, "id").is_empty() {
        value["id"] = json!(uuid::Uuid::new_v4().simple().to_string());
    }
    if value.get("mode").is_none() {
        value["mode"] = json!("profile");
    }
    if value.get("hidden").is_none() {
        value["hidden"] = json!(false);
    }
    let title = text(&value, "title");
    if title.trim().is_empty() || title.chars().count() > 200 || title.chars().any(|c| c < ' ') {
        return Err(invalid(
            "Give this executable a short title without control characters.",
        ));
    }
    if !matches!(text(&value, "mode"), "profile" | "direct") {
        return Err(invalid("Choose a supported launch mode."));
    }
    let steam = text(&value, "steam_app_id");
    if !steam.is_empty()
        && (steam.len() > 10
            || !matches!(steam.as_bytes()[0], b'1'..=b'9')
            || !steam.bytes().all(|c| c.is_ascii_digit()))
    {
        return Err(invalid(
            "Steam application ID must be a positive numeric ID or empty.",
        ));
    }
    for key in ["binary", "arguments", "working_directory"] {
        let text = text(&value, key);
        if text.chars().count() > 32760 || text.contains(['\0', '\n', '\r']) {
            return Err(invalid(
                "Executable fields must fit one Windows command line without control characters.",
            ));
        }
    }
    let args = text(&value, "arguments").to_lowercase();
    if [
        "apikey",
        "api_key",
        "api-key",
        "password",
        "accesstoken",
        "access_token",
        "access-token",
        "nxm://",
    ]
    .iter()
    .any(|key| args.contains(key))
    {
        return Err(invalid(
            "Credential-bearing arguments cannot be stored in executable definitions.",
        ));
    }
    for key in ["title", "binary", "working_directory"] {
        value[key] = json!(text(&value, key).trim());
    }
    Ok(value)
}

pub fn imported(configuration: &Value, instance: &str) -> Vec<Value> {
    let mut result = Vec::new();
    for item in configuration
        .get("executables")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let title = text(item, "title").trim();
        if title.is_empty()
            || ["binary", "arguments", "workingdirectory"]
                .iter()
                .any(|field| text(item, field).contains("[credential-bearing"))
        {
            continue;
        }
        let value = json!({"id":identity(&format!("{instance}/executable/{}",fold(title))),"title":title,"binary":text(item,"binary"),"arguments":text(item,"arguments"),"working_directory":text(item,"workingdirectory"),"instance_id":instance,"hidden":fold(text(item,"hide"))=="true","steam_app_id":text(item,"steamappid")});
        if let Ok(value) = definition(value) {
            result.push(value);
        }
    }
    result
}

pub fn import_into(connection: &Connection, configuration: &Value, instance: &str) -> PyResult<()> {
    for value in imported(configuration, instance) {
        connection
            .execute(
                "INSERT OR IGNORE INTO executables(id,instance_id,configuration) VALUES (?,?,?)",
                params![text(&value, "id"), instance, value.to_string()],
            )
            .map_err(error)?;
    }
    Ok(())
}

#[pyfunction]
fn executable_definition(value: &str) -> PyResult<String> {
    Ok(definition(encoding::parse(value)?)?.to_string())
}
#[pyfunction]
fn executable_imported(configuration: &str, instance: &str) -> PyResult<String> {
    Ok(json!(imported(&encoding::parse(configuration)?, instance)).to_string())
}
#[pyfunction]
fn executable_migrate(db: &Database) -> PyResult<()> {
    let connection = db.connection()?;
    let values = connection
        .prepare("SELECT id,configuration FROM instances")
        .map_err(error)?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(error)?;
    for (instance, configuration) in values {
        import_into(&connection, &encoding::parse(&configuration)?, &instance)?;
    }
    Ok(())
}
#[pyfunction]
fn executable_records(db: &Database, instance: &str) -> PyResult<String> {
    let connection = db.connection()?;
    let rows=connection.prepare("SELECT configuration FROM executables WHERE removed=0 AND (instance_id='' OR instance_id=?) ORDER BY json_extract(configuration,'$.title') COLLATE NOCASE,id").map_err(error)?.query_map([instance],|r|r.get::<_,String>(0)).map_err(error)?.collect::<rusqlite::Result<Vec<_>>>().map_err(error)?;
    Ok(json!(
        rows.iter()
            .map(|v| encoding::parse(v))
            .collect::<PyResult<Vec<_>>>()?
    )
    .to_string())
}
#[pyfunction]
fn executable_save(db: &Database, value: &str) -> PyResult<String> {
    let value = definition(encoding::parse(value)?)?;
    db.connection()?.execute("INSERT INTO executables VALUES (?,?,?,0) ON CONFLICT(id) DO UPDATE SET instance_id=excluded.instance_id,configuration=excluded.configuration,removed=0",params![text(&value,"id"),text(&value,"instance_id"),value.to_string()]).map_err(error)?;
    Ok(value.to_string())
}
#[pyfunction]
fn executable_remove(db: &Database, id: &str) -> PyResult<()> {
    db.connection()?
        .execute("UPDATE executables SET removed=1 WHERE id=?", [id])
        .map_err(error)?;
    Ok(())
}
#[pyfunction]
fn manager_identity(value: &str) -> String {
    identity(value)
}

#[pyfunction]
fn executable_start(
    plan: &Bound<'_, PyAny>,
    spawn: &Bound<'_, PyAny>,
    disable_mcp: bool,
) -> PyResult<Py<PyAny>> {
    let py = plan.py();
    let record = py.import("dataclasses")?.call_method1("asdict", (plan,))?;
    let record: String = py
        .import("json")?
        .call_method1("dumps", (record,))?
        .extract()?;
    let fingerprint = py
        .import("creation_lib.mod_manager.launcher")?
        .getattr("fingerprint")?;
    executable_validate_launch(&record, &fingerprint)?;
    let subprocess = py.import("subprocess")?;
    let args = pyo3::types::PyList::new(
        py,
        plan.getattr("argv")?
            .try_iter()?
            .collect::<PyResult<Vec<_>>>()?,
    )?;
    let kwargs = pyo3::types::PyDict::new(py);
    kwargs.set_item("cwd", plan.getattr("cwd")?)?;
    kwargs.set_item("shell", false)?;
    kwargs.set_item("stdin", subprocess.getattr("DEVNULL")?)?;
    kwargs.set_item("creationflags", if cfg!(windows) { 0x08000000 } else { 0 })?;
    let result: PyResult<Py<PyAny>> = (|| {
        let steam: String = plan
            .getattr("executable")?
            .getattr("steam_app_id")?
            .extract()?;
        kwargs.set_item("env", crate::launch_environment(&steam, disable_mcp))?;
        let spawn = if spawn.is_truthy()? {
            spawn.clone()
        } else {
            subprocess.getattr("Popen")?
        };
        Ok(spawn.call((args,), Some(&kwargs))?.unbind())
    })();
    match result {
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyOSError>(py) => Err(invalid(format!(
            "Could not start {}: {}",
            plan.getattr("executable")?.getattr("title")?.str()?,
            e.value(py)
        ))),
        result => result,
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(executable_start, m)?)?;
    m.add_function(wrap_pyfunction!(local_mod_target, m)?)?;
    m.add_function(wrap_pyfunction!(launch_local_path, m)?)?;
    m.add_function(wrap_pyfunction!(executable_review, m)?)?;
    m.add_function(wrap_pyfunction!(executable_validate_launch, m)?)?;
    m.add_function(wrap_pyfunction!(executable_definition, m)?)?;
    m.add_function(wrap_pyfunction!(executable_imported, m)?)?;
    m.add_function(wrap_pyfunction!(executable_migrate, m)?)?;
    m.add_function(wrap_pyfunction!(executable_records, m)?)?;
    m.add_function(wrap_pyfunction!(executable_save, m)?)?;
    m.add_function(wrap_pyfunction!(executable_remove, m)?)?;
    m.add_function(wrap_pyfunction!(manager_identity, m)?)?;
    Ok(())
}
