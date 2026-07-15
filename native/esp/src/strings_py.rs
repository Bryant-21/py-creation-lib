use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::collections::HashMap;
use std::path::Path;

use crate::plugin_runtime::strings::{
    STRING_TABLE_TYPES, find_all_string_table_paths, load_all_string_tables, normalize_language,
    parse_string_table, write_string_table,
};

#[pyfunction]
pub(crate) fn parse_string_table_native(
    py: Python<'_>,
    path: &str,
) -> PyResult<Vec<(u32, String)>> {
    let path = path.to_string();
    py.detach(move || parse_string_table(Path::new(path.as_str())))
        .map(|values| values.into_iter().collect::<Vec<_>>())
        .map_err(PyValueError::new_err)
}

#[pyfunction]
pub(crate) fn write_string_table_native(
    py: Python<'_>,
    path: &str,
    values: Vec<(u32, String)>,
    table_type: &str,
) -> PyResult<String> {
    let owned: HashMap<u32, String> = values.into_iter().collect();
    let path = path.to_string();
    let table_type = table_type.to_string();
    py.detach(move || -> Result<String, String> {
        let target = Path::new(path.as_str());
        write_string_table(target, &owned, table_type.as_str())?;
        Ok(target.to_string_lossy().into_owned())
    })
    .map_err(PyValueError::new_err)
}

#[pyfunction]
#[pyo3(signature = (plugin_name, strings_dir, language=None))]
pub(crate) fn load_string_tables_native(
    py: Python<'_>,
    plugin_name: &str,
    strings_dir: Option<&str>,
    language: Option<&str>,
) -> PyResult<Vec<(u32, String)>> {
    let Some(strings_dir) = strings_dir else {
        return Ok(Vec::new());
    };
    let plugin_name = plugin_name.to_string();
    let strings_dir = strings_dir.to_string();
    let language = language.map(str::to_string);
    let values = py.detach(move || {
        let root = Path::new(strings_dir.as_str());
        if !root.is_dir() {
            return HashMap::new();
        }
        let all_paths = find_all_string_table_paths(plugin_name.as_str(), root);
        if all_paths.is_empty() {
            return HashMap::new();
        }
        let wanted = language.as_deref().and_then(normalize_language);
        let chosen_language = if let Some(wanted) = wanted.as_ref() {
            if all_paths.contains_key(wanted) {
                Some(wanted.clone())
            } else {
                None
            }
        } else {
            None
        };
        let chosen_language = chosen_language
            .or_else(|| {
                for fallback in ["en", "english"] {
                    if all_paths.contains_key(fallback) {
                        return Some(fallback.to_string());
                    }
                }
                None
            })
            .or_else(|| {
                let mut keys: Vec<&String> = all_paths.keys().collect();
                keys.sort_by(|a, b| {
                    let ua = a.matches('_').count();
                    let ub = b.matches('_').count();
                    ua.cmp(&ub).then_with(|| a.cmp(b))
                });
                keys.first().map(|s| (*s).clone())
            });
        let Some(chosen) = chosen_language else {
            return HashMap::new();
        };
        let tables = &all_paths[&chosen];
        let mut values = HashMap::new();
        for table_type in STRING_TABLE_TYPES.iter() {
            let Some(path) = tables.get(*table_type) else {
                continue;
            };
            let loaded = match parse_string_table(path) {
                Ok(values) => values,
                Err(_) => continue,
            };
            for (string_id, text) in loaded {
                values.entry(string_id).or_insert(text);
            }
        }
        values
    });
    Ok(values.into_iter().collect())
}

#[pyfunction]
#[pyo3(signature = (plugin_name, strings_dir))]
pub(crate) fn load_all_string_tables_native(
    py: Python<'_>,
    plugin_name: &str,
    strings_dir: Option<&str>,
) -> PyResult<(Vec<(String, u32, String)>, Vec<(u32, String)>)> {
    let Some(strings_dir) = strings_dir else {
        return Ok((Vec::new(), Vec::new()));
    };
    let plugin_name = plugin_name.to_string();
    let strings_dir = strings_dir.to_string();
    let (by_language, table_types) = py.detach(move || {
        load_all_string_tables(plugin_name.as_str(), Path::new(strings_dir.as_str()))
    });
    let values = by_language
        .into_iter()
        .flat_map(|(language, inner)| {
            inner
                .into_iter()
                .map(move |(string_id, text)| (language.clone(), string_id, text))
        })
        .collect::<Vec<_>>();
    Ok((values, table_types.into_iter().collect()))
}
