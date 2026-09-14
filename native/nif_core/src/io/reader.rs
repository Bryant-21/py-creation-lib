use std::io::{Cursor, Read, Seek, SeekFrom};

use indexmap::IndexMap;

use crate::expr::{EvalContext, NifExpr, Value};
use crate::model::{NifBlock, NifFile, NifHeader, NifValue, RawBlockContext};
use crate::schema::{FieldDef, NifSchema};

use super::basic_io::{BasicReader, IoError};

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("io: {0}")]
    Io(#[from] IoError),
    #[error("read: {0}")]
    Other(String),
}

// --- Version helpers ---

pub fn pack_version(v: (u8, u8, u8, u8)) -> u32 {
    ((v.0 as u32) << 24) | ((v.1 as u32) << 16) | ((v.2 as u32) << 8) | (v.3 as u32)
}

pub fn parse_version_string(s: &str) -> (u8, u8, u8, u8) {
    let mut parts = [0u8; 4];
    for (i, p) in s.split('.').take(4).enumerate() {
        parts[i] = p.trim().parse::<u32>().unwrap_or(0).min(255) as u8;
    }
    (parts[0], parts[1], parts[2], parts[3])
}

pub fn parse_header_version_string(header: &str) -> (u8, u8, u8, u8) {
    let bytes = header.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            // Try to match d+.d+.d+.d+
            let start = i;
            let mut dots = 0;
            let mut j = i;
            while j < bytes.len() {
                let b = bytes[j];
                if b.is_ascii_digit() {
                    j += 1;
                } else if b == b'.' {
                    dots += 1;
                    j += 1;
                } else {
                    break;
                }
            }
            if dots == 3 {
                let slice = &header[start..j];
                let parts: Vec<&str> = slice.split('.').collect();
                if parts.len() == 4 {
                    let a = parts[0].parse::<u32>().unwrap_or(0).min(255) as u8;
                    let b = parts[1].parse::<u32>().unwrap_or(0).min(255) as u8;
                    let c = parts[2].parse::<u32>().unwrap_or(0).min(255) as u8;
                    let d = parts[3].parse::<u32>().unwrap_or(0).min(255) as u8;
                    return (a, b, c, d);
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    (0, 0, 0, 0)
}

// Convert a NifValue to an evaluator Value for condition evaluation.
fn nif_to_eval(v: &NifValue) -> Value {
    match v {
        NifValue::Null => Value::Null,
        NifValue::Bool(b) => Value::Bool(*b),
        NifValue::Int(i) => Value::Int(*i),
        NifValue::UInt(u) => Value::Int(*u as i64),
        NifValue::Float(f) => Value::Float(*f),
        NifValue::FloatNan(_) => Value::Int(0),
        NifValue::Ref(r) => Value::Int(*r as i64),
        NifValue::String(s) => Value::Bool(!s.trim_matches('\0').trim().is_empty()),
        _ => Value::Int(0),
    }
}

// Evaluator context that looks at both the current block fields and the file-level version globals.
struct BlockCtx<'a> {
    block_fields: &'a IndexMap<String, NifValue>,
    version_packed: u32,
    user_version: u32,
    bs_version: u32,
    arg: Value,
}

impl<'a> EvalContext for BlockCtx<'a> {
    fn get_field(&self, path: &str) -> Value {
        // Prefer block-local fields (so a struct-scoped ARG entry wins over globals).
        if let Some(v) = self.block_fields.get(path) {
            return nif_to_eval(v);
        }
        match path {
            "Version" => Value::Int(self.version_packed as i64),
            "User Version" | "User Version 2" => Value::Int(self.user_version as i64),
            "BS Header\\BS Version" => Value::Int(self.bs_version as i64),
            "ARG" => self.arg.clone(),
            "INFINITY" => Value::Float(f64::INFINITY),
            _ => Value::Null,
        }
    }
    fn get_field_len(&self, path: &str) -> Option<usize> {
        match self.block_fields.get(path) {
            Some(NifValue::Array(v)) => Some(v.len()),
            Some(NifValue::Bytes(b)) => Some(b.len()),
            _ => None,
        }
    }
    fn get_field_len2(&self, path: &str) -> Option<usize> {
        match self.block_fields.get(path) {
            Some(NifValue::Array(v)) => Some(
                v.iter()
                    .map(|item| match item {
                        NifValue::Array(row) => row.len(),
                        _ => 1,
                    })
                    .sum(),
            ),
            Some(NifValue::Bytes(b)) => Some(b.len()),
            _ => None,
        }
    }
}

// --- Header ---

pub fn read_header<R: Read + Seek>(reader: &mut BasicReader<R>) -> Result<NifHeader, ReadError> {
    let mut h = NifHeader::default();
    h.header_string = reader.read_header_string()?;
    h.version = parse_header_version_string(&h.header_string);
    h.version_packed = pack_version(h.version);
    let ver = h.version_packed;

    // Copyright strings (version <= 3.1.0.0)
    if ver <= 0x03010000 {
        for _ in 0..3 {
            reader.read_header_string()?;
        }
    }

    // FileVersion (ulittle32) — since 3.1.0.1
    if ver >= 0x03010001 {
        reader.read_ulittle32()?;
    }

    // Endian Type byte — since 20.0.0.3
    if ver >= 0x14000003 {
        h.endian_type = reader.read_byte()?;
        reader.big_endian = h.endian_type == 0;
    }

    // User Version (uint) — since 10.0.1.8
    if ver >= 0x0A000108 {
        h.user_version = reader.read_uint()?;
    }

    // Num Blocks (uint) — since 3.1.0.1
    if ver >= 0x03010001 {
        h.num_blocks = reader.read_uint()?;
    }

    // BS header
    let ver_10012: u32 = 0x0A000102;
    let ver_20207: u32 = 0x14020007;
    let ver_20005: u32 = 0x14000005;
    let ver_10100: u32 = 0x0A010000;
    let ver_20004: u32 = 0x14000004;
    let has_bs_header = ver == ver_10012
        || ((ver == ver_20207
            || ver == ver_20005
            || (ver >= ver_10100 && ver <= ver_20004 && h.user_version <= 11))
            && h.user_version >= 3);

    if has_bs_header {
        h.bs_version = reader.read_ulittle32()?;
        h.creator = reader.read_export_string()?;
        if h.bs_version > 130 {
            reader.read_uint()?; // unknown int
        }
        h.export_info = Vec::new();
        if h.bs_version < 131 {
            h.export_info.push(reader.read_export_string()?); // process script
        }
        h.export_info.push(reader.read_export_string()?); // export script
        if h.bs_version >= 103 && h.bs_version < 170 {
            h.export_info.push(reader.read_export_string()?); // max filepath
        }
        if h.bs_version >= 170 {
            let sf_len = reader.read_byte()? as usize;
            if sf_len > 0 {
                h.sf_export_data = reader.read_n_bytes(sf_len)?;
            }
        }
    }

    // Metadata (ByteArray) — since 30.0.0.0
    if ver >= 0x1E000000 {
        let meta_size = reader.read_uint()? as usize;
        if meta_size > 0 {
            let _ = reader.read_n_bytes(meta_size)?;
        }
    }

    // Block types — since 5.0.0.1
    if ver >= 0x05000001 {
        let num_types = reader.read_ushort()? as usize;
        h.block_type_names = Vec::with_capacity(num_types);
        for _ in 0..num_types {
            h.block_type_names.push(reader.read_sized_string()?);
        }
        h.block_type_index = Vec::with_capacity(h.num_blocks as usize);
        for _ in 0..h.num_blocks as usize {
            let raw = reader.read_ushort()?;
            h.block_type_index.push(raw & 0x7FFF);
        }
    }

    // Block sizes — since 20.2.0.5
    if ver >= 0x14020005 {
        h.block_sizes = Vec::with_capacity(h.num_blocks as usize);
        for _ in 0..h.num_blocks as usize {
            h.block_sizes.push(reader.read_uint()?);
        }
    }

    // String table — since 20.1.0.1
    if ver >= 0x14010001 {
        let num_strings = reader.read_uint()? as usize;
        h.max_string_length = reader.read_uint()?;
        h.strings = Vec::with_capacity(num_strings);
        for _ in 0..num_strings {
            h.strings.push(reader.read_sized_string()?);
        }
    }

    // Groups — since 5.0.0.6
    if ver >= 0x05000006 {
        h.num_groups = reader.read_uint()?;
        h.groups = Vec::with_capacity(h.num_groups as usize);
        for _ in 0..h.num_groups as usize {
            h.groups.push(reader.read_uint()?);
        }
    }

    Ok(h)
}

// --- Field reading ---

fn should_read_field(
    fdef: &FieldDef,
    block_fields: &IndexMap<String, NifValue>,
    actual_type: &str,
    schema: &NifSchema,
    version_packed: u32,
    user_version: u32,
    bs_version: u32,
) -> bool {
    if fdef.is_abstract {
        return false;
    }
    if let Some(since) = fdef.since {
        let sp = pack_version(parse_version_string(since));
        if version_packed < sp {
            return false;
        }
    }
    if let Some(until) = fdef.until {
        let up = pack_version(parse_version_string(until));
        if version_packed > up {
            return false;
        }
    }
    if let Some(vc) = fdef.vercond {
        if !vc.is_empty() {
            let ctx = BlockCtx {
                block_fields,
                version_packed,
                user_version,
                bs_version,
                arg: Value::Null,
            };
            match NifExpr::cached(vc) {
                Ok(e) => {
                    if !e.evaluate_bool(&ctx) {
                        return false;
                    }
                }
                Err(_) => return false,
            }
        }
    }
    if let Some(c) = fdef.cond {
        if !c.is_empty() {
            let ctx = BlockCtx {
                block_fields,
                version_packed,
                user_version,
                bs_version,
                arg: Value::Null,
            };
            match NifExpr::cached(c) {
                Ok(e) => {
                    if !e.evaluate_bool(&ctx) {
                        return false;
                    }
                }
                Err(_) => return false,
            }
        }
    }
    if let Some(only_t) = fdef.only_t {
        if !schema.is_subtype_of(actual_type, only_t) {
            return false;
        }
    }
    if let Some(exclude_t) = fdef.exclude_t {
        if schema.is_subtype_of(actual_type, exclude_t) {
            return false;
        }
    }
    true
}

fn resolve_length(
    length_expr: &str,
    block_fields: &IndexMap<String, NifValue>,
    version_packed: u32,
    user_version: u32,
    bs_version: u32,
) -> usize {
    if let Some(v) = block_fields.get(length_expr) {
        return v.as_usize();
    }
    let ctx = BlockCtx {
        block_fields,
        version_packed,
        user_version,
        bs_version,
        arg: Value::Null,
    };
    match NifExpr::cached(length_expr) {
        Ok(e) => {
            let v = e.evaluate(&ctx).as_int();
            if v < 0 { 0 } else { v as usize }
        }
        Err(_) => 0,
    }
}

fn resolve_arg(
    arg_str: &str,
    block_fields: &IndexMap<String, NifValue>,
    version_packed: u32,
    user_version: u32,
    bs_version: u32,
) -> Value {
    let ctx = BlockCtx {
        block_fields,
        version_packed,
        user_version,
        bs_version,
        arg: Value::Null,
    };
    match NifExpr::cached(arg_str) {
        Ok(e) => e.evaluate(&ctx),
        Err(_) => Value::Null,
    }
}

fn eval_to_nif(v: &Value) -> NifValue {
    match v {
        Value::Int(i) => NifValue::Int(*i),
        Value::Float(f) => NifValue::Float(*f),
        Value::Bool(b) => NifValue::Bool(*b),
        Value::Null => NifValue::Null,
    }
}

fn read_fast_array<R: Read + Seek>(
    type_name: &str,
    count: usize,
    reader: &mut BasicReader<R>,
) -> Option<Result<Vec<NifValue>, IoError>> {
    if count == 0 {
        return Some(Ok(Vec::new()));
    }
    Some(match type_name {
        "int" => reader.read_bulk_i32(count),
        "Ref" | "Ptr" => reader.read_bulk_i32(count).map(|v| {
            v.into_iter()
                .map(|x| match x {
                    NifValue::Int(i) => NifValue::Ref(i as i32),
                    other => other,
                })
                .collect()
        }),
        "uint" | "StringOffset" | "ulittle32" => reader.read_bulk_u32(count),
        "float" => reader.read_bulk_f32(count),
        "ushort" => reader.read_bulk_u16(count),
        "short" | "BlockTypeIndex" => reader.read_bulk_i16(count),
        "byte" => reader.read_bulk_u8(count),
        "sbyte" => reader.read_bulk_i8(count),
        _ => return None,
    })
}

fn read_triangle_array<R: Read + Seek>(
    count: usize,
    reader: &mut BasicReader<R>,
) -> Result<Vec<NifValue>, IoError> {
    let raw = reader.read_n_bytes(count * 6)?;
    let be = reader.big_endian;
    Ok(raw
        .chunks_exact(6)
        .map(|chunk| {
            let mut fields = IndexMap::new();
            for (name, bytes) in [
                ("v1", [chunk[0], chunk[1]]),
                ("v2", [chunk[2], chunk[3]]),
                ("v3", [chunk[4], chunk[5]]),
            ] {
                let value = if be {
                    u16::from_be_bytes(bytes)
                } else {
                    u16::from_le_bytes(bytes)
                };
                fields.insert(name.to_string(), NifValue::UInt(value as u64));
            }
            NifValue::Struct(fields)
        })
        .collect())
}

fn read_float_vec3<R: Read + Seek>(reader: &mut BasicReader<R>) -> Result<NifValue, IoError> {
    let mut fields = IndexMap::new();
    fields.insert("x".to_string(), reader.read_float()?);
    fields.insert("y".to_string(), reader.read_float()?);
    fields.insert("z".to_string(), reader.read_float()?);
    Ok(NifValue::Struct(fields))
}

fn read_half_vec3<R: Read + Seek>(reader: &mut BasicReader<R>) -> Result<NifValue, IoError> {
    let mut fields = IndexMap::new();
    fields.insert("x".to_string(), reader.read_hfloat()?);
    fields.insert("y".to_string(), reader.read_hfloat()?);
    fields.insert("z".to_string(), reader.read_hfloat()?);
    Ok(NifValue::Struct(fields))
}

fn read_normbyte_vec3<R: Read + Seek>(reader: &mut BasicReader<R>) -> Result<NifValue, IoError> {
    let mut fields = IndexMap::new();
    fields.insert("x".to_string(), NifValue::Float(reader.read_normbyte()?));
    fields.insert("y".to_string(), NifValue::Float(reader.read_normbyte()?));
    fields.insert("z".to_string(), NifValue::Float(reader.read_normbyte()?));
    Ok(NifValue::Struct(fields))
}

fn read_half_tex_coord<R: Read + Seek>(reader: &mut BasicReader<R>) -> Result<NifValue, IoError> {
    let mut fields = IndexMap::with_capacity(2);
    fields.insert("u".to_string(), reader.read_hfloat()?);
    fields.insert("v".to_string(), reader.read_hfloat()?);
    Ok(NifValue::Struct(fields))
}

fn read_byte_color4<R: Read + Seek>(reader: &mut BasicReader<R>) -> Result<NifValue, IoError> {
    let mut fields = IndexMap::with_capacity(4);
    fields.insert("r".to_string(), NifValue::UInt(reader.read_byte()? as u64));
    fields.insert("g".to_string(), NifValue::UInt(reader.read_byte()? as u64));
    fields.insert("b".to_string(), NifValue::UInt(reader.read_byte()? as u64));
    fields.insert("a".to_string(), NifValue::UInt(reader.read_byte()? as u64));
    Ok(NifValue::Struct(fields))
}

fn read_fixed_array<R: Read + Seek>(
    count: usize,
    reader: &mut BasicReader<R>,
    mut read_item: impl FnMut(&mut BasicReader<R>) -> Result<NifValue, IoError>,
) -> Result<NifValue, IoError> {
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        items.push(read_item(reader)?);
    }
    Ok(NifValue::Array(items))
}

