use serde::{Deserialize, Serialize};

use super::types::{EdgeSelectionInput, VertexFloatInput, VertexSelectionInput};
use crate::hkx::model::HkxMember;

// ---------------------------------------------------------------------------
// Tagged enum — mirrors Python's SETUP_TYPE factory dispatch
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ConstraintSetupObject {
    StandardLink(StandardLinkSetup),
    StretchLink(StretchLinkSetup),
    BendStiffness(BendStiffnessSetup),
    LocalRange(LocalRangeSetup),
    BonePlanes(BonePlanesSetup),
    Volume(VolumeSetup),
    Opaque(OpaqueConstraintSetup),
}

impl ConstraintSetupObject {
    pub fn setup_type(&self) -> &'static str {
        match self {
            Self::StandardLink(_) => "StandardLink",
            Self::StretchLink(_) => "StretchLink",
            Self::BendStiffness(_) => "BendStiffness",
            Self::LocalRange(_) => "LocalRange",
            Self::BonePlanes(_) => "BonePlanes",
            Self::Volume(_) => "Volume",
            Self::Opaque(_) => "Opaque",
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::StandardLink(s) => &s.name,
            Self::StretchLink(s) => &s.name,
            Self::BendStiffness(s) => &s.name,
            Self::LocalRange(s) => &s.name,
            Self::BonePlanes(s) => &s.name,
            Self::Volume(s) => &s.name,
            Self::Opaque(s) => &s.name,
        }
    }
}

// ---------------------------------------------------------------------------
// StandardLink
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StandardLinkSetup {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub vertex_selection: VertexSelectionInput,
    #[serde(default)]
    pub edge_selection: EdgeSelectionInput,
    #[serde(default)]
    pub ignore_hidden_edges: bool,
    #[serde(default = "vfi_one")]
    pub stiffness: VertexFloatInput,
    #[serde(default)]
    pub allowed_compression: VertexFloatInput,
    #[serde(default)]
    pub allowed_stretching: VertexFloatInput,
}

impl Default for StandardLinkSetup {
    fn default() -> Self {
        Self {
            name: String::new(),
            vertex_selection: VertexSelectionInput::default(),
            edge_selection: EdgeSelectionInput::default(),
            ignore_hidden_edges: false,
            stiffness: VertexFloatInput::constant(1.0),
            allowed_compression: VertexFloatInput::constant(0.0),
            allowed_stretching: VertexFloatInput::constant(0.0),
        }
    }
}

// ---------------------------------------------------------------------------
// StretchLink
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StretchLinkSetup {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub movable_particles_selection: VertexSelectionInput,
    #[serde(default)]
    pub fixed_particles_selection: VertexSelectionInput,
    #[serde(default = "vfi_one")]
    pub rigid_factor: VertexFloatInput,
    #[serde(default = "vfi_one")]
    pub stiffness: VertexFloatInput,
    #[serde(default)]
    pub stretch_direction: [f32; 4],
    #[serde(default)]
    pub use_stretch_direction: bool,
    #[serde(default)]
    pub use_mesh_topology: bool,
    #[serde(default)]
    pub allow_dynamic_links: bool,
    #[serde(default)]
    pub use_topological_stretch_distance: bool,
}

impl Default for StretchLinkSetup {
    fn default() -> Self {
        Self {
            name: String::new(),
            movable_particles_selection: VertexSelectionInput::default(),
            fixed_particles_selection: VertexSelectionInput::default(),
            rigid_factor: VertexFloatInput::constant(1.0),
            stiffness: VertexFloatInput::constant(1.0),
            stretch_direction: [0.0; 4],
            use_stretch_direction: false,
            use_mesh_topology: false,
            allow_dynamic_links: false,
            use_topological_stretch_distance: false,
        }
    }
}

// ---------------------------------------------------------------------------
// BendStiffness
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BendStiffnessSetup {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub vertex_selection: VertexSelectionInput,
    #[serde(default = "vfi_half")]
    pub bend_stiffness: VertexFloatInput,
    #[serde(default = "default_true")]
    pub use_rest_pose_config: bool,
}

impl Default for BendStiffnessSetup {
    fn default() -> Self {
        Self {
            name: String::new(),
            vertex_selection: VertexSelectionInput::default(),
            bend_stiffness: VertexFloatInput::constant(0.5),
            use_rest_pose_config: true,
        }
    }
}

// ---------------------------------------------------------------------------
// LocalRange
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalRangeSetup {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub vertex_selection: VertexSelectionInput,
    #[serde(default = "vfi_one")]
    pub maximum_distance: VertexFloatInput,
    #[serde(default = "vfi_neg_one")]
    pub min_normal_distance: VertexFloatInput,
    #[serde(default = "vfi_one")]
    pub max_normal_distance: VertexFloatInput,
    #[serde(default = "one_f32")]
    pub stiffness: f32,
    #[serde(default)]
    pub local_range_shape: i32,
    #[serde(default)]
    pub use_min_normal_distance: bool,
    #[serde(default)]
    pub use_max_normal_distance: bool,
}

impl Default for LocalRangeSetup {
    fn default() -> Self {
        Self {
            name: String::new(),
            vertex_selection: VertexSelectionInput::default(),
            maximum_distance: VertexFloatInput::constant(1.0),
            min_normal_distance: VertexFloatInput::constant(-1.0),
            max_normal_distance: VertexFloatInput::constant(1.0),
            stiffness: 1.0,
            local_range_shape: 0,
            use_min_normal_distance: false,
            use_max_normal_distance: false,
        }
    }
}

