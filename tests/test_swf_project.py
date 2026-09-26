"""Tests for `.swfproj` project assembly (code-free SWF authoring)."""
from __future__ import annotations

import pytest

from creation_lib.swf.parser import parse_swf
from creation_lib.swf.project import build_document
from creation_lib.swf.tags import (
    DefineSpriteTag, PlaceObject2Tag, SymbolClassTag,
)
from creation_lib.swf.writer import write_swf

SQUARE_SVG = (
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">'
    '<path d="M 0 0 L 10 0 L 10 10 L 0 10 Z" fill="#ffffff"/>'
    "</svg>"
)
TWO_PATH_SVG = (
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10">'
    '<path d="M 0 0 L 10 0 L 10 10 L 0 10 Z" fill="#ff0000"/>'
    '<path d="M 10 0 L 20 0 L 20 10 L 10 10 Z" fill="#0000ff"/>'
    "</svg>"
)


@pytest.fixture
def art(tmp_path):
    (tmp_path / "square.svg").write_text(SQUARE_SVG, encoding="utf-8")
    (tmp_path / "two.svg").write_text(TWO_PATH_SVG, encoding="utf-8")
    return tmp_path


def _star_row_project(frames: int = 6) -> dict:
    """A sprite with `frames` frames: frame N lights up slot N-1."""
    timeline = [{"label": "slot0", "place": [
        {"depth": d, "character": 1, "x": 10 * (d - 1)} for d in range(1, frames)
    ]}]
    for n in range(1, frames):
        timeline.append({"label": f"slot{n}",
                         "place": [{"depth": n, "character": 2, "x": 10 * (n - 1)}]})
    return {
        "canvas": [90, 16],
        "fps": 24,
        "background": "#010203",
        "shapes": [{"id": 1, "svg": "square.svg"}, {"id": 2, "svg": "square.svg"}],
        "sprites": [{"id": 10, "frames": timeline}],
        "stage": [{"place": [{"depth": 1, "character": 10, "name": "starRow"}]}],
        "exports": [{"character": 10, "class": "B21_StarRow"},
                    {"character": 0, "class": "B21_StarWidget"}],
    }


def test_frame_n_shows_n_minus_one_lit_slots(art):
    doc = build_document(_star_row_project(6), art)
    timeline = doc.sprites[10].timeline

    for index in range(6):
        display = timeline.display_list_at(index)
        assert sorted(display) == [1, 2, 3, 4, 5]
        lit = [d for d in display if display[d].character_id == 2]
        assert len(lit) == index


def test_reused_depth_becomes_a_move(art):
    doc = build_document(_star_row_project(6), art)
    sprite = next(t for t in doc.tags if isinstance(t, DefineSpriteTag))
    at_depth_1 = [t for t in sprite.tags
                  if isinstance(t, PlaceObject2Tag) and t.depth == 1]

    assert [(t.character_id, t.move) for t in at_depth_1] == [(1, False), (2, True)]


def test_canvas_fps_and_background(art):
    doc = build_document(_star_row_project(6), art)

    assert doc.header.frame_size.width_px == 90
    assert doc.header.frame_size.height_px == 16
    assert doc.header.fps == 24
    assert doc.background_color.to_hex() == "#010203"


def test_exports_survive_write_and_reparse(art):
    doc = build_document(_star_row_project(6), art)

    reparsed = parse_swf(write_swf(doc))

    assert reparsed.symbols == [(10, "B21_StarRow"), (0, "B21_StarWidget")]


def test_definitions_precede_placements_symbolclass_and_showframe(art):
    from creation_lib.swf.tags import DefineShapeTag, ShowFrameTag

    doc = build_document(_star_row_project(6), art)
    index = {}
    for i, tag in enumerate(doc.tags):
        index.setdefault(type(tag).__name__, []).append(i)

    last_definition = max(index["DefineShapeTag"] + index["DefineSpriteTag"])
    assert last_definition < min(index["PlaceObject2Tag"])
    assert max(index["PlaceObject2Tag"]) < index["SymbolClassTag"][0]
    # A SymbolClass after the frame's ShowFrame registers nothing in frame 1 --
    # the file still parses, so only ordering catches it.
    assert index["SymbolClassTag"][0] < index["ShowFrameTag"][0]


def test_multiple_drawables_merge_into_one_character_with_rebased_fills(art):
    doc = build_document({"shapes": [{"id": 3, "svg": "two.svg"}]}, art)
    shape = doc.shapes[3]

    assert len(shape.fill_styles) == 2
    assert [(f.color.r, f.color.g, f.color.b) for f in shape.fill_styles] == [
        (255, 0, 0), (0, 0, 255),
    ]
    fill_refs = [r.fill1 for r in shape.records
                 if getattr(r, "fill1", None) is not None]
    assert fill_refs == [1, 2]
    assert shape.bounds == (0, 0, 400, 200)


@pytest.mark.parametrize("project,match", [
    ({"shapes": [{"id": 1, "svg": "square.svg"}],
      "sprites": [{"id": 1, "frames": [{}]}]}, "already defined"),
    ({"shapes": [{"id": 1, "svg": "square.svg"}],
      "stage": [{"place": [{"depth": 1, "character": 99}]}]}, "undefined character 99"),
    ({"shapes": [{"id": 1, "svg": "square.svg"}],
      "exports": [{"character": 42, "class": "Ghost"}]}, "undefined character 42"),
    ({"shapes": [{"id": 1}]}, "missing required key 'svg'"),
])
def test_invalid_project_configs_are_rejected(art, project, match):
    with pytest.raises(ValueError, match=match):
        build_document(project, art)
