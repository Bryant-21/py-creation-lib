use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

pub const BTD4_VERSION: u32 = 2;

// ---------------------------------------------------------------------------
// Public data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Btd4Header {
    pub version: u32,
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

/// Dense ALPH layout: per cell, `ALPH_PLANE_COUNT` planes = 4 quadrants × 5 FO4
/// percentArrays slots, plane index `quadrant * 5 + slot`. Each plane is a 65×65
/// (`kQuadVerts`²) row-major grid of layer opacity (0..255), indexed `j*65 + i`
/// (i = X / gi, j = Y / gj). Empty slots are all-zero. The base weight is not
/// stored; the consumer derives `base = 1 - sum(slots)`. Slot order matches the
/// LAND's per-quadrant ATXT/VTXT `Layer` order because the dense terrain reuses
/// the vanilla land material (65×65 here vs the LAND's 17×17). `None` when the
/// cell has no alpha.
pub const ALPH_PLANE_VERTS: usize = 65;
pub const ALPH_PLANE_LEN: usize = ALPH_PLANE_VERTS * ALPH_PLANE_VERTS; // 4225
pub const ALPH_PLANE_COUNT: usize = 20; // 4 quadrants × 5 slots

#[derive(Debug, Clone)]
pub struct CellChannels {
    pub heights: Option<Vec<u16>>,    // 129*129 = 16641
    pub alphas: Option<Vec<Vec<u8>>>, // ALPH_PLANE_COUNT × ALPH_PLANE_LEN (see above)
    pub layers: Option<Vec<LayerRef>>,
    pub gcvr: Option<GcvrChunk>,
    pub colors: Option<Vec<u8>>, // 129*129*3 = 49923
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

pub struct Btd4Writer {
    header: Btd4Header,
    cells: Vec<(i32, i32, CellChannels)>,
}

impl Btd4Writer {
    pub fn new(header: Btd4Header) -> Self {
        assert_eq!(
            header.version, BTD4_VERSION,
            "Btd4Writer only emits the v2 contract"
        );
        Self {
            header,
            cells: Vec::new(),
        }
    }

    pub fn add_cell(&mut self, x: i32, y: i32, channels: CellChannels) -> Result<(), String> {
        validate_channels(&channels)?;
        self.cells.push((x, y, channels));
        Ok(())
    }

    pub fn finish(mut self, path: &Path) -> Result<(), String> {
        // Sort cells ascending by (y, x) — matching C++ reader expectation.
        self.cells.sort_by_key(|(x, y, _)| (*y, *x));

        let cell_count = self.cells.len() as u32;

        // --- Pass 1: build header bytes + empty index placeholders ---
        let mut buf: Vec<u8> = Vec::new();

        write_header_bytes(&mut buf, &self.header, cell_count);

        // Cell index starts here; record offset so we can back-patch.
        let index_start = buf.len();

        // Emit placeholder index: x, y, then 5 × (u64 offset + u64 len) = 8 + 5*16 = 88 bytes per cell.
        const ENTRY_BYTES: usize = 8 + 5 * 16; // x(4)+y(4) + 5*(8+8)
        let index_size = self.cells.len() * ENTRY_BYTES;
        buf.resize(index_start + index_size, 0u8);

        // --- Pass 2: emit compressed chunks and record offsets/lengths ---
        // Each cell has up to 5 channel slots: HGTS, ALPH, LAYR, GCVR, CLRS.
        let mut cell_offsets: Vec<[Option<(u64, u64)>; 5]> = Vec::with_capacity(self.cells.len());

        for (_, _, channels) in &self.cells {
            let mut slots: [Option<(u64, u64)>; 5] = [None; 5];

            // Channel 0: HGTS
            if let Some(heights) = &channels.heights {
                let raw = encode_hgts(heights);
                let compressed = zlib_compress(&raw)?;
                let off = buf.len() as u64;
                let len = compressed.len() as u64;
                buf.extend_from_slice(&compressed);
                slots[0] = Some((off, len));
            }
            // Channel 1: ALPH
            if let Some(alphas) = &channels.alphas {
                let raw = encode_alph(alphas);
                let compressed = zlib_compress(&raw)?;
                let off = buf.len() as u64;
                let len = compressed.len() as u64;
                buf.extend_from_slice(&compressed);
                slots[1] = Some((off, len));
            }
            // Channel 2: LAYR
            if let Some(layers) = &channels.layers {
                let raw = encode_layr(layers);
                let compressed = zlib_compress(&raw)?;
                let off = buf.len() as u64;
                let len = compressed.len() as u64;
                buf.extend_from_slice(&compressed);
                slots[2] = Some((off, len));
            }
            // Channel 3: GCVR
            if let Some(gcvr) = &channels.gcvr {
                let raw = encode_gcvr(gcvr);
                let compressed = zlib_compress(&raw)?;
                let off = buf.len() as u64;
                let len = compressed.len() as u64;
                buf.extend_from_slice(&compressed);
                slots[3] = Some((off, len));
            }
            // Channel 4: CLRS
            if let Some(colors) = &channels.colors {
                let compressed = zlib_compress(colors)?;
                let off = buf.len() as u64;
                let len = compressed.len() as u64;
                buf.extend_from_slice(&compressed);
                slots[4] = Some((off, len));
            }

            cell_offsets.push(slots);
        }

        // --- Pass 3: back-patch the cell index ---
        let mut ipos = index_start;
        for (i, (x, y, _)) in self.cells.iter().enumerate() {
            write_i32_le(&mut buf[ipos..ipos + 4], *x);
            ipos += 4;
            write_i32_le(&mut buf[ipos..ipos + 4], *y);
            ipos += 4;
            for slot in &cell_offsets[i] {
                let (off, len) = slot.unwrap_or((0, 0));
                write_u64_le(&mut buf[ipos..ipos + 8], off);
                ipos += 8;
                write_u64_le(&mut buf[ipos..ipos + 8], len);
                ipos += 8;
            }
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("btd4 write: create parent dir: {e}"))?;
        }
        std::fs::write(path, &buf).map_err(|e| format!("btd4 write: {e}"))?;
        Ok(())
    }
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
    channels: [ChannelEntry; 5], // HGTS, ALPH, LAYR, GCVR, CLRS
}

