use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

pub mod authoring_emit;
pub mod btd;
pub mod btd4;
mod diagnostics;
pub mod fo4_frame;
pub mod global_blend;
pub mod height_resample;
pub mod heightmap_dds;
pub mod land_encode;
pub mod texture_bridge;
pub mod texture_layers;
mod water_scan;

fn terrain_error(error: impl std::error::Error) -> PyErr {
    PyRuntimeError::new_err(error.to_string())
}

#[pyfunction]
fn read_btd_header(py: Python<'_>, path: &str) -> PyResult<String> {
    let path = path.to_owned();
    py.detach(move || {
        let btd = btd::BtdFile::open_header(&path).map_err(terrain_error)?;
        serde_json::to_string(&btd.to_report()).map_err(terrain_error)
    })
}

#[pyfunction(signature = (path, cell_x, cell_y, lod=0))]
fn probe_btd_cell(
    py: Python<'_>,
    path: &str,
    cell_x: i32,
    cell_y: i32,
    lod: u8,
) -> PyResult<String> {
    let path = path.to_owned();
    py.detach(move || {
        let mut btd = btd::BtdFile::open(&path).map_err(terrain_error)?;
        let heights = btd
            .cell_height_map_u16(cell_x, cell_y, lod)
            .map_err(terrain_error)?;
        serde_json::to_string(&heights).map_err(terrain_error)
    })
}

#[pyfunction]
fn convert_btd_to_fo4_land(py: Python<'_>, options_json: &str) -> PyResult<String> {
    let options_json = options_json.to_owned();
    py.detach(move || {
        let options: authoring_emit::ConvertOptions =
            serde_json::from_str(&options_json).map_err(terrain_error)?;
        authoring_emit::convert_btd_to_authoring(options).map_err(terrain_error)
    })
}

#[pyfunction]
fn write_water_manifest(py: Python<'_>, options_json: &str) -> PyResult<String> {
    let options_json = options_json.to_owned();
    py.detach(move || {
        let options: water_scan::WaterScanOptions =
            serde_json::from_str(&options_json).map_err(terrain_error)?;
        water_scan::write_water_manifest(options).map_err(terrain_error)
    })
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(read_btd_header, m)?)?;
    m.add_function(wrap_pyfunction!(probe_btd_cell, m)?)?;
    m.add_function(wrap_pyfunction!(convert_btd_to_fo4_land, m)?)?;
    m.add_function(wrap_pyfunction!(write_water_manifest, m)?)?;
    Ok(())
}

#[pymodule]
fn terrain_native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_module(m)
}
