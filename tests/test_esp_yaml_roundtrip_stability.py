"""YAML/JSON authoring-dir round-trip stability gate.

Detects YAML-codec asymmetries that the binary-only round-trip test
(`test_esp_official_byte_exact.py`) cannot catch. The binary-only test
does ESM → in-memory → ESM and never exercises authoring-dir order or
the YAML inspect↔build path.

Failures here indicate that one of:
  * inspect-mod loses information that build can't reconstruct
  * build emits subrecords / groups in a different order than inspect
  * a codec round-trip is lossy (e.g. empty MODT, fragment tail, INTV bytes)
  * the streaming build path diverges from in-memory save

Strategy: construct a plugin with known content, write it through the
authoring-dir path, re-build, and assert byte stability across rounds.
This is the fast synthetic version (deterministic, no game data needed).
A vanilla-ESM round-trip is left as an on-demand check (vanilla data
exposes accumulated codec bugs that fail loudly — useful for triage but
not a CI gate).
"""

from __future__ import annotations

import json
import struct
from pathlib import Path

import pytest

from creation_lib.esp import Plugin, build_authoring_dir, export_authoring_dir
from creation_lib.esp.native_runtime import load_native_module
import creation_lib.esp.api as esp_api


pytestmark = pytest.mark.skipif(
    load_native_module() is None,
    reason="esp_authoring_core is not installed",
)


def _walk_top_level_groups(esp_bytes: bytes) -> list[tuple[str, int]]:
    """Return [(label, size), ...] for top-level type-0 GRUPs."""
    tes4_data_size = struct.unpack_from("<I", esp_bytes, 4)[0]
    offset = 24 + tes4_data_size
    groups: list[tuple[str, int]] = []
    while offset + 24 <= len(esp_bytes):
        if esp_bytes[offset : offset + 4] == b"GRUP":
            grup_size = struct.unpack_from("<I", esp_bytes, offset + 4)[0]
            label = esp_bytes[offset + 8 : offset + 12]
            group_type = struct.unpack_from("<i", esp_bytes, offset + 12)[0]
            if group_type == 0:
                try:
                    groups.append((label.decode("ascii"), grup_size))
                except UnicodeDecodeError:
                    pass
            offset += grup_size
        else:
            offset += 1
    return groups


def _record_form_ids(esp_bytes: bytes, target_sig: bytes) -> list[int]:
    """Return all FormIDs of records with the given signature (for assertions)."""
    out: list[int] = []
    i = 0
    while i + 24 <= len(esp_bytes):
        sig = esp_bytes[i : i + 4]
        if sig == b"GRUP":
            i += 24
            continue
        if sig == b"TES4":
            data_size = struct.unpack_from("<I", esp_bytes, 4)[0]
            i += 24 + data_size
            continue
        if sig == target_sig:
            data_size = struct.unpack_from("<I", esp_bytes, i + 4)[0]
            if 0 <= data_size < 100_000:
                form_id = struct.unpack_from("<I", esp_bytes, i + 12)[0]
                out.append(form_id)
                i += 24 + data_size
                continue
        i += 1
    return out


def _scaffold_authoring_dir(
    authoring_dir: Path,
    plugin_name: str = "RoundTrip.esp",
    *,
    masters: list[str] | None = None,
) -> None:
    masters = masters or []
    authoring_dir.mkdir(parents=True, exist_ok=True)
    (authoring_dir / "plugin.json").write_text(
        json.dumps(
            {
                "plugin": plugin_name,
                "game": "fo4",
                "header": {
                    "version": 1.0,
                    "num_records": 0,
                    "next_object_id": "000900",
                    "author": "rt-test",
                    "description": "",
                    "masters": masters,
                    "master_sizes": [0] * len(masters),
                    "overridden_forms": [],
                    "flags": "00000000",
                    "version_control": 0,
                    "extra_subrecords": [],
                },
            }
        ),
        encoding="utf-8",
    )


