"""Skeleton manager — load HKX skeleton, bone hierarchy, FK chain walk, world transforms."""
from __future__ import annotations

import logging
import re
import shutil
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path

import numpy as np

from creation_lib._native.havok_native import unpack_hkx_to_xml

_log = logging.getLogger("bone_edit.skeleton")

_PAREN_RE = re.compile(r"\(([^)]+)\)")


def _quat_to_matrix(q: np.ndarray) -> np.ndarray:
    """Convert quaternion (x, y, z, w) to 3x3 rotation matrix."""
    x, y, z, w = q
    return np.array([
        [1 - 2*(y*y + z*z), 2*(x*y - z*w),     2*(x*z + y*w)],
        [2*(x*y + z*w),     1 - 2*(x*x + z*z), 2*(y*z - x*w)],
        [2*(x*z - y*w),     2*(y*z + x*w),     1 - 2*(x*x + y*y)],
    ], dtype=np.float64)


class SkeletonManager:
    """Manages a Havok skeleton: bone hierarchy, reference pose, world transforms."""

    def __init__(
        self,
        bone_names: list[str],
        parent_indices: list[int],
        ref_translations: np.ndarray,
        ref_rotations: np.ndarray,
        ref_scales: np.ndarray,
    ):
        self.bone_names = bone_names
        self.parent_indices = parent_indices
        self.ref_translations = ref_translations  # (N, 3)
        self.ref_rotations = ref_rotations        # (N, 4) xyzw
        self.ref_scales = ref_scales              # (N, 3)
        self.bone_count = len(bone_names)

        # O(1) name lookup
        self._name_to_idx: dict[str, int] = {
            name: i for i, name in enumerate(bone_names)
        }

        # Build children map
        self._children: dict[int, list[int]] = {}
        for i, parent in enumerate(parent_indices):
            if parent >= 0:
                self._children.setdefault(parent, []).append(i)

        # Cache world transforms (computed lazily)
        self._world_translations: np.ndarray | None = None
        self._world_rotations: list[np.ndarray] | None = None

    @classmethod
    def from_hkx(cls, hkx_path: Path) -> SkeletonManager:
        """Load a skeleton from a binary HKX file."""
        tmp_dir = tempfile.mkdtemp(prefix="hkxunpack_")
        xml_path = Path(tmp_dir) / f"{hkx_path.stem}.xml"
        xml_path.write_text(unpack_hkx_to_xml(str(hkx_path)), encoding="utf-8")
        try:
            return cls._from_xml(xml_path)
        finally:
            shutil.rmtree(tmp_dir, ignore_errors=True)

    @classmethod
    def from_xml(cls, xml_path: Path) -> SkeletonManager:
        """Load a skeleton from an already-unpacked XML file."""
        return cls._from_xml(xml_path)

    @classmethod
    def _from_xml(cls, xml_path: Path) -> SkeletonManager:
        """Parse hkaSkeleton from XML and build SkeletonManager."""
        tree = ET.parse(str(xml_path))
        root = tree.getroot()

        for obj in root.iter("hkobject"):
            if obj.get("class") != "hkaSkeleton":
                continue

            # Bone names
            bone_names: list[str] = []
            bones_param = obj.find("hkparam[@name='bones']")
            if bones_param is not None:
                for bone_obj in bones_param.findall("hkobject"):
                    name_p = bone_obj.find("hkparam[@name='name']")
                    if name_p is not None and name_p.text:
                        bone_names.append(name_p.text.strip())

            # Parent indices
            parent_indices: list[int] = []
            parents_param = obj.find("hkparam[@name='parentIndices']")
            if parents_param is not None and parents_param.text:
                parent_indices = [
                    int(x) for x in parents_param.text.split()
                    if x.lstrip("-").isdigit()
                ]

            # Reference pose — supports two XML formats:
            # 1. Compact: one paren group per bone with 12 floats (tx ty tz tw qx qy qz qw sx sy sz sw)
            # 2. Triplet: three paren groups per bone (translation)(quaternion)(scale)
            ref_translations = []
            ref_rotations = []
            ref_scales = []
            pose_param = obj.find("hkparam[@name='referencePose']")
            if pose_param is not None and pose_param.text:
                groups = _PAREN_RE.findall(pose_param.text)
                if groups and len(groups[0].split()) >= 12:
                    # Compact format: 12 floats per group
                    for g in groups:
                        vals = [float(x) for x in g.split()]
                        ref_translations.append(vals[0:3])
                        ref_rotations.append(vals[4:8])
                        ref_scales.append(vals[8:11])
                else:
                    # Triplet format: 3 groups per bone
                    i = 0
                    while i + 2 < len(groups):
                        t_vals = [float(x) for x in groups[i].split()]
                        q_vals = [float(x) for x in groups[i + 1].split()]
                        s_vals = [float(x) for x in groups[i + 2].split()]
                        ref_translations.append(t_vals[:3])
                        ref_rotations.append(q_vals[:4])
                        ref_scales.append(s_vals[:3])
                        i += 3

            return cls(
                bone_names=bone_names,
                parent_indices=parent_indices,
                ref_translations=np.array(ref_translations, dtype=np.float64),
                ref_rotations=np.array(ref_rotations, dtype=np.float64),
                ref_scales=np.array(ref_scales, dtype=np.float64),
            )

        raise ValueError(f"No hkaSkeleton found in {xml_path}")

    def get_bone_index(self, name: str) -> int | None:
        """Return bone index by name, or None if not found."""
        return self._name_to_idx.get(name)

    def get_children(self, name: str) -> list[str]:
        """Return names of direct children of the given bone."""
        idx = self._name_to_idx.get(name)
        if idx is None:
            return []
        child_indices = self._children.get(idx, [])
        return [self.bone_names[ci] for ci in child_indices]

    def get_bone_local_transform(self, name: str) -> dict | None:
        """Return reference pose local transform as {translation, rotation}."""
        idx = self._name_to_idx.get(name)
        if idx is None:
            return None
        return {
            "translation": self.ref_translations[idx].copy(),
            "rotation": self.ref_rotations[idx].copy(),
        }

    def get_bone_world_transform(self, name: str) -> dict | None:
        """Return world-space transform computed via FK chain walk."""
        idx = self._name_to_idx.get(name)
        if idx is None:
            return None
        self._ensure_world_transforms()
        return {
            "translation": self._world_translations[idx].copy(),
            "rotation": self._world_rotations[idx].copy(),
        }

    def _ensure_world_transforms(self) -> None:
        """Compute world transforms for all bones if not cached."""
        if self._world_translations is not None:
            return

        n = self.bone_count
        world_trans = [None] * n
        world_rots = [None] * n

        for i in range(n):
            parent = self.parent_indices[i]
            local_t = self.ref_translations[i]
            local_r = _quat_to_matrix(self.ref_rotations[i])

            if parent < 0 or parent >= n:
                # Root bone
                world_trans[i] = local_t.copy()
                world_rots[i] = local_r.copy()
            else:
                # world_pos = parent_rot @ local_trans + parent_trans
                world_trans[i] = world_rots[parent] @ local_t + world_trans[parent]
                world_rots[i] = world_rots[parent] @ local_r

        self._world_translations = np.array(world_trans, dtype=np.float64)
        self._world_rotations = world_rots

    def augment_from_nif(self, nif_path: Path) -> int:
        """Add bones from a NIF skeleton that are missing from the HKX.

        NIF skeletons contain '_skin' marker bones used for mesh deformation
        that don't exist in the Havok skeleton.  This method walks the NIF
        hierarchy, finds bones not already present, and adds them with their
        world-space transforms computed from the NIF parent chain.

        Returns the number of bones added.
        """
        from creation_lib.nif.nif_file import NifFile

        nif = NifFile.load(str(nif_path))
        # Invalidate cached world transforms — will be recomputed on next access
        self._world_translations = None
        self._world_rotations = None

        # Build NIF bone hierarchy: block_id -> (name, parent_block_id, local_transform)
        nif_bones: dict[int, dict] = {}
        parent_map: dict[int, int] = {}  # child_block_id -> parent_block_id

        for i, block in enumerate(nif.blocks):
            if block.type_name != "NiNode":
                continue
            name = block.get_field("Name")
            if not name:
                continue
            name = str(name)
            trans = block.get_field("Translation") or {}
            rot = block.get_field("Rotation") or {}
            nif_bones[i] = {
                "name": name,
                "translation": np.array([
                    float(trans.get("x", 0)),
                    float(trans.get("y", 0)),
                    float(trans.get("z", 0)),
                ], dtype=np.float64),
                # NIF Matrix33 uses column-major naming: mCR = col C, row R
                "rotation": np.array([
                    [float(rot.get("m11", 1)), float(rot.get("m21", 0)), float(rot.get("m31", 0))],
                    [float(rot.get("m12", 0)), float(rot.get("m22", 1)), float(rot.get("m32", 0))],
                    [float(rot.get("m13", 0)), float(rot.get("m23", 0)), float(rot.get("m33", 1))],
                ], dtype=np.float64),
            }
            # Record parent-child relationships
            children = block.get_field("Children") or []
            if isinstance(children, list):
                for child_id in children:
                    if isinstance(child_id, int) and child_id >= 0:
                        parent_map[child_id] = i

        # Compute NIF world transforms via parent chain walk
        nif_world_cache: dict[int, tuple[np.ndarray, np.ndarray]] = {}

        def _nif_world(block_id: int) -> tuple[np.ndarray, np.ndarray]:
            if block_id in nif_world_cache:
                return nif_world_cache[block_id]
            bone = nif_bones.get(block_id)
            if bone is None:
                result = (np.zeros(3, dtype=np.float64), np.eye(3, dtype=np.float64))
                nif_world_cache[block_id] = result
                return result
            local_t = bone["translation"]
            local_r = bone["rotation"]
            pid = parent_map.get(block_id, -1)
            if pid < 0 or pid not in nif_bones:
                nif_world_cache[block_id] = (local_t.copy(), local_r.copy())
            else:
                pt, pr = _nif_world(pid)
                wt = pr @ local_t + pt
                wr = pr @ local_r
                nif_world_cache[block_id] = (wt, wr)
            return nif_world_cache[block_id]

        # Add missing bones
        added = 0
        for block_id, bone in nif_bones.items():
            name = bone["name"]
            if name in self._name_to_idx:
                continue  # already in HKX skeleton

            # Determine HKX parent name
            pid = parent_map.get(block_id, -1)
            hkx_parent_idx = -1
            if pid >= 0 and pid in nif_bones:
                parent_name = nif_bones[pid]["name"]
                hkx_parent_idx = self._name_to_idx.get(parent_name, -1)

            # Compute local transform relative to the HKX parent
            local_t = bone["translation"]
            local_r = bone["rotation"]

            # Convert rotation matrix to quaternion (x,y,z,w)
            m = local_r
            trace = m[0, 0] + m[1, 1] + m[2, 2]
            if trace > 0:
                s = 0.5 / np.sqrt(trace + 1.0)
                w = 0.25 / s
                x = (m[2, 1] - m[1, 2]) * s
                y = (m[0, 2] - m[2, 0]) * s
                z = (m[1, 0] - m[0, 1]) * s
            elif m[0, 0] > m[1, 1] and m[0, 0] > m[2, 2]:
                s = 2.0 * np.sqrt(1.0 + m[0, 0] - m[1, 1] - m[2, 2])
                w = (m[2, 1] - m[1, 2]) / s
                x = 0.25 * s
                y = (m[0, 1] + m[1, 0]) / s
                z = (m[0, 2] + m[2, 0]) / s
            elif m[1, 1] > m[2, 2]:
                s = 2.0 * np.sqrt(1.0 + m[1, 1] - m[0, 0] - m[2, 2])
                w = (m[0, 2] - m[2, 0]) / s
                x = (m[0, 1] + m[1, 0]) / s
                y = 0.25 * s
                z = (m[1, 2] + m[2, 1]) / s
            else:
                s = 2.0 * np.sqrt(1.0 + m[2, 2] - m[0, 0] - m[1, 1])
                w = (m[1, 0] - m[0, 1]) / s
                x = (m[0, 2] + m[2, 0]) / s
                y = (m[1, 2] + m[2, 1]) / s
                z = 0.25 * s
            local_quat = np.array([x, y, z, w], dtype=np.float64)

            # Add to skeleton arrays
            new_idx = self.bone_count
            self.bone_names.append(name)
            self.parent_indices.append(hkx_parent_idx)
            self.ref_translations = np.vstack([self.ref_translations, [local_t]])
            self.ref_rotations = np.vstack([self.ref_rotations, [local_quat]])
            self.ref_scales = np.vstack([self.ref_scales, [[1.0, 1.0, 1.0]]])
            self._name_to_idx[name] = new_idx
            if hkx_parent_idx >= 0:
                self._children.setdefault(hkx_parent_idx, []).append(new_idx)
            self.bone_count = len(self.bone_names)
            added += 1

        _log.info("Augmented skeleton with %d NIF-only bones (total: %d)", added, self.bone_count)
        return added
