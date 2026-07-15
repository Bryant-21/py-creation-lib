"""Apply a PoseDelta to one HKX animation, operating on an in-memory HKXFile.

Three format-specific writers (lossless, interleaved, spline). The
top-level dispatcher in apply_pose_to_animation handles format detection
and missing-track creation.

Deltas are already in parent-local space (PoseDelta invariant), so the
core compose operation per bone, per frame, is:

    new_local_rot = pose.rotations[bone] * existing_local_rot
    new_local_pos = existing_local_pos + pose.translations[bone]

No FK chain walk. No world->local conversion. No per-frame parent computation.
No XML round-trip: all mutation happens directly on HKXObject members.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass, field
from typing import Optional

import numpy as np

from creation_lib._native.havok_native import (
    HKXArrayMember,
    HKXDirectMember,
    HKXFile,
    HKXObject,
)

from .pose import PoseDelta
from .quat_util import quat_multiply, quat_normalize
from .skeleton import SkeletonManager

_log = logging.getLogger("bone_edit.pose_writer")

_COMPRESSION_MAP = {
    "hkaLosslessCompressedAnimation": "lossless",
    "hkaSplineCompressedAnimation": "spline",
    "hkaInterleavedUncompressedAnimation": "interleaved",
}


@dataclass
class WriteResult:
    success: bool = True
    message: str = ""
    compression_type: str = "unknown"
    bones_modified: list[str] = field(default_factory=list)


# --------------------------------------------------------------
# HKXFile navigation helpers
# --------------------------------------------------------------

def _find_object(hkx_file: HKXFile, class_name: str) -> Optional[HKXObject]:
    for obj in hkx_file.objects:
        if obj.class_name == class_name:
            return obj
    return None


def _get_member(obj: HKXObject, name: str):
    for m in obj.members:
        if m.name == name:
            return m
    return None


def _get_direct_int(obj: HKXObject, name: str, default: int = 0) -> int:
    m = _get_member(obj, name)
    if isinstance(m, HKXDirectMember):
        return int(m.value)
    return default


def _set_direct_int(obj: HKXObject, name: str, value: int) -> None:
    m = _get_member(obj, name)
    if isinstance(m, HKXDirectMember):
        m.value = int(value)


def _get_array(obj: HKXObject, name: str) -> Optional[HKXArrayMember]:
    m = _get_member(obj, name)
    if isinstance(m, HKXArrayMember):
        return m
    return None


# --------------------------------------------------------------
# Top-level dispatch
# --------------------------------------------------------------

def detect_compression_type(hkx_file: HKXFile) -> str:
    """Return 'lossless' / 'spline' / 'interleaved' / 'unknown' for the animation
    in the given HKXFile."""
    for obj in hkx_file.objects:
        if obj.class_name in _COMPRESSION_MAP:
            return _COMPRESSION_MAP[obj.class_name]
    return "unknown"


def build_bone_to_track_map(hkx_file: HKXFile) -> dict[int, int]:
    """Read hkaAnimationBinding.transformTrackToBoneIndices and build bone->track.

    Returned keys are the bone indices *as recorded in the animation's binding*,
    which correspond to positions in the authoring skeleton — not the editor's
    current skeleton. Prefer build_track_name_map for name-keyed lookup.
    """
    binding = _find_object(hkx_file, "hkaAnimationBinding")
    if binding is None:
        return {}
    tti = _get_array(binding, "transformTrackToBoneIndices")
    if tti is None:
        return {}
    return {int(bone_idx): track_idx for track_idx, bone_idx in enumerate(tti.contents)}


def _get_animation_num_tracks(hkx_file: HKXFile) -> int:
    """Return numberOfTransformTracks from whichever animation class is present."""
    for oc in (
        "hkaSplineCompressedAnimation",
        "hkaLosslessCompressedAnimation",
        "hkaInterleavedUncompressedAnimation",
    ):
        anim = _find_object(hkx_file, oc)
        if anim is not None:
            return _get_direct_int(anim, "numberOfTransformTracks", 0)
    return 0


def build_track_name_map(
    hkx_file: HKXFile,
    skeleton: SkeletonManager,
    source_name: str = "<unknown>",
) -> dict[str, int]:
    """Return {bone_name: track_index} for the animation in `hkx_file`.

    Resolves each track's recorded bone index against the editor's skeleton
    bone name list. This assumes the editor skeleton's leading bones match the
    animation's authoring skeleton (true for FO4 1st/3rd person vanilla, plus
    any editor skeleton that was NIF-augmented only by *appending* extra bones).

    Two Havok conventions are supported:
      1. Explicit binding — `transformTrackToBoneIndices` populated: track_idx
         maps to the recorded bone index; bone name comes from skeleton.bone_names
         at that index.
      2. Implicit identity binding — `transformTrackToBoneIndices` EMPTY and
         `numberOfTransformTracks > 0`: FO4 runtime treats this as the identity
         mapping, i.e. track `i` drives bone index `i` directly. Common for
         animations exported by 3DS Max / Maya Havok plugins that assume the
         animation was authored against the exact target skeleton.

    If an index from the binding falls outside the skeleton, that track is
    dropped from the map — name-based lookup simply won't find a match for it,
    which is the correct behavior.
    """
    binding = _find_object(hkx_file, "hkaAnimationBinding")
    if binding is None:
        classes = [o.class_name for o in hkx_file.objects]
        _log.warning(
            "[%s] no hkaAnimationBinding object found; file contains %d object(s): %s",
            source_name, len(classes), ", ".join(classes),
        )
        return {}

    tti = _get_array(binding, "transformTrackToBoneIndices")

    # Implicit identity mapping: empty/missing binding + nonzero track count
    if tti is None or not tti.contents:
        num_tracks = _get_animation_num_tracks(hkx_file)
        if num_tracks <= 0:
            member_names = [getattr(m, "name", None) or "?" for m in binding.members]
            _log.warning(
                "[%s] binding has empty transformTrackToBoneIndices AND the "
                "animation reports 0 tracks; binding members: %s",
                source_name, ", ".join(member_names),
            )
            return {}

        names = skeleton.bone_names
        n = len(names)
        name_map: dict[str, int] = {}
        for track_idx in range(num_tracks):
            if track_idx < n:
                name_map[names[track_idx]] = track_idx

        _log.info(
            "[%s] track_name_map (implicit identity): %d/%d track(s) resolved "
            "(skeleton=%d bones) — binding.transformTrackToBoneIndices was empty, "
            "using Havok identity convention (track i -> bone i)",
            source_name, len(name_map), num_tracks, n,
        )
        if num_tracks > n:
            _log.warning(
                "[%s] animation has %d tracks but skeleton only has %d bones; "
                "%d trailing track(s) unreachable by name",
                source_name, num_tracks, n, num_tracks - n,
            )
        return name_map

    # Explicit binding path
    raw_indices = list(tti.contents)
    names = skeleton.bone_names
    n = len(names)
    name_map = {}
    out_of_range = 0
    for track_idx, bone_idx in enumerate(raw_indices):
        bi = int(bone_idx)
        if 0 <= bi < n:
            name_map[names[bi]] = track_idx
        else:
            out_of_range += 1

    _log.info(
        "[%s] track_name_map: %d track(s) resolved, %d out-of-range "
        "(binding=%d entries, skeleton=%d bones, binding range=[%d..%d])",
        source_name, len(name_map), out_of_range,
        len(raw_indices), n,
        min(int(b) for b in raw_indices), max(int(b) for b in raw_indices),
    )
    if out_of_range:
        _log.warning(
            "[%s] %d binding index(es) fall outside the loaded skeleton — "
            "those tracks are unreachable by name. First 5 out-of-range: %s",
            source_name, out_of_range,
            [int(b) for b in raw_indices if not (0 <= int(b) < n)][:5],
        )
    return name_map


def apply_pose_to_animation(
    hkx_file: HKXFile,
    pose: PoseDelta,
    skeleton: SkeletonManager,
    source_name: str = "<unknown>",
) -> WriteResult:
    """Apply pose deltas to a parsed animation HKXFile, in place.

    `source_name` is used for log messages only (typically the animation
    filename) — does not affect behavior.
    """
    result = WriteResult()
    edited_bones = list(pose.edited_bones())
    if pose.is_empty():
        _log.info("[%s] pose is empty, nothing to apply", source_name)
        result.message = "Pose is empty; nothing to write"
        return result

    fmt = detect_compression_type(hkx_file)
    result.compression_type = fmt
    _log.info(
        "[%s] applying pose: format=%s, skeleton=%d bones, pose edits=%d bone(s) %s",
        source_name, fmt, skeleton.bone_count, len(edited_bones),
        edited_bones[:6] + (["..."] if len(edited_bones) > 6 else []),
    )
    if fmt == "unknown":
        _log.warning(
            "[%s] unknown compression type — file objects: %s",
            source_name, [o.class_name for o in hkx_file.objects],
        )
        result.success = False
        result.message = "Unknown animation compression type"
        return result

    track_name_map = build_track_name_map(hkx_file, skeleton, source_name=source_name)

    missing_names: list[str] = []
    missing_bone_indices: list[int] = []
    unknown_names: list[str] = []
    resolved_pairs: list[tuple[str, int]] = []
    for bone_name in edited_bones:
        track_idx = track_name_map.get(bone_name)
        if track_idx is not None:
            resolved_pairs.append((bone_name, track_idx))
            continue
        if skeleton.get_bone_index(bone_name) is None:
            unknown_names.append(bone_name)
            continue
        missing_names.append(bone_name)
        bi = skeleton.get_bone_index(bone_name)
        if bi is not None:
            missing_bone_indices.append(bi)

    _log.info(
        "[%s] lookup: %d resolved %s, %d unknown-in-skeleton %s, %d missing-from-anim %s",
        source_name,
        len(resolved_pairs),
        [f"{n}=>t{t}" for n, t in resolved_pairs[:6]] + (["..."] if len(resolved_pairs) > 6 else []),
        len(unknown_names), unknown_names[:6] + (["..."] if len(unknown_names) > 6 else []),
        len(missing_names), missing_names[:6] + (["..."] if len(missing_names) > 6 else []),
    )

    if unknown_names:
        _log.warning(
            "[%s] %d pose bone(s) do not exist in the loaded skeleton at all "
            "(check you loaded the right skeleton): %s",
            source_name, len(unknown_names), unknown_names[:8],
        )

    skipped_msg = ""
    if missing_names:
        if fmt == "spline":
            _log.info(
                "[%s] spline: skipping %d bone(s) with no track: %s "
                "(skeleton=%d bones, tracks=%d)",
                source_name, len(missing_names), ", ".join(missing_names[:8]),
                skeleton.bone_count, len(track_name_map),
            )
            skipped_msg = (
                f" (skipped {len(missing_names)} bone(s) with no track: "
                f"{', '.join(missing_names[:4])}"
                f"{'...' if len(missing_names) > 4 else ''})"
            )
        else:
            _log.info(
                "[%s] %s: adding %d identity track(s) for missing bones: %s",
                source_name, fmt, len(missing_names), missing_names[:8],
            )
            try:
                _add_identity_tracks(hkx_file, missing_bone_indices, missing_names, fmt, track_name_map)
            except Exception as e:
                _log.exception("[%s] _add_identity_tracks failed", source_name)
                result.success = False
                result.message = f"Failed to add tracks: {e}"
                return result

    try:
        if fmt == "lossless":
            modified = _write_lossless(hkx_file, pose, track_name_map)
        elif fmt == "interleaved":
            modified = _write_interleaved(hkx_file, pose, track_name_map)
        elif fmt == "spline":
            modified = _write_spline(hkx_file, pose, track_name_map)
        else:
            result.success = False
            result.message = f"Unhandled format: {fmt}"
            return result
    except Exception as e:
        _log.exception("[%s] write failed", source_name)
        result.success = False
        result.message = f"Write failed: {e}"
        return result

    result.bones_modified = sorted(modified)
    result.message = f"Modified {len(modified)} bone(s)" + skipped_msg
    _log.info(
        "[%s] DONE: modified %d/%d bone(s) %s%s",
        source_name, len(modified), len(edited_bones),
        sorted(modified)[:6] + (["..."] if len(modified) > 6 else []),
        f" — skipped {len(missing_names)}" if missing_names else "",
    )
    return result


# --------------------------------------------------------------
# Lossless format
# --------------------------------------------------------------

def _write_lossless(
    hkx_file: HKXFile,
    pose: PoseDelta,
    track_name_map: dict[str, int],
) -> set[str]:
    anim = _find_object(hkx_file, "hkaLosslessCompressedAnimation")
    if anim is None:
        raise ValueError("No hkaLosslessCompressedAnimation found")

    num_frames = _get_direct_int(anim, "numFrames", 1)

    trans_tao = _get_array(anim, "translationTypeAndOffsets")
    rot_tao = _get_array(anim, "rotationTypeAndOffsets")
    static_t = _get_array(anim, "staticTranslations")
    dyn_t = _get_array(anim, "dynamicTranslations")
    static_r = _get_array(anim, "staticRotations")
    dyn_r = _get_array(anim, "dynamicRotations")

    if None in (trans_tao, rot_tao, static_t, dyn_t, static_r, dyn_r):
        raise ValueError("Lossless animation missing required arrays")

    modified: set[str] = set()

    for bone_name, delta_vec in pose.translations.items():
        track_idx = track_name_map.get(bone_name)
        if track_idx is None or track_idx >= len(trans_tao.contents):
            continue
        _compose_translation_lossless(
            track_idx, delta_vec,
            trans_tao.contents, static_t.contents, dyn_t.contents, num_frames,
        )
        modified.add(bone_name)

    for bone_name, delta_quat in pose.rotations.items():
        track_idx = track_name_map.get(bone_name)
        if track_idx is None or track_idx >= len(rot_tao.contents):
            continue
        _compose_rotation_lossless(
            track_idx, delta_quat,
            rot_tao.contents, static_r.contents, dyn_r.contents, num_frames,
        )
        modified.add(bone_name)

    return modified


def _compose_translation_lossless(
    track_idx: int, delta: np.ndarray,
    trans_tao: list[int], static_t: list[float], dyn_t: list[float],
    num_frames: int,
) -> None:
    """Add `delta` to track `track_idx`'s translation values."""
    tao = trans_tao[track_idx]
    ttype = tao & 3
    toffset = tao >> 2

    dx, dy, dz = float(delta[0]), float(delta[1]), float(delta[2])

    if ttype == 0:
        new_idx = len(static_t) // 3
        static_t.extend([dx, dy, dz])
        trans_tao[track_idx] = (new_idx << 2) | 1
    elif ttype == 1:
        base = toffset * 3
        if base + 2 < len(static_t):
            static_t[base] += dx
            static_t[base + 1] += dy
            static_t[base + 2] += dz
    elif ttype == 2:
        for f in range(num_frames):
            base = (toffset + f) * 3
            if base + 2 < len(dyn_t):
                dyn_t[base] += dx
                dyn_t[base + 1] += dy
                dyn_t[base + 2] += dz


