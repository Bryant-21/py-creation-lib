"""Character metadata parser binding for the native Havok backend."""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass
class CharacterData:
    """Parsed character metadata."""

    rig_name: str = ""
    behavior_filename: str = ""
    model_up: str = ""
    model_forward: str = ""
    model_right: str = ""


def parse_character(xml_path: Path) -> CharacterData:
    """Parse character XML via the Rust Havok backend."""
    from creation_lib.havok.native_runtime import parse_character_xml_native

    try:
        data = parse_character_xml_native(Path(xml_path).read_text(encoding="utf-8"))
    except Exception:
        return CharacterData()
    return CharacterData(
        rig_name=str(data.get("rig_name", "")),
        behavior_filename=str(data.get("behavior_filename", "")),
        model_up=str(data.get("model_up", "")),
        model_forward=str(data.get("model_forward", "")),
        model_right=str(data.get("model_right", "")),
    )