fn read_bs_vertex_data<R: Read + Seek>(
    type_name: &str,
    attributes: u64,
    reader: &mut BasicReader<R>,
) -> Result<NifValue, IoError> {
    let mut field_count = 0;
    field_count += 2 * usize::from(attributes & 0x1 != 0);
    field_count += usize::from(attributes & 0x2 != 0);
    field_count += 2 * usize::from(attributes & 0x8 != 0);
    field_count += 2 * usize::from(attributes & 0x18 == 0x18);
    field_count += usize::from(attributes & 0x20 != 0);
    field_count += 2 * usize::from(attributes & 0x40 != 0);
    field_count += usize::from(attributes & 0x100 != 0);
    let mut fields = IndexMap::with_capacity(field_count);
    if type_name == "BSVertexDataSSE" {
        if attributes & 0x1 != 0 {
            fields.insert("Vertex".to_string(), read_float_vec3(reader)?);
        }
        if attributes & 0x11 == 0x11 {
            fields.insert("Bitangent X".to_string(), reader.read_float()?);
        }
        if attributes & 0x11 == 0x1 {
            fields.insert(
                "Unused W".to_string(),
                NifValue::UInt(reader.read_uint()? as u64),
            );
        }
    } else if attributes & 0x401 == 0x401 {
        fields.insert("Vertex".to_string(), read_float_vec3(reader)?);
        if attributes & 0x411 == 0x411 {
            fields.insert("Bitangent X".to_string(), reader.read_float()?);
        }
        if attributes & 0x411 == 0x401 {
            fields.insert(
                "Unused W".to_string(),
                NifValue::UInt(reader.read_uint()? as u64),
            );
        }
    } else if attributes & 0x401 == 0x1 {
        fields.insert("Vertex".to_string(), read_half_vec3(reader)?);
        if attributes & 0x411 == 0x11 {
            fields.insert("Bitangent X".to_string(), reader.read_hfloat()?);
        }
        if attributes & 0x411 == 0x1 {
            fields.insert(
                "Unused W".to_string(),
                NifValue::UInt(reader.read_ushort()? as u64),
            );
        }
    }

    if attributes & 0x2 != 0 {
        fields.insert("UV".to_string(), read_half_tex_coord(reader)?);
    }
    if attributes & 0x8 != 0 {
        fields.insert("Normal".to_string(), read_normbyte_vec3(reader)?);
        fields.insert(
            "Bitangent Y".to_string(),
            NifValue::Float(reader.read_normbyte()?),
        );
    }
    if attributes & 0x18 == 0x18 {
        fields.insert("Tangent".to_string(), read_normbyte_vec3(reader)?);
        fields.insert(
            "Bitangent Z".to_string(),
            NifValue::Float(reader.read_normbyte()?),
        );
    }
    if attributes & 0x20 != 0 {
        fields.insert("Vertex Colors".to_string(), read_byte_color4(reader)?);
    }
    if attributes & 0x40 != 0 {
        fields.insert(
            "Bone Weights".to_string(),
            read_fixed_array(4, reader, |r| r.read_hfloat())?,
        );
        fields.insert(
            "Bone Indices".to_string(),
            read_fixed_array(4, reader, |r| Ok(NifValue::UInt(r.read_byte()? as u64)))?,
        );
    }
    if attributes & 0x100 != 0 {
        fields.insert("Eye Data".to_string(), reader.read_float()?);
    }

    debug_assert_eq!(fields.len(), field_count);
    Ok(NifValue::Struct(fields))
}

