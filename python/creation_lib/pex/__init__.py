"""Pure Python PEX decompiler.

Public API:
    decompile_pex(path) → str    — Decompile a .pex file to Papyrus source text
    parse_pex(path) → PexFile    — Parse a .pex file into structured data
"""
from pathlib import Path

from creation_lib.pex.parser import parse_pex as _parse
from creation_lib.pex.decompiler import decompile_pex_file
from creation_lib.pex.types import PexFile
from creation_lib.papyrus_lsp.native_runtime import emit_script_native as _emit_script


def decompile_pex(
    path: Path,
    *,
    type_adapter=None,
    drop_script_const: bool = False,
    skip_internal_functions: bool = False,
    fo4_api_compat: bool = False,
) -> str:
    """Decompile a .pex file to Papyrus source text."""
    pex_file = _parse(path)
    script_node = decompile_pex_file(
        pex_file,
        type_adapter=type_adapter,
        drop_script_const=drop_script_const,
        skip_internal_functions=skip_internal_functions,
        fo4_api_compat=fo4_api_compat,
    )
    return _emit_script(script_node)


def parse_pex(path: Path) -> PexFile:
    """Parse a .pex file into structured data without decompiling."""
    return _parse(path)
