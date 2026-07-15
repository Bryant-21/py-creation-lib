use pyo3::PyErr;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MaterialError {
    #[error("{0}")]
    InvalidData(String),
    #[error("{0}")]
    Runtime(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl MaterialError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidData(message.into())
    }

    pub fn runtime(message: impl Into<String>) -> Self {
        Self::Runtime(message.into())
    }
}

impl From<MaterialError> for PyErr {
    fn from(value: MaterialError) -> Self {
        match value {
            MaterialError::InvalidData(message) => PyValueError::new_err(message),
            MaterialError::Runtime(message) => PyRuntimeError::new_err(message),
            MaterialError::Json(error) => PyValueError::new_err(error.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, MaterialError>;
