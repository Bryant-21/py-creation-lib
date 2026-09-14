"""SWF tag definitions for the ~12 tags Fallout 4 uses.

Unknown tags are preserved as RawTag for lossless round-trip.
"""
from __future__ import annotations

from dataclasses import dataclass, field

from creation_lib.swf.types import (
    BitReader, BitWriter, RECT, MATRIX, COLOR, RGBA, FillStyle, LineStyle,
    parse_fill_styles, parse_line_styles, write_fill_styles, write_line_styles,
    _sbits_needed,
)
from creation_lib.swf.shapes import (
    ShapeDef, ShapeRecord, parse_shape_records, write_shape_records, _ubits_needed,
)

# Tag IDs
TAG_END = 0
TAG_SHOW_FRAME = 1
TAG_DEFINE_SHAPE = 2
TAG_PLACE_OBJECT = 4
TAG_REMOVE_OBJECT = 5
TAG_SET_BG_COLOR = 9
TAG_DEFINE_SHAPE2 = 22
TAG_PLACE_OBJECT2 = 26
TAG_REMOVE_OBJECT2 = 28
TAG_DEFINE_SHAPE3 = 32
TAG_DEFINE_SPRITE = 39
TAG_FRAME_LABEL = 43
TAG_FILE_ATTRIBUTES = 69
TAG_SYMBOL_CLASS = 76
TAG_METADATA = 77
# DoABCDefine has no dataclass on purpose: ABC bytes are emitted and read by the
# native crate, so they stay a RawTag here rather than passing through this
# byte-lossy codec.
TAG_DO_ABC = 82
TAG_DEFINE_SHAPE4 = 83

DEFINE_SHAPE_IDS = {TAG_DEFINE_SHAPE, TAG_DEFINE_SHAPE2, TAG_DEFINE_SHAPE3, TAG_DEFINE_SHAPE4}
SHAPE_VERSION_MAP = {
    TAG_DEFINE_SHAPE: 1,
    TAG_DEFINE_SHAPE2: 2,
    TAG_DEFINE_SHAPE3: 3,
    TAG_DEFINE_SHAPE4: 4,
}


@dataclass
class RawTag:
    """Unparsed tag -- preserved for lossless round-trip."""
    tag_id: int
    data: bytes


@dataclass
class EndTag:
    tag_id: int = TAG_END


@dataclass
class ShowFrameTag:
    tag_id: int = TAG_SHOW_FRAME


@dataclass
class SetBackgroundColorTag:
    color: RGBA
    tag_id: int = TAG_SET_BG_COLOR

    @classmethod
    def parse(cls, data: bytes) -> "SetBackgroundColorTag":
        reader = BitReader(data)
        c = COLOR.parse(reader)
        return cls(color=RGBA(c.r, c.g, c.b, 255))

    def to_bytes(self) -> bytes:
        writer = BitWriter()
        COLOR(self.color.r, self.color.g, self.color.b).write(writer)
        return writer.getvalue()


@dataclass
class FileAttributesTag:
    has_metadata: bool = False
    action_script3: bool = False
    use_network: bool = False
    use_direct_blit: bool = False
    use_gpu: bool = False
    _raw_flags: int = 0  # preserve original flags for lossless round-trip
    tag_id: int = TAG_FILE_ATTRIBUTES

    @classmethod
    def parse(cls, data: bytes) -> "FileAttributesTag":
        if len(data) < 4:
            return cls()
        reader = BitReader(data)
        flags = reader.read_ui32()
        return cls(
            use_direct_blit=bool(flags & (1 << 6)),
            use_gpu=bool(flags & (1 << 5)),
            has_metadata=bool(flags & (1 << 4)),
            action_script3=bool(flags & (1 << 3)),
            use_network=bool(flags & 1),
            _raw_flags=flags,
        )

    def to_bytes(self) -> bytes:
        # If raw flags were captured on parse, reproduce them exactly
        # to preserve any unknown/ambiguous bits across SWF spec versions.
        if self._raw_flags:
            writer = BitWriter()
            writer.write_ui32(self._raw_flags)
            return writer.getvalue()
        flags = 0
        if self.use_direct_blit:
            flags |= (1 << 6)
        if self.use_gpu:
            flags |= (1 << 5)
        if self.has_metadata:
            flags |= (1 << 4)
        if self.action_script3:
            flags |= (1 << 3)
        if self.use_network:
            flags |= 1
        writer = BitWriter()
        writer.write_ui32(flags)
        return writer.getvalue()


