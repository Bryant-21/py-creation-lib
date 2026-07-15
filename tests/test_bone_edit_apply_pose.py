import glob
import shutil
from pathlib import Path

import numpy as np
import pytest


def _copy_real_animations_to_tempdir(tmp_path, n: int = 3) -> Path:
    """Find real FO4 animations and copy a few into tmp_path/in/."""
    in_dir = tmp_path / "in"
    in_dir.mkdir()
    patterns = [
        "extracted/fo4/meshes/actors/character/_1stperson/animations/**/*.hkx",
    ]
    found = []
    for pat in patterns:
        found.extend(glob.glob(pat, recursive=True))
    if not found:
        pytest.skip("No FO4 animations available")
    for src in sorted(found)[:n]:
        shutil.copy2(src, in_dir / Path(src).name)
    return in_dir


def test_apply_pose_to_folder_dry_run(tmp_path):
    from creation_lib.bone_edit.apply_pose import apply_pose_to_folder
    from creation_lib.bone_edit.pose import PoseDelta

    in_dir = _copy_real_animations_to_tempdir(tmp_path)
    out_dir = tmp_path / "out"
    pose = PoseDelta()
    pose.set_translation("RArm_Hand", np.array([1.0, 0.0, 0.0]))

    results = apply_pose_to_folder(
        pose=pose,
        skeleton_hkx_path=Path("resource/skeleton.hkx"),
        animation_folder=in_dir,
        output_folder=out_dir,
        dry_run=True,
    )
    assert len(results) == 3
    assert all(r.success for r in results)
    assert not out_dir.exists() or not any(out_dir.iterdir())


def test_apply_pose_to_folder_writes_outputs(tmp_path):
    from creation_lib.bone_edit.apply_pose import apply_pose_to_folder
    from creation_lib.bone_edit.pose import PoseDelta

    in_dir = _copy_real_animations_to_tempdir(tmp_path)
    out_dir = tmp_path / "out"
    pose = PoseDelta()
    pose.set_translation("RArm_Hand", np.array([0.5, 0.0, 0.0]))

    results = apply_pose_to_folder(
        pose=pose,
        skeleton_hkx_path=Path("resource/skeleton.hkx"),
        animation_folder=in_dir,
        output_folder=out_dir,
    )
    successes = [r for r in results if r.success]
    assert len(results) == 3
    assert out_dir.exists()
    output_files = list(out_dir.glob("*.hkx"))
    assert len(output_files) == len(successes)


def test_apply_pose_to_folder_progress_callback(tmp_path):
    from creation_lib.bone_edit.apply_pose import apply_pose_to_folder
    from creation_lib.bone_edit.pose import PoseDelta

    in_dir = _copy_real_animations_to_tempdir(tmp_path, n=3)
    out_dir = tmp_path / "out"
    pose = PoseDelta()
    pose.set_translation("RArm_Hand", np.array([0.5, 0.0, 0.0]))

    calls: list[tuple[int, int, str]] = []
    def cb(cur, total, name):
        calls.append((cur, total, name))

    apply_pose_to_folder(
        pose=pose,
        skeleton_hkx_path=Path("resource/skeleton.hkx"),
        animation_folder=in_dir,
        output_folder=out_dir,
        dry_run=True,
        progress_callback=cb,
    )
    assert len(calls) == 3
    assert calls[0][0] == 1
    assert calls[-1][0] == 3
    assert all(c[1] == 3 for c in calls)


def test_apply_pose_to_empty_folder_returns_empty(tmp_path):
    from creation_lib.bone_edit.apply_pose import apply_pose_to_folder
    from creation_lib.bone_edit.pose import PoseDelta

    in_dir = tmp_path / "in"
    in_dir.mkdir()
    out_dir = tmp_path / "out"
    pose = PoseDelta()
    pose.set_rotation("RArm_Hand", np.array([0.0, 0.0, 0.1, 0.995]))

    results = apply_pose_to_folder(
        pose=pose,
        skeleton_hkx_path=Path("resource/skeleton.hkx"),
        animation_folder=in_dir,
        output_folder=out_dir,
    )
    assert results == []
