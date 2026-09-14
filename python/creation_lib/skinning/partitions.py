"""Body part partition assignment, NiSkinPartition block generation, and SSF path helpers."""
from __future__ import annotations

import logging
import copy
from pathlib import PureWindowsPath
from typing import TYPE_CHECKING

import numpy as np
from creation_lib.scientific.native_runtime import CKDTree as cKDTree

if TYPE_CHECKING:
    from .skin_data import SkinData

_log = logging.getLogger("skinning.partitions")

# Bethesda body part IDs (shared across FO4 / Skyrim SE)
BODY_PART_IDS: dict[str, int] = {
    "Body": 30,
    "Head": 32,
    "Hair": 33,
    "L Arm": 34,
    "R Arm": 35,
    "L Hand": 36,
    "R Hand": 36,
    "L Leg": 37,
    "R Leg": 38,
    "L Foot": 39,
    "R Foot": 39,
    "Brain": 13,
    "Weapon": 41,
    "Shield": 42,
    "Tail": 43,
    "Long Hair": 44,
    "Circlet": 45,
    "Ears": 50,
    "Mouth": 52,
    "Eyes": 53,
    "Beard": 54,
    "Decap Head": 20,
    "Decap": 21,
}

# Bone name substrings -> body part ID mapping
BONE_TO_BODY_PART: dict[str, int] = {
    # Torso / Body
    "Pelvis": 30, "Spine": 30, "Chest": 30, "COM": 30,
    "Spine1": 30, "Spine2": 30, "Breast": 30,
    # Head / Neck
    "Head": 32, "Neck": 32,
    # Hair
    "Hair": 33,
    # Left arm
    "LArm": 34, "L_Arm": 34, "LeftArm": 34,
    "L Arm": 34, "LForeArm": 34, "L UpperArm": 34,
    "LArm_UpperArm": 34, "LArm_ForeArm": 34,
    "L Clavicle": 34, "LArm_Collarbone": 34,
    # Right arm
    "RArm": 35, "R_Arm": 35, "RightArm": 35,
    "R Arm": 35, "RForeArm": 35, "R UpperArm": 35,
    "RArm_UpperArm": 35, "RArm_ForeArm": 35,
    "R Clavicle": 35, "RArm_Collarbone": 35,
    # Left hand
    "LHand": 36, "L_Hand": 36, "LeftHand": 36,
    "LArm_Hand": 36, "LArm_Finger": 36,
    # Right hand
    "RHand": 36, "R_Hand": 36, "RightHand": 36,
    "RArm_Hand": 36, "RArm_Finger": 36,
    # Left leg
    "LLeg": 37, "L_Leg": 37, "LeftLeg": 37,
    "L Leg": 37, "LCalf": 37, "L Thigh": 37,
    "LLeg_Thigh": 37, "LLeg_Calf": 37,
    # Right leg
    "RLeg": 38, "R_Leg": 38, "RightLeg": 38,
    "R Leg": 38, "RCalf": 38, "R Thigh": 38,
    "RLeg_Thigh": 38, "RLeg_Calf": 38,
    # Feet
    "LFoot": 39, "L_Foot": 39, "LeftFoot": 39, "LLeg_Foot": 39,
    "RFoot": 39, "R_Foot": 39, "RightFoot": 39, "RLeg_Foot": 39,
    "LToe": 39, "RToe": 39,
    "LLeg_Toe": 39, "RLeg_Toe": 39,
}


def _dominant_bone_for_triangle(
    tri: np.ndarray,
    weights: np.ndarray,
    bone_indices: np.ndarray,
) -> int:
    """Return the bone index with highest total weight across triangle corners."""
    bone_weight_map: dict[int, float] = {}
    for vi in tri:
        for j in range(weights.shape[1]):
            w = float(weights[vi, j])
            bi = int(bone_indices[vi, j])
            if w > 0:
                bone_weight_map[bi] = bone_weight_map.get(bi, 0.0) + w

    if not bone_weight_map:
        return 0
    return max(bone_weight_map, key=bone_weight_map.get)  # type: ignore[arg-type]


