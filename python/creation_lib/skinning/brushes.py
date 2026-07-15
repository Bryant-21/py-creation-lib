"""Weight painting brush operations — pure numpy math, no UI dependencies."""
from __future__ import annotations

from collections import deque

import numpy as np


def build_adjacency(triangles: np.ndarray, num_vertices: int) -> list[set[int]]:
    """Build vertex adjacency list from triangle array.

    Args:
        triangles: (M, 3) uint32 triangle indices.
        num_vertices: Total number of vertices.

    Returns:
        List of sets, where adj[i] is the set of vertex indices adjacent to vertex i.
    """
    adj: list[set[int]] = [set() for _ in range(num_vertices)]
    for tri in triangles:
        v0, v1, v2 = int(tri[0]), int(tri[1]), int(tri[2])
        adj[v0].add(v1)
        adj[v0].add(v2)
        adj[v1].add(v0)
        adj[v1].add(v2)
        adj[v2].add(v0)
        adj[v2].add(v1)
    return adj


def _falloff_factor(distance: float, radius: float, falloff: float) -> float:
    """Compute brush falloff: 1.0 at center, 0.0 at edge, controlled by falloff curve."""
    if radius <= 0:
        return 0.0
    t = distance / radius
    if t >= 1.0:
        return 0.0
    # Smoothstep-like falloff with adjustable sharpness
    return (1.0 - t ** (1.0 / max(falloff, 0.01))) ** 2


def _ensure_bone_slot(
    weights: np.ndarray,
    bone_indices: np.ndarray,
    vi: int,
    bone_idx: int,
) -> int:
    """Ensure bone_idx has a slot in the per-vertex arrays. Returns the slot index."""
    max_b = weights.shape[1]

    # Already present?
    for j in range(max_b):
        if int(bone_indices[vi, j]) == bone_idx:
            return j

    # Find empty slot (weight == 0)
    for j in range(max_b):
        if weights[vi, j] <= 0:
            bone_indices[vi, j] = bone_idx
            weights[vi, j] = 0.0
            return j

    # No empty slot — evict the lowest weight bone
    min_j = int(np.argmin(weights[vi]))
    bone_indices[vi, min_j] = bone_idx
    weights[vi, min_j] = 0.0
    return min_j


def _normalize_vertex(weights: np.ndarray, vi: int) -> None:
    """Normalize weights for a single vertex to sum to 1.0."""
    total = weights[vi].sum()
    if total > 0:
        weights[vi] /= total
    weights[vi] = np.clip(weights[vi], 0.0, 1.0)


def paint_weight(
    weights: np.ndarray,
    bone_indices: np.ndarray,
    bone_idx: int,
    vertex_positions: np.ndarray,
    brush_center: np.ndarray,
    brush_radius: float,
    brush_strength: float,
    mode: str = "add",
    falloff: float = 0.5,
    auto_normalize: bool = True,
    vertex_mask: np.ndarray | None = None,
    selection_mask: np.ndarray | None = None,
) -> tuple[np.ndarray, np.ndarray]:
    """Apply paint brush to vertices within radius.

    Args:
        weights: (N, max_bones) float32 weight values (modified in-place).
        bone_indices: (N, max_bones) int32 bone indices (modified in-place).
        bone_idx: Index of the bone to paint.
        vertex_positions: (N, 3) float32 vertex positions.
        brush_center: (3,) float32 center of the brush in world space.
        brush_radius: Radius of the brush.
        brush_strength: Strength of the brush [0, 1].
        mode: "add", "subtract", or "set".
        falloff: Falloff curve sharpness (0.01 = sharp, 1.0 = linear).
        auto_normalize: Whether to normalize weights after painting.
        vertex_mask: (N,) float32 mask (0.0=editable, 1.0=locked). None = no mask.

    Returns:
        New (weights, bone_indices) copies with modifications applied.
    """
    weights = np.array(weights, dtype=np.float32, copy=True)
    bone_indices = np.array(bone_indices, dtype=np.int32, copy=True)
    brush_center = np.asarray(brush_center, dtype=np.float32)

    dists = np.linalg.norm(vertex_positions - brush_center, axis=1)
    in_radius = dists < brush_radius
    if selection_mask is not None:
        in_radius &= np.asarray(selection_mask, dtype=bool)

    for vi in np.where(in_radius)[0]:
        mask_factor = 1.0 - float(vertex_mask[vi]) if vertex_mask is not None else 1.0
        if mask_factor <= 0.0:
            continue

        ff = _falloff_factor(float(dists[vi]), brush_radius, falloff)
        delta = brush_strength * ff * mask_factor

        slot = _ensure_bone_slot(weights, bone_indices, vi, bone_idx)

        if mode == "add":
            weights[vi, slot] = min(1.0, weights[vi, slot] + delta)
        elif mode == "subtract":
            weights[vi, slot] = max(0.0, weights[vi, slot] - delta)
        elif mode == "set":
            weights[vi, slot] = np.clip(delta, 0.0, 1.0)

        if auto_normalize:
            _normalize_vertex(weights, vi)

    return weights, bone_indices