def _compose_rotation_lossless(
    track_idx: int, delta_q: np.ndarray,
    rot_tao: list[int], static_r: list[list[float]], dyn_r: list[list[float]],
    num_frames: int,
) -> None:
    """Pre-multiply track's rotation values by delta_q (parent-local)."""
    rao = rot_tao[track_idx]
    rtype = rao & 3
    roffset = rao >> 2

    if rtype == 0:
        new_idx = len(static_r)
        static_r.append([float(delta_q[0]), float(delta_q[1]),
                         float(delta_q[2]), float(delta_q[3])])
        rot_tao[track_idx] = (new_idx << 2) | 1
    elif rtype == 1:
        if roffset < len(static_r):
            old_q = np.array(static_r[roffset][:4])
            new_q = quat_normalize(quat_multiply(delta_q, old_q))
            static_r[roffset] = [float(new_q[0]), float(new_q[1]),
                                 float(new_q[2]), float(new_q[3])]
    elif rtype == 2:
        for f in range(num_frames):
            idx = roffset + f
            if idx < len(dyn_r):
                old_q = np.array(dyn_r[idx][:4])
                new_q = quat_normalize(quat_multiply(delta_q, old_q))
                dyn_r[idx] = [float(new_q[0]), float(new_q[1]),
                              float(new_q[2]), float(new_q[3])]


