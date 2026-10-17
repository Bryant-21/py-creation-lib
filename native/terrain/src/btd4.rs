use flate2::Compression;
use flate2::write::ZlibEncoder;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

/// Per-cell channels in index order: HGTS, ALPH, LAYR, GCVR, CLRS, RAWT, GRSP. Layout:
/// `docs/BTD4.md`.
const CHANNEL_COUNT: usize = 7;

// ---------------------------------------------------------------------------
// Public data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Btd4Header {
    pub density: u32,
    pub height_min: f32,
    pub height_scale: f32,
    pub worldspace_editor_id: String,
    pub plugin_names: Vec<String>,
    pub cell_min_x: i32,
    pub cell_min_y: i32,
    pub cell_max_x: i32,
    pub cell_max_y: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerRef {
    pub plugin_index: u8,
    pub object_id: u32,
    pub kind: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GcvrEntry {
    pub plugin_index: u8,
    pub object_id: u32,
    pub mask: Vec<u8>, // exactly 128*128
}

#[derive(Debug, Clone, PartialEq)]
pub struct GcvrChunk {
    pub entries: Vec<GcvrEntry>,
}

/// ALPH stores seven final-coverage alpha planes per quadrant in LAYR order.
/// The base weight is 255 minus their sum.
/// These are final coverage bytes, unlike sequential native LAND VTXT opacities.
pub const ALPH_PLANE_VERTS: usize = 65;
pub const ALPH_PLANE_LEN: usize = ALPH_PLANE_VERTS * ALPH_PLANE_VERTS;
pub const LAYERS_PER_QUADRANT: usize = 8;
pub const LAYER_COUNT: usize = 4 * LAYERS_PER_QUADRANT;
pub const ALPH_PLANE_COUNT: usize = 4 * (LAYERS_PER_QUADRANT - 1);

#[derive(Debug, Clone)]
pub struct CellChannels {
    pub raw_terrain: Option<Vec<u8>>,
    pub heights: Option<Vec<u16>>,    // 129*129 = 16641
    pub alphas: Option<Vec<Vec<u8>>>, // ALPH_PLANE_COUNT × ALPH_PLANE_LEN (see above)
    pub layers: Option<Vec<LayerRef>>,
    pub gcvr: Option<GcvrChunk>,
    pub colors: Option<Vec<u8>>, // 129*129*3 = 49923
    /// `None` for a cell where FO76 places no ground-cover grass.
    pub grass_points: Option<GrassPointsChunk>,
}

/// GRSP channel: FO76 ground-cover grass points for one FO4 cell, precomputed by
/// `fo76_grass` (FO76 `AddCellGrass` → `CreateGrassPoissonDiskSample`). Byte layout,
/// little-endian, zlib-compressed like every other channel:
///
/// ```text
/// u8   grass_count                          1..=255
/// grass_count x 41 bytes (grass table):
///   u8  plugin_index                        index into the header plugin table
///   u32 object_id                           non-zero, plugin-local
///   u8  density, u8 min_slope, u8 max_slope, u8 flags          (raw FO76 GRAS DNAM)
///   u16 water_above, u16 water_below        0xFFFF = none
///   f32 height_range, color_range, wave_period, freq_base, sharp, cover, lod_blend
/// u16  run_count                            0..=6144 (256 blocks x 24 params)
/// run_count x run, strictly ascending by (block_x, block_y, param_index):
///   u8  block_x, u8 block_y                 0..=15, 256x256-unit blocks of the cell
///   u8  param_index                         0..=23, FO76 param index k (seed term)
///   u8  grass                               index into the grass table
///   i32 seed_after                          BSRandom seed after Poisson sampling
///   u16 point_count                         1..=MAX_RUN_POINTS
///   point_count x (i16 x, i16 y)            block-local, 1/16 unit: -512..=4096
/// ```
///
/// A point's cell-local position is `block * 256 + value / 16`. The seeding point can
/// sit up to one sample (32 units) before its block, hence the negative range. Full
/// reader contract: `docs/BTD4.md`.
#[derive(Debug, Clone, PartialEq)]
pub struct GrassPointsChunk {
    pub grasses: Vec<GrassPointsGrass>,
    pub runs: Vec<GrassPointsRun>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GrassPointsGrass {
    pub plugin_index: u8,
    pub object_id: u32,
    pub dnam: crate::fo76_grass::GrassDnam,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GrassPointsRun {
    pub block_x: u8,
    pub block_y: u8,
    pub param_index: u8,
    pub grass: u8,
    pub seed_after: i32,
    pub points: Vec<[i16; 2]>,
}

pub const GRASS_POINT_UNITS: f32 = 16.0;
pub const GRASS_POINT_MIN: i16 = -512;
pub const GRASS_POINT_MAX: i16 = 4096;
pub const MAX_RUN_POINTS: usize = 4096;
pub const MAX_CELL_GRASS_POINTS: usize = 1 << 20;
const GRASS_TABLE_ENTRY_BYTES: usize = 41;
const GRASS_RUN_HEADER_BYTES: usize = 10;
const MAX_GRASS_RUNS: usize = 16 * 16 * crate::fo76_grass::MAX_PARAMS_PER_BLOCK;
pub(crate) const MAX_GRASS_POINTS_BYTES: usize = 1
    + 255 * GRASS_TABLE_ENTRY_BYTES
    + 2
    + MAX_GRASS_RUNS * GRASS_RUN_HEADER_BYTES
    + 4 * MAX_CELL_GRASS_POINTS;

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

struct IndexRow {
    x: i32,
    y: i32,
    channels: [(u64, u64); CHANNEL_COUNT],
}

/// Where compressed channel chunks wait until `finish` knows the index size.
/// Full Appalachia is ~40k cells, so production writers spill to disk and only
/// keep the index rows resident.
enum ChunkSink {
    Memory(Vec<u8>),
    Spill {
        writer: std::io::BufWriter<std::fs::File>,
        path: std::path::PathBuf,
        len: u64,
    },
}

impl ChunkSink {
    fn append(&mut self, bytes: &[u8]) -> Result<u64, String> {
        match self {
            ChunkSink::Memory(buffer) => {
                let offset = buffer.len() as u64;
                buffer.extend_from_slice(bytes);
                Ok(offset)
            }
            ChunkSink::Spill { writer, len, .. } => {
                let offset = *len;
                writer
                    .write_all(bytes)
                    .map_err(|e| format!("btd4 write: spill chunk: {e}"))?;
                *len += bytes.len() as u64;
                Ok(offset)
            }
        }
    }
}

pub struct Btd4Writer {
    header: Btd4Header,
    rows: Vec<IndexRow>,
    seen: std::collections::HashSet<(i32, i32)>,
    sink: ChunkSink,
    finished: bool,
    raw_material_bank: Vec<u8>,
}

impl Btd4Writer {
    /// Keeps compressed chunks in memory; for small outputs and tests. `raw_material_bank` is the
    /// `raw_terrain` bank every cell's RAWT patch indexes.
    pub fn new(header: Btd4Header, raw_material_bank: Vec<u8>) -> Result<Self, String> {
        Self::with_sink(header, raw_material_bank, ChunkSink::Memory(Vec::new()))
    }

    /// Streams compressed chunks to `spill_path` so resident memory stays at the
    /// index size regardless of worldspace size. The spill file is removed by
    /// `finish` or when the writer is dropped unfinished.
    pub fn with_spill_file(header: Btd4Header, raw_material_bank: Vec<u8>, spill_path: &Path) -> Result<Self, String> {
        crate::raw_terrain::validate_bank(&raw_material_bank)?;
        if let Some(parent) = spill_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("btd4 write: create spill dir: {e}"))?;
        }
        let file = std::fs::File::create(spill_path)
            .map_err(|e| format!("btd4 write: create spill {}: {e}", spill_path.display()))?;
        Self::with_sink(
            header,
            raw_material_bank,
            ChunkSink::Spill {
                writer: std::io::BufWriter::with_capacity(1 << 20, file),
                path: spill_path.to_path_buf(),
                len: 0,
            },
        )
    }

    fn with_sink(header: Btd4Header, raw_material_bank: Vec<u8>, sink: ChunkSink) -> Result<Self, String> {
        crate::raw_terrain::validate_bank(&raw_material_bank)?;
        Ok(Self {
            header,
            rows: Vec::new(),
            seen: std::collections::HashSet::new(),
            sink,
            finished: false,
            raw_material_bank,
        })
    }

    pub fn cell_count(&self) -> usize {
        self.rows.len()
    }

    /// Compresses the cell immediately; the raw channels are not retained.
    pub fn add_cell(&mut self, x: i32, y: i32, channels: CellChannels) -> Result<(), String> {
        validate_channels(&channels)?;
        self.validate_against_reader_contract(x, y, &channels)?;
        let mut slots = [(0u64, 0u64); CHANNEL_COUNT];
        let payloads: [Option<Vec<u8>>; CHANNEL_COUNT] = [
            channels.heights.as_deref().map(encode_hgts),
            channels.alphas.as_deref().map(encode_alph),
            channels.layers.as_deref().map(encode_layr),
            channels.gcvr.as_ref().map(encode_gcvr),
            channels.colors,
            channels.raw_terrain,
            channels.grass_points.as_ref().map(encode_grass_points),
        ];
        for (slot, payload) in slots.iter_mut().zip(payloads) {
            if let Some(raw) = payload {
                let compressed = zlib_compress(&raw)?;
                let offset = self.sink.append(&compressed)?;
                *slot = (offset, compressed.len() as u64);
            }
        }
        self.seen.insert((x, y));
        self.rows.push(IndexRow { x, y, channels: slots });
        Ok(())
    }

    /// Every rule `Btd4Reader::open` and the Tales runtime reader enforce, so a
    /// written file can never be rejected at load.
    fn validate_against_reader_contract(
        &self,
        x: i32,
        y: i32,
        channels: &CellChannels,
    ) -> Result<(), String> {
        let h = &self.header;
        let raw = channels.raw_terrain.as_deref().ok_or_else(|| format!("btd4 cell ({x},{y}) has no RAWT channel"))?;
        crate::raw_terrain::validate_cell(&self.raw_material_bank, raw, x, y)?;
        if let Some(grass_points) = &channels.grass_points {
            validate_grass_points(grass_points, h.plugin_names.len())
                .map_err(|error| format!("btd4 cell ({x},{y}) GRSP: {error}"))?;
        }
        if x < h.cell_min_x || x > h.cell_max_x || y < h.cell_min_y || y > h.cell_max_y {
            return Err(format!(
                "btd4 cell ({x},{y}) is outside header bounds ({},{})..({},{})",
                h.cell_min_x, h.cell_min_y, h.cell_max_x, h.cell_max_y
            ));
        }
        if self.seen.contains(&(x, y)) {
            return Err(format!("btd4 cell ({x},{y}) was added twice"));
        }
        if channels.heights.is_none() {
            return Err(format!("btd4 cell ({x},{y}) has no HGTS channel"));
        }
        if channels.alphas.is_some() && channels.layers.is_none() {
            return Err(format!("btd4 cell ({x},{y}) has ALPH without its LAYR table"));
        }
        let plugin_count = h.plugin_names.len();
        if let Some(layers) = &channels.layers {
            for (slot, layer) in layers.iter().enumerate() {
                let empty = layer.plugin_index == u8::MAX && layer.object_id == 0;
                if !empty
                    && (layer.plugin_index as usize >= plugin_count
                        || layer.object_id == 0
                        || layer.kind != 0)
                {
                    return Err(format!(
                        "btd4 cell ({x},{y}) LAYR slot {slot} is not a valid plugin-local LTEX reference"
                    ));
                }
            }
        }
        if let Some(gcvr) = &channels.gcvr {
            for entry in &gcvr.entries {
                if entry.plugin_index as usize >= plugin_count || entry.object_id == 0 {
                    return Err(format!(
                        "btd4 cell ({x},{y}) GCVR entry {:06X} is not a valid plugin-local GRAS reference",
                        entry.object_id
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn finish(mut self, path: &Path) -> Result<(), String> {
        // Index rows ascend by (y, x), the order readers expect. Chunk bytes stay in
        // add order; offsets are absolute, so no reader depends on chunk order.
        self.rows.sort_by_key(|row| (row.y, row.x));

        let mut head: Vec<u8> = Vec::new();
        write_header_bytes(&mut head, &self.header, self.rows.len() as u32);
        head.extend_from_slice(&(self.raw_material_bank.len() as u32).to_le_bytes());
        head.extend_from_slice(&self.raw_material_bank);
        let data_start = (head.len() + self.rows.len() * (8 + CHANNEL_COUNT * 16)) as u64;
        for row in &self.rows {
            head.extend_from_slice(&row.x.to_le_bytes());
            head.extend_from_slice(&row.y.to_le_bytes());
            for &(offset, len) in &row.channels {
                let absolute = if len == 0 { 0 } else { data_start + offset };
                head.extend_from_slice(&absolute.to_le_bytes());
                head.extend_from_slice(&len.to_le_bytes());
            }
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("btd4 write: create parent dir: {e}"))?;
        }
        let partial = partial_output_path(path);
        let result = self.write_output(&partial, &head).and_then(|()| {
            std::fs::rename(&partial, path).map_err(|e| format!("btd4 write: rename: {e}"))
        });
        if result.is_err() {
            let _ = std::fs::remove_file(&partial);
        }
        self.finished = true;
        self.remove_spill();
        result
    }

    fn write_output(&mut self, partial: &Path, head: &[u8]) -> Result<(), String> {
        let file = std::fs::File::create(partial).map_err(|e| format!("btd4 write: {e}"))?;
        let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
        out.write_all(head).map_err(|e| format!("btd4 write: {e}"))?;
        match &mut self.sink {
            ChunkSink::Memory(buffer) => {
                out.write_all(buffer).map_err(|e| format!("btd4 write: {e}"))?;
            }
            ChunkSink::Spill { writer, path, .. } => {
                writer
                    .flush()
                    .map_err(|e| format!("btd4 write: flush spill: {e}"))?;
                let mut spill = std::fs::File::open(&*path)
                    .map_err(|e| format!("btd4 write: reopen spill: {e}"))?;
                std::io::copy(&mut spill, &mut out)
                    .map_err(|e| format!("btd4 write: copy spill: {e}"))?;
            }
        }
        out.flush().map_err(|e| format!("btd4 write: {e}"))?;
        Ok(())
    }

    fn remove_spill(&mut self) {
        if let ChunkSink::Spill { path, .. } = &self.sink {
            let path = path.clone();
            // Dropping the BufWriter closes the handle before the delete.
            self.sink = ChunkSink::Memory(Vec::new());
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for Btd4Writer {
    fn drop(&mut self) {
        if !self.finished {
            self.remove_spill();
        }
    }
}

/// The complete file is renamed into place only after its last byte is written,
/// so a failed run never leaves a truncated sidecar for the runtime to load.
fn partial_output_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".partial");
    path.with_file_name(name)
}

/// Spill file for [`Btd4Writer::with_spill_file`], beside the output.
pub fn spill_path_for(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".chunks.tmp");
    path.with_file_name(name)
}

pub(crate) fn compressed_bound(maximum: usize) -> usize {
    maximum + (maximum >> 12) + (maximum >> 14) + (maximum >> 25) + 13
}

pub(crate) fn decompress_bounded(compressed: &[u8], maximum: usize) -> Result<Vec<u8>, String> {
    if compressed.is_empty() || compressed.len() > compressed_bound(maximum) { return Err("compressed channel exceeds writer bound".into()); }
    let mut decoder = flate2::Decompress::new(true);
    let mut out = Vec::with_capacity(maximum + 1);
    let status = decoder.decompress_vec(compressed, &mut out, flate2::FlushDecompress::Finish)
        .map_err(|error| format!("corrupt compressed channel: {error}"))?;
    if status != flate2::Status::StreamEnd || decoder.total_in() != compressed.len() as u64 || out.len() > maximum {
        return Err("compressed channel is truncated, trailing or oversized".into());
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ChannelEntry {
    offset: u64,
    len: u64,
}

#[derive(Debug, Clone)]
struct CellIndex {
    channels: [ChannelEntry; CHANNEL_COUNT],
}

pub struct Btd4Reader {
    header: Btd4Header,
    cells: BTreeMap<(i32, i32), CellIndex>,
    data: Vec<u8>,
    raw_material_bank: Vec<u8>,
}

impl Btd4Reader {
    pub fn open(path: &Path) -> Result<Self, String> {
        let data = std::fs::read(path).map_err(|e| format!("btd4 open: {e}"))?;
        let buf = &data;
        let mut pos = 0usize;

        // Magic
        let magic = read_bytes(buf, &mut pos, 4)?;
        if magic != b"BTD4" {
            return Err("btd4: bad magic".into());
        }

        let density = read_u32_le(buf, &mut pos)?;
        let height_min = read_f32_le(buf, &mut pos)?;
        let height_scale = read_f32_le(buf, &mut pos)?;
        let worldspace_editor_id = read_string(buf, &mut pos)?;
        if density != 128
            || !height_min.is_finite()
            || !height_scale.is_finite()
            || height_scale <= 0.0
            || worldspace_editor_id.is_empty()
        {
            return Err("btd4: invalid header".into());
        }

        let plugin_count = read_u16_le(buf, &mut pos)? as usize;
        if plugin_count == 0 {
            return Err("btd4: empty plugin table".into());
        }
        let mut plugin_names = Vec::with_capacity(plugin_count);
        for _ in 0..plugin_count {
            let name = read_string(buf, &mut pos)?;
            if name.is_empty() {
                return Err("btd4: empty plugin name".into());
            }
            plugin_names.push(name);
        }

        let cell_min_x = read_i32_le(buf, &mut pos)?;
        let cell_min_y = read_i32_le(buf, &mut pos)?;
        let cell_max_x = read_i32_le(buf, &mut pos)?;
        let cell_max_y = read_i32_le(buf, &mut pos)?;
        let cell_count = read_u32_le(buf, &mut pos)? as usize;
        if cell_min_x > cell_max_x || cell_min_y > cell_max_y || cell_count > 1_000_000 {
            return Err("btd4: invalid cell bounds or count".into());
        }

        let raw_material_bank = read_bank(buf, &mut pos)?;
        let mut cells: BTreeMap<(i32, i32), CellIndex> = BTreeMap::new();
        for _ in 0..cell_count {
            let cx = read_i32_le(buf, &mut pos)?;
            let cy = read_i32_le(buf, &mut pos)?;
            if cx < cell_min_x || cx > cell_max_x || cy < cell_min_y || cy > cell_max_y {
                return Err("btd4: indexed cell outside header bounds".into());
            }
            let mut channels: [ChannelEntry; CHANNEL_COUNT] =
                std::array::from_fn(|_| ChannelEntry { offset: 0, len: 0 });
            for ch in channels.iter_mut() {
                ch.offset = read_u64_le(buf, &mut pos)?;
                ch.len = read_u64_le(buf, &mut pos)?;
                if ch.len > 0 {
                    let end = ch
                        .offset
                        .checked_add(ch.len)
                        .ok_or("btd4: offset overflow")?;
                    if end as usize > buf.len() {
                        return Err("btd4: channel range out of file".into());
                    }
                }
            }
            if channels[0].len == 0 || cells.insert((cx, cy), CellIndex { channels }).is_some() {
                return Err("btd4: missing HGTS or duplicate cell".into());
            }
        }

        let reader = Self {
            header: Btd4Header {
                density,
                height_min,
                height_scale,
                worldspace_editor_id,
                plugin_names,
                cell_min_x,
                cell_min_y,
                cell_max_x,
                cell_max_y,
            },
            cells,
            data,
            raw_material_bank,
        };
        for (&(x, y), cell) in &reader.cells {
            if reader.decode_hgts(&cell.channels[0]).is_none()
                || (cell.channels[1].len != 0 && reader.decode_alph(&cell.channels[1]).is_none())
                || (cell.channels[2].len != 0 && reader.decode_layr(&cell.channels[2]).is_none())
                || (cell.channels[3].len != 0 && reader.decode_gcvr(&cell.channels[3]).is_none())
                || (cell.channels[4].len != 0 && reader.decode_clrs(&cell.channels[4]).is_none())
                || (cell.channels[1].len != 0 && cell.channels[2].len == 0)
                || reader.decode_rawt(&cell.channels[5], x, y).is_none()
                || (cell.channels[6].len != 0 && reader.decode_grass_points(&cell.channels[6]).is_err())
            {
                return Err(format!("btd4: corrupt channel in cell ({x},{y})"));
            }
        }
        Ok(reader)
    }

    pub fn header(&self) -> &Btd4Header {
        &self.header
    }

    pub fn raw_material_bank(&self) -> &[u8] { &self.raw_material_bank }

    pub fn cell(&self, x: i32, y: i32) -> Option<CellChannels> {
        let idx = self.cells.get(&(x, y))?;

        Some(CellChannels {
            raw_terrain: self.decode_rawt(&idx.channels[5], x, y),
            heights: self.decode_hgts(&idx.channels[0]),
            alphas: self.decode_alph(&idx.channels[1]),
            layers: self.decode_layr(&idx.channels[2]),
            gcvr: self.decode_gcvr(&idx.channels[3]),
            colors: self.decode_clrs(&idx.channels[4]),
            grass_points: self.decode_grass_points(&idx.channels[6]).ok().flatten(),
        })
    }

    fn decode_grass_points(&self, ch: &ChannelEntry) -> Result<Option<GrassPointsChunk>, String> {
        if ch.len == 0 {
            return Ok(None);
        }
        let raw = self.decompress(ch, MAX_GRASS_POINTS_BYTES).ok_or("corrupt GRSP chunk")?;
        decode_grass_points(&raw, self.header.plugin_names.len()).map(Some)
    }

    fn decode_rawt(&self, ch: &ChannelEntry, x: i32, y: i32) -> Option<Vec<u8>> {
        let raw = self.decompress(ch, crate::raw_terrain::CELL_BYTES)?;
        crate::raw_terrain::validate_cell(&self.raw_material_bank, &raw, x, y).ok()?;
        Some(raw)
    }

    fn decompress(&self, ch: &ChannelEntry, maximum: usize) -> Option<Vec<u8>> {
        let start = usize::try_from(ch.offset).ok()?;
        let end = start.checked_add(usize::try_from(ch.len).ok()?)?;
        decompress_bounded(self.data.get(start..end)?, maximum).ok()
    }

    fn decode_hgts(&self, ch: &ChannelEntry) -> Option<Vec<u16>> {
        if ch.len == 0 {
            return None;
        }
        let raw = self.decompress(ch, 129 * 129 * 2)?;
        const EXPECTED: usize = 129 * 129 * 2;
        if raw.len() != EXPECTED {
            return None;
        }
        let heights: Vec<u16> = raw
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        Some(heights)
    }

    fn decode_alph(&self, ch: &ChannelEntry) -> Option<Vec<Vec<u8>>> {
        if ch.len == 0 {
            return None;
        }
        let raw = self.decompress(ch, 1 + ALPH_PLANE_COUNT * ALPH_PLANE_LEN)?;
        if raw.is_empty() {
            return None;
        }
        if raw[0] as usize != ALPH_PLANE_COUNT || raw.len() != 1 + ALPH_PLANE_COUNT * ALPH_PLANE_LEN {
            return None;
        }
        Some(raw[1..].chunks_exact(ALPH_PLANE_LEN).map(<[u8]>::to_vec).collect())
    }

    fn decode_layr(&self, ch: &ChannelEntry) -> Option<Vec<LayerRef>> {
        if ch.len == 0 {
            return None;
        }
        let raw = self.decompress(ch, 1 + LAYER_COUNT * 6)?;
        if raw.is_empty() {
            return None;
        }
        const ROW_SIZE: usize = 6;
        if raw[0] as usize != LAYER_COUNT || raw.len() != 1 + LAYER_COUNT * ROW_SIZE {
            return None;
        }
        let layers: Vec<_> = (0..LAYER_COUNT)
            .map(|i| {
                let base = 1 + i * ROW_SIZE;
                LayerRef {
                    plugin_index: raw[base],
                    object_id: u32::from_le_bytes([
                        raw[base + 1],
                        raw[base + 2],
                        raw[base + 3],
                        raw[base + 4],
                    ]),
                    kind: raw[base + 5],
                }
            })
            .collect();
        if layers.iter().any(|layer| {
            let empty = layer.plugin_index == u8::MAX && layer.object_id == 0;
            !empty
                && (layer.plugin_index as usize >= self.header.plugin_names.len()
                    || layer.object_id == 0
                    || layer.kind != 0)
        }) {
            return None;
        }
        Some(layers)
    }

    fn decode_gcvr(&self, ch: &ChannelEntry) -> Option<GcvrChunk> {
        if ch.len == 0 {
            return None;
        }
        let raw = self.decompress(ch, 1 + 255 * (5 + 128 * 128))?;
        const MASK_SIZE: usize = 128 * 128;
        const ENTRY_SIZE: usize = 5 + MASK_SIZE;
        if raw.is_empty() {
            return None;
        }
        let form_count = raw[0] as usize;
        let expected = 1 + form_count * ENTRY_SIZE;
        if raw.len() != expected {
            return None;
        }
        let entries: Vec<_> = (0..form_count)
            .map(|i| {
                let base = 1 + i * ENTRY_SIZE;
                GcvrEntry {
                    plugin_index: raw[base],
                    object_id: u32::from_le_bytes([
                        raw[base + 1],
                        raw[base + 2],
                        raw[base + 3],
                        raw[base + 4],
                    ]),
                    mask: raw[base + 5..base + ENTRY_SIZE].to_vec(),
                }
            })
            .collect();
        if entries.iter().any(|entry| {
            entry.plugin_index as usize >= self.header.plugin_names.len() || entry.object_id == 0
        }) {
            return None;
        }
        Some(GcvrChunk { entries })
    }

    fn decode_clrs(&self, ch: &ChannelEntry) -> Option<Vec<u8>> {
        if ch.len == 0 {
            return None;
        }
        let raw = self.decompress(ch, 129 * 129 * 3)?;
        const EXPECTED: usize = 129 * 129 * 3;
        if raw.len() != EXPECTED {
            return None;
        }
        Some(raw)
    }
}

/// The raw source material bank, read from the file's header alone. `Btd4Reader::open` loads and validates
/// the whole file, which for a worldspace is gigabytes; terrain LOD needs only this bank.
pub fn read_raw_material_bank(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    // Header and bank fit well inside this prefix; a header that does not is reported as truncated.
    const PREFIX: u64 = 4 << 20;
    let mut buf = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(PREFIX).read_to_end(&mut buf))
        .map_err(|e| format!("btd4 open: {e}"))?;
    let mut pos = 0usize;
    if read_bytes(&buf, &mut pos, 4)? != b"BTD4" {
        return Err("btd4: bad magic".into());
    }
    if read_u32_le(&buf, &mut pos)? != 128 {
        return Err("btd4: invalid header".into());
    }
    read_bytes(&buf, &mut pos, 8)?;
    read_string(&buf, &mut pos)?;
    let plugin_count = read_u16_le(&buf, &mut pos)?;
    for _ in 0..plugin_count {
        read_string(&buf, &mut pos)?;
    }
    read_bytes(&buf, &mut pos, 20)?;
    read_bank(&buf, &mut pos)
}

fn read_bank(buf: &[u8], pos: &mut usize) -> Result<Vec<u8>, String> {
    let bank_length = read_u32_le(buf, pos)? as usize;
    if bank_length > crate::raw_terrain::MAX_BANK_BYTES {
        return Err("raw bank exceeds size limit".into());
    }
    let bank = read_bytes(buf, pos, bank_length)?.to_vec();
    crate::raw_terrain::validate_bank(&bank)?;
    Ok(bank)
}

// ---------------------------------------------------------------------------
// Encode helpers (decompressed payload builders)
// ---------------------------------------------------------------------------

fn encode_hgts(heights: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(heights.len() * 2);
    for &h in heights {
        out.extend_from_slice(&h.to_le_bytes());
    }
    out
}

fn encode_alph(alphas: &[Vec<u8>]) -> Vec<u8> {
    let layer_count = alphas.len() as u8;
    let mut out = Vec::with_capacity(1 + alphas.len() * 128 * 128);
    out.push(layer_count);
    for layer in alphas {
        out.extend_from_slice(layer);
    }
    out
}

fn encode_layr(layers: &[LayerRef]) -> Vec<u8> {
    let row_count = layers.len() as u8;
    let mut out = Vec::with_capacity(1 + layers.len() * 6);
    out.push(row_count);
    for row in layers {
        out.push(row.plugin_index);
        out.extend_from_slice(&row.object_id.to_le_bytes());
        out.push(row.kind);
    }
    out
}

fn encode_gcvr(gcvr: &GcvrChunk) -> Vec<u8> {
    let form_count = gcvr.entries.len() as u8;
    let mut out = Vec::with_capacity(1 + gcvr.entries.len() * (5 + 128 * 128));
    out.push(form_count);
    for entry in &gcvr.entries {
        out.push(entry.plugin_index);
        out.extend_from_slice(&entry.object_id.to_le_bytes());
        out.extend_from_slice(&entry.mask);
    }
    out
}

pub(crate) fn encode_grass_points(chunk: &GrassPointsChunk) -> Vec<u8> {
    let point_count: usize = chunk.runs.iter().map(|run| run.points.len()).sum();
    let mut out = Vec::with_capacity(
        3 + chunk.grasses.len() * GRASS_TABLE_ENTRY_BYTES
            + chunk.runs.len() * GRASS_RUN_HEADER_BYTES
            + point_count * 4,
    );
    out.push(chunk.grasses.len() as u8);
    for grass in &chunk.grasses {
        let d = &grass.dnam;
        out.push(grass.plugin_index);
        out.extend_from_slice(&grass.object_id.to_le_bytes());
        out.extend_from_slice(&[d.density, d.min_slope, d.max_slope, d.flags]);
        out.extend_from_slice(&d.water_above.to_le_bytes());
        out.extend_from_slice(&d.water_below.to_le_bytes());
        for value in [d.height_range, d.color_range, d.wave_period, d.freq_base, d.sharp, d.cover, d.lod_blend] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out.extend_from_slice(&(chunk.runs.len() as u16).to_le_bytes());
    for run in &chunk.runs {
        out.extend_from_slice(&[run.block_x, run.block_y, run.param_index, run.grass]);
        out.extend_from_slice(&run.seed_after.to_le_bytes());
        out.extend_from_slice(&(run.points.len() as u16).to_le_bytes());
        for point in &run.points {
            out.extend_from_slice(&point[0].to_le_bytes());
            out.extend_from_slice(&point[1].to_le_bytes());
        }
    }
    out
}

/// The GRSP rules shared by the writer, `Btd4Reader` and `btd4_verify`.
pub(crate) fn validate_grass_points(chunk: &GrassPointsChunk, plugin_count: usize) -> Result<(), String> {
    if chunk.grasses.is_empty() || chunk.grasses.len() > 255 {
        return Err(format!("grass table has {} entries (1..=255)", chunk.grasses.len()));
    }
    for (index, grass) in chunk.grasses.iter().enumerate() {
        if grass.plugin_index as usize >= plugin_count || grass.object_id == 0 {
            return Err(format!("grass {index} ({:06X}) is not a valid plugin-local GRAS reference", grass.object_id));
        }
    }
    if chunk.runs.len() > MAX_GRASS_RUNS {
        return Err(format!("{} runs exceed {MAX_GRASS_RUNS}", chunk.runs.len()));
    }
    let mut total_points = 0usize;
    let mut previous: Option<(u8, u8, u8)> = None;
    for run in &chunk.runs {
        let key = (run.block_x, run.block_y, run.param_index);
        if previous.is_some_and(|previous| previous >= key) {
            return Err(format!("run {key:?} is not strictly ascending"));
        }
        previous = Some(key);
        if usize::from(run.block_x) >= crate::fo76_grass::BLOCKS_PER_AXIS
            || usize::from(run.block_y) >= crate::fo76_grass::BLOCKS_PER_AXIS
            || usize::from(run.param_index) >= crate::fo76_grass::MAX_PARAMS_PER_BLOCK
        {
            return Err(format!("run {key:?} is outside the 16x16 block / 24 param grid"));
        }
        if usize::from(run.grass) >= chunk.grasses.len() {
            return Err(format!("run {key:?} names grass {} of {}", run.grass, chunk.grasses.len()));
        }
        if run.points.is_empty() || run.points.len() > MAX_RUN_POINTS {
            return Err(format!("run {key:?} has {} points (1..={MAX_RUN_POINTS})", run.points.len()));
        }
        if let Some(point) = run.points.iter().flatten().find(|v| !(GRASS_POINT_MIN..=GRASS_POINT_MAX).contains(*v)) {
            return Err(format!("run {key:?} point coordinate {point} is outside its block window"));
        }
        total_points += run.points.len();
    }
    if total_points > MAX_CELL_GRASS_POINTS {
        return Err(format!("{total_points} points exceed {MAX_CELL_GRASS_POINTS}"));
    }
    Ok(())
}

pub(crate) fn decode_grass_points(raw: &[u8], plugin_count: usize) -> Result<GrassPointsChunk, String> {
    let mut pos = 0usize;
    let grass_count = usize::from(*read_bytes(raw, &mut pos, 1)?.first().unwrap());
    let mut grasses = Vec::with_capacity(grass_count);
    for _ in 0..grass_count {
        let b = read_bytes(raw, &mut pos, GRASS_TABLE_ENTRY_BYTES)?;
        let f32_at = |offset: usize| f32::from_le_bytes(b[offset..offset + 4].try_into().unwrap());
        grasses.push(GrassPointsGrass {
            plugin_index: b[0],
            object_id: u32::from_le_bytes(b[1..5].try_into().unwrap()),
            dnam: crate::fo76_grass::GrassDnam {
                density: b[5],
                min_slope: b[6],
                max_slope: b[7],
                flags: b[8],
                water_above: u16::from_le_bytes([b[9], b[10]]),
                water_below: u16::from_le_bytes([b[11], b[12]]),
                height_range: f32_at(13),
                color_range: f32_at(17),
                wave_period: f32_at(21),
                freq_base: f32_at(25),
                sharp: f32_at(29),
                cover: f32_at(33),
                lod_blend: f32_at(37),
            },
        });
    }
    let run_count = usize::from(read_u16_le(raw, &mut pos)?);
    if run_count > MAX_GRASS_RUNS {
        return Err(format!("{run_count} runs exceed {MAX_GRASS_RUNS}"));
    }
    let mut runs = Vec::with_capacity(run_count);
    for _ in 0..run_count {
        let b = read_bytes(raw, &mut pos, GRASS_RUN_HEADER_BYTES)?;
        let point_count = usize::from(u16::from_le_bytes([b[8], b[9]]));
        let point_bytes = read_bytes(raw, &mut pos, point_count * 4)?;
        runs.push(GrassPointsRun {
            block_x: b[0],
            block_y: b[1],
            param_index: b[2],
            grass: b[3],
            seed_after: i32::from_le_bytes(b[4..8].try_into().unwrap()),
            points: point_bytes
                .chunks_exact(4)
                .map(|p| [i16::from_le_bytes([p[0], p[1]]), i16::from_le_bytes([p[2], p[3]])])
                .collect(),
        });
    }
    if pos != raw.len() {
        return Err(format!("{} trailing bytes", raw.len() - pos));
    }
    let chunk = GrassPointsChunk { grasses, runs };
    validate_grass_points(&chunk, plugin_count)?;
    Ok(chunk)
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn validate_channels(ch: &CellChannels) -> Result<(), String> {
    if let Some(h) = &ch.heights {
        if h.len() != 129 * 129 {
            return Err(format!(
                "heights must be 129*129={} u16, got {}",
                129 * 129,
                h.len()
            ));
        }
    }
    if let Some(alphas) = &ch.alphas {
        if alphas.len() != ALPH_PLANE_COUNT {
            return Err(format!(
                "dense alpha must be exactly {ALPH_PLANE_COUNT} planes (4 quadrants × 7 slots), got {}",
                alphas.len()
            ));
        }
        for (i, plane) in alphas.iter().enumerate() {
            if plane.len() != ALPH_PLANE_LEN {
                return Err(format!(
                    "alpha plane {i} must be 65*65={ALPH_PLANE_LEN} u8, got {}",
                    plane.len()
                ));
            }
        }
    }
    if let Some(layers) = &ch.layers {
        if layers.len() != LAYER_COUNT {
            return Err(format!(
                "LAYR must contain exactly 32 ordered refs (4 quadrants × 8 slots), got {}",
                layers.len()
            ));
        }
    }
    if let Some(gcvr) = &ch.gcvr {
        if gcvr.entries.len() > 255 {
            return Err(format!(
                "gcvr form count {} exceeds 255",
                gcvr.entries.len()
            ));
        }
        for (index, entry) in gcvr.entries.iter().enumerate() {
            if entry.mask.len() != 128 * 128 {
                return Err(format!(
                    "gcvr entry {index} mask must be 128*128={} u8, got {}",
                    128 * 128,
                    entry.mask.len()
                ));
            }
        }
    }
    if let Some(colors) = &ch.colors {
        if colors.len() != 129 * 129 * 3 {
            return Err(format!(
                "colors must be 129*129*3={} u8, got {}",
                129 * 129 * 3,
                colors.len()
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Compression
// ---------------------------------------------------------------------------

pub(crate) fn zlib_compress(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(data)
        .map_err(|e| format!("zlib compress: {e}"))?;
    encoder.finish().map_err(|e| format!("zlib finish: {e}"))
}

// ---------------------------------------------------------------------------
// File serialisation helpers
// ---------------------------------------------------------------------------

fn write_header_bytes(buf: &mut Vec<u8>, header: &Btd4Header, cell_count: u32) {
    buf.extend_from_slice(b"BTD4");
    buf.extend_from_slice(&header.density.to_le_bytes());
    buf.extend_from_slice(&header.height_min.to_le_bytes());
    buf.extend_from_slice(&header.height_scale.to_le_bytes());

    write_pascal_string(buf, &header.worldspace_editor_id);

    buf.extend_from_slice(&(header.plugin_names.len() as u16).to_le_bytes());
    for name in &header.plugin_names {
        write_pascal_string(buf, name);
    }

    buf.extend_from_slice(&header.cell_min_x.to_le_bytes());
    buf.extend_from_slice(&header.cell_min_y.to_le_bytes());
    buf.extend_from_slice(&header.cell_max_x.to_le_bytes());
    buf.extend_from_slice(&header.cell_max_y.to_le_bytes());
    buf.extend_from_slice(&cell_count.to_le_bytes());
}

fn write_pascal_string(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    buf.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(bytes);
}

// ---------------------------------------------------------------------------
// Read helpers (for Btd4Reader::open)
// ---------------------------------------------------------------------------

fn read_bytes<'a>(buf: &'a [u8], pos: &mut usize, n: usize) -> Result<&'a [u8], String> {
    let end = pos.checked_add(n).ok_or("btd4 read: overflow")?;
    if end > buf.len() {
        return Err(format!("btd4 read: truncated at {}", *pos));
    }
    let slice = &buf[*pos..end];
    *pos = end;
    Ok(slice)
}

fn read_u16_le(buf: &[u8], pos: &mut usize) -> Result<u16, String> {
    let b = read_bytes(buf, pos, 2)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn read_u32_le(buf: &[u8], pos: &mut usize) -> Result<u32, String> {
    let b = read_bytes(buf, pos, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_u64_le(buf: &[u8], pos: &mut usize) -> Result<u64, String> {
    let b = read_bytes(buf, pos, 8)?;
    Ok(u64::from_le_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}

fn read_i32_le(buf: &[u8], pos: &mut usize) -> Result<i32, String> {
    let b = read_bytes(buf, pos, 4)?;
    Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_f32_le(buf: &[u8], pos: &mut usize) -> Result<f32, String> {
    let b = read_bytes(buf, pos, 4)?;
    Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_string(buf: &[u8], pos: &mut usize) -> Result<String, String> {
    let len = read_u16_le(buf, pos)? as usize;
    if len == 0 {
        return Ok(String::new());
    }
    let b = read_bytes(buf, pos, len)?;
    String::from_utf8(b.to_vec()).map_err(|e| format!("btd4 string utf8: {e}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    /// Test cells leave RAWT to this: every cell carries the patch of its own coordinates.
    trait AddWithRaw {
        fn add(&mut self, x: i32, y: i32, channels: CellChannels) -> Result<(), String>;
    }

    impl AddWithRaw for Btd4Writer {
        fn add(&mut self, x: i32, y: i32, mut channels: CellChannels) -> Result<(), String> {
            channels.raw_terrain.get_or_insert_with(|| raw_cell(x, y));
            self.add_cell(x, y, channels)
        }
    }

    fn writer() -> Btd4Writer {
        Btd4Writer::new(make_header(), raw_bank()).unwrap()
    }

    fn make_header() -> Btd4Header {
        Btd4Header {
            density: 128,
            height_min: -2048.0,
            height_scale: 0.125,
            worldspace_editor_id: "TestWorld".into(),
            plugin_names: vec!["B21_Test.esp".into(), "Fallout4.esm".into()],
            cell_min_x: 0,
            cell_min_y: 0,
            cell_max_x: 1,
            cell_max_y: 2,
        }
    }

    fn synthetic_alpha_planes() -> Vec<Vec<u8>> {
        // ALPH_PLANE_COUNT (28) planes of ALPH_PLANE_LEN (4225) u8;
        // plane p, texel k => ((p*31 + k) % 256). Distinct per plane.
        (0u32..ALPH_PLANE_COUNT as u32)
            .map(|p| {
                (0u32..ALPH_PLANE_LEN as u32)
                    .map(|k| ((p * 31 + k) % 256) as u8)
                    .collect()
            })
            .collect()
    }

    fn full_channels() -> CellChannels {
        let mut layers = vec![
            LayerRef {
                plugin_index: u8::MAX,
                object_id: 0,
                kind: 0,
            };
            LAYER_COUNT
        ];
        layers[0] = LayerRef {
            plugin_index: 0,
            object_id: 0x000800,
            kind: 0,
        };
        layers[1] = LayerRef {
            plugin_index: 1,
            object_id: 0x0001A7,
            kind: 0,
        };
        CellChannels {
            raw_terrain: None,
            heights: Some((0u32..16641u32).map(|i| (i % 1000) as u16).collect()),
            alphas: Some(synthetic_alpha_planes()),
            layers: Some(layers),
            gcvr: Some(GcvrChunk {
                entries: vec![GcvrEntry {
                    plugin_index: 0,
                    object_id: 0x000810,
                    mask: (0u32..16384u32).map(|i| (i % 251) as u8).collect(),
                }],
            }),
            colors: Some((0u32..49923u32).map(|i| (i % 256) as u8).collect()),
            grass_points: None,
        }
    }

    fn hgts_only_channels() -> CellChannels {
        CellChannels {
            raw_terrain: None,
            heights: Some(vec![4096u16; 16641]),
            alphas: None,
            layers: None,
            gcvr: None,
            colors: None,
            grass_points: None,
        }
    }

    pub(crate) fn raw_bank() -> Vec<u8> {
        crate::raw_terrain::encode_bank(&[crate::raw_terrain::MaterialSource {
            source_ltex_form_id: 0x1234,
            rows: [[0.25, 0.0, 1.0, 0.0], [0.0; 4], [0.0; 4]],
            textures: ["d", "n", "l", "r"].map(|map| format!("textures/terrain/test_{map}.dds")),
        }])
        .unwrap()
    }

    pub(crate) fn raw_cell(cell_x: i32, cell_y: i32) -> Vec<u8> {
        (0..4)
            .flat_map(|quadrant: i32| {
                let (qx, qy) = (quadrant & 1, quadrant >> 1);
                crate::raw_terrain::encode_patch(&crate::btd::RawTerrainPatch {
                    storage_origin: [cell_x * 128 + 64 + qx * 64, cell_y * 128 + 64 + qy * 64],
                    native_grid_origin: [(qx * 64) as f32, (qy * 64) as f32],
                    selectors: vec![0; crate::btd::RAW_SELECTOR_SIDE * crate::btd::RAW_SELECTOR_SIDE],
                    palettes: [[0; 8]; 9],
                    colors: vec![0; crate::btd::RAW_COLOR_SIDE * crate::btd::RAW_COLOR_SIDE],
                })
                .unwrap()
            })
            .collect()
    }

    pub(crate) fn grass_points_chunk() -> GrassPointsChunk {
        let dnam = crate::fo76_grass::GrassDnam {
            sharp: 95.0,
            freq_base: 0.1,
            height_range: 0.3,
            color_range: 0.2,
            wave_period: 145.0,
            water_above: 0,
            water_below: 0xFFFF,
            density: 94,
            min_slope: 0,
            max_slope: 57,
            flags: 0x15,
            cover: 97.0,
            lod_blend: 0.1,
        };
        GrassPointsChunk {
            grasses: vec![
                GrassPointsGrass { plugin_index: 0, object_id: 0x000900, dnam },
                GrassPointsGrass { plugin_index: 1, object_id: 0x0BA520, dnam: crate::fo76_grass::GrassDnam { density: 50, ..dnam } },
            ],
            runs: vec![
                GrassPointsRun { block_x: 0, block_y: 0, param_index: 0, grass: 0, seed_after: 2274, points: vec![[-85, 509], [GRASS_POINT_MAX, 1]] },
                GrassPointsRun { block_x: 0, block_y: 0, param_index: 3, grass: 1, seed_after: -7, points: vec![[GRASS_POINT_MIN, 0]] },
                GrassPointsRun { block_x: 15, block_y: 2, param_index: 23, grass: 0, seed_after: i32::MAX, points: vec![[4000, 4000]] },
            ],
        }
    }

    #[test]
    fn raw_material_bank_reads_from_the_header_alone() {
        let tmp = NamedTempFile::new().unwrap();
        let mut writer = writer();
        writer.add(0, 0, hgts_only_channels()).unwrap();
        writer.finish(tmp.path()).unwrap();
        assert_eq!(read_raw_material_bank(tmp.path()).unwrap(), raw_bank());

        let mut bytes = std::fs::read(tmp.path()).unwrap();
        bytes[4..8].copy_from_slice(&5u32.to_le_bytes());
        std::fs::write(tmp.path(), bytes).unwrap();
        assert!(read_raw_material_bank(tmp.path()).is_err(), "a versioned header is not this format");
    }

    fn grass_channels(grass_points: Option<GrassPointsChunk>) -> CellChannels {
        CellChannels { grass_points, ..hgts_only_channels() }
    }

    #[test]
    fn grass_points_round_trip_byte_exact() {
        let tmp = NamedTempFile::new().unwrap();
        let mut writer = writer();
        writer.add(0, 0, grass_channels(Some(grass_points_chunk()))).unwrap();
        writer.add(1, 2, grass_channels(None)).unwrap();
        writer.finish(tmp.path()).unwrap();

        let reader = Btd4Reader::open(tmp.path()).unwrap();
        assert_eq!(reader.cell(0, 0).unwrap().grass_points, Some(grass_points_chunk()));
        assert_eq!(reader.cell(1, 2).unwrap().grass_points, None);
        assert_eq!(reader.cell(1, 2).unwrap().raw_terrain, Some(raw_cell(1, 2)));

        let encoded = encode_grass_points(&grass_points_chunk());
        // 1 + 2*41 table, 2 run count, 3 runs * 10 header, 4 points * 4.
        assert_eq!(encoded.len(), 1 + 82 + 2 + 30 + 16);
        assert_eq!(&encoded[1..6], &[0, 0x00, 0x09, 0x00, 0x00]);
        assert_eq!(&encoded[6..10], &[94, 0, 57, 0x15]);
        assert_eq!(&encoded[38..42], &0.1f32.to_le_bytes(), "lod_blend closes the 41-byte entry");
        assert_eq!(&encoded[83..85], &3u16.to_le_bytes());
        assert_eq!(&encoded[85..95], &[0, 0, 0, 0, 0xE2, 0x08, 0, 0, 2, 0]);
        assert_eq!(&encoded[95..99], &[0xAB, 0xFF, 0xFD, 0x01], "i16 -85, 509");
    }

    #[test]
    fn invalid_grass_points_are_rejected() {
        let mut writer = writer();
        let reject = |writer: &mut Btd4Writer, edit: &dyn Fn(&mut GrassPointsChunk), what: &str| {
            let mut chunk = grass_points_chunk();
            edit(&mut chunk);
            assert!(writer.add(0, 0, grass_channels(Some(chunk))).is_err(), "{what}");
        };
        reject(&mut writer, &|c| c.runs.swap(0, 1), "unsorted runs");
        reject(&mut writer, &|c| c.runs[1].param_index = 0, "duplicate run key");
        reject(&mut writer, &|c| c.runs[0].points[0][1] = GRASS_POINT_MAX + 1, "point past its block");
        reject(&mut writer, &|c| c.runs[0].points[0][0] = GRASS_POINT_MIN - 1, "point before its block window");
        reject(&mut writer, &|c| c.runs[2].grass = 2, "grass index out of table");
        reject(&mut writer, &|c| c.grasses[1].object_id = 0, "null form");
        reject(&mut writer, &|c| c.grasses[1].plugin_index = 2, "plugin out of table");
        reject(&mut writer, &|c| c.runs[2].block_x = 16, "block out of range");
        reject(&mut writer, &|c| c.runs[2].param_index = 24, "param out of range");
        reject(&mut writer, &|c| c.runs[1].points.clear(), "empty run");
        reject(&mut writer, &|c| c.grasses.clear(), "empty grass table");
        writer.add(0, 0, grass_channels(Some(grass_points_chunk()))).unwrap();

        let mut trailing = encode_grass_points(&grass_points_chunk());
        assert!(decode_grass_points(&trailing, 2).is_ok());
        trailing.push(0);
        assert!(decode_grass_points(&trailing, 2).is_err(), "trailing byte");
        let short = encode_grass_points(&grass_points_chunk());
        assert!(decode_grass_points(&short[..short.len() - 1], 2).is_err(), "truncated point");
    }

    #[test]
    fn compressed_channels_require_stream_end_checksum_and_exact_input() {
        let raw = vec![123; 16641 * 2];
        let encoded = zlib_compress(&raw).unwrap();
        assert_eq!(decompress_bounded(&encoded, raw.len()).unwrap(), raw);
        for missing in 1..=5 { assert!(decompress_bounded(&encoded[..encoded.len()-missing], raw.len()).is_err()); }
        let mut trailing = encoded.clone(); trailing.push(0);
        assert!(decompress_bounded(&trailing, raw.len()).is_err());
        assert!(decompress_bounded(&encoded, raw.len()-1).is_err());
        let mut damaged = encoded.clone(); let last=damaged.len()-1; damaged[last]^=1;
        assert!(decompress_bounded(&damaged, raw.len()).is_err());
    }

    #[test]
    fn round_trip() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        let header = make_header();
        let full = full_channels();
        let hgts_only = hgts_only_channels();

        let mut writer = Btd4Writer::new(header, raw_bank()).unwrap();
        writer.add(0, 0, full.clone()).unwrap();
        writer.add(1, 2, hgts_only.clone()).unwrap();
        writer.finish(path).unwrap();

        let reader = Btd4Reader::open(path).unwrap();

        let h = reader.header();
        assert_eq!(h.density, 128);
        assert_eq!(h.height_min, -2048.0f32);
        assert_eq!(h.height_scale, 0.125f32);
        assert_eq!(h.worldspace_editor_id, "TestWorld");
        assert_eq!(h.plugin_names, vec!["B21_Test.esp", "Fallout4.esm"]);
        assert_eq!(h.cell_min_x, 0);
        assert_eq!(h.cell_min_y, 0);
        assert_eq!(h.cell_max_x, 1);
        assert_eq!(h.cell_max_y, 2);

        // cell (0,0) — all channels present
        let cell00 = reader.cell(0, 0).expect("cell (0,0) should exist");
        let expected_heights = full.heights.as_ref().unwrap();
        assert_eq!(cell00.heights.as_ref().unwrap(), expected_heights);

        let expected_alphas = full.alphas.as_ref().unwrap();
        let got_alphas = cell00.alphas.as_ref().unwrap();
        assert_eq!(got_alphas.len(), expected_alphas.len());
        for (g, e) in got_alphas.iter().zip(expected_alphas.iter()) {
            assert_eq!(g, e);
        }

        let expected_layers = full.layers.as_ref().unwrap();
        let got_layers = cell00.layers.as_ref().unwrap();
        assert_eq!(got_layers.len(), expected_layers.len());
        for (g, e) in got_layers.iter().zip(expected_layers.iter()) {
            assert_eq!(g.plugin_index, e.plugin_index);
            assert_eq!(g.object_id, e.object_id);
            assert_eq!(g.kind, e.kind);
        }

        let expected_gcvr = full.gcvr.as_ref().unwrap();
        let got_gcvr = cell00.gcvr.as_ref().unwrap();
        assert_eq!(got_gcvr.entries.len(), expected_gcvr.entries.len());
        for (g, e) in got_gcvr.entries.iter().zip(expected_gcvr.entries.iter()) {
            assert_eq!(g.plugin_index, e.plugin_index);
            assert_eq!(g.object_id, e.object_id);
            assert_eq!(g.mask, e.mask);
        }

        assert_eq!(
            cell00.colors.as_ref().unwrap(),
            full.colors.as_ref().unwrap()
        );

        // cell (1,2) — heights only
        let cell12 = reader.cell(1, 2).expect("cell (1,2) should exist");
        assert_eq!(
            cell12.heights.as_ref().unwrap(),
            hgts_only.heights.as_ref().unwrap()
        );
        assert!(cell12.alphas.is_none());
        assert!(cell12.layers.is_none());
        assert!(cell12.gcvr.is_none());
        assert!(cell12.colors.is_none());

        // non-existent cell
        assert!(reader.cell(9, 9).is_none());

        let tmp2 = NamedTempFile::new().unwrap();
        let mut writer = Btd4Writer::new(make_header(), raw_bank()).unwrap();
        writer.add(0, 0, full.clone()).unwrap();
        writer.add(1, 2, hgts_only.clone()).unwrap();
        writer.finish(tmp2.path()).unwrap();
        assert_eq!(
            std::fs::read(path).unwrap(),
            std::fs::read(tmp2.path()).unwrap(),
            "two identical writes must produce identical bytes"
        );
    }

    #[test]
    fn reader_rejects_a_bad_header_and_truncated_files() {
        let tmp = NamedTempFile::new().unwrap();
        let mut writer = writer();
        writer.add(0, 0, hgts_only_channels()).unwrap();
        writer.finish(tmp.path()).unwrap();

        let valid = std::fs::read(tmp.path()).unwrap();
        let mut bad = valid.clone();
        bad[4..8].copy_from_slice(&5u32.to_le_bytes());
        std::fs::write(tmp.path(), bad).unwrap();
        assert!(Btd4Reader::open(tmp.path()).is_err());

        std::fs::write(tmp.path(), &valid[..64]).unwrap();
        assert!(Btd4Reader::open(tmp.path()).is_err());
    }

    #[test]
    fn gcvr_preserves_each_form_mask() {
        let tmp = NamedTempFile::new().unwrap();
        let mut first = vec![0u8; 128 * 128];
        first[17] = u8::MAX;
        let mut second = vec![0u8; 128 * 128];
        second[8192] = 73;
        let mut channels = hgts_only_channels();
        channels.gcvr = Some(GcvrChunk {
            entries: vec![
                GcvrEntry {
                    plugin_index: 0,
                    object_id: 0x810,
                    mask: first.clone(),
                },
                GcvrEntry {
                    plugin_index: 1,
                    object_id: 0x1A7,
                    mask: second.clone(),
                },
            ],
        });
        let mut writer = writer();
        writer.add(0, 0, channels).unwrap();
        writer.finish(tmp.path()).unwrap();

        let reader = Btd4Reader::open(tmp.path()).unwrap();
        let entries = reader.cell(0, 0).unwrap().gcvr.unwrap().entries;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].object_id, 0x810);
        assert_eq!(entries[0].mask, first);
        assert_eq!(entries[1].object_id, 0x1A7);
        assert_eq!(entries[1].mask, second);
    }

    #[test]
    fn spill_writer_matches_memory_writer_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        // Terrain/ may not exist yet: finish must create missing parents.
        let memory_path = dir.path().join("Terrain").join("memory.btd4");
        let spill_path = dir.path().join("Terrain").join("spill.btd4");
        let spill = spill_path_for(&spill_path);

        let mut memory = writer();
        let mut streamed = Btd4Writer::with_spill_file(make_header(), raw_bank(), &spill).unwrap();
        for writer in [&mut memory, &mut streamed] {
            // Added out of (y, x) order: the index must still be sorted.
            writer.add(1, 2, hgts_only_channels()).unwrap();
            writer.add(0, 0, full_channels()).unwrap();
        }
        assert!(spill.is_file());
        memory.finish(&memory_path).unwrap();
        streamed.finish(&spill_path).unwrap();

        assert!(!spill.exists());
        assert!(!partial_output_path(&spill_path).exists());
        assert_eq!(std::fs::read(&memory_path).unwrap(), std::fs::read(&spill_path).unwrap());
        let reader = Btd4Reader::open(&spill_path).unwrap();
        assert_eq!(reader.cell(0, 0).unwrap().colors, full_channels().colors);
        assert_eq!(reader.cell(1, 2).unwrap().heights, hgts_only_channels().heights);

        let dropped = spill_path_for(&dir.path().join("w.btd4"));
        {
            let mut writer = Btd4Writer::with_spill_file(make_header(), raw_bank(), &dropped).unwrap();
            writer.add(0, 0, hgts_only_channels()).unwrap();
        }
        assert!(!dropped.exists(), "dropped spill writer removes its spill file");
    }

    #[test]
    fn every_cell_needs_its_rawt_patch() {
        let mut writer = writer();
        assert!(writer.add_cell(0, 0, hgts_only_channels()).unwrap_err().contains("RAWT"));
        let mut channels = hgts_only_channels();
        channels.raw_terrain = Some(raw_cell(1, 0));
        assert!(writer.add_cell(0, 0, channels).is_err(), "patch of another cell");
        assert!(Btd4Writer::new(make_header(), Vec::new()).is_err(), "no bank");
    }

    #[test]
    fn writer_rejects_what_the_reader_would_reject() {
        let mut writer = writer();
        let mut wrong_height_size = hgts_only_channels();
        wrong_height_size.heights = Some(vec![0u16; 100]);
        assert!(writer.add(0, 0, wrong_height_size).is_err(), "HGTS size");
        let wrong_plane_count = CellChannels {
            raw_terrain: None,
            heights: None,
            alphas: Some(vec![vec![0u8; ALPH_PLANE_LEN]; ALPH_PLANE_COUNT - 1]),
            layers: None,
            gcvr: None,
            colors: None,
            grass_points: None,
        };
        assert!(writer.add(0, 0, wrong_plane_count).is_err(), "ALPH plane count");
        writer.add(0, 0, hgts_only_channels()).unwrap();
        assert!(writer.add(0, 0, hgts_only_channels()).is_err(), "duplicate");
        assert!(writer.add(2, 0, hgts_only_channels()).is_err(), "outside bounds");

        let mut no_heights = full_channels();
        no_heights.heights = None;
        assert!(writer.add(1, 0, no_heights).is_err(), "no HGTS");

        let mut alpha_only = hgts_only_channels();
        alpha_only.alphas = Some(synthetic_alpha_planes());
        assert!(writer.add(1, 0, alpha_only).is_err(), "ALPH without LAYR");

        let mut bad_layer = full_channels();
        bad_layer.layers.as_mut().unwrap()[2] = LayerRef {
            plugin_index: 7,
            object_id: 0x10,
            kind: 0,
        };
        assert!(writer.add(1, 0, bad_layer).is_err(), "LAYR plugin out of table");

        let mut bad_grass = full_channels();
        bad_grass.gcvr.as_mut().unwrap().entries[0].object_id = 0;
        assert!(writer.add(1, 0, bad_grass).is_err(), "GCVR null form");
        assert_eq!(writer.cell_count(), 1);
    }
}
