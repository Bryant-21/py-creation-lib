"""PEX bytecode opcode definitions."""
from enum import IntEnum


class PexOpcode(IntEnum):
    NOP = 0x00
    IADD = 0x01
    FADD = 0x02
    ISUB = 0x03
    FSUB = 0x04
    IMUL = 0x05
    FMUL = 0x06
    IDIV = 0x07
    FDIV = 0x08
    IMOD = 0x09
    NOT = 0x0A
    INEG = 0x0B
    FNEG = 0x0C
    ASSIGN = 0x0D
    CAST = 0x0E
    CMP_EQ = 0x0F
    CMP_LT = 0x10
    CMP_LTE = 0x11
    CMP_GT = 0x12
    CMP_GTE = 0x13
    JMP = 0x14
    JMPT = 0x15
    JMPF = 0x16
    CALLMETHOD = 0x17
    CALLPARENT = 0x18
    CALLSTATIC = 0x19
    RETURN = 0x1A
    STRCAT = 0x1B
    PROPGET = 0x1C
    PROPSET = 0x1D
    ARRAY_CREATE = 0x1E
    ARRAY_LENGTH = 0x1F
    ARRAY_GETELEMENT = 0x20
    ARRAY_SETELEMENT = 0x21
    ARRAY_FINDELEMENT = 0x22
    ARRAY_RFINDELEMENT = 0x23
    # FO4+ extended opcodes
    IS = 0x24
    STRUCT_CREATE = 0x25
    STRUCT_GET = 0x26
    STRUCT_SET = 0x27
    ARRAY_FINDSTRUCT = 0x28
    ARRAY_RFINDSTRUCT = 0x29
    ARRAY_ADD = 0x2A
    ARRAY_INSERT = 0x2B
    ARRAY_REMOVELAST = 0x2C
    ARRAY_REMOVE = 0x2D
    ARRAY_CLEAR = 0x2E
    # FO76+ extended opcodes
    ARRAY_GETALLMATCHINGSTRUCTS = 0x2F
    # Starfield+ extended opcodes
    LOCK_GUARDS = 0x30
    UNLOCK_GUARDS = 0x31
    TRY_LOCK_GUARDS = 0x32


# Number of fixed arguments per opcode.
OPCODE_ARG_COUNTS: dict[PexOpcode, int] = {
    PexOpcode.NOP: 0,
    PexOpcode.IADD: 3,
    PexOpcode.FADD: 3,
    PexOpcode.ISUB: 3,
    PexOpcode.FSUB: 3,
    PexOpcode.IMUL: 3,
    PexOpcode.FMUL: 3,
    PexOpcode.IDIV: 3,
    PexOpcode.FDIV: 3,
    PexOpcode.IMOD: 3,
    PexOpcode.NOT: 2,
    PexOpcode.INEG: 2,
    PexOpcode.FNEG: 2,
    PexOpcode.ASSIGN: 2,
    PexOpcode.CAST: 2,
    PexOpcode.CMP_EQ: 3,
    PexOpcode.CMP_LT: 3,
    PexOpcode.CMP_LTE: 3,
    PexOpcode.CMP_GT: 3,
    PexOpcode.CMP_GTE: 3,
    PexOpcode.JMP: 1,
    PexOpcode.JMPT: 2,
    PexOpcode.JMPF: 2,
    PexOpcode.CALLMETHOD: 3,
    PexOpcode.CALLPARENT: 2,
    PexOpcode.CALLSTATIC: 3,
    PexOpcode.RETURN: 1,
    PexOpcode.STRCAT: 3,
    PexOpcode.PROPGET: 3,
    PexOpcode.PROPSET: 3,
    PexOpcode.ARRAY_CREATE: 2,
    PexOpcode.ARRAY_LENGTH: 2,
    PexOpcode.ARRAY_GETELEMENT: 3,
    PexOpcode.ARRAY_SETELEMENT: 3,
    PexOpcode.ARRAY_FINDELEMENT: 4,
    PexOpcode.ARRAY_RFINDELEMENT: 4,
    PexOpcode.IS: 3,
    PexOpcode.STRUCT_CREATE: 1,
    PexOpcode.STRUCT_GET: 3,
    PexOpcode.STRUCT_SET: 3,
    PexOpcode.ARRAY_FINDSTRUCT: 5,
    PexOpcode.ARRAY_RFINDSTRUCT: 5,
    PexOpcode.ARRAY_ADD: 3,
    PexOpcode.ARRAY_INSERT: 3,
    PexOpcode.ARRAY_REMOVELAST: 1,
    PexOpcode.ARRAY_REMOVE: 3,
    PexOpcode.ARRAY_CLEAR: 1,
    PexOpcode.ARRAY_GETALLMATCHINGSTRUCTS: 6,
    PexOpcode.LOCK_GUARDS: 0,
    PexOpcode.UNLOCK_GUARDS: 0,
    PexOpcode.TRY_LOCK_GUARDS: 1,
}

# Opcodes that have variable arguments after the fixed args.
# Format: read fixed args, then an integer count value, then count more values.
VARARG_OPCODES: frozenset[PexOpcode] = frozenset({
    PexOpcode.CALLMETHOD,
    PexOpcode.CALLPARENT,
    PexOpcode.CALLSTATIC,
    PexOpcode.LOCK_GUARDS,
    PexOpcode.UNLOCK_GUARDS,
    PexOpcode.TRY_LOCK_GUARDS,
})
