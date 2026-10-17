use std::collections::HashMap;

use crate::error::{HavokError, HavokResult};

use super::descriptors::{DescriptorRegistry, MemberTemplate};
use super::model::{ArraySource, HkxMember, HkxObject};
use super::packfile::{ParsedPackfile, SectionHeader};
use super::types::{HkxType, HkxValue, deserialize_member_value};

const DATA_VIRTUAL_FIXUP_SECTION_INDEX: u32 = 0;
const SKYRIM_CHARACTER_DATA_SIGNATURE: u32 = 0x300d_6808;
const SKYRIM_CHARACTER_STRING_DATA_SIGNATURE: u32 = 0x655b_42bc;
const SKYRIM_CHARACTER_CONTROLLER_INFO_CLASS: &str = "hkbCharacterDataCharacterControllerInfo";
const SKYRIM_CLIP_GENERATOR_SIGNATURE: u32 = 0x333b_85b9;
const SKYRIM_BEHAVIOR_GRAPH_SIGNATURE: u32 = 0xb121_8f86;
const SKYRIM_BEHAVIOR_REFERENCE_GENERATOR_SIGNATURE: u32 = 0x0fcb_5423;
const SKYRIM_MANUAL_SELECTOR_GENERATOR_SIGNATURE: u32 = 0xd932_fab8;
const SKYRIM_BLENDER_GENERATOR_SIGNATURE: u32 = 0x22df_7147;
const SKYRIM_CYCLIC_BLEND_TRANSITION_GENERATOR_SIGNATURE: u32 = 0x5119_eb06;
const SKYRIM_GAMEBRYO_SEQUENCE_GENERATOR_SIGNATURE: u32 = 0xc8df_2d77;
const SKYRIM_DAMPING_MODIFIER_SIGNATURE: u32 = 0x9a04_0f03;
const SKYRIM_MODIFIER_GENERATOR_SIGNATURE: u32 = 0x1f81_fae6;
const SKYRIM_STATE_TAGGING_GENERATOR_SIGNATURE: u32 = 0xf082_6fc1;
const SKYRIM_STATE_MACHINE_SIGNATURE: u32 = 0x816c_1dcb;
const SKYRIM_STATE_MACHINE_STATE_INFO_SIGNATURE: u32 = 0x0ed7_f9d0;
const SKYRIM_BEHAVIOR_STRING_DATA_SIGNATURE: u32 = 0xc713_064e;
const SKYRIM_TRANSITION_INFO_ARRAY_SIGNATURE: u32 = 0xe397_b11e;
const SKYRIM_BEHAVIOR_GRAPH_DATA_SIGNATURE: u32 = 0x095a_ca5d;
const SKYRIM_BLENDING_TRANSITION_EFFECT_SIGNATURE: u32 = 0xfd85_84fe;
const SKYRIM_EVALUATE_EXPRESSION_MODIFIER_SIGNATURE: u32 = 0xf900_f6be;
const SKYRIM_POSE_MATCHING_GENERATOR_SIGNATURE: u32 = 0x29e2_71b4;
const SKYRIM_GET_UP_MODIFIER_SIGNATURE: u32 = 0x61cb_7ac0;
const SKYRIM_POWERED_RAGDOLL_CONTROLS_SIGNATURE: u32 = 0x7cb5_4065;
const SKYRIM_RIGID_BODY_RAGDOLL_CONTROLS_SIGNATURE: u32 = 0xaa87_d1eb;
const SKYRIM_EVENT_DRIVEN_MODIFIER_SIGNATURE: u32 = 0x7ed3_f44e;
const SKYRIM_KEYFRAME_BONES_MODIFIER_SIGNATURE: u32 = 0x95f6_6629;

