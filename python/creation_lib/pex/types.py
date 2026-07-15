"""Typed dataclasses for PEX file structures."""
from __future__ import annotations
from dataclasses import dataclass, field
from enum import IntEnum

from creation_lib.pex.opcodes import PexOpcode


class ValueType(IntEnum):
    NONE = 0
    IDENTIFIER = 1
    STRING = 2
    INTEGER = 3
    FLOAT = 4
    BOOL = 5


@dataclass
class PexValue:
    type: ValueType
    data: object  # str for IDENTIFIER/STRING, int, float, bool, or None

    @staticmethod
    def none() -> PexValue:
        return PexValue(ValueType.NONE, None)

    @staticmethod
    def identifier(name: str) -> PexValue:
        return PexValue(ValueType.IDENTIFIER, name)

    @staticmethod
    def string(s: str) -> PexValue:
        return PexValue(ValueType.STRING, s)

    @staticmethod
    def integer(i: int) -> PexValue:
        return PexValue(ValueType.INTEGER, i)

    @staticmethod
    def floating(f: float) -> PexValue:
        return PexValue(ValueType.FLOAT, f)

    @staticmethod
    def boolean(b: bool) -> PexValue:
        return PexValue(ValueType.BOOL, b)


@dataclass
class PexInstruction:
    opcode: PexOpcode
    args: list[PexValue] = field(default_factory=list)


@dataclass
class PexParam:
    name: str
    type: str


@dataclass
class PexLocal:
    name: str
    type: str


@dataclass
class PexFunction:
    name: str
    return_type: str
    docstring: str
    is_native: bool
    is_global: bool
    user_flags: int = 0
    params: list[PexParam] = field(default_factory=list)
    locals: list[PexLocal] = field(default_factory=list)
    instructions: list[PexInstruction] = field(default_factory=list)


@dataclass
class PexStructMember:
    name: str
    type: str
    user_flags: int = 0
    data: PexValue = field(default_factory=PexValue.none)
    is_const: bool = False
    docstring: str = ""


@dataclass
class PexStruct:
    name: str
    members: list[PexStructMember] = field(default_factory=list)


@dataclass
class PexVariable:
    name: str
    type: str
    user_flags: int = 0
    data: PexValue = field(default_factory=PexValue.none)
    is_const: bool = False


@dataclass
class PexProperty:
    name: str
    type: str
    docstring: str = ""
    user_flags: int = 0
    flags: int = 0  # bitfield: 1=read, 2=write, 4=autovar
    auto_var: str = ""
    getter: PexFunction | None = None
    setter: PexFunction | None = None


@dataclass
class PexState:
    name: str  # empty string = default/unnamed state
    functions: list[PexFunction] = field(default_factory=list)


@dataclass
class PexObject:
    name: str
    parent: str  # empty string if no parent
    docstring: str = ""
    is_const: bool = False
    auto_state: str = ""
    user_flags: int = 0
    structs: list[PexStruct] = field(default_factory=list)
    variables: list[PexVariable] = field(default_factory=list)
    guards: list[str] = field(default_factory=list)
    properties: list[PexProperty] = field(default_factory=list)
    states: list[PexState] = field(default_factory=list)


@dataclass
class PexDebugPropertyGroup:
    object_name: str
    group_name: str
    docstring: str = ""
    user_flags: int = 0
    property_names: list[str] = field(default_factory=list)


@dataclass
class PexDebugStructOrder:
    object_name: str
    struct_name: str
    member_names: list[str] = field(default_factory=list)


@dataclass
class PexDebugFunction:
    object_name: str
    state_name: str
    function_name: str
    function_type: int
    line_numbers: list[int] = field(default_factory=list)


@dataclass
class PexDebugInfo:
    modification_time: int = 0
    functions: list[PexDebugFunction] = field(default_factory=list)
    property_groups: list[PexDebugPropertyGroup] = field(default_factory=list)
    struct_orders: list[PexDebugStructOrder] = field(default_factory=list)


@dataclass
class PexUserFlag:
    name: str
    index: int


@dataclass
class PexFile:
    magic: int
    major_version: int
    minor_version: int
    game_id: int
    compilation_time: int
    source_filename: str
    username: str
    machine_name: str
    string_table: list[str] = field(default_factory=list)
    debug_info: PexDebugInfo | None = None
    user_flags: list[PexUserFlag] = field(default_factory=list)
    objects: list[PexObject] = field(default_factory=list)
