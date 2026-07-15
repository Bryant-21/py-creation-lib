import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import math
from creation_lib.nif.nif_file import NifFile, NifBlock
from creation_lib.nif.operations.tangent_space import generate_tangent_space


def _make_shape_nif() -> NifFile:
    """Create a NIF with one BSTriShape containing a quad (two triangles).

    The quad lies in the XY plane with normals pointing along +Z.
    UVs map directly to XY so expected tangent = +X, bitangent = +Y.
    """
    nif = NifFile()
    root = NifBlock(block_id=0, type_name="BSFadeNode")
    root.set_field("Children", [1])
    nif.blocks.append(root)

    shape = NifBlock(block_id=1, type_name="BSTriShape")
    shape.set_field("Vertex Data", [
        {"Vertex": {"x": 0, "y": 0, "z": 0}, "Normal": {"x": 0, "y": 0, "z": 1}, "UV": {"u": 0, "v": 0}},
        {"Vertex": {"x": 1, "y": 0, "z": 0}, "Normal": {"x": 0, "y": 0, "z": 1}, "UV": {"u": 1, "v": 0}},
        {"Vertex": {"x": 1, "y": 1, "z": 0}, "Normal": {"x": 0, "y": 0, "z": 1}, "UV": {"u": 1, "v": 1}},
        {"Vertex": {"x": 0, "y": 1, "z": 0}, "Normal": {"x": 0, "y": 0, "z": 1}, "UV": {"u": 0, "v": 1}},
    ])
    shape.set_field("Triangles", [
        {"v1": 0, "v2": 1, "v3": 2},
        {"v1": 0, "v2": 2, "v3": 3},
    ])
    nif.blocks.append(shape)
    nif.find_blocks = lambda t: [b for b in nif.blocks if b.type_name == t]
    return nif


def _vec_length(x, y, z):
    return math.sqrt(x * x + y * y + z * z)


def _dot(a, b):
    return sum(ai * bi for ai, bi in zip(a, b))


def test_tangent_space_succeeds():
    nif = _make_shape_nif()
    result = generate_tangent_space(nif, 1)
    assert result.success
    assert 1 in result.modified_block_ids


def test_tangent_vectors_nonzero():
    nif = _make_shape_nif()
    generate_tangent_space(nif, 1)
    vdata = nif.blocks[1].get_field("Vertex Data")
    for vd in vdata:
        t = vd.get("Tangent", {})
        length = _vec_length(float(t.get("x", 0)), float(t.get("y", 0)), float(t.get("z", 0)))
        assert length > 0.9, f"Tangent vector too short: {length}"


def test_tangent_vectors_unit_length():
    nif = _make_shape_nif()
    generate_tangent_space(nif, 1)
    vdata = nif.blocks[1].get_field("Vertex Data")
    for vd in vdata:
        t = vd.get("Tangent", {})
        length = _vec_length(float(t.get("x", 0)), float(t.get("y", 0)), float(t.get("z", 0)))
        assert abs(length - 1.0) < 1e-4, f"Tangent not unit length: {length}"


def test_tangent_orthogonal_to_normal():
    """Tangent vectors must be orthogonal to the vertex normal."""
    nif = _make_shape_nif()
    generate_tangent_space(nif, 1)
    vdata = nif.blocks[1].get_field("Vertex Data")
    for vd in vdata:
        n = vd.get("Normal", {})
        t = vd.get("Tangent", {})
        normal = [float(n.get("x", 0)), float(n.get("y", 0)), float(n.get("z", 0))]
        tangent = [float(t.get("x", 0)), float(t.get("y", 0)), float(t.get("z", 0))]
        dot = _dot(normal, tangent)
        assert abs(dot) < 1e-4, f"Tangent not orthogonal to normal: dot={dot}"


def test_tangent_direction_for_flat_quad():
    """For a flat XY quad with identity UV mapping, tangent should be ~+X."""
    nif = _make_shape_nif()
    generate_tangent_space(nif, 1)
    vdata = nif.blocks[1].get_field("Vertex Data")
    for vd in vdata:
        t = vd.get("Tangent", {})
        tx = float(t.get("x", 0))
        # Tangent should point primarily along +X
        assert tx > 0.9, f"Expected tangent X > 0.9, got {tx}"


def test_bitangent_fields_present():
    """Bitangent components should be written to vertex data."""
    nif = _make_shape_nif()
    generate_tangent_space(nif, 1)
    vdata = nif.blocks[1].get_field("Vertex Data")
    for vd in vdata:
        assert "Bitangent X" in vd
        assert "Bitangent Y" in vd
        assert "Bitangent Z" in vd


def test_invalid_block_id():
    nif = _make_shape_nif()
    result = generate_tangent_space(nif, 99)
    assert not result.success


def test_block_without_data():
    nif = NifFile()
    block = NifBlock(block_id=0, type_name="BSTriShape")
    nif.blocks.append(block)
    result = generate_tangent_space(nif, 0)
    assert not result.success
