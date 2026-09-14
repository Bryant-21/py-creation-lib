use crate::btd::{BtdFile, BtdHeader, BtdTileCacheStats};
#[cfg(test)]
use crate::btd::{CellTextureSet, QuadrantTextureSet};
use crate::diagnostics::{CellDiagnostic, TerrainDiagnostics};
use crate::global_blend::{
    GlobalLandscapeBlend, LAND_QUADRANT_VERTICES, SourceAlphaLookup,
    collect_required_source_ltex_object_ids_profiled, fo76_layer_alpha_passes,
};
use crate::height_resample::{
    AxisTap, LANCZOS2_REACH, LANCZOS2_TAPS, build_axis_taps, clamp_offset_index, lanczos2_kernel,
};
#[cfg(test)]
use crate::land_encode::{EncodedVhgt, decode_vhgt_heights};
use crate::land_encode::{encode_vhgt, generate_vnml};
use crate::texture_bridge::{ConvertedTerrainGrass, ConvertedTerrainTexture, TextureManifest};
#[cfg(test)]
use crate::texture_layers::{decode_alpha_layers, map_cell_layers};
use rayon::prelude::*;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use thiserror::Error;

const CELL_SOURCE_SAMPLES: usize = 128;
const LAND_CELL_VERTICES: usize = 33;
const LAND_CELL_INTERVALS: usize = 32;
#[cfg(test)]
const CELL_SOURCE_QUADRANT_SAMPLES: usize = 64;
#[cfg(test)]
const MIN_GROUND_COVER_LAYER_COVERAGE: f32 = 0.2;
// FO4 still asks grass LTEX variants for parameters at trace alpha values that
// barely affect the visual terrain blend; require roughly 5/255 average masked
// contribution before selecting the GCVR-backed LTEX.
#[cfg(test)]
const MIN_GROUND_COVER_ALPHA_VISUAL_WEIGHT: f32 = 5.0 / 255.0;
const FO4_CELL_SIZE: f32 = 4096.0;
const VHGT_HEIGHT_STEP: f32 = 8.0;
// FO4 CK rewrites LAND with -128 VHGT deltas as corrupt height data.
const VHGT_MIN_DELTA_STEP: f32 = -127.0;
const VHGT_MAX_DELTA_STEP: f32 = 127.0;
const LAND_FLAG_HAS_VERTEX_NORMALS_HEIGHT_MAP: u32 = 0x01;
const LAND_FLAG_HAS_VERTEX_COLORS: u32 = 0x02;
const LAND_FLAG_HAS_LAYERS: u32 = 0x04;
const LAND_FLAG_UNKNOWN_4: u32 = 0x08;
const LAND_FLAG_AUTO_CALC_NORMALS: u32 = 0x10;
const FO76_VCLR_NEUTRAL_SRGB_BYTE: f32 = 187.67568;
const FO4_DEFAULT_WATER_OBJECT_ID: u32 = 0x0C8633;
const SOURCE_TEXTURE_ALPHA_CACHE_MAX_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum AuthoringEmitError {
    #[error("{0}")]
    Message(String),
    #[error("BTD read failed: {0}")]
    Btd(#[from] crate::btd::BtdError),
    #[error("heightmap export failed: {0}")]
    Heightmap(#[from] crate::heightmap_dds::HeightmapDdsError),
    #[error("file operation failed: {0}")]
    Io(#[from] io::Error),
    #[error("JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConvertOptions {
    pub btd_path: String,
    pub output_authoring_dir: String,
    pub plugin_name: String,
    pub worldspace_editor_id: String,
    pub source_min_x: i32,
    pub source_min_y: i32,
    pub source_max_x: i32,
    pub source_max_y: i32,
    #[serde(
        default = "default_first_form_id",
        deserialize_with = "deserialize_u32_or_default"
    )]
    pub first_form_id: u32,
    #[serde(default, deserialize_with = "deserialize_u32_or_zero")]
    pub world_form_id: u32,
    #[serde(default, deserialize_with = "deserialize_u32_or_zero")]
    pub first_cell_form_id: u32,
    pub resample_mode: String,
    #[serde(default, deserialize_with = "deserialize_string_or_default")]
    pub debug_output_dir: String,
    #[serde(default, deserialize_with = "deserialize_string_or_default")]
    pub texture_manifest_path: String,
    #[serde(default, deserialize_with = "deserialize_string_or_default")]
    pub water_manifest_path: String,
    pub emit_textures: bool,
    #[serde(default)]
    pub export_heightmap: bool,
    /// Diagnostic: emit all LAND VHGT/VNML as one flat plane while leaving
    /// texture-layer records unchanged. This isolates texture blending from
    /// height/normal artifacts in CK/in-game tests.
    #[serde(default)]
    pub debug_flat_land: bool,
    #[serde(default)]
    pub preserve_source_ids: bool,
    #[serde(default)]
    pub reserved_object_ids: Vec<u32>,
    #[serde(default, deserialize_with = "deserialize_string_or_default")]
    pub source_worldspace_authoring_dir: String,
    #[serde(default, deserialize_with = "deserialize_string_or_default")]
    pub source_worldspace_terrain_ids_json: String,
    #[serde(default, deserialize_with = "deserialize_string_or_default")]
    pub heightmap_output_path: String,
    /// When non-empty, emit a `.btd4` dense terrain sidecar at this path
    /// (consumed by the B21_BTD plugin). Empty disables the emit (zero-cost).
    #[serde(default, deserialize_with = "deserialize_string_or_default")]
    pub btd4_output_path: String,
    /// Unused; kept for compatibility. The caller's texture phase owns the
    /// conversion workers.
    #[serde(default)]
    pub conversion_workers: Option<usize>,
    /// Ignored: LAND BTXT/ATXT layers always resolve the plain base LTEX, never the
    /// `{base}_GC_{gcvr}` composite. Ground cover reaches the native scatter
    /// through the `.btd4` GCVR channel. Kept for compatibility.
    #[serde(default)]
    pub land_skip_ground_cover_variants: bool,
    /// Unused; kept for compatibility (terrain planning does not encode DDS files).
    #[serde(default)]
    pub reuse_existing_textures: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConvertReport {
    pub status: String,
    pub cells_written: u32,
    pub worldspace_editor_id: String,
    pub authoring_dir: String,
    pub diagnostics_path: String,
    pub required_ltex_form_ids: Vec<String>,
    pub height_rms_error: f32,
    pub height_max_error: f32,
    pub vhgt_delta_clamp_underflows: u32,
    pub vhgt_delta_clamp_overflows: u32,
    pub dropped_texture_layers: u32,
    #[serde(default)]
    pub ground_cover_layers: u32,
    #[serde(default)]
    pub no_ground_cover_layers: u32,
    #[serde(default)]
    pub grass_ltex_variants: u32,
    pub converted_texture_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub btd4_output_path: Option<String>,
    /// Layers referenced in the `.btd4` LAYR table beyond the ones the LAND's
    /// per-quadrant texture stack kept (i.e. the dense sidecar's extra coverage).
    #[serde(default)]
    pub layers_recovered: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heightmap_output_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heightmap_preview_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heightmap_stats_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heightmap_cell_0_0_output_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heightmap_cell_0_0_preview_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heightmap_cell_0_0_stats_path: Option<String>,
    /// Starfield only: count of FO4 quadrants whose BTXT-base area vote had a
    /// runner-up candidate within 10% of the winner (0 for FO76 identity).
    #[serde(default)]
    pub quadrant_base_split: u32,
    #[serde(default)]
    pub operation_counts: BTreeMap<String, u64>,
    pub timings: Vec<TimingEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimingEntry {
    pub name: String,
    pub elapsed_seconds: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthoringRecordPayload {
    pub signature: String,
    pub relative_path: String,
    pub yaml: String,
}

#[derive(Debug, Clone)]
pub struct AuthoringRecordValuePayload {
    pub signature: String,
    pub relative_path: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct CollectedAuthoringOutput {
    pub plugin_yaml: String,
    pub records: Vec<AuthoringRecordPayload>,
}

#[derive(Debug, Clone, Copy)]
pub enum TerrainRecordOutput {
    WriteAuthoringFiles,
    CollectOnly,
    ReportOnly,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConvertOutput {
    pub report: ConvertReport,
    pub authoring: CollectedAuthoringOutput,
}

struct AuthoringOutput<'a> {
    output_dir: PathBuf,
    write_files: bool,
    collect_records: bool,
    record_sink:
        Option<&'a mut dyn FnMut(AuthoringRecordPayload) -> Result<(), AuthoringEmitError>>,
    record_value_sink:
        Option<&'a mut dyn FnMut(AuthoringRecordValuePayload) -> Result<(), AuthoringEmitError>>,
    plugin_yaml: String,
    records: Vec<AuthoringRecordPayload>,
    record_sink_elapsed: Duration,
    record_sink_payload_count: u64,
    record_sink_yaml_bytes: u64,
    structured_cell_payload_count: u64,
}

impl<'a> AuthoringOutput<'a> {
    fn write_files(output_dir: PathBuf) -> Self {
        Self {
            output_dir,
            write_files: true,
            collect_records: true,
            record_sink: None,
            record_value_sink: None,
            plugin_yaml: String::new(),
            records: Vec::new(),
            record_sink_elapsed: Duration::ZERO,
            record_sink_payload_count: 0,
            record_sink_yaml_bytes: 0,
            structured_cell_payload_count: 0,
        }
    }

    fn collect_only(output_dir: PathBuf) -> Self {
        Self {
            output_dir,
            write_files: false,
            collect_records: true,
            record_sink: None,
            record_value_sink: None,
            plugin_yaml: String::new(),
            records: Vec::new(),
            record_sink_elapsed: Duration::ZERO,
            record_sink_payload_count: 0,
            record_sink_yaml_bytes: 0,
            structured_cell_payload_count: 0,
        }
    }

    fn stream_records(
        output_dir: PathBuf,
        record_sink: &'a mut dyn FnMut(AuthoringRecordPayload) -> Result<(), AuthoringEmitError>,
    ) -> Self {
        Self {
            output_dir,
            write_files: false,
            collect_records: false,
            record_sink: Some(record_sink),
            record_value_sink: None,
            plugin_yaml: String::new(),
            records: Vec::new(),
            record_sink_elapsed: Duration::ZERO,
            record_sink_payload_count: 0,
            record_sink_yaml_bytes: 0,
            structured_cell_payload_count: 0,
        }
    }

    fn stream_records_with_structured_cells(
        output_dir: PathBuf,
        record_sink: &'a mut dyn FnMut(AuthoringRecordPayload) -> Result<(), AuthoringEmitError>,
        record_value_sink: &'a mut dyn FnMut(
            AuthoringRecordValuePayload,
        ) -> Result<(), AuthoringEmitError>,
    ) -> Self {
        Self {
            output_dir,
            write_files: false,
            collect_records: false,
            record_sink: Some(record_sink),
            record_value_sink: Some(record_value_sink),
            plugin_yaml: String::new(),
            records: Vec::new(),
            record_sink_elapsed: Duration::ZERO,
            record_sink_payload_count: 0,
            record_sink_yaml_bytes: 0,
            structured_cell_payload_count: 0,
        }
    }

    fn report_only(output_dir: PathBuf) -> Self {
        Self {
            output_dir,
            write_files: false,
            collect_records: false,
            record_sink: None,
            record_value_sink: None,
            plugin_yaml: String::new(),
            records: Vec::new(),
            record_sink_elapsed: Duration::ZERO,
            record_sink_payload_count: 0,
            record_sink_yaml_bytes: 0,
            structured_cell_payload_count: 0,
        }
    }

    fn write_plugin_yaml(&mut self, yaml: String) -> Result<(), AuthoringEmitError> {
        if self.write_files {
            fs::create_dir_all(&self.output_dir)?;
            fs::write(self.output_dir.join("plugin.yaml"), &yaml)?;
        }
        if self.collect_records {
            self.plugin_yaml = yaml;
        }
        Ok(())
    }

    fn write_record_yaml(
        &mut self,
        signature: &str,
        relative_path: PathBuf,
        yaml: String,
    ) -> Result<(), AuthoringEmitError> {
        if self.write_files {
            let path = self.output_dir.join(&relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, &yaml)?;
        }
        if self.collect_records || self.record_sink.is_some() {
            let payload = AuthoringRecordPayload {
                signature: signature.to_owned(),
                relative_path: relative_path.display().to_string().replace('\\', "/"),
                yaml,
            };
            if self.collect_records {
                self.records.push(payload.clone());
            }
            if let Some(record_sink) = self.record_sink.as_mut() {
                self.record_sink_payload_count = self.record_sink_payload_count.saturating_add(1);
                self.record_sink_yaml_bytes = self
                    .record_sink_yaml_bytes
                    .saturating_add(payload.yaml.len() as u64);
                let sink_started = Instant::now();
                let result = record_sink(payload);
                self.record_sink_elapsed += sink_started.elapsed();
                result?;
            }
        }
        Ok(())
    }

    fn write_record_value(
        &mut self,
        signature: &str,
        relative_path: PathBuf,
        value: serde_json::Value,
    ) -> Result<(), AuthoringEmitError> {
        let Some(record_value_sink) = self.record_value_sink.as_mut() else {
            return Err(AuthoringEmitError::Message(
                "structured terrain record sink is not configured".to_owned(),
            ));
        };
        self.record_sink_payload_count = self.record_sink_payload_count.saturating_add(1);
        self.structured_cell_payload_count = self.structured_cell_payload_count.saturating_add(1);
        let sink_started = Instant::now();
        let result = record_value_sink(AuthoringRecordValuePayload {
            signature: signature.to_owned(),
            relative_path: relative_path.display().to_string().replace('\\', "/"),
            value,
        });
        self.record_sink_elapsed += sink_started.elapsed();
        result
    }

    fn uses_structured_cell_sink(&self) -> bool {
        self.record_value_sink.is_some()
    }

    fn record_sink_profile(&self) -> (Duration, u64, u64, u64) {
        (
            self.record_sink_elapsed,
            self.record_sink_payload_count,
            self.record_sink_yaml_bytes,
            self.structured_cell_payload_count,
        )
    }

    fn finish(self) -> CollectedAuthoringOutput {
        CollectedAuthoringOutput {
            plugin_yaml: self.plugin_yaml,
            records: self.records,
        }
    }
}

#[derive(Debug, Default)]
struct PreservedTerrainIds {
    world_form_id: Option<u32>,
    world_editor_id: Option<String>,
    cells: HashMap<(i32, i32), PreservedCellIds>,
    used_object_ids: HashSet<u32>,
}

#[derive(Debug, Deserialize)]
struct SourceWorldspaceTerrainIdsPayload {
    world_form_id: Option<u32>,
    world_editor_id: Option<String>,
    #[serde(default)]
    cells: Vec<SourceWorldspaceTerrainCellIds>,
}

#[derive(Debug, Deserialize)]
struct SourceWorldspaceTerrainCellIds {
    x: i32,
    y: i32,
    cell_form_id: u32,
    #[serde(default)]
    cell_editor_id: String,
    land_form_id: Option<u32>,
}

#[derive(Debug, Default, Clone)]
struct PreservedCellIds {
    cell_form_id: Option<u32>,
    cell_editor_id: Option<String>,
    land_form_id: Option<u32>,
}

#[derive(Debug)]
struct TerrainIdPlan {
    world_form_id: u32,
    world_editor_id: Option<String>,
    preserved_cells: HashMap<(i32, i32), PreservedCellIds>,
    source_backed: bool,
    next_object_id_after_terrain: u32,
    used_object_ids: HashSet<u32>,
}

impl TerrainIdPlan {
    fn world_editor_id(&self, options: &ConvertOptions) -> String {
        self.world_editor_id
            .clone()
            .unwrap_or_else(|| options.worldspace_editor_id.clone())
    }

    fn cell_form_id(&self, options: &ConvertOptions, cell_x: i32, cell_y: i32, index: u32) -> u32 {
        self.preserved_cells
            .get(&(cell_x, cell_y))
            .and_then(|cell| cell.cell_form_id)
            .unwrap_or_else(|| cell_form_id(options, index))
    }

    fn cell_editor_id(&self, options: &ConvertOptions, cell_x: i32, cell_y: i32) -> Option<String> {
        if let Some(editor_id) = self
            .preserved_cells
            .get(&(cell_x, cell_y))
            .and_then(|cell| cell.cell_editor_id.clone())
        {
            return Some(editor_id);
        }
        if self.source_backed {
            return None;
        }
        let world_editor_id = self.world_editor_id(options);
        Some(cell_editor_id(&world_editor_id, cell_x, cell_y))
    }

    fn land_form_id(&self, options: &ConvertOptions, cell_x: i32, cell_y: i32, index: u32) -> u32 {
        self.preserved_cells
            .get(&(cell_x, cell_y))
            .and_then(|cell| cell.land_form_id)
            .unwrap_or_else(|| land_form_id(options, index))
    }
}

#[derive(Debug, Serialize)]
struct DiagnosticsReport {
    diagnostics: TerrainDiagnostics,
    btd_path: String,
    source_min_x: i32,
    source_min_y: i32,
    source_max_x: i32,
    source_max_y: i32,
    cells_written: u32,
    resample_mode: String,
    emit_textures: bool,
    texture_manifest_path: String,
    water_manifest_path: String,
    export_heightmap: bool,
    heightmap_output_path: Option<String>,
    heightmap_preview_path: Option<String>,
    heightmap_stats_path: Option<String>,
    heightmap_cell_0_0_output_path: Option<String>,
    heightmap_cell_0_0_preview_path: Option<String>,
    heightmap_cell_0_0_stats_path: Option<String>,
    operation_counts: BTreeMap<String, u64>,
    timings: Vec<TimingEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct RequiredTextureUsage {
    pub ltex_form_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ground_cover_form_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RequiredTextureUsageProfile {
    pub usages: Vec<RequiredTextureUsage>,
    pub timings: Vec<TimingEntry>,
    pub operation_counts: BTreeMap<String, u64>,
}

pub struct PreparedTerrainSource {
    btd: BtdFile,
    btd_path: String,
    source_min_x: i32,
    source_min_y: i32,
    source_max_x: i32,
    source_max_y: i32,
}

pub struct PreparedTerrainTextureScan {
    pub profile: RequiredTextureUsageProfile,
    pub source: PreparedTerrainSource,
}

#[derive(Debug, Clone)]
struct EmittedTexture {
    converted: ConvertedTerrainTexture,
    txst_object_id: u32,
    ltex_object_id: u32,
    grass_object_ids: Vec<u32>,
}

struct LandTextureFields {
    fields: Vec<LandTextureField>,
    layer_count: u32,
    ground_cover_layer_count: u32,
    no_ground_cover_layer_count: u32,
    // Ordered as four repetitions of [base, alpha0..alpha4]. Empty material
    // slots remain None so the v2 runtime can validate the exact live LAND stack.
    btd4_layer_object_ids: Vec<Option<u32>>,
    btd4_source_layer_object_ids: Vec<Option<u32>>,
}

enum LandTextureField {
    Layer {
        signature: &'static str,
        texture_object_id: u32,
        plugin_name: String,
        quadrant: u8,
        layer: i16,
    },
    AlphaLayerData {
        raw_hex: String,
    },
}

impl LandTextureField {
    fn yaml(&self) -> String {
        match self {
            Self::Layer {
                signature,
                texture_object_id,
                plugin_name,
                quadrant,
                layer,
            } => land_texture_layer_field(
                signature,
                *texture_object_id,
                plugin_name,
                *quadrant,
                *layer,
            ),
            Self::AlphaLayerData { raw_hex } => {
                format!("    - AlphaLayerData:\n        raw_hex: \"{raw_hex}\"\n")
            }
        }
    }

    fn value(&self) -> serde_json::Value {
        match self {
            Self::Layer {
                signature,
                texture_object_id,
                plugin_name,
                quadrant,
                layer,
            } => {
                let unknown_byte_3 = if *signature == "BTXT" { 2 } else { 0 };
                let mut field = serde_json::Map::new();
                field.insert(
                    (*signature).to_owned(),
                    serde_json::json!({
                        "Texture": {
                            "reference": {
                                "plugin": plugin_name,
                                "object_id": form_id_hex(*texture_object_id),
                            }
                        },
                        "Quadrant": quadrant,
                        "UnknownByte3": unknown_byte_3,
                        "Layer": layer,
                    }),
                );
                serde_json::Value::Object(field)
            }
            Self::AlphaLayerData { raw_hex } => serde_json::json!({
                "AlphaLayerData": { "raw_hex": raw_hex }
            }),
        }
    }
}

#[derive(Default)]
struct TextureAlphaMasks {
    by_source_ltex_object_id: HashMap<u32, TextureAlphaMask>,
}

#[derive(Clone)]
struct TextureAlphaMask {
    width: usize,
    height: usize,
    alpha: Arc<[u8]>,
}

impl TextureAlphaMask {
    fn sample(&self, u: i32, v: i32) -> u8 {
        let x = u.rem_euclid(self.width as i32) as usize;
        let y = v.rem_euclid(self.height as i32) as usize;
        self.alpha[y * self.width + x]
    }
}

impl SourceAlphaLookup for TextureAlphaMasks {
    fn sample_alpha(&self, source_ltex_object_id: u32, u: i32, v: i32) -> u8 {
        self.by_source_ltex_object_id
            .get(&source_ltex_object_id)
            .map(|mask| mask.sample(u, v))
            .unwrap_or(u8::MAX)
    }
}

struct CachedTextureAlphaMask {
    file_len: u64,
    modified: Option<SystemTime>,
    last_used: u64,
    mask: TextureAlphaMask,
}

#[derive(Default)]
struct TextureAlphaMaskCache {
    entries: HashMap<PathBuf, CachedTextureAlphaMask>,
    bytes: usize,
    tick: u64,
}

impl TextureAlphaMaskCache {
    fn get(
        &mut self,
        path: &Path,
        file_len: u64,
        modified: Option<SystemTime>,
    ) -> Option<TextureAlphaMask> {
        self.tick = self.tick.wrapping_add(1);
        let entry = self.entries.get_mut(path)?;
        if entry.file_len != file_len || entry.modified != modified {
            return None;
        }
        entry.last_used = self.tick;
        Some(entry.mask.clone())
    }

    fn insert(
        &mut self,
        path: PathBuf,
        file_len: u64,
        modified: Option<SystemTime>,
        mask: TextureAlphaMask,
    ) {
        let mask_bytes = mask.alpha.len();
        if mask_bytes > SOURCE_TEXTURE_ALPHA_CACHE_MAX_BYTES {
            return;
        }
        if let Some(previous) = self.entries.remove(&path) {
            self.bytes = self.bytes.saturating_sub(previous.mask.alpha.len());
        }
        while self.bytes + mask_bytes > SOURCE_TEXTURE_ALPHA_CACHE_MAX_BYTES {
            let Some(oldest_path) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(path, _)| path.clone())
            else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest_path) {
                self.bytes = self.bytes.saturating_sub(removed.mask.alpha.len());
            }
        }
        self.tick = self.tick.wrapping_add(1);
        self.bytes += mask_bytes;
        self.entries.insert(
            path,
            CachedTextureAlphaMask {
                file_len,
                modified,
                last_used: self.tick,
                mask,
            },
        );
    }
}

static SOURCE_TEXTURE_ALPHA_CACHE: OnceLock<Mutex<TextureAlphaMaskCache>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize)]
struct WaterManifest {
    #[serde(default = "default_water_object_id")]
    default_water_object_id: u32,
    #[serde(default)]
    cells: Vec<WaterManifestCell>,
}

#[derive(Debug, Clone, Deserialize)]
struct WaterManifestCell {
    x: i32,
    y: i32,
    height: f32,
    #[serde(default)]
    water_object_id: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
struct WaterCell {
    height: f32,
    water_object_id: u32,
}

#[derive(Debug, Clone, Copy)]
enum ResampleMode {
    Sample4,
    Weighted,
    Feature,
    Lanczos,
}

#[derive(Debug, Clone)]
struct TargetHeightGrid {
    width: usize,
    height: usize,
    values: Vec<f32>,
}

impl TargetHeightGrid {
    fn get(&self, x: usize, y: usize) -> f32 {
        self.values[y * self.width + x]
    }
}

#[derive(Debug, Clone)]
struct HeightmapExportPaths {
    output_path: String,
    preview_path: String,
    stats_path: String,
}

#[derive(Debug, Clone, Copy, Default)]
struct VhgtDeltaClampStats {
    underflows: u32,
    overflows: u32,
}

impl VhgtDeltaClampStats {
    fn add(self, other: Self) -> Self {
        Self {
            underflows: self.underflows.saturating_add(other.underflows),
            overflows: self.overflows.saturating_add(other.overflows),
        }
    }
}

/// World frame of a BTD's source data, chosen once per header. FO76 identity:
/// BTD == FO4 world, half-cell shifted. Starfield: 100m SF cells, converted via
/// `sf_frame`. Emitters branch on this instead of sharing addressing math.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceFrame {
    Fo76Identity,
    Starfield,
}

impl SourceFrame {
    fn for_header(h: &BtdHeader) -> Self {
        if h.is_starfield_layout {
            Self::Starfield
        } else {
            Self::Fo76Identity
        }
    }
}

struct SourceCellCache {
    cells: HashMap<(i32, i32), Vec<f32>>,
    // Parallel cache of RAW u16 source samples (never dequantized). HGTS for the
    // .btd4 sidecar must round-trip losslessly, so it reads these instead of
    // inverting the f32 `cells` (which would reintroduce quantization error).
    raw_cells: HashMap<(i32, i32), Vec<u16>>,
    height_min: f32,
    height_scale: f32,
    source_min_x: i32,
    source_min_y: i32,
    source_width: usize,
    source_height: usize,
    frame: SourceFrame,
    // Starfield only: the BTD's own SF cell-min anchor (`source_min_x/y` under
    // Starfield mirrors this — kept as a separate, explicitly-named field so
    // call sites never have to guess which "min" a given value is), and the
    // FO4 window's first cell, needed to re-derive `sf_frame` positions for
    // resample modes that don't consume the precomputed `axis_taps_*`.
    btd_cell_min_x: i32,
    btd_cell_min_y: i32,
    target_cell_min_x: i32,
    target_cell_min_y: i32,
    axis_taps_x: Option<Vec<AxisTap>>,
    axis_taps_y: Option<Vec<AxisTap>>,
}

impl SourceCellCache {
    fn new(
        header: &BtdHeader,
        options: &ConvertOptions,
        cells_x: usize,
        cells_y: usize,
        frame: SourceFrame,
    ) -> Result<Self, AuthoringEmitError> {
        match frame {
            SourceFrame::Fo76Identity => Self::new_fo76_identity(header, options, cells_x, cells_y),
            SourceFrame::Starfield => Self::new_starfield(header, options, cells_x, cells_y),
        }
    }

    fn new_fo76_identity(
        header: &BtdHeader,
        options: &ConvertOptions,
        cells_x: usize,
        cells_y: usize,
    ) -> Result<Self, AuthoringEmitError> {
        // One extra source cell per axis when the BTD extends past the requested
        // range — the +HALF_CELL_SAMPLES shift reads into it. (When already at the
        // BTD max edge, the sample() clamp handles the final half cell.)
        let extra_x = usize::from(options.source_max_x < header.cell_max_x);
        let extra_y = usize::from(options.source_max_y < header.cell_max_y);
        let source_width = cells_x
            .checked_add(extra_x)
            .and_then(|c| c.checked_mul(CELL_SOURCE_SAMPLES))
            .ok_or_else(|| AuthoringEmitError::Message("source grid width overflow".to_string()))?;
        let source_height = cells_y
            .checked_add(extra_y)
            .and_then(|c| c.checked_mul(CELL_SOURCE_SAMPLES))
            .ok_or_else(|| {
                AuthoringEmitError::Message("source grid height overflow".to_string())
            })?;
        Ok(Self {
            cells: HashMap::new(),
            raw_cells: HashMap::new(),
            height_min: header.world_height_min,
            height_scale: (header.world_height_max - header.world_height_min) / u16::MAX as f32,
            source_min_x: options.source_min_x,
            source_min_y: options.source_min_y,
            source_width,
            source_height,
            frame: SourceFrame::Fo76Identity,
            btd_cell_min_x: header.cell_min_x,
            btd_cell_min_y: header.cell_min_y,
            target_cell_min_x: options.source_min_x,
            target_cell_min_y: options.source_min_y,
            axis_taps_x: None,
            axis_taps_y: None,
        })
    }

    fn new_starfield(
        header: &BtdHeader,
        options: &ConvertOptions,
        cells_x: usize,
        cells_y: usize,
    ) -> Result<Self, AuthoringEmitError> {
        // Bounds against the BTD's real SF-cell extent (not the FO4 window,
        // which has no fixed relationship to the SF cell count); every access
        // clamps into this range, so sizing to the full file is simply safe.
        let source_width = header
            .cells_x
            .checked_mul(CELL_SOURCE_SAMPLES)
            .ok_or_else(|| AuthoringEmitError::Message("source grid width overflow".to_string()))?;
        let source_height = header
            .cells_y
            .checked_mul(CELL_SOURCE_SAMPLES)
            .ok_or_else(|| {
                AuthoringEmitError::Message("source grid height overflow".to_string())
            })?;
        let vertex_count_x = cells_x
            .checked_mul(LAND_CELL_INTERVALS)
            .and_then(|v| v.checked_add(1))
            .ok_or_else(|| AuthoringEmitError::Message("target grid width overflow".to_string()))?;
        let vertex_count_y = cells_y
            .checked_mul(LAND_CELL_INTERVALS)
            .and_then(|v| v.checked_add(1))
            .ok_or_else(|| {
                AuthoringEmitError::Message("target grid height overflow".to_string())
            })?;
        let axis_taps_x = build_axis_taps(
            vertex_count_x,
            options.source_min_x,
            header.cell_min_x,
            crate::sf_frame::SF_SAMPLES_PER_FO4_INTERVAL,
        );
        let axis_taps_y = build_axis_taps(
            vertex_count_y,
            options.source_min_y,
            header.cell_min_y,
            crate::sf_frame::SF_SAMPLES_PER_FO4_INTERVAL,
        );
        Ok(Self {
            cells: HashMap::new(),
            raw_cells: HashMap::new(),
            // Starfield header floats are meters; LAND's VHGT lattice is FO4
            // units. This is the ONLY Z-scale site — cell_values() below,
            // encode_vhgt, generate_vnml, and WHGT all consume the result and
            // come out in FO4 units for free.
            height_min: crate::sf_frame::meters_to_fo4_units(header.world_height_min as f64) as f32,
            height_scale: crate::sf_frame::meters_to_fo4_units(
                ((header.world_height_max - header.world_height_min) / u16::MAX as f32) as f64,
            ) as f32,
            source_min_x: header.cell_min_x,
            source_min_y: header.cell_min_y,
            source_width,
            source_height,
            frame: SourceFrame::Starfield,
            btd_cell_min_x: header.cell_min_x,
            btd_cell_min_y: header.cell_min_y,
            target_cell_min_x: options.source_min_x,
            target_cell_min_y: options.source_min_y,
            axis_taps_x: Some(axis_taps_x.taps),
            axis_taps_y: Some(axis_taps_y.taps),
        })
    }

    /// Fractional global BTD sample position (relative to `btd_cell_min_x`,
    /// matching `sample()`'s Starfield addressing) for FO4 target vertex
    /// `target_x` in the emitted window. Shared by the non-Lanczos resample
    /// modes, which need a fractional center rather than the precomputed
    /// per-vertex `axis_taps_x`.
    fn starfield_center_x(&self, target_x: usize) -> f64 {
        let units = crate::sf_frame::fo4_land_vertex_units(self.target_cell_min_x, target_x);
        crate::sf_frame::fo4_units_to_btd_sample(units, self.btd_cell_min_x)
    }

    fn starfield_center_y(&self, target_y: usize) -> f64 {
        let units = crate::sf_frame::fo4_land_vertex_units(self.target_cell_min_y, target_y);
        crate::sf_frame::fo4_units_to_btd_sample(units, self.btd_cell_min_y)
    }

    fn retain_neighbor_rows(&mut self, cell_y: i32) {
        let min_y = cell_y.saturating_sub(1);
        let max_y = cell_y.saturating_add(1);
        self.cells
            .retain(|(_, cached_y), _| *cached_y >= min_y && *cached_y <= max_y);
        self.raw_cells
            .retain(|(_, cached_y), _| *cached_y >= min_y && *cached_y <= max_y);
    }

    /// Clamp a caller's source index into bounds. FO76 identity: callers pass the
    /// unshifted index and this adds the +HALF_CELL_SAMPLES layout shift.
    /// Starfield: callers pass the `sf_frame` global sample index (relative to
    /// `btd_cell_min_x/y`), which must not get the FO76 shift.
    fn resolve_sample_index(&self, source_x: usize, source_y: usize) -> (usize, usize) {
        match self.frame {
            SourceFrame::Fo76Identity => (
                (source_x + crate::fo4_frame::HALF_CELL_SAMPLES).min(self.source_width - 1),
                (source_y + crate::fo4_frame::HALF_CELL_SAMPLES).min(self.source_height - 1),
            ),
            SourceFrame::Starfield => (
                source_x.min(self.source_width - 1),
                source_y.min(self.source_height - 1),
            ),
        }
    }

    fn sample(
        &mut self,
        btd: &mut BtdFile,
        source_x: usize,
        source_y: usize,
    ) -> Result<f32, AuthoringEmitError> {
        let (source_x, source_y) = self.resolve_sample_index(source_x, source_y);
        let cell_offset_x = source_x / CELL_SOURCE_SAMPLES;
        let cell_offset_y = source_y / CELL_SOURCE_SAMPLES;
        let local_x = source_x % CELL_SOURCE_SAMPLES;
        let local_y = source_y % CELL_SOURCE_SAMPLES;
        let cell_x = self
            .source_min_x
            .checked_add(usize_to_i32(cell_offset_x)?)
            .ok_or_else(|| {
                AuthoringEmitError::Message("source cell x coordinate overflow".to_string())
            })?;
        let cell_y = self
            .source_min_y
            .checked_add(usize_to_i32(cell_offset_y)?)
            .ok_or_else(|| {
                AuthoringEmitError::Message("source cell y coordinate overflow".to_string())
            })?;
        let values = self.cell_values(btd, cell_x, cell_y)?;
        Ok(values[local_y * CELL_SOURCE_SAMPLES + local_x])
    }

    fn cell_values(
        &mut self,
        btd: &mut BtdFile,
        cell_x: i32,
        cell_y: i32,
    ) -> Result<&Vec<f32>, AuthoringEmitError> {
        if !self.cells.contains_key(&(cell_x, cell_y)) {
            let samples = btd.cell_height_map_u16(cell_x, cell_y, 0)?;
            if samples.len() != CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES {
                return Err(AuthoringEmitError::Message(format!(
                    "cell ({cell_x}, {cell_y}) returned {} height samples",
                    samples.len()
                )));
            }
            let values = samples
                .into_iter()
                .map(|sample| self.height_min + sample as f32 * self.height_scale)
                .collect();
            self.cells.insert((cell_x, cell_y), values);
        }
        Ok(self.cells.get(&(cell_x, cell_y)).unwrap())
    }

    /// RAW-u16 twin of `sample` (identical `resolve_sample_index` addressing
    /// and cell/local arithmetic). Returns the undequantized source sample so
    /// the .btd4 HGTS aligns byte-for-byte with the f32 LAND heights' source.
    fn sample_raw_u16(
        &mut self,
        btd: &mut BtdFile,
        source_x: usize,
        source_y: usize,
    ) -> Result<u16, AuthoringEmitError> {
        let (source_x, source_y) = self.resolve_sample_index(source_x, source_y);
        let cell_offset_x = source_x / CELL_SOURCE_SAMPLES;
        let cell_offset_y = source_y / CELL_SOURCE_SAMPLES;
        let local_x = source_x % CELL_SOURCE_SAMPLES;
        let local_y = source_y % CELL_SOURCE_SAMPLES;
        let cell_x = self
            .source_min_x
            .checked_add(usize_to_i32(cell_offset_x)?)
            .ok_or_else(|| {
                AuthoringEmitError::Message("source cell x coordinate overflow".to_string())
            })?;
        let cell_y = self
            .source_min_y
            .checked_add(usize_to_i32(cell_offset_y)?)
            .ok_or_else(|| {
                AuthoringEmitError::Message("source cell y coordinate overflow".to_string())
            })?;
        let values = self.cell_values_raw(btd, cell_x, cell_y)?;
        Ok(values[local_y * CELL_SOURCE_SAMPLES + local_x])
    }

    fn cell_values_raw(
        &mut self,
        btd: &mut BtdFile,
        cell_x: i32,
        cell_y: i32,
    ) -> Result<&Vec<u16>, AuthoringEmitError> {
        if !self.raw_cells.contains_key(&(cell_x, cell_y)) {
            let samples = btd.cell_height_map_u16(cell_x, cell_y, 0)?;
            if samples.len() != CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES {
                return Err(AuthoringEmitError::Message(format!(
                    "cell ({cell_x}, {cell_y}) returned {} height samples",
                    samples.len()
                )));
            }
            self.raw_cells.insert((cell_x, cell_y), samples);
        }
        Ok(self.raw_cells.get(&(cell_x, cell_y)).unwrap())
    }
}

pub fn convert_btd_to_authoring(options: ConvertOptions) -> Result<String, AuthoringEmitError> {
    let output = convert_btd(options, TerrainRecordOutput::WriteAuthoringFiles)?;
    Ok(serde_json::to_string(&output.report)?)
}

pub fn collect_required_ltex_form_ids_for_options(
    mut options: ConvertOptions,
) -> Result<Vec<String>, AuthoringEmitError> {
    let mut btd = BtdFile::open(&options.btd_path)?;
    resolve_full_extent_sentinel(&mut options, btd.header());
    validate_range(&options)?;
    validate_btd_bounds(&btd, &options)?;
    collect_required_ltex_form_ids(&mut btd, &options)
}

pub fn collect_required_texture_usages_for_options(
    mut options: ConvertOptions,
) -> Result<Vec<RequiredTextureUsage>, AuthoringEmitError> {
    let mut btd = BtdFile::open(&options.btd_path)?;
    resolve_full_extent_sentinel(&mut options, btd.header());
    validate_range(&options)?;
    validate_btd_bounds(&btd, &options)?;
    collect_required_texture_usages(&mut btd, &options)
}

pub fn collect_required_texture_usages_lightweight_for_options(
    options: ConvertOptions,
) -> Result<Vec<RequiredTextureUsage>, AuthoringEmitError> {
    collect_required_texture_usages_lightweight_profiled_for_options(options)
        .map(|profile| profile.usages)
}

pub fn collect_required_texture_usages_lightweight_profiled_for_options(
    options: ConvertOptions,
) -> Result<RequiredTextureUsageProfile, AuthoringEmitError> {
    prepare_terrain_texture_scan(options).map(|prepared| prepared.profile)
}

pub fn prepare_terrain_texture_scan(
    mut options: ConvertOptions,
) -> Result<PreparedTerrainTextureScan, AuthoringEmitError> {
    let mut btd = BtdFile::open(&options.btd_path)?;
    resolve_full_extent_sentinel(&mut options, btd.header());
    validate_range(&options)?;
    validate_btd_bounds(&btd, &options)?;
    let cache_before = btd.tile_cache_stats();
    let mut profile = collect_required_texture_usages_lightweight_profiled(&mut btd, &options)?;
    append_btd_cache_counts(
        &mut profile.operation_counts,
        "btd_cache",
        cache_before,
        btd.tile_cache_stats(),
    );
    Ok(PreparedTerrainTextureScan {
        profile,
        source: PreparedTerrainSource {
            btd,
            btd_path: options.btd_path,
            source_min_x: options.source_min_x,
            source_min_y: options.source_min_y,
            source_max_x: options.source_max_x,
            source_max_y: options.source_max_y,
        },
    })
}

pub fn convert_btd_with_record_sink<F>(
    options: ConvertOptions,
    record_sink: &mut F,
) -> Result<ConvertReport, AuthoringEmitError>
where
    F: FnMut(AuthoringRecordPayload) -> Result<(), AuthoringEmitError>,
{
    let sink: &mut dyn FnMut(AuthoringRecordPayload) -> Result<(), AuthoringEmitError> =
        record_sink;
    let output = convert_btd_inner(
        options,
        TerrainRecordOutput::CollectOnly,
        Some(sink),
        None,
        None,
    )?;
    Ok(output.report)
}

pub fn convert_prepared_btd_with_record_sink<F>(
    prepared: PreparedTerrainSource,
    options: ConvertOptions,
    record_sink: &mut F,
) -> Result<ConvertReport, AuthoringEmitError>
where
    F: FnMut(AuthoringRecordPayload) -> Result<(), AuthoringEmitError>,
{
    let sink: &mut dyn FnMut(AuthoringRecordPayload) -> Result<(), AuthoringEmitError> =
        record_sink;
    let output = convert_btd_inner(
        options,
        TerrainRecordOutput::CollectOnly,
        Some(sink),
        None,
        Some(prepared),
    )?;
    Ok(output.report)
}

pub fn convert_btd(
    options: ConvertOptions,
    output: TerrainRecordOutput,
) -> Result<ConvertOutput, AuthoringEmitError> {
    convert_btd_inner(options, output, None, None, None)
}

pub fn convert_prepared_btd(
    prepared: PreparedTerrainSource,
    options: ConvertOptions,
    output: TerrainRecordOutput,
) -> Result<ConvertOutput, AuthoringEmitError> {
    convert_btd_inner(options, output, None, None, Some(prepared))
}

pub fn convert_prepared_btd_with_structured_cell_sink<F, G>(
    prepared: PreparedTerrainSource,
    options: ConvertOptions,
    record_sink: &mut F,
    structured_cell_sink: &mut G,
) -> Result<ConvertReport, AuthoringEmitError>
where
    F: FnMut(AuthoringRecordPayload) -> Result<(), AuthoringEmitError>,
    G: FnMut(AuthoringRecordValuePayload) -> Result<(), AuthoringEmitError>,
{
    let record_sink: &mut dyn FnMut(AuthoringRecordPayload) -> Result<(), AuthoringEmitError> =
        record_sink;
    let structured_cell_sink: &mut dyn FnMut(
        AuthoringRecordValuePayload,
    ) -> Result<(), AuthoringEmitError> = structured_cell_sink;
    let output = convert_btd_inner(
        options,
        TerrainRecordOutput::CollectOnly,
        Some(record_sink),
        Some(structured_cell_sink),
        Some(prepared),
    )?;
    Ok(output.report)
}

fn convert_btd_inner(
    mut options: ConvertOptions,
    output: TerrainRecordOutput,
    record_sink: Option<&mut dyn FnMut(AuthoringRecordPayload) -> Result<(), AuthoringEmitError>>,
    record_value_sink: Option<
        &mut dyn FnMut(AuthoringRecordValuePayload) -> Result<(), AuthoringEmitError>,
    >,
    prepared_source: Option<PreparedTerrainSource>,
) -> Result<ConvertOutput, AuthoringEmitError> {
    let total_started = Instant::now();
    let mut timings = Vec::new();
    let mut operation_counts = BTreeMap::new();
    let setup_started = Instant::now();
    let reused_prepared_source = prepared_source.is_some();
    let (mut btd, prepared_bounds) = match prepared_source {
        Some(prepared) => {
            if prepared.btd_path != options.btd_path {
                return Err(AuthoringEmitError::Message(
                    "prepared terrain BTD path does not match conversion options".to_owned(),
                ));
            }
            (
                prepared.btd,
                Some((
                    prepared.source_min_x,
                    prepared.source_min_y,
                    prepared.source_max_x,
                    prepared.source_max_y,
                )),
            )
        }
        None => (BtdFile::open(&options.btd_path)?, None),
    };
    let btd_cache_before = btd.tile_cache_stats();
    let frame = SourceFrame::for_header(btd.header());
    resolve_full_extent_sentinel(&mut options, btd.header());
    if prepared_bounds.is_some()
        && prepared_bounds
            != Some((
                options.source_min_x,
                options.source_min_y,
                options.source_max_x,
                options.source_max_y,
            ))
    {
        return Err(AuthoringEmitError::Message(
            "prepared terrain cell range does not match conversion options".to_owned(),
        ));
    }
    validate_range(&options)?;
    let cells_x = cell_span(options.source_min_x, options.source_max_x)?;
    let cells_y = cell_span(options.source_min_y, options.source_max_y)?;
    validate_btd_bounds(&btd, &options)?;
    let resample_mode = parse_resample_mode(&options.resample_mode)?;

    let output_dir = PathBuf::from(&options.output_authoring_dir);
    let mut authoring_output = match output {
        TerrainRecordOutput::WriteAuthoringFiles => {
            fs::create_dir_all(&output_dir)?;
            ensure_game_file(&output_dir)?;
            AuthoringOutput::write_files(output_dir.clone())
        }
        TerrainRecordOutput::CollectOnly => match (record_sink, record_value_sink) {
            (Some(record_sink), Some(record_value_sink)) => {
                AuthoringOutput::stream_records_with_structured_cells(
                    output_dir.clone(),
                    record_sink,
                    record_value_sink,
                )
            }
            (Some(record_sink), None) => {
                AuthoringOutput::stream_records(output_dir.clone(), record_sink)
            }
            (None, None) => AuthoringOutput::collect_only(output_dir.clone()),
            (None, Some(_)) => {
                return Err(AuthoringEmitError::Message(
                    "structured terrain cell sink requires a YAML header sink".to_owned(),
                ));
            }
        },
        TerrainRecordOutput::ReportOnly => AuthoringOutput::report_only(output_dir.clone()),
    };

    let diagnostics_dir = if options.debug_output_dir.is_empty() {
        output_dir
            .parent()
            .unwrap_or(output_dir.as_path())
            .join("debug")
            .join("terrain")
    } else {
        PathBuf::from(&options.debug_output_dir)
    };
    fs::create_dir_all(&diagnostics_dir)?;
    let diagnostics_path = diagnostics_dir.join("terrain_diagnostics.json");
    push_timing(&mut timings, "setup", setup_started);

    let metadata_started = Instant::now();
    let cell_count_started = Instant::now();
    let cell_count = checked_cell_count(cells_x, cells_y)?;
    push_timing(
        &mut timings,
        "metadata_and_texture_setup.cell_count",
        cell_count_started,
    );

    let preserved_ids_started = Instant::now();
    let preserved_ids = load_preserved_terrain_ids(&options)?;
    push_timing(
        &mut timings,
        "metadata_and_texture_setup.load_preserved_terrain_ids",
        preserved_ids_started,
    );

    let id_plan_started = Instant::now();
    let mut id_plan = build_terrain_id_plan(&options, &preserved_ids, cells_x, cells_y)?;
    push_timing(
        &mut timings,
        "metadata_and_texture_setup.build_terrain_id_plan",
        id_plan_started,
    );

    let converted_textures_started = Instant::now();
    let converted_textures = load_converted_textures(&options)?;
    push_timing(
        &mut timings,
        "metadata_and_texture_setup.load_converted_textures",
        converted_textures_started,
    );

    let assign_textures_started = Instant::now();
    let emitted_textures = assign_texture_form_ids(
        id_plan.next_object_id_after_terrain,
        converted_textures,
        options.preserve_source_ids,
        std::mem::take(&mut id_plan.used_object_ids),
    )?;
    push_timing(
        &mut timings,
        "metadata_and_texture_setup.assign_texture_form_ids",
        assign_textures_started,
    );

    let water_cells_started = Instant::now();
    let water_cells = load_water_cells(&options)?;
    push_timing(
        &mut timings,
        "metadata_and_texture_setup.load_water_cells",
        water_cells_started,
    );

    let next_object_id_started = Instant::now();
    let next_object_id =
        next_texture_object_id(id_plan.next_object_id_after_terrain, &emitted_textures)?;
    push_timing(
        &mut timings,
        "metadata_and_texture_setup.next_texture_object_id",
        next_object_id_started,
    );

    let texture_index_started = Instant::now();
    let textures_by_source_usage = index_textures_by_source_usage(&emitted_textures);
    let gcvr_grass_object_ids = index_grass_by_source_gcvr(&emitted_textures);
    push_timing(
        &mut timings,
        "metadata_and_texture_setup.index_textures_by_source_usage",
        texture_index_started,
    );

    let source_alpha_started = Instant::now();
    let source_alpha_masks = load_source_texture_alpha_masks(&options)?;
    push_timing(
        &mut timings,
        "metadata_and_texture_setup.load_source_texture_alpha_masks",
        source_alpha_started,
    );

    let global_blend_started = Instant::now();
    let (global_blend, global_blend_profile) = GlobalLandscapeBlend::build_profiled(
        &mut btd,
        options.source_min_x,
        options.source_min_y,
        cells_x,
        cells_y,
        &source_alpha_masks,
    )?;
    push_measured_timing(
        &mut timings,
        "metadata_and_texture_setup.build_global_landscape_blend.sample_and_materialize_vertices_exclusive",
        global_blend_profile.sample_and_materialize_seconds,
    );
    push_measured_timing(
        &mut timings,
        "metadata_and_texture_setup.build_global_landscape_blend.collect_quadrant_bases_exclusive",
        global_blend_profile.quadrant_bases_seconds,
    );
    push_measured_timing(
        &mut timings,
        "metadata_and_texture_setup.build_global_landscape_blend.plan_edge_retention_exclusive",
        global_blend_profile.edge_retention_seconds,
    );
    push_timing(
        &mut timings,
        "metadata_and_texture_setup.build_global_landscape_blend",
        global_blend_started,
    );
    operation_counts.insert(
        "global_blend.expected_global_vertices".to_owned(),
        global_blend_profile.global_vertex_count,
    );
    operation_counts.insert(
        "global_blend.expected_source_sample_selections".to_owned(),
        global_blend_profile.source_sample_selection_count,
    );
    operation_counts.insert(
        "global_blend.expected_quadrants".to_owned(),
        global_blend_profile.quadrant_count,
    );

    let required_ltex_started = Instant::now();
    let is_starfield_layout = btd.header().is_starfield_layout;
    let required_ltex_form_ids = global_blend
        .source_ltex_object_ids()
        .into_iter()
        .map(|object_id| source_ltex_form_key(object_id, is_starfield_layout))
        .collect::<Vec<_>>();
    push_timing(
        &mut timings,
        "metadata_and_texture_setup.collect_required_ltex_form_ids",
        required_ltex_started,
    );

    let converted_texture_count_started = Instant::now();
    let converted_texture_count = u32::try_from(emitted_textures.len())
        .map_err(|_| AuthoringEmitError::Message("texture count exceeds u32".to_string()))?;
    push_timing(
        &mut timings,
        "metadata_and_texture_setup.converted_texture_count",
        converted_texture_count_started,
    );
    push_timing(&mut timings, "metadata_and_texture_setup", metadata_started);

    let header_started = Instant::now();
    write_plugin_yaml(&mut authoring_output, &options, next_object_id)?;
    let world_editor_id = id_plan.world_editor_id(&options);
    let world_dir = PathBuf::from("records").join("WRLD").join(format!(
        "{} - {}_{}",
        world_editor_id,
        form_id_hex(id_plan.world_form_id),
        options.plugin_name
    ));
    write_world_yaml(
        &mut authoring_output,
        &world_dir,
        &options,
        &world_editor_id,
        id_plan.world_form_id,
    )?;
    write_texture_records(
        &mut authoring_output,
        &options,
        &emitted_textures,
        &world_editor_id,
    )?;
    push_timing(&mut timings, "write_header_records", header_started);
    let sink_profile_before_cells = authoring_output.record_sink_profile();

    let mut index = 0u32;
    let mut height_error_sum = 0.0f64;
    let mut height_error_count = 0u64;
    let mut height_max_error = 0.0f32;
    let mut dropped_texture_layers = 0u32;
    let mut ground_cover_layers = 0u32;
    let mut no_ground_cover_layers = 0u32;
    let mut vhgt_delta_clamp_stats = VhgtDeltaClampStats::default();
    let mut cell_diagnostics = Vec::with_capacity(cell_count as usize);
    let mut source_cache = SourceCellCache::new(btd.header(), &options, cells_x, cells_y, frame)?;
    let height_grid_started = Instant::now();
    let target_heights = build_target_height_grid(
        &mut btd,
        &options,
        cells_x,
        cells_y,
        resample_mode,
        &mut source_cache,
    )?;
    // Capture the global lattice origin (grid.values[0]) so each cell can be
    // quantized on the fly during the write loop, instead of materializing a
    // second full-worldspace f32 grid (the old `quantized_target_heights`,
    // which doubled the dominant terrain-phase allocation).
    let quant_origin = target_heights.values.first().copied();
    let debug_flat_height = if options.debug_flat_land {
        if target_heights.values.is_empty() {
            None
        } else {
            Some(
                target_heights.values.iter().copied().sum::<f32>()
                    / target_heights.values.len() as f32,
            )
        }
    } else {
        None
    };
    push_timing(&mut timings, "build_height_grid", height_grid_started);

    // Optional dense `.btd4` sidecar. It carries the lossless raw grid, which only
    // exists for FO76 identity; Starfield samples are already resampled onto the
    // FO4 lattice, so no .btd4 is written for them.
    let mut btd4_writer = if frame == SourceFrame::Starfield {
        if !options.btd4_output_path.is_empty() {
            eprintln!(
                "terrain_native: ignoring btd4_output_path=\"{}\" for a Starfield source — \
                 Starfield BTD samples are already resampled onto the FO4 lattice by the time \
                 LAND is emitted, so no lossless raw grid exists to write a .btd4 sidecar from.",
                options.btd4_output_path
            );
        }
        None
    } else if options.btd4_output_path.is_empty() {
        None
    } else {
        Some(crate::btd4::Btd4Writer::new(crate::btd4::Btd4Header {
            version: crate::btd4::BTD4_VERSION,
            density: CELL_SOURCE_SAMPLES as u32,
            height_min: source_cache.height_min,
            height_scale: source_cache.height_scale,
            worldspace_editor_id: world_editor_id.clone(),
            plugin_names: vec![options.plugin_name.clone()],
            cell_min_x: options.source_min_x,
            cell_min_y: options.source_min_y,
            cell_max_x: options.source_max_x,
            cell_max_y: options.source_max_y,
        }))
    };
    let mut btd4_layers_recovered = 0u32;

    let write_cells_started = Instant::now();
    let mut land_texture_fields_elapsed = Duration::ZERO;
    let mut cell_payload_emit_elapsed = Duration::ZERO;
    for cell_y in options.source_min_y..=options.source_max_y {
        for cell_x in options.source_min_x..=options.source_max_x {
            let target_cell_offset_x =
                usize::try_from(cell_x - options.source_min_x).map_err(|_| {
                    AuthoringEmitError::Message(
                        "target cell x offset cannot be represented".to_string(),
                    )
                })?;
            let target_cell_offset_y =
                usize::try_from(cell_y - options.source_min_y).map_err(|_| {
                    AuthoringEmitError::Message(
                        "target cell y offset cannot be represented".to_string(),
                    )
                })?;
            let heights = extract_target_cell_heights(
                &target_heights,
                target_cell_offset_x,
                target_cell_offset_y,
            );
            let land_heights = if let Some(height) = debug_flat_height {
                vec![height; LAND_CELL_VERTICES * LAND_CELL_VERTICES]
            } else {
                heights.clone()
            };
            // Element-wise quantization with the global origin commutes with
            // per-cell extraction, so quantizing this cell's raw heights is
            // byte-identical to extracting from a fully-quantized grid.
            let quantized_heights = match quant_origin {
                Some(origin) => quantize_cell_heights_to_lattice(&land_heights, origin),
                None => land_heights.clone(),
            };
            let (base_encodable_heights, cell_vhgt_delta_clamp_stats) =
                clamp_vhgt_delta_stream_with_stats(&quantized_heights);
            vhgt_delta_clamp_stats = vhgt_delta_clamp_stats.add(cell_vhgt_delta_clamp_stats);
            let encodable_heights = base_encodable_heights;
            let vhgt = encode_vhgt(&encodable_heights).map_err(AuthoringEmitError::Message)?;
            let mut cell_error_sum = 0.0f64;
            let mut cell_error_count = 0u64;
            let mut cell_max_error = 0.0f32;
            for (source, encoded) in heights.iter().zip(encodable_heights.iter()) {
                let error = (source - encoded).abs();
                height_error_sum += f64::from(error * error);
                height_error_count += 1;
                height_max_error = height_max_error.max(error);
                cell_error_sum += f64::from(error * error);
                cell_error_count += 1;
                cell_max_error = cell_max_error.max(error);
            }
            let vnml = generate_vnml(&encodable_heights);
            if vnml.len() != LAND_CELL_VERTICES * LAND_CELL_VERTICES * 3 {
                return Err(AuthoringEmitError::Message(
                    "VNML generation returned an invalid byte count".to_string(),
                ));
            }
            let vclr = build_land_vertex_colors(&mut btd, cell_x, cell_y)?;
            // Starfield's vclr is a synthetic neutral fill, not real source
            // data — never claim LAND_FLAG_HAS_VERTEX_COLORS for it.
            let has_vertex_colors = frame == SourceFrame::Fo76Identity;

            let cell_dir = world_dir
                .join(format!(
                    "{}, {}",
                    floor_div(cell_x, 32),
                    floor_div(cell_y, 32)
                ))
                .join(format!(
                    "{}, {}",
                    floor_div(cell_x, 8),
                    floor_div(cell_y, 8)
                ))
                .join(format!("{cell_x}, {cell_y}"));
            let land_texture_fields_started = Instant::now();
            let texture_fields = build_land_texture_fields(
                cell_x,
                cell_y,
                &global_blend,
                &textures_by_source_usage,
                &options.plugin_name,
                &mut dropped_texture_layers,
                btd4_writer.is_some(),
                options.land_skip_ground_cover_variants,
            )?;
            land_texture_fields_elapsed += land_texture_fields_started.elapsed();
            let cell_layers = texture_fields.layer_count;
            ground_cover_layers =
                ground_cover_layers.saturating_add(texture_fields.ground_cover_layer_count);
            no_ground_cover_layers =
                no_ground_cover_layers.saturating_add(texture_fields.no_ground_cover_layer_count);
            let cell_eid = id_plan.cell_editor_id(&options, cell_x, cell_y);
            let cell_payload_emit_started = Instant::now();
            write_cell_yaml(
                &mut authoring_output,
                &cell_dir,
                &options,
                cell_x,
                cell_y,
                cell_eid.as_deref(),
                id_plan.cell_form_id(&options, cell_x, cell_y, index),
                id_plan.land_form_id(&options, cell_x, cell_y, index),
                &vnml,
                &vhgt.raw,
                &vclr,
                has_vertex_colors,
                &texture_fields.fields,
                water_cells.get(&(cell_x, cell_y)),
            )?;
            cell_payload_emit_elapsed += cell_payload_emit_started.elapsed();
            let cell_rms_error = if cell_error_count == 0 {
                0.0
            } else {
                (cell_error_sum / cell_error_count as f64).sqrt() as f32
            };
            cell_diagnostics.push(CellDiagnostic {
                x: cell_x,
                y: cell_y,
                rms_error: cell_rms_error,
                max_error: cell_max_error,
                vhgt_delta_clamp_underflows: cell_vhgt_delta_clamp_stats.underflows,
                vhgt_delta_clamp_overflows: cell_vhgt_delta_clamp_stats.overflows,
                layers: cell_layers,
            });
            if let Some(writer) = btd4_writer.as_mut() {
                let gathered = gather_btd4_cell_channels(
                    &mut btd,
                    cell_x,
                    cell_y,
                    target_cell_offset_x,
                    target_cell_offset_y,
                    &texture_fields.btd4_layer_object_ids,
                    &texture_fields.btd4_source_layer_object_ids,
                    &gcvr_grass_object_ids,
                    &source_alpha_masks,
                    &mut source_cache,
                )?;
                btd4_layers_recovered =
                    btd4_layers_recovered.saturating_add(gathered.layers_recovered);
                writer
                    .add_cell(cell_x, cell_y, gathered.channels)
                    .map_err(AuthoringEmitError::Message)?;
            }
            index += 1;
        }
    }
    let write_cells_elapsed = write_cells_started.elapsed();
    let sink_profile_after_cells = authoring_output.record_sink_profile();
    let cell_sink_elapsed = sink_profile_after_cells
        .0
        .saturating_sub(sink_profile_before_cells.0);
    let cell_sink_payload_count = sink_profile_after_cells
        .1
        .saturating_sub(sink_profile_before_cells.1);
    let cell_sink_yaml_bytes = sink_profile_after_cells
        .2
        .saturating_sub(sink_profile_before_cells.2);
    let structured_cell_payload_count = sink_profile_after_cells
        .3
        .saturating_sub(sink_profile_before_cells.3);
    let yaml_format_elapsed = cell_payload_emit_elapsed.saturating_sub(cell_sink_elapsed);
    let remaining_cell_work_elapsed = write_cells_elapsed
        .saturating_sub(land_texture_fields_elapsed)
        .saturating_sub(cell_payload_emit_elapsed);
    push_elapsed_timing(
        &mut timings,
        "write_cells.build_land_texture_fields_exclusive",
        land_texture_fields_elapsed,
    );
    let (yaml_elapsed, structured_value_elapsed) = if structured_cell_payload_count > 0 {
        (Duration::ZERO, yaml_format_elapsed)
    } else {
        (yaml_format_elapsed, Duration::ZERO)
    };
    push_elapsed_timing(
        &mut timings,
        "write_cells.format_cell_yaml_excluding_sink",
        yaml_elapsed,
    );
    push_elapsed_timing(
        &mut timings,
        "write_cells.build_structured_cell_values_excluding_sink",
        structured_value_elapsed,
    );
    push_elapsed_timing(
        &mut timings,
        "write_cells.record_sink_callback_exclusive",
        cell_sink_elapsed,
    );
    push_elapsed_timing(
        &mut timings,
        "write_cells.remaining_cell_work_exclusive",
        remaining_cell_work_elapsed,
    );
    push_elapsed_timing(&mut timings, "write_cells", write_cells_elapsed);
    let cell_count_u64 = u64::from(cell_count);
    operation_counts.insert("write_cells.cells".to_owned(), cell_count_u64);
    operation_counts.insert(
        "write_cells.cell_yaml_documents".to_owned(),
        cell_count_u64.saturating_sub(structured_cell_payload_count),
    );
    operation_counts.insert(
        "write_cells.expected_quadrant_serializations".to_owned(),
        cell_count_u64.saturating_mul(4),
    );
    operation_counts.insert(
        "write_cells.expected_vertex_quantizations".to_owned(),
        cell_count_u64
            .saturating_mul(4)
            .saturating_mul((LAND_QUADRANT_VERTICES * LAND_QUADRANT_VERTICES) as u64),
    );
    operation_counts.insert(
        "write_cells.record_sink_payloads".to_owned(),
        cell_sink_payload_count,
    );
    operation_counts.insert(
        "write_cells.record_sink_yaml_bytes".to_owned(),
        cell_sink_yaml_bytes,
    );
    operation_counts.insert(
        "write_cells.structured_cell_payloads".to_owned(),
        structured_cell_payload_count,
    );

    let btd4_started = Instant::now();
    let btd4_output_path = if let Some(writer) = btd4_writer.take() {
        writer
            .finish(Path::new(&options.btd4_output_path))
            .map_err(AuthoringEmitError::Message)?;
        Some(options.btd4_output_path.clone())
    } else {
        None
    };
    push_timing(&mut timings, "write_btd4_sidecar", btd4_started);

    let heightmap_started = Instant::now();
    let (
        heightmap_output_path,
        heightmap_preview_path,
        heightmap_stats_path,
        heightmap_cell_0_0_output_path,
        heightmap_cell_0_0_preview_path,
        heightmap_cell_0_0_stats_path,
    ) = if options.export_heightmap {
        let heightmap = north_up_heightmap_grid(&target_heights);
        let path = if options.heightmap_output_path.is_empty() {
            diagnostics_dir.join(format!(
                "{}_heightmap_r32f.dds",
                options.worldspace_editor_id
            ))
        } else {
            PathBuf::from(&options.heightmap_output_path)
        };
        let stats_path = path.with_file_name(format!("{}_Stats.txt", options.worldspace_editor_id));
        let full_paths = write_heightmap_files(
            &path,
            &path.with_extension("bmp"),
            &stats_path,
            &options.worldspace_editor_id,
            id_plan.world_form_id,
            &heightmap,
        )?;

        let cell_paths = if source_range_contains_cell(&options, 0, 0) {
            let cell_offset_x = usize::try_from(0 - options.source_min_x).map_err(|_| {
                AuthoringEmitError::Message("cell 0,0 x offset overflow".to_string())
            })?;
            let cell_offset_y = usize::try_from(0 - options.source_min_y).map_err(|_| {
                AuthoringEmitError::Message("cell 0,0 y offset overflow".to_string())
            })?;
            let cell_heightmap =
                north_up_cell_heightmap_grid(&target_heights, cell_offset_x, cell_offset_y);
            let cell_path = path.with_file_name(format!(
                "{}_Cell_X{}_Y{}_heightmap_r32f.dds",
                options.worldspace_editor_id,
                coordinate_token(0),
                coordinate_token(0)
            ));
            let cell_stats_path = cell_path.with_file_name(format!(
                "{}_Cell_X{}_Y{}_Stats.txt",
                options.worldspace_editor_id,
                coordinate_token(0),
                coordinate_token(0)
            ));
            Some(write_heightmap_files(
                &cell_path,
                &cell_path.with_extension("bmp"),
                &cell_stats_path,
                &options.worldspace_editor_id,
                id_plan.world_form_id,
                &cell_heightmap,
            )?)
        } else {
            None
        };
        (
            Some(full_paths.output_path),
            Some(full_paths.preview_path),
            Some(full_paths.stats_path),
            cell_paths.as_ref().map(|paths| paths.output_path.clone()),
            cell_paths.as_ref().map(|paths| paths.preview_path.clone()),
            cell_paths.map(|paths| paths.stats_path),
        )
    } else {
        (None, None, None, None, None, None)
    };
    push_timing(&mut timings, "export_heightmaps", heightmap_started);
    let height_rms_error = if height_error_count == 0 {
        0.0
    } else {
        (height_error_sum / height_error_count as f64).sqrt() as f32
    };
    append_btd_cache_counts(
        &mut operation_counts,
        "btd_cache",
        btd_cache_before,
        btd.tile_cache_stats(),
    );
    operation_counts.insert(
        "btd_cache.prepared_source_reused".to_owned(),
        u64::from(reused_prepared_source),
    );

    let terrain_diagnostics = TerrainDiagnostics {
        height_rms_error,
        height_max_error,
        vhgt_delta_clamp_underflows: vhgt_delta_clamp_stats.underflows,
        vhgt_delta_clamp_overflows: vhgt_delta_clamp_stats.overflows,
        dropped_texture_layers,
        ground_cover_layers,
        no_ground_cover_layers,
        grass_ltex_variants: emitted_textures
            .iter()
            .filter(|texture| texture.converted.source_gcvr_form_key.is_some())
            .count() as u32,
        required_ltex_form_ids: required_ltex_form_ids.clone(),
        converted_texture_count,
        cells: cell_diagnostics,
    };
    let diagnostics = DiagnosticsReport {
        diagnostics: terrain_diagnostics,
        btd_path: options.btd_path.clone(),
        source_min_x: options.source_min_x,
        source_min_y: options.source_min_y,
        source_max_x: options.source_max_x,
        source_max_y: options.source_max_y,
        cells_written: cell_count,
        resample_mode: options.resample_mode.clone(),
        emit_textures: options.emit_textures,
        texture_manifest_path: options.texture_manifest_path.clone(),
        water_manifest_path: options.water_manifest_path.clone(),
        export_heightmap: options.export_heightmap,
        heightmap_output_path: heightmap_output_path.clone(),
        heightmap_preview_path: heightmap_preview_path.clone(),
        heightmap_stats_path: heightmap_stats_path.clone(),
        heightmap_cell_0_0_output_path: heightmap_cell_0_0_output_path.clone(),
        heightmap_cell_0_0_preview_path: heightmap_cell_0_0_preview_path.clone(),
        heightmap_cell_0_0_stats_path: heightmap_cell_0_0_stats_path.clone(),
        operation_counts: operation_counts.clone(),
        timings: timings.clone(),
    };
    let diagnostics_started = Instant::now();
    fs::write(
        &diagnostics_path,
        serde_json::to_string_pretty(&diagnostics)?,
    )?;
    push_timing(&mut timings, "write_diagnostics", diagnostics_started);
    push_timing(&mut timings, "total", total_started);

    let report = ConvertReport {
        status: "ok".to_string(),
        cells_written: cell_count,
        worldspace_editor_id: world_editor_id,
        authoring_dir: options.output_authoring_dir.clone(),
        diagnostics_path: diagnostics_path.display().to_string(),
        required_ltex_form_ids,
        height_rms_error,
        height_max_error,
        vhgt_delta_clamp_underflows: vhgt_delta_clamp_stats.underflows,
        vhgt_delta_clamp_overflows: vhgt_delta_clamp_stats.overflows,
        dropped_texture_layers,
        ground_cover_layers,
        no_ground_cover_layers,
        grass_ltex_variants: emitted_textures
            .iter()
            .filter(|texture| texture.converted.source_gcvr_form_key.is_some())
            .count() as u32,
        converted_texture_count,
        btd4_output_path,
        layers_recovered: btd4_layers_recovered,
        heightmap_output_path,
        heightmap_preview_path,
        heightmap_stats_path,
        heightmap_cell_0_0_output_path,
        heightmap_cell_0_0_preview_path,
        heightmap_cell_0_0_stats_path,
        quadrant_base_split: global_blend.quadrant_base_split_count(),
        operation_counts,
        timings,
    };
    Ok(ConvertOutput {
        report,
        authoring: authoring_output.finish(),
    })
}

fn load_converted_textures(
    options: &ConvertOptions,
) -> Result<Vec<ConvertedTerrainTexture>, AuthoringEmitError> {
    if !options.emit_textures || options.texture_manifest_path.is_empty() {
        return Ok(Vec::new());
    }
    let manifest_text = fs::read_to_string(&options.texture_manifest_path)?;
    let manifest: TextureManifest = serde_json::from_str(&manifest_text)?;
    crate::texture_bridge::plan_required_textures(&manifest).map_err(AuthoringEmitError::Message)
}

fn load_source_texture_alpha_masks(
    options: &ConvertOptions,
) -> Result<TextureAlphaMasks, AuthoringEmitError> {
    if options.texture_manifest_path.is_empty() {
        return Ok(TextureAlphaMasks::default());
    }
    let manifest_text = fs::read_to_string(&options.texture_manifest_path)?;
    let manifest: TextureManifest = serde_json::from_str(&manifest_text)?;
    let mut masks = TextureAlphaMasks::default();
    for bundle in manifest.textures {
        let Some(object_id) = object_id_from_source_form_key(&bundle.source_ltex_form_key) else {
            continue;
        };
        if masks.by_source_ltex_object_id.contains_key(&object_id) {
            continue;
        }
        masks.by_source_ltex_object_id.insert(
            object_id,
            load_source_texture_alpha_mask(Path::new(&bundle.diffuse_path))?,
        );
    }
    Ok(masks)
}

fn load_source_texture_alpha_mask(path: &Path) -> Result<TextureAlphaMask, AuthoringEmitError> {
    let metadata = fs::metadata(path).map_err(|error| {
        AuthoringEmitError::Message(format!(
            "read terrain texture metadata {}: {error}",
            path.display()
        ))
    })?;
    let file_len = metadata.len();
    let modified = metadata.modified().ok();
    let cache = SOURCE_TEXTURE_ALPHA_CACHE.get_or_init(Default::default);
    {
        let mut cache = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(mask) = cache.get(path, file_len, modified) {
            return Ok(mask);
        }
    }

    let image =
        directxtex_native::read_dds_rgba_image(path).map_err(AuthoringEmitError::Message)?;
    let mask = TextureAlphaMask {
        width: image.width as usize,
        height: image.height as usize,
        alpha: image
            .rgba
            .chunks_exact(4)
            .map(|pixel| pixel[3])
            .collect::<Vec<_>>()
            .into(),
    };

    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cached) = cache.get(path, file_len, modified) {
        return Ok(cached);
    }
    cache.insert(path.to_path_buf(), file_len, modified, mask.clone());
    Ok(mask)
}

fn push_timing(timings: &mut Vec<TimingEntry>, name: &str, started: Instant) {
    push_elapsed_timing(timings, name, started.elapsed());
}

fn push_elapsed_timing(timings: &mut Vec<TimingEntry>, name: &str, elapsed: Duration) {
    push_measured_timing(timings, name, elapsed.as_secs_f64());
}

fn push_measured_timing(timings: &mut Vec<TimingEntry>, name: &str, elapsed_seconds: f64) {
    timings.push(TimingEntry {
        name: name.to_owned(),
        elapsed_seconds: (elapsed_seconds * 1_000_000.0).round() / 1_000_000.0,
    });
}

fn append_btd_cache_counts(
    counts: &mut BTreeMap<String, u64>,
    prefix: &str,
    before: BtdTileCacheStats,
    after: BtdTileCacheStats,
) {
    counts.insert(
        format!("{prefix}.hits"),
        after.hits.saturating_sub(before.hits),
    );
    counts.insert(
        format!("{prefix}.misses"),
        after.misses.saturating_sub(before.misses),
    );
    counts.insert(format!("{prefix}.cached_tiles_start"), before.cached_tiles);
    counts.insert(format!("{prefix}.cached_tiles_end"), after.cached_tiles);
    counts.insert(
        format!("{prefix}.cached_payload_bytes_start"),
        before.cached_payload_bytes,
    );
    counts.insert(
        format!("{prefix}.cached_payload_bytes_end"),
        after.cached_payload_bytes,
    );
}

fn load_water_cells(
    options: &ConvertOptions,
) -> Result<HashMap<(i32, i32), WaterCell>, AuthoringEmitError> {
    if options.water_manifest_path.is_empty() {
        return Ok(HashMap::new());
    }
    let manifest_text = fs::read_to_string(&options.water_manifest_path)?;
    let manifest: WaterManifest = serde_json::from_str(&manifest_text)?;
    let mut cells = HashMap::with_capacity(manifest.cells.len());
    for cell in manifest.cells {
        if !cell.height.is_finite() {
            return Err(AuthoringEmitError::Message(format!(
                "water height for cell ({}, {}) is not finite",
                cell.x, cell.y
            )));
        }
        cells.insert(
            (cell.x, cell.y),
            WaterCell {
                height: cell.height,
                water_object_id: cell
                    .water_object_id
                    .unwrap_or(manifest.default_water_object_id),
            },
        );
    }
    Ok(cells)
}

fn assign_texture_form_ids(
    first_texture_object_id: u32,
    converted: Vec<ConvertedTerrainTexture>,
    preserve_source_ids: bool,
    mut used_object_ids: HashSet<u32>,
) -> Result<Vec<EmittedTexture>, AuthoringEmitError> {
    let mut emitted = Vec::with_capacity(converted.len());
    let mut next_object_id = first_texture_object_id;
    let mut txst_object_ids: HashMap<String, u32> = HashMap::new();
    let mut grass_object_ids_by_source: HashMap<String, u32> = HashMap::new();
    for texture in converted {
        let txst_key = txst_allocation_key(&texture);
        let txst_object_id = if let Some(object_id) = txst_object_ids.get(&txst_key).copied() {
            object_id
        } else {
            let object_id = preserved_or_allocated_object_id(
                &texture.source_txst_form_key,
                preserve_source_ids,
                &mut next_object_id,
                &mut used_object_ids,
            )?;
            txst_object_ids.insert(txst_key, object_id);
            object_id
        };
        let ltex_object_id = if texture.source_gcvr_form_key.is_some() {
            allocate_next_texture_object_id(&mut next_object_id, &mut used_object_ids)?
        } else {
            preserved_or_allocated_object_id(
                &texture.source_ltex_form_key,
                preserve_source_ids,
                &mut next_object_id,
                &mut used_object_ids,
            )?
        };
        let mut grass_object_ids = Vec::with_capacity(texture.grass.len());
        for grass in &texture.grass {
            let grass_key = normalize_source_form_key(&grass.source_form_key);
            let object_id =
                if let Some(object_id) = grass_object_ids_by_source.get(&grass_key).copied() {
                    object_id
                } else {
                    let object_id = preserved_or_allocated_object_id(
                        &grass.source_form_key,
                        preserve_source_ids,
                        &mut next_object_id,
                        &mut used_object_ids,
                    )?;
                    grass_object_ids_by_source.insert(grass_key, object_id);
                    object_id
                };
            grass_object_ids.push(object_id);
        }
        emitted.push(EmittedTexture {
            converted: texture,
            txst_object_id,
            ltex_object_id,
            grass_object_ids,
        });
    }
    Ok(emitted)
}

fn txst_allocation_key(texture: &ConvertedTerrainTexture) -> String {
    let normalized_source = normalize_source_form_key(&texture.source_txst_form_key);
    if normalized_source != "000000" {
        return format!("source:{normalized_source}");
    }
    format!(
        "assets:{}|{}|{}|{}",
        texture.diffuse_rel_path.to_ascii_lowercase(),
        texture.normal_rel_path.to_ascii_lowercase(),
        texture.specgloss_rel_path.to_ascii_lowercase(),
        texture.glow_rel_path.to_ascii_lowercase()
    )
}

fn preserved_or_allocated_object_id(
    source_form_key: &str,
    preserve_source_ids: bool,
    next_object_id: &mut u32,
    used_object_ids: &mut HashSet<u32>,
) -> Result<u32, AuthoringEmitError> {
    if preserve_source_ids {
        if let Some(object_id) = object_id_from_source_form_key(source_form_key) {
            if used_object_ids.insert(object_id) {
                if object_id >= *next_object_id {
                    *next_object_id = object_id.checked_add(1).ok_or_else(|| {
                        AuthoringEmitError::Message(
                            "texture form ID allocation overflow".to_string(),
                        )
                    })?;
                }
                return Ok(object_id);
            }
        }
    }
    allocate_next_texture_object_id(next_object_id, used_object_ids)
}

fn allocate_next_texture_object_id(
    next_object_id: &mut u32,
    used_object_ids: &mut HashSet<u32>,
) -> Result<u32, AuthoringEmitError> {
    loop {
        let object_id = *next_object_id;
        *next_object_id = next_object_id.checked_add(1).ok_or_else(|| {
            AuthoringEmitError::Message("texture form ID allocation overflow".to_string())
        })?;
        if used_object_ids.insert(object_id) {
            return Ok(object_id);
        }
    }
}

fn object_id_from_source_form_key(value: &str) -> Option<u32> {
    for object in value.split([':', '@']) {
        let object = object.trim();
        let object = object.strip_prefix("0x").unwrap_or(object);
        if object.is_empty() {
            continue;
        }
        if let Ok(parsed) = u32::from_str_radix(object, 16) {
            return Some(parsed & 0x00FF_FFFF);
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SourceTextureUsageKey {
    ltex: String,
    gcvr: Option<String>,
}

fn index_textures_by_source_usage(
    textures: &[EmittedTexture],
) -> HashMap<SourceTextureUsageKey, &EmittedTexture> {
    let mut result = HashMap::new();
    for texture in textures {
        result.insert(
            SourceTextureUsageKey {
                ltex: normalize_source_form_key(&texture.converted.source_ltex_form_key),
                gcvr: texture
                    .converted
                    .source_gcvr_form_key
                    .as_deref()
                    .map(normalize_source_form_key),
            },
            texture,
        );
    }
    result
}

fn index_grass_by_source_gcvr(textures: &[EmittedTexture]) -> HashMap<u32, Vec<u32>> {
    let mut result = HashMap::<u32, Vec<u32>>::new();
    for texture in textures {
        let Some(source_gcvr) = texture
            .converted
            .source_gcvr_form_key
            .as_deref()
            .and_then(object_id_from_source_form_key)
        else {
            continue;
        };
        let grass = result.entry(source_gcvr).or_default();
        for object_id in &texture.grass_object_ids {
            if !grass.contains(object_id) {
                grass.push(*object_id);
            }
        }
        grass.sort_unstable();
    }
    result
}

fn collect_required_ltex_form_ids(
    btd: &mut BtdFile,
    options: &ConvertOptions,
) -> Result<Vec<String>, AuthoringEmitError> {
    let usages = collect_required_texture_usages(btd, options)?;
    let mut required = BTreeSet::new();
    for usage in usages {
        required.insert(usage.ltex_form_key);
    }
    Ok(required.into_iter().collect())
}

fn collect_required_texture_usages(
    btd: &mut BtdFile,
    options: &ConvertOptions,
) -> Result<Vec<RequiredTextureUsage>, AuthoringEmitError> {
    let cells_x = cell_span(options.source_min_x, options.source_max_x)?;
    let cells_y = cell_span(options.source_min_y, options.source_max_y)?;
    let source_alpha_masks = load_source_texture_alpha_masks(options)?;
    let blend = GlobalLandscapeBlend::build(
        btd,
        options.source_min_x,
        options.source_min_y,
        cells_x,
        cells_y,
        &source_alpha_masks,
    )?;
    let mut required_source_ltex_object_ids = BTreeSet::new();
    for cell_y in options.source_min_y..=options.source_max_y {
        for cell_x in options.source_min_x..=options.source_max_x {
            for quadrant in 0..4 {
                let Some(quadrant_blend) = blend
                    .serialize_quadrant(cell_x, cell_y, quadrant)
                    .map_err(AuthoringEmitError::Message)?
                else {
                    continue;
                };
                required_source_ltex_object_ids.insert(quadrant_blend.base_source_ltex_object_id);
                required_source_ltex_object_ids
                    .extend(quadrant_blend.alpha_source_ltex_object_ids.iter().copied());
            }
        }
    }
    texture_usages_for_required_ids(btd, options, &required_source_ltex_object_ids)
}

fn collect_required_texture_usages_lightweight_profiled(
    btd: &mut BtdFile,
    options: &ConvertOptions,
) -> Result<RequiredTextureUsageProfile, AuthoringEmitError> {
    let mut timings = Vec::new();
    let mut operation_counts = BTreeMap::new();
    let cells_x = cell_span(options.source_min_x, options.source_max_x)?;
    let cells_y = cell_span(options.source_min_y, options.source_max_y)?;
    let source_alpha_started = Instant::now();
    let source_alpha_masks = load_source_texture_alpha_masks(options)?;
    push_timing(
        &mut timings,
        "load_source_texture_alpha_masks_exclusive",
        source_alpha_started,
    );
    let (required_source_ltex_object_ids, blend_profile) =
        collect_required_source_ltex_object_ids_profiled(
            btd,
            options.source_min_x,
            options.source_min_y,
            cells_x,
            cells_y,
            &source_alpha_masks,
        )?;
    push_measured_timing(
        &mut timings,
        "sample_quantize_and_summarize_vertices_exclusive",
        blend_profile.sample_and_summarize_seconds,
    );
    push_measured_timing(
        &mut timings,
        "collect_quadrant_bases_exclusive",
        blend_profile.quadrant_bases_seconds,
    );
    push_measured_timing(
        &mut timings,
        "edge_retention_and_texture_selection_exclusive",
        blend_profile.retention_and_selection_seconds,
    );
    operation_counts.insert(
        "global_blend.expected_global_vertices".to_owned(),
        blend_profile.global_vertex_count,
    );
    operation_counts.insert(
        "global_blend.expected_source_sample_selections".to_owned(),
        blend_profile.source_sample_selection_count,
    );
    operation_counts.insert(
        "global_blend.expected_quadrants".to_owned(),
        blend_profile.quadrant_count,
    );

    let resolve_usages_started = Instant::now();
    let usages = texture_usages_for_required_ids(btd, options, &required_source_ltex_object_ids)?;
    push_timing(
        &mut timings,
        "resolve_texture_usages_exclusive",
        resolve_usages_started,
    );
    operation_counts.insert(
        "required_source_ltex_object_ids".to_owned(),
        required_source_ltex_object_ids.len() as u64,
    );
    operation_counts.insert("resolved_texture_usages".to_owned(), usages.len() as u64);
    Ok(RequiredTextureUsageProfile {
        usages,
        timings,
        operation_counts,
    })
}

fn texture_usages_for_required_ids(
    btd: &BtdFile,
    options: &ConvertOptions,
    required_source_ltex_object_ids: &BTreeSet<u32>,
) -> Result<Vec<RequiredTextureUsage>, AuthoringEmitError> {
    let is_starfield_layout = btd.header().is_starfield_layout;
    let mut usages = required_source_ltex_object_ids
        .iter()
        .copied()
        .map(|object_id| RequiredTextureUsage {
            ltex_form_key: source_ltex_form_key(object_id, is_starfield_layout),
            ground_cover_form_key: None,
        })
        .collect::<BTreeSet<_>>();

    if btd.header().gcvr_count == 0 {
        return Ok(usages.into_iter().collect());
    }

    for cell_y in options.source_min_y..=options.source_max_y {
        for cell_x in options.source_min_x..=options.source_max_x {
            let set = btd.cell_texture_set(cell_x, cell_y)?;
            for quad in &set.quadrants {
                add_ground_cover_usage_for_layer(
                    btd,
                    &required_source_ltex_object_ids,
                    quad.base,
                    quad.base_source_slot,
                    &quad.ground_cover,
                    &mut usages,
                );
                for (texture_index, source_slot) in quad
                    .additional
                    .iter()
                    .zip(quad.additional_source_slots.iter())
                {
                    add_ground_cover_usage_for_layer(
                        btd,
                        &required_source_ltex_object_ids,
                        *texture_index,
                        *source_slot,
                        &quad.ground_cover,
                        &mut usages,
                    );
                }
            }
        }
    }

    Ok(usages.into_iter().collect())
}

fn add_ground_cover_usage_for_layer(
    btd: &BtdFile,
    required_source_ltex_object_ids: &BTreeSet<u32>,
    texture_index: Option<u8>,
    source_slot: Option<u8>,
    ground_cover: &[Option<u8>; 8],
    usages: &mut BTreeSet<RequiredTextureUsage>,
) {
    let Some(texture_index) = texture_index else {
        return;
    };
    let Some(source_slot) = source_slot else {
        return;
    };
    let Some(source_ltex_object_id) = btd
        .land_texture_form_id(texture_index as usize)
        .map(|form_id| form_id & 0x00FF_FFFF)
    else {
        return;
    };
    if !required_source_ltex_object_ids.contains(&source_ltex_object_id) {
        return;
    }
    let Some(ground_cover_index) = ground_cover.get(source_slot as usize).copied().flatten() else {
        return;
    };
    let Some(ground_cover_object_id) = btd
        .ground_cover_form_id(ground_cover_index as usize)
        .map(|form_id| form_id & 0x00FF_FFFF)
    else {
        return;
    };
    usages.insert(RequiredTextureUsage {
        ltex_form_key: source_ltex_form_key(
            source_ltex_object_id,
            btd.header().is_starfield_layout,
        ),
        ground_cover_form_key: Some(source_ltex_form_key(
            ground_cover_object_id,
            btd.header().is_starfield_layout,
        )),
    });
}

fn resolve_full_extent_sentinel(options: &mut ConvertOptions, header: &BtdHeader) {
    if options.source_max_x != -1 || options.source_max_y != -1 {
        return;
    }
    match SourceFrame::for_header(header) {
        SourceFrame::Fo76Identity => {
            options.source_min_x = header.cell_min_x;
            options.source_min_y = header.cell_min_y;
            options.source_max_x = header.cell_max_x;
            options.source_max_y = header.cell_max_y;
        }
        // Under the Starfield frame, source_min/max_x/y denote the emitted
        // FO4 cell window, not BTD (SF) cell indices — fill from the FO4
        // window that covers the BTD's full SF cell extent.
        SourceFrame::Starfield => {
            let (min_x, max_x) =
                crate::sf_frame::fo4_cell_range(header.cell_min_x, header.cell_max_x);
            let (min_y, max_y) =
                crate::sf_frame::fo4_cell_range(header.cell_min_y, header.cell_max_y);
            options.source_min_x = min_x;
            options.source_max_x = max_x;
            options.source_min_y = min_y;
            options.source_max_y = max_y;
        }
    }
}

fn validate_range(options: &ConvertOptions) -> Result<(), AuthoringEmitError> {
    if options.source_min_x > options.source_max_x || options.source_min_y > options.source_max_y {
        return Err(AuthoringEmitError::Message(
            "source min coordinates must be <= source max coordinates".to_string(),
        ));
    }
    Ok(())
}

fn validate_btd_bounds(btd: &BtdFile, options: &ConvertOptions) -> Result<(), AuthoringEmitError> {
    let header = btd.header();
    // Under Starfield, options.source_min/max_x/y are FO4 cell coordinates —
    // compare against the FO4 window that covers the BTD's SF cell extent,
    // not the raw (SF-space) header bounds.
    let (min_x, max_x, min_y, max_y) = match SourceFrame::for_header(header) {
        SourceFrame::Fo76Identity => (
            header.cell_min_x,
            header.cell_max_x,
            header.cell_min_y,
            header.cell_max_y,
        ),
        SourceFrame::Starfield => {
            let (min_x, max_x) =
                crate::sf_frame::fo4_cell_range(header.cell_min_x, header.cell_max_x);
            let (min_y, max_y) =
                crate::sf_frame::fo4_cell_range(header.cell_min_y, header.cell_max_y);
            (min_x, max_x, min_y, max_y)
        }
    };
    if options.source_min_x < min_x
        || options.source_max_x > max_x
        || options.source_min_y < min_y
        || options.source_max_y > max_y
    {
        return Err(AuthoringEmitError::Message(format!(
            "requested cell range ({}, {})..({}, {}) is outside BTD bounds ({}, {})..({}, {})",
            options.source_min_x,
            options.source_min_y,
            options.source_max_x,
            options.source_max_y,
            min_x,
            min_y,
            max_x,
            max_y
        )));
    }
    Ok(())
}

fn parse_resample_mode(mode: &str) -> Result<ResampleMode, AuthoringEmitError> {
    match mode {
        "sample4" => Ok(ResampleMode::Sample4),
        "weighted" => Ok(ResampleMode::Weighted),
        "feature" => Ok(ResampleMode::Feature),
        "lanczos" => Ok(ResampleMode::Lanczos),
        other => Err(AuthoringEmitError::Message(format!(
            "unsupported resample mode: {other}"
        ))),
    }
}

fn build_target_height_grid(
    btd: &mut BtdFile,
    options: &ConvertOptions,
    cells_x: usize,
    cells_y: usize,
    mode: ResampleMode,
    source_cache: &mut SourceCellCache,
) -> Result<TargetHeightGrid, AuthoringEmitError> {
    let width = cells_x
        .checked_mul(LAND_CELL_INTERVALS)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| AuthoringEmitError::Message("target grid width overflow".to_string()))?;
    let height = cells_y
        .checked_mul(LAND_CELL_INTERVALS)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| AuthoringEmitError::Message("target grid height overflow".to_string()))?;
    let len = width
        .checked_mul(height)
        .ok_or_else(|| AuthoringEmitError::Message("target grid size overflow".to_string()))?;
    let max_vec_elements = isize::MAX as usize / std::mem::size_of::<f32>();
    if len > max_vec_elements {
        return Err(AuthoringEmitError::Message(
            "target grid exceeds Vec allocation limit".to_string(),
        ));
    }

    if source_cache.frame == SourceFrame::Fo76Identity && matches!(mode, ResampleMode::Lanczos) {
        return build_fo76_lanczos_height_grid(btd, source_cache, width, height);
    }
    let mut values = Vec::with_capacity(len);
    for target_y in 0..height {
        let source_y = target_y
            .saturating_mul(4)
            .min(source_cache.source_height - 1);
        let source_cell_y = options
            .source_min_y
            .checked_add(usize_to_i32(source_y / CELL_SOURCE_SAMPLES)?)
            .ok_or_else(|| {
                AuthoringEmitError::Message("source cell y coordinate overflow".to_string())
            })?;
        source_cache.retain_neighbor_rows(source_cell_y);
        for target_x in 0..width {
            let height = match mode {
                ResampleMode::Sample4 => {
                    sample4_target_height(btd, source_cache, target_x, target_y)?
                }
                ResampleMode::Weighted => {
                    weighted_target_height(btd, source_cache, target_x, target_y)?
                }
                ResampleMode::Feature => {
                    feature_target_height(btd, source_cache, target_x, target_y)?
                }
                ResampleMode::Lanczos => {
                    lanczos_target_height(btd, source_cache, target_x, target_y)?
                }
            };
            values.push(height);
        }
    }

    Ok(TargetHeightGrid {
        width,
        height,
        values,
    })
}

fn build_fo76_lanczos_height_grid(
    btd: &mut BtdFile,
    source_cache: &mut SourceCellCache,
    width: usize,
    height: usize,
) -> Result<TargetHeightGrid, AuthoringEmitError> {
    let kernel = lanczos2_kernel();
    let taps = |target: usize, extent: usize| -> [usize; LANCZOS2_TAPS] {
        std::array::from_fn(|index| {
            let unshifted = clamp_offset_index(
                target.saturating_mul(4),
                index as isize - LANCZOS2_REACH,
                extent,
            );
            (unshifted + crate::fo4_frame::HALF_CELL_SAMPLES).min(extent - 1)
        })
    };
    let x_taps: Vec<_> = (0..width)
        .map(|x| taps(x, source_cache.source_width))
        .collect();
    let mut rows: HashMap<usize, Vec<f32>> = HashMap::new();
    let mut values = Vec::with_capacity(width * height);
    let mut load_time = std::time::Duration::ZERO;
    let mut sample_time = std::time::Duration::ZERO;
    for target_y in 0..height {
        let ys = taps(target_y, source_cache.source_height);
        let started = Instant::now();
        rows.retain(|y, _| ys.contains(y));
        let center_cell_y = source_cache
            .source_min_y
            .checked_add(usize_to_i32(ys[LANCZOS2_TAPS / 2] / CELL_SOURCE_SAMPLES)?)
            .ok_or_else(|| {
                AuthoringEmitError::Message("source cell y coordinate overflow".into())
            })?;
        source_cache.retain_neighbor_rows(center_cell_y);
        for y in ys {
            if rows.contains_key(&y) {
                continue;
            }
            let cell_y = source_cache
                .source_min_y
                .checked_add(usize_to_i32(y / CELL_SOURCE_SAMPLES)?)
                .ok_or_else(|| {
                    AuthoringEmitError::Message("source cell y coordinate overflow".into())
                })?;
            let mut row = Vec::with_capacity(source_cache.source_width);
            for cell_offset_x in 0..source_cache.source_width / CELL_SOURCE_SAMPLES {
                let cell_x = source_cache
                    .source_min_x
                    .checked_add(usize_to_i32(cell_offset_x)?)
                    .ok_or_else(|| {
                        AuthoringEmitError::Message("source cell x coordinate overflow".into())
                    })?;
                let start = (y % CELL_SOURCE_SAMPLES) * CELL_SOURCE_SAMPLES;
                row.extend_from_slice(
                    &source_cache.cell_values(btd, cell_x, cell_y)?
                        [start..start + CELL_SOURCE_SAMPLES],
                );
            }
            rows.insert(y, row);
        }
        load_time += started.elapsed();
        let started = Instant::now();
        let source_rows: [&[f32]; LANCZOS2_TAPS] =
            std::array::from_fn(|index| rows[&ys[index]].as_slice());
        let row: Vec<f32> = x_taps
            .par_iter()
            .map(|xs| {
                let mut weighted_sum = 0.0f64;
                let mut min_value = f32::INFINITY;
                let mut max_value = f32::NEG_INFINITY;
                for (ky, wy) in kernel.iter().enumerate() {
                    for (kx, wx) in kernel.iter().enumerate() {
                        let value = source_rows[ky][xs[kx]];
                        min_value = min_value.min(value);
                        max_value = max_value.max(value);
                        weighted_sum += (wx * wy) as f64 * value as f64;
                    }
                }
                (weighted_sum as f32).clamp(min_value, max_value)
            })
            .collect();
        values.extend(row);
        sample_time += started.elapsed();
    }
    eprintln!(
        "[terrain_height_grid] frame=fo76 mode=lanczos vertices={} workers={} source_rows_ms={} resample_ms={}",
        values.len(),
        rayon::current_num_threads(),
        load_time.as_millis(),
        sample_time.as_millis()
    );
    Ok(TargetHeightGrid {
        width,
        height,
        values,
    })
}

fn north_up_heightmap_grid(grid: &TargetHeightGrid) -> TargetHeightGrid {
    let mut values = Vec::with_capacity(grid.values.len());
    for y in (0..grid.height).rev() {
        let row_start = y * grid.width;
        values.extend_from_slice(&grid.values[row_start..row_start + grid.width]);
    }

    TargetHeightGrid {
        width: grid.width,
        height: grid.height,
        values,
    }
}

fn north_up_cell_heightmap_grid(
    grid: &TargetHeightGrid,
    cell_offset_x: usize,
    cell_offset_y: usize,
) -> TargetHeightGrid {
    let values = extract_target_cell_heights(grid, cell_offset_x, cell_offset_y);
    let cell_grid = TargetHeightGrid {
        width: LAND_CELL_VERTICES,
        height: LAND_CELL_VERTICES,
        values,
    };
    north_up_heightmap_grid(&cell_grid)
}

fn write_heightmap_files(
    output_path: &Path,
    preview_path: &Path,
    stats_path: &Path,
    worldspace_editor_id: &str,
    world_form_id: u32,
    heightmap: &TargetHeightGrid,
) -> Result<HeightmapExportPaths, AuthoringEmitError> {
    let (height_min, height_max) = height_value_range(&heightmap.values)?;
    let normalized_heights = normalize_heightmap_values(&heightmap.values, height_min, height_max);
    crate::heightmap_dds::write_r32_float_dds(
        output_path,
        heightmap.width,
        heightmap.height,
        &normalized_heights,
    )?;
    crate::heightmap_dds::write_grayscale_bmp(
        preview_path,
        heightmap.width,
        heightmap.height,
        &normalized_heights,
    )?;
    write_heightmap_stats(
        stats_path,
        worldspace_editor_id,
        world_form_id,
        height_min,
        height_max,
    )?;
    Ok(HeightmapExportPaths {
        output_path: output_path.display().to_string(),
        preview_path: preview_path.display().to_string(),
        stats_path: stats_path.display().to_string(),
    })
}

fn source_range_contains_cell(options: &ConvertOptions, cell_x: i32, cell_y: i32) -> bool {
    options.source_min_x <= cell_x
        && cell_x <= options.source_max_x
        && options.source_min_y <= cell_y
        && cell_y <= options.source_max_y
}

fn height_value_range(values: &[f32]) -> Result<(f32, f32), AuthoringEmitError> {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for value in values {
        if !value.is_finite() {
            return Err(AuthoringEmitError::Message(
                "heightmap values must be finite".to_string(),
            ));
        }
        min = min.min(*value);
        max = max.max(*value);
    }
    if values.is_empty() {
        return Err(AuthoringEmitError::Message(
            "heightmap values are empty".to_string(),
        ));
    }
    Ok((min, max))
}

fn normalize_heightmap_values(values: &[f32], min: f32, max: f32) -> Vec<f32> {
    let range = max - min;
    if range <= 0.0 {
        return vec![0.0; values.len()];
    }
    values
        .iter()
        .map(|value| ((*value - min) / range).clamp(0.0, 1.0))
        .collect()
}

fn write_heightmap_stats(
    path: &Path,
    worldspace_editor_id: &str,
    world_form_id: u32,
    height_min: f32,
    height_max: f32,
) -> Result<(), AuthoringEmitError> {
    let payload = format!(
        "Worldspace: {} [{}]\r\nMax height: {:.6}\r\nMin height: {:.6}\r\n",
        worldspace_editor_id,
        compact_form_id_hex(world_form_id),
        height_max,
        height_min
    );
    fs::write(path, payload)?;
    Ok(())
}

/// Nearest BTD sample index for a fractional `sf_frame` position, never
/// negative (mirrors `sample()`'s Starfield lower-bound clamp).
fn round_nonneg_index(value: f64) -> usize {
    value.round().max(0.0) as usize
}

fn sample4_target_height(
    btd: &mut BtdFile,
    source_cache: &mut SourceCellCache,
    target_x: usize,
    target_y: usize,
) -> Result<f32, AuthoringEmitError> {
    match source_cache.frame {
        SourceFrame::Fo76Identity => {
            source_cache.sample(btd, target_x.saturating_mul(4), target_y.saturating_mul(4))
        }
        SourceFrame::Starfield => {
            let x = round_nonneg_index(source_cache.starfield_center_x(target_x));
            let y = round_nonneg_index(source_cache.starfield_center_y(target_y));
            source_cache.sample(btd, x, y)
        }
    }
}

fn weighted_target_height(
    btd: &mut BtdFile,
    source_cache: &mut SourceCellCache,
    target_x: usize,
    target_y: usize,
) -> Result<f32, AuthoringEmitError> {
    let (center_x, center_y) = match source_cache.frame {
        SourceFrame::Fo76Identity => (
            target_x.saturating_mul(4) as f32,
            target_y.saturating_mul(4) as f32,
        ),
        SourceFrame::Starfield => (
            source_cache.starfield_center_x(target_x) as f32,
            source_cache.starfield_center_y(target_y) as f32,
        ),
    };
    let min_x = clamp_floor_index(center_x - 1.0, source_cache.source_width);
    let max_x = clamp_ceil_index(center_x + 2.0, source_cache.source_width);
    let min_y = clamp_floor_index(center_y - 1.0, source_cache.source_height);
    let max_y = clamp_ceil_index(center_y + 2.0, source_cache.source_height);

    let mut xs = [0usize; 25];
    let mut ys = [0usize; 25];
    let mut values = [0.0f32; 25];
    let mut count = 0usize;
    let mut min_value = f32::INFINITY;
    let mut max_value = f32::NEG_INFINITY;
    let mut sum = 0.0f32;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let value = source_cache.sample(btd, x, y)?;
            xs[count] = x;
            ys[count] = y;
            values[count] = value;
            count += 1;
            min_value = min_value.min(value);
            max_value = max_value.max(value);
            sum += value;
        }
    }

    let mean = sum / count as f32;
    let variance = values[..count]
        .iter()
        .map(|value| {
            let diff = *value - mean;
            diff * diff
        })
        .sum::<f32>()
        / count as f32;
    let stddev = variance.sqrt();
    let mut weighted_sum = 0.0;
    let mut total_weight = 0.0;

    for i in 0..count {
        let value = values[i];
        let dx = xs[i] as f32 - center_x;
        let dy = ys[i] as f32 - center_y;
        let distance = (dx * dx + dy * dy).sqrt();
        let mut weight = 1.0 / (distance + 1.0);
        if stddev > 0.0 && (value - mean).abs() > stddev {
            weight *= 1.5;
        }
        weighted_sum += value * weight;
        total_weight += weight;
    }

    Ok((weighted_sum / total_weight).clamp(min_value, max_value))
}

