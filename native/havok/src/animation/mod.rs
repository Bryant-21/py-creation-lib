pub mod attachment;
pub mod clip;
pub mod compare_pose;
pub mod expand;
pub mod extrapolate;
pub mod footstep;
pub mod ik;
pub mod mapper_mt;
pub mod parsers;
pub mod pose;
pub mod pose_matching;
pub mod quantized;
pub mod retarget;
pub mod root_motion;
pub mod spline;
pub mod writer;

pub use clip::{
    AnimationClip, AnimationEvent, AnimationKeyframe, BoneChannel, extract_clip, infer_clip_fps,
};
pub use writer::write_interleaved_animation_xml;

pub use parsers::{
    AnimationRecord, BehaviorRecord, CharacterRecord, ProjectRecord, SkeletonRecord,
    parse_animation_xml_str, parse_behavior_graph_to_ui_json, parse_behavior_xml,
    parse_character_xml, parse_project_xml, parse_skeleton_xml,
};

use crate::error::{HavokError, HavokResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationCompression {
    Lossless,
    Spline,
    Interleaved,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimationMetadata {
    pub duration: f32,
    pub transform_track_count: usize,
    pub frame_count: usize,
    pub compression: AnimationCompression,
    pub float_track_count: usize,
    pub annotation_tracks: Vec<AnnotationTrack>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationTrack {
    pub name: String,
    pub annotations: Vec<Annotation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    pub time: f32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkeletonSummary {
    pub name: String,
    pub bones: Vec<String>,
    pub parent_indices: Vec<i32>,
}

pub fn parse_animation_metadata_from_tagxml(xml: &str) -> HavokResult<AnimationMetadata> {
    let document = parse_xml(xml)?;
    let animation = find_animation_object(&document).ok_or_else(|| {
        HavokError::InvalidInput("TagXML does not contain an hka animation object".to_string())
    })?;
    let class_name = animation.attribute("class").unwrap_or("");
    let transform_track_count =
        int_param(animation, "numberOfTransformTracks").unwrap_or(0) as usize;
    let frame_count = int_param(animation, "numberOfFrames")
        .or_else(|| int_param(animation, "numFrames"))
        .map(|value| value as usize)
        .unwrap_or_else(|| infer_interleaved_frame_count(animation, transform_track_count));

    Ok(AnimationMetadata {
        duration: float_param(animation, "duration").unwrap_or(0.0),
        transform_track_count,
        frame_count,
        compression: compression_from_class(class_name),
        float_track_count: int_param(animation, "numberOfFloatTracks").unwrap_or(0) as usize,
        annotation_tracks: parse_annotation_tracks(animation),
    })
}

pub fn extract_skeleton_from_tagxml(xml: &str) -> HavokResult<SkeletonSummary> {
    let document = parse_xml(xml)?;
    let skeleton = document
        .descendants()
        .find(|node| {
            node.has_tag_name("hkobject") && node.attribute("class") == Some("hkaSkeleton")
        })
        .ok_or_else(|| {
            HavokError::InvalidInput("TagXML does not contain hkaSkeleton".to_string())
        })?;

    let name = text_param(skeleton, "name").unwrap_or_default();
    let parent_indices = text_param(skeleton, "parentIndices")
        .map(|text| parse_i32_list(&text))
        .unwrap_or_default();
    let bones = child_param(skeleton, "bones")
        .map(|bones_param| {
            bones_param
                .children()
                .filter(|child| child.has_tag_name("hkobject"))
                .filter_map(|bone| text_param(bone, "name"))
                .collect()
        })
        .unwrap_or_default();

    Ok(SkeletonSummary {
        name,
        bones,
        parent_indices,
    })
}

fn parse_xml(xml: &str) -> HavokResult<roxmltree::Document<'_>> {
    roxmltree::Document::parse(xml)
        .map_err(|error| HavokError::InvalidInput(format!("invalid TagXML: {error}")))
}

fn find_animation_object<'a>(
    document: &'a roxmltree::Document<'a>,
) -> Option<roxmltree::Node<'a, 'a>> {
    document.descendants().find(|node| {
        if !node.has_tag_name("hkobject") {
            return false;
        }
        let class_name = node.attribute("class").unwrap_or("");
        class_name.starts_with("hka")
            && class_name.contains("Animation")
            && !matches!(class_name, "hkaAnimationContainer" | "hkaAnimationBinding")
    })
}

fn compression_from_class(class_name: &str) -> AnimationCompression {
    if class_name.contains("Lossless") {
        AnimationCompression::Lossless
    } else if class_name.contains("Spline") {
        AnimationCompression::Spline
    } else if class_name.contains("Interleaved") {
        AnimationCompression::Interleaved
    } else {
        AnimationCompression::Unknown
    }
}

fn infer_interleaved_frame_count(animation: roxmltree::Node<'_, '_>, track_count: usize) -> usize {
    if track_count == 0 {
        return 0;
    }
    let Some(transforms) = child_param(animation, "transforms") else {
        return 0;
    };
    if let Some(numelements) = transforms.attribute("numelements") {
        if let Ok(count) = numelements.parse::<usize>() {
            return count / track_count;
        }
    }
    transforms
        .text()
        .map(count_parenthesized_groups)
        .unwrap_or(0)
        / track_count
}

fn parse_annotation_tracks(animation: roxmltree::Node<'_, '_>) -> Vec<AnnotationTrack> {
    let Some(param) = child_param(animation, "annotationTracks") else {
        return Vec::new();
    };
    param
        .children()
        .filter(|child| child.has_tag_name("hkobject"))
        .map(|track| AnnotationTrack {
            name: text_param(track, "trackName").unwrap_or_default(),
            annotations: child_param(track, "annotations")
                .map(|annotations| {
                    annotations
                        .children()
                        .filter(|child| child.has_tag_name("hkobject"))
                        .map(|annotation| Annotation {
                            time: text_param(annotation, "time")
                                .and_then(|text| text.parse::<f32>().ok())
                                .unwrap_or(0.0),
                            text: text_param(annotation, "text").unwrap_or_default(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect()
}

fn child_param<'a>(node: roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> {
    node.children()
        .find(|child| child.has_tag_name("hkparam") && child.attribute("name") == Some(name))
}

fn text_param(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    child_param(node, name).and_then(|param| param.text().map(|text| text.trim().to_string()))
}

fn int_param(node: roxmltree::Node<'_, '_>, name: &str) -> Option<i64> {
    text_param(node, name).and_then(|text| text.parse::<i64>().ok())
}

fn float_param(node: roxmltree::Node<'_, '_>, name: &str) -> Option<f32> {
    text_param(node, name).and_then(|text| text.parse::<f32>().ok())
}

fn parse_i32_list(text: &str) -> Vec<i32> {
    text.split_whitespace()
        .filter_map(|value| value.parse::<i32>().ok())
        .collect()
}

fn count_parenthesized_groups(text: &str) -> usize {
    text.bytes().filter(|byte| *byte == b'(').count()
}
