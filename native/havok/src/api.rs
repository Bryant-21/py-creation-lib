use crate::animation::clip::AnimationClip;
use crate::animation::clip::extract_clip;
use crate::animation::parsers::{
    parse_behavior_graph_to_ui_json, parse_behavior_xml, parse_skeleton_xml,
};
use crate::animation::writer::write_interleaved_animation_xml;
use crate::collision::preview::collision_preview_json;
use crate::convert;
use crate::error::{HavokError, HavokResult};
use crate::hkx;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

const HKX_MAGIC: &[u8; 8] = b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10";
const TAG0_MAGIC: &[u8; 4] = b"TAG0";
// Skyrim SE binary tagfile (.hkt) magic: 0xCAB00D1E 0xD011FACE little-endian.
const BINARY_TAG_MAGIC_0: u32 = 0xCAB0_0D1E;
const BINARY_TAG_MAGIC_1_FACE: u32 = 0xD011_FACE;
const BINARY_TAG_MAGIC_1_CODE: u32 = 0xDEAD_C0DE;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectFormatResult {
    /// `"packfile"`, `"tagfile"`, or `"binary_tagfile"`.
    pub kind: String,
    /// Version string: packfile version_name (e.g. `hk_2014.1.0-r1`),
    /// tagfile SDK version (e.g. `20150100`), or `v<n>` for binary tagfile.
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HavokConversionReport {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HkxClassSummary {
    pub contents_version: String,
    pub class_counts: BTreeMap<String, usize>,
    pub has_cloth_data: bool,
    pub has_cloth_setup_data: bool,
    pub is_setup_only_cloth: bool,
}

/// Backwards-compatible "kind only" detector. Returns `"packfile"` /
/// `"tagfile"` (binary_tagfile is reported as `"binary_tagfile"`).
pub fn hkx_detect_format(data: &[u8]) -> HavokResult<&'static str> {
    let kind = match hkx_detect_format_full(data)?.kind.as_str() {
        "packfile" => "packfile",
        "tagfile" => "tagfile",
        "binary_tagfile" => "binary_tagfile",
        _ => "unknown",
    };
    Ok(kind)
}

/// Structured detector: returns the format kind plus a version string.
/// Mirrors `py_creation_lib/python/creation_lib/hkxpack/__init__.py::detect_format`.
pub fn hkx_detect_format_full(data: &[u8]) -> HavokResult<DetectFormatResult> {
    if data.len() < 4 {
        return Err(HavokError::InvalidInput(
            "Havok data must contain at least 4 bytes".to_string(),
        ));
    }

    // Packfile (v8/v11): version_name lives at [0x28, NUL).
    if data.len() >= 8 && &data[0..8] == HKX_MAGIC {
        let mut version = "unknown".to_string();
        if data.len() > 0x28 {
            let end = (0x28..data.len().min(0x40))
                .find(|&i| data[i] == 0)
                .unwrap_or(0x40.min(data.len()));
            version = String::from_utf8_lossy(&data[0x28..end]).into_owned();
        }
        return Ok(DetectFormatResult {
            kind: "packfile".to_string(),
            version,
        });
    }

    // TAG0 tagfile: TAG0 marker in first 8 bytes; SDKV section follows.
    if data.len() >= 8 && &data[4..8] == TAG0_MAGIC {
        let mut version = "unknown".to_string();
        if let Some(p) = (8..data.len().min(64)).find(|&i| data.get(i..i + 4) == Some(b"SDKV")) {
            let start = p + 4;
            let end = (start..data.len().min(start + 8))
                .find(|&i| data[i] == 0)
                .unwrap_or((start + 8).min(data.len()));
            version = String::from_utf8_lossy(&data[start..end])
                .trim_matches(char::from(0))
                .to_string();
        }
        return Ok(DetectFormatResult {
            kind: "tagfile".to_string(),
            version,
        });
    }

    // Binary tagfile (Skyrim SE .hkt): magic 0xCAB00D1E 0xD011FACE.
    if data.len() >= 16 {
        let m0 = u32::from_le_bytes(data[0..4].try_into().unwrap());
        let m1 = u32::from_le_bytes(data[4..8].try_into().unwrap());
        if m0 == BINARY_TAG_MAGIC_0
            && (m1 == BINARY_TAG_MAGIC_1_FACE || m1 == BINARY_TAG_MAGIC_1_CODE)
        {
            let ver = u32::from_le_bytes(data[12..16].try_into().unwrap());
            return Ok(DetectFormatResult {
                kind: "binary_tagfile".to_string(),
                version: format!("v{}", ver),
            });
        }
        // Byte-swapped variant.
        let m0s = u32::from_be_bytes(data[0..4].try_into().unwrap());
        let m1s = u32::from_be_bytes(data[4..8].try_into().unwrap());
        if m0s == BINARY_TAG_MAGIC_0
            && (m1s == BINARY_TAG_MAGIC_1_FACE || m1s == BINARY_TAG_MAGIC_1_CODE)
        {
            let ver = u32::from_be_bytes(data[12..16].try_into().unwrap());
            return Ok(DetectFormatResult {
                kind: "binary_tagfile".to_string(),
                version: format!("v{}", ver),
            });
        }
    }

    Err(HavokError::UnsupportedFormat(format!(
        "magic {:02X?}",
        &data[0..4]
    )))
}

pub fn hkx_class_summary(data: &[u8]) -> HavokResult<HkxClassSummary> {
    let format = hkx_detect_format(data)?;
    let hkx = match format {
        "packfile" => hkx::read_packfile(data)?,
        "tagfile" => hkx::parse_tagfile(data)?.materialize_hkx()?,
        other => return Err(HavokError::UnsupportedFormat(other.to_string())),
    };

    let mut class_counts = BTreeMap::new();
    for object in hkx.objects() {
        *class_counts.entry(object.class_name.clone()).or_insert(0) += 1;
    }
    let has_cloth_data = class_counts.contains_key("hclClothData");
    let has_cloth_setup_data = class_counts.keys().any(|class_name| {
        class_name.starts_with("hcl")
            && (class_name.contains("ClothSetup")
                || class_name.contains("SetupMesh")
                || class_name.contains("SetupObject"))
    });
    let is_setup_only_cloth = has_cloth_setup_data && !has_cloth_data;

    Ok(HkxClassSummary {
        contents_version: hkx.contents_version().to_string(),
        class_counts,
        has_cloth_data,
        has_cloth_setup_data,
        is_setup_only_cloth,
    })
}

pub fn hkx_roundtrip_bytes(data: &[u8]) -> HavokResult<Vec<u8>> {
    match hkx_detect_format(data)? {
        "packfile" => Ok(hkx::read_packfile(data)?.save_unchanged()),
        "tagfile" => Ok(hkx::parse_tagfile(data)?.source_bytes_clone()),
        // Binary tagfile (Tagfile2014 v13) unchanged roundtrip preserves
        // source bytes. Explicit v13 serialization lives at
        // hkx::tagfile2014::write_tagfile2014.
        "binary_tagfile" => {
            hkx::tagfile2014::read_tagfile2014(data)?;
            Ok(data.to_vec())
        }
        other => Err(HavokError::UnsupportedFormat(other.to_string())),
    }
}

pub fn havok_convert_bytes(data: &[u8], target_version: &str) -> HavokResult<Vec<u8>> {
    havok_convert_bytes_report(data, target_version).map(|report| report.bytes)
}

pub fn havok_convert_bytes_report(
    data: &[u8],
    target_version: &str,
) -> HavokResult<HavokConversionReport> {
    let target = convert::parse_target_version(target_version)?;
    let format = hkx_detect_format(data)?;
    let source_version = match format {
        "packfile" => {
            let hkx = hkx::read_packfile(data)?;
            let source_version = convert::detect_version_id(hkx.contents_version())?;
            if source_version == 56 && target.id == 53 {
                let conversion = convert::fo76::migrate_2015_packfile_to_2014_with_warnings(
                    hkx,
                    convert::fo76::Fo76MigrationOptions::default(),
                )?;
                let mut warnings = conversion.warnings;
                warnings.extend(collect_target_classxml_warnings(
                    &conversion.hkx,
                    target.name,
                ));
                let mut registry =
                    hkx::descriptors::DescriptorRegistry::for_contents_version(target.name);
                let bytes = hkx::write_hkx(&conversion.hkx, &mut registry);
                return Ok(HavokConversionReport { bytes, warnings });
            }
            source_version
        }
        "tagfile" => {
            let tagfile = hkx::parse_tagfile(data)?;
            let source_version = convert::detect_version_id(&tagfile.contents_version)?;
            if source_version == 56 && target.id == 53 {
                let conversion = convert::fo76::migrate_2015_tag0_to_2014_with_warnings(
                    &tagfile,
                    convert::fo76::Fo76MigrationOptions::default(),
                )?;
                let mut warnings = conversion.warnings;
                warnings.extend(collect_target_classxml_warnings(
                    &conversion.hkx,
                    target.name,
                ));
                let mut registry =
                    hkx::descriptors::DescriptorRegistry::for_contents_version(target.name);
                let bytes = hkx::write_hkx(&conversion.hkx, &mut registry);
                return Ok(HavokConversionReport { bytes, warnings });
            }
            source_version
        }
        other => return Err(HavokError::UnsupportedFormat(other.to_string())),
    };

    if source_version == target.id {
        return Ok(HavokConversionReport {
            bytes: hkx_roundtrip_bytes(data)?,
            warnings: Vec::new(),
        });
    }

    // Patch-chain route: parse the input, walk the patch corpus from source to
    // target, and re-serialize. The native corpus is the source of truth — if
    // the manager refuses (corpus_complete=false), the chain is incomplete and
    // forcing a run would silently drop members or run zero-defaulted re-adds.
    if format == "packfile" {
        let mut hkx = hkx::read_packfile(data)?;
        let manager = convert::PatchManager::with_native_corpus();
        manager.convert_hkx(&mut hkx, source_version, target.id)?;
        let warnings = collect_target_classxml_warnings(&hkx, target.name);
        let mut registry = hkx::descriptors::DescriptorRegistry::for_contents_version(target.name);
        return Ok(HavokConversionReport {
            bytes: hkx::write_hkx(&hkx, &mut registry),
            warnings,
        });
    }

    Err(convert::conversion_not_implemented(
        source_version,
        target.id,
        format,
    ))
}

fn collect_target_classxml_warnings(hkx_file: &hkx::HkxFile, target_version: &str) -> Vec<String> {
    let mut registry = hkx::descriptors::DescriptorRegistry::for_contents_version(target_version);
    let mut warned_classes = HashSet::new();
    let mut warnings = Vec::new();

    for object in hkx_file.objects() {
        if !warned_classes.insert(object.class_name.clone()) {
            continue;
        }
        match registry.get(&object.class_name) {
            Ok(Some(_)) => {}
            Ok(None) => warnings.push(format!(
                "target {target_version} classxml has no descriptor for {} (example object {}); output may serialize this class without members",
                object.class_name,
                object.name.as_deref().unwrap_or("<unnamed>")
            )),
            Err(error) => warnings.push(format!(
                "target {target_version} classxml descriptor lookup failed for {}: {error}",
                object.class_name
            )),
        }
    }

    warnings
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HavokBatchError {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HavokBatchResult {
    pub converted: usize,
    pub skipped: usize,
    pub errors: Vec<HavokBatchError>,
}

pub fn havok_convert_file(
    src_path: impl AsRef<Path>,
    dst_path: impl AsRef<Path>,
    target_version: &str,
) -> HavokResult<()> {
    havok_convert_file_report(src_path, dst_path, target_version).map(|_| ())
}

pub fn havok_convert_file_report(
    src_path: impl AsRef<Path>,
    dst_path: impl AsRef<Path>,
    target_version: &str,
) -> HavokResult<Vec<String>> {
    let src_path = src_path.as_ref();
    let dst_path = dst_path.as_ref();
    let data = std::fs::read(src_path).map_err(|source| HavokError::Io {
        path: src_path.display().to_string(),
        operation: "read",
        source,
    })?;
    let report = havok_convert_bytes_report(&data, target_version)?;
    if let Some(parent) = dst_path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| HavokError::Io {
            path: parent.display().to_string(),
            operation: "create_dir_all",
            source,
        })?;
    }
    std::fs::write(dst_path, report.bytes).map_err(|source| HavokError::Io {
        path: dst_path.display().to_string(),
        operation: "write",
        source,
    })?;
    Ok(report.warnings)
}

pub fn havok_convert_batch(
    src_dir: impl AsRef<Path>,
    dst_dir: impl AsRef<Path>,
    target_version: &str,
    preserve_structure: bool,
) -> HavokResult<HavokBatchResult> {
    let src_dir = src_dir.as_ref();
    let dst_dir = dst_dir.as_ref();
    let mut hkx_files = Vec::new();
    collect_hkx_files(src_dir, &mut hkx_files)?;
    hkx_files.sort();
    reject_duplicate_batch_destinations(src_dir, dst_dir, &hkx_files, preserve_structure)?;

    let target_version = target_version.to_string();
    let mut outcomes: Vec<_> = hkx_files
        .par_iter()
        .map(|src_path| {
            let dst_path = batch_destination(src_dir, dst_dir, src_path, preserve_structure);
            match havok_convert_file(src_path, &dst_path, &target_version) {
                Ok(()) => BatchOutcome::Converted,
                Err(error) => BatchOutcome::Error(HavokBatchError {
                    path: src_path.display().to_string(),
                    error: error.to_string(),
                }),
            }
        })
        .collect();

    let converted = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, BatchOutcome::Converted))
        .count();
    let mut errors: Vec<_> = outcomes
        .drain(..)
        .filter_map(|outcome| match outcome {
            BatchOutcome::Converted => None,
            BatchOutcome::Error(error) => Some(error),
        })
        .collect();
    errors.sort_by(|left, right| left.path.cmp(&right.path));

    Ok(HavokBatchResult {
        converted,
        skipped: 0,
        errors,
    })
}

enum BatchOutcome {
    Converted,
    Error(HavokBatchError),
}

fn reject_duplicate_batch_destinations(
    src_dir: &Path,
    dst_dir: &Path,
    hkx_files: &[PathBuf],
    preserve_structure: bool,
) -> HavokResult<()> {
    let mut destinations = HashSet::new();
    for src_path in hkx_files {
        let dst_path = batch_destination(src_dir, dst_dir, src_path, preserve_structure);
        if !destinations.insert(dst_path.clone()) {
            return Err(HavokError::InvalidInput(format!(
                "duplicate batch destination: {}",
                dst_path.display()
            )));
        }
    }
    Ok(())
}

