"""Animation metadata parser binding for the native Havok backend."""
from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class AnimationData:
    """Parsed animation metadata and frame 0 transforms."""

    duration: float = 0.0
    bone_count: int = 0
    frame_count: int = 0
    compression_type: str = "unknown"
    float_track_count: int = 0
    annotation_tracks: list[dict] = field(default_factory=list)
    frame0_transforms: bytes | None = None


def parse_animation(xml_path: Path) -> AnimationData:
    """Parse animation XML via the Rust Havok backend."""
    from creation_lib.havok.native_runtime import parse_animation_xml_native

    try:
        data = parse_animation_xml_native(Path(xml_path).read_text(encoding="utf-8"))
    except Exception:
        return AnimationData()
    frame0 = data.get("frame0_transforms")
    return AnimationData(
        duration=float(data.get("duration", 0.0)),
        bone_count=int(data.get("bone_count", 0)),
        frame_count=int(data.get("frame_count", 0)),
        compression_type=str(data.get("compression_type", "unknown")),
        float_track_count=int(data.get("float_track_count", 0)),
        annotation_tracks=list(data.get("annotation_tracks", [])),
        frame0_transforms=bytes(frame0) if frame0 is not None else None,
    )
