import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from collections import Counter
from pathlib import Path

import pytest
import numpy as np
import math
from creation_lib.nif.nif_file import NifFile, NifBlock
from creation_lib.nif.operations.collision import (
    create_convex_hull, convert_collision_shape,
    generate_collision, generate_collision_from_geometry, remove_collision,
    HAVOK_SCALE, HAVOK_SCALE_FO4, FO4_LAYERS, get_collision_layers,
    _extract_vertices, _create_convex_shape,
)
from creation_lib.core.game_profiles import get_profile




class _FakeSchema:
    """Minimal schema stub for collision tests."""
    def is_subtype_of(self, type_name, base):
        return type_name == base
    def get_all_fields(self, type_name):
        return []
    enums = {}
    bitflags = {}


def _make_cube_nif() -> NifFile:
    """Create a NIF with a BSFadeNode root and BSTriShape child containing cube vertices."""
    nif = NifFile()
    nif._schema = _FakeSchema()

    root = NifBlock(block_id=0, type_name="BSFadeNode")
    root.set_field("Children", [1])
    root.set_field("Num Children", 1)
    root.set_field("Collision Object", -1)
    nif.blocks.append(root)

    shape = NifBlock(block_id=1, type_name="BSTriShape")
    verts = []
    for x in (0.0, 10.0):
        for y in (0.0, 10.0):
            for z in (0.0, 10.0):
                verts.append({
                    "Vertex": {"x": x, "y": y, "z": z},
                    "Normal": {"x": 0, "y": 0, "z": 1},
                })
    shape.set_field("Vertex Data", verts)
    shape.set_field("Triangles", [
        {"v1": 0, "v2": 1, "v3": 2},
        {"v1": 2, "v2": 1, "v3": 3},
        {"v1": 4, "v2": 5, "v3": 6},
        {"v1": 6, "v2": 5, "v3": 7},
    ])
    nif.blocks.append(shape)

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
        # Remap refs in fields (simplified)
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


def _add_cube_shape(nif: NifFile, block_id: int, *, x_offset: float = 0.0) -> NifBlock:
    shape = NifBlock(block_id=block_id, type_name="BSTriShape")
    shape.set_field("Vertex Data", [
        {
            "Vertex": {"x": x + x_offset, "y": y, "z": z},
            "Normal": {"x": 0, "y": 0, "z": 1},
        }
        for x in (0.0, 10.0)
        for y in (0.0, 10.0)
        for z in (0.0, 10.0)
    ])
    shape.set_field("Triangles", [
        {"v1": 0, "v2": 1, "v3": 2},
        {"v1": 2, "v2": 1, "v3": 3},
        {"v1": 4, "v2": 5, "v3": 6},
        {"v1": 6, "v2": 5, "v3": 7},
    ])
    return shape


# --- Legacy API Tests ---
def test_convex_hull_succeeds():
    nif = _make_cube_nif()
    result = create_convex_hull(nif, 1)
    assert result.success
    assert len(result.modified_block_ids) == 1
def test_convex_hull_creates_block():
    nif = _make_cube_nif()
    initial_count = len(nif.blocks)
    create_convex_hull(nif, 1)
    assert len(nif.blocks) == initial_count + 1
    new_block = nif.blocks[-1]
    assert new_block.type_name == "bhkConvexVerticesShape"
def test_convex_hull_has_vertices():
    nif = _make_cube_nif()
    create_convex_hull(nif, 1)
    new_block = nif.blocks[-1]
    hull_verts = new_block.get_field("Vertices")
    assert hull_verts is not None
    assert len(hull_verts) == 8  # all 8 cube vertices are on the hull
def test_convex_hull_havok_scale():
    """Vertices should be scaled by 1/HAVOK_SCALE_FO4 (~1/70), NOT 1/7."""
    nif = _make_cube_nif()
    create_convex_hull(nif, 1)
    new_block = nif.blocks[-1]
    hull_verts = new_block.get_field("Vertices")
    # Original cube goes from 0 to 10. Scaled should be 0 to ~0.143 (10/70)
    max_coord = max(abs(v["x"]) for v in hull_verts)
    # With correct scale (1/70), max should be ~0.143
    # With old bug (1/7), max would be ~1.43
    assert max_coord < 0.2, f"Havok scale bug: max_coord={max_coord} (expected ~0.143, got >0.2 means 1/7 bug)"
    assert max_coord > 0.1, f"max_coord={max_coord} too small"


def test_convex_hull_invalid_block():
    nif = _make_cube_nif()
    result = create_convex_hull(nif, 99)
    assert not result.success


def test_convex_hull_no_vertex_data():
    nif = NifFile()
    block = NifBlock(block_id=0, type_name="BSTriShape")
    nif.blocks.append(block)
    result = create_convex_hull(nif, 0)
    assert not result.success


def test_convert_collision_shape_not_implemented():
    nif = NifFile()
    result = convert_collision_shape(nif, 0, "bhkBoxShape")
    assert not result.success


