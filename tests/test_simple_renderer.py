"""Tests for SimpleRenderer — PBR renderer for non-NIF viewports."""
import pytest
import numpy as np


def test_pbr_material_defaults():
    """PBRMaterial has sensible defaults."""
    from creation_lib.renderer.simple_renderer import PBRMaterial
    mat = PBRMaterial()
    assert mat.metallic == 0.0
    assert mat.roughness == 0.5
    assert mat.albedo_tex is None


def test_simple_mesh_dataclass():
    """SimpleMesh stores transform and material."""
    import glm
    from creation_lib.renderer.simple_renderer import SimpleMesh, PBRMaterial
    mat = PBRMaterial()
    # We can't test GPU objects without a context, but verify dataclass works
    mesh = SimpleMesh(vao=None, vbo=None, ibo=None, num_indices=0,
                      material=mat, transform=glm.mat4(1.0))
    assert mesh.num_indices == 0
    assert mesh.material.roughness == 0.5


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
