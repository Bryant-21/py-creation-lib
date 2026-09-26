"""Tests for SWF shape SVG rendering and category classification.

Synthetic only: no extracted game files or built shape-library DB required.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from creation_lib.swf.shapes import ShapeDef, StraightEdge, StyleChange, EndShape
from creation_lib.swf.svg_io import shape_to_svg
from creation_lib.swf.types import RGBA, FillStyle


def _make_shape(fill0: int | None = None, fill1: int | None = None) -> ShapeDef:
    fill = FillStyle(fill_type=0x00, color=RGBA(255, 255, 255, 255))
    return ShapeDef(
        shape_id=1,
        bounds=(0, 0, 2000, 1000),
        fill_styles=[fill],
        line_styles=[],
        records=[
            StyleChange(move_x=0, move_y=0, fill0=fill0, fill1=fill1),
            StraightEdge(dx=2000, dy=0),
            StraightEdge(dx=0, dy=1000),
            StraightEdge(dx=-2000, dy=0),
            StraightEdge(dx=0, dy=-1000),
            EndShape(),
        ],
    )


def test_fill0_only_was_previously_broken():
    """fill0-only shapes must not render with fill='none'."""
    shape = _make_shape(fill0=1, fill1=None)
    svg = shape_to_svg(shape, background=None)
    assert 'fill="none"' not in svg
    assert 'fill="#ffffff"' in svg


def test_dark_background_present():
    """Default background should be a dark rect for white shape visibility."""
    shape = _make_shape(fill1=1)
    svg = shape_to_svg(shape)
    assert "#333333" in svg
    assert "<rect" in svg


@pytest.mark.parametrize("path,expected_category", [
    ("Components/VaultBoys/Perks/Test.swf", "Perk"),
    ("Components/VaultBoys/SPECIAL/Test.swf", "SPECIAL"),
    ("Components/Quest Vault Boys/Act 1 Quest/Test.swf", "Quest"),
    ("Components/Faction Vault boys/Minutemen/Test.swf", "Faction"),
    ("Components/Magazine perks/Test.swf", "Magazine"),
    ("Components/ConditionClips/Test.swf", "Condition"),
    ("99439_DLC04 Quest animations/SWF/Test.swf", "Quest"),
    ("Components/VaultBoys/DLC04/Test.swf", "DLC Perk"),
])
def test_classify_swf_assigns_category(path, expected_category):
    from creation_lib.preprocessor.swf import _classify_swf

    root = Path("/game/Interface")
    cat, _sub = _classify_swf(root / path, root)
    assert cat == expected_category
