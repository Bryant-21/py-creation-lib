use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type Vec4 = [f32; 4];
pub type Tri = [u32; 3];

/// Mesh data for cloth setup — positions, triangles, channels, skinning.
/// Mirrors Python's SetupMesh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetupMesh {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub positions: Vec<Vec4>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub normals: Vec<Vec4>,
    #[serde(default)]
    pub triangles: Vec<Tri>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub vertex_float_channels: HashMap<String, Vec<f32>>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub vertex_selection_channels: HashMap<String, Vec<i32>>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub triangle_selection_channels: HashMap<String, Vec<i32>>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub edge_selection_channels: HashMap<String, Vec<i32>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bone_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bone_from_skin_transforms: Vec<Vec<f32>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bone_weights: Vec<Vec<[f32; 2]>>,
    #[serde(default = "default_identity")]
    pub world_from_mesh: Vec<f32>,
}

fn default_identity() -> Vec<f32> {
    vec![
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

impl Default for SetupMesh {
    fn default() -> Self {
        Self {
            name: String::new(),
            positions: Vec::new(),
            normals: Vec::new(),
            triangles: Vec::new(),
            vertex_float_channels: HashMap::new(),
            vertex_selection_channels: HashMap::new(),
            triangle_selection_channels: HashMap::new(),
            edge_selection_channels: HashMap::new(),
            bone_names: Vec::new(),
            bone_from_skin_transforms: Vec::new(),
            bone_weights: Vec::new(),
            world_from_mesh: default_identity(),
        }
    }
}

/// Simulation mesh — merged duplicate vertices. Mirrors Python's SimulationSetupMesh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulationSetupMesh {
    #[serde(default)]
    pub positions: Vec<Vec4>,
    #[serde(default)]
    pub triangles: Vec<Tri>,
    #[serde(default)]
    pub sim_to_render_map: Vec<Vec<u32>>,
    #[serde(default)]
    pub render_to_sim_map: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub normals: Vec<Vec4>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub vertex_float_channels: HashMap<String, Vec<f32>>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub vertex_selection_channels: HashMap<String, Vec<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_mesh: Option<Box<SetupMesh>>,
}

impl Default for SimulationSetupMesh {
    fn default() -> Self {
        Self {
            positions: Vec::new(),
            triangles: Vec::new(),
            sim_to_render_map: Vec::new(),
            render_to_sim_map: Vec::new(),
            normals: Vec::new(),
            vertex_float_channels: HashMap::new(),
            vertex_selection_channels: HashMap::new(),
            source_mesh: None,
        }
    }
}

/// Extended user setup mesh — identical to SetupMesh.
pub type ExtendedUserSetupMesh = SetupMesh;
