"""NIF to ModernGL scene graph loader.

Walks the NIF block hierarchy and builds a matching SceneNode tree
with Mesh objects for BSTriShape blocks.
Vertex data is bulk-copied via numpy for performance.
"""

from __future__ import annotations
import os
import logging
import time
from pathlib import Path
from collections import defaultdict

import numpy as np
import glm
import moderngl

from creation_lib.geometry.preview_meshes import (
    box_mesh_from_half_extents,
    capsule_mesh_from_endpoints,
    mesh_to_wireframe_lines,
    sphere_mesh_from_center,
)
from creation_lib.havok.collision_preview import extract_preview_meshes_from_blob
from creation_lib.nif.nif_file import NifFile
from creation_lib.renderer.scene_renderer import SceneNode, Mesh, Material

_log = logging.getLogger("renderer.nif_loader")

from dataclasses import dataclass, field
from typing import Any


@dataclass
class CollisionOverlay:
    """Wireframe collision shape overlay data."""

    positions: np.ndarray  # Nx3 line endpoints (pairs of points for LINES mode)
    color: tuple = (
        0.9,
        0.7,
        0.4,
        0.85,
    )  # orange wireframe (matches scene tree category)
    shapes: list["CollisionShapeOverlay"] = field(default_factory=list)


@dataclass
class CollisionShapeOverlay:
    """Selectable triangle geometry for one collision shape."""

    source_block_id: int
    body_id: int | None
    shape_index: int | None
    shape_type: str
    vertices: np.ndarray
    triangles: np.ndarray
    positions: np.ndarray


@dataclass
class _PrepareContext:
    """Context passed through the recursive walk for mesh resolution."""

    nif_id: str
    texture_dirs: list
    ba2_mgr: object
    nif_path: str = ""


@dataclass
class PreparedTextureSlice:
    """One triangle group from a FO76 shader texture array."""

    slice_index: int
    verts: "np.ndarray"
    normals: "np.ndarray"
    uvs: "np.ndarray"
    tris: "np.ndarray"
    colors: "np.ndarray | None"
    tangents: "np.ndarray | None"
    bitangents: "np.ndarray | None"
    texture_paths: dict[str, str]
    uv2: "np.ndarray | None" = None


@dataclass
class PreparedShape:
    """Vertex data extracted from one BSTriShape — no GL objects."""

    block_id: int
    name: str
    verts: "np.ndarray"
    normals: "np.ndarray"
    uvs: "np.ndarray"
    tris: "np.ndarray"
    colors: "np.ndarray | None"
    tangents: "np.ndarray | None"
    bitangents: "np.ndarray | None"
    transform: "glm.mat4"
    material_inputs: dict  # reserved for future pre-extraction; currently always {} since build_material reads from the nif block directly
    external_mesh_paths: list = (
        None  # Starfield external .mesh references (list of paths)
    )
    uv2: "np.ndarray | None" = (
        None  # Starfield second UV channel (None for non-Starfield meshes)
    )
    texture_slices: "list[PreparedTextureSlice] | None" = None


@dataclass
class PreparedRenderBatch:
    """Combined render data for large LOD files that contain many tiny shapes."""

    name: str
    block_id: int
    material_shape_id: int
    source_block_ids: tuple[int, ...]
    verts: "np.ndarray"
    normals: "np.ndarray"
    uvs: "np.ndarray"
    tris: "np.ndarray"
    colors: "np.ndarray | None"
    tangents: "np.ndarray | None"
    bitangents: "np.ndarray | None"
    uv2: "np.ndarray | None" = None


@dataclass
class PreparedNifData:
    """All data from the CPU phase of NIF loading — no GL objects."""

    nif: Any  # NifFile — already parsed, thread-safe for reading
    nif_id: str
    filepath: str
    shapes: dict  # dict[int, PreparedShape] keyed by block_id
    decoded_textures: dict  # dict[str, Any] path → DecodedTexture or None
    texture_dirs: list
    ba2_mgr: Any  # BA2Manager snapshot — read-only from background thread
    game_profile: Any = None  # GameProfile — used for havok_scale, etc.
    render_batches: list[PreparedRenderBatch] | None = None


@dataclass
class PreparedAttachData:
    """Result of the background attach phase — no GL objects."""

    prepared: PreparedNifData  # result of prepare_nif_data
    matched_cp: str  # parent CP name to attach at (e.g. "P-Barrel")
    parent_nif_id: str  # which session to graft into (usually "main")


def prepare_attach_data(
    filepath: str,
    texture_dirs: list,
    ba2_mgr,
    nif_id: str,
    parent_cp_names: set,
    occupied_cps: set,
    game_profile=None,
    parent_nif_id: str = "main",
) -> "PreparedAttachData":
    """Background-thread phase of async attach — no GL calls.

    Loads and prepares the child NIF, validates connect points against
    pre-snapshotted sets from the UI thread, and returns PreparedAttachData.
    Raises ValueError with a user-facing message on any validation failure.
    """
    prepared = prepare_nif_data(
        filepath, texture_dirs, ba2_mgr, nif_id, game_profile=game_profile
    )

    # Collect child connect point names from BSConnectPoint::Children blocks
    child_cp_names = []
    for block in prepared.nif.blocks:
        if block.type_name == "BSConnectPoint::Children":
            point_names = block.get_field("Point Name") or []
            if isinstance(point_names, str):
                point_names = [point_names]
            for name in point_names:
                child_cp_names.append(str(name))

    if not child_cp_names:
        from pathlib import Path

        raise ValueError(
            f"Cannot attach {Path(filepath).name}: "
            "no child connect points (BSConnectPoint::Children) found"
        )

    # Convert child CP names → parent CP candidates (C-X → P-X)
    parent_candidates = []
    for cp in child_cp_names:
        if cp.startswith("C-") or cp.startswith("c-"):
            parent_candidates.append("P-" + cp[2:])
        else:
            parent_candidates.append("P-" + cp)

    # Find first matching parent CP in the snapshot
    matched_cp = next((c for c in parent_candidates if c in parent_cp_names), None)
    if matched_cp is None:
        from pathlib import Path

        child_names = ", ".join(child_cp_names)
        parent_names = ", ".join(sorted(parent_cp_names)) if parent_cp_names else "none"
        raise ValueError(
            f"Cannot attach {Path(filepath).name}: "
            f"child CPs [{child_names}] have no matching parent CPs "
            f"(available: [{parent_names}])"
        )

    # Check if CP is already occupied
    if matched_cp in occupied_cps:
        from pathlib import Path

        raise ValueError(f"Cannot attach at {matched_cp}: already attached there")

    return PreparedAttachData(
        prepared=prepared,
        matched_cp=matched_cp,
        parent_nif_id=parent_nif_id,
    )


def interleave_vertex_data(
    verts: np.ndarray,
    normals: np.ndarray,
    uvs: np.ndarray,
    colors: np.ndarray | None = None,
    tangents: np.ndarray | None = None,
    bitangents: np.ndarray | None = None,
    uv2: np.ndarray | None = None,
) -> tuple[np.ndarray, str, list[str]]:
    """Interleave vertex arrays into a single buffer.

    Returns:
        (interleaved_data, format_string, attribute_names)
        format_string is ModernGL format e.g. "3f 3f 2f"
        attribute_names is list e.g. ["in_position", "in_normal", "in_texcoord"]
    """
    n = len(verts)
    arrays = [verts, normals, uvs]
    fmt_parts = ["3f", "3f", "2f"]
    attrs = ["in_position", "in_normal", "in_texcoord"]

    if uv2 is not None:
        arrays.append(uv2)
        fmt_parts.append("2f")
        attrs.append("in_texcoord2")

    if colors is not None:
        arrays.append(colors)
        fmt_parts.append("4f")
        attrs.append("in_color")

    if tangents is not None:
        arrays.append(tangents)
        fmt_parts.append("3f")
        attrs.append("in_tangent")

    if bitangents is not None:
        arrays.append(bitangents)
        fmt_parts.append("3f")
        attrs.append("in_bitangent")

    total_cols = sum(a.shape[1] for a in arrays)
    interleaved = np.empty((n, total_cols), dtype=np.float32)
    col = 0
    for arr in arrays:
        w = arr.shape[1]
        interleaved[:, col : col + w] = arr
        col += w

    return interleaved, " ".join(fmt_parts), attrs


def nif_transform_to_mat4(
    translation: list | dict,
    rotation: list | dict,
    scale: float,
) -> glm.mat4:
    """Convert NIF transform fields to a glm.mat4.

    NIF binary stores the 3x3 rotation matrix in row-major order.
    nif.xml field names use m[col][row] convention (m11,m21,m31 = row 0),
    NOT the standard m[row][col].  We extract 3 rows (r0,r1,r2) and build
    a column-major GLM mat4 from them.
    """
    # Handle dict-style translation
    if isinstance(translation, dict):
        tx = float(translation.get("x", 0))
        ty = float(translation.get("y", 0))
        tz = float(translation.get("z", 0))
    elif isinstance(translation, (list, tuple)):
        tx, ty, tz = float(translation[0]), float(translation[1]), float(translation[2])
    else:
        tx = ty = tz = 0.0

    # Extract 3 rows of the rotation matrix in standard [row][col] order.
    if isinstance(rotation, dict):
        # nif.xml dict: binary row-major but names are m[col][row].
        # First 3 binary floats = row 0 = dict keys m11, m21, m31.
        r0 = (
            float(rotation.get("m11", 1.0)),
            float(rotation.get("m21", 0.0)),
            float(rotation.get("m31", 0.0)),
        )
        r1 = (
            float(rotation.get("m12", 0.0)),
            float(rotation.get("m22", 1.0)),
            float(rotation.get("m32", 0.0)),
        )
        r2 = (
            float(rotation.get("m13", 0.0)),
            float(rotation.get("m23", 0.0)),
            float(rotation.get("m33", 1.0)),
        )
    elif isinstance(rotation, (list, tuple)) and len(rotation) == 3:
        # List of rows — already standard [row][col] order.
        r0 = tuple(float(x) for x in rotation[0])
        r1 = tuple(float(x) for x in rotation[1])
        r2 = tuple(float(x) for x in rotation[2])
    else:
        r0 = (1.0, 0.0, 0.0)
        r1 = (0.0, 1.0, 0.0)
        r2 = (0.0, 0.0, 1.0)

    s = float(scale)

    # GLM mat4 is column-major: column j = [row0[j], row1[j], row2[j], 0].
    return glm.mat4(
        r0[0] * s,
        r1[0] * s,
        r2[0] * s,
        0,  # column 0
        r0[1] * s,
        r1[1] * s,
        r2[1] * s,
        0,  # column 1
        r0[2] * s,
        r1[2] * s,
        r2[2] * s,
        0,  # column 2
        tx,
        ty,
        tz,
        1,  # column 3: translation
    )


def compute_normals(verts: np.ndarray, tris: np.ndarray) -> np.ndarray:
    """Compute smooth vertex normals from face normals (area-weighted)."""
    normals = np.zeros_like(verts, dtype=np.float32)

    if len(tris) == 0:
        normals[:, 2] = 1.0  # Default up
        return normals

    v0 = verts[tris[:, 0]]
    v1 = verts[tris[:, 1]]
    v2 = verts[tris[:, 2]]
    face_normals = np.cross(v1 - v0, v2 - v0)

    # Accumulate face normals at each vertex
    for col in range(3):
        np.add.at(normals, tris[:, col], face_normals)

    # Normalize
    lengths = np.linalg.norm(normals, axis=1, keepdims=True)
    lengths[lengths < 1e-8] = 1.0
    return (normals / lengths).astype(np.float32)


