"""Tests for SimpleRenderer — PBR renderer for non-NIF viewports."""
import numpy as np


def test_compute_tangents_basic():
    """Tangent computation from positions, normals, UVs."""
    from creation_lib.renderer.simple_renderer import compute_tangents
    # Simple quad: 2 triangles
    positions = np.array([
        [0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0],
    ], dtype=np.float32)
    normals = np.array([
        [0, 0, 1], [0, 0, 1], [0, 0, 1], [0, 0, 1],
    ], dtype=np.float32)
    uvs = np.array([
        [0, 0], [1, 0], [1, 1], [0, 1],
    ], dtype=np.float32)
    faces = np.array([[0, 1, 2], [0, 2, 3]], dtype=np.int32)

    tangents = compute_tangents(positions, normals, uvs, faces)
    assert tangents.shape == (4, 3)
    # Tangent should point along +X for this UV layout
    assert tangents[0][0] > 0.9
