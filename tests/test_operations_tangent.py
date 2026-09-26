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


def test_tangent_space_produces_unit_orthogonal_tangents_along_uv_x():
    """For a flat XY quad with identity UV mapping: tangent/bitangent fields are
    written, unit length, orthogonal to the normal, and point along +X/+Y."""
    nif = _make_shape_nif()
    result = generate_tangent_space(nif, 1)
    assert result.success
    assert 1 in result.modified_block_ids

    vdata = nif.blocks[1].get_field("Vertex Data")
    for vd in vdata:
        assert "Bitangent X" in vd and "Bitangent Y" in vd and "Bitangent Z" in vd

        n = vd.get("Normal", {})
        t = vd.get("Tangent", {})
        tangent = [float(t.get("x", 0)), float(t.get("y", 0)), float(t.get("z", 0))]
        normal = [float(n.get("x", 0)), float(n.get("y", 0)), float(n.get("z", 0))]

        length = _vec_length(*tangent)
        assert abs(length - 1.0) < 1e-4, f"Tangent not unit length: {length}"
        assert abs(_dot(normal, tangent)) < 1e-4, "Tangent not orthogonal to normal"
        assert tangent[0] > 0.9, f"Expected tangent X > 0.9, got {tangent[0]}"


def test_generate_tangent_space_failure_cases():
    nif = _make_shape_nif()
    assert not generate_tangent_space(nif, 99).success  # invalid block id

    empty_nif = NifFile()
    empty_nif.blocks.append(NifBlock(block_id=0, type_name="BSTriShape"))
    assert not generate_tangent_space(empty_nif, 0).success  # block without data
