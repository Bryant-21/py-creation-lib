"""SWF binary parser -- reads .swf files into SwfDocument.

Supports FWS (uncompressed), CWS (zlib), and ZWS (LZMA).
Fallout 4 uses CWS (zlib) compression, SWF version 17.
"""
from __future__ import annotations

import struct
import zlib
from dataclasses import dataclass, field
from pathlib import Path

from creation_lib.swf.types import BitReader, RECT, RGBA
from creation_lib.swf.tags import (
    SwfTag, RawTag, EndTag, ShowFrameTag, SetBackgroundColorTag,
    DefineShapeTag, PlaceObject2Tag, RemoveObject2Tag,
    DefineSpriteTag, FrameLabelTag, SymbolClassTag,
    TAG_END, TAG_SHOW_FRAME, TAG_PLACE_OBJECT2, TAG_REMOVE_OBJECT2,
    TAG_DEFINE_SPRITE, DEFINE_SHAPE_IDS,
    parse_tag_body,
)
from creation_lib.swf.shapes import ShapeDef
from creation_lib.swf.timeline import Timeline, Frame, SpriteDef


@dataclass
class SwfHeader:
    compression: str   # "FWS", "CWS", "ZWS"
    version: int
    file_length: int   # uncompressed total length
    frame_size: RECT
    fps: float
    frame_count: int


@dataclass
class SwfDocument:
    """Parsed SWF document."""
    header: SwfHeader
    background_color: RGBA = field(default_factory=lambda: RGBA(0x33, 0x33, 0x33, 255))
    main_timeline: Timeline = field(default_factory=Timeline)
    shapes: dict[int, ShapeDef] = field(default_factory=dict)       # character_id -> ShapeDef
    sprites: dict[int, SpriteDef] = field(default_factory=dict)     # character_id -> SpriteDef
    symbols: list[tuple[int, str]] = field(default_factory=list)    # (character_id, class name)
    tags: list[SwfTag] = field(default_factory=list)                # all tags in order
    raw_tags: list[RawTag] = field(default_factory=list)            # unparsed tags only


def parse_swf(data: bytes | bytearray) -> SwfDocument:
    """Parse SWF binary data into a SwfDocument."""
    if len(data) < 8:
        raise ValueError("Data too short to be a valid SWF file")

    sig = data[0:3].decode("ascii")
    if sig not in ("FWS", "CWS", "ZWS"):
        raise ValueError(f"Invalid SWF signature: {sig!r}")

    version = data[3]
    file_length = struct.unpack_from("<I", data, 4)[0]

    # Decompress body (everything after the 8-byte header prefix)
    if sig == "CWS":
        body = zlib.decompress(data[8:])
    elif sig == "ZWS":
        import lzma
        body = lzma.decompress(data[12:])  # ZWS has 4 extra bytes for compressed length
    else:
        body = data[8:]

    reader = BitReader(body)

    # Parse RECT (frame size)
    frame_size = RECT.parse(reader)
    reader.align()

    # FPS: 8.8 fixed point
    fps_raw = reader.read_ui16()
    fps = fps_raw >> 8  # integer part

    frame_count = reader.read_ui16()

    header = SwfHeader(
        compression=sig,
        version=version,
        file_length=file_length,
        frame_size=frame_size,
        fps=fps,
        frame_count=frame_count,
    )

    # Parse tag stream
    tag_data = body[reader.tell():]
    tags = _parse_tag_stream(tag_data)

    # Build document from parsed tags
    doc = SwfDocument(header=header)
    doc.tags = tags
    _build_document(doc, tags)

    return doc


def parse_swf_file(path: str | Path) -> SwfDocument:
    """Parse SWF from a file path."""
    return parse_swf(Path(path).read_bytes())


def _parse_tag_stream(data: bytes) -> list[SwfTag]:
    """Parse a sequence of SWF tags from raw bytes."""
    tags: list[SwfTag] = []
    pos = 0

    while pos < len(data):
        if pos + 2 > len(data):
            break

        tag_code_and_length = struct.unpack_from("<H", data, pos)[0]
        pos += 2

        tag_id = tag_code_and_length >> 6
        length = tag_code_and_length & 0x3F

        if length == 0x3F:
            if pos + 4 > len(data):
                break
            length = struct.unpack_from("<I", data, pos)[0]
            pos += 4

        tag_body = data[pos:pos + length]
        pos += length

        # For DefineSprite, pass recursive parser
        def nested_parser(nested_data: bytes) -> list[SwfTag]:
            return _parse_tag_stream(nested_data)

        tag = parse_tag_body(tag_id, tag_body,
                            tag_parser=nested_parser if tag_id == TAG_DEFINE_SPRITE else None)
        tags.append(tag)

        if tag_id == TAG_END:
            break

    return tags


def _build_document(doc: SwfDocument, tags: list[SwfTag]) -> None:
    """Populate document fields from parsed tags."""
    current_frame = Frame()

    for tag in tags:
        if isinstance(tag, SetBackgroundColorTag):
            doc.background_color = tag.color

        elif isinstance(tag, SymbolClassTag):
            doc.symbols.extend(tag.symbols)

        elif isinstance(tag, DefineShapeTag):
            doc.shapes[tag.shape.shape_id] = tag.shape

        elif isinstance(tag, DefineSpriteTag):
            sprite_timeline = Timeline()
            _build_sprite_timeline(sprite_timeline, tag.tags)
            doc.sprites[tag.sprite_id] = SpriteDef(
                sprite_id=tag.sprite_id,
                timeline=sprite_timeline,
            )

        elif isinstance(tag, PlaceObject2Tag):
            if tag.character_id is not None:
                from creation_lib.swf.types import MATRIX
                current_frame.place(
                    depth=tag.depth,
                    character_id=tag.character_id,
                    matrix=tag.matrix or MATRIX(),
                    name=tag.name,
                    color_transform=tag.color_transform,
                    ratio=tag.ratio,
                    clip_depth=tag.clip_depth,
                )
            elif tag.move and tag.matrix:
                current_frame.update_transform(tag.depth, tag.matrix)

        elif isinstance(tag, RemoveObject2Tag):
            current_frame.remove(tag.depth)

        elif isinstance(tag, FrameLabelTag):
            current_frame.label = tag.name

        elif isinstance(tag, ShowFrameTag):
            doc.main_timeline.frames.append(current_frame)
            current_frame = Frame()

        elif isinstance(tag, RawTag):
            doc.raw_tags.append(tag)


def _build_sprite_timeline(timeline: Timeline, tags: list[SwfTag]) -> None:
    """Build timeline from a DefineSprite's nested tags."""
    current_frame = Frame()
    for tag in tags:
        if isinstance(tag, PlaceObject2Tag):
            if tag.character_id is not None:
                from creation_lib.swf.types import MATRIX
                current_frame.place(
                    depth=tag.depth,
                    character_id=tag.character_id,
                    matrix=tag.matrix or MATRIX(),
                    name=tag.name,
                )
            elif tag.move and tag.matrix:
                current_frame.update_transform(tag.depth, tag.matrix)
        elif isinstance(tag, RemoveObject2Tag):
            current_frame.remove(tag.depth)
        elif isinstance(tag, FrameLabelTag):
            current_frame.label = tag.name
        elif isinstance(tag, ShowFrameTag):
            timeline.frames.append(current_frame)
            current_frame = Frame()
