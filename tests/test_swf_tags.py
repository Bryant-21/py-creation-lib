"""Tests for SWF tag definitions."""
from __future__ import annotations

from creation_lib.swf.types import RGBA
from creation_lib.swf.tags import (
    SetBackgroundColorTag, SymbolClassTag, TAG_SYMBOL_CLASS,
    parse_tag_body, write_tag,
)


class TestSetBackgroundColorTag:
    def test_parse_and_write_bg_color_round_trip(self):
        data = bytes([0x33, 0x33, 0x33])
        tag = SetBackgroundColorTag.parse(data)
        assert (tag.color.r, tag.color.g, tag.color.b) == (0x33, 0x33, 0x33)
        assert tag.to_bytes() == data

        written = SetBackgroundColorTag(color=RGBA(0x33, 0x33, 0x33, 255)).to_bytes()
        assert written == data


class TestSymbolClassTag:
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