def smooth_weights(
    weights: np.ndarray,
    bone_indices: np.ndarray,
    adjacency: list[set[int]],
    vertex_positions: np.ndarray,
    brush_center: np.ndarray,
    brush_radius: float,
    brush_strength: float,
    iterations: int = 1,
    vertex_mask: np.ndarray | None = None,
    selection_mask: np.ndarray | None = None,
) -> tuple[np.ndarray, np.ndarray]:
    """Smooth weights using topological neighbors within brush radius.

    For each vertex in the brush, averages its weights with its mesh-connected
    neighbors, blending by brush_strength.

    Args:
        weights: (N, max_bones) float32 (copied, not modified in-place).
        bone_indices: (N, max_bones) int32 (copied, not modified in-place).
        adjacency: Vertex adjacency list from build_adjacency().
        vertex_positions: (N, 3) vertex positions.
        brush_center: (3,) brush center.
        brush_radius: Brush radius.
        brush_strength: Blend factor [0, 1].
        iterations: Number of smoothing iterations.
        vertex_mask: (N,) float32 mask (0.0=editable, 1.0=locked). None = no mask.

    Returns:
        Modified (weights, bone_indices).
    """
    weights = np.array(weights, dtype=np.float32, copy=True)
    bone_indices = np.array(bone_indices, dtype=np.int32, copy=True)
    brush_center = np.asarray(brush_center, dtype=np.float32)
    max_b = weights.shape[1]

    dists = np.linalg.norm(vertex_positions - brush_center, axis=1)
    in_radius = dists < brush_radius
    if selection_mask is not None:
        in_radius &= np.asarray(selection_mask, dtype=bool)
    affected = np.where(in_radius)[0]

    for _ in range(iterations):
        new_weights = weights.copy()

        for vi in affected:
            mask_factor = 1.0 - float(vertex_mask[vi]) if vertex_mask is not None else 1.0
            if mask_factor <= 0.0:
                continue
            neighbors = adjacency[vi]
            if not neighbors:
                continue

            # Accumulate bone weights from neighbors
            bone_map: dict[int, float] = {}
            for ni in neighbors:
                for j in range(max_b):
                    w = float(weights[ni, j])
                    bi = int(bone_indices[ni, j])
                    if w > 0:
                        bone_map[bi] = bone_map.get(bi, 0.0) + w

            # Average
            n_neighbors = len(neighbors)
            for bi in bone_map:
                bone_map[bi] /= n_neighbors

            # Blend with original (scaled by mask_factor)
            effective_strength = brush_strength * mask_factor
            sorted_bones = sorted(bone_map.items(), key=lambda x: -x[1])[:max_b]
            for j in range(max_b):
                if j < len(sorted_bones):
                    bi, avg_w = sorted_bones[j]
                    orig_w = 0.0
                    for k in range(max_b):
                        if int(bone_indices[vi, k]) == bi:
                            orig_w = float(weights[vi, k])
                            break
                    new_weights[vi, j] = orig_w * (1 - effective_strength) + avg_w * effective_strength
                    bone_indices[vi, j] = bi
                else:
                    new_weights[vi, j] = 0.0
                    bone_indices[vi, j] = 0

            # Normalize
            total = new_weights[vi].sum()
            if total > 0:
                new_weights[vi] /= total

        weights = new_weights

    return weights, bone_indices