def _bone_name_to_part(bone_name: str) -> int:
    """Map a bone name to a body part ID using substring matching."""
    # Try exact match first
    if bone_name in BONE_TO_BODY_PART:
        return BONE_TO_BODY_PART[bone_name]
    # Try substring match (longest match first for specificity)
    best_match = ""
    best_part = 30  # Default to Body
    for pattern, part_id in BONE_TO_BODY_PART.items():
        if pattern in bone_name and len(pattern) > len(best_match):
            best_match = pattern
            best_part = part_id
    return best_part


def assign_partitions_from_reference(
    target: "SkinData",
    reference: "SkinData",
) -> np.ndarray:
    """Assign partition IDs by matching target triangles to nearest reference triangles.

    Each target triangle copies the partition ID of the reference triangle with
    the nearest centroid. Returns (M,) int32 IDs, -1 where none applies.
    """
    n_target_tris = target.num_triangles
    partitions = np.full(n_target_tris, -1, dtype=np.int32)

    if reference.num_triangles == 0 or n_target_tris == 0:
        return partitions

    # Build centroid KD-tree from reference
    ref_verts = np.asarray(reference.vertices, dtype=np.float64)
    ref_tris = np.asarray(reference.triangles, dtype=np.int32)
    ref_centroids = (
        ref_verts[ref_tris[:, 0]]
        + ref_verts[ref_tris[:, 1]]
        + ref_verts[ref_tris[:, 2]]
    ) / 3.0
    tree = cKDTree(ref_centroids)

    # Target centroids
    tgt_verts = np.asarray(target.vertices, dtype=np.float64)
    tgt_tris = np.asarray(target.triangles, dtype=np.int32)
    tgt_centroids = (
        tgt_verts[tgt_tris[:, 0]]
        + tgt_verts[tgt_tris[:, 1]]
        + tgt_verts[tgt_tris[:, 2]]
    ) / 3.0

    _, nearest = tree.query(tgt_centroids)
    ref_parts = np.asarray(reference.segment_ids, dtype=np.int32)

    for i in range(n_target_tris):
        ri = int(nearest[i]) if not isinstance(nearest, (int, np.integer)) else int(nearest)
        partitions[i] = ref_parts[ri] if 0 <= ri < len(ref_parts) else -1

    return partitions


def assign_partitions_from_bones(
    skin_data: "SkinData",
) -> np.ndarray:
    """Assign partition IDs based on dominant bone per triangle.

    The bone with the highest total weight over a triangle's three corners maps
    to a body part ID; the default is 30 (Body). Returns (M,) int32 IDs.
    """
    n_tris = skin_data.num_triangles
    partitions = np.full(n_tris, 30, dtype=np.int32)  # Default to Body

    if n_tris == 0 or not skin_data.bone_names:
        return partitions

    for i in range(n_tris):
        tri = skin_data.triangles[i]
        dom_bone_idx = _dominant_bone_for_triangle(
            tri, skin_data.weights, skin_data.bone_indices
        )
        if 0 <= dom_bone_idx < len(skin_data.bone_names):
            bone_name = skin_data.bone_names[dom_bone_idx]
            partitions[i] = _bone_name_to_part(bone_name)

    return partitions


def rebuild_fo4_segments(
    skin_data: "SkinData",
) -> list["SegmentInfo"]:
    """Rebuild FO4 SegmentInfo hierarchy from flat segment_ids array.

    ``segment_ids`` values are treated as body part IDs (user_index). Each
    contiguous run of one ID becomes a segment with a single sub-segment.
    """
    segments, _ = rebuild_fo4_segments_from_body_parts(
        skin_data, skin_data.segment_ids,
    )
    return segments


