"""Ray-cast mesh picking for vertex, face, and surface point selection.

Provides unprojection and intersection utilities for brush-based weight
painting and vertex selection. Uses the vectorized Moller-Trumbore
algorithm with optional BVH acceleration for large meshes.
"""
from __future__ import annotations

import logging
from typing import Optional

import glm
import numpy as np

_log = logging.getLogger("nif.rendering.mesh_picking")

# Small epsilon for ray intersection tests
_EPS = 1e-6

# Threshold: meshes with more triangles than this use BVH acceleration
_BVH_THRESHOLD = 1000

# Maximum triangles per BVH leaf node
_BVH_LEAF_SIZE = 32


def unproject_ray(screen_x: float, screen_y: float,
                  viewport_width: float, viewport_height: float,
                  view_matrix: glm.mat4, proj_matrix: glm.mat4
                  ) -> tuple[np.ndarray, np.ndarray]:
    """Convert screen coordinates to a world-space ray (origin, direction).

    Args:
        screen_x: Mouse X in viewport-local pixels (0 = left edge).
        screen_y: Mouse Y in viewport-local pixels (0 = top edge).
        viewport_width: Viewport width in pixels.
        viewport_height: Viewport height in pixels.
        view_matrix: Camera view matrix.
        proj_matrix: Camera projection matrix.

    Returns:
        (ray_origin, ray_direction) as (3,) float64 numpy arrays.
        Direction is normalized.
    """
    # Convert to NDC [-1, 1]
    ndc_x = (screen_x / max(viewport_width, 1)) * 2.0 - 1.0
    ndc_y = 1.0 - (screen_y / max(viewport_height, 1)) * 2.0

    inv_vp = glm.inverse(proj_matrix * view_matrix)

    near_ndc = glm.vec4(ndc_x, ndc_y, -1.0, 1.0)
    far_ndc = glm.vec4(ndc_x, ndc_y, 1.0, 1.0)

    near_world = inv_vp * near_ndc
    far_world = inv_vp * far_ndc
    near_world /= near_world.w
    far_world /= far_world.w

    origin = np.array([near_world.x, near_world.y, near_world.z], dtype=np.float64)
    far_pt = np.array([far_world.x, far_world.y, far_world.z], dtype=np.float64)

    direction = far_pt - origin
    length = np.linalg.norm(direction)
    if length > _EPS:
        direction /= length

    return origin, direction


def pick_vertex(ray_origin: np.ndarray, ray_direction: np.ndarray,
                vertices: np.ndarray, radius: float = 0.5
                ) -> int | None:
    """Find the closest vertex to the ray within a screen-space radius.

    Projects each vertex onto the ray and checks if its perpendicular
    distance is within the given radius. Returns the index of the closest
    vertex (smallest t along the ray), or None if no vertex is within range.

    Args:
        ray_origin: Ray origin, shape (3,).
        ray_direction: Normalized ray direction, shape (3,).
        vertices: Vertex positions, shape (N, 3).
        radius: Maximum perpendicular distance from ray to consider a hit.

    Returns:
        Index of the closest hit vertex, or None.
    """
    if vertices is None or len(vertices) == 0:
        return None

    # Vector from ray origin to each vertex: (N, 3)
    to_vert = vertices.astype(np.float64) - ray_origin

    # Project onto ray: t = dot(to_vert, direction)
    t = np.einsum('ij,j->i', to_vert, ray_direction)

    # Closest point on ray to each vertex
    closest_on_ray = ray_origin + t[:, np.newaxis] * ray_direction  # (N, 3)

    # Perpendicular distance
    diff = vertices.astype(np.float64) - closest_on_ray
    dist_sq = np.einsum('ij,ij->i', diff, diff)

    # Filter: positive t (in front of camera) and within radius
    radius_sq = radius * radius
    valid = (t > _EPS) & (dist_sq < radius_sq)

    if not np.any(valid):
        return None

    # Among valid, pick the one with smallest t (closest to camera)
    valid_t = np.where(valid, t, np.inf)
    return int(np.argmin(valid_t))


