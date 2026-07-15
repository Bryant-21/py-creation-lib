"""Decode, modify, and re-encode spline-compressed animation blobs.

Adapted from ui/aligner/spline_decoder.py (decode-only) and
refs/pynifly/io_scene_nifly/hkx/anim_fo4.py (full encode/decode).

Handles per-track patching: unmodified tracks are copied byte-verbatim,
modified tracks are decoded, offset-applied, and re-encoded.
"""

from __future__ import annotations

import logging
import math
import struct
from dataclasses import dataclass, field
from typing import List, Optional

import numpy as np

from creation_lib._native.havok_native import (
    HKXArrayMember,
    HKXDirectMember,
    HKXFile,
    HKXObject,
)

_log = logging.getLogger("bone_edit.spline")

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

# Position/Scale quantization types
QT_8BIT = 0
QT_16BIT = 1

# Rotation quantization types (stored in mask as value-2)
ROTQT_32BIT = 2       # 4 bytes — polar
ROTQT_40BIT = 3       # 5 bytes — three-comp 12-bit
ROTQT_48BIT = 4       # 6 bytes — three-comp 15-bit
ROTQT_THREECOMP16 = 5 # 8 bytes — four uint16
ROTQT_UNCOMPRESSED = 6 # 16 bytes — four float32

ROT_SIZES = {
    ROTQT_32BIT: 4,
    ROTQT_40BIT: 5,
    ROTQT_48BIT: 6,
    ROTQT_THREECOMP16: 8,
    ROTQT_UNCOMPRESSED: 16,
}

# Sub-track flag bits
_STATIC_X = 1
_STATIC_Y = 2
_STATIC_Z = 4
_STATIC_W = 8
_SPLINE_X = 16
_SPLINE_Y = 32
_SPLINE_Z = 64
_SPLINE_W = 128


# ---------------------------------------------------------------------------
# Low-level helpers
# ---------------------------------------------------------------------------

def _align(pos: int, alignment: int) -> int:
    return (pos + alignment - 1) & ~(alignment - 1)


def _count_set_bits(flags: int, mask: int) -> int:
    return bin(flags & mask).count("1")


def _rot_alignment(rot_quant: int) -> int:
    """Return byte alignment required before rotation data.
      32-bit polar    -> 4
      40-bit 12-bit   -> 1  (no alignment — the 5-byte format is byte-packed)
      48-bit 15-bit   -> 2
      ThreeComp16     -> 2
      Uncompressed f4 -> 4
    """
    if rot_quant == ROTQT_40BIT:
        return 1
    if rot_quant in (ROTQT_48BIT, ROTQT_THREECOMP16):
        return 2
    return 4


# ---------------------------------------------------------------------------
# TrackMask
# ---------------------------------------------------------------------------

@dataclass
class TrackMask:
    """Decoded per-track 4-byte mask from spline animation."""
    pos_quant: int     # 0=8-bit, 1=16-bit
    rot_quant: int     # 2=32bit, 3=40bit, 4=48bit, 5=threecomp16, 6=uncompressed
    scale_quant: int   # 0=8-bit, 1=16-bit
    pos_type: str      # "identity", "static", "spline"
    rot_type: str      # "identity", "static", "spline"
    scale_type: str    # "identity", "static", "spline"
    pos_flags: int     # raw byte 1
    rot_flags: int     # raw byte 2
    scale_flags: int   # raw byte 3


def parse_track_mask(mask_bytes: bytes) -> TrackMask:
    """Parse a 4-byte per-track mask."""
    b0, b1, b2, b3 = mask_bytes[0], mask_bytes[1], mask_bytes[2], mask_bytes[3]

    pos_quant = b0 & 0x3
    rot_quant = ((b0 >> 2) & 0xF) + 2
    scale_quant = (b0 >> 6) & 0x3

    def _classify(flags: int) -> str:
        if flags & 0xF0:
            return "spline"
        elif flags & 0x0F:
            return "static"
        return "identity"

    return TrackMask(
        pos_quant=pos_quant,
        rot_quant=rot_quant,
        scale_quant=scale_quant,
        pos_type=_classify(b1),
        rot_type=_classify(b2),
        scale_type=_classify(b3),
        pos_flags=b1,
        rot_flags=b2,
        scale_flags=b3,
    )


