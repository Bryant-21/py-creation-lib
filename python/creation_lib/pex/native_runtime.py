"""Compatibility boundary for the native PEX parser."""
from __future__ import annotations

import importlib
import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from creation_lib.pex.opcodes import PexOpcode
from creation_lib.pex.types import (
    PexDebugFunction,
    PexDebugInfo,
    PexDebugPropertyGroup,
    PexDebugStructOrder,
    PexFile,
    PexFunction,
    PexInstruction,
    PexLocal,
    PexObject,
    PexParam,
    PexProperty,
    PexState,
    PexStruct,
    PexStructMember,
    PexUserFlag,
    PexValue,
    PexVariable,
    ValueType,
)

_MISSING_NATIVE_MESSAGE = "papyrus_core native PEX parser is not available"


def load_native_module() -> Any | None:
    try:
        module = importlib.import_module("creation_lib._native.papyrus_core")
    except ImportError:
        return None
    if not callable(getattr(module, "parse_pex_bytes", None)):
        return None
    if not callable(getattr(module, "parse_pex_file", None)):
        return None
    if not callable(getattr(module, "write_pex_bytes", None)):
        return None
    if not callable(getattr(module, "compile_source", None)):
        return None
    return module


def is_available() -> bool:
    return load_native_module() is not None


def parse_pex_bytes_native(data: bytes) -> PexFile:
    native = load_native_module()
    if native is None:
        raise RuntimeError(_MISSING_NATIVE_MESSAGE)

    return _pex_file_from_native_payload(native.parse_pex_bytes(data))


def parse_pex_file_native(path: str | Path) -> PexFile:
    native = load_native_module()
    if native is None:
        raise RuntimeError(_MISSING_NATIVE_MESSAGE)

    pex_path = Path(path)
    return _pex_file_from_native_payload(native.parse_pex_file(str(pex_path)))


def _pex_file_from_native_payload(payload: str | dict[str, Any]) -> PexFile:
    if isinstance(payload, str):
        payload = json.loads(payload)
    return pex_file_from_payload(payload)


def pex_file_from_payload(payload: dict[str, Any]) -> PexFile:
    return PexFile(
        magic=payload["magic"],
        major_version=payload["major_version"],
        minor_version=payload["minor_version"],
        game_id=payload["game_id"],
        compilation_time=payload["compilation_time"],
        source_filename=payload["source_filename"],
        username=payload["username"],
        machine_name=payload["machine_name"],
        string_table=list(payload.get("string_table", [])),
        debug_info=_debug_info_from_payload(payload.get("debug_info")),
        user_flags=[_user_flag_from_payload(flag) for flag in payload.get("user_flags", [])],
        objects=[_object_from_payload(obj) for obj in payload.get("objects", [])],
    )


def _debug_info_from_payload(payload: dict[str, Any] | None) -> PexDebugInfo | None:
    if payload is None:
        return None
    return PexDebugInfo(
        modification_time=payload.get("modification_time", 0),
        functions=[_debug_function_from_payload(fn) for fn in payload.get("functions", [])],
        property_groups=[_debug_property_group_from_payload(g) for g in payload.get("property_groups", [])],
        struct_orders=[_debug_struct_order_from_payload(so) for so in payload.get("struct_orders", [])],
    )


def _debug_property_group_from_payload(payload: dict[str, Any]) -> PexDebugPropertyGroup:
    return PexDebugPropertyGroup(
        object_name=payload.get("object_name", ""),
        group_name=payload.get("group_name", ""),
        docstring=payload.get("docstring", ""),
        user_flags=payload.get("user_flags", 0),
        property_names=list(payload.get("property_names", [])),
    )


def _debug_struct_order_from_payload(payload: dict[str, Any]) -> PexDebugStructOrder:
    return PexDebugStructOrder(
        object_name=payload.get("object_name", ""),
        struct_name=payload.get("struct_name", ""),
        member_names=list(payload.get("member_names", [])),
    )


def _debug_function_from_payload(payload: dict[str, Any]) -> PexDebugFunction:
    return PexDebugFunction(
        object_name=payload.get("object_name", ""),
        state_name=payload.get("state_name", ""),
        function_name=payload.get("function_name", ""),
        function_type=payload.get("function_type", 0),
        line_numbers=list(payload.get("line_numbers", [])),
    )


def _user_flag_from_payload(payload: dict[str, Any]) -> PexUserFlag:
    return PexUserFlag(
        name=payload.get("name", ""),
        index=payload.get("index", 0),
    )


def _struct_member_from_payload(payload: dict[str, Any]) -> PexStructMember:
    return PexStructMember(
        name=payload.get("name", ""),
        type=payload.get("type", ""),
        user_flags=payload.get("user_flags", 0),
        data=_value_from_payload(payload.get("data")),
        is_const=payload.get("is_const", False),
        docstring=payload.get("docstring", ""),
    )


def _struct_from_payload(payload: dict[str, Any]) -> PexStruct:
    return PexStruct(
        name=payload.get("name", ""),
        members=[_struct_member_from_payload(m) for m in payload.get("members", [])],
    )


