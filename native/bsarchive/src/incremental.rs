//! Incremental FO4 BA2 writer: files are compressed/chunked the
//! moment they arrive ("append-as-complete") into a single append-only spill
//! file; `finalize` assembles a complete archive equivalent to the one-shot
//! `pack_fo4_stream::pack_archive` over the same files.
//!
//! Mirrors the one-shot path's decision logic exactly (chunk planning,
//! compression keep/raw, DX10 fast path with fallback, entry sort order). The
//! final payload copy follows spill append order so large archives do not turn
//! finalization into a random-read pass over the spill file.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};

use memmap2::MmapOptions;
use rayon::prelude::*;

use crate::pack::{
    FileEntry, allows_compression_for_path, normalize_archive_entry_path, should_keep_compressed,
};
use crate::pack_fo4_stream::{
    PackOptions, StreamedChunk, archive_front_size_counts, dds_metadata, fo4_compression_level,
    make_hash_and_name, plan_dx10_ranges, plan_gnrl_chunks_for_size, stream_zlib_payload,
    write_file_record_parts, write_header, write_wstring,
};
use crate::{Borrowed, CompressionResult, ReaderWithOptions as _, fo4};

type PackResult<T> = Result<T, String>;

const DIRECT_PACK_MEMORY_BUDGET: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fo4WriterKind {
    /// `"fo4"` / `"fo4og"` / `"fo76"` archive types: GNRL, Zip, force_compress off.
    Gnrl,
    /// `"fo4dds"` / `"fo4ogdds"` / `"fo76dds"` archive types: DX10, Zip, force_compress on.
    Dx10,
}

#[derive(Clone, Copy, Debug)]
pub struct CompressionSettings {
    pub compress: bool,
    pub compression_level: u32,
}

impl Default for CompressionSettings {
    /// Test-only default. NOT the pack level: production uses fo4_default_compression_level / for_writer_kind.
    fn default() -> Self {
        Self {
            compress: true,
            compression_level: 9,
        }
    }
}

impl CompressionSettings {
    /// Per-format level from the single source of truth in pack_fo4_stream.
    pub fn for_writer_kind(kind: Fo4WriterKind) -> Self {
        let compression_level = match kind {
            Fo4WriterKind::Dx10 => crate::pack_fo4_stream::DX10_COMPRESSION_LEVEL,
            Fo4WriterKind::Gnrl => crate::pack_fo4_stream::GNRL_COMPRESSION_LEVEL,
        };
        Self {
            compress: true,
            compression_level,
        }
    }
}

struct SpillChunk {
    spill_offset: u64,
    stored_len: u64,
    packed_len: u32,
    unpacked_len: u32,
    mips: Option<Range<u16>>,
}

struct SpillEntry {
    /// Case-preserved backslash rel path — hash/name + the finalize sort key
    /// (collect_entry_specs sorts by rel_backslash; mirrored here).
    rel_backslash: String,
    header: fo4::FileHeader,
    chunks: Vec<SpillChunk>,
}

struct SpillFile {
    file: File,
    len: u64,
}

struct EntryEncoder {
    kind: Fo4WriterKind,
    opts: PackOptions,
}

struct PreparedEntry {
    rel_lower: String,
    rel_backslash: String,
    header: fo4::FileHeader,
    chunks: Vec<ChunkPlan>,
    payload: Vec<u8>,
}

struct DirectEntry {
    rel_backslash: String,
    header: fo4::FileHeader,
    chunks: Vec<StreamedChunk>,
}

/// A compressed entry queued for the dedicated writer thread. The memory
/// guard rides along so the in-flight budget is released only once the
/// payload has actually been written.
struct QueuedDirectEntry<'a> {
    rel_backslash: String,
    header: fo4::FileHeader,
    chunk_plans: Vec<ChunkPlan>,
    payload: Vec<u8>,
    _memory_guard: InFlightMemoryGuard<'a>,
}

struct InFlightMemory {
    used: Mutex<usize>,
    available: Condvar,
    limit: usize,
}

struct InFlightMemoryGuard<'a> {
    budget: &'a InFlightMemory,
    reserved: usize,
}

impl InFlightMemory {
    fn new(limit: usize) -> Self {
        Self {
            used: Mutex::new(0),
            available: Condvar::new(),
            limit: limit.max(1),
        }
    }

    fn acquire(&self, requested: usize) -> InFlightMemoryGuard<'_> {
        let reserved = requested.min(self.limit).max(1);
        let mut used = self.used.lock().expect("memory budget mutex poisoned");
        while *used != 0 && used.saturating_add(reserved) > self.limit {
            used = self
                .available
                .wait(used)
                .expect("memory budget mutex poisoned");
        }
        *used += reserved;
        InFlightMemoryGuard {
            budget: self,
            reserved,
        }
    }
}