pub struct Btd4Reader {
    header: Btd4Header,
    cells: BTreeMap<(i32, i32), CellIndex>,
    data: Vec<u8>,
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

        let version = read_u32_le(buf, &mut pos)?;
        if version != BTD4_VERSION {
            return Err(format!("btd4: unsupported version {version}"));
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
            return Err("btd4: invalid v2 header".into());
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

        let mut cells: BTreeMap<(i32, i32), CellIndex> = BTreeMap::new();
        for _ in 0..cell_count {
            let cx = read_i32_le(buf, &mut pos)?;
            let cy = read_i32_le(buf, &mut pos)?;
            if cx < cell_min_x || cx > cell_max_x || cy < cell_min_y || cy > cell_max_y {
                return Err("btd4: indexed cell outside header bounds".into());
            }
            let mut channels: [ChannelEntry; 5] =
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
            },
            cells,
            data,
        };
        for (&(x, y), cell) in &reader.cells {
            if reader.decode_hgts(&cell.channels[0]).is_none()
                || (cell.channels[1].len != 0 && reader.decode_alph(&cell.channels[1]).is_none())
                || (cell.channels[2].len != 0 && reader.decode_layr(&cell.channels[2]).is_none())
                || (cell.channels[3].len != 0 && reader.decode_gcvr(&cell.channels[3]).is_none())
                || (cell.channels[4].len != 0 && reader.decode_clrs(&cell.channels[4]).is_none())
                || (cell.channels[1].len != 0 && cell.channels[2].len == 0)
            {
                return Err(format!("btd4: corrupt channel in cell ({x},{y})"));
            }
        }
        Ok(reader)
    }

    pub fn header(&self) -> &Btd4Header {
        &self.header
    }

    pub fn cell(&self, x: i32, y: i32) -> Option<CellChannels> {
        let idx = self.cells.get(&(x, y))?;

        Some(CellChannels {
            heights: self.decode_hgts(&idx.channels[0]),
            alphas: self.decode_alph(&idx.channels[1]),
            layers: self.decode_layr(&idx.channels[2]),
            gcvr: self.decode_gcvr(&idx.channels[3]),
            colors: self.decode_clrs(&idx.channels[4]),
        })
    }

    fn decompress(&self, ch: &ChannelEntry) -> Option<Vec<u8>> {
        if ch.len == 0 {
            return None;
        }
        let start = ch.offset as usize;
        let end = start + ch.len as usize;
        let compressed = &self.data[start..end];
        let mut decoder = ZlibDecoder::new(compressed);
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).ok()?;
        Some(out)
    }

    fn decode_hgts(&self, ch: &ChannelEntry) -> Option<Vec<u16>> {
        if ch.len == 0 {
            return None;
        }
        let raw = self.decompress(ch)?;
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
        let raw = self.decompress(ch)?;
        if raw.is_empty() {
            return None;
        }
        let plane_count = raw[0] as usize;
        let expected = 1 + plane_count * ALPH_PLANE_LEN;
        if plane_count != ALPH_PLANE_COUNT || raw.len() != expected {
            return None;
        }
        let alphas = (0..plane_count)
            .map(|i| {
                let base = 1 + i * ALPH_PLANE_LEN;
                raw[base..base + ALPH_PLANE_LEN].to_vec()
            })
            .collect();
        Some(alphas)
    }

    fn decode_layr(&self, ch: &ChannelEntry) -> Option<Vec<LayerRef>> {
        if ch.len == 0 {
            return None;
        }
        let raw = self.decompress(ch)?;
        if raw.is_empty() {
            return None;
        }
        let row_count = raw[0] as usize;
        const ROW_SIZE: usize = 6;
        let expected = 1 + row_count * ROW_SIZE;
        if row_count != 24 || raw.len() != expected {
            return None;
        }
        let layers: Vec<_> = (0..row_count)
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
        let raw = self.decompress(ch)?;
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
        let raw = self.decompress(ch)?;
        const EXPECTED: usize = 129 * 129 * 3;
        if raw.len() != EXPECTED {
            return None;
        }
        Some(raw)
    }
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
                "dense alpha must be exactly {ALPH_PLANE_COUNT} planes (4 quadrants × 5 slots), got {}",
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
        if layers.len() != 24 {
            return Err(format!(
                "LAYR must contain exactly 24 ordered refs (4 quadrants × 6 slots), got {}",
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

fn zlib_compress(data: &[u8]) -> Result<Vec<u8>, String> {
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
    buf.extend_from_slice(&header.version.to_le_bytes());
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

fn write_i32_le(dst: &mut [u8], v: i32) {
    dst[..4].copy_from_slice(&v.to_le_bytes());
}

fn write_u64_le(dst: &mut [u8], v: u64) {
    dst[..8].copy_from_slice(&v.to_le_bytes());
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
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn make_header() -> Btd4Header {
        Btd4Header {
            version: BTD4_VERSION,
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
        // ALPH_PLANE_COUNT (20) planes of ALPH_PLANE_LEN (4225) u8;
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
            24
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
        }
    }

    fn hgts_only_channels() -> CellChannels {
        CellChannels {
            heights: Some(vec![4096u16; 16641]),
            alphas: None,
            layers: None,
            gcvr: None,
            colors: None,
        }
    }

    #[test]
    fn round_trip() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        let header = make_header();
        let full = full_channels();
        let hgts_only = hgts_only_channels();

        let mut writer = Btd4Writer::new(header);
        writer.add_cell(0, 0, full.clone()).unwrap();
        writer.add_cell(1, 2, hgts_only.clone()).unwrap();
        writer.finish(path).unwrap();

        let reader = Btd4Reader::open(path).unwrap();

        let h = reader.header();
        assert_eq!(h.version, BTD4_VERSION);
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
    }

    /// The terrain phase emits to `mods/<out>/Terrain/<EDID>.btd4` and the
    /// Terrain/ dir may not exist yet — finish must create missing parents.
    #[test]
    fn finish_creates_missing_parent_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("Terrain").join("APPALACHIA.btd4");
        let mut writer = Btd4Writer::new(make_header());
        writer.add_cell(0, 0, hgts_only_channels()).unwrap();
        writer.finish(&path).unwrap();
        assert!(path.is_file());
    }

    #[test]
    fn validation_wrong_height_size() {
        let mut writer = Btd4Writer::new(make_header());
        let ch = CellChannels {
            heights: Some(vec![0u16; 100]), // wrong size
            alphas: None,
            layers: None,
            gcvr: None,
            colors: None,
        };
        assert!(writer.add_cell(0, 0, ch).is_err());
    }

    #[test]
    fn validation_wrong_alpha_plane_count() {
        let mut writer = Btd4Writer::new(make_header());
        let ch = CellChannels {
            heights: None,
            alphas: Some(vec![vec![0u8; ALPH_PLANE_LEN]; 19]), // 19 != ALPH_PLANE_COUNT (20)
            layers: None,
            gcvr: None,
            colors: None,
        };
        assert!(writer.add_cell(0, 0, ch).is_err());
    }

    #[test]
    fn reader_rejects_v1_and_truncated_files() {
        let tmp = NamedTempFile::new().unwrap();
        let mut writer = Btd4Writer::new(make_header());
        writer.add_cell(0, 0, hgts_only_channels()).unwrap();
        writer.finish(tmp.path()).unwrap();

        let valid = std::fs::read(tmp.path()).unwrap();
        let mut v1 = valid.clone();
        v1[4..8].copy_from_slice(&1u32.to_le_bytes());
        std::fs::write(tmp.path(), v1).unwrap();
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
        let mut writer = Btd4Writer::new(make_header());
        writer.add_cell(0, 0, channels).unwrap();
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
    fn determinism() {
        let tmp1 = NamedTempFile::new().unwrap();
        let tmp2 = NamedTempFile::new().unwrap();

        let write = |path: &std::path::Path| {
            let mut writer = Btd4Writer::new(make_header());
            writer.add_cell(0, 0, full_channels()).unwrap();
            writer.add_cell(1, 2, hgts_only_channels()).unwrap();
            writer.finish(path).unwrap();
            std::fs::read(path).unwrap()
        };

        let b1 = write(tmp1.path());
        let b2 = write(tmp2.path());
        assert_eq!(b1, b2, "two identical writes must produce identical bytes");
    }

    // ---------------------------------------------------------------------------
    // Cross-language fixture generator for the C++ host-test consumer.
    // Run with: cargo test -p terrain_native -- btd4::tests::write_cpp_fixture --ignored
    // ---------------------------------------------------------------------------

    /// Writes a deterministic 2-cell .btd4 fixture for C++ host tests.
    ///
    /// # Exact documented values (the C++ host test must mirror these constants)
    ///
    /// Header:
    ///   version = 2, density = 128
    ///   height_min = 0.0_f32, height_scale = 0.5_f32
    ///   worldspace_editor_id = "B21TestWorld"
    ///   plugin_names = ["B21_Test.esp", "Fallout4.esm"]   (index 0 = producer)
    ///   cell bounds: min(0,0) max(1,1)
    ///   cell_count = 2
    ///
    /// Cell (0, 0) — all 5 channels:
    ///   HGTS: heights[i] = (i % 1000) as u16  for i in 0..16641
    ///   ALPH: ALPH_PLANE_COUNT (20) planes of ALPH_PLANE_LEN (4225) u8
    ///     plane p, texel k: ((p*31 + k) % 256) as u8   (p in 0..20, k in 0..4225)
    ///   LAYR: 24 ordered rows (4 quadrants × base+5 alpha); first two populated
    ///     row 0: plugin_index=0, object_id=0x000800, kind=0
    ///     row 1: plugin_index=1, object_id=0x0001A7, kind=0
    ///   GCVR: one entry: plugin_index=0, object_id=0x000810,
    ///         mask[i] = (i % 251) as u8  for i in 0..16384
    ///   CLRS: colors[i] = (i % 256) as u8  for i in 0..49923
    ///
    /// Cell (1, 1) — HGTS only:
    ///   HGTS: heights[i] = 4096  (constant, all elements)
    #[cfg(test)]
    fn write_synthetic_two_cell_file(path: &Path) {
        let header = Btd4Header {
            version: BTD4_VERSION,
            density: 128,
            height_min: 0.0,
            height_scale: 0.5,
            worldspace_editor_id: "B21TestWorld".into(),
            plugin_names: vec!["B21_Test.esp".into(), "Fallout4.esm".into()],
            cell_min_x: 0,
            cell_min_y: 0,
            cell_max_x: 1,
            cell_max_y: 1,
        };

        let mut layers = vec![
            LayerRef {
                plugin_index: u8::MAX,
                object_id: 0,
                kind: 0,
            };
            24
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
        let cell00 = CellChannels {
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
        };

        let cell11 = CellChannels {
            heights: Some(vec![4096u16; 16641]),
            alphas: None,
            layers: None,
            gcvr: None,
            colors: None,
        };

        let mut writer = Btd4Writer::new(header);
        writer.add_cell(0, 0, cell00).unwrap();
        writer.add_cell(1, 1, cell11).unwrap();
        writer.finish(path).unwrap();
    }

    #[test]
    #[ignore = "writes the cross-language fixture for B21_SmoothTerrain host tests"]
    fn write_cpp_fixture() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../mods/B21_SmoothTerrain/tests/fixtures/mini_v2.btd4");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_synthetic_two_cell_file(&path);
    }
}