def _is_tri_based_shape(schema, type_name: str) -> bool:
    return (
        schema.is_subtype_of(type_name, "BSTriShape")
        or schema.is_subtype_of(type_name, "NiTriShape")
        or schema.is_subtype_of(type_name, "NiTriStrips")
    )


def _uses_legacy_shape_data(schema, block) -> bool:
    if _get_ref_id(block.get_field("Data")) >= 0:
        return True
    if schema.is_subtype_of(block.type_name, "BSTriShape"):
        return False
    return schema.is_subtype_of(block.type_name, "NiTriShape") or schema.is_subtype_of(
        block.type_name, "NiTriStrips"
    )


def _triangles_from_strips(
    strip_lengths: list[int], points: list[int] | list[list[int]]
) -> np.ndarray:
    tris: list[tuple[int, int, int]] = []
    grouped_points = bool(points) and isinstance(points[0], list)
    offset = 0
    for strip_index, strip_length in enumerate(strip_lengths):
        if grouped_points:
            raw_strip = points[strip_index] if strip_index < len(points) else []
            strip = [int(v or 0) for v in raw_strip[:strip_length]]
        else:
            strip = [int(v or 0) for v in points[offset : offset + strip_length]]
            offset += strip_length
        for index in range(max(0, len(strip) - 2)):
            a, b, c = strip[index], strip[index + 1], strip[index + 2]
            if len({a, b, c}) < 3:
                continue
            if index % 2:
                tris.append((b, a, c))
            else:
                tris.append((a, b, c))
    return np.array(tris, dtype=np.uint32)


def _compute_bounds_from_verts(
    verts: np.ndarray, world_transform: glm.mat4
) -> tuple[glm.vec3, float]:
    """Compute world-space bounding sphere from vertex positions."""
    if len(verts) == 0:
        return glm.vec3(0), 0.0
    center_local = verts.mean(axis=0)
    dists = np.linalg.norm(verts - center_local, axis=1)
    radius_local = float(dists.max())
    center_world = glm.vec3(world_transform * glm.vec4(*center_local, 1.0))
    scale = max(
        glm.length(glm.vec3(world_transform[0])),
        glm.length(glm.vec3(world_transform[1])),
        glm.length(glm.vec3(world_transform[2])),
    )
    radius_world = radius_local * scale
    return center_world, radius_world


def _update_world_transforms(node: SceneNode, parent_world: glm.mat4):
    """Recursively compute world transforms and bounding spheres."""
    node.world_transform = parent_world * node.transform
    # Compute world-space bounding sphere for mesh nodes
    if node.mesh and hasattr(node, "_local_verts") and node._local_verts is not None:
        node.bound_center, node.bound_radius = _compute_bounds_from_verts(
            node._local_verts, node.world_transform
        )
    for child in node.children:
        _update_world_transforms(child, node.world_transform)


def _extract_collision_overlay(
    nif, node_block, havok_scale: float | None = None
) -> CollisionOverlay | None:
    """Extract collision wireframe geometry from a node's collision object."""
    coll_ref = node_block.get_field("Collision Object")
    if coll_ref is None:
        return None
    coll_id = coll_ref if isinstance(coll_ref, int) else -1
    if coll_id < 0:
        return None
    coll_block = nif.get_block(coll_id)
    if not coll_block:
        return None

    lines = None
    shapes: list[CollisionShapeOverlay] = []

    if coll_block.type_name == "bhkCollisionObject":
        # Legacy format: bhkCollisionObject → bhkRigidBody → shape
        shapes = _extract_legacy_collision_shapes(
            nif, coll_block, havok_scale=havok_scale
        )
        if shapes:
            lines = [point.tolist() for shape in shapes for point in shape.positions]
        else:
            lines = _extract_legacy_collision_lines(
                nif, coll_block, havok_scale=havok_scale
            )

    elif coll_block.type_name == "bhkNPCollisionObject":
        # NP format: bhkNPCollisionObject → bhkPhysicsSystem (binary blob)
        shapes = _extract_np_collision_shapes(nif, coll_block, havok_scale=havok_scale)
        if shapes:
            lines = [point.tolist() for shape in shapes for point in shape.positions]
        else:
            lines = _extract_np_collision_lines(
                nif, coll_block, havok_scale=havok_scale
            )

    if lines is None or len(lines) == 0:
        return None

    return CollisionOverlay(
        positions=np.array(lines, dtype=np.float32),
        shapes=shapes,
    )


def _extract_legacy_collision_lines(
    nif, coll_block, havok_scale: float | None = None
) -> list | None:
    """Extract wireframe from legacy bhkCollisionObject → bhkRigidBody → shape chain."""
    body_ref = coll_block.get_field("Body")
    if body_ref is None:
        return None
    body_id = body_ref if isinstance(body_ref, int) else -1
    if body_id < 0:
        return None
    body_block = nif.get_block(body_id)
    if not body_block:
        return None

    shape_ref = body_block.get_field("Shape")
    if shape_ref is None:
        return None
    shape_id = shape_ref if isinstance(shape_ref, int) else -1
    if shape_id < 0:
        return None

    return _extract_shape_lines(nif, shape_id, havok_scale=havok_scale)


def _mesh_overlay(
    mesh: dict,
    source_block_id: int,
    shape_type: str,
    body_id: int | None = None,
    shape_index: int | None = None,
    transform: np.ndarray | None = None,
) -> CollisionShapeOverlay | None:
    raw_vertices = mesh.get("vertices") or []
    raw_triangles = mesh.get("triangles") or []
    if not raw_vertices or not raw_triangles:
        return None
    vertices = np.array(
        [
            [
                float(vertex.get("x", 0.0)),
                float(vertex.get("y", 0.0)),
                float(vertex.get("z", 0.0)),
            ]
            for vertex in raw_vertices
        ],
        dtype=np.float32,
    )
    triangles = np.array(
        [
            [
                int(triangle.get("v1", 0)),
                int(triangle.get("v2", 0)),
                int(triangle.get("v3", 0)),
            ]
            for triangle in raw_triangles
        ],
        dtype=np.uint32,
    )
    if transform is not None:
        homogeneous = np.hstack(
            [vertices, np.ones((len(vertices), 1), dtype=np.float32)]
        )
        vertices = (transform @ homogeneous.T).T[:, :3].astype(np.float32)
    transformed_mesh = {
        "vertices": [
            {"x": float(v[0]), "y": float(v[1]), "z": float(v[2])} for v in vertices
        ],
        "triangles": raw_triangles,
    }
    positions = np.array(mesh_to_wireframe_lines(transformed_mesh), dtype=np.float32)
    return CollisionShapeOverlay(
        source_block_id=source_block_id,
        body_id=body_id,
        shape_index=shape_index,
        shape_type=shape_type,
        vertices=vertices,
        triangles=triangles,
        positions=positions,
    )


def _legacy_shape_transform(value, havok_scale: float) -> np.ndarray:
    transform = np.eye(4, dtype=np.float32)
    if not isinstance(value, dict):
        return transform
    for row in range(3):
        for column in range(3):
            key = f"m{row + 1}{column + 1}"
            if key in value:
                transform[row, column] = float(value[key])
    translation = value.get("Translation")
    if isinstance(translation, dict):
        transform[0, 3] = float(translation.get("x", 0.0)) * havok_scale
        transform[1, 3] = float(translation.get("y", 0.0)) * havok_scale
        transform[2, 3] = float(translation.get("z", 0.0)) * havok_scale
    else:
        transform[0, 3] = float(value.get("m14", 0.0)) * havok_scale
        transform[1, 3] = float(value.get("m24", 0.0)) * havok_scale
        transform[2, 3] = float(value.get("m34", 0.0)) * havok_scale
    return transform


def _convex_shape_mesh(block, havok_scale: float) -> dict | None:
    vertices = block.get_field("Vertices") or []
    if len(vertices) < 4:
        return None
    mesh_vertices = [
        {
            "x": float(vertex.get("x", 0.0)) * havok_scale,
            "y": float(vertex.get("y", 0.0)) * havok_scale,
            "z": float(vertex.get("z", 0.0)) * havok_scale,
        }
        for vertex in vertices
    ]
    try:
        from creation_lib.scientific.native_runtime import convex_hull_triangles

        faces = convex_hull_triangles([[v["x"], v["y"], v["z"]] for v in mesh_vertices])
    except Exception:
        return None
    return {
        "vertices": mesh_vertices,
        "triangles": [
            {"v1": int(face[0]), "v2": int(face[1]), "v3": int(face[2])}
            for face in faces
        ],
    }


def _packed_tri_strips_mesh(nif, block, havok_scale: float) -> dict | None:
    """Triangle soup from a Gamebryo-era bhkPackedNiTriStripsShape (FO3/FNV/Oblivion)."""
    data_id = block.get_field("Data")
    if not isinstance(data_id, int) or data_id < 0:
        return None
    data = nif.get_block(data_id)
    if data is None or data.type_name != "hkPackedNiTriStripsData":
        return None
    if data.get_field("Compressed"):
        return None

    shape_scale = block.get_field("Scale") or {}
    axis_scale = [
        float(shape_scale.get(axis, 1.0) or 1.0) if isinstance(shape_scale, dict) else 1.0
        for axis in ("x", "y", "z")
    ]
    vertices = [
        {
            "x": float(vertex.get("x", 0.0)) * axis_scale[0] * havok_scale,
            "y": float(vertex.get("y", 0.0)) * axis_scale[1] * havok_scale,
            "z": float(vertex.get("z", 0.0)) * axis_scale[2] * havok_scale,
        }
        for vertex in (data.get_field("Vertices") or [])
    ]
    triangles = []
    for entry in data.get_field("Triangles") or []:
        triangle = entry.get("Triangle", entry) if isinstance(entry, dict) else None
        if not isinstance(triangle, dict):
            continue
        indices = [int(triangle.get(key, 0)) for key in ("v1", "v2", "v3")]
        if any(index >= len(vertices) for index in indices):
            continue
        triangles.append({"v1": indices[0], "v2": indices[1], "v3": indices[2]})
    if len(vertices) < 3 or not triangles:
        return None
    return {"vertices": vertices, "triangles": triangles}


