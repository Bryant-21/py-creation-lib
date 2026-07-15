"""Normal vector operations — fix, flip, normalize."""
import numpy as np
from ..actions import OperationResult


def fix_normals(nif, block_id: int | None = None) -> OperationResult:
    """Recompute smooth normals (area-weighted) for a shape or all shapes.
    If block_id is None, operates on all BSTriShape blocks."""
    shapes = [nif.get_block(block_id)] if block_id is not None else nif.find_blocks("BSTriShape")
    count = 0
    modified = []

    for shape in shapes:
        if shape is None:
            continue
        vertex_data = shape.get_field("Vertex Data") or []
        triangles = shape.get_field("Triangles") or []
        if not vertex_data or not triangles:
            continue

        n_verts = len(vertex_data)
        verts = np.zeros((n_verts, 3), dtype=np.float32)
        for i, vd in enumerate(vertex_data):
            v = vd.get("Vertex", {})
            verts[i] = [float(v.get("x", 0)), float(v.get("y", 0)), float(v.get("z", 0))]

        tris = np.array(
            [[int(t.get("v1", 0)), int(t.get("v2", 0)), int(t.get("v3", 0))] for t in triangles],
            dtype=np.uint32,
        )

        normals = np.zeros_like(verts)
        if len(tris) > 0:
            v0, v1, v2 = verts[tris[:, 0]], verts[tris[:, 1]], verts[tris[:, 2]]
            face_normals = np.cross(v1 - v0, v2 - v0)
            for col in range(3):
                np.add.at(normals, tris[:, col], face_normals)
            lengths = np.linalg.norm(normals, axis=1, keepdims=True)
            lengths[lengths < 1e-8] = 1.0
            normals = normals / lengths

        for i, vd in enumerate(vertex_data):
            vd["Normal"] = {"x": float(normals[i, 0]), "y": float(normals[i, 1]), "z": float(normals[i, 2])}
        shape.set_field("Vertex Data", vertex_data)
        modified.append(shape.block_id)
        count += 1

    return OperationResult(True, f"Fixed normals on {count} shape(s)", modified)


def flip_normals(nif, block_id: int | None = None) -> OperationResult:
    """Negate all vertex normals."""
    shapes = [nif.get_block(block_id)] if block_id is not None else nif.find_blocks("BSTriShape")
    count = 0
    modified = []
    for shape in shapes:
        if shape is None:
            continue
        vertex_data = shape.get_field("Vertex Data") or []
        if not vertex_data:
            continue
        for vd in vertex_data:
            n = vd.get("Normal")
            if n:
                n["x"] = -float(n.get("x", 0))
                n["y"] = -float(n.get("y", 0))
                n["z"] = -float(n.get("z", 0))
        shape.set_field("Vertex Data", vertex_data)
        modified.append(shape.block_id)
        count += 1
    return OperationResult(True, f"Flipped normals on {count} shape(s)", modified)


def normalize_normals(nif, block_id: int | None = None) -> OperationResult:
    """Normalize all vertex normals to unit length."""
    shapes = [nif.get_block(block_id)] if block_id is not None else nif.find_blocks("BSTriShape")
    count = 0
    modified = []
    for shape in shapes:
        if shape is None:
            continue
        vertex_data = shape.get_field("Vertex Data") or []
        if not vertex_data:
            continue
        fixed = 0
        for vd in vertex_data:
            n = vd.get("Normal")
            if not n:
                continue
            nx, ny, nz = float(n.get("x", 0)), float(n.get("y", 0)), float(n.get("z", 0))
            length = (nx * nx + ny * ny + nz * nz) ** 0.5
            if length > 1e-8 and abs(length - 1.0) > 1e-4:
                n["x"], n["y"], n["z"] = nx / length, ny / length, nz / length
                fixed += 1
        if fixed:
            shape.set_field("Vertex Data", vertex_data)
            modified.append(shape.block_id)
            count += 1
    return OperationResult(True, f"Normalized normals on {count} shape(s)", modified)
