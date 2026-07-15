from types import SimpleNamespace

import numpy as np

from creation_lib.nif.nif_file import NifFile
from creation_lib.nif.operations import collision
from creation_lib.nif.operations.collision import generate_collision


def _large_independent_triangle_mesh(count: int = 300):
    vertices = []
    triangles = []
    for i in range(count):
        base = i * 3
        x = float(i * 2)
        vertices.extend(
            [
                {"Vertex": {"x": x, "y": 0.0, "z": 0.0}},
                {"Vertex": {"x": x + 1.0, "y": 0.0, "z": 0.0}},
                {"Vertex": {"x": x, "y": 1.0, "z": 0.0}},
            ]
        )
        triangles.append({"v1": base, "v2": base + 1, "v3": base + 2})
    return vertices, triangles


def _large_panel_with_small_detail_mesh():
    vertices: list[list[float]] = []
    triangles: list[list[int]] = []

    def add_grid_face(origin, u_vec, v_vec, u_steps=16, v_steps=16):
        start = len(vertices)
        origin = np.asarray(origin, dtype=np.float32)
        u_vec = np.asarray(u_vec, dtype=np.float32)
        v_vec = np.asarray(v_vec, dtype=np.float32)
        for u in range(u_steps + 1):
            for v in range(v_steps + 1):
                point = origin + (u_vec * (u / u_steps)) + (v_vec * (v / v_steps))
                vertices.append(point.tolist())
        stride = v_steps + 1
        for u in range(u_steps):
            for v in range(v_steps):
                a = start + u * stride + v
                b = a + stride
                c = b + 1
                d = a + 1
                triangles.append([a, b, c])
                triangles.append([a, c, d])

    add_grid_face([-12.0, -72.0, -74.0], [17.0, 0.0, 0.0], [0.0, 73.0, 0.0])
    add_grid_face([-12.0, -72.0, 10.0], [17.0, 0.0, 0.0], [0.0, 73.0, 0.0])

    center = np.array([2.0, -35.0, -25.0], dtype=np.float32)
    length = np.array([3.0, 0.0, 0.0], dtype=np.float32)
    radius = 0.8
    ring_start = len(vertices)
    segments = 48
    for segment in range(segments):
        angle = (segment / segments) * np.pi * 2.0
        offset = np.array([0.0, np.cos(angle) * radius, np.sin(angle) * radius], dtype=np.float32)
        vertices.append((center + offset).tolist())
        vertices.append((center + length + offset).tolist())
    for segment in range(segments):
        a = ring_start + segment * 2
        b = ring_start + ((segment + 1) % segments) * 2
        c = b + 1
        d = a + 1
        triangles.append([a, b, c])
        triangles.append([a, c, d])

    return np.asarray(vertices, dtype=np.float32), np.asarray(triangles, dtype=np.int32)


def _many_closed_cubes(count: int = 12):
    vertices: list[list[float]] = []
    triangles: list[list[int]] = []
    cube_tris = [
        [0, 1, 2],
        [0, 2, 3],
        [4, 6, 5],
        [4, 7, 6],
        [0, 4, 5],
        [0, 5, 1],
        [1, 5, 6],
        [1, 6, 2],
        [2, 6, 7],
        [2, 7, 3],
        [3, 7, 4],
        [3, 4, 0],
    ]
    for index in range(count):
        x = float(index * 2)
        base = len(vertices)
        vertices.extend(
            [
                [x, 0.0, 0.0],
                [x + 1.0, 0.0, 0.0],
                [x + 1.0, 1.0, 0.0],
                [x, 1.0, 0.0],
                [x, 0.0, 1.0],
                [x + 1.0, 0.0, 1.0],
                [x + 1.0, 1.0, 1.0],
                [x, 1.0, 1.0],
            ]
        )
        triangles.extend([[base + a, base + b, base + c] for a, b, c in cube_tris])
    return np.asarray(vertices, dtype=np.float32), np.asarray(triangles, dtype=np.int32)


def test_bbox_collision_mesh_winds_outward():
    """The bbox fallback must wind every face outward (normal points away from
    the box center).

    hknpCompressedMeshShape collision is one-sided per triangle. Inward winding
    means the top face only collides from inside, so the player falls straight
    through the top — the B21_TheBank safe (639-vert base) decimated past FO4's
    compressed-mesh limits, fell back to this bbox, and was un-standable until
    the winding was corrected to match vanilla Safe01.
    """
    verts = np.asarray(
        [[-2.0, -3.0, -1.0], [5.0, 4.0, 7.0], [1.0, 0.0, 2.0]],
        dtype=np.float32,
    )
    box_verts, box_tris = collision._bbox_collision_mesh(verts)
    center = box_verts.mean(axis=0)
    for tri in box_tris:
        a, b, c = (box_verts[int(i)] for i in tri)
        normal = np.cross(b - a, c - a)
        outward = ((a + b + c) / 3.0) - center
        assert float(np.dot(normal, outward)) > 0.0, (
            f"bbox triangle {tri.tolist()} winds inward (normal points toward center)"
        )