def pick_face(ray_origin: np.ndarray, ray_direction: np.ndarray,
              vertices: np.ndarray, triangles: np.ndarray
              ) -> tuple[int, np.ndarray] | None:
    """Find the closest triangle hit and return (triangle_index, barycentric_coords).

    Uses vectorized Moller-Trumbore intersection (same algorithm as
    ui/editor/selection.py).

    Args:
        ray_origin: Ray origin, shape (3,).
        ray_direction: Normalized ray direction, shape (3,).
        vertices: Vertex positions, shape (N, 3).
        triangles: Triangle indices, shape (M, 3) with dtype uint32.

    Returns:
        (triangle_index, barycentric_coords) where barycentric_coords is
        a (3,) array [1-u-v, u, v], or None if no hit.
    """
    if triangles is None or len(triangles) == 0:
        return None
    if vertices is None or len(vertices) == 0:
        return None

    verts = vertices.astype(np.float64)
    origin = ray_origin.astype(np.float64)
    direction = ray_direction.astype(np.float64)

    # Gather triangle vertices: (M, 3) each
    v0 = verts[triangles[:, 0]]
    v1 = verts[triangles[:, 1]]
    v2 = verts[triangles[:, 2]]

    edge1 = v1 - v0
    edge2 = v2 - v0

    # h = direction x edge2
    h = np.cross(direction, edge2)
    a = np.einsum('ij,ij->i', edge1, h)

    # Filter near-parallel triangles
    valid = np.abs(a) > _EPS
    if not np.any(valid):
        return None

    f = np.zeros_like(a)
    f[valid] = 1.0 / a[valid]

    s = origin - v0
    u = f * np.einsum('ij,ij->i', s, h)
    valid &= (u >= 0.0) & (u <= 1.0)
    if not np.any(valid):
        return None

    q = np.cross(s, edge1)
    v = f * np.einsum('ij,ij->i', q, direction[np.newaxis] * np.ones_like(q))
    valid &= (v >= 0.0) & ((u + v) <= 1.0)
    if not np.any(valid):
        return None

    t = f * np.einsum('ij,ij->i', edge2, q)
    valid &= t > _EPS

    if not np.any(valid):
        return None

    # Find closest hit
    valid_t = np.where(valid, t, np.inf)
    best_idx = int(np.argmin(valid_t))

    bary = np.array([1.0 - u[best_idx] - v[best_idx],
                     u[best_idx], v[best_idx]], dtype=np.float64)

    return best_idx, bary


def pick_surface_point(ray_origin: np.ndarray, ray_direction: np.ndarray,
                       vertices: np.ndarray, triangles: np.ndarray
                       ) -> tuple[np.ndarray, int] | None:
    """Find the 3D point where the ray hits the mesh surface.

    Returns (hit_point_3d, triangle_index) or None.
    Used for brush center positioning in weight painting.

    Args:
        ray_origin: Ray origin, shape (3,).
        ray_direction: Normalized ray direction, shape (3,).
        vertices: Vertex positions, shape (N, 3).
        triangles: Triangle indices, shape (M, 3) with dtype uint32.

    Returns:
        (hit_point, triangle_index) or None.
    """
    result = pick_face(ray_origin, ray_direction, vertices, triangles)
    if result is None:
        return None

    tri_idx, bary = result
    verts = vertices.astype(np.float64)

    # Interpolate hit point from barycentric coordinates
    v0 = verts[triangles[tri_idx, 0]]
    v1 = verts[triangles[tri_idx, 1]]
    v2 = verts[triangles[tri_idx, 2]]

    hit_point = bary[0] * v0 + bary[1] * v1 + bary[2] * v2

    return hit_point, tri_idx


# ---------------------------------------------------------------------------
# BVH (Bounding Volume Hierarchy) for accelerated ray-triangle intersection
# ---------------------------------------------------------------------------

class _BVHNode:
    """A node in an axis-aligned bounding box BVH tree.

    Leaf nodes store a list of triangle indices. Internal nodes store
    left/right children and the split axis.
    """
    __slots__ = ("bbox_min", "bbox_max", "left", "right", "tri_indices")

    def __init__(self):
        self.bbox_min: np.ndarray = np.zeros(3, dtype=np.float64)
        self.bbox_max: np.ndarray = np.zeros(3, dtype=np.float64)
        self.left: _BVHNode | None = None
        self.right: _BVHNode | None = None
        self.tri_indices: np.ndarray | None = None  # leaf only


