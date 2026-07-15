use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use serde::de::DeserializeOwned;

pub mod base;
pub mod bgem;
pub mod bgsm;
pub mod bsrefl;
pub mod cdb;
mod ce2;
pub mod convert;
pub mod error;
mod pbr;
mod string_table;
pub mod texture_convert;

fn json_to_py(py: Python<'_>, value: serde_json::Value) -> PyResult<Py<PyAny>> {
    match value {
        serde_json::Value::Null => Ok(py.None()),
        serde_json::Value::Bool(value) => {
            Ok(value.into_pyobject(py)?.to_owned().into_any().unbind())
        }
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Ok(value.into_pyobject(py)?.into_any().unbind())
            } else if let Some(value) = value.as_u64() {
                Ok(value.into_pyobject(py)?.into_any().unbind())
            } else if let Some(value) = value.as_f64() {
                Ok(value.into_pyobject(py)?.into_any().unbind())
            } else {
                Ok(py.None())
            }
        }
        serde_json::Value::String(value) => {
            Ok(value.as_str().into_pyobject(py)?.into_any().unbind())
        }
        serde_json::Value::Array(values) => {
            let list = PyList::empty(py);
            for value in values {
                list.append(json_to_py(py, value)?)?;
            }
            Ok(list.into_any().unbind())
        }
        serde_json::Value::Object(values) => {
            let dict = PyDict::new(py);
            for (key, value) in values {
                dict.set_item(key, json_to_py(py, value)?)?;
            }
            Ok(dict.into_any().unbind())
        }
    }
}

fn to_py_payload<T: serde::Serialize>(py: Python<'_>, value: &T) -> PyResult<Py<PyAny>> {
    let json = serde_json::to_value(value).map_err(error::MaterialError::from)?;
    json_to_py(py, json)
}

fn from_py_payload<T: DeserializeOwned>(py: Python<'_>, payload: &Bound<'_, PyAny>) -> PyResult<T> {
    let json = py.import("json")?;
    let raw = json
        .call_method1("dumps", (payload,))?
        .extract::<String>()?;
    serde_json::from_str(&raw)
        .map_err(error::MaterialError::from)
        .map_err(PyErr::from)
}

#[pyfunction]
fn bethesda_crc32(data: &[u8]) -> PyResult<u32> {
    Ok(cdb::bethesda_crc32(data))
}

#[pyfunction]
fn resource_id_from_path(py: Python<'_>, path: &str) -> PyResult<Py<PyAny>> {
    let rid = cdb::resource_id_from_path(path);
    let dict = PyDict::new(py);
    dict.set_item("dir", rid.dir)?;
    dict.set_item("file", rid.file)?;
    dict.set_item("ext", rid.ext)?;
    Ok(dict.into_any().unbind())
}

#[pyfunction]
fn parse_cdb(py: Python<'_>, data: &[u8]) -> PyResult<Py<PyAny>> {
    let parsed = cdb::parse_cdb(data).map_err(PyErr::from)?;
    to_py_payload(py, &parsed)
}

#[pyfunction]
fn project_ce2_material(
    py: Python<'_>,
    payload: &Bound<'_, PyAny>,
    db_id: u32,
) -> PyResult<Py<PyAny>> {
    let parsed: cdb::CdbPayload = from_py_payload(py, payload)?;
    match ce2::project(&parsed, db_id) {
        Some(projected) => to_py_payload(py, &projected),
        None => Ok(py.None()),
    }
}

#[pyfunction]
fn walk_component(
    py: Python<'_>,
    blob_payload: &Bound<'_, PyAny>,
    class_defs_payload: &Bound<'_, PyAny>,
    _objects_payload: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let blob: cdb::ComponentBlobPayload = from_py_payload(py, blob_payload)?;
    let class_defs: Vec<cdb::ClassDefPayload> = from_py_payload(py, class_defs_payload)?;
    match ce2::walk_component(&blob, &class_defs) {
        Some(value) => json_to_py(py, value),
        None => Ok(py.None()),
    }
}

#[pyfunction]
fn parse_bgsm(py: Python<'_>, data: &[u8]) -> PyResult<Py<PyAny>> {
    let parsed = bgsm::parse(data).map_err(PyErr::from)?;
    to_py_payload(py, &parsed)
}

#[pyfunction]
fn write_bgsm(py: Python<'_>, payload: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    let parsed: bgsm::BgsmData = from_py_payload(py, payload)?;
    Ok(bgsm::write(&parsed))
}

#[pyfunction]
fn parse_bgem(py: Python<'_>, data: &[u8]) -> PyResult<Py<PyAny>> {
    let parsed = bgem::parse(data).map_err(PyErr::from)?;
    to_py_payload(py, &parsed)
}

#[pyfunction]
fn write_bgem(py: Python<'_>, payload: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    let parsed: bgem::BgemData = from_py_payload(py, payload)?;
    Ok(bgem::write(&parsed))
}

#[pyfunction]
fn find_master_string(value: &str) -> PyResult<i32> {
    Ok(bsrefl::find_master_string(value))
}

#[pyfunction]
fn inspect_bsrefl(py: Python<'_>, data: &[u8]) -> PyResult<Py<PyAny>> {
    let summary = bsrefl::inspect(data).map_err(PyErr::from)?;
    to_py_payload(py, &summary)
}

