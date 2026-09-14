"""End-to-end test of the Python LOD facade against the real FO4 install.

Drives `creation_lib.lod.generate_lod` through the umbrella `_native.pyd` with REAL
ESP enumeration (the `lodgen_native/real-esp` feature, enabled in the umbrella
`lodgen` feature). Skips when the FO4 install is absent.

`run()` in `lodgen_native/src/lib.rs` only opens a plugin named
`<world_editor_id>.es[mp]`, falling back to `Fallout4.esm`, so FarHarbor
(`DLC03FarHarbor`, in `DLCCoast.esm`) is unreachable through the facade.
`DiamondCity` is in `Fallout4.esm`, small (a few hundred cells), and has terrain
LAND and placed objects, so it covers terrain `.btr` + `.dds`, object `.bto` and
`.lod` quickly.

The DDS-header assertion checks that the external `directxtex` (via `bsarchive`)
and `directxtex_native` coexist in the cdylib without the `E_NOTIMPL` /
wrong-copy collision: a real BC-encoded terrain texture is written and parses
as a valid DDS.
"""
from __future__ import annotations

import json
import os
import struct
from pathlib import Path

import pytest

from creation_lib.lod import generate_lod, is_available
from creation_lib.lod.default_settings import fo4_default_settings

FO4_DATA = Path(os.environ.get("FO4_DATA") or "")
EXTRACTED_FO4 = Path(
    os.environ.get("FO4_EXTRACTED_DIR")
    or Path(__file__).resolve().parents[5] / "extracted" / "fo4"
)
WORLD_EDID = "DiamondCity"


def _require_game() -> list[str]:
    if not is_available():
        pytest.skip("lodgen_native unavailable (native not built)")
    if not (FO4_DATA / "Fallout4.esm").is_file():
        pytest.skip(f"FO4 install not found (set FO4_DATA): {FO4_DATA}")
    data_dirs = [str(FO4_DATA)]
    if EXTRACTED_FO4.is_dir():
        data_dirs.append(str(EXTRACTED_FO4))
    return data_dirs


def _valid_dds_header(path: Path) -> tuple[int, int]:
    """Return (width, height) if `path` is a valid DDS, else raise AssertionError."""
    hdr = path.read_bytes()[:20]
    assert hdr[:4] == b"DDS ", f"{path.name}: bad magic {hdr[:4]!r}"
    assert struct.unpack("<I", hdr[4:8])[0] == 124, f"{path.name}: bad dwSize"
    height = struct.unpack("<I", hdr[12:16])[0]
    width = struct.unpack("<I", hdr[16:20])[0]
    assert width > 0 and height > 0, f"{path.name}: {width}x{height}"
    return width, height


def test_generate_lod_end_to_end(tmp_path):
    data_dirs = _require_game()
    events: list[tuple[str, float]] = []

    result = generate_lod(
        WORLD_EDID,
        fo4_default_settings(),
        data_dirs=data_dirs,
        output_dir=str(tmp_path),
        progress=lambda msg, frac: events.append((msg, frac)),
    )

    # Terrain MUST have run.
    assert result.btr > 0, f"no terrain .btr produced: {result}"
    assert result.dds > 0, f"no terrain .dds produced: {result}"
    assert result.lod_written, "LODSettings .lod not written"
    assert events, "progress callback never fired"

    btr = list(tmp_path.rglob("*.btr"))
    bto = list(tmp_path.rglob("*.bto"))
    dds = list(tmp_path.rglob("*.dds"))
    lod = list(tmp_path.rglob("*.lod"))

    assert len(btr) == result.btr, f"stats.btr={result.btr} but {len(btr)} .btr files"
    assert len(dds) == result.dds, f"stats.dds={result.dds} but {len(dds)} .dds files"
    assert lod, "no .lod file on disk"
    assert lod[0].stat().st_size > 0, "LODSettings .lod is empty"

    # Every terrain DDS is a valid DDS (the directxtex-in-.pyd proof).
    for d in dds:
        _valid_dds_header(d)

    # A terrain .btr parses as a real NIF with geometry.
    from creation_lib._native import nif_core_native

    model = nif_core_native.load_nif(str(btr[0]))
    assert model["blocks"], f"{btr[0].name} parsed to no NIF blocks"

    # Objects are produced for DiamondCity (sparse is acceptable elsewhere, but this
    # world has placed LOD statics); when present, every .bto parses too.
    assert result.bto == len(bto), f"stats.bto={result.bto} but {len(bto)} .bto files"
    for b in bto:
        m = nif_core_native.load_nif(str(b))
        assert m["blocks"], f"{b.name} parsed to no NIF blocks"


def test_settings_serde_roundtrip_runs(tmp_path):
    """The Python default settings dict -> JSON -> native parse contract.

    Asserts the default-settings JSON the facade emits is accepted by the native
    serde `LodSettings` parser and yields a valid run (not a JSON/serde error).
    """
    data_dirs = _require_game()

    settings_dict = fo4_default_settings()
    settings_json = json.dumps(settings_dict)
    # Round-trips losslessly on the Python side.
    assert json.loads(settings_json) == settings_dict

    # The native parser accepts it and produces real stats.
    result = generate_lod(
        WORLD_EDID,
        settings_json,
        data_dirs=data_dirs,
        output_dir=str(tmp_path),
        progress=None,
    )
    assert result.btr > 0 and result.dds > 0 and result.lod_written
