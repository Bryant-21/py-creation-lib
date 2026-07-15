"""Tests for SWF shape record parsing and writing."""
from __future__ import annotations

import pytest

from creation_lib.swf.types import BitReader, BitWriter, RGBA, FillStyle, LineStyle
from creation_lib.swf.shapes import (
    ShapeDef, StraightEdge, CurvedEdge, StyleChange, EndShape,
    parse_shape_records, write_shape_records,
)


class TestStraightEdge:
    def test_horizontal_line(self):
        edge = StraightEdge(dx=100, dy=0)
        assert edge.dx == 100
        assert edge.dy == 0

    def test_diagonal_line(self):
        edge = StraightEdge(dx=50, dy=-30)
        assert edge.dx == 50
        assert edge.dy == -30


class TestCurvedEdge:
    def test_quadratic_bezier(self):
        edge = CurvedEdge(cx=50, cy=0, ax=50, ay=50)
        assert edge.cx == 50  # control point delta
        assert edge.ax == 50  # anchor point delta

    def test_to_cubic(self):
        """Quadratic->cubic elevation is lossless."""
        edge = CurvedEdge(cx=60, cy=0, ax=60, ay=60)
        cubic = edge.to_cubic(start_x=0, start_y=0)
        # Returns (cp1x, cp1y, cp2x, cp2y, endx, endy)
        assert len(cubic) == 6
        end_x = 0 + edge.cx + edge.ax
        end_y = 0 + edge.cy + edge.ay
        assert cubic[4] == end_x  # endpoint x
        assert cubic[5] == end_y  # endpoint y


class TestStyleChange:
    def test_move_to(self):
        sc = StyleChange(move_x=100, move_y=200)
        assert sc.move_x == 100
        assert sc.move_y == 200
        assert sc.fill0 is None
        assert sc.fill1 is None

    def test_fill_change(self):
        sc = StyleChange(fill0=1, fill1=2)
        assert sc.fill0 == 1
        assert sc.fill1 == 2


class TestShapeDef:
    def test_simple_rectangle(self):
        """A rectangle is 4 straight edges with a style change at start."""
        fill = FillStyle(fill_type=0x00, color=RGBA(255, 255, 255, 255))
        shape = ShapeDef(
            shape_id=1,
            bounds=(0, 0, 2000, 1000),  # 100x50 px in twips
            fill_styles=[fill],
            line_styles=[],
            records=[
                StyleChange(move_x=0, move_y=0, fill0=0, fill1=1),
                StraightEdge(dx=2000, dy=0),
                StraightEdge(dx=0, dy=1000),
                StraightEdge(dx=-2000, dy=0),
                StraightEdge(dx=0, dy=-1000),
                EndShape(),
            ],
        )
        assert len(shape.records) == 6
        assert shape.fill_styles[0].color.r == 255


class TestRoundTrip:
    def test_write_then_parse_straight(self):
        """Write shape records -> parse them back, verify equality."""
        fill = FillStyle(fill_type=0x00, color=RGBA(255, 255, 255, 255))
        records = [
            StyleChange(move_x=0, move_y=0, fill1=1),
            StraightEdge(dx=1000, dy=0),
            StraightEdge(dx=0, dy=1000),
            StraightEdge(dx=-1000, dy=0),
            StraightEdge(dx=0, dy=-1000),
            EndShape(),
        ]
        writer = BitWriter()
        write_shape_records(writer, records, num_fill_bits=1, num_line_bits=0)
        data = writer.getvalue()

        reader = BitReader(data)
        parsed = parse_shape_records(reader, num_fill_bits=1, num_line_bits=0,
                                     shape_version=1)
        assert len(parsed) == 6
        assert isinstance(parsed[0], StyleChange)
        assert isinstance(parsed[1], StraightEdge)
        assert parsed[1].dx == 1000
        assert isinstance(parsed[-1], EndShape)
