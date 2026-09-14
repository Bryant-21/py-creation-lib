use crate::{
    CompressionResult, ReaderWithOptions as _, fo4,
    pack::{FileEntry, allows_compression_for_path, should_keep_compressed},
};
use bstr::BString;
use directxtex::{CP_FLAGS, DDS_FLAGS, DDSMetaData, TEX_DIMENSION, TexMetadata};
use flate2::{Compress, Compression, write::ZlibEncoder};
use memmap2::MmapOptions;
use rayon::prelude::*;
use std::{
    cell::RefCell,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    ops::Range,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

type PackResult<T> = Result<T, String>;

const MAGIC: u32 = u32::from_le_bytes(*b"BTDX");
const GNRL: u32 = u32::from_le_bytes(*b"GNRL");
const DX10: u32 = u32::from_le_bytes(*b"DX10");
const HEADER_SIZE_V1: u64 = 0x18;
const HEADER_SIZE_V2: u64 = 0x20;
const HEADER_SIZE_V3: u64 = 0x24;
const FILE_HEADER_SIZE_GNRL: u16 = 0x10;
const FILE_HEADER_SIZE_DX10: u16 = 0x18;
const CHUNK_SIZE_GNRL: u64 = 0x14;
const CHUNK_SIZE_DX10: u64 = 0x18;
const CHUNK_SENTINEL: u32 = 0xBAAD_F00D;
const DDS_HEADER_LEN: usize = 128;
pub(crate) const DDS_DX10_HEADER_LEN: usize = 148;
const DDS_HEADER_SIZE_OFFSET: usize = 4;
const DDS_PIXELFORMAT_FOURCC_OFFSET: usize = 84;
const GNRL_CHUNK_LIMIT: u64 = u32::MAX as u64;
const MAX_GNRL_CHUNKS: usize = 4;
const BUFFER_SIZE: usize = 1024 * 1024;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) struct PackOptions {
    pub(crate) version: fo4::Version,
    pub(crate) format: fo4::Format,
    pub(crate) compression_format: fo4::CompressionFormat,
    pub(crate) compress: bool,
    pub(crate) compression_level: u32,
    pub(crate) force_compress: bool,
    pub(crate) xbox_profile: bool,
}

struct TempContext {
    dir: PathBuf,
    prefix: String,
}

struct TempPayload {
    path: PathBuf,
}

pub(crate) struct StreamedChunk {
    pub(crate) payload_offset: u64,
    pub(crate) packed_len: u32,
    pub(crate) unpacked_len: u32,
    pub(crate) mips: Option<Range<u16>>,
}

struct StreamedEntry {
    hash: fo4::FileHash,
    name: Vec<u8>,
    header: fo4::FileHeader,
    chunks: Vec<StreamedChunk>,
    payload: TempPayload,
}

struct CountWriter<'a, W: Write> {
    inner: &'a mut W,
    written: u64,
}

pub(crate) fn pack_archive<Out>(
    entries: &[FileEntry],
    stream: &mut Out,
    output_path: &Path,
    version: fo4::Version,
    format: fo4::Format,
    compression_format: fo4::CompressionFormat,
    compress: bool,
    compression_level: u32,
    force_compress: bool,
    xbox_profile: bool,
) -> PackResult<()>
where
    Out: ?Sized + Write,
{
    let options = PackOptions {
        version,
        format,
        compression_format,
        compress,
        compression_level,
        force_compress,
        xbox_profile,
    };
    let temp_context = TempContext::new(output_path)?;
    let mut streamed_entries: Vec<_> = entries
        .par_iter()
        .enumerate()
        .map(|(index, entry)| build_entry(entry, index, &options, &temp_context))
        .collect::<PackResult<Vec<_>>>()?;

    assign_payload_offsets(&mut streamed_entries, &options)?;
    write_archive(stream, &streamed_entries, &options)
}

