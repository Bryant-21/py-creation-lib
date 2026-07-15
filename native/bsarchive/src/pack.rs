use crate::incremental::{CompressionSettings, Fo4WriterKind, pack_fo4_direct};
use crate::{
    Borrowed, CompressionResult, ReaderWithOptions as _, containers::Bytes, fo4, pack_fo4_stream,
    tes4,
};
use bstr::BString;
use rayon::prelude::*;
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

type PackResult<T> = Result<T, String>;

#[derive(Clone, Copy)]
enum PackKind {
    Tes4 {
        version: tes4::Version,
    },
    Fo4 {
        version: fo4::Version,
        format: fo4::Format,
        compression_format: fo4::CompressionFormat,
        force_compress: bool,
        xbox_profile: bool,
    },
}

pub(crate) struct FileEntry {
    pub(crate) rel_slash: String,
    pub(crate) rel_slash_lower: String,
    pub(crate) rel_backslash: String,
    pub(crate) rel_backslash_lower: String,
    pub(crate) full_path: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct PackEntrySpec {
    pub(crate) source_path: PathBuf,
    pub(crate) archive_path: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PackFilters {
    pub(crate) include_prefixes: Vec<String>,
    pub(crate) exclude_prefixes: Vec<String>,
}

impl PackFilters {
    pub(crate) fn new(include_prefixes: Vec<String>, exclude_prefixes: Vec<String>) -> Self {
        Self {
            include_prefixes: normalize_prefixes(include_prefixes),
            exclude_prefixes: normalize_prefixes(exclude_prefixes),
        }
    }

    pub(crate) fn matches(&self, rel_path: &str) -> bool {
        let rel_path = normalize_filter_path(rel_path);
        let included = self.include_prefixes.is_empty()
            || self
                .include_prefixes
                .iter()
                .any(|prefix| rel_path.starts_with(prefix));
        let excluded = self
            .exclude_prefixes
            .iter()
            .any(|prefix| rel_path.starts_with(prefix));
        included && !excluded
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompressionPolicy {
    Allow,
    Forbid,
}

pub(crate) fn pack_archive(
    source_dir: &Path,
    output_path: &Path,
    archive_type: &str,
    compress: bool,
    compression_level: u32,
    share_data: bool,
    manifest_path: Option<&Path>,
    jobs: Option<usize>,
    filters: PackFilters,
) -> PackResult<usize> {
    let kind = parse_pack_kind(archive_type)?;
    let entries = collect_files(source_dir, manifest_path, &filters)?;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let run = || -> PackResult<()> {
        let mut out = fs::File::create(output_path).map_err(|err| err.to_string())?;
        match kind {
            PackKind::Tes4 { version } => {
                let (archive, options) =
                    build_tes4_archive(&entries, version, compress, compression_level, share_data)?;
                archive
                    .write(&mut out, &options)
                    .map_err(|err| err.to_string())?;
            }
            PackKind::Fo4 {
                version,
                format,
                compression_format,
                force_compress,
                xbox_profile,
            } if !share_data => {
                if let Some(writer_kind) = incremental_writer_kind(
                    version,
                    format,
                    compression_format,
                    force_compress,
                    xbox_profile,
                ) {
                    drop(out);
                    pack_fo4_direct_entries(
                        &entries,
                        output_path,
                        writer_kind,
                        compress,
                        compression_level,
                    )?;
                } else {
                    pack_fo4_stream::pack_archive(
                        &entries,
                        &mut out,
                        output_path,
                        version,
                        format,
                        compression_format,
                        compress,
                        compression_level,
                        force_compress,
                        xbox_profile,
                    )?;
                }
            }
            PackKind::Fo4 {
                version,
                format,
                compression_format,
                force_compress,
                xbox_profile,
            } => {
                let (archive, order) = build_fo4_archive(
                    &entries,
                    version,
                    format,
                    compression_format,
                    compress,
                    compression_level,
                    force_compress,
                    xbox_profile,
                    share_data,
                )?;
                let options = fo4::ArchiveOptions::builder()
                    .version(version)
                    .format(format)
                    .compression_format(compression_format)
                    .share_data(share_data)
                    .strings(true)
                    .build();
                archive
                    .write_in_order(&mut out, &options, &order)
                    .map_err(|err| err.to_string())?;
            }
        }
        Ok(())
    };

    if let Some(jobs) = jobs {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs.max(1))
            .build()
            .map_err(|err| format!("rayon pool error: {err}"))?;
        pool.install(run)?;
    } else {
        run()?;
    }

    Ok(entries.len())
}

pub(crate) fn pack_archive_entries(
    entries: &[PackEntrySpec],
    output_path: &Path,
    archive_type: &str,
    compress: bool,
    compression_level: u32,
    share_data: bool,
    manifest_path: Option<&Path>,
    jobs: Option<usize>,
) -> PackResult<usize> {
    let kind = parse_pack_kind(archive_type)?;
    let mut entries = collect_entry_specs(entries)?;
    if let Some(manifest_path) = manifest_path {
        if manifest_path.is_file() {
            entries = apply_manifest(entries, manifest_path)?;
        }
    }
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let run = || -> PackResult<()> {
        let mut out = fs::File::create(output_path).map_err(|err| err.to_string())?;
        match kind {
            PackKind::Tes4 { version } => {
                let (archive, options) =
                    build_tes4_archive(&entries, version, compress, compression_level, share_data)?;
                archive
                    .write(&mut out, &options)
                    .map_err(|err| err.to_string())?;
            }
            PackKind::Fo4 {
                version,
                format,
                compression_format,
                force_compress,
                xbox_profile,
            } if !share_data => {
                if let Some(writer_kind) = incremental_writer_kind(
                    version,
                    format,
                    compression_format,
                    force_compress,
                    xbox_profile,
                ) {
                    drop(out);
                    pack_fo4_direct_entries(
                        &entries,
                        output_path,
                        writer_kind,
                        compress,
                        compression_level,
                    )?;
                } else {
                    pack_fo4_stream::pack_archive(
                        &entries,
                        &mut out,
                        output_path,
                        version,
                        format,
                        compression_format,
                        compress,
                        compression_level,
                        force_compress,
                        xbox_profile,
                    )?;
                }
            }
            PackKind::Fo4 {
                version,
                format,
                compression_format,
                force_compress,
                xbox_profile,
            } => {
                let (archive, order) = build_fo4_archive(
                    &entries,
                    version,
                    format,
                    compression_format,
                    compress,
                    compression_level,
                    force_compress,
                    xbox_profile,
                    share_data,
                )?;
                let options = fo4::ArchiveOptions::builder()
                    .version(version)
                    .format(format)
                    .compression_format(compression_format)
                    .share_data(share_data)
                    .strings(true)
                    .build();
                archive
                    .write_in_order(&mut out, &options, &order)
                    .map_err(|err| err.to_string())?;
            }
        }
        Ok(())
    };

    if let Some(jobs) = jobs {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs.max(1))
            .build()
            .map_err(|err| format!("rayon pool error: {err}"))?;
        pool.install(run)?;
    } else {
        run()?;
    }

    Ok(entries.len())
}

fn incremental_writer_kind(
    version: fo4::Version,
    format: fo4::Format,
    compression_format: fo4::CompressionFormat,
    force_compress: bool,
    xbox_profile: bool,
) -> Option<Fo4WriterKind> {
    match (
        version,
        format,
        compression_format,
        force_compress,
        xbox_profile,
    ) {
        (fo4::Version::v8, fo4::Format::GNRL, fo4::CompressionFormat::Zip, false, false) => {
            Some(Fo4WriterKind::Gnrl)
        }
        (fo4::Version::v8, fo4::Format::DX10, fo4::CompressionFormat::Zip, true, false) => {
            Some(Fo4WriterKind::Dx10)
        }
        _ => None,
    }
}

fn pack_fo4_direct_entries(
    entries: &[FileEntry],
    output_path: &Path,
    writer_kind: Fo4WriterKind,
    compress: bool,
    compression_level: u32,
) -> PackResult<()> {
    pack_fo4_direct(
        entries,
        output_path,
        writer_kind,
        CompressionSettings {
            compress,
            compression_level,
        },
    )
}

fn parse_pack_kind(value: &str) -> PackResult<PackKind> {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "tes4" | "oblivion" => Ok(PackKind::Tes4 {
            version: tes4::Version::v103,
        }),
        "fo3" | "fonv" | "tes5" | "skyrim" => Ok(PackKind::Tes4 {
            version: tes4::Version::v104,
        }),
        "sse" | "skyrimse" => Ok(PackKind::Tes4 {
            version: tes4::Version::v105,
        }),
        "fo76" | "fo4og" => Ok(PackKind::Fo4 {
            version: fo4::Version::v1,
            format: fo4::Format::GNRL,
            compression_format: fo4::CompressionFormat::Zip,
            force_compress: false,
            xbox_profile: false,
        }),
        "fo76dds" | "fo4ogdds" => Ok(PackKind::Fo4 {
            version: fo4::Version::v1,
            format: fo4::Format::DX10,
            compression_format: fo4::CompressionFormat::Zip,
            force_compress: true,
            xbox_profile: false,
        }),
        "fo4" => Ok(PackKind::Fo4 {
            version: fo4::Version::v8,
            format: fo4::Format::GNRL,
            compression_format: fo4::CompressionFormat::Zip,
            force_compress: false,
            xbox_profile: false,
        }),
        "fo4dds" => Ok(PackKind::Fo4 {
            version: fo4::Version::v8,
            format: fo4::Format::DX10,
            compression_format: fo4::CompressionFormat::Zip,
            force_compress: true,
            xbox_profile: false,
        }),
        "fo4xbox" => Ok(PackKind::Fo4 {
            version: fo4::Version::v8,
            format: fo4::Format::GNRL,
            compression_format: fo4::CompressionFormat::Zip,
            force_compress: false,
            xbox_profile: true,
        }),
        "fo4xboxdds" => Ok(PackKind::Fo4 {
            version: fo4::Version::v8,
            format: fo4::Format::DX10,
            compression_format: fo4::CompressionFormat::Zip,
            force_compress: true,
            xbox_profile: true,
        }),
        "starfield" | "sf" => Ok(PackKind::Fo4 {
            version: fo4::Version::v2,
            format: fo4::Format::GNRL,
            compression_format: fo4::CompressionFormat::Zip,
            force_compress: false,
            xbox_profile: false,
        }),
        "starfielddds" | "sfdds" => Ok(PackKind::Fo4 {
            version: fo4::Version::v3,
            format: fo4::Format::DX10,
            compression_format: fo4::CompressionFormat::LZ4,
            force_compress: true,
            xbox_profile: false,
        }),
        _ => Err(format!("unsupported archive type: {value}")),
    }
}

pub(crate) fn archive_type_default_level(archive_type: &str) -> u32 {
    match parse_pack_kind(archive_type) {
        Ok(PackKind::Fo4 { format, .. }) => {
            crate::pack_fo4_stream::fo4_default_compression_level(format)
        }
        // TES4/BSA has no DX10/GNRL split; keep the historical pyfunction default.
        _ => crate::pack_fo4_stream::GNRL_COMPRESSION_LEVEL,
    }
}

fn normalize_filter_path(path: &str) -> String {
    let mut value = path.replace('\\', "/").to_ascii_lowercase();
    while value.starts_with('/') {
        value.remove(0);
    }
    while value.starts_with("./") {
        value.drain(..2);
    }
    value
}

fn normalize_prefixes(prefixes: Vec<String>) -> Vec<String> {
    let mut prefixes: Vec<_> = prefixes
        .into_iter()
        .filter_map(|prefix| {
            let mut value = normalize_filter_path(prefix.trim());
            if !value.is_empty() && !value.ends_with('/') {
                value.push('/');
            }
            (!value.is_empty()).then_some(value)
        })
        .collect();
    prefixes.sort();
    prefixes.dedup();
    prefixes
}

fn collect_files(
    source_dir: &Path,
    manifest_path: Option<&Path>,
    filters: &PackFilters,
) -> PackResult<Vec<FileEntry>> {
    if !source_dir.is_dir() {
        return Err(format!(
            "source directory not found: {}",
            source_dir.display()
        ));
    }

    let mut entries = Vec::new();
    for entry in WalkDir::new(source_dir) {
        let entry = entry.map_err(|err| err.to_string())?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(source_dir)
            .map_err(|err| err.to_string())?;
        let rel_slash = normalize_slashes(rel);
        let rel_slash_lower = rel_slash.to_ascii_lowercase();
        if !filters.matches(&rel_slash_lower) {
            continue;
        }
        let rel_backslash = rel_slash.replace('/', "\\");
        let rel_backslash_lower = rel_backslash.to_ascii_lowercase();
        entries.push(FileEntry {
            rel_slash,
            rel_slash_lower,
            rel_backslash,
            rel_backslash_lower,
            full_path: entry.path().to_path_buf(),
        });
    }
    if let Some(manifest_path) = manifest_path {
        if manifest_path.is_file() {
            return apply_manifest(entries, manifest_path);
        }
    }
    entries.sort_by(|a, b| a.rel_backslash.cmp(&b.rel_backslash));
    Ok(entries)
}

fn collect_entry_specs(entries: &[PackEntrySpec]) -> PackResult<Vec<FileEntry>> {
    let mut collected = Vec::with_capacity(entries.len());
    let mut seen = std::collections::HashSet::with_capacity(entries.len());
    for entry in entries {
        if !entry.source_path.is_file() {
            return Err(format!(
                "planned archive source file not found: {}",
                entry.source_path.display()
            ));
        }
        let rel_slash = normalize_archive_entry_path(&entry.archive_path)?;
        let rel_slash_lower = rel_slash.to_ascii_lowercase();
        if !seen.insert(rel_slash_lower.clone()) {
            return Err(format!(
                "duplicate archive path in planned entries: {rel_slash}"
            ));
        }
        let rel_backslash = rel_slash.replace('/', "\\");
        let rel_backslash_lower = rel_backslash.to_ascii_lowercase();
        collected.push(FileEntry {
            rel_slash,
            rel_slash_lower,
            rel_backslash,
            rel_backslash_lower,
            full_path: entry.source_path.clone(),
        });
    }
    collected.sort_by(|a, b| a.rel_backslash.cmp(&b.rel_backslash));
    Ok(collected)
}

pub(crate) fn normalize_archive_entry_path(path: &str) -> PackResult<String> {
    let trimmed = path.trim();
    let rooted_path = trimmed.starts_with('/') || trimmed.starts_with('\\');
    let normalized = trimmed
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string();
    let unsafe_path = rooted_path
        || normalized.is_empty()
        || normalized.contains(':')
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..");
    if unsafe_path {
        return Err(format!("unsafe archive path: {path}"));
    }
    Ok(normalized)
}

fn normalize_slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn apply_manifest(entries: Vec<FileEntry>, manifest_path: &Path) -> PackResult<Vec<FileEntry>> {
    let manifest = load_manifest_paths(manifest_path)?;
    let mut by_lower = HashMap::with_capacity(entries.len());
    for entry in entries {
        by_lower.insert(entry.rel_backslash_lower.clone(), entry);
    }

    let mut ordered = Vec::with_capacity(by_lower.len());
    for raw_path in manifest {
        let canonical_backslash = canonicalize_manifest_path(&raw_path);
        let canonical_backslash_lower = canonical_backslash.to_ascii_lowercase();
        if let Some(mut entry) = by_lower.remove(&canonical_backslash_lower) {
            entry.rel_backslash = canonical_backslash.clone();
            entry.rel_backslash_lower = canonical_backslash_lower;
            entry.rel_slash = canonical_backslash.replace('\\', "/");
            entry.rel_slash_lower = entry.rel_slash.to_ascii_lowercase();
            ordered.push(entry);
        }
    }

    let mut remaining: Vec<_> = by_lower.into_values().collect();
    remaining.sort_by(|a, b| a.rel_backslash.cmp(&b.rel_backslash));
    ordered.extend(remaining);
    Ok(ordered)
}

fn canonicalize_manifest_path(path: &str) -> String {
    let path = path.replace('/', "\\");
    let path = path
        .strip_prefix("Data\\")
        .or_else(|| path.strip_prefix("data\\"))
        .unwrap_or(&path);
    path.to_string()
}

fn load_manifest_paths(manifest_path: &Path) -> PackResult<Vec<String>> {
    let raw = fs::read_to_string(manifest_path).map_err(|err| err.to_string())?;
    let value: Value = serde_json::from_str(&raw).map_err(|err| err.to_string())?;
    let Some(items) = value.as_array() else {
        return Err(format!(
            "archive manifest must be a JSON array: {}",
            manifest_path.display()
        ));
    };
    let mut paths = Vec::with_capacity(items.len());
    for item in items {
        let Some(path) = item.as_str() else {
            return Err(format!(
                "archive manifest contains a non-string entry: {}",
                manifest_path.display()
            ));
        };
        paths.push(path.to_string());
    }
    Ok(paths)
}

fn split_tes4_path(path: &str) -> (String, String) {
    match path.rsplit_once('\\') {
        Some((dir, file)) if !dir.is_empty() => (dir.to_string(), file.to_string()),
        _ => (".".to_string(), path.to_string()),
    }
}

fn build_tes4_archive(
    entries: &[FileEntry],
    version: tes4::Version,
    compress: bool,
    compression_level: u32,
    share_data: bool,
) -> PackResult<(tes4::Archive<'static>, tes4::ArchiveOptions)> {
    let mut archive = tes4::Archive::new();
    let mut archive_types = tes4::ArchiveTypes::empty();
    let mut archive_flags =
        tes4::ArchiveFlags::DIRECTORY_STRINGS | tes4::ArchiveFlags::FILE_STRINGS;
    let compression_options = tes4::FileCompressionOptions::builder()
        .version(version)
        .zlib_level(compression_level)
        .build();

    struct Tes4Built {
        dir_name: String,
        file_name: String,
        file: tes4::File<'static>,
        types: tes4::ArchiveTypes,
        flags: tes4::ArchiveFlags,
        saw_pex: bool,
    }

    let built: Vec<Tes4Built> = entries
        .par_iter()
        .map(|entry| -> PackResult<Tes4Built> {
            let mut types = tes4::ArchiveTypes::empty();
            let mut flags = tes4::ArchiveFlags::empty();
            let mut saw_pex = false;
            infer_tes4_flags(&entry.rel_slash_lower, &mut types, &mut flags, &mut saw_pex);

            let bytes = fs::read(&entry.full_path).map_err(|err| err.to_string())?;
            let mut file = <tes4::File as crate::CompressableFrom<Box<[u8]>>>::from_decompressed(
                bytes.into_boxed_slice(),
            );
            if compress && compression_policy(&entry.rel_slash_lower).allows_compression() {
                let compressed = file
                    .compress(&compression_options)
                    .map_err(|err| err.to_string())?;
                if should_keep_compressed(file.len(), compressed.len(), false) {
                    file = compressed;
                }
            }

            let (dir_name, file_name) = split_tes4_path(&entry.rel_backslash);
            Ok(Tes4Built {
                dir_name,
                file_name,
                file,
                types,
                flags,
                saw_pex,
            })
        })
        .collect::<PackResult<Vec<_>>>()?;

    let mut saw_pex = false;
    for item in built {
        archive_types.insert(item.types);
        archive_flags.insert(item.flags);
        saw_pex |= item.saw_pex;
        let dir_key = tes4::ArchiveKey::from(item.dir_name.as_str());
        let file_key = tes4::DirectoryKey::from(item.file_name.as_str());
        if let Some(directory) = archive.get_mut(&dir_key) {
            directory.insert(file_key, item.file);
        } else {
            let mut directory = tes4::Directory::new();
            directory.insert(file_key, item.file);
            archive.insert(dir_key, directory);
        }
    }

    if archive_types == tes4::ArchiveTypes::TEXTURES && version != tes4::Version::v105 {
        archive_flags.insert(tes4::ArchiveFlags::EMBEDDED_FILE_NAMES);
    }
    if archive_types.intersects(tes4::ArchiveTypes::MESHES) {
        archive_flags.insert(tes4::ArchiveFlags::RETAIN_STRINGS_DURING_STARTUP);
    }
    if archive_types.intersects(tes4::ArchiveTypes::SOUNDS | tes4::ArchiveTypes::VOICES) || saw_pex
    {
        archive_flags.insert(tes4::ArchiveFlags::RETAIN_FILE_NAMES);
    }
    if compress {
        archive_flags.insert(tes4::ArchiveFlags::COMPRESSED);
    }

    let options = tes4::ArchiveOptions::builder()
        .version(version)
        .flags(archive_flags)
        .share_data(share_data)
        .types(archive_types)
        .build();
    Ok((archive, options))
}

fn infer_tes4_flags(
    rel_path: &str,
    archive_types: &mut tes4::ArchiveTypes,
    archive_flags: &mut tes4::ArchiveFlags,
    saw_pex: &mut bool,
) {
    let ext = rel_path
        .rsplit_once('.')
        .map(|(_, ext)| format!(".{ext}"))
        .unwrap_or_default();

    if rel_path.starts_with("meshes/") {
        archive_types.insert(tes4::ArchiveTypes::MESHES);
    } else if rel_path.starts_with("textures/") {
        archive_types.insert(tes4::ArchiveTypes::TEXTURES);
    } else if rel_path.starts_with("sound/voice/") {
        archive_types.insert(tes4::ArchiveTypes::VOICES);
    } else if rel_path.starts_with("sound/") {
        archive_types.insert(tes4::ArchiveTypes::SOUNDS);
    } else if rel_path.starts_with("menus/") {
        archive_types.insert(tes4::ArchiveTypes::MENUS);
    } else if rel_path.starts_with("shaders/") {
        archive_types.insert(tes4::ArchiveTypes::SHADERS);
    } else if rel_path.starts_with("trees/") {
        archive_types.insert(tes4::ArchiveTypes::TREES);
    } else if rel_path.starts_with("fonts/") {
        archive_types.insert(tes4::ArchiveTypes::FONTS);
    } else {
        match ext.as_str() {
            ".nif" | ".lod" | ".bto" | ".btr" | ".btt" | ".dtl" | ".kf" | ".kfm" | ".hkx" => {
                archive_types.insert(tes4::ArchiveTypes::MESHES)
            }
            ".dds" => archive_types.insert(tes4::ArchiveTypes::TEXTURES),
            ".wav" | ".fuz" | ".lip" | ".mp3" | ".ogg" => {
                archive_types.insert(tes4::ArchiveTypes::SOUNDS)
            }
            ".xml" | ".txt" | ".htm" | ".bat" | ".scc" => {
                archive_types.insert(tes4::ArchiveTypes::MENUS)
            }
            ".spt" => archive_types.insert(tes4::ArchiveTypes::TREES),
            ".fnt" | ".tex" => archive_types.insert(tes4::ArchiveTypes::FONTS),
            _ => archive_types.insert(tes4::ArchiveTypes::MISC),
        }
    }

    if ext == ".pex" {
        *saw_pex = true;
        archive_flags.insert(tes4::ArchiveFlags::RETAIN_FILE_NAMES);
    }
}

fn build_fo4_archive(
    entries: &[FileEntry],
    _version: fo4::Version,
    format: fo4::Format,
    compression_format: fo4::CompressionFormat,
    compress: bool,
    compression_level: u32,
    force_compress: bool,
    xbox_profile: bool,
    _share_data: bool,
) -> PackResult<(fo4::Archive<'static>, Vec<fo4::FileHash>)> {
    let mut archive = fo4::Archive::new();
    let compression_level = if xbox_profile {
        fo4::CompressionLevel::FO4Xbox
    } else {
        fo4::CompressionLevel::Custom(compression_level)
    };
    let read_options = fo4::FileReadOptions::builder()
        .format(format)
        .compression_format(compression_format)
        .compression_level(compression_level)
        .compression_result(CompressionResult::Decompressed)
        .build();
    let chunk_options = fo4::ChunkCompressionOptions::builder()
        .compression_format(compression_format)
        .compression_level(compression_level)
        .build();

    let built: Vec<(fo4::ArchiveKey<'static>, fo4::File<'static>)> = entries
        .par_iter()
        .map(
            |entry| -> PackResult<(fo4::ArchiveKey<'static>, fo4::File<'static>)> {
                if format == fo4::Format::DX10 && !entry.rel_slash_lower.ends_with(".dds") {
                    return Err(format!(
                        "DX10 archives can only contain DDS files: {}",
                        entry.rel_slash
                    ));
                }

                let bytes = fs::read(&entry.full_path).map_err(|err| err.to_string())?;
                let source_file = fo4::File::read(Borrowed(&bytes), &read_options)
                    .map_err(|err| err.to_string())?;
                let allow_compression =
                    compression_policy(&entry.rel_slash_lower).allows_compression();

                let mut chunks = Vec::with_capacity(source_file.len());
                for chunk in &source_file {
                    if force_compress || (compress && allow_compression) {
                        let compressed = chunk
                            .compress(&chunk_options)
                            .map_err(|err| err.to_string())?;
                        if should_keep_compressed(chunk.len(), compressed.len(), force_compress) {
                            chunks.push(compressed);
                            continue;
                        }
                    }
                    chunks.push(chunk.clone().into_owned());
                }

                let key = make_fo4_key(&entry.rel_backslash);
                Ok((
                    key,
                    fo4::File {
                        chunks,
                        header: source_file.header.clone(),
                    },
                ))
            },
        )
        .collect::<PackResult<Vec<_>>>()?;

    let mut order = Vec::with_capacity(built.len());
    for (key, file) in built {
        order.push(*key.hash());
        archive.insert(key, file);
    }

    Ok((archive, order))
}

pub(crate) fn should_keep_compressed(
    original_len: usize,
    compressed_len: usize,
    force_compress: bool,
) -> bool {
    force_compress || compressed_len.saturating_add(32) < original_len
}

pub(crate) fn allows_compression_for_path(rel_slash: &str) -> bool {
    compression_policy(rel_slash).allows_compression()
}

fn compression_policy(rel_slash: &str) -> CompressionPolicy {
    let path = rel_slash.trim_start_matches("./");
    if path.ends_with(".fuz") {
        return CompressionPolicy::Allow;
    }
    if path.starts_with("sound/") || path.starts_with("music/") || path.starts_with("strings/") {
        CompressionPolicy::Forbid
    } else {
        CompressionPolicy::Allow
    }
}

impl CompressionPolicy {
    fn allows_compression(self) -> bool {
        matches!(self, Self::Allow)
    }
}

fn make_fo4_key(path_backslash: &str) -> fo4::ArchiveKey<'static> {
    let mut normalized = BString::from(path_backslash);
    let hash = fo4::hash_file_in_place(&mut normalized);
    fo4::ArchiveKey {
        hash,
        name: Bytes::from_owned(path_backslash.as_bytes().to_vec().into_boxed_slice()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Reader as _;
    use anyhow::Context as _;
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time went backwards")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "bsarchive-native-pack-{}-{nanos}-{unique}",
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

    fn assert_dx10_dds_payload_matches(original: &[u8], extracted: &[u8]) {
        assert_eq!(extracted.len(), original.len());
        assert_eq!(&extracted[148..], &original[148..]);
    }

    #[test]
    fn incremental_writer_route_is_fo4_v8_only() {
        assert_eq!(
            incremental_writer_kind(
                fo4::Version::v8,
                fo4::Format::GNRL,
                fo4::CompressionFormat::Zip,
                false,
                false,
            ),
            Some(Fo4WriterKind::Gnrl)
        );
        assert_eq!(
            incremental_writer_kind(
                fo4::Version::v8,
                fo4::Format::DX10,
                fo4::CompressionFormat::Zip,
                true,
                false,
            ),
            Some(Fo4WriterKind::Dx10)
        );
        assert_eq!(
            incremental_writer_kind(
                fo4::Version::v1,
                fo4::Format::DX10,
                fo4::CompressionFormat::Zip,
                true,
                false,
            ),
            None
        );
        assert_eq!(
            incremental_writer_kind(
                fo4::Version::v8,
                fo4::Format::DX10,
                fo4::CompressionFormat::Zip,
                true,
                true,
            ),
            None
        );
    }

    #[test]
    fn tes4_pack_skips_compression_for_wav_files() -> anyhow::Result<()> {
        let dir = TestDir::new();
        let source_dir = dir.path().join("src");
        fs::create_dir_all(source_dir.join("sound"))?;
        fs::create_dir_all(source_dir.join("misc"))?;
        fs::write(source_dir.join("sound").join("test.wav"), vec![b'A'; 8192])?;
        fs::write(source_dir.join("misc").join("test.txt"), vec![b'B'; 8192])?;

        let archive_path = dir.path().join("out.bsa");
        let written = pack_archive(
            &source_dir,
            &archive_path,
            "fo3",
            true,
            9,
            false,
            None,
            None,
            PackFilters::default(),
        )
        .map_err(anyhow::Error::msg)?;
        assert_eq!(written, 2);

        let (archive, options) = tes4::Archive::read(archive_path.as_path())?;
        assert!(options.flags().compressed());

        let sound_dir = archive
            .get(&tes4::ArchiveKey::from("sound"))
            .expect("missing sound directory");
        let wav = sound_dir
            .get(&tes4::DirectoryKey::from("test.wav"))
            .expect("missing wav");
        assert!(!wav.is_compressed());

        let misc_dir = archive
            .get(&tes4::ArchiveKey::from("misc"))
            .expect("missing misc directory");
        let txt = misc_dir
            .get(&tes4::DirectoryKey::from("test.txt"))
            .expect("missing txt");
        assert!(txt.is_compressed());

        Ok(())
    }

    #[test]
    fn fo4_pack_skips_compression_for_wav_files() -> anyhow::Result<()> {
        let dir = TestDir::new();
        let source_dir = dir.path().join("src");
        fs::create_dir_all(source_dir.join("sound"))?;
        fs::create_dir_all(source_dir.join("misc"))?;
        fs::write(source_dir.join("sound").join("test.wav"), vec![b'A'; 8192])?;
        fs::write(source_dir.join("misc").join("test.txt"), vec![b'B'; 8192])?;

        let archive_path = dir.path().join("out.ba2");
        let written = pack_archive(
            &source_dir,
            &archive_path,
            "fo4",
            true,
            9,
            false,
            None,
            None,
            PackFilters::default(),
        )
        .map_err(anyhow::Error::msg)?;
        assert_eq!(written, 2);

        let (archive, options) = fo4::Archive::read(archive_path.as_path())?;
        assert_eq!(options.version(), fo4::Version::v8);
        let wav = archive
            .get(&fo4::ArchiveKey::from("sound/test.wav"))
            .expect("missing wav");
        assert!(wav.iter().all(|chunk| !chunk.is_compressed()));

        let txt = archive
            .get(&fo4::ArchiveKey::from("misc/test.txt"))
            .expect("missing txt");
        assert!(txt.iter().any(fo4::Chunk::is_compressed));

        Ok(())
    }

    #[test]
    fn fo4_pack_keeps_tiny_gnrl_chunks_raw_when_compression_saves_too_little() -> anyhow::Result<()>
    {
        let dir = TestDir::new();
        let source_dir = dir.path().join("src");
        fs::create_dir_all(source_dir.join("misc"))?;
        fs::write(source_dir.join("misc").join("tiny.bin"), b"tiny payload")?;
        fs::write(
            source_dir.join("misc").join("compressible.txt"),
            vec![b'C'; 8192],
        )?;

        let archive_path = dir.path().join("out.ba2");
        let written = pack_archive(
            &source_dir,
            &archive_path,
            "fo4",
            true,
            9,
            false,
            None,
            None,
            PackFilters::default(),
        )
        .map_err(anyhow::Error::msg)?;
        assert_eq!(written, 2);

        let (archive, options) = fo4::Archive::read(archive_path.as_path())?;
        assert_eq!(options.version(), fo4::Version::v8);
        let tiny = archive
            .get(&fo4::ArchiveKey::from("misc/tiny.bin"))
            .expect("missing tiny file");
        assert!(tiny.iter().all(|chunk| !chunk.is_compressed()));

        let compressible = archive
            .get(&fo4::ArchiveKey::from("misc/compressible.txt"))
            .expect("missing compressible file");
        assert!(compressible.iter().any(fo4::Chunk::is_compressed));

        Ok(())
    }

    #[test]
    fn fo4_pack_entries_uses_archive_paths_without_common_source_root() -> anyhow::Result<()> {
        let dir = TestDir::new();
        let source_a = dir.path().join("source_a");
        let source_b = dir.path().join("source_b");
        fs::create_dir_all(&source_a)?;
        fs::create_dir_all(&source_b)?;
        let mesh_path = source_a.join("a.nif");
        let sound_path = source_b.join("b.wav");
        fs::write(&mesh_path, b"mesh")?;
        fs::write(&sound_path, vec![b'S'; 8192])?;

        let archive_path = dir.path().join("out.ba2");
        let written = pack_archive_entries(
            &[
                PackEntrySpec {
                    source_path: mesh_path,
                    archive_path: "Meshes/Generated/a.nif".to_string(),
                },
                PackEntrySpec {
                    source_path: sound_path,
                    archive_path: "Sound/Generated/b.wav".to_string(),
                },
            ],
            &archive_path,
            "fo4",
            true,
            9,
            false,
            None,
            None,
        )
        .map_err(anyhow::Error::msg)?;
        assert_eq!(written, 2);

        let (archive, options) = fo4::Archive::read(archive_path.as_path())?;
        assert_eq!(options.version(), fo4::Version::v8);
        assert!(
            archive
                .get(&fo4::ArchiveKey::from("Meshes\\Generated\\a.nif"))
                .is_some()
        );
        assert!(
            archive
                .get(&fo4::ArchiveKey::from("Sound\\Generated\\b.wav"))
                .is_some()
        );
        Ok(())
    }

    #[test]
    fn fo4_texture_pack_streams_pc_dds_fixture() -> anyhow::Result<()> {
        let dir = TestDir::new();
        let source_dir = dir.path().join("src");
        let texture_dir = source_dir.join("Textures");
        fs::create_dir_all(&texture_dir)?;
        let file_name = "Fence006_1K_Roughness.dds";
        let source_file = texture_dir.join(file_name);
        fs::copy(Path::new("data/fo4_dds_test").join(file_name), &source_file)?;

        let archive_path = dir.path().join("out.ba2");
        let written = pack_archive(
            &source_dir,
            &archive_path,
            "fo4dds",
            true,
            9,
            false,
            None,
            None,
            PackFilters::default(),
        )
        .map_err(anyhow::Error::msg)?;
        assert_eq!(written, 1);

        let original = fs::read(&source_file)?;
        let (archive, options) = fo4::Archive::read(archive_path.as_path())?;
        assert_eq!(options.format(), fo4::Format::DX10);
        assert_eq!(options.compression_format(), fo4::CompressionFormat::Zip);
        let file = archive
            .get(&fo4::ArchiveKey::from(
                format!("Textures\\{file_name}").as_str(),
            ))
            .expect("missing packed texture");
        let write_options: fo4::FileWriteOptions = options.into();
        let mut extracted = Vec::new();
        file.write(&mut extracted, &write_options)?;
        assert_dx10_dds_payload_matches(&original, &extracted);

        Ok(())
    }

    #[test]
    fn fo4_texture_pack_streams_multiple_textures_with_jobs() -> anyhow::Result<()> {
        let dir = TestDir::new();
        let source_dir = dir.path().join("src");
        let texture_dir = source_dir.join("Textures");
        fs::create_dir_all(&texture_dir)?;
        let fixture = Path::new("data/fo4_dds_test").join("Fence006_1K_Roughness.dds");
        for index in 0..8 {
            fs::copy(
                &fixture,
                texture_dir.join(format!("texture_{index:02}.dds")),
            )?;
        }

        let archive_path = dir.path().join("out.ba2");
        let written = pack_archive(
            &source_dir,
            &archive_path,
            "fo4dds",
            true,
            9,
            false,
            None,
            Some(2),
            PackFilters::default(),
        )
        .map_err(anyhow::Error::msg)?;
        assert_eq!(written, 8);

        let (archive, options) = fo4::Archive::read(archive_path.as_path())?;
        assert_eq!(archive.len(), 8);
        let write_options: fo4::FileWriteOptions = options.into();
        let original = fs::read(&fixture)?;
        for index in 0..8 {
            let path = format!("Textures\\texture_{index:02}.dds");
            let file = archive
                .get(&fo4::ArchiveKey::from(path.as_str()))
                .with_context(|| format!("missing packed texture: {path}"))?;
            let mut extracted = Vec::new();
            file.write(&mut extracted, &write_options)?;
            assert_dx10_dds_payload_matches(&original, &extracted);
        }

        Ok(())
    }

    #[test]
    fn fo4_texture_pack_removes_temp_payloads_on_error() -> anyhow::Result<()> {
        let dir = TestDir::new();
        let source_dir = dir.path().join("src");
        let texture_dir = source_dir.join("Textures");
        fs::create_dir_all(&texture_dir)?;
        fs::write(texture_dir.join("bad.dds"), b"DDS invalid")?;

        let archive_path = dir.path().join("out.ba2");
        let err = pack_archive(
            &source_dir,
            &archive_path,
            "fo4dds",
            true,
            9,
            false,
            None,
            None,
            PackFilters::default(),
        )
        .expect_err("invalid DDS should fail");
        assert!(!err.is_empty());

        let leftovers: Vec<_> = fs::read_dir(dir.path())?
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(".out.ba2.payload")
            })
            .collect();
        assert!(leftovers.is_empty(), "leftover temp payload files found");

        Ok(())
    }