def _build_bvh(
    tri_bboxes_min: np.ndarray,
    tri_bboxes_max: np.ndarray,
    centroids: np.ndarray,
    indices: np.ndarray,
    leaf_size: int = _BVH_LEAF_SIZE,
) -> _BVHNode:
    """Recursively build a BVH from triangle AABBs.

    Args:
        tri_bboxes_min: (M, 3) per-triangle AABB min corners.
        tri_bboxes_max: (M, 3) per-triangle AABB max corners.
        centroids: (M, 3) per-triangle centroids.
        indices: Triangle indices into the original mesh.
        leaf_size: Max triangles per leaf before splitting.
    """
    node = _BVHNode()
    node.bbox_min = tri_bboxes_min[indices].min(axis=0)
    node.bbox_max = tri_bboxes_max[indices].max(axis=0)

    if len(indices) <= leaf_size:
        node.tri_indices = indices
        return node

    # Split on the longest axis at the centroid median
    extent = node.bbox_max - node.bbox_min
    axis = int(np.argmax(extent))

    cent_vals = centroids[indices, axis]
    median = np.median(cent_vals)

    left_mask = cent_vals <= median
    right_mask = ~left_mask

    # Avoid empty splits
    if not np.any(left_mask) or not np.any(right_mask):
        node.tri_indices = indices
        return node

    node.left = _build_bvh(
        tri_bboxes_min, tri_bboxes_max, centroids,
        indices[left_mask], leaf_size,
    )
    node.right = _build_bvh(
        tri_bboxes_min, tri_bboxes_max, centroids,
        indices[right_mask], leaf_size,
    )
    return node


def _ray_aabb_intersect(
    ray_origin: np.ndarray,
    ray_inv_dir: np.ndarray,
    bbox_min: np.ndarray,
    bbox_max: np.ndarray,
) -> bool:
    """Fast ray-AABB slab test. Returns True if ray intersects the box."""
    t1 = (bbox_min - ray_origin) * ray_inv_dir
    t2 = (bbox_max - ray_origin) * ray_inv_dir

    tmin = np.minimum(t1, t2)
    tmax = np.maximum(t1, t2)

    enter = tmin.max()
    exit_ = tmax.min()

    return enter <= exit_ and exit_ >= 0.0


def _bvh_query(
    node: _BVHNode,
    ray_origin: np.ndarray,
    ray_inv_dir: np.ndarray,
) -> list[np.ndarray]:
    """Traverse BVH and collect leaf triangle index arrays that the ray may hit."""
    if not _ray_aabb_intersect(ray_origin, ray_inv_dir, node.bbox_min, node.bbox_max):
        return []

    if node.tri_indices is not None:
        return [node.tri_indices]

    results: list[np.ndarray] = []
    if node.left is not None:
        results.extend(_bvh_query(node.left, ray_origin, ray_inv_dir))
    if node.right is not None:
        results.extend(_bvh_query(node.right, ray_origin, ray_inv_dir))
    return results


def _pick_face_bvh(
    ray_origin: np.ndarray,
    ray_direction: np.ndarray,
    vertices: np.ndarray,
    triangles: np.ndarray,
    bvh_root: _BVHNode,
) -> tuple[int, np.ndarray] | None:
    """Accelerated pick_face using BVH to narrow candidates."""
    origin = ray_origin.astype(np.float64)
    direction = ray_direction.astype(np.float64)

    # Inverse direction for slab test (handle near-zero components)
    inv_dir = np.empty(3, dtype=np.float64)
    for i in range(3):
        if abs(direction[i]) > _EPS:
            inv_dir[i] = 1.0 / direction[i]
        else:
            inv_dir[i] = 1e18 if direction[i] >= 0 else -1e18

    # Collect candidate triangle indices from BVH
    index_lists = _bvh_query(bvh_root, origin, inv_dir)
    if not index_lists:
        return None

    candidates = np.concatenate(index_lists)

    # Run Moller-Trumbore only on candidate triangles
    verts = vertices.astype(np.float64)
    v0 = verts[triangles[candidates, 0]]
    v1 = verts[triangles[candidates, 1]]
    v2 = verts[triangles[candidates, 2]]

    edge1 = v1 - v0
    edge2 = v2 - v0

    h = np.cross(direction, edge2)
    a = np.einsum('ij,ij->i', edge1, h)

    valid = np.abs(a) > _EPS
    if not np.any(valid):
        return None

    f = np.zeros_like(a)
    f[valid] = 1.0 / a[valid]

    s = origin - v0
    u = f * np.einsum('ij,ij->i', s, h)
    valid &= (u >= 0.0) & (u <= 1.0)
    if not np.any(valid):
        return None

    q = np.cross(s, edge1)
    v = f * np.einsum('ij,ij->i', q, direction[np.newaxis] * np.ones_like(q))
    valid &= (v >= 0.0) & ((u + v) <= 1.0)
    if not np.any(valid):
        return None

    t = f * np.einsum('ij,ij->i', edge2, q)
    valid &= t > _EPS

    if not np.any(valid):
        return None

    valid_t = np.where(valid, t, np.inf)
    best_local = int(np.argmin(valid_t))

    # Map back to original triangle index
    best_idx = int(candidates[best_local])

    bary = np.array([
        1.0 - u[best_local] - v[best_local],
        u[best_local],
        v[best_local],
    ], dtype=np.float64)

    return best_idx, bary


