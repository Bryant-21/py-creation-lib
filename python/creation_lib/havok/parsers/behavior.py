"""Behavior graph metadata parser binding for the native Havok backend."""
from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class BehaviorData:
    """Parsed behavior graph metadata."""

    events: list[str] = field(default_factory=list)
    variables: list[tuple[str, str]] = field(default_factory=list)
    sequences: list[str] = field(default_factory=list)
    transitions: list[tuple[str, str]] = field(default_factory=list)
    node_count: int = 0
    node_classes: list[str] = field(default_factory=list)


def parse_behavior(xml_path: Path) -> BehaviorData:
    """Parse behavior XML via the Rust Havok backend."""
    from creation_lib.havok.native_runtime import parse_behavior_xml_native

    try:
        data = parse_behavior_xml_native(Path(xml_path).read_text(encoding="utf-8"))
    except Exception:
        return BehaviorData()
    return BehaviorData(
        events=[str(value) for value in data.get("events", [])],
        variables=[tuple(value) for value in data.get("variables", [])],
        sequences=[str(value) for value in data.get("sequences", [])],
        transitions=[tuple(value) for value in data.get("transitions", [])],
        node_count=int(data.get("node_count", 0)),
        node_classes=[str(value) for value in data.get("node_classes", [])],
    )