fn build_entry(
    entry: &FileEntry,
    index: usize,
    options: &PackOptions,
    temp_context: &TempContext,
) -> PackResult<StreamedEntry> {
    if options.format == fo4::Format::DX10 && !entry.rel_slash_lower.ends_with(".dds") {
        return Err(format!(
            "DX10 archives can only contain DDS files: {}",
            entry.rel_slash
        ));
    }

    match options.format {
        fo4::Format::GNRL if options.compression_format == fo4::CompressionFormat::Zip => {
            build_gnrl_entry(entry, index, options, temp_context)
        }
        fo4::Format::DX10
            if options.compression_format == fo4::CompressionFormat::Zip
                && !options.xbox_profile =>
        {
            build_dx10_entry(entry, index, options, temp_context)
                .or_else(|_| build_fallback_entry(entry, index, options, temp_context))
        }
        fo4::Format::DX10 | fo4::Format::GNRL => {
            build_fallback_entry(entry, index, options, temp_context)
        }
        fo4::Format::GNMF => Err("GNMF archives are not supported for packing".to_string()),
    }
}

fn build_gnrl_entry(
    entry: &FileEntry,
    index: usize,
    options: &PackOptions,
    temp_context: &TempContext,
) -> PackResult<StreamedEntry> {
    let (payload, mut payload_file) = TempPayload::create(temp_context, index)?;
    let mut source = File::open(&entry.full_path).map_err(|err| err.to_string())?;
    let source_len = source.metadata().map_err(|err| err.to_string())?.len();
    let ranges = plan_gnrl_chunks_for_size(source_len)?;
    let allow_compression = allows_compression_for_path(&entry.rel_slash_lower);
    let should_compress = options.force_compress || (options.compress && allow_compression);
    let mut chunks = Vec::with_capacity(ranges.len());

    for range in ranges {
        let range_len = range_len(&range);
        let unpacked_len: u32 = range
            .end
            .checked_sub(range.start)
            .ok_or_else(|| "invalid GNRL chunk range".to_string())?
            .try_into()
            .map_err(|_| "GNRL chunk size exceeds u32".to_string())?;
        let stream_pos = payload_file
            .stream_position()
            .map_err(|err| err.to_string())?;

        if should_compress {
            source
                .seek(SeekFrom::Start(range.start))
                .map_err(|err| err.to_string())?;
            let compressed_len = stream_zlib_payload(
                Read::by_ref(&mut source).take(range_len),
                &mut payload_file,
                options,
            )?;
            if compressed_len <= u64::from(u32::MAX)
                && should_keep_compressed(
                    range_len as usize,
                    compressed_len as usize,
                    options.force_compress,
                )
            {
                chunks.push(StreamedChunk {
                    payload_offset: 0,
                    packed_len: compressed_len as u32,
                    unpacked_len,
                    mips: None,
                });
                continue;
            }

            payload_file
                .set_len(stream_pos)
                .and_then(|()| payload_file.seek(SeekFrom::Start(stream_pos)).map(|_| ()))
                .map_err(|err| err.to_string())?;
        }

        source
            .seek(SeekFrom::Start(range.start))
            .map_err(|err| err.to_string())?;
        copy_exact(Read::by_ref(&mut source).take(range_len), &mut payload_file)?;
        chunks.push(StreamedChunk {
            payload_offset: 0,
            packed_len: 0,
            unpacked_len,
            mips: None,
        });
    }

    let (hash, name) = make_hash_and_name(&entry.rel_backslash);
    Ok(StreamedEntry {
        hash,
        name,
        header: fo4::FileHeader::GNRL,
        chunks,
        payload,
    })
}

