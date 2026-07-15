//! FO4 BA2 reader (GNRL v1/v7, DX10 v1/v7/v8).
//!
//! Starfield-specific v2/v3 branches live in `starfield.rs` and share the
//! per-file record parsing + DDS synthesis helpers exposed from here.
//!
//! # References
//! - `py_creation_lib/python/creation_lib/ba2/ba2_reader.py:91-207` — Python oracle (byte-equality target).
//! - `refs/bsa-rs/src/fo4/archive.rs` — Rust reference for the header and
//!   per-record layouts; study-only, not linked.
//! - `refs/xedit/Core/wbBSArchive.pas` — `LoadFromStreamFO4` and related
//!   routines, authoritative for version/field meaning.

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use memmap2::Mmap;

use crate::error::{NativeError, NativeResult};
use crate::formats::{ArchiveInfoRow, ArchiveReader};
use crate::io::compress::zlib_decompress;
use crate::io::dds::synthesize_dds;
use crate::io::Cursor;

const BA2_MAGIC: &[u8; 4] = b"BTDX";
const BA2_TYPE_GNRL: u32 = 0x4C52_4E47; // "GNRL" (little-endian u32)
const BA2_TYPE_DX10: u32 = 0x3031_5844; // "DX10"

pub(crate) const GNRL_RECORD_SIZE: usize = 36;
pub(crate) const DX10_HEADER_SIZE: usize = 24;
pub(crate) const DX10_CHUNK_SIZE: usize = 24;
const BA2_HEADER_SIZE: usize = 24;

// Header layout (24 bytes):
//   u32 magic ("BTDX")
//   u32 version         1 / 2 / 3 / 7 / 8
//   u32 type            "GNRL" or "DX10"
//   u32 file_count
//   u64 name_table_offset
//
// v2/v3 (Starfield) archives prepend an 8-byte extension after the header
// and before the file record table. For DX10 v3 an additional 4 bytes are
// present. Those are handled in `starfield.rs`.

#[derive(Clone, Debug)]
pub(crate) struct GnrlRecord {
    pub offset: u64,
    pub packed_len: u32,
    pub unpacked_len: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct Dx10Chunk {
    pub offset: u64,
    pub packed_len: u32,
    pub unpacked_len: u32,
    pub start_mip: u16,
    pub end_mip: u16,
}

#[derive(Clone, Debug)]
pub(crate) struct Dx10Record {
    pub width: u32,
    pub height: u32,
    pub num_mips: u32,
    pub dxgi_format: u32,
    pub flags: u16,
    pub chunks: Vec<Dx10Chunk>,
}

#[derive(Clone, Debug)]
pub(crate) enum FileRecord {
    Gnrl(GnrlRecord),
    Dx10(Dx10Record),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArchiveKind {
    Gnrl,
    Dx10,
}

/// Per-entry decompressor selector. FO4 always uses zlib; Starfield v3 DX10
/// uses LZ4 block. `starfield.rs` instantiates with `Lz4Block`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum ChunkCompression {
    Zlib,
    Lz4Block,
}

pub struct Fo4Archive {
    mmap: Arc<Mmap>,
    version: u32,
    kind: ArchiveKind,
    entries: Vec<(String, FileRecord)>,
    by_path: std::collections::HashMap<String, usize>,
    compression: ChunkCompression,
}

impl Fo4Archive {
    pub fn open(path: &Path) -> NativeResult<Self> {
        let fd = File::open(path)?;
        let mmap = unsafe { Mmap::map(&fd) }?;
        let mmap = Arc::new(mmap);
        Self::parse(mmap, FO4_VERSIONS)
    }

