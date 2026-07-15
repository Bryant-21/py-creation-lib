from types import SimpleNamespace

from creation_lib.nif.nif_file import NifFile
from creation_lib.nif.operations import collision
from creation_lib.nif.operations.collision import generate_collision


def _add_shape(nif: NifFile, name: str, x: float = 0.0):
    return nif.add_block(
        "BSTriShape",
        {
            "Name": name,
            "Vertex Data": [
                {"Vertex": {"x": x, "y": 0.0, "z": 0.0}},
                {"Vertex": {"x": x + 1.0, "y": 0.0, "z": 0.0}},
                {"Vertex": {"x": x, "y": 1.0, "z": 0.0}},
                {"Vertex": {"x": x, "y": 0.0, "z": 1.0}},
            ],
        },
    )


def _add_collision_source(nif: NifFile, name: str, x: float = 0.0):
    node = nif.add_block(
        "NiNode",
        {
            "Name": name,
            "Children": [],
            "Num Children": 0,
            "Collision Object": -1,
        },
    )
    shape = _add_shape(nif, f"{name}:0", x)
    node.set_field("Children", [shape.block_id])
    node.set_field("Num Children", 1)
    return node


def test_fo4_generating_multiple_nodes_uses_one_shared_physics_system():
    nif = NifFile()
    node_a = _add_collision_source(nif, "Body")
    node_b = _add_collision_source(nif, "Door")
    profile = SimpleNamespace(
        id="fo4",
        collision_layer_enum="Fallout4Layer",
        havok_scale=69.99125,
    )

    result_a = generate_collision(
        nif,
        node_a.block_id,
        shape_type="convex_fit",
        profile=profile,
    )
    result_b = generate_collision(
        nif,
        node_b.block_id,
        shape_type="convex_fit",
        profile=profile,
    )

    assert result_a.success
    assert result_b.success
    collisions = [
        block for block in nif.blocks if block.type_name == "bhkNPCollisionObject"
    ]
    assert len(collisions) == 2
    assert [block.get_field("Body ID") for block in collisions] == [0, 1]
    assert collisions[0].get_field("Data") == collisions[1].get_field("Data")
    assert len([block for block in nif.blocks if block.type_name == "bhkPhysicsSystem"]) == 1


def test_fo4_replace_existing_target_keeps_other_bodies_shared():
    nif = NifFile()
    node_a = _add_collision_source(nif, "Body")
    node_b = _add_collision_source(nif, "Door")
    profile = SimpleNamespace(
        id="fo4",
        collision_layer_enum="Fallout4Layer",
        havok_scale=69.99125,
    )

    assert generate_collision(nif, node_a.block_id, shape_type="convex_fit", profile=profile).success
    assert generate_collision(nif, node_b.block_id, shape_type="convex_fit", profile=profile).success
    assert generate_collision(
        nif,
        node_b.block_id,
        shape_type="convex_fit",
        profile=profile,
        replace=True,
    ).success

    collisions = [
        block for block in nif.blocks if block.type_name == "bhkNPCollisionObject"
    ]
    data_ids = {block.get_field("Data") for block in collisions}
    assert len(collisions) == 2
    assert [block.get_field("Body ID") for block in collisions] == [0, 1]
    assert len(data_ids) == 1
    assert len([block for block in nif.blocks if block.type_name == "bhkPhysicsSystem"]) == 1


def test_fo4_existing_target_requires_replace_to_rebuild():
    nif = NifFile()
    node = _add_collision_source(nif, "Body")
    profile = SimpleNamespace(
        id="fo4",
        collision_layer_enum="Fallout4Layer",
        havok_scale=69.99125,
    )

    assert generate_collision(nif, node.block_id, shape_type="convex_fit", profile=profile).success
    result = generate_collision(
        nif,
        node.block_id,
        shape_type="convex_fit",
        profile=profile,
        replace=False,
    )

    assert result.success is False
    assert "already exists" in result.description


def test_fo4_preserves_existing_collision_from_sources_when_preview_fails(monkeypatch):
    nif = NifFile()
    node_a = _add_collision_source(nif, "Body")
    node_b = _add_collision_source(nif, "Door")
    profile = SimpleNamespace(
        id="fo4",
        collision_layer_enum="Fallout4Layer",
        havok_scale=69.99125,
    )

    assert generate_collision(nif, node_a.block_id, shape_type="convex_fit", profile=profile).success

    def fail_preview(_record):
        raise ValueError("no preview geometry for body 0")

    monkeypatch.setattr(collision, "_fo4_body_spec_from_preview", fail_preview)
    result = generate_collision(
        nif,
        node_b.block_id,
        shape_type="convex_fit",
        profile=profile,
    )

    assert result.success, result.description
    assert any("Rebuilt existing collision on block" in warning for warning in result.warnings)
    collisions = [
        block for block in nif.blocks if block.type_name == "bhkNPCollisionObject"
    ]
    assert len(collisions) == 2
    assert collisions[0].get_field("Data") == collisions[1].get_field("Data")


