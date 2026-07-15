use serde::{Deserialize, Serialize};

use crate::error::{HavokError, HavokResult};

use super::buffer_setup::{BufferSetupObject, TransformSetSetupObject};
use super::operator_setup::OperatorSetupObject;
use super::sim_cloth_setup::SimClothSetupObject;

/// Top-level cloth setup container. Mirrors Python's ClothSetupObject.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClothSetupObject {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub buffer_setups: Vec<BufferSetupObject>,
    #[serde(default)]
    pub transform_set_setups: Vec<TransformSetSetupObject>,
    #[serde(default)]
    pub sim_cloth_setups: Vec<SimClothSetupObject>,
    #[serde(default)]
    pub operator_setups: Vec<OperatorSetupObject>,
    #[serde(default)]
    pub state_setups: Vec<serde_json::Value>,
}

impl Default for ClothSetupObject {
    fn default() -> Self {
        Self {
            name: String::new(),
            buffer_setups: Vec::new(),
            transform_set_setups: Vec::new(),
            sim_cloth_setups: Vec::new(),
            operator_setups: Vec::new(),
            state_setups: Vec::new(),
        }
    }
}

impl ClothSetupObject {
    pub fn to_json(&self) -> HavokResult<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| HavokError::InvalidInput(format!("JSON serialization failed: {e}")))
    }

    pub fn from_json(s: &str) -> HavokResult<Self> {
        serde_json::from_str(s)
            .map_err(|e| HavokError::InvalidInput(format!("JSON deserialization failed: {e}")))
    }
}
