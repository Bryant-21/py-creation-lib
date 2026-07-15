use serde::{Deserialize, Serialize};
use std::fs;

pub const PEX_MAGIC: u32 = 0xFA57_C0DE;
pub(crate) const GAME_FO4: u16 = 2;
pub(crate) const GAME_STARFIELD: u16 = 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexFilePayload {
    pub magic: u32,
    pub major_version: u8,
    pub minor_version: u8,
    pub game_id: u16,
    pub compilation_time: u64,
    pub source_filename: String,
    pub username: String,
    pub machine_name: String,
    pub string_table: Vec<String>,
    pub debug_info: Option<PexDebugInfoPayload>,
    pub user_flags: Vec<PexUserFlagPayload>,
    pub objects: Vec<PexObjectPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexDebugInfoPayload {
    pub modification_time: u64,
    pub functions: Vec<PexDebugFunctionPayload>,
    #[serde(default)]
    pub property_groups: Vec<PexDebugPropertyGroupPayload>,
    #[serde(default)]
    pub struct_orders: Vec<PexDebugStructOrderPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexDebugPropertyGroupPayload {
    pub object_name: String,
    pub group_name: String,
    pub docstring: String,
    pub user_flags: u32,
    pub property_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexDebugStructOrderPayload {
    pub object_name: String,
    pub struct_name: String,
    pub member_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexDebugFunctionPayload {
    pub object_name: String,
    pub state_name: String,
    pub function_name: String,
    pub function_type: u8,
    pub line_numbers: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexUserFlagPayload {
    pub name: String,
    pub index: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexStructPayload {
    pub name: String,
    pub members: Vec<PexStructMemberPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexStructMemberPayload {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub user_flags: u32,
    pub data: PexValuePayload,
    pub is_const: bool,
    pub docstring: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexObjectPayload {
    pub name: String,
    pub parent: String,
    pub docstring: String,
    pub is_const: bool,
    pub auto_state: String,
    #[serde(default)]
    pub structs: Vec<PexStructPayload>,
    pub user_flags: u32,
    pub variables: Vec<PexVariablePayload>,
    #[serde(default)]
    pub guards: Vec<String>,
    pub properties: Vec<PexPropertyPayload>,
    pub states: Vec<PexStatePayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexVariablePayload {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub user_flags: u32,
    pub data: PexValuePayload,
    #[serde(default)]
    pub is_const: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexPropertyPayload {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub docstring: String,
    pub user_flags: u32,
    pub flags: u8,
    pub auto_var: String,
    pub getter: Option<PexFunctionPayload>,
    pub setter: Option<PexFunctionPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexStatePayload {
    pub name: String,
    pub functions: Vec<PexFunctionPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexFunctionPayload {
    pub name: String,
    pub return_type: String,
    pub docstring: String,
    #[serde(default)]
    pub user_flags: u32,
    pub is_native: bool,
    pub is_global: bool,
    pub params: Vec<PexParamPayload>,
    pub locals: Vec<PexLocalPayload>,
    pub instructions: Vec<PexInstructionPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexParamPayload {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexLocalPayload {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexInstructionPayload {
    pub opcode: u8,
    pub args: Vec<PexValuePayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PexValuePayload {
    #[serde(rename = "type")]
    pub value_type: u8,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Copy)]
enum Endian {
    Little,
    Big,
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    endian: Endian,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8], endian: Endian) -> Self {
        Self {
            data,
            pos: 0,
            endian,
        }
    }

    fn read(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| format!("read overflow at offset {}", self.pos))?;
        if end > self.data.len() {
            return Err(format!(
                "Unexpected end of data at offset {}, need {} bytes",
                self.pos, n
            ));
        }
        let chunk = &self.data[self.pos..end];
        self.pos = end;
        Ok(chunk)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.read(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        let b = self.read(2)?;
        Ok(match self.endian {
            Endian::Little => u16::from_le_bytes([b[0], b[1]]),
            Endian::Big => u16::from_be_bytes([b[0], b[1]]),
        })
    }

    #[allow(dead_code)]
    fn i32(&mut self) -> Result<i32, String> {
        let b = self.read(4)?;
        Ok(match self.endian {
            Endian::Little => i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            Endian::Big => i32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        })
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.read(4)?;
        Ok(match self.endian {
            Endian::Little => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            Endian::Big => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        })
    }

    fn u64(&mut self) -> Result<u64, String> {
        let b = self.read(8)?;
        Ok(match self.endian {
            Endian::Little => u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
            Endian::Big => u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
        })
    }

    #[allow(dead_code)]
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn wstring(&mut self) -> Result<String, String> {
        let len = usize::from(self.u16()?);
        let bytes = self.read(len)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

fn detect_endian(data: &[u8]) -> Result<Endian, String> {
    if data.len() < 4 {
        return Err("File too small to be a PEX file".to_string());
    }
    let big = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let little = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if big == PEX_MAGIC {
        Ok(Endian::Big)
    } else if little == PEX_MAGIC {
        Ok(Endian::Little)
    } else {
        Err(format!("Invalid PEX magic: 0x{big:08X}"))
    }
}

fn string_at(strings: &[String], index: u16) -> Result<String, String> {
    strings.get(usize::from(index)).cloned().ok_or_else(|| {
        format!(
            "Invalid string table index {index}; string table has {} entries",
            strings.len()
        )
    })
}

fn value_string_at(strings: &[String], index: u16) -> String {
    strings
        .get(usize::from(index))
        .cloned()
        .unwrap_or_else(|| format!("<invalid:{index}>"))
}

fn read_string_table(reader: &mut Reader<'_>) -> Result<Vec<String>, String> {
    let count = reader.u16()?;
    let mut strings = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        strings.push(reader.wstring()?);
    }
    Ok(strings)
}

pub fn parse_pex_bytes(data: &[u8]) -> Result<PexFilePayload, String> {
    let endian = detect_endian(data)?;
    let mut reader = Reader::new(data, endian);
    let magic = reader.u32()?;
    let major_version = reader.u8()?;
    let minor_version = reader.u8()?;
    let game_id = reader.u16()?;
    let compilation_time = reader.u64()?;
    let source_filename = reader.wstring()?;
    let username = reader.wstring()?;
    let machine_name = reader.wstring()?;
    let string_table = read_string_table(&mut reader)?;
    let debug_info = read_debug_info(&mut reader, &string_table, game_id)?;
    let user_flags = read_user_flags(&mut reader, &string_table)?;
    let object_count = reader.u16()?;
    let mut objects = Vec::with_capacity(usize::from(object_count));
    for _ in 0..object_count {
        objects.push(read_object(&mut reader, &string_table, game_id)?);
    }

    Ok(PexFilePayload {
        magic,
        major_version,
        minor_version,
        game_id,
        compilation_time,
        source_filename,
        username,
        machine_name,
        string_table,
        debug_info,
        user_flags,
        objects,
    })
}

pub fn parse_pex_file(path: &str) -> Result<PexFilePayload, String> {
    let data = fs::read(path).map_err(|e| format!("failed to read PEX file {path}: {e}"))?;
    parse_pex_bytes(&data)
}

fn read_debug_info(
    reader: &mut Reader<'_>,
    strings: &[String],
    game_id: u16,
) -> Result<Option<PexDebugInfoPayload>, String> {
    let has_debug = reader.u8()?;
    if has_debug == 0 {
        return Ok(None);
    }
    let modification_time = reader.u64()?;
    let function_count = reader.u16()?;
    let mut functions = Vec::with_capacity(usize::from(function_count));
    for _ in 0..function_count {
        let object_name = string_at(strings, reader.u16()?)?;
        let state_name = string_at(strings, reader.u16()?)?;
        let function_name = string_at(strings, reader.u16()?)?;
        let function_type = reader.u8()?;
        let line_count = reader.u16()?;
        let mut line_numbers = Vec::with_capacity(usize::from(line_count));
        for _ in 0..line_count {
            line_numbers.push(reader.u16()?);
        }
        functions.push(PexDebugFunctionPayload {
            object_name,
            state_name,
            function_name,
            function_type,
            line_numbers,
        });
    }
    let (property_groups, struct_orders) = if game_id >= GAME_FO4 {
        read_fo4_debug_extensions(reader, strings)?
    } else {
        (Vec::new(), Vec::new())
    };
    Ok(Some(PexDebugInfoPayload {
        modification_time,
        functions,
        property_groups,
        struct_orders,
    }))
}

fn read_fo4_debug_extensions(
    reader: &mut Reader<'_>,
    strings: &[String],
) -> Result<
    (
        Vec<PexDebugPropertyGroupPayload>,
        Vec<PexDebugStructOrderPayload>,
    ),
    String,
> {
    let property_group_count = reader.u16()?;
    let mut property_groups = Vec::with_capacity(usize::from(property_group_count));
    for _ in 0..property_group_count {
        let object_name = string_at(strings, reader.u16()?)?;
        let group_name = string_at(strings, reader.u16()?)?;
        let docstring = string_at(strings, reader.u16()?)?;
        let user_flags = reader.u32()?;
        let property_count = reader.u16()?;
        let mut property_names = Vec::with_capacity(usize::from(property_count));
        for _ in 0..property_count {
            property_names.push(string_at(strings, reader.u16()?)?);
        }
        property_groups.push(PexDebugPropertyGroupPayload {
            object_name,
            group_name,
            docstring,
            user_flags,
            property_names,
        });
    }
    let struct_order_count = reader.u16()?;
    let mut struct_orders = Vec::with_capacity(usize::from(struct_order_count));
    for _ in 0..struct_order_count {
        let object_name = string_at(strings, reader.u16()?)?;
        let struct_name = string_at(strings, reader.u16()?)?;
        let member_count = reader.u16()?;
        let mut member_names = Vec::with_capacity(usize::from(member_count));
        for _ in 0..member_count {
            member_names.push(string_at(strings, reader.u16()?)?);
        }
        struct_orders.push(PexDebugStructOrderPayload {
            object_name,
            struct_name,
            member_names,
        });
    }
    Ok((property_groups, struct_orders))
}

fn read_user_flags(
    reader: &mut Reader<'_>,
    strings: &[String],
) -> Result<Vec<PexUserFlagPayload>, String> {
    let count = reader.u16()?;
    let mut flags = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let name = string_at(strings, reader.u16()?)?;
        let index = reader.u8()?;
        flags.push(PexUserFlagPayload { name, index });
    }
    Ok(flags)
}

fn read_value(reader: &mut Reader<'_>, strings: &[String]) -> Result<PexValuePayload, String> {
    let value_type = reader.u8()?;
    let data = match value_type {
        0 => serde_json::Value::Null,
        1 | 2 => {
            let index = reader.u16()?;
            serde_json::Value::String(value_string_at(strings, index))
        }
        3 => serde_json::json!(reader.i32()?),
        4 => serde_json::json!(reader.f32()?),
        5 => serde_json::json!(reader.u8()? != 0),
        _ => serde_json::Value::Null,
    };
    Ok(PexValuePayload { value_type, data })
}

fn read_object(
    reader: &mut Reader<'_>,
    strings: &[String],
    game_id: u16,
) -> Result<PexObjectPayload, String> {
    let name = string_at(strings, reader.u16()?)?;
    let _size = reader.u32()?;
    let parent = string_at(strings, reader.u16()?)?;
    let docstring = string_at(strings, reader.u16()?)?;
    let is_const = if game_id >= GAME_FO4 {
        reader.u8()? != 0
    } else {
        false
    };
    let user_flags = reader.u32()?;
    let auto_state = string_at(strings, reader.u16()?)?;

    let structs = if game_id >= GAME_FO4 {
        read_struct_definitions(reader, strings)?
    } else {
        Vec::new()
    };

    let variables = read_variables(reader, strings, game_id)?;

    let guards = if game_id >= GAME_STARFIELD {
        let guard_count = reader.u16()?;
        let mut g = Vec::with_capacity(usize::from(guard_count));
        for _ in 0..guard_count {
            g.push(string_at(strings, reader.u16()?)?);
        }
        g
    } else {
        Vec::new()
    };

    let property_count = reader.u16()?;
    let mut properties = Vec::with_capacity(usize::from(property_count));
    for _ in 0..property_count {
        properties.push(read_property(reader, strings)?);
    }

    let state_count = reader.u16()?;
    let mut states = Vec::with_capacity(usize::from(state_count));
    for _ in 0..state_count {
        states.push(read_state(reader, strings)?);
    }

    Ok(PexObjectPayload {
        name,
        parent,
        docstring,
        is_const,
        auto_state,
        structs,
        user_flags,
        variables,
        guards,
        properties,
        states,
    })
}

fn read_struct_definitions(
    reader: &mut Reader<'_>,
    strings: &[String],
) -> Result<Vec<PexStructPayload>, String> {
    let struct_count = reader.u16()?;
    let mut structs = Vec::with_capacity(usize::from(struct_count));
    for _ in 0..struct_count {
        let name = string_at(strings, reader.u16()?)?;
        let member_count = reader.u16()?;
        let mut members = Vec::with_capacity(usize::from(member_count));
        for _ in 0..member_count {
            let member_name = string_at(strings, reader.u16()?)?;
            let member_type = string_at(strings, reader.u16()?)?;
            let user_flags = reader.u32()?;
            let data = read_value(reader, strings)?;
            let is_const = reader.u8()? != 0;
            let docstring = string_at(strings, reader.u16()?)?;
            members.push(PexStructMemberPayload {
                name: member_name,
                ty: member_type,
                user_flags,
                data,
                is_const,
                docstring,
            });
        }
        structs.push(PexStructPayload { name, members });
    }
    Ok(structs)
}

fn read_variables(
    reader: &mut Reader<'_>,
    strings: &[String],
    game_id: u16,
) -> Result<Vec<PexVariablePayload>, String> {
    let count = reader.u16()?;
    let mut variables = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let name = string_at(strings, reader.u16()?)?;
        let ty = string_at(strings, reader.u16()?)?;
        let user_flags = reader.u32()?;
        let data = read_value(reader, strings)?;
        let is_const = if game_id >= GAME_FO4 {
            reader.u8()? != 0
        } else {
            false
        };
        variables.push(PexVariablePayload {
            name,
            ty,
            user_flags,
            data,
            is_const,
        });
    }
    Ok(variables)
}

fn read_property(
    reader: &mut Reader<'_>,
    strings: &[String],
) -> Result<PexPropertyPayload, String> {
    let name = string_at(strings, reader.u16()?)?;
    let ty = string_at(strings, reader.u16()?)?;
    let docstring = string_at(strings, reader.u16()?)?;
    let user_flags = reader.u32()?;
    let flags = reader.u8()?;
    let mut auto_var = String::new();
    if flags & 4 != 0 {
        auto_var = string_at(strings, reader.u16()?)?;
    }
    let mut getter = None;
    if flags & 1 != 0 && flags & 4 == 0 {
        let mut function = read_function(reader, strings)?;
        function.name = format!("get_{name}");
        getter = Some(function);
    }
    let mut setter = None;
    if flags & 2 != 0 && flags & 4 == 0 {
        let mut function = read_function(reader, strings)?;
        function.name = format!("set_{name}");
        setter = Some(function);
    }
    Ok(PexPropertyPayload {
        name,
        ty,
        docstring,
        user_flags,
        flags,
        auto_var,
        getter,
        setter,
    })
}

fn read_state(reader: &mut Reader<'_>, strings: &[String]) -> Result<PexStatePayload, String> {
    let name = string_at(strings, reader.u16()?)?;
    let function_count = reader.u16()?;
    let mut functions = Vec::with_capacity(usize::from(function_count));
    for _ in 0..function_count {
        let function_name = string_at(strings, reader.u16()?)?;
        let mut function = read_function(reader, strings)?;
        function.name = function_name;
        functions.push(function);
    }
    Ok(PexStatePayload { name, functions })
}

fn read_function(
    reader: &mut Reader<'_>,
    strings: &[String],
) -> Result<PexFunctionPayload, String> {
    let return_type = string_at(strings, reader.u16()?)?;
    let docstring = string_at(strings, reader.u16()?)?;
    let user_flags = reader.u32()?;
    let raw_flags = reader.u8()?;
    // Papyrus function-flags byte: bit 0 = global, bit 1 = native (matches the
    // stock compiler / Champollion).
    let is_global = raw_flags & 0x01 != 0;
    let is_native = raw_flags & 0x02 != 0;

    let param_count = reader.u16()?;
    let mut params = Vec::with_capacity(usize::from(param_count));
    for _ in 0..param_count {
        let name = string_at(strings, reader.u16()?)?;
        let ty = string_at(strings, reader.u16()?)?;
        params.push(PexParamPayload { name, ty });
    }

    let local_count = reader.u16()?;
    let mut locals = Vec::with_capacity(usize::from(local_count));
    for _ in 0..local_count {
        let name = string_at(strings, reader.u16()?)?;
        let ty = string_at(strings, reader.u16()?)?;
        locals.push(PexLocalPayload { name, ty });
    }

    let instruction_count = reader.u16()?;
    let mut instructions = Vec::with_capacity(usize::from(instruction_count));
    for _ in 0..instruction_count {
        instructions.push(read_instruction(reader, strings)?);
    }

    Ok(PexFunctionPayload {
        name: String::new(),
        return_type,
        docstring,
        user_flags,
        is_native,
        is_global,
        params,
        locals,
        instructions,
    })
}

pub(crate) fn fixed_arg_count(opcode: u8) -> usize {
    match opcode {
        0x00 => 0,
        0x01 | 0x02 | 0x03 | 0x04 | 0x05 | 0x06 | 0x07 | 0x08 | 0x09 => 3,
        0x0A | 0x0B | 0x0C | 0x0D | 0x0E => 2,
        0x0F | 0x10 | 0x11 | 0x12 | 0x13 => 3,
        0x14 => 1,
        0x15 | 0x16 => 2,
        0x17 => 3,
        0x18 => 2,
        0x19 => 3,
        0x1A => 1,
        0x1B | 0x1C | 0x1D => 3,
        0x1E | 0x1F => 2,
        0x20 | 0x21 => 3,
        0x22 | 0x23 => 4,
        0x24 => 3,
        0x25 => 1,
        0x26 | 0x27 => 3,
        0x28 | 0x29 => 5,
        0x2A | 0x2B => 3,
        0x2C => 1,
        0x2D => 3,
        0x2E => 1,
        0x2F => 6,
        0x30 | 0x31 => 0,
        0x32 => 1,
        _ => 0,
    }
}

pub(crate) fn is_vararg_opcode(opcode: u8) -> bool {
    matches!(opcode, 0x17 | 0x18 | 0x19 | 0x30 | 0x31 | 0x32)
}

fn read_instruction(
    reader: &mut Reader<'_>,
    strings: &[String],
) -> Result<PexInstructionPayload, String> {
    let opcode = reader.u8()?;
    let mut args = Vec::new();
    for _ in 0..fixed_arg_count(opcode) {
        args.push(read_value(reader, strings)?);
    }
    if is_vararg_opcode(opcode) {
        let count_value = read_value(reader, strings)?;
        let count = count_value
            .data
            .as_i64()
            .and_then(|x| usize::try_from(x).ok())
            .unwrap_or(0);
        args.push(count_value);
        for _ in 0..count {
            args.push(read_value(reader, strings)?);
        }
    }
    Ok(PexInstructionPayload { opcode, args })
}

/// Builds the byte buffer for a Skyrim game_id=1 PEX file with one object, one
/// variable, and one state function. Exposed at module level so `pex_writer`
/// tests can call `crate::pex::tests_fixture_object_variable_and_function()`.
#[cfg(test)]
pub(crate) fn tests_fixture_object_variable_and_function() -> Vec<u8> {
    fn pw(buf: &mut Vec<u8>, s: &str) {
        buf.extend_from_slice(&(s.len() as u16).to_le_bytes());
        buf.extend_from_slice(s.as_bytes());
    }
    let strings = ["MyScript", "None", "", "Add", "Int", "a", "b", "result"];
    let mut buf = Vec::new();
    buf.extend_from_slice(&PEX_MAGIC.to_le_bytes());
    buf.push(3);
    buf.push(9);
    buf.extend_from_slice(&1u16.to_le_bytes()); // game_id = 1 (Skyrim)
    buf.extend_from_slice(&1700000000u64.to_le_bytes());
    pw(&mut buf, "test.psc");
    pw(&mut buf, "tester");
    pw(&mut buf, "pc");
    buf.extend_from_slice(&(strings.len() as u16).to_le_bytes());
    for s in &strings {
        pw(&mut buf, s);
    }
    buf.push(0); // no debug
    buf.extend_from_slice(&0u16.to_le_bytes()); // user flags
    buf.extend_from_slice(&1u16.to_le_bytes()); // object count
    buf.extend_from_slice(&0u16.to_le_bytes()); // object name = "MyScript" (index 0)

    let mut body = Vec::new();
    body.extend_from_slice(&2u16.to_le_bytes()); // parent = ""
    body.extend_from_slice(&2u16.to_le_bytes()); // docstring = ""
    body.extend_from_slice(&0u32.to_le_bytes()); // user_flags
    body.extend_from_slice(&2u16.to_le_bytes()); // auto_state = ""
    body.extend_from_slice(&1u16.to_le_bytes()); // variable_count = 1
    body.extend_from_slice(&7u16.to_le_bytes()); // var name = "result"
    body.extend_from_slice(&4u16.to_le_bytes()); // var type = "Int"
    body.extend_from_slice(&0u32.to_le_bytes()); // var user_flags
    body.push(3); // value type = Integer
    body.extend_from_slice(&42i32.to_le_bytes()); // integer = 42
    body.extend_from_slice(&0u16.to_le_bytes()); // property_count = 0
    body.extend_from_slice(&1u16.to_le_bytes()); // state_count = 1
    body.extend_from_slice(&2u16.to_le_bytes()); // state name = ""
    body.extend_from_slice(&1u16.to_le_bytes()); // function_count = 1
    body.extend_from_slice(&3u16.to_le_bytes()); // function name = "Add"
    body.extend_from_slice(&4u16.to_le_bytes()); // return_type = "Int"
    body.extend_from_slice(&2u16.to_le_bytes()); // docstring = ""
    body.extend_from_slice(&0u32.to_le_bytes()); // user_flags (discarded by reader)
    body.push(0); // raw_flags (not native, not global)
    body.extend_from_slice(&2u16.to_le_bytes()); // param_count = 2
    body.extend_from_slice(&5u16.to_le_bytes()); // param[0] name = "a"
    body.extend_from_slice(&4u16.to_le_bytes()); // param[0] type = "Int"
    body.extend_from_slice(&6u16.to_le_bytes()); // param[1] name = "b"
    body.extend_from_slice(&4u16.to_le_bytes()); // param[1] type = "Int"
    body.extend_from_slice(&1u16.to_le_bytes()); // local_count = 1
    body.extend_from_slice(&7u16.to_le_bytes()); // local[0] name = "result"
    body.extend_from_slice(&4u16.to_le_bytes()); // local[0] type = "Int"
    body.extend_from_slice(&2u16.to_le_bytes()); // instruction_count = 2
    body.push(0x01); // iadd
    body.push(1);
    body.extend_from_slice(&7u16.to_le_bytes()); // dest = "result"
    body.push(1);
    body.extend_from_slice(&5u16.to_le_bytes()); // src0 = "a"
    body.push(1);
    body.extend_from_slice(&6u16.to_le_bytes()); // src1 = "b"
    body.push(0x1A); // return
    body.push(1);
    body.extend_from_slice(&7u16.to_le_bytes()); // value = "result"

    buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
    buf.extend_from_slice(&body);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_wstring(buf: &mut Vec<u8>, value: &str) {
        buf.extend_from_slice(&(value.len() as u16).to_le_bytes());
        buf.extend_from_slice(value.as_bytes());
    }

    fn push_header(buf: &mut Vec<u8>, game_id: u16) {
        buf.extend_from_slice(&PEX_MAGIC.to_le_bytes());
        buf.push(3);
        buf.push(9);
        buf.extend_from_slice(&game_id.to_le_bytes());
        buf.extend_from_slice(&1700000000u64.to_le_bytes());
        push_wstring(buf, "test.psc");
        push_wstring(buf, "tester");
        push_wstring(buf, "pc");
    }

    fn push_string_table(buf: &mut Vec<u8>, strings: &[&str]) {
        buf.extend_from_slice(&(strings.len() as u16).to_le_bytes());
        for value in strings {
            push_wstring(buf, value);
        }
    }

    fn minimal_pex() -> Vec<u8> {
        let mut buf = Vec::new();
        push_header(&mut buf, 1);
        push_string_table(&mut buf, &["MyScript", "None", "ObjectReference"]);
        buf.push(0);
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf
    }

    #[test]
    fn parses_minimal_header_and_string_table() {
        let parsed = parse_pex_bytes(&minimal_pex()).unwrap();
        assert_eq!(parsed.magic, PEX_MAGIC);
        assert_eq!(parsed.major_version, 3);
        assert_eq!(parsed.minor_version, 9);
        assert_eq!(parsed.game_id, 1);
        assert_eq!(parsed.source_filename, "test.psc");
        assert_eq!(
            parsed.string_table,
            vec!["MyScript", "None", "ObjectReference"]
        );
        assert!(parsed.objects.is_empty());
    }

    #[test]
    fn rejects_invalid_magic() {
        let mut data = minimal_pex();
        data[0] = 0;
        let err = parse_pex_bytes(&data).unwrap_err();
        assert!(err.contains("Invalid PEX magic"));
    }

    #[test]
    fn rejects_invalid_structural_string_index() {
        let mut data = Vec::new();
        push_header(&mut data, 1);
        push_string_table(&mut data, &["MyScript"]);
        data.push(0);
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&99u16.to_le_bytes());
        data.push(0);

        let err = parse_pex_bytes(&data).unwrap_err();

        assert!(err.contains("Invalid string table index 99"));
    }

    #[test]
    fn rejects_invalid_fo4_debug_extension_string_index() {
        let mut data = Vec::new();
        push_header(&mut data, GAME_FO4);
        push_string_table(&mut data, &["MyScript"]);
        data.push(1);
        data.extend_from_slice(&1700000001u64.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&99u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());

        let err = parse_pex_bytes(&data).unwrap_err();

        assert!(err.contains("Invalid string table index 99"));
    }

    #[test]
    fn parses_object_variable_and_function() {
        let buf = super::tests_fixture_object_variable_and_function();
        let parsed = parse_pex_bytes(&buf).unwrap();
        let object = &parsed.objects[0];
        assert_eq!(object.name, "MyScript");
        assert_eq!(object.variables[0].data.data, serde_json::json!(42));
        let function = &object.states[0].functions[0];
        assert_eq!(function.name, "Add");
        assert_eq!(function.instructions[0].opcode, 0x01);
        assert_eq!(function.instructions[1].opcode, 0x1A);
    }

    #[test]
    fn value_string_refs_preserve_python_invalid_placeholder() {
        let strings = ["MyScript", "None", "", "bad"];
        let mut buf = Vec::new();
        push_header(&mut buf, 1);
        push_string_table(&mut buf, &strings);
        buf.push(0);
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());

        let mut body = Vec::new();
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&3u16.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.push(2);
        body.extend_from_slice(&99u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());

        buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
        buf.extend_from_slice(&body);

        let parsed = parse_pex_bytes(&buf).unwrap();

        assert_eq!(
            parsed.objects[0].variables[0].data.data,
            serde_json::json!("<invalid:99>")
        );
    }

    #[test]
    fn captures_fo4_debug_property_groups() {
        let strings = ["MyScript", "Grp", "doc", "PropA"];
        let mut buf = Vec::new();
        push_header(&mut buf, GAME_FO4);
        push_string_table(&mut buf, &strings);
        buf.push(1); // has debug
        buf.extend_from_slice(&1700000001u64.to_le_bytes()); // modification_time
        buf.extend_from_slice(&0u16.to_le_bytes()); // function_count
        // FO4 debug property groups
        buf.extend_from_slice(&1u16.to_le_bytes()); // property_group_count = 1
        buf.extend_from_slice(&0u16.to_le_bytes()); // object_name = "MyScript"
        buf.extend_from_slice(&1u16.to_le_bytes()); // group_name = "Grp"
        buf.extend_from_slice(&2u16.to_le_bytes()); // docstring = "doc"
        buf.extend_from_slice(&0u32.to_le_bytes()); // user_flags
        buf.extend_from_slice(&1u16.to_le_bytes()); // property_count = 1
        buf.extend_from_slice(&3u16.to_le_bytes()); // property = "PropA"
        buf.extend_from_slice(&0u16.to_le_bytes()); // struct_order_count = 0
        buf.extend_from_slice(&0u16.to_le_bytes()); // user flags (file)
        buf.extend_from_slice(&0u16.to_le_bytes()); // object count
        let parsed = parse_pex_bytes(&buf).unwrap();
        let dbg = parsed.debug_info.unwrap();
        assert_eq!(dbg.property_groups.len(), 1);
        assert_eq!(dbg.property_groups[0].group_name, "Grp");
        assert_eq!(
            dbg.property_groups[0].property_names,
            vec!["PropA".to_string()]
        );
    }

    #[test]
    fn captures_variable_const_flag_fo4() {
        let strings = ["MyScript", "None", "", "myVar", "Int"];
        let mut buf = Vec::new();
        push_header(&mut buf, GAME_FO4);
        push_string_table(&mut buf, &strings);
        buf.push(0); // no debug info
        buf.extend_from_slice(&0u16.to_le_bytes()); // user flags
        buf.extend_from_slice(&1u16.to_le_bytes()); // object count
        buf.extend_from_slice(&0u16.to_le_bytes()); // object name = "MyScript"

        let mut body = Vec::new();
        body.extend_from_slice(&1u16.to_le_bytes()); // parent = "None"
        body.extend_from_slice(&2u16.to_le_bytes()); // docstring = ""
        body.push(0); // is_const
        body.extend_from_slice(&0u32.to_le_bytes()); // user_flags
        body.extend_from_slice(&2u16.to_le_bytes()); // auto_state = ""
        body.extend_from_slice(&0u16.to_le_bytes()); // struct_count = 0
        body.extend_from_slice(&1u16.to_le_bytes()); // variable_count = 1
        body.extend_from_slice(&3u16.to_le_bytes()); // var name = "myVar"
        body.extend_from_slice(&4u16.to_le_bytes()); // var type = "Int"
        body.extend_from_slice(&0u32.to_le_bytes()); // var user_flags
        body.push(0); // value type = None
        body.push(1); // const flag = 1
        body.extend_from_slice(&0u16.to_le_bytes()); // properties
        body.extend_from_slice(&0u16.to_le_bytes()); // states
        buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
        buf.extend_from_slice(&body);

        let parsed = parse_pex_bytes(&buf).unwrap();
        assert!(parsed.objects[0].variables[0].is_const);
    }

    #[test]
    fn captures_fo4_struct_definitions() {
        // MyScript with one struct "Pair" { Int First (uf=0, default Int 7, const=0, doc "") }
        let strings = ["MyScript", "None", "", "Pair", "First", "Int"];
        let mut buf = Vec::new();
        push_header(&mut buf, GAME_FO4);
        push_string_table(&mut buf, &strings);
        buf.push(0); // no debug info
        buf.extend_from_slice(&0u16.to_le_bytes()); // user flags
        buf.extend_from_slice(&1u16.to_le_bytes()); // object count
        buf.extend_from_slice(&0u16.to_le_bytes()); // object name = "MyScript"

        let mut body = Vec::new();
        body.extend_from_slice(&1u16.to_le_bytes()); // parent = "None"
        body.extend_from_slice(&2u16.to_le_bytes()); // docstring = ""
        body.push(0); // is_const
        body.extend_from_slice(&0u32.to_le_bytes()); // user_flags
        body.extend_from_slice(&2u16.to_le_bytes()); // auto_state = ""
        // struct definitions
        body.extend_from_slice(&1u16.to_le_bytes()); // struct_count = 1
        body.extend_from_slice(&3u16.to_le_bytes()); // struct name = "Pair"
        body.extend_from_slice(&1u16.to_le_bytes()); // member_count = 1
        body.extend_from_slice(&4u16.to_le_bytes()); // member name = "First"
        body.extend_from_slice(&5u16.to_le_bytes()); // member type = "Int"
        body.extend_from_slice(&0u32.to_le_bytes()); // member user_flags
        body.push(3); // default value type = Integer
        body.extend_from_slice(&7i32.to_le_bytes()); // default = 7
        body.push(0); // const flag
        body.extend_from_slice(&2u16.to_le_bytes()); // member docstring = ""
        body.extend_from_slice(&0u16.to_le_bytes()); // variables count
        body.extend_from_slice(&0u16.to_le_bytes()); // properties count
        body.extend_from_slice(&0u16.to_le_bytes()); // states count

        buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
        buf.extend_from_slice(&body);

        let parsed = parse_pex_bytes(&buf).unwrap();
        let st = &parsed.objects[0].structs;
        assert_eq!(st.len(), 1);
        assert_eq!(st[0].name, "Pair");
        assert_eq!(st[0].members[0].name, "First");
        assert_eq!(st[0].members[0].ty, "Int");
        assert_eq!(st[0].members[0].data.data, serde_json::json!(7));
    }

    #[test]
    fn serde_json_roundtrip_preserves_object_fixture() {
        let buf = super::tests_fixture_object_variable_and_function();
        let payload = parse_pex_bytes(&buf).unwrap();
        let json = serde_json::to_string(&payload).unwrap();
        let restored: PexFilePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, payload);
    }

    #[test]
    fn parses_starfield_object_after_guard_definitions() {
        let strings = ["MyScript", "None", "", "guard"];
        let mut buf = Vec::new();
        push_header(&mut buf, 4);
        push_string_table(&mut buf, &strings);
        buf.push(0);
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());

        let mut body = Vec::new();
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        body.push(0);
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&3u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());

        buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
        buf.extend_from_slice(&body);

        let parsed = parse_pex_bytes(&buf).unwrap();

        assert_eq!(parsed.game_id, 4);
        assert_eq!(parsed.objects[0].name, "MyScript");
        assert_eq!(parsed.objects[0].guards, vec!["guard".to_string()]);
    }
}
