use crate::btd::{BtdFile, BtdHeader};
#[cfg(test)]
use crate::btd::{CellTextureSet, QuadrantTextureSet};
use crate::diagnostics::{CellDiagnostic, TerrainDiagnostics};
use crate::global_blend::{
    GlobalLandscapeBlend, SourceAlphaLookup, collect_required_source_ltex_object_ids,
};
use crate::height_resample::{LANCZOS2_REACH, clamp_offset_index, lanczos2_kernel};
use crate::land_encode::{encode_vhgt, generate_vnml};
use crate::texture_bridge::{ConvertedTerrainGrass, ConvertedTerrainTexture, TextureManifest};
#[cfg(test)]
use crate::texture_layers::{decode_alpha_layers, map_cell_layers};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::Instant;
use thiserror::Error;

const CELL_SOURCE_SAMPLES: usize = 128;
const LAND_CELL_VERTICES: usize = 33;
const LAND_CELL_INTERVALS: usize = 32;
const LAND_QUADRANT_VERTICES: usize = 17;
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
    /// Retained for compatibility; texture conversion workers are now owned by
    /// the caller's unified texture phase.
    #[serde(default)]
    pub conversion_workers: Option<usize>,
    /// When true, LAND BTXT/ATXT layers resolve the PLAIN base LTEX instead of the
    /// `{base}_GC_{gcvr}` ground-cover composite. The composite eats a 6th per-quad
    /// texture slot and its incomplete TXST makes FO4's landscape shader render the
    /// quad black (the data is valid — render.exe + CK confirm — the engine chokes
    /// on the max-density quad). Ground cover still flows to the `.btd4` GCVR channel
    /// for the native scatter (M4). Default false preserves legacy baked-GC behavior.
    #[serde(default)]
    pub land_skip_ground_cover_variants: bool,
    /// Retained for compatibility; terrain planning no longer encodes DDS files.
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
    plugin_yaml: String,
    records: Vec<AuthoringRecordPayload>,
}

impl<'a> AuthoringOutput<'a> {
    fn write_files(output_dir: PathBuf) -> Self {
        Self {
            output_dir,
            write_files: true,
            collect_records: true,
            record_sink: None,
            plugin_yaml: String::new(),
            records: Vec::new(),
        }
    }

    fn collect_only(output_dir: PathBuf) -> Self {
        Self {
            output_dir,
            write_files: false,
            collect_records: true,
            record_sink: None,
            plugin_yaml: String::new(),
            records: Vec::new(),
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
            plugin_yaml: String::new(),
            records: Vec::new(),
        }
    }

