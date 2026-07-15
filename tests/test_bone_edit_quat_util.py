import numpy as np


def test_quat_multiply_identity():
    from creation_lib.bone_edit.quat_util import quat_multiply
    identity = np.array([0.0, 0.0, 0.0, 1.0])
    q = np.array([0.1, 0.2, 0.3, np.sqrt(1 - 0.14)])
    result = quat_multiply(identity, q)
    np.testing.assert_allclose(result, q, atol=1e-12)


def test_quat_multiply_known():
    """90 deg rotation around X composed with 90 deg around X = 180 deg around X."""
    from creation_lib.bone_edit.quat_util import quat_multiply
    half = np.sqrt(0.5)
    rx90 = np.array([half, 0.0, 0.0, half])
    result = quat_multiply(rx90, rx90)
    # 180 deg around X = (1, 0, 0, 0)
    np.testing.assert_allclose(result, [1.0, 0.0, 0.0, 0.0], atol=1e-12)


def test_quat_normalize_unit_unchanged():
    from creation_lib.bone_edit.quat_util import quat_normalize
    half = np.sqrt(0.5)
    q = np.array([half, 0.0, 0.0, half])
    np.testing.assert_allclose(quat_normalize(q), q, atol=1e-12)


def test_quat_normalize_scales_to_unit():
    from creation_lib.bone_edit.quat_util import quat_normalize
    q = np.array([2.0, 0.0, 0.0, 2.0])
    expected = np.array([np.sqrt(0.5), 0.0, 0.0, np.sqrt(0.5)])
    np.testing.assert_allclose(quat_normalize(q), expected, atol=1e-12)


def test_quat_conjugate():
    from creation_lib.bone_edit.quat_util import quat_conjugate
    q = np.array([1.0, 2.0, 3.0, 4.0])
    np.testing.assert_array_equal(quat_conjugate(q), [-1.0, -2.0, -3.0, 4.0])


def test_quat_to_matrix_identity():
    from creation_lib.bone_edit.quat_util import quat_to_matrix
    identity = np.array([0.0, 0.0, 0.0, 1.0])
    np.testing.assert_allclose(quat_to_matrix(identity), np.eye(3), atol=1e-12)


def test_quat_to_matrix_rotation_90_x():
    """90 deg around X: y -> z, z -> -y."""
    from creation_lib.bone_edit.quat_util import quat_to_matrix
    half = np.sqrt(0.5)
    q = np.array([half, 0.0, 0.0, half])
    m = quat_to_matrix(q)
    np.testing.assert_allclose(m @ [0, 1, 0], [0, 0, 1], atol=1e-12)
    np.testing.assert_allclose(m @ [0, 0, 1], [0, -1, 0], atol=1e-12)
