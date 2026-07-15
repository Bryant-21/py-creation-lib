"""Project metadata parser binding for the native Havok backend."""
from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class ProjectData:
    """Parsed project metadata."""

    character_filenames: list[str] = field(default_factory=list)


def parse_project(xml_path: Path) -> ProjectData:
    """Parse project XML via the Rust Havok backend."""
    from creation_lib.havok.native_runtime import parse_project_xml_native

    try:
        data = parse_project_xml_native(Path(xml_path).read_text(encoding="utf-8"))
    except Exception:
        return ProjectData()
    return ProjectData(
        character_filenames=[str(value) for value in data.get("character_filenames", [])]
    )
