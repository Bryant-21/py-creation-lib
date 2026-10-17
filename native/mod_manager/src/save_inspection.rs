use crate::{
    check_cancel, encoding,
    executables::text,
    history_prune::{py_path, stamp},
    invalid,
    python_io::with_context,
    reject_links, relative_path,
    saves::{Reader, fold},
};
use pyo3::prelude::*;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
fn plugin(name: &str) -> PyResult<String> {
    let name = relative_path(name, false)?;
    if name.contains('/')
        || ![".esp", ".esm", ".esl"]
            .iter()
            .any(|suffix| fold(&name).ends_with(suffix))
    {
        return Err(invalid(
            "Plugin names must be root-level ESP, ESM or ESL filenames.",
        ));
    }
    Ok(name)
}
fn validate(py: Python<'_>, inspection: &Value, cancel: &Bound<'_, PyAny>) -> PyResult<()> {
    check_cancel(Some(cancel))?;
    let path = PathBuf::from(text(inspection, "path"));
    reject_links(&path)?;
    if json!(stamp(&py_path(py, &path)?.call_method0("stat")?)?) != inspection["stamp"] {
        return Err(invalid(
            "Save changed after plugin inspection. Refresh and inspect it again.",
        ));
    }
    Ok(())
}
#[pyfunction]
fn save_inspection_validate(
    py: Python<'_>,
    inspection: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<()> {
    validate(py, &encoding::parse(inspection)?, cancel)
}
#[pyfunction]
fn save_inspect(save: &Bound<'_, PyAny>, cancel: &Bound<'_, PyAny>) -> PyResult<String> {
    let py = save.py();
    let mut basic = crate::saves::header(save, cancel)?;
    let path = PathBuf::from(save.getattr("path")?.extract::<String>()?);
    reject_links(&path)?;
    let path_py = py_path(py, &path)?;
    let before = stamp(&path_py.call_method0("stat")?)?;
    let identity = save.getattr("identity")?;
    if (before.2, before.3)
        != (
            save.getattr("size")?.extract::<u64>()?,
            save.getattr("modified")?.extract::<i64>()?,
        )
        || (!identity.is_none() && identity.extract::<(u64, u64)>()? != (before.0, before.1))
    {
        return Err(invalid("Save changed after inventory. Refresh it."));
    }
    let mut result = with_context(&path_py.call_method1("open", ("rb",))?, |stream| {
        let os = py.import("os")?;
        let fd = stream.call_method0("fileno")?;
        if stamp(&os.call_method1("fstat", (&fd,))?)? != before {
            return Err(invalid("Save identity changed before plugin inspection."));
        }
        let mut reader = Reader {
            stream: stream.clone(),
            cancel: cancel.clone(),
            boundary: before.2,
            basic: false,
            notices: Vec::new(),
        };
        reader.read(12)?;
        reader.boundary = 16 + reader.u32()? as u64;
        if reader.boundary > before.2.min(65536) {
            return Err(invalid("Save basic header exceeds its inspection limit."));
        }
        let version = reader.u32()?;
        if ![11, 12, 13, 15].contains(&version) {
            return Err(invalid(format!(
                "FO4 save header version {version} has no supported plugin-table layout."
            )));
        }
        reader.read(4)?;
        reader.text()?;
        reader.read(4)?;
        for _ in 0..3 {
            reader.text()?;
        }
        reader.read(18)?;
        let width = reader.u32()?;
        let height = reader.u32()?;
        if reader.position()? != reader.boundary {
            return Err(invalid(
                "Save header length does not match the supported FO4 layout.",
            ));
        }
        let image_size = width as u64 * height as u64 * 4;
        let image_offset = reader.position()?;
        if width == 0
            || width > 8192
            || height == 0
            || height > 8192
            || image_size > 64 * 1024 * 1024
        {
            return Err(invalid(
                "Save screenshot dimensions exceed the supported 64 MiB limit.",
            ));
        }
        reader.boundary = before.2;
        if reader.position()? + image_size > reader.boundary {
            return Err(invalid("Save screenshot is truncated."));
        }
        stream.call_method1("seek", (image_size, 1))?;
        let form_version = reader.u8()?;
        let game_version = reader.text()?;
        if form_version >= 68 && version != 15 {
            return Err(invalid(
                "This header/form-version combination has an unverified FO4 light-plugin layout.",
            ));
        }
        let block_size = reader.u32()?;
        if !(1..=8 * 1024 * 1024).contains(&block_size) {
            return Err(invalid("Save plugin table exceeds its 8 MiB limit."));
        }
        reader.boundary = reader.position()? + block_size as u64;
        if reader.boundary > before.2 {
            return Err(invalid("Save plugin table is truncated."));
        }
        let mut regular = Vec::new();
        for _ in 0..reader.u8()? {
            regular.push(plugin(&reader.text()?)?);
        }
        let light_count = if version == 15 && form_version >= 68 {
            reader.u16()?
        } else {
            0
        };
        if light_count > 4096 {
            return Err(invalid("Save declares more than 4,096 light plugins."));
        }
        let mut light = Vec::new();
        for _ in 0..light_count {
            light.push(plugin(&reader.text()?)?);
        }
        if reader.position()? != reader.boundary {
            return Err(invalid(
                "Save plugin-table length does not match the supported FO4 layout.",
            ));
        }
        if regular
            .iter()
            .chain(&light)
            .map(|v| fold(v))
            .collect::<BTreeSet<_>>()
            .len()
            != regular.len() + light.len()
        {
            return Err(invalid("Save has duplicate plugin names ignoring case."));
        }
        check_cancel(Some(cancel))?;
        reject_links(&path)?;
        if stamp(&os.call_method1("fstat", (&fd,))?)? != before
            || stamp(&path_py.call_method0("stat")?)? != before
        {
            return Err(invalid(
                "Save changed during plugin inspection. Refresh it.",
            ));
        }
        Ok(
            json!({"path":path.to_string_lossy(),"stamp":before,"form_version":form_version,"game_version":game_version,"width":width,"height":height,"regular":regular,"light":light,"notices":reader.notices,"image_offset":image_offset}),
        )
    })?;
    basic["coverage"] = json!(
        "FO4 header and bounded regular/light plugin tables. Save body, extender companion and runtime compatibility remain unverified."
    );
    result["basic"] = basic;
    Ok(result.to_string())
}
#[pyfunction]
fn save_screenshot<'py>(
    py: Python<'py>,
    inspection: &str,
    cancel: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let inspection = encoding::parse(inspection)?;
    validate(py, &inspection, cancel)?;
    let width = inspection["width"].as_u64().unwrap_or(0);
    let height = inspection["height"].as_u64().unwrap_or(0);
    let size = width.saturating_mul(height).saturating_mul(4);
    if !(1..=8192).contains(&width)
        || !(1..=8192).contains(&height)
        || !(1..=16 * 1024 * 1024).contains(&size)
    {
        return Err(invalid(
            "Save screenshot preview exceeds its 16 MiB / four-megapixel limit. Plugin inspection remains available.",
        ));
    }
    let offset = inspection["image_offset"].as_u64().unwrap_or(0);
    if !(24..=65536).contains(&offset)
        || offset + size > inspection["stamp"][2].as_u64().unwrap_or(0)
    {
        return Err(invalid(
            "Save screenshot range does not match the inspected metadata.",
        ));
    }
    let path = PathBuf::from(text(&inspection, "path"));
    let pixels = with_context(
        &py_path(py, &path)?.call_method1("open", ("rb",))?,
        |stream| {
            let os = py.import("os")?;
            let fd = stream.call_method0("fileno")?;
            if json!(stamp(&os.call_method1("fstat", (&fd,))?)?) != inspection["stamp"] {
                return Err(invalid("Save identity changed before screenshot preview."));
            }
            if stream.call_method1("read", (12,))?.extract::<Vec<u8>>()? != b"FO4_SAVEGAME" {
                return Err(invalid("Save screenshot has an unsupported signature."));
            }
            let length = stream.call_method1("read", (4,))?.extract::<Vec<u8>>()?;
            if length.len() != 4
                || 16 + u32::from_le_bytes(length.try_into().unwrap()) as u64 != offset
            {
                return Err(invalid("Save screenshot offset changed after inspection."));
            }
            stream.call_method1("seek", (offset - 8,))?;
            let mut expected = (width as u32).to_le_bytes().to_vec();
            expected.extend((height as u32).to_le_bytes());
            if stream.call_method1("read", (8,))?.extract::<Vec<u8>>()? != expected {
                return Err(invalid(
                    "Save screenshot dimensions changed after inspection.",
                ));
            }
            let mut pixels = Vec::with_capacity(size as usize);
            while (pixels.len() as u64) < size {
                check_cancel(Some(cancel))?;
                let chunk = stream
                    .call_method1("read", ((256 * 1024).min(size - pixels.len() as u64),))?
                    .extract::<Vec<u8>>()?;
                if chunk.is_empty() {
                    return Err(invalid("Save screenshot is truncated."));
                }
                pixels.extend(chunk);
            }
            if json!(stamp(&os.call_method1("fstat", (&fd,))?)?) != inspection["stamp"] {
                return Err(invalid("Save changed during screenshot preview."));
            }
            Ok(pixels)
        },
    )?;
    validate(py, &inspection, cancel)?;
    let result = (|| -> PyResult<Bound<'py, PyAny>> {
        let image = py
            .import("PIL.Image")?
            .call_method1(
                "frombytes",
                (
                    "RGBa",
                    (width, height),
                    pyo3::types::PyBytes::new(py, &pixels),
                ),
            )?
            .call_method1("convert", ("RGBA",))?
            .call_method1("convert", ("RGB",))?;
        image.call_method1("thumbnail", ((1024, 1024),))?;
        py.import("numpy")?
            .call_method1("asarray", (image,))?
            .call_method0("copy")
    })()
    .map_err(|error| {
        if error.is_instance_of::<pyo3::exceptions::PyValueError>(py)
            || error.is_instance_of::<pyo3::exceptions::PyOSError>(py)
        {
            invalid("Save screenshot could not be decoded.")
        } else {
            error
        }
    })?;
    validate(py, &inspection, cancel)?;
    Ok(result)
}
fn indices(regular: &[String], light: &[String]) -> BTreeMap<String, String> {
    regular
        .iter()
        .enumerate()
        .map(|(index, name)| (fold(name), format!("{index:02X}")))
        .chain(
            light
                .iter()
                .enumerate()
                .map(|(index, name)| (fold(name), format!("FE:{index:03X}"))),
        )
        .collect()
}
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}
fn finish(mut result: Value) -> String {
    result["same_plugins"] = json!(
        [
            "missing",
            "disabled",
            "added",
            "moved",
            "type_changed",
            "unknown"
        ]
        .iter()
        .all(|key| result[*key].as_array().is_some_and(Vec::is_empty))
    );
    result.to_string()
}
#[pyfunction]
fn save_compare_saved(
    py: Python<'_>,
    left: &str,
    right: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let left = encoding::parse(left)?;
    let right = encoding::parse(right)?;
    validate(py, &left, cancel)?;
    validate(py, &right, cancel)?;
    let regular = strings(&left["regular"]);
    let light = strings(&left["light"]);
    let target_regular = strings(&right["regular"]);
    let target_light = strings(&right["light"]);
    let old = indices(&regular, &light);
    let new = indices(&target_regular, &target_light);
    let mut changed = Vec::new();
    let mut moved = Vec::new();
    let mut missing = Vec::new();
    for name in regular.iter().chain(&light) {
        let key = fold(name);
        if let Some(index) = new.get(&key) {
            if old[&key].starts_with("FE:") != index.starts_with("FE:") {
                changed.push(name);
            }
            if &old[&key] != index {
                moved.push(json!([name, old[&key], index]));
            }
        } else {
            missing.push(name);
        }
    }
    let added = target_regular
        .iter()
        .chain(&target_light)
        .filter(|name| !old.contains_key(&fold(name)))
        .collect::<Vec<_>>();
    let mut issues = vec![
        "Matching plugin names/indices do not establish save-body or runtime compatibility."
            .to_string(),
    ];
    if left["game_version"] != right["game_version"] {
        issues.push(format!(
            "Recorded game versions differ: {} / {}.",
            text(&left, "game_version"),
            text(&right, "game_version")
        ));
    }
    Ok(finish(
        json!({"target":Path::new(text(&right,"path")).file_name().unwrap_or_default().to_string_lossy(),"missing":missing,"disabled":[],"added":added,"moved":moved,"type_changed":changed,"unknown":[],"issues":issues}),
    ))
}
#[pyfunction]
fn save_compare_profile(
    py: Python<'_>,
    inspection: &str,
    catalog: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let inspection = encoding::parse(inspection)?;
    let catalog = encoding::parse(catalog)?;
    validate(py, &inspection, cancel)?;
    if catalog["game"] != "fo4" {
        return Err(invalid(
            "FO4 save comparison requires a Fallout 4 plugin inventory.",
        ));
    }
    let plugins = catalog["plugins"]
        .as_array()
        .ok_or_else(|| invalid("Invalid plugin inventory."))?;
    let installed = plugins
        .iter()
        .map(|v| (fold(text(v, "name")), v))
        .collect::<BTreeMap<_, _>>();
    let mut active = plugins
        .iter()
        .filter(|v| v["enabled"] == true && !text(v, "path").is_empty())
        .collect::<Vec<_>>();
    active.sort_by_key(|v| v["priority"].as_i64().unwrap_or(0));
    let is_light =
        |v: &Value| v["header"]["flags"]["esl"] == true || fold(text(v, "name")).ends_with(".esl");
    let mut regular = Vec::new();
    let mut light = Vec::new();
    let mut unknown = Vec::new();
    for item in &active {
        let name = text(item, "name").to_string();
        if item["header"].is_null() {
            unknown.push(name);
        } else if is_light(item) {
            light.push(name);
        } else {
            regular.push(name);
        }
    }
    let old = indices(
        &strings(&inspection["regular"]),
        &strings(&inspection["light"]),
    );
    let new = indices(&regular, &light);
    let mut missing = Vec::new();
    let mut disabled = Vec::new();
    let mut changed = Vec::new();
    let mut moved = Vec::new();
    for name in strings(&inspection["regular"])
        .into_iter()
        .chain(strings(&inspection["light"]))
    {
        let key = fold(&name);
        match installed.get(&key) {
            None => missing.push(name.clone()),
            Some(item) if text(item, "path").is_empty() => missing.push(name.clone()),
            Some(item) if item["enabled"] != true => disabled.push(name.clone()),
            _ => {}
        }
        if let Some(index) = new.get(&key) {
            if old[&key].starts_with("FE:") != index.starts_with("FE:") {
                changed.push(name.clone());
            }
            if unknown.is_empty() && &old[&key] != index {
                moved.push(json!([name, old[&key], index]));
            }
        }
    }
    let added = active
        .iter()
        .filter(|v| !old.contains_key(&fold(text(v, "name"))))
        .map(|v| text(v, "name"))
        .collect::<Vec<_>>();
    let mut issues = strings(&catalog["issues"]);
    issues.push("Names, enablement and indices are metadata warnings; they do not establish runtime compatibility.".into());
    Ok(finish(
        json!({"target":"Current local profile","missing":missing,"disabled":disabled,"added":added,"moved":moved,"type_changed":changed,"unknown":unknown,"issues":issues}),
    ))
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(save_inspection_validate, m)?)?;
    m.add_function(wrap_pyfunction!(save_inspect, m)?)?;
    m.add_function(wrap_pyfunction!(save_screenshot, m)?)?;
    m.add_function(wrap_pyfunction!(save_compare_saved, m)?)?;
    m.add_function(wrap_pyfunction!(save_compare_profile, m)?)?;
    Ok(())
}