# --- New API Tests ---
def test_generate_collision_convex_hull():
    """Full hierarchy: node -> CollisionObject -> RigidBody -> ConvexVerticesShape."""
    nif = _make_cube_nif()
    result = generate_collision(nif, node_block_id=0, shape_type="convex_hull")
    assert result.success

    # Check root has collision object reference
    root = nif.get_block(0)
    coll_ref = root.get_field("Collision Object")
    assert coll_ref is not None and coll_ref >= 0

    # Find the collision object block
    coll_block = nif.get_block(coll_ref)
    assert coll_block.type_name == "bhkCollisionObject"
    assert coll_block.get_field("Target") == 0  # points back to root
    assert coll_block.get_field("Flags") == 0x81

    # Find rigid body
    body_ref = coll_block.get_field("Body")
    assert body_ref >= 0
    body_block = nif.get_block(body_ref)
    assert body_block.type_name == "bhkRigidBody"

    # Find shape
    shape_ref = body_block.get_field("Shape")
    assert shape_ref >= 0
    shape_block = nif.get_block(shape_ref)
    assert shape_block.type_name == "bhkConvexVerticesShape"
def test_generate_collision_box():
    """Box shape with bhkTransformShape wrapper."""
    nif = _make_cube_nif()
    result = generate_collision(nif, node_block_id=0, shape_type="box")
    assert result.success

    # Find the transform shape through the hierarchy
    root = nif.get_block(0)
    coll_ref = root.get_field("Collision Object")
    coll_block = nif.get_block(coll_ref)
    body_ref = coll_block.get_field("Body")
    body_block = nif.get_block(body_ref)
    shape_ref = body_block.get_field("Shape")
    shape_block = nif.get_block(shape_ref)
    assert shape_block.type_name == "bhkTransformShape"

    # Find the box shape inside the transform
    box_ref = shape_block.get_field("Shape")
    box_block = nif.get_block(box_ref)
    assert box_block.type_name == "bhkBoxShape"
    dims = box_block.get_field("Dimensions")
    assert dims is not None
def test_generate_collision_list():
    """Compound collision from multiple meshes -- should create bhkConvexVerticesShape(s)."""
    # Add a second shape to the NIF
    nif = _make_cube_nif()
    shape2 = nif.add_block("BSTriShape")
    verts = []
    for x in (20.0, 30.0):
        for y in (20.0, 30.0):
            for z in (20.0, 30.0):
                verts.append({
                    "Vertex": {"x": x, "y": y, "z": z},
                    "Normal": {"x": 0, "y": 0, "z": 1},
                })
    shape2.set_field("Vertex Data", verts)
    shape2.set_field("Triangles", [
        {"v1": 0, "v2": 1, "v3": 2},
        {"v1": 2, "v2": 1, "v3": 3},
    ])

    # Update root children
    root = nif.get_block(0)
    root.set_field("Children", [1, shape2.block_id])
    root.set_field("Num Children", 2)

    result = generate_collision(nif, node_block_id=0, shape_type="list")
    assert result.success

    # Should create a list shape wrapping two convex hulls
    root = nif.get_block(0)
    coll_ref = root.get_field("Collision Object")
    coll_block = nif.get_block(coll_ref)
    body_ref = coll_block.get_field("Body")
    body_block = nif.get_block(body_ref)
    shape_ref = body_block.get_field("Shape")
    shape_block = nif.get_block(shape_ref)
    assert shape_block.type_name == "bhkListShape"
def test_havok_scale_fo4():
    """Vertices should be divided by ~70, not 7 (regression test)."""
    nif = _make_cube_nif()
    result = generate_collision(nif, node_block_id=0, shape_type="convex_hull")
    assert result.success

    # Find the convex shape
    cvs_blocks = [b for b in nif.blocks if b.type_name == "bhkConvexVerticesShape"]
    assert len(cvs_blocks) == 1
    hull_verts = cvs_blocks[0].get_field("Vertices")
    assert hull_verts

    # Original vertices go from 0 to 10
    # With correct scale (1/70): max = 10/70 ~ 0.143
    # With bug scale (1/7): max = 10/7 ~ 1.43
    max_coord = max(
        max(abs(v["x"]), abs(v["y"]), abs(v["z"]))
        for v in hull_verts
    )
    expected = 10.0 / HAVOK_SCALE_FO4
    assert abs(max_coord - expected) < 0.01, \
        f"Havok scale wrong: got {max_coord}, expected {expected}"
def test_generate_collision_replaces_existing():
    """replace=True should remove old collision before creating new."""
    nif = _make_cube_nif()

    # Generate first collision
    result1 = generate_collision(nif, node_block_id=0, shape_type="convex_hull")
    assert result1.success
    count_after_first = len(nif.blocks)

    # Generate again with replace=True
    result2 = generate_collision(nif, node_block_id=0, shape_type="box", replace=True)
    assert result2.success

    # Should not have accumulated collision blocks
    # The new collision should have replaced the old one
    root = nif.get_block(0)
    coll_ref = root.get_field("Collision Object")
    assert coll_ref >= 0
    coll_block = nif.get_block(coll_ref)
    assert coll_block.type_name == "bhkCollisionObject"