def test_fo4_preview_filters_dynamic_compound_body():
    nif = NifFile()
    node_a = _add_collision_source(nif, "Body", 0.0)
    node_b = nif.add_block(
        "NiNode",
        {
            "Name": "Door",
            "Children": [],
            "Num Children": 0,
            "Collision Object": -1,
        },
    )
    door_shape = _add_shape(nif, "Door:0", 100.0)
    wheel = _add_collision_source(nif, "Wheel", 200.0)
    node_b.set_field("Children", [door_shape.block_id, wheel.block_id])
    node_b.set_field("Num Children", 2)
    profile = SimpleNamespace(
        id="fo4",
        collision_layer_enum="Fallout4Layer",
        havok_scale=69.99125,
    )

    assert generate_collision(nif, node_a.block_id, shape_type="convex_fit", profile=profile).success
    assert generate_collision(nif, node_b.block_id, shape_type="convex_fit", profile=profile).success

    phys = next(block for block in nif.blocks if block.type_name == "bhkPhysicsSystem")
    blob = bytes((phys.get_field("Binary Data") or {}).get("Data") or [])
    from creation_lib.havok.native_runtime import collision_preview_native

    body0 = collision_preview_native(blob, havok_scale=1.0, body_id=0).get("meshes") or []
    body1 = collision_preview_native(blob, havok_scale=1.0, body_id=1).get("meshes") or []
    body2 = collision_preview_native(blob, havok_scale=1.0, body_id=2).get("meshes") or []

    assert len(body0) == 1
    assert len(body1) == 2
    assert body2 == []
    body1_xs = [
        vertex["x"]
        for mesh_info in body1
        for vertex in (mesh_info.get("mesh") or {}).get("vertices") or []
    ]
    assert min(body1_xs) > 1.0


def test_fo4_animstatic_body_emits_motion_cinfo_and_valid_motion_id():
    """Vanilla Safe01 parity for the FO4 workshop-sweep crash on bank.nif.

    The safe/container pattern is a STATIC base + an ANIMSTATIC (keyframed)
    door. Vanilla `Safe01.nif` emits the door's motionCinfo only — the static
    base carries `motionId = 0x7FFFFFFF (HK_INVALID)` and NO motionCinfo. A
    static base that instead advertises a motion frame reproduces the
    hknpHybridBroadPhase null-deref during workshop placement
    (Fallout4.exe+13E82D0).

    So: exactly one motionCinfo (the door); the static base is HK_INVALID; the
    keyframed door points at the sole motionCinfos[0]; materialId is still
    reindexed so each body owns its own material slot.
    """
    import json

    nif = NifFile()
    node_a = _add_collision_source(nif, "Body")
    node_b = _add_collision_source(nif, "Door")
    profile = SimpleNamespace(
        id="fo4",
        collision_layer_enum="Fallout4Layer",
        havok_scale=69.99125,
    )

    # Base body: STATIC.
    assert generate_collision(
        nif, node_a.block_id, shape_type="convex_fit", profile=profile
    ).success
    # Door: ANIMSTATIC — this is the path that crashes without the fix.
    assert generate_collision(
        nif,
        node_b.block_id,
        shape_type="convex_hull",
        layer="ANIMSTATIC",
        profile=profile,
    ).success

    phys = next(block for block in nif.blocks if block.type_name == "bhkPhysicsSystem")
    blob = bytes((phys.get_field("Binary Data") or {}).get("Data") or [])

    from creation_lib._native import havok_native

    summary = json.loads(havok_native.havok_collision_summary(blob))
    bodies = summary.get("bodies") or []
    assert len(bodies) == 2, f"expected 2 bodies, got {len(bodies)}"

    # Inspect the parsed packfile directly to assert structural invariants.
    HK_INVALID = 0x7FFFFFFF
    from creation_lib.hkxpack.native_runtime import hkx_load_to_json
    import re

    xml = hkx_load_to_json(blob)["content"]
    mc_count = int(re.search(r'motionCinfos" numelements="(\d+)"', xml).group(1))
    assert mc_count == 1, (
        "vanilla Safe01 parity: only the ANIMSTATIC door emits a motionCinfo; "
        f"the STATIC base must not (got {mc_count})"
    )

    body_objs = re.findall(
        r'<hkobject>\s*<hkparam name="shape">[\s\S]*?</hkobject>', xml
    )
    assert len(body_objs) == 2
    body_motions = [int(re.search(r'motionId">(-?\d+)', b).group(1)) for b in body_objs]
    body_mats = [int(re.search(r'materialId">(-?\d+)', b).group(1)) for b in body_objs]
    body_filters = [int(re.search(r'collisionFilterInfo">(-?\d+)', b).group(1)) for b in body_objs]

    # Body 0 is STATIC: vanilla Safe01 emits HK_INVALID and no motionCinfo for it.
    assert body_motions[0] == HK_INVALID, (
        "static base must be HK_INVALID (Safe01 parity); a motion frame here "
        "re-arms the workshop-sweep CTD"
    )
    assert body_filters[0] == 1
    # Body 1 is ANIMSTATIC: motionId points at the sole motionCinfos[0], filter=2.
    assert body_motions[1] == 0, (
        f"ANIMSTATIC body must point at the single motionCinfos[0], got {body_motions[1]}"
    )
    assert body_filters[1] == 2
    # Each body must own its material slot.
    assert body_mats == [0, 1], (
        f"materialId reindexing failed: got {body_mats}, expected [0, 1]"
    )
