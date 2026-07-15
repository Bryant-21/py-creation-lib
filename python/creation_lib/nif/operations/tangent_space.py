"""Tangent space generation -- critical for FO4 normal mapping."""
import numpy as np
from ..actions import OperationResult


def generate_tangent_space(nif, block_id: int) -> OperationResult:
    """Generate tangent + bitangent vectors for a BSTriShape.

    Uses a Mikktspace-style algorithm: accumulates per-face tangent/bitangent
    contributions weighted by face area, then orthogonalizes against the vertex
    normal using Gram-Schmidt.
    """
    block = nif.get_block(block_id)
    if not block:
        return OperationResult(False, f"Block {block_id} not found")

    vertex_data = block.get_field("Vertex Data") or []
    triangles = block.get_field("Triangles") or []
    if not vertex_data or not triangles:
        return OperationResult(False, "Block has no vertex/triangle data")

    n_verts = len(vertex_data)

    # Extract positions, normals, UVs
    positions = np.zeros((n_verts, 3), dtype=np.float64)
    normals = np.zeros((n_verts, 3), dtype=np.float64)
    uvs = np.zeros((n_verts, 2), dtype=np.float64)

    for i, vd in enumerate(vertex_data):
        v = vd.get("Vertex", {})
        positions[i] = [float(v.get("x", 0)), float(v.get("y", 0)), float(v.get("z", 0))]
        n = vd.get("Normal", {})
        normals[i] = [float(n.get("x", 0)), float(n.get("y", 0)), float(n.get("z", 0))]
        uv = vd.get("UV", {})
        uvs[i] = [float(uv.get("u", 0)), float(uv.get("v", 0))]

    # Normalize input normals
    n_len = np.linalg.norm(normals, axis=1, keepdims=True)
    n_len[n_len < 1e-8] = 1.0
    normals = normals / n_len

    tris = np.array(
        [[int(t.get("v1", 0)), int(t.get("v2", 0)), int(t.get("v3", 0))] for t in triangles],
        dtype=np.uint32,
    )

    # Accumulate per-vertex tangent/bitangent from triangles
    tangents = np.zeros((n_verts, 3), dtype=np.float64)
    bitangents = np.zeros((n_verts, 3), dtype=np.float64)

    if len(tris) > 0:
        i0, i1, i2 = tris[:, 0], tris[:, 1], tris[:, 2]
        p0, p1, p2 = positions[i0], positions[i1], positions[i2]
        uv0, uv1, uv2 = uvs[i0], uvs[i1], uvs[i2]

        edge1 = p1 - p0
        edge2 = p2 - p0
        duv1 = uv1 - uv0
        duv2 = uv2 - uv0

        # Determinant of UV matrix
        det = duv1[:, 0] * duv2[:, 1] - duv1[:, 1] * duv2[:, 0]
        # Avoid division by zero for degenerate UV triangles
        det[np.abs(det) < 1e-12] = 1.0
        inv_det = 1.0 / det

        # Per-face tangent and bitangent
        face_t = np.zeros_like(edge1)
        face_b = np.zeros_like(edge1)
        for axis in range(3):
            face_t[:, axis] = inv_det * (duv2[:, 1] * edge1[:, axis] - duv1[:, 1] * edge2[:, axis])
            face_b[:, axis] = inv_det * (-duv2[:, 0] * edge1[:, axis] + duv1[:, 0] * edge2[:, axis])

        # Accumulate onto vertices
        for col in range(3):
            np.add.at(tangents, tris[:, col], face_t)
            np.add.at(bitangents, tris[:, col], face_b)

    # Gram-Schmidt orthogonalize: T' = normalize(T - N * dot(N, T))
    dot_nt = np.sum(normals * tangents, axis=1, keepdims=True)
    tangents = tangents - normals * dot_nt
    t_len = np.linalg.norm(tangents, axis=1, keepdims=True)
    t_len[t_len < 1e-8] = 1.0
    tangents = tangents / t_len

    # Compute bitangent sign (handedness)
    cross = np.cross(normals, tangents)
    sign = np.sign(np.sum(cross * bitangents, axis=1))
    sign[sign == 0] = 1.0
    bitangents = cross * sign[:, np.newaxis]

    # Normalize bitangents
    b_len = np.linalg.norm(bitangents, axis=1, keepdims=True)
    b_len[b_len < 1e-8] = 1.0
    bitangents = bitangents / b_len

    # Write back to vertex data
    for i, vd in enumerate(vertex_data):
        vd["Tangent"] = {
            "x": float(tangents[i, 0]),
            "y": float(tangents[i, 1]),
            "z": float(tangents[i, 2]),
        }
        vd["Bitangent X"] = float(bitangents[i, 0])
        vd["Bitangent Y"] = float(bitangents[i, 1])
        vd["Bitangent Z"] = float(bitangents[i, 2])
    block.set_field("Vertex Data", vertex_data)

    return OperationResult(True, f"Generated tangent space for block {block_id}", [block_id])