fn build_dx10_entry(
    entry: &FileEntry,
    index: usize,
    options: &PackOptions,
    temp_context: &TempContext,
) -> PackResult<StreamedEntry> {
    let (payload, mut payload_file) = TempPayload::create(temp_context, index)?;
    let mut source = File::open(&entry.full_path).map_err(|err| err.to_string())?;
    let source_len = source.metadata().map_err(|err| err.to_string())?.len();
    let mapping = unsafe { MmapOptions::new().map(&source) }.map_err(|err| err.to_string())?;
    let (metadata, header_len) = dds_metadata(&mapping)?;
    let (header, ranges) = plan_dx10_ranges(&metadata, header_len, source_len)?;
    let mut chunks = Vec::with_capacity(ranges.len());

    for range in ranges {
        let range_len = range_len(&range.bytes);
        source
            .seek(SeekFrom::Start(range.bytes.start))
            .map_err(|err| err.to_string())?;
        let compressed_len = stream_zlib_payload(
            Read::by_ref(&mut source).take(range_len),
            &mut payload_file,
            options,
        )?;
        let packed_len: u32 = compressed_len
            .try_into()
            .map_err(|_| "compressed DX10 chunk size exceeds u32".to_string())?;
        let unpacked_len: u32 = range
            .bytes
            .end
            .checked_sub(range.bytes.start)
            .ok_or_else(|| "invalid DX10 chunk range".to_string())?
            .try_into()
            .map_err(|_| "DX10 chunk size exceeds u32".to_string())?;
        chunks.push(StreamedChunk {
            payload_offset: 0,
            packed_len,
            unpacked_len,
            mips: Some(range.mips),
        });
    }

    let (hash, name) = make_hash_and_name(&entry.rel_backslash);
    Ok(StreamedEntry {
        hash,
        name,
        header: fo4::FileHeader::DX10(header),
        chunks,
        payload,
    })
}

fn build_fallback_entry(
    entry: &FileEntry,
    index: usize,
    options: &PackOptions,
    temp_context: &TempContext,
) -> PackResult<StreamedEntry> {
    let (payload, mut payload_file) = TempPayload::create(temp_context, index)?;
    let compression_level = fo4_compression_level(options);
    let read_options = fo4::FileReadOptions::builder()
        .format(options.format)
        .compression_format(options.compression_format)
        .compression_level(compression_level)
        .compression_result(CompressionResult::Decompressed)
        .build();
    let chunk_options = fo4::ChunkCompressionOptions::builder()
        .compression_format(options.compression_format)
        .compression_level(compression_level)
        .build();
    let source_file =
        fo4::File::read(entry.full_path.as_path(), &read_options).map_err(|err| err.to_string())?;
    let allow_compression = allows_compression_for_path(&entry.rel_slash_lower);
    let mut chunks = Vec::with_capacity(source_file.len());

    for chunk in &source_file {
        if options.force_compress || (options.compress && allow_compression) {
            let compressed = chunk
                .compress(&chunk_options)
                .map_err(|err| err.to_string())?;
            if should_keep_compressed(chunk.len(), compressed.len(), options.force_compress) {
                payload_file
                    .write_all(compressed.as_bytes())
                    .map_err(|err| err.to_string())?;
                chunks.push(StreamedChunk {
                    payload_offset: 0,
                    packed_len: compressed
                        .len()
                        .try_into()
                        .map_err(|_| "compressed chunk size exceeds u32".to_string())?,
                    unpacked_len: chunk
                        .len()
                        .try_into()
                        .map_err(|_| "chunk size exceeds u32".to_string())?,
                    mips: chunk
                        .mips
                        .as_ref()
                        .map(|mips| *mips.start()..mips.end().saturating_add(1)),
                });
                continue;
            }
        }

        payload_file
            .write_all(chunk.as_bytes())
            .map_err(|err| err.to_string())?;
        chunks.push(StreamedChunk {
            payload_offset: 0,
            packed_len: 0,
            unpacked_len: chunk
                .len()
                .try_into()
                .map_err(|_| "chunk size exceeds u32".to_string())?,
            mips: chunk
                .mips
                .as_ref()
                .map(|mips| *mips.start()..mips.end().saturating_add(1)),
        });
    }

    let (hash, name) = make_hash_and_name(&entry.rel_backslash);
    Ok(StreamedEntry {
        hash,
        name,
        header: source_file.header.clone(),
        chunks,
        payload,
    })
}

