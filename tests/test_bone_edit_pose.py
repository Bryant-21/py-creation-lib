import numpy as np
import pytest


def test_pose_delta_starts_empty():
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    assert p.is_empty()
    assert p.edited_bones() == set()


def test_pose_delta_set_rotation():
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    q = np.array([0.0, 0.0, 0.1, 0.99498744])
    p.set_rotation("RArm_Hand", q)
    assert not p.is_empty()
    assert p.edited_bones() == {"RArm_Hand"}
    np.testing.assert_array_equal(p.rotations["RArm_Hand"], q)


def test_pose_delta_set_translation():
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    p.set_translation("RArm_Hand", np.array([1.5, 0.0, 0.0]))
    assert "RArm_Hand" in p.edited_bones()
    np.testing.assert_array_equal(p.translations["RArm_Hand"], [1.5, 0.0, 0.0])


def test_pose_delta_clear_bone_removes_both():
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    p.set_rotation("RArm_Hand", np.array([0.0, 0.0, 0.1, 0.995]))
    p.set_translation("RArm_Hand", np.array([1.0, 0.0, 0.0]))
    p.clear_bone("RArm_Hand")
    assert p.is_empty()


def test_pose_delta_clear_bone_missing_is_noop():
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    p.clear_bone("nonexistent")
    assert p.is_empty()


def test_pose_delta_identity_rotation_not_stored():
    """Setting an identity rotation should remove the entry."""
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    p.set_rotation("RArm_Hand", np.array([0.0, 0.0, 0.1, 0.995]))
    p.set_rotation("RArm_Hand", np.array([0.0, 0.0, 0.0, 1.0]))
    assert "RArm_Hand" not in p.rotations


def test_pose_delta_zero_translation_not_stored():
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    p.set_translation("RArm_Hand", np.array([1.0, 0.0, 0.0]))
    p.set_translation("RArm_Hand", np.array([0.0, 0.0, 0.0]))
    assert "RArm_Hand" not in p.translations


def test_pose_delta_get_local_transform_combined():
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    p.set_rotation("RArm_Hand", np.array([0.0, 0.0, 0.1, 0.995]))
    p.set_translation("RArm_Hand", np.array([1.5, 0.0, 0.0]))
    rot, trans = p.get_local_transform("RArm_Hand")
    np.testing.assert_allclose(rot, [0.0, 0.0, 0.1, 0.995])
    np.testing.assert_array_equal(trans, [1.5, 0.0, 0.0])


def test_pose_delta_get_local_transform_only_rotation():
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    p.set_rotation("RArm_Hand", np.array([0.0, 0.0, 0.1, 0.995]))
    rot, trans = p.get_local_transform("RArm_Hand")
    np.testing.assert_allclose(rot, [0.0, 0.0, 0.1, 0.995])
    np.testing.assert_array_equal(trans, [0.0, 0.0, 0.0])


def test_pose_delta_get_local_transform_missing():
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    assert p.get_local_transform("RArm_Hand") is None


def test_pose_delta_round_trip_json():
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    p.set_rotation("RArm_Hand", np.array([0.0, 0.0, 0.1, 0.995]))
    p.set_translation("HEAD", np.array([0.0, 0.0, 0.5]))
    data = p.to_json()
    p2 = PoseDelta.from_json(data)
    assert p2.edited_bones() == {"RArm_Hand", "HEAD"}
    np.testing.assert_allclose(p2.rotations["RArm_Hand"], [0.0, 0.0, 0.1, 0.995])
    np.testing.assert_array_equal(p2.translations["HEAD"], [0.0, 0.0, 0.5])


def test_pose_delta_copy_independent():
    from creation_lib.bone_edit.pose import PoseDelta
    p = PoseDelta()
    p.set_rotation("RArm_Hand", np.array([0.0, 0.0, 0.1, 0.995]))
    p2 = p.copy()
    p.clear_bone("RArm_Hand")
    assert p.is_empty()
    assert "RArm_Hand" in p2.rotations
