"""Skeleton metadata parser binding for the native Havok backend."""
from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class SkeletonData:
    """Parsed skeleton metadata."""

    name: str = ""
    bone_count: int = 0
    bone_names: list[str] = field(default_factory=list)
    parent_indices: list[int] = field(default_factory=list)
    reference_pose: list[dict] = field(default_factory=list)
    lock_translation: list[bool] = field(default_factory=list)
    float_count: int = 0
    float_slots: list[str] = field(default_factory=list)
    reference_floats: list[float] = field(default_factory=list)
    partition_names: list[str] = field(default_factory=list)


def parse_skeleton(xml_path: Path) -> SkeletonData:
    """Parse skeleton XML via the Rust Havok backend."""
    from creation_lib.havok.native_runtime import parse_skeleton_xml_native

    try:
        data = parse_skeleton_xml_native(Path(xml_path).read_text(encoding="utf-8"))
    except Exception:
        return SkeletonData()
    return SkeletonData(
        name=str(data.get("name", "")),
        bone_count=int(data.get("bone_count", 0)),
        bone_names=[str(value) for value in data.get("bone_names", [])],
        parent_indices=[int(value) for value in data.get("parent_indices", [])],
        reference_pose=list(data.get("reference_pose", [])),
        lock_translation=[bool(value) for value in data.get("lock_translation", [])],
        float_count=int(data.get("float_count", 0)),
        float_slots=[str(value) for value in data.get("float_slots", [])],
        reference_floats=[float(value) for value in data.get("reference_floats", [])],
        partition_names=[str(value) for value in data.get("partition_names", [])],
    )