    #[test]
    fn pack_entries_rejects_unsafe_archive_paths() -> anyhow::Result<()> {
        let dir = TestDir::new();
        let source_path = dir.path().join("source.txt");
        fs::write(&source_path, b"source")?;

        let err = pack_archive_entries(
            &[PackEntrySpec {
                source_path,
                archive_path: "../bad.txt".to_string(),
            }],
            &dir.path().join("out.ba2"),
            "fo4",
            true,
            9,
            false,
            None,
            None,
        )
        .expect_err("unsafe archive path should fail");
        assert!(err.contains("unsafe archive path"));
        Ok(())
    }

    #[test]
    fn pack_entries_rejects_rooted_archive_paths() -> anyhow::Result<()> {
        let dir = TestDir::new();
        let source_path = dir.path().join("source.txt");
        fs::write(&source_path, b"source")?;

        for archive_path in [
            "/Meshes/a.nif",
            "\\Meshes\\a.nif",
            "\\\\server\\share\\a.nif",
        ] {
            let err = pack_archive_entries(
                &[PackEntrySpec {
                    source_path: source_path.clone(),
                    archive_path: archive_path.to_string(),
                }],
                &dir.path().join("out.ba2"),
                "fo4",
                true,
                9,
                false,
                None,
                None,
            )
            .expect_err("rooted archive path should fail");
            assert!(err.contains("unsafe archive path"));
        }

        assert_eq!(
            normalize_archive_entry_path("./Meshes/a.nif").map_err(anyhow::Error::msg)?,
            "Meshes/a.nif"
        );
        Ok(())
    }

