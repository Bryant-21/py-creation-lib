from __future__ import annotations

import json
import struct

import pytest

from creation_lib.papyrus_lsp import native_runtime as papyrus_native


def _minimal_pex() -> bytes:
    buf = struct.pack("<I", 0xFA57C0DE)
    buf += struct.pack("B", 3)
    buf += struct.pack("B", 9)
    buf += struct.pack("<H", 2)
    buf += struct.pack("<Q", 1700000000)
    for value in ("test.psc", "tester", "pc"):
        raw = value.encode("utf-8")
        buf += struct.pack("<H", len(raw)) + raw
    strings = ["MyScript", "None", "ObjectReference"]
    buf += struct.pack("<H", len(strings))
    for value in strings:
        raw = value.encode("utf-8")
        buf += struct.pack("<H", len(raw)) + raw
    buf += struct.pack("B", 0)
    buf += struct.pack("<H", 0)
    buf += struct.pack("<H", 0)
    return buf


@pytest.fixture(scope="module")
def native_module():
    if not papyrus_native.is_available():
        pytest.skip("papyrus_core native module not installed")
    module = papyrus_native.load_native_module()
    if not hasattr(module, "parse_pex_bytes"):
        pytest.skip("papyrus_core native module missing parse_pex_bytes")
    return module


def test_native_parse_pex_bytes_returns_json_payload(native_module):
    payload = json.loads(native_module.parse_pex_bytes(_minimal_pex()))

    assert payload["magic"] == 0xFA57C0DE
    assert payload["major_version"] == 3
    assert payload["minor_version"] == 9
    assert payload["game_id"] == 2
    assert payload["source_filename"] == "test.psc"
    assert payload["string_table"] == ["MyScript", "None", "ObjectReference"]
    assert payload["objects"] == []


def test_native_parse_pex_file_returns_json_payload(native_module, tmp_path):
    pex_path = tmp_path / "minimal.pex"
    pex_path.write_bytes(_minimal_pex())

    payload = json.loads(native_module.parse_pex_file(str(pex_path)))

    assert payload["source_filename"] == "test.psc"
    assert payload["objects"] == []


def test_native_parse_pex_bytes_rejects_bad_magic(native_module):
    with pytest.raises(ValueError, match="Invalid PEX magic"):
        native_module.parse_pex_bytes(b"\x00\x00\x00\x00")