fn assign_payload_offsets(entries: &mut [StreamedEntry], options: &PackOptions) -> PackResult<()> {
    let mut next_offset = archive_front_size(entries, options)?;
    for entry in entries {
        let mut payload_len = 0u64;
        for chunk in &mut entry.chunks {
            chunk.payload_offset = next_offset;
            let stored_len = if chunk.packed_len == 0 {
                u64::from(chunk.unpacked_len)
            } else {
                u64::from(chunk.packed_len)
            };
            next_offset = next_offset
                .checked_add(stored_len)
                .ok_or_else(|| "BA2 payload offsets overflowed u64".to_string())?;
            payload_len = payload_len
                .checked_add(stored_len)
                .ok_or_else(|| "BA2 payload length overflowed u64".to_string())?;
        }
        let actual_len = fs::metadata(entry.payload.path())
            .map_err(|err| err.to_string())?
            .len();
        if actual_len != payload_len {
            return Err(format!(
                "internal payload length mismatch for {}: expected {payload_len}, got {actual_len}",
                String::from_utf8_lossy(&entry.name)
            ));
        }
    }
    Ok(())
}

fn write_archive<Out>(
    stream: &mut Out,
    entries: &[StreamedEntry],
    options: &PackOptions,
) -> PackResult<()>
where
    Out: ?Sized + Write,
{
    let string_table_offset = entries
        .iter()
        .flat_map(|entry| entry.chunks.iter())
        .try_fold(archive_front_size(entries, options)?, |offset, chunk| {
            let stored_len = if chunk.packed_len == 0 {
                u64::from(chunk.unpacked_len)
            } else {
                u64::from(chunk.packed_len)
            };
            offset
                .checked_add(stored_len)
                .ok_or_else(|| "BA2 string table offset overflowed u64".to_string())
        })?;

    write_header(stream, options, entries.len(), string_table_offset)?;
    for entry in entries {
        write_file_record(stream, entry, options)?;
    }
    for entry in entries {
        let mut payload = File::open(entry.payload.path()).map_err(|err| err.to_string())?;
        io::copy(&mut payload, stream).map_err(|err| err.to_string())?;
    }
    for entry in entries {
        write_wstring(stream, &entry.name)?;
    }
    Ok(())
}

fn archive_front_size(entries: &[StreamedEntry], options: &PackOptions) -> PackResult<u64> {
    let chunk_count: u64 = entries.iter().map(|entry| entry.chunks.len() as u64).sum();
    archive_front_size_counts(entries.len() as u64, chunk_count, options)
}

/// Front-matter size from counts alone — shared with the incremental writer.
pub(crate) fn archive_front_size_counts(
    file_count: u64,
    chunk_count: u64,
    options: &PackOptions,
) -> PackResult<u64> {
    let header_size = match options.version {
        fo4::Version::v1 | fo4::Version::v7 | fo4::Version::v8 => HEADER_SIZE_V1,
        fo4::Version::v2 => HEADER_SIZE_V2,
        fo4::Version::v3 => HEADER_SIZE_V3,
    };
    let file_header_size = match options.format {
        fo4::Format::GNRL => u64::from(FILE_HEADER_SIZE_GNRL),
        fo4::Format::DX10 => u64::from(FILE_HEADER_SIZE_DX10),
        fo4::Format::GNMF => return Err("GNMF archives are not supported".to_string()),
    };
    let chunk_size = match options.format {
        fo4::Format::GNRL => CHUNK_SIZE_GNRL,
        fo4::Format::DX10 => CHUNK_SIZE_DX10,
        fo4::Format::GNMF => return Err("GNMF archives are not supported".to_string()),
    };
    let file_records = file_count
        .checked_mul(file_header_size)
        .ok_or_else(|| "BA2 file record size overflowed u64".to_string())?;
    let chunk_records = chunk_count
        .checked_mul(chunk_size)
        .ok_or_else(|| "BA2 chunk record size overflowed u64".to_string())?;
    header_size
        .checked_add(file_records)
        .and_then(|size| size.checked_add(chunk_records))
        .ok_or_else(|| "BA2 front matter size overflowed u64".to_string())
}