impl Drop for InFlightMemoryGuard<'_> {
    fn drop(&mut self) {
        let mut used = self
            .budget
            .used
            .lock()
            .expect("memory budget mutex poisoned");
        *used -= self.reserved;
        self.budget.available.notify_all();
    }
}

pub struct IncrementalFo4Writer {
    encoder: EntryEncoder,
    spill_path: PathBuf,
    spill: Mutex<SpillFile>,
    /// Keyed by rel_slash_lower (the dedup space `collect_entry_specs` uses).
    entries: Mutex<HashMap<String, SpillEntry>>,
}

impl IncrementalFo4Writer {
    pub fn new(
        spill_path: PathBuf,
        kind: Fo4WriterKind,
        settings: CompressionSettings,
    ) -> PackResult<Self> {
        if let Some(parent) = spill_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| format!("spill dir: {e}"))?;
            }
        }
        let file = File::create(&spill_path).map_err(|e| format!("spill create: {e}"))?;
        let encoder = EntryEncoder::new(kind, fo4::Version::v8, settings);
        Ok(Self {
            encoder,
            spill_path,
            spill: Mutex::new(SpillFile { file, len: 0 }),
            entries: Mutex::new(HashMap::new()),
        })
    }

    pub fn spill_path(&self) -> &Path {
        &self.spill_path
    }

    pub fn contains(&self, rel: &str) -> bool {
        let Ok(rel_slash) = normalize_archive_entry_path(rel) else {
            return false;
        };
        self.entries
            .lock()
            .expect("entries mutex poisoned")
            .contains_key(&rel_slash.to_ascii_lowercase())
    }

    /// Normalized (lowercase, forward-slash) rel paths streamed so far.
    pub fn rel_paths(&self) -> Vec<String> {
        self.entries
            .lock()
            .expect("entries mutex poisoned")
            .keys()
            .cloned()
            .collect()
    }

    pub fn entry_count(&self) -> usize {
        self.entries.lock().expect("entries mutex poisoned").len()
    }

    /// Add a file's bytes. Returns Ok(false) if the rel path was already
    /// added (first-wins dedup). Compression happens OUTSIDE the spill lock;
    /// callable from rayon threads.
    pub fn add_bytes(&self, rel: &str, bytes: &[u8]) -> PackResult<bool> {
        let rel_slash = normalize_archive_entry_path(rel)?;
        let rel_lower = rel_slash.to_ascii_lowercase();
        if self
            .entries
            .lock()
            .expect("entries mutex poisoned")
            .contains_key(&rel_lower)
        {
            return Ok(false);
        }
        let prepared = self.encoder.prepare_bytes(&rel_slash, bytes)?;

        // ONE locked append for the whole entry.
        let base_offset = {
            let mut spill = self.spill.lock().expect("spill mutex poisoned");
            let offset = spill.len;
            spill
                .file
                .write_all(&prepared.payload)
                .map_err(|e| format!("spill write: {e}"))?;
            spill.len += prepared.payload.len() as u64;
            offset
        };

        let mut chunks = Vec::with_capacity(prepared.chunks.len());
        let mut running = base_offset;
        for plan in prepared.chunks {
            chunks.push(SpillChunk {
                spill_offset: running,
                stored_len: plan.stored_len,
                packed_len: plan.packed_len,
                unpacked_len: plan.unpacked_len,
                mips: plan.mips,
            });
            running += plan.stored_len;
        }

        let mut entries = self.entries.lock().expect("entries mutex poisoned");
        if entries.contains_key(&prepared.rel_lower) {
            // Lost a concurrent first-wins race; the spilled bytes are
            // unreferenced and harmless.
            return Ok(false);
        }
        entries.insert(
            prepared.rel_lower,
            SpillEntry {
                rel_backslash: prepared.rel_backslash,
                header: prepared.header,
                chunks,
            },
        );
        Ok(true)
    }

    pub fn add_file(&self, rel: &str, path: &Path) -> PackResult<bool> {
        // Cheap dedup before touching the file.
        if self.contains(rel) {
            return Ok(false);
        }
        let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        self.add_bytes(rel, &bytes)
    }

    /// Write a complete BA2 containing exactly `ordered_rels` (membership;
    /// the on-disk entry order is the one-shot packer's rel_backslash sort).
    pub fn finalize(&self, output: &Path, ordered_rels: &[&str]) -> PackResult<()> {
        let entries = self.entries.lock().expect("entries mutex poisoned");
        let mut selected: Vec<&SpillEntry> = Vec::with_capacity(ordered_rels.len());
        let mut seen: std::collections::HashSet<String> =
            std::collections::HashSet::with_capacity(ordered_rels.len());
        for rel in ordered_rels {
            let rel_lower = normalize_archive_entry_path(rel)?.to_ascii_lowercase();
            if !seen.insert(rel_lower.clone()) {
                return Err(format!("duplicate archive path in finalize: {rel}"));
            }
            let entry = entries
                .get(&rel_lower)
                .ok_or_else(|| format!("finalize: rel path was never added: {rel}"))?;
            selected.push(entry);
        }
        // Mirror collect_entry_specs' sort (pack.rs): order by rel_backslash.
        selected.sort_by(|a, b| a.rel_backslash.cmp(&b.rel_backslash));

        let chunk_count: u64 = selected.iter().map(|e| e.chunks.len() as u64).sum();
        let front =
            archive_front_size_counts(selected.len() as u64, chunk_count, &self.encoder.opts)?;

        let mut records: Vec<(fo4::FileHash, Vec<u8>, &fo4::FileHeader, Vec<StreamedChunk>)> =
            Vec::with_capacity(selected.len());
        let mut payloads = Vec::with_capacity(chunk_count as usize);
        for (record_index, entry) in selected.iter().enumerate() {
            let (hash, name) = make_hash_and_name(&entry.rel_backslash);
            let mut chunks = Vec::with_capacity(entry.chunks.len());
            for (chunk_index, chunk) in entry.chunks.iter().enumerate() {
                chunks.push(StreamedChunk {
                    payload_offset: 0,
                    packed_len: chunk.packed_len,
                    unpacked_len: chunk.unpacked_len,
                    mips: chunk.mips.clone(),
                });
                payloads.push(PayloadCopy {
                    spill_offset: chunk.spill_offset,
                    stored_len: chunk.stored_len,
                    record_index,
                    chunk_index,
                });
            }
            records.push((hash, name, &entry.header, chunks));
        }

        // Payload order in BA2 is governed by per-chunk offsets, not by file
        // record order. Preserve sorted records but copy the append-only spill
        // in append order; otherwise a large texture BA2 performs hundreds of
        // thousands of random seeks through a multi-GB spill file.
        payloads.sort_by_key(|payload| payload.spill_offset);
        let mut next_offset = front;
        for payload in &payloads {
            records[payload.record_index].3[payload.chunk_index].payload_offset = next_offset;
            next_offset = next_offset
                .checked_add(payload.stored_len)
                .ok_or_else(|| "BA2 payload offsets overflowed u64".to_string())?;
        }
        let string_table_offset = next_offset;

        if let Some(parent) = output.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| format!("output dir: {e}"))?;
            }
        }
        let out_file = File::create(output).map_err(|e| format!("output create: {e}"))?;
        let mut out = BufWriter::new(out_file);

        write_header(
            &mut out,
            &self.encoder.opts,
            selected.len(),
            string_table_offset,
        )?;
        for (hash, _, header, chunks) in &records {
            write_file_record_parts(&mut out, hash, header, chunks, &self.encoder.opts)?;
        }

        // Payload: copy stored bytes from the spill in append order.
        let spill_read = File::open(&self.spill_path).map_err(|e| format!("spill open: {e}"))?;
        let mut spill_reader = BufReader::new(spill_read);
        let mut spill_position = 0u64;
        for payload in &payloads {
            if spill_position != payload.spill_offset {
                spill_reader
                    .seek(SeekFrom::Start(payload.spill_offset))
                    .map_err(|e| format!("spill seek: {e}"))?;
                spill_position = payload.spill_offset;
            }
            let mut take = Read::by_ref(&mut spill_reader).take(payload.stored_len);
            let copied =
                std::io::copy(&mut take, &mut out).map_err(|e| format!("spill copy: {e}"))?;
            if copied != payload.stored_len {
                return Err(format!(
                    "spill short read: expected {} bytes, got {copied}",
                    payload.stored_len
                ));
            }
            spill_position = spill_position
                .checked_add(copied)
                .ok_or_else(|| "spill position overflowed u64".to_string())?;
        }

        for (_, name, _, _) in &records {
            write_wstring(&mut out, name)?;
        }
        out.flush().map_err(|e| format!("output flush: {e}"))?;
        Ok(())
    }
}

