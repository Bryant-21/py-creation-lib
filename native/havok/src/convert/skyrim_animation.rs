use std::collections::{BTreeMap, HashSet};

use crate::animation::clip::{AnimationClip, AnimationEvent, extract_clip};
use crate::animation::parsers::{
    AnimationRecord, SkeletonRecord, parse_animation_xml_str, parse_skeleton_xml,
};
use crate::error::{HavokError, HavokResult};
use crate::hkx::descriptors::DescriptorRegistry;
use crate::hkx::model::{HkxFile, HkxMember, HkxObject};
use crate::hkx::tagxml::write_tagxml_string;
use crate::hkx::types::HkxValue;

const SKYRIM_CONTENTS_VERSION: &str = "hk_2010.2.0-r1";
const FO4_CONTENTS_VERSION: &str = "hk_2014.1.0-r1";
const SKYRIM_CLASS_VERSION: u32 = 8;
const FO4_CLASS_VERSION: u32 = 11;

const CLIP_CLASSES: &[&str] = &[
    "hkRootLevelContainer",
    "hkaAnimationContainer",
    "hkaAnimationBinding",
    "hkaSplineCompressedAnimation",
    "hkaDefaultAnimatedReferenceFrame",
    "hkMemoryResourceContainer",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkyrimAnimationAssetKind {
    Skeleton,
    Clip,
}

#[derive(Debug)]
struct ClipSignature {
    animation: AnimationRecord,
    original_skeleton_name: String,
    is_additive: bool,
    track_to_bone_indices: Vec<u32>,
    extracted_motion_ref: String,
    decoded_channel_count: usize,
    events: Vec<AnimationEvent>,
}

/// Re-emit a decoded Skyrim 2010 skeleton or spline animation through the
/// canonical packfile writer using FO4 packfile metadata.
///
/// This route deliberately does not invoke or bypass the general 40 -> 53
/// patch manager. It accepts only the verified skeleton and spline-clip class
/// graphs and exports semantic TagXML for domain validation. Skeleton inputs
/// are reduced to their animation container and primary animation skeleton so
/// no Skyrim physics or ragdoll layout is converted. Every retained object is
/// then reconstructed from its FO4 class descriptor, including its target
/// signature and member set, before the graph is packed, reread, and audited.
pub fn reemit_skyrim_2010_animation_asset_to_fo4(data: &[u8]) -> HavokResult<Vec<u8>> {
    let source = HkxFile::read(data)?;
    validate_source_header(&source)?;
    let source_kind = classify_source(&source)?;
    let source_xml = write_tagxml_string(&source)?;
    let source_semantics = capture_semantics(source_kind, &source_xml)?;

    let retained_source = match (&source_kind, &source_semantics) {
        (SkyrimAnimationAssetKind::Skeleton, AssetSemantics::Skeleton(skeleton)) => {
            extract_animation_skeleton_graph(&source, &skeleton.name)?
        }
        (SkyrimAnimationAssetKind::Clip, AssetSemantics::Clip(_)) => source.clone(),
        _ => return Err(reemit_error("source semantic asset kind changed")),
    };
    let retained_xml = write_tagxml_string(&retained_source)?;
    let retained_semantics = capture_semantics(source_kind, &retained_xml)?;
    validate_semantics_preserved(&source_semantics, &retained_semantics)?;
    if source_kind == SkyrimAnimationAssetKind::Clip {
        validate_clip_graph(&retained_source)?;
    }
    let retained_classes = class_counts(&retained_source);

    let mut normalized_source = retained_source.clone();
    if source_kind == SkyrimAnimationAssetKind::Clip {
        normalize_clip_for_fo4(&mut normalized_source)?;
    }
    let target_graph = reconstruct_fo4_graph(&normalized_source)?;
    validate_target_descriptor_signatures(&target_graph)?;

    let output = target_graph.save();
    let target = HkxFile::read(&output)?;
    validate_target_header(&target)?;
    validate_target_descriptor_signatures(&target)?;
    if class_counts(&target) != retained_classes {
        return Err(reemit_error("target top-level class counts changed"));
    }
    validate_graph_preserved(&normalized_source, &target, "target packfile")?;
    if let AssetSemantics::Skeleton(skeleton) = &retained_semantics {
        validate_minimal_skeleton_graph(&target, &skeleton.name)?;
    }

    let target_kind = classify_source_graph(&target)?;
    if target_kind != source_kind {
        return Err(reemit_error("target asset kind changed"));
    }
    let target_xml = write_tagxml_string(&target)?;
    if target_kind == SkyrimAnimationAssetKind::Clip {
        validate_clip_graph(&target)?;
    }
    let target_semantics = capture_semantics(target_kind, &target_xml)?;
    validate_semantics_preserved(&retained_semantics, &target_semantics)?;

    if target.save() != output {
        return Err(reemit_error("target packfile did not round-trip unchanged"));
    }
    Ok(output)
}

fn validate_source_header(source: &HkxFile) -> HavokResult<()> {
    if source.class_version() != SKYRIM_CLASS_VERSION
        || source.contents_version() != SKYRIM_CONTENTS_VERSION
    {
        return Err(reemit_error(format!(
            "expected classversion {SKYRIM_CLASS_VERSION} {SKYRIM_CONTENTS_VERSION}, found classversion {} {}",
            source.class_version(),
            source.contents_version()
        )));
    }
    if source.packfile().header.pointer_size != 8 {
        return Err(reemit_error(format!(
            "expected 8-byte pointers, found {}",
            source.packfile().header.pointer_size
        )));
    }
    Ok(())
}

fn validate_target_header(target: &HkxFile) -> HavokResult<()> {
    if target.class_version() != FO4_CLASS_VERSION
        || target.contents_version() != FO4_CONTENTS_VERSION
        || target.packfile().header.pointer_size != 8
    {
        return Err(reemit_error(format!(
            "invalid target header: classversion {} contentsversion {} pointer_size {}",
            target.class_version(),
            target.contents_version(),
            target.packfile().header.pointer_size
        )));
    }
    Ok(())
}

fn classify_source(source: &HkxFile) -> HavokResult<SkyrimAnimationAssetKind> {
    let kind = classify_source_graph(source)?;
    if kind == SkyrimAnimationAssetKind::Clip {
        for object in source.objects() {
            if !CLIP_CLASSES.contains(&object.class_name.as_str()) {
                return Err(reemit_error(format!(
                    "unsupported {kind:?} source class {}",
                    object.class_name
                )));
            }
        }
    }
    validate_required_classes(source, kind)?;
    Ok(kind)
}

fn normalize_clip_for_fo4(file: &mut HkxFile) -> HavokResult<()> {
    let animation_index = unique_class_index(file, "hkaSplineCompressedAnimation")?;
    let animation = &mut file.objects_mut()[animation_index];
    let animation_type = animation
        .members
        .iter_mut()
        .find(|member| member.name == "type")
        .ok_or_else(|| reemit_error("hkaSplineCompressedAnimation.type is missing"))?;
    match &mut animation_type.value {
        HkxValue::I32(value) if *value == 5 => *value = 3,
        HkxValue::I32(value) => {
            return Err(reemit_error(format!(
                "expected Skyrim spline animation type 5, found {value}"
            )));
        }
        value => {
            return Err(reemit_error(format!(
                "hkaSplineCompressedAnimation.type has unexpected {} value",
                value.variant_name()
            )));
        }
    }
    Ok(())
}

fn validate_clip_graph(file: &HkxFile) -> HavokResult<()> {
    let animation_index = unique_class_index(file, "hkaSplineCompressedAnimation")?;
    let animation = &file.objects()[animation_index];
    let animation_duration = f32_member(&animation.members, "duration", &animation.class_name)?;
    let extracted_motion = animation
        .members
        .iter()
        .find(|member| member.name == "extractedMotion")
        .ok_or_else(|| reemit_error("hkaSplineCompressedAnimation.extractedMotion is missing"))?;
    let extracted_motion_index = match extracted_motion.value {
        HkxValue::Pointer(index) => index,
        ref value => {
            return Err(reemit_error(format!(
                "hkaSplineCompressedAnimation.extractedMotion has unexpected {} value",
                value.variant_name()
            )));
        }
    };

    let reference_frames = file
        .objects()
        .iter()
        .enumerate()
        .filter_map(|(index, object)| {
            (object.class_name == "hkaDefaultAnimatedReferenceFrame").then_some(index)
        })
        .collect::<Vec<_>>();
    match (extracted_motion_index, reference_frames.as_slice()) {
        (None, []) => return Ok(()),
        (None, _) => return Err(reemit_error("orphan animated reference-frame object")),
        (Some(_), []) => {
            return Err(reemit_error(
                "extractedMotion has no reference-frame object",
            ));
        }
        (Some(index), [reference_index]) if index == *reference_index => {}
        (Some(index), [reference_index]) => {
            return Err(reemit_error(format!(
                "extractedMotion references object {index}, not reference-frame object {reference_index}"
            )));
        }
        (Some(_), _) => return Err(reemit_error("multiple animated reference-frame objects")),
    }

    let reference = &file.objects()[reference_frames[0]];
    if file.contents_version() == SKYRIM_CONTENTS_VERSION && reference.signature != 0x6d85_e445 {
        return Err(reemit_error(format!(
            "Skyrim hkaDefaultAnimatedReferenceFrame has unexpected signature 0x{:08x}",
            reference.signature
        )));
    }
    let up = vector4_member(&reference.members, "up", &reference.class_name)?;
    let forward = vector4_member(&reference.members, "forward", &reference.class_name)?;
    validate_reference_axes(up, forward)?;
    let reference_duration = f32_member(&reference.members, "duration", &reference.class_name)?;
    if !reference_duration.is_finite()
        || reference_duration <= 0.0
        || reference_duration.to_bits() != animation_duration.to_bits()
    {
        return Err(reemit_error(format!(
            "reference-frame duration {reference_duration} does not match animation duration {animation_duration}"
        )));
    }
    let samples = array_member(
        &reference.members,
        "referenceFrameSamples",
        &reference.class_name,
    )?;
    if samples.is_empty() {
        return Err(reemit_error("animated reference frame has no samples"));
    }
    for (sample_index, sample) in samples.iter().enumerate() {
        let HkxValue::F32List(values) = sample else {
            return Err(reemit_error(format!(
                "referenceFrameSamples[{sample_index}] is not a vector4"
            )));
        };
        if values.len() != 4 || values.iter().any(|value| !value.is_finite()) {
            return Err(reemit_error(format!(
                "referenceFrameSamples[{sample_index}] is not a finite vector4"
            )));
        }
    }
    Ok(())
}

fn f32_member(members: &[HkxMember], name: &str, class_name: &str) -> HavokResult<f32> {
    let member = members
        .iter()
        .find(|member| member.name == name)
        .ok_or_else(|| reemit_error(format!("{class_name}.{name} is missing")))?;
    match member.value {
        HkxValue::F32(value) => Ok(value),
        ref value => Err(reemit_error(format!(
            "{class_name}.{name} has unexpected {} value",
            value.variant_name()
        ))),
    }
}

fn vector4_member<'a>(
    members: &'a [HkxMember],
    name: &str,
    class_name: &str,
) -> HavokResult<&'a [f32]> {
    let member = members
        .iter()
        .find(|member| member.name == name)
        .ok_or_else(|| reemit_error(format!("{class_name}.{name} is missing")))?;
    match &member.value {
        HkxValue::F32List(values) if values.len() == 4 => Ok(values),
        value => Err(reemit_error(format!(
            "{class_name}.{name} has unexpected {} value",
            value.variant_name()
        ))),
    }
}

