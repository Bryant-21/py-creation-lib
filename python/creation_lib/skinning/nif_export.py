"""Write edited SkinData back to a NIF file.

Given the original NIF (used as template) and modified SkinData, this module
updates vertex weights, bone indices, and partition/segment data in-place on
the NIF blocks, then saves to disk.

The import path (reference_body.py) merges all BSTriShape blocks into a single
flat SkinData, applying a vertex_offset per shape. Export reverses that:
it walks BSTriShape blocks in the same order and slices the SkinData arrays
back into per-shape portions.
"""
from __future__ import annotations

import copy
import logging
from pathlib import Path

import numpy as np

from .skin_data import SegmentInfo, SkinData, SubSegmentInfo

_log = logging.getLogger("skinning.nif_export")


def write_skin_data_to_nif(
    nif,
    skin_data: SkinData,
    output_path: str,
) -> None:
    """Write modified SkinData back to a NIF and save.

    Args:
        nif: The original NifFile instance (will be modified in-place).
        skin_data: The edited SkinData with updated weights/bone_indices/partitions.
        output_path: Path to write the output NIF.

    Raises:
        ValueError: If NIF structure doesn't match expectations.
    """
    # Find all BSTriShape blocks in order (same order as import)
    shapes = [
        block for block in nif.blocks
        if nif.schema.is_subtype_of(block.type_name, "BSTriShape")
    ]

    if not shapes:
        raise ValueError("No BSTriShape blocks found in NIF")

    # Rebuild FO4 segment hierarchy from segment_ids if needed
    _ensure_segments_synced(nif, shapes, skin_data)

    # Walk shapes and split skin_data back into per-shape slices
    vertex_offset = 0
    tri_offset = 0

    for shape in shapes:
        vertex_data_list = shape.get_field("Vertex Data") or []
        triangles_list = shape.get_field("Triangles") or []

        if not vertex_data_list:
            continue

        n_verts = len(vertex_data_list)
        n_tris = len(triangles_list)

        # Get the shape's local bone list
        shape_bone_names = _get_shape_bone_names(nif, shape)

        # Build global-to-local bone index mapping
        global_to_local = _build_global_to_local_map(
            skin_data.bone_names, shape_bone_names
        )

        # Check if any vertices reference bones not in the shape's bone list.
        # If so, we need to add those bones to the shape's skin instance.
        new_bones = _find_new_bones_for_shape(
            skin_data, vertex_offset, n_verts, shape_bone_names, global_to_local,
        )
        if new_bones:
            _log.info(
                "Adding %d new bones to shape '%s': %s",
                len(new_bones),
                shape.get_field("Name") or shape.type_name,
                new_bones,
            )
            shape_bone_names, global_to_local = _add_bones_to_skin(
                nif, shape, shape_bone_names, new_bones, skin_data,
            )

        # Update vertex data: weights and bone indices
        _write_vertex_weights(
            vertex_data_list, skin_data, vertex_offset, n_verts,
            global_to_local,
        )
        shape.set_field("Vertex Data", vertex_data_list)

        # Write segment/partition data back if present
        if nif.schema.is_subtype_of(shape.type_name, "BSSubIndexTriShape"):
            _write_fo4_segments(shape, skin_data, tri_offset, n_tris)
        else:
            _write_dismember_partitions(nif, shape, skin_data, tri_offset, n_tris)

        vertex_offset += n_verts
        tri_offset += n_tris

    nif.save(output_path)
    _log.info("Exported NIF to %s", output_path)


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------

