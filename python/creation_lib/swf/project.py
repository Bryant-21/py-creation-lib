"""`.swfproj` project files -- declarative, code-free SWF authoring.

A `.swfproj` is JSON a human hand-writes. It names the art (SVG files on disk),
the sprites whose timelines that art animates on, what sits on the main
timeline, and which characters are exported under an AS3 class name so anything
outside the file can find them.

    {
      "canvas": [128, 24],
      "fps": 30,
      "background": "#000000",
      "shapes":  [{"id": 1, "svg": "star_empty.svg"}],
      "sprites": [{"id": 10, "frames": [
                     {"label": "zero", "place": [{"depth": 1, "character": 1}]}]}],
      "stage":   [{"place": [{"depth": 1, "character": 10, "name": "starRow"}]}],
      "scripts": ["MyStarRow.as", "hudframework/IHUDWidget.as"],
      "exports": [{"character": 10, "class": "MyStarRow"}]
    }

`scripts` lists ActionScript 3 source files, relative to the project file. They
are compiled together into one `DoABC` tag emitted ahead of the `SymbolClass`.
AS3 allows one package per file, so a widget's document class and an interface
it implements are separate entries; order does not matter. Every name an
`exports` entry binds must be a class those sources define, or the build fails
naming the missing ones.

Omit `scripts` and each `exports` entry instead gets a class *synthesized* for
it: a `flash.display.MovieClip` subclass with an empty constructor and no
methods. That resolves the `SymbolClass` binding (marker-icon SWFs are built
this way) but does nothing; a widget that receives messages needs `scripts`.

Declare neither and the file gets no `DoABC` and no `SymbolClass`: an asset-only
SWF with a plain `MovieClip` root.

`shapes` / `sprites` / `stage` / `scripts` / `exports` are all optional. A frame is
`{"label": str, "remove": [depth], "place": [placement]}`; a placement is
`{"depth", "character", "x", "y", "scale_x", "scale_y", "name"}` with x/y in
pixels. `stage` is a list of frames in the same shape as a sprite's, and its
length becomes the header frame count.

The display list is cumulative, so re-placing a depth that an earlier frame
already filled is emitted as a PlaceObject2 *move* -- the caller does not
declare that.
"""
from __future__ import annotations

import json
from dataclasses import replace
from pathlib import Path

from creation_lib.swf import native_runtime
from creation_lib.swf.parser import SwfDocument, SwfHeader, _build_document
from creation_lib.swf.shapes import EndShape, ShapeDef, StyleChange
from creation_lib.swf.svg_io import svg_to_shapes
from creation_lib.swf.tags import (
    SHAPE_VERSION_MAP, TAG_DO_ABC, DefineShapeTag, DefineSpriteTag, EndTag,
    FileAttributesTag, FrameLabelTag, PlaceObject2Tag, RawTag, RemoveObject2Tag,
    SetBackgroundColorTag, ShowFrameTag, SwfTag, SymbolClassTag,
)
from creation_lib.swf.types import MATRIX, RECT, RGBA, TWIPS_PER_PIXEL

DEFAULT_CANVAS = (550, 400)
DEFAULT_FPS = 30
DEFAULT_BACKGROUND = "#333333"
DEFAULT_VERSION = 17

_SHAPE_TAG_FOR_VERSION = {v: k for k, v in SHAPE_VERSION_MAP.items()}


def _read_script(base_dir: Path, entry: str) -> str:
    """Read one ActionScript source named by a project's `scripts` list."""
    path = base_dir / entry
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError as exc:
        raise ValueError(f"script {entry!r} not found at {path}") from exc


def load_project_file(path: str | Path) -> SwfDocument:
    """Read a `.swfproj` and assemble the SWF document it describes."""
    path = Path(path)
    project = json.loads(path.read_text(encoding="utf-8"))
    return build_document(project, path.parent)


def build_document(project: dict, base_dir: str | Path) -> SwfDocument:
    """Assemble a writable SwfDocument from a parsed `.swfproj`.

    `base_dir` roots the relative SVG paths -- normally the project file's own
    directory.
    """
    base_dir = Path(base_dir)
    canvas = project.get("canvas", DEFAULT_CANVAS)
    exports = project.get("exports", [])

    definitions: list[SwfTag] = []
    declared: dict[int, str] = {}
    for entry in project.get("shapes", []):
        definitions.append(_shape_tag(entry, base_dir, declared))
    for entry in project.get("sprites", []):
        definitions.append(_sprite_tag(entry, declared))

    for export in exports:
        character = _require(export, "character", "export")
        if character != 0 and character not in declared:
            raise ValueError(f"export {_require(export, 'class', 'export')!r} names "
                             f"undefined character {character}")

    scripts = project.get("scripts", [])
    sources = [_read_script(base_dir, s) for s in scripts]

    stage_frames = project.get("stage") or [{}]
    stage_tags = _timeline_tags(stage_frames, declared)
    if exports or sources:
        class_names = [_require(e, "class", "export") for e in exports]
        if sources:
            defined = native_runtime.compile_as3_class_names(sources)
            missing = [n for n in class_names if n not in defined]
            if missing:
                raise ValueError(
                    f"exports name {missing} but the compiled ActionScript defines "
                    f"{sorted(defined)}; a SymbolClass entry with no class behind it "
                    f"is a binding the engine cannot construct")
            abc = native_runtime.compile_as3_do_abc(sources)
        else:
            abc = native_runtime.build_movieclip_class_doabc(class_names)

        # Both tags have to land inside the first frame -- a player that reaches
        # ShowFrame before seeing them enters frame 1 with nothing registered --
        # and DoABC has to come first so the classes exist by the time the
        # SymbolClass entries bind to them.
        at = _first_show_frame(stage_tags)
        if exports:
            stage_tags.insert(at, SymbolClassTag(
                symbols=[(e["character"], name) for e, name in zip(exports, class_names)]))
        stage_tags.insert(at, RawTag(tag_id=TAG_DO_ABC, data=abc))

    tags: list[SwfTag] = [
        FileAttributesTag(action_script3=bool(exports or sources)),
        SetBackgroundColorTag(color=RGBA.from_hex(project.get("background", DEFAULT_BACKGROUND))),
    ]
    tags.extend(definitions)
    tags.extend(stage_tags)
    tags.append(EndTag())

    doc = SwfDocument(
        header=SwfHeader(
            compression="CWS",
            version=project.get("version", DEFAULT_VERSION),
            file_length=0,
            frame_size=RECT(xmin=0, xmax=canvas[0] * TWIPS_PER_PIXEL,
                            ymin=0, ymax=canvas[1] * TWIPS_PER_PIXEL),
            fps=project.get("fps", DEFAULT_FPS),
            frame_count=len(stage_frames),
        ),
    )
    doc.tags = tags
    _build_document(doc, tags)
    return doc


