use std::collections::HashMap;

use crate::error::{HavokError, HavokResult};

use super::descriptors::{DescriptorRegistry, MemberTemplate};
use super::model::{ArraySource, HkxMember, HkxObject};
use super::packfile::{ParsedPackfile, SectionHeader};
use super::types::{HkxType, HkxValue, deserialize_member_value};

const DATA_VIRTUAL_FIXUP_SECTION_INDEX: u32 = 0;
const SKYRIM_CHARACTER_DATA_SIGNATURE: u32 = 0x300d_6808;
const SKYRIM_CHARACTER_CONTROLLER_INFO_CLASS: &str = "hkbCharacterDataCharacterControllerInfo";
const SKYRIM_CLIP_GENERATOR_SIGNATURE: u32 = 0x333b_85b9;
const SKYRIM_BEHAVIOR_GRAPH_SIGNATURE: u32 = 0xb121_8f86;
const SKYRIM_BEHAVIOR_REFERENCE_GENERATOR_SIGNATURE: u32 = 0x0fcb_5423;
const SKYRIM_MANUAL_SELECTOR_GENERATOR_SIGNATURE: u32 = 0xd932_fab8;
const SKYRIM_BLENDER_GENERATOR_SIGNATURE: u32 = 0x22df_7147;
const SKYRIM_MODIFIER_GENERATOR_SIGNATURE: u32 = 0x1f81_fae6;
const SKYRIM_STATE_TAGGING_GENERATOR_SIGNATURE: u32 = 0xf082_6fc1;
const SKYRIM_STATE_MACHINE_SIGNATURE: u32 = 0x816c_1dcb;
const SKYRIM_STATE_MACHINE_STATE_INFO_SIGNATURE: u32 = 0x0ed7_f9d0;
const SKYRIM_BEHAVIOR_STRING_DATA_SIGNATURE: u32 = 0xc713_064e;
const SKYRIM_TRANSITION_INFO_ARRAY_SIGNATURE: u32 = 0xe397_b11e;
const SKYRIM_BEHAVIOR_GRAPH_DATA_SIGNATURE: u32 = 0x095a_ca5d;
const SKYRIM_BLENDING_TRANSITION_EFFECT_SIGNATURE: u32 = 0xfd85_84fe;
const SKYRIM_EVALUATE_EXPRESSION_MODIFIER_SIGNATURE: u32 = 0xf900_f6be;

#[derive(Debug, Default)]
pub struct ReadOutcome {
    pub objects: Vec<HkxObject>,
    pub array_sources: Vec<ArraySource>,
}

pub fn read_objects(data: &[u8], packfile: &ParsedPackfile) -> HavokResult<Vec<HkxObject>> {
    let mut registry = DescriptorRegistry::new();
    read_objects_with_registry(data, packfile, &mut registry)
}

pub fn read_objects_with_registry(
    data: &[u8],
    packfile: &ParsedPackfile,
    registry: &mut DescriptorRegistry,
) -> HavokResult<Vec<HkxObject>> {
    Ok(read_objects_with_sources(data, packfile, registry)?.objects)
}

pub fn read_objects_with_sources(
    data: &[u8],
    packfile: &ParsedPackfile,
    registry: &mut DescriptorRegistry,
) -> HavokResult<ReadOutcome> {
    let data_section = packfile
        .section("__data__")
        .ok_or_else(|| HavokError::InvalidInput("missing __data__ section".to_string()))?;
    let data_section_index = packfile
        .sections
        .iter()
        .position(|section| section.name == "__data__")
        .ok_or_else(|| HavokError::InvalidInput("missing __data__ section".to_string()))?;
    let mut object_stubs = Vec::new();

    for fixup in &packfile.virtual_fixups {
        let (class_name, signature) = resolve_class_entry(packfile, fixup.classname_offset)?;
        if fixup.section != DATA_VIRTUAL_FIXUP_SECTION_INDEX {
            return Err(HavokError::InvalidInput(
                "virtual fixup section must be __data__".to_string(),
            ));
        }
        packfile
            .sections
            .get(fixup.section as usize)
            .ok_or_else(|| {
                HavokError::InvalidInput("virtual fixup section out of bounds".to_string())
            })?;
        let offset = absolute_data_payload_offset(
            data_section,
            fixup.source as usize,
            "virtual fixup source",
        )?;
        object_stubs.push((offset, class_name, signature));
    }

    object_stubs.sort_by_key(|(offset, _, _)| *offset);
    object_stubs.dedup_by_key(|(offset, _, _)| *offset);

    let object_index_by_offset: HashMap<usize, usize> = object_stubs
        .iter()
        .enumerate()
        .map(|(index, (offset, _, _))| (*offset, index))
        .collect();
    let object_signature_by_offset: HashMap<usize, u32> = object_stubs
        .iter()
        .map(|(offset, _, signature)| (*offset, *signature))
        .collect();
    let local_fixups: HashMap<usize, usize> = packfile
        .local_fixups
        .iter()
        .map(|fixup| {
            let source = validate_data_payload_relative(
                data_section,
                fixup.source as usize,
                "local fixup source",
            )?;
            let target = absolute_data_payload_offset(
                data_section,
                fixup.target as usize,
                "local fixup target",
            )?;
            Ok((source, target))
        })
        .collect::<HavokResult<_>>()?;
    let global_fixups: HashMap<usize, usize> = packfile
        .global_fixups
        .iter()
        .map(|fixup| {
            let source = validate_data_payload_relative(
                data_section,
                fixup.source as usize,
                "global fixup source",
            )?;
            let target_section =
                packfile
                    .sections
                    .get(fixup.section as usize)
                    .ok_or_else(|| {
                        HavokError::InvalidInput("global fixup section out of bounds".to_string())
                    })?;
            if fixup.section as usize != data_section_index {
                return Err(HavokError::InvalidInput(
                    "global fixup target section must be __data__".to_string(),
                ));
            }
            let target = absolute_payload_offset(
                target_section,
                fixup.target as usize,
                "global fixup target",
            )?;
            Ok((source, target))
        })
        .collect::<HavokResult<_>>()?;

    let mut objects = Vec::with_capacity(object_stubs.len());
    let mut array_sources = Vec::new();
    let context = ReadContext {
        data,
        data_section,
        contents_version: &packfile.header.version_name,
        local_fixups: &local_fixups,
        global_fixups: &global_fixups,
        object_index_by_offset: &object_index_by_offset,
        object_signature_by_offset: &object_signature_by_offset,
    };

    for (object_index, (offset, class_name, signature)) in object_stubs.into_iter().enumerate() {
        ensure_len(data, offset, 0, "object offset")?;
        if is_legacy_2010(&context)
            && class_name == "hkbCharacterData"
            && signature != SKYRIM_CHARACTER_DATA_SIGNATURE
        {
            return Err(HavokError::FeatureNotImplemented {
                feature: format!(
                    "{} class layout hkbCharacterData signature 0x{signature:08x}",
                    context.contents_version
                ),
                reason: "only Skyrim's descriptor-proven 0x300d6808 legacy controller layout is supported"
                    .to_string(),
            });
        }
        let mut path = Vec::new();
        let members = read_members(
            registry,
            &context,
            offset,
            &class_name,
            object_index,
            &mut path,
            &mut array_sources,
        )?;
        objects.push(HkxObject {
            name: None,
            offset,
            signature,
            class_name,
            members,
        });
    }

    Ok(ReadOutcome {
        objects,
        array_sources,
    })
}

