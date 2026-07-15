use crate::error::{HavokError, HavokResult};

use super::descriptors::{DescriptorRegistry, MemberTemplate};
use super::model::{HkxFile, HkxMember, HkxObject};
use super::types::{HkxType, HkxValue, half_to_f32};
use std::collections::HashMap;

pub fn read_tagxml_string(xml: &str) -> HavokResult<HkxFile> {
    let contents_version = detect_tagxml_contents_version(xml)?;
    let mut registry = DescriptorRegistry::for_contents_version(&contents_version);
    read_tagxml_string_with_registry(xml, &mut registry)
}

pub fn read_tagxml_string_with_registry(
    xml: &str,
    registry: &mut DescriptorRegistry,
) -> HavokResult<HkxFile> {
    let document = roxmltree::Document::parse(xml)
        .map_err(|error| HavokError::InvalidInput(format!("invalid TagXML: {error}")))?;
    let root = document.root_element();
    if root.tag_name().name() != "hkpackfile" {
        return Err(HavokError::InvalidInput(
            "TagXML root must be hkpackfile".to_string(),
        ));
    }

    let class_version = root
        .attribute("classversion")
        .unwrap_or("11")
        .parse::<u32>()
        .map_err(|error| HavokError::InvalidInput(format!("invalid classversion: {error}")))?;
    let contents_version = root
        .attribute("contentsversion")
        .unwrap_or("hk_2014.1.0-r1");
    let mut object_nodes = Vec::new();

    for section in root
        .children()
        .filter(|node| node.has_tag_name("hksection"))
    {
        if section.attribute("name") != Some("__data__") {
            continue;
        }
        for object_node in section
            .children()
            .filter(|node| node.has_tag_name("hkobject"))
        {
            object_nodes.push(object_node);
        }
    }
    let mut object_names: HashMap<String, usize> = HashMap::new();
    for (index, node) in object_nodes.iter().enumerate() {
        let Some(name) = node.attribute("name") else {
            continue;
        };
        if object_names.insert(name.to_string(), index).is_some() {
            return Err(HavokError::InvalidInput(format!(
                "duplicate hkobject name {name}"
            )));
        }
    }
    let mut objects = Vec::with_capacity(object_nodes.len());
    for (index, object_node) in object_nodes.into_iter().enumerate() {
        objects.push(parse_object(registry, object_node, index, &object_names)?);
    }

    Ok(HkxFile::from_tagxml(
        class_version,
        contents_version.to_string(),
        objects,
    ))
}

pub fn write_tagxml_string(hkx: &HkxFile) -> HavokResult<String> {
    let mut registry = DescriptorRegistry::for_contents_version(hkx.contents_version());
    write_tagxml_string_with_registry(hkx, &mut registry)
}

pub fn write_tagxml_string_with_registry(
    hkx: &HkxFile,
    registry: &mut DescriptorRegistry,
) -> HavokResult<String> {
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"ASCII\" standalone=\"no\"?>\n");
    xml.push_str(&format!(
        "<hkpackfile classversion=\"{}\" contentsversion=\"{}\">\n",
        hkx.class_version(),
        escape_attr(hkx.contents_version())
    ));
    xml.push_str("    <hksection name=\"__data__\">\n");
    for (index, object) in hkx.objects().iter().enumerate() {
        write_object(&mut xml, object, 2, index, registry)?;
    }
    xml.push_str("    </hksection>\n");
    xml.push_str("</hkpackfile>\n");
    Ok(xml)
}

fn parse_object(
    registry: &mut DescriptorRegistry,
    node: roxmltree::Node<'_, '_>,
    index: usize,
    object_names: &HashMap<String, usize>,
) -> HavokResult<HkxObject> {
    let class_name = node.attribute("class").unwrap_or("").to_string();
    Ok(HkxObject {
        name: node.attribute("name").map(str::to_string),
        offset: index,
        signature: parse_signature(node.attribute("signature"))?,
        members: parse_members(registry, node, &class_name, object_names)?,
        class_name,
    })
}

fn detect_tagxml_contents_version(xml: &str) -> HavokResult<String> {
    let document = roxmltree::Document::parse(xml)
        .map_err(|error| HavokError::InvalidInput(format!("invalid TagXML: {error}")))?;
    let root = document.root_element();
    if root.tag_name().name() != "hkpackfile" {
        return Err(HavokError::InvalidInput(
            "TagXML root must be hkpackfile".to_string(),
        ));
    }
    Ok(root
        .attribute("contentsversion")
        .unwrap_or("hk_2014.1.0-r1")
        .to_string())
}