fn collect_hkx_files(dir: &Path, out: &mut Vec<PathBuf>) -> HavokResult<()> {
    for entry in std::fs::read_dir(dir).map_err(|source| HavokError::Io {
        path: dir.display().to_string(),
        operation: "read_dir",
        source,
    })? {
        let entry = entry.map_err(|source| HavokError::Io {
            path: dir.display().to_string(),
            operation: "read_dir_entry",
            source,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| HavokError::Io {
            path: path.display().to_string(),
            operation: "file_type",
            source,
        })?;
        if file_type.is_dir() {
            collect_hkx_files(&path, out)?;
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("hkx"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn batch_destination(
    src_dir: &Path,
    dst_dir: &Path,
    src_path: &Path,
    preserve_structure: bool,
) -> PathBuf {
    if preserve_structure {
        if let Ok(relative) = src_path.strip_prefix(src_dir) {
            return dst_dir.join(relative);
        }
    }
    dst_dir.join(src_path.file_name().unwrap_or_default())
}

/// Round-trip a packfile through `patch_hkx`: parse, then overlay any current
/// model values onto the source bytes.
///
/// Mirrors the patcher branch of `py_creation_lib/python/creation_lib/hkxpack/__init__.py::save_hkx`. With no
/// intervening mutation this is byte-exact with the input. Returns
/// `HavokError::InvalidInput` (mirroring Python's `CannotPatch`) when an
/// array's serialized length no longer matches its source length.
pub fn hkx_patch_roundtrip(data: &[u8]) -> HavokResult<Vec<u8>> {
    let hkx = hkx::read_packfile(data)?;
    hkx::patcher::patch_hkx(&hkx)
}

// ---------------------------------------------------------------------------
// Native XML I/O
// ---------------------------------------------------------------------------

/// Accept HKX bytes (auto-detect packfile vs tagfile), parse, and serialize to TagXML.
///
/// Returns the XML string; callers are responsible for writing it to a file if needed.
pub fn havok_hkx_to_xml(data: &[u8]) -> HavokResult<String> {
    let hkx_file = match hkx_detect_format(data)? {
        "packfile" => hkx::read_packfile(data)?,
        "tagfile" => hkx::parse_tagfile(data)?.materialize_hkx()?,
        "binary_tagfile" => hkx::tagfile2014::read_tagfile2014(data)?,
        other => return Err(HavokError::UnsupportedFormat(other.to_string())),
    };
    hkx::tagxml::write_tagxml_string(&hkx_file)
}

/// Accept a TagXML string, parse it, serialize to a Havok packfile, and return the bytes.
pub fn havok_xml_to_hkx(xml: &str) -> HavokResult<Vec<u8>> {
    let hkx_file = hkx::tagxml::read_tagxml_string(xml)?;
    let mut registry = hkx::descriptors::DescriptorRegistry::new();
    Ok(hkx::write_hkx(&hkx_file, &mut registry))
}

/// Parse HKX bytes and return a JSON envelope wrapping the TagXML representation.
///
/// Output: `{"format": "tagxml", "content": "<hkpackfile ...>...</hkpackfile>"}`
///
/// Pair with `hkx_save_from_json` for edit cycles.
pub fn hkx_load_to_json(data: &[u8]) -> HavokResult<String> {
    let xml = havok_hkx_to_xml(data)?;
    serde_json::to_string(&serde_json::json!({"format": "tagxml", "content": xml}))
        .map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Accept a JSON envelope produced by `hkx_load_to_json` and return HKX packfile bytes.
///
/// Input: `{"format": "tagxml", "content": "<hkpackfile ...>...</hkpackfile>"}`
pub fn hkx_save_from_json(json_str: &str) -> HavokResult<Vec<u8>> {
    let v: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| HavokError::InvalidInput(format!("hkx_save_from_json: invalid JSON: {e}")))?;
    let xml = v.get("content").and_then(|c| c.as_str()).ok_or_else(|| {
        HavokError::InvalidInput("hkx_save_from_json: missing 'content' key".into())
    })?;
    havok_xml_to_hkx(xml)
}

/// Compress per-frame transforms (as JSON) into a spline-compressed binary blob.
///
/// Input JSON shape: `[[{"translation":[x,y,z],"rotation":[x,y,z,w],"scale":[x,y,z]}, ...], ...]`
/// (same shape as `havok_decompress_spline` output).
///
/// Returns raw compressed bytes suitable for the `data` member of an
/// `hkaSplineCompressedAnimation` object.
pub fn havok_compress_spline(frames_json: &str, duration: f32, fps: f32) -> HavokResult<Vec<u8>> {
    use crate::animation::spline::{SplineFrame, SplineTransform, compress_spline};

    let raw: Vec<Vec<serde_json::Value>> = serde_json::from_str(frames_json).map_err(|e| {
        HavokError::InvalidInput(format!("havok_compress_spline: invalid frames JSON: {e}"))
    })?;

    let frames: Vec<SplineFrame> = raw
        .into_iter()
        .map(|frame_arr| {
            let transforms: Vec<SplineTransform> = frame_arr
                .into_iter()
                .map(|t| {
                    let translation = [
                        t["translation"][0].as_f64().unwrap_or(0.0) as f32,
                        t["translation"][1].as_f64().unwrap_or(0.0) as f32,
                        t["translation"][2].as_f64().unwrap_or(0.0) as f32,
                    ];
                    let rotation = [
                        t["rotation"][0].as_f64().unwrap_or(0.0) as f32,
                        t["rotation"][1].as_f64().unwrap_or(0.0) as f32,
                        t["rotation"][2].as_f64().unwrap_or(0.0) as f32,
                        t["rotation"][3].as_f64().unwrap_or(1.0) as f32,
                    ];
                    let scale = [
                        t["scale"][0].as_f64().unwrap_or(1.0) as f32,
                        t["scale"][1].as_f64().unwrap_or(1.0) as f32,
                        t["scale"][2].as_f64().unwrap_or(1.0) as f32,
                    ];
                    SplineTransform {
                        translation,
                        rotation,
                        scale,
                    }
                })
                .collect();
            SplineFrame { transforms }
        })
        .collect();

    let blob = compress_spline(&frames, duration, fps)?;
    Ok(blob.data)
}

/// Generate per-version classxml directories from an SDK patches directory.
///
/// `source_dir`: base classxml directory (e.g. `resource/classxml`).
/// `patches_dir`: SDK patches directory (e.g. `refs/hk.../Patches`).
/// `output_base`: parent output directory; each target goes under `<output_base>/classxml_<suffix>/`.
/// `targets_json`: JSON array of `[suffix, version_id]` pairs,
///   e.g. `[["2012", 46], ["2013", 49], ["2015", 57]]`.
/// `base_version_id`: version ID of the source classxml (53 for FO4).
pub fn generate_classxml(
    source_dir: &str,
    patches_dir: &str,
    output_base: &str,
    targets_json: &str,
    base_version_id: i32,
) -> HavokResult<()> {
    use crate::asset::classxml::{ClassXmlTarget, generate_per_version_classxml};
    use std::path::Path;

    let raw_targets: Vec<(String, i32)> = serde_json::from_str(targets_json).map_err(|e| {
        HavokError::InvalidInput(format!("generate_classxml: invalid targets JSON: {e}"))
    })?;

    let targets: Vec<ClassXmlTarget> = raw_targets
        .into_iter()
        .map(|(suffix, version_id)| ClassXmlTarget { suffix, version_id })
        .collect();

    generate_per_version_classxml(
        Path::new(source_dir),
        Path::new(patches_dir),
        &targets,
        Path::new(output_base),
        base_version_id,
    )
}

/// Parse a Havok packfile and return its header, sections, classnames, and
/// fixup tables as JSON.
///
/// Output JSON shape:
/// ```json
/// {
///   "header": {"version": int, "version_name": str, "padding_size": int,
///              "pointer_size": int, "section_header_size": int,
///              "contents_section_index": int, "contents_section_offset": int,
///              "contents_class_name_section_index": int,
///              "contents_class_name_section_offset": int},
///   "sections": [{"name": str, "offset": int, "data1": int, "data2": int,
///                 "data3": int, "exports": int, "imports": int, "end": int}, ...],
///   "classnames": [{"position": int, "signature": int, "name": str}, ...],
///   "local_fixups":  [[source, target], ...],
///   "global_fixups": [[source, section, target], ...],
///   "virtual_fixups":[[source, section, classname_offset], ...]
/// }
/// ```
///
/// Section `offset`/`data1`/.../`end` fields are absolute byte offsets into
/// the packfile (the section start has already been added to the relative
/// values stored in the section header table).
pub fn hkx_inspect_packfile(data: &[u8]) -> HavokResult<String> {
    let parsed = hkx::packfile::parse_packfile(data)?;
    let value = serde_json::json!({
        "header": {
            "version": parsed.header.version,
            "version_name": parsed.header.version_name,
            "padding_size": parsed.header.padding_size,
            "pointer_size": parsed.header.pointer_size,
            "section_header_size": parsed.header.section_header_size,
            "contents_section_index": parsed.header.contents_section_index,
            "contents_section_offset": parsed.header.contents_section_offset,
            "contents_class_name_section_index": parsed.header.contents_class_name_section_index,
            "contents_class_name_section_offset": parsed.header.contents_class_name_section_offset,
        },
        "sections": parsed.sections.iter().map(|s| serde_json::json!({
            "name": s.name,
            "offset": s.offset,
            "data1": s.data1,
            "data2": s.data2,
            "data3": s.data3,
            "exports": s.exports,
            "imports": s.imports,
            "end": s.end,
        })).collect::<Vec<_>>(),
        "classnames": parsed.classnames.iter().map(|c| serde_json::json!({
            "position": c.position,
            "signature": c.signature,
            "name": c.name,
        })).collect::<Vec<_>>(),
        "local_fixups": parsed.local_fixups.iter()
            .map(|f| serde_json::json!([f.source, f.target]))
            .collect::<Vec<_>>(),
        "global_fixups": parsed.global_fixups.iter()
            .map(|f| serde_json::json!([f.source, f.section, f.target]))
            .collect::<Vec<_>>(),
        "virtual_fixups": parsed.virtual_fixups.iter()
            .map(|f| serde_json::json!([f.source, f.section, f.classname_offset]))
            .collect::<Vec<_>>(),
    });
    serde_json::to_string(&value).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

// ---------------------------------------------------------------------------
// High-level animation + collision API
// ---------------------------------------------------------------------------

/// Extract an `AnimationClip` from Havok XML and return it as JSON.
///
/// `skeleton_xml` is optional; when provided, bone track indices are mapped to
/// named bones from the skeleton. Without it, tracks are named `track_N`.
pub fn havok_extract_clip(xml: &str, skeleton_xml: Option<&str>) -> HavokResult<String> {
    let skeleton = match skeleton_xml {
        Some(s) => Some(parse_skeleton_xml(s)?),
        None => None,
    };
    let clip = extract_clip(xml, skeleton.as_ref())?;
    serde_json::to_string(&clip).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Deserialize a JSON `AnimationClip` and write it as interleaved Havok XML.
pub fn havok_write_animation_xml(
    clip_json: &str,
    skeleton_bone_names: &[String],
) -> HavokResult<String> {
    let clip: AnimationClip = serde_json::from_str(clip_json)
        .map_err(|e| HavokError::InvalidInput(format!("invalid clip JSON: {e}")))?;
    write_interleaved_animation_xml(&clip, skeleton_bone_names)
}

/// Extract preview meshes from a Havok collision blob and return JSON.
///
/// When `body_id` is `Some`, only the shape referenced by that body in the
/// embedded `hknpPhysicsSystemData.bodyCinfos` array is extracted.
pub fn havok_collision_preview(
    blob: &[u8],
    havok_scale: f32,
    body_id: Option<usize>,
) -> HavokResult<String> {
    collision_preview_json(blob, havok_scale, body_id)
}

fn hkx_value_as_u32(value: &crate::hkx::types::HkxValue) -> Option<u32> {
    use crate::hkx::types::HkxValue;
    match value {
        HkxValue::Bool(v) => Some(u32::from(*v)),
        HkxValue::I8(v) => Some((*v).max(0) as u32),
        HkxValue::U8(v) => Some(*v as u32),
        HkxValue::I16(v) => Some((*v).max(0) as u32),
        HkxValue::U16(v) => Some(*v as u32),
        HkxValue::I32(v) => Some((*v).max(0) as u32),
        HkxValue::U32(v) => Some(*v),
        HkxValue::I64(v) => Some((*v).max(0) as u32),
        HkxValue::U64(v) => Some((*v).min(u32::MAX as u64) as u32),
        HkxValue::F32(v) => Some((*v).max(0.0) as u32),
        HkxValue::Half(v) => Some((*v).max(0.0) as u32),
        _ => None,
    }
}

fn hkx_value_as_i64(value: &crate::hkx::types::HkxValue) -> Option<i64> {
    use crate::hkx::types::HkxValue;
    match value {
        HkxValue::Bool(v) => Some(i64::from(*v)),
        HkxValue::I8(v) => Some(*v as i64),
        HkxValue::U8(v) => Some(*v as i64),
        HkxValue::I16(v) => Some(*v as i64),
        HkxValue::U16(v) => Some(*v as i64),
        HkxValue::I32(v) => Some(*v as i64),
        HkxValue::U32(v) => Some(*v as i64),
        HkxValue::I64(v) => Some(*v),
        HkxValue::U64(v) => (*v <= i64::MAX as u64).then_some(*v as i64),
        HkxValue::F32(v) => Some(*v as i64),
        HkxValue::Half(v) => Some(*v as i64),
        _ => None,
    }
}

fn hkx_value_as_u64(value: &crate::hkx::types::HkxValue) -> Option<u64> {
    use crate::hkx::types::HkxValue;
    match value {
        HkxValue::Bool(v) => Some(u64::from(*v)),
        HkxValue::I8(v) => (*v >= 0).then_some(*v as u64),
        HkxValue::U8(v) => Some(*v as u64),
        HkxValue::I16(v) => (*v >= 0).then_some(*v as u64),
        HkxValue::U16(v) => Some(*v as u64),
        HkxValue::I32(v) => (*v >= 0).then_some(*v as u64),
        HkxValue::U32(v) => Some(*v as u64),
        HkxValue::I64(v) => (*v >= 0).then_some(*v as u64),
        HkxValue::U64(v) => Some(*v),
        HkxValue::F32(v) => (*v >= 0.0).then_some(*v as u64),
        HkxValue::Half(v) => (*v >= 0.0).then_some(*v as u64),
        _ => None,
    }
}

fn hkx_value_as_f64(value: &crate::hkx::types::HkxValue) -> Option<f64> {
    use crate::hkx::types::HkxValue;
    match value {
        HkxValue::Bool(v) => Some(if *v { 1.0 } else { 0.0 }),
        HkxValue::I8(v) => Some(*v as f64),
        HkxValue::U8(v) => Some(*v as f64),
        HkxValue::I16(v) => Some(*v as f64),
        HkxValue::U16(v) => Some(*v as f64),
        HkxValue::I32(v) => Some(*v as f64),
        HkxValue::U32(v) => Some(*v as f64),
        HkxValue::I64(v) => Some(*v as f64),
        HkxValue::U64(v) => Some(*v as f64),
        HkxValue::F32(v) => Some(*v as f64),
        HkxValue::Half(v) => Some(*v as f64),
        _ => None,
    }
}

fn hkx_object_member_value<'a>(
    obj: &'a crate::hkx::model::HkxObject,
    name: &str,
) -> Option<&'a crate::hkx::types::HkxValue> {
    obj.members
        .iter()
        .find(|member| member.name == name)
        .map(|member| &member.value)
}

fn hkx_pointer_index(
    objects: &[crate::hkx::model::HkxObject],
    value: &crate::hkx::types::HkxValue,
) -> Option<usize> {
    use crate::hkx::types::HkxValue;
    match value {
        HkxValue::Pointer(index) => *index,
        HkxValue::String { value, .. } if !value.is_empty() => objects
            .iter()
            .position(|object| object.name.as_deref() == Some(value.as_str())),
        _ => None,
    }
}

fn bs_materials_for_shape(
    objects: &[crate::hkx::model::HkxObject],
    shape_index: Option<usize>,
) -> Vec<serde_json::Value> {
    use crate::hkx::types::HkxValue;

    let Some(shape_obj) = shape_index.and_then(|idx| objects.get(idx)) else {
        return Vec::new();
    };
    let Some(properties_index) = hkx_object_member_value(shape_obj, "properties")
        .and_then(|value| hkx_pointer_index(objects, value))
    else {
        return Vec::new();
    };
    let Some(properties_obj) = objects.get(properties_index) else {
        return Vec::new();
    };
    let Some(entries) = hkx_object_member_value(properties_obj, "entries").and_then(|value| {
        if let HkxValue::Array(entries) = value {
            Some(entries)
        } else {
            None
        }
    }) else {
        return Vec::new();
    };

    let mut materials = Vec::new();
    for entry in entries {
        let Some(entry_members) = entry.as_object_members() else {
            continue;
        };
        let Some(material_props_index) = entry_members
            .iter()
            .find(|member| member.name == "object")
            .and_then(|member| hkx_pointer_index(objects, &member.value))
        else {
            continue;
        };
        let Some(material_props) = objects.get(material_props_index) else {
            continue;
        };
        if material_props.class_name != "hknpBSMaterialProperties" {
            continue;
        }
        let Some(material_array) =
            hkx_object_member_value(material_props, "MaterialA").and_then(|value| {
                if let HkxValue::Array(values) = value {
                    Some(values)
                } else {
                    None
                }
            })
        else {
            continue;
        };
        for material in material_array {
            let Some(material_members) = material.as_object_members() else {
                continue;
            };
            let filter_info = material_members
                .iter()
                .find(|member| member.name == "uiFilterInfo")
                .and_then(|member| hkx_value_as_u32(&member.value));
            let material_crc = material_members
                .iter()
                .find(|member| member.name == "uiMaterialCRC")
                .and_then(|member| hkx_value_as_u32(&member.value));
            materials.push(serde_json::json!({
                "filter_info": filter_info,
                "material_crc": material_crc,
            }));
        }
    }
    materials
}

/// Parse a Havok 2014 packfile collision blob and return classification metadata as JSON.
///
/// Output shape:
/// ```json
/// {
///   "shape_kind": "convex_polytope" | "compound_polytope" | "compound_mesh" | "compressed_mesh" | "unknown",
///   "objects": [{"class_name": str, "n_vertices": int|null, "n_faces": int|null,
///                "n_planes": int|null, "n_instances": int|null}],
///   "n_subshapes": int | null,
///   "blob_size": int
/// }
/// ```
///
/// Mirrors the Python `_parse_packfile_summary` walk in `ui/editor/panels/collision_info.py`.
pub fn havok_collision_summary(blob: &[u8]) -> HavokResult<String> {
    use crate::hkx::types::HkxValue;

    let hkx = read_collision_hkx(blob)?;
    let objects = hkx.objects();

    let mut obj_summaries: Vec<serde_json::Value> = objects
        .iter()
        .map(|obj| {
            let arr_len = |name: &str| -> Option<usize> {
                obj.members
                    .iter()
                    .find(|m| m.name == name)
                    .and_then(|m| match &m.value {
                        HkxValue::Array(items) => Some(items.len()),
                        _ => None,
                    })
            };

            let nested_arr_len = |struct_field: &str, arr_field: &str| -> Option<usize> {
                obj.members
                    .iter()
                    .find(|m| m.name == struct_field)
                    .and_then(|m| match &m.value {
                        HkxValue::Object(nested_members) => nested_members
                            .iter()
                            .find(|nm| nm.name == arr_field)
                            .and_then(|nm| match &nm.value {
                                HkxValue::Array(items) => Some(items.len()),
                                _ => None,
                            }),
                        _ => None,
                    })
            };

            serde_json::json!({
                "class_name": &obj.class_name,
                "n_vertices": arr_len("vertices"),
                "n_faces": arr_len("faces"),
                "n_planes": arr_len("planes"),
                "n_instances": nested_arr_len("instances", "elements"),
            })
        })
        .collect();

    let physics_system_data = objects
        .iter()
        .find(|o| o.class_name == "hknpPhysicsSystemData");
    let motion_cinfos: &[HkxValue] = physics_system_data
        .and_then(|psd| {
            psd.members
                .iter()
                .find(|m| m.name == "motionCinfos")
                .and_then(|m| match &m.value {
                    HkxValue::Array(items) => Some(items.as_slice()),
                    _ => None,
                })
        })
        .unwrap_or(&[]);
    let bodies: Vec<serde_json::Value> = physics_system_data
        .and_then(|psd| {
            psd.members
                .iter()
                .find(|m| m.name == "bodyCinfos")
                .and_then(|m| match &m.value {
                    HkxValue::Array(items) => Some(items),
                    _ => None,
                })
        })
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(body_id, body)| {
                    let body_members = body.as_object_members();
                    let shape_index = body_members.and_then(|members| {
                        members
                            .iter()
                            .find(|m| m.name == "shape")
                            .and_then(|m| match &m.value {
                                HkxValue::Pointer(idx) => *idx,
                                HkxValue::String { value, .. } if !value.is_empty() => objects
                                    .iter()
                                    .position(|o| o.name.as_deref() == Some(value.as_str())),
                                _ => None,
                            })
                    });
                    let shape_class = shape_index
                        .and_then(|idx| objects.get(idx))
                        .map(|o| o.class_name.clone());
                    let shape_user_data = shape_index
                        .and_then(|idx| objects.get(idx))
                        .and_then(|shape| hkx_object_member_value(shape, "userData"))
                        .and_then(hkx_value_as_u64);
                    let bs_materials = bs_materials_for_shape(objects, shape_index);
                    let material_crc = shape_user_data
                        .and_then(|value| (value <= u32::MAX as u64).then_some(value as u32))
                        .filter(|value| *value != 0)
                        .or_else(|| {
                            bs_materials.iter().find_map(|material| {
                                material
                                    .get("material_crc")
                                    .and_then(|value| value.as_u64())
                                    .and_then(|value| {
                                        (value <= u32::MAX as u64 && value != 0)
                                            .then_some(value as u32)
                                    })
                            })
                        });
                    let collision_filter_info = body_members.and_then(|members| {
                        members
                            .iter()
                            .find(|m| m.name == "collisionFilterInfo")
                            .and_then(|m| hkx_value_as_u32(&m.value))
                    });
                    let body_member = |name: &str| {
                        body_members.and_then(|members| {
                            members.iter().find(|m| m.name == name).map(|m| &m.value)
                        })
                    };
                    // The body's motionId indexes into `motionCinfos`; the matching
                    // hknpMotionCinfo carries the body's `inverseMass` and
                    // `inverseInertiaLocal`. An INVALID/out-of-range motionId (e.g. a
                    // static body's 0x7FFFFFFF) simply leaves these null.
                    let motion_cinfo = body_member("motionId")
                        .and_then(hkx_value_as_i64)
                        .and_then(|id| usize::try_from(id).ok())
                        .and_then(|id| motion_cinfos.get(id))
                        .and_then(HkxValue::as_object_members);
                    let motion_cinfo_member = |name: &str| {
                        motion_cinfo.and_then(|members| {
                            members.iter().find(|m| m.name == name).map(|m| &m.value)
                        })
                    };
                    let inverse_mass =
                        motion_cinfo_member("inverseMass").and_then(hkx_value_as_f64);
                    let inverse_inertia: Option<Vec<f64>> =
                        motion_cinfo_member("inverseInertiaLocal").map(|value| match value {
                            HkxValue::F32List(items) => {
                                items.iter().take(3).map(|v| *v as f64).collect()
                            }
                            HkxValue::Array(items) => {
                                items.iter().take(3).filter_map(hkx_value_as_f64).collect()
                            }
                            _ => Vec::new(),
                        });
                    serde_json::json!({
                        "body_id": body_id,
                        "shape_object_index": shape_index,
                        "shape_class": shape_class,
                        "shape_user_data": shape_user_data,
                        "material_crc": material_crc,
                        "bs_materials": bs_materials,
                        "collision_filter_info": collision_filter_info,
                        "layer": collision_filter_info.map(|info| info & 0xFF),
                        "flags": body_member("flags").and_then(hkx_value_as_i64),
                        "motion_id": body_member("motionId").and_then(hkx_value_as_i64),
                        "motion_type": body_member("motionType").and_then(hkx_value_as_i64),
                        "motion_properties_id": body_member("motionPropertiesId")
                            .and_then(hkx_value_as_i64),
                        "reserved_motion_id": body_member("reservedMotionId")
                            .and_then(hkx_value_as_i64),
                        "quality_id": body_member("qualityId").and_then(hkx_value_as_i64),
                        "mass": body_member("mass").and_then(hkx_value_as_f64),
                        "inverse_mass": inverse_mass,
                        "inverse_inertia": inverse_inertia,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let empty_polytope = objects.iter().any(|obj| {
        obj.class_name == "hknpConvexPolytopeShape"
            && obj
                .members
                .iter()
                .find(|m| m.name == "vertices")
                .and_then(|m| match &m.value {
                    HkxValue::Array(items) => Some(items.is_empty()),
                    _ => None,
                })
                .unwrap_or(false)
            && obj
                .members
                .iter()
                .find(|m| m.name == "faces")
                .and_then(|m| match &m.value {
                    HkxValue::Array(items) => Some(items.is_empty()),
                    _ => None,
                })
                .unwrap_or(false)
            && obj
                .members
                .iter()
                .find(|m| m.name == "planes")
                .and_then(|m| match &m.value {
                    HkxValue::Array(items) => Some(items.is_empty()),
                    _ => None,
                })
                .unwrap_or(false)
    });

    let tag0_payload = if empty_polytope {
        crate::collision::parse_tag0_collision_payload(blob)
            .ok()
            .filter(|payload| {
                !payload.vertices.is_empty()
                    && !payload.faces.is_empty()
                    && !payload.indices.is_empty()
            })
    } else {
        None
    };

    if let Some(payload) = &tag0_payload {
        if let Some(summary) = obj_summaries.iter_mut().find(|summary| {
            summary.get("class_name").and_then(|v| v.as_str()) == Some("hknpConvexPolytopeShape")
        }) {
            summary["n_vertices"] = serde_json::json!(payload.vertices.len());
            summary["n_faces"] = serde_json::json!(payload.faces.len());
            summary["n_planes"] = serde_json::json!(payload.planes.len());
            summary["geometry_source"] = serde_json::json!("tag0_payload");
        }
    }

    let geometry_status = if empty_polytope && tag0_payload.is_none() {
        "empty_polytope"
    } else {
        "ok"
    };

    let (shape_kind, n_subshapes): (&str, Option<usize>) = if bodies.len() > 1 {
        let mut classes: Vec<&str> = bodies
            .iter()
            .filter_map(|body| body.get("shape_class").and_then(|v| v.as_str()))
            .collect();
        classes.sort_unstable();
        classes.dedup();
        if classes.as_slice() == ["hknpCompressedMeshShape"] {
            ("multi_body_compressed_mesh", Some(bodies.len()))
        } else if classes.as_slice() == ["hknpConvexPolytopeShape"] {
            ("multi_body_polytope", Some(bodies.len()))
        } else {
            ("multi_body_mixed", Some(bodies.len()))
        }
    } else if objects.len() > 1 {
        match objects[1].class_name.as_str() {
            "hknpConvexPolytopeShape" => ("convex_polytope", None),
            "hknpCompoundShape" | "hknpDynamicCompoundShape" | "hknpStaticCompoundShape" => {
                let has_compressed = objects
                    .get(2..)
                    .unwrap_or(&[])
                    .iter()
                    .any(|o| o.class_name == "hknpCompressedMeshShape");
                let kind = if has_compressed {
                    "compound_mesh"
                } else {
                    "compound_polytope"
                };
                let count = objects
                    .iter()
                    .filter(|o| {
                        matches!(
                            o.class_name.as_str(),
                            "hknpBoxShape"
                                | "hknpCapsuleShape"
                                | "hknpCompressedMeshShape"
                                | "hknpConvexPolytopeShape"
                                | "hknpSphereShape"
                        )
                    })
                    .count();
                (kind, Some(count))
            }
            "hknpCompressedMeshShape" => ("compressed_mesh", None),
            _ => ("unknown", None),
        }
    } else {
        ("unknown", None)
    };

    let value = serde_json::json!({
        "shape_kind": shape_kind,
        "objects": obj_summaries,
        "bodies": bodies,
        "n_subshapes": n_subshapes,
        "geometry_status": geometry_status,
        "blob_size": blob.len(),
    });
    serde_json::to_string(&value).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

pub fn validate_collision_blob(blob: &[u8], invariants_json: &str) -> HavokResult<String> {
    use crate::collision::validate::{Invariants, validate_objects};

    let inv: Invariants = serde_json::from_str(invariants_json)
        .map_err(|e| HavokError::InvalidInput(format!("invalid invariants json: {e}")))?;
    let hkx = read_collision_hkx(blob)?;
    let violations = validate_objects(hkx.objects(), &inv);
    serde_json::to_string(&violations)
        .map_err(|e| HavokError::InvalidInput(format!("serialize violations: {e}")))
}

fn read_collision_hkx(blob: &[u8]) -> HavokResult<hkx::HkxFile> {
    match hkx_detect_format(blob)? {
        "packfile" => hkx::read_packfile(blob),
        "tagfile" => hkx::parse_tagfile(blob)?.materialize_hkx(),
        "binary_tagfile" => hkx::tagfile2014::read_tagfile2014(blob),
        other => Err(HavokError::UnsupportedFormat(other.to_string())),
    }
}

/// Build a Havok 2019 TAG0 tagged binary blob for Starfield convex collision.
///
/// Wraps `collision::payload::build_convex_collision`. The `friction`, `restitution`,
/// `layer`, and `mass` parameters are accepted for API parity with the FO4 variant but
/// are not yet applied to the blob (the reference-based builder uses the embedded
/// novablast defaults; material patching is not yet implemented).
pub fn starfield_convex_collision_blob(
    verts: &[[f32; 3]],
    _friction: f32,
    _restitution: f32,
    _layer: u8,
    _mass: f32,
) -> HavokResult<Vec<u8>> {
    crate::collision::payload::build_convex_collision(verts)
}

/// Compute a 3D convex hull and return the data needed to populate
/// `bhkConvexVerticesShape` blocks (legacy bhk chain — Skyrim SE/LE, FO3, FNV).
///
/// Returns `(hull_vertices, planes)` where:
///   - `hull_vertices` is the deduplicated subset of `verts` actually on the hull,
///     in NIF-space (caller applies any havok-scale conversion).
///   - `planes[i] = [nx, ny, nz, offset]` per face. The plane equation is
///     `n·x + offset = d_interior`; equivalently, the offset matches Python's
///     `-hull.equations[:, 3]` convention so callers can write
///     `Normal.w = offset * havok_scale` directly.
///
/// Replaces the old Python convex hull path in
/// `py_creation_lib/python/creation_lib/nif/operations/collision.py::_create_convex_shape`.
pub fn convex_hull_simple(verts: &[[f32; 3]]) -> HavokResult<(Vec<[f32; 3]>, Vec<[f32; 4]>)> {
    let topo = crate::collision::hull::compute_hull_topology(verts)?;
    let planes = topo
        .planes
        .into_iter()
        .map(|p| [p[0], p[1], p[2], -p[3]])
        .collect();
    Ok((topo.vertices, planes))
}

/// Compute a 3D convex hull and return a triangle mesh for callers that need
/// surface triangles rather than Havok plane equations.
pub fn convex_hull_triangles(verts: &[[f32; 3]]) -> HavokResult<(Vec<[f32; 3]>, Vec<[u32; 3]>)> {
    let topo = crate::collision::hull::compute_hull_topology(verts)?;
    let mut triangles = Vec::new();

    for &(first_index, num_indices, _) in &topo.faces {
        if num_indices < 3 {
            continue;
        }
        let base = first_index as usize;
        let count = num_indices as usize;
        if base + count > topo.indices.len() {
            return Err(HavokError::InvalidInput(
                "convex hull face index range exceeds index buffer".to_string(),
            ));
        }
        let first = topo.indices[base] as u32;
        for offset in 1..(count - 1) {
            triangles.push([
                first,
                topo.indices[base + offset] as u32,
                topo.indices[base + offset + 1] as u32,
            ]);
        }
    }

    Ok((topo.vertices, triangles))
}

/// Decimate a triangle mesh with the native QEM reducer.
pub fn decimate_mesh(
    vertices: &[[f32; 3]],
    triangles: &[[u32; 3]],
    target_tri_count: usize,
) -> HavokResult<(Vec<[f32; 3]>, Vec<[u32; 3]>)> {
    for (tri_index, tri) in triangles.iter().enumerate() {
        for &vertex_index in tri {
            if vertex_index as usize >= vertices.len() {
                return Err(HavokError::InvalidInput(format!(
                    "triangle {tri_index} references vertex {vertex_index}, but only {} vertices exist",
                    vertices.len()
                )));
            }
        }
    }
    let mesh = crate::geometry::decimate(vertices, triangles, target_tri_count);
    Ok((mesh.vertices, mesh.triangles))
}

/// Parse a Havok skeleton XML and return it as JSON.
pub fn havok_parse_skeleton(xml: &str) -> HavokResult<String> {
    let record = parse_skeleton_xml(xml)?;
    serde_json::to_string(&record).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Parse a Havok behavior graph XML and return it as JSON.
pub fn havok_parse_behavior(xml: &str) -> HavokResult<String> {
    let record = parse_behavior_xml(xml)?;
    serde_json::to_string(&record).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Parse a Havok behavior graph XML and return the full UI dict-node graph as JSON.
///
/// Returns a JSON object matching the shape produced by
/// `py_creation_lib/python/creation_lib/behavior/xml_import.py::import_xml_file`:
///
/// ```json
/// {
///   "nodes": {"1": {...node dict...}, ...},
///   "connections": [[port_idx, from_id, to_id], ...],
///   "global_state": {
///     "events": [...], "variables": [...], "transitions": [...],
///     "payloads": [...], "properties": [...]
///   },
///   "unhandled": [...]
/// }
/// ```
pub fn havok_behavior_graph_to_ui_json(xml: &str) -> HavokResult<String> {
    parse_behavior_graph_to_ui_json(xml)
}

/// Decompress a spline-compressed animation block and return per-frame transforms as JSON.
///
/// # Output
///
/// `[[{"translation": [x,y,z], "rotation": [x,y,z,w], "scale": [x,y,z]}, ...], ...]`
///
/// Outer array is frames; inner array is one entry per transform track.
#[allow(clippy::too_many_arguments)]
pub fn havok_decompress_spline(
    data: &[u8],
    num_transform_tracks: u32,
    num_float_tracks: u32,
    num_frames: u32,
    max_frames_per_block: u32,
    num_blocks: u32,
    block_offsets: &[u32],
    float_block_offsets: &[u32],
    mask_and_quant_size: u32,
    block_duration: f32,
    block_inverse_duration: f32,
    frame_duration: f32,
) -> HavokResult<String> {
    use crate::animation::spline::decompress_spline;
    let frames = decompress_spline(
        data,
        num_transform_tracks,
        num_float_tracks,
        num_frames,
        max_frames_per_block,
        num_blocks,
        block_offsets,
        float_block_offsets,
        mask_and_quant_size,
        block_duration,
        block_inverse_duration,
        frame_duration,
    )?;
    let json_frames: Vec<Vec<serde_json::Value>> = frames
        .iter()
        .map(|frame| {
            frame
                .transforms
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "translation": t.translation,
                        "rotation": t.rotation,
                        "scale": t.scale,
                    })
                })
                .collect()
        })
        .collect();
    serde_json::to_string(&json_frames).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

// ---------------------------------------------------------------------------
// Cloth API
// ---------------------------------------------------------------------------

/// Return a JSON summary of the class inventory inside a cloth blob.
///
/// Output shape:
/// `{ "object_count": N, "class_inventory": [{"class_name": str, "count": int}, ...],
///    "has_cloth_data": bool, "has_setup_data": bool, "has_runtime_data": bool }`
pub fn cloth_metadata_from_blob(blob_bytes: &[u8]) -> HavokResult<String> {
    let metadata = crate::cloth::cloth_metadata_from_blob(blob_bytes)?;
    let value = serde_json::json!({
        "object_count": metadata.object_count,
        "class_inventory": metadata.class_inventory.iter().map(|entry| serde_json::json!({
            "class_name": entry.class_name,
            "count": entry.count,
        })).collect::<Vec<_>>(),
        "has_cloth_data": metadata.has_cloth_data,
        "has_setup_data": metadata.has_setup_data,
        "has_runtime_data": metadata.has_runtime_data,
    });
    serde_json::to_string(&value).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Bake a `ClothSetupObject` (from JSON) into a Havok packfile and return the bytes.
///
/// The returned bytes are ready to embed into a NIF through `nif_core_native`.
pub fn cloth_bake(setup_json: &str) -> HavokResult<Vec<u8>> {
    let setup = crate::cloth::ClothSetupObject::from_json(setup_json)?;
    let hkx_file = crate::cloth::bake_cloth_setup(&setup)?;
    let mut registry = hkx::descriptors::DescriptorRegistry::new();
    Ok(hkx::write_hkx(&hkx_file, &mut registry))
}

/// Validate a cloth blob and return a JSON lint report.
///
/// NIF containers are handled by `nif_core_native`; this API accepts raw HCL
/// packfile bytes only.
pub fn cloth_validate(blob_bytes: &[u8]) -> HavokResult<String> {
    let hkx_file = hkx::read_packfile(blob_bytes)?;
    let cloth_data = crate::cloth::ClothData::from_hkx_file(&hkx_file);
    let result = crate::cloth::validate_cloth_data(cloth_data.as_ref());
    serde_json::to_string(&result.to_summary()).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Run the XPBD cloth solver for `steps` frames and return JSON of final positions.
///
/// # Input
///
/// `setup_json` — JSON-serialized `ClothSetupObject`.  Positions, triangles, and
/// constraint parameters are derived from `sim_cloth_setups[0]`.
/// `config_json` — optional override for solver parameters (`dt`, `substeps`,
///   `constraint_iterations`, `gravity`, `wind`, `damping`, `collision_epsilon`).
///   Missing fields fall back to defaults.
///
/// # Output
///
/// `{"positions": [[x, y, z], ...]}`
pub fn cloth_simulate(
    setup_json: &str,
    steps: u32,
    config_json: Option<&str>,
) -> HavokResult<String> {
    use crate::cloth::setup::cloth_setup::ClothSetupObject;
    use crate::cloth::setup::constraint_setup::ConstraintSetupObject;
    use crate::cloth::solver::{
        Capsule, DistanceConstraint, Solver, Vec3, havok_stiffness_to_compliance,
    };
    use std::collections::{BTreeSet, HashSet};

    let setup = ClothSetupObject::from_json(setup_json)
        .map_err(|e| HavokError::InvalidInput(format!("setup_json: {e}")))?;

    let scd = setup
        .sim_cloth_setups
        .first()
        .ok_or_else(|| HavokError::InvalidInput("setup has no sim_cloth_setups".into()))?;
    let mesh = scd
        .simulation_mesh
        .as_ref()
        .ok_or_else(|| HavokError::InvalidInput("sim_cloth_setup has no simulation_mesh".into()))?;

    // Positions: Vec<[f32; 4]> → Vec<[f32; 3]> (drop W).
    let positions: Vec<Vec3> = mesh.positions.iter().map(|p| [p[0], p[1], p[2]]).collect();
    let n = positions.len();

    // Build sorted unique edge set from triangles.
    let mut edges: BTreeSet<(u32, u32)> = BTreeSet::new();
    for tri in &mesh.triangles {
        let (a, b, c) = (tri[0], tri[1], tri[2]);
        edges.insert((a.min(b), a.max(b)));
        edges.insert((b.min(c), b.max(c)));
        edges.insert((a.min(c), a.max(c)));
    }

    // Derive pinned (fixed) particles from scd.fixed_particles selection.
    // kind: 0=ALL, 1=NONE, 2=CHANNEL, 3=INVERSE_CHANNEL
    let pins: HashSet<u32> = {
        let ch_name = &scd.fixed_particles.channel_name;
        match scd.fixed_particles.kind {
            0 => (0..n as u32).collect(), // ALL
            2 => {
                // CHANNEL
                if ch_name.is_empty() {
                    HashSet::new()
                } else {
                    mesh.vertex_selection_channels
                        .get(ch_name)
                        .map(|ch| ch.iter().filter(|&&v| v >= 0).map(|&v| v as u32).collect())
                        .unwrap_or_default()
                }
            }
            3 => {
                // INVERSE_CHANNEL
                let ch_set: HashSet<u32> = if ch_name.is_empty() {
                    HashSet::new()
                } else {
                    mesh.vertex_selection_channels
                        .get(ch_name)
                        .map(|ch| ch.iter().filter(|&&v| v >= 0).map(|&v| v as u32).collect())
                        .unwrap_or_default()
                };
                (0..n as u32).filter(|i| !ch_set.contains(i)).collect()
            }
            _ => HashSet::new(), // NONE or unknown
        }
    };

    let config = parse_solver_config(config_json)?;

    // Build distance constraints from StandardLink and StretchLink setups.
    let mut distance_constraints: Vec<DistanceConstraint> = Vec::new();
    for cs in &scd.constraint_setups {
        match cs {
            ConstraintSetupObject::StandardLink(sl) => {
                let stiffness = sl.stiffness.constant_value;
                let compliance = havok_stiffness_to_compliance(stiffness, config.dt);
                for &(a, b) in &edges {
                    if (a as usize) < n && (b as usize) < n {
                        let pa = positions[a as usize];
                        let pb = positions[b as usize];
                        let rest = dist3(pa, pb);
                        distance_constraints.push(DistanceConstraint {
                            a,
                            b,
                            rest_length: rest,
                            compliance,
                        });
                    }
                }
            }
            ConstraintSetupObject::StretchLink(sl) => {
                let stiffness = sl.stiffness.constant_value;
                let compliance = havok_stiffness_to_compliance(stiffness, config.dt);
                for &(a, b) in &edges {
                    let a_fixed = pins.contains(&a);
                    let b_fixed = pins.contains(&b);
                    if a_fixed != b_fixed && (a as usize) < n && (b as usize) < n {
                        let pa = positions[a as usize];
                        let pb = positions[b as usize];
                        let rest = dist3(pa, pb);
                        distance_constraints.push(DistanceConstraint {
                            a,
                            b,
                            rest_length: rest,
                            compliance,
                        });
                    }
                }
            }
            _ => {} // Bend, LocalRange, BonePlanes, Volume — not handled
        }
    }

    let masses = vec![1.0f32; n];
    let mut solver = Solver::new(
        positions,
        masses,
        distance_constraints,
        Vec::new(), // bend constraints (not yet implemented)
        Vec::new(), // range constraints
        Vec::<Capsule>::new(),
        pins,
    );
    for _ in 0..steps {
        solver.step(&config);
    }

    let out_positions: Vec<[f32; 3]> = solver.positions().to_vec();
    let value = serde_json::json!({ "positions": out_positions });
    serde_json::to_string(&value).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

// ---------------------------------------------------------------------------
// cloth_simulate_from_blob — derive solver inputs from an HCL blob
// ---------------------------------------------------------------------------

/// Run the XPBD cloth solver on an existing cloth blob for `steps` frames.
///
/// Extracts simulation inputs (particle positions, pin indices, distance
/// constraints, capsule collidables) directly from the parsed HCL packfile
/// and returns final positions as JSON.
///
/// # Input
///
/// `blob` — raw HCL packfile bytes (from `BSClothExtraData`).
/// `steps` — number of simulation frames to advance.
/// `config_json` — optional solver config override (same schema as `cloth_simulate`).
///
/// # Output
///
/// `{"positions": [[x, y, z], ...], "n_particles": N, "fixed_count": K}`
pub fn cloth_simulate_from_blob(
    blob: &[u8],
    steps: u32,
    config_json: Option<&str>,
) -> HavokResult<String> {
    let config = parse_solver_config(config_json)?;
    let inputs = cloth_solver_inputs_from_blob(blob, &config)?;
    let initial_positions = inputs.positions.clone();
    let fixed_particle_indices = inputs.fixed_particle_indices.clone();
    let fixed_count = fixed_particle_indices.len();
    let n = initial_positions.len();
    let mut solver = inputs.into_solver();
    for _ in 0..steps {
        solver.step(&config);
    }

    let out_positions: Vec<[f32; 3]> = solver.positions().to_vec();
    let value = serde_json::json!({
        "positions": out_positions,
        "initial_positions": initial_positions,
        "fixed_particle_indices": fixed_particle_indices,
        "n_particles": n,
        "fixed_count": fixed_count,
    });
    serde_json::to_string(&value).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

pub fn cloth_step_from_blob_state(
    blob: &[u8],
    positions_json: &str,
    prev_positions_json: &str,
    config_json: Option<&str>,
) -> HavokResult<String> {
    let config = parse_solver_config(config_json)?;
    let mut inputs = cloth_solver_inputs_from_blob(blob, &config)?;
    let positions = parse_vec3_json(positions_json, "positions_json")?;
    let prev_positions = parse_vec3_json(prev_positions_json, "prev_positions_json")?;
    if positions.len() != inputs.positions.len() || prev_positions.len() != inputs.positions.len() {
        return Err(HavokError::InvalidInput(format!(
            "state particle count mismatch: blob has {}, positions has {}, prev_positions has {}",
            inputs.positions.len(),
            positions.len(),
            prev_positions.len()
        )));
    }
    inputs.positions = positions;
    let mut solver = inputs.into_solver();
    solver.prev_positions = prev_positions;
    solver.step(&config);

    let out_positions: Vec<[f32; 3]> = solver.positions().to_vec();
    let prev_positions = solver.prev_positions.clone();
    let value = serde_json::json!({
        "positions": out_positions,
        "prev_positions": prev_positions,
    });
    serde_json::to_string(&value).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

struct ClothSolverInputs {
    positions: Vec<[f32; 3]>,
    masses: Vec<f32>,
    distance_constraints: Vec<crate::cloth::solver::DistanceConstraint>,
    capsules: Vec<crate::cloth::solver::Capsule>,
    pins: std::collections::HashSet<u32>,
    fixed_particle_indices: Vec<u32>,
}

impl ClothSolverInputs {
    fn into_solver(self) -> crate::cloth::solver::Solver {
        crate::cloth::solver::Solver::new(
            self.positions,
            self.masses,
            self.distance_constraints,
            Vec::new(),
            Vec::new(),
            self.capsules,
            self.pins,
        )
    }
}

fn cloth_solver_inputs_from_blob(
    blob: &[u8],
    config: &crate::cloth::solver::SolverConfig,
) -> HavokResult<ClothSolverInputs> {
    use crate::cloth::solver::{Capsule, DistanceConstraint, havok_stiffness_to_compliance};
    use crate::cloth::{ClothData, SimClothData};
    use crate::hkx::types::HkxValue;
    use std::collections::HashSet;

    let hkx = hkx::read_packfile(blob)?;
    let cloth_data = ClothData::from_hkx_file(&hkx)
        .ok_or_else(|| HavokError::InvalidInput("blob contains no hclClothData object".into()))?;
    let sim_datas = cloth_data.sim_cloth_datas();
    let scd: SimClothData<'_> = sim_datas.into_iter().next().ok_or_else(|| {
        HavokError::InvalidInput("hclClothData has no simClothDatas entries".into())
    })?;

    let positions: Vec<[f32; 3]> = scd
        .default_pose()
        .map(|pose| {
            pose.get_array("positions")
                .iter()
                .filter_map(|v| match v {
                    HkxValue::F32List(floats) if floats.len() >= 3 => {
                        Some([floats[0], floats[1], floats[2]])
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    if positions.is_empty() {
        return Err(HavokError::InvalidInput(
            "cloth pose has no particle positions".into(),
        ));
    }
    let n = positions.len();
    let pins: HashSet<u32> = scd.fixed_particle_indices().into_iter().collect();

    let mut distance_constraints: Vec<DistanceConstraint> = Vec::new();
    for cs_ref in scd.constraint_sets() {
        let class = cs_ref.class_name().to_string();
        let links = cs_ref.get_array("links");

        match class.as_str() {
            "hclStandardLinkConstraintSet" | "hclStretchLinkConstraintSet" => {
                for link in links {
                    let (a, b, stiff) = match link {
                        HkxValue::Object(members) => {
                            let a = members
                                .iter()
                                .find(|m| m.name == "particleA")
                                .and_then(|m| as_u32(&m.value));
                            let b = members
                                .iter()
                                .find(|m| m.name == "particleB")
                                .and_then(|m| as_u32(&m.value));
                            let stiff = members
                                .iter()
                                .find(|m| m.name == "stiffness")
                                .and_then(|m| as_f32(&m.value))
                                .unwrap_or(1.0);
                            match (a, b) {
                                (Some(a), Some(b)) => (a, b, stiff),
                                _ => continue,
                            }
                        }
                        _ => continue,
                    };
                    if (a as usize) < n && (b as usize) < n {
                        let rest = dist3(positions[a as usize], positions[b as usize]);
                        let compliance = havok_stiffness_to_compliance(stiff, config.dt);
                        distance_constraints.push(DistanceConstraint {
                            a,
                            b,
                            rest_length: rest,
                            compliance,
                        });
                    }
                }
            }
            "hclBendStiffnessConstraintSet" => {
                for link in links {
                    let (c, d, stiff) = match link {
                        HkxValue::Object(members) => {
                            let c = members
                                .iter()
                                .find(|m| m.name == "particleC")
                                .and_then(|m| as_u32(&m.value));
                            let d = members
                                .iter()
                                .find(|m| m.name == "particleD")
                                .and_then(|m| as_u32(&m.value));
                            let stiff = members
                                .iter()
                                .find(|m| m.name == "bendStiffness")
                                .and_then(|m| as_f32(&m.value).map(f32::abs))
                                .unwrap_or(1.0);
                            match (c, d) {
                                (Some(c), Some(d)) => (c, d, stiff),
                                _ => continue,
                            }
                        }
                        _ => continue,
                    };
                    if (c as usize) < n && (d as usize) < n {
                        let rest = dist3(positions[c as usize], positions[d as usize]);
                        if rest >= 1e-6 {
                            let compliance = havok_stiffness_to_compliance(stiff, config.dt);
                            distance_constraints.push(DistanceConstraint {
                                a: c,
                                b: d,
                                rest_length: rest,
                                compliance,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let mut capsules: Vec<Capsule> = Vec::new();
    for col_ref in scd.per_instance_collidables() {
        if let Some(shape_val) = col_ref.get_member("shape").map(|m| &m.value) {
            if let Some(shape_ref) = col_ref.resolve_ptr(shape_val) {
                if shape_ref.class_name() == "hclCapsuleShape" {
                    let start_arr = shape_ref.get_array("start");
                    let end_arr = shape_ref.get_array("end");
                    let radius = shape_ref.get_float("radius").unwrap_or(1.0);
                    let to_vec3 = |arr: &[HkxValue]| -> Option<[f32; 3]> {
                        if let Some(HkxValue::F32List(floats)) = arr.first() {
                            if floats.len() >= 3 {
                                return Some([floats[0], floats[1], floats[2]]);
                            }
                        }
                        if arr.len() >= 3 {
                            let x = as_f32(&arr[0])?;
                            let y = as_f32(&arr[1])?;
                            let z = as_f32(&arr[2])?;
                            return Some([x, y, z]);
                        }
                        None
                    };

                    if let (Some(s), Some(e)) = (to_vec3(start_arr), to_vec3(end_arr)) {
                        capsules.push(Capsule {
                            start: s,
                            end: e,
                            radius,
                        });
                    }
                }
            }
        }
    }

    let mut masses: Vec<f32> = vec![0.02f32; n];
    for (i, particle_val) in scd.particles().iter().enumerate() {
        if i >= n {
            break;
        }
        if let HkxValue::Object(members) = particle_val {
            let inv_mass = members
                .iter()
                .find(|m| m.name == "invMass")
                .and_then(|m| as_f32(&m.value));
            let mass = members
                .iter()
                .find(|m| m.name == "mass")
                .and_then(|m| as_f32(&m.value));
            if let Some(inv) = inv_mass {
                if inv > 0.0 {
                    masses[i] = 1.0 / inv;
                    continue;
                }
            }
            if let Some(v) = mass {
                if v > 0.0 {
                    masses[i] = v;
                }
            }
        }
    }

    let mut fixed_particle_indices: Vec<u32> = pins.iter().copied().collect();
    fixed_particle_indices.sort_unstable();
    Ok(ClothSolverInputs {
        positions,
        masses,
        distance_constraints,
        capsules,
        pins,
        fixed_particle_indices,
    })
}

fn parse_vec3_json(json: &str, label: &str) -> HavokResult<Vec<[f32; 3]>> {
    let rows: Vec<Vec<f32>> = serde_json::from_str(json)
        .map_err(|e| HavokError::InvalidInput(format!("{label}: invalid positions JSON: {e}")))?;
    rows.into_iter()
        .enumerate()
        .map(|(i, row)| {
            if row.len() != 3 {
                return Err(HavokError::InvalidInput(format!(
                    "{label}: position {i} has {} components, expected 3",
                    row.len()
                )));
            }
            Ok([row[0], row[1], row[2]])
        })
        .collect()
}

/// Extract a u32 from common HkxValue integer variants.
fn as_u32(v: &hkx::types::HkxValue) -> Option<u32> {
    use hkx::types::HkxValue;
    match v {
        HkxValue::U8(n) => Some(u32::from(*n)),
        HkxValue::U16(n) => Some(u32::from(*n)),
        HkxValue::U32(n) => Some(*n),
        HkxValue::I32(n) => Some(*n as u32),
        HkxValue::U64(n) => Some(*n as u32),
        HkxValue::I64(n) => Some(*n as u32),
        _ => None,
    }
}

/// Extract an f32 from F32 / integer HkxValue variants.
fn as_f32(v: &hkx::types::HkxValue) -> Option<f32> {
    use hkx::types::HkxValue;
    match v {
        HkxValue::F32(f) => Some(*f),
        HkxValue::U8(n) => Some(*n as f32),
        HkxValue::U16(n) => Some(*n as f32),
        HkxValue::U32(n) => Some(*n as f32),
        HkxValue::I32(n) => Some(*n as f32),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// ClothEditor pyfunctions — bytes-in / bytes-out wrappers
// ---------------------------------------------------------------------------

/// Run `op` inside a `ClothEditor` built from `blob`, serialize the mutated file, and
/// return the new packfile bytes together with `op`'s return value.
fn with_editor<F, T>(blob: &[u8], op: F) -> HavokResult<(Vec<u8>, T)>
where
    F: FnOnce(&mut crate::cloth::ClothEditor) -> HavokResult<T>,
{
    let mut hkx = hkx::read_packfile(blob)?;
    let result = {
        let mut editor = crate::cloth::ClothEditor::new(&mut hkx)?;
        op(&mut editor)?
    };
    let mut registry = hkx::descriptors::DescriptorRegistry::new();
    let bytes = hkx::write_hkx(&hkx, &mut registry);
    Ok((bytes, result))
}

/// Set mass for all movable (non-fixed) particles. Returns `(new_blob, count)`.
pub fn cloth_set_particle_mass_all(
    blob: &[u8],
    mass: f32,
    sim_cloth_idx: usize,
) -> HavokResult<(Vec<u8>, usize)> {
    with_editor(blob, |e| e.set_particle_mass_all(mass, sim_cloth_idx))
}

/// Scale mass of all movable particles. Returns `(new_blob, count)`.
pub fn cloth_scale_particle_mass(
    blob: &[u8],
    factor: f32,
    sim_cloth_idx: usize,
) -> HavokResult<(Vec<u8>, usize)> {
    with_editor(blob, |e| e.scale_particle_mass(factor, sim_cloth_idx))
}

/// Set radius for all particles. Returns `(new_blob, count)`.
pub fn cloth_set_particle_radius_all(
    blob: &[u8],
    radius: f32,
    sim_cloth_idx: usize,
) -> HavokResult<(Vec<u8>, usize)> {
    with_editor(blob, |e| e.set_particle_radius_all(radius, sim_cloth_idx))
}

/// Set friction for all particles. Returns `(new_blob, count)`.
pub fn cloth_set_particle_friction_all(
    blob: &[u8],
    friction: f32,
    sim_cloth_idx: usize,
) -> HavokResult<(Vec<u8>, usize)> {
    with_editor(blob, |e| {
        e.set_particle_friction_all(friction, sim_cloth_idx)
    })
}

/// Toggle a single particle between fixed and dynamic. Returns `(new_blob, count)`.
pub fn cloth_set_particle_fixed(
    blob: &[u8],
    particle_index: usize,
    fixed: bool,
    sim_cloth_idx: usize,
) -> HavokResult<(Vec<u8>, usize)> {
    with_editor(blob, |e| {
        e.set_particle_fixed(particle_index, fixed, sim_cloth_idx)
    })
}

/// Toggle multiple particles. Returns `(new_blob, count)`.
pub fn cloth_set_particles_fixed(
    blob: &[u8],
    indices: &[u32],
    fixed: bool,
    sim_cloth_idx: usize,
) -> HavokResult<(Vec<u8>, usize)> {
    with_editor(blob, |e| {
        e.set_particles_fixed(indices, fixed, sim_cloth_idx)
    })
}

/// Set mass on a subset of particles. Returns `(new_blob, count)`.
pub fn cloth_set_particles_mass(
    blob: &[u8],
    indices: &[usize],
    mass: f32,
    sim_cloth_idx: usize,
) -> HavokResult<(Vec<u8>, usize)> {
    with_editor(blob, |e| e.set_particles_mass(indices, mass, sim_cloth_idx))
}

/// Set radius on a subset of particles. Returns `(new_blob, count)`.
pub fn cloth_set_particles_radius(
    blob: &[u8],
    indices: &[usize],
    radius: f32,
    sim_cloth_idx: usize,
) -> HavokResult<(Vec<u8>, usize)> {
    with_editor(blob, |e| {
        e.set_particles_radius(indices, radius, sim_cloth_idx)
    })
}

/// Scale stiffness for matching constraint sets. Returns `(new_blob, count)`.
pub fn cloth_scale_stiffness(
    blob: &[u8],
    constraint_class: Option<&str>,
    factor: f32,
    sim_cloth_idx: usize,
) -> HavokResult<(Vec<u8>, usize)> {
    with_editor(blob, |e| {
        e.scale_stiffness(constraint_class, factor, sim_cloth_idx)
    })
}

/// Set absolute stiffness for matching constraint sets. Returns `(new_blob, count)`.
pub fn cloth_set_stiffness(
    blob: &[u8],
    constraint_class: Option<&str>,
    value: f32,
    sim_cloth_idx: usize,
) -> HavokResult<(Vec<u8>, usize)> {
    with_editor(blob, |e| {
        e.set_stiffness(constraint_class, value, sim_cloth_idx)
    })
}

/// Set gravity vector. Returns new blob bytes.
pub fn cloth_set_gravity(
    blob: &[u8],
    gravity: [f32; 4],
    sim_cloth_idx: usize,
) -> HavokResult<Vec<u8>> {
    let (bytes, ()) = with_editor(blob, |e| e.set_gravity(gravity, sim_cloth_idx))?;
    Ok(bytes)
}

/// Set global damping. Returns new blob bytes.
pub fn cloth_set_damping(blob: &[u8], damping: f32, sim_cloth_idx: usize) -> HavokResult<Vec<u8>> {
    let (bytes, ()) = with_editor(blob, |e| e.set_damping(damping, sim_cloth_idx))?;
    Ok(bytes)
}

/// Set collision tolerance. Returns new blob bytes.
pub fn cloth_set_collision_tolerance(
    blob: &[u8],
    tolerance: f32,
    sim_cloth_idx: usize,
) -> HavokResult<Vec<u8>> {
    let (bytes, ()) = with_editor(blob, |e| {
        e.set_collision_tolerance(tolerance, sim_cloth_idx)
    })?;
    Ok(bytes)
}

/// Set substeps on the simulate operator. Returns new blob bytes.
pub fn cloth_set_substeps(blob: &[u8], substeps: u32) -> HavokResult<Vec<u8>> {
    let (bytes, ()) = with_editor(blob, |e| e.set_substeps(substeps))?;
    Ok(bytes)
}

/// Set solver iterations on the simulate operator. Returns new blob bytes.
pub fn cloth_set_solver_iterations(blob: &[u8], iterations: u32) -> HavokResult<Vec<u8>> {
    let (bytes, ()) = with_editor(blob, |e| e.set_solver_iterations(iterations))?;
    Ok(bytes)
}

/// Set capsule radius. Returns new blob bytes.
pub fn cloth_set_capsule_radius(
    blob: &[u8],
    collidable_idx: usize,
    radius: f32,
    sim_cloth_idx: usize,
) -> HavokResult<Vec<u8>> {
    let (bytes, ()) = with_editor(blob, |e| {
        e.set_capsule_radius(collidable_idx, radius, sim_cloth_idx)
    })?;
    Ok(bytes)
}

/// Scale all capsule radii. Returns `(new_blob, count)`.
pub fn cloth_scale_all_capsule_radii(
    blob: &[u8],
    factor: f32,
    sim_cloth_idx: usize,
) -> HavokResult<(Vec<u8>, usize)> {
    with_editor(blob, |e| e.scale_all_capsule_radii(factor, sim_cloth_idx))
}

/// Set capsule start/end endpoints. Returns new blob bytes.
pub fn cloth_set_capsule_endpoints(
    blob: &[u8],
    collidable_idx: usize,
    start: [f32; 4],
    end: [f32; 4],
    sim_cloth_idx: usize,
) -> HavokResult<Vec<u8>> {
    let (bytes, ()) = with_editor(blob, |e| {
        e.set_capsule_endpoints(collidable_idx, start, end, sim_cloth_idx)
    })?;
    Ok(bytes)
}

/// Add a new capsule. Returns `(new_blob, new_collidable_idx)`.
pub fn cloth_add_capsule(
    blob: &[u8],
    bone_name: &str,
    radius: f32,
    start: [f32; 4],
    end: [f32; 4],
    sim_cloth_idx: usize,
) -> HavokResult<(Vec<u8>, usize)> {
    with_editor(blob, |e| {
        e.add_capsule(bone_name, radius, start, end, sim_cloth_idx)
    })
}

/// Remove a capsule. Returns new blob bytes.
pub fn cloth_remove_capsule(
    blob: &[u8],
    collidable_idx: usize,
    sim_cloth_idx: usize,
) -> HavokResult<Vec<u8>> {
    let (bytes, ()) = with_editor(blob, |e| e.remove_capsule(collidable_idx, sim_cloth_idx))?;
    Ok(bytes)
}

/// Return a JSON summary of the cloth parameters.
pub fn cloth_summary_json(blob: &[u8], sim_cloth_idx: usize) -> HavokResult<String> {
    let mut hkx = hkx::read_packfile(blob)?;
    let editor = crate::cloth::ClothEditor::new(&mut hkx)?;
    editor.summary_json(sim_cloth_idx)
}

/// Return a JSON representation of every HKX object in the cloth blob.
///
/// Output shape (native returns JSON, Python formats):
/// ```json
/// {
///   "havok_version": "hk_2014.1.0-r1",
///   "object_count": N,
///   "objects": [{"class": "...", "name": "...", "members": { ... }}, ...]
/// }
/// ```
pub fn cloth_inspect_blob_json(blob: &[u8]) -> HavokResult<String> {
    use crate::cloth::edit::hkx_value_to_json as val_json;

    let hkx = hkx::read_packfile(blob)?;
    let version = hkx.contents_version().to_string();

    let objects: Vec<serde_json::Value> = hkx
        .objects()
        .iter()
        .map(|obj| {
            let members: serde_json::Map<String, serde_json::Value> = obj
                .members
                .iter()
                .map(|m| (m.name.clone(), val_json(&m.value)))
                .collect();

            serde_json::json!({
                "class": obj.class_name,
                "name": obj.name.as_deref().unwrap_or(""),
                "members": members,
            })
        })
        .collect();

    let value = serde_json::json!({
        "havok_version": version,
        "object_count": objects.len(),
        "objects": objects,
    });

    serde_json::to_string(&value).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

// ---------------------------------------------------------------------------
// cloth_inspect_full_json — complete workspace-display JSON
// ---------------------------------------------------------------------------

/// Return the full cloth inspection JSON needed by the UI workspace.
///
/// Parses the HCL packfile blob and returns a complete, walkable JSON tree
/// covering every field the cloth_maker UI panels and cloth_skin_bind read:
/// particles (with positions from the default pose), fixed_particle_indices,
/// simulation_info, constraint_sets (with per-link detail), collidables (with
/// shape geometry), poses.
///
/// JSON shape:
/// ```json
/// {
///   "name": "...",
///   "sim_cloths": [{
///     "name": "...",
///     "particles": [{"position": [x,y,z], "mass": f, "inv_mass": f, "radius": f, "friction": f}, ...],
///     "fixed_particle_indices": [...],
///     "simulation_info": {"gravity": [...], "globalDampingPerSecond": f, "collisionTolerance": f, ...},
///     "constraint_sets": [{"class_name": "...", "name": "...", "link_count": N, "links": [...]}, ...],
///     "collidables": [{"name": "...", "shape_class": "hclCapsuleShape", "start": [...], "end": [...], "radius": f}, ...],
///     "poses": [{"name": "...", "positions": [[x,y,z], ...]}, ...]
///   }],
///   "operators": [{"class_name": "...", "name": "..."}, ...],
///   "cloth_states": [{"name": "...", "operator_indices": [...], "used_sim_cloth_indices": [...]}, ...]
/// }
/// ```
pub fn cloth_inspect_full_json(blob: &[u8]) -> HavokResult<String> {
    use crate::cloth::runtime::cloth_data::ClothData;
    use crate::cloth::runtime::cloth_state::ClothState;

    let hkx = hkx::read_packfile(blob)?;

    let cloth = ClothData::from_hkx_file(&hkx)
        .ok_or_else(|| HavokError::InvalidInput("No hclClothData in blob".to_string()))?;

    let cloth_name = cloth.name().to_string();

    let sim_cloths: Vec<serde_json::Value> = cloth
        .sim_cloth_datas()
        .into_iter()
        .map(|scd| inspect_sim_cloth(&scd, &hkx))
        .collect();

    let operators: Vec<serde_json::Value> = cloth
        .operators()
        .into_iter()
        .map(|op| {
            serde_json::json!({
                "class_name": op.class_name(),
                "name": op.get_string("name").unwrap_or(""),
            })
        })
        .collect();

    let cloth_states: Vec<serde_json::Value> = cloth
        .cloth_states()
        .into_iter()
        .map(|cs_ref| {
            let cs = ClothState::new(cs_ref);
            serde_json::json!({
                "name": cs.name(),
                "operator_indices": cs.operator_indices(),
                "used_sim_cloth_indices": cs.used_sim_cloth_indices(),
            })
        })
        .collect();

    let value = serde_json::json!({
        "name": cloth_name,
        "sim_cloths": sim_cloths,
        "operators": operators,
        "cloth_states": cloth_states,
    });

    serde_json::to_string(&value).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Build the JSON for one `hclSimClothData`.
fn inspect_sim_cloth(
    scd: &crate::cloth::runtime::sim_cloth_data::SimClothData<'_>,
    _hkx: &hkx::HkxFile,
) -> serde_json::Value {
    use crate::cloth::edit::hkx_value_to_json;
    use crate::cloth::runtime::sim_cloth_pose::SimClothPose;
    use crate::hkx::types::HkxValue;

    let name = scd.name().to_string();

    // --- Poses (needed before particles so positions are available) ---
    let pose_refs = scd.sim_cloth_poses();
    let poses_json: Vec<serde_json::Value> = pose_refs
        .iter()
        .map(|pose_ref| {
            let typed = SimClothPose::new(*pose_ref);
            let pose_name = typed.name().to_string();
            let positions: Vec<serde_json::Value> = typed
                .positions()
                .iter()
                .map(|p| serde_json::json!([p[0], p[1], p[2]]))
                .collect();
            serde_json::json!({"name": pose_name, "positions": positions})
        })
        .collect();

    // Default pose positions indexed by particle (for particle.position)
    let default_positions: Vec<[f32; 3]> = pose_refs
        .first()
        .map(|pose_ref| SimClothPose::new(*pose_ref).positions())
        .unwrap_or_default();

    // --- Particles ---
    let fixed_set: std::collections::HashSet<u32> =
        scd.fixed_particle_indices().into_iter().collect();
    let particles_json: Vec<serde_json::Value> = scd
        .particles()
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (mass, inv_mass, radius, friction) = if let HkxValue::Object(members) = p {
                let get_f32 = |name: &str| -> f32 {
                    members
                        .iter()
                        .find(|m| m.name == name)
                        .and_then(|m| {
                            if let HkxValue::F32(v) = &m.value {
                                Some(*v)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0.0)
                };
                (
                    get_f32("mass"),
                    get_f32("invMass"),
                    get_f32("radius"),
                    get_f32("friction"),
                )
            } else {
                (0.0f32, 0.0f32, 0.0f32, 0.0f32)
            };
            let pos = default_positions.get(i).copied().unwrap_or([0.0, 0.0, 0.0]);
            serde_json::json!({
                "position": [pos[0], pos[1], pos[2]],
                "mass": mass,
                "inv_mass": inv_mass,
                "radius": radius,
                "friction": friction,
            })
        })
        .collect();

    let fixed_indices_json: Vec<serde_json::Value> = {
        let mut fixed_vec: Vec<u32> = fixed_set.into_iter().collect();
        fixed_vec.sort();
        fixed_vec.iter().map(|&i| serde_json::json!(i)).collect()
    };

    // --- Simulation info ---
    let sim_info_json: serde_json::Value = scd
        .as_ref()
        .get_member("simulationInfo")
        .and_then(|m| {
            if let HkxValue::Object(info_members) = &m.value {
                let map: serde_json::Map<String, serde_json::Value> = info_members
                    .iter()
                    .map(|im| (im.name.clone(), hkx_value_to_json(&im.value)))
                    .collect();
                Some(serde_json::Value::Object(map))
            } else {
                None
            }
        })
        .unwrap_or(serde_json::json!({}));

    // --- Constraint sets ---
    let constraint_sets_json: Vec<serde_json::Value> = scd
        .constraint_sets()
        .iter()
        .map(|cs_ref| {
            let class_name = cs_ref.class_name().to_string();
            let cs_name = cs_ref.get_string("name").unwrap_or("").to_string();
            let links_arr = cs_ref.get_array("links");
            let link_count = links_arr.len();

            let _stiffness_key = if class_name == "hclBendStiffnessConstraintSet" {
                "bendStiffness"
            } else {
                "stiffness"
            };

            let links_json: Vec<serde_json::Value> = links_arr
                .iter()
                .map(|link| {
                    if let HkxValue::Object(members) = link {
                        let map: serde_json::Map<String, serde_json::Value> = members
                            .iter()
                            .map(|m| (m.name.clone(), hkx_value_to_json(&m.value)))
                            .collect();
                        serde_json::Value::Object(map)
                    } else {
                        serde_json::json!({})
                    }
                })
                .collect();

            serde_json::json!({
                "class_name": class_name,
                "name": cs_name,
                "link_count": link_count,
                "links": links_json,
            })
        })
        .collect();

    // --- Collidables ---
    let collidables_json: Vec<serde_json::Value> = scd.per_instance_collidables().iter().filter_map(|col_ref| {
        let col_name = col_ref.get_string("name").unwrap_or("").to_string();
        let shape_ptr = col_ref.get_member("shape")?;
        let shape_ref = col_ref.resolve_ptr(&shape_ptr.value)?;
        let shape_class = shape_ref.class_name().to_string();

        if shape_class == "hclCapsuleShape" {
            let start = shape_ref.get_member("start")
                .and_then(|m| if let HkxValue::F32List(v) = &m.value { Some(v.clone()) } else { None })
                .unwrap_or_else(|| vec![0.0, 0.0, 0.0, 0.0]);
            let end = shape_ref.get_member("end")
                .and_then(|m| if let HkxValue::F32List(v) = &m.value { Some(v.clone()) } else { None })
                .unwrap_or_else(|| vec![0.0, 0.0, 0.0, 0.0]);
            let radius = shape_ref.get_float("radius").unwrap_or(0.0);
            Some(serde_json::json!({
                "name": col_name,
                "shape_class": shape_class,
                "start": [start.get(0).copied().unwrap_or(0.0), start.get(1).copied().unwrap_or(0.0), start.get(2).copied().unwrap_or(0.0)],
                "end": [end.get(0).copied().unwrap_or(0.0), end.get(1).copied().unwrap_or(0.0), end.get(2).copied().unwrap_or(0.0)],
                "radius": radius,
            }))
        } else if shape_class == "hclSphereShape" {
            // Try both "centre" and "center" spellings
            let center = shape_ref.get_member("centre")
                .or_else(|| shape_ref.get_member("center"))
                .and_then(|m| if let HkxValue::F32List(v) = &m.value { Some(v.clone()) } else { None })
                .unwrap_or_else(|| vec![0.0, 0.0, 0.0, 0.0]);
            let radius = shape_ref.get_float("radius").unwrap_or(0.0);
            Some(serde_json::json!({
                "name": col_name,
                "shape_class": shape_class,
                "center": [center.get(0).copied().unwrap_or(0.0), center.get(1).copied().unwrap_or(0.0), center.get(2).copied().unwrap_or(0.0)],
                "radius": radius,
            }))
        } else {
            // Unknown shape type — emit minimal entry so UI can show it
            Some(serde_json::json!({
                "name": col_name,
                "shape_class": shape_class,
            }))
        }
    }).collect();

    serde_json::json!({
        "name": name,
        "particles": particles_json,
        "fixed_particle_indices": fixed_indices_json,
        "simulation_info": sim_info_json,
        "constraint_sets": constraint_sets_json,
        "collidables": collidables_json,
        "poses": poses_json,
    })
}

// ---------------------------------------------------------------------------
// Cloth helper pyfunctions
// ---------------------------------------------------------------------------

/// Return a JSON array of all material preset names.
pub fn cloth_material_list() -> HavokResult<String> {
    let names: Vec<&str> = crate::cloth::materials::PRESETS
        .iter()
        .map(|p| p.name)
        .collect();
    serde_json::to_string(&names).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Return a JSON object with all fields of the named material preset.
pub fn cloth_material_get(name: &str) -> HavokResult<String> {
    let p = crate::cloth::materials::get_preset(name)?;
    let v = serde_json::json!({
        "name": p.name,
        "particle_mass": p.particle_mass,
        "particle_radius": p.particle_radius,
        "particle_friction": p.particle_friction,
        "standard_link_stiffness": p.standard_link_stiffness,
        "stretch_link_stiffness": p.stretch_link_stiffness,
        "bend_stiffness": p.bend_stiffness,
        "global_damping_per_second": p.global_damping_per_second,
        "gravity_factor": p.gravity_factor,
        "collision_tolerance": p.collision_tolerance,
        "num_substeps": p.num_substeps,
        "num_solve_iterations": p.num_solve_iterations,
    });
    serde_json::to_string(&v).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Patch all sim_cloth_setups in a ClothSetupObject JSON with the named material preset.
/// Returns the updated setup JSON.
pub fn cloth_material_apply(setup_json: &str, preset_name: &str) -> HavokResult<String> {
    use crate::cloth::setup::cloth_setup::ClothSetupObject;
    let p = crate::cloth::materials::get_preset(preset_name)?;
    let mut setup: ClothSetupObject = serde_json::from_str(setup_json)
        .map_err(|e| HavokError::InvalidInput(format!("setup JSON parse error: {e}")))?;
    let vfi_constant = |v: f32| crate::cloth::setup::types::VertexFloatInput::constant(v);
    for sc in &mut setup.sim_cloth_setups {
        sc.particle_mass = vfi_constant(p.particle_mass);
        sc.particle_radius = vfi_constant(p.particle_radius);
        sc.particle_friction = vfi_constant(p.particle_friction);
        sc.global_damping_per_second = p.global_damping_per_second;
        sc.collision_tolerance = p.collision_tolerance;
        for cs in &mut sc.constraint_setups {
            use crate::cloth::setup::constraint_setup::ConstraintSetupObject;
            match cs {
                ConstraintSetupObject::StandardLink(s) => {
                    s.stiffness = vfi_constant(p.standard_link_stiffness);
                }
                ConstraintSetupObject::StretchLink(s) => {
                    s.stiffness = vfi_constant(p.stretch_link_stiffness);
                }
                ConstraintSetupObject::BendStiffness(s) => {
                    s.bend_stiffness = vfi_constant(p.bend_stiffness);
                }
                _ => {}
            }
        }
    }
    setup.to_json()
}

/// Return a JSON array of topology preset summaries.
pub fn cloth_topology_list() -> HavokResult<String> {
    let arr: Vec<serde_json::Value> = crate::cloth::topology::TOPOLOGY_PRESETS
        .iter()
        .map(|p| topology_preset_to_json(p))
        .collect();
    serde_json::to_string(&arr).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Return a JSON object with the named topology preset's fields.
pub fn cloth_topology_get(name: &str) -> HavokResult<String> {
    let p = crate::cloth::topology::get_preset(name)?;
    let v = topology_preset_to_json(p);
    serde_json::to_string(&v).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

fn topology_preset_to_json(p: &crate::cloth::topology::TopologyPreset) -> serde_json::Value {
    let constraints: Vec<&str> = {
        let mut v = Vec::new();
        if p.use_standard_links {
            v.push("StandardLink");
        }
        if p.use_stretch_links {
            v.push("StretchLink");
        }
        if p.use_bend_stiffness {
            v.push("BendStiffness");
        }
        if p.use_local_range {
            v.push("LocalRange");
        }
        if p.use_bone_planes {
            v.push("BonePlanes");
        }
        if p.use_volume {
            v.push("Volume");
        }
        v
    };
    serde_json::json!({
        "name": p.name,
        "description": p.description,
        "use_standard_links": p.use_standard_links,
        "use_stretch_links": p.use_stretch_links,
        "use_bend_stiffness": p.use_bend_stiffness,
        "use_local_range": p.use_local_range,
        "use_bone_planes": p.use_bone_planes,
        "use_volume": p.use_volume,
        "standard_link_stiffness": p.standard_link_stiffness,
        "stretch_link_stiffness": p.stretch_link_stiffness,
        "bend_stiffness": p.bend_stiffness,
        "local_range_max_distance": p.local_range_max_distance,
        "local_range_stiffness": p.local_range_stiffness,
        "volume_stiffness": p.volume_stiffness,
        "default_material": p.default_material,
        "auto_capsule_radius": p.auto_capsule_radius,
        "num_substeps": p.num_substeps,
        "num_solve_iterations": p.num_solve_iterations,
        "constraints": constraints,
        "stiffness": {
            "standard_link": p.standard_link_stiffness,
            "stretch_link": p.stretch_link_stiffness,
            "bend": p.bend_stiffness,
        },
    })
}

/// Generate a full ClothSetupObject JSON from a region definition JSON.
///
/// `region_json` shape: `{"name": str, "positions": [[x,y,z,w],...],
///   "triangles": [[i,j,k],...], "fixed_indices": [i,...]}`
/// `topology_name`: name of topology preset
/// `args_json`: optional overrides `{"material": str, "parent_bone": str,
///   "bone_rows": int, "bone_cols": int}`
///
/// Returns a JSON string of a `ClothSetupObject`.
pub fn cloth_region_generate(
    region_json: &str,
    topology_name: &str,
    args_json: &str,
) -> HavokResult<String> {
    use crate::cloth::materials;
    use crate::cloth::setup::buffer_setup::{
        BufferSetupObject, BufferType, TransformSetSetupObject,
    };
    use crate::cloth::setup::cloth_setup::ClothSetupObject;
    use crate::cloth::setup::mesh::{SetupMesh, SimulationSetupMesh};
    use crate::cloth::setup::operator_setup::{
        CopyVerticesSetup, MoveParticlesSetup, OperatorSetupObject, SimulateSetup,
        SimulateSetupConfig, SkinSetup,
    };
    use crate::cloth::setup::sim_cloth_setup::SimClothSetupObject;
    use crate::cloth::setup::types::{VertexFloatInput, VertexSelectionInput};
    use crate::cloth::skeleton;
    use crate::cloth::skinning;
    use crate::cloth::topology;

    // --- Parse region definition ---
    #[derive(serde::Deserialize)]
    struct RegionDef {
        #[serde(default)]
        name: String,
        positions: Vec<[f32; 4]>,
        triangles: Vec<[u32; 3]>,
        #[serde(default)]
        fixed_indices: Vec<usize>,
    }
    let region: RegionDef = serde_json::from_str(region_json)
        .map_err(|e| HavokError::InvalidInput(format!("region JSON parse error: {e}")))?;

    if region.positions.is_empty() || region.triangles.is_empty() {
        return Err(HavokError::InvalidInput(
            "region must have non-empty positions and triangles".into(),
        ));
    }

    // --- Parse args ---
    #[derive(serde::Deserialize, Default)]
    struct Args {
        material: Option<String>,
        parent_bone: Option<String>,
        bone_rows: Option<usize>,
        bone_cols: Option<usize>,
    }
    let args: Args = if args_json.trim() == "{}" || args_json.trim().is_empty() {
        Args::default()
    } else {
        serde_json::from_str(args_json)
            .map_err(|e| HavokError::InvalidInput(format!("args JSON parse error: {e}")))?
    };

    let parent_bone = args.parent_bone.as_deref().unwrap_or("COM");
    let bone_rows = args.bone_rows.unwrap_or(4);
    let bone_cols = args.bone_cols.unwrap_or(4);

    // --- Resolve topology and material ---
    let topo = topology::get_preset(topology_name)?;
    let mat_name = args.material.as_deref().unwrap_or(topo.default_material);
    let mat = materials::get_preset(mat_name)?;

    let n_particles = region.positions.len();
    let region_name = &region.name;

    // --- Generate cloth bones ---
    let bone_prefix = format!("Cloth_BN_{region_name}");
    let bones = skeleton::generate_bones_from_particles(
        &region.positions,
        &bone_prefix,
        bone_rows,
        bone_cols,
        parent_bone,
    );
    let (bone_names, bone_positions) = skeleton::bones_to_transform_set(&bones);

    // --- Auto-skin ---
    let skin_weights = skinning::auto_skin_to_cloth_bones(
        &region.positions,
        &bone_positions,
        4,   // max_bones_per_vertex
        2.0, // falloff_power
    );

    // Convert skin weights to SetupMesh format: Vec<Vec<[f32; 2]>>
    // SetupMesh stores [bone_index_as_f32, weight] pairs.
    let bone_weights_mesh: Vec<Vec<[f32; 2]>> = skin_weights
        .iter()
        .map(|vw| vw.iter().map(|&(bi, w)| [bi as f32, w]).collect())
        .collect();

    // --- Build setup mesh ---
    let setup_mesh = SetupMesh {
        name: region_name.clone(),
        positions: region.positions.clone(),
        triangles: region.triangles.clone(),
        bone_names: bone_names.clone(),
        bone_weights: bone_weights_mesh,
        ..Default::default()
    };

    let n = n_particles as u32;
    let sim_to_render: Vec<Vec<u32>> = (0..n).map(|i| vec![i]).collect();
    let render_to_sim: Vec<u32> = (0..n).collect();

    let sim_mesh = SimulationSetupMesh {
        positions: region.positions.clone(),
        triangles: region.triangles.clone(),
        sim_to_render_map: sim_to_render,
        render_to_sim_map: render_to_sim,
        source_mesh: Some(Box::new(setup_mesh.clone())),
        ..Default::default()
    };

    // --- Fixed particles ---
    let valid_fixed: Vec<usize> = region
        .fixed_indices
        .iter()
        .copied()
        .filter(|&i| i < n_particles)
        .collect();

    let fixed_sel = if valid_fixed.is_empty() {
        VertexSelectionInput::none()
    } else {
        VertexSelectionInput::all()
    };

    // --- Build constraints ---
    let constraints = topo.build_constraints(region_name, n_particles, &valid_fixed);

    // --- Auto-capsules from bounding box ---
    let collidable_setups = auto_capsules_from_topology(topo, &region.positions);

    // --- Sim cloth setup ---
    let sim_cloth = SimClothSetupObject {
        name: region_name.clone(),
        simulation_mesh: Some(sim_mesh),
        gravity: [0.0, 0.0, -686.7 * mat.gravity_factor, 0.0],
        global_damping_per_second: mat.global_damping_per_second,
        collision_tolerance: mat.collision_tolerance,
        particle_mass: VertexFloatInput::constant(mat.particle_mass),
        particle_radius: VertexFloatInput::constant(mat.particle_radius),
        particle_friction: VertexFloatInput::constant(mat.particle_friction),
        fixed_particles: fixed_sel,
        constraint_setups: constraints,
        collidable_setups,
        ..Default::default()
    };

    // --- Buffers ---
    let buffer_setups = vec![
        BufferSetupObject {
            name: "display".to_string(),
            buffer_type: BufferType::Display as u8,
            setup_mesh: Some(setup_mesh),
            has_normals: true,
            ..Default::default()
        },
        BufferSetupObject {
            name: "static_display".to_string(),
            buffer_type: BufferType::StaticDisplay as u8,
            ..Default::default()
        },
    ];

    // --- Transform sets ---
    let ts_name = "skeleton".to_string();
    let transform_set_setups = vec![TransformSetSetupObject {
        name: ts_name.clone(),
        bone_names: bone_names.clone(),
        skeleton_name: String::new(),
    }];

    // --- Operators ---
    let operator_setups = vec![
        OperatorSetupObject::Simulate(SimulateSetup {
            name: "simulate".to_string(),
            sim_cloth_setup_name: region_name.clone(),
            configs: vec![SimulateSetupConfig {
                name: "default".to_string(),
                num_substeps: topo.num_substeps as usize,
                num_solve_iterations: topo.num_solve_iterations as usize,
                ..Default::default()
            }],
        }),
        OperatorSetupObject::Skin(SkinSetup {
            name: "skin".to_string(),
            transform_set_name: ts_name.clone(),
            output_buffer_name: "display".to_string(),
            skin_normals: true,
            ..Default::default()
        }),
        OperatorSetupObject::CopyVertices(CopyVerticesSetup {
            name: "copy".to_string(),
            input_buffer_name: "display".to_string(),
            output_buffer_name: "static_display".to_string(),
            copy_normals: true,
        }),
        OperatorSetupObject::MoveParticles(MoveParticlesSetup {
            name: "move_fixed".to_string(),
            sim_cloth_setup_name: region_name.clone(),
            display_buffer_name: "display".to_string(),
        }),
    ];

    // --- States ---
    let state_setups = vec![serde_json::json!({
        "name": "Simulated",
        "operator_indices": (0..operator_setups.len()).collect::<Vec<_>>(),
    })];

    let setup = ClothSetupObject {
        name: region_name.clone(),
        buffer_setups,
        transform_set_setups,
        sim_cloth_setups: vec![sim_cloth],
        operator_setups,
        state_setups,
    };

    setup.to_json()
}

/// Default body capsule definitions — mirrors Python topology._DEFAULT_BODY_BONES.
const DEFAULT_BODY_BONES: &[(&str, [f32; 4], [f32; 4], f32)] = &[
    (
        "Pelvis",
        [0.0, 0.0, -35.0, 0.0],
        [0.0, 0.0, -40.0, 0.0],
        8.0,
    ),
    (
        "LLeg_Thigh",
        [-5.0, 0.0, -34.0, 0.0],
        [-5.0, 0.0, -54.0, 0.0],
        5.0,
    ),
    (
        "RLeg_Thigh",
        [5.0, 0.0, -34.0, 0.0],
        [5.0, 0.0, -54.0, 0.0],
        5.0,
    ),
    (
        "LLeg_Calf",
        [-5.0, 0.0, -54.0, 0.0],
        [-5.0, 0.0, -74.0, 0.0],
        4.0,
    ),
    (
        "RLeg_Calf",
        [5.0, 0.0, -54.0, 0.0],
        [5.0, 0.0, -74.0, 0.0],
        4.0,
    ),
];

fn auto_capsules_from_topology(
    topo: &crate::cloth::topology::TopologyPreset,
    positions: &[[f32; 4]],
) -> Vec<crate::cloth::setup::collidable_setup::CollidableSetup> {
    use crate::cloth::setup::collidable_setup::{CapsuleShapeSetup, CollidableSetup};

    if positions.is_empty() {
        return Vec::new();
    }

    let z_min = positions.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
    let z_max = positions
        .iter()
        .map(|p| p[2])
        .fold(f32::NEG_INFINITY, f32::max);

    DEFAULT_BODY_BONES
        .iter()
        .filter_map(|(bone_name, start, end, radius)| {
            let cap_z_min = start[2].min(end[2]);
            let cap_z_max = start[2].max(end[2]);
            if z_min <= cap_z_max && z_max >= cap_z_min {
                Some(CollidableSetup {
                    name: bone_name.to_string(),
                    shape: Some(CapsuleShapeSetup {
                        start: *start,
                        end: *end,
                        big_radius: *radius + topo.auto_capsule_radius * 0.1,
                        small_radius: *radius,
                    }),
                    driving_bone_name: bone_name.to_string(),
                    ..Default::default()
                })
            } else {
                None
            }
        })
        .collect()
}

/// Reverse a cloth blob to a ClothSetupObject and return as JSON.
///
/// Reads a Havok packfile blob, parses it, extracts the hclClothData,
/// and calls `reverse_cloth_data` to produce a best-effort setup JSON.
pub fn cloth_reverse_to_setup(blob: &[u8]) -> HavokResult<String> {
    let hkx = crate::hkx::read_packfile(blob)?;
    let cloth_data = crate::cloth::ClothData::from_hkx_file(&hkx)
        .ok_or_else(|| HavokError::InvalidInput("no hclClothData found in blob".into()))?;
    // Inspect-style API: tolerate unknown operator/constraint classes by
    // stubbing them, so a single future-Havok class doesn't blow up the
    // whole inspector. The strict reverse_cloth_data is reserved for
    // edit/bake round-trip workflows.
    let setup = crate::cloth::reverse::reverse_cloth_data_lossy(&cloth_data);
    setup.to_json()
}

/// Generate ClothBone list from particle positions and return as JSON array.
///
/// `positions_json`: JSON array of `[x, y, z, w]` positions.
/// `args_json`: `{"bone_prefix": str, "rows": int, "cols": int, "parent_bone": str}`.
///
/// Returns a JSON array of `{"name": str, "position": [x,y,z,w], "parent_bone": str}`.
pub fn cloth_generate_bones_from_particles(
    positions_json: &str,
    args_json: &str,
) -> HavokResult<String> {
    use crate::cloth::skeleton;

    let positions: Vec<[f32; 4]> = serde_json::from_str(positions_json)
        .map_err(|e| HavokError::InvalidInput(format!("positions JSON parse error: {e}")))?;

    #[derive(serde::Deserialize, Default)]
    struct Args {
        bone_prefix: Option<String>,
        rows: Option<usize>,
        cols: Option<usize>,
        parent_bone: Option<String>,
    }
    let args: Args = serde_json::from_str(args_json).unwrap_or_default();

    let prefix = args.bone_prefix.as_deref().unwrap_or("Cloth_BN");
    let rows = args.rows.unwrap_or(1);
    let cols = args.cols.unwrap_or(1);
    let parent = args.parent_bone.as_deref().unwrap_or("COM");

    let bones = skeleton::generate_bones_from_particles(&positions, prefix, rows, cols, parent);

    let arr: Vec<serde_json::Value> = bones
        .iter()
        .map(|b| {
            serde_json::json!({
                "name": b.name,
                "position": b.position,
                "parent_bone": b.parent_bone,
            })
        })
        .collect();

    serde_json::to_string(&arr).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Extract `{"names": [...], "positions": [[x,y,z,w],...]}` from a bone list JSON.
///
/// `bones_json`: JSON array of `{"name": str, "position": [x,y,z,w], "parent_bone": str}`.
pub fn cloth_bones_to_transform_set(bones_json: &str) -> HavokResult<String> {
    #[derive(serde::Deserialize)]
    struct BoneEntry {
        name: String,
        position: [f32; 4],
    }
    let bones: Vec<BoneEntry> = serde_json::from_str(bones_json)
        .map_err(|e| HavokError::InvalidInput(format!("bones JSON parse error: {e}")))?;

    let names: Vec<&str> = bones.iter().map(|b| b.name.as_str()).collect();
    let positions: Vec<[f32; 4]> = bones.iter().map(|b| b.position).collect();

    let v = serde_json::json!({
        "names": names,
        "positions": positions,
    });
    serde_json::to_string(&v).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Compute per-vertex bone weights and return as JSON.
///
/// `positions_json`: vertex positions `[[x,y,z,w],...]`
/// `bone_positions_json`: bone positions `[[x,y,z,w],...]`
/// `args_json`: `{"max_bones_per_vertex": int, "falloff_power": float}`
///
/// Returns JSON `[[[bone_idx, weight], ...], ...]` — one entry per vertex.
pub fn cloth_auto_skin(
    positions_json: &str,
    bone_positions_json: &str,
    args_json: &str,
) -> HavokResult<String> {
    use crate::cloth::skinning::auto_skin_to_cloth_bones;

    let positions: Vec<[f32; 4]> = serde_json::from_str(positions_json)
        .map_err(|e| HavokError::InvalidInput(format!("positions JSON parse error: {e}")))?;
    let bone_positions: Vec<[f32; 4]> = serde_json::from_str(bone_positions_json)
        .map_err(|e| HavokError::InvalidInput(format!("bone_positions JSON parse error: {e}")))?;

    #[derive(serde::Deserialize, Default)]
    struct Args {
        max_bones_per_vertex: Option<usize>,
        falloff_power: Option<f32>,
    }
    let args: Args = serde_json::from_str(args_json).unwrap_or_default();
    let max_bones = args.max_bones_per_vertex.unwrap_or(4);
    let falloff = args.falloff_power.unwrap_or(2.0);

    let weights = auto_skin_to_cloth_bones(&positions, &bone_positions, max_bones, falloff);

    // Serialize as [[bone_idx, weight], ...] per vertex
    let arr: Vec<Vec<(usize, f64)>> = weights
        .into_iter()
        .map(|vw| vw.into_iter().map(|(bi, w)| (bi, w as f64)).collect())
        .collect();

    serde_json::to_string(&arr).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Return a JSON array of template summaries.
pub fn cloth_template_list() -> HavokResult<String> {
    crate::cloth::templates::template_list_json()
}

/// Return a JSON object with full template details.
pub fn cloth_template_get(name: &str) -> HavokResult<String> {
    crate::cloth::templates::template_get_json(name)
}

/// Build a cloth template and return the serialized HCL packfile blob.
///
/// `args_json`: `{"material": str, "parent_bone": str}`
pub fn cloth_template_blob(name: &str, args_json: &str) -> HavokResult<Vec<u8>> {
    crate::cloth::templates::template_blob(name, args_json)
}

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn parse_solver_config(
    config_json: Option<&str>,
) -> HavokResult<crate::cloth::solver::SolverConfig> {
    use crate::cloth::solver::SolverConfig;
    let defaults = SolverConfig::default();
    let Some(cfg_str) = config_json else {
        return Ok(defaults);
    };
    #[derive(serde::Deserialize, Default)]
    struct RawConfig {
        dt: Option<f32>,
        substeps: Option<u32>,
        constraint_iterations: Option<u32>,
        gravity: Option<[f32; 3]>,
        wind: Option<[f32; 3]>,
        damping: Option<f32>,
        collision_epsilon: Option<f32>,
    }
    let raw: RawConfig = serde_json::from_str(cfg_str)
        .map_err(|e| HavokError::InvalidInput(format!("config_json: {e}")))?;
    Ok(SolverConfig {
        dt: raw.dt.unwrap_or(defaults.dt),
        substeps: raw.substeps.unwrap_or(defaults.substeps),
        constraint_iterations: raw
            .constraint_iterations
            .unwrap_or(defaults.constraint_iterations),
        gravity: raw.gravity.unwrap_or(defaults.gravity),
        wind: raw.wind.unwrap_or(defaults.wind),
        damping: raw.damping.unwrap_or(defaults.damping),
        damping_per_second: None,
        collision_epsilon: raw.collision_epsilon.unwrap_or(defaults.collision_epsilon),
    })
}

// ---------------------------------------------------------------------------
// Asset discovery / manifest pyfunctions
// ---------------------------------------------------------------------------

/// Walk a Meshes directory and return classified file entries as JSON.
pub fn walk_meshes_dir_json(meshes_dir: &str, source: &str) -> HavokResult<String> {
    use crate::asset::discovery::walk_meshes_dir;
    use std::path::Path;
    let entries = walk_meshes_dir(Path::new(meshes_dir));
    // Augment with source so callers can reconstruct manifest IDs.
    let json_entries: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "rel_path": e.rel_path,
                "role": e.role,
                "category": e.category,
                "file_type": e.file_type,
                "is_xml": e.is_xml,
                "source": source,
            })
        })
        .collect();
    serde_json::to_string(&json_entries).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Classify a file path's category and return as a string.
pub fn classify_category_str(path: &str) -> &'static str {
    crate::asset::discovery::classify_category(path)
}

/// Classify a file path's role and return as a string.
pub fn classify_role_str(path: &str) -> &'static str {
    crate::asset::discovery::classify_role(path, None)
}

/// Build manifests from JSON-encoded entries + character_data, return JSON.
///
/// `entries_json` — JSON array of objects with at minimum: `rel_path`, `role`,
///   `category`, `file_type`, `is_xml`.  This is the shape emitted by
///   `walk_meshes_dir_json`.
/// `character_data_json` — JSON object mapping `rel_path → CharacterRecord`.
/// `source` — game source identifier (fo4, fo76, starfield).
pub fn build_manifests_json(
    entries_json: &str,
    character_data_json: &str,
    source: &str,
) -> HavokResult<String> {
    use crate::animation::parsers::CharacterRecord;
    use crate::asset::discovery::FileEntry;
    use crate::asset::manifest::build_manifests;
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[derive(serde::Deserialize)]
    struct EntryInput {
        rel_path: String,
        #[serde(default)]
        role: String,
        #[serde(default)]
        category: String,
        #[serde(default)]
        file_type: String,
        #[serde(default)]
        is_xml: bool,
    }

    let raw_entries: Vec<EntryInput> = serde_json::from_str(entries_json)
        .map_err(|e| HavokError::InvalidInput(format!("entries_json parse error: {e}")))?;

    let entries: Vec<FileEntry> = raw_entries
        .into_iter()
        .map(|e| FileEntry {
            abs_path: PathBuf::new(),
            rel_path: e.rel_path,
            role: e.role,
            category: e.category,
            file_type: e.file_type,
            is_xml: e.is_xml,
        })
        .collect();

    let character_data: HashMap<String, CharacterRecord> =
        serde_json::from_str(character_data_json).map_err(|e| {
            HavokError::InvalidInput(format!("character_data_json parse error: {e}"))
        })?;

    let manifests = build_manifests(&entries, &character_data, source);
    serde_json::to_string(&manifests).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Parse a Havok animation XML and return structured metadata as JSON.
pub fn parse_animation_xml_json(xml: &str) -> HavokResult<String> {
    use crate::animation::parsers::parse_animation_xml_str;
    let record = parse_animation_xml_str(xml)?;
    serde_json::to_string(&record).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Parse a Havok character XML and return structured metadata as JSON.
pub fn parse_character_xml_json(xml: &str) -> HavokResult<String> {
    use crate::animation::parsers::parse_character_xml;
    let record = parse_character_xml(xml)?;
    serde_json::to_string(&record).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Parse a Havok project XML and return structured metadata as JSON.
pub fn parse_project_xml_json(xml: &str) -> HavokResult<String> {
    use crate::animation::parsers::parse_project_xml;
    let record = parse_project_xml(xml)?;
    serde_json::to_string(&record).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_packfile_magic() {
        let mut data = Vec::from(*HKX_MAGIC);
        data.extend_from_slice(b"rest");
        assert_eq!(hkx_detect_format(&data).unwrap(), "packfile");
    }

    #[test]
    fn rejects_packfile_prefix_without_full_magic() {
        let mut data = Vec::from(*b"\x57\xE0\xE0\x57");
        data.extend_from_slice(b"rest");

        let error = hkx_detect_format(&data).unwrap_err();

        assert!(
            error.to_string().contains("unsupported Havok format"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn detects_tag0_magic() {
        let mut data = Vec::from(*b"\0\0\0\0");
        data.extend_from_slice(TAG0_MAGIC);
        data.extend_from_slice(b"rest");
        assert_eq!(hkx_detect_format(&data).unwrap(), "tagfile");
    }

    #[test]
    fn roundtrip_rejects_truncated_packfile() {
        let error = hkx_roundtrip_bytes(b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10").unwrap_err();
        assert!(
            error.to_string().contains("packfile header"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn roundtrip_rejects_malformed_tag0() {
        let error = hkx_roundtrip_bytes(b"\0\0\0\0TAG0rest").unwrap_err();
        assert!(
            error.to_string().contains("invalid HFF section"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn convert_rejects_unknown_target_version() {
        let error = havok_convert_bytes(b"\0\0\0\0TAG0rest", "hk_nope").unwrap_err();
        assert!(
            error.to_string().contains("unknown Havok version"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn target_classxml_warning_reports_unknown_output_class() {
        let hkx_file = hkx::HkxFile::from_tagxml(
            11,
            "hk_2014.1.0-r1",
            vec![hkx::HkxObject {
                name: Some("#0015".to_string()),
                offset: 0,
                signature: 0,
                class_name: "hknpBoxShape".to_string(),
                members: Vec::new(),
            }],
        );

        let warnings = collect_target_classxml_warnings(&hkx_file, "hk_2014.1.0-r1");

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("hknpBoxShape"));
        assert!(warnings[0].contains("#0015"));
    }

    #[test]
    fn convex_hull_triangles_fans_polygon_faces() {
        let verts = vec![
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ];

        let (hull_verts, triangles) = convex_hull_triangles(&verts).expect("cube hull");

        assert_eq!(hull_verts.len(), 8);
        assert_eq!(triangles.len(), 12);
        for tri in triangles {
            assert!(
                tri.iter().all(|index| (*index as usize) < hull_verts.len()),
                "triangle has out-of-range index: {tri:?}"
            );
        }
    }

    #[test]
    fn decimate_mesh_rejects_invalid_triangle_indices() {
        let verts = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let triangles = vec![[0, 1, 3]];

        let error = decimate_mesh(&verts, &triangles, 1).unwrap_err();

        assert!(
            error.to_string().contains("triangle 0 references vertex 3"),
            "unexpected error: {error}"
        );
    }

    // ── detect_format returns kind+version ───────────────

    #[test]
    fn detect_format_full_returns_packfile_version_name() {
        // Real fo4 fixture lives outside the api crate; build a synthetic
        // packfile header carrying the version_name string at 0x28.
        let mut data = vec![0u8; 0x40];
        data[0..8].copy_from_slice(HKX_MAGIC);
        let name = b"hk_2014.1.0-r1";
        data[0x28..0x28 + name.len()].copy_from_slice(name);
        let result = hkx_detect_format_full(&data).unwrap();
        assert_eq!(result.kind, "packfile");
        assert_eq!(result.version, "hk_2014.1.0-r1");
    }

    #[test]
    fn detect_format_full_recognizes_skyrim_se_binary_tagfile() {
        // 0xCAB00D1E 0xD011FACE little-endian, version word at offset 12.
        let mut data = vec![0u8; 16];
        data[0..4].copy_from_slice(&0xCAB0_0D1Eu32.to_le_bytes());
        data[4..8].copy_from_slice(&0xD011_FACEu32.to_le_bytes());
        data[12..16].copy_from_slice(&13u32.to_le_bytes());
        let result = hkx_detect_format_full(&data).unwrap();
        assert_eq!(result.kind, "binary_tagfile");
        assert_eq!(result.version, "v13");
    }

    #[test]
    fn detect_format_full_returns_tagfile_for_tag0_marker() {
        let mut data = Vec::from(*b"\0\0\0\0");
        data.extend_from_slice(TAG0_MAGIC);
        data.extend_from_slice(b"\0\0\0\0SDKV20150100\0");
        let result = hkx_detect_format_full(&data).unwrap();
        assert_eq!(result.kind, "tagfile");
        assert_eq!(result.version, "20150100");
    }

    #[test]
    fn hkx_to_xml_routes_binary_tagfile_through_tagfile2014_reader() {
        // Minimal valid v13 stream (header + TAG_FILE_INFO v3 +
        // TAG_FILE_END). Verifies binary_tagfile bytes flow through
        // tagfile2014::read_tagfile2014 and out to TagXML.
        let mut data = Vec::new();
        data.extend_from_slice(&0xCAB0_0D1Eu32.to_le_bytes());
        data.extend_from_slice(&0xD011_FACEu32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes()); // tag
        data.extend_from_slice(&13u32.to_le_bytes()); // version (v13)
        // VLE-encoded ints: 1 (TAG_FILE_INFO), 3 (file-info version),
        // 7 (TAG_FILE_END). Each fits in 6 magnitude bits, so each
        // encodes as a single byte = (mag << 1).
        data.push(1u8 << 1);
        data.push(3u8 << 1);
        data.push(7u8 << 1);
        let xml = havok_hkx_to_xml(&data).expect("binary_tagfile flows through hkx_to_xml");
        assert!(
            xml.contains("<hkpackfile") || xml.contains("hkpackfile"),
            "expected TagXML envelope, got: {xml}"
        );
    }
}
