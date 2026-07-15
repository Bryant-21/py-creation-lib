/// Animation domain model — Rust mirror of `creation_lib.animation.models`.
///
/// Reference: `py_creation_lib/python/creation_lib/havok/animation_reader.py`.
use crate::animation::parsers::SkeletonRecord;
use crate::animation::quantized::read_quantized_animation;
use crate::animation::spline::decompress_spline;
use crate::error::{HavokError, HavokResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnimationKeyframe<T> {
    pub time: f32,
    pub value: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoneChannel {
    pub bone_name: String,
    pub translations: Vec<AnimationKeyframe<[f32; 3]>>,
    pub rotations: Vec<AnimationKeyframe<[f32; 4]>>,
    pub scales: Vec<AnimationKeyframe<[f32; 3]>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnimationEvent {
    pub time: f32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnimationClip {
    pub source_format: String,
    pub duration: f32,
    pub native_fps: f32,
    pub channels: Vec<BoneChannel>,
    pub events: Vec<AnimationEvent>,
    pub original_skeleton_name: Option<String>,
    pub warnings: Vec<String>,
    /// True when blendHint == "ADDITIVE" in the source binding.
    pub is_additive: bool,
    /// transformTrackToBoneIndices from hkaAnimationBinding; empty = identity mapping.
    pub track_to_bone_indices: Vec<u32>,
    /// Non-null extractedMotion pointer reference (e.g. "#0006"); empty = #null.
    pub extracted_motion_ref: String,
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Extract full animation keyframe data from Havok XML into an `AnimationClip`.
///
/// Detects compression class (lossless, interleaved, spline), decodes
/// keyframes, maps track indices to bone names via `skeleton` when provided
/// (otherwise generates `track_{idx}` names), and populates events from the
/// first annotation track.
pub fn extract_clip(xml: &str, skeleton: Option<&SkeletonRecord>) -> HavokResult<AnimationClip> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| HavokError::InvalidInput(format!("invalid XML: {e}")))?;

    let skip_classes = [
        "hkaAnimationContainer",
        "hkaAnimationBinding",
        "hkRootLevelContainer",
    ];

    // Find animation object and detect compression
    let anim_node = doc.descendants().find(|node| {
        if !node.has_tag_name("hkobject") {
            return false;
        }
        let cls = node.attribute("class").unwrap_or("");
        cls.contains("Animation") && cls.starts_with("hka") && !skip_classes.contains(&cls)
    });

    let Some(anim_node) = anim_node else {
        return Err(HavokError::InvalidInput(
            "XML does not contain a supported hka animation object".to_string(),
        ));
    };

    let class_name = anim_node.attribute("class").unwrap_or("");
    let compression = if class_name.contains("Lossless") {
        "lossless"
    } else if class_name.contains("Spline") {
        "spline"
    } else if class_name.contains("Interleaved") {
        "interleaved"
    } else if class_name.contains("Quantized") {
        // hkaQuantizedAnimation: binary-blob format decoded via
        // animation::quantized (see extract_quantized).
        "quantized"
    } else if class_name.contains("Mirrored") {
        // hkaMirroredAnimation: wraps a source animation with mirror semantics.
        // We treat it as the source animation with a mirrored flag; this gives
        // callers a non-empty clip without crashing.
        "mirrored"
    } else if class_name.contains("ReferencePose") {
        // hkaReferencePoseAnimation: single-keyframe clip from skeleton ref pose.
        "reference_pose"
    } else {
        "unknown"
    };

    let duration = float_param(anim_node, "duration").unwrap_or(0.0);
    let bone_count = int_param(anim_node, "numberOfTransformTracks").unwrap_or(0) as usize;

    // Parse events from annotation tracks (from the animation object itself or
    // from separate top-level hkaAnnotationTrack objects).
    let events = parse_events_from_anim(anim_node);

    // Resolve bone names
    let bone_names = resolve_bone_names(bone_count, skeleton);

    // Parse binding: originalSkeletonName, blendHint, transformTrackToBoneIndices
    let binding_node = doc.descendants().find(|node| {
        node.has_tag_name("hkobject") && node.attribute("class") == Some("hkaAnimationBinding")
    });
    let original_skeleton_name = binding_node
        .and_then(|b| text_param(b, "originalSkeletonName"))
        .filter(|s| !s.is_empty());
    let is_additive = binding_node
        .and_then(|b| text_param(b, "blendHint"))
        .map(|h| h.to_ascii_uppercase() == "ADDITIVE" || h == "1")
        .unwrap_or(false);
    let track_to_bone_indices = binding_node
        .and_then(|b| text_param(b, "transformTrackToBoneIndices"))
        .map(|text| {
            text.split_whitespace()
                .filter_map(|s| s.parse::<u32>().ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // Preserve non-null extractedMotion pointer.
    let extracted_motion_ref = text_param(anim_node, "extractedMotion")
        .filter(|s| s != "#null" && s.to_ascii_lowercase() != "null" && !s.is_empty())
        .unwrap_or_default();

    let mut warnings = Vec::new();

    let channels = match compression {
        "lossless" => extract_lossless(anim_node, bone_count, &bone_names, &mut warnings),
        "interleaved" => extract_interleaved(anim_node, bone_count, &bone_names, duration),
        "spline" => extract_spline(anim_node, bone_count, &bone_names, &mut warnings),
        "quantized" => extract_quantized(anim_node, bone_count, &bone_names, &mut warnings),
        "mirrored" => {
            // hkaMirroredAnimation wraps a source animation with mirrored bone
            // semantics. We decode the source animation data directly from this
            // object so callers get a usable (non-empty) clip. The mirroring
            // semantics (left↔right bone swap) are not applied; callers that
            // need true mirrored output must apply hkaMirroredSkeleton mappings
            // themselves.
            warnings.push(format!(
                "hkaMirroredAnimation decoded as source data; mirror semantics not applied \
                 (class: {class_name})"
            ));
            // The Mirrored animation stores the original data inline — try all
            // formats in priority order.
            let cls = class_name;
            if cls.contains("Spline") {
                extract_spline(anim_node, bone_count, &bone_names, &mut warnings)
            } else if cls.contains("Interleaved") {
                extract_interleaved(anim_node, bone_count, &bone_names, duration)
            } else {
                extract_lossless(anim_node, bone_count, &bone_names, &mut warnings)
            }
        }
        "reference_pose" => {
            // hkaReferencePoseAnimation: a single-keyframe clip using the
            // skeleton's reference pose. Extract as lossless single frame.
            extract_lossless(anim_node, bone_count, &bone_names, &mut warnings)
        }
        _ => {
            warnings.push(format!("Unknown animation compression class: {class_name}"));
            Vec::new()
        }
    };

    let mut clip = AnimationClip {
        source_format: "hkx".to_string(),
        duration,
        native_fps: 0.0,
        channels,
        events,
        original_skeleton_name,
        warnings,
        is_additive,
        track_to_bone_indices,
        extracted_motion_ref,
    };

    clip.native_fps = infer_clip_fps_from_xml(anim_node, duration);
    if clip.native_fps == 0.0 {
        clip.native_fps = infer_clip_fps(&clip, 30.0);
    }

    Ok(clip)
}

/// Derive native sample rate from clip keyframe spacing.
///
/// Looks for the first channel with ≥2 keyframes and returns 1/dt.
/// Falls back to `default` for static-only / single-frame clips.
pub fn infer_clip_fps(clip: &AnimationClip, default: f32) -> f32 {
    for ch in &clip.channels {
        for series_dt in [
            first_two_dt(&ch.rotations),
            first_two_dt(&ch.translations),
            first_two_dt(&ch.scales),
        ] {
            if let Some(dt) = series_dt {
                if dt > 1e-6 {
                    return 1.0 / dt;
                }
            }
        }
    }
    default
}

// ---------------------------------------------------------------------------
// FPS inference from XML metadata
// ---------------------------------------------------------------------------

fn infer_clip_fps_from_xml(anim_node: roxmltree::Node<'_, '_>, duration: f32) -> f32 {
    // Spline: use frameDuration directly
    if anim_node
        .attribute("class")
        .unwrap_or("")
        .contains("Spline")
    {
        if let Some(fd) = float_param(anim_node, "frameDuration") {
            if fd > 1e-8 {
                return 1.0 / fd;
            }
        }
    }

    // Generic: (numFrames-1) / duration
    let num_frames = int_param(anim_node, "numberOfFrames")
        .or_else(|| int_param(anim_node, "numFrames"))
        .unwrap_or(0) as usize;

    if num_frames > 1 && duration > 1e-8 {
        return (num_frames as f32 - 1.0) / duration;
    }

    0.0
}

// ---------------------------------------------------------------------------
// Lossless extractor
// ---------------------------------------------------------------------------

fn extract_lossless(
    anim_node: roxmltree::Node<'_, '_>,
    bone_count: usize,
    bone_names: &[String],
    _warnings: &mut Vec<String>,
) -> Vec<BoneChannel> {
    // --- Rotations ---
    let static_rots = child_param(anim_node, "staticRotations")
        .and_then(|p| p.text())
        .map(parse_paren_groups)
        .unwrap_or_default();

    let rot_offsets = text_param(anim_node, "rotationTypeAndOffsets")
        .map(|t| parse_int_list(&t))
        .unwrap_or_default();

    let dyn_rots = child_param(anim_node, "dynamicRotations")
        .and_then(|p| p.text())
        .map(parse_paren_groups)
        .unwrap_or_default();

    // --- Translations ---
    let static_trans_floats = text_param(anim_node, "staticTranslations")
        .map(|t| parse_float_list(&t))
        .unwrap_or_default();

    let trans_raw = text_param(anim_node, "translationTypeAndOffsets")
        .map(|t| parse_int_list(&t))
        .unwrap_or_default();
    let trans_components = decode_per_component_offsets(&trans_raw, bone_count);

    let dyn_trans_floats = text_param(anim_node, "dynamicTranslations")
        .map(|t| parse_float_list(&t))
        .unwrap_or_default();

    // --- Scales ---
    let static_scale_floats = text_param(anim_node, "staticScales")
        .map(|t| parse_float_list(&t))
        .unwrap_or_default();

    let scale_raw = text_param(anim_node, "scaleTypeAndOffsets")
        .map(|t| parse_int_list(&t))
        .unwrap_or_default();
    let scale_components = decode_per_component_offsets(&scale_raw, bone_count);

    let dyn_scale_floats = text_param(anim_node, "dynamicScales")
        .map(|t| parse_float_list(&t))
        .unwrap_or_default();

    // --- Frame count ---
    let duration = float_param(anim_node, "duration").unwrap_or(0.0);
    let num_frames = int_param(anim_node, "numberOfFrames").unwrap_or(1) as usize;
    let frame_duration = if num_frames > 1 {
        duration / (num_frames as f32 - 1.0)
    } else {
        duration
    };

    // Dynamic rotation track indices (type == 2)
    let dyn_rot_track_indices: Vec<usize> = (0..bone_count.min(rot_offsets.len()))
        .filter(|&i| (rot_offsets[i] & 3) == 2)
        .collect();

    // Count dynamic components for translation and scale
    let num_dyn_trans = count_dyn_components(&trans_components, bone_count);
    let num_dyn_scale = count_dyn_components(&scale_components, bone_count);

    let mut channels = Vec::with_capacity(bone_count);
    for bone_idx in 0..bone_count {
        let name = bone_names
            .get(bone_idx)
            .cloned()
            .unwrap_or_else(|| format!("track_{bone_idx}"));

        // Rotation
        let rotations = decode_lossless_rotation(
            bone_idx,
            &rot_offsets,
            &static_rots,
            &dyn_rots,
            &dyn_rot_track_indices,
            num_frames,
            frame_duration,
        );

        // Translation
        let translations = decode_lossless_vec3(
            bone_idx,
            &trans_components,
            &static_trans_floats,
            &dyn_trans_floats,
            num_dyn_trans,
            num_frames,
            frame_duration,
            0.0,
        );

        // Scale
        let scales = decode_lossless_vec3(
            bone_idx,
            &scale_components,
            &static_scale_floats,
            &dyn_scale_floats,
            num_dyn_scale,
            num_frames,
            frame_duration,
            1.0,
        );

        channels.push(BoneChannel {
            bone_name: name,
            translations,
            rotations,
            scales,
        });
    }

    channels
}

fn decode_lossless_rotation(
    bone_idx: usize,
    rot_offsets: &[i64],
    static_rots: &[Vec<f32>],
    dyn_rots: &[Vec<f32>],
    dyn_track_indices: &[usize],
    num_frames: usize,
    frame_duration: f32,
) -> Vec<AnimationKeyframe<[f32; 4]>> {
    if bone_idx >= rot_offsets.len() {
        return vec![AnimationKeyframe {
            time: 0.0,
            value: [0.0, 0.0, 0.0, 1.0],
        }];
    }

    let raw = rot_offsets[bone_idx];
    let rtype = raw & 3;
    let roffset = (raw >> 2) as usize;

    match rtype {
        0 => {
            // identity
            vec![AnimationKeyframe {
                time: 0.0,
                value: [0.0, 0.0, 0.0, 1.0],
            }]
        }
        1 => {
            // static
            if roffset < static_rots.len() {
                let q = &static_rots[roffset];
                let val = [
                    q.first().copied().unwrap_or(0.0),
                    q.get(1).copied().unwrap_or(0.0),
                    q.get(2).copied().unwrap_or(0.0),
                    q.get(3).copied().unwrap_or(1.0),
                ];
                vec![AnimationKeyframe {
                    time: 0.0,
                    value: val,
                }]
            } else {
                vec![AnimationKeyframe {
                    time: 0.0,
                    value: [0.0, 0.0, 0.0, 1.0],
                }]
            }
        }
        2 => {
            // dynamic
            if let Some(track_order) = dyn_track_indices.iter().position(|&t| t == bone_idx) {
                (0..num_frames)
                    .filter_map(|frame| {
                        let data_idx = frame * dyn_track_indices.len() + track_order;
                        if data_idx < dyn_rots.len() {
                            let q = &dyn_rots[data_idx];
                            Some(AnimationKeyframe {
                                time: frame as f32 * frame_duration,
                                value: [
                                    q.first().copied().unwrap_or(0.0),
                                    q.get(1).copied().unwrap_or(0.0),
                                    q.get(2).copied().unwrap_or(0.0),
                                    q.get(3).copied().unwrap_or(1.0),
                                ],
                            })
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                vec![]
            }
        }
        _ => vec![AnimationKeyframe {
            time: 0.0,
            value: [0.0, 0.0, 0.0, 1.0],
        }],
    }
}

fn decode_lossless_vec3(
    bone_idx: usize,
    components: &[[u16; 4]],
    static_floats: &[f32],
    dyn_floats: &[f32],
    num_dyn_components: usize,
    num_frames: usize,
    frame_duration: f32,
    identity: f32,
) -> Vec<AnimationKeyframe<[f32; 3]>> {
    if bone_idx >= components.len() {
        return vec![AnimationKeyframe {
            time: 0.0,
            value: [identity; 3],
        }];
    }

    let comp = components[bone_idx];
    let desc: [(u16, u16); 3] = [
        (comp[0] & 3, comp[0] >> 2),
        (comp[1] & 3, comp[1] >> 2),
        (comp[2] & 3, comp[2] >> 2),
    ];

    let has_dynamic = desc.iter().any(|(t, _)| *t == 2);

    if has_dynamic {
        (0..num_frames)
            .filter_map(|frame| {
                let mut xyz = [identity; 3];
                for (j, (ctype, coff)) in desc.iter().enumerate() {
                    match ctype {
                        0 => xyz[j] = identity,
                        1 => {
                            if (*coff as usize) < static_floats.len() {
                                xyz[j] = static_floats[*coff as usize];
                            } else {
                                return None;
                            }
                        }
                        2 => {
                            let data_idx = frame * num_dyn_components + *coff as usize;
                            if data_idx < dyn_floats.len() {
                                xyz[j] = dyn_floats[data_idx];
                            } else {
                                return None;
                            }
                        }
                        _ => {}
                    }
                }
                Some(AnimationKeyframe {
                    time: frame as f32 * frame_duration,
                    value: xyz,
                })
            })
            .collect()
    } else {
        // All static/identity → single keyframe
        let mut xyz = [identity; 3];
        for (j, (ctype, coff)) in desc.iter().enumerate() {
            match ctype {
                0 => xyz[j] = identity,
                1 => {
                    if (*coff as usize) < static_floats.len() {
                        xyz[j] = static_floats[*coff as usize];
                    } else {
                        return vec![];
                    }
                }
                _ => {}
            }
        }
        vec![AnimationKeyframe {
            time: 0.0,
            value: xyz,
        }]
    }
}

/// Decode per-component uint64 offsets from a list of integer values.
/// Each uint64 packs four uint16 values (x, y, z, w) little-endian:
///   x = val & 0xFFFF, y = (val >> 16) & 0xFFFF, z = (val >> 32) & 0xFFFF
fn decode_per_component_offsets(raw: &[i64], bone_count: usize) -> Vec<[u16; 4]> {
    (0..bone_count)
        .map(|i| {
            if i < raw.len() {
                let v = raw[i] as u64;
                [
                    (v & 0xFFFF) as u16,
                    ((v >> 16) & 0xFFFF) as u16,
                    ((v >> 32) & 0xFFFF) as u16,
                    ((v >> 48) & 0xFFFF) as u16,
                ]
            } else {
                [0u16; 4]
            }
        })
        .collect()
}

fn count_dyn_components(components: &[[u16; 4]], bone_count: usize) -> usize {
    let mut count = 0;
    for &comp in components.iter().take(bone_count) {
        for j in 0..3 {
            if (comp[j] & 3) == 2 {
                count += 1;
            }
        }
    }
    count
}

// ---------------------------------------------------------------------------
// Interleaved extractor
// ---------------------------------------------------------------------------

fn extract_interleaved(
    anim_node: roxmltree::Node<'_, '_>,
    bone_count: usize,
    bone_names: &[String],
    duration: f32,
) -> Vec<BoneChannel> {
    if bone_count == 0 {
        return Vec::new();
    }

    let transforms_text = match child_param(anim_node, "transforms").and_then(|p| p.text()) {
        Some(t) => t.to_string(),
        None => return Vec::new(),
    };

    let tuples = parse_paren_groups(&transforms_text);
    if tuples.is_empty() {
        return Vec::new();
    }

    // Compact format: 12+ floats per tuple (one tuple per bone per frame)
    // Legacy format: 3 tuples per bone per frame (t, q, s)
    let compact = tuples.first().map(|t| t.len() >= 12).unwrap_or(false);
    let tuples_per_bone_frame = if compact { 1 } else { 3 };

    let total_bone_frames = tuples.len() / tuples_per_bone_frame;
    let num_frames = total_bone_frames / bone_count;
    let frame_duration = if num_frames > 1 {
        duration / (num_frames as f32 - 1.0)
    } else {
        duration
    };

    let mut per_bone_trans: Vec<Vec<AnimationKeyframe<[f32; 3]>>> = vec![Vec::new(); bone_count];
    let mut per_bone_rots: Vec<Vec<AnimationKeyframe<[f32; 4]>>> = vec![Vec::new(); bone_count];
    let mut per_bone_scales: Vec<Vec<AnimationKeyframe<[f32; 3]>>> = vec![Vec::new(); bone_count];

    for frame in 0..num_frames {
        let time = frame as f32 * frame_duration;
        for bone_idx in 0..bone_count {
            let base = (frame * bone_count + bone_idx) * tuples_per_bone_frame;
            if base >= tuples.len() {
                break;
            }

            let (t, q, s) = if compact {
                let v = &tuples[base];
                (
                    [
                        v.first().copied().unwrap_or(0.0),
                        v.get(1).copied().unwrap_or(0.0),
                        v.get(2).copied().unwrap_or(0.0),
                    ],
                    [
                        v.get(4).copied().unwrap_or(0.0),
                        v.get(5).copied().unwrap_or(0.0),
                        v.get(6).copied().unwrap_or(0.0),
                        v.get(7).copied().unwrap_or(1.0),
                    ],
                    [
                        v.get(8).copied().unwrap_or(1.0),
                        v.get(9).copied().unwrap_or(1.0),
                        v.get(10).copied().unwrap_or(1.0),
                    ],
                )
            } else {
                if base + 2 >= tuples.len() {
                    break;
                }
                let tv = &tuples[base];
                let qv = &tuples[base + 1];
                let sv = &tuples[base + 2];
                (
                    [
                        tv.first().copied().unwrap_or(0.0),
                        tv.get(1).copied().unwrap_or(0.0),
                        tv.get(2).copied().unwrap_or(0.0),
                    ],
                    [
                        qv.first().copied().unwrap_or(0.0),
                        qv.get(1).copied().unwrap_or(0.0),
                        qv.get(2).copied().unwrap_or(0.0),
                        qv.get(3).copied().unwrap_or(1.0),
                    ],
                    [
                        sv.first().copied().unwrap_or(1.0),
                        sv.get(1).copied().unwrap_or(1.0),
                        sv.get(2).copied().unwrap_or(1.0),
                    ],
                )
            };

            per_bone_trans[bone_idx].push(AnimationKeyframe { time, value: t });
            per_bone_rots[bone_idx].push(AnimationKeyframe { time, value: q });

            // Only emit scale keyframes when non-identity (mirrors Python)
            if s.iter().any(|&v| (v - 1.0f32).abs() > 1e-6) {
                per_bone_scales[bone_idx].push(AnimationKeyframe { time, value: s });
            }
        }
    }

    (0..bone_count)
        .map(|bone_idx| {
            let name = bone_names
                .get(bone_idx)
                .cloned()
                .unwrap_or_else(|| format!("track_{bone_idx}"));
            BoneChannel {
                bone_name: name,
                translations: per_bone_trans.remove(0),
                rotations: per_bone_rots.remove(0),
                scales: per_bone_scales.remove(0),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Spline extractor
// ---------------------------------------------------------------------------

fn extract_spline(
    anim_node: roxmltree::Node<'_, '_>,
    bone_count: usize,
    bone_names: &[String],
    warnings: &mut Vec<String>,
) -> Vec<BoneChannel> {
    if bone_count == 0 {
        return Vec::new();
    }

    let num_frames = int_param(anim_node, "numFrames").unwrap_or(0) as u32;
    let num_blocks = int_param(anim_node, "numBlocks").unwrap_or(1) as u32;
    let max_frames_per_block = int_param(anim_node, "maxFramesPerBlock").unwrap_or(256) as u32;
    let mask_and_quant_size = int_param(anim_node, "maskAndQuantizationSize").unwrap_or(0) as u32;
    let block_duration = float_param(anim_node, "blockDuration").unwrap_or(0.0);
    let block_inverse_duration = float_param(anim_node, "blockInverseDuration").unwrap_or(0.0);
    let frame_duration = float_param(anim_node, "frameDuration").unwrap_or(0.0);
    let num_float_tracks = int_param(anim_node, "numberOfFloatTracks").unwrap_or(0) as u32;

    let block_offsets = text_param(anim_node, "blockOffsets")
        .map(|t| {
            t.split_whitespace()
                .filter_map(|s| s.parse::<u32>().ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let float_block_offsets = text_param(anim_node, "floatBlockOffsets")
        .map(|t| {
            t.split_whitespace()
                .filter_map(|s| s.parse::<u32>().ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // Parse raw data blob (space-separated decimal byte values)
    let data_text = match child_param(anim_node, "data").and_then(|p| p.text()) {
        Some(t) => t.to_string(),
        None => {
            warnings.push("spline animation has no data param".to_string());
            return Vec::new();
        }
    };

    let data_bytes: Vec<u8> = data_text
        .split_whitespace()
        .filter_map(|s| s.parse::<u8>().ok())
        .collect();

    if data_bytes.is_empty() || num_frames == 0 {
        warnings.push("spline animation data is empty".to_string());
        return Vec::new();
    }

    let all_frames = match decompress_spline(
        &data_bytes,
        bone_count as u32,
        num_float_tracks,
        num_frames,
        max_frames_per_block,
        num_blocks,
        &block_offsets,
        &float_block_offsets,
        mask_and_quant_size,
        block_duration,
        block_inverse_duration,
        frame_duration,
    ) {
        Ok(frames) => frames,
        Err(e) => {
            warnings.push(format!("spline decompression failed: {e}"));
            return Vec::new();
        }
    };

    if all_frames.is_empty() {
        return Vec::new();
    }

    let mut per_bone_trans: Vec<Vec<AnimationKeyframe<[f32; 3]>>> = vec![Vec::new(); bone_count];
    let mut per_bone_rots: Vec<Vec<AnimationKeyframe<[f32; 4]>>> = vec![Vec::new(); bone_count];
    let mut per_bone_scales: Vec<Vec<AnimationKeyframe<[f32; 3]>>> = vec![Vec::new(); bone_count];

    for (frame_idx, frame) in all_frames.iter().enumerate() {
        let time = frame_idx as f32 * frame_duration;
        for (bone_idx, xf) in frame.transforms.iter().enumerate() {
            if bone_idx >= bone_count {
                break;
            }
            per_bone_trans[bone_idx].push(AnimationKeyframe {
                time,
                value: xf.translation,
            });
            per_bone_rots[bone_idx].push(AnimationKeyframe {
                time,
                value: xf.rotation,
            });

            if xf.scale.iter().any(|&v| (v - 1.0f32).abs() > 1e-4) {
                per_bone_scales[bone_idx].push(AnimationKeyframe {
                    time,
                    value: xf.scale,
                });
            }
        }
    }

    (0..bone_count)
        .map(|bone_idx| {
            let name = bone_names
                .get(bone_idx)
                .cloned()
                .unwrap_or_else(|| format!("track_{bone_idx}"));
            BoneChannel {
                bone_name: name,
                translations: per_bone_trans.remove(0),
                rotations: per_bone_rots.remove(0),
                scales: per_bone_scales.remove(0),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Quantized extractor
// ---------------------------------------------------------------------------

fn extract_quantized(
    anim_node: roxmltree::Node<'_, '_>,
    bone_count: usize,
    bone_names: &[String],
    warnings: &mut Vec<String>,
) -> Vec<BoneChannel> {
    let data_text = match child_param(anim_node, "data").and_then(|p| p.text()) {
        Some(t) => t,
        None => {
            warnings.push("quantized animation has no data param".to_string());
            return Vec::new();
        }
    };

    let data_bytes: Vec<u8> = data_text
        .split_whitespace()
        .filter_map(|s| s.parse::<u8>().ok())
        .collect();

    if data_bytes.is_empty() {
        warnings.push("quantized animation data is empty".to_string());
        return Vec::new();
    }

    let quantized = match read_quantized_animation(&data_bytes) {
        Ok(animation) => animation,
        Err(e) => {
            warnings.push(format!("quantized animation decode failed: {e}"));
            return Vec::new();
        }
    };

    let track_count = if bone_count == 0 {
        quantized.num_tracks() as usize
    } else {
        bone_count.min(quantized.num_tracks() as usize)
    };
    let num_frames = quantized.num_frames() as usize;
    if track_count == 0 || num_frames == 0 {
        warnings.push("quantized animation has no tracks or frames".to_string());
        return Vec::new();
    }

    let frame_duration = if num_frames > 1 && quantized.duration() > 0.0 {
        quantized.duration() / (num_frames as f32 - 1.0)
    } else {
        0.0
    };

    let mut per_bone_trans: Vec<Vec<AnimationKeyframe<[f32; 3]>>> = vec![Vec::new(); track_count];
    let mut per_bone_rots: Vec<Vec<AnimationKeyframe<[f32; 4]>>> = vec![Vec::new(); track_count];
    let mut per_bone_scales: Vec<Vec<AnimationKeyframe<[f32; 3]>>> = vec![Vec::new(); track_count];

    for frame_idx in 0..num_frames {
        let pose = match quantized.sample_frame(frame_idx) {
            Ok(pose) => pose,
            Err(e) => {
                warnings.push(format!("quantized animation sample failed: {e}"));
                return Vec::new();
            }
        };
        let time = frame_idx as f32 * frame_duration;

        for bone_idx in 0..track_count {
            per_bone_trans[bone_idx].push(AnimationKeyframe {
                time,
                value: pose
                    .translations
                    .get(bone_idx)
                    .copied()
                    .unwrap_or([0.0, 0.0, 0.0]),
            });
            per_bone_rots[bone_idx].push(AnimationKeyframe {
                time,
                value: pose
                    .rotations
                    .get(bone_idx)
                    .copied()
                    .unwrap_or([0.0, 0.0, 0.0, 1.0]),
            });

            let scale = pose
                .scales
                .get(bone_idx)
                .copied()
                .unwrap_or([1.0, 1.0, 1.0]);
            if scale.iter().any(|&v| (v - 1.0f32).abs() > 1e-6) {
                per_bone_scales[bone_idx].push(AnimationKeyframe { time, value: scale });
            }
        }
    }

    (0..track_count)
        .map(|bone_idx| {
            let name = bone_names
                .get(bone_idx)
                .cloned()
                .unwrap_or_else(|| format!("track_{bone_idx}"));
            BoneChannel {
                bone_name: name,
                translations: per_bone_trans.remove(0),
                rotations: per_bone_rots.remove(0),
                scales: per_bone_scales.remove(0),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Event parsing
// ---------------------------------------------------------------------------

/// Parse animation events from annotation tracks.
///
/// Handles two layouts:
/// 1. Inline: annotation tracks are `hkobject` children of the animation's
///    `annotationTracks` param (typical in compact packfile XML).
/// 2. Separate: top-level `hkobject class="hkaAnnotationTrack"` elements
///    (typical in fully expanded TAG XML).
///
/// The function inspects the animation node first (inline), then falls back
/// to searching the document for separate annotation track objects.
fn parse_events_from_anim(anim_node: roxmltree::Node<'_, '_>) -> Vec<AnimationEvent> {
    let mut events = Vec::new();

    // Inline path: annotationTracks param inside the animation node
    if let Some(ann_tracks_param) = child_param(anim_node, "annotationTracks") {
        for track in ann_tracks_param
            .children()
            .filter(|n| n.has_tag_name("hkobject"))
        {
            if let Some(annotations_param) = child_param(track, "annotations") {
                for ann in annotations_param
                    .children()
                    .filter(|n| n.has_tag_name("hkobject"))
                {
                    let time = text_param(ann, "time")
                        .and_then(|t| t.parse::<f32>().ok())
                        .unwrap_or(0.0);
                    let text = text_param(ann, "text").unwrap_or_default();
                    if !text.is_empty() {
                        events.push(AnimationEvent { time, text });
                    }
                }
            }
        }
    }

    events
}

// ---------------------------------------------------------------------------
// Bone name resolution
// ---------------------------------------------------------------------------

fn resolve_bone_names(bone_count: usize, skeleton: Option<&SkeletonRecord>) -> Vec<String> {
    let mut names = Vec::with_capacity(bone_count);
    if let Some(skel) = skeleton {
        for i in 0..bone_count {
            if i < skel.bone_names.len() {
                names.push(skel.bone_names[i].clone());
            } else {
                names.push(format!("track_{i}"));
            }
        }
    } else {
        for i in 0..bone_count {
            names.push(format!("track_{i}"));
        }
    }
    names
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn first_two_dt<T>(keyframes: &[AnimationKeyframe<T>]) -> Option<f32> {
    if keyframes.len() >= 2 {
        Some(keyframes[1].time - keyframes[0].time)
    } else {
        None
    }
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

fn parse_paren_groups(text: &str) -> Vec<Vec<f32>> {
    let mut groups = Vec::new();
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

fn parse_float_list(text: &str) -> Vec<f32> {
    text.split_whitespace()
        .filter_map(|s| s.parse::<f32>().ok())
        .collect()
}

fn parse_int_list(text: &str) -> Vec<i64> {
    text.split_whitespace()
        .filter_map(|s| {
            // Handle both signed and unsigned integers
            s.parse::<i64>()
                .ok()
                .or_else(|| s.parse::<u64>().ok().map(|v| v as i64))
        })
        .collect()
}
