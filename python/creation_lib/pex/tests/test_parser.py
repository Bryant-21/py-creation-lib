import struct

import pytest

from creation_lib.pex.parser import parse_pex_bytes
from creation_lib.pex.tests.pex_samples import find_pex_files


def _build_header(
    magic=0xFA57C0DE, major=3, minor=9, game_id=1,
    comp_time=1700000000, source="test.psc", user="tester", machine="pc",
):
    """Build a minimal PEX header as bytes."""
    buf = struct.pack("<I", magic)           # magic
    buf += struct.pack("B", major)           # major version
    buf += struct.pack("B", minor)           # minor version
    buf += struct.pack("<H", game_id)        # game id
    buf += struct.pack("<Q", comp_time)      # compilation time
    # wstring: uint16 length + bytes
    for s in (source, user, machine):
        encoded = s.encode("utf-8")
        buf += struct.pack("<H", len(encoded)) + encoded
    return buf


def _build_string_table(*strings):
    """Build a string table section."""
    buf = struct.pack("<H", len(strings))
    for s in strings:
        encoded = s.encode("utf-8")
        buf += struct.pack("<H", len(encoded)) + encoded
    return buf


def _build_minimal_pex():
    """Build a complete minimal PEX with no objects."""
    buf = _build_header()
    buf += _build_string_table("MyScript", "None", "ObjectReference")
    buf += struct.pack("B", 0)   # no debug info
    buf += struct.pack("<H", 0)  # no user flags
    buf += struct.pack("<H", 0)  # no objects
    return buf


def test_parse_header():
    data = _build_minimal_pex()
    pex = parse_pex_bytes(data)
    assert pex.magic == 0xFA57C0DE
    assert pex.major_version == 3
    assert pex.minor_version == 9
    assert pex.game_id == 1
    assert pex.source_filename == "test.psc"
    assert pex.username == "tester"
    assert pex.machine_name == "pc"


def test_parse_string_table():
    data = _build_minimal_pex()
    pex = parse_pex_bytes(data)
    assert pex.string_table == ["MyScript", "None", "ObjectReference"]


def test_invalid_magic():
    data = _build_minimal_pex()
    bad = b"\x00\x00\x00\x00" + data[4:]
    with pytest.raises(ValueError, match="magic"):
        parse_pex_bytes(bad)


def _build_pex_with_object():
    """Build a PEX with one object containing a variable and an empty state."""
    strings = ["MyScript", "ObjectReference", "None", "", "myVar", "Int"]
    # str indices: 0=MyScript, 1=ObjectReference, 2=None, 3="", 4=myVar, 5=Int

    buf = _build_header()
    buf += _build_string_table(*strings)
    buf += struct.pack("B", 0)   # no debug info
    buf += struct.pack("<H", 0)  # no user flags
    buf += struct.pack("<H", 1)  # 1 object

    # Object: name, size (placeholder), parent, docstring, user_flags, auto_state
    obj_data = b""
    obj_data += struct.pack("<H", 0)  # name = "MyScript"
    # We'll insert size after building obj_data
    parent_and_rest = b""
    parent_and_rest += struct.pack("<H", 1)   # parent = "ObjectReference"
    parent_and_rest += struct.pack("<H", 3)   # docstring = ""
    parent_and_rest += struct.pack("<I", 0)   # user_flags
    parent_and_rest += struct.pack("<H", 3)   # auto_state = ""

    # 1 variable: myVar: Int = 0
    parent_and_rest += struct.pack("<H", 1)   # var count
    parent_and_rest += struct.pack("<H", 4)   # name = "myVar"
    parent_and_rest += struct.pack("<H", 5)   # type = "Int"
    parent_and_rest += struct.pack("<I", 0)   # user_flags
    parent_and_rest += struct.pack("B", 3)    # value type = INTEGER
    parent_and_rest += struct.pack("<i", 42)  # value = 42

    parent_and_rest += struct.pack("<H", 0)   # 0 properties
    # 1 state (default, empty)
    parent_and_rest += struct.pack("<H", 1)   # state count
    parent_and_rest += struct.pack("<H", 3)   # state name = ""
    parent_and_rest += struct.pack("<H", 0)   # 0 functions

    buf += obj_data
    buf += struct.pack("<I", len(parent_and_rest))  # object data size
    buf += parent_and_rest
    return buf


def test_parse_object_with_variable():
    data = _build_pex_with_object()
    pex = parse_pex_bytes(data)
    assert len(pex.objects) == 1
    obj = pex.objects[0]
    assert obj.name == "MyScript"
    assert obj.parent == "ObjectReference"
    assert len(obj.variables) == 1
    assert obj.variables[0].name == "myVar"
    assert obj.variables[0].type == "Int"
    assert obj.variables[0].data.data == 42


