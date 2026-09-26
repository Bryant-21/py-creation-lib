"""Thin Python boundary for native ``swf_native`` entrypoints (byte-exact SWF).

The pure-Python ``creation_lib.swf`` codec (``parser``/``writer``) re-minimizes
shape bit-widths and is therefore byte-LOSSY — it must NOT be used to edit real
menu SWFs. This module wraps the Rust ``swf_native`` crate, which treats
untouched tags as opaque byte ranges and splices new symbols in without
disturbing them. Use it for marker-symbol injection (FO76 → FO4).
"""

from __future__ import annotations

from collections.abc import Iterable

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
    "abc_class_names",
    "unbacked_symbol_classes",
    "build_movieclip_class_doabc",
    "inject_symbols_into",
    "inject_symbols_renamed_into",
)
# `compile_as3_do_abc` / `compile_as3_class_names` are deliberately absent from
# the probe above. It decides whether a `.pyd` is usable at all, so listing a
# newly-added function there makes every extension built before it fail to load
# — taking out inspection and injection, which do not need the compiler. Callers
# that need the compiler get an AttributeError naming it instead.


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


def render_symbol_png(data: bytes, name: str, frame: int = 1, scale: float = 1.0) -> bytes:
    return bytes(load_native_module().render_symbol_png(bytes(data), name, frame, scale))


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


def abc_class_names(data: bytes) -> list[str]:
    """Fully-qualified names (``package.Class``, or bare ``Class`` in the unnamed
    package) of every AS3 class *defined* by a DoABC tag in this SWF."""
    return load_native_module().abc_class_names(bytes(data))


def unbacked_symbol_classes(data: bytes) -> list[str]:
    """SymbolClass export names no DoABC in the same SWF defines.

    Non-empty means the file ships a dangling binding — a character id pointing
    at a class that does not exist, which leaves the engine unable to construct
    that symbol."""
    return load_native_module().unbacked_symbol_classes(bytes(data))


def build_movieclip_class_doabc(names: list[str]) -> bytes:
    """DoABCDefine (tag 82) *body* defining one ``flash.display.MovieClip``
    subclass with an empty constructor per name. The caller writes the tag header
    and must place the tag ahead of the SymbolClass that binds these names."""
    return bytes(load_native_module().build_movieclip_class_doabc([str(n) for n in names]))


def compile_as3_do_abc(sources: list[str]) -> bytes:
    """Compile ActionScript 3 sources to a DoABCDefine (tag 82) *body*.

    Each element is the full text of one ``.as`` file. AS3 allows one package
    per file, so a widget's document class and the interface it implements are
    separate entries; order does not matter, because types are sorted so a base
    class or interface is defined before whatever depends on it. The caller
    writes the tag header and must place the tag ahead of the SymbolClass that
    binds these classes."""
    return bytes(load_native_module().compile_as3_do_abc([str(s) for s in sources]))


def replace_as3_classes(data: bytes, sources: dict[str, str], dependencies: Iterable[bytes] = ()) -> bytes:
    return bytes(load_native_module().replace_as3_classes(bytes(data), sources, list(dependencies)))


def augment_as3_classes(data: bytes, sources: dict[str, str], dependencies: Iterable[bytes] = ()) -> bytes:
    return bytes(load_native_module().augment_as3_classes(bytes(data), sources, list(dependencies)))


def rename_as3_classes(data: bytes, prefix: str, keep: Iterable[str] = ()) -> bytes:
    """Move every class the movie defines under ``prefix`` so a host menu's classes
    of the same name cannot take their place; ``keep`` lists packages to leave
    alone, such as the player's own ``scaleform.gfx`` stubs."""
    return bytes(load_native_module().rename_as3_classes(bytes(data), str(prefix), [str(k) for k in keep]))


def patch_as3_method(data: bytes, class_name: str, method_name: str, pattern: list, replacement: list,
                     *, expected_matches: int = 1) -> bytes:
    import json
    return bytes(load_native_module().patch_as3_method(
        bytes(data), class_name, method_name, json.dumps(pattern), json.dumps(replacement), expected_matches))


def compile_as3_class_names(sources: list[str]) -> list[str]:
    """Fully-qualified names of every class the given ActionScript defines.

    Use this to check that each SymbolClass export a packer is about to write is
    actually backed by a compiled class, before the SWF ships."""
    return list(load_native_module().compile_as3_class_names([str(s) for s in sources]))


def abc_class_outline(data: bytes, class_name: str) -> dict[str, Any]:
    """Superclass, interfaces, traits and method signatures of one AS3 class."""
    import json
    return json.loads(load_native_module().abc_class_outline(bytes(data), str(class_name)))


def abc_disassemble(data: bytes, class_name: str, method: str | None = None) -> list[dict[str, Any]]:
    """Instruction listings for a class's methods; ``method`` matches a label such
    as ``populate``, ``get total`` or just ``total`` (both accessors)."""
    import json
    return json.loads(load_native_module().abc_disassemble(bytes(data), str(class_name), method))


def abc_class_references(data: bytes, class_name: str, transitive: bool = False) -> dict[str, Any]:
    """Names a class depends on, split into movie-defined, built-in and external
    classes plus accessed members; ``transitive`` follows movie-defined classes."""
    import json
    return json.loads(load_native_module().abc_class_references(bytes(data), str(class_name), bool(transitive)))


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