    /// Entry point used by `starfield.rs` — accepts a broader version set.
    #[allow(dead_code)]
    pub(crate) fn parse(
        mmap: Arc<Mmap>,
        accepted_versions: &[u32],
    ) -> NativeResult<Self> {
        let data: &[u8] = &mmap;
        if data.len() < BA2_HEADER_SIZE {
            return Err(NativeError::Parse(format!(
                "BA2 file too short ({} bytes)",
                data.len()
            )));
        }

        let mut c = Cursor::new(data);
        let magic = c.read_bytes(4)?;
        if magic != BA2_MAGIC {
            return Err(NativeError::Parse(format!(
                "not a BA2 file (magic={magic:?})"
            )));
        }
        let version = c.read_u32()?;
        if !accepted_versions.contains(&version) {
            return Err(NativeError::Unsupported(format!(
                "BA2 version {version} not supported here (accepted: {accepted_versions:?})"
            )));
        }
        let arc_type = c.read_u32()?;
        let file_count = c.read_u32()? as usize;
        let name_table_offset = c.read_u64()?;

        // Starfield v2/v3: 8-byte extension right after the 24-byte header.
        if matches!(version, 2 | 3) {
            let _sentinel = c.read_u64()?;
        }

        let (kind, compression) = match arc_type {
            BA2_TYPE_GNRL => (ArchiveKind::Gnrl, ChunkCompression::Zlib),
            BA2_TYPE_DX10 => {
                // Starfield DX10 v3 has an additional 4-byte compression-format
                // discriminator and switches to LZ4 block.
                let chunk_compression = if version == 3 {
                    let _disc = c.read_u32()?;
                    ChunkCompression::Lz4Block
                } else {
                    ChunkCompression::Zlib
                };
                (ArchiveKind::Dx10, chunk_compression)
            }
            other => {
                return Err(NativeError::Parse(format!(
                    "unknown BA2 type: 0x{other:08X}"
                )));
            }
        };

        let records: Vec<FileRecord> = match kind {
            ArchiveKind::Gnrl => parse_gnrl_records(&mut c, file_count)?,
            ArchiveKind::Dx10 => parse_dx10_records(&mut c, file_count)?,
        };

        let names = parse_name_table(data, name_table_offset as usize, file_count)?;

        let mut entries = Vec::with_capacity(records.len());
        let mut by_path = std::collections::HashMap::with_capacity(records.len());
        for (idx, (name, rec)) in names.into_iter().zip(records.into_iter()).enumerate() {
            let key = name.to_ascii_lowercase().replace('\\', "/");
            by_path.insert(key.clone(), idx);
            entries.push((key, rec));
        }

        Ok(Self {
            mmap,
            version,
            kind,
            entries,
            by_path,
            compression,
        })
    }
}

const FO4_VERSIONS: &[u32] = &[1, 7, 8];

pub(crate) fn parse_gnrl_records(c: &mut Cursor<'_>, count: usize) -> NativeResult<Vec<FileRecord>> {
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        // name_hash(4) + ext(4) + dir_hash(4) + flags(4) + offset(8) +
        // packed_len(4) + unpacked_len(4) + pad(4) = 36 bytes.
        let _name_hash = c.read_u32()?;
        let _ext = c.read_u32()?;
        let _dir_hash = c.read_u32()?;
        let _flags = c.read_u32()?;
        let offset = c.read_u64()?;
        let packed_len = c.read_u32()?;
        let unpacked_len = c.read_u32()?;
        let _pad = c.read_u32()?;
        out.push(FileRecord::Gnrl(GnrlRecord {
            offset,
            packed_len,
            unpacked_len,
        }));
    }
    Ok(out)
}

pub(crate) fn parse_dx10_records(c: &mut Cursor<'_>, count: usize) -> NativeResult<Vec<FileRecord>> {
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        // name_hash(4) + ext(4) + dir_hash(4) + unk8(1) + num_chunks(1) +
        // chunk_header_size(2) + height(2) + width(2) + num_mips(1) +
        // dxgi_format(1) + flags(2) = 24 bytes.
        let _name_hash = c.read_u32()?;
        let _ext = c.read_u32()?;
        let _dir_hash = c.read_u32()?;
        let _unk8 = c.read_u8()?;
        let num_chunks = c.read_u8()? as usize;
        let _chunk_header_size = c.read_u16()?;
        let height = c.read_u16()? as u32;
        let width = c.read_u16()? as u32;
        let num_mips = c.read_u8()? as u32;
        let dxgi_format = c.read_u8()? as u32;
        let flags = c.read_u16()?;
        let chunks = parse_dx10_chunks(c, num_chunks)?;
        out.push(FileRecord::Dx10(Dx10Record {
            width,
            height,
            num_mips,
            dxgi_format,
            flags,
            chunks,
        }));
    }
    Ok(out)
}

fn parse_dx10_chunks(c: &mut Cursor<'_>, count: usize) -> NativeResult<Vec<Dx10Chunk>> {
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        // offset(8) + packed_len(4) + unpacked_len(4) + start_mip(2) +
        // end_mip(2) + pad(4) = 24 bytes.
        let offset = c.read_u64()?;
        let packed_len = c.read_u32()?;
        let unpacked_len = c.read_u32()?;
        let start_mip = c.read_u16()?;
        let end_mip = c.read_u16()?;
        let _pad = c.read_u32()?;
        out.push(Dx10Chunk {
            offset,
            packed_len,
            unpacked_len,
            start_mip,
            end_mip,
        });
    }
    Ok(out)
}

