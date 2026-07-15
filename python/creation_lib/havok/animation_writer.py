"""Animation XML writer binding for the native Havok backend."""
from __future__ import annotations

import dataclasses
import math
from pathlib import Path

from creation_lib.animation.models import AnimationClip

SAMPLE_RATE = 30


def _slerp(
    q0: tuple[float, ...], q1: tuple[float, ...], t: float
) -> tuple[float, float, float, float]:
    """Compatibility helper retained for legacy tests; production writing is native."""
    x0, y0, z0, w0 = q0
    x1, y1, z1, w1 = q1
    dot = x0 * x1 + y0 * y1 + z0 * z1 + w0 * w1
    if dot < 0.0:
        x1, y1, z1, w1 = -x1, -y1, -z1, -w1
        dot = -dot
    dot = min(dot, 1.0)
    if dot > 0.9995:
        rx = x0 + t * (x1 - x0)
        ry = y0 + t * (y1 - y0)
        rz = z0 + t * (z1 - z0)
        rw = w0 + t * (w1 - w0)
    else:
        theta = math.acos(dot)
        sin_theta = math.sin(theta)
        a = math.sin((1.0 - t) * theta) / sin_theta
        b = math.sin(t * theta) / sin_theta
        rx = a * x0 + b * x1
        ry = a * y0 + b * y1
        rz = a * z0 + b * z1
        rw = a * w0 + b * w1
    length = math.sqrt(rx * rx + ry * ry + rz * rz + rw * rw)
    if length > 0.0:
        rx /= length
        ry /= length
        rz /= length
        rw /= length
    return (rx, ry, rz, rw)


def _lerp_tuple(
    a: tuple[float, ...], b: tuple[float, ...], t: float
) -> tuple[float, ...]:
    """Compatibility helper retained for legacy tests; production writing is native."""
    return tuple(a_i + t * (b_i - a_i) for a_i, b_i in zip(a, b))


def write_animation_xml(
    clip: AnimationClip,
    skeleton_bone_names: list[str],
    output_path: str | Path,
) -> None:
    """Write an AnimationClip as Havok XML via the Rust Havok backend."""
    from creation_lib.havok.native_runtime import write_animation_xml_native

    xml = write_animation_xml_native(dataclasses.asdict(clip), list(skeleton_bone_names))
    Path(output_path).write_text(xml, encoding="utf-8")
