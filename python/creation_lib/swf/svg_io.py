"""SVG <-> SWF shape conversion.

Export: SWF shape records -> SVG path strings (quadratic elevated to cubic)
Import: SVG path/rect/circle/ellipse -> SWF shape records (cubic subdivided to quadratic)
"""
from __future__ import annotations

import re
import xml.etree.ElementTree as ET
from dataclasses import dataclass

from creation_lib.swf.types import RGBA, FillStyle, LineStyle, LineStyle2, TWIPS_PER_PIXEL
from creation_lib.swf.shapes import (
    ShapeDef, StraightEdge, CurvedEdge, StyleChange, EndShape, ShapeRecord,
)


# A geometry segment in twips: start, end, and an optional quadratic control point.
# (sx, sy, ex, ey, cx | None, cy | None)
_Segment = tuple[int, int, int, int, "int | None", "int | None"]


def shape_to_svg(
    shape: ShapeDef,
    scale: float = 1.0 / TWIPS_PER_PIXEL,
    background: str | None = "#333333",
) -> str:
    """Convert a ShapeDef to an SVG string.

    SWF shapes are an unordered soup of edges, each tagged with the fill on its
    left (fillStyle0) and right (fillStyle1) side; a hole is a contour wound
    against its enclosing one. Rendering each style-change run as its own solid
    path fills holes in and mis-fills fill0-only contours. So every edge is
    bucketed under its fill style (fill1 kept, fill0 reversed so fill0 and fill1
    edges of a style run oppositely), each bucket is stitched into closed
    contours, and one nonzero-winding path is emitted per style. nonzero (not
    even-odd) is the SWF rule: concentric same-side contours accumulate (a
    bordered emblem stays solid inside) while an oppositely-wound cutout cancels
    to a hole, so the Vault-76 "76" shows through a filled body.

    ``scale`` defaults to twips to pixels; ``background`` is the preview rect
    color, or None to omit it.
    """
    bx, by, bw, bh = shape.bounds
    vw = (bw - bx) * scale
    vh = (bh - by) * scale

    fill_buckets, line_buckets = _bucket_edges(shape)

    paths: list[str] = []
    for style, segs in fill_buckets.values():
        d = _contours_d(segs, scale, close=True)
        color = _fill_color(style)
        if not d or color is None:
            continue
        attrs = [f'd="{d}"', f'fill="{color.to_hex()}"']
        if color.a < 255:
            attrs.append(f'fill-opacity="{color.a / 255:.2f}"')
        attrs.append('fill-rule="nonzero"')
        paths.append(f'<path {" ".join(attrs)}/>')
    for style, segs in line_buckets.values():
        d = _contours_d(segs, scale, close=False)
        if not d:
            continue
        attrs = [f'd="{d}"', 'fill="none"',
                 f'stroke="{style.color.to_hex()}"',
                 f'stroke-width="{style.width * scale:.2f}"']
        paths.append(f'<path {" ".join(attrs)}/>')

    bg = ""
    if background:
        bg = (
            f'  <rect x="{bx * scale:.2f}" y="{by * scale:.2f}" '
            f'width="{vw:.2f}" height="{vh:.2f}" fill="{background}"/>\n'
        )

    svg_paths = "\n  ".join(paths)
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" '
        f'viewBox="{bx * scale:.2f} {by * scale:.2f} {vw:.2f} {vh:.2f}">\n'
        f'{bg}'
        f'  {svg_paths}\n'
        f'</svg>'
    )