    fn report_only(output_dir: PathBuf) -> Self {
        Self {
            output_dir,
            write_files: false,
            collect_records: false,
            record_sink: None,
            plugin_yaml: String::new(),
            records: Vec::new(),
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
                record_sink(payload)?;
            }
        }
        Ok(())
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
    timings: Vec<TimingEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct RequiredTextureUsage {
    pub ltex_form_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ground_cover_form_key: Option<String>,
}

#[derive(Debug, Clone)]
struct EmittedTexture {
    converted: ConvertedTerrainTexture,
    txst_object_id: u32,
    ltex_object_id: u32,
    grass_object_ids: Vec<u32>,
}

struct LandTextureFields {
    fields: Vec<String>,
    layer_count: u32,
    ground_cover_layer_count: u32,
    no_ground_cover_layer_count: u32,
    btd4_layer_object_ids: Vec<u32>,
    btd4_grass_object_ids: Vec<u32>,
    // Dense per-quadrant texture-blend planes for the .btd4 ALPH channel
    // (Some only when `want_dense_alpha` and the cell has ≥1 emitted alpha layer).
    // ALPH_PLANE_COUNT planes of ALPH_PLANE_LEN u8; plane = quadrant*5 + slot.
    dense_alpha: Option<Vec<Vec<u8>>>,
}

#[derive(Default)]
struct TextureAlphaMasks {
    by_source_ltex_object_id: HashMap<u32, TextureAlphaMask>,
}

struct TextureAlphaMask {
    width: usize,
    height: usize,
    alpha: Vec<u8>,
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
}

impl SourceCellCache {
    fn new(
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
        })
    }

    fn retain_neighbor_rows(&mut self, cell_y: i32) {
        let min_y = cell_y.saturating_sub(1);
        let max_y = cell_y.saturating_add(1);
        self.cells
            .retain(|(_, cached_y), _| *cached_y >= min_y && *cached_y <= max_y);
        self.raw_cells
            .retain(|(_, cached_y), _| *cached_y >= min_y && *cached_y <= max_y);
    }

    fn sample(
        &mut self,
        btd: &mut BtdFile,
        source_x: usize,
        source_y: usize,
    ) -> Result<f32, AuthoringEmitError> {
        // Identity frame (BTD is half-cell offset): FO4 world == FO76 world, so
        // the global source sample index shifts +HALF_CELL_SAMPLES per axis. At
        // the outer BTD edge the clamp stretches the final half cell.
        let source_x = (source_x + crate::fo4_frame::HALF_CELL_SAMPLES).min(self.source_width - 1);
        let source_y = (source_y + crate::fo4_frame::HALF_CELL_SAMPLES).min(self.source_height - 1);
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

    /// RAW-u16 twin of `sample` (identical +HALF_CELL_SAMPLES shift, clamp, and
    /// cell/local arithmetic). Returns the undequantized source sample so the
    /// .btd4 HGTS aligns byte-for-byte with the f32 LAND heights' source.
    fn sample_raw_u16(
        &mut self,
        btd: &mut BtdFile,
        source_x: usize,
        source_y: usize,
    ) -> Result<u16, AuthoringEmitError> {
        let source_x = (source_x + crate::fo4_frame::HALF_CELL_SAMPLES).min(self.source_width - 1);
        let source_y = (source_y + crate::fo4_frame::HALF_CELL_SAMPLES).min(self.source_height - 1);
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
    mut options: ConvertOptions,
) -> Result<Vec<RequiredTextureUsage>, AuthoringEmitError> {
    let mut btd = BtdFile::open(&options.btd_path)?;
    resolve_full_extent_sentinel(&mut options, btd.header());
    validate_range(&options)?;
    validate_btd_bounds(&btd, &options)?;
    collect_required_texture_usages_lightweight(&mut btd, &options)
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
    let output = convert_btd_inner(options, TerrainRecordOutput::CollectOnly, Some(sink))?;
    Ok(output.report)
}

pub fn convert_btd(
    options: ConvertOptions,
    output: TerrainRecordOutput,
) -> Result<ConvertOutput, AuthoringEmitError> {
    convert_btd_inner(options, output, None)
}

fn convert_btd_inner(
    mut options: ConvertOptions,
    output: TerrainRecordOutput,
    record_sink: Option<&mut dyn FnMut(AuthoringRecordPayload) -> Result<(), AuthoringEmitError>>,
) -> Result<ConvertOutput, AuthoringEmitError> {
    let total_started = Instant::now();
    let mut timings = Vec::new();
    let setup_started = Instant::now();
    let mut btd = BtdFile::open(&options.btd_path)?;
    resolve_full_extent_sentinel(&mut options, btd.header());
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
        TerrainRecordOutput::CollectOnly => {
            if let Some(record_sink) = record_sink {
                AuthoringOutput::stream_records(output_dir.clone(), record_sink)
            } else {
                AuthoringOutput::collect_only(output_dir.clone())
            }
        }
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
    let id_plan = build_terrain_id_plan(&options, &preserved_ids, cells_x, cells_y)?;
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
        &id_plan.used_object_ids,
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
    let global_blend = GlobalLandscapeBlend::build(
        &mut btd,
        options.source_min_x,
        options.source_min_y,
        cells_x,
        cells_y,
        &source_alpha_masks,
    )?;
    push_timing(
        &mut timings,
        "metadata_and_texture_setup.build_global_landscape_blend",
        global_blend_started,
    );

    let required_ltex_started = Instant::now();
    let required_ltex_form_ids = global_blend
        .source_ltex_object_ids()
        .into_iter()
        .map(source_ltex_form_key)
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

    let mut index = 0u32;
    let mut height_error_sum = 0.0f64;
    let mut height_error_count = 0u64;
    let mut height_max_error = 0.0f32;
    let mut dropped_texture_layers = 0u32;
    let mut ground_cover_layers = 0u32;
    let mut no_ground_cover_layers = 0u32;
    let mut vhgt_delta_clamp_stats = VhgtDeltaClampStats::default();
    let mut cell_diagnostics = Vec::with_capacity(cell_count as usize);
    let mut source_cache = SourceCellCache::new(btd.header(), &options, cells_x, cells_y)?;
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

    // Optional dense `.btd4` sidecar (zero-cost when the path is empty).
    let mut btd4_writer = if options.btd4_output_path.is_empty() {
        None
    } else {
        Some(crate::btd4::Btd4Writer::new(crate::btd4::Btd4Header {
            version: 1,
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
            let mut texture_fields = build_land_texture_fields(
                cell_x,
                cell_y,
                &global_blend,
                &textures_by_source_usage,
                &options.plugin_name,
                &mut dropped_texture_layers,
                btd4_writer.is_some(),
                options.land_skip_ground_cover_variants,
            )?;
            let cell_layers = texture_fields.layer_count;
            ground_cover_layers =
                ground_cover_layers.saturating_add(texture_fields.ground_cover_layer_count);
            no_ground_cover_layers =
                no_ground_cover_layers.saturating_add(texture_fields.no_ground_cover_layer_count);
            let cell_eid = id_plan.cell_editor_id(&options, cell_x, cell_y);
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
                &texture_fields.fields,
                water_cells.get(&(cell_x, cell_y)),
            )?;
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
                    &texture_fields.btd4_grass_object_ids,
                    &mut source_cache,
                    texture_fields.dense_alpha.take(),
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
    push_timing(&mut timings, "write_cells", write_cells_started);

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
        let image = directxtex_native::read_dds_rgba_image(Path::new(&bundle.diffuse_path))
            .map_err(AuthoringEmitError::Message)?;
        let alpha = image
            .rgba
            .chunks_exact(4)
            .map(|pixel| pixel[3])
            .collect::<Vec<_>>();
        masks.by_source_ltex_object_id.insert(
            object_id,
            TextureAlphaMask {
                width: image.width as usize,
                height: image.height as usize,
                alpha,
            },
        );
    }
    Ok(masks)
}

fn push_timing(timings: &mut Vec<TimingEntry>, name: &str, started: Instant) {
    let elapsed = started.elapsed().as_secs_f64();
    timings.push(TimingEntry {
        name: name.to_owned(),
        elapsed_seconds: (elapsed * 1_000_000.0).round() / 1_000_000.0,
    });
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
    initial_used_object_ids: &HashSet<u32>,
) -> Result<Vec<EmittedTexture>, AuthoringEmitError> {
    let mut emitted = Vec::with_capacity(converted.len());
    let mut next_object_id = first_texture_object_id;
    let mut used_object_ids: BTreeSet<u32> = initial_used_object_ids
        .iter()
        .map(|id| id & 0x00FF_FFFF)
        .filter(|id| *id != 0)
        .collect();
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
    used_object_ids: &mut BTreeSet<u32>,
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
    used_object_ids: &mut BTreeSet<u32>,
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

fn collect_required_texture_usages_lightweight(
    btd: &mut BtdFile,
    options: &ConvertOptions,
) -> Result<Vec<RequiredTextureUsage>, AuthoringEmitError> {
    let cells_x = cell_span(options.source_min_x, options.source_max_x)?;
    let cells_y = cell_span(options.source_min_y, options.source_max_y)?;
    let source_alpha_masks = load_source_texture_alpha_masks(options)?;
    let required_source_ltex_object_ids = collect_required_source_ltex_object_ids(
        btd,
        options.source_min_x,
        options.source_min_y,
        cells_x,
        cells_y,
        &source_alpha_masks,
    )?;
    texture_usages_for_required_ids(btd, options, &required_source_ltex_object_ids)
}

fn texture_usages_for_required_ids(
    btd: &BtdFile,
    options: &ConvertOptions,
    required_source_ltex_object_ids: &BTreeSet<u32>,
) -> Result<Vec<RequiredTextureUsage>, AuthoringEmitError> {
    let mut usages = required_source_ltex_object_ids
        .iter()
        .copied()
        .map(|object_id| RequiredTextureUsage {
            ltex_form_key: source_ltex_form_key(object_id),
            ground_cover_form_key: None,
        })
        .collect::<BTreeSet<_>>();

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
        ltex_form_key: source_ltex_form_key(source_ltex_object_id),
        ground_cover_form_key: Some(source_ltex_form_key(ground_cover_object_id)),
    });
}

fn resolve_full_extent_sentinel(options: &mut ConvertOptions, header: &BtdHeader) {
    if options.source_max_x != -1 || options.source_max_y != -1 {
        return;
    }
    options.source_min_x = header.cell_min_x;
    options.source_min_y = header.cell_min_y;
    options.source_max_x = header.cell_max_x;
    options.source_max_y = header.cell_max_y;
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
    if options.source_min_x < header.cell_min_x
        || options.source_max_x > header.cell_max_x
        || options.source_min_y < header.cell_min_y
        || options.source_max_y > header.cell_max_y
    {
        return Err(AuthoringEmitError::Message(format!(
            "requested cell range ({}, {})..({}, {}) is outside BTD bounds ({}, {})..({}, {})",
            options.source_min_x,
            options.source_min_y,
            options.source_max_x,
            options.source_max_y,
            header.cell_min_x,
            header.cell_min_y,
            header.cell_max_x,
            header.cell_max_y
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

fn sample4_target_height(
    btd: &mut BtdFile,
    source_cache: &mut SourceCellCache,
    target_x: usize,
    target_y: usize,
) -> Result<f32, AuthoringEmitError> {
    source_cache.sample(btd, target_x.saturating_mul(4), target_y.saturating_mul(4))
}

fn weighted_target_height(
    btd: &mut BtdFile,
    source_cache: &mut SourceCellCache,
    target_x: usize,
    target_y: usize,
) -> Result<f32, AuthoringEmitError> {
    let center_x = target_x.saturating_mul(4) as f32;
    let center_y = target_y.saturating_mul(4) as f32;
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
    let center_x = target_x.saturating_mul(4) as f32;
    let center_y = target_y.saturating_mul(4) as f32;
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

fn lanczos_target_height(
    btd: &mut BtdFile,
    source_cache: &mut SourceCellCache,
    target_x: usize,
    target_y: usize,
) -> Result<f32, AuthoringEmitError> {
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

    // Clamp to the footprint range so negative-lobe ringing cannot overshoot
    // past what the VHGT delta encode can represent.
    Ok((weighted_sum as f32).clamp(min_value, max_value))
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
    let (max_x, max_y) = (btd.header().cell_max_x, btd.header().cell_max_y);
    let colors = crate::fo4_frame::assemble_cell_grid(
        |cx, cy| btd.cell_terrain_color_u16(cx.min(max_x), cy.min(max_y), 0),
        cell_x,
        cell_y,
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
                    // FO4 terrain (landscape) materials have NO emissive map.
                    // The FO76->FO4 terrain remix derives a `_g` glow from the
                    // FO76 lighting map's alpha, but binding it into the TXST Glow
                    // slot puts the landscape shader on a glow permutation it does
                    // not have -> the quad renders black. Vanilla terrain TXSTs are
                    // Diffuse + NormalGloss + SmoothSpec only, so we omit Glow.
                    // Vanilla landscape TXSTs also set the NoSpecularMap flag.
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
        let grass_payload = texture
            .grass_object_ids
            .iter()
            .map(|object_id| {
                format!(
                    "- Grass:\n    reference:\n      plugin: {}\n      object_id: \"{}\"\n",
                    options.plugin_name,
                    form_id_hex(*object_id)
                )
            })
            .collect::<String>();
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
    texture_fields: &[String],
    water_cell: Option<&WaterCell>,
) -> Result<(), AuthoringEmitError> {
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
    let land_data_flags = land_data_flags(!texture_fields.is_empty(), !vclr.is_empty());
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
        land_fields.push_str(field);
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

/// Per-cell grass placement, computed from the FO76 BTD 128x128 ground-cover
/// stencil instead of FO4's texture-% scatter. For every ground-cover-bearing
/// texture slot we walk the quadrant's samples and emit one grass instance per set
/// mask bit (stride-thinned), grouped by target GRAS object id. Grass type metadata
/// (NIF path, GRAS id) reuses the same `texture_for_layer` resolution the LTEX path
/// uses. Z is bilinear-interpolated from the cell's authored LAND heights so grass
/// sits on the terrain. World XY uses the FO4 cell origin directly (no +2048
/// placed-record offset — terrain is authored fresh in FO4 convention).
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
    want_dense_alpha: bool,
    _land_skip_ground_cover_variants: bool,
) -> Result<LandTextureFields, AuthoringEmitError> {
    if textures_by_source_usage.is_empty() {
        return Ok(LandTextureFields {
            fields: Vec::new(),
            layer_count: 0,
            ground_cover_layer_count: 0,
            no_ground_cover_layer_count: 0,
            btd4_layer_object_ids: Vec::new(),
            btd4_grass_object_ids: Vec::new(),
            dense_alpha: None,
        });
    }

    let mut fields = Vec::new();
    let mut layer_count = 0u32;
    let mut ground_cover_layer_count = 0u32;
    let mut no_ground_cover_layer_count = 0u32;
    let mut btd4_layer_object_ids = Vec::new();
    let mut btd4_grass_object_ids = Vec::new();
    let mut dense_alpha_planes: Vec<Vec<u8>> = if want_dense_alpha {
        vec![vec![0u8; crate::btd4::ALPH_PLANE_LEN]; crate::btd4::ALPH_PLANE_COUNT]
    } else {
        Vec::new()
    };
    let mut dense_alpha_any = false;

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
        fields.push(land_texture_layer_field(
            "BTXT",
            base_texture.ltex_object_id,
            plugin_name,
            quadrant,
            -1,
        ));
        layer_count = layer_count.saturating_add(1);
        push_btd4_layer_refs(
            base_texture,
            &mut btd4_layer_object_ids,
            &mut btd4_grass_object_ids,
        );

        let mut emitted_alpha_slots = Vec::new();
        for (source_slot, (source_ltex_object_id, vtxt)) in quadrant_blend
            .alpha_source_ltex_object_ids
            .iter()
            .copied()
            .zip(quadrant_blend.alpha_vtxt.iter())
            .enumerate()
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
            let new_slot = emitted_alpha_slots.len();
            count_texture_layer_ground_cover(
                false,
                &mut ground_cover_layer_count,
                &mut no_ground_cover_layer_count,
            );
            fields.push(land_texture_layer_field(
                "ATXT",
                texture.ltex_object_id,
                plugin_name,
                quadrant,
                new_slot as i16,
            ));
            fields.push(alpha_layer_data_field(vtxt));
            layer_count = layer_count.saturating_add(1);
            push_btd4_layer_refs(
                texture,
                &mut btd4_layer_object_ids,
                &mut btd4_grass_object_ids,
            );
            emitted_alpha_slots.push(source_slot);
        }

        if want_dense_alpha && !emitted_alpha_slots.is_empty() {
            let dense_slot_limit = crate::btd4::ALPH_PLANE_COUNT / 4;
            for (new_slot, &source_slot) in emitted_alpha_slots
                .iter()
                .take(dense_slot_limit)
                .enumerate()
            {
                let plane =
                    &mut dense_alpha_planes[quadrant as usize * dense_slot_limit + new_slot];
                fill_dense_alpha_from_land_vtxt(&quadrant_blend.alpha_vtxt[source_slot], plane);
                dense_alpha_any = true;
            }
        }
    }

    Ok(LandTextureFields {
        fields,
        layer_count,
        ground_cover_layer_count,
        no_ground_cover_layer_count,
        btd4_layer_object_ids,
        btd4_grass_object_ids,
        dense_alpha: if dense_alpha_any {
            Some(dense_alpha_planes)
        } else {
            None
        },
    })
}

fn push_btd4_layer_refs(
    texture: &EmittedTexture,
    layer_object_ids: &mut Vec<u32>,
    grass_object_ids: &mut Vec<u32>,
) {
    if !layer_object_ids.contains(&texture.ltex_object_id) {
        layer_object_ids.push(texture.ltex_object_id);
    }
    for grass_id in &texture.grass_object_ids {
        if !grass_object_ids.contains(grass_id) {
            grass_object_ids.push(*grass_id);
        }
    }
}

fn fill_dense_alpha_from_land_vtxt(vtxt: &[u8], plane: &mut [u8]) {
    const LAND_V: usize = LAND_QUADRANT_VERTICES;
    const DENSE_V: usize = crate::btd4::ALPH_PLANE_VERTS;
    let mut land = [0u8; LAND_V * LAND_V];
    for entry in vtxt.chunks_exact(8) {
        let position = u16::from_le_bytes([entry[0], entry[1]]) as usize;
        if position >= land.len() {
            continue;
        }
        let opacity = f32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]);
        land[position] = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    for j in 0..DENSE_V {
        for i in 0..DENSE_V {
            let row = ((j + 2) / 4).min(LAND_V - 1);
            let column = ((i + 2) / 4).min(LAND_V - 1);
            plane[j * DENSE_V + i] = land[row * LAND_V + column];
        }
    }
}

/// Per-cell channel gather for the `.btd4` dense sidecar.
///
/// - HGTS: 129x129 RAW u16 samples, read via `SourceCellCache::sample_raw_u16`
///   so they share the +HALF_CELL_SAMPLES shift / clamp / cell-local arithmetic
///   with the f32 LAND heights (the LAND's 33x33 verts are the vx,vy in
///   {0,4,...,128} subset of this grid). The +1 edge column/row reads into the
///   next source cell, which the extra-cell growth already covers.
/// - LAYR: every mapped layer (base + alpha + synthesized + dropped), deduped by
///   emitted LTEX object id, all `plugin_index = 0` (the producer plugin) and
///   `kind = 0` (LTEX). `recovered` counts the layers referenced here that the
///   LAND's per-quadrant stack dropped — the sidecar's extra coverage.
/// - GCVR: the assembled 128x128 ground-cover mask, plus the union of the
///   producer-plugin GRAS object ids referenced by this cell's mapped layers.
///   Emitted only when there is ground-cover data (mask non-empty or forms).
/// - ALPH: dense per-quadrant blend planes, passed in from
///   `build_land_texture_fields` (`dense_alpha`) so they share that function's
///   exact per-quadrant ATXT slot ordering — the engine reuses the vanilla LAND
///   material for the dense terrain, so the planes must match its percentArrays
///   layout. None when the cell has no alpha layers.
/// - CLRS: deferred (None) — the FO76 vertex-color chunk is not yet decoded.
struct Btd4CellGather {
    channels: crate::btd4::CellChannels,
    layers_recovered: u32,
}

#[allow(clippy::too_many_arguments)]
fn gather_btd4_cell_channels(
    btd: &mut BtdFile,
    _cell_x: i32,
    _cell_y: i32,
    target_cell_offset_x: usize,
    target_cell_offset_y: usize,
    layer_object_ids: &[u32],
    grass_object_ids: &[u32],
    source_cache: &mut SourceCellCache,
    dense_alpha: Option<Vec<Vec<u8>>>,
) -> Result<Btd4CellGather, AuthoringEmitError> {
    let mut heights = Vec::with_capacity(129 * 129);
    for vy in 0..=128usize {
        for vx in 0..=128usize {
            let gx = target_cell_offset_x * CELL_SOURCE_SAMPLES + vx;
            let gy = target_cell_offset_y * CELL_SOURCE_SAMPLES + vy;
            heights.push(source_cache.sample_raw_u16(btd, gx, gy)?);
        }
    }

    let layers: Vec<crate::btd4::LayerRef> = layer_object_ids
        .iter()
        .map(|object_id| crate::btd4::LayerRef {
            plugin_index: 0,
            object_id: *object_id,
            kind: 0,
        })
        .collect();
    // Writer caps LAYR rows and GCVR forms at 255; drop the long tail rather
    // than fail the whole emit on a pathological cell.
    let layers = if layers.len() > 255 {
        layers[..255].to_vec()
    } else {
        layers
    };

    let gcvr_has_data = !grass_object_ids.is_empty();
    let gcvr = if gcvr_has_data {
        let mut forms: Vec<u32> = grass_object_ids.to_vec();
        forms.sort_unstable();
        forms.dedup();
        forms.truncate(255);
        Some(crate::btd4::GcvrChunk {
            mask: vec![0; CELL_SOURCE_SAMPLES * CELL_SOURCE_SAMPLES],
            forms: forms
                .into_iter()
                .map(|object_id| crate::btd4::GcvrForm {
                    plugin_index: 0,
                    object_id,
                })
                .collect(),
        })
    } else {
        None
    };

    Ok(Btd4CellGather {
        channels: crate::btd4::CellChannels {
            heights: Some(heights),
            alphas: dense_alpha,
            layers: if layers.is_empty() {
                None
            } else {
                Some(layers)
            },
            gcvr,
            colors: None,
        },
        layers_recovered: 0,
    })
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

fn alpha_layer_data_field(bytes: &[u8]) -> String {
    format!(
        "    - AlphaLayerData:\n        raw_hex: \"{}\"\n",
        bytes_hex(bytes)
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

fn source_ltex_form_key(value: u32) -> String {
    format!("{:06X}:SeventySix.esm", value & 0x00FF_FFFF)
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
            assign_texture_form_ids(terrain_next_object_id, converted, false, &HashSet::new())
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
            assign_texture_form_ids(0x800, converted, true, &HashSet::new()).expect("texture IDs");

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
            &HashSet::new(),
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
            assign_texture_form_ids(0x800, converted, true, &reserved).expect("texture IDs");

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
            assign_texture_form_ids(0x800, converted, true, &HashSet::new()).expect("texture IDs");

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
        write_texture_records(&mut output, &options, &[texture], "APPALACHIA")
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
        write_texture_records(&mut output, &options, &[texture], "APPALACHIA")
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
    fn source_ltex_form_key_formats_fo76_object_id() {
        assert_eq!(source_ltex_form_key(0xFF00_ABCD), "00ABCD:SeventySix.esm");
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

    fn compare_texture_usage_collectors(options: ConvertOptions) -> Vec<RequiredTextureUsage> {
        let lightweight =
            collect_required_texture_usages_lightweight_for_options(options.clone()).unwrap();
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
        let mut cache = SourceCellCache::new(btd.header(), &options, 1, 1).unwrap();

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
        assert_eq!(header.version, 1);
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
        let mut probe_cache = SourceCellCache::new(btd2.header(), &probe_options, 1, 1).unwrap();
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
}