fn validate_reference_axes(up: &[f32], forward: &[f32]) -> HavokResult<()> {
    if up.iter().chain(forward).any(|value| !value.is_finite()) || up[3] != 0.0 || forward[3] != 0.0
    {
        return Err(reemit_error(
            "reference-frame axes are not finite direction vectors",
        ));
    }
    let up_length = up[..3].iter().map(|value| value * value).sum::<f32>();
    let forward_length = forward[..3].iter().map(|value| value * value).sum::<f32>();
    let dot = up[..3]
        .iter()
        .zip(&forward[..3])
        .map(|(up, forward)| up * forward)
        .sum::<f32>();
    if (up_length - 1.0).abs() > 1.0e-4
        || (forward_length - 1.0).abs() > 1.0e-4
        || dot.abs() > 1.0e-4
    {
        return Err(reemit_error(
            "reference-frame up and forward axes are not unit and orthogonal",
        ));
    }
    Ok(())
}

fn classify_source_graph(source: &HkxFile) -> HavokResult<SkyrimAnimationAssetKind> {
    let has_skeleton = source
        .objects()
        .iter()
        .any(|object| object.class_name == "hkaSkeleton");
    let has_clip = source
        .objects()
        .iter()
        .any(|object| object.class_name == "hkaSplineCompressedAnimation");
    match (has_skeleton, has_clip) {
        (true, false) => Ok(SkyrimAnimationAssetKind::Skeleton),
        (false, true) => Ok(SkyrimAnimationAssetKind::Clip),
        (true, true) => Err(reemit_error("mixed skeleton and animation graph")),
        (false, false) => Err(reemit_error(
            "unsupported source graph; expected hkaSkeleton or hkaSplineCompressedAnimation",
        )),
    }
}