def _bucket_edges(shape: ShapeDef):
    """Group every edge under the fill/line styles that touch it.

    Returns (fill_buckets, line_buckets), each an insertion-ordered dict
    ``id(style) -> (style, [segment, ...])``. fill0 edges are reversed so all of a
    style's edges run with the fill consistently on one side (needed to stitch
    closed contours); the quadratic control point is direction-agnostic, so it is
    carried unchanged through a reversal.
    """
    active_fills: list[FillStyle] = list(shape.fill_styles)
    active_lines: list[LineStyle | LineStyle2] = list(shape.line_styles)
    fill_buckets: dict[int, tuple] = {}
    line_buckets: dict[int, tuple] = {}
    f0 = f1 = ln = 0
    x = y = 0

    def add(buckets, style, seg):
        buckets.setdefault(id(style), (style, []))[1].append(seg)

    def emit(sx, sy, ex, ey, cx, cy):
        if 0 < f1 <= len(active_fills):
            add(fill_buckets, active_fills[f1 - 1], (sx, sy, ex, ey, cx, cy))
        if 0 < f0 <= len(active_fills):
            add(fill_buckets, active_fills[f0 - 1], (ex, ey, sx, sy, cx, cy))
        if 0 < ln <= len(active_lines):
            add(line_buckets, active_lines[ln - 1], (sx, sy, ex, ey, cx, cy))

    for rec in shape.records:
        if isinstance(rec, StyleChange):
            if rec.has_new_styles:
                active_fills = list(rec.new_fill_styles or [])
                active_lines = list(rec.new_line_styles or [])
                f0 = f1 = ln = 0
            if rec.has_move:
                x, y = rec.move_x, rec.move_y
            if rec.fill0 is not None:
                f0 = rec.fill0
            if rec.fill1 is not None:
                f1 = rec.fill1
            if rec.line is not None:
                ln = rec.line
        elif isinstance(rec, StraightEdge):
            nx, ny = x + rec.dx, y + rec.dy
            emit(x, y, nx, ny, None, None)
            x, y = nx, ny
        elif isinstance(rec, CurvedEdge):
            ctrlx, ctrly = x + rec.cx, y + rec.cy
            nx, ny = ctrlx + rec.ax, ctrly + rec.ay
            emit(x, y, nx, ny, ctrlx, ctrly)
            x, y = nx, ny
        # EndShape: nothing to bucket

    return fill_buckets, line_buckets


def _stitch(segments: list[_Segment]) -> list[list[_Segment]]:
    """Chain edges into contours by matching exact (integer-twip) endpoints."""
    starts: dict[tuple[int, int], list[int]] = {}
    for i, s in enumerate(segments):
        starts.setdefault((s[0], s[1]), []).append(i)
    used = [False] * len(segments)
    loops: list[list[_Segment]] = []
    for i in range(len(segments)):
        if used[i]:
            continue
        loop: list[_Segment] = []
        cur: int | None = i
        while cur is not None and not used[cur]:
            used[cur] = True
            s = segments[cur]
            loop.append(s)
            nxt = None
            for j in starts.get((s[2], s[3]), ()):
                if not used[j]:
                    nxt = j
                    break
            cur = nxt
        loops.append(loop)
    return loops


def _contours_d(segments: list[_Segment], scale: float, close: bool) -> str:
    """Build an SVG path `d` for stitched contours. Fills force-close each loop;
    strokes close only a loop whose chain returns to its start."""
    parts: list[str] = []
    for loop in _stitch(segments):
        sx, sy = loop[0][0], loop[0][1]
        d = [f"M {sx * scale:.2f} {sy * scale:.2f}"]
        for s in loop:
            ex, ey, cx, cy = s[2], s[3], s[4], s[5]
            if cx is None:
                d.append(f"L {ex * scale:.2f} {ey * scale:.2f}")
            else:
                d.append(f"Q {cx * scale:.2f} {cy * scale:.2f} {ex * scale:.2f} {ey * scale:.2f}")
        if close or (loop[-1][2], loop[-1][3]) == (loop[0][0], loop[0][1]):
            d.append("Z")
        parts.append(" ".join(d))
    return " ".join(parts)


