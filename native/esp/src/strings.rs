// Native string-table I/O (Python shims live in py_creation_lib/python/creation_lib/esp/strings.py).
//
// Binary .STRINGS / .ILSTRINGS / .DLSTRINGS format (all little-endian):
//   Header : u32 count, u32 data_size
//   Dir    : count × (u32 string_id, u32 offset)
//   Data   : string_id 0x00: .STRINGS = null-terminated bytes
//                           .ILSTRINGS / .DLSTRINGS = u32 length prefix + bytes (null-terminated)
//
// Text decodes as UTF-8, falling back to windows-1252.

use bytes::Bytes;
use encoding_rs::WINDOWS_1252;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::LocalizedStringsState;

pub(crate) const STRING_TABLE_TYPES: [&str; 3] = ["strings", "ilstrings", "dlstrings"];

fn table_type_from_extension(ext_lower: &str) -> Option<&'static str> {
    match ext_lower {
        "strings" => Some("strings"),
        "ilstrings" => Some("ilstrings"),
        "dlstrings" => Some("dlstrings"),
        _ => None,
    }
}

/// Decode raw bytes: UTF-8 strict first, then windows-1252 (encoding_rs maps
/// every byte per the WHATWG spec, so it never errors).
///
/// CR/LF line endings are normalized to LF. Do not trim other trailing
/// whitespace: shipped string tables may use a single space as a meaningful
/// placeholder value, and collapsing that to empty changes runtime behavior.
fn decode_bytes(data: &[u8]) -> String {
    let decoded = match std::str::from_utf8(data) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (decoded, _, _had_errors) = WINDOWS_1252.decode(data);
            decoded.into_owned()
        }
    };
    decoded.replace("\r\n", "\n")
}

fn read_cstring(data: &[u8], start: usize) -> (String, usize) {
    let end = data[start..]
        .iter()
        .position(|&b| b == 0)
        .map(|idx| start + idx)
        .unwrap_or(data.len());
    let text = decode_bytes(&data[start..end]);
    (text, end + 1)
}

fn read_bstring(data: &[u8], start: usize) -> Result<(String, usize), String> {
    if start + 4 > data.len() {
        return Err("Length-prefixed string extends past end of table".to_string());
    }
    let raw_length_bytes: [u8; 4] = data[start..start + 4].try_into().expect("4-byte slice");
    let raw_length = u32::from_le_bytes(raw_length_bytes) as usize;
    let begin = start + 4;
    let end = begin
        .checked_add(raw_length)
        .ok_or_else(|| "Length-prefixed string extends past end of table".to_string())?;
    if end > data.len() {
        return Err("Length-prefixed string extends past end of table".to_string());
    }
    let mut slice: &[u8] = &data[begin..end];
    if slice.last() == Some(&0) {
        slice = &slice[..slice.len() - 1];
    }
    Ok((decode_bytes(slice), end))
}

/// Parse a .STRINGS / .ILSTRINGS / .DLSTRINGS blob into a map of id -> text.
/// `table_type` must be one of "strings", "ilstrings", "dlstrings". The label
/// is only used in error messages.
pub(crate) fn parse_string_table_blob(
    blob: &[u8],
    table_type: &str,
    label: &str,
) -> Result<HashMap<u32, String>, String> {
    if blob.len() < 8 {
        return Err(format!("Invalid string table header: {label}"));
    }
    let count = u32::from_le_bytes(blob[0..4].try_into().unwrap()) as usize;
    let data_size = u32::from_le_bytes(blob[4..8].try_into().unwrap()) as usize;
    let directory_size = count
        .checked_mul(8)
        .ok_or_else(|| format!("Invalid string table directory size: {label}"))?;
    let data_start = 8usize
        .checked_add(directory_size)
        .ok_or_else(|| format!("Invalid string table size: {label}"))?;
    let data_end = data_start
        .checked_add(data_size)
        .ok_or_else(|| format!("Invalid string table size: {label}"))?;
    if data_end > blob.len() {
        return Err(format!("Invalid string table size: {label}"));
    }
    let data_block = &blob[data_start..data_end];
    let length_prefixed = matches!(table_type, "ilstrings" | "dlstrings");

    let mut values: HashMap<u32, String> = HashMap::with_capacity(count);
    for index in 0..count {
        let entry_offset = 8 + index * 8;
        let string_id =
            u32::from_le_bytes(blob[entry_offset..entry_offset + 4].try_into().unwrap());
        let offset =
            u32::from_le_bytes(blob[entry_offset + 4..entry_offset + 8].try_into().unwrap())
                as usize;
        if offset > data_block.len() {
            return Err(format!("Invalid string table offset: {label}"));
        }
        let (text, _) = if length_prefixed {
            read_bstring(data_block, offset).map_err(|err| format!("{label}: {err}"))?
        } else {
            read_cstring(data_block, offset)
        };
        values.insert(string_id, text);
    }
    Ok(values)
}