fn validate_required_classes(source: &HkxFile, kind: SkyrimAnimationAssetKind) -> HavokResult<()> {
    let counts = class_counts(source);
    require_class_count(&counts, "hkRootLevelContainer", 1)?;
    require_class_count(&counts, "hkaAnimationContainer", 1)?;
    match kind {
        SkyrimAnimationAssetKind::Skeleton => {
            if counts.get("hkaSkeleton").copied().unwrap_or(0) == 0 {
                return Err(reemit_error("skeleton graph has no hkaSkeleton"));
            }
        }
        SkyrimAnimationAssetKind::Clip => {
            require_class_count(&counts, "hkaSplineCompressedAnimation", 1)?;
            require_class_count(&counts, "hkaAnimationBinding", 1)?;
            require_class_count(&counts, "hkMemoryResourceContainer", 1)?;
        }
    }
    Ok(())
}

fn require_class_count(
    counts: &BTreeMap<String, usize>,
    class_name: &str,
    expected: usize,
) -> HavokResult<()> {
    let actual = counts.get(class_name).copied().unwrap_or(0);
    if actual != expected {
        return Err(reemit_error(format!(
            "expected {expected} {class_name} object(s), found {actual}"
        )));
    }
    Ok(())
}

fn class_counts(file: &HkxFile) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for object in file.objects() {
        *counts.entry(object.class_name.clone()).or_default() += 1;
    }
    counts
}

