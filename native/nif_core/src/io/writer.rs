use std::collections::HashMap;
use std::io::{Cursor, Write};

use indexmap::IndexMap;

use crate::expr::{EvalContext, NifExpr, Value};
use crate::model::{NifBlock, NifFile, NifHeader, NifValue};
use crate::schema::{EnumOptionDef, FieldDef, NifSchema};

use super::basic_io::{
    BasicWriter, IoError, nif_val_to_i8, nif_val_to_i16, nif_val_to_i32, nif_val_to_i64,
    nif_val_to_u8, nif_val_to_u16, nif_val_to_u32, nif_val_to_u64,
};

pub const BTO_NUM_PRIMITIVES_OVERRIDE_FIELD: &str = "__BTO Num Primitives";

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("io: {0}")]
    Io(#[from] IoError),
    #[error("io: {0}")]
    Std(#[from] std::io::Error),
    #[error("write: {0}")]
    Other(String),
}

// --- Evaluation context for conditions ---

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

struct BlockCtx<'a> {
    block_fields: &'a dyn FieldLookup,
    version_packed: u32,
    user_version: u32,
    bs_version: u32,
    arg: Value,
}

trait FieldLookup {
    fn get_value(&self, path: &str) -> Option<&NifValue>;
}

impl FieldLookup for IndexMap<String, NifValue> {
    fn get_value(&self, path: &str) -> Option<&NifValue> {
        self.get(path)
    }
}

struct OverlayFields<'a> {
    source: &'a IndexMap<String, NifValue>,
    written: &'a IndexMap<String, NifValue>,
    current: Option<(&'a str, &'a NifValue)>,
}

impl FieldLookup for OverlayFields<'_> {
    fn get_value(&self, path: &str) -> Option<&NifValue> {
        if let Some((key, value)) = self.current
            && key == path
        {
            return Some(value);
        }
        self.written.get(path).or_else(|| self.source.get(path))
    }
}

