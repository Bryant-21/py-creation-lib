from __future__ import annotations

import importlib.util
import json
from pathlib import Path

from creation_lib.swf.parser import SwfDocument, SwfHeader
from creation_lib.swf.timeline import Frame, SpriteDef, Timeline
from creation_lib.swf.types import MATRIX, RECT


def _load_script_module():
    script_path = Path(__file__).resolve().parents[2] / "scripts" / "prismahud_layout_extract.py"
    spec = importlib.util.spec_from_file_location("prismahud_layout_extract", script_path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


layout_extract = _load_script_module()


def _fixture_doc() -> SwfDocument:
    header = SwfHeader(
        compression="FWS",
        version=17,
        file_length=0,
        frame_size=RECT(xmin=0, xmax=25600, ymin=0, ymax=14400),
        fps=30,
        frame_count=1,
    )

    main_frame = Frame()
    main_frame.place(
        depth=1,
        character_id=100,
        matrix=MATRIX(
            scale_x=2.0,
            scale_y=0.5,
            translate_x=1280,
            translate_y=13680,
        ),
        name="LeftMeters_mc",
    )
    main_frame.place(
        depth=2,
        character_id=101,
        matrix=MATRIX(translate_x=400, translate_y=800),
        name=None,
    )

    child_frame = Frame()
    child_frame.place(
        depth=1,
        character_id=200,
        matrix=MATRIX(
            scale_x=0.75,
            scale_y=0.5,
            translate_x=200,
            translate_y=-40,
        ),
        name="HPMeter_mc",
    )
    child_frame.place(
        depth=2,
        character_id=201,
        matrix=MATRIX(translate_x=300, translate_y=300),
        name=None,
    )

    doc = SwfDocument(
        header=header,
        main_timeline=Timeline(frames=[main_frame]),
    )
    doc.sprites[100] = SpriteDef(
        sprite_id=100,
        timeline=Timeline(frames=[child_frame]),
    )
    return doc


def test_stage_size_converts_twips_to_pixels():
    layout = layout_extract.extract_layout(_fixture_doc())

    assert layout["stage"] == {"width": 1280.0, "height": 720.0}


def test_named_top_level_placement_converts_twips_to_pixels():
    layout = layout_extract.extract_layout(_fixture_doc())

    assert layout["placements"]["LeftMeters_mc"] == {
        "x": 64.0,
        "y": 684.0,
        "scale_x": 2.0,
        "scale_y": 0.5,
    }


def test_named_child_placement_uses_absolute_stage_position():
    layout = layout_extract.extract_layout(_fixture_doc())

    assert layout["placements"]["LeftMeters_mc/HPMeter_mc"] == {
        "x": 84.0,
        "y": 683.0,
        "scale_x": 1.5,
        "scale_y": 0.25,
    }


def test_unnamed_entries_are_skipped():
    layout = layout_extract.extract_layout(_fixture_doc())

    assert None not in layout["placements"]
    assert all("None" not in name for name in layout["placements"])
    assert len(layout["placements"]) == 2


def test_layout_is_json_roundtrippable():
    layout = layout_extract.extract_layout(_fixture_doc())

    assert json.loads(json.dumps(layout)) == layout


def test_help_returns_zero(capsys):
    assert layout_extract.main(["--help"]) == 0

    captured = capsys.readouterr()
    assert "Usage:" in captured.out