/// Parse a .STRINGS / .ILSTRINGS / .DLSTRINGS file into a map of id -> text.
pub(crate) fn parse_string_table(path: &Path) -> Result<HashMap<u32, String>, String> {
    let blob = fs::read(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let table_type = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    parse_string_table_blob(&blob, table_type.as_str(), &path.display().to_string())
}

/// Encode a string table blob. Strings are encoded as UTF-8 (matches Python's
/// `str.encode("utf-8")`). Keys are emitted in ascending numeric order.
pub(crate) fn build_string_table_blob(
    values: &HashMap<u32, String>,
    table_type: &str,
) -> Result<Vec<u8>, String> {
    let length_prefixed = match table_type {
        "strings" => false,
        "ilstrings" | "dlstrings" => true,
        _ => return Err(format!("Unsupported string table type: {table_type:?}")),
    };
    let mut ordered: Vec<(&u32, &String)> = values.iter().collect();
    ordered.sort_by_key(|(key, _)| **key);

    let mut directory: Vec<u8> = Vec::with_capacity(ordered.len() * 8);
    let mut payload: Vec<u8> = Vec::new();
    for (string_id, text) in ordered {
        let encoded = text.as_bytes();
        let offset = payload.len() as u32;
        directory.extend_from_slice(&string_id.to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        if length_prefixed {
            payload.extend_from_slice(&((encoded.len() + 1) as u32).to_le_bytes());
        }
        payload.extend_from_slice(encoded);
        payload.push(0);
    }
    let mut out = Vec::with_capacity(8 + directory.len() + payload.len());
    out.extend_from_slice(&(values.len() as u32).to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&directory);
    out.extend_from_slice(&payload);
    Ok(out)
}

pub(crate) fn write_string_table(
    path: &Path,
    values: &HashMap<u32, String>,
    table_type: &str,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "failed to create strings directory '{}': {err}",
                    parent.display()
                )
            })?;
        }
    }
    let blob = build_string_table_blob(values, table_type)?;
    fs::write(path, blob).map_err(|err| format!("failed to write '{}': {err}", path.display()))?;
    Ok(())
}

/// Normalize a raw language token (as parsed from a filename or user input)
/// into a canonical language code. Mirrors `creation_lib.esp.strings._normalize_language`.
pub(crate) fn normalize_language(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let key: String = trimmed
        .chars()
        .map(|c| match c {
            '-' | ' ' => '_',
            other => other.to_ascii_lowercase(),
        })
        .collect();
    let mapped = match key.as_str() {
        "chinese" | "cn" => "cn",
        "chinesesimplified" | "chinese_simplified" | "zhhans" => "zhhans",
        "chinesetraditional" | "chinese_traditional" | "zhhant" => "zhhant",
        "german" | "de" => "de",
        "english" | "en" => "en",
        "spanish" | "es" => "es",
        "spanish_mexico" | "esmx" => "esmx",
        "french" | "fr" => "fr",
        "italian" | "it" => "it",
        "japanese" | "ja" => "ja",
        "korean" | "ko" => "ko",
        "polish" | "pl" => "pl",
        "portuguese_brazil" | "ptbr" => "ptbr",
        "russian" | "ru" => "ru",
        other => other,
    };
    Some(mapped.to_string())
}

pub(crate) fn language_code(raw: Option<&str>) -> String {
    match raw.and_then(normalize_language) {
        Some(code) => code,
        None => "en".to_string(),
    }
}

/// Find matching string-table files under `strings_dir` for `plugin_name`.
/// Returns a map keyed by normalized language code -> { table_type -> path }.
pub(crate) fn find_all_string_table_paths(
    plugin_name: &str,
    strings_dir: &Path,
) -> HashMap<String, HashMap<String, PathBuf>> {
    let mut by_language: HashMap<String, HashMap<String, PathBuf>> = HashMap::new();
    if !strings_dir.is_dir() {
        return by_language;
    }
    let stem = Path::new(plugin_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(plugin_name)
        .to_ascii_lowercase();
    let stem_prefix = format!("{stem}_");

    let iter = match fs::read_dir(strings_dir) {
        Ok(iter) => iter,
        Err(_) => return by_language,
    };
    for entry in iter.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name,
            None => continue,
        };
        let lower_name = file_name.to_ascii_lowercase();
        if !lower_name.starts_with(&stem_prefix) {
            continue;
        }
        let ext_lower = match path.extension().and_then(|e| e.to_str()) {
            Some(ext) => ext.to_ascii_lowercase(),
            None => continue,
        };
        let table_type = match table_type_from_extension(ext_lower.as_str()) {
            Some(t) => t,
            None => continue,
        };
        // Extract the language portion: between the stem prefix and extension.
        // lower_name already lowercase; the slice below preserves indices since
        // both strings are ASCII for matched stems and extensions.
        let without_ext = &lower_name[..lower_name.len() - (ext_lower.len() + 1)];
        let language_part = &without_ext[stem_prefix.len()..];
        let normalized = language_code(Some(language_part));
        by_language
            .entry(normalized)
            .or_default()
            .insert(table_type.to_string(), path);
    }
    by_language
}