// ---------------------------------------------------------------------------
// BonePlanes sub-types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerParticlePlane {
    #[serde(default)]
    pub transform_name: String,
    #[serde(default)]
    pub particles: VertexSelectionInput,
    #[serde(default = "direction_default")]
    pub direction_bone_space: [f32; 4],
    #[serde(default)]
    pub allowed_distance: VertexFloatInput,
    #[serde(default = "vfi_one")]
    pub stiffness: VertexFloatInput,
}

impl Default for PerParticlePlane {
    fn default() -> Self {
        Self {
            transform_name: String::new(),
            particles: VertexSelectionInput::default(),
            direction_bone_space: [0.0, 1.0, 0.0, 0.0],
            allowed_distance: VertexFloatInput::constant(0.0),
            stiffness: VertexFloatInput::constant(1.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalPlane {
    #[serde(default)]
    pub transform_name: String,
    #[serde(default)]
    pub particles: VertexSelectionInput,
    #[serde(default = "direction_default")]
    pub plane_equation_bone_space: [f32; 4],
    #[serde(default)]
    pub allowed_penetration: VertexFloatInput,
    #[serde(default = "vfi_one")]
    pub stiffness: VertexFloatInput,
}

impl Default for GlobalPlane {
    fn default() -> Self {
        Self {
            transform_name: String::new(),
            particles: VertexSelectionInput::default(),
            plane_equation_bone_space: [0.0, 1.0, 0.0, 0.0],
            allowed_penetration: VertexFloatInput::constant(0.0),
            stiffness: VertexFloatInput::constant(1.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerParticleAngle {
    #[serde(default)]
    pub transform_name: String,
    #[serde(default)]
    pub particles_max_angle: VertexSelectionInput,
    #[serde(default)]
    pub particles_min_angle: VertexSelectionInput,
    #[serde(default)]
    pub origin_bone_space: [f32; 4],
    #[serde(default = "direction_default")]
    pub axis_bone_space: [f32; 4],
    #[serde(default)]
    pub min_angle: VertexFloatInput,
    #[serde(default)]
    pub max_angle: VertexFloatInput,
    #[serde(default = "vfi_one")]
    pub stiffness: VertexFloatInput,
}

impl Default for PerParticleAngle {
    fn default() -> Self {
        Self {
            transform_name: String::new(),
            particles_max_angle: VertexSelectionInput::default(),
            particles_min_angle: VertexSelectionInput::default(),
            origin_bone_space: [0.0; 4],
            axis_bone_space: [0.0, 1.0, 0.0, 0.0],
            min_angle: VertexFloatInput::constant(0.0),
            max_angle: VertexFloatInput::constant(0.0),
            stiffness: VertexFloatInput::constant(1.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BonePlanesSetup {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub per_particle_planes: Vec<PerParticlePlane>,
    #[serde(default)]
    pub global_planes: Vec<GlobalPlane>,
    #[serde(default)]
    pub per_particle_angles: Vec<PerParticleAngle>,
    #[serde(default = "default_true")]
    pub angle_specified_in_degrees: bool,
}

impl Default for BonePlanesSetup {
    fn default() -> Self {
        Self {
            name: String::new(),
            per_particle_planes: Vec::new(),
            global_planes: Vec::new(),
            per_particle_angles: Vec::new(),
            angle_specified_in_degrees: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Volume
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VolumeSetup {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub apply_to_particles: VertexSelectionInput,
    #[serde(default = "vfi_one")]
    pub stiffness: VertexFloatInput,
    #[serde(default)]
    pub influence_particles: VertexSelectionInput,
    #[serde(default = "vfi_one")]
    pub particle_weights: VertexFloatInput,
}

impl Default for VolumeSetup {
    fn default() -> Self {
        Self {
            name: String::new(),
            apply_to_particles: VertexSelectionInput::default(),
            stiffness: VertexFloatInput::constant(1.0),
            influence_particles: VertexSelectionInput::default(),
            particle_weights: VertexFloatInput::constant(1.0),
        }
    }
}

// ---------------------------------------------------------------------------
// Opaque — pass-through blob for unknown constraint classes
// ---------------------------------------------------------------------------

/// Round-trip container for constraint classes not modelled in setup.
/// `members` is NOT serialized (serde skip) so this is in-process only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpaqueConstraintSetup {
    pub name: String,
    pub class_name: String,
    #[serde(skip)]
    pub members: Vec<HkxMember>,
}

impl Default for OpaqueConstraintSetup {
    fn default() -> Self {
        Self {
            name: String::new(),
            class_name: String::new(),
            members: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn vfi_one() -> VertexFloatInput {
    VertexFloatInput::constant(1.0)
}
fn vfi_half() -> VertexFloatInput {
    VertexFloatInput::constant(0.5)
}
fn vfi_neg_one() -> VertexFloatInput {
    VertexFloatInput::constant(-1.0)
}
fn one_f32() -> f32 {
    1.0
}
fn default_true() -> bool {
    true
}
fn direction_default() -> [f32; 4] {
    [0.0, 1.0, 0.0, 0.0]
}