    #[test]
    fn fo4_archive_type_names_select_expected_generations() {
        fn fo4_kind(value: &str) -> (fo4::Version, fo4::Format) {
            match parse_pack_kind(value).expect("archive type should parse") {
                PackKind::Fo4 {
                    version, format, ..
                } => (version, format),
                PackKind::Tes4 { .. } => panic!("expected FO4 pack kind"),
            }
        }

        assert_eq!(fo4_kind("fo4"), (fo4::Version::v8, fo4::Format::GNRL));
        assert_eq!(fo4_kind("fo4dds"), (fo4::Version::v8, fo4::Format::DX10));
        assert_eq!(fo4_kind("fo4xbox"), (fo4::Version::v8, fo4::Format::GNRL));
        assert_eq!(
            fo4_kind("fo4xboxdds"),
            (fo4::Version::v8, fo4::Format::DX10)
        );
        assert_eq!(fo4_kind("fo4og"), (fo4::Version::v1, fo4::Format::GNRL));
        assert_eq!(fo4_kind("fo4ogdds"), (fo4::Version::v1, fo4::Format::DX10));
    }

    #[test]
    fn pack_filters_include_only_matching_prefixes() {
        let filters = PackFilters::new(vec!["Textures\\".to_string()], Vec::new());

        assert!(filters.matches("textures/foo/bar.dds"));
        assert!(filters.matches("TEXTURES\\foo\\bar.dds"));
        assert!(!filters.matches("meshes/foo.nif"));
    }

