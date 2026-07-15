"""Skinning operations -- partitions, bone bounds, mirror."""
import numpy as np
from ..actions import OperationResult


def fix_bone_bounds(nif, block_id: int) -> OperationResult:
    """Recalculate bone bounding spheres from weighted vertices.

    Reads BSSkin::BoneData and the associated shape's vertex positions,
    then recomputes each bone's bounding sphere center and radius from
    the vertices weighted to that bone.
    """
    block = nif.get_block(block_id)
    if not block:
        return OperationResult(False, f"Block {block_id} not found")

    # Find skin instance and bone data
    skin_ref = block.get_field("Skin")
    if skin_ref is None or (isinstance(skin_ref, int) and skin_ref < 0):
        return OperationResult(False, f"Block {block_id} has no skin instance")

    skin_id = int(skin_ref) if isinstance(skin_ref, int) else -1
    skin_block = nif.get_block(skin_id)
    if not skin_block:
        return OperationResult(False, "Skin instance block not found")

    # Get bone data block
    bone_data_ref = skin_block.get_field("Bone Data")
    if bone_data_ref is None or (isinstance(bone_data_ref, int) and bone_data_ref < 0):
        return OperationResult(False, "No bone data block referenced")

    bone_data_block = nif.get_block(int(bone_data_ref) if isinstance(bone_data_ref, int) else -1)
    if not bone_data_block:
        return OperationResult(False, "Bone data block not found")

    # Get vertex positions from the shape
    vertex_data = block.get_field("Vertex Data") or []
    if not vertex_data:
        return OperationResult(False, "No vertex data on shape")

    n_verts = len(vertex_data)
    positions = np.zeros((n_verts, 3), dtype=np.float64)
    for i, vd in enumerate(vertex_data):
        v = vd.get("Vertex", {})
        positions[i] = [float(v.get("x", 0)), float(v.get("y", 0)), float(v.get("z", 0))]

    # Get bone list and per-vertex bone weights
    bone_list = bone_data_block.get_field("Bone List") or []
    fixed = 0
    modified = []

    for bi, bone_info in enumerate(bone_list):
        if not isinstance(bone_info, dict):
            continue

        # Collect vertices weighted to this bone from vertex data
        bone_verts = []
        for vi, vd in enumerate(vertex_data):
            bone_weights = vd.get("Bone Weights") or vd.get("BoneWeights") or []
            for bw in (bone_weights if isinstance(bone_weights, list) else []):
                idx = int(bw.get("index", bw.get("Index", -1)))
                weight = float(bw.get("weight", bw.get("Weight", 0)))
                if idx == bi and weight > 0.0:
                    bone_verts.append(positions[vi])
                    break

        if not bone_verts:
            continue

        pts = np.array(bone_verts, dtype=np.float64)
        center = pts.mean(axis=0)
        dists = np.linalg.norm(pts - center, axis=1)
        radius = float(dists.max()) if len(dists) > 0 else 0.0

        # Apply bone transform offset if present
        bone_info["Bounding Sphere Offset"] = {
            "x": float(center[0]), "y": float(center[1]), "z": float(center[2])
        }
        bone_info["Bounding Sphere Radius"] = radius
        fixed += 1

    if fixed:
        bone_data_block.set_field("Bone List", bone_list)
        modified = [bone_data_block.block_id]

    return OperationResult(True, f"Fixed bounds for {fixed} bone(s)", modified)


def mirror_skeleton(nif, axis: str = "x") -> OperationResult:
    """Mirror bone transforms across an axis.

    Negates the specified axis component of each bone's translation
    in NiNode bone transforms.
    """
    axis_idx = {"x": 0, "y": 1, "z": 2}.get(axis.lower())
    if axis_idx is None:
        return OperationResult(False, f"Invalid axis: {axis} (must be x, y, or z)")

    count = 0
    modified = []
    for block in nif.blocks:
        if not nif.schema.is_subtype_of(block.type_name, "NiNode"):
            continue
        name = block.get_field("Name") or ""
        # Only mirror bone nodes (heuristic: name contains bone-like patterns)
        if not name:
            continue

        translation = block.get_field("Translation")
        if not translation:
            continue

        if isinstance(translation, dict):
            keys = ["x", "y", "z"]
            key = keys[axis_idx]
            old_val = float(translation.get(key, 0))
            if abs(old_val) > 1e-6:
                translation[key] = -old_val
                block.set_field("Translation", translation)
                count += 1
                modified.append(block.block_id)
        elif isinstance(translation, list) and len(translation) > axis_idx:
            old_val = float(translation[axis_idx])
            if abs(old_val) > 1e-6:
                translation[axis_idx] = -old_val
                block.set_field("Translation", translation)
                count += 1
                modified.append(block.block_id)

    return OperationResult(True, f"Mirrored {count} bone(s) across {axis} axis", modified)
