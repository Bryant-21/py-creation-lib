// Native string-table I/O — pure Rust replacement for py_creation_lib/python/creation_lib/esp/strings.py.
//
// Binary .STRINGS / .ILSTRINGS / .DLSTRINGS format (all little-endian):
//   Header : u32 count, u32 data_size
//   Dir    : count × (u32 string_id, u32 offset)
//   Data   : string_id 0x00: .STRINGS = null-terminated bytes
//                           .ILSTRINGS / .DLSTRINGS = u32 length prefix + bytes (null-terminated)
//
// Text decoding matches the Python fallback chain (UTF-8, then windows-1252) so
// localized values survive byte-exact roundtrip through Python APIs.

use encoding_rs::WINDOWS_1252;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

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

/// Decode raw bytes following Python's `_decode_bytes` preference:
/// UTF-8 strict first, then windows-1252 (which always maps all bytes via the
/// WHATWG encoding spec). Python's additional cp1251 fallback is not needed
/// because encoding_rs' WINDOWS_1252 never errors.
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

/// Read string tables from a single sibling BA2/BSA archive.
fn load_string_tables_from_archive(
    archive_path: &Path,
    plugin_stem: &str,
    language_filter: Option<&str>,
) -> Vec<(String, &'static str, HashMap<u32, String>)> {
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
    let mut out: Vec<(String, &'static str, HashMap<u32, String>)> = Vec::new();

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
                let mut blob = Vec::new();
                if file.write(&mut blob, &write_options).is_err() {
                    continue;
                }
                let label = format!("{archive_label}::{raw_name}");
                if let Ok(values) = parse_string_table_blob(&blob, table_type, &label) {
                    out.push((language, table_type, values));
                }
            }
        }
        FileFormat::TES4 => {
            let (archive, options) = match tes4::Archive::read(archive_path) {
                Ok(pair) => pair,
                Err(_) => return Vec::new(),
            };
            let write_options: tes4::FileCompressionOptions = (&options).into();
            for (dir_key, directory) in &archive {
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
                    let mut blob = Vec::new();
                    if file.write(&mut blob, &write_options).is_err() {
                        continue;
                    }
                    let label = format!("{archive_label}::{rel}");
                    if let Ok(values) = parse_string_table_blob(&blob, table_type, &label) {
                        out.push((language, table_type, values));
                    }
                }
            }
        }
    }
    out
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
        format!("{stem} - Main.bsa"),
    ];
    candidates
        .iter()
        .map(|name| parent.join(name))
        .filter(|p| p.is_file())
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