struct ReadContext<'a> {
    data: &'a [u8],
    data_section: &'a SectionHeader,
    contents_version: &'a str,
    local_fixups: &'a HashMap<usize, usize>,
    global_fixups: &'a HashMap<usize, usize>,
    object_index_by_offset: &'a HashMap<usize, usize>,
    object_signature_by_offset: &'a HashMap<usize, u32>,
}

fn read_members(
    registry: &mut DescriptorRegistry,
    context: &ReadContext<'_>,
    base: usize,
    class_name: &str,
    object_index: usize,
    path: &mut Vec<String>,
    array_sources: &mut Vec<ArraySource>,
) -> HavokResult<Vec<HkxMember>> {
    let members = registry
        .get_all_members(class_name)
        .map_err(|error| HavokError::InvalidInput(error.to_string()))?;
    let mut values = Vec::with_capacity(members.len());

    for member in members {
        if member.flags == "SERIALIZE_IGNORED" {
            continue;
        }
        if is_skyrim_character_controller_info(context, base, class_name, &member) {
            values.push(HkxMember {
                name: "characterControllerInfo".to_string(),
                value: read_skyrim_character_controller_info(context, base)?,
            });
            continue;
        }
        if is_absent_legacy_member(context, base, class_name, &member) {
            values.push(HkxMember {
                name: member.name,
                value: HkxValue::Array(Vec::new()),
            });
            continue;
        }
        let member_offset =
            legacy_member_offset(context, base, class_name, &member).unwrap_or(member.offset);
        let offset = checked_add(base, member_offset, "member offset")?;
        path.push(member.name.clone());
        let value = read_member_value(
            registry,
            context,
            offset,
            class_name,
            &member,
            object_index,
            path,
            array_sources,
        )?;
        path.pop();
        values.push(HkxMember {
            name: member.name,
            value,
        });
    }

    Ok(values)
}

fn read_member_value(
    registry: &mut DescriptorRegistry,
    context: &ReadContext<'_>,
    offset: usize,
    owner_class: &str,
    member: &MemberTemplate,
    object_index: usize,
    path: &mut Vec<String>,
    array_sources: &mut Vec<ArraySource>,
) -> HavokResult<HkxValue> {
    if member.arrsize > 0
        && !matches!(
            member.vtype,
            HkxType::Array | HkxType::SimpleArray | HkxType::RelArray
        )
    {
        return read_fixed_array(
            registry,
            context,
            offset,
            member,
            object_index,
            path,
            array_sources,
        );
    }

    match member.vtype {
        HkxType::Array | HkxType::SimpleArray => read_array(
            registry,
            context,
            offset,
            owner_class,
            member,
            object_index,
            path,
            array_sources,
        ),
        HkxType::RelArray => read_rel_array(
            registry,
            context,
            offset,
            member,
            object_index,
            path,
            array_sources,
        ),
        HkxType::Pointer | HkxType::FunctionPointer => Ok(read_pointer(context, offset)),
        HkxType::CString | HkxType::StringPtr => read_string(context, offset),
        HkxType::Struct => read_inline_struct(
            registry,
            context,
            offset,
            &member.ctype,
            object_index,
            path,
            array_sources,
        ),
        _ => read_direct_value(context.data, offset, member),
    }
}

