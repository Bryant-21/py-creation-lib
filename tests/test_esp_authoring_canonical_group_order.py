"""Authoring-dir → ESP build emits top-level GRUPs in the engine's canonical
record-type order, not alphabetical.

When KYWD records appear *after* records that reference them (COBJ.FNAM,
CONT.KWDA, ACTI.KWDA, …), CK's forward-pass loader can't resolve the
references and logs ``[FORMS] Unable to find keyword (XXXXXXXX)``.
Alphabetical order puts ACTI/ARTO/COBJ/CONT before KYWD, so native authoring
must emit groups in the canonical order extracted from each game's vanilla ESM.

These tests scaffold authoring dirs whose record-type subdirs would sort
alphabetically into the *wrong* order, build, and assert the resulting
binary's top-level GRUP labels appear in the canonical order.
"""

from __future__ import annotations

import json
import struct
from pathlib import Path

import pytest

from creation_lib.esp.native_runtime import load_native_module
import creation_lib.esp.api as esp_api


pytestmark = pytest.mark.skipif(
    load_native_module() is None,
    reason="esp_authoring_core is not installed",
)


def _scaffold_plugin_json(authoring_dir: Path, plugin_name: str, next_object_id: str = "000810") -> None:
    authoring_dir.mkdir(parents=True, exist_ok=True)
    (authoring_dir / "plugin.json").write_text(
        json.dumps(
            {
                "plugin": plugin_name,
                "game": "fo4",
                "header": {
                    "version": 1.0,
                    "num_records": 0,
                    "next_object_id": next_object_id,
                    "author": "",
                    "description": "",
                    "masters": [],
                    "master_sizes": [],
                    "overridden_forms": [],
                    "flags": "00000000",
                    "version_control": 0,
                    "extra_subrecords": [],
                },
            }
        ),
        encoding="utf-8",
    )


def _write_record_json(
    authoring_dir: Path,
    signature: str,
    file_stem: str,
    form_id: str,
    plugin_name: str,
    edid_text: str,
    extra_subrecords: list[dict] | None = None,
) -> None:
    sig_dir = authoring_dir / "records" / signature
    sig_dir.mkdir(parents=True, exist_ok=True)
    edid_hex = edid_text.encode("ascii").hex() + "00"
    subrecords = [{"signature": "EDID", "data_hex": edid_hex}]
    if extra_subrecords:
        subrecords.extend(extra_subrecords)
    (sig_dir / f"{file_stem}.json").write_text(
        json.dumps(
            {
                "form_id": f"{form_id}:{plugin_name}",
                "subrecords": subrecords,
            }
        ),
        encoding="utf-8",
    )


def _read_top_level_groups(esp_bytes: bytes) -> list[str]:
    """Return the deduped list of top-level GRUP labels in file order."""
    tes4_data_size = struct.unpack_from("<I", esp_bytes, 4)[0]
    offset = 24 + tes4_data_size
    groups: list[str] = []
    while offset + 24 <= len(esp_bytes):
        if esp_bytes[offset : offset + 4] == b"GRUP":
            grup_size = struct.unpack_from("<I", esp_bytes, offset + 4)[0]
            label = esp_bytes[offset + 8 : offset + 12]
            group_type = struct.unpack_from("<i", esp_bytes, offset + 12)[0]
            if group_type == 0:
                try:
                    groups.append(label.decode("ascii"))
                except UnicodeDecodeError:
                    pass
            offset += grup_size
        else:
            offset += 1
    return groups


