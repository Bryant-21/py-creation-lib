//! `pex_writer` — inverse of `pex.rs`. Serializes a `PexFilePayload` back to
//! little-endian PEX bytes (PC target; big-endian console output is out of scope).
//!
//! Strings are interned against `payload.string_table` *in its existing order*,
//! so a `parse → write` of an unmodified payload is byte-identical. Codegen is
//! responsible for ordering the table the way PCompiler does.

use crate::pex::{
    GAME_FO4, GAME_STARFIELD, PEX_MAGIC, PexFilePayload, PexFunctionPayload, PexObjectPayload,
    PexPropertyPayload, PexStatePayload, PexStructPayload, PexValuePayload, PexVariablePayload,
};
use std::collections::HashMap;

struct Writer {
    buf: Vec<u8>,
    /// string → index into `payload.string_table`.
    intern: HashMap<String, u16>,
}

impl Writer {
    fn new(string_table: &[String]) -> Self {
        let mut intern = HashMap::with_capacity(string_table.len());
        for (i, s) in string_table.iter().enumerate() {
            // First occurrence wins (matches reader index semantics).
            intern.entry(s.clone()).or_insert(i as u16);
        }
        Self {
            buf: Vec::new(),
            intern,
        }
    }

    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_bits().to_le_bytes());
    }

    fn wstring(&mut self, s: &str) -> Result<(), String> {
        let len = u16::try_from(s.len())
            .map_err(|_| format!("string too long for PEX wstring: {} bytes", s.len()))?;
        self.u16(len);
        self.buf.extend_from_slice(s.as_bytes());
        Ok(())
    }

    /// Emit the u16 string-table index for `s`.
    fn str_ref(&mut self, s: &str) -> Result<(), String> {
        let idx = *self.intern.get(s).ok_or_else(|| {
            format!("string {s:?} not present in string table; writer cannot intern out-of-table strings")
        })?;
        self.u16(idx);
        Ok(())
    }
}

pub fn write_pex_bytes(payload: &PexFilePayload) -> Result<Vec<u8>, String> {
    let mut w = Writer::new(&payload.string_table);
    w.u32(PEX_MAGIC);
    w.u8(payload.major_version);
    w.u8(payload.minor_version);
    w.u16(payload.game_id);
    w.u64(payload.compilation_time);
    w.wstring(&payload.source_filename)?;
    w.wstring(&payload.username)?;
    w.wstring(&payload.machine_name)?;
    write_string_table(&mut w, &payload.string_table)?;
    write_debug_info(&mut w, payload)?;
    write_user_flags(&mut w, payload)?;
    w.u16(u16::try_from(payload.objects.len()).map_err(|_| "too many objects".to_string())?);
    for obj in &payload.objects {
        write_object(&mut w, payload, obj)?;
    }
    Ok(w.buf)
}

fn write_string_table(w: &mut Writer, strings: &[String]) -> Result<(), String> {
    w.u16(u16::try_from(strings.len()).map_err(|_| "string table too large".to_string())?);
    for s in strings {
        w.wstring(s)?;
    }
    Ok(())
}

fn write_user_flags(w: &mut Writer, payload: &PexFilePayload) -> Result<(), String> {
    w.u16(u16::try_from(payload.user_flags.len()).map_err(|_| "too many user flags".to_string())?);
    for flag in &payload.user_flags {
        w.str_ref(&flag.name)?;
        w.u8(flag.index);
    }
    Ok(())
}

/// Inverse of `pex::read_value`. `value_type`: 0 None, 1 Identifier, 2 String,
/// 3 Integer, 4 Float, 5 Bool. Types 1 and 2 both carry a string-table index.
fn write_value(w: &mut Writer, v: &PexValuePayload) -> Result<(), String> {
    w.u8(v.value_type);
    match v.value_type {
        0 => {} // None — no payload
        1 | 2 => {
            let s = v.data.as_str().ok_or_else(|| {
                format!(
                    "value type {} expected string data, got {:?}",
                    v.value_type, v.data
                )
            })?;
            w.str_ref(s)?;
        }
        3 => {
            let n = v
                .data
                .as_i64()
                .ok_or_else(|| format!("Integer value expected i64, got {:?}", v.data))?;
            w.i32(i32::try_from(n).map_err(|_| format!("Integer value {n} out of i32 range"))?);
        }
        4 => {
            let f = v
                .data
                .as_f64()
                .ok_or_else(|| format!("Float value expected f64, got {:?}", v.data))?;
            w.f32(f as f32);
        }
        5 => {
            let b = v
                .data
                .as_bool()
                .ok_or_else(|| format!("Bool value expected bool, got {:?}", v.data))?;
            w.u8(if b { 1 } else { 0 });
        }
        other => return Err(format!("unknown PEX value type {other}")),
    }
    Ok(())
}

