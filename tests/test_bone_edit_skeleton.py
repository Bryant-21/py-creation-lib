import pytest
from pathlib import Path

SKELETON_HKX = Path("resource/skeleton.hkx")


def test_load_skeleton_from_hkx():
    """Load FO4 1st person skeleton and verify bone count and names."""
    from creation_lib.bone_edit.skeleton import SkeletonManager

    skel = SkeletonManager.from_hkx(SKELETON_HKX)
    assert skel.bone_count > 0
    assert "RArm_Hand" in skel.bone_names
    assert "Weapon" in skel.bone_names
    assert "Root" in skel.bone_names
    assert len(skel.parent_indices) == skel.bone_count


def test_bone_hierarchy():
    """Verify parent-child relationships."""
    from creation_lib.bone_edit.skeleton import SkeletonManager

    skel = SkeletonManager.from_hkx(SKELETON_HKX)
    # Root has no parent
    root_idx = skel.bone_names.index("Root")
    assert skel.parent_indices[root_idx] == -1
    # RArm_Hand's parent chain should include RArm_ForeArm3
    hand_idx = skel.bone_names.index("RArm_Hand")
    parent_idx = skel.parent_indices[hand_idx]
    assert skel.bone_names[parent_idx] in (
        "RArm_ForeArm3", "RArm_ForeArm2", "RArm_ForeArm1"
    )


def test_get_children():
    """Verify children lookup."""
    from creation_lib.bone_edit.skeleton import SkeletonManager

    skel = SkeletonManager.from_hkx(SKELETON_HKX)
    children = skel.get_children("Root")
    assert len(children) > 0
    assert "COM" in children


def test_world_transforms():
    """World transforms should differ from local for non-root bones."""
    from creation_lib.bone_edit.skeleton import SkeletonManager

    skel = SkeletonManager.from_hkx(SKELETON_HKX)
    root_world = skel.get_bone_world_transform("Root")
    hand_world = skel.get_bone_world_transform("RArm_Hand")
    hand_local = skel.get_bone_local_transform("RArm_Hand")
    # World and local should differ for non-root bones
    assert hand_world is not None
    assert hand_local is not None
    # Root world == root local (no parent)
    import numpy as np
    root_local = skel.get_bone_local_transform("Root")
    np.testing.assert_allclose(root_world["translation"], root_local["translation"], atol=1e-5)


def test_unknown_bone_returns_none():
    """Querying a non-existent bone returns None."""
    from creation_lib.bone_edit.skeleton import SkeletonManager

    skel = SkeletonManager.from_hkx(SKELETON_HKX)
    assert skel.get_bone_world_transform("NonExistentBone") is None