fn read_fixed_array(
    registry: &mut DescriptorRegistry,
    context: &ReadContext<'_>,
    offset: usize,
    member: &MemberTemplate,
    object_index: usize,
    path: &mut Vec<String>,
    array_sources: &mut Vec<ArraySource>,
) -> HavokResult<HkxValue> {
    let element = MemberTemplate {
        name: member.name.clone(),
        offset: 0,
        vtype: member.vtype,
        vsubtype: member.vsubtype,
        ctype: member.ctype.clone(),
        arrsize: 0,
        flags: member.flags.clone(),
        etype: member.etype.clone(),
        default: None,
    };
    let stride = member_value_size_for_stride(registry, &element)?;
    let total = stride
        .checked_mul(member.arrsize)
        .ok_or_else(|| HavokError::InvalidInput("fixed array byte length overflows".to_string()))?;
    ensure_len(context.data, offset, total, "fixed array")?;

    let mut values = Vec::with_capacity(member.arrsize);
    for index in 0..member.arrsize {
        let element_offset = offset + index * stride;
        values.push(read_member_value(
            registry,
            context,
            element_offset,
            "",
            &element,
            object_index,
            path,
            array_sources,
        )?);
    }
    Ok(HkxValue::Array(values))
}

fn read_array(
    registry: &mut DescriptorRegistry,
    context: &ReadContext<'_>,
    offset: usize,
    owner_class: &str,
    member: &MemberTemplate,
    object_index: usize,
    path: &mut Vec<String>,
    array_sources: &mut Vec<ArraySource>,
) -> HavokResult<HkxValue> {
    ensure_len(context.data, offset, member.vtype.size(), "array header")?;
    let count = read_i32(context.data, offset + 8, "array size")?;
    if count < 0 {
        return Err(HavokError::InvalidInput(format!(
            "negative array size for {owner_class}.{}",
            member.name
        )));
    }
    let count = count as usize;
    let source = section_relative(context, offset)?;
    let Some(data_offset) = context.local_fixups.get(&source).copied() else {
        if count == 0 {
            return Ok(HkxValue::Array(Vec::new()));
        }
        if is_legacy_2010(context) {
            return Err(HavokError::FeatureNotImplemented {
                feature: format!(
                    "{} class layout {owner_class}.{}",
                    context.contents_version, member.name
                ),
                reason: format!(
                    "non-empty array has no local fixup at __data__+0x{source:x}; a version-specific descriptor is required"
                ),
            });
        }
        return Err(HavokError::InvalidInput(format!(
            "missing local fixup for non-empty array {owner_class}.{}",
            member.name
        )));
    };

    let stride = legacy_array_element_stride(context, owner_class, member)
        .unwrap_or(element_stride(registry, member)?);

    // Cap count to the remaining data length to prevent gigabyte allocations
    // from malformed files before the overflow check below runs.
    let max_count = context.data.len().saturating_sub(data_offset) / stride.max(1);
    if count > max_count {
        return Err(HavokError::InvalidInput(format!(
            "array count {count} exceeds available data for {owner_class}.{}",
            member.name
        )));
    }

    let total = stride
        .checked_mul(count)
        .ok_or_else(|| HavokError::InvalidInput("array byte length overflows".to_string()))?;
    ensure_len(context.data, data_offset, total, "array payload")?;

    array_sources.push(ArraySource {
        object_index,
        member_path: path.clone(),
        content_offset: data_offset,
        content_length: total,
        element_subtype: member.vsubtype,
        ctype: member.ctype.clone(),
    });

    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        let element_offset = data_offset + index * stride;
        values.push(read_array_element(
            registry,
            context,
            element_offset,
            member,
            object_index,
            path,
            index,
            array_sources,
        )?);
    }
    Ok(HkxValue::Array(values))
}

fn read_rel_array(
    registry: &mut DescriptorRegistry,
    context: &ReadContext<'_>,
    offset: usize,
    member: &MemberTemplate,
    object_index: usize,
    path: &mut Vec<String>,
    array_sources: &mut Vec<ArraySource>,
) -> HavokResult<HkxValue> {
    ensure_len(
        context.data,
        offset,
        HkxType::RelArray.size(),
        "relative array header",
    )?;
    let count = read_u16(context.data, offset, "relative array size")? as usize;
    let offset_value = read_u16(context.data, offset + 2, "relative array offset")? as usize;
    if count == 0 {
        return Ok(HkxValue::Array(Vec::new()));
    }

    let data_offset = checked_add(offset, offset_value, "relative array payload")?;
    let stride = element_stride(registry, member)?;
    let total = stride.checked_mul(count).ok_or_else(|| {
        HavokError::InvalidInput("relative array byte length overflows".to_string())
    })?;
    ensure_len(context.data, data_offset, total, "relative array payload")?;

    array_sources.push(ArraySource {
        object_index,
        member_path: path.clone(),
        content_offset: data_offset,
        content_length: total,
        element_subtype: member.vsubtype,
        ctype: member.ctype.clone(),
    });

    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        values.push(read_array_element(
            registry,
            context,
            data_offset + index * stride,
            member,
            object_index,
            path,
            index,
            array_sources,
        )?);
    }
    Ok(HkxValue::Array(values))
}

