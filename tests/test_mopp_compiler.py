"""Tests for MOPP bytecode compiler."""
import math

import pytest


def test_compile_mopp_cube():
    """A simple cube (8 verts, 12 triangles) should produce valid MOPP bytecode."""
    from creation_lib.nif.operations.mopp_compiler import compile_mopp

    # Unit cube centered at origin, in Havok space
    verts = [
        (-0.5, -0.5, -0.5), (0.5, -0.5, -0.5),
        (0.5, 0.5, -0.5), (-0.5, 0.5, -0.5),
        (-0.5, -0.5, 0.5), (0.5, -0.5, 0.5),
        (0.5, 0.5, 0.5), (-0.5, 0.5, 0.5),
    ]
    triangles = [
        (0, 1, 2), (0, 2, 3),  # front
        (4, 6, 5), (4, 7, 6),  # back
        (0, 4, 5), (0, 5, 1),  # bottom
        (2, 6, 7), (2, 7, 3),  # top
        (0, 3, 7), (0, 7, 4),  # left
        (1, 5, 6), (1, 6, 2),  # right
    ]
    mopp_bytes, origin, scale = compile_mopp(verts, triangles, radius=0.005)

    assert isinstance(mopp_bytes, bytes)
    assert len(mopp_bytes) > 0
    # Origin should be min(verts) - radius on each axis
    assert origin[0] == pytest.approx(-0.505, abs=0.01)
    assert origin[1] == pytest.approx(-0.505, abs=0.01)
    assert origin[2] == pytest.approx(-0.505, abs=0.01)
    # Scale should be positive
    assert scale > 0
    # First 9 bytes should be 3 FILTER instructions (opcodes 0x26, 0x27, 0x28)
    assert mopp_bytes[0] == 0x26  # FILTER X
    assert mopp_bytes[3] == 0x27  # FILTER Y
    assert mopp_bytes[6] == 0x28  # FILTER Z


def test_compile_mopp_empty():
    """Empty triangle list returns empty bytes."""
    from creation_lib.nif.operations.mopp_compiler import compile_mopp

    mopp_bytes, origin, scale = compile_mopp([], [], radius=0.005)
    assert mopp_bytes == b""
    assert scale == 0.0


def test_compile_mopp_single_triangle():
    """Single triangle should produce FILTER + LEAF bytecode."""
    from creation_lib.nif.operations.mopp_compiler import compile_mopp

    verts = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
    triangles = [(0, 1, 2)]
    mopp_bytes, origin, scale = compile_mopp(verts, triangles, radius=0.005)

    assert len(mopp_bytes) > 0
    # Should have 3 root FILTERs + 3 leaf FILTERs + 1 LEAF opcode
    # Root: 3 * 3 bytes = 9
    # Leaf: 3 * 3 bytes + 1 byte (LEAF 0x30) = 10
    # Total: 19 bytes
    assert len(mopp_bytes) == 19
    # Last byte should be LEAF opcode 0x30 (output_id=0)
    assert mopp_bytes[-1] == 0x30


def test_compile_mopp_custom_output_ids():
    """Custom output IDs should appear in leaf nodes."""
    from creation_lib.nif.operations.mopp_compiler import compile_mopp

    verts = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
    triangles = [(0, 1, 2)]
    mopp_bytes, _, _ = compile_mopp(verts, triangles, radius=0.005, output_ids=[5])

    # output_id=5 → opcode 0x30 + 5 = 0x35
    assert mopp_bytes[-1] == 0x35


def test_compile_mopp_oblivion_radius():
    """Oblivion/FO3 uses radius=0.1 instead of Skyrim's 0.005."""
    from creation_lib.nif.operations.mopp_compiler import compile_mopp

    verts = [(0, 0, 0), (1, 0, 0), (0, 1, 0), (1, 1, 0)]
    triangles = [(0, 1, 2), (1, 3, 2)]
    mopp_bytes, origin, scale = compile_mopp(verts, triangles, radius=0.1)

    assert len(mopp_bytes) > 0
    # Origin offset should be larger due to bigger radius
    assert origin[0] == pytest.approx(-0.1, abs=0.01)


def test_mopp_disassemble_round_trip():
    """Disassembling compiled MOPP should produce valid instruction tree."""
    from creation_lib.nif.operations.mopp_compiler import compile_mopp, disassemble_mopp

    # Icosahedron-ish shape (20 triangles)
    t = (1 + math.sqrt(5)) / 2
    verts = [
        (-1, t, 0), (1, t, 0), (-1, -t, 0), (1, -t, 0),
        (0, -1, t), (0, 1, t), (0, -1, -t), (0, 1, -t),
        (t, 0, -1), (t, 0, 1), (-t, 0, -1), (-t, 0, 1),
    ]
    triangles = [
        (0, 11, 5), (0, 5, 1), (0, 1, 7), (0, 7, 10), (0, 10, 11),
        (1, 5, 9), (5, 11, 4), (11, 10, 2), (10, 7, 6), (7, 1, 8),
        (3, 9, 4), (3, 4, 2), (3, 2, 6), (3, 6, 8), (3, 8, 9),
        (4, 9, 5), (2, 4, 11), (6, 2, 10), (8, 6, 7), (9, 8, 1),
    ]
    mopp_bytes, origin, scale = compile_mopp(verts, triangles)

    lines = disassemble_mopp(mopp_bytes, origin, scale)
    assert len(lines) > 0
    # Should have FILTER instructions at the top
    assert any("FILTER" in line for line in lines)
    # Should have 20 LEAF entries (one per triangle)
    leaf_count = sum(1 for line in lines if "LEAF" in line)
    assert leaf_count == 20


def test_compile_mopp_large_output_ids():
    """Output IDs > 31 should use multi-byte leaf opcodes."""
    from creation_lib.nif.operations.mopp_compiler import compile_mopp

    verts = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
    triangles = [(0, 1, 2)]

    # output_id=0x100 → opcode 0x51 (2-byte)
    mopp_bytes, _, _ = compile_mopp(verts, triangles, radius=0.005, output_ids=[0x100])
    # Last 3 bytes: 0x51, 0x01, 0x00
    assert mopp_bytes[-3] == 0x51
    assert mopp_bytes[-2] == 0x01
    assert mopp_bytes[-1] == 0x00

    # output_id=0x20 → opcode 0x50 (1-byte payload)
    mopp_bytes, _, _ = compile_mopp(verts, triangles, radius=0.005, output_ids=[0x20])
    assert mopp_bytes[-2] == 0x50
    assert mopp_bytes[-1] == 0x20
