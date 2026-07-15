use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TerrainDiagnostics {
    pub height_rms_error: f32,
    pub height_max_error: f32,
    #[serde(default)]
    pub vhgt_delta_clamp_underflows: u32,
    #[serde(default)]
    pub vhgt_delta_clamp_overflows: u32,
    pub dropped_texture_layers: u32,
    #[serde(default)]
    pub ground_cover_layers: u32,
    #[serde(default)]
    pub no_ground_cover_layers: u32,
    #[serde(default)]
    pub grass_ltex_variants: u32,
    pub required_ltex_form_ids: Vec<String>,
    pub converted_texture_count: u32,
    pub cells: Vec<CellDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellDiagnostic {
    pub x: i32,
    pub y: i32,
    pub rms_error: f32,
    pub max_error: f32,
    #[serde(default)]
    pub vhgt_delta_clamp_underflows: u32,
    #[serde(default)]
    pub vhgt_delta_clamp_overflows: u32,
    pub layers: u32,
}