# --------------------------------------------------------------
# Interleaved format
# --------------------------------------------------------------

def _write_interleaved(
    hkx_file: HKXFile,
    pose: PoseDelta,
    track_name_map: dict[str, int],
) -> set[str]:
    anim = _find_object(hkx_file, "hkaInterleavedUncompressedAnimation")
    if anim is None:
        raise ValueError("No hkaInterleavedUncompressedAnimation found")

    num_tracks = _get_direct_int(anim, "numberOfTransformTracks", 0)
    if num_tracks == 0:
        return set()

    transforms = _get_array(anim, "transforms")
    if transforms is None:
        raise ValueError("Interleaved animation has no transforms array")

    # Each entry is a flat 12-float list: [px py pz pw  rx ry rz rw  sx sy sz sw]
    # Frame-major layout: transforms[frame * num_tracks + track]
    flat = transforms.contents
    total = len(flat)
    if total == 0 or total % num_tracks != 0:
        return set()
    num_frames = total // num_tracks

    modified: set[str] = set()

    for bone_name, delta_vec in pose.translations.items():
        track_idx = track_name_map.get(bone_name)
        if track_idx is None or track_idx >= num_tracks:
            continue
        dx, dy, dz = float(delta_vec[0]), float(delta_vec[1]), float(delta_vec[2])
        for f in range(num_frames):
            row = flat[f * num_tracks + track_idx]
            row[0] += dx
            row[1] += dy
            row[2] += dz
        modified.add(bone_name)

    for bone_name, delta_quat in pose.rotations.items():
        track_idx = track_name_map.get(bone_name)
        if track_idx is None or track_idx >= num_tracks:
            continue
        for f in range(num_frames):
            row = flat[f * num_tracks + track_idx]
            old_q = np.array(row[4:8])
            new_q = quat_normalize(quat_multiply(delta_quat, old_q))
            row[4] = float(new_q[0])
            row[5] = float(new_q[1])
            row[6] = float(new_q[2])
            row[7] = float(new_q[3])
        modified.add(bone_name)

    return modified


