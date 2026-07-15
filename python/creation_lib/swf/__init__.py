"""SWF parser/writer for Fallout 4 Pipboy icons."""

from creation_lib.swf.parser import parse_swf, parse_swf_file, SwfDocument, SwfHeader
from creation_lib.swf.writer import write_swf, write_swf_file
from creation_lib.swf.types import RECT, MATRIX, COLOR, RGBA, FillStyle, LineStyle
from creation_lib.swf.shapes import ShapeDef, StraightEdge, CurvedEdge, StyleChange, EndShape
from creation_lib.swf.timeline import Timeline, Frame, DisplayEntry, SpriteDef
from creation_lib.swf.tags import (
    SwfTag, RawTag, EndTag, ShowFrameTag, SetBackgroundColorTag,
    DefineShapeTag, PlaceObject2Tag, RemoveObject2Tag,
    DefineSpriteTag, FrameLabelTag, FileAttributesTag, MetadataTag,
)
from creation_lib.swf.svg_io import shape_to_svg, svg_to_shapes
from creation_lib.swf.trace import trace_image, trace_image_file, TraceSettings

__all__ = [
    "parse_swf", "parse_swf_file", "write_swf", "write_swf_file",
    "SwfDocument", "SwfHeader",
    "RECT", "MATRIX", "COLOR", "RGBA", "FillStyle", "LineStyle",
    "ShapeDef", "StraightEdge", "CurvedEdge", "StyleChange", "EndShape",
    "Timeline", "Frame", "DisplayEntry", "SpriteDef",
    "SwfTag", "RawTag", "EndTag", "ShowFrameTag", "SetBackgroundColorTag",
    "DefineShapeTag", "PlaceObject2Tag", "RemoveObject2Tag",
    "DefineSpriteTag", "FrameLabelTag", "FileAttributesTag", "MetadataTag",
    "shape_to_svg", "svg_to_shapes",
    "trace_image", "trace_image_file", "TraceSettings",
]
