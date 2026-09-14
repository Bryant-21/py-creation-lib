"""Promote plain BSTriShape meshes to skinned meshes.

`creation_lib.skinning` edits weights only on shapes that already have
`BSSkin::Instance` + `BSSkin::BoneData`. This module creates the bone node, skin
instance, and bone data, enables skinning in the vertex descriptor, and adds bone
weight/index fields to each vertex.

Typical call order for the weight painter's "promote to skinned" flow:

    from creation_lib.nif.operations.skinning import (
        add_bone_node, make_shape_skinned, set_rigid_weights,
    )

    root_bone = add_bone_node(nif, "Flag_Root", translation=(0, 52, 5))
    make_shape_skinned(nif, shape_id=6, bone_ids=[root_bone])
    set_rigid_weights(nif, shape_id=6, bone_local_index=0)
    nif.save("test_rigid.nif")
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Sequence

import numpy as np

from creation_lib.nif.nif_file import NifFile, NifBlock


# BSVertexDesc layout (uint64):
#   bits  0- 3  Vertex Data Size  (stride in dwords)
#   bits  4- 7  Dynamic Vertex Size
#   bits  8-11  UV1 Offset        (dword offset within the vertex record)
#   bits 12-15  UV2 Offset
#   bits 16-19  Normal Offset
#   bits 20-23  Tangent Offset
#   bits 24-27  Color Offset
#   bits 28-31  Skinning Data Offset
#   bits 32-35  Landscape Data Offset
#   bits 36-39  Eye Data Offset
#   bits 44-55  Vertex Attribute flags (VF_*)
#
# The VF_* flags double as the `ARG` bitfield that BSVertexData field
# conditions test (e.g. `(ARG & 0x40) != 0` gates Bone Weights/Indices).
VF_VERTEX = 0x0001
VF_UVS = 0x0002
VF_UVS_2 = 0x0004
VF_NORMALS = 0x0008
VF_TANGENTS = 0x0010
VF_VERTEX_COLORS = 0x0020
VF_SKINNED = 0x0040
VF_LAND_DATA = 0x0080
VF_EYE_DATA = 0x0100
VF_FULL_PRECISION = 0x0400

# Skinning data adds: 4 hfloat weights (8B) + 4 byte indices (4B) = 12B = 3 dwords.
SKIN_DWORDS = 3


@dataclass
class BoneSpec:
    """A bone reference for skin creation.

    block_id:   NifBlock id of an existing NiNode (use add_bone_node to create)
    inv_bind:   4x4 inverse bind transform as a numpy array. Identity by default.
    """
    block_id: int
    inv_bind: np.ndarray = field(
        default_factory=lambda: np.eye(4, dtype=np.float32)
    )


def add_bone_node(
    nif: NifFile,
    name: str,
    *,
    translation: tuple[float, float, float] = (0.0, 0.0, 0.0),
    rotation: np.ndarray | None = None,
    scale: float = 1.0,
    parent_id: int = 0,
) -> int:
    """Create a NiNode to serve as a bone and link it under a parent NiNode.

    Returns the new block id.
    """
    if rotation is None:
        rotation = np.eye(3, dtype=np.float32)
    if rotation.shape != (3, 3):
        raise ValueError(f"rotation must be 3x3, got {rotation.shape}")

    node = nif.add_block("NiNode")
    node.set_field("Name", name)
    node.set_field("Translation", {
        "x": float(translation[0]),
        "y": float(translation[1]),
        "z": float(translation[2]),
    })
    node.set_field("Rotation", _mat3_to_dict(rotation))
    node.set_field("Scale", float(scale))
    node.set_field("Flags", 14)  # standard Bethesda default

    parent = nif.get_block(parent_id)
    if parent is None:
        raise ValueError(f"parent block {parent_id} not found")
    children = list(parent.get_field("Children") or [])
    children.append(node.block_id)
    parent.set_field("Children", children)
    parent.set_field("Num Children", len(children))

    return node.block_id


def make_shape_skinned(
    nif: NifFile,
    shape_id: int,
    bone_ids: Sequence[int],
    *,
    skeleton_root_id: int = 0,
    inv_bind_transforms: Sequence[np.ndarray] | None = None,
) -> int:
    """Promote a plain BSTriShape into a skinned shape; return the new BSSkin::Instance id.

    Creates and links `BSSkin::BoneData` + `BSSkin::Instance`, sets VF_Skinned in
    the vertex descriptor, and zero-fills each vertex's `Bone Weights` / `Bone
    Indices` for `set_rigid_weights` or the painter export to fill. `bone_ids` (at
    least one) are NiNodes in shape-local index order. `inv_bind_transforms`
    default to identity, which is correct when the bones sit at the mesh origin.
    """
    if not bone_ids:
        raise ValueError("make_shape_skinned requires at least one bone")

    shape = nif.get_block(shape_id)
    if shape is None:
        raise ValueError(f"shape block {shape_id} not found")
    if not nif.schema.is_subtype_of(shape.type_name, "BSTriShape"):
        raise ValueError(
            f"block {shape_id} is {shape.type_name}, expected BSTriShape"
        )

    existing_skin = shape.get_field("Skin")
    if existing_skin is not None and int(existing_skin) >= 0:
        raise ValueError(
            f"shape {shape_id} already has a skin (block {existing_skin}); "
            f"this function only promotes unskinned shapes"
        )

    n_bones = len(bone_ids)
    if inv_bind_transforms is None:
        inv_bind_transforms = [np.eye(4, dtype=np.float32)] * n_bones
    if len(inv_bind_transforms) != n_bones:
        raise ValueError(
            f"inv_bind_transforms length {len(inv_bind_transforms)} "
            f"!= bone count {n_bones}"
        )

    # 1. BSSkin::BoneData — per-bone bounding sphere + inverse bind transform.
    bone_list = [
        _bone_data_entry(inv_bind_transforms[i]) for i in range(n_bones)
    ]
    bone_data = nif.add_block("BSSkin::BoneData")
    bone_data.set_field("Num Bones", n_bones)
    bone_data.set_field("Bone List", bone_list)

    # 2. BSSkin::Instance — links skeleton root, bone refs, bone data.
    skin_instance = nif.add_block("BSSkin::Instance")
    skin_instance.set_field("Skeleton Root", int(skeleton_root_id))
    skin_instance.set_field("Data", bone_data.block_id)
    skin_instance.set_field("Num Bones", n_bones)
    skin_instance.set_field("Bones", [int(b) for b in bone_ids])
    skin_instance.set_field("Num Scales", 0)
    skin_instance.set_field("Scales", [])

    # 3. Update the shape: Skin ref + vertex descriptor + vertex data.
    shape.set_field("Skin", skin_instance.block_id)

    desc = int(shape.get_field("Vertex Desc") or 0)
    new_desc = _vertex_desc_enable_skinning(desc)
    shape.set_field("Vertex Desc", new_desc)

    # FO4 shaders require the SLSF1_Skinned flag on the shape's shader
    # property, otherwise the renderer routes the shape through the
    # non-skinned pipeline and the mesh goes invisible in-game.
    _mark_shader_skinned(nif, shape)

    vertex_data = list(shape.get_field("Vertex Data") or [])
    for vd in vertex_data:
        vd.setdefault("Bone Weights", [0.0, 0.0, 0.0, 0.0])
        vd.setdefault("Bone Indices", [0, 0, 0, 0])
    shape.set_field("Vertex Data", vertex_data)

    # 4. Recalculate Data Size from the new stride.
    num_verts = int(shape.get_field("Num Vertices") or 0)
    num_tris = int(shape.get_field("Num Triangles") or 0)
    stride_dwords = new_desc & 0xF
    shape.set_field(
        "Data Size", (stride_dwords * num_verts * 4) + (num_tris * 6)
    )

    return skin_instance.block_id


def set_rigid_weights(
    nif: NifFile,
    shape_id: int,
    bone_local_index: int = 0,
) -> None:
    """Weight every vertex 1.0 to a single bone (rigid/smoke-test skinning).

    `bone_local_index` is the shape-local bone index, i.e. the position in
    the BSSkin::Instance `Bones` array — NOT a NiNode block id.
    """
    shape = nif.get_block(shape_id)
    if shape is None:
        raise ValueError(f"shape block {shape_id} not found")

    vertex_data = list(shape.get_field("Vertex Data") or [])
    for vd in vertex_data:
        vd["Bone Weights"] = [1.0, 0.0, 0.0, 0.0]
        vd["Bone Indices"] = [int(bone_local_index), 0, 0, 0]
    shape.set_field("Vertex Data", vertex_data)


def convert_to_sub_index_tri_shape(
    nif: NifFile,
    shape_id: int,
) -> None:
    """Promote a BSTriShape to BSSubIndexTriShape with a single full-mesh segment.

    BSSubIndexTriShape inherits from BSTriShape and adds segment / material
    partition data. FO4 cloth-simulated meshes (hair, bathrobe skirts) use
    BSSubIndexTriShape, not plain BSTriShape — the cloth deformer runtime
    walks the segment array. Converts by retyping the block, appending the
    minimal segment fields (one segment covering all triangles), and
    rebuilding the header block-type table.
    """
    shape = nif.get_block(shape_id)
    if shape is None:
        raise ValueError(f"shape block {shape_id} not found")
    if shape.type_name == "BSSubIndexTriShape":
        return
    if not nif.schema.is_subtype_of(shape.type_name, "BSTriShape"):
        raise ValueError(
            f"block {shape_id} is {shape.type_name}, expected BSTriShape"
        )

    num_tris = int(shape.get_field("Num Triangles") or 0)
    data_size = int(shape.get_field("Data Size") or 0)

    shape.type_name = "BSSubIndexTriShape"

    if data_size > 0:
        shape.set_field("Num Primitives", num_tris)
        shape.set_field("Num Segments", 1)
        shape.set_field("Total Segments", 1)
        shape.set_field("Segment", [
            {
                "Start Index": 0,
                "Num Primitives": num_tris,
                "Parent Array Index": 0xFFFFFFFF,
                "Num Sub Segments": 0,
                "Sub Segment": [],
            }
        ])

    nif._rebuild_header()


def set_vertex_weights(
    nif: NifFile,
    shape_id: int,
    weights: np.ndarray,
    bone_indices: np.ndarray,
) -> None:
    """Write per-vertex bone weights and local indices into the shape.

    Args:
        weights:       shape (N, 4) float32, rows should sum to ~1.0.
        bone_indices:  shape (N, 4) int, values are shape-local bone indices.
    """
    if weights.shape != bone_indices.shape:
        raise ValueError(
            f"weights {weights.shape} and bone_indices {bone_indices.shape} "
            f"must match"
        )
    if weights.ndim != 2 or weights.shape[1] != 4:
        raise ValueError(f"weights must be (N, 4), got {weights.shape}")

    shape = nif.get_block(shape_id)
    if shape is None:
        raise ValueError(f"shape block {shape_id} not found")

    vertex_data = list(shape.get_field("Vertex Data") or [])
    n = min(len(vertex_data), weights.shape[0])
    for i in range(n):
        vertex_data[i]["Bone Weights"] = [float(w) for w in weights[i]]
        vertex_data[i]["Bone Indices"] = [int(b) for b in bone_indices[i]]
    shape.set_field("Vertex Data", vertex_data)


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------

#: Bit 1 of Fallout4ShaderPropertyFlags1 (and the Skyrim equivalent).
#: Required for the runtime to route a shape through the skinned vertex
#: shader pipeline — without it the mesh renders as rigid (or invisible).
_SHADER_FLAG1_SKINNED = 0x02


def _mark_shader_skinned(nif: NifFile, shape: NifBlock) -> None:
    """Set the Skinned bit on the shape's shader property's Flags 1.

    ``Shader Flags 1`` is stored as a uint bitfield. OR in 0x02 (bit 1,
    ``Skinned``) so the FO4 renderer routes the shape through the skinned
    shader pipeline; without it the mesh goes invisible in-game.
    """
    shader_ref = shape.get_field("Shader Property")
    if shader_ref is None or int(shader_ref) < 0:
        return
    shader = nif.get_block(int(shader_ref))
    if shader is None:
        return
    if shader.type_name not in (
        "BSLightingShaderProperty",
        "BSEffectShaderProperty",
    ):
        return

    flags = int(shader.get_field("Shader Flags 1") or 0)
    if not (flags & _SHADER_FLAG1_SKINNED):
        shader.set_field("Shader Flags 1", flags | _SHADER_FLAG1_SKINNED)


def _vertex_desc_enable_skinning(desc: int) -> int:
    """Return a new Vertex Desc with VF_Skinned set and skin fields sized.

    Assumes the shape does not already have skinning in its descriptor.
    Appends the 3 skinning dwords at the end of the current vertex record.
    """
    stride = desc & 0xF
    skin_offset = stride  # append at end
    new_stride = stride + SKIN_DWORDS

    # Clear and rewrite affected fields.
    desc &= ~0xF                       # Vertex Data Size
    desc &= ~(0xF << 28)               # Skinning Data Offset
    desc |= (new_stride & 0xF)
    desc |= (skin_offset & 0xF) << 28
    desc |= VF_SKINNED << 44           # attribute flag bit
    return desc


def _bone_data_entry(inv_bind: np.ndarray) -> dict:
    """Build a single BSSkin::BoneData `Bone List` entry from a 4x4 matrix."""
    if inv_bind.shape != (4, 4):
        raise ValueError(f"inv_bind must be 4x4, got {inv_bind.shape}")
    rot = inv_bind[:3, :3].astype(np.float32)
    trans = inv_bind[:3, 3].astype(np.float32)
    return {
        "Bounding Sphere": {
            "Center": {"x": 0.0, "y": 0.0, "z": 0.0},
            "Radius": 0.0,
        },
        "Rotation": _mat3_to_dict(rot),
        "Translation": {
            "x": float(trans[0]),
            "y": float(trans[1]),
            "z": float(trans[2]),
        },
        "Scale": 1.0,
    }


def _mat3_to_dict(m: np.ndarray) -> dict:
    """Serialize a 3x3 rotation matrix to the NIF's row/column dict format.

    NIF rotation fields use `mRC` keying where R is the row, C the column.
    """
    return {
        "m11": float(m[0, 0]), "m12": float(m[0, 1]), "m13": float(m[0, 2]),
        "m21": float(m[1, 0]), "m22": float(m[1, 1]), "m23": float(m[1, 2]),
        "m31": float(m[2, 0]), "m32": float(m[2, 1]), "m33": float(m[2, 2]),
    }
