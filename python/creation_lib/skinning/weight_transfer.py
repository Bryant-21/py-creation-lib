"""Weight transfer algorithms: barycentric projection, proximity, and hybrid."""
from __future__ import annotations

import logging
from typing import TYPE_CHECKING

import numpy as np
from creation_lib.scientific.native_runtime import CKDTree as cKDTree

if TYPE_CHECKING:
    from .skin_data import SkinData

_log = logging.getLogger("skinning.weight_transfer")


# ---------------------------------------------------------------------------
# Barycentric helpers
# ---------------------------------------------------------------------------

def _barycentric_coords(
    point: np.ndarray,
    v0: np.ndarray,
    v1: np.ndarray,
    v2: np.ndarray,
) -> tuple[np.ndarray, float]:
    """Compute barycentric coordinates of *point* projected onto triangle (v0, v1, v2).

    Returns:
        bary: (3,) float64 array [u, v, w] where u + v + w = 1
        dist: distance from point to projected position on triangle plane
    """
    edge1 = v1 - v0
    edge2 = v2 - v0
    normal = np.cross(edge1, edge2)
    area2 = np.dot(normal, normal)

    if area2 < 1e-12:
        # Degenerate triangle
        return np.array([1.0, 0.0, 0.0]), float("inf")

    # Project point onto triangle plane
    n_hat = normal / np.sqrt(area2)
    diff = point - v0
    plane_dist = np.dot(diff, n_hat)
    projected = point - plane_dist * n_hat

    # Barycentric coords via sub-triangle areas
    dp = projected - v0
    d00 = np.dot(edge1, edge1)
    d01 = np.dot(edge1, edge2)
    d11 = np.dot(edge2, edge2)
    d20 = np.dot(dp, edge1)
    d21 = np.dot(dp, edge2)

    denom = d00 * d11 - d01 * d01
    if abs(denom) < 1e-12:
        return np.array([1.0, 0.0, 0.0]), float("inf")

    v_coord = (d11 * d20 - d01 * d21) / denom
    w_coord = (d00 * d21 - d01 * d20) / denom
    u_coord = 1.0 - v_coord - w_coord

    bary = np.array([u_coord, v_coord, w_coord])
    return bary, abs(plane_dist)


def _find_nearest_triangle(
    point: np.ndarray,
    tree: cKDTree,
    source_verts: np.ndarray,
    source_tris: np.ndarray,
    k: int = 8,
) -> tuple[int, np.ndarray, float]:
    """Find the nearest source triangle to *point* using centroid KD-tree.

    Returns:
        tri_idx: index of best triangle
        bary: (3,) barycentric coordinates
        dist: distance to triangle plane
    """
    _, candidate_indices = tree.query(point, k=min(k, len(source_tris)))
    if isinstance(candidate_indices, (int, np.integer)):
        candidate_indices = [int(candidate_indices)]
    else:
        candidate_indices = candidate_indices.tolist()

    best_tri = 0
    best_bary = np.array([1.0, 0.0, 0.0])
    best_score = float("inf")

    for ti in candidate_indices:
        tri = source_tris[ti]
        v0, v1, v2 = source_verts[tri[0]], source_verts[tri[1]], source_verts[tri[2]]
        bary, plane_dist = _barycentric_coords(point, v0, v1, v2)

        # Clamp barycentric coords to measure how far outside the triangle
        clamped = np.clip(bary, 0, 1)
        clamped /= clamped.sum() + 1e-12
        # Reconstruct closest point on triangle
        closest = clamped[0] * v0 + clamped[1] * v1 + clamped[2] * v2
        dist_to_closest = float(np.linalg.norm(point - closest))

        if dist_to_closest < best_score:
            best_score = dist_to_closest
            best_tri = ti
            best_bary = bary

    return best_tri, best_bary, best_score


def _interpolate_weights(
    bary: np.ndarray,
    tri_idx: int,
    source: "SkinData",
) -> tuple[np.ndarray, np.ndarray]:
    """Interpolate bone weights at barycentric position within a source triangle.

    Returns:
        weights: (max_bones,) float32
        bone_indices: (max_bones,) int32
    """
    tri = source.triangles[tri_idx]
    max_b = source.weights.shape[1]

    # Clamp barycentric coords for interpolation
    bary_c = np.clip(bary, 0, 1)
    bary_sum = bary_c.sum()
    if bary_sum < 1e-12:
        bary_c = np.array([1.0, 0.0, 0.0])
    else:
        bary_c /= bary_sum

    # Collect all (bone_idx, weight) from the three corners
    bone_weight_map: dict[int, float] = {}
    for corner, bc in enumerate(bary_c):
        vi = int(tri[corner])
        for j in range(max_b):
            w = float(source.weights[vi, j])
            bi = int(source.bone_indices[vi, j])
            if w > 0:
                bone_weight_map[bi] = bone_weight_map.get(bi, 0.0) + w * bc

    # Sort by weight descending, keep top max_b
    sorted_bw = sorted(bone_weight_map.items(), key=lambda x: -x[1])[:max_b]

    out_w = np.zeros(max_b, dtype=np.float32)
    out_bi = np.zeros(max_b, dtype=np.int32)
    for j, (bi, w) in enumerate(sorted_bw):
        out_bi[j] = bi
        out_w[j] = w

    # Normalize
    total = out_w.sum()
    if total > 0:
        out_w /= total

    return out_w, out_bi


# ---------------------------------------------------------------------------
# Proximity transfer
# ---------------------------------------------------------------------------