def _legacy_body_transform(nif, body_block, havok_scale: float) -> np.ndarray:
    """bhkRigidBodyT bakes its own translation/rotation into the shape; bhkRigidBody does not."""
    transform = np.eye(4, dtype=np.float32)
    if body_block is None or body_block.type_name != "bhkRigidBodyT":
        return transform
    info = (
        body_block.get_field("Rigid Body Info:550_660")
        or body_block.get_field("Rigid Body Info:2010")
        or body_block.get_field("Rigid Body Info")
    )
    if not isinstance(info, dict):
        return transform
    rotation = info.get("Rotation")
    if isinstance(rotation, dict):
        x, y, z, w = (float(rotation.get(key, 0.0)) for key in ("x", "y", "z", "w"))
        norm = (x * x + y * y + z * z + w * w) ** 0.5
        if norm > 1e-6:
            x, y, z, w = x / norm, y / norm, z / norm, w / norm
            transform[:3, :3] = np.array(
                [
                    [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
                    [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
                    [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
                ],
                dtype=np.float32,
            )
    translation = info.get("Translation")
    if isinstance(translation, dict):
        for row, axis in enumerate(("x", "y", "z")):
            transform[row, 3] = float(translation.get(axis, 0.0)) * havok_scale
    return transform


def _extract_legacy_collision_shapes(
    nif,
    coll_block,
    havok_scale: float | None = None,
) -> list[CollisionShapeOverlay]:
    from creation_lib.nif.operations.collision import HAVOK_SCALE_FO4

    body_id = coll_block.get_field("Body")
    body = nif.get_block(body_id) if isinstance(body_id, int) and body_id >= 0 else None
    shape_id = body.get_field("Shape") if body is not None else None
    if not isinstance(shape_id, int) or shape_id < 0:
        return []
    scale = havok_scale if havok_scale is not None else HAVOK_SCALE_FO4

    def _walk(
        current_id: int,
        transform: np.ndarray,
        visiting: set[int],
    ) -> list[CollisionShapeOverlay]:
        if current_id in visiting:
            return []
        visiting = set(visiting)
        visiting.add(current_id)
        block = nif.get_block(current_id)
        if block is None:
            return []
        type_name = block.type_name
        if type_name in ("bhkTransformShape", "bhkConvexTransformShape"):
            child_id = block.get_field("Shape")
            if not isinstance(child_id, int) or child_id < 0:
                return []
            child_transform = _legacy_shape_transform(
                block.get_field("Transform"), scale
            )
            return _walk(child_id, transform @ child_transform, visiting)
        if type_name == "bhkListShape":
            shapes = []
            for child_id in block.get_field("Sub Shapes") or []:
                if isinstance(child_id, int) and child_id >= 0:
                    shapes.extend(_walk(child_id, transform, visiting))
            return shapes
        if type_name == "bhkMoppBvTreeShape":
            child_id = block.get_field("Shape")
            if isinstance(child_id, int) and child_id >= 0:
                return _walk(child_id, transform, visiting)
            return []

        mesh = None
        if type_name == "bhkPackedNiTriStripsShape":
            mesh = _packed_tri_strips_mesh(nif, block, scale)
        elif type_name == "bhkConvexVerticesShape":
            mesh = _convex_shape_mesh(block, scale)
        elif type_name == "bhkBoxShape":
            dimensions = block.get_field("Dimensions") or {}
            mesh = box_mesh_from_half_extents(
                [
                    float(dimensions.get("x", 0.0)),
                    float(dimensions.get("y", 0.0)),
                    float(dimensions.get("z", 0.0)),
                ],
                scale,
            )
        elif type_name == "bhkSphereShape":
            mesh = sphere_mesh_from_center(
                [0.0, 0.0, 0.0],
                float(block.get_field("Radius") or 0.0),
                scale,
            )
        elif type_name == "bhkCapsuleShape":
            first = block.get_field("First Point") or {}
            second = block.get_field("Second Point") or {}
            radius = block.get_field("Radius")
            if radius is None:
                radius = block.get_field("Radius 1") or 0.0
            mesh = capsule_mesh_from_endpoints(
                [float(first.get(axis, 0.0)) for axis in ("x", "y", "z")],
                [float(second.get(axis, 0.0)) for axis in ("x", "y", "z")],
                float(radius),
                scale,
            )
        if mesh is None:
            return []
        overlay = _mesh_overlay(
            mesh,
            source_block_id=current_id,
            shape_type=type_name,
            transform=transform,
        )
        return [overlay] if overlay is not None else []

    return _walk(shape_id, _legacy_body_transform(nif, body, scale), set())


def _shapes_to_wireframe_lines(all_shapes: list) -> list | None:
    """Convert a list of vertex arrays to wireframe line pairs via native hulls."""
    all_lines = []
    for pts in all_shapes:
        try:
            from creation_lib.scientific.native_runtime import convex_hull_triangles

            triangles = convex_hull_triangles(pts)
            edges = set()
            for simplex in triangles:
                for k in range(len(simplex)):
                    a, b = simplex[k], simplex[(k + 1) % len(simplex)]
                    edge = (min(a, b), max(a, b))
                    if edge not in edges:
                        edges.add(edge)
                        all_lines.append(pts[a].tolist())
                        all_lines.append(pts[b].tolist())
        except ImportError:
            for k in range(len(pts)):
                all_lines.append(pts[k].tolist())
                all_lines.append(pts[(k + 1) % len(pts)].tolist())
        except Exception:
            continue
    return all_lines if all_lines else None


_HKX_MAGIC = b"\x57\xe0\xe0\x57\x10\xc0\xc0\x10"


def _parse_compound_instance_transforms(
    data: bytes,
    scale: float,
) -> list[tuple[np.ndarray, np.ndarray]] | None:
    """Parse per-instance transforms from hknpDynamicCompoundShape in packfile data.

    The bhkPhysicsSystem binary blob is a complete Havok packfile.  For compound
    collision shapes (hknpDynamicCompoundShape), each sub-shape stores vertices
    in local space.  The compound shape holds per-instance transforms (3×3
    rotation matrix + translation) that position each sub-shape correctly.

    Returns a list of ``(rotation_3x3, translation_nif)`` tuples (one per
    sub-shape), or ``None`` when no compound shape is present.
    """
    import json as _json
    import struct as _s

    if len(data) < 16 or data[:8] != _HKX_MAGIC:
        return None

    try:
        from creation_lib.havok.native_runtime import _require_native
        inspected = _json.loads(_require_native().hkx_inspect_packfile(bytes(data)))
    except Exception:
        return None

    sections = {s["name"]: s for s in inspected.get("sections", [])}
    data_section = sections.get("__data__")
    if data_section is None or "__classnames__" not in sections:
        return None

    ds = int(data_section["offset"])
    classnames = {int(c["position"]): c["name"] for c in inspected.get("classnames", [])}
    vfixups = inspected.get("virtual_fixups", [])
    lfixups = inspected.get("local_fixups", [])

    # Locate compound shape and count convex polytope sub-shapes
    compound_rel: int | None = None
    shape_count = 0
    obj_offsets = sorted(int(entry[0]) for entry in vfixups)

    for src, _section, cn_pos in vfixups:
        name = classnames.get(int(cn_pos))
        if name == "hknpDynamicCompoundShape":
            compound_rel = int(src)
        elif name == "hknpConvexPolytopeShape":
            shape_count += 1

    if compound_rel is None or shape_count < 2:
        return None

    # Bound the compound shape object (up to the next object)
    compound_end = len(data) - ds
    for off in obj_offsets:
        if off > compound_rel:
            compound_end = off
            break

    # Find the local fixup within the compound shape → instance data array
    inst_data_rel: int | None = None
    arr_member_rel: int | None = None
    for src, dst in sorted(lfixups, key=lambda f: (int(f[0]), int(f[1]))):
        src_i = int(src)
        if compound_rel < src_i < compound_end:
            inst_data_rel = int(dst)
            arr_member_rel = src_i
            break

    if inst_data_rel is None or arr_member_rel is None:
        return None

    # hkArray layout: ptr(8) + count(4) + capacity(4)
    count_pos = ds + arr_member_rel + 8
    if count_pos + 4 > len(data):
        return None
    arr_count = _s.unpack_from("<I", data, count_pos)[0]

    if arr_count != shape_count or arr_count == 0 or arr_count > 64:
        return None

    # Instance size: total space from instance data to next object / count
    inst_total = compound_end - inst_data_rel
    inst_size = inst_total // arr_count
    _MIN_INST = 0x40  # rotation(48) + translation(16) minimum
    if inst_size < _MIN_INST:
        return None

    transforms: list[tuple[np.ndarray, np.ndarray]] = []
    for i in range(arr_count):
        base = ds + inst_data_rel + i * inst_size
        if base + _MIN_INST > len(data):
            return None
        # 3×3 rotation matrix (3 rows stored as vec4, w component ignored)
        row0 = _s.unpack_from("<3f", data, base + 0x00)
        row1 = _s.unpack_from("<3f", data, base + 0x10)
        row2 = _s.unpack_from("<3f", data, base + 0x20)
        rot = np.array([row0, row1, row2], dtype=np.float32)
        # Translation (Havok units → NIF space)
        tx, ty, tz = _s.unpack_from("<3f", data, base + 0x30)
        trans = np.array([tx * scale, ty * scale, tz * scale], dtype=np.float32)
        transforms.append((rot, trans))

    return transforms


def _extract_np_collision_shapes(
    nif,
    coll_block,
    havok_scale: float | None = None,
) -> list[CollisionShapeOverlay]:
    from creation_lib.nif.operations.collision import HAVOK_SCALE_FO4

    data_id = coll_block.get_field("Data")
    if not isinstance(data_id, int) or data_id < 0:
        return []
    data_block = nif.get_block(data_id)
    if data_block is None or data_block.type_name != "bhkPhysicsSystem":
        return []
    binary = data_block.get_field("Binary Data")
    raw = binary.get("Data") if isinstance(binary, dict) else None
    if isinstance(raw, (bytes, bytearray)):
        blob = bytes(raw)
    elif isinstance(raw, list) and raw:
        try:
            blob = bytes(raw)
        except (TypeError, ValueError):
            return []
    else:
        return []

    body_id = coll_block.get_field("Body ID")
    try:
        body_id_int = int(body_id) if body_id is not None else 0
    except (TypeError, ValueError):
        body_id_int = 0
    scale = havok_scale if havok_scale is not None else HAVOK_SCALE_FO4
    try:
        previews = extract_preview_meshes_from_blob(
            blob,
            havok_scale=scale,
            body_id=body_id_int,
        )
    except Exception:
        return []

    shapes = []
    for shape_index, preview in enumerate(previews):
        mesh = preview.get("mesh") if isinstance(preview, dict) else None
        if not isinstance(mesh, dict):
            continue
        overlay = _mesh_overlay(
            mesh,
            source_block_id=data_id,
            body_id=body_id_int,
            shape_index=shape_index,
            shape_type=str(preview.get("shape_type") or "unknown"),
        )
        if overlay is not None:
            shapes.append(overlay)
    return shapes


def _extract_np_collision_lines(
    nif, coll_block, havok_scale: float | None = None
) -> list | None:
    """Extract wireframe from bhkNPCollisionObject → bhkPhysicsSystem binary data.

    Supports two Havok formats:
    - FO4/FO76 packfile: vertices as hkVector4 with w=0x3F0000XX index pattern
    - Starfield tagged (TAG0/SDKV2019+): vertices as hkFloat3 in ITEM entries
      (already in NIF-space, no Havok scale conversion needed)
    """
    import struct
    from creation_lib.nif.operations.collision import HAVOK_SCALE_FO4

    data_ref = coll_block.get_field("Data")
    if data_ref is None:
        return None
    data_id = data_ref if isinstance(data_ref, int) else -1
    if data_id < 0:
        return None
    data_block = nif.get_block(data_id)
    if not data_block or data_block.type_name != "bhkPhysicsSystem":
        return None

    binary_data_field = data_block.get_field("Binary Data")
    if not binary_data_field:
        return None

    raw = binary_data_field.get("Data") if isinstance(binary_data_field, dict) else None
    if not raw or not isinstance(raw, list):
        return None

    data = bytes(raw)

    preview_scale = havok_scale if havok_scale is not None else HAVOK_SCALE_FO4
    body_id = coll_block.get_field("Body ID")
    try:
        body_id_int = int(body_id) if body_id is not None else None
    except (TypeError, ValueError):
        body_id_int = None

    preview_meshes = extract_preview_meshes_from_blob(
        data,
        havok_scale=preview_scale,
        body_id=body_id_int,
    )
    if preview_meshes:
        all_lines = []
        for preview in preview_meshes:
            all_lines.extend(mesh_to_wireframe_lines(preview["mesh"]))
        if all_lines:
            return all_lines

    # Detect Havok tagged format (Starfield): TAG0 signature at bytes 4-7
    # Tagged format vertices are already in NIF-space (scale=1.0)
    if len(data) > 8 and data[4:8] == b"TAG0":
        tagged_scale = havok_scale if havok_scale is not None else 1.0
        return _extract_tagged_np_collision_lines(data, tagged_scale)

    # FO4/FO76 packfile: Havok units need conversion to NIF-space
    scale = havok_scale if havok_scale is not None else HAVOK_SCALE_FO4

    # FO4/FO76 packfile format: scan for w=0x3F0000XX vertex pattern
    all_shapes = []
    i = 0
    while i <= len(data) - 16:
        if (
            data[i + 15] == 0x3F
            and data[i + 14] == 0x00
            and data[i + 13] == 0x00
            and data[i + 12] == 0x00
        ):
            verts = []
            j = i
            expected_idx = 0
            while j <= len(data) - 16:
                if (
                    data[j + 15] == 0x3F
                    and data[j + 14] == 0x00
                    and data[j + 13] == 0x00
                    and data[j + 12] == expected_idx
                ):
                    x, y, z = struct.unpack_from("<fff", data, j)
                    if all(abs(c) < 50.0 for c in (x, y, z)):
                        verts.append([x * scale, y * scale, z * scale])
                        j += 16
                        expected_idx += 1
                        if expected_idx > 255:
                            break
                    else:
                        break
                else:
                    break
            if len(verts) >= 4:
                all_shapes.append(np.array(verts, dtype=np.float32))
                i = j
                continue
        i += 4

    # Apply compound shape instance transforms (rotation + translation)
    if len(all_shapes) >= 2:
        transforms = _parse_compound_instance_transforms(data, scale)
        if transforms and len(transforms) == len(all_shapes):
            for idx, (rot, trans) in enumerate(transforms):
                all_shapes[idx] = (all_shapes[idx] @ rot.T) + trans

    return _shapes_to_wireframe_lines(all_shapes)


def _extract_tagged_np_collision_lines(data: bytes, scale: float) -> list | None:
    """Extract collision vertices from Havok 2019+ tagged format (Starfield).

    The tagged format uses TAG0 container with DATA (raw object bytes) and
    ITEM entries (type, offset, count) referencing into DATA. Convex hull
    vertices are stored as hkFloat3 arrays (12-byte stride) in ITEM entries.
    """
    import struct

    # Walk TAG0 children to find DATA section bounds and ITEM entries
    data_start = data_end = 0
    item_offset = item_size = 0

    pos = 8  # skip TAG0 header (8 bytes)
    blob_end = len(data)
    while pos < blob_end - 8:
        hdr = struct.unpack_from(">I", data, pos)[0]
        tag_type = (hdr >> 24) & 0xFF
        tag_size = hdr & 0x00FFFFFF
        tag_name = data[pos + 4 : pos + 8]

        if tag_name == b"DATA":
            data_start = pos + 8
            data_end = pos + tag_size
        elif tag_name == b"ITEM":
            item_offset = pos + 8
            item_size = tag_size - 8

        if tag_type == 0x00:  # container — step into children
            pos += 8
        else:  # leaf — skip past content
            pos += tag_size

    if data_start == 0 or item_size == 0:
        return None

    data_section = data[data_start:data_end]
    data_section_len = len(data_section)

    # Parse ITEM entries (12 bytes each): type_encoded(4), data_offset(4), count(4)
    item_data = data[item_offset : item_offset + item_size]
    entries = []
    for i in range(0, len(item_data) - 11, 12):
        type_enc, d_offset, count = struct.unpack_from("<III", item_data, i)
        if count > 0 and d_offset < data_section_len:
            entries.append((type_enc, d_offset, count))

    if not entries:
        return None

    # Sort by offset to compute element sizes from gaps
    entries.sort(key=lambda x: x[1])

    all_shapes = []
    for idx, (type_enc, offset, count) in enumerate(entries):
        next_off = entries[idx + 1][1] if idx + 1 < len(entries) else data_section_len
        total_bytes = next_off - offset
        if count == 0:
            continue
        elem_size = total_bytes / count

        # hkFloat3 vertex arrays: 12-byte elements, 4+ vertices
        if abs(elem_size - 12.0) < 0.01 and count >= 4:
            verts = []
            valid = True
            for j in range(count):
                off = offset + j * 12
                if off + 12 > data_section_len:
                    valid = False
                    break
                x, y, z = struct.unpack_from("<fff", data_section, off)
                # Havok-scale coords: magnitude typically < 10
                mag_sq = x * x + y * y + z * z
                if mag_sq > 2500.0:  # > 50 units — not a vertex
                    valid = False
                    break
                verts.append([x * scale, y * scale, z * scale])
            if valid and len(verts) >= 4:
                all_shapes.append(np.array(verts, dtype=np.float32))

    return _shapes_to_wireframe_lines(all_shapes)


def _extract_shape_lines(
    nif, shape_id: int, havok_scale: float | None = None
) -> list | None:
    """Extract wireframe lines from a collision shape block. Returns list of [x,y,z] pairs."""
    block = nif.get_block(shape_id)
    if not block:
        return None

    from creation_lib.nif.operations.collision import HAVOK_SCALE_FO4

    scale = havok_scale if havok_scale is not None else HAVOK_SCALE_FO4

    if block.type_name == "bhkConvexVerticesShape":
        return _convex_shape_lines(block, scale)
    elif block.type_name == "bhkBoxShape":
        return _box_shape_lines(block, scale)
    elif block.type_name in ("bhkTransformShape", "bhkConvexTransformShape"):
        child_ref = block.get_field("Shape")
        child_id = child_ref if isinstance(child_ref, int) else -1
        if child_id < 0:
            return None
        child_block = nif.get_block(child_id)
        if not child_block:
            return None
        # Get transform matrix
        xf = block.get_field("Transform") or {}
        # Extract translation from transform (column 3 in row-major 4x4)
        tx = float(xf.get("m14", 0)) * scale
        ty = float(xf.get("m24", 0)) * scale
        tz = float(xf.get("m34", 0)) * scale
        offset = np.array([tx, ty, tz])

        if child_block.type_name == "bhkBoxShape":
            return _box_shape_lines(child_block, scale, offset)
        else:
            lines = _extract_shape_lines(nif, child_id, havok_scale=havok_scale)
            if lines and len(offset) > 0:
                # Apply offset
                result = []
                for pt in lines:
                    result.append(
                        [pt[0] + offset[0], pt[1] + offset[1], pt[2] + offset[2]]
                    )
                return result
            return lines
    elif block.type_name == "bhkListShape":
        sub_shapes = block.get_field("Sub Shapes") or []
        all_lines = []
        for ref in sub_shapes:
            ref_id = ref if isinstance(ref, int) else -1
            if ref_id >= 0:
                child_lines = _extract_shape_lines(nif, ref_id, havok_scale=havok_scale)
                if child_lines:
                    all_lines.extend(child_lines)
        return all_lines if all_lines else None

    return None


def _convex_shape_lines(block, scale: float) -> list:
    """Extract wireframe lines from bhkConvexVerticesShape using native hull edges."""
    verts_data = block.get_field("Vertices") or []
    if not verts_data:
        return []

    # Convert to NIF-space vertices
    pts = np.array(
        [
            [
                float(v.get("x", 0)) * scale,
                float(v.get("y", 0)) * scale,
                float(v.get("z", 0)) * scale,
            ]
            for v in verts_data
        ],
        dtype=np.float32,
    )

    if len(pts) < 4:
        return []

    try:
        from creation_lib.scientific.native_runtime import convex_hull_triangles

        triangles = convex_hull_triangles(pts)
        lines = []
        edges = set()
        for simplex in triangles:
            for i in range(len(simplex)):
                a, b = simplex[i], simplex[(i + 1) % len(simplex)]
                edge = (min(a, b), max(a, b))
                if edge not in edges:
                    edges.add(edge)
                    lines.append(pts[a].tolist())
                    lines.append(pts[b].tolist())
        return lines
    except ImportError:
        # Fallback: connect vertices in order as a simple wireframe
        lines = []
        for i in range(len(pts)):
            lines.append(pts[i].tolist())
            lines.append(pts[(i + 1) % len(pts)].tolist())
        return lines
    except Exception as exc:
        _log.debug("ConvexHull visualization failed: %s", exc)
        return []


def _box_shape_lines(block, scale: float, offset: np.ndarray | None = None) -> list:
    """Extract 12 box edges from bhkBoxShape Dimensions."""
    dims = block.get_field("Dimensions") or {}
    hx = float(dims.get("x", 0.5)) * scale
    hy = float(dims.get("y", 0.5)) * scale
    hz = float(dims.get("z", 0.5)) * scale

    ox, oy, oz = (
        (0.0, 0.0, 0.0) if offset is None else (offset[0], offset[1], offset[2])
    )

    # 8 corners of the box
    corners = [
        (ox - hx, oy - hy, oz - hz),
        (ox + hx, oy - hy, oz - hz),
        (ox + hx, oy + hy, oz - hz),
        (ox - hx, oy + hy, oz - hz),
        (ox - hx, oy - hy, oz + hz),
        (ox + hx, oy - hy, oz + hz),
        (ox + hx, oy + hy, oz + hz),
        (ox - hx, oy + hy, oz + hz),
    ]

    # 12 edges
    edge_indices = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),  # bottom
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),  # top
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),  # vertical
    ]

    lines = []
    for a, b in edge_indices:
        lines.append(list(corners[a]))
        lines.append(list(corners[b]))
    return lines