# ---------------------------------------------------------------------------
# Scalar quantization codec
# ---------------------------------------------------------------------------

def decode_scalar(data: bytes, pos: int, mn: float, mx: float, quant: int) -> float:
    """Decode an 8-bit or 16-bit quantized scalar."""
    if quant == QT_8BIT:
        raw = data[pos]
        return mn + (mx - mn) * (raw / 255.0)
    else:
        raw = struct.unpack_from("<H", data, pos)[0]
        return mn + (mx - mn) * (raw / 65535.0)


def encode_scalar(val: float, mn: float, mx: float, quant: int) -> bytes:
    """Encode a scalar value as 8-bit or 16-bit quantized bytes."""
    if abs(mx - mn) < 1e-30:
        if quant == QT_8BIT:
            return struct.pack("B", 0)
        return struct.pack("<H", 0)

    t = (val - mn) / (mx - mn)
    if quant == QT_8BIT:
        raw = min(255, max(0, int(round(t * 255))))
        return struct.pack("B", raw)
    else:
        raw = min(65535, max(0, int(round(t * 65535))))
        return struct.pack("<H", raw)


def _scalar_size(quant: int) -> int:
    return 1 if quant == QT_8BIT else 2


# ---------------------------------------------------------------------------
# Quaternion codec
# ---------------------------------------------------------------------------

def _smallest_three_result(a: float, b: float, c: float, d: float, which: int) -> list:
    """Reconstruct XYZW quaternion from smallest-three encoding."""
    if which == 0:
        return [d, a, b, c]
    elif which == 1:
        return [a, d, b, c]
    elif which == 2:
        return [a, b, d, c]
    else:
        return [a, b, c, d]


def _decode_quat_48bit(data: bytes, pos: int) -> np.ndarray:
    """Decode 48-bit (6 byte) quaternion — three int16 values."""
    FRACTAL = 0.000043161
    MASK = (1 << 15) - 1
    HALF = MASK >> 1  # 16383

    x_raw, y_raw, z_raw = struct.unpack_from("<3H", data, pos)
    shift = ((y_raw >> 14) & 2) | ((x_raw >> 15) & 1)
    r_sign = (z_raw >> 15) != 0

    vals = [((v & MASK) - HALF) * FRACTAL for v in (x_raw, y_raw, z_raw)]
    sum_sq = sum(v * v for v in vals)
    w = math.sqrt(max(0.0, 1.0 - sum_sq))
    if r_sign:
        w = -w

    result = _smallest_three_result(vals[0], vals[1], vals[2], w, shift)
    n = math.sqrt(sum(c * c for c in result))
    if n > 1e-10:
        result = [c / n for c in result]
    return np.array(result)


def _encode_quat_48bit(q: np.ndarray) -> bytes:
    """Encode a unit quaternion [x,y,z,w] as 6 bytes (48-bit)."""
    FRACTAL = 0.000043161
    MASK = (1 << 15) - 1
    HALF = MASK >> 1  # 16383

    abs_q = [abs(float(c)) for c in q]
    shift = abs_q.index(max(abs_q))
    r_sign = float(q[shift]) < 0

    vals = [float(q[j]) for j in range(4) if j != shift]

    raw = [min(MASK, max(0, int(round(v / FRACTAL + HALF)))) for v in vals]

    x_raw = raw[0] & MASK
    y_raw = raw[1] & MASK
    z_raw = raw[2] & MASK

    x_raw |= (shift & 1) << 15
    y_raw |= ((shift >> 1) & 1) << 15
    if r_sign:
        z_raw |= 1 << 15

    return struct.pack("<HHH", x_raw, y_raw, z_raw)


def _decode_quat_40bit(data: bytes, pos: int) -> np.ndarray:
    """Decode 40-bit (5 byte) quaternion — 12-bit components."""
    FRACTAL = 0.000345436
    raw = int.from_bytes(data[pos:pos + 5], "little")

    a = (raw >> 0) & 0xFFF
    b = (raw >> 12) & 0xFFF
    c = (raw >> 24) & 0xFFF
    vals = [(v - 2049) * FRACTAL for v in (a, b, c)]

    sum_sq = sum(v * v for v in vals)
    w = math.sqrt(max(0.0, 1.0 - sum_sq))
    if (raw >> 38) & 1:
        w = -w

    shift = (raw >> 36) & 3
    result = _smallest_three_result(vals[0], vals[1], vals[2], w, shift)
    n = math.sqrt(sum(c * c for c in result))
    if n > 1e-10:
        result = [c / n for c in result]
    return np.array(result)