pub(crate) fn write_header<Out>(
    stream: &mut Out,
    options: &PackOptions,
    file_count: usize,
    string_table_offset: u64,
) -> PackResult<()>
where
    Out: ?Sized + Write,
{
    write_u32(stream, MAGIC)?;
    write_u32(
        stream,
        match options.version {
            fo4::Version::v1 => 1,
            fo4::Version::v2 => 2,
            fo4::Version::v3 => 3,
            fo4::Version::v7 => 7,
            fo4::Version::v8 => 8,
        },
    )?;
    write_u32(
        stream,
        match options.format {
            fo4::Format::GNRL => GNRL,
            fo4::Format::DX10 => DX10,
            fo4::Format::GNMF => return Err("GNMF archives are not supported".to_string()),
        },
    )?;
    write_u32(
        stream,
        file_count
            .try_into()
            .map_err(|_| "BA2 file count exceeds u32".to_string())?,
    )?;
    write_u64(stream, string_table_offset)?;
    if matches!(options.version, fo4::Version::v2 | fo4::Version::v3) {
        write_u64(stream, 1)?;
    }
    if options.version == fo4::Version::v3 {
        write_u32(
            stream,
            match options.compression_format {
                fo4::CompressionFormat::Zip => 0,
                fo4::CompressionFormat::LZ4 => 3,
            },
        )?;
    }
    Ok(())
}

fn write_file_record<Out>(
    stream: &mut Out,
    entry: &StreamedEntry,
    options: &PackOptions,
) -> PackResult<()>
where
    Out: ?Sized + Write,
{
    write_file_record_parts(stream, &entry.hash, &entry.header, &entry.chunks, options)
}

/// File-record writer over the entry's parts — shared with the incremental
/// writer (whose entries hold spill offsets instead of temp payloads).
pub(crate) fn write_file_record_parts<Out>(
    stream: &mut Out,
    hash: &fo4::FileHash,
    header: &fo4::FileHeader,
    chunks: &[StreamedChunk],
    options: &PackOptions,
) -> PackResult<()>
where
    Out: ?Sized + Write,
{
    write_u32(stream, hash.file)?;
    write_u32(stream, hash.extension)?;
    write_u32(stream, hash.directory)?;
    write_u8(stream, 0)?;
    write_u8(
        stream,
        chunks
            .len()
            .try_into()
            .map_err(|_| "BA2 chunk count exceeds u8".to_string())?,
    )?;
    write_u16(
        stream,
        match options.format {
            fo4::Format::GNRL => FILE_HEADER_SIZE_GNRL,
            fo4::Format::DX10 => FILE_HEADER_SIZE_DX10,
            fo4::Format::GNMF => return Err("GNMF archives are not supported".to_string()),
        },
    )?;

    match (header, options.format) {
        (fo4::FileHeader::GNRL, fo4::Format::GNRL) => {}
        (fo4::FileHeader::DX10(header), fo4::Format::DX10) => {
            write_u16(stream, header.height)?;
            write_u16(stream, header.width)?;
            write_u8(stream, header.mip_count)?;
            write_u8(stream, header.format)?;
            write_u8(stream, header.flags)?;
            write_u8(stream, header.tile_mode)?;
        }
        _ => return Err("BA2 file header does not match archive format".to_string()),
    }

    for chunk in chunks {
        write_u64(stream, chunk.payload_offset)?;
        write_u32(stream, chunk.packed_len)?;
        write_u32(stream, chunk.unpacked_len)?;
        if options.format == fo4::Format::DX10 {
            let Some(mips) = &chunk.mips else {
                return Err("DX10 chunk missing mip metadata".to_string());
            };
            write_u16(stream, mips.start)?;
            write_u16(stream, mips.end.saturating_sub(1))?;
        }
        write_u32(stream, CHUNK_SENTINEL)?;
    }
    Ok(())
}

pub(crate) fn dds_metadata(bytes: &[u8]) -> PackResult<(TexMetadata, usize)> {
    if bytes.len() < DDS_HEADER_LEN || &bytes[..4] != b"DDS " {
        return Err("not a DDS file".to_string());
    }
    let header_size = u32::from_le_bytes(
        bytes[DDS_HEADER_SIZE_OFFSET..DDS_HEADER_SIZE_OFFSET + 4]
            .try_into()
            .expect("slice length is fixed"),
    );
    if header_size != 124 {
        return Err("invalid DDS header size".to_string());
    }
    if &bytes[DDS_PIXELFORMAT_FOURCC_OFFSET..DDS_PIXELFORMAT_FOURCC_OFFSET + 4] == b"XBOX" {
        return Err("Xbox DDS files require normalization fallback".to_string());
    }

    let mut dds_metadata = DDSMetaData::default();
    let metadata = TexMetadata::from_dds(bytes, DDS_FLAGS::DDS_FLAGS_NONE, Some(&mut dds_metadata))
        .map_err(|err| err.to_string())?;
    let header_len = if dds_metadata.is_dx10() {
        DDS_DX10_HEADER_LEN
    } else {
        DDS_HEADER_LEN
    };
    if bytes.len() < header_len {
        return Err("DDS file ended before pixel payload".to_string());
    }
    Ok((metadata, header_len))
}