def test_remove_collision():
    """Clean removal + cleared ref."""
    nif = _make_cube_nif()
    generate_collision(nif, node_block_id=0, shape_type="convex_hull")

    # Verify collision exists
    root = nif.get_block(0)
    assert root.get_field("Collision Object") >= 0

    # Remove it
    result = remove_collision(nif, node_block_id=0)
    assert result.success

    # Verify it's gone
    root = nif.get_block(0)
    coll_ref = root.get_field("Collision Object")
    assert coll_ref is None or coll_ref < 0

    # No collision blocks should remain
    coll_types = {"bhkCollisionObject", "bhkRigidBody", "bhkConvexVerticesShape"}
    for block in nif.blocks:
        assert block.type_name not in coll_types, \
            f"Collision block {block.type_name} still present after removal"
def test_generate_collision_sets_layer():
    """Layer should propagate to HavokFilter."""
    nif = _make_cube_nif()
    result = generate_collision(nif, node_block_id=0, layer="WEAPON")
    assert result.success

    # Find rigid body and check layer
    rb_blocks = [b for b in nif.blocks if b.type_name == "bhkRigidBody"]
    assert len(rb_blocks) == 1
    havok_filter = rb_blocks[0].get_field("Havok Filter")
    assert havok_filter is not None
    assert (
        havok_filter.get("Layer:FO4", havok_filter.get("Layer"))
        == FO4_LAYERS["WEAPON"]
    )
def test_generate_collision_from_geometry_fo4_emits_np_collision_object():
    nif = _make_cube_nif()
    profile = get_profile("fo4")
    result = generate_collision_from_geometry(
        nif,
        node_block_id=0,
        vertices=[
            {"x": -5.0, "y": -5.0, "z": 0.0},
            {"x": 5.0, "y": -5.0, "z": 0.0},
            {"x": 0.0, "y": 5.0, "z": 0.0},
            {"x": 0.0, "y": 0.0, "z": 10.0},
        ],
        shape_type="convex_hull",
        replace=True,
        profile=profile,
    )
    assert result.success
    root = nif.get_block(0)
    assert root.get_field("Collision Object") >= 0
    assert any(block.type_name == "bhkNPCollisionObject" for block in nif.blocks)
    assert any(block.type_name == "bhkPhysicsSystem" for block in nif.blocks)


def test_generate_collision_no_meshes():
    """Should error when no BSTriShape children found."""
    nif = NifFile()
    nif._schema = _FakeSchema()
    root = NifBlock(block_id=0, type_name="BSFadeNode")
    root.set_field("Children", [])
    root.set_field("Collision Object", -1)
    nif.blocks.append(root)

    def _add_block(type_name, fields=None):
        bid = len(nif.blocks)
        block = NifBlock(block_id=bid, type_name=type_name)
        nif.blocks.append(block)
        return block
    nif.add_block = _add_block

    result = generate_collision(nif, node_block_id=0)
    assert not result.success
    assert "No BSTriShape" in result.description


def test_remove_collision_no_collision():
    """Should error when no collision exists."""
    nif = NifFile()
    nif._schema = _FakeSchema()
    root = NifBlock(block_id=0, type_name="BSFadeNode")
    root.set_field("Collision Object", -1)
    nif.blocks.append(root)

    result = remove_collision(nif, node_block_id=0)
    assert not result.success


def test_extract_vertices():
    """Test vertex extraction from BSTriShape."""
    nif = _make_cube_nif()
    verts = _extract_vertices(nif, 1)
    assert verts is not None
    assert verts.shape == (8, 3)
    assert verts.min() == 0.0
    assert verts.max() == 10.0


def test_extract_vertices_missing():
    """Missing block returns None."""
    nif = _make_cube_nif()
    assert _extract_vertices(nif, 99) is None


def test_generate_collision_uses_selected_mesh_as_source():
    nif = _make_cube_nif()

    result = generate_collision(nif, node_block_id=1, shape_type="convex_hull")

    assert result.success, result.description


def test_generate_collision_recurses_and_applies_child_node_transform():
    nif = _make_cube_nif()
    root = nif.get_block(0)
    root.set_field("Children", [2])
    root.set_field("Num Children", 1)

    child_node = NifBlock(block_id=2, type_name="NiNode")
    child_node.set_field("Children", [3])
    child_node.set_field("Num Children", 1)
    child_node.set_field("Translation", {"x": 20.0, "y": 0.0, "z": 0.0})
    child_node.set_field("Rotation", {
        "m11": 1.0, "m21": 0.0, "m31": 0.0,
        "m12": 0.0, "m22": 1.0, "m32": 0.0,
        "m13": 0.0, "m23": 0.0, "m33": 1.0,
    })
    child_node.set_field("Scale", 1.0)
    nif.blocks.append(child_node)
    nif.blocks.append(_add_cube_shape(nif, 3))

    result = generate_collision(nif, node_block_id=0, shape_type="convex_hull")

    assert result.success, result.description
    convex = next(b for b in nif.blocks if b.type_name == "bhkConvexVerticesShape")
    hull_verts = convex.get_field("Vertices")
    max_x = max(v["x"] for v in hull_verts)
    assert max_x == pytest.approx(30.0 / HAVOK_SCALE_FO4, abs=0.01)


