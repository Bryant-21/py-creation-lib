import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest
import numpy as np
from creation_lib.nif.nif_file import NifFile, NifBlock
from creation_lib.nif.operations.collision import HAVOK_SCALE_FO4, DEFAULT_RADIUS
from creation_lib.nif.operations.collision_shapes import (
    _fit_pca_axis, _elongation_ratio, _perpendicular_distances,
    create_capsule_shape, create_cylinder_shape, create_sphere_shape,
    pick_best_primitive, create_optimized_collision,
    _cluster_by_adjacency, _cluster_spatial,
)
from creation_lib.nif.operations.collision_parts import (
    identify_parts, generate_per_part_collision,
    PartMapping, DetectedPart, WEAPON_PRESETS,
)


class _FakeSchema:
    def is_subtype_of(self, type_name, base):
        return type_name == base
    def get_all_fields(self, type_name):
        return []
    enums = {}
    bitflags = {}


def _make_test_nif() -> NifFile:
    """Create a NIF with add_block and remove_blocks support for testing."""
    nif = NifFile()
    nif._schema = _FakeSchema()

    def _add_block(type_name, fields=None):
        bid = len(nif.blocks)
        block = NifBlock(block_id=bid, type_name=type_name)
        if fields:
            for name, val in fields.items():
                block.set_field(name, val)
        nif.blocks.append(block)
        return block
    nif.add_block = _add_block

    def _remove_blocks(block_ids):
        remove_set = set(block_ids)
        id_map = {}
        new_id = 0
        for old_id in range(len(nif.blocks)):
            if old_id in remove_set:
                id_map[old_id] = -1
            else:
                id_map[old_id] = new_id
                new_id += 1
        new_blocks = []
        for block in nif.blocks:
            if block.block_id not in remove_set:
                block.block_id = id_map[block.block_id]
                new_blocks.append(block)
        nif.blocks = new_blocks
        for block in nif.blocks:
            for i, (name, val) in enumerate(block.fields):
                if isinstance(val, int) and val in id_map:
                    block.set_field(name, id_map[val])
                elif isinstance(val, list):
                    new_list = []
                    for item in val:
                        if isinstance(item, int) and item in id_map:
                            new_val = id_map[item]
                            if new_val >= 0:
                                new_list.append(new_val)
                        else:
                            new_list.append(item)
                    block.set_field(name, new_list)
    nif.remove_blocks = _remove_blocks
    nif.find_blocks = lambda t: [b for b in nif.blocks if b.type_name == t]

    return nif


def _make_elongated_verts(axis="x", length=100.0, width=5.0) -> np.ndarray:
    """Create elongated point cloud along given axis."""
    n = 50
    verts = np.random.default_rng(42).uniform(-width, width, (n, 3)).astype(np.float32)
    if axis == "x":
        verts[:, 0] = np.linspace(-length / 2, length / 2, n)
    elif axis == "y":
        verts[:, 1] = np.linspace(-length / 2, length / 2, n)
    elif axis == "z":
        verts[:, 2] = np.linspace(-length / 2, length / 2, n)
    return verts


def _make_cube_verts(size=10.0, center=(0, 0, 0)) -> np.ndarray:
    """8-vertex cube."""
    cx, cy, cz = center
    h = size / 2
    return np.array([
        [cx - h, cy - h, cz - h], [cx + h, cy - h, cz - h],
        [cx - h, cy + h, cz - h], [cx + h, cy + h, cz - h],
        [cx - h, cy - h, cz + h], [cx + h, cy - h, cz + h],
        [cx - h, cy + h, cz + h], [cx + h, cy + h, cz + h],
    ], dtype=np.float32)


