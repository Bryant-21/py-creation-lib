import numpy as np
import pytest


def _identity_quat():
    return np.array([0.0, 0.0, 0.0, 1.0])


def _identity_rot_mat():
    return np.eye(3, dtype=np.float64)


def test_ik_solver_target_at_current_returns_identity_change():
    """If target is exactly at the current tip, root and mid rotations are unchanged."""
    from creation_lib.bone_edit.ik_solver import solve_two_bone_ik

    root_pos = np.array([0.0, 0.0, 0.0])
    mid_pos = np.array([1.0, 0.0, 0.0])
    tip_pos = np.array([2.0, 0.0, 0.0])
    target = tip_pos.copy()
    pole = np.array([1.0, 0.0, 1.0])

    new_root_q, new_mid_q = solve_two_bone_ik(
        root_world_pos=root_pos,
        mid_world_pos=mid_pos,
        tip_world_pos=tip_pos,
        target_world_pos=target,
        pole_world_pos=pole,
        root_to_mid_length=1.0,
        mid_to_tip_length=1.0,
        root_world_rot=_identity_quat(),
        mid_world_rot=_identity_quat(),
    )
    assert abs(new_root_q[0]) < 1e-6
    assert abs(new_root_q[1]) < 1e-6
    assert abs(new_root_q[2]) < 1e-6
    assert abs(new_root_q[3] - 1.0) < 1e-6


def test_ik_solver_unreachable_target_extends_chain():
    """Target beyond reach: chain fully extended along target direction."""
    from creation_lib.bone_edit.ik_solver import solve_two_bone_ik
    from creation_lib.bone_edit.quat_util import quat_to_matrix

    root_pos = np.array([0.0, 0.0, 0.0])
    mid_pos = np.array([1.0, 0.0, 0.0])
    tip_pos = np.array([2.0, 0.0, 0.0])
    target = np.array([5.0, 0.0, 0.0])
    pole = np.array([1.0, 0.0, 1.0])
    l1, l2 = 1.0, 1.0

    new_root_q, new_mid_q = solve_two_bone_ik(
        root_world_pos=root_pos, mid_world_pos=mid_pos, tip_world_pos=tip_pos,
        target_world_pos=target, pole_world_pos=pole,
        root_to_mid_length=l1, mid_to_tip_length=l2,
        root_world_rot=_identity_quat(), mid_world_rot=_identity_quat(),
    )

    new_root_mat = quat_to_matrix(new_root_q)
    new_mid_world = root_pos + new_root_mat @ np.array([l1, 0, 0])
    new_mid_mat = quat_to_matrix(new_mid_q) @ new_root_mat
    new_tip_world = new_mid_world + new_mid_mat @ np.array([l2, 0, 0])

    np.testing.assert_allclose(new_tip_world, [l1 + l2, 0, 0], atol=1e-3)


def test_ik_solver_reachable_target_hits_target():
    """Target within reach: solved tip lands on target."""
    from creation_lib.bone_edit.ik_solver import solve_two_bone_ik
    from creation_lib.bone_edit.quat_util import quat_to_matrix

    root_pos = np.array([0.0, 0.0, 0.0])
    mid_pos = np.array([1.0, 0.0, 0.0])
    tip_pos = np.array([2.0, 0.0, 0.0])
    target = np.array([1.5, 0.0, 0.5])
    pole = np.array([1.0, 0.0, 1.0])
    l1, l2 = 1.0, 1.0

    new_root_q, new_mid_q = solve_two_bone_ik(
        root_world_pos=root_pos, mid_world_pos=mid_pos, tip_world_pos=tip_pos,
        target_world_pos=target, pole_world_pos=pole,
        root_to_mid_length=l1, mid_to_tip_length=l2,
        root_world_rot=_identity_quat(), mid_world_rot=_identity_quat(),
    )

    new_root_mat = quat_to_matrix(new_root_q)
    bone_axis = np.array([1.0, 0.0, 0.0])
    new_mid_world = root_pos + new_root_mat @ (bone_axis * l1)
    new_mid_mat = new_root_mat @ quat_to_matrix(new_mid_q)
    new_tip_world = new_mid_world + new_mid_mat @ (bone_axis * l2)

    np.testing.assert_allclose(new_tip_world, target, atol=1e-4)


def test_ik_solver_preserves_root_to_mid_length():
    """Bone length must never change."""
    from creation_lib.bone_edit.ik_solver import solve_two_bone_ik
    from creation_lib.bone_edit.quat_util import quat_to_matrix

    root_pos = np.array([0.0, 0.0, 0.0])
    mid_pos = np.array([1.0, 0.0, 0.0])
    tip_pos = np.array([2.0, 0.0, 0.0])
    target = np.array([0.5, 0.5, 0.5])
    pole = np.array([1.0, 0.0, 1.0])
    l1, l2 = 1.0, 1.0

    new_root_q, _ = solve_two_bone_ik(
        root_world_pos=root_pos, mid_world_pos=mid_pos, tip_world_pos=tip_pos,
        target_world_pos=target, pole_world_pos=pole,
        root_to_mid_length=l1, mid_to_tip_length=l2,
        root_world_rot=_identity_quat(), mid_world_rot=_identity_quat(),
    )

    new_root_mat = quat_to_matrix(new_root_q)
    new_mid_world = root_pos + new_root_mat @ np.array([l1, 0, 0])
    actual_len = float(np.linalg.norm(new_mid_world - root_pos))
    assert abs(actual_len - l1) < 1e-6


def test_world_rot_delta_to_local():
    """world_rot_delta_to_local: world_new = parent * local_new gives the right inverse."""
    from creation_lib.bone_edit.ik_solver import world_rot_delta_to_local
    from creation_lib.bone_edit.quat_util import quat_multiply, quat_normalize

    half = np.sqrt(0.5)
    parent_world = np.array([0.0, half, 0.0, half])
    world_old = np.array([0.0, 0.0, 0.0, 1.0])
    angle = np.pi / 4
    world_new = np.array([0.0, 0.0, np.sin(angle / 2), np.cos(angle / 2)])

    local_delta = world_rot_delta_to_local(world_new, world_old, parent_world)

    from creation_lib.bone_edit.quat_util import quat_conjugate
    parent_inv = quat_conjugate(parent_world)
    reconstructed_world_delta = quat_normalize(
        quat_multiply(quat_multiply(parent_world, local_delta), parent_inv)
    )
    expected_world_delta = quat_normalize(quat_multiply(world_new, quat_conjugate(world_old)))
    same = np.allclose(reconstructed_world_delta, expected_world_delta, atol=1e-9) \
        or np.allclose(reconstructed_world_delta, -expected_world_delta, atol=1e-9)
    assert same