fn parse_members(
    registry: &mut DescriptorRegistry,
    node: roxmltree::Node<'_, '_>,
    class_name: &str,
    object_names: &HashMap<String, usize>,
) -> HavokResult<Vec<HkxMember>> {
    let templates = registry
        .get_all_members(class_name)
        .map_err(|error| HavokError::InvalidInput(error.to_string()))?;

    let mut explicit: Vec<HkxMember> = node
        .children()
        .filter(|child| child.has_tag_name("hkparam"))
        .map(|param| {
            let name = param.attribute("name").unwrap_or("").to_string();
            let template = templates.iter().find(|template| template.name == name);
            Ok(HkxMember {
                value: parse_param_value(registry, param, template, class_name, object_names)?,
                name,
            })
        })
        .collect::<HavokResult<_>>()?;

    // Fill in defaulted members that the source XML omitted.
    for template in &templates {
        let Some(default_value) = &template.default else {
            continue;
        };
        if !explicit.iter().any(|m| m.name == template.name) {
            explicit.push(HkxMember {
                name: template.name.clone(),
                value: default_value.clone(),
            });
        }
    }

    Ok(explicit)
}

fn parse_param_value(
    registry: &mut DescriptorRegistry,
    param: roxmltree::Node<'_, '_>,
    template: Option<&MemberTemplate>,
    owner_class: &str,
    object_names: &HashMap<String, usize>,
) -> HavokResult<HkxValue> {
    let string_children: Vec<_> = param
        .children()
        .filter(|child| child.has_tag_name("hkcstring"))
        .collect();
    if !string_children.is_empty() {
        if let Some(template) = template {
            let subtype = if template.vsubtype == HkxType::Void {
                template.vtype
            } else {
                template.vsubtype
            };
            if matches!(subtype, HkxType::CString | HkxType::StringPtr) {
                return Ok(HkxValue::Array(
                    string_children
                        .into_iter()
                        .map(|child| HkxValue::String {
                            value: child.text().unwrap_or("").to_string(),
                            is_null: false,
                        })
                        .collect(),
                ));
            }
        }
    }

    let object_children: Vec<_> = param
        .children()
        .filter(|child| child.has_tag_name("hkobject"))
        .collect();
    if !object_children.is_empty() {
        let class_name = template
            .map(|template| template.ctype.as_str())
            .unwrap_or("");
        if matches!(
            template.map(|template| template.vtype),
            Some(HkxType::Struct)
        ) {
            if object_children.len() != 1 {
                return Err(HavokError::InvalidInput(format!(
                    "inline struct {} must contain exactly one hkobject",
                    param.attribute("name").unwrap_or("")
                )));
            }
            return parse_members(registry, object_children[0], class_name, object_names)
                .map(HkxValue::Object);
        }
        let values = object_children
            .into_iter()
            .map(|object| {
                parse_members(registry, object, class_name, object_names).map(HkxValue::Object)
            })
            .collect::<HavokResult<Vec<_>>>()?;
        return Ok(HkxValue::Array(values));
    }

    let text = param.text().unwrap_or("").trim();
    let numelements = param
        .attribute("numelements")
        .and_then(|s| s.parse::<usize>().ok());
    if let Some(template) = template {
        return parse_descriptor_value(
            registry,
            text,
            template,
            param.attribute("numelements").is_some(),
            numelements,
            owner_class,
            object_names,
        );
    }
    if param.attribute("numelements").is_some() {
        return Ok(HkxValue::Array(
            text.split_whitespace().map(parse_scalar).collect(),
        ));
    }
    Ok(parse_scalar(text))
}

fn parse_scalar(text: &str) -> HkxValue {
    if let Some(index) = parse_pointer_index(text) {
        HkxValue::Pointer(Some(index))
    } else if text == "null" {
        HkxValue::Pointer(None)
    } else if let Ok(value) = text.parse::<i32>() {
        HkxValue::I32(value)
    } else if looks_like_float(text) {
        text.parse::<f32>()
            .map(HkxValue::F32)
            .unwrap_or_else(|_| string_value(text))
    } else {
        string_value(text)
    }
}