use crate::pex::{PexInstructionPayload, fixed_arg_count, is_vararg_opcode};

fn write_instruction(w: &mut Writer, instr: &PexInstructionPayload) -> Result<(), String> {
    w.u8(instr.opcode);
    let fixed = fixed_arg_count(instr.opcode);
    if is_vararg_opcode(instr.opcode) {
        // args layout from the reader: [fixed...][count value][count extra values]
        if instr.args.len() < fixed + 1 {
            return Err(format!(
                "vararg opcode {:#x} missing count operand",
                instr.opcode
            ));
        }
        for v in &instr.args[..fixed] {
            write_value(w, v)?;
        }
        let count_value = &instr.args[fixed];
        write_value(w, count_value)?;
        let count = count_value
            .data
            .as_i64()
            .and_then(|x| usize::try_from(x).ok())
            .ok_or_else(|| {
                format!(
                    "vararg opcode {:#x} count operand is not a non-negative int",
                    instr.opcode
                )
            })?;
        let extra = &instr.args[fixed + 1..];
        if extra.len() != count {
            return Err(format!(
                "vararg opcode {:#x}: count={count} but {} extra args present",
                instr.opcode,
                extra.len()
            ));
        }
        for v in extra {
            write_value(w, v)?;
        }
    } else {
        if instr.args.len() != fixed {
            return Err(format!(
                "opcode {:#x} expects {fixed} args, got {}",
                instr.opcode,
                instr.args.len()
            ));
        }
        for v in &instr.args {
            write_value(w, v)?;
        }
    }
    Ok(())
}

/// Writes everything `read_function` reads — i.e. NOT the function name (the
/// name is written by the state / property caller, matching the reader).
fn write_function_body(w: &mut Writer, f: &PexFunctionPayload) -> Result<(), String> {
    w.str_ref(&f.return_type)?;
    w.str_ref(&f.docstring)?;
    w.u32(f.user_flags);
    // Function-flags byte: bit 0 = global, bit 1 = native (stock-compiler order).
    let mut raw = 0u8;
    if f.is_global {
        raw |= 0x01;
    }
    if f.is_native {
        raw |= 0x02;
    }
    w.u8(raw);
    w.u16(u16::try_from(f.params.len()).map_err(|_| "too many params".to_string())?);
    for p in &f.params {
        w.str_ref(&p.name)?;
        w.str_ref(&p.ty)?;
    }
    w.u16(u16::try_from(f.locals.len()).map_err(|_| "too many locals".to_string())?);
    for l in &f.locals {
        w.str_ref(&l.name)?;
        w.str_ref(&l.ty)?;
    }
    w.u16(u16::try_from(f.instructions.len()).map_err(|_| "too many instructions".to_string())?);
    for instr in &f.instructions {
        write_instruction(w, instr)?;
    }
    Ok(())
}

fn write_struct_definitions(w: &mut Writer, structs: &[PexStructPayload]) -> Result<(), String> {
    w.u16(u16::try_from(structs.len()).map_err(|_| "too many structs".to_string())?);
    for s in structs {
        w.str_ref(&s.name)?;
        w.u16(u16::try_from(s.members.len()).map_err(|_| "too many struct members".to_string())?);
        for m in &s.members {
            w.str_ref(&m.name)?;
            w.str_ref(&m.ty)?;
            w.u32(m.user_flags);
            write_value(w, &m.data)?;
            w.u8(if m.is_const { 1 } else { 0 });
            w.str_ref(&m.docstring)?;
        }
    }
    Ok(())
}

fn write_variables(
    w: &mut Writer,
    vars: &[PexVariablePayload],
    game_id: u16,
) -> Result<(), String> {
    w.u16(u16::try_from(vars.len()).map_err(|_| "too many variables".to_string())?);
    for v in vars {
        w.str_ref(&v.name)?;
        w.str_ref(&v.ty)?;
        w.u32(v.user_flags);
        write_value(w, &v.data)?;
        if game_id >= GAME_FO4 {
            w.u8(if v.is_const { 1 } else { 0 });
        }
    }
    Ok(())
}