def rebuild_scene_from_nif(
    nif: NifFile,
    ctx: moderngl.Context,
    program: moderngl.Program,
    texture_dirs: list[Path],
    ba2_mgr=None,
    nif_id: str = "main",
    game_profile=None,
) -> SceneNode:
    """Rebuild the scene graph from an in-memory NifFile.

    Used by undo/redo and block operations to refresh the viewport.
    """
    root = SceneNode(name="nif_root", block_id=-1, nif_id=nif_id)
    game_id = game_profile.id if game_profile else "fo4"
    if nif.blocks:
        child = _convert_block(
            nif,
            nif.blocks[0],
            ctx,
            program,
            texture_dirs,
            ba2_mgr,
            nif_id=nif_id,
            game_id=game_id,
        )
        if child:
            root.children.append(child)
    _update_world_transforms(root, glm.mat4(1.0))
    return root


def load_nif_to_scene(
    filepath: str,
    ctx: moderngl.Context,
    program: moderngl.Program,
    texture_dirs: list[Path],
    ba2_mgr=None,
    nif_id: str = "main",
    game_profile=None,
) -> tuple[SceneNode, NifFile]:
    """Load a NIF file and return (scene_root, nif_file).

    Backward-compatible synchronous wrapper around prepare_nif_data + upload_nif_to_gpu.
    Used by attach_nif() and file-watcher reloads that remain synchronous.
    """
    prepared = prepare_nif_data(
        filepath, texture_dirs, ba2_mgr, nif_id, game_profile=game_profile
    )
    return upload_nif_to_gpu(prepared, ctx, program)


