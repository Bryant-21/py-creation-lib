"""Read FO3/FNV .kf NIF files and produce AnimationClip objects.

.kf files are standard NIF files with NiControllerSequence as the root block.
This module extracts animation data (bone transforms, float channels, events)
into the engine-neutral AnimationClip intermediate representation.
"""
from __future__ import annotations

import math
from pathlib import Path

from creation_lib.nif.nif_file import NifFile, NifBlock

from creation_lib.animation.models import (
    AnimationClip,
    AnimationEvent,
    AnimationKeyframe,
    BoneChannel,
    FloatChannel,
)

# NIF rotation-type enum values
_ROT_LINEAR = 1
_ROT_QUADRATIC = 2
_ROT_TBC = 3
_ROT_XYZ = 4

# NIF interpolation enum values
_INTERP_LINEAR = 1
_INTERP_QUADRATIC = 2
_INTERP_TBC = 3

# NIF cycle-type enum values → our string names
_CYCLE_TYPE_MAP = {
    0: "loop",
    1: "clamp",
    2: "reverse",
}

_INTERP_NAME = {
    _INTERP_LINEAR: "linear",
    _INTERP_QUADRATIC: "quadratic",
    _INTERP_TBC: "tbc",
}


def read_kf(path: str | Path) -> AnimationClip:
    """Read a FO3/FNV .kf file into an AnimationClip.

    Raises ValueError if the file has no NiControllerSequence block.
    """
    nif = NifFile.load(str(path))

    # Find the NiControllerSequence root (usually block 0)
    seq_block: NifBlock | None = None
    for block in nif.blocks:
        if block.type_name == "NiControllerSequence":
            seq_block = block
            break
    if seq_block is None:
        raise ValueError(f"No NiControllerSequence found in {path}")

    name = seq_block.get_field("Name") or ""
    frequency = seq_block.get_field("Frequency") or 1.0
    start_time = seq_block.get_field("Start Time") or 0.0
    stop_time = seq_block.get_field("Stop Time") or 0.0
    duration = stop_time - start_time
    cycle_raw = seq_block.get_field("Cycle Type")
    cycle_type = _CYCLE_TYPE_MAP.get(cycle_raw, "clamp")
    accum_root = seq_block.get_field("Accum Root Name") or ""

    # --- Events from NiTextKeyExtraData ---
    events: list[AnimationEvent] = []
    text_keys_ref = seq_block.get_field("Text Keys")
    if text_keys_ref is not None and text_keys_ref >= 0:
        tk_block = nif.get_block(text_keys_ref)
        if tk_block is not None and tk_block.type_name == "NiTextKeyExtraData":
            raw_keys = tk_block.get_field("Text Keys") or []
            for tk in raw_keys:
                events.append(AnimationEvent(
                    time=tk.get("Time", 0.0),
                    text=tk.get("Value", ""),
                ))

    # --- Controlled blocks → channels ---
    bone_channels: list[BoneChannel] = []
    float_channels: list[FloatChannel] = []
    warnings: list[str] = []

    controlled = seq_block.get_field("Controlled Blocks") or []
    for cb in controlled:
        interp_ref = cb.get("Interpolator", -1)
        node_name = cb.get("Node Name") or ""
        ctrl_type = cb.get("Controller Type") or ""
        priority = cb.get("Priority", 26)
        prop_type = cb.get("Property Type")

        if interp_ref < 0:
            warnings.append(f"No interpolator for '{node_name}'")
            continue

        interp_block = nif.get_block(interp_ref)
        if interp_block is None:
            warnings.append(f"Invalid interpolator ref {interp_ref} for '{node_name}'")
            continue

        if interp_block.type_name == "NiTransformInterpolator":
            channel = _read_transform_channel(
                nif, interp_block, node_name, priority, warnings
            )
            if channel is not None:
                bone_channels.append(channel)

        elif interp_block.type_name in ("NiFloatInterpolator", "NiBoolInterpolator"):
            fc = _read_float_channel(
                nif, interp_block, node_name, ctrl_type, prop_type, warnings
            )
            if fc is not None:
                float_channels.append(fc)

        else:
            # Other interpolator types — record but skip
            warnings.append(
                f"Unsupported interpolator {interp_block.type_name} "
                f"for '{node_name}'"
            )

    return AnimationClip(
        name=name,
        duration=duration,
        cycle_type=cycle_type,
        frequency=frequency,
        accum_root=accum_root,
        channels=tuple(bone_channels),
        float_channels=tuple(float_channels),
        events=tuple(events),
        source_format="kf",
        warnings=tuple(warnings),
    )


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------