def blur_weights(
    weights: np.ndarray,
    bone_indices: np.ndarray,
    vertex_positions: np.ndarray,
    brush_center: np.ndarray,
    brush_radius: float,
    brush_strength: float,
    vertex_mask: np.ndarray | None = None,
    selection_mask: np.ndarray | None = None,
) -> tuple[np.ndarray, np.ndarray]:
    """Blur weights using spatial neighbors (all vertices in radius).

    Unlike smooth_weights, this uses spatial proximity (all vertices within
    brush radius) rather than topological connectivity.

    Args:
        weights: (N, max_bones) float32 (copied).
        bone_indices: (N, max_bones) int32 (copied).
        vertex_positions: (N, 3) vertex positions.
        brush_center: (3,) brush center.
        brush_radius: Brush radius.
        brush_strength: Blend factor [0, 1].
        vertex_mask: (N,) float32 mask (0.0=editable, 1.0=locked). None = no mask.

    Returns:
        Modified (weights, bone_indices).
    """
    weights = np.array(weights, dtype=np.float32, copy=True)
    bone_indices = np.array(bone_indices, dtype=np.int32, copy=True)
    brush_center = np.asarray(brush_center, dtype=np.float32)
    max_b = weights.shape[1]

    dists = np.linalg.norm(vertex_positions - brush_center, axis=1)
    in_radius = dists < brush_radius
    if selection_mask is not None:
        in_radius &= np.asarray(selection_mask, dtype=bool)
    affected = np.where(in_radius)[0]

    if len(affected) == 0:
        return weights, bone_indices

    # Pre-compute average weights across all affected vertices
    bone_map: dict[int, float] = {}
    for vi in affected:
        inv_dist = 1.0 / (float(dists[vi]) + 1e-6)
        for j in range(max_b):
            w = float(weights[vi, j])
            bi = int(bone_indices[vi, j])
            if w > 0:
                bone_map[bi] = bone_map.get(bi, 0.0) + w * inv_dist

    # Normalize accumulated
    total_inv = sum(1.0 / (float(dists[vi]) + 1e-6) for vi in affected)
    if total_inv > 0:
        for bi in bone_map:
            bone_map[bi] /= total_inv

    sorted_bones = sorted(bone_map.items(), key=lambda x: -x[1])[:max_b]

    # Blend each affected vertex toward the average
    for vi in affected:
        mask_factor = 1.0 - float(vertex_mask[vi]) if vertex_mask is not None else 1.0
        if mask_factor <= 0.0:
            continue
        effective_strength = brush_strength * mask_factor
        for j in range(max_b):
            if j < len(sorted_bones):
                bi, avg_w = sorted_bones[j]
                orig_w = 0.0
                for k in range(max_b):
                    if int(bone_indices[vi, k]) == bi:
                        orig_w = float(weights[vi, k])
                        break
                weights[vi, j] = orig_w * (1 - effective_strength) + avg_w * effective_strength
                bone_indices[vi, j] = bi
            else:
                weights[vi, j] = 0.0
                bone_indices[vi, j] = 0

        total = weights[vi].sum()
        if total > 0:
            weights[vi] /= total

    return weights, bone_indices