def test_fo4_authoring_build_emits_canonical_top_level_group_order(tmp_path: Path) -> None:
    """KYWD must precede COBJ/ACTI/CONT/ARMO etc. in the built binary, even
    when authoring-dir subdir names would sort alphabetically into the
    opposite order. This is what Bethesda's engine and CK require — without
    it, CK reports `[FORMS] Unable to find keyword`.
    """
    authoring_dir = tmp_path / "canonical_order_authoring"
    output_path = tmp_path / "CanonicalOrder.esp"
    plugin_name = "CanonicalOrder.esp"
    _scaffold_plugin_json(authoring_dir, plugin_name)

    # Author records in record-type subdirs whose alphabetical sort places
    # them BEFORE KYWD. If the codec emitted alphabetically the binary would
    # load with `[FORMS] Unable to find keyword` errors in CK.
    _write_record_json(authoring_dir, "ACTI", "TestActi", "000800", plugin_name, "TestActi")
    _write_record_json(authoring_dir, "ARMO", "TestArmo", "000801", plugin_name, "TestArmo")
    _write_record_json(authoring_dir, "COBJ", "TestCobj", "000802", plugin_name, "TestCobj")
    _write_record_json(authoring_dir, "CONT", "TestCont", "000803", plugin_name, "TestCont")
    _write_record_json(authoring_dir, "KYWD", "TestKywd", "000804", plugin_name, "TestKywd")
    _write_record_json(authoring_dir, "MISC", "TestMisc", "000805", plugin_name, "TestMisc")

    esp_api._native_runtime.build_authoring_dir_streaming_native(
        str(authoring_dir),
        str(output_path),
        game="fo4",
        jobs=1,
    )

    groups = _read_top_level_groups(output_path.read_bytes())
    assert groups, "no top-level groups found in built ESP"

    # Canonical FO4 order has KYWD at rank 1, then ACTI(19), CONT(23), ARMO(21),
    # MISC(27), COBJ(114). Assert KYWD precedes every keyword-referencing type.
    assert "KYWD" in groups, f"KYWD group missing from output, got: {groups}"
    kywd_pos = groups.index("KYWD")
    for sig in ("ACTI", "ARMO", "CONT", "COBJ", "MISC"):
        if sig in groups:
            assert groups.index(sig) > kywd_pos, (
                f"{sig} group must follow KYWD in canonical FO4 order, "
                f"got order: {groups}"
            )

    # Specifically: ACTI before CONT before ARMO before MISC before COBJ
    # (ranks 19 < 21 < 23 < 27 < 114 in the canonical order).
    expected_subset_order = ["KYWD", "ACTI", "ARMO", "CONT", "MISC", "COBJ"]
    actual_subset = [sig for sig in groups if sig in expected_subset_order]
    assert actual_subset == expected_subset_order, (
        f"FO4 groups out of canonical order; expected {expected_subset_order}, got {actual_subset}"
    )


def test_fo4_alphabetical_authoring_dir_does_not_dictate_emit_order(tmp_path: Path) -> None:
    """Build twice — once with subdirs created in alphabetical order, once in
    canonical order — and assert both binaries emit groups in the same
    canonical order. The codec must not depend on filesystem readdir order.
    """
    plugin_name = "OrderIndependence.esp"

    def build(create_in_order: list[str]) -> bytes:
        d = tmp_path / f"order_{'_'.join(create_in_order)}"
        _scaffold_plugin_json(d, plugin_name)
        for i, sig in enumerate(create_in_order):
            _write_record_json(d, sig, f"Test{sig}", f"00080{i}", plugin_name, f"Test{sig}")
        out = tmp_path / f"out_{'_'.join(create_in_order)}.esp"
        esp_api._native_runtime.build_authoring_dir_streaming_native(
            str(d), str(out), game="fo4", jobs=1
        )
        return out.read_bytes()

    alpha_groups = _read_top_level_groups(build(["ACTI", "COBJ", "KYWD", "MISC"]))
    canon_groups = _read_top_level_groups(build(["KYWD", "ACTI", "MISC", "COBJ"]))
    assert alpha_groups == canon_groups, (
        f"build is sensitive to authoring-dir creation order: "
        f"alpha-input → {alpha_groups}, canonical-input → {canon_groups}"
    )


def _build_synthetic_master_esm(out_path: Path, group_signatures: list[str]) -> None:
    """Write a minimal valid FO4-style ESM with one empty top-level GRUP per
    signature, in the order given. Used to verify live group-order discovery
    reads from the actual master rather than the hardcoded baseline.
    """
    import struct

    parts: list[bytes] = []
    # TES4 record (24-byte header + minimal HEDR + CNAM + INTV)
    hedr_data = struct.pack("<fII", 1.0, 0, 0x00000800)
    cnam_data = b"test\x00"
    intv_data = struct.pack("<I", 1)
    payload = b""
    payload += b"HEDR" + struct.pack("<H", len(hedr_data)) + hedr_data
    payload += b"CNAM" + struct.pack("<H", len(cnam_data)) + cnam_data
    payload += b"INTV" + struct.pack("<H", len(intv_data)) + intv_data
    tes4_header = b"TES4" + struct.pack("<I", len(payload)) + struct.pack("<I", 0x00000001)
    tes4_header += struct.pack("<I", 0)  # form_id
    tes4_header += struct.pack("<I", 0)  # vc1
    tes4_header += struct.pack("<H", 131)  # form_version
    tes4_header += struct.pack("<H", 0)  # version2
    parts.append(tes4_header + payload)

    # Empty GRUPs in given order — group_size = 24 (header only, no children)
    for sig in group_signatures:
        if len(sig) != 4:
            raise ValueError(f"signature must be 4 chars: {sig!r}")
        grup = b"GRUP"
        grup += struct.pack("<I", 24)  # size = header only
        grup += sig.encode("ascii")  # label
        grup += struct.pack("<i", 0)  # group_type = top-level
        grup += struct.pack("<H", 0)  # timestamp
        grup += struct.pack("<H", 0)  # vc2
        grup += struct.pack("<I", 0)  # unknown
        parts.append(grup)

    out_path.write_bytes(b"".join(parts))


