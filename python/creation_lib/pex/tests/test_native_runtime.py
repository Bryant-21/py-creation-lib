from __future__ import annotations

import json

from creation_lib.pex.opcodes import PexOpcode
from creation_lib.pex.types import PexFile, ValueType


def _minimal_payload() -> dict:
    return {
        "magic": 0xFA57C0DE,
        "major_version": 3,
        "minor_version": 9,
        "game_id": 2,
        "compilation_time": 1700000000,
        "source_filename": "test.psc",
        "username": "tester",
        "machine_name": "pc",
        "string_table": ["MyScript", "ObjectReference", "", "DoIt", "None", "x", "Int"],
        "debug_info": None,
        "user_flags": [{"name": "Hidden", "index": 0}],
        "objects": [
            {
                "name": "MyScript",
                "parent": "ObjectReference",
                "docstring": "",
                "is_const": False,
                "auto_state": "",
                "user_flags": 0,
                "variables": [
                    {"name": "x", "type": "Int", "user_flags": 0, "data": {"type": 3, "data": 42}},
                ],
                "properties": [],
                "states": [
                    {
                        "name": "",
                        "functions": [
                            {
                                "name": "DoIt",
                                "return_type": "None",
                                "docstring": "",
                                "is_native": False,
                                "is_global": False,
                                "params": [],
                                "locals": [],
                                "instructions": [
                                    {"opcode": 26, "args": [{"type": 0, "data": None}]},
                                ],
                            },
                        ],
                    },
                ],
            },
        ],
    }


def test_pex_file_from_payload_rebuilds_existing_dataclasses():
    from creation_lib.pex.native_runtime import pex_file_from_payload

    result = pex_file_from_payload(_minimal_payload())

    assert isinstance(result, PexFile)
    assert result.magic == 0xFA57C0DE
    assert result.objects[0].name == "MyScript"
    assert result.objects[0].variables[0].data.type == ValueType.INTEGER
    assert result.objects[0].variables[0].data.data == 42
    fn = result.objects[0].states[0].functions[0]
    assert fn.name == "DoIt"
    assert fn.instructions[0].opcode == PexOpcode.RETURN
    assert fn.instructions[0].args[0].type == ValueType.NONE


def test_pex_file_from_payload_normalizes_numeric_bool_values():
    from creation_lib.pex.native_runtime import pex_file_from_payload

    payload = _minimal_payload()
    payload["objects"][0]["variables"].extend(
        [
            {"name": "truthy", "type": "Bool", "user_flags": 0, "data": {"type": 5, "data": 1}},
            {"name": "falsy", "type": "Bool", "user_flags": 0, "data": {"type": 5, "data": 0}},
        ]
    )

    result = pex_file_from_payload(payload)

    truthy = result.objects[0].variables[1].data
    falsy = result.objects[0].variables[2].data
    assert truthy.type == ValueType.BOOL
    assert truthy.data is True
    assert falsy.type == ValueType.BOOL
    assert falsy.data is False


def test_parse_pex_bytes_native_uses_loaded_native_module(monkeypatch):
    import creation_lib.pex.native_runtime as runtime

    class FakeNative:
        def parse_pex_bytes(self, data: bytes) -> str:
            assert data == b"pex-bytes"
            return json.dumps(_minimal_payload())

    monkeypatch.setattr(runtime, "load_native_module", lambda: FakeNative())

    result = runtime.parse_pex_bytes_native(b"pex-bytes")

    assert result.source_filename == "test.psc"
    assert result.objects[0].states[0].functions[0].name == "DoIt"


def test_parse_pex_file_native_uses_native_file_parser_when_available(monkeypatch, tmp_path):
    import creation_lib.pex.native_runtime as runtime

    pex_path = tmp_path / "test.pex"
    pex_path.write_bytes(b"pex-bytes")

    class FakeNative:
        def parse_pex_bytes(self, data: bytes) -> str:
            raise AssertionError("parse_pex_bytes should not be used when parse_pex_file is available")

        def parse_pex_file(self, path: str) -> str:
            assert path == str(pex_path)
            return json.dumps(_minimal_payload())

    monkeypatch.setattr(runtime, "load_native_module", lambda: FakeNative())

    result = runtime.parse_pex_file_native(pex_path)

    assert result.source_filename == "test.psc"


def test_load_native_module_requires_file_parser(monkeypatch):
    import creation_lib.pex.native_runtime as runtime

    class FakeNative:
        def parse_pex_bytes(self, data: bytes) -> str:
            return json.dumps(_minimal_payload())

    monkeypatch.setattr(runtime.importlib, "import_module", lambda name: FakeNative())

    assert runtime.load_native_module() is None


def test_parse_pex_bytes_native_raises_when_native_missing(monkeypatch):
    import pytest
    import creation_lib.pex.native_runtime as runtime

    monkeypatch.setattr(runtime, "load_native_module", lambda: None)

    with pytest.raises(RuntimeError, match="papyrus_core native PEX parser is not available"):
        runtime.parse_pex_bytes_native(b"pex-bytes")