fn write_property(w: &mut Writer, p: &PexPropertyPayload) -> Result<(), String> {
    w.str_ref(&p.name)?;
    w.str_ref(&p.ty)?;
    w.str_ref(&p.docstring)?;
    w.u32(p.user_flags);
    w.u8(p.flags);
    if p.flags & 4 != 0 {
        w.str_ref(&p.auto_var)?;
    }
    if p.flags & 1 != 0 && p.flags & 4 == 0 {
        let g = p
            .getter
            .as_ref()
            .ok_or_else(|| format!("property {} flagged readable but no getter", p.name))?;
        write_function_body(w, g)?;
    }
    if p.flags & 2 != 0 && p.flags & 4 == 0 {
        let s = p
            .setter
            .as_ref()
            .ok_or_else(|| format!("property {} flagged writable but no setter", p.name))?;
        write_function_body(w, s)?;
    }
    Ok(())
}

fn write_state(w: &mut Writer, st: &PexStatePayload) -> Result<(), String> {
    w.str_ref(&st.name)?;
    w.u16(u16::try_from(st.functions.len()).map_err(|_| "too many functions".to_string())?);
    for f in &st.functions {
        w.str_ref(&f.name)?; // state functions DO carry their name (matches reader)
        write_function_body(w, f)?;
    }
    Ok(())
}

fn write_object(
    w: &mut Writer,
    payload: &PexFilePayload,
    obj: &PexObjectPayload,
) -> Result<(), String> {
    w.str_ref(&obj.name)?;
    // Serialize the body into a scratch writer that shares the intern map.
    let mut body = Writer {
        buf: Vec::new(),
        intern: w.intern.clone(),
    };
    body.str_ref(&obj.parent)?;
    body.str_ref(&obj.docstring)?;
    if payload.game_id >= GAME_FO4 {
        body.u8(if obj.is_const { 1 } else { 0 });
    }
    body.u32(obj.user_flags);
    body.str_ref(&obj.auto_state)?;
    if payload.game_id >= GAME_FO4 {
        write_struct_definitions(&mut body, &obj.structs)?;
    }
    write_variables(&mut body, &obj.variables, payload.game_id)?;
    if payload.game_id >= GAME_STARFIELD {
        body.u16(u16::try_from(obj.guards.len()).map_err(|_| "too many guards".to_string())?);
        for g in &obj.guards {
            body.str_ref(g)?;
        }
    }
    body.u16(u16::try_from(obj.properties.len()).map_err(|_| "too many properties".to_string())?);
    for p in &obj.properties {
        write_property(&mut body, p)?;
    }
    body.u16(u16::try_from(obj.states.len()).map_err(|_| "too many states".to_string())?);
    for st in &obj.states {
        write_state(&mut body, st)?;
    }
    // FO4+ includes the size field itself (4 bytes) in the size value; Skyrim does not.
    let size_extra: usize = if payload.game_id >= GAME_FO4 { 4 } else { 0 };
    w.u32(u32::try_from(body.buf.len() + size_extra).map_err(|_| "object too large".to_string())?);
    w.buf.extend_from_slice(&body.buf);
    Ok(())
}

/// Zero the five PC-identifying / wall-clock fields (spec §5) so the emitted
/// `.pex` carries no building-machine identity. Applied by `compile_source`
/// before `write_pex_bytes`; the faithful round-trip path never calls
/// this (it must reproduce stock bytes exactly).
pub fn neutralize_identity_fields(payload: &mut PexFilePayload) {
    payload.username.clear();
    payload.machine_name.clear();
    payload.source_filename = basename(&payload.source_filename);
    payload.compilation_time = 0;
    if let Some(dbg) = payload.debug_info.as_mut() {
        dbg.modification_time = 0;
    }
}