def test_generate_collision_can_exclude_child_ninode_meshes():
    nif = _make_cube_nif()
    root = nif.get_block(0)
    root.set_field("Children", [1, 2])
    root.set_field("Num Children", 2)

    child_node = NifBlock(block_id=2, type_name="NiNode")
    child_node.set_field("Children", [3])
    child_node.set_field("Num Children", 1)
    child_node.set_field("Translation", {"x": 20.0, "y": 0.0, "z": 0.0})
    child_node.set_field("Rotation", {
        "m11": 1.0, "m21": 0.0, "m31": 0.0,
        "m12": 0.0, "m22": 1.0, "m32": 0.0,
        "m13": 0.0, "m23": 0.0, "m33": 1.0,
    })
    child_node.set_field("Scale", 1.0)
    nif.blocks.append(child_node)
    nif.blocks.append(_add_cube_shape(nif, 3))

    result = generate_collision(
        nif,
        node_block_id=0,
        shape_type="convex_hull",
        include_child_nodes=False,
    )

    assert result.success, result.description
    convex = next(b for b in nif.blocks if b.type_name == "bhkConvexVerticesShape")
    hull_verts = convex.get_field("Vertices")
    max_x = max(v["x"] for v in hull_verts)
    assert max_x == pytest.approx(10.0 / HAVOK_SCALE_FO4, abs=0.01)


def test_generate_collision_applies_transform_for_explicit_child_source():
    nif = _make_cube_nif()
    root = nif.get_block(0)
    root.set_field("Children", [2])
    root.set_field("Num Children", 1)

    child_node = NifBlock(block_id=2, type_name="NiNode")
    child_node.set_field("Children", [3])
    child_node.set_field("Num Children", 1)
    child_node.set_field("Translation", {"x": 20.0, "y": 0.0, "z": 0.0})
    child_node.set_field("Rotation", {
        "m11": 1.0, "m21": 0.0, "m31": 0.0,
        "m12": 0.0, "m22": 1.0, "m32": 0.0,
        "m13": 0.0, "m23": 0.0, "m33": 1.0,
    })
    child_node.set_field("Scale", 1.0)
    nif.blocks.append(child_node)
    nif.blocks.append(_add_cube_shape(nif, 3))

    result = generate_collision(
        nif,
        node_block_id=0,
        source_block_ids=[3],
        shape_type="convex_hull",
    )

    assert result.success, result.description
    convex = next(b for b in nif.blocks if b.type_name == "bhkConvexVerticesShape")
    hull_verts = convex.get_field("Vertices")
    max_x = max(v["x"] for v in hull_verts)
    assert max_x == pytest.approx(30.0 / HAVOK_SCALE_FO4, abs=0.01)


def test_havok_scale_alias():
    """HAVOK_SCALE_FO4 is a backward-compat alias for HAVOK_SCALE."""
    assert HAVOK_SCALE == HAVOK_SCALE_FO4


def test_get_collision_layers_default():
    layers = get_collision_layers()
    assert layers["STATIC"] == 1
    assert layers["NPC"] == 7
    assert layers["CHARCONTROLLER"] == 30


def test_get_collision_layers_skyrim():
    profile = get_profile("skyrimse")
    layers = get_collision_layers(profile)
    assert "STATIC" in layers


# ---------------------------------------------------------------------------
# FO4 routing integration tests
# ---------------------------------------------------------------------------

_PF_MAGIC = b'\x57\xE0\xE0\x57\x10\xC0\xC0\x10'
_LEGACY_TYPES = {"bhkRigidBody", "bhkConvexVerticesShape"}
_FO4_COLL_TYPES = {"bhkNPCollisionObject", "bhkPhysicsSystem"}


def _get_fo4_profile():
    return get_profile("fo4")


def _check_fo4_result(nif, result):
    """Verify that a FO4 collision result has the right block types."""
    assert result.success, result.description
    block_types = {b.type_name for b in nif.blocks}
    assert "bhkNPCollisionObject" in block_types, \
        f"Missing bhkNPCollisionObject; have: {block_types}"
    assert "bhkPhysicsSystem" in block_types, \
        f"Missing bhkPhysicsSystem; have: {block_types}"
    assert "bhkRigidBody" not in block_types, \
        "Legacy bhkRigidBody present in FO4 collision"
    assert "bhkConvexVerticesShape" not in block_types, \
        "Legacy bhkConvexVerticesShape present in FO4 collision"


def _get_phys_sys_blob(nif):
    for block in nif.blocks:
        if block.type_name == "bhkPhysicsSystem":
            bd = block.get_field("Binary Data")
            if isinstance(bd, dict):
                data = bd.get("Data", [])
                return bytes(data)
            elif isinstance(bd, (bytes, bytearray)):
                return bytes(bd)
    return None


def _preview_extents_from_blob(blob):
    from creation_lib.havok.collision_preview import extract_preview_meshes_from_blob

    previews = extract_preview_meshes_from_blob(
        blob,
        havok_scale=HAVOK_SCALE_FO4,
        body_id=0,
    )
    vertices = np.array(
        [
            [vertex["x"], vertex["y"], vertex["z"]]
            for preview in previews
            for vertex in preview["mesh"]["vertices"]
        ],
        dtype=np.float32,
    )
    assert len(vertices) > 0
    return vertices.max(axis=0) - vertices.min(axis=0)