#[pyfunction]
fn pbr_to_specgloss_f32(
    py: Python<'_>,
    albedo: &[u8],
    metallic: &[u8],
    roughness: &[u8],
    ao: Option<&[u8]>,
    pixel_count: usize,
    ao_multiplier: f32,
    specular_multiplier: f32,
    gloss_multiplier: f32,
    spec_offset: f32,
) -> PyResult<Py<PyAny>> {
    let params = pbr::PbrToSpecGlossParams {
        ao_multiplier,
        specular_multiplier,
        gloss_multiplier,
        spec_offset,
    };
    let converted = pbr::convert_buffers(albedo, metallic, roughness, ao, pixel_count, params)
        .map_err(PyErr::from)?;

    let dict = PyDict::new(py);
    dict.set_item(
        "diffuse",
        PyBytes::new(py, &pbr::f32_vec_to_bytes(&converted.diffuse)),
    )?;
    dict.set_item(
        "specular",
        PyBytes::new(py, &pbr::f32_vec_to_bytes(&converted.specular)),
    )?;
    dict.set_item(
        "gloss",
        PyBytes::new(py, &pbr::f32_vec_to_bytes(&converted.gloss)),
    )?;
    Ok(dict.into_any().unbind())
}

#[pyfunction(signature = (
    diffuse,
    reflectivity,
    lighting,
    width,
    height,
    reflectivity_width,
    reflectivity_height,
    lighting_width,
    lighting_height,
    ao_multiplier,
    specular_multiplier,
    gloss_multiplier,
    spec_offset,
    emit_lighting_alpha_glow=false
))]
fn fo76_bundle_to_fo4_f32(
    py: Python<'_>,
    diffuse: &[u8],
    reflectivity: &[u8],
    lighting: &[u8],
    width: usize,
    height: usize,
    reflectivity_width: usize,
    reflectivity_height: usize,
    lighting_width: usize,
    lighting_height: usize,
    ao_multiplier: f32,
    specular_multiplier: f32,
    gloss_multiplier: f32,
    spec_offset: f32,
    emit_lighting_alpha_glow: bool,
) -> PyResult<Py<PyAny>> {
    let params = texture_convert::TextureConversionParams {
        ao_multiplier,
        specular_multiplier,
        gloss_multiplier,
        spec_offset,
    };
    let converted = texture_convert::fo76_bundle_to_fo4_buffers(
        diffuse,
        reflectivity,
        lighting,
        width,
        height,
        reflectivity_width,
        reflectivity_height,
        lighting_width,
        lighting_height,
        params,
        emit_lighting_alpha_glow,
    )
    .map_err(PyErr::from)?;

    let dict = PyDict::new(py);
    dict.set_item(
        "diffuse",
        PyBytes::new(py, &texture_convert::f32_vec_to_bytes(&converted.diffuse)),
    )?;
    dict.set_item(
        "specgloss",
        PyBytes::new(py, &texture_convert::f32_vec_to_bytes(&converted.specgloss)),
    )?;
    if let Some(glow) = converted.glow {
        dict.set_item(
            "glow",
            PyBytes::new(py, &texture_convert::f32_vec_to_bytes(&glow)),
        )?;
    }
    Ok(dict.into_any().unbind())
}

#[pyfunction]
fn fo76_normal_to_fo4_f32(normal: &[u8], width: usize, height: usize) -> PyResult<Vec<u8>> {
    let converted =
        texture_convert::fo76_normal_to_fo4_buffer(normal, width, height).map_err(PyErr::from)?;
    Ok(texture_convert::f32_vec_to_bytes(&converted))
}

#[pyfunction]
fn passthrough_rgba_f32(rgba: &[u8], width: usize, height: usize) -> PyResult<Vec<u8>> {
    let converted =
        texture_convert::passthrough_rgba_buffer(rgba, width, height).map_err(PyErr::from)?;
    Ok(texture_convert::f32_vec_to_bytes(&converted))
}

#[pyfunction]
fn convert_texture_set_paths(py: Python<'_>, payload: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let request: texture_convert::TextureSetPathRequest = from_py_payload(py, payload)?;
    let result = py
        .detach(|| texture_convert::convert_texture_set_paths(request))
        .map_err(PyErr::from)?;
    to_py_payload(py, &result)
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(bethesda_crc32, m)?)?;
    m.add_function(wrap_pyfunction!(resource_id_from_path, m)?)?;
    m.add_function(wrap_pyfunction!(parse_cdb, m)?)?;
    m.add_function(wrap_pyfunction!(project_ce2_material, m)?)?;
    m.add_function(wrap_pyfunction!(walk_component, m)?)?;
    m.add_function(wrap_pyfunction!(parse_bgsm, m)?)?;
    m.add_function(wrap_pyfunction!(write_bgsm, m)?)?;
    m.add_function(wrap_pyfunction!(parse_bgem, m)?)?;
    m.add_function(wrap_pyfunction!(write_bgem, m)?)?;
    m.add_function(wrap_pyfunction!(find_master_string, m)?)?;
    m.add_function(wrap_pyfunction!(inspect_bsrefl, m)?)?;
    m.add_function(wrap_pyfunction!(pbr_to_specgloss_f32, m)?)?;
    m.add_function(wrap_pyfunction!(fo76_bundle_to_fo4_f32, m)?)?;
    m.add_function(wrap_pyfunction!(fo76_normal_to_fo4_f32, m)?)?;
    m.add_function(wrap_pyfunction!(passthrough_rgba_f32, m)?)?;
    m.add_function(wrap_pyfunction!(convert_texture_set_paths, m)?)?;
    Ok(())
}

#[pymodule]
fn materials_native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_module(m)
}
