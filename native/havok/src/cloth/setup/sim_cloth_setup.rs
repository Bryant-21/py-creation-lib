use serde::{Deserialize, Serialize};

use crate::cloth::units::GRAVITY_Z;
use crate::hkx::model::HkxMember;

use super::buffer_setup::TransformSetSetupObject;
use super::collidable_setup::CollidableSetup;
use super::constraint_setup::ConstraintSetupObject;
use super::mesh::SimulationSetupMesh;
use super::types::{VertexFloatInput, VertexSelectionInput};

/// Simulation cloth setup — the core authoring object.
/// Mirrors Python's SimClothSetupObject.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimClothSetupObject {
    #[serde(default)]
    pub name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub simulation_mesh: Option<SimulationSetupMesh>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub collidable_transform_set: Option<TransformSetSetupObject>,

    #[serde(default = "default_gravity")]
    pub gravity: [f32; 4],
    #[serde(default)]
    pub global_damping_per_second: f32,
    #[serde(default = "default_true")]
    pub do_normals: bool,

    #[serde(default)]
    pub specify_density: bool,
    #[serde(default = "vfi_one")]
    pub vertex_density: VertexFloatInput,
    #[serde(default)]
    pub rescale_mass: bool,
    #[serde(default = "one_f32")]
    pub total_mass: f32,
    #[serde(default = "vfi_002")]
    pub particle_mass: VertexFloatInput,
    #[serde(default = "vfi_05")]
    pub particle_radius: VertexFloatInput,
    #[serde(default = "vfi_035")]
    pub particle_friction: VertexFloatInput,

    #[serde(default)]
    pub fixed_particles: VertexSelectionInput,

    #[serde(default)]
    pub enable_pinch_detection: bool,
    #[serde(default = "vsi_none")]
    pub pinch_detection_enabled_particles: VertexSelectionInput,

    #[serde(default = "f32_03")]
    pub to_anim_period: f32,
    #[serde(default = "f32_03")]
    pub to_sim_period: f32,

    #[serde(default = "f32_05")]
    pub collision_tolerance: f32,

    #[serde(default)]
    pub enable_stuck_particle_detection: bool,
    #[serde(default = "f32_2")]
    pub stuck_particles_stretch_factor: f32,

    #[serde(default)]
    pub enable_transfer_motion: bool,

    #[serde(default)]
    pub constraint_setups: Vec<ConstraintSetupObject>,
    #[serde(default)]
    pub collidable_setups: Vec<CollidableSetup>,

    /// Per-triangle normal flip flags (parallel to simulation_mesh.triangles).
    /// SDK `hclSimClothData::m_triangleFlips`: `hkArray<hkUint8>` of length
    /// `triangleIndices.size() / 3`. When `do_normals=true` the runtime uses
    /// these to invert per-triangle normals; defaults to all-zero (no flip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triangle_flips: Option<Vec<u8>>,

    /// Index into `cloth_setup.transform_set_setups` describing which
    /// transform set drives the collidables of this sim cloth (SDK
    /// `m_collidableTransformMap.m_transformSetIndex`). Defaults to 0.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub collidable_transform_set_index: u32,

    /// Per-collidable hkMatrix4 offsets (16 floats, row-major) parallel to
    /// `collidable_setups`. SDK `m_collidableTransformMap.m_offsets` —
    /// without this every collidable snaps to the bone origin.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collidable_offsets: Vec<[f32; 16]>,

    /// Round-trip preservation bag for `hclSimClothData` members the
    /// authoring layer does not yet model semantically (m_simOpIds,
    /// m_actions, m_landscapeCollisionData, m_virtualCollisionPointsData,
    /// etc.). Populated by reverse-from-runtime; re-emitted verbatim by
    /// `bake.rs::emit_sim_cloth_data`. Skipped during JSON serialization
    /// since the values are not Serde-serializable; round-trip survival
    /// only applies within a single in-process bake/reverse cycle.
    #[serde(skip)]
    pub passthrough_members: Vec<HkxMember>,
}

impl Default for SimClothSetupObject {
    fn default() -> Self {
        Self {
            name: String::new(),
            simulation_mesh: None,
            collidable_transform_set: None,
            gravity: [0.0, 0.0, GRAVITY_Z, 0.0],
            global_damping_per_second: 0.0,
            do_normals: true,
            specify_density: false,
            vertex_density: VertexFloatInput::constant(1.0),
            rescale_mass: false,
            total_mass: 1.0,
            particle_mass: VertexFloatInput::constant(0.02),
            particle_radius: VertexFloatInput::constant(0.5),
            particle_friction: VertexFloatInput::constant(0.35),
            fixed_particles: VertexSelectionInput::default(),
            enable_pinch_detection: false,
            pinch_detection_enabled_particles: VertexSelectionInput::none(),
            to_anim_period: 0.3,
            to_sim_period: 0.3,
            collision_tolerance: 0.5,
            enable_stuck_particle_detection: false,
            stuck_particles_stretch_factor: 2.0,
            enable_transfer_motion: false,
            constraint_setups: Vec::new(),
            collidable_setups: Vec::new(),
            triangle_flips: None,
            collidable_transform_set_index: 0,
            collidable_offsets: Vec::new(),
            passthrough_members: Vec::new(),
        }
    }
}

// Helpers
fn default_gravity() -> [f32; 4] {
    [0.0, 0.0, GRAVITY_Z, 0.0]
}
fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}
fn default_true() -> bool {
    true
}
fn one_f32() -> f32 {
    1.0
}
fn vfi_one() -> VertexFloatInput {
    VertexFloatInput::constant(1.0)
}
fn vfi_002() -> VertexFloatInput {
    VertexFloatInput::constant(0.02)
}
fn vfi_05() -> VertexFloatInput {
    VertexFloatInput::constant(0.5)
}
fn vfi_035() -> VertexFloatInput {
    VertexFloatInput::constant(0.35)
}
fn vsi_none() -> VertexSelectionInput {
    VertexSelectionInput::none()
}
fn f32_03() -> f32 {
    0.3
}
fn f32_05() -> f32 {
    0.5
}
fn f32_2() -> f32 {
    2.0
}
