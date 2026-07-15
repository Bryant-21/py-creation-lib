from creation_lib.pex.opcodes import PexOpcode


def test_opcode_values():
    """Verify opcode enum has correct numeric values."""
    assert PexOpcode.NOP == 0x00
    assert PexOpcode.IADD == 0x01
    assert PexOpcode.ASSIGN == 0x0D
    assert PexOpcode.JMP == 0x14
    assert PexOpcode.JMPT == 0x15
    assert PexOpcode.JMPF == 0x16
    assert PexOpcode.CALLMETHOD == 0x17
    assert PexOpcode.RETURN == 0x1A
    assert PexOpcode.PROPGET == 0x1C
    assert PexOpcode.PROPSET == 0x1D
    assert PexOpcode.ARRAY_GETELEMENT == 0x20
    assert PexOpcode.ARRAY_SETELEMENT == 0x21


def test_opcode_arg_count():
    """Verify argument count lookup."""
    from creation_lib.pex.opcodes import OPCODE_ARG_COUNTS
    assert OPCODE_ARG_COUNTS[PexOpcode.NOP] == 0
    assert OPCODE_ARG_COUNTS[PexOpcode.IADD] == 3
    assert OPCODE_ARG_COUNTS[PexOpcode.ASSIGN] == 2
    assert OPCODE_ARG_COUNTS[PexOpcode.JMP] == 1
    assert OPCODE_ARG_COUNTS[PexOpcode.JMPF] == 2
    # Vararg opcodes store the fixed arg count (not -1)
    assert OPCODE_ARG_COUNTS[PexOpcode.CALLMETHOD] == 3
    assert OPCODE_ARG_COUNTS[PexOpcode.CALLSTATIC] == 3
    assert OPCODE_ARG_COUNTS[PexOpcode.CALLPARENT] == 2
    from creation_lib.pex.opcodes import VARARG_OPCODES
    assert PexOpcode.CALLMETHOD in VARARG_OPCODES
    assert PexOpcode.CALLSTATIC in VARARG_OPCODES