fn parse_descriptor_value(
    registry: &mut DescriptorRegistry,
    text: &str,
    template: &MemberTemplate,
    is_array: bool,
    numelements: Option<usize>,
    owner_class: &str,
    object_names: &HashMap<String, usize>,
) -> HavokResult<HkxValue> {
    if is_array {
        let subtype = if template.vsubtype == HkxType::Void {
            template.vtype
        } else {
            template.vsubtype
        };
        // COMPLEX subtypes (Vector4/Quaternion/QsTransform/...) can be
        // serialized as Python-style parenthesized groups: `(a b c d)(e f g h)`.
        // Detect the parens and parse group-wise into F32List elements.
        // Falls through to the flat path for flat-formatted input (Rust's
        // pre-fix output), keeping the parser tolerant of both shapes.
        if is_complex_type(subtype) && text.contains('(') {
            return parse_complex_array(text, subtype).map(HkxValue::Array);
        }
        // Try the standard whitespace-separated form first.
        let parsed = text
            .split_whitespace()
            .map(|item| {
                parse_typed_scalar(registry, item, subtype, template, owner_class, object_names)
            })
            .collect::<HavokResult<Vec<_>>>();
        match parsed {
            Ok(values) => {
                // Flat-formatted COMPLEX arrays decode element-wise as raw
                // floats above; rebundle them into per-element F32List groups
                // so downstream consumers see one HkxValue per array entry.
                if is_complex_type(subtype) {
                    return Ok(HkxValue::Array(rebundle_complex_floats(values, subtype)));
                }
                return Ok(HkxValue::Array(values));
            }
            Err(err) => {
                // Fall back to hkxpack-cli's packed-hex blob encoding for
                // small-int subtypes — used by hkArray<hkUint8> etc., e.g.
                // hkaSplineCompressedAnimation::data. Mirrors
                // py_creation_lib/python/creation_lib/hkxpack/tagreader.py:204-227.
                if let Some(values) = try_parse_packed_hex(text, subtype, numelements) {
                    return Ok(HkxValue::Array(values));
                }
                return Err(err);
            }
        }
    }
    // Single COMPLEX value: accept parenthesized form by stripping parens
    // and parsing as a flat float list.
    if is_complex_type(template.vtype) && text.contains('(') {
        return parse_complex_single(text, template.vtype).map(HkxValue::F32List);
    }
    parse_typed_scalar(
        registry,
        text,
        template.vtype,
        template,
        owner_class,
        object_names,
    )
}

fn complex_group_size(hkx_type: HkxType) -> usize {
    match hkx_type {
        HkxType::Vector4 | HkxType::Quaternion => 4,
        HkxType::QsTransform | HkxType::Transform | HkxType::Matrix3 => 12,
        HkxType::Matrix4 => 16,
        _ => 0,
    }
}

fn parse_float_run(text: &str) -> HavokResult<Vec<f32>> {
    text.split_whitespace()
        .filter(|piece| !piece.is_empty())
        .map(|piece| {
            piece.parse::<f32>().map_err(|error| {
                HavokError::InvalidInput(format!("invalid float {piece}: {error}"))
            })
        })
        .collect()
}

fn parse_complex_single(text: &str, hkx_type: HkxType) -> HavokResult<Vec<f32>> {
    // Strip parens — Python's tagreader._parse_complex_text does the same via
    // a `[-\d.eE+]+` regex; we use a cheaper sanitize since we already know
    // the format. Both `(a b c d)` and `(a b c d)(e f g h)...` collapse to
    // a single flat float run.
    let cleaned: String = text
        .chars()
        .map(|c| if c == '(' || c == ')' { ' ' } else { c })
        .collect();
    let floats = parse_float_run(&cleaned)?;
    let expected = complex_group_size(hkx_type);
    if expected > 0 && floats.len() != expected {
        return Err(HavokError::InvalidInput(format!(
            "complex {hkx_type:?} expected {expected} floats, got {}",
            floats.len()
        )));
    }
    Ok(floats)
}

fn parse_complex_array(text: &str, subtype: HkxType) -> HavokResult<Vec<HkxValue>> {
    let group_size = complex_group_size(subtype);
    if group_size == 0 {
        return Err(HavokError::InvalidInput(format!(
            "non-complex subtype {subtype:?} cannot be parsed as parenthesized array",
        )));
    }

    // Two parenthesized layouts in the wild:
    //   1. Python's `py_creation_lib/python/creation_lib/hkxpack/tagwriter.py:164-169` — one `(...)` per
    //      element, each group holding the full `group_size` floats.
    //   2. Java hkxpack-cli — vector-aligned: one `(a b c d)` per 4 floats,
    //      so a 12-float QsTransform is `(t)(r)(s)`. The reference fixture
    //      `resource/skeleton.xml` is in this form.
    // First parse every `(...)` group as a 4..N-float chunk, then promote the
    // chunks to elements based on which layout fits the total float count
    // and the chunk shape.
    let mut chunks: Vec<Vec<f32>> = Vec::new();
    let mut cursor = 0;
    let bytes = text.as_bytes();
    while cursor < bytes.len() {
        match memchr_byte(bytes, b'(', cursor) {
            Some(open) => {
                let close = memchr_byte(bytes, b')', open + 1).ok_or_else(|| {
                    HavokError::InvalidInput(format!(
                        "complex array {subtype:?}: unclosed '(' at byte {open}",
                    ))
                })?;
                let inner = &text[open + 1..close];
                let floats = parse_float_run(inner)?;
                chunks.push(floats);
                cursor = close + 1;
            }
            None => break,
        }
    }

    // Layout 1: one chunk per element, each holding `group_size` floats.
    if chunks.iter().all(|chunk| chunk.len() == group_size) {
        return Ok(chunks.into_iter().map(HkxValue::F32List).collect());
    }
    // Layout 2: every chunk is exactly 4 floats and the total float count is
    // a multiple of `group_size`. Concat and re-chunk by `group_size`.
    if chunks.iter().all(|chunk| chunk.len() == 4) {
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        if total.is_multiple_of(group_size) {
            let mut floats: Vec<f32> = Vec::with_capacity(total);
            for chunk in chunks {
                floats.extend(chunk);
            }
            return Ok(floats
                .chunks_exact(group_size)
                .map(|chunk| HkxValue::F32List(chunk.to_vec()))
                .collect());
        }
    }

    Err(HavokError::InvalidInput(format!(
        "complex array {subtype:?}: cannot reconcile parenthesized chunk shape with group size {group_size} (chunks: {:?})",
        chunks.iter().map(|c| c.len()).collect::<Vec<_>>()
    )))
}