/// Load all string tables under `strings_dir` for `plugin_name`, returning
/// (values_by_language, table_types_by_id).
pub(crate) fn load_all_string_tables(
    plugin_name: &str,
    strings_dir: &Path,
) -> (HashMap<String, HashMap<u32, String>>, HashMap<u32, String>) {
    let all_paths = find_all_string_table_paths(plugin_name, strings_dir);
    let mut values_by_language: HashMap<String, HashMap<u32, String>> = HashMap::new();
    let mut table_types: HashMap<u32, String> = HashMap::new();
    // Sort languages for deterministic iteration order (consistent default_language
    // selection when no English tables are present).
    let mut languages: Vec<&String> = all_paths.keys().collect();
    languages.sort();
    for language in languages {
        let tables = &all_paths[language];
        let mut language_values: HashMap<u32, String> = HashMap::new();
        for table_type in STRING_TABLE_TYPES.iter() {
            let Some(path) = tables.get(*table_type) else {
                continue;
            };
            let loaded = match parse_string_table(path) {
                Ok(values) => values,
                Err(_) => continue,
            };
            for (string_id, text) in loaded {
                language_values.insert(string_id, text);
                table_types
                    .entry(string_id)
                    .or_insert_with(|| (*table_type).to_string());
            }
        }
        if !language_values.is_empty() {
            values_by_language.insert(language.clone(), language_values);
        }
    }
    (values_by_language, table_types)
}

pub(crate) fn load_string_tables_for_language(
    plugin_name: &str,
    strings_dir: &Path,
    language: &str,
) -> (HashMap<String, HashMap<u32, String>>, HashMap<u32, String>) {
    let all_paths = find_all_string_table_paths(plugin_name, strings_dir);
    let Some(tables) = all_paths.get(language) else {
        return (HashMap::new(), HashMap::new());
    };
    let mut values_by_language: HashMap<String, HashMap<u32, String>> = HashMap::new();
    let mut table_types: HashMap<u32, String> = HashMap::new();
    let mut language_values: HashMap<u32, String> = HashMap::new();
    for table_type in STRING_TABLE_TYPES.iter() {
        let Some(path) = tables.get(*table_type) else {
            continue;
        };
        let loaded = match parse_string_table(path) {
            Ok(values) => values,
            Err(_) => continue,
        };
        for (string_id, text) in loaded {
            language_values.insert(string_id, text);
            table_types
                .entry(string_id)
                .or_insert_with(|| (*table_type).to_string());
        }
    }
    if !language_values.is_empty() {
        values_by_language.insert(language.to_string(), language_values);
    }
    (values_by_language, table_types)
}

/// Identify Bethesda string-table entries inside a BA2/BSA archive.
/// Returns Some((language, table_type)) for paths shaped like
/// `strings/<stem>_<lang>.<ext>`; otherwise None.
fn parse_archive_string_entry(
    archive_path: &str,
    plugin_stem: &str,
) -> Option<(String, &'static str)> {
    let normalized = archive_path.replace('\\', "/").to_ascii_lowercase();
    let stem_lower = plugin_stem.to_ascii_lowercase();

    let (without_ext, ext) = match normalized.rsplit_once('.') {
        Some((rest, ext)) => (rest, ext),
        None => return None,
    };
    let table_type = match ext {
        "strings" => "strings",
        "ilstrings" => "ilstrings",
        "dlstrings" => "dlstrings",
        _ => return None,
    };
    let file_part = without_ext.rsplit_once('/').map_or(without_ext, |(_, f)| f);
    if file_part == without_ext {
        // Path had no slashes and lived at archive root; still acceptable.
    } else {
        let dir_part = without_ext.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        // Must live inside a "strings" folder (case-insensitive); reject anything else.
        let last_dir = dir_part.rsplit('/').next().unwrap_or("");
        if last_dir != "strings" {
            return None;
        }
    }
    let prefix = format!("{stem_lower}_");
    let language_part = file_part.strip_prefix(&prefix)?;
    let language = normalize_language(language_part)?;
    Some((language, table_type))
}

/// One string table lifted out of an archive, still encoded.
struct ArchiveTableBlob {
    language: String,
    table_type: &'static str,
    label: String,
    blob: Vec<u8>,
}

/// How much of an archive's string tables to pull out.
#[derive(Clone, Copy)]
enum ArchiveTableSelection<'a> {
    /// Entry names only. Nothing is decompressed, so `blob` comes back empty.
    Names,
    /// Every table, decompressed.
    All,
    /// One table, decompressed, stopping as soon as it is found.
    One {
        language: &'a str,
        table_type: &'static str,
    },
}