def _convert_block(
    nif: NifFile,
    block,
    ctx: moderngl.Context,
    program: moderngl.Program,
    texture_dirs: list[Path],
    ba2_mgr=None,
    nif_id: str = "main",
    game_id: str = "fo4",
) -> SceneNode | None:
    """Convert a NIF block to a SceneNode (recursive)."""
    schema = nif.schema
    type_name = block.type_name

    if schema.is_subtype_of(type_name, "NiNode"):
        return _convert_ninode(
            nif,
            block,
            ctx,
            program,
            texture_dirs,
            ba2_mgr,
            nif_id=nif_id,
            game_id=game_id,
        )

    if _is_tri_based_shape(schema, type_name):
        return _convert_shape(
            nif,
            block,
            ctx,
            program,
            texture_dirs,
            ba2_mgr,
            nif_id=nif_id,
            game_id=game_id,
        )

    return None


def _convert_ninode(
    nif: NifFile,
    block,
    ctx: moderngl.Context,
    program: moderngl.Program,
    texture_dirs: list[Path],
    ba2_mgr=None,
    nif_id: str = "main",
    game_id: str = "fo4",
) -> SceneNode:
    """Convert NiNode to a SceneNode, recursing into children."""
    name = _get_string(block, "Name") or f"NiNode_{block.block_id}"
    node = SceneNode(name=name, block_id=block.block_id, nif_id=nif_id)

    # Apply transform
    trans = block.get_field("Translation") or {}
    rot = block.get_field("Rotation") or {}
    scale = block.get_field("Scale")
    if scale is None:
        scale = 1.0
    node.transform = nif_transform_to_mat4(trans, rot, float(scale))

    # Recurse into Children refs
    children_refs = block.get_field("Children")
    if children_refs:
        for ref in children_refs:
            ref_id = _get_ref_id(ref)
            if ref_id >= 0:
                child_block = nif.get_block(ref_id)
                if child_block:
                    child = _convert_block(
                        nif,
                        child_block,
                        ctx,
                        program,
                        texture_dirs,
                        ba2_mgr,
                        nif_id=nif_id,
                        game_id=game_id,
                    )
                    if child:
                        node.children.append(child)

    # Check for collision overlay
    try:
        overlay = _extract_collision_overlay(nif, block)
        if overlay:
            node.collision_overlay = overlay
    except Exception as exc:
        _log.debug(
            "Collision overlay extraction failed for block %d: %s", block.block_id, exc
        )

    return node


def _convert_shape(
    nif: NifFile,
    block,
    ctx: moderngl.Context,
    program: moderngl.Program,
    texture_dirs: list[Path],
    ba2_mgr=None,
    nif_id: str = "main",
    game_id: str = "fo4",
) -> SceneNode | None:
    """Convert BSTriShape to a SceneNode with Mesh."""
    ps = _extract_shape_data(nif, block, nif_id=nif_id)
    if ps is None:
        return None

    if ps.texture_slices:
        return _build_texture_slice_node(
            ps,
            nif,
            block,
            ctx,
            program,
            texture_dirs,
            ba2_mgr,
            nif_id,
            game_id,
        )

    # Build mesh
    mesh = _build_mesh(
        ctx,
        program,
        ps.verts,
        ps.normals,
        ps.uvs,
        ps.tris,
        colors=ps.colors,
        tangents=ps.tangents,
        bitangents=ps.bitangents,
        uv2=ps.uv2,
    )

    node = SceneNode(name=ps.name, block_id=ps.block_id, nif_id=nif_id, mesh=mesh)
    node._local_verts = ps.verts
    node._local_tris = ps.tris
    node.transform = ps.transform

    # Apply material
    from .material_pipeline import build_material

    mesh.material = build_material(
        ctx, nif, block, texture_dirs, ba2_mgr, game_id=game_id
    )

    return node


def _build_texture_slice_node(
    ps: PreparedShape,
    nif,
    block,
    ctx,
    program,
    texture_dirs,
    ba2_mgr,
    nif_id: str,
    game_id: str,
) -> SceneNode | None:
    from .material_pipeline import build_material

    parent = SceneNode(name=ps.name, block_id=ps.block_id, nif_id=nif_id)
    parent.transform = ps.transform
    for part in ps.texture_slices or []:
        mesh = _build_mesh(
            ctx,
            program,
            part.verts,
            part.normals,
            part.uvs,
            part.tris,
            colors=part.colors,
            tangents=part.tangents,
            bitangents=part.bitangents,
            uv2=part.uv2,
        )
        mesh.material = build_material(
            ctx,
            nif,
            block,
            texture_dirs,
            ba2_mgr,
            game_id=game_id,
            texture_paths_override=part.texture_paths,
        )
        child = SceneNode(
            name=f"{ps.name} [texture {part.slice_index}]",
            block_id=ps.block_id,
            nif_id=nif_id,
            mesh=mesh,
        )
        child._local_verts = part.verts
        child._local_tris = part.tris
        parent.children.append(child)
    return parent if parent.children else None


def _build_mesh(
    ctx: moderngl.Context,
    program: moderngl.Program,
    verts: np.ndarray,
    normals: np.ndarray,
    uvs: np.ndarray,
    tris: np.ndarray,
    colors: np.ndarray | None = None,
    tangents: np.ndarray | None = None,
    bitangents: np.ndarray | None = None,
    uv2: np.ndarray | None = None,
) -> Mesh:
    """Build a ModernGL Mesh from numpy arrays."""
    data, fmt, attrs = interleave_vertex_data(
        verts, normals, uvs, colors, tangents, bitangents, uv2=uv2
    )
    vbo = ctx.buffer(data.tobytes())
    ibo = ctx.buffer(tris.astype(np.uint32).tobytes())

    # Filter attrs to only include those the program actually has
    content = []
    fmt_parts = fmt.split()
    offset_parts = []
    for f_part, attr_name in zip(fmt_parts, attrs):
        if attr_name in program:
            offset_parts.append(f_part)
        else:
            # Use padding format to skip this attribute
            # Convert float count to byte padding (e.g. "3f" = 12 bytes -> "12x")
            n_floats = int(f_part[:-1])
            offset_parts.append(f"{n_floats * 4}x")

    vao = ctx.vertex_array(
        program,
        [(vbo, " ".join(offset_parts), *[a for a in attrs if a in program])],
        index_buffer=ibo,
    )
    return Mesh(
        vao=vao,
        vbo=vbo,
        ibo=ibo,
        num_indices=tris.size,
        material=Material(),
        vbo_format=fmt,
        vbo_attrs=attrs,
    )


def _get_string(block, field_name: str) -> str | None:
    """Extract a string from a NIF field (may be str or list of chars)."""
    val = block.get_field(field_name)
    if val is None:
        return None
    if isinstance(val, str):
        return val if val else None
    if isinstance(val, list):
        return "".join(str(c) for c in val) or None
    return str(val)


def _get_ref_id(ref) -> int:
    """Extract block index from a reference value."""
    if isinstance(ref, (int, float)):
        return int(ref)
    if isinstance(ref, dict):
        return int(ref.get("value", ref.get("Value", -1)))
    return -1


def _extract_external_geometry_info(block) -> list[str]:
    """Extract external mesh paths from a BSGeometry block's Meshes array.

    Starfield BSGeometry blocks have up to 4 BSMeshArray entries. Each may contain
    a BSMesh with either a Mesh Path (external) or Mesh Data (inline). Returns
    the list of external mesh paths found.
    """
    meshes = block.get_field("Meshes")
    if not meshes:
        return []
    paths = []
    for entry in meshes:
        if not isinstance(entry, dict):
            continue
        if not entry.get("Has Mesh"):
            continue
        mesh = entry.get("Mesh")
        if not isinstance(mesh, dict):
            continue
        mesh_path = mesh.get("Mesh Path")
        if mesh_path and isinstance(mesh_path, str) and mesh_path.strip():
            paths.append(mesh_path.strip())
    return paths


def _make_bbox_placeholder(block, external_paths: list[str]) -> "PreparedShape":
    """Create a bounding box placeholder PreparedShape for a BSGeometry with external geometry.

    Uses BSBoundingBox if available, falls back to BoundingSphere radius, then a unit box.
    """
    name = _get_string(block, "Name") or f"BSGeometry_{block.block_id}"

    # Determine box extents
    bbox = block.get_field("Bounding Box")
    bsphere = block.get_field("Bounding Sphere")

    if bbox and isinstance(bbox, dict):
        center = bbox.get("Center", {})
        dims = bbox.get("Dimensions", {})
        cx = float(center.get("x", 0))
        cy = float(center.get("y", 0))
        cz = float(center.get("z", 0))
        hx = float(dims.get("x", 1))
        hy = float(dims.get("y", 1))
        hz = float(dims.get("z", 1))
    elif bsphere and isinstance(bsphere, dict):
        center = bsphere.get("Center", {})
        cx = float(center.get("x", 0))
        cy = float(center.get("y", 0))
        cz = float(center.get("z", 0))
        r = float(bsphere.get("Radius", 1.0))
        hx = hy = hz = r
    else:
        cx = cy = cz = 0.0
        hx = hy = hz = 1.0

    # 8 corners of the bounding box
    verts = np.array(
        [
            [cx - hx, cy - hy, cz - hz],
            [cx + hx, cy - hy, cz - hz],
            [cx + hx, cy + hy, cz - hz],
            [cx - hx, cy + hy, cz - hz],
            [cx - hx, cy - hy, cz + hz],
            [cx + hx, cy - hy, cz + hz],
            [cx + hx, cy + hy, cz + hz],
            [cx - hx, cy + hy, cz + hz],
        ],
        dtype=np.float32,
    )

    # 12 triangles (2 per face, 6 faces)
    tris = np.array(
        [
            [0, 1, 2],
            [0, 2, 3],  # -Z face
            [4, 6, 5],
            [4, 7, 6],  # +Z face
            [0, 4, 5],
            [0, 5, 1],  # -Y face
            [2, 6, 7],
            [2, 7, 3],  # +Y face
            [0, 3, 7],
            [0, 7, 4],  # -X face
            [1, 5, 6],
            [1, 6, 2],  # +X face
        ],
        dtype=np.uint32,
    )

    normals = np.zeros((8, 3), dtype=np.float32)
    uvs = np.zeros((8, 2), dtype=np.float32)

    trans = block.get_field("Translation") or {}
    rot = block.get_field("Rotation") or {}
    blk_scale = block.get_field("Scale") or 1.0
    transform = nif_transform_to_mat4(trans, rot, float(blk_scale))

    return PreparedShape(
        block_id=block.block_id,
        name=name,
        verts=verts,
        normals=normals,
        uvs=uvs,
        tris=tris,
        colors=None,
        tangents=None,
        bitangents=None,
        transform=transform,
        material_inputs={},
        external_mesh_paths=external_paths,
    )


def _resolve_sf_mesh_file(
    mesh_path: str, texture_dirs: list, ba2_mgr=None, nif_path: str = None
) -> bytes | None:
    """Resolve a Starfield mesh hash path to file contents.

    Searches texture_dirs for geometries/<hash1>/<hash2>.mesh,
    walks up from the NIF's directory to find a geometries/ sibling,
    then falls back to BA2 archive lookup.
    """
    normalized = mesh_path.replace("\\", "/")
    rel_path = f"geometries/{normalized}.mesh"

    # Search texture_dirs
    for d in texture_dirs:
        candidate = d / rel_path
        if candidate.exists():
            try:
                return candidate.read_bytes()
            except OSError as e:
                _log.debug("Failed to read mesh file %s: %s", candidate, e)

    # Walk up from NIF directory looking for a geometries/ sibling
    if nif_path:
        p = Path(nif_path).parent
        for _ in range(8):
            candidate = p / rel_path
            if candidate.exists():
                try:
                    return candidate.read_bytes()
                except OSError as e:
                    _log.debug("Failed to read mesh file %s: %s", candidate, e)
            p = p.parent
            if p == p.parent:
                break

    # Fallback: BA2 archive
    if ba2_mgr is not None:
        try:
            data = ba2_mgr.find(rel_path)
            if data:
                return data
        except Exception as e:
            _log.debug("BA2 mesh lookup failed for %s: %s", rel_path, e)

    return None


