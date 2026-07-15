use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum BehaviorEvalError {
    #[error("invalid behavior graph: {0}")]
    InvalidGraph(String),

    #[error("missing required member `{member}` on {class_name} object {object_index}")]
    MissingMember {
        object_index: usize,
        class_name: String,
        member: String,
    },

    #[error("invalid member `{member}` on {class_name} object {object_index}: {detail}")]
    InvalidMember {
        object_index: usize,
        class_name: String,
        member: String,
        detail: String,
    },

    #[error("unsupported active class `{class_name}` at object {object_index}")]
    UnsupportedActiveClass {
        object_index: usize,
        class_name: String,
    },

    #[error("unsupported active condition `{class_name}` at object {object_index}")]
    UnsupportedActiveCondition {
        object_index: usize,
        class_name: String,
    },

    #[error("unsupported active transition feature `{feature}` at object {object_index}")]
    UnsupportedTransitionFeature {
        object_index: usize,
        feature: String,
    },

    #[error("unsupported binding type {binding_type} for `{member_path}` at object {object_index}")]
    UnsupportedBinding {
        object_index: usize,
        member_path: String,
        binding_type: i32,
    },

    #[error("unsupported variable type {variable_type} for `{name}`")]
    UnsupportedVariableType { name: String, variable_type: i32 },

    #[error("variable `{name}` expects {expected}, got {actual}")]
    VariableTypeMismatch {
        name: String,
        expected: &'static str,
        actual: &'static str,
    },

    #[error("unknown variable `{0}`")]
    UnknownVariable(String),

    #[error("unknown event `{0}`")]
    UnknownEvent(String),

    #[error("generator selector did not resolve: {0}")]
    UnknownGenerator(String),

    #[error("generator selector is ambiguous: {0}")]
    AmbiguousGenerator(String),

    #[error("state machine `{state_machine}` has no state {state_id}")]
    UnknownState {
        state_machine: String,
        state_id: i32,
    },

    #[error("animation `{animation}` required by clip object {clip_object} was not loaded")]
    MissingAnimation {
        clip_object: usize,
        animation: String,
    },

    #[error("unsupported clip mode {mode} at object {object_index}")]
    UnsupportedClipMode { object_index: usize, mode: i32 },

    #[error("invalid timestep {0:?}; dt must be finite and non-negative")]
    InvalidTimestep(f32),

    #[error("event dispatch exceeded the deterministic limit")]
    EventDispatchLimit,
}

pub type BehaviorEvalResult<T> = Result<T, BehaviorEvalError>;
