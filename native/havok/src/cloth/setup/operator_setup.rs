use serde::{Deserialize, Serialize};

use super::types::{TriangleSelectionInput, VertexSelectionInput};
use crate::hkx::model::HkxMember;

// ---------------------------------------------------------------------------
// Tagged enum — mirrors Python's SETUP_TYPE factory dispatch
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OperatorSetupObject {
    Simulate(SimulateSetup),
    MeshBoneDeform(MeshBoneDeformSetup),
    Skin(SkinSetup),
    CopyVertices(CopyVerticesSetup),
    MoveParticles(MoveParticlesSetup),
    GatherAllVertices(GatherAllVerticesSetup),
    Opaque(OpaqueOperatorSetup),
}

impl OperatorSetupObject {
    pub fn name(&self) -> &str {
        match self {
            Self::Simulate(s) => &s.name,
            Self::MeshBoneDeform(s) => &s.name,
            Self::Skin(s) => &s.name,
            Self::CopyVertices(s) => &s.name,
            Self::MoveParticles(s) => &s.name,
            Self::GatherAllVertices(s) => &s.name,
            Self::Opaque(s) => &s.name,
        }
    }
}

// ---------------------------------------------------------------------------
// Simulate
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulateSetupConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default = "one_usize")]
    pub num_substeps: usize,
    #[serde(default)]
    pub adapt_constraint_stiffness: bool,
    #[serde(default = "three_usize")]
    pub num_solve_iterations: usize,
    #[serde(default = "default_true")]
    pub use_all_collidables: bool,
    #[serde(default)]
    pub specific_collidables: Vec<String>,
    #[serde(default)]
    pub explicit_constraint_order: bool,
    #[serde(default)]
    pub constraint_execution_order_names: Vec<String>,
}

impl Default for SimulateSetupConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            num_substeps: 1,
            adapt_constraint_stiffness: false,
            num_solve_iterations: 3,
            use_all_collidables: true,
            specific_collidables: Vec::new(),
            explicit_constraint_order: false,
            constraint_execution_order_names: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulateSetup {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub sim_cloth_setup_name: String,
    #[serde(default)]
    pub configs: Vec<SimulateSetupConfig>,
}

impl Default for SimulateSetup {
    fn default() -> Self {
        Self {
            name: String::new(),
            sim_cloth_setup_name: String::new(),
            configs: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// MeshBoneDeform
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshBoneDeformSetup {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub input_buffer_name: String,
    #[serde(default)]
    pub input_triangle_selection: TriangleSelectionInput,
    #[serde(default)]
    pub output_transform_set_name: String,
    #[serde(default)]
    pub deformed_bones: Vec<String>,
    #[serde(default = "one_usize")]
    pub max_triangles_per_bone: usize,
    #[serde(default)]
    pub minimum_triangle_weight: f32,
    #[serde(default)]
    pub bone_rest_positions: Vec<Vec<f32>>,
}

impl Default for MeshBoneDeformSetup {
    fn default() -> Self {
        Self {
            name: String::new(),
            input_buffer_name: String::new(),
            input_triangle_selection: TriangleSelectionInput::default(),
            output_transform_set_name: String::new(),
            deformed_bones: Vec::new(),
            max_triangles_per_bone: 1,
            minimum_triangle_weight: 0.0,
            bone_rest_positions: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Skin
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkinSetup {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub transform_set_name: String,
    #[serde(default)]
    pub reference_buffer_name: String,
    #[serde(default)]
    pub output_buffer_name: String,
    #[serde(default)]
    pub vertex_selection: VertexSelectionInput,
    #[serde(default = "default_true")]
    pub skin_normals: bool,
    #[serde(default)]
    pub skin_tangents: bool,
    #[serde(default)]
    pub skin_bitangents: bool,
    #[serde(default)]
    pub use_dual_quaternion: bool,
}

impl Default for SkinSetup {
    fn default() -> Self {
        Self {
            name: String::new(),
            transform_set_name: String::new(),
            reference_buffer_name: String::new(),
            output_buffer_name: String::new(),
            vertex_selection: VertexSelectionInput::default(),
            skin_normals: true,
            skin_tangents: false,
            skin_bitangents: false,
            use_dual_quaternion: false,
        }
    }
}

// ---------------------------------------------------------------------------
// CopyVertices
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopyVerticesSetup {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub input_buffer_name: String,
    #[serde(default)]
    pub output_buffer_name: String,
    #[serde(default = "default_true")]
    pub copy_normals: bool,
}

impl Default for CopyVerticesSetup {
    fn default() -> Self {
        Self {
            name: String::new(),
            input_buffer_name: String::new(),
            output_buffer_name: String::new(),
            copy_normals: true,
        }
    }
}

// ---------------------------------------------------------------------------
// MoveParticles
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveParticlesSetup {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub sim_cloth_setup_name: String,
    #[serde(default)]
    pub display_buffer_name: String,
}

impl Default for MoveParticlesSetup {
    fn default() -> Self {
        Self {
            name: String::new(),
            sim_cloth_setup_name: String::new(),
            display_buffer_name: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// GatherAllVertices — non-sparse vertex gather from input → output buffer.
// Mirrors SDK `hclGatherAllVerticesOperator`.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatherAllVerticesSetup {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub input_buffer_name: String,
    #[serde(default)]
    pub output_buffer_name: String,
    /// Per-output-vertex source index into the input buffer. -1 marks an
    /// output vertex that should not be written (sparse position).
    #[serde(default)]
    pub vertex_input_from_vertex_output: Vec<i16>,
    #[serde(default = "default_true")]
    pub gather_normals: bool,
    /// Whether any output vertex has source index -1. Defaults to false; the
    /// bake pipeline derives this automatically from the index list when not
    /// set explicitly.
    #[serde(default)]
    pub partial_gather: bool,
}

impl Default for GatherAllVerticesSetup {
    fn default() -> Self {
        Self {
            name: String::new(),
            input_buffer_name: String::new(),
            output_buffer_name: String::new(),
            vertex_input_from_vertex_output: Vec::new(),
            gather_normals: true,
            partial_gather: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Opaque — pass-through blob for unknown operator classes
// ---------------------------------------------------------------------------

/// Round-trip container for operator classes not modelled in setup.
/// `members` is NOT serialized (serde skip) so this is in-process only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpaqueOperatorSetup {
    pub name: String,
    pub class_name: String,
    #[serde(skip)]
    pub members: Vec<HkxMember>,
}

impl Default for OpaqueOperatorSetup {
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

fn one_usize() -> usize {
    1
}
fn three_usize() -> usize {
    3
}
fn default_true() -> bool {
    true
}