fn memchr_byte(haystack: &[u8], needle: u8, start: usize) -> Option<usize> {
    haystack[start.min(haystack.len())..]
        .iter()
        .position(|byte| *byte == needle)
        .map(|offset| start + offset)
}

fn rebundle_complex_floats(values: Vec<HkxValue>, subtype: HkxType) -> Vec<HkxValue> {
    let group_size = complex_group_size(subtype);
    if group_size == 0 {
        return values;
    }
    // Pull all f32s from the parsed scalar values; bail out (return as-is)
    // if anything isn't a float — happens on COMPLEX-named-but-non-float
    // edge cases that the caller can still consume via the original list.
    let mut floats: Vec<f32> = Vec::with_capacity(values.len());
    for value in &values {
        match value {
            HkxValue::F32(v) => floats.push(*v),
            _ => return values,
        }
    }
    if !floats.len().is_multiple_of(group_size) {
        return values;
    }
    floats
        .chunks_exact(group_size)
        .map(|chunk| HkxValue::F32List(chunk.to_vec()))
        .collect()
}

/// Look up an enum value name in the descriptor registry, searching up the
/// inheritance chain. Returns None when the enum doesn't carry that name.
fn lookup_enum_int(
    registry: &mut DescriptorRegistry,
    class_name: &str,
    enum_name: &str,
    str_value: &str,
) -> Option<i32> {
    use std::collections::HashSet;
    let mut current = Some(class_name.to_string());
    let mut seen = HashSet::new();
    while let Some(name) = current {
        if !seen.insert(name.clone()) {
            return None;
        }
        let (parent, found) = {
            let desc = registry.get(&name).ok().flatten()?;
            let found = desc
                .enums
                .get(enum_name)
                .and_then(|enum_def| enum_def.name_to_value(str_value))
                .map(|v| v as i32);
            (desc.parent.clone(), found)
        };
        if let Some(value) = found {
            return Some(value);
        }
        current = parent.filter(|p| !p.is_empty());
    }
    None
}

fn try_parse_packed_hex(
    text: &str,
    subtype: HkxType,
    numelements: Option<usize>,
) -> Option<Vec<HkxValue>> {
    if !matches!(
        subtype,
        HkxType::Int8 | HkxType::Uint8 | HkxType::Int16 | HkxType::Uint16 | HkxType::Half
    ) {
        return None;
    }
    let numelements = numelements?;
    let hex_text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if hex_text.is_empty() || !hex_text.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let bytes_per = match subtype {
        HkxType::Int8 | HkxType::Uint8 => 1,
        HkxType::Int16 | HkxType::Uint16 | HkxType::Half => 2,
        _ => return None,
    };
    let expected = numelements.checked_mul(bytes_per)?.checked_mul(2)?;
    if expected == 0 || hex_text.len() != expected {
        return None;
    }
    let signed = matches!(subtype, HkxType::Int8 | HkxType::Int16);
    let bits = bytes_per * 8;
    let sign_bit: u64 = 1u64 << (bits - 1);
    let modulus: u64 = 1u64 << bits;
    let step = bytes_per * 2;
    let mut values = Vec::with_capacity(numelements);
    for i in 0..numelements {
        let chunk = &hex_text[i * step..(i + 1) * step];
        let raw = u64::from_str_radix(chunk, 16).ok()?;
        if signed && (raw & sign_bit) != 0 {
            let signed_val = (raw as i64) - (modulus as i64);
            values.push(match subtype {
                HkxType::Int8 => HkxValue::I8(signed_val as i8),
                HkxType::Int16 => HkxValue::I16(signed_val as i16),
                _ => unreachable!(),
            });
        } else {
            values.push(match subtype {
                HkxType::Int8 => HkxValue::I8(raw as i8),
                HkxType::Uint8 => HkxValue::U8(raw as u8),
                HkxType::Int16 => HkxValue::I16(raw as i16),
                HkxType::Uint16 => HkxValue::U16(raw as u16),
                HkxType::Half => HkxValue::Half(half_to_f32(raw as u16)),
                _ => unreachable!(),
            });
        }
    }
    Some(values)
}

