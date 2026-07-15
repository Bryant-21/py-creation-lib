"""Animation clip extraction bindings for the native Havok backend."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from creation_lib.animation.models import (
    AnimationClip,
    AnimationEvent,
    AnimationKeyframe,
    BoneChannel,
    FloatChannel,
)
from creation_lib.havok.parsers.skeleton import SkeletonData


def _keyframes_from_native(items: list[dict[str, Any]]) -> tuple[AnimationKeyframe, ...]:
    return tuple(
        AnimationKeyframe(
            time=float(item.get("time", 0.0)),
            value=tuple(item.get("value", ())),
            interpolation=str(item.get("interpolation", "linear")),
            forward=tuple(item["forward"]) if item.get("forward") is not None else None,
            backward=tuple(item["backward"]) if item.get("backward") is not None else None,
            tbc=tuple(item["tbc"]) if item.get("tbc") is not None else None,
        )
        for item in items
    )


def _clip_from_native_data(
    data: dict[str, Any],
    skeleton: SkeletonData | None = None,
) -> AnimationClip | None:
    if not data:
        return None

    channels: list[BoneChannel] = []
    for channel in data.get("channels", []):
        bone_name = str(channel.get("bone_name", ""))
        if skeleton is not None and bone_name.startswith("track_"):
            try:
                track_index = int(bone_name[len("track_"):])
            except ValueError:
                track_index = -1
            if 0 <= track_index < len(skeleton.bone_names):
                bone_name = skeleton.bone_names[track_index]

        channels.append(
            BoneChannel(
                bone_name=bone_name,
                priority=int(channel.get("priority", 26)),
                rotations=_keyframes_from_native(channel.get("rotations", [])),
                translations=_keyframes_from_native(channel.get("translations", [])),
                scales=_keyframes_from_native(channel.get("scales", [])),
            )
        )

    float_channels: list[FloatChannel] = []
    for channel in data.get("float_channels", []):
        float_channels.append(
            FloatChannel(
                target_name=str(channel.get("target_name", "")),
                property_type=str(channel.get("property_type", "")),
                controller_type=str(channel.get("controller_type", "")),
                keyframes=_keyframes_from_native(channel.get("keyframes", [])),
            )
        )

    events = tuple(
        AnimationEvent(time=float(event.get("time", 0.0)), text=str(event.get("text", "")))
        for event in data.get("events", [])
    )

    return AnimationClip(
        name=str(data.get("name", "")),
        duration=float(data.get("duration", 0.0)),
        cycle_type=data.get("cycle_type", "clamp"),
        frequency=float(data.get("frequency", 1.0)),
        accum_root=str(data.get("accum_root", "")),
        channels=tuple(channels),
        float_channels=tuple(float_channels),
        events=events,
        source_format=str(data.get("source_format", "hkx")),
        warnings=tuple(str(warning) for warning in data.get("warnings", [])),
        native_fps=float(data.get("native_fps", 30.0)),
        is_additive=bool(data.get("is_additive", False)),
        track_to_bone_indices=tuple(int(i) for i in data.get("track_to_bone_indices", [])),
        original_skeleton_name=str(data.get("original_skeleton_name", "")),
        extracted_motion_ref=str(data.get("extracted_motion_ref", "")),
    )


def infer_clip_fps(clip: AnimationClip, default: float = 30.0) -> float:
    """Return native_fps, falling back to keyframe spacing for legacy clips."""
    if clip.native_fps and clip.native_fps > 0.0:
        return clip.native_fps
    for channel in clip.channels:
        for series in (channel.rotations, channel.translations, channel.scales):
            if len(series) >= 2:
                delta = series[1].time - series[0].time
                if delta > 1e-6:
                    return 1.0 / delta
    return default


def infer_clip_fps_from_xml(_anim_obj: object, duration: float, default: float = 30.0) -> float:
    """Compatibility shim for callers that still import the old helper."""
    return default if duration <= 0.0 else default


def extract_clip(
    xml_path: str | Path,
    skeleton: SkeletonData | None = None,
) -> AnimationClip | None:
    """Extract a full animation clip via the Rust Havok backend."""
    from creation_lib.havok.native_runtime import extract_clip_native

    try:
        xml = Path(xml_path).read_text(encoding="utf-8")
        data = extract_clip_native(xml, None)
    except Exception:
        return None
    return _clip_from_native_data(data, skeleton=skeleton)