fn read_array_element(
    registry: &mut DescriptorRegistry,
    context: &ReadContext<'_>,
    offset: usize,
    member: &MemberTemplate,
    object_index: usize,
    path: &mut Vec<String>,
    element_index: usize,
    array_sources: &mut Vec<ArraySource>,
) -> HavokResult<HkxValue> {
    match member.vsubtype {
        HkxType::Pointer | HkxType::FunctionPointer => Ok(read_pointer(context, offset)),
        HkxType::CString | HkxType::StringPtr => read_string(context, offset),
        HkxType::Struct => {
            // Recurse into struct elements so their nested arrays get tracked.
            // Use a synthetic path component so the patcher can address each
            // element independently. The patcher itself walks struct elements
            // separately and never patches the array's struct body in place.
            path.push(format!("[{element_index}]"));
            let result = read_inline_struct(
                registry,
                context,
                offset,
                &member.ctype,
                object_index,
                path,
                array_sources,
            );
            path.pop();
            result
        }
        _ => {
            let element = MemberTemplate {
                name: member.name.clone(),
                offset: 0,
                vtype: member.vsubtype,
                vsubtype: HkxType::Void,
                ctype: member.ctype.clone(),
                arrsize: 0,
                flags: member.flags.clone(),
                etype: member.etype.clone(),
                default: None,
            };
            read_direct_value(context.data, offset, &element)
        }
    }
}

fn read_inline_struct(
    registry: &mut DescriptorRegistry,
    context: &ReadContext<'_>,
    offset: usize,
    class_name: &str,
    object_index: usize,
    path: &mut Vec<String>,
    array_sources: &mut Vec<ArraySource>,
) -> HavokResult<HkxValue> {
    if class_name.is_empty() {
        return Ok(HkxValue::Object(Vec::new()));
    }
    let size = struct_stride(registry, class_name)?;
    ensure_len(context.data, offset, size, "inline struct")?;
    Ok(HkxValue::Object(read_members(
        registry,
        context,
        offset,
        class_name,
        object_index,
        path,
        array_sources,
    )?))
}

fn read_pointer(context: &ReadContext<'_>, offset: usize) -> HkxValue {
    let Ok(source) = section_relative(context, offset) else {
        return HkxValue::Pointer(None);
    };
    let target_index = context
        .global_fixups
        .get(&source)
        .and_then(|target| context.object_index_by_offset.get(target))
        .copied();
    HkxValue::Pointer(target_index)
}

fn read_string(context: &ReadContext<'_>, offset: usize) -> HavokResult<HkxValue> {
    let source = section_relative(context, offset)?;
    let Some(target) = context.local_fixups.get(&source).copied() else {
        return Ok(HkxValue::String {
            value: String::new(),
            is_null: true,
        });
    };
    Ok(HkxValue::String {
        value: read_c_string(context.data, target)?,
        is_null: false,
    })
}

fn read_direct_value(data: &[u8], offset: usize, member: &MemberTemplate) -> HavokResult<HkxValue> {
    let size = member_value_size(member);
    ensure_len(data, offset, size, "member value")?;
    deserialize_member_value(member.vtype, member.vsubtype, &data[offset..offset + size])
        .ok_or_else(|| HavokError::InvalidInput(format!("unsupported member {}", member.name)))
}

fn resolve_class_entry(
    packfile: &ParsedPackfile,
    classname_offset: u32,
) -> HavokResult<(String, u32)> {
    packfile
        .classnames
        .iter()
        .find(|entry| entry.position == classname_offset as usize)
        .map(|entry| (entry.name.clone(), entry.signature))
        .ok_or_else(|| {
            HavokError::InvalidInput(format!("unknown classname offset {classname_offset}"))
        })
}

fn element_stride(
    registry: &mut DescriptorRegistry,
    member: &MemberTemplate,
) -> HavokResult<usize> {
    match member.vsubtype {
        HkxType::Struct => struct_stride(registry, &member.ctype),
        HkxType::Enum | HkxType::Flags => Ok(member.vsubtype.size()),
        _ => Ok(member.vsubtype.size()),
    }
}

fn is_legacy_2010(context: &ReadContext<'_>) -> bool {
    context.contents_version == "hk_2010.2.0-r1"
}

fn is_skyrim_character_controller_info(
    context: &ReadContext<'_>,
    base: usize,
    owner_class: &str,
    member: &MemberTemplate,
) -> bool {
    is_legacy_2010(context)
        && owner_class == "hkbCharacterData"
        && member.name == "characterControllerSetup"
        && context.object_signature_by_offset.get(&base) == Some(&SKYRIM_CHARACTER_DATA_SIGNATURE)
}

fn read_skyrim_character_controller_info(
    context: &ReadContext<'_>,
    character_base: usize,
) -> HavokResult<HkxValue> {
    // Signature 0x300d6808 is hkbCharacterData v7. Its reflected inline
    // controller is the 24-byte 0xa0f415bf struct, before v9->v10 replaced it
    // with hkbCharacterControllerSetup. The corpus fixup at +0x20 proves the
    // nullable cinfo slot; the next 16-byte boundary begins modelUpMS.
    let base = checked_add(character_base, 16, "Skyrim character controller info")?;
    ensure_len(context.data, base, 24, "Skyrim character controller info")?;
    Ok(HkxValue::TypedObject {
        class_name: SKYRIM_CHARACTER_CONTROLLER_INFO_CLASS.to_string(),
        members: vec![
            HkxMember {
                name: "capsuleHeight".to_string(),
                value: HkxValue::F32(read_f32(context.data, base, "Skyrim capsule height")?),
            },
            HkxMember {
                name: "capsuleRadius".to_string(),
                value: HkxValue::F32(read_f32(context.data, base + 4, "Skyrim capsule radius")?),
            },
            HkxMember {
                name: "collisionFilterInfo".to_string(),
                value: HkxValue::U32(read_u32(
                    context.data,
                    base + 8,
                    "Skyrim controller collision filter",
                )?),
            },
            HkxMember {
                name: "characterControllerCinfo".to_string(),
                value: read_pointer(context, base + 16),
            },
        ],
    })
}

