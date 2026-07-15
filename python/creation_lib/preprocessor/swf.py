"""Build SWF shape library index from extracted game SWFs.

Extracts individual shapes from VaultBoy SWF files and stores them
in an FTS5-indexed SQLite database for searchable shape browsing.
"""

from __future__ import annotations

import argparse
import json
import logging
import os
import sys
from pathlib import Path

from creation_lib.db.native_runtime import BulkInserter

from creation_lib.core.game_profiles import GAME_PROFILES

_log = logging.getLogger(__name__)


SHAPE_COLS = [
    "name", "source_swf", "source_tag", "bounds", "fill_colors",
    "path_count", "shape_data", "svg_preview", "tags", "user_added",
]

BULK_SCHEMA = json.dumps({
    "tables": [
        {"name": "shapes", "columns": SHAPE_COLS},
    ],
})

CREATE_TABLES_DDL = """
CREATE TABLE IF NOT EXISTS shapes (
    id          INTEGER PRIMARY KEY,
    name        TEXT,
    source_swf  TEXT,
    source_tag  INTEGER,
    bounds      TEXT,
    fill_colors TEXT,
    path_count  INTEGER,
    shape_data  BLOB,
    svg_preview TEXT,
    tags        TEXT,
    user_added  INTEGER DEFAULT 0
);
CREATE VIRTUAL TABLE IF NOT EXISTS shapes_fts USING fts5(
    name, tags, source_swf, content=shapes, content_rowid=id
);
"""


def serialize_shape_records(shape) -> bytes:
    from creation_lib.swf.shapes import StyleChange, StraightEdge, CurvedEdge, EndShape

    records = []
    for r in shape.records:
        if isinstance(r, StyleChange):
            records.append({
                "type": "sc",
                "move": r.has_move,
                "dx": getattr(r, "move_x", 0),
                "dy": getattr(r, "move_y", 0),
                "fill0": getattr(r, "fill0", None),
                "fill1": getattr(r, "fill1", None),
            })
        elif isinstance(r, StraightEdge):
            records.append({"type": "se", "dx": r.dx, "dy": r.dy})
        elif isinstance(r, CurvedEdge):
            records.append({"type": "ce", "cx": r.cx, "cy": r.cy, "ax": r.ax, "ay": r.ay})
        elif isinstance(r, EndShape):
            records.append({"type": "end"})

    fill_styles = []
    for fs in shape.fill_styles:
        if fs.color:
            fill_styles.append({"color": fs.color.to_hex(), "type": fs.fill_type})

    return json.dumps({
        "records": records,
        "fill_styles": fill_styles,
        "bounds": list(shape.bounds_px),
    }).encode()


def deserialize_shape_records(blob: bytes) -> dict:
    return json.loads(blob.decode())


_CATEGORY_RULES: list[tuple[str, str]] = [
    ("components/vaultboys/perks", "Perk"),
    ("components/vaultboys/special", "SPECIAL"),
    ("components/vaultboys/dlc", "DLC Perk"),
    ("components/vaultboys", "Perk"),
    ("components/quest vault boys", "Quest"),
    ("quest animations", "Quest"),
    ("components/faction vault boys", "Faction"),
    ("components/magazine perks", "Magazine"),
    ("components/conditionclips", "Condition"),
]


def _classify_swf(swf_path: Path, interface_root: Path) -> tuple[str, str]:
    try:
        rel = swf_path.relative_to(interface_root)
    except ValueError:
        return ("Other", swf_path.parent.name)

    rel_lower = str(rel).replace("\\", "/").lower()

    for pattern, cat in _CATEGORY_RULES:
        if pattern in rel_lower:
            subcat = swf_path.parent.name
            if subcat.lower() == "swf":
                subcat = swf_path.parent.parent.name
            return (cat, subcat)

    return ("UI", swf_path.parent.name)


def _collect_swf_dirs(interface_root: Path) -> list[Path]:
    scan_roots = [
        interface_root / "Components" / "VaultBoys",
        interface_root / "Components" / "Quest Vault Boys",
        interface_root / "Components" / "Faction Vault boys",
        interface_root / "Components" / "Magazine perks",
        interface_root / "Components" / "ConditionClips",
    ]
    for d in interface_root.iterdir():
        if d.is_dir() and "quest" in d.name.lower() and "animation" in d.name.lower():
            scan_roots.append(d)

    return [d for d in scan_roots if d.is_dir()]