fn read_bs_vertex_data_array<R: Read + Seek>(
    type_name: &str,
    count: usize,
    attributes: u64,
    reader: &mut BasicReader<R>,
) -> Result<Vec<NifValue>, IoError> {
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(read_bs_vertex_data(type_name, attributes, reader)?);
    }
    Ok(out)
}

fn read_value<R: Read + Seek>(
    type_name: &str,
    template: Option<&str>,
    arg: Option<&Value>,
    parent_fields: &IndexMap<String, NifValue>,
    schema: &NifSchema,
    reader: &mut BasicReader<R>,
    version_packed: u32,
    user_version: u32,
    bs_version: u32,
    strings: &[String],
    depth: u32,
) -> Result<NifValue, ReadError> {
    // Resolve #T# placeholder
    let mut tn: &str = type_name;
    if tn == "#T#" {
        if let Some(t) = template {
            tn = t;
        }
    }

    // Basic type?
    if schema.get_basic(tn).is_some()
        || matches!(
            tn,
            "string" | "bool" | "NiFixedString" | "SizedString" | "SizedString16"
        )
    {
        return Ok(reader.read_basic(tn, version_packed, strings)?);
    }
    // Enum -> read storage
    if let Some(e) = schema.get_enum(tn) {
        return Ok(reader.read_basic(e.storage, version_packed, strings)?);
    }
    if let Some(b) = schema.get_bitflag(tn) {
        return Ok(reader.read_basic(b.storage, version_packed, strings)?);
    }
    if let Some(b) = schema.get_bitfield(tn) {
        return Ok(reader.read_basic(b.storage, version_packed, strings)?);
    }
    // Struct
    if schema.get_struct(tn).is_some() {
        return read_struct(
            tn,
            template,
            arg,
            parent_fields,
            schema,
            reader,
            version_packed,
            user_version,
            bs_version,
            strings,
            depth + 1,
        );
    }
    // Fallback: read as uint
    Ok(NifValue::UInt(reader.read_uint()? as u64))
}

