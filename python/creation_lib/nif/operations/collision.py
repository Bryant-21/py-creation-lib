"""Collision/Havok operations — generate, remove, and convert collision shapes.

Generates Havok collision per-engine: Starfield uses Havok 2019 TAG0
(hknpPhysicsSystemData via build_convex_collision); FO4 uses Havok 2014.1.0 packfile
(hknpConvexPolytopeShape / hknpDynamicCompoundShape / hknpCompressedMeshShape);
legacy bhkRigidBody+bhkConvexVerticesShape chain reserved for Skyrim LE / FO3 / FNV.
"""
import json
import math

import numpy as np
from ..actions import OperationResult
from .collision_materials import resolve_collision_material

# Havok scale factor — NIF units / Havok units
# This value (69.99125) is used by all Bethesda Creation Engine games
# (Skyrim SE, Fallout 4, Fallout 76) for bhk collision blocks.
# Confirmed in NifSkope (gl/gltools.cpp:643) and PyNifly (nifconstants.py:11).
# The scale is a NIF-level constant for converting between NIF world units
# and Havok physics units, and does NOT vary between game versions.
HAVOK_SCALE = 69.99125

# Backward compat alias
HAVOK_SCALE_FO4 = HAVOK_SCALE

DEFAULT_RADIUS = 0.05
SIMPLIFIED_CONVEX_FIT_MAX_POINTS = 32
COMPRESSED_MESH_MAX_VERTICES = 255
COMPRESSED_MESH_MAX_TRIANGLES = 128

_FO4_MULTI_BODY_SHAPE_TYPES = {
    "auto",
    "box",
    "capsule",
    "compressed_mesh",
    "convex_fit",
    "convex_hull",
    "cylinder",
    "list",
    "mesh",
    "sphere",
    "auto_compressed_mesh",
}

_SHAPE_TYPES = ("BSTriShape", "BSSubIndexTriShape", "BSMeshLODTriShape")
_NODE_TYPES = ("NiNode", "BSFadeNode", "BSLeafAnimNode", "BSOrderedNode", "NiBillboardNode")
_BSX_HAVOK_MASK = 0x02

# Collision layers per game (exact values from nif.xml enums)
_COLLISION_LAYERS = {
    "Fallout4Layer": {
        "STATIC": 1, "ANIMSTATIC": 2, "TRANSPARENT": 3,
        "CLUTTER": 4, "WEAPON": 5, "PROJECTILE": 6,
        "NPC": 7, "TERRAIN": 13, "BIPED": 14,
        "TREES": 15, "DEADBIP": 17, "CHARCONTROLLER": 30,
    },
    "SkyrimLayer": {
        "STATIC": 1, "ANIMSTATIC": 2, "TRANSPARENT": 3,
        "CLUTTER": 4, "WEAPON": 5, "PROJECTILE": 6,
        "NPC": 7, "TERRAIN": 13, "BIPED": 14,
        "TREES": 15, "DEADBIP": 17, "CHARCONTROLLER": 30,
    },
    "StarfieldLayer": {
        "STATIC": 1, "ANIMSTATIC": 2, "TRANSPARENT": 3,
        "CLUTTER": 4, "WEAPON": 5, "PROJECTILE": 6,
        "NPC": 7, "TERRAIN": 13, "BIPED": 14,
        "TREES": 15, "DEADBIP": 17, "CHARCONTROLLER": 30,
    },
}

# Backward compat alias
FO4_LAYERS = _COLLISION_LAYERS["Fallout4Layer"]


def _bbox_collision_mesh(vertices: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    mins = np.asarray(vertices, dtype=np.float32).min(axis=0)
    maxs = np.asarray(vertices, dtype=np.float32).max(axis=0)
    x0, y0, z0 = mins.tolist()
    x1, y1, z1 = maxs.tolist()
    verts = np.asarray(
        [
            [x0, y0, z0],
            [x1, y0, z0],
            [x1, y1, z0],
            [x0, y1, z0],
            [x0, y0, z1],
            [x1, y0, z1],
            [x1, y1, z1],
            [x0, y1, z1],
        ],
        dtype=np.float32,
    )
    # Outward-facing winding (normals point away from the box center). hknp
    # compressed-mesh collision is one-sided; inward winding makes the top
    # un-standable (player falls through). Matches vanilla Safe01's base.
    tris = np.asarray(
        [
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 5, 4],
            [0, 1, 5],
            [1, 6, 5],
            [1, 2, 6],
            [2, 7, 6],
            [2, 3, 7],
            [3, 4, 7],
            [3, 0, 4],
        ],
        dtype=np.int32,
    )
    return verts, tris


def _qem_decimate_mesh(
    vertices: np.ndarray,
    triangles: np.ndarray,
    target_triangles: int,
) -> tuple[np.ndarray, np.ndarray]:
    from creation_lib._native.havok_native import decimate_mesh

    out_vertices, out_triangles = decimate_mesh(
        [[float(v[0]), float(v[1]), float(v[2])] for v in vertices],
        [[int(t[0]), int(t[1]), int(t[2])] for t in triangles],
        int(target_triangles),
    )
    return (
        np.asarray(out_vertices, dtype=np.float32),
        np.asarray(out_triangles, dtype=np.int32),
    )


def _triangle_mesh_has_closed_edges(
    triangles: np.ndarray,
    vertex_count: int,
) -> bool:
    triangles = np.asarray(triangles, dtype=np.int32)
    if triangles.ndim != 2 or triangles.shape[1] != 3 or len(triangles) == 0:
        return False

    edge_counts: dict[tuple[int, int], int] = {}
    for tri in triangles:
        a, b, c = (int(tri[0]), int(tri[1]), int(tri[2]))
        if a == b or b == c or a == c:
            return False
        if a < 0 or b < 0 or c < 0 or a >= vertex_count or b >= vertex_count or c >= vertex_count:
            return False
        for v0, v1 in ((a, b), (b, c), (c, a)):
            key = (v0, v1) if v0 < v1 else (v1, v0)
            edge_counts[key] = edge_counts.get(key, 0) + 1

    return bool(edge_counts) and all(count == 2 for count in edge_counts.values())


def _simplify_mesh_for_compressed_collision(
    vertices: np.ndarray,
    triangles: np.ndarray,
) -> tuple[np.ndarray, np.ndarray, bool]:
    vertices = np.asarray(vertices, dtype=np.float32)
    triangles = np.asarray(triangles, dtype=np.int32)
    if (
        len(vertices) <= COMPRESSED_MESH_MAX_VERTICES
        and len(triangles) <= COMPRESSED_MESH_MAX_TRIANGLES
    ):
        return vertices, triangles, False

    out_vertices, out_triangles = _qem_decimate_mesh(
        vertices,
        triangles,
        COMPRESSED_MESH_MAX_TRIANGLES,
    )
    if (
        len(out_vertices) > COMPRESSED_MESH_MAX_VERTICES
        or len(out_triangles) > COMPRESSED_MESH_MAX_TRIANGLES
        or not _triangle_mesh_has_closed_edges(out_triangles, len(out_vertices))
    ):
        out_vertices, out_triangles = _bbox_collision_mesh(vertices)
    return (
        np.asarray(out_vertices, dtype=np.float32),
        np.asarray(out_triangles, dtype=np.int32),
        True,
    )


def _profile_havok_scale(profile=None) -> float:
    return (
        float(profile.havok_scale)
        if profile and hasattr(profile, "havok_scale") and profile.havok_scale
        else HAVOK_SCALE
    )


def _add_unique_direction(directions: list[np.ndarray], direction: np.ndarray) -> None:
    norm = float(np.linalg.norm(direction))
    if norm <= 1e-8:
        return
    unit = np.asarray(direction, dtype=np.float64) / norm
    key = tuple(np.round(unit, 6))
    if any(tuple(np.round(existing, 6)) == key for existing in directions):
        return
    directions.append(unit)


def _convex_fit_support_directions(verts: np.ndarray) -> list[np.ndarray]:
    directions: list[np.ndarray] = []
    coeffs = [
        np.array([x, y, z], dtype=np.float64)
        for x in (-1.0, 0.0, 1.0)
        for y in (-1.0, 0.0, 1.0)
        for z in (-1.0, 0.0, 1.0)
        if (x, y, z) != (0.0, 0.0, 0.0)
    ]

    for coeff in coeffs:
        _add_unique_direction(directions, coeff)

    if len(verts) >= 3:
        centered = verts.astype(np.float64) - verts.mean(axis=0)
        cov = np.dot(centered.T, centered) / max(len(centered) - 1, 1)
        try:
            _values, axes = np.linalg.eigh(cov)
        except np.linalg.LinAlgError:
            axes = None
        if axes is not None:
            for coeff in coeffs:
                _add_unique_direction(directions, axes @ coeff)

    return directions


def _unique_points(verts: np.ndarray, tolerance: float = 1e-5) -> np.ndarray:
    seen: set[tuple[int, int, int]] = set()
    unique: list[np.ndarray] = []
    for vert in verts:
        key = tuple(np.round(vert / tolerance).astype(np.int64))
        if key in seen:
            continue
        seen.add(key)
        unique.append(vert)
    return np.asarray(unique, dtype=np.float32)


def _simplify_convex_fit_vertices(
    verts: np.ndarray,
    max_points: int = SIMPLIFIED_CONVEX_FIT_MAX_POINTS,
) -> np.ndarray:
    """Reduce a mesh point cloud to support vertices for a coarse convex fit."""
    verts = _unique_points(np.asarray(verts, dtype=np.float32))
    if len(verts) <= max_points:
        return verts

    center = verts.mean(axis=0)
    centered = verts - center
    candidates = []
    for direction in _convex_fit_support_directions(verts):
        idx = int(np.argmax(np.dot(centered, direction)))
        candidates.append(verts[idx])
    candidates_arr = _unique_points(np.asarray(candidates, dtype=np.float32))
    if len(candidates_arr) <= max_points:
        return candidates_arr

    extents = verts.max(axis=0) - verts.min(axis=0)
    scale = np.where(extents > 1e-6, extents, 1.0)
    normalized = (candidates_arr - center) / scale

    selected = [int(np.argmax(np.linalg.norm(normalized, axis=1)))]
    min_distances = np.linalg.norm(normalized - normalized[selected[0]], axis=1)
    while len(selected) < max_points:
        min_distances[selected] = -1.0
        next_idx = int(np.argmax(min_distances))
        if min_distances[next_idx] < 0.0:
            break
        selected.append(next_idx)
        dist = np.linalg.norm(normalized - normalized[next_idx], axis=1)
        min_distances = np.minimum(min_distances, dist)

    return candidates_arr[selected]


def get_collision_layers(profile=None) -> dict[str, int]:
    """Get collision layer map for a game profile (defaults to FO4)."""
    if profile is None:
        return _COLLISION_LAYERS["Fallout4Layer"]
    enum_name = profile.collision_layer_enum
    return _COLLISION_LAYERS.get(enum_name, _COLLISION_LAYERS["Fallout4Layer"])


def _extract_vertices(nif, block_id: int) -> np.ndarray | None:
    """Extract vertex positions from a BSTriShape block as Nx3 float32 array."""
    block = nif.get_block(block_id)
    if not block:
        return None
    vertex_data = block.get_field("Vertex Data") or []
    if not vertex_data:
        return None
    return np.array(
        [[float(vd.get("Vertex", {}).get(a, 0)) for a in ("x", "y", "z")]
         for vd in vertex_data],
        dtype=np.float32,
    )


