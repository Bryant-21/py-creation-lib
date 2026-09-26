import numpy as np
import glm
import pytest


def test_interleave_vertices_basic_and_with_colors_and_tangents():
    from creation_lib.renderer.nif_loader import interleave_vertex_data

    # Basic: position + normal + UV.
    verts = np.array([[1, 2, 3], [4, 5, 6]], dtype=np.float32)
    normals = np.array([[0, 0, 1], [0, 1, 0]], dtype=np.float32)
    uvs = np.array([[0, 0], [1, 1]], dtype=np.float32)
    data, fmt, attrs = interleave_vertex_data(verts, normals, uvs)
    assert data.shape == (2, 8)  # 3+3+2
    assert fmt == "3f 3f 2f"
    np.testing.assert_array_equal(data[0, :3], [1, 2, 3])

    # With vertex colors and tangents.
    verts = np.array([[1, 2, 3]], dtype=np.float32)
    normals = np.array([[0, 0, 1]], dtype=np.float32)
    uvs = np.array([[0, 0]], dtype=np.float32)
    colors = np.array([[1, 0, 0, 1]], dtype=np.float32)
    tangents = np.array([[1, 0, 0]], dtype=np.float32)
    bitangents = np.array([[0, 1, 0]], dtype=np.float32)
    data, fmt, attrs = interleave_vertex_data(
        verts, normals, uvs, colors=colors,
        tangents=tangents, bitangents=bitangents
    )
    assert data.shape[1] == 3 + 3 + 2 + 4 + 3 + 3  # 18
    assert "4f" in fmt  # color
    assert fmt.count("3f") == 4  # pos, normal, tangent, bitangent


def test_nif_transform_to_mat4_and_compute_normals_flat():
    from creation_lib.renderer.nif_loader import nif_transform_to_mat4, compute_normals

    # NIF translation + rotation + scale -> glm.mat4 (identity transform).
    mat = nif_transform_to_mat4(
        translation=[0, 0, 0],
        rotation=[[1, 0, 0], [0, 1, 0], [0, 0, 1]],
        scale=1.0,
    )
    assert isinstance(mat, glm.mat4)
    for i in range(4):
        for j in range(4):
            expected = 1.0 if i == j else 0.0
            assert abs(mat[i][j] - expected) < 1e-6

    # Normal computation on a single triangle should produce correct normal.
    verts = np.array([[0, 0, 0], [1, 0, 0], [0, 1, 0]], dtype=np.float32)
    tris = np.array([[0, 1, 2]], dtype=np.int32)
    normals = compute_normals(verts, tris)
    # Normal of XY plane triangle should be +Z
    assert normals.shape == (3, 3)
    for i in range(3):
        assert abs(normals[i, 2] - 1.0) < 1e-4