    #[test]
    fn pack_filters_exclude_matching_prefixes() {
        let filters = PackFilters::new(Vec::new(), vec!["Textures/".to_string()]);

        assert!(!filters.matches("textures/foo/bar.dds"));
        assert!(filters.matches("meshes/foo.nif"));
    }

    #[test]
    fn pack_filters_preserve_manifest_order_for_remaining_files() -> anyhow::Result<()> {
        let dir = TestDir::new();
        let source_dir = dir.path().join("src");
        fs::create_dir_all(source_dir.join("Textures"))?;
        fs::create_dir_all(source_dir.join("Meshes"))?;
        fs::create_dir_all(source_dir.join("Misc"))?;
        fs::write(source_dir.join("Textures").join("a.dds"), b"dds")?;
        fs::write(source_dir.join("Meshes").join("b.nif"), b"nif")?;
        fs::write(source_dir.join("Misc").join("c.txt"), b"txt")?;
        let manifest_path = dir.path().join("manifest.json");
        fs::write(
            &manifest_path,
            r#"["Misc\\c.txt","Textures\\a.dds","Meshes\\b.nif"]"#,
        )?;

        let filters = PackFilters::new(Vec::new(), vec!["textures".to_string()]);
        let entries = collect_files(&source_dir, Some(&manifest_path), &filters)
            .map_err(anyhow::Error::msg)?;
        let paths: Vec<_> = entries
            .iter()
            .map(|entry| entry.rel_backslash.as_str())
            .collect();

        assert_eq!(paths, vec!["Misc\\c.txt", "Meshes\\b.nif"]);
        Ok(())
    }

    #[test]
    fn archive_type_default_level_picks_per_format() {
        assert_eq!(archive_type_default_level("fo4dds"), 4);
        assert_eq!(archive_type_default_level("fo4"), 6);
        assert_eq!(archive_type_default_level("fo76dds"), 4);
        assert_eq!(archive_type_default_level("starfielddds"), 4);
        assert_eq!(archive_type_default_level("sse"), 6);
    }
}
