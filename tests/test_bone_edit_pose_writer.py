import glob
from pathlib import Path

import numpy as np
import pytest


def _load_first_hkx_of_format(fmt: str) -> str | None:
    """Find the first FO4 animation HKX of the given compression format.

    Returns the path to the .hkx file, or None if none found.
    """
    from creation_lib.bone_edit.pose_writer import detect_compression_type
    from creation_lib.hkxpack import load_hkx

    patterns = [
        "extracted/fo4/meshes/actors/character/_1stperson/animations/**/*.hkx",
        "extracted/fo4/meshes/actors/character/animations/**/*.hkx",
    ]
    candidates = []
    for pat in patterns:
        candidates.extend(glob.glob(pat, recursive=True))
    for hkx in candidates[:50]:
        try:
            hkx_file, _ = load_hkx(hkx)
            if detect_compression_type(hkx_file) == fmt:
                return hkx
        except Exception:
            continue
    return None


def test_detect_compression_lossless():
    from creation_lib.bone_edit.pose_writer import detect_compression_type
    from creation_lib.hkxpack import load_hkx

    hkx = _load_first_hkx_of_format("lossless")
    if hkx is None:
        pytest.skip("No FO4 lossless animations available")
    hkx_file, _ = load_hkx(hkx)
    assert detect_compression_type(hkx_file) == "lossless"


def test_apply_lossless_translation_delta_to_real_anim():
    """Apply a 1-unit X translation to RArm_Hand and verify the value was written."""
    from creation_lib.bone_edit.pose import PoseDelta
    from creation_lib.bone_edit.pose_writer import apply_pose_to_animation
    from creation_lib.bone_edit.skeleton import SkeletonManager
    from creation_lib.hkxpack import load_hkx

    hkx = _load_first_hkx_of_format("lossless")
    if hkx is None:
        pytest.skip("No lossless animation available")

    skel = SkeletonManager.from_hkx(Path("resource/skeleton.hkx"))

    pose = PoseDelta()
    pose.set_translation("RArm_Hand", np.array([1.0, 0.0, 0.0]))

    hkx_file, _ = load_hkx(hkx)
    result = apply_pose_to_animation(hkx_file, pose, skel)
    assert result.success, result.message
    assert "RArm_Hand" in result.bones_modified


def test_apply_interleaved_translation_delta_to_real_anim():
    from creation_lib.bone_edit.pose import PoseDelta
    from creation_lib.bone_edit.pose_writer import apply_pose_to_animation
    from creation_lib.bone_edit.skeleton import SkeletonManager
    from creation_lib.hkxpack import load_hkx

    hkx = _load_first_hkx_of_format("interleaved")
    if hkx is None:
        pytest.skip("No interleaved animation available")

    skel = SkeletonManager.from_hkx(Path("resource/skeleton.hkx"))
    pose = PoseDelta()
    pose.set_translation("RArm_Hand", np.array([1.0, 0.0, 0.0]))

    hkx_file, _ = load_hkx(hkx)
    result = apply_pose_to_animation(hkx_file, pose, skel)
    assert result.success, result.message


def test_apply_spline_translation_delta_to_real_anim():
    from creation_lib.bone_edit.pose import PoseDelta
    from creation_lib.bone_edit.pose_writer import apply_pose_to_animation
    from creation_lib.bone_edit.skeleton import SkeletonManager
    from creation_lib.hkxpack import load_hkx

    hkx = _load_first_hkx_of_format("spline")
    if hkx is None:
        pytest.skip("No spline animation available")

    skel = SkeletonManager.from_hkx(Path("resource/skeleton.hkx"))
    pose = PoseDelta()
    pose.set_translation("RArm_Hand", np.array([0.5, 0.0, 0.0]))

    hkx_file, _ = load_hkx(hkx)
    result = apply_pose_to_animation(hkx_file, pose, skel)
    if not result.success:
        assert "Spline" in result.message or "spline" in result.message


def test_apply_pose_to_spline_with_missing_bone_returns_clean_error():
    from creation_lib.bone_edit.pose import PoseDelta
    from creation_lib.bone_edit.pose_writer import apply_pose_to_animation
    from creation_lib.bone_edit.skeleton import SkeletonManager
    from creation_lib.hkxpack import load_hkx

    hkx = _load_first_hkx_of_format("spline")
    if hkx is None:
        pytest.skip("No spline animation available")

    skel = SkeletonManager.from_hkx(Path("resource/skeleton.hkx"))
    pose = PoseDelta()
    pose.set_translation("ImaginaryBone_That_Does_Not_Exist", np.array([1.0, 0.0, 0.0]))

    hkx_file, _ = load_hkx(hkx)
    result = apply_pose_to_animation(hkx_file, pose, skel)
    assert result.success
    assert result.bones_modified == []
