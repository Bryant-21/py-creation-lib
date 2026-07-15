import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from creation_lib.nif.nif_file import NifFile, NifBlock
from creation_lib.nif.operations.normals import fix_normals, flip_normals, normalize_normals


def _make_shape_nif() -> NifFile:
    """Create a NIF with one BSTriShape containing a triangle."""
    nif = NifFile()
    root = NifBlock(block_id=0, type_name="BSFadeNode")
    root.set_field("Children", [1])
    nif.blocks.append(root)

    shape = NifBlock(block_id=1, type_name="BSTriShape")
    shape.set_field("Vertex Data", [
        {"Vertex": {"x": 0, "y": 0, "z": 0}, "Normal": {"x": 0, "y": 0, "z": 1}},
        {"Vertex": {"x": 1, "y": 0, "z": 0}, "Normal": {"x": 0, "y": 0, "z": 1}},
        {"Vertex": {"x": 0, "y": 1, "z": 0}, "Normal": {"x": 0, "y": 0, "z": 1}},
    ])
    shape.set_field("Triangles", [{"v1": 0, "v2": 1, "v3": 2}])
    nif.blocks.append(shape)
    # Stub find_blocks for testing
    nif.find_blocks = lambda t: [b for b in nif.blocks if b.type_name == t]
    return nif


def test_fix_normals_succeeds():
    nif = _make_shape_nif()
    result = fix_normals(nif)
    assert result.success
    assert len(result.modified_block_ids) == 1


def test_flip_normals_negates():
    nif = _make_shape_nif()
    result = flip_normals(nif)
    assert result.success
    vd = nif.blocks[1].get_field("Vertex Data")
    assert vd[0]["Normal"]["z"] == -1.0
