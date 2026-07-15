use numpy::{PyArray1, PyReadonlyArray1};
use pyo3::prelude::*;

pub mod pitch_shift;

#[pyfunction]
#[pyo3(name = "pitch_shift", signature = (samples, sr, semitones))]
fn pitch_shift_py<'py>(
    py: Python<'py>,
    samples: PyReadonlyArray1<'py, f32>,
    sr: u32,
    semitones: f32,
) -> PyResult<Bound<'py, PyArray1<f32>>> {
    let _ = sr; // accepted for API symmetry with librosa; not used internally
    let samples_vec: Vec<f32> = samples.as_array().iter().copied().collect();
    let out_vec = py.detach(move || {
        let arr = ndarray::Array1::from_vec(samples_vec);
        pitch_shift::pitch_shift(arr.view(), semitones)
            .into_raw_vec_and_offset()
            .0
    });
    Ok(PyArray1::from_vec(py, out_vec))
}

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(pitch_shift_py, m)?)?;
    Ok(())
}

#[pymodule]
fn audio_native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_module(m)
}