def _proximity_transfer(
    source: "SkinData",
    target_vertices: np.ndarray,
    search_radius: float,
    k: int = 8,
) -> tuple[np.ndarray, np.ndarray]:
    """Transfer weights via inverse-distance-weighted nearest vertices."""
    max_b = source.weights.shape[1]
    n_target = len(target_vertices)
    out_w = np.zeros((n_target, max_b), dtype=np.float32)
    out_bi = np.zeros((n_target, max_b), dtype=np.int32)

    tree = cKDTree(source.vertices)

    for i in range(n_target):
        dists, indices = tree.query(target_vertices[i], k=min(k, len(source.vertices)))
        if isinstance(indices, (int, np.integer)):
            dists = np.array([float(dists)])
            indices = np.array([int(indices)])

        # Filter by search radius
        mask = dists <= search_radius
        if not mask.any():
            # Use nearest vertex regardless of radius
            mask = np.zeros_like(dists, dtype=bool)
            mask[0] = True

        valid_dists = dists[mask]
        valid_indices = indices[mask]

        # Inverse distance weights (with epsilon to avoid division by zero)
        inv_dists = 1.0 / (valid_dists + 1e-6)
        inv_dists /= inv_dists.sum()

        # Accumulate bone weights
        bone_weight_map: dict[int, float] = {}
        for vi, idw in zip(valid_indices, inv_dists):
            for j in range(max_b):
                w = float(source.weights[vi, j])
                bi = int(source.bone_indices[vi, j])
                if w > 0:
                    bone_weight_map[bi] = bone_weight_map.get(bi, 0.0) + w * idw

        sorted_bw = sorted(bone_weight_map.items(), key=lambda x: -x[1])[:max_b]
        for j, (bi, w) in enumerate(sorted_bw):
            out_bi[i, j] = bi
            out_w[i, j] = w

        total = out_w[i].sum()
        if total > 0:
            out_w[i] /= total

    return out_w, out_bi


# ---------------------------------------------------------------------------
# Main API
# ---------------------------------------------------------------------------

def transfer_weights(
    source: "SkinData",
    target_vertices: np.ndarray,
    target_triangles: np.ndarray,
    method: str = "barycentric",
    search_radius: float = 10.0,
    fallback_threshold: float = 0.0,
) -> tuple[np.ndarray, np.ndarray, dict]:
    """Transfer bone weights from source to target mesh.

    ``method`` is "barycentric", "proximity", or "hybrid". ``search_radius`` caps
    the proximity fallback distance (world units). In hybrid mode, targets
    farther than ``fallback_threshold`` use proximity; 0 auto-computes it as the
    source mesh's median edge length. Returns ``(weights, bone_indices, stats)``:
    (N, 4) float32 weights, (N, 4) int32 indices, and a dict of quality metrics.
    """
    target_vertices = np.asarray(target_vertices, dtype=np.float32).reshape(-1, 3)
    target_triangles = np.asarray(target_triangles, dtype=np.uint32).reshape(-1, 3)
    n_target = len(target_vertices)
    max_b = source.weights.shape[1] if source.num_vertices > 0 else 4

    # Edge case: empty source
    if source.num_vertices == 0 or source.num_triangles == 0:
        _log.warning("Source mesh is empty, returning zero weights")
        return (
            np.zeros((n_target, max_b), dtype=np.float32),
            np.zeros((n_target, max_b), dtype=np.int32),
            {"method": method, "transferred": 0, "fallback_count": 0},
        )

    if method == "proximity":
        out_w, out_bi = _proximity_transfer(source, target_vertices, search_radius)
        return out_w, out_bi, {
            "method": "proximity",
            "transferred": n_target,
            "fallback_count": 0,
        }

    # Barycentric or hybrid — build centroid KD-tree
    src_verts = np.asarray(source.vertices, dtype=np.float64)
    src_tris = np.asarray(source.triangles, dtype=np.int32)

    centroids = (
        src_verts[src_tris[:, 0]]
        + src_verts[src_tris[:, 1]]
        + src_verts[src_tris[:, 2]]
    ) / 3.0
    tree = cKDTree(centroids)

    # Auto-compute fallback threshold for hybrid
    if method == "hybrid" and fallback_threshold <= 0:
        # Use median edge length as a reasonable threshold
        e0 = np.linalg.norm(src_verts[src_tris[:, 1]] - src_verts[src_tris[:, 0]], axis=1)
        fallback_threshold = float(np.median(e0)) * 3.0
        _log.info("Auto fallback threshold: %.4f", fallback_threshold)

    out_w = np.zeros((n_target, max_b), dtype=np.float32)
    out_bi = np.zeros((n_target, max_b), dtype=np.int32)
    fallback_count = 0

    for i in range(n_target):
        pt = target_vertices[i].astype(np.float64)
        tri_idx, bary, dist = _find_nearest_triangle(pt, tree, src_verts, src_tris)

        use_proximity = False
        if method == "hybrid" and dist > fallback_threshold:
            use_proximity = True

        if not use_proximity:
            w, bi = _interpolate_weights(bary, tri_idx, source)
            out_w[i] = w
            out_bi[i] = bi
        else:
            # Proximity fallback for this vertex
            fallback_count += 1
            prox_w, prox_bi = _proximity_transfer(
                source, target_vertices[i:i + 1], search_radius
            )
            out_w[i] = prox_w[0]
            out_bi[i] = prox_bi[0]

    stats = {
        "method": method,
        "transferred": n_target,
        "fallback_count": fallback_count,
    }
    if method == "hybrid":
        stats["fallback_threshold"] = fallback_threshold

    return out_w, out_bi, stats
