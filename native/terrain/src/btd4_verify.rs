//! Bounded-memory verification of a BTD4 v2 sidecar.
//!
//! Reads only the header and the 88-byte index rows up front, then decodes one
//! cell at a time. Edge continuity is checked against the previous index row and
//! the west neighbour, so resident memory is one row of cell edges no matter how
//! large the worldspace is. The same decode rules as `Btd4Reader` and the Tales
//! runtime reader apply; any rejection is an error the runtime would also hit.

use crate::btd4::{ALPH_PLANE_COUNT, ALPH_PLANE_LEN, BTD4_VERSION, Btd4Header};
use flate2::read::ZlibDecoder;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

const CELL_VERTS: usize = 129;
const CELL_VERTEX_COUNT: usize = CELL_VERTS * CELL_VERTS;
const GRASS_MASK_LEN: usize = 128 * 128;
const LAYER_COUNT: usize = 24;
const SAMPLE_SPACING: f32 = 32.0;
const CELL_UNITS: f32 = 4096.0;
const MAX_ERRORS: usize = 64;
const MAX_MISSING_SAMPLES: usize = 32;
/// Coarse FO4 LAND vertices sit on every 4th dense sample (33x33 per cell).
const COARSE_STRIDE: usize = 4;
const DEVIATION_BUCKETS: [f32; 6] = [4.0, 8.0, 16.0, 32.0, 64.0, 128.0];

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct VerifyOptions {
    /// (cell x, cell y, water height), for shoreline test-location selection.
    pub water_heights: Vec<(i32, i32, f32)>,
    /// LTEX object ids whose source material is a road surface.
    pub road_layer_ids: BTreeSet<u32>,
    /// LTEX / GRAS object ids the same conversion wrote; references to the output
    /// plugin (index 0) outside these sets are reported as unresolved.
    pub written_ltex_ids: Option<BTreeSet<u32>>,
    pub written_grass_ids: Option<BTreeSet<u32>>,
    pub locations_per_category: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TestLocation {
    pub cell_x: i32,
    pub cell_y: i32,
    pub world_x: f32,
    pub world_y: f32,
    pub world_z: f32,
    pub metric: f32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TestLocations {
    pub flat: Vec<TestLocation>,
    pub steep: Vec<TestLocation>,
    pub shoreline: Vec<TestLocation>,
    pub road: Vec<TestLocation>,
    pub grass: Vec<TestLocation>,
    pub collision: Vec<TestLocation>,
    pub cell_border: Vec<TestLocation>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Btd4VerifyReport {
    pub path: String,
    pub worldspace_editor_id: String,
    pub plugin_names: Vec<String>,
    pub file_bytes: u64,
    pub cell_bounds: [i32; 4],
    pub cells: usize,
    /// Cells inside the header bounds with no entry. The runtime keeps vanilla
    /// LAND for these.
    pub fallback_cells: usize,
    pub fallback_cell_samples: Vec<(i32, i32)>,
    pub cells_with_alpha: usize,
    pub cells_with_grass: usize,
    pub cells_with_colors: usize,
    pub height_min_world: f32,
    pub height_max_world: f32,
    pub edges_checked: usize,
    pub height_edge_mismatches: usize,
    pub color_edge_mismatches: usize,
    pub max_height_edge_delta: f32,
    /// Max |dense - bilinear(33x33 coarse anchors)| per cell, bucketed at
    /// 4/8/16/32/64/128 world units (last bucket = above 128).
    pub coarse_deviation_histogram: [usize; 7],
    pub max_coarse_deviation: f32,
    pub referenced_ltex_ids: Vec<String>,
    pub referenced_grass_ids: Vec<String>,
    pub unresolved_ltex_ids: Vec<String>,
    pub unresolved_grass_ids: Vec<String>,
    pub grass_intervals_over_16_types: usize,
    pub errors: Vec<String>,
    pub error_count: usize,
    pub test_locations: TestLocations,
}

impl Btd4VerifyReport {
    pub fn is_valid(&self) -> bool {
        self.error_count == 0
            && self.height_edge_mismatches == 0
            && self.color_edge_mismatches == 0
            && self.unresolved_ltex_ids.is_empty()
            && self.unresolved_grass_ids.is_empty()
            && self.grass_intervals_over_16_types == 0
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ChannelEntry {
    offset: u64,
    len: u64,
}

struct IndexRow {
    x: i32,
    y: i32,
    channels: [ChannelEntry; 5],
}

struct DecodedCell {
    heights: Vec<u16>,
    alphas: Option<Vec<u8>>,
    layers: Option<Vec<(u8, u32)>>,
    grass: Option<Vec<(u8, u32, Vec<u8>)>>,
    colors: Option<Vec<u8>>,
}

struct Edges {
    heights_top: Vec<u16>,
    heights_right: Vec<u16>,
    colors_top: Option<Vec<u8>>,
    colors_right: Option<Vec<u8>>,
}

fn read_exact<R: Read>(reader: &mut R, n: usize) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0u8; n];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| format!("btd4 verify: truncated header/index: {e}"))?;
    Ok(bytes)
}

fn read_u16<R: Read>(reader: &mut R) -> Result<u16, String> {
    let b = read_exact(reader, 2)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn read_u32<R: Read>(reader: &mut R) -> Result<u32, String> {
    let b = read_exact(reader, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_i32<R: Read>(reader: &mut R) -> Result<i32, String> {
    Ok(read_u32(reader)? as i32)
}

fn read_u64<R: Read>(reader: &mut R) -> Result<u64, String> {
    let b = read_exact(reader, 8)?;
    Ok(u64::from_le_bytes(b.try_into().unwrap()))
}

fn read_f32<R: Read>(reader: &mut R) -> Result<f32, String> {
    Ok(f32::from_bits(read_u32(reader)?))
}

fn read_string<R: Read>(reader: &mut R) -> Result<String, String> {
    let len = read_u16(reader)? as usize;
    String::from_utf8(read_exact(reader, len)?).map_err(|e| format!("btd4 verify: string: {e}"))
}

fn read_head(path: &Path) -> Result<(Btd4Header, Vec<IndexRow>, u64), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("btd4 verify: open: {e}"))?;
    let file_len = file
        .metadata()
        .map_err(|e| format!("btd4 verify: metadata: {e}"))?
        .len();
    let mut reader = BufReader::new(file);
    if read_exact(&mut reader, 4)? != b"BTD4" {
        return Err("btd4 verify: bad magic".into());
    }
    let version = read_u32(&mut reader)?;
    if version != BTD4_VERSION {
        return Err(format!("btd4 verify: unsupported version {version}"));
    }
    let density = read_u32(&mut reader)?;
    let height_min = read_f32(&mut reader)?;
    let height_scale = read_f32(&mut reader)?;
    let worldspace_editor_id = read_string(&mut reader)?;
    if density != 128
        || !height_min.is_finite()
        || !height_scale.is_finite()
        || height_scale <= 0.0
        || worldspace_editor_id.is_empty()
    {
        return Err("btd4 verify: invalid v2 header".into());
    }
    let plugin_count = read_u16(&mut reader)? as usize;
    if plugin_count == 0 {
        return Err("btd4 verify: empty plugin table".into());
    }
    let mut plugin_names = Vec::with_capacity(plugin_count);
    for _ in 0..plugin_count {
        let name = read_string(&mut reader)?;
        if name.is_empty() {
            return Err("btd4 verify: empty plugin name".into());
        }
        plugin_names.push(name);
    }
    let cell_min_x = read_i32(&mut reader)?;
    let cell_min_y = read_i32(&mut reader)?;
    let cell_max_x = read_i32(&mut reader)?;
    let cell_max_y = read_i32(&mut reader)?;
    let cell_count = read_u32(&mut reader)? as usize;
    if cell_min_x > cell_max_x || cell_min_y > cell_max_y || cell_count > 1_000_000 {
        return Err("btd4 verify: invalid cell bounds or count".into());
    }
    let mut rows = Vec::with_capacity(cell_count);
    let mut previous: Option<(i32, i32)> = None;
    for _ in 0..cell_count {
        let x = read_i32(&mut reader)?;
        let y = read_i32(&mut reader)?;
        if x < cell_min_x || x > cell_max_x || y < cell_min_y || y > cell_max_y {
            return Err(format!("btd4 verify: cell ({x},{y}) outside header bounds"));
        }
        if previous.is_some_and(|p| p >= (y, x)) {
            return Err(format!(
                "btd4 verify: index not strictly ascending by (y,x) at ({x},{y})"
            ));
        }
        previous = Some((y, x));
        let mut channels = [ChannelEntry::default(); 5];
        for channel in &mut channels {
            channel.offset = read_u64(&mut reader)?;
            channel.len = read_u64(&mut reader)?;
            if channel.len != 0
                && channel
                    .offset
                    .checked_add(channel.len)
                    .is_none_or(|end| end > file_len)
            {
                return Err(format!(
                    "btd4 verify: cell ({x},{y}) channel range out of file"
                ));
            }
        }
        if channels[0].len == 0 {
            return Err(format!("btd4 verify: cell ({x},{y}) has no HGTS"));
        }
        rows.push(IndexRow { x, y, channels });
    }
    let header = Btd4Header {
        version,
        density,
        height_min,
        height_scale,
        worldspace_editor_id,
        plugin_names,
        cell_min_x,
        cell_min_y,
        cell_max_x,
        cell_max_y,
    };
    Ok((header, rows, file_len))
}

fn inflate<R: Read + Seek>(
    file: &mut R,
    entry: ChannelEntry,
    expected: Option<usize>,
    maximum: usize,
) -> Result<Vec<u8>, String> {
    file.seek(SeekFrom::Start(entry.offset))
        .map_err(|e| format!("seek: {e}"))?;
    let mut compressed = vec![0u8; entry.len as usize];
    file.read_exact(&mut compressed)
        .map_err(|e| format!("read chunk: {e}"))?;
    let mut out = Vec::with_capacity(expected.unwrap_or(0));
    ZlibDecoder::new(compressed.as_slice())
        .take(maximum as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("inflate: {e}"))?;
    if out.len() > maximum || expected.is_some_and(|size| out.len() != size) {
        return Err(format!("decoded size {} is wrong", out.len()));
    }
    Ok(out)
}

fn decode_cell<R: Read + Seek>(
    file: &mut R,
    row: &IndexRow,
    plugin_count: usize,
) -> Result<DecodedCell, String> {
    let raw = inflate(file, row.channels[0], Some(CELL_VERTEX_COUNT * 2), CELL_VERTEX_COUNT * 2)
        .map_err(|e| format!("HGTS: {e}"))?;
    let heights = raw
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();

    let alphas = if row.channels[1].len == 0 {
        None
    } else {
        let size = 1 + ALPH_PLANE_COUNT * ALPH_PLANE_LEN;
        let raw = inflate(file, row.channels[1], Some(size), size).map_err(|e| format!("ALPH: {e}"))?;
        if raw[0] as usize != ALPH_PLANE_COUNT {
            return Err("ALPH: plane count is not 20".into());
        }
        if row.channels[2].len == 0 {
            return Err("ALPH present without LAYR".into());
        }
        Some(raw[1..].to_vec())
    };

    let layers = if row.channels[2].len == 0 {
        None
    } else {
        let size = 1 + LAYER_COUNT * 6;
        let raw = inflate(file, row.channels[2], Some(size), size).map_err(|e| format!("LAYR: {e}"))?;
        if raw[0] as usize != LAYER_COUNT {
            return Err("LAYR: row count is not 24".into());
        }
        let mut layers = Vec::with_capacity(LAYER_COUNT);
        for slot in 0..LAYER_COUNT {
            let base = 1 + slot * 6;
            let plugin = raw[base];
            let object_id = u32::from_le_bytes(raw[base + 1..base + 5].try_into().unwrap());
            let empty = plugin == u8::MAX && object_id == 0;
            if !empty && (plugin as usize >= plugin_count || object_id == 0 || raw[base + 5] != 0) {
                return Err(format!("LAYR: slot {slot} is invalid"));
            }
            layers.push((plugin, object_id));
        }
        Some(layers)
    };

    let grass = if row.channels[3].len == 0 {
        None
    } else {
        let entry_size = 5 + GRASS_MASK_LEN;
        let raw = inflate(file, row.channels[3], None, 1 + 255 * entry_size)
            .map_err(|e| format!("GCVR: {e}"))?;
        let count = *raw.first().ok_or("GCVR: empty")? as usize;
        if raw.len() != 1 + count * entry_size {
            return Err("GCVR: length does not match its form count".into());
        }
        let mut grass = Vec::with_capacity(count);
        for index in 0..count {
            let base = 1 + index * entry_size;
            let plugin = raw[base];
            let object_id = u32::from_le_bytes(raw[base + 1..base + 5].try_into().unwrap());
            if plugin as usize >= plugin_count || object_id == 0 {
                return Err(format!("GCVR: entry {index} is invalid"));
            }
            grass.push((plugin, object_id, raw[base + 5..base + entry_size].to_vec()));
        }
        Some(grass)
    };

    let colors = if row.channels[4].len == 0 {
        None
    } else {
        let size = CELL_VERTEX_COUNT * 3;
        Some(inflate(file, row.channels[4], Some(size), size).map_err(|e| format!("CLRS: {e}"))?)
    };

    Ok(DecodedCell {
        heights,
        alphas,
        layers,
        grass,
        colors,
    })
}

fn world_height(header: &Btd4Header, raw: u16) -> f32 {
    header.height_min + f32::from(raw) * header.height_scale
}

/// Max |dense - bilinear(coarse anchors)|: how far the dense visual surface can
/// leave the 33x33 surface FO4 collision, feet and AI height queries use.
pub fn coarse_deviation(heights: &[f32]) -> f32 {
    let mut maximum = 0.0f32;
    for y in 0..CELL_VERTS {
        let cy = (y / COARSE_STRIDE).min(31);
        let fy = (y - cy * COARSE_STRIDE) as f32 / COARSE_STRIDE as f32;
        for x in 0..CELL_VERTS {
            let cx = (x / COARSE_STRIDE).min(31);
            let fx = (x - cx * COARSE_STRIDE) as f32 / COARSE_STRIDE as f32;
            let at = |ax: usize, ay: usize| heights[ay * COARSE_STRIDE * CELL_VERTS + ax * COARSE_STRIDE];
            let bottom = at(cx, cy) * (1.0 - fx) + at(cx + 1, cy) * fx;
            let top = at(cx, cy + 1) * (1.0 - fx) + at(cx + 1, cy + 1) * fx;
            let coarse = bottom * (1.0 - fy) + top * fy;
            maximum = maximum.max((heights[y * CELL_VERTS + x] - coarse).abs());
        }
    }
    maximum
}

/// Largest slope between adjacent dense samples, in degrees.
pub fn max_slope_degrees(heights: &[f32]) -> f32 {
    let mut steepest = 0.0f32;
    for y in 0..CELL_VERTS {
        for x in 0..CELL_VERTS {
            let h = heights[y * CELL_VERTS + x];
            if x + 1 < CELL_VERTS {
                steepest = steepest.max((heights[y * CELL_VERTS + x + 1] - h).abs());
            }
            if y + 1 < CELL_VERTS {
                steepest = steepest.max((heights[(y + 1) * CELL_VERTS + x] - h).abs());
            }
        }
    }
    (steepest / SAMPLE_SPACING).atan().to_degrees()
}

/// Worst per-interval GRAS count; FO4's grass builder takes at most 16 types.
fn max_grass_types_per_interval(grass: &[(u8, u32, Vec<u8>)]) -> usize {
    let mut worst = 0;
    for iy in 0..32 {
        for ix in 0..32 {
            let covering = grass
                .iter()
                .filter(|(_, _, mask)| {
                    (iy * 4..iy * 4 + 4).any(|y| (ix * 4..ix * 4 + 4).any(|x| mask[y * 128 + x] != 0))
                })
                .count();
            worst = worst.max(covering);
        }
    }
    worst
}

fn road_fraction(cell: &DecodedCell, road: &BTreeSet<u32>) -> f32 {
    let (Some(layers), false) = (&cell.layers, road.is_empty()) else {
        return 0.0;
    };
    let mut covered = 0usize;
    for quadrant in 0..4 {
        let is_road = |slot: usize| road.contains(&layers[quadrant * 6 + slot].1);
        for texel in 0..ALPH_PLANE_LEN {
            let mut sum = 0u32;
            let mut on_road = false;
            if let Some(alphas) = &cell.alphas {
                for slot in 0..5 {
                    let value = alphas[(quadrant * 5 + slot) * ALPH_PLANE_LEN + texel];
                    sum += u32::from(value);
                    on_road |= value > 127 && is_road(slot + 1);
                }
            }
            on_road |= sum < 128 && is_road(0);
            covered += usize::from(on_road);
        }
    }
    covered as f32 / (4 * ALPH_PLANE_LEN) as f32
}

/// Keeps the `limit` best entries by (metric, then cell y/x for stable ties).
struct Best {
    limit: usize,
    highest: bool,
    items: Vec<TestLocation>,
}

impl Best {
    fn new(limit: usize, highest: bool) -> Self {
        Self {
            limit,
            highest,
            items: Vec::new(),
        }
    }

    fn offer(&mut self, location: TestLocation) {
        if !location.metric.is_finite() {
            return;
        }
        self.items.push(location);
        let highest = self.highest;
        self.items.sort_by(|a, b| {
            let order = a.metric.total_cmp(&b.metric);
            let order = if highest { order.reverse() } else { order };
            order.then((a.cell_y, a.cell_x).cmp(&(b.cell_y, b.cell_x)))
        });
        self.items.truncate(self.limit);
    }
}

fn location(cell_x: i32, cell_y: i32, heights: &[f32], sx: usize, sy: usize, metric: f32) -> TestLocation {
    TestLocation {
        cell_x,
        cell_y,
        world_x: cell_x as f32 * CELL_UNITS + sx as f32 * SAMPLE_SPACING,
        world_y: cell_y as f32 * CELL_UNITS + sy as f32 * SAMPLE_SPACING,
        world_z: heights[sy * CELL_VERTS + sx],
        metric,
    }
}

fn hex_ids(ids: &BTreeSet<u32>) -> Vec<String> {
    ids.iter().map(|id| format!("{id:06X}")).collect()
}

pub fn verify_btd4(path: &Path, options: &VerifyOptions) -> Result<Btd4VerifyReport, String> {
    let (header, rows, file_len) = read_head(path)?;
    let plugin_count = header.plugin_names.len();
    let limit = options.locations_per_category.max(1);
    let water_heights: HashMap<(i32, i32), f32> =
        options.water_heights.iter().map(|&(x, y, h)| ((x, y), h)).collect();
    let mut report = Btd4VerifyReport {
        path: path.display().to_string(),
        worldspace_editor_id: header.worldspace_editor_id.clone(),
        plugin_names: header.plugin_names.clone(),
        file_bytes: file_len,
        cell_bounds: [
            header.cell_min_x,
            header.cell_min_y,
            header.cell_max_x,
            header.cell_max_y,
        ],
        cells: rows.len(),
        height_min_world: f32::INFINITY,
        height_max_world: f32::NEG_INFINITY,
        ..Default::default()
    };

    let span_x = i64::from(header.cell_max_x) - i64::from(header.cell_min_x) + 1;
    let span_y = i64::from(header.cell_max_y) - i64::from(header.cell_min_y) + 1;
    report.fallback_cells = (span_x * span_y) as usize - rows.len();
    {
        let present: std::collections::HashSet<(i32, i32)> = rows.iter().map(|r| (r.x, r.y)).collect();
        'outer: for y in header.cell_min_y..=header.cell_max_y {
            for x in header.cell_min_x..=header.cell_max_x {
                if report.fallback_cell_samples.len() >= MAX_MISSING_SAMPLES {
                    break 'outer;
                }
                if !present.contains(&(x, y)) {
                    report.fallback_cell_samples.push((x, y));
                }
            }
        }
    }

    let mut file = BufReader::with_capacity(
        1 << 16,
        std::fs::File::open(path).map_err(|e| format!("btd4 verify: open: {e}"))?,
    );
    let mut ltex_ids = BTreeSet::new();
    let mut grass_ids = BTreeSet::new();
    let mut own_grass_ids = BTreeSet::new();
    let mut previous_row: HashMap<i32, Edges> = HashMap::new();
    let mut current_row: HashMap<i32, Edges> = HashMap::new();
    let mut row_y: Option<i32> = None;
    let mut west: Option<(i32, i32)> = None;

    let mut flat = Best::new(limit, false);
    let mut steep = Best::new(limit, true);
    let mut shoreline = Best::new(limit, false);
    let mut road = Best::new(limit, true);
    let mut grass_best = Best::new(limit, true);
    let mut collision = Best::new(limit, true);
    let mut border = Best::new(limit, true);

    for row in &rows {
        if row_y != Some(row.y) {
            if row_y == Some(row.y - 1) {
                previous_row = std::mem::take(&mut current_row);
            } else {
                previous_row.clear();
                current_row.clear();
            }
            row_y = Some(row.y);
            west = None;
        }
        let cell = match decode_cell(&mut file, row, plugin_count) {
            Ok(cell) => cell,
            Err(error) => {
                report.error_count += 1;
                if report.errors.len() < MAX_ERRORS {
                    report.errors.push(format!("cell ({},{}): {error}", row.x, row.y));
                }
                west = None;
                continue;
            }
        };

        let heights: Vec<f32> = cell.heights.iter().map(|&h| world_height(&header, h)).collect();
        for &h in &heights {
            report.height_min_world = report.height_min_world.min(h);
            report.height_max_world = report.height_max_world.max(h);
        }

        let edges = Edges {
            heights_top: cell.heights[(CELL_VERTS - 1) * CELL_VERTS..].to_vec(),
            heights_right: (0..CELL_VERTS).map(|y| cell.heights[y * CELL_VERTS + CELL_VERTS - 1]).collect(),
            colors_top: cell
                .colors
                .as_ref()
                .map(|c| c[(CELL_VERTS - 1) * CELL_VERTS * 3..].to_vec()),
            colors_right: cell.colors.as_ref().map(|c| {
                (0..CELL_VERTS)
                    .flat_map(|y| c[(y * CELL_VERTS + CELL_VERTS - 1) * 3..][..3].to_vec())
                    .collect()
            }),
        };

        let compare_edge = |neighbour_heights: &[u16],
                                neighbour_colors: Option<&Vec<u8>>,
                                own_heights: Vec<u16>,
                                own_colors: Option<Vec<u8>>,
                                report: &mut Btd4VerifyReport| {
            report.edges_checked += 1;
            let delta = neighbour_heights
                .iter()
                .zip(&own_heights)
                .map(|(a, b)| (f32::from(*a) - f32::from(*b)).abs() * header.height_scale)
                .fold(0.0f32, f32::max);
            report.max_height_edge_delta = report.max_height_edge_delta.max(delta);
            report.height_edge_mismatches += usize::from(neighbour_heights != own_heights.as_slice());
            if let (Some(a), Some(b)) = (neighbour_colors, own_colors) {
                report.color_edge_mismatches += usize::from(*a != b);
            }
        };

        if west == Some((row.x - 1, row.y)) {
            if let Some(neighbour) = current_row.get(&(row.x - 1)) {
                let own: Vec<u16> = (0..CELL_VERTS).map(|y| cell.heights[y * CELL_VERTS]).collect();
                let own_colors = cell.colors.as_ref().map(|c| {
                    (0..CELL_VERTS)
                        .flat_map(|y| c[y * CELL_VERTS * 3..][..3].to_vec())
                        .collect::<Vec<u8>>()
                });
                compare_edge(
                    &neighbour.heights_right,
                    neighbour.colors_right.as_ref(),
                    own,
                    own_colors,
                    &mut report,
                );
            }
        }
        if let Some(neighbour) = previous_row.get(&row.x) {
            let own = cell.heights[..CELL_VERTS].to_vec();
            let own_colors = cell.colors.as_ref().map(|c| c[..CELL_VERTS * 3].to_vec());
            compare_edge(
                &neighbour.heights_top,
                neighbour.colors_top.as_ref(),
                own,
                own_colors,
                &mut report,
            );
            let relief = (0..CELL_VERTS)
                .map(|x| heights[x])
                .fold(f32::NEG_INFINITY, f32::max)
                - (0..CELL_VERTS).map(|x| heights[x]).fold(f32::INFINITY, f32::min);
            border.offer(location(row.x, row.y, &heights, 64, 0, relief));
        }
        current_row.insert(row.x, edges);
        west = Some((row.x, row.y));

        let deviation = coarse_deviation(&heights);
        report.max_coarse_deviation = report.max_coarse_deviation.max(deviation);
        let bucket = DEVIATION_BUCKETS
            .iter()
            .position(|&limit| deviation <= limit)
            .unwrap_or(DEVIATION_BUCKETS.len());
        report.coarse_deviation_histogram[bucket] += 1;
        let slope = max_slope_degrees(&heights);
        flat.offer(location(row.x, row.y, &heights, 64, 64, slope));
        steep.offer(location(row.x, row.y, &heights, 64, 64, slope));
        collision.offer(location(row.x, row.y, &heights, 64, 64, deviation));

        if let Some(&water) = water_heights.get(&(row.x, row.y)) {
            let below = heights.iter().filter(|&&h| h < water).count() as f32 / heights.len() as f32;
            if below > 0.0 && below < 1.0 {
                shoreline.offer(location(row.x, row.y, &heights, 64, 64, (below - 0.5).abs()));
            }
        }
        let road_share = road_fraction(&cell, &options.road_layer_ids);
        if road_share > 0.0 {
            road.offer(location(row.x, row.y, &heights, 64, 64, road_share));
        }

        report.cells_with_alpha += usize::from(cell.alphas.is_some());
        report.cells_with_colors += usize::from(cell.colors.is_some());
        if let Some(layers) = &cell.layers {
            ltex_ids.extend(layers.iter().filter(|(_, id)| *id != 0).map(|(_, id)| *id));
        }
        if let Some(grass) = &cell.grass {
            report.cells_with_grass += 1;
            grass_ids.extend(grass.iter().map(|(_, id, _)| *id));
            own_grass_ids.extend(grass.iter().filter(|(plugin, _, _)| *plugin == 0).map(|(_, id, _)| *id));
            if max_grass_types_per_interval(grass) > 16 {
                report.grass_intervals_over_16_types += 1;
            }
            let covered = grass
                .iter()
                .map(|(_, _, mask)| mask.iter().filter(|&&v| v != 0).count())
                .sum::<usize>() as f32
                / GRASS_MASK_LEN as f32;
            grass_best.offer(location(row.x, row.y, &heights, 64, 64, covered));
        }
    }

    if report.cells == 0 {
        report.height_min_world = 0.0;
        report.height_max_world = 0.0;
    }
    if let Some(written) = &options.written_ltex_ids {
        report.unresolved_ltex_ids = hex_ids(&ltex_ids.difference(written).copied().collect());
    }
    if let Some(written) = &options.written_grass_ids {
        report.unresolved_grass_ids = hex_ids(&own_grass_ids.difference(written).copied().collect());
    }
    report.referenced_ltex_ids = hex_ids(&ltex_ids);
    report.referenced_grass_ids = hex_ids(&grass_ids);
    report.test_locations = TestLocations {
        flat: flat.items,
        steep: steep.items,
        shoreline: shoreline.items,
        road: road.items,
        grass: grass_best.items,
        collision: collision.items,
        cell_border: border.items,
    };
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btd4::{Btd4Writer, CellChannels, GcvrChunk, GcvrEntry, LayerRef, spill_path_for};

    fn header(max_x: i32, max_y: i32) -> Btd4Header {
        Btd4Header {
            version: BTD4_VERSION,
            density: 128,
            height_min: -1000.0,
            height_scale: 0.5,
            worldspace_editor_id: "B21TestWorld".into(),
            plugin_names: vec!["SeventySix.esm".into()],
            cell_min_x: 0,
            cell_min_y: 0,
            cell_max_x: max_x,
            cell_max_y: max_y,
        }
    }

    /// A continuous global height field sampled per cell, so shared edges agree.
    fn cell_heights(cx: i32, cy: i32, f: impl Fn(i64, i64) -> u16) -> Vec<u16> {
        let mut heights = Vec::with_capacity(CELL_VERTEX_COUNT);
        for y in 0..CELL_VERTS as i64 {
            for x in 0..CELL_VERTS as i64 {
                heights.push(f(i64::from(cx) * 128 + x, i64::from(cy) * 128 + y));
            }
        }
        heights
    }

    fn channels(heights: Vec<u16>) -> CellChannels {
        CellChannels {
            heights: Some(heights),
            alphas: None,
            layers: None,
            gcvr: None,
            colors: None,
        }
    }

    #[test]
    fn continuous_streamed_file_verifies_clean() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Terrain").join("B21TestWorld.btd4");
        let spill = spill_path_for(&path);
        let mut writer = Btd4Writer::with_spill_file(header(2, 1), &spill).unwrap();
        let slope = |x: i64, y: i64| (2000 + x * 3 + y) as u16;
        for cy in 0..=1 {
            for cx in 0..=2 {
                if (cx, cy) == (2, 1) {
                    continue;
                }
                let mut cell = channels(cell_heights(cx, cy, slope));
                if (cx, cy) == (0, 0) {
                    let mut layers = vec![LayerRef { plugin_index: u8::MAX, object_id: 0, kind: 0 }; 24];
                    layers[0] = LayerRef { plugin_index: 0, object_id: 0x0101, kind: 0 };
                    cell.layers = Some(layers);
                    let mut mask = vec![0u8; GRASS_MASK_LEN];
                    mask[0] = 255;
                    cell.gcvr = Some(GcvrChunk {
                        entries: vec![GcvrEntry { plugin_index: 0, object_id: 0x0202, mask }],
                    });
                }
                writer.add_cell(cx, cy, cell).unwrap();
            }
        }
        writer.finish(&path).unwrap();
        assert!(!spill.exists(), "spill file must be removed after finish");

        let options = VerifyOptions {
            written_ltex_ids: Some([0x0101].into()),
            written_grass_ids: Some([0x0202].into()),
            locations_per_category: 3,
            ..Default::default()
        };
        let report = verify_btd4(&path, &options).unwrap();
        assert!(report.is_valid(), "{report:?}");
        assert_eq!(report.cells, 5);
        assert_eq!(report.fallback_cells, 1);
        assert_eq!(report.fallback_cell_samples, vec![(2, 1)]);
        // 2 west/east pairs in row 0, 1 in row 1, 2 south/north pairs.
        assert_eq!(report.edges_checked, 5);
        assert_eq!(report.referenced_ltex_ids, vec!["000101"]);
        assert_eq!(report.referenced_grass_ids, vec!["000202"]);
        assert_eq!(report.cells_with_grass, 1);
        // A planar field has no deviation from its own bilinear anchors.
        assert_eq!(report.max_coarse_deviation, 0.0);
        assert_eq!(report.test_locations.grass[0].cell_x, 0);
    }

    #[test]
    fn edge_mismatch_and_unresolved_references_fail_verification() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.btd4");
        let mut writer = Btd4Writer::new(header(1, 0));
        writer.add_cell(0, 0, channels(vec![100; CELL_VERTEX_COUNT])).unwrap();
        let mut gcvr_cell = channels(vec![101; CELL_VERTEX_COUNT]);
        gcvr_cell.gcvr = Some(GcvrChunk {
            entries: vec![GcvrEntry { plugin_index: 0, object_id: 0x0999, mask: vec![1; GRASS_MASK_LEN] }],
        });
        writer.add_cell(1, 0, gcvr_cell).unwrap();
        writer.finish(&path).unwrap();

        let options = VerifyOptions {
            written_grass_ids: Some(BTreeSet::new()),
            ..Default::default()
        };
        let report = verify_btd4(&path, &options).unwrap();
        assert_eq!(report.height_edge_mismatches, 1);
        assert_eq!(report.max_height_edge_delta, 0.5);
        assert_eq!(report.unresolved_grass_ids, vec!["000999"]);
        assert!(!report.is_valid());
    }

    #[test]
    fn coarse_deviation_measures_detail_between_lattice_vertices() {
        let mut heights = vec![0.0f32; CELL_VERTEX_COUNT];
        heights[2 * CELL_VERTS + 2] = 24.0; // midway between coarse vertices
        assert_eq!(coarse_deviation(&heights), 24.0);
        heights[2 * CELL_VERTS + 2] = 0.0;
        // A spike on a coarse vertex spreads over the coarse surface's whole
        // neighbourhood while the dense surface stays flat next to it.
        heights[4 * CELL_VERTS + 4] = 24.0;
        assert_eq!(coarse_deviation(&heights), 18.0);
    }
}