def _preview_nonmanifold_edge_count(blob: bytes) -> int:
    from creation_lib.havok.collision_preview import extract_preview_meshes_from_blob

    previews = extract_preview_meshes_from_blob(
        blob,
        havok_scale=HAVOK_SCALE_FO4,
        body_id=0,
    )
    assert previews
    edge_counts: Counter[tuple[int, int]] = Counter()
    for preview in previews:
        for tri in preview["mesh"]["triangles"]:
            indices = [int(tri["v1"]), int(tri["v2"]), int(tri["v3"])]
            assert len(set(indices)) == 3
            for a, b in (
                (indices[0], indices[1]),
                (indices[1], indices[2]),
                (indices[2], indices[0]),
            ):
                edge_counts[tuple(sorted((a, b)))] += 1
    return sum(1 for count in edge_counts.values() if count != 2)
def test_fo4_convex_hull_emits_np_collision_object():
    nif = _make_cube_nif()
    profile = _get_fo4_profile()
    result = generate_collision(nif, node_block_id=0, shape_type="convex_hull", profile=profile)
    _check_fo4_result(nif, result)
def test_fo4_box_emits_np_collision_object():
    nif = _make_cube_nif()
    profile = _get_fo4_profile()
    result = generate_collision(nif, node_block_id=0, shape_type="box", profile=profile)
    _check_fo4_result(nif, result)
def test_fo4_sphere_emits_np_collision_object():
    nif = _make_cube_nif()
    profile = _get_fo4_profile()
    result = generate_collision(nif, node_block_id=0, shape_type="sphere", profile=profile)
    _check_fo4_result(nif, result)
def test_fo4_list_emits_np_collision_object():
    nif = _make_cube_nif()
    profile = _get_fo4_profile()
    result = generate_collision(nif, node_block_id=0, shape_type="list", profile=profile)
    _check_fo4_result(nif, result)
def test_fo4_convex_hull_blob_starts_with_pf_magic():
    nif = _make_cube_nif()
    profile = _get_fo4_profile()
    result = generate_collision(nif, node_block_id=0, shape_type="convex_hull", profile=profile)
    assert result.success
    blob = _get_phys_sys_blob(nif)
    assert blob is not None, "bhkPhysicsSystem has no Binary Data"
    assert blob[:8] == _PF_MAGIC, \
        f"Blob does not start with Havok packfile magic: {blob[:8].hex()}"


def test_fo4_convex_hull_multiple_source_meshes_uses_compound_packfile():
    nif = _make_cube_nif()
    shape2 = nif.add_block("BSTriShape")
    shape2.set_field("Vertex Data", [
        {
            "Vertex": {"x": float(x + 20.0), "y": float(y), "z": float(z)},
            "Normal": {"x": 0, "y": 0, "z": 1},
        }
        for x in (0.0, 10.0)
        for y in (0.0, 10.0)
        for z in (0.0, 10.0)
    ])
    shape2.set_field("Triangles", [
        {"v1": 0, "v2": 1, "v3": 2},
        {"v1": 2, "v2": 1, "v3": 3},
    ])
    root = nif.get_block(0)
    root.set_field("Children", [1, shape2.block_id])
    root.set_field("Num Children", 2)

    result = generate_collision(
        nif,
        node_block_id=0,
        shape_type="convex_hull",
        profile=_get_fo4_profile(),
    )

    assert result.success, result.description
    blob = _get_phys_sys_blob(nif)
    assert blob is not None
    assert b"hknpDynamicCompoundShape" in blob


def test_fo4_compound_collision_preview_scale_matches_source_meshes():
    nif = _make_cube_nif()
    shape2 = nif.add_block("BSTriShape")
    shape2.set_field("Vertex Data", [
        {
            "Vertex": {"x": float(x + 20.0), "y": float(y), "z": float(z)},
            "Normal": {"x": 0, "y": 0, "z": 1},
        }
        for x in (0.0, 10.0)
        for y in (0.0, 10.0)
        for z in (0.0, 10.0)
    ])
    shape2.set_field("Triangles", [
        {"v1": 0, "v2": 1, "v3": 2},
        {"v1": 2, "v2": 1, "v3": 3},
    ])
    root = nif.get_block(0)
    root.set_field("Children", [1, shape2.block_id])
    root.set_field("Num Children", 2)

    result = generate_collision(
        nif,
        node_block_id=0,
        shape_type="convex_hull",
        profile=_get_fo4_profile(),
    )

    assert result.success, result.description
    blob = _get_phys_sys_blob(nif)
    assert blob is not None
    extents = _preview_extents_from_blob(blob)
    assert extents == pytest.approx([30.0, 10.0, 10.0], abs=0.05)