pub(crate) struct Dx10Range {
    pub(crate) bytes: Range<u64>,
    pub(crate) mips: Range<u16>,
}

pub(crate) fn plan_dx10_ranges(
    metadata: &TexMetadata,
    header_len: usize,
    source_len: u64,
) -> PackResult<(fo4::DX10Header, Vec<Dx10Range>)> {
    if metadata.dimension != TEX_DIMENSION::TEX_DIMENSION_TEXTURE2D || metadata.depth != 1 {
        return Err("unsupported DDS texture dimension for streaming fast path".to_string());
    }
    if metadata.array_size != 1 && !metadata.is_cubemap() {
        return Err("texture arrays require fallback".to_string());
    }
    let payload_len = source_len
        .checked_sub(header_len as u64)
        .ok_or_else(|| "DDS payload offset exceeds file size".to_string())?;
    let header = fo4::DX10Header {
        height: metadata
            .height
            .try_into()
            .map_err(|_| "DDS height exceeds BA2 DX10 header range".to_string())?,
        width: metadata
            .width
            .try_into()
            .map_err(|_| "DDS width exceeds BA2 DX10 header range".to_string())?,
        mip_count: metadata
            .mip_levels
            .try_into()
            .map_err(|_| "DDS mip count exceeds BA2 DX10 header range".to_string())?,
        format: metadata
            .format
            .bits()
            .try_into()
            .map_err(|_| "DDS DXGI format exceeds BA2 DX10 header range".to_string())?,
        flags: u8::from(metadata.is_cubemap()),
        tile_mode: 8,
    };

    if metadata.mip_levels == 0 {
        return Err("DDS has no mip levels".to_string());
    }

    if metadata.is_cubemap() {
        let mips_end: u16 = metadata
            .mip_levels
            .try_into()
            .map_err(|_| "DDS mip count exceeds BA2 DX10 header range".to_string())?;
        return Ok((
            header,
            vec![Dx10Range {
                bytes: header_len as u64..source_len,
                mips: 0..mips_end,
            }],
        ));
    }

    let mip_sizes = dx10_mip_sizes(metadata)?;
    let expected_len: u64 = mip_sizes.iter().sum();
    if expected_len != payload_len {
        return Err(format!(
            "DDS payload length mismatch for streaming fast path: expected {expected_len}, got {payload_len}"
        ));
    }

    let chunk_limit = metadata
        .format
        .compute_pitch(512, 512, CP_FLAGS::CP_FLAGS_NONE)
        .map_err(|err| err.to_string())?
        .slice as u64;
    let mip_ranges = chunk_mips(&mip_sizes, chunk_limit);
    let mut ranges = Vec::with_capacity(mip_ranges.len());
    for mip_range in mip_ranges {
        let start_offset: u64 = mip_sizes[..mip_range.start].iter().sum();
        let len: u64 = mip_sizes[mip_range.clone()].iter().sum();
        ranges.push(Dx10Range {
            bytes: (header_len as u64 + start_offset)..(header_len as u64 + start_offset + len),
            mips: mip_range
                .start
                .try_into()
                .map_err(|_| "DDS mip range start exceeds u16".to_string())?
                ..mip_range
                    .end
                    .try_into()
                    .map_err(|_| "DDS mip range end exceeds u16".to_string())?,
        });
    }

    Ok((header, ranges))
}

