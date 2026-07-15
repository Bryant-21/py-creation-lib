use numpy::{
    PyArray1, PyArray2, PyArray3, PyArrayMethods, PyReadonlyArray1, PyReadonlyArray2,
    PyUntypedArrayMethods,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use scirs2_core::ndarray::{Array1, Array2, Ix3};
use scirs2_interpolate::interp1d::pchip::PchipInterpolator;
use scirs2_interpolate::interp1d::{ExtrapolateMode, Interp1d, InterpolationMethod};
use scirs2_ndimage::filters::{BorderMode, gaussian_filter, median_filter};
use scirs2_ndimage::morphology::{Connectivity, distance_transform_edt, find_objects_2d, label_2d};
use scirs2_signal::filter::{FilterType, butter, lfilter};
use scirs2_spatial::KDTree;
use scirs2_spatial::convex_hull::ConvexHull;

fn value_error(err: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(err.to_string())
}

fn array2_f64(input: PyReadonlyArray2<'_, f64>) -> PyResult<Array2<f64>> {
    let rows = input.shape()[0];
    let cols = input.shape()[1];
    let values: Vec<f64> = input.as_array().iter().copied().collect();
    Array2::from_shape_vec((rows, cols), values).map_err(value_error)
}

fn array2_bool(input: PyReadonlyArray2<'_, bool>) -> PyResult<Array2<bool>> {
    let rows = input.shape()[0];
    let cols = input.shape()[1];
    let values: Vec<bool> = input.as_array().iter().copied().collect();
    Array2::from_shape_vec((rows, cols), values).map_err(value_error)
}

fn array2_usize(input: PyReadonlyArray2<'_, usize>) -> PyResult<Array2<usize>> {
    let rows = input.shape()[0];
    let cols = input.shape()[1];
    let values: Vec<usize> = input.as_array().iter().copied().collect();
    Array2::from_shape_vec((rows, cols), values).map_err(value_error)
}

fn array1_f64(input: PyReadonlyArray1<'_, f64>) -> Array1<f64> {
    Array1::from_iter(input.as_array().iter().copied())
}

#[pyfunction]
fn kdtree_query<'py>(
    py: Python<'py>,
    points: PyReadonlyArray2<'_, f64>,
    queries: PyReadonlyArray2<'_, f64>,
    k: usize,
) -> PyResult<(Bound<'py, PyArray2<f64>>, Bound<'py, PyArray2<usize>>)> {
    let points = array2_f64(points)?;
    let queries = array2_f64(queries)?;

    if points.nrows() == 0 {
        return Err(PyValueError::new_err("KDTree input has no points"));
    }
    if k == 0 {
        return Err(PyValueError::new_err("k must be at least 1"));
    }
    if points.ncols() != queries.ncols() {
        return Err(PyValueError::new_err(format!(
            "query dimension {} does not match point dimension {}",
            queries.ncols(),
            points.ncols()
        )));
    }

    let nearest_count = k.min(points.nrows());
    let tree = KDTree::new(&points).map_err(value_error)?;
    let mut distances = Array2::<f64>::zeros((queries.nrows(), nearest_count));
    let mut indices = Array2::<usize>::zeros((queries.nrows(), nearest_count));

    for (row_index, query) in queries.outer_iter().enumerate() {
        let query_vec: Vec<f64> = query.iter().copied().collect();
        let (query_indices, query_distances) =
            tree.query(&query_vec, nearest_count).map_err(value_error)?;
        for neighbor_index in 0..nearest_count {
            indices[[row_index, neighbor_index]] = query_indices[neighbor_index];
            distances[[row_index, neighbor_index]] = query_distances[neighbor_index];
        }
    }

    let (distances_vec, _) = distances.into_raw_vec_and_offset();
    let distances_py = PyArray1::from_vec(py, distances_vec)
        .reshape([queries.nrows(), nearest_count])
        .map_err(value_error)?;
    let (indices_vec, _) = indices.into_raw_vec_and_offset();
    let indices_py = PyArray1::from_vec(py, indices_vec)
        .reshape([queries.nrows(), nearest_count])
        .map_err(value_error)?;

    Ok((distances_py, indices_py))
}

#[pyfunction]
fn convex_hull_triangles(vertices: Vec<[f64; 3]>) -> PyResult<Vec<[usize; 3]>> {
    if vertices.len() < 4 {
        return Ok(Vec::new());
    }

    let flat: Vec<f64> = vertices
        .iter()
        .flat_map(|vertex| vertex.iter().copied())
        .collect();
    let points = Array2::from_shape_vec((vertices.len(), 3), flat).map_err(value_error)?;
    let hull = ConvexHull::new(&points.view()).map_err(value_error)?;
    let mut triangles = Vec::new();

    for simplex in hull.simplices() {
        if simplex.len() == 3 {
            triangles.push([simplex[0], simplex[1], simplex[2]]);
        } else if simplex.len() > 3 {
            for index in 1..(simplex.len() - 1) {
                triangles.push([simplex[0], simplex[index], simplex[index + 1]]);
            }
        }
    }

    Ok(triangles)
}

#[pyfunction]
fn butter_lfilter<'py>(
    py: Python<'py>,
    data: PyReadonlyArray1<'_, f64>,
    normal_cutoff: f64,
    filter_type: &str,
    order: usize,
) -> PyResult<Bound<'py, PyArray1<f64>>> {
    let filter_type = match filter_type {
        "high" | "highpass" => FilterType::Highpass,
        "low" | "lowpass" => FilterType::Lowpass,
        other => {
            return Err(PyValueError::new_err(format!(
                "unsupported filter type: {other}"
            )));
        }
    };
    let input: Vec<f64> = data.as_array().iter().copied().collect();
    let (b, a) = butter(order, normal_cutoff, filter_type).map_err(value_error)?;
    let output = lfilter(&b, &a, &input).map_err(value_error)?;
    Ok(PyArray1::from_vec(py, output))
}

