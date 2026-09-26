"""SymbolClass bindings must be backed by a real AS3 class.

A `SymbolClass` entry is a character-id -> class-name binding. If nothing in the
file defines that class the binding dangles and the engine cannot construct the
symbol, which is invisible to every structural check that only counts tags. These
tests cover the validator that detects the condition and the `DoABC` emitter that
makes an authored SWF satisfy it.
"""
from __future__ import annotations

import pytest

from creation_lib.swf import native_runtime
from creation_lib.swf.project import build_document
from creation_lib.swf.tags import TAG_DO_ABC, RawTag, SymbolClassTag
from creation_lib.swf.writer import write_swf

SQUARE_SVG = (
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">'
    '<path d="M 0 0 L 10 0 L 10 10 L 0 10 Z" fill="#ffffff"/>'
    "</svg>"
)


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


def test_every_export_gets_a_class(art):
    data = _packed(art)

    assert native_runtime.abc_class_names(data) == [
        "B21_LegendaryStarRow", "B21_LegendaryStars",
    ]
    assert native_runtime.unbacked_symbol_classes(data) == []


def test_doabc_precedes_symbolclass_inside_the_first_frame(art):
    tags = build_document(_widget_project(), art).tags
    abc_at = next(i for i, t in enumerate(tags)
                  if isinstance(t, RawTag) and t.tag_id == TAG_DO_ABC)
    symbols_at = next(i for i, t in enumerate(tags) if isinstance(t, SymbolClassTag))

    assert abc_at < symbols_at


def test_a_project_without_exports_gets_neither_tag(art):
    data = write_swf(build_document({"shapes": [{"id": 1, "svg": "square.svg"}]}, art))

    assert native_runtime.list_symbols(data) == []
    assert native_runtime.abc_class_names(data) == []
    assert native_runtime.abc_string_pools(data) == []


def test_emitted_abc_survives_parse_and_rewrite_byte_for_byte(art):
    from creation_lib.swf.parser import parse_swf

    data = _packed(art)
    original = next(t for t in parse_swf(data).tags
                    if isinstance(t, RawTag) and t.tag_id == TAG_DO_ABC)

    rewritten = write_swf(parse_swf(data))
    reparsed = next(t for t in parse_swf(rewritten).tags
                    if isinstance(t, RawTag) and t.tag_id == TAG_DO_ABC)

    assert reparsed.data == original.data
    assert native_runtime.unbacked_symbol_classes(rewritten) == []


def test_a_symbolclass_with_no_doabc_is_reported(art):
    """The pre-fix shape of the authored widget: exports, no ABC."""
    doc = build_document(_widget_project(), art)
    doc.tags = [t for t in doc.tags
                if not (isinstance(t, RawTag) and t.tag_id == TAG_DO_ABC)]

    assert native_runtime.unbacked_symbol_classes(write_swf(doc)) == [
        "B21_LegendaryStarRow", "B21_LegendaryStars",
    ]


def test_renaming_one_export_leaves_only_that_name_unbacked(art):
    doc = build_document(_widget_project(), art)
    symbols = next(t for t in doc.tags if isinstance(t, SymbolClassTag))
    symbols.symbols[0] = (10, "B21_TypoedRowName")

    assert native_runtime.unbacked_symbol_classes(write_swf(doc)) == [
        "B21_TypoedRowName",
    ]
