use serde::{Deserialize, Serialize};

use super::mesh::SetupMesh;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum BufferType {
    Display = 0,
    StaticDisplay = 1,
    SimCloth = 2,
    Scratch = 3,
}

impl Default for BufferType {
    fn default() -> Self {
        Self::Display
    }
}

/// Setup representation for a runtime buffer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BufferSetupObject {
    #[serde(default)]
    pub name: String,
    pub buffer_type: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup_mesh: Option<SetupMesh>,
    #[serde(default = "r#true")]
    pub has_normals: bool,
    #[serde(default)]
    pub has_tangents: bool,
    #[serde(default)]
    pub has_bitangents: bool,
    #[serde(default)]
    pub has_triangles: bool,
}

impl Default for BufferSetupObject {
    fn default() -> Self {
        Self {
            name: String::new(),
            buffer_type: BufferType::Display as u8,
            setup_mesh: None,
            has_normals: true,
            has_tangents: false,
            has_bitangents: false,
            has_triangles: false,
        }
    }
}

/// Setup representation for a transform set (skeleton binding).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformSetSetupObject {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub bone_names: Vec<String>,
    #[serde(default)]
    pub skeleton_name: String,
}

impl Default for TransformSetSetupObject {
    fn default() -> Self {
        Self {
            name: String::new(),
            bone_names: Vec::new(),
            skeleton_name: String::new(),
        }
    }
}

fn r#true() -> bool {
    true
}