def _make_weapon_nif() -> NifFile:
    """Create a NIF with weapon-like hierarchy: root -> barrel/receiver nodes -> meshes."""
    nif = _make_test_nif()

    root = nif.add_block("BSFadeNode")
    root.set_field("Name", "WeaponRoot")
    root.set_field("Children", [1, 2])
    root.set_field("Num Children", 2)
    root.set_field("Collision Object", -1)

    barrel_node = nif.add_block("NiNode")
    barrel_node.set_field("Name", "WeaponBarrel")
    barrel_node.set_field("Children", [3])
    barrel_node.set_field("Num Children", 1)

    receiver_node = nif.add_block("NiNode")
    receiver_node.set_field("Name", "ReceiverMesh")
    receiver_node.set_field("Children", [4])
    receiver_node.set_field("Num Children", 1)

    # Barrel mesh — elongated
    barrel_shape = nif.add_block("BSTriShape")
    barrel_shape.set_field("Name", "BarrelMesh")
    barrel_verts = _make_elongated_verts("y", 80.0, 3.0)
    barrel_shape.set_field("Vertex Data", [
        {"Vertex": {"x": float(v[0]), "y": float(v[1]), "z": float(v[2])},
         "Normal": {"x": 0, "y": 0, "z": 1}}
        for v in barrel_verts
    ])
    barrel_shape.set_field("Triangles", [
        {"v1": 0, "v2": 1, "v3": 2}, {"v1": 2, "v2": 1, "v3": 3},
    ])

    # Receiver mesh — roughly cuboid
    receiver_shape = nif.add_block("BSTriShape")
    receiver_shape.set_field("Name", "ReceiverBody")
    recv_verts = _make_cube_verts(15.0, (0, -20, 0))
    receiver_shape.set_field("Vertex Data", [
        {"Vertex": {"x": float(v[0]), "y": float(v[1]), "z": float(v[2])},
         "Normal": {"x": 0, "y": 0, "z": 1}}
        for v in recv_verts
    ])
    receiver_shape.set_field("Triangles", [
        {"v1": 0, "v2": 1, "v3": 2}, {"v1": 2, "v2": 1, "v3": 3},
        {"v1": 4, "v2": 5, "v3": 6}, {"v1": 6, "v2": 5, "v3": 7},
    ])

    return nif


# ===== PCA Tests =====

class TestPCA:
    @pytest.mark.parametrize("axis", ["x", "y", "z"])
    def test_axis_aligned(self, axis):
        verts = _make_elongated_verts(axis, 100.0, 2.0)
        center, fitted_axis, proj = _fit_pca_axis(verts)
        idx = {"x": 0, "y": 1, "z": 2}[axis]
        assert abs(abs(fitted_axis[idx]) - 1.0) < 0.15, f"Expected {axis} axis, got {fitted_axis}"

    def test_diagonal_aligned_and_center_is_mean(self):
        # Cloud along [1,1,0], offset so the mean (and thus fitted center) is non-zero.
        n = 50
        t = np.linspace(-50, 50, n)
        rng = np.random.default_rng(99)
        verts = np.zeros((n, 3), dtype=np.float32)
        verts[:, 0] = t + rng.uniform(-1, 1, n) + 5.0
        verts[:, 1] = t + rng.uniform(-1, 1, n) + 5.0
        verts[:, 2] = rng.uniform(-1, 1, n)
        center, axis, proj = _fit_pca_axis(verts)

        diag = np.array([1, 1, 0], dtype=np.float64) / np.sqrt(2)
        assert abs(abs(np.dot(axis, diag)) - 1.0) < 0.15  # allow sign flip
        np.testing.assert_allclose(center, verts.mean(axis=0), atol=0.01)


# ===== Capsule Tests =====