impl EntryEncoder {
    fn new(kind: Fo4WriterKind, version: fo4::Version, settings: CompressionSettings) -> Self {
        let opts = PackOptions {
            version,
            format: match kind {
                Fo4WriterKind::Gnrl => fo4::Format::GNRL,
                Fo4WriterKind::Dx10 => fo4::Format::DX10,
            },
            compression_format: fo4::CompressionFormat::Zip,
            compress: settings.compress,
            compression_level: settings.compression_level,
            // "fo4dds" hardwires force_compress (pack.rs parse_pack_kind).
            force_compress: kind == Fo4WriterKind::Dx10,
            xbox_profile: false,
        };
        Self { kind, opts }
    }

    fn prepare_bytes(&self, rel: &str, bytes: &[u8]) -> PackResult<PreparedEntry> {
        let rel_slash = normalize_archive_entry_path(rel)?;
        let rel_lower = rel_slash.to_ascii_lowercase();
        let rel_backslash = rel_slash.replace('/', "\\");

        if self.kind == Fo4WriterKind::Dx10 && !rel_lower.ends_with(".dds") {
            return Err(format!(
                "DX10 archives can only contain DDS files: {rel_slash}"
            ));
        }

        let (header, chunk_plans, payload) = match self.kind {
            Fo4WriterKind::Gnrl => self.build_gnrl_from_bytes(&rel_lower, bytes)?,
            Fo4WriterKind::Dx10 => self
                .build_dx10_from_bytes(bytes)
                .or_else(|_| self.build_fallback_from_bytes(&rel_lower, bytes))?,
        };

        Ok(PreparedEntry {
            rel_lower,
            rel_backslash,
            header,
            chunks: chunk_plans,
            payload,
        })
    }

