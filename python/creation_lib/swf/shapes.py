"""SWF shape records: edges, curves, style changes.

Shape records are the vector primitives inside DefineShape tags.
SWF uses quadratic beziers natively. The editor elevates to cubic
beziers (lossless) and subdivides back to quadratic on export.
"""
from __future__ import annotations

from dataclasses import dataclass, field

from creation_lib.swf.types import (
    BitReader, BitWriter, RECT, FillStyle, LineStyle, LineStyle2,
    parse_fill_styles, parse_line_styles, write_fill_styles, write_line_styles,
    _sbits_needed,
)


@dataclass
class StraightEdge:
    """Straight line segment (delta from current position)."""
    dx: int
    dy: int


@dataclass
class CurvedEdge:
    """Quadratic bezier curve.

    cx, cy: delta to control point from current position
    ax, ay: delta to anchor (endpoint) from control point
    """
    cx: int
    cy: int
    ax: int
    ay: int

    def to_cubic(self, start_x: int, start_y: int) -> tuple[int, int, int, int, int, int]:
        """Elevate to cubic bezier. Returns (cp1x, cp1y, cp2x, cp2y, endx, endy)."""
        # Quadratic: P0=start, P1=control, P2=anchor
        p1x = start_x + self.cx
        p1y = start_y + self.cy
        p2x = p1x + self.ax
        p2y = p1y + self.ay
        # Cubic: CP1 = P0 + 2/3*(P1-P0), CP2 = P2 + 2/3*(P1-P2)
        cp1x = start_x + 2 * (p1x - start_x) // 3
        cp1y = start_y + 2 * (p1y - start_y) // 3
        cp2x = p2x + 2 * (p1x - p2x) // 3
        cp2y = p2y + 2 * (p1y - p2y) // 3
        return (cp1x, cp1y, cp2x, cp2y, p2x, p2y)


@dataclass
class StyleChange:
    """Style change record -- updates position, fills, lines, or styles."""
    move_x: int | None = None
    move_y: int | None = None
    fill0: int | None = None
    fill1: int | None = None
    line: int | None = None
    new_fill_styles: list[FillStyle] | None = None
    new_line_styles: list[LineStyle | LineStyle2] | None = None

    @property
    def has_move(self) -> bool:
        return self.move_x is not None

    @property
    def has_new_styles(self) -> bool:
        return self.new_fill_styles is not None


@dataclass
class EndShape:
    """End of shape records marker."""
    pass


ShapeRecord = StraightEdge | CurvedEdge | StyleChange | EndShape


@dataclass
class ShapeDef:
    """Complete shape definition (from DefineShape tag)."""
    shape_id: int
    bounds: tuple[int, int, int, int]  # xmin, ymin, xmax, ymax in twips
    fill_styles: list[FillStyle]
    line_styles: list[LineStyle | LineStyle2]
    records: list[ShapeRecord]
    shape_version: int = 1  # 1-4 matching DefineShape version
    edge_bounds: tuple[int, int, int, int] | None = None  # DefineShape4 only

    @property
    def bounds_px(self) -> tuple[float, float, float, float]:
        """Bounds in pixels."""
        return (
            self.bounds[0] / 20.0, self.bounds[1] / 20.0,
            self.bounds[2] / 20.0, self.bounds[3] / 20.0,
        )


def parse_shape_records(
    reader: BitReader,
    num_fill_bits: int,
    num_line_bits: int,
    shape_version: int = 1,
) -> list[ShapeRecord]:
    """Parse shape records from bit stream."""
    records: list[ShapeRecord] = []
    nfb = num_fill_bits
    nlb = num_line_bits

    while True:
        is_edge = reader.read_ubits(1)
        if is_edge:
            is_straight = reader.read_ubits(1)
            if is_straight:
                nbits = reader.read_ubits(4) + 2
                is_general = reader.read_ubits(1)
                if is_general:
                    dx = reader.read_sbits(nbits)
                    dy = reader.read_sbits(nbits)
                else:
                    is_vert = reader.read_ubits(1)
                    if is_vert:
                        dx = 0
                        dy = reader.read_sbits(nbits)
                    else:
                        dx = reader.read_sbits(nbits)
                        dy = 0
                records.append(StraightEdge(dx=dx, dy=dy))
            else:
                nbits = reader.read_ubits(4) + 2
                cx = reader.read_sbits(nbits)
                cy = reader.read_sbits(nbits)
                ax = reader.read_sbits(nbits)
                ay = reader.read_sbits(nbits)
                records.append(CurvedEdge(cx=cx, cy=cy, ax=ax, ay=ay))
        else:
            # Non-edge: check for EndShape (all flags zero)
            flags = reader.read_ubits(5)
            if flags == 0:
                records.append(EndShape())
                break

            sc = StyleChange()
            if flags & 0x01:  # HasMoveTo
                move_bits = reader.read_ubits(5)
                sc.move_x = reader.read_sbits(move_bits)
                sc.move_y = reader.read_sbits(move_bits)
            if flags & 0x02:  # HasFillStyle0
                sc.fill0 = reader.read_ubits(nfb)
            if flags & 0x04:  # HasFillStyle1
                sc.fill1 = reader.read_ubits(nfb)
            if flags & 0x08:  # HasLineStyle
                sc.line = reader.read_ubits(nlb)
            if flags & 0x10:  # HasNewStyles (DefineShape2+ only)
                has_alpha = shape_version >= 3
                sc.new_fill_styles = parse_fill_styles(reader, has_alpha)
                sc.new_line_styles = parse_line_styles(reader, shape_version, has_alpha)
                nfb = reader.read_ubits(4)
                nlb = reader.read_ubits(4)
            records.append(sc)

    return records


