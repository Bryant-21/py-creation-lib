"""Universal animation intermediate representation."""
from __future__ import annotations

from dataclasses import dataclass
from typing import Literal


@dataclass(frozen=True)
class AnimationKeyframe:
    """Single keyframe value at a point in time."""

    time: float
    value: tuple  # (qx,qy,qz,qw) for rotation, (x,y,z) for translation, (s,) for scale
    interpolation: str = "linear"  # "linear", "quadratic", "tbc"
    forward: tuple | None = None   # forward tangent (quadratic)
    backward: tuple | None = None  # backward tangent (quadratic)
    tbc: tuple | None = None       # tension, bias, continuity (tbc)


@dataclass(frozen=True)
class BoneChannel:
    """Animation data for a single bone."""

    bone_name: str
    priority: int = 26  # default Havok priority
    rotations: tuple[AnimationKeyframe, ...] = ()
    translations: tuple[AnimationKeyframe, ...] = ()
    scales: tuple[AnimationKeyframe, ...] = ()


@dataclass(frozen=True)
class FloatChannel:
    """Non-transform animation channel (visibility, alpha, material params)."""

    target_name: str
    property_type: str  # "visibility", "alpha", "material_param", "morph"
    controller_type: str = ""
    keyframes: tuple[AnimationKeyframe, ...] = ()


@dataclass(frozen=True)
class AnimationEvent:
    """Timestamped animation event/annotation."""

    time: float
    text: str


@dataclass(frozen=True)
class AnimationClip:
    """Universal intermediate representation for a single animation sequence.

    Produced by kf_reader (FO3/FNV .kf) or hkx_reader (FO4 .hkx).
    Consumed by kf_writer or hkx animation_writer.
    """

    name: str
    duration: float
    cycle_type: Literal["loop", "clamp", "reverse"] = "clamp"
    frequency: float = 1.0
    accum_root: str = ""
    channels: tuple[BoneChannel, ...] = ()
    float_channels: tuple[FloatChannel, ...] = ()
    events: tuple[AnimationEvent, ...] = ()
    source_format: str = ""  # "kf" or "hkx" — provenance
    warnings: tuple[str, ...] = ()
    native_fps: float = 30.0
    is_additive: bool = False
    track_to_bone_indices: tuple[int, ...] = ()
    original_skeleton_name: str = ""
    # Pointer reference (e.g. "#0006") to the extracted-motion reference frame
    # object in the source .hkx, or "" if the source carries no extracted motion.
    # Without this the writer hard-codes #null, silently flattening forward
    # locomotion / root displacement on round-trip.
    extracted_motion_ref: str = ""
