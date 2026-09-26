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

    class _FakeSchema:
        def is_subtype_of(self, t, base):
            return t == base or t == "NiNode"
        def get_all_fields(self, t):
            return []
        def get_type_hierarchy(self, t):
            return [t]
    nif._schema = _FakeSchema()

    return nif


def test_mirror_skeleton_negates_x_and_reports_no_change_on_y():
    nif = _make_skeleton_nif()
    result = mirror_skeleton(nif, "x")
    assert result.success
    assert result.modified_block_ids  # at least one bone mirrored
    assert nif.blocks[1].get_field("Translation")["x"] == -5.0
    assert nif.blocks[2].get_field("Translation")["x"] == 5.0

    # Y was 0 for all bones, so mirroring on Y should modify nothing.
    y_result = mirror_skeleton(_make_skeleton_nif(), "y")
    assert y_result.success
    assert len(y_result.modified_block_ids) == 0


def test_mirror_skeleton_invalid_axis():
    nif = _make_skeleton_nif()
    assert not mirror_skeleton(nif, "w").success


def test_fix_bone_bounds_failure_cases():
    """Missing block, and a block without skin, should both fail gracefully."""
    nif = NifFile()
    shape = NifBlock(block_id=0, type_name="BSTriShape")
    nif.blocks.append(shape)
    assert not fix_bone_bounds(nif, 0).success  # no skin
    assert not fix_bone_bounds(nif, 99).success  # invalid block
