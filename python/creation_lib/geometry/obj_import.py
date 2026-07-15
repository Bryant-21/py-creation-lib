"""Shared OBJ geometry parser."""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class ObjGeometry:
    vertices: list[tuple[float, float, float]]
    normals: list[tuple[float, float, float]]
    uvs: list[tuple[float, float]]
    triangles: list[tuple[int, int, int]]
    has_normals: bool
    has_uvs: bool


def load_obj_geometry(path: str | Path, *, flip_v: bool = False) -> ObjGeometry:
    path = Path(path)
    if not path.exists():
        raise FileNotFoundError(f"OBJ file not found: {path}")

    raw_verts: list[tuple[float, float, float]] = []
    raw_normals: list[tuple[float, float, float]] = []
    raw_uvs: list[tuple[float, float]] = []
    faces: list[list[tuple[int, int, int]]] = []

    with open(path, "r", encoding="utf-8", errors="replace") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue

            parts = line.split()
            prefix = parts[0]

            if prefix == "v" and len(parts) >= 4:
                raw_verts.append((float(parts[1]), float(parts[2]), float(parts[3])))
            elif prefix == "vn" and len(parts) >= 4:
                raw_normals.append((float(parts[1]), float(parts[2]), float(parts[3])))
            elif prefix == "vt" and len(parts) >= 3:
                raw_uvs.append((float(parts[1]), float(parts[2])))
            elif prefix == "f" and len(parts) >= 4:
                faces.append([
                    _parse_face_vertex(part, len(raw_verts), len(raw_uvs), len(raw_normals))
                    for part in parts[1:]
                ])

    if not raw_verts:
        raise ValueError(f"No vertices found in OBJ file: {path}")

    unique_map: dict[tuple[int, int, int], int] = {}
    out_verts: list[tuple[float, float, float]] = []
    out_normals: list[tuple[float, float, float]] = []
    out_uvs: list[tuple[float, float]] = []
    out_tris: list[tuple[int, int, int]] = []
    has_normals = False
    has_uvs = False

    def get_unique_vertex(vi: int, vti: int, vni: int) -> int:
        nonlocal has_normals, has_uvs
        key = (vi, vti, vni)
        if key in unique_map:
            return unique_map[key]

        idx = len(out_verts)
        unique_map[key] = idx
        out_verts.append(raw_verts[vi] if 0 <= vi < len(raw_verts) else (0.0, 0.0, 0.0))

        if 0 <= vni < len(raw_normals):
            out_normals.append(raw_normals[vni])
            has_normals = True
        else:
            out_normals.append((0.0, 0.0, 0.0))

        if 0 <= vti < len(raw_uvs):
            u, v = raw_uvs[vti]
            out_uvs.append((u, 1.0 - v if flip_v else v))
            has_uvs = True
        else:
            out_uvs.append((0.0, 0.0))

        return idx

    for face in faces:
        if len(face) < 3:
            continue
        i0 = get_unique_vertex(*face[0])
        for k in range(1, len(face) - 1):
            i1 = get_unique_vertex(*face[k])
            i2 = get_unique_vertex(*face[k + 1])
            out_tris.append((i0, i1, i2))

    if not out_tris:
        raise ValueError(f"No triangles found in OBJ file: {path}")

    return ObjGeometry(
        vertices=out_verts,
        normals=out_normals,
        uvs=out_uvs,
        triangles=out_tris,
        has_normals=has_normals,
        has_uvs=has_uvs,
    )


def _parse_face_vertex(part: str, num_verts: int, num_uvs: int, num_normals: int) -> tuple[int, int, int]:
    indices = part.split("/")
    vi = _resolve_obj_index(indices[0], num_verts)
    vti = _resolve_obj_index(indices[1], num_uvs) if len(indices) > 1 and indices[1] else -1
    vni = _resolve_obj_index(indices[2], num_normals) if len(indices) > 2 and indices[2] else -1
    return vi, vti, vni


def _resolve_obj_index(value: str, count: int) -> int:
    try:
        index = int(value)
    except ValueError:
        return -1
    if index > 0:
        return index - 1
    if index < 0:
        return count + index
    return -1