fn reconstruct_fo4_graph(source: &HkxFile) -> HavokResult<HkxFile> {
    let mut registry = DescriptorRegistry::for_contents_version(FO4_CONTENTS_VERSION);
    let mut objects = Vec::with_capacity(source.objects().len());
    for (index, source_object) in source.objects().iter().enumerate() {
        let descriptor = registry
            .get(&source_object.class_name)
            .map_err(|error| {
                reemit_error(format!(
                    "failed to load FO4 descriptor for {}: {error}",
                    source_object.class_name
                ))
            })?
            .cloned()
            .ok_or_else(|| {
                reemit_error(format!(
                    "FO4 descriptor unavailable for {}",
                    source_object.class_name
                ))
            })?;
        let signature = descriptor_signature(&descriptor.name, &descriptor.signature)?;
        let target_members = registry
            .get_all_members(&source_object.class_name)
            .map_err(|error| {
                reemit_error(format!(
                    "failed to resolve FO4 members for {}: {error}",
                    source_object.class_name
                ))
            })?;

        for source_member in &source_object.members {
            if !target_members
                .iter()
                .any(|target_member| target_member.name == source_member.name)
            {
                return Err(reemit_error(format!(
                    "{} source member {} has no FO4 descriptor member",
                    source_object.class_name, source_member.name
                )));
            }
        }

        let mut members = Vec::new();
        for target_member in target_members {
            if target_member.flags == "SERIALIZE_IGNORED" {
                continue;
            }
            if let Some(source_member) = source_object
                .members
                .iter()
                .find(|source_member| source_member.name == target_member.name)
            {
                members.push(source_member.clone());
            } else if let Some(default) = target_member.default {
                members.push(HkxMember {
                    name: target_member.name,
                    value: default,
                });
            } else {
                return Err(reemit_error(format!(
                    "{} FO4 member {} is unavailable in the decoded source object",
                    source_object.class_name, target_member.name
                )));
            }
        }

        objects.push(HkxObject {
            name: source_object.name.clone(),
            offset: index,
            signature,
            class_name: source_object.class_name.clone(),
            members,
        });
    }
    Ok(HkxFile::from_tagxml(
        FO4_CLASS_VERSION,
        FO4_CONTENTS_VERSION,
        objects,
    ))
}

fn validate_target_descriptor_signatures(file: &HkxFile) -> HavokResult<()> {
    let mut registry = DescriptorRegistry::for_contents_version(FO4_CONTENTS_VERSION);
    for object in file.objects() {
        let descriptor = registry
            .get(&object.class_name)
            .map_err(|error| {
                reemit_error(format!(
                    "failed to load FO4 descriptor for {}: {error}",
                    object.class_name
                ))
            })?
            .ok_or_else(|| {
                reemit_error(format!(
                    "FO4 descriptor unavailable for {}",
                    object.class_name
                ))
            })?;
        let expected = descriptor_signature(&descriptor.name, &descriptor.signature)?;
        if object.signature != expected {
            return Err(reemit_error(format!(
                "{} has signature 0x{:08x}; FO4 descriptor requires 0x{expected:08x}",
                object.class_name, object.signature
            )));
        }
    }
    Ok(())
}

fn descriptor_signature(class_name: &str, signature: &str) -> HavokResult<u32> {
    u32::from_str_radix(signature.trim_start_matches("0x"), 16).map_err(|error| {
        reemit_error(format!(
            "invalid FO4 descriptor signature {signature:?} for {class_name}: {error}"
        ))
    })
}

