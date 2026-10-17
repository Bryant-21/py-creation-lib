use crate::{check_cancel, encoding, nexus::failure};
use pyo3::prelude::*;
use pyo3::types::PyTuple;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use unicode_casefold::UnicodeCaseFold;

pub fn identifier(value: &Value) -> PyResult<u64> {
    value
        .as_u64()
        .filter(|v| *v > 0 && *v < 1_000_000_000_000)
        .ok_or_else(|| failure("Nexus file update identity is invalid."))
}
#[pyfunction]
fn nexus_file_identifier(value: &str) -> PyResult<u64> {
    identifier(&encoding::parse(value)?)
}

#[pyclass]
struct NexusFileState {
    files: Vec<(u64, bool)>,
    graph: BTreeMap<u64, BTreeSet<u64>>,
    updatable: BTreeSet<u64>,
    cache: VecDeque<(u64, Py<PyTuple>)>,
    objects: Py<PyTuple>,
}

#[pymethods]
impl NexusFileState {
    #[new]
    fn new(files: &str, updates: Vec<(u64, u64)>, objects: Py<PyTuple>) -> PyResult<Self> {
        let files = encoding::parse(files)?;
        let files = files
            .as_array()
            .ok_or_else(|| failure("Invalid file state."))?
            .iter()
            .map(|file| {
                let category = file["category"]
                    .as_str()
                    .unwrap_or("")
                    .case_fold()
                    .collect::<String>();
                Ok((
                    identifier(&file["id"])?,
                    !matches!(category.as_str(), "deleted" | "archived"),
                ))
            })
            .collect::<PyResult<Vec<_>>>()?;
        let mut graph: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
        let mut reverse: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
        for (old, new) in updates {
            graph.entry(old).or_default().insert(new);
            reverse.entry(new).or_default().insert(old);
        }
        let mut stack: Vec<_> = files
            .iter()
            .filter(|(id, active)| *active && !graph.contains_key(id))
            .map(|(id, _)| *id)
            .collect();
        let mut visited = BTreeSet::new();
        let mut updatable = BTreeSet::new();
        while let Some(current) = stack.pop() {
            if !visited.insert(current) {
                continue;
            }
            if let Some(parents) = reverse.get(&current) {
                updatable.extend(parents);
                stack.extend(parents);
            }
        }
        Ok(Self {
            files,
            graph,
            updatable,
            cache: VecDeque::new(),
            objects,
        })
    }
    #[getter]
    fn updatable(&self) -> BTreeSet<u64> {
        self.updatable.clone()
    }
    #[getter]
    fn cached(&self) -> Vec<u64> {
        self.cache.iter().map(|(id, _)| *id).collect()
    }
    fn has_replacement(&self, id: u64) -> bool {
        id != 0 && self.updatable.contains(&id)
    }
    fn replacements(&mut self, py: Python<'_>, id: u64) -> PyResult<Py<PyTuple>> {
        if id == 0 || !self.graph.contains_key(&id) {
            return Ok(PyTuple::empty(py).unbind());
        }
        if let Some(index) = self.cache.iter().position(|(key, _)| *key == id) {
            let cached = self.cache.remove(index).unwrap();
            let result = cached.1.clone_ref(py);
            self.cache.push_back(cached);
            return Ok(result);
        }
        let mut found = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            if !visited.insert(current) {
                continue;
            }
            if let Some(following) = self.graph.get(&current) {
                stack.extend(following);
            } else if current != id {
                found.insert(current);
            }
        }
        let result: Vec<_> = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, (file, active))| *active && found.contains(file))
            .map(|(i, _)| self.objects.bind(py).get_item(i))
            .collect::<PyResult<_>>()?;
        let result = PyTuple::new(py, result)?.unbind();
        self.cache.push_back((id, result.clone_ref(py)));
        if self.cache.len() > 64 {
            self.cache.pop_front();
        }
        Ok(result)
    }
}

