import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from creation_lib.nif.nif_file import NifFile, NifBlock
from creation_lib.nif.operations.skeleton import fix_bone_bounds, mirror_skeleton


def _make_skeleton_nif() -> NifFile:
    """Create a NIF with NiNode bones having non-zero translations."""
    nif = NifFile()

    root = NifBlock(block_id=0, type_name="NiNode")
    root.set_field("Name", "Root")
    root.set_field("Translation", {"x": 0.0, "y": 0.0, "z": 0.0})
    root.set_field("Children", [1, 2])
    nif.blocks.append(root)

    bone_l = NifBlock(block_id=1, type_name="NiNode")
    bone_l.set_field("Name", "Bone_L")
    bone_l.set_field("Translation", {"x": 5.0, "y": 0.0, "z": 10.0})
    bone_l.set_field("Children", [])
    nif.blocks.append(bone_l)

    bone_r = NifBlock(block_id=2, type_name="NiNode")
    bone_r.set_field("Name", "Bone_R")
    bone_r.set_field("Translation", {"x": -5.0, "y": 0.0, "z": 10.0})
    bone_r.set_field("Children", [])
    nif.blocks.append(bone_r)

    # Stub schema methods
    class _FakeSchema:
        def is_subtype_of(self, t, base):
            return t == base or t == "NiNode"
        def get_all_fields(self, t):
            return []
        def get_type_hierarchy(self, t):
            return [t]
    nif._schema = _FakeSchema()

    return nif


def test_mirror_skeleton_x():
    nif = _make_skeleton_nif()
    result = mirror_skeleton(nif, "x")
    assert result.success
    assert result.modified_block_ids  # at least one bone mirrored

    bone_l = nif.blocks[1]
    t = bone_l.get_field("Translation")
    assert t["x"] == -5.0  # negated

    bone_r = nif.blocks[2]
    t = bone_r.get_field("Translation")
    assert t["x"] == 5.0  # negated back


def test_mirror_skeleton_y():
    nif = _make_skeleton_nif()
    result = mirror_skeleton(nif, "y")
    assert result.success
    # Y was 0 for all bones, so nothing should be modified
    assert len(result.modified_block_ids) == 0


def test_mirror_skeleton_invalid_axis():
    nif = _make_skeleton_nif()
    result = mirror_skeleton(nif, "w")
    assert not result.success


def test_fix_bone_bounds_no_skin():
    """Block without skin should fail gracefully."""
    nif = NifFile()
    shape = NifBlock(block_id=0, type_name="BSTriShape")
    nif.blocks.append(shape)
    result = fix_bone_bounds(nif, 0)
    assert not result.success


def test_fix_bone_bounds_invalid_block():
    nif = NifFile()
    result = fix_bone_bounds(nif, 99)
    assert not result.success
