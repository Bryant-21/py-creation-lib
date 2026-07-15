"""Phase 5 PyO3 smoke test — confirms the new cloth pyfunctions are wired up."""
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


def test_cloth_extract_blob_skeleton_returns_structured_error():
    nif = _nif_native()
    nif_bytes = (Path(__file__).parent.parent.parent / "resource" / "skeleton.nif").read_bytes()
    with pytest.raises(ValueError, match="No BSClothExtraData"):
        nif.cloth_extract_blob(nif_bytes)


def test_cloth_pack_blob_round_trip_with_skeleton_fixture():
    nif = _nif_native()
    base = Path(__file__).parent.parent.parent / "resource"
    nif_bytes = (base / "skeleton.nif").read_bytes()
    blob_bytes = (base / "skeleton.hkx").read_bytes()

    packed = nif.cloth_pack_blob(nif_bytes, blob_bytes)
    extracted = nif.cloth_extract_blob(packed)
    assert extracted == blob_bytes

    repacked = nif.cloth_pack_blob(packed, blob_bytes)
    assert repacked == packed


def test_cloth_metadata_from_blob_reports_class_inventory():
    havok = _havok_native()
    blob_bytes = (Path(__file__).parent.parent.parent / "resource" / "skeleton.hkx").read_bytes()

    metadata_json = havok.cloth_metadata_from_blob(blob_bytes)
    metadata = json.loads(metadata_json)

    assert metadata["object_count"] > 0
    classes = [entry["class_name"] for entry in metadata["class_inventory"]]
    assert "hkRootLevelContainer" in classes


def test_phase5_pyfunctions_are_registered():
    havok = _havok_native()
    expected_havok = {
        "cloth_bake",
        "cloth_validate",
        "cloth_simulate",
        "cloth_metadata_from_blob",
    }
    listed = set(dir(havok))
    missing = expected_havok - listed
    assert not missing, f"missing havok pyfunctions: {missing}"

    nif = _nif_native()
    expected_nif = {
        "cloth_extract_blob",
        "cloth_pack_blob",
        "cloth_extract_blobs",
        "cloth_template_apply",
    }
    missing = expected_nif - set(dir(nif))
    assert not missing, f"missing nif pyfunctions: {missing}"


def test_cloth_bake_min_fixture_produces_hkx_magic():
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


def test_cloth_validate_skeleton_hkx_reports_no_cloth_data():
    havok = _havok_native()
    blob_bytes = (Path(__file__).parent.parent.parent / "resource" / "skeleton.hkx").read_bytes()

    result_json = havok.cloth_validate(blob_bytes)
    result = json.loads(result_json)

    # skeleton.hkx has no hclClothData — should produce a NO_CLOTH_DATA error
    codes = [issue["code"] for issue in result.get("issues", [])]
    assert "NO_CLOTH_DATA" in codes, (
        f"expected NO_CLOTH_DATA issue but got: {codes}"
    )
    assert result["valid"] is False


def test_cloth_simulate_returns_positions_json():
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

    result_json = havok.cloth_simulate(setup_json, 10, None)
    result = json.loads(result_json)

    assert "positions" in result, f"missing 'positions' key in: {result}"
    assert len(result["positions"]) > 0, "setup with particles in → positions out"
