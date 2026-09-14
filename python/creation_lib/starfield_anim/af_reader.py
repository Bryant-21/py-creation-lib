"""Parse Starfield .af animation files (SFBGS Animation Format) — metadata only.

Reads the 64-byte header to extract bone count, frame count, version, and flags.
Full keyframe transform decoding (prefix folding compression) is not implemented.

Binary layout (little-endian):
  Offset  Type     Field
  0       u64      magic
  8       f32x4    header_rotation (w, x, y, z)
  24      f32x3    header_translation (x, y, z)
  36      u8x4     flags (byte0 = bit field, bytes 1-3 = reserved)
  40      i16      version
  42      u16      bone_count
  44      u16      frame_count
  46      u16      index_atlas_count
  48      u16      fill_count
  50      u16      preamble_offset
  52      f32x3    zero_floats

Reference: refs/CALUMI.Animation/CALUMI.Animation/SFBGS_Animation.cpp lines 970-1018
Flags byte0 bits: 0=firstEntry, 1=shortKeyCounters, 2=shortKeyFrameEntries, 3=scalarSequenceFlag
"""
from __future__ import annotations

import struct
from dataclasses import dataclass, field
from pathlib import Path

_AF_HEADER_SIZE = 64  # Minimum file size for a valid .af
_DEFAULT_FPS = 30.0   # Assumed framerate; TBD confirm from CALUMI source


@dataclass
class AfData:
    """Parsed .af animation metadata."""
    magic: int = 0
    bone_count: int = 0
    frame_count: int = 0
    duration: float = 0.0
    version: int = 0
    flags: int = 0  # Raw byte0 as int (bit field)
    flags_raw: bytes = b"\x00\x00\x00\x00"  # All 4 flag bytes
    header_rotation: tuple = (0.0, 0.0, 0.0, 1.0)  # xyzw
    header_translation: tuple = (0.0, 0.0, 0.0)
    index_atlas_count: int = 0
    fill_count: int = 0
    preamble_offset: int = 0
    # Derived flag fields
    short_key_counters: bool = False
    short_key_frame_entries: bool = False
    has_scalar: bool = False
    # AnimationData-compatible fields
    compression_type: str = "sfbgs"
    annotation_tracks: list = field(default_factory=list)
    frame0_transforms: bytes | None = None


def parse_af(af_path: Path) -> AfData:
    """Parse an .af file header and return animation metadata."""
    result = AfData()
    try:
        data = af_path.read_bytes()
    except OSError:
        return result

    if len(data) < _AF_HEADER_SIZE:
        return result

    off = 0

    # magic (u64)
    result.magic = struct.unpack_from("<Q", data, off)[0]; off += 8

    # header_rotation: stored as w,x,y,z in binary -> normalize to x,y,z,w
    rw, rx, ry, rz = struct.unpack_from("<4f", data, off); off += 16
    result.header_rotation = (rx, ry, rz, rw)

    # header_translation
    result.header_translation = struct.unpack_from("<3f", data, off); off += 12

    # flags: 4 bytes, byte 0 is the bit field
    flag_bytes = data[off:off + 4]; off += 4
    result.flags_raw = flag_bytes
    b0 = flag_bytes[0]
    result.flags = b0
    result.short_key_counters = bool(b0 & 0x02)
    result.short_key_frame_entries = bool(b0 & 0x04)
    result.has_scalar = bool(b0 & 0x08)

    # version (i16), bone_count (u16), frame_count (u16)
    result.version = struct.unpack_from("<h", data, off)[0]; off += 2
    result.bone_count = struct.unpack_from("<H", data, off)[0]; off += 2
    result.frame_count = struct.unpack_from("<H", data, off)[0]; off += 2

    # index_atlas_count (u16), fill_count (u16), preamble_offset (u16)
    result.index_atlas_count = struct.unpack_from("<H", data, off)[0]; off += 2
    result.fill_count = struct.unpack_from("<H", data, off)[0]; off += 2
    result.preamble_offset = struct.unpack_from("<H", data, off)[0]; off += 2

    # zero_floats (3xf32) — skip
    off += 12

    # Derive duration
    if result.frame_count > 0:
        result.duration = result.frame_count / _DEFAULT_FPS

    return result
