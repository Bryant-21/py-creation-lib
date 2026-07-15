"""Tests for py_creation_lib/python/creation_lib/nif/actions.py — verify execute/undo round-trips."""
import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from creation_lib.nif.nif_file import NifFile, NifBlock
from creation_lib.nif.actions import (
    SetFieldAction, SnapshotAction, CompositeAction, OperationResult,
)


def _make_test_nif() -> NifFile:
    """Create a minimal NIF with one block for testing."""
    nif = NifFile()
    block = NifBlock(block_id=0, type_name="NiNode")
    block.set_field("Name", "TestNode")
    block.set_field("Scale", 1.0)
    nif.blocks.append(block)
    nif.header.num_blocks = 1
    return nif


def test_set_field_action_execute_and_undo():
    nif = _make_test_nif()
    action = SetFieldAction(block_id=0, field_name="Name", old_value="TestNode", new_value="NewName")
    result = action.execute(nif)
    assert result.success
    assert nif.blocks[0].get_field("Name") == "NewName"
    result = action.undo(nif)
    assert result.success
    assert nif.blocks[0].get_field("Name") == "TestNode"


def test_set_field_action_updates_list_item_paths_without_appending_fields():
    nif = _make_test_nif()
    block = nif.blocks[0]
    colors = [
        {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0},
        {"r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0},
    ]
    block.set_field("Colors", colors)
    new_color = {"r": 0.25, "g": 0.5, "b": 0.75, "a": 1.0}
    action = SetFieldAction(
        block_id=0,
        field_name="Colors[0]",
        old_value=colors[0],
        new_value=new_color,
    )

    result = action.execute(nif)

    assert result.success
    assert block.get_field("Colors")[0] == new_color
    assert block.get_field("Colors[0]") == new_color
    assert "Colors[0]" not in dict(block.fields)

    result = action.undo(nif)

    assert result.success
    assert block.get_field("Colors")[0] == colors[0]
    assert "Colors[0]" not in dict(block.fields)


def test_snapshot_action_execute_and_undo():
    nif = _make_test_nif()
    action = SnapshotAction(_description="test snapshot")
    action.capture_before(nif)
    nif.blocks[0].set_field("Name", "Modified")
    action.capture_after(nif)
    # Undo restores original
    action.undo(nif)
    assert nif.blocks[0].get_field("Name") == "TestNode"
    # Redo restores modified
    action.execute(nif)
    assert nif.blocks[0].get_field("Name") == "Modified"


def test_composite_action():
    nif = _make_test_nif()
    children = [
        SetFieldAction(block_id=0, field_name="Name", old_value="TestNode", new_value="A"),
        SetFieldAction(block_id=0, field_name="Scale", old_value=1.0, new_value=2.0),
    ]
    comp = CompositeAction(children=children, _description="composite test")
    comp.execute(nif)
    assert nif.blocks[0].get_field("Name") == "A"
    assert nif.blocks[0].get_field("Scale") == 2.0
    comp.undo(nif)
    assert nif.blocks[0].get_field("Name") == "TestNode"
    assert nif.blocks[0].get_field("Scale") == 1.0
