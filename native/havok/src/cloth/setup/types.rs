use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[repr(u8)]
pub enum VertexFloatType {
    Constant = 0,
    Channel = 1,
}

impl Default for VertexFloatType {
    fn default() -> Self {
        Self::Constant
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum VertexSelectionType {
    #[serde(rename = "ALL")]
    All = 0,
    #[serde(rename = "NONE")]
    None = 1,
    #[serde(rename = "CHANNEL")]
    Channel = 2,
    #[serde(rename = "INVERSE_CHANNEL")]
    InverseChannel = 3,
}

impl Default for VertexSelectionType {
    fn default() -> Self {
        Self::All
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum EdgeSelectionType {
    #[serde(rename = "ALL")]
    All = 0,
    #[serde(rename = "NONE")]
    None = 1,
    #[serde(rename = "CHANNEL")]
    Channel = 2,
}

impl Default for EdgeSelectionType {
    fn default() -> Self {
        Self::All
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum TriangleSelectionType {
    #[serde(rename = "ALL")]
    All = 0,
    #[serde(rename = "NONE")]
    None = 1,
    #[serde(rename = "CHANNEL")]
    Channel = 2,
}

impl Default for TriangleSelectionType {
    fn default() -> Self {
        Self::All
    }
}

// ---------------------------------------------------------------------------
// VertexFloatInput — mirrors Python's to_dict / from_dict with integer type
// The Python enum uses IntEnum so values serialize as integers (0/1).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VertexFloatInput {
    /// 0 = CONSTANT, 1 = CHANNEL
    #[serde(rename = "type")]
    pub kind: u8,
    pub constant_value: f32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub channel_name: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub override_scale: bool,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub override_scale_min: f32,
    #[serde(default = "one_f32", skip_serializing_if = "is_one_f32")]
    pub override_scale_max: f32,
}

impl Default for VertexFloatInput {
    fn default() -> Self {
        Self {
            kind: 0,
            constant_value: 0.0,
            channel_name: String::new(),
            override_scale: false,
            override_scale_min: 0.0,
            override_scale_max: 1.0,
        }
    }
}

impl VertexFloatInput {
    pub fn constant(value: f32) -> Self {
        Self {
            kind: 0,
            constant_value: value,
            ..Default::default()
        }
    }

    pub fn channel(name: impl Into<String>) -> Self {
        Self {
            kind: 1,
            constant_value: 0.0,
            channel_name: name.into(),
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// VertexSelectionInput
// The Python enum uses IntEnum so "type" serializes as integer (0..3).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VertexSelectionInput {
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub channel_name: String,
}

impl Default for VertexSelectionInput {
    fn default() -> Self {
        Self {
            kind: 0,
            channel_name: String::new(),
        }
    }
}

impl VertexSelectionInput {
    pub fn all() -> Self {
        Self {
            kind: 0,
            ..Default::default()
        }
    }

    pub fn none() -> Self {
        Self {
            kind: 1,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// EdgeSelectionInput
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeSelectionInput {
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub channel_name: String,
}

impl Default for EdgeSelectionInput {
    fn default() -> Self {
        Self {
            kind: 0,
            channel_name: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// TriangleSelectionInput
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriangleSelectionInput {
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub channel_name: String,
}

impl Default for TriangleSelectionInput {
    fn default() -> Self {
        Self {
            kind: 0,
            channel_name: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_false(b: &bool) -> bool {
    !b
}
fn is_zero_f32(f: &f32) -> bool {
    *f == 0.0
}
fn is_one_f32(f: &f32) -> bool {
    *f == 1.0
}
fn one_f32() -> f32 {
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_float_input_constant_round_trips() {
        let v = VertexFloatInput::constant(0.5);
        let json = serde_json::to_string(&v).unwrap();
        let back: VertexFloatInput = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
        assert_eq!(back.constant_value, 0.5);
    }

    #[test]
    fn vertex_selection_input_default_is_all() {
        let v = VertexSelectionInput::default();
        assert_eq!(v.kind, 0);
    }

    #[test]
    fn vertex_selection_none_has_kind_1() {
        let v = VertexSelectionInput::none();
        assert_eq!(v.kind, 1);
    }
}
