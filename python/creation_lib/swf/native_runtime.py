"""Thin Python boundary for native ``swf_native`` entrypoints (byte-exact SWF).

The pure-Python ``creation_lib.swf`` codec (``parser``/``writer``) re-minimizes
shape bit-widths and is therefore byte-LOSSY — it must NOT be used to edit real
menu SWFs. This module wraps the Rust ``swf_native`` crate, which treats
untouched tags as opaque byte ranges and splices new symbols in without
disturbing them. Use it for marker-symbol injection (FO76 → FO4).
"""

from __future__ import annotations

from importlib import import_module
from typing import Any

_NATIVE_MODULE: Any | None = None
_NATIVE_IMPORT_ATTEMPTED = False

_CAPABILITIES = (
    "swf_info",
    "list_symbols",
    "tag_histogram",
    "roundtrip_ok",
    "abc_string_pools",
    "inject_symbols_into",
    "inject_symbols_renamed_into",
)


def _looks_like_native_module(module: Any | None) -> bool:
    if module is None:
        return False
    return all(callable(getattr(module, name, None)) for name in _CAPABILITIES)


def load_native_module() -> Any:
    global _NATIVE_MODULE, _NATIVE_IMPORT_ATTEMPTED
    if _NATIVE_IMPORT_ATTEMPTED:
        if _NATIVE_MODULE is None:
            raise RuntimeError("swf_native is required for byte-exact SWF operations")
        return _NATIVE_MODULE
    _NATIVE_IMPORT_ATTEMPTED = True
    umbrella = import_module("creation_lib._native")
    module = getattr(umbrella, "swf_native", None)
    if not _looks_like_native_module(module):
        raise RuntimeError(
            "creation_lib._native.swf_native is missing or incomplete — "
            "rebuild the native extension (scripts/ensure_native.py)"
        )
    _NATIVE_MODULE = module
    return module


def swf_info(data: bytes) -> dict[str, Any]:
    sig, version, file_length, decompressed_total, num_tags = load_native_module().swf_info(
        bytes(data)
    )
    return {
        "signature": sig,
        "version": version,
        "file_length_field": file_length,
        "decompressed_total": decompressed_total,
        "num_tags": num_tags,
    }


def list_symbols(data: bytes) -> list[tuple[int, str]]:
    """``(character_id, export_name)`` for every SymbolClass entry, in file order."""
    return load_native_module().list_symbols(bytes(data))


def tag_histogram(data: bytes) -> list[tuple[int, int]]:
    """``(tag_code, count)`` sorted by code — a structural fingerprint."""
    return load_native_module().tag_histogram(bytes(data))


def roundtrip_ok(data: bytes) -> bool:
    """True iff the tag splitter tiles the whole movie and ends on an End tag."""
    return load_native_module().roundtrip_ok(bytes(data))


def abc_string_pools(data: bytes) -> list[tuple[int, int, int, int, int, int, list[str]]]:
    """Per DoABC tag, its constant-pool string table (where every AS3 class name
    lives): ``(tag_code, minor, major, int_count, uint_count, double_count, strings)``.
    Read-only — parses up through the string pool and stops."""
    return load_native_module().abc_string_pools(bytes(data))


def inject_symbols(src: bytes, dst: bytes, names: list[str]) -> bytes:
    """Inject the named SymbolClass symbols (with their full character closures)
    from ``src`` into ``dst``; return the re-assembled destination SWF bytes."""
    out = load_native_module().inject_symbols_into(bytes(src), bytes(dst), list(names))
    return bytes(out)


def inject_symbols_renamed(src: bytes, dst: bytes, pairs: list[tuple[str, str]]) -> bytes:
    """Like :func:`inject_symbols` but each entry is ``(source_symbol, export_name)``
    so a symbol can be registered in ``dst`` under a different SymbolClass export
    name than it has in ``src`` (rename to avoid colliding with a stock export)."""
    out = load_native_module().inject_symbols_renamed_into(
        bytes(src), bytes(dst), [(str(s), str(e)) for s, e in pairs]
    )
    return bytes(out)