    fn prepare_file(&self, entry: &FileEntry) -> PackResult<PreparedEntry> {
        let rel_slash = normalize_archive_entry_path(&entry.rel_slash)?;
        let rel_lower = rel_slash.to_ascii_lowercase();
        let rel_backslash = rel_slash.replace('/', "\\");
        if self.kind == Fo4WriterKind::Dx10 && !rel_lower.ends_with(".dds") {
            return Err(format!(
                "DX10 archives can only contain DDS files: {rel_slash}"
            ));
        }

        let fast_result = match self.kind {
            Fo4WriterKind::Gnrl => self.build_gnrl_from_file(&rel_lower, &entry.full_path),
            Fo4WriterKind::Dx10 => self.build_dx10_from_file(&entry.full_path),
        };
        let (header, chunks, payload) = match fast_result {
            Ok(prepared) => prepared,
            Err(_) if self.kind == Fo4WriterKind::Dx10 => {
                let bytes = fs::read(&entry.full_path)
                    .map_err(|e| format!("read {}: {e}", entry.full_path.display()))?;
                let prepared = self.prepare_bytes(&rel_slash, &bytes)?;
                return Ok(prepared);
            }
            Err(err) => return Err(err),
        };
        Ok(PreparedEntry {
            rel_lower,
            rel_backslash,
            header,
            chunks,
            payload,
        })
    }

    // -- entry builders (byte-parity mirrors of pack_fo4_stream::build_*) ----

    fn build_gnrl_from_bytes(
        &self,
        rel_slash_lower: &str,
        bytes: &[u8],
    ) -> PackResult<(fo4::FileHeader, Vec<ChunkPlan>, Vec<u8>)> {
        let ranges = plan_gnrl_chunks_for_size(bytes.len() as u64)?;
        let allow_compression = allows_compression_for_path(rel_slash_lower);
        let should_compress = self.opts.force_compress || (self.opts.compress && allow_compression);
        let mut plans = Vec::with_capacity(ranges.len());
        let mut payload = Vec::new();

        for range in ranges {
            let slice = &bytes[range.start as usize..range.end as usize];
            let unpacked_len: u32 = (range.end - range.start)
                .try_into()
                .map_err(|_| "GNRL chunk size exceeds u32".to_string())?;

            if should_compress {
                let mut compressed = Vec::new();
                stream_zlib_payload(slice, &mut compressed, &self.opts)?;
                if compressed.len() as u64 <= u64::from(u32::MAX)
                    && should_keep_compressed(
                        slice.len(),
                        compressed.len(),
                        self.opts.force_compress,
                    )
                {
                    plans.push(ChunkPlan {
                        stored_len: compressed.len() as u64,
                        packed_len: compressed.len() as u32,
                        unpacked_len,
                        mips: None,
                    });
                    payload.extend_from_slice(&compressed);
                    continue;
                }
            }

            plans.push(ChunkPlan {
                stored_len: slice.len() as u64,
                packed_len: 0,
                unpacked_len,
                mips: None,
            });
            payload.extend_from_slice(slice);
        }

        Ok((fo4::FileHeader::GNRL, plans, payload))
    }