class MeshPicker:
    """Stateful mesh picker that caches mesh data for repeated picks.

    Convenience wrapper around the module-level pick functions for use
    in editor loops where the same mesh is picked against many times.
    Uses BVH acceleration for meshes with more than _BVH_THRESHOLD triangles.
    """

    def __init__(self, vertices: np.ndarray | None = None,
                 triangles: np.ndarray | None = None):
        self.vertices = vertices
        self.triangles = triangles
        self._bvh: _BVHNode | None = None

    def set_mesh(self, vertices: np.ndarray, triangles: np.ndarray):
        """Update the mesh data for picking. Builds BVH for large meshes."""
        self.vertices = vertices.astype(np.float64) if vertices is not None else None
        self.triangles = triangles
        self._bvh = None

        if (self.vertices is not None and self.triangles is not None
                and len(self.triangles) > _BVH_THRESHOLD):
            self._build_bvh()

    def _build_bvh(self):
        """Build BVH from current mesh data."""
        verts = self.vertices
        tris = self.triangles

        v0 = verts[tris[:, 0]]
        v1 = verts[tris[:, 1]]
        v2 = verts[tris[:, 2]]

        # Per-triangle AABBs
        stacked = np.stack([v0, v1, v2], axis=1)  # (M, 3, 3)
        tri_min = stacked.min(axis=1)  # (M, 3)
        tri_max = stacked.max(axis=1)  # (M, 3)
        centroids = stacked.mean(axis=1)  # (M, 3)

        indices = np.arange(len(tris), dtype=np.int64)
        self._bvh = _build_bvh(tri_min, tri_max, centroids, indices)
        _log.debug("Built BVH for %d triangles", len(tris))

    def pick_vertex(self, ray_origin: np.ndarray, ray_direction: np.ndarray,
                    radius: float = 0.5) -> int | None:
        """Pick the nearest vertex to the ray."""
        if self.vertices is None:
            return None
        return pick_vertex(ray_origin, ray_direction, self.vertices, radius)

    def pick_face(self, ray_origin: np.ndarray, ray_direction: np.ndarray
                  ) -> tuple[int, np.ndarray] | None:
        """Pick the nearest face and return (tri_index, barycentric)."""
        if self.vertices is None or self.triangles is None:
            return None
        if self._bvh is not None:
            return _pick_face_bvh(
                ray_origin, ray_direction,
                self.vertices, self.triangles, self._bvh,
            )
        return pick_face(ray_origin, ray_direction, self.vertices, self.triangles)

    def pick_surface_point(self, ray_origin: np.ndarray, ray_direction: np.ndarray
                           ) -> tuple[np.ndarray, int] | None:
        """Pick the 3D surface point under the cursor."""
        if self.vertices is None or self.triangles is None:
            return None

        result = self.pick_face(ray_origin, ray_direction)
        if result is None:
            return None

        tri_idx, bary = result
        verts = self.vertices.astype(np.float64)
        v0 = verts[self.triangles[tri_idx, 0]]
        v1 = verts[self.triangles[tri_idx, 1]]
        v2 = verts[self.triangles[tri_idx, 2]]
        hit_point = bary[0] * v0 + bary[1] * v1 + bary[2] * v2
        return hit_point, tri_idx

    def unproject_ray(self, screen_x: float, screen_y: float,
                      viewport_width: float, viewport_height: float,
                      view_matrix: glm.mat4, proj_matrix: glm.mat4
                      ) -> tuple[np.ndarray, np.ndarray]:
        """Convenience: unproject screen coords to world ray."""
        return unproject_ray(screen_x, screen_y,
                             viewport_width, viewport_height,
                             view_matrix, proj_matrix)
