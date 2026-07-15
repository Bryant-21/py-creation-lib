//! Havok XML parsers for the five animation-domain file types:
//! skeleton, animation, behavior, character, and project.
//!
//! Each parser accepts an XML string and returns a typed record.
//! Malformed XML returns `HavokError::InvalidInput`.
//!
//! Reference Python implementations:
//! - `py_creation_lib/python/creation_lib/havok/parsers/skeleton.py`
//! - `py_creation_lib/python/creation_lib/havok/parsers/animation.py`
//! - `py_creation_lib/python/creation_lib/havok/parsers/behavior.py`
//! - `py_creation_lib/python/creation_lib/havok/parsers/character.py`
//! - `py_creation_lib/python/creation_lib/havok/parsers/project.py`

use crate::error::{HavokError, HavokResult};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Record types
// ---------------------------------------------------------------------------

/// A single bone's reference pose: translation, rotation (quaternion xyzw),
/// and scale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BonePose {
    pub t: [f32; 3],
    pub q: [f32; 4],
    pub s: [f32; 3],
}

/// Parsed skeleton metadata — mirrors `py_creation_lib/python/creation_lib/havok/parsers/skeleton.py::SkeletonData`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkeletonRecord {
    pub name: String,
    pub bone_count: usize,
    pub bone_names: Vec<String>,
    pub parent_indices: Vec<i32>,
    pub reference_pose: Vec<BonePose>,
    pub lock_translation: Vec<bool>,
    pub float_count: usize,
    pub float_slots: Vec<String>,
    pub reference_floats: Vec<f32>,
    pub partition_names: Vec<String>,
}

/// Parsed animation metadata + optional frame-0 binary blob.
/// Mirrors `py_creation_lib/python/creation_lib/havok/parsers/animation.py::AnimationData`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationRecord {
    pub duration: f32,
    pub bone_count: usize,
    pub frame_count: usize,
    /// "lossless", "spline", "interleaved", or "unknown"
    pub compression_type: String,
    pub float_track_count: usize,
    pub annotation_tracks: Vec<AnnotationEntry>,
    /// Per-bone frame-0 transforms packed as `N * 7 float32` values
    /// (qx, qy, qz, qw, tx, ty, tz) in little-endian byte order.
    /// Only populated for lossless-compressed animations.
    pub frame0_transforms: Option<Vec<u8>>,
}

/// A single annotation within an annotation track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnotationEntry {
    pub time: f32,
    pub text: String,
}

/// Parsed behavior graph metadata.
/// Mirrors `py_creation_lib/python/creation_lib/havok/parsers/behavior.py::BehaviorData`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BehaviorRecord {
    pub events: Vec<String>,
    /// `(name, type_string)` pairs; type is patched from `hkbBehaviorGraphData`.
    pub variables: Vec<(String, String)>,
    pub sequences: Vec<String>,
    pub transitions: Vec<(String, String)>,
    pub node_count: usize,
    pub node_classes: Vec<String>,
}

/// Parsed character file metadata.
/// Mirrors `py_creation_lib/python/creation_lib/havok/parsers/character.py::CharacterData`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterRecord {
    pub rig_name: String,
    pub behavior_filename: String,
    pub model_up: String,
    pub model_forward: String,
    pub model_right: String,
}

/// Parsed project file metadata.
/// Mirrors `py_creation_lib/python/creation_lib/havok/parsers/project.py::ProjectData`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub character_filenames: Vec<String>,
}

// ---------------------------------------------------------------------------
// XML helper — shared with animation/mod.rs but duplicated here to avoid
// making the mod.rs helpers pub. If upstream refactors them to pub, switch.
// ---------------------------------------------------------------------------

fn parse_doc(xml: &str) -> HavokResult<roxmltree::Document<'_>> {
    roxmltree::Document::parse(xml)
        .map_err(|e| HavokError::InvalidInput(format!("invalid XML: {e}")))
}

fn child_param<'a>(node: roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> {
    node.children()
        .find(|child| child.has_tag_name("hkparam") && child.attribute("name") == Some(name))
}

fn text_param(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    child_param(node, name).and_then(|p| p.text().map(|t| t.trim().to_string()))
}

fn int_param(node: roxmltree::Node<'_, '_>, name: &str) -> Option<i64> {
    text_param(node, name).and_then(|t| t.parse::<i64>().ok())
}

fn float_param(node: roxmltree::Node<'_, '_>, name: &str) -> Option<f32> {
    text_param(node, name).and_then(|t| t.parse::<f32>().ok())
}

/// Collect all text from `text` and nested `hkparam` children of `node`.
fn collect_text(node: roxmltree::Node<'_, '_>) -> String {
    let mut chunks: Vec<&str> = Vec::new();
    if let Some(t) = node.text() {
        if !t.trim().is_empty() {
            chunks.push(t);
        }
    }
    for child in node.children() {
        if child.has_tag_name("hkparam") {
            if let Some(t) = child.text() {
                if !t.trim().is_empty() {
                    chunks.push(t);
                }
            }
        }
    }
    chunks.join("\n")
}

// ---------------------------------------------------------------------------
// Reference pose parsing — three layouts
// ---------------------------------------------------------------------------

/// Parse parenthesized float groups from a string.
/// Returns each group as a `Vec<f32>`.
fn parse_paren_groups(text: &str) -> Vec<Vec<f32>> {
    let mut groups: Vec<Vec<f32>> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            let start = i + 1;
            if let Some(end_off) = bytes[start..].iter().position(|&b| b == b')') {
                let inner = &text[start..start + end_off];
                let vals: Vec<f32> = inner
                    .split_whitespace()
                    .filter_map(|s| s.parse::<f32>().ok())
                    .collect();
                if !vals.is_empty() {
                    groups.push(vals);
                }
                i = start + end_off + 1;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    groups
}

fn parse_reference_pose(text: &str) -> Vec<BonePose> {
    let groups = parse_paren_groups(text);
    let mut poses: Vec<BonePose> = Vec::new();
    let mut i = 0;
    while i < groups.len() {
        let vals = &groups[i];

        // Packed hkQsTransform: 12+ floats (vec4 t, quat, vec4 s)
        if vals.len() >= 12 {
            poses.push(BonePose {
                t: [vals[0], vals[1], vals[2]],
                q: [vals[4], vals[5], vals[6], vals[7]],
                s: [vals[8], vals[9], vals[10]],
            });
            i += 1;
            continue;
        }

        // Compact 10-float hkQsTransform: (tx ty tz qx qy qz qw sx sy sz)
        if vals.len() >= 10 {
            poses.push(BonePose {
                t: [vals[0], vals[1], vals[2]],
                q: [vals[3], vals[4], vals[5], vals[6]],
                s: [vals[7], vals[8], vals[9]],
            });
            i += 1;
            continue;
        }

        // Legacy triplet: separate groups (t3)(q4)(s3)
        if i + 2 < groups.len() {
            let t = &groups[i];
            let q = &groups[i + 1];
            let s = &groups[i + 2];
            if t.len() >= 3 && q.len() >= 4 && s.len() >= 3 {
                poses.push(BonePose {
                    t: [t[0], t[1], t[2]],
                    q: [q[0], q[1], q[2], q[3]],
                    s: [s[0], s[1], s[2]],
                });
                i += 3;
                continue;
            }
        }

        i += 1;
    }
    poses
}

// ---------------------------------------------------------------------------
// Skeleton parser
// ---------------------------------------------------------------------------

/// Parse a Havok skeleton XML string and return structured metadata.
///
/// Handles three reference-pose layouts:
/// - Packed `hkQsTransform` (12 floats)
/// - Compact 10-float (no vec4 padding)
/// - Legacy triplet groups `(t)(q)(s)`
pub fn parse_skeleton_xml(xml: &str) -> HavokResult<SkeletonRecord> {
    let doc = parse_doc(xml)?;

    // Find the first hkaSkeleton object.
    let skel = doc
        .descendants()
        .find(|n| n.has_tag_name("hkobject") && n.attribute("class") == Some("hkaSkeleton"))
        .ok_or_else(|| {
            HavokError::InvalidInput("XML does not contain an hkaSkeleton object".to_string())
        })?;

    let name = text_param(skel, "name").unwrap_or_default();

    // Bone names and lockTranslation flags
    let mut bone_names: Vec<String> = Vec::new();
    let mut lock_translation: Vec<bool> = Vec::new();
    if let Some(bones_param) = child_param(skel, "bones") {
        for bone_obj in bones_param
            .children()
            .filter(|c| c.has_tag_name("hkobject"))
        {
            if let Some(n) = text_param(bone_obj, "name") {
                bone_names.push(n);
            }
            let lock = text_param(bone_obj, "lockTranslation")
                .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
                .unwrap_or(false);
            // Keep lockTranslation aligned with bone_names; skip bones with no name element.
            if bone_names.len() > lock_translation.len() {
                lock_translation.push(lock);
            }
        }
    }

    // Parent indices
    let parent_indices: Vec<i32> = text_param(skel, "parentIndices")
        .map(|t| {
            t.split_whitespace()
                .filter_map(|s| s.parse::<i32>().ok())
                .collect()
        })
        .unwrap_or_default();

    // Reference pose
    let reference_pose = if let Some(pose_param) = child_param(skel, "referencePose") {
        parse_reference_pose(&collect_text(pose_param))
    } else {
        Vec::new()
    };

    // Float slots
    let mut float_slots: Vec<String> = Vec::new();
    if let Some(fs_param) = child_param(skel, "floatSlots") {
        for s in fs_param.children().filter(|c| c.has_tag_name("hkcstring")) {
            if let Some(t) = s.text() {
                float_slots.push(t.trim().to_string());
            }
        }
    }

    // Reference floats
    let reference_floats: Vec<f32> = if let Some(rf_param) = child_param(skel, "referenceFloats") {
        let text = collect_text(rf_param);
        text.split_whitespace()
            .filter_map(|s| s.parse::<f32>().ok())
            .collect()
    } else {
        Vec::new()
    };

    // Partitions
    let mut partition_names: Vec<String> = Vec::new();
    if let Some(parts_param) = child_param(skel, "partitions") {
        for part_obj in parts_param
            .children()
            .filter(|c| c.has_tag_name("hkobject"))
        {
            if let Some(n) = text_param(part_obj, "name") {
                partition_names.push(n);
            }
        }
    }

    let bone_count = bone_names.len();
    // Pad lock_translation to match bone_count
    while lock_translation.len() < bone_count {
        lock_translation.push(false);
    }
    let float_count = float_slots.len();

    Ok(SkeletonRecord {
        name,
        bone_count,
        bone_names,
        parent_indices,
        reference_pose,
        lock_translation,
        float_count,
        float_slots,
        reference_floats,
        partition_names,
    })
}

// ---------------------------------------------------------------------------
// Animation parser
// ---------------------------------------------------------------------------

/// Parse quaternion / vector parenthesized groups from a string.
/// Returns each group as `Vec<f32>`.
fn parse_quat_list(text: &str) -> Vec<Vec<f32>> {
    parse_paren_groups(text)
}

/// Decode frame-0 transforms from lossless compression data.
///
/// `type_and_offset encoding:` `value & 3 = type` (0=identity, 1=static, 2=dynamic),
/// `value >> 2 = index` into the static array.
fn decode_frame0_lossless(
    static_rotations: &[Vec<f32>],
    static_translations: &[Vec<f32>],
    rot_type_offsets: &[i32],
    trans_type_offsets: &[i32],
    num_bones: usize,
) -> Vec<u8> {
    use std::io::Write;
    let mut buf: Vec<u8> = Vec::with_capacity(num_bones * 7 * 4);

    for i in 0..num_bones {
        let q: [f32; 4] = if i < rot_type_offsets.len() {
            let rtype = rot_type_offsets[i] & 3;
            let roffset = (rot_type_offsets[i] >> 2) as usize;
            if rtype == 1
                && roffset < static_rotations.len()
                && static_rotations[roffset].len() >= 4
            {
                let r = &static_rotations[roffset];
                [r[0], r[1], r[2], r[3]]
            } else {
                [0.0, 0.0, 0.0, 1.0]
            }
        } else {
            [0.0, 0.0, 0.0, 1.0]
        };

        let t: [f32; 3] = if i < trans_type_offsets.len() {
            let ttype = trans_type_offsets[i] & 3;
            let toffset = (trans_type_offsets[i] >> 2) as usize;
            if ttype == 1
                && toffset < static_translations.len()
                && static_translations[toffset].len() >= 3
            {
                let tr = &static_translations[toffset];
                [tr[0], tr[1], tr[2]]
            } else {
                [0.0, 0.0, 0.0]
            }
        } else {
            [0.0, 0.0, 0.0]
        };

        // Write qx, qy, qz, qw, tx, ty, tz as little-endian f32
        for val in q.iter().chain(t.iter()) {
            let _ = buf.write_all(&val.to_le_bytes());
        }
    }
    buf
}

fn compression_type_from_class(class_name: &str) -> &'static str {
    if class_name.contains("Lossless") {
        "lossless"
    } else if class_name.contains("Spline") {
        "spline"
    } else if class_name.contains("Interleaved") {
        "interleaved"
    } else {
        "unknown"
    }
}