def _load_sf_geometry(
    nif, block, ext_paths: list[str], ctx: _PrepareContext
) -> PreparedShape | None:
    """Try to load actual geometry from external .mesh files for a BSGeometry block."""
    from .sf_mesh_loader import parse_sf_mesh

    mesh_data_bytes = _resolve_sf_mesh_file(
        ext_paths[0], ctx.texture_dirs, ctx.ba2_mgr, nif_path=ctx.nif_path
    )
    if mesh_data_bytes is None:
        _log.debug("Could not resolve Starfield mesh: %s", ext_paths[0])
        return None

    mesh = parse_sf_mesh(mesh_data_bytes)
    if mesh is None:
        _log.warning("Failed to parse Starfield mesh: %s", ext_paths[0])
        return None

    name = _get_string(block, "Name") or f"BSGeometry_{block.block_id}"
    trans = block.get_field("Translation") or {}
    rot = block.get_field("Rotation") or {}
    blk_scale = block.get_field("Scale") or 1.0
    transform = nif_transform_to_mat4(trans, rot, float(blk_scale))

    # Starfield vertex colors are blend/mask data for multi-layer materials,
    # not literal tint colors. Always use white so albedo isn't darkened.
    # The .mat layer system handles tinting via sfLayerTint uniforms.
    colors = np.ones((mesh.positions.shape[0], 4), dtype=np.float32)

    return PreparedShape(
        block_id=block.block_id,
        name=name,
        verts=mesh.positions,
        normals=mesh.normals,
        uvs=mesh.uvs,
        tris=mesh.triangles,
        colors=colors,
        tangents=mesh.tangents,
        bitangents=mesh.bitangents,
        transform=transform,
        material_inputs={},
        external_mesh_paths=ext_paths,
        uv2=mesh.uv2,
    )


def _prepared_shape_from_arrays(
    nif,
    block,
    verts,
    normals,
    uvs,
    tris,
    *,
    colors=None,
    tangents=None,
    bitangents=None,
):
    name = _get_string(block, "Name") or f"Shape_{block.block_id}"
    trans = block.get_field("Translation") or {}
    rot = block.get_field("Rotation") or {}
    blk_scale = block.get_field("Scale") or 1.0
    transform = nif_transform_to_mat4(trans, rot, float(blk_scale))
    return PreparedShape(
        block_id=block.block_id,
        name=name,
        verts=verts,
        normals=normals,
        uvs=uvs,
        tris=tris,
        colors=colors,
        tangents=tangents,
        bitangents=bitangents,
        transform=transform,
        material_inputs={},
    )


def _legacy_skin_correction_transform(nif, block) -> "glm.mat4 | None":
    skin_ref = block.get_field("Skin Instance")
    if _get_ref_id(skin_ref) < 0:
        skin_ref = block.get_field("Skin")
    skin_id = _get_ref_id(skin_ref)
    if skin_id < 0:
        return None

    skin_block = nif.get_block(skin_id)
    if skin_block is None:
        return None

    skin_data_id = _get_ref_id(skin_block.get_field("Data"))
    if skin_data_id < 0:
        return None

    skin_data_block = nif.get_block(skin_data_id)
    if skin_data_block is None:
        return None

    skin_transform = skin_data_block.get_field("Skin Transform")
    if not isinstance(skin_transform, dict):
        return None

    trans = block.get_field("Translation") or {}
    rot = block.get_field("Rotation") or {}
    blk_scale = block.get_field("Scale")
    shape_local = nif_transform_to_mat4(
        trans,
        rot,
        float(blk_scale) if blk_scale is not None else 1.0,
    )
    skin_space = nif_transform_to_mat4(
        skin_transform.get("Translation") or {},
        skin_transform.get("Rotation") or {},
        float(skin_transform.get("Scale", 1.0) or 1.0),
    )
    inverse_pair = _mat4_to_numpy(shape_local * skin_space)
    if np.allclose(
        inverse_pair,
        np.eye(4, dtype=np.float32),
        rtol=1e-4,
        atol=1e-4,
    ):
        return None
    return glm.inverse(shape_local) * skin_space


def _mat4_to_numpy(mat: glm.mat4) -> np.ndarray:
    return np.array(
        [
            [float(mat[0][0]), float(mat[1][0]), float(mat[2][0]), float(mat[3][0])],
            [float(mat[0][1]), float(mat[1][1]), float(mat[2][1]), float(mat[3][1])],
            [float(mat[0][2]), float(mat[1][2]), float(mat[2][2]), float(mat[3][2])],
            [float(mat[0][3]), float(mat[1][3]), float(mat[2][3]), float(mat[3][3])],
        ],
        dtype=np.float32,
    )


def _transform_positions(positions: np.ndarray, transform: glm.mat4) -> np.ndarray:
    matrix = _mat4_to_numpy(transform)
    hom = np.ones((len(positions), 4), dtype=np.float32)
    hom[:, :3] = positions
    return (hom @ matrix.T)[:, :3].astype(np.float32, copy=False)


def _transform_directions(directions: np.ndarray, transform: glm.mat4) -> np.ndarray:
    matrix = _mat4_to_numpy(transform)[:3, :3]
    rotated = (directions @ matrix.T).astype(np.float32, copy=False)
    lengths = np.linalg.norm(rotated, axis=1, keepdims=True)
    nonzero = lengths[:, 0] > 1e-8
    rotated[nonzero] /= lengths[nonzero]
    return rotated


def _extract_legacy_shape_data(nif, block) -> "PreparedShape | None":
    data_ref = block.get_field("Data")
    data_id = _get_ref_id(data_ref)
    if data_id < 0:
        return None
    data_block = nif.get_block(data_id)
    if data_block is None:
        return None

    vertices = data_block.get_field("Vertices") or []
    if not vertices:
        return None

    verts = np.array(
        [
            [float(v.get("x", 0)), float(v.get("y", 0)), float(v.get("z", 0))]
            for v in vertices
        ],
        dtype=np.float32,
    )
    normals_list = data_block.get_field("Normals") or []
    has_normals = bool(data_block.get_field("Has Normals") and normals_list)
    if has_normals:
        normals = np.array(
            [
                [float(n.get("x", 0)), float(n.get("y", 0)), float(n.get("z", 1))]
                for n in normals_list
            ],
            dtype=np.float32,
        )
    else:
        normals = np.zeros((len(verts), 3), dtype=np.float32)

    uv_sets = data_block.get_field("UV Sets") or []
    first_uv_set = uv_sets[0] if uv_sets else []
    if first_uv_set:
        uvs = np.array(
            [[float(uv.get("u", 0)), float(uv.get("v", 0))] for uv in first_uv_set],
            dtype=np.float32,
        )
    else:
        uvs = np.zeros((len(verts), 2), dtype=np.float32)

    colors_list = data_block.get_field("Vertex Colors") or []
    if data_block.get_field("Has Vertex Colors") and colors_list:
        colors = np.array(
            [
                [
                    float(color.get("r", 1.0)),
                    float(color.get("g", 1.0)),
                    float(color.get("b", 1.0)),
                    float(color.get("a", 1.0)),
                ]
                for color in colors_list
            ],
            dtype=np.float32,
        )
        if colors.size and np.max(colors) > 1.0:
            colors /= 255.0
    else:
        colors = np.ones((len(verts), 4), dtype=np.float32)

    tangents_list = data_block.get_field("Tangents") or []
    bitangents_list = data_block.get_field("Bitangents") or []
    tangents = None
    bitangents = None
    if tangents_list and bitangents_list:
        tangents = np.array(
            [
                [float(t.get("x", 0)), float(t.get("y", 0)), float(t.get("z", 0))]
                for t in tangents_list
            ],
            dtype=np.float32,
        )
        bitangents = np.array(
            [
                [float(bt.get("x", 0)), float(bt.get("y", 0)), float(bt.get("z", 0))]
                for bt in bitangents_list
            ],
            dtype=np.float32,
        )

    if nif.schema.is_subtype_of(block.type_name, "NiTriStrips"):
        tris = _triangles_from_strips(
            [int(v or 0) for v in (data_block.get_field("Strip Lengths") or [])],
            data_block.get_field("Points") or [],
        )
    else:
        tris = np.array(
            [
                (int(t.get("v1", 0)), int(t.get("v2", 0)), int(t.get("v3", 0)))
                for t in (data_block.get_field("Triangles") or [])
            ],
            dtype=np.uint32,
        )

    if len(tris) == 0:
        return None

    correction = _legacy_skin_correction_transform(nif, block)
    if correction is not None:
        verts = _transform_positions(verts, correction)
        if has_normals:
            normals = _transform_directions(normals, correction)

    if not has_normals:
        normals = compute_normals(verts, tris)

    return _prepared_shape_from_arrays(
        nif,
        block,
        verts,
        normals,
        uvs,
        tris,
        colors=colors,
        tangents=tangents,
        bitangents=bitangents,
    )


def _extract_modern_shape_data(
    nif, block, nif_id: str = "main"
) -> "PreparedShape | None":
    """Extract vertex/triangle data from a BSTriShape block into a PreparedShape.

    No GL calls. Safe to run on a background thread.
    """
    vertex_data_list = block.get_field("Vertex Data") or []
    triangles_list = block.get_field("Triangles") or []
    if not vertex_data_list or not triangles_list:
        return None

    n_verts = len(vertex_data_list)

    verts = np.zeros((n_verts, 3), dtype=np.float32)
    normals = np.zeros((n_verts, 3), dtype=np.float32)
    uvs = np.zeros((n_verts, 2), dtype=np.float32)
    colors = np.ones((n_verts, 4), dtype=np.float32)
    tangents_arr = np.zeros((n_verts, 3), dtype=np.float32)
    bitangents_arr = np.zeros((n_verts, 3), dtype=np.float32)
    has_normals = has_uvs = has_colors = has_tangents = False

    for i, vd in enumerate(vertex_data_list):
        v = vd.get("Vertex") or {}
        verts[i] = [float(v.get("x", 0)), float(v.get("y", 0)), float(v.get("z", 0))]
        n = vd.get("Normal")
        if n:
            normals[i] = [
                float(n.get("x", 0)),
                float(n.get("y", 0)),
                float(n.get("z", 0)),
            ]
            has_normals = True
        uv = vd.get("UV")
        if uv:
            uvs[i] = [float(uv.get("u", 0)), float(uv.get("v", 0))]
            has_uvs = True
        vc = vd.get("Vertex Colors")
        if vc:
            r, g, b, a = (
                float(vc.get("r", 255)),
                float(vc.get("g", 255)),
                float(vc.get("b", 255)),
                float(vc.get("a", 255)),
            )
            scale = 1.0 / 255.0 if max(r, g, b, a) > 1.0 else 1.0
            colors[i] = [r * scale, g * scale, b * scale, a * scale]
            has_colors = True
        t = vd.get("Tangent")
        if t and isinstance(t, dict):
            tangents_arr[i] = [
                float(t.get("x", 0)),
                float(t.get("y", 0)),
                float(t.get("z", 0)),
            ]
            bt = vd.get("Bitangent")
            if bt and isinstance(bt, dict):
                bitangents_arr[i] = [
                    float(bt.get("x", 0)),
                    float(bt.get("y", 0)),
                    float(bt.get("z", 0)),
                ]
            else:
                bitangents_arr[i] = [
                    float(vd.get("Bitangent X", 0)),
                    float(vd.get("Bitangent Y", 0)),
                    float(vd.get("Bitangent Z", 0)),
                ]
            has_tangents = True

    tris = np.array(
        [
            (int(t.get("v1", 0)), int(t.get("v2", 0)), int(t.get("v3", 0)))
            for t in triangles_list
        ],
        dtype=np.uint32,
    )

    if not has_normals:
        normals = compute_normals(verts, tris)

    prepared = _prepared_shape_from_arrays(
        nif,
        block,
        verts,
        normals,
        uvs,
        tris,
        colors=colors,
        tangents=tangents_arr if has_tangents else None,
        bitangents=bitangents_arr if has_tangents else None,
    )
    prepared.texture_slices = _split_fo76_texture_array_shape(nif, block, prepared)
    return prepared