fn dx10_mip_sizes(metadata: &TexMetadata) -> PackResult<Vec<u64>> {
    let mut sizes = Vec::with_capacity(metadata.mip_levels);
    let mut width = metadata.width;
    let mut height = metadata.height;
    for _ in 0..metadata.mip_levels {
        let pitch = metadata
            .format
            .compute_pitch(width, height, CP_FLAGS::CP_FLAGS_NONE)
            .map_err(|err| err.to_string())?;
        sizes.push(pitch.slice as u64);
        width = usize::max(1, width / 2);
        height = usize::max(1, height / 2);
    }
    Ok(sizes)
}

fn chunk_mips(mip_sizes: &[u64], chunk_limit: u64) -> Vec<Range<usize>> {
    let mut chunks = Vec::with_capacity(4);
    let mut size = 0u64;
    let mut start = 0usize;
    let mut stop = 0usize;
    loop {
        let mip_size = mip_sizes[stop];
        if size == 0 || size + mip_size < chunk_limit {
            size += mip_size;
        } else {
            chunks.push(start..stop);
            start = stop;
            size = mip_size;
        }

        if chunks.len() == 3 {
            break;
        }

        stop += 1;
        if stop == mip_sizes.len() {
            break;
        }
    }

    if stop < mip_sizes.len() {
        chunks.push(stop..mip_sizes.len());
    } else {
        chunks.push(start..stop);
    }
    chunks
}

pub(crate) fn plan_gnrl_chunks_for_size(size: u64) -> PackResult<Vec<Range<u64>>> {
    if size == 0 {
        return Ok(vec![0..0]);
    }
    let chunk_count = size.div_ceil(GNRL_CHUNK_LIMIT) as usize;
    if chunk_count > MAX_GNRL_CHUNKS {
        return Err(format!(
            "GNRL file is too large for {MAX_GNRL_CHUNKS} BA2 chunks: {size} bytes"
        ));
    }
    let mut ranges = Vec::with_capacity(chunk_count);
    let mut start = 0u64;
    while start < size {
        let end = size.min(start + GNRL_CHUNK_LIMIT);
        ranges.push(start..end);
        start = end;
    }
    Ok(ranges)
}