def test_fo4_compound_packfile_is_parseable():
    from creation_lib._native import havok_native

    nif = _make_cube_nif()
    root = nif.get_block(0)
    children = [1]
    for idx in range(2, 11):
        shape = nif.add_block("BSTriShape")
        shape.set_field("Vertex Data", [
            {
                "Vertex": {"x": float(x + idx * 20.0), "y": float(y), "z": float(z)},
                "Normal": {"x": 0, "y": 0, "z": 1},
            }
            for x in (0.0, 10.0)
            for y in (0.0, 10.0)
            for z in (0.0, 10.0)
        ])
        shape.set_field("Triangles", [
            {"v1": 0, "v2": 1, "v3": 2},
            {"v1": 2, "v2": 1, "v3": 3},
        ])
        children.append(shape.block_id)
    root.set_field("Children", children)
    root.set_field("Num Children", len(children))

    result = generate_collision(
        nif,
        node_block_id=0,
        shape_type="convex_hull",
        profile=_get_fo4_profile(),
    )

    assert result.success, result.description
    blob = _get_phys_sys_blob(nif)
    assert blob is not None
    summary = havok_native.havok_collision_summary(blob)
    assert '"shape_kind":"compound_polytope"' in summary


def test_fo4_compound_collision_overlay_extracts_lines():
    from creation_lib.renderer.nif_loader import _extract_np_collision_lines

    nif = _make_cube_nif()
    shape2 = nif.add_block("BSTriShape")
    shape2.set_field("Vertex Data", [
        {
            "Vertex": {"x": float(x + 20.0), "y": float(y), "z": float(z)},
            "Normal": {"x": 0, "y": 0, "z": 1},
        }
        for x in (0.0, 10.0)
        for y in (0.0, 10.0)
        for z in (0.0, 10.0)
    ])
    shape2.set_field("Triangles", [
        {"v1": 0, "v2": 1, "v3": 2},
        {"v1": 2, "v2": 1, "v3": 3},
    ])
    root = nif.get_block(0)
    root.set_field("Children", [1, shape2.block_id])
    root.set_field("Num Children", 2)

    result = generate_collision(
        nif,
        node_block_id=0,
        shape_type="convex_hull",
        profile=_get_fo4_profile(),
    )

    assert result.success, result.description
    coll_obj = next(block for block in nif.blocks if block.type_name == "bhkNPCollisionObject")
    lines = _extract_np_collision_lines(nif, coll_obj)
    assert lines


def test_fo4_polytope_cylinder_hull_has_reasonable_face_count():
    import json
    from creation_lib._native import havok_native

    verts = []
    for z in (0.0, 6.48):
        for i in range(32):
            angle = 2.0 * math.pi * i / 32
            verts.append([
                (3.135 / 2.0) * math.cos(angle),
                (1.716 / 2.0) * math.sin(angle),
                z,
            ])

    blob = havok_native.fo4_polytope_collision_blob(verts, 0.5, 0.4, 1, 0.0)
    summary = json.loads(havok_native.havok_collision_summary(blob))
    shape = next(o for o in summary["objects"] if o["class_name"] == "hknpConvexPolytopeShape")

    assert shape["n_faces"] < 200


def _make_cylinder_nif(segments: int = 96) -> NifFile:
    nif = _make_cube_nif()
    shape = nif.get_block(1)
    verts = []
    for z in (0.0, 10.0):
        for i in range(segments):
            angle = 2.0 * math.pi * i / segments
            verts.append({
                "Vertex": {
                    "x": 6.0 * math.cos(angle),
                    "y": 3.0 * math.sin(angle),
                    "z": z,
                },
                "Normal": {"x": 0, "y": 0, "z": 1},
            })
    shape.set_field("Vertex Data", verts)
    shape.set_field("Triangles", [])
    return nif


def _fo4_polytope_face_count(blob: bytes) -> int:
    import json
    from creation_lib._native import havok_native

    summary = json.loads(havok_native.havok_collision_summary(blob))
    return sum(
        int(obj.get("n_faces") or 0)
        for obj in summary["objects"]
        if obj["class_name"] == "hknpConvexPolytopeShape"
    )


def test_fo4_simplified_convex_fit_reduces_complex_hull_faces():
    profile = _get_fo4_profile()

    exact_nif = _make_cylinder_nif()
    exact = generate_collision(
        exact_nif,
        node_block_id=0,
        shape_type="convex_hull",
        profile=profile,
    )
    assert exact.success, exact.description
    exact_faces = _fo4_polytope_face_count(_get_phys_sys_blob(exact_nif))

    fit_nif = _make_cylinder_nif()
    fit = generate_collision(
        fit_nif,
        node_block_id=0,
        shape_type="convex_fit",
        profile=profile,
    )

    assert fit.success, fit.description
    fit_blob = _get_phys_sys_blob(fit_nif)
    fit_faces = _fo4_polytope_face_count(fit_blob)
    extents = _preview_extents_from_blob(fit_blob)
    assert fit_faces < exact_faces * 0.6
    assert fit_faces <= 64
    assert extents == pytest.approx([12.0, 6.0, 10.0], abs=0.35)
    assert _preview_nonmanifold_edge_count(fit_blob) == 0


def test_fo4_geometry_simplified_convex_fit_reduces_input_vertices():
    nif = _make_cube_nif()
    vertices = []
    for z in (0.0, 10.0):
        for i in range(96):
            angle = 2.0 * math.pi * i / 96
            vertices.append({
                "x": 6.0 * math.cos(angle),
                "y": 3.0 * math.sin(angle),
                "z": z,
            })

    result = generate_collision_from_geometry(
        nif,
        node_block_id=0,
        vertices=vertices,
        shape_type="convex_fit",
        profile=_get_fo4_profile(),
    )

    assert result.success, result.description
    blob = _get_phys_sys_blob(nif)
    assert _fo4_polytope_face_count(blob) <= 64
    assert _preview_nonmanifold_edge_count(blob) == 0
