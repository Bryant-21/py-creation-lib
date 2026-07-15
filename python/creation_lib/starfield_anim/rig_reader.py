"""Parse Starfield .rig skeleton files (SFBGS Skeleton Rig format).

Binary format: 80-byte header + N x 96-byte bone entries + 628-byte bone map + string table.
Reference: refs/CALUMI.Animation/CALUMI.Animation/SFBGS_SkeletonRig.cpp lines 582-679.
"""
from __future__ import annotations

import struct
from dataclasses import dataclass, field
from pathlib import Path

# Bone entry: 96 bytes
# local_rot wxyz (4f) + global_rot wxyz (4f) + position xyz (3f) + bone_type (i)
# + name_offset (Q) + parent_index (i) + twist_mqn (i) + twist_driver (i)
# + pad1 (i) + mirror (i) + term (i) + twist_weight (f) + pad2 (i)
# + unknown_scalar (f) + pad3 (i)
_BONE_FMT = "<4f4f3fiQiiiiiififi"
_BONE_SIZE = struct.calcsize(_BONE_FMT)  # 96

# Bone map: 157 x i16 = 314 bytes
# Reference: SFBGS_RigPackage.h defines SFBGSMAPSIZE = 157
_BONE_MAP_COUNT = 157
_BONE_MAP_FMT = f"<{_BONE_MAP_COUNT}h"
_BONE_MAP_SIZE = struct.calcsize(_BONE_MAP_FMT)  # 314


@dataclass
class RigData:
    """Parsed Starfield skeleton rig."""
    name: str = ""  # SkeletonData-compatible: set from first bone name or path
    version: int = 0
    bone_count: int = 0
    animated_bone_count: int = 0
    bone_names: list[str] = field(default_factory=list)
    parent_indices: list[int] = field(default_factory=list)
    reference_pose: list[dict] = field(default_factory=list)  # [{t,q,s}, ...]
    bone_types: list[int] = field(default_factory=list)
    mirror_indices: list[int] = field(default_factory=list)
    twist_drivers: list[dict] = field(default_factory=list)
    low_precision: float = 0.0
    high_precision: float = 0.0
    bone_map: list[int] = field(default_factory=list)
    # SkeletonData-compatible fields (not present in .rig format)
    float_count: int = 0
    float_slots: list[str] = field(default_factory=list)
    partition_names: list[str] = field(default_factory=list)


def parse_rig(rig_path: Path) -> RigData:
    """Parse a .rig file and return skeleton data."""
    result = RigData()
    try:
        data = rig_path.read_bytes()
    except OSError:
        return result

    if len(data) < 80:
        return result

    # --- Header (80 bytes) ---
    # Read fields manually for clarity and to handle the 16-byte reserved block
    off = 0
    result.version = struct.unpack_from("<i", data, off)[0]; off += 4
    file_size = struct.unpack_from("<I", data, off)[0]; off += 4
    header_size = struct.unpack_from("<I", data, off)[0]; off += 4
    _pad1 = struct.unpack_from("<I", data, off)[0]; off += 4
    bone_map_offset = struct.unpack_from("<I", data, off)[0]; off += 4
    _pad2 = struct.unpack_from("<I", data, off)[0]; off += 4
    _tracking = struct.unpack_from("<3q", data, off); off += 24
    result.low_precision = struct.unpack_from("<f", data, off)[0]; off += 4
    result.high_precision = struct.unpack_from("<f", data, off)[0]; off += 4
    result.bone_count = struct.unpack_from("<H", data, off)[0]; off += 2
    result.animated_bone_count = struct.unpack_from("<H", data, off)[0]; off += 2
    _pad3 = struct.unpack_from("<I", data, off)[0]; off += 4
    # 16-byte reserved block
    off += 16  # skip _endOfHeader
    # off should now be 80

    # --- Bone entries (96 bytes each) ---
    name_offsets: list[int] = []  # Absolute file offsets into string table
    for i in range(result.bone_count):
        if off + _BONE_SIZE > len(data):
            break
        fields = struct.unpack_from(_BONE_FMT, data, off)
        off += _BONE_SIZE

        # Unpack fields matching C++ read order:
        # local_rot: w,x,y,z (indices 0-3)
        # global_rot: w,x,y,z (indices 4-7)
        # position: x,y,z (indices 8-10)
        # bone_type (11), name_offset (12), parent_index (13)
        # twist_mqn (14), twist_driver (15), pad1 (16)
        # mirror (17), term05 (18), twist_weight (19)
        # pad2 (20), unknown_scalar (21), pad3 (22)
        lw, lx, ly, lz = fields[0], fields[1], fields[2], fields[3]
        px, py, pz = fields[8], fields[9], fields[10]
        bone_type = fields[11]
        name_offset = fields[12]
        parent_index = fields[13]
        twist_mqn = fields[14]
        twist_driver_idx = fields[15]
        mirror = fields[17]
        twist_weight = fields[19]

        name_offsets.append(name_offset)

        # Normalize quaternion to xyzw (matching existing SkeletonData convention)
        result.reference_pose.append({
            "t": [px, py, pz],
            "q": [lx, ly, lz, lw],  # wxyz in binary -> xyzw for output
            "s": [1.0, 1.0, 1.0],
        })
        result.parent_indices.append(parent_index)
        result.bone_types.append(bone_type)
        result.mirror_indices.append(mirror)
        result.twist_drivers.append({
            "driver_index": twist_driver_idx,
            "mqn_index": twist_mqn,
            "weight": twist_weight,
        })

    # --- Bone map (314 bytes at bone_map_offset) ---
    if bone_map_offset + _BONE_MAP_SIZE <= len(data):
        result.bone_map = list(struct.unpack_from(_BONE_MAP_FMT, data, bone_map_offset))

    # --- String table --- use name_offset from each bone entry ---
    # C++ reads strings via absolute file offset stored in bone entry's name_offset field
    # (see SFBGS_SkeletonRig.cpp line 659)
    for name_off in name_offsets:
        if name_off >= len(data):
            result.bone_names.append("")
            continue
        # Find null terminator
        end = data.index(b"\x00", name_off) if b"\x00" in data[name_off:] else len(data)
        result.bone_names.append(data[name_off:end].decode("utf-8", errors="replace"))

    # Set name from first bone or empty
    if result.bone_names:
        result.name = result.bone_names[0]

    return result
