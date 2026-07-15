"""Classify bones into categories that drive gizmo behavior.

Default rules target FO4 (1st and 3rd person) skeleton naming. Users can
override individual bones via `set_override`. The full classification is
deterministic given the bone name list, parent indices, and overrides.
"""

from __future__ import annotations

import fnmatch
import re
from dataclasses import dataclass, field
from enum import Enum
from typing import Dict, List, Optional

import numpy as np


class BoneCategory(str, Enum):
    LIMB_SEGMENT = "limb_segment"  # rotation only, length locked
    IK_TIP = "ik_tip"               # translate gizmo = IK target
    IK_POLE = "ik_pole"             # translate gizmo = swivel control
    MOUNT = "mount"                 # translate gizmo = direct local-translation


@dataclass
class IkChain:
    """A 2-bone IK chain: root -> mid -> tip."""
    root: str
    mid: str
    tip: str
    pole_offset: np.ndarray = field(
        default_factory=lambda: np.array([0.0, 0.0, 1.0])
    )


# Pattern -> category. First match wins. fnmatch glob syntax (case-insensitive
# via the wrapper). Order matters.
_DEFAULT_RULES: list[tuple[str, BoneCategory]] = [
    ("*_skin", BoneCategory.LIMB_SEGMENT),    # hidden, handled separately
    ("AnimObject*", BoneCategory.LIMB_SEGMENT),  # hidden
    ("*_Hand", BoneCategory.IK_TIP),
    ("*_Foot", BoneCategory.IK_TIP),
    ("*_ForeArm1", BoneCategory.IK_POLE),
    ("*_Calf", BoneCategory.IK_POLE),
    ("*_ForeArm2", BoneCategory.LIMB_SEGMENT),
    ("*_ForeArm3", BoneCategory.LIMB_SEGMENT),
    ("*_UpperArm", BoneCategory.LIMB_SEGMENT),
    ("*_Thigh", BoneCategory.LIMB_SEGMENT),
    ("*Collarbone", BoneCategory.LIMB_SEGMENT),
    ("Spine*", BoneCategory.LIMB_SEGMENT),
    ("Chest", BoneCategory.LIMB_SEGMENT),
    ("Neck", BoneCategory.LIMB_SEGMENT),
    # Head rotates about the neck joint; translating it would stretch the
    # skull off the neck and distort the skinned mesh. Rotation-only.
    # Match common naming variants: HEAD, Head, head, Bip01 Head, HeadBone, …
    ("HEAD", BoneCategory.LIMB_SEGMENT),
    ("Head", BoneCategory.LIMB_SEGMENT),
    ("head", BoneCategory.LIMB_SEGMENT),
    ("*Head", BoneCategory.LIMB_SEGMENT),
    ("Head*", BoneCategory.LIMB_SEGMENT),
    ("*_Foot_Toe", BoneCategory.MOUNT),
    ("*_Toe*", BoneCategory.MOUNT),
    ("*_Finger*", BoneCategory.MOUNT),
    ("*_Thumb*", BoneCategory.MOUNT),
    ("Pelvis*", BoneCategory.MOUNT),
    ("COM*", BoneCategory.MOUNT),
    ("Root", BoneCategory.MOUNT),
    ("Camera*", BoneCategory.MOUNT),
    ("Weapon*", BoneCategory.MOUNT),
]


_HIDDEN_PATTERNS = ["*_skin", "AnimObject*"]


class BoneClassifier:
    def __init__(self):
        self._overrides: Dict[str, BoneCategory] = {}

    def set_override(self, bone_name: str, category: BoneCategory) -> None:
        self._overrides[bone_name] = category

    def clear_override(self, bone_name: str) -> None:
        self._overrides.pop(bone_name, None)

    def classify_one(self, bone_name: str) -> BoneCategory:
        if bone_name in self._overrides:
            return self._overrides[bone_name]
        for pattern, category in _DEFAULT_RULES:
            if fnmatch.fnmatchcase(bone_name, pattern):
                return category
        return BoneCategory.LIMB_SEGMENT

    def classify_all(
        self,
        bone_names: List[str],
        chains: Optional[Dict[str, IkChain]] = None,
    ) -> Dict[str, BoneCategory]:
        """Return bone_name -> category for every bone.

        If `chains` is provided, IK_TIP bones with no chain are reclassified
        to MOUNT.
        """
        result: Dict[str, BoneCategory] = {}
        for name in bone_names:
            cat = self.classify_one(name)
            if cat == BoneCategory.IK_TIP and chains is not None and name not in chains:
                cat = BoneCategory.MOUNT
            result[name] = cat
        return result

    def hidden_bones(self, bone_names: List[str]) -> set[str]:
        """Bones that should never appear in the editor (NIF skin markers, etc.)."""
        result: set[str] = set()
        for name in bone_names:
            for pat in _HIDDEN_PATTERNS:
                if fnmatch.fnmatchcase(name, pat):
                    result.add(name)
                    break
        return result

    def detect_chains(
        self,
        bone_names: List[str],
        parent_indices: List[int],
    ) -> Dict[str, IkChain]:
        """For every bone classified as IK_TIP, walk parents to find a
        valid (root, mid, tip) chain. Returns tip_name -> IkChain for tips
        that have a complete chain. Tips without one are omitted.
        """
        name_to_idx = {n: i for i, n in enumerate(bone_names)}
        chains: Dict[str, IkChain] = {}

        for tip_name in bone_names:
            if self.classify_one(tip_name) != BoneCategory.IK_TIP:
                continue
            tip_idx = name_to_idx[tip_name]

            # Walk up to find first IK_POLE ancestor
            mid_idx = self._walk_to_category(
                tip_idx, bone_names, parent_indices, BoneCategory.IK_POLE,
            )
            if mid_idx is None:
                continue

            # Walk up from mid to find first matching limb-root
            root_idx = self._walk_to_limb_root(
                mid_idx, bone_names, parent_indices,
            )
            if root_idx is None:
                continue

            chains[tip_name] = IkChain(
                root=bone_names[root_idx],
                mid=bone_names[mid_idx],
                tip=tip_name,
            )
        return chains

    def _walk_to_category(
        self,
        start_idx: int,
        bone_names: List[str],
        parent_indices: List[int],
        target: BoneCategory,
        max_steps: int = 10,
    ) -> Optional[int]:
        cur = parent_indices[start_idx]
        steps = 0
        while 0 <= cur < len(bone_names) and steps < max_steps:
            if self.classify_one(bone_names[cur]) == target:
                return cur
            cur = parent_indices[cur]
            steps += 1
        return None

    def _walk_to_limb_root(
        self,
        start_idx: int,
        bone_names: List[str],
        parent_indices: List[int],
        max_steps: int = 10,
    ) -> Optional[int]:
        """Walk up from a mid bone to find a *_UpperArm or *_Thigh ancestor."""
        cur = parent_indices[start_idx]
        steps = 0
        while 0 <= cur < len(bone_names) and steps < max_steps:
            name = bone_names[cur]
            if name.endswith("_UpperArm") or name.endswith("_Thigh"):
                return cur
            cur = parent_indices[cur]
            steps += 1
        return None
