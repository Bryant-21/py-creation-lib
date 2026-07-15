use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SupportedGame {
    Oblivion,
    Fo3,
    Fnv,
    Skyrimse,
    Fo4,
    Fo76,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Warning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub ok: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<Warning>,
    pub timings_ms: BTreeMap<String, f64>,
    pub counts: BTreeMap<String, u64>,
    pub data: Value,
}

impl Report {
    pub fn ok(data: Value) -> Self {
        Self {
            ok: true,
            errors: Vec::new(),
            warnings: Vec::new(),
            timings_ms: BTreeMap::new(),
            counts: BTreeMap::new(),
            data,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            errors: vec![message.into()],
            warnings: Vec::new(),
            timings_ms: BTreeMap::new(),
            counts: BTreeMap::new(),
            data: json!({}),
        }
    }

    pub fn with_count(mut self, name: impl Into<String>, value: u64) -> Self {
        self.counts.insert(name.into(), value);
        self
    }

    pub fn with_timing(mut self, name: impl Into<String>, value_ms: f64) -> Self {
        self.timings_ms.insert(name.into(), value_ms);
        self
    }

    pub fn push_warning(&mut self, code: impl Into<String>, message: impl Into<String>) {
        self.warnings.push(Warning {
            code: code.into(),
            message: message.into(),
        });
    }

    pub fn merge(&mut self, other: Report) {
        self.errors.extend(other.errors);
        self.warnings.extend(other.warnings);
        self.timings_ms.extend(other.timings_ms);
        self.counts.extend(other.counts);
        self.ok = self.ok && other.ok;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldSessionConfig {
    pub game: SupportedGame,
    #[serde(default)]
    pub plugin_paths: Vec<String>,
    #[serde(default)]
    pub data_paths: Vec<String>,
    #[serde(default)]
    pub archive_paths: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct WorldSession {
    pub game: SupportedGame,
    pub plugin_paths: Vec<String>,
    pub data_paths: Vec<String>,
    pub archive_paths: Vec<String>,
}

impl WorldSession {
    pub fn from_config(config: WorldSessionConfig) -> Self {
        Self {
            game: config.game,
            plugin_paths: config.plugin_paths,
            data_paths: config.data_paths,
            archive_paths: config.archive_paths,
        }
    }

    pub fn game_id(&self) -> &'static str {
        match self.game {
            SupportedGame::Oblivion => "oblivion",
            SupportedGame::Fo3 => "fo3",
            SupportedGame::Fnv => "fnv",
            SupportedGame::Skyrimse => "skyrimse",
            SupportedGame::Fo4 => "fo4",
            SupportedGame::Fo76 => "fo76",
        }
    }

    pub fn synthetic(game: &str) -> Self {
        let parsed = match game {
            "oblivion" => SupportedGame::Oblivion,
            "fo3" => SupportedGame::Fo3,
            "fnv" => SupportedGame::Fnv,
            "skyrimse" => SupportedGame::Skyrimse,
            "fo76" => SupportedGame::Fo76,
            _ => SupportedGame::Fo4,
        };
        Self {
            game: parsed,
            plugin_paths: Vec::new(),
            data_paths: Vec::new(),
            archive_paths: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum BatchKind {
    Terrain,
    Static,
    Water,
    Marker,
}

impl BatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BatchKind::Terrain => "terrain",
            BatchKind::Static => "static",
            BatchKind::Water => "water",
            BatchKind::Marker => "marker",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CellBounds {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

impl Default for CellBounds {
    fn default() -> Self {
        Self {
            min_x: -1,
            min_y: -1,
            max_x: 1,
            max_y: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderSettings {
    pub include_terrain: bool,
    pub include_statics: bool,
    pub include_static_collections: bool,
    pub include_markers: bool,
    pub include_water: bool,
    pub include_disabled_refs: bool,
    pub include_lights: bool,
    pub include_foliage: bool,
    pub render_mode: String,
    pub debug_buffer: String,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            include_terrain: true,
            include_statics: true,
            include_static_collections: true,
            include_markers: true,
            include_water: true,
            include_disabled_refs: false,
            include_lights: true,
            include_foliage: true,
            render_mode: "lit".to_string(),
            debug_buffer: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CameraQuery {
    pub position: [f32; 3],
    pub target: [f32; 3],
    pub up: [f32; 3],
    pub fov_degrees: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for CameraQuery {
    fn default() -> Self {
        Self {
            position: [0.0, -4096.0, 2048.0],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
            fov_degrees: 60.0,
            near: 1.0,
            far: 250000.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldBatch {
    pub id: String,
    pub kind: BatchKind,
    pub mesh_buffer: String,
    pub instance_buffer: String,
    pub material_id: String,
    pub instance_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pick_buffer: Option<String>,
    #[serde(default)]
    pub debug_buffers: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct WorldInstance {
    pub instance_id: u64,
    pub kind: BatchKind,
    pub disabled: bool,
    pub form_key: String,
    pub base_form_key: String,
    pub signature: String,
    pub source_plugin: String,
    pub cell: [i32; 2],
    pub model_path: String,
    pub position: [f32; 3],
    pub rotation_degrees: [f32; 3],
    pub scale: f32,
    pub layer_form_key: Option<String>,
    pub static_collection_parent: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshPacket {
    pub mesh_id: String,
    pub model_path: String,
    pub vertex_buffer: String,
    pub index_buffer: String,
    pub material_id: String,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialDescriptor {
    pub material_id: String,
    pub shader_model: String,
    pub alpha_mode: String,
    pub diffuse_texture: Option<String>,
    pub normal_texture: Option<String>,
    pub specular_texture: Option<String>,
    pub env_texture: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerrainTile {
    pub tile_id: String,
    pub cell: [i32; 2],
    pub height_buffer: String,
    pub blend_buffer: String,
    pub material_layers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaterSurface {
    pub water_id: String,
    pub cell: [i32; 2],
    pub height: f32,
    pub material_id: String,
    pub color_rgba: [f32; 4],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkerPacket {
    pub marker_id: String,
    pub instance_id: u64,
    pub marker_type: String,
    pub position: [f32; 3],
}

#[derive(Debug, Clone)]
pub struct WorldScene {
    pub worldspace: String,
    pub bounds: CellBounds,
    pub instances: Vec<WorldInstance>,
    pub buffers: BTreeMap<String, Vec<u8>>,
    pub mesh_packets: Vec<MeshPacket>,
    pub materials: Vec<MaterialDescriptor>,
    pub terrain_tiles: Vec<TerrainTile>,
    pub water_surfaces: Vec<WaterSurface>,
    pub markers: Vec<MarkerPacket>,
    pub load_report: Report,
}

impl WorldScene {
    pub fn empty(worldspace: impl Into<String>, bounds: CellBounds) -> Self {
        Self {
            worldspace: worldspace.into(),
            bounds,
            instances: Vec::new(),
            buffers: BTreeMap::new(),
            mesh_packets: Vec::new(),
            materials: Vec::new(),
            terrain_tiles: Vec::new(),
            water_surfaces: Vec::new(),
            markers: Vec::new(),
            load_report: Report::ok(json!({})),
        }
    }

    pub fn push_warning(&mut self, code: impl Into<String>, message: impl Into<String>) {
        self.load_report.push_warning(code, message);
    }

    pub fn merge_report(&mut self, report: Report) {
        self.load_report.merge(report);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfflineRenderJob {
    pub output_path: String,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub camera: Option<CameraQuery>,
    #[serde(default)]
    pub settings: Option<RenderSettings>,
}