def gradient_weights(
    weights: np.ndarray,
    bone_indices: np.ndarray,
    bone_idx: int,
    vertex_positions: np.ndarray,
    start_point: np.ndarray,
    end_point: np.ndarray,
    auto_normalize: bool = True,
) -> tuple[np.ndarray, np.ndarray]:
    """Apply linear gradient weight between two points.

    Vertices at start_point get weight 1.0, vertices at end_point get weight 0.0,
    with linear interpolation between. Vertices beyond the endpoints are clamped.

    Args:
        weights: (N, max_bones) float32 (copied).
        bone_indices: (N, max_bones) int32 (copied).
        bone_idx: Bone index to apply gradient to.
        vertex_positions: (N, 3) vertex positions.
        start_point: (3,) start of gradient (weight = 1.0).
        end_point: (3,) end of gradient (weight = 0.0).
        auto_normalize: Normalize after applying.

    Returns:
        Modified (weights, bone_indices).
    """
    weights = np.array(weights, dtype=np.float32, copy=True)
    bone_indices = np.array(bone_indices, dtype=np.int32, copy=True)
    start_point = np.asarray(start_point, dtype=np.float32)
    end_point = np.asarray(end_point, dtype=np.float32)

    direction = end_point - start_point
    length_sq = np.dot(direction, direction)

    if length_sq < 1e-12:
        return weights, bone_indices

    # Only affect vertices near the gradient line (within the line length as radius)
    length = float(np.sqrt(length_sq))
    max_dist = length  # perpendicular distance cutoff

    n_verts = len(vertex_positions)
    for vi in range(n_verts):
        # Project vertex onto the start->end line
        to_vert = vertex_positions[vi] - start_point
        t = np.dot(to_vert, direction) / length_sq

        # Skip vertices outside the line segment range
        if t < 0.0 or t > 1.0:
            continue

        # Skip vertices too far from the line (perpendicular distance)
        closest_point = start_point + t * direction
        perp_dist = float(np.linalg.norm(vertex_positions[vi] - closest_point))
        if perp_dist > max_dist:
            continue

        grad_weight = 1.0 - t  # 1.0 at start, 0.0 at end

        slot = _ensure_bone_slot(weights, bone_indices, vi, bone_idx)
        weights[vi, slot] = grad_weight

        if auto_normalize:
            _normalize_vertex(weights, vi)

    return weights, bone_indices


def mirror_weights(
    weights: np.ndarray,
    bone_indices: np.ndarray,
    bone_names: list[str],
    vertex_positions: np.ndarray,
    axis: int = 0,
    tolerance: float = 0.01,
    source_side: str = "positive",
) -> tuple[np.ndarray, np.ndarray]:
    """Mirror weights across an axis, mapping L_ bones to R_ bones.

    Finds mirror-matched vertex pairs (vertices whose position differs only
    by sign on the mirror axis within tolerance), then copies weights from
    +axis side to -axis side, remapping bone names with L/R swaps.

    Args:
        weights: (N, max_bones) float32 (copied).
        bone_indices: (N, max_bones) int32 (copied).
        bone_names: List of bone names for index resolution.
        vertex_positions: (N, 3) vertex positions.
        axis: Mirror axis (0=X, 1=Y, 2=Z).
        tolerance: Spatial tolerance for matching mirror vertices.

    Returns:
        Modified (weights, bone_indices).
    """
    weights = np.array(weights, dtype=np.float32, copy=True)
    bone_indices = np.array(bone_indices, dtype=np.int32, copy=True)

    # Build L<->R bone index mapping
    _MIRROR_PAIRS = [
        ("L", "R"), ("Left", "Right"), ("l_", "r_"), ("L_", "R_"),
        ("LArm", "RArm"), ("LLeg", "RLeg"),
    ]

    bone_mirror: dict[int, int] = {}
    for i, name in enumerate(bone_names):
        if i in bone_mirror:
            continue
        for left_pat, right_pat in _MIRROR_PAIRS:
            if left_pat in name:
                mirror_name = name.replace(left_pat, right_pat, 1)
                if mirror_name in bone_names:
                    j = bone_names.index(mirror_name)
                    bone_mirror[i] = j
                    bone_mirror[j] = i
                    break
            elif right_pat in name:
                mirror_name = name.replace(right_pat, left_pat, 1)
                if mirror_name in bone_names:
                    j = bone_names.index(mirror_name)
                    bone_mirror[i] = j
                    bone_mirror[j] = i
                    break

    n_verts = len(vertex_positions)
    max_b = weights.shape[1]

    # Build mirrored positions for lookup
    mirrored_pos = vertex_positions.copy()
    mirrored_pos[:, axis] = -mirrored_pos[:, axis]

    from creation_lib.scientific.native_runtime import CKDTree as cKDTree

    def copy_side(source_positive: bool) -> None:
        if source_positive:
            source_mask = vertex_positions[:, axis] > tolerance
            target_mask = vertex_positions[:, axis] < -tolerance
        else:
            source_mask = vertex_positions[:, axis] < -tolerance
            target_mask = vertex_positions[:, axis] > tolerance

        source_indices = np.where(source_mask)[0]
        target_indices = np.where(target_mask)[0]
        if len(source_indices) == 0 or len(target_indices) == 0:
            return

        target_tree = cKDTree(vertex_positions[target_indices])

        for si in source_indices:
            mirror_pt = mirrored_pos[si]
            dist, idx = target_tree.query(mirror_pt)
            if dist > tolerance:
                continue
            ti = target_indices[idx]

            for j in range(max_b):
                bi = int(bone_indices[si, j])
                w = float(weights[si, j])
                mirrored_bi = bone_mirror.get(bi, bi)
                bone_indices[ti, j] = mirrored_bi
                weights[ti, j] = w

    if source_side == "negative":
        copy_side(source_positive=False)
    elif source_side == "both":
        original_weights = weights.copy()
        original_indices = bone_indices.copy()
        copy_side(source_positive=True)
        weights_pos_to_neg = weights.copy()
        indices_pos_to_neg = bone_indices.copy()
        weights = original_weights
        bone_indices = original_indices
        copy_side(source_positive=False)
        positive_mask = vertex_positions[:, axis] > tolerance
        weights_pos_to_neg[positive_mask] = weights[positive_mask]
        indices_pos_to_neg[positive_mask] = bone_indices[positive_mask]
        weights = weights_pos_to_neg
        bone_indices = indices_pos_to_neg
    else:
        copy_side(source_positive=True)

    return weights, bone_indices


