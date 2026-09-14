"""MOPP (Memory Optimized Partial Polytope) bytecode compiler.

Compiles a BVH decision tree from collision triangles into MOPP bytecode
used by Havok physics in Skyrim LE/SE and Fallout 3/NV.

Ported from pynifly pyn/mopp_compiler.py.
Reference: https://github.com/niftools/nifxml/wiki/Havok-MOPP-Data-format
"""
from __future__ import annotations

import logging
import math
from typing import Sequence

log = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

def compile_mopp(
    verts: Sequence[tuple[float, float, float]],
    triangles: Sequence[tuple[int, int, int]],
    radius: float = 0.005,
    output_ids: list[int] | None = None,
) -> tuple[bytes, tuple[float, float, float], float]:
    """Build MOPP bytecode for triangles in Havok space; return ``(mopp_bytes, origin, scale)``.

    ``radius`` is 0.005 for Skyrim SE, 0.1 for Oblivion/FO3. ``output_ids`` are
    per-triangle uint32 ids, sequential by default.
    """
    if not triangles:
        return b"", (0.0, 0.0, 0.0), 0.0

    if output_ids is None:
        output_ids = list(range(len(triangles)))

    # Compute AABB expanded by radius
    xs = [verts[i][0] for tri in triangles for i in tri]
    ys = [verts[i][1] for tri in triangles for i in tri]
    zs = [verts[i][2] for tri in triangles for i in tri]

    min_x, max_x = min(xs), max(xs)
    min_y, max_y = min(ys), max(ys)
    min_z, max_z = min(zs), max(zs)

    origin = (min_x - radius, min_y - radius, min_z - radius)
    size_x = (max_x - min_x) + 2 * radius
    size_y = (max_y - min_y) + 2 * radius
    size_z = (max_z - min_z) + 2 * radius
    largest_dim = max(size_x, size_y, size_z)

    if largest_dim <= 0:
        largest_dim = 1e-6

    scale = 254.0 * 256.0 * 256.0 / largest_dim

    # Build per-triangle AABBs (expanded by radius)
    tri_data: list[_TriInfo] = []
    for ti, tri in enumerate(triangles):
        v0, v1, v2 = verts[tri[0]], verts[tri[1]], verts[tri[2]]
        tmin = [min(v0[a], v1[a], v2[a]) - radius for a in range(3)]
        tmax = [max(v0[a], v1[a], v2[a]) + radius for a in range(3)]
        centroid = [(tmin[a] + tmax[a]) * 0.5 for a in range(3)]
        tri_data.append(_TriInfo(ti, output_ids[ti], tmin, tmax, centroid))

    # Build BVH
    root = _build_bvh(tri_data, origin, largest_dim)

    # Encode to bytecode
    code = _encode_node(root, origin, largest_dim)

    # Prepend root bounding filters
    code = _add_root_filters(code, origin, largest_dim,
                             root.bbox_min, root.bbox_max)

    return bytes(code), origin, scale