def test_fo4_geometry_collision_preview_scale_matches_input_vertices():
    nif = _make_cube_nif()
    result = generate_collision_from_geometry(
        nif,
        node_block_id=0,
        vertices=[
            {"x": 0.0, "y": 0.0, "z": 0.0},
            {"x": 10.0, "y": 0.0, "z": 0.0},
            {"x": 10.0, "y": 10.0, "z": 0.0},
            {"x": 0.0, "y": 10.0, "z": 0.0},
            {"x": 0.0, "y": 0.0, "z": 10.0},
            {"x": 10.0, "y": 0.0, "z": 10.0},
            {"x": 10.0, "y": 10.0, "z": 10.0},
            {"x": 0.0, "y": 10.0, "z": 10.0},
        ],
        shape_type="convex_hull",
        profile=_get_fo4_profile(),
    )

    assert result.success, result.description
    blob = _get_phys_sys_blob(nif)
    assert blob is not None
    extents = _preview_extents_from_blob(blob)
    assert extents == pytest.approx([10.0, 10.0, 10.0], abs=0.05)
def test_fo4_box_blob_starts_with_pf_magic():
    nif = _make_cube_nif()
    profile = _get_fo4_profile()
    result = generate_collision(nif, node_block_id=0, shape_type="box", profile=profile)
    assert result.success
    blob = _get_phys_sys_blob(nif)
    assert blob is not None
    assert blob[:8] == _PF_MAGIC
def test_fo4_list_blob_starts_with_pf_magic():
    nif = _make_cube_nif()
    profile = _get_fo4_profile()
    result = generate_collision(nif, node_block_id=0, shape_type="list", profile=profile)
    assert result.success
    blob = _get_phys_sys_blob(nif)
    assert blob is not None
    assert blob[:8] == _PF_MAGIC
def test_legacy_path_still_uses_bhk_rigid_body():
    """No profile → legacy bhkRigidBody+bhkConvexVerticesShape chain."""
    nif = _make_cube_nif()
    result = generate_collision(nif, node_block_id=0, shape_type="convex_hull", profile=None)
    assert result.success
    block_types = {b.type_name for b in nif.blocks}
    assert "bhkRigidBody" in block_types, "Legacy path should emit bhkRigidBody"
    assert "bhkConvexVerticesShape" in block_types, "Legacy path should emit bhkConvexVerticesShape"
    assert "bhkNPCollisionObject" not in block_types, "Legacy path must not emit bhkNPCollisionObject"
    assert "bhkPhysicsSystem" not in block_types, "Legacy path must not emit bhkPhysicsSystem"
def test_skyrimse_profile_uses_legacy_chain():
    """SkyrimSE uses the legacy bhkRigidBody chain despite engine="creation1"."""
    from creation_lib.core.game_profiles import get_profile as _gp
    profile = _gp("skyrimse")
    nif = _make_cube_nif()
    result = generate_collision(nif, node_block_id=0, shape_type="convex_hull", profile=profile)
    assert result.success
    block_types = {b.type_name for b in nif.blocks}
    assert "bhkRigidBody" in block_types, "SkyrimSE should use legacy bhkRigidBody chain"
    assert "bhkNPCollisionObject" not in block_types, "SkyrimSE must not emit bhkNPCollisionObject"
@pytest.mark.parametrize("shape_type", ["convex_hull", "box", "sphere", "list"])
def test_fo4_geometry_from_path_emits_np_collision(shape_type):
    """generate_collision_from_geometry (used by Max plugin / batch UI / any
    caller with explicit verts in hand) must route FO4 profile through the
    new packfile path, not the legacy bhkRigidBody chain."""
    from creation_lib.nif.operations.collision import generate_collision_from_geometry
    nif = _make_cube_nif()
    profile = _get_fo4_profile()
    # Cube vertices as the Max bridge supplies them: list of {x,y,z} dicts
    cube = [
        {"x": 0.0, "y": 0.0, "z": 0.0}, {"x": 1.0, "y": 0.0, "z": 0.0},
        {"x": 1.0, "y": 1.0, "z": 0.0}, {"x": 0.0, "y": 1.0, "z": 0.0},
        {"x": 0.0, "y": 0.0, "z": 1.0}, {"x": 1.0, "y": 0.0, "z": 1.0},
        {"x": 1.0, "y": 1.0, "z": 1.0}, {"x": 0.0, "y": 1.0, "z": 1.0},
    ]
    triangles = None
    if shape_type in {"compressed_mesh", "mesh"}:
        triangles = [{"v1": 0, "v2": 1, "v3": 2}, {"v1": 0, "v2": 2, "v3": 3}]
    result = generate_collision_from_geometry(
        nif, node_block_id=0, vertices=cube, triangles=triangles,
        shape_type=shape_type, profile=profile,
    )
    assert result.success, result.description
    block_types = {b.type_name for b in nif.blocks}
    assert "bhkNPCollisionObject" in block_types
    assert "bhkPhysicsSystem" in block_types
    assert "bhkRigidBody" not in block_types
    # The collision object's Data ref must point at the physics system here too
    coll_obj = next(b for b in nif.blocks if b.type_name == "bhkNPCollisionObject")
    phys_sys = next(b for b in nif.blocks if b.type_name == "bhkPhysicsSystem")
    assert coll_obj.get_field("Data") == phys_sys.block_id
