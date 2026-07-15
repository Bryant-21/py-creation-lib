use pyo3::PyErr;
use pyo3::exceptions::{PyKeyError, PyRuntimeError, PyValueError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorldRendererError {
    #[error("invalid world renderer JSON for {context}: {source}")]
    InvalidJson {
        context: &'static str,
        source: serde_json::Error,
    },
    #[error("world renderer handle not found: {kind} {id}")]
    MissingHandle { kind: &'static str, id: u64 },
    #[error("buffer not found: {0}")]
    MissingBuffer(String),
    #[error("{0}")]
    Message(String),
}

impl From<WorldRendererError> for PyErr {
    fn from(value: WorldRendererError) -> Self {
        match value {
            WorldRendererError::InvalidJson { .. } => PyValueError::new_err(value.to_string()),
            WorldRendererError::MissingHandle { .. } | WorldRendererError::MissingBuffer(_) => {
                PyKeyError::new_err(value.to_string())
            }
            WorldRendererError::Message(_) => PyRuntimeError::new_err(value.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, WorldRendererError>;
