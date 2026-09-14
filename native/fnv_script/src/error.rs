use thiserror::Error;

#[derive(Debug, Error)]
pub enum FnvScriptError {
    #[error("script parse error at line {line}, col {col}: {msg}")]
    Parse {
        line: usize,
        col: usize,
        msg: String,
    },
    #[error("script translation error: unmapped {kind} '{name}'")]
    Translate { kind: &'static str, name: String },
    #[error("script translation error: unsupported {kind} '{name}' ({reason})")]
    Unsupported {
        kind: &'static str,
        name: String,
        reason: String,
    },
    #[error("script translation drop: {kind} '{name}' ({reason})")]
    Drop {
        kind: &'static str,
        name: String,
        reason: String,
    },
    #[error("formkey resolution error: {0}")]
    FormKey(String),
    #[error("vmad synth error: {0}")]
    VmadSynth(String),
    #[error("decompile error: {0}")]
    Decompile(String),
}