def _extract_triangles(nif, block_id: int) -> np.ndarray | None:
    """Extract triangle indices from a BSTriShape block as Mx3 int32 array."""
    block = nif.get_block(block_id)
    if not block:
        return None
    tris = block.get_field("Triangles") or []
    if not tris:
        return None
    return np.array(
        [[t["v1"], t["v2"], t["v3"]] for t in tris],
        dtype=np.int32,
    )


def _collect_all_triangles(nif, source_block_ids: list[int]) -> np.ndarray:
    """Collect triangles from multiple BSTriShapes, adjusting indices for combined vertex array."""
    all_tris = []
    vert_offset = 0
    for sid in source_block_ids:
        tris = _extract_triangles(nif, sid)
        verts = _extract_vertices(nif, sid)
        if tris is not None and verts is not None:
            adjusted = tris + vert_offset
            all_tris.append(adjusted)
            vert_offset += len(verts)
    if all_tris:
        return np.vstack(all_tris)
    return np.empty((0, 3), dtype=np.int32)


def _ref_to_block_id(ref) -> int:
    if isinstance(ref, int):
        return ref
    if isinstance(ref, float):
        return int(ref)
    if isinstance(ref, dict):
        return int(ref.get("value", ref.get("Value", -1)))
    return -1


def _rotation_field_to_3x3(rotation) -> np.ndarray:
    """Coerce a NIF block Rotation field to a 3×3 ndarray.

    Native NIF blocks return Rotation as a nested 3-row list where row i, col j
    holds m{i+1}{j+1}; the {m11..m33} dict form is accepted for back-compat.
    """
    if isinstance(rotation, dict):
        return np.array(
            [
                [rotation.get("m11", 1.0), rotation.get("m12", 0.0), rotation.get("m13", 0.0)],
                [rotation.get("m21", 0.0), rotation.get("m22", 1.0), rotation.get("m23", 0.0)],
                [rotation.get("m31", 0.0), rotation.get("m32", 0.0), rotation.get("m33", 1.0)],
            ],
            dtype=np.float64,
        )
    if isinstance(rotation, (list, tuple)) and len(rotation) == 3:
        return np.array(
            [[float(v) for v in row] for row in rotation],
            dtype=np.float64,
        )
    return np.identity(3, dtype=np.float64)


def _block_local_transform(block) -> np.ndarray:
    translation = block.get_field("Translation") or {}
    rotation = _rotation_field_to_3x3(block.get_field("Rotation"))
    scale = float(block.get_field("Scale") or 1.0)

    matrix = np.eye(4, dtype=np.float32)
    matrix[:3, :3] = (rotation.T * scale).astype(np.float32)
    matrix[0, 3] = float(translation.get("x", 0.0))
    matrix[1, 3] = float(translation.get("y", 0.0))
    matrix[2, 3] = float(translation.get("z", 0.0))
    return matrix


def _transform_vertices(vertices: np.ndarray, transform: np.ndarray) -> np.ndarray:
    if np.allclose(transform, np.eye(4, dtype=np.float32)):
        return vertices
    hom = np.ones((len(vertices), 4), dtype=np.float32)
    hom[:, :3] = vertices
    return (transform @ hom.T).T[:, :3].astype(np.float32)


def _connected_vertex_components(
    vertex_count: int,
    triangles: np.ndarray | None,
) -> list[np.ndarray]:
    """Build connected vertex components from triangle adjacency."""
    if vertex_count <= 0:
        return []
    if triangles is None or len(triangles) == 0:
        return [np.arange(vertex_count, dtype=np.int32)]

    adjacency: list[set[int]] = [set() for _ in range(vertex_count)]
    for tri in triangles:
        if len(tri) < 3:
            continue
        v1, v2, v3 = int(tri[0]), int(tri[1]), int(tri[2])
        if min(v1, v2, v3) < 0 or max(v1, v2, v3) >= vertex_count:
            continue
        adjacency[v1].update((v2, v3))
        adjacency[v2].update((v1, v3))
        adjacency[v3].update((v1, v2))

    components: list[np.ndarray] = []
    visited: set[int] = set()
    for start in range(vertex_count):
        if start in visited:
            continue
        stack = [start]
        component: list[int] = []
        while stack:
            current = stack.pop()
            if current in visited:
                continue
            visited.add(current)
            component.append(current)
            stack.extend(adjacency[current] - visited)
        components.append(np.array(component, dtype=np.int32))
    return components


def _submesh_vertices_for_components(
    verts_nif: np.ndarray,
    triangles: np.ndarray | None,
) -> list[np.ndarray]:
    """Split a mesh into per-component vertex clouds."""
    components = _connected_vertex_components(len(verts_nif), triangles)
    return [verts_nif[component] for component in components if len(component) > 0]


def _create_convex_shape(nif, verts_nif: np.ndarray, radius: float = DEFAULT_RADIUS,
                         havok_scale_factor: float = HAVOK_SCALE) -> int | None:
    """Create bhkConvexVerticesShape from NIF-space vertices.

    Args:
        havok_scale_factor: NIF-to-Havok scale (default HAVOK_SCALE).
            Pass GameProfile.havok_scale for multi-game support.

    Returns block_id of the new shape, or None on failure.
    """
    from creation_lib._native.havok_native import convex_hull_simple

    try:
        hull_verts, hull_planes = convex_hull_simple(
            [[float(v[0]), float(v[1]), float(v[2])] for v in verts_nif]
        )
    except ValueError:
        return None

    new_block = nif.add_block("bhkConvexVerticesShape")
    if not new_block:
        return None

    havok_scale = 1.0 / havok_scale_factor

    new_block.set_field("Num Vertices", len(hull_verts))
    new_block.set_field("Vertices", [
        {"x": v[0] * havok_scale, "y": v[1] * havok_scale, "z": v[2] * havok_scale, "w": 0.0}
        for v in hull_verts
    ])
    new_block.set_field("Num Normals", len(hull_planes))
    new_block.set_field("Normals", [
        {"x": p[0], "y": p[1], "z": p[2], "w": p[3] * havok_scale}
        for p in hull_planes
    ])
    new_block.set_field("Radius", radius)
    return new_block.block_id


def _create_box_shape(nif, verts_nif: np.ndarray, radius: float = DEFAULT_RADIUS,
                      havok_scale_factor: float = HAVOK_SCALE) -> tuple[int, int] | None:
    """Create bhkBoxShape wrapped in bhkTransformShape from AABB of vertices.

    Args:
        havok_scale_factor: NIF-to-Havok scale (default HAVOK_SCALE).

    Returns (transform_block_id, box_block_id) or None on failure.
    """
    if len(verts_nif) == 0:
        return None

    havok_scale = 1.0 / havok_scale_factor

    mins = verts_nif.min(axis=0)
    maxs = verts_nif.max(axis=0)
    center = (mins + maxs) / 2.0
    half_extents = (maxs - mins) / 2.0

    # Create bhkBoxShape
    box_block = nif.add_block("bhkBoxShape")
    if not box_block:
        return None

    box_block.set_field("Dimensions", {
        "x": float(half_extents[0] * havok_scale),
        "y": float(half_extents[1] * havok_scale),
        "z": float(half_extents[2] * havok_scale),
    })
    box_block.set_field("Radius", radius)

    # Create bhkTransformShape to position the box at the AABB center
    transform_block = nif.add_block("bhkTransformShape")
    if not transform_block:
        return None

    center_havok = center * havok_scale
    transform_block.set_field("Shape", box_block.block_id)
    transform_block.set_field("Transform", {
        "m11": 1.0, "m12": 0.0, "m13": 0.0, "m14": float(center_havok[0]),
        "m21": 0.0, "m22": 1.0, "m23": 0.0, "m24": float(center_havok[1]),
        "m31": 0.0, "m32": 0.0, "m33": 1.0, "m34": float(center_havok[2]),
        "m41": 0.0, "m42": 0.0, "m43": 0.0, "m44": 1.0,
    })
    transform_block.set_field("Radius", radius)

    return transform_block.block_id, box_block.block_id


def _create_list_shape(nif, shape_ids: list[int]) -> int | None:
    """Create bhkListShape wrapping multiple child shapes.

    Returns block_id of the list shape, or None on failure.
    """
    if not shape_ids:
        return None

    list_block = nif.add_block("bhkListShape")
    if not list_block:
        return None

    list_block.set_field("Num Sub Shapes", len(shape_ids))
    list_block.set_field("Sub Shapes", shape_ids)
    return list_block.block_id


def _build_hierarchy(nif, parent_node_id: int, shape_id: int,
                     layer: str = "STATIC", mass: float = 0.0,
                     friction: float = 0.5, restitution: float = 0.4,
                     layers: dict[str, int] | None = None) -> tuple[int, int] | None:
    """Build full collision hierarchy: bhkCollisionObject → bhkRigidBody → shape.

    Wires the collision object to the parent node and returns
    (collision_object_id, rigid_body_id) or None on failure.
    """
    parent = nif.get_block(parent_node_id)
    if not parent:
        return None

    if layers is None:
        layers = FO4_LAYERS
    layer_value = layers.get(layer, layers["STATIC"])
    layer_key = "Layer"

    # Create bhkRigidBody
    rigid_body = nif.add_block("bhkRigidBody")
    if not rigid_body:
        return None

    rigid_body.set_field("Shape", shape_id)
    rigid_body.set_field(
        "Havok Filter",
        {layer_key: layer_value, "Flags": 0, "Group": 0},
    )
    rigid_body_info = dict(rigid_body.get_field("Rigid Body Info") or {})
    rigid_body_info["Havok Filter"] = {
        layer_key: layer_value,
        "Flags": 0,
        "Group": 0,
    }
    rigid_body_info["Mass"] = mass
    rigid_body_info["Friction"] = friction
    rigid_body_info["Restitution"] = restitution
    # Motion system: fixed for static, dynamic for clutter/weapon
    if mass > 0:
        rigid_body_info["Motion System"] = 1  # MO_SYS_DYNAMIC
        rigid_body_info["Quality Type"] = 1   # MO_QUAL_MOVING
    else:
        rigid_body_info["Motion System"] = 7  # MO_SYS_FIXED
        rigid_body_info["Quality Type"] = 1   # MO_QUAL_FIXED
    rigid_body.set_field("Rigid Body Info", rigid_body_info)

    # Create bhkCollisionObject
    coll_obj = nif.add_block("bhkCollisionObject")
    if not coll_obj:
        return None

    coll_obj.set_field("Flags", 0x81)  # ACTIVE | USE_ABV
    coll_obj.set_field("Target", parent_node_id)
    coll_obj.set_field("Body", rigid_body.block_id)

    # Wire collision object to parent node
    parent.set_field("Collision Object", coll_obj.block_id)

    return coll_obj.block_id, rigid_body.block_id


def _find_parent_node_id(nif, child_id: int) -> int | None:
    for block in nif.blocks:
        if block.type_name not in _NODE_TYPES:
            continue
        children = block.get_field("Children") or []
        if isinstance(children, list) and child_id in children:
            return block.block_id
    return None


