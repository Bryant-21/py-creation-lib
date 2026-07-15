pub mod animation_bridge;
pub mod bake;
pub mod edit;
pub mod io;
pub mod materials;
pub mod reverse;
pub mod runtime;
pub mod schema;
pub mod setup;
pub mod skeleton;
pub mod skinning;
pub mod solver;
pub mod templates;
pub mod topology;
pub mod units;
pub mod validate;
pub mod vcp;
pub mod winding_optimize;

pub use bake::bake_cloth_setup;
pub use edit::ClothEditor;
pub use io::{
    ClothClassInventoryEntry, ClothMetadata, cloth_metadata_from_blob, load_cloth_hkx,
    validate_cloth_blob,
};
pub use materials::MaterialPreset;
pub use reverse::{LossyMode, ReverseError, reverse_cloth_data, reverse_cloth_data_lossy};
pub use runtime::{ClothData, SimClothData};
pub use schema::{HAVOK_VERSION, KNOWN_CLASSES, expand_from_fixture, is_known};
pub use setup::{
    BufferSetupObject, ClothSetupObject, ConstraintSetupObject, OperatorSetupObject,
    SimClothSetupObject, TransformSetSetupObject,
};
pub use skeleton::ClothBone;
pub use solver::{Solver, SolverConfig};
pub use topology::TopologyPreset;
pub use validate::{LintIssue, Severity, ValidationResult, validate_cloth_data};
