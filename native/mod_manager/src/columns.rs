use crate::invalid;
use pyo3::{
    prelude::*,
    types::{PyDict, PyFloat, PyInt, PyList, PyString, PyTuple},
};
const ORDER: [&str; 8] = [
    "enabled", "priority", "name", "status", "version", "category", "nexus", "notes",
];
const LABELS: [&str; 8] = [
    "On", "Priority", "Mod name", "Status", "Version", "Category", "ID", "Notes",
];
const DEFAULT: [f64; 8] = [38., 76., 280., 78., 96., 110., 62., 130.];
const MIN: [f64; 8] = [28., 54., 120., 28., 48., 48., 42., 48.];
const MAX: [f64; 8] = [100., 220., 1200., 360., 600., 800., 220., 1200.];
fn width(value: &Bound<'_, PyAny>) -> Option<f64> {
    if !value.is_exact_instance_of::<PyInt>() && !value.is_exact_instance_of::<PyFloat>() {
        return None;
    }
    value.extract::<f64>().ok().filter(|v| v.is_finite())
}
fn normalize(value: &Bound<'_, PyAny>) -> PyResult<(Vec<usize>, [Option<f64>; 8])> {
    let mut order = vec![];
    let mut widths = [None; 8];
    if let Ok(value) = value.cast::<PyDict>() {
        if let Some(items) = value.get_item("order")? {
            if items.is_instance_of::<PyList>() || items.is_instance_of::<PyTuple>() {
                for key in items.try_iter()? {
                    let key = key?;
                    if key.is_instance_of::<PyString>() {
                        let key = key.extract::<String>()?;
                        if let Some(index) = ORDER.iter().position(|&v| v == key) {
                            if !order.contains(&index) {
                                order.push(index);
                            }
                        }
                    }
                }
            }
        }
        if let Some(values) = value.get_item("widths")? {
            if let Ok(values) = values.cast::<PyDict>() {
                for (key, value) in values.iter() {
                    if let Ok(key) = key.extract::<String>() {
                        if let Some(index) = ORDER.iter().position(|&v| v == key) {
                            if let Some(value) = width(&value) {
                                widths[index] = Some(value.clamp(MIN[index], MAX[index]));
                            }
                        }
                    }
                }
            }
        }
    }
    for index in 0..8 {
        if !order.contains(&index) {
            order.push(index);
        }
    }
    Ok((order, widths))
}
fn config(py: Python<'_>, order: &[usize], widths: &[Option<f64>; 8]) -> PyResult<Py<PyAny>> {
    let result = PyDict::new(py);
    let supplied = PyDict::new(py);
    for (i, value) in widths.iter().enumerate() {
        if let Some(value) = value {
            supplied.set_item(ORDER[i], value)?;
        }
    }
    result.set_item("order", PyList::new(py, order.iter().map(|&i| ORDER[i]))?)?;
    result.set_item("widths", supplied)?;
    Ok(result.into_any().unbind())
}
#[pyfunction]
fn columns_normalize(value: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let (order, widths) = normalize(value)?;
    config(value.py(), &order, &widths)
}
#[pyfunction]
fn columns_resized(
    value: &Bound<'_, PyAny>,
    key: &str,
    size: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let (Some(index), Some(size)) = (ORDER.iter().position(|&v| v == key), width(size)) else {
        return Err(invalid("Choose a finite width for a known mod column."));
    };
    let (order, mut widths) = normalize(value)?;
    widths[index] = Some(size.clamp(MIN[index], MAX[index]));
    config(value.py(), &order, &widths)
}
#[pyfunction]
fn columns_moved(value: &Bound<'_, PyAny>, key: &str, before: &str) -> PyResult<Py<PyAny>> {
    let (Some(index), Some(before)) = (
        ORDER.iter().position(|&v| v == key),
        ORDER.iter().position(|&v| v == before),
    ) else {
        return Err(invalid("Column order contains an unknown column."));
    };
    let (mut order, widths) = normalize(value)?;
    if index != before {
        order.retain(|&v| v != index);
        let position = order.iter().position(|&v| v == before).unwrap();
        order.insert(position, index);
    }
    config(value.py(), &order, &widths)
}
#[pyfunction]
fn columns_layout(
    available: f64,
    visible: &Bound<'_, PyAny>,
    value: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let py = value.py();
    let (order, supplied) = normalize(value)?;
    let mut visible_keys = [false; 8];
    for key in visible.try_iter()? {
        if let Ok(key) = key?.extract::<String>() {
            if let Some(i) = ORDER.iter().position(|&v| v == key) {
                visible_keys[i] = true;
            }
        }
    }
    let mut chosen = [true, true, true, false, false, false, false, false];
    let mut widths = DEFAULT;
    for i in 0..8 {
        if let Some(width) = supplied[i] {
            widths[i] = width;
        }
    }
    if supplied.iter().any(Option::is_some) || order != (0..8).collect::<Vec<_>>() {
        for i in 0..8 {
            chosen[i] |= visible_keys[i];
        }
        let other: f64 = (0..8)
            .filter(|&i| i != 2 && chosen[i])
            .map(|i| widths[i])
            .sum();
        if supplied[2].is_none() {
            widths[2] = (available - other).max(160.);
        }
    } else {
        let mut budget = (available - 274.).max(0.);
        for i in 3..8 {
            if visible_keys[i] {
                let size = if i == 3 {
                    widths[i].min(budget.max(28.))
                } else {
                    widths[i]
                };
                if i == 3 || size <= budget {
                    chosen[i] = true;
                    widths[i] = size;
                    budget -= size;
                }
            }
        }
        widths[2] = (available
            - (0..8)
                .filter(|&i| i != 2 && chosen[i])
                .map(|i| widths[i])
                .sum::<f64>())
        .max(120.);
    }
    let constructor = py
        .import("creation_lib.mod_manager.columns")?
        .getattr("Column")?;
    let mut columns = vec![];
    let mut offset = 0.;
    for i in order {
        if chosen[i] {
            columns.push(constructor.call1((ORDER[i], LABELS[i], offset, widths[i]))?);
            offset += widths[i];
        }
    }
    Ok(PyTuple::new(py, columns)?.into_any().unbind())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(columns_normalize, m)?)?;
    m.add_function(wrap_pyfunction!(columns_resized, m)?)?;
    m.add_function(wrap_pyfunction!(columns_moved, m)?)?;
    m.add_function(wrap_pyfunction!(columns_layout, m)?)?;
    Ok(())
}