/// Last path component, splitting on both separators (PCompiler emits Windows
/// paths; be robust to either).
fn basename(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

fn write_debug_info(w: &mut Writer, payload: &PexFilePayload) -> Result<(), String> {
    let Some(dbg) = &payload.debug_info else {
        w.u8(0);
        return Ok(());
    };
    w.u8(1);
    w.u64(dbg.modification_time);
    w.u16(u16::try_from(dbg.functions.len()).map_err(|_| "too many debug functions".to_string())?);
    for f in &dbg.functions {
        w.str_ref(&f.object_name)?;
        w.str_ref(&f.state_name)?;
        w.str_ref(&f.function_name)?;
        w.u8(f.function_type);
        w.u16(
            u16::try_from(f.line_numbers.len()).map_err(|_| "too many line numbers".to_string())?,
        );
        for ln in &f.line_numbers {
            w.u16(*ln);
        }
    }
    if payload.game_id >= GAME_FO4 {
        w.u16(
            u16::try_from(dbg.property_groups.len())
                .map_err(|_| "too many property groups".to_string())?,
        );
        for g in &dbg.property_groups {
            w.str_ref(&g.object_name)?;
            w.str_ref(&g.group_name)?;
            w.str_ref(&g.docstring)?;
            w.u32(g.user_flags);
            w.u16(
                u16::try_from(g.property_names.len())
                    .map_err(|_| "too many group properties".to_string())?,
            );
            for p in &g.property_names {
                w.str_ref(p)?;
            }
        }
        w.u16(
            u16::try_from(dbg.struct_orders.len())
                .map_err(|_| "too many struct orders".to_string())?,
        );
        for so in &dbg.struct_orders {
            w.str_ref(&so.object_name)?;
            w.str_ref(&so.struct_name)?;
            w.u16(
                u16::try_from(so.member_names.len())
                    .map_err(|_| "too many struct-order members".to_string())?,
            );
            for m in &so.member_names {
                w.str_ref(m)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pex::{GAME_FO4, PEX_MAGIC, parse_pex_bytes};

    #[test]
    fn neutralizes_all_five_identity_fields() {
        // Minimal Skyrim (game_id=1) file WITH debug info so modification_time exists.
        let mut buf = Vec::new();
        buf.extend_from_slice(&PEX_MAGIC.to_le_bytes());
        buf.push(3);
        buf.push(9);
        buf.extend_from_slice(&1u16.to_le_bytes()); // game_id Skyrim
        buf.extend_from_slice(&1_700_000_000u64.to_le_bytes()); // compilation_time
        push_ws(&mut buf, r"C:\dev\alice\Mods\MyScript.psc"); // source_filename
        push_ws(&mut buf, "alice"); // username
        push_ws(&mut buf, "ALICE-PC"); // machine_name
        buf.extend_from_slice(&0u16.to_le_bytes()); // string count
        buf.push(1); // has debug
        buf.extend_from_slice(&1_700_000_005u64.to_le_bytes()); // modification_time
        buf.extend_from_slice(&0u16.to_le_bytes()); // debug function count
        buf.extend_from_slice(&0u16.to_le_bytes()); // user flags
        buf.extend_from_slice(&0u16.to_le_bytes()); // object count

        let mut payload = crate::pex::parse_pex_bytes(&buf).unwrap();
        neutralize_identity_fields(&mut payload);
        assert_eq!(payload.username, "");
        assert_eq!(payload.machine_name, "");
        assert_eq!(payload.source_filename, "MyScript.psc");
        assert_eq!(payload.compilation_time, 0);
        assert_eq!(payload.debug_info.unwrap().modification_time, 0);
    }

    fn push_ws(buf: &mut Vec<u8>, s: &str) {
        buf.extend_from_slice(&(s.len() as u16).to_le_bytes());
        buf.extend_from_slice(s.as_bytes());
    }

    fn build_minimal_bytes() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&PEX_MAGIC.to_le_bytes());
        buf.push(3);
        buf.push(9); // major/minor
        buf.extend_from_slice(&1u16.to_le_bytes()); // game_id Skyrim
        buf.extend_from_slice(&1700000000u64.to_le_bytes());
        push_ws(&mut buf, "test.psc");
        push_ws(&mut buf, "tester");
        push_ws(&mut buf, "pc");
        buf.extend_from_slice(&3u16.to_le_bytes()); // string count
        push_ws(&mut buf, "MyScript");
        push_ws(&mut buf, "None");
        push_ws(&mut buf, "ObjectReference");
        buf.push(0); // no debug
        buf.extend_from_slice(&0u16.to_le_bytes()); // user flags
        buf.extend_from_slice(&0u16.to_le_bytes()); // object count
        buf
    }

    #[test]
    fn round_trips_minimal_header_and_table() {
        let original = build_minimal_bytes();
        let payload = parse_pex_bytes(&original).unwrap();
        let written = write_pex_bytes(&payload).unwrap();
        assert_eq!(
            written, original,
            "minimal file must round-trip byte-identical"
        );
    }

    #[test]
    fn round_trips_user_flags() {
        // Build a file with two user flags: "hidden"=0, "conditional"=1.
        let mut buf = Vec::new();
        buf.extend_from_slice(&PEX_MAGIC.to_le_bytes());
        buf.push(3);
        buf.push(9);
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&1700000000u64.to_le_bytes());
        push_ws(&mut buf, "test.psc");
        push_ws(&mut buf, "tester");
        push_ws(&mut buf, "pc");
        buf.extend_from_slice(&2u16.to_le_bytes());
        push_ws(&mut buf, "hidden");
        push_ws(&mut buf, "conditional");
        buf.push(0); // no debug
        buf.extend_from_slice(&2u16.to_le_bytes()); // user flag count
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.push(0); // "hidden" -> bit 0
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.push(1); // "conditional" -> bit 1
        buf.extend_from_slice(&0u16.to_le_bytes()); // object count

        let payload = crate::pex::parse_pex_bytes(&buf).unwrap();
        assert_eq!(write_pex_bytes(&payload).unwrap(), buf);
    }

    #[test]
    fn round_trips_instruction_stream() {
        use crate::pex::{PexInstructionPayload, PexValuePayload};
        // iadd ::temp0, a, 1   (opcode 0x01, three args: identifier, identifier, integer)
        let instr = PexInstructionPayload {
            opcode: 0x01,
            args: vec![
                PexValuePayload {
                    value_type: 1,
                    data: serde_json::json!("::temp0"),
                },
                PexValuePayload {
                    value_type: 1,
                    data: serde_json::json!("a"),
                },
                PexValuePayload {
                    value_type: 3,
                    data: serde_json::json!(1),
                },
            ],
        };
        let mut w = Writer::new(&["::temp0".into(), "a".into()]);
        write_instruction(&mut w, &instr).unwrap();
        // Re-read via a throwaway reader path: the bytes must parse back identically
        // through pex::parse. Here assert opcode byte first.
        assert_eq!(w.buf[0], 0x01);
        assert_eq!(w.buf.len(), 1 + 3 /*type*/ + 2 + 2 + 4 /*idx,idx,i32*/ + 0);
    }

    #[test]
    fn writes_function_flags_and_signature() {
        use crate::pex::PexFunctionPayload;
        let f = PexFunctionPayload {
            name: "Add".into(),
            return_type: "Int".into(),
            docstring: "".into(),
            user_flags: 5,
            is_native: false,
            is_global: true,
            params: vec![crate::pex::PexParamPayload {
                name: "a".into(),
                ty: "Int".into(),
            }],
            locals: vec![],
            instructions: vec![],
        };
        let mut w = Writer::new(&["Int".into(), "".into(), "a".into()]);
        write_function_body(&mut w, &f).unwrap();
        // return_type idx(0), docstring idx(1), user_flags(5u32), raw_flags(0x01 global)
        assert_eq!(&w.buf[0..2], &0u16.to_le_bytes());
        assert_eq!(&w.buf[2..4], &1u16.to_le_bytes());
        assert_eq!(&w.buf[4..8], &5u32.to_le_bytes());
        assert_eq!(w.buf[8], 0x01);
    }

    #[test]
    fn round_trips_object_with_function() {
        // Reuse the reader's own object fixture by parsing it, then writing it back.
        let original = crate::pex::tests_fixture_object_variable_and_function();
        let payload = crate::pex::parse_pex_bytes(&original).unwrap();
        let written = write_pex_bytes(&payload).unwrap();
        assert_eq!(
            written, original,
            "object+function file must round-trip byte-identical"
        );
    }

    #[test]
    fn round_trips_fo4_debug_info() {
        let original = {
            let mut buf = Vec::new();
            buf.extend_from_slice(&PEX_MAGIC.to_le_bytes());
            buf.push(3);
            buf.push(2);
            buf.extend_from_slice(&GAME_FO4.to_le_bytes());
            buf.extend_from_slice(&1700000000u64.to_le_bytes());
            push_ws(&mut buf, "t.psc");
            push_ws(&mut buf, "u");
            push_ws(&mut buf, "m");
            buf.extend_from_slice(&4u16.to_le_bytes());
            push_ws(&mut buf, "MyScript");
            push_ws(&mut buf, "Grp");
            push_ws(&mut buf, "doc");
            push_ws(&mut buf, "PropA");
            buf.push(1);
            buf.extend_from_slice(&1700000001u64.to_le_bytes());
            buf.extend_from_slice(&0u16.to_le_bytes()); // function_count
            buf.extend_from_slice(&1u16.to_le_bytes()); // property_group_count
            buf.extend_from_slice(&0u16.to_le_bytes()); // object_name
            buf.extend_from_slice(&1u16.to_le_bytes()); // group_name
            buf.extend_from_slice(&2u16.to_le_bytes()); // docstring
            buf.extend_from_slice(&0u32.to_le_bytes()); // user_flags
            buf.extend_from_slice(&1u16.to_le_bytes()); // property_count
            buf.extend_from_slice(&3u16.to_le_bytes()); // PropA
            buf.extend_from_slice(&0u16.to_le_bytes()); // struct_order_count
            buf.extend_from_slice(&0u16.to_le_bytes()); // file user flags
            buf.extend_from_slice(&0u16.to_le_bytes()); // object count
            buf
        };
        let payload = crate::pex::parse_pex_bytes(&original).unwrap();
        assert_eq!(write_pex_bytes(&payload).unwrap(), original);
    }
}