def _get_shape_bone_names(nif, shape) -> list[str]:
    """Get bone name list for a shape (same logic as reference_body)."""
    skin_ref = shape.get_field("Skin Instance")
    if skin_ref is None or (isinstance(skin_ref, int) and skin_ref < 0):
        skin_ref = shape.get_field("Skin")
    if skin_ref is None or (isinstance(skin_ref, int) and skin_ref < 0):
        return []

    skin_block = nif.get_block(int(skin_ref))
    if skin_block is None:
        return []

    bone_refs = skin_block.get_field("Bones") or []
    names: list[str] = []
    for ref in bone_refs:
        bone_id = int(ref) if isinstance(ref, (int, float)) else -1
        if bone_id >= 0:
            bone_block = nif.get_block(bone_id)
            if bone_block:
                name = bone_block.get_field("Name") or f"Bone_{bone_id}"
                if isinstance(name, int):
                    names.append(f"Bone_{bone_id}")
                else:
                    names.append(str(name))
            else:
                names.append(f"Bone_{len(names)}")
        else:
            names.append(f"Bone_{len(names)}")
    return names


def _build_global_to_local_map(
    global_bones: list[str],
    local_bones: list[str],
) -> dict[int, int]:
    """Map global bone indices to local (shape) bone indices."""
    local_lookup = {name: idx for idx, name in enumerate(local_bones)}
    return {
        gi: local_lookup[name]
        for gi, name in enumerate(global_bones)
        if name in local_lookup
    }


def _find_new_bones_for_shape(
    skin_data: SkinData,
    vertex_offset: int,
    n_verts: int,
    shape_bone_names: list[str],
    global_to_local: dict[int, int],
) -> list[str]:
    """Find global bones used by this shape's vertices but not in its bone list."""
    shape_bone_set = set(shape_bone_names)
    needed: set[str] = set()

    for vi in range(vertex_offset, vertex_offset + n_verts):
        if vi >= skin_data.weights.shape[0]:
            break
        for j in range(skin_data.weights.shape[1]):
            w = skin_data.weights[vi, j]
            if w > 0:
                gi = int(skin_data.bone_indices[vi, j])
                if 0 <= gi < len(skin_data.bone_names):
                    bname = skin_data.bone_names[gi]
                    if bname not in shape_bone_set:
                        needed.add(bname)

    # Return in deterministic order (sorted)
    return sorted(needed)


def _add_bones_to_skin(
    nif, shape,
    shape_bone_names: list[str],
    new_bones: list[str],
    skin_data: SkinData,
) -> tuple[list[str], dict[int, int]]:
    """Add new bones to the shape's BSSkin::Instance and BSSkin::BoneData.

    Returns updated (shape_bone_names, global_to_local).
    """
    skin_ref = shape.get_field("Skin Instance") or shape.get_field("Skin")
    skin_block = nif.get_block(int(skin_ref))
    if skin_block is None:
        return shape_bone_names, _build_global_to_local_map(
            skin_data.bone_names, shape_bone_names
        )

    bone_refs = list(skin_block.get_field("Bones") or [])

    # Find bone node block IDs by name
    bone_name_to_block: dict[str, int] = {}
    for block in nif.blocks:
        name = block.get_field("Name")
        if name and isinstance(name, str):
            bone_name_to_block[name] = block.block_id

    # Also get bone data block for adding inverse bind entries
    data_ref = skin_block.get_field("Data")
    data_block = nif.get_block(int(data_ref)) if data_ref is not None and int(data_ref) >= 0 else None
    bone_list = list(data_block.get_field("Bone List") or []) if data_block else []

    for bname in new_bones:
        block_id = bone_name_to_block.get(bname, -1)
        if block_id < 0:
            _log.warning("Cannot find bone node '%s' in NIF — skipping", bname)
            continue

        bone_refs.append(block_id)
        shape_bone_names.append(bname)

        # Add identity inverse bind transform for new bone
        if data_block:
            bone_entry = {
                "Rotation": {
                    "m11": 1.0, "m12": 0.0, "m13": 0.0,
                    "m21": 0.0, "m22": 1.0, "m23": 0.0,
                    "m31": 0.0, "m32": 0.0, "m33": 1.0,
                },
                "Translation": {"x": 0.0, "y": 0.0, "z": 0.0},
                "Scale": 1.0,
            }
            # Use actual inv_bind if available from skin_data
            gi = next(
                (i for i, n in enumerate(skin_data.bone_names) if n == bname),
                -1,
            )
            if gi >= 0 and gi < len(skin_data.inv_bind_transforms):
                mat = skin_data.inv_bind_transforms[gi]
                bone_entry = {
                    "Rotation": {
                        "m11": float(mat[0, 0]), "m12": float(mat[1, 0]), "m13": float(mat[2, 0]),
                        "m21": float(mat[0, 1]), "m22": float(mat[1, 1]), "m23": float(mat[2, 1]),
                        "m31": float(mat[0, 2]), "m32": float(mat[1, 2]), "m33": float(mat[2, 2]),
                    },
                    "Translation": {
                        "x": float(mat[0, 3]),
                        "y": float(mat[1, 3]),
                        "z": float(mat[2, 3]),
                    },
                    "Scale": 1.0,
                }
            bone_list.append(bone_entry)

    # Write back updated bone refs
    skin_block.set_field("Num Bones", len(bone_refs))
    skin_block.set_field("Bones", bone_refs)

    if data_block:
        data_block.set_field("Num Bones", len(bone_list))
        data_block.set_field("Bone List", bone_list)

    global_to_local = _build_global_to_local_map(
        skin_data.bone_names, shape_bone_names
    )
    return shape_bone_names, global_to_local


