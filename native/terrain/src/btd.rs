use flate2::read::ZlibDecoder;
use memmap2::MmapOptions;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Read;

const HEADER_LEN: usize = 0x2c;
const CELL_SAMPLES: usize = 128;
const TILE_CELLS: usize = 8;
const TILE_SAMPLE_WIDTH: usize = TILE_CELLS * CELL_SAMPLES;
const TILE_SAMPLE_COUNT: usize = TILE_SAMPLE_WIDTH * TILE_SAMPLE_WIDTH;
const TILE_VERTEX_COLOR_WIDTH: usize = TILE_CELLS * 32;
const TILE_VERTEX_COLOR_SAMPLE_COUNT: usize = TILE_VERTEX_COLOR_WIDTH * TILE_VERTEX_COLOR_WIDTH;
const ZLIB_ENTRY_LEN: usize = 8;
const HEIGHT_LAND_BLOCK_LEN: usize = 49152;
const STARFIELD_HEIGHT_LAND_BLOCK_LEN: usize = 65536;
const GROUND_COVER_BLOCK_LEN: usize = 16384;
const VERTEX_COLOR_BLOCK_LEN: usize = 32768;

#[derive(Debug, thiserror::Error)]
pub enum BtdError {
    #[error("BTD file is too small")]
    Truncated,
    #[error("input file format is not BTD")]
    BadMagic,
    #[error("unsupported BTD format version {0}")]
    UnsupportedVersion(u32),
    #[error("requested cell ({0}, {1}) is outside BTD bounds")]
    CellOutOfBounds(i32, i32),
    #[error("unsupported BTD lod {0}")]
    UnsupportedLod(u8),
    #[error("zlib decode failed: {0}")]
    Zlib(String),
    #[error("BTD offset is outside file: {0}")]
    BadOffset(usize),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BtdHeaderReport {
    pub magic: String,
    pub version: u32,
    pub world_height_min: f32,
    pub world_height_max: f32,
    pub resolution_x: u32,
    pub resolution_y: u32,
    pub cell_min_x: i32,
    pub cell_min_y: i32,
    pub cell_max_x: i32,
    pub cell_max_y: i32,
    pub ltex_count: u32,
}

#[derive(Debug, Clone)]
pub struct BtdHeader {
    pub version: u32,
    pub world_height_min: f32,
    pub world_height_max: f32,
    pub resolution_x: u32,
    pub resolution_y: u32,
    pub cell_min_x: i32,
    pub cell_min_y: i32,
    pub cell_max_x: i32,
    pub cell_max_y: i32,
    pub cells_x: usize,
    pub cells_y: usize,
    pub ltex_count: usize,
    pub ltex_offset: usize,
    pub cell_height_minmax_offset: usize,
    pub ltex_map_offset: usize,
    pub gcvr_count: usize,
    pub gcvr_offset: usize,
    pub gcvr_map_offset: usize,
    pub height_lod4_offset: usize,
    pub land_texture_lod4_offset: usize,
    pub vertex_color_lod4_offset: usize,
    pub zlib_table_offset: usize,
    pub zlib_lod3_offset: usize,
    pub zlib_lod2_offset: usize,
    pub zlib_lod1_offset: usize,
    pub zlib_lod0_offset: usize,
    pub zlib_data_offset: usize,
    pub is_starfield_layout: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellTextureSet {
    pub quadrants: Vec<QuadrantTextureSet>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuadrantTextureSet {
    pub base: Option<u8>,
    pub base_source_slot: Option<u8>,
    pub additional: [Option<u8>; 5],
    pub additional_source_slots: [Option<u8>; 5],
    pub ground_cover: [Option<u8>; 8],
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct TileData {
    x0: u16,
    y0: u16,
    block_mask: u32,
    heights: Vec<u16>,
    land_alphas: Vec<u16>,
    ground_cover: Vec<u8>,
    vertex_color: Vec<u16>,
}

pub struct BtdFile {
    bytes: memmap2::Mmap,
    header: BtdHeader,
    ltex_form_ids: Vec<u32>,
    gcvr_form_ids: Vec<u32>,
    tile_cache: HashMap<u32, TileData>,
    tile_cache_hits: u64,
    tile_cache_misses: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BtdTileCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub cached_tiles: u64,
    pub cached_payload_bytes: u64,
    pub cached_height_payload_bytes: u64,
    pub cached_land_alpha_payload_bytes: u64,
    pub cached_ground_cover_payload_bytes: u64,
    pub cached_vertex_color_payload_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
enum ExtractKind {
    Height,
    LandAlpha,
    TerrainColor,
}

impl BtdHeader {
    pub fn to_report(&self) -> BtdHeaderReport {
        BtdHeaderReport {
            magic: "BTDB".to_owned(),
            version: self.version,
            world_height_min: self.world_height_min,
            world_height_max: self.world_height_max,
            resolution_x: self.resolution_x,
            resolution_y: self.resolution_y,
            cell_min_x: self.cell_min_x,
            cell_min_y: self.cell_min_y,
            cell_max_x: self.cell_max_x,
            cell_max_y: self.cell_max_y,
            ltex_count: self.ltex_count as u32,
        }
    }
}

impl BtdFile {
    pub fn open_header(path: &str) -> Result<BtdHeader, BtdError> {
        let bytes = fs::read(path).map_err(|_| BtdError::Truncated)?;
        Self::parse_header(&bytes)
    }

    pub fn open(path: &str) -> Result<Self, BtdError> {
        let file = fs::File::open(path).map_err(|_| BtdError::Truncated)?;
        // SAFETY: we never mutate the mapped region and hold the file open
        // for the lifetime of the mapping.
        let bytes = unsafe { MmapOptions::new().map(&file) }.map_err(|_| BtdError::Truncated)?;
        let header = Self::parse_header(&bytes)?;
        let mut ltex_form_ids = Vec::with_capacity(header.ltex_count);
        for index in 0..header.ltex_count {
            ltex_form_ids.push(read_u32(&bytes, header.ltex_offset + index * 4)?);
        }
        let mut gcvr_form_ids = Vec::with_capacity(header.gcvr_count);
        for index in 0..header.gcvr_count {
            gcvr_form_ids.push(read_u32(&bytes, header.gcvr_offset + index * 4)?);
        }
        Ok(Self {
            bytes,
            header,
            ltex_form_ids,
            gcvr_form_ids,
            tile_cache: HashMap::new(),
            tile_cache_hits: 0,
            tile_cache_misses: 0,
        })
    }

    pub fn header(&self) -> &BtdHeader {
        &self.header
    }

    pub fn tile_cache_stats(&self) -> BtdTileCacheStats {
        let cached_height_payload_bytes = self
            .tile_cache
            .values()
            .map(|tile| tile.heights.len() * std::mem::size_of::<u16>())
            .sum::<usize>();
        let cached_land_alpha_payload_bytes = self
            .tile_cache
            .values()
            .map(|tile| tile.land_alphas.len() * std::mem::size_of::<u16>())
            .sum::<usize>();
        let cached_ground_cover_payload_bytes = self
            .tile_cache
            .values()
            .map(|tile| tile.ground_cover.len())
            .sum::<usize>();
        let cached_vertex_color_payload_bytes = self
            .tile_cache
            .values()
            .map(|tile| tile.vertex_color.len() * std::mem::size_of::<u16>())
            .sum::<usize>();
        let cached_payload_bytes = cached_height_payload_bytes
            + cached_land_alpha_payload_bytes
            + cached_ground_cover_payload_bytes
            + cached_vertex_color_payload_bytes;
        BtdTileCacheStats {
            hits: self.tile_cache_hits,
            misses: self.tile_cache_misses,
            cached_tiles: self.tile_cache.len() as u64,
            cached_payload_bytes: cached_payload_bytes as u64,
            cached_height_payload_bytes: cached_height_payload_bytes as u64,
            cached_land_alpha_payload_bytes: cached_land_alpha_payload_bytes as u64,
            cached_ground_cover_payload_bytes: cached_ground_cover_payload_bytes as u64,
            cached_vertex_color_payload_bytes: cached_vertex_color_payload_bytes as u64,
        }
    }

    pub(crate) fn release_land_alpha_payloads(&mut self) -> u64 {
        self.tile_cache
            .values_mut()
            .map(|tile| {
                let released = tile.land_alphas.len() * std::mem::size_of::<u16>();
                drop(std::mem::take(&mut tile.land_alphas));
                released as u64
            })
            .sum()
    }

    pub fn land_texture_form_id(&self, index: usize) -> Option<u32> {
        self.ltex_form_ids.get(index).copied()
    }

    pub fn ground_cover_form_id(&self, index: usize) -> Option<u32> {
        self.gcvr_form_ids.get(index).copied()
    }

    pub fn cell_height_map_u16(
        &mut self,
        cell_x: i32,
        cell_y: i32,
        lod: u8,
    ) -> Result<Vec<u16>, BtdError> {
        self.extract_cell_u16(cell_x, cell_y, lod, ExtractKind::Height)
    }

    pub fn cell_land_alpha_u16(
        &mut self,
        cell_x: i32,
        cell_y: i32,
        lod: u8,
    ) -> Result<Vec<u16>, BtdError> {
        self.extract_cell_u16(cell_x, cell_y, lod, ExtractKind::LandAlpha)
    }

    pub fn cell_terrain_color_u16(
        &mut self,
        cell_x: i32,
        cell_y: i32,
        lod: u8,
    ) -> Result<Vec<u16>, BtdError> {
        self.extract_cell_u16(cell_x, cell_y, lod, ExtractKind::TerrainColor)
    }

    pub fn cell_texture_set(&self, cell_x: i32, cell_y: i32) -> Result<CellTextureSet, BtdError> {
        self.assert_cell_in_bounds(cell_x, cell_y)?;
        let x = usize::try_from(cell_x - self.header.cell_min_x).unwrap();
        let y = usize::try_from(cell_y - self.header.cell_min_y).unwrap();
        let mut quadrants = Vec::with_capacity(4);
        for q in 0..4usize {
            let offset =
                ((((y << 1) | (q >> 1)) * (self.header.cells_x << 1) + ((x << 1) | (q & 1))) << 3)
                    + self.header.ltex_map_offset;
            let mut slots = [None; 6];
            let mut slot_source_slots = [None; 6];
            let mut ground_cover = [None; 8];
            for i in 0..8usize {
                let raw = read_u8(&self.bytes, offset + i)?;
                let decoded = decode_reversed_index(raw, self.header.ltex_count);
                if i < 5 {
                    slots[5 - i] = decoded;
                    slot_source_slots[5 - i] = decoded.map(|_| i as u8);
                }
                if decoded.is_some() {
                    slots[0] = decoded;
                    slot_source_slots[0] = Some(i as u8);
                }
            }
            let gcvr_offset =
                ((((y << 1) | (q >> 1)) * (self.header.cells_x << 1) + ((x << 1) | (q & 1))) << 3)
                    + self.header.gcvr_map_offset;
            for (i, slot) in ground_cover.iter_mut().enumerate() {
                let raw = read_u8(&self.bytes, gcvr_offset + i)?;
                *slot = decode_direct_index(raw, self.header.gcvr_count);
            }
            quadrants.push(QuadrantTextureSet {
                base: slots[0],
                base_source_slot: slot_source_slots[0],
                additional: [slots[1], slots[2], slots[3], slots[4], slots[5]],
                additional_source_slots: [
                    slot_source_slots[1],
                    slot_source_slots[2],
                    slot_source_slots[3],
                    slot_source_slots[4],
                    slot_source_slots[5],
                ],
                ground_cover,
            });
        }
        Ok(CellTextureSet { quadrants })
    }

    pub fn cell_ground_cover_mask_u8(
        &self,
        cell_x: i32,
        cell_y: i32,
        lod: u8,
    ) -> Result<Vec<u8>, BtdError> {
        if lod > 4 {
            return Err(BtdError::UnsupportedLod(lod));
        }
        self.assert_cell_in_bounds(cell_x, cell_y)?;
        let samples = CELL_SAMPLES >> lod;
        if self.header.gcvr_count == 0 {
            return Ok(vec![0; samples * samples]);
        }

        let x = usize::try_from(cell_x - self.header.cell_min_x).unwrap();
        let y = usize::try_from(cell_y - self.header.cell_min_y).unwrap();
        let block_index = y * self.header.cells_x + x;
        let block = self.read_zlib_block_len(0, block_index, 1, GROUND_COVER_BLOCK_LEN)?;
        let step = 1usize << lod;
        let half = samples >> 1;
        let candidate_masks = [
            self.ground_cover_candidate_mask(cell_x, cell_y, 0)?,
            self.ground_cover_candidate_mask(cell_x, cell_y, 1)?,
            self.ground_cover_candidate_mask(cell_x, cell_y, 2)?,
            self.ground_cover_candidate_mask(cell_x, cell_y, 3)?,
        ];
        let mut result = Vec::with_capacity(samples * samples);
        for row in 0..samples {
            for column in 0..samples {
                let quadrant = u8::from(column >= half) | (u8::from(row >= half) << 1);
                let source = block[(row * step) * CELL_SAMPLES + column * step];
                let mask =
                    reorder_ground_cover_bits(source) & candidate_masks[usize::from(quadrant)];
                result.push(mask);
            }
        }
        Ok(result)
    }

    fn parse_header(bytes: &[u8]) -> Result<BtdHeader, BtdError> {
        if bytes.len() < HEADER_LEN {
            return Err(BtdError::Truncated);
        }
        if bytes.get(0..4) != Some(b"BTDB") {
            return Err(BtdError::BadMagic);
        }

        let version = read_u32(bytes, 0x04)?;
        if version != 5 && version != 6 {
            return Err(BtdError::UnsupportedVersion(version));
        }

        let world_height_min = read_f32(bytes, 0x08)?;
        let world_height_max = read_f32(bytes, 0x0c)?;
        let resolution_x = read_u32(bytes, 0x10)?;
        let resolution_y = read_u32(bytes, 0x14)?;
        let mut cell_min_x = read_i32(bytes, 0x18)?;
        let mut cell_min_y = read_i32(bytes, 0x1c)?;
        let mut cell_max_x = read_i32(bytes, 0x20)?;
        let mut cell_max_y = read_i32(bytes, 0x24)?;

        // Starfield BTD reuses the FO76 BTDB container (same version numbers)
        // but zeroes the SW/NE cell coordinates and carries no ground-cover
        // or vertex-color sections; real cell bounds are derived from the
        // height-map resolution instead. `resolution_x/y > 128` (more than a
        // single cell) rules out a degenerate one-cell FO76 world that would
        // otherwise also read as all-zero bounds. Ground truth:
        // refs/fo76utils-main/src/btdfile.cpp (BTDFile::BTDFile, isStarfieldBTD).
        let is_starfield_layout = cell_min_x == 0
            && cell_min_y == 0
            && cell_max_x == 0
            && cell_max_y == 0
            && resolution_x > 128
            && resolution_y > 128;
        if is_starfield_layout {
            // No x8 here: unlike fo76utils' `worldHeightMin *= 8.0f` (its own
            // `FIXME`), Starfield's header floats are already world units, as
            // placed-ref Z ranges, WRLD LandData and slope plausibility confirm.
            cell_min_x =
                -(i32::try_from(resolution_x >> 8).map_err(|_| BtdError::BadOffset(0x10))?);
            cell_min_y =
                -(i32::try_from(resolution_y >> 8).map_err(|_| BtdError::BadOffset(0x14))?);
            cell_max_x = cell_min_x
                + i32::try_from(resolution_x >> 7).map_err(|_| BtdError::BadOffset(0x10))?
                - 1;
            cell_max_y = cell_min_y
                + i32::try_from(resolution_y >> 7).map_err(|_| BtdError::BadOffset(0x14))?
                - 1;
        }

        let ltex_count =
            usize::try_from(read_u32(bytes, 0x28)?).map_err(|_| BtdError::BadOffset(0x28))?;
        let cells_x = checked_cell_span(cell_min_x, cell_max_x, 0x20)?;
        let cells_y = checked_cell_span(cell_min_y, cell_max_y, 0x24)?;
        let cell_count = checked_mul(cells_x, cells_y, HEADER_LEN)?;

        let ltex_offset = HEADER_LEN;
        let cell_height_minmax_offset =
            checked_add(ltex_offset, checked_mul(ltex_count, 4, ltex_offset)?)?;
        let ltex_map_offset = checked_add(
            cell_height_minmax_offset,
            checked_mul(cell_count, 8, cell_height_minmax_offset)?,
        )?;

        // Starfield BTD carries no GCVR section at all (ground cover isn't
        // baked into terrain there); LOD4 height data starts right after the
        // LTEX quadrant map instead of after a gcvr count/table/map block.
        let (gcvr_count, gcvr_offset, gcvr_map_offset, height_lod4_offset) = if is_starfield_layout
        {
            let after_ltex_map = checked_add(
                ltex_map_offset,
                checked_mul(cell_count, 32, ltex_map_offset)?,
            )?;
            (0usize, ltex_offset, ltex_offset, after_ltex_map)
        } else {
            let gcvr_count_offset = checked_add(
                ltex_map_offset,
                checked_mul(cell_count, 32, ltex_map_offset)?,
            )?;
            let gcvr_count = usize::try_from(read_u32(bytes, gcvr_count_offset)?)
                .map_err(|_| BtdError::BadOffset(gcvr_count_offset))?;
            let gcvr_offset = checked_add(gcvr_count_offset, 4)?;
            let gcvr_map_offset =
                checked_add(gcvr_offset, checked_mul(gcvr_count, 4, gcvr_offset)?)?;
            let after_gcvr_map = checked_add(
                gcvr_map_offset,
                checked_mul(cell_count, 32, gcvr_map_offset)?,
            )?;
            (gcvr_count, gcvr_offset, gcvr_map_offset, after_gcvr_map)
        };

        let land_texture_lod4_offset = checked_add(
            height_lod4_offset,
            checked_mul(cell_count, 128, height_lod4_offset)?,
        )?;

        // Starfield BTD also carries no vertex-color LOD4 section; the ZLib
        // block table follows the land-texture LOD4 data directly.
        let (vertex_color_lod4_offset, zlib_table_offset) = if is_starfield_layout {
            let zlib_table_offset = checked_add(
                land_texture_lod4_offset,
                checked_mul(cell_count, 128, land_texture_lod4_offset)?,
            )?;
            (land_texture_lod4_offset, zlib_table_offset)
        } else {
            let vertex_color_lod4_offset = checked_add(
                land_texture_lod4_offset,
                checked_mul(cell_count, 128, land_texture_lod4_offset)?,
            )?;
            let zlib_table_offset = checked_add(
                vertex_color_lod4_offset,
                checked_mul(cell_count, 128, vertex_color_lod4_offset)?,
            )?;
            (vertex_color_lod4_offset, zlib_table_offset)
        };

        // Starfield ZLib block tables hold one entry per block at every LOD
        // (no ground-cover/vertex-color second stream to interleave).
        let zlib_lod3_offset = zlib_table_offset;
        let zlib_lod3_block_count =
            checked_mul(cells_y.div_ceil(8), cells_x.div_ceil(8), zlib_lod3_offset)?;
        let zlib_lod3_entry_count = if is_starfield_layout {
            zlib_lod3_block_count
        } else {
            checked_mul(zlib_lod3_block_count, 2, zlib_lod3_offset)?
        };
        let zlib_lod2_offset = checked_add(
            zlib_lod3_offset,
            checked_mul(zlib_lod3_entry_count, ZLIB_ENTRY_LEN, zlib_lod3_offset)?,
        )?;
        let zlib_lod2_block_count =
            checked_mul(cells_y.div_ceil(4), cells_x.div_ceil(4), zlib_lod2_offset)?;
        let zlib_lod2_entry_count = if is_starfield_layout {
            zlib_lod2_block_count
        } else {
            checked_mul(zlib_lod2_block_count, 2, zlib_lod2_offset)?
        };
        let zlib_lod1_offset = checked_add(
            zlib_lod2_offset,
            checked_mul(zlib_lod2_entry_count, ZLIB_ENTRY_LEN, zlib_lod2_offset)?,
        )?;
        let zlib_lod1_count =
            checked_mul(cells_y.div_ceil(2), cells_x.div_ceil(2), zlib_lod1_offset)?;
        let zlib_lod0_offset = checked_add(
            zlib_lod1_offset,
            checked_mul(zlib_lod1_count, ZLIB_ENTRY_LEN, zlib_lod1_offset)?,
        )?;
        let zlib_lod0_entry_count = if is_starfield_layout {
            cell_count
        } else {
            checked_mul(cell_count, 2, zlib_lod0_offset)?
        };
        let zlib_data_offset = checked_add(
            zlib_lod0_offset,
            checked_mul(zlib_lod0_entry_count, ZLIB_ENTRY_LEN, zlib_lod0_offset)?,
        )?;
        ensure_range(bytes, zlib_data_offset, 0)?;

        Ok(BtdHeader {
            version,
            world_height_min,
            world_height_max,
            resolution_x,
            resolution_y,
            cell_min_x,
            cell_min_y,
            cell_max_x,
            cell_max_y,
            cells_x,
            cells_y,
            ltex_count,
            ltex_offset,
            cell_height_minmax_offset,
            ltex_map_offset,
            gcvr_count,
            gcvr_offset,
            gcvr_map_offset,
            height_lod4_offset,
            land_texture_lod4_offset,
            vertex_color_lod4_offset,
            zlib_table_offset,
            zlib_lod3_offset,
            zlib_lod2_offset,
            zlib_lod1_offset,
            zlib_lod0_offset,
            zlib_data_offset,
            is_starfield_layout,
        })
    }

    fn assert_cell_in_bounds(&self, cell_x: i32, cell_y: i32) -> Result<(), BtdError> {
        if cell_x < self.header.cell_min_x
            || cell_x > self.header.cell_max_x
            || cell_y < self.header.cell_min_y
            || cell_y > self.header.cell_max_y
        {
            return Err(BtdError::CellOutOfBounds(cell_x, cell_y));
        }
        Ok(())
    }

    fn extract_cell_u16(
        &mut self,
        cell_x: i32,
        cell_y: i32,
        lod: u8,
        kind: ExtractKind,
    ) -> Result<Vec<u16>, BtdError> {
        if lod > 4 {
            return Err(BtdError::UnsupportedLod(lod));
        }
        self.assert_cell_in_bounds(cell_x, cell_y)?;
        let x = usize::try_from(cell_x - self.header.cell_min_x).unwrap();
        let y = usize::try_from(cell_y - self.header.cell_min_y).unwrap();
        let tile_x = x / TILE_CELLS;
        let tile_y = y / TILE_CELLS;
        let local_x = x % TILE_CELLS;
        let local_y = y % TILE_CELLS;
        match kind {
            ExtractKind::Height | ExtractKind::LandAlpha => {
                let tile = self.load_tile(tile_x, tile_y, lod, kind)?;
                let source = match kind {
                    ExtractKind::Height => &tile.heights,
                    ExtractKind::LandAlpha => &tile.land_alphas,
                    ExtractKind::TerrainColor => unreachable!(),
                };
                if source.len() < TILE_SAMPLE_COUNT {
                    return Err(BtdError::BadOffset(source.len()));
                }

                let step = 1usize << lod;
                let samples = CELL_SAMPLES >> lod;
                let sample_x = local_x * CELL_SAMPLES;
                let sample_y = local_y * CELL_SAMPLES;
                let mut result = Vec::with_capacity(samples * samples);
                for row in 0..samples {
                    let row_start = (sample_y + row * step) * TILE_SAMPLE_WIDTH + sample_x;
                    for column in 0..samples {
                        result.push(source[row_start + column * step]);
                    }
                }
                if matches!(kind, ExtractKind::LandAlpha) {
                    reorder_land_alpha_bits(&mut result);
                }
                Ok(result)
            }
            ExtractKind::TerrainColor => {
                let tile = self.load_tile(tile_x, tile_y, lod.max(2), kind)?;
                if tile.vertex_color.len() < TILE_VERTEX_COLOR_SAMPLE_COUNT {
                    return Err(BtdError::BadOffset(tile.vertex_color.len()));
                }
                let samples = CELL_SAMPLES >> lod;
                let sample_x = local_x * 32;
                let sample_y = local_y * 32;
                let mut result = Vec::with_capacity(samples * samples);
                for row in 0..samples {
                    let y = if lod >= 2 {
                        row << (usize::from(lod) - 2)
                    } else {
                        row >> (2 - usize::from(lod))
                    };
                    let row_start = (sample_y + y) * TILE_VERTEX_COLOR_WIDTH + sample_x;
                    for column in 0..samples {
                        let x = if lod >= 2 {
                            column << (usize::from(lod) - 2)
                        } else {
                            column >> (2 - usize::from(lod))
                        };
                        result.push(tile.vertex_color[row_start + x]);
                    }
                }
                Ok(result)
            }
        }
    }

    fn load_tile(
        &mut self,
        tile_x: usize,
        tile_y: usize,
        lod: u8,
        kind: ExtractKind,
    ) -> Result<&TileData, BtdError> {
        let tiles_x = self.header.cells_x.div_ceil(TILE_CELLS);
        let tiles_y = self.header.cells_y.div_ceil(TILE_CELLS);
        if tile_x >= tiles_x || tile_y >= tiles_y {
            return Err(BtdError::BadOffset(tile_y * tiles_x + tile_x));
        }
        let tile_index = tile_y * tiles_x + tile_x;
        let kind_key = match kind {
            ExtractKind::Height | ExtractKind::LandAlpha => 0u32,
            ExtractKind::TerrainColor => 1u32,
        };
        let cache_key =
            (kind_key << 28) | ((u32::from(lod)) << 24) | u32::try_from(tile_index).unwrap();
        let cache_hit = self
            .tile_cache
            .get(&cache_key)
            .is_some_and(|tile| match kind {
                ExtractKind::Height => tile.heights.len() >= TILE_SAMPLE_COUNT,
                ExtractKind::LandAlpha => tile.land_alphas.len() >= TILE_SAMPLE_COUNT,
                ExtractKind::TerrainColor => {
                    tile.vertex_color.len() >= TILE_VERTEX_COLOR_SAMPLE_COUNT
                }
            });
        if !cache_hit {
            self.tile_cache_misses = self.tile_cache_misses.saturating_add(1);
            let mut decoded = self.read_tile(tile_x, tile_y, lod, kind)?;
            if let Some(tile) = self.tile_cache.get_mut(&cache_key) {
                match kind {
                    ExtractKind::Height => {
                        tile.heights = std::mem::take(&mut decoded.heights);
                    }
                    ExtractKind::LandAlpha => {
                        tile.land_alphas = std::mem::take(&mut decoded.land_alphas);
                    }
                    ExtractKind::TerrainColor => {
                        tile.vertex_color = std::mem::take(&mut decoded.vertex_color);
                    }
                }
            } else {
                self.tile_cache.insert(cache_key, decoded);
            }
        } else {
            self.tile_cache_hits = self.tile_cache_hits.saturating_add(1);
        }
        Ok(self.tile_cache.get(&cache_key).unwrap())
    }

    fn read_tile(
        &self,
        tile_x: usize,
        tile_y: usize,
        lod: u8,
        kind: ExtractKind,
    ) -> Result<TileData, BtdError> {
        if lod > 4 {
            return Err(BtdError::UnsupportedLod(lod));
        }
        let x0 = tile_x * TILE_CELLS;
        let y0 = tile_y * TILE_CELLS;
        let height_land = matches!(kind, ExtractKind::Height | ExtractKind::LandAlpha);
        let mut tile = TileData {
            x0: u16::try_from(x0).unwrap_or(u16::MAX),
            y0: u16::try_from(y0).unwrap_or(u16::MAX),
            block_mask: 0,
            heights: if height_land {
                vec![0; TILE_SAMPLE_COUNT]
            } else {
                Vec::new()
            },
            land_alphas: if height_land {
                vec![0; TILE_SAMPLE_COUNT]
            } else {
                Vec::new()
            },
            ground_cover: Vec::new(),
            vertex_color: if matches!(kind, ExtractKind::TerrainColor) {
                vec![0; TILE_VERTEX_COLOR_SAMPLE_COUNT]
            } else {
                Vec::new()
            },
        };

        if matches!(kind, ExtractKind::TerrainColor) {
            if self.header.is_starfield_layout {
                // Starfield BTD bakes no per-vertex terrain color at all;
                // report the "no data" sentinel instead of reading a
                // vertex-color section that does not exist in the file.
                tile.vertex_color.fill(0xFFFF);
                tile.block_mask = 0x02A0;
                return Ok(tile);
            }
            self.load_lod4_vertex_color(&mut tile, x0, y0)?;
            for block_lod in (lod.max(2)..4u8).rev() {
                let lod_scale = 1usize << block_lod;
                let block_width = 8usize >> block_lod;
                let row_width = self.header.cells_x.div_ceil(lod_scale);
                let row_count = self.header.cells_y.div_ceil(lod_scale);
                for yy in 0..block_width {
                    let block_y = (y0 >> block_lod) + yy;
                    if block_y >= row_count {
                        break;
                    }
                    for xx in 0..block_width {
                        let block_x = (x0 >> block_lod) + xx;
                        if block_x >= row_width {
                            break;
                        }
                        let block_index = block_y * row_width + block_x;
                        let data_offset = ((yy << 8) + xx) << (usize::from(block_lod) + 5);
                        let decoded = self.read_zlib_block_len(
                            block_lod,
                            block_index,
                            1,
                            VERTEX_COLOR_BLOCK_LEN,
                        )?;
                        load_vertex_color_block(
                            &mut tile.vertex_color,
                            data_offset,
                            block_lod,
                            &decoded,
                        )?;
                    }
                }
            }
            tile.block_mask = 0x02A0;
            return Ok(tile);
        }

        self.load_lod4_height_land(&mut tile, x0, y0)?;
        // FO76 blocks are progressive refinements (only samples absent from
        // coarser LODs are stored), so the tile must be assembled LOD3->LOD0.
        // Starfield blocks are complete rasters at their own LOD (see the `break`
        // in fo76utils btdfile.cpp), so coarser levels would be wasted reads.
        let block_lods: Vec<u8> = if self.header.is_starfield_layout {
            if lod <= 3 { vec![lod] } else { Vec::new() }
        } else {
            (lod..4u8).rev().collect()
        };
        for block_lod in block_lods {
            let lod_scale = 1usize << block_lod;
            let block_width = 8usize >> block_lod;
            let row_width = self.header.cells_x.div_ceil(lod_scale);
            let row_count = self.header.cells_y.div_ceil(lod_scale);
            for yy in 0..block_width {
                let block_y = (y0 >> block_lod) + yy;
                if block_y >= row_count {
                    break;
                }
                for xx in 0..block_width {
                    let block_x = (x0 >> block_lod) + xx;
                    if block_x >= row_width {
                        break;
                    }
                    let block_index = block_y * row_width + block_x;
                    let data_offset = ((yy << 10) + xx) << (usize::from(block_lod) + 7);
                    if self.header.is_starfield_layout {
                        let decoded = self.read_zlib_block_len(
                            block_lod,
                            block_index,
                            0,
                            STARFIELD_HEIGHT_LAND_BLOCK_LEN,
                        )?;
                        load_height_land_block_starfield(
                            &mut tile.heights,
                            &mut tile.land_alphas,
                            data_offset,
                            block_lod,
                            &decoded,
                        )?;
                    } else {
                        let decoded = self.read_zlib_block(block_lod, block_index, 0)?;
                        load_height_land_block(
                            &mut tile.heights,
                            &mut tile.land_alphas,
                            data_offset,
                            block_lod,
                            &decoded,
                        )?;
                    }
                }
            }
        }
        tile.block_mask = 0x0155;
        Ok(tile)
    }

    fn load_lod4_height_land(
        &self,
        tile: &mut TileData,
        x0: usize,
        y0: usize,
    ) -> Result<(), BtdError> {
        for yy in 0..64usize {
            if y0 + (yy >> 3) >= self.header.cells_y {
                break;
            }
            for xx in 0..64usize {
                if x0 + (xx >> 3) >= self.header.cells_x {
                    break;
                }
                let source_offset = (yy + (y0 << 3)) * (self.header.cells_x << 3) + xx + (x0 << 3);
                let dest_offset = ((yy << 10) + xx) << 4;
                tile.heights[dest_offset] = read_u16(
                    &self.bytes,
                    self.header.height_lod4_offset + (source_offset << 1),
                )?;
                tile.land_alphas[dest_offset] = read_u16(
                    &self.bytes,
                    self.header.land_texture_lod4_offset + (source_offset << 1),
                )?;
            }
        }
        Ok(())
    }

    fn load_lod4_vertex_color(
        &self,
        tile: &mut TileData,
        x0: usize,
        y0: usize,
    ) -> Result<(), BtdError> {
        for yy in 0..64usize {
            if y0 + (yy >> 3) >= self.header.cells_y {
                break;
            }
            for xx in 0..64usize {
                if x0 + (xx >> 3) >= self.header.cells_x {
                    break;
                }
                let source_offset = (yy + (y0 << 3)) * (self.header.cells_x << 3) + xx + (x0 << 3);
                let dest_offset = ((yy << 8) + xx) << 2;
                tile.vertex_color[dest_offset] = read_u16(
                    &self.bytes,
                    self.header.vertex_color_lod4_offset + (source_offset << 1),
                )?;
            }
        }
        Ok(())
    }

    fn read_zlib_block(
        &self,
        lod: u8,
        block_index: usize,
        stream: usize,
    ) -> Result<Vec<u8>, BtdError> {
        self.read_zlib_block_len(lod, block_index, stream, HEIGHT_LAND_BLOCK_LEN)
    }

    fn read_zlib_block_len(
        &self,
        lod: u8,
        block_index: usize,
        stream: usize,
        expected_len: usize,
    ) -> Result<Vec<u8>, BtdError> {
        let table_base = match lod {
            0 => self.header.zlib_lod0_offset,
            1 => self.header.zlib_lod1_offset,
            2 => self.header.zlib_lod2_offset,
            3 => self.header.zlib_lod3_offset,
            _ => return Err(BtdError::UnsupportedLod(lod)),
        };
        let table_index = if self.header.is_starfield_layout {
            block_index
        } else if lod == 0 && stream != 0 {
            block_index + (self.header.cells_x * self.header.cells_y)
        } else if lod >= 2 {
            (block_index << 1) + stream
        } else {
            block_index
        };
        let table_offset = table_base + table_index * ZLIB_ENTRY_LEN;
        let compressed_offset = usize::try_from(read_u32(&self.bytes, table_offset)?)
            .map_err(|_| BtdError::BadOffset(table_offset))?;
        let compressed_size = usize::try_from(read_u32(&self.bytes, table_offset + 4)?)
            .map_err(|_| BtdError::BadOffset(table_offset + 4))?;
        let absolute_offset = checked_add(self.header.zlib_data_offset, compressed_offset)?;
        decompress_block(&self.bytes, absolute_offset, compressed_size, expected_len)
    }

    fn ground_cover_candidate_mask(
        &self,
        cell_x: i32,
        cell_y: i32,
        quadrant: u8,
    ) -> Result<u8, BtdError> {
        let x = usize::try_from(cell_x - self.header.cell_min_x).unwrap();
        let y = usize::try_from(cell_y - self.header.cell_min_y).unwrap();
        let q = usize::from(quadrant);
        let offset = ((((y << 1) | (q >> 1)) * (self.header.cells_x << 1) + ((x << 1) | (q & 1)))
            << 3)
            + self.header.gcvr_map_offset;
        let mut mask = 0u8;
        for source_slot in 0..8usize {
            let raw = read_u8(&self.bytes, offset + source_slot)?;
            if decode_direct_index(raw, self.header.gcvr_count).is_some() {
                mask |= 1u8 << (7 - source_slot);
            }
        }
        Ok(mask)
    }
}

fn decode_reversed_index(raw: u8, count: usize) -> Option<u8> {
    if raw == 0 || usize::from(raw) > count {
        None
    } else {
        Some((count - usize::from(raw)) as u8)
    }
}

fn decode_direct_index(raw: u8, count: usize) -> Option<u8> {
    if usize::from(raw) >= count {
        None
    } else {
        Some(raw)
    }
}

fn reorder_ground_cover_bits(raw: u8) -> u8 {
    let rotated = raw.rotate_left(4);
    let swapped_pairs = ((rotated & 0xCC) >> 2) | ((rotated & 0x33) << 2);
    ((swapped_pairs & 0xAA) >> 1) | ((swapped_pairs & 0x55) << 1)
}

fn load_height_land_block(
    heights: &mut [u16],
    land_alphas: &mut [u16],
    data_offset: usize,
    lod: u8,
    bytes: &[u8],
) -> Result<(), BtdError> {
    if bytes.len() != HEIGHT_LAND_BLOCK_LEN {
        return Err(BtdError::Zlib(format!(
            "unexpected height/land block size {}",
            bytes.len()
        )));
    }
    let mut cursor = 0usize;
    let lod_shift = usize::from(lod);
    let xd = 1usize << lod_shift;
    let yd = (TILE_SAMPLE_WIDTH - CELL_SAMPLES) << lod_shift;
    for y in (0..CELL_SAMPLES).step_by(2) {
        load_block_lines_16(
            heights,
            data_offset + (y << (lod_shift + 10)),
            &bytes[cursor..cursor + 384],
            xd,
            yd,
        )?;
        cursor += 384;
    }
    for y in (0..CELL_SAMPLES).step_by(2) {
        load_block_lines_16(
            land_alphas,
            data_offset + (y << (lod_shift + 10)),
            &bytes[cursor..cursor + 384],
            xd,
            yd,
        )?;
        cursor += 384;
    }
    Ok(())
}

// Starfield blocks carry the full 128x128 height grid followed by the full
// 128x128 land-alpha grid (no lower-LOD delta/skip trick, unlike FO76's
// `load_height_land_block`). Ground truth: refs/fo76utils-main/src/btdfile.cpp
// `BTDFile::loadBlock_SF`.
fn load_height_land_block_starfield(
    heights: &mut [u16],
    land_alphas: &mut [u16],
    data_offset: usize,
    lod: u8,
    bytes: &[u8],
) -> Result<(), BtdError> {
    if bytes.len() != STARFIELD_HEIGHT_LAND_BLOCK_LEN {
        return Err(BtdError::Zlib(format!(
            "unexpected starfield height/land block size {}",
            bytes.len()
        )));
    }
    let xd = 1usize << usize::from(lod);
    let mut cursor = 0usize;
    for y in 0..CELL_SAMPLES {
        let mut index = data_offset + (y << (usize::from(lod) + 10));
        for _ in 0..CELL_SAMPLES {
            ensure_index_u16(heights, index)?;
            heights[index] = u16::from_le_bytes([bytes[cursor], bytes[cursor + 1]]);
            cursor += 2;
            index += xd;
        }
    }
    for y in 0..CELL_SAMPLES {
        let mut index = data_offset + (y << (usize::from(lod) + 10));
        for _ in 0..CELL_SAMPLES {
            ensure_index_u16(land_alphas, index)?;
            land_alphas[index] = u16::from_le_bytes([bytes[cursor], bytes[cursor + 1]]);
            cursor += 2;
            index += xd;
        }
    }
    Ok(())
}

fn load_vertex_color_block(
    vertex_color: &mut [u16],
    data_offset: usize,
    lod: u8,
    bytes: &[u8],
) -> Result<(), BtdError> {
    if bytes.len() != VERTEX_COLOR_BLOCK_LEN {
        return Err(BtdError::Zlib(format!(
            "unexpected vertex color block size {}",
            bytes.len()
        )));
    }
    let mut cursor = 0usize;
    let lod_shift = usize::from(lod);
    let xd = 1usize << (lod_shift - 2);
    let yd = 32usize << lod_shift;
    for y in (0..CELL_SAMPLES).step_by(2) {
        load_block_lines_16(
            vertex_color,
            data_offset + (y << (lod_shift + 6)),
            &bytes[cursor..cursor + 384],
            xd,
            yd,
        )?;
        cursor += 384;
    }
    Ok(())
}

fn load_block_lines_16(
    dst: &mut [u16],
    offset: usize,
    bytes: &[u8],
    xd: usize,
    yd: usize,
) -> Result<(), BtdError> {
    let mut cursor = 0usize;
    let mut index = offset;
    for _ in 0..64 {
        index += xd;
        ensure_index_u16(dst, index)?;
        dst[index] = u16::from_le_bytes([bytes[cursor], bytes[cursor + 1]]);
        cursor += 2;
        index += xd;
    }
    index += yd;
    for _ in 0..CELL_SAMPLES {
        ensure_index_u16(dst, index)?;
        dst[index] = u16::from_le_bytes([bytes[cursor], bytes[cursor + 1]]);
        index += xd;
        cursor += 2;
    }
    Ok(())
}

fn reorder_land_alpha_bits(values: &mut [u16]) {
    for value in values {
        let mut tmp = u32::from(*value);
        tmp = ((tmp & 0x7E00) >> 9) | (tmp & 0x01C0) | ((tmp & 0x003F) << 9);
        tmp = ((tmp & 0x7038) >> 3) | (tmp & 0x01C0) | ((tmp & 0x0E07) << 3);
        *value = tmp as u16;
    }
}

fn read_u8(bytes: &[u8], offset: usize) -> Result<u8, BtdError> {
    ensure_range(bytes, offset, 1)?;
    Ok(bytes[offset])
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, BtdError> {
    ensure_range(bytes, offset, 2)?;
    Ok(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, BtdError> {
    ensure_range(bytes, offset, 4)?;
    Ok(u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, BtdError> {
    ensure_range(bytes, offset, 4)?;
    Ok(i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}

fn read_f32(bytes: &[u8], offset: usize) -> Result<f32, BtdError> {
    ensure_range(bytes, offset, 4)?;
    Ok(f32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}

fn decompress_block(
    bytes: &[u8],
    offset: usize,
    size: usize,
    expected_len: usize,
) -> Result<Vec<u8>, BtdError> {
    ensure_range(bytes, offset, size)?;
    let mut decoder = ZlibDecoder::new(&bytes[offset..offset + size]);
    let mut decoded = Vec::new();
    decoder
        .by_ref()
        .take(u64::try_from(expected_len).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut decoded)
        .map_err(|error| BtdError::Zlib(error.to_string()))?;
    if decoded.len() != expected_len {
        return Err(BtdError::Zlib(format!(
            "unexpected decoded block size {}",
            decoded.len()
        )));
    }
    Ok(decoded)
}

fn checked_add(a: usize, b: usize) -> Result<usize, BtdError> {
    a.checked_add(b).ok_or(BtdError::BadOffset(a))
}

fn checked_cell_span(min: i32, max: i32, offset: usize) -> Result<usize, BtdError> {
    let span = i64::from(max)
        .checked_sub(i64::from(min))
        .and_then(|value| value.checked_add(1))
        .ok_or(BtdError::BadOffset(offset))?;
    if span <= 0 {
        return Err(BtdError::BadOffset(offset));
    }
    usize::try_from(span).map_err(|_| BtdError::BadOffset(offset))
}

fn checked_mul(a: usize, b: usize, offset: usize) -> Result<usize, BtdError> {
    a.checked_mul(b).ok_or(BtdError::BadOffset(offset))
}

fn ensure_range(bytes: &[u8], offset: usize, len: usize) -> Result<(), BtdError> {
    let end = offset.checked_add(len).ok_or(BtdError::BadOffset(offset))?;
    if end > bytes.len() {
        return Err(BtdError::BadOffset(offset));
    }
    Ok(())
}

fn ensure_index_u16(values: &[u16], index: usize) -> Result<(), BtdError> {
    if index >= values.len() {
        return Err(BtdError::BadOffset(index));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Verifies that BtdFile::open uses the mmap path and reaches the same
    /// parse logic as open_header: both return BadMagic on a non-BTD file
    /// that is large enough to pass the Truncated check.
    #[test]
    fn btd_mmap_matches_fs_read() {
        let mut tmp = tempfile::NamedTempFile::new().expect("temp file");
        // Write 0x2c bytes (HEADER_LEN) of non-magic data so Truncated is not hit.
        tmp.write_all(&[0u8; HEADER_LEN]).expect("write temp file");
        tmp.flush().expect("flush temp file");
        let path = tmp.path().to_string_lossy().into_owned();

        // Both the header-only path and the mmap-based open path should return
        // BadMagic — confirming the mmap read reaches the same parser.
        assert!(
            matches!(BtdFile::open_header(&path), Err(BtdError::BadMagic)),
            "open_header should return BadMagic"
        );
        assert!(
            matches!(BtdFile::open(&path), Err(BtdError::BadMagic)),
            "open (mmap) should return BadMagic"
        );
    }

    #[test]
    fn block_line_decode_matches_fo76utils_destination_stride() {
        let mut dst = vec![0u16; TILE_SAMPLE_WIDTH * 2];
        let mut bytes = Vec::with_capacity(384);
        for value in 1u16..=192 {
            bytes.extend_from_slice(&value.to_le_bytes());
        }

        load_block_lines_16(&mut dst, 0, &bytes, 1, TILE_SAMPLE_WIDTH - CELL_SAMPLES).unwrap();

        assert_eq!(dst[1], 1);
        assert_eq!(dst[3], 2);
        assert_eq!(dst[127], 64);
        assert_eq!(dst[2], 0);
        assert_eq!(dst[64], 0);
        assert_eq!(dst[TILE_SAMPLE_WIDTH], 65);
        assert_eq!(dst[TILE_SAMPLE_WIDTH + 127], 192);
    }

    #[test]
    fn gcvr_indices_and_ground_cover_bits_match_fo76utils() {
        assert_eq!(decode_direct_index(0, 3), Some(0));
        assert_eq!(decode_direct_index(2, 3), Some(2));
        assert_eq!(decode_direct_index(3, 3), None);
        assert_eq!(decode_direct_index(0xFF, 3), None);
        assert_eq!(decode_reversed_index(1, 3), Some(2));
        for bit in 0..8 {
            assert_eq!(reorder_ground_cover_bits(1 << bit), 1 << (7 - bit));
        }
    }

    #[test]
    fn one_cell_zero_bounds_fixture_still_parses_as_fo76_layout() {
        // A one-cell FO76 world at the origin (as in tests/btd_header.rs) also has
        // all-zero cell bounds; resolution 128 must keep it on the FO76 layout.
        // 600 zero bytes cover the one-cell, zero-ltex layout through
        // zlib_data_offset (560) and make gcvr_count read as 0.
        let mut bytes = vec![0u8; 600];
        bytes[0..4].copy_from_slice(b"BTDB");
        bytes[4..8].copy_from_slice(&6u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&128u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&128u32.to_le_bytes());
        let header = BtdFile::parse_header(&bytes).expect("minimal one-cell header parses");
        assert!(!header.is_starfield_layout);
        assert_eq!(header.cells_x, 1);
        assert_eq!(header.cells_y, 1);
    }
}
