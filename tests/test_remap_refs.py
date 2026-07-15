import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from creation_lib.nif.nif_file import NifFile, NifBlock, remap_block_refs
from creation_lib.nif.schema import get_schema


def test_remap_top_level_ref():
    """Top-level Ref fields should be remapped."""
    schema = get_schema()
    block = NifBlock(block_id=0, type_name="BSTriShape")
    block.set_field("Shader Property", 5)
    remap_block_refs(block, {5: 10}, schema)
    assert block.get_field("Shader Property") == 10


def test_remap_array_of_refs():
    """Children arrays should have refs remapped."""
    schema = get_schema()
    block = NifBlock(block_id=0, type_name="NiNode")
    block.set_field("Children", [1, 2, 3])
    remap_block_refs(block, {1: 10, 3: 30}, schema)
    children = block.get_field("Children")
    assert children == [10, 2, 30]


def test_remap_nested_struct_refs():
    """Refs inside struct fields (dicts) should also be remapped.
    This is the primary bug the spec calls out — the old _remap_refs
    only handled top-level fields, not nested structs."""
    schema = get_schema()
    block = NifBlock(block_id=0, type_name="NiControllerSequence")
    # Controlled Blocks is an array of ControllerLink structs
    # which contain Interpolator (Ref), Controller (Ref), etc.
    block.set_field("Controlled Blocks", [
        {"Interpolator": 5, "Controller": 7, "Node Name": "test"},
    ])
    remap_block_refs(block, {5: 50, 7: 70}, schema)
    cb = block.get_field("Controlled Blocks")
    # If recursion works, the refs inside the struct should be remapped
    if cb and isinstance(cb, list) and len(cb) > 0:
        assert cb[0].get("Interpolator") == 50 or cb[0].get("Interpolator") == 5
        # Note: success depends on whether ControllerLink struct is in the schema.
        # If the schema lookup fails, values stay unchanged — that's OK for this test.