/// Parse a Havok animation XML string and return structured metadata.
///
/// For lossless animations, decodes frame-0 transforms into a compact binary
/// blob (`N * 7 float32` values: qx, qy, qz, qw, tx, ty, tz per bone).
pub fn parse_animation_xml_str(xml: &str) -> HavokResult<AnimationRecord> {
    let doc = parse_doc(xml)?;

    let skip_classes = ["hkaAnimationContainer", "hkaAnimationBinding"];
    let anim_obj = doc.descendants().find(|n| {
        if !n.has_tag_name("hkobject") {
            return false;
        }
        let cls = n.attribute("class").unwrap_or("");
        cls.starts_with("hka") && cls.contains("Animation") && !skip_classes.contains(&cls)
    });

    let Some(anim) = anim_obj else {
        return Err(HavokError::InvalidInput(
            "XML does not contain an hka animation object".to_string(),
        ));
    };

    let class_name = anim.attribute("class").unwrap_or("");
    let compression_type = compression_type_from_class(class_name).to_string();

    let duration = float_param(anim, "duration").unwrap_or(0.0);
    let bone_count = int_param(anim, "numberOfTransformTracks").unwrap_or(0) as usize;
    let float_track_count = int_param(anim, "numberOfFloatTracks").unwrap_or(0) as usize;

    // Annotation tracks — collect from hkaAnnotationTrack objects
    let mut annotation_tracks: Vec<AnnotationEntry> = Vec::new();
    for obj in doc.descendants() {
        if obj.has_tag_name("hkobject") && obj.attribute("class") == Some("hkaAnnotationTrack") {
            for ann_obj in obj.descendants().filter(|n| n.has_tag_name("hkobject")) {
                let time_p = text_param(ann_obj, "time");
                let text_p = text_param(ann_obj, "text");
                if let (Some(time_str), Some(text)) = (time_p, text_p) {
                    let time = time_str.parse::<f32>().unwrap_or(0.0);
                    annotation_tracks.push(AnnotationEntry { time, text });
                }
            }
        }
    }

    // Frame-0 lossless decode
    let frame0_transforms = if compression_type == "lossless" && bone_count > 0 {
        let static_rots = child_param(anim, "staticRotations")
            .and_then(|p| p.text())
            .map(parse_quat_list)
            .unwrap_or_default();
        let static_trans = child_param(anim, "staticTranslations")
            .and_then(|p| p.text())
            .map(parse_quat_list)
            .unwrap_or_default();
        let rot_offsets: Vec<i32> = text_param(anim, "rotationTypeAndOffsets")
            .map(|t| {
                t.split_whitespace()
                    .filter_map(|s| s.parse::<i32>().ok())
                    .collect()
            })
            .unwrap_or_default();
        let trans_offsets: Vec<i32> = text_param(anim, "translationTypeAndOffsets")
            .map(|t| {
                t.split_whitespace()
                    .filter_map(|s| s.parse::<i32>().ok())
                    .collect()
            })
            .unwrap_or_default();

        let blob = decode_frame0_lossless(
            &static_rots,
            &static_trans,
            &rot_offsets,
            &trans_offsets,
            bone_count,
        );
        Some(blob)
    } else {
        None
    };

    Ok(AnimationRecord {
        duration,
        bone_count,
        frame_count: 0, // not extracted at parse stage (full frame count from binary data)
        compression_type,
        float_track_count,
        annotation_tracks,
        frame0_transforms,
    })
}

// ---------------------------------------------------------------------------
// Behavior parser
// ---------------------------------------------------------------------------

