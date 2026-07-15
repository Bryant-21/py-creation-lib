use thiserror::Error;

pub type HavokResult<T> = Result<T, HavokError>;

#[derive(Debug, Error)]
pub enum HavokError {
    #[error("unsupported Havok format: {0}")]
    UnsupportedFormat(String),

    #[error("unknown Havok version: {0}")]
    UnknownVersion(String),

    #[error(
        "conversion not yet implemented: source={source_version} target={target_version} route={route}: {reason}"
    )]
    ConversionNotImplemented {
        source_version: u8,
        target_version: u8,
        route: String,
        reason: String,
    },

    #[error("unported conversion edge case: route={route} edge_case={edge_case}: {detail}")]
    UnportedEdgeCase {
        route: String,
        edge_case: String,
        detail: String,
    },

    #[error("I/O error at {path}: {operation}: {source}")]
    Io {
        path: String,
        operation: &'static str,
        source: std::io::Error,
    },

    #[error("feature not yet implemented: {feature}: {reason}")]
    FeatureNotImplemented { feature: String, reason: String },

    #[error("invalid input: {0}")]
    InvalidInput(String),
}