def disassemble_mopp(
    mopp_bytes: bytes,
    origin: tuple[float, float, float] | None = None,
    scale: float | None = None,
) -> list[str]:
    """Disassemble MOPP bytecode into indented tree lines.

    ``origin`` and ``scale`` only add world-space annotations.
    """
    if not mopp_bytes:
        return []

    data = mopp_bytes
    lines: list[str] = []

    # Derive largest_dim from root FILTER bounds + origin
    largest_dim = None
    if origin is not None:
        largest_dim = _derive_largest_dim(data, origin)

    axis_names = ["X", "Y", "Z"]

    def _world(axis: int, bound_byte: int, is_upper: bool) -> str:
        if origin is None or largest_dim is None or axis >= 3:
            return ""
        if is_upper:
            val = (bound_byte - 1) / 254.0 * largest_dim + origin[axis]
        else:
            val = bound_byte / 254.0 * largest_dim + origin[axis]
        return f"={val:.4f}"

    def _walk(pos: int, end: int, indent: int) -> None:
        pad = "    " * indent
        while pos < end and pos < len(data):
            op = data[pos]

            if 0x01 <= op <= 0x04:
                shift = op
                xx, yy, zz = data[pos + 1], data[pos + 2], data[pos + 3]
                lines.append(
                    f"{pad}[{pos:04X}] RESCALE shift={shift} "
                    f"sub=({xx:02X},{yy:02X},{zz:02X})"
                )
                pos += 4

            elif op == 0x05:
                cc = data[pos + 1]
                target = pos + 2 + cc
                lines.append(f"{pad}[{pos:04X}] JUMP -> {target:04X}")
                return

            elif op == 0x06:
                cc = (data[pos + 1] << 8) | data[pos + 2]
                target = pos + 3 + cc
                lines.append(f"{pad}[{pos:04X}] JUMP -> {target:04X}")
                return

            elif op == 0x09:
                ii = data[pos + 1]
                lines.append(f"{pad}[{pos:04X}] ADD_OUTPUT +0x{ii:02X}")
                pos += 2

            elif op == 0x0A:
                ii = (data[pos + 1] << 8) | data[pos + 2]
                lines.append(f"{pad}[{pos:04X}] ADD_OUTPUT +0x{ii:04X}")
                pos += 3

            elif op == 0x0B:
                ii = (
                    (data[pos + 1] << 24)
                    | (data[pos + 2] << 16)
                    | (data[pos + 3] << 8)
                    | data[pos + 4]
                )
                lines.append(f"{pad}[{pos:04X}] SET_OUTPUT 0x{ii:08X}")
                pos += 5

            elif 0x10 <= op <= 0x1C:
                axis = op - 0x10
                bb, aa, cc = data[pos + 1], data[pos + 2], data[pos + 3]
                right_start = pos + 4 + cc
                aname = _split_axis_name(axis)
                w_hi = _world(axis, bb, True)
                w_lo = _world(axis, aa, False)
                lines.append(
                    f"{pad}[{pos:04X}] SPLIT {aname}  "
                    f"<{bb:02X}{w_hi} | >={aa:02X}{w_lo}"
                )
                lines.append(f"{pad}  if {aname} < {bb:02X}{w_hi}:")
                _walk(pos + 4, right_start, indent + 1)
                lines.append(f"{pad}  if {aname} >= {aa:02X}{w_lo}:")
                _walk(right_start, end, indent + 1)
                return

            elif 0x20 <= op <= 0x22:
                axis = op - 0x20
                xx, cc = data[pos + 1], data[pos + 2]
                right_start = pos + 3 + cc
                aname = axis_names[axis]
                w = _world(axis, xx, True)
                lines.append(
                    f"{pad}[{pos:04X}] SPLIT {aname}  bound={xx:02X}{w}"
                )
                lines.append(f"{pad}  if {aname} < {xx:02X}{w}:")
                _walk(pos + 3, right_start, indent + 1)
                lines.append(f"{pad}  if {aname} >= {xx:02X}{w}:")
                _walk(right_start, end, indent + 1)
                return

            elif 0x23 <= op <= 0x25:
                axis = op - 0x23
                bb, aa = data[pos + 1], data[pos + 2]
                cc = (data[pos + 3] << 8) | data[pos + 4]
                dd = (data[pos + 5] << 8) | data[pos + 6]
                instr_end = pos + 7
                lo_start = instr_end + cc
                hi_start = instr_end + dd
                aname = axis_names[axis]
                w_hi = _world(axis, bb, True)
                w_lo = _world(axis, aa, False)
                lines.append(
                    f"{pad}[{pos:04X}] SPLIT16 {aname}  "
                    f"<{bb:02X}{w_hi} | >={aa:02X}{w_lo}"
                )
                lines.append(f"{pad}  if {aname} < {bb:02X}{w_hi}:")
                _walk(lo_start, hi_start, indent + 1)
                lines.append(f"{pad}  if {aname} >= {aa:02X}{w_lo}:")
                _walk(hi_start, end, indent + 1)
                return

            elif 0x26 <= op <= 0x28:
                axis = op - 0x26
                aa, bb = data[pos + 1], data[pos + 2]
                aname = axis_names[axis]
                w_lo = _world(axis, aa, False)
                w_hi = _world(axis, bb, True)
                lines.append(
                    f"{pad}[{pos:04X}] FILTER {aname}  "
                    f"{aa:02X}{w_lo}..{bb:02X}{w_hi}"
                )
                pos += 3

            elif 0x29 <= op <= 0x2B:
                axis = op - 0x29
                aa = (
                    (data[pos + 1] << 16) | (data[pos + 2] << 8) | data[pos + 3]
                )
                bb = (
                    (data[pos + 4] << 16) | (data[pos + 5] << 8) | data[pos + 6]
                )
                aname = axis_names[axis]
                lines.append(
                    f"{pad}[{pos:04X}] FILTER24 {aname}  {aa:06X}..{bb:06X}"
                )
                pos += 7

            elif 0x30 <= op <= 0x4F:
                output_id = op - 0x30
                lines.append(f"{pad}[{pos:04X}] LEAF 0x{output_id:08X}")
                pos += 1

            elif op == 0x50:
                ii = data[pos + 1]
                lines.append(f"{pad}[{pos:04X}] LEAF 0x{ii:08X}")
                pos += 2

            elif op == 0x51:
                ii = (data[pos + 1] << 8) | data[pos + 2]
                lines.append(f"{pad}[{pos:04X}] LEAF 0x{ii:08X}")
                pos += 3

            elif op == 0x52:
                ii = (
                    (data[pos + 1] << 16)
                    | (data[pos + 2] << 8)
                    | data[pos + 3]
                )
                lines.append(f"{pad}[{pos:04X}] LEAF 0x{ii:08X}")
                pos += 4

            else:
                lines.append(f"{pad}[{pos:04X}] UNKNOWN 0x{op:02X}")
                pos += 1

    _walk(0, len(data), 0)
    return lines


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------

