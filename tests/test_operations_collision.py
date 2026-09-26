import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

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


def test_convex_hull_creates_scaled_hull_block():
    """create_convex_hull adds a bhkConvexVerticesShape with all cube vertices,
    scaled by 1/HAVOK_SCALE_FO4 (~1/70), NOT 1/7 (regression)."""
    nif = _make_cube_nif()
    result = create_convex_hull(nif, 1)
    assert result.success
    assert len(result.modified_block_ids) == 1

    new_block = nif.blocks[-1]
    assert new_block.type_name == "bhkConvexVerticesShape"
    hull_verts = new_block.get_field("Vertices")
    assert hull_verts is not None
    assert len(hull_verts) == 8  # all 8 cube vertices are on the hull

    max_coord = max(abs(v["x"]) for v in hull_verts)
    assert 0.1 < max_coord < 0.2, f"Havok scale bug: max_coord={max_coord}"


def test_convex_hull_and_convert_shape_failure_cases():
    nif = _make_cube_nif()
    assert not create_convex_hull(nif, 99).success  # invalid block

    empty_nif = NifFile()
    empty_nif.blocks.append(NifBlock(block_id=0, type_name="BSTriShape"))
    assert not create_convex_hull(empty_nif, 0).success  # no vertex data

    assert not convert_collision_shape(NifFile(), 0, "bhkBoxShape").success  # not implemented


def test_generate_collision_convex_hull():
    """Full hierarchy: node -> CollisionObject -> RigidBody -> ConvexVerticesShape,
    with vertices scaled by ~1/70 (regression: old bug used 1/7), and the layer
    propagated into the rigid body's HavokFilter."""
    nif = _make_cube_nif()
    result = generate_collision(nif, node_block_id=0, shape_type="convex_hull", layer="WEAPON")
    assert result.success

    root = nif.get_block(0)
    coll_ref = root.get_field("Collision Object")
    assert coll_ref is not None and coll_ref >= 0

    coll_block = nif.get_block(coll_ref)
    assert coll_block.type_name == "bhkCollisionObject"
    assert coll_block.get_field("Target") == 0  # points back to root
    assert coll_block.get_field("Flags") == 0x81

    body_ref = coll_block.get_field("Body")
    assert body_ref >= 0
    body_block = nif.get_block(body_ref)
    assert body_block.type_name == "bhkRigidBody"
    havok_filter = body_block.get_field("Havok Filter")
    assert havok_filter.get("Layer:FO4", havok_filter.get("Layer")) == FO4_LAYERS["WEAPON"]

    shape_ref = body_block.get_field("Shape")
    assert shape_ref >= 0
    shape_block = nif.get_block(shape_ref)
    assert shape_block.type_name == "bhkConvexVerticesShape"

    hull_verts = shape_block.get_field("Vertices")
    max_coord = max(
        max(abs(v["x"]), abs(v["y"]), abs(v["z"])) for v in hull_verts
    )
    expected = 10.0 / HAVOK_SCALE_FO4
    assert abs(max_coord - expected) < 0.01, \
        f"Havok scale wrong: got {max_coord}, expected {expected}"


def test_generate_collision_box():
    """Box shape with bhkTransformShape wrapper."""
    nif = _make_cube_nif()
    result = generate_collision(nif, node_block_id=0, shape_type="box")
    assert result.success

    root = nif.get_block(0)
    coll_ref = root.get_field("Collision Object")
    coll_block = nif.get_block(coll_ref)
    body_ref = coll_block.get_field("Body")
    body_block = nif.get_block(body_ref)
    shape_ref = body_block.get_field("Shape")
    shape_block = nif.get_block(shape_ref)
    assert shape_block.type_name == "bhkTransformShape"

    box_ref = shape_block.get_field("Shape")
    box_block = nif.get_block(box_ref)
    assert box_block.type_name == "bhkBoxShape"
    assert box_block.get_field("Dimensions") is not None


def test_generate_collision_list():
    """Compound collision from multiple meshes -- should create bhkListShape."""
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

    root = nif.get_block(0)
    root.set_field("Children", [1, shape2.block_id])
    root.set_field("Num Children", 2)

    result = generate_collision(nif, node_block_id=0, shape_type="list")
    assert result.success

    root = nif.get_block(0)
    coll_ref = root.get_field("Collision Object")
    coll_block = nif.get_block(coll_ref)
    body_ref = coll_block.get_field("Body")
    body_block = nif.get_block(body_ref)
    shape_ref = body_block.get_field("Shape")
    shape_block = nif.get_block(shape_ref)
    assert shape_block.type_name == "bhkListShape"


def test_generate_collision_lifecycle_replace_and_remove():
    """replace=True swaps out old collision; remove_collision then cleans it up
    fully and reports failure if called again with nothing present."""
    nif = _make_cube_nif()

    result1 = generate_collision(nif, node_block_id=0, shape_type="convex_hull")
    assert result1.success

    result2 = generate_collision(nif, node_block_id=0, shape_type="box", replace=True)
    assert result2.success

    root = nif.get_block(0)
    coll_ref = root.get_field("Collision Object")
    assert coll_ref >= 0
    assert nif.get_block(coll_ref).type_name == "bhkCollisionObject"

    result = remove_collision(nif, node_block_id=0)
    assert result.success

    root = nif.get_block(0)
    coll_ref = root.get_field("Collision Object")
    assert coll_ref is None or coll_ref < 0

    coll_types = {"bhkCollisionObject", "bhkRigidBody", "bhkConvexVerticesShape", "bhkTransformShape", "bhkBoxShape"}
    for block in nif.blocks:
        assert block.type_name not in coll_types, \
            f"Collision block {block.type_name} still present after removal"

    assert not remove_collision(nif, node_block_id=0).success