def write_shape_records(
    writer: BitWriter,
    records: list[ShapeRecord],
    num_fill_bits: int,
    num_line_bits: int,
    shape_version: int = 1,
) -> None:
    """Write shape records to bit stream."""
    nfb = num_fill_bits
    nlb = num_line_bits

    for rec in records:
        if isinstance(rec, StraightEdge):
            writer.write_ubits(1, 1)  # is_edge
            writer.write_ubits(1, 1)  # is_straight
            nbits = max(_sbits_needed(rec.dx), _sbits_needed(rec.dy), 2)
            writer.write_ubits(4, nbits - 2)
            if rec.dx != 0 and rec.dy != 0:
                writer.write_ubits(1, 1)  # general
                writer.write_sbits(nbits, rec.dx)
                writer.write_sbits(nbits, rec.dy)
            elif rec.dx == 0:
                writer.write_ubits(1, 0)  # not general
                writer.write_ubits(1, 1)  # vertical
                writer.write_sbits(nbits, rec.dy)
            else:
                writer.write_ubits(1, 0)  # not general
                writer.write_ubits(1, 0)  # horizontal
                writer.write_sbits(nbits, rec.dx)

        elif isinstance(rec, CurvedEdge):
            writer.write_ubits(1, 1)  # is_edge
            writer.write_ubits(1, 0)  # is_curved
            nbits = max(
                _sbits_needed(rec.cx), _sbits_needed(rec.cy),
                _sbits_needed(rec.ax), _sbits_needed(rec.ay), 2
            )
            writer.write_ubits(4, nbits - 2)
            writer.write_sbits(nbits, rec.cx)
            writer.write_sbits(nbits, rec.cy)
            writer.write_sbits(nbits, rec.ax)
            writer.write_sbits(nbits, rec.ay)

        elif isinstance(rec, StyleChange):
            writer.write_ubits(1, 0)  # non-edge
            flags = 0
            if rec.has_move:
                flags |= 0x01
            if rec.fill0 is not None:
                flags |= 0x02
            if rec.fill1 is not None:
                flags |= 0x04
            if rec.line is not None:
                flags |= 0x08
            if rec.has_new_styles:
                flags |= 0x10
            writer.write_ubits(5, flags)

            if flags & 0x01:
                vals = [rec.move_x, rec.move_y]
                move_bits = max(_sbits_needed(v) for v in vals) if any(v != 0 for v in vals) else 0
                writer.write_ubits(5, move_bits)
                writer.write_sbits(move_bits, rec.move_x)
                writer.write_sbits(move_bits, rec.move_y)
            if flags & 0x02:
                writer.write_ubits(nfb, rec.fill0)
            if flags & 0x04:
                writer.write_ubits(nfb, rec.fill1)
            if flags & 0x08:
                writer.write_ubits(nlb, rec.line)
            if flags & 0x10:
                has_alpha = shape_version >= 3
                write_fill_styles(writer, rec.new_fill_styles, has_alpha)
                write_line_styles(writer, rec.new_line_styles, shape_version, has_alpha)
                nfb = _ubits_needed(len(rec.new_fill_styles))
                nlb = _ubits_needed(len(rec.new_line_styles))
                writer.write_ubits(4, nfb)
                writer.write_ubits(4, nlb)

        elif isinstance(rec, EndShape):
            writer.write_ubits(1, 0)  # non-edge
            writer.write_ubits(5, 0)  # all flags zero = end


def _ubits_needed(value: int) -> int:
    """Minimum unsigned bits to represent value."""
    if value <= 0:
        return 0
    return value.bit_length()
