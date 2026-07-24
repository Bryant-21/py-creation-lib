//! Worldspace/cell input model (Task-7 deliverable).
//!
//! The data structures here are complete; only `enumerate_worldspace` (real ESP
//! LAND/VTXT/ATXT extraction) is deferred — it currently `bail!`s and
//! callers must supply a pre-built `WorldspaceInput`.

#[derive(Clone, Debug)]
pub struct CellInput {
    pub x: i32,
    pub y: i32,
    /// 33*33 row-major height posts
    pub heights: Vec<f32>,
    /// per-post vertex colors (33*33)
    pub vertex_colors: Vec<[u8; 3]>,
    pub layers: Vec<LayerTexture>,
    pub hidden_quadrants: [bool; 4],
    pub water_height: f32,
}

#[derive(Clone, Debug)]
pub struct LayerTexture {
    pub diffuse: String,
    pub normal: String,
    pub quadrant: u8,
    pub alpha: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct RefInput {
    pub ref_id: String,
    pub ref_flags: u32,
    pub enable_parent: u32,
    pub cell: (i32, i32),
    pub pos: [f32; 3],
    pub rot: [f32; 3],
    pub scale: f32,
    pub color: f32,
    pub alpha_threshold: u8,
    pub is_billboard: bool,
    pub is_grass: bool,
    pub base_name: String,
    pub base_flags: u32,
    pub material_name: String,
    pub full_model: String,
    pub lod_models: [Option<String>; 4],
    pub part_transform: [[f32; 4]; 4],
    pub part_scale: f32,
    pub material_swap: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
struct GrassModelInput {
    base_name: String,
    model: String,
    max_slope_degrees: f32,
}

#[derive(Clone, Debug)]
struct GrassLayerInput {
    quadrant: u8,
    alpha: Vec<f32>,
    models: Vec<GrassModelInput>,
}

const GRASS_CELL_SIZE: f32 = 4096.0;
const GRASS_POST_GRID: usize = 33;
const GRASS_ALPHA_EDGE: usize = 17;

fn sample_grass_layer_alpha(alpha: &[f32], quadrant: u8, u: f32, v: f32) -> f32 {
    let (qu, qv) = match quadrant {
        0 => (u * 2.0, v * 2.0),
        1 => ((u - 0.5) * 2.0, v * 2.0),
        2 => (u * 2.0, (v - 0.5) * 2.0),
        3 => ((u - 0.5) * 2.0, (v - 0.5) * 2.0),
        _ => return 0.0,
    };
    if !(0.0..=1.0).contains(&qu) || !(0.0..=1.0).contains(&qv) {
        return 0.0;
    }
    if alpha.len() != GRASS_ALPHA_EDGE * GRASS_ALPHA_EDGE {
        return 1.0;
    }

    let fx = qu.clamp(0.0, 1.0) * (GRASS_ALPHA_EDGE - 1) as f32;
    let fy = qv.clamp(0.0, 1.0) * (GRASS_ALPHA_EDGE - 1) as f32;
    let x0 = fx.floor() as usize;
    let y0 = fy.floor() as usize;
    let x1 = (x0 + 1).min(GRASS_ALPHA_EDGE - 1);
    let y1 = (y0 + 1).min(GRASS_ALPHA_EDGE - 1);
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;
    let get = |x: usize, y: usize| alpha[y * GRASS_ALPHA_EDGE + x].clamp(0.0, 1.0);
    let top = get(x0, y0) * (1.0 - tx) + get(x1, y0) * tx;
    let bottom = get(x0, y1) * (1.0 - tx) + get(x1, y1) * tx;
    (top * (1.0 - ty) + bottom * ty).clamp(0.0, 1.0)
}

fn grass_height_and_slope(heights: &[f32], u: f32, v: f32) -> Option<(f32, f32)> {
    if heights.len() != GRASS_POST_GRID * GRASS_POST_GRID {
        return None;
    }
    let fx = u.clamp(0.0, 1.0) * (GRASS_POST_GRID - 1) as f32;
    let fy = v.clamp(0.0, 1.0) * (GRASS_POST_GRID - 1) as f32;
    let x0 = fx.floor() as usize;
    let y0 = fy.floor() as usize;
    let x1 = (x0 + 1).min(GRASS_POST_GRID - 1);
    let y1 = (y0 + 1).min(GRASS_POST_GRID - 1);
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;
    let h00 = heights[x0 + y0 * GRASS_POST_GRID];
    let h10 = heights[x1 + y0 * GRASS_POST_GRID];
    let h01 = heights[x0 + y1 * GRASS_POST_GRID];
    let h11 = heights[x1 + y1 * GRASS_POST_GRID];
    let south = h00 * (1.0 - tx) + h10 * tx;
    let north = h01 * (1.0 - tx) + h11 * tx;
    let height = south * (1.0 - ty) + north * ty;
    let post_spacing = GRASS_CELL_SIZE / (GRASS_POST_GRID - 1) as f32;
    let dz_dx = ((h10 - h00) * (1.0 - ty) + (h11 - h01) * ty) / post_spacing;
    let dz_dy = ((h01 - h00) * (1.0 - tx) + (h11 - h10) * tx) / post_spacing;
    let slope = (dz_dx * dz_dx + dz_dy * dz_dy).sqrt().atan().to_degrees();
    Some((height, slope))
}

fn mix_grass_seed(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn grass_random(seed: u64, stream: u64) -> f32 {
    let bits = mix_grass_seed(seed ^ stream) >> 40;
    bits as f32 / 0x00FF_FFFFu32 as f32
}

fn grass_seed(
    cell: (i32, i32),
    level_index: usize,
    layer_index: usize,
    gx: usize,
    gy: usize,
) -> u64 {
    [
        cell.0 as u32 as u64,
        cell.1 as u32 as u64,
        level_index as u64,
        layer_index as u64,
        gx as u64,
        gy as u64,
    ]
    .into_iter()
    .fold(0, |seed, value| mix_grass_seed(seed ^ value))
}

fn synthesize_grass_refs_for_cell(
    cell: (i32, i32),
    heights: &[f32],
    hidden_quadrants: [bool; 4],
    layers: &[GrassLayerInput],
    settings: &crate::settings::GrassSettings,
    out: &mut Vec<RefInput>,
) {
    if !settings.enabled || layers.is_empty() {
        return;
    }
    let min_alpha = settings.min_alpha.clamp(0.0, 1.0);
    for (level_index, &spacing) in settings.spacings.iter().enumerate() {
        if !spacing.is_finite() || spacing <= 0.0 {
            continue;
        }
        let edge_count = (GRASS_CELL_SIZE / spacing).ceil().max(1.0) as usize;
        let step = GRASS_CELL_SIZE / edge_count as f32;
        for (layer_index, layer) in layers.iter().enumerate() {
            if layer.models.is_empty()
                || hidden_quadrants
                    .get(layer.quadrant as usize)
                    .copied()
                    .unwrap_or(true)
            {
                continue;
            }
            for gy in 0..edge_count {
                for gx in 0..edge_count {
                    let seed = grass_seed(cell, level_index, layer_index, gx, gy);
                    let jitter_x = (grass_random(seed, 1) - 0.5) * step * 0.5;
                    let jitter_y = (grass_random(seed, 2) - 0.5) * step * 0.5;
                    let local_x = ((gx as f32 + 0.5) * step + jitter_x)
                        .clamp(0.0, GRASS_CELL_SIZE - f32::EPSILON);
                    let local_y = ((gy as f32 + 0.5) * step + jitter_y)
                        .clamp(0.0, GRASS_CELL_SIZE - f32::EPSILON);
                    let u = local_x / GRASS_CELL_SIZE;
                    let v = local_y / GRASS_CELL_SIZE;
                    let mut coverage = sample_grass_layer_alpha(&layer.alpha, layer.quadrant, u, v);
                    if layer.alpha.is_empty() {
                        let overlay_coverage: f32 = layers
                            .iter()
                            .filter(|other| {
                                other.quadrant == layer.quadrant && !other.alpha.is_empty()
                            })
                            .map(|other| {
                                sample_grass_layer_alpha(&other.alpha, other.quadrant, u, v)
                            })
                            .sum();
                        coverage *= 1.0 - overlay_coverage.clamp(0.0, 1.0);
                    }
                    if coverage < min_alpha || grass_random(seed, 3) > coverage {
                        continue;
                    }
                    let Some((height, slope)) = grass_height_and_slope(heights, u, v) else {
                        continue;
                    };
                    let model_index = (mix_grass_seed(seed ^ 4) as usize) % layer.models.len();
                    let grass = &layer.models[model_index];
                    if slope > grass.max_slope_degrees {
                        continue;
                    }
                    let mut lod_models = [None, None, None, None];
                    lod_models[level_index] = Some(grass.model.clone());
                    out.push(RefInput {
                        ref_id: format!(
                            "GRASS:{}:{}:{level_index}:{layer_index}:{gx}:{gy}",
                            cell.0, cell.1
                        ),
                        ref_flags: 0,
                        enable_parent: 0,
                        cell,
                        pos: [
                            cell.0 as f32 * GRASS_CELL_SIZE + local_x,
                            cell.1 as f32 * GRASS_CELL_SIZE + local_y,
                            height,
                        ],
                        rot: [0.0, 0.0, grass_random(seed, 5) * std::f32::consts::TAU],
                        scale: 0.85 + grass_random(seed, 6) * 0.3,
                        color: 1.0,
                        alpha_threshold: 128,
                        is_billboard: false,
                        is_grass: true,
                        base_name: grass.base_name.clone(),
                        base_flags: 0,
                        material_name: String::new(),
                        full_model: grass.model.clone(),
                        lod_models,
                        part_transform: identity_part_transform(),
                        part_scale: 1.0,
                        material_swap: Default::default(),
                    });
                }
            }
        }
    }
}

pub type StaticDesc = RefInput;

pub fn identity_part_transform() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

#[derive(Clone, Debug)]
pub struct WorldspaceInput {
    pub editor_id: String,
    pub sw_cell: (i32, i32),
    pub ne_cell: (i32, i32),
    pub water_height: f32,
    pub no_lod_water: bool,
    pub default_diffuse: String,
    pub default_normal: String,
    pub cells: Vec<CellInput>,
    pub refs: Vec<RefInput>,
}

impl WorldspaceInput {
    /// Bounding box of the cells that actually carry a LAND record, as
    /// `(min_x, min_y, max_x, max_y)`. This is xLODGen's `bbWorld` — accumulated
    /// over every cell present in the terrain data (`TerrainLOD.cs:195`) — and
    /// drives terrain quad emission (`terrain_quads_for`). Returns `None` when the
    /// worldspace has no land cells.
    pub fn land_cell_bounds(&self) -> Option<(i32, i32, i32, i32)> {
        let mut it = self.cells.iter();
        let first = it.next()?;
        let mut min_x = first.x;
        let mut min_y = first.y;
        let mut max_x = first.x;
        let mut max_y = first.y;
        for c in it {
            min_x = min_x.min(c.x);
            min_y = min_y.min(c.y);
            max_x = max_x.max(c.x);
            max_y = max_y.max(c.y);
        }
        Some((min_x, min_y, max_x, max_y))
    }

    /// Test/helper constructor — derives sw/ne bounds from cell list.
    pub fn from_cells(editor_id: impl Into<String>, cells: Vec<CellInput>) -> Self {
        let sw_x = cells.iter().map(|c| c.x).min().unwrap_or(0);
        let sw_y = cells.iter().map(|c| c.y).min().unwrap_or(0);
        let ne_x = cells.iter().map(|c| c.x).max().unwrap_or(0);
        let ne_y = cells.iter().map(|c| c.y).max().unwrap_or(0);
        WorldspaceInput {
            editor_id: editor_id.into(),
            sw_cell: (sw_x, sw_y),
            ne_cell: (ne_x, ne_y),
            water_height: 0.0,
            no_lod_water: false,
            default_diffuse: String::new(),
            default_normal: String::new(),
            cells,
            refs: Vec::new(),
        }
    }
}

/// Decode hidden-quadrant bit flags to `[SW, SE, NW, NE]` bool array.
/// Bit layout per R1 §7 (InQuadrant, TerrainLOD.cs:140-163):
///   bit 1 = SW (index 0), bit 2 = SE (index 1), bit 4 = NW (index 2), bit 8 = NE (index 3).
pub fn decode_hidden_quadrants(land_flags: i32) -> [bool; 4] {
    [
        (land_flags & 0b0001) != 0, // SW
        (land_flags & 0b0010) != 0, // SE
        (land_flags & 0b0100) != 0, // NW
        (land_flags & 0b1000) != 0, // NE
    ]
}

// ===========================================================================
// Real ESP enumeration (feature `real-esp`). Off by default — see Cargo.toml
// [features]: esp pulls a directxtex FFI that collides with directxtex_native
// in exe/test links, so the default build keeps a stub and only the e2e golden
// test (and the umbrella cdylib) link the real reader.
// ===========================================================================
mod object_lod_overlay;

#[cfg(feature = "real-esp")]
mod esp_enum {
    use super::object_lod_overlay::{ObjectLodOverlay, OverlayEntry};
    use super::{
        CellInput, GrassLayerInput, GrassModelInput, LayerTexture, RefInput, WorldspaceInput,
        decode_hidden_quadrants, synthesize_grass_refs_for_cell,
    };
    use esp_authoring_core::plugin_runtime::{
        ParsedGroup, ParsedItem, ParsedPlugin, ParsedRecord, parse_plugin_file,
    };

    /// A parsed plugin plus its parsed masters, in load order.
    ///
    /// `plugin` is the plugin that *contains* the requested WRLD (e.g. `DLCCoast.esm`
    /// for the `DLC03FarHarbor` worldspace). `masters` are the parsed master files in
    /// the same order they appear in `plugin.header.masters`, used to resolve LTEX /
    /// TXST records the worldspace references from a master.
    pub struct EspHandle {
        plugin: Option<ParsedPlugin>,
        masters: Vec<ParsedPlugin>,
        object_lod_overlay: Option<ObjectLodOverlay>,
    }

    impl EspHandle {
        pub fn new() -> Self {
            EspHandle {
                plugin: None,
                masters: Vec::new(),
                object_lod_overlay: None,
            }
        }

        /// Load the plugin at `plugin_path` and (best-effort) its masters from the
        /// same directory. Records are eagerly decompressed so subrecords are
        /// directly readable.
        pub fn load(plugin_path: &std::path::Path, game: &str) -> anyhow::Result<Self> {
            Self::load_with_overlay(plugin_path, game, None)
        }

        pub fn load_with_overlay(
            plugin_path: &std::path::Path,
            game: &str,
            overlay_path: Option<&std::path::Path>,
        ) -> anyhow::Result<Self> {
            let path_str = plugin_path.to_string_lossy().into_owned();
            let plugin = parse_plugin_file(&path_str, Some(game.to_string()), true)
                .map_err(py_err_to_anyhow)?;

            let dir = plugin_path.parent().map(std::path::Path::to_path_buf);
            let mut masters = Vec::new();
            if let Some(dir) = dir {
                for master_name in &plugin.header.masters {
                    let mpath = dir.join(master_name);
                    if mpath.is_file() {
                        // A master that fails to parse is skipped (with no LTEX
                        // resolution from it) rather than aborting the whole run.
                        if let Ok(m) = parse_plugin_file(
                            &mpath.to_string_lossy(),
                            Some(game.to_string()),
                            true,
                        ) {
                            masters.push(m);
                        }
                    }
                }
            }
            Ok(EspHandle {
                plugin: Some(plugin),
                masters,
                object_lod_overlay: overlay_path
                    .map(|path| ObjectLodOverlay::load(path, plugin_path))
                    .transpose()?,
            })
        }
    }

    impl Default for EspHandle {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Convert a pyo3 `PyErr` into an `anyhow::Error` without requiring a live
    /// interpreter on the success path. `parse_plugin_file` only constructs a
    /// `PyErr` on failure; rendering its message needs the GIL, so we attach.
    fn py_err_to_anyhow(e: pyo3::PyErr) -> anyhow::Error {
        let msg = pyo3::Python::attach(|py| e.value(py).to_string());
        anyhow::anyhow!("esp parse: {msg}")
    }

    // ---------------------------------------------------------------------------
    // LAND subrecord decode (FO4) — ported from xLODGen TerrainData.ReadWorldData
    // (TerrainData.cs:67-108) for heights, and the FO4 LAND schema
    // (esp generated/fo4.rs) for VCLR/BTXT/ATXT/VTXT.
    // ---------------------------------------------------------------------------

    const GRID: usize = 33; // posts per cell edge
    const FO4_CELL_SIZE: f32 = 4096.0;

    /// Decode a VHGT subrecord into 33*33 row-major world-unit post heights
    /// (`heights[col + row*33]`).
    ///
    /// Port of `TerrainData.ReadWorldData` (TerrainData.cs:69-86): base*8, then a
    /// running delta accumulation where each row resets to the column-0 running
    /// value before adding its own deltas (each delta is the signed byte * 8).
    pub fn decode_vhgt_heights(vhgt: &[u8]) -> anyhow::Result<Vec<f32>> {
        let map = esp_authoring_core::land::heightmap::parse_heightmap(vhgt)
            .map_err(|e| anyhow::anyhow!("VHGT decode: {e}"))?;
        let base = map.base * 8.0;
        let mut heights = vec![0.0f32; GRID * GRID];
        // num3 = running column-0 baseline; num4 = running accumulator (TerrainData.cs).
        let mut num3 = base;
        let mut num4 = base;
        for r in 0..GRID {
            for c in 0..GRID {
                let delta = (map.deltas[r][c] as i32 * 8) as f32;
                if c == 0 {
                    num4 = num3;
                    num3 += delta;
                }
                num4 += delta;
                heights[c + r * GRID] = num4;
            }
        }
        Ok(heights)
    }

    /// Decode a VCLR subrecord into 33*33 row-major `[R,G,B]` vertex colors.
    /// FO4 VCLR is a row_array of 3 bytes/post (esp generated/fo4.rs:82878-82917).
    /// Missing/short data yields white (255,255,255) — xLODGen treats absent VCLR
    /// as identity tint.
    pub fn decode_vclr(vclr: &[u8]) -> Vec<[u8; 3]> {
        let mut out = vec![[255u8, 255, 255]; GRID * GRID];
        if vclr.len() >= GRID * GRID * 3 {
            for i in 0..GRID * GRID {
                out[i] = [vclr[i * 3], vclr[i * 3 + 1], vclr[i * 3 + 2]];
            }
        }
        out
    }

    /// A BTXT/ATXT layer header (FO4 codec `struct:I,B,B,h`).
    struct LayerHeader {
        texture_form_id: u32,
        quadrant: u8,
        layer: i16,
    }

    fn decode_layer_header(data: &[u8]) -> Option<LayerHeader> {
        if data.len() < 8 {
            return None;
        }
        Some(LayerHeader {
            texture_form_id: u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            quadrant: data[4],
            layer: i16::from_le_bytes([data[6], data[7]]),
        })
    }

    /// FO4 per-quadrant alpha grid edge length (17x17 covers one cell quadrant).
    const QUADRANT_ALPHA_EDGE: usize = 17;

    /// Decode a VTXT subrecord (FO4 codec `array_struct:H,B,B,f`) into a 17x17
    /// row-major opacity grid for one quadrant. Each entry is
    /// `position(u16), unk(u8), unk(u8), opacity(f32)`; `position` is the post index
    /// within the 17x17 quadrant (0..289), row-major. Posts absent from the VTXT are
    /// 0 (transparent) — the standard FO4 sparse alpha encoding.
    pub fn decode_vtxt_alpha(vtxt: &[u8]) -> Vec<f32> {
        let mut alpha = vec![0.0f32; QUADRANT_ALPHA_EDGE * QUADRANT_ALPHA_EDGE];
        let stride = 8usize;
        let n = vtxt.len() / stride;
        for i in 0..n {
            let off = i * stride;
            let pos = u16::from_le_bytes([vtxt[off], vtxt[off + 1]]) as usize;
            let opacity =
                f32::from_le_bytes([vtxt[off + 4], vtxt[off + 5], vtxt[off + 6], vtxt[off + 7]]);
            if pos < alpha.len() {
                alpha[pos] = opacity.clamp(0.0, 1.0);
            }
        }
        alpha
    }

    // ---------------------------------------------------------------------------
    // Plugin traversal helpers (mirror esp cell_slice.rs WRLD->CELL->LAND walk).
    // ---------------------------------------------------------------------------

    const EXTERIOR_CELL_BLOCK: i32 = 4;
    const EXTERIOR_CELL_SUBBLOCK: i32 = 5;
    const CELL_CHILD_GROUP: i32 = 6;
    const PERSISTENT_GROUP: i32 = 8;
    const TEMPORARY_GROUP: i32 = 9;
    const VISIBLE_DISTANT_GROUP: i32 = 10;

    /// FO4 record flag bit 0x800 = "Initially Disabled" (a disabled placed ref).
    /// Refs flagged disabled are excluded from LOD (xLODGen ProcessReference skips them).
    const REFR_FLAG_INITIALLY_DISABLED: u32 = 0x0000_0800;
    const RECORD_FLAG_DELETED: u32 = 0x0000_0020;
    const VISIBLE_WHEN_DISTANT_FLAG: u32 = 0x0000_8000;
    const MULTIREF_LOD_KEYWORD_OBJECT_ID: u32 = 0x0019_5411;

    pub(super) fn ref_can_emit_object_lod(
        ref_flags: u32,
        base_flags: u32,
        in_visible_distant_group: bool,
        has_multiref_lod_link: bool,
        base_signature: &str,
    ) -> bool {
        base_signature == "TREE"
            || in_visible_distant_group
            || has_multiref_lod_link
            || ref_flags & VISIBLE_WHEN_DISTANT_FLAG != 0
            || base_flags & VISIBLE_WHEN_DISTANT_FLAG != 0
    }

    fn scol_part_can_emit_object_lod(base: &ResolvedBase) -> bool {
        base.signature == "TREE" || base.record_flags & VISIBLE_WHEN_DISTANT_FLAG != 0
    }

    fn subrecord<'a>(record: &'a ParsedRecord, sig: &str) -> Option<&'a [u8]> {
        record
            .subrecords
            .iter()
            .find(|s| s.signature.as_str() == sig)
            .map(|s| s.data.as_ref())
    }

    fn placed_ref_has_multiref_lod_link(record: &ParsedRecord) -> bool {
        record
            .subrecords
            .iter()
            .filter(|s| s.signature.as_str() == "XLKR")
            .any(|s| {
                read_u32_at(s.data.as_ref(), 0)
                    .map(|raw| raw & 0x00ff_ffff)
                    .is_some_and(|object_id| object_id == MULTIREF_LOD_KEYWORD_OBJECT_ID)
            })
    }

    fn read_u32_at(data: &[u8], offset: usize) -> Option<u32> {
        let bytes = data.get(offset..offset.checked_add(4)?)?;
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn record_editor_id(record: &ParsedRecord) -> Option<String> {
        subrecord(record, "EDID").map(|d| {
            let end = d.iter().position(|&b| b == 0).unwrap_or(d.len());
            String::from_utf8_lossy(&d[..end]).into_owned()
        })
    }

    fn top_group<'a>(plugin: &'a ParsedPlugin, sig: &str) -> Option<&'a ParsedGroup> {
        let w = sig.as_bytes();
        plugin.root_items.iter().find_map(|item| match item {
            ParsedItem::Group(g) if g.group_type == 0 && g.label == [w[0], w[1], w[2], w[3]] => {
                Some(g)
            }
            _ => None,
        })
    }

    fn find_world<'a>(plugin: &'a ParsedPlugin, editor_id: &str) -> Option<&'a ParsedRecord> {
        let wrld_group = top_group(plugin, "WRLD")?;
        wrld_group.children.iter().find_map(|item| match item {
            ParsedItem::Record(r)
                if r.signature.as_str() == "WRLD"
                    && record_editor_id(r)
                        .map(|e| e.eq_ignore_ascii_case(editor_id))
                        .unwrap_or(false) =>
            {
                Some(r)
            }
            _ => None,
        })
    }

    fn find_world_children_group(
        plugin: &ParsedPlugin,
        world_form_id: u32,
    ) -> Option<&ParsedGroup> {
        let wrld_group = top_group(plugin, "WRLD")?;
        wrld_group.children.iter().find_map(|item| match item {
            ParsedItem::Group(g)
                if g.group_type == 1 && u32::from_le_bytes(g.label) == world_form_id =>
            {
                Some(g)
            }
            _ => None,
        })
    }

    fn lod_eligible_worldspaces_in_group(wrld_group: &ParsedGroup) -> Vec<String> {
        let mut worldspaces = Vec::new();
        for item in &wrld_group.children {
            let ParsedItem::Record(record) = item else {
                continue;
            };
            if record.signature.as_str() != "WRLD" {
                continue;
            }
            let Some(editor_id) = record_editor_id(record).filter(|value| !value.is_empty()) else {
                continue;
            };
            let Some(children) = wrld_group.children.iter().find_map(|item| match item {
                ParsedItem::Group(group)
                    if group.group_type == 1
                        && u32::from_le_bytes(group.label) == record.form_id =>
                {
                    Some(group)
                }
                _ => None,
            }) else {
                continue;
            };
            let mut cell_lands = Vec::new();
            collect_cell_lands(children, &mut cell_lands);
            if !cell_lands.is_empty() {
                worldspaces.push(editor_id);
            }
        }
        worldspaces
    }

    fn lod_eligible_worldspaces(plugin: &ParsedPlugin) -> Vec<String> {
        top_group(plugin, "WRLD")
            .map(lod_eligible_worldspaces_in_group)
            .unwrap_or_default()
    }

    /// Return every top-level WRLD that the native LOD generator can enumerate.
    ///
    /// Eligibility requires a direct WRLD record in the plugin's top WRLD group,
    /// a matching World Children group, and at least one exterior CELL+LAND pair.
    /// Nested records and empty/test worldspace shells are intentionally excluded.
    pub fn discover_worldspaces(
        plugin_path: &std::path::Path,
        game: &str,
    ) -> anyhow::Result<Vec<String>> {
        let path = plugin_path.to_string_lossy().into_owned();
        let plugin =
            parse_plugin_file(&path, Some(game.to_string()), true).map_err(py_err_to_anyhow)?;
        Ok(lod_eligible_worldspaces(&plugin))
    }

    #[cfg(test)]
    mod discovery_tests {
        use super::*;
        use esp_authoring_core::plugin_runtime::ParsedSubrecord;

        fn record(signature: &str, form_id: u32, subrecords: Vec<ParsedSubrecord>) -> ParsedRecord {
            ParsedRecord {
                signature: signature.into(),
                form_id,
                flags: 0,
                version_control: 0,
                form_version: None,
                version2: None,
                subrecords,
                raw_payload: None,
                parse_error: None,
            }
        }

        fn subrecord(signature: &str, data: Vec<u8>) -> ParsedSubrecord {
            ParsedSubrecord {
                signature: signature.into(),
                data: data.into(),
                semantic_type: None,
            }
        }

        fn group(label: [u8; 4], group_type: i32, children: Vec<ParsedItem>) -> ParsedGroup {
            ParsedGroup {
                label,
                group_type,
                tail: Vec::new().into(),
                children,
            }
        }

        fn world(form_id: u32, editor_id: &str) -> ParsedItem {
            let mut edid = editor_id.as_bytes().to_vec();
            edid.push(0);
            ParsedItem::Record(record("WRLD", form_id, vec![subrecord("EDID", edid)]))
        }

        fn exterior_cell_with_land(form_id: u32, x: i32, y: i32) -> Vec<ParsedItem> {
            let mut grid = x.to_le_bytes().to_vec();
            grid.extend_from_slice(&y.to_le_bytes());
            let cell = ParsedItem::Record(record("CELL", form_id, vec![subrecord("XCLC", grid)]));
            let temporary = group(
                form_id.to_le_bytes(),
                TEMPORARY_GROUP,
                vec![ParsedItem::Record(record("LAND", form_id + 1, Vec::new()))],
            );
            let children = ParsedItem::Group(group(
                form_id.to_le_bytes(),
                CELL_CHILD_GROUP,
                vec![ParsedItem::Group(temporary)],
            ));
            vec![cell, children]
        }

        fn world_children(world_form_id: u32, children: Vec<ParsedItem>) -> ParsedItem {
            ParsedItem::Group(group(world_form_id.to_le_bytes(), 1, children))
        }

        #[test]
        fn discovery_returns_multiple_direct_lod_eligible_worldspaces_only() {
            let alpha = 0x0100;
            let beta = 0x0200;
            let empty_shell = 0x0300;
            let nested = 0x0400;

            let nested_record_group = group(*b"JUNK", 7, vec![world(nested, "NestedWorld")]);
            let wrld_group = group(
                *b"WRLD",
                0,
                vec![
                    world(alpha, "AlphaWorld"),
                    world_children(alpha, exterior_cell_with_land(0x1000, -1, 2)),
                    ParsedItem::Group(nested_record_group),
                    world(empty_shell, "EmptyShell"),
                    world_children(empty_shell, Vec::new()),
                    world(beta, "BetaWorld"),
                    world_children(beta, exterior_cell_with_land(0x2000, 3, 4)),
                ],
            );

            assert_eq!(
                lod_eligible_worldspaces_in_group(&wrld_group),
                vec!["AlphaWorld".to_string(), "BetaWorld".to_string()]
            );
        }

        #[test]
        fn discovery_excludes_world_children_without_land() {
            let object_only = 0x0500;
            let cell_form_id = 0x5000;
            let mut grid = 8_i32.to_le_bytes().to_vec();
            grid.extend_from_slice(&9_i32.to_le_bytes());
            let exterior_cell =
                ParsedItem::Record(record("CELL", cell_form_id, vec![subrecord("XCLC", grid)]));
            let wrld_group = group(
                *b"WRLD",
                0,
                vec![
                    world(object_only, "ObjectOnlyWorld"),
                    world_children(object_only, vec![exterior_cell]),
                ],
            );

            assert!(lod_eligible_worldspaces_in_group(&wrld_group).is_empty());
        }
    }

    fn cell_grid(record: &ParsedRecord) -> Option<(i32, i32)> {
        let xclc = subrecord(record, "XCLC")?;
        if xclc.len() < 8 {
            return None;
        }
        Some((
            i32::from_le_bytes([xclc[0], xclc[1], xclc[2], xclc[3]]),
            i32::from_le_bytes([xclc[4], xclc[5], xclc[6], xclc[7]]),
        ))
    }

    /// One CELL + its LAND record (the LAND lives in the cell's temporary child group).
    struct CellLand<'a> {
        grid: (i32, i32),
        cell: &'a ParsedRecord,
        land: &'a ParsedRecord,
    }

    /// Find the LAND record inside a CELL's child group (type 6 -> temporary 9 -> LAND).
    fn land_in_cell_child_group<'a>(child_group: &'a ParsedGroup) -> Option<&'a ParsedRecord> {
        for item in &child_group.children {
            if let ParsedItem::Group(g) = item {
                if g.group_type == TEMPORARY_GROUP {
                    for ti in &g.children {
                        if let ParsedItem::Record(r) = ti {
                            if r.signature.as_str() == "LAND" {
                                return Some(r);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Walk an exterior-cell block/sub-block tree and emit (CELL grid, LAND) pairs.
    /// CELL records are followed by their type-6 cell-child group as a sibling.
    fn collect_cell_lands<'a>(group: &'a ParsedGroup, out: &mut Vec<CellLand<'a>>) {
        let items = &group.children;
        let mut i = 0;
        while i < items.len() {
            match &items[i] {
                ParsedItem::Group(g)
                    if g.group_type == EXTERIOR_CELL_BLOCK
                        || g.group_type == EXTERIOR_CELL_SUBBLOCK =>
                {
                    collect_cell_lands(g, out);
                }
                ParsedItem::Record(r) if r.signature.as_str() == "CELL" => {
                    // The following sibling is normally this cell's child group (type 6).
                    if let Some((cx, cy)) = cell_grid(r) {
                        let child = items.get(i + 1).and_then(|next| match next {
                            ParsedItem::Group(g) if g.group_type == CELL_CHILD_GROUP => Some(g),
                            _ => None,
                        });
                        if let Some(child) = child {
                            if let Some(land) = land_in_cell_child_group(child) {
                                out.push(CellLand {
                                    grid: (cx, cy),
                                    cell: r,
                                    land,
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    // ---------------------------------------------------------------------------
    // LTEX -> TXST -> diffuse/normal resolution across plugin + masters.
    // ---------------------------------------------------------------------------

    fn find_record_by_form_id<'a>(
        plugin: &'a ParsedPlugin,
        sig: &str,
        form_id: u32,
    ) -> Option<&'a ParsedRecord> {
        let group = top_group(plugin, sig)?;
        let mut found = None;
        fn walk<'a>(
            g: &'a ParsedGroup,
            sig: &str,
            form_id: u32,
            found: &mut Option<&'a ParsedRecord>,
        ) {
            for item in &g.children {
                match item {
                    ParsedItem::Record(r)
                        if r.signature.as_str() == sig && r.form_id == form_id =>
                    {
                        *found = Some(r);
                        return;
                    }
                    ParsedItem::Group(child) => walk(child, sig, form_id, found),
                    _ => {}
                }
            }
        }
        walk(group, sig, form_id, &mut found);
        found
    }

    fn zstring(data: &[u8]) -> String {
        let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
        String::from_utf8_lossy(&data[..end]).into_owned()
    }

    /// Resolve an LTEX form id to its (diffuse, normal) Data-relative texture paths.
    /// LTEX.TNAM -> TXST; TXST.TX00 = diffuse, TX01 = normal (FO4 schema). Searches
    /// the WRLD plugin first, then masters. Returns empty strings if unresolved.
    fn resolve_ltex_textures(handle: &EspHandle, ltex_form_id: u32) -> (String, String) {
        let plugins = std::iter::once(handle.plugin.as_ref())
            .flatten()
            .chain(handle.masters.iter());

        // Find the LTEX record.
        let mut ltex = None;
        for p in plugins.clone() {
            if let Some(r) = find_record_by_form_id(p, "LTEX", ltex_form_id) {
                ltex = Some(r);
                break;
            }
        }
        let Some(ltex) = ltex else {
            return (String::new(), String::new());
        };
        let Some(tnam) = subrecord(ltex, "TNAM") else {
            return (String::new(), String::new());
        };
        if tnam.len() < 4 {
            return (String::new(), String::new());
        }
        let txst_form_id = u32::from_le_bytes([tnam[0], tnam[1], tnam[2], tnam[3]]);

        let mut txst = None;
        for p in plugins {
            if let Some(r) = find_record_by_form_id(p, "TXST", txst_form_id) {
                txst = Some(r);
                break;
            }
        }
        let Some(txst) = txst else {
            return (String::new(), String::new());
        };
        let diffuse = subrecord(txst, "TX00").map(zstring).unwrap_or_default();
        let normal = subrecord(txst, "TX01").map(zstring).unwrap_or_default();
        (diffuse, normal)
    }

    fn resolve_ltex_grasses(handle: &EspHandle, ltex_form_id: u32) -> Vec<GrassModelInput> {
        let plugins = std::iter::once(handle.plugin.as_ref())
            .flatten()
            .chain(handle.masters.iter());
        let Some(ltex) = plugins
            .clone()
            .find_map(|plugin| find_record_by_form_id(plugin, "LTEX", ltex_form_id))
        else {
            return Vec::new();
        };

        let mut grasses = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for gnam in ltex
            .subrecords
            .iter()
            .filter(|sub| sub.signature.as_str() == "GNAM" && sub.data.len() >= 4)
        {
            let grass_form_id =
                u32::from_le_bytes([gnam.data[0], gnam.data[1], gnam.data[2], gnam.data[3]]);
            if !seen.insert(grass_form_id) {
                continue;
            }
            let Some(grass) = plugins
                .clone()
                .find_map(|plugin| find_record_by_form_id(plugin, "GRAS", grass_form_id))
            else {
                continue;
            };
            let model = subrecord(grass, "MODL").map(zstring).unwrap_or_default();
            if model.is_empty() {
                continue;
            }
            let max_slope_degrees = subrecord(grass, "DATA")
                .and_then(|data| data.get(2))
                .copied()
                .map(f32::from)
                .filter(|value| *value > 0.0)
                .unwrap_or(90.0);
            grasses.push(GrassModelInput {
                base_name: record_editor_id(grass).unwrap_or_default(),
                model,
                max_slope_degrees,
            });
        }
        grasses
    }

    /// Build the `LayerTexture` list for a LAND record. The BTXT base texture for a
    /// quadrant becomes an opaque layer (alpha treated as 1.0); each ATXT becomes a
    /// layer whose alpha comes from the VTXT that immediately follows it in
    /// subrecord order (FO4 LAND encoding).
    fn build_layers(
        handle: &EspHandle,
        land: &ParsedRecord,
    ) -> (Vec<LayerTexture>, Vec<GrassLayerInput>) {
        let mut layers = Vec::new();
        let mut grass_layers = Vec::new();
        let mut subs = land.subrecords.iter().peekable();
        while let Some(sub) = subs.next() {
            match sub.signature.as_str() {
                "BTXT" => {
                    if let Some(h) = decode_layer_header(&sub.data) {
                        let (diffuse, normal) = resolve_ltex_textures(handle, h.texture_form_id);
                        let grasses = resolve_ltex_grasses(handle, h.texture_form_id);
                        layers.push(LayerTexture {
                            diffuse,
                            normal,
                            quadrant: h.quadrant,
                            // Base layer: opaque. textures.rs treats a non-17x17 alpha
                            // vec as fully opaque, which is the intended base behavior.
                            alpha: Vec::new(),
                        });
                        if !grasses.is_empty() {
                            grass_layers.push(GrassLayerInput {
                                quadrant: h.quadrant,
                                alpha: Vec::new(),
                                models: grasses,
                            });
                        }
                    }
                }
                "ATXT" => {
                    if let Some(h) = decode_layer_header(&sub.data) {
                        let (diffuse, normal) = resolve_ltex_textures(handle, h.texture_form_id);
                        let grasses = resolve_ltex_grasses(handle, h.texture_form_id);
                        // The paired VTXT (if any) follows this ATXT.
                        let alpha = match subs.peek() {
                            Some(next) if next.signature.as_str() == "VTXT" => {
                                let a = decode_vtxt_alpha(&next.data);
                                subs.next();
                                a
                            }
                            _ => vec![0.0f32; QUADRANT_ALPHA_EDGE * QUADRANT_ALPHA_EDGE],
                        };
                        let _ = h.layer;
                        layers.push(LayerTexture {
                            diffuse,
                            normal,
                            quadrant: h.quadrant,
                            alpha: alpha.clone(),
                        });
                        if !grasses.is_empty() {
                            grass_layers.push(GrassLayerInput {
                                quadrant: h.quadrant,
                                alpha,
                                models: grasses,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        (layers, grass_layers)
    }

    // ---------------------------------------------------------------------------
    // REFR placed-object enumeration.
    //
    // Port: xLODGen's xEdit-side reference walk that fills `StaticDesc` (the
    // LODGenerator C# `StaticDesc` struct, StaticDesc.cs) + the FO4 object path of
    // LODApp.ParseNif (LODApp.cs:1369-1391). The decompiled `LODGenerator` consumes
    // a pre-built `staticModels[4]`; we reproduce that array here from the base
    // record's DistantLOD (STAT/SCOL/MSTT/... MNAM) subrecord.
    //
    // FO4 binary layouts (verified against esp generated/fo4.rs):
    //   REFR.NAME = formid (4 bytes LE) — the base form id.
    //   REFR.DATA = 6×f32 (posX,posY,posZ, rotX,rotY,rotZ) — 24 bytes.
    //   REFR.XSCL = 1×f32 scale — 4 bytes (absent → 1.0).
    //   REFR.XESP = formid(4) + flags(1) + 3 pad — 8 bytes (enable parent).
    //   base.EDID = zstring; base.MODL = zstring (full model);
    //   base.MNAM "DistantLOD" = 4 × char[260] (CP-1252, zero-padded) = 1040 bytes,
    //             one fixed-260 slot per LOD level (Level0..Level3), NO trailing flags.
    // ---------------------------------------------------------------------------

    /// Decode REFR.DATA → (position[3], rotation[3]). Returns None if too short.
    pub fn decode_refr_data(data: &[u8]) -> Option<([f32; 3], [f32; 3])> {
        if data.len() < 24 {
            return None;
        }
        let f = |off: usize| {
            f32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
        };
        Some(([f(0), f(4), f(8)], [f(12), f(16), f(20)]))
    }

    /// Decode REFR.XSCL → scale. Returns None if too short.
    pub fn decode_refr_scale(data: &[u8]) -> Option<f32> {
        if data.len() < 4 {
            return None;
        }
        Some(f32::from_le_bytes([data[0], data[1], data[2], data[3]]))
    }

    /// Decode a base record's MNAM "DistantLOD" subrecord into 4 optional LOD model
    /// paths. Each slot is a fixed 260-byte CP-1252 buffer; an all-empty slot yields
    /// `None`. A truncated subrecord fills the remaining slots with `None`.
    /// port: the FO4 STAT/SCOL/MSTT MNAM (DistantLOD, char[260]×4) → StaticDesc.staticModels.
    pub fn decode_distant_lod(mnam: &[u8]) -> [Option<String>; 4] {
        const SLOT: usize = 260;
        let mut out: [Option<String>; 4] = [None, None, None, None];
        for (i, slot) in out.iter_mut().enumerate() {
            let off = i * SLOT;
            if off + SLOT > mnam.len() {
                break;
            }
            let buf = &mnam[off..off + SLOT];
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            if end == 0 {
                continue;
            }
            // CP-1252 is ASCII-compatible for path bytes; lossy UTF-8 is faithful for
            // the LOD model paths xLODGen consumes (they are plain ASCII filenames).
            let s = String::from_utf8_lossy(&buf[..end]).trim().to_string();
            if !s.is_empty() {
                *slot = Some(s);
            }
        }
        out
    }

    /// A base record (STAT/SCOL/MSTT/...) resolved for object-LOD enumeration.
    #[derive(Clone)]
    struct ResolvedBase {
        signature: String,
        editor_id: String,
        record_flags: u32,
        full_model: String,
        lod_models: [Option<String>; 4],
        scol_parts: Vec<ScolPart>,
        /// Material file referenced by the base model's MODS/material name, if any.
        /// (FO4 LOD only needs the per-shape material from the NIF; left empty here —
        /// material swaps come from the REFR, not the base.)
        material_name: String,
    }

    #[derive(Clone)]
    struct ScolPart {
        base_form_id: u32,
        pos: [f32; 3],
        rot: [f32; 3],
        scale: f32,
    }

    fn decode_scol_parts(record: &ParsedRecord) -> Vec<ScolPart> {
        let mut parts = Vec::new();
        let mut current_base = None;
        for sub in &record.subrecords {
            match sub.signature.as_str() {
                "ONAM" if sub.data.len() >= 4 => {
                    current_base = Some(u32::from_le_bytes([
                        sub.data[0],
                        sub.data[1],
                        sub.data[2],
                        sub.data[3],
                    ]));
                }
                "DATA" => {
                    let Some(base_form_id) = current_base else {
                        continue;
                    };
                    for placement in sub.data.chunks_exact(28) {
                        let f = |off: usize| {
                            f32::from_le_bytes([
                                placement[off],
                                placement[off + 1],
                                placement[off + 2],
                                placement[off + 3],
                            ])
                        };
                        let scale = f(24);
                        parts.push(ScolPart {
                            base_form_id,
                            pos: [f(0), f(4), f(8)],
                            rot: [f(12), f(16), f(20)],
                            scale: if scale == 0.0 { 1.0 } else { scale },
                        });
                    }
                }
                _ => {}
            }
        }
        parts
    }

    fn rotation_from_ref(rot: [f32; 3]) -> [[f32; 3]; 3] {
        let mx = rot_x(-rot[0]);
        let my = rot_y(-rot[1]);
        let mz = rot_z(-rot[2]);
        mat3_mul(&mat3_mul(&mx, &my), &mz)
    }

    fn rot_x(a: f32) -> [[f32; 3]; 3] {
        let (s, c) = a.sin_cos();
        [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
    }

    fn rot_y(a: f32) -> [[f32; 3]; 3] {
        let (s, c) = a.sin_cos();
        [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
    }

    fn rot_z(a: f32) -> [[f32; 3]; 3] {
        let (s, c) = a.sin_cos();
        [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
    }

    fn mat3_mul(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
        let mut out = [[0.0f32; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                for k in 0..3 {
                    out[i][j] += a[i][k] * b[k][j];
                }
            }
        }
        out
    }

    fn scol_part_transform(part: &ScolPart) -> [[f32; 4]; 4] {
        let r = rotation_from_ref(part.rot);
        [
            [r[0][0], r[0][1], r[0][2], part.pos[0]],
            [r[1][0], r[1][1], r[1][2], part.pos[1]],
            [r[2][0], r[2][1], r[2][2], part.pos[2]],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }

    /// Resolve a base form id to its EDID / flags / MODL / DistantLOD across the
    /// WRLD plugin first, then masters. Returns None if the base record is absent.
    /// Only base record types that can carry DistantLOD LOD models are resolved.
    fn resolve_base_record(handle: &EspHandle, base_form_id: u32) -> Option<ResolvedBase> {
        // Object base record signatures that can have a DistantLOD (MNAM) in FO4.
        // port: the xEdit reference walk only fetches LOD for these LOD-capable bases.
        const LOD_BASE_SIGS: [&str; 6] = ["STAT", "SCOL", "MSTT", "TREE", "FLOR", "ACTI"];

        let plugins = std::iter::once(handle.plugin.as_ref())
            .flatten()
            .chain(handle.masters.iter());

        for p in plugins {
            for sig in LOD_BASE_SIGS {
                if let Some(rec) = find_record_by_form_id(p, sig, base_form_id) {
                    let editor_id = record_editor_id(rec).unwrap_or_default();
                    let full_model = subrecord(rec, "MODL").map(zstring).unwrap_or_default();
                    let lod_models = subrecord(rec, "MNAM")
                        .map(decode_distant_lod)
                        .unwrap_or([None, None, None, None]);
                    let scol_parts = if sig == "SCOL" {
                        decode_scol_parts(rec)
                    } else {
                        Vec::new()
                    };
                    return Some(ResolvedBase {
                        signature: sig.to_string(),
                        editor_id,
                        record_flags: rec.flags,
                        full_model,
                        lod_models,
                        scol_parts,
                        material_name: String::new(),
                    });
                }
            }
        }
        None
    }

    /// A REFR found inside a CELL child group, paired with the grid it sits in.
    struct PlacedRef<'a> {
        record: &'a ParsedRecord,
        cell: (i32, i32),
        in_visible_distant_group: bool,
    }

    fn ref_position_cell(record: &ParsedRecord) -> (i32, i32) {
        let (pos, _) = subrecord(record, "DATA")
            .and_then(decode_refr_data)
            .unwrap_or(([0.0; 3], [0.0; 3]));
        (
            (pos[0] / FO4_CELL_SIZE).floor() as i32,
            (pos[1] / FO4_CELL_SIZE).floor() as i32,
        )
    }

    /// Collect REFR records from a CELL's child group: persistent group 8,
    /// temporary group 9, and visible-when-distant group 10. ACHR/PGRE/etc. are
    /// ignored — only static placements get object LOD.
    fn collect_cell_refs<'a>(
        child_group: &'a ParsedGroup,
        cell: Option<(i32, i32)>,
        out: &mut Vec<PlacedRef<'a>>,
    ) {
        for item in &child_group.children {
            if let ParsedItem::Group(g) = item {
                if matches!(
                    g.group_type,
                    PERSISTENT_GROUP | TEMPORARY_GROUP | VISIBLE_DISTANT_GROUP
                ) {
                    let in_visible_distant_group = g.group_type == VISIBLE_DISTANT_GROUP;
                    for ti in &g.children {
                        if let ParsedItem::Record(r) = ti {
                            if r.signature.as_str() == "REFR" {
                                out.push(PlacedRef {
                                    record: r,
                                    cell: cell.unwrap_or_else(|| ref_position_cell(r)),
                                    in_visible_distant_group,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    /// Walk the exterior-cell block/sub-block tree and collect all REFRs (with their
    /// owning CELL grid). Mirrors `collect_cell_lands` but for placed references.
    fn collect_world_refs<'a>(group: &'a ParsedGroup, out: &mut Vec<PlacedRef<'a>>) {
        let items = &group.children;
        let mut i = 0;
        while i < items.len() {
            match &items[i] {
                ParsedItem::Group(g)
                    if g.group_type == EXTERIOR_CELL_BLOCK
                        || g.group_type == EXTERIOR_CELL_SUBBLOCK =>
                {
                    collect_world_refs(g, out);
                }
                ParsedItem::Record(r) if r.signature.as_str() == "CELL" => {
                    let child = items.get(i + 1).and_then(|next| match next {
                        ParsedItem::Group(g) if g.group_type == CELL_CHILD_GROUP => Some(g),
                        _ => None,
                    });
                    if let Some(child) = child {
                        collect_cell_refs(child, cell_grid(r), out);
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// Decode a REFR's XESP enable-parent form id (0 if absent).
    fn refr_enable_parent(record: &ParsedRecord) -> u32 {
        subrecord(record, "XESP")
            .filter(|d| d.len() >= 4)
            .map(|d| u32::from_le_bytes([d[0], d[1], d[2], d[3]]))
            .unwrap_or(0)
    }

    fn normalize_material_swap_path(path: &str) -> String {
        let mut name = path.trim_end_matches('\0').trim().replace('/', "\\");
        let lower = name.to_lowercase();
        if let Some(pos) = lower.rfind("data\\") {
            name = name[pos + 5..].to_string();
        }
        name.to_lowercase()
    }

    fn decode_material_swap_record(
        record: &ParsedRecord,
    ) -> std::collections::BTreeMap<String, String> {
        let mut out = std::collections::BTreeMap::new();
        let mut from = None;
        for sub in &record.subrecords {
            match sub.signature.as_str() {
                "BNAM" => from = Some(normalize_material_swap_path(&zstring(sub.data.as_ref()))),
                "SNAM" => {
                    let Some(old) = from.take() else {
                        continue;
                    };
                    let new = normalize_material_swap_path(&zstring(sub.data.as_ref()));
                    if !old.is_empty() && !new.is_empty() {
                        if let Some(stripped) = old.strip_prefix("materials\\") {
                            out.insert(stripped.to_string(), new.clone());
                        } else {
                            out.insert(format!("materials\\{old}"), new.clone());
                        }
                        out.insert(old, new);
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn resolve_ref_material_swap(
        handle: &EspHandle,
        record: &ParsedRecord,
    ) -> std::collections::BTreeMap<String, String> {
        let Some(xmsp) = subrecord(record, "XMSP").filter(|d| d.len() >= 4) else {
            return std::collections::BTreeMap::new();
        };
        let mswp_form_id = u32::from_le_bytes([xmsp[0], xmsp[1], xmsp[2], xmsp[3]]);
        let plugins = std::iter::once(handle.plugin.as_ref())
            .flatten()
            .chain(handle.masters.iter());
        for p in plugins {
            if let Some(r) = find_record_by_form_id(p, "MSWP", mswp_form_id) {
                return decode_material_swap_record(r);
            }
        }
        std::collections::BTreeMap::new()
    }

    fn static_desc_base_flags(base: &ResolvedBase) -> u32 {
        if base.signature == "TREE" {
            base.record_flags | crate::objects::static_desc::ShapeFlags::IS_TREE.bits()
        } else {
            base.record_flags
        }
    }

    fn push_ref_input(
        out: &mut Vec<RefInput>,
        record: &ParsedRecord,
        placed: &PlacedRef,
        base: &ResolvedBase,
        pos: [f32; 3],
        rot: [f32; 3],
        scale: f32,
        ref_id_suffix: &str,
        part_transform: [[f32; 4]; 4],
        part_scale: f32,
        material_swap: &std::collections::BTreeMap<String, String>,
    ) {
        let enable_parent = refr_enable_parent(record);
        out.push(RefInput {
            ref_id: format!("{:08X}{ref_id_suffix}", record.form_id),
            ref_flags: record.flags,
            enable_parent,
            cell: placed.cell,
            pos,
            rot,
            scale,
            // color (XCLP) / per-vertex tint is a Phase-3 grass/tree concern; default 1.0.
            color: 1.0,
            // alpha threshold comes from the shape's NiAlphaProperty/material at parse time.
            alpha_threshold: 128,
            // .dds LOD model → billboard (LODApp.cs:1389-1391). Set from slot-0 model.
            is_billboard: base
                .lod_models
                .iter()
                .flatten()
                .next()
                .map(|m| m.to_lowercase().ends_with(".dds"))
                .unwrap_or(false),
            is_grass: false,
            base_name: base.editor_id.clone(),
            base_flags: static_desc_base_flags(base),
            material_name: base.material_name.clone(),
            full_model: base.full_model.clone(),
            lod_models: base.lod_models.clone(),
            part_transform,
            part_scale,
            material_swap: material_swap.clone(),
        });
    }

    #[cfg(test)]
    mod ref_input_tests {
        use super::*;

        fn resolved_base(signature: &str, record_flags: u32) -> ResolvedBase {
            ResolvedBase {
                signature: signature.to_string(),
                editor_id: String::new(),
                record_flags,
                full_model: String::new(),
                lod_models: [None, None, None, None],
                scol_parts: Vec::new(),
                material_name: String::new(),
            }
        }

        #[test]
        fn tree_base_signature_sets_static_desc_tree_flag() {
            let tree = resolved_base("TREE", 0x20);
            let stat = resolved_base("STAT", 0x20);

            assert_eq!(static_desc_base_flags(&tree), 0x1020);
            assert_eq!(static_desc_base_flags(&stat), 0x20);
        }

        #[test]
        fn virtual_overlay_changes_only_the_resolved_copy() {
            let original = resolved_base("ACTI", 0);
            let overlay = OverlayEntry {
                reference_form_id: 0x100,
                placed_base_form_id: 0x200,
                component_index: None,
                component_base_form_id: None,
                base_signature: "ACTI".to_string(),
                lod_models: [Some(r"LOD\Test.nif".to_string()), None, None, None],
                force_visible: true,
            };

            let (resolved, force_visible) = overlay_resolved_base(original.clone(), Some(&overlay));
            assert_eq!(resolved.lod_models, overlay.lod_models);
            assert!(force_visible);
            assert!(original.lod_models.iter().all(Option::is_none));

            let (unchanged, force_visible) = overlay_resolved_base(original.clone(), None);
            assert_eq!(unchanged.lod_models, original.lod_models);
            assert!(!force_visible);
        }

        #[test]
        fn disabled_reference_filter_remains_authoritative() {
            let record = ParsedRecord {
                signature: "REFR".into(),
                form_id: 0x100,
                flags: REFR_FLAG_INITIALLY_DISABLED,
                version_control: 0,
                form_version: None,
                version2: None,
                subrecords: Vec::new(),
                raw_payload: None,
                parse_error: None,
            };

            assert!(should_skip_placed_ref(&record));
        }

        #[test]
        fn deleted_reference_and_deleted_bases_are_skipped() {
            let record = ParsedRecord {
                signature: "REFR".into(),
                form_id: 0x100,
                flags: RECORD_FLAG_DELETED,
                version_control: 0,
                form_version: None,
                version2: None,
                subrecords: Vec::new(),
                raw_payload: None,
                parse_error: None,
            };
            assert!(should_skip_placed_ref(&record));

            for signature in ["ACTI", "SCOL", "MSTT"] {
                let deleted = resolved_base(signature, RECORD_FLAG_DELETED);
                assert!(
                    should_skip_base(&deleted),
                    "deleted {signature} was accepted"
                );
            }
        }

        #[test]
        fn scol_parent_overlay_emits_only_visible_or_force_visible_placement() {
            fn subrecord(
                signature: &str,
                data: Vec<u8>,
            ) -> esp_authoring_core::plugin_runtime::ParsedSubrecord {
                esp_authoring_core::plugin_runtime::ParsedSubrecord {
                    signature: signature.into(),
                    data: bytes::Bytes::from(data),
                    semantic_type: None,
                }
            }
            fn record(
                signature: &str,
                form_id: u32,
                subrecords: Vec<esp_authoring_core::plugin_runtime::ParsedSubrecord>,
            ) -> ParsedRecord {
                ParsedRecord {
                    signature: signature.into(),
                    form_id,
                    flags: 0,
                    version_control: 0,
                    form_version: None,
                    version2: None,
                    subrecords,
                    raw_payload: None,
                    parse_error: None,
                }
            }

            let base_form_id = 0x200;
            let hidden_ref_id = 0x100;
            let visible_ref_id = 0x101;
            let base = record("SCOL", base_form_id, vec![]);
            let hidden_ref = record(
                "REFR",
                hidden_ref_id,
                vec![subrecord("NAME", base_form_id.to_le_bytes().to_vec())],
            );
            let visible_ref = record(
                "REFR",
                visible_ref_id,
                vec![subrecord("NAME", base_form_id.to_le_bytes().to_vec())],
            );
            let overlay_entry = |reference_form_id, force_visible| OverlayEntry {
                reference_form_id,
                placed_base_form_id: base_form_id,
                component_index: None,
                component_base_form_id: None,
                base_signature: "SCOL".to_string(),
                lod_models: [Some(r"LOD\SCOL.nif".to_string()), None, None, None],
                force_visible,
            };
            let handle = EspHandle {
                plugin: Some(ParsedPlugin {
                    plugin_name: "Output.esm".to_string(),
                    file_path: String::new(),
                    header_size: 0,
                    header: esp_authoring_core::plugin_runtime::ParsedPluginHeader {
                        version: 1.0,
                        num_records: 0,
                        next_object_id: 0x800,
                        author: String::new(),
                        description: String::new(),
                        masters: Vec::new(),
                        master_sizes: Vec::new(),
                        overridden_forms: Vec::new(),
                        flags: 0,
                        extra_subrecords: Vec::new(),
                        version_control: 0,
                        form_version: None,
                        version2: None,
                        hedr_raw: None,
                        raw_subrecords: Vec::new(),
                    },
                    root_items: vec![ParsedItem::Group(ParsedGroup {
                        label: *b"SCOL",
                        group_type: 0,
                        tail: bytes::Bytes::new(),
                        children: vec![ParsedItem::Record(base)],
                    })],
                    game: Some("fo4".to_string()),
                }),
                masters: Vec::new(),
                object_lod_overlay: Some(ObjectLodOverlay::from_entries_for_test(vec![
                    overlay_entry(hidden_ref_id, false),
                    overlay_entry(visible_ref_id, true),
                ])),
            };
            let placements = [
                PlacedRef {
                    record: &hidden_ref,
                    cell: (0, 0),
                    in_visible_distant_group: false,
                },
                PlacedRef {
                    record: &visible_ref,
                    cell: (0, 0),
                    in_visible_distant_group: false,
                },
            ];
            let mut output = Vec::new();
            for placement in &placements {
                append_ref_inputs(&handle, placement, &mut output);
            }

            assert_eq!(output.len(), 1);
            assert_eq!(output[0].ref_id, format!("{visible_ref_id:08X}"));
        }

        #[test]
        fn visible_scol_component_overlay_is_consumed() {
            fn subrecord(
                signature: &str,
                data: Vec<u8>,
            ) -> esp_authoring_core::plugin_runtime::ParsedSubrecord {
                esp_authoring_core::plugin_runtime::ParsedSubrecord {
                    signature: signature.into(),
                    data: bytes::Bytes::from(data),
                    semantic_type: None,
                }
            }
            fn record(
                signature: &str,
                form_id: u32,
                subrecords: Vec<esp_authoring_core::plugin_runtime::ParsedSubrecord>,
            ) -> ParsedRecord {
                ParsedRecord {
                    signature: signature.into(),
                    form_id,
                    flags: 0,
                    version_control: 0,
                    form_version: None,
                    version2: None,
                    subrecords,
                    raw_payload: None,
                    parse_error: None,
                }
            }
            let scol_form_id = 0x200_u32;
            let component_form_id = 0x201_u32;
            let stat_component_form_id = 0x202_u32;
            let reference_form_id = 0x100_u32;
            let mut component_data = vec![0_u8; 28];
            component_data[24..28].copy_from_slice(&1.0_f32.to_le_bytes());
            let mut stat_lod = vec![0_u8; 260 * 4];
            stat_lod[..12].copy_from_slice(b"LOD\\Stat.nif");
            let scol = record(
                "SCOL",
                scol_form_id,
                vec![
                    subrecord("ONAM", component_form_id.to_le_bytes().to_vec()),
                    subrecord("DATA", component_data.clone()),
                    subrecord("ONAM", stat_component_form_id.to_le_bytes().to_vec()),
                    subrecord("DATA", component_data),
                ],
            );
            let component = record("MSTT", component_form_id, vec![]);
            let stat_component = record(
                "STAT",
                stat_component_form_id,
                vec![subrecord("MNAM", stat_lod)],
            );
            let placed = record(
                "REFR",
                reference_form_id,
                vec![subrecord("NAME", scol_form_id.to_le_bytes().to_vec())],
            );
            let handle = EspHandle {
                plugin: Some(ParsedPlugin {
                    plugin_name: "Output.esm".to_string(),
                    file_path: String::new(),
                    header_size: 0,
                    header: esp_authoring_core::plugin_runtime::ParsedPluginHeader {
                        version: 1.0,
                        num_records: 0,
                        next_object_id: 0x800,
                        author: String::new(),
                        description: String::new(),
                        masters: Vec::new(),
                        master_sizes: Vec::new(),
                        overridden_forms: Vec::new(),
                        flags: 0,
                        extra_subrecords: Vec::new(),
                        version_control: 0,
                        form_version: None,
                        version2: None,
                        hedr_raw: None,
                        raw_subrecords: Vec::new(),
                    },
                    root_items: vec![
                        ParsedItem::Group(ParsedGroup {
                            label: *b"SCOL",
                            group_type: 0,
                            tail: bytes::Bytes::new(),
                            children: vec![ParsedItem::Record(scol)],
                        }),
                        ParsedItem::Group(ParsedGroup {
                            label: *b"MSTT",
                            group_type: 0,
                            tail: bytes::Bytes::new(),
                            children: vec![ParsedItem::Record(component)],
                        }),
                        ParsedItem::Group(ParsedGroup {
                            label: *b"STAT",
                            group_type: 0,
                            tail: bytes::Bytes::new(),
                            children: vec![ParsedItem::Record(stat_component)],
                        }),
                    ],
                    game: Some("fo4".to_string()),
                }),
                masters: Vec::new(),
                object_lod_overlay: Some(ObjectLodOverlay::from_entries_for_test(vec![
                    OverlayEntry {
                        reference_form_id,
                        placed_base_form_id: scol_form_id,
                        component_index: Some(0),
                        component_base_form_id: Some(component_form_id),
                        base_signature: "MSTT".to_string(),
                        lod_models: [Some(r"LOD\Component.nif".to_string()), None, None, None],
                        force_visible: true,
                    },
                ])),
            };
            let mut output = Vec::new();
            append_ref_inputs(
                &handle,
                &PlacedRef {
                    record: &placed,
                    cell: (0, 0),
                    in_visible_distant_group: true,
                },
                &mut output,
            );

            assert_eq!(output.len(), 2);
            assert_eq!(output[0].ref_id, format!("{reference_form_id:08X}:0"));
            assert_eq!(
                output[0].lod_models[0].as_deref(),
                Some(r"LOD\Component.nif")
            );
            assert_eq!(output[1].ref_id, format!("{reference_form_id:08X}:1"));
            assert_eq!(output[1].lod_models[0].as_deref(), Some(r"LOD\Stat.nif"));
        }
    }

    /// Build `RefInput`s from a placed REFR + its resolved base. SCOL bases expand
    /// to their component STAT placements, matching wbLOD.pas ProcessReference.
    fn append_ref_inputs(handle: &EspHandle, placed: &PlacedRef, out: &mut Vec<RefInput>) {
        let record = placed.record;

        // port: ProcessReference — skip disabled placements.
        if should_skip_placed_ref(record) {
            return;
        }

        let Some(name) = subrecord(record, "NAME") else {
            return;
        };
        if name.len() < 4 {
            return;
        }
        let base_form_id = u32::from_le_bytes([name[0], name[1], name[2], name[3]]);

        let Some(base) = resolve_base_record(handle, base_form_id) else {
            return;
        };
        if should_skip_base(&base) {
            return;
        }
        let parent_overlay = handle
            .object_lod_overlay
            .as_ref()
            .and_then(|overlay| overlay.parent(record.form_id, base_form_id))
            .filter(|entry| entry.base_signature == base.signature);
        let (base, parent_overlay_force_visible) = overlay_resolved_base(base, parent_overlay);
        let has_multiref_lod_link = placed_ref_has_multiref_lod_link(record);
        let parent_can_emit = ref_can_emit_object_lod(
            record.flags,
            base.record_flags,
            placed.in_visible_distant_group,
            has_multiref_lod_link,
            base.signature.as_str(),
        );
        if base.signature != "SCOL" && !parent_can_emit && !parent_overlay_force_visible {
            return;
        }

        let (pos, rot) = subrecord(record, "DATA")
            .and_then(decode_refr_data)
            .unwrap_or(([0.0; 3], [0.0; 3]));
        let scale = subrecord(record, "XSCL")
            .and_then(decode_refr_scale)
            .unwrap_or(1.0);
        let material_swap = resolve_ref_material_swap(handle, record);

        if base.signature == "SCOL" {
            if parent_overlay.is_some() {
                if !parent_can_emit && !parent_overlay_force_visible {
                    return;
                }
                push_ref_input(
                    out,
                    record,
                    placed,
                    &base,
                    pos,
                    rot,
                    scale,
                    "",
                    super::identity_part_transform(),
                    1.0,
                    &material_swap,
                );
                return;
            }
            for (i, part) in base.scol_parts.iter().enumerate() {
                let Some(part_base) = resolve_base_record(handle, part.base_form_id) else {
                    continue;
                };
                if should_skip_base(&part_base) {
                    continue;
                }
                let component_overlay = handle.object_lod_overlay.as_ref().and_then(|overlay| {
                    overlay.component(record.form_id, base_form_id, i as u32, part.base_form_id)
                });
                let component_overlay =
                    component_overlay.filter(|entry| entry.base_signature == part_base.signature);
                let (part_base, component_force_visible) =
                    overlay_resolved_base(part_base, component_overlay);
                if !parent_can_emit
                    && !component_force_visible
                    && !scol_part_can_emit_object_lod(&part_base)
                {
                    continue;
                }
                if part_base.lod_models.iter().all(Option::is_none) {
                    continue;
                }
                push_ref_input(
                    out,
                    record,
                    placed,
                    &part_base,
                    pos,
                    rot,
                    scale,
                    &format!(":{i}"),
                    scol_part_transform(part),
                    part.scale,
                    &material_swap,
                );
            }
            return;
        }

        // port: ProcessReference — a ref with no LOD model in ANY slot is not LOD'd.
        if base.lod_models.iter().all(Option::is_none) {
            return;
        }
        push_ref_input(
            out,
            record,
            placed,
            &base,
            pos,
            rot,
            scale,
            "",
            super::identity_part_transform(),
            1.0,
            &material_swap,
        );
    }

    fn overlay_resolved_base(
        mut base: ResolvedBase,
        overlay: Option<&OverlayEntry>,
    ) -> (ResolvedBase, bool) {
        let force_visible = overlay.is_some_and(|entry| entry.force_visible);
        if let Some(entry) = overlay {
            base.lod_models = entry.lod_models.clone();
        }
        (base, force_visible)
    }

    fn should_skip_placed_ref(record: &ParsedRecord) -> bool {
        record.flags & (REFR_FLAG_INITIALLY_DISABLED | RECORD_FLAG_DELETED) != 0
    }

    fn should_skip_base(base: &ResolvedBase) -> bool {
        base.record_flags & RECORD_FLAG_DELETED != 0
    }

    /// Return `true` iff `plugin` has a WRLD record whose EDID == `editor_id`
    /// (case-insensitive). Used by `scan_for_wrld_plugin` as a cheap presence
    /// check — only the top WRLD group is visited, no cell/LAND data is read.
    pub fn plugin_contains_wrld(plugin: &ParsedPlugin, editor_id: &str) -> bool {
        find_world(plugin, editor_id).is_some()
    }

    /// Scan every `.esm` and `.esp` in `dir` (sorted lexicographically for
    /// determinism) and return the path of the last plugin that contains a WRLD
    /// record whose editor id == `world_id`.
    ///
    /// **Tie-break caveat**: "alphabetically last in sorted order" is a
    /// simplification — it does NOT reflect real FO4 Plugins.txt load order.
    /// When multiple plugins define the same worldspace, the true load-order
    /// winner is determined by the player's Plugins.txt, which is not consulted
    /// here. In the common single-owner case (one plugin per WRLD) the distinction
    /// is irrelevant; for the FO76→FO4 converted-mod case (APPALACHIA in
    /// SeventySix.esm) the fast-path in `run` hits SeventySix.esm directly and
    /// this scan is never reached.
    ///
    /// Returns `None` if no plugin in the dir contains the worldspace, or if the
    /// dir cannot be read.
    pub fn scan_for_wrld_plugin(
        dir: &std::path::Path,
        world_id: &str,
    ) -> Option<std::path::PathBuf> {
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.is_file()
                    && matches!(
                        p.extension()
                            .and_then(|e| e.to_str())
                            .map(|s| s.to_ascii_lowercase())
                            .as_deref(),
                        Some("esm") | Some("esp")
                    )
            })
            .collect();
        entries.sort();

        let mut winner: Option<std::path::PathBuf> = None;
        for path in &entries {
            let path_str = path.to_string_lossy().into_owned();
            // parse_plugin_file with eager_compressed=false (lazy) — we only need the
            // top-level WRLD group items (records), so a lazy parse is fine and cheap
            // relative to a full enumeration.
            if let Ok(plugin) = parse_plugin_file(&path_str, Some("fo4".to_string()), false)
                .map_err(py_err_to_anyhow)
            {
                if plugin_contains_wrld(&plugin, world_id) {
                    winner = Some(path.clone());
                    // Keep scanning: a later plugin in sorted order is the load-order
                    // winner if multiple plugins define the same WRLD.
                }
            }
        }
        winner
    }

    /// Enumerate placed-object references for object LOD.
    /// port: the xEdit reference walk → `StaticDesc[]` + ProcessReference filtering.
    fn enumerate_refs(handle: &EspHandle, children: &ParsedGroup) -> Vec<RefInput> {
        let mut placed = Vec::new();
        collect_world_refs(children, &mut placed);

        let mut refs = Vec::with_capacity(placed.len());
        for p in &placed {
            append_ref_inputs(handle, p, &mut refs);
        }
        refs
    }

    /// Read the worldspace's declared cell bounds from NAM0 (SW object bounds) and
    /// NAM9 (NE object bounds). Each is two f32 world-unit coordinates; dividing by
    /// the 4096-unit cell size and flooring gives the SW / ceil-ing the NE cell.
    /// xLODGen anchors its LOD quad grid to these declared bounds (which can extend
    /// past the cells that actually carry LAND), so terrain quad names line up.
    /// Returns None if NAM0/NAM9 are absent or malformed.
    pub fn worldspace_declared_cell_bounds(
        wrld: &ParsedRecord,
    ) -> Option<((i32, i32), (i32, i32))> {
        let nam0 = subrecord(wrld, "NAM0")?;
        let nam9 = subrecord(wrld, "NAM9")?;
        if nam0.len() < 8 || nam9.len() < 8 {
            return None;
        }
        let sw_x = f32::from_le_bytes([nam0[0], nam0[1], nam0[2], nam0[3]]);
        let sw_y = f32::from_le_bytes([nam0[4], nam0[5], nam0[6], nam0[7]]);
        let ne_x = f32::from_le_bytes([nam9[0], nam9[1], nam9[2], nam9[3]]);
        let ne_y = f32::from_le_bytes([nam9[4], nam9[5], nam9[6], nam9[7]]);
        let cell = |v: f32| (v / 4096.0).floor() as i32;
        Some(((cell(sw_x), cell(sw_y)), (cell(ne_x), cell(ne_y))))
    }

    /// Read the worldspace's water height (WRLD.DNAM, second f32) and the default
    /// land textures. The default land diffuse/normal come from the worldspace's
    /// own NNAM/... in some games; for FO4 the LOD generator falls back to the
    /// per-cell layers, so defaults stay empty unless a base layer is unresolved.
    fn worldspace_water_height(wrld: &ParsedRecord) -> f32 {
        if let Some(dnam) = subrecord(wrld, "DNAM") {
            if dnam.len() >= 8 {
                let wh = f32::from_le_bytes([dnam[4], dnam[5], dnam[6], dnam[7]]);
                // xLODGen sentinel: water above 2^24 means "no water" (TerrainData.cs:63).
                if wh <= 16_777_216.0 {
                    return wh;
                }
            } else if dnam.len() >= 4 {
                let wh = f32::from_le_bytes([dnam[0], dnam[1], dnam[2], dnam[3]]);
                if wh <= 16_777_216.0 {
                    return wh;
                }
            }
        }
        f32::MIN
    }

    /// Read the worldspace HD-LOD default land textures: WRLD.TNAM = "HD LOD
    /// Diffuse Texture", WRLD.UNAM = "HD LOD Normal Texture" (both zstrings,
    /// Data-relative). These are xLODGen's default land diffuse/normal used to fill
    /// cells that carry no usable layer (R3 §3). Returns empty strings when absent
    /// (converted FO76→FO4 worldspaces typically drop them).
    fn worldspace_default_textures(wrld: &ParsedRecord) -> (String, String) {
        let diffuse = subrecord(wrld, "TNAM").map(zstring).unwrap_or_default();
        let normal = subrecord(wrld, "UNAM").map(zstring).unwrap_or_default();
        (diffuse, normal)
    }

    /// Most frequently referenced layer diffuse/normal across all cells. Used as
    /// the worldspace default base ONLY when the WRLD carries no TNAM/UNAM (the
    /// converted FO76→FO4 case): it gives layerless cells a real, worldspace-local
    /// land texture so terrain LOD stays continuous instead of grey/black.
    fn most_common_layer_textures(cells: &[CellInput]) -> (String, String) {
        use std::collections::HashMap;
        let mut diffuse: HashMap<&str, usize> = HashMap::new();
        let mut normal: HashMap<&str, usize> = HashMap::new();
        for cell in cells {
            for l in &cell.layers {
                if !l.diffuse.is_empty() {
                    *diffuse.entry(l.diffuse.as_str()).or_default() += 1;
                }
                if !l.normal.is_empty() {
                    *normal.entry(l.normal.as_str()).or_default() += 1;
                }
            }
        }
        let pick = |m: HashMap<&str, usize>| {
            m.into_iter()
                .max_by_key(|&(_, n)| n)
                .map(|(s, _)| s.to_string())
                .unwrap_or_default()
        };
        (pick(diffuse), pick(normal))
    }

    /// Per-cell water height (FO4 CELL.XCLW = "Water Height", float32), falling back
    /// to the worldspace `default_water` when the cell has no XCLW.
    ///
    /// xLODGen's terrain `.dat` carries a per-cell `waterHeight` (TerrainData.cs:184)
    /// — the value the water-emit rule (`GenerateWater`, TerrainLOD.cs:984-995) tests
    /// against the cell's terrain floor. The decompiled source reads it from the
    /// `.dat`; the equivalent ESP-side field is CELL.XCLW, which most exterior cells
    /// carry (FarHarbor cells set XCLW even where the worldspace DNAM is 0). Reading
    /// it here lets the water block emit for the real-LAND cells that sit below their
    /// own water level. The sentinel ">2^24 = no water" rule applies identically.
    fn cell_water_height(cell: &ParsedRecord, default_water: f32) -> f32 {
        if let Some(xclw) = subrecord(cell, "XCLW") {
            if xclw.len() >= 4 {
                let wh = f32::from_le_bytes([xclw[0], xclw[1], xclw[2], xclw[3]]);
                if wh <= 16_777_216.0 {
                    // Commonwealth cells commonly carry XCLW=0 as an inherited/default
                    // value; xLODGen's terrain data uses the WRLD water height there.
                    if wh == 0.0 && default_water > 0.0 {
                        return default_water;
                    }
                    return wh;
                }
                // XCLW sentinel "no water" — fall through to the worldspace default.
            }
        }
        default_water
    }

    /// Enumerate the TERRAIN of a worldspace into a `WorldspaceInput`.
    ///
    /// Reads the named WRLD's exterior cells and their LAND records into per-cell
    /// `CellInput`s (heights from VHGT, vertex colors from VCLR, LTEX layers from
    /// BTXT/ATXT base+alpha, hidden quadrants from LAND DATA flags). `refs` is left
    /// EMPTY — object enumeration is not implemented here.
    pub fn enumerate_worldspace(
        handle: &EspHandle,
        world_editor_id: &str,
        settings: &crate::settings::LodSettings,
    ) -> anyhow::Result<WorldspaceInput> {
        let plugin = handle
            .plugin
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("EspHandle has no plugin loaded"))?;

        let wrld = find_world(plugin, world_editor_id)
            .ok_or_else(|| anyhow::anyhow!("worldspace '{world_editor_id}' not found in plugin"))?;
        let world_form_id = wrld.form_id;
        let water_height = worldspace_water_height(wrld);

        let children = find_world_children_group(plugin, world_form_id).ok_or_else(|| {
            anyhow::anyhow!("worldspace '{world_editor_id}' has no World Children group")
        })?;

        let mut cell_lands = Vec::new();
        collect_cell_lands(children, &mut cell_lands);

        if cell_lands.is_empty() {
            anyhow::bail!("worldspace '{world_editor_id}' has no exterior CELL+LAND records");
        }

        let mut cells = Vec::with_capacity(cell_lands.len());
        let mut grass_refs = Vec::new();
        for cl in &cell_lands {
            let (x, y) = cl.grid;
            let heights = match subrecord(cl.land, "VHGT") {
                Some(vhgt) => decode_vhgt_heights(vhgt)?,
                None => vec![0.0f32; GRID * GRID],
            };
            let vertex_colors = match subrecord(cl.land, "VCLR") {
                Some(vclr) => decode_vclr(vclr),
                None => vec![[255u8, 255, 255]; GRID * GRID],
            };
            let (layers, grass_layers) = build_layers(handle, cl.land);
            let hidden_quadrants = subrecord(cl.land, "DATA")
                .filter(|d| d.len() >= 4)
                .map(|d| decode_hidden_quadrants(i32::from_le_bytes([d[0], d[1], d[2], d[3]])))
                .unwrap_or([false; 4]);

            synthesize_grass_refs_for_cell(
                (x, y),
                &heights,
                hidden_quadrants,
                &grass_layers,
                &settings.grass,
                &mut grass_refs,
            );

            // Per-cell water height (CELL.XCLW) with the worldspace DNAM as fallback.
            // The water-emit rule (water.rs / GenerateWater) tests this per cell.
            let cell_water = cell_water_height(cl.cell, water_height);

            cells.push(CellInput {
                x,
                y,
                heights,
                vertex_colors,
                layers,
                hidden_quadrants,
                water_height: cell_water,
            });
        }

        // LAND extent (cells that actually carry terrain).
        let land_sw_x = cells.iter().map(|c| c.x).min().unwrap();
        let land_sw_y = cells.iter().map(|c| c.y).min().unwrap();
        let land_ne_x = cells.iter().map(|c| c.x).max().unwrap();
        let land_ne_y = cells.iter().map(|c| c.y).max().unwrap();

        // xLODGen anchors its quad grid to the worldspace's DECLARED bounds (NAM0/
        // NAM9), which can extend past the LAND extent. Use the declared SW corner as
        // the grid anchor so quad origins (and thus `.btr` filenames) match xLODGen;
        // widen to include any LAND cell that falls outside the declared box.
        let (sw, ne) = match worldspace_declared_cell_bounds(wrld) {
            Some((dsw, dne)) => (
                (dsw.0.min(land_sw_x), dsw.1.min(land_sw_y)),
                (dne.0.max(land_ne_x), dne.1.max(land_ne_y)),
            ),
            None => ((land_sw_x, land_sw_y), (land_ne_x, land_ne_y)),
        };

        let _ = (FO4_CELL_SIZE, world_editor_id);

        // Worldspace default land textures for layerless / sparse cells: prefer the
        // WRLD HD-LOD textures (TNAM/UNAM); when absent (converted FO76→FO4), fall
        // back to the most common per-cell layer texture so terrain LOD stays
        // continuous instead of dropping to a grey/black fallback.
        let (mut default_diffuse, mut default_normal) = worldspace_default_textures(wrld);
        if default_diffuse.is_empty() || default_normal.is_empty() {
            let (cd, cn) = most_common_layer_textures(&cells);
            if default_diffuse.is_empty() {
                default_diffuse = cd;
            }
            if default_normal.is_empty() {
                default_normal = cn;
            }
        }

        // Enumerate placed-object references (REFR) for object LOD. Each REFR
        // whose base record carries a DistantLOD model becomes a RefInput; disabled
        // refs and bases without LOD models are filtered out (ProcessReference).
        let mut refs = enumerate_refs(handle, children);
        refs.extend(grass_refs);

        Ok(WorldspaceInput {
            editor_id: world_editor_id.to_string(),
            sw_cell: sw,
            ne_cell: ne,
            water_height,
            no_lod_water: false,
            default_diffuse,
            default_normal,
            cells,
            refs,
        })
    }
}

#[cfg(feature = "real-esp")]
pub use esp_enum::{
    EspHandle, decode_distant_lod, decode_refr_data, decode_refr_scale, decode_vclr,
    decode_vhgt_heights, decode_vtxt_alpha, discover_worldspaces, enumerate_worldspace,
    plugin_contains_wrld, scan_for_wrld_plugin, worldspace_declared_cell_bounds,
};

// ---------------------------------------------------------------------------
// Stub path (default build, feature `real-esp` off). Keeps `lib.rs` compiling
// without linking esp. The real reader is available via `--features real-esp`
// and inside the umbrella `_native.pyd`.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "real-esp"))]
pub struct EspHandle {
    _private: (),
}

#[cfg(not(feature = "real-esp"))]
impl EspHandle {
    pub fn new() -> Self {
        EspHandle { _private: () }
    }

    pub fn load(_plugin_path: &std::path::Path, _game: &str) -> anyhow::Result<Self> {
        anyhow::bail!(
            "lodgen_native built without the `real-esp` feature: real ESP enumeration unavailable"
        )
    }

    pub fn load_with_overlay(
        _plugin_path: &std::path::Path,
        _game: &str,
        _overlay_path: Option<&std::path::Path>,
    ) -> anyhow::Result<Self> {
        anyhow::bail!(
            "lodgen_native built without the `real-esp` feature: real ESP enumeration unavailable"
        )
    }
}

#[cfg(not(feature = "real-esp"))]
impl Default for EspHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(feature = "real-esp"))]
pub fn enumerate_worldspace(
    _handle: &EspHandle,
    _world_editor_id: &str,
    _settings: &crate::settings::LodSettings,
) -> anyhow::Result<WorldspaceInput> {
    anyhow::bail!(
        "lodgen_native built without the `real-esp` feature: real ESP enumeration unavailable"
    )
}

#[cfg(not(feature = "real-esp"))]
pub fn discover_worldspaces(
    _plugin_path: &std::path::Path,
    _game: &str,
) -> anyhow::Result<Vec<String>> {
    anyhow::bail!(
        "lodgen_native built without the `real-esp` feature: real ESP enumeration unavailable"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(x: i32, y: i32) -> CellInput {
        CellInput {
            x,
            y,
            heights: vec![0.0; 33 * 33],
            vertex_colors: vec![[255, 255, 255]; 33 * 33],
            layers: Vec::new(),
            hidden_quadrants: [false; 4],
            water_height: 0.0,
        }
    }

    #[test]
    fn bounds_from_cells() {
        let w =
            WorldspaceInput::from_cells("TestWorld", vec![cell(-2, -3), cell(5, 4), cell(0, 0)]);
        assert_eq!(w.sw_cell, (-2, -3));
        assert_eq!(w.ne_cell, (5, 4));
    }

    #[test]
    fn hidden_quadrant_bits_decode() {
        // landFlags bit1=SW, bit2=SE, bit4=NW, bit8=NE (R1 §7)
        let q = decode_hidden_quadrants(0b1011); // SW + SE + NE
        assert_eq!(q, [true, true, false, true]);
        let none = decode_hidden_quadrants(0);
        assert_eq!(none, [false; 4]);
    }

    fn grass_layer(max_slope_degrees: f32) -> GrassLayerInput {
        GrassLayerInput {
            quadrant: 0,
            alpha: Vec::new(),
            models: vec![GrassModelInput {
                base_name: "TestGrass".to_string(),
                model: r"Landscape\Grass\TestGrass.nif".to_string(),
                max_slope_degrees,
            }],
        }
    }

    #[test]
    fn grass_refs_are_deterministic_and_level_scoped() {
        let settings = crate::settings::GrassSettings {
            enabled: true,
            spacings: [1024.0, 0.0, 0.0, 0.0],
            min_alpha: 0.35,
        };
        let mut first = Vec::new();
        let mut second = Vec::new();
        synthesize_grass_refs_for_cell(
            (2, -3),
            &vec![100.0; 33 * 33],
            [false; 4],
            &[grass_layer(90.0)],
            &settings,
            &mut first,
        );
        synthesize_grass_refs_for_cell(
            (2, -3),
            &vec![100.0; 33 * 33],
            [false; 4],
            &[grass_layer(90.0)],
            &settings,
            &mut second,
        );

        assert_eq!(first.len(), 4);
        assert_eq!(
            first
                .iter()
                .map(|reference| &reference.ref_id)
                .collect::<Vec<_>>(),
            second
                .iter()
                .map(|reference| &reference.ref_id)
                .collect::<Vec<_>>()
        );
        assert!(first.iter().all(|reference| reference.is_grass));
        assert!(first.iter().all(|reference| reference.pos[2] == 100.0));
        assert!(first.iter().all(|reference| {
            reference.lod_models[0].is_some()
                && reference.lod_models[1..].iter().all(Option::is_none)
        }));
    }

    #[test]
    fn grass_refs_respect_source_slope_limit() {
        let settings = crate::settings::GrassSettings {
            enabled: true,
            spacings: [1024.0, 0.0, 0.0, 0.0],
            min_alpha: 0.35,
        };
        let mut heights = vec![0.0; 33 * 33];
        for y in 0..33 {
            for x in 0..33 {
                heights[x + y * 33] = x as f32 * 512.0;
            }
        }
        let mut refs = Vec::new();
        synthesize_grass_refs_for_cell(
            (0, 0),
            &heights,
            [false; 4],
            &[grass_layer(10.0)],
            &settings,
            &mut refs,
        );

        assert!(refs.is_empty());
    }

    #[cfg(feature = "real-esp")]
    #[test]
    fn object_lod_visibility_accepts_ref_or_base_visible_flag_except_trees() {
        assert!(super::esp_enum::ref_can_emit_object_lod(
            0x0000_8000,
            0,
            false,
            false,
            "STAT"
        ));
        assert!(super::esp_enum::ref_can_emit_object_lod(
            0,
            0x0000_8000,
            false,
            false,
            "STAT"
        ));
        assert!(super::esp_enum::ref_can_emit_object_lod(
            0, 0, true, false, "STAT"
        ));
        assert!(super::esp_enum::ref_can_emit_object_lod(
            0, 0, false, true, "STAT"
        ));
        assert!(!super::esp_enum::ref_can_emit_object_lod(
            0, 0, false, false, "STAT"
        ));
        assert!(super::esp_enum::ref_can_emit_object_lod(
            0, 0, false, false, "TREE"
        ));
    }
}
