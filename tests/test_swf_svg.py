"""Tests for SVG <-> SWF shape conversion."""
from __future__ import annotations

import pytest

from creation_lib.swf.types import RGBA, FillStyle
from creation_lib.swf.shapes import ShapeDef, StraightEdge, CurvedEdge, StyleChange, EndShape
from creation_lib.swf.svg_io import shape_to_svg, svg_to_shapes


class TestShapeToSvg:
    def test_rectangle_to_svg(self):
        fill = FillStyle(fill_type=0x00, color=RGBA(255, 255, 255, 255))
        shape = ShapeDef(
            shape_id=1,
            bounds=(0, 0, 2000, 1000),
            fill_styles=[fill],
            line_styles=[],
            records=[
                StyleChange(move_x=0, move_y=0, fill1=1),
                StraightEdge(dx=2000, dy=0),
                StraightEdge(dx=0, dy=1000),
                StraightEdge(dx=-2000, dy=0),
                StraightEdge(dx=0, dy=-1000),
                EndShape(),
            ],
        )
        svg = shape_to_svg(shape)
        assert "<svg" in svg
        assert "<path" in svg
        assert 'fill="#ffffff"' in svg

    def test_curved_shape_to_svg(self):
        fill = FillStyle(fill_type=0x00, color=RGBA(255, 255, 255, 255))
        shape = ShapeDef(
            shape_id=2,
            bounds=(0, 0, 1000, 1000),
            fill_styles=[fill],
            line_styles=[],
            records=[
                StyleChange(move_x=0, move_y=500, fill1=1),
                CurvedEdge(cx=250, cy=-500, ax=250, ay=500),
                CurvedEdge(cx=250, cy=-500, ax=250, ay=500),
                StraightEdge(dx=0, dy=0),
                EndShape(),
            ],
        )
        svg = shape_to_svg(shape)
        # Cubic beziers in SVG use 'C' command
        assert "C" in svg or "Q" in svg


class TestSvgToShapes:
    def test_simple_rect_svg(self):
        svg_content = '''<svg viewBox="0 0 100 50">
            <rect x="0" y="0" width="100" height="50" fill="white"/>
        </svg>'''
        shapes = svg_to_shapes(svg_content)
        assert len(shapes) >= 1
        assert shapes[0].fill_styles[0].color.r == 255

    def test_path_svg(self):
        svg_content = '''<svg viewBox="0 0 100 100">
            <path d="M 0 0 L 100 0 L 100 100 L 0 100 Z" fill="#ffffff"/>
        </svg>'''
        shapes = svg_to_shapes(svg_content)
        assert len(shapes) >= 1