def _find_scene_root_id(nif, node_block_id: int) -> int:
    current = node_block_id
    seen: set[int] = set()
    while current not in seen:
        seen.add(current)
        parent_id = _find_parent_node_id(nif, current)
        if parent_id is None:
            return current
        current = parent_id
    return 0 if nif.get_block(0) is not None else node_block_id


def _ensure_root_havok_bsx_flag(nif, node_block_id: int) -> None:
    root = nif.get_block(_find_scene_root_id(nif, node_block_id))
    if root is None:
        return

    extra_ids = list(root.get_field("Extra Data List") or [])
    for extra_id in extra_ids:
        extra = nif.get_block(extra_id)
        if extra is not None and extra.type_name == "BSXFlags":
            extra.set_field(
                "Integer Data",
                int(extra.get_field("Integer Data") or 0) | _BSX_HAVOK_MASK,
            )
            return

    bsx = nif.add_block(
        "BSXFlags",
        {"Name": "BSX", "Integer Data": _BSX_HAVOK_MASK},
    )
    extra_ids.append(bsx.block_id)
    root.set_field("Extra Data List", extra_ids)
    root.set_field("Num Extra Data List", len(extra_ids))


def _next_np_collision_body_id(nif) -> int:
    body_ids = [
        int(body_id)
        for block in nif.blocks
        if block.type_name == "bhkNPCollisionObject"
        for body_id in [block.get_field("Body ID")]
        if isinstance(body_id, int) and body_id >= 0
    ]
    return max(body_ids, default=-1) + 1


def _np_binary_blob(nif, physics_system_id: int) -> bytes | None:
    block = nif.get_block(physics_system_id)
    if block is None or block.type_name != "bhkPhysicsSystem":
        return None
    binary = block.get_field("Binary Data")
    raw = binary.get("Data") if isinstance(binary, dict) else None
    if isinstance(raw, (bytes, bytearray)):
        return bytes(raw)
    if isinstance(raw, list) and raw:
        return bytes(raw)
    return None


def _fo4_body_material_crc_from_blob(blob: bytes, body_id: int) -> int | None:
    try:
        from creation_lib._native import havok_native

        summary = json.loads(havok_native.havok_collision_summary(blob))
    except Exception:
        return None
    for body in summary.get("bodies") or []:
        try:
            if int(body.get("body_id")) != int(body_id):
                continue
        except (TypeError, ValueError):
            continue
        material_crc = body.get("material_crc")
        if material_crc is None:
            return None
        try:
            return int(material_crc)
        except (TypeError, ValueError):
            return None
    return None


def _fo4_body_layer_from_blob(blob: bytes, body_id: int) -> int | None:
    """Recover the collision-layer byte (low byte of collisionFilterInfo) for
    a body in an existing FO4 packfile blob. Returned so a preserved body
    keeps its original layer (e.g. ANIMSTATIC) when we regenerate the merged
    bhkPhysicsSystem after adding a new body."""
    try:
        from creation_lib._native import havok_native

        summary = json.loads(havok_native.havok_collision_summary(blob))
    except Exception:
        return None
    for body in summary.get("bodies") or []:
        try:
            if int(body.get("body_id")) != int(body_id):
                continue
        except (TypeError, ValueError):
            continue
        layer = body.get("layer")
        try:
            return int(layer) if layer is not None else None
        except (TypeError, ValueError):
            return None
    return None


def _matrix_to_quaternion_xyzw(rot: np.ndarray) -> list[float]:
    """Convert a 3×3 rotation matrix to an XYZW quaternion using Shepperd's method.

    Input must be ``v_world = rot @ v_local``-style (the matrix stored in
    Havok bodyCinfo.orientation / NIF Rotation after transposing m_ij).
    """
    m = np.asarray(rot, dtype=np.float64)
    trace = float(m[0, 0] + m[1, 1] + m[2, 2])
    if trace > 0.0:
        s = math.sqrt(trace + 1.0) * 2.0
        w = 0.25 * s
        x = (m[2, 1] - m[1, 2]) / s
        y = (m[0, 2] - m[2, 0]) / s
        z = (m[1, 0] - m[0, 1]) / s
    elif m[0, 0] > m[1, 1] and m[0, 0] > m[2, 2]:
        s = math.sqrt(1.0 + m[0, 0] - m[1, 1] - m[2, 2]) * 2.0
        w = (m[2, 1] - m[1, 2]) / s
        x = 0.25 * s
        y = (m[0, 1] + m[1, 0]) / s
        z = (m[0, 2] + m[2, 0]) / s
    elif m[1, 1] > m[2, 2]:
        s = math.sqrt(1.0 + m[1, 1] - m[0, 0] - m[2, 2]) * 2.0
        w = (m[0, 2] - m[2, 0]) / s
        x = (m[0, 1] + m[1, 0]) / s
        y = 0.25 * s
        z = (m[1, 2] + m[2, 1]) / s
    else:
        s = math.sqrt(1.0 + m[2, 2] - m[0, 0] - m[1, 1]) * 2.0
        w = (m[1, 0] - m[0, 1]) / s
        x = (m[0, 2] + m[2, 0]) / s
        y = (m[1, 2] + m[2, 1]) / s
        z = 0.25 * s
    return [float(x), float(y), float(z), float(w)]


def _node_local_matrix(block) -> np.ndarray:
    """Build a 4×4 local-to-parent transform from a NiNode block.

    NIF stores Rotation as m_ij in column-major convention (Havok-compatible);
    transposing the m_ij matrix yields the rotation that maps local→world.
    """
    trans = block.get_field("Translation") or {}
    scale = float(block.get_field("Scale") or 1.0)
    r = _rotation_field_to_3x3(block.get_field("Rotation")).T  # NIF m_ij is column-major
    t = np.array(
        [trans.get("x", 0.0), trans.get("y", 0.0), trans.get("z", 0.0)],
        dtype=np.float64,
    )
    m = np.identity(4, dtype=np.float64)
    m[:3, :3] = r * scale
    m[:3, 3] = t
    return m


def _node_world_transform_havok(
    nif, node_block_id: int, havok_scale: float
) -> tuple[list[float], list[float]]:
    """Walk root→node, composing local transforms; return position in Havok units
    (NIF translation / havok_scale) and orientation quaternion XYZW.

    Position written into hknpBodyCinfo.position; orientation into
    hknpBodyCinfo.orientation. Both replicate into the matching motionCinfo's
    centerOfMassWorld + orientation when the body is Keyframed.
    """
    parent_of: dict[int, int] = {}
    for block in nif.blocks:
        if block.type_name not in _NODE_TYPES:
            continue
        for child in block.get_field("Children") or []:
            if isinstance(child, int) and child >= 0:
                parent_of[child] = block.block_id

    chain: list[int] = []
    cur: int | None = node_block_id
    seen: set[int] = set()
    while cur is not None and cur not in seen:
        seen.add(cur)
        chain.append(cur)
        cur = parent_of.get(cur)
    chain.reverse()  # root first

    world = np.identity(4, dtype=np.float64)
    for bid in chain:
        block = nif.get_block(bid)
        if block is None:
            continue
        world = world @ _node_local_matrix(block)

    pos = world[:3, 3] / float(havok_scale)
    quat = _matrix_to_quaternion_xyzw(world[:3, :3])
    return [float(pos[0]), float(pos[1]), float(pos[2])], quat


def _fo4_body_spec_from_preview(record: dict) -> tuple[str, list, list | None, None]:
    from creation_lib.havok.native_runtime import collision_preview_native

    preview = collision_preview_native(
        record["blob"],
        havok_scale=1.0,
        body_id=int(record["body_id"]),
    )
    meshes = list(preview.get("meshes") or [])
    if not meshes:
        raise ValueError(f"no preview geometry for body {record['body_id']}")

    if len(meshes) == 1 and meshes[0].get("shape_type") == "compressed_mesh":
        mesh = meshes[0].get("mesh") or {}
        vertices = [
            [float(v.get("x", 0.0)), float(v.get("y", 0.0)), float(v.get("z", 0.0))]
            for v in (mesh.get("vertices") or [])
        ]
        triangles = [
            [int(t.get("v1", 0)), int(t.get("v2", 0)), int(t.get("v3", 0))]
            for t in (mesh.get("triangles") or [])
        ]
        return ("compressed_mesh", vertices, triangles, None)

    if len(meshes) > 1:
        children = []
        for mesh_info in meshes:
            mesh = mesh_info.get("mesh") or {}
            child_vertices = [
                [float(v.get("x", 0.0)), float(v.get("y", 0.0)), float(v.get("z", 0.0))]
                for v in (mesh.get("vertices") or [])
            ]
            if len(child_vertices) >= 3:
                children.append(("polytope", child_vertices, None))
        if children:
            return ("compound", [], None, children)

    vertices: list[list[float]] = []
    for mesh_info in meshes:
        mesh = mesh_info.get("mesh") or {}
        vertices.extend(
            [
                [float(v.get("x", 0.0)), float(v.get("y", 0.0)), float(v.get("z", 0.0))]
                for v in (mesh.get("vertices") or [])
            ]
        )
    if len(vertices) < 3:
        raise ValueError(f"body {record['body_id']} preview has no usable vertices")
    return ("polytope", vertices, None, None)


def _aabb_box_vertices(vertices: np.ndarray) -> np.ndarray:
    minimum = np.min(vertices, axis=0)
    maximum = np.max(vertices, axis=0)
    return np.asarray(
        [
            [x, y, z]
            for x in (minimum[0], maximum[0])
            for y in (minimum[1], maximum[1])
            for z in (minimum[2], maximum[2])
        ],
        dtype=vertices.dtype,
    )


def _fo4_body_spec_from_node_geometry(
    nif,
    node_block_id: int,
    shape_type: str,
    source_block_ids: list[int] | None,
    include_child_nodes: bool,
    profile,
    layer: str = "STATIC",
) -> tuple[tuple[str, list, list | None, None], list[str]]:
    source_block_ids, all_verts, component_verts = _collect_source_vertices(
        nif,
        node_block_id,
        source_block_ids,
        include_child_nodes=include_child_nodes,
    )
    if not all_verts:
        raise ValueError("No valid vertex data found in source shapes")

    havok_scale = _profile_havok_scale(profile)
    combined_verts = np.vstack(all_verts)
    warnings: list[str] = []
    # Vanilla parity: a single-component STATIC convex request becomes a
    # compressed mesh (see _coerce_static_convex_to_compressed_mesh). Gated on
    # triangles being present so a vertex-only convex request stays convex, and
    # on single-component so multi-component keeps its one-hull-per-component
    # compound path below.
    tris = _collect_all_triangles(nif, source_block_ids)
    shape_type = _coerce_static_convex_to_compressed_mesh(
        shape_type, layer, eligible=len(component_verts) <= 1 and len(tris) > 0
    )
    mesh_types = {"compressed_mesh", "mesh", "auto_compressed_mesh"}
    if shape_type in mesh_types:
        if len(tris) == 0:
            raise ValueError("No triangle data found for mesh collision")
        verts_for_mesh = combined_verts
        tris_for_mesh = tris
        if shape_type == "auto_compressed_mesh":
            verts_for_mesh, tris_for_mesh, simplified = _simplify_mesh_for_compressed_collision(
                verts_for_mesh,
                tris_for_mesh,
            )
            if simplified:
                warnings.append(
                    "simplified compressed mesh collision to fit FO4 hknpCompressedMeshShape limits"
                )
        return (
            (
                "compressed_mesh",
                (verts_for_mesh / havok_scale).tolist(),
                tris_for_mesh.tolist(),
                None,
            ),
            warnings,
        )

    if shape_type == "convex_fit":
        verts = _simplify_convex_fit_vertices(combined_verts)
    elif shape_type == "box":
        verts = _aabb_box_vertices(combined_verts)
    else:
        verts = combined_verts
    if shape_type in {"convex_hull", "convex_fit", "list", "auto"} and len(component_verts) > 1:
        children = []
        for component in component_verts:
            child_verts = (
                _simplify_convex_fit_vertices(component)
                if shape_type == "convex_fit"
                else component
            )
            children.append(("polytope", (child_verts / havok_scale).tolist(), None))
        return (("compound", [], None, children), warnings)
    return (("polytope", (verts / havok_scale).tolist(), None, None), warnings)