fn feature_target_height(
    btd: &mut BtdFile,
    source_cache: &mut SourceCellCache,
    target_x: usize,
    target_y: usize,
) -> Result<f32, AuthoringEmitError> {
    let (center_x, center_y) = match source_cache.frame {
        SourceFrame::Fo76Identity => (
            target_x.saturating_mul(4) as f32,
            target_y.saturating_mul(4) as f32,
        ),
        SourceFrame::Starfield => (
            source_cache.starfield_center_x(target_x) as f32,
            source_cache.starfield_center_y(target_y) as f32,
        ),
    };
    let min_x = clamp_floor_index(center_x - 2.0, source_cache.source_width);
    let max_x = clamp_ceil_index(center_x + 2.0, source_cache.source_width);
    let min_y = clamp_floor_index(center_y - 2.0, source_cache.source_height);
    let max_y = clamp_ceil_index(center_y + 2.0, source_cache.source_height);

    let mut count = 0usize;
    let mut min_value = f32::INFINITY;
    let mut max_value = f32::NEG_INFINITY;
    let mut min_distance = f32::INFINITY;
    let mut max_distance = f32::INFINITY;
    let mut sum = 0.0f32;
    let mut weighted_sum = 0.0f32;
    let mut total_weight = 0.0f32;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let value = source_cache.sample(btd, x, y)?;
            let dx = x as f32 - center_x;
            let dy = y as f32 - center_y;
            let distance = (dx * dx + dy * dy).sqrt();
            let weight = 1.0 / (distance + 1.0);
            if value < min_value {
                min_value = value;
                min_distance = distance;
            }
            if value > max_value {
                max_value = value;
                max_distance = distance;
            }
            sum += value;
            weighted_sum += value * weight;
            total_weight += weight;
            count += 1;
        }
    }

    let mean = sum / count as f32;
    let weighted = weighted_sum / total_weight;
    let relief = max_value - min_value;
    let high_centrality = 1.0 / (max_distance + 1.0);
    let low_centrality = 1.0 / (min_distance + 1.0);
    let (feature, centrality) =
        if (max_value - mean) * high_centrality >= (mean - min_value) * low_centrality {
            (max_value, high_centrality)
        } else {
            (min_value, low_centrality)
        };
    let blend = ((relief - 64.0) / 512.0).clamp(0.0, 1.0) * centrality;

    Ok((weighted * (1.0 - blend) + feature * blend).clamp(min_value, max_value))
}

