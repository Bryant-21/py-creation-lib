use std::collections::HashMap;

use crate::error::{HavokError, HavokResult};

use super::descriptors::{DescriptorRegistry, MemberTemplate};
use super::model::{ArraySource, HkxMember, HkxObject};
use super::packfile::{ParsedPackfile, SectionHeader};
use super::types::{HkxType, HkxValue, deserialize_member_value};

const DATA_VIRTUAL_FIXUP_SECTION_INDEX: u32 = 0;

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
        local_fixups: &local_fixups,
        global_fixups: &global_fixups,
        object_index_by_offset: &object_index_by_offset,
    };

    for (object_index, (offset, class_name, signature)) in object_stubs.into_iter().enumerate() {
        ensure_len(data, offset, 0, "object offset")?;
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
    local_fixups: &'a HashMap<usize, usize>,
    global_fixups: &'a HashMap<usize, usize>,
    object_index_by_offset: &'a HashMap<usize, usize>,
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
        let offset = checked_add(base, member.offset, "member offset")?;
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
        return Err(HavokError::InvalidInput(format!(
            "missing local fixup for non-empty array {owner_class}.{}",
            member.name
        )));
    };

    let stride = element_stride(registry, member)?;

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