def _fo4_body_spec_from_explicit_geometry(
    vertices: np.ndarray,
    triangles: np.ndarray | None,
    shape_type: str,
    profile,
) -> tuple[tuple[str, list, list | None, None], list[str]]:
    havok_scale = _profile_havok_scale(profile)
    mesh_types = {"compressed_mesh", "mesh", "auto_compressed_mesh"}
    warnings: list[str] = []
    if shape_type in mesh_types:
        if triangles is None or len(triangles) == 0:
            raise ValueError("Mesh collision requires triangle data")
        verts_for_mesh = vertices
        tris_for_mesh = triangles
        if shape_type == "auto_compressed_mesh":
            verts_for_mesh, tris_for_mesh, simplified = _simplify_mesh_for_compressed_collision(
                verts_for_mesh,
                tris_for_mesh,
            )
            if simplified:
                warnings.append(
                    "simplified compressed mesh collision to fit FO4 hknpCompressedMeshShape limits"
                )
        return (
            (
                "compressed_mesh",
                (verts_for_mesh / havok_scale).tolist(),
                tris_for_mesh.tolist(),
                None,
            ),
            warnings,
        )

    if shape_type == "convex_fit":
        verts = _simplify_convex_fit_vertices(vertices)
    elif shape_type == "box":
        verts = _aabb_box_vertices(vertices)
    else:
        verts = vertices
    return (("polytope", (verts / havok_scale).tolist(), None, None), warnings)


def _install_fo4_multi_body_collision(
    nif,
    node_block_id: int,
    new_body_spec: tuple[str, list, list | None, None],
    material_crc: int | None,
    layer: str,
    mass: float,
    friction: float,
    restitution: float,
    replace: bool,
    profile,
) -> OperationResult:
    from creation_lib._native.havok_native import (
        fo4_multi_body_collision_blob as _native_fo4_multi_body,
    )

    node = nif.get_block(node_block_id)
    if node is None:
        return OperationResult(False, f"Node block {node_block_id} not found")

    existing_target_coll_id = _find_collision_object(nif, node_block_id)
    if existing_target_coll_id is not None and not replace:
        return OperationResult(False, "Collision already exists on target node")

    havok_scale = _profile_havok_scale(profile)
    layers = get_collision_layers(profile)
    layer_value = layers.get(layer, layers["STATIC"])
    # ANIMSTATIC (and KEYFRAMED-ish layers) need a populated motionCinfos entry
    # — without it the FO4 broadphase null-derefs in workshop placement sweeps.
    animstatic_layer = layers.get("ANIMSTATIC", 2)

    def _motion_type_for_layer(value: int) -> str:
        return "keyframed" if int(value) == int(animstatic_layer) else "static"

    preserved: list[
        tuple[int, tuple[str, list, list | None, None], int | None, int, int]
    ] = []
    old_physics_ids: set[int] = set()
    warnings: list[str] = []
    for block in list(nif.blocks):
        if block.type_name != "bhkNPCollisionObject":
            continue
        if block.block_id == existing_target_coll_id:
            continue
        data_id = block.get_field("Data")
        body_id = block.get_field("Body ID")
        target_id = block.get_field("Target")
        if not isinstance(data_id, int) or data_id < 0:
            continue
        if not isinstance(body_id, int) or body_id < 0:
            continue
        blob = _np_binary_blob(nif, data_id)
        if blob is None:
            continue
        record = {
            "collision_id": block.block_id,
            "target_id": target_id,
            "physics_id": data_id,
            "body_id": body_id,
            "blob": blob,
        }
        preserved_material_crc = _fo4_body_material_crc_from_blob(blob, body_id)
        preserved_layer = _fo4_body_layer_from_blob(blob, body_id)
        if preserved_layer is None:
            preserved_layer = layers["STATIC"]
        try:
            body_spec = _fo4_body_spec_from_preview(record)
        except Exception as exc:
            if not isinstance(target_id, int) or target_id < 0:
                warnings.append(
                    f"Skipped existing collision on block {block.block_id}: {exc}"
                )
                continue
            try:
                body_spec, fallback_warnings = _fo4_body_spec_from_node_geometry(
                    nif,
                    target_id,
                    "convex_fit",
                    None,
                    True,
                    profile,
                    layer=next(
                        (n for n, v in layers.items() if v == int(preserved_layer)),
                        "STATIC",
                    ),
                )
                warnings.extend(fallback_warnings)
                warnings.append(
                    f"Rebuilt existing collision on block {block.block_id} from target geometry because preview failed: {exc}"
                )
            except Exception as fallback_exc:
                warnings.append(
                    f"Skipped existing collision on block {block.block_id}: {fallback_exc}"
                )
                continue
        preserved.append(
            (
                block.block_id,
                body_spec,
                preserved_material_crc,
                int(preserved_layer),
                int(target_id) if isinstance(target_id, int) else node_block_id,
            )
        )
        old_physics_ids.add(data_id)

    # Per-body metadata: layer + world transform + motion type. The new body
    # uses the caller-supplied layer; preserved bodies keep the layer recorded
    # in their original blob so an existing ANIMSTATIC door isn't downgraded
    # to STATIC during a regeneration.
    body_entries: list[dict[str, object]] = []
    for _coll_id, _spec, _mat, b_layer, target in preserved:
        pos, quat = _node_world_transform_havok(nif, target, havok_scale)
        body_entries.append(
            {
                "coll_id": _coll_id,
                "spec": _spec,
                "material_crc": _mat,
                "meta": (int(b_layer) & 0xFF, pos, quat, _motion_type_for_layer(b_layer)),
            }
        )
    pos, quat = _node_world_transform_havok(nif, node_block_id, havok_scale)
    body_entries.append(
        {
            "coll_id": None,
            "spec": new_body_spec,
            "material_crc": material_crc,
            "meta": (
                int(layer_value) & 0xFF,
                pos,
                quat,
                _motion_type_for_layer(layer_value),
            ),
        }
    )

    # Keep the vanilla body ordering: compressed mesh bodies first, then
    # polytopes/compounds. Fallout4.esm CM* assets include shared multi-CM
    # systems, so repeated compressed mesh bodies must remain compressed.
    def _shape_sort_key(spec) -> int:
        kind = spec[0] if spec else ""
        return 0 if kind in ("compressed_mesh", "mesh") else 1

    body_entries.sort(key=lambda entry: _shape_sort_key(entry["spec"]))
    body_specs = [entry["spec"] for entry in body_entries]
    material_crcs = [entry["material_crc"] for entry in body_entries]
    body_metas = [entry["meta"] for entry in body_entries]

    try:
        kwargs: dict = {"body_metas": body_metas}
        if any(mat is not None for mat in material_crcs):
            blob = _native_fo4_multi_body(
                body_specs,
                float(friction),
                float(restitution),
                int(layer_value),
                float(mass),
                material_crcs,
                **kwargs,
            )
        else:
            blob = _native_fo4_multi_body(
                body_specs,
                float(friction),
                float(restitution),
                int(layer_value),
                float(mass),
                None,
                **kwargs,
            )
    except (ValueError, RuntimeError) as e:
        return OperationResult(False, f"Collision generation failed: {e}", warnings=warnings)

    if existing_target_coll_id is not None:
        existing_target_coll = nif.get_block(existing_target_coll_id)
        old_data = existing_target_coll.get_field("Data") if existing_target_coll else None
        if isinstance(old_data, int) and old_data >= 0:
            old_physics_ids.add(old_data)

    phys_sys = nif.add_block("bhkPhysicsSystem")
    if not phys_sys:
        return OperationResult(False, "Failed to create bhkPhysicsSystem block", warnings=warnings)
    phys_sys.set_field("Binary Data", {"Data Size": len(blob), "Data": list(blob)})

    if existing_target_coll_id is not None:
        coll_obj = nif.get_block(existing_target_coll_id)
    else:
        coll_obj = nif.add_block("bhkNPCollisionObject")
        if not coll_obj:
            return OperationResult(
                False,
                "Failed to create bhkNPCollisionObject block",
                warnings=warnings,
            )
    coll_obj.set_field("Flags", 0x80)
    coll_obj.set_field("Target", node_block_id)
    node.set_field("Collision Object", coll_obj.block_id)
    for entry in body_entries:
        if entry["coll_id"] is None:
            entry["coll_id"] = coll_obj.block_id

    for body_id, entry in enumerate(body_entries):
        block = nif.get_block(int(entry["coll_id"]))
        if block is None:
            continue
        block.set_field("Data", phys_sys.block_id)
        block.set_field("Body ID", body_id)

    _ensure_root_havok_bsx_flag(nif, node_block_id)

    old_physics_ids.discard(phys_sys.block_id)
    if old_physics_ids:
        nif.remove_blocks(sorted(old_physics_ids))

    return OperationResult(
        True,
        f"Generated FO4 multi-body collision ({len(body_specs)} bodies, layer={layer}, blob={len(blob)} bytes)",
        [phys_sys.block_id, *(int(entry["coll_id"]) for entry in body_entries)],
        warnings=warnings,
    )


def _find_collision_object(nif, node_block_id: int) -> int | None:
    """Find the bhkCollisionObject attached to a node, if any."""
    node = nif.get_block(node_block_id)
    if not node:
        return None
    coll_ref = node.get_field("Collision Object")
    if coll_ref is None or (isinstance(coll_ref, int) and coll_ref < 0):
        return None
    coll_id = coll_ref if isinstance(coll_ref, int) else int(coll_ref)
    block = nif.get_block(coll_id)
    if block and block.type_name in ("bhkCollisionObject", "bhkNPCollisionObject"):
        return coll_id
    return None


def _collect_collision_subtree(nif, coll_obj_id: int) -> list[int]:
    """Collect all block IDs in the collision subtree rooted at a collision object.

    Walks known Havok Ref fields (Body, Shape, Sub Shapes) instead of relying
    on schema get_refs(), so this works with stubbed NIFs in tests.
    """
    ids = []
    visited = set()
    # Known Ref field names in Havok collision blocks.
    # Real FO4 NIFs (loaded via NifFile.load) use:
    #   bhkNPCollisionObject: "Data" -> bhkPhysicsSystem block ID
    #   bhkCollisionObject: "Body" -> bhkRigidBody block ID
    # "Body ID" in bhkNPCollisionObject is a body-array index (not a block ref).
    _REF_FIELDS = ("Body", "Data", "Shape", "Sub Shapes")

    def _walk(bid: int):
        if bid < 0 or bid in visited:
            return
        block = nif.get_block(bid)
        if not block:
            return
        visited.add(bid)
        ids.append(bid)
        for fname in _REF_FIELDS:
            val = block.get_field(fname)
            if val is None:
                continue
            if isinstance(val, int) and val >= 0:
                _walk(val)
            elif isinstance(val, list):
                for item in val:
                    if isinstance(item, int) and item >= 0:
                        _walk(item)

    _walk(coll_obj_id)
    return ids


