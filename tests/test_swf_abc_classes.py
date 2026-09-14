"""SymbolClass bindings must be backed by a real AS3 class.

A `SymbolClass` entry is a character-id -> class-name binding. If nothing in the
file defines that class the binding dangles and the engine cannot construct the
symbol, which is invisible to every structural check that only counts tags. These
tests cover the validator that detects the condition and the `DoABC` emitter that
makes an authored SWF satisfy it.

The reference file for byte-level layout is a shipping HUDFramework consumer,
`interface/weaponcnd.swf`; the in-repo stand-in exercised here is HUDFramework's
own `HUDMenu.swf`, which carries 177 exports against a 198-class ABC.
"""
from __future__ import annotations

from pathlib import Path

import pytest

from creation_lib.swf import native_runtime
from creation_lib.swf.parser import parse_swf
from creation_lib.swf.project import build_document
from creation_lib.swf.tags import TAG_DO_ABC, RawTag, SymbolClassTag
from creation_lib.swf.writer import write_swf

SQUARE_SVG = (
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">'
    '<path d="M 0 0 L 10 0 L 10 10 L 0 10 Z" fill="#ffffff"/>'
    "</svg>"
)

_HUD_MENU = Path(__file__).resolve().parents[2] / "external_mods" / "HudFramework" / "Interface" / "HUDMenu.swf"


@pytest.fixture
def art(tmp_path):
    (tmp_path / "square.svg").write_text(SQUARE_SVG, encoding="utf-8")
    return tmp_path


def _widget_project() -> dict:
    return {
        "canvas": [90, 16],
        "shapes": [{"id": 1, "svg": "square.svg"}],
        "sprites": [{"id": 10, "frames": [
            {"label": "stars0", "place": [{"depth": 1, "character": 1}]},
        ]}],
        "stage": [{"place": [{"depth": 1, "character": 10, "name": "starRow"}]}],
        "exports": [{"character": 10, "class": "B21_LegendaryStarRow"},
                    {"character": 0, "class": "B21_LegendaryStars"}],
    }


def _packed(art) -> bytes:
    return write_swf(build_document(_widget_project(), art))


class TestEmittedClasses:
    def test_every_export_gets_a_class(self, art):
        data = _packed(art)

        assert native_runtime.abc_class_names(data) == [
            "B21_LegendaryStarRow", "B21_LegendaryStars",
        ]
        assert native_runtime.unbacked_symbol_classes(data) == []

    def test_class_names_and_supertype_are_in_the_abc_string_pool(self, art):
        pools = native_runtime.abc_string_pools(_packed(art))

        assert len(pools) == 1
        code, minor, major, ints, uints, doubles, strings = pools[0]
        assert (code, major, minor) == (TAG_DO_ABC, 46, 16)
        assert (ints, uints, doubles) == (0, 0, 0)
        assert set(strings) >= {
            "B21_LegendaryStarRow", "B21_LegendaryStars", "flash.display", "MovieClip",
        }

    def test_doabc_precedes_symbolclass_inside_the_first_frame(self, art):
        tags = build_document(_widget_project(), art).tags
        abc_at = next(i for i, t in enumerate(tags)
                      if isinstance(t, RawTag) and t.tag_id == TAG_DO_ABC)
        symbols_at = next(i for i, t in enumerate(tags) if isinstance(t, SymbolClassTag))

        assert abc_at < symbols_at

    def test_a_project_without_exports_gets_neither_tag(self, art):
        data = write_swf(build_document({"shapes": [{"id": 1, "svg": "square.svg"}]}, art))

        assert native_runtime.list_symbols(data) == []
        assert native_runtime.abc_class_names(data) == []
        assert native_runtime.abc_string_pools(data) == []

    def test_the_display_list_instance_name_is_untouched_by_class_emission(self, art):
        """`HudBridge.cpp` resolves the widget by the depth-1 instance name, not by
        any SymbolClass name, so adding classes must not disturb it."""
        placed = parse_swf(_packed(art)).main_timeline.display_list_at(0)

        assert placed[1].name == "starRow"


class TestRoundTrip:
    def test_emitted_abc_survives_parse_and_rewrite_byte_for_byte(self, art):
        data = _packed(art)
        original = next(t for t in parse_swf(data).tags
                        if isinstance(t, RawTag) and t.tag_id == TAG_DO_ABC)

        rewritten = write_swf(parse_swf(data))
        reparsed = next(t for t in parse_swf(rewritten).tags
                        if isinstance(t, RawTag) and t.tag_id == TAG_DO_ABC)

        assert reparsed.data == original.data
        assert native_runtime.unbacked_symbol_classes(rewritten) == []

    def test_the_tag_stream_tiles_the_movie_exactly(self, art):
        assert native_runtime.roundtrip_ok(_packed(art)) is True


class TestUnbackedDetection:
    def test_a_symbolclass_with_no_doabc_is_reported(self, art):
        """The pre-fix shape of the authored widget: exports, no ABC."""
        doc = build_document(_widget_project(), art)
        doc.tags = [t for t in doc.tags
                    if not (isinstance(t, RawTag) and t.tag_id == TAG_DO_ABC)]

        assert native_runtime.unbacked_symbol_classes(write_swf(doc)) == [
            "B21_LegendaryStarRow", "B21_LegendaryStars",
        ]

    def test_renaming_one_export_leaves_only_that_name_unbacked(self, art):
        doc = build_document(_widget_project(), art)
        symbols = next(t for t in doc.tags if isinstance(t, SymbolClassTag))
        symbols.symbols[0] = (10, "B21_TypoedRowName")

        assert native_runtime.unbacked_symbol_classes(write_swf(doc)) == [
            "B21_TypoedRowName",
        ]


@pytest.mark.skipif(not _HUD_MENU.is_file(), reason="HUDFramework HUDMenu.swf not available")
class TestShippingReference:
    """HUDMenu.swf is authored by Adobe's compiler, so it is the check that the
    validator agrees with a real toolchain rather than only with our own emitter."""

    def test_every_export_is_backed(self):
        data = _HUD_MENU.read_bytes()

        assert len(native_runtime.list_symbols(data)) == 177
        assert len(native_runtime.abc_class_names(data)) == 198
        assert native_runtime.unbacked_symbol_classes(data) == []

    def test_package_qualified_class_names_are_resolved(self):
        names = native_runtime.abc_class_names(_HUD_MENU.read_bytes())

        # Package-qualified and unnamed-package classes both have to come back in
        # the spelling a SymbolClass entry uses.
        assert "Shared.AS3.BSButtonHint" in names
        assert "HUDMenu_fla.tick_119" in names
        assert "HUDMenu" in names

    def test_the_movie_body_is_preserved_byte_for_byte(self):
        """Splitting and re-tiling a real ABC-bearing file must not perturb a byte
        — the property every splice in this crate depends on."""
        import zlib

        raw = _HUD_MENU.read_bytes()
        body = zlib.decompress(raw[8:])
        info = native_runtime.swf_info(raw)

        assert info["signature"] == "CWS"
        assert info["decompressed_total"] == len(body) + 8
        assert native_runtime.roundtrip_ok(raw) is True
