"""Geometry helpers for building preview mesh dicts (vertices/triangles).

These are pure geometry — no Havok format dependency.
"""
from __future__ import annotations

import math
from typing import Any


def merge_preview_meshes(meshes: list[dict[str, Any]]) -> dict[str, Any] | None:
    merged_vertices: list[dict[str, float]] = []
    merged_triangles: list[dict[str, int]] = []
    for mesh in meshes:
        vertices = list(mesh.get("vertices") or [])
        triangles = list(mesh.get("triangles") or [])
        if not vertices or not triangles:
            continue
        offset = len(merged_vertices)
        merged_vertices.extend(vertices)
        merged_triangles.extend(
            {
                "v1": int(tri.get("v1", 0)) + offset,
                "v2": int(tri.get("v2", 0)) + offset,
                "v3": int(tri.get("v3", 0)) + offset,
            }
            for tri in triangles
        )
    if not merged_vertices:
        return None
    return {"vertices": merged_vertices, "triangles": merged_triangles}


def mesh_to_wireframe_lines(mesh: dict[str, Any]) -> list[list[float]]:
    vertices = list(mesh.get("vertices") or [])
    triangles = list(mesh.get("triangles") or [])
    if not vertices or not triangles:
        return []

    edges: set[tuple[int, int]] = set()
    lines: list[list[float]] = []
    for tri in triangles:
        ids = [int(tri.get("v1", 0)), int(tri.get("v2", 0)), int(tri.get("v3", 0))]
        for first, second in ((ids[0], ids[1]), (ids[1], ids[2]), (ids[2], ids[0])):
            edge = (min(first, second), max(first, second))
            if edge in edges:
                continue
            edges.add(edge)
            va = vertices[first]
            vb = vertices[second]
            lines.append(
                [
                    float(va.get("x", 0.0)),
                    float(va.get("y", 0.0)),
                    float(va.get("z", 0.0)),
                ]
            )
            lines.append(
                [
                    float(vb.get("x", 0.0)),
                    float(vb.get("y", 0.0)),
                    float(vb.get("z", 0.0)),
                ]
            )
    return lines


def capsule_mesh_from_endpoints(
    point_a: list[float],
    point_b: list[float],
    radius: float,
    havok_scale: float,
    segments: int = 12,
    hemi_rings: int = 4,
) -> dict[str, Any]:
    ax = float(point_a[0]) * havok_scale
    ay = float(point_a[1]) * havok_scale
    az = float(point_a[2]) * havok_scale
    bx = float(point_b[0]) * havok_scale
    by = float(point_b[1]) * havok_scale
    bz = float(point_b[2]) * havok_scale
    radius_nif = float(radius) * havok_scale
    direction = [bx - ax, by - ay, bz - az]
    basis_u, basis_v = _orthonormal_basis(direction)
    axis = _normalize_vector(direction)

    vertices: list[dict[str, float]] = []
    for ring in range(hemi_rings + 1):
        angle = (math.pi / 2.0) * (ring / hemi_rings)
        radial = math.sin(angle) * radius_nif
        offset = math.cos(angle) * radius_nif
        for segment in range(segments):
            phi = (2.0 * math.pi * segment) / segments
            dx = math.cos(phi) * basis_u[0] + math.sin(phi) * basis_v[0]
            dy = math.cos(phi) * basis_u[1] + math.sin(phi) * basis_v[1]
            dz = math.cos(phi) * basis_u[2] + math.sin(phi) * basis_v[2]
            vertices.append(
                {
                    "x": ax - axis[0] * offset + dx * radial,
                    "y": ay - axis[1] * offset + dy * radial,
                    "z": az - axis[2] * offset + dz * radial,
                }
            )
            vertices.append(
                {
                    "x": bx + axis[0] * offset + dx * radial,
                    "y": by + axis[1] * offset + dy * radial,
                    "z": bz + axis[2] * offset + dz * radial,
                }
            )
    return {"vertices": vertices, "triangles": _convex_hull_triangles(vertices)}