impl ArchiveTableSelection<'_> {
    fn wants(&self, language: &str, table_type: &'static str) -> bool {
        match self {
            Self::Names | Self::All => true,
            Self::One {
                language: wanted_language,
                table_type: wanted_type,
            } => language == *wanted_language && table_type == *wanted_type,
        }
    }

    fn decompresses(&self) -> bool {
        !matches!(self, Self::Names)
    }

    fn stops_after_first(&self) -> bool {
        matches!(self, Self::One { .. })
    }
}

/// Walk the string tables in a sibling BA2/BSA, taking as much as `selection` asks for.
fn scan_archive_string_tables(
    archive_path: &Path,
    plugin_stem: &str,
    language_filter: Option<&str>,
    selection: ArchiveTableSelection<'_>,
) -> Vec<ArchiveTableBlob> {
    use bsarchive_native::{FileFormat, Reader, fo4, guess_format, tes4};

    let mut file = match fs::File::open(archive_path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let format = match guess_format(&mut file) {
        Some(f) => f,
        None => return Vec::new(),
    };
    let archive_label = archive_path.display().to_string();
    let mut out: Vec<ArchiveTableBlob> = Vec::new();

    match format {
        FileFormat::FO4 => {
            let (archive, options) = match fo4::Archive::read(archive_path) {
                Ok(pair) => pair,
                Err(_) => return Vec::new(),
            };
            let write_options: fo4::FileWriteOptions = (&options).into();
            for (key, file) in &archive {
                let raw_name = key.name().to_string();
                let Some((language, table_type)) =
                    parse_archive_string_entry(&raw_name, plugin_stem)
                else {
                    continue;
                };
                if language_filter.is_some_and(|target| language != target) {
                    continue;
                }
                if !selection.wants(language.as_str(), table_type) {
                    continue;
                }
                let mut blob = Vec::with_capacity(if selection.decompresses() {
                    file.iter()
                        .map(|chunk| chunk.decompressed_len().unwrap_or_else(|| chunk.len()))
                        .sum()
                } else {
                    0
                });
                if selection.decompresses() && file.write(&mut blob, &write_options).is_err() {
                    continue;
                }
                out.push(ArchiveTableBlob {
                    language,
                    table_type,
                    label: format!("{archive_label}::{raw_name}"),
                    blob,
                });
                if selection.stops_after_first() {
                    break;
                }
            }
        }
        FileFormat::TES4 => {
            let (archive, options) = match tes4::Archive::read(archive_path) {
                Ok(pair) => pair,
                Err(_) => return Vec::new(),
            };
            let write_options: tes4::FileCompressionOptions = (&options).into();
            'directories: for (dir_key, directory) in &archive {
                let dir_name = dir_key.name().to_string();
                for (file_key, file) in directory {
                    let file_name = file_key.name().to_string();
                    let rel = if dir_name.is_empty() {
                        file_name.clone()
                    } else {
                        format!("{dir_name}/{file_name}")
                    };
                    let Some((language, table_type)) =
                        parse_archive_string_entry(&rel, plugin_stem)
                    else {
                        continue;
                    };
                    if language_filter.is_some_and(|target| language != target) {
                        continue;
                    }
                    if !selection.wants(language.as_str(), table_type) {
                        continue;
                    }
                    let mut blob = Vec::with_capacity(if selection.decompresses() {
                        file.decompressed_len().unwrap_or_else(|| file.len())
                    } else {
                        0
                    });
                    if selection.decompresses() && file.write(&mut blob, &write_options).is_err() {
                        continue;
                    }
                    out.push(ArchiveTableBlob {
                        language,
                        table_type,
                        label: format!("{archive_label}::{rel}"),
                        blob,
                    });
                    if selection.stops_after_first() {
                        break 'directories;
                    }
                }
            }
        }
    }
    out
}

/// Read string tables from a single sibling BA2/BSA archive.
fn load_string_tables_from_archive(
    archive_path: &Path,
    plugin_stem: &str,
    language_filter: Option<&str>,
) -> Vec<(String, &'static str, HashMap<u32, String>)> {
    scan_archive_string_tables(
        archive_path,
        plugin_stem,
        language_filter,
        ArchiveTableSelection::All,
    )
    .into_iter()
    .filter_map(|entry| {
        parse_string_table_blob(&entry.blob, entry.table_type, &entry.label)
            .ok()
            .map(|values| (entry.language, entry.table_type, values))
    })
    .collect()
}