// The bundled pre-FO4 descriptors begin at Havok 2012. Vanilla Skyrim's
// object boundaries and local-fixup topology prove these 2010 layouts without
// requiring a speculative full 2010 descriptor set: the mirrored-skeleton
// object ends at partitionPairMap's 2012 offset, the mapper payload begins
// after its shorter v40 body, and BlenderGenerator.children has a fixup 64
// bytes before its 2012 offset.
fn is_absent_legacy_member(
    context: &ReadContext<'_>,
    _base: usize,
    owner_class: &str,
    member: &MemberTemplate,
) -> bool {
    if !is_legacy_2010(context) || member.vtype != HkxType::Array {
        return false;
    }

    match (owner_class, member.name.as_str()) {
        ("hkbAssetBundleStringData", "assetNames")
        | ("hkaSkeleton", "partitions")
        | ("hkbMirroredSkeletonInfo", "partitionPairMap")
        | ("hkaAnimationBinding", "partitionIndices")
        | ("hkaSkeletonMapperData", "simpleMappingPartitionRanges")
        | ("hkaSkeletonMapperData", "chainMappingPartitionRanges")
        | ("hkaSkeletonMapperData", "simpleMappings")
        | ("hkbCharacterData", "aiControlDriverInfo")
        | ("hkbCharacterData", "boneAttachmentBoneIndices")
        | ("hkbCharacterData", "boneAttachmentTransforms") => true,
        _ => false,
    }
}

fn legacy_member_offset(
    context: &ReadContext<'_>,
    base: usize,
    owner_class: &str,
    member: &MemberTemplate,
) -> Option<usize> {
    if !is_legacy_2010(context) {
        return None;
    }

    let signature = context.object_signature_by_offset.get(&base).copied();
    if let Some(offset) =
        skyrim_behavior_member_offset(signature, owner_class, &member.name, member.offset)
    {
        return Some(offset);
    }

    match (owner_class, member.name.as_str()) {
        ("hkbCharacterData", "modelUpMS" | "modelForwardMS" | "modelRightMS") => {
            member.offset.checked_sub(16)
        }
        (
            "hkbCharacterData",
            "characterPropertyInfos"
            | "numBonesPerLod"
            | "characterPropertyValues"
            | "footIkDriverInfo"
            | "handIkDriverInfo",
        ) => member.offset.checked_sub(16),
        ("hkbCharacterData", "stringData" | "mirroredSkeletonInfo") => {
            member.offset.checked_sub(24)
        }
        ("hkbCharacterData", "scale") => member.offset.checked_sub(56),
        (
            "hkaDefaultAnimatedReferenceFrame",
            "up" | "forward" | "duration" | "referenceFrameSamples",
        ) => member.offset.checked_sub(16),
        ("hkaAnimationBinding", "blendHint") => member.offset.checked_sub(16),
        ("BSLookAtModifier", "bones") => Some(0x58),
        ("BSLookAtModifier", "eyeBones") => Some(0x68),
        ("hkbFootIkControlsModifier", "legs") => Some(0x80),
        ("hkbFootIkModifier", "legs") => Some(0x80),
        ("hkbPoseMatchingGenerator", "children") => Some(0x60),
        ("hkbModifierList", "modifiers") if member.offset == 88 => Some(80),
        ("hkbModifierList", "modifiers") => member.offset.checked_sub(8),
        ("hkaSkeletonMapperData", "chainMappings" | "unmappedBones") => {
            member.offset.checked_sub(48)
        }
        (
            "hkaSkeletonMapperData",
            "extractedMotionMapping" | "keepUnmappedLocal" | "mappingType",
        ) => member.offset.checked_sub(48),
        _ => None,
    }
}