def _derive_largest_dim(
    data: bytes, origin: tuple[float, float, float]
) -> float | None:
    """Estimate largest_dim from root FILTER nodes and origin."""
    filters: dict[int, tuple[int, int]] = {}
    pos = 0
    while pos < len(data) and len(filters) < 3:
        op = data[pos]
        if 0x26 <= op <= 0x28:
            filters[op - 0x26] = (data[pos + 1], data[pos + 2])
            pos += 3
        else:
            break

    if not filters:
        return None

    for axis, (lo, hi) in filters.items():
        if hi == 0xFF and lo == 0x00:
            ld = -2.0 * origin[axis]
            if ld > 0:
                return ld

    return None


def _split_axis_name(axis: int) -> str:
    """Return axis name for split opcodes 0x10-0x1C."""
    names = [
        "X", "Y", "Z",
        "YpZ", "nYpZ",
        "XpZ", "nXpZ",
        "XpY", "nXpY",
        "XpYpZ",
        "XpYnZ", "XnYpZ", "nXpYpZ",
    ]
    if axis < len(names):
        return names[axis]
    return f"?{axis}"


class _TriInfo:
    """Per-triangle data for BVH construction."""

    __slots__ = ("index", "output_id", "bbox_min", "bbox_max", "centroid")

    def __init__(
        self,
        index: int,
        output_id: int,
        bbox_min: list[float],
        bbox_max: list[float],
        centroid: list[float],
    ) -> None:
        self.index = index
        self.output_id = output_id
        self.bbox_min = bbox_min
        self.bbox_max = bbox_max
        self.centroid = centroid


class _BVHNode:
    """Binary BVH node."""

    __slots__ = ("tris", "left", "right", "split_axis", "bbox_min", "bbox_max")

    def __init__(self) -> None:
        self.tris: list[_TriInfo] = []
        self.left: _BVHNode | None = None
        self.right: _BVHNode | None = None
        self.split_axis: int = -1
        self.bbox_min: list[float] = [0, 0, 0]
        self.bbox_max: list[float] = [0, 0, 0]


def _compute_bbox(tris: list[_TriInfo]) -> tuple[list[float], list[float]]:
    """Compute encompassing AABB of a list of _TriInfo."""
    bmin = [min(t.bbox_min[a] for t in tris) for a in range(3)]
    bmax = [max(t.bbox_max[a] for t in tris) for a in range(3)]
    return bmin, bmax


def _build_bvh(
    tris: list[_TriInfo],
    origin: tuple[float, float, float],
    largest_dim: float,
    depth: int = 0,
) -> _BVHNode:
    """Recursively build a BVH from triangle AABBs."""
    node = _BVHNode()
    node.bbox_min, node.bbox_max = _compute_bbox(tris)

    # Leaf condition: single triangle or max depth
    if len(tris) <= 1 or depth > 40:
        node.tris = tris
        return node

    # Choose split axis: longest AABB dimension, cycling on ties
    extents = [node.bbox_max[a] - node.bbox_min[a] for a in range(3)]
    max_ext = max(extents)
    tied = [a for a in range(3) if abs(extents[a] - max_ext) < 1e-9]
    axis = tied[depth % len(tied)]

    # Sort by centroid along split axis
    sorted_tris = sorted(tris, key=lambda t: t.centroid[axis])

    # Median split
    mid = len(sorted_tris) // 2
    if mid == 0:
        mid = 1

    left_tris = sorted_tris[:mid]
    right_tris = sorted_tris[mid:]

    # Degenerate case: all centroids identical on this axis
    if not left_tris or not right_tris:
        node.tris = tris
        return node

    node.split_axis = axis
    node.left = _build_bvh(left_tris, origin, largest_dim, depth + 1)
    node.right = _build_bvh(right_tris, origin, largest_dim, depth + 1)

    return node


def _encode_bound_upper(
    bound_max: float, origin_axis: float, largest_dim: float
) -> int:
    """Encode an upper bound to a MOPP byte (exclusive comparison)."""
    val = math.floor(1 + 254.0 * (bound_max - origin_axis) / largest_dim)
    return max(0, min(255, val))