/// Absolute (`AxisTap.first`-relative) BTD sample index, clamped in bounds —
/// the Starfield twin of `clamp_offset_index`, which takes a `usize` center
/// + `isize` offset instead of a signed absolute index.
fn clamp_absolute_index(value: i32, extent: usize) -> usize {
    value.clamp(0, extent as i32 - 1) as usize
}

fn lanczos_target_height(
    btd: &mut BtdFile,
    source_cache: &mut SourceCellCache,
    target_x: usize,
    target_y: usize,
) -> Result<f32, AuthoringEmitError> {
    match source_cache.frame {
        SourceFrame::Fo76Identity => {
            let kernel = lanczos2_kernel();
            let center_x = target_x.saturating_mul(4);
            let center_y = target_y.saturating_mul(4);
            let mut weighted_sum = 0.0f64;
            let mut min_value = f32::INFINITY;
            let mut max_value = f32::NEG_INFINITY;

            for (ky, wy) in kernel.iter().enumerate() {
                let y = clamp_offset_index(
                    center_y,
                    ky as isize - LANCZOS2_REACH,
                    source_cache.source_height,
                );
                for (kx, wx) in kernel.iter().enumerate() {
                    let x = clamp_offset_index(
                        center_x,
                        kx as isize - LANCZOS2_REACH,
                        source_cache.source_width,
                    );
                    let value = source_cache.sample(btd, x, y)?;
                    min_value = min_value.min(value);
                    max_value = max_value.max(value);
                    weighted_sum += (wx * wy) as f64 * value as f64;
                }
            }

            // Clamp to the footprint range so negative-lobe ringing cannot
            // overshoot past what the VHGT delta encode can represent.
            Ok((weighted_sum as f32).clamp(min_value, max_value))
        }
        // Starfield: the fixed-ratio-4 15-tap kernel above doesn't apply (the
        // source/target sample ratio is SF_SAMPLES_PER_FO4_INTERVAL, not 4) —
        // use the precomputed, variable-width taps instead. Separable: each
        // axis's tap set was built once in `SourceCellCache::new` from the
        // same `sf_frame` addressing `starfield_center_x/y` use elsewhere.
        SourceFrame::Starfield => {
            let tap_x = source_cache
                .axis_taps_x
                .as_ref()
                .expect("Starfield frame must have precomputed axis_taps_x")[target_x]
                .clone();
            let tap_y = source_cache
                .axis_taps_y
                .as_ref()
                .expect("Starfield frame must have precomputed axis_taps_y")[target_y]
                .clone();
            let mut weighted_sum = 0.0f64;
            let mut min_value = f32::INFINITY;
            let mut max_value = f32::NEG_INFINITY;

            for ky in 0..tap_y.count as usize {
                let y = clamp_absolute_index(tap_y.first + ky as i32, source_cache.source_height);
                let wy = tap_y.weights[ky];
                for kx in 0..tap_x.count as usize {
                    let x =
                        clamp_absolute_index(tap_x.first + kx as i32, source_cache.source_width);
                    let wx = tap_x.weights[kx];
                    let value = source_cache.sample(btd, x, y)?;
                    min_value = min_value.min(value);
                    max_value = max_value.max(value);
                    weighted_sum += (wx * wy) as f64 * value as f64;
                }
            }

            Ok((weighted_sum as f32).clamp(min_value, max_value))
        }
    }
}

#[cfg(test)]
fn quantize_target_grid_to_vhgt_lattice(grid: &TargetHeightGrid) -> TargetHeightGrid {
    let Some(&origin) = grid.values.first() else {
        return grid.clone();
    };
    TargetHeightGrid {
        width: grid.width,
        height: grid.height,
        values: quantize_cell_heights_to_lattice(&grid.values, origin),
    }
}

/// Quantize a single cell's heights to the same global VHGT lattice that
/// `quantize_target_grid_to_vhgt_lattice` applies to the whole grid. Callers
/// pass the global grid origin (`grid.values[0]`) so the lattice is identical
/// across every cell.
fn quantize_cell_heights_to_lattice(heights: &[f32], origin: f32) -> Vec<f32> {
    heights
        .iter()
        .map(|height| origin + ((*height - origin) / VHGT_HEIGHT_STEP).round() * VHGT_HEIGHT_STEP)
        .collect()
}

