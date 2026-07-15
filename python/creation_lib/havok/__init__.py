"""Havok animation and collision domain logic — thin wrappers over creation_lib._native.havok_native."""

from creation_lib.animation.models import AnimationClip
from creation_lib.havok.animation_reader import extract_clip, infer_clip_fps, infer_clip_fps_from_xml
from creation_lib.havok.animation_writer import write_animation_xml
from creation_lib.havok.discovery import FileEntry, classify_category, classify_role, discover_havok_files
from creation_lib.havok.manifest import ManifestData, ManifestDep, ManifestFileEntry, build_manifests
from creation_lib.havok.parsers.animation import AnimationData, parse_animation
from creation_lib.havok.parsers.behavior import BehaviorData, parse_behavior
from creation_lib.havok.parsers.character import CharacterData, parse_character
from creation_lib.havok.parsers.project import ProjectData, parse_project
from creation_lib.havok.parsers.skeleton import SkeletonData, parse_skeleton
from creation_lib.havok.spline_decompress import SplineTransform, decompress_spline

__all__ = [
    # conversion models re-exported for convenience
    "AnimationClip",
    # animation_reader
    "extract_clip",
    "infer_clip_fps",
    "infer_clip_fps_from_xml",
    # animation_writer
    "write_animation_xml",
    # discovery
    "FileEntry",
    "classify_category",
    "classify_role",
    "discover_havok_files",
    # manifest
    "ManifestData",
    "ManifestDep",
    "ManifestFileEntry",
    "build_manifests",
    # parsers
    "AnimationData",
    "BehaviorData",
    "CharacterData",
    "ProjectData",
    "SkeletonData",
    "parse_animation",
    "parse_behavior",
    "parse_character",
    "parse_project",
    "parse_skeleton",
    # spline_decompress
    "SplineTransform",
    "decompress_spline",
]