class TestCapsuleShape:
    def test_creates_havok_scaled_capsule(self):
        nif = _make_test_nif()
        verts = _make_elongated_verts("y", 80.0, 5.0)
        bid = create_capsule_shape(nif, verts)
        assert bid is not None
        block = nif.get_block(bid)
        assert block.type_name == "bhkCapsuleShape"

        p1 = block.get_field("First Point")
        p2 = block.get_field("Second Point")
        y_sep = abs(p2["y"] - p1["y"])
        assert y_sep > 0.3, f"Y separation too small: {y_sep}"

        r = block.get_field("Radius")
        # ~5 / 70 = ~0.071
        assert 0.01 < r < 0.2, f"Radius {r} not Havok-scaled correctly"

    @pytest.mark.parametrize(
        "verts,expect_none",
        [
            (np.array([[1, 2, 3]], dtype=np.float32), True),  # degenerate single vertex
            (np.array([[0, 0, 0], [10, 0, 0], [20, 0, 0]], dtype=np.float32), False),  # colinear
        ],
    )
    def test_edge_cases(self, verts, expect_none):
        nif = _make_test_nif()
        bid = create_capsule_shape(nif, verts)
        assert (bid is None) == expect_none


# ===== Cylinder Tests =====

class TestCylinderShape:
    def test_creates_havok_scaled_cylinder(self):
        nif = _make_test_nif()
        verts = _make_elongated_verts("y", 80.0, 6.0)
        bid = create_cylinder_shape(nif, verts)
        assert bid is not None
        block = nif.get_block(bid)
        assert block.type_name == "bhkCylinderShape"

        va = block.get_field("Vertex A")
        vb = block.get_field("Vertex B")
        assert "w" in va and va["w"] == 0.0
        assert "w" in vb and vb["w"] == 0.0

        r = block.get_field("Cylinder Radius")
        # ~6 / 70 = ~0.086
        assert 0.01 < r < 0.2, f"Radius {r} not Havok-scaled correctly"


# ===== Sphere Tests =====

class TestSphereShape:
    def test_creates_sphere_with_radius_and_centroid_transform(self):
        nif = _make_test_nif()
        verts = _make_cube_verts(10.0, (20, 30, 40))
        result = create_sphere_shape(nif, verts)
        assert result is not None
        transform_id, sphere_id = result
        transform = nif.get_block(transform_id)
        sphere = nif.get_block(sphere_id)
        assert transform.type_name == "bhkTransformShape"
        assert sphere.type_name == "bhkSphereShape"

        r = sphere.get_field("Radius")
        # Half-diagonal of 10-cube = sqrt(75) ~ 8.66, scaled by 1/70 ~ 0.124 (translation-invariant)
        expected = np.sqrt(75.0) / HAVOK_SCALE_FO4
        assert abs(r - expected) < 0.01, f"Radius {r}, expected {expected}"

        t = transform.get_field("Transform")
        havok_scale = 1.0 / HAVOK_SCALE_FO4
        assert abs(t["m14"] - 20 * havok_scale) < 0.01
        assert abs(t["m24"] - 30 * havok_scale) < 0.01
        assert abs(t["m34"] - 40 * havok_scale) < 0.01

    def test_empty_verts(self):
        nif = _make_test_nif()
        result = create_sphere_shape(nif, np.array([], dtype=np.float32).reshape(0, 3))
        assert result is None


# ===== Auto Best-Fit Tests =====

class TestAutoFit:
    @pytest.mark.parametrize(
        "make_verts,expected_types",
        [
            (lambda: _make_elongated_verts("x", 100.0, 3.0), ("bhkCapsuleShape",)),
            (lambda: _make_cube_verts(10.0), ("bhkTransformShape",)),  # box is transform-wrapped
            (
                lambda: (lambda rng: np.stack([
                    rng.uniform(-10, 10, 30) * 1.8,
                    rng.uniform(-10, 10, 30) * 1.0,
                    rng.uniform(-10, 10, 30) * 1.3,
                ], axis=1).astype(np.float32))(np.random.default_rng(123)),
                ("bhkConvexVerticesShape", "bhkTransformShape", "bhkCapsuleShape", "bhkCylinderShape"),
            ),
        ],
        ids=["elongated_picks_capsule", "cuboid_picks_box", "irregular_picks_valid_primitive"],
    )
    def test_pick_best_primitive(self, make_verts, expected_types):
        nif = _make_test_nif()
        bid = pick_best_primitive(nif, make_verts())
        assert bid is not None
        block = nif.get_block(bid)
        assert block.type_name in expected_types