def _first_show_frame(tags: list[SwfTag]) -> int:
    for i, tag in enumerate(tags):
        if isinstance(tag, ShowFrameTag):
            return i
    return len(tags)


def _shape_tag(entry: dict, base_dir: Path, declared: dict[int, str]) -> DefineShapeTag:
    character_id = _claim(declared, _require(entry, "id", "shape"), "shape")
    svg_path = base_dir / _require(entry, "svg", "shape")
    shapes = svg_to_shapes(svg_path.read_text(encoding="utf-8"))
    if not shapes:
        raise ValueError(f"{svg_path} defines no drawable shapes")
    shape = _merge_shapes(shapes, character_id)
    return DefineShapeTag(shape=shape, tag_id=_SHAPE_TAG_FOR_VERSION[shape.shape_version])


def _merge_shapes(shapes: list[ShapeDef], character_id: int) -> ShapeDef:
    """Fold every drawable in one SVG into a single character.

    `shape_to_svg` emits one <path> per fill style plus a preview background
    rect, so a faithful re-import has to concatenate them under one character
    rather than keep only the first. Each sub-shape opens with an absolute
    moveto, so the runs concatenate without a joining edge; only the style
    indices need rebasing onto the merged style tables.
    """
    fill_styles = []
    line_styles = []
    records = []
    for shape in shapes:
        fill_offset = len(fill_styles)
        line_offset = len(line_styles)
        fill_styles.extend(shape.fill_styles)
        line_styles.extend(shape.line_styles)
        for record in shape.records:
            if isinstance(record, EndShape):
                continue
            if isinstance(record, StyleChange):
                record = replace(
                    record,
                    fill0=_rebase_style(record.fill0, fill_offset),
                    fill1=_rebase_style(record.fill1, fill_offset),
                    line=_rebase_style(record.line, line_offset),
                )
            records.append(record)
    records.append(EndShape())

    return ShapeDef(
        shape_id=character_id,
        bounds=(
            min(s.bounds[0] for s in shapes),
            min(s.bounds[1] for s in shapes),
            max(s.bounds[2] for s in shapes),
            max(s.bounds[3] for s in shapes),
        ),
        fill_styles=fill_styles,
        line_styles=line_styles,
        records=records,
        shape_version=max(s.shape_version for s in shapes),
    )


def _rebase_style(index: int | None, offset: int) -> int | None:
    """Shift a 1-based style index onto the merged table. 0 means 'no style'."""
    if index is None or index == 0:
        return index
    return index + offset


def _sprite_tag(entry: dict, declared: dict[int, str]) -> DefineSpriteTag:
    sprite_id = _claim(declared, _require(entry, "id", "sprite"), "sprite")
    frames = _require(entry, "frames", "sprite")
    tags = _timeline_tags(frames, declared)
    tags.append(EndTag())
    return DefineSpriteTag(sprite_id=sprite_id, frame_count=len(frames), tags=tags)


def _timeline_tags(frames: list[dict], declared: dict[int, str]) -> list[SwfTag]:
    tags: list[SwfTag] = []
    occupied: set[int] = set()
    for frame in frames:
        label = frame.get("label")
        if label:
            tags.append(FrameLabelTag(name=label))
        for depth in frame.get("remove", []):
            tags.append(RemoveObject2Tag(depth=depth))
            occupied.discard(depth)
        for placement in frame.get("place", []):
            depth = _require(placement, "depth", "placement")
            character = _require(placement, "character", "placement")
            if character not in declared:
                raise ValueError(f"placement at depth {depth} names "
                                 f"undefined character {character}")
            tags.append(PlaceObject2Tag(
                depth=depth,
                character_id=character,
                matrix=MATRIX(
                    scale_x=placement.get("scale_x", 1.0),
                    scale_y=placement.get("scale_y", 1.0),
                    translate_x=round(placement.get("x", 0) * TWIPS_PER_PIXEL),
                    translate_y=round(placement.get("y", 0) * TWIPS_PER_PIXEL),
                ),
                name=placement.get("name"),
                move=depth in occupied,
            ))
            occupied.add(depth)
        tags.append(ShowFrameTag())
    return tags


def _require(mapping: dict, key: str, context: str):
    if key not in mapping:
        raise ValueError(f"{context} entry is missing required key {key!r}: {mapping}")
    return mapping[key]


def _claim(declared: dict[int, str], character_id: int, kind: str) -> int:
    if character_id in declared:
        raise ValueError(f"character id {character_id} is already defined "
                         f"as a {declared[character_id]}")
    declared[character_id] = kind
    return character_id