fn parse_typed_scalar(
    registry: &mut DescriptorRegistry,
    text: &str,
    value_type: HkxType,
    template: &MemberTemplate,
    owner_class: &str,
    object_names: &HashMap<String, usize>,
) -> HavokResult<HkxValue> {
    match value_type {
        HkxType::Bool => parse_bool(text).map(HkxValue::Bool),
        HkxType::Int8 => parse_i64(text).and_then(|value| narrow_i8(value).map(HkxValue::I8)),
        HkxType::Uint8 => parse_u64(text).and_then(|value| narrow_u8(value).map(HkxValue::U8)),
        HkxType::Int16 => parse_i64(text).and_then(|value| narrow_i16(value).map(HkxValue::I16)),
        HkxType::Uint16 => parse_u64(text).and_then(|value| narrow_u16(value).map(HkxValue::U16)),
        HkxType::Half => text.parse::<f32>().map(HkxValue::Half).map_err(|error| {
            HavokError::InvalidInput(format!("invalid half {}: {error}", template.name))
        }),
        HkxType::Int32 => parse_i64(text).and_then(|value| narrow_i32(value).map(HkxValue::I32)),
        HkxType::Enum | HkxType::Flags => {
            // Numeric form: just an integer.
            if let Ok(value) = text.parse::<i64>() {
                return narrow_i32(value).map(HkxValue::I32);
            }
            // Textual form (hkxpack-cli style): resolve via descriptor's enum
            // mapping, searching the owner class first then the etype's
            // declaring class.
            let enum_name = if template.etype.is_empty() {
                &template.ctype
            } else {
                &template.etype
            };
            if !owner_class.is_empty() && !enum_name.is_empty() {
                if let Some(value) = lookup_enum_int(registry, owner_class, enum_name, text) {
                    return Ok(HkxValue::I32(value));
                }
                if let Some(value) = lookup_enum_int(registry, enum_name, enum_name, text) {
                    return Ok(HkxValue::I32(value));
                }
            }
            Err(HavokError::InvalidInput(format!(
                "invalid enum value {text} for {}.{}",
                owner_class, template.name
            )))
        }
        HkxType::Uint32 => parse_u64(text).and_then(|value| narrow_u32(value).map(HkxValue::U32)),
        HkxType::Int64 => parse_i64(text).map(HkxValue::I64),
        HkxType::Uint64 | HkxType::Ulong => parse_u64(text).map(HkxValue::U64),
        HkxType::Real => text.parse::<f32>().map(HkxValue::F32).map_err(|error| {
            HavokError::InvalidInput(format!("invalid real {}: {error}", template.name))
        }),
        HkxType::CString | HkxType::StringPtr => Ok(HkxValue::String {
            value: if text == "null" {
                String::new()
            } else {
                text.to_string()
            },
            is_null: text == "null",
        }),
        HkxType::Pointer | HkxType::FunctionPointer => parse_pointer(text, object_names),
        _ => Ok(parse_scalar(text)),
    }
}

fn looks_like_float(text: &str) -> bool {
    text.contains('.') || text.contains('e') || text.contains('E')
}

fn string_value(text: &str) -> HkxValue {
    HkxValue::String {
        value: text.to_string(),
        is_null: false,
    }
}

fn write_object(
    xml: &mut String,
    object: &HkxObject,
    indent: usize,
    index: usize,
    registry: &mut DescriptorRegistry,
) -> HavokResult<()> {
    push_indent(xml, indent);
    xml.push_str("<hkobject");
    // Always emit a `name` attribute. When the object was loaded from a
    // packfile (where names aren't stored explicitly), synthesize one from
    // the object's position in the file so it matches the pointer format
    // used elsewhere in this writer (`#{index+1:04}`). Without this,
    // pointer cross-references resolve to objects with empty names and
    // any Python caller filtering by object name (e.g. body_id-based
    // collision shape selection in `creation_lib.havok.collision_preview`) breaks.
    let name = match &object.name {
        Some(name) if !name.is_empty() => name.clone(),
        _ => format!("#{:04}", index + 1),
    };
    xml.push_str(&format!(" name=\"{}\"", escape_attr(&name)));
    xml.push_str(&format!(
        " class=\"{}\" signature=\"0x{:08x}\">\n",
        escape_attr(&object.class_name),
        object.signature
    ));
    let templates = registry
        .get_all_members(&object.class_name)
        .unwrap_or_default();
    for member in &object.members {
        let template = templates.iter().find(|t| t.name == member.name).cloned();
        // Skip members whose value equals the template default — matches the
        // SDK pattern where CK omits defaulted fields from tagxml output.
        if let Some(ref t) = template {
            if let Some(ref default_val) = t.default {
                if &member.value == default_val {
                    continue;
                }
            }
        }
        write_member(
            xml,
            member,
            indent + 1,
            registry,
            &object.class_name,
            template.as_ref(),
        )?;
    }
    push_indent(xml, indent);
    xml.push_str("</hkobject>\n");
    Ok(())
}