def _encode_quat_40bit(q: np.ndarray) -> bytes:
    """Encode a unit quaternion [x,y,z,w] as 5 bytes (40-bit)."""
    FRACTAL = 0.000345436
    HALF = 2049

    abs_q = [abs(float(c)) for c in q]
    shift = abs_q.index(max(abs_q))
    r_sign = float(q[shift]) < 0

    vals = [float(q[j]) for j in range(4) if j != shift]
    raw_vals = [min(0xFFF, max(0, int(round(v / FRACTAL + HALF)))) for v in vals]

    packed = raw_vals[0] | (raw_vals[1] << 12) | (raw_vals[2] << 24)
    packed |= (shift & 3) << 36
    if r_sign:
        packed |= 1 << 38

    return packed.to_bytes(5, "little")


def _decode_quat_32bit(data: bytes, pos: int) -> np.ndarray:
    """Decode 32-bit quaternion (polar encoding)."""
    cval = struct.unpack_from("<I", data, pos)[0]
    r_mask = (1 << 10) - 1
    r_frac = 1.0 / r_mask
    R = ((cval >> 18) & r_mask) * r_frac
    R = 1.0 - R * R

    phi_theta = float(cval & 0x3FFFF)
    phi = math.floor(math.sqrt(phi_theta))
    theta = 0.0
    if phi > 0.0:
        theta = (math.pi / 4.0) * (phi_theta - phi * phi) / phi
        phi = (math.pi / 2.0 / 511.0) * phi

    magnitude = math.sqrt(max(0, 1.0 - R * R))
    sp, cp = math.sin(phi), math.cos(phi)
    st, ct = math.sin(theta), math.cos(theta)
    result = [sp * ct * magnitude, sp * st * magnitude, cp * magnitude, R]

    sign_masks = [0x10000000, 0x20000000, 0x40000000, 0x80000000]
    for i in range(4):
        if cval & sign_masks[i]:
            result[i] = -result[i]

    n = math.sqrt(sum(c * c for c in result))
    if n > 1e-10:
        result = [c / n for c in result]
    return np.array(result)


def _encode_quat_32bit(q: np.ndarray) -> bytes:
    """Encode a unit quaternion as 32-bit polar.

    This is lossy — round-trip will have quantization error.
    Uses the same polar encoding as the HavokLib reader.
    """
    x, y, z, w = [float(c) for c in q]

    # Sign bits
    signs = 0
    vals = [x, y, z, w]
    abs_vals = [abs(v) for v in vals]
    for i in range(4):
        if vals[i] < 0:
            signs |= 1 << (28 + i)
            abs_vals[i] = -vals[i]

    x_a, y_a, z_a, w_a = abs_vals

    # R = w component (clamped)
    R = min(1.0, max(0.0, w_a))
    r_mask = (1 << 10) - 1
    R_quant = min(r_mask, int(round(math.sqrt(max(0, 1.0 - R)) * r_mask)))

    # Compute spherical angles from the xyz components
    magnitude = math.sqrt(x_a * x_a + y_a * y_a + z_a * z_a)
    if magnitude < 1e-10:
        phi = 0.0
        theta = 0.0
    else:
        cos_phi = min(1.0, z_a / magnitude)
        phi = math.acos(cos_phi)
        sin_phi = math.sin(phi)
        if sin_phi < 1e-10:
            theta = 0.0
        else:
            theta = math.atan2(y_a / magnitude / sin_phi, x_a / magnitude / sin_phi)

    phi_quant = int(round(phi * 511.0 / (math.pi / 2.0)))
    phi_quant = min(511, max(0, phi_quant))

    if phi_quant > 0:
        theta_quant = int(round(theta * phi_quant / (math.pi / 4.0)))
    else:
        theta_quant = 0

    phi_theta = phi_quant * phi_quant + theta_quant
    phi_theta = min(0x3FFFF, max(0, phi_theta))

    cval = phi_theta | (R_quant << 18) | signs
    return struct.pack("<I", cval)