def extract_shapes(swf_dir: Path, db_path: Path, on_progress=None) -> int:
    """Extract shapes from all SWFs under Interface/ into the database."""
    from creation_lib.swf.parser import parse_swf_file
    from creation_lib.swf.svg_io import shape_to_svg
    from creation_lib.swf.shapes import StyleChange

    scan_dirs = _collect_swf_dirs(swf_dir)
    swf_files: list[Path] = []
    for d in scan_dirs:
        swf_files.extend(sorted(d.rglob("*.swf")))

    if not swf_files:
        if on_progress:
            on_progress(f"No SWF files found under {swf_dir}")
        return 0

    if on_progress:
        on_progress(f"Found {len(swf_files)} SWF files across {len(scan_dirs)} directories")

    total = 0
    db_path.parent.mkdir(parents=True, exist_ok=True)

    with BulkInserter(str(db_path), BULK_SCHEMA, fresh=True) as bulk:
        bulk.execute(CREATE_TABLES_DDL)
        buf = {c: [] for c in SHAPE_COLS}

        for idx, swf_path in enumerate(swf_files):
            try:
                doc = parse_swf_file(swf_path)
            except Exception as exc:
                _log.warning("Failed to parse %s: %s", swf_path.name, exc)
                continue

            rel_path = swf_path.name
            category, subcategory = _classify_swf(swf_path, swf_dir)
            tags = f"{category}/{subcategory}" if subcategory != category else category

            for shape_id, shape in doc.shapes.items():
                try:
                    svg = shape_to_svg(shape)
                except Exception:
                    svg = ""

                fill_colors = [fs.color.to_hex() for fs in shape.fill_styles if fs.color]
                name = f"{swf_path.stem}_shape{shape_id}"
                bounds = json.dumps(list(shape.bounds_px))
                path_count = sum(
                    1 for r in shape.records if isinstance(r, StyleChange) and r.has_move
                )

                try:
                    shape_blob = serialize_shape_records(shape)
                except Exception:
                    shape_blob = b""

                buf["name"].append(name)
                buf["source_swf"].append(rel_path)
                buf["source_tag"].append(int(shape_id))
                buf["bounds"].append(bounds)
                buf["fill_colors"].append(json.dumps(fill_colors))
                buf["path_count"].append(path_count)
                buf["shape_data"].append(shape_blob)
                buf["svg_preview"].append(svg)
                buf["tags"].append(tags)
                buf["user_added"].append(0)
                total += 1

                if len(buf["name"]) >= 500:
                    bulk.add_chunk("shapes", buf)
                    for lst in buf.values():
                        lst.clear()

            if on_progress and (idx + 1) % 10 == 0:
                on_progress(f"  {idx + 1}/{len(swf_files)} files — {total} shapes...")

        if buf["name"]:
            bulk.add_chunk("shapes", buf)

        bulk.rebuild_fts("shapes_fts")

    if on_progress:
        on_progress(f"Indexed {total} shapes from {len(swf_files)} SWF files")

    return total


def main():
    parser = argparse.ArgumentParser(description="Build SWF shape library index")
    parser.add_argument("--game", default="fo4")
    parser.add_argument("--extracted-dir", dest="extracted_dir", default=None)
    parser.add_argument("--db-path", dest="db_path", required=True)
    parser.add_argument("--embeddings", action="store_true")
    args = parser.parse_args()

    logging.basicConfig(level=logging.INFO)

    if args.game not in GAME_PROFILES:
        print(f"error: unknown game {args.game!r}. Valid: {', '.join(GAME_PROFILES)}",
              file=sys.stderr)
        sys.exit(1)

    profile = GAME_PROFILES[args.game]

    extracted = args.extracted_dir

    if not extracted:
        print(f"error: {profile.env_var_name} not supplied — pass --extracted-dir",
              file=sys.stderr)
        sys.exit(1)

    swf_dir = Path(extracted) / "Interface"
    if not swf_dir.is_dir():
        print(f"error: Interface directory not found: {swf_dir}", file=sys.stderr)
        sys.exit(1)

    db_path = Path(args.db_path)

    count = extract_shapes(swf_dir, db_path, on_progress=print)

    print(f"Database: {db_path} ({count} shapes)")


if __name__ == "__main__":
    main()