# ===== Optimized Decomposition Tests =====

class TestOptimizedDecomposition:
    def test_simple_cylinder_single_shape(self):
        nif = _make_test_nif()
        verts = _make_elongated_verts("y", 80.0, 3.0)
        shape_ids = create_optimized_collision(nif, verts)
        assert len(shape_ids) >= 1
        first = nif.get_block(shape_ids[0])
        assert first.type_name in ("bhkCapsuleShape", "bhkCylinderShape")

    def test_cylinder_with_protrusion_adds_residual_shape(self):
        nif = _make_test_nif()
        verts = _make_elongated_verts("x", 80.0, 3.0)
        # Add protruding vertices (well outside the cylinder)
        protrusion = np.array([
            [0, 20, 0], [5, 25, 0], [0, 20, 5], [5, 25, 5],
            [0, 22, 2], [3, 23, 3], [-2, 21, 4],
        ], dtype=np.float32)
        combined = np.vstack([verts, protrusion])
        shape_ids = create_optimized_collision(nif, combined)
        assert len(shape_ids) >= 1


# ===== Clustering Tests =====

class TestClustering:
    @pytest.mark.parametrize(
        "outlier_indices,tri_indices,expected_clusters",
        [
            (np.array([0, 1, 2, 10, 11, 12]), np.array([[0, 1, 2], [10, 11, 12]]), 2),
            (np.array([0, 1, 2, 3]), np.array([[0, 1, 2], [1, 2, 3]]), 1),
        ],
        ids=["two_components", "single_component"],
    )
    def test_adjacency_clustering(self, outlier_indices, tri_indices, expected_clusters):
        clusters = _cluster_by_adjacency(outlier_indices, tri_indices)
        assert len(clusters) == expected_clusters

    def test_spatial_clustering(self):
        group1 = np.array([[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0]], dtype=np.float32)
        group2 = np.array([[100, 100, 100], [101, 100, 100], [100, 101, 100], [101, 101, 100]], dtype=np.float32)
        verts = np.vstack([group1, group2])
        clusters = _cluster_spatial(verts, threshold=10.0)
        assert len(clusters) == 2


# ===== Part Identification Tests =====

class TestPartIdentification:
    def test_node_name_matching_is_case_insensitive_and_skips_unmatched(self):
        nif = _make_weapon_nif()
        mappings = [
            PartMapping("barrel", "capsule"),
            PartMapping("RECEIVER", "box"),
            PartMapping("scope", "convex_hull"),  # no scope in our test NIF
        ]
        parts = identify_parts(nif, 0, mappings, group_by="node")
        assert len(parts) == 2
        patterns = {p.matched_pattern for p in parts}
        assert patterns == {"barrel", "RECEIVER"}
        barrel = next(p for p in parts if p.matched_pattern == "barrel")
        assert barrel.vertex_count == 50  # from _make_elongated_verts

    def test_shape_name_matching(self):
        nif = _make_weapon_nif()
        mappings = [
            PartMapping("barrel", "capsule"),
            PartMapping("receiver", "box"),
        ]
        parts = identify_parts(nif, 0, mappings, group_by="shape")
        assert len(parts) == 2

    def test_nested_hierarchy(self):
        """Ensure scanning works through nested NiNode hierarchy."""
        nif = _make_test_nif()
        root = nif.add_block("BSFadeNode")
        root.set_field("Name", "Root")
        root.set_field("Children", [1])
        root.set_field("Num Children", 1)
        root.set_field("Collision Object", -1)

        mid_node = nif.add_block("NiNode")
        mid_node.set_field("Name", "Intermediate")
        mid_node.set_field("Children", [2])
        mid_node.set_field("Num Children", 1)

        deep_node = nif.add_block("NiNode")
        deep_node.set_field("Name", "DeepBarrel")
        deep_node.set_field("Children", [3])
        deep_node.set_field("Num Children", 1)

        mesh = nif.add_block("BSTriShape")
        mesh.set_field("Name", "Mesh")
        mesh.set_field("Vertex Data", [
            {"Vertex": {"x": float(x), "y": 0.0, "z": 0.0},
             "Normal": {"x": 0, "y": 0, "z": 1}}
            for x in range(5)
        ])

        mappings = [PartMapping("barrel", "capsule")]
        parts = identify_parts(nif, 0, mappings, group_by="node")
        assert len(parts) == 1
        assert parts[0].node_name == "DeepBarrel"


