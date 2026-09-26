import numpy as np
import pytest


def test_pose_delta_set_rotation_and_translation():
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    assert p.is_empty()
    assert p.edited_bones() == set()

    q = np.array([0.0, 0.0, 0.1, 0.99498744])
    p.set_rotation("RArm_Hand", q)
    p.set_translation("RArm_Hand", np.array([1.5, 0.0, 0.0]))
    assert not p.is_empty()
    assert p.edited_bones() == {"RArm_Hand"}
    np.testing.assert_array_equal(p.rotations["RArm_Hand"], q)
    np.testing.assert_array_equal(p.translations["RArm_Hand"], [1.5, 0.0, 0.0])


def test_pose_delta_clear_bone_removes_both_and_missing_is_noop():
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    p.set_rotation("RArm_Hand", np.array([0.0, 0.0, 0.1, 0.995]))
    p.set_translation("RArm_Hand", np.array([1.0, 0.0, 0.0]))
    p.clear_bone("RArm_Hand")
    assert p.is_empty()
    p.clear_bone("nonexistent")
    assert p.is_empty()


def test_pose_delta_identity_rotation_and_zero_translation_not_stored():
    """Setting an identity rotation or zero translation should remove the entry."""
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    p.set_rotation("RArm_Hand", np.array([0.0, 0.0, 0.1, 0.995]))
    p.set_rotation("RArm_Hand", np.array([0.0, 0.0, 0.0, 1.0]))
    assert "RArm_Hand" not in p.rotations
    p.set_translation("RArm_Hand", np.array([1.0, 0.0, 0.0]))
    p.set_translation("RArm_Hand", np.array([0.0, 0.0, 0.0]))
    assert "RArm_Hand" not in p.translations


def test_pose_delta_get_local_transform():
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    assert p.get_local_transform("RArm_Hand") is None
    p.set_rotation("RArm_Hand", np.array([0.0, 0.0, 0.1, 0.995]))
    rot, trans = p.get_local_transform("RArm_Hand")
    np.testing.assert_allclose(rot, [0.0, 0.0, 0.1, 0.995])
    np.testing.assert_array_equal(trans, [0.0, 0.0, 0.0])
    p.set_translation("RArm_Hand", np.array([1.5, 0.0, 0.0]))
    rot, trans = p.get_local_transform("RArm_Hand")
    np.testing.assert_allclose(rot, [0.0, 0.0, 0.1, 0.995])
    np.testing.assert_array_equal(trans, [1.5, 0.0, 0.0])


def test_pose_delta_round_trip_json_and_copy_independent():
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    p.set_rotation("RArm_Hand", np.array([0.0, 0.0, 0.1, 0.995]))
    p.set_translation("HEAD", np.array([0.0, 0.0, 0.5]))
    data = p.to_json()
    p2 = PoseDelta.from_json(data)
    assert p2.edited_bones() == {"RArm_Hand", "HEAD"}
    np.testing.assert_allclose(p2.rotations["RArm_Hand"], [0.0, 0.0, 0.1, 0.995])
    np.testing.assert_array_equal(p2.translations["HEAD"], [0.0, 0.0, 0.5])

    p3 = p.copy()
    p.clear_bone("RArm_Hand")
    assert "RArm_Hand" not in p.rotations
    assert "RArm_Hand" in p3.rotations
