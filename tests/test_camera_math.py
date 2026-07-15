import math
import glm
import pytest


def test_default_camera_view_matrix_is_valid():
    """Default camera should produce a valid view matrix (not identity, invertible)."""
    from creation_lib.renderer.camera import OrbitCamera
    cam = OrbitCamera()
    view = cam.get_view_matrix()
    assert isinstance(view, glm.mat4)
    assert view != glm.mat4(1.0)  # not identity
    assert abs(glm.determinant(view)) > 1e-6  # invertible


def test_camera_projection_matrix():
    """Projection matrix should be valid perspective."""
    from creation_lib.renderer.camera import OrbitCamera
    cam = OrbitCamera()
    proj = cam.get_projection_matrix(16.0 / 9.0)
    assert isinstance(proj, glm.mat4)
    assert abs(glm.determinant(proj)) > 1e-6


def test_camera_eye_position_matches_spherical():
    """Eye position should match spherical coordinate calculation."""
    from creation_lib.renderer.camera import OrbitCamera
    cam = OrbitCamera()
    cam.azimuth = 0.0
    cam.elevation = 0.0
    cam.distance = 10.0
    cam.target = glm.vec3(0, 0, 0)
    eye = cam.get_eye_position()
    # At az=0, el=0: eye should be at (distance, 0, 0)
    assert abs(eye.x - 10.0) < 1e-4
    assert abs(eye.y) < 1e-4
    assert abs(eye.z) < 1e-4


def test_camera_elevation_clamp():
    """Elevation should be clamped to +/-89 degrees."""
    from creation_lib.renderer.camera import OrbitCamera
    cam = OrbitCamera()
    cam.elevation = 100.0
    cam._clamp_elevation()
    assert cam.elevation <= 89.0
    cam.elevation = -100.0
    cam._clamp_elevation()
    assert cam.elevation >= -89.0


def test_frame_sets_distance_and_target():
    """frame() should set target and distance from bounding sphere."""
    from creation_lib.renderer.camera import OrbitCamera
    cam = OrbitCamera()
    cam.frame_on_bounds(center=glm.vec3(5, 5, 5), radius=10.0)
    assert glm.length(cam.target - glm.vec3(5, 5, 5)) < 1e-4
    assert cam.distance > 10.0  # should be > radius


def test_set_front_view():
    """set_front() should set azimuth=90, elevation=0."""
    from creation_lib.renderer.camera import OrbitCamera
    cam = OrbitCamera()
    cam.set_front()
    assert abs(cam.azimuth - 90.0) < 1e-4
    assert abs(cam.elevation) < 1e-4