fn skyrim_behavior_member_offset(
    signature: Option<u32>,
    owner_class: &str,
    member_name: &str,
    descriptor_offset: usize,
) -> Option<usize> {
    match (signature, owner_class, member_name) {
        (Some(SKYRIM_BEHAVIOR_GRAPH_SIGNATURE), "hkbBehaviorGraph", "rootGenerator") => Some(128),
        (Some(SKYRIM_BEHAVIOR_GRAPH_SIGNATURE), "hkbBehaviorGraph", "data") => Some(136),
        (
            Some(SKYRIM_BEHAVIOR_GRAPH_DATA_SIGNATURE),
            "hkbBehaviorGraphData",
            "variableInitialValues",
        ) => Some(112),
        (Some(SKYRIM_BEHAVIOR_GRAPH_DATA_SIGNATURE), "hkbBehaviorGraphData", "stringData") => {
            Some(120)
        }
        (
            Some(SKYRIM_BLENDING_TRANSITION_EFFECT_SIGNATURE),
            "hkbBlendingTransitionEffect",
            "selfTransitionMode",
        ) => Some(72),
        (
            Some(SKYRIM_BLENDING_TRANSITION_EFFECT_SIGNATURE),
            "hkbBlendingTransitionEffect",
            "eventMode",
        ) => Some(73),
        (
            Some(SKYRIM_BLENDING_TRANSITION_EFFECT_SIGNATURE),
            "hkbBlendingTransitionEffect",
            "duration",
        ) => Some(80),
        (
            Some(SKYRIM_BLENDING_TRANSITION_EFFECT_SIGNATURE),
            "hkbBlendingTransitionEffect",
            "toGeneratorStartTimeFraction",
        ) => Some(84),
        (
            Some(SKYRIM_BLENDING_TRANSITION_EFFECT_SIGNATURE),
            "hkbBlendingTransitionEffect",
            "flags",
        ) => Some(88),
        (
            Some(SKYRIM_BLENDING_TRANSITION_EFFECT_SIGNATURE),
            "hkbBlendingTransitionEffect",
            "endMode",
        ) => Some(90),
        (
            Some(SKYRIM_BLENDING_TRANSITION_EFFECT_SIGNATURE),
            "hkbBlendingTransitionEffect",
            "blendCurve",
        ) => Some(91),
        (
            Some(SKYRIM_BLENDING_TRANSITION_EFFECT_SIGNATURE),
            "hkbBlendingTransitionEffect",
            "alignmentBone",
        ) => Some(92),
        (
            Some(SKYRIM_EVALUATE_EXPRESSION_MODIFIER_SIGNATURE),
            "hkbEvaluateExpressionModifier",
            "expressions",
        ) => Some(80),
        (
            Some(SKYRIM_BEHAVIOR_REFERENCE_GENERATOR_SIGNATURE),
            "hkbBehaviorReferenceGenerator",
            "behaviorName",
        ) => Some(72),
        (
            Some(SKYRIM_MANUAL_SELECTOR_GENERATOR_SIGNATURE),
            "hkbManualSelectorGenerator",
            "generators"
            | "selectedGeneratorIndex"
            | "indexSelector"
            | "selectedIndexCanChangeAfterActivate",
        ) => descriptor_offset.checked_sub(64),
        (
            Some(SKYRIM_BLENDER_GENERATOR_SIGNATURE),
            "hkbBlenderGenerator",
            "referencePoseWeightThreshold"
            | "blendParameter"
            | "minCyclicBlendParameter"
            | "maxCyclicBlendParameter"
            | "indexOfSyncMasterChild"
            | "flags"
            | "subtractLastChild"
            | "children",
        ) => descriptor_offset.checked_sub(64),
        (Some(SKYRIM_MODIFIER_GENERATOR_SIGNATURE), "hkbModifierGenerator", "modifier") => Some(72),
        (Some(SKYRIM_MODIFIER_GENERATOR_SIGNATURE), "hkbModifierGenerator", "generator") => {
            Some(80)
        }
        (
            Some(SKYRIM_STATE_TAGGING_GENERATOR_SIGNATURE),
            "BSiStateTaggingGenerator",
            "pDefaultGenerator",
        ) => Some(80),
        (
            Some(SKYRIM_CLIP_GENERATOR_SIGNATURE),
            "hkbClipGenerator",
            "animationBundleName"
            | "animationName"
            | "triggers"
            | "userPartitionMask"
            | "cropStartAmountLocalTime"
            | "cropEndAmountLocalTime"
            | "startTime"
            | "playbackSpeed"
            | "enforcedDuration"
            | "userControlledTimeFraction"
            | "animationBindingIndex"
            | "mode"
            | "flags",
        ) => descriptor_offset.checked_sub(72),
        (
            Some(SKYRIM_STATE_MACHINE_SIGNATURE),
            "hkbStateMachine",
            "eventToSendWhenStateOrTransitionChanges"
            | "startStateIdSelector"
            | "startStateId"
            | "returnToPreviousStateEventId"
            | "randomTransitionEventId"
            | "transitionToNextHigherStateEventId"
            | "transitionToNextLowerStateEventId"
            | "syncVariableIndex"
            | "wrapAroundStateId"
            | "maxSimultaneousTransitions"
            | "startStateMode"
            | "selfTransitionMode"
            | "states"
            | "wildcardTransitions",
        ) => descriptor_offset.checked_sub(64),
        (
            Some(SKYRIM_STATE_MACHINE_STATE_INFO_SIGNATURE),
            "hkbStateMachineStateInfo",
            "listeners"
            | "enterNotifyEvents"
            | "exitNotifyEvents"
            | "transitions"
            | "generator"
            | "name"
            | "stateId"
            | "probability"
            | "enable"
            | "hasEventlessTransitions",
        ) => Some(descriptor_offset),
        (
            Some(SKYRIM_BEHAVIOR_STRING_DATA_SIGNATURE),
            "hkbBehaviorGraphStringData",
            "eventNames" | "attributeNames" | "variableNames" | "characterPropertyNames",
        ) => Some(descriptor_offset),
        (
            Some(SKYRIM_TRANSITION_INFO_ARRAY_SIGNATURE),
            "hkbStateMachineTransitionInfoArray",
            "transitions" | "hasEventlessTransitions" | "hasTimeBoundedTransitions",
        ) => Some(descriptor_offset),
        _ => None,
    }
}

fn legacy_array_element_stride(
    context: &ReadContext<'_>,
    owner_class: &str,
    member: &MemberTemplate,
) -> Option<usize> {
    if !is_legacy_2010(context) {
        return None;
    }
    match (owner_class, member.ctype.as_str()) {
        (_, "hkbAssetBundleStringData") if member.vsubtype == HkxType::Struct => {
            Some(HkxType::StringPtr.size())
        }
        ("BSLookAtModifier", "BSLookAtModifierBoneData") => Some(64),
        ("hkbFootIkControlsModifier", "hkbFootIkControlsModifierLeg") => Some(48),
        ("hkbFootIkModifier", "hkbFootIkModifierLeg") => Some(160),
        _ => None,
    }
}