fn extract_target_cell_heights(
    grid: &TargetHeightGrid,
    cell_offset_x: usize,
    cell_offset_y: usize,
) -> Vec<f32> {
    let base_x = cell_offset_x * LAND_CELL_INTERVALS;
    let base_y = cell_offset_y * LAND_CELL_INTERVALS;
    let mut heights = Vec::with_capacity(LAND_CELL_VERTICES * LAND_CELL_VERTICES);
    for y in 0..LAND_CELL_VERTICES {
        for x in 0..LAND_CELL_VERTICES {
            heights.push(grid.get(base_x + x, base_y + y));
        }
    }
    heights
}

fn build_land_vertex_colors(
    btd: &mut BtdFile,
    cell_x: i32,
    cell_y: i32,
) -> Result<Vec<u8>, AuthoringEmitError> {
    // Starfield BTD carries no vertex-colour LOD4 section at all (see
    // btd.rs's is_starfield_layout parsing) — there is nothing to sample.
    if btd.header().is_starfield_layout {
        return Ok(starfield_neutral_vertex_colors());
    }
    let (max_x, max_y) = (btd.header().cell_max_x, btd.header().cell_max_y);
    let colors = crate::fo4_frame::assemble_cell_grid(
        |cx, cy| btd.cell_terrain_color_u16(cx.min(max_x), cy.min(max_y), 0),
        cell_x,
        cell_y,
        None,
    )?;
    let mut out = Vec::with_capacity(LAND_CELL_VERTICES * LAND_CELL_VERTICES * 3);
    for y in 0..LAND_CELL_VERTICES {
        let source_y = (y * 4).min(CELL_SOURCE_SAMPLES - 1);
        for x in 0..LAND_CELL_VERTICES {
            let source_x = (x * 4).min(CELL_SOURCE_SAMPLES - 1);
            out.extend_from_slice(&fo76_vclr_to_fo4_vclr(
                colors[source_y * CELL_SOURCE_SAMPLES + source_x],
            ));
        }
    }
    Ok(out)
}

/// Neutral (no-tint) vertex-colour fill for a Starfield-sourced cell.
/// `FO76_VCLR_NEUTRAL_SRGB_BYTE` is calibrated so a neutral FO76 input
/// saturates to output byte 255 through `fo76_vclr_channel_to_fo4_byte`
/// (its srgb=1.0 "no tint" case) — reuse that formula directly instead of
/// hard-coding 255, so the two stay derived from the same constant.
fn starfield_neutral_vertex_colors() -> Vec<u8> {
    let neutral = (255.0 * 255.0 / FO76_VCLR_NEUTRAL_SRGB_BYTE)
        .round()
        .clamp(0.0, 255.0) as u8;
    vec![neutral; LAND_CELL_VERTICES * LAND_CELL_VERTICES * 3]
}

fn fo76_vclr_to_fo4_vclr(value: u16) -> [u8; 3] {
    let r5 = ((value >> 10) & 0x1f) as u8;
    let g5 = ((value >> 5) & 0x1f) as u8;
    let b5 = (value & 0x1f) as u8;
    [
        fo76_vclr_channel_to_fo4_byte(r5),
        fo76_vclr_channel_to_fo4_byte(g5),
        fo76_vclr_channel_to_fo4_byte(b5),
    ]
}

fn fo76_vclr_channel_to_fo4_byte(value: u8) -> u8 {
    let linear = f32::from(value.min(31)) / 31.0;
    let srgb = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    ((srgb * 255.0 * 255.0) / FO76_VCLR_NEUTRAL_SRGB_BYTE)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn clamp_floor_index(value: f32, extent: usize) -> usize {
    value.floor().clamp(0.0, (extent - 1) as f32) as usize
}

fn clamp_ceil_index(value: f32, extent: usize) -> usize {
    value.ceil().clamp(0.0, (extent - 1) as f32) as usize
}

fn usize_to_i32(value: usize) -> Result<i32, AuthoringEmitError> {
    i32::try_from(value)
        .map_err(|_| AuthoringEmitError::Message("coordinate exceeds i32".to_string()))
}

#[cfg(test)]
fn clamp_vhgt_delta_stream(heights: &[f32]) -> Vec<f32> {
    clamp_vhgt_delta_stream_with_stats(heights).0
}

fn clamp_vhgt_delta_stream_with_stats(heights: &[f32]) -> (Vec<f32>, VhgtDeltaClampStats) {
    let Some((&first, rest)) = heights.split_first() else {
        return (Vec::new(), VhgtDeltaClampStats::default());
    };
    let mut result = Vec::with_capacity(heights.len());
    let mut stats = VhgtDeltaClampStats::default();
    result.push(first);
    let mut encoded_row_start = first;
    for (rest_index, height) in rest.iter().enumerate() {
        let index = rest_index + 1;
        let encoded_previous = if index % LAND_CELL_VERTICES == 0 {
            encoded_row_start
        } else {
            result[index - 1]
        };
        let unclamped_delta = ((*height - encoded_previous) / VHGT_HEIGHT_STEP).round();
        let delta = if unclamped_delta < VHGT_MIN_DELTA_STEP {
            stats.underflows = stats.underflows.saturating_add(1);
            VHGT_MIN_DELTA_STEP
        } else if unclamped_delta > VHGT_MAX_DELTA_STEP {
            stats.overflows = stats.overflows.saturating_add(1);
            VHGT_MAX_DELTA_STEP
        } else {
            unclamped_delta
        };
        let encodable = encoded_previous + delta * VHGT_HEIGHT_STEP;
        result.push(encodable);
        if index % LAND_CELL_VERTICES == 0 {
            encoded_row_start = encodable;
        }
    }
    (result, stats)
}

fn ensure_game_file(output_dir: &Path) -> Result<(), AuthoringEmitError> {
    if output_dir
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("yaml"))
    {
        if let Some(mod_root) = output_dir.parent() {
            fs::create_dir_all(mod_root)?;
            let game_path = mod_root.join(".game");
            if !game_path.exists() {
                fs::write(game_path, "fo4\n")?;
            }
        }
    }
    Ok(())
}

fn write_plugin_yaml(
    output: &mut AuthoringOutput,
    options: &ConvertOptions,
    next_object_id: u32,
) -> Result<(), AuthoringEmitError> {
    let payload = format!(
        "format_version: 1\nplugin: {}\ngame: fo4\nheader_size: 24\nheader:\n  version: 1.0\n  num_records: 0\n  next_object_id: '{}'\n  author: ''\n  description: ''\n  masters:\n    - Fallout4.esm\n  master_sizes:\n    - 0\n  overridden_forms: []\n  flags: []\n  version_control: 0\n  extra_subrecords: []\n",
        options.plugin_name,
        form_id_hex(next_object_id)
    );
    output.write_plugin_yaml(payload)
}

fn write_world_yaml(
    output: &mut AuthoringOutput,
    world_dir: &Path,
    options: &ConvertOptions,
    world_editor_id: &str,
    form_id: u32,
) -> Result<(), AuthoringEmitError> {
    let min_x_world = options.source_min_x as f32 * FO4_CELL_SIZE;
    let min_y_world = options.source_min_y as f32 * FO4_CELL_SIZE;
    let max_x_world = (options.source_max_x + 1) as f32 * FO4_CELL_SIZE;
    let max_y_world = (options.source_max_y + 1) as f32 * FO4_CELL_SIZE;
    let payload = format!(
        "signature: WRLD\nform_id: \"{}:{}\"\nform_version: 131\nversion2: 1\neid: {}\nsubrecords:\n  - signature: EDID\n    data_hex: \"{}\"\n  - signature: NAMA\n    data_hex: \"{}\"\n  - signature: DATA\n    data_hex: \"00\"\n  - signature: NAM0\n    data_hex: \"{}{}\"\n  - signature: NAM9\n    data_hex: \"{}{}\"\n",
        form_id_hex(form_id),
        options.plugin_name,
        world_editor_id,
        zstring_hex(world_editor_id),
        f32_hex(1.0),
        f32_hex(min_x_world),
        f32_hex(min_y_world),
        f32_hex(max_x_world),
        f32_hex(max_y_world)
    );
    output.write_record_yaml("WRLD", world_dir.join("RecordData.yaml"), payload)
}

fn write_texture_records(
    output: &mut AuthoringOutput,
    options: &ConvertOptions,
    textures: &[EmittedTexture],
    worldspace_editor_id: &str,
) -> Result<(), AuthoringEmitError> {
    if textures.is_empty() {
        return Ok(());
    }
    let txst_dir = PathBuf::from("records").join("TXST");
    let ltex_dir = PathBuf::from("records").join("LTEX");
    let gras_dir = PathBuf::from("records").join("GRAS");
    let mut written_txst_object_ids = HashSet::new();
    let mut written_grass_object_ids = HashSet::new();
    for texture in textures {
        let txst_eid = preserved_or_generated_editor_id(
            options.preserve_source_ids,
            &texture.converted.source_txst_editor_id,
            || format!("{}_TXST_{}", worldspace_editor_id, texture.converted.suffix),
        );
        let ltex_eid = ltex_editor_id(
            options.preserve_source_ids,
            worldspace_editor_id,
            &texture.converted,
        );
        let txst_payload = written_txst_object_ids
            .insert(texture.txst_object_id)
            .then(|| {
                format!(
                    // No Glow slot: the FO76 remix derives a `_g` glow from the
                    // lighting map's alpha, but the FO4 landscape shader has no glow
                    // permutation and the quad renders black. Vanilla terrain TXSTs
                    // use Diffuse + NormalGloss + SmoothSpec only, with NoSpecularMap.
                    "form_id: \"{}\"\nform_version: 131\nversion2: 1\neid: {}\nfields:\n- ObjectBounds:\n    ObjectBoundsX1: -8\n    ObjectBoundsY1: -30\n    ObjectBoundsZ1: -20\n    ObjectBoundsX2: 7\n    ObjectBoundsY2: 30\n    ObjectBoundsZ2: 20\n- TexturesRgbAs:\n  - Diffuse: {}\n    NormalGloss: {}\n    SmoothSpec: {}\n- Flags:\n  - NoSpecularMap\n",
                    form_id_hex(texture.txst_object_id),
                    txst_eid,
                    yaml_quote(&texture_slot_path(&texture.converted.diffuse_rel_path)),
                    yaml_quote(&texture_slot_path(&texture.converted.normal_rel_path)),
                    yaml_quote(&texture_slot_path(&texture.converted.specgloss_rel_path))
                )
            });
        let material_type_payload = texture
            .converted
            .material_type_object_id
            .as_ref()
            .map(|object_id| {
                format!(
                    "- MaterialType:\n    reference:\n      plugin: Fallout4.esm\n      object_id: \"{}\"\n",
                    object_id
                )
            })
            .unwrap_or_default();
        let grass_payload = if options.btd4_output_path.is_empty() {
            texture
                .grass_object_ids
                .iter()
                .map(|object_id| {
                    format!(
                        "- Grass:\n    reference:\n      plugin: {}\n      object_id: \"{}\"\n",
                        options.plugin_name,
                        form_id_hex(*object_id)
                    )
                })
                .collect::<String>()
        } else {
            String::new()
        };
        let ltex_payload = format!(
            "form_id: \"{}\"\nform_version: 131\nversion2: 1\neid: {}\nfields:\n- TextureSet:\n    reference:\n      plugin: {}\n      object_id: \"{}\"\n{}- HavokData:\n    Friction: {}\n    Restitution: {}\n- TextureSpecularExponent: 30\n{}",
            form_id_hex(texture.ltex_object_id),
            ltex_eid,
            options.plugin_name,
            form_id_hex(texture.txst_object_id),
            material_type_payload,
            texture.converted.havok_friction,
            texture.converted.havok_restitution,
            grass_payload
        );
        for (grass, object_id) in texture
            .converted
            .grass
            .iter()
            .zip(texture.grass_object_ids.iter())
        {
            if !written_grass_object_ids.insert(*object_id) {
                continue;
            }
            let grass_eid = preserved_or_generated_editor_id(
                options.preserve_source_ids,
                &grass.source_editor_id,
                || grass_editor_id(worldspace_editor_id, &texture.converted.suffix, grass),
            );
            output.write_record_yaml(
                "GRAS",
                gras_dir.join(format!(
                    "{} - {}_{}.yaml",
                    grass_eid,
                    form_id_hex(*object_id),
                    options.plugin_name
                )),
                grass_payload_yaml(&grass_eid, *object_id, grass),
            )?;
        }
        if let Some(txst_payload) = txst_payload {
            output.write_record_yaml(
                "TXST",
                txst_dir.join(format!(
                    "{} - {}_{}.yaml",
                    txst_eid,
                    form_id_hex(texture.txst_object_id),
                    options.plugin_name
                )),
                txst_payload,
            )?;
        }
        output.write_record_yaml(
            "LTEX",
            ltex_dir.join(format!(
                "{} - {}_{}.yaml",
                ltex_eid,
                form_id_hex(texture.ltex_object_id),
                options.plugin_name
            )),
            ltex_payload,
        )?;
    }
    Ok(())
}

fn ltex_editor_id(
    preserve_source_ids: bool,
    worldspace_editor_id: &str,
    texture: &ConvertedTerrainTexture,
) -> String {
    let base = preserved_or_generated_editor_id(
        preserve_source_ids,
        &texture.source_ltex_editor_id,
        || format!("{}_LTEX_{}", worldspace_editor_id, texture.suffix),
    );
    if texture.source_gcvr_form_key.is_none() {
        return base;
    }
    let gcvr = texture
        .source_gcvr_editor_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            texture
                .source_gcvr_form_key
                .as_deref()
                .and_then(object_id_from_source_form_key)
                .map(|object_id| format!("GCVR_{}", form_id_hex(object_id)))
        })
        .unwrap_or_else(|| "GCVR".to_owned());
    format!(
        "{}_GC_{}",
        safe_editor_token(&base),
        safe_editor_token(&gcvr)
    )
}

fn preserved_or_generated_editor_id(
    preserve_source_ids: bool,
    source_editor_id: &str,
    generated: impl FnOnce() -> String,
) -> String {
    let trimmed = source_editor_id.trim();
    if preserve_source_ids && !trimmed.is_empty() {
        trimmed.to_string()
    } else {
        generated()
    }
}

fn grass_editor_id(
    worldspace_editor_id: &str,
    texture_suffix: &str,
    grass: &ConvertedTerrainGrass,
) -> String {
    format!(
        "{}_GRAS_{}_{}",
        worldspace_editor_id,
        safe_editor_token(texture_suffix),
        safe_editor_token(&grass.source_editor_id)
    )
}

fn grass_payload_yaml(eid: &str, object_id: u32, grass: &ConvertedTerrainGrass) -> String {
    let bounds = &grass.object_bounds;
    let model_file_name = grass.model_file_name.replace('/', "\\");
    let flags = if grass.flags.is_empty() {
        "    Flags: []\n".to_string()
    } else {
        format!(
            "    Flags:\n{}",
            grass
                .flags
                .iter()
                .map(|flag| format!("    - {}\n", flag))
                .collect::<String>()
        )
    };
    format!(
        "form_id: \"{}\"\nform_version: 131\nversion2: 6\neid: {}\nfields:\n- ObjectBounds:\n    ObjectBoundsX1: {}\n    ObjectBoundsY1: {}\n    ObjectBoundsZ1: {}\n    ObjectBoundsX2: {}\n    ObjectBoundsY2: {}\n    ObjectBoundsZ2: {}\n- ModelFileName: {}\n- ModelInformation:\n    raw_hex: {}\n- DATA:\n    Density: {}\n    MaxSlope: {}\n    PositionRange: {}\n    HeightRange: {}\n    ColorRange: {}\n    WavePeriod: {}\n{}",
        form_id_hex(object_id),
        eid,
        bounds.x1,
        bounds.y1,
        bounds.z1,
        bounds.x2,
        bounds.y2,
        bounds.z2,
        yaml_quote(&model_file_name),
        yaml_quote(&grass.model_information),
        grass.density,
        grass.max_slope,
        grass.position_range,
        grass.height_range,
        grass.color_range,
        grass.wave_period,
        flags
    )
}

fn write_cell_yaml(
    output: &mut AuthoringOutput,
    cell_dir: &Path,
    options: &ConvertOptions,
    cell_x: i32,
    cell_y: i32,
    cell_eid: Option<&str>,
    cell_form_id: u32,
    land_form_id: u32,
    vnml: &[u8],
    vhgt: &[u8],
    vclr: &[u8],
    has_vertex_colors: bool,
    texture_fields: &[LandTextureField],
    water_cell: Option<&WaterCell>,
) -> Result<(), AuthoringEmitError> {
    if output.uses_structured_cell_sink() {
        let value = cell_record_value(
            options,
            cell_x,
            cell_y,
            cell_eid,
            cell_form_id,
            land_form_id,
            vnml,
            vhgt,
            vclr,
            has_vertex_colors,
            texture_fields,
            water_cell,
        );
        return output.write_record_value("CELL", cell_dir.join("RecordData.yaml"), value);
    }
    let xclc_hex = format!("{}{}00000000", i32_hex(cell_x), i32_hex(cell_y));
    // FO4 exterior cells always have DATA bit 1 (has_water) set — vanilla
    // Commonwealth/DLC cells all carry this flag whether or not a per-cell
    // water override exists. Without it CK adds the flag on first save and
    // the cell's render-init pass fails before grass spawns.
    let cell_data_flags = u16_hex(2);
    let (xclw_hex, xcwt_subrecord) = match water_cell {
        Some(water) => (
            f32_hex(water.height),
            format!(
                "  - signature: XCWT\n    data_hex: \"{}\"\n",
                u32_hex(master_form_id(water.water_object_id))
            ),
        ),
        // FFFF7F7F is f32 ≈ 3.4028235e38 — the "use worldspace default water
        // height" sentinel vanilla FO4 cells use when they have no override.
        None => ("FFFF7F7F".to_string(), String::new()),
    };
    let land_data_flags = land_data_flags(!texture_fields.is_empty(), has_vertex_colors);
    let mut land_fields = format!(
        "    - Flags: {}\n    - VertexNormals:\n        raw_hex: \"{}\"\n    - VertexHeightMap:\n        raw_hex: \"{}\"\n",
        land_data_flags,
        bytes_hex(vnml),
        bytes_hex(vhgt)
    );
    if !vclr.is_empty() {
        land_fields.push_str(&format!(
            "    - VertexColors:\n        raw_hex: \"{}\"\n",
            bytes_hex(vclr)
        ));
    }
    for field in texture_fields {
        land_fields.push_str(&field.yaml());
    }
    let editor_id_fields = cell_eid
        .map(|eid| {
            format!(
                "eid: {eid}\nsubrecords:\n  - signature: EDID\n    data_hex: \"{}\"\n",
                zstring_hex(eid)
            )
        })
        .unwrap_or_else(|| "subrecords:\n".to_string());

    let payload = format!(
        "signature: CELL\nform_id: \"{}:{}\"\nform_version: 131\nversion2: 1\n{}  - signature: DATA\n    data_hex: \"{}\"\n  - signature: XCLC\n    data_hex: \"{}\"\n  - signature: XCLW\n    data_hex: \"{}\"\n{}Landscape:\n  signature: LAND\n  form_id: \"{}:{}\"\n  form_version: 131\n  version2: 1\n  fields:\n{}",
        form_id_hex(cell_form_id),
        options.plugin_name,
        editor_id_fields,
        cell_data_flags,
        xclc_hex,
        xclw_hex,
        xcwt_subrecord,
        form_id_hex(land_form_id),
        options.plugin_name,
        land_fields
    );
    output.write_record_yaml("CELL", cell_dir.join("RecordData.yaml"), payload)
}

#[allow(clippy::too_many_arguments)]
fn cell_record_value(
    options: &ConvertOptions,
    cell_x: i32,
    cell_y: i32,
    cell_eid: Option<&str>,
    cell_form_id: u32,
    land_form_id: u32,
    vnml: &[u8],
    vhgt: &[u8],
    vclr: &[u8],
    has_vertex_colors: bool,
    texture_fields: &[LandTextureField],
    water_cell: Option<&WaterCell>,
) -> serde_json::Value {
    let mut subrecords = Vec::with_capacity(5);
    if let Some(eid) = cell_eid {
        subrecords.push(serde_json::json!({
            "signature": "EDID",
            "data_hex": zstring_hex(eid),
        }));
    }
    subrecords.push(serde_json::json!({
        "signature": "DATA",
        "data_hex": u16_hex(2),
    }));
    subrecords.push(serde_json::json!({
        "signature": "XCLC",
        "data_hex": format!("{}{}00000000", i32_hex(cell_x), i32_hex(cell_y)),
    }));
    subrecords.push(serde_json::json!({
        "signature": "XCLW",
        "data_hex": water_cell
            .map(|water| f32_hex(water.height))
            .unwrap_or_else(|| "FFFF7F7F".to_owned()),
    }));
    if let Some(water) = water_cell {
        subrecords.push(serde_json::json!({
            "signature": "XCWT",
            "data_hex": u32_hex(master_form_id(water.water_object_id)),
        }));
    }

    let mut land_fields = vec![
        serde_json::json!({
            "Flags": land_data_flags(!texture_fields.is_empty(), has_vertex_colors),
        }),
        serde_json::json!({
            "VertexNormals": { "raw_hex": bytes_hex(vnml) },
        }),
        serde_json::json!({
            "VertexHeightMap": { "raw_hex": bytes_hex(vhgt) },
        }),
    ];
    if !vclr.is_empty() {
        land_fields.push(serde_json::json!({
            "VertexColors": { "raw_hex": bytes_hex(vclr) },
        }));
    }
    land_fields.extend(texture_fields.iter().map(LandTextureField::value));

    let mut cell = serde_json::Map::new();
    cell.insert("signature".to_owned(), serde_json::json!("CELL"));
    cell.insert(
        "form_id".to_owned(),
        serde_json::json!(format!(
            "{}:{}",
            form_id_hex(cell_form_id),
            options.plugin_name
        )),
    );
    cell.insert("form_version".to_owned(), serde_json::json!(131));
    cell.insert("version2".to_owned(), serde_json::json!(1));
    if let Some(eid) = cell_eid {
        cell.insert("eid".to_owned(), serde_json::json!(eid));
    }
    cell.insert(
        "subrecords".to_owned(),
        serde_json::Value::Array(subrecords),
    );
    cell.insert(
        "Landscape".to_owned(),
        serde_json::json!({
            "signature": "LAND",
            "form_id": format!("{}:{}", form_id_hex(land_form_id), options.plugin_name),
            "form_version": 131,
            "version2": 1,
            "fields": land_fields,
        }),
    );
    serde_json::Value::Object(cell)
}

fn land_data_flags(has_layers: bool, has_vertex_colors: bool) -> u32 {
    let mut flags =
        LAND_FLAG_HAS_VERTEX_NORMALS_HEIGHT_MAP | LAND_FLAG_UNKNOWN_4 | LAND_FLAG_AUTO_CALC_NORMALS;
    if has_vertex_colors {
        flags |= LAND_FLAG_HAS_VERTEX_COLORS;
    }
    if has_layers {
        flags |= LAND_FLAG_HAS_LAYERS;
    }
    flags
}

/// One grass type's stencil-derived placements for a cell: the grass NIF path, its
/// GRAS object id, and the world positions where the FO76 128x128 ground-cover
/// stencil enabled it. Currently unused; kept (with [`collect_cell_grass_instances`])
/// as the position source for a grass sidecar / SCOL emitter.
#[cfg(test)]
#[allow(dead_code)]
struct CellGrassType {
    name: String,
    gras_object_id: u32,
    instances: Vec<GrassInstance>,
}

#[cfg(test)]
#[allow(dead_code)]
struct GrassInstance {
    x: f32,
    y: f32,
    z: f32,
}