fn extract_animation_skeleton_graph(
    source: &HkxFile,
    expected_skeleton_name: &str,
) -> HavokResult<HkxFile> {
    let root_index = unique_class_index(source, "hkRootLevelContainer")?;
    let container_index = unique_class_index(source, "hkaAnimationContainer")?;
    validate_root_variant(source, root_index, container_index)?;
    let skeleton_index = animation_skeleton_index(source, container_index, expected_skeleton_name)?;

    let mut retained = source.clone();
    prune_root_variants(&mut retained, root_index, container_index)?;
    prune_animation_container(&mut retained, container_index, skeleton_index)?;

    let keep = HashSet::from([root_index, container_index, skeleton_index]);
    validate_pointer_closure(&retained, &keep, "retained source graph")?;
    retained.retain_objects_remap_pointers(|index, _| keep.contains(&index));
    validate_minimal_skeleton_graph(&retained, expected_skeleton_name)?;
    Ok(retained)
}

fn unique_class_index(file: &HkxFile, class_name: &str) -> HavokResult<usize> {
    let indices = file
        .objects()
        .iter()
        .enumerate()
        .filter_map(|(index, object)| (object.class_name == class_name).then_some(index))
        .collect::<Vec<_>>();
    if indices.len() != 1 {
        return Err(reemit_error(format!(
            "expected one {class_name} object, found {}",
            indices.len()
        )));
    }
    Ok(indices[0])
}

fn validate_root_variant(
    file: &HkxFile,
    root_index: usize,
    container_index: usize,
) -> HavokResult<()> {
    let variants = array_member(
        &file.objects()[root_index].members,
        "namedVariants",
        "hkRootLevelContainer",
    )?;
    let matching = variants
        .iter()
        .filter(|variant| animation_container_variant(variant, container_index))
        .count();
    if matching != 1 {
        return Err(reemit_error(format!(
            "expected one root animation-container variant, found {matching}"
        )));
    }
    Ok(())
}

fn animation_skeleton_index(
    file: &HkxFile,
    container_index: usize,
    expected_name: &str,
) -> HavokResult<usize> {
    let skeletons = array_member(
        &file.objects()[container_index].members,
        "skeletons",
        "hkaAnimationContainer",
    )?;
    let mut matching = Vec::new();
    for value in skeletons {
        let HkxValue::Pointer(Some(index)) = value else {
            return Err(reemit_error(
                "hkaAnimationContainer.skeletons contains a non-object reference",
            ));
        };
        let skeleton = file.objects().get(*index).ok_or_else(|| {
            reemit_error(format!(
                "hkaAnimationContainer.skeletons references missing object {index}"
            ))
        })?;
        if skeleton.class_name != "hkaSkeleton" {
            return Err(reemit_error(format!(
                "hkaAnimationContainer.skeletons references {}",
                skeleton.class_name
            )));
        }
        if string_member(&skeleton.members, "name") == Some(expected_name) {
            matching.push(*index);
        }
    }
    if matching.len() != 1 {
        return Err(reemit_error(format!(
            "expected one animation skeleton named {expected_name:?}, found {}",
            matching.len()
        )));
    }
    Ok(matching[0])
}

fn prune_root_variants(
    file: &mut HkxFile,
    root_index: usize,
    container_index: usize,
) -> HavokResult<()> {
    let variants = array_member_mut(
        &mut file.objects_mut()[root_index].members,
        "namedVariants",
        "hkRootLevelContainer",
    )?;
    variants.retain(|variant| animation_container_variant(variant, container_index));
    if variants.len() != 1 {
        return Err(reemit_error(
            "failed to retain root animation-container variant",
        ));
    }
    Ok(())
}

fn prune_animation_container(
    file: &mut HkxFile,
    container_index: usize,
    skeleton_index: usize,
) -> HavokResult<()> {
    let container = &mut file.objects_mut()[container_index];
    for member_name in ["animations", "bindings", "attachments", "skins"] {
        if !array_member(&container.members, member_name, "hkaAnimationContainer")?.is_empty() {
            return Err(reemit_error(format!(
                "skeleton hkaAnimationContainer.{member_name} is not empty"
            )));
        }
    }
    let skeletons = array_member_mut(&mut container.members, "skeletons", "hkaAnimationContainer")?;
    skeletons.retain(
        |value| matches!(value, HkxValue::Pointer(Some(index)) if *index == skeleton_index),
    );
    if skeletons.len() != 1 {
        return Err(reemit_error("failed to retain primary animation skeleton"));
    }
    Ok(())
}