def _read_transform_channel(
    nif: NifFile,
    interp_block: NifBlock,
    bone_name: str,
    priority: int,
    warnings: list[str],
) -> BoneChannel | None:
    """Parse NiTransformInterpolator → NiTransformData into a BoneChannel."""
    data_ref = interp_block.get_field("Data")
    if data_ref is None or data_ref < 0:
        # Interpolator with no data — bone has a static pose from the
        # Transform field; we skip it (no keyframes to represent).
        return BoneChannel(bone_name=bone_name, priority=priority)

    data_block = nif.get_block(data_ref)
    if data_block is None or data_block.type_name != "NiTransformData":
        warnings.append(
            f"Expected NiTransformData at block {data_ref} for '{bone_name}', "
            f"got {data_block.type_name if data_block else 'None'}"
        )
        return BoneChannel(bone_name=bone_name, priority=priority)

    rotations = _read_rotation_keys(data_block, bone_name, warnings)
    translations = _read_translation_keys(data_block)
    scales = _read_scale_keys(data_block)

    return BoneChannel(
        bone_name=bone_name,
        priority=priority,
        rotations=tuple(rotations),
        translations=tuple(translations),
        scales=tuple(scales),
    )


def _read_rotation_keys(
    data_block: NifBlock, bone_name: str, warnings: list[str]
) -> list[AnimationKeyframe]:
    """Extract rotation keyframes from NiTransformData."""
    rot_type = data_block.get_field("Rotation Type")
    num_keys = data_block.get_field("Num Rotation Keys") or 0

    if num_keys == 0 and rot_type != _ROT_XYZ:
        return []

    if rot_type == _ROT_XYZ:
        return _read_xyz_rotation_keys(data_block, bone_name, warnings)

    # Quaternion-based rotation types (LINEAR, QUADRATIC, TBC)
    quat_keys = data_block.get_field("Quaternion Keys") or []
    keyframes: list[AnimationKeyframe] = []

    interp_name = _INTERP_NAME.get(rot_type, "linear")

    for qk in quat_keys:
        time = qk.get("Time", 0.0)
        val = qk.get("Value", {})
        # NIF stores (w, x, y, z) — convert to (x, y, z, w) for Havok convention
        w = val.get("w", 1.0)
        x = val.get("x", 0.0)
        y = val.get("y", 0.0)
        z = val.get("z", 0.0)

        kf = AnimationKeyframe(
            time=time,
            value=(x, y, z, w),
            interpolation=interp_name,
        )
        keyframes.append(kf)

    return keyframes


def _read_xyz_rotation_keys(
    data_block: NifBlock, bone_name: str, warnings: list[str]
) -> list[AnimationKeyframe]:
    """Convert XYZ Euler rotation keys to quaternion keyframes.

    XYZ_ROTATION_KEY stores three separate float-key arrays (one per axis).
    We sample at each unique time and convert Euler angles → quaternion.
    """
    xyz_data = data_block.get_field("XYZ Rotations")
    if not xyz_data or len(xyz_data) < 3:
        warnings.append(f"Missing XYZ Rotations data for '{bone_name}'")
        return []

    # Collect all unique times across the three axes
    axis_keys: list[list[tuple[float, float]]] = []
    all_times: set[float] = set()
    for axis_entry in xyz_data[:3]:
        keys = axis_entry.get("Keys", [])
        pairs = []
        for k in keys:
            t = k.get("Time", 0.0)
            v = k.get("Value", 0.0)
            pairs.append((t, v))
            all_times.add(t)
        axis_keys.append(pairs)

    if not all_times:
        return []

    sorted_times = sorted(all_times)
    keyframes: list[AnimationKeyframe] = []

    for t in sorted_times:
        angles = []
        for pairs in axis_keys:
            angles.append(_sample_at_time(pairs, t))

        # Euler XYZ → quaternion
        qx, qy, qz, qw = _euler_to_quat(angles[0], angles[1], angles[2])
        keyframes.append(AnimationKeyframe(
            time=t,
            value=(qx, qy, qz, qw),
            interpolation="linear",
        ))

    return keyframes


def _sample_at_time(pairs: list[tuple[float, float]], t: float) -> float:
    """Linearly interpolate a value at time *t* from sorted (time, value) pairs."""
    if not pairs:
        return 0.0
    if len(pairs) == 1 or t <= pairs[0][0]:
        return pairs[0][1]
    if t >= pairs[-1][0]:
        return pairs[-1][1]
    for i in range(len(pairs) - 1):
        t0, v0 = pairs[i]
        t1, v1 = pairs[i + 1]
        if t0 <= t <= t1:
            if abs(t1 - t0) < 1e-9:
                return v0
            frac = (t - t0) / (t1 - t0)
            return v0 + frac * (v1 - v0)
    return pairs[-1][1]


def _euler_to_quat(rx: float, ry: float, rz: float) -> tuple[float, float, float, float]:
    """Convert Euler angles (radians, XYZ order) to quaternion (x, y, z, w)."""
    cx, sx = math.cos(rx / 2), math.sin(rx / 2)
    cy, sy = math.cos(ry / 2), math.sin(ry / 2)
    cz, sz = math.cos(rz / 2), math.sin(rz / 2)

    w = cx * cy * cz + sx * sy * sz
    x = sx * cy * cz - cx * sy * sz
    y = cx * sy * cz + sx * cy * sz
    z = cx * cy * sz - sx * sy * cz
    return (x, y, z, w)