def test_auto_compressed_mesh_simplifies_to_compressed_mesh_limits():
    verts_arr, tris_arr = _large_panel_with_small_detail_mesh()
    out_verts, out_tris, simplified = collision._simplify_mesh_for_compressed_collision(
        verts_arr,
        tris_arr,
    )

    assert simplified is True
    assert len(out_verts) <= collision.COMPRESSED_MESH_MAX_VERTICES
    assert len(out_tris) <= collision.COMPRESSED_MESH_MAX_TRIANGLES
    np.testing.assert_allclose(out_verts.min(axis=0), verts_arr.min(axis=0), atol=1e-5)
    np.testing.assert_allclose(out_verts.max(axis=0), verts_arr.max(axis=0), atol=1e-5)


def test_auto_compressed_mesh_falls_back_when_decimator_opens_closed_mesh(monkeypatch):
    verts_arr, tris_arr = _many_closed_cubes()
    assert collision._triangle_mesh_has_closed_edges(tris_arr, len(verts_arr))

    def fake_qem(vertices, triangles, target_triangles):
        return vertices[:3], np.asarray([[0, 1, 2]], dtype=np.int32)

    monkeypatch.setattr(collision, "_qem_decimate_mesh", fake_qem)

    out_verts, out_tris, simplified = collision._simplify_mesh_for_compressed_collision(
        verts_arr,
        tris_arr,
    )

    assert simplified is True
    assert len(out_verts) == 8
    assert len(out_tris) == 12
    assert collision._triangle_mesh_has_closed_edges(out_tris, len(out_verts))
    np.testing.assert_allclose(out_verts.min(axis=0), verts_arr.min(axis=0), atol=1e-5)
    np.testing.assert_allclose(out_verts.max(axis=0), verts_arr.max(axis=0), atol=1e-5)


def test_fo4_auto_compressed_mesh_simplifies_before_building_blob(monkeypatch):
    vertices, triangles = _large_independent_triangle_mesh()
    nif = NifFile()
    node = nif.add_block(
        "NiNode",
        {"Name": "CollisionTarget", "Children": [], "Num Children": 0, "Collision Object": -1},
    )
    shape = nif.add_block(
        "BSTriShape",
        {"Name": "CollisionTarget:0", "Vertex Data": vertices, "Triangles": triangles},
    )
    node.set_field("Children", [shape.block_id])
    node.set_field("Num Children", 1)

    def fake_qem(vertices, triangles, target_triangles):
        return vertices, triangles[:target_triangles]

    captured = {}

    def fake_multi_body_blob(bodies, *_args, **_kwargs):
        captured["bodies"] = bodies
        return b"blob"

    monkeypatch.setattr(collision, "_qem_decimate_mesh", fake_qem)
    monkeypatch.setattr(
        "creation_lib._native.havok_native.fo4_multi_body_collision_blob",
        fake_multi_body_blob,
    )

    profile = SimpleNamespace(
        id="fo4",
        collision_layer_enum="Fallout4Layer",
        havok_scale=69.99125,
    )

    result = generate_collision(
        nif,
        node.block_id,
        shape_type="auto_compressed_mesh",
        profile=profile,
    )

    assert result.success
    kind, out_vertices, out_triangles, compound_children = captured["bodies"][0]
    assert kind == "compressed_mesh"
    assert compound_children is None
    assert len(out_vertices) <= collision.COMPRESSED_MESH_MAX_VERTICES
    assert len(out_triangles) <= collision.COMPRESSED_MESH_MAX_TRIANGLES
    assert result.warnings
    assert "simplified compressed mesh" in result.warnings[0]


