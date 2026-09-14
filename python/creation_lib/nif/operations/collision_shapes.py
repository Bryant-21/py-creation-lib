"""Collision shape generators — capsule, cylinder, sphere, auto, optimized.

Complements collision.py which has convex_hull and box. These generators
use PCA axis fitting to create best-fit primitives for weapon parts and
general NIF meshes.
"""
import numpy as np

from .collision import HAVOK_SCALE_FO4, DEFAULT_RADIUS, _create_convex_shape, _create_box_shape


def _fit_pca_axis(verts: np.ndarray) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Fit a principal axis to a point cloud via PCA.

    Returns:
        (center, principal_axis_unit, projections_along_axis)
    """
    center = verts.mean(axis=0)
    centered = verts - center
    # 3x3 covariance matrix — eigenvectors via numpy.
    cov = np.dot(centered.T, centered) / max(len(verts) - 1, 1)
    eigenvalues, eigenvectors = np.linalg.eigh(cov)
    # eigh returns sorted ascending — last eigenvector is principal axis
    principal = eigenvectors[:, -1].astype(np.float64)
    principal /= max(np.linalg.norm(principal), 1e-12)
    projections = np.dot(centered, principal)
    return center, principal, projections


def _perpendicular_distances(centered: np.ndarray, axis: np.ndarray,
                             projections: np.ndarray) -> np.ndarray:
    """Compute perpendicular distances from each point to the axis."""
    axis_components = np.outer(projections, axis)
    perp_vectors = centered - axis_components
    return np.linalg.norm(perp_vectors, axis=1)


def _elongation_ratio(verts: np.ndarray) -> tuple[float, np.ndarray, np.ndarray, np.ndarray]:
    """Compute elongation ratio (principal extent / perpendicular extent).

    Returns (ratio, center, axis, projections).
    """
    center, axis, projections = _fit_pca_axis(verts)
    principal_extent = projections.max() - projections.min()
    centered = verts - center
    perp_dists = _perpendicular_distances(centered, axis, projections)
    perp_extent = 2.0 * perp_dists.max() if len(perp_dists) > 0 else 1e-12
    ratio = principal_extent / max(perp_extent, 1e-12)
    return ratio, center, axis, projections


def create_capsule_shape(nif, verts_nif: np.ndarray,
                         radius: float = DEFAULT_RADIUS,
                         havok_scale_factor: float = HAVOK_SCALE_FO4) -> int | None:
    """Create bhkCapsuleShape from NIF-space vertices via PCA; return its block_id or None.

    The axis follows the principal component, endpoints sit at the min/max
    projection, and the radius is the max perpendicular distance.
    """
    if len(verts_nif) < 2:
        return None

    center, axis, projections = _fit_pca_axis(verts_nif)
    centered = verts_nif - center
    perp_dists = _perpendicular_distances(centered, axis, projections)

    proj_min = float(projections.min())
    proj_max = float(projections.max())
    capsule_radius = float(perp_dists.max()) if len(perp_dists) > 0 else 0.1

    # Endpoints in NIF space
    point1 = center + axis * proj_min
    point2 = center + axis * proj_max

    havok_scale = 1.0 / havok_scale_factor

    capsule = nif.add_block("bhkCapsuleShape")
    if not capsule:
        return None

    capsule.set_field("First Point", {
        "x": float(point1[0] * havok_scale),
        "y": float(point1[1] * havok_scale),
        "z": float(point1[2] * havok_scale),
    })
    capsule.set_field("Second Point", {
        "x": float(point2[0] * havok_scale),
        "y": float(point2[1] * havok_scale),
        "z": float(point2[2] * havok_scale),
    })
    capsule.set_field("Radius", float(capsule_radius * havok_scale))
    capsule.set_field("Radius 1", float(capsule_radius * havok_scale))
    capsule.set_field("Radius 2", float(capsule_radius * havok_scale))
    return capsule.block_id


def create_cylinder_shape(nif, verts_nif: np.ndarray,
                          radius: float = DEFAULT_RADIUS,
                          havok_scale_factor: float = HAVOK_SCALE_FO4) -> int | None:
    """Create bhkCylinderShape from NIF-space vertices via PCA.

    Same axis fitting as capsule. Vertex A/B are Vector4 (w=0).

    Args:
        havok_scale_factor: NIF-to-Havok scale (default HAVOK_SCALE_FO4).

    Returns block_id of the cylinder shape, or None.
    """
    if len(verts_nif) < 2:
        return None

    center, axis, projections = _fit_pca_axis(verts_nif)
    centered = verts_nif - center
    perp_dists = _perpendicular_distances(centered, axis, projections)

    proj_min = float(projections.min())
    proj_max = float(projections.max())
    cyl_radius = float(perp_dists.max()) if len(perp_dists) > 0 else 0.1

    point_a = center + axis * proj_min
    point_b = center + axis * proj_max

    havok_scale = 1.0 / havok_scale_factor

    cylinder = nif.add_block("bhkCylinderShape")
    if not cylinder:
        return None

    cylinder.set_field("Vertex A", {
        "x": float(point_a[0] * havok_scale),
        "y": float(point_a[1] * havok_scale),
        "z": float(point_a[2] * havok_scale),
        "w": 0.0,
    })
    cylinder.set_field("Vertex B", {
        "x": float(point_b[0] * havok_scale),
        "y": float(point_b[1] * havok_scale),
        "z": float(point_b[2] * havok_scale),
        "w": 0.0,
    })
    cylinder.set_field("Cylinder Radius", float(cyl_radius * havok_scale))
    cylinder.set_field("Radius", float(radius))
    return cylinder.block_id


def create_sphere_shape(nif, verts_nif: np.ndarray,
                        radius: float = DEFAULT_RADIUS,
                        havok_scale_factor: float = HAVOK_SCALE_FO4) -> tuple[int, int] | None:
    """Create bhkSphereShape wrapped in bhkTransformShape.

    Sphere radius = max distance from centroid. TransformShape positions it.

    Args:
        havok_scale_factor: NIF-to-Havok scale (default HAVOK_SCALE_FO4).

    Returns (transform_block_id, sphere_block_id) or None.
    """
    if len(verts_nif) == 0:
        return None

    havok_scale = 1.0 / havok_scale_factor
    centroid = verts_nif.mean(axis=0)
    dists = np.linalg.norm(verts_nif - centroid, axis=1)
    sphere_radius = float(dists.max())

    sphere = nif.add_block("bhkSphereShape")
    if not sphere:
        return None
    sphere.set_field("Radius", float(sphere_radius * havok_scale))

    transform = nif.add_block("bhkTransformShape")
    if not transform:
        return None

    center_havok = centroid * havok_scale
    transform.set_field("Shape", sphere.block_id)
    transform.set_field("Transform", {
        "m11": 1.0, "m12": 0.0, "m13": 0.0, "m14": float(center_havok[0]),
        "m21": 0.0, "m22": 1.0, "m23": 0.0, "m24": float(center_havok[1]),
        "m31": 0.0, "m32": 0.0, "m33": 1.0, "m34": float(center_havok[2]),
        "m41": 0.0, "m42": 0.0, "m43": 0.0, "m44": 1.0,
    })
    transform.set_field("Radius", radius)
    return transform.block_id, sphere.block_id


def pick_best_primitive(nif, verts_nif: np.ndarray,
                        radius: float = DEFAULT_RADIUS) -> int | None:
    """Auto-select and create the best-fit primitive for a vertex cloud.

    Decision logic based on PCA elongation ratio:
      > 2.5  → capsule (cylindrical/elongated)
      1.5-2.5 → cylinder
      < 1.5 and good box fit → box
      else → convex hull

    Returns the block_id of the created shape (or the transform wrapper), or None.
    """
    if len(verts_nif) < 3:
        return None

    ratio, center, axis, projections = _elongation_ratio(verts_nif)

    if ratio > 2.5:
        return create_capsule_shape(nif, verts_nif, radius)

    if ratio > 1.5:
        return create_cylinder_shape(nif, verts_nif, radius)

    # Check box fit — ratio of AABB volume to convex hull volume
    # Approximate: if the shape is roughly cuboid, use a box
    mins = verts_nif.min(axis=0)
    maxs = verts_nif.max(axis=0)
    extents = maxs - mins
    # Sort extents — if they're all similar, it's box-like
    sorted_ext = np.sort(extents)
    if sorted_ext[0] > 1e-6:
        box_ratio = sorted_ext[2] / sorted_ext[0]  # max / min extent
        if box_ratio < 2.0:
            # Roughly cuboid — use box
            result = _create_box_shape(nif, verts_nif, radius)
            if result is not None:
                return result[0]  # transform_id

    # Fallback: convex hull
    return _create_convex_shape(nif, verts_nif, radius)


def create_optimized_collision(nif, verts_nif: np.ndarray,
                               tri_indices: np.ndarray | None = None,
                               radius: float = DEFAULT_RADIUS) -> list[int]:
    """Optimized decomposition: dominant primitive + residual convex hulls.

    1. Fit best primitive via PCA
    2. Classify vertices as inliers (within shape + tolerance) or outliers
    3. If outliers < 10%, return just the primitive
    4. Cluster outlier vertices spatially (triangle adjacency if available)
    5. Create convex hull per cluster (min 4 verts)

    Returns list of shape block_ids (first is the dominant primitive).
    """
    if len(verts_nif) < 4:
        # Too few vertices for convex hull — try capsule or cylinder instead
        if len(verts_nif) >= 2:
            shape_id = create_capsule_shape(nif, verts_nif, radius)
            return [shape_id] if shape_id is not None else []
        return []

    # Step 1: Fit PCA axis and determine dominant shape
    ratio, center, axis, projections = _elongation_ratio(verts_nif)
    centered = verts_nif - center
    perp_dists = _perpendicular_distances(centered, axis, projections)

    proj_min = float(projections.min())
    proj_max = float(projections.max())
    max_perp = float(perp_dists.max()) if len(perp_dists) > 0 else 0.1

    # Step 2: Classify inliers/outliers using the fitted cylinder envelope
    # A vertex is an inlier if it's within the cylinder + tolerance
    tolerance = max_perp * 0.15  # 15% of max perpendicular distance
    is_inlier = perp_dists <= (max_perp + tolerance)

    outlier_mask = ~is_inlier
    n_outliers = int(outlier_mask.sum())

    # Step 3: Create the dominant primitive; stop there if outliers are under 10%
    shape_ids = []
    if ratio > 2.0:
        prim_id = create_capsule_shape(nif, verts_nif, radius)
    elif ratio > 1.3:
        prim_id = create_cylinder_shape(nif, verts_nif, radius)
    else:
        result = _create_box_shape(nif, verts_nif, radius)
        prim_id = result[0] if result is not None else None

    if prim_id is None:
        # Fallback to single convex hull
        hull_id = _create_convex_shape(nif, verts_nif, radius)
        return [hull_id] if hull_id is not None else []

    shape_ids.append(prim_id)

    if n_outliers < len(verts_nif) * 0.10:
        return shape_ids

    # Step 4: Cluster outlier vertices
    outlier_indices = np.where(outlier_mask)[0]
    outlier_verts = verts_nif[outlier_indices]

    if tri_indices is not None and len(tri_indices) > 0:
        # Use triangle adjacency for clustering
        clusters = _cluster_by_adjacency(outlier_indices, tri_indices)
    else:
        # Spatial clustering fallback — simple distance-based grouping
        clusters = _cluster_spatial(outlier_verts)

    # Step 5: Create convex hull per cluster (min 4 vertices)
    for cluster_verts in clusters:
        if len(cluster_verts) < 4:
            continue
        hull_id = _create_convex_shape(nif, cluster_verts, radius)
        if hull_id is not None:
            shape_ids.append(hull_id)

    return shape_ids


def _cluster_by_adjacency(outlier_indices: np.ndarray,
                           tri_indices: np.ndarray) -> list[np.ndarray]:
    """Cluster outlier vertices using triangle adjacency (connected components).

    Args:
        outlier_indices: Global vertex indices that are outliers
        tri_indices: Nx3 triangle index array

    Returns:
        List of vertex position arrays, one per cluster
    """
    outlier_set = set(int(i) for i in outlier_indices)

    # Build adjacency graph among outlier vertices
    adjacency: dict[int, set[int]] = {i: set() for i in outlier_set}
    for tri in tri_indices:
        tri_verts = [int(tri[j]) for j in range(3)]
        outlier_tri = [v for v in tri_verts if v in outlier_set]
        for i in range(len(outlier_tri)):
            for j in range(i + 1, len(outlier_tri)):
                adjacency[outlier_tri[i]].add(outlier_tri[j])
                adjacency[outlier_tri[j]].add(outlier_tri[i])

    # Connected components via BFS
    visited: set[int] = set()
    components: list[list[int]] = []
    for start in outlier_set:
        if start in visited:
            continue
        component = []
        queue = [start]
        while queue:
            v = queue.pop()
            if v in visited:
                continue
            visited.add(v)
            component.append(v)
            queue.extend(adjacency[v] - visited)
        components.append(component)

    return [np.array(comp) for comp in components]


def _cluster_spatial(verts: np.ndarray, threshold: float | None = None) -> list[np.ndarray]:
    """Simple distance-based spatial clustering.

    Uses a greedy approach: assign each vertex to the nearest cluster
    if within threshold, else start a new cluster.
    """
    if len(verts) == 0:
        return []

    if threshold is None:
        # Auto threshold: 20% of the bounding box diagonal
        extents = verts.max(axis=0) - verts.min(axis=0)
        threshold = float(np.linalg.norm(extents)) * 0.20

    clusters: list[list[int]] = []
    centroids: list[np.ndarray] = []

    for i in range(len(verts)):
        assigned = False
        for ci, centroid in enumerate(centroids):
            if np.linalg.norm(verts[i] - centroid) < threshold:
                clusters[ci].append(i)
                # Update centroid
                centroids[ci] = verts[np.array(clusters[ci])].mean(axis=0)
                assigned = True
                break
        if not assigned:
            clusters.append([i])
            centroids.append(verts[i].copy())

    return [verts[np.array(c)] for c in clusters]