def rebuild_fo4_segments_from_body_parts(
    skin_data: "SkinData",
    body_part_ids: np.ndarray,
) -> tuple[list["SegmentInfo"], np.ndarray]:
    """Build FO4 segments from per-triangle body part IDs.

    Returns the rebuilt segment hierarchy plus the per-triangle segment index
    array that points at that hierarchy.  Body part IDs are stored in segment
    metadata, not reused as segment indexes.
    """
    from .skin_data import SegmentInfo, SubSegmentInfo

    n_tris = skin_data.num_triangles
    if n_tris == 0:
        return [], np.empty((0,), dtype=np.int32)

    part_ids = np.asarray(body_part_ids, dtype=np.int32).reshape(-1)
    if len(part_ids) != n_tris:
        raise ValueError(
            f"Expected {n_tris} body part IDs, got {len(part_ids)}"
        )

    segments: list[SegmentInfo] = []
    segment_ids = np.full(n_tris, -1, dtype=np.int32)

    run_start = 0
    while run_start < n_tris:
        pid = int(part_ids[run_start])
        if pid < 0:
            pid = 30
        run_end = run_start + 1
        while run_end < n_tris:
            next_pid = int(part_ids[run_end])
            if next_pid < 0:
                next_pid = 30
            if next_pid != pid:
                break
            run_end += 1

        seg_idx = len(segments)
        count = run_end - run_start
        sub = SubSegmentInfo(
            start_index=run_start,
            num_primitives=count,
            user_index=pid,
            bone_id=0xFFFFFFFF,
        )
        segments.append(SegmentInfo(
            start_index=run_start,
            num_primitives=count,
            sub_segments=[sub],
            user_index=pid,
        ))
        segment_ids[run_start:run_end] = seg_idx
        run_start = run_end

    return segments, segment_ids


def sync_fo4_segments_from_ids(skin_data: "SkinData") -> list["SegmentInfo"]:
    """Rebuild FO4 segment ranges from per-triangle segment indexes.

    Existing ``skin_data.segments`` entries are used as metadata templates for
    matching segment indexes.  This keeps body part IDs, bone IDs, cut offsets,
    and SSF metadata attached while allowing painted triangle assignments to
    change segment ranges.
    """
    from .skin_data import SegmentInfo, SubSegmentInfo

    n_tris = skin_data.num_triangles
    if n_tris == 0:
        return []

    segment_ids = np.asarray(skin_data.segment_ids, dtype=np.int32).reshape(-1)
    if len(segment_ids) != n_tris:
        raise ValueError(
            f"Expected {n_tris} segment IDs, got {len(segment_ids)}"
        )

    synced: list[SegmentInfo] = []
    remapped_ids = np.full(n_tris, -1, dtype=np.int32)
    run_start = 0
    while run_start < n_tris:
        seg_id = int(segment_ids[run_start])
        run_end = run_start + 1
        while run_end < n_tris and int(segment_ids[run_end]) == seg_id:
            run_end += 1

        if seg_id >= 0:
            count = run_end - run_start
            template = (
                skin_data.segments[seg_id]
                if 0 <= seg_id < len(skin_data.segments)
                else None
            )
            if template is not None:
                user_index = template.user_index
                if template.sub_segments:
                    source_sub = template.sub_segments[0]
                    sub_segments = [
                        SubSegmentInfo(
                            start_index=run_start,
                            num_primitives=count,
                            user_index=source_sub.user_index,
                            bone_id=source_sub.bone_id,
                            cut_offsets=copy.deepcopy(source_sub.cut_offsets),
                        )
                    ]
                else:
                    sub_segments = []
            else:
                user_index = seg_id
                sub_segments = [
                    SubSegmentInfo(
                        start_index=run_start,
                        num_primitives=count,
                        user_index=seg_id,
                        bone_id=0xFFFFFFFF,
                    )
                ]

            remapped_ids[run_start:run_end] = len(synced)
            synced.append(SegmentInfo(
                start_index=run_start,
                num_primitives=count,
                sub_segments=sub_segments,
                user_index=user_index,
            ))

        run_start = run_end

    skin_data.segment_ids = remapped_ids
    return synced


