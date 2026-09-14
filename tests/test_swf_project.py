"""Tests for `.swfproj` project assembly (code-free SWF authoring)."""
from __future__ import annotations

import json

import pytest

from creation_lib.swf.parser import parse_swf
from creation_lib.swf.project import build_document, load_project_file
from creation_lib.swf.tags import (
    DefineShapeTag, DefineSpriteTag, FileAttributesTag, PlaceObject2Tag,
    ShowFrameTag, SymbolClassTag, TAG_DEFINE_SHAPE3,
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


class TestSpriteTimeline:
    def test_sprite_reports_six_frames(self, art):
        doc = build_document(_star_row_project(6), art)

        sprite = next(t for t in doc.tags if isinstance(t, DefineSpriteTag))
        assert sprite.sprite_id == 10
        assert sprite.frame_count == 6
        assert doc.sprites[10].timeline.frame_count == 6

    def test_frame_n_shows_n_minus_one_lit_slots(self, art):
        doc = build_document(_star_row_project(6), art)
        timeline = doc.sprites[10].timeline

        for index in range(6):
            display = timeline.display_list_at(index)
            assert sorted(display) == [1, 2, 3, 4, 5]
            lit = [d for d in display if display[d].character_id == 2]
            assert len(lit) == index

    def test_reused_depth_becomes_a_move(self, art):
        doc = build_document(_star_row_project(6), art)
        sprite = next(t for t in doc.tags if isinstance(t, DefineSpriteTag))
        at_depth_1 = [t for t in sprite.tags
                      if isinstance(t, PlaceObject2Tag) and t.depth == 1]

        assert [(t.character_id, t.move) for t in at_depth_1] == [(1, False), (2, True)]

    def test_placement_x_is_pixels_converted_to_twips(self, art):
        doc = build_document(_star_row_project(6), art)
        sprite = next(t for t in doc.tags if isinstance(t, DefineSpriteTag))
        first_frame = [t for t in sprite.tags if isinstance(t, PlaceObject2Tag)][:5]

        assert [t.matrix.translate_x for t in first_frame] == [0, 200, 400, 600, 800]

    def test_removal_clears_the_depth(self, art):
        doc = build_document({
            "shapes": [{"id": 1, "svg": "square.svg"}],
            "sprites": [{"id": 5, "frames": [
                {"place": [{"depth": 1, "character": 1}]},
                {"remove": [1]},
            ]}],
        }, art)

        assert doc.sprites[5].timeline.display_list_at(1) == {}


class TestHeader:
    def test_frame_count_follows_the_stage(self, art):
        doc = build_document({
            "canvas": [64, 32],
            "shapes": [{"id": 1, "svg": "square.svg"}],
            "stage": [{"place": [{"depth": 1, "character": 1}]}, {}, {}],
        }, art)

        assert doc.header.frame_count == 3
        assert doc.main_timeline.frame_count == 3

    def test_canvas_fps_and_background(self, art):
        doc = build_document(_star_row_project(6), art)

        assert doc.header.frame_size.width_px == 90
        assert doc.header.frame_size.height_px == 16
        assert doc.header.fps == 24
        assert doc.background_color.to_hex() == "#010203"

    def test_stage_defaults_to_one_empty_frame(self, art):
        doc = build_document({"shapes": [{"id": 1, "svg": "square.svg"}]}, art)

        assert doc.header.frame_count == 1


class TestSymbolExports:
    def test_exports_survive_write_and_reparse(self, art):
        doc = build_document(_star_row_project(6), art)

        reparsed = parse_swf(write_swf(doc))

        assert reparsed.symbols == [(10, "B21_StarRow"), (0, "B21_StarWidget")]

    def test_action_script3_flag_set_when_exporting(self, art):
        doc = build_document(_star_row_project(6), art)
        attrs = next(t for t in doc.tags if isinstance(t, FileAttributesTag))

        assert attrs.action_script3 is True

    def test_no_symbol_class_tag_without_exports(self, art):
        doc = build_document({"shapes": [{"id": 1, "svg": "square.svg"}]}, art)

        assert not any(isinstance(t, SymbolClassTag) for t in doc.tags)
        assert next(t for t in doc.tags if isinstance(t, FileAttributesTag)).action_script3 is False


class TestTagOrder:
    def test_definitions_precede_placements_symbolclass_and_showframe(self, art):
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

    def test_symbol_class_lands_inside_the_first_frame_of_a_multi_frame_stage(self, art):
        doc = build_document({
            "shapes": [{"id": 1, "svg": "square.svg"}],
            "stage": [{"place": [{"depth": 1, "character": 1}]}, {}, {}],
            "exports": [{"character": 1, "class": "Thing"}],
        }, art)
        show_frames = [i for i, t in enumerate(doc.tags) if isinstance(t, ShowFrameTag)]
        symbol_class = next(i for i, t in enumerate(doc.tags) if isinstance(t, SymbolClassTag))

        assert symbol_class < show_frames[0]


class TestShapeImport:
    def test_svg_becomes_a_defineshape3_with_the_declared_character_id(self, art):
        doc = build_document({"shapes": [{"id": 7, "svg": "square.svg"}]}, art)
        tag = next(t for t in doc.tags if isinstance(t, DefineShapeTag))

        assert tag.tag_id == TAG_DEFINE_SHAPE3
        assert tag.shape.shape_id == 7
        assert doc.shapes[7].bounds == (0, 0, 200, 200)

    def test_multiple_drawables_merge_into_one_character_with_rebased_fills(self, art):
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

    def test_empty_svg_is_rejected(self, art):
        (art / "blank.svg").write_text(
            '<svg xmlns="http://www.w3.org/2000/svg"></svg>', encoding="utf-8")

        with pytest.raises(ValueError, match="no drawable shapes"):
            build_document({"shapes": [{"id": 1, "svg": "blank.svg"}]}, art)


class TestValidation:
    def test_duplicate_character_id_is_rejected(self, art):
        with pytest.raises(ValueError, match="already defined"):
            build_document({
                "shapes": [{"id": 1, "svg": "square.svg"}],
                "sprites": [{"id": 1, "frames": [{}]}],
            }, art)

    def test_placing_an_undefined_character_is_rejected(self, art):
        with pytest.raises(ValueError, match="undefined character 99"):
            build_document({
                "shapes": [{"id": 1, "svg": "square.svg"}],
                "stage": [{"place": [{"depth": 1, "character": 99}]}],
            }, art)

    def test_exporting_an_undefined_character_is_rejected(self, art):
        with pytest.raises(ValueError, match="undefined character 42"):
            build_document({
                "shapes": [{"id": 1, "svg": "square.svg"}],
                "exports": [{"character": 42, "class": "Ghost"}],
            }, art)

    def test_character_zero_may_be_exported_as_the_root(self, art):
        doc = build_document({
            "shapes": [{"id": 1, "svg": "square.svg"}],
            "exports": [{"character": 0, "class": "Root"}],
        }, art)

        assert doc.symbols == [(0, "Root")]

    def test_missing_required_key_names_the_key(self, art):
        with pytest.raises(ValueError, match="missing required key 'svg'"):
            build_document({"shapes": [{"id": 1}]}, art)


class TestLoadProjectFile:
    def test_svg_paths_resolve_relative_to_the_project_file(self, art):
        nested = art / "proj"
        nested.mkdir()
        (nested / "widget.swfproj").write_text(json.dumps({
            "shapes": [{"id": 1, "svg": "../square.svg"}],
            "exports": [{"character": 1, "class": "Square"}],
        }), encoding="utf-8")

        doc = load_project_file(nested / "widget.swfproj")

        assert doc.symbols == [(1, "Square")]
        assert doc.shapes[1].bounds == (0, 0, 200, 200)