def _write_record(
    authoring_dir: Path,
    signature: str,
    file_stem: str,
    form_id: str,
    plugin_name: str,
    subrecords: list[dict],
) -> None:
    sig_dir = authoring_dir / "records" / signature
    sig_dir.mkdir(parents=True, exist_ok=True)
    (sig_dir / f"{file_stem}.json").write_text(
        json.dumps(
            {
                "form_id": f"{form_id}:{plugin_name}",
                "subrecords": subrecords,
            }
        ),
        encoding="utf-8",
    )


def _build_streaming(authoring_dir: Path, output_path: Path) -> None:
    esp_api._native_runtime.build_authoring_dir_streaming_native(
        str(authoring_dir),
        str(output_path),
        game="fo4",
        jobs=1,
    )


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


def test_authoring_dir_double_build_is_byte_stable(tmp_path: Path) -> None:
    """Building the same authoring dir twice produces byte-identical ESPs.
    A non-deterministic build (random subrecord ordering, non-stable hash
    iteration, etc.) would fail this.
    """
    authoring_dir = tmp_path / "stable_authoring"
    plugin_name = "Stable.esp"
    _scaffold_authoring_dir(authoring_dir, plugin_name)

    # Mix several record types so the multi-group path is exercised
    edid_hex = lambda s: s.encode("ascii").hex() + "00"  # noqa: E731
    _write_record(
        authoring_dir, "KYWD", "Kw1", "000800", plugin_name,
        [{"signature": "EDID", "data_hex": edid_hex("Kw1")},
         {"signature": "CNAM", "data_hex": "ffffffff"},
         {"signature": "TNAM", "data_hex": "09000000"}],
    )
    _write_record(
        authoring_dir, "KYWD", "Kw2", "000801", plugin_name,
        [{"signature": "EDID", "data_hex": edid_hex("Kw2")},
         {"signature": "CNAM", "data_hex": "00ff00ff"},
         {"signature": "TNAM", "data_hex": "00000000"}],
    )
    _write_record(
        authoring_dir, "MISC", "Misc1", "000810", plugin_name,
        [{"signature": "EDID", "data_hex": edid_hex("Misc1")}],
    )
    _write_record(
        authoring_dir, "ACTI", "Acti1", "000820", plugin_name,
        [{"signature": "EDID", "data_hex": edid_hex("Acti1")}],
    )

    out_a = tmp_path / "build_a.esp"
    out_b = tmp_path / "build_b.esp"
    _build_streaming(authoring_dir, out_a)
    _build_streaming(authoring_dir, out_b)

    assert out_a.read_bytes() == out_b.read_bytes(), (
        "double build is not byte-stable — codec output depends on "
        "non-deterministic state (HashMap iteration, file timestamps, ...)"
    )