#[cfg(test)]
#[path = "reader_recovery_tests.rs"]
mod recovery_layout_tests;

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
    let mut members = registry
        .get_all_members(class_name)
        .map_err(|error| HavokError::InvalidInput(error.to_string()))?;
    if is_legacy_2010(context)
        && context.object_signature_by_offset.get(&base) == Some(&0x19a0_05c0)
        && class_name == "BSDirectAtModifier"
    {
        members = skyrim_direct_at_members(members);
    }
    if is_legacy_2010(context)
        && context.object_signature_by_offset.get(&base)
            == Some(&SKYRIM_MANUAL_SELECTOR_GENERATOR_SIGNATURE)
        && class_name == "hkbManualSelectorGenerator"
    {
        for member in &mut members {
            if member.name == "selectedGeneratorIndex" {
                member.vtype = HkxType::Int8;
            }
        }
    }
    if is_legacy_2010(context)
        && context.object_signature_by_offset.get(&base)
            == Some(&SKYRIM_CYCLIC_BLEND_TRANSITION_GENERATOR_SIGNATURE)
        && class_name == "BSCyclicBlendTransitionGenerator"
    {
        members.retain(|member| {
            !matches!(
                member.name.as_str(),
                "TransitionOutEvent" | "TransitionInEvent"
            )
        });
    }
    let mut values = Vec::with_capacity(members.len());

    for member in members {
        if member.flags == "SERIALIZE_IGNORED" {
            continue;
        }
        if is_legacy_2010(context)
            && context.object_signature_by_offset.get(&base)
                == Some(&SKYRIM_CLIP_GENERATOR_SIGNATURE)
            && class_name == "hkbClipGenerator"
        {
            let absent = match member.name.as_str() {
                "animationBundleName" => Some(HkxValue::String {
                    value: String::new(),
                    is_null: true,
                }),
                "userPartitionMask" => Some(HkxValue::U32(0)),
                _ => None,
            };
            if let Some(value) = absent {
                values.push(HkxMember {
                    name: member.name,
                    value,
                });
                continue;
            }
        }
        if is_skyrim_character_controller_info(context, base, class_name, &member) {
            values.push(HkxMember {
                name: "characterControllerInfo".to_string(),
                value: read_skyrim_character_controller_info(context, base)?,
            });
            continue;
        }
        if let Some(value) = absent_legacy_member_value(context, base, class_name, &member) {
            values.push(HkxMember {
                name: member.name,
                value,
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
fn absent_legacy_member_value(
    context: &ReadContext<'_>,
    base: usize,
    owner_class: &str,
    member: &MemberTemplate,
) -> Option<HkxValue> {
    if !is_legacy_2010(context) {
        return None;
    }

    if member.name == "animationBlendFraction"
        && matches!(
            (
                owner_class,
                context.object_signature_by_offset.get(&base).copied()
            ),
            (
                "hkbPoweredRagdollControlsModifier",
                Some(SKYRIM_POWERED_RAGDOLL_CONTROLS_SIGNATURE)
            ) | (
                "hkbRigidBodyRagdollControlsModifier",
                Some(SKYRIM_RIGID_BODY_RAGDOLL_CONTROLS_SIGNATURE)
            )
        )
    {
        // These source versions predate the field; 0 is its declared Havok default.
        return Some(HkxValue::F32(0.0));
    }
    if owner_class == "hkbManualSelectorGenerator"
        && context.object_signature_by_offset.get(&base)
            == Some(&SKYRIM_MANUAL_SELECTOR_GENERATOR_SIGNATURE)
    {
        // Version 0 ends at +96; later selector controls must not read the next object.
        match member.name.as_str() {
            "indexSelector" | "generatorChangedTransitionEffect" => {
                return Some(HkxValue::Pointer(None));
            }
            "selectedIndexCanChangeAfterActivate" => return Some(HkxValue::Bool(true)),
            _ => {}
        }
    }
    if owner_class == "hkbCharacterStringData"
        && context.object_signature_by_offset.get(&base)
            == Some(&SKYRIM_CHARACTER_STRING_DATA_SIGNATURE)
    {
        // The source struct ends at +192; bow animation pointers begin there.
        match member.name.as_str() {
            "luaScriptOnCharacterActivated" | "luaScriptOnCharacterDeactivated" => {
                return Some(HkxValue::String {
                    value: String::new(),
                    is_null: true,
                });
            }
            "luaFiles" => return Some(HkxValue::Array(Vec::new())),
            _ => {}
        }
    }
    if member.vtype != HkxType::Array {
        return None;
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
        | ("hkbCharacterData", "boneAttachmentTransforms") => Some(HkxValue::Array(Vec::new())),
        _ => None,
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
    if member.name == "enable"
        && member.offset == 80
        && (owner_class.ends_with("Modifier") || owner_class == "hkbModifierList")
    {
        return Some(72);
    }
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
        (
            Some(SKYRIM_POSE_MATCHING_GENERATOR_SIGNATURE),
            "hkbPoseMatchingGenerator",
            "referencePoseWeightThreshold"
            | "blendParameter"
            | "minCyclicBlendParameter"
            | "maxCyclicBlendParameter"
            | "indexOfSyncMasterChild"
            | "flags"
            | "subtractLastChild"
            | "children"
            | "worldFromModelRotation"
            | "blendSpeed"
            | "minSpeedToSwitch"
            | "minSwitchTimeNoError"
            | "minSwitchTimeFullError"
            | "startPlayingEventId"
            | "startMatchingEventId"
            | "rootBoneIndex"
            | "otherBoneIndex"
            | "anotherBoneIndex"
            | "pelvisIndex"
            | "mode",
        ) => descriptor_offset.checked_sub(64),
        (
            Some(SKYRIM_GET_UP_MODIFIER_SIGNATURE),
            "hkbGetUpModifier",
            "groundNormal" | "duration" | "alignWithGroundDuration" | "rootBoneIndex"
            | "otherBoneIndex" | "anotherBoneIndex",
        )
        | (
            Some(SKYRIM_POWERED_RAGDOLL_CONTROLS_SIGNATURE),
            "hkbPoweredRagdollControlsModifier",
            "controlData" | "bones" | "worldFromModelModeData" | "boneWeights",
        )
        | (
            Some(SKYRIM_RIGID_BODY_RAGDOLL_CONTROLS_SIGNATURE),
            "hkbRigidBodyRagdollControlsModifier",
            "controlData" | "bones",
        ) => descriptor_offset.checked_sub(16),
        (
            Some(SKYRIM_EVENT_DRIVEN_MODIFIER_SIGNATURE),
            "hkbEventDrivenModifier",
            "modifier" | "activateEventId" | "deactivateEventId" | "activeByDefault",
        )
        | (
            Some(SKYRIM_KEYFRAME_BONES_MODIFIER_SIGNATURE),
            "hkbKeyframeBonesModifier",
            "keyframeInfo" | "keyframedBonesList",
        ) => descriptor_offset.checked_sub(8),
        (
            Some(SKYRIM_CYCLIC_BLEND_TRANSITION_GENERATOR_SIGNATURE),
            "BSCyclicBlendTransitionGenerator",
            "pBlenderGenerator" | "EventToFreezeBlendValue" | "EventToCrossBlend",
        ) => descriptor_offset.checked_sub(64),
        (
            Some(SKYRIM_CYCLIC_BLEND_TRANSITION_GENERATOR_SIGNATURE),
            "BSCyclicBlendTransitionGenerator",
            "fBlendParameter" | "fTransitionDuration" | "eBlendCurve",
        ) => descriptor_offset.checked_sub(96),
        (Some(0x338b_4879), "hkbTimerModifier", "alarmTimeSeconds" | "alarmEvent")
        | (Some(0x1062_d993), "BSEventOnDeactivateModifier", "event")
        | (
            Some(0xb0fd_e45a),
            "BSIsActiveModifier",
            "bIsActive0" | "bInvertActive0" | "bIsActive1" | "bInvertActive1" | "bIsActive2"
            | "bInvertActive2" | "bIsActive3" | "bInvertActive3" | "bIsActive4" | "bInvertActive4",
        ) => descriptor_offset.checked_sub(8),
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
            "generators" | "selectedGeneratorIndex",
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
        (
            Some(SKYRIM_DAMPING_MODIFIER_SIGNATURE),
            "hkbDampingModifier",
            "enable"
            | "kP"
            | "kI"
            | "kD"
            | "enableScalarDamping"
            | "enableVectorDamping"
            | "rawValue"
            | "dampedValue",
        ) => descriptor_offset.checked_sub(8),
        (
            Some(SKYRIM_GAMEBRYO_SEQUENCE_GENERATOR_SIGNATURE),
            "BGSGamebryoSequenceGenerator",
            "pSequence" | "eBlendModeFunction" | "fPercent" | "eUseTimePercentage" | "fTimePercent",
        ) => descriptor_offset.checked_sub(64),
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
            "animationName" | "triggers",
        ) => descriptor_offset.checked_sub(72),
        (
            Some(SKYRIM_CLIP_GENERATOR_SIGNATURE),
            "hkbClipGenerator",
            "cropStartAmountLocalTime"
            | "cropEndAmountLocalTime"
            | "startTime"
            | "playbackSpeed"
            | "enforcedDuration"
            | "userControlledTimeFraction"
            | "animationBindingIndex"
            | "mode"
            | "flags",
        ) => descriptor_offset.checked_sub(76),
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

// The 0x19a005c0 Skyrim class predates the FO4 asymmetric limits and context enum.
// Offsets match its reflection metadata and the mounted-bow object's raw payload.
fn skyrim_direct_at_members(mut inherited: Vec<MemberTemplate>) -> Vec<MemberTemplate> {
    inherited.retain(|member| {
        matches!(
            member.name.as_str(),
            "variableBindingSet" | "userData" | "name" | "enable"
        )
    });
    if let Some(enable) = inherited.iter_mut().find(|member| member.name == "enable") {
        enable.offset = 72;
    }
    for (name, offset, vtype) in [
        ("directAtTarget", 80, HkxType::Bool),
        ("sourceBoneIndex", 82, HkxType::Int16),
        ("startBoneIndex", 84, HkxType::Int16),
        ("endBoneIndex", 86, HkxType::Int16),
        ("limitHeadingDegrees", 88, HkxType::Real),
        ("limitPitchDegrees", 92, HkxType::Real),
        ("offsetHeadingDegrees", 96, HkxType::Real),
        ("offsetPitchDegrees", 100, HkxType::Real),
        ("onGain", 104, HkxType::Real),
        ("offGain", 108, HkxType::Real),
        ("targetLocation", 112, HkxType::Vector4),
        ("userInfo", 128, HkxType::Uint32),
        ("directAtCamera", 132, HkxType::Bool),
        ("directAtCameraX", 136, HkxType::Real),
        ("directAtCameraY", 140, HkxType::Real),
        ("directAtCameraZ", 144, HkxType::Real),
        ("active", 148, HkxType::Bool),
        ("currentHeadingOffset", 152, HkxType::Real),
        ("currentPitchOffset", 156, HkxType::Real),
    ] {
        inherited.push(MemberTemplate {
            name: name.into(),
            offset,
            vtype,
            vsubtype: HkxType::Void,
            ctype: String::new(),
            arrsize: 0,
            flags: "FLAGS_NONE".into(),
            etype: String::new(),
            default: None,
        });
    }
    inherited
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
mod character_string_data_tests {
    use super::*;

    #[test]
    fn character_string_lua_fields_are_absent_only_in_exact_skyrim_layout() {
        let mut data = vec![0u8; 384];
        for (offset, text) in [(256, "activated.lua"), (320, "deactivated.lua")] {
            data[offset..offset + text.len()].copy_from_slice(text.as_bytes());
        }
        let section = SectionHeader {
            name: "__data__".into(),
            offset: 0,
            data1: data.len(),
            data2: data.len(),
            data3: data.len(),
            exports: data.len(),
            imports: data.len(),
            end: data.len(),
        };
        let local_fixups = HashMap::from([(192, 256), (200, 320)]);
        let empty = HashMap::new();
        for (version, signature, absent) in [
            (
                "hk_2010.2.0-r1",
                SKYRIM_CHARACTER_STRING_DATA_SIGNATURE,
                true,
            ),
            (
                "hk_2010.2.0-r1",
                SKYRIM_CHARACTER_STRING_DATA_SIGNATURE ^ 1,
                false,
            ),
            (
                "hk_2014.1.0-r1",
                SKYRIM_CHARACTER_STRING_DATA_SIGNATURE,
                false,
            ),
        ] {
            let signatures = HashMap::from([(0, signature)]);
            let context = ReadContext {
                data: &data,
                data_section: &section,
                contents_version: version,
                local_fixups: &local_fixups,
                global_fixups: &empty,
                object_index_by_offset: &empty,
                object_signature_by_offset: &signatures,
            };
            let mut registry = DescriptorRegistry::for_contents_version(version);
            let members = read_members(
                &mut registry,
                &context,
                0,
                "hkbCharacterStringData",
                0,
                &mut Vec::new(),
                &mut Vec::new(),
            )
            .unwrap();
            for (field, text) in [
                ("luaScriptOnCharacterActivated", "activated.lua"),
                ("luaScriptOnCharacterDeactivated", "deactivated.lua"),
            ] {
                assert_eq!(
                    members
                        .iter()
                        .find(|member| member.name == field)
                        .unwrap()
                        .value,
                    HkxValue::String {
                        value: if absent { String::new() } else { text.into() },
                        is_null: absent,
                    },
                    "{version} 0x{signature:08x} {field}"
                );
            }
        }
    }

    #[test]
    #[ignore = "requires the extracted Skyrim bow and crossbow characters"]
    fn skyrim_character_animation_pointers_do_not_become_lua_hooks() {
        let root = std::env::var_os("MODKIT_SKYRIM_MESH_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../extracted/skyrimse/Meshes")
            });
        for (relative, expected) in [
            (
                "weapons/bow/characters/bowcharacter.hkx",
                vec![
                    r"..\Bow\Animations\Bow_DrawHeavy.hkx",
                    r"..\Bow\Animations\Bow_DrawLight.hkx",
                    r"..\Bow\Animations\Bow_IdleDrawn.hkx",
                    r"..\Bow\Animations\Bow_Release.hkx",
                ],
            ),
            (
                "dlc01/weapons/crossbow/characters/crossbowcharacter.hkx",
                vec![
                    r"Animations\CrossBow_Release.hkx",
                    r"Animations\CrossBow_Reload.hkx",
                ],
            ),
        ] {
            let bytes = std::fs::read(root.join(relative)).unwrap();
            let file = crate::hkx::HkxFile::read(&bytes).unwrap();
            let (index, object) = file
                .objects()
                .iter()
                .enumerate()
                .find(|(_, object)| object.class_name == "hkbCharacterStringData")
                .unwrap();
            assert_eq!(object.signature, SKYRIM_CHARACTER_STRING_DATA_SIGNATURE);
            let value = |name: &str| {
                &object
                    .members
                    .iter()
                    .find(|member| member.name == name)
                    .unwrap()
                    .value
            };
            let source = file
                .array_sources()
                .iter()
                .find(|source| {
                    source.object_index == index
                        && source.member_path == ["animationBundleNameData"]
                })
                .unwrap();
            assert_eq!(source.content_offset, object.offset + 192, "{relative}");
            assert_eq!(source.content_length, expected.len() * 8);
            let HkxValue::Array(entries) = value("animationBundleNameData") else {
                panic!("missing source animation names: {relative}")
            };
            let names = entries
                .iter()
                .map(|entry| {
                    let members = entry.as_object_members().unwrap();
                    assert_eq!(
                        members
                            .iter()
                            .find(|member| member.name == "assetNames")
                            .unwrap()
                            .value,
                        HkxValue::Array(Vec::new())
                    );
                    let HkxValue::String {
                        value,
                        is_null: false,
                    } = &members
                        .iter()
                        .find(|member| member.name == "bundleName")
                        .unwrap()
                        .value
                    else {
                        panic!("missing source animation path")
                    };
                    value.as_str()
                })
                .collect::<Vec<_>>();
            assert_eq!(names, expected, "{relative}");
            for field in [
                "luaScriptOnCharacterActivated",
                "luaScriptOnCharacterDeactivated",
            ] {
                assert_eq!(
                    value(field),
                    &HkxValue::String {
                        value: String::new(),
                        is_null: true
                    },
                    "{relative} {field}"
                );
            }
            assert_eq!(value("luaFiles"), &HkxValue::Array(Vec::new()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires the extracted Skyrim hagraven behavior"]
    fn skyrim_cyclic_blend_preserves_source_child_and_transition_events() {
        let root = std::env::var("MODKIT_REPO_ROOT").expect("MODKIT_REPO_ROOT");
        let bytes = std::fs::read(
            std::path::Path::new(&root)
                .join("extracted/skyrimse/meshes/actors/hagraven/behaviors/havgravenbehavior.hkx"),
        )
        .unwrap();
        let file = crate::hkx::HkxFile::read(&bytes).unwrap();
        let member = |object: &HkxObject, name: &str| {
            object
                .members
                .iter()
                .find(|member| member.name == name)
                .unwrap()
                .value
                .clone()
        };
        let nodes = file
            .objects()
            .iter()
            .filter(|object| object.class_name == "BSCyclicBlendTransitionGenerator")
            .collect::<Vec<_>>();
        assert_eq!(nodes.len(), 2);
        for node in nodes {
            let HkxValue::Pointer(Some(child)) = member(node, "pBlenderGenerator") else {
                panic!("source cyclic blend lost its directional blender")
            };
            let child = &file.objects()[child];
            assert_eq!(child.class_name, "hkbBlenderGenerator");
            assert!(
                matches!(member(child, "name"), HkxValue::String { value, .. } if value == "MT_DirectionalBlend" || value == "Magic_DirectionalBlend")
            );
            for (field, expected, raw_offset) in [
                ("EventToFreezeBlendValue", 37, 88),
                ("EventToCrossBlend", 38, 104),
            ] {
                let event = member(node, field);
                assert_eq!(
                    event
                        .as_object_members()
                        .unwrap()
                        .iter()
                        .find(|member| member.name == "id")
                        .unwrap()
                        .value,
                    HkxValue::I32(expected)
                );
                assert_eq!(
                    i32::from_le_bytes(
                        bytes[node.offset + raw_offset..node.offset + raw_offset + 4]
                            .try_into()
                            .unwrap()
                    ),
                    expected
                );
            }
            assert_eq!(member(node, "fBlendParameter"), HkxValue::F32(0.0));
            assert_eq!(member(node, "fTransitionDuration"), HkxValue::F32(0.2));
            assert!(!node.members.iter().any(|member| matches!(
                member.name.as_str(),
                "TransitionOutEvent" | "TransitionInEvent"
            )));
        }
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(0x8640_1931),
                "BSCyclicBlendTransitionGenerator",
                "pBlenderGenerator",
                144
            ),
            None
        );
    }

    #[test]
    #[ignore = "requires the extracted Skyrim horse behavior"]
    fn skyrim_manual_selectors_do_not_read_adjacent_runtime_state_or_object_tails() {
        let root = std::env::var("MODKIT_REPO_ROOT").expect("MODKIT_REPO_ROOT");
        let mut bytes = std::fs::read(
            std::path::Path::new(&root)
                .join("extracted/skyrimse/meshes/actors/character/behaviors/horsebehavior.hkx"),
        )
        .unwrap();
        let file = crate::hkx::HkxFile::read(&bytes).unwrap();
        let value = |object: &HkxObject, name: &str| {
            object
                .members
                .iter()
                .find(|member| member.name == name)
                .unwrap()
                .value
                .clone()
        };
        let selectors = file
            .objects()
            .iter()
            .filter(|object| object.class_name == "hkbManualSelectorGenerator")
            .collect::<Vec<_>>();
        assert!(selectors.len() > 10);
        for selector in &selectors {
            assert_eq!(
                value(selector, "selectedGeneratorIndex"),
                HkxValue::I8(bytes[selector.offset + 88] as i8)
            );
            for name in ["indexSelector", "generatorChangedTransitionEffect"] {
                assert_eq!(value(selector, name), HkxValue::Pointer(None));
            }
            assert_eq!(
                value(selector, "selectedIndexCanChangeAfterActivate"),
                HkxValue::Bool(true)
            );
        }
        let selector = selectors
            .into_iter()
            .find(|selector| {
                matches!(value(selector, "name"), HkxValue::String { value, .. } if value == "MC_WeapEquip_MSG")
            })
            .unwrap();
        bytes[selector.offset + 88] = 7;
        bytes[selector.offset + 89] = 0xff;
        let mutated = crate::hkx::HkxFile::read(&bytes).unwrap();
        let mutated = mutated
            .objects()
            .iter()
            .find(|object| object.offset == selector.offset)
            .unwrap();
        assert_eq!(value(mutated, "selectedGeneratorIndex"), HkxValue::I8(7));
    }

    #[test]
    #[ignore = "requires the extracted Skyrim horse behavior"]
    fn skyrim_mounted_clip_timing_and_aim_match_source_payload() {
        let root = std::env::var("MODKIT_REPO_ROOT").expect("MODKIT_REPO_ROOT");
        let bytes = std::fs::read(
            std::path::Path::new(&root)
                .join("extracted/skyrimse/meshes/actors/character/behaviors/horsebehavior.hkx"),
        )
        .unwrap();
        let file = crate::hkx::HkxFile::read(&bytes).unwrap();
        let value = |object: &HkxObject, name: &str| {
            object
                .members
                .iter()
                .find(|member| member.name == name)
                .unwrap()
                .value
                .clone()
        };
        let clip = |name: &str| {
            file.objects()
                .iter()
                .find(|object| {
                    object.class_name == "hkbClipGenerator"
                        && value(object, "name")
                            == HkxValue::String {
                                value: name.into(),
                                is_null: false,
                            }
                })
                .unwrap()
        };
        let normal = clip("Bow_DrawNock");
        let quick = clip("Bow_DrawNock_QuickShot");
        assert_eq!(value(normal, "startTime"), HkxValue::F32(0.0));
        assert_eq!(value(normal, "playbackSpeed"), HkxValue::F32(1.0));
        assert_eq!(value(quick, "playbackSpeed"), HkxValue::F32(2.0));
        assert_eq!(value(normal, "animationBindingIndex"), HkxValue::I16(-1));
        assert_eq!(
            f32::from_le_bytes(
                bytes[normal.offset + 100..normal.offset + 104]
                    .try_into()
                    .unwrap()
            ),
            1.0
        );
        let aim = file
            .objects()
            .iter()
            .find(|object| object.class_name == "BSDirectAtModifier")
            .unwrap();
        assert_eq!(value(aim, "sourceBoneIndex"), HkxValue::I16(26));
        assert_eq!(value(aim, "startBoneIndex"), HkxValue::I16(24));
        assert_eq!(value(aim, "endBoneIndex"), HkxValue::I16(26));
        assert_eq!(value(aim, "limitHeadingDegrees"), HkxValue::F32(90.0));
        assert_eq!(value(aim, "limitPitchDegrees"), HkxValue::F32(90.0));
        assert_eq!(value(aim, "onGain"), HkxValue::F32(0.05));
        assert_eq!(value(aim, "userInfo"), HkxValue::U32(2));
        assert!(!aim.members.iter().any(|member| member.name == "context"));
        let timer = file
            .objects()
            .iter()
            .find(|object| {
                object.class_name == "hkbTimerModifier"
                    && value(object, "name")
                        == HkxValue::String {
                            value: "SendEvent_BowRelease".into(),
                            is_null: false,
                        }
            })
            .unwrap();
        assert_eq!(value(timer, "enable"), HkxValue::Bool(true));
        let event = value(timer, "alarmEvent");
        assert_eq!(
            event
                .as_object_members()
                .unwrap()
                .iter()
                .find(|member| member.name == "id")
                .unwrap()
                .value,
            HkxValue::I32(156)
        );
        let deactivate = file
            .objects()
            .iter()
            .find(|object| {
                object.class_name == "BSEventOnDeactivateModifier"
                    && value(object, "name")
                        == HkxValue::String {
                            value: "BowZoomStop_OnDeactivateMod".into(),
                            is_null: false,
                        }
            })
            .unwrap();
        assert_eq!(value(deactivate, "enable"), HkxValue::Bool(true));
        let event = value(deactivate, "event");
        assert_eq!(
            event
                .as_object_members()
                .unwrap()
                .iter()
                .find(|member| member.name == "id")
                .unwrap()
                .value,
            HkxValue::I32(174)
        );
    }

    #[test]
    fn skyrim_nif_sequence_generator_layout_is_signature_gated() {
        for (field, target, source) in [
            ("pSequence", 136, 72),
            ("eBlendModeFunction", 144, 80),
            ("fPercent", 148, 84),
            ("eUseTimePercentage", 152, 88),
            ("fTimePercent", 156, 92),
        ] {
            assert_eq!(
                skyrim_behavior_member_offset(
                    Some(SKYRIM_GAMEBRYO_SEQUENCE_GENERATOR_SIGNATURE),
                    "BGSGamebryoSequenceGenerator",
                    field,
                    target
                ),
                Some(source)
            );
            assert_eq!(
                skyrim_behavior_member_offset(
                    Some(0x4e708fb6),
                    "BGSGamebryoSequenceGenerator",
                    field,
                    target
                ),
                None
            );
        }
    }

    #[test]
    fn skyrim_damping_layout_preserves_vector_alignment() {
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_DAMPING_MODIFIER_SIGNATURE),
                "hkbDampingModifier",
                "enable",
                80
            ),
            Some(72)
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_DAMPING_MODIFIER_SIGNATURE),
                "hkbDampingModifier",
                "kP",
                88
            ),
            Some(80)
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_DAMPING_MODIFIER_SIGNATURE),
                "hkbDampingModifier",
                "enableScalarDamping",
                100
            ),
            Some(92)
        );
        assert_eq!(
            skyrim_behavior_member_offset(
                Some(SKYRIM_DAMPING_MODIFIER_SIGNATURE),
                "hkbDampingModifier",
                "rawVector",
                112
            ),
            None
        );
        assert_eq!(
            skyrim_behavior_member_offset(Some(0x68a51d05), "hkbDampingModifier", "kP", 88),
            None
        );
    }

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