def test_live_master_esm_order_overrides_hardcoded_baseline(tmp_path: Path) -> None:
    """Live extraction from the master ESM must take precedence over the
    hardcoded baseline. Build a synthetic master with a deliberately unusual
    group order and verify the resulting plugin uses *that* order, proving
    we actually read the master rather than falling through to hardcoded.
    """
    # Deliberately reverse alphabetical — opposite of both sorted-readdir
    # AND the canonical FO4 baseline.
    synthetic_order = ["MISC", "KYWD", "ACTI"]
    fake_master = tmp_path / "FakeFO4Master.esm"
    _build_synthetic_master_esm(fake_master, synthetic_order)

    authoring_dir = tmp_path / "live_order_authoring"
    plugin_name = "LiveOrderTest.esp"
    _scaffold_plugin_json(authoring_dir, plugin_name)
    # plugin.json uses empty masters[] by default; override to reference our master
    (authoring_dir / "plugin.json").write_text(
        json.dumps(
            {
                "plugin": plugin_name,
                "game": "fo4",
                "header": {
                    "version": 1.0,
                    "num_records": 0,
                    "next_object_id": "000810",
                    "author": "",
                    "description": "",
                    "masters": [fake_master.name],
                    "master_sizes": [0],
                    "overridden_forms": [],
                    "flags": "00000000",
                    "version_control": 0,
                    "extra_subrecords": [],
                },
            }
        ),
        encoding="utf-8",
    )

    _write_record_json(authoring_dir, "ACTI", "TestActi", "000800", plugin_name, "TestActi")
    _write_record_json(authoring_dir, "KYWD", "TestKywd", "000801", plugin_name, "TestKywd")
    _write_record_json(authoring_dir, "MISC", "TestMisc", "000802", plugin_name, "TestMisc")

    output_path = tmp_path / "LiveOrderTest.esp"
    esp_api._native_runtime.build_authoring_dir_streaming_native(
        str(authoring_dir),
        str(output_path),
        game="fo4",
        jobs=1,
        master_esm_paths=[str(fake_master)],
    )

    groups = _read_top_level_groups(output_path.read_bytes())
    # Our authoring dir has MISC, KYWD, ACTI. Synthetic master order:
    # MISC < KYWD < ACTI. So the build should emit them in that order,
    # NOT the hardcoded FO4 baseline (which would put KYWD before ACTI before MISC).
    assert groups == ["MISC", "KYWD", "ACTI"], (
        f"live master order should override hardcoded baseline; "
        f"expected ['MISC', 'KYWD', 'ACTI'], got {groups}. "
        f"(If this matches the FO4 baseline, the master ESM scan didn't run.)"
    )


def test_fo4_unknown_signatures_appended_after_canonical_order(tmp_path: Path) -> None:
    """Authoring dirs may contain mod-specific or future signatures not in
    the bundled canonical order list. Those must still build (placed after
    the canonical-order tail), not error or get silently dropped.
    """
    authoring_dir = tmp_path / "unknown_sig_authoring"
    output_path = tmp_path / "UnknownSig.esp"
    plugin_name = "UnknownSig.esp"
    _scaffold_plugin_json(authoring_dir, plugin_name)

    # KYWD is canonical (rank 1). XYZQ doesn't exist in any FO4 vanilla.
    # The build must place KYWD before XYZQ but emit both.
    _write_record_json(authoring_dir, "KYWD", "TestKywd", "000800", plugin_name, "TestKywd")
    _write_record_json(authoring_dir, "XYZQ", "TestXYZQ", "000801", plugin_name, "TestXYZQ")

    esp_api._native_runtime.build_authoring_dir_streaming_native(
        str(authoring_dir), str(output_path), game="fo4", jobs=1
    )

    groups = _read_top_level_groups(output_path.read_bytes())
    assert "KYWD" in groups, f"KYWD missing: {groups}"
    assert "XYZQ" in groups, f"XYZQ missing: {groups}"
    assert groups.index("KYWD") < groups.index("XYZQ")
