"""Tests for SWF tag definitions."""
from __future__ import annotations

import pytest

from creation_lib.swf.types import BitReader, BitWriter, RGBA, RECT, MATRIX, FillStyle
from creation_lib.swf.shapes import ShapeDef, StraightEdge, StyleChange, EndShape
from creation_lib.swf.tags import (
    RawTag, EndTag, ShowFrameTag, SetBackgroundColorTag,
    DefineShapeTag, PlaceObject2Tag, RemoveObject2Tag,
    DefineSpriteTag, FrameLabelTag, FileAttributesTag, SymbolClassTag,
    TAG_END, TAG_SHOW_FRAME, TAG_SET_BG_COLOR,
    TAG_DEFINE_SHAPE, TAG_DEFINE_SHAPE2, TAG_DEFINE_SHAPE3, TAG_DEFINE_SHAPE4,
    TAG_PLACE_OBJECT2, TAG_REMOVE_OBJECT2,
    TAG_DEFINE_SPRITE, TAG_FRAME_LABEL, TAG_FILE_ATTRIBUTES, TAG_SYMBOL_CLASS,
    parse_tag_body, write_tag,
)


class TestRawTag:
    def test_preserves_data(self):
        tag = RawTag(tag_id=999, data=b"\x01\x02\x03")
        assert tag.tag_id == 999
        assert tag.data == b"\x01\x02\x03"


class TestEndTag:
    def test_end_tag_id(self):
        assert TAG_END == 0
        tag = EndTag()
        assert tag.tag_id == TAG_END


class TestShowFrameTag:
    def test_show_frame_id(self):
        assert TAG_SHOW_FRAME == 1


class TestSetBackgroundColorTag:
    def test_parse_bg_color(self):
        # RGB bytes: 0x33, 0x33, 0x33
        data = bytes([0x33, 0x33, 0x33])
        tag = SetBackgroundColorTag.parse(data)
        assert tag.color.r == 0x33
        assert tag.color.g == 0x33
        assert tag.color.b == 0x33

    def test_write_bg_color(self):
        tag = SetBackgroundColorTag(color=RGBA(0x33, 0x33, 0x33, 255))
        data = tag.to_bytes()
        assert data == bytes([0x33, 0x33, 0x33])


class TestFileAttributesTag:
    def test_default_pipboy_attrs(self):
        # FO4 Pipboy SWFs: no ActionScript3, no metadata, no network
        tag = FileAttributesTag(
            has_metadata=False,
            action_script3=False,
            use_network=False,
        )
        assert not tag.action_script3


class TestPlaceObject2Tag:
    def test_place_with_transform(self):
        tag = PlaceObject2Tag(
            depth=1,
            character_id=5,
            matrix=MATRIX(translate_x=1000, translate_y=2000),
        )
        assert tag.depth == 1
        assert tag.character_id == 5
        assert tag.matrix.translate_x == 1000


class TestFrameLabelTag:
    def test_label(self):
        tag = FrameLabelTag(name="idle")
        data = tag.to_bytes()
        parsed = FrameLabelTag.parse(data)
        assert parsed.name == "idle"


class TestSymbolClassTag:
    def test_tag_id(self):
        assert TAG_SYMBOL_CLASS == 76

    def test_body_layout_matches_spec(self):
        """UI16 NumSymbols, then per symbol UI16 CharacterID + NUL-terminated name."""
        tag = SymbolClassTag(symbols=[(10, "StarRow"), (0, "Root")])
        assert tag.to_bytes() == (
            b"\x02\x00"
            + b"\x0a\x00" + b"StarRow\x00"
            + b"\x00\x00" + b"Root\x00"
        )

    def test_round_trip(self):
        tag = SymbolClassTag(symbols=[(3, "CritMeterStar"), (643, "HUDMenu_fla.Group_116")])
        assert SymbolClassTag.parse(tag.to_bytes()).symbols == [
            (3, "CritMeterStar"), (643, "HUDMenu_fla.Group_116"),
        ]

    def test_empty_table(self):
        assert SymbolClassTag().to_bytes() == b"\x00\x00"
        assert SymbolClassTag.parse(b"\x00\x00").symbols == []

    def test_dispatches_through_tag_stream_helpers(self):
        body = SymbolClassTag(symbols=[(7, "Widget")]).to_bytes()
        tag = parse_tag_body(TAG_SYMBOL_CLASS, body)
        assert isinstance(tag, SymbolClassTag)
        assert tag.symbols == [(7, "Widget")]
        assert write_tag(tag) == (TAG_SYMBOL_CLASS, body)