#[pyfunction]
#[pyo3(signature = (data, cancel=None))]
fn nexus_file_state(data: &str, cancel: Option<&Bound<'_, PyAny>>) -> PyResult<String> {
    let data = encoding::parse(data)?;
    let Some(records) = data["files"].as_array().filter(|v| v.len() <= 10_000) else {
        return Err(failure(
            "Nexus file update inventory is invalid or exceeds its safety limit.",
        ));
    };
    let empty = json!([]);
    let Some(updates) = data
        .get("file_updates")
        .unwrap_or(&empty)
        .as_array()
        .filter(|v| v.len() <= 20_000)
    else {
        return Err(failure(
            "Nexus file update inventory is invalid or exceeds its safety limit.",
        ));
    };
    let mut files = Vec::new();
    let mut seen = BTreeSet::new();
    for record in records {
        check_cancel(cancel)?;
        if !record.is_object() {
            return Err(failure("Nexus file metadata is invalid."));
        }
        let id = identifier(&record["file_id"])?;
        let raw = record["file_name"]
            .as_str()
            .ok_or_else(|| failure("Nexus file metadata has no usable filename."))?;
        let name = crate::relative_path(raw, false)?;
        if !seen.insert(id) || name.contains('/') {
            return Err(failure(
                "Nexus file metadata has duplicate identities or unsafe filenames.",
            ));
        }
        let version = record
            .get("version")
            .map(|v| v.as_str())
            .unwrap_or(Some(""));
        let category = record
            .get("category_name")
            .map(|v| v.as_str())
            .unwrap_or(Some("Unknown"));
        let (Some(version), Some(category)) = (version, category) else {
            return Err(failure(
                "Nexus version/category metadata exceeds its safety limit.",
            ));
        };
        if version.chars().count() > 256 || category.chars().count() > 100 {
            return Err(failure(
                "Nexus version/category metadata exceeds its safety limit.",
            ));
        }
        files.push(json!({"id":id,"name":name,"version":version,"category":category}));
    }
    let mut edges = BTreeSet::new();
    for record in updates {
        check_cancel(cancel)?;
        if !record.is_object() {
            return Err(failure("Nexus file update metadata is invalid."));
        }
        edges.insert((
            identifier(&record["old_file_id"])?,
            identifier(&record["new_file_id"])?,
        ));
    }
    let mut graph: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    let mut indegree: BTreeMap<u64, usize> = BTreeMap::new();
    for (old, new) in &edges {
        graph.entry(*old).or_default().push(*new);
        indegree.entry(*old).or_default();
        *indegree.entry(*new).or_default() += 1;
    }
    let mut ready: Vec<_> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut count = 0;
    while let Some(node) = ready.pop() {
        check_cancel(cancel)?;
        count += 1;
        for child in graph.get(&node).into_iter().flatten() {
            let degree = indegree.get_mut(child).unwrap();
            *degree -= 1;
            if *degree == 0 {
                ready.push(*child);
            }
        }
    }
    if count != indegree.len() {
        return Err(failure(
            "Nexus file update history contains a cycle; replacement mapping is unknown.",
        ));
    }
    check_cancel(cancel)?;
    Ok(json!({"files":files,"updates":edges}).to_string())
}

#[pyfunction]
#[pyo3(signature = (data, endorsement, cancel=None))]
fn nexus_account_snapshot(
    data: &str,
    endorsement: bool,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let data = encoding::parse(data)?;
    let records = data
        .as_array()
        .filter(|v| v.len() <= 20000)
        .ok_or_else(|| {
            failure(if endorsement {
                "Nexus endorsement inventory is invalid."
            } else {
                "Nexus tracking inventory is invalid."
            })
        })?;
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for record in records {
        check_cancel(cancel)?;
        if !record.is_object() {
            return Err(failure(if endorsement {
                "Nexus endorsement identity is invalid."
            } else {
                "Nexus account tracking metadata is invalid."
            }));
        }
        let domain = if endorsement {
            record["domain_name"].as_str()
        } else {
            record
                .get("domain_name")
                .or_else(|| record.get("game_domain_name"))
                .map(|v| v.as_str())
                .unwrap_or(Some(""))
        };
        let domain = domain.ok_or_else(|| {
            failure(if endorsement {
                "Nexus endorsement identity is invalid."
            } else {
                "Nexus tracked-mod game identity is invalid."
            })
        })?;
        let id = identifier(&record["mod_id"])?;
        if !seen.insert((domain, id)) {
            return Err(failure(if endorsement {
                "Nexus returned duplicate endorsement identities."
            } else {
                "Nexus returned duplicate tracking identities."
            }));
        }
        if endorsement {
            let status = record["status"]
                .as_str()
                .filter(|v| matches!(*v, "Endorsed" | "Abstained" | "Undecided"))
                .unwrap_or("Unknown");
            result.push(json!([domain, id, status]));
        } else {
            result.push(json!([domain, id]));
        }
    }
    check_cancel(cancel)?;
    Ok(json!(result).to_string())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<NexusFileState>()?;
    m.add_function(wrap_pyfunction!(nexus_file_identifier, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_file_state, m)?)?;
    m.add_function(wrap_pyfunction!(nexus_account_snapshot, m)?)?;
    Ok(())
}
