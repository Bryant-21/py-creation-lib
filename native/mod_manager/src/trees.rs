use crate::{check_cancel, invalid, saves::fold};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::{BTreeMap, BTreeSet};

#[pyclass]
struct DataTree {
    entries: Vec<Py<PyAny>>,
    paths: Vec<String>,
    children: BTreeMap<String, Vec<(String, Option<usize>)>>,
    cache: Option<(BTreeSet<String>, String, Py<PyAny>)>,
}
#[pymethods]
impl DataTree {
    #[new]
    fn new(entries: &Bound<'_, PyAny>, cancel: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut stored = Vec::new();
        let mut paths = Vec::new();
        let mut children = BTreeMap::<String, BTreeMap<(u8, String), Option<usize>>>::from([(
            "".into(),
            BTreeMap::new(),
        )]);
        let mut labels = BTreeMap::<String, String>::new();
        for entry in entries.try_iter()? {
            check_cancel(Some(cancel))?;
            let entry = entry?;
            let path: String = entry.getattr("path")?.extract()?;
            let parts = path.split('/').collect::<Vec<_>>();
            let mut parent = String::new();
            for part in &parts[..parts.len().saturating_sub(1)] {
                let path = if parent.is_empty() {
                    part.to_string()
                } else {
                    format!("{parent}/{part}")
                };
                let canonical = fold(&path);
                let label = labels.entry(canonical.clone()).or_insert(path).clone();
                children
                    .entry(fold(&parent))
                    .or_default()
                    .insert((0, canonical.clone()), None);
                children.entry(canonical).or_default();
                parent = label;
            }
            children
                .entry(fold(&parent))
                .or_default()
                .insert((1, path.clone()), Some(stored.len()));
            stored.push(entry.unbind());
            paths.push(path);
            if labels.len() > 500_000 {
                return Err(invalid(
                    "Virtual Data tree exceeds its 500,000-directory limit.",
                ));
            }
        }
        let children = children
            .into_iter()
            .map(|(parent, values)| {
                let mut values = values.into_iter().collect::<Vec<_>>();
                values.sort_by_key(|((kind, key), _)| (*kind, fold(key)));
                (
                    parent,
                    values
                        .into_iter()
                        .map(|((_, key), index)| {
                            (
                                if index.is_some() {
                                    key.clone()
                                } else {
                                    labels[&key].clone()
                                },
                                index,
                            )
                        })
                        .collect(),
                )
            })
            .collect();
        Ok(Self {
            entries: stored,
            paths,
            children,
            cache: None,
        })
    }
    fn visible(
        &mut self,
        py: Python<'_>,
        expanded: Vec<String>,
        query: &str,
    ) -> PyResult<Py<PyAny>> {
        let expanded = expanded.iter().map(|p| fold(p)).collect::<BTreeSet<_>>();
        let query = fold(query);
        if let Some((previous, text, rows)) = &self.cache {
            if previous == &expanded && text == &query {
                return Ok(rows.clone_ref(py));
            }
        }
        let mut included = BTreeSet::new();
        if !query.is_empty() {
            for path in &self.paths {
                if fold(path).contains(&query) {
                    included.insert(fold(path));
                    let parts = path.split('/').collect::<Vec<_>>();
                    for end in 1..parts.len() {
                        included.insert(fold(&parts[..end].join("/")));
                    }
                }
            }
        }
        let mut rows = Vec::new();
        let mut stack = self.children[""]
            .iter()
            .rev()
            .map(|(p, i)| (p.clone(), *i, 0usize))
            .collect::<Vec<_>>();
        let row = py
            .import("creation_lib.mod_manager.data_tree")?
            .getattr("Row")?;
        while let Some((path, index, depth)) = stack.pop() {
            if !query.is_empty() && !included.contains(&fold(&path)) {
                continue;
            }
            let entry = index
                .map(|i| self.entries[i].clone_ref(py))
                .unwrap_or_else(|| py.None());
            rows.push(row.call1((&path, depth, entry))?);
            if index.is_none() && (!query.is_empty() || expanded.contains(&fold(&path))) {
                if let Some(children) = self.children.get(&fold(&path)) {
                    stack.extend(
                        children
                            .iter()
                            .rev()
                            .map(|(p, i)| (p.clone(), *i, depth + 1)),
                    );
                }
            }
        }
        let rows = PyList::new(py, rows)?.into_any().unbind();
        self.cache = Some((expanded, query, rows.clone_ref(py)));
        Ok(rows)
    }
}
struct Node {
    key: String,
    name: String,
    depth: usize,
    row: Option<Py<PyAny>>,
    children: Vec<usize>,
    priority: i64,
    count: usize,
}
fn materialize<'py>(
    py: Python<'py>,
    index: usize,
    nodes: &[Node],
    groups: &Bound<'py, PyDict>,
) -> PyResult<Bound<'py, PyAny>> {
    let node = &nodes[index];
    let children = node
        .children
        .iter()
        .map(|i| materialize(py, *i, nodes, groups))
        .collect::<PyResult<Vec<_>>>()?;
    let row = node
        .row
        .as_ref()
        .map(|v| v.clone_ref(py))
        .unwrap_or_else(|| py.None());
    let result = py
        .import("creation_lib.mod_manager.tree")?
        .getattr("Node")?
        .call1((
            &node.key,
            &node.name,
            node.depth,
            row,
            PyList::new(py, children)?,
            node.priority,
            node.count,
        ))?;
    if let Some(path) = node.key.strip_prefix("group:") {
        groups.set_item(path, &result)?;
    }
    Ok(result)
}
#[pyfunction]
fn mod_tree_build(all: &Bound<'_, PyAny>, matching: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = all.py();
    let mut separators = BTreeMap::new();
    let mut priorities = BTreeMap::<String, i64>::new();
    for row in all.try_iter()? {
        let row = row?;
        let group: String = row.getattr("group")?.extract()?;
        let priority: i64 = row.getattr("priority")?.extract()?;
        if row.getattr("kind")?.extract::<String>()? == "separator" && !group.is_empty() {
            separators.insert(group.clone(), row.clone().unbind());
        }
        if !group.is_empty() {
            let parts = group.split('/').collect::<Vec<_>>();
            for end in 1..=parts.len() {
                priorities
                    .entry(parts[..end].join("/"))
                    .and_modify(|v| *v = (*v).min(priority))
                    .or_insert(priority);
            }
        }
    }
    let mut roots = Vec::new();
    let mut groups = BTreeMap::<String, usize>::new();
    let mut nodes = Vec::<Node>::new();
    for row in matching.try_iter()? {
        let row = row?;
        let group: String = row.getattr("group")?.extract()?;
        let priority: i64 = row.getattr("priority")?.extract()?;
        let separator = row.getattr("kind")?.extract::<String>()? == "separator";
        let mut parent: Option<usize> = None;
        let mut path = String::new();
        let parts = if group.is_empty() {
            Vec::new()
        } else {
            group.split('/').collect::<Vec<_>>()
        };
        for (depth, name) in parts.iter().enumerate() {
            path = format!("{path}/{name}").trim_matches('/').to_string();
            let index = if let Some(i) = groups.get(&path) {
                *i
            } else {
                let index = nodes.len();
                nodes.push(Node {
                    key: format!("group:{path}"),
                    name: name.to_string(),
                    depth,
                    row: separators.get(&path).map(|v| v.clone_ref(py)),
                    children: Vec::new(),
                    priority: *priorities.get(&path).unwrap_or(&priority),
                    count: 0,
                });
                if let Some(parent) = parent {
                    nodes[parent].children.push(index);
                } else {
                    roots.push(index);
                }
                groups.insert(path.clone(), index);
                index
            };
            if !separator {
                nodes[index].count += 1;
            }
            parent = Some(index);
        }
        if !separator {
            let index = nodes.len();
            nodes.push(Node {
                key: row.getattr("id")?.extract()?,
                name: row.getattr("name")?.extract()?,
                depth: parts.len(),
                row: Some(row.unbind()),
                children: Vec::new(),
                priority,
                count: 0,
            });
            if let Some(parent) = parent {
                nodes[parent].children.push(index);
            } else {
                roots.push(index);
            }
        }
    }
    fn order(indices: &mut [usize], nodes: &[Node]) {
        let mut groups = indices
            .iter()
            .copied()
            .filter(|i| nodes[*i].key.starts_with("group:"))
            .collect::<Vec<_>>();
        groups.sort_by(|a, b| {
            (nodes[*a].priority, &nodes[*a].key).cmp(&(nodes[*b].priority, &nodes[*b].key))
        });
        let mut groups = groups.into_iter();
        for index in indices {
            if nodes[*index].key.starts_with("group:") {
                *index = groups.next().unwrap();
            }
        }
    }
    order(&mut roots, &nodes);
    for index in 0..nodes.len() {
        let mut children = std::mem::take(&mut nodes[index].children);
        order(&mut children, &nodes);
        nodes[index].children = children;
    }
    let groups = PyDict::new(py);
    let roots = roots
        .iter()
        .map(|i| materialize(py, *i, &nodes, &groups))
        .collect::<PyResult<Vec<_>>>()?;
    Ok((PyList::new(py, roots)?, groups)
        .into_pyobject(py)?
        .into_any()
        .unbind())
}
#[pyfunction]
fn mod_tree_visible(
    roots: &Bound<'_, PyAny>,
    closed: BTreeSet<String>,
    reveal: bool,
) -> PyResult<Py<PyAny>> {
    let py = roots.py();
    let mut result = Vec::new();
    let mut stack = roots.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    stack.reverse();
    while let Some(node) = stack.pop() {
        let key: String = node.getattr("key")?.extract()?;
        if reveal || !closed.contains(&key) {
            let mut children = node
                .getattr("children")?
                .try_iter()?
                .collect::<PyResult<Vec<_>>>()?;
            children.reverse();
            stack.extend(children);
        }
        result.push(node);
    }
    Ok(PyList::new(py, result)?.into_any().unbind())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<DataTree>()?;
    m.add_function(wrap_pyfunction!(mod_tree_build, m)?)?;
    m.add_function(wrap_pyfunction!(mod_tree_visible, m)?)?;
    Ok(())
}