def _decode_quat_threecomp16(data: bytes, pos: int) -> np.ndarray:
    """Decode ThreeComp16 quaternion (4 x uint16, 8 bytes)."""
    a, b, c, d = struct.unpack_from("<4H", data, pos)
    frac = 1.0 / 32767.0
    x = (a - 32767) * frac
    y = (b - 32767) * frac
    z = (c - 32767) * frac
    sum_sq = x * x + y * y + z * z
    w = math.sqrt(max(0.0, 1.0 - sum_sq))
    if d & 0x8000:
        w = -w
    return np.array([x, y, z, w])


def _encode_quat_threecomp16(q: np.ndarray) -> bytes:
    """Encode quaternion as ThreeComp16 (4 x uint16, 8 bytes)."""
    x, y, z, w = [float(c) for c in q]
    a = min(65535, max(0, int(round(x * 32767.0 + 32767))))
    b = min(65535, max(0, int(round(y * 32767.0 + 32767))))
    c = min(65535, max(0, int(round(z * 32767.0 + 32767))))
    d = 0x8000 if w < 0 else 0
    return struct.pack("<4H", a, b, c, d)


def _decode_quat_uncompressed(data: bytes, pos: int) -> np.ndarray:
    """Read uncompressed float4 quaternion (XYZW)."""
    x, y, z, w = struct.unpack_from("<4f", data, pos)
    return np.array([x, y, z, w])


def _encode_quat_uncompressed(q: np.ndarray) -> bytes:
    """Write uncompressed float4 quaternion."""
    return struct.pack("<4f", *[float(c) for c in q])


def decode_quaternion(data: bytes, pos: int, rot_quant: int) -> np.ndarray:
    """Decode a quaternion based on quantization type."""
    decoders = {
        ROTQT_32BIT: _decode_quat_32bit,
        ROTQT_40BIT: _decode_quat_40bit,
        ROTQT_48BIT: _decode_quat_48bit,
        ROTQT_THREECOMP16: _decode_quat_threecomp16,
        ROTQT_UNCOMPRESSED: _decode_quat_uncompressed,
    }
    decoder = decoders.get(rot_quant)
    if decoder is None:
        raise ValueError(f"Unknown rotation quantization type: {rot_quant}")
    return decoder(data, pos)


def encode_quaternion(q: np.ndarray, rot_quant: int) -> bytes:
    """Encode a quaternion based on quantization type."""
    encoders = {
        ROTQT_32BIT: _encode_quat_32bit,
        ROTQT_40BIT: _encode_quat_40bit,
        ROTQT_48BIT: _encode_quat_48bit,
        ROTQT_THREECOMP16: _encode_quat_threecomp16,
        ROTQT_UNCOMPRESSED: _encode_quat_uncompressed,
    }
    encoder = encoders.get(rot_quant)
    if encoder is None:
        raise ValueError(f"Unknown rotation quantization type: {rot_quant}")
    return encoder(q)


# ---------------------------------------------------------------------------
# SplinePatcher — blob-level parsing, patching, and re-encoding
# ---------------------------------------------------------------------------

@dataclass
class _TrackLayout:
    """Byte ranges for a single track within a block."""
    pos_start: int = 0
    pos_end: int = 0
    rot_start: int = 0
    rot_end: int = 0
    scale_start: int = 0
    scale_end: int = 0


@dataclass
class _PosSplineInfo:
    """Parsed position spline metadata for a track."""
    num_items: int = 0
    degree: int = 0
    knots_start: int = 0
    knots_end: int = 0
    # Per dynamic axis: (min, max, axis_index)
    dynamic_axes: list = field(default_factory=list)
    static_values: dict = field(default_factory=dict)  # axis -> float
    cps_start: int = 0
    num_cps: int = 0
    num_dynamic: int = 0
    quant: int = 0


def _find_spline_animation(hkx_file: HKXFile) -> Optional[HKXObject]:
    """Return the hkaSplineCompressedAnimation object, if any."""
    for obj in hkx_file.objects:
        if "SplineCompressed" in obj.class_name:
            return obj
    return None


