"""PyO3 smoke test: the cloth pyfunctions are wired up."""
from __future__ import annotations

from pathlib import Path
import json

import pytest


def _havok_native():
    """Import havok_native; skip the whole module if it isn't built."""
    pytest.importorskip("creation_lib._native.havok_native")
    from creation_lib._native import havok_native
    return havok_native


def _nif_native():
    """Import nif_core_native; skip the whole module if it isn't built."""
    pytest.importorskip("creation_lib._native.nif_core_native")
    from creation_lib._native import nif_core_native
    return nif_core_native


def test_cloth_pack_extract_round_trip_and_missing_data_error():
    nif = _nif_native()
    base = Path(__file__).parent.parent.parent / "resource"
    nif_bytes = (base / "skeleton.nif").read_bytes()
    blob_bytes = (base / "skeleton.hkx").read_bytes()

    with pytest.raises(ValueError, match="No BSClothExtraData"):
        nif.cloth_extract_blob(nif_bytes)

    packed = nif.cloth_pack_blob(nif_bytes, blob_bytes)
    extracted = nif.cloth_extract_blob(packed)
    assert extracted == blob_bytes

    repacked = nif.cloth_pack_blob(packed, blob_bytes)
    assert repacked == packed


def test_cloth_bake_and_simulate_from_min_fixture():
    havok = _havok_native()
    fixture_path = (
        Path(__file__).parent.parent.parent
        / "py_creation_lib"
        / "native"
        / "havok"
        / "tests"
        / "fixtures"
        / "cloth_setup_min.json"
    )
    setup_json = fixture_path.read_text(encoding="utf-8")

    result = havok.cloth_bake(setup_json)

    # HKX packfile magic: 0x57 0xE0 0xE0 0x57 0x10 0xC0 0xC0 0x10
    assert result[:8] == b"\x57\xE0\xE0\x57\x10\xC0\xC0\x10", (
        f"cloth_bake did not return a packfile; first 8 bytes: {result[:8].hex()}"
    )

    result_json = havok.cloth_simulate(setup_json, 10, None)
    result = json.loads(result_json)

    assert "positions" in result, f"missing 'positions' key in: {result}"
    assert len(result["positions"]) > 0, "setup with particles in → positions out"