def _split_fo76_texture_array_shape(
    nif, block, prepared: PreparedShape
) -> list[PreparedTextureSlice] | None:
    if prepared.bitangents is None or len(prepared.tris) == 0:
        return None

    from .material_pipeline import (
        _get_fo76_texture_array_paths,
        _get_fo76_texture_arrays,
        _get_shape_property_block,
    )

    shader_prop = _get_shape_property_block(
        nif, block, "Shader Property", "BSShaderProperty"
    )
    if shader_prop is None or not _get_fo76_texture_arrays(shader_prop):
        return None

    encoded_bitangents = prepared.bitangents
    vertex_slices = encoded_bitangents[:, 0]
    triangle_slices = np.rint(vertex_slices[prepared.tris].mean(axis=1)).astype(
        np.int64
    )
    reconstructed_bitangents = None
    if prepared.tangents is not None:
        reconstructed_bitangents = np.cross(prepared.normals, prepared.tangents)
        orientation = np.einsum(
            "ij,ij->i",
            reconstructed_bitangents[:, 1:],
            encoded_bitangents[:, 1:],
        )
        reconstructed_bitangents[orientation < 0.0] *= -1.0
        lengths = np.linalg.norm(reconstructed_bitangents, axis=1)
        valid = lengths > 1e-8
        reconstructed_bitangents[valid] /= lengths[valid, None]

    parts = []
    for slice_index in np.unique(triangle_slices):
        texture_paths = _get_fo76_texture_array_paths(
            shader_prop, int(slice_index)
        )
        if not texture_paths.get("diffuse"):
            continue
        slice_tris = prepared.tris[triangle_slices == slice_index]
        vertex_ids, remapped = np.unique(
            slice_tris.reshape(-1), return_inverse=True
        )
        parts.append(
            PreparedTextureSlice(
                slice_index=int(slice_index),
                verts=prepared.verts[vertex_ids],
                normals=prepared.normals[vertex_ids],
                uvs=prepared.uvs[vertex_ids],
                tris=remapped.reshape(-1, 3).astype(np.uint32),
                colors=(
                    prepared.colors[vertex_ids]
                    if prepared.colors is not None
                    else None
                ),
                tangents=(
                    prepared.tangents[vertex_ids]
                    if prepared.tangents is not None
                    else None
                ),
                bitangents=(
                    reconstructed_bitangents[vertex_ids]
                    if reconstructed_bitangents is not None
                    else None
                ),
                texture_paths=texture_paths,
                uv2=prepared.uv2[vertex_ids] if prepared.uv2 is not None else None,
            )
        )
    return parts or None


def _extract_shape_data(nif, block, nif_id: str = "main") -> "PreparedShape | None":
    if _uses_legacy_shape_data(nif.schema, block):
        return _extract_legacy_shape_data(nif, block)
    return _extract_modern_shape_data(nif, block, nif_id=nif_id)


def _prepare_walk_blocks(nif, block, shapes: dict, ctx: _PrepareContext):
    """Recursively walk blocks and populate shapes dict with PreparedShape per BSTriShape."""
    schema = nif.schema
    type_name = block.type_name

    if _is_tri_based_shape(schema, type_name):
        ps = _extract_shape_data(nif, block, ctx.nif_id)
        if ps:
            shapes[block.block_id] = ps

    elif type_name == "BSGeometry":
        ext_paths = _extract_external_geometry_info(block)
        if ext_paths:
            ps = _load_sf_geometry(nif, block, ext_paths, ctx)
            if ps is None:
                ps = _make_bbox_placeholder(block, ext_paths)
            shapes[block.block_id] = ps

    elif schema.is_subtype_of(type_name, "NiNode"):
        children_refs = block.get_field("Children") or []
        for ref in children_refs:
            ref_id = _get_ref_id(ref)
            if ref_id >= 0:
                child = nif.get_block(ref_id)
                if child:
                    _prepare_walk_blocks(nif, child, shapes, ctx)


_LOD_BATCH_SHAPE_THRESHOLD = 512


def _should_prepare_lod_batches(filepath: str, shapes: dict[int, PreparedShape]) -> bool:
    return (
        Path(filepath).suffix.lower() == ".bto"
        and len(shapes) >= _LOD_BATCH_SHAPE_THRESHOLD
        and not any(shape.texture_slices for shape in shapes.values())
    )


def _freeze_signature_value(value):
    if isinstance(value, dict):
        return tuple(sorted((k, _freeze_signature_value(v)) for k, v in value.items()))
    if isinstance(value, (list, tuple)):
        return tuple(_freeze_signature_value(v) for v in value)
    if isinstance(value, float):
        return round(value, 6)
    return value


def _block_signature(block) -> tuple:
    if block is None:
        return ()
    return tuple(
        (name, _freeze_signature_value(value))
        for name, value in block.fields
        if name not in {"Controller", "Extra Data List", "Num Extra Data List"}
    )


def _lod_batch_signature(nif, shape_block) -> tuple:
    shader_id = _get_ref_id(shape_block.get_field("Shader Property"))
    alpha_id = _get_ref_id(shape_block.get_field("Alpha Property"))
    shader = nif.get_block(shader_id)
    alpha = nif.get_block(alpha_id)
    return (
        shader.type_name if shader else "",
        _block_signature(shader),
        alpha.type_name if alpha else "",
        _block_signature(alpha),
    )


def _iter_shape_world_transforms(nif, block, shapes, parent_world: glm.mat4):
    schema = nif.schema
    type_name = block.type_name

    if schema.is_subtype_of(type_name, "NiNode"):
        trans = block.get_field("Translation") or {}
        rot = block.get_field("Rotation") or {}
        scale = block.get_field("Scale")
        local = nif_transform_to_mat4(
            trans, rot, float(scale) if scale is not None else 1.0
        )
        world = parent_world * local
        for ref in block.get_field("Children") or []:
            ref_id = _get_ref_id(ref)
            child = nif.get_block(ref_id)
            if child:
                yield from _iter_shape_world_transforms(nif, child, shapes, world)
        return

    if _is_tri_based_shape(schema, type_name):
        ps = shapes.get(block.block_id)
        if ps is not None:
            yield block, ps, parent_world * ps.transform
        return

    if type_name == "BSGeometry":
        ps = shapes.get(block.block_id)
        if ps is not None:
            yield block, ps, parent_world * ps.transform


def _combine_batch_arrays(items: list[tuple[object, PreparedShape, glm.mat4]]):
    verts_parts = []
    normals_parts = []
    uvs_parts = []
    tris_parts = []
    colors_parts = []
    tangents_parts = []
    bitangents_parts = []
    uv2_parts = []

    use_colors = all(ps.colors is not None for _block, ps, _world in items)
    use_tangents = all(ps.tangents is not None for _block, ps, _world in items)
    use_bitangents = all(ps.bitangents is not None for _block, ps, _world in items)
    use_uv2 = all(ps.uv2 is not None for _block, ps, _world in items)

    vert_offset = 0
    for _block, ps, world in items:
        verts_parts.append(_transform_positions(ps.verts, world))
        normals_parts.append(_transform_directions(ps.normals, world))
        uvs_parts.append(ps.uvs)
        tris_parts.append(ps.tris + np.uint32(vert_offset))
        if use_colors:
            colors_parts.append(ps.colors)
        if use_tangents:
            tangents_parts.append(_transform_directions(ps.tangents, world))
        if use_bitangents:
            bitangents_parts.append(_transform_directions(ps.bitangents, world))
        if use_uv2:
            uv2_parts.append(ps.uv2)
        vert_offset += len(ps.verts)

    return (
        np.concatenate(verts_parts).astype(np.float32, copy=False),
        np.concatenate(normals_parts).astype(np.float32, copy=False),
        np.concatenate(uvs_parts).astype(np.float32, copy=False),
        np.concatenate(tris_parts).astype(np.uint32, copy=False),
        np.concatenate(colors_parts).astype(np.float32, copy=False) if use_colors else None,
        np.concatenate(tangents_parts).astype(np.float32, copy=False) if use_tangents else None,
        np.concatenate(bitangents_parts).astype(np.float32, copy=False) if use_bitangents else None,
        np.concatenate(uv2_parts).astype(np.float32, copy=False) if use_uv2 else None,
    )


def _prepare_lod_render_batches(nif, shapes: dict[int, PreparedShape]) -> list[PreparedRenderBatch]:
    if not nif.blocks:
        return []

    groups = defaultdict(list)
    for shape_block, ps, world in _iter_shape_world_transforms(
        nif, nif.blocks[0], shapes, glm.mat4(1.0)
    ):
        groups[_lod_batch_signature(nif, shape_block)].append((shape_block, ps, world))

    batches: list[PreparedRenderBatch] = []
    for index, items in enumerate(groups.values(), start=1):
        (
            verts,
            normals,
            uvs,
            tris,
            colors,
            tangents,
            bitangents,
            uv2,
        ) = _combine_batch_arrays(items)
        representative = items[0][0]
        source_ids = tuple(int(block.block_id) for block, _ps, _world in items)
        batches.append(
            PreparedRenderBatch(
                name=f"BTO batch {index} ({len(items)} shapes)",
                block_id=int(representative.block_id),
                material_shape_id=int(representative.block_id),
                source_block_ids=source_ids,
                verts=verts,
                normals=normals,
                uvs=uvs,
                tris=tris,
                colors=colors,
                tangents=tangents,
                bitangents=bitangents,
                uv2=uv2,
            )
        )
    return batches


from concurrent.futures import ThreadPoolExecutor, as_completed
from .material_pipeline import (
    collect_nif_texture_paths,
    _decode_cache,
    _lru_put,
    _MAX_DECODE_CACHE,
    build_material,
)
from .dds_loader import decode_texture, decode_texture_bytes, batch_decode_dds