    fn build_gnrl_from_file(
        &self,
        rel_slash_lower: &str,
        path: &Path,
    ) -> PackResult<(fo4::FileHeader, Vec<ChunkPlan>, Vec<u8>)> {
        let mut source = File::open(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let source_len = source.metadata().map_err(|e| e.to_string())?.len();
        let ranges = plan_gnrl_chunks_for_size(source_len)?;
        let allow_compression = allows_compression_for_path(rel_slash_lower);
        let should_compress = self.opts.force_compress || (self.opts.compress && allow_compression);
        let mut plans = Vec::with_capacity(ranges.len());
        let mut payload = Vec::new();

        for range in ranges {
            let range_len = range.end - range.start;
            let unpacked_len: u32 = range_len
                .try_into()
                .map_err(|_| "GNRL chunk size exceeds u32".to_string())?;
            let payload_start = payload.len();
            if should_compress {
                source
                    .seek(SeekFrom::Start(range.start))
                    .map_err(|e| e.to_string())?;
                let compressed_len = stream_zlib_payload(
                    Read::by_ref(&mut source).take(range_len),
                    &mut payload,
                    &self.opts,
                )?;
                if compressed_len <= u64::from(u32::MAX)
                    && should_keep_compressed(
                        range_len as usize,
                        compressed_len as usize,
                        self.opts.force_compress,
                    )
                {
                    plans.push(ChunkPlan {
                        stored_len: compressed_len,
                        packed_len: compressed_len as u32,
                        unpacked_len,
                        mips: None,
                    });
                    continue;
                }
                payload.truncate(payload_start);
            }

            source
                .seek(SeekFrom::Start(range.start))
                .map_err(|e| e.to_string())?;
            let copied = io::copy(&mut Read::by_ref(&mut source).take(range_len), &mut payload)
                .map_err(|e| e.to_string())?;
            plans.push(ChunkPlan {
                stored_len: copied,
                packed_len: 0,
                unpacked_len,
                mips: None,
            });
        }
        Ok((fo4::FileHeader::GNRL, plans, payload))
    }

    fn build_dx10_from_bytes(
        &self,
        bytes: &[u8],
    ) -> PackResult<(fo4::FileHeader, Vec<ChunkPlan>, Vec<u8>)> {
        let (metadata, header_len) = dds_metadata(bytes)?;
        let (header, ranges) = plan_dx10_ranges(&metadata, header_len, bytes.len() as u64)?;
        let mut plans = Vec::with_capacity(ranges.len());
        let mut payload = Vec::new();

        for range in ranges {
            let slice = &bytes[range.bytes.start as usize..range.bytes.end as usize];
            let before = payload.len();
            stream_zlib_payload(slice, &mut payload, &self.opts)?;
            let compressed_len = payload.len() - before;
            let packed_len: u32 = compressed_len
                .try_into()
                .map_err(|_| "compressed DX10 chunk size exceeds u32".to_string())?;
            let unpacked_len: u32 = (range.bytes.end - range.bytes.start)
                .try_into()
                .map_err(|_| "DX10 chunk size exceeds u32".to_string())?;
            plans.push(ChunkPlan {
                stored_len: compressed_len as u64,
                packed_len,
                unpacked_len,
                mips: Some(range.mips),
            });
        }

        Ok((fo4::FileHeader::DX10(header), plans, payload))
    }

    fn build_dx10_from_file(
        &self,
        path: &Path,
    ) -> PackResult<(fo4::FileHeader, Vec<ChunkPlan>, Vec<u8>)> {
        let mut source = File::open(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let source_len = source.metadata().map_err(|e| e.to_string())?.len();
        let mapping = unsafe { MmapOptions::new().map(&source) }.map_err(|e| e.to_string())?;
        let (metadata, header_len) = dds_metadata(&mapping)?;
        let (header, ranges) = plan_dx10_ranges(&metadata, header_len, source_len)?;
        let mut plans = Vec::with_capacity(ranges.len());
        let mut payload = Vec::new();

        for range in ranges {
            let range_len = range.bytes.end - range.bytes.start;
            source
                .seek(SeekFrom::Start(range.bytes.start))
                .map_err(|e| e.to_string())?;
            let compressed_len = stream_zlib_payload(
                Read::by_ref(&mut source).take(range_len),
                &mut payload,
                &self.opts,
            )?;
            plans.push(ChunkPlan {
                stored_len: compressed_len,
                packed_len: compressed_len
                    .try_into()
                    .map_err(|_| "compressed DX10 chunk size exceeds u32".to_string())?,
                unpacked_len: range_len
                    .try_into()
                    .map_err(|_| "DX10 chunk size exceeds u32".to_string())?,
                mips: Some(range.mips),
            });
        }
        Ok((fo4::FileHeader::DX10(header), plans, payload))
    }

    fn build_fallback_from_bytes(
        &self,
        rel_slash_lower: &str,
        bytes: &[u8],
    ) -> PackResult<(fo4::FileHeader, Vec<ChunkPlan>, Vec<u8>)> {
        let compression_level = fo4_compression_level(&self.opts);
        let read_options = fo4::FileReadOptions::builder()
            .format(self.opts.format)
            .compression_format(self.opts.compression_format)
            .compression_level(compression_level)
            .compression_result(CompressionResult::Decompressed)
            .build();
        let chunk_options = fo4::ChunkCompressionOptions::builder()
            .compression_format(self.opts.compression_format)
            .compression_level(compression_level)
            .build();
        let source_file =
            fo4::File::read(Borrowed(bytes), &read_options).map_err(|err| err.to_string())?;
        let allow_compression = allows_compression_for_path(rel_slash_lower);
        let mut plans = Vec::with_capacity(source_file.len());
        let mut payload = Vec::new();

        for chunk in &source_file {
            let mips = chunk
                .mips
                .as_ref()
                .map(|mips| *mips.start()..mips.end().saturating_add(1));
            if self.opts.force_compress || (self.opts.compress && allow_compression) {
                let compressed = chunk
                    .compress(&chunk_options)
                    .map_err(|err| err.to_string())?;
                if should_keep_compressed(chunk.len(), compressed.len(), self.opts.force_compress) {
                    plans.push(ChunkPlan {
                        stored_len: compressed.len() as u64,
                        packed_len: compressed
                            .len()
                            .try_into()
                            .map_err(|_| "compressed chunk size exceeds u32".to_string())?,
                        unpacked_len: chunk
                            .len()
                            .try_into()
                            .map_err(|_| "chunk size exceeds u32".to_string())?,
                        mips,
                    });
                    payload.extend_from_slice(compressed.as_bytes());
                    continue;
                }
            }

            plans.push(ChunkPlan {
                stored_len: chunk.len() as u64,
                packed_len: 0,
                unpacked_len: chunk
                    .len()
                    .try_into()
                    .map_err(|_| "chunk size exceeds u32".to_string())?,
                mips,
            });
            payload.extend_from_slice(chunk.as_bytes());
        }

        let header = source_file.header.clone();
        Ok((header, plans, payload))
    }
}

pub(crate) fn pack_fo4_direct(
    entries: &[FileEntry],
    output_path: &Path,
    writer_kind: Fo4WriterKind,
    version: fo4::Version,
    settings: CompressionSettings,
) -> PackResult<()> {
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("output dir: {e}"))?;
        }
    }

    let encoder = EntryEncoder::new(writer_kind, version, settings);
    let reserved_chunk_count = entries
        .len()
        .checked_mul(4)
        .ok_or_else(|| "BA2 reserved chunk count overflowed usize".to_string())?;
    let payload_start = archive_front_size_counts(
        entries.len() as u64,
        reserved_chunk_count as u64,
        &encoder.opts,
    )?;
    let file = File::create(output_path).map_err(|e| format!("output create: {e}"))?;
    file.set_len(payload_start)
        .map_err(|e| format!("output reserve: {e}"))?;

    let memory_budget = InFlightMemory::new(DIRECT_PACK_MEMORY_BUDGET);

    // Workers compress in parallel and hand finished payloads to a single
    // writer thread that appends them sequentially and assigns offsets in
    // arrival order. Sequential appends keep NTFS from zero-filling gaps
    // (positional out-of-order writes are pathologically slow there), and no
    // worker ever blocks on disk I/O — only on the in-flight memory budget,
    // which the writer drains, so the pipeline cannot deadlock.
    let (mut direct_entries, next_offset, mut file) =
        std::thread::scope(|scope| -> PackResult<(Vec<DirectEntry>, u64, File)> {
            let (tx, rx) = std::sync::mpsc::channel::<QueuedDirectEntry<'_>>();
            let writer = scope.spawn(move || -> PackResult<(Vec<DirectEntry>, u64, File)> {
                let mut file = file;
                file.seek(SeekFrom::Start(payload_start))
                    .map_err(|e| format!("output seek: {e}"))?;
                let mut next_offset = payload_start;
                let mut written_entries = Vec::new();
                for item in rx {
                    file.write_all(&item.payload)
                        .map_err(|e| format!("archive payload write: {e}"))?;
                    let mut chunks = Vec::with_capacity(item.chunk_plans.len());
                    for chunk in item.chunk_plans {
                        chunks.push(StreamedChunk {
                            payload_offset: next_offset,
                            packed_len: chunk.packed_len,
                            unpacked_len: chunk.unpacked_len,
                            mips: chunk.mips,
                        });
                        next_offset = next_offset
                            .checked_add(chunk.stored_len)
                            .ok_or_else(|| "BA2 payload offsets overflowed u64".to_string())?;
                    }
                    written_entries.push(DirectEntry {
                        rel_backslash: item.rel_backslash,
                        header: item.header,
                        chunks,
                    });
                }
                Ok((written_entries, next_offset, file))
            });

            let pack_result = entries.par_iter().try_for_each(|entry| -> PackResult<()> {
                let source_len = match entry.source_size {
                    Some(source_size) => source_size,
                    None => entry
                        .full_path
                        .metadata()
                        .map_err(|e| format!("stat {}: {e}", entry.full_path.display()))?
                        .len(),
                };
                let requested_memory = usize::try_from(source_len)
                    .unwrap_or(usize::MAX)
                    .saturating_add((source_len / 8).try_into().unwrap_or(usize::MAX))
                    .saturating_add(1024 * 1024);
                let memory_guard = memory_budget.acquire(requested_memory);
                let prepared = encoder.prepare_file(entry)?;
                tx.send(QueuedDirectEntry {
                    rel_backslash: prepared.rel_backslash,
                    header: prepared.header,
                    chunk_plans: prepared.chunks,
                    payload: prepared.payload,
                    _memory_guard: memory_guard,
                })
                .map_err(|_| "archive payload writer stopped".to_string())
            });
            drop(tx);
            let writer_result = writer
                .join()
                .map_err(|_| "archive payload writer panicked".to_string())?;
            match (pack_result, writer_result) {
                (_, Err(err)) => Err(err),
                (Err(err), _) => Err(err),
                (Ok(()), Ok(written)) => Ok(written),
            }
        })?;

    direct_entries.sort_by(|a, b| a.rel_backslash.cmp(&b.rel_backslash));
    let string_table_offset = next_offset;

    let actual_chunk_count: u64 = direct_entries
        .iter()
        .map(|entry| entry.chunks.len() as u64)
        .sum();
    let actual_front = archive_front_size_counts(
        direct_entries.len() as u64,
        actual_chunk_count,
        &encoder.opts,
    )?;
    if actual_front > payload_start {
        return Err(format!(
            "BA2 front matter ({actual_front} bytes, {actual_chunk_count} chunks) exceeds the \
             reserved region ({payload_start} bytes, {reserved_chunk_count} chunk slots); \
             refusing to overwrite payload"
        ));
    }

    file.seek(SeekFrom::Start(string_table_offset))
        .map_err(|e| format!("string table seek: {e}"))?;
    for entry in &direct_entries {
        let (_, name) = make_hash_and_name(&entry.rel_backslash);
        write_wstring(&mut file, &name)?;
    }

    file.seek(SeekFrom::Start(0))
        .map_err(|e| format!("header seek: {e}"))?;
    write_header(
        &mut file,
        &encoder.opts,
        direct_entries.len(),
        string_table_offset,
    )?;
    for entry in &direct_entries {
        let (hash, _) = make_hash_and_name(&entry.rel_backslash);
        write_file_record_parts(
            &mut file,
            &hash,
            &entry.header,
            &entry.chunks,
            &encoder.opts,
        )?;
    }
    file.flush().map_err(|e| format!("output flush: {e}"))
}