fn read_struct<R: Read + Seek>(
    type_name: &str,
    template: Option<&str>,
    arg: Option<&Value>,
    _parent_fields: &IndexMap<String, NifValue>,
    schema: &NifSchema,
    reader: &mut BasicReader<R>,
    version_packed: u32,
    user_version: u32,
    bs_version: u32,
    strings: &[String],
    depth: u32,
) -> Result<NifValue, ReadError> {
    let s = match schema.get_struct(type_name) {
        Some(s) => s,
        None => return Err(ReadError::Other(format!("unknown struct: {}", type_name))),
    };

    let mut fields: IndexMap<String, NifValue> = IndexMap::new();
    // Inject ARG for condition evaluation
    if let Some(arg_val) = arg {
        if !matches!(arg_val, Value::Null) {
            fields.insert("ARG".to_string(), eval_to_nif(arg_val));
        }
    }

    for fdef in s.fields.iter() {
        if !should_read_field(
            fdef,
            &fields,
            type_name,
            schema,
            version_packed,
            user_version,
            bs_version,
        ) {
            continue;
        }

        let key = if let Some(sfx) = fdef.suffix {
            format!("{}:{}", fdef.name, sfx)
        } else {
            fdef.name.to_string()
        };

        let val = read_field_value(
            fdef,
            &mut fields,
            type_name,
            template,
            schema,
            reader,
            version_packed,
            user_version,
            bs_version,
            strings,
            depth,
        )?;
        fields.insert(key, val);
    }

    fields.shift_remove("ARG");
    Ok(NifValue::Struct(fields))
}

fn read_field_value<R: Read + Seek>(
    fdef: &FieldDef,
    block_fields: &mut IndexMap<String, NifValue>,
    _actual_type: &str,
    struct_template: Option<&str>,
    schema: &NifSchema,
    reader: &mut BasicReader<R>,
    version_packed: u32,
    user_version: u32,
    bs_version: u32,
    strings: &[String],
    depth: u32,
) -> Result<NifValue, ReadError> {
    // Resolve #T#
    let mut field_type: &str = fdef.type_name;
    let mut field_template: Option<&str> = fdef.template;
    if let Some(tmpl) = struct_template {
        if field_type == "#T#" {
            field_type = tmpl;
        }
        if field_template == Some("#T#") {
            field_template = Some(tmpl);
        }
    }

    // Binary blob
    if fdef.is_binary {
        if let Some(len_expr) = fdef.length {
            let count = resolve_length(
                len_expr,
                block_fields,
                version_packed,
                user_version,
                bs_version,
            );
            let bytes = reader.read_n_bytes(count)?;
            return Ok(NifValue::Bytes(bytes));
        }
        return Ok(NifValue::Bytes(Vec::new()));
    }

    // Recursive depth guard
    if fdef.recursive && depth > 64 {
        return Ok(NifValue::Null);
    }

    // Resolve ARG for nested evaluation
    let arg_val = fdef
        .arg
        .map(|a| resolve_arg(a, block_fields, version_packed, user_version, bs_version));

    if let Some(len_expr) = fdef.length {
        let count = resolve_length(
            len_expr,
            block_fields,
            version_packed,
            user_version,
            bs_version,
        );

        if let Some(width_expr) = fdef.width {
            // Jagged or uniform 2D
            let widths: Option<Vec<usize>> = block_fields.get(width_expr).and_then(|v| match v {
                NifValue::Array(arr) => Some(arr.iter().map(|x| x.as_usize()).collect()),
                _ => None,
            });
            let uniform_width: usize = if widths.is_none() {
                resolve_length(
                    width_expr,
                    block_fields,
                    version_packed,
                    user_version,
                    bs_version,
                )
            } else {
                0
            };

            let mut outer = Vec::with_capacity(count);
            for i in 0..count {
                let w = match &widths {
                    Some(v) => v.get(i).copied().unwrap_or(0),
                    None => uniform_width,
                };
                let mut row = Vec::with_capacity(w);
                for _ in 0..w {
                    let v = read_value(
                        field_type,
                        field_template,
                        arg_val.as_ref(),
                        block_fields,
                        schema,
                        reader,
                        version_packed,
                        user_version,
                        bs_version,
                        strings,
                        depth,
                    )?;
                    row.push(v);
                }
                outer.push(NifValue::Array(row));
            }
            return Ok(NifValue::Array(outer));
        }

        // 1D array — fast path for numeric basic types (no arg, no template)
        if field_type == "Triangle" && field_template.is_none() {
            return Ok(NifValue::Array(read_triangle_array(count, reader)?));
        }

        if matches!(field_type, "BSVertexData" | "BSVertexDataSSE") && field_template.is_none() {
            let attributes = arg_val.as_ref().map(Value::as_int).unwrap_or(0).max(0) as u64;
            return Ok(NifValue::Array(read_bs_vertex_data_array(
                field_type, count, attributes, reader,
            )?));
        }

        if let Some(res) = read_fast_array(field_type, count, reader) {
            return Ok(NifValue::Array(res?));
        }

        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let v = read_value(
                field_type,
                field_template,
                arg_val.as_ref(),
                block_fields,
                schema,
                reader,
                version_packed,
                user_version,
                bs_version,
                strings,
                depth,
            )?;
            out.push(v);
        }
        return Ok(NifValue::Array(out));
    }

    if fdef.recursive {
        return read_struct(
            field_type,
            field_template,
            arg_val.as_ref(),
            block_fields,
            schema,
            reader,
            version_packed,
            user_version,
            bs_version,
            strings,
            depth + 1,
        );
    }

    read_value(
        field_type,
        field_template,
        arg_val.as_ref(),
        block_fields,
        schema,
        reader,
        version_packed,
        user_version,
        bs_version,
        strings,
        depth,
    )
}

// --- NifReader ---

pub struct NifReader;

fn read_block_fields<R: Read + Seek>(
    block: &mut NifBlock,
    reader: &mut BasicReader<R>,
    expected_size: Option<u32>,
    schema: &NifSchema,
    version_packed: u32,
    user_version: u32,
    bs_version: u32,
    strings: &[String],
) -> Result<(), ReadError> {
    let block_start = reader.pos();
    let all_fields = schema.get_all_field_plan(&block.type_name);
    if !all_fields.is_empty() {
        for entry in all_fields.iter() {
            let fdef = entry.fdef;
            if !should_read_field(
                fdef,
                &block.fields,
                &block.type_name,
                schema,
                version_packed,
                user_version,
                bs_version,
            ) {
                continue;
            }
            let result = read_field_value(
                fdef,
                &mut block.fields,
                &block.type_name,
                None,
                schema,
                reader,
                version_packed,
                user_version,
                bs_version,
                strings,
                0,
            );
            match result {
                Ok(val) => {
                    block.fields.insert(entry.key.clone(), val);
                }
                Err(_) => {
                    break;
                }
            }
        }
        block.fields.shift_remove("ARG");
    } else if let Some(size) = expected_size {
        block.remainder = reader.read_n_bytes(size as usize)?;
    }

    if let Some(size) = expected_size {
        let bytes_read = (reader.pos() - block_start) as u32;
        if bytes_read < size {
            let rem = (size - bytes_read) as usize;
            let r = reader.read_n_bytes(rem)?;
            if block.remainder.is_empty() {
                block.remainder = r;
            } else {
                block.remainder.extend(r);
            }
        }
    }

    Ok(())
}