def prepare_nif_data(
    filepath: str,
    texture_dirs: list,
    ba2_mgr,
    nif_id: str = "main",
    game_profile=None,
) -> PreparedNifData:
    """CPU phase of NIF loading — thread-safe, no GL calls, no module-level writes.

    Parses the NIF, extracts vertex arrays, and decodes textures into a local dict.
    Pass the result to upload_nif_to_gpu() on the UI thread.
    """

    t0 = t_prev = time.perf_counter()
    nif = NifFile.load(filepath)
    t = time.perf_counter(); _log.debug("[nif-timing] nif_parse: %.1f ms", (t - t_prev) * 1000); t_prev = t

    # Extract all vertex data (CPU only)
    shapes: dict[int, PreparedShape] = {}
    prep_ctx = _PrepareContext(
        nif_id=nif_id, texture_dirs=texture_dirs, ba2_mgr=ba2_mgr, nif_path=filepath
    )
    if nif.blocks:
        _prepare_walk_blocks(nif, nif.blocks[0], shapes, prep_ctx)
    t = time.perf_counter(); _log.debug("[nif-timing] vertex_extract: %.1f ms", (t - t_prev) * 1000); t_prev = t

    render_batches = None
    if _should_prepare_lod_batches(filepath, shapes):
        render_batches = _prepare_lod_render_batches(nif, shapes)
        t = time.perf_counter(); _log.debug("[nif-timing] bto_batch_prepare: %.1f ms (%d batches)", (t - t_prev) * 1000, len(render_batches)); t_prev = t

    # Pre-scan all texture paths (loose + BA2) and decode them in parallel.
    # Skip textures already in the cross-NIF decode cache.
    extra_texture_paths = [
        part.texture_paths
        for shape in shapes.values()
        for part in shape.texture_slices or []
    ]
    all_textures = collect_nif_texture_paths(
        nif,
        texture_dirs,
        ba2_mgr,
        extra_texture_paths=extra_texture_paths,
    )
    t = time.perf_counter(); _log.debug("[nif-timing] tex_path_collect: %.1f ms (found %d)", (t - t_prev) * 1000, len(all_textures)); t_prev = t
    decoded_textures: dict[str, object] = {}

    if all_textures:
        # Partition into: already cached, loose DDS, BA2 bytes, non-DDS
        from pathlib import Path as _Path

        loose_dds: list[_Path] = []
        loose_other: list[tuple[str, _Path]] = []
        ba2_items: list[tuple[str, bytes]] = []

        for cache_key, resolved in all_textures.items():
            if cache_key in _decode_cache:
                decoded_textures[cache_key] = _decode_cache[cache_key]
                continue
            if isinstance(resolved, bytes):
                ba2_items.append((cache_key, resolved))
            elif isinstance(resolved, _Path):
                if resolved.suffix.lower() == ".dds":
                    loose_dds.append(resolved)
                else:
                    loose_other.append((cache_key, resolved))

        _log.debug(
            "Pre-decode: %d total, %d cached, %d loose DDS, %d BA2, %d other",
            len(all_textures),
            len(decoded_textures),
            len(loose_dds),
            len(ba2_items),
            len(loose_other),
        )

        # Batch decode loose DDS files natively.
        if loose_dds:
            batch_results = batch_decode_dds(loose_dds)
            for path_str, decoded in batch_results.items():
                decoded_textures[path_str] = decoded
            t = time.perf_counter(); _log.debug("[nif-timing] batch_dds_decode: %.1f ms (%d files)", (t - t_prev) * 1000, len(loose_dds)); t_prev = t

        # Decode BA2 bytes and non-DDS in parallel (these are fast individually)
        remaining = []
        for cache_key, data in ba2_items:
            remaining.append((cache_key, data, True))  # is_bytes=True
        for cache_key, path in loose_other:
            remaining.append((cache_key, path, False))  # is_bytes=False

        if remaining:
            workers = min(8, len(remaining))
            with ThreadPoolExecutor(max_workers=workers) as pool:

                def _decode_item(item):
                    key, data, is_bytes = item
                    if is_bytes:
                        return key, decode_texture_bytes(data, name=key)
                    else:
                        return key, decode_texture(str(data))

                futures = {pool.submit(_decode_item, item): item for item in remaining}
                for fut in as_completed(futures):
                    try:
                        key, result = fut.result()
                        decoded_textures[key] = result
                    except Exception as exc:
                        item = futures[fut]
                        decoded_textures[item[0]] = None
                        _log.debug("Pre-decode failed for %s: %s", item[0], exc)
            t = time.perf_counter(); _log.debug("[nif-timing] parallel_decode: %.1f ms (%d items)", (t - t_prev) * 1000, len(remaining)); t_prev = t

    _log.debug("[nif-timing] TOTAL prepare_nif_data: %.1f ms", (time.perf_counter() - t0) * 1000)
    return PreparedNifData(
        nif=nif,
        nif_id=nif_id,
        filepath=filepath,
        shapes=shapes,
        decoded_textures=decoded_textures,
        texture_dirs=texture_dirs,
        ba2_mgr=ba2_mgr,
        game_profile=game_profile,
        render_batches=render_batches,
    )


def upload_nif_to_gpu(
    prepared: PreparedNifData,
    ctx: "moderngl.Context",
    program: "moderngl.Program",
) -> tuple["SceneNode", "NifFile"]:
    """GPU phase of NIF loading — must run on the UI thread (ModernGL/OpenGL calls).

    Clears texture caches, merges pre-decoded textures, uploads GPU buffers,
    and reconstructs the SceneNode hierarchy.
    """
    t0 = t_prev = time.perf_counter()
    # Merge pre-decoded textures into the module-level cache so build_material
    # finds them without re-reading from disk or BA2 archives.
    # Caches persist across NIF loads (LRU-bounded) — no full wipe needed.
    for key, val in prepared.decoded_textures.items():
        _lru_put(_decode_cache, key, val, _MAX_DECODE_CACHE)
    t = time.perf_counter(); _log.debug("[nif-timing] cache_merge: %.1f ms", (t - t_prev) * 1000); t_prev = t

    root = SceneNode(name="nif_root", block_id=-1, nif_id=prepared.nif_id)
    if prepared.render_batches:
        child = _upload_render_batches(prepared, ctx, program)
        if child:
            root.children.append(child)
    elif prepared.nif.blocks:
        child = _upload_block(
            prepared.nif,
            prepared.nif.blocks[0],
            ctx,
            program,
            prepared,
        )
        if child:
            root.children.append(child)
    t = time.perf_counter(); _log.debug("[nif-timing] scene_graph_build: %.1f ms", (t - t_prev) * 1000); t_prev = t

    _update_world_transforms(root, glm.mat4(1.0))
    _log.debug("[nif-timing] TOTAL upload_nif_to_gpu: %.1f ms", (time.perf_counter() - t0) * 1000)
    return root, prepared.nif


def _upload_render_batches(
    prepared: PreparedNifData,
    ctx: "moderngl.Context",
    program: "moderngl.Program",
) -> "SceneNode | None":
    batch_root_name = Path(prepared.filepath).stem or "BTO batches"
    batch_root = SceneNode(
        name=f"{batch_root_name} (batched)",
        block_id=0,
        nif_id=prepared.nif_id,
    )
    for batch in prepared.render_batches or []:
        node = _upload_render_batch(prepared, batch, ctx, program)
        if node:
            batch_root.children.append(node)
    return batch_root if batch_root.children else None


def _upload_render_batch(
    prepared: PreparedNifData,
    batch: PreparedRenderBatch,
    ctx: "moderngl.Context",
    program: "moderngl.Program",
) -> "SceneNode | None":
    source_shape = prepared.nif.get_block(batch.material_shape_id)
    if source_shape is None:
        return None

    mesh = _build_mesh(
        ctx,
        program,
        batch.verts,
        batch.normals,
        batch.uvs,
        batch.tris,
        colors=batch.colors,
        tangents=batch.tangents,
        bitangents=batch.bitangents,
        uv2=batch.uv2,
    )
    mesh.material = build_material(
        ctx,
        prepared.nif,
        source_shape,
        prepared.texture_dirs,
        prepared.ba2_mgr,
        game_id=prepared.game_profile.id if prepared.game_profile else "fo4",
    )
    node = SceneNode(
        name=batch.name,
        block_id=batch.block_id,
        nif_id=prepared.nif_id,
        mesh=mesh,
    )
    node._local_verts = batch.verts
    node._local_tris = batch.tris
    node.source_block_ids = batch.source_block_ids
    node.is_lod_batch = True
    return node


def _upload_block(
    nif,
    block,
    ctx,
    program,
    prepared: PreparedNifData,
) -> "SceneNode | None":
    """Recursively build SceneNode tree, uploading GPU buffers for BSTriShape nodes."""
    schema = nif.schema
    type_name = block.type_name

    if schema.is_subtype_of(type_name, "NiNode"):
        return _upload_ninode(nif, block, ctx, program, prepared)
    if _is_tri_based_shape(schema, type_name):
        return _upload_shape(nif, block, ctx, program, prepared)
    if type_name == "BSGeometry":
        return _upload_external_geometry(nif, block, ctx, program, prepared)
    return None


def _upload_ninode(nif, block, ctx, program, prepared: PreparedNifData) -> "SceneNode":
    """Build a SceneNode for a NiNode, recursing into children."""
    name = _get_string(block, "Name") or f"NiNode_{block.block_id}"
    node = SceneNode(name=name, block_id=block.block_id, nif_id=prepared.nif_id)
    trans = block.get_field("Translation") or {}
    rot = block.get_field("Rotation") or {}
    scale = block.get_field("Scale")
    node.transform = nif_transform_to_mat4(
        trans, rot, float(scale) if scale is not None else 1.0
    )
    children_refs = block.get_field("Children") or []
    for ref in children_refs:
        ref_id = _get_ref_id(ref)
        if ref_id >= 0:
            child_block = nif.get_block(ref_id)
            if child_block:
                child = _upload_block(nif, child_block, ctx, program, prepared)
                if child:
                    node.children.append(child)

    # Check for collision overlay
    havok = prepared.game_profile.havok_scale if prepared.game_profile else None
    try:
        overlay = _extract_collision_overlay(nif, block, havok_scale=havok)
        if overlay:
            node.collision_overlay = overlay
    except Exception as exc:
        _log.debug(
            "Collision overlay extraction failed for block %d: %s", block.block_id, exc
        )

    return node


def _upload_external_geometry(
    nif, block, ctx, program, prepared: PreparedNifData
) -> "SceneNode | None":
    """Upload placeholder bounding box for Starfield BSGeometry with external mesh."""
    return _upload_shape(nif, block, ctx, program, prepared)


def _upload_shape(
    nif, block, ctx, program, prepared: PreparedNifData
) -> "SceneNode | None":
    """Upload GPU buffers for one BSTriShape using pre-extracted PreparedShape data."""
    ps = prepared.shapes.get(block.block_id)
    if ps is None:
        return None  # was empty during prepare phase

    if ps.texture_slices:
        return _build_texture_slice_node(
            ps,
            nif,
            block,
            ctx,
            program,
            prepared.texture_dirs,
            prepared.ba2_mgr,
            prepared.nif_id,
            prepared.game_profile.id if prepared.game_profile else "fo4",
        )

    mesh = _build_mesh(
        ctx,
        program,
        ps.verts,
        ps.normals,
        ps.uvs,
        ps.tris,
        colors=ps.colors,
        tangents=ps.tangents,
        bitangents=ps.bitangents,
        uv2=ps.uv2,
    )
    node = SceneNode(
        name=ps.name, block_id=ps.block_id, nif_id=prepared.nif_id, mesh=mesh
    )
    node._local_verts = ps.verts
    node._local_tris = ps.tris
    node.transform = ps.transform
    if ps.external_mesh_paths and ps.verts.shape[0] <= 8:
        # Still a placeholder (8 verts = bbox) — skip material
        node.is_external_geometry = True
        node.external_mesh_path = ps.external_mesh_paths[0]
    else:
        mesh.material = build_material(
            ctx,
            nif,
            block,
            prepared.texture_dirs,
            prepared.ba2_mgr,
            game_id=prepared.game_profile.id if prepared.game_profile else "fo4",
        )
        if ps.external_mesh_paths:
            node.is_external_geometry = True
            node.external_mesh_path = ps.external_mesh_paths[0]
    return node