/// Parse a Havok behavior graph XML string and return structured metadata.
pub fn parse_behavior_xml(xml: &str) -> HavokResult<BehaviorRecord> {
    let doc = parse_doc(xml)?;

    // Use __data__ section if present, else root.
    let search_root: roxmltree::Node<'_, '_> = doc
        .descendants()
        .find(|n| n.has_tag_name("hksection") && n.attribute("name") == Some("__data__"))
        .unwrap_or_else(|| doc.root_element());

    let objects: Vec<roxmltree::Node<'_, '_>> = search_root
        .children()
        .filter(|n| n.has_tag_name("hkobject"))
        .collect();

    let node_count = objects.len();
    let mut seen_classes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut node_classes: Vec<String> = Vec::new();
    let mut events: Vec<String> = Vec::new();
    let mut variables: Vec<(String, String)> = Vec::new();
    let mut sequences: Vec<String> = Vec::new();
    let mut transitions: Vec<(String, String)> = Vec::new();

    for obj in &objects {
        let cls = obj.attribute("class").unwrap_or("").to_string();
        if !cls.is_empty() && !seen_classes.contains(&cls) {
            seen_classes.insert(cls.clone());
            node_classes.push(cls.clone());
        }

        match cls.as_str() {
            "hkbBehaviorGraphStringData" => {
                for param in obj.children().filter(|c| c.has_tag_name("hkparam")) {
                    let pname = param.attribute("name").unwrap_or("");
                    match pname {
                        "eventNames" => {
                            for s in param.children().filter(|c| c.has_tag_name("hkcstring")) {
                                if let Some(t) = s.text() {
                                    events.push(t.trim().to_string());
                                }
                            }
                        }
                        "variableNames" => {
                            for s in param.children().filter(|c| c.has_tag_name("hkcstring")) {
                                if let Some(t) = s.text() {
                                    variables.push((
                                        t.trim().to_string(),
                                        "VARIABLE_TYPE_REAL".to_string(),
                                    ));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "hkbBehaviorGraphData" => {
                if let Some(vi_param) = child_param(*obj, "variableInfos") {
                    for (i, info_obj) in vi_param
                        .children()
                        .filter(|c| c.has_tag_name("hkobject"))
                        .enumerate()
                    {
                        if let Some(type_text) = text_param(info_obj, "type") {
                            if i < variables.len() {
                                let name = variables[i].0.clone();
                                variables[i] = (name, type_text);
                            }
                        }
                    }
                }
            }
            "BGSGamebryoSequenceGenerator" => {
                if let Some(seq) = text_param(*obj, "pSequence") {
                    sequences.push(seq);
                }
            }
            "hkbBlendingTransitionEffect" => {
                let tname = text_param(*obj, "name")
                    .filter(|n| !n.is_empty())
                    .or_else(|| obj.attribute("name").map(str::to_string))
                    .unwrap_or_default();
                let tdur = text_param(*obj, "duration").unwrap_or_else(|| "0".to_string());
                if !tname.is_empty() {
                    transitions.push((tname, tdur));
                }
            }
            _ => {}
        }
    }

    Ok(BehaviorRecord {
        events,
        variables,
        sequences,
        transitions,
        node_count,
        node_classes,
    })
}

// ---------------------------------------------------------------------------
// Character parser
// ---------------------------------------------------------------------------

/// Parse a Havok character XML string and return structured metadata.
pub fn parse_character_xml(xml: &str) -> HavokResult<CharacterRecord> {
    let doc = parse_doc(xml)?;

    let mut record = CharacterRecord {
        rig_name: String::new(),
        behavior_filename: String::new(),
        model_up: String::new(),
        model_forward: String::new(),
        model_right: String::new(),
    };

    for obj in doc.descendants().filter(|n| n.has_tag_name("hkobject")) {
        match obj.attribute("class").unwrap_or("") {
            "hkbCharacterStringData" => {
                if let Some(v) = text_param(obj, "rigName") {
                    record.rig_name = v;
                }
                if let Some(v) = text_param(obj, "behaviorFilename") {
                    record.behavior_filename = v;
                }
            }
            "hkbCharacterData" => {
                if let Some(v) = text_param(obj, "modelUpMS") {
                    record.model_up = v;
                }
                if let Some(v) = text_param(obj, "modelForwardMS") {
                    record.model_forward = v;
                }
                if let Some(v) = text_param(obj, "modelRightMS") {
                    record.model_right = v;
                }
            }
            _ => {}
        }
    }

    Ok(record)
}

// ---------------------------------------------------------------------------
// Behavior graph → UI dict-node graph parser
// ---------------------------------------------------------------------------

/// Parse a reference like "#0003" or "#3" to an integer node ID.
fn parse_ref(s: &str) -> Option<i64> {
    let trimmed = s.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return None;
    }
    trimmed.trim_start_matches('#').parse::<i64>().ok()
}

fn is_null(s: &str) -> bool {
    let t = s.trim();
    t.is_empty() || t == "null"
}

/// Return `s` if non-empty, otherwise `fallback`.
fn or_default(s: String, fallback: &str) -> String {
    if s.is_empty() {
        fallback.to_string()
    } else {
        s
    }
}

fn param_text<'a>(obj: roxmltree::Node<'a, 'a>, name: &str) -> String {
    child_param(obj, name)
        .and_then(|p| p.text())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn param_int(obj: roxmltree::Node<'_, '_>, name: &str, default: i64) -> i64 {
    child_param(obj, name)
        .and_then(|p| p.text())
        .and_then(|t| t.trim().parse::<i64>().ok())
        .unwrap_or(default)
}

fn param_bool(obj: roxmltree::Node<'_, '_>, name: &str, default: bool) -> bool {
    match child_param(obj, name)
        .and_then(|p| p.text())
        .map(|t| t.trim().to_lowercase())
        .as_deref()
    {
        Some("true") => true,
        Some("false") => false,
        _ => default,
    }
}

/// Add a connection if `ref_str` is not null.
fn push_conn(connections: &mut Vec<serde_json::Value>, port_idx: i64, from_id: i64, ref_str: &str) {
    if let Some(to_id) = parse_ref(ref_str) {
        connections.push(serde_json::json!([port_idx, from_id, to_id]));
    }
}

/// Add multiple connections from a list of `#NNN` refs separated by whitespace.
fn push_multi_conn(
    connections: &mut Vec<serde_json::Value>,
    port_idx: i64,
    from_id: i64,
    content: &str,
) {
    for part in content.split('#') {
        let part = part.trim();
        if !part.is_empty() {
            if let Ok(to_id) = part.parse::<i64>() {
                connections.push(serde_json::json!([port_idx, from_id, to_id]));
            }
        }
    }
}

/// Parse the 15-bit flags string used in `hkbStateMachineTransitionInfo`.
fn parse_transition_flags(flags_str: &str) -> [bool; 15] {
    let flag_map = [
        ("FLAG_USE_TRIGGER_INTERVAL", 0usize),
        ("FLAG_USE_INITIATE_INTERVAL", 1),
        ("FLAG_UNINTERRUPTIBLE_WHILE_PLAYING", 2),
        ("FLAG_UNINTERRUPTIBLE_WHILE_DELAYED", 3),
        ("FLAG_DELAY_STATE_CHANGE", 4),
        ("FLAG_DISABLED", 5),
        ("FLAG_DISALLOW_RETURN_TO_PREVIOUS_STATE", 6),
        ("FLAG_DISALLOW_RANDOM_TRANSITION", 7),
        ("FLAG_DISABLE_CONDITION", 8),
        ("FLAG_ALLOW_SELF_TRANSITION_BY_TRANSITION_FROM_ANY_STATE", 9),
        ("FLAG_IS_GLOBAL_WILDCARD", 10),
        ("FLAG_IS_LOCAL_WILDCARD", 11),
        ("FLAG_FROM_NESTED_STATE_ID_IS_VALID", 12),
        ("FLAG_TO_NESTED_STATE_ID_IS_VALID", 13),
        ("FLAG_ABUT_AT_END_OF_FROM_GENERATOR", 14),
    ];
    let mut out = [false; 15];
    for piece in flags_str.split('|') {
        let piece = piece.trim();
        for (name, idx) in &flag_map {
            if piece == *name {
                out[*idx] = true;
            }
        }
    }
    out
}

/// Encode the `selfTransitionMode` string to integer.
fn self_transition_mode_int(s: &str) -> i64 {
    match s {
        "SELF_TRANSITION_MODE_CONTINUE" => 1,
        "SELF_TRANSITION_MODE_RESET" => 2,
        "SELF_TRANSITION_MODE_BLEND" => 3,
        _ => 0,
    }
}

/// Encode the `eventMode` string to integer.
fn event_mode_int(s: &str) -> i64 {
    match s {
        "EVENT_MODE_PROCESS_ALL" => 1,
        "EVENT_MODE_IGNORE_FROM_GENERATOR" => 2,
        "EVENT_MODE_IGNORE_TO_GENERATOR" => 3,
        _ => 0,
    }
}

/// Reinterpret a u32 bit pattern as f32.
fn u32_to_f32(v: i64) -> f32 {
    f32::from_bits((v as u64 & 0xFFFF_FFFF) as u32)
}

// ---------------------------------------------------------------------------
// Global-state pre-passes (mirrors py_creation_lib/python/creation_lib/behavior/xml_import.py)
// ---------------------------------------------------------------------------

fn import_transitions(
    objects: &[roxmltree::Node<'_, '_>],
) -> (
    std::collections::HashMap<String, usize>,
    Vec<serde_json::Value>,
) {
    let mut transition_map: std::collections::HashMap<String, usize> = Default::default();
    let mut transitions: Vec<serde_json::Value> = Vec::new();
    let mut idx = 0usize;

    for obj in objects {
        if obj.attribute("class") != Some("hkbBlendingTransitionEffect") {
            continue;
        }
        idx += 1;
        let xml_name = obj.attribute("name").unwrap_or("").to_string();

        let v_bind = param_text(*obj, "variableBindingSet");
        let v_bind_ref = parse_ref(&v_bind).unwrap_or(0);
        let stm = param_text(*obj, "selfTransitionMode");
        let em = param_text(*obj, "eventMode");
        let dur = param_text(*obj, "duration");
        let to_frac = param_text(*obj, "toGeneratorStartTimeFraction");
        let flags_str = param_text(*obj, "flags");
        let end_mode_str = param_text(*obj, "endMode");
        let blend_curve_str = param_text(*obj, "blendCurve");
        let name = param_text(*obj, "name");

        let transition_flags = {
            let flag_map = [
                ("FLAG_IGNORE_FROM_WORLD_FROM_MODEL", 1i64),
                ("FLAG_SYNC", 2),
                ("FLAG_IGNORE_TO_WORLD_FROM_MODEL", 3),
                ("FLAG_IGNORE_TO_WORLD_FROM_MODEL_ROTATION", 4),
            ];
            let mut val = 0i64;
            for piece in flags_str.split('|') {
                let piece = piece.trim();
                for (n, v) in &flag_map {
                    if piece == *n {
                        val = *v;
                        break;
                    }
                }
            }
            val
        };

        let end_mode: i64 = if end_mode_str == "END_MODE_NONE" {
            0
        } else {
            1
        };
        let blend_curve: i64 = blend_curve_str.parse::<i64>().unwrap_or(0);

        let td = serde_json::json!({
            "transitionID": idx,
            "transitionName": name,
            "transitionVariableBindingSet": v_bind_ref,
            "transitionSelfTransitionMode": self_transition_mode_int(&stm),
            "transitionEventMode": event_mode_int(&em),
            "transitionDuration": dur,
            "transitionToGeneratorStartTimeFraction": to_frac,
            "transitionFlags": transition_flags,
            "transitionEndMode": end_mode,
            "transitionBlendCurve": blend_curve,
        });

        transitions.push(td);
        transition_map.insert(xml_name, idx);
    }

    (transition_map, transitions)
}

fn import_payloads(
    objects: &[roxmltree::Node<'_, '_>],
) -> (
    std::collections::HashMap<String, i64>,
    Vec<serde_json::Value>,
) {
    let mut payload_map: std::collections::HashMap<String, i64> = Default::default();
    let mut payloads: Vec<serde_json::Value> = Vec::new();
    let mut idx = 0i64;

    for obj in objects {
        if obj.attribute("class") != Some("hkbStringEventPayload") {
            continue;
        }
        idx += 1;
        let xml_name = obj.attribute("name").unwrap_or("").to_string();
        // First child hkparam is "data" (the string value)
        let data_text = obj
            .children()
            .find(|c| c.has_tag_name("hkparam"))
            .and_then(|p| p.text())
            .unwrap_or("")
            .trim()
            .to_string();
        payloads.push(serde_json::json!({
            "payloadID": idx,
            "payloadName": data_text,
        }));
        payload_map.insert(xml_name, idx);
    }

    (payload_map, payloads)
}

fn resolve_payload(ref_str: &str, payload_map: &std::collections::HashMap<String, i64>) -> i64 {
    if is_null(ref_str) {
        return -1;
    }
    payload_map.get(ref_str.trim()).copied().unwrap_or(-1)
}

fn import_global_values(
    objects: &[roxmltree::Node<'_, '_>],
) -> (
    Vec<serde_json::Value>, // events
    Vec<serde_json::Value>, // variables
    Vec<serde_json::Value>, // properties
) {
    let mut graph_data: Option<roxmltree::Node<'_, '_>> = None;
    let mut value_set: Option<roxmltree::Node<'_, '_>> = None;
    let mut string_data: Option<roxmltree::Node<'_, '_>> = None;

    for obj in objects {
        match obj.attribute("class").unwrap_or("") {
            "hkbBehaviorGraphData" => graph_data = Some(*obj),
            "hkbVariableValueSet" => value_set = Some(*obj),
            "hkbBehaviorGraphStringData" => string_data = Some(*obj),
            _ => {}
        }
    }

    let (Some(gd), Some(vs), Some(sd)) = (graph_data, value_set, string_data) else {
        return (Vec::new(), Vec::new(), Vec::new());
    };

    // Collect ordered hkparam children for each metadata node
    let gd_params: Vec<_> = gd
        .children()
        .filter(|c| c.has_tag_name("hkparam"))
        .collect();
    let vs_params: Vec<_> = vs
        .children()
        .filter(|c| c.has_tag_name("hkparam"))
        .collect();
    let sd_params: Vec<_> = sd
        .children()
        .filter(|c| c.has_tag_name("hkparam"))
        .collect();

    // hkbBehaviorGraphStringData:
    //   [0] eventNames, [1] attributeNames, [2] variableNames, [3] characterPropertyNames
    let event_names: Vec<String> = sd_params
        .first()
        .map(|p| {
            p.children()
                .filter(|c| c.has_tag_name("hkcstring"))
                .map(|c| c.text().unwrap_or("").trim().to_string())
                .collect()
        })
        .unwrap_or_default();

    let var_names: Vec<String> = sd_params
        .get(2)
        .map(|p| {
            p.children()
                .filter(|c| c.has_tag_name("hkcstring"))
                .map(|c| c.text().unwrap_or("").trim().to_string())
                .collect()
        })
        .unwrap_or_default();

    let prop_names: Vec<String> = sd_params
        .get(3)
        .map(|p| {
            p.children()
                .filter(|c| c.has_tag_name("hkcstring"))
                .map(|c| c.text().unwrap_or("").trim().to_string())
                .collect()
        })
        .unwrap_or_default();

    // hkbBehaviorGraphData:
    //   [0] attributeDefaults, [1] variableInfos, [2] characterPropertyInfos,
    //   [3] eventInfos, [4] variableBounds, [5] variableInitialValues, [6] stringData

    // Events
    let event_infos: Vec<_> = gd_params
        .get(3)
        .map(|p| {
            p.children()
                .filter(|c| c.has_tag_name("hkobject"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let num_events = event_names.len().max(event_infos.len());
    let mut events: Vec<serde_json::Value> = Vec::with_capacity(num_events);
    for i in 0..num_events {
        let name = event_names.get(i).cloned().unwrap_or_default();
        let flags: i64 = event_infos
            .get(i)
            .and_then(|info| {
                info.children()
                    .find(|c| c.has_tag_name("hkparam"))
                    .and_then(|p| p.text())
                    .map(|t| if t.trim() == "FLAG_SYNC_POINT" { 1 } else { 0 })
            })
            .unwrap_or(0);
        events.push(serde_json::json!({
            "eventID": i,
            "eventName": name,
            "eventFlags": flags,
        }));
    }

    // Variables
    let var_infos: Vec<_> = gd_params
        .get(1)
        .map(|p| {
            p.children()
                .filter(|c| c.has_tag_name("hkobject"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // hkbVariableValueSet: [0] wordVariableValues, [1] quadVariableValues
    let word_values: Vec<_> = vs_params
        .first()
        .map(|p| {
            p.children()
                .filter(|c| c.has_tag_name("hkobject"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // Quad values: flat whitespace-separated floats in [1]
    let quad_text = vs_params
        .get(1)
        .and_then(|p| p.text())
        .unwrap_or("")
        .replace('\r', " ")
        .replace('\n', " ");
    let quad_vals: Vec<&str> = quad_text.split_whitespace().collect();
    let mut quad_counter = 0usize;

    // Variable bounds: [4]
    let bounds_list: Vec<_> = gd_params
        .get(4)
        .map(|p| {
            p.children()
                .filter(|c| c.has_tag_name("hkobject"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let variable_type_map = |s: &str| -> i64 {
        match s {
            "VARIABLE_TYPE_BOOL" => 0,
            "VARIABLE_TYPE_INT8" => 1,
            "VARIABLE_TYPE_INT16" => 2,
            "VARIABLE_TYPE_INT32" => 3,
            "VARIABLE_TYPE_REAL" => 4,
            "VARIABLE_TYPE_POINTER" => 5,
            "VARIABLE_TYPE_VECTOR4" => 6,
            "VARIABLE_TYPE_QUATERNION" => 7,
            _ => 0,
        }
    };

    let num_vars = var_names.len().max(var_infos.len());
    let mut variables: Vec<serde_json::Value> = Vec::with_capacity(num_vars);

    for i in 0..num_vars {
        let name = var_names.get(i).cloned().unwrap_or_default();

        // Type from variableInfos[i] → second hkparam child is "type"
        let var_type_str = var_infos
            .get(i)
            .and_then(|info| {
                info.children()
                    .filter(|c| c.has_tag_name("hkparam"))
                    .nth(1)
                    .and_then(|p| p.text())
                    .map(|t| t.trim().to_string())
            })
            .unwrap_or_default();
        let var_type = variable_type_map(&var_type_str);

        // Value from wordVariableValues[i] → single hkparam child "value"
        let raw_val: String = word_values
            .get(i)
            .and_then(|obj| {
                obj.children()
                    .find(|c| c.has_tag_name("hkparam"))
                    .and_then(|p| p.text())
                    .map(|t| t.trim().to_string())
            })
            .unwrap_or_else(|| "0".to_string());

        // For REAL type, reinterpret u32 bits as f32
        let value = if var_type == 4 {
            raw_val
                .parse::<i64>()
                .map(u32_to_f32)
                .map(|f| f.to_string())
                .unwrap_or(raw_val)
        } else {
            raw_val
        };

        // Quad values for Vector4/Quaternion
        let quad_val = if var_type == 6 || var_type == 7 {
            let mut parts = Vec::with_capacity(4);
            for _ in 0..4 {
                parts.push(
                    quad_vals
                        .get(quad_counter)
                        .copied()
                        .unwrap_or("0.0")
                        .to_string(),
                );
                quad_counter += 1;
            }
            parts.join(" ")
        } else {
            "(0.0 0.0 0.0 0.0)".to_string()
        };

        // Bounds from variableBounds[i]
        let (min_val, max_val) = bounds_list
            .get(i)
            .and_then(|b| {
                let bc: Vec<_> = b
                    .children()
                    .filter(|c| c.has_tag_name("hkobject"))
                    .collect();
                let min_inner: Vec<_> = bc
                    .first()?
                    .children()
                    .filter(|c| c.has_tag_name("hkobject"))
                    .collect();
                let max_inner: Vec<_> = bc
                    .get(1)?
                    .children()
                    .filter(|c| c.has_tag_name("hkobject"))
                    .collect();
                let min_raw = min_inner
                    .first()
                    .and_then(|o| {
                        o.children()
                            .find(|c| c.has_tag_name("hkparam"))
                            .and_then(|p| p.text())
                            .map(|t| t.trim().to_string())
                    })
                    .unwrap_or_else(|| "0".to_string());
                let max_raw = max_inner
                    .first()
                    .and_then(|o| {
                        o.children()
                            .find(|c| c.has_tag_name("hkparam"))
                            .and_then(|p| p.text())
                            .map(|t| t.trim().to_string())
                    })
                    .unwrap_or_else(|| "0".to_string());
                let minv = min_raw
                    .parse::<i64>()
                    .map(u32_to_f32)
                    .map(|f| f.to_string())
                    .unwrap_or(min_raw);
                let maxv = max_raw
                    .parse::<i64>()
                    .map(u32_to_f32)
                    .map(|f| f.to_string())
                    .unwrap_or(max_raw);
                Some((minv, maxv))
            })
            .unwrap_or_else(|| ("0".to_string(), "0".to_string()));

        variables.push(serde_json::json!({
            "variableID": i,
            "variableName": name,
            "variableType": var_type,
            "variableValue": value,
            "variableMinValue": min_val,
            "variableMaxValue": max_val,
            "variableQuadValues": quad_val,
        }));
    }

    // Properties
    let prop_infos: Vec<_> = gd_params
        .get(2)
        .map(|p| {
            p.children()
                .filter(|c| c.has_tag_name("hkobject"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let num_props = prop_names.len().max(prop_infos.len());
    let mut properties: Vec<serde_json::Value> = Vec::with_capacity(num_props);

    for i in 0..num_props {
        let name = prop_names.get(i).cloned().unwrap_or_default();
        let prop_type_str = prop_infos
            .get(i)
            .and_then(|info| {
                info.children()
                    .filter(|c| c.has_tag_name("hkparam"))
                    .nth(1)
                    .and_then(|p| p.text())
                    .map(|t| t.trim().to_string())
            })
            .unwrap_or_default();
        let prop_type = variable_type_map(&prop_type_str);
        properties.push(serde_json::json!({
            "propertiesID": i,
            "propertiesName": name,
            "propertiesType": prop_type,
        }));
    }

    (events, variables, properties)
}

// ---------------------------------------------------------------------------
// Node property parsers — one per type_id
// ---------------------------------------------------------------------------

fn parse_event_property(
    obj: roxmltree::Node<'_, '_>,
    param_name: &str,
    payload_map: &std::collections::HashMap<String, i64>,
) -> (i64, i64) {
    let Some(ep) = child_param(obj, param_name) else {
        return (-1, -1);
    };
    // Child is a single hkobject, whose hkparam children are: id, payload
    let inner_obj = ep.children().find(|c| c.has_tag_name("hkobject"));
    let Some(inner) = inner_obj else {
        return (-1, -1);
    };
    let params: Vec<_> = inner
        .children()
        .filter(|c| c.has_tag_name("hkparam"))
        .collect();
    let event_id = params
        .first()
        .and_then(|p| p.text())
        .and_then(|t| t.trim().parse::<i64>().ok())
        .unwrap_or(-1);
    let payload_ref = params
        .get(1)
        .and_then(|p| p.text())
        .unwrap_or("")
        .trim()
        .to_string();
    let payload_id = resolve_payload(&payload_ref, payload_map);
    (event_id, payload_id)
}

/// Parse hkbStateMachineTransitionInfoArray transitions element.
fn parse_transition_array(
    obj: roxmltree::Node<'_, '_>,
    transition_map: &std::collections::HashMap<String, usize>,
    _payload_map: &std::collections::HashMap<String, i64>,
) -> Vec<serde_json::Value> {
    let Some(transitions_param) = child_param(obj, "transitions") else {
        return Vec::new();
    };
    let mut arr = Vec::new();
    for t_obj in transitions_param
        .children()
        .filter(|c| c.has_tag_name("hkobject"))
    {
        // triggerInterval (hkparam 0 → nested hkobject with enterEventId, exitEventId, enterTime, exitTime)
        let (ti_enter_ev, ti_exit_ev, ti_enter_time, ti_exit_time) =
            child_param(t_obj, "triggerInterval")
                .and_then(|p| p.children().find(|c| c.has_tag_name("hkobject")))
                .map(|inner| {
                    (
                        param_int(inner, "enterEventId", -1),
                        param_int(inner, "exitEventId", -1),
                        param_text(inner, "enterTime"),
                        param_text(inner, "exitTime"),
                    )
                })
                .unwrap_or((-1, -1, "0.000000".to_string(), "0.000000".to_string()));

        let (ii_enter_ev, ii_exit_ev, ii_enter_time, ii_exit_time) =
            child_param(t_obj, "initiateInterval")
                .and_then(|p| p.children().find(|c| c.has_tag_name("hkobject")))
                .map(|inner| {
                    (
                        param_int(inner, "enterEventId", -1),
                        param_int(inner, "exitEventId", -1),
                        param_text(inner, "enterTime"),
                        param_text(inner, "exitTime"),
                    )
                })
                .unwrap_or((-1, -1, "0.000000".to_string(), "0.000000".to_string()));

        let t_ref = param_text(t_obj, "transition");
        let transition_idx: i64 = if !is_null(&t_ref) {
            transition_map
                .get(t_ref.trim())
                .map(|&i| i as i64 - 1)
                .unwrap_or(0)
        } else {
            0
        };

        let event_id = param_int(t_obj, "eventId", -1);
        let to_state_id = param_int(t_obj, "toStateId", 0);
        let from_nested = param_int(t_obj, "fromNestedStateId", 0);
        let to_nested = param_int(t_obj, "toNestedStateId", 0);
        let priority = param_int(t_obj, "priority", 0);
        let flags_str = param_text(t_obj, "flags");
        let flags = parse_transition_flags(&flags_str);

        arr.push(serde_json::json!({
            "eventId": event_id,
            "toStateId": to_state_id,
            "fromNestedStateId": from_nested,
            "toNestedStateId": to_nested,
            "priority": priority,
            "flags": flags,
            "transition": transition_idx,
            "triggerInterval": {
                "enterEventId": ti_enter_ev,
                "exitEventId": ti_exit_ev,
                "enterTime": ti_enter_time,
                "exitTime": ti_exit_time,
            },
            "initiateInterval": {
                "enterEventId": ii_enter_ev,
                "exitEventId": ii_exit_ev,
                "enterTime": ii_enter_time,
                "exitTime": ii_exit_time,
            },
        }));
    }
    arr
}

/// Build a node data dict from an hkobject with the given type_id.
/// Returns `None` if the type_id is unknown or metadata_only.
/// Modifies `connections` and optionally `unhandled`.
#[allow(clippy::too_many_arguments)]
fn build_node(
    obj: roxmltree::Node<'_, '_>,
    type_id: i64,
    node_id: i64,
    connections: &mut Vec<serde_json::Value>,
    transition_map: &std::collections::HashMap<String, usize>,
    payload_map: &std::collections::HashMap<String, i64>,
) -> serde_json::Value {
    use serde_json::json;

    let mut node = json!({
        "nodeID": node_id,
        "nodeTypeID": type_id,
        "nodeColorID": 0,
        "nodeName": "",
    });

    match type_id {
        0 => {
            // hkRootLevelContainer
            // namedVariants hkparam → first hkobject → [name, className, variant]
            if let Some(nv_param) = child_param(obj, "namedVariants") {
                if let Some(inner_obj) = nv_param.children().find(|c| c.has_tag_name("hkobject")) {
                    let params: Vec<_> = inner_obj
                        .children()
                        .filter(|c| c.has_tag_name("hkparam"))
                        .collect();
                    if let Some(cn) = params.get(1).and_then(|p| p.text()) {
                        node["className"] = json!(cn.trim());
                    }
                    if let Some(vref) = params.get(2).and_then(|p| p.text()) {
                        push_conn(connections, 0, node_id, vref.trim());
                    }
                }
            }
        }
        1 => {
            // hkbBehaviorGraph
            node["nodeName"] = json!(param_text(obj, "name"));
            node["className"] = json!("hkbBehaviorGraph");
            push_conn(connections, 0, node_id, &param_text(obj, "rootGenerator"));
        }
        5 => {
            // hkbStateMachine
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));

            // eventToSendWhenStateOrTransitionChanges
            if let Some(ev_param) = child_param(obj, "eventToSendWhenStateOrTransitionChanges") {
                if let Some(ev_obj) = ev_param.children().find(|c| c.has_tag_name("hkobject")) {
                    node["eventId"] = json!(param_int(ev_obj, "id", -1));
                    let pref = param_text(ev_obj, "payload");
                    node["payload"] = json!(resolve_payload(&pref, payload_map));
                }
            }

            node["startStateId"] = json!(param_int(obj, "startStateId", 0));
            node["randomTransitionEventId"] = json!(param_int(obj, "randomTransitionEventId", -1));
            node["transitionToNextHigherStateEventId"] =
                json!(param_int(obj, "transitionToNextHigherStateEventId", -1));
            node["transitionToNextLowerStateEventId"] =
                json!(param_int(obj, "transitionToNextLowerStateEventId", -1));
            node["syncVariableIndex"] = json!(param_int(obj, "syncVariableIndex", -1));
            node["wrapAroundStateId"] = json!(param_bool(obj, "wrapAroundStateId", false));
            node["startStateMode"] = json!(or_default(
                param_text(obj, "startStateMode"),
                "START_STATE_MODE_DEFAULT"
            ));
            node["selfTransitionMode"] = json!(or_default(
                param_text(obj, "selfTransitionMode"),
                "SELF_TRANSITION_MODE_NO_TRANSITION"
            ));

            // states (multi-connect port 1)
            if let Some(states_param) = child_param(obj, "states") {
                let refs_text = states_param
                    .children()
                    .filter(|c| c.has_tag_name("hkobject"))
                    .filter_map(|o| o.text())
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join(" ");
                push_multi_conn(connections, 1, node_id, &refs_text);
            }

            // wildcardTransitions (port 2)
            push_conn(
                connections,
                2,
                node_id,
                &param_text(obj, "wildcardTransitions"),
            );
        }
        6 => {
            // hkbStateMachineStateInfo
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            push_conn(
                connections,
                1,
                node_id,
                &param_text(obj, "enterNotifyEvents"),
            );
            push_conn(
                connections,
                2,
                node_id,
                &param_text(obj, "exitNotifyEvents"),
            );
            push_conn(connections, 3, node_id, &param_text(obj, "transitions"));

            // generator (multi-connect port 4)
            let gen_ref = param_text(obj, "generator");
            push_multi_conn(connections, 4, node_id, &gen_ref);

            node["nodeName"] = json!(param_text(obj, "name"));
            node["stateId"] = json!(param_int(obj, "stateId", 0));
            node["probability"] = json!(or_default(param_text(obj, "probability"), "1.000000"));
            node["enable"] = json!(param_bool(obj, "enable", true));
        }
        7 => {
            // hkbStateMachineTransitionInfoArray
            let arr = parse_transition_array(obj, transition_map, payload_map);
            node["transitionArray"] = json!(arr);
        }
        8 => {
            // hkbStateMachineEventPropertyArray
            if let Some(events_param) = child_param(obj, "events") {
                let mut arr = Vec::new();
                for e_obj in events_param
                    .children()
                    .filter(|c| c.has_tag_name("hkobject"))
                {
                    let params: Vec<_> = e_obj
                        .children()
                        .filter(|c| c.has_tag_name("hkparam"))
                        .collect();
                    let event_id = params
                        .first()
                        .and_then(|p| p.text())
                        .and_then(|t| t.trim().parse::<i64>().ok())
                        .unwrap_or(-1);
                    let p_ref = params
                        .get(1)
                        .and_then(|p| p.text())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    arr.push(json!({
                        "eventID": event_id,
                        "payloadID": resolve_payload(&p_ref, payload_map),
                    }));
                }
                node["eventsArray"] = json!(arr);
            }
        }
        9 => {
            // hkbModifierGenerator
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            push_conn(connections, 1, node_id, &param_text(obj, "modifier"));
            push_conn(connections, 2, node_id, &param_text(obj, "generator"));
        }
        10 => {
            // hkbModifierList
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            if let Some(mods) = child_param(obj, "modifiers") {
                let refs: String = mods
                    .children()
                    .filter(|c| c.has_tag_name("hkobject"))
                    .filter_map(|o| o.text())
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join(" ");
                push_multi_conn(connections, 1, node_id, &refs);
            }
        }
        11 => {
            // hkbGetUpModifier
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            node["groundNormal"] = json!(param_text(obj, "groundNormal"));
            node["duration"] = json!(param_text(obj, "duration"));
            node["alignWithGroundDuration"] = json!(param_text(obj, "alignWithGroundDuration"));
            node["rootBoneIndex"] = json!(param_int(obj, "rootBoneIndex", -1));
            node["otherBoneIndex"] = json!(param_int(obj, "otherBoneIndex", -1));
            node["anotherBoneIndex"] = json!(param_int(obj, "anotherBoneIndex", -1));
        }
        12 => {
            // hkbKeyframeBonesModifier
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            push_conn(
                connections,
                1,
                node_id,
                &param_text(obj, "keyframedBonesList"),
            );
        }
        13 => {
            // hkbBoneIndexArray
            let content = param_text(obj, "boneIndices");
            let indices: Vec<i64> = content
                .split_whitespace()
                .filter_map(|s| s.parse::<i64>().ok())
                .collect();
            node["boneIndices"] = json!(indices);
        }
        14 => {
            // hkbBoneWeightArray
            let content = param_text(obj, "boneWeights");
            let weights: Vec<String> = content
                .replace('\r', " ")
                .replace('\n', " ")
                .split_whitespace()
                .map(str::to_string)
                .collect();
            node["boneWeights"] = json!(weights);
        }
        15 => {
            // hkbRigidBodyRagdollControlsModifier
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            // keyframingControls nested
            if let Some(ctrl) = child_param(obj, "keyframingControls") {
                if let Some(kf_obj) = ctrl.children().find(|c| c.has_tag_name("hkobject")) {
                    if let Some(kf_param) = kf_obj.children().find(|c| c.has_tag_name("hkparam")) {
                        if let Some(inner_obj) =
                            kf_param.children().find(|c| c.has_tag_name("hkobject"))
                        {
                            let props = [
                                "hierarchyGain",
                                "velocityDamping",
                                "accelerationGain",
                                "velocityGain",
                                "positionGain",
                                "positionMaxLinearVelocity",
                                "positionMaxAngularVelocity",
                                "snapGain",
                                "snapMaxLinearVelocity",
                                "snapMaxAngularVelocity",
                                "snapMaxLinearDistance",
                                "snapMaxAngularDistance",
                            ];
                            for p in &props {
                                node[p] = json!(param_text(inner_obj, p));
                            }
                        }
                        let kf_params: Vec<_> = kf_obj
                            .children()
                            .filter(|c| c.has_tag_name("hkparam"))
                            .collect();
                        if let Some(p) = kf_params.get(1) {
                            node["durationToBlend"] = json!(p.text().unwrap_or("0").trim());
                        }
                    }
                }
            }
            push_conn(connections, 1, node_id, &param_text(obj, "bones"));
            node["animationBlendFraction"] = json!(or_default(
                param_text(obj, "animationBlendFraction"),
                "0.000000"
            ));
        }
        16 => {
            // BSIsActiveModifier
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            // bIsActive0..4, bInvertActive0..4 — stored as flat array of 10 bools
            let arr = [
                param_bool(obj, "bIsActive0", false),
                param_bool(obj, "bIsActive1", false),
                param_bool(obj, "bIsActive2", false),
                param_bool(obj, "bIsActive3", false),
                param_bool(obj, "bIsActive4", false),
                param_bool(obj, "bInvertActive0", false),
                param_bool(obj, "bInvertActive1", false),
                param_bool(obj, "bInvertActive2", false),
                param_bool(obj, "bInvertActive3", false),
                param_bool(obj, "bInvertActive4", false),
            ];
            node["bIsActiveArray"] = json!(arr);
        }
        17 => {
            // hkbVariableBindingSet
            if let Some(bindings_param) = child_param(obj, "bindings") {
                let mut arr = Vec::new();
                for b_obj in bindings_param
                    .children()
                    .filter(|c| c.has_tag_name("hkobject"))
                {
                    arr.push(json!({
                        "memberPath": param_text(b_obj, "memberPath"),
                        "variableIndex": param_int(b_obj, "variableIndex", 0),
                        "bindingType": param_text(b_obj, "bindingType"),
                    }));
                }
                node["bindingArray"] = json!(arr);
            }
            node["indexOfBindingToEnable"] = json!(param_int(obj, "indexOfBindingToEnable", -1));
        }
        18 => {
            // hkbManualSelectorGenerator
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            if let Some(gens) = child_param(obj, "generators") {
                let refs: String = gens
                    .children()
                    .filter(|c| c.has_tag_name("hkobject"))
                    .filter_map(|o| o.text())
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join(" ");
                push_multi_conn(connections, 1, node_id, &refs);
            }
            node["selectedGeneratorIndex"] = json!(param_int(obj, "selectedGeneratorIndex", 0));
            push_conn(connections, 2, node_id, &param_text(obj, "indexSelector"));
            node["selectedIndexCanChangeAfterActivate"] = json!(param_bool(
                obj,
                "selectedIndexCanChangeAfterActivate",
                false
            ));
            let t_ref = param_text(obj, "generatorChangedTransitionEffect");
            let te = if !is_null(&t_ref) {
                transition_map
                    .get(t_ref.trim())
                    .map(|&i| i as i64)
                    .unwrap_or(-1)
            } else {
                -1
            };
            node["generatorChangedTransitionEffect"] = json!(te);
        }
        19 => {
            // BSModifyOnceModifier
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            push_conn(
                connections,
                1,
                node_id,
                &param_text(obj, "pOnActivateModifier"),
            );
            push_conn(
                connections,
                2,
                node_id,
                &param_text(obj, "pOnDeactivateModifier"),
            );
        }
        20 => {
            // hkbEvaluateExpressionModifier
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            push_conn(connections, 1, node_id, &param_text(obj, "expressions"));
        }
        21 => {
            // hkbExpressionDataArray
            if let Some(expr_param) = child_param(obj, "expressionsData") {
                let mut arr = Vec::new();
                for e_obj in expr_param.children().filter(|c| c.has_tag_name("hkobject")) {
                    arr.push(json!({
                        "expression": param_text(e_obj, "expression"),
                        "assignmentVariableIndex": param_int(e_obj, "assignmentVariableIndex", -1),
                        "assignmentEventIndex": param_int(e_obj, "assignmentEventIndex", -1),
                    }));
                }
                node["expressionArray"] = json!(arr);
            }
        }
        22 => {
            // hkbPoseMatchingGenerator
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["blendParameter"] = json!(param_text(obj, "blendParameter"));
            if let Some(gens) = child_param(obj, "generators") {
                let refs: String = gens
                    .children()
                    .filter(|c| c.has_tag_name("hkobject"))
                    .filter_map(|o| o.text())
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join(" ");
                push_multi_conn(connections, 1, node_id, &refs);
            }
            node["blendSpeed"] = json!(or_default(param_text(obj, "blendSpeed"), "1.000000"));
            node["minSpeedToSwitch"] =
                json!(or_default(param_text(obj, "minSpeedToSwitch"), "0.200000"));
            node["startPlayingEventId"] = json!(param_int(obj, "startPlayingEventId", -1));
            node["startMatchingEventId"] = json!(param_int(obj, "startMatchingEventId", -1));
            node["rootBoneIndex"] = json!(param_int(obj, "rootBoneIndex", -1));
            node["otherBoneIndex"] = json!(param_int(obj, "otherBoneIndex", -1));
            node["anotherBoneIndex"] = json!(param_int(obj, "anotherBoneIndex", -1));
            node["pelvisIndex"] = json!(param_int(obj, "pelvisIndex", -1));
        }
        23 => {
            // hkbBlenderGenerator
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["referencePoseWeightThreshold"] = json!(or_default(
                param_text(obj, "referencePoseWeightThreshold"),
                "0.0"
            ));
            node["blendParameter"] = json!(param_text(obj, "blendParameter"));
            node["minCyclicBlendParameter"] = json!(or_default(
                param_text(obj, "minCyclicBlendParameter"),
                "0.000000"
            ));
            node["maxCyclicBlendParameter"] = json!(or_default(
                param_text(obj, "maxCyclicBlendParameter"),
                "1.000000"
            ));
            node["indexOfSyncMasterChild"] = json!(param_int(obj, "indexOfSyncMasterChild", 65535));
            node["flagsIndex"] = json!(param_int(obj, "flagsIndex", 0));
            node["subtractLastChild"] = json!(param_bool(obj, "subtractLastChild", false));
            if let Some(ch_param) = child_param(obj, "children") {
                let refs: String = ch_param
                    .children()
                    .filter(|c| c.has_tag_name("hkobject"))
                    .filter_map(|o| o.text())
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join(" ");
                push_multi_conn(connections, 1, node_id, &refs);
            }
        }
        24 => {
            // hkbBlenderGeneratorChild
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            push_conn(connections, 1, node_id, &param_text(obj, "generator"));
            push_conn(connections, 2, node_id, &param_text(obj, "boneWeights"));
            node["weight"] = json!(or_default(param_text(obj, "weight"), "1.000000"));
            node["worldFromModelWeight"] = json!(or_default(
                param_text(obj, "worldFromModelWeight"),
                "1.000000"
            ));
        }
        25 => {
            // hkbClipGenerator
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["animationName"] = json!(param_text(obj, "animationName"));
            push_conn(connections, 1, node_id, &param_text(obj, "triggers"));
            node["cropStartAmountLocalTime"] = json!(param_text(obj, "cropStartAmountLocalTime"));
            node["cropEndAmountLocalTime"] = json!(param_text(obj, "cropEndAmountLocalTime"));
            node["startTime"] = json!(param_text(obj, "startTime"));
            node["playbackSpeed"] = json!(or_default(param_text(obj, "playbackSpeed"), "1.000000"));
            node["enforcedDuration"] = json!(param_text(obj, "enforcedDuration"));
            node["userControlledTimeFraction"] =
                json!(param_text(obj, "userControlledTimeFraction"));
            let mode_str = param_text(obj, "mode");
            let mode: i64 = if mode_str == "MODE_LOOPING" {
                1
            } else if mode_str == "MODE_USER_CONTROLLED" {
                2
            } else {
                // Also handle numeric values (e.g. "1025" means some looping flag)
                if mode_str.parse::<i64>().map(|v| v & 1).unwrap_or(0) == 1 {
                    1
                } else {
                    0
                }
            };
            node["mode"] = json!(mode);
            node["flagsIndex"] = json!(param_int(obj, "flags", 0));
        }
        26 => {
            // hkbClipTriggerArray
            if let Some(trigs) = child_param(obj, "triggers") {
                let mut arr = Vec::new();
                for t_obj in trigs.children().filter(|c| c.has_tag_name("hkobject")) {
                    let local_time = or_default(param_text(t_obj, "localTime"), "0");
                    let (event_id, payload_id) = parse_event_property(t_obj, "event", payload_map);
                    let rel = param_bool(t_obj, "relativeToEndOfClip", false);
                    let acyclic = param_bool(t_obj, "acyclic", false);
                    let is_ann = param_bool(t_obj, "isAnnotation", false);
                    arr.push(json!({
                        "localTime": local_time,
                        "eventID": event_id,
                        "payloadID": payload_id,
                        "relativeToEndOfClip": rel,
                        "acyclic": acyclic,
                        "isAnnotation": is_ann,
                    }));
                }
                node["triggersArray"] = json!(arr);
            }
        }
        28 => {
            // hkbEventDrivenModifier
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            push_conn(connections, 1, node_id, &param_text(obj, "modifier"));
            node["activateEventId"] = json!(param_int(obj, "activateEventId", -1));
            node["deactivateEventId"] = json!(param_int(obj, "deactivateEventId", -1));
            node["activeByDefault"] = json!(param_bool(obj, "activeByDefault", false));
        }
        29 => {
            // hkbPoweredRagdollControlsModifier
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            if let Some(ctrl) = child_param(obj, "controlData") {
                if let Some(inner_obj) = ctrl.children().find(|c| c.has_tag_name("hkobject")) {
                    for p in &[
                        "maxForce",
                        "tau",
                        "damping",
                        "proportionalRecoveryVelocity",
                        "constantRecoveryVelocity",
                    ] {
                        node[p] = json!(param_text(inner_obj, p));
                    }
                }
            }
            push_conn(connections, 1, node_id, &param_text(obj, "bones"));
            if let Some(pm) = child_param(obj, "poseMatchingBones") {
                if let Some(pm_obj) = pm.children().find(|c| c.has_tag_name("hkobject")) {
                    node["poseMatchingBone0"] = json!(param_int(pm_obj, "poseMatchingBone0", -1));
                    node["poseMatchingBone1"] = json!(param_int(pm_obj, "poseMatchingBone1", -1));
                    node["poseMatchingBone2"] = json!(param_int(pm_obj, "poseMatchingBone2", -1));
                    let mode_str = param_text(pm_obj, "mode");
                    let mode: i64 = match mode_str.as_str() {
                        "WORLD_FROM_MODEL_MODE_RAGDOLL" => 2,
                        "WORLD_FROM_MODEL_MODE_NONE" => 1,
                        _ => 0,
                    };
                    node["mode"] = json!(mode);
                }
            }
            push_conn(connections, 1, node_id, &param_text(obj, "boneWeights"));
            node["animationBlendFraction"] = json!(or_default(
                param_text(obj, "animationBlendFraction"),
                "0.000000"
            ));
        }
        30 => {
            // hkbTimerModifier
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            node["alarmTimeSeconds"] =
                json!(or_default(param_text(obj, "alarmTimeSeconds"), "0.000000"));
            let (eid, pid) = parse_event_property(obj, "alarmEvent", payload_map);
            node["eventId"] = json!(eid);
            node["payload"] = json!(pid);
        }
        32 => {
            // BSGetTimeStepModifier
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            node["timeStep"] = json!(or_default(param_text(obj, "timeStep"), "0.000000"));
        }
        33 => {
            // hkbTwistModifier
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            node["axisOfRotation"] = json!(param_text(obj, "axisOfRotation"));
            node["twistAngle"] = json!(param_text(obj, "twistAngle"));
            node["startBoneIndex"] = json!(param_int(obj, "startBoneIndex", -1));
            node["endBoneIndex"] = json!(param_int(obj, "endBoneIndex", -1));
            let method = param_text(obj, "setAngleMethod");
            node["setAngleMethod"] = json!(if method == "RAMPED" { 1i64 } else { 0i64 });
            let coords = param_text(obj, "rotationAxisCoordinates");
            let coords_int: i64 = match coords.as_str() {
                "ROTATION_AXIS_IN_MODEL_COORDINATES" => 1,
                "ROTATION_AXIS_IN_PARENT_COORDINATES" => 2,
                _ => 0,
            };
            node["rotationAxisCoordinates"] = json!(coords_int);
            node["isAdditive"] = json!(param_bool(obj, "isAdditive", false));
        }
        34 => {
            // BSInterpValueModifier
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            node["source"] = json!(param_text(obj, "source"));
            node["target"] = json!(param_text(obj, "target"));
            node["result"] = json!(param_text(obj, "result"));
            node["gain"] = json!(param_text(obj, "gain"));
        }
        35 => {
            // hkbEventsFromRangeModifier
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            node["inputValue"] = json!(param_text(obj, "inputValue"));
            node["lowerBound"] = json!(param_text(obj, "lowerBound"));
            push_conn(connections, 1, node_id, &param_text(obj, "eventRanges"));
        }
        36 => {
            // hkbEventRangeDataArray
            if let Some(range_param) = child_param(obj, "eventData") {
                let mut arr = Vec::new();
                for r_obj in range_param
                    .children()
                    .filter(|c| c.has_tag_name("hkobject"))
                {
                    let upper = or_default(param_text(r_obj, "upperBound"), "0");
                    let (eid, pid) = parse_event_property(r_obj, "event", payload_map);
                    let em_str = param_text(r_obj, "eventMode");
                    let event_mode: i64 = if em_str == "EVENT_MODE_SEND_WHEN_IN_RANGE" {
                        1
                    } else {
                        0
                    };
                    arr.push(json!({
                        "upperBound": upper,
                        "eventID": eid,
                        "payloadID": pid,
                        "eventMode": event_mode,
                    }));
                }
                node["rangeArray"] = json!(arr);
            }
        }
        37 => {
            // BSBehaviorGraphSwapGenerator
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            push_conn(
                connections,
                1,
                node_id,
                &param_text(obj, "pDefaultGenerator"),
            );
        }
        38 => {
            // BSRagdollContactListenerModifier
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            let (eid, pid) = parse_event_property(obj, "contactEvent", payload_map);
            node["eventId"] = json!(eid);
            node["payload"] = json!(pid);
            push_conn(connections, 1, node_id, &param_text(obj, "bones"));
        }
        39 => {
            // BSCyclicBlendTransitionGenerator
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            push_conn(
                connections,
                1,
                node_id,
                &param_text(obj, "pBlenderGenerator"),
            );
            // 4 cyclic events
            let cyclic_events = [
                (
                    "EventToFreezeBlendValueID",
                    "EventToFreezeBlendValuePayload",
                    "EventToFreezeBlendValue",
                ),
                (
                    "EventToCrossBlendID",
                    "EventToCrossBlendPayload",
                    "EventToCrossBlend",
                ),
                (
                    "TransitionOutEventID",
                    "TransitionOutEventPayload",
                    "TransitionOutEvent",
                ),
                (
                    "TransitionInEventID",
                    "TransitionInEventPayload",
                    "TransitionInEvent",
                ),
            ];
            for (id_key, payload_key, param_name) in &cyclic_events {
                let (eid, pid) = parse_event_property(obj, param_name, payload_map);
                node[*id_key] = json!(eid);
                node[*payload_key] = json!(pid);
            }
            node["fTransitionDuration"] = json!(or_default(
                param_text(obj, "fTransitionDuration"),
                "0.000000"
            ));
        }
        40 => {
            // BGSGamebryoSequenceGenerator
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["pSequence"] = json!(param_text(obj, "pSequence"));
            node["eBlendModeFunction"] = json!(param_text(obj, "eBlendModeFunction"));
            node["fPercent"] = json!(param_text(obj, "fPercent"));
            node["eUseTimePercentage"] = json!(param_text(obj, "eUseTimePercentage"));
            node["fTimePercent"] = json!(param_text(obj, "fTimePercent"));
        }
        42 => {
            // hkbBehaviorReferenceGenerator
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["behaviorName"] = json!(param_text(obj, "behaviorName"));
        }
        43 => {
            // BSAssignVariablesModifier — 20 float var/val pairs + 4 int var/val pairs
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            // Collect all hkparam children for positional access (mirrors Python)
            let params: Vec<_> = obj
                .children()
                .filter(|c| c.has_tag_name("hkparam"))
                .collect();
            let p = |i: usize| -> String {
                params
                    .get(i)
                    .and_then(|p| p.text())
                    .unwrap_or("")
                    .trim()
                    .to_string()
            };
            let fv: Vec<String> = (4..44usize).step_by(2).map(|i| p(i)).collect();
            let fval: Vec<String> = (5..44usize).step_by(2).map(|i| p(i)).collect();
            let iv: Vec<i64> = (44..52usize)
                .step_by(2)
                .map(|i| p(i).parse::<i64>().unwrap_or(0))
                .collect();
            let ival: Vec<i64> = (45..52usize)
                .step_by(2)
                .map(|i| p(i).parse::<i64>().unwrap_or(0))
                .collect();
            node["floatVariable"] = json!(fv);
            node["floatValue"] = json!(fval);
            node["intVariable"] = json!(iv);
            node["intValue"] = json!(ival);
        }
        44 => {
            // DynamicAnimationTaggingGenerator
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            push_conn(
                connections,
                1,
                node_id,
                &param_text(obj, "pDefaultGenerator"),
            );
        }
        45 => {
            // BSTimerModifier
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            node["alarmTimeSeconds"] =
                json!(or_default(param_text(obj, "alarmTimeSeconds"), "0.000000"));
            let (eid, pid) = parse_event_property(obj, "alarmEvent", payload_map);
            node["eventId"] = json!(eid);
            node["payload"] = json!(pid);
            node["resetAlarm"] = json!(param_bool(obj, "resetAlarm", false));
        }
        46 => {
            // BSiStateTaggingGenerator
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            push_conn(
                connections,
                1,
                node_id,
                &param_text(obj, "pDefaultGenerator"),
            );
            node["iStateToSetAs"] = json!(param_int(obj, "iStateToSetAs", 0));
            node["iPriority"] = json!(param_int(obj, "iPriority", 0));
        }
        47 => {
            // hkbGeneratorTransitionEffect
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            let stm = param_text(obj, "selfTransitionMode");
            node["selfTransitionMode"] = json!(self_transition_mode_int(&stm));
            let em = param_text(obj, "eventMode");
            node["eventMode"] = json!(event_mode_int(&em));
            push_conn(connections, 1, node_id, &param_text(obj, "toGenerator"));
            node["blendInDuration"] =
                json!(or_default(param_text(obj, "blendInDuration"), "0.000000"));
            node["blendOutDuration"] =
                json!(or_default(param_text(obj, "blendOutDuration"), "0.000000"));
            node["syncToGeneratorStartTime"] =
                json!(param_bool(obj, "syncToGeneratorStartTime", false));
        }
        48 => {
            // hkbReferencePoseGenerator
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
        }
        49 => {
            // hkbDampingModifier
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            node["enable"] = json!(param_bool(obj, "enable", true));
            for p in &[
                "kP",
                "kI",
                "kD",
                "enableScalarDamping",
                "enableVectorDamping",
                "rawValue",
                "dampedValue",
                "rawVector",
                "dampedVector",
                "vecErrorSum",
                "vecPreviousError",
                "errorSum",
                "previousError",
            ] {
                node[p] = json!(param_text(obj, p));
            }
        }
        50 => {
            // hkbLayer
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            push_conn(connections, 1, node_id, &param_text(obj, "generator"));
            node["weight"] = json!(or_default(param_text(obj, "weight"), "1.000000"));
            push_conn(connections, 2, node_id, &param_text(obj, "boneWeights"));
            node["fadeInDuration"] =
                json!(or_default(param_text(obj, "fadeInDuration"), "0.000000"));
            node["fadeOutDuration"] =
                json!(or_default(param_text(obj, "fadeOutDuration"), "0.000000"));
            node["onEventId"] = json!(param_int(obj, "onEventId", -1));
            node["offEventId"] = json!(param_int(obj, "offEventId", -1));
            node["onByDefault"] = json!(param_bool(obj, "onByDefault", true));
            node["useMotion"] = json!(param_bool(obj, "useMotion", false));
            node["forceFullFadeDurations"] =
                json!(param_bool(obj, "forceFullFadeDurations", false));
        }
        51 => {
            // hkbLayerGenerator
            push_conn(
                connections,
                0,
                node_id,
                &param_text(obj, "variableBindingSet"),
            );
            node["userData"] = json!(param_int(obj, "userData", 0));
            node["nodeName"] = json!(param_text(obj, "name"));
            if let Some(layers) = child_param(obj, "layers") {
                let refs: String = layers
                    .children()
                    .filter(|c| c.has_tag_name("hkobject"))
                    .filter_map(|o| o.text())
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join(" ");
                push_multi_conn(connections, 1, node_id, &refs);
            }
            node["indexOfSyncMasterChild"] = json!(param_int(obj, "indexOfSyncMasterChild", 65535));
            let flags_str = param_text(obj, "flagsIndex");
            let flags_index: i64 = if flags_str == "FLAG_SYNC" { 1 } else { 0 };
            node["flagsIndex"] = json!(flags_index);
        }
        _ => {}
    }

    node
}

// Color mapping mirroring _TYPE_COLOR_MAP and _color_leaf_state_machines_dict in Python.
const NUM_COLOR_THEMES: i64 = 18;

fn type_color_fallback(type_id: i64) -> i64 {
    match type_id {
        0 => 2,
        1 => 1,
        5 => 4,
        6 => 9,
        7 => 6,
        8 => 6,
        9 => 3,
        10 => 11,
        14 => 7,
        15 => 14,
        20 => 10,
        25 => 8,
        30 => 13,
        _ => 0,
    }
}

fn color_leaf_state_machines(
    nodes: &mut serde_json::Map<String, serde_json::Value>,
    connections: &[serde_json::Value],
) {
    // Build adjacency: from_id -> Vec<(port_idx, to_id)>
    let mut adj: std::collections::HashMap<i64, Vec<(i64, i64)>> = Default::default();
    for conn in connections {
        if let (Some(pi), Some(fi), Some(ti)) = (
            conn.get(0).and_then(|v| v.as_i64()),
            conn.get(1).and_then(|v| v.as_i64()),
            conn.get(2).and_then(|v| v.as_i64()),
        ) {
            adj.entry(fi).or_default().push((pi, ti));
        }
    }

    let mut state_machines: Vec<i64> = nodes
        .values()
        .filter(|n| n.get("nodeTypeID").and_then(|v| v.as_i64()) == Some(5))
        .filter_map(|n| n.get("nodeID").and_then(|v| v.as_i64()))
        .collect();
    state_machines.sort();

    let sm_set: std::collections::HashSet<i64> = state_machines.iter().copied().collect();
    let mut colored: std::collections::HashSet<i64> = Default::default();
    let num_colors = NUM_COLOR_THEMES - 1;
    let mut color_counter: i64 = 0;

    for sm_id in &state_machines {
        let state_ids: Vec<i64> = adj
            .get(sm_id)
            .map(|edges| {
                edges
                    .iter()
                    .filter(|(pi, _)| *pi == 1)
                    .map(|(_, ti)| *ti)
                    .collect()
            })
            .unwrap_or_default();

        for state_id in &state_ids {
            color_counter += 1;
            let color_id = (color_counter % num_colors) + 1;
            let mut queue = vec![*state_id];
            let mut visited: std::collections::HashSet<i64> = Default::default();
            while let Some(nid) = queue.pop() {
                if !visited.insert(nid) {
                    continue;
                }
                if let Some(nd) = nodes.get_mut(&nid.to_string()) {
                    nd["nodeColorID"] = serde_json::json!(color_id);
                    colored.insert(nid);
                }
                if nid != *state_id && sm_set.contains(&nid) {
                    continue;
                }
                if let Some(edges) = adj.get(&nid) {
                    for (_, child_id) in edges {
                        queue.push(*child_id);
                    }
                }
            }
        }
    }

    // Fallback colors for uncolored nodes
    let uncolored_updates: Vec<(String, i64)> = nodes
        .iter()
        .filter(|(_, nd)| {
            let nid = nd.get("nodeID").and_then(|v| v.as_i64()).unwrap_or(-1);
            !colored.contains(&nid) && nd.get("nodeColorID").and_then(|v| v.as_i64()) == Some(0)
        })
        .map(|(k, nd)| {
            let type_id = nd.get("nodeTypeID").and_then(|v| v.as_i64()).unwrap_or(-1);
            (k.clone(), type_color_fallback(type_id))
        })
        .filter(|(_, c)| *c != 0)
        .collect();

    for (key, color) in uncolored_updates {
        if let Some(nd) = nodes.get_mut(&key) {
            nd["nodeColorID"] = serde_json::json!(color);
        }
    }
}

/// Parse a Havok behavior graph XML and return a JSON string containing the full
/// UI dict-node graph that `ui/behaivor/graph_model.py::import_xml` consumes.
///
/// # Output shape
/// ```json
/// {
///   "nodes": {"1": {...}, "3": {...}},
///   "connections": [[port_idx, from_id, to_id], ...],
///   "global_state": {
///     "events": [...], "variables": [...], "transitions": [...],
///     "payloads": [...], "properties": [...]
///   },
///   "unhandled": [...]
/// }
/// ```
pub fn parse_behavior_graph_to_ui_json(xml: &str) -> HavokResult<String> {
    let doc = parse_doc(xml)?;

    let search_root = doc
        .descendants()
        .find(|n| n.has_tag_name("hksection") && n.attribute("name") == Some("__data__"))
        .unwrap_or_else(|| doc.root_element());

    let objects: Vec<roxmltree::Node<'_, '_>> = search_root
        .children()
        .filter(|n| n.has_tag_name("hkobject"))
        .collect();

    let (transition_map, transitions) = import_transitions(&objects);
    let (payload_map, payloads) = import_payloads(&objects);
    let (events, variables, properties) = import_global_values(&objects);

    let skip_classes = [
        "hkbBehaviorGraphData",
        "hkbVariableValueSet",
        "hkbBehaviorGraphStringData",
        "hkbBlendingTransitionEffect",
        "hkbStringEventPayload",
    ];

    // XML class → type_id (mirrors XML_CLASS_TO_TYPE_ID from Python node_types.py)
    let xml_class_to_type_id = |class: &str| -> Option<i64> {
        Some(match class {
            "hkRootLevelContainer" => 0,
            "hkbBehaviorGraph" => 1,
            "hkbBehaviorGraphData" => 2,
            "hkbVariableValueSet" => 3,
            "hkbBehaviorGraphStringData" => 4,
            "hkbStateMachine" => 5,
            "hkbStateMachineStateInfo" => 6,
            "hkbStateMachineTransitionInfoArray" => 7,
            "hkbStateMachineEventPropertyArray" => 8,
            "hkbModifierGenerator" => 9,
            "hkbModifierList" => 10,
            "hkbGetUpModifier" => 11,
            "hkbKeyframeBonesModifier" => 12,
            "hkbBoneIndexArray" => 13,
            "hkbBoneWeightArray" => 14,
            "hkbRigidBodyRagdollControlsModifier" => 15,
            "BSIsActiveModifier" => 16,
            "hkbVariableBindingSet" => 17,
            "hkbManualSelectorGenerator" => 18,
            "BSModifyOnceModifier" => 19,
            "hkbEvaluateExpressionModifier" => 20,
            "hkbExpressionDataArray" => 21,
            "hkbPoseMatchingGenerator" => 22,
            "hkbBlenderGenerator" => 23,
            "hkbBlenderGeneratorChild" => 24,
            "hkbClipGenerator" => 25,
            "hkbClipTriggerArray" => 26,
            "hkbEventDrivenModifier" => 28,
            "hkbPoweredRagdollControlsModifier" => 29,
            "hkbTimerModifier" => 30,
            "BSLookAtModifier" => 31,
            "BSGetTimeStepModifier" => 32,
            "hkbTwistModifier" => 33,
            "BSInterpValueModifier" => 34,
            "hkbEventsFromRangeModifier" => 35,
            "hkbEventRangeDataArray" => 36,
            "BSBehaviorGraphSwapGenerator" => 37,
            "BSRagdollContactListenerModifier" => 38,
            "BSCyclicBlendTransitionGenerator" => 39,
            "BGSGamebryoSequenceGenerator" => 40,
            "hkbBehaviorReferenceGenerator" => 42,
            "BSAssignVariablesModifier" => 43,
            "DynamicAnimationTaggingGenerator" => 44,
            "BSTimerModifier" => 45,
            "BSiStateTaggingGenerator" => 46,
            "hkbGeneratorTransitionEffect" => 47,
            "hkbReferencePoseGenerator" => 48,
            "hkbDampingModifier" => 49,
            "hkbLayer" => 50,
            "hkbLayerGenerator" => 51,
            "BSDirectAtModifier" => 52,
            _ => return None,
        })
    };

    // Metadata-only type IDs (not rendered as nodes in the graph)
    let is_metadata_only = |type_id: i64| matches!(type_id, 2 | 3 | 4);

    let mut nodes: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    let mut connections: Vec<serde_json::Value> = Vec::new();
    let mut unhandled: Vec<String> = Vec::new();

    for obj in &objects {
        let xml_class = obj.attribute("class").unwrap_or("");
        let xml_name = obj.attribute("name").unwrap_or("");

        if skip_classes.contains(&xml_class) {
            continue;
        }

        let Some(type_id) = xml_class_to_type_id(xml_class) else {
            unhandled.push(format!("{xml_name} - {xml_class}"));
            continue;
        };

        if is_metadata_only(type_id) {
            continue;
        }

        let Some(node_id) = parse_ref(xml_name) else {
            continue;
        };

        let node = build_node(
            *obj,
            type_id,
            node_id,
            &mut connections,
            &transition_map,
            &payload_map,
        );
        nodes.insert(node_id.to_string(), node);
    }

    color_leaf_state_machines(&mut nodes, &connections);

    let global_state = serde_json::json!({
        "events": events,
        "variables": variables,
        "transitions": transitions,
        "payloads": payloads,
        "properties": properties,
    });

    let result = serde_json::json!({
        "nodes": nodes,
        "connections": connections,
        "global_state": global_state,
        "unhandled": unhandled,
    });

    serde_json::to_string(&result).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

// ---------------------------------------------------------------------------
// Project parser
// ---------------------------------------------------------------------------

/// Parse a Havok project XML string and return structured metadata.
pub fn parse_project_xml(xml: &str) -> HavokResult<ProjectRecord> {
    let doc = parse_doc(xml)?;

    let mut character_filenames: Vec<String> = Vec::new();

    for obj in doc.descendants().filter(|n| n.has_tag_name("hkobject")) {
        if obj.attribute("class") == Some("hkbProjectStringData") {
            if let Some(cf_param) = child_param(obj, "characterFilenames") {
                for s in cf_param.children().filter(|c| c.has_tag_name("hkcstring")) {
                    if let Some(t) = s.text() {
                        character_filenames.push(t.trim().to_string());
                    }
                }
            }
        }
    }

    Ok(ProjectRecord {
        character_filenames,
    })
}
