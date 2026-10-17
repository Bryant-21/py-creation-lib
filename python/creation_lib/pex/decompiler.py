"""Compatibility adapter for the native PEX decompiler."""
from __future__ import annotations

import json
from dataclasses import asdict
from typing import Callable

from creation_lib.papyrus_lsp.ast_nodes import ScriptNode
from creation_lib.papyrus_lsp.native_runtime import _stmt_from, script_node_from_json
from creation_lib.pex.native_runtime import load_native_module
from creation_lib.pex.types import PexFile, PexFunction

TypeAdapter = Callable[[str], str]


def _native():
    native = load_native_module()
    if native is None or not callable(getattr(native, "decompile_pex_json", None)):
        raise RuntimeError("papyrus_core native PEX decompiler is not available")
    return native


def decompile_function(
    fn: PexFunction,
    auto_var_map: dict[str, str] | None = None,
    type_normalizer: object | None = None,
    emit_locals: bool = False,
    external_types: dict[str, str] | None = None,
    fo4_api_compat: bool = False,
) -> list:
    aliases = {}
    if callable(type_normalizer):
        aliases = {item.type: type_normalizer(item.type) for item in (*fn.locals, *fn.params)}
    payload = _native().decompile_function_json(json.dumps({
        "function": asdict(fn),
        "auto_vars": auto_var_map or {},
        "type_aliases": aliases,
        "emit_locals": emit_locals,
        "external_types": external_types or {},
        "fo4_api_compat": fo4_api_compat,
    }))
    return [_stmt_from(statement) for statement in json.loads(payload)]


def decompile_pex_file(
    pex: PexFile,
    *,
    type_adapter: TypeAdapter | None = None,
    drop_script_const: bool = False,
    skip_internal_functions: bool = False,
    fo4_api_compat: bool = False,
) -> ScriptNode:
    native = _native()
    payload = json.dumps(asdict(pex))
    aliases = {}
    if type_adapter is not None:
        aliases = {name.lower(): type_adapter(name) for name in native.decompile_pex_type_names(payload)}
    ast = native.decompile_pex_json(payload, json.dumps({
        "type_aliases": aliases,
        "drop_script_const": drop_script_const,
        "skip_internal_functions": skip_internal_functions,
        "fo4_api_compat": fo4_api_compat,
    }))
    return script_node_from_json(json.loads(ast))
