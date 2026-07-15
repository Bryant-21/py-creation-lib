/// Generate per-version classxml directories from SDK patches.
///
/// Port of `py_creation_lib/python/creation_lib/havok/gen_classxml.py`.
///
/// Key public API:
/// - `parse_patches(content: &str) -> Vec<(Option<String>, i32, Option<String>, i32)>`
/// - `generate_per_version_classxml(source_dir, patches_dir, targets, output_dir)`
use std::collections::HashMap;
use std::path::Path;

use crate::error::{HavokError, HavokResult};

// ---------------------------------------------------------------------------
// Version ID mapping (from hkHavokVersions.h)
// ---------------------------------------------------------------------------

/// Maps SDK patch directory names to their Havok version IDs.
pub fn dir_to_version_id(dir_name: &str) -> Option<i32> {
    match dir_name {
        "2010_1" => Some(39),
        "2010_2" => Some(40),
        "2011_1" => Some(41),
        "2011_2" => Some(42),
        "2011_3" => Some(43),
        "2012_1" => Some(45),
        "2012_2" => Some(46),
        "2013_1" => Some(48),
        "2013_2" => Some(49),
        "2013_3" => Some(50),
        "2014_1" => Some(53),
        "2014_2" => Some(55),
        "2014_2_5" => Some(56),
        "2015_1" => Some(57),
        "2016_1" => Some(58),
        "2016_2" => Some(58),
        "2017_1" => Some(59),
        "2017_2" => Some(60),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Patch parsing — no regex; hand-written scanner for `HK_PATCH_BEGIN(...)`
// ---------------------------------------------------------------------------

/// Parse `HK_PATCH_BEGIN(...)` entries from `.hxx` file *content* (not path).
///
/// Returns tuples of `(old_name, old_ver, new_name, new_ver)` where:
/// - `None` means `HK_NULL`
/// - `-1` means `HK_CLASS_ADDED`
/// - `-2` means `HK_CLASS_REMOVED`
pub fn parse_patches(content: &str) -> Vec<(Option<String>, i32, Option<String>, i32)> {
    const TOKEN: &str = "HK_PATCH_BEGIN(";
    let mut results = Vec::new();
    let mut search_from = 0;

    while let Some(rel) = content[search_from..].find(TOKEN) {
        let start = search_from + rel + TOKEN.len();
        search_from = start;

        // Find matching closing paren — scan for ')' at the top level (no nesting)
        let Some(end) = content[start..].find(')') else {
            break;
        };
        let args_str = &content[start..start + end];

        // Parse the four comma-separated args
        let Some((old_name, old_ver, new_name, new_ver)) = parse_patch_args(args_str) else {
            continue;
        };
        results.push((old_name, old_ver, new_name, new_ver));
    }

    results
}

/// Parse the four args inside `HK_PATCH_BEGIN(...)`.
fn parse_patch_args(args: &str) -> Option<(Option<String>, i32, Option<String>, i32)> {
    // Split on commas, respecting quoted strings
    let parts = split_args(args);
    if parts.len() != 4 {
        return None;
    }

    let old_name = parse_name_arg(parts[0].trim());
    let old_ver = parse_ver_arg(parts[1].trim())?;
    let new_name = parse_name_arg(parts[2].trim());
    let new_ver = parse_ver_arg(parts[3].trim())?;

    Some((old_name, old_ver, new_name, new_ver))
}

/// Split a comma-separated args string, respecting double-quoted strings.
fn split_args(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quote = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            ',' if !in_quote => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Parse a name argument: `"ClassName"` → `Some("ClassName")`, `HK_NULL` → `None`.
fn parse_name_arg(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed == "HK_NULL" {
        return None;
    }
    // Strip surrounding quotes
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        return Some(trimmed[1..trimmed.len() - 1].to_string());
    }
    None
}

/// Parse a version argument: integer or `HK_CLASS_ADDED` (-1) / `HK_CLASS_REMOVED` (-2).
fn parse_ver_arg(s: &str) -> Option<i32> {
    let trimmed = s.trim();
    if trimmed == "HK_CLASS_ADDED" {
        return Some(-1);
    }
    if trimmed == "HK_CLASS_REMOVED" {
        return Some(-2);
    }
    trimmed.parse::<i32>().ok()
}

// ---------------------------------------------------------------------------
// Version map builder
// ---------------------------------------------------------------------------

/// Build a map of `version_id -> Vec<patches>` from a SDK patches directory.
///
/// `patches_dir` is the `refs/hk2018_1_0_r1/Source/Common/Compat/Patches/` directory.
pub fn build_version_map(
    patches_dir: &Path,
) -> HavokResult<HashMap<i32, Vec<(Option<String>, i32, Option<String>, i32)>>> {
    let mut version_map: HashMap<i32, Vec<_>> = HashMap::new();

    let read_dir = std::fs::read_dir(patches_dir).map_err(|e| HavokError::Io {
        path: patches_dir.to_string_lossy().into_owned(),
        operation: "read_dir",
        source: e,
    })?;

    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some(version_id) = dir_to_version_id(&dir_name) else {
            continue;
        };

        let hxx_dir = std::fs::read_dir(&path).map_err(|e| HavokError::Io {
            path: path.to_string_lossy().into_owned(),
            operation: "read_dir",
            source: e,
        })?;

        for hxx_entry in hxx_dir.flatten() {
            let hxx_path = hxx_entry.path();
            if hxx_path.extension().and_then(|e| e.to_str()) != Some("hxx") {
                continue;
            }
            let content = std::fs::read_to_string(&hxx_path).unwrap_or_default();
            let patches = parse_patches(&content);
            version_map.entry(version_id).or_default().extend(patches);
        }
    }

    Ok(version_map)
}

// ---------------------------------------------------------------------------
// Class-version computation
// ---------------------------------------------------------------------------

/// Compute class versions at `target_version_id` from a base set.
///
/// - Upgrade (target > base): apply patches forward.
/// - Downgrade (target < base): reverse patches backward.
pub fn compute_class_versions(
    base_versions: &HashMap<String, i32>,
    version_map: &HashMap<i32, Vec<(Option<String>, i32, Option<String>, i32)>>,
    base_version_id: i32,
    target_version_id: i32,
) -> HashMap<String, i32> {
    let mut result = base_versions.clone();

    if target_version_id == base_version_id {
        return result;
    }

    let mut all_ids: Vec<i32> = version_map.keys().copied().collect();
    all_ids.sort_unstable();

    if target_version_id > base_version_id {
        for vid in &all_ids {
            if *vid <= base_version_id {
                continue;
            }
            if *vid > target_version_id {
                break;
            }
            for (old_name, old_ver, new_name, new_ver) in &version_map[vid] {
                apply_patch_forward(&mut result, old_name, *old_ver, new_name, *new_ver);
            }
        }
    } else {
        for vid in all_ids.iter().rev() {
            if *vid > base_version_id {
                continue;
            }
            if *vid <= target_version_id {
                break;
            }
            for (old_name, old_ver, new_name, new_ver) in &version_map[vid] {
                apply_patch_backward(&mut result, old_name, *old_ver, new_name, *new_ver);
            }
        }
    }

    result
}

fn apply_patch_forward(
    result: &mut HashMap<String, i32>,
    old_name: &Option<String>,
    old_ver: i32,
    new_name: &Option<String>,
    new_ver: i32,
) {
    match (old_name, new_name) {
        (None, Some(nn)) => {
            // CLASS_ADDED
            result.insert(nn.clone(), new_ver);
        }
        (Some(_), None) => {
            // CLASS_REMOVED — keep in map
        }
        (Some(on), Some(nn)) => {
            if result.get(on.as_str()) == Some(&old_ver) {
                if on != nn {
                    result.remove(on.as_str());
                }
                result.insert(nn.clone(), new_ver);
            }
        }
        _ => {}
    }
}

fn apply_patch_backward(
    result: &mut HashMap<String, i32>,
    old_name: &Option<String>,
    old_ver: i32,
    new_name: &Option<String>,
    new_ver: i32,
) {
    match (old_name, new_name) {
        (None, Some(_)) => {
            // CLASS_ADDED was applied → reverse = remove (but keep for descriptor compat)
        }
        (Some(on), None) => {
            // CLASS_REMOVED was applied → reverse = restore
            result.insert(on.clone(), old_ver);
        }
        (Some(on), Some(nn)) => {
            if result.get(nn.as_str()) == Some(&new_ver) {
                if on != nn {
                    result.remove(nn.as_str());
                }
                result.insert(on.clone(), old_ver);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// ClassXML generation
// ---------------------------------------------------------------------------

/// Generate a classxml directory for a target version.
///
/// Reads `_index.json` from `source_dir`, copies XML files to `target_dir`
/// adjusting version numbers where they differ.
///
/// `source_versions` and `target_versions` are class-name → version maps.
pub fn generate_classxml(
    target_dir: &Path,
    target_versions: &HashMap<String, i32>,
    source_dir: &Path,
    source_versions: &HashMap<String, i32>,
) -> HavokResult<()> {
    std::fs::create_dir_all(target_dir).map_err(|e| HavokError::Io {
        path: target_dir.to_string_lossy().into_owned(),
        operation: "create_dir_all",
        source: e,
    })?;

    // Read source index
    let index_path = source_dir.join("_index.json");
    let index_text = std::fs::read_to_string(&index_path).map_err(|e| HavokError::Io {
        path: index_path.to_string_lossy().into_owned(),
        operation: "read_to_string",
        source: e,
    })?;

    // Parse as a simple JSON object (class_name -> filename)
    let source_index: HashMap<String, String> = serde_json::from_str(&index_text)
        .map_err(|e| HavokError::InvalidInput(format!("_index.json parse error: {e}")))?;

    let mut new_index: HashMap<String, String> = HashMap::new();

    for (class_name, src_filename) in &source_index {
        let src_path = source_dir.join(src_filename);
        if !src_path.exists() {
            continue;
        }

        let source_ver = *source_versions.get(class_name.as_str()).unwrap_or(&0);
        let target_ver = *target_versions
            .get(class_name.as_str())
            .unwrap_or(&source_ver);

        let new_filename = format!("{}_{}.xml", class_name, target_ver);
        let dst_path = target_dir.join(&new_filename);
        new_index.insert(class_name.clone(), new_filename);

        if target_ver != source_ver {
            // Text-level version replacement
            let content = std::fs::read_to_string(&src_path).map_err(|e| HavokError::Io {
                path: src_path.to_string_lossy().into_owned(),
                operation: "read_to_string",
                source: e,
            })?;
            // Replace first occurrence of version='N'
            let new_content = replace_version_attr(&content, target_ver);
            std::fs::write(&dst_path, new_content).map_err(|e| HavokError::Io {
                path: dst_path.to_string_lossy().into_owned(),
                operation: "write",
                source: e,
            })?;
        } else {
            std::fs::copy(&src_path, &dst_path).map_err(|e| HavokError::Io {
                path: src_path.to_string_lossy().into_owned(),
                operation: "copy",
                source: e,
            })?;
        }
    }

    // Write new index
    let new_index_path = target_dir.join("_index.json");
    let index_json = serde_json::to_string_pretty(&new_index)
        .map_err(|e| HavokError::InvalidInput(format!("index serialization error: {e}")))?
        + "\n";
    std::fs::write(&new_index_path, index_json).map_err(|e| HavokError::Io {
        path: new_index_path.to_string_lossy().into_owned(),
        operation: "write",
        source: e,
    })?;

    Ok(())
}

/// Replace the first `version='N'` attribute value in an XML string.
fn replace_version_attr(content: &str, new_ver: i32) -> String {
    const NEEDLE: &str = "version='";
    if let Some(pos) = content.find(NEEDLE) {
        let after = &content[pos + NEEDLE.len()..];
        if let Some(end_quote) = after.find('\'') {
            let before = &content[..pos + NEEDLE.len()];
            let rest = &after[end_quote..]; // includes the closing quote
            return format!("{}{}{}", before, new_ver, rest);
        }
    }
    content.to_string()
}

// ---------------------------------------------------------------------------
// High-level entry point
// ---------------------------------------------------------------------------

/// Convenience struct for `generate_per_version_classxml`.
pub struct ClassXmlTarget {
    /// Output directory suffix (e.g. "2012").
    pub suffix: String,
    /// Target Havok version ID.
    pub version_id: i32,
}

/// Generate per-version classxml directories.
///
/// - `source_dir` — base classxml directory (e.g. `resource/classxml`).
/// - `patches_dir` — SDK patches directory (e.g. `refs/hk2018_1_0_r1/.../Patches`).
/// - `targets` — list of `(output_dir_suffix, version_id)`.
/// - `output_base` — parent directory for output; each target goes under
///   `<output_base>/classxml_<suffix>/`.
/// - `base_version_id` — version ID of the source classxml (53 for FO4).
pub fn generate_per_version_classxml(
    source_dir: &Path,
    patches_dir: &Path,
    targets: &[ClassXmlTarget],
    output_base: &Path,
    base_version_id: i32,
) -> HavokResult<()> {
    let version_map = build_version_map(patches_dir)?;

    // Read base class versions from _index.json
    let index_path = source_dir.join("_index.json");
    let index_text = std::fs::read_to_string(&index_path).map_err(|e| HavokError::Io {
        path: index_path.to_string_lossy().into_owned(),
        operation: "read_to_string",
        source: e,
    })?;
    let source_index: HashMap<String, String> = serde_json::from_str(&index_text)
        .map_err(|e| HavokError::InvalidInput(format!("_index.json parse error: {e}")))?;

    // Extract version numbers from filenames (ClassName_Version.xml)
    let mut fo4_versions: HashMap<String, i32> = HashMap::new();
    for (class_name, filename) in &source_index {
        let stem = Path::new(filename)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ver = stem
            .rsplit('_')
            .next()
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0);
        fo4_versions.insert(class_name.clone(), ver);
    }

    for target in targets {
        let target_versions = compute_class_versions(
            &fo4_versions,
            &version_map,
            base_version_id,
            target.version_id,
        );
        let target_dir = output_base.join(format!("classxml_{}", target.suffix));
        generate_classxml(&target_dir, &target_versions, source_dir, &fo4_versions)?;
    }

    Ok(())
}