def generate_ssf_path(nif_path: str) -> str:
    """Generate an SSF file path from a NIF file path.

    Bethesda convention: the SSF path mirrors the NIF path with a .ssf
    extension, backslash-delimited and rooted at ``meshes\\``. A path with no
    ``meshes`` component puts the file name directly under ``meshes\\``.

    Examples:
        ``meshes\\Clothes\\Bathrobe\\OutfitM.nif`` → ``meshes\\Clothes\\Bathrobe\\OutfitM.ssf``
        ``C:\\Data\\meshes\\Armor\\MyArmor.nif`` → ``meshes\\Armor\\MyArmor.ssf``
    """
    p = PureWindowsPath(nif_path)

    # Find the 'meshes' component and build relative from there
    parts_lower = [part.lower() for part in p.parts]
    try:
        meshes_idx = parts_lower.index("meshes")
        rel = PureWindowsPath(*p.parts[meshes_idx:])
    except ValueError:
        # No meshes component — use filename only under meshes\
        rel = PureWindowsPath("meshes", p.name)

    return str(rel.with_suffix(".ssf"))


def generate_skin_partition_blocks(
    skin_data: "SkinData",
    max_bones_per_partition: int = 80,
) -> list[dict]:
    """Generate NiSkinPartition-compatible partition block data.

    Groups triangles by partition ID, then splits groups that exceed
    *max_bones_per_partition* bones. Each dict has "body_part", "bones" (bone
    indices), "triangles" ((v0, v1, v2) tuples), "vertex_map" (global vertex
    indices), "num_vertices", and "num_triangles".
    """
    if skin_data.num_triangles == 0:
        return []

    # Group triangles by partition ID
    groups: dict[int, list[int]] = {}
    for ti in range(skin_data.num_triangles):
        pid = int(skin_data.segment_ids[ti])
        groups.setdefault(pid, []).append(ti)

    partitions: list[dict] = []

    for pid, tri_indices in sorted(groups.items()):
        # Collect all vertices and bones used in this group
        vert_set: set[int] = set()
        bone_set: set[int] = set()

        for ti in tri_indices:
            tri = skin_data.triangles[ti]
            for vi in tri:
                vi = int(vi)
                vert_set.add(vi)
                for j in range(skin_data.weights.shape[1]):
                    w = float(skin_data.weights[vi, j])
                    bi = int(skin_data.bone_indices[vi, j])
                    if w > 0:
                        bone_set.add(bi)

        bone_list = sorted(bone_set)
        vert_list = sorted(vert_set)

        # If under the bone limit, emit as a single partition
        if len(bone_list) <= max_bones_per_partition:
            tris = []
            for ti in tri_indices:
                t = skin_data.triangles[ti]
                tris.append((int(t[0]), int(t[1]), int(t[2])))

            partitions.append({
                "body_part": pid,
                "bones": bone_list,
                "triangles": tris,
                "vertex_map": vert_list,
                "num_vertices": len(vert_list),
                "num_triangles": len(tris),
            })
        else:
            # Split into sub-partitions (greedy approach)
            _log.info(
                "Partition %d has %d bones, splitting (max %d)",
                pid, len(bone_list), max_bones_per_partition,
            )
            remaining = list(tri_indices)
            sub_idx = 0
            while remaining:
                sub_tris: list[int] = []
                sub_bones: set[int] = set()
                still_remaining: list[int] = []

                for ti in remaining:
                    # Bones needed for this triangle
                    tri = skin_data.triangles[ti]
                    tri_bones: set[int] = set()
                    for vi in tri:
                        vi = int(vi)
                        for j in range(skin_data.weights.shape[1]):
                            w = float(skin_data.weights[vi, j])
                            bi = int(skin_data.bone_indices[vi, j])
                            if w > 0:
                                tri_bones.add(bi)

                    candidate = sub_bones | tri_bones
                    if len(candidate) <= max_bones_per_partition:
                        sub_bones = candidate
                        sub_tris.append(ti)
                    else:
                        still_remaining.append(ti)

                # Emit sub-partition
                sub_vert_set: set[int] = set()
                sub_tri_list = []
                for ti in sub_tris:
                    t = skin_data.triangles[ti]
                    sub_tri_list.append((int(t[0]), int(t[1]), int(t[2])))
                    sub_vert_set.update(int(v) for v in t)

                partitions.append({
                    "body_part": pid,
                    "bones": sorted(sub_bones),
                    "triangles": sub_tri_list,
                    "vertex_map": sorted(sub_vert_set),
                    "num_vertices": len(sub_vert_set),
                    "num_triangles": len(sub_tri_list),
                })

                remaining = still_remaining
                sub_idx += 1

    return partitions