/// Grass placement from the FO76 BTD 128x128 ground-cover stencil instead of
/// FO4's texture-% scatter: one instance per set mask bit (stride-thinned),
/// grouped by GRAS object id. Z is bilinear from the cell's LAND heights. XY
/// uses the FO4 cell origin directly, with no +2048 placed-record offset.
#[cfg(test)]
#[allow(dead_code)]
fn collect_cell_grass_instances(
    btd: &BtdFile,
    cell_x: i32,
    cell_y: i32,
    textures_by_source_usage: &HashMap<SourceTextureUsageKey, &EmittedTexture>,
    heights: &[f32],
    stride: usize,
) -> Result<Vec<CellGrassType>, AuthoringEmitError> {
    let max_x = btd.header().cell_max_x;
    let max_y = btd.header().cell_max_y;
    let set = crate::fo4_frame::assemble_cell_texture_set(btd, cell_x, cell_y)?;
    let mask = crate::fo4_frame::assemble_cell_grid(
        |cx, cy| btd.cell_ground_cover_mask_u8(cx.min(max_x), cy.min(max_y), 0),
        cell_x,
        cell_y,
        None,
    )?;
    let layers = map_cell_layers(&set);

    let mut types: Vec<CellGrassType> = Vec::new();
    let mut index_by_object_id: HashMap<u32, usize> = HashMap::new();

    for base in &layers.base_layers {
        accumulate_grass_for_layer(
            btd,
            &mask,
            heights,
            cell_x,
            cell_y,
            base.quadrant,
            base.source_slot,
            base.texture_index,
            base.ground_cover_index,
            textures_by_source_usage,
            stride,
            &mut types,
            &mut index_by_object_id,
        );
    }
    for alpha in &layers.alpha_layers {
        accumulate_grass_for_layer(
            btd,
            &mask,
            heights,
            cell_x,
            cell_y,
            alpha.quadrant,
            alpha.source_slot,
            alpha.texture_index,
            alpha.ground_cover_index,
            textures_by_source_usage,
            stride,
            &mut types,
            &mut index_by_object_id,
        );
    }
    Ok(types)
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
#[allow(dead_code)]
fn accumulate_grass_for_layer(
    btd: &BtdFile,
    mask: &[u8],
    heights: &[f32],
    cell_x: i32,
    cell_y: i32,
    quadrant: u8,
    source_slot: Option<u8>,
    texture_index: u8,
    ground_cover_index: Option<u8>,
    textures_by_source_usage: &HashMap<SourceTextureUsageKey, &EmittedTexture>,
    stride: usize,
    types: &mut Vec<CellGrassType>,
    index_by_object_id: &mut HashMap<u32, usize>,
) {
    let (Some(slot), Some(gci)) = (source_slot, ground_cover_index) else {
        return;
    };
    let Some(bit) = ground_cover_mask_bit_for_source_slot(slot) else {
        return;
    };
    let Some(texture) = texture_for_layer(btd, texture_index, &[gci], textures_by_source_usage)
    else {
        return;
    };
    if texture.converted.grass.is_empty() {
        return;
    }

    let quadrant_x = usize::from(quadrant & 1);
    let quadrant_y = usize::from((quadrant >> 1) & 1);
    let origin_x = quadrant_x * CELL_SOURCE_QUADRANT_SAMPLES;
    let origin_y = quadrant_y * CELL_SOURCE_QUADRANT_SAMPLES;
    let sample_size = FO4_CELL_SIZE / CELL_SOURCE_SAMPLES as f32;
    let mut round_robin = 0usize;

    for sample_y in (origin_y..origin_y + CELL_SOURCE_QUADRANT_SAMPLES).step_by(stride) {
        for sample_x in (origin_x..origin_x + CELL_SOURCE_QUADRANT_SAMPLES).step_by(stride) {
            if mask
                .get(sample_y * CELL_SOURCE_SAMPLES + sample_x)
                .copied()
                .unwrap_or(0)
                & bit
                == 0
            {
                continue;
            }
            // A ground cover may resolve to several grass NIFs (grass + flowers
            // etc.); spread them across samples rather than stacking all at once.
            let grass_index = round_robin % texture.converted.grass.len();
            round_robin += 1;
            let grass = &texture.converted.grass[grass_index];
            let object_id = texture
                .grass_object_ids
                .get(grass_index)
                .copied()
                .unwrap_or(0);

            let world_x = cell_x as f32 * FO4_CELL_SIZE + (sample_x as f32 + 0.5) * sample_size;
            let world_y = cell_y as f32 * FO4_CELL_SIZE + (sample_y as f32 + 0.5) * sample_size;
            let world_z = interpolate_cell_height(heights, sample_x, sample_y);

            let slot_index = *index_by_object_id.entry(object_id).or_insert_with(|| {
                types.push(CellGrassType {
                    name: grass.model_file_name.clone(),
                    gras_object_id: object_id,
                    instances: Vec::new(),
                });
                types.len() - 1
            });
            types[slot_index].instances.push(GrassInstance {
                x: world_x,
                y: world_y,
                z: world_z,
            });
        }
    }
}

/// Bilinear-sample the 33x33 cell LAND heights at a 128x128 ground-cover sample.
#[cfg(test)]
#[allow(dead_code)]
fn interpolate_cell_height(heights: &[f32], sample_x: usize, sample_y: usize) -> f32 {
    let grid_x = (sample_x as f32 + 0.5) / CELL_SOURCE_SAMPLES as f32 * LAND_CELL_INTERVALS as f32;
    let grid_y = (sample_y as f32 + 0.5) / CELL_SOURCE_SAMPLES as f32 * LAND_CELL_INTERVALS as f32;
    let x0 = clamp_floor_index(grid_x, LAND_CELL_VERTICES);
    let y0 = clamp_floor_index(grid_y, LAND_CELL_VERTICES);
    let x1 = (x0 + 1).min(LAND_CELL_VERTICES - 1);
    let y1 = (y0 + 1).min(LAND_CELL_VERTICES - 1);
    let tx = (grid_x - x0 as f32).clamp(0.0, 1.0);
    let ty = (grid_y - y0 as f32).clamp(0.0, 1.0);
    let at = |x: usize, y: usize| heights[y * LAND_CELL_VERTICES + x];
    let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
    let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
    top * (1.0 - ty) + bottom * ty
}

fn build_land_texture_fields(
    cell_x: i32,
    cell_y: i32,
    global_blend: &GlobalLandscapeBlend,
    textures_by_source_usage: &HashMap<SourceTextureUsageKey, &EmittedTexture>,
    plugin_name: &str,
    dropped_texture_layers: &mut u32,
    _want_dense_alpha: bool,
    _land_skip_ground_cover_variants: bool,
) -> Result<LandTextureFields, AuthoringEmitError> {
    if textures_by_source_usage.is_empty() {
        return Ok(LandTextureFields {
            fields: Vec::new(),
            layer_count: 0,
            ground_cover_layer_count: 0,
            no_ground_cover_layer_count: 0,
            btd4_layer_object_ids: vec![None; 24],
            btd4_source_layer_object_ids: vec![None; 24],
        });
    }

    let mut fields = Vec::new();
    let mut layer_count = 0u32;
    let mut ground_cover_layer_count = 0u32;
    let mut no_ground_cover_layer_count = 0u32;
    let mut btd4_layer_object_ids = vec![None; 24];
    let mut btd4_source_layer_object_ids = vec![None; 24];

    for quadrant in 0..4u8 {
        let Some(quadrant_blend) = global_blend
            .serialize_quadrant(cell_x, cell_y, quadrant)
            .map_err(AuthoringEmitError::Message)?
        else {
            continue;
        };
        *dropped_texture_layers = dropped_texture_layers.saturating_add(
            u32::try_from(quadrant_blend.dropped_source_ltex_object_ids.len()).map_err(|_| {
                AuthoringEmitError::Message("dropped texture layer count exceeds u32".to_string())
            })?,
        );

        let base_texture = texture_for_source_ltex_object_id(
            quadrant_blend.base_source_ltex_object_id,
            textures_by_source_usage,
        )
        .ok_or_else(|| {
            AuthoringEmitError::Message(format!(
                "missing converted plain LTEX for source {:06X} used as BTXT in cell ({cell_x},{cell_y}) quadrant {quadrant}",
                quadrant_blend.base_source_ltex_object_id
            ))
        })?;
        count_texture_layer_ground_cover(
            false,
            &mut ground_cover_layer_count,
            &mut no_ground_cover_layer_count,
        );
        fields.push(LandTextureField::Layer {
            signature: "BTXT",
            texture_object_id: base_texture.ltex_object_id,
            plugin_name: plugin_name.to_owned(),
            quadrant,
            layer: -1,
        });
        layer_count = layer_count.saturating_add(1);
        let btd4_base = quadrant as usize * 6;
        btd4_layer_object_ids[btd4_base] = Some(base_texture.ltex_object_id);
        btd4_source_layer_object_ids[btd4_base] = Some(quadrant_blend.base_source_ltex_object_id);

        let mut next_alpha_slot = 0usize;
        for (source_ltex_object_id, vtxt) in quadrant_blend
            .alpha_source_ltex_object_ids
            .iter()
            .copied()
            .zip(quadrant_blend.alpha_vtxt.iter())
        {
            if vtxt.is_empty() {
                continue;
            }
            let texture =
                texture_for_source_ltex_object_id(source_ltex_object_id, textures_by_source_usage)
                    .ok_or_else(|| {
                        AuthoringEmitError::Message(format!(
                            "missing converted plain LTEX for source {source_ltex_object_id:06X} used as ATXT in cell ({cell_x},{cell_y}) quadrant {quadrant}",
                        ))
                    })?;
            let new_slot = next_alpha_slot;
            count_texture_layer_ground_cover(
                false,
                &mut ground_cover_layer_count,
                &mut no_ground_cover_layer_count,
            );
            fields.push(LandTextureField::Layer {
                signature: "ATXT",
                texture_object_id: texture.ltex_object_id,
                plugin_name: plugin_name.to_owned(),
                quadrant,
                layer: new_slot as i16,
            });
            fields.push(LandTextureField::AlphaLayerData {
                raw_hex: bytes_hex(vtxt),
            });
            layer_count = layer_count.saturating_add(1);
            if new_slot < 5 {
                btd4_layer_object_ids[btd4_base + 1 + new_slot] = Some(texture.ltex_object_id);
                btd4_source_layer_object_ids[btd4_base + 1 + new_slot] =
                    Some(source_ltex_object_id);
            }
            next_alpha_slot += 1;
        }
    }

    Ok(LandTextureFields {
        fields,
        layer_count,
        ground_cover_layer_count,
        no_ground_cover_layer_count,
        btd4_layer_object_ids,
        btd4_source_layer_object_ids,
    })
}

/// Per-cell channel gather for the `.btd4` dense sidecar.
///
/// - HGTS: 129x129 raw u16 via `SourceCellCache::sample_raw_u16`, sharing the
///   +HALF_CELL_SAMPLES shift and clamping with the f32 LAND heights (the LAND's
///   33x33 verts are the {0,4,...,128} subset). The +1 edge row/column reads into
///   the next source cell, which the extra-cell growth covers.
/// - LAYR: the 24-slot table (4 quadrants × base + 5 alpha) from
///   `build_land_texture_fields`; `kind = 0` (LTEX), `plugin_index = 0` (the
///   producer) or `u8::MAX` for an empty slot.
/// - GCVR: one 128x128 mask per producer GRAS object id; errors past FO4's 16
///   grass types per LAND interval.
/// - ALPH: planes in the LAND's per-quadrant ATXT slot order, since the engine
///   reuses the vanilla LAND material for the dense terrain; `None` without
///   alpha layers.
/// - CLRS: 129x129 RGB converted from the BTD terrain colors.
///
/// `layers_recovered` is always 0.
struct Btd4CellGather {
    channels: crate::btd4::CellChannels,
    layers_recovered: u32,
}

#[allow(clippy::too_many_arguments)]
fn gather_btd4_cell_channels(
    btd: &mut BtdFile,
    cell_x: i32,
    cell_y: i32,
    target_cell_offset_x: usize,
    target_cell_offset_y: usize,
    layer_object_ids: &[Option<u32>],
    source_layer_object_ids: &[Option<u32>],
    grass_object_ids_by_gcvr: &HashMap<u32, Vec<u32>>,
    source_alpha_masks: &TextureAlphaMasks,
    source_cache: &mut SourceCellCache,
) -> Result<Btd4CellGather, AuthoringEmitError> {
    let mut heights = Vec::with_capacity(129 * 129);
    for vy in 0..=128usize {
        for vx in 0..=128usize {
            let gx = target_cell_offset_x * CELL_SOURCE_SAMPLES + vx;
            let gy = target_cell_offset_y * CELL_SOURCE_SAMPLES + vy;
            heights.push(source_cache.sample_raw_u16(btd, gx, gy)?);
        }
    }

    if layer_object_ids.len() != 24 || source_layer_object_ids.len() != 24 {
        return Err(AuthoringEmitError::Message(format!(
            "cell ({cell_x},{cell_y}) did not produce the 24-slot BTD4 material table"
        )));
    }
    let layers: Vec<crate::btd4::LayerRef> = layer_object_ids
        .iter()
        .map(|object_id| crate::btd4::LayerRef {
            plugin_index: if object_id.is_some() { 0 } else { u8::MAX },
            object_id: object_id.unwrap_or(0),
            kind: 0,
        })
        .collect();

    let dense_texture_ids = dense_cell_texture_object_ids(btd, cell_x, cell_y, source_alpha_masks)?;
    let dense_alpha = dense_alpha_planes(&dense_texture_ids, source_layer_object_ids);
    let gcvr = dense_gcvr_entries(btd, cell_x, cell_y, grass_object_ids_by_gcvr)?;
    let colors = dense_cell_colors(btd, cell_x, cell_y)?;

    Ok(Btd4CellGather {
        channels: crate::btd4::CellChannels {
            heights: Some(heights),
            alphas: dense_alpha,
            layers: Some(layers),
            gcvr,
            colors: Some(colors),
        },
        layers_recovered: 0,
    })
}

fn dense_cell_texture_object_ids(
    btd: &mut BtdFile,
    cell_x: i32,
    cell_y: i32,
    alpha_lookup: &impl SourceAlphaLookup,
) -> Result<Vec<Option<u32>>, AuthoringEmitError> {
    let header = btd.header();
    let (min_x, min_y, max_x, max_y) = (
        header.cell_min_x,
        header.cell_min_y,
        header.cell_max_x,
        header.cell_max_y,
    );
    let is_starfield = header.is_starfield_layout;
    let starfield_btd_cell_min = is_starfield.then_some((header.cell_min_x, header.cell_min_y));
    let textures = crate::fo4_frame::assemble_cell_texture_set(btd, cell_x, cell_y)?;
    let packed = crate::fo4_frame::assemble_cell_grid(
        |cx, cy| btd.cell_land_alpha_u16(cx.clamp(min_x, max_x), cy.clamp(min_y, max_y), 0),
        cell_x,
        cell_y,
        starfield_btd_cell_min,
    )?;
    let mut result = vec![None; CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES];
    for y in 0..CELL_SOURCE_SAMPLES {
        for x in 0..CELL_SOURCE_SAMPLES {
            let quadrant = (x / 64) | ((y / 64) << 1);
            let quad = &textures.quadrants[quadrant];
            let alpha = packed[y * CELL_SOURCE_SAMPLES + x];
            // Starfield: the true global source sample index via sf_frame,
            // not the FO76 half-cell-shifted `cell*128+64+x`.
            let (source_world_x, source_world_y) = if is_starfield {
                (
                    crate::fo4_frame::starfield_global_sample_index(cell_x, x, min_x),
                    crate::fo4_frame::starfield_global_sample_index(cell_y, y, min_y),
                )
            } else {
                (
                    cell_x * CELL_SOURCE_SAMPLES as i32
                        + crate::fo4_frame::HALF_CELL_SAMPLES as i32
                        + x as i32,
                    cell_y * CELL_SOURCE_SAMPLES as i32
                        + crate::fo4_frame::HALF_CELL_SAMPLES as i32
                        + y as i32,
                )
            };
            let u = source_world_x - source_world_y;
            let v = source_world_x + source_world_y;
            let mut selected = None;
            for slot in (0..5usize).rev() {
                let layer_value = ((alpha >> (slot * 3)) & 0x7) as u8;
                let Some(texture_index) = quad.additional[slot] else {
                    continue;
                };
                let Some(source_ltex) = btd
                    .land_texture_form_id(texture_index as usize)
                    .map(|form_id| form_id & 0x00FF_FFFF)
                else {
                    continue;
                };
                if fo76_layer_alpha_passes(
                    layer_value,
                    alpha_lookup.sample_alpha(source_ltex, u, v),
                ) {
                    selected = Some(source_ltex);
                    break;
                }
            }
            if selected.is_none() {
                selected = quad.base.and_then(|texture_index| {
                    btd.land_texture_form_id(texture_index as usize)
                        .map(|form_id| form_id & 0x00FF_FFFF)
                });
            }
            result[y * CELL_SOURCE_SAMPLES + x] = selected;
        }
    }
    Ok(result)
}

fn dense_alpha_planes(
    texture_ids: &[Option<u32>],
    ordered_source_layers: &[Option<u32>],
) -> Option<Vec<Vec<u8>>> {
    let has_alpha = (0..4usize).any(|quadrant| {
        ordered_source_layers[quadrant * 6 + 1..quadrant * 6 + 6]
            .iter()
            .any(Option::is_some)
    });
    if !has_alpha {
        return None;
    }
    let mut planes = vec![vec![0u8; crate::btd4::ALPH_PLANE_LEN]; crate::btd4::ALPH_PLANE_COUNT];
    for quadrant in 0..4usize {
        let qx = quadrant & 1;
        let qy = quadrant >> 1;
        for y in 0..crate::btd4::ALPH_PLANE_VERTS {
            for x in 0..crate::btd4::ALPH_PLANE_VERTS {
                let sample_x = qx * 64 + x.min(63);
                let sample_y = qy * 64 + y.min(63);
                let selected = texture_ids[sample_y * CELL_SOURCE_SAMPLES + sample_x];
                for slot in 0..5usize {
                    if selected.is_some()
                        && selected == ordered_source_layers[quadrant * 6 + 1 + slot]
                    {
                        planes[quadrant * 5 + slot][y * crate::btd4::ALPH_PLANE_VERTS + x] =
                            u8::MAX;
                    }
                }
            }
        }
    }
    Some(planes)
}

fn dense_gcvr_entries(
    btd: &BtdFile,
    cell_x: i32,
    cell_y: i32,
    grass_object_ids_by_gcvr: &HashMap<u32, Vec<u32>>,
) -> Result<Option<crate::btd4::GcvrChunk>, AuthoringEmitError> {
    let header = btd.header();
    let (min_x, min_y, max_x, max_y) = (
        header.cell_min_x,
        header.cell_min_y,
        header.cell_max_x,
        header.cell_max_y,
    );
    let starfield_btd_cell_min = header
        .is_starfield_layout
        .then_some((header.cell_min_x, header.cell_min_y));
    let textures = crate::fo4_frame::assemble_cell_texture_set(btd, cell_x, cell_y)?;
    let packed = crate::fo4_frame::assemble_cell_grid(
        |cx, cy| btd.cell_ground_cover_mask_u8(cx.clamp(min_x, max_x), cy.clamp(min_y, max_y), 0),
        cell_x,
        cell_y,
        starfield_btd_cell_min,
    )?;
    let mut masks = std::collections::BTreeMap::<u32, Vec<u8>>::new();
    for y in 0..CELL_SOURCE_SAMPLES {
        for x in 0..CELL_SOURCE_SAMPLES {
            let value = packed[y * CELL_SOURCE_SAMPLES + x];
            if value == 0 {
                continue;
            }
            let quadrant = (x / 64) | ((y / 64) << 1);
            let quad = &textures.quadrants[quadrant];
            for source_slot in 0..8usize {
                let bit = 1u8 << (7 - source_slot);
                if value & bit == 0 {
                    continue;
                }
                let Some(gcvr_index) = quad.ground_cover[source_slot] else {
                    continue;
                };
                let Some(source_gcvr) = btd
                    .ground_cover_form_id(gcvr_index as usize)
                    .map(|form_id| form_id & 0x00FF_FFFF)
                else {
                    continue;
                };
                let Some(grass_ids) = grass_object_ids_by_gcvr.get(&source_gcvr) else {
                    continue;
                };
                for object_id in grass_ids {
                    masks
                        .entry(*object_id)
                        .or_insert_with(|| vec![0; CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES])
                        [y * CELL_SOURCE_SAMPLES + x] = u8::MAX;
                }
            }
        }
    }
    for interval_y in 0..32usize {
        for interval_x in 0..32usize {
            let covering = masks
                .values()
                .filter(|mask| {
                    (interval_y * 4..interval_y * 4 + 4).any(|y| {
                        (interval_x * 4..interval_x * 4 + 4)
                            .any(|x| mask[y * CELL_SOURCE_SAMPLES + x] != 0)
                    })
                })
                .count();
            if covering > 16 {
                return Err(AuthoringEmitError::Message(format!(
                    "BTD4 GCVR cell ({cell_x},{cell_y}) interval ({interval_x},{interval_y}) uses {covering} grass types; FO4 supports at most 16"
                )));
            }
        }
    }
    if masks.is_empty() {
        return Ok(None);
    }
    Ok(Some(crate::btd4::GcvrChunk {
        entries: masks
            .into_iter()
            .map(|(object_id, mask)| crate::btd4::GcvrEntry {
                plugin_index: 0,
                object_id,
                mask,
            })
            .collect(),
    }))
}

fn dense_cell_colors(
    btd: &mut BtdFile,
    cell_x: i32,
    cell_y: i32,
) -> Result<Vec<u8>, AuthoringEmitError> {
    let header = btd.header();
    let (min_x, min_y, max_x, max_y) = (
        header.cell_min_x,
        header.cell_min_y,
        header.cell_max_x,
        header.cell_max_y,
    );
    let starfield_btd_cell_min = header
        .is_starfield_layout
        .then_some((header.cell_min_x, header.cell_min_y));
    let mut fetch = |cx: i32, cy: i32| {
        crate::fo4_frame::assemble_cell_grid(
            |sx, sy| btd.cell_terrain_color_u16(sx.clamp(min_x, max_x), sy.clamp(min_y, max_y), 0),
            cx,
            cy,
            starfield_btd_cell_min,
        )
    };
    let current = fetch(cell_x, cell_y)?;
    let right = fetch(cell_x.saturating_add(1), cell_y)?;
    let top = fetch(cell_x, cell_y.saturating_add(1))?;
    let diagonal = fetch(cell_x.saturating_add(1), cell_y.saturating_add(1))?;
    let mut colors = Vec::with_capacity(129 * 129 * 3);
    for y in 0..=128usize {
        for x in 0..=128usize {
            let value = match (x == 128, y == 128) {
                (false, false) => current[y * CELL_SOURCE_SAMPLES + x],
                (true, false) => right[y * CELL_SOURCE_SAMPLES],
                (false, true) => top[x],
                (true, true) => diagonal[0],
            };
            colors.extend_from_slice(&fo76_vclr_to_fo4_vclr(value));
        }
    }
    Ok(colors)
}

fn count_texture_layer_ground_cover(
    has_ground_cover: bool,
    ground_cover_layer_count: &mut u32,
    no_ground_cover_layer_count: &mut u32,
) {
    if has_ground_cover {
        *ground_cover_layer_count = ground_cover_layer_count.saturating_add(1);
    } else {
        *no_ground_cover_layer_count = no_ground_cover_layer_count.saturating_add(1);
    }
}

#[cfg(test)]
fn effective_ground_cover_indices_for_layer(
    set: &CellTextureSet,
    ground_cover_mask: &[u8],
    quadrant: u8,
    source_slot: Option<u8>,
    ground_cover_index: Option<u8>,
    alpha_layer: Option<(&[u16], usize)>,
) -> Vec<u8> {
    let Some(quad) = set.quadrants.get(usize::from(quadrant)) else {
        return Vec::new();
    };
    let Some(source_slot) = source_slot else {
        return Vec::new();
    };
    let Some(ground_cover_index) = ground_cover_index else {
        return Vec::new();
    };
    if ground_cover_index_for_source_slot(quad, source_slot) != Some(ground_cover_index) {
        return Vec::new();
    }
    let Some(mask_bit) = ground_cover_mask_bit_for_source_slot(source_slot) else {
        return Vec::new();
    };
    let coverage = ground_cover_mask_coverage(ground_cover_mask, quadrant, mask_bit, alpha_layer);
    let min_visual_weight = if alpha_layer.is_some() {
        MIN_GROUND_COVER_ALPHA_VISUAL_WEIGHT
    } else {
        MIN_GROUND_COVER_LAYER_COVERAGE
    };
    if coverage.masked_fraction >= MIN_GROUND_COVER_LAYER_COVERAGE
        && coverage.visual_weight >= min_visual_weight
    {
        vec![ground_cover_index]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
fn ground_cover_index_for_source_slot(quad: &QuadrantTextureSet, source_slot: u8) -> Option<u8> {
    quad.ground_cover
        .get(usize::from(source_slot))
        .copied()
        .flatten()
}

#[cfg(test)]
fn ground_cover_mask_bit_for_source_slot(source_slot: u8) -> Option<u8> {
    if source_slot >= 8 {
        return None;
    }
    Some(1u8 << (7 - source_slot))
}

#[derive(Debug, Clone, Copy)]
#[cfg(test)]
struct GroundCoverCoverage {
    masked_fraction: f32,
    visual_weight: f32,
}

#[cfg(test)]
fn ground_cover_mask_coverage(
    ground_cover_mask: &[u8],
    quadrant: u8,
    bit: u8,
    alpha_layer: Option<(&[u16], usize)>,
) -> GroundCoverCoverage {
    let quadrant_x = usize::from(quadrant & 1);
    let quadrant_y = usize::from((quadrant >> 1) & 1);
    let source_origin_x = quadrant_x * CELL_SOURCE_QUADRANT_SAMPLES;
    let source_origin_y = quadrant_y * CELL_SOURCE_QUADRANT_SAMPLES;
    let mut covered = 0.0f32;
    let mut total = 0.0f32;

    for y in source_origin_y..source_origin_y + CELL_SOURCE_QUADRANT_SAMPLES {
        for x in source_origin_x..source_origin_x + CELL_SOURCE_QUADRANT_SAMPLES {
            let weight = alpha_layer
                .and_then(|(alphas, layer)| {
                    alphas
                        .get(y * CELL_SOURCE_SAMPLES + x)
                        .map(|packed| decode_alpha_layers(*packed)[layer])
                })
                .unwrap_or(1.0);
            if weight <= 0.0 {
                continue;
            }
            total += weight;
            if ground_cover_mask
                .get(y * CELL_SOURCE_SAMPLES + x)
                .copied()
                .unwrap_or(0)
                & bit
                != 0
            {
                covered += weight;
            }
        }
    }
    GroundCoverCoverage {
        masked_fraction: if total > 0.0 { covered / total } else { 0.0 },
        visual_weight: covered
            / (CELL_SOURCE_QUADRANT_SAMPLES * CELL_SOURCE_QUADRANT_SAMPLES) as f32,
    }
}

#[cfg(test)]
#[allow(dead_code)]
fn texture_for_layer<'a>(
    btd: &BtdFile,
    texture_index: u8,
    ground_cover_indices: &[u8],
    textures_by_source_usage: &'a HashMap<SourceTextureUsageKey, &EmittedTexture>,
) -> Option<&'a EmittedTexture> {
    let source_form_id = btd.land_texture_form_id(texture_index as usize)?;
    let ltex = format!("{:06X}", source_form_id & 0x00FF_FFFF);
    let mut gcvr_form_keys = Vec::new();
    for ground_cover_index in ground_cover_indices {
        let Some(gcvr_form_key) = btd
            .ground_cover_form_id(usize::from(*ground_cover_index))
            .map(|form_id| format!("{:06X}", form_id & 0x00FF_FFFF))
        else {
            continue;
        };
        gcvr_form_keys.push(gcvr_form_key);
    }
    texture_for_source_usage(&ltex, &gcvr_form_keys, textures_by_source_usage)
}

fn texture_for_source_ltex_object_id<'a>(
    source_ltex_object_id: u32,
    textures_by_source_usage: &'a HashMap<SourceTextureUsageKey, &EmittedTexture>,
) -> Option<&'a EmittedTexture> {
    texture_for_source_usage(
        &format!("{:06X}", source_ltex_object_id & 0x00FF_FFFF),
        &[],
        textures_by_source_usage,
    )
}

fn texture_for_source_usage<'a>(
    ltex: &str,
    ground_cover_form_keys: &[String],
    textures_by_source_usage: &'a HashMap<SourceTextureUsageKey, &EmittedTexture>,
) -> Option<&'a EmittedTexture> {
    for gcvr in ground_cover_form_keys {
        if let Some(texture) = textures_by_source_usage
            .get(&SourceTextureUsageKey {
                ltex: ltex.to_owned(),
                gcvr: Some(gcvr.clone()),
            })
            .copied()
        {
            return Some(texture);
        }
    }
    textures_by_source_usage
        .get(&SourceTextureUsageKey {
            ltex: ltex.to_owned(),
            gcvr: None,
        })
        .copied()
}

fn land_texture_layer_field(
    signature: &str,
    texture_object_id: u32,
    plugin_name: &str,
    quadrant: u8,
    layer: i16,
) -> String {
    // v2/v5 baseline (known-good for grass). CK's re-save values
    // (BTXT=1/ATXT=127) have no effect on grass or seams.
    let unknown_byte_3 = if signature == "BTXT" { 2 } else { 0 };
    format!(
        "    - {}:\n        Texture:\n          reference:\n            plugin: {}\n            object_id: \"{}\"\n        Quadrant: {}\n        UnknownByte3: {}\n        Layer: {}\n",
        signature,
        plugin_name,
        form_id_hex(texture_object_id),
        quadrant,
        unknown_byte_3,
        layer
    )
}

fn texture_slot_path(value: &str) -> String {
    let normalized = value.trim().replace('\\', "/");
    let stripped = normalized
        .strip_prefix("textures/")
        .or_else(|| normalized.strip_prefix("Textures/"))
        .unwrap_or(&normalized);
    stripped.replace('/', "\\")
}

fn cell_editor_id(worldspace_editor_id: &str, cell_x: i32, cell_y: i32) -> String {
    // CK silently strips '_' from CELL EditorIDs on first save. Emit the
    // post-strip form so the local YAML, the local ESP, and the deployed
    // ESP all agree without an in-CK rename. Other record types keep
    // underscores — this normalisation is CELL-specific.
    format!(
        "{}CellX{}Y{}",
        worldspace_editor_id,
        coordinate_token(cell_x),
        coordinate_token(cell_y)
    )
}

fn coordinate_token(value: i32) -> String {
    let sign = if value < 0 { 'N' } else { 'P' };
    format!("{sign}{:03}", value.unsigned_abs())
}

fn safe_editor_token(value: &str) -> String {
    let token = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if token.is_empty() {
        "Grass".to_string()
    } else {
        token
    }
}

fn load_preserved_terrain_ids(
    options: &ConvertOptions,
) -> Result<PreservedTerrainIds, AuthoringEmitError> {
    let mut preserved = PreservedTerrainIds::default();
    if !options.preserve_source_ids {
        return Ok(preserved);
    }

    merge_preserved_terrain_ids_payload(options, &mut preserved)?;

    let source_worldspace_authoring_dir = options.source_worldspace_authoring_dir.trim();
    if source_worldspace_authoring_dir.is_empty() {
        return Ok(preserved);
    }

    let worldspace_dir = PathBuf::from(source_worldspace_authoring_dir);
    if !worldspace_dir.is_dir() {
        return Ok(preserved);
    }

    let world_record = worldspace_dir.join("RecordData.yaml");
    if world_record.is_file() {
        preserved.world_form_id = read_first_form_id(&world_record)?;
        if let Some(world_form_id) = preserved.world_form_id {
            preserved
                .used_object_ids
                .insert(world_form_id & 0x00FF_FFFF);
        }
        preserved.world_editor_id = read_record_eid(&world_record)?;
    }
    scan_preserved_record_data_files(&worldspace_dir, &mut preserved, true)?;
    Ok(preserved)
}

fn merge_preserved_terrain_ids_payload(
    options: &ConvertOptions,
    preserved: &mut PreservedTerrainIds,
) -> Result<(), AuthoringEmitError> {
    let payload_text = options.source_worldspace_terrain_ids_json.trim();
    if payload_text.is_empty() {
        return Ok(());
    }

    let payload: SourceWorldspaceTerrainIdsPayload = serde_json::from_str(payload_text)?;
    if let Some(world_form_id) = nonzero_object_id(payload.world_form_id) {
        preserved.world_form_id = Some(world_form_id);
        preserved.used_object_ids.insert(world_form_id);
    }
    if let Some(world_editor_id) = nonempty_trimmed(payload.world_editor_id) {
        preserved.world_editor_id = Some(world_editor_id);
    }
    for cell in payload.cells {
        let cell_form_id = nonzero_object_id(Some(cell.cell_form_id));
        let land_form_id = nonzero_object_id(cell.land_form_id);
        if let Some(id) = cell_form_id {
            preserved.used_object_ids.insert(id);
        }
        if let Some(id) = land_form_id {
            preserved.used_object_ids.insert(id);
        }
        preserved.cells.insert(
            (cell.x, cell.y),
            PreservedCellIds {
                cell_form_id,
                cell_editor_id: nonempty_trimmed(Some(cell.cell_editor_id)),
                land_form_id,
            },
        );
    }
    Ok(())
}

fn nonzero_object_id(value: Option<u32>) -> Option<u32> {
    value.map(|id| id & 0x00FF_FFFF).filter(|id| *id != 0)
}

fn nonempty_trimmed(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

fn build_terrain_id_plan(
    options: &ConvertOptions,
    preserved: &PreservedTerrainIds,
    cells_x: usize,
    cells_y: usize,
) -> Result<TerrainIdPlan, AuthoringEmitError> {
    let mut used_object_ids = reserved_object_ids(options);
    let mut max_object_id = used_object_ids.iter().copied().max().unwrap_or(0);
    let world_form_id = planned_world_form_id(options, preserved, &mut used_object_ids)?;
    max_object_id = max_object_id.max(world_form_id & 0x00FF_FFFF);
    let mut planned_cells = preserved.cells.clone();
    let use_preserved_ids = options.preserve_source_ids && !preserved.used_object_ids.is_empty();
    let mut missing_cell_ids = Vec::new();
    let mut missing_land_ids = Vec::new();
    let mut index = 0u32;

    for cell_y in options.source_min_y..=options.source_max_y {
        for cell_x in options.source_min_x..=options.source_max_x {
            if use_preserved_ids {
                let planned_cell = planned_cells.entry((cell_x, cell_y)).or_default();
                if !claim_planned_object_id(
                    planned_cell.cell_form_id,
                    &mut used_object_ids,
                    &mut max_object_id,
                ) {
                    planned_cell.cell_form_id = None;
                    missing_cell_ids.push((cell_x, cell_y));
                }
                if !claim_planned_object_id(
                    planned_cell.land_form_id,
                    &mut used_object_ids,
                    &mut max_object_id,
                ) {
                    planned_cell.land_form_id = None;
                    missing_land_ids.push((cell_x, cell_y));
                }
            } else {
                let planned_cell = planned_cells.entry((cell_x, cell_y)).or_default();
                let cell_id = planned_cell
                    .cell_form_id
                    .unwrap_or_else(|| cell_form_id(options, index));
                let land_id = planned_cell
                    .land_form_id
                    .unwrap_or_else(|| land_form_id(options, index));
                planned_cell.cell_form_id = Some(claim_or_allocate_object_id(
                    cell_id,
                    &mut used_object_ids,
                    &mut max_object_id,
                )?);
                planned_cell.land_form_id = Some(claim_or_allocate_object_id(
                    land_id,
                    &mut used_object_ids,
                    &mut max_object_id,
                )?);
            }
            index = index.checked_add(1).ok_or_else(|| {
                AuthoringEmitError::Message("terrain form ID allocation overflow".to_string())
            })?;
        }
    }

    let expected_count = checked_cell_count(cells_x, cells_y)?;
    if index != expected_count {
        return Err(AuthoringEmitError::Message(
            "terrain cell ID planning count mismatch".to_string(),
        ));
    }

    if use_preserved_ids {
        let mut next_synthetic_id = max_object_id.checked_add(1).ok_or_else(|| {
            AuthoringEmitError::Message("terrain form ID allocation overflow".to_string())
        })?;
        for (cell_x, cell_y) in missing_cell_ids {
            let id = allocate_unused_object_id(&mut used_object_ids, &mut next_synthetic_id)?;
            planned_cells
                .entry((cell_x, cell_y))
                .or_default()
                .cell_form_id = Some(id);
        }
        for (cell_x, cell_y) in missing_land_ids {
            let id = allocate_unused_object_id(&mut used_object_ids, &mut next_synthetic_id)?;
            planned_cells
                .entry((cell_x, cell_y))
                .or_default()
                .land_form_id = Some(id);
        }
        max_object_id = used_object_ids
            .iter()
            .copied()
            .max()
            .unwrap_or(world_form_id & 0x00FF_FFFF);
    }

    let next_object_id_after_terrain = max_object_id.checked_add(1).ok_or_else(|| {
        AuthoringEmitError::Message("terrain form ID allocation overflow".to_string())
    })?;

    Ok(TerrainIdPlan {
        world_form_id,
        world_editor_id: preserved.world_editor_id.clone(),
        preserved_cells: planned_cells,
        source_backed: use_preserved_ids,
        next_object_id_after_terrain,
        used_object_ids,
    })
}

fn reserved_object_ids(options: &ConvertOptions) -> HashSet<u32> {
    options
        .reserved_object_ids
        .iter()
        .map(|id| id & 0x00FF_FFFF)
        .filter(|id| *id != 0)
        .collect()
}

fn planned_world_form_id(
    options: &ConvertOptions,
    preserved: &PreservedTerrainIds,
    used_object_ids: &mut HashSet<u32>,
) -> Result<u32, AuthoringEmitError> {
    let configured_world_form_id = terrain_world_form_id(options);
    let world_form_id = if options.world_form_id != 0 {
        configured_world_form_id
    } else {
        preserved.world_form_id.unwrap_or(configured_world_form_id) & 0x00FF_FFFF
    };
    let world_object_id = world_form_id & 0x00FF_FFFF;
    if options.world_form_id != 0 {
        used_object_ids.remove(&world_object_id);
        used_object_ids.insert(world_object_id);
        return Ok(world_form_id);
    }
    if world_object_id != 0 && used_object_ids.insert(world_object_id) {
        return Ok(world_form_id);
    }
    let mut next_object_id = used_object_ids
        .iter()
        .copied()
        .max()
        .unwrap_or(configured_world_form_id & 0x00FF_FFFF)
        .checked_add(1)
        .ok_or_else(|| {
            AuthoringEmitError::Message("terrain form ID allocation overflow".to_string())
        })?;
    allocate_unused_object_id(used_object_ids, &mut next_object_id)
}

fn claim_planned_object_id(
    object_id: Option<u32>,
    used_object_ids: &mut HashSet<u32>,
    max_object_id: &mut u32,
) -> bool {
    let Some(object_id) = object_id else {
        return false;
    };
    let object_id = object_id & 0x00FF_FFFF;
    if object_id == 0 || !used_object_ids.insert(object_id) {
        return false;
    }
    *max_object_id = (*max_object_id).max(object_id);
    true
}

fn claim_or_allocate_object_id(
    object_id: u32,
    used_object_ids: &mut HashSet<u32>,
    max_object_id: &mut u32,
) -> Result<u32, AuthoringEmitError> {
    let object_id = object_id & 0x00FF_FFFF;
    if object_id != 0 && used_object_ids.insert(object_id) {
        *max_object_id = (*max_object_id).max(object_id);
        return Ok(object_id);
    }
    let mut next_object_id = max_object_id.checked_add(1).ok_or_else(|| {
        AuthoringEmitError::Message("terrain form ID allocation overflow".to_string())
    })?;
    let allocated = allocate_unused_object_id(used_object_ids, &mut next_object_id)?;
    *max_object_id = (*max_object_id).max(allocated);
    Ok(allocated)
}

fn allocate_unused_object_id(
    used_object_ids: &mut HashSet<u32>,
    next_object_id: &mut u32,
) -> Result<u32, AuthoringEmitError> {
    while used_object_ids.contains(next_object_id) {
        *next_object_id = next_object_id.checked_add(1).ok_or_else(|| {
            AuthoringEmitError::Message("terrain form ID allocation overflow".to_string())
        })?;
    }
    let id = *next_object_id;
    used_object_ids.insert(id);
    *next_object_id = next_object_id.checked_add(1).ok_or_else(|| {
        AuthoringEmitError::Message("terrain form ID allocation overflow".to_string())
    })?;
    Ok(id)
}

fn read_first_form_id(path: &Path) -> Result<Option<u32>, AuthoringEmitError> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = line?;
        if let Some((key, value)) = line_key_value(&line) {
            if key == "form_id" {
                return Ok(object_id_from_value(&value));
            }
        }
    }
    Ok(None)
}

fn read_record_eid(path: &Path) -> Result<Option<String>, AuthoringEmitError> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = line?;
        if let Some((key, value)) = line_key_value(&line) {
            if (key == "eid" || key == "editor_id") && !value.is_empty() {
                return Ok(Some(value));
            }
        }
    }
    Ok(None)
}

fn scan_preserved_record_data_files(
    dir: &Path,
    preserved: &mut PreservedTerrainIds,
    is_worldspace_root: bool,
) -> Result<(), AuthoringEmitError> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            scan_preserved_record_data_files(&path, preserved, false)?;
        } else if path.file_name().and_then(|name| name.to_str()) == Some("RecordData.yaml") {
            if is_worldspace_root {
                continue;
            }
            if let Some(((cell_x, cell_y), cell_ids)) = read_preserved_cell_ids(&path, preserved)? {
                preserved.cells.insert((cell_x, cell_y), cell_ids);
            }
        }
    }
    Ok(())
}

fn read_preserved_cell_ids(
    path: &Path,
    preserved: &mut PreservedTerrainIds,
) -> Result<Option<((i32, i32), PreservedCellIds)>, AuthoringEmitError> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut is_cell = false;
    let mut in_landscape = false;
    let mut expect_xclc_data = false;
    let mut cell_x = None;
    let mut cell_y = None;
    let mut cell_form_id = None;
    let mut cell_editor_id = None;
    let mut land_form_id = None;
    let mut saw_grid = false;
    let mut in_cell_children = false;

    for line in reader.lines() {
        let line = line?;
        let stripped = line.trim();
        if stripped == "Landscape:" {
            in_landscape = true;
            continue;
        }
        if stripped == "Persistent:"
            || stripped == "Temporary:"
            || stripped == "VisibleWhenDistant:"
            || stripped == "NavigationMeshes:"
        {
            in_cell_children = true;
        }
        if let Some((key, value)) = line_key_value(&line) {
            match key.as_str() {
                "signature" if value == "CELL" => is_cell = true,
                "Grid" => {
                    saw_grid = true;
                    is_cell = true;
                }
                "signature" if value == "XCLC" => expect_xclc_data = true,
                "eid" | "editor_id"
                    if !in_landscape && !in_cell_children && cell_editor_id.is_none() =>
                {
                    cell_editor_id = Some(value);
                }
                "form_id" => {
                    let object_id = object_id_from_value(&value);
                    if let Some(id) = object_id {
                        preserved.used_object_ids.insert(id & 0x00FF_FFFF);
                    }
                    if in_landscape && land_form_id.is_none() {
                        land_form_id = object_id;
                    } else if !in_landscape && cell_form_id.is_none() {
                        cell_form_id = object_id;
                    }
                }
                "data_hex" if expect_xclc_data => {
                    if let Some((x, y)) = cell_coords_from_xclc_hex(&value) {
                        cell_x = Some(x);
                        cell_y = Some(y);
                        is_cell = true;
                    }
                    expect_xclc_data = false;
                }
                "X" if cell_x.is_none() => {
                    cell_x = value.parse::<i32>().ok();
                    saw_grid = true;
                }
                "Y" if cell_y.is_none() => {
                    cell_y = value.parse::<i32>().ok();
                    saw_grid = true;
                }
                _ => {}
            }
        }
    }

    if !is_cell && !(saw_grid && cell_form_id.is_some()) {
        return Ok(None);
    }
    if saw_grid {
        cell_x = Some(cell_x.unwrap_or(0));
        cell_y = Some(cell_y.unwrap_or(0));
    }
    let Some(cell_x) = cell_x else {
        return Ok(None);
    };
    let Some(cell_y) = cell_y else {
        return Ok(None);
    };
    Ok(Some((
        (cell_x, cell_y),
        PreservedCellIds {
            cell_form_id,
            cell_editor_id,
            land_form_id,
        },
    )))
}

fn line_key_value(line: &str) -> Option<(String, String)> {
    let (key, value) = line.trim().split_once(':')?;
    let key = key
        .trim()
        .trim_start_matches("- ")
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string();
    Some((key, unquote_yaml_scalar(value)))
}

fn unquote_yaml_scalar(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string()
}

fn object_id_from_value(value: &str) -> Option<u32> {
    let object_id = value.split(':').next().unwrap_or(value).trim();
    u32::from_str_radix(object_id.trim_start_matches("0x"), 16)
        .ok()
        .map(|value| value & 0x00FF_FFFF)
}

fn cell_coords_from_xclc_hex(value: &str) -> Option<(i32, i32)> {
    let hex: String = value.chars().filter(|ch| ch.is_ascii_hexdigit()).collect();
    if hex.len() < 16 {
        return None;
    }
    let mut bytes = [0u8; 8];
    for index in 0..8 {
        bytes[index] = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?;
    }
    let x = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let y = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    Some((x, y))
}

fn cell_span(min: i32, max: i32) -> Result<usize, AuthoringEmitError> {
    let span = i64::from(max) - i64::from(min) + 1;
    usize::try_from(span)
        .map_err(|_| AuthoringEmitError::Message("cell range span overflow".to_string()))
}

fn checked_cell_count(cells_x: usize, cells_y: usize) -> Result<u32, AuthoringEmitError> {
    let count = cells_x
        .checked_mul(cells_y)
        .ok_or_else(|| AuthoringEmitError::Message("cell count overflow".to_string()))?;
    u32::try_from(count)
        .map_err(|_| AuthoringEmitError::Message("cell count exceeds u32".to_string()))
}

#[cfg(test)]
fn next_terrain_object_id(first_form_id: u32, cell_count: u32) -> Result<u32, AuthoringEmitError> {
    first_form_id
        .checked_add(1)
        .and_then(|value| value.checked_add(cell_count.checked_mul(2)?))
        .ok_or_else(|| AuthoringEmitError::Message("form ID allocation overflow".to_string()))
}

fn next_texture_object_id(
    first_texture_object_id: u32,
    textures: &[EmittedTexture],
) -> Result<u32, AuthoringEmitError> {
    let mut next_object_id = first_texture_object_id;
    for texture in textures {
        for object_id in std::iter::once(texture.txst_object_id)
            .chain(std::iter::once(texture.ltex_object_id))
            .chain(texture.grass_object_ids.iter().copied())
        {
            if object_id >= next_object_id {
                next_object_id = object_id.checked_add(1).ok_or_else(|| {
                    AuthoringEmitError::Message("texture form ID allocation overflow".to_string())
                })?;
            }
        }
    }
    Ok(next_object_id)
}

fn zstring_hex(value: &str) -> String {
    let mut bytes = value.as_bytes().to_vec();
    bytes.push(0);
    bytes_hex(&bytes)
}

fn bytes_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

fn yaml_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn u16_hex(value: u16) -> String {
    bytes_hex(&value.to_le_bytes())
}

fn u32_hex(value: u32) -> String {
    bytes_hex(&value.to_le_bytes())
}

fn i32_hex(value: i32) -> String {
    bytes_hex(&value.to_le_bytes())
}

fn f32_hex(value: f32) -> String {
    bytes_hex(&value.to_le_bytes())
}

fn master_form_id(object_id: u32) -> u32 {
    object_id & 0x00FF_FFFF
}

fn normalize_source_form_key(value: &str) -> String {
    format!("{:06X}", object_id_from_source_form_key(value).unwrap_or(0))
}

fn floor_div(value: i32, divisor: i32) -> i32 {
    let quotient = value / divisor;
    let remainder = value % divisor;
    if remainder != 0 && ((remainder < 0) != (divisor < 0)) {
        quotient - 1
    } else {
        quotient
    }
}

fn cell_form_id(options: &ConvertOptions, index: u32) -> u32 {
    first_cell_form_id(options) + index * 2
}

fn land_form_id(options: &ConvertOptions, index: u32) -> u32 {
    first_cell_form_id(options) + 1 + index * 2
}

fn terrain_world_form_id(options: &ConvertOptions) -> u32 {
    if options.world_form_id != 0 {
        options.world_form_id
    } else {
        options.first_form_id
    }
}

fn first_cell_form_id(options: &ConvertOptions) -> u32 {
    if options.first_cell_form_id != 0 {
        options.first_cell_form_id
    } else {
        options.first_form_id + 1
    }
}