def _write_vertex_weights(
    vertex_data_list: list[dict],
    skin_data: SkinData,
    vertex_offset: int,
    n_verts: int,
    global_to_local: dict[int, int],
) -> None:
    """Update Bone Weights and Bone Indices in vertex data dicts."""
    max_b = skin_data.weights.shape[1]

    for i in range(n_verts):
        gi = vertex_offset + i  # global vertex index in skin_data
        if gi >= skin_data.weights.shape[0]:
            break

        vd = vertex_data_list[i]

        # Collect (local_bone_idx, weight) pairs, sorted by weight desc
        pairs: list[tuple[int, float]] = []
        for j in range(max_b):
            w = float(skin_data.weights[gi, j])
            if w > 0:
                gbi = int(skin_data.bone_indices[gi, j])
                lbi = global_to_local.get(gbi, 0)
                pairs.append((lbi, w))

        # Sort by weight descending, keep top 4
        pairs.sort(key=lambda x: -x[1])
        pairs = pairs[:4]

        # Normalize weights to sum to 1.0
        total = sum(w for _, w in pairs)
        if total > 0:
            pairs = [(bi, w / total) for bi, w in pairs]

        # Pad to 4 slots
        while len(pairs) < 4:
            pairs.append((0, 0.0))

        # Detect format: combined dicts vs separate arrays
        bw_existing = vd.get("Bone Weights") or vd.get("BoneWeights")
        if isinstance(bw_existing, list) and bw_existing and isinstance(bw_existing[0], dict):
            # Combined format: [{"index": N, "weight": F}, ...]
            key_idx = "index" if "index" in bw_existing[0] else "Index"
            key_wt = "weight" if "weight" in bw_existing[0] else "Weight"
            new_bw = []
            for bi, w in pairs:
                new_bw.append({key_idx: bi, key_wt: w})
            # Use the original key name
            if "BoneWeights" in vd:
                vd["BoneWeights"] = new_bw
            else:
                vd["Bone Weights"] = new_bw
        else:
            # Separate arrays format
            new_weights = [w for _, w in pairs]
            new_indices = [bi for bi, _ in pairs]
            if "BoneWeights" in vd:
                vd["BoneWeights"] = new_weights
            else:
                vd["Bone Weights"] = new_weights
            vd["Bone Indices"] = new_indices