@dataclass
class SymbolClassTag:
    """SymbolClass -- binds character IDs to AS3 class names.

    This is the only handle anything outside the SWF has on a character: FO4
    resolves marker icons and HUD widgets by export name, and the native
    splicer refuses to inject into a file that has no SymbolClass tag.
    """
    symbols: list[tuple[int, str]] = field(default_factory=list)  # (character_id, class_name)
    tag_id: int = TAG_SYMBOL_CLASS

    @classmethod
    def parse(cls, data: bytes) -> "SymbolClassTag":
        if len(data) < 2:
            return cls()
        reader = BitReader(data)
        count = reader.read_ui16()
        symbols = []
        for _ in range(count):
            character_id = reader.read_ui16()
            symbols.append((character_id, reader.read_string()))
        return cls(symbols=symbols)

    def to_bytes(self) -> bytes:
        writer = BitWriter()
        writer.write_ui16(len(self.symbols))
        for character_id, name in self.symbols:
            writer.write_ui16(character_id)
            writer.write_string(name)
        return writer.getvalue()


@dataclass
class MetadataTag:
    xml: str
    tag_id: int = TAG_METADATA

    @classmethod
    def parse(cls, data: bytes) -> "MetadataTag":
        return cls(xml=data.rstrip(b"\x00").decode("utf-8", errors="replace"))

    def to_bytes(self) -> bytes:
        return self.xml.encode("utf-8") + b"\x00"


@dataclass
class DefineShapeTag:
    """DefineShape 1-4 -- vector shape definition."""
    shape: ShapeDef
    tag_id: int = TAG_DEFINE_SHAPE

    @classmethod
    def parse(cls, data: bytes, shape_tag_id: int) -> "DefineShapeTag":
        version = SHAPE_VERSION_MAP[shape_tag_id]
        has_alpha = version >= 3
        reader = BitReader(data)

        shape_id = reader.read_ui16()
        bounds = RECT.parse(reader)
        reader.align()

        edge_bounds = None
        if version >= 4:
            edge_bounds = RECT.parse(reader)
            reader.align()
            _flags = reader.read_ui8()  # UsesFillWindingRule, UsesNonScalingStrokes, UsesScalingStrokes

        fill_styles = parse_fill_styles(reader, has_alpha)
        line_styles = parse_line_styles(reader, version, has_alpha)

        num_fill_bits = reader.read_ubits(4)
        num_line_bits = reader.read_ubits(4)
        records = parse_shape_records(reader, num_fill_bits, num_line_bits, version)

        shape = ShapeDef(
            shape_id=shape_id,
            bounds=(bounds.xmin, bounds.ymin, bounds.xmax, bounds.ymax),
            fill_styles=fill_styles,
            line_styles=line_styles,
            records=records,
            shape_version=version,
            edge_bounds=(edge_bounds.xmin, edge_bounds.ymin, edge_bounds.xmax, edge_bounds.ymax) if edge_bounds else None,
        )
        return cls(shape=shape, tag_id=shape_tag_id)

    def to_bytes(self) -> bytes:
        s = self.shape
        version = s.shape_version
        has_alpha = version >= 3
        writer = BitWriter()

        writer.write_ui16(s.shape_id)
        RECT(s.bounds[0], s.bounds[2], s.bounds[1], s.bounds[3]).write(writer)
        writer.align()

        if version >= 4 and s.edge_bounds:
            RECT(s.edge_bounds[0], s.edge_bounds[2], s.edge_bounds[1], s.edge_bounds[3]).write(writer)
            writer.align()
            writer.write_ui8(0)  # flags

        write_fill_styles(writer, s.fill_styles, has_alpha)
        write_line_styles(writer, s.line_styles, version, has_alpha)

        nfb = _ubits_needed(len(s.fill_styles))
        nlb = _ubits_needed(len(s.line_styles))
        writer.write_ubits(4, nfb)
        writer.write_ubits(4, nlb)
        write_shape_records(writer, s.records, nfb, nlb, version)
        writer.align()

        return writer.getvalue()


