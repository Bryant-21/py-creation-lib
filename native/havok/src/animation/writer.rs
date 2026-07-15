/// Port of `py_creation_lib/python/creation_lib/havok/animation_writer.py`.
///
/// Writes an `AnimationClip` as Havok interleaved-uncompressed animation XML
/// (`hkaInterleavedUncompressedAnimation`).  The caller decides where to save
/// the returned string.
use super::clip::{AnimationClip, AnimationKeyframe, BoneChannel};
use crate::error::HavokResult;

const DEFAULT_SAMPLE_RATE: u32 = 30;

// ---------------------------------------------------------------------------
// Quaternion SLERP
// ---------------------------------------------------------------------------

fn slerp(q0: [f32; 4], mut q1: [f32; 4], t: f32) -> [f32; 4] {
    let mut dot = q0[0] * q1[0] + q0[1] * q1[1] + q0[2] * q1[2] + q0[3] * q1[3];

    if dot < 0.0 {
        q1 = [-q1[0], -q1[1], -q1[2], -q1[3]];
        dot = -dot;
    }

    let dot = dot.min(1.0);

    let (rx, ry, rz, rw) = if dot > 0.9995 {
        (
            q0[0] + t * (q1[0] - q0[0]),
            q0[1] + t * (q1[1] - q0[1]),
            q0[2] + t * (q1[2] - q0[2]),
            q0[3] + t * (q1[3] - q0[3]),
        )
    } else {
        let theta = dot.acos();
        let sin_theta = theta.sin();
        let a = ((1.0 - t) * theta).sin() / sin_theta;
        let b = (t * theta).sin() / sin_theta;
        (
            a * q0[0] + b * q1[0],
            a * q0[1] + b * q1[1],
            a * q0[2] + b * q1[2],
            a * q0[3] + b * q1[3],
        )
    };

    let len = (rx * rx + ry * ry + rz * rz + rw * rw).sqrt();
    if len > 0.0 {
        [rx / len, ry / len, rz / len, rw / len]
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + t * (b[0] - a[0]),
        a[1] + t * (b[1] - a[1]),
        a[2] + t * (b[2] - a[2]),
    ]
}

// ---------------------------------------------------------------------------
// Keyframe sampling
// ---------------------------------------------------------------------------

fn sample_translation(keyframes: &[AnimationKeyframe<[f32; 3]>], time: f32) -> [f32; 3] {
    const IDENTITY: [f32; 3] = [0.0, 0.0, 0.0];
    sample_vec3(keyframes, time, IDENTITY)
}

fn sample_scale(keyframes: &[AnimationKeyframe<[f32; 3]>], time: f32) -> [f32; 3] {
    const IDENTITY: [f32; 3] = [1.0, 1.0, 1.0];
    sample_vec3(keyframes, time, IDENTITY)
}

fn sample_vec3(
    keyframes: &[AnimationKeyframe<[f32; 3]>],
    time: f32,
    default: [f32; 3],
) -> [f32; 3] {
    if keyframes.is_empty() {
        return default;
    }
    if time <= keyframes[0].time {
        return keyframes[0].value;
    }
    if time >= keyframes[keyframes.len() - 1].time {
        return keyframes[keyframes.len() - 1].value;
    }
    for window in keyframes.windows(2) {
        let k0 = &window[0];
        let k1 = &window[1];
        if k0.time <= time && time <= k1.time {
            let span = k1.time - k0.time;
            let t = if span > 0.0 {
                (time - k0.time) / span
            } else {
                0.0
            };
            return lerp3(k0.value, k1.value, t);
        }
    }
    keyframes[keyframes.len() - 1].value
}

fn sample_rotation(keyframes: &[AnimationKeyframe<[f32; 4]>], time: f32) -> [f32; 4] {
    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    if keyframes.is_empty() {
        return IDENTITY;
    }
    if time <= keyframes[0].time {
        return keyframes[0].value;
    }
    if time >= keyframes[keyframes.len() - 1].time {
        return keyframes[keyframes.len() - 1].value;
    }
    for window in keyframes.windows(2) {
        let k0 = &window[0];
        let k1 = &window[1];
        if k0.time <= time && time <= k1.time {
            let span = k1.time - k0.time;
            let t = if span > 0.0 {
                (time - k0.time) / span
            } else {
                0.0
            };
            return slerp(k0.value, k1.value, t);
        }
    }
    keyframes[keyframes.len() - 1].value
}