def test_authoring_build_then_inspect_then_build_is_byte_stable(tmp_path: Path) -> None:
    """build → load → re-export → build produces the same final ESP.

    This catches codec asymmetries: anything inspect-mod produces in YAML
    that build emits as different bytes than the original, OR anything
    build emits that inspect-mod can't reproduce on a re-export.

    The first round may differ from the second (because empty subrecords
    or implicit default fields can normalize), but subsequent rounds must
    be byte-stable. A non-fixed-point round-trip means the codec hasn't
    converged — every save would produce drift.
    """
    src_authoring = tmp_path / "src_authoring"
    plugin_name = "RT.esp"
    _scaffold_authoring_dir(src_authoring, plugin_name)
    edid_hex = lambda s: s.encode("ascii").hex() + "00"  # noqa: E731

    _write_record(
        src_authoring, "KYWD", "RtKw", "000800", plugin_name,
        [{"signature": "EDID", "data_hex": edid_hex("RtKw")},
         {"signature": "CNAM", "data_hex": "abcdef01"},
         {"signature": "TNAM", "data_hex": "09000000"}],
    )
    _write_record(
        src_authoring, "MISC", "RtMisc", "000810", plugin_name,
        [{"signature": "EDID", "data_hex": edid_hex("RtMisc")},
         {"signature": "FULL", "data_hex": "526f756e6454726970004d6973630000".replace("00", "")[:14] + "00"}],
    )

    esp_round_1 = tmp_path / "rt1.esp"
    _build_streaming(src_authoring, esp_round_1)

    plugin = Plugin.load(str(esp_round_1), game="fo4", backend="native")
    yaml_round_1 = tmp_path / "yaml_round_1"
    yaml_round_1.mkdir()
    export_authoring_dir(plugin, yaml_round_1, format="json", backend="native")

    esp_round_2 = tmp_path / "rt2.esp"
    build_authoring_dir(yaml_round_1, esp_round_2, game="fo4", jobs=1)

    plugin2 = Plugin.load(str(esp_round_2), game="fo4", backend="native")
    yaml_round_2 = tmp_path / "yaml_round_2"
    yaml_round_2.mkdir()
    export_authoring_dir(plugin2, yaml_round_2, format="json", backend="native")

    esp_round_3 = tmp_path / "rt3.esp"
    build_authoring_dir(yaml_round_2, esp_round_3, game="fo4", jobs=1)

    # Round 2 → Round 3 must be byte-stable (the codec has converged).
    # Round 1 → Round 2 may differ because of normalization (e.g. CK-injected
    # defaults that inspect-mod surfaces but our scaffold didn't write).
    assert esp_round_2.read_bytes() == esp_round_3.read_bytes(), (
        "build→inspect→build is not a fixed point — every save drifts. "
        "Indicates a codec asymmetry: inspect-mod produces a YAML that "
        "build encodes as different bytes than were inspected."
    )


def test_authoring_build_preserves_keyword_resolution_chain(tmp_path: Path) -> None:
    """A KYWD referenced by another record's KSIZ/KWDA must:
      1. exist in the built binary at the form_id KSIZ/KWDA points at
      2. appear in the file *before* the referencing record (canonical order)

    This is the actual invariant CK requires — failure produces
    `[FORMS] Unable to find keyword`. We assert both halves: bytes
    correct AND order correct.
    """
    authoring_dir = tmp_path / "kwref_authoring"
    plugin_name = "KwRef.esp"
    _scaffold_authoring_dir(authoring_dir, plugin_name)
    edid_hex = lambda s: s.encode("ascii").hex() + "00"  # noqa: E731

    # Keyword at 000800; MISC at 000810 references it via KSIZ+KWDA
    _write_record(
        authoring_dir, "KYWD", "ChainKw", "000800", plugin_name,
        [{"signature": "EDID", "data_hex": edid_hex("ChainKw")},
         {"signature": "CNAM", "data_hex": "00112233"},
         {"signature": "TNAM", "data_hex": "00000000"}],
    )
    # 0 masters → own_index=0, so the plugin's own records get form_id
    # 0x00xxxxxx. Author KWDA pointing to KYWD at 0x00000800.
    _write_record(
        authoring_dir, "MISC", "ChainMisc", "000810", plugin_name,
        [{"signature": "EDID", "data_hex": edid_hex("ChainMisc")},
         {"signature": "KSIZ", "data_hex": "01000000"},
         {"signature": "KWDA", "data_hex": "00080000"}],
    )

    output = tmp_path / "KwRef.esp"
    _build_streaming(authoring_dir, output)

    binary = output.read_bytes()
    groups = _walk_top_level_groups(binary)
    group_labels = [label for label, _ in groups]
    assert "KYWD" in group_labels and "MISC" in group_labels
    # KYWD must come first — otherwise CK can't resolve KWDA references on load
    assert group_labels.index("KYWD") < group_labels.index("MISC"), (
        f"KYWD must precede MISC for keyword resolution, got: {group_labels}"
    )

    kywd_form_ids = _record_form_ids(binary, b"KYWD")
    # 0 masters → own_index=0, keyword stored at form_id 0x00000800
    assert 0x00000800 in kywd_form_ids, (
        f"keyword at form_id 0x00000800 missing from output, got: "
        f"{[f'0x{f:08X}' for f in kywd_form_ids]}"
    )
