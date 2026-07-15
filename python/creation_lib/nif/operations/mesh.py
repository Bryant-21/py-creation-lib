"""Mesh operations — bounds, degenerate tris, duplicate verts, flips."""
from ..actions import OperationResult


def update_bounds(nif, block_id: int | None = None) -> OperationResult:
    """Recompute bounding sphere from vertex positions."""
    shapes = [nif.get_block(block_id)] if block_id is not None else nif.find_blocks("BSTriShape")
    count = 0
    modified = []
    for shape in shapes:
        if shape is None:
            continue
        vertex_data = shape.get_field("Vertex Data") or []
        if not vertex_data:
            continue
        xs = [float(vd.get("Vertex", {}).get("x", 0)) for vd in vertex_data]
        ys = [float(vd.get("Vertex", {}).get("y", 0)) for vd in vertex_data]
        zs = [float(vd.get("Vertex", {}).get("z", 0)) for vd in vertex_data]
        cx = (min(xs) + max(xs)) / 2.0
        cy = (min(ys) + max(ys)) / 2.0
        cz = (min(zs) + max(zs)) / 2.0
        radius = max(((x - cx)**2 + (y - cy)**2 + (z - cz)**2)**0.5 for x, y, z in zip(xs, ys, zs))
        shape.set_field("Bounding Sphere", {
            "Center": {"x": cx, "y": cy, "z": cz},
            "Radius": radius,
        })
        modified.append(shape.block_id)
        count += 1
    return OperationResult(True, f"Updated bounds on {count} shape(s)", modified)


def prune_degenerate_tris(nif, block_id: int | None = None) -> OperationResult:
    """Remove zero-area and duplicate-index triangles."""
    shapes = [nif.get_block(block_id)] if block_id is not None else nif.find_blocks("BSTriShape")
    total_removed = 0
    modified = []
    for shape in shapes:
        if shape is None:
            continue
        triangles = shape.get_field("Triangles") or []
        if not triangles:
            continue
        clean = [t for t in triangles if int(t.get("v1", 0)) != int(t.get("v2", 0))
                 and int(t.get("v2", 0)) != int(t.get("v3", 0))
                 and int(t.get("v1", 0)) != int(t.get("v3", 0))]
        removed = len(triangles) - len(clean)
        if removed:
            shape.set_field("Triangles", clean)
            shape.set_field("Num Triangles", len(clean))
            total_removed += removed
            modified.append(shape.block_id)
    return OperationResult(True, f"Pruned {total_removed} degenerate triangle(s)", modified)


def flip_faces(nif, block_id: int | None = None) -> OperationResult:
    """Reverse triangle winding order."""
    shapes = [nif.get_block(block_id)] if block_id is not None else nif.find_blocks("BSTriShape")
    count = 0
    modified = []
    for shape in shapes:
        if shape is None:
            continue
        triangles = shape.get_field("Triangles") or []
        if not triangles:
            continue
        for t in triangles:
            t["v1"], t["v2"] = t.get("v2", 0), t.get("v1", 0)
        shape.set_field("Triangles", triangles)
        modified.append(shape.block_id)
        count += 1
    return OperationResult(True, f"Flipped faces on {count} shape(s)", modified)


def flip_uvs_v(nif, block_id: int | None = None) -> OperationResult:
    """Flip UV V coordinate (1-v)."""
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
            uv = vd.get("UV")
            if uv:
                uv["v"] = 1.0 - float(uv.get("v", 0))
        shape.set_field("Vertex Data", vertex_data)
        modified.append(shape.block_id)
        count += 1
    return OperationResult(True, f"Flipped UV V on {count} shape(s)", modified)