fn validate_minimal_skeleton_graph(file: &HkxFile, expected_name: &str) -> HavokResult<()> {
    let counts = class_counts(file);
    if file.objects().len() != 3 {
        return Err(reemit_error(format!(
            "minimal animation skeleton graph has {} objects instead of 3",
            file.objects().len()
        )));
    }
    require_class_count(&counts, "hkRootLevelContainer", 1)?;
    require_class_count(&counts, "hkaAnimationContainer", 1)?;
    require_class_count(&counts, "hkaSkeleton", 1)?;
    if counts.keys().any(|class_name| {
        !matches!(
            class_name.as_str(),
            "hkRootLevelContainer" | "hkaAnimationContainer" | "hkaSkeleton"
        )
    }) {
        return Err(reemit_error(
            "minimal animation skeleton graph retained a forbidden class",
        ));
    }

    let root_index = unique_class_index(file, "hkRootLevelContainer")?;
    let container_index = unique_class_index(file, "hkaAnimationContainer")?;
    validate_root_variant(file, root_index, container_index)?;
    let _ = animation_skeleton_index(file, container_index, expected_name)?;
    let all_objects = (0..file.objects().len()).collect::<HashSet<_>>();
    validate_pointer_closure(file, &all_objects, "minimal animation skeleton graph")
}

fn validate_pointer_closure(
    file: &HkxFile,
    allowed: &HashSet<usize>,
    path: &str,
) -> HavokResult<()> {
    for &object_index in allowed {
        let object = file
            .objects()
            .get(object_index)
            .ok_or_else(|| reemit_error(format!("{path} retains missing object {object_index}")))?;
        for member in &object.members {
            validate_value_pointer_closure(
                &member.value,
                allowed,
                file.objects().len(),
                &format!("{path} object {object_index}.{}", member.name),
            )?;
        }
    }
    Ok(())
}

fn validate_value_pointer_closure(
    value: &HkxValue,
    allowed: &HashSet<usize>,
    object_count: usize,
    path: &str,
) -> HavokResult<()> {
    match value {
        HkxValue::Pointer(Some(index)) => {
            if *index >= object_count || !allowed.contains(index) {
                return Err(reemit_error(format!(
                    "{path} references object {index} outside the retained animation skeleton"
                )));
            }
        }
        HkxValue::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                validate_value_pointer_closure(
                    value,
                    allowed,
                    object_count,
                    &format!("{path}[{index}]"),
                )?;
            }
        }
        HkxValue::Object(members) | HkxValue::TypedObject { members, .. } => {
            for member in members {
                validate_value_pointer_closure(
                    &member.value,
                    allowed,
                    object_count,
                    &format!("{path}.{}", member.name),
                )?;
            }
        }
        HkxValue::PendingPtr(name) => {
            return Err(reemit_error(format!(
                "{path} retains unresolved pointer {name}"
            )));
        }
        _ => {}
    }
    Ok(())
}

fn animation_container_variant(value: &HkxValue, container_index: usize) -> bool {
    let Some(members) = value.as_object_members() else {
        return false;
    };
    string_member(members, "className") == Some("hkaAnimationContainer")
        && pointer_member(members, "variant") == Some(container_index)
}

fn array_member<'a>(
    members: &'a [HkxMember],
    name: &str,
    class_name: &str,
) -> HavokResult<&'a [HkxValue]> {
    let member = members
        .iter()
        .find(|member| member.name == name)
        .ok_or_else(|| reemit_error(format!("{class_name}.{name} is missing")))?;
    match &member.value {
        HkxValue::Array(values) => Ok(values),
        _ => Err(reemit_error(format!("{class_name}.{name} is not an array"))),
    }
}

fn array_member_mut<'a>(
    members: &'a mut [HkxMember],
    name: &str,
    class_name: &str,
) -> HavokResult<&'a mut Vec<HkxValue>> {
    let member = members
        .iter_mut()
        .find(|member| member.name == name)
        .ok_or_else(|| reemit_error(format!("{class_name}.{name} is missing")))?;
    match &mut member.value {
        HkxValue::Array(values) => Ok(values),
        _ => Err(reemit_error(format!("{class_name}.{name} is not an array"))),
    }
}

fn string_member<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a str> {
    members
        .iter()
        .find(|member| member.name == name)
        .and_then(|member| match &member.value {
            HkxValue::String {
                value,
                is_null: false,
            } => Some(value.as_str()),
            _ => None,
        })
}

fn pointer_member(members: &[HkxMember], name: &str) -> Option<usize> {
    members
        .iter()
        .find(|member| member.name == name)
        .and_then(|member| match &member.value {
            HkxValue::Pointer(Some(index)) => Some(*index),
            _ => None,
        })
}

enum AssetSemantics {
    Skeleton(SkeletonRecord),
    Clip(ClipSignature),
}

