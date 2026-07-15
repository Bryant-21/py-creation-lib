"""Bone editing engine — pose deltas, IK solver, animation patcher."""

from .skeleton import SkeletonManager
from .pose import PoseDelta
from .bone_classifier import BoneClassifier, BoneCategory, IkChain
from .ik_solver import solve_two_bone_ik, world_rot_delta_to_local
from .pose_writer import (
    apply_pose_to_animation,
    detect_compression_type,
    WriteResult,
)
from .apply_pose import apply_pose_to_folder, FileResult, discover_hkx_files

__all__ = [
    "SkeletonManager",
    "PoseDelta",
    "BoneClassifier", "BoneCategory", "IkChain",
    "solve_two_bone_ik", "world_rot_delta_to_local",
    "apply_pose_to_animation", "detect_compression_type", "WriteResult",
    "apply_pose_to_folder", "FileResult", "discover_hkx_files",
]
