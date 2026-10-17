use crate::{controller::callback, invalid, saves::fold};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyString, PyTuple},
};
#[pyfunction]
fn controller_status_rows(manager: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = manager.py();
    let builtins = py.import("builtins")?;
    let rows = manager.getattr("rows")?;
    let conflicts = manager.getattr("conflict_index")?;
    let key = PyTuple::new(
        py,
        [
            builtins.call_method1("id", (&rows,))?,
            builtins.call_method1("id", (&conflicts,))?,
            manager.getattr("_status_revision")?,
        ],
    )?;
    if !manager.getattr("_status_key")?.eq(&key)? {
        let statuses = py
            .import("creation_lib.mod_manager.status_query")?
            .call_method1(
                "inventory",
                (
                    &rows,
                    conflicts,
                    manager.getattr("updates")?,
                    manager.getattr("nexus_states")?,
                ),
            )?;
        manager.setattr("_status_rows", statuses)?;
        manager.setattr("_status_key", key)?;
        manager.setattr("_status_sources", (rows, manager.getattr("file_index")?))?;
    }
    Ok(manager.getattr("_status_rows")?.unbind())
}
#[pyfunction]
fn controller_filtered(
    manager: &Bound<'_, PyAny>,
    query: &str,
    group: &str,
    enabled_only: bool,
    sort_by: &str,
    descending: bool,
    origin: &str,
    category: &Bound<'_, PyAny>,
    statuses: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = manager.py();
    let status_module = py.import("creation_lib.mod_manager.status_query")?;
    let filters = status_module.call_method1("normalized", (statuses,))?;
    let status_sort =
        ["status", "conflicts", "updates", "endorsement", "tracking"].contains(&sort_by);
    let category = if category.is_none() {
        py.None().into_bound(py)
    } else {
        category.call_method0("casefold")?
    };
    let builtins = py.import("builtins")?;
    let key = PyList::empty(py);
    key.append(builtins.call_method1("id", (manager.getattr("rows")?,))?)?;
    key.append(fold(query))?;
    key.append(group)?;
    key.append(enabled_only)?;
    key.append(sort_by)?;
    key.append(descending)?;
    key.append(origin)?;
    key.append(&category)?;
    key.append(builtins.call_method1("tuple", (filters.call_method0("items")?,))?)?;
    key.append(builtins.call_method1("id", (manager.getattr("conflict_index")?,))?)?;
    key.append(manager.getattr("_status_revision")?)?;
    let key = key.to_tuple();
    if manager.getattr("_filter_key")?.eq(&key)? {
        return Ok(manager.getattr("_filtered")?.unbind());
    }
    let mut needs_status = status_sort;
    for v in filters.call_method0("values")?.try_iter()? {
        if !v?.eq("all")? {
            needs_status = true;
            break;
        }
    }
    let states = if needs_status {
        manager.call_method0("status_rows")?
    } else {
        PyDict::new(py).into_any()
    };
    let found = PyList::empty(py);
    for row in manager.getattr("rows")?.try_iter()? {
        let row = row?;
        let row_group: String = row.getattr("group")?.extract()?;
        if !group.is_empty()
            && row_group != group
            && !row_group.starts_with(&(group.to_string() + "/"))
        {
            continue;
        }
        if !origin.is_empty() && !row.getattr("kind")?.eq(origin)? {
            continue;
        }
        let categories = row.getattr("categories")?;
        let row_category = row.getattr("category")?;
        let chosen_categories = if categories.is_truthy()? {
            categories.clone()
        } else if row_category.is_truthy()? {
            PyTuple::new(py, [&row_category])?.into_any()
        } else {
            PyTuple::empty(py).into_any()
        };
        if !py
            .import("creation_lib.mod_manager.categories")?
            .call_method1("matches_filter", (chosen_categories, &category))?
            .is_truthy()?
        {
            continue;
        }
        if enabled_only && !row.getattr("enabled")?.is_truthy()? {
            continue;
        }
        if !status_module
            .call_method1("matches", (&row, &states, &filters))?
            .is_truthy()?
        {
            continue;
        }
        if !query.is_empty() {
            let joined: String = " "
                .into_pyobject(py)?
                .call_method1("join", (categories,))?
                .extract()?;
            let labels = if joined.is_empty() {
                row_category.extract::<String>()?
            } else {
                joined
            };
            let searchable = format!(
                "{} {} {labels} {row_group}",
                row.getattr("name")?.extract::<String>()?,
                row.getattr("tags")?.extract::<String>()?
            );
            if !fold(&searchable).contains(&fold(query)) {
                continue;
            }
        }
        found.append(row)?;
    }
    manager.setattr("_filtered", &found)?;
    if ![
        "priority",
        "name",
        "group",
        "version",
        "enabled",
        "category",
        "nexus",
        "notes",
        "status",
        "source",
        "conflicts",
        "updates",
        "endorsement",
        "tracking",
    ]
    .contains(&sort_by)
    {
        return Err(invalid("Unsupported visual sort column."));
    }
    let states = states.unbind();
    let sort_by = sort_by.to_string();
    let visual_key = callback(py, move |py, args, _| {
        let row = args.get_item(0)?;
        let value = if status_sort {
            py.import("creation_lib.mod_manager.status_query")?
                .call_method1("sort_key", (&row, states.bind(py), &sort_by))?
        } else {
            row.getattr(match sort_by.as_str() {
                "nexus" => "nexus_mod_id",
                "source" => "kind",
                name => name,
            })?
        };
        let value = if value.is_instance_of::<PyString>() {
            value.call_method0("casefold")?
        } else {
            value
        };
        Ok(PyTuple::new(
            py,
            [
                value,
                row.getattr("priority")?,
                row.getattr("name")?.call_method0("casefold")?,
                row.getattr("id")?,
            ],
        )?
        .into_any()
        .unbind())
    })?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("key", visual_key)?;
    kwargs.set_item("reverse", descending)?;
    found.call_method("sort", (), Some(&kwargs))?;
    manager.setattr("_filter_key", key)?;
    Ok(found.into_any().unbind())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(controller_status_rows, m)?)?;
    m.add_function(wrap_pyfunction!(controller_filtered, m)?)?;
    Ok(())
}