fn struct_stride(registry: &mut DescriptorRegistry, class_name: &str) -> HavokResult<usize> {
    let members = registry
        .get_all_members(class_name)
        .map_err(|error| HavokError::InvalidInput(error.to_string()))?;
    let mut size = 0;
    let mut alignment = 1;
    for member in &members {
        let member_size = member_value_size_for_stride(registry, member)?;
        size = size.max(member.offset + member_size);
        alignment = alignment.max(member_alignment(registry, member)?);
    }
    Ok(align_up(size, alignment.max(1)))
}

fn member_value_size_for_stride(
    registry: &mut DescriptorRegistry,
    member: &MemberTemplate,
) -> HavokResult<usize> {
    let element_size = if member.vtype == HkxType::Struct && !member.ctype.is_empty() {
        struct_stride(registry, &member.ctype)?
    } else {
        member_value_size(member)
    };
    element_size
        .checked_mul(member.arrsize.max(1))
        .ok_or_else(|| HavokError::InvalidInput("member byte span overflows".to_string()))
}

fn member_value_size(member: &MemberTemplate) -> usize {
    match member.vtype {
        HkxType::Enum | HkxType::Flags => member.vsubtype.size(),
        _ => member.vtype.size(),
    }
}

fn member_alignment(
    registry: &mut DescriptorRegistry,
    member: &MemberTemplate,
) -> HavokResult<usize> {
    Ok(match member.vtype {
        HkxType::Vector4
        | HkxType::Quaternion
        | HkxType::Matrix3
        | HkxType::Matrix4
        | HkxType::Transform
        | HkxType::QsTransform => 16,
        HkxType::Pointer
        | HkxType::FunctionPointer
        | HkxType::CString
        | HkxType::StringPtr
        | HkxType::Array
        | HkxType::SimpleArray
        | HkxType::Int64
        | HkxType::Uint64 => 8,
        HkxType::Int16 | HkxType::Uint16 | HkxType::Half => 2,
        HkxType::Bool | HkxType::Int8 | HkxType::Uint8 => 1,
        HkxType::Enum | HkxType::Flags => member.vsubtype.size().clamp(1, 8),
        HkxType::Struct if !member.ctype.is_empty() => struct_alignment(registry, &member.ctype)?,
        _ => member.vtype.size().clamp(1, 16),
    })
}

fn struct_alignment(registry: &mut DescriptorRegistry, class_name: &str) -> HavokResult<usize> {
    let members = registry
        .get_all_members(class_name)
        .map_err(|error| HavokError::InvalidInput(error.to_string()))?;
    let mut alignment = 1;
    for member in &members {
        alignment = alignment.max(member_alignment(registry, member)?);
    }
    Ok(alignment)
}

fn section_relative(context: &ReadContext<'_>, offset: usize) -> HavokResult<usize> {
    offset
        .checked_sub(context.data_section.offset)
        .ok_or_else(|| HavokError::InvalidInput("offset before data section".to_string()))
}

fn ensure_len(data: &[u8], offset: usize, len: usize, label: &str) -> HavokResult<()> {
    let end = checked_add(offset, len, label)?;
    if end > data.len() {
        return Err(HavokError::InvalidInput(format!(
            "{label} is out of bounds: need {end} bytes, got {}",
            data.len()
        )));
    }
    Ok(())
}

fn validate_data_payload_relative(
    data_section: &SectionHeader,
    offset: usize,
    label: &str,
) -> HavokResult<usize> {
    absolute_data_payload_offset(data_section, offset, label)?;
    Ok(offset)
}

fn absolute_data_payload_offset(
    data_section: &SectionHeader,
    offset: usize,
    label: &str,
) -> HavokResult<usize> {
    absolute_payload_offset(data_section, offset, label)
        .map_err(|_| HavokError::InvalidInput(format!("{label} is outside __data__ payload")))
}

fn absolute_payload_offset(
    section: &SectionHeader,
    offset: usize,
    label: &str,
) -> HavokResult<usize> {
    let absolute = checked_add(section.offset, offset, label)?;
    if absolute >= section.data1 {
        return Err(HavokError::InvalidInput(format!(
            "{label} is outside {} payload",
            section.name
        )));
    }
    Ok(absolute)
}

fn checked_add(left: usize, right: usize, label: &str) -> HavokResult<usize> {
    left.checked_add(right)
        .ok_or_else(|| HavokError::InvalidInput(format!("{label} overflows")))
}

fn align_up(value: usize, alignment: usize) -> usize {
    if alignment <= 1 {
        value
    } else {
        value.div_ceil(alignment) * alignment
    }
}

fn read_i32(data: &[u8], offset: usize, label: &str) -> HavokResult<i32> {
    ensure_len(data, offset, 4, label)?;
    Ok(i32::from_le_bytes(
        data[offset..offset + 4]
            .try_into()
            .expect("4-byte slice after bounds check"),
    ))
}

fn read_u32(data: &[u8], offset: usize, label: &str) -> HavokResult<u32> {
    ensure_len(data, offset, 4, label)?;
    Ok(u32::from_le_bytes(
        data[offset..offset + 4]
            .try_into()
            .expect("4-byte slice after bounds check"),
    ))
}

fn read_f32(data: &[u8], offset: usize, label: &str) -> HavokResult<f32> {
    Ok(f32::from_bits(read_u32(data, offset, label)?))
}

fn read_u16(data: &[u8], offset: usize, label: &str) -> HavokResult<u16> {
    ensure_len(data, offset, 2, label)?;
    Ok(u16::from_le_bytes(
        data[offset..offset + 2]
            .try_into()
            .expect("2-byte slice after bounds check"),
    ))
}