def test_fo4_auto_compressed_mesh_preview_is_parseable():
    nif = NifFile()
    node = nif.add_block(
        "NiNode",
        {"Name": "CollisionTarget", "Children": [], "Num Children": 0, "Collision Object": -1},
    )
    shape = nif.add_block(
        "BSTriShape",
        {
            "Name": "CollisionTarget:0",
            "Vertex Data": [
                {"Vertex": {"x": 0.0, "y": 0.0, "z": 0.0}},
                {"Vertex": {"x": 1.0, "y": 0.0, "z": 0.0}},
                {"Vertex": {"x": 1.0, "y": 1.0, "z": 0.0}},
                {"Vertex": {"x": 0.0, "y": 1.0, "z": 0.0}},
                {"Vertex": {"x": 0.0, "y": 0.0, "z": 1.0}},
                {"Vertex": {"x": 1.0, "y": 0.0, "z": 1.0}},
                {"Vertex": {"x": 1.0, "y": 1.0, "z": 1.0}},
                {"Vertex": {"x": 0.0, "y": 1.0, "z": 1.0}},
            ],
            "Triangles": [
                {"v1": 0, "v2": 1, "v3": 2},
                {"v1": 0, "v2": 2, "v3": 3},
                {"v1": 4, "v2": 6, "v3": 5},
                {"v1": 4, "v2": 7, "v3": 6},
            ],
        },
    )
    node.set_field("Children", [shape.block_id])
    node.set_field("Num Children", 1)
    profile = SimpleNamespace(
        id="fo4",
        collision_layer_enum="Fallout4Layer",
        havok_scale=69.99125,
    )

    result = generate_collision(
        nif,
        node.block_id,
        shape_type="auto_compressed_mesh",
        profile=profile,
    )

    assert result.success, result.description
    phys = next(block for block in nif.blocks if block.type_name == "bhkPhysicsSystem")
    blob = bytes((phys.get_field("Binary Data") or {}).get("Data") or [])
    from creation_lib.havok.native_runtime import collision_preview_native

    preview = collision_preview_native(blob, havok_scale=1.0, body_id=0)
    meshes = preview.get("meshes") or []
    assert meshes
    assert meshes[0].get("shape_type") == "compressed_mesh"


def _add_compressed_source(nif: NifFile, name: str, x: float):
    node = nif.add_block(
        "NiNode",
        {"Name": name, "Children": [], "Num Children": 0, "Collision Object": -1},
    )
    shape = nif.add_block(
        "BSTriShape",
        {
            "Name": f"{name}:0",
            "Vertex Data": [
                {"Vertex": {"x": x, "y": 0.0, "z": 0.0}},
                {"Vertex": {"x": x + 1.0, "y": 0.0, "z": 0.0}},
                {"Vertex": {"x": x + 1.0, "y": 1.0, "z": 0.0}},
                {"Vertex": {"x": x, "y": 1.0, "z": 0.0}},
                {"Vertex": {"x": x, "y": 0.0, "z": 1.0}},
                {"Vertex": {"x": x + 1.0, "y": 0.0, "z": 1.0}},
                {"Vertex": {"x": x + 1.0, "y": 1.0, "z": 1.0}},
                {"Vertex": {"x": x, "y": 1.0, "z": 1.0}},
            ],
            "Triangles": [
                {"v1": 0, "v2": 1, "v3": 2},
                {"v1": 0, "v2": 2, "v3": 3},
                {"v1": 4, "v2": 6, "v3": 5},
                {"v1": 4, "v2": 7, "v3": 6},
                {"v1": 0, "v2": 4, "v3": 5},
                {"v1": 0, "v2": 5, "v3": 1},
                {"v1": 1, "v2": 5, "v3": 6},
                {"v1": 1, "v2": 6, "v3": 2},
                {"v1": 2, "v2": 6, "v3": 7},
                {"v1": 2, "v2": 7, "v3": 3},
                {"v1": 3, "v2": 7, "v3": 4},
                {"v1": 3, "v2": 4, "v3": 0},
            ],
        },
    )
    node.set_field("Children", [shape.block_id])
    node.set_field("Num Children", 1)
    return node


def test_fo4_multi_body_auto_compressed_mesh_preview_filters_body():
    import json

    nif = NifFile()
    node_a = _add_compressed_source(nif, "Body", 0.0)
    node_b = _add_compressed_source(nif, "Door", 100.0)
    profile = SimpleNamespace(
        id="fo4",
        collision_layer_enum="Fallout4Layer",
        havok_scale=69.99125,
    )

    assert generate_collision(
        nif,
        node_a.block_id,
        shape_type="auto_compressed_mesh",
        profile=profile,
    ).success
    assert generate_collision(
        nif,
        node_b.block_id,
        shape_type="auto_compressed_mesh",
        profile=profile,
    ).success

    phys = next(block for block in nif.blocks if block.type_name == "bhkPhysicsSystem")
    blob = bytes((phys.get_field("Binary Data") or {}).get("Data") or [])
    from creation_lib._native import havok_native
    from creation_lib.havok.native_runtime import collision_preview_native

    summary = json.loads(havok_native.havok_collision_summary(blob))
    bodies = summary.get("bodies") or []
    assert [body.get("shape_class") for body in bodies] == [
        "hknpCompressedMeshShape",
        "hknpCompressedMeshShape",
    ]

    body0 = collision_preview_native(blob, havok_scale=1.0, body_id=0).get("meshes") or []
    body1 = collision_preview_native(blob, havok_scale=1.0, body_id=1).get("meshes") or []
    body2 = collision_preview_native(blob, havok_scale=1.0, body_id=2).get("meshes") or []

    assert len(body0) == 1
    assert len(body1) == 1
    assert body2 == []
    body1_xs = [
        vertex["x"]
        for mesh_info in body1
        for vertex in (mesh_info.get("mesh") or {}).get("vertices") or []
    ]
    assert min(body1_xs) > 1.0
