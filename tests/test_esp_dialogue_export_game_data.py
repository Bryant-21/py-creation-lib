"""Exported voice types match the voice files shipped for official FO4 plugins.

Uses the local Fallout 4 install: FO4_DIR, or MODKIT_TEST_FO4_DATA for the Data folder.
"""

from __future__ import annotations

import os
from collections import defaultdict
from pathlib import Path

import pytest

from creation_lib.ba2 import native_runtime as ba2_native_runtime
from creation_lib.esp.dialogue_export import export_dialogue_lines

pytestmark = pytest.mark.integration


def _fo4_data_dir() -> Path:
    raw = os.environ.get("MODKIT_TEST_FO4_DATA") or os.environ.get("FO4_DIR")
    path = Path(raw) if raw else None
    if path is not None and path.name.lower() != "data":
        path = path / "Data"
    if path is None or not path.is_dir():
        pytest.skip("FO4_DIR not configured (set env var or .env)")
    return path


def _shipped_voice_types(archive: Path, plugin: str) -> dict[str, set[str]]:
    prefix = f"sound/voice/{plugin.lower()}/"
    shipped: dict[str, set[str]] = defaultdict(set)
    for member in ba2_native_runtime.list_archive(str(archive)) or []:
        member = member.lower().replace("\\", "/")
        if member.startswith(prefix) and member.endswith(".fuz"):
            voice_type, name = member[len(prefix):].split("/", 1)
            shipped[name[: -len(".fuz")]].add(voice_type)
    return shipped


@pytest.mark.parametrize(
    ("plugin", "min_precision", "min_recall"),
    [("DLCworkshop03.esm", 0.99, 0.99), ("DLCRobot.esm", 0.93, 0.90)],
)
def test_voice_types_match_shipped_voice_files(plugin: str, min_precision: float, min_recall: float) -> None:
    data_dir = _fo4_data_dir()
    archive = data_dir / f"{Path(plugin).stem} - Voices_en.ba2"
    if not (data_dir / plugin).is_file() or not archive.is_file():
        pytest.skip(f"{plugin} or its voice archive is not installed")
    shipped = _shipped_voice_types(archive, plugin)

    exported: dict[str, set[str]] = defaultdict(set)
    for line in export_dialogue_lines(data_dir / plugin, data_dir=data_dir):
        voices = exported[line.file_name.lower()]
        if line.voice_type:
            voices.add(line.voice_type.lower())

    voiced = exported.keys() & shipped.keys()
    matched = sum(len(exported[name] & shipped[name]) for name in voiced)
    precision = matched / max(1, sum(len(exported[name]) for name in voiced))
    recall = matched / max(1, sum(len(shipped[name]) for name in voiced))
    assert precision >= min_precision and recall >= min_recall, f"precision={precision:.3f} recall={recall:.3f}"