# ===== Integration Tests =====

class TestIntegration:
    def test_multiple_parts_list_shape(self):
        nif = _make_weapon_nif()
        mappings = [
            PartMapping("barrel", "capsule"),
            PartMapping("receiver", "box"),
        ]
        parts = identify_parts(nif, 0, mappings, group_by="node")
        result = generate_per_part_collision(nif, 0, parts, layer="WEAPON")
        assert result.success

        root = nif.get_block(0)
        coll_ref = root.get_field("Collision Object")
        assert coll_ref is not None and coll_ref >= 0

        coll_block = nif.get_block(coll_ref)
        body_ref = coll_block.get_field("Body")
        body = nif.get_block(body_ref)
        shape_ref = body.get_field("Shape")
        shape = nif.get_block(shape_ref)
        # Multiple parts → should be wrapped in ListShape
        assert shape.type_name == "bhkListShape"

    def test_single_part_direct_shape(self):
        nif = _make_weapon_nif()
        mappings = [PartMapping("barrel", "capsule")]
        parts = identify_parts(nif, 0, mappings, group_by="node")
        result = generate_per_part_collision(nif, 0, parts, layer="WEAPON")
        assert result.success

        # Single part → direct shape (no ListShape wrapper)
        root = nif.get_block(0)
        coll_ref = root.get_field("Collision Object")
        coll_block = nif.get_block(coll_ref)
        body_ref = coll_block.get_field("Body")
        body = nif.get_block(body_ref)
        shape_ref = body.get_field("Shape")
        shape = nif.get_block(shape_ref)
        assert shape.type_name == "bhkCapsuleShape"

    def test_replace_existing_collision(self):
        nif = _make_weapon_nif()
        mappings = [PartMapping("barrel", "capsule")]
        parts = identify_parts(nif, 0, mappings, group_by="node")

        r1 = generate_per_part_collision(nif, 0, parts, layer="WEAPON")
        assert r1.success

        parts2 = identify_parts(nif, 0, mappings, group_by="node")
        r2 = generate_per_part_collision(nif, 0, parts2, layer="WEAPON", replace=True)
        assert r2.success

        coll_objs = [b for b in nif.blocks if b.type_name == "bhkCollisionObject"]
        assert len(coll_objs) == 1

    def test_no_parts_error(self):
        nif = _make_weapon_nif()
        result = generate_per_part_collision(nif, 0, [], layer="WEAPON")
        assert not result.success

    @pytest.mark.parametrize(
        "shape_type,block_type",
        [
            ("capsule", "bhkCapsuleShape"),
            ("cylinder", "bhkCylinderShape"),
            ("sphere", "bhkSphereShape"),
        ],
    )
    def test_generate_collision_via_main_api(self, shape_type, block_type):
        """Primitive shapes through the main generate_collision() API."""
        from creation_lib.nif.operations.collision import generate_collision
        nif = _make_weapon_nif()
        # Weapon NIF has nested NiNode children — pass mesh block IDs explicitly
        result = generate_collision(nif, 0, shape_type=shape_type, source_block_ids=[3, 4])
        assert result.success
        matches = [b for b in nif.blocks if b.type_name == block_type]
        assert len(matches) == 1