def _fill_color(style: FillStyle) -> RGBA | None:
    """Solid color, or a gradient's middle stop as a flat approximation; None for
    bitmap fills (which we can't represent as a flat SVG fill)."""
    if style.color is not None:
        return style.color
    if style.gradient and style.gradient.records:
        recs = style.gradient.records
        return recs[len(recs) // 2].color
    return None


def svg_to_shapes(svg_content: str) -> list[ShapeDef]:
    """Parse SVG content into SWF ShapeDefs.

    Handles: <path>, <rect>, <circle>, <ellipse>, <line>, <polygon>.
    Groups (<g>) are flattened. Unsupported elements are skipped.
    """
    root = ET.fromstring(svg_content)
    ns = {"svg": "http://www.w3.org/2000/svg"}
    shapes: list[ShapeDef] = []
    shape_id_counter = 1

    # Process all elements recursively
    for elem in _iter_elements(root, ns):
        tag = _strip_ns(elem.tag)
        fill_color = _parse_fill(elem.get("fill", "white"))
        fill_style = FillStyle(fill_type=0x00, color=fill_color)
        records: list[ShapeRecord] = []

        if tag == "path":
            d = elem.get("d", "")
            records = _parse_svg_path_d(d)
        elif tag == "rect":
            records = _svg_rect_to_records(elem)
        elif tag == "circle":
            records = _svg_circle_to_records(elem)
        elif tag == "ellipse":
            records = _svg_ellipse_to_records(elem)
        else:
            continue

        if not records:
            continue

        # Prepend style change with fill
        records.insert(0, StyleChange(fill1=1))
        records.append(EndShape())

        # Compute bounds from records
        bounds = _compute_bounds(records)

        shapes.append(ShapeDef(
            shape_id=shape_id_counter,
            bounds=bounds,
            fill_styles=[fill_style],
            line_styles=[],
            records=records,
            shape_version=3,
        ))
        shape_id_counter += 1

    return shapes


def _iter_elements(root: ET.Element, ns: dict) -> list[ET.Element]:
    """Recursively yield drawable SVG elements."""
    elements = []
    for child in root:
        tag = _strip_ns(child.tag)
        if tag == "g":
            elements.extend(_iter_elements(child, ns))
        elif tag in ("path", "rect", "circle", "ellipse", "line", "polygon"):
            elements.append(child)
    return elements


def _strip_ns(tag: str) -> str:
    if "}" in tag:
        return tag.split("}")[-1]
    return tag


def _parse_fill(fill_str: str) -> RGBA:
    if not fill_str or fill_str == "none":
        return RGBA(0, 0, 0, 0)
    if fill_str == "white":
        return RGBA(255, 255, 255, 255)
    if fill_str == "black":
        return RGBA(0, 0, 0, 255)
    if fill_str.startswith("#"):
        return RGBA.from_hex(fill_str)
    return RGBA(255, 255, 255, 255)


def _parse_svg_path_d(d: str) -> list[ShapeRecord]:
    """Parse SVG path `d` attribute into shape records.

    Converts cubic beziers (C) to quadratic via midpoint approximation.
    """
    records: list[ShapeRecord] = []
    tokens = re.findall(r"[MmLlHhVvCcSsQqTtAaZz]|[-+]?[0-9]*\.?[0-9]+", d)
    i = 0
    cx, cy = 0.0, 0.0
    start_x, start_y = 0.0, 0.0

    def _next_float() -> float:
        nonlocal i
        val = float(tokens[i])
        i += 1
        return val

    while i < len(tokens):
        cmd = tokens[i]
        if cmd.isalpha():
            i += 1
        else:
            cmd = "L"  # implicit lineto

        if cmd == "M":
            x, y = _next_float(), _next_float()
            twx, twy = int(x * TWIPS_PER_PIXEL), int(y * TWIPS_PER_PIXEL)
            records.append(StyleChange(move_x=twx, move_y=twy))
            cx, cy = x, y
            start_x, start_y = x, y
        elif cmd == "m":
            dx, dy = _next_float(), _next_float()
            cx += dx
            cy += dy
            twx, twy = int(cx * TWIPS_PER_PIXEL), int(cy * TWIPS_PER_PIXEL)
            records.append(StyleChange(move_x=twx, move_y=twy))
            start_x, start_y = cx, cy
        elif cmd == "L":
            x, y = _next_float(), _next_float()
            dx = int((x - cx) * TWIPS_PER_PIXEL)
            dy = int((y - cy) * TWIPS_PER_PIXEL)
            records.append(StraightEdge(dx=dx, dy=dy))
            cx, cy = x, y
        elif cmd == "l":
            dx, dy = _next_float(), _next_float()
            records.append(StraightEdge(
                dx=int(dx * TWIPS_PER_PIXEL),
                dy=int(dy * TWIPS_PER_PIXEL),
            ))
            cx += dx
            cy += dy
        elif cmd == "H":
            x = _next_float()
            records.append(StraightEdge(dx=int((x - cx) * TWIPS_PER_PIXEL), dy=0))
            cx = x
        elif cmd == "h":
            dx = _next_float()
            records.append(StraightEdge(dx=int(dx * TWIPS_PER_PIXEL), dy=0))
            cx += dx
        elif cmd == "V":
            y = _next_float()
            records.append(StraightEdge(dx=0, dy=int((y - cy) * TWIPS_PER_PIXEL)))
            cy = y
        elif cmd == "v":
            dy = _next_float()
            records.append(StraightEdge(dx=0, dy=int(dy * TWIPS_PER_PIXEL)))
            cy += dy
        elif cmd == "C":
            # Cubic bezier -> approximate as quadratic
            c1x, c1y = _next_float(), _next_float()
            c2x, c2y = _next_float(), _next_float()
            ex, ey = _next_float(), _next_float()
            _records = _cubic_to_quadratics(cx, cy, c1x, c1y, c2x, c2y, ex, ey)
            records.extend(_records)
            cx, cy = ex, ey
        elif cmd == "c":
            c1x, c1y = _next_float(), _next_float()
            c2x, c2y = _next_float(), _next_float()
            dx, dy = _next_float(), _next_float()
            _records = _cubic_to_quadratics(
                cx, cy, cx + c1x, cy + c1y, cx + c2x, cy + c2y, cx + dx, cy + dy
            )
            records.extend(_records)
            cx += dx
            cy += dy
        elif cmd == "Q":
            qcx, qcy = _next_float(), _next_float()
            ex, ey = _next_float(), _next_float()
            records.append(CurvedEdge(
                cx=int((qcx - cx) * TWIPS_PER_PIXEL),
                cy=int((qcy - cy) * TWIPS_PER_PIXEL),
                ax=int((ex - qcx) * TWIPS_PER_PIXEL),
                ay=int((ey - qcy) * TWIPS_PER_PIXEL),
            ))
            cx, cy = ex, ey
        elif cmd == "q":
            dcx, dcy = _next_float(), _next_float()
            dx, dy = _next_float(), _next_float()
            records.append(CurvedEdge(
                cx=int(dcx * TWIPS_PER_PIXEL),
                cy=int(dcy * TWIPS_PER_PIXEL),
                ax=int((dx - dcx) * TWIPS_PER_PIXEL),
                ay=int((dy - dcy) * TWIPS_PER_PIXEL),
            ))
            cx += dx
            cy += dy
        elif cmd in ("Z", "z"):
            dx = int((start_x - cx) * TWIPS_PER_PIXEL)
            dy = int((start_y - cy) * TWIPS_PER_PIXEL)
            if dx != 0 or dy != 0:
                records.append(StraightEdge(dx=dx, dy=dy))
            cx, cy = start_x, start_y
        else:
            # Skip unsupported commands (A, S, T, etc.)
            i += 1

    return records


def _cubic_to_quadratics(
    x0: float, y0: float,
    c1x: float, c1y: float,
    c2x: float, c2y: float,
    x3: float, y3: float,
    tolerance: float = 0.5,
) -> list[CurvedEdge]:
    """Approximate cubic bezier with one or more quadratic beziers.

    Uses midpoint approximation. Subdivides recursively if error > tolerance.
    """
    # Single quadratic approximation: control = (3*(c1+c2) - (p0+p3)) / 4
    qx = (3 * (c1x + c2x) - (x0 + x3)) / 4
    qy = (3 * (c1y + c2y) - (y0 + y3)) / 4

    # Measure error: distance from cubic midpoint to quadratic midpoint
    cubic_mid_x = (x0 + 3 * c1x + 3 * c2x + x3) / 8
    cubic_mid_y = (y0 + 3 * c1y + 3 * c2y + y3) / 8
    quad_mid_x = (x0 + 2 * qx + x3) / 4
    quad_mid_y = (y0 + 2 * qy + y3) / 4
    error = ((cubic_mid_x - quad_mid_x) ** 2 + (cubic_mid_y - quad_mid_y) ** 2) ** 0.5

    if error <= tolerance:
        return [CurvedEdge(
            cx=int((qx - x0) * TWIPS_PER_PIXEL),
            cy=int((qy - y0) * TWIPS_PER_PIXEL),
            ax=int((x3 - qx) * TWIPS_PER_PIXEL),
            ay=int((y3 - qy) * TWIPS_PER_PIXEL),
        )]

    # Subdivide at t=0.5 (de Casteljau)
    m01x = (x0 + c1x) / 2;  m01y = (y0 + c1y) / 2
    m12x = (c1x + c2x) / 2; m12y = (c1y + c2y) / 2
    m23x = (c2x + x3) / 2;  m23y = (c2y + y3) / 2
    m012x = (m01x + m12x) / 2; m012y = (m01y + m12y) / 2
    m123x = (m12x + m23x) / 2; m123y = (m12y + m23y) / 2
    mx = (m012x + m123x) / 2;  my = (m012y + m123y) / 2

    left = _cubic_to_quadratics(x0, y0, m01x, m01y, m012x, m012y, mx, my, tolerance)
    right = _cubic_to_quadratics(mx, my, m123x, m123y, m23x, m23y, x3, y3, tolerance)
    return left + right


def _svg_rect_to_records(elem: ET.Element) -> list[ShapeRecord]:
    x = float(elem.get("x", "0"))
    y = float(elem.get("y", "0"))
    w = float(elem.get("width", "0"))
    h = float(elem.get("height", "0"))
    tw = TWIPS_PER_PIXEL
    return [
        StyleChange(move_x=int(x * tw), move_y=int(y * tw)),
        StraightEdge(dx=int(w * tw), dy=0),
        StraightEdge(dx=0, dy=int(h * tw)),
        StraightEdge(dx=int(-w * tw), dy=0),
        StraightEdge(dx=0, dy=int(-h * tw)),
    ]


def _svg_circle_to_records(elem: ET.Element) -> list[ShapeRecord]:
    cx = float(elem.get("cx", "0"))
    cy = float(elem.get("cy", "0"))
    r = float(elem.get("r", "0"))
    return _svg_ellipse_records(cx, cy, r, r)


def _svg_ellipse_to_records(elem: ET.Element) -> list[ShapeRecord]:
    cx = float(elem.get("cx", "0"))
    cy = float(elem.get("cy", "0"))
    rx = float(elem.get("rx", "0"))
    ry = float(elem.get("ry", "0"))
    return _svg_ellipse_records(cx, cy, rx, ry)


def _svg_ellipse_records(cx: float, cy: float, rx: float, ry: float) -> list[ShapeRecord]:
    """Approximate ellipse with 4 quadratic bezier segments."""
    tw = TWIPS_PER_PIXEL
    # Kappa for quadratic approximation of quarter circle
    k = 0.4142
    records: list[ShapeRecord] = [
        StyleChange(move_x=int((cx + rx) * tw), move_y=int(cy * tw)),
    ]
    # 4 quadrant control/end points (start at right, go clockwise)
    quadrants = [
        ((cx + rx, cy + ry * k), (cx, cy + ry)),      # right -> bottom
        ((cx - rx * k, cy + ry), (cx - rx, cy)),       # bottom -> left
        ((cx - rx, cy - ry * k), (cx, cy - ry)),       # left -> top
        ((cx + rx * k, cy - ry), (cx + rx, cy)),       # top -> right
    ]
    prev_x, prev_y = cx + rx, cy
    for ctrl, end in quadrants:
        records.append(CurvedEdge(
            cx=int((ctrl[0] - prev_x) * tw),
            cy=int((ctrl[1] - prev_y) * tw),
            ax=int((end[0] - ctrl[0]) * tw),
            ay=int((end[1] - ctrl[1]) * tw),
        ))
        prev_x, prev_y = end
    return records


def _compute_bounds(records: list[ShapeRecord]) -> tuple[int, int, int, int]:
    """Compute approximate bounding box from shape records."""
    xs, ys = [], []
    cx, cy = 0, 0
    for rec in records:
        if isinstance(rec, StyleChange) and rec.has_move:
            cx, cy = rec.move_x, rec.move_y
            xs.append(cx)
            ys.append(cy)
        elif isinstance(rec, StraightEdge):
            cx += rec.dx
            cy += rec.dy
            xs.append(cx)
            ys.append(cy)
        elif isinstance(rec, CurvedEdge):
            xs.append(cx + rec.cx)
            ys.append(cy + rec.cy)
            cx += rec.cx + rec.ax
            cy += rec.cy + rec.ay
            xs.append(cx)
            ys.append(cy)
    if not xs:
        return (0, 0, 0, 0)
    return (min(xs), min(ys), max(xs), max(ys))