def _object_from_payload(payload: dict[str, Any]) -> PexObject:
    return PexObject(
        name=payload.get("name", ""),
        parent=payload.get("parent", ""),
        docstring=payload.get("docstring", ""),
        is_const=payload.get("is_const", False),
        auto_state=payload.get("auto_state", ""),
        user_flags=payload.get("user_flags", 0),
        structs=[_struct_from_payload(s) for s in payload.get("structs", [])],
        variables=[_variable_from_payload(var) for var in payload.get("variables", [])],
        guards=list(payload.get("guards", [])),
        properties=[_property_from_payload(prop) for prop in payload.get("properties", [])],
        states=[_state_from_payload(state) for state in payload.get("states", [])],
    )


def _variable_from_payload(payload: dict[str, Any]) -> PexVariable:
    return PexVariable(
        name=payload.get("name", ""),
        type=payload.get("type", ""),
        user_flags=payload.get("user_flags", 0),
        data=_value_from_payload(payload.get("data")),
        is_const=payload.get("is_const", False),
    )


def _property_from_payload(payload: dict[str, Any]) -> PexProperty:
    getter = payload.get("getter")
    setter = payload.get("setter")
    return PexProperty(
        name=payload.get("name", ""),
        type=payload.get("type", ""),
        docstring=payload.get("docstring", ""),
        user_flags=payload.get("user_flags", 0),
        flags=payload.get("flags", 0),
        auto_var=payload.get("auto_var", ""),
        getter=_function_from_payload(getter) if getter is not None else None,
        setter=_function_from_payload(setter) if setter is not None else None,
    )


def _state_from_payload(payload: dict[str, Any]) -> PexState:
    return PexState(
        name=payload.get("name", ""),
        functions=[_function_from_payload(fn) for fn in payload.get("functions", [])],
    )


def _function_from_payload(payload: dict[str, Any]) -> PexFunction:
    return PexFunction(
        name=payload.get("name", ""),
        return_type=payload.get("return_type", ""),
        docstring=payload.get("docstring", ""),
        is_native=payload.get("is_native", False),
        is_global=payload.get("is_global", False),
        user_flags=payload.get("user_flags", 0),
        params=[_param_from_payload(param) for param in payload.get("params", [])],
        locals=[_local_from_payload(local) for local in payload.get("locals", [])],
        instructions=[_instruction_from_payload(instr) for instr in payload.get("instructions", [])],
    )


def _param_from_payload(payload: dict[str, Any]) -> PexParam:
    return PexParam(
        name=payload.get("name", ""),
        type=payload.get("type", ""),
    )


def _local_from_payload(payload: dict[str, Any]) -> PexLocal:
    return PexLocal(
        name=payload.get("name", ""),
        type=payload.get("type", ""),
    )


def _instruction_from_payload(payload: dict[str, Any]) -> PexInstruction:
    try:
        opcode = PexOpcode(payload.get("opcode", PexOpcode.NOP))
    except ValueError:
        opcode = PexOpcode.NOP
    return PexInstruction(
        opcode=opcode,
        args=[_value_from_payload(arg) for arg in payload.get("args", [])],
    )


def _value_from_payload(payload: dict[str, Any] | None) -> PexValue:
    if payload is None:
        return PexValue.none()

    try:
        value_type = ValueType(payload.get("type", ValueType.NONE))
    except ValueError:
        return PexValue.none()

    data = payload.get("data")
    if value_type == ValueType.NONE:
        return PexValue.none()
    if value_type == ValueType.IDENTIFIER:
        return PexValue.identifier(data)
    if value_type == ValueType.STRING:
        return PexValue.string(data)
    if value_type == ValueType.INTEGER:
        return PexValue.integer(data)
    if value_type == ValueType.FLOAT:
        return PexValue.floating(data)
    if value_type == ValueType.BOOL:
        return PexValue.boolean(bool(data))
    return PexValue.none()


# ---------------------------------------------------------------------------
# Writer path — Python PexFile → JSON payload dict → native write_pex_bytes
# ---------------------------------------------------------------------------

def write_pex_bytes_native(pex: PexFile) -> bytes:
    native = load_native_module()
    if native is None:
        raise RuntimeError(_MISSING_NATIVE_MESSAGE)
    payload = _payload_from_pex_file(pex)
    return bytes(native.write_pex_bytes(json.dumps(payload)))


def _payload_from_pex_file(pex: PexFile) -> dict[str, Any]:
    return {
        "magic": pex.magic,
        "major_version": pex.major_version,
        "minor_version": pex.minor_version,
        "game_id": pex.game_id,
        "compilation_time": pex.compilation_time,
        "source_filename": pex.source_filename,
        "username": pex.username,
        "machine_name": pex.machine_name,
        "string_table": list(pex.string_table),
        "debug_info": _payload_from_debug_info(pex.debug_info),
        "user_flags": [{"name": f.name, "index": f.index} for f in pex.user_flags],
        "objects": [_payload_from_object(o) for o in pex.objects],
    }


