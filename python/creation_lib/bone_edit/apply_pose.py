"""Iterate a folder of HKX animations, apply a PoseDelta to each, write to output folder.

Goes binary-direct: read_hkx → mutate HKXFile in memory → write_hkx. No XML
tempdir, no tagreader/tagwriter round-trip.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Optional

from .pose import PoseDelta
from .pose_writer import apply_pose_to_animation
from .skeleton import SkeletonManager

_log = logging.getLogger("bone_edit.apply_pose")


@dataclass
class FileResult:
    filename: str = ""
    success: bool = True
    message: str = ""
    dry_run: bool = False


def discover_hkx_files(folder: Path, recursive: bool = False) -> list[Path]:
    iterator = folder.rglob("*.hkx") if recursive else folder.glob("*.hkx")
    return sorted(iterator, key=lambda p: str(p).lower())


def apply_pose_to_folder(
    pose: PoseDelta,
    skeleton_hkx_path: Path,
    animation_folder: Path,
    output_folder: Path,
    dry_run: bool = False,
    progress_callback: Optional[Callable[[int, int, str], None]] = None,
    recursive: bool = False,
) -> list[FileResult]:
    """Apply `pose` to every .hkx file in `animation_folder`, write to `output_folder`.

    Per-file errors are captured in FileResult.message and do not halt
    the run. The skeleton is loaded once and reused for every file.
    """
    from creation_lib._native.havok_native import load_hkx, save_hkx

    files = discover_hkx_files(animation_folder, recursive=recursive)
    if not files:
        _log.warning("No .hkx files found in %s", animation_folder)
        return []

    _log.info(
        "apply_pose_to_folder: skeleton=%s, in=%s, out=%s, files=%d, dry_run=%s, recursive=%s",
        skeleton_hkx_path, animation_folder, output_folder, len(files), dry_run, recursive,
    )
    skeleton = SkeletonManager.from_hkx(skeleton_hkx_path)
    _log.info(
        "skeleton loaded: %d bones, first 5 = %s, last 5 = %s",
        skeleton.bone_count, skeleton.bone_names[:5], skeleton.bone_names[-5:],
    )

    edited = list(pose.edited_bones())
    _log.info(
        "pose edits: %d bone(s) = %s",
        len(edited),
        edited[:10] + (["..."] if len(edited) > 10 else []),
    )

    output_folder.mkdir(parents=True, exist_ok=True)

    results: list[FileResult] = []
    total = len(files)
    n_ok = 0
    n_fail = 0

    for i, src in enumerate(files, 1):
        try:
            rel_path = src.relative_to(animation_folder)
        except ValueError:
            rel_path = Path(src.name)
        display = rel_path.as_posix()

        if progress_callback:
            progress_callback(i, total, display)

        if dry_run:
            results.append(FileResult(
                filename=display, success=True,
                message="Dry run - would modify", dry_run=True,
            ))
            continue

        try:
            hkx_file, registry = load_hkx(str(src))
            wr = apply_pose_to_animation(hkx_file, pose, skeleton, source_name=display)
            if not wr.success:
                n_fail += 1
                _log.warning("[%s] FAILED: %s", display, wr.message)
                results.append(FileResult(
                    filename=display, success=False, message=wr.message,
                ))
                continue
            out_path = output_folder / rel_path
            out_path.parent.mkdir(parents=True, exist_ok=True)
            save_hkx(hkx_file, registry, str(out_path))
            n_ok += 1
            _log.info(
                "[%s] SAVED -> %s (%d bone(s) modified)",
                display, out_path, len(wr.bones_modified),
            )
            results.append(FileResult(
                filename=display, success=True, message=wr.message,
            ))
        except Exception as e:
            n_fail += 1
            _log.exception("[%s] EXCEPTION during apply", display)
            results.append(FileResult(
                filename=display, success=False, message=str(e),
            ))

    _log.info(
        "apply_pose_to_folder: completed %d/%d files OK, %d failed",
        n_ok, total, n_fail,
    )
    return results
