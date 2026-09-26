//! Creation Kit-equivalent generators (CK-free AnimTextData).
use pyo3::prelude::*;
use pyo3::types::PyModule;

pub mod anim_text_data;
pub mod python_api;

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    python_api::register(m)
}