def flood_fill_weight(
    weights: np.ndarray,
    bone_indices: np.ndarray,
    bone_idx: int,
    weight_value: float,
    start_vertex: int,
    adjacency: list[set[int]],
    threshold: float = 0.01,
    auto_normalize: bool = True,
    vertex_mask: np.ndarray | None = None,
    selection_mask: np.ndarray | None = None,
) -> tuple[np.ndarray, np.ndarray]:
    """Flood fill weight from a starting vertex through connected region.

    Spreads outward from start_vertex through connected vertices that already
    have weight for the specified bone above threshold. Sets the weight to
    weight_value for all reached vertices.

    Args:
        weights: (N, max_bones) float32 (copied).
        bone_indices: (N, max_bones) int32 (copied).
        bone_idx: Bone index to flood fill.
        weight_value: Weight value to set on reached vertices.
        start_vertex: Starting vertex index.
        adjacency: Vertex adjacency list.
        threshold: Minimum existing weight to allow flood expansion.
        auto_normalize: Normalize after filling.
        vertex_mask: (N,) float32 mask (0.0=editable, 1.0=locked). None = no mask.

    Returns:
        Modified (weights, bone_indices).
    """
    weights = np.array(weights, dtype=np.float32, copy=True)
    bone_indices = np.array(bone_indices, dtype=np.int32, copy=True)
    n_verts = weights.shape[0]

    if start_vertex < 0 or start_vertex >= n_verts:
        return weights, bone_indices

    visited: set[int] = set()
    queue: deque[int] = deque([start_vertex])
    visited.add(start_vertex)

    while queue:
        vi = queue.popleft()

        # Skip masked vertices
        if vertex_mask is not None and float(vertex_mask[vi]) >= 1.0:
            continue
        if selection_mask is not None and not bool(selection_mask[vi]):
            continue

        # Set weight
        slot = _ensure_bone_slot(weights, bone_indices, vi, bone_idx)
        weights[vi, slot] = np.clip(weight_value, 0.0, 1.0)

        if auto_normalize:
            _normalize_vertex(weights, vi)

        # Expand to neighbors
        for ni in adjacency[vi]:
            if ni in visited:
                continue
            if selection_mask is not None and not bool(selection_mask[ni]):
                continue

            # Check if neighbor has sufficient existing weight for this bone
            has_weight = False
            for j in range(weights.shape[1]):
                if int(bone_indices[ni, j]) == bone_idx and weights[ni, j] >= threshold:
                    has_weight = True
                    break

            # Also allow expansion if neighbor has no weight at all (initial fill)
            total_w = float(weights[ni].sum())
            if has_weight or total_w < threshold:
                visited.add(ni)
                queue.append(ni)

    return weights, bone_indices