impl<'a> EvalContext for BlockCtx<'a> {
    fn get_field(&self, path: &str) -> Value {
        if let Some(v) = self.block_fields.get_value(path) {
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
        match self.block_fields.get_value(path) {
            Some(NifValue::Array(v)) => Some(v.len()),
            Some(NifValue::Bytes(b)) => Some(b.len()),
            _ => None,
        }
    }
    fn get_field_len2(&self, path: &str) -> Option<usize> {
        match self.block_fields.get_value(path) {
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

fn pack_version(v: (u8, u8, u8, u8)) -> u32 {
    ((v.0 as u32) << 24) | ((v.1 as u32) << 16) | ((v.2 as u32) << 8) | (v.3 as u32)
}

fn parse_version_string(s: &str) -> (u8, u8, u8, u8) {
    let mut parts = [0u8; 4];
    for (i, p) in s.split('.').take(4).enumerate() {
        parts[i] = p.trim().parse::<u32>().unwrap_or(0).min(255) as u8;
    }
    (parts[0], parts[1], parts[2], parts[3])
}

fn should_write_field(
    fdef: &FieldDef,
    block_fields: &dyn FieldLookup,
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
    block_fields: &dyn FieldLookup,
    version_packed: u32,
    user_version: u32,
    bs_version: u32,
) -> usize {
    if let Some(v) = block_fields.get_value(length_expr) {
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
    block_fields: &dyn FieldLookup,
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

fn calc_field_value(
    fdef: &FieldDef,
    block_fields: &dyn FieldLookup,
    actual_type: &str,
    version_packed: u32,
    user_version: u32,
    bs_version: u32,
) -> Option<NifValue> {
    let calc = fdef.calc?;
    if calc.is_empty() {
        return None;
    }
    if actual_type == "BSSubIndexTriShape"
        && fdef.name == "Num Primitives"
        && let Some(value) = block_fields.get_value(BTO_NUM_PRIMITIVES_OVERRIDE_FIELD)
    {
        return Some(value.clone());
    }
    if actual_type == "BSTriShape" && fdef.name == "Data Size" {
        let vertex_words = block_fields
            .get_value("Vertex Desc")
            .map(|value| (value.as_i64().max(0) as u64) & 0xF)
            .unwrap_or(0);
        let has_vertex_data = vertex_words > 0
            && matches!(
                block_fields.get_value("Vertex Data"),
                Some(NifValue::Array(values)) if !values.is_empty()
            );
        let has_triangles = matches!(
            block_fields.get_value("Triangles"),
            Some(NifValue::Array(values)) if !values.is_empty()
        );
        if !has_vertex_data && !has_triangles {
            return Some(NifValue::UInt(0));
        }
    }
    let ctx = BlockCtx {
        block_fields,
        version_packed,
        user_version,
        bs_version,
        arg: Value::Null,
    };
    NifExpr::cached(calc)
        .ok()
        .map(|expr| eval_to_nif(&expr.evaluate(&ctx)))
}

fn bytearray_data_len(value: &NifValue) -> Option<usize> {
    match value {
        NifValue::Bytes(bytes) => Some(bytes.len()),
        NifValue::Array(values) => Some(values.len()),
        _ => None,
    }
}

fn bytearray_data_size(value: &NifValue) -> Option<usize> {
    match value {
        NifValue::Int(size) if *size >= 0 => Some(*size as usize),
        NifValue::UInt(size) => Some(*size as usize),
        _ => None,
    }
}

fn validate_bytearray_size(
    type_name: &str,
    field_vals: &IndexMap<String, NifValue>,
) -> Result<(), WriteError> {
    if type_name != "ByteArray" {
        return Ok(());
    }

    let Some(data_size) = field_vals.get("Data Size").and_then(bytearray_data_size) else {
        return Ok(());
    };
    let Some(data_len) = field_vals.get("Data").and_then(bytearray_data_len) else {
        return Ok(());
    };

    if data_size != data_len {
        return Err(WriteError::Other(format!(
            "ByteArray Data Size mismatch: Data Size is {}, Data has {} byte(s)",
            data_size, data_len
        )));
    }

    Ok(())
}

// Look up an enum option by name and return its integer value as a NifValue.
// Returns None if the input is already numeric (no resolution needed) or
// is a name we don't recognize (let the basic writer fall through).
fn resolve_enum_value(val: &NifValue, options: &[EnumOptionDef]) -> Option<NifValue> {
    let NifValue::String(name) = val else {
        return None;
    };
    let trimmed = name.trim();
    options
        .iter()
        .find(|opt| opt.name == trimmed)
        .map(|opt| NifValue::UInt(opt.value as u64))
}

// Bitflag fields accept either a single name string or an array of name
// strings — OR the matching bits together. Numeric inputs pass through.
fn resolve_bitflag_value(val: &NifValue, options: &[EnumOptionDef]) -> Option<NifValue> {
    match val {
        NifValue::String(name) => {
            let trimmed = name.trim();
            options
                .iter()
                .find(|opt| opt.name == trimmed)
                .map(|opt| NifValue::UInt(opt.value as u64))
        }
        NifValue::Array(items) => {
            let mut bits: u64 = 0;
            let mut saw_name = false;
            for item in items {
                let NifValue::String(name) = item else {
                    return None;
                };
                let trimmed = name.trim();
                let Some(opt) = options.iter().find(|opt| opt.name == trimmed) else {
                    return None;
                };
                bits |= opt.value as u64;
                saw_name = true;
            }
            saw_name.then(|| NifValue::UInt(bits))
        }
        _ => None,
    }
}

// --- Value writing ---

fn write_value<W: Write>(
    type_name: &str,
    val: &NifValue,
    template: Option<&str>,
    arg: Option<&Value>,
    schema: &NifSchema,
    writer: &mut BasicWriter<W>,
    version_packed: u32,
    user_version: u32,
    bs_version: u32,
    string_index_map: &HashMap<String, i32>,
    depth: u32,
) -> Result<(), WriteError> {
    let mut tn: &str = type_name;
    if tn == "#T#" {
        if let Some(t) = template {
            tn = t;
        }
    }

    if schema.get_basic(tn).is_some()
        || matches!(
            tn,
            "string" | "bool" | "NiFixedString" | "SizedString" | "SizedString16"
        )
    {
        writer.write_basic(tn, val, version_packed, string_index_map)?;
        return Ok(());
    }
    if let Some(e) = schema.get_enum(tn) {
        let resolved = resolve_enum_value(val, e.options);
        writer.write_basic(
            e.storage,
            resolved.as_ref().unwrap_or(val),
            version_packed,
            string_index_map,
        )?;
        return Ok(());
    }
    if let Some(b) = schema.get_bitflag(tn) {
        let resolved = resolve_bitflag_value(val, b.options);
        writer.write_basic(
            b.storage,
            resolved.as_ref().unwrap_or(val),
            version_packed,
            string_index_map,
        )?;
        return Ok(());
    }
    if let Some(b) = schema.get_bitfield(tn) {
        writer.write_basic(b.storage, val, version_packed, string_index_map)?;
        return Ok(());
    }
    if schema.get_struct(tn).is_some() {
        let coerced = match val {
            NifValue::Struct(_) => None,
            _ => struct_fields_from_compact_value(tn, val),
        };
        let empty = IndexMap::new();
        let sub_fields = match val {
            NifValue::Struct(m) => m,
            _ => coerced.as_ref().unwrap_or(&empty),
        };
        return write_struct(
            tn,
            sub_fields,
            template,
            arg,
            schema,
            writer,
            version_packed,
            user_version,
            bs_version,
            string_index_map,
            depth + 1,
        );
    }
    writer.write_uint(val.as_i64() as u32)?;
    Ok(())
}

fn write_basic_array_fast<W: Write>(
    field_type: &str,
    arr: &[NifValue],
    count: usize,
    writer: &mut BasicWriter<W>,
    version_packed: u32,
) -> Result<bool, WriteError> {
    let zero = NifValue::Int(0);
    match field_type {
        "byte" => {
            for i in 0..count {
                writer.write_byte(nif_val_to_u8(arr.get(i).unwrap_or(&zero)))?;
            }
        }
        "sbyte" => {
            for i in 0..count {
                writer.write_sbyte(nif_val_to_i8(arr.get(i).unwrap_or(&zero)))?;
            }
        }
        "ushort" => {
            for i in 0..count {
                writer.write_ushort(nif_val_to_u16(arr.get(i).unwrap_or(&zero)))?;
            }
        }
        "short" => {
            for i in 0..count {
                writer.write_short(nif_val_to_i16(arr.get(i).unwrap_or(&zero)))?;
            }
        }
        "uint" => {
            for i in 0..count {
                writer.write_uint(nif_val_to_u32(arr.get(i).unwrap_or(&zero)))?;
            }
        }
        "int" => {
            for i in 0..count {
                writer.write_int(nif_val_to_i32(arr.get(i).unwrap_or(&zero)))?;
            }
        }
        "ulittle32" => {
            for i in 0..count {
                writer.write_ulittle32(nif_val_to_u32(arr.get(i).unwrap_or(&zero)))?;
            }
        }
        "uint64" => {
            for i in 0..count {
                writer.write_uint64(nif_val_to_u64(arr.get(i).unwrap_or(&zero)))?;
            }
        }
        "int64" => {
            for i in 0..count {
                writer.write_int64(nif_val_to_i64(arr.get(i).unwrap_or(&zero)))?;
            }
        }
        "float" => {
            for i in 0..count {
                writer.write_float(arr.get(i).unwrap_or(&zero))?;
            }
        }
        "hfloat" => {
            for i in 0..count {
                writer.write_hfloat(arr.get(i).unwrap_or(&zero))?;
            }
        }
        "normbyte" => {
            for i in 0..count {
                let value = match arr.get(i).unwrap_or(&zero) {
                    NifValue::Float(f) => *f,
                    value => value.as_i64() as f64,
                };
                writer.write_normbyte(value)?;
            }
        }
        "bool" => {
            for i in 0..count {
                writer.write_bool(arr.get(i).unwrap_or(&zero), version_packed)?;
            }
        }
        "Ref" | "Ptr" => {
            for i in 0..count {
                let value = match arr.get(i).unwrap_or(&zero) {
                    NifValue::Ref(r) => *r,
                    value => value.as_i64() as i32,
                };
                writer.write_ref(value)?;
            }
        }
        "BlockTypeIndex" => {
            for i in 0..count {
                writer.write_block_type_index(nif_val_to_i16(arr.get(i).unwrap_or(&zero)))?;
            }
        }
        "FileVersion" => {
            for i in 0..count {
                writer.write_file_version(nif_val_to_u32(arr.get(i).unwrap_or(&zero)))?;
            }
        }
        "StringOffset" => {
            for i in 0..count {
                writer.write_string_offset(nif_val_to_u32(arr.get(i).unwrap_or(&zero)))?;
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn triangle_component(value: &NifValue, field: &str, index: usize) -> u16 {
    match value {
        NifValue::Struct(fields) => fields.get(field).map(nif_val_to_u16).unwrap_or(0),
        NifValue::Array(items) => items.get(index).map(nif_val_to_u16).unwrap_or(0),
        _ => 0,
    }
}

fn write_triangle_array_fast<W: Write>(
    arr: &[NifValue],
    count: usize,
    writer: &mut BasicWriter<W>,
) -> Result<(), WriteError> {
    let zero = NifValue::Int(0);
    for i in 0..count {
        let triangle = arr.get(i).unwrap_or(&zero);
        writer.write_ushort(triangle_component(triangle, "v1", 0))?;
        writer.write_ushort(triangle_component(triangle, "v2", 1))?;
        writer.write_ushort(triangle_component(triangle, "v3", 2))?;
    }
    Ok(())
}

fn struct_field<'a>(value: &'a NifValue, name: &str) -> Option<&'a NifValue> {
    match value {
        NifValue::Struct(fields) => fields.get(name),
        _ => None,
    }
}

fn write_float_component<W: Write>(
    value: Option<&NifValue>,
    writer: &mut BasicWriter<W>,
) -> Result<(), WriteError> {
    static ZERO: NifValue = NifValue::Float(0.0);
    writer.write_float(value.unwrap_or(&ZERO))?;
    Ok(())
}

fn write_hfloat_component<W: Write>(
    value: Option<&NifValue>,
    writer: &mut BasicWriter<W>,
) -> Result<(), WriteError> {
    static ZERO: NifValue = NifValue::Float(0.0);
    writer.write_hfloat(value.unwrap_or(&ZERO))?;
    Ok(())
}

fn write_float_vec3<W: Write>(
    value: Option<&NifValue>,
    writer: &mut BasicWriter<W>,
) -> Result<(), WriteError> {
    match value {
        Some(NifValue::Vec3(vector)) => {
            for item in vector {
                writer.write_float(&NifValue::Float(*item as f64))?;
            }
        }
        Some(NifValue::Struct(fields)) => {
            write_float_component(fields.get("x"), writer)?;
            write_float_component(fields.get("y"), writer)?;
            write_float_component(fields.get("z"), writer)?;
        }
        _ => {
            for _ in 0..3 {
                writer.write_float(&NifValue::Float(0.0))?;
            }
        }
    }
    Ok(())
}

fn write_hfloat_vec3<W: Write>(
    value: Option<&NifValue>,
    writer: &mut BasicWriter<W>,
) -> Result<(), WriteError> {
    match value {
        Some(NifValue::Vec3(vector)) => {
            for item in vector {
                writer.write_hfloat(&NifValue::Float(*item as f64))?;
            }
        }
        Some(NifValue::Struct(fields)) => {
            write_hfloat_component(fields.get("x"), writer)?;
            write_hfloat_component(fields.get("y"), writer)?;
            write_hfloat_component(fields.get("z"), writer)?;
        }
        _ => {
            for _ in 0..3 {
                writer.write_hfloat(&NifValue::Float(0.0))?;
            }
        }
    }
    Ok(())
}

fn normbyte_value(value: Option<&NifValue>) -> f64 {
    match value {
        Some(NifValue::Float(f)) => *f,
        Some(other) => other.as_i64() as f64,
        None => 0.0,
    }
}

fn write_normbyte_vec3<W: Write>(
    value: Option<&NifValue>,
    writer: &mut BasicWriter<W>,
) -> Result<(), WriteError> {
    match value {
        Some(NifValue::Vec3(vector)) => {
            for item in vector {
                writer.write_normbyte(*item as f64)?;
            }
        }
        Some(NifValue::Struct(fields)) => {
            writer.write_normbyte(normbyte_value(fields.get("x")))?;
            writer.write_normbyte(normbyte_value(fields.get("y")))?;
            writer.write_normbyte(normbyte_value(fields.get("z")))?;
        }
        _ => {
            for _ in 0..3 {
                writer.write_normbyte(0.0)?;
            }
        }
    }
    Ok(())
}

fn write_half_tex_coord<W: Write>(
    value: Option<&NifValue>,
    writer: &mut BasicWriter<W>,
) -> Result<(), WriteError> {
    match value {
        Some(NifValue::Struct(fields)) => {
            write_hfloat_component(fields.get("u"), writer)?;
            write_hfloat_component(fields.get("v"), writer)?;
        }
        _ => {
            writer.write_hfloat(&NifValue::Float(0.0))?;
            writer.write_hfloat(&NifValue::Float(0.0))?;
        }
    }
    Ok(())
}

fn byte_color_component(value: Option<&NifValue>) -> u8 {
    value.map(nif_val_to_u8).unwrap_or(0)
}

fn write_byte_color4<W: Write>(
    value: Option<&NifValue>,
    writer: &mut BasicWriter<W>,
) -> Result<(), WriteError> {
    match value {
        Some(NifValue::Color4(color)) => {
            for item in color {
                writer.write_byte((item.clamp(0.0, 1.0) * 255.0).round() as u8)?;
            }
        }
        Some(NifValue::Struct(fields)) => {
            writer.write_byte(byte_color_component(fields.get("r")))?;
            writer.write_byte(byte_color_component(fields.get("g")))?;
            writer.write_byte(byte_color_component(fields.get("b")))?;
            writer.write_byte(byte_color_component(fields.get("a")))?;
        }
        _ => {
            for _ in 0..4 {
                writer.write_byte(0)?;
            }
        }
    }
    Ok(())
}

fn write_fixed_array<W: Write>(
    value: Option<&NifValue>,
    count: usize,
    writer: &mut BasicWriter<W>,
    mut write_item: impl FnMut(&NifValue, &mut BasicWriter<W>) -> Result<(), IoError>,
) -> Result<(), WriteError> {
    let zero = NifValue::Int(0);
    let arr: &[NifValue] = match value {
        Some(NifValue::Array(items)) => items.as_slice(),
        _ => &[],
    };
    for i in 0..count {
        write_item(arr.get(i).unwrap_or(&zero), writer)?;
    }
    Ok(())
}

fn write_bs_vertex_data<W: Write>(
    type_name: &str,
    value: &NifValue,
    attributes: u64,
    writer: &mut BasicWriter<W>,
) -> Result<(), WriteError> {
    if type_name == "BSVertexDataSSE" {
        if attributes & 0x1 != 0 {
            write_float_vec3(struct_field(value, "Vertex"), writer)?;
        }
        if attributes & 0x11 == 0x11 {
            write_float_component(struct_field(value, "Bitangent X"), writer)?;
        }
        if attributes & 0x11 == 0x1 {
            writer.write_uint(nif_val_to_u32(
                struct_field(value, "Unused W").unwrap_or(&NifValue::Int(0)),
            ))?;
        }
    } else if attributes & 0x401 == 0x401 {
        write_float_vec3(struct_field(value, "Vertex"), writer)?;
        if attributes & 0x411 == 0x411 {
            write_float_component(struct_field(value, "Bitangent X"), writer)?;
        }
        if attributes & 0x411 == 0x401 {
            writer.write_uint(nif_val_to_u32(
                struct_field(value, "Unused W").unwrap_or(&NifValue::Int(0)),
            ))?;
        }
    } else if attributes & 0x401 == 0x1 {
        write_hfloat_vec3(struct_field(value, "Vertex"), writer)?;
        if attributes & 0x411 == 0x11 {
            write_hfloat_component(struct_field(value, "Bitangent X"), writer)?;
        }
        if attributes & 0x411 == 0x1 {
            writer.write_ushort(nif_val_to_u16(
                struct_field(value, "Unused W").unwrap_or(&NifValue::Int(0)),
            ))?;
        }
    }

    if attributes & 0x2 != 0 {
        write_half_tex_coord(struct_field(value, "UV"), writer)?;
    }
    if attributes & 0x8 != 0 {
        write_normbyte_vec3(struct_field(value, "Normal"), writer)?;
        writer.write_normbyte(normbyte_value(struct_field(value, "Bitangent Y")))?;
    }
    if attributes & 0x18 == 0x18 {
        write_normbyte_vec3(struct_field(value, "Tangent"), writer)?;
        writer.write_normbyte(normbyte_value(struct_field(value, "Bitangent Z")))?;
    }
    if attributes & 0x20 != 0 {
        write_byte_color4(struct_field(value, "Vertex Colors"), writer)?;
    }
    if attributes & 0x40 != 0 {
        write_fixed_array(struct_field(value, "Bone Weights"), 4, writer, |v, w| {
            w.write_hfloat(v)
        })?;
        write_fixed_array(struct_field(value, "Bone Indices"), 4, writer, |v, w| {
            w.write_byte(nif_val_to_u8(v))
        })?;
    }
    if attributes & 0x100 != 0 {
        write_float_component(struct_field(value, "Eye Data"), writer)?;
    }

    Ok(())
}

fn write_bs_vertex_data_array_fast<W: Write>(
    field_type: &str,
    arr: &[NifValue],
    count: usize,
    attributes: u64,
    writer: &mut BasicWriter<W>,
) -> Result<(), WriteError> {
    let empty = NifValue::Struct(IndexMap::new());
    for i in 0..count {
        write_bs_vertex_data(field_type, arr.get(i).unwrap_or(&empty), attributes, writer)?;
    }
    Ok(())
}

fn struct_fields_from_compact_value(
    type_name: &str,
    value: &NifValue,
) -> Option<IndexMap<String, NifValue>> {
    match (type_name, value) {
        ("Vector3" | "HalfVector3" | "ByteVector3", NifValue::Vec3(vector)) => {
            Some(vec3_fields(*vector))
        }
        ("Color3", NifValue::Color3(color)) => Some(color3_fields(*color)),
        ("Color4", NifValue::Color4(color)) => Some(color4_fields(*color)),
        ("ByteColor4", NifValue::Color4(color)) => Some(byte_color4_fields(*color, false)),
        ("ByteColor4BGRA", NifValue::Color4(color)) => Some(byte_color4_fields(*color, true)),
        ("Vector4", NifValue::Vec4(vector)) => Some(vec4_fields(*vector)),
        ("Vector4", NifValue::Quaternion(quaternion)) => Some(vec4_fields(*quaternion)),
        ("Quaternion", NifValue::Quaternion(quaternion)) => Some(quaternion_fields(*quaternion)),
        ("Matrix33", NifValue::Matrix33(matrix)) => Some(matrix33_fields(*matrix)),
        ("Matrix44", NifValue::Matrix44(matrix)) => Some(matrix44_fields(*matrix)),
        _ => None,
    }
}

fn vec3_fields(vector: [f32; 3]) -> IndexMap<String, NifValue> {
    IndexMap::from([
        ("x".to_string(), NifValue::Float(vector[0] as f64)),
        ("y".to_string(), NifValue::Float(vector[1] as f64)),
        ("z".to_string(), NifValue::Float(vector[2] as f64)),
    ])
}

fn vec4_fields(vector: [f32; 4]) -> IndexMap<String, NifValue> {
    IndexMap::from([
        ("x".to_string(), NifValue::Float(vector[0] as f64)),
        ("y".to_string(), NifValue::Float(vector[1] as f64)),
        ("z".to_string(), NifValue::Float(vector[2] as f64)),
        ("w".to_string(), NifValue::Float(vector[3] as f64)),
    ])
}

fn quaternion_fields(quaternion: [f32; 4]) -> IndexMap<String, NifValue> {
    IndexMap::from([
        ("w".to_string(), NifValue::Float(quaternion[0] as f64)),
        ("x".to_string(), NifValue::Float(quaternion[1] as f64)),
        ("y".to_string(), NifValue::Float(quaternion[2] as f64)),
        ("z".to_string(), NifValue::Float(quaternion[3] as f64)),
    ])
}

fn color3_fields(color: [f32; 3]) -> IndexMap<String, NifValue> {
    IndexMap::from([
        ("r".to_string(), NifValue::Float(color[0] as f64)),
        ("g".to_string(), NifValue::Float(color[1] as f64)),
        ("b".to_string(), NifValue::Float(color[2] as f64)),
    ])
}

fn color4_fields(color: [f32; 4]) -> IndexMap<String, NifValue> {
    IndexMap::from([
        ("r".to_string(), NifValue::Float(color[0] as f64)),
        ("g".to_string(), NifValue::Float(color[1] as f64)),
        ("b".to_string(), NifValue::Float(color[2] as f64)),
        ("a".to_string(), NifValue::Float(color[3] as f64)),
    ])
}

fn byte_color4_fields(color: [f32; 4], bgra: bool) -> IndexMap<String, NifValue> {
    let r = color_channel_to_byte(color[0]);
    let g = color_channel_to_byte(color[1]);
    let b = color_channel_to_byte(color[2]);
    let a = color_channel_to_byte(color[3]);
    if bgra {
        IndexMap::from([
            ("b".to_string(), NifValue::UInt(b)),
            ("g".to_string(), NifValue::UInt(g)),
            ("r".to_string(), NifValue::UInt(r)),
            ("a".to_string(), NifValue::UInt(a)),
        ])
    } else {
        IndexMap::from([
            ("r".to_string(), NifValue::UInt(r)),
            ("g".to_string(), NifValue::UInt(g)),
            ("b".to_string(), NifValue::UInt(b)),
            ("a".to_string(), NifValue::UInt(a)),
        ])
    }
}

fn color_channel_to_byte(value: f32) -> u64 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u64
}

fn matrix33_fields(matrix: [[f32; 3]; 3]) -> IndexMap<String, NifValue> {
    IndexMap::from([
        ("m11".to_string(), NifValue::Float(matrix[0][0] as f64)),
        ("m21".to_string(), NifValue::Float(matrix[0][1] as f64)),
        ("m31".to_string(), NifValue::Float(matrix[0][2] as f64)),
        ("m12".to_string(), NifValue::Float(matrix[1][0] as f64)),
        ("m22".to_string(), NifValue::Float(matrix[1][1] as f64)),
        ("m32".to_string(), NifValue::Float(matrix[1][2] as f64)),
        ("m13".to_string(), NifValue::Float(matrix[2][0] as f64)),
        ("m23".to_string(), NifValue::Float(matrix[2][1] as f64)),
        ("m33".to_string(), NifValue::Float(matrix[2][2] as f64)),
    ])
}

fn matrix44_fields(matrix: [[f32; 4]; 4]) -> IndexMap<String, NifValue> {
    IndexMap::from([
        ("m11".to_string(), NifValue::Float(matrix[0][0] as f64)),
        ("m21".to_string(), NifValue::Float(matrix[0][1] as f64)),
        ("m31".to_string(), NifValue::Float(matrix[0][2] as f64)),
        ("m41".to_string(), NifValue::Float(matrix[0][3] as f64)),
        ("m12".to_string(), NifValue::Float(matrix[1][0] as f64)),
        ("m22".to_string(), NifValue::Float(matrix[1][1] as f64)),
        ("m32".to_string(), NifValue::Float(matrix[1][2] as f64)),
        ("m42".to_string(), NifValue::Float(matrix[1][3] as f64)),
        ("m13".to_string(), NifValue::Float(matrix[2][0] as f64)),
        ("m23".to_string(), NifValue::Float(matrix[2][1] as f64)),
        ("m33".to_string(), NifValue::Float(matrix[2][2] as f64)),
        ("m43".to_string(), NifValue::Float(matrix[2][3] as f64)),
        ("m14".to_string(), NifValue::Float(matrix[3][0] as f64)),
        ("m24".to_string(), NifValue::Float(matrix[3][1] as f64)),
        ("m34".to_string(), NifValue::Float(matrix[3][2] as f64)),
        ("m44".to_string(), NifValue::Float(matrix[3][3] as f64)),
    ])
}

fn write_struct<W: Write>(
    type_name: &str,
    field_vals: &IndexMap<String, NifValue>,
    template: Option<&str>,
    arg: Option<&Value>,
    schema: &NifSchema,
    writer: &mut BasicWriter<W>,
    version_packed: u32,
    user_version: u32,
    bs_version: u32,
    string_index_map: &HashMap<String, i32>,
    depth: u32,
) -> Result<(), WriteError> {
    let s = match schema.get_struct(type_name) {
        Some(s) => s,
        None => return Ok(()),
    };

    validate_bytearray_size(type_name, field_vals)?;

    let mut written: IndexMap<String, NifValue> = IndexMap::new();
    if let Some(arg_val) = arg {
        if !matches!(arg_val, Value::Null) {
            written.insert("ARG".to_string(), eval_to_nif(arg_val));
        }
    }

    for fdef in s.fields.iter() {
        let key = field_key(fdef);

        let eval_fields = OverlayFields {
            source: field_vals,
            written: &written,
            current: None,
        };
        if !should_write_field(
            fdef,
            &eval_fields,
            type_name,
            schema,
            version_packed,
            user_version,
            bs_version,
        ) {
            continue;
        }

        let mut field_type: &str = fdef.type_name;
        let mut field_template: Option<&str> = fdef.template;
        if let Some(tmpl) = template {
            if field_type == "#T#" {
                field_type = tmpl;
            }
            if field_template == Some("#T#") {
                field_template = Some(tmpl);
            }
        }

        let mut val_ref = field_vals
            .get(&key)
            .or_else(|| field_vals.get(fdef.name))
            .cloned()
            .unwrap_or(NifValue::Null);
        if let Some(calc_val) = calc_field_value(
            fdef,
            &eval_fields,
            type_name,
            version_packed,
            user_version,
            bs_version,
        ) {
            val_ref = calc_val;
        }

        let field_context = OverlayFields {
            source: field_vals,
            written: &written,
            current: Some((&key, &val_ref)),
        };
        write_field_value(
            fdef,
            field_type,
            field_template,
            &val_ref,
            &field_context,
            type_name,
            schema,
            writer,
            version_packed,
            user_version,
            bs_version,
            string_index_map,
            depth,
        )?;
        written.insert(key, val_ref);
    }
    Ok(())
}

fn write_field_value<W: Write>(
    fdef: &FieldDef,
    field_type: &str,
    field_template: Option<&str>,
    val: &NifValue,
    block_fields: &dyn FieldLookup,
    _actual_type: &str,
    schema: &NifSchema,
    writer: &mut BasicWriter<W>,
    version_packed: u32,
    user_version: u32,
    bs_version: u32,
    string_index_map: &HashMap<String, i32>,
    depth: u32,
) -> Result<(), WriteError> {
    if fdef.is_binary {
        let count = if let Some(len_expr) = fdef.length {
            Some(resolve_length(
                len_expr,
                block_fields,
                version_packed,
                user_version,
                bs_version,
            ))
        } else {
            None
        };
        let raw: Vec<u8> = match val {
            NifValue::Bytes(b) => b.clone(),
            NifValue::Array(a) => a.iter().map(|x| (x.as_i64() & 0xFF) as u8).collect(),
            _ => Vec::new(),
        };
        let final_bytes: Vec<u8> = match count {
            Some(c) => {
                if raw.len() < c {
                    let mut v = raw;
                    v.resize(c, 0u8);
                    v
                } else if raw.len() > c {
                    raw[..c].to_vec()
                } else {
                    raw
                }
            }
            None => raw,
        };
        if !final_bytes.is_empty() {
            writer.write_bytes(&final_bytes)?;
        }
        return Ok(());
    }

    if fdef.recursive && depth > 64 {
        return Ok(());
    }

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

        let arr: &[NifValue] = match val {
            NifValue::Array(a) => a.as_slice(),
            _ => &[],
        };
        if let Some(width_expr) = fdef.width {
            let widths: Option<Vec<usize>> =
                block_fields.get_value(width_expr).and_then(|v| match v {
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

            for i in 0..count {
                let row: &[NifValue] = match arr.get(i) {
                    Some(NifValue::Array(r)) => r.as_slice(),
                    _ => &[],
                };
                let w = match &widths {
                    Some(v) => v.get(i).copied().unwrap_or(0),
                    None => uniform_width,
                };
                for j in 0..w {
                    let zero = NifValue::Int(0);
                    let v = row.get(j).unwrap_or(&zero);
                    write_value(
                        field_type,
                        v,
                        field_template,
                        arg_val.as_ref(),
                        schema,
                        writer,
                        version_packed,
                        user_version,
                        bs_version,
                        string_index_map,
                        depth,
                    )?;
                }
            }
            return Ok(());
        }

        if field_type == "Triangle" && field_template.is_none() {
            write_triangle_array_fast(arr, count, writer)?;
            return Ok(());
        }

        if matches!(field_type, "BSVertexData" | "BSVertexDataSSE") && field_template.is_none() {
            let attributes = arg_val.as_ref().map(Value::as_int).unwrap_or(0).max(0) as u64;
            write_bs_vertex_data_array_fast(field_type, arr, count, attributes, writer)?;
            return Ok(());
        }

        if field_template.is_none()
            && write_basic_array_fast(field_type, arr, count, writer, version_packed)?
        {
            return Ok(());
        }

        let zero = NifValue::Int(0);
        for i in 0..count {
            let v = arr.get(i).unwrap_or(&zero);
            write_value(
                field_type,
                v,
                field_template,
                arg_val.as_ref(),
                schema,
                writer,
                version_packed,
                user_version,
                bs_version,
                string_index_map,
                depth,
            )?;
        }
        return Ok(());
    }

    if fdef.recursive {
        let empty = IndexMap::new();
        let sub_fields = match val {
            NifValue::Struct(m) => m,
            _ => &empty,
        };
        return write_struct(
            field_type,
            sub_fields,
            field_template,
            arg_val.as_ref(),
            schema,
            writer,
            version_packed,
            user_version,
            bs_version,
            string_index_map,
            depth + 1,
        );
    }

    write_value(
        field_type,
        val,
        field_template,
        arg_val.as_ref(),
        schema,
        writer,
        version_packed,
        user_version,
        bs_version,
        string_index_map,
        depth,
    )
}

fn field_key(fdef: &FieldDef) -> String {
    if let Some(sfx) = fdef.suffix {
        format!("{}:{}", fdef.name, sfx)
    } else {
        fdef.name.to_string()
    }
}

// --- String table rebuild ---

const STRING_TYPES: &[&str] = &["string", "NiFixedString"];

fn collect_from_field(
    ftype: &str,
    val: &NifValue,
    template: Option<&str>,
    schema: &NifSchema,
    seen: &mut IndexMap<String, ()>,
) {
    if STRING_TYPES.contains(&ftype) {
        match val {
            NifValue::String(s) => {
                seen.entry(s.clone()).or_insert(());
            }
            NifValue::Array(arr) => {
                for item in arr {
                    if let NifValue::String(s) = item {
                        seen.entry(s.clone()).or_insert(());
                    }
                }
            }
            _ => {}
        }
        return;
    }
    if schema.get_struct(ftype).is_some() {
        match val {
            NifValue::Struct(m) => collect_from_struct(ftype, m, template, schema, seen),
            NifValue::Array(arr) => {
                for item in arr {
                    if let NifValue::Struct(m) = item {
                        collect_from_struct(ftype, m, template, schema, seen);
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_from_struct(
    struct_type: &str,
    data: &IndexMap<String, NifValue>,
    template: Option<&str>,
    schema: &NifSchema,
    seen: &mut IndexMap<String, ()>,
) {
    let s = match schema.get_struct(struct_type) {
        Some(s) => s,
        None => return,
    };
    for fdef in s.fields.iter() {
        let key = field_key(fdef);
        let val = data.get(&key).or_else(|| data.get(fdef.name));
        let val = match val {
            Some(v) => v,
            None => continue,
        };
        let ftype: &str = if fdef.type_name == "#T#" {
            template.unwrap_or(fdef.type_name)
        } else {
            fdef.type_name
        };
        collect_from_field(ftype, val, fdef.template, schema, seen);
    }
}

fn rebuild_string_table(nif: &mut NifFile, schema: &NifSchema) {
    if nif.header.version_packed < 0x14010003 {
        return;
    }
    let mut seen: IndexMap<String, ()> = IndexMap::new();
    for block in nif.blocks.iter() {
        let all_fields = schema.get_all_field_plan(&block.type_name);
        for entry in all_fields.iter() {
            let fdef = entry.fdef;
            let key = entry.key.as_str();
            let val = block
                .fields
                .get(key)
                .or_else(|| block.fields.get(fdef.name));
            let val = match val {
                Some(v) => v,
                None => continue,
            };
            collect_from_field(fdef.type_name, val, fdef.template, schema, &mut seen);
        }
    }
    // Garbage-collect: drop strings that no block field references. Without
    // this, removing a block that referenced a string leaves the string in
    // the table forever, which can inflate `max_string_length` past safe
    // limits for downstream consumers (notably FO4's NIF loader, where an
    // oversize value causes string-buffer reads to corrupt subsequent
    // block parsing).
    //
    // Block fields store string *values* in memory; indices are re-derived
    // at write time via `string_index_map`, so dropping orphan strings is
    // safe — kept strings just renumber to consecutive indices. We preserve
    // the relative order of retained originals for diff stability across
    // saves, then append any newly-introduced strings.
    let seen_set: std::collections::HashSet<&String> = seen.keys().collect();
    let kept: Vec<String> = nif
        .header
        .strings
        .iter()
        .filter(|s| seen_set.contains(*s))
        .cloned()
        .collect();
    let kept_set: std::collections::HashSet<String> = kept.iter().cloned().collect();
    let mut combined: Vec<String> = kept;
    for k in seen.keys() {
        if !kept_set.contains(k) {
            combined.push(k.clone());
        }
    }
    let max_len = combined
        .iter()
        .map(|s| s.as_bytes().len() as u32)
        .max()
        .unwrap_or(0);
    nif.header.strings = combined;
    nif.header.max_string_length = max_len;
}

// --- Header writing ---

fn write_header<W: Write>(writer: &mut BasicWriter<W>, h: &NifHeader) -> Result<(), WriteError> {
    let ver_tuple = h.version;
    let ver_str = format!(
        "Gamebryo File Format, Version {}.{}.{}.{}",
        ver_tuple.0, ver_tuple.1, ver_tuple.2, ver_tuple.3
    );
    writer.write_header_string(&ver_str)?;

    let ver = h.version_packed;

    if ver >= 0x03010001 {
        writer.write_ulittle32(ver)?;
    }

    if ver >= 0x14000003 {
        writer.write_byte(h.endian_type)?;
    }

    if ver >= 0x0A000108 {
        writer.write_uint(h.user_version)?;
    }

    if ver >= 0x03010001 {
        writer.write_uint(h.num_blocks)?;
    }

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
        writer.write_ulittle32(h.bs_version)?;
        writer.write_export_string(h.creator.as_bytes())?;
        if h.bs_version > 130 {
            writer.write_uint(0)?;
        }
        let ei = &h.export_info;
        let mut idx = 0usize;
        if h.bs_version < 131 {
            let s = ei.get(idx).map(String::as_str).unwrap_or("");
            writer.write_export_string(s.as_bytes())?;
            idx += 1;
        }
        let s = ei.get(idx).map(String::as_str).unwrap_or("");
        writer.write_export_string(s.as_bytes())?;
        idx += 1;
        if h.bs_version >= 103 && h.bs_version < 170 {
            let s = ei.get(idx).map(String::as_str).unwrap_or("");
            writer.write_export_string(s.as_bytes())?;
        }
        if h.bs_version >= 170 {
            writer.write_byte(h.sf_export_data.len() as u8)?;
            if !h.sf_export_data.is_empty() {
                writer.write_bytes(&h.sf_export_data)?;
            }
        }
    }

    if ver >= 0x1E000000 {
        writer.write_uint(0)?;
    }

    if ver >= 0x05000001 {
        writer.write_ushort(h.block_type_names.len() as u16)?;
        for name in h.block_type_names.iter() {
            writer.write_sized_string(name)?;
        }
        for idx in h.block_type_index.iter() {
            writer.write_ushort(*idx)?;
        }
    }

    if ver >= 0x14020005 {
        for size in h.block_sizes.iter() {
            writer.write_uint(*size)?;
        }
    }

    if ver >= 0x14010001 {
        writer.write_uint(h.strings.len() as u32)?;
        writer.write_uint(h.max_string_length)?;
        for s in h.strings.iter() {
            writer.write_sized_string(s)?;
        }
    }

    if ver >= 0x05000006 {
        writer.write_uint(h.num_groups)?;
        for g in h.groups.iter() {
            writer.write_uint(*g)?;
        }
    }

    Ok(())
}

// --- Per-block serialization ---

fn serialize_block(
    block: &NifBlock,
    schema: &NifSchema,
    version_packed: u32,
    user_version: u32,
    bs_version: u32,
    big_endian: bool,
    string_index_map: &HashMap<String, i32>,
) -> Result<Vec<u8>, WriteError> {
    let mut buf: Vec<u8> = Vec::new();
    {
        let cursor = Cursor::new(&mut buf);
        let mut w = BasicWriter::new(cursor);
        w.big_endian = big_endian;

        let all_fields = schema.get_all_field_plan(&block.type_name);
        let mut written: IndexMap<String, NifValue> = IndexMap::new();
        let stored_keys: std::collections::HashSet<&str> =
            block.fields.keys().map(|s| s.as_str()).collect();

        for entry in all_fields.iter() {
            let fdef = entry.fdef;
            let key = entry.key.as_str();
            let stored = stored_keys.contains(key) || stored_keys.contains(fdef.name);
            if !stored && fdef.calc.is_none() {
                continue;
            }
            let eval_fields = OverlayFields {
                source: &block.fields,
                written: &written,
                current: None,
            };
            if !should_write_field(
                fdef,
                &eval_fields,
                &block.type_name,
                schema,
                version_packed,
                user_version,
                bs_version,
            ) {
                continue;
            }

            let mut val = block
                .fields
                .get(key)
                .or_else(|| block.fields.get(fdef.name))
                .cloned()
                .unwrap_or(NifValue::Null);
            if let Some(calc_val) = calc_field_value(
                fdef,
                &eval_fields,
                &block.type_name,
                version_packed,
                user_version,
                bs_version,
            ) {
                val = calc_val;
            }

            let field_type: &str = fdef.type_name;
            let field_template: Option<&str> = fdef.template;

            let field_context = OverlayFields {
                source: &block.fields,
                written: &written,
                current: Some((key, &val)),
            };
            write_field_value(
                fdef,
                field_type,
                field_template,
                &val,
                &field_context,
                &block.type_name,
                schema,
                &mut w,
                version_packed,
                user_version,
                bs_version,
                string_index_map,
                0,
            )?;
            written.insert(key.to_string(), val);
        }
    }
    if !block.remainder.is_empty() {
        buf.extend_from_slice(&block.remainder);
    }
    Ok(buf)
}

fn raw_block_bytes_if_unchanged(block: &NifBlock, raw_context_ok: bool) -> Option<Vec<u8>> {
    if !raw_context_ok {
        return None;
    }
    let original_bytes = block.original_bytes.as_ref()?;
    let original_hash = block.original_content_hash?;
    if original_hash == block.content_hash() {
        Some(original_bytes.clone())
    } else {
        None
    }
}

// --- NifWriter ---

pub struct NifWriter;

impl NifWriter {
    /// Preferred name matching `NifReader::read` — serialize `nif` to bytes.
    pub fn write(nif: &mut NifFile, schema: &NifSchema) -> Result<Vec<u8>, WriteError> {
        Self::write_to_bytes(nif, schema)
    }

    pub fn write_to_bytes(nif: &mut NifFile, schema: &NifSchema) -> Result<Vec<u8>, WriteError> {
        rebuild_string_table(nif, schema);
        let raw_context_ok = nif
            .raw_block_context
            .as_ref()
            .map(|ctx| ctx.matches_header(&nif.header))
            .unwrap_or(false);

        let mut string_index_map: HashMap<String, i32> =
            HashMap::with_capacity(nif.header.strings.len());
        for (i, s) in nif.header.strings.iter().enumerate() {
            string_index_map.insert(s.clone(), i as i32);
        }

        let v = nif.header.version_packed;
        let uv = nif.header.user_version;
        let bv = nif.header.bs_version;
        // endian_type: 0 == BE, 1 == LE (matches reader semantics).
        let big_endian = nif.header.endian_type == 0 && v >= 0x14000003;

        let mut block_buffers: Vec<Vec<u8>> = Vec::with_capacity(nif.blocks.len());
        for block in nif.blocks.iter() {
            let buf = match raw_block_bytes_if_unchanged(block, raw_context_ok) {
                Some(bytes) => bytes,
                None => serialize_block(block, schema, v, uv, bv, big_endian, &string_index_map)?,
            };
            block_buffers.push(buf);
        }

        nif.header.block_sizes = block_buffers.iter().map(|b| b.len() as u32).collect();

        let mut out: Vec<u8> = Vec::new();
        {
            let cursor = Cursor::new(&mut out);
            let mut w = BasicWriter::new(cursor);
            w.big_endian = big_endian;
            write_header(&mut w, &nif.header)?;
            for buf in block_buffers.iter() {
                w.writer.write_all(buf)?;
            }
            if v >= 0x05000001 {
                w.write_uint(nif.header.footer_roots.len() as u32)?;
                for r in nif.header.footer_roots.iter() {
                    w.write_int(*r)?;
                }
            }
        }
        Ok(out)
    }
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::NifReader;
    use crate::schema::NifSchema;

    #[test]
    fn raw_block_bytes_are_reused_only_while_content_is_unchanged() {
        let mut block = NifBlock::new(0, "NiNode");
        block
            .fields
            .insert("Name".to_string(), NifValue::String("Root".to_string()));
        block.original_bytes = Some(vec![1, 2, 3, 4]);
        block.original_content_hash = Some(block.content_hash());

        assert_eq!(
            raw_block_bytes_if_unchanged(&block, true),
            Some(vec![1, 2, 3, 4])
        );
        assert_eq!(raw_block_bytes_if_unchanged(&block, false), None);

        block
            .fields
            .insert("Name".to_string(), NifValue::String("Changed".to_string()));
        assert_eq!(raw_block_bytes_if_unchanged(&block, true), None);
    }

    #[test]
    fn minimal_header_roundtrip_v20207() {
        let mut nif = NifFile::default();
        nif.header.header_string = "Gamebryo File Format, Version 20.2.0.7".to_string();
        nif.header.version = (20, 2, 0, 7);
        nif.header.version_packed = 0x14020007;
        nif.header.endian_type = 1;
        nif.header.user_version = 12;
        nif.header.bs_version = 130;
        nif.header.num_blocks = 0;
        nif.header.creator = String::new();
        nif.header.export_info = vec![String::new(), String::new(), String::new()];

        let schema = NifSchema::from_generated();
        let bytes = NifWriter::write_to_bytes(&mut nif, &schema).expect("write");
        let parsed = NifReader::read(&bytes, &schema).expect("read");
        assert_eq!(parsed.header.version_packed, 0x14020007);
        assert_eq!(parsed.header.user_version, 12);
        assert_eq!(parsed.header.bs_version, 130);
        assert_eq!(parsed.header.num_blocks, 0);
    }

    #[test]
    fn write_basic_values_roundtrip() {
        use crate::io::basic_io::BasicReader;
        use std::io::Cursor as Cur;

        let mut buf: Vec<u8> = Vec::new();
        {
            let mut w = BasicWriter::new(Cur::new(&mut buf));
            w.write_uint(12345).unwrap();
            w.write_int(-9876).unwrap();
            w.write_ushort(0xBEEF).unwrap();
            w.write_short(-3).unwrap();
            w.write_float(&NifValue::Float(3.5)).unwrap();
            w.write_sized_string("hi").unwrap();
        }
        let mut r = BasicReader::new(Cur::new(buf));
        assert_eq!(r.read_uint().unwrap(), 12345);
        assert_eq!(r.read_int().unwrap(), -9876);
        assert_eq!(r.read_ushort().unwrap(), 0xBEEF);
        assert_eq!(r.read_short().unwrap(), -3);
        match r.read_float().unwrap() {
            NifValue::Float(f) => assert!((f - 3.5).abs() < 1e-6),
            other => panic!("expected Float, got {:?}", other),
        }
        assert_eq!(r.read_sized_string().unwrap(), "hi");
    }

    #[test]
    fn string_fields_are_truthy_for_conditions() {
        assert!(nif_to_eval(&NifValue::String("Name".to_string())).as_bool());
        assert!(!nif_to_eval(&NifValue::String("".to_string())).as_bool());
        assert!(!nif_to_eval(&NifValue::String("\0\0".to_string())).as_bool());
    }

    #[test]
    fn len2_counts_nested_array_items() {
        let mut fields = IndexMap::new();
        fields.insert(
            "Strips".to_string(),
            NifValue::Array(vec![NifValue::Array(vec![
                NifValue::UInt(0),
                NifValue::UInt(1),
                NifValue::UInt(2),
                NifValue::UInt(3),
                NifValue::UInt(4),
            ])]),
        );
        let ctx = BlockCtx {
            block_fields: &fields,
            version_packed: 0,
            user_version: 0,
            bs_version: 0,
            arg: Value::Null,
        };

        assert_eq!(ctx.get_field_len("Strips"), Some(1));
        assert_eq!(ctx.get_field_len2("Strips"), Some(5));
    }

    #[test]
    fn calc_field_value_can_see_later_source_fields() {
        let fdef = FieldDef {
            name: "Num Triangles",
            type_name: "ushort",
            template: None,
            suffix: None,
            default: None,
            length: None,
            width: None,
            cond: None,
            vercond: None,
            since: None,
            until: None,
            arg: None,
            is_abstract: false,
            is_binary: false,
            calc: Some("#LEN[Triangles]#"),
            only_t: None,
            exclude_t: None,
            recursive: false,
        };
        let mut source = IndexMap::new();
        source.insert(
            "Triangles".to_string(),
            NifValue::Array(vec![triangle(0, 1, 2), triangle(2, 3, 0)]),
        );
        let calc = calc_field_value(&fdef, &source, "NiTriShape", 0, 0, 0).expect("calc");

        assert_eq!(calc.as_i64(), 2);
    }

    #[test]
    fn bto_num_primitives_override_wins_over_schema_calc() {
        let fdef = FieldDef {
            name: "Num Primitives",
            type_name: "uint",
            template: None,
            suffix: None,
            default: None,
            length: None,
            width: None,
            cond: None,
            vercond: None,
            since: None,
            until: None,
            arg: None,
            is_abstract: false,
            is_binary: false,
            calc: Some("#LEN[Triangles]#"),
            only_t: None,
            exclude_t: None,
            recursive: false,
        };
        let mut source = IndexMap::new();
        source.insert(
            "Triangles".to_string(),
            NifValue::Array(vec![triangle(0, 1, 2), triangle(2, 3, 0)]),
        );
        source.insert(
            BTO_NUM_PRIMITIVES_OVERRIDE_FIELD.to_string(),
            NifValue::UInt(4),
        );

        let calc = calc_field_value(&fdef, &source, "BSSubIndexTriShape", 0, 0, 0).expect("calc");

        assert_eq!(calc.as_i64(), 4);
    }

    #[test]
    fn nan_tagged_float_roundtrips_bits() {
        use crate::io::basic_io::BasicReader;
        use crate::model::FLOAT_NAN_TAG;
        use std::io::Cursor as Cur;

        let raw: u32 = 0x7FC00001;
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut w = BasicWriter::new(Cur::new(&mut buf));
            w.write_float(&NifValue::FloatNan(raw as u64 | FLOAT_NAN_TAG))
                .unwrap();
        }
        assert_eq!(buf.len(), 4);
        assert_eq!(u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]), raw);
        let mut r = BasicReader::new(Cur::new(buf));
        match r.read_float().unwrap() {
            NifValue::FloatNan(t) => {
                assert_eq!((t & 0xFFFF_FFFF) as u32, raw);
            }
            other => panic!("expected FloatNan, got {:?}", other),
        }
    }

    #[test]
    fn new_fo4_root_and_calc_bstrishape_roundtrip() {
        let schema = NifSchema::from_generated();
        let mut nif = NifFile::new("fo4");

        let mut fields = IndexMap::new();
        fields.insert("Name".to_string(), NifValue::String("Triangle".to_string()));
        fields.insert("Num Extra Data List".to_string(), NifValue::UInt(0));
        fields.insert("Extra Data List".to_string(), NifValue::Array(Vec::new()));
        fields.insert("Controller".to_string(), NifValue::Ref(-1));
        fields.insert("Flags".to_string(), NifValue::UInt(14));
        fields.insert("Translation".to_string(), NifValue::Vec3([0.0, 0.0, 0.0]));
        fields.insert(
            "Rotation".to_string(),
            NifValue::Matrix33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        );
        fields.insert("Scale".to_string(), NifValue::Float(1.0));
        fields.insert("Collision Object".to_string(), NifValue::Ref(-1));
        fields.insert(
            "Bounding Sphere".to_string(),
            NifValue::Struct(IndexMap::new()),
        );
        fields.insert("Skin".to_string(), NifValue::Ref(-1));
        fields.insert("Shader Property".to_string(), NifValue::Ref(-1));
        fields.insert("Alpha Property".to_string(), NifValue::Ref(-1));
        fields.insert(
            "Vertex Desc".to_string(),
            NifValue::Int(193_514_046_685_700),
        );
        fields.insert("Num Triangles".to_string(), NifValue::UInt(1));
        fields.insert("Num Vertices".to_string(), NifValue::UInt(3));
        fields.insert("Data Size".to_string(), NifValue::UInt(0));
        fields.insert(
            "Vertex Data".to_string(),
            NifValue::Array(vec![
                vertex_data([0.0, 0.0, 0.0], [0.0, 0.0]),
                vertex_data([1.0, 0.0, 0.0], [1.0, 0.0]),
                vertex_data([0.0, 1.0, 0.0], [0.0, 1.0]),
            ]),
        );
        fields.insert(
            "Triangles".to_string(),
            NifValue::Array(vec![triangle(0, 1, 2)]),
        );

        let shape_id = nif.add_block("BSTriShape", Some(fields));
        let root = nif.blocks.get_mut(0).expect("root");
        root.set_field("Num Children", NifValue::UInt(1));
        root.set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(shape_id as i32)]),
        );

        let bytes = NifWriter::write_to_bytes(&mut nif, &schema).expect("write");
        let parsed = NifReader::read(&bytes, &schema).expect("read");

        assert_eq!(parsed.header.block_sizes[0], 80);
        match parsed.blocks[0].fields.get("Children") {
            Some(NifValue::Array(children)) => assert_eq!(children.len(), 1),
            other => panic!("expected root Children array, got {:?}", other),
        }
        let shape = &parsed.blocks[shape_id];
        match shape.fields.get("Name") {
            Some(NifValue::String(name)) => assert_eq!(name, "Triangle"),
            other => panic!("expected shape name, got {:?}", other),
        }
        assert_eq!(
            shape.fields.get("Data Size").map(NifValue::as_usize),
            Some(54)
        );
        match shape.fields.get("Vertex Data") {
            Some(NifValue::Array(vertices)) => {
                assert_eq!(vertices.len(), 3);
                assert_eq!(vertex_x(&vertices[1]), Some(1.0));
            }
            other => panic!("expected vertex data, got {:?}", other),
        }
        match shape.fields.get("Triangles") {
            Some(NifValue::Array(triangles)) => assert_eq!(triangles.len(), 1),
            other => panic!("expected triangles, got {:?}", other),
        }
    }

    #[test]
    fn bstrishape_position_data_mesh_preserves_zero_data_size() {
        let schema = NifSchema::from_generated();
        let mut nif = NifFile::new("fo4");

        let mut position_data = IndexMap::new();
        position_data.insert(
            "Name".to_string(),
            NifValue::String("BSPosData".to_string()),
        );
        position_data.insert("Num Data".to_string(), NifValue::UInt(3));
        position_data.insert(
            "Data".to_string(),
            NifValue::Array(vec![
                NifValue::Float(0.0),
                NifValue::Float(1.0),
                NifValue::Float(2.0),
            ]),
        );
        let position_data_id = nif.add_block("BSPositionData", Some(position_data));

        let mut fields = IndexMap::new();
        fields.insert("Name".to_string(), NifValue::String("emit1:0".to_string()));
        fields.insert("Num Extra Data List".to_string(), NifValue::UInt(1));
        fields.insert(
            "Extra Data List".to_string(),
            NifValue::Array(vec![NifValue::Ref(position_data_id as i32)]),
        );
        fields.insert("Controller".to_string(), NifValue::Ref(-1));
        fields.insert("Flags".to_string(), NifValue::UInt(14));
        fields.insert("Translation".to_string(), NifValue::Vec3([0.0, 0.0, 0.0]));
        fields.insert(
            "Rotation".to_string(),
            NifValue::Matrix33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        );
        fields.insert("Scale".to_string(), NifValue::Float(1.0));
        fields.insert("Collision Object".to_string(), NifValue::Ref(-1));
        fields.insert(
            "Bounding Sphere".to_string(),
            NifValue::Struct(IndexMap::new()),
        );
        fields.insert("Skin".to_string(), NifValue::Ref(-1));
        fields.insert("Shader Property".to_string(), NifValue::Ref(-1));
        fields.insert("Alpha Property".to_string(), NifValue::Ref(-1));
        fields.insert("Vertex Desc".to_string(), NifValue::UInt(0));
        fields.insert("Num Triangles".to_string(), NifValue::UInt(30));
        fields.insert("Num Vertices".to_string(), NifValue::UInt(32));
        fields.insert("Data Size".to_string(), NifValue::UInt(0));
        fields.insert(
            "Vertex Data".to_string(),
            NifValue::Array(vec![NifValue::Struct(IndexMap::new()); 32]),
        );
        let shape_id = nif.add_block("BSTriShape", Some(fields));

        let root = nif.blocks.get_mut(0).expect("root");
        root.set_field("Num Children", NifValue::UInt(1));
        root.set_field(
            "Children",
            NifValue::Array(vec![NifValue::Ref(shape_id as i32)]),
        );

        let bytes = NifWriter::write_to_bytes(&mut nif, &schema).expect("write");
        let parsed = NifReader::read(&bytes, &schema).expect("read");
        let shape = &parsed.blocks[shape_id];

        assert_eq!(
            shape.fields.get("Data Size").map(NifValue::as_usize),
            Some(0)
        );
        assert!(!shape.fields.contains_key("Vertex Data"));
        assert!(!shape.fields.contains_key("Triangles"));
        assert!(matches!(
            shape.fields.get("Extra Data List"),
            Some(NifValue::Array(values)) if matches!(values.as_slice(), [NifValue::Ref(id)] if *id == position_data_id as i32)
        ));
        assert_eq!(parsed.blocks[position_data_id].type_name, "BSPositionData");
    }

    fn vertex_data(vertex: [f32; 3], uv: [f32; 2]) -> NifValue {
        let mut data = IndexMap::new();
        data.insert("Vertex".to_string(), NifValue::Vec3(vertex));
        data.insert("Unused W".to_string(), NifValue::UInt(0));
        data.insert("UV".to_string(), tex_coord(uv));
        data.insert("Normal".to_string(), NifValue::Vec3([0.0, 0.0, 1.0]));
        data.insert("Bitangent Y".to_string(), NifValue::Float(0.0));
        NifValue::Struct(data)
    }

    fn tex_coord(uv: [f32; 2]) -> NifValue {
        let mut data = IndexMap::new();
        data.insert("u".to_string(), NifValue::Float(uv[0] as f64));
        data.insert("v".to_string(), NifValue::Float(uv[1] as f64));
        NifValue::Struct(data)
    }

    fn triangle(v1: i64, v2: i64, v3: i64) -> NifValue {
        let mut data = IndexMap::new();
        data.insert("v1".to_string(), NifValue::Int(v1));
        data.insert("v2".to_string(), NifValue::Int(v2));
        data.insert("v3".to_string(), NifValue::Int(v3));
        NifValue::Struct(data)
    }

    fn vertex_x(value: &NifValue) -> Option<f32> {
        match value {
            NifValue::Struct(fields) => match fields.get("Vertex")? {
                NifValue::Struct(vertex) => match vertex.get("x")? {
                    NifValue::Float(value) => Some(*value as f32),
                    _ => None,
                },
                NifValue::Vec3(vertex) => Some(vertex[0]),
                _ => None,
            },
            _ => None,
        }
    }
}