/// Find sibling BA2/BSA archives next to a plugin that may contain its strings.
/// Bethesda packages DLC strings in `<stem> - Main.ba2` (FO4 / Starfield) or
/// `<stem>.bsa` (Skyrim / older). `<stem> - Strings.ba2` is accepted for
/// backwards compatibility with earlier ModBox split-archive outputs. Order
/// matters: archives are tried in the returned order and the first match for
/// each (language, table_type) wins.
fn sibling_string_archives(plugin_path: &Path) -> Vec<PathBuf> {
    let parent = match plugin_path.parent() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let stem = match plugin_path.file_stem().and_then(|s| s.to_str()) {
        Some(s) => s,
        None => return Vec::new(),
    };
    let candidates = [
        format!("{stem} - Main.ba2"),
        format!("{stem} - Localization.ba2"),
        format!("{stem} - Strings.ba2"),
        format!("{stem}.ba2"),
        format!("{stem} - Interface.ba2"),
        format!("{stem}.bsa"),
        format!("{stem} - Interface.bsa"),
        "Skyrim - Interface.bsa".to_string(),
        format!("{stem} - Main.bsa"),
    ];
    let mut seen = std::collections::HashSet::new();
    candidates
        .iter()
        .map(|name| parent.join(name))
        .filter(|path| path.is_file() && seen.insert(path.clone()))
        .collect()
}