struct ChunkPlan {
    stored_len: u64,
    packed_len: u32,
    unpacked_len: u32,
    mips: Option<Range<u16>>,
}

struct PayloadCopy {
    spill_offset: u64,
    stored_len: u64,
    record_index: usize,
    chunk_index: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{PackEntrySpec, pack_archive_entries};
    use crate::python::{extract_one_impl, list_archive_files};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(tag: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let unique = NEXT.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time went backwards")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "bsarchive-incremental-{tag}-{}-{nanos}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("failed to create temp dir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// (abs path, archive rel path) fixtures: tiny text + 2 MB pseudo-random
    /// blob + an empty file + a sound file (compression-forbidden path).
    fn build_gnrl_fixture_tree(dir: &Path) -> Vec<(PathBuf, String)> {
        let mut blob = vec![0u8; 2 * 1024 * 1024];
        let mut state = 0x1234_5678_u32;
        for b in blob.iter_mut() {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *b = (state >> 24) as u8;
        }
        let files = vec![
            ("Misc/readme.txt", b"hello ba2 world".to_vec()),
            ("Meshes/Generated/a.nif", blob),
            ("Misc/empty.bin", Vec::new()),
            ("Sound/fx/s.xwm", vec![b'S'; 4096]),
        ];
        let mut out = Vec::new();
        for (rel, bytes) in files {
            let abs = dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            fs::create_dir_all(abs.parent().unwrap()).unwrap();
            fs::write(&abs, &bytes).unwrap();
            out.push((abs, rel.to_string()));
        }
        out
    }

    fn oneshot_pack(files: &[(PathBuf, String)], output: &Path, archive_type: &str) {
        let specs: Vec<PackEntrySpec> = files
            .iter()
            .map(|(abs, rel)| PackEntrySpec {
                source_path: abs.clone(),
                archive_path: rel.clone(),
                source_size: fs::metadata(abs).ok().map(|metadata| metadata.len()),
            })
            .collect();
        pack_archive_entries(&specs, output, archive_type, true, 9, false, None, None)
            .expect("one-shot pack");
    }

    fn assert_archives_equivalent(oneshot: &Path, incr: &Path, files: &[(PathBuf, String)]) {
        // Content oracle: same file list + same extracted bytes per file.
        assert_eq!(
            list_archive_files(oneshot).unwrap(),
            list_archive_files(incr).unwrap()
        );
        for (_, rel) in files {
            assert_eq!(
                extract_one_impl(oneshot, rel).unwrap(),
                extract_one_impl(incr, rel).unwrap(),
                "extracted bytes differ for {rel}"
            );
        }
    }

    #[test]
    fn incremental_gnrl_archive_matches_oneshot_pack() {
        let tmp = TestDir::new("gnrl");
        let files = build_gnrl_fixture_tree(tmp.path());

        let oneshot = tmp.path().join("oneshot.ba2");
        oneshot_pack(&files, &oneshot, "fo4");

        let w = IncrementalFo4Writer::new(
            tmp.path().join("spill.bin"),
            Fo4WriterKind::Gnrl,
            CompressionSettings::default(),
        )
        .unwrap();
        // Arrival order REVERSED vs final order: add() must be order-free.
        for (abs, rel) in files.iter().rev() {
            assert!(w.add_file(rel, abs).unwrap());
        }
        // Duplicate add is first-wins.
        assert!(!w.add_file(&files[0].1, &files[0].0).unwrap());
        assert_eq!(w.entry_count(), files.len());

        let incr = tmp.path().join("incr.ba2");
        let order: Vec<&str> = files.iter().map(|(_, rel)| rel.as_str()).collect();
        w.finalize(&incr, &order).unwrap();

        assert_archives_equivalent(&oneshot, &incr, &files);
    }

    #[test]
    fn incremental_dx10_archive_matches_oneshot_pack() {
        let fixture = Path::new("data/fo4_dds_test/Fence006_1K_Roughness.dds");
        assert!(
            fixture.is_file(),
            "missing DDS fixture: {}",
            fixture.display()
        );
        let tmp = TestDir::new("dx10");
        let abs = tmp.path().join("Fence006_1K_Roughness.dds");
        fs::copy(fixture, &abs).unwrap();
        let files = vec![(abs, "Textures/Fence006_1K_Roughness.dds".to_string())];

        let oneshot = tmp.path().join("oneshot.ba2");
        oneshot_pack(&files, &oneshot, "fo4dds");

        let w = IncrementalFo4Writer::new(
            tmp.path().join("spill.bin"),
            Fo4WriterKind::Dx10,
            CompressionSettings::default(),
        )
        .unwrap();
        for (abs, rel) in &files {
            assert!(w.add_file(rel, abs).unwrap());
        }
        let incr = tmp.path().join("incr.ba2");
        let order: Vec<&str> = files.iter().map(|(_, rel)| rel.as_str()).collect();
        w.finalize(&incr, &order).unwrap();

        assert_archives_equivalent(&oneshot, &incr, &files);
    }

    #[test]
    fn dx10_writer_rejects_non_dds() {
        let tmp = TestDir::new("dx10rej");
        let w = IncrementalFo4Writer::new(
            tmp.path().join("spill.bin"),
            Fo4WriterKind::Dx10,
            CompressionSettings::default(),
        )
        .unwrap();
        let err = w.add_bytes("Meshes/a.nif", b"not dds").unwrap_err();
        assert!(err.contains("DDS"), "got: {err}");
    }

    #[test]
    fn finalize_subset_membership() {
        let tmp = TestDir::new("subset");
        let files = build_gnrl_fixture_tree(tmp.path());
        let w = IncrementalFo4Writer::new(
            tmp.path().join("spill.bin"),
            Fo4WriterKind::Gnrl,
            CompressionSettings::default(),
        )
        .unwrap();
        for (abs, rel) in &files {
            w.add_file(rel, abs).unwrap();
        }
        // Finalize only two of the four — one spill can feed multiple shards.
        let incr = tmp.path().join("subset.ba2");
        w.finalize(&incr, &["Misc/readme.txt", "Sound/fx/s.xwm"])
            .unwrap();
        let listed = list_archive_files(&incr).unwrap();
        assert_eq!(listed.len(), 2);

        let oneshot = tmp.path().join("subset_oneshot.ba2");
        let subset: Vec<(PathBuf, String)> = files
            .iter()
            .filter(|(_, rel)| rel == "Misc/readme.txt" || rel == "Sound/fx/s.xwm")
            .cloned()
            .collect();
        oneshot_pack(&subset, &oneshot, "fo4");
        assert_archives_equivalent(&oneshot, &incr, &subset);
    }

    #[test]
    fn compression_settings_for_writer_kind_match_format_defaults() {
        assert_eq!(
            CompressionSettings::for_writer_kind(Fo4WriterKind::Dx10).compression_level,
            4
        );
        assert_eq!(
            CompressionSettings::for_writer_kind(Fo4WriterKind::Gnrl).compression_level,
            6
        );
    }

    #[test]
    fn finalize_unknown_rel_errors() {
        let tmp = TestDir::new("unknown");
        let w = IncrementalFo4Writer::new(
            tmp.path().join("spill.bin"),
            Fo4WriterKind::Gnrl,
            CompressionSettings::default(),
        )
        .unwrap();
        let err = w
            .finalize(&tmp.path().join("x.ba2"), &["Misc/never_added.txt"])
            .unwrap_err();
        assert!(err.contains("never added"), "got: {err}");
    }
}