def _build_pex_with_function():
    """Build a PEX with one object containing a function: result = a + b."""
    strings = ["MyScript", "None", "", "Add", "Int", "a", "b", "result"]
    # 0=MyScript 1=None 2="" 3=Add 4=Int 5=a 6=b 7=result

    buf = _build_header()
    buf += _build_string_table(*strings)
    buf += struct.pack("B", 0)   # no debug info
    buf += struct.pack("<H", 0)  # no user flags
    buf += struct.pack("<H", 1)  # 1 object

    # Object header
    buf += struct.pack("<H", 0)  # name = "MyScript"

    obj_body = b""
    obj_body += struct.pack("<H", 2)  # parent = "" (idx 2, empty = no parent effectively)
    obj_body += struct.pack("<H", 2)  # docstring = ""
    obj_body += struct.pack("<I", 0)  # user_flags
    obj_body += struct.pack("<H", 2)  # auto_state = ""
    obj_body += struct.pack("<H", 0)  # 0 variables
    obj_body += struct.pack("<H", 0)  # 0 properties

    # 1 state with 1 function
    obj_body += struct.pack("<H", 1)  # state count
    obj_body += struct.pack("<H", 2)  # state name = ""
    obj_body += struct.pack("<H", 1)  # 1 function
    obj_body += struct.pack("<H", 3)  # function name = "Add"

    # Function body
    obj_body += struct.pack("<H", 4)  # return type = "Int"
    obj_body += struct.pack("<H", 2)  # docstring = ""
    obj_body += struct.pack("<I", 0)  # user_flags
    obj_body += struct.pack("B", 0)   # flags (not native, not global)

    # 2 params: a: Int, b: Int
    obj_body += struct.pack("<H", 2)
    obj_body += struct.pack("<H", 5) + struct.pack("<H", 4)  # a: Int
    obj_body += struct.pack("<H", 6) + struct.pack("<H", 4)  # b: Int

    # 1 local: result: Int
    obj_body += struct.pack("<H", 1)
    obj_body += struct.pack("<H", 7) + struct.pack("<H", 4)  # result: Int

    # 2 instructions: IADD result a b; RETURN result
    obj_body += struct.pack("<H", 2)  # instruction count
    # IADD (0x01): dest=result(id), left=a(id), right=b(id)
    obj_body += struct.pack("B", 0x01)  # IADD
    obj_body += struct.pack("B", 1) + struct.pack("<H", 7)  # identifier "result"
    obj_body += struct.pack("B", 1) + struct.pack("<H", 5)  # identifier "a"
    obj_body += struct.pack("B", 1) + struct.pack("<H", 6)  # identifier "b"
    # RETURN (0x1A): value=result(id)
    obj_body += struct.pack("B", 0x1A)  # RETURN
    obj_body += struct.pack("B", 1) + struct.pack("<H", 7)  # identifier "result"

    buf += struct.pack("<I", len(obj_body))
    buf += obj_body
    return buf


def test_parse_function_with_instructions():
    data = _build_pex_with_function()
    pex = parse_pex_bytes(data)
    obj = pex.objects[0]
    assert len(obj.states) == 1
    state = obj.states[0]
    assert len(state.functions) == 1
    fn = state.functions[0]
    assert fn.name == "Add"
    assert fn.return_type == "Int"
    assert len(fn.params) == 2
    assert fn.params[0].name == "a"
    assert fn.params[1].name == "b"
    assert len(fn.locals) == 1
    assert fn.locals[0].name == "result"
    assert len(fn.instructions) == 2
    # IADD result a b
    from creation_lib.pex.opcodes import PexOpcode
    assert fn.instructions[0].opcode == PexOpcode.IADD
    assert fn.instructions[0].args[0].data == "result"
    assert fn.instructions[0].args[1].data == "a"
    assert fn.instructions[0].args[2].data == "b"
    # RETURN result
    assert fn.instructions[1].opcode == PexOpcode.RETURN
    assert fn.instructions[1].args[0].data == "result"


def test_parse_real_pex():
    """Smoke test: parse a small sample of available .pex files without crashing."""
    pex_files = find_pex_files(limit=10)
    if not pex_files:
        pytest.skip("No .pex files found")
    failures = []
    for pex_path in pex_files:
        try:
            pex = parse_pex_bytes(pex_path.read_bytes())
            assert pex.magic == 0xFA57C0DE
            assert len(pex.objects) >= 1
            assert pex.objects[0].name
        except Exception as e:
            failures.append(f"{pex_path.name}: {e}")
    if failures:
        pytest.fail(f"Failed to parse {len(failures)}/{len(pex_files)} files:\n" +
                    "\n".join(failures[:10]))


def test_parse_pex_bytes_uses_native_runtime(monkeypatch):
    from creation_lib.pex import parser as pex_parser
    from creation_lib.pex.types import PexFile

    expected = PexFile(
        magic=0xFA57C0DE,
        major_version=3,
        minor_version=9,
        game_id=2,
        compilation_time=1,
        source_filename="native.psc",
        username="u",
        machine_name="m",
    )

    monkeypatch.setattr(pex_parser.native_runtime, "parse_pex_bytes_native", lambda data: expected)

    assert pex_parser.parse_pex_bytes(b"native-bytes") is expected


def test_parse_pex_uses_native_file_runtime(monkeypatch, tmp_path):
    from creation_lib.pex import parser as pex_parser
    from creation_lib.pex.types import PexFile

    expected = PexFile(
        magic=0xFA57C0DE,
        major_version=3,
        minor_version=9,
        game_id=2,
        compilation_time=1,
        source_filename="native.psc",
        username="u",
        machine_name="m",
    )
    pex_path = tmp_path / "native.pex"
    pex_path.write_bytes(b"native-bytes")

    monkeypatch.setattr(pex_parser.native_runtime, "parse_pex_file_native", lambda path: expected)

    assert pex_parser.parse_pex(pex_path) is expected
