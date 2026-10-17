"""Versioned vector-library geometry shared by indexing, preview and placement."""
from __future__ import annotations

import base64
import json
import math
import xml.etree.ElementTree as ET

from creation_lib.swf.shapes import ShapeDef, StyleChange, StraightEdge, CurvedEdge, EndShape
from creation_lib.swf.tags import DefineShapeTag
from creation_lib.swf.types import FillStyle, RGBA

MAX_ASSET_BYTES = 16 * 1024 * 1024
FORMAT = "swf-library-v2"
SHAPE_CODES = {1: 2, 2: 22, 3: 32, 4: 83}


def encoded_geometry(shape: ShapeDef) -> dict:
    code = SHAPE_CODES[shape.shape_version]
    return {"format": FORMAT, "tag_code": code,
            "tag_data": base64.b64encode(DefineShapeTag(shape, code).to_bytes()).decode("ascii")}


def decode_geometry(blob: bytes, svg: str = "") -> tuple[ShapeDef, str]:
    """Decode authoritative tags or recover generated legacy SVG without its background."""
    if len(blob) > MAX_ASSET_BYTES or len(svg) > MAX_ASSET_BYTES:
        raise ValueError("Library geometry exceeds the 16 MB asset limit")
    data = json.loads(blob) if blob else {}
    if not isinstance(data, dict):
        raise ValueError('Library geometry must contain an object')
    warning = ""
    if data.get("format") == FORMAT:
        code = data["tag_code"]
        if code not in SHAPE_CODES.values():
            raise ValueError("Library entry does not contain a vector definition")
        shape = DefineShapeTag.parse(base64.b64decode(data["tag_data"], validate=True), code).shape
    elif svg:
        root = ET.fromstring(svg)
        children = list(root)
        # Old index export always prepended this preview-only background rect.
        if children and children[0].tag.rsplit("}", 1)[-1] == "rect" and children[0].get("fill") == "#333333":
            child = children[0]
            view = [float(v) for v in root.get("viewBox", "").split()]
            rectangle = [float(child.get(n, "0")) for n in ("x", "y", "width", "height")]
            if len(view) == 4 and all(abs(a - b) < .011 for a, b in zip(view, rectangle)):
                root.remove(child)
        if any(element.get("stroke", "none") != "none" for element in root.iter()):
            raise ValueError("Legacy index omitted stroke geometry; rebuild this library in Settings > Indexes")
        from creation_lib.swf.svg_io import svg_to_shapes
        from creation_lib.swf.project import _merge_shapes
        shapes = svg_to_shapes(ET.tostring(root, encoding="unicode"))
        if not shapes:
            raise ValueError("Library entry has no drawable vector paths")
        shape = _merge_shapes(shapes, 1)
        warning = "Legacy SVG recovery: source gradients may be flattened. Rebuild the index for full geometry."
    else:
        fills = []
        for style in data.get("fill_styles", []):
            if style.get("type", 0) != 0:
                raise ValueError("Legacy gradient data is incomplete; rebuild this library")
            color = style["color"].lstrip("#")
            if len(color) not in {6, 8}:
                raise ValueError("Invalid indexed fill color")
            fills.append(FillStyle(0, RGBA(int(color[:2], 16), int(color[2:4], 16), int(color[4:6], 16),
                                         int(color[6:8], 16) if len(color) == 8 else 255)))
        records = []
        for rec in data.get("records", []):
            kind = rec["type"]
            if kind == "sc":
                records.append(StyleChange(move_x=rec.get("dx", 0) if rec.get("move") else None,
                                           move_y=rec.get("dy", 0) if rec.get("move") else None,
                                           fill0=rec.get("fill0"), fill1=rec.get("fill1")))
            elif kind == "se":
                records.append(StraightEdge(rec["dx"], rec["dy"]))
            elif kind == "ce":
                records.append(CurvedEdge(rec["cx"], rec["cy"], rec["ax"], rec["ay"]))
            elif kind == "end":
                records.append(EndShape())
            else:
                raise ValueError("Unrecognized indexed shape record")
        bounds = data.get("bounds", [0, 0, 0, 0])
        shape = ShapeDef(1, tuple(round(v * 20) for v in bounds), fills, [], records, shape_version=3)
        warning = "Legacy geometry: alpha/strokes/gradients may be missing. Rebuild the index for full geometry."
    validate_geometry(shape)
    return shape, warning


def validate_geometry(shape: ShapeDef):
    if len(shape.bounds) != 4 or not all(math.isfinite(v) for v in shape.bounds):
        raise ValueError("Invalid vector bounds")
    if shape.bounds[2] <= shape.bounds[0] or shape.bounds[3] <= shape.bounds[1]:
        raise ValueError("Vector has empty bounds")
    if not shape.records or not isinstance(shape.records[-1], EndShape):
        raise ValueError("Vector has no complete shape records")
    if not any((isinstance(r, StraightEdge) and (r.dx or r.dy)) or
               (isinstance(r, CurvedEdge) and any((r.cx, r.cy, r.ax, r.ay))) for r in shape.records):
        raise ValueError('Vector has no drawable edges')
    fills = list(shape.fill_styles)
    lines = list(shape.line_styles)
    for record in shape.records:
        if isinstance(record, StyleChange) and record.new_fill_styles:
            fills.extend(record.new_fill_styles)
        if isinstance(record, StyleChange) and record.new_line_styles:
            lines.extend(record.new_line_styles)
    fills.extend(line.fill_style for line in lines if getattr(line, 'fill_style', None))
    if any(style.fill_type in {0x40, 0x41, 0x42, 0x43} for style in fills):
        raise ValueError("Bitmap fill needs source movie assets; reuse its exported symbol in Document > Structure")
    if any(style.fill_type not in {0, 0x10, 0x12, 0x13} for style in fills):
        raise ValueError('Unsupported vector fill style')
    if not fills and not lines:
        raise ValueError("Vector has no fill or stroke styles")