fn read_c_string(data: &[u8], offset: usize) -> HavokResult<String> {
    ensure_len(data, offset, 0, "string offset")?;
    let tail = &data[offset..];
    let end = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| HavokError::InvalidInput("string is not null terminated".to_string()))?;
    std::str::from_utf8(&tail[..end])
        .map(str::to_string)
        .map_err(|_| HavokError::InvalidInput("string is not UTF-8".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skyrim_behavior_offsets_are_exact_and_signature_scoped() {
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_BEHAVIOR_GRAPH_SIGNATURE),
                "hkbBehaviorGraph",
                "rootGenerator",
                192,
            ),
            Some(128)
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_BEHAVIOR_GRAPH_DATA_SIGNATURE),
                "hkbBehaviorGraphData",
                "variableInitialValues",
                96,
            ),
            Some(112)
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_BEHAVIOR_GRAPH_DATA_SIGNATURE),
                "hkbBehaviorGraphData",
                "stringData",
                104,
            ),
            Some(120)
        );
        for (member, descriptor_offset, expected) in [
            ("selfTransitionMode", 136, 72),
            ("eventMode", 137, 73),
            ("duration", 168, 80),
            ("toGeneratorStartTimeFraction", 172, 84),
            ("flags", 176, 88),
            ("endMode", 178, 90),
            ("blendCurve", 179, 91),
            ("alignmentBone", 180, 92),
        ] {
            assert_eq!(
                skyrim_behavior_member_offset(
                    Some(SKYRIM_BLENDING_TRANSITION_EFFECT_SIGNATURE),
                    "hkbBlendingTransitionEffect",
                    member,
                    descriptor_offset,
                ),
                Some(expected),
                "{member}"
            );
        }
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_MODIFIER_GENERATOR_SIGNATURE),
                "hkbModifierGenerator",
                "generator",
                88,
            ),
            Some(80)
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_EVALUATE_EXPRESSION_MODIFIER_SIGNATURE),
                "hkbEvaluateExpressionModifier",
                "expressions",
                144,
            ),
            Some(80)
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_BEHAVIOR_REFERENCE_GENERATOR_SIGNATURE),
                "hkbBehaviorReferenceGenerator",
                "behaviorName",
                80,
            ),
            Some(72)
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_MANUAL_SELECTOR_GENERATOR_SIGNATURE),
                "hkbManualSelectorGenerator",
                "generators",
                136,
            ),
            Some(72)
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_BLENDER_GENERATOR_SIGNATURE),
                "hkbBlenderGenerator",
                "children",
                160,
            ),
            Some(96)
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_STATE_TAGGING_GENERATOR_SIGNATURE),
                "BSiStateTaggingGenerator",
                "pDefaultGenerator",
                88,
            ),
            Some(80)
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_CLIP_GENERATOR_SIGNATURE),
                "hkbClipGenerator",
                "animationName",
                144,
            ),
            Some(72)
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_BEHAVIOR_GRAPH_SIGNATURE ^ 1),
                "hkbBehaviorGraph",
                "rootGenerator",
                192,
            ),
            None
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_BEHAVIOR_GRAPH_DATA_SIGNATURE ^ 1),
                "hkbBehaviorGraphData",
                "variableInitialValues",
                96,
            ),
            None
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_BLENDING_TRANSITION_EFFECT_SIGNATURE ^ 1),
                "hkbBlendingTransitionEffect",
                "duration",
                168,
            ),
            None
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_MODIFIER_GENERATOR_SIGNATURE ^ 1),
                "hkbModifierGenerator",
                "generator",
                88,
            ),
            None
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_EVALUATE_EXPRESSION_MODIFIER_SIGNATURE ^ 1),
                "hkbEvaluateExpressionModifier",
                "expressions",
                144,
            ),
            None
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_BEHAVIOR_REFERENCE_GENERATOR_SIGNATURE ^ 1),
                "hkbBehaviorReferenceGenerator",
                "behaviorName",
                80,
            ),
            None
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_MANUAL_SELECTOR_GENERATOR_SIGNATURE ^ 1),
                "hkbManualSelectorGenerator",
                "generators",
                136,
            ),
            None
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_BLENDER_GENERATOR_SIGNATURE ^ 1),
                "hkbBlenderGenerator",
                "children",
                160,
            ),
            None
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_STATE_TAGGING_GENERATOR_SIGNATURE ^ 1),
                "BSiStateTaggingGenerator",
                "pDefaultGenerator",
                88,
            ),
            None
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_STATE_MACHINE_SIGNATURE),
                "hkbStateMachine",
                "states",
                208,
            ),
            Some(144)
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_STATE_MACHINE_STATE_INFO_SIGNATURE),
                "hkbStateMachineStateInfo",
                "transitions",
                80,
            ),
            Some(80)
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_BEHAVIOR_STRING_DATA_SIGNATURE),
                "hkbBehaviorGraphStringData",
                "eventNames",
                16,
            ),
            Some(16)
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_TRANSITION_INFO_ARRAY_SIGNATURE),
                "hkbStateMachineTransitionInfoArray",
                "transitions",
                16,
            ),
            Some(16)
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_CLIP_GENERATOR_SIGNATURE ^ 1),
                "hkbClipGenerator",
                "animationName",
                144,
            ),
            None
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_STATE_MACHINE_SIGNATURE ^ 1),
                "hkbStateMachine",
                "states",
                208,
            ),
            None
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_BEHAVIOR_STRING_DATA_SIGNATURE ^ 1),
                "hkbBehaviorGraphStringData",
                "eventNames",
                16,
            ),
            None
        );
    }
}