def _ensure_segments_synced(nif, shapes, skin_data: SkinData) -> None:
    """Rebuild FO4 segment hierarchy from segment_ids if segments are missing or stale.

    If any shape is a BSSubIndexTriShape and skin_data has segment_ids but
    no segments list, rebuild the hierarchy so export can write it.
    Also auto-generates SSF path if empty.
    """
    has_fo4_shapes = any(
        nif.schema.is_subtype_of(s.type_name, "BSSubIndexTriShape")
        for s in shapes
    )
    if not has_fo4_shapes:
        return

    has_assigned = skin_data.num_triangles > 0 and np.any(skin_data.segment_ids >= 0)
    if has_assigned and skin_data.segments:
        from .partitions import sync_fo4_segments_from_ids
        skin_data.segments = sync_fo4_segments_from_ids(skin_data)
        _log.info("Synced %d FO4 segments from segment_ids", len(skin_data.segments))
    elif has_assigned:
        from .partitions import rebuild_fo4_segments_from_body_parts
        skin_data.segments, skin_data.segment_ids = rebuild_fo4_segments_from_body_parts(
            skin_data, skin_data.segment_ids,
        )
        _log.info("Rebuilt %d FO4 segments from body part IDs", len(skin_data.segments))


def _write_fo4_segments(
    shape,
    skin_data: SkinData,
    tri_offset: int,
    n_tris: int,
) -> None:
    """Write FO4 BSSubIndexTriShape segment hierarchy from SkinData.segments.

    Rebuilds the Segment and Segment Data fields on the shape block.
    """
    if not skin_data.segments:
        return

    # Filter segments relevant to this shape's triangle range
    shape_segments: list[SegmentInfo] = []
    for seg in skin_data.segments:
        # Check if segment overlaps with this shape's triangle range
        seg_end = seg.start_index + seg.num_primitives
        shape_end = tri_offset + n_tris
        if seg.start_index < shape_end and seg_end > tri_offset:
            shape_segments.append(seg)

    if not shape_segments:
        return

    # Rebuild the Segment field (list of segment dicts)
    segment_list = []
    per_segment_data = []

    for seg in shape_segments:
        # Convert start_index from global to local (shape-relative)
        local_start = (seg.start_index - tri_offset) * 3  # convert to index units

        seg_dict: dict = {
            "Start Index": max(0, local_start),
            "Num Primitives": seg.num_primitives,
        }

        # Sub-segments
        sub_list = []
        if seg.sub_segments:
            # Parent segment entry first
            per_segment_data.append({
                "User Index": seg.user_index,
                "Bone ID": 0xFFFFFFFF,
                "Cut Offsets": [],
                "Num Cut Offsets": 0,
            })
            for ss in seg.sub_segments:
                local_ss_start = (ss.start_index - tri_offset) * 3
                sub_list.append({
                    "Start Index": max(0, local_ss_start),
                    "Num Primitives": ss.num_primitives,
                })
                per_segment_data.append({
                    "User Index": ss.user_index,
                    "Bone ID": ss.bone_id,
                    "Cut Offsets": list(ss.cut_offsets) if ss.cut_offsets else [],
                    "Num Cut Offsets": len(ss.cut_offsets) if ss.cut_offsets else 0,
                })
        else:
            # Segment without sub-segments gets one entry
            per_segment_data.append({
                "User Index": seg.user_index,
                "Bone ID": 0xFFFFFFFF,
                "Cut Offsets": [],
                "Num Cut Offsets": 0,
            })

        seg_dict["Num Sub Segments"] = len(sub_list)
        seg_dict["Sub Segment"] = sub_list
        segment_list.append(seg_dict)

    shape.set_field("Num Segments", len(segment_list))
    shape.set_field("Segment", segment_list)

    # Rebuild Segment Data
    seg_data_dict: dict = {
        "Num Segments": len(per_segment_data),
        "Per Segment Data": per_segment_data,
        "SSF File": skin_data.ssf_file or "",
    }
    shape.set_field("Segment Data", seg_data_dict)