def _auto_discover_shape_sources(
    nif,
    node_block_id: int,
    include_child_nodes: bool = True,
) -> list[tuple[int, np.ndarray]]:
    """Find render shapes under the selected node with transforms relative to it."""
    node = nif.get_block(node_block_id)
    if not node:
        return []
    if node.type_name in _SHAPE_TYPES:
        return [(node_block_id, np.eye(4, dtype=np.float32))]

    shape_sources: list[tuple[int, np.ndarray]] = []
    visited: set[int] = set()

    def walk(block_id: int, parent_transform: np.ndarray) -> None:
        if block_id in visited:
            return
        visited.add(block_id)
        block = nif.get_block(block_id)
        if not block:
            return

        transform = parent_transform @ _block_local_transform(block)
        if block.type_name in _SHAPE_TYPES:
            shape_sources.append((block_id, transform))
            return
        if block.type_name not in _NODE_TYPES:
            return

        for ref in block.get_field("Children") or []:
            child_id = _ref_to_block_id(ref)
            if child_id >= 0:
                walk(child_id, transform)

    for ref in node.get_field("Children") or []:
        child_id = _ref_to_block_id(ref)
        child = nif.get_block(child_id) if child_id >= 0 else None
        if (
            child
            and not include_child_nodes
            and child.type_name in _NODE_TYPES
        ):
            continue
        if child_id >= 0:
            walk(child_id, np.eye(4, dtype=np.float32))

    return shape_sources


def _auto_discover_shapes(
    nif,
    node_block_id: int,
    include_child_nodes: bool = True,
) -> list[int]:
    return [
        sid
        for sid, _ in _auto_discover_shape_sources(
            nif,
            node_block_id,
            include_child_nodes=include_child_nodes,
        )
    ]


def _collect_source_vertices(
    nif,
    node_block_id: int,
    source_block_ids: list[int] | None,
    include_child_nodes: bool = True,
) -> tuple[list[int], list[np.ndarray], list[np.ndarray]]:
    if source_block_ids is None:
        shape_sources = _auto_discover_shape_sources(
            nif,
            node_block_id,
            include_child_nodes=include_child_nodes,
        )
    else:
        identity = np.eye(4, dtype=np.float32)
        discovered_sources = dict(_auto_discover_shape_sources(nif, node_block_id))
        shape_sources = [
            (sid, discovered_sources.get(sid, identity))
            for sid in source_block_ids
        ]

    resolved_source_ids: list[int] = []
    all_verts: list[np.ndarray] = []
    per_shape_verts: list[np.ndarray] = []
    for sid, transform in shape_sources:
        verts = _extract_vertices(nif, sid)
        if verts is not None and len(verts) >= 3:
            transformed = _transform_vertices(verts, transform)
            resolved_source_ids.append(sid)
            all_verts.append(transformed)
            per_shape_verts.append(transformed)

    return resolved_source_ids, all_verts, per_shape_verts


def _generate_starfield_collision(
    nif,
    node_block_id: int,
    source_block_ids: list[int] | None = None,
    include_child_nodes: bool = True,
    layer: str = "STATIC",
    mass: float = 0.0,
    friction: float = 0.5,
    restitution: float = 0.4,
    radius: float = DEFAULT_RADIUS,
    replace: bool = True,
    profile=None,
) -> OperationResult:
    """Generate Starfield collision using Havok 2019 tagged binary format.

    Creates bhkNPCollisionObject → bhkPhysicsSystem with tagged binary blob.
    """
    from creation_lib._native.havok_native import starfield_convex_collision_blob as _native_build_convex

    auto_sources = source_block_ids is None
    source_block_ids, all_verts, _ = _collect_source_vertices(
        nif,
        node_block_id,
        source_block_ids,
        include_child_nodes=include_child_nodes,
    )
    if not source_block_ids:
        return OperationResult(False, "No BSTriShape children found for collision generation")

    if not all_verts:
        return OperationResult(False, "No valid vertex data found in source shapes")

    combined_verts = np.vstack(all_verts)

    # Handle existing collision
    node = nif.get_block(node_block_id)
    existing_coll = _find_collision_object(nif, node_block_id)
    if existing_coll is not None:
        if replace:
            subtree = _collect_collision_subtree(nif, existing_coll)
            node.set_field("Collision Object", -1)
            nif.remove_blocks(subtree)
            node = nif.get_block(node_block_id)
            source_block_ids, all_verts, _ = _collect_source_vertices(
                nif,
                node_block_id,
                None if auto_sources else source_block_ids,
                include_child_nodes=include_child_nodes,
            )
            if not all_verts:
                return OperationResult(False, "No valid vertex data after removing old collision")
            combined_verts = np.vstack(all_verts)
        else:
            return OperationResult(False, "Merge mode not supported for Starfield collision")

    # Resolve layer value
    layers = get_collision_layers(profile)
    layer_value = layers.get(layer, layers["STATIC"])

    # Build tagged blob — vertices in NIF-space (no Havok scaling for Starfield)
    try:
        blob = _native_build_convex(
            combined_verts.tolist(),
            float(friction),
            float(restitution),
            int(layer_value),
            float(mass),
        )
    except (ValueError, RuntimeError) as e:
        return OperationResult(False, f"Collision generation failed: {e}")

    # Create bhkPhysicsSystem block with binary data
    phys_sys = nif.add_block("bhkPhysicsSystem")
    if not phys_sys:
        return OperationResult(False, "Failed to create bhkPhysicsSystem block")

    phys_sys.set_field("Binary Data", {"Data Size": len(blob), "Data": list(blob)})

    # Create bhkNPCollisionObject
    coll_obj = nif.add_block("bhkNPCollisionObject")
    if not coll_obj:
        return OperationResult(False, "Failed to create bhkNPCollisionObject block")

    coll_obj.set_field("Flags", 0x81)  # ACTIVE | USE_ABV
    coll_obj.set_field("Target", node_block_id)
    # Per nif.xml bhkNPCollisionObject schema: Ref to bhkSystem is "Data"
    # (not "Body" — that name belongs to the legacy bhkCollisionObject path).
    coll_obj.set_field("Data", phys_sys.block_id)

    # Wire collision to parent node
    node.set_field("Collision Object", coll_obj.block_id)

    n_verts = len(combined_verts)
    created_ids = [phys_sys.block_id, coll_obj.block_id]
    return OperationResult(
        True,
        f"Generated Starfield convex collision ({n_verts} vertices, layer={layer}, blob={len(blob)} bytes)",
        created_ids,
    )


def _coerce_static_convex_to_compressed_mesh(
    shape_type: str, layer: str, eligible: bool
) -> str:
    """Vanilla parity: a single FO4 static world-collision body is a concave
    compressed mesh (vanilla ``Safe01.nif`` base, building kits), not a convex
    hull.

    A convex polytope on the STATIC layer inside a multi-body safe/door system
    reproduces the workshop-placement sweep CTD at ``Fallout4.exe+13E82D0``
    (B21_TheBank CONT): the static base must look
    like vanilla's compressed-mesh base. The Max-plugin export defaults
    ``shape_type`` to ``convex_hull`` (see ``bridge._apply_collision_geometry``),
    so a STATIC base authored without an explicit shape lands as a polytope.

    ``eligible`` gates the coercion to the single-body case where source
    triangles are available: a multi-component request keeps its compound /
    one-hull-per-component path, and a triangle-less request stays convex (the
    native builder still emits ``motionId=HK_INVALID`` for static bodies, which
    removes the bad motion linkage on its own).
    """
    if (
        shape_type in {"convex_hull", "convex_fit"}
        and str(layer).upper() == "STATIC"
        and eligible
    ):
        return "auto_compressed_mesh"
    return shape_type


