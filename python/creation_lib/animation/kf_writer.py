"""Write AnimationClip objects to FO3/FNV .kf NIF files.

Produces standard NIF files with NiControllerSequence as the root block,
NiTextKeyExtraData for events, and NiTransformInterpolator/NiTransformData
pairs for each bone channel.
"""
from __future__ import annotations

from pathlib import Path

from creation_lib.core.game_profiles import get_profile
from creation_lib.nif.nif_file import NifFile, NifBlock, NifHeader

from creation_lib.animation.models import (
    AnimationClip,
    AnimationKeyframe,
    BoneChannel,
    FloatChannel,
)

# Cycle type string → NIF enum value
# NOTE: The reader maps 0→loop, 1→clamp, 2→reverse.  This matches
# the binary values observed in shipped FO3/FNV .kf files (which differ
# from the nif.xml enum names).
_CYCLE_TYPE_ENUM = {
    "loop": 0,
    "clamp": 1,
    "reverse": 2,
}

# Interpolation name → NIF KeyType enum
_INTERP_ENUM = {
    "linear": 1,
    "quadratic": 2,
    "tbc": 3,
}


def write_kf(clip: AnimationClip, path: str | Path, game: str = "fo3") -> None:
    """Write an AnimationClip to a .kf NIF file.

    Parameters
    ----------
    clip : AnimationClip
        The animation to write.
    path : str or Path
        Output file path.
    game : str
        Game target: ``"fo3"`` or ``"fnv"``.  Controls NIF header versions.
    """
    nif = NifFile()
    profile = get_profile(game)

    # --- Header ---
    nif.header = NifHeader()
    nif.header.version = profile.nif_version
    nif.header.user_version = profile.user_version
    nif.header.bs_version = profile.bs_version_range[0]
    nif.header.endian_type = 1  # little-endian
    nif.header.creator = "modkit21"
    nif.header.export_info = []

    # We will build blocks manually and use NifFile.add_block() to keep
    # header metadata (type index, sizes) in sync.

    # Count how many blocks we will produce so we can compute refs.
    # Layout:
    #   0  NiControllerSequence
    #   1  NiTextKeyExtraData
    #   2+ For each BoneChannel:  NiTransformInterpolator, NiTransformData (if has keys)
    #      For each FloatChannel: NiFloatInterpolator, NiFloatData (if has keys)
    # We need to pre-calculate block indices for the Controlled Blocks refs.

    block_idx = 2  # first available index after seq + text keys
    interp_refs: list[int] = []  # interpolator block index per channel
    channel_meta: list[dict] = []  # metadata for each controlled block entry

    # Pre-calculate bone channel block indices
    for ch in clip.channels:
        interp_refs.append(block_idx)
        has_data = bool(ch.rotations or ch.translations or ch.scales)
        channel_meta.append({
            "interp_idx": block_idx,
            "node_name": ch.bone_name,
            "priority": ch.priority,
            "controller_type": "NiTransformController",
            "property_type": "",
            "is_transform": True,
            "has_data": has_data,
        })
        block_idx += 1  # interpolator
        if has_data:
            block_idx += 1  # data

    # Pre-calculate float channel block indices
    for fc in clip.float_channels:
        interp_refs.append(block_idx)
        has_data = bool(fc.keyframes)
        channel_meta.append({
            "interp_idx": block_idx,
            "node_name": fc.target_name,
            "priority": 0,
            "controller_type": fc.controller_type or "NiFloatInterpController",
            "property_type": fc.property_type or "",
            "is_transform": False,
            "has_data": has_data,
        })
        block_idx += 1  # interpolator
        if has_data:
            block_idx += 1  # data

    total_controlled = len(channel_meta)

    # --- Build Controlled Blocks struct array ---
    controlled_blocks = []
    for meta in channel_meta:
        controlled_blocks.append({
            "Interpolator": meta["interp_idx"],
            "Controller": -1,
            "Priority": meta["priority"],
            "Node Name": meta["node_name"],
            "Property Type": meta["property_type"],
            "Controller Type": meta["controller_type"],
            "Controller ID": "",
            "Interpolator ID": "",
        })

    # --- Block 0: NiControllerSequence ---
    seq = nif.add_block("NiControllerSequence")
    seq.set_field("Name", clip.name)
    seq.set_field("Num Controlled Blocks", total_controlled)
    seq.set_field("Array Grow By", 0)
    seq.set_field("Controlled Blocks", controlled_blocks)
    seq.set_field("Weight", 1.0)
    seq.set_field("Text Keys", 1)  # ref to block 1
    seq.set_field("Cycle Type", _CYCLE_TYPE_ENUM.get(clip.cycle_type, 2))
    seq.set_field("Frequency", clip.frequency)
    seq.set_field("Start Time", 0.0)
    seq.set_field("Stop Time", clip.duration)
    seq.set_field("Manager", -1)
    seq.set_field("Accum Root Name", clip.accum_root)
    # Anim note arrays (bs_version > 28 for FNV)
    if nif.header.bs_version > 28:
        seq.set_field("Num Anim Note Arrays", 0)
        seq.set_field("Anim Note Arrays", [])

    # --- Block 1: NiTextKeyExtraData ---
    tk = nif.add_block("NiTextKeyExtraData")
    tk.set_field("Name", "")
    text_keys = []
    for evt in clip.events:
        text_keys.append({"Time": evt.time, "Value": evt.text})
    tk.set_field("Num Text Keys", len(text_keys))
    tk.set_field("Text Keys", text_keys)

    # --- Blocks 2+: Interpolator + Data pairs ---
    for i, ch in enumerate(clip.channels):
        has_data = bool(ch.rotations or ch.translations or ch.scales)
        data_ref = -1

        if has_data:
            # We need to know the data block index: it's the next block after
            # the interpolator we are about to create.
            data_ref = len(nif.blocks) + 1

        # NiTransformInterpolator
        interp = nif.add_block("NiTransformInterpolator")
        transform = _make_identity_transform(ch)
        interp.set_field("Transform", transform)
        interp.set_field("Data", data_ref)

        if has_data:
            # NiTransformData
            td = nif.add_block("NiTransformData")
            _populate_transform_data(td, ch)

    for fc in clip.float_channels:
        has_data = bool(fc.keyframes)
        data_ref = -1
        if has_data:
            data_ref = len(nif.blocks) + 1

        # NiFloatInterpolator
        fi = nif.add_block("NiFloatInterpolator")
        if fc.keyframes:
            fi.set_field("Value", fc.keyframes[0].value[0])
        else:
            fi.set_field("Value", 0.0)
        fi.set_field("Data", data_ref)

        if has_data:
            # NiFloatData
            fd = nif.add_block("NiFloatData")
            _populate_float_data(fd, fc)

    # Footer: block 0 is root
    nif._footer_roots = [0]

    # Write to disk
    nif.save(str(path))


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------