def test_generate_collision_from_geometry_fo4_emits_np_collision_object_and_bsx_bit():
    nif = _make_cube_nif()
    profile = get_profile("fo4")
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
            {"x": 10.0, "y": 10.0, "z": 0.0},
            {"x": 0.0, "y": 10.0, "z": 0.0},
            {"x": 0.0, "y": 0.0, "z": 10.0},
            {"x": 10.0, "y": 0.0, "z": 10.0},
            {"x": 10.0, "y": 10.0, "z": 10.0},
            {"x": 0.0, "y": 10.0, "z": 10.0},
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
    assert bsx.get_field("Integer Data") == 0x0B  # havok bit set


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


@pytest.mark.parametrize("include_child_nodes", [True, False])
def test_generate_collision_child_node_transform_and_exclusion(include_child_nodes):
    """Recursion applies the child node's transform when included, and the
    child mesh is excluded entirely when include_child_nodes=False."""
    nif = _make_cube_nif()
    root = nif.get_block(0)
    root.set_field("Children", [1, 2] if not include_child_nodes else [2])
    root.set_field("Num Children", 2 if not include_child_nodes else 1)

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
        nif, node_block_id=0, shape_type="convex_hull",
        include_child_nodes=include_child_nodes,
    )

    assert result.success, result.description
    convex = next(b for b in nif.blocks if b.type_name == "bhkConvexVerticesShape")
    hull_verts = convex.get_field("Vertices")
    max_x = max(v["x"] for v in hull_verts)
    if include_child_nodes:
        # root's own cube (0-10) plus the translated child cube (20-30) -> max 30
        assert max_x == pytest.approx(30.0 / HAVOK_SCALE_FO4, abs=0.01)
    else:
        # only root's own cube (0-10) contributes
        assert max_x == pytest.approx(10.0 / HAVOK_SCALE_FO4, abs=0.01)


# ---------------------------------------------------------------------------
# FO4 routing integration tests
# ---------------------------------------------------------------------------


def _get_fo4_profile():
    return get_profile("fo4")


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


@pytest.mark.parametrize("shape_type", ["convex_hull", "box", "sphere", "list"])
def test_fo4_emits_np_collision_object(shape_type):
    """FO4 profile must route through bhkNPCollisionObject/bhkPhysicsSystem
    (packfile blob starting with the Havok PF magic), never the legacy
    bhkRigidBody/bhkConvexVerticesShape chain."""
    nif = _make_cube_nif()
    result = generate_collision(nif, node_block_id=0, shape_type=shape_type, profile=_get_fo4_profile())
    assert result.success, result.description
    block_types = {b.type_name for b in nif.blocks}
    assert "bhkNPCollisionObject" in block_types, f"Missing bhkNPCollisionObject; have: {block_types}"
    assert "bhkPhysicsSystem" in block_types, f"Missing bhkPhysicsSystem; have: {block_types}"
    assert "bhkRigidBody" not in block_types
    assert "bhkConvexVerticesShape" not in block_types

    blob = _get_phys_sys_blob(nif)
    assert blob is not None
    assert blob[:8] == b'\x57\xE0\xE0\x57\x10\xC0\xC0\x10'


def test_fo4_convex_hull_multiple_source_meshes_uses_parseable_compound_packfile():
    from creation_lib._native import havok_native

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

    extents = _preview_extents_from_blob(blob)
    assert extents == pytest.approx([30.0, 10.0, 10.0], abs=0.05)

    summary = havok_native.havok_collision_summary(blob)
    assert '"shape_kind":"compound_polytope"' in summary


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


@pytest.mark.parametrize("profile_name", [None, "skyrimse"])
def test_legacy_path_still_uses_bhk_rigid_body(profile_name):
    """No profile (or SkyrimSE, despite engine='creation1') -> legacy
    bhkRigidBody+bhkConvexVerticesShape chain, never bhkNPCollisionObject."""
    nif = _make_cube_nif()
    profile = get_profile(profile_name) if profile_name else None
    result = generate_collision(nif, node_block_id=0, shape_type="convex_hull", profile=profile)
    assert result.success
    block_types = {b.type_name for b in nif.blocks}
    assert "bhkRigidBody" in block_types, "Legacy path should emit bhkRigidBody"
    assert "bhkNPCollisionObject" not in block_types
    assert "bhkPhysicsSystem" not in block_types


@pytest.mark.parametrize("shape_type", ["convex_hull", "box", "sphere", "capsule", "cylinder", "list"])
def test_fo4_collision_object_data_ref_points_to_physics_system(shape_type):
    """bhkNPCollisionObject must reference bhkPhysicsSystem via the schema-correct
    ``Data`` field (Ref<bhkSystem>). Earlier wiring set ``Body`` instead, which
    NifBlock.set_field silently appended as an unknown field -- leaving the
    bhkPhysicsSystem orphaned with no Ref pointing at it. In-game collisions
    silently broke even though the block list looked correct. Flags must also
    use the kPausesGame-safe sync-on-update-only value (0x80)."""
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
    assert coll_obj.get_field("Flags") == 0x80


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