# --------------------------------------------------------------
# Spline format
# --------------------------------------------------------------

def _write_spline(
    hkx_file: HKXFile,
    pose: PoseDelta,
    track_name_map: dict[str, int],
) -> set[str]:
    """Apply pose deltas to a spline-compressed animation.

    Delegates to SplinePatcher, which decodes/patches/re-encodes individual
    track spline blobs. Works entirely on the HKXFile's 'data' array member.
    """
    from .spline_patcher import SplinePatcher

    patcher = SplinePatcher.from_hkx_file(hkx_file)
    modified: set[str] = set()

    for bone_name, delta_vec in pose.translations.items():
        track_idx = track_name_map.get(bone_name)
        if track_idx is None:
            continue
        try:
            patcher.offset_translation(track_idx, delta_vec)
            modified.add(bone_name)
        except Exception as e:
            _log.warning("Spline offset_translation failed for %s: %s", bone_name, e)

    for bone_name, delta_quat in pose.rotations.items():
        track_idx = track_name_map.get(bone_name)
        if track_idx is None:
            continue
        try:
            patcher.offset_rotation(track_idx, delta_quat)
            modified.add(bone_name)
        except Exception as e:
            _log.warning("Spline offset_rotation failed for %s: %s", bone_name, e)

    patcher.write_to_hkx_file(hkx_file)
    return modified