def _read_translation_keys(data_block: NifBlock) -> list[AnimationKeyframe]:
    """Extract translation keyframes from NiTransformData."""
    trans = data_block.get_field("Translations")
    if not trans or trans.get("Num Keys", 0) == 0:
        return []

    interp_val = trans.get("Interpolation", _INTERP_LINEAR)
    interp_name = _INTERP_NAME.get(interp_val, "linear")
    keys = trans.get("Keys", [])
    keyframes: list[AnimationKeyframe] = []

    for k in keys:
        time = k.get("Time", 0.0)
        val = k.get("Value", {})
        x = val.get("x", 0.0)
        y = val.get("y", 0.0)
        z = val.get("z", 0.0)

        forward = None
        backward = None
        tbc = None
        if interp_name == "quadratic":
            fwd = k.get("Forward")
            bwd = k.get("Backward")
            if fwd:
                forward = (fwd.get("x", 0.0), fwd.get("y", 0.0), fwd.get("z", 0.0))
            if bwd:
                backward = (bwd.get("x", 0.0), bwd.get("y", 0.0), bwd.get("z", 0.0))
        elif interp_name == "tbc":
            tbc_val = k.get("TBC")
            if tbc_val:
                tbc = (tbc_val.get("t", 0.0), tbc_val.get("b", 0.0), tbc_val.get("c", 0.0))

        keyframes.append(AnimationKeyframe(
            time=time,
            value=(x, y, z),
            interpolation=interp_name,
            forward=forward,
            backward=backward,
            tbc=tbc,
        ))

    return keyframes


def _read_scale_keys(data_block: NifBlock) -> list[AnimationKeyframe]:
    """Extract scale keyframes from NiTransformData."""
    scales = data_block.get_field("Scales")
    if not scales or scales.get("Num Keys", 0) == 0:
        return []

    keys = scales.get("Keys", [])
    # Determine interpolation from key structure
    interp_name = "linear"
    if keys and "Forward" in keys[0]:
        interp_name = "quadratic"
    elif keys and "TBC" in keys[0]:
        interp_name = "tbc"

    keyframes: list[AnimationKeyframe] = []
    for k in keys:
        time = k.get("Time", 0.0)
        val = k.get("Value", 1.0)

        forward = None
        backward = None
        tbc = None
        if interp_name == "quadratic":
            fwd = k.get("Forward")
            bwd = k.get("Backward")
            if fwd is not None:
                forward = (fwd,)
            if bwd is not None:
                backward = (bwd,)
        elif interp_name == "tbc":
            tbc_val = k.get("TBC")
            if tbc_val:
                tbc = (tbc_val.get("t", 0.0), tbc_val.get("b", 0.0), tbc_val.get("c", 0.0))

        keyframes.append(AnimationKeyframe(
            time=time,
            value=(val,),
            interpolation=interp_name,
            forward=forward,
            backward=backward,
            tbc=tbc,
        ))

    return keyframes


def _read_float_channel(
    nif: NifFile,
    interp_block: NifBlock,
    target_name: str,
    controller_type: str,
    property_type: str | None,
    warnings: list[str],
) -> FloatChannel | None:
    """Parse NiFloatInterpolator or NiBoolInterpolator into a FloatChannel."""
    data_ref = interp_block.get_field("Data")
    if data_ref is None or data_ref < 0:
        return None

    data_block = nif.get_block(data_ref)
    if data_block is None:
        warnings.append(
            f"Invalid data ref {data_ref} for float channel '{target_name}'"
        )
        return None

    keys_data = data_block.get_field("Data")
    if keys_data is None:
        # Try Keys field directly
        keys_data = data_block.get_field("Keys")

    keyframes: list[AnimationKeyframe] = []
    if isinstance(keys_data, dict):
        raw_keys = keys_data.get("Keys", [])
        for k in raw_keys:
            keyframes.append(AnimationKeyframe(
                time=k.get("Time", 0.0),
                value=(k.get("Value", 0.0),),
            ))
    elif isinstance(keys_data, list):
        for k in keys_data:
            keyframes.append(AnimationKeyframe(
                time=k.get("Time", 0.0),
                value=(k.get("Value", 0.0),),
            ))

    if not keyframes:
        return None

    # Infer property type from controller type if not set
    prop = property_type or ""
    if not prop:
        if "Visibility" in controller_type:
            prop = "visibility"
        elif "Alpha" in controller_type:
            prop = "alpha"
        else:
            prop = "float"

    return FloatChannel(
        target_name=target_name,
        property_type=prop,
        controller_type=controller_type,
        keyframes=tuple(keyframes),
    )