/// Merge BA2/BSA-extracted string tables into existing language maps without
/// overwriting values already populated from loose `Strings/` files.
fn merge_archive_tables_into(
    by_language: &mut HashMap<String, HashMap<u32, String>>,
    table_types: &mut HashMap<u32, String>,
    archive_tables: Vec<(String, &'static str, HashMap<u32, String>)>,
) {
    for (language, table_type, values) in archive_tables {
        let lang_map = by_language.entry(language).or_default();
        for (string_id, text) in values {
            lang_map.entry(string_id).or_insert(text);
            table_types
                .entry(string_id)
                .or_insert_with(|| table_type.to_string());
        }
    }
}

/// A string table that has been indexed but not decoded.
///
/// Holds the file bytes plus the `id -> offset` directory the format already
/// carries. Decoding every entry up front makes opening a localized master
/// expensive: 207 MB of loose tables across 13 languages expand roughly 5x as
/// owned `String`s in a `HashMap`.
pub(crate) struct LazyStringTable {
    language: String,
    table_type: &'static str,
    blob: Bytes,
    data_start: usize,
    data_end: usize,
    offsets: rustc_hash::FxHashMap<u32, u32>,
}

impl LazyStringTable {
    fn index(
        blob: Bytes,
        table_type: &'static str,
        language: String,
        label: &str,
    ) -> Result<Self, String> {
        if blob.len() < 8 {
            return Err(format!("Invalid string table header: {label}"));
        }
        let count = u32::from_le_bytes(blob[0..4].try_into().unwrap()) as usize;
        let data_size = u32::from_le_bytes(blob[4..8].try_into().unwrap()) as usize;
        let directory_size = count
            .checked_mul(8)
            .ok_or_else(|| format!("Invalid string table directory size: {label}"))?;
        let data_start = 8usize
            .checked_add(directory_size)
            .ok_or_else(|| format!("Invalid string table size: {label}"))?;
        let data_end = data_start
            .checked_add(data_size)
            .ok_or_else(|| format!("Invalid string table size: {label}"))?;
        if data_end > blob.len() {
            return Err(format!("Invalid string table size: {label}"));
        }
        let mut offsets = rustc_hash::FxHashMap::default();
        offsets.reserve(count);
        for index in 0..count {
            let entry = 8 + index * 8;
            let string_id = u32::from_le_bytes(blob[entry..entry + 4].try_into().unwrap());
            let offset = u32::from_le_bytes(blob[entry + 4..entry + 8].try_into().unwrap());
            offsets.insert(string_id, offset);
        }
        Ok(Self {
            language,
            table_type,
            blob,
            data_start,
            data_end,
            offsets,
        })
    }

    fn decode_at(&self, offset: u32) -> Option<String> {
        let data_block = self.blob.get(self.data_start..self.data_end)?;
        let offset = offset as usize;
        if offset > data_block.len() {
            return None;
        }
        if matches!(self.table_type, "ilstrings" | "dlstrings") {
            read_bstring(data_block, offset).ok().map(|(text, _)| text)
        } else {
            Some(read_cstring(data_block, offset).0)
        }
    }

    fn get(&self, string_id: u32) -> Option<String> {
        self.decode_at(*self.offsets.get(&string_id)?)
    }

    fn decode_all(&self) -> HashMap<u32, String> {
        let mut out = HashMap::with_capacity(self.offsets.len());
        for (&string_id, &offset) in &self.offsets {
            if let Some(text) = self.decode_at(offset) {
                out.insert(string_id, text);
            }
        }
        out
    }
}

/// Indexed loose tables plus the sibling archives that back them.
///
/// Archives backfill ids the loose extract lacks, and misses are routine (the
/// loose dir is often stale in a language or two), so deferring the whole
/// archive until the first miss saves little. Decompression is the cost:
/// SeventySix's tables are 215 MB packed, ~860 MB unpacked. So the archive
/// index is scanned for names only, and a table is unpacked when a lookup
/// needs that language and table type.
pub(crate) struct LazyStringTables {
    tables: Vec<LazyStringTable>,
    archive_paths: Vec<PathBuf>,
    plugin_stem: String,
    requested_language: Option<String>,
    archive_directory: std::sync::OnceLock<Vec<(String, &'static str)>>,
    archive_tables: Mutex<HashMap<(String, &'static str), Option<Arc<LazyStringTable>>>>,
    archive_table_types: Mutex<HashMap<u32, &'static str>>,
}

impl LazyStringTables {
    /// The `(language, table_type)` pairs the archives hold, names only.
    fn archive_directory(&self) -> &[(String, &'static str)] {
        self.archive_directory.get_or_init(|| {
            let mut seen: Vec<(String, &'static str)> = Vec::new();
            for archive in &self.archive_paths {
                for entry in scan_archive_string_tables(
                    archive,
                    self.plugin_stem.as_str(),
                    self.requested_language.as_deref(),
                    ArchiveTableSelection::Names,
                ) {
                    let key = (entry.language, entry.table_type);
                    if !seen.contains(&key) {
                        seen.push(key);
                    }
                }
            }
            seen
        })
    }

    /// Unpack and index one archive table, remembering the answer either way.
    fn archive_table(&self, language: &str, table_type: &'static str) -> Option<Arc<LazyStringTable>> {
        let key = (language.to_string(), table_type);
        if let Some(cached) = self.archive_tables.lock().unwrap().get(&key) {
            return cached.clone();
        }
        let mut found: Option<Arc<LazyStringTable>> = None;
        if self.archive_directory().contains(&key) {
            for archive in &self.archive_paths {
                let Some(entry) = scan_archive_string_tables(
                    archive,
                    self.plugin_stem.as_str(),
                    self.requested_language.as_deref(),
                    ArchiveTableSelection::One {
                        language,
                        table_type,
                    },
                )
                .pop() else {
                    continue;
                };
                if let Ok(indexed) = LazyStringTable::index(
                    Bytes::from(entry.blob),
                    table_type,
                    entry.language,
                    entry.label.as_str(),
                ) {
                    found = Some(Arc::new(indexed));
                    break;
                }
            }
        }
        self.archive_tables.lock().unwrap().insert(key, found.clone());
        found
    }

    /// Table types to search for `string_id`, narrowest first.
    ///
    /// An id lives in exactly one table type, so naming it turns a miss into
    /// one archive table per language instead of three. The loose tables
    /// usually name it; when they do not, the first language to resolve the id
    /// from an archive does, and the other twelve follow it.
    fn candidate_table_types(&self, string_id: u32) -> Vec<&'static str> {
        if let Some(table_type) = self.archive_table_types.lock().unwrap().get(&string_id) {
            return vec![table_type];
        }
        for table in &self.tables {
            if table.offsets.contains_key(&string_id) {
                return vec![table.table_type];
            }
        }
        vec!["strings", "ilstrings", "dlstrings"]
    }

    pub(crate) fn languages(&self) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for table in &self.tables {
            if !seen.iter().any(|existing| existing == &table.language) {
                seen.push(table.language.clone());
            }
        }
        for (language, _) in self.archive_directory() {
            if !seen.iter().any(|existing| existing == language) {
                seen.push(language.clone());
            }
        }
        seen.sort();
        seen
    }

    pub(crate) fn get(&self, language: &str, string_id: u32) -> Option<String> {
        for table in &self.tables {
            if table.language == language {
                if let Some(text) = table.get(string_id) {
                    return Some(text);
                }
            }
        }
        for table_type in self.candidate_table_types(string_id) {
            if let Some(text) = self
                .archive_table(language, table_type)
                .and_then(|table| table.get(string_id))
            {
                self.archive_table_types
                    .lock()
                    .unwrap()
                    .insert(string_id, table_type);
                return Some(text);
            }
        }
        None
    }

    /// Decode everything, archives included. For paths that enumerate or
    /// rewrite the whole corpus rather than resolving single ids.
    pub(crate) fn decode_all(
        &self,
    ) -> (HashMap<String, HashMap<u32, String>>, HashMap<u32, String>) {
        let mut by_language: HashMap<String, HashMap<u32, String>> = HashMap::new();
        let mut table_types: HashMap<u32, String> = HashMap::new();
        let mut absorb = |table: &LazyStringTable,
                          by_language: &mut HashMap<String, HashMap<u32, String>>,
                          table_types: &mut HashMap<u32, String>| {
            let target = by_language.entry(table.language.clone()).or_default();
            for (string_id, text) in table.decode_all() {
                table_types
                    .entry(string_id)
                    .or_insert_with(|| table.table_type.to_string());
                target.entry(string_id).or_insert(text);
            }
        };
        for table in &self.tables {
            absorb(table, &mut by_language, &mut table_types);
        }
        for (language, table_type) in self.archive_directory().to_vec() {
            if let Some(table) = self.archive_table(language.as_str(), table_type) {
                absorb(table.as_ref(), &mut by_language, &mut table_types);
            }
        }
        (by_language, table_types)
    }
}

/// Read a table file, mmapping the large ones so the bytes stay evictable.
fn read_table_bytes(path: &Path) -> Result<Bytes, String> {
    const TABLE_MMAP_THRESHOLD: u64 = 1024 * 1024;
    let metadata = fs::metadata(path).map_err(|err| format!("{}: {err}", path.display()))?;
    if metadata.len() >= TABLE_MMAP_THRESHOLD {
        let file = fs::File::open(path).map_err(|err| format!("{}: {err}", path.display()))?;
        // SAFETY: read-only mapping of a file we hold open. Concurrent external
        // writes would corrupt any reader regardless of mmap vs read.
        if let Ok(mmap) = unsafe { memmap2::Mmap::map(&file) } {
            return Ok(Bytes::from_owner(mmap));
        }
    }
    fs::read(path)
        .map(Bytes::from)
        .map_err(|err| format!("{}: {err}", path.display()))
}

/// `hydrate_strings_state` without decoding: indexes the loose tables and
/// remembers the sibling archives for on-demand backfill.
pub(crate) fn index_strings_state(
    plugin_path: &str,
    plugin_name: &str,
    strings_dir: Option<&str>,
    language: Option<&str>,
) -> LocalizedStringsState {
    let requested_language = language.and_then(normalize_language);
    let plugin_path_obj = Path::new(plugin_path);
    let derived_dir: Option<PathBuf> = match strings_dir {
        Some(dir) => Some(PathBuf::from(dir)),
        None => plugin_path_obj
            .parent()
            .map(|parent| parent.join("Strings"))
            .filter(|p| p.is_dir()),
    };
    let source_strings_dir = derived_dir
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());

    let mut tables: Vec<LazyStringTable> = Vec::new();
    if let Some(dir) = derived_dir.as_deref() {
        let all_paths = find_all_string_table_paths(plugin_name, dir);
        let mut languages: Vec<&String> = all_paths.keys().collect();
        languages.sort();
        for language_key in languages {
            if let Some(target) = requested_language.as_deref() {
                if language_key != target {
                    continue;
                }
            }
            let Some(by_type) = all_paths.get(language_key) else {
                continue;
            };
            let mut types: Vec<&String> = by_type.keys().collect();
            types.sort();
            for table_type in types {
                let Some(path) = by_type.get(table_type) else {
                    continue;
                };
                let Some(static_type) = table_type_from_extension(table_type.as_str()) else {
                    continue;
                };
                let Ok(blob) = read_table_bytes(path) else {
                    continue;
                };
                if let Ok(indexed) = LazyStringTable::index(
                    blob,
                    static_type,
                    language_key.clone(),
                    &path.display().to_string(),
                ) {
                    tables.push(indexed);
                }
            }
        }
    }

    let plugin_stem = Path::new(plugin_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(plugin_name)
        .to_string();

    let lazy = LazyStringTables {
        tables,
        archive_paths: sibling_string_archives(plugin_path_obj),
        plugin_stem,
        requested_language: requested_language.clone(),
        archive_directory: std::sync::OnceLock::new(),
        archive_tables: Mutex::new(HashMap::new()),
        archive_table_types: Mutex::new(HashMap::new()),
    };

    let is_filtered = requested_language.is_some();
    let default_language = if let Some(target) = requested_language.as_ref() {
        target.clone()
    } else {
        let languages = lazy.languages();
        if languages.iter().any(|code| code == "en") {
            "en".to_string()
        } else {
            languages.first().cloned().unwrap_or_default()
        }
    };

    let mut state = LocalizedStringsState::default();
    state.default_language = default_language;
    state.is_filtered = is_filtered;
    state.requested_language = requested_language;
    state.source_strings_dir = source_strings_dir;
    state.lazy_tables = Some(std::sync::Arc::new(lazy));
    state
}

/// Hydrate a `LocalizedStringsState` for a plugin without any Python callback.
///
/// Lookup order:
///   1. `<strings_dir>` (or `<plugin_dir>/Strings/`) — loose `<stem>_<lang>.STRINGS` files
///   2. Sibling archives — `<stem> - Main.ba2`, `<stem>.bsa`, etc.
///
/// DLCs and many third-party mods keep their string tables packed inside the
/// archive, so the BA2/BSA fallback is required for byte-faithful round-trips
/// of plugins that ship without loose strings.
pub(crate) fn hydrate_strings_state(
    plugin_path: &str,
    plugin_name: &str,
    strings_dir: Option<&str>,
    language: Option<&str>,
) -> LocalizedStringsState {
    let requested_language = language.and_then(normalize_language);
    let plugin_path_obj = Path::new(plugin_path);
    let derived_dir: Option<PathBuf> = match strings_dir {
        Some(dir) => Some(PathBuf::from(dir)),
        None => plugin_path_obj
            .parent()
            .map(|parent| parent.join("Strings"))
            .filter(|p| p.is_dir()),
    };
    let source_strings_dir = derived_dir
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());

    let (mut by_language, mut table_types) =
        match (derived_dir.as_deref(), requested_language.as_deref()) {
            (Some(dir), Some(target)) => load_string_tables_for_language(plugin_name, dir, target),
            (Some(dir), None) => load_all_string_tables(plugin_name, dir),
            (None, _) => (HashMap::new(), HashMap::new()),
        };

    // Always consult sibling archives — they may carry languages or tables
    // missing from the loose dir. Loose values take precedence (already
    // inserted; merge only fills gaps).
    let plugin_stem = Path::new(plugin_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(plugin_name);
    for archive in sibling_string_archives(plugin_path_obj) {
        let archive_tables =
            load_string_tables_from_archive(&archive, plugin_stem, requested_language.as_deref());
        if !archive_tables.is_empty() {
            merge_archive_tables_into(&mut by_language, &mut table_types, archive_tables);
        }
    }

    let is_filtered = requested_language.is_some();

    let default_language = if is_filtered {
        let target = requested_language
            .as_ref()
            .expect("filtered state has language");
        target.clone()
    } else if by_language.contains_key("en") {
        "en".to_string()
    } else {
        let mut keys: Vec<&String> = by_language.keys().collect();
        keys.sort();
        keys.first().map(|s| (*s).clone()).unwrap_or_default()
    };
    let mut state = LocalizedStringsState::default();
    state.by_language = by_language;
    state.table_types = table_types;
    state.default_language = default_language;
    state.is_filtered = is_filtered;
    state.requested_language = requested_language;
    state.source_strings_dir = source_strings_dir;
    state
}

#[cfg(test)]
mod tests {
    use super::{build_string_table_blob, parse_string_table_blob, sibling_string_archives};
    use std::collections::HashMap;
    use std::path::Path;

    #[test]
    fn parse_string_table_preserves_trailing_space_placeholders() {
        let values = HashMap::from([(0x110, " ".to_string())]);
        let blob = build_string_table_blob(&values, "dlstrings").unwrap();
        let parsed = parse_string_table_blob(&blob, "dlstrings", "test").unwrap();

        assert_eq!(parsed.get(&0x110).map(String::as_str), Some(" "));
    }

    #[test]
    fn sibling_string_archives_includes_fo76_localization_ba2() {
        // FO76 packs SeventySix.esm strings in `SeventySix - Localization.ba2`;
        // the loose `Data/Strings/` extract can be stale/partial, so the loader
        // must consult this archive to backfill missing string ids.
        let dir = std::env::temp_dir().join(format!("modbox_strings_loc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("SeventySix - Localization.ba2");
        std::fs::write(&archive, b"BTDX").unwrap();

        let found = sibling_string_archives(&dir.join("SeventySix.esm"));
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            found.iter().any(|p| p == &archive),
            "expected `SeventySix - Localization.ba2` among sibling archives, got {found:?}"
        );
    }

    #[test]
    fn sibling_string_archives_includes_skyrim_interface_bsa_deterministically() {
        let dir = std::env::temp_dir().join(format!(
            "modbox_strings_skyrim_interface_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let interface = dir.join("Skyrim - Interface.bsa");
        let main = dir.join("Skyrim - Main.bsa");
        let dawnguard_interface = dir.join("Dawnguard - Interface.bsa");
        std::fs::write(&interface, b"BSA\0").unwrap();
        std::fs::write(&main, b"BSA\0").unwrap();
        std::fs::write(&dawnguard_interface, b"BSA\0").unwrap();

        let first = sibling_string_archives(&dir.join("Skyrim.esm"));
        let second = sibling_string_archives(&dir.join("Skyrim.esm"));
        let dawnguard = sibling_string_archives(&dir.join("Dawnguard.esm"));
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(first, second);
        assert_eq!(first, vec![interface.clone(), main]);
        let unique = first.iter().collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), first.len());
        assert_eq!(dawnguard, vec![dawnguard_interface, interface]);
    }
}

pub(crate) fn rehydrate_all(
    state: &mut LocalizedStringsState,
    plugin_path: &str,
    plugin_name: &str,
    strings_dir: Option<&str>,
) {
    let remembered_strings_dir = state.source_strings_dir.clone();
    let effective_strings_dir = strings_dir.or(remembered_strings_dir.as_deref());
    let hydrate_path = if plugin_path.trim().is_empty() {
        plugin_name
    } else {
        plugin_path
    };
    *state = hydrate_strings_state(hydrate_path, plugin_name, effective_strings_dir, None);
    state.is_filtered = false;
    state.requested_language = None;
    if state.source_strings_dir.is_none() {
        state.source_strings_dir = remembered_strings_dir;
    }
}
