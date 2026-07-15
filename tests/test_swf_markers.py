"""FO76 -> FO4 map-marker icon injection (UI-integration phase, workstream A4).

Locks the invariants the deterministic build must hold; gated on the extracted
game SWFs (skipped when absent) and the native extension.
"""
from __future__ import annotations

from collections import Counter
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_FO76_LIB = _REPO / "extracted" / "fo76" / "interface" / "mapmarkerlibrary.swf"
_FO4_DIR = _REPO / "extracted" / "fo4" / "Interface"
_FO4_TARGETS = [_FO4_DIR / n for n in ("MapMarkers.swf", "HUDMenu.swf", "Pipboy_MapPage.swf")]

_HAS_ASSETS = _FO76_LIB.is_file() and all(p.is_file() for p in _FO4_TARGETS)

pytestmark = pytest.mark.skipif(
    not _HAS_ASSETS, reason="extracted FO76/FO4 marker SWFs not available"
)


def _table():
    from creation_lib.swf.markers import marker_icon_table

    return marker_icon_table()


def test_canonical_table_shape():
    table = _table()
    assert len(table) == 42
    assert [m.fo4_byte for m in table] == list(range(81, 123))
    assert [m.fo76_type for m in table] == list(range(64, 106))
    # Every FO4 export name is unique (no SymbolClass collision possible).
    exports = [m.symbol for m in table]
    assert len(set(exports)) == 42
    # Only Monorail is renamed away from its FO76 source symbol.
    renamed = [(m.fo4_byte, m.source_symbol, m.symbol) for m in table if m.source_symbol != m.symbol]
    assert renamed == [(112, "MonorailMarker", "WhitespringMonorailMarker")]


def test_build_is_deterministic(tmp_path: Path):
    from creation_lib.swf.markers import build_marker_swfs

    a = build_marker_swfs(_FO76_LIB, _FO4_TARGETS, tmp_path / "a")
    b = build_marker_swfs(_FO76_LIB, _FO4_TARGETS, tmp_path / "b")
    assert a["symbols"] == 42
    for r in a["swfs"]:
        assert (tmp_path / "a" / r["name"]).read_bytes() == (tmp_path / "b" / r["name"]).read_bytes()
    assert (tmp_path / "a" / "marker_injection.json").read_bytes() == (
        tmp_path / "b" / "marker_injection.json"
    ).read_bytes()


def test_fo76_markers_absent_from_fo4_abc():
    """Evidence for the A3 class-synthesis decision: FO4's markers are class-backed,
    and none of the 42 FO76 marker exports already exist as AS3 classes in the FO4
    menu SWFs (so SymbolClass-only injection supplies no backing class)."""
    from creation_lib.swf import native_runtime
    from creation_lib.swf.markers import abc_class_name_presence

    for target in _FO4_TARGETS:
        data = target.read_bytes()
        pools = native_runtime.abc_string_pools(data)
        assert pools, f"{target.name}: no DoABC tag parsed"
        assert any(len(strs) > 0 for *_h, strs in pools), f"{target.name}: empty ABC string pool"
        presence = abc_class_name_presence(data)
        assert presence and not any(presence.values()), (
            f"{target.name}: unexpectedly already defines FO76 marker classes "
            f"{[n for n, ok in presence.items() if ok]}"
        )


def test_no_duplicate_exports_and_native_monorail_preserved(tmp_path: Path):
    from creation_lib.swf import native_runtime
    from creation_lib.swf.markers import build_marker_swfs

    build_marker_swfs(_FO76_LIB, _FO4_TARGETS, tmp_path)
    table = _table()
    for target in _FO4_TARGETS:
        out = (tmp_path / target.name).read_bytes()
        names = [n for _cid, n in native_runtime.list_symbols(out)]
        # No SymbolClass export name appears twice.
        assert not {n: c for n, c in Counter(names).items() if c > 1}, f"dup export in {target.name}"
        # The result still tiles cleanly and ends on End.
        assert native_runtime.roundtrip_ok(out)
        # Every canonical export is present.
        assert all(m.symbol in names for m in table)
        # FO4's stock MonorailMarker (icon 73) is left untouched where it existed.
        orig = [n for _cid, n in native_runtime.list_symbols(target.read_bytes())]
        assert names.count("MonorailMarker") == orig.count("MonorailMarker")