fn capture_semantics(kind: SkyrimAnimationAssetKind, xml: &str) -> HavokResult<AssetSemantics> {
    match kind {
        SkyrimAnimationAssetKind::Skeleton => {
            let skeleton = parse_skeleton_xml(xml)?;
            validate_skeleton(&skeleton)?;
            Ok(AssetSemantics::Skeleton(skeleton))
        }
        SkyrimAnimationAssetKind::Clip => {
            let animation = parse_animation_xml_str(xml)?;
            let clip = extract_clip(xml, None)?;
            let signature = ClipSignature {
                animation,
                original_skeleton_name: clip.original_skeleton_name.clone().unwrap_or_default(),
                is_additive: clip.is_additive,
                track_to_bone_indices: clip.track_to_bone_indices.clone(),
                extracted_motion_ref: clip.extracted_motion_ref.clone(),
                decoded_channel_count: clip.channels.len(),
                events: clip.events.clone(),
            };
            validate_clip(&signature, &clip)?;
            Ok(AssetSemantics::Clip(signature))
        }
    }
}

fn validate_skeleton(skeleton: &SkeletonRecord) -> HavokResult<()> {
    let count = skeleton.bone_count;
    if count == 0
        || skeleton.parent_indices.len() != count
        || skeleton.reference_pose.len() != count
        || skeleton.lock_translation.len() != count
    {
        return Err(reemit_error(format!(
            "invalid skeleton arrays: bones={count} parents={} reference_pose={} lock_translation={}",
            skeleton.parent_indices.len(),
            skeleton.reference_pose.len(),
            skeleton.lock_translation.len()
        )));
    }
    if skeleton.bone_names.iter().any(|name| name.is_empty()) {
        return Err(reemit_error("skeleton has an empty bone name"));
    }
    for (bone_index, parent) in skeleton.parent_indices.iter().copied().enumerate() {
        if parent >= bone_index as i32 || parent < -1 {
            return Err(reemit_error(format!(
                "bone {bone_index} has invalid parent index {parent}"
            )));
        }
    }
    for (bone_index, pose) in skeleton.reference_pose.iter().enumerate() {
        if pose
            .t
            .iter()
            .chain(&pose.q)
            .chain(&pose.s)
            .any(|value| !value.is_finite())
        {
            return Err(reemit_error(format!(
                "bone {bone_index} has a non-finite reference pose"
            )));
        }
    }
    Ok(())
}

fn validate_clip(signature: &ClipSignature, clip: &AnimationClip) -> HavokResult<()> {
    let track_count = signature.animation.bone_count;
    if signature.animation.compression_type != "spline"
        || track_count == 0
        || !signature.animation.duration.is_finite()
        || signature.animation.duration <= 0.0
        || signature.decoded_channel_count != track_count
    {
        return Err(reemit_error(format!(
            "invalid spline clip: compression={} tracks={track_count} decoded_channels={} duration={}",
            signature.animation.compression_type,
            signature.decoded_channel_count,
            signature.animation.duration
        )));
    }
    if signature.original_skeleton_name.is_empty() {
        return Err(reemit_error(
            "animation binding has no originalSkeletonName",
        ));
    }
    if signature.original_skeleton_name == "PairedRoot" {
        return Err(HavokError::UnportedEdgeCase {
            route: "skyrim_2010_animation_to_fo4".to_string(),
            edge_case: "paired_root_binding".to_string(),
            detail: "originalSkeletonName is PairedRoot; paired actor alignment requires the paired-animation pipeline".to_string(),
        });
    }
    let mapping = &signature.track_to_bone_indices;
    if !mapping.is_empty() {
        let unique = mapping.iter().copied().collect::<HashSet<_>>();
        if mapping.len() != track_count
            || unique.len() != mapping.len()
            || mapping.iter().any(|&index| index > i16::MAX as u32)
        {
            return Err(reemit_error(format!(
                "invalid transformTrackToBoneIndices for {track_count} tracks"
            )));
        }
    }
    if clip
        .events
        .iter()
        .any(|event| !event.time.is_finite() || event.text.is_empty())
    {
        return Err(reemit_error("clip has an invalid annotation"));
    }
    Ok(())
}

fn validate_semantics_preserved(
    source: &AssetSemantics,
    target: &AssetSemantics,
) -> HavokResult<()> {
    match (source, target) {
        (AssetSemantics::Skeleton(source), AssetSemantics::Skeleton(target)) => {
            if source != target {
                return Err(reemit_error("target skeleton semantics changed"));
            }
        }
        (AssetSemantics::Clip(source), AssetSemantics::Clip(target)) => {
            if source.animation.duration.to_bits() != target.animation.duration.to_bits()
                || source.animation.bone_count != target.animation.bone_count
                || source.animation.float_track_count != target.animation.float_track_count
                || source.animation.compression_type != target.animation.compression_type
                || !annotations_equal(
                    &source.animation.annotation_tracks,
                    &target.animation.annotation_tracks,
                )
                || source.original_skeleton_name != target.original_skeleton_name
                || source.is_additive != target.is_additive
                || source.track_to_bone_indices != target.track_to_bone_indices
                || source.extracted_motion_ref != target.extracted_motion_ref
                || source.decoded_channel_count != target.decoded_channel_count
                || !events_equal(&source.events, &target.events)
            {
                return Err(reemit_error("target animation semantics changed"));
            }
        }
        _ => return Err(reemit_error("target semantic asset kind changed")),
    }
    Ok(())
}

