use numpy::{PyArray1, PyArray2, PyArrayMethods, PyReadonlyArray2, PyUntypedArrayMethods};
use pyo3::prelude::*;

pub mod clustering;

#[pyfunction]
#[pyo3(name = "cluster_rgb", signature = (fit_pixels, predict_pixels, n_clusters, seed, n_init))]
fn cluster_rgb_py<'py>(
    py: Python<'py>,
    fit_pixels: PyReadonlyArray2<'py, f32>,
    predict_pixels: PyReadonlyArray2<'py, f32>,
    n_clusters: u32,
    seed: u64,
    n_init: u32,
) -> PyResult<(Bound<'py, PyArray2<f32>>, Bound<'py, PyArray1<i32>>)> {
    let fit_shape = (fit_pixels.shape()[0], fit_pixels.shape()[1]);
    let pred_shape = (predict_pixels.shape()[0], predict_pixels.shape()[1]);
    let fit_vec: Vec<f32> = fit_pixels.as_array().iter().copied().collect();
    let pred_vec: Vec<f32> = predict_pixels.as_array().iter().copied().collect();

    let (centers, labels) = py.detach(move || {
        let fit = ndarray::Array2::from_shape_vec(fit_shape, fit_vec).unwrap();
        let pred = ndarray::Array2::from_shape_vec(pred_shape, pred_vec).unwrap();
        clustering::cluster_rgb(fit.view(), pred.view(), n_clusters as usize, seed, n_init)
    });

    let centers_rows = centers.nrows();
    let centers_cols = centers.ncols();
    let (centers_vec, _) = centers.into_raw_vec_and_offset();
    let centers_flat = PyArray1::from_vec(py, centers_vec);
    let centers_py = centers_flat
        .reshape([centers_rows, centers_cols])
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("reshape centers: {e}")))?;

    let (labels_vec, _) = labels.into_raw_vec_and_offset();
    let labels_py = PyArray1::from_vec(py, labels_vec);

    Ok((centers_py, labels_py))
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(cluster_rgb_py, m)?)?;
    Ok(())
}

#[pymodule]
fn palette_native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_module(m)
}