thread_local! {
    static ZLIB_READ_BUFFER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn stream_zlib_payload<R, W>(
    mut reader: R,
    writer: &mut W,
    options: &PackOptions,
) -> PackResult<u64>
where
    R: Read,
    W: Write,
{
    let (level, window_bits) = zlib_settings(options);
    let counter = CountWriter {
        inner: writer,
        written: 0,
    };
    let mut encoder = ZlibEncoder::new_with_compress(
        counter,
        Compress::new_with_window_bits(level, true, window_bits),
    );
    ZLIB_READ_BUFFER.with(|cell| {
        let mut buffer = cell.borrow_mut();
        if buffer.len() < BUFFER_SIZE {
            buffer.resize(BUFFER_SIZE, 0);
        }
        loop {
            let len = reader.read(&mut buffer).map_err(|err| err.to_string())?;
            if len == 0 {
                break;
            }
            encoder
                .write_all(&buffer[..len])
                .map_err(|err| err.to_string())?;
        }
        Ok::<_, String>(())
    })?;
    let counter = encoder.finish().map_err(|err| err.to_string())?;
    Ok(counter.written)
}

fn copy_exact<R, W>(reader: R, writer: &mut W) -> PackResult<u64>
where
    R: Read,
    W: Write,
{
    let mut counter = CountWriter {
        inner: writer,
        written: 0,
    };
    let mut reader = reader;
    io::copy(&mut reader, &mut counter).map_err(|err| err.to_string())
}

fn range_len(range: &Range<u64>) -> u64 {
    range.end - range.start
}

fn zlib_settings(options: &PackOptions) -> (Compression, u8) {
    if options.xbox_profile {
        (Compression::best(), 12)
    } else {
        (Compression::new(options.compression_level), 15)
    }
}

pub(crate) fn fo4_compression_level(options: &PackOptions) -> fo4::CompressionLevel {
    if options.xbox_profile {
        fo4::CompressionLevel::FO4Xbox
    } else {
        fo4::CompressionLevel::Custom(options.compression_level)
    }
}

pub(crate) const DX10_COMPRESSION_LEVEL: u32 = 4;
pub(crate) const GNRL_COMPRESSION_LEVEL: u32 = 6;

/// Single source of truth for the zlib level applied to FO4 BA2 chunks.
/// Textures (BC-compressed, near-incompressible) get a low level; general
/// data gets a moderate one. The level is not stored in the BA2.
pub(crate) fn fo4_default_compression_level(format: fo4::Format) -> u32 {
    match format {
        fo4::Format::DX10 => DX10_COMPRESSION_LEVEL,
        _ => GNRL_COMPRESSION_LEVEL,
    }
}

pub(crate) fn make_hash_and_name(path_backslash: &str) -> (fo4::FileHash, Vec<u8>) {
    let mut normalized = BString::from(path_backslash);
    let hash = fo4::hash_file_in_place(&mut normalized);
    (hash, path_backslash.as_bytes().to_vec())
}

pub(crate) fn write_wstring<Out>(stream: &mut Out, bytes: &[u8]) -> PackResult<()>
where
    Out: ?Sized + Write,
{
    let len: u16 = bytes
        .len()
        .try_into()
        .map_err(|_| "BA2 string table entry exceeds u16".to_string())?;
    write_u16(stream, len)?;
    stream.write_all(bytes).map_err(|err| err.to_string())
}

fn write_u8<Out>(stream: &mut Out, value: u8) -> PackResult<()>
where
    Out: ?Sized + Write,
{
    stream.write_all(&[value]).map_err(|err| err.to_string())
}

fn write_u16<Out>(stream: &mut Out, value: u16) -> PackResult<()>
where
    Out: ?Sized + Write,
{
    stream
        .write_all(&value.to_le_bytes())
        .map_err(|err| err.to_string())
}

fn write_u32<Out>(stream: &mut Out, value: u32) -> PackResult<()>
where
    Out: ?Sized + Write,
{
    stream
        .write_all(&value.to_le_bytes())
        .map_err(|err| err.to_string())
}

fn write_u64<Out>(stream: &mut Out, value: u64) -> PackResult<()>
where
    Out: ?Sized + Write,
{
    stream
        .write_all(&value.to_le_bytes())
        .map_err(|err| err.to_string())
}

impl TempContext {
    fn new(output_path: &Path) -> PackResult<Self> {
        let dir = output_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .map_or_else(std::env::temp_dir, Path::to_path_buf);
        fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
        let unique = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| err.to_string())?
            .as_nanos();
        let output_name = output_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("archive.ba2");
        Ok(Self {
            dir,
            prefix: format!(
                ".{output_name}.payload-{}-{nanos}-{unique}",
                std::process::id()
            ),
        })
    }
}

impl TempPayload {
    fn create(context: &TempContext, index: usize) -> PackResult<(Self, File)> {
        let path = context.dir.join(format!("{}-{index}.tmp", context.prefix));
        let file = File::create(&path).map_err(|err| err.to_string())?;
        Ok((Self { path }, file))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempPayload {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl<W: Write> Write for CountWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::plan_gnrl_chunks_for_size;

    #[test]
    fn plans_large_gnrl_chunks_without_allocating_payload() {
        let size = (u32::MAX as u64 * 2) + 17;
        let chunks = plan_gnrl_chunks_for_size(size).expect("size should fit");
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], 0..u32::MAX as u64);
        assert_eq!(chunks[1], u32::MAX as u64..u32::MAX as u64 * 2);
        assert_eq!(chunks[2], u32::MAX as u64 * 2..size);
    }

    #[test]
    fn rejects_gnrl_files_that_exceed_four_u32_chunks() {
        let size = (u32::MAX as u64 * 4) + 1;
        let err = plan_gnrl_chunks_for_size(size).expect_err("size should be too large");
        assert!(err.contains("too large"));
    }

    #[test]
    fn default_levels_are_four_for_dx10_six_for_gnrl() {
        use crate::fo4;
        assert_eq!(super::fo4_default_compression_level(fo4::Format::DX10), 4);
        assert_eq!(super::fo4_default_compression_level(fo4::Format::GNRL), 6);
    }
}
