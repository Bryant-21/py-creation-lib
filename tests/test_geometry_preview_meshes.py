from creation_lib.geometry import preview_meshes


def test_convex_hull_triangles_uses_native_hull():
    vertices = [
        {"x": 0.0, "y": 0.0, "z": 0.0},
        {"x": 1.0, "y": 0.0, "z": 0.0},
        {"x": 0.0, "y": 1.0, "z": 0.0},
        {"x": 0.0, "y": 0.0, "z": 1.0},
    ]

    triangles = preview_meshes._convex_hull_triangles(vertices)

    assert len(triangles) == 4
    assert all(set(tri) == {"v1", "v2", "v3"} for tri in triangles)
