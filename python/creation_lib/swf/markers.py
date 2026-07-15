"""FO76 → FO4 map-marker icon injection: canonical table + deterministic build.

The ``(fo76_type, fo4_byte, symbol_name)`` table is owned by the Rust
``esp_authoring_core`` crate (``marker_type.rs::FO76_CUSTOM_ICONS``) and exposed
via ``_native.esp_authoring_core.fo76_custom_marker_icons()``. Everything here is
derived from that single source so the schema enum, the injected SWF symbols, and
the F4SE hook never drift apart.

The build is deterministic: the native injector uses a fixed character-id offset,
sorted closure order, and fixed zlib parameters, so building twice from the same
inputs yields byte-identical SWFs. The sidecar JSON is sorted likewise.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from importlib import import_module
from pathlib import Path
from typing import Any

from creation_lib.swf import native_runtime


@dataclass(frozen=True)
class MarkerIcon:
    fo76_type: int
    fo4_byte: int
    source_symbol: str  # SymbolClass export name in FO76's mapmarkerlibrary.swf
    symbol: str  # SymbolClass export name to register in FO4 (the F4SE hook key)


def marker_icon_table() -> list[MarkerIcon]:
    """The canonical FO76→FO4 custom marker table, sorted by FO4 byte."""
    esp = getattr(import_module("creation_lib._native"), "esp_authoring_core", None)
    if esp is None or not callable(getattr(esp, "fo76_custom_marker_icons", None)):
        raise RuntimeError(
            "esp_authoring_core.fo76_custom_marker_icons is missing — "
            "rebuild the native extension (scripts/ensure_native.py)"
        )
    rows = [
        MarkerIcon(int(src), int(fo4), str(source), str(export))
        for src, fo4, source, export in esp.fo76_custom_marker_icons()
    ]
    rows.sort(key=lambda m: m.fo4_byte)
    return rows


def missing_source_symbols(fo76_lib: bytes, table: list[MarkerIcon] | None = None) -> list[str]:
    """Source symbol names from the canonical table absent from the FO76 library —
    a drift check before injection (the injector would otherwise raise on the first
    missing name)."""
    table = table or marker_icon_table()
    present = {name for _cid, name in native_runtime.list_symbols(fo76_lib)}
    return [m.source_symbol for m in table if m.source_symbol not in present]


def abc_class_name_presence(
    fo4_swf: bytes, table: list[MarkerIcon] | None = None
) -> dict[str, bool]:
    """For each canonical FO4 export name, whether an AS3 identifier of that name is
    already present in the SWF's ABC string pool (i.e. it already has a backing
    class). FO76-only marker exports are expected absent — evidence that injecting
    them as SymbolClass-only may need a synthesized backing class to match FO4's
    class-backed marker pattern."""
    table = table or marker_icon_table()
    strings: set[str] = set()
    for _code, _minor, _major, _i, _u, _d, strs in native_runtime.abc_string_pools(fo4_swf):
        strings.update(strs)
    return {m.symbol: (m.symbol in strings) for m in table}


def marker_sidecar(swfs: list[str] | None = None) -> dict[str, Any]:
    """The `marker_injection.json` payload: the byte->symbol map the F4SE hook reads,
    plus the list of injected SWF names (empty when written standalone)."""
    table = marker_icon_table()
    return {
        "markers": [
            {
                "fo76_type": m.fo76_type,
                "fo4_byte": m.fo4_byte,
                "symbol": m.symbol,
                "source_symbol": m.source_symbol,
            }
            for m in table
        ],
        "swfs": swfs or [],
    }


def write_marker_sidecar(out_path: Path, swfs: list[str] | None = None) -> Path:
    """Write `marker_injection.json` for the F4SE companion. Deterministic; safe to
    write standalone (no SWF build needed) so the companion can ship the map."""
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(marker_sidecar(swfs), indent=2), encoding="utf-8")
    return out_path


def build_marker_swfs(
    fo76_lib: Path,
    fo4_swfs: list[Path],
    out_dir: Path,
) -> dict[str, Any]:
    """Inject all 42 FO76 region-marker symbols into each FO4 menu SWF and write a
    ``marker_injection.json`` sidecar (byte→symbol for the F4SE hook).

    Returns a summary dict. Raises if a target SWF has no SymbolClass tag or if a
    canonical symbol is missing from the FO76 source library.
    """
    table = marker_icon_table()
    pairs = [(m.source_symbol, m.symbol) for m in table]

    src = fo76_lib.read_bytes()
    missing = missing_source_symbols(src, table)
    if missing:
        raise ValueError(
            f"FO76 library {fo76_lib.name} is missing {len(missing)} marker symbol(s): "
            + ", ".join(missing)
        )

    out_dir.mkdir(parents=True, exist_ok=True)
    results: list[dict[str, Any]] = []
    for swf in fo4_swfs:
        dst = swf.read_bytes()
        injected = native_runtime.inject_symbols_renamed(src, dst, pairs)
        out_path = out_dir / swf.name
        out_path.write_bytes(injected)
        results.append(
            {
                "name": swf.name,
                "in_bytes": len(dst),
                "out_bytes": len(injected),
                "symbols_added": len(pairs),
            }
        )

    sidecar_path = out_dir / "marker_injection.json"
    sidecar = marker_sidecar([r["name"] for r in results])
    sidecar_path.write_text(json.dumps(sidecar, indent=2), encoding="utf-8")

    return {
        "symbols": len(pairs),
        "swfs": results,
        "sidecar": str(sidecar_path),
    }
