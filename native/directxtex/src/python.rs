use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use std::path::Path;

use crate::{
    dds_base_rgba, remix_fo76_bundle_bytes, remix_fo76_texture_bytes, texdiag_info_bytes,
    write_dds_rgba_image_gpu,
};

#[pyfunction]
fn read_dds_rgba(py: Python<'_>, path: &str) -> PyResult<Py<PyAny>> {
    let path = path.to_owned();
    let (width, height, rgba, dxgi_format) = py
        .detach(|| dds_base_rgba(Path::new(&path)))
        .map_err(PyRuntimeError::new_err)?;
    let info = PyDict::new(py);
    info.set_item("width", width)?;
    info.set_item("height", height)?;
    info.set_item("dxgi_format", dxgi_format)?;
    info.set_item("rgba", PyBytes::new(py, &rgba))?;
    Ok(info.into_any().unbind())
}

#[pyfunction(signature = (output_path, width, height, rgba, format="BC7_UNORM", generate_mips=false, use_gpu=true))]
fn write_dds_rgba(
    py: Python<'_>,
    output_path: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
    format: &str,
    generate_mips: bool,
    use_gpu: bool,
) -> PyResult<()> {
    let output_path = output_path.to_owned();
    let rgba = rgba.to_vec();
    let format = format.to_owned();
    // GPU accelerates BC7 only; write_dds_rgba_image_gpu routes BC7 to the GPU
    // (with silent CPU fallback) and keeps every other format on the CPU path,
    // identical to the previous write_dds_bytes behaviour when use_gpu=false.
    py.detach(|| {
        write_dds_rgba_image_gpu(
            Path::new(&output_path),
            width,
            height,
            &rgba,
            &format,
            generate_mips,
            use_gpu,
        )
    })
    .map_err(|err| {
        if err.starts_with("unsupported DDS format") {
            PyValueError::new_err(err)
        } else {
            PyRuntimeError::new_err(err)
        }
    })
}

#[pyfunction]
fn texdiag_info(py: Python<'_>, path: &str) -> PyResult<Py<PyAny>> {
    let path = path.to_owned();
    let info = py
        .detach(|| texdiag_info_bytes(Path::new(&path)))
        .map_err(PyRuntimeError::new_err)?;
    let out = PyDict::new(py);
    out.set_item("width", info.width)?;
    out.set_item("height", info.height)?;
    out.set_item("depth", info.depth)?;
    out.set_item("mip_levels", info.mip_levels)?;
    out.set_item("array_size", info.array_size)?;
    out.set_item("dxgi_format", info.format_bits)?;
    out.set_item("format", info.format_name)?;
    out.set_item("dimension", info.dimension)?;
    out.set_item("alpha_mode", info.alpha_mode)?;
    out.set_item("is_cubemap", info.is_cubemap)?;
    out.set_item("is_compressed", info.is_compressed)?;
    out.set_item("bits_per_pixel", info.bits_per_pixel)?;
    out.set_item("bits_per_color", info.bits_per_color)?;
    out.set_item("image_count", info.image_count)?;
    out.set_item("file_size", info.file_size)?;
    Ok(out.into_any().unbind())
}

#[pyfunction(signature = (
    src_path,
    dst_path,
    role,
    format,
    ao_multiplier,
    specular_multiplier,
    gloss_multiplier,
    spec_offset
))]
fn remix_fo76_texture_to_fo4(
    py: Python<'_>,
    src_path: &str,
    dst_path: &str,
    role: &str,
    format: &str,
    ao_multiplier: f32,
    specular_multiplier: f32,
    gloss_multiplier: f32,
    spec_offset: f32,
) -> PyResult<()> {
    let src_path = src_path.to_owned();
    let dst_path = dst_path.to_owned();
    let role = role.to_owned();
    let format = format.to_owned();
    py.detach(|| {
        remix_fo76_texture_bytes(
            Path::new(&src_path),
            Path::new(&dst_path),
            &role,
            &format,
            ao_multiplier,
            specular_multiplier,
            gloss_multiplier,
            spec_offset,
        )
    })
    .map_err(PyRuntimeError::new_err)
}

#[pyfunction(signature = (
    diffuse_path,
    reflectivity_path,
    lighting_path,
    diffuse_out_path,
    specgloss_out_path,
    glow_out_path,
    diffuse_format,
    specgloss_format,
    glow_format,
    ao_multiplier,
    specular_multiplier,
    gloss_multiplier,
    spec_offset
))]
fn remix_fo76_bundle_to_fo4(
    py: Python<'_>,
    diffuse_path: &str,
    reflectivity_path: &str,
    lighting_path: &str,
    diffuse_out_path: &str,
    specgloss_out_path: &str,
    glow_out_path: &str,
    diffuse_format: &str,
    specgloss_format: &str,
    glow_format: &str,
    ao_multiplier: f32,
    specular_multiplier: f32,
    gloss_multiplier: f32,
    spec_offset: f32,
) -> PyResult<()> {
    let diffuse_path = diffuse_path.to_owned();
    let reflectivity_path = reflectivity_path.to_owned();
    let lighting_path = lighting_path.to_owned();
    let diffuse_out_path = diffuse_out_path.to_owned();
    let specgloss_out_path = specgloss_out_path.to_owned();
    let glow_out_path = glow_out_path.to_owned();
    let diffuse_format = diffuse_format.to_owned();
    let specgloss_format = specgloss_format.to_owned();
    let glow_format = glow_format.to_owned();
    py.detach(|| {
        remix_fo76_bundle_bytes(
            Path::new(&diffuse_path),
            Path::new(&reflectivity_path),
            Path::new(&lighting_path),
            Path::new(&diffuse_out_path),
            Path::new(&specgloss_out_path),
            Path::new(&glow_out_path),
            &diffuse_format,
            &specgloss_format,
            &glow_format,
            ao_multiplier,
            specular_multiplier,
            gloss_multiplier,
            spec_offset,
        )
    })
    .map_err(PyRuntimeError::new_err)
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(read_dds_rgba, m)?)?;
    m.add_function(wrap_pyfunction!(write_dds_rgba, m)?)?;
    m.add_function(wrap_pyfunction!(texdiag_info, m)?)?;
    m.add_function(wrap_pyfunction!(remix_fo76_texture_to_fo4, m)?)?;
    m.add_function(wrap_pyfunction!(remix_fo76_bundle_to_fo4, m)?)?;
    Ok(())
}

#[pymodule]
fn directxtex_native(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_module(m)
}