@dataclass
class PlaceObject2Tag:
    """PlaceObject2 -- place or update a character on the display list."""
    depth: int
    character_id: int | None = None
    matrix: MATRIX | None = None
    color_transform: bytes | None = None  # opaque for now
    ratio: int | None = None
    name: str | None = None
    clip_depth: int | None = None
    clip_actions: bytes | None = None  # opaque
    move: bool = False
    tag_id: int = TAG_PLACE_OBJECT2

    @classmethod
    def parse(cls, data: bytes) -> "PlaceObject2Tag":
        reader = BitReader(data)
        flags = reader.read_ui8()
        has_clip_actions = bool(flags & 0x80)
        has_clip_depth = bool(flags & 0x40)
        has_name = bool(flags & 0x20)
        has_ratio = bool(flags & 0x10)
        has_color_transform = bool(flags & 0x08)
        has_matrix = bool(flags & 0x04)
        has_character = bool(flags & 0x02)
        move = bool(flags & 0x01)

        depth = reader.read_ui16()
        char_id = reader.read_ui16() if has_character else None
        matrix = MATRIX.parse(reader) if has_matrix else None
        reader.align()

        # Color transform -- parse opaquely for round-trip
        ct_data = None
        if has_color_transform:
            ct_start = reader.tell()
            _has_add = reader.read_ubits(1)
            _has_mult = reader.read_ubits(1)
            nbits = reader.read_ubits(4)
            field_count = 0
            if _has_mult:
                field_count += 4
            if _has_add:
                field_count += 4
            for _ in range(field_count):
                reader.read_sbits(nbits)
            reader.align()
            ct_end = reader.tell()
            ct_data = data[ct_start:ct_end]

        ratio = reader.read_ui16() if has_ratio else None
        name = reader.read_string() if has_name else None
        clip_depth = reader.read_ui16() if has_clip_depth else None
        clip_acts = None
        if has_clip_actions:
            clip_acts = data[reader.tell():]

        return cls(
            depth=depth, character_id=char_id, matrix=matrix,
            color_transform=ct_data, ratio=ratio, name=name,
            clip_depth=clip_depth, clip_actions=clip_acts, move=move,
        )

    def to_bytes(self) -> bytes:
        writer = BitWriter()
        flags = 0
        if self.clip_actions:
            flags |= 0x80
        if self.clip_depth is not None:
            flags |= 0x40
        if self.name is not None:
            flags |= 0x20
        if self.ratio is not None:
            flags |= 0x10
        if self.color_transform:
            flags |= 0x08
        if self.matrix:
            flags |= 0x04
        if self.character_id is not None:
            flags |= 0x02
        if self.move:
            flags |= 0x01
        writer.write_ui8(flags)
        writer.write_ui16(self.depth)
        if self.character_id is not None:
            writer.write_ui16(self.character_id)
        if self.matrix:
            self.matrix.write(writer)
            writer.align()
        if self.color_transform:
            writer.write_bytes(self.color_transform)
        if self.ratio is not None:
            writer.write_ui16(self.ratio)
        if self.name is not None:
            writer.write_string(self.name)
        if self.clip_depth is not None:
            writer.write_ui16(self.clip_depth)
        if self.clip_actions:
            writer.write_bytes(self.clip_actions)
        return writer.getvalue()


@dataclass
class RemoveObject2Tag:
    """RemoveObject2 -- remove character from display list by depth."""
    depth: int
    tag_id: int = TAG_REMOVE_OBJECT2

    @classmethod
    def parse(cls, data: bytes) -> "RemoveObject2Tag":
        reader = BitReader(data)
        return cls(depth=reader.read_ui16())

    def to_bytes(self) -> bytes:
        writer = BitWriter()
        writer.write_ui16(self.depth)
        return writer.getvalue()


@dataclass
class FrameLabelTag:
    """Named frame marker."""
    name: str
    tag_id: int = TAG_FRAME_LABEL

    @classmethod
    def parse(cls, data: bytes) -> "FrameLabelTag":
        return cls(name=data.rstrip(b"\x00").decode("utf-8", errors="replace"))

    def to_bytes(self) -> bytes:
        return self.name.encode("utf-8") + b"\x00"