def _generate_fo4_collision(
    nif,
    node_block_id: int,
    shape_type: str = "convex_hull",
    source_block_ids: list[int] | None = None,
    include_child_nodes: bool = True,
    layer: str = "STATIC",
    material: object | None = None,
    mass: float = 0.0,
    friction: float = 0.5,
    restitution: float = 0.4,
    radius: float = DEFAULT_RADIUS,
    replace: bool = True,
    profile=None,
) -> OperationResult:
    """Generate FO4 collision using Havok 2014.1.0 packfile format.

    Creates bhkNPCollisionObject → bhkPhysicsSystem with a packfile blob containing:
      - convex_hull / convex_fit / box / sphere / capsule / cylinder → hknpConvexPolytopeShape
      - list / auto → hknpDynamicCompoundShape with polytope sub-shapes
      - compressed_mesh / mesh → hknpCompressedMeshShape (existing builder)
    """
    from creation_lib._native.havok_native import (
        fo4_polytope_collision_blob as _native_fo4_polytope,
        fo4_compound_collision_blob as _native_fo4_compound,
        fo4_compressed_mesh_collision_blob as _native_fo4_compressed_mesh,
    )

    try:
        material_crc = resolve_collision_material(material, profile)
    except ValueError as exc:
        return OperationResult(False, str(exc))

    if shape_type in _FO4_MULTI_BODY_SHAPE_TYPES:
        try:
            body_spec, body_warnings = _fo4_body_spec_from_node_geometry(
                nif,
                node_block_id,
                shape_type,
                source_block_ids,
                include_child_nodes,
                profile,
                layer=layer,
            )
        except ValueError as e:
            return OperationResult(False, str(e))
        result = _install_fo4_multi_body_collision(
            nif=nif,
            node_block_id=node_block_id,
            new_body_spec=body_spec,
            material_crc=material_crc,
            layer=layer,
            mass=mass,
            friction=friction,
            restitution=restitution,
            replace=replace,
            profile=profile,
        )
        result.warnings = body_warnings + result.warnings
        return result

    auto_sources = source_block_ids is None
    source_block_ids, all_verts, component_verts_list = _collect_source_vertices(
        nif,
        node_block_id,
        source_block_ids,
        include_child_nodes=include_child_nodes,
    )
    if not source_block_ids:
        return OperationResult(False, "No BSTriShape children found for collision generation")

    if not all_verts:
        return OperationResult(False, "No valid vertex data found in source shapes")

    combined_verts = np.vstack(all_verts)

    # Handle existing collision
    node = nif.get_block(node_block_id)
    existing_coll = _find_collision_object(nif, node_block_id)
    if existing_coll is not None:
        if replace:
            subtree = _collect_collision_subtree(nif, existing_coll)
            node.set_field("Collision Object", -1)
            nif.remove_blocks(subtree)
            node = nif.get_block(node_block_id)
            source_block_ids, all_verts, component_verts_list = _collect_source_vertices(
                nif,
                node_block_id,
                None if auto_sources else source_block_ids,
                include_child_nodes=include_child_nodes,
            )
            if not all_verts:
                return OperationResult(False, "No valid vertex data after removing old collision")
            combined_verts = np.vstack(all_verts)
        else:
            return OperationResult(False, "Merge mode not supported for FO4 packfile collision")

    # Resolve collision layer integer
    layers = get_collision_layers(profile)
    layer_value = layers.get(layer, layers["STATIC"])
    havok_scale = _profile_havok_scale(profile)
    combined_verts_havok = combined_verts / havok_scale
    component_verts_havok = [verts / havok_scale for verts in component_verts_list]
    fit_verts_havok = _simplify_convex_fit_vertices(combined_verts) / havok_scale

    # Dispatch on shape_type → packfile builder
    # bhkNPCollisionObject.Body references bhkPhysicsSystem directly
    # (no sub-block Refs; all shape data is inside Binary Data blob)
    _POLYTOPE_TYPES = ("convex_hull", "box", "sphere", "capsule", "cylinder")
    _COMPOUND_TYPES = ("list", "auto")
    _MESH_TYPES = ("compressed_mesh", "mesh")

    try:
        if shape_type == "convex_fit":
            blob = _native_fo4_polytope(
                fit_verts_havok.tolist(),
                float(friction),
                float(restitution),
                int(layer_value),
                float(mass),
                material_crc,
            )
        elif shape_type in _POLYTOPE_TYPES:
            if shape_type == "convex_hull" and len(component_verts_list) > 1:
                eye4 = np.eye(4, dtype=np.float32).flatten().tolist()
                sub_shapes = [
                    (eye4, "polytope", v.tolist(), None)
                    for v in component_verts_havok
                ]
                blob = _native_fo4_compound(
                    sub_shapes,
                    float(friction),
                    float(restitution),
                    int(layer_value),
                    float(mass),
                    material_crc,
                )
            else:
                blob = _native_fo4_polytope(
                    combined_verts_havok.tolist(),
                    float(friction),
                    float(restitution),
                    int(layer_value),
                    float(mass),
                    material_crc,
                )
        elif shape_type in _COMPOUND_TYPES:
            eye4 = np.eye(4, dtype=np.float32).flatten().tolist()
            sub_shapes = [
                (eye4, "polytope", v.tolist(), None)
                for v in component_verts_havok
            ]
            blob = _native_fo4_compound(
                sub_shapes,
                float(friction),
                float(restitution),
                int(layer_value),
                float(mass),
                material_crc,
            )
        elif shape_type in _MESH_TYPES:
            combined_tris = _collect_all_triangles(nif, source_block_ids)
            if len(combined_tris) == 0:
                return OperationResult(False, "No triangle data found for mesh collision")
            blob = _native_fo4_compressed_mesh(
                combined_verts_havok.tolist(),
                combined_tris.tolist(),
                float(friction),
                float(restitution),
                int(layer_value),
                float(mass),
                material_crc,
            )
        else:
            return OperationResult(False, f"Unknown shape_type for FO4: {shape_type!r}")
    except ValueError as e:
        return OperationResult(False, f"Collision generation failed: {e}")

    # Create bhkPhysicsSystem block with binary blob
    phys_sys = nif.add_block("bhkPhysicsSystem")
    if not phys_sys:
        return OperationResult(False, "Failed to create bhkPhysicsSystem block")
    phys_sys.set_field("Binary Data", {"Data Size": len(blob), "Data": list(blob)})

    body_id = _next_np_collision_body_id(nif)

    # Create bhkNPCollisionObject.
    # _find_collision_object already handles both "bhkCollisionObject" and
    # "bhkNPCollisionObject" — intentional, see line 328.
    coll_obj = nif.add_block("bhkNPCollisionObject")
    if not coll_obj:
        return OperationResult(False, "Failed to create bhkNPCollisionObject block")
    coll_obj.set_field("Flags", 0x80)
    coll_obj.set_field("Target", node_block_id)
    coll_obj.set_field("Body ID", body_id)
    # Per nif.xml bhkNPCollisionObject schema: Ref to bhkSystem is "Data"
    # (not "Body" — that name belongs to the legacy bhkCollisionObject path).
    coll_obj.set_field("Data", phys_sys.block_id)

    # Wire to parent node
    node.set_field("Collision Object", coll_obj.block_id)
    _ensure_root_havok_bsx_flag(nif, node_block_id)

    n_verts = len(combined_verts)
    return OperationResult(
        True,
        f"Generated FO4 {shape_type} collision ({n_verts} vertices, layer={layer}, blob={len(blob)} bytes)",
        [phys_sys.block_id, coll_obj.block_id],
    )


# ---- Public API ----


def _generate_fo4_collision_from_geometry(
    nif,
    node_block_id: int,
    vertices: np.ndarray,
    triangles: np.ndarray | None,
    shape_type: str,
    layer: str,
    material: object | None,
    mass: float,
    friction: float,
    restitution: float,
    replace: bool,
    profile,
) -> OperationResult:
    """Geometry-driven FO4 collision generator.

    Companion to ``_generate_fo4_collision`` (which discovers source meshes from
    NIF children) for callers that already have explicit verts/tris — the Max
    plugin, the Maya plugin, batch CLI flows, and any caller with a procedural
    mesh in hand. Same output: bhkNPCollisionObject + bhkPhysicsSystem with a
    Havok 2014.1.0 packfile blob.
    """
    from creation_lib._native.havok_native import (
        fo4_polytope_collision_blob as _native_fo4_polytope,
        fo4_compound_collision_blob as _native_fo4_compound,
        fo4_compressed_mesh_collision_blob as _native_fo4_compressed_mesh,
    )

    if vertices is None or len(vertices) < 3:
        return OperationResult(False, "No valid collision geometry provided")

    try:
        material_crc = resolve_collision_material(material, profile)
    except ValueError as exc:
        return OperationResult(False, str(exc))

    # Replace existing collision if requested
    node = nif.get_block(node_block_id)
    if node is None:
        return OperationResult(False, f"Node block {node_block_id} not found")

    if shape_type in _FO4_MULTI_BODY_SHAPE_TYPES:
        try:
            body_spec, body_warnings = _fo4_body_spec_from_explicit_geometry(
                vertices,
                triangles,
                shape_type,
                profile,
            )
        except ValueError as e:
            return OperationResult(False, str(e))
        result = _install_fo4_multi_body_collision(
            nif=nif,
            node_block_id=node_block_id,
            new_body_spec=body_spec,
            material_crc=material_crc,
            layer=layer,
            mass=mass,
            friction=friction,
            restitution=restitution,
            replace=replace,
            profile=profile,
        )
        result.warnings = body_warnings + result.warnings
        return result

    existing_coll = _find_collision_object(nif, node_block_id)
    if existing_coll is not None:
        if replace:
            subtree = _collect_collision_subtree(nif, existing_coll)
            node.set_field("Collision Object", -1)
            nif.remove_blocks(subtree)
            node = nif.get_block(node_block_id)
        else:
            return OperationResult(False, "Merge mode not supported for FO4 packfile collision")

    layers = get_collision_layers(profile)
    layer_value = layers.get(layer, layers["STATIC"])
    havok_scale = _profile_havok_scale(profile)
    vertices_havok = vertices / havok_scale
    fit_vertices_havok = _simplify_convex_fit_vertices(vertices) / havok_scale

    _POLYTOPE_TYPES = ("convex_hull", "box", "sphere", "capsule", "cylinder")
    _COMPOUND_TYPES = ("list", "auto")
    _MESH_TYPES = ("compressed_mesh", "mesh")

    try:
        if shape_type == "convex_fit":
            blob = _native_fo4_polytope(
                fit_vertices_havok.tolist(),
                float(friction),
                float(restitution),
                int(layer_value),
                float(mass),
                material_crc,
            )
        elif shape_type in _POLYTOPE_TYPES:
            blob = _native_fo4_polytope(
                vertices_havok.tolist(),
                float(friction),
                float(restitution),
                int(layer_value),
                float(mass),
                material_crc,
            )
        elif shape_type in _COMPOUND_TYPES:
            # No connected-component split when we receive raw geometry — caller
            # is expected to invoke once per component if a true compound is wanted.
            # Single sub-shape → still wraps in DynamicCompoundShape per vanilla.
            eye4 = np.eye(4, dtype=np.float32).flatten().tolist()
            blob = _native_fo4_compound(
                [(eye4, "polytope", vertices_havok.tolist(), None)],
                float(friction),
                float(restitution),
                int(layer_value),
                float(mass),
                material_crc,
            )
        elif shape_type in _MESH_TYPES:
            if triangles is None or len(triangles) == 0:
                return OperationResult(False, "Mesh collision requires triangle data")
            blob = _native_fo4_compressed_mesh(
                vertices_havok.tolist(),
                triangles.tolist() if hasattr(triangles, 'tolist') else list(triangles),
                float(friction),
                float(restitution),
                int(layer_value),
                float(mass),
                material_crc,
            )
        else:
            return OperationResult(False, f"Unknown shape_type for FO4: {shape_type!r}")
    except ValueError as e:
        return OperationResult(False, f"Collision generation failed: {e}")

    phys_sys = nif.add_block("bhkPhysicsSystem")
    if not phys_sys:
        return OperationResult(False, "Failed to create bhkPhysicsSystem block")
    phys_sys.set_field("Binary Data", {"Data Size": len(blob), "Data": list(blob)})

    body_id = _next_np_collision_body_id(nif)
    coll_obj = nif.add_block("bhkNPCollisionObject")
    if not coll_obj:
        return OperationResult(False, "Failed to create bhkNPCollisionObject block")
    coll_obj.set_field("Flags", 0x80)
    coll_obj.set_field("Target", node_block_id)
    coll_obj.set_field("Body ID", body_id)
    coll_obj.set_field("Data", phys_sys.block_id)
    node.set_field("Collision Object", coll_obj.block_id)
    _ensure_root_havok_bsx_flag(nif, node_block_id)

    return OperationResult(
        True,
        f"Generated FO4 {shape_type} collision ({len(vertices)} vertices, layer={layer}, blob={len(blob)} bytes)",
        [phys_sys.block_id, coll_obj.block_id],
    )