def _make_identity_transform(ch: BoneChannel) -> dict:
    """Build a NiQuatTransform dict from the first keyframe (or identity)."""
    tx, ty, tz = 0.0, 0.0, 0.0
    qw, qx, qy, qz = 1.0, 0.0, 0.0, 0.0
    scale = 1.0

    if ch.translations:
        v = ch.translations[0].value
        tx, ty, tz = v[0], v[1], v[2]
    if ch.rotations:
        v = ch.rotations[0].value
        # Our format: (x, y, z, w) → NIF (w, x, y, z)
        qx, qy, qz, qw = v[0], v[1], v[2], v[3]
    if ch.scales:
        scale = ch.scales[0].value[0]

    return {
        "Translation": {"x": tx, "y": ty, "z": tz},
        "Rotation": {"w": qw, "x": qx, "y": qy, "z": qz},
        "Scale": scale,
    }


def _populate_transform_data(td: NifBlock, ch: BoneChannel) -> None:
    """Fill NiTransformData fields from a BoneChannel."""
    # --- Rotations ---
    if ch.rotations:
        rot_type = _INTERP_ENUM.get(ch.rotations[0].interpolation, 1)
        td.set_field("Num Rotation Keys", len(ch.rotations))
        td.set_field("Rotation Type", rot_type)

        quat_keys = []
        for kf in ch.rotations:
            x, y, z, w = kf.value
            entry: dict = {
                "Time": kf.time,
                "Value": {"w": w, "x": x, "y": y, "z": z},
            }
            if kf.tbc is not None:
                entry["TBC"] = {"t": kf.tbc[0], "b": kf.tbc[1], "c": kf.tbc[2]}
            quat_keys.append(entry)
        td.set_field("Quaternion Keys", quat_keys)
    else:
        td.set_field("Num Rotation Keys", 0)

    # --- Translations ---
    if ch.translations:
        interp_val = _INTERP_ENUM.get(ch.translations[0].interpolation, 1)
        keys = []
        for kf in ch.translations:
            entry: dict = {
                "Time": kf.time,
                "Value": {"x": kf.value[0], "y": kf.value[1], "z": kf.value[2]},
            }
            if kf.forward is not None:
                entry["Forward"] = {"x": kf.forward[0], "y": kf.forward[1], "z": kf.forward[2]}
            if kf.backward is not None:
                entry["Backward"] = {"x": kf.backward[0], "y": kf.backward[1], "z": kf.backward[2]}
            if kf.tbc is not None:
                entry["TBC"] = {"t": kf.tbc[0], "b": kf.tbc[1], "c": kf.tbc[2]}
            keys.append(entry)
        td.set_field("Translations", {
            "Num Keys": len(keys),
            "Interpolation": interp_val,
            "Keys": keys,
        })
    else:
        td.set_field("Translations", {"Num Keys": 0})

    # --- Scales ---
    if ch.scales:
        interp_val = _INTERP_ENUM.get(ch.scales[0].interpolation, 1)
        keys = []
        for kf in ch.scales:
            entry: dict = {
                "Time": kf.time,
                "Value": kf.value[0],
            }
            if kf.forward is not None:
                entry["Forward"] = kf.forward[0]
            if kf.backward is not None:
                entry["Backward"] = kf.backward[0]
            if kf.tbc is not None:
                entry["TBC"] = {"t": kf.tbc[0], "b": kf.tbc[1], "c": kf.tbc[2]}
            keys.append(entry)
        td.set_field("Scales", {
            "Num Keys": len(keys),
            "Interpolation": interp_val,
            "Keys": keys,
        })
    else:
        td.set_field("Scales", {"Num Keys": 0})


def _populate_float_data(fd: NifBlock, fc: FloatChannel) -> None:
    """Fill NiFloatData fields from a FloatChannel."""
    if not fc.keyframes:
        fd.set_field("Data", {"Num Keys": 0})
        return

    interp_val = _INTERP_ENUM.get(fc.keyframes[0].interpolation, 1)
    keys = []
    for kf in fc.keyframes:
        entry: dict = {
            "Time": kf.time,
            "Value": kf.value[0],
        }
        if kf.forward is not None:
            entry["Forward"] = kf.forward[0]
        if kf.backward is not None:
            entry["Backward"] = kf.backward[0]
        if kf.tbc is not None:
            entry["TBC"] = {"t": kf.tbc[0], "b": kf.tbc[1], "c": kf.tbc[2]}
        keys.append(entry)

    fd.set_field("Data", {
        "Num Keys": len(keys),
        "Interpolation": interp_val,
        "Keys": keys,
    })