fn write_member(
    xml: &mut String,
    member: &HkxMember,
    indent: usize,
    registry: &mut DescriptorRegistry,
    owner_class: &str,
    template: Option<&MemberTemplate>,
) -> HavokResult<()> {
    match &member.value {
        HkxValue::Array(values)
            if values
                .iter()
                .any(|value| matches!(value, HkxValue::Object(_))) =>
        {
            if values
                .iter()
                .any(|value| !matches!(value, HkxValue::Object(_)))
            {
                return Err(HavokError::InvalidInput(format!(
                    "mixed object/scalar array {} cannot be written as TagXML",
                    member.name
                )));
            }
            push_indent(xml, indent);
            xml.push_str(&format!(
                "<hkparam name=\"{}\" numelements=\"{}\">\n",
                escape_attr(&member.name),
                values.len()
            ));
            let nested_class = template.map(|t| t.ctype.clone()).unwrap_or_default();
            for value in values {
                if let HkxValue::Object(members) = value {
                    push_indent(xml, indent + 1);
                    xml.push_str("<hkobject>\n");
                    let nested_templates =
                        registry.get_all_members(&nested_class).unwrap_or_default();
                    for nested_member in members {
                        let nested_template = nested_templates
                            .iter()
                            .find(|t| t.name == nested_member.name)
                            .cloned();
                        write_member(
                            xml,
                            nested_member,
                            indent + 2,
                            registry,
                            &nested_class,
                            nested_template.as_ref(),
                        )?;
                    }
                    push_indent(xml, indent + 1);
                    xml.push_str("</hkobject>\n");
                }
            }
            push_indent(xml, indent);
            xml.push_str("</hkparam>\n");
        }
        HkxValue::Array(values) => {
            // Arrays of COMPLEX subtypes (Vector4/Quaternion/QsTransform/...)
            // emit each element wrapped in a single parenthesized group,
            // joined by newlines. Mirrors `py_creation_lib/python/creation_lib/hkxpack/tagwriter.py` lines
            // 164-169: an N-element QsTransform array writes N lines of
            // `(f1 f2 ... f12)`, NOT `(f1 f2 f3 f4)(f5 f6 f7 f8)(f9 f10 f11
            // f12)`. The 3-paren form is reserved for *single* QsTransform
            // fields outside an array. Python's tagreader regex
            // `\(([^)]+)\)` matches each parenthesized group as one element,
            // so the per-element single-paren wrapping is required for the
            // array reader to recover the right element count.
            let subtype = template.map(|t| t.vsubtype).unwrap_or(HkxType::Void);
            // String arrays (hkArray<hkStringPtr> / hkArray<const char*>) emit
            // each element as a `<hkcstring>` child, matching the canonical
            // hkxpack-cli format that `py_creation_lib/python/creation_lib/hkxpack/tagwriter.py` produces
            // (see lines 172-175 there). Joining with spaces would corrupt
            // strings that contain whitespace and confuses the Python
            // tagreader, which expects child elements for STRING arrays.
            if matches!(subtype, HkxType::CString | HkxType::StringPtr) {
                push_indent(xml, indent);
                xml.push_str(&format!(
                    "<hkparam name=\"{}\" numelements=\"{}\">\n",
                    escape_attr(&member.name),
                    values.len()
                ));
                for value in values {
                    let text = match value {
                        HkxValue::String {
                            value: s,
                            is_null: false,
                        } => escape_text(s),
                        HkxValue::String { is_null: true, .. } => String::new(),
                        _ => escape_text(&value_to_text(value)),
                    };
                    push_indent(xml, indent + 1);
                    xml.push_str(&format!("<hkcstring>{text}</hkcstring>\n"));
                }
                push_indent(xml, indent);
                xml.push_str("</hkparam>\n");
                return Ok(());
            }
            let formatted = if is_complex_type(subtype) {
                values
                    .iter()
                    .map(|v| match v {
                        HkxValue::F32List(floats) => format!(
                            "({})",
                            floats
                                .iter()
                                .map(|f| format!("{f:.6}"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        ),
                        _ => value_to_text(v),
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                values
                    .iter()
                    .map(|v| value_to_text_typed(v, registry, owner_class, template))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            push_indent(xml, indent);
            xml.push_str(&format!(
                "<hkparam name=\"{}\" numelements=\"{}\">{}</hkparam>\n",
                escape_attr(&member.name),
                values.len(),
                formatted
            ));
        }
        HkxValue::Object(members) => {
            push_indent(xml, indent);
            xml.push_str(&format!(
                "<hkparam name=\"{}\">\n",
                escape_attr(&member.name)
            ));
            push_indent(xml, indent + 1);
            xml.push_str("<hkobject>\n");
            let nested_class = template.map(|t| t.ctype.clone()).unwrap_or_default();
            let nested_templates = registry.get_all_members(&nested_class).unwrap_or_default();
            for nested_member in members {
                let nested_template = nested_templates
                    .iter()
                    .find(|t| t.name == nested_member.name)
                    .cloned();
                write_member(
                    xml,
                    nested_member,
                    indent + 2,
                    registry,
                    &nested_class,
                    nested_template.as_ref(),
                )?;
            }
            push_indent(xml, indent + 1);
            xml.push_str("</hkobject>\n");
            push_indent(xml, indent);
            xml.push_str("</hkparam>\n");
        }
        value => {
            push_indent(xml, indent);
            xml.push_str(&format!(
                "<hkparam name=\"{}\">{}</hkparam>\n",
                escape_attr(&member.name),
                value_to_text_typed(value, registry, owner_class, template)
            ));
        }
    }
    Ok(())
}

/// Like `value_to_text`, but converts int values to their enum name when the
/// member template marks the field as TYPE_ENUM/TYPE_FLAGS and the registry
/// has a name for the value. Mirrors Python's `HKXEnumMember.value` semantics.
/// Also routes F32List values through `format_complex_f32list` when the
/// effective type is COMPLEX, so vector/quaternion/qstransform fields emit
/// in Python-compatible parenthesized form: `(x y z w)` for Vector4 and
/// Quaternion, `(x y z w)(...)(...)` for QsTransform/Transform/Matrix3,
/// and four groups for Matrix4. Matches `py_creation_lib/python/creation_lib/hkxpack/tagwriter.py:_write_member`.
fn value_to_text_typed(
    value: &HkxValue,
    registry: &mut DescriptorRegistry,
    owner_class: &str,
    template: Option<&MemberTemplate>,
) -> String {
    if let (Some(template), HkxValue::I32(int_value)) = (template, value) {
        if matches!(template.vtype, HkxType::Enum | HkxType::Flags)
            && !template.etype.is_empty()
            && !owner_class.is_empty()
        {
            let name = registry.get_enum_value(owner_class, &template.etype, *int_value);
            // get_enum_value returns the integer-as-string when the enum
            // doesn't carry a name for the value. Detect that by trying to
            // parse it back; if it parses, fall through to the standard path.
            if name.parse::<i32>().is_err() {
                return escape_text(&name);
            }
        }
    }
    if let (Some(template), HkxValue::F32List(values)) = (template, value) {
        if let Some(formatted) = format_complex_f32list(values, template.vtype) {
            return formatted;
        }
    }
    value_to_text(value)
}

fn is_complex_type(hkx_type: HkxType) -> bool {
    matches!(
        hkx_type,
        HkxType::Vector4
            | HkxType::Quaternion
            | HkxType::Matrix3
            | HkxType::Matrix4
            | HkxType::Transform
            | HkxType::QsTransform
    )
}

/// Format an `F32List` value as a Python-compatible parenthesized COMPLEX
/// literal. Returns `None` when the type isn't COMPLEX, letting the caller
/// fall through to the flat space-separated default.
fn format_complex_f32list(values: &[f32], hkx_type: HkxType) -> Option<String> {
    match hkx_type {
        HkxType::Vector4 | HkxType::Quaternion => {
            // Single 4-tuple. Python emits `(a b c d)`.
            if values.len() < 4 {
                return None;
            }
            Some(format!(
                "({:.6} {:.6} {:.6} {:.6})",
                values[0], values[1], values[2], values[3]
            ))
        }
        HkxType::QsTransform | HkxType::Transform | HkxType::Matrix3 => {
            // 3 groups of 4. Python emits `(a b c d)(e f g h)(i j k l)`.
            Some(chunked_complex_text(values, 3))
        }
        HkxType::Matrix4 => {
            // 4 groups of 4.
            Some(chunked_complex_text(values, 4))
        }
        _ => None,
    }
}

fn chunked_complex_text(values: &[f32], group_count: usize) -> String {
    let mut out = String::new();
    for i in 0..group_count {
        let base = i * 4;
        if base + 4 > values.len() {
            break;
        }
        out.push_str(&format!(
            "({:.6} {:.6} {:.6} {:.6})",
            values[base],
            values[base + 1],
            values[base + 2],
            values[base + 3]
        ));
    }
    out
}

fn value_to_text(value: &HkxValue) -> String {
    match value {
        HkxValue::Void => String::new(),
        HkxValue::Bool(value) => {
            if *value {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        HkxValue::I8(value) => value.to_string(),
        HkxValue::U8(value) => value.to_string(),
        HkxValue::I16(value) => value.to_string(),
        HkxValue::U16(value) => value.to_string(),
        HkxValue::I32(value) => value.to_string(),
        HkxValue::U32(value) => value.to_string(),
        HkxValue::I64(value) => value.to_string(),
        HkxValue::U64(value) => value.to_string(),
        HkxValue::Half(value) => format!("{value:.6}"),
        HkxValue::F32(value) => format!("{value:.6}"),
        HkxValue::F32List(values) => values
            .iter()
            .map(|value| format!("{value:.6}"))
            .collect::<Vec<_>>()
            .join(" "),
        HkxValue::String { value, is_null } => {
            if *is_null {
                "null".to_string()
            } else {
                escape_text(value)
            }
        }
        HkxValue::Pointer(Some(index)) => format!("#{:04}", index + 1),
        HkxValue::Pointer(None) => "null".to_string(),
        HkxValue::Array(values) => values
            .iter()
            .map(value_to_text)
            .collect::<Vec<_>>()
            .join(" "),
        HkxValue::Object(_) | HkxValue::TypedObject { .. } => String::new(),
        // PendingPtr must be resolved before writing; emit the raw name as a
        // best-effort fallback so callers get something parseable rather than
        // a silent empty string.
        HkxValue::PendingPtr(name) => name.clone(),
    }
}

fn parse_signature(signature: Option<&str>) -> HavokResult<u32> {
    let Some(signature) = signature else {
        return Ok(0);
    };
    let value = signature.strip_prefix("0x").unwrap_or(signature);
    u32::from_str_radix(value, 16)
        .or_else(|_| signature.parse::<u32>())
        .map_err(|error| {
            HavokError::InvalidInput(format!("invalid object signature {signature}: {error}"))
        })
}

fn parse_pointer(text: &str, object_names: &HashMap<String, usize>) -> HavokResult<HkxValue> {
    if text == "null" || text == "#null" {
        Ok(HkxValue::Pointer(None))
    } else if let Some(index) = parse_pointer_index(text) {
        Ok(HkxValue::Pointer(Some(index)))
    } else if let Some(index) = object_names.get(text) {
        Ok(HkxValue::Pointer(Some(*index)))
    } else {
        Err(HavokError::InvalidInput(format!("invalid pointer {text}")))
    }
}

fn parse_pointer_index(text: &str) -> Option<usize> {
    let id = text.strip_prefix('#')?.parse::<usize>().ok()?;
    id.checked_sub(1)
}

fn parse_bool(text: &str) -> HavokResult<bool> {
    match text {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(HavokError::InvalidInput(format!("invalid bool {text}"))),
    }
}

fn parse_i64(text: &str) -> HavokResult<i64> {
    text.parse::<i64>()
        .map_err(|error| HavokError::InvalidInput(format!("invalid integer {text}: {error}")))
}

fn parse_u64(text: &str) -> HavokResult<u64> {
    text.parse::<u64>().map_err(|error| {
        HavokError::InvalidInput(format!("invalid unsigned integer {text}: {error}"))
    })
}

fn narrow_i8(value: i64) -> HavokResult<i8> {
    i8::try_from(value)
        // Havok XML sometimes stores signed ints as unsigned bit-patterns (e.g. 255 → -1).
        .or_else(|_| u8::try_from(value).map(|v| v as i8))
        .map_err(|error| HavokError::InvalidInput(error.to_string()))
}

fn narrow_u8(value: u64) -> HavokResult<u8> {
    u8::try_from(value).map_err(|error| HavokError::InvalidInput(error.to_string()))
}

fn narrow_i16(value: i64) -> HavokResult<i16> {
    i16::try_from(value)
        // Havok XML sometimes stores signed ints as unsigned bit-patterns (e.g. 65535 → -1).
        .or_else(|_| u16::try_from(value).map(|v| v as i16))
        .map_err(|error| HavokError::InvalidInput(error.to_string()))
}

fn narrow_u16(value: u64) -> HavokResult<u16> {
    u16::try_from(value).map_err(|error| HavokError::InvalidInput(error.to_string()))
}

fn narrow_i32(value: i64) -> HavokResult<i32> {
    i32::try_from(value)
        // Havok XML sometimes stores signed ints as unsigned bit-patterns (e.g. 4294967295 → -1).
        .or_else(|_| u32::try_from(value).map(|v| v as i32))
        .map_err(|error| HavokError::InvalidInput(error.to_string()))
}

fn narrow_u32(value: u64) -> HavokResult<u32> {
    u32::try_from(value).map_err(|error| HavokError::InvalidInput(error.to_string()))
}

fn push_indent(xml: &mut String, indent: usize) {
    for _ in 0..indent {
        xml.push_str("    ");
    }
}

fn escape_attr(value: &str) -> String {
    escape_text(value)
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