def generate_collision_from_geometry(
    nif,
    node_block_id: int,
    vertices: list[dict[str, float]] | np.ndarray,
    triangles: list[dict[str, int]] | np.ndarray | None = None,
    shape_type: str = "convex_hull",
    layer: str = "STATIC",
    material: object | None = None,
    mass: float = 0.0,
    friction: float = 0.5,
    restitution: float = 0.4,
    radius: float = DEFAULT_RADIUS,
    replace: bool = True,
    profile=None,
) -> OperationResult:
    """Generate collision on a node from explicit geometry instead of source blocks."""
    node = nif.get_block(node_block_id)
    if not node:
        return OperationResult(False, f"Node block {node_block_id} not found")

    verts_nif = np.array(
        [
            [float(v.get("x", 0.0)), float(v.get("y", 0.0)), float(v.get("z", 0.0))]
            for v in (vertices or [])
        ],
        dtype=np.float32,
    )
    if len(verts_nif) < 3:
        return OperationResult(False, "No valid collision geometry provided")

    triangles_arr: np.ndarray | None = None
    if triangles is not None and len(triangles) > 0:
        if isinstance(triangles, np.ndarray):
            triangles_arr = triangles.astype(np.int32, copy=False)
        else:
            triangles_arr = np.array(
                [
                    [int(t.get("v1", 0)), int(t.get("v2", 0)), int(t.get("v3", 0))]
                    for t in triangles
                ],
                dtype=np.int32,
            )
        if len(triangles_arr) == 0:
            triangles_arr = None

    effective_shape_type = shape_type
    if shape_type == "convex_hull" and len(verts_nif) < 4 and triangles_arr is not None:
        effective_shape_type = "compressed_mesh"
    effective_shape_type = _coerce_static_convex_to_compressed_mesh(
        effective_shape_type, layer, triangles_arr is not None
    )

    # FO4 / FO76 routing: emit bhkNPCollisionObject + bhkPhysicsSystem with a
    # Havok 2014.1.0 packfile blob, mirroring _generate_fo4_collision but
    # operating on caller-supplied verts/tris (used by the Max plugin and
    # other callers that already have geometry in hand).
    if profile is not None and getattr(profile, "id", None) in {"fo4", "fo76"}:
        return _generate_fo4_collision_from_geometry(
            nif=nif,
            node_block_id=node_block_id,
            vertices=verts_nif,
            triangles=triangles_arr,
            shape_type=effective_shape_type,
            layer=layer,
            material=material,
            mass=mass,
            friction=friction,
            restitution=restitution,
            replace=replace,
            profile=profile,
        )

    existing_coll = _find_collision_object(nif, node_block_id)
    existing_shape_id = None
    if existing_coll is not None:
        if replace:
            subtree = _collect_collision_subtree(nif, existing_coll)
            node.set_field("Collision Object", -1)
            nif.remove_blocks(subtree)
            node = nif.get_block(node_block_id)
        else:
            coll_block = nif.get_block(existing_coll)
            body_id = coll_block.get_field("Body") if coll_block else None
            if isinstance(body_id, int) and body_id >= 0:
                body_block = nif.get_block(body_id)
                if body_block is not None:
                    existing_shape_id = body_block.get_field("Shape")
                    if isinstance(existing_shape_id, int) and existing_shape_id < 0:
                        existing_shape_id = None

    created_ids: list[int] = []

    if effective_shape_type in {"convex_hull", "convex_fit"}:
        shape_verts = (
            _simplify_convex_fit_vertices(verts_nif)
            if effective_shape_type == "convex_fit"
            else verts_nif
        )
        shape_id = _create_convex_shape(nif, shape_verts, radius)
        if shape_id is None:
            return OperationResult(False, "Failed to create convex hull shape")
        created_ids.append(shape_id)

    elif effective_shape_type == "box":
        result = _create_box_shape(nif, verts_nif, radius)
        if result is None:
            return OperationResult(False, "Failed to create box shape")
        transform_id, box_id = result
        shape_id = transform_id
        created_ids.extend([transform_id, box_id])

    elif effective_shape_type == "capsule":
        from .collision_shapes import create_capsule_shape

        shape_id = create_capsule_shape(nif, verts_nif, radius)
        if shape_id is None:
            return OperationResult(False, "Failed to create capsule shape")
        created_ids.append(shape_id)

    elif effective_shape_type == "cylinder":
        from .collision_shapes import create_cylinder_shape

        shape_id = create_cylinder_shape(nif, verts_nif, radius)
        if shape_id is None:
            return OperationResult(False, "Failed to create cylinder shape")
        created_ids.append(shape_id)

    elif effective_shape_type == "sphere":
        from .collision_shapes import create_sphere_shape

        result = create_sphere_shape(nif, verts_nif, radius)
        if result is None:
            return OperationResult(False, "Failed to create sphere shape")
        transform_id, sphere_id = result
        shape_id = transform_id
        created_ids.extend([transform_id, sphere_id])

    elif effective_shape_type == "auto":
        from .collision_shapes import pick_best_primitive

        child_shape_ids = []
        for component_verts in _submesh_vertices_for_components(verts_nif, triangles_arr):
            sid = pick_best_primitive(nif, component_verts, radius)
            if sid is not None:
                child_shape_ids.append(sid)
                created_ids.append(sid)
        if not child_shape_ids:
            return OperationResult(False, "Failed to create any auto-fit shapes")
        if len(child_shape_ids) == 1:
            shape_id = child_shape_ids[0]
        else:
            shape_id = _create_list_shape(nif, child_shape_ids)
            if shape_id is None:
                return OperationResult(False, "Failed to create list shape")
            created_ids.append(shape_id)

    elif effective_shape_type == "list":
        child_shape_ids = []
        for component_verts in _submesh_vertices_for_components(verts_nif, triangles_arr):
            sid = _create_convex_shape(nif, component_verts, radius)
            if sid is not None:
                child_shape_ids.append(sid)
                created_ids.append(sid)
        if not child_shape_ids:
            return OperationResult(False, "Failed to create any sub-shapes")
        if len(child_shape_ids) == 1:
            shape_id = child_shape_ids[0]
        else:
            shape_id = _create_list_shape(nif, child_shape_ids)
            if shape_id is None:
                return OperationResult(False, "Failed to create list shape")
            created_ids.append(shape_id)

    elif effective_shape_type == "mopp":
        if triangles_arr is None or len(triangles_arr) == 0:
            return OperationResult(False, "MOPP collision requires triangle data")
        from .collision_mesh import create_mopp_shape

        mopp_radius = 0.005
        if profile and hasattr(profile, "id") and profile.id in ("fo3", "fnv"):
            mopp_radius = 0.1
        havok_sf = (
            profile.havok_scale
            if profile and hasattr(profile, "havok_scale") and profile.havok_scale
            else HAVOK_SCALE
        )
        result_ids = create_mopp_shape(
            nif,
            verts_nif,
            triangles_arr,
            radius=mopp_radius,
            havok_scale_factor=havok_sf,
        )
        if result_ids is None:
            return OperationResult(False, "Failed to create MOPP shape")
        mopp_id, packed_id, data_id = result_ids
        shape_id = mopp_id
        created_ids.extend([mopp_id, packed_id, data_id])

    elif effective_shape_type == "compressed_mesh":
        if triangles_arr is None or len(triangles_arr) == 0:
            return OperationResult(
                False,
                "Compressed mesh collision requires triangle data",
            )
        from .collision_mesh import create_compressed_mesh_shape

        havok_sf = (
            profile.havok_scale
            if profile and hasattr(profile, "havok_scale") and profile.havok_scale
            else HAVOK_SCALE
        )
        result_ids = create_compressed_mesh_shape(
            nif,
            verts_nif,
            triangles_arr,
            havok_scale_factor=havok_sf,
        )
        if result_ids is None:
            return OperationResult(False, "Failed to create compressed mesh shape")
        shape_id, data_id = result_ids
        created_ids.extend([shape_id, data_id])

    else:
        return OperationResult(False, f"Unknown shape_type: {shape_type}")

    if existing_shape_id is not None and not replace:
        merged_id = _create_list_shape(nif, [existing_shape_id, shape_id])
        if merged_id is not None:
            shape_id = merged_id
            created_ids.append(merged_id)

    layers = get_collision_layers(profile)
    result = _build_hierarchy(
        nif,
        node_block_id,
        shape_id,
        layer,
        mass,
        friction,
        restitution,
        layers,
    )
    if result is None:
        return OperationResult(False, "Failed to build collision hierarchy")

    coll_obj_id, rigid_body_id = result
    created_ids.extend([coll_obj_id, rigid_body_id])
    return OperationResult(
        True,
        f"Generated {effective_shape_type} collision ({len(verts_nif)} vertices, layer={layer})",
        created_ids,
    )