def _encode_bound_lower(
    bound_min: float, origin_axis: float, largest_dim: float
) -> int:
    """Encode a lower bound to a MOPP byte (inclusive comparison)."""
    val = math.floor(254.0 * (bound_min - origin_axis) / largest_dim)
    return max(0, min(255, val))


def _emit_leaf(output_id: int) -> bytearray:
    """Emit leaf opcodes for a given output ID."""
    code = bytearray()
    if output_id <= 0x1F:
        code.append(0x30 + output_id)
    elif output_id <= 0xFF:
        code.append(0x50)
        code.append(output_id)
    elif output_id <= 0xFFFF:
        code.append(0x51)
        code.append((output_id >> 8) & 0xFF)
        code.append(output_id & 0xFF)
    else:
        code.append(0x52)
        code.append((output_id >> 16) & 0xFF)
        code.append((output_id >> 8) & 0xFF)
        code.append(output_id & 0xFF)
    return code


def _encode_node(
    node: _BVHNode,
    origin: tuple[float, float, float],
    largest_dim: float,
) -> bytearray:
    """Recursively encode a BVH node to MOPP bytecode."""
    code = bytearray()

    # Leaf node
    if node.left is None and node.right is None:
        for tri in node.tris:
            # Per-triangle FILTER nodes reject points outside this
            # triangle's AABB
            for a in range(3):
                lo = _encode_bound_lower(tri.bbox_min[a], origin[a], largest_dim)
                hi = _encode_bound_upper(tri.bbox_max[a], origin[a], largest_dim)
                code.append(0x26 + a)
                code.append(lo)
                code.append(hi)
            code.extend(_emit_leaf(tri.output_id))
        return code

    # Internal split node
    axis = node.split_axis

    left_code = _encode_node(node.left, origin, largest_dim)
    right_code = _encode_node(node.right, origin, largest_dim)

    # BB = upper bound of left child on split axis
    # AA = lower bound of right child on split axis
    bb = _encode_bound_upper(
        node.left.bbox_max[axis], origin[axis], largest_dim
    )
    aa = _encode_bound_lower(
        node.right.bbox_min[axis], origin[axis], largest_dim
    )

    right_offset = len(left_code)

    if right_offset <= 255:
        # 1-byte jump: opcode 0x10+axis, BB, AA, CC
        code.append(0x10 + axis)
        code.append(bb)
        code.append(aa)
        code.append(right_offset)
    elif right_offset <= 65535:
        # 2-byte jumps: opcode 0x23+axis, BB, AA, CC_hi, CC_lo, DD_hi, DD_lo
        # CC = offset to left child (0, immediately after)
        # DD = offset to right child (len(left_code))
        code.append(0x23 + axis)
        code.append(bb)
        code.append(aa)
        code.append(0)
        code.append(0)
        code.append((right_offset >> 8) & 0xFF)
        code.append(right_offset & 0xFF)
    else:
        # Tree too large for 2-byte jump — flatten to leaves
        log.warning(
            "MOPP subtree exceeds 64K bytes (%d); "
            "collision tree may be degraded",
            right_offset,
        )

        def _collect_leaves(n: _BVHNode) -> list[_TriInfo]:
            if n.left is None and n.right is None:
                return list(n.tris)
            result: list[_TriInfo] = []
            if n.left:
                result.extend(_collect_leaves(n.left))
            if n.right:
                result.extend(_collect_leaves(n.right))
            return result

        for tri in _collect_leaves(node):
            code.extend(_emit_leaf(tri.output_id))
        return code

    code.extend(left_code)
    code.extend(right_code)

    return code


def _add_root_filters(
    code: bytearray,
    origin: tuple[float, float, float],
    largest_dim: float,
    bbox_min: list[float],
    bbox_max: list[float],
) -> bytearray:
    """Prepend axis filter nodes for the root bounding box.

    The engine derives the quantisation scale from these filters.
    The largest axis MUST have hi=0xFF or the scale will be wrong
    and all spatial queries will miss.
    """
    prefix = bytearray()
    filters: list[tuple[int, int]] = []
    for axis in range(3):
        lo = _encode_bound_lower(bbox_min[axis], origin[axis], largest_dim)
        hi = _encode_bound_upper(bbox_max[axis], origin[axis], largest_dim)
        filters.append((lo, hi))

    # Ensure at least one axis reaches 0xFF
    max_hi = max(f[1] for f in filters)
    if max_hi < 0xFF:
        for i in range(3):
            if filters[i][1] == max_hi:
                filters[i] = (filters[i][0], 0xFF)
                break

    for axis in range(3):
        prefix.append(0x26 + axis)
        prefix.append(filters[axis][0])
        prefix.append(filters[axis][1])
    return prefix + code
