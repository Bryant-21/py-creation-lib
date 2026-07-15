use std::error::Error;
use std::fmt;
use std::path::PathBuf;

pub use havok_native::behavior_eval::{GeneratorSelector, PathAction};

use super::contour::{CenterMode, Entry};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RecipeRecordHandle(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EvaluationRequestId(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProducerClass {
    Collection,
    Individual,
    SpeedSampled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecipeRecordParentage {
    SourceRoot {
        behavior_file: usize,
        state_machine: usize,
    },
    Child {
        parent: RecipeRecordHandle,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProducerRecord {
    pub handle: RecipeRecordHandle,
    pub recipe_ordinal: u32,
    pub class: ProducerClass,
    pub parentage: RecipeRecordParentage,
    pub source_file: usize,
    pub source_object: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NeedsEvaluation {
    pub records: Vec<ProducerRecord>,
    pub roots: Vec<SpeedInfoRootRecipe>,
    pub requests: Vec<EvaluationRequest>,
}

impl NeedsEvaluation {
    pub fn stats(&self) -> RecipeStats {
        let mut stats = RecipeStats {
            roots: self.roots.len(),
            ..RecipeStats::default()
        };
        for root in &self.roots {
            root.contour.add_stats(&mut stats);
        }
        stats
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpeedInfoRootRecipe {
    pub record: RecipeRecordHandle,
    pub state_machine_path: String,
    pub contour: RecipeContour,
    pub metadata: RootMetadataRecipe,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RootMetadataRecipe {
    Collection {
        center_mode: CenterMode,
        evaluation: EvaluationRequestId,
    },
    DirectIndividual {
        evaluation: EvaluationRequestId,
    },
    DirectSpeedSampled {
        evaluation: EvaluationRequestId,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum RecipeContour {
    Collection {
        record: RecipeRecordHandle,
        children: Vec<RecipeContour>,
    },
    Individual {
        record: RecipeRecordHandle,
        parameter: String,
        clip: String,
        condition: String,
        entry: RecipeEntry,
        evaluation: EvaluationRequestId,
    },
    SpeedSampled {
        record: RecipeRecordHandle,
        center_mode: CenterMode,
        clip: String,
        condition: String,
        entry: RecipeEntry,
        evaluation: EvaluationRequestId,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum RecipeEntry {
    Selector(Entry),
    RootMetadata(EvaluationRequestId),
}

impl RecipeContour {
    pub fn stats(&self) -> RecipeStats {
        let mut stats = RecipeStats::default();
        self.add_stats(&mut stats);
        stats
    }

    fn add_stats(&self, stats: &mut RecipeStats) {
        match self {
            Self::Collection { children, .. } => {
                stats.collections += 1;
                for child in children {
                    child.add_stats(stats);
                }
            }
            Self::Individual { .. } => stats.individuals += 1,
            Self::SpeedSampled { .. } => stats.speed_sampled += 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct RecipeStats {
    pub roots: usize,
    pub collections: usize,
    pub individuals: usize,
    pub speed_sampled: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EvaluationRequest {
    Individual(IndividualEvaluation),
    SpeedSampled(SpeedSampledEvaluation),
    RootMetadata(RootMetadataEvaluation),
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndividualEvaluation {
    pub record: RecipeRecordHandle,
    pub animation_name: String,
    pub animation_path: PathBuf,
    pub playback_parameter: Option<String>,
    pub replay: Vec<BehaviorReplay>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpeedSampledEvaluation {
    pub record: RecipeRecordHandle,
    pub domain: SampleDomain,
    pub directional_summary: Vec<DirectionalSummaryEvaluation>,
    pub replay: Vec<BehaviorReplay>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DirectionalSummaryEvaluation {
    pub animation_name: String,
    pub animation_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RootMetadataEvaluation {
    pub record: RecipeRecordHandle,
    pub replay: Vec<BehaviorReplay>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BehaviorGraphOwner {
    pub behavior_file: usize,
    pub relative_path: String,
    pub graph_name: String,
}

/// One evaluator load/replay segment. Segments and actions are applied in vector order.
#[derive(Clone, Debug, PartialEq)]
pub struct BehaviorReplay {
    pub owner: BehaviorGraphOwner,
    pub root: GeneratorSelector,
    pub actions: Vec<PathAction>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SampleDomain {
    pub direction_variable: String,
    pub speed_variable: String,
    pub direction_min: f32,
    pub direction_max: f32,
    pub direction_step: f32,
    pub speed_min: f32,
    pub speed_max: f32,
    pub speed_step: f32,
    pub warmup_updates: u32,
    pub measurement_updates: u32,
    pub timestep: f32,
    pub displacement_scale: f32,
    pub chord_error_tolerance: f32,
}

impl SampleDomain {
    pub fn historical_fo4(
        direction_variable: String,
        speed_variable: String,
        direction_min: f32,
        direction_max: f32,
        speed_min: f32,
        speed_max: f32,
    ) -> Self {
        Self {
            direction_variable,
            speed_variable,
            direction_min,
            direction_max,
            // These sampling/reduction values are historical CK policy. Variable names and
            // domain endpoints are supplied by the source graph and are not policy constants.
            direction_step: f32::from_bits(0x3e86_0a92),
            speed_min,
            speed_max,
            speed_step: 20.0,
            warmup_updates: 8,
            measurement_updates: 29,
            timestep: f32::from_bits(0x3d08_8889),
            displacement_scale: f32::from_bits(0x41ef_ffff),
            chord_error_tolerance: 2.0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpeedInfoProducerError {
    BehaviorNotFound(String),
    BehaviorDecode(String),
    UnsupportedProducerClass {
        class_name: String,
        behavior_file: usize,
        object_index: usize,
    },
    MissingProducerData {
        behavior_file: usize,
        object_index: usize,
        field: &'static str,
    },
    RecordHandleOverflow,
    RequestIdOverflow,
    Evaluation(String),
    MissingEvaluation(EvaluationRequestId),
    NoSpeedInfoTopology,
}

impl fmt::Display for SpeedInfoProducerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BehaviorNotFound(path) => write!(f, "behavior not found: {path}"),
            Self::BehaviorDecode(path) => write!(f, "could not decode behavior: {path}"),
            Self::UnsupportedProducerClass {
                class_name,
                behavior_file,
                object_index,
            } => write!(
                f,
                "unsupported SpeedInfo producer class {class_name} at file {behavior_file}, object {object_index}"
            ),
            Self::MissingProducerData {
                behavior_file,
                object_index,
                field,
            } => write!(
                f,
                "missing SpeedInfo producer field {field} at file {behavior_file}, object {object_index}"
            ),
            Self::RecordHandleOverflow => write!(f, "SpeedInfo recipe record handle overflow"),
            Self::RequestIdOverflow => write!(f, "SpeedInfo evaluation request ID overflow"),
            Self::Evaluation(reason) => write!(f, "SpeedInfo evaluation failed: {reason}"),
            Self::MissingEvaluation(id) => {
                write!(f, "SpeedInfo evaluation request {} was not resolved", id.0)
            }
            Self::NoSpeedInfoTopology => write!(f, "behavior has no SpeedInfo topology"),
        }
    }
}

impl Error for SpeedInfoProducerError {}