def test_geometry_from_path_no_profile_uses_legacy_chain():
    """No profile → legacy bhk* chain (regression check for the geometry-from path)."""
    from creation_lib.nif.operations.collision import generate_collision_from_geometry
    nif = _make_cube_nif()
    cube = [
        {"x": 0.0, "y": 0.0, "z": 0.0}, {"x": 1.0, "y": 0.0, "z": 0.0},
        {"x": 1.0, "y": 1.0, "z": 0.0}, {"x": 0.0, "y": 1.0, "z": 0.0},
        {"x": 0.0, "y": 0.0, "z": 1.0}, {"x": 1.0, "y": 0.0, "z": 1.0},
        {"x": 1.0, "y": 1.0, "z": 1.0}, {"x": 0.0, "y": 1.0, "z": 1.0},
    ]
    result = generate_collision_from_geometry(
        nif, node_block_id=0, vertices=cube, shape_type="convex_hull", profile=None,
    )
    assert result.success
    block_types = {b.type_name for b in nif.blocks}
    assert "bhkRigidBody" in block_types, "Legacy path should emit bhkRigidBody"
    assert "bhkNPCollisionObject" not in block_types, "Legacy path must not emit bhkNPCollisionObject"
@pytest.mark.parametrize("shape_type", ["convex_hull", "box", "sphere", "capsule", "cylinder", "list"])
def test_fo4_collision_object_data_ref_points_to_physics_system(shape_type):
    """bhkNPCollisionObject must reference bhkPhysicsSystem via the schema-correct
    ``Data`` field (Ref<bhkSystem>). Earlier wiring set ``Body`` instead, which
    NifBlock.set_field silently appended as an unknown field — leaving the
    bhkPhysicsSystem orphaned with no Ref pointing at it. In-game collisions
    silently broke even though the block list looked correct."""
    nif = _make_cube_nif()
    profile = _get_fo4_profile()
    result = generate_collision(nif, node_block_id=0, shape_type=shape_type, profile=profile)
    assert result.success, result.description
    coll_obj = next(b for b in nif.blocks if b.type_name == "bhkNPCollisionObject")
    phys_sys = next(b for b in nif.blocks if b.type_name == "bhkPhysicsSystem")
    data_ref = coll_obj.get_field("Data")
    assert data_ref == phys_sys.block_id, (
        f"bhkNPCollisionObject.Data must point to bhkPhysicsSystem (block_id={phys_sys.block_id}); "
        f"got {data_ref!r}"
    )


def test_fo4_collision_object_uses_sync_on_update_only():
    nif = _make_cube_nif()
    result = generate_collision(
        nif,
        node_block_id=0,
        shape_type="convex_hull",
        profile=_get_fo4_profile(),
    )

    assert result.success, result.description
    coll_obj = next(b for b in nif.blocks if b.type_name == "bhkNPCollisionObject")
    assert coll_obj.get_field("Flags") == 0x80


def test_fo4_geometry_collision_sets_root_bsx_havok_bit():
    nif = _make_cube_nif()
    root = nif.get_block(0)
    bsx = nif.add_block("BSXFlags", {"Name": "BSX", "Integer Data": 0x09})
    root.set_field("Extra Data List", [bsx.block_id])
    root.set_field("Num Extra Data List", 1)

    result = generate_collision_from_geometry(
        nif,
        node_block_id=0,
        vertices=[
            {"x": 0.0, "y": 0.0, "z": 0.0},
            {"x": 10.0, "y": 0.0, "z": 0.0},
            {"x": 0.0, "y": 10.0, "z": 0.0},
            {"x": 0.0, "y": 0.0, "z": 10.0},
        ],
        shape_type="convex_hull",
        profile=_get_fo4_profile(),
    )

    assert result.success, result.description
    assert bsx.get_field("Integer Data") == 0x0B


def test_fo4_geometry_collision_assigns_unique_body_ids():
    nif = _make_cube_nif()
    child = nif.add_block("NiNode", {"Collision Object": -1})

    vertices = [
        {"x": 0.0, "y": 0.0, "z": 0.0},
        {"x": 10.0, "y": 0.0, "z": 0.0},
        {"x": 0.0, "y": 10.0, "z": 0.0},
        {"x": 0.0, "y": 0.0, "z": 10.0},
    ]
    first = generate_collision_from_geometry(
        nif,
        node_block_id=0,
        vertices=vertices,
        shape_type="convex_hull",
        profile=_get_fo4_profile(),
    )
    second = generate_collision_from_geometry(
        nif,
        node_block_id=child.block_id,
        vertices=vertices,
        shape_type="convex_hull",
        profile=_get_fo4_profile(),
    )

    assert first.success, first.description
    assert second.success, second.description
    body_ids = [
        block.get_field("Body ID")
        for block in nif.blocks
        if block.type_name == "bhkNPCollisionObject"
    ]
    assert body_ids == [0, 1]