@dataclass
class DefineSpriteTag:
    """DefineSprite -- nested movieclip with own timeline."""
    sprite_id: int
    frame_count: int
    tags: list  # list of parsed tags (nested timeline)
    tag_id: int = TAG_DEFINE_SPRITE

    @classmethod
    def parse(cls, data: bytes, tag_parser) -> "DefineSpriteTag":
        """Parse sprite. tag_parser is a callable(reader) -> list[Tag]."""
        reader = BitReader(data)
        sprite_id = reader.read_ui16()
        frame_count = reader.read_ui16()
        # Remaining data is the nested tag stream
        nested_data = data[reader.tell():]
        nested_tags = tag_parser(nested_data)
        return cls(sprite_id=sprite_id, frame_count=frame_count, tags=nested_tags)

    def to_bytes(self, tag_writer) -> bytes:
        """Serialize sprite. tag_writer is a callable(tags) -> bytes."""
        writer = BitWriter()
        writer.write_ui16(self.sprite_id)
        writer.write_ui16(self.frame_count)
        writer.write_bytes(tag_writer(self.tags))
        return writer.getvalue()


# Type alias for any parsed tag
SwfTag = (
    EndTag | ShowFrameTag | SetBackgroundColorTag | FileAttributesTag |
    MetadataTag | SymbolClassTag | DefineShapeTag | PlaceObject2Tag |
    RemoveObject2Tag | FrameLabelTag | DefineSpriteTag | RawTag
)


def parse_tag_body(tag_id: int, data: bytes, tag_parser=None) -> SwfTag:
    """Dispatch tag parsing by ID. Unknown tags -> RawTag."""
    if tag_id == TAG_END:
        return EndTag()
    if tag_id == TAG_SHOW_FRAME:
        return ShowFrameTag()
    if tag_id == TAG_SET_BG_COLOR:
        return SetBackgroundColorTag.parse(data)
    if tag_id == TAG_FILE_ATTRIBUTES:
        return FileAttributesTag.parse(data)
    if tag_id == TAG_SYMBOL_CLASS:
        return SymbolClassTag.parse(data)
    if tag_id == TAG_METADATA:
        return MetadataTag.parse(data)
    if tag_id in DEFINE_SHAPE_IDS:
        return DefineShapeTag.parse(data, tag_id)
    if tag_id == TAG_PLACE_OBJECT2:
        return PlaceObject2Tag.parse(data)
    if tag_id == TAG_REMOVE_OBJECT2:
        return RemoveObject2Tag.parse(data)
    if tag_id == TAG_FRAME_LABEL:
        return FrameLabelTag.parse(data)
    if tag_id == TAG_DEFINE_SPRITE:
        if tag_parser:
            return DefineSpriteTag.parse(data, tag_parser)
        return RawTag(tag_id=tag_id, data=data)
    # PlaceObject (4) and RemoveObject (5) -- less common, passthrough
    return RawTag(tag_id=tag_id, data=data)


def write_tag(tag: SwfTag, tag_writer=None) -> tuple[int, bytes]:
    """Serialize a tag to (tag_id, body_bytes). For writing to tag stream."""
    if isinstance(tag, EndTag):
        return TAG_END, b""
    if isinstance(tag, ShowFrameTag):
        return TAG_SHOW_FRAME, b""
    if isinstance(tag, SetBackgroundColorTag):
        return tag.tag_id, tag.to_bytes()
    if isinstance(tag, FileAttributesTag):
        return tag.tag_id, tag.to_bytes()
    if isinstance(tag, MetadataTag):
        return tag.tag_id, tag.to_bytes()
    if isinstance(tag, SymbolClassTag):
        return tag.tag_id, tag.to_bytes()
    if isinstance(tag, DefineShapeTag):
        return tag.tag_id, tag.to_bytes()
    if isinstance(tag, PlaceObject2Tag):
        return tag.tag_id, tag.to_bytes()
    if isinstance(tag, RemoveObject2Tag):
        return tag.tag_id, tag.to_bytes()
    if isinstance(tag, FrameLabelTag):
        return tag.tag_id, tag.to_bytes()
    if isinstance(tag, DefineSpriteTag):
        if tag_writer:
            return tag.tag_id, tag.to_bytes(tag_writer)
        raise ValueError("DefineSpriteTag requires tag_writer for serialization")
    if isinstance(tag, RawTag):
        return tag.tag_id, tag.data
    raise TypeError(f"Unknown tag type: {type(tag)}")