def _write_dismember_partitions(
    nif, shape,
    skin_data: SkinData,
    tri_offset: int,
    n_tris: int,
) -> None:
    """Write Skyrim BSDismemberSkinInstance partition assignments.

    Updates the per-triangle partition IDs in the NiSkinPartition block
    and the body part list in BSDismemberSkinInstance.
    """
    skin_ref = shape.get_field("Skin Instance") or shape.get_field("Skin")
    if skin_ref is None or (isinstance(skin_ref, int) and skin_ref < 0):
        return

    skin_block = nif.get_block(int(skin_ref))
    if skin_block is None:
        return

    if not nif.schema.is_subtype_of(skin_block.type_name, "BSDismemberSkinInstance"):
        return

    # Get the shape's local partition IDs
    local_parts = skin_data.segment_ids[tri_offset:tri_offset + n_tris]

    # Find unique partition IDs used
    unique_parts = sorted(set(int(p) for p in local_parts if p >= 0))
    if not unique_parts:
        return

    # Update the body part list on BSDismemberSkinInstance
    bp_list = []
    for part_id in unique_parts:
        bp_list.append({
            "Part Flag": 1,  # PF_EDITOR_VISIBLE
            "Body Part": part_id,
        })
    skin_block.set_field("Num Partitions", len(bp_list))
    skin_block.set_field("Partitions", bp_list)

    # Update NiSkinPartition if present
    sp_ref = skin_block.get_field("Skin Partition")
    if sp_ref is None or (isinstance(sp_ref, int) and sp_ref < 0):
        return

    sp_block = nif.get_block(int(sp_ref))
    if sp_block is None:
        return

    # Rebuild NiSkinPartition partitions from the triangle assignments.
    # Group triangles by partition ID.
    triangles_list = shape.get_field("Triangles") or []

    part_id_to_idx = {pid: idx for idx, pid in enumerate(unique_parts)}

    # Group triangle indices by partition
    part_tris: dict[int, list[int]] = {pid: [] for pid in unique_parts}
    for ti in range(n_tris):
        pid = int(local_parts[ti])
        if pid >= 0 and pid in part_tris:
            part_tris[pid].append(ti)

    # Build new partition entries
    new_partitions = []
    for pid in unique_parts:
        tri_indices = part_tris[pid]
        if not tri_indices:
            continue

        # Collect vertices used by these triangles
        vert_set: set[int] = set()
        tris_for_part = []
        for ti in tri_indices:
            if ti < len(triangles_list):
                tri = triangles_list[ti]
                if isinstance(tri, dict):
                    v0 = int(tri.get("v1", tri.get("V1", 0)))
                    v1 = int(tri.get("v2", tri.get("V2", 0)))
                    v2 = int(tri.get("v3", tri.get("V3", 0)))
                elif isinstance(tri, (list, tuple)) and len(tri) >= 3:
                    v0, v1, v2 = int(tri[0]), int(tri[1]), int(tri[2])
                else:
                    continue
                vert_set.update((v0, v1, v2))
                tris_for_part.append((v0, v1, v2))

        # Build vertex map (sorted global indices -> local 0..N)
        vert_list = sorted(vert_set)
        vert_map_dict = {v: li for li, v in enumerate(vert_list)}

        # Remap triangles to local indices
        local_tris = []
        for v0, v1, v2 in tris_for_part:
            local_tris.append({
                "v1": vert_map_dict[v0],
                "v2": vert_map_dict[v1],
                "v3": vert_map_dict[v2],
            })

        new_partitions.append({
            "Num Vertices": len(vert_list),
            "Num Triangles": len(local_tris),
            "Vertex Map": vert_list,
            "Triangles": local_tris,
        })

    sp_block.set_field("Num Partitions", len(new_partitions))
    sp_block.set_field("Partitions", new_partitions)