def sphere_mesh_from_center(
    center: list[float],
    radius: float,
    havok_scale: float,
    segments: int = 12,
    rings: int = 8,
) -> dict[str, Any]:
    cx = float(center[0]) * havok_scale
    cy = float(center[1]) * havok_scale
    cz = float(center[2]) * havok_scale
    radius_nif = float(radius) * havok_scale

    vertices: list[dict[str, float]] = []
    triangles: list[dict[str, int]] = []
    for ring in range(rings + 1):
        theta = math.pi * ring / rings
        sin_theta = math.sin(theta)
        cos_theta = math.cos(theta)
        for segment in range(segments):
            phi = (2.0 * math.pi * segment) / segments
            vertices.append(
                {
                    "x": cx + radius_nif * sin_theta * math.cos(phi),
                    "y": cy + radius_nif * sin_theta * math.sin(phi),
                    "z": cz + radius_nif * cos_theta,
                }
            )
    for ring in range(rings):
        for segment in range(segments):
            next_segment = (segment + 1) % segments
            current = ring * segments + segment
            next_row = (ring + 1) * segments + segment
            current_next = ring * segments + next_segment
            next_row_next = (ring + 1) * segments + next_segment
            triangles.append({"v1": current, "v2": next_row, "v3": current_next})
            triangles.append({"v1": current_next, "v2": next_row, "v3": next_row_next})
    return {"vertices": vertices, "triangles": triangles}


def box_mesh_from_half_extents(
    half_extents: list[float],
    havok_scale: float,
    center: list[float] | None = None,
) -> dict[str, Any]:
    center = center or [0.0, 0.0, 0.0]
    hx = float(half_extents[0]) * havok_scale
    hy = float(half_extents[1]) * havok_scale
    hz = float(half_extents[2]) * havok_scale
    cx = float(center[0]) * havok_scale
    cy = float(center[1]) * havok_scale
    cz = float(center[2]) * havok_scale

    vertices = [
        {"x": cx - hx, "y": cy - hy, "z": cz - hz},
        {"x": cx + hx, "y": cy - hy, "z": cz - hz},
        {"x": cx + hx, "y": cy + hy, "z": cz - hz},
        {"x": cx - hx, "y": cy + hy, "z": cz - hz},
        {"x": cx - hx, "y": cy - hy, "z": cz + hz},
        {"x": cx + hx, "y": cy - hy, "z": cz + hz},
        {"x": cx + hx, "y": cy + hy, "z": cz + hz},
        {"x": cx - hx, "y": cy + hy, "z": cz + hz},
    ]
    triangles = [
        {"v1": 0, "v2": 1, "v3": 2},
        {"v1": 0, "v2": 2, "v3": 3},
        {"v1": 4, "v2": 6, "v3": 5},
        {"v1": 4, "v2": 7, "v3": 6},
        {"v1": 0, "v2": 4, "v3": 5},
        {"v1": 0, "v2": 5, "v3": 1},
        {"v1": 1, "v2": 5, "v3": 6},
        {"v1": 1, "v2": 6, "v3": 2},
        {"v1": 2, "v2": 6, "v3": 7},
        {"v1": 2, "v2": 7, "v3": 3},
        {"v1": 3, "v2": 7, "v3": 4},
        {"v1": 3, "v2": 4, "v3": 0},
    ]
    return {"vertices": vertices, "triangles": triangles}


def _convex_hull_triangles(vertices: list[dict[str, float]]) -> list[dict[str, int]]:
    if len(vertices) < 3:
        return []
    from creation_lib.scientific.native_runtime import convex_hull_triangles

    try:
        triangles = convex_hull_triangles(
            [[vertex["x"], vertex["y"], vertex["z"]] for vertex in vertices]
        )
    except Exception as exc:
        raise ValueError("failed to build 3D convex hull preview triangles") from exc
    return [
        {"v1": int(face[0]), "v2": int(face[1]), "v3": int(face[2])}
        for face in triangles
    ]


def _orthonormal_basis(direction: list[float]) -> tuple[list[float], list[float]]:
    axis = _normalize_vector(direction)
    reference = [0.0, 0.0, 1.0] if abs(axis[2]) < 0.95 else [0.0, 1.0, 0.0]
    u = _normalize_vector(_cross(reference, axis))
    v = _normalize_vector(_cross(axis, u))
    return u, v


def _normalize_vector(vector: list[float]) -> list[float]:
    length = math.sqrt(sum(component * component for component in vector))
    if length <= 1e-8:
        return [1.0, 0.0, 0.0]
    return [component / length for component in vector]


def _cross(left: list[float], right: list[float]) -> list[float]:
    return [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
