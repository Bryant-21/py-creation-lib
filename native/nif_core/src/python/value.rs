use indexmap::IndexMap;
use pyo3::IntoPyObjectExt as _;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDict, PyList, PyTuple};

use crate::model::NifValue;

/// Convert a `NifValue` to a Python object using the PyO3 0.28 API.
pub fn nif_to_py(py: Python<'_>, v: &NifValue) -> Py<PyAny> {
    match v {
        NifValue::Null => py.None(),
        NifValue::Bool(b) => (*b).into_py_any(py).unwrap(),
        NifValue::Int(i) => (*i).into_py_any(py).unwrap(),
        NifValue::UInt(u) => (*u as i64).into_py_any(py).unwrap(),
        NifValue::Float(f) => (*f).into_py_any(py).unwrap(),
        NifValue::FloatNan(u) => (*u as i64).into_py_any(py).unwrap(),
        NifValue::String(s) | NifValue::Char(s) => s.as_str().into_py_any(py).unwrap(),
        NifValue::Ref(r) => (*r).into_py_any(py).unwrap(),
        NifValue::Bytes(b) => PyBytes::new(py, b).into_any().unbind(),

        // Vec/Color/Matrix/Quaternion variants are defined for future direct
        // encoding but are not currently emitted by the reader (which stores
        // all compound types as NifValue::Struct). Convert to Python lists.
        NifValue::Vec3(arr) => {
            let v: Vec<f64> = arr.iter().map(|x| *x as f64).collect();
            v.into_py_any(py).unwrap()
        }
        NifValue::Vec4(arr) | NifValue::Color4(arr) | NifValue::Quaternion(arr) => {
            let v: Vec<f64> = arr.iter().map(|x| *x as f64).collect();
            v.into_py_any(py).unwrap()
        }
        NifValue::Color3(arr) => {
            let v: Vec<f64> = arr.iter().map(|x| *x as f64).collect();
            v.into_py_any(py).unwrap()
        }
        NifValue::Matrix33(m) => {
            let rows: Vec<Vec<f64>> = m
                .iter()
                .map(|r| r.iter().map(|x| *x as f64).collect())
                .collect();
            rows.into_py_any(py).unwrap()
        }
        NifValue::Matrix44(m) => {
            let rows: Vec<Vec<f64>> = m
                .iter()
                .map(|r| r.iter().map(|x| *x as f64).collect())
                .collect();
            rows.into_py_any(py).unwrap()
        }
        NifValue::Array(arr) => {
            let items: Vec<Py<PyAny>> = arr.iter().map(|item| nif_to_py(py, item)).collect();
            items.into_py_any(py).unwrap()
        }
        NifValue::Struct(map) => {
            let d = PyDict::new(py);
            for (k, val) in map.iter() {
                d.set_item(k, nif_to_py(py, val)).expect("dict set_item");
            }
            d.into_any().unbind()
        }
    }
}

/// Convert a Python object to a `NifValue`.
///
/// Inference order: None → bool (before int) → int → float → str →
/// bytes → list → tuple → dict → Null.
#[allow(deprecated)]
pub fn py_to_nif(val: &Bound<'_, PyAny>) -> NifValue {
    if val.is_none() {
        return NifValue::Null;
    }
    if val.is_instance_of::<PyBool>() {
        let b: bool = val.extract().unwrap_or(false);
        return NifValue::Bool(b);
    }
    if let Ok(i) = val.extract::<i64>() {
        return NifValue::Int(i);
    }
    if let Ok(f) = val.extract::<f64>() {
        return NifValue::Float(f);
    }
    if let Ok(s) = val.extract::<String>() {
        return NifValue::String(s);
    }
    if val.is_instance_of::<PyBytes>() {
        let b: Vec<u8> = val.extract().unwrap_or_default();
        return NifValue::Bytes(b);
    }
    if let Ok(lst) = val.downcast::<PyList>() {
        let items: Vec<NifValue> = lst.iter().map(|item| py_to_nif(&item)).collect();
        return NifValue::Array(items);
    }
    if let Ok(tup) = val.downcast::<PyTuple>() {
        let items: Vec<NifValue> = tup.iter().map(|item| py_to_nif(&item)).collect();
        return NifValue::Array(items);
    }
    if let Ok(d) = val.downcast::<PyDict>() {
        let mut map: IndexMap<String, NifValue> = IndexMap::new();
        for (k, v) in d.iter() {
            let key: String = k.extract().unwrap_or_default();
            map.insert(key, py_to_nif(&v));
        }
        return NifValue::Struct(map);
    }
    NifValue::Null
}