def generate_collision(
    nif,
    node_block_id: int,
    shape_type: str = "convex_hull",
    source_block_ids: list[int] | None = None,
    layer: str = "STATIC",
    material: object | None = None,
    mass: float = 0.0,
    friction: float = 0.5,
    restitution: float = 0.4,
    radius: float = DEFAULT_RADIUS,
    replace: bool = True,
    profile=None,
    include_child_nodes: bool = True,
) -> OperationResult:
    """Generate full collision hierarchy on a node.

    Args:
        nif: NifFile instance
        node_block_id: Parent node (e.g. root BSFadeNode, block 0)
        shape_type: "convex_hull", "box", "capsule", "cylinder", "sphere", "auto", or "list"
        source_block_ids: BSTriShape block IDs to generate from (auto-discovers if None)
        include_child_nodes: If auto-discovering from a node, include shapes under child NiNodes.
        layer: Havok collision layer name
        material: Bethesda Havok material name or numeric CRC for FO4/FO76 packfiles
        mass: Object mass (0 = static/fixed)
        friction: Surface friction coefficient
        restitution: Bounciness coefficient
        radius: Convex radius / shell thickness
        replace: If True, remove existing collision first. If False, merge into bhkListShape.

    Returns:
        OperationResult with modified block IDs
    """
    node = nif.get_block(node_block_id)
    if not node:
        return OperationResult(False, f"Node block {node_block_id} not found")

    # Starfield: generate Havok 2019 TAG0 blob
    if profile and getattr(profile, 'engine', None) == "creation2":
        return _generate_starfield_collision(
            nif, node_block_id, source_block_ids,
            include_child_nodes=include_child_nodes,
            layer=layer, mass=mass, friction=friction,
            restitution=restitution, radius=radius,
            replace=replace, profile=profile,
        )

    # FO4: generate Havok 2014.1.0 packfile blob (bhkNPCollisionObject + bhkPhysicsSystem)
    # FO76 also uses Havok 2014 packfiles; SkyrimSE uses the legacy bhk chain despite
    # sharing engine="creation1", so we gate on profile.id rather than engine.
    _FO4_PACKFILE_IDS = {"fo4", "fo76"}
    if profile and getattr(profile, 'id', None) in _FO4_PACKFILE_IDS:
        return _generate_fo4_collision(
            nif, node_block_id,
            shape_type=shape_type,
            source_block_ids=source_block_ids,
            include_child_nodes=include_child_nodes,
            layer=layer, material=material, mass=mass, friction=friction,
            restitution=restitution, radius=radius,
            replace=replace, profile=profile,
        )

    # Legacy bhkRigidBody + shape chain — Skyrim LE / FO3 / FNV
    auto_sources = source_block_ids is None
    source_block_ids, all_verts, per_shape_verts = _collect_source_vertices(
        nif,
        node_block_id,
        source_block_ids,
        include_child_nodes=include_child_nodes,
    )
    if not source_block_ids:
        return OperationResult(False, "No BSTriShape children found for collision generation")

    if not all_verts:
        return OperationResult(False, "No valid vertex data found in source shapes")

    combined_verts = np.vstack(all_verts)

    # Handle existing collision
    existing_coll = _find_collision_object(nif, node_block_id)
    existing_shape_id = None
    if existing_coll is not None:
        if replace:
            # Remove existing collision subtree
            subtree = _collect_collision_subtree(nif, existing_coll)
            node.set_field("Collision Object", -1)
            nif.remove_blocks(subtree)
            # Block IDs shifted after removal — re-resolve node
            node = nif.get_block(node_block_id)
            source_block_ids, all_verts, per_shape_verts = _collect_source_vertices(
                nif,
                node_block_id,
                None if auto_sources else source_block_ids,
                include_child_nodes=include_child_nodes,
            )
            if not all_verts:
                return OperationResult(False, "No valid vertex data after removing old collision")
            combined_verts = np.vstack(all_verts)
        else:
            # Merge mode: find existing shape to wrap in ListShape later
            rb_block = nif.get_block(existing_coll)
            if rb_block:
                body_ref = rb_block.get_field("Body")
                if body_ref is not None and isinstance(body_ref, int) and body_ref >= 0:
                    body_block = nif.get_block(body_ref)
                    if body_block:
                        existing_shape_id = body_block.get_field("Shape")
                        if isinstance(existing_shape_id, int) and existing_shape_id < 0:
                            existing_shape_id = None

    created_ids = []

    if shape_type == "convex_hull":
        shape_id = _create_convex_shape(nif, combined_verts, radius)
        if shape_id is None:
            return OperationResult(False, "Failed to create convex hull shape")
        created_ids.append(shape_id)

    elif shape_type == "box":
        result = _create_box_shape(nif, combined_verts, radius)
        if result is None:
            return OperationResult(False, "Failed to create box shape")
        transform_id, box_id = result
        shape_id = transform_id
        created_ids.extend([transform_id, box_id])

    elif shape_type == "capsule":
        from .collision_shapes import create_capsule_shape
        shape_id = create_capsule_shape(nif, combined_verts, radius)
        if shape_id is None:
            return OperationResult(False, "Failed to create capsule shape")
        created_ids.append(shape_id)

    elif shape_type == "cylinder":
        from .collision_shapes import create_cylinder_shape
        shape_id = create_cylinder_shape(nif, combined_verts, radius)
        if shape_id is None:
            return OperationResult(False, "Failed to create cylinder shape")
        created_ids.append(shape_id)

    elif shape_type == "sphere":
        from .collision_shapes import create_sphere_shape
        result = create_sphere_shape(nif, combined_verts, radius)
        if result is None:
            return OperationResult(False, "Failed to create sphere shape")
        transform_id, sphere_id = result
        shape_id = transform_id
        created_ids.extend([transform_id, sphere_id])

    elif shape_type == "auto":
        from .collision_shapes import pick_best_primitive
        # Each source shape gets its own best-fit primitive
        child_shape_ids = []
        for verts in per_shape_verts:
            sid = pick_best_primitive(nif, verts, radius)
            if sid is not None:
                child_shape_ids.append(sid)
                created_ids.append(sid)
        if not child_shape_ids:
            return OperationResult(False, "Failed to create any auto-fit shapes")
        if len(child_shape_ids) == 1:
            shape_id = child_shape_ids[0]
        else:
            shape_id = _create_list_shape(nif, child_shape_ids)
            if shape_id is None:
                return OperationResult(False, "Failed to create list shape")
            created_ids.append(shape_id)

    elif shape_type == "list":
        # Create one convex hull per source shape, wrap in ListShape
        child_shape_ids = []
        for verts in per_shape_verts:
            sid = _create_convex_shape(nif, verts, radius)
            if sid is not None:
                child_shape_ids.append(sid)
                created_ids.append(sid)
        if not child_shape_ids:
            return OperationResult(False, "Failed to create any sub-shapes for compound collision")
        if len(child_shape_ids) == 1:
            shape_id = child_shape_ids[0]
        else:
            shape_id = _create_list_shape(nif, child_shape_ids)
            if shape_id is None:
                return OperationResult(False, "Failed to create list shape")
            created_ids.append(shape_id)
    elif shape_type == "mopp":
        from .collision_mesh import create_mopp_shape
        # Collect triangles from all source BSTriShapes
        combined_tris = _collect_all_triangles(nif, source_block_ids)
        if len(combined_tris) == 0:
            return OperationResult(False, "No triangle data found for MOPP generation")

        # Determine radius based on game profile
        mopp_radius = 0.005  # Skyrim default
        if profile and hasattr(profile, 'id'):
            if profile.id in ('fo3', 'fnv'):
                mopp_radius = 0.1
        havok_sf = profile.havok_scale if profile and hasattr(profile, 'havok_scale') and profile.havok_scale else HAVOK_SCALE

        result_ids = create_mopp_shape(nif, combined_verts, combined_tris,
                                       radius=mopp_radius, havok_scale_factor=havok_sf)
        if result_ids is None:
            return OperationResult(False, "Failed to create MOPP shape")
        mopp_id, packed_id, data_id = result_ids
        shape_id = mopp_id
        created_ids.extend([mopp_id, packed_id, data_id])

    elif shape_type == "compressed_mesh":
        from .collision_mesh import create_compressed_mesh_shape
        combined_tris = _collect_all_triangles(nif, source_block_ids)
        if len(combined_tris) == 0:
            return OperationResult(False, "No triangle data for compressed mesh")
        havok_sf = profile.havok_scale if profile and hasattr(profile, 'havok_scale') and profile.havok_scale else HAVOK_SCALE
        result_ids = create_compressed_mesh_shape(nif, combined_verts, combined_tris,
                                                   havok_scale_factor=havok_sf)
        if result_ids is None:
            return OperationResult(False, "Failed to create compressed mesh shape")
        shape_id, data_id = result_ids
        created_ids.extend([shape_id, data_id])

    else:
        return OperationResult(False, f"Unknown shape_type: {shape_type}")

    # Merge with existing shape if not replacing
    if existing_shape_id is not None and not replace:
        merged_id = _create_list_shape(nif, [existing_shape_id, shape_id])
        if merged_id is not None:
            shape_id = merged_id
            created_ids.append(merged_id)

    # Build hierarchy
    layers = get_collision_layers(profile)
    result = _build_hierarchy(nif, node_block_id, shape_id, layer, mass, friction, restitution, layers)
    if result is None:
        return OperationResult(False, "Failed to build collision hierarchy")

    coll_obj_id, rigid_body_id = result
    created_ids.extend([coll_obj_id, rigid_body_id])

    n_verts = len(combined_verts)
    return OperationResult(
        True,
        f"Generated {shape_type} collision ({n_verts} vertices, layer={layer})",
        created_ids,
    )


def remove_collision(nif, node_block_id: int) -> OperationResult:
    """Remove collision subtree from a node and clear the Collision Object reference.

    Args:
        nif: NifFile instance
        node_block_id: Node block ID to remove collision from

    Returns:
        OperationResult
    """
    node = nif.get_block(node_block_id)
    if not node:
        return OperationResult(False, f"Node block {node_block_id} not found")

    coll_id = _find_collision_object(nif, node_block_id)
    if coll_id is None:
        return OperationResult(False, "No collision object found on this node")

    subtree = _collect_collision_subtree(nif, coll_id)
    node.set_field("Collision Object", -1)
    nif.remove_blocks(subtree)

    return OperationResult(
        True,
        f"Removed {len(subtree)} collision block(s) from node {node_block_id}",
    )


# ---- Legacy API (kept for backward compatibility) ----

def create_convex_hull(nif, shape_block_id: int) -> OperationResult:
    """Create bhkConvexVerticesShape from a mesh's vertex positions.

    Legacy function — creates a standalone shape without hierarchy.
    Use generate_collision() for full collision setup.
    """
    verts = _extract_vertices(nif, shape_block_id)
    if verts is None:
        block = nif.get_block(shape_block_id)
        if not block:
            return OperationResult(False, f"Block {shape_block_id} not found")
        return OperationResult(False, "No vertex data")

    shape_id = _create_convex_shape(nif, verts)
    if shape_id is None:
        return OperationResult(False, "Failed to create convex hull")

    return OperationResult(True,
        f"Created convex hull with {len(verts)} verts from block {shape_block_id}",
        [shape_id])


def convert_collision_shape(nif, block_id: int, target_type: str,
                            profile=None) -> OperationResult:
    """Convert between collision shape types by extracting and regenerating geometry.

    Extracts vertices (and triangles where available) from the source shape,
    then creates a new shape of the target type.

    Args:
        nif: NifFile instance.
        block_id: Source collision shape block ID.
        target_type: "convex_hull", "mopp", or "compressed_mesh".
        profile: Optional game profile for scale/radius defaults.

    Returns:
        OperationResult with created block IDs.
    """
    block = nif.get_block(block_id)
    if not block:
        return OperationResult(False, f"Block {block_id} not found")

    # Extract geometry from source shape
    source_type = block.type_name
    verts = None
    tris = None

    havok_sf = profile.havok_scale if profile and hasattr(profile, 'havok_scale') and profile.havok_scale else HAVOK_SCALE

    if source_type == "bhkConvexVerticesShape":
        raw_verts = block.get_field("Vertices") or []
        if not raw_verts:
            return OperationResult(False, "No vertices in convex shape")
        verts = np.array(
            [[v["x"], v["y"], v["z"]] for v in raw_verts],
            dtype=np.float32,
        ) * havok_sf
        # Convex shapes store planes, not triangles; ask native hull code for a surface mesh.
        try:
            from creation_lib._native.havok_native import convex_hull_triangles

            hull_verts, hull_tris = convex_hull_triangles(
                [[float(v[0]), float(v[1]), float(v[2])] for v in verts]
            )
            verts = np.array(hull_verts, dtype=np.float32)
            tris = np.array(hull_tris, dtype=np.int32)
        except Exception as e:
            return OperationResult(False, f"Failed to triangulate convex hull: {e}")

    elif source_type == "bhkMoppBvTreeShape":
        from .collision_mesh import extract_mopp_geometry
        verts, tris, _ = extract_mopp_geometry(nif, block_id, havok_sf)

    elif source_type == "bhkCompressedMeshShapeData":
        from .collision_mesh import extract_compressed_mesh
        verts, tris, _ = extract_compressed_mesh(nif, block_id, havok_sf)

    elif source_type == "bhkCompressedMeshShape":
        # Navigate to the data block
        data_id = block.get_field("Data")
        if data_id is None or data_id < 0:
            return OperationResult(False, "bhkCompressedMeshShape has no Data ref")
        from .collision_mesh import extract_compressed_mesh
        verts, tris, _ = extract_compressed_mesh(nif, data_id, havok_sf)

    else:
        return OperationResult(False, f"Cannot extract geometry from {source_type}")

    if verts is None or len(verts) < 3:
        return OperationResult(False, "Insufficient geometry for conversion")

    # Create target shape
    if target_type == "convex_hull":
        shape_id = _create_convex_shape(nif, verts)
        if shape_id is None:
            return OperationResult(False, "Failed to create convex hull")
        return OperationResult(
            True, f"Converted {source_type} -> convex_hull", [shape_id]
        )

    elif target_type == "mopp":
        if tris is None or len(tris) == 0:
            return OperationResult(False, "MOPP requires triangles")
        from .collision_mesh import create_mopp_shape
        result = create_mopp_shape(nif, verts, tris)
        if result is None:
            return OperationResult(False, "Failed to create MOPP shape")
        return OperationResult(
            True, f"Converted {source_type} -> mopp", list(result)
        )

    elif target_type == "compressed_mesh":
        if tris is None or len(tris) == 0:
            return OperationResult(False, "Compressed mesh requires triangles")
        from .collision_mesh import create_compressed_mesh_shape
        result = create_compressed_mesh_shape(nif, verts, tris)
        if result is None:
            return OperationResult(False, "Failed to create compressed mesh")
        return OperationResult(
            True, f"Converted {source_type} -> compressed_mesh", list(result)
        )

    else:
        return OperationResult(False, f"Unknown target shape type: {target_type}")
