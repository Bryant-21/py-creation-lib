use crate::{check_cancel, encoding, invalid, plugin_inventory::value, saves::fold};
use pyo3::{prelude::*, types::PyTuple};
use serde_json::json;
fn details(catalog: &Bound<'_, PyAny>) -> PyResult<String> {
    let mut plugins = Vec::new();
    for plugin in catalog.getattr("plugins")?.try_iter()? {
        let plugin = plugin?;
        let mut fields = Vec::new();
        for name in ["name", "path", "mod_id", "forced", "header", "masters"] {
            fields.push(value(&plugin.getattr(name)?)?);
        }
        plugins.push(fields);
    }
    Ok(encoding::digest(&json!(plugins), false, false))
}
fn locks<'py>(panes: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = panes.py();
    let sorted = py
        .import("builtins")?
        .getattr("sorted")?
        .call1((panes.getattr("locked")?.call_method0("items")?,))?;
    py.import("builtins")?.getattr("tuple")?.call1((sorted,))
}
#[pyfunction]
fn plugin_priority_begin(panes: &Bound<'_, PyAny>, name: &str) -> PyResult<Py<PyAny>> {
    let py = panes.py();
    let manager = panes.getattr("manager")?;
    let catalog = panes.getattr("catalog")?;
    if manager.getattr("busy")?.is_truthy()?
        || manager.getattr("runtime_active")?.is_truthy()?
        || !catalog.is_truthy()?
        || !manager.getattr("library")?.is_truthy()?
    {
        return Err(invalid(
            "Wait for the current worker and runtime before editing plugin priority.",
        ));
    }
    let order = panes.getattr("order")?;
    if !manager.getattr("plugin_catalog")?.is(&catalog) || !order.contains(name)? {
        return Err(invalid(
            "Plugin inventory changed. Refresh the draft first.",
        ));
    }
    let mut selected = None;
    for plugin in catalog.getattr("plugins")?.try_iter()? {
        let plugin = plugin?;
        if plugin.getattr("name")?.extract::<String>()? == name {
            selected = Some(plugin);
            break;
        }
    }
    if selected.is_none() || selected.unwrap().getattr("forced")?.is_truthy()? {
        return Err(invalid("Required game masters have fixed priorities."));
    }
    if panes.getattr("locked")?.contains(fold(name))? {
        return Err(invalid("Unlock this plugin priority before editing it."));
    }
    let builtins = py.import("builtins")?;
    Ok(py
        .import("creation_lib.mod_manager.plugin_priority")?
        .getattr("Edit")?
        .call1((
            manager.getattr("library")?,
            manager.getattr("profile_id")?,
            &catalog,
            details(&catalog)?,
            name,
            order.call_method1("index", (name,))?,
            builtins.getattr("tuple")?.call1((order,))?,
            builtins
                .getattr("frozenset")?
                .call1((panes.getattr("active")?,))?,
            locks(panes)?,
        ))?
        .unbind())
}
fn same(panes: &Bound<'_, PyAny>, plan: &Bound<'_, PyAny>) -> PyResult<()> {
    let manager = panes.getattr("manager")?;
    if manager.getattr("busy")?.is_truthy()? || manager.getattr("runtime_active")?.is_truthy()? {
        return Err(invalid(
            "Wait for the current worker and runtime before changing plugin priority.",
        ));
    }
    let builtins = panes.py().import("builtins")?;
    if !manager.getattr("library")?.is(&plan.getattr("library")?)
        || !manager
            .getattr("profile_id")?
            .eq(plan.getattr("profile_id")?)?
        || !manager
            .getattr("plugin_catalog")?
            .is(&plan.getattr("catalog")?)
        || !panes.getattr("catalog")?.is(&plan.getattr("catalog")?)
        || details(&panes.getattr("catalog")?)? != plan.getattr("details")?.extract::<String>()?
        || !builtins
            .getattr("tuple")?
            .call1((panes.getattr("order")?,))?
            .eq(plan.getattr("order")?)?
        || !builtins
            .getattr("frozenset")?
            .call1((panes.getattr("active")?,))?
            .eq(plan.getattr("active")?)?
        || !locks(panes)?.eq(plan.getattr("locked")?)?
    {
        return Err(invalid(
            "The plugin draft changed after opening this edit. Open it again.",
        ));
    }
    Ok(())
}
#[pyfunction]
fn plugin_priority_apply(
    panes: &Bound<'_, PyAny>,
    plan: &Bound<'_, PyAny>,
    input: &Bound<'_, PyAny>,
    cancel: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<()> {
    same(panes, plan)?;
    let py = panes.py();
    let priority: usize = py
        .import("creation_lib.mod_manager.row_edit")?
        .call_method1("_value", ("priority", input))?
        .extract()?;
    let count = plan.getattr("order")?.len()?;
    if priority >= count {
        return Err(invalid(format!(
            "Plugin priority must be between 0 and {}.",
            count.saturating_sub(1)
        )));
    }
    let mut locks = Vec::new();
    for item in plan.getattr("locked")?.try_iter()? {
        locks.push(item?.get_item(1)?);
    }
    let locks = PyTuple::new(py, locks)?;
    let name = plan.getattr("name")?;
    let order = py
        .import("creation_lib.mod_manager.plugin_order")?
        .call_method1(
            "move",
            (
                plan.getattr("catalog")?,
                plan.getattr("order")?,
                (&name,),
                priority,
                &locks,
            ),
        )?;
    py.import("creation_lib.mod_manager.plugin_state")?
        .call_method1(
            "review",
            (
                plan.getattr("catalog")?,
                &order,
                plan.getattr("active")?,
                locks,
            ),
        )?;
    check_cancel(Some(cancel))?;
    checkpoint.call0()?;
    check_cancel(Some(cancel))?;
    same(panes, plan)?;
    panes.setattr("order", order)?;
    panes.setattr("plan", py.None())?;
    panes.setattr("_scroll_plugin", name)?;
    Ok(())
}
#[pyfunction]
fn plugin_drag_begin(panes: &Bound<'_, PyAny>, origin: &str) -> PyResult<Py<PyAny>> {
    let py = panes.py();
    let manager = panes.getattr("manager")?;
    let catalog = panes.getattr("catalog")?;
    if manager.getattr("busy")?.is_truthy()?
        || manager.getattr("runtime_active")?.is_truthy()?
        || !catalog.is_truthy()?
    {
        return Err(invalid(
            "Wait for the current worker and runtime before dragging plugins.",
        ));
    }
    let order: Vec<String> = panes.getattr("order")?.extract()?;
    let chosen = panes.getattr("selected_plugins")?;
    let selected: Vec<String> = if chosen.contains(origin)? {
        order
            .iter()
            .filter_map(|name| match chosen.contains(name) {
                Ok(true) => Some(Ok(name.clone())),
                Ok(false) => None,
                Err(e) => Some(Err(e)),
            })
            .collect::<PyResult<_>>()?
    } else {
        order
            .iter()
            .filter(|name| name.as_str() == origin)
            .cloned()
            .collect()
    };
    let count = if chosen.contains(origin)? {
        chosen.len()?
    } else {
        1
    };
    if !order.iter().any(|name| name == origin) || selected.is_empty() || selected.len() != count {
        return Err(invalid(
            "Selected plugins changed; refresh this draft before dragging them.",
        ));
    }
    let builtins = py.import("builtins")?;
    Ok(py
        .import("creation_lib.mod_manager.plugin_drag")?
        .getattr("Drag")?
        .call1((
            manager.getattr("profile_id")?,
            catalog,
            PyTuple::new(py, order)?,
            builtins
                .getattr("frozenset")?
                .call1((panes.getattr("active")?,))?,
            locks(panes)?,
            PyTuple::new(py, selected)?,
        ))?
        .unbind())
}
#[pyfunction]
fn plugin_drag_drop(
    panes: &Bound<'_, PyAny>,
    drag: &Bound<'_, PyAny>,
    target: &str,
) -> PyResult<()> {
    let manager = panes.getattr("manager")?;
    if manager.getattr("busy")?.is_truthy()? || manager.getattr("runtime_active")?.is_truthy()? {
        return Err(invalid(
            "Wait for the current worker and runtime before dropping plugins.",
        ));
    }
    let builtins = panes.py().import("builtins")?;
    if !manager
        .getattr("profile_id")?
        .eq(drag.getattr("profile_id")?)?
        || !panes.getattr("catalog")?.is(&drag.getattr("catalog")?)
        || !builtins
            .getattr("tuple")?
            .call1((panes.getattr("order")?,))?
            .eq(drag.getattr("order")?)?
        || !builtins
            .getattr("frozenset")?
            .call1((panes.getattr("active")?,))?
            .eq(drag.getattr("active")?)?
        || !locks(panes)?.eq(drag.getattr("locked")?)?
    {
        return Err(invalid(
            "The plugin draft changed during this drag. Drag the selection again.",
        ));
    }
    let order = drag.getattr("order")?;
    let selected = drag.getattr("selected")?;
    if !order.contains(target)? {
        return Err(invalid(
            "The target plugin changed. Drag the selection again.",
        ));
    }
    if selected.contains(target)? {
        return Ok(());
    }
    let mut slot = 0;
    for name in order.try_iter()? {
        let name = name?;
        if name.eq(target)? {
            break;
        }
        if !selected.contains(&name)? {
            slot += 1;
        }
    }
    panes.call_method1("move_plugins", (selected, slot))?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(plugin_drag_begin, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_drag_drop, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_priority_begin, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_priority_apply, m)?)?;
    Ok(())
}
