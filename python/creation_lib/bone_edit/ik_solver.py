"""Analytic two-bone IK with pole vector.

Pure functions, no state. Inputs are world-space positions and rotations.
The solver returns the new ROOT rotation in world space and the new MID
rotation in PARENT-LOCAL space (parent = the new root). See
``solve_two_bone_ik`` for the full convention. The mid result is therefore
already in PoseDelta's storage frame; the root result still needs
``world_rot_delta_to_local`` before being stored as a delta.

Bone lengths are inputs, never outputs — the solver only emits rotations.
This makes "skeleton can't deform past original lengths" true by construction.
"""

from __future__ import annotations

import numpy as np

from .quat_util import (
    quat_conjugate,
    quat_from_to,
    quat_multiply,
    quat_normalize,
    quat_to_matrix,
)


_EPS = 1e-9
_DEFAULT_BONE_AXIS = np.array([1.0, 0.0, 0.0])  # fallback when caller passes None


def solve_two_bone_ik(
    root_world_pos: np.ndarray,
    mid_world_pos: np.ndarray,
    tip_world_pos: np.ndarray,
    target_world_pos: np.ndarray,
    pole_world_pos: np.ndarray,
    root_to_mid_length: float,
    mid_to_tip_length: float,
    root_world_rot: np.ndarray,
    mid_world_rot: np.ndarray,
    root_local_child_dir: np.ndarray | None = None,
    mid_local_child_dir: np.ndarray | None = None,
) -> tuple[np.ndarray, np.ndarray]:
    """Solve a 2-bone IK chain.

    Returns (new_root_world_quat, new_mid_local_quat).

    CONVENTION: the root quaternion is in WORLD space, but the mid
    quaternion is in PARENT-LOCAL space, where the parent is the *new*
    root rotation after this solve. This matches PoseDelta's storage
    convention (parent-local) and avoids an extra world->local conversion
    at the call site. Forward-kinematic reconstruction of the tip is:

        new_root_mat = quat_to_matrix(new_root_world_quat)
        new_mid_world = root_pos + new_root_mat @ [l1, 0, 0]
        new_mid_world_mat = new_root_mat @ quat_to_matrix(new_mid_local_quat)
        new_tip_world = new_mid_world + new_mid_world_mat @ [l2, 0, 0]

    Bone lengths are preserved exactly. If the target is unreachable, the
    chain extends fully along the target direction.

    All input positions and rotations are world space. Quaternions are
    (x, y, z, w).

    ``root_local_child_dir`` and ``mid_local_child_dir`` are unit vectors
    in the respective bone's local frame pointing toward the next joint
    in the chain (root→mid and mid→tip). They default to +X for
    backward compatibility with bone conventions that already align the
    bone's +X axis with its child. Any chain where the bone's rest
    rotation does NOT align +X with the child direction (e.g. FO4 Power
    Armor UpperArm has a ~1.2° offset) MUST pass these explicitly, or
    the solver becomes non-idempotent: each re-solve with the same
    inputs drifts the tip, which appears as the chain slowly swinging
    while the user is holding a pole or IK target stationary.
    """
    l1 = float(root_to_mid_length)
    l2 = float(mid_to_tip_length)
    max_reach = l1 + l2 - _EPS

    if root_local_child_dir is None:
        root_axis = _DEFAULT_BONE_AXIS
    else:
        root_axis = np.asarray(root_local_child_dir, dtype=np.float64)
        n = float(np.linalg.norm(root_axis))
        root_axis = root_axis / n if n > _EPS else _DEFAULT_BONE_AXIS
    if mid_local_child_dir is None:
        mid_axis = _DEFAULT_BONE_AXIS
    else:
        mid_axis = np.asarray(mid_local_child_dir, dtype=np.float64)
        n = float(np.linalg.norm(mid_axis))
        mid_axis = mid_axis / n if n > _EPS else _DEFAULT_BONE_AXIS

    # 1. Direction and clamped distance from root to target
    d_vec = target_world_pos - root_world_pos
    d_len = float(np.linalg.norm(d_vec))
    if d_len < _EPS:
        return root_world_rot.copy(), mid_world_rot.copy()
    d_clamped = min(max(d_len, _EPS), max_reach)
    forward = d_vec / d_len

    # 2. Law of cosines: angle at root between root-to-target and root-to-mid
    cos_root = (l1 * l1 + d_clamped * d_clamped - l2 * l2) / (2.0 * l1 * d_clamped)
    cos_root = max(-1.0, min(1.0, cos_root))
    theta_root = float(np.arccos(cos_root))

    # 3. Build the IK plane from (root, target_dir, pole)
    pole_dir = pole_world_pos - root_world_pos
    pole_len = float(np.linalg.norm(pole_dir))
    if pole_len < _EPS:
        cur_mid_dir = mid_world_pos - root_world_pos
        pole_dir = cur_mid_dir - forward * float(np.dot(cur_mid_dir, forward))
        pole_len = float(np.linalg.norm(pole_dir))
    if pole_len < _EPS:
        pole_dir = np.array([0.0, 0.0, 1.0])
    else:
        pole_dir = pole_dir / pole_len

    right = np.cross(pole_dir, forward)
    right_len = float(np.linalg.norm(right))
    if right_len < _EPS:
        if abs(forward[2]) < 0.9:
            ortho = np.array([0.0, 0.0, 1.0])
        else:
            ortho = np.array([0.0, 1.0, 0.0])
        right = np.cross(ortho, forward)
        right_len = float(np.linalg.norm(right))
    right = right / right_len
    up = np.cross(forward, right)  # in-plane "up" toward the pole side

    # 4. Place new mid (elbow) world position
    new_mid_world = (
        root_world_pos
        + forward * (l1 * np.cos(theta_root))
        + up * (l1 * np.sin(theta_root))
    )

    # 5. New root rotation: rotate root so that its local child direction
    # (root_axis, in root-local frame) maps to the new world direction
    # toward new_mid_world. For bones where root_axis == +X this reduces
    # to the old behavior; for PA's UpperArm (root_axis slightly off +X)
    # this is what keeps the solve idempotent.
    old_root_axis_world = quat_to_matrix(root_world_rot) @ root_axis
    new_root_axis_world = (new_mid_world - root_world_pos) / l1
    swing_root = quat_from_to(old_root_axis_world, new_root_axis_world)
    new_root_world_rot = quat_normalize(quat_multiply(swing_root, root_world_rot))

    # 6. New mid rotation: expressed as local-to-new-root. We want a
    # rotation q_mid_local such that
    #   new_root_mat @ (q_mid_local @ mid_axis) == tip_dir
    # i.e. mid's local child direction, rotated by q_mid_local and then
    # by the new root, lands along the desired tip direction.
    tip_dir = target_world_pos - new_mid_world
    tip_dir_len = float(np.linalg.norm(tip_dir))
    if tip_dir_len < _EPS:
        return new_root_world_rot, np.array([0.0, 0.0, 0.0, 1.0])
    tip_dir = tip_dir / tip_dir_len
    # Bring tip_dir into new-root-local space.
    new_root_inv = quat_conjugate(new_root_world_rot)
    tip_dir_local = quat_to_matrix(new_root_inv) @ tip_dir
    new_mid_local_rot = quat_normalize(quat_from_to(mid_axis, tip_dir_local))

    return new_root_world_rot, new_mid_local_rot


def world_rot_delta_to_local(
    world_rot_new: np.ndarray,
    world_rot_old: np.ndarray,
    parent_world_rot: np.ndarray,
) -> np.ndarray:
    """Convert a world-space rotation change to a parent-local delta.

    The result is the quaternion `q_local` such that the bone's new local
    rotation is `q_local * old_local`. In world space this is equivalent to:

        world_new = parent * (q_local * old_local)
                  = (parent * q_local * parent^-1) * (parent * old_local)
                  = world_delta * world_old

    Solving: q_local = parent^-1 * world_delta * parent
             where world_delta = world_new * world_old^-1
    """
    world_delta = quat_normalize(quat_multiply(world_rot_new, quat_conjugate(world_rot_old)))
    parent_inv = quat_conjugate(parent_world_rot)
    return quat_normalize(
        quat_multiply(quat_multiply(parent_inv, world_delta), parent_world_rot)
    )