impl NifReader {
    /// Read only the blocks that can contribute external asset dependencies.
    ///
    /// Sized-block NIFs let this skip geometry payloads without constructing their
    /// values. Older formats without block boundaries must use the full reader.
    pub fn read_referenced_asset_paths<R: Read + Seek>(
        source: R,
        schema: &NifSchema,
    ) -> Result<Option<crate::model::ReferencedAssetPaths>, ReadError> {
        let mut reader = BasicReader::new(source);
        let header = read_header(&mut reader)?;
        if header.version_packed < 0x14020005
            || header.block_sizes.len() != header.num_blocks as usize
        {
            return Ok(None);
        }

        let blocks_start = reader.pos();
        let file_len = reader
            .reader
            .seek(SeekFrom::End(0))
            .map_err(IoError::from)?;
        reader.seek(blocks_start)?;

        let mut nif = NifFile {
            header,
            ..Default::default()
        };
        let v = nif.header.version_packed;
        let uv = nif.header.user_version;
        let bv = nif.header.bs_version;

        for block_idx in 0..nif.header.num_blocks as usize {
            let type_idx = nif
                .header
                .block_type_index
                .get(block_idx)
                .copied()
                .unwrap_or(0) as usize;
            let type_name = nif
                .header
                .block_type_names
                .get(type_idx)
                .cloned()
                .unwrap_or_else(|| "NiUnknown".to_string());
            let size = nif.header.block_sizes[block_idx] as u64;
            let block_start = reader.pos();
            let block_end = block_start.checked_add(size).ok_or_else(|| {
                ReadError::Other(format!("block {block_idx} extends past u64 range"))
            })?;
            if block_end > file_len {
                return Err(ReadError::Other(format!(
                    "block {block_idx} extends past end of file"
                )));
            }

            let is_dependency_block = matches!(
                type_name.as_str(),
                "BSShaderTextureSet"
                    | "TallGrassShaderProperty"
                    | "BSShaderNoLightingProperty"
                    | "BSLightingShaderProperty"
                    | "BSEffectShaderProperty"
            );
            if !is_dependency_block {
                reader.seek(block_end)?;
                continue;
            }

            let block_bytes = reader.read_n_bytes(size as usize)?;
            let mut block_reader = BasicReader::new(Cursor::new(block_bytes.as_slice()));
            block_reader.big_endian = reader.big_endian;
            let mut block = NifBlock::new(block_idx, &type_name);
            read_block_fields(
                &mut block,
                &mut block_reader,
                Some(size as u32),
                schema,
                v,
                uv,
                bv,
                &nif.header.strings,
            )?;
            nif.blocks.push(block);
        }

        Ok(Some(nif.referenced_asset_paths()))
    }

    pub fn read(data: &[u8], schema: &NifSchema) -> Result<NifFile, ReadError> {
        Self::read_with_original_metadata(data, schema, true)
    }

    /// Decode a complete NIF without retaining per-block source bytes or
    /// content hashes used only by lossless raw-block reuse during writing.
    pub fn read_lean(data: &[u8], schema: &NifSchema) -> Result<NifFile, ReadError> {
        Self::read_with_original_metadata(data, schema, false)
    }

    fn read_with_original_metadata(
        data: &[u8],
        schema: &NifSchema,
        retain_original_metadata: bool,
    ) -> Result<NifFile, ReadError> {
        let cursor = Cursor::new(data);
        let mut reader = BasicReader::new(cursor);
        let mut nif = NifFile::default();
        nif.header = read_header(&mut reader)?;

        let v = nif.header.version_packed;
        let uv = nif.header.user_version;
        let bv = nif.header.bs_version;

        if nif.header.num_blocks == 0 {
            return Ok(nif);
        }

        for block_idx in 0..nif.header.num_blocks as usize {
            let type_idx = nif
                .header
                .block_type_index
                .get(block_idx)
                .copied()
                .unwrap_or(0) as usize;
            let type_name = nif
                .header
                .block_type_names
                .get(type_idx)
                .cloned()
                .unwrap_or_else(|| "NiUnknown".to_string());

            let block_start = reader.pos();
            let expected_size = nif.header.block_sizes.get(block_idx).copied();

            let mut block = NifBlock::new(block_idx, &type_name);

            if let Some(size) = expected_size {
                if retain_original_metadata {
                    let block_bytes = reader.read_n_bytes(size as usize)?;
                    let mut block_reader = BasicReader::new(Cursor::new(block_bytes.as_slice()));
                    block_reader.big_endian = reader.big_endian;
                    read_block_fields(
                        &mut block,
                        &mut block_reader,
                        Some(size),
                        schema,
                        v,
                        uv,
                        bv,
                        &nif.header.strings,
                    )?;
                    block.original_bytes = Some(block_bytes);
                    block.original_content_hash = Some(block.content_hash());
                } else {
                    let block_end = block_start.checked_add(size as u64).ok_or_else(|| {
                        ReadError::Other(format!("block {block_idx} extends past u64 range"))
                    })?;
                    if block_end > data.len() as u64 {
                        return Err(IoError::Io(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "failed to fill whole buffer",
                        ))
                        .into());
                    }
                    let mut block_reader = BasicReader::new(Cursor::new(
                        &data[block_start as usize..block_end as usize],
                    ));
                    block_reader.big_endian = reader.big_endian;
                    read_block_fields(
                        &mut block,
                        &mut block_reader,
                        Some(size),
                        schema,
                        v,
                        uv,
                        bv,
                        &nif.header.strings,
                    )?;
                    reader.seek(block_end)?;
                }
            } else {
                read_block_fields(
                    &mut block,
                    &mut reader,
                    None,
                    schema,
                    v,
                    uv,
                    bv,
                    &nif.header.strings,
                )?;
                let block_end = reader.pos();
                if retain_original_metadata
                    && block_start <= block_end
                    && block_end <= data.len() as u64
                {
                    let start = block_start as usize;
                    let end = block_end as usize;
                    block.original_bytes = Some(data[start..end].to_vec());
                    block.original_content_hash = Some(block.content_hash());
                }
            }

            nif.blocks.push(block);
        }

        // Footer — since 5.0.0.1
        if v >= 0x05000001 {
            let num_roots = reader.read_uint().unwrap_or(0) as usize;
            let mut roots = Vec::with_capacity(num_roots);
            for _ in 0..num_roots {
                match reader.read_int() {
                    Ok(r) => roots.push(r),
                    Err(_) => break,
                }
            }
            nif.header.footer_roots = roots;
        }

