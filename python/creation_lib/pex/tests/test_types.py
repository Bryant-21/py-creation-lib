from creation_lib.pex.types import (
    PexFile, PexObject, PexState, PexFunction, PexInstruction,
    PexValue, PexVariable, PexProperty, PexParam, PexLocal,
    PexDebugInfo, PexDebugFunction, PexUserFlag, ValueType,
)
from creation_lib.pex.opcodes import PexOpcode


def test_pex_value_none():
    v = PexValue(ValueType.NONE, None)
    assert v.type == ValueType.NONE
    assert v.data is None


def test_pex_value_string():
    v = PexValue(ValueType.STRING, "hello")
    assert v.data == "hello"


def test_pex_instruction():
    instr = PexInstruction(
        opcode=PexOpcode.IADD,
        args=[
            PexValue(ValueType.IDENTIFIER, "result"),
            PexValue(ValueType.IDENTIFIER, "a"),
            PexValue(ValueType.IDENTIFIER, "b"),
        ],
    )
    assert instr.opcode == PexOpcode.IADD
    assert len(instr.args) == 3


def test_pex_function_minimal():
    fn = PexFunction(
        name="OnInit",
        return_type="None",
        docstring="",
        is_native=False,
        is_global=False,
        params=[],
        locals=[],
        instructions=[],
    )
    assert fn.name == "OnInit"
    assert fn.return_type == "None"


def test_pex_object():
    obj = PexObject(
        name="MyScript",
        parent="ObjectReference",
        docstring="",
        is_const=False,
        auto_state="",
        user_flags=0,
        variables=[],
        properties=[],
        states=[PexState(name="", functions=[])],
    )
    assert obj.name == "MyScript"
    assert obj.parent == "ObjectReference"
    assert len(obj.states) == 1


def test_pex_file():
    pf = PexFile(
        magic=0xFA57C0DE,
        major_version=3,
        minor_version=9,
        game_id=2,
        compilation_time=0,
        source_filename="test.psc",
        username="user",
        machine_name="machine",
        string_table=["MyScript", "None"],
        debug_info=None,
        user_flags=[],
        objects=[],
    )
    assert pf.magic == 0xFA57C0DE
    assert pf.game_id == 2