# --------------------------------------------------------------
# Track addition for missing bones (lossless / interleaved only)
# --------------------------------------------------------------

def _add_identity_tracks(
    hkx_file: HKXFile,
    missing_bone_indices: list[int],
    missing_bone_names: list[str],
    fmt: str,
    track_name_map: dict[str, int],
) -> None:
    """Append identity tracks for the given bones, updating the name->track map."""
    binding = _find_object(hkx_file, "hkaAnimationBinding")
    if binding is None:
        return
    tti = _get_array(binding, "transformTrackToBoneIndices")
    if tti is None:
        return

    next_track = len(tti.contents)
    for bone_idx, bone_name in zip(missing_bone_indices, missing_bone_names):
        track_name_map[bone_name] = next_track
        tti.contents.append(int(bone_idx))
        next_track += 1
    num_new = len(missing_bone_indices)

    if fmt == "lossless":
        anim = _find_object(hkx_file, "hkaLosslessCompressedAnimation")
        if anim is None:
            return
        for name in ("translationTypeAndOffsets", "rotationTypeAndOffsets",
                     "scaleTypeAndOffsets"):
            arr = _get_array(anim, name)
            if arr is not None:
                arr.contents.extend([0] * num_new)
        old_n = _get_direct_int(anim, "numberOfTransformTracks", 0)
        _set_direct_int(anim, "numberOfTransformTracks", old_n + num_new)

    elif fmt == "interleaved":
        anim = _find_object(hkx_file, "hkaInterleavedUncompressedAnimation")
        if anim is None:
            return
        transforms = _get_array(anim, "transforms")
        if transforms is None:
            return
        old_n = _get_direct_int(anim, "numberOfTransformTracks", 0)
        if old_n == 0:
            return
        flat = transforms.contents
        total = len(flat)
        if total == 0 or total % old_n != 0:
            return
        num_frames = total // old_n
        # Rebuild frame-major with num_new identity QSTRANSFORMs appended per frame
        identity = [0.0, 0.0, 0.0, 0.0,   # position
                    0.0, 0.0, 0.0, 1.0,   # rotation (identity)
                    1.0, 1.0, 1.0, 1.0]   # scale
        new_flat: list[list[float]] = []
        for f in range(num_frames):
            start = f * old_n
            for t in range(old_n):
                new_flat.append(flat[start + t])
            for _ in range(num_new):
                new_flat.append(list(identity))
        transforms.contents = new_flat
        _set_direct_int(anim, "numberOfTransformTracks", old_n + num_new)
