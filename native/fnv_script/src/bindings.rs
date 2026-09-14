use crate::context::{FnvScriptContext, TargetMetadata};
use crate::decompile::decompile_bytecode;
use crate::emit::emit_psc;
use crate::function_map::FunctionMap;
use crate::lower::lower;
use crate::parser::parse_script;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::collections::HashMap;

#[pyclass(module = "creation_lib._native.fnv_script_native")]
pub struct PyContext {
    inner: FnvScriptContext,
}

#[pymethods]
impl PyContext {
    #[new]
    fn new(
        function_map_yaml: &str,
        actor_value_map: HashMap<String, String>,
        mod_prefix: String,
        script_class_name: String,
        papyrus_extends: String,
        strict: bool,
    ) -> PyResult<Self> {
        let function_map = FunctionMap::from_yaml(function_map_yaml)
            .map_err(|err| PyValueError::new_err(err.to_string()))?;
        let target = TargetMetadata::for_extends(&papyrus_extends);
        Ok(Self {
            inner: FnvScriptContext {
                function_map,
                actor_value_map,
                mod_prefix,
                strict,
                script_class_name,
                papyrus_extends,
                target,
            },
        })
    }
}

#[pyfunction]
fn translate(py: Python<'_>, source: &str, ctx: PyRef<'_, PyContext>) -> PyResult<String> {
    let owned_source = source.to_owned();
    let owned_ctx = ctx.inner.clone();
    py.detach(move || {
        let ast =
            parse_script(&owned_source).map_err(|err| PyValueError::new_err(err.to_string()))?;
        let ir = lower(&ast, &owned_ctx).map_err(|err| PyValueError::new_err(err.to_string()))?;
        Ok(emit_psc(&ir))
    })
}

#[pyfunction]
fn decompile(py: Python<'_>, bytes: Vec<u8>) -> PyResult<String> {
    py.detach(move || {
        decompile_bytecode(&bytes).map_err(|err| PyValueError::new_err(err.to_string()))
    })
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyContext>()?;
    m.add_function(wrap_pyfunction!(translate, m)?)?;
    m.add_function(wrap_pyfunction!(decompile, m)?)?;
    Ok(())
}