fn parse_name_table(data: &[u8], offset: usize, count: usize) -> NativeResult<Vec<String>> {
    if offset == 0 || offset > data.len() {
        return Ok((0..count).map(|i| format!("unnamed_{i}")).collect());
    }
    let mut c = Cursor::at(data, offset);
    let mut names = Vec::with_capacity(count);
    for _ in 0..count {
        let len = c.read_u16()? as usize;
        let raw = c.read_bytes(len)?;
        let stripped = if raw.last().copied() == Some(0) {
            &raw[..raw.len() - 1]
        } else {
            raw
        };
        names.push(String::from_utf8_lossy(stripped).into_owned());
    }
    Ok(names)
}

fn decompress_chunk(
    data: &[u8],
    packed_len: u32,
    unpacked_len: u32,
    offset: u64,
    compression: ChunkCompression,
) -> NativeResult<Vec<u8>> {
    let start = offset as usize;
    if packed_len == 0 {
        // Raw payload — `unpacked_len` is authoritative size.
        let end = start.saturating_add(unpacked_len as usize);
        if end > data.len() {
            return Err(NativeError::Parse(format!(
                "raw chunk at {start} len {unpacked_len} runs past archive end"
            )));
        }
        return Ok(data[start..end].to_vec());
    }
    let end = start.saturating_add(packed_len as usize);
    if end > data.len() {
        return Err(NativeError::Parse(format!(
            "compressed chunk at {start} len {packed_len} runs past archive end"
        )));
    }
    let body = &data[start..end];
    match compression {
        ChunkCompression::Zlib => zlib_decompress(body, unpacked_len as usize),
        ChunkCompression::Lz4Block => {
            crate::io::compress::lz4_block_decompress(body, unpacked_len as usize)
        }
    }
}

impl ArchiveReader for Fo4Archive {
    fn list(&self) -> Vec<String> {
        self.entries.iter().map(|(k, _)| k.clone()).collect()
    }

    fn extract(&self, path: &str) -> NativeResult<Vec<u8>> {
        let key = path.to_ascii_lowercase().replace('\\', "/");
        let idx = *self
            .by_path
            .get(&key)
            .ok_or_else(|| NativeError::FileNotFound(path.to_string()))?;
        let (_, record) = &self.entries[idx];
        let data: &[u8] = &self.mmap;
        match record {
            FileRecord::Gnrl(rec) => decompress_chunk(
                data,
                rec.packed_len,
                rec.unpacked_len,
                rec.offset,
                self.compression,
            ),
            FileRecord::Dx10(rec) => {
                // DDS header + concatenated decompressed chunks, matching
                // `py_creation_lib/python/creation_lib/ba2/ba2_reader.py:273-301` (oracle).
                let header = synthesize_dds(
                    rec.dxgi_format,
                    rec.width,
                    rec.height,
                    rec.num_mips,
                    // FO4 "cubemaps" are 2D env maps, not true 6-face
                    // cubemaps — oracle keeps is_cubemap=False.
                    false,
                );
                let mut pixel_data = Vec::new();
                for chunk in &rec.chunks {
                    let decoded = decompress_chunk(
                        data,
                        chunk.packed_len,
                        chunk.unpacked_len,
                        chunk.offset,
                        self.compression,
                    )?;
                    pixel_data.extend(decoded);
                }
                let mut out = header;
                out.extend(pixel_data);
                Ok(out)
            }
        }
    }

    fn info(&self) -> ArchiveInfoRow {
        let format = match self.kind {
            ArchiveKind::Gnrl => "fo4_gnrl",
            ArchiveKind::Dx10 => "fo4_dx10",
        };
        let compressed = self.entries.iter().any(|(_, r)| match r {
            FileRecord::Gnrl(g) => g.packed_len != 0,
            FileRecord::Dx10(d) => d.chunks.iter().any(|c| c.packed_len != 0),
        });
        ArchiveInfoRow {
            format: format.to_string(),
            version: self.version,
            file_count: self.entries.len(),
            compressed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_sizes_match_spec() {
        // Sanity: the on-disk sizes referenced in the oracle.
        assert_eq!(GNRL_RECORD_SIZE, 36);
        assert_eq!(DX10_HEADER_SIZE, 24);
        assert_eq!(DX10_CHUNK_SIZE, 24);
    }

    #[test]
    fn magic_constants_are_little_endian_fourcc() {
        assert_eq!(BA2_MAGIC, b"BTDX");
        assert_eq!(BA2_TYPE_GNRL.to_le_bytes(), *b"GNRL");
        assert_eq!(BA2_TYPE_DX10.to_le_bytes(), *b"DX10");
    }
}