// ---------------------------------------------------------------------------
// Formatting helpers — mirror Python _fmt / _transform_str exactly
// ---------------------------------------------------------------------------

#[inline]
fn fmt(v: f32) -> String {
    format!("{:.6}", v)
}

fn transform_str(translation: [f32; 3], rotation: [f32; 4], scale: [f32; 3]) -> String {
    let t = format!(
        "{} {} {} {}",
        fmt(translation[0]),
        fmt(translation[1]),
        fmt(translation[2]),
        fmt(0.0)
    );
    let q = format!(
        "{} {} {} {}",
        fmt(rotation[0]),
        fmt(rotation[1]),
        fmt(rotation[2]),
        fmt(rotation[3])
    );
    let s = format!(
        "{} {} {} {}",
        fmt(scale[0]),
        fmt(scale[1]),
        fmt(scale[2]),
        fmt(0.0)
    );
    format!("({t} {q} {s})")
}

// ---------------------------------------------------------------------------
// Public writer
// ---------------------------------------------------------------------------

/// Write `clip` as Havok interleaved-uncompressed animation XML.
///
/// `skeleton_bone_names` defines the track order.  Returns the XML as a
/// `String`; the caller is responsible for writing it to disk.
pub fn write_interleaved_animation_xml(
    clip: &AnimationClip,
    skeleton_bone_names: &[String],
) -> HavokResult<String> {
    let bone_count = skeleton_bone_names.len();

    // Build name → channel lookup
    let channel_map: std::collections::HashMap<&str, &BoneChannel> = clip
        .channels
        .iter()
        .map(|ch| (ch.bone_name.as_str(), ch))
        .collect();

    let sample_rate = {
        let r = clip.native_fps.round() as u32;
        if r < 1 { DEFAULT_SAMPLE_RATE } else { r }
    };

    let duration = clip.duration.max(0.0);
    let frame_count = ((duration * sample_rate as f32) as usize + 1).max(1);

    // Build annotation events for track 0
    let events_on_track0: Vec<(f32, &str)> = clip
        .events
        .iter()
        .map(|ev| (ev.time, ev.text.as_str()))
        .collect();

    // ------------------------------------------------------------------
    // Build the XML string manually (no external XML dep)
    // ------------------------------------------------------------------
    let mut out = String::with_capacity(4096);

    out.push_str("<?xml version=\"1.0\" encoding=\"ASCII\" standalone=\"no\"?>\n");
    out.push_str("<hkpackfile classversion=\"11\" contentsversion=\"hk_2014.1.0-r1\">\n");
    out.push_str("  <hksection name=\"__data__\">\n");

    // hkaAnimationContainer
    out.push_str("    <hkobject class=\"hkaAnimationContainer\" signature=\"0x8dc20571\">\n");
    out.push_str("      <hkparam name=\"skeletons\" numelements=\"0\"></hkparam>\n");
    out.push_str("      <hkparam name=\"animations\" numelements=\"1\">#animation</hkparam>\n");
    out.push_str("      <hkparam name=\"bindings\" numelements=\"1\">#binding</hkparam>\n");
    out.push_str("      <hkparam name=\"attachments\" numelements=\"0\"></hkparam>\n");
    out.push_str("      <hkparam name=\"skins\" numelements=\"0\"></hkparam>\n");
    out.push_str("    </hkobject>\n");

    // hkaInterleavedUncompressedAnimation
    out.push_str("    <hkobject name=\"#animation\" class=\"hkaInterleavedUncompressedAnimation\" signature=\"0x930af031\">\n");
    out.push_str("      <hkparam name=\"type\">HK_INTERLEAVED_ANIMATION</hkparam>\n");
    out.push_str(&format!(
        "      <hkparam name=\"duration\">{}</hkparam>\n",
        fmt(duration)
    ));
    out.push_str(&format!(
        "      <hkparam name=\"numberOfTransformTracks\">{bone_count}</hkparam>\n"
    ));
    out.push_str("      <hkparam name=\"numberOfFloatTracks\">0</hkparam>\n");
    // Preserve extracted-motion reference if the source clip carried one.
    let extracted_motion = if clip.extracted_motion_ref.is_empty() {
        "#null".to_string()
    } else {
        clip.extracted_motion_ref.clone()
    };
    out.push_str(&format!(
        "      <hkparam name=\"extractedMotion\">{extracted_motion}</hkparam>\n"
    ));

    // Annotation tracks
    out.push_str(&format!(
        "      <hkparam name=\"annotationTracks\" numelements=\"{bone_count}\">\n"
    ));
    for (track_idx, bone_name) in skeleton_bone_names.iter().enumerate() {
        out.push_str("        <hkobject>\n");
        out.push_str(&format!(
            "          <hkparam name=\"trackName\">{bone_name}</hkparam>\n"
        ));
        let track_events: &[(f32, &str)] = if track_idx == 0 {
            &events_on_track0
        } else {
            &[]
        };
        out.push_str(&format!(
            "          <hkparam name=\"annotations\" numelements=\"{}\">\n",
            track_events.len()
        ));
        for (ev_time, ev_text) in track_events {
            out.push_str("            <hkobject>\n");
            out.push_str(&format!(
                "              <hkparam name=\"time\">{}</hkparam>\n",
                fmt(*ev_time)
            ));
            out.push_str(&format!(
                "              <hkparam name=\"text\">{ev_text}</hkparam>\n"
            ));
            out.push_str("            </hkobject>\n");
        }
        out.push_str("          </hkparam>\n");
        out.push_str("        </hkobject>\n");
    }
    out.push_str("      </hkparam>\n");

    // Transforms
    let total_transforms = frame_count * bone_count;
    out.push_str(&format!(
        "      <hkparam name=\"transforms\" numelements=\"{total_transforms}\">\n"
    ));
    for frame_idx in 0..frame_count {
        let time = if frame_count > 1 {
            (frame_idx as f32 / sample_rate as f32).min(duration)
        } else {
            0.0
        };
        for bone_name in skeleton_bone_names {
            let (tr, rot, sc) = if let Some(ch) = channel_map.get(bone_name.as_str()) {
                (
                    sample_translation(&ch.translations, time),
                    sample_rotation(&ch.rotations, time),
                    sample_scale(&ch.scales, time),
                )
            } else {
                ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
            };
            out.push_str(&format!("        {}\n", transform_str(tr, rot, sc)));
        }
    }
    out.push_str("      </hkparam>\n");

    // Floats
    out.push_str("      <hkparam name=\"floats\" numelements=\"0\"></hkparam>\n");

    out.push_str("    </hkobject>\n");

    // hkaAnimationBinding
    let skeleton_name = clip.original_skeleton_name.as_deref().unwrap_or("skeleton");
    out.push_str(
        "    <hkobject name=\"#binding\" class=\"hkaAnimationBinding\" signature=\"0x66eac971\">\n",
    );
    out.push_str(&format!(
        "      <hkparam name=\"originalSkeletonName\">{skeleton_name}</hkparam>\n"
    ));
    out.push_str("      <hkparam name=\"animation\">#animation</hkparam>\n");
    // Round-trip transformTrackToBoneIndices; fall back to identity mapping.
    let indices: Vec<String> = if clip.track_to_bone_indices.len() == bone_count {
        clip.track_to_bone_indices
            .iter()
            .map(|i| i.to_string())
            .collect()
    } else {
        (0..bone_count).map(|i| i.to_string()).collect()
    };
    out.push_str(&format!(
        "      <hkparam name=\"transformTrackToBoneIndices\" numelements=\"{bone_count}\">{}</hkparam>\n",
        indices.join(" ")
    ));
    out.push_str(
        "      <hkparam name=\"floatTrackToFloatSlotIndices\" numelements=\"0\"></hkparam>\n",
    );
    // Round-trip blendHint; fall back to NORMAL.
    out.push_str(&format!(
        "      <hkparam name=\"blendHint\">{}</hkparam>\n",
        if clip.is_additive {
            "ADDITIVE"
        } else {
            "NORMAL"
        }
    ));
    out.push_str("    </hkobject>\n");

    out.push_str("  </hksection>\n");
    out.push_str("</hkpackfile>\n");

    Ok(out)
}
