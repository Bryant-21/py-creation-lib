use crate::{
    check_cancel,
    executables::text,
    history_prune::{py_path, stamp},
    invalid,
    python_io::with_context,
    reject_links, relative_path,
    transactions::transaction_key,
};
use pyo3::{prelude::*, types::PyDict};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use unicode_casefold::UnicodeCaseFold;
pub fn fold(value: &str) -> String {
    value.case_fold().collect()
}
fn game(library: &Bound<'_, PyAny>, profile: &str) -> PyResult<String> {
    Ok(crate::nexus::nexus_game_domain(
        &library
            .call_method1("profile_snapshot", (profile,))?
            .get_item(0)?
            .getattr("files")?
            .call_method1("get", ("_modbox_game", ""))?
            .extract::<String>()?,
    ))
}
fn identity(py: Python<'_>, path: &Path) -> PyResult<Value> {
    if !path.exists() {
        return Ok(Value::Null);
    }
    let stamp = stamp(&py_path(py, path)?.call_method0("stat")?)?;
    Ok(json!([stamp.0, stamp.1]))
}
fn inventory(
    py: Python<'_>,
    profile: &str,
    game: &str,
    root: &Path,
    external: bool,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<Value> {
    reject_links(root)?;
    let before = identity(py, root)?;
    let mut result = Vec::new();
    let mut found = BTreeMap::new();
    if root.is_dir() {
        for (count, entry) in std::fs::read_dir(root)?.enumerate() {
            check_cancel(Some(cancel))?;
            if count >= 100_000 {
                return Err(invalid(
                    "Save directory exceeds its 100,000-entry inventory limit.",
                ));
            }
            let entry = entry?;
            let path = entry.path();
            let extension = fold(&path.extension().unwrap_or_default().to_string_lossy());
            if !["fos", "f4se", "ess", "skse", "sfs"].contains(&extension.as_str()) {
                continue;
            }
            reject_links(&path)?;
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let key = fold(&name);
            if found.contains_key(&key) {
                return Err(invalid("Save names are ambiguous ignoring case."));
            }
            let info = stamp(&py_path(py, &path)?.call_method0("stat")?)?;
            found.insert(key,json!({"name":relative_path(&name,false)?,"path":path.to_string_lossy(),"size":info.2,"modified":info.3,"companion":"","identity":[info.0,info.1]}));
            if found.len() > 10_000 {
                return Err(invalid("Save inventory exceeds its 10,000-file limit."));
            }
        }
        for (name, item) in &found {
            check_cancel(Some(cancel))?;
            if name.ends_with(".f4se") || name.ends_with(".skse") {
                continue;
            }
            let mut item = item.clone();
            if !name.ends_with(".sfs") {
                let companion = Path::new(name)
                    .with_extension(if name.ends_with(".fos") {
                        "f4se"
                    } else {
                        "skse"
                    })
                    .to_string_lossy()
                    .into_owned();
                if let Some(companion) = found.get(&companion) {
                    item["companion"] = companion["path"].clone();
                }
            }
            result.push(item);
        }
    }
    check_cancel(Some(cancel))?;
    reject_links(root)?;
    if before != identity(py, root)? {
        return Err(invalid(
            "Save folder identity changed during inventory. Refresh it.",
        ));
    }
    if !progress.is_none() {
        let count = py
            .import("builtins")?
            .call_method1("format", (result.len(), ",d"))?
            .extract::<String>()?;
        progress.call1((
            1,
            format!(
                "Indexed {count} {} saves",
                if external { "source" } else { "owned" }
            ),
        ))?;
    }
    check_cancel(Some(cancel))?;
    result.sort_by(|a, b| {
        b["modified"]
            .as_i64()
            .cmp(&a["modified"].as_i64())
            .then_with(|| fold(text(a, "name")).cmp(&fold(text(b, "name"))))
    });
    Ok(
        json!({"profile_id":profile,"game":game,"root":root.to_string_lossy(),"files":result,"external":external,"identity":before}),
    )
}
#[pyfunction]
fn saves_inventory(
    library: &Bound<'_, PyAny>,
    profile: &str,
    folder: Option<PathBuf>,
    cancel: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<String> {
    let py = library.py();
    let game = game(library, profile)?;
    let external = folder.is_some();
    let root = if let Some(folder) = folder {
        let root = py_path(py, &folder)?.call_method0("absolute")?;
        if !py
            .import("creation_lib.mod_manager.mo2")?
            .call_method1("local_mod_target", (&root,))?
            .is_truthy()?
        {
            return Err(invalid(
                "Choose an ordinary save folder on a local drive, not a root or network share.",
            ));
        }
        let path = root.extract::<PathBuf>()?;
        reject_links(&path)?;
        if !path.is_dir() {
            return Err(invalid("Choose an ordinary source/global save folder."));
        }
        path
    } else {
        library
            .getattr("root")?
            .extract::<PathBuf>()?
            .join("runtime")
            .join("profiles")
            .join(transaction_key(profile)?)
            .join("saves")
    };
    Ok(inventory(py, profile, &game, &root, external, cancel, progress)?.to_string())
}
pub struct Reader<'py> {
    pub stream: Bound<'py, PyAny>,
    pub cancel: Bound<'py, PyAny>,
    pub boundary: u64,
    pub basic: bool,
    pub notices: Vec<String>,
}
impl Reader<'_> {
    pub fn position(&self) -> PyResult<u64> {
        self.stream.call_method0("tell")?.extract()
    }
    pub fn read(&self, size: u64) -> PyResult<Vec<u8>> {
        check_cancel(Some(&self.cancel))?;
        if self
            .position()?
            .checked_add(size)
            .is_none_or(|end| end > self.boundary)
        {
            return Err(invalid(if self.basic {
                "Save basic header exceeds its 64 KiB inspection limit."
            } else {
                "Save plugin metadata is truncated or exceeds its declared block."
            }));
        }
        let data = self
            .stream
            .call_method1("read", (size,))?
            .extract::<Vec<u8>>()?;
        if data.len() as u64 != size {
            return Err(invalid(if self.basic {
                "Save basic header is truncated."
            } else {
                "Save plugin metadata is truncated."
            }));
        }
        Ok(data)
    }
    pub fn u8(&self) -> PyResult<u8> {
        Ok(self.read(1)?[0])
    }
    pub fn u16(&self) -> PyResult<u16> {
        Ok(u16::from_le_bytes(self.read(2)?.try_into().unwrap()))
    }
    pub fn u32(&self) -> PyResult<u32> {
        Ok(u32::from_le_bytes(self.read(4)?.try_into().unwrap()))
    }
    pub fn u64(&self) -> PyResult<u64> {
        Ok(u64::from_le_bytes(self.read(8)?.try_into().unwrap()))
    }
    pub fn text(&mut self) -> PyResult<String> {
        let length = self.u16()?;
        if length > 4096 {
            return Err(invalid(if self.basic {
                "Save header text exceeds its 4 KiB field limit."
            } else {
                "Save metadata text exceeds its 4 KiB field limit."
            }));
        }
        let bytes = self.read(length as u64)?;
        let value = match String::from_utf8(bytes.clone()) {
            Ok(value) => value,
            Err(_) if self.basic => {
                return Err(invalid("Save header contains invalid UTF-8 text."));
            }
            Err(_) => {
                let value = pyo3::types::PyBytes::new(self.stream.py(), &bytes)
                    .call_method1("decode", ("cp1252",))
                    .and_then(|v| v.extract::<String>())
                    .map_err(|_| invalid("Save plugin text uses an unsupported encoding."))?;
                if self.notices.is_empty() {
                    self.notices.push("Non-UTF-8 plugin metadata was decoded as Windows-1252; other legacy encodings are unsupported.".into());
                }
                value
            }
        };
        if value.chars().any(|c| c < ' ') {
            return Err(invalid(if self.basic {
                "Save header text contains control characters."
            } else {
                "Save plugin metadata contains control characters."
            }));
        }
        Ok(value)
    }
}
pub fn header(save: &Bound<'_, PyAny>, cancel: &Bound<'_, PyAny>) -> PyResult<Value> {
    let py = save.py();
    let path = PathBuf::from(save.getattr("path")?.extract::<String>()?);
    reject_links(&path)?;
    let path_py = py_path(py, &path)?;
    let before = stamp(&path_py.call_method0("stat")?)?;
    let size = save.getattr("size")?.extract::<u64>()?;
    let modified = save.getattr("modified")?.extract::<i64>()?;
    let identity = save.getattr("identity")?;
    if (before.2, before.3) != (size, modified)
        || (!identity.is_none() && identity.extract::<(u64, u64)>()? != (before.0, before.1))
    {
        return Err(invalid(
            "Save changed after inventory. Refresh owned saves first.",
        ));
    }
    check_cancel(Some(cancel))?;
    let result = with_context(&path_py.call_method1("open", ("rb",))?, |stream| {
        let mut reader = Reader {
            stream: stream.clone(),
            cancel: cancel.clone(),
            boundary: 65536,
            basic: true,
            notices: Vec::new(),
        };
        if reader.read(12)? != b"FO4_SAVEGAME" {
            return Err(invalid(
                "This file has no supported Fallout 4 save signature.",
            ));
        }
        let size = reader.u32()?;
        let version = reader.u32()?;
        let ordinal = reader.u32()?;
        if size == 0 || size > 65536 {
            return Err(invalid("Save declares an unsupported basic header size."));
        }
        let player = reader.text()?;
        let level = reader.u32()?;
        let location = reader.text()?;
        let played = reader.text()?;
        let race = reader.text()?;
        reader.read(10)?;
        let ticks = reader.u64()?;
        if reader.position()? > 16 + size as u64 {
            return Err(invalid(
                "Save fields extend beyond the declared basic header.",
            ));
        }
        let datetime = py.import("datetime")?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("tzinfo", datetime.getattr("UTC")?)?;
        let epoch = datetime
            .getattr("datetime")?
            .call((1601, 1, 1), Some(&kwargs))?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("microseconds", ticks / 10)?;
        let created = (|| -> PyResult<String> {
            epoch
                .call_method1(
                    "__add__",
                    (datetime.getattr("timedelta")?.call((), Some(&kwargs))?,),
                )?
                .call_method0("isoformat")?
                .extract()
        })();
        let created =
            created.map_err(|_| invalid("Save declares an invalid creation timestamp."))?;
        Ok(
            json!({"header_version":version,"number":ordinal,"player":player,"level":level,"location":location,"played":played,"race":race,"created":created,"coverage":"Basic FO4 display fields only. Plugin compatibility and save-body validity are unverified."}),
        )
    })?;
    let after = stamp(&path_py.call_method0("stat")?)?;
    check_cancel(Some(cancel))?;
    reject_links(&path)?;
    if (after.2, after.3) != (size, modified) || (after.0, after.1) != (before.0, before.1) {
        return Err(invalid("Save changed while its header was inspected."));
    }
    Ok(result)
}
#[pyfunction]
fn saves_fo4_header(save: &Bound<'_, PyAny>, cancel: &Bound<'_, PyAny>) -> PyResult<String> {
    Ok(header(save, cancel)?.to_string())
}
fn global_folder(
    library: &Bound<'_, PyAny>,
    profile: &str,
    known_folder: &Bound<'_, PyAny>,
) -> PyResult<PathBuf> {
    if game(library, profile)? != "fallout4" {
        return Err(invalid(
            "Default global save discovery currently supports Fallout 4. Choose another game's folder explicitly.",
        ));
    }
    let documents = known_folder
        .call1(("FDD39AD0-238F-46AF-ADB4-6C85480369C7",))?
        .extract::<PathBuf>()?;
    if !documents.is_absolute()
        || !library
            .py()
            .import("creation_lib.mod_manager.mo2")?
            .call_method1("local_mod_target", (py_path(library.py(), &documents)?,))?
            .is_truthy()?
    {
        return Err(invalid(
            "Windows Documents must be an ordinary absolute folder on a local drive. Choose a save folder explicitly.",
        ));
    }
    let candidate = documents.join("My Games").join("Fallout4").join("Saves");
    reject_links(&candidate)?;
    Ok(candidate)
}
#[pyfunction]
fn saves_global_folder(
    library: &Bound<'_, PyAny>,
    profile: &str,
    known_folder: &Bound<'_, PyAny>,
) -> PyResult<PathBuf> {
    global_folder(library, profile, known_folder)
}
#[pyfunction]
fn saves_global_inventory<'py>(
    library: &Bound<'py, PyAny>,
    profile: &str,
    known_folder: &Bound<'py, PyAny>,
    inventory_folder: &Bound<'py, PyAny>,
    cancel: &Bound<'py, PyAny>,
    progress: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    check_cancel(Some(cancel))?;
    let candidate = global_folder(library, profile, known_folder)?;
    check_cancel(Some(cancel))?;
    if !candidate.is_dir() {
        return Err(invalid(
            "The default Fallout 4 global Saves folder is absent. Choose the configured save folder explicitly; no directory was created.",
        ));
    }
    let result = inventory_folder.call1((
        library,
        profile,
        py_path(library.py(), &candidate)?,
        cancel,
        progress,
    ))?;
    if result.getattr("game")?.extract::<String>()? != "fallout4"
        || game(library, profile)? != "fallout4"
    {
        return Err(invalid(
            "The profile game changed during global save discovery. Select the profile again.",
        ));
    }
    Ok(result)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(saves_inventory, m)?)?;
    m.add_function(wrap_pyfunction!(saves_fo4_header, m)?)?;
    m.add_function(wrap_pyfunction!(saves_global_folder, m)?)?;
    m.add_function(wrap_pyfunction!(saves_global_inventory, m)?)?;
    Ok(())
}