fn form_id_hex(value: u32) -> String {
    format!("{:06X}", value & 0x00FF_FFFF)
}

fn compact_form_id_hex(value: u32) -> String {
    format!("{:x}", value & 0x00FF_FFFF)
}

fn source_ltex_form_key(value: u32, is_starfield_layout: bool) -> String {
    let plugin_name = if is_starfield_layout {
        "Starfield.esm"
    } else {
        "SeventySix.esm"
    };
    format!("{:06X}:{plugin_name}", value & 0x00FF_FFFF)
}

fn default_first_form_id() -> u32 {
    0x000800
}

fn default_water_object_id() -> u32 {
    FO4_DEFAULT_WATER_OBJECT_ID
}

fn deserialize_u32_or_default<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<u32>::deserialize(deserializer)?.unwrap_or_else(default_first_form_id))
}

fn deserialize_u32_or_zero<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<u32>::deserialize(deserializer)?.unwrap_or_default())
}

fn deserialize_string_or_default<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_texture_alpha_masks_reuse_cached_decode() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("terrain_alpha.dds");
        let rgba = vec![
            10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160,
        ];
        directxtex_native::write_dds_rgba_image(&path, 2, 2, &rgba, "R8G8B8A8_UNORM", false)
            .expect("write fixture DDS");

        let first = load_source_texture_alpha_mask(&path).expect("first decode");
        let second = load_source_texture_alpha_mask(&path).expect("cached decode");

        assert_eq!(&*first.alpha, &[40, 80, 120, 160]);
        assert!(Arc::ptr_eq(&first.alpha, &second.alpha));
    }

    #[test]
    fn dense_alpha_preserves_full_resolution_slot_changes() {
        let mut ordered_layers = vec![None; 24];
        ordered_layers[1] = Some(0x100);
        ordered_layers[2] = Some(0x200);
        ordered_layers[7] = Some(0x200);
        let mut texture_ids = vec![None; CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES];
        for y in 0..CELL_SOURCE_SAMPLES {
            for x in 0..CELL_SOURCE_SAMPLES {
                texture_ids[y * CELL_SOURCE_SAMPLES + x] =
                    Some(if x & 1 == 0 { 0x100 } else { 0x200 });
            }
            texture_ids[y * CELL_SOURCE_SAMPLES + 63] = Some(0x100);
            texture_ids[y * CELL_SOURCE_SAMPLES + 64] = Some(0x200);
        }

        let planes = dense_alpha_planes(&texture_ids, &ordered_layers).expect("dense alpha");
        assert_eq!(planes.len(), crate::btd4::ALPH_PLANE_COUNT);
        assert_eq!(planes[0][10], u8::MAX);
        assert_eq!(planes[0][11], 0);
        assert_eq!(planes[1][10], 0);
        assert_eq!(planes[1][11], u8::MAX);
        assert_eq!(planes[0][64], u8::MAX);
        assert_eq!(planes[5][0], u8::MAX);
    }

    #[test]
    fn per_cell_quantization_matches_full_grid_quantization() {
        // Regression guard for the memory optimization that replaced a second
        // full-worldspace quantized grid with on-the-fly per-cell quantization.
        // Build a multi-cell grid (2x2 cells) with a nonzero origin and varied,
        // non-lattice-aligned heights, then prove per-cell == full-grid output
        // bit-for-bit.
        let width = 2 * LAND_CELL_INTERVALS + 1; // 65
        let height = 2 * LAND_CELL_INTERVALS + 1; // 65
        let values: Vec<f32> = (0..width * height)
            .map(|i| 100.0 + (i as f32) * 0.37 + ((i * 7) % 13) as f32)
            .collect();
        let grid = TargetHeightGrid {
            width,
            height,
            values,
        };
        let origin = grid.values[0];
        assert_ne!(origin, 0.0, "origin must be nonzero to exercise the offset");

        let full = quantize_target_grid_to_vhgt_lattice(&grid);
        for cell_y in 0..2 {
            for cell_x in 0..2 {
                let raw = extract_target_cell_heights(&grid, cell_x, cell_y);
                let per_cell = quantize_cell_heights_to_lattice(&raw, origin);
                let from_full = extract_target_cell_heights(&full, cell_x, cell_y);
                assert_eq!(per_cell.len(), from_full.len());
                for (a, b) in per_cell.iter().zip(from_full.iter()) {
                    assert_eq!(a.to_bits(), b.to_bits(), "cell ({cell_x},{cell_y})");
                }
            }
        }
    }

    #[test]
    fn collect_only_authoring_output_keeps_records_off_disk() {
        let output_dir = std::env::temp_dir().join(format!(
            "terrain_native_collect_only_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be after epoch")
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&output_dir);
        let mut output = AuthoringOutput::collect_only(output_dir.clone());

        output
            .write_plugin_yaml("plugin: B21_Test.esp\n".to_string())
            .expect("plugin payload");
        output
            .write_record_yaml(
                "CELL",
                PathBuf::from("records")
                    .join("WRLD")
                    .join("B21_Test - 000800_B21_Test.esp")
                    .join("0, 0")
                    .join("0, 0")
                    .join("0, 0")
                    .join("RecordData.yaml"),
                "form_id: \"000801:B21_Test.esp\"\neid: B21_TestCell\n".to_string(),
            )
            .expect("cell payload");
        let collected = output.finish();

        assert_eq!(collected.plugin_yaml, "plugin: B21_Test.esp\n");
        assert_eq!(collected.records.len(), 1);
        assert_eq!(collected.records[0].signature, "CELL");
        assert!(collected.records[0].yaml.contains("B21_TestCell"));
        assert!(!output_dir.join("records").exists());
    }

    #[test]
    fn report_only_authoring_output_keeps_records_off_disk_and_out_of_memory() {
        let output_dir = std::env::temp_dir().join(format!(
            "terrain_native_report_only_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be after epoch")
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&output_dir);
        let mut output = AuthoringOutput::report_only(output_dir.clone());

        output
            .write_plugin_yaml("plugin: B21_Test.esp\n".to_string())
            .expect("plugin payload");
        output
            .write_record_yaml(
                "CELL",
                PathBuf::from("records")
                    .join("WRLD")
                    .join("B21_Test - 000800_B21_Test.esp")
                    .join("0, 0")
                    .join("0, 0")
                    .join("0, 0")
                    .join("RecordData.yaml"),
                "form_id: \"000801:B21_Test.esp\"\neid: B21_TestCell\n".to_string(),
            )
            .expect("cell payload");
        let collected = output.finish();

        assert!(collected.plugin_yaml.is_empty());
        assert!(collected.records.is_empty());
        assert!(!output_dir.join("records").exists());
    }

    #[test]
    fn streamed_authoring_output_sends_records_without_retaining_them() {
        let output_dir = std::env::temp_dir().join(format!(
            "terrain_native_stream_only_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be after epoch")
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&output_dir);
        let mut streamed = Vec::new();
        {
            let mut sink = |record: AuthoringRecordPayload| {
                streamed.push(record);
                Ok(())
            };
            let mut output = AuthoringOutput::stream_records(output_dir.clone(), &mut sink);

            output
                .write_record_yaml(
                    "CELL",
                    PathBuf::from("records")
                        .join("WRLD")
                        .join("B21_Test - 000800_B21_Test.esp")
                        .join("0, 0")
                        .join("0, 0")
                        .join("0, 0")
                        .join("RecordData.yaml"),
                    "form_id: \"000801:B21_Test.esp\"\neid: B21_TestCell\n".to_string(),
                )
                .expect("cell payload");
            let collected = output.finish();

            assert!(collected.records.is_empty());
        }

        assert_eq!(streamed.len(), 1);
        assert_eq!(streamed[0].signature, "CELL");
        assert!(streamed[0].yaml.contains("B21_TestCell"));
        assert!(!output_dir.join("records").exists());
    }

    #[test]
    fn texture_form_ids_start_after_terrain_record_range() {
        let options = ConvertOptions {
            btd_path: String::new(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esp".to_string(),
            worldspace_editor_id: "B21_Test".to_string(),
            source_min_x: 0,
            source_min_y: 0,
            source_max_x: 0,
            source_max_y: 0,
            first_form_id: 0x800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: true,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: false,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };
        let cell_count = 6144;
        let terrain_next_object_id = next_terrain_object_id(options.first_form_id, cell_count)
            .expect("terrain range should fit");
        let converted = vec![ConvertedTerrainTexture {
            source_ltex_form_key: "001234".to_string(),
            source_ltex_editor_id: "LTest".to_string(),
            source_gcvr_form_key: None,
            source_gcvr_editor_id: None,
            source_txst_form_key: "001235".to_string(),
            source_txst_editor_id: "LandscapeTest".to_string(),
            suffix: "Test".to_string(),
            diffuse_rel_path: "textures/terrain/test_d.dds".to_string(),
            normal_rel_path: "textures/terrain/test_n.dds".to_string(),
            specgloss_rel_path: "textures/terrain/test_s.dds".to_string(),
            glow_rel_path: "textures/terrain/test_g.dds".to_string(),
            material_type_object_id: Some("012F38".to_string()),
            havok_friction: 30,
            havok_restitution: 30,
            grass: Vec::new(),
        }];

        let emitted =
            assign_texture_form_ids(terrain_next_object_id, converted, false, HashSet::new())
                .expect("texture IDs");

        assert_eq!(emitted[0].txst_object_id, terrain_next_object_id);
        assert_eq!(emitted[0].ltex_object_id, terrain_next_object_id + 1);
        assert_eq!(
            next_texture_object_id(terrain_next_object_id, &emitted).unwrap(),
            terrain_next_object_id + 2
        );
    }

    #[test]
    fn texture_form_ids_preserve_source_object_ids_when_enabled() {
        let converted = vec![ConvertedTerrainTexture {
            source_ltex_form_key: "001234:SeventySix.esm".to_string(),
            source_ltex_editor_id: "LTest".to_string(),
            source_gcvr_form_key: None,
            source_gcvr_editor_id: None,
            source_txst_form_key: "001235:SeventySix.esm".to_string(),
            source_txst_editor_id: "LandscapeTest".to_string(),
            suffix: "Test".to_string(),
            diffuse_rel_path: "textures/terrain/test_d.dds".to_string(),
            normal_rel_path: "textures/terrain/test_n.dds".to_string(),
            specgloss_rel_path: "textures/terrain/test_s.dds".to_string(),
            glow_rel_path: "textures/terrain/test_g.dds".to_string(),
            material_type_object_id: None,
            havok_friction: 30,
            havok_restitution: 30,
            grass: vec![crate::texture_bridge::ConvertedTerrainGrass {
                source_form_key: "3900A5:SeventySix.esm".to_string(),
                source_editor_id: "Forest76GrassObj03A".to_string(),
                object_bounds: crate::texture_bridge::ConvertedGrassObjectBounds::default(),
                model_file_name: "landscape/grass/Forest76GrassObj03A.nif".to_string(),
                model_information: String::new(),
                density: 0,
                max_slope: 0,
                position_range: 0.0,
                height_range: 0.0,
                color_range: 0.0,
                wave_period: 0.0,
                flags: Vec::new(),
            }],
        }];

        let emitted =
            assign_texture_form_ids(0x800, converted, true, HashSet::new()).expect("texture IDs");

        assert_eq!(emitted[0].txst_object_id, 0x001235);
        assert_eq!(emitted[0].ltex_object_id, 0x001234);
        assert_eq!(emitted[0].grass_object_ids, vec![0x3900A5]);
        assert_eq!(next_texture_object_id(0x800, &emitted).unwrap(), 0x3900A6);
    }

    #[test]
    fn texture_form_ids_share_txst_and_grass_across_gcvr_variants() {
        let grass = crate::texture_bridge::ConvertedTerrainGrass {
            source_form_key: "3900A5:SeventySix.esm".to_string(),
            source_editor_id: "Forest76GrassObj03A".to_string(),
            object_bounds: crate::texture_bridge::ConvertedGrassObjectBounds::default(),
            model_file_name: "landscape/grass/Forest76GrassObj03A.nif".to_string(),
            model_information: String::new(),
            density: 0,
            max_slope: 0,
            position_range: 12.0,
            height_range: 0.0,
            color_range: 0.0,
            wave_period: 0.0,
            flags: Vec::new(),
        };
        let base = ConvertedTerrainTexture {
            source_ltex_form_key: "001234:SeventySix.esm".to_string(),
            source_ltex_editor_id: "LForestGrass01".to_string(),
            source_gcvr_form_key: None,
            source_gcvr_editor_id: None,
            source_txst_form_key: "001235:SeventySix.esm".to_string(),
            source_txst_editor_id: "LandscapeForestGrass01".to_string(),
            suffix: "ForestGrass".to_string(),
            diffuse_rel_path: "textures/terrain/test_d.dds".to_string(),
            normal_rel_path: "textures/terrain/test_n.dds".to_string(),
            specgloss_rel_path: "textures/terrain/test_s.dds".to_string(),
            glow_rel_path: "textures/terrain/test_g.dds".to_string(),
            material_type_object_id: None,
            havok_friction: 30,
            havok_restitution: 30,
            grass: Vec::new(),
        };
        let mut variant = base.clone();
        variant.source_gcvr_form_key = Some("011C67:SeventySix.esm".to_string());
        variant.source_gcvr_editor_id = Some("GCForestGrass01".to_string());
        variant.grass = vec![grass.clone()];
        let mut second_variant = variant.clone();
        second_variant.source_gcvr_form_key = Some("081263:SeventySix.esm".to_string());
        second_variant.source_gcvr_editor_id = Some("GCForestLeaves01".to_string());
        second_variant.grass = vec![grass];

        let emitted = assign_texture_form_ids(
            0x800,
            vec![base, variant, second_variant],
            true,
            HashSet::new(),
        )
        .expect("texture IDs");

        assert_eq!(emitted[0].txst_object_id, 0x001235);
        assert_eq!(emitted[1].txst_object_id, 0x001235);
        assert_eq!(emitted[2].txst_object_id, 0x001235);
        assert_eq!(emitted[0].ltex_object_id, 0x001234);
        assert_ne!(emitted[1].ltex_object_id, 0x001234);
        assert_ne!(emitted[2].ltex_object_id, 0x001234);
        assert_ne!(emitted[1].ltex_object_id, emitted[2].ltex_object_id);
        assert_eq!(emitted[1].grass_object_ids, vec![0x3900A5]);
        assert_eq!(emitted[2].grass_object_ids, vec![0x3900A5]);
    }

    #[test]
    fn texture_form_ids_skip_reserved_source_object_ids() {
        let converted = vec![ConvertedTerrainTexture {
            source_ltex_form_key: "001234:SeventySix.esm".to_string(),
            source_ltex_editor_id: "LTest".to_string(),
            source_gcvr_form_key: None,
            source_gcvr_editor_id: None,
            source_txst_form_key: "001235:SeventySix.esm".to_string(),
            source_txst_editor_id: "LandscapeTest".to_string(),
            suffix: "Test".to_string(),
            diffuse_rel_path: "textures/terrain/test_d.dds".to_string(),
            normal_rel_path: "textures/terrain/test_n.dds".to_string(),
            specgloss_rel_path: "textures/terrain/test_s.dds".to_string(),
            glow_rel_path: "textures/terrain/test_g.dds".to_string(),
            material_type_object_id: None,
            havok_friction: 30,
            havok_restitution: 30,
            grass: Vec::new(),
        }];
        let reserved = HashSet::from([0x001235]);

        let emitted =
            assign_texture_form_ids(0x800, converted, true, reserved).expect("texture IDs");

        assert_eq!(emitted[0].txst_object_id, 0x800);
        assert_eq!(emitted[0].ltex_object_id, 0x001234);
    }

    #[test]
    fn texture_form_ids_preserve_plugin_first_source_object_ids() {
        let converted = vec![ConvertedTerrainTexture {
            source_ltex_form_key: "SeventySix.esm:003B1B".to_string(),
            source_ltex_editor_id: "LTest".to_string(),
            source_gcvr_form_key: None,
            source_gcvr_editor_id: None,
            source_txst_form_key: "SeventySix.esm:001235".to_string(),
            source_txst_editor_id: "LandscapeTest".to_string(),
            suffix: "Test".to_string(),
            diffuse_rel_path: "textures/terrain/test_d.dds".to_string(),
            normal_rel_path: "textures/terrain/test_n.dds".to_string(),
            specgloss_rel_path: "textures/terrain/test_s.dds".to_string(),
            glow_rel_path: "textures/terrain/test_g.dds".to_string(),
            material_type_object_id: None,
            havok_friction: 30,
            havok_restitution: 30,
            grass: vec![crate::texture_bridge::ConvertedTerrainGrass {
                source_form_key: "SeventySix.esm:3B396F".to_string(),
                source_editor_id: "Forest76WeedObj01".to_string(),
                object_bounds: crate::texture_bridge::ConvertedGrassObjectBounds::default(),
                model_file_name: "landscape/grass/Forest76WeedObj01.nif".to_string(),
                model_information: String::new(),
                density: 0,
                max_slope: 0,
                position_range: 0.0,
                height_range: 0.0,
                color_range: 0.0,
                wave_period: 0.0,
                flags: Vec::new(),
            }],
        }];

        let emitted =
            assign_texture_form_ids(0x800, converted, true, HashSet::new()).expect("texture IDs");

        assert_eq!(emitted[0].txst_object_id, 0x001235);
        assert_eq!(emitted[0].ltex_object_id, 0x003B1B);
        assert_eq!(emitted[0].grass_object_ids, vec![0x3B396F]);
        assert_eq!(normalize_source_form_key("SeventySix.esm:3B396F"), "3B396F");
    }

    #[test]
    fn texture_records_emit_fo4_texture_slots_from_converted_paths() {
        let output_dir = std::env::temp_dir().join(format!(
            "terrain_native_texture_records_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be after epoch")
                .as_nanos()
        ));
        let options = ConvertOptions {
            btd_path: String::new(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esp".to_string(),
            worldspace_editor_id: "B21_Test".to_string(),
            source_min_x: 0,
            source_min_y: 0,
            source_max_x: 0,
            source_max_y: 0,
            first_form_id: 0x800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: true,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: true,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };
        let texture = EmittedTexture {
            converted: ConvertedTerrainTexture {
                source_ltex_form_key: "001234".to_string(),
                source_ltex_editor_id: "LTest".to_string(),
                source_gcvr_form_key: None,
                source_gcvr_editor_id: None,
                source_txst_form_key: "001235".to_string(),
                source_txst_editor_id: "LandscapeTest".to_string(),
                suffix: "Test".to_string(),
                diffuse_rel_path: "textures/terrain/test_d.dds".to_string(),
                normal_rel_path: "textures/terrain/test_n.dds".to_string(),
                specgloss_rel_path: "textures/terrain/test_s.dds".to_string(),
                glow_rel_path: "textures/terrain/test_g.dds".to_string(),
                material_type_object_id: Some("012F38".to_string()),
                havok_friction: 30,
                havok_restitution: 30,
                grass: Vec::new(),
            },
            txst_object_id: 0x900,
            ltex_object_id: 0x901,
            grass_object_ids: Vec::new(),
        };

        let mut output = AuthoringOutput::write_files(output_dir.clone());
        write_texture_records(&mut output, &options, &[texture.clone()], "APPALACHIA")
            .expect("texture records");
        let txst_path = output_dir
            .join("records")
            .join("TXST")
            .join("LandscapeTest - 000900_B21_Test.esp.yaml");
        let payload = fs::read_to_string(txst_path).expect("txst yaml");
        let ltex_payload = fs::read_to_string(
            output_dir
                .join("records")
                .join("LTEX")
                .join("LTest - 000901_B21_Test.esp.yaml"),
        )
        .expect("ltex yaml");
        let _ = fs::remove_dir_all(&output_dir);

        assert!(payload.contains("- TexturesRgbAs:\n"));
        assert!(payload.contains("  - Diffuse: 'terrain\\test_d.dds'\n"));
        assert!(payload.contains("    NormalGloss: 'terrain\\test_n.dds'\n"));
        assert!(payload.contains("    SmoothSpec: 'terrain\\test_s.dds'\n"));
        // FO4 terrain materials have no emissive map; the Glow slot is omitted.
        assert!(!payload.contains("Glow:"));
        // Vanilla landscape TXSTs set NoSpecularMap; match it.
        assert!(payload.contains("- Flags:\n  - NoSpecularMap\n"));
        assert!(!payload.contains("- Material:"));
        assert!(ltex_payload.contains("- TextureSet:\n"));
        assert!(ltex_payload.contains("      plugin: B21_Test.esp\n"));
        assert!(ltex_payload.contains("      object_id: \"000900\"\n"));
        assert!(ltex_payload.contains("- MaterialType:\n"));
        assert!(ltex_payload.contains("      plugin: Fallout4.esm\n"));
        assert!(ltex_payload.contains("      object_id: \"012F38\"\n"));
    }

    #[test]
    fn texture_slot_paths_are_relative_to_textures_root() {
        assert_eq!(
            texture_slot_path("textures/terrain/test_d.dds"),
            "terrain\\test_d.dds"
        );
        assert_eq!(
            texture_slot_path("Textures/Landscape/Ground/DriedGrass01_D.dds"),
            "Landscape\\Ground\\DriedGrass01_D.dds"
        );
    }

    #[test]
    fn texture_records_emit_local_grass_and_link_from_ltex() {
        let output_dir = std::env::temp_dir().join(format!(
            "terrain_native_texture_grass_records_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be after epoch")
                .as_nanos()
        ));
        let options = ConvertOptions {
            btd_path: String::new(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esp".to_string(),
            worldspace_editor_id: "B21_Test".to_string(),
            source_min_x: 0,
            source_min_y: 0,
            source_max_x: 0,
            source_max_y: 0,
            first_form_id: 0x800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: true,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: true,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };
        let texture = EmittedTexture {
            converted: ConvertedTerrainTexture {
                source_ltex_form_key: "001234".to_string(),
                source_ltex_editor_id: "LForestGrass01".to_string(),
                source_gcvr_form_key: None,
                source_gcvr_editor_id: None,
                source_txst_form_key: "001235".to_string(),
                source_txst_editor_id: "LandscapeForestGrass01".to_string(),
                suffix: "ForestGrass".to_string(),
                diffuse_rel_path: "textures/terrain/test_d.dds".to_string(),
                normal_rel_path: "textures/terrain/test_n.dds".to_string(),
                specgloss_rel_path: "textures/terrain/test_s.dds".to_string(),
                glow_rel_path: "textures/terrain/test_g.dds".to_string(),
                material_type_object_id: Some("012F46".to_string()),
                havok_friction: 30,
                havok_restitution: 30,
                grass: vec![crate::texture_bridge::ConvertedTerrainGrass {
                    source_form_key: "3900A5:SeventySix.esm".to_string(),
                    source_editor_id: "Forest76GrassObj03A".to_string(),
                    object_bounds: crate::texture_bridge::ConvertedGrassObjectBounds {
                        x1: -43,
                        y1: -26,
                        z1: -3,
                        x2: 28,
                        y2: 36,
                        z2: 54,
                    },
                    model_file_name: "landscape/grass/Forest76GrassObj03A.nif".to_string(),
                    model_information: "0400000A".to_string(),
                    density: 94,
                    max_slope: 57,
                    position_range: 0.1,
                    height_range: 0.3,
                    color_range: 0.2,
                    wave_period: 145.0,
                    flags: vec!["VertexLighting".to_string(), "FitToSlope".to_string()],
                }],
            },
            txst_object_id: 0x900,
            ltex_object_id: 0x901,
            grass_object_ids: vec![0x902],
        };

        let mut output = AuthoringOutput::write_files(output_dir.clone());
        write_texture_records(&mut output, &options, &[texture.clone()], "APPALACHIA")
            .expect("texture records");
        let ltex_payload = fs::read_to_string(
            output_dir
                .join("records")
                .join("LTEX")
                .join("LForestGrass01 - 000901_B21_Test.esp.yaml"),
        )
        .expect("ltex yaml");
        let grass_payload = fs::read_to_string(
            output_dir
                .join("records")
                .join("GRAS")
                .join("Forest76GrassObj03A - 000902_B21_Test.esp.yaml"),
        )
        .expect("grass yaml");
        let _ = fs::remove_dir_all(&output_dir);

        assert!(ltex_payload.contains("- Grass:\n"));
        assert!(ltex_payload.contains("      plugin: B21_Test.esp\n"));
        assert!(ltex_payload.contains("      object_id: \"000902\"\n"));
        assert!(grass_payload.contains("eid: Forest76GrassObj03A\n"));
        assert!(
            grass_payload
                .contains("- ModelFileName: 'landscape\\grass\\Forest76GrassObj03A.nif'\n")
        );
        assert!(grass_payload.contains("- ModelInformation:\n    raw_hex: '0400000A'\n"));
        assert!(grass_payload.contains("    Density: 94\n"));
        assert!(grass_payload.contains("    MaxSlope: 57\n"));
        assert!(grass_payload.contains("    PositionRange: 0.1\n"));
        assert!(grass_payload.contains("    HeightRange: 0.3\n"));
        assert!(grass_payload.contains("    ColorRange: 0.2\n"));
        assert!(grass_payload.contains("    WavePeriod: 145\n"));
        assert!(grass_payload.contains("    - VertexLighting\n"));
        assert!(grass_payload.contains("    - FitToSlope\n"));

        let mut btd4_options = options.clone();
        btd4_options.btd4_output_path = "Terrain/B21TestWorld.btd4".to_string();
        let mut btd4_output = AuthoringOutput::collect_only(output_dir);
        write_texture_records(&mut btd4_output, &btd4_options, &[texture], "APPALACHIA")
            .expect("BTD4 texture records");
        let records = btd4_output.finish();
        let btd4_ltex = records
            .records
            .iter()
            .find(|record| record.signature == "LTEX")
            .expect("BTD4 LTEX");
        assert!(!btd4_ltex.yaml.contains("- Grass:\n"));
        assert_eq!(
            records
                .records
                .iter()
                .filter(|record| record.signature == "GRAS")
                .count(),
            1
        );
    }

    #[test]
    fn texture_records_emit_no_grass_and_gcvr_grass_ltex_variants() {
        let options = ConvertOptions {
            btd_path: String::new(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esp".to_string(),
            worldspace_editor_id: "B21_Test".to_string(),
            source_min_x: 0,
            source_min_y: 0,
            source_max_x: 0,
            source_max_y: 0,
            first_form_id: 0x800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: true,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: true,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };
        let base = ConvertedTerrainTexture {
            source_ltex_form_key: "001234:SeventySix.esm".to_string(),
            source_ltex_editor_id: "LForestGrass01".to_string(),
            source_gcvr_form_key: None,
            source_gcvr_editor_id: None,
            source_txst_form_key: "001235:SeventySix.esm".to_string(),
            source_txst_editor_id: "LandscapeForestGrass01".to_string(),
            suffix: "ForestGrass".to_string(),
            diffuse_rel_path: "textures/terrain/test_d.dds".to_string(),
            normal_rel_path: "textures/terrain/test_n.dds".to_string(),
            specgloss_rel_path: "textures/terrain/test_s.dds".to_string(),
            glow_rel_path: "textures/terrain/test_g.dds".to_string(),
            material_type_object_id: None,
            havok_friction: 30,
            havok_restitution: 30,
            grass: Vec::new(),
        };
        let mut variant = base.clone();
        variant.source_gcvr_form_key = Some("011C67:SeventySix.esm".to_string());
        variant.source_gcvr_editor_id = Some("GCForestGrass01".to_string());
        variant.grass = vec![crate::texture_bridge::ConvertedTerrainGrass {
            source_form_key: "3900A5:SeventySix.esm".to_string(),
            source_editor_id: "Forest76GrassObj03A".to_string(),
            object_bounds: crate::texture_bridge::ConvertedGrassObjectBounds::default(),
            model_file_name: "landscape/grass/Forest76GrassObj03A.nif".to_string(),
            model_information: String::new(),
            density: 94,
            max_slope: 57,
            position_range: 12.0,
            height_range: 0.3,
            color_range: 0.2,
            wave_period: 145.0,
            flags: Vec::new(),
        }];
        let textures = vec![
            EmittedTexture {
                converted: base,
                txst_object_id: 0x900,
                ltex_object_id: 0x901,
                grass_object_ids: Vec::new(),
            },
            EmittedTexture {
                converted: variant,
                txst_object_id: 0x900,
                ltex_object_id: 0x902,
                grass_object_ids: vec![0x903],
            },
        ];

        let output_dir = std::env::temp_dir().join("terrain_native_collect_variants");
        let mut output = AuthoringOutput::collect_only(output_dir);
        write_texture_records(&mut output, &options, &textures, "APPALACHIA")
            .expect("texture records");
        let collected = output.finish();

        assert_eq!(
            collected
                .records
                .iter()
                .filter(|record| record.signature == "TXST")
                .count(),
            1
        );
        let ltex_records: Vec<_> = collected
            .records
            .iter()
            .filter(|record| record.signature == "LTEX")
            .collect();
        assert_eq!(ltex_records.len(), 2);
        let no_grass = ltex_records
            .iter()
            .find(|record| record.yaml.contains("form_id: \"000901\""))
            .unwrap();
        let grass_variant = ltex_records
            .iter()
            .find(|record| record.yaml.contains("form_id: \"000902\""))
            .unwrap();
        assert!(!no_grass.yaml.contains("- Grass:\n"));
        assert!(
            grass_variant
                .yaml
                .contains("eid: LForestGrass01_GC_GCForestGrass01\n")
        );
        assert!(grass_variant.yaml.contains("- Grass:\n"));
        assert!(grass_variant.yaml.contains("      object_id: \"000903\"\n"));
    }

    #[test]
    fn source_ltex_form_key_uses_btd_layout_owner() {
        assert_eq!(
            source_ltex_form_key(0xFF00_ABCD, false),
            "00ABCD:SeventySix.esm"
        );
        assert_eq!(
            source_ltex_form_key(0xFF00_ABCD, true),
            "00ABCD:Starfield.esm"
        );
    }

    #[test]
    fn preserved_terrain_ids_read_world_cell_and_land_ids() {
        let world_dir = std::env::temp_dir().join(format!(
            "terrain_native_preserved_ids_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be after epoch")
                .as_nanos()
        ));
        let cell_dir = world_dir.join("0, -1").join("0, -1").join("3, -2");
        fs::create_dir_all(&cell_dir).expect("temp source cell dir");
        fs::write(
            world_dir.join("RecordData.yaml"),
            "signature: WRLD\nform_id: \"25DA15:SeventySix.esm\"\neid: APPALACHIA\n",
        )
        .expect("source world yaml");
        fs::write(
            cell_dir.join("RecordData.yaml"),
            "signature: CELL\nform_id: \"52ACBF:SeventySix.esm\"\neid: AppalachiaCell3Minus2\nsubrecords:\n  - signature: XCLC\n    data_hex: \"03000000FEFFFFFF00000000\"\nLandscape:\n  signature: LAND\n  form_id: \"52ACC0:SeventySix.esm\"\n",
        )
        .expect("source cell yaml");
        let options = ConvertOptions {
            btd_path: String::new(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esm".to_string(),
            worldspace_editor_id: "B21_Test".to_string(),
            source_min_x: 3,
            source_min_y: -2,
            source_max_x: 3,
            source_max_y: -2,
            first_form_id: 0x800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: false,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: true,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: world_dir.to_string_lossy().into_owned(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };

        let preserved = load_preserved_terrain_ids(&options).expect("preserved ids");
        let cell = preserved.cells.get(&(3, -2)).expect("cell ids");
        let plan = build_terrain_id_plan(&options, &preserved, 1, 1).expect("id plan");
        let _ = fs::remove_dir_all(&world_dir);

        assert_eq!(preserved.world_form_id, Some(0x25DA15));
        assert_eq!(preserved.world_editor_id.as_deref(), Some("APPALACHIA"));
        assert_eq!(cell.cell_form_id, Some(0x52ACBF));
        assert_eq!(
            cell.cell_editor_id.as_deref(),
            Some("AppalachiaCell3Minus2")
        );
        assert_eq!(cell.land_form_id, Some(0x52ACC0));
        assert_eq!(plan.world_form_id, 0x25DA15);
        assert_eq!(plan.world_editor_id(&options), "APPALACHIA");
        assert_eq!(plan.cell_form_id(&options, 3, -2, 0), 0x52ACBF);
        assert_eq!(
            plan.cell_editor_id(&options, 3, -2).as_deref(),
            Some("AppalachiaCell3Minus2")
        );
        assert_eq!(plan.cell_editor_id(&options, 4, -2), None);
        assert_eq!(plan.land_form_id(&options, 3, -2, 0), 0x52ACC0);
        assert_eq!(plan.next_object_id_after_terrain, 0x52ACC1);
    }

    #[test]
    fn preserved_terrain_ids_load_from_source_plugin_payload_without_authoring_dir() {
        let options = ConvertOptions {
            btd_path: String::new(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esm".to_string(),
            worldspace_editor_id: "B21_Test".to_string(),
            source_min_x: 3,
            source_min_y: -2,
            source_max_x: 3,
            source_max_y: -2,
            first_form_id: 0x800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: false,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: true,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: serde_json::json!({
                "world_form_id": 0x25DA15u32,
                "world_editor_id": "APPALACHIA",
                "cells": [{
                    "x": 3,
                    "y": -2,
                    "cell_form_id": 0x52ACBFu32,
                    "cell_editor_id": "AppalachiaCell3Minus2",
                    "land_form_id": 0x52ACC0u32
                }]
            })
            .to_string(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };

        let preserved = load_preserved_terrain_ids(&options).expect("preserved ids");
        let cell = preserved.cells.get(&(3, -2)).expect("cell ids");
        let plan = build_terrain_id_plan(&options, &preserved, 1, 1).expect("id plan");

        assert_eq!(preserved.world_form_id, Some(0x25DA15));
        assert_eq!(preserved.world_editor_id.as_deref(), Some("APPALACHIA"));
        assert_eq!(cell.cell_form_id, Some(0x52ACBF));
        assert_eq!(
            cell.cell_editor_id.as_deref(),
            Some("AppalachiaCell3Minus2")
        );
        assert_eq!(cell.land_form_id, Some(0x52ACC0));
        assert_eq!(plan.world_form_id, 0x25DA15);
        assert_eq!(plan.cell_form_id(&options, 3, -2, 0), 0x52ACBF);
        assert_eq!(plan.land_form_id(&options, 3, -2, 0), 0x52ACC0);
        assert_eq!(plan.next_object_id_after_terrain, 0x52ACC1);
    }

    #[test]
    fn terrain_id_plan_allocates_missing_land_from_reserved_floor() {
        let options = ConvertOptions {
            btd_path: String::new(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esm".to_string(),
            worldspace_editor_id: "B21_Test".to_string(),
            source_min_x: 3,
            source_min_y: -2,
            source_max_x: 3,
            source_max_y: -2,
            first_form_id: 0x800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: false,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: true,
            reserved_object_ids: vec![0x9FFFFF],
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };
        let mut preserved = PreservedTerrainIds::default();
        preserved.world_form_id = Some(0x25DA15);
        preserved.used_object_ids.extend([0x25DA15, 0x52ACBF]);
        preserved.cells.insert(
            (3, -2),
            PreservedCellIds {
                cell_form_id: Some(0x52ACBF),
                cell_editor_id: Some("AppalachiaCell3Minus2".to_string()),
                land_form_id: None,
            },
        );

        let plan = build_terrain_id_plan(&options, &preserved, 1, 1).expect("id plan");

        assert_eq!(plan.world_form_id, 0x25DA15);
        assert_eq!(plan.cell_form_id(&options, 3, -2, 0), 0x52ACBF);
        assert_eq!(plan.land_form_id(&options, 3, -2, 0), 0xA00000);
        assert_eq!(plan.next_object_id_after_terrain, 0xA00001);
    }

    #[test]
    fn preserved_terrain_ids_do_not_treat_world_cell_references_as_cells() {
        let world_dir = std::env::temp_dir().join(format!(
            "terrain_native_world_cell_refs_preserved_ids_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&world_dir).expect("temp source world dir");
        fs::write(
            world_dir.join("RecordData.yaml"),
            r#"form_id: "25DA15"
eid: APPALACHIA
fields:
- Cell:
    References:
    - Ref:
        reference:
          plugin: SeventySix.esm
          object_id: 52A823
      "Y": 7
    - Ref:
        reference:
          plugin: SeventySix.esm
          object_id: 52A80B
      X: 3
"#,
        )
        .expect("source world yaml");
        let options = ConvertOptions {
            btd_path: String::new(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esm".to_string(),
            worldspace_editor_id: "B21_Test".to_string(),
            source_min_x: 3,
            source_min_y: 7,
            source_max_x: 3,
            source_max_y: 7,
            first_form_id: 0x800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: false,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: true,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: world_dir.to_string_lossy().into_owned(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };

        let preserved = load_preserved_terrain_ids(&options).expect("preserved ids");
        let plan = build_terrain_id_plan(&options, &preserved, 1, 1).expect("id plan");
        let _ = fs::remove_dir_all(&world_dir);

        assert_eq!(preserved.world_form_id, Some(0x25DA15));
        assert_eq!(preserved.world_editor_id.as_deref(), Some("APPALACHIA"));
        assert!(!preserved.cells.contains_key(&(3, 7)));
        assert_eq!(plan.world_form_id, 0x25DA15);
        assert_eq!(plan.cell_form_id(&options, 3, 7, 0), 0x25DA16);
        assert_eq!(plan.cell_editor_id(&options, 3, 7), None);
    }

    #[test]
    fn terrain_id_plan_reallocates_preserved_ids_reserved_by_target_handle() {
        let options = ConvertOptions {
            btd_path: String::new(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esm".to_string(),
            worldspace_editor_id: "APPALACHIA".to_string(),
            source_min_x: 3,
            source_min_y: -2,
            source_max_x: 3,
            source_max_y: -2,
            first_form_id: 0x800,
            world_form_id: 0x25DA15,
            first_cell_form_id: 0x300000,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: false,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: true,
            reserved_object_ids: vec![0x52ACBF, 0x52ACC0],
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };
        let mut preserved = PreservedTerrainIds::default();
        preserved.world_form_id = Some(0x25DA15);
        preserved.used_object_ids.extend([0x52ACBF, 0x52ACC0]);
        preserved.cells.insert(
            (3, -2),
            PreservedCellIds {
                cell_form_id: Some(0x52ACBF),
                cell_editor_id: Some("AppalachiaCell3Minus2".to_string()),
                land_form_id: Some(0x52ACC0),
            },
        );

        let plan = build_terrain_id_plan(&options, &preserved, 1, 1).expect("id plan");

        assert_eq!(plan.world_form_id, 0x25DA15);
        assert_eq!(plan.cell_form_id(&options, 3, -2, 0), 0x52ACC1);
        assert_eq!(plan.land_form_id(&options, 3, -2, 0), 0x52ACC2);
        assert_eq!(plan.next_object_id_after_terrain, 0x52ACC3);
    }

    #[test]
    fn preserved_terrain_ids_read_signatureless_projected_cells() {
        let world_dir = std::env::temp_dir().join(format!(
            "terrain_native_signatureless_preserved_ids_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be after epoch")
                .as_nanos()
        ));
        let cell_dir = world_dir.join("0, -1").join("0, -1").join("1, -1");
        let zero_cell_dir = world_dir.join("0, 0").join("0, 0").join("0, 0");
        fs::create_dir_all(&cell_dir).expect("temp source cell dir");
        fs::create_dir_all(&zero_cell_dir).expect("temp zero source cell dir");
        fs::write(
            world_dir.join("RecordData.yaml"),
            "form_id: \"25DA15\"\neid: APPALACHIA\n",
        )
        .expect("source world yaml");
        fs::write(
            cell_dir.join("RecordData.yaml"),
            r#"form_id: "000900"
eid: PreservedSourceCell
fields:
- Grid:
    X: 1
    "Y": -1
Temporary:
- form_id: "000950"
  signature: REFR
"#,
        )
        .expect("source cell yaml");
        fs::write(
            zero_cell_dir.join("RecordData.yaml"),
            r#"form_id: "000901"
fields:
- Grid:
"#,
        )
        .expect("source zero cell yaml");
        let options = ConvertOptions {
            btd_path: String::new(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esm".to_string(),
            worldspace_editor_id: "B21_Test".to_string(),
            source_min_x: 1,
            source_min_y: -1,
            source_max_x: 1,
            source_max_y: -1,
            first_form_id: 0x800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: false,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: true,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: world_dir.to_string_lossy().into_owned(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };

        let preserved = load_preserved_terrain_ids(&options).expect("preserved ids");
        let plan = build_terrain_id_plan(&options, &preserved, 1, 1).expect("id plan");
        let _ = fs::remove_dir_all(&world_dir);

        assert_eq!(
            preserved.cells.get(&(1, -1)).unwrap().cell_form_id,
            Some(0x900)
        );
        assert_eq!(
            preserved.cells.get(&(0, 0)).unwrap().cell_form_id,
            Some(0x901)
        );
        assert!(preserved.used_object_ids.contains(&0x950));
        assert_eq!(plan.cell_form_id(&options, 1, -1, 0), 0x900);
        assert_eq!(
            plan.cell_editor_id(&options, 1, -1).as_deref(),
            Some("PreservedSourceCell")
        );
        assert_eq!(plan.land_form_id(&options, 1, -1, 0), 0x25DA16);
        assert_eq!(plan.next_object_id_after_terrain, 0x25DA17);
    }

    #[test]
    fn preserved_terrain_ids_disabled_uses_generated_range() {
        let options = ConvertOptions {
            btd_path: String::new(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esm".to_string(),
            worldspace_editor_id: "B21_Test".to_string(),
            source_min_x: 3,
            source_min_y: -2,
            source_max_x: 3,
            source_max_y: -2,
            first_form_id: 0x800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: false,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: false,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };

        let preserved = load_preserved_terrain_ids(&options).expect("preserved ids");
        let plan = build_terrain_id_plan(&options, &preserved, 1, 1).expect("id plan");

        assert_eq!(plan.world_form_id, 0x800);
        assert_eq!(plan.cell_form_id(&options, 3, -2, 0), 0x801);
        assert_eq!(
            plan.cell_editor_id(&options, 3, -2).as_deref(),
            Some("B21_TestCellXP003YN002")
        );
        assert_eq!(plan.land_form_id(&options, 3, -2, 0), 0x802);
        assert_eq!(plan.next_object_id_after_terrain, 0x803);
    }

    #[test]
    fn generated_terrain_ids_can_reuse_world_and_reserve_new_cell_range() {
        let options = ConvertOptions {
            btd_path: String::new(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esm".to_string(),
            worldspace_editor_id: "B21_Test".to_string(),
            source_min_x: 3,
            source_min_y: -2,
            source_max_x: 3,
            source_max_y: -2,
            first_form_id: 0x800,
            world_form_id: 0x25DA15,
            first_cell_form_id: 0x300000,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: false,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: false,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };

        let preserved = load_preserved_terrain_ids(&options).expect("preserved ids");
        let plan = build_terrain_id_plan(&options, &preserved, 1, 1).expect("id plan");

        assert_eq!(plan.world_form_id, 0x25DA15);
        assert_eq!(plan.cell_form_id(&options, 3, -2, 0), 0x300000);
        assert_eq!(plan.land_form_id(&options, 3, -2, 0), 0x300001);
        assert_eq!(plan.next_object_id_after_terrain, 0x300002);
    }

    #[test]
    fn generated_cell_editor_ids_keep_negative_coordinates_unique_after_ck_sanitizing() {
        let ids = [
            cell_editor_id("B21_TestWorld", -2, -2),
            cell_editor_id("B21_TestWorld", 2, -2),
            cell_editor_id("B21_TestWorld", -2, 2),
            cell_editor_id("B21_TestWorld", 2, 2),
        ];
        let sanitized = ids
            .iter()
            .map(|id| {
                id.chars()
                    .filter(|ch| ch.is_ascii_alphanumeric())
                    .collect::<String>()
            })
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(ids[0], "B21_TestWorldCellXN002YN002");
        assert_eq!(ids[3], "B21_TestWorldCellXP002YP002");
        assert_eq!(sanitized.len(), ids.len());
    }

    #[test]
    fn land_data_flags_advertise_emitted_payloads() {
        assert_eq!(
            land_data_flags(false, false),
            LAND_FLAG_HAS_VERTEX_NORMALS_HEIGHT_MAP
                | LAND_FLAG_UNKNOWN_4
                | LAND_FLAG_AUTO_CALC_NORMALS
        );
        assert_eq!(
            land_data_flags(true, false),
            LAND_FLAG_HAS_VERTEX_NORMALS_HEIGHT_MAP
                | LAND_FLAG_HAS_LAYERS
                | LAND_FLAG_UNKNOWN_4
                | LAND_FLAG_AUTO_CALC_NORMALS
        );
        assert_eq!(
            land_data_flags(true, true),
            LAND_FLAG_HAS_VERTEX_NORMALS_HEIGHT_MAP
                | LAND_FLAG_HAS_VERTEX_COLORS
                | LAND_FLAG_HAS_LAYERS
                | LAND_FLAG_UNKNOWN_4
                | LAND_FLAG_AUTO_CALC_NORMALS
        );
        assert_eq!(u32_hex(land_data_flags(true, true)), "1F000000");
    }

    #[test]
    fn fo76_vertex_color_neutral_grey_maps_near_fo4_white() {
        assert_eq!(fo76_vclr_to_fo4_vclr(0), [0, 0, 0]);
        assert_eq!(fo76_vclr_to_fo4_vclr(0x7FFF), [255, 255, 255]);

        let neutral = fo76_vclr_to_fo4_vclr(0xBDEF);
        assert!(neutral[0] > 240, "{neutral:?}");
        assert_eq!(neutral[0], neutral[1]);
        assert_eq!(neutral[1], neutral[2]);
    }

    #[test]
    fn land_texture_layer_fields_use_plugin_local_form_references() {
        let field = land_texture_layer_field("BTXT", 0x008A26, "B21_Test.esp", 0, -1);

        assert!(field.contains("Texture:\n          reference:\n"));
        assert!(field.contains("plugin: B21_Test.esp\n"));
        assert!(field.contains("object_id: \"008A26\"\n"));
        assert!(field.contains("UnknownByte3: 2\n"));
        assert!(!field.contains("01008A26"));

        let field = land_texture_layer_field("ATXT", 0x008A27, "B21_Test.esp", 3, 1);
        assert!(field.contains("Quadrant: 3\n"));
        assert!(field.contains("UnknownByte3: 0\n"));
        assert!(field.contains("Layer: 1\n"));
    }

    fn test_emitted_texture(
        source_gcvr_form_key: Option<&str>,
        ltex_object_id: u32,
        grass_object_ids: Vec<u32>,
    ) -> EmittedTexture {
        EmittedTexture {
            converted: ConvertedTerrainTexture {
                source_ltex_form_key: "001234:SeventySix.esm".to_string(),
                source_ltex_editor_id: "LForestGrass01".to_string(),
                source_gcvr_form_key: source_gcvr_form_key.map(str::to_owned),
                source_gcvr_editor_id: source_gcvr_form_key.map(|_| "GCForestGrass01".to_string()),
                source_txst_form_key: "001235:SeventySix.esm".to_string(),
                source_txst_editor_id: "LandscapeForestGrass01".to_string(),
                suffix: "ForestGrass".to_string(),
                diffuse_rel_path: "textures/terrain/test_d.dds".to_string(),
                normal_rel_path: "textures/terrain/test_n.dds".to_string(),
                specgloss_rel_path: "textures/terrain/test_s.dds".to_string(),
                glow_rel_path: "textures/terrain/test_g.dds".to_string(),
                material_type_object_id: None,
                havok_friction: 30,
                havok_restitution: 30,
                grass: Vec::new(),
            },
            txst_object_id: 0x900,
            ltex_object_id,
            grass_object_ids,
        }
    }

    fn ground_cover_test_set(source_slot: usize, ground_cover_index: u8) -> CellTextureSet {
        let mut quad = QuadrantTextureSet {
            base: None,
            base_source_slot: None,
            additional: [None; 5],
            additional_source_slots: [None; 5],
            ground_cover: [None; 8],
        };
        quad.ground_cover[source_slot] = Some(ground_cover_index);
        CellTextureSet {
            quadrants: vec![quad],
        }
    }

    fn paint_alpha_and_mask_rows(
        alphas: &mut [u16],
        ground_cover_mask: &mut [u8],
        rows: usize,
        mask_bit: u8,
    ) {
        for y in 0..rows {
            for x in 0..CELL_SOURCE_QUADRANT_SAMPLES {
                alphas[y * CELL_SOURCE_SAMPLES + x] = 1;
                ground_cover_mask[y * CELL_SOURCE_SAMPLES + x] = mask_bit;
            }
        }
    }

    fn gcvr_keys_for_candidates(candidates: &[u8]) -> Vec<String> {
        candidates
            .iter()
            .map(|candidate| match candidate {
                0 => "011C67".to_string(),
                other => format!("{other:06X}"),
            })
            .collect()
    }

    #[test]
    fn low_alpha_ground_cover_layer_falls_back_to_no_grass_ltex() {
        let source_slot = 3usize;
        let mask_bit = ground_cover_mask_bit_for_source_slot(source_slot as u8).unwrap();
        let set = ground_cover_test_set(source_slot, 0);
        let mut ground_cover_mask = vec![0u8; CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES];
        let mut alphas = vec![0u16; CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES];
        paint_alpha_and_mask_rows(&mut alphas, &mut ground_cover_mask, 1, mask_bit);
        let textures = vec![
            test_emitted_texture(None, 0x901, Vec::new()),
            test_emitted_texture(Some("011C67:SeventySix.esm"), 0x902, vec![0x903]),
        ];
        let textures_by_source_usage = index_textures_by_source_usage(&textures);

        let candidates = effective_ground_cover_indices_for_layer(
            &set,
            &ground_cover_mask,
            0,
            Some(source_slot as u8),
            Some(0),
            Some((&alphas, 0)),
        );
        let selected = texture_for_source_usage(
            "001234",
            &gcvr_keys_for_candidates(&candidates),
            &textures_by_source_usage,
        )
        .expect("no-grass texture fallback");

        assert!(candidates.is_empty());
        assert_eq!(selected.ltex_object_id, 0x901);
        assert!(selected.grass_object_ids.is_empty());
    }

    #[test]
    fn meaningful_alpha_ground_cover_layer_emits_grass_ltex() {
        let source_slot = 3usize;
        let mask_bit = ground_cover_mask_bit_for_source_slot(source_slot as u8).unwrap();
        let set = ground_cover_test_set(source_slot, 0);
        let mut ground_cover_mask = vec![0u8; CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES];
        let mut alphas = vec![0u16; CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES];
        paint_alpha_and_mask_rows(&mut alphas, &mut ground_cover_mask, 16, mask_bit);
        let textures = vec![
            test_emitted_texture(None, 0x901, Vec::new()),
            test_emitted_texture(Some("011C67:SeventySix.esm"), 0x902, vec![0x903]),
        ];
        let textures_by_source_usage = index_textures_by_source_usage(&textures);

        let candidates = effective_ground_cover_indices_for_layer(
            &set,
            &ground_cover_mask,
            0,
            Some(source_slot as u8),
            Some(0),
            Some((&alphas, 0)),
        );
        let selected = texture_for_source_usage(
            "001234",
            &gcvr_keys_for_candidates(&candidates),
            &textures_by_source_usage,
        )
        .expect("grass texture variant");

        assert_eq!(candidates, vec![0]);
        assert_eq!(selected.ltex_object_id, 0x902);
        assert_eq!(selected.grass_object_ids, vec![0x903]);
    }

    #[test]
    fn ground_cover_candidates_follow_layer_source_slot() {
        let mut quad = QuadrantTextureSet {
            base: None,
            base_source_slot: None,
            additional: [None; 5],
            additional_source_slots: [None; 5],
            ground_cover: [None; 8],
        };
        quad.ground_cover[3] = Some(11);
        quad.ground_cover[4] = Some(22);
        let set = CellTextureSet {
            quadrants: vec![quad],
        };
        let mut ground_cover_mask = vec![0u8; CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES];
        for y in 0..CELL_SOURCE_QUADRANT_SAMPLES {
            for x in 0..CELL_SOURCE_QUADRANT_SAMPLES {
                ground_cover_mask[y * CELL_SOURCE_SAMPLES + x] = 1 << 4;
            }
        }
        let alphas = vec![7u16; CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES];

        assert_eq!(ground_cover_mask_bit_for_source_slot(3), Some(1 << 4));
        assert_eq!(
            effective_ground_cover_indices_for_layer(
                &set,
                &ground_cover_mask,
                0,
                Some(3),
                Some(11),
                Some((&alphas, 0)),
            ),
            vec![11]
        );
    }

    #[test]
    fn ground_cover_candidates_reject_other_source_slot_mask_bits() {
        let mut quad = QuadrantTextureSet {
            base: None,
            base_source_slot: None,
            additional: [None; 5],
            additional_source_slots: [None; 5],
            ground_cover: [None; 8],
        };
        quad.ground_cover[3] = Some(11);
        quad.ground_cover[4] = Some(22);
        let set = CellTextureSet {
            quadrants: vec![quad],
        };
        let mut ground_cover_mask = vec![0u8; CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES];
        for y in 0..CELL_SOURCE_QUADRANT_SAMPLES {
            for x in 0..CELL_SOURCE_QUADRANT_SAMPLES {
                ground_cover_mask[y * CELL_SOURCE_SAMPLES + x] = 1 << 4;
            }
        }
        let alphas = vec![7u16; CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES];

        assert_eq!(
            effective_ground_cover_indices_for_layer(
                &set,
                &ground_cover_mask,
                0,
                Some(4),
                Some(22),
                Some((&alphas, 0)),
            ),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn ground_cover_candidates_keep_sparse_source_slot_coverage() {
        let mut quad = QuadrantTextureSet {
            base: None,
            base_source_slot: None,
            additional: [None; 5],
            additional_source_slots: [None; 5],
            ground_cover: [None; 8],
        };
        quad.ground_cover[3] = Some(11);
        let set = CellTextureSet {
            quadrants: vec![quad],
        };
        let mut ground_cover_mask = vec![0u8; CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES];
        for y in 0..16 {
            for x in 0..CELL_SOURCE_QUADRANT_SAMPLES {
                ground_cover_mask[y * CELL_SOURCE_SAMPLES + x] = 1 << 4;
            }
        }
        let alphas = vec![7u16; CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES];

        assert_eq!(
            effective_ground_cover_indices_for_layer(
                &set,
                &ground_cover_mask,
                0,
                Some(3),
                Some(11),
                Some((&alphas, 0)),
            ),
            vec![11]
        );
    }

    #[test]
    fn ground_cover_candidates_reject_trace_source_slot_coverage() {
        let mut quad = QuadrantTextureSet {
            base: None,
            base_source_slot: None,
            additional: [None; 5],
            additional_source_slots: [None; 5],
            ground_cover: [None; 8],
        };
        quad.ground_cover[3] = Some(11);
        let set = CellTextureSet {
            quadrants: vec![quad],
        };
        let mut ground_cover_mask = vec![0u8; CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES];
        for y in 0..6 {
            for x in 0..CELL_SOURCE_QUADRANT_SAMPLES {
                ground_cover_mask[y * CELL_SOURCE_SAMPLES + x] = 1 << 4;
            }
        }
        let alphas = vec![7u16; CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES];

        assert_eq!(
            effective_ground_cover_indices_for_layer(
                &set,
                &ground_cover_mask,
                0,
                Some(3),
                Some(11),
                Some((&alphas, 0)),
            ),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn full_extent_sentinel_expands_to_btd_header_bounds() {
        let header = BtdHeader {
            version: 1,
            world_height_min: 0.0,
            world_height_max: 1024.0,
            resolution_x: 0,
            resolution_y: 0,
            cell_min_x: -100,
            cell_min_y: -50,
            cell_max_x: 100,
            cell_max_y: 75,
            cells_x: 201,
            cells_y: 126,
            ltex_count: 0,
            ltex_offset: 0,
            cell_height_minmax_offset: 0,
            ltex_map_offset: 0,
            gcvr_count: 0,
            gcvr_offset: 0,
            gcvr_map_offset: 0,
            height_lod4_offset: 0,
            land_texture_lod4_offset: 0,
            vertex_color_lod4_offset: 0,
            zlib_table_offset: 0,
            zlib_lod3_offset: 0,
            zlib_lod2_offset: 0,
            zlib_lod1_offset: 0,
            zlib_lod0_offset: 0,
            zlib_data_offset: 0,
            is_starfield_layout: false,
        };
        let mut options = ConvertOptions {
            btd_path: String::new(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esp".to_string(),
            worldspace_editor_id: "B21_Test".to_string(),
            source_min_x: 0,
            source_min_y: 0,
            source_max_x: -1,
            source_max_y: -1,
            first_form_id: 0x800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: true,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: true,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };

        resolve_full_extent_sentinel(&mut options, &header);

        assert_eq!(options.source_min_x, -100);
        assert_eq!(options.source_min_y, -50);
        assert_eq!(options.source_max_x, 100);
        assert_eq!(options.source_max_y, 75);
    }

    #[test]
    fn vhgt_delta_clamp_tracks_quantized_encoder_state() {
        let mut heights = vec![0.0; LAND_CELL_VERTICES * LAND_CELL_VERTICES];
        heights[1] = 4.0;
        heights[2] = -2000.0;

        let encodable = clamp_vhgt_delta_stream(&heights);

        encode_vhgt(&encodable).expect("clamped heights must be encodable");
    }

    #[test]
    fn vhgt_delta_clamp_reports_overflow_direction() {
        let heights = vec![
            0.0,
            (VHGT_MAX_DELTA_STEP + 1.0) * VHGT_HEIGHT_STEP,
            (VHGT_MIN_DELTA_STEP - 1.0) * VHGT_HEIGHT_STEP,
        ];

        let (_encodable, stats) = clamp_vhgt_delta_stream_with_stats(&heights);

        assert_eq!(stats.overflows, 1);
        assert_eq!(stats.underflows, 1);
    }

    #[test]
    fn vhgt_delta_clamp_resets_at_land_row_start() {
        let mut heights = vec![0.0; LAND_CELL_VERTICES * LAND_CELL_VERTICES];
        for x in 0..LAND_CELL_VERTICES {
            heights[x] = x as f32 * VHGT_MAX_DELTA_STEP * VHGT_HEIGHT_STEP;
        }
        heights[LAND_CELL_VERTICES] = VHGT_HEIGHT_STEP;

        let (encodable, stats) = clamp_vhgt_delta_stream_with_stats(&heights);

        assert_eq!(stats.underflows, 0);
        assert_eq!(stats.overflows, 0);
        assert_eq!(encodable[LAND_CELL_VERTICES], VHGT_HEIGHT_STEP);
    }

    #[test]
    fn generated_cell_yaml_without_water_manifest_is_dry() {
        let cell_dir = std::env::temp_dir().join(format!(
            "terrain_native_cell_yaml_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&cell_dir).expect("temp cell dir");
        let options = ConvertOptions {
            btd_path: String::new(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esp".to_string(),
            worldspace_editor_id: "B21_Test".to_string(),
            source_min_x: 0,
            source_min_y: 0,
            source_max_x: 0,
            source_max_y: 0,
            first_form_id: 0x800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: false,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: true,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };

        let cell_eid = cell_editor_id(&options.worldspace_editor_id, 0, 0);
        let output_dir = cell_dir
            .parent()
            .expect("temp cell dir should have a parent")
            .to_path_buf();
        let relative_cell_dir = cell_dir
            .file_name()
            .map(PathBuf::from)
            .expect("temp cell dir should have a file name");
        let mut output = AuthoringOutput::write_files(output_dir);
        write_cell_yaml(
            &mut output,
            &relative_cell_dir,
            &options,
            0,
            0,
            Some(&cell_eid),
            0x801,
            0x802,
            &[],
            &[],
            &[],
            false,
            &[],
            None,
        )
        .expect("cell yaml");
        let payload = fs::read_to_string(cell_dir.join("RecordData.yaml")).expect("cell yaml text");
        let _ = fs::remove_dir_all(&cell_dir);

        assert!(payload.contains("eid: B21_TestCellXP000YP000\n"));
        assert!(payload.contains("  - signature: EDID\n"));
        assert!(payload.contains("  - signature: DATA\n    data_hex: \"0200\"\n"));
        assert!(
            payload.contains("  - signature: XCLC\n    data_hex: \"000000000000000000000000\"\n")
        );
        assert!(!payload.contains("  - signature: LTMP\n"));
        assert!(payload.contains("  - signature: XCLW\n    data_hex: \"FFFF7F7F\"\n"));
        assert!(!payload.contains("  - signature: XCWT\n"));
    }

    #[test]
    fn source_backed_anonymous_cell_yaml_omits_editor_id() {
        let cell_dir = std::env::temp_dir().join(format!(
            "terrain_native_anonymous_cell_yaml_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&cell_dir).expect("temp cell dir");
        let options = ConvertOptions {
            btd_path: String::new(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esp".to_string(),
            worldspace_editor_id: "B21_Test".to_string(),
            source_min_x: 0,
            source_min_y: 0,
            source_max_x: 0,
            source_max_y: 0,
            first_form_id: 0x800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: false,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: true,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };
        let output_dir = cell_dir
            .parent()
            .expect("temp cell dir should have a parent")
            .to_path_buf();
        let relative_cell_dir = cell_dir
            .file_name()
            .map(PathBuf::from)
            .expect("temp cell dir should have a file name");
        let mut output = AuthoringOutput::write_files(output_dir);

        write_cell_yaml(
            &mut output,
            &relative_cell_dir,
            &options,
            0,
            0,
            None,
            0x801,
            0x802,
            &[],
            &[],
            &[],
            false,
            &[],
            None,
        )
        .expect("cell yaml");
        let payload = fs::read_to_string(cell_dir.join("RecordData.yaml")).expect("cell yaml text");
        let _ = fs::remove_dir_all(&cell_dir);

        assert!(!payload.contains("\neid: "));
        assert!(!payload.contains("  - signature: EDID\n"));
        assert!(payload.contains("subrecords:\n  - signature: DATA\n"));
        assert!(
            payload.contains("  - signature: XCLC\n    data_hex: \"000000000000000000000000\"\n")
        );
    }

    #[test]
    fn generated_cell_yaml_with_water_manifest_uses_real_height_and_water_type() {
        let cell_dir = std::env::temp_dir().join(format!(
            "terrain_native_water_cell_yaml_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&cell_dir).expect("temp cell dir");
        let options = ConvertOptions {
            btd_path: String::new(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esp".to_string(),
            worldspace_editor_id: "B21_Test".to_string(),
            source_min_x: 0,
            source_min_y: 0,
            source_max_x: 0,
            source_max_y: 0,
            first_form_id: 0x800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: false,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: true,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };
        let water = WaterCell {
            height: 1600.0,
            water_object_id: 0x0C8633,
        };
        let cell_eid = cell_editor_id(&options.worldspace_editor_id, 0, 0);
        let output_dir = cell_dir
            .parent()
            .expect("temp cell dir should have a parent")
            .to_path_buf();
        let relative_cell_dir = cell_dir
            .file_name()
            .map(PathBuf::from)
            .expect("temp cell dir should have a file name");
        let mut output = AuthoringOutput::write_files(output_dir);

        write_cell_yaml(
            &mut output,
            &relative_cell_dir,
            &options,
            0,
            0,
            Some(&cell_eid),
            0x801,
            0x802,
            &[],
            &[],
            &[],
            false,
            &[],
            Some(&water),
        )
        .expect("cell yaml");
        let payload = fs::read_to_string(cell_dir.join("RecordData.yaml")).expect("cell yaml text");
        let _ = fs::remove_dir_all(&cell_dir);

        assert!(payload.contains("  - signature: DATA\n    data_hex: \"0200\"\n"));
        assert!(!payload.contains("  - signature: LTMP\n"));
        assert!(payload.contains("  - signature: XCLW\n    data_hex: \"0000C844\"\n"));
        assert!(payload.contains("  - signature: XCWT\n    data_hex: \"33860C00\"\n"));
        assert!(!payload.contains("FFFF7F7F"));
    }

    #[test]
    fn structured_cell_value_matches_legacy_yaml_for_binary_and_scalar_edges() {
        let options = texture_usage_options(Path::new("unused.btd"), -7, -7);
        let cell_eid = "B21_Terrain_Été_XN007YP013";
        let water = WaterCell {
            height: -123.45679,
            water_object_id: 0x01C8633,
        };
        let vnml = [0x00, 0x7F, 0x80, 0xFF];
        let vhgt = [0xFF, 0x00, 0xA5, 0x5A];
        let vclr = [0x01, 0xFE, 0x10, 0xEF];
        let texture_fields = vec![
            LandTextureField::Layer {
                signature: "BTXT",
                texture_object_id: 0x008A26,
                plugin_name: options.plugin_name.clone(),
                quadrant: 3,
                layer: -1,
            },
            LandTextureField::Layer {
                signature: "ATXT",
                texture_object_id: 0x008A27,
                plugin_name: options.plugin_name.clone(),
                quadrant: 3,
                layer: 0,
            },
            LandTextureField::AlphaLayerData {
                raw_hex: "00FF7F80A55A".to_owned(),
            },
        ];

        let mut output = AuthoringOutput::collect_only(PathBuf::new());
        write_cell_yaml(
            &mut output,
            Path::new("-1, 1/-1, 1/-7, 13"),
            &options,
            -7,
            13,
            Some(cell_eid),
            0x801,
            0x802,
            &vnml,
            &vhgt,
            &vclr,
            true,
            &texture_fields,
            Some(&water),
        )
        .expect("legacy cell YAML");
        let legacy = output.finish().records.pop().expect("legacy CELL payload");
        let parsed: serde_json::Value =
            serde_saphyr::from_str(&legacy.yaml).expect("parse legacy CELL YAML");
        let structured = cell_record_value(
            &options,
            -7,
            13,
            Some(cell_eid),
            0x801,
            0x802,
            &vnml,
            &vhgt,
            &vclr,
            true,
            &texture_fields,
            Some(&water),
        );

        assert_eq!(structured, parsed);
        assert_eq!(
            structured["subrecords"][3]["data_hex"],
            serde_json::json!(f32_hex(water.height))
        );
        assert_eq!(
            structured["Landscape"]["fields"][6]["AlphaLayerData"]["raw_hex"],
            "00FF7F80A55A"
        );
    }

    // ---- .btd4 dense sidecar ----------------------------------------

    /// Builds a valid one-cell BTD whose every LOD block decodes to a constant
    /// u16 height/land-alpha of `height_sample`. A constant block sidesteps
    /// re-deriving the BTD's intricate per-sample decode in the test while still
    /// exercising the real extraction + raw-sample path end to end.
    fn one_cell_btd_constant_height(height_sample: u16) -> Vec<u8> {
        one_cell_btd(height_sample, &[], [[0; 8]; 4])
    }

    fn one_cell_btd(
        height_sample: u16,
        ltex_form_ids: &[u32],
        quadrant_ltex_maps: [[u8; 8]; 4],
    ) -> Vec<u8> {
        btd_row_fixture(
            height_sample,
            ltex_form_ids,
            &[quadrant_ltex_maps],
            &[],
            &[[[0; 8]; 4]],
        )
    }

    fn btd_row_fixture(
        height_sample: u16,
        ltex_form_ids: &[u32],
        cell_ltex_maps: &[[[u8; 8]; 4]],
        gcvr_form_ids: &[u32],
        cell_gcvr_maps: &[[[u8; 8]; 4]],
    ) -> Vec<u8> {
        use flate2::Compression;
        use flate2::write::ZlibEncoder;
        use std::io::Write;

        assert_eq!(cell_ltex_maps.len(), cell_gcvr_maps.len());
        let cells_x = cell_ltex_maps.len();
        let ltex_count = ltex_form_ids.len();
        let gcvr_count = gcvr_form_ids.len();
        let cell_count = cells_x;
        let ltex_offset = 0x2cusize;
        let cell_height_minmax_offset = ltex_offset + ltex_count * 4;
        let ltex_map_offset = cell_height_minmax_offset + cell_count * 8;
        let gcvr_count_offset = ltex_map_offset + cell_count * 32;
        let gcvr_offset = gcvr_count_offset + 4;
        let gcvr_map_offset = gcvr_offset + gcvr_count * 4;
        let height_lod4_offset = gcvr_map_offset + cell_count * 32;
        let land_texture_lod4_offset = height_lod4_offset + cell_count * 128;
        let vertex_color_lod4_offset = land_texture_lod4_offset + cell_count * 128;
        let zlib_table_offset = vertex_color_lod4_offset + cell_count * 128;
        let lod3_count = cells_x.div_ceil(8) * 2;
        let lod2_count = cells_x.div_ceil(4) * 2;
        let lod1_count = cells_x.div_ceil(2);
        let lod0_count = cell_count * 2;
        let entry_count = lod3_count + lod2_count + lod1_count + lod0_count;
        let zlib_data_offset = zlib_table_offset + entry_count * 8;

        // A height/land block is 49152 bytes; the decoder reads it as u16 lines.
        // Filling it entirely with `height_sample` makes every decoded sample
        // equal to that value regardless of the line-walk pattern.
        let raw_block = {
            let mut b = vec![0u8; HEIGHT_LAND_BLOCK_LEN_TEST];
            for chunk in b.chunks_exact_mut(2) {
                chunk.copy_from_slice(&height_sample.to_le_bytes());
            }
            b
        };
        let compressed_block = {
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
            enc.write_all(&raw_block).unwrap();
            enc.finish().unwrap()
        };
        let raw_vclr_block = {
            let mut b = vec![0u8; 32768];
            for chunk in b.chunks_exact_mut(2) {
                chunk.copy_from_slice(&0xBDEFu16.to_le_bytes());
            }
            b
        };
        let compressed_vclr_block = {
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
            enc.write_all(&raw_vclr_block).unwrap();
            enc.finish().unwrap()
        };

        let mut bytes = vec![0u8; zlib_data_offset];
        bytes[0..4].copy_from_slice(b"BTDB");
        put_u32_le(&mut bytes, 0x04, 6); // version
        put_f32_le(&mut bytes, 0x08, 0.0); // world_height_min
        put_f32_le(&mut bytes, 0x0c, 1024.0); // world_height_max
        put_u32_le(&mut bytes, 0x10, (cells_x * 128) as u32); // resolution_x
        put_u32_le(&mut bytes, 0x14, 128); // resolution_y
        put_i32_le(&mut bytes, 0x18, 0); // cell_min_x
        put_i32_le(&mut bytes, 0x1c, 0); // cell_min_y
        put_i32_le(&mut bytes, 0x20, cells_x as i32 - 1); // cell_max_x
        put_i32_le(&mut bytes, 0x24, 0); // cell_max_y
        put_u32_le(&mut bytes, 0x28, ltex_count as u32);
        for (index, form_id) in ltex_form_ids.iter().copied().enumerate() {
            put_u32_le(&mut bytes, ltex_offset + index * 4, form_id);
        }
        for (cell_index, quadrant_maps) in cell_ltex_maps.iter().enumerate() {
            for (quadrant, texture_map) in quadrant_maps.iter().enumerate() {
                let offset = ltex_map_offset + cell_index * 32 + quadrant * 8;
                bytes[offset..offset + 8].copy_from_slice(texture_map);
            }
        }
        put_u32_le(&mut bytes, gcvr_count_offset, gcvr_count as u32);
        for (index, form_id) in gcvr_form_ids.iter().copied().enumerate() {
            put_u32_le(&mut bytes, gcvr_offset + index * 4, form_id);
        }
        for (cell_index, quadrant_maps) in cell_gcvr_maps.iter().enumerate() {
            for (quadrant, gcvr_map) in quadrant_maps.iter().enumerate() {
                let offset = gcvr_map_offset + cell_index * 32 + quadrant * 8;
                bytes[offset..offset + 8].copy_from_slice(gcvr_map);
            }
        }
        for chunk in bytes[vertex_color_lod4_offset..zlib_table_offset].chunks_exact_mut(2) {
            chunk.copy_from_slice(&0xBDEFu16.to_le_bytes());
        }

        // Point height/land entries at the 49152-byte block, and vertex-color
        // stream entries at the 32768-byte block.
        let height_block_off = 0u32; // relative to zlib_data_offset
        let height_block_len = compressed_block.len() as u32;
        let vclr_block_off = compressed_block.len() as u32;
        let vclr_block_len = compressed_vclr_block.len() as u32;
        for i in 0..entry_count {
            let entry = zlib_table_offset + i * 8;
            put_u32_le(&mut bytes, entry, height_block_off);
            put_u32_le(&mut bytes, entry + 4, height_block_len);
        }
        let lod2_offset = lod3_count;
        let terrain_color_entries = (1..lod3_count)
            .step_by(2)
            .chain((lod2_offset + 1..lod2_offset + lod2_count).step_by(2));
        for entry_index in terrain_color_entries {
            let entry = zlib_table_offset + entry_index * 8;
            put_u32_le(&mut bytes, entry, vclr_block_off);
            put_u32_le(&mut bytes, entry + 4, vclr_block_len);
        }
        bytes.extend_from_slice(&compressed_block);
        bytes.extend_from_slice(&compressed_vclr_block);
        bytes
    }

    fn texture_usage_options(
        btd_path: &Path,
        source_min_x: i32,
        source_max_x: i32,
    ) -> ConvertOptions {
        ConvertOptions {
            btd_path: btd_path.display().to_string(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esp".to_string(),
            worldspace_editor_id: "B21TestWorld".to_string(),
            source_min_x,
            source_min_y: 0,
            source_max_x,
            source_max_y: 0,
            first_form_id: 0x000800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: false,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: false,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        }
    }

    fn alpha_fallthrough_options() -> (tempfile::TempDir, ConvertOptions) {
        let temp = tempfile::tempdir().expect("tempdir");
        let btd_path = temp.path().join("alpha_fallthrough.btd");
        fs::write(
            &btd_path,
            one_cell_btd(
                0x6000,
                &[0x000100, 0x000200],
                base_and_additional_texture_maps(),
            ),
        )
        .expect("write BTD fixture");

        let base_diffuse_path = temp.path().join("base_d.dds");
        let overlay_diffuse_path = temp.path().join("overlay_d.dds");
        directxtex_native::write_dds_rgba_image(
            &base_diffuse_path,
            2,
            2,
            &[
                10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 1, 2, 3, 255,
            ],
            "R8G8B8A8_UNORM",
            false,
        )
        .expect("write base DDS fixture");
        directxtex_native::write_dds_rgba_image(
            &overlay_diffuse_path,
            2,
            2,
            &[10, 20, 30, 0, 40, 50, 60, 0, 70, 80, 90, 0, 1, 2, 3, 0],
            "R8G8B8A8_UNORM",
            false,
        )
        .expect("write transparent overlay DDS fixture");

        let manifest_path = temp.path().join("texture_manifest.json");
        let texture_bundle = |object_id: &str, editor_id: &str, diffuse_path: &Path| {
            serde_json::json!({
                "source_ltex_form_key": format!("{object_id}:SeventySix.esm"),
                "source_ltex_editor_id": editor_id,
                "source_txst_form_key": format!("{object_id}:SeventySix.esm"),
                "source_txst_editor_id": format!("{editor_id}TXST"),
                "diffuse_path": diffuse_path,
                "normal_path": "",
                "reflectivity_path": "",
                "lighting_path": "",
                "output_prefix": format!("textures/terrain/{editor_id}"),
                "output_material_path": null,
                "material_type_object_id": null,
                "havok_friction": 30,
                "havok_restitution": 30,
                "grass": [],
            })
        };
        fs::write(
            &manifest_path,
            serde_json::to_vec(&serde_json::json!({
                "textures": [
                    texture_bundle("000100", "B21_TestBase", &base_diffuse_path),
                    texture_bundle("000200", "B21_TestOverlay", &overlay_diffuse_path),
                ]
            }))
            .expect("serialize texture manifest"),
        )
        .expect("write texture manifest");

        let mut options = texture_usage_options(&btd_path, 0, 0);
        options.output_authoring_dir = temp.path().join("authoring").display().to_string();
        options.debug_output_dir = temp.path().join("debug").display().to_string();
        options.texture_manifest_path = manifest_path.display().to_string();
        options.emit_textures = true;
        (temp, options)
    }

    fn prepare_without_manifest(options: &ConvertOptions) -> PreparedTerrainTextureScan {
        let mut scan_options = options.clone();
        scan_options.emit_textures = false;
        scan_options.texture_manifest_path.clear();
        prepare_terrain_texture_scan(scan_options).expect("prepare texture scan")
    }

    fn assert_yaml_records_equal(
        left: &[AuthoringRecordPayload],
        right: &[AuthoringRecordPayload],
    ) {
        assert_eq!(left.len(), right.len());
        for (left, right) in left.iter().zip(right) {
            assert_eq!(left.signature, right.signature);
            assert_eq!(left.relative_path, right.relative_path);
            assert_eq!(left.yaml, right.yaml);
        }
    }

    fn report_without_measurements(report: &ConvertReport) -> serde_json::Value {
        let mut value = serde_json::to_value(report).expect("serialize report");
        let object = value.as_object_mut().expect("report object");
        object.remove("timings");
        object.remove("operation_counts");
        value
    }

    #[test]
    fn prepared_btd_reuse_matches_reopened_alpha_fallthrough() {
        let (_temp, options) = alpha_fallthrough_options();
        let prepared = prepare_without_manifest(&options);
        assert!(
            prepared
                .profile
                .usages
                .iter()
                .any(|usage| usage.ltex_form_key == "000200:SeventySix.esm"),
            "pass 1 must retain the overlay that needs a manifest alpha check in pass 2"
        );

        let mut reopened_records = Vec::new();
        let reopened_report = convert_btd_with_record_sink(options.clone(), &mut |record| {
            reopened_records.push(record);
            Ok(())
        })
        .expect("legacy reopened conversion");
        let mut prepared_records = Vec::new();
        let prepared_report =
            convert_prepared_btd_with_record_sink(prepared.source, options, &mut |record| {
                prepared_records.push(record);
                Ok(())
            })
            .expect("prepared conversion");

        assert_yaml_records_equal(&reopened_records, &prepared_records);
        assert_eq!(
            report_without_measurements(&reopened_report),
            report_without_measurements(&prepared_report)
        );
        let cell_yaml = prepared_records
            .iter()
            .find(|record| record.signature == "CELL")
            .expect("CELL record")
            .yaml
            .as_str();
        assert!(cell_yaml.contains("    - BTXT:"));
        assert!(
            !cell_yaml.contains("    - ATXT:"),
            "pass 2 must use the real transparent overlay alpha and fall through to the base"
        );
        assert_eq!(
            reopened_report
                .operation_counts
                .get("btd_cache.cached_tiles_start"),
            Some(&0)
        );
        assert!(
            prepared_report.operation_counts["btd_cache.cached_tiles_start"] > 0,
            "the prepared source must carry decoded tiles into pass 2"
        );
        assert!(
            prepared_report.operation_counts["btd_cache.misses"]
                < reopened_report.operation_counts["btd_cache.misses"]
        );
    }

    #[test]
    fn prepared_structured_cells_match_reopened_legacy_yaml_records() {
        let (_temp, options) = alpha_fallthrough_options();
        let mut legacy_records = Vec::new();
        let legacy_report = convert_btd_with_record_sink(options.clone(), &mut |record| {
            legacy_records.push(record);
            Ok(())
        })
        .expect("legacy YAML conversion");

        let prepared = prepare_without_manifest(&options);
        let mut structured_headers = Vec::new();
        let mut structured_cells = Vec::new();
        let structured_report = convert_prepared_btd_with_structured_cell_sink(
            prepared.source,
            options,
            &mut |record| {
                structured_headers.push(record);
                Ok(())
            },
            &mut |record| {
                structured_cells.push(record);
                Ok(())
            },
        )
        .expect("prepared structured conversion");

        let legacy_headers = legacy_records
            .iter()
            .filter(|record| record.signature != "CELL")
            .cloned()
            .collect::<Vec<_>>();
        assert_yaml_records_equal(&legacy_headers, &structured_headers);
        let legacy_cells = legacy_records
            .iter()
            .filter(|record| record.signature == "CELL")
            .collect::<Vec<_>>();
        assert_eq!(legacy_cells.len(), structured_cells.len());
        for (legacy, structured) in legacy_cells.into_iter().zip(&structured_cells) {
            assert_eq!(legacy.signature, structured.signature);
            assert_eq!(legacy.relative_path, structured.relative_path);
            let parsed: serde_json::Value =
                serde_saphyr::from_str(&legacy.yaml).expect("parse legacy CELL YAML");
            assert_eq!(parsed, structured.value);
        }
        assert_eq!(
            report_without_measurements(&legacy_report),
            report_without_measurements(&structured_report)
        );
        assert_eq!(
            structured_report.operation_counts["write_cells.cell_yaml_documents"],
            0
        );
        assert_eq!(
            structured_report.operation_counts["write_cells.structured_cell_payloads"],
            1
        );
    }

    fn compare_texture_usage_collectors(options: ConvertOptions) -> Vec<RequiredTextureUsage> {
        let profile =
            collect_required_texture_usages_lightweight_profiled_for_options(options.clone())
                .unwrap();
        let cells_x = u64::try_from(options.source_max_x - options.source_min_x + 1).unwrap();
        let cells_y = u64::try_from(options.source_max_y - options.source_min_y + 1).unwrap();
        let global_vertices =
            (cells_x * LAND_CELL_INTERVALS as u64 + 1) * (cells_y * LAND_CELL_INTERVALS as u64 + 1);
        assert_eq!(
            profile
                .operation_counts
                .get("global_blend.expected_global_vertices"),
            Some(&global_vertices)
        );
        assert_eq!(
            profile
                .operation_counts
                .get("global_blend.expected_source_sample_selections"),
            Some(&(global_vertices * 16))
        );
        assert_eq!(
            profile
                .operation_counts
                .get("global_blend.expected_quadrants"),
            Some(&(cells_x * cells_y * 4))
        );
        assert_eq!(
            profile
                .timings
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            [
                "load_source_texture_alpha_masks_exclusive",
                "sample_quantize_and_summarize_vertices_exclusive",
                "collect_quadrant_bases_exclusive",
                "edge_retention_and_texture_selection_exclusive",
                "resolve_texture_usages_exclusive",
            ]
        );
        let lightweight = profile.usages;
        let blended = collect_required_texture_usages_for_options(options).unwrap();
        assert_eq!(lightweight, blended);
        lightweight
    }

    #[test]
    fn lightweight_texture_scan_matches_blended_required_usages() {
        let form_ids = [0x000100, 0x000200, 0x000300, 0x000400];
        let mut quadrant_ltex_maps = [[0u8; 8]; 4];
        for (quadrant, texture_map) in quadrant_ltex_maps.iter_mut().enumerate() {
            texture_map[7] = (form_ids.len() - quadrant) as u8;
        }
        let btd_bytes = one_cell_btd(0, &form_ids, quadrant_ltex_maps);
        let btd_path = temp_path("mapped_textures", "btd");
        fs::write(&btd_path, btd_bytes).unwrap();
        let mapped = compare_texture_usage_collectors(texture_usage_options(&btd_path, 0, 0));
        let _ = fs::remove_file(&btd_path);
        assert_eq!(
            mapped
                .iter()
                .map(|usage| usage.ltex_form_key.as_str())
                .collect::<Vec<_>>(),
            [
                "000100:SeventySix.esm",
                "000200:SeventySix.esm",
                "000300:SeventySix.esm",
                "000400:SeventySix.esm",
            ]
        );
    }

    fn base_and_additional_texture_maps() -> [[u8; 8]; 4] {
        let mut maps = [[0u8; 8]; 4];
        for map in &mut maps {
            map[4] = 1;
            map[7] = 2;
        }
        maps
    }

    #[test]
    fn lightweight_texture_scan_ignores_unused_additional_layer() {
        let btd_path = temp_path("unused_additional", "btd");
        fs::write(
            &btd_path,
            one_cell_btd(0, &[0x000100, 0x000200], base_and_additional_texture_maps()),
        )
        .unwrap();

        let usages = compare_texture_usage_collectors(texture_usage_options(&btd_path, 0, 0));
        let _ = fs::remove_file(&btd_path);

        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].ltex_form_key, "000100:SeventySix.esm");
    }

    #[test]
    fn lightweight_texture_scan_keeps_nonzero_additional_layer() {
        let btd_path = temp_path("used_additional", "btd");
        fs::write(
            &btd_path,
            one_cell_btd(
                0x7000,
                &[0x000100, 0x000200],
                base_and_additional_texture_maps(),
            ),
        )
        .unwrap();

        let usages = compare_texture_usage_collectors(texture_usage_options(&btd_path, 0, 0));
        let _ = fs::remove_file(&btd_path);

        assert!(
            usages
                .iter()
                .any(|usage| usage.ltex_form_key == "000200:SeventySix.esm")
        );
    }

    #[test]
    fn lightweight_texture_scan_preserves_ground_cover_association() {
        let mut gcvr_maps = [[0xFFu8; 8]; 4];
        for map in &mut gcvr_maps {
            map[4] = 0;
        }
        let btd_path = temp_path("ground_cover", "btd");
        fs::write(
            &btd_path,
            btd_row_fixture(
                0x7000,
                &[0x000100, 0x000200],
                &[base_and_additional_texture_maps()],
                &[0x000900],
                &[gcvr_maps],
            ),
        )
        .unwrap();

        let usages = compare_texture_usage_collectors(texture_usage_options(&btd_path, 0, 0));
        let _ = fs::remove_file(&btd_path);

        assert!(usages.iter().any(|usage| {
            usage.ltex_form_key == "000200:SeventySix.esm"
                && usage.ground_cover_form_key.as_deref() == Some("000900:SeventySix.esm")
        }));
    }

    #[test]
    fn lightweight_texture_scan_samples_neighbor_for_partial_range() {
        let mut first_cell = [[0u8; 8]; 4];
        let mut second_cell = [[0u8; 8]; 4];
        for map in &mut first_cell {
            map[7] = 2;
        }
        for map in &mut second_cell {
            map[7] = 1;
        }
        let btd_path = temp_path("partial_neighbor", "btd");
        fs::write(
            &btd_path,
            btd_row_fixture(
                0,
                &[0x000100, 0x000200],
                &[first_cell, second_cell],
                &[],
                &[[[0; 8]; 4], [[0; 8]; 4]],
            ),
        )
        .unwrap();

        let usages = compare_texture_usage_collectors(texture_usage_options(&btd_path, 0, 0));
        let _ = fs::remove_file(&btd_path);

        assert_eq!(
            usages
                .iter()
                .map(|usage| usage.ltex_form_key.as_str())
                .collect::<Vec<_>>(),
            ["000100:SeventySix.esm", "000200:SeventySix.esm"]
        );
    }

    #[test]
    fn lightweight_texture_scan_reports_malformed_alpha_at_sampling() {
        let btd_path = temp_path("malformed_alpha", "btd");
        let mut bytes = one_cell_btd(
            0x7000,
            &[0x000100, 0x000200],
            base_and_additional_texture_maps(),
        );
        fs::write(&btd_path, &bytes).unwrap();
        let zlib_data_offset = BtdFile::open_header(btd_path.to_str().unwrap())
            .unwrap()
            .zlib_data_offset;
        bytes[zlib_data_offset] ^= 0xFF;
        fs::write(&btd_path, bytes).unwrap();
        let options = texture_usage_options(&btd_path, 0, 0);

        let lightweight_error =
            collect_required_texture_usages_lightweight_for_options(options.clone())
                .unwrap_err()
                .to_string();
        let blended_error = collect_required_texture_usages_for_options(options)
            .unwrap_err()
            .to_string();
        let _ = fs::remove_file(&btd_path);

        assert_eq!(lightweight_error, blended_error);
        assert!(lightweight_error.contains("BTD"));
    }

    const HEIGHT_LAND_BLOCK_LEN_TEST: usize = 49152;

    fn put_u32_le(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put_i32_le(bytes: &mut [u8], offset: usize, value: i32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put_f32_le(bytes: &mut [u8], offset: usize, value: f32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn temp_path(label: &str, ext: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("terrain_btd4_{label}_{unique}.{ext}"))
    }

    #[test]
    fn sample_raw_u16_mirrors_dequantized_f32_sample() {
        // The raw u16 path must reproduce the exact source index the approved f32
        // `sample` reads (same +HALF_CELL_SAMPLES shift, clamp, cell/local math)
        // so HGTS aligns byte-for-byte with the LAND's VHGT source.
        let height_sample = 4096u16;
        let btd_bytes = one_cell_btd_constant_height(height_sample);
        let btd_path = temp_path("align", "btd");
        fs::write(&btd_path, &btd_bytes).unwrap();
        let mut btd = BtdFile::open(btd_path.to_str().unwrap()).unwrap();

        let options = ConvertOptions {
            btd_path: btd_path.display().to_string(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esp".to_string(),
            worldspace_editor_id: "B21TestWorld".to_string(),
            source_min_x: 0,
            source_min_y: 0,
            source_max_x: 0,
            source_max_y: 0,
            first_form_id: 0x000800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: false,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: false,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };
        let mut cache =
            SourceCellCache::new(btd.header(), &options, 1, 1, SourceFrame::Fo76Identity).unwrap();

        // The exact decoded sample at a coordinate depends on the BTD line-walk,
        // which the test deliberately does not re-derive; instead it asserts the
        // raw and f32 paths share index arithmetic by proving the f32 value is
        // the dequantization of the raw u16 at every probed coordinate, and that
        // at least one probe carries the non-zero block value (real data flows).
        let mut saw_nonzero = false;
        for (sx, sy) in [(0usize, 0usize), (16, 48), (64, 64), (127, 0), (0, 127)] {
            let raw = cache.sample_raw_u16(&mut btd, sx, sy).unwrap();
            let f32_value = cache.sample(&mut btd, sx, sy).unwrap();
            saw_nonzero |= raw == height_sample;
            let expected = cache.height_min + raw as f32 * cache.height_scale;
            assert_eq!(
                f32_value.to_bits(),
                expected.to_bits(),
                "raw path must dequantize to the f32 path at ({sx},{sy})"
            );
        }
        assert!(
            saw_nonzero,
            "fixture should surface the non-zero block value at some probe"
        );
        let _ = fs::remove_file(&btd_path);
    }

    #[test]
    fn emits_btd4_sidecar_with_round_tripping_hgts() {
        let height_sample = 5000u16;
        let btd_bytes = one_cell_btd_constant_height(height_sample);
        let btd_path = temp_path("emit", "btd");
        fs::write(&btd_path, &btd_bytes).unwrap();

        let authoring_dir = temp_path("emit_out", "dir");
        let btd4_path = temp_path("emit_side", "btd4");
        let options = ConvertOptions {
            btd_path: btd_path.display().to_string(),
            output_authoring_dir: authoring_dir.display().to_string(),
            plugin_name: "B21_Test.esp".to_string(),
            worldspace_editor_id: "B21TestWorld".to_string(),
            source_min_x: 0,
            source_min_y: 0,
            source_max_x: 0,
            source_max_y: 0,
            first_form_id: 0x000800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: false,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: false,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: btd4_path.display().to_string(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        };

        let output = convert_btd(options, TerrainRecordOutput::ReportOnly).expect("convert");
        assert_eq!(
            output.report.btd4_output_path.as_deref(),
            Some(btd4_path.display().to_string().as_str())
        );
        assert!(btd4_path.is_file(), "sidecar file must exist");

        let reader = crate::btd4::Btd4Reader::open(&btd4_path).expect("open sidecar");
        let header = reader.header();
        assert_eq!(header.version, crate::btd4::BTD4_VERSION);
        assert_eq!(header.density, 128);
        assert_eq!(header.cell_min_x, 0);
        assert_eq!(header.cell_max_x, 0);
        assert_eq!(header.plugin_names, vec!["B21_Test.esp".to_string()]);
        // height_min/scale must match the BTD header so the consumer dequantizes
        // HGTS the same way the LAND heights were derived.
        assert_eq!(header.height_min.to_bits(), 0.0f32.to_bits());
        assert_eq!(
            header.height_scale.to_bits(),
            (1024.0f32 / u16::MAX as f32).to_bits()
        );

        let cell = reader.cell(0, 0).expect("cell (0,0)");
        let hgts = cell.heights.expect("HGTS present");
        assert_eq!(hgts.len(), 129 * 129);
        // Some block value must survive into HGTS (not a degenerate all-zero
        // grid), proving real raw samples were gathered and round-tripped.
        assert!(
            hgts.iter().any(|h| *h == height_sample),
            "HGTS should carry the non-zero block value somewhere"
        );

        // HGTS must equal the raw source sampler over the same global coords, and
        // its {0,4,...,128} subset must match the f32 LAND's VHGT source samples
        // (both go through the identical +HALF_CELL_SAMPLES / cell-local math).
        let mut btd2 = BtdFile::open(btd_path.to_str().unwrap()).unwrap();
        let probe_options = ConvertOptions {
            source_max_x: 0,
            source_max_y: 0,
            ..clone_options_for_probe(&btd_path)
        };
        let mut probe_cache = SourceCellCache::new(
            btd2.header(),
            &probe_options,
            1,
            1,
            SourceFrame::Fo76Identity,
        )
        .unwrap();
        for vy in 0..=128usize {
            for vx in 0..=128usize {
                let expected = probe_cache.sample_raw_u16(&mut btd2, vx, vy).unwrap();
                assert_eq!(
                    hgts[vy * 129 + vx],
                    expected,
                    "HGTS sample ({vx},{vy}) must equal sample_raw_u16"
                );
            }
        }
        for vy32 in 0..=32usize {
            for vx32 in 0..=32usize {
                // LAND vertex (vx32,vy32) samples source (vx32*4, vy32*4); that is
                // HGTS index (vx32*4, vy32*4).
                let land_source = probe_cache
                    .sample_raw_u16(&mut btd2, vx32 * 4, vy32 * 4)
                    .unwrap();
                assert_eq!(
                    hgts[(vy32 * 4) * 129 + (vx32 * 4)],
                    land_source,
                    "LAND vert ({vx32},{vy32}) must be the matching HGTS sample"
                );
            }
        }

        let _ = fs::remove_file(&btd_path);
        let _ = fs::remove_file(&btd4_path);
        let _ = fs::remove_dir_all(&authoring_dir);
    }

    fn clone_options_for_probe(btd_path: &Path) -> ConvertOptions {
        ConvertOptions {
            btd_path: btd_path.display().to_string(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_Test.esp".to_string(),
            worldspace_editor_id: "B21TestWorld".to_string(),
            source_min_x: 0,
            source_min_y: 0,
            source_max_x: 0,
            source_max_y: 0,
            first_form_id: 0x000800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "sample4".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: false,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: false,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        }
    }

    fn lanczos_grid_probe(
        cells_x: usize,
        threads: usize,
    ) -> (std::time::Duration, std::time::Duration) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("height.btd");
        fs::write(&path, one_cell_btd_constant_height(12345)).unwrap();
        let mut btd = BtdFile::open(path.to_str().unwrap()).unwrap();
        let mut options = clone_options_for_probe(&path);
        options.source_min_x = -2;
        options.source_max_x = cells_x as i32 - 3;
        options.source_max_y = 1;
        let cache = || {
            let mut cache = SourceCellCache::new(
                btd.header(),
                &options,
                cells_x,
                2,
                SourceFrame::Fo76Identity,
            )
            .unwrap();
            for cy in 0..cache.source_height / CELL_SOURCE_SAMPLES {
                for cx in 0..cache.source_width / CELL_SOURCE_SAMPLES {
                    let samples = (0..CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES)
                        .map(|index| {
                            let x = cx * CELL_SOURCE_SAMPLES + index % CELL_SOURCE_SAMPLES;
                            let y = cy * CELL_SOURCE_SAMPLES + index / CELL_SOURCE_SAMPLES;
                            ((x * 719 + y * 157 + x * y) % 65536) as f32 * 0.375 - 9000.0
                        })
                        .collect();
                    cache
                        .cells
                        .insert((options.source_min_x + cx as i32, cy as i32), samples);
                }
            }
            cache
        };
        let mut reference_cache = cache();
        let mut buffered_cache = cache();
        let width = cells_x * LAND_CELL_INTERVALS + 1;
        let height = 2 * LAND_CELL_INTERVALS + 1;
        let started = Instant::now();
        let mut reference = Vec::new();
        for y in 0..height {
            for x in 0..width {
                reference.push(
                    lanczos_target_height(&mut btd, &mut reference_cache, x, y)
                        .unwrap()
                        .to_bits(),
                );
            }
        }
        let reference_time = started.elapsed();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let started = Instant::now();
        let grid = pool
            .install(|| {
                build_target_height_grid(
                    &mut btd,
                    &options,
                    cells_x,
                    2,
                    ResampleMode::Lanczos,
                    &mut buffered_cache,
                )
            })
            .unwrap();
        let buffered_time = started.elapsed();
        assert_eq!(
            reference,
            grid.values.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
        (reference_time, buffered_time)
    }

    #[test]
    fn buffered_lanczos_preserves_sample_bits_at_cell_and_world_edges() {
        for threads in [1, 4] {
            lanczos_grid_probe(2, threads);
            lanczos_grid_probe(3, threads);
        }
    }

    #[test]
    #[ignore = "manual full-overwrite resampling benchmark"]
    fn benchmark_buffered_lanczos() {
        let (reference, buffered) = lanczos_grid_probe(64, 4);
        eprintln!("lanczos reference={reference:?} buffered={buffered:?} equal_bits=true");
    }

    fn extract_raw_hex_field(yaml: &str, field_name: &str) -> Option<String> {
        let marker = format!("{field_name}:\n        raw_hex: \"");
        let start = yaml.find(&marker)? + marker.len();
        let end = yaml[start..].find('"')? + start;
        Some(yaml[start..end].to_string())
    }

    /// The golden VHGT hex predates the Starfield frame: the FO76 identity path
    /// must stay bit-identical, so any change means shared height/addressing code
    /// was altered instead of branched on `SourceFrame`.
    #[test]
    fn fo76_identity_frame_is_bit_identical() {
        let height_sample = 12345u16;
        let btd_bytes = one_cell_btd_constant_height(height_sample);
        let btd_path = temp_path("golden_identity", "btd");
        fs::write(&btd_path, &btd_bytes).unwrap();

        let options = clone_options_for_probe(&btd_path);
        let output = convert_btd(options, TerrainRecordOutput::CollectOnly).expect("convert");
        let _ = fs::remove_file(&btd_path);

        let cell_record = output
            .authoring
            .records
            .iter()
            .find(|record| record.signature == "CELL")
            .expect("CELL record present");
        let vhgt_hex = extract_raw_hex_field(&cell_record.yaml, "VertexHeightMap")
            .expect("VertexHeightMap raw_hex present");

        assert_eq!(
            vhgt_hex,
            "0000000000180000E8180000E8180000E81800000000000000000000000000000000000000180000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000E8180000E8180000E8180000E81800000000000000000000000000000000000000180000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000E8180000E8180000E8180000E81800000000000000000000000000000000000000180000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000E8180000E8180000E8180000E81800000000000000000000000000000000000000180000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
        );
    }

    fn starfield_akila_btd_path() -> PathBuf {
        let root = std::env::var_os("STARFIELD_EXTRACTED_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/starfield")
            });
        root.join("terrain/akilacity.btd")
    }

    fn starfield_akila_options() -> ConvertOptions {
        ConvertOptions {
            btd_path: starfield_akila_btd_path().display().to_string(),
            output_authoring_dir: String::new(),
            plugin_name: "B21_AkilaTest.esp".to_string(),
            worldspace_editor_id: "B21_AkilaTest".to_string(),
            source_min_x: 0,
            source_min_y: 0,
            source_max_x: -1,
            source_max_y: -1,
            first_form_id: 0x000800,
            world_form_id: 0,
            first_cell_form_id: 0,
            resample_mode: "lanczos".to_string(),
            debug_output_dir: String::new(),
            texture_manifest_path: String::new(),
            water_manifest_path: String::new(),
            emit_textures: false,
            export_heightmap: false,
            debug_flat_land: false,
            preserve_source_ids: false,
            reserved_object_ids: Vec::new(),
            source_worldspace_authoring_dir: String::new(),
            source_worldspace_terrain_ids_json: String::new(),
            heightmap_output_path: String::new(),
            btd4_output_path: String::new(),
            conversion_workers: None,
            land_skip_ground_cover_variants: false,
            reuse_existing_textures: false,
        }
    }

    #[test]
    fn starfield_lightweight_texture_scan_skips_absent_ground_cover() {
        let path = starfield_akila_btd_path();
        if !path.exists() {
            eprintln!("skip: starfield extracted data not present");
            return;
        }

        let usages =
            collect_required_texture_usages_lightweight_for_options(starfield_akila_options())
                .expect("Starfield texture scan accepts the emitted FO4 cell window");

        assert!(!usages.is_empty());
        assert!(
            usages
                .iter()
                .any(|usage| usage.ltex_form_key == "01085C:Starfield.esm")
        );
        assert!(
            usages
                .iter()
                .all(|usage| usage.ground_cover_form_key.is_none())
        );
    }

    /// Runs the real Akila conversion once per test binary (the 3 real-data
    /// tests below all need it) and caches the result. `None` means the
    /// extracted fixture is absent -- every caller must skip, not fail.
    fn starfield_akila_conversion() -> Option<&'static ConvertOutput> {
        static RESULT: std::sync::OnceLock<Option<ConvertOutput>> = std::sync::OnceLock::new();
        RESULT
            .get_or_init(|| {
                let path = starfield_akila_btd_path();
                if !path.exists() {
                    return None;
                }
                let options = starfield_akila_options();
                Some(convert_btd(options, TerrainRecordOutput::CollectOnly).expect("akila convert"))
            })
            .as_ref()
    }

    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex byte"))
            .collect()
    }

    fn decode_cell_land_heights(record: &AuthoringRecordPayload) -> Vec<f32> {
        let hex = extract_raw_hex_field(&record.yaml, "VertexHeightMap")
            .expect("VertexHeightMap raw_hex present");
        let encoded = EncodedVhgt {
            offset: 0.0,
            raw: hex_to_bytes(&hex),
        };
        decode_vhgt_heights(&encoded).expect("VHGT decodes")
    }

    /// Real akilacity.btd, skip-if-missing: the Starfield frame emits the
    /// FO4 cell window that `sf_frame::fo4_cell_range` predicts for Akila's
    /// SF cell extent (-4..=4 both axes) -- 16x16 = 256 cells, (-7,-7)..(8,8).
    #[test]
    fn starfield_frame_emits_expected_fo4_cell_window() {
        let Some(output) = starfield_akila_conversion() else {
            eprintln!("skip: starfield extracted data not present");
            return;
        };
        assert_eq!(output.report.cells_written, 256);

        let mut min_x = i32::MAX;
        let mut max_x = i32::MIN;
        let mut min_y = i32::MAX;
        let mut max_y = i32::MIN;
        let mut cell_dirs = 0u32;
        for record in &output.authoring.records {
            if record.signature != "CELL" {
                continue;
            }
            cell_dirs += 1;
            let cell_dir = Path::new(&record.relative_path)
                .parent()
                .expect("CELL record has a parent dir");
            let name = cell_dir
                .file_name()
                .and_then(|n| n.to_str())
                .expect("cell dir has a name");
            let (x_str, y_str) = name.split_once(", ").expect("cell dir name is 'x, y'");
            let x: i32 = x_str.parse().expect("cell x parses");
            let y: i32 = y_str.parse().expect("cell y parses");
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        assert_eq!(cell_dirs, 256);
        assert_eq!((min_x, min_y), (-7, -7));
        assert_eq!((max_x, max_y), (8, 8));
        // 256 cells * 4 quadrants each; the split counter can never exceed
        // the number of quadrants that had a base to vote on at all.
        assert!(output.report.quadrant_base_split <= 256 * 4);
        eprintln!(
            "Akila quadrant_base_split = {} (of up to {} quadrants)",
            output.report.quadrant_base_split,
            256 * 4
        );
    }

    /// Real akilacity.btd, skip-if-missing. Expected values come from the BTD's
    /// own per-cell f32 min/max table (`cell_height_minmax` at HEADER_LEN +
    /// ltex*4), read independently of the resample code: max 49.187m at SF cell
    /// (1,0), min -2.998m at SF cell (0,4).
    ///
    /// Checks proximity, not containment, since a flat plane (e.g.
    /// `debug_flat_land`) also lies inside [min, max]. The 0.995 floor allows
    /// slight lanczos attenuation (a real run measured max 3440, span 3648 FO4
    /// units, within ~0.15%); the 3-VHGT-step ceiling covers quantisation only,
    /// because `lanczos_target_height` clamps to its footprint's min/max.
    #[test]
    fn starfield_heights_land_in_fo4_units() {
        let Some(output) = starfield_akila_conversion() else {
            eprintln!("skip: starfield extracted data not present");
            return;
        };
        let mut min_h = f32::INFINITY;
        let mut max_h = f32::NEG_INFINITY;
        for record in &output.authoring.records {
            if record.signature != "CELL" {
                continue;
            }
            for height in decode_cell_land_heights(record) {
                min_h = min_h.min(height);
                max_h = max_h.max(height);
            }
        }
        assert!(min_h.is_finite() && max_h.is_finite(), "no heights decoded");
        let span_h = max_h - min_h;

        let expected_max_m = 49.187f32;
        let expected_min_m = -2.998f32;
        let expected_max_h = expected_max_m * crate::sf_frame::FO4_UNITS_PER_METER as f32;
        let expected_span_h =
            (expected_max_m - expected_min_m) * crate::sf_frame::FO4_UNITS_PER_METER as f32;
        let quantisation_headroom = 3.0 * VHGT_HEIGHT_STEP;
        let floor_ratio = 0.995;

        assert!(
            max_h >= expected_max_h * floor_ratio
                && max_h <= expected_max_h + quantisation_headroom,
            "max height {max_h} outside [{}, {}]",
            expected_max_h * floor_ratio,
            expected_max_h + quantisation_headroom
        );
        assert!(
            span_h >= expected_span_h * floor_ratio
                && span_h <= expected_span_h + quantisation_headroom,
            "height span {span_h} outside [{}, {}]",
            expected_span_h * floor_ratio,
            expected_span_h + quantisation_headroom
        );
    }

    /// Real akilacity.btd, skip-if-missing: VHGT's signed-i8 delta clamp
    /// should rarely trigger on real terrain -- bound overflows to under 1%
    /// of the emitted vertices as a sanity check on the Starfield resampler's
    /// output smoothness.
    #[test]
    fn vhgt_delta_clamp_stats_stay_bounded() {
        let Some(output) = starfield_akila_conversion() else {
            eprintln!("skip: starfield extracted data not present");
            return;
        };
        let total_vertices = u64::from(output.report.cells_written)
            * (LAND_CELL_VERTICES as u64)
            * (LAND_CELL_VERTICES as u64);
        let overflow_ratio =
            f64::from(output.report.vhgt_delta_clamp_overflows) / total_vertices as f64;
        assert!(
            overflow_ratio < 0.01,
            "vhgt_delta_clamp_overflows {} / {} vertices = {:.4}%, expected < 1%",
            output.report.vhgt_delta_clamp_overflows,
            total_vertices,
            overflow_ratio * 100.0
        );
    }

    /// `sf_frame::SF_BTD_ORIGIN_BIAS_METERS` is a const 0.0, so a shim of the same
    /// formula with an explicit bias checks that the real function matches it at
    /// bias 0 and that a 50m bias shifts the grid by ~3500 units.
    #[test]
    fn origin_bias_is_zero() {
        fn shim_fo4_units_to_btd_sample(units: f64, btd_cell_min: i32, bias_meters: f64) -> f64 {
            let origin_meters = btd_cell_min as f64 * crate::sf_frame::SF_CELL_METERS - bias_meters;
            (crate::sf_frame::fo4_units_to_meters(units) - origin_meters)
                / crate::sf_frame::SF_BTD_SAMPLE_METERS
        }

        let units = crate::sf_frame::fo4_land_vertex_units(3, 17);
        let btd_cell_min = -2;

        let real = crate::sf_frame::fo4_units_to_btd_sample(units, btd_cell_min);
        let shim_zero_bias = shim_fo4_units_to_btd_sample(units, btd_cell_min, 0.0);
        assert!(
            (real - shim_zero_bias).abs() < 1e-9,
            "real {real} vs shim-at-zero-bias {shim_zero_bias} -- SF_BTD_ORIGIN_BIAS_METERS may not be 0"
        );

        let shim_fifty_bias = shim_fo4_units_to_btd_sample(units, btd_cell_min, 50.0);
        let sample_shift = shim_fifty_bias - shim_zero_bias;
        let unit_shift = crate::sf_frame::btd_sample_to_fo4_units(sample_shift, 0)
            - crate::sf_frame::btd_sample_to_fo4_units(0.0, 0);
        assert!(
            (unit_shift - 3500.0).abs() < 1.0,
            "expected a ~3500 unit grid shift for a 50m origin bias, got {unit_shift} -- \
             SF_BTD_ORIGIN_BIAS_METERS is not load-bearing in the position formula"
        );
    }
}