def _payload_from_debug_info(info: PexDebugInfo | None) -> dict[str, Any] | None:
    if info is None:
        return None
    return {
        "modification_time": info.modification_time,
        "functions": [_payload_from_debug_function(f) for f in info.functions],
        "property_groups": [_payload_from_debug_property_group(g) for g in info.property_groups],
        "struct_orders": [_payload_from_debug_struct_order(so) for so in info.struct_orders],
    }


def _payload_from_debug_function(f: PexDebugFunction) -> dict[str, Any]:
    return {
        "object_name": f.object_name,
        "state_name": f.state_name,
        "function_name": f.function_name,
        "function_type": f.function_type,
        "line_numbers": list(f.line_numbers),
    }


def _payload_from_debug_property_group(g: PexDebugPropertyGroup) -> dict[str, Any]:
    return {
        "object_name": g.object_name,
        "group_name": g.group_name,
        "docstring": g.docstring,
        "user_flags": g.user_flags,
        "property_names": list(g.property_names),
    }


def _payload_from_debug_struct_order(so: PexDebugStructOrder) -> dict[str, Any]:
    return {
        "object_name": so.object_name,
        "struct_name": so.struct_name,
        "member_names": list(so.member_names),
    }


def _payload_from_object(o: PexObject) -> dict[str, Any]:
    return {
        "name": o.name,
        "parent": o.parent,
        "docstring": o.docstring,
        "is_const": o.is_const,
        "auto_state": o.auto_state,
        "structs": [_payload_from_struct(s) for s in o.structs],
        "user_flags": o.user_flags,
        "variables": [_payload_from_variable(v) for v in o.variables],
        "guards": list(o.guards),
        "properties": [_payload_from_property(p) for p in o.properties],
        "states": [_payload_from_state(s) for s in o.states],
    }


def _payload_from_struct(s: PexStruct) -> dict[str, Any]:
    return {
        "name": s.name,
        "members": [_payload_from_struct_member(m) for m in s.members],
    }


def _payload_from_struct_member(m: PexStructMember) -> dict[str, Any]:
    return {
        "name": m.name,
        "type": m.type,
        "user_flags": m.user_flags,
        "data": _payload_from_value(m.data),
        "is_const": m.is_const,
        "docstring": m.docstring,
    }


def _payload_from_variable(v: PexVariable) -> dict[str, Any]:
    return {
        "name": v.name,
        "type": v.type,
        "user_flags": v.user_flags,
        "data": _payload_from_value(v.data),
        "is_const": v.is_const,
    }


def _payload_from_property(p: PexProperty) -> dict[str, Any]:
    return {
        "name": p.name,
        "type": p.type,
        "docstring": p.docstring,
        "user_flags": p.user_flags,
        "flags": p.flags,
        "auto_var": p.auto_var,
        "getter": _payload_from_function(p.getter) if p.getter is not None else None,
        "setter": _payload_from_function(p.setter) if p.setter is not None else None,
    }


def _payload_from_state(s: PexState) -> dict[str, Any]:
    return {
        "name": s.name,
        "functions": [_payload_from_function(f) for f in s.functions],
    }


def _payload_from_function(f: PexFunction) -> dict[str, Any]:
    return {
        "name": f.name,
        "return_type": f.return_type,
        "docstring": f.docstring,
        "is_native": f.is_native,
        "is_global": f.is_global,
        "user_flags": f.user_flags,
        "params": [_payload_from_param(p) for p in f.params],
        "locals": [_payload_from_local(l) for l in f.locals],
        "instructions": [_payload_from_instruction(i) for i in f.instructions],
    }


def _payload_from_param(p: PexParam) -> dict[str, Any]:
    return {"name": p.name, "type": p.type}


def _payload_from_local(l: PexLocal) -> dict[str, Any]:
    return {"name": l.name, "type": l.type}


def _payload_from_instruction(instr: PexInstruction) -> dict[str, Any]:
    return {
        "opcode": int(instr.opcode),
        "args": [_payload_from_value(arg) for arg in instr.args],
    }


def _payload_from_value(v: PexValue) -> dict[str, Any]:
    return {"type": int(v.type), "data": v.data}


# ---------------------------------------------------------------------------
# Compiler path — .psc source → native compile_source → .pex bytes
# ---------------------------------------------------------------------------

@dataclass
class CompileResult:
    ok: bool
    pex_bytes: bytes | None = None
    diagnostics: list[dict[str, Any]] = field(default_factory=list)


def compile_psc(
    source: str,
    *,
    imports: list[str] | None = None,
    game: str = "fo4",
    flags: str | None = None,
    source_path: str | None = None,
) -> CompileResult:
    """Compile Papyrus source to a `.pex` with the identity header fields neutralized."""
    native = load_native_module()
    if native is None:
        raise RuntimeError(_MISSING_NATIVE_MESSAGE)
    meta_json, pex_bytes = native.compile_source(
        source,
        list(imports or []),
        game,
        flags,
        source_path,
    )
    meta = json.loads(meta_json)
    return CompileResult(
        ok=meta["ok"],
        pex_bytes=bytes(pex_bytes) if pex_bytes is not None else None,
        diagnostics=meta.get("diagnostics", []),
    )
