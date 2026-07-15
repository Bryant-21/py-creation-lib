use serde::Serialize;

use super::model::{RootTranslation, VariableValue};

pub const BEHAVIOR_TRACE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BehaviorTrace {
    pub schema_version: u32,
    pub advances: Vec<AdvanceTrace>,
}

impl Default for BehaviorTrace {
    fn default() -> Self {
        Self {
            schema_version: BEHAVIOR_TRACE_SCHEMA_VERSION,
            advances: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AdvanceTrace {
    pub sequence: u64,
    pub dt: f32,
    pub active_paths: Vec<ActivePathTrace>,
    pub transitions: Vec<TransitionTrace>,
    pub cyclic: Vec<CyclicTrace>,
    pub blenders: Vec<BlenderTrace>,
    pub clips: Vec<ClipTrace>,
    pub node_outputs: Vec<NodeOutputTrace>,
    pub operations: Vec<TraceOperation>,
    pub output: RootTranslation,
    pub accumulator: RootTranslation,
}

impl AdvanceTrace {
    pub(crate) fn new(sequence: u64, dt: f32) -> Self {
        Self {
            sequence,
            dt,
            active_paths: Vec::new(),
            transitions: Vec::new(),
            cyclic: Vec::new(),
            blenders: Vec::new(),
            clips: Vec::new(),
            node_outputs: Vec::new(),
            operations: Vec::new(),
            output: RootTranslation::ZERO,
            accumulator: RootTranslation::ZERO,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActivePathTrace {
    pub nodes: Vec<PathNodeTrace>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathNodeTrace {
    pub object: usize,
    pub class: String,
    pub name: Option<String>,
    pub state: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TransitionTrace {
    pub state_machine: usize,
    pub from_state: i32,
    pub to_state: i32,
    pub from_generator: usize,
    pub to_generator: usize,
    pub elapsed_before: f32,
    pub elapsed: f32,
    pub duration: f32,
    pub from_weight: f32,
    pub to_weight: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CyclicModeTrace {
    Frozen,
    Blending,
    Settled,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CyclicTrace {
    pub object: usize,
    pub current: f32,
    pub start: f32,
    pub target: f32,
    pub elapsed: f32,
    pub duration: f32,
    pub mode: CyclicModeTrace,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PhaseTrace {
    pub old: f32,
    pub new: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BlenderChildTrace {
    pub child: usize,
    pub generator: usize,
    pub weight: f32,
    pub root_weight: f32,
    pub phase: Option<PhaseTrace>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BlenderTrace {
    pub object: usize,
    pub parameter: Option<f32>,
    pub sync_master: Option<usize>,
    pub input_phase: Option<PhaseTrace>,
    pub children: Vec<BlenderChildTrace>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ClipTrace {
    pub object: usize,
    pub path: Vec<PathNodeTrace>,
    pub animation: String,
    pub mode: i32,
    pub old_local_time: f32,
    pub new_local_time: f32,
    pub wraps: i32,
    pub effective_rate: f32,
    pub phase: PhaseTrace,
    pub raw_reference_delta: RootTranslation,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodeOutputTrace {
    pub object: usize,
    pub path: Vec<PathNodeTrace>,
    pub root_translation: RootTranslation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VariableWriteSourceTrace {
    External,
    Assignment,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TraceOperation {
    VariableWrite {
        source: VariableWriteSourceTrace,
        index: usize,
        name: String,
        old: VariableValue,
        new: VariableValue,
    },
    Event {
        id: i32,
        name: Option<String>,
    },
    StateSet {
        state_machine: usize,
        state: i32,
    },
}
