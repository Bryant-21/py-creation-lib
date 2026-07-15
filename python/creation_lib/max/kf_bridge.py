from __future__ import annotations

from dataclasses import asdict
from typing import Any

from creation_lib.animation.models import (
    AnimationClip,
    AnimationEvent,
    AnimationKeyframe,
    BoneChannel,
    FloatChannel,
)


_CYCLE_TYPE_MAP = {
    "loop": "loop",
    "cycle_loop": "loop",
    "clamp": "clamp",
    "cycle_clamp": "clamp",
    "reverse": "reverse",
    "cycle_reverse": "reverse",
}


def animation_document_from_clip(
    clip: AnimationClip,
    *,
    source_path: str = "",
) -> dict[str, Any]:
    return {
        "kind": "kf_animation_document",
        "format_version": 1,
        "source_path": source_path,
        "name": clip.name,
        "duration": clip.duration,
        "frequency": clip.frequency,
        "cycle_type": _normalize_cycle_type(clip.cycle_type),
        "accum_root": clip.accum_root,
        "channels": [asdict(channel) for channel in clip.channels],
        "float_channels": [asdict(channel) for channel in clip.float_channels],
        "events": [asdict(event) for event in clip.events],
        "source_format": clip.source_format,
        "warnings": list(clip.warnings),
        "native_fps": clip.native_fps,
        "is_additive": clip.is_additive,
        "track_to_bone_indices": list(clip.track_to_bone_indices),
        "original_skeleton_name": clip.original_skeleton_name,
        "extracted_motion_ref": clip.extracted_motion_ref,
    }


def clip_from_animation_document(document: dict[str, Any]) -> AnimationClip:
    return AnimationClip(
        name=str(document.get("name") or ""),
        duration=float(document.get("duration") or 0.0),
        frequency=float(document.get("frequency") or 1.0),
        cycle_type=_normalize_cycle_type(document.get("cycle_type")),
        accum_root=str(document.get("accum_root") or ""),
        channels=tuple(_bone_channel(value) for value in document.get("channels") or []),
        float_channels=tuple(
            _float_channel(value) for value in document.get("float_channels") or []
        ),
        events=tuple(_animation_event(value) for value in document.get("events") or []),
        source_format=str(document.get("source_format") or ""),
        warnings=tuple(str(value) for value in document.get("warnings") or []),
        native_fps=float(document.get("native_fps") or 30.0),
        is_additive=bool(document.get("is_additive", False)),
        track_to_bone_indices=tuple(
            int(value) for value in document.get("track_to_bone_indices") or []
        ),
        original_skeleton_name=str(document.get("original_skeleton_name") or ""),
        extracted_motion_ref=str(document.get("extracted_motion_ref") or ""),
    )


def _bone_channel(value: dict[str, Any]) -> BoneChannel:
    return BoneChannel(
        bone_name=str(value.get("bone_name") or ""),
        priority=int(value.get("priority") or 26),
        rotations=tuple(_keyframe(item) for item in value.get("rotations") or []),
        translations=tuple(_keyframe(item) for item in value.get("translations") or []),
        scales=tuple(_keyframe(item) for item in value.get("scales") or []),
    )


def _float_channel(value: dict[str, Any]) -> FloatChannel:
    return FloatChannel(
        target_name=str(value.get("target_name") or ""),
        property_type=str(value.get("property_type") or ""),
        controller_type=str(value.get("controller_type") or ""),
        keyframes=tuple(_keyframe(item) for item in value.get("keyframes") or []),
    )


def _keyframe(value: dict[str, Any]) -> AnimationKeyframe:
    return AnimationKeyframe(
        time=float(value.get("time") or 0.0),
        value=_tuple_or_none(value.get("value")) or (),
        interpolation=str(value.get("interpolation") or "linear"),
        forward=_tuple_or_none(value.get("forward")),
        backward=_tuple_or_none(value.get("backward")),
        tbc=_tuple_or_none(value.get("tbc")),
    )


def _animation_event(value: dict[str, Any]) -> AnimationEvent:
    return AnimationEvent(
        time=float(value.get("time") or 0.0),
        text=str(value.get("text") or ""),
    )


def _tuple_or_none(value: Any) -> tuple[Any, ...] | None:
    if value is None:
        return None
    if isinstance(value, tuple):
        return value
    if isinstance(value, list):
        return tuple(value)
    return (value,)


def _normalize_cycle_type(value: Any) -> str:
    key = str(value or "clamp").strip().lower()
    return _CYCLE_TYPE_MAP.get(key, "clamp")