        nif.raw_block_context = Some(RawBlockContext::from_header(&nif.header));
        Ok(nif)
    }
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::NifSchema;
    use std::io::Cursor;

    #[test]
    fn read_basic_types() {
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(&42u32.to_le_bytes());
        data.extend_from_slice(&(-5i32).to_le_bytes());
        data.extend_from_slice(&1.5f32.to_le_bytes());
        let mut reader = BasicReader::new(Cursor::new(data));
        assert_eq!(reader.read_uint().unwrap(), 42);
        assert_eq!(reader.read_int().unwrap(), -5);
        match reader.read_float().unwrap() {
            NifValue::Float(f) => assert!((f - 1.5).abs() < 1e-6),
            other => panic!("expected Float, got {:?}", other),
        }
    }

    #[test]
    fn dependency_reader_matches_full_reader_across_asset_path_blocks() {
        let mut nif = NifFile::new("fo4");

        let mut lighting = IndexMap::new();
        lighting.insert(
            "Name".to_string(),
            NifValue::String("Materials\\Landscape\\Rock.bgsm".to_string()),
        );
        nif.add_block("BSLightingShaderProperty", Some(lighting));

        let mut texture_set = IndexMap::new();
        texture_set.insert(
            "Textures".to_string(),
            NifValue::Array(vec![
                NifValue::String("Textures\\Landscape\\Rock_d.dds".to_string()),
                NifValue::String("textures/landscape/rock_d.dds".to_string()),
                NifValue::String("Textures\\Landscape\\Rock_n.dds".to_string()),
            ]),
        );
        nif.add_block("BSShaderTextureSet", Some(texture_set));

        let mut tall_grass = IndexMap::new();
        tall_grass.insert(
            "File Name".to_string(),
            NifValue::String("Textures\\Grass\\Tall.dds".to_string()),
        );
        nif.add_block("TallGrassShaderProperty", Some(tall_grass));

        let mut no_lighting = IndexMap::new();
        no_lighting.insert(
            "File Name".to_string(),
            NifValue::String("Textures\\Effects\\NoLight.dds".to_string()),
        );
        nif.add_block("BSShaderNoLightingProperty", Some(no_lighting));

        let mut effect = IndexMap::new();
        effect.insert(
            "Source Texture".to_string(),
            NifValue::String("Textures\\Effects\\Source.dds".to_string()),
        );
        effect.insert(
            "Greyscale Texture".to_string(),
            NifValue::String("Textures\\Effects\\Grey.dds".to_string()),
        );
        effect.insert(
            "Env Map Texture".to_string(),
            NifValue::String("Textures\\Effects\\Env.dds".to_string()),
        );
        effect.insert(
            "Normal Texture".to_string(),
            NifValue::String("Textures\\Effects\\Normal.dds".to_string()),
        );
        effect.insert(
            "Env Mask Texture".to_string(),
            NifValue::String("Textures\\Effects\\Mask.dds".to_string()),
        );
        nif.add_block("BSEffectShaderProperty", Some(effect));

        let mut unrelated = IndexMap::new();
        unrelated.insert(
            "Name".to_string(),
            NifValue::String("unrelated".to_string()),
        );
        nif.add_block("NiNode", Some(unrelated));

        let bytes = nif.to_bytes().expect("serialize fixture");
        let schema = NifSchema::from_generated();
        let full = NifReader::read(&bytes, &schema)
            .expect("full parse")
            .referenced_asset_paths();
        let selective = NifReader::read_referenced_asset_paths(Cursor::new(bytes), &schema)
            .expect("selective parse")
            .expect("sized FO76 fixture");

        assert_eq!(selective, full);
        assert_eq!(selective.materials, vec!["materials/landscape/rock.bgsm"],);
        assert_eq!(
            selective.textures,
            vec![
                "textures/landscape/rock_d.dds",
                "textures/landscape/rock_n.dds",
                "textures/grass/tall.dds",
                "textures/effects/nolight.dds",
                "textures/effects/source.dds",
                "textures/effects/grey.dds",
                "textures/effects/env.dds",
                "textures/effects/normal.dds",
                "textures/effects/mask.dds",
            ]
        );
    }

    #[test]
    fn dependency_reader_rejects_sized_block_past_end_of_file() {
        let mut nif = NifFile::new("fo76");
        nif.add_block("NiNode", None);
        let bytes = nif.to_bytes().expect("serialize fixture");
        let mut header_reader = BasicReader::new(Cursor::new(bytes.as_slice()));
        let header = read_header(&mut header_reader).expect("read fixture header");
        let block_end = header_reader.pos() as usize + header.block_sizes[0] as usize;
        let truncated = bytes[..block_end - 1].to_vec();

        let schema = NifSchema::from_generated();
        assert!(NifReader::read(&truncated, &schema).is_err());
        assert!(NifReader::read_referenced_asset_paths(Cursor::new(truncated), &schema).is_err());
    }

    #[test]
    fn dependency_reader_matches_full_reader_for_invalid_block_type_index() {
        let mut nif = NifFile::new("fo76");
        let mut bytes = nif.to_bytes().expect("serialize fixture");
        let type_name = b"BSFadeNode";
        let name_offset = bytes
            .windows(type_name.len())
            .position(|window| window == type_name)
            .expect("block type name in header");
        let index_offset = name_offset + type_name.len();
        bytes[index_offset..index_offset + 2].copy_from_slice(&u16::MAX.to_le_bytes());

        let mut header_reader = BasicReader::new(Cursor::new(bytes.as_slice()));
        let header = read_header(&mut header_reader).expect("read patched header");
        assert_eq!(header.num_blocks, 1);
        assert_eq!(header.block_type_index, vec![0x7fff]);

        let schema = NifSchema::from_generated();
        let full = NifReader::read(&bytes, &schema)
            .expect("full parser tolerates unknown type index")
            .referenced_asset_paths();
        let selective = NifReader::read_referenced_asset_paths(Cursor::new(bytes), &schema)
            .expect("selective parser tolerates unknown type index")
            .expect("sized FO76 fixture");

        assert_eq!(selective, full);
    }

    #[test]
    fn dependency_reader_falls_back_for_valid_legacy_header_without_block_sizes() {
        let mut bytes = b"Gamebryo File Format, Version 20.0.0.5\n".to_vec();
        bytes.extend_from_slice(&0u32.to_le_bytes()); // FileVersion
        bytes.push(1); // little-endian
        bytes.extend_from_slice(&0u32.to_le_bytes()); // User Version
        bytes.extend_from_slice(&0u32.to_le_bytes()); // Num Blocks
        bytes.extend_from_slice(&0u16.to_le_bytes()); // Num Block Types
        bytes.extend_from_slice(&0u32.to_le_bytes()); // Num Groups

        let schema = NifSchema::from_generated();
        assert!(NifReader::read(&bytes, &schema).is_ok());
        assert!(NifReader::read_referenced_asset_paths(Cursor::new(bytes), &schema)
            .expect("legacy header is readable")
            .is_none());
    }

    #[test]
    fn lean_reader_matches_lossless_decode_without_original_metadata() {
        let mut nif = NifFile::new("fo4");
        nif.blocks[0].set_field(
            "Scale",
            NifValue::FloatNan(crate::model::FLOAT_NAN_TAG | 0x7FC0_0001),
        );
        let bytes = nif.to_bytes().expect("serialize fixture");
        let schema = NifSchema::from_generated();
        let lossless = NifReader::read(&bytes, &schema).expect("lossless parse");
        let lean = NifReader::read_lean(&bytes, &schema).expect("lean parse");

        assert_eq!(format!("{:?}", lean.header), format!("{:?}", lossless.header));
        assert_eq!(lean.blocks.len(), lossless.blocks.len());
        for (lean_block, lossless_block) in lean.blocks.iter().zip(&lossless.blocks) {
            assert_eq!(lean_block.block_id, lossless_block.block_id);
            assert_eq!(lean_block.type_name, lossless_block.type_name);
            assert_eq!(lean_block.fields, lossless_block.fields);
            assert_eq!(lean_block.remainder, lossless_block.remainder);
            assert!(lean_block.original_bytes.is_none());
            assert!(lean_block.original_content_hash.is_none());
            assert!(lossless_block.original_bytes.is_some());
            assert!(lossless_block.original_content_hash.is_some());
        }
        assert_eq!(lean.raw_block_context, lossless.raw_block_context);
        assert_eq!(lean.referenced_asset_paths(), lossless.referenced_asset_paths());
    }

    #[test]
    fn lean_reader_matches_lossless_errors_and_unknown_block_remainder() {
        let mut nif = NifFile::new("fo4");
        let mut bytes = nif.to_bytes().expect("serialize fixture");
        let type_name = b"BSFadeNode";
        let name_offset = bytes
            .windows(type_name.len())
            .position(|window| window == type_name)
            .expect("block type name in header");
        let index_offset = name_offset + type_name.len();
        bytes[index_offset..index_offset + 2].copy_from_slice(&u16::MAX.to_le_bytes());

        let schema = NifSchema::from_generated();
        let lossless = NifReader::read(&bytes, &schema).expect("lossless unknown parse");
        let lean = NifReader::read_lean(&bytes, &schema).expect("lean unknown parse");
        assert_eq!(lean.blocks[0].type_name, "NiUnknown");
        assert_eq!(lean.blocks[0].fields, lossless.blocks[0].fields);
        assert_eq!(lean.blocks[0].remainder, lossless.blocks[0].remainder);
        assert!(!lean.blocks[0].remainder.is_empty());

        let mut header_reader = BasicReader::new(Cursor::new(bytes.as_slice()));
        let header = read_header(&mut header_reader).expect("read patched header");
        let block_end = header_reader.pos() as usize + header.block_sizes[0] as usize;
        let truncated = &bytes[..block_end - 1];
        let lossless_error = NifReader::read(truncated, &schema).unwrap_err();
        let lean_error = NifReader::read_lean(truncated, &schema).unwrap_err();
        assert_eq!(lossless_error.to_string(), lean_error.to_string());
    }

    #[test]
    fn nan_preservation_read_float() {
        let raw: u32 = 0x7FC00001;
        let bytes = raw.to_le_bytes();
        let mut reader = BasicReader::new(Cursor::new(bytes.to_vec()));
        match reader.read_float().unwrap() {
            NifValue::FloatNan(tagged) => {
                assert_eq!(tagged & 0xFFFF_FFFF, raw as u64);
                assert_eq!(
                    tagged & crate::model::FLOAT_NAN_TAG,
                    crate::model::FLOAT_NAN_TAG
                );
            }
            other => panic!("expected FloatNan, got {:?}", other),
        }
    }

    #[test]
    fn sized_string_roundtrip() {
        let s = "hello";
        let mut data = Vec::new();
        data.extend_from_slice(&(s.len() as u32).to_le_bytes());
        data.extend_from_slice(s.as_bytes());
        let mut reader = BasicReader::new(Cursor::new(data));
        assert_eq!(reader.read_sized_string().unwrap(), "hello");
    }

    #[test]
    fn string_fields_are_truthy_for_conditions() {
        assert!(nif_to_eval(&NifValue::String("Name".to_string())).as_bool());
        assert!(!nif_to_eval(&NifValue::String("".to_string())).as_bool());
        assert!(!nif_to_eval(&NifValue::String("\0\0".to_string())).as_bool());
    }

    #[test]
    fn header_string_reads_until_newline() {
        let raw = b"Gamebryo File Format, Version 20.2.0.7\nextra";
        let mut reader = BasicReader::new(Cursor::new(raw.to_vec()));
        let s = reader.read_header_string().unwrap();
        assert_eq!(s, "Gamebryo File Format, Version 20.2.0.7");
        assert_eq!(reader.read_byte().unwrap(), b'e');
    }

    #[test]
    fn parse_header_version_works() {
        let (a, b, c, d) = parse_header_version_string("Gamebryo File Format, Version 20.2.0.7");
        assert_eq!((a, b, c, d), (20, 2, 0, 7));
    }

    #[test]
    fn pack_version_matches() {
        assert_eq!(pack_version((20, 2, 0, 7)), 0x14020007);
    }

    #[test]
    fn bulk_u32_read() {
        let mut data = Vec::new();
        for i in 0..4u32 {
            data.extend_from_slice(&i.to_le_bytes());
        }
        let mut reader = BasicReader::new(Cursor::new(data));
        let vals = reader.read_bulk_u32(4).unwrap();
        assert_eq!(vals.len(), 4);
        for (i, v) in vals.iter().enumerate() {
            match v {
                NifValue::UInt(x) => assert_eq!(*x, i as u64),
                other => panic!("expected UInt, got {:?}", other),
            }
        }
    }

    #[test]
    fn sized_block_read_stops_at_declared_boundary_on_field_miss() {
        let schema = NifSchema::from_generated();
        let mut block = NifBlock::new(0, "NiNode");
        let mut data = Vec::new();
        data.extend_from_slice(&0i32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());

        let mut reader = BasicReader::new(Cursor::new(data.as_slice()));
        read_block_fields(
            &mut block,
            &mut reader,
            Some(data.len() as u32),
            &schema,
            0x14020007,
            12,
            130,
            &["Root".to_string()],
        )
        .expect("sized block read should tolerate a field miss");

        assert_eq!(reader.pos(), data.len() as u64);
        assert!(matches!(
            block.get_field("Name"),
            Some(NifValue::String(name)) if name == "Root"
        ));
        assert_eq!(
            block.get_field("Num Extra Data List").map(NifValue::as_i64),
            Some(0)
        );
    }

    #[test]
    #[ignore]
    fn read_fo4_all_blocks_bisect() {
        let Ok(p) = std::env::var("FO4_TEST_NIF") else {
            eprintln!("SKIP: FO4_TEST_NIF unset");
            return;
        };
        let bytes = match std::fs::read(p) {
            Ok(b) => b,
            Err(_) => return,
        };
        let mut r = BasicReader::new(Cursor::new(&bytes[..]));
        let h = read_header(&mut r).expect("header parse");
        let schema = NifSchema::from_generated();
        let v = h.version_packed;
        let uv = h.user_version;
        let bv = h.bs_version;

        for bi in 0..h.num_blocks as usize {
            let type_idx = h.block_type_index[bi] as usize;
            let type_name = h.block_type_names[type_idx].clone();
            let expected_size = h.block_sizes[bi] as usize;
            let start = r.pos();
            println!(
                "block {} type={} start={} expected_size={}",
                bi, type_name, start, expected_size
            );

            let fields = schema.get_all_fields(&type_name);
            let mut block_fields: IndexMap<String, NifValue> = IndexMap::new();
            for fdef in fields.iter() {
                if !should_read_field(fdef, &block_fields, &type_name, &schema, v, uv, bv) {
                    continue;
                }
                let key = if let Some(sfx) = fdef.suffix {
                    format!("{}:{}", fdef.name, sfx)
                } else {
                    fdef.name.to_string()
                };
                let pos_before = r.pos();
                let consumed_from_start = pos_before - start;
                if consumed_from_start > (expected_size as u64) + 128 {
                    panic!(
                        "runaway read in block {} type={}: consumed {} bytes already (expected {}), field {:?}",
                        bi, type_name, consumed_from_start, expected_size, key
                    );
                }
                let val = read_field_value(
                    fdef,
                    &mut block_fields,
                    &type_name,
                    None,
                    &schema,
                    &mut r,
                    v,
                    uv,
                    bv,
                    &h.strings,
                    0,
                )
                .unwrap_or_else(|e| {
                    panic!(
                        "block {} type={} field {:?} @{}: {:?}",
                        bi, type_name, key, pos_before, e
                    )
                });
                block_fields.insert(key, val);
            }
            let end = r.pos();
            let consumed = end - start;
            if consumed != expected_size as u64 {
                println!(
                    "  MISMATCH: consumed {} vs expected {} (delta {})",
                    consumed,
                    expected_size,
                    (consumed as i64) - (expected_size as i64)
                );
            }
            // Skip to next block boundary per recorded size to recover
            if consumed < expected_size as u64 {
                let rem = expected_size as u64 - consumed;
                let _ = r.read_n_bytes(rem as usize).unwrap();
            } else if consumed > expected_size as u64 {
                panic!(
                    "block {} type={} overshot: consumed {} vs expected {}",
                    bi, type_name, consumed, expected_size
                );
            }
        }
        println!("final pos {}", r.pos());
    }

    #[test]
    #[ignore]
    fn read_fo4_block0_ninode() {
        let Ok(p) = std::env::var("FO4_TEST_NIF") else {
            eprintln!("SKIP: FO4_TEST_NIF unset");
            return;
        };
        let bytes = match std::fs::read(p) {
            Ok(b) => b,
            Err(_) => return,
        };
        let mut r = BasicReader::new(Cursor::new(&bytes[..]));
        let h = read_header(&mut r).expect("header parse");
        let schema = NifSchema::from_generated();

        let v = h.version_packed;
        let uv = h.user_version;
        let bv = h.bs_version;
        let type_name = &h.block_type_names[h.block_type_index[0] as usize];
        let fields = schema.get_all_fields(type_name);
        println!("block0 type={} field_defs={}", type_name, fields.len());

        let mut block_fields: IndexMap<String, NifValue> = IndexMap::new();
        for fdef in fields.iter() {
            if !should_read_field(fdef, &block_fields, type_name, &schema, v, uv, bv) {
                continue;
            }
            let key = if let Some(sfx) = fdef.suffix {
                format!("{}:{}", fdef.name, sfx)
            } else {
                fdef.name.to_string()
            };
            let pos_before = r.pos();
            let val = read_field_value(
                fdef,
                &mut block_fields,
                type_name,
                None,
                &schema,
                &mut r,
                v,
                uv,
                bv,
                &h.strings,
                0,
            )
            .unwrap_or_else(|e| panic!("field {} at pos {}: {:?}", key, pos_before, e));
            println!(
                "  field {:?} @{} -> @{} = {}",
                key,
                pos_before,
                r.pos(),
                match &val {
                    NifValue::String(s) => format!("String({:?})", s),
                    NifValue::Int(i) => format!("Int({})", i),
                    NifValue::UInt(u) => format!("UInt({})", u),
                    NifValue::Ref(r) => format!("Ref({})", r),
                    NifValue::Array(a) => format!("Array(len={})", a.len()),
                    NifValue::Struct(m) => format!("Struct(keys={})", m.len()),
                    other => format!("{:?}", other),
                }
            );
            block_fields.insert(key, val);
            if pos_before > 2000 {
                break;
            }
        }
    }

    #[test]
    #[ignore]
    fn header_only_from_fo4_fixture() {
        let Ok(p) = std::env::var("FO4_TEST_NIF") else {
            eprintln!("SKIP: FO4_TEST_NIF unset");
            return;
        };
        let bytes = match std::fs::read(p) {
            Ok(b) => b,
            Err(_) => return,
        };
        let mut r = BasicReader::new(Cursor::new(bytes));
        let h = read_header(&mut r).expect("header parse");
        println!(
            "version={:x} uv={} bs={} nblocks={}",
            h.version_packed, h.user_version, h.bs_version, h.num_blocks
        );
        println!(
            "block_type_names={}, block_sizes={}, strings={}, groups={}",
            h.block_type_names.len(),
            h.block_sizes.len(),
            h.strings.len(),
            h.groups.len()
        );
        for (i, n) in h.block_type_names.iter().enumerate().take(5) {
            println!("  btn[{}] = {:?}", i, n);
        }
        for (i, s) in h.block_sizes.iter().enumerate().take(5) {
            println!("  bsz[{}] = {}", i, s);
        }
        println!("pos after header = {}", r.pos());
        assert_eq!(h.num_blocks, 45);
    }

    #[test]
    fn minimal_nif_header_roundtrip() {
        // Build the smallest NIF header we can: version-only, num_blocks=0,
        // FO4 version 20.2.0.7 with BS 130.
        let mut data: Vec<u8> = Vec::new();
        // Header string
        let hdr = b"Gamebryo File Format, Version 20.2.0.7";
        data.extend_from_slice(hdr);
        data.push(b'\n');
        // FileVersion = 0x14020007 (LE)
        data.extend_from_slice(&0x14020007u32.to_le_bytes());
        // Endian byte = 1 (LE)
        data.push(1);
        // User Version = 12
        data.extend_from_slice(&12u32.to_le_bytes());
        // Num Blocks = 0
        data.extend_from_slice(&0u32.to_le_bytes());
        // BS header (since version matches + user_version>=3): bs_version=130
        data.extend_from_slice(&130u32.to_le_bytes());
        // Creator export string: len=0
        data.push(0);
        // bs_version < 131 -> process script export string
        data.push(0);
        // export script
        data.push(0);
        // bs_version >= 103 && < 170 -> max filepath
        data.push(0);
        // Block type names (since 5.0.0.1): num_block_types = 0
        data.extend_from_slice(&0u16.to_le_bytes());
        // block sizes (since 20.2.0.5): none (num_blocks=0 already)
        // String table (since 20.1.0.1): num_strings=0, max_string_length=0
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        // Groups (since 5.0.0.6): num_groups=0
        data.extend_from_slice(&0u32.to_le_bytes());
        // Footer: num_roots = 0
        data.extend_from_slice(&0u32.to_le_bytes());

        let schema = NifSchema::from_generated();
        let nif = NifReader::read(&data, &schema).expect("header parse ok");
        assert_eq!(nif.header.version, (20, 2, 0, 7));
        assert_eq!(nif.header.version_packed, 0x14020007);
        assert_eq!(nif.header.user_version, 12);
        assert_eq!(nif.header.bs_version, 130);
        assert_eq!(nif.header.num_blocks, 0);
        assert_eq!(nif.header.footer_roots.len(), 0);
    }
}