def _get_member(obj: HKXObject, name: str):
    for m in obj.members:
        if m.name == name:
            return m
    return None


class SplinePatcher:
    """Parse, modify, and re-encode a spline-compressed animation blob.

    Usage:
        patcher = SplinePatcher.from_hkx_file(hkx_file)
        patcher.offset_translation(track_idx, np.array([1,0,0]))
        patcher.offset_rotation(track_idx, delta_quat)
        patcher.write_to_hkx_file(hkx_file)
    """

    def __init__(
        self,
        blob: bytearray,
        num_transform_tracks: int,
        num_float_tracks: int,
        num_blocks: int,
        block_offsets: list[int],
        max_frames_per_block: int,
    ):
        self.blob = blob
        self.num_transform_tracks = num_transform_tracks
        self.num_float_tracks = num_float_tracks
        self.num_blocks = num_blocks
        self.block_offsets = block_offsets
        self.max_frames_per_block = max_frames_per_block

        # Parse masks from block 0
        self.track_masks: list[TrackMask] = []
        self._parse_masks()

    @classmethod
    def from_hkx_file(cls, hkx_file: HKXFile) -> "SplinePatcher":
        """Construct a SplinePatcher from an in-memory HKXFile.

        Reads the hkaSplineCompressedAnimation object's fields directly from
        the parsed HKXObject members — no XML, no text parsing.
        """
        anim_obj = _find_spline_animation(hkx_file)
        if anim_obj is None:
            raise ValueError("No hkaSplineCompressedAnimation found in HKXFile")

        def _int(name: str, default: int) -> int:
            m = _get_member(anim_obj, name)
            if isinstance(m, HKXDirectMember):
                return int(m.value)
            return default

        def _int_array(name: str) -> list[int]:
            m = _get_member(anim_obj, name)
            if isinstance(m, HKXArrayMember):
                return [int(x) for x in m.contents]
            return []

        num_transform_tracks = _int("numberOfTransformTracks", 0)
        num_float_tracks = _int("numberOfFloatTracks", 0)
        num_blocks = _int("numBlocks", 1)
        max_frames = _int("maxFramesPerBlock", 255)

        block_offsets = _int_array("blockOffsets") or [0]

        data_member = _get_member(anim_obj, "data")
        if isinstance(data_member, HKXArrayMember):
            blob = bytearray(int(b) & 0xFF for b in data_member.contents)
        else:
            blob = bytearray()

        return cls(
            blob=blob,
            num_transform_tracks=num_transform_tracks,
            num_float_tracks=num_float_tracks,
            num_blocks=num_blocks,
            block_offsets=block_offsets,
            max_frames_per_block=max_frames,
        )

    def _parse_masks(self):
        """Parse track masks from block 0."""
        pos = self.block_offsets[0] if self.block_offsets else 0
        self.track_masks = []
        for i in range(self.num_transform_tracks):
            mask_bytes = bytes(self.blob[pos:pos + 4])
            self.track_masks.append(parse_track_mask(mask_bytes))
            pos += 4

    def _get_data_start(self, block_idx: int = 0) -> int:
        """Get the byte position where per-track data starts (after masks + float tracks + alignment)."""
        start = self.block_offsets[block_idx] if block_idx < len(self.block_offsets) else 0
        pos = start + 4 * self.num_transform_tracks + self.num_float_tracks
        return _align(pos, 4)

    def _walk_to_track_pos(self, target_track: int, block_idx: int = 0) -> tuple[int, _PosSplineInfo | None]:
        """Walk through blob to find position data for target_track.

        Returns (pos_after_walking_to_target, spline_info_or_None).
        """
        pos = self._get_data_start(block_idx)

        for track_idx in range(self.num_transform_tracks):
            mask = self.track_masks[track_idx]

            # --- Position ---
            if mask.pos_type == "spline":
                if track_idx == target_track:
                    return pos, self._parse_pos_spline_at(pos, mask)

                # Skip this track's position data
                pos = self._skip_pos_spline(pos, mask)

            elif mask.pos_type == "static":
                if track_idx == target_track:
                    return pos, None  # static, no spline info
                num_static = _count_set_bits(mask.pos_flags, 0x0F)
                pos += num_static * 4  # float32 per static axis

            # Identity: no data to skip

            # --- Rotation ---
            pos = self._skip_rotation(pos, mask)

            # --- Scale ---
            pos = self._skip_scale(pos, mask)

        return pos, None

    def _parse_pos_spline_at(self, pos: int, mask: TrackMask) -> _PosSplineInfo:
        """Parse position spline metadata starting at pos."""
        info = _PosSplineInfo()
        info.quant = mask.pos_quant

        # Spline header
        info.num_items = struct.unpack_from("<H", self.blob, pos)[0]
        pos += 2
        info.degree = self.blob[pos]
        pos += 1
        num_knots = info.num_items + info.degree + 2
        info.knots_start = pos
        info.knots_end = pos + num_knots
        pos = info.knots_end
        pos = _align(pos, 4)

        # Min/max and static values per axis
        info.dynamic_axes = []
        info.static_values = {}
        info.num_dynamic = 0

        for comp in range(3):
            if mask.pos_flags & (_SPLINE_X << comp):
                mn = struct.unpack_from("<f", self.blob, pos)[0]
                pos += 4
                mx = struct.unpack_from("<f", self.blob, pos)[0]
                pos += 4
                info.dynamic_axes.append((comp, mn, mx))
                info.num_dynamic += 1
            elif mask.pos_flags & (_STATIC_X << comp):
                val = struct.unpack_from("<f", self.blob, pos)[0]
                pos += 4
                info.static_values[comp] = val

        info.num_cps = info.num_items + 1
        info.cps_start = pos

        return info

    def _skip_pos_spline(self, pos: int, mask: TrackMask) -> int:
        """Skip past a position spline track's data."""
        num_items = struct.unpack_from("<H", self.blob, pos)[0]
        pos += 2
        degree = self.blob[pos]
        pos += 1
        num_knots = num_items + degree + 2
        pos += num_knots
        pos = _align(pos, 4)

        num_dynamic = _count_set_bits(mask.pos_flags, 0xF0)

        for comp in range(3):
            if mask.pos_flags & (_SPLINE_X << comp):
                pos += 8  # min + max (float32 each)
            elif mask.pos_flags & (_STATIC_X << comp):
                pos += 4  # static float32

        num_cps = num_items + 1
        cp_size = _scalar_size(mask.pos_quant) * num_dynamic
        pos += num_cps * cp_size
        pos = _align(pos, 4)
        return pos

    def _skip_rotation(self, pos: int, mask: TrackMask) -> int:
        """Skip past a rotation track's data."""
        if mask.rot_type == "spline":
            num_items = struct.unpack_from("<H", self.blob, pos)[0]
            pos += 2
            degree = self.blob[pos]
            pos += 1
            num_knots = num_items + degree + 2
            pos += num_knots
            pos = _align(pos, _rot_alignment(mask.rot_quant))

            num_cps = num_items + 1
            rot_size = ROT_SIZES.get(mask.rot_quant, 4)
            pos += num_cps * rot_size
            pos = _align(pos, 4)

        elif mask.rot_type == "static":
            pos = _align(pos, _rot_alignment(mask.rot_quant))
            rot_size = ROT_SIZES.get(mask.rot_quant, 4)
            pos += rot_size
            pos = _align(pos, 4)

        return pos

    def _skip_scale(self, pos: int, mask: TrackMask) -> int:
        """Skip past a scale track's data."""
        if mask.scale_type == "spline":
            num_items = struct.unpack_from("<H", self.blob, pos)[0]
            pos += 2
            degree = self.blob[pos]
            pos += 1
            num_knots = num_items + degree + 2
            pos += num_knots
            pos = _align(pos, 4)

            num_dynamic = _count_set_bits(mask.scale_flags, 0xF0)
            for comp in range(3):
                if mask.scale_flags & (_SPLINE_X << comp):
                    pos += 8  # min + max
                elif mask.scale_flags & (_STATIC_X << comp):
                    pos += 4

            num_cps = num_items + 1
            cp_size = _scalar_size(mask.scale_quant) * num_dynamic
            pos += num_cps * cp_size
            pos = _align(pos, 4)

        elif mask.scale_type == "static":
            num_static = _count_set_bits(mask.scale_flags, 0x0F)
            pos += num_static * 4

        return pos

    def get_translation_cps(self, track_idx: int, block_idx: int = 0) -> list[np.ndarray]:
        """Get decoded translation control points for a track.

        Returns list of (3,) arrays, one per control point.
        """
        pos, info = self._walk_to_track_pos(track_idx, block_idx)
        mask = self.track_masks[track_idx]

        if mask.pos_type != "spline" or info is None:
            return []

        result = []
        cp_pos = info.cps_start
        sz = _scalar_size(info.quant)

        for cp_i in range(info.num_cps):
            vals = [0.0, 0.0, 0.0]
            # Fill static values
            for axis, val in info.static_values.items():
                vals[axis] = val
            # Decode dynamic axes
            for axis, mn, mx in info.dynamic_axes:
                vals[axis] = decode_scalar(self.blob, cp_pos, mn, mx, info.quant)
                cp_pos += sz
            result.append(np.array(vals))

        return result

    def offset_translation(self, track_idx: int, delta: np.ndarray, block_idx: int = 0):
        """Apply a translation offset to a track's spline control points.

        Decodes CPs, adds delta, recomputes min/max range, re-quantizes
        all CPs, and writes back to blob.
        """
        pos, info = self._walk_to_track_pos(track_idx, block_idx)
        mask = self.track_masks[track_idx]

        if mask.pos_type != "spline" or info is None:
            _log.warning("Track %d has no spline position data", track_idx)
            return

        # Decode all CPs
        cps = self.get_translation_cps(track_idx, block_idx)
        if not cps:
            return

        # Apply delta
        new_cps = [cp + delta for cp in cps]

        # Recompute min/max per dynamic axis
        new_ranges = {}
        for axis, old_mn, old_mx in info.dynamic_axes:
            axis_vals = [float(cp[axis]) for cp in new_cps]
            new_mn = min(axis_vals)
            new_mx = max(axis_vals)
            # Add small epsilon to avoid degenerate range
            if abs(new_mx - new_mn) < 1e-10:
                new_mx = new_mn + 1e-6
            new_ranges[axis] = (new_mn, new_mx)

        # Write new min/max values back to blob
        range_pos = info.knots_end
        range_pos = _align(range_pos, 4)

        for comp in range(3):
            if mask.pos_flags & (_SPLINE_X << comp):
                mn, mx = new_ranges[comp]
                struct.pack_into("<f", self.blob, range_pos, mn)
                range_pos += 4
                struct.pack_into("<f", self.blob, range_pos, mx)
                range_pos += 4
            elif mask.pos_flags & (_STATIC_X << comp):
                # Static value gets delta applied
                old_val = struct.unpack_from("<f", self.blob, range_pos)[0]
                struct.pack_into("<f", self.blob, range_pos, old_val + float(delta[comp]))
                range_pos += 4

        # Re-quantize and write CPs
        cp_pos = info.cps_start
        sz = _scalar_size(info.quant)

        for cp in new_cps:
            for axis, mn, mx in info.dynamic_axes:
                encoded = encode_scalar(float(cp[axis]), mn, mx, info.quant)
                # Wait — we need to use the NEW ranges
                encoded = encode_scalar(float(cp[axis]), new_ranges[axis][0], new_ranges[axis][1], info.quant)
                self.blob[cp_pos:cp_pos + sz] = encoded
                cp_pos += sz

    def get_rotation_cps(self, track_idx: int, block_idx: int = 0) -> list[np.ndarray]:
        """Get decoded rotation control points for a track.

        Returns list of (4,) quaternion arrays (x,y,z,w).
        """
        mask = self.track_masks[track_idx]
        if mask.rot_type != "spline":
            return []

        # Walk to rotation data for this track
        pos = self._walk_to_rotation(track_idx, block_idx)
        if pos is None:
            return []

        # Parse spline header
        num_items = struct.unpack_from("<H", self.blob, pos)[0]
        pos += 2
        degree = self.blob[pos]
        pos += 1
        num_knots = num_items + degree + 2
        pos += num_knots
        pos = _align(pos, _rot_alignment(mask.rot_quant))

        num_cps = num_items + 1
        rot_size = ROT_SIZES.get(mask.rot_quant, 4)

        result = []
        for _ in range(num_cps):
            q = decode_quaternion(self.blob, pos, mask.rot_quant)
            result.append(q)
            pos += rot_size

        return result

    def get_single_rotation(self, track_idx: int, block_idx: int = 0) -> np.ndarray:
        """Get rotation for a track — static value, first CP, or identity.

        Used for parent chain FK approximation in world-to-local conversion.
        Returns (x,y,z,w) quaternion.
        """
        if track_idx >= len(self.track_masks):
            return np.array([0.0, 0.0, 0.0, 1.0])

        mask = self.track_masks[track_idx]
        if mask.rot_type == "identity":
            return np.array([0.0, 0.0, 0.0, 1.0])

        pos = self._walk_to_rotation(track_idx, block_idx)
        if pos is None:
            return np.array([0.0, 0.0, 0.0, 1.0])

        if mask.rot_type == "static":
            pos = _align(pos, _rot_alignment(mask.rot_quant))
            return decode_quaternion(self.blob, pos, mask.rot_quant)

        # Spline: read first control point
        num_items = struct.unpack_from("<H", self.blob, pos)[0]
        pos += 2
        degree = self.blob[pos]
        pos += 1
        num_knots = num_items + degree + 2
        pos += num_knots
        pos = _align(pos, _rot_alignment(mask.rot_quant))

        return decode_quaternion(self.blob, pos, mask.rot_quant)

    def offset_rotation(self, track_idx: int, delta_q: np.ndarray, block_idx: int = 0):
        """Apply a rotation offset to a track's spline control points.

        Pre-multiplies each CP by delta_q: new_q = delta_q * old_q.
        """
        from .quat_util import quat_multiply, quat_normalize

        mask = self.track_masks[track_idx]
        if mask.rot_type != "spline":
            _log.warning("Track %d has no spline rotation data", track_idx)
            return

        pos = self._walk_to_rotation(track_idx, block_idx)
        if pos is None:
            return

        # Parse spline header
        num_items = struct.unpack_from("<H", self.blob, pos)[0]
        pos += 2
        degree = self.blob[pos]
        pos += 1
        num_knots = num_items + degree + 2
        pos += num_knots
        pos = _align(pos, _rot_alignment(mask.rot_quant))

        num_cps = num_items + 1
        rot_size = ROT_SIZES.get(mask.rot_quant, 4)

        for _ in range(num_cps):
            old_q = decode_quaternion(self.blob, pos, mask.rot_quant)
            new_q = quat_normalize(quat_multiply(delta_q, old_q))
            encoded = encode_quaternion(new_q, mask.rot_quant)
            self.blob[pos:pos + rot_size] = encoded
            pos += rot_size

    def _walk_to_rotation(self, target_track: int, block_idx: int = 0) -> int | None:
        """Walk blob to find start of rotation data for target_track."""
        pos = self._get_data_start(block_idx)

        for track_idx in range(self.num_transform_tracks):
            mask = self.track_masks[track_idx]

            # Skip position
            if mask.pos_type == "spline":
                pos = self._skip_pos_spline(pos, mask)
            elif mask.pos_type == "static":
                num_static = _count_set_bits(mask.pos_flags, 0x0F)
                pos += num_static * 4

            # Rotation
            if track_idx == target_track:
                if mask.rot_type != "identity":
                    return pos
                return None

            pos = self._skip_rotation(pos, mask)

            # Skip scale
            pos = self._skip_scale(pos, mask)

        return None

    def get_patched_blob(self) -> bytes:
        """Return the modified blob as bytes."""
        return bytes(self.blob)

    def write_to_hkx_file(self, hkx_file: HKXFile) -> None:
        """Write the patched blob back into the HKXFile's SplineCompressedAnimation data array."""
        anim_obj = _find_spline_animation(hkx_file)
        if anim_obj is None:
            raise ValueError("No hkaSplineCompressedAnimation found in HKXFile")
        data_member = _get_member(anim_obj, "data")
        if not isinstance(data_member, HKXArrayMember):
            raise ValueError("hkaSplineCompressedAnimation has no 'data' array member")
        data_member.contents = [int(b) & 0xFF for b in self.blob]