#[pyfunction]
fn distance_transform_indices_2d<'py>(
    py: Python<'py>,
    input: PyReadonlyArray2<'_, bool>,
) -> PyResult<Bound<'py, PyArray3<i32>>> {
    let input = array2_bool(input)?.into_dyn();
    let (_, indices) = distance_transform_edt(&input, None, false, true).map_err(value_error)?;
    let indices = indices.ok_or_else(|| PyValueError::new_err("missing EDT indices"))?;
    let indices = indices.into_dimensionality::<Ix3>().map_err(value_error)?;
    let shape = indices.dim();
    let (indices_vec, _) = indices.into_raw_vec_and_offset();
    PyArray1::from_vec(py, indices_vec)
        .reshape([shape.0, shape.1, shape.2])
        .map_err(value_error)
}

#[pyfunction]
fn label_2d_native<'py>(
    py: Python<'py>,
    input: PyReadonlyArray2<'_, bool>,
) -> PyResult<(Bound<'py, PyArray2<usize>>, usize)> {
    let input = array2_bool(input)?;
    let (labels, count) = label_2d(&input, Some(Connectivity::Face)).map_err(value_error)?;
    let shape = labels.dim();
    let (labels_vec, _) = labels.into_raw_vec_and_offset();
    let labels_py = PyArray1::from_vec(py, labels_vec)
        .reshape([shape.0, shape.1])
        .map_err(value_error)?;
    Ok((labels_py, count))
}

#[pyfunction]
fn find_objects_2d_native(
    labels: PyReadonlyArray2<'_, usize>,
) -> PyResult<Vec<(usize, usize, usize, usize, usize)>> {
    let labels = array2_usize(labels)?;
    let objects = find_objects_2d(&labels).map_err(value_error)?;
    Ok(objects
        .into_iter()
        .map(|bbox| {
            (
                bbox.label,
                bbox.min_row,
                bbox.max_row,
                bbox.min_col,
                bbox.max_col,
            )
        })
        .collect())
}

#[pyfunction]
fn gaussian_filter1d<'py>(
    py: Python<'py>,
    data: PyReadonlyArray1<'_, f64>,
    sigma: f64,
) -> PyResult<Bound<'py, PyArray1<f64>>> {
    let input = array1_f64(data);
    let output =
        gaussian_filter(&input, sigma, Some(BorderMode::Nearest), None).map_err(value_error)?;
    Ok(PyArray1::from_vec(py, output.iter().copied().collect()))
}

#[pyfunction]
fn median_filter1d<'py>(
    py: Python<'py>,
    data: PyReadonlyArray1<'_, f64>,
    size: usize,
) -> PyResult<Bound<'py, PyArray1<f64>>> {
    if size == 0 {
        return Err(PyValueError::new_err("median filter size must be positive"));
    }
    let input = array1_f64(data);
    let output = median_filter(&input, &[size], Some(BorderMode::Nearest)).map_err(value_error)?;
    Ok(PyArray1::from_vec(py, output.iter().copied().collect()))
}

#[pyfunction]
fn pchip_interpolate<'py>(
    py: Python<'py>,
    x: PyReadonlyArray1<'_, f64>,
    y: PyReadonlyArray1<'_, f64>,
    xi: PyReadonlyArray1<'_, f64>,
) -> PyResult<Bound<'py, PyArray1<f64>>> {
    let x = array1_f64(x);
    let y = array1_f64(y);
    let xi = array1_f64(xi);
    let interpolator = PchipInterpolator::new(&x.view(), &y.view(), true).map_err(value_error)?;
    let output = interpolator
        .evaluate_array(&xi.view())
        .map_err(value_error)?;
    Ok(PyArray1::from_vec(py, output.iter().copied().collect()))
}

#[pyfunction]
fn cubic_interpolate<'py>(
    py: Python<'py>,
    x: PyReadonlyArray1<'_, f64>,
    y: PyReadonlyArray1<'_, f64>,
    xi: PyReadonlyArray1<'_, f64>,
) -> PyResult<Bound<'py, PyArray1<f64>>> {
    let x = array1_f64(x);
    let y = array1_f64(y);
    let xi = array1_f64(xi);
    let interpolator = Interp1d::new(
        &x.view(),
        &y.view(),
        InterpolationMethod::Cubic,
        ExtrapolateMode::Extrapolate,
    )
    .map_err(value_error)?;
    let output = interpolator
        .evaluate_array(&xi.view())
        .map_err(value_error)?;
    Ok(PyArray1::from_vec(py, output.iter().copied().collect()))
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(kdtree_query, m)?)?;
    m.add_function(wrap_pyfunction!(convex_hull_triangles, m)?)?;
    m.add_function(wrap_pyfunction!(butter_lfilter, m)?)?;
    m.add_function(wrap_pyfunction!(distance_transform_indices_2d, m)?)?;
    m.add_function(wrap_pyfunction!(label_2d_native, m)?)?;
    m.add_function(wrap_pyfunction!(find_objects_2d_native, m)?)?;
    m.add_function(wrap_pyfunction!(gaussian_filter1d, m)?)?;
    m.add_function(wrap_pyfunction!(median_filter1d, m)?)?;
    m.add_function(wrap_pyfunction!(pchip_interpolate, m)?)?;
    m.add_function(wrap_pyfunction!(cubic_interpolate, m)?)?;
    Ok(())
}

#[pymodule]
fn scientific_native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_module(m)
}