fn annotations_equal(
    source: &[crate::animation::parsers::AnnotationEntry],
    target: &[crate::animation::parsers::AnnotationEntry],
) -> bool {
    source.len() == target.len()
        && source.iter().zip(target).all(|(source, target)| {
            source.time.to_bits() == target.time.to_bits() && source.text == target.text
        })
}

fn events_equal(source: &[AnimationEvent], target: &[AnimationEvent]) -> bool {
    source.len() == target.len()
        && source.iter().zip(target).all(|(source, target)| {
            source.time.to_bits() == target.time.to_bits() && source.text == target.text
        })
}

fn validate_graph_preserved(
    source: &HkxFile,
    target: &HkxFile,
    target_name: &str,
) -> HavokResult<()> {
    if source.objects().len() != target.objects().len() {
        return Err(reemit_error(format!(
            "{target_name} object count changed from {} to {}",
            source.objects().len(),
            target.objects().len()
        )));
    }
    for (object_index, (source_object, target_object)) in
        source.objects().iter().zip(target.objects()).enumerate()
    {
        if source_object.class_name != target_object.class_name {
            return Err(reemit_error(format!(
                "{target_name} object {object_index} class changed from {} to {}",
                source_object.class_name, target_object.class_name
            )));
        }
        validate_members_preserved(
            &source_object.members,
            &target_object.members,
            &format!("{target_name} object {object_index}"),
        )?;
    }
    Ok(())
}

fn validate_members_preserved(
    source: &[HkxMember],
    target: &[HkxMember],
    path: &str,
) -> HavokResult<()> {
    for source_member in source {
        let target_member = target
            .iter()
            .find(|member| member.name == source_member.name)
            .ok_or_else(|| reemit_error(format!("{path}.{} was dropped", source_member.name)))?;
        validate_value_preserved(
            &source_member.value,
            &target_member.value,
            &format!("{path}.{}", source_member.name),
        )?;
    }
    Ok(())
}

fn validate_value_preserved(source: &HkxValue, target: &HkxValue, path: &str) -> HavokResult<()> {
    match (source, target) {
        (HkxValue::F32(source), HkxValue::F32(target)) if source.to_bits() == target.to_bits() => {}
        (HkxValue::Half(source), HkxValue::Half(target))
            if source.to_bits() == target.to_bits() => {}
        (HkxValue::F32List(source), HkxValue::F32List(target))
            if float_lists_semantically_equal(source, target) => {}
        (HkxValue::Array(source), HkxValue::Array(target)) if source.len() == target.len() => {
            for (index, (source, target)) in source.iter().zip(target).enumerate() {
                validate_value_preserved(source, target, &format!("{path}[{index}]"))?;
            }
        }
        (HkxValue::Object(source), HkxValue::Object(target)) => {
            validate_members_preserved(source, target, path)?;
        }
        (
            HkxValue::TypedObject {
                class_name: source_class,
                members: source,
            },
            HkxValue::TypedObject {
                class_name: target_class,
                members: target,
            },
        ) if source_class == target_class => validate_members_preserved(source, target, path)?,
        (HkxValue::F32(source), HkxValue::F32(target)) => {
            return Err(reemit_error(format!(
                "{path} changed from {source:?} (0x{:08x}) to {target:?} (0x{:08x})",
                source.to_bits(),
                target.to_bits()
            )));
        }
        (HkxValue::F32List(source), HkxValue::F32List(target)) => {
            return Err(reemit_error(format!(
                "{path} changed from {source:?} to {target:?}"
            )));
        }
        _ if source == target => {}
        _ => return Err(reemit_error(format!("{path} changed"))),
    }
    Ok(())
}

fn float_lists_semantically_equal(source: &[f32], target: &[f32]) -> bool {
    if source.len() != target.len() {
        return false;
    }
    if source
        .iter()
        .zip(target)
        .all(|(source, target)| source.to_bits() == target.to_bits())
    {
        return true;
    }
    source.len() == 12
        && [0, 1, 2, 4, 5, 6, 7, 8, 9, 10]
            .into_iter()
            .all(|index| source[index].to_bits() == target[index].to_bits())
}

fn reemit_error(message: impl Into<String>) -> HavokError {
    HavokError::InvalidInput(format!(
        "Skyrim 2010 animation semantic re-emitter: {}",
        message.into()
    ))
}
