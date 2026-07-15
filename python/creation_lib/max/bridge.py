from __future__ import annotations

import base64
import hashlib
import io
import json
import math
import re
import tempfile
from copy import deepcopy
from dataclasses import fields as dc_fields
from pathlib import Path
from typing import Any

import numpy as np

from creation_lib.core.game_profiles import detect_game, get_profile
from creation_lib.max.cloth import extract_cloth_document, pack_cloth_document
from creation_lib.material_tools.base import BaseHeader
from creation_lib.material_tools.bgem_bin import BGEMData, BGEM_SIGNATURE, read_bgem
from creation_lib.material_tools.bgsm_bin import BGSMData, BGSM_SIGNATURE, read_bgsm
from creation_lib.nif.operations.collision import (
    DEFAULT_RADIUS,
    HAVOK_SCALE,
    generate_collision_from_geometry,
    get_collision_layers,
)
from creation_lib.geometry.preview_meshes import merge_preview_meshes
from creation_lib.havok.collision_preview import extract_preview_meshes_from_blob
from creation_lib.nif.operations.collision_mesh import (
    extract_compressed_mesh,
    extract_mopp_geometry,
)
from creation_lib.nif.nif_file import NifBlock, NifFile
from creation_lib.nif.operations.skinning import convert_to_sub_index_tri_shape
from creation_lib.skinning.nif_export import (
    _write_dismember_partitions,
    _write_fo4_segments,
)
from creation_lib.skinning.normalization import normalize_weights
from creation_lib.skinning.partitions import rebuild_fo4_segments
from creation_lib.skinning.reference_body import _extract_dismember_partitions
from creation_lib.skinning.skin_data import SegmentInfo, SkinData, SubSegmentInfo

FORMAT_VERSION = 1
DEFAULT_SCENE_GAME = "fo4"
NODE_TYPES = ("BSFadeNode", "NiNode")
SHAPE_TYPES = (
    "NiTriShape",
    "NiTriStrips",
    "BSLODTriShape",
    "BSTriShape",
    "BSMeshLODTriShape",
    "BSSubIndexTriShape",
    "BSDynamicTriShape",
)
MAX_SKIN_INFLUENCES = 4
ADDON_NODE_RE = re.compile(r"^AddOnNode(\d+)$", re.IGNORECASE)
COLLISION_EXTRA_NAME = "MB21_Collision"
ATTACHED_COLLISION_EXTRA_NAME = "MB21_AttachedCollision"
BS_VERTEX_ATTR_VERTEX = 0x1
BS_VERTEX_ATTR_UV = 0x2
BS_VERTEX_ATTR_NORMAL = 0x8
BS_VERTEX_ATTR_TANGENT = 0x10
BS_VERTEX_ATTR_COLOR = 0x20
BS_VERTEX_ATTR_FULL_PRECISION = 0x400
SHADER_FLAG1_SPECULAR = 1 << 0
SHADER_FLAG1_VERTEX_ALPHA = 1 << 3
SHADER_FLAG1_CAST_SHADOWS = 1 << 9
SHADER_FLAG1_OWN_EMIT = 1 << 22
SHADER_FLAG1_ZBUFFER_TEST = 1 << 31
SHADER_FLAG2_ZBUFFER_WRITE = 1 << 0
SHADER_FLAG2_VERTEX_COLORS = 1 << 5
LIGHTING_SHADER_DEFAULT_FLAGS1 = (
    SHADER_FLAG1_SPECULAR
    | SHADER_FLAG1_CAST_SHADOWS
    | SHADER_FLAG1_OWN_EMIT
    | SHADER_FLAG1_ZBUFFER_TEST
)
LIGHTING_SHADER_DEFAULT_FLAGS2 = SHADER_FLAG2_ZBUFFER_WRITE
# Sentinel used by Bethesda's exporters to mean "rim lighting disabled" on
# BSLightingShaderProperty.Rimlight Power. The schema's `Backlight Power` field
# is gated on Rimlight Power == FLT_MAX, but FO4's binary loader reads
# Backlight Power unconditionally — writing any other Rimlight value here
# leaves the block 4 bytes short and shifts every subsequent shader/material
# field, producing a corrupt texture-set pointer on load. Every vanilla FO4
# static-mesh NIF carries Rimlight=FLT_MAX, Backlight=0.0 for this reason.
FLT_MAX = 3.4028234663852886e38
PREVIEW_TEXTURE_FIELDS = {
    "bgsm": ("DiffuseTexture", "NormalTexture", "GlowTexture"),
    "bgem": ("BaseTexture", "NormalTexture", "GlowTexture"),
}


def import_nif_to_scene_document(path: str) -> dict[str, Any]:
    nif_path = Path(path)
    nif = NifFile.load(str(nif_path))
    game = _detect_nif_game(nif)

    raw_bytes = nif_path.read_bytes()
    supported_ids: set[int] = set()
    materials: dict[str, dict[str, Any]] = {}
    root_nodes: list[dict[str, Any]] = []

    for root_id in _find_root_block_ids(nif):
        node_doc = _build_node_document(
            nif=nif,
            block_id=root_id,
            nif_path=nif_path,
            game=game,
            supported_ids=supported_ids,
            materials=materials,
        )
        if node_doc is not None:
            root_nodes.append(node_doc)

    root_metadata = _extract_root_metadata(nif, supported_ids)
    cloth_metadata = extract_cloth_document(nif, raw_bytes, supported_ids)
    if cloth_metadata is not None:
        root_metadata["cloth"] = cloth_metadata
    unsupported_blocks = [
        _block_summary(block)
        for block in nif.blocks
        if block.block_id not in supported_ids
    ]

    return {
        "format_version": FORMAT_VERSION,
        "game": game,
        "source_path": str(nif_path),
        "root_nodes": root_nodes,
        "materials": list(materials.values()),
        "metadata": {"root": root_metadata},
        "warnings": _build_import_warnings(unsupported_blocks),
        "opaque_payload": {
            "sha256": hashlib.sha256(raw_bytes).hexdigest(),
            "original_nif_base64": base64.b64encode(raw_bytes).decode("ascii"),
            "unsupported_blocks": unsupported_blocks,
        },
    }


def export_scene_document_to_nif(
    document: dict[str, Any],
    output_path: str,
) -> dict[str, Any]:
    game = _normalize_scene_game(document.get("game"))
    profile = get_profile(game)

    output = Path(output_path)
    output.parent.mkdir(parents=True, exist_ok=True)

    warnings: list[str] = []
    nif = _load_or_create_export_nif(document, game=game)
    _promote_root_for_animated_scene(nif, document)
    animation_export_state = _snapshot_animation_export_state(nif)
    root_nodes = list(document.get("root_nodes") or [])
    if not root_nodes:
        raise ValueError("Scene document has no root_nodes")

    materials_by_id = {
        str(material.get("id")): material
        for material in document.get("materials") or []
        if material.get("id")
    }

    scene_root = nif.get_block(0)
    if scene_root is None:
        raise ValueError("Export NIF is missing block 0 root")

    export_settings = dict(document.get("export_settings") or {})
    if _uses_synthetic_scene_root(
        root_nodes, export_settings
    ) or _uses_existing_scene_root(root_nodes, export_settings):
        _apply_node_document(
            nif=nif,
            node_doc=root_nodes[0],
            owner_parent=None,
            materials_by_id=materials_by_id,
            output_path=output,
            warnings=warnings,
            profile=profile,
            forced_block=scene_root,
        )
    else:
        # Scene root is a separate wrapper above the user's root nodes —
        # give it a meaningful name (file stem). Bethesda tooling and the
        # Creation Kit expect a non-empty root name.
        if not str(scene_root.get_field("Name") or ""):
            scene_root.set_field("Name", output.stem)
        for root_doc in root_nodes:
            _apply_root_document(
                nif=nif,
                scene_root=scene_root,
                node_doc=root_doc,
                materials_by_id=materials_by_id,
                output_path=output,
                warnings=warnings,
                profile=profile,
            )

    _apply_root_metadata(
        nif=nif,
        root=scene_root,
        metadata=(document.get("metadata") or {}).get("root") or {},
    )
    _ensure_bsx_havok_if_collision(nif, scene_root)
    _retarget_scene_animations(
        nif=nif,
        document=document,
        warnings=warnings,
        export_settings=export_settings,
        export_state=animation_export_state,
    )
    _remove_attached_collision_extra_blocks(nif)
    temp_output: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            prefix=f"{output.name}.",
            suffix=".tmp",
            dir=output.parent,
            delete=False,
        ) as handle:
            temp_output = Path(handle.name)

        nif.save(str(temp_output))
        cloth_metadata = ((document.get("metadata") or {}).get("root") or {}).get(
            "cloth"
        )
        if cloth_metadata:
            temp_output.write_bytes(
                pack_cloth_document(temp_output.read_bytes(), cloth_metadata)
            )
        temp_output.replace(output)
    finally:
        if temp_output is not None:
            try:
                temp_output.unlink()
            except FileNotFoundError:
                pass

    return {
        "format_version": FORMAT_VERSION,
        "game": game,
        "output_path": str(output),
        "warnings": warnings,
        "num_blocks": len(nif.blocks),
    }


def read_material_document(path: str) -> dict[str, Any]:
    material_path = Path(path)
    material_type = _detect_material_type(material_path)
    with material_path.open("rb") as handle:
        if material_type == "bgsm":
            data = read_bgsm(handle)
        else:
            data = read_bgem(handle)
    fields = _flatten_material_data(data)

    return {
        "format_version": FORMAT_VERSION,
        "material_type": material_type,
        "source_path": str(material_path),
        "version": int(data.header.version),
        "fields": fields,
        "preview_paths": _resolve_material_preview_paths(
            material_type,
            fields,
            anchors=[material_path],
        ),
    }


def write_material_document(
    document: dict[str, Any],
    output_path: str | None = None,
) -> dict[str, Any]:
    material_type = str(document.get("material_type") or "").lower()
    if material_type not in {"bgsm", "bgem"}:
        raise ValueError("material_type must be 'bgsm' or 'bgem'")

    destination = Path(
        output_path or document.get("output_path") or document.get("source_path") or ""
    )
    if not destination:
        raise ValueError("No output path supplied for material write")
    destination.parent.mkdir(parents=True, exist_ok=True)

    fields = dict(document.get("fields") or {})
    version = int(document.get("version") or 2)
    if material_type == "bgsm":
        material = _unflatten_bgsm(fields, version)
    else:
        material = _unflatten_bgem(fields, version)

    with destination.open("wb") as handle:
        material.write(handle)

    return {
        "format_version": FORMAT_VERSION,
        "material_type": material_type,
        "output_path": str(destination),
        "version": version,
    }


def _normalize_scene_game(value: Any) -> str:
    game = str(value or DEFAULT_SCENE_GAME).strip().lower()
    try:
        get_profile(game)
    except KeyError as exc:
        raise ValueError(f"Unsupported scene game '{value}'") from exc
    return game


def _detect_nif_game(nif: NifFile) -> str:
    bs_version = int(getattr(nif.header, "bs_version", 0) or 0)
    profile = detect_game(bs_version)
    if profile is None:
        raise ValueError(f"Unsupported NIF bs_version={bs_version}")
    return profile.id


def _find_root_block_ids(nif: NifFile) -> list[int]:
    root = nif.get_block(0)
    if root is not None and nif.schema.is_subtype_of(root.type_name, "NiNode"):
        return [0]

    referenced: set[int] = set()
    for block in nif.blocks:
        children = block.get_field("Children") or []
        for child_ref in children:
            child_id = _ref_id(child_ref)
            if child_id >= 0:
                referenced.add(child_id)

    roots = [
        block.block_id
        for block in nif.blocks
        if block.block_id not in referenced
        and nif.schema.is_subtype_of(block.type_name, "NiNode")
    ]
    return roots or ([0] if nif.blocks else [])


def _build_node_document(
    nif: NifFile,
    block_id: int,
    nif_path: Path,
    game: str,
    supported_ids: set[int],
    materials: dict[str, dict[str, Any]],
) -> dict[str, Any] | None:
    block = nif.get_block(block_id)
    if block is None:
        return None

    is_node = nif.schema.is_subtype_of(block.type_name, "NiNode")
    is_shape = any(
        nif.schema.is_subtype_of(block.type_name, base) for base in SHAPE_TYPES
    )
    if not is_node and not is_shape:
        return None

    supported_ids.add(block_id)
    collision_metadata, collision_mesh, collision_node_type = (
        _extract_collision_metadata(nif, block, supported_ids, game=game)
    )
    node_type = collision_node_type or ("mesh" if is_shape else "node")
    document: dict[str, Any] = {
        "id": f"node-{block.block_id}",
        "type": node_type,
        "nif_type": block.type_name,
        "source_block_id": block.block_id,
        "name": block.get_field("Name") or "",
        "transform": _extract_transform(block),
        "metadata": _extract_node_metadata(nif, block, supported_ids),
        "children": [],
    }

    if block.type_name == "BSValueNode":
        addon_index = _addon_index_for_block(block)
        if addon_index is not None:
            document["metadata"]["addon_node_index"] = addon_index

    if is_shape:
        document["mesh"] = _extract_mesh(nif, block, supported_ids)
        alpha_property = _extract_alpha_property(nif, block, supported_ids)
        if alpha_property is not None:
            document["mesh"]["alpha_property"] = alpha_property
        skin_info = _extract_skin_info(nif, block, supported_ids)
        if skin_info is not None:
            document["mesh"]["skin"] = skin_info
        partition_info = _extract_partition_info(nif, block, supported_ids)
        if partition_info is not None:
            document["mesh"]["partitions"] = partition_info

        material_binding = _extract_material_binding(
            nif=nif,
            nif_path=nif_path,
            game=game,
            shape=block,
            supported_ids=supported_ids,
            materials=materials,
        )
        if material_binding is not None:
            document["material_binding"] = material_binding
    elif collision_mesh is not None:
        document["mesh"] = collision_mesh

    if collision_metadata is not None:
        document["metadata"]["collision"] = collision_metadata

    for child_ref in block.get_field("Children") or []:
        child_id = _ref_id(child_ref)
        child_doc = _build_node_document(
            nif=nif,
            block_id=child_id,
            nif_path=nif_path,
            game=game,
            supported_ids=supported_ids,
            materials=materials,
        )
        if child_doc is not None:
            document["children"].append(child_doc)

    document["children"].extend(
        _extract_attached_collision_children(nif, block, supported_ids, game=game)
    )
    helper_children = _extract_connect_point_helpers(nif, block, supported_ids)
    document["children"].extend(helper_children)

    return document


def _extract_transform(block: NifBlock) -> dict[str, Any]:
    translation = block.get_field("Translation") or {"x": 0.0, "y": 0.0, "z": 0.0}
    rotation = block.get_field("Rotation") or [[1, 0, 0], [0, 1, 0], [0, 0, 1]]
    scale = block.get_field("Scale")
    return {
        "translation": _vector3(translation),
        "rotation": _matrix33(rotation),
        "scale": float(1.0 if scale in (None, "") else scale),
    }


def _extract_mesh(
    nif: NifFile,
    block: NifBlock,
    supported_ids: set[int],
) -> dict[str, Any]:
    if _is_legacy_tri_based_shape(nif, block):
        return _extract_legacy_tri_based_mesh(nif, block, supported_ids)
    return _extract_bs_tri_shape_mesh(block)


def _extract_bs_tri_shape_mesh(block: NifBlock) -> dict[str, Any]:
    source_vertices: list[dict[str, float]] = []
    normals: list[dict[str, float]] = []
    uvs: list[dict[str, float]] = []
    colors: list[dict[str, float]] = []
    has_colors = False
    for entry in block.get_field("Vertex Data") or []:
        source_vertices.append(_vector3(entry.get("Vertex") or {}))
        normals.append(_vector3(entry.get("Normal") or {}))
        uv = entry.get("UV") or {}
        uvs.append({"u": float(uv.get("u", 0.0)), "v": float(uv.get("v", 0.0))})
        color = entry.get("Vertex Colors")
        if color not in (None, ""):
            has_colors = True
        colors.append(_color4(color))

    source_triangles: list[dict[str, int]] = []
    for tri in block.get_field("Triangles") or []:
        source_triangles.append(
            {
                "v1": int(tri.get("v1", 0)),
                "v2": int(tri.get("v2", 0)),
                "v3": int(tri.get("v3", 0)),
            }
        )

    welded_vertices, weld_map = _weld_vertices(source_vertices)
    welded_triangles = [
        {
            "v1": weld_map[tri["v1"]],
            "v2": weld_map[tri["v2"]],
            "v3": weld_map[tri["v3"]],
        }
        for tri in source_triangles
    ]

    mesh = {
        "vertices": welded_vertices,
        "normals": normals,
        "uvs": uvs,
        "triangles": welded_triangles,
        "uv_triangles": source_triangles,
        "is_skinned": _is_skinned(block),
        "preserve_source_geometry": True,
    }
    if has_colors:
        mesh["colors"] = colors
    return mesh


def _extract_legacy_tri_based_mesh(
    nif: NifFile,
    block: NifBlock,
    supported_ids: set[int],
) -> dict[str, Any]:
    data_id = _ref_id(block.get_field("Data"))
    data = nif.get_block(data_id)
    if data is None:
        return {
            "vertices": [],
            "normals": [],
            "uvs": [],
            "triangles": [],
            "is_skinned": _is_skinned(block),
            "preserve_source_geometry": True,
        }
    supported_ids.add(data.block_id)

    vertices = [_vector3(vertex) for vertex in (data.get_field("Vertices") or [])]
    normals = (
        [_vector3(normal) for normal in (data.get_field("Normals") or [])]
        if data.get_field("Has Normals")
        else []
    )
    uv_sets = list(data.get_field("UV Sets") or [])
    uvs = (
        [_uv(uv) for uv in (uv_sets[0] if uv_sets else [])]
        if data.get_field("Has UV")
        else []
    )
    colors = (
        [_color4(color) for color in (data.get_field("Vertex Colors") or [])]
        if data.get_field("Has Vertex Colors")
        else []
    )
    if data.type_name == "NiTriStripsData":
        triangles = _triangles_from_strips(
            strip_lengths=[
                int(value or 0) for value in (data.get_field("Strip Lengths") or [])
            ],
            points=[int(value or 0) for value in (data.get_field("Points") or [])],
        )
    else:
        triangles = _build_triangles(data.get_field("Triangles") or [])

    mesh = {
        "vertices": vertices,
        "normals": normals,
        "uvs": uvs,
        "triangles": triangles,
        "uv_triangles": triangles,
        "is_skinned": _is_skinned(block),
        "preserve_source_geometry": True,
    }
    if colors:
        mesh["colors"] = colors
    return mesh


def _triangles_from_strips(
    *, strip_lengths: list[int], points: list[int]
) -> list[dict[str, int]]:
    triangles: list[dict[str, int]] = []
    offset = 0
    for strip_length in strip_lengths:
        strip = points[offset : offset + strip_length]
        offset += strip_length
        for index in range(max(0, len(strip) - 2)):
            a, b, c = strip[index], strip[index + 1], strip[index + 2]
            if a < 0 or b < 0 or c < 0 or a == b or b == c or a == c:
                continue
            if index % 2:
                triangles.append({"v1": b, "v2": a, "v3": c})
            else:
                triangles.append({"v1": a, "v2": b, "v3": c})
    return triangles


def _weld_vertices(
    vertices: list[dict[str, float]],
) -> tuple[list[dict[str, float]], list[int]]:
    welded: list[dict[str, float]] = []
    index_by_key: dict[tuple[float, float, float], int] = {}
    weld_map: list[int] = []
    for vertex in vertices:
        key = (
            round(float(vertex.get("x", 0.0)), 6),
            round(float(vertex.get("y", 0.0)), 6),
            round(float(vertex.get("z", 0.0)), 6),
        )
        mapped = index_by_key.get(key)
        if mapped is None:
            mapped = len(welded)
            index_by_key[key] = mapped
            welded.append(dict(vertex))
        weld_map.append(mapped)
    return welded, weld_map


def _extract_skin_info(
    nif: NifFile,
    block: NifBlock,
    supported_ids: set[int],
) -> dict[str, Any] | None:
    skin_id = _ref_id(block.get_field("Skin Instance"))
    if skin_id < 0:
        skin_id = _ref_id(block.get_field("Skin"))
    if skin_id < 0:
        return None

    skin_block = nif.get_block(skin_id)
    if skin_block is None:
        return None

    supported_ids.add(skin_id)
    bone_refs = list(skin_block.get_field("Bones") or [])
    bone_names: list[str] = []
    bone_ids: list[int] = []
    for bone_ref in bone_refs:
        bone_id = _ref_id(bone_ref)
        if bone_id < 0:
            continue
        supported_ids.add(bone_id)
        bone_ids.append(bone_id)
        bone_block = nif.get_block(bone_id)
        bone_names.append((bone_block.get_field("Name") if bone_block else "") or "")

    bone_data_id = _ref_id(skin_block.get_field("Bone Data"))
    if bone_data_id >= 0:
        supported_ids.add(bone_data_id)

    vertex_weights: list[list[dict[str, Any]]] = []
    vertex_data = block.get_field("Vertex Data") or []
    for entry in vertex_data:
        influences = _extract_vertex_influences(
            nif=nif,
            skin_block=skin_block,
            vertex_entry=entry or {},
            bone_names=bone_names,
            bone_ids=bone_ids,
        )
        if influences:
            vertex_weights.append(influences)
        else:
            vertex_weights.append([])

    bone_data = nif.get_block(bone_data_id) if bone_data_id >= 0 else None
    inv_bind_transforms = _extract_inv_bind_transforms(bone_data)

    return {
        "enabled": True,
        "source_block_id": skin_id,
        "source_instance_type": skin_block.type_name,
        "bone_names": bone_names,
        "bone_ids": bone_ids,
        "weights": vertex_weights,
        "inv_bind_transforms": inv_bind_transforms,
        "vertex_count": int(block.get_field("Num Vertices") or len(vertex_data) or 0),
        "triangle_count": int(
            block.get_field("Num Triangles") or len(block.get_field("Triangles") or [])
            or 0
        ),
    }


def _extract_partition_info(
    nif: NifFile,
    block: NifBlock,
    supported_ids: set[int],
) -> dict[str, Any] | None:
    if not nif.schema.is_subtype_of(block.type_name, "BSTriShape"):
        return None

    shape_type = block.type_name
    if shape_type == "BSSubIndexTriShape":
        segment_data = deepcopy(block.get_field("Segment Data") or {})
        segments = _normalize_fo4_segments(block.get_field("Segment") or [])
        if not segments and not segment_data:
            return None
        triangle_ids = _triangle_partition_ids_from_segments(
            triangles=block.get_field("Triangles") or [],
            segments=segments,
        )
        return {
            "enabled": True,
            "source_block_id": block.block_id,
            "source_shape_type": shape_type,
            "segment_file": str((segment_data or {}).get("SSF File") or ""),
            "segments": segments,
            "segment_data": segment_data,
            "segment_ids": triangle_ids,
        }

    skin_id = _ref_id(block.get_field("Skin Instance"))
    if skin_id < 0:
        skin_id = _ref_id(block.get_field("Skin"))
    if skin_id < 0:
        return None

    skin_block = nif.get_block(skin_id)
    if skin_block is None:
        return None

    if skin_block.type_name != "BSDismemberSkinInstance":
        return None

    supported_ids.add(skin_id)
    partition_entries = [
        {
            "part_flag": int(entry.get("Part Flag", 0)),
            "body_part": int(entry.get("Body Part", 0)),
        }
        for entry in (skin_block.get_field("Partitions") or [])
        if isinstance(entry, dict)
    ]

    partition_ids = _extract_dismember_partition_ids(nif, block)
    skin_partition_id = _ref_id(skin_block.get_field("Skin Partition"))
    if skin_partition_id >= 0:
        supported_ids.add(skin_partition_id)

    return {
        "enabled": True,
        "source_block_id": skin_id,
        "source_shape_type": shape_type,
        "source_instance_type": skin_block.type_name,
        "partitions": partition_entries,
        "segment_ids": partition_ids.tolist() if hasattr(partition_ids, "tolist") else [],
    }


def _extract_vertex_influences(
    *,
    nif: NifFile,
    skin_block: NifBlock,
    vertex_entry: dict[str, Any],
    bone_names: list[str],
    bone_ids: list[int],
) -> list[dict[str, Any]]:
    weights_raw = vertex_entry.get("Bone Weights") or vertex_entry.get("BoneWeights") or []
    indices_raw = vertex_entry.get("Bone Indices") or []
    influences: list[dict[str, Any]] = []

    if isinstance(weights_raw, list) and weights_raw and isinstance(weights_raw[0], dict):
        for entry in weights_raw[:MAX_SKIN_INFLUENCES]:
            bone_index = int(entry.get("index", entry.get("Index", 0)))
            weight = float(entry.get("weight", entry.get("Weight", 0.0)))
            if weight <= 0.0:
                continue
            influences.append(
                _skin_influence_doc(
                    bone_index=bone_index,
                    weight=weight,
                    bone_names=bone_names,
                    bone_ids=bone_ids,
                )
            )
        return _sort_influences(influences)

    if not isinstance(weights_raw, list):
        return []

    for slot, weight_value in enumerate(weights_raw[:MAX_SKIN_INFLUENCES]):
        weight = float(weight_value or 0.0)
        if weight <= 0.0:
            continue
        bone_index = int(indices_raw[slot]) if slot < len(indices_raw) else 0
        influences.append(
            _skin_influence_doc(
                bone_index=bone_index,
                weight=weight,
                bone_names=bone_names,
                bone_ids=bone_ids,
            )
        )
    return _sort_influences(influences)


def _skin_influence_doc(
    *,
    bone_index: int,
    weight: float,
    bone_names: list[str],
    bone_ids: list[int],
) -> dict[str, Any]:
    bone_name = bone_names[bone_index] if 0 <= bone_index < len(bone_names) else ""
    bone_id = bone_ids[bone_index] if 0 <= bone_index < len(bone_ids) else -1
    return {
        "bone_index": bone_index,
        "bone_name": bone_name,
        "bone_id": bone_id,
        "weight": float(weight),
    }


def _sort_influences(influences: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return sorted(
        influences,
        key=lambda entry: (
            -float(entry.get("weight") or 0.0),
            str(entry.get("bone_name") or ""),
            int(entry.get("bone_index") or 0),
            int(entry.get("bone_id") or 0),
        ),
    )


def _extract_inv_bind_transforms(bone_data: NifBlock | None) -> list[list[list[float]]]:
    if bone_data is None:
        return []

    matrices: list[list[list[float]]] = []
    for entry in bone_data.get_field("Bone List") or []:
        if not isinstance(entry, dict):
            continue
        rotation = entry.get("Rotation") or {}
        translation = entry.get("Translation") or {}
        matrices.append(
            [
                [
                    float(rotation.get("m11", 1.0)),
                    float(rotation.get("m12", 0.0)),
                    float(rotation.get("m13", 0.0)),
                    0.0,
                ],
                [
                    float(rotation.get("m21", 0.0)),
                    float(rotation.get("m22", 1.0)),
                    float(rotation.get("m23", 0.0)),
                    0.0,
                ],
                [
                    float(rotation.get("m31", 0.0)),
                    float(rotation.get("m32", 0.0)),
                    float(rotation.get("m33", 1.0)),
                    0.0,
                ],
                [
                    float(translation.get("x", 0.0)),
                    float(translation.get("y", 0.0)),
                    float(translation.get("z", 0.0)),
                    1.0,
                ],
            ]
        )
    return matrices


def _normalize_fo4_segments(segment_list: list[dict[str, Any]]) -> list[dict[str, Any]]:
    normalized: list[dict[str, Any]] = []
    for entry in segment_list:
        if not isinstance(entry, dict):
            continue
        sub_segments: list[dict[str, Any]] = []
        for sub_entry in entry.get("Sub Segment") or []:
            if not isinstance(sub_entry, dict):
                continue
            sub_segments.append(
                {
                    "start_index": int(sub_entry.get("Start Index", 0)) // 3,
                    "num_primitives": int(sub_entry.get("Num Primitives", 0)),
                    "user_index": int(sub_entry.get("User Index", 0)),
                    "bone_id": int(sub_entry.get("Bone ID", 0xFFFFFFFF)),
                    "cut_offsets": [
                        float(value)
                        for value in (sub_entry.get("Cut Offsets") or [])
                    ],
                }
            )
        normalized.append(
            {
                "start_index": int(entry.get("Start Index", 0)) // 3,
                "num_primitives": int(entry.get("Num Primitives", 0)),
                "user_index": int(entry.get("User Index", 0)),
                "sub_segments": sub_segments,
            }
        )
    return normalized


def _triangle_partition_ids_from_segments(
    *,
    triangles: list[dict[str, Any]],
    segments: list[dict[str, Any]],
) -> list[int]:
    partition_ids = [-1] * len(triangles)
    for segment in segments:
        start_index = max(0, int(segment.get("start_index") or 0))
        num_primitives = max(0, int(segment.get("num_primitives") or 0))
        user_index = int(segment.get("user_index") or 0)
        for tri_index in range(start_index, min(start_index + num_primitives, len(partition_ids))):
            partition_ids[tri_index] = user_index
        for sub_segment in segment.get("sub_segments") or []:
            if not isinstance(sub_segment, dict):
                continue
            sub_start = max(0, int(sub_segment.get("start_index") or 0))
            sub_count = max(0, int(sub_segment.get("num_primitives") or 0))
            sub_user = int(sub_segment.get("user_index") or user_index)
            for tri_index in range(sub_start, min(sub_start + sub_count, len(partition_ids))):
                partition_ids[tri_index] = sub_user
    return partition_ids


def _extract_dismember_partition_ids(
    nif: NifFile,
    block: NifBlock,
) -> list[int]:
    partition_array = _extract_dismember_partitions(
        nif,
        block,
        len(block.get_field("Triangles") or []),
    )
    if partition_array is None:
        return []
    return [int(value) for value in partition_array.tolist()]


def _extract_string_extra_data(
    nif: NifFile,
    owner: NifBlock,
    supported_ids: set[int],
) -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []
    for extra_block in _iter_extra_blocks(nif, owner):
        if extra_block.type_name != "NiStringExtraData":
            continue
        if (extra_block.get_field("Name") or "") in {
            COLLISION_EXTRA_NAME,
            ATTACHED_COLLISION_EXTRA_NAME,
        }:
            continue
        supported_ids.add(extra_block.block_id)
        entries.append(
            {
                "source_block_id": extra_block.block_id,
                "name": extra_block.get_field("Name") or "",
                "value": extra_block.get_field("String Data") or "",
            }
        )
    return entries


def _extract_node_metadata(
    nif: NifFile,
    owner: NifBlock,
    supported_ids: set[int],
) -> dict[str, Any]:
    metadata: dict[str, Any] = {
        "extra_string_data": _extract_string_extra_data(nif, owner, supported_ids)
    }
    flags = owner.get_field("Flags")
    if flags not in (None, ""):
        metadata["node_flags"] = int(flags)
    behavior_graphs: list[dict[str, Any]] = []
    for extra_block in _iter_extra_blocks(nif, owner):
        if extra_block.type_name == "BSXFlags":
            supported_ids.add(extra_block.block_id)
            metadata["bsx_flags"] = int(extra_block.get_field("Integer Data") or 0)
        elif extra_block.type_name == "BSBehaviorGraphExtraData":
            supported_ids.add(extra_block.block_id)
            behavior_graphs.append(
                {
                    "source_block_id": extra_block.block_id,
                    "path": extra_block.get_field("Behaviour Graph File") or "",
                    "controls_base_skeleton": bool(
                        extra_block.get_field("Controls Base Skeleton")
                    ),
                }
            )
    if behavior_graphs:
        metadata["behavior_graphs"] = behavior_graphs
    return metadata


def _extract_collision_metadata(
    nif: NifFile,
    owner: NifBlock,
    supported_ids: set[int],
    game: str,
) -> tuple[dict[str, Any] | None, dict[str, Any] | None, str | None]:
    for extra_block in _iter_extra_blocks(nif, owner):
        if extra_block.type_name != "NiStringExtraData":
            continue
        if (extra_block.get_field("Name") or "") != COLLISION_EXTRA_NAME:
            continue
        supported_ids.add(extra_block.block_id)
        try:
            payload = json.loads(extra_block.get_field("String Data") or "{}")
        except json.JSONDecodeError:
            return (
                {
                    "enabled": True,
                    "representation": "placeholder",
                    "source_block_id": extra_block.block_id,
                },
                None,
                None,
            )

        metadata = dict(payload.get("metadata") or {})
        metadata.setdefault("enabled", True)
        metadata.setdefault("representation", "placeholder")
        metadata["source_block_id"] = extra_block.block_id
        mesh = payload.get("mesh") if isinstance(payload.get("mesh"), dict) else None
        node_type = "collision" if payload.get("node_type") == "collision" else None
        return metadata, mesh, node_type

    collision_id = _ref_id(owner.get_field("Collision Object"))
    if collision_id < 0:
        return None, None, None

    supported_ids.add(collision_id)
    collision_block = nif.get_block(collision_id)
    data_id = (
        _ref_id(collision_block.get_field("Data"))
        if collision_block is not None
        and collision_block.type_name == "bhkNPCollisionObject"
        else -1
    )
    if data_id >= 0:
        supported_ids.add(data_id)
    body_id = (
        _ref_id(collision_block.get_field("Body"))
        if collision_block is not None
        else -1
    )
    if body_id >= 0:
        supported_ids.add(body_id)
    body_block = nif.get_block(body_id) if body_id >= 0 else None
    shape_id = _ref_id(body_block.get_field("Shape")) if body_block is not None else -1
    if shape_id >= 0:
        supported_ids.add(shape_id)

    return (
        {
            "enabled": True,
            "representation": "attached",
            "collision_object_block_id": collision_id,
            "data_block_id": data_id if data_id >= 0 else None,
            "body_block_id": body_id if body_id >= 0 else None,
            "shape_block_id": shape_id if shape_id >= 0 else None,
        },
        None,
        None,
    )


def _extract_attached_collision_children(
    nif: NifFile,
    owner: NifBlock,
    supported_ids: set[int],
    game: str,
) -> list[dict[str, Any]]:
    collision_id = _ref_id(owner.get_field("Collision Object"))
    if collision_id < 0:
        return []

    collision_block = nif.get_block(collision_id)
    if collision_block is None:
        return []

    supported_ids.add(collision_id)
    if collision_block.type_name == "bhkNPCollisionObject":
        mesh, info = _extract_np_collision_mesh_payload(
            nif, collision_block, supported_ids
        )
        if mesh is None:
            return []
        metadata = {
            "extra_string_data": [],
            "collision": {
                "enabled": True,
                "representation": "attached",
                "collision_object_block_id": collision_id,
                **info,
            },
        }
        return [
            {
                "id": f"collision-{owner.block_id}-{collision_id}",
                "type": "collision",
                "nif_type": "NiNode",
                "name": f"{str(owner.get_field('Name') or f'Node{owner.block_id}')}_Collision",
                "transform": {
                    "translation": {"x": 0.0, "y": 0.0, "z": 0.0},
                    "rotation": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                    "scale": 1.0,
                },
                "metadata": metadata,
                "mesh": mesh,
                "children": [],
            }
        ]

    body_id = _ref_id(collision_block.get_field("Body"))
    if body_id < 0:
        return []
    body_block = nif.get_block(body_id)
    if body_block is None:
        return []

    supported_ids.add(body_id)
    shape_id = _ref_id(body_block.get_field("Shape"))
    if shape_id < 0:
        return []

    mesh, info = _extract_collision_mesh_payload(nif, shape_id, supported_ids)
    if mesh is None:
        return []

    owner_name = str(owner.get_field("Name") or f"Node{owner.block_id}")
    actual_collision = {
        "enabled": True,
        "representation": "attached",
        "collision_object_block_id": collision_id,
        "body_block_id": body_id,
        "shape_block_id": shape_id,
        "layer": _layer_name_for_body(body_block, game=game),
        "mass": float(body_block.get_field("Mass") or 0.0),
        "friction": float(body_block.get_field("Friction") or 0.5),
        "restitution": float(body_block.get_field("Restitution") or 0.4),
        **info,
    }
    payload_nodes = _read_attached_collision_nodes(nif, owner, supported_ids)
    if payload_nodes:
        hydrated_nodes: list[dict[str, Any]] = []
        for index, payload_node in enumerate(payload_nodes):
            if not isinstance(payload_node, dict):
                continue
            payload_metadata = dict(payload_node.get("metadata") or {})
            payload_collision = dict(payload_metadata.get("collision") or {})
            payload_metadata["collision"] = {**payload_collision, **actual_collision}
            payload_metadata["extra_string_data"] = list(
                payload_metadata.get("extra_string_data") or []
            )
            hydrated_nodes.append(
                {
                    "id": payload_node.get("id")
                    or f"collision-{owner.block_id}-{shape_id}-{index}",
                    "type": "collision",
                    "nif_type": payload_node.get("nif_type") or "NiNode",
                    "name": payload_node.get("name") or f"{owner_name}_Collision",
                    "transform": payload_node.get("transform")
                    or {
                        "translation": {"x": 0.0, "y": 0.0, "z": 0.0},
                        "rotation": [
                            [1.0, 0.0, 0.0],
                            [0.0, 1.0, 0.0],
                            [0.0, 0.0, 1.0],
                        ],
                        "scale": 1.0,
                    },
                    "metadata": payload_metadata,
                    "mesh": payload_node.get("mesh") or mesh,
                    "children": [],
                }
            )
        if hydrated_nodes:
            return hydrated_nodes

    metadata = {"extra_string_data": [], "collision": actual_collision}
    return [
        {
            "id": f"collision-{owner.block_id}-{shape_id}",
            "type": "collision",
            "nif_type": "NiNode",
            "name": f"{owner_name}_Collision",
            "transform": {
                "translation": {"x": 0.0, "y": 0.0, "z": 0.0},
                "rotation": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                "scale": 1.0,
            },
            "metadata": metadata,
            "mesh": mesh,
            "children": [],
        }
    ]


def _extract_collision_mesh_payload(
    nif: NifFile,
    shape_id: int,
    supported_ids: set[int],
) -> tuple[dict[str, Any] | None, dict[str, Any]]:
    block = nif.get_block(shape_id)
    if block is None:
        return None, {}

    supported_ids.add(shape_id)
    type_name = block.type_name

    if type_name == "bhkConvexVerticesShape":
        vertices = [
            {
                "x": float(vertex.get("x", 0.0)) * HAVOK_SCALE,
                "y": float(vertex.get("y", 0.0)) * HAVOK_SCALE,
                "z": float(vertex.get("z", 0.0)) * HAVOK_SCALE,
            }
            for vertex in (block.get_field("Vertices") or [])
        ]
        triangles = _convex_hull_triangles(vertices)
        return (
            {"vertices": vertices, "triangles": triangles},
            {
                "shape_type": "convex_hull",
                "radius": float(block.get_field("Radius") or DEFAULT_RADIUS),
            },
        )

    if type_name == "bhkBoxShape":
        return _box_collision_mesh(block), {
            "shape_type": "box",
            "radius": float(block.get_field("Radius") or DEFAULT_RADIUS),
        }

    if type_name in {"bhkTransformShape", "bhkConvexTransformShape"}:
        child_id = _ref_id(block.get_field("Shape"))
        child_mesh, info = _extract_collision_mesh_payload(nif, child_id, supported_ids)
        if child_mesh is None:
            return None, {}
        return (
            _apply_havok_transform_to_mesh(
                child_mesh,
                block.get_field("Transform") or {},
            ),
            info,
        )

    if type_name == "bhkListShape":
        merged_vertices: list[dict[str, float]] = []
        merged_triangles: list[dict[str, int]] = []
        for sub_ref in block.get_field("Sub Shapes") or []:
            child_mesh, _ = _extract_collision_mesh_payload(
                nif, _ref_id(sub_ref), supported_ids
            )
            if child_mesh is None:
                continue
            offset = len(merged_vertices)
            merged_vertices.extend(child_mesh.get("vertices") or [])
            merged_triangles.extend(
                {
                    "v1": int(tri.get("v1", 0)) + offset,
                    "v2": int(tri.get("v2", 0)) + offset,
                    "v3": int(tri.get("v3", 0)) + offset,
                }
                for tri in (child_mesh.get("triangles") or [])
            )
        if not merged_vertices:
            return None, {}
        return (
            {"vertices": merged_vertices, "triangles": merged_triangles},
            {"shape_type": "list"},
        )

    if type_name == "bhkSphereShape":
        return _uv_sphere_mesh(
            radius=float(block.get_field("Radius") or 0.5) * HAVOK_SCALE
        ), {
            "shape_type": "sphere",
            "radius": float(block.get_field("Radius") or DEFAULT_RADIUS),
        }

    if type_name == "bhkCapsuleShape":
        radius = float(
            block.get_field("Radius")
            or block.get_field("Radius 1")
            or block.get_field("Radius 2")
            or 0.5
        )
        return _capsule_mesh(block, radius * HAVOK_SCALE), {
            "shape_type": "capsule",
            "radius": radius,
        }

    if type_name == "bhkCylinderShape":
        radius = float(
            block.get_field("Cylinder Radius") or block.get_field("Radius") or 0.5
        )
        return _cylinder_mesh(block, radius * HAVOK_SCALE), {
            "shape_type": "cylinder",
            "radius": float(block.get_field("Radius") or DEFAULT_RADIUS),
        }

    if type_name == "bhkMoppBvTreeShape":
        vertices, triangles, _ = extract_mopp_geometry(
            nif, shape_id, havok_scale_factor=HAVOK_SCALE
        )
        return (
            _mesh_from_arrays(vertices, triangles),
            {"shape_type": "mopp"},
        )

    if type_name == "bhkCompressedMeshShape":
        data_id = _ref_id(block.get_field("Data"))
        supported_ids.add(data_id)
        vertices, triangles, _ = extract_compressed_mesh(
            nif, data_id, havok_scale_factor=HAVOK_SCALE
        )
        return (
            _mesh_from_arrays(vertices, triangles),
            {
                "shape_type": "compressed_mesh",
                "radius": float(block.get_field("Radius") or DEFAULT_RADIUS),
            },
        )

    if type_name == "bhkCompressedMeshShapeData":
        vertices, triangles, _ = extract_compressed_mesh(
            nif, shape_id, havok_scale_factor=HAVOK_SCALE
        )
        return (
            _mesh_from_arrays(vertices, triangles),
            {"shape_type": "compressed_mesh"},
        )

    return None, {}


def _extract_np_collision_mesh_payload(
    nif: NifFile,
    collision_block: NifBlock,
    supported_ids: set[int],
) -> tuple[dict[str, Any] | None, dict[str, Any]]:
    data_id = _ref_id(collision_block.get_field("Data"))
    if data_id < 0:
        return None, {}
    supported_ids.add(data_id)
    data_block = nif.get_block(data_id)
    if data_block is None or data_block.type_name != "bhkPhysicsSystem":
        return None, {}
    binary_data = data_block.get_field("Binary Data") or {}
    raw = binary_data.get("Data") if isinstance(binary_data, dict) else None
    if not isinstance(raw, list) or not raw:
        return None, {}
    blob = bytes(raw)

    body_id = collision_block.get_field("Body ID")
    try:
        body_id_int = int(body_id) if body_id is not None else None
    except (TypeError, ValueError):
        body_id_int = None

    previews = extract_preview_meshes_from_blob(
        blob,
        havok_scale=HAVOK_SCALE,
        body_id=body_id_int,
    )
    if not previews:
        return None, {}

    mesh = merge_preview_meshes([preview["mesh"] for preview in previews])
    if mesh is None:
        return None, {}
    body_metadata = _np_collision_body_metadata(blob, body_id_int)
    if len(previews) == 1:
        info = {
            "data_block_id": data_id,
            "body_id": body_id_int,
            "shape_type": previews[0].get("shape_type") or "np_shape",
            **body_metadata,
        }
        if "radius" in previews[0]:
            info["radius"] = float(previews[0]["radius"])
        return mesh, info
    return mesh, {
        "data_block_id": data_id,
        "body_id": body_id_int,
        "shape_type": "list",
        **body_metadata,
    }


def _np_collision_body_metadata(blob: bytes, body_id: int | None) -> dict[str, Any]:
    if body_id is None:
        return {}
    try:
        from creation_lib._native import havok_native

        summary = json.loads(havok_native.havok_collision_summary(blob))
    except Exception:
        return {}
    for body in summary.get("bodies") or []:
        try:
            if int(body.get("body_id")) != int(body_id):
                continue
        except (TypeError, ValueError):
            continue
        metadata: dict[str, Any] = {}
        if body.get("material_crc") is not None:
            metadata["material"] = int(body["material_crc"])
        if body.get("layer") is not None:
            metadata["layer"] = _layer_name_for_value(int(body["layer"]))
        return metadata
    return {}


def _layer_name_for_value(layer_value: int, *, game: str = DEFAULT_SCENE_GAME) -> str:
    layers = get_collision_layers(get_profile(_normalize_scene_game(game)))
    for name, value in layers.items():
        if int(value) == int(layer_value):
            return name
    return "STATIC"


def _layer_name_for_body(
    body_block: NifBlock,
    *,
    game: str = DEFAULT_SCENE_GAME,
) -> str:
    filter_value = body_block.get_field("Havok Filter") or {}
    layer_value = None
    if isinstance(filter_value, dict):
        for key, value in filter_value.items():
            if "layer" in str(key).lower():
                layer_value = int(value)
                break
    return _layer_name_for_value(int(layer_value or -1), game=game)


def _convex_hull_triangles(vertices: list[dict[str, float]]) -> list[dict[str, int]]:
    if len(vertices) < 3:
        return []
    try:
        from creation_lib.scientific.native_runtime import convex_hull_triangles

        triangles = convex_hull_triangles(
            [[vertex["x"], vertex["y"], vertex["z"]] for vertex in vertices]
        )
        return [
            {"v1": int(face[0]), "v2": int(face[1]), "v3": int(face[2])}
            for face in triangles
        ]
    except Exception:
        return [
            {"v1": 0, "v2": index, "v3": index + 1}
            for index in range(1, len(vertices) - 1)
        ]


def _box_collision_mesh(block: NifBlock) -> dict[str, Any]:
    dimensions = block.get_field("Dimensions") or {}
    hx = float(dimensions.get("x", 0.5)) * HAVOK_SCALE
    hy = float(dimensions.get("y", 0.5)) * HAVOK_SCALE
    hz = float(dimensions.get("z", 0.5)) * HAVOK_SCALE
    vertices = [
        {"x": -hx, "y": -hy, "z": -hz},
        {"x": hx, "y": -hy, "z": -hz},
        {"x": hx, "y": hy, "z": -hz},
        {"x": -hx, "y": hy, "z": -hz},
        {"x": -hx, "y": -hy, "z": hz},
        {"x": hx, "y": -hy, "z": hz},
        {"x": hx, "y": hy, "z": hz},
        {"x": -hx, "y": hy, "z": hz},
    ]
    triangles = [
        {"v1": 0, "v2": 1, "v3": 2},
        {"v1": 0, "v2": 2, "v3": 3},
        {"v1": 4, "v2": 6, "v3": 5},
        {"v1": 4, "v2": 7, "v3": 6},
        {"v1": 0, "v2": 4, "v3": 5},
        {"v1": 0, "v2": 5, "v3": 1},
        {"v1": 1, "v2": 5, "v3": 6},
        {"v1": 1, "v2": 6, "v3": 2},
        {"v1": 2, "v2": 6, "v3": 7},
        {"v1": 2, "v2": 7, "v3": 3},
        {"v1": 3, "v2": 7, "v3": 4},
        {"v1": 3, "v2": 4, "v3": 0},
    ]
    return {"vertices": vertices, "triangles": triangles}


def _apply_havok_transform_to_mesh(
    mesh: dict[str, Any],
    transform: dict[str, Any],
) -> dict[str, Any]:
    m11 = float(transform.get("m11", 1.0))
    m12 = float(transform.get("m12", 0.0))
    m13 = float(transform.get("m13", 0.0))
    m14 = float(transform.get("m14", 0.0)) * HAVOK_SCALE
    m21 = float(transform.get("m21", 0.0))
    m22 = float(transform.get("m22", 1.0))
    m23 = float(transform.get("m23", 0.0))
    m24 = float(transform.get("m24", 0.0)) * HAVOK_SCALE
    m31 = float(transform.get("m31", 0.0))
    m32 = float(transform.get("m32", 0.0))
    m33 = float(transform.get("m33", 1.0))
    m34 = float(transform.get("m34", 0.0)) * HAVOK_SCALE
    vertices = []
    for vertex in mesh.get("vertices") or []:
        x = float(vertex.get("x", 0.0))
        y = float(vertex.get("y", 0.0))
        z = float(vertex.get("z", 0.0))
        vertices.append(
            {
                "x": m11 * x + m12 * y + m13 * z + m14,
                "y": m21 * x + m22 * y + m23 * z + m24,
                "z": m31 * x + m32 * y + m33 * z + m34,
            }
        )
    return {"vertices": vertices, "triangles": list(mesh.get("triangles") or [])}


def _uv_sphere_mesh(
    radius: float,
    segments: int = 12,
    rings: int = 8,
) -> dict[str, Any]:
    vertices: list[dict[str, float]] = []
    triangles: list[dict[str, int]] = []
    for ring in range(rings + 1):
        theta = math.pi * ring / rings
        sin_theta = math.sin(theta)
        cos_theta = math.cos(theta)
        for segment in range(segments):
            phi = (2.0 * math.pi * segment) / segments
            vertices.append(
                {
                    "x": radius * sin_theta * math.cos(phi),
                    "y": radius * sin_theta * math.sin(phi),
                    "z": radius * cos_theta,
                }
            )
    for ring in range(rings):
        for segment in range(segments):
            next_segment = (segment + 1) % segments
            current = ring * segments + segment
            next_row = (ring + 1) * segments + segment
            current_next = ring * segments + next_segment
            next_row_next = (ring + 1) * segments + next_segment
            triangles.append({"v1": current, "v2": next_row, "v3": current_next})
            triangles.append({"v1": current_next, "v2": next_row, "v3": next_row_next})
    return {"vertices": vertices, "triangles": triangles}


def _cylinder_mesh(
    block: NifBlock,
    radius_nif: float,
    segments: int = 12,
) -> dict[str, Any]:
    vertex_a = _vector3(block.get_field("Vertex A") or {})
    vertex_b = _vector3(block.get_field("Vertex B") or {})
    ax = vertex_a["x"] * HAVOK_SCALE
    ay = vertex_a["y"] * HAVOK_SCALE
    az = vertex_a["z"] * HAVOK_SCALE
    bx = vertex_b["x"] * HAVOK_SCALE
    by = vertex_b["y"] * HAVOK_SCALE
    bz = vertex_b["z"] * HAVOK_SCALE
    direction = [bx - ax, by - ay, bz - az]
    basis_u, basis_v = _orthonormal_basis(direction)

    vertices: list[dict[str, float]] = []
    triangles: list[dict[str, int]] = []
    for cap_index, center in enumerate(((ax, ay, az), (bx, by, bz))):
        cx, cy, cz = center
        for segment in range(segments):
            phi = (2.0 * math.pi * segment) / segments
            dx = math.cos(phi) * basis_u[0] + math.sin(phi) * basis_v[0]
            dy = math.cos(phi) * basis_u[1] + math.sin(phi) * basis_v[1]
            dz = math.cos(phi) * basis_u[2] + math.sin(phi) * basis_v[2]
            vertices.append(
                {
                    "x": cx + dx * radius_nif,
                    "y": cy + dy * radius_nif,
                    "z": cz + dz * radius_nif,
                }
            )
    vertices.append({"x": ax, "y": ay, "z": az})
    vertices.append({"x": bx, "y": by, "z": bz})
    bottom_center = len(vertices) - 2
    top_center = len(vertices) - 1
    for segment in range(segments):
        next_segment = (segment + 1) % segments
        bottom_a = segment
        bottom_b = next_segment
        top_a = segments + segment
        top_b = segments + next_segment
        triangles.append({"v1": bottom_a, "v2": top_a, "v3": bottom_b})
        triangles.append({"v1": bottom_b, "v2": top_a, "v3": top_b})
        triangles.append({"v1": bottom_center, "v2": bottom_b, "v3": bottom_a})
        triangles.append({"v1": top_center, "v2": top_a, "v3": top_b})
    return {"vertices": vertices, "triangles": triangles}


def _capsule_mesh(
    block: NifBlock,
    radius_nif: float,
    segments: int = 12,
    hemi_rings: int = 4,
) -> dict[str, Any]:
    first = _vector3(block.get_field("First Point") or {})
    second = _vector3(block.get_field("Second Point") or {})
    ax = first["x"] * HAVOK_SCALE
    ay = first["y"] * HAVOK_SCALE
    az = first["z"] * HAVOK_SCALE
    bx = second["x"] * HAVOK_SCALE
    by = second["y"] * HAVOK_SCALE
    bz = second["z"] * HAVOK_SCALE
    direction = [bx - ax, by - ay, bz - az]
    basis_u, basis_v = _orthonormal_basis(direction)
    axis = _normalize_vector(direction)

    vertices: list[dict[str, float]] = []
    for ring in range(hemi_rings + 1):
        angle = (math.pi / 2.0) * (ring / hemi_rings)
        radial = math.sin(angle) * radius_nif
        offset = math.cos(angle) * radius_nif
        for segment in range(segments):
            phi = (2.0 * math.pi * segment) / segments
            dx = math.cos(phi) * basis_u[0] + math.sin(phi) * basis_v[0]
            dy = math.cos(phi) * basis_u[1] + math.sin(phi) * basis_v[1]
            dz = math.cos(phi) * basis_u[2] + math.sin(phi) * basis_v[2]
            vertices.append(
                {
                    "x": ax - axis[0] * offset + dx * radial,
                    "y": ay - axis[1] * offset + dy * radial,
                    "z": az - axis[2] * offset + dz * radial,
                }
            )
            vertices.append(
                {
                    "x": bx + axis[0] * offset + dx * radial,
                    "y": by + axis[1] * offset + dy * radial,
                    "z": bz + axis[2] * offset + dz * radial,
                }
            )
    return {"vertices": vertices, "triangles": _convex_hull_triangles(vertices)}


def _orthonormal_basis(direction: list[float]) -> tuple[list[float], list[float]]:
    axis = _normalize_vector(direction)
    reference = [0.0, 0.0, 1.0] if abs(axis[2]) < 0.95 else [0.0, 1.0, 0.0]
    u = _normalize_vector(_cross(reference, axis))
    v = _normalize_vector(_cross(axis, u))
    return u, v


def _normalize_vector(vector: list[float]) -> list[float]:
    length = math.sqrt(sum(component * component for component in vector))
    if length <= 1e-8:
        return [1.0, 0.0, 0.0]
    return [component / length for component in vector]


def _cross(a: list[float], b: list[float]) -> list[float]:
    return [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]


def _mesh_from_arrays(vertices: Any, triangles: Any) -> dict[str, Any]:
    return {
        "vertices": [
            {"x": float(vertex[0]), "y": float(vertex[1]), "z": float(vertex[2])}
            for vertex in vertices
        ],
        "triangles": [
            {"v1": int(triangle[0]), "v2": int(triangle[1]), "v3": int(triangle[2])}
            for triangle in triangles
        ],
    }


def _extract_alpha_property(
    nif: NifFile,
    shape: NifBlock,
    supported_ids: set[int],
) -> dict[str, Any] | None:
    alpha_id = _ref_id(shape.get_field("Alpha Property"))
    if alpha_id < 0:
        return None
    alpha = nif.get_block(alpha_id)
    if alpha is None or alpha.type_name != "NiAlphaProperty":
        return None
    supported_ids.add(alpha_id)
    return {
        "enabled": True,
        "source_block_id": alpha_id,
        "flags": int(alpha.get_field("Flags") or 4844),
        "threshold": int(alpha.get_field("Threshold") or 128),
    }


def _extract_connect_point_helpers(
    nif: NifFile,
    owner: NifBlock,
    supported_ids: set[int],
) -> list[dict[str, Any]]:
    helpers: list[dict[str, Any]] = []
    for extra_block in _iter_extra_blocks(nif, owner):
        if extra_block.type_name == "BSConnectPoint::Parents":
            supported_ids.add(extra_block.block_id)
            for index, point in enumerate(
                extra_block.get_field("Connect Points") or []
            ):
                helpers.append(
                    {
                        "id": f"helper-parent-{extra_block.block_id}-{index}",
                        "type": "helper",
                        "helper_type": "connect_point_parent",
                        "name": point.get("Name") or "",
                        "source_block_id": extra_block.block_id,
                        "transform": {
                            "translation": _vector3(point.get("Translation") or {}),
                            "rotation": _quaternion(point.get("Rotation") or {}),
                            "scale": float(point.get("Scale", 1.0)),
                        },
                        "metadata": {
                            "parent_name": point.get("Parent")
                            or "WorkshopConnectPoints",
                        },
                    }
                )
        elif extra_block.type_name == "BSConnectPoint::Children":
            supported_ids.add(extra_block.block_id)
            for index, name in enumerate(extra_block.get_field("Point Name") or []):
                helpers.append(
                    {
                        "id": f"helper-child-{extra_block.block_id}-{index}",
                        "type": "helper",
                        "helper_type": "connect_point_child",
                        "name": str(name),
                        "source_block_id": extra_block.block_id,
                        "transform": {
                            "translation": {"x": 0.0, "y": 0.0, "z": 0.0},
                            "rotation": {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0},
                            "scale": 1.0,
                        },
                        "metadata": {
                            "skinned": bool(extra_block.get_field("Skinned")),
                        },
                    }
                )
    return helpers


def _extract_material_binding(
    nif: NifFile,
    nif_path: Path,
    game: str,
    shape: NifBlock,
    supported_ids: set[int],
    materials: dict[str, dict[str, Any]],
) -> dict[str, Any] | None:
    shader_id = _ref_id(shape.get_field("Shader Property"))
    if shader_id < 0:
        return None

    shader = nif.get_block(shader_id)
    if shader is None:
        return None

    supported_ids.add(shader_id)
    material_type = "bgem" if shader.type_name == "BSEffectShaderProperty" else "bgsm"
    material_id = f"material-{shader_id}"
    material_path = str(shader.get_field("Name") or "").strip()
    mode = "linked" if material_path.lower().endswith((".bgsm", ".bgem")) else "inline"
    if not _scene_supports_material_documents(game):
        mode = "inline"

    resolved_material_path = _resolve_material_path(material_path, nif_path)
    fields: dict[str, Any]
    version = 2
    source_path: str | None = None
    preview_paths: dict[str, str] = {}

    if (
        mode == "linked"
        and resolved_material_path is not None
        and resolved_material_path.exists()
    ):
        material_doc = read_material_document(str(resolved_material_path))
        fields = dict(material_doc.get("fields") or {})
        version = int(material_doc.get("version") or 2)
        source_path = str(resolved_material_path)
        preview_paths = dict(material_doc.get("preview_paths") or {})
    else:
        fields = _extract_inline_material_fields(
            nif, shader, material_type, supported_ids
        )
        preview_paths = _resolve_material_preview_paths(
            material_type,
            fields,
            anchors=[nif_path],
        )

    materials[material_id] = {
        "id": material_id,
        "material_type": material_type,
        "mode": mode,
        "path": material_path,
        "source_block_id": shader_id,
        "source_path": source_path,
        "version": version,
        "fields": fields,
        "preview_paths": preview_paths,
    }
    return {"material_id": material_id, "mode": mode, "path": material_path}


def _extract_inline_material_fields(
    nif: NifFile,
    shader: NifBlock,
    material_type: str,
    supported_ids: set[int],
) -> dict[str, Any]:
    if material_type == "bgsm":
        fields = _flatten_material_data(_default_bgsm_data())
        texture_set_id = _ref_id(shader.get_field("Texture Set"))
        if texture_set_id >= 0:
            supported_ids.add(texture_set_id)
            texture_set = nif.get_block(texture_set_id)
            textures = (
                list(texture_set.get_field("Textures") or []) if texture_set else []
            )
        else:
            textures = []

        fields.update(
            {
                "DiffuseTexture": _texture_slot(textures, 0),
                "NormalTexture": _texture_slot(textures, 1),
                "SmoothSpecTexture": _texture_slot(textures, 2),
                "GreyscaleTexture": _texture_slot(textures, 3),
                "EnvmapTexture": _texture_slot(textures, 4),
                "GlowTexture": _texture_slot(textures, 5),
                "InnerLayerTexture": _texture_slot(textures, 6),
                "SpecularTexture": _texture_slot(textures, 7),
                "SpecularColor": _color3_tuple(shader.get_field("Specular Color")),
                "SpecularMult": float(shader.get_field("Specular Strength") or 1.0),
                "Smoothness": float(shader.get_field("Smoothness") or 1.0),
                "FresnelPower": float(shader.get_field("Fresnel Power") or 5.0),
                "RootMaterialPath": shader.get_field("Root Material") or "",
                "EmittanceColor": _color3_tuple(shader.get_field("Emissive Color")),
                "EmittanceMult": float(shader.get_field("Emissive Multiple") or 1.0),
            }
        )
        return fields

    fields = _flatten_material_data(_default_bgem_data())
    base_color = shader.get_field("Base Color") or {}
    fields.update(
        {
            "BaseTexture": shader.get_field("Source Texture") or "",
            "GrayscaleTexture": shader.get_field("Greyscale Texture") or "",
            "EnvmapTexture": shader.get_field("Env Map Texture") or "",
            "NormalTexture": shader.get_field("Normal Texture") or "",
            "EnvmapMaskTexture": shader.get_field("Env Mask Texture") or "",
            "BaseColor": _color3_tuple(base_color),
            "BaseColorScale": float(shader.get_field("Base Color Scale") or 1.0),
            "FalloffStartAngle": float(shader.get_field("Falloff Start Angle") or 1.0),
            "FalloffStopAngle": float(shader.get_field("Falloff Stop Angle") or 1.0),
            "FalloffStartOpacity": float(
                shader.get_field("Falloff Start Opacity") or 0.0
            ),
            "FalloffStopOpacity": float(
                shader.get_field("Falloff Stop Opacity") or 0.0
            ),
            "LightingInfluence": float(shader.get_field("Lighting Influence") or 0.0),
            "EnvmapMinLOD": int(shader.get_field("Env Map Min LOD") or 0),
            "SoftDepth": float(shader.get_field("Soft Falloff Depth") or 100.0),
            "EnvironmentMapping": True,
            "EnvironmentMappingMaskScale": float(
                shader.get_field("Environment Map Scale") or 1.0
            ),
        }
    )
    return fields


def _extract_root_metadata(nif: NifFile, supported_ids: set[int]) -> dict[str, Any]:
    root = nif.get_block(0)
    if root is None:
        return {"behavior_graphs": []}

    metadata = _extract_node_metadata(nif, root, supported_ids)
    metadata.setdefault("behavior_graphs", [])
    animations = _extract_animation_metadata(nif, supported_ids)
    if animations:
        metadata["animations"] = animations
    return metadata


def _extract_animation_metadata(
    nif: NifFile, supported_ids: set[int]
) -> dict[str, Any]:
    managers: list[dict[str, Any]] = []
    sequences: list[dict[str, Any]] = []
    sequence_controller_ids: set[int] = set()

    for block in nif.blocks:
        if block.type_name != "NiControllerManager":
            continue
        manager_doc = _extract_controller_manager_metadata(
            nif,
            block,
            supported_ids,
            sequences=sequences,
            sequence_controller_ids=sequence_controller_ids,
        )
        if manager_doc is not None:
            managers.append(manager_doc)

    direct_controllers: list[dict[str, Any]] = []
    shader_owner_by_shader_id = _shader_owner_by_shader_id(nif)
    for block in nif.blocks:
        if not _is_shader_property_controller(block):
            continue
        if block.block_id in sequence_controller_ids:
            continue
        _mark_animation_subgraph_supported(nif, block.block_id, supported_ids)
        direct_controllers.append(
            _shader_controller_summary(nif, block, shader_owner_by_shader_id)
        )

    if not managers and not sequences and not direct_controllers:
        return {}
    return {
        "managers": managers,
        "sequences": sequences,
        "direct_controllers": direct_controllers,
    }


def _extract_controller_manager_metadata(
    nif: NifFile,
    manager: NifBlock,
    supported_ids: set[int],
    *,
    sequences: list[dict[str, Any]],
    sequence_controller_ids: set[int],
) -> dict[str, Any] | None:
    _mark_animation_subgraph_supported(nif, manager.block_id, supported_ids)
    sequence_ids = [
        _ref_id(ref) for ref in (manager.get_field("Controller Sequences") or [])
    ]
    sequence_ids = [block_id for block_id in sequence_ids if block_id >= 0]
    for sequence_id in sequence_ids:
        sequence = nif.get_block(sequence_id)
        if sequence is None or sequence.type_name != "NiControllerSequence":
            continue
        sequences.append(
            _controller_sequence_summary(
                nif,
                sequence,
                supported_ids,
                sequence_controller_ids,
            )
        )
    return {
        "source_block_id": manager.block_id,
        "target_block_id": _ref_id(manager.get_field("Target")),
        "controller_sequence_ids": sequence_ids,
        "object_palette_block_id": _ref_id(manager.get_field("Object Palette")),
    }


def _controller_sequence_summary(
    nif: NifFile,
    sequence: NifBlock,
    supported_ids: set[int],
    sequence_controller_ids: set[int],
) -> dict[str, Any]:
    _mark_animation_subgraph_supported(nif, sequence.block_id, supported_ids)
    palette_targets = _sequence_palette_targets(nif, sequence)
    shader_owner_by_shader_id = _shader_owner_by_shader_id(nif)
    text_key_events: list[dict[str, Any]] = []
    text_key_id = _ref_id(sequence.get_field("Text Keys"))
    if text_key_id >= 0:
        text_keys = nif.get_block(text_key_id)
        if text_keys is not None and text_keys.type_name == "NiTextKeyExtraData":
            for entry in text_keys.get_field("Text Keys") or []:
                text_key_events.append(
                    {
                        "time": float(entry.get("Time") or 0.0),
                        "value": entry.get("Value") or "",
                    }
                )

    controlled_blocks: list[dict[str, Any]] = []
    for controlled_index, controlled in enumerate(sequence.get_field("Controlled Blocks") or []):
        controller_id = _ref_id(controlled.get("Controller"))
        interpolator_id = _ref_id(controlled.get("Interpolator"))
        if controller_id >= 0:
            sequence_controller_ids.add(controller_id)
            _mark_animation_subgraph_supported(nif, controller_id, supported_ids)
        if interpolator_id >= 0:
            _mark_animation_subgraph_supported(nif, interpolator_id, supported_ids)
        controller = nif.get_block(controller_id)
        interpolator = nif.get_block(interpolator_id)
        entry = {
            "controlled_block_index": controlled_index,
            "controller_block_id": controller_id,
            "interpolator_block_id": interpolator_id,
            "priority": int(controlled.get("Priority") or 0),
            "node_name": controlled.get("Node Name") or "",
            "property_type": controlled.get("Property Type") or "",
            "controller_type": controlled.get("Controller Type") or "",
            "controller_id": controlled.get("Controller ID") or "",
            "interpolator_id": controlled.get("Interpolator ID") or "",
        }
        if controller is not None or interpolator is not None:
            target_name = str(entry["node_name"] or "")
            target_block_id = palette_targets.get(target_name, -1)
            channel_type = _classify_animation_channel(controller, interpolator)
            if channel_type == "transform":
                entry.update(
                    _extract_transform_channel(
                        nif,
                        sequence=sequence,
                        controller=controller,
                        interpolator=interpolator,
                        target_name=target_name,
                        target_block_id=target_block_id,
                    )
                )
            elif channel_type == "bool":
                entry.update(
                    _extract_bool_channel(
                        nif,
                        interpolator=interpolator,
                        target_name=target_name,
                        target_block_id=target_block_id,
                    )
                )
            elif channel_type == "point3":
                entry.update(
                    _extract_point3_channel(
                        nif,
                        interpolator=interpolator,
                        target_name=target_name,
                        target_block_id=target_block_id,
                    )
                )
            elif channel_type == "float":
                entry.update(
                    _extract_float_channel(
                        nif,
                        controller=controller,
                        interpolator=interpolator,
                        shader_owner_by_shader_id=shader_owner_by_shader_id,
                        target_name=target_name,
                        target_block_id=target_block_id,
                    )
                )
            else:
                entry.update(
                    {
                        "channel_type": "unsupported",
                        "target_name": target_name,
                        "target_block_id": target_block_id,
                        "warning": (
                            "Unsupported animation channel "
                            f"controller={entry['controller_type'] or (controller.type_name if controller else '')} "
                            f"interpolator={interpolator.type_name if interpolator else ''}"
                        ),
                    }
                )
        controlled_blocks.append(entry)

    return {
        "source_block_id": sequence.block_id,
        "manager_block_id": _ref_id(sequence.get_field("Manager")),
        "name": sequence.get_field("Name") or "",
        "cycle_type": sequence.get_field("Cycle Type") or "",
        "accum_root_name": sequence.get_field("Accum Root Name") or "",
        "start_time": float(sequence.get_field("Start Time") or 0.0),
        "stop_time": float(sequence.get_field("Stop Time") or 0.0),
        "text_key_block_id": text_key_id,
        "text_keys": text_key_events,
        "controlled_blocks": controlled_blocks,
    }


def _mark_animation_subgraph_supported(
    nif: NifFile,
    block_id: int,
    supported_ids: set[int],
    visited: set[int] | None = None,
) -> None:
    if block_id < 0:
        return
    if visited is None:
        visited = set()
    if block_id in visited:
        return
    block = nif.get_block(block_id)
    if block is None or not _is_animation_block(block):
        return
    visited.add(block_id)
    supported_ids.add(block_id)
    for ref_id in block.get_refs(nif.schema):
        target = nif.get_block(ref_id)
        if target is None or not _is_animation_block(target):
            continue
        _mark_animation_subgraph_supported(nif, ref_id, supported_ids, visited)


def _is_animation_block(block: NifBlock) -> bool:
    type_name = block.type_name
    if type_name in {"NiDefaultAVObjectPalette", "NiTextKeyExtraData"}:
        return True
    return any(
        token in type_name
        for token in (
            "Controller",
            "Interpolator",
            "Sequence",
            "FloatData",
            "TransformData",
            "Point3Data",
            "PosData",
            "ColorData",
            "BoolData",
        )
    )


def _is_shader_property_controller(block: NifBlock) -> bool:
    return "ShaderPropertyFloatController" in block.type_name


def _classify_animation_channel(
    controller: NifBlock | None, interpolator: NifBlock | None
) -> str:
    controller_type = controller.type_name if controller is not None else ""
    interpolator_type = interpolator.type_name if interpolator is not None else ""
    if (
        "TransformController" in controller_type
        or interpolator_type == "NiTransformInterpolator"
    ):
        return "transform"
    if (
        controller_type == "NiVisController"
        or "Bool" in controller_type
        or interpolator_type in {
            "NiBoolInterpolator",
            "NiBoolTimelineInterpolator",
        }
    ):
        return "bool"
    if (
        "Point3" in controller_type
        or "Color" in controller_type
        or interpolator_type in {"NiPoint3Interpolator", "NiColorInterpolator"}
    ):
        return "point3"
    if (
        "FloatController" in controller_type
        or interpolator_type in {"NiFloatInterpolator", "NiBlendFloatInterpolator"}
    ):
        return "float"
    return "unsupported"


def _sequence_palette_targets(nif: NifFile, sequence: NifBlock) -> dict[str, int]:
    manager = nif.get_block(_ref_id(sequence.get_field("Manager")))
    if manager is None:
        return {}
    palette = nif.get_block(_ref_id(manager.get_field("Object Palette")))
    if palette is None or palette.type_name != "NiDefaultAVObjectPalette":
        return {}
    return {
        str(entry.get("Name") or ""): _ref_id(entry.get("AV Object"))
        for entry in (palette.get_field("Objs") or [])
        if str(entry.get("Name") or "")
    }


def _extract_transform_channel(
    nif: NifFile,
    *,
    sequence: NifBlock,
    controller: NifBlock | None,
    interpolator: NifBlock | None,
    target_name: str,
    target_block_id: int,
) -> dict[str, Any]:
    data_id = -1
    translation_keys: list[dict[str, Any]] = []
    rotation_keys = {
        "rotation_type": "XYZ_ROTATION_KEY",
        "x": [],
        "y": [],
        "z": [],
    }
    scale_keys: list[dict[str, Any]] = []

    if interpolator is not None and interpolator.type_name == "NiTransformInterpolator":
        data_id = _ref_id(interpolator.get_field("Data"))
        data = nif.get_block(data_id)
        if data is not None and data.type_name == "NiTransformData":
            translations = data.get_field("Translations") or {}
            translation_keys = [
                {
                    "time": float(entry.get("Time") or 0.0),
                    "value": _vector3(entry.get("Value") or {}),
                }
                for entry in (translations.get("Keys") or [])
            ]
            rotation_type = str(data.get_field("Rotation Type") or "XYZ_ROTATION_KEY")
            rotation_keys["rotation_type"] = rotation_type
            xyz_rotations = list(data.get_field("XYZ Rotations") or [])
            for axis_index, axis_name in enumerate(("x", "y", "z")):
                axis = xyz_rotations[axis_index] if axis_index < len(xyz_rotations) else {}
                rotation_keys[axis_name] = [
                    {
                        "time": float(entry.get("Time") or 0.0),
                        "value": float(entry.get("Value") or 0.0),
                    }
                    for entry in (axis.get("Keys") or [])
                ]
            scales = data.get_field("Scales") or {}
            scale_keys = [
                {
                    "time": float(entry.get("Time") or 0.0),
                    "value": float(entry.get("Value") or 1.0),
                }
                for entry in (scales.get("Keys") or [])
            ]
        else:
            default_transform = interpolator.get_field("Transform") or {}
            default_translation = _vector3(default_transform.get("Translation") or {})
            default_rotation = _quaternion(default_transform.get("Rotation") or {})
            default_scale = float(default_transform.get("Scale") or 1.0)
            if _has_finite_vector(default_translation):
                translation_keys = [
                    {
                        "time": float(sequence.get_field("Start Time") or 0.0),
                        "value": default_translation,
                    }
                ]
            if _has_finite_quaternion(default_rotation):
                euler = _quaternion_to_euler_xyz(default_rotation)
                for axis_name, axis_value in zip(("x", "y", "z"), euler, strict=False):
                    rotation_keys[axis_name] = [
                        {
                            "time": float(sequence.get_field("Start Time") or 0.0),
                            "value": float(axis_value),
                        }
                    ]
            if math.isfinite(default_scale) and abs(default_scale) < 1.0e20:
                scale_keys = [
                    {
                        "time": float(sequence.get_field("Start Time") or 0.0),
                        "value": default_scale,
                    }
                ]

    return {
        "channel_type": "transform",
        "target_name": target_name,
        "target_block_id": target_block_id,
        "data_block_id": data_id,
        "translation_keys": translation_keys,
        "rotation_keys": rotation_keys,
        "scale_keys": scale_keys,
    }


def _extract_bool_channel(
    nif: NifFile,
    *,
    interpolator: NifBlock | None,
    target_name: str,
    target_block_id: int,
) -> dict[str, Any]:
    data_id = -1
    interpolation = 1
    bool_keys: list[dict[str, Any]] = []
    constant_value = None

    if interpolator is not None:
        data_id = _ref_id(interpolator.get_field("Data"))
        value = interpolator.get_field("Value")
        if value not in (None, ""):
            constant_value = bool(value)
        data = nif.get_block(data_id)
        if data is not None and data.type_name == "NiBoolData":
            payload = data.get_field("Data") or {}
            interpolation = int(payload.get("Interpolation") or 1)
            bool_keys = [
                {
                    "time": float(entry.get("Time") or 0.0),
                    "value": bool(entry.get("Value")),
                }
                for entry in (payload.get("Keys") or [])
            ]

    if not bool_keys and constant_value is not None:
        bool_keys = [{"time": 0.0, "value": constant_value}]

    return {
        "channel_type": "bool",
        "target_name": target_name,
        "target_block_id": target_block_id,
        "data_block_id": data_id,
        "interpolation": interpolation,
        "bool_keys": bool_keys,
    }


def _extract_point3_channel(
    nif: NifFile,
    *,
    interpolator: NifBlock | None,
    target_name: str,
    target_block_id: int,
) -> dict[str, Any]:
    data_id = -1
    point3_keys: list[dict[str, Any]] = []
    constant_value = None

    if interpolator is not None:
        data_id = _ref_id(interpolator.get_field("Data"))
        value = interpolator.get_field("Value")
        if value not in (None, ""):
            if interpolator.type_name == "NiColorInterpolator":
                constant_value = _color3_point3(value)
            else:
                constant_value = _vector3(value)
        data = nif.get_block(data_id)
        if data is not None and data.type_name in {"NiPoint3Data", "NiPosData"}:
            payload = data.get_field("Data") or {}
            point3_keys = [
                {
                    "time": float(entry.get("Time") or 0.0),
                    "value": _vector3(entry.get("Value") or {}),
                }
                for entry in (payload.get("Keys") or [])
            ]
        elif data is not None and data.type_name == "NiColorData":
            payload = data.get_field("Data") or {}
            point3_keys = [
                {
                    "time": float(entry.get("Time") or 0.0),
                    "value": _color3_point3(entry.get("Value") or {}),
                }
                for entry in (payload.get("Keys") or [])
            ]

    if not point3_keys and constant_value is not None:
        point3_keys = [{"time": 0.0, "value": constant_value}]

    return {
        "channel_type": "point3",
        "target_name": target_name,
        "target_block_id": target_block_id,
        "data_block_id": data_id,
        "point3_keys": point3_keys,
    }


def _color3_point3(value: Any) -> dict[str, float]:
    rgb = _color3_tuple(value)
    return {"x": float(rgb[0]), "y": float(rgb[1]), "z": float(rgb[2])}


def _extract_float_channel(
    nif: NifFile,
    *,
    controller: NifBlock | None,
    interpolator: NifBlock | None,
    shader_owner_by_shader_id: dict[int, int],
    target_name: str,
    target_block_id: int,
) -> dict[str, Any]:
    target_id = _ref_id(controller.get_field("Target")) if controller is not None else -1
    if target_block_id < 0:
        target_block_id = shader_owner_by_shader_id.get(target_id, -1)
    if not target_name:
        target_name = _animation_target_name(nif, target_id, shader_owner_by_shader_id)

    data_id = -1
    interpolation = 1
    float_keys: list[dict[str, Any]] = []
    constant_value = None

    if interpolator is not None and interpolator.type_name == "NiFloatInterpolator":
        data_id = _ref_id(interpolator.get_field("Data"))
        value = interpolator.get_field("Value")
        if isinstance(value, (int, float)) and math.isfinite(float(value)):
            constant_value = float(value)
        data = nif.get_block(data_id)
        if data is not None and data.type_name == "NiFloatData":
            payload = data.get_field("Data") or {}
            interpolation = int(payload.get("Interpolation") or 1)
            float_keys = [
                {
                    "time": float(entry.get("Time") or 0.0),
                    "value": float(entry.get("Value") or 0.0),
                }
                for entry in (payload.get("Keys") or [])
            ]

    if not float_keys and constant_value is not None:
        float_keys = [{"time": 0.0, "value": constant_value}]

    return {
        "channel_type": "float",
        "target_name": target_name,
        "target_block_id": target_block_id,
        "target_owner_block_id": target_block_id,
        "data_block_id": data_id,
        "interpolation": interpolation,
        "controlled_variable": (
            controller.get_field("Controlled Variable") if controller is not None else ""
        )
        or "",
        "type": controller.type_name if controller is not None else "",
        "float_keys": float_keys,
    }


def _has_finite_vector(value: dict[str, float]) -> bool:
    return all(math.isfinite(float(value.get(axis, 0.0))) and abs(float(value.get(axis, 0.0))) < 1.0e20 for axis in ("x", "y", "z"))


def _has_finite_quaternion(value: dict[str, float]) -> bool:
    return all(math.isfinite(float(value.get(axis, 0.0))) and abs(float(value.get(axis, 0.0))) < 1.0e20 for axis in ("w", "x", "y", "z"))


def _quaternion_to_euler_xyz(value: dict[str, float]) -> tuple[float, float, float]:
    w = float(value.get("w", 1.0))
    x = float(value.get("x", 0.0))
    y = float(value.get("y", 0.0))
    z = float(value.get("z", 0.0))

    sinr_cosp = 2.0 * (w * x + y * z)
    cosr_cosp = 1.0 - 2.0 * (x * x + y * y)
    roll = math.atan2(sinr_cosp, cosr_cosp)

    sinp = 2.0 * (w * y - z * x)
    if abs(sinp) >= 1.0:
        pitch = math.copysign(math.pi / 2.0, sinp)
    else:
        pitch = math.asin(sinp)

    siny_cosp = 2.0 * (w * z + x * y)
    cosy_cosp = 1.0 - 2.0 * (y * y + z * z)
    yaw = math.atan2(siny_cosp, cosy_cosp)
    return (roll, pitch, yaw)


def _shader_controller_summary(
    nif: NifFile,
    controller: NifBlock,
    shader_owner_by_shader_id: dict[int, int],
) -> dict[str, Any]:
    target_id = _ref_id(controller.get_field("Target"))
    target_owner_id = shader_owner_by_shader_id.get(target_id, -1)
    target_name = _animation_target_name(
        nif,
        target_id,
        shader_owner_by_shader_id,
    )
    return {
        "source_block_id": controller.block_id,
        "type": controller.type_name,
        "target_block_id": target_id,
        "target_owner_block_id": target_owner_id,
        "target_name": target_name,
        "controlled_variable": controller.get_field("Controlled Variable") or "",
        "next_controller_block_id": _ref_id(controller.get_field("Next Controller")),
        "interpolator_block_id": _ref_id(controller.get_field("Interpolator")),
    }


def _build_import_warnings(unsupported_blocks: list[dict[str, Any]]) -> list[str]:
    warnings: list[str] = []
    if unsupported_blocks:
        warnings.append(
            f"Preserving {len(unsupported_blocks)} unsupported blocks as opaque payload"
        )
    return warnings


def _promote_root_for_animated_scene(nif: NifFile, document: dict[str, Any]) -> None:
    animation_metadata = (
        ((document.get("metadata") or {}).get("root") or {}).get("animations") or {}
    )
    if not _has_animation_metadata(animation_metadata):
        return
    root = nif.get_block(0)
    if root is None or root.type_name != "BSFadeNode":
        return
    root.type_name = "NiNode"
    nif._rebuild_header()


def _load_or_create_export_nif(document: dict[str, Any], *, game: str) -> NifFile:
    export_settings = dict(document.get("export_settings") or {})
    if bool(export_settings.get("selected_roots")):
        return NifFile.new(game)

    payload = dict(document.get("opaque_payload") or {})
    original_b64 = payload.get("original_nif_base64")
    if original_b64:
        raw_bytes = base64.b64decode(original_b64)
        with tempfile.NamedTemporaryFile(suffix=".nif", delete=False) as handle:
            handle.write(raw_bytes)
            temp_path = handle.name
        nif = NifFile.load(temp_path)
        nif_game = _detect_nif_game(nif)
        if nif_game != game:
            raise ValueError(
                f"Scene game '{game}' does not match imported NIF game '{nif_game}'. "
                "Changing game on imported scenes is not supported yet."
            )
        return nif
    return NifFile.new(game)


def _uses_synthetic_scene_root(
    root_nodes: list[dict[str, Any]], export_settings: dict[str, Any]
) -> bool:
    if not root_nodes:
        return False
    if not bool(export_settings.get("synthesize_scene_root")):
        return False
    root = root_nodes[0]
    return root.get("id") == "mb21-export-root" and root.get("type") == "node"


def _uses_existing_scene_root(
    root_nodes: list[dict[str, Any]], export_settings: dict[str, Any]
) -> bool:
    # A single 'node' root IS the scene root — merge it onto block 0 instead of
    # renaming block 0 to the file stem and re-attaching the user's node as a
    # child, which produces a duplicate NiNode with the same name (e.g. file
    # stem "bank" wrapping user helper "bank"). Applies whether or not the user
    # invoked selected-only export.
    if not root_nodes:
        return False
    if len(root_nodes) != 1:
        return False
    return root_nodes[0].get("type") == "node"


def _apply_root_document(
    nif: NifFile,
    scene_root: NifBlock,
    node_doc: dict[str, Any],
    materials_by_id: dict[str, dict[str, Any]],
    output_path: Path,
    warnings: list[str],
    profile,
) -> None:
    if node_doc.get("type") == "helper":
        _apply_connect_point_helpers(nif, scene_root, [node_doc])
        return
    if node_doc.get("type") == "collision":
        host_doc = {
            **node_doc,
            "type": "node",
            "metadata": {
                key: value
                for key, value in dict(node_doc.get("metadata") or {}).items()
                if key != "collision"
            },
        }
        host_block = _apply_node_document(
            nif=nif,
            node_doc=host_doc,
            owner_parent=scene_root,
            materials_by_id=materials_by_id,
            output_path=output_path,
            warnings=warnings,
            profile=profile,
        )
        if host_block is not None:
            _apply_collision_children(
                nif=nif,
                host=host_block,
                collision_nodes=[node_doc],
                fallback_node_doc=None,
                warnings=warnings,
                profile=profile,
            )
        return

    _apply_node_document(
        nif=nif,
        node_doc=node_doc,
        owner_parent=scene_root,
        materials_by_id=materials_by_id,
        output_path=output_path,
        warnings=warnings,
        profile=profile,
    )


def _apply_node_document(
    nif: NifFile,
    node_doc: dict[str, Any],
    owner_parent: NifBlock | None,
    materials_by_id: dict[str, dict[str, Any]],
    output_path: Path,
    warnings: list[str],
    profile,
    forced_block: NifBlock | None = None,
) -> NifBlock | None:
    node_type = node_doc.get("type")
    if node_type == "helper":
        return None
    if node_type == "collision":
        return None
    if forced_block is None and _mesh_node_needs_parent_wrapper(node_doc):
        return _apply_mesh_parent_wrapper_document(
            nif=nif,
            node_doc=node_doc,
            owner_parent=owner_parent,
            materials_by_id=materials_by_id,
            output_path=output_path,
            warnings=warnings,
            profile=profile,
        )

    block = forced_block or _resolve_or_create_block(nif, node_doc)
    if forced_block is not None:
        _coerce_forced_block_type(nif, block, node_doc)
    _apply_basic_node_fields(block, node_doc)
    _apply_special_node_fields(block, node_doc)

    if owner_parent is not None:
        _attach_child(owner_parent, block.block_id)

    _apply_node_extra_data(nif, block, node_doc)
    if node_doc.get("type") == "mesh":
        _apply_mesh_data(nif, block, node_doc, warnings)
        _apply_alpha_property(nif, block, node_doc)
        _apply_material_binding(
            nif=nif,
            game=profile.id,
            shape=block,
            node_doc=node_doc,
            materials_by_id=materials_by_id,
            output_path=output_path,
            warnings=warnings,
        )
        _apply_vertex_color_shader_flags(nif, block, node_doc)

    helper_children = [
        child
        for child in node_doc.get("children") or []
        if child.get("type") == "helper"
    ]
    _apply_connect_point_helpers(nif, block, helper_children)
    collision_children = [
        child
        for child in node_doc.get("children") or []
        if child.get("type") == "collision"
    ]
    _apply_collision_children(
        nif=nif,
        host=block,
        collision_nodes=collision_children,
        fallback_node_doc=node_doc,
        warnings=warnings,
        profile=profile,
    )

    for child_doc in node_doc.get("children") or []:
        if child_doc.get("type") in {"helper", "collision"}:
            continue
        _apply_node_document(
            nif=nif,
            node_doc=child_doc,
            owner_parent=block,
            materials_by_id=materials_by_id,
            output_path=output_path,
            warnings=warnings,
            profile=profile,
        )

    return block


def _mesh_node_needs_parent_wrapper(node_doc: dict[str, Any]) -> bool:
    if node_doc.get("type") != "mesh":
        return False
    if any(
        child.get("type") != "helper"
        for child in node_doc.get("children") or []
    ):
        return True
    collision = (node_doc.get("metadata") or {}).get("collision")
    return isinstance(collision, dict) and bool(collision.get("enabled"))


def _apply_mesh_parent_wrapper_document(
    nif: NifFile,
    node_doc: dict[str, Any],
    owner_parent: NifBlock | None,
    materials_by_id: dict[str, dict[str, Any]],
    output_path: Path,
    warnings: list[str],
    profile,
) -> NifBlock:
    wrapper = nif.add_block("NiNode")
    wrapper_doc = {
        "type": "node",
        "nif_type": "NiNode",
        "name": node_doc.get("name") or "",
        "transform": node_doc.get("transform") or {},
        "metadata": {"extra_string_data": []},
        "children": [],
    }
    _apply_basic_node_fields(wrapper, wrapper_doc)
    if owner_parent is not None:
        _attach_child(owner_parent, wrapper.block_id)

    collision_children = [
        child
        for child in node_doc.get("children") or []
        if child.get("type") == "collision"
    ]
    _apply_collision_children(
        nif=nif,
        host=wrapper,
        collision_nodes=collision_children,
        fallback_node_doc=node_doc,
        warnings=warnings,
        profile=profile,
    )

    shape_doc = dict(node_doc)
    shape_doc["id"] = f"{node_doc.get('id') or node_doc.get('name') or 'mesh'}-shape"
    base_name = str(node_doc.get("name") or "")
    if base_name and not base_name.endswith(":0"):
        shape_doc["name"] = f"{base_name}:0"
    shape_doc["transform"] = _identity_transform()
    shape_doc_metadata = dict(node_doc.get("metadata") or {})
    shape_doc_metadata.pop("collision", None)
    shape_doc["metadata"] = shape_doc_metadata
    shape_doc["children"] = [
        child
        for child in node_doc.get("children") or []
        if child.get("type") == "helper"
    ]
    _apply_node_document(
        nif=nif,
        node_doc=shape_doc,
        owner_parent=wrapper,
        materials_by_id=materials_by_id,
        output_path=output_path,
        warnings=warnings,
        profile=profile,
    )

    for child_doc in node_doc.get("children") or []:
        if child_doc.get("type") in {"helper", "collision"}:
            continue
        _apply_node_document(
            nif=nif,
            node_doc=child_doc,
            owner_parent=wrapper,
            materials_by_id=materials_by_id,
            output_path=output_path,
            warnings=warnings,
            profile=profile,
        )
    return wrapper


def _resolve_or_create_block(nif: NifFile, node_doc: dict[str, Any]) -> NifBlock:
    source_block_id = node_doc.get("source_block_id")
    if isinstance(source_block_id, int):
        existing = nif.get_block(source_block_id)
        if existing is not None:
            return existing

    type_name = str(node_doc.get("nif_type") or "")
    if not type_name:
        type_name = "BSTriShape" if node_doc.get("type") == "mesh" else "NiNode"
    return nif.add_block(type_name)


def _coerce_forced_block_type(
    nif: NifFile, block: NifBlock, node_doc: dict[str, Any]
) -> None:
    type_name = str(node_doc.get("nif_type") or "")
    if type_name not in NODE_TYPES or block.type_name == type_name:
        return
    if block.type_name not in NODE_TYPES:
        return
    block.type_name = type_name
    nif._rebuild_header()


def _identity_transform() -> dict[str, Any]:
    return {
        "translation": {"x": 0.0, "y": 0.0, "z": 0.0},
        "rotation": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        "scale": 1.0,
    }


def _apply_basic_node_fields(block: NifBlock, node_doc: dict[str, Any]) -> None:
    block.set_field("Name", node_doc.get("name") or "")
    metadata = dict(node_doc.get("metadata") or {})
    if metadata.get("node_flags") not in (None, ""):
        block.set_field("Flags", int(metadata["node_flags"]))
    elif block.type_name in NODE_TYPES:
        block.set_field("Flags", 14)
    transform = node_doc.get("transform") or {}
    block.set_field("Translation", _vector3(transform.get("translation") or {}))
    block.set_field("Rotation", _matrix33(transform.get("rotation") or {}))
    block.set_field("Scale", float(transform.get("scale", 1.0)))


def _apply_special_node_fields(block: NifBlock, node_doc: dict[str, Any]) -> None:
    if block.type_name != "BSValueNode":
        return
    metadata = dict(node_doc.get("metadata") or {})
    addon_index = metadata.get("addon_node_index")
    if addon_index is None:
        addon_index = _addon_index_from_name(str(node_doc.get("name") or ""))
    if addon_index is None:
        return
    block.set_field("Value", int(addon_index))


def _apply_mesh_data(
    nif: NifFile,
    block: NifBlock,
    node_doc: dict[str, Any],
    warnings: list[str],
) -> None:
    mesh = dict(node_doc.get("mesh") or {})
    if mesh.get("preserve_source_geometry") and not mesh.get("rewrite_geometry"):
        return

    if _is_legacy_tri_based_shape(nif, block):
        _apply_legacy_tri_based_mesh_data(nif, block, mesh)
        return

    skin = dict(mesh.get("skin") or {})
    partitions = dict(mesh.get("partitions") or {})
    deformable = (
        bool(skin.get("enabled"))
        or bool(partitions.get("enabled"))
        or bool(partitions.get("segments"))
        or bool(partitions.get("segment_ids"))
        or bool(partitions.get("partitions"))
        or _is_skinned(block)
    )
    if deformable and not _mesh_topology_matches(block, mesh):
        warnings.append(
            f"Skipped geometry rewrite for deformable mesh '{node_doc.get('name') or block.block_id}' because topology changed"
        )
        return

    vertices = list(mesh.get("vertices") or [])
    normals = list(mesh.get("normals") or [])
    uvs = list(mesh.get("uvs") or [])
    triangles = list(mesh.get("triangles") or [])
    colors = list(mesh.get("colors") or [])

    if deformable:
        vertex_data = list(block.get_field("Vertex Data") or [])
        _update_vertex_data_geometry(
            vertex_data=vertex_data,
            vertices=vertices,
            normals=normals,
            uvs=uvs,
            colors=colors,
        )
        block.set_field("Vertex Data", vertex_data)
    else:
        block.set_field(
            "Vertex Data",
            _build_vertex_data(
                vertices=vertices,
                normals=normals,
                uvs=uvs,
                colors=colors,
                triangles=triangles,
            ),
        )
    block.set_field("Num Vertices", len(vertices))
    block.set_field("Triangles", _build_triangles(triangles))
    block.set_field("Num Triangles", len(triangles))
    block.set_field("Bounding Sphere", _compute_bounding_sphere(vertices))
    if not deformable:
        block.set_field(
            "Vertex Desc",
            _build_bs_vertex_desc(
                vertices=vertices,
                normals=normals,
                uvs=uvs,
                colors=colors,
                existing_desc=int(block.get_field("Vertex Desc") or 0),
                triangles=triangles,
            ),
        )

    if deformable:
        _apply_skin_weights(nif, block, skin, warnings)
        _apply_partition_data(nif, block, mesh, warnings)


def _apply_legacy_tri_based_mesh_data(
    nif: NifFile,
    block: NifBlock,
    mesh: dict[str, Any],
) -> None:
    data_id = _ref_id(block.get_field("Data"))
    data = nif.get_block(data_id)
    desired_type = (
        "NiTriStripsData" if block.type_name == "NiTriStrips" else "NiTriShapeData"
    )
    if data is None or data.type_name != desired_type:
        data = nif.add_block(desired_type)
        block.set_field("Data", data.block_id)

    vertices = [_vector3(vertex) for vertex in (mesh.get("vertices") or [])]
    normals = [_vector3(normal) for normal in (mesh.get("normals") or [])]
    uvs = [_uv(uv) for uv in (mesh.get("uvs") or [])]
    colors = [_color4(color) for color in (mesh.get("colors") or [])]
    triangles = _build_triangles(
        list(mesh.get("uv_triangles") or mesh.get("triangles") or [])
    )

    data.set_field("Num Vertices", len(vertices))
    data.set_field("Vertices", vertices)
    data.set_field("Has Normals", 1 if normals else 0)
    data.set_field("Normals", normals)
    data.set_field("Has UV", 1 if uvs else 0)
    data.set_field("UV Sets", [uvs] if uvs else [])
    data.set_field("Has Vertex Colors", 1 if colors else 0)
    data.set_field("Vertex Colors", colors)
    data.set_field("Num Triangles", len(triangles))
    if data.type_name == "NiTriStripsData":
        strip_lengths = [3 for _ in triangles]
        points = [
            vertex_index
            for triangle in triangles
            for vertex_index in (triangle["v1"], triangle["v2"], triangle["v3"])
        ]
        data.set_field("Num Strips", len(strip_lengths))
        data.set_field("Strip Lengths", strip_lengths)
        data.set_field("Points", points)
    else:
        data.set_field("Num Triangle Points", len(triangles) * 3)
        data.set_field("Has Triangles", 1 if triangles else 0)
        data.set_field("Triangles", triangles)


def _update_vertex_data_geometry(
    *,
    vertex_data: list[dict[str, Any]],
    vertices: list[dict[str, Any]],
    normals: list[dict[str, Any]],
    uvs: list[dict[str, Any]],
    colors: list[dict[str, Any]],
) -> None:
    limit = min(len(vertex_data), len(vertices))
    for index in range(limit):
        entry = vertex_data[index]
        entry["Vertex"] = _vector3(vertices[index] if index < len(vertices) else {})
        entry["Normal"] = _vector3(normals[index] if index < len(normals) else {})
        entry["UV"] = _uv(uvs[index] if index < len(uvs) else {})
        if colors:
            entry["Vertex Colors"] = _byte_color4(
                colors[index] if index < len(colors) else {}
            )
        elif "Vertex Colors" in entry:
            entry.pop("Vertex Colors", None)


def _apply_skin_weights(
    nif: NifFile,
    block: NifBlock,
    skin: dict[str, Any],
    warnings: list[str],
) -> None:
    if not skin.get("weights"):
        return

    skin_block = _skin_block_for_shape(nif, block)
    if skin_block is None:
        warnings.append(
            f"Skipped skin weights for '{block.get_field('Name') or block.block_id}' because the shape has no skin instance"
        )
        return

    bone_names = _skin_bone_names(nif, skin_block)
    if not bone_names:
        bone_names = [str(name) for name in (skin.get("bone_names") or [])]
    bone_name_to_index = {name: index for index, name in enumerate(bone_names) if name}

    vertex_data = list(block.get_field("Vertex Data") or [])
    if not vertex_data:
        return

    raw_weights = list(skin.get("weights") or [])
    if not raw_weights:
        return

    dense_width = max(
        MAX_SKIN_INFLUENCES,
        max((len(vertex) for vertex in raw_weights if isinstance(vertex, list)), default=0),
    )
    weights = np.zeros((len(vertex_data), dense_width), dtype=np.float32)
    bone_indices = np.zeros((len(vertex_data), dense_width), dtype=np.int32)

    for vertex_index, influences in enumerate(raw_weights[: len(vertex_data)]):
        if not isinstance(influences, list):
            continue
        sortable: list[tuple[float, str, int, int]] = []
        for entry in influences:
            if not isinstance(entry, dict):
                continue
            weight = float(entry.get("weight") or 0.0)
            if weight <= 0.0:
                continue
            bone_index = entry.get("bone_index")
            bone_name = str(entry.get("bone_name") or "")
            if bone_index in (None, "") and bone_name in bone_name_to_index:
                bone_index = bone_name_to_index[bone_name]
            if bone_index in (None, ""):
                continue
            sortable.append(
                (
                    float(weight),
                    bone_name,
                    int(bone_index),
                    int(entry.get("bone_id") or -1),
                )
            )
        sortable.sort(key=lambda item: (-item[0], item[1], item[2], item[3]))
        for slot, (weight, _bone_name, bone_index, _bone_id) in enumerate(
            sortable[:dense_width]
        ):
            weights[vertex_index, slot] = float(weight)
            bone_indices[vertex_index, slot] = int(bone_index)

    weights, bone_indices, _ = normalize_weights(
        weights,
        bone_indices,
        max_bones=MAX_SKIN_INFLUENCES,
    )

    for index, entry in enumerate(vertex_data[: len(weights)]):
        entry["Bone Weights"] = [float(value) for value in weights[index]]
        entry["Bone Indices"] = [int(value) for value in bone_indices[index]]

    block.set_field("Vertex Data", vertex_data)


def _apply_partition_data(
    nif: NifFile,
    block: NifBlock,
    mesh: dict[str, Any],
    warnings: list[str],
) -> None:
    partitions = dict(mesh.get("partitions") or {})
    if not partitions:
        return

    triangles = list(mesh.get("triangles") or [])
    if not triangles:
        return

    segment_ids = [int(value) for value in (partitions.get("segment_ids") or [])]
    segments = [
        _segment_info_from_doc(entry)
        for entry in (partitions.get("segments") or [])
        if isinstance(entry, dict)
    ]
    if not segments and segment_ids:
        stub = SkinData.from_geometry(
            _mesh_vertices_array(mesh),
            _mesh_triangles_array(triangles),
        )
        stub.segment_ids = np.asarray(segment_ids, dtype=np.int32)
        segments = rebuild_fo4_segments(stub)
    if not segments and not segment_ids and not partitions.get("partitions"):
        return

    skin_block = _skin_block_for_shape(nif, block)
    if partitions.get("source_shape_type") == "BSSubIndexTriShape" or segments:
        if block.type_name != "BSSubIndexTriShape":
            convert_to_sub_index_tri_shape(nif, block.block_id)
        skin_data = _build_partition_skin_data(mesh, partitions, segment_ids, segments)
        _write_fo4_segments(block, skin_data, 0, len(triangles))

    if skin_block is not None and skin_block.type_name == "BSDismemberSkinInstance":
        skin_data = _build_partition_skin_data(mesh, partitions, segment_ids, segments)
        _write_dismember_partitions(nif, block, skin_data, 0, len(triangles))


def _build_partition_skin_data(
    mesh: dict[str, Any],
    partitions: dict[str, Any],
    segment_ids: list[int],
    segments: list[SegmentInfo],
) -> SkinData:
    vertices = _mesh_vertices_array(mesh)
    triangles = _mesh_triangles_array(list(mesh.get("triangles") or []))
    skin_data = SkinData.from_geometry(vertices, triangles)
    if segment_ids:
        skin_data.segment_ids = np.asarray(segment_ids, dtype=np.int32)
    elif partitions.get("segment_ids"):
        skin_data.segment_ids = np.asarray(partitions.get("segment_ids"), dtype=np.int32)
    if not segments and np.any(skin_data.segment_ids >= 0):
        skin_data.segments = rebuild_fo4_segments(skin_data)
    else:
        skin_data.segments = segments
    skin_data.ssf_file = str(partitions.get("segment_file") or "")
    return skin_data


def _mesh_vertices_array(mesh: dict[str, Any]) -> np.ndarray:
    return np.asarray(
        [
            [
                float(vertex.get("x", 0.0)),
                float(vertex.get("y", 0.0)),
                float(vertex.get("z", 0.0)),
            ]
            for vertex in (mesh.get("vertices") or [])
            if isinstance(vertex, dict)
        ],
        dtype=np.float32,
    ).reshape(-1, 3)


def _mesh_triangles_array(triangles: list[dict[str, Any]]) -> np.ndarray:
    return np.asarray(
        [
            [
                int(triangle.get("v1", triangle.get("V1", 0))),
                int(triangle.get("v2", triangle.get("V2", 0))),
                int(triangle.get("v3", triangle.get("V3", 0))),
            ]
            for triangle in triangles
            if isinstance(triangle, dict)
        ],
        dtype=np.uint32,
    ).reshape(-1, 3)


def _segment_info_from_doc(entry: dict[str, Any]) -> SegmentInfo:
    sub_segments = [
        _sub_segment_info_from_doc(sub_entry)
        for sub_entry in entry.get("sub_segments") or []
        if isinstance(sub_entry, dict)
    ]
    return SegmentInfo(
        start_index=int(entry.get("start_index") or 0),
        num_primitives=int(entry.get("num_primitives") or 0),
        sub_segments=sub_segments,
        user_index=int(entry.get("user_index") or 0),
    )


def _sub_segment_info_from_doc(entry: dict[str, Any]) -> SubSegmentInfo:
    return SubSegmentInfo(
        start_index=int(entry.get("start_index") or 0),
        num_primitives=int(entry.get("num_primitives") or 0),
        user_index=int(entry.get("user_index") or 0),
        bone_id=int(entry.get("bone_id") or 0xFFFFFFFF),
        cut_offsets=[float(value) for value in entry.get("cut_offsets") or []],
    )


def _skin_block_for_shape(nif: NifFile, block: NifBlock) -> NifBlock | None:
    skin_id = _ref_id(block.get_field("Skin Instance"))
    if skin_id < 0:
        skin_id = _ref_id(block.get_field("Skin"))
    if skin_id < 0:
        return None
    return nif.get_block(skin_id)


def _skin_bone_names(nif: NifFile, skin_block: NifBlock) -> list[str]:
    bone_names: list[str] = []
    for bone_ref in skin_block.get_field("Bones") or []:
        bone_id = _ref_id(bone_ref)
        bone_block = nif.get_block(bone_id) if bone_id >= 0 else None
        bone_names.append((bone_block.get_field("Name") if bone_block else "") or "")
    return bone_names


def _mesh_topology_matches(block: NifBlock, mesh: dict[str, Any]) -> bool:
    block_triangles = _build_triangles(block.get_field("Triangles") or [])
    mesh_triangles = _build_triangles(
        list(mesh.get("uv_triangles") or mesh.get("triangles") or [])
    )
    vertex_data_len = len(block.get_field("Vertex Data") or [])
    per_corner = mesh.get("uvs") or mesh.get("normals")
    mesh_corner_len = (
        len(per_corner) if per_corner is not None else len(mesh.get("vertices") or [])
    )
    if vertex_data_len != mesh_corner_len:
        return False
    if len(block_triangles) != len(mesh_triangles):
        return False
    return block_triangles == mesh_triangles


def _apply_node_extra_data(
    nif: NifFile, owner: NifBlock, node_doc: dict[str, Any]
) -> None:
    metadata = dict(node_doc.get("metadata") or {})
    entries = list(metadata.get("extra_string_data") or [])
    existing = {
        extra.block_id: extra
        for extra in _iter_extra_blocks(nif, owner)
        if extra.type_name == "NiStringExtraData"
    }

    for entry in entries:
        extra = None
        source_id = entry.get("source_block_id")
        if isinstance(source_id, int):
            extra = existing.get(source_id)
        if extra is None:
            extra = nif.add_block("NiStringExtraData")
            _attach_extra(owner, extra.block_id)
        extra.set_field("Name", entry.get("name") or "")
        extra.set_field("String Data", entry.get("value") or "")

    if "bsx_flags" in metadata:
        bsx = _ensure_extra_block(nif, owner, "BSXFlags")
        bsx.set_field("Name", "BSX")
        bsx.set_field("Integer Data", int(metadata.get("bsx_flags") or 0))

    for behavior in metadata.get("behavior_graphs") or []:
        block = None
        source_id = behavior.get("source_block_id")
        if isinstance(source_id, int):
            existing_block = nif.get_block(source_id)
            if (
                existing_block is not None
                and existing_block.type_name == "BSBehaviorGraphExtraData"
            ):
                block = existing_block
        if block is None:
            block = nif.add_block("BSBehaviorGraphExtraData")
            _attach_extra(owner, block.block_id)
        block.set_field("Behaviour Graph File", behavior.get("path") or "")
        block.set_field(
            "Controls Base Skeleton",
            bool(behavior.get("controls_base_skeleton")),
        )


def _apply_collision_children(
    nif: NifFile,
    host: NifBlock,
    collision_nodes: list[dict[str, Any]],
    fallback_node_doc: dict[str, Any] | None,
    warnings: list[str],
    profile,
) -> None:
    if collision_nodes:
        export_nodes = _coalesce_collision_nodes_for_export(collision_nodes)
        for index, collision_doc in enumerate(export_nodes):
            _apply_collision_geometry(
                nif=nif,
                host=host,
                node_doc=collision_doc,
                warnings=warnings,
                profile=profile,
                replace=index == 0,
                apply_local_transform=bool(
                    collision_doc.get("_mb21_apply_local_transform", True)
                ),
            )
        return

    if fallback_node_doc is None:
        return

    metadata = dict(fallback_node_doc.get("metadata") or {})
    collision = metadata.get("collision")
    if not isinstance(collision, dict) or not collision.get("enabled"):
        return
    if fallback_node_doc.get("type") != "mesh":
        return
    if not (fallback_node_doc.get("mesh") or {}).get("vertices"):
        return

    _apply_collision_geometry(
        nif=nif,
        host=host,
        node_doc=fallback_node_doc,
        warnings=warnings,
        profile=profile,
        replace=True,
        apply_local_transform=False,
    )


def _coalesce_collision_nodes_for_export(
    collision_nodes: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    grouped: list[dict[str, Any]] = []
    convex_groups: dict[tuple[Any, ...], dict[str, Any]] = {}
    order = 0

    for node_doc in collision_nodes:
        metadata = dict(node_doc.get("metadata") or {})
        collision = dict(metadata.get("collision") or {})
        shape_type = str(collision.get("shape_type") or "convex_hull")
        if not collision.get("enabled") or shape_type != "convex_hull":
            grouped.append(
                {
                    **node_doc,
                    "_mb21_apply_local_transform": True,
                    "_mb21_export_order": order,
                }
            )
            order += 1
            continue

        key = (
            shape_type,
            str(collision.get("layer") or "STATIC"),
            str(collision.get("material") or ""),
            float(collision.get("mass") or 0.0),
            float(collision.get("friction") or 0.5),
            float(collision.get("restitution") or 0.4),
            float(collision.get("radius") or DEFAULT_RADIUS),
        )
        group = convex_groups.get(key)
        if group is None:
            group = {
                "order": order,
                "template": node_doc,
                "vertices": [],
            }
            convex_groups[key] = group
            order += 1

        group["vertices"].extend(
            _collision_vertices_for_export(node_doc, apply_local_transform=True)
        )

    for group in convex_groups.values():
        template = group["template"]
        metadata = dict(template.get("metadata") or {})
        collision = dict(metadata.get("collision") or {})
        grouped.append(
            {
                **template,
                "mesh": {
                    "vertices": list(group["vertices"]),
                    "triangles": [],
                },
                "metadata": {
                    **metadata,
                    "collision": collision,
                },
                "_mb21_apply_local_transform": False,
                "_mb21_export_order": group["order"],
            }
        )

    grouped.sort(key=lambda item: int(item.get("_mb21_export_order", 0)))
    return grouped


def _apply_collision_geometry(
    nif: NifFile,
    host: NifBlock,
    node_doc: dict[str, Any],
    warnings: list[str],
    profile,
    *,
    replace: bool,
    apply_local_transform: bool,
) -> None:
    metadata = dict(node_doc.get("metadata") or {})
    collision = dict(metadata.get("collision") or {})
    if not collision.get("enabled"):
        return

    mesh = dict(node_doc.get("mesh") or {})
    vertices = _collision_vertices_for_export(
        node_doc, apply_local_transform=apply_local_transform
    )
    triangles = list(mesh.get("triangles") or [])
    if len(vertices) < 3:
        warnings.append(
            f"Skipped collision export for '{node_doc.get('name') or host.block_id}' because it has no usable geometry"
        )
        return

    shape_type = str(collision.get("shape_type") or "convex_hull")
    result = generate_collision_from_geometry(
        nif=nif,
        node_block_id=host.block_id,
        vertices=vertices,
        triangles=triangles,
        shape_type=shape_type,
        layer=str(collision.get("layer") or "STATIC"),
        material=collision.get("material"),
        mass=float(collision.get("mass") or 0.0),
        friction=float(collision.get("friction") or 0.5),
        restitution=float(collision.get("restitution") or 0.4),
        radius=float(collision.get("radius") or DEFAULT_RADIUS),
        replace=replace,
        profile=profile,
    )
    if not result.success:
        warnings.append(
            f"Failed to export collision for '{node_doc.get('name') or host.block_id}': {result.description}"
        )
    else:
        warnings.extend(result.warnings)


def _collision_vertices_for_export(
    node_doc: dict[str, Any],
    *,
    apply_local_transform: bool,
) -> list[dict[str, float]]:
    vertices = [
        _vector3(vertex)
        for vertex in (dict(node_doc.get("mesh") or {}).get("vertices") or [])
    ]
    if not apply_local_transform:
        return vertices

    transform = dict(node_doc.get("transform") or {})
    rotation = _matrix33(transform.get("rotation") or {})
    translation = _vector3(transform.get("translation") or {})
    scale = float(transform.get("scale", 1.0))
    transformed: list[dict[str, float]] = []
    for vertex in vertices:
        x = float(vertex["x"]) * scale
        y = float(vertex["y"]) * scale
        z = float(vertex["z"]) * scale
        transformed.append(
            {
                "x": rotation[0][0] * x
                + rotation[0][1] * y
                + rotation[0][2] * z
                + translation["x"],
                "y": rotation[1][0] * x
                + rotation[1][1] * y
                + rotation[1][2] * z
                + translation["y"],
                "z": rotation[2][0] * x
                + rotation[2][1] * y
                + rotation[2][2] * z
                + translation["z"],
            }
        )
    return transformed


def _apply_alpha_property(
    nif: NifFile, shape: NifBlock, node_doc: dict[str, Any]
) -> None:
    mesh = dict(node_doc.get("mesh") or {})
    alpha_doc = mesh.get("alpha_property")
    if alpha_doc is None:
        return

    if not bool(alpha_doc.get("enabled", True)):
        shape.set_field("Alpha Property", -1)
        return

    alpha = None
    source_id = alpha_doc.get("source_block_id")
    if isinstance(source_id, int):
        existing = nif.get_block(source_id)
        if existing is not None and existing.type_name == "NiAlphaProperty":
            alpha = existing
    if alpha is None:
        alpha = _ensure_alpha_property(nif, shape)
    else:
        shape.set_field("Alpha Property", alpha.block_id)

    alpha.set_field("Flags", int(alpha_doc.get("flags") or 4844))
    alpha.set_field("Threshold", int(alpha_doc.get("threshold") or 128))


def _apply_connect_point_helpers(
    nif: NifFile,
    owner: NifBlock,
    helpers: list[dict[str, Any]],
) -> None:
    if not helpers:
        return

    parent_points = [
        helper
        for helper in helpers
        if _helper_connect_point_type(helper) == "connect_point_parent"
    ]
    child_points = [
        helper
        for helper in helpers
        if _helper_connect_point_type(helper) == "connect_point_child"
    ]

    if parent_points:
        cp_parent_block = _ensure_extra_block(nif, owner, "BSConnectPoint::Parents")
        cp_parent_block.set_field("Num Connect Points", len(parent_points))
        cp_parent_block.set_field(
            "Connect Points",
            [
                {
                    "Parent": (helper.get("metadata") or {}).get("parent_name")
                    or "WorkshopConnectPoints",
                    "Name": helper.get("name") or "",
                    "Rotation": _quaternion(
                        (helper.get("transform") or {}).get("rotation") or {}
                    ),
                    "Translation": _vector3(
                        (helper.get("transform") or {}).get("translation") or {}
                    ),
                    "Scale": float((helper.get("transform") or {}).get("scale", 1.0)),
                }
                for helper in parent_points
            ],
        )

    if child_points:
        cp_child_block = _ensure_extra_block(nif, owner, "BSConnectPoint::Children")
        cp_child_block.set_field(
            "Skinned",
            any(
                bool((helper.get("metadata") or {}).get("skinned"))
                for helper in child_points
            ),
        )
        cp_child_block.set_field("Num Points", len(child_points))
        cp_child_block.set_field(
            "Point Name", [helper.get("name") or "" for helper in child_points]
        )


def _helper_connect_point_type(helper: dict[str, Any]) -> str:
    helper_type = str(helper.get("helper_type") or "")
    if helper_type:
        return helper_type
    name = str(helper.get("name") or "").upper()
    if name.startswith("C-"):
        return "connect_point_child"
    return "connect_point_parent"


def _apply_root_metadata(
    nif: NifFile, root: NifBlock, metadata: dict[str, Any]
) -> None:
    metadata = dict(metadata or {})
    if "bsx_flags" not in metadata and _has_animation_metadata(
        metadata.get("animations") or {}
    ):
        # 0x01 = Animated, 0x08 = Complex (controller graph). Matches
        # Safe01's BSXFlags=0x0B minus the Havok bit (we don't have collision).
        metadata["bsx_flags"] = 0x09
    _apply_node_extra_data(nif, root, {"metadata": metadata})


_BSX_HAVOK_BIT = 0x02
_COLLISION_BLOCK_TYPES = frozenset({"bhkNPCollisionObject", "bhkCollisionObject"})


def _ensure_bsx_havok_if_collision(nif: NifFile, root: NifBlock) -> None:
    # _apply_root_metadata writes BSXFlags from doc metadata after collision export
    # has already run, silently clearing the Havok bit. Re-apply it if the NIF
    # contains any collision object.
    if not any(block.type_name in _COLLISION_BLOCK_TYPES for block in nif.blocks):
        return
    bsx = _ensure_extra_block(nif, root, "BSXFlags")
    if not bsx.get_field("Name"):
        bsx.set_field("Name", "BSX")
    bsx.set_field("Integer Data", int(bsx.get_field("Integer Data") or 0) | _BSX_HAVOK_BIT)


def _snapshot_animation_export_state(nif: NifFile) -> dict[str, Any]:
    return {
        "shader_by_shape": {
            block.block_id: _ref_id(block.get_field("Shader Property"))
            for block in nif.blocks
            if any(nif.schema.is_subtype_of(block.type_name, base) for base in SHAPE_TYPES)
        },
        "palette_targets": {
            str(entry.get("Name") or ""): _ref_id(entry.get("AV Object"))
            for block in nif.blocks
            if block.type_name == "NiDefaultAVObjectPalette"
            for entry in (block.get_field("Objs") or [])
            if str(entry.get("Name") or "")
        },
    }


def _retarget_scene_animations(
    nif: NifFile,
    document: dict[str, Any],
    warnings: list[str],
    export_settings: dict[str, Any],
    export_state: dict[str, Any],
) -> None:
    animation_metadata = (
        ((document.get("metadata") or {}).get("root") or {}).get("animations") or {}
    )
    if _has_animation_metadata(animation_metadata):
        _apply_animation_metadata(
            nif=nif,
            animation_metadata=animation_metadata,
        )

    if not bool(export_settings.get("selected_roots")):
        shader_replacements = _shader_replacement_map(nif, export_state)
        if shader_replacements:
            _retarget_shader_controller_targets(nif, shader_replacements)
        _retarget_animation_palette_names(nif)
        _retarget_sequence_controlled_block_names(nif, export_state)


def _has_animation_metadata(metadata: dict[str, Any]) -> bool:
    # A bare `managers` entry without sequences/direct_controllers is stale
    # state left over after the user removed their sequences — emitting a
    # NiControllerManager + palette + transform controller in that case
    # produces an "animated" NIF with zero playable content, which crashes
    # the game loader. Require real animation content before promoting the
    # scene to animated.
    return any(metadata.get(key) for key in ("sequences", "direct_controllers"))


def _nif_has_animation_blocks(nif: NifFile) -> bool:
    return any(
        block.type_name in {"NiControllerManager", "NiControllerSequence"}
        or _is_shader_property_controller(block)
        for block in nif.blocks
    )


def _shader_replacement_map(
    nif: NifFile, export_state: dict[str, Any]
) -> dict[int, int]:
    before = dict(export_state.get("shader_by_shape") or {})
    after = {
        block.block_id: _ref_id(block.get_field("Shader Property"))
        for block in nif.blocks
        if any(nif.schema.is_subtype_of(block.type_name, base) for base in SHAPE_TYPES)
    }
    replacements: dict[int, int] = {}
    for shape_id, old_shader_id in before.items():
        new_shader_id = after.get(shape_id, -1)
        if old_shader_id >= 0 and new_shader_id >= 0 and old_shader_id != new_shader_id:
            replacements[old_shader_id] = new_shader_id
    return replacements


def _retarget_shader_controller_targets(
    nif: NifFile, shader_replacements: dict[int, int]
) -> None:
    for old_shader_id, new_shader_id in shader_replacements.items():
        old_shader = nif.get_block(old_shader_id)
        new_shader = nif.get_block(new_shader_id)
        if old_shader is None or new_shader is None:
            continue
        old_head = _ref_id(old_shader.get_field("Controller"))
        new_head = _ref_id(new_shader.get_field("Controller"))
        if old_head >= 0 and new_head < 0:
            new_shader.set_field("Controller", old_head)
            old_shader.set_field("Controller", -1)

    for block in nif.blocks:
        if not _is_shader_property_controller(block):
            continue
        target_id = _ref_id(block.get_field("Target"))
        replacement = shader_replacements.get(target_id)
        if replacement is not None:
            block.set_field("Target", replacement)


def _retarget_animation_palette_names(nif: NifFile) -> None:
    for block in nif.blocks:
        if block.type_name != "NiDefaultAVObjectPalette":
            continue
        entries = list(block.get_field("Objs") or [])
        changed = False
        for entry in entries:
            target_id = _ref_id(entry.get("AV Object"))
            if target_id < 0:
                continue
            target = nif.get_block(target_id)
            if target is None:
                continue
            target_name = str(target.get_field("Name") or "")
            if target_name and entry.get("Name") != target_name:
                entry["Name"] = target_name
                changed = True
        if changed:
            block.set_field("Objs", entries)


def _retarget_sequence_controlled_block_names(
    nif: NifFile, export_state: dict[str, Any]
) -> None:
    shader_owner_by_shader_id = _shader_owner_by_shader_id(nif)
    palette_targets = dict(export_state.get("palette_targets") or {})
    for block in nif.blocks:
        if block.type_name != "NiControllerSequence":
            continue
        controlled_blocks = list(block.get_field("Controlled Blocks") or [])
        changed = False
        for controlled in controlled_blocks:
            target_name = ""
            palette_target_id = palette_targets.get(str(controlled.get("Node Name") or ""))
            if palette_target_id is not None:
                target_name = _animation_target_name(
                    nif,
                    palette_target_id,
                    shader_owner_by_shader_id,
                )
            if not target_name:
                controller = nif.get_block(_ref_id(controlled.get("Controller")))
                if controller is None:
                    continue
                target_name = _animation_target_name(
                    nif,
                    _ref_id(controller.get_field("Target")),
                    shader_owner_by_shader_id,
                )
            if target_name and controlled.get("Node Name") != target_name:
                controlled["Node Name"] = target_name
                changed = True
        if changed:
            block.set_field("Controlled Blocks", controlled_blocks)


def _shader_owner_by_shader_id(nif: NifFile) -> dict[int, int]:
    owners: dict[int, int] = {}
    for block in nif.blocks:
        if not any(nif.schema.is_subtype_of(block.type_name, base) for base in SHAPE_TYPES):
            continue
        shader_id = _ref_id(block.get_field("Shader Property"))
        if shader_id >= 0:
            owners[shader_id] = block.block_id
    return owners


def _animation_target_name(
    nif: NifFile,
    target_id: int,
    shader_owner_by_shader_id: dict[int, int],
) -> str:
    owner_id = shader_owner_by_shader_id.get(target_id)
    if owner_id is not None:
        owner = nif.get_block(owner_id)
        if owner is not None:
            return str(owner.get_field("Name") or "")
    target = nif.get_block(target_id)
    if target is None:
        return ""
    return str(target.get_field("Name") or "")


def _apply_animation_metadata(
    nif: NifFile,
    animation_metadata: dict[str, Any],
) -> None:
    root = nif.get_block(0)
    if root is None:
        return

    node_name_to_id = {
        str(block.get_field("Name") or ""): block.block_id
        for block in nif.blocks
        if str(block.get_field("Name") or "")
    }
    manager_doc = ((animation_metadata.get("managers") or [{}])[0]) if (animation_metadata.get("sequences") or []) else {}
    manager = _ensure_controller_manager(nif, root, manager_doc)
    palette = _ensure_object_palette(nif, manager, root, manager_doc)
    transform_controller = _ensure_transform_manager_controller(nif, manager, root)

    sequence_ids: list[int] = []
    claimed_sequence_block_ids: set[int] = set()
    claimed_typed_block_ids: set[int] = set()
    palette_entries = {
        str(entry.get("Name") or ""): dict(entry)
        for entry in (palette.get_field("Objs") or [])
        if str(entry.get("Name") or "")
    }

    for index, sequence_doc in enumerate(animation_metadata.get("sequences") or [], start=1):
        sequence = _ensure_sequence_block(
            nif, sequence_doc, claimed=claimed_sequence_block_ids
        )
        sequence_ids.append(sequence.block_id)
        sequence.set_field("Name", sequence_doc.get("name") or f"sequence_{index}")
        sequence.set_field("Manager", manager.block_id)
        sequence.set_field("Cycle Type", sequence_doc.get("cycle_type") or "CYCLE_CLAMP")
        sequence.set_field("Accum Root Name", sequence_doc.get("accum_root_name") or "")
        sequence.set_field("Start Time", float(sequence_doc.get("start_time") or 0.0))
        sequence.set_field("Stop Time", float(sequence_doc.get("stop_time") or 0.0))
        # Runtime defaults — without Frequency=1.0 + Weight=1.0 the
        # sequence has no contribution and animation never plays.
        sequence.set_field("Frequency", 1.0)
        sequence.set_field("Weight", 1.0)
        sequence.set_field("Array Grow By", 1)

        if "text_keys" in sequence_doc:
            text_key_block = _ensure_text_key_block(nif, sequence_doc)
            text_keys = [
                {
                    "Time": float(entry.get("time") or 0.0),
                    "Value": str(entry.get("value") or ""),
                }
                for entry in (sequence_doc.get("text_keys") or [])
            ]
            text_key_block.set_field("Num Text Keys", len(text_keys))
            text_key_block.set_field("Text Keys", text_keys)
            sequence.set_field("Text Keys", text_key_block.block_id)

        controlled_input = list(sequence_doc.get("controlled_blocks") or [])
        if not controlled_input:
            existing_blocks = list(sequence.get_field("Controlled Blocks") or [])
            if existing_blocks:
                continue

        controlled_blocks: list[dict[str, Any]] = []
        for controlled in controlled_input:
            channel_type = str(controlled.get("channel_type") or "")
            node_name = str(controlled.get("node_name") or controlled.get("target_name") or "")
            target_block_id = node_name_to_id.get(node_name, -1)
            if channel_type == "transform":
                interpolator = _ensure_typed_block(
                    nif,
                    controlled.get("interpolator_block_id"),
                    "NiTransformInterpolator",
                    claimed=claimed_typed_block_ids,
                )
                data_block = _ensure_typed_block(
                    nif,
                    controlled.get("data_block_id"),
                    "NiTransformData",
                    claimed=claimed_typed_block_ids,
                )
                interpolator.set_field("Data", data_block.block_id)
                _apply_transform_interpolator_runtime_defaults(interpolator)
                _write_transform_data_block(data_block, controlled)
                controlled_blocks.append(
                    {
                        "Interpolator": interpolator.block_id,
                        "Controller": transform_controller.block_id,
                        "Priority": int(controlled.get("priority") or 0),
                        "Node Name": node_name,
                        "Property Type": controlled.get("property_type") or None,
                        "Controller Type": controlled.get("controller_type") or "NiTransformController",
                        "Controller ID": controlled.get("controller_id") or None,
                        "Interpolator ID": controlled.get("interpolator_id") or None,
                    }
                )
                if target_block_id >= 0:
                    palette_entries[node_name] = {"Name": node_name, "AV Object": target_block_id}
            elif channel_type == "float":
                controller = _ensure_float_controller_for_node(
                    nif,
                    controlled=controlled,
                    node_name=node_name,
                    node_name_to_id=node_name_to_id,
                )
                if controller is None:
                    continue
                interpolator = _ensure_typed_block(
                    nif,
                    controlled.get("interpolator_block_id"),
                    "NiFloatInterpolator",
                    claimed=claimed_typed_block_ids,
                )
                data_block = _ensure_typed_block(
                    nif,
                    controlled.get("data_block_id"),
                    "NiFloatData",
                    claimed=claimed_typed_block_ids,
                )
                interpolator.set_field("Data", data_block.block_id)
                _write_float_data_block(data_block, controlled)
                controller.set_field("Interpolator", interpolator.block_id)
                controller.set_field(
                    "Controlled Variable", controlled.get("controlled_variable") or ""
                )
                controlled_blocks.append(
                    {
                        "Interpolator": interpolator.block_id,
                        "Controller": controller.block_id,
                        "Priority": int(controlled.get("priority") or 0),
                        "Node Name": node_name,
                        "Property Type": controlled.get("property_type") or "",
                        "Controller Type": controlled.get("controller_type") or controller.type_name,
                        "Controller ID": controlled.get("controller_id") or None,
                        "Interpolator ID": controlled.get("interpolator_id") or None,
                    }
                )
                if target_block_id >= 0:
                    palette_entries[node_name] = {"Name": node_name, "AV Object": target_block_id}
        sequence.set_field("Num Controlled Blocks", len(controlled_blocks))
        sequence.set_field("Controlled Blocks", controlled_blocks)

    if sequence_ids:
        manager.set_field("Num Controller Sequences", len(sequence_ids))
        manager.set_field("Controller Sequences", sequence_ids)
    if palette_entries:
        objs = list(palette_entries.values())
        palette.set_field("Num Objs", len(objs))
        palette.set_field("Objs", objs)
        # NiMultiTargetTransformController.Extra Targets is the list of
        # animated node block ids. Pulling them from palette entries gives
        # us exactly the animated AVObjects. Bethesda pads the list with
        # -1 to a fixed size; we use the populated count for safety.
        target_ids = [int(entry.get("AV Object", -1)) for entry in objs]
        target_ids = [tid for tid in target_ids if tid >= 0]
        if target_ids:
            transform_controller.set_field("Num Extra Targets", len(target_ids))
            transform_controller.set_field("Extra Targets", target_ids)

    for direct_doc in animation_metadata.get("direct_controllers") or []:
        controller = _ensure_float_controller_for_node(
            nif,
            controlled=direct_doc,
            node_name=str(direct_doc.get("target_name") or ""),
            node_name_to_id=node_name_to_id,
        )
        if controller is None:
            continue
        interpolator = _ensure_typed_block(
            nif,
            direct_doc.get("interpolator_block_id"),
            "NiFloatInterpolator",
            claimed=claimed_typed_block_ids,
        )
        data_block = _ensure_typed_block(
            nif,
            direct_doc.get("data_block_id"),
            "NiFloatData",
            claimed=claimed_typed_block_ids,
        )
        interpolator.set_field("Data", data_block.block_id)
        _write_float_data_block(data_block, direct_doc)
        controller.set_field("Interpolator", interpolator.block_id)
        controller.set_field(
            "Controlled Variable", direct_doc.get("controlled_variable") or ""
        )


def _ensure_typed_block(
    nif: NifFile,
    block_id: Any,
    type_name: str,
    *,
    claimed: set[int] | None = None,
) -> NifBlock:
    # Two sequence_docs can carry the same stamped interpolator/data
    # block_id when round-tripped metadata aliases them across sequences.
    # Reusing the same NiTransformInterpolator/NiTransformData makes the
    # later sequence's _write_*_data_block call overwrite the earlier
    # sequence's keys (last writer wins), and both NiControllerSequences
    # end up referencing the same block. Claim a typed block on first
    # match; force fresh allocation for subsequent docs so each sequence
    # gets its own interpolator + data slot.
    if isinstance(block_id, int) and (claimed is None or block_id not in claimed):
        block = nif.get_block(int(block_id))
        if block is not None and block.type_name == type_name:
            if claimed is not None:
                claimed.add(block_id)
            return block
    new_block = nif.add_block(type_name)
    if claimed is not None:
        claimed.add(new_block.block_id)
    return new_block


def _ensure_controller_manager(
    nif: NifFile,
    root: NifBlock,
    manager_doc: dict[str, Any],
) -> NifBlock:
    manager_id = manager_doc.get("source_block_id")
    manager = nif.get_block(int(manager_id)) if isinstance(manager_id, int) else None
    fresh = manager is None or manager.type_name != "NiControllerManager"
    if fresh:
        manager = nif.add_block("NiControllerManager")
        _apply_controller_runtime_defaults(manager, flags=76)
    manager.set_field("Target", root.block_id)
    root.set_field("Controller", manager.block_id)
    return manager


def _apply_controller_runtime_defaults(controller: NifBlock, *, flags: int) -> None:
    """Set the runtime defaults Bethesda controllers expect: Frequency=1,
    proper Flags, and ±FLT_MAX as the un-set marker for Start/Stop Time.
    Without these, the animation runtime treats Frequency=0 as 'don't play'."""
    FLT_MAX = 3.4028234663852886e38
    controller.set_field("Flags", flags)
    controller.set_field("Frequency", 1.0)
    controller.set_field("Phase", 0.0)
    controller.set_field("Start Time", FLT_MAX)
    controller.set_field("Stop Time", -FLT_MAX)


def _apply_transform_interpolator_runtime_defaults(interpolator: NifBlock) -> None:
    sentinel = -FLT_MAX
    interpolator.set_field(
        "Transform",
        {
            "Translation": {"x": sentinel, "y": sentinel, "z": sentinel},
            "Rotation": {
                "w": sentinel,
                "x": sentinel,
                "y": sentinel,
                "z": sentinel,
            },
            "Scale": sentinel,
        },
    )


def _ensure_object_palette(
    nif: NifFile,
    manager: NifBlock,
    root: NifBlock,
    manager_doc: dict[str, Any],
) -> NifBlock:
    palette_id = manager_doc.get("object_palette_block_id")
    palette = nif.get_block(int(palette_id)) if isinstance(palette_id, int) else None
    if palette is None or palette.type_name != "NiDefaultAVObjectPalette":
        palette = nif.add_block("NiDefaultAVObjectPalette")
    palette.set_field("Scene", root.block_id)
    manager.set_field("Object Palette", palette.block_id)
    return palette


def _ensure_transform_manager_controller(
    nif: NifFile,
    manager: NifBlock,
    root: NifBlock,
) -> NifBlock:
    block = nif.get_block(_ref_id(manager.get_field("Next Controller")))
    if block is None or block.type_name != "NiMultiTargetTransformController":
        block = nif.add_block("NiMultiTargetTransformController")
        manager.set_field("Next Controller", block.block_id)
        _apply_controller_runtime_defaults(block, flags=108)
    block.set_field("Target", root.block_id)
    return block


def _ensure_sequence_block(
    nif: NifFile,
    sequence_doc: dict[str, Any],
    *,
    claimed: set[int] | None = None,
) -> NifBlock:
    # Two sequence_docs can share a source_block_id when round-tripped
    # metadata duplicates an entry. Reusing the same NiControllerSequence
    # makes the second writer call overwrite the first sequence's text
    # keys + controlled blocks, stranding the originals as orphan blocks
    # in the file. Claim the source block on first match; force fresh
    # allocation for subsequent docs so each sequence gets its own slot.
    block_id = sequence_doc.get("source_block_id")
    if isinstance(block_id, int) and (claimed is None or block_id not in claimed):
        block = nif.get_block(int(block_id))
        if block is not None and block.type_name == "NiControllerSequence":
            if claimed is not None:
                claimed.add(block_id)
            return block
    new_block = nif.add_block("NiControllerSequence")
    if claimed is not None:
        claimed.add(new_block.block_id)
    return new_block


def _ensure_text_key_block(nif: NifFile, sequence_doc: dict[str, Any]) -> NifBlock:
    block_id = sequence_doc.get("text_key_block_id")
    block = nif.get_block(int(block_id)) if isinstance(block_id, int) else None
    if block is not None and block.type_name == "NiTextKeyExtraData":
        return block
    return nif.add_block("NiTextKeyExtraData")


def _ensure_float_controller_for_node(
    nif: NifFile,
    *,
    controlled: dict[str, Any],
    node_name: str,
    node_name_to_id: dict[str, int],
) -> NifBlock | None:
    node_id = node_name_to_id.get(node_name, -1)
    if node_id < 0:
        return None
    node = nif.get_block(node_id)
    if node is None:
        return None
    shader_id = _ref_id(node.get_field("Shader Property"))
    shader = nif.get_block(shader_id)
    if shader is None:
        return None
    controller_type = str(controlled.get("type") or controlled.get("controller_type") or "")
    if controller_type not in {
        "BSLightingShaderPropertyFloatController",
        "BSEffectShaderPropertyFloatController",
    }:
        controller_type = (
            "BSEffectShaderPropertyFloatController"
            if shader.type_name == "BSEffectShaderProperty"
            else "BSLightingShaderPropertyFloatController"
        )
    controller_id = controlled.get("controller_block_id") or controlled.get("source_block_id")
    controller = nif.get_block(int(controller_id)) if isinstance(controller_id, int) else None
    if controller is None or controller.type_name != controller_type:
        controller = nif.add_block(controller_type)
    controller.set_field("Target", shader.block_id)
    _attach_shader_controller(nif, shader, controller)
    return controller


def _attach_shader_controller(
    nif: NifFile, shader: NifBlock, controller: NifBlock
) -> None:
    head_id = _ref_id(shader.get_field("Controller"))
    if head_id == controller.block_id:
        return
    if _controller_in_chain(nif, head_id, controller.block_id):
        shader.set_field("Controller", head_id)
        return
    controller.set_field("Next Controller", head_id)
    shader.set_field("Controller", controller.block_id)


def _controller_in_chain(nif: NifFile, head_id: int, controller_id: int) -> bool:
    current_id = head_id
    visited: set[int] = set()
    while current_id >= 0 and current_id not in visited:
        if current_id == controller_id:
            return True
        visited.add(current_id)
        current = nif.get_block(current_id)
        if current is None:
            break
        current_id = _ref_id(current.get_field("Next Controller"))
    return False


def _write_transform_data_block(data_block: NifBlock, controlled: dict[str, Any]) -> None:
    translation_keys = [
        {
            "Time": float(entry.get("time") or 0.0),
            "Value": _vector3(entry.get("value") or {}),
        }
        for entry in (controlled.get("translation_keys") or [])
    ]
    rotation_keys = dict(controlled.get("rotation_keys") or {})
    xyz_rotations: list[dict[str, Any]] = []
    has_xyz_rotation_keys = False
    for axis_name in ("x", "y", "z"):
        axis_keys = [
            {
                "Time": float(entry.get("time") or 0.0),
                "Value": float(entry.get("value") or 0.0),
            }
            for entry in (rotation_keys.get(axis_name) or [])
        ]
        if axis_keys:
            has_xyz_rotation_keys = True
        xyz_rotations.append(
            {
                "Num Keys": len(axis_keys),
                "Interpolation": 1,
                "Keys": axis_keys,
            }
        )
    scale_keys = [
        {
            "Time": float(entry.get("time") or 0.0),
            "Value": float(entry.get("value") or 1.0),
        }
        for entry in (controlled.get("scale_keys") or [])
    ]
    data_block.set_field("Num Rotation Keys", 1 if has_xyz_rotation_keys else 0)
    data_block.set_field("Rotation Type", 4 if has_xyz_rotation_keys else 0)
    data_block.set_field("XYZ Rotations", xyz_rotations)
    data_block.set_field(
        "Translations",
        {
            "Num Keys": len(translation_keys),
            "Interpolation": 1,
            "Keys": translation_keys,
        },
    )
    data_block.set_field(
        "Scales",
        {
            "Num Keys": len(scale_keys),
            "Interpolation": 1,
            "Keys": scale_keys,
        },
    )


def _write_float_data_block(data_block: NifBlock, controlled: dict[str, Any]) -> None:
    float_keys = [
        {
            "Time": float(entry.get("time") or 0.0),
            "Value": float(entry.get("value") or 0.0),
        }
        for entry in (controlled.get("float_keys") or [])
    ]
    data_block.set_field(
        "Data",
        {
            "Num Keys": len(float_keys),
            "Interpolation": int(controlled.get("interpolation") or 1),
            "Keys": float_keys,
        },
    )


def _apply_material_binding(
    nif: NifFile,
    game: str,
    shape: NifBlock,
    node_doc: dict[str, Any],
    materials_by_id: dict[str, dict[str, Any]],
    output_path: Path,
    warnings: list[str],
) -> None:
    binding = dict(node_doc.get("material_binding") or {})
    material_id = binding.get("material_id")
    if not material_id:
        return

    material_doc = dict(materials_by_id.get(str(material_id)) or {})
    material_type = str(material_doc.get("material_type") or "bgsm").lower()
    shader = _ensure_shader_block(nif, shape, material_type)
    fields = dict(material_doc.get("fields") or {})
    desired_path = str(binding.get("path") or material_doc.get("path") or "").replace(
        "\\", "/"
    )
    mode = str(binding.get("mode") or material_doc.get("mode") or "linked")
    supports_documents = _scene_supports_material_documents(game)

    if not supports_documents and mode != "inline":
        warnings.append(
            f"Scene game '{game}' uses inline shader materials; exporting material on "
            f"'{node_doc.get('name') or shape.block_id}' as inline."
        )
        mode = "inline"

    if mode == "inline":
        if supports_documents:
            if _looks_like_material_document_path(desired_path):
                write_material_document(
                    {
                        "material_type": material_type,
                        "version": int(material_doc.get("version") or 2),
                        "fields": fields,
                    },
                    output_path=str(output_path.parent / desired_path),
                )
            elif desired_path:
                warnings.append(
                    f"Material path '{desired_path}' on "
                    f"'{node_doc.get('name') or shape.block_id}' is not a BGSM/BGEM file; "
                    "keeping material inline."
                )
                desired_path = ""
        else:
            if _looks_like_material_document_path(desired_path):
                warnings.append(
                    f"Discarded external material path '{desired_path}' on "
                    f"'{node_doc.get('name') or shape.block_id}' because game '{game}' "
                    "expects inline shader materials."
                )
                desired_path = ""
    elif not desired_path:
        warnings.append(
            f"Material on '{node_doc.get('name') or shape.block_id}' has no linked path; leaving shader name unchanged"
        )
        desired_path = str(shader.get_field("Name") or "")

    shader.set_field("Name", desired_path)
    if material_type == "bgsm":
        _apply_bgsm_fields_to_shader(nif, shader, fields)
        _apply_shader_nif_fields(shader, fields)
    else:
        _apply_bgem_fields_to_shader(shader, fields)


def _apply_vertex_color_shader_flags(
    nif: NifFile, shape: NifBlock, node_doc: dict[str, Any]
) -> None:
    mesh = dict(node_doc.get("mesh") or {})
    if (
        "colors" not in mesh
        and mesh.get("preserve_source_geometry")
        and not mesh.get("rewrite_geometry")
    ):
        return

    shader_id = _ref_id(shape.get_field("Shader Property"))
    if shader_id < 0:
        return

    shader = nif.get_block(shader_id)
    if shader is None or "ShaderProperty" not in shader.type_name:
        return

    colors = list(mesh.get("colors") or [])
    flags1 = int(shader.get_field("Shader Flags 1") or 0)
    flags2 = int(shader.get_field("Shader Flags 2") or 0)

    if colors:
        flags2 |= SHADER_FLAG2_VERTEX_COLORS
        if any(abs(float(color.get("a", 1.0)) - 1.0) > 1e-6 for color in colors):
            flags1 |= SHADER_FLAG1_VERTEX_ALPHA
        else:
            flags1 &= ~SHADER_FLAG1_VERTEX_ALPHA
    else:
        flags2 &= ~SHADER_FLAG2_VERTEX_COLORS
        flags1 &= ~SHADER_FLAG1_VERTEX_ALPHA

    shader.set_field("Shader Flags 1", flags1)
    shader.set_field("Shader Flags 2", flags2)


def _ensure_shader_block(nif: NifFile, shape: NifBlock, material_type: str) -> NifBlock:
    desired_type = (
        "BSEffectShaderProperty"
        if material_type == "bgem"
        else "BSLightingShaderProperty"
    )
    shader_id = _ref_id(shape.get_field("Shader Property"))
    shader = nif.get_block(shader_id) if shader_id >= 0 else None
    if shader is None or shader.type_name != desired_type:
        shader = nif.add_block(desired_type)
        shape.set_field("Shader Property", shader.block_id)
        if desired_type == "BSLightingShaderProperty":
            _apply_lighting_shader_defaults(shader)
    return shader


def _apply_lighting_shader_defaults(shader: NifBlock) -> None:
    """Set Bethesda-correct render defaults on a freshly created
    BSLightingShaderProperty. Values mirror Safe01.nif's typical static-mesh
    shader. Without these the shape renders as fully transparent (Alpha=0)
    with no UVs (UV Scale=0,0), or with bright white emission/specular."""
    shader.set_field("Shader Type", 0)  # SHADER_DEFAULT
    shader.set_field("Shader Flags 1", LIGHTING_SHADER_DEFAULT_FLAGS1)
    shader.set_field("Shader Flags 2", LIGHTING_SHADER_DEFAULT_FLAGS2)
    shader.set_field("UV Scale", {"u": 1.0, "v": 1.0})
    shader.set_field("Texture Clamp Mode", 3)  # WRAP_S_WRAP_T
    shader.set_field("Alpha", 1.0)
    shader.set_field("Smoothness", 0.5)
    # Black emission with multiplier 1 = no self-emission.
    shader.set_field("Emissive Color", {"r": 0.0, "g": 0.0, "b": 0.0})
    shader.set_field("Emissive Multiple", 1.0)
    # Slightly cool, moderate specular — typical FO4 static-mesh values.
    shader.set_field("Specular Color", {"r": 0.882, "g": 0.894, "b": 0.898})
    shader.set_field("Specular Strength", 0.5)
    shader.set_field("Subsurface Rolloff", 0.3)
    # Rimlight=FLT_MAX is the "rim lighting disabled" sentinel and is
    # load-bearing for FO4 — see the FLT_MAX comment above. Backlight is
    # gated on Rimlight=FLT_MAX in the schema, so we must set both together.
    shader.set_field("Rimlight Power", FLT_MAX)
    shader.set_field("Backlight Power", 0.0)
    shader.set_field("Grayscale to Palette Scale", 1.0)
    shader.set_field("Fresnel Power", 5.0)
    # -1.0 sentinels mean "no wetness" (matches Bethesda export).
    shader.set_field("Wetness", {
        "Spec Scale": -1.0, "Spec Power": -1.0, "Min Var": -1.0,
        "Env Map Scale": -1.0, "Fresnel Power": -1.0, "Metalness": -1.0,
    })


def _apply_bgsm_fields_to_shader(
    nif: NifFile, shader: NifBlock, fields: dict[str, Any]
) -> None:
    # Only override fields the BGSM actually specifies. _color3_tuple(None)
    # returns white (1,1,1), so unconditionally calling set_field on an
    # unspecified EmittanceColor would clobber the (0,0,0) default with white
    # and the mesh self-emits → renders bright white in NifSkope.
    #
    # Emittance is gated on EmitEnabled: Max's mb21 material defaults the
    # emittance color to white (255,255,255) even when emittance is disabled,
    # so the BGSM serialization always carries a white EmittanceColor for
    # default materials. Honoring it unconditionally produces a bright white
    # glow on every mesh that hasn't explicitly enabled emittance.
    shader.set_field("Root Material", fields.get("RootMaterialPath") or "")
    emit_enabled = bool(fields.get("EmitEnabled") or False)
    if emit_enabled and "EmittanceColor" in fields:
        shader.set_field("Emissive Color", _tuple_to_color3(fields.get("EmittanceColor")))
    if emit_enabled and "EmittanceMult" in fields:
        shader.set_field("Emissive Multiple", float(fields.get("EmittanceMult") or 0.0))
    spec_enabled = fields.get("SpecularEnabled")
    if spec_enabled is None or bool(spec_enabled):
        if "SpecularColor" in fields:
            shader.set_field("Specular Color", _tuple_to_color3(fields.get("SpecularColor")))
        if "SpecularMult" in fields:
            shader.set_field("Specular Strength", float(fields.get("SpecularMult") or 0.0))
    if "Smoothness" in fields:
        shader.set_field("Smoothness", float(fields.get("Smoothness") or 0.0))
    if "FresnelPower" in fields:
        shader.set_field("Fresnel Power", float(fields.get("FresnelPower") or 5.0))
    if "GrayscaleToPaletteScale" in fields:
        shader.set_field(
            "Grayscale to Palette Scale",
            float(fields.get("GrayscaleToPaletteScale") or 1.0),
        )

    # Rim/Back lighting must always be applied together: BSLightingShaderProperty
    # writes Backlight Power only when Rimlight Power == FLT_MAX (per schema),
    # and FO4 reads Backlight Power unconditionally — so emit them as a pair.
    # When BGSM says RimLighting=false, push FLT_MAX (the "rim disabled"
    # sentinel) so Backlight Power still serializes.
    rim_enabled = bool(fields.get("RimLighting") or False)
    if rim_enabled and "RimPower" in fields:
        shader.set_field("Rimlight Power", float(fields.get("RimPower") or 0.0))
    else:
        shader.set_field("Rimlight Power", FLT_MAX)
    if "BackLightPower" in fields:
        shader.set_field("Backlight Power", float(fields.get("BackLightPower") or 0.0))

    if "SubsurfaceLightingRolloff" in fields:
        shader.set_field(
            "Subsurface Rolloff",
            float(fields.get("SubsurfaceLightingRolloff") or 0.3),
        )

    # Environment Map Scale only matters on BSLightingShaderProperty when
    # Shader Type == 1 (Environment Map). The Max-side value lives under
    # different field names depending on the BGSM/BGEM version:
    #   - BGSM v < 10: header field `env_mapping_mask_scale` (snake_case),
    #     spinner labeled "Environment Mask Scale" in General Rendering.
    #   - BGEM v >= 10: dataclass field `EnvironmentMappingMaskScale`
    #     (CamelCase), spinner labeled "Env Mapping Mask Scale" in Advanced.
    # BGSM v >= 10 has no equivalent material-side field at all, so the
    # shader keeps whatever schema default for Type 1 (usually 0).
    env_scale = (
        fields.get("EnvironmentMappingMaskScale")
        if "EnvironmentMappingMaskScale" in fields
        else fields.get("env_mapping_mask_scale")
        if "env_mapping_mask_scale" in fields
        else None
    )
    if env_scale is not None:
        shader.set_field("Environment Map Scale", float(env_scale))

    texture_set_id = _ref_id(shader.get_field("Texture Set"))
    texture_set = nif.get_block(texture_set_id) if texture_set_id >= 0 else None
    if texture_set is None or texture_set.type_name != "BSShaderTextureSet":
        texture_set = nif.add_block("BSShaderTextureSet")
        shader.set_field("Texture Set", texture_set.block_id)

    # FO4 BSShaderTextureSet expects 10 slots (vanilla Safe01.nif Num Textures=10).
    # Writing only 8 leaves the runtime reading garbage for slots 8/9 — symptom
    # is the shader silently failing to load and the shape rendering invisible
    # (collision still works because the bhkPhysicsSystem doesn't go through
    # the visual shader). Pad to 10 with empty strings for Wrinkles + Detail.
    textures = [
        fields.get("DiffuseTexture") or "",
        fields.get("NormalTexture") or "",
        fields.get("SmoothSpecTexture") or "",
        fields.get("GreyscaleTexture") or "",
        fields.get("EnvmapTexture") or "",
        fields.get("GlowTexture") or "",
        fields.get("InnerLayerTexture") or "",
        fields.get("SpecularTexture") or "",
        fields.get("WrinklesTexture") or "",
        fields.get("DisplacementTexture") or "",
    ]
    texture_set.set_field("Num Textures", len(textures))
    texture_set.set_field("Textures", textures)


# Fallout4ShaderPropertyFlags1/2 bit definitions (nif.xml). Keys mirror the
# `nif_sf1_<lowercased>` / `nif_sf2_<lowercased>` attrs emitted by the Max
# material UI's Shader Flags 1 / Shader Flags 2 sections.
_F4_SHADER_FLAGS_1: tuple[tuple[str, int], ...] = (
    ("Specular", 0), ("Skinned", 1), ("Temp_Refraction", 2), ("Vertex_Alpha", 3),
    ("GS_To_Palette_Color", 4), ("GS_To_Palette_Alpha", 5), ("Use_Falloff", 6),
    ("Environment_Mapping", 7), ("RGB_Falloff", 8), ("Cast_Shadows", 9),
    ("Face", 10), ("UI_Mask_Rects", 11), ("Model_Space_Normals", 12),
    ("Non_Projective_Shadows", 13), ("Landscape", 14), ("Refraction", 15),
    ("Fire_Refraction", 16), ("Eye_Env_Mapping", 17), ("Hair", 18),
    ("Screendoor_Alpha_Fade", 19), ("Localmap_Hide_Secret", 20), ("Skin_Tint", 21),
    ("Own_Emit", 22), ("Projected_UV", 23), ("Multiple_Textures", 24),
    ("Tessellate", 25), ("Decal", 26), ("Dynamic_Decal", 27),
    ("Character_Lighting", 28), ("External_Emittance", 29), ("Soft_Effect", 30),
    ("ZBuffer_Test", 31),
)
_F4_SHADER_FLAGS_2: tuple[tuple[str, int], ...] = (
    ("ZBuffer_Write", 0), ("LOD_Landscape", 1), ("LOD_Objects", 2), ("No_Fade", 3),
    ("Double_Sided", 4), ("Vertex_Colors", 5), ("Glow_Map", 6), ("Transform_Changed", 7),
    ("Dismemberment_Meatcuff", 8), ("Tint", 9), ("Grass_Vertex_Lighting", 10),
    ("Grass_Uniform_Scale", 11), ("Grass_Fit_Slope", 12), ("Grass_Billboard", 13),
    ("No_LOD_Land_Blend", 14), ("Dismemberment", 15), ("Wireframe", 16),
    ("Weapon_Blood", 17), ("Hide_On_Local_Map", 18), ("Premult_Alpha", 19),
    ("VATS_Target", 20), ("Anisotropic_Lighting", 21), ("Skew_Specular_Alpha", 22),
    ("Menu_Screen", 23), ("Multi_Layer_Parallax", 24), ("Alpha_Test", 25),
    ("Gradient_Remap", 26), ("VATS_Target_Draw_All", 27), ("Pipboy_Screen", 28),
    ("Tree_Anim", 29), ("Effect_Lighting", 30), ("Refraction_Writes_Depth", 31),
)
_SHADER_FLAGS_1_BIT_BY_ATTR: dict[str, int] = {
    f"nif_sf1_{name.lower()}": bit for name, bit in _F4_SHADER_FLAGS_1
}
_SHADER_FLAGS_2_BIT_BY_ATTR: dict[str, int] = {
    f"nif_sf2_{name.lower()}": bit for name, bit in _F4_SHADER_FLAGS_2
}


def _apply_shader_nif_fields(shader: NifBlock, fields: dict[str, Any]) -> None:
    """Apply BSLightingShaderProperty fields (Shader Type, Shader Flags 1/2)
    from the Max material's Shader section.

    The Max-side `nif_shader_type` is a 1-based dropdownList selection
    (1 = "Default" = enum 0). Per-bit bool attrs `nif_sf1_<name>` /
    `nif_sf2_<name>` are re-assembled into the two 32-bit flag words.

    Only writes a flag word if at least one of its constituent attrs is
    present in `fields` — otherwise the default-flags set by
    `_apply_lighting_shader_defaults` is preserved.
    """
    raw_type = fields.get("nif_shader_type")
    if raw_type is not None:
        try:
            type_value = max(0, int(raw_type) - 1)
        except (TypeError, ValueError):
            type_value = 0
        shader.set_field("Shader Type", type_value)

    sf1, has_sf1 = 0, False
    for attr, bit in _SHADER_FLAGS_1_BIT_BY_ATTR.items():
        if attr in fields:
            has_sf1 = True
            if fields[attr]:
                sf1 |= 1 << bit
    if has_sf1:
        shader.set_field("Shader Flags 1", sf1)

    sf2, has_sf2 = 0, False
    for attr, bit in _SHADER_FLAGS_2_BIT_BY_ATTR.items():
        if attr in fields:
            has_sf2 = True
            if fields[attr]:
                sf2 |= 1 << bit
    if has_sf2:
        shader.set_field("Shader Flags 2", sf2)


def _apply_bgem_fields_to_shader(shader: NifBlock, fields: dict[str, Any]) -> None:
    base_color = fields.get("BaseColor") or (1.0, 1.0, 1.0)
    shader.set_field("Source Texture", fields.get("BaseTexture") or "")
    shader.set_field("Greyscale Texture", fields.get("GrayscaleTexture") or "")
    shader.set_field("Env Map Texture", fields.get("EnvmapTexture") or "")
    shader.set_field("Normal Texture", fields.get("NormalTexture") or "")
    shader.set_field("Env Mask Texture", fields.get("EnvmapMaskTexture") or "")
    shader.set_field(
        "Base Color",
        {
            "r": float(base_color[0]),
            "g": float(base_color[1]),
            "b": float(base_color[2]),
            "a": 1.0,
        },
    )
    shader.set_field("Base Color Scale", float(fields.get("BaseColorScale") or 1.0))
    shader.set_field(
        "Falloff Start Angle", float(fields.get("FalloffStartAngle") or 1.0)
    )
    shader.set_field("Falloff Stop Angle", float(fields.get("FalloffStopAngle") or 1.0))
    shader.set_field(
        "Falloff Start Opacity", float(fields.get("FalloffStartOpacity") or 0.0)
    )
    shader.set_field(
        "Falloff Stop Opacity", float(fields.get("FalloffStopOpacity") or 0.0)
    )
    shader.set_field("Lighting Influence", int(fields.get("LightingInfluence") or 0))
    shader.set_field("Env Map Min LOD", int(fields.get("EnvmapMinLOD") or 0))
    shader.set_field("Soft Falloff Depth", float(fields.get("SoftDepth") or 100.0))
    shader.set_field(
        "Environment Map Scale", float(fields.get("EnvironmentMappingMaskScale") or 1.0)
    )


def _flatten_material_data(data: BGSMData | BGEMData) -> dict[str, Any]:
    flattened: dict[str, Any] = {}
    for field in dc_fields(BaseHeader):
        if field.name in {"signature", "version"}:
            continue
        value = getattr(data.header, field.name)
        if value is not None:
            flattened[field.name] = _sanitize_material_value(value)
    for field in dc_fields(type(data)):
        if field.name == "header":
            continue
        value = getattr(data, field.name)
        if value is not None:
            flattened[field.name] = _sanitize_material_value(value)
    return flattened


def _sanitize_material_value(value: Any) -> Any:
    if isinstance(value, str):
        return value.rstrip("\x00")
    return value


def _unflatten_bgsm(fields: dict[str, Any], version: int) -> BGSMData:
    material = _default_bgsm_data(version)
    for field in dc_fields(BaseHeader):
        if field.name in {"signature", "version"}:
            continue
        if field.name in fields:
            setattr(material.header, field.name, fields[field.name])
    for field in dc_fields(BGSMData):
        if field.name == "header":
            continue
        if field.name in fields:
            setattr(material, field.name, fields[field.name])
    return material


def _unflatten_bgem(fields: dict[str, Any], version: int) -> BGEMData:
    material = _default_bgem_data(version)
    for field in dc_fields(BaseHeader):
        if field.name in {"signature", "version"}:
            continue
        if field.name in fields:
            setattr(material.header, field.name, fields[field.name])
    for field in dc_fields(BGEMData):
        if field.name == "header":
            continue
        if field.name in fields:
            setattr(material, field.name, fields[field.name])
    return material


def _default_header(signature: int, version: int) -> BaseHeader:
    return BaseHeader(
        signature=signature,
        version=version,
        tile_u=False,
        tile_v=False,
        u_offset=0.0,
        v_offset=0.0,
        u_scale=1.0,
        v_scale=1.0,
        alpha=1.0,
        alpha_blend_mode0=0,
        alpha_blend_mode1=0,
        alpha_blend_mode2=0,
        alpha_test_ref=128,
        alpha_test=False,
        zbuffer_write=True,
        zbuffer_test=True,
        ssr=False,
        wet_ssr=False,
        decal=False,
        two_sided=False,
        decal_nofade=False,
        non_occluder=False,
        refraction=False,
        refraction_falloff=False,
        refraction_power=0.0,
        env_mapping=False,
        env_mapping_mask_scale=0.0,
        depth_bias=None if version < 10 else False,
        grayscale_to_palette_color=False,
        mask_writes=0 if version >= 6 else None,
    )


def _default_bgsm_data(version: int = 2) -> BGSMData:
    return BGSMData(
        header=_default_header(BGSM_SIGNATURE, version),
        DiffuseTexture="",
        NormalTexture="",
        SmoothSpecTexture="",
        GreyscaleTexture="",
        EnvmapTexture="",
        GlowTexture="",
        InnerLayerTexture="",
        WrinklesTexture="",
        DisplacementTexture="",
        SpecularTexture=None,
        LightingTexture=None,
        FlowTexture=None,
        DistanceFieldAlphaTexture=None,
        EnableEditorAlphaRef=False,
        RimLighting=False if version < 8 else None,
        RimPower=0.0 if version < 8 else None,
        BackLightPower=0.0 if version < 8 else None,
        SubsurfaceLighting=False if version < 8 else None,
        SubsurfaceLightingRolloff=0.0 if version < 8 else None,
        Translucency=False if version >= 8 else None,
        TranslucencyThickObject=False if version >= 8 else None,
        TranslucencyMixAlbedoWithSubsurfaceColor=False if version >= 8 else None,
        TranslucencySubsurfaceColor=(1.0, 1.0, 1.0) if version >= 8 else None,
        TranslucencyTransmissiveScale=0.0 if version >= 8 else None,
        TranslucencyTurbulence=0.0 if version >= 8 else None,
        SpecularEnabled=True,
        SpecularColor=(1.0, 1.0, 1.0),
        SpecularMult=1.0,
        Smoothness=1.0,
        FresnelPower=5.0,
        WetnessControlSpecScale=0.0,
        WetnessControlSpecPowerScale=0.0,
        WetnessControlSpecMinvar=0.0,
        WetnessControlEnvMapScale=0.0 if version < 10 else None,
        WetnessControlFresnelPower=0.0,
        WetnessControlMetalness=0.0,
        PBR=False if version > 2 else None,
        CustomPorosity=False if version >= 9 else None,
        PorosityValue=0.0 if version >= 9 else None,
        RootMaterialPath="",
        AnisoLighting=False,
        EmitEnabled=False,
        EmittanceColor=(1.0, 1.0, 1.0),
        EmittanceMult=1.0,
        ModelSpaceNormals=False,
        ExternalEmittance=False,
        LumEmittance=0.0 if version >= 12 else None,
        UseAdaptativeEmissive=False if version >= 13 else None,
        AdaptativeEmissive_ExposureOffset=0.0 if version >= 13 else None,
        AdaptativeEmissive_FinalExposureMin=0.0 if version >= 13 else None,
        AdaptativeEmissive_FinalExposureMax=0.0 if version >= 13 else None,
        BackLighting=False if version < 8 else None,
        ReceiveShadows=True,
        HideSecret=False,
        CastShadows=True,
        DissolveFade=False,
        AssumeShadowmask=False,
        Glowmap=False,
        EnvironmentMappingWindow=False if version < 7 else None,
        EnvironmentMappingEye=False if version < 7 else None,
        Hair=False,
        HairTintColor=(0.0, 0.0, 0.0),
        Tree=False,
        Facegen=False,
        SkinTint=False,
        Tessellate=False,
        DisplacementTextureBias=0.0 if version < 3 else None,
        DisplacementTextureScale=0.0 if version < 3 else None,
        TessellationPnScale=0.0 if version < 3 else None,
        TessellationBaseFactor=0.0 if version < 3 else None,
        TessellationFadeDistance=0.0 if version < 3 else None,
        GrayscaleToPaletteScale=1.0,
        SkewSpecularAlpha=False if version >= 1 else None,
        Terrain=False if version >= 3 else None,
        UnkInt1=None,
        TerrainThresholdFalloff=0.0 if version >= 3 else None,
        TerrainTilingDistance=0.0 if version >= 3 else None,
        TerrainRotationAngle=0.0 if version >= 3 else None,
    )


def _default_bgem_data(version: int = 2) -> BGEMData:
    return BGEMData(
        header=_default_header(BGEM_SIGNATURE, version),
        BaseTexture="",
        GrayscaleTexture="",
        EnvmapTexture="",
        NormalTexture="",
        EnvmapMaskTexture="",
        SpecularTexture="" if version >= 11 else None,
        LightingTexture="" if version >= 11 else None,
        GlowTexture="" if version >= 11 else None,
        GlassRoughnessScratch="" if version >= 21 else None,
        GlassDirtOverlay="" if version >= 21 else None,
        GlassEnabled=False if version >= 21 else None,
        GlassFresnelColor=(1.0, 1.0, 1.0) if version >= 21 else None,
        GlassBlurScaleBase=0.0 if version >= 21 else None,
        GlassBlurScaleFactor=0.0 if version >= 22 else None,
        GlassRefractionScaleBase=0.0 if version >= 21 else None,
        EnvironmentMapping=False if version >= 10 else None,
        EnvironmentMappingMaskScale=0.0 if version >= 10 else None,
        BloodEnabled=False,
        EffectLightingEnabled=False,
        FalloffEnabled=False,
        FalloffColorEnabled=False,
        GrayscaleToPaletteAlpha=False,
        SoftEnabled=False,
        BaseColor=(1.0, 1.0, 1.0),
        BaseColorScale=1.0,
        FalloffStartAngle=1.0,
        FalloffStopAngle=1.0,
        FalloffStartOpacity=0.0,
        FalloffStopOpacity=0.0,
        LightingInfluence=0.0,
        EnvmapMinLOD=0,
        SoftDepth=100.0,
        EmittanceColor=(1.0, 1.0, 1.0) if version >= 11 else None,
        AdaptativeEmissive_ExposureOffset=0.0 if version >= 15 else None,
        AdaptativeEmissive_FinalExposureMin=0.0 if version >= 15 else None,
        AdaptativeEmissive_FinalExposureMax=0.0 if version >= 15 else None,
        Glowmap=False if version >= 16 else None,
        EffectPbrSpecular=False if version >= 20 else None,
    )


def _detect_material_type(path: Path) -> str:
    with path.open("rb") as handle:
        signature = int.from_bytes(handle.read(4), "little")
    if signature == BGSM_SIGNATURE:
        return "bgsm"
    if signature == BGEM_SIGNATURE:
        return "bgem"
    raise ValueError(f"Unknown material signature: 0x{signature:08X}")


def _resolve_material_path(material_path: str, nif_path: Path) -> Path | None:
    return _resolve_asset_path(material_path, anchors=[nif_path], default_prefix="materials")


def _scene_supports_material_documents(game: str) -> bool:
    return _normalize_scene_game(game) in {"fo4", "fo76"}


def _looks_like_material_document_path(material_path: str) -> bool:
    lower = str(material_path or "").strip().lower()
    return lower.endswith((".bgsm", ".bgem", ".mat"))


def _resolve_material_preview_paths(
    material_type: str,
    fields: dict[str, Any],
    *,
    anchors: list[Path],
) -> dict[str, str]:
    preview_paths: dict[str, str] = {}
    for field_name in PREVIEW_TEXTURE_FIELDS.get(material_type, ()):
        resolved = _resolve_asset_path(
            str(fields.get(field_name) or ""),
            anchors=anchors,
            default_prefix="textures",
        )
        if resolved is not None:
            preview_paths[field_name] = str(resolved)
    return preview_paths


def _resolve_asset_path(
    asset_path: str,
    *,
    anchors: list[Path],
    default_prefix: str,
) -> Path | None:
    if not asset_path:
        return None

    asset_path = asset_path.replace("\x00", "").strip()
    candidate = Path(asset_path)
    if candidate.is_absolute():
        return candidate if candidate.exists() else None

    normalized = asset_path.replace("\\", "/").strip().lstrip("/")
    if not normalized:
        return None

    candidate_paths = [normalized]
    lower = normalized.lower()
    prefix = f"{default_prefix.lower()}/"
    if not lower.startswith(prefix):
        candidate_paths.append(f"{default_prefix}/{normalized}")

    search_roots: list[Path] = []
    for anchor in anchors:
        search_roots.extend(_asset_search_roots(anchor))

    seen_roots: set[Path] = set()
    ordered_roots: list[Path] = []
    for root in search_roots:
        if root in seen_roots:
            continue
        seen_roots.add(root)
        ordered_roots.append(root)

    for root in ordered_roots:
        for candidate_path in candidate_paths:
            direct = root / candidate_path
            if direct.is_file():
                return direct
            resolved = _case_insensitive_resolve(root, candidate_path)
            if resolved is not None:
                return resolved
    return None


def _asset_search_roots(anchor: Path) -> list[Path]:
    roots: list[Path] = []
    base = anchor if anchor.is_dir() else anchor.parent
    roots.append(base)
    parts = list(base.parts)
    lowered = [part.lower() for part in parts]
    for index, part in enumerate(lowered):
        if part == "data":
            roots.append(Path(*parts[: index + 1]))
        elif part in {"materials", "textures", "meshes"} and index > 0:
            roots.append(Path(*parts[:index]))
    return roots


def _case_insensitive_resolve(base: Path, relative_path: str) -> Path | None:
    current = base
    for segment in relative_path.split("/"):
        if not segment:
            continue
        if not current.is_dir():
            return None
        lowered = segment.lower()
        match: Path | None = None
        try:
            for child in current.iterdir():
                if child.name.lower() == lowered:
                    match = child
                    break
        except PermissionError:
            return None
        if match is None:
            return None
        current = match
    return current if current.is_file() else None


def _default_material_path(
    output_path: Path, node_doc: dict[str, Any], material_type: str
) -> str:
    base_name = (node_doc.get("name") or output_path.stem or "material").replace(
        " ", "_"
    )
    return f"materials/{base_name}.{material_type}"


def _iter_extra_blocks(nif: NifFile, owner: NifBlock) -> list[NifBlock]:
    blocks: list[NifBlock] = []
    for ref in owner.get_field("Extra Data List") or []:
        ref_id = _ref_id(ref)
        block = nif.get_block(ref_id)
        if block is not None:
            blocks.append(block)
    return blocks


def _ensure_extra_block(nif: NifFile, owner: NifBlock, type_name: str) -> NifBlock:
    for extra in _iter_extra_blocks(nif, owner):
        if extra.type_name == type_name:
            return extra
    block = nif.add_block(type_name)
    _attach_extra(owner, block.block_id)
    return block


def _ensure_string_extra_block(nif: NifFile, owner: NifBlock, name: str) -> NifBlock:
    for extra in _iter_extra_blocks(nif, owner):
        if (
            extra.type_name == "NiStringExtraData"
            and (extra.get_field("Name") or "") == name
        ):
            return extra
    block = nif.add_block("NiStringExtraData")
    _attach_extra(owner, block.block_id)
    return block


def _read_attached_collision_nodes(
    nif: NifFile,
    owner: NifBlock,
    supported_ids: set[int],
) -> list[dict[str, Any]] | None:
    for extra in _iter_extra_blocks(nif, owner):
        if extra.type_name != "NiStringExtraData":
            continue
        if (extra.get_field("Name") or "") != ATTACHED_COLLISION_EXTRA_NAME:
            continue
        supported_ids.add(extra.block_id)
        try:
            payload = json.loads(extra.get_field("String Data") or "{}")
        except json.JSONDecodeError:
            return None
        nodes = payload.get("nodes")
        return nodes if isinstance(nodes, list) else None
    return None


def _remove_attached_collision_extra_blocks(nif: NifFile) -> None:
    remove_ids = [
        block.block_id
        for block in nif.blocks
        if block.type_name == "NiStringExtraData"
        and (block.get_field("Name") or "") == ATTACHED_COLLISION_EXTRA_NAME
    ]
    if not remove_ids:
        return

    remove_set = set(remove_ids)
    for owner in nif.blocks:
        extras = [_ref_id(ref) for ref in owner.get_field("Extra Data List") or []]
        if not extras:
            continue
        filtered = [ref for ref in extras if ref not in remove_set]
        if len(filtered) != len(extras):
            owner.set_field("Extra Data List", filtered)
            owner.set_field("Num Extra Data List", len(filtered))

    nif.remove_blocks(sorted(remove_ids))


def _ensure_alpha_property(nif: NifFile, shape: NifBlock) -> NifBlock:
    alpha_id = _ref_id(shape.get_field("Alpha Property"))
    alpha = nif.get_block(alpha_id) if alpha_id >= 0 else None
    if alpha is None or alpha.type_name != "NiAlphaProperty":
        alpha = nif.add_block("NiAlphaProperty")
        shape.set_field("Alpha Property", alpha.block_id)
    return alpha


def _attach_child(parent: NifBlock, child_id: int) -> None:
    children = [_ref_id(ref) for ref in parent.get_field("Children") or []]
    if child_id not in children:
        children.append(child_id)
    parent.set_field("Children", children)
    parent.set_field("Num Children", len(children))


def _attach_extra(owner: NifBlock, extra_id: int) -> None:
    extras = [_ref_id(ref) for ref in owner.get_field("Extra Data List") or []]
    if extra_id not in extras:
        extras.append(extra_id)
    owner.set_field("Extra Data List", extras)
    owner.set_field("Num Extra Data List", len(extras))


def _build_vertex_data(
    vertices: list[dict[str, Any]],
    normals: list[dict[str, Any]],
    uvs: list[dict[str, Any]],
    colors: list[dict[str, Any]],
    triangles: list[dict[str, Any]] | None = None,
) -> list[dict[str, Any]]:
    count = max(len(vertices), len(normals), len(uvs), len(colors))
    has_tangents = bool(normals) and bool(uvs) and bool(triangles)
    if has_tangents:
        tangents, bitangents = _compute_vertex_tangents(
            vertices, normals, uvs, triangles or [], count
        )
    out: list[dict[str, Any]] = []
    for index in range(count):
        entry: dict[str, Any] = {
            "Vertex": _vector3(vertices[index] if index < len(vertices) else {}),
            "Normal": _vector3(normals[index] if index < len(normals) else {}),
            "UV": _uv(uvs[index] if index < len(uvs) else {}),
        }
        if has_tangents:
            t = tangents[index]
            b = bitangents[index]
            # Bitangent X stored in Vertex.W slot (half-precision); Bitangent Y
            # after Normal byte3; Bitangent Z after Tangent byte3.
            entry["Bitangent X"] = float(b[0])
            entry["Tangent"] = {"x": float(t[0]), "y": float(t[1]), "z": float(t[2])}
            entry["Bitangent Y"] = float(b[1])
            entry["Bitangent Z"] = float(b[2])
        else:
            entry["Unused W"] = 0
            entry["Bitangent Y"] = 0.0
        if colors:
            entry["Vertex Colors"] = _byte_color4(
                colors[index] if index < len(colors) else {}
            )
        out.append(entry)
    return out


def _compute_vertex_tangents(
    vertices: list[dict[str, Any]],
    normals: list[dict[str, Any]],
    uvs: list[dict[str, Any]],
    triangles: list[dict[str, Any]],
    count: int,
) -> tuple[list[list[float]], list[list[float]]]:
    """Per-vertex tangent space from face Lengyel accumulation, then
    Gram-Schmidt orthonormalize against the vertex normal."""
    tan_acc = [[0.0, 0.0, 0.0] for _ in range(count)]
    bit_acc = [[0.0, 0.0, 0.0] for _ in range(count)]
    for tri in triangles:
        i0, i1, i2 = int(tri.get("v1", 0)), int(tri.get("v2", 0)), int(tri.get("v3", 0))
        if max(i0, i1, i2) >= count or min(i0, i1, i2) < 0:
            continue
        v0, v1, v2 = vertices[i0], vertices[i1], vertices[i2]
        uv0, uv1, uv2 = uvs[i0], uvs[i1], uvs[i2]
        e1 = (
            float(v1.get("x", 0.0)) - float(v0.get("x", 0.0)),
            float(v1.get("y", 0.0)) - float(v0.get("y", 0.0)),
            float(v1.get("z", 0.0)) - float(v0.get("z", 0.0)),
        )
        e2 = (
            float(v2.get("x", 0.0)) - float(v0.get("x", 0.0)),
            float(v2.get("y", 0.0)) - float(v0.get("y", 0.0)),
            float(v2.get("z", 0.0)) - float(v0.get("z", 0.0)),
        )
        du1 = float(uv1.get("u", 0.0)) - float(uv0.get("u", 0.0))
        dv1 = float(uv1.get("v", 0.0)) - float(uv0.get("v", 0.0))
        du2 = float(uv2.get("u", 0.0)) - float(uv0.get("u", 0.0))
        dv2 = float(uv2.get("v", 0.0)) - float(uv0.get("v", 0.0))
        denom = du1 * dv2 - du2 * dv1
        if abs(denom) < 1e-12:
            continue
        f = 1.0 / denom
        tx = f * (dv2 * e1[0] - dv1 * e2[0])
        ty = f * (dv2 * e1[1] - dv1 * e2[1])
        tz = f * (dv2 * e1[2] - dv1 * e2[2])
        bx = f * (du1 * e2[0] - du2 * e1[0])
        by = f * (du1 * e2[1] - du2 * e1[1])
        bz = f * (du1 * e2[2] - du2 * e1[2])
        for idx in (i0, i1, i2):
            tan_acc[idx][0] += tx; tan_acc[idx][1] += ty; tan_acc[idx][2] += tz
            bit_acc[idx][0] += bx; bit_acc[idx][1] += by; bit_acc[idx][2] += bz
    out_t: list[list[float]] = []
    out_b: list[list[float]] = []
    for i in range(count):
        n_doc = normals[i] if i < len(normals) else {"x": 0.0, "y": 0.0, "z": 1.0}
        nx = float(n_doc.get("x", 0.0))
        ny = float(n_doc.get("y", 0.0))
        nz = float(n_doc.get("z", 1.0))
        tx, ty, tz = tan_acc[i]
        # Gram-Schmidt: subtract normal-aligned component
        ndt = nx * tx + ny * ty + nz * tz
        tx -= nx * ndt; ty -= ny * ndt; tz -= nz * ndt
        tlen = (tx * tx + ty * ty + tz * tz) ** 0.5
        if tlen > 1e-9:
            tx /= tlen; ty /= tlen; tz /= tlen
        else:
            tx, ty, tz = 1.0, 0.0, 0.0
        # Bitangent from N × T, with handedness from accumulated bitangent
        bcx = ny * tz - nz * ty
        bcy = nz * tx - nx * tz
        bcz = nx * ty - ny * tx
        bx, by, bz = bit_acc[i]
        handedness = 1.0 if (bx * bcx + by * bcy + bz * bcz) >= 0.0 else -1.0
        out_t.append([tx, ty, tz])
        out_b.append([bcx * handedness, bcy * handedness, bcz * handedness])
    return out_t, out_b


def _build_bs_vertex_desc(
    *,
    vertices: list[dict[str, Any]],
    normals: list[dict[str, Any]],
    uvs: list[dict[str, Any]],
    colors: list[dict[str, Any]],
    existing_desc: int,
    triangles: list[dict[str, Any]] | None = None,
) -> int:
    attr = 0
    data_size_words = 0
    uv_offset = 0
    normal_offset = 0
    tangent_offset = 0
    color_offset = 0
    use_full_precision = bool(
        ((int(existing_desc or 0) >> 44) & BS_VERTEX_ATTR_FULL_PRECISION) != 0
    )
    has_tangents = bool(normals) and bool(uvs) and bool(triangles)

    if vertices:
        attr |= BS_VERTEX_ATTR_VERTEX
        if use_full_precision:
            attr |= BS_VERTEX_ATTR_FULL_PRECISION
            data_size_words += 4
        else:
            data_size_words += 2

    if uvs:
        attr |= BS_VERTEX_ATTR_UV
        uv_offset = data_size_words
        data_size_words += 1

    if normals:
        attr |= BS_VERTEX_ATTR_NORMAL
        normal_offset = data_size_words
        data_size_words += 1

    if has_tangents:
        attr |= BS_VERTEX_ATTR_TANGENT
        tangent_offset = data_size_words
        data_size_words += 1

    if colors:
        attr |= BS_VERTEX_ATTR_COLOR
        color_offset = data_size_words
        data_size_words += 1

    # Byte 2 (bits 16-23) packs normal_offset in the low nibble and
    # tangent_offset in the high nibble.
    normal_tangent_byte = (int(normal_offset) & 0xF) | ((int(tangent_offset) & 0xF) << 4)

    return (
        int(data_size_words)
        | (int(uv_offset) << 8)
        | (normal_tangent_byte << 16)
        | (int(color_offset) << 24)
        | (int(attr) << 44)
    )


def _build_triangles(triangles: list[dict[str, Any]]) -> list[dict[str, int]]:
    return [
        {
            "v1": int(triangle.get("v1", 0)),
            "v2": int(triangle.get("v2", 0)),
            "v3": int(triangle.get("v3", 0)),
        }
        for triangle in triangles
    ]


def _texture_slot(textures: list[str], index: int) -> str:
    return str(textures[index]) if index < len(textures) else ""


def _color4(value: Any) -> dict[str, float]:
    if isinstance(value, dict):
        return {
            "r": _byte_color_component_to_float(value.get("r", 255)),
            "g": _byte_color_component_to_float(value.get("g", 255)),
            "b": _byte_color_component_to_float(value.get("b", 255)),
            "a": _byte_color_component_to_float(value.get("a", 255)),
        }
    if isinstance(value, (list, tuple)):
        padded = list(value) + [1.0] * max(0, 4 - len(value))
        return {
            "r": _byte_color_component_to_float(padded[0]),
            "g": _byte_color_component_to_float(padded[1]),
            "b": _byte_color_component_to_float(padded[2]),
            "a": _byte_color_component_to_float(padded[3]),
        }
    return {"r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0}


def _byte_color4(value: Any) -> dict[str, int]:
    color = _color4(value)
    return {
        "r": _float_color_component_to_byte(color["r"]),
        "g": _float_color_component_to_byte(color["g"]),
        "b": _float_color_component_to_byte(color["b"]),
        "a": _float_color_component_to_byte(color["a"]),
    }


def _byte_color_component_to_float(value: Any) -> float:
    numeric = float(value or 0.0)
    if numeric > 1.0:
        numeric /= 255.0
    return max(0.0, min(1.0, numeric))


def _float_color_component_to_byte(value: Any) -> int:
    numeric = _byte_color_component_to_float(value)
    return max(0, min(255, int(round(numeric * 255.0))))


def _is_skinned(block: NifBlock) -> bool:
    return (
        _ref_id(block.get_field("Skin Instance")) >= 0
        or _ref_id(block.get_field("Skin")) >= 0
    )


def _is_legacy_tri_based_shape(nif: NifFile, block: NifBlock) -> bool:
    return nif.schema.is_subtype_of(
        block.type_name, "NiTriBasedGeom"
    ) and not nif.schema.is_subtype_of(block.type_name, "BSTriShape")


def _ref_id(ref: Any) -> int:
    if isinstance(ref, int):
        return ref
    if isinstance(ref, dict):
        return int(ref.get("block_id", -1))
    return -1


def _block_summary(block: NifBlock) -> dict[str, Any]:
    return {
        "block_id": block.block_id,
        "type": block.type_name,
        "name": block.get_field("Name") or "",
    }


def _addon_index_for_block(block: NifBlock) -> int | None:
    value = block.get_field("Value")
    if value not in (None, ""):
        return int(value)
    return _addon_index_from_name(block.get_field("Name") or "")


def _addon_index_from_name(name: str) -> int | None:
    match = ADDON_NODE_RE.match(name or "")
    if not match:
        return None
    return int(match.group(1))


def _vector3(value: Any) -> dict[str, float]:
    if isinstance(value, dict):
        return {
            "x": float(value.get("x", 0.0)),
            "y": float(value.get("y", 0.0)),
            "z": float(value.get("z", 0.0)),
        }
    if isinstance(value, (list, tuple)) and len(value) >= 3:
        return {"x": float(value[0]), "y": float(value[1]), "z": float(value[2])}
    return {"x": 0.0, "y": 0.0, "z": 0.0}


def _compute_bounding_sphere(vertices: list[Any]) -> dict[str, Any]:
    if not vertices:
        return {"Center": {"x": 0.0, "y": 0.0, "z": 0.0}, "Radius": 0.0}
    points = np.asarray(
        [
            (
                float(v.get("x", 0.0)),
                float(v.get("y", 0.0)),
                float(v.get("z", 0.0)),
            )
            if isinstance(v, dict)
            else (float(v[0]), float(v[1]), float(v[2]))
            for v in vertices
        ],
        dtype=np.float64,
    )
    aabb_center = (points.min(axis=0) + points.max(axis=0)) * 0.5
    radius = float(np.linalg.norm(points - aabb_center, axis=1).max())
    return {
        "Center": {
            "x": float(aabb_center[0]),
            "y": float(aabb_center[1]),
            "z": float(aabb_center[2]),
        },
        "Radius": radius,
    }


def _matrix33(value: Any) -> list[list[float]]:
    if isinstance(value, list) and len(value) == 3:
        return [[float(row[0]), float(row[1]), float(row[2])] for row in value]
    if isinstance(value, dict):
        return [
            [
                float(value.get("m11", 1.0)),
                float(value.get("m12", 0.0)),
                float(value.get("m13", 0.0)),
            ],
            [
                float(value.get("m21", 0.0)),
                float(value.get("m22", 1.0)),
                float(value.get("m23", 0.0)),
            ],
            [
                float(value.get("m31", 0.0)),
                float(value.get("m32", 0.0)),
                float(value.get("m33", 1.0)),
            ],
        ]
    return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]


def _quaternion(value: Any) -> dict[str, float]:
    if isinstance(value, dict):
        return {
            "w": float(value.get("w", 1.0)),
            "x": float(value.get("x", 0.0)),
            "y": float(value.get("y", 0.0)),
            "z": float(value.get("z", 0.0)),
        }
    if isinstance(value, (list, tuple)) and len(value) >= 4:
        return {
            "w": float(value[0]),
            "x": float(value[1]),
            "y": float(value[2]),
            "z": float(value[3]),
        }
    return {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0}


def _uv(value: Any) -> dict[str, float]:
    if isinstance(value, dict):
        return {"u": float(value.get("u", 0.0)), "v": float(value.get("v", 0.0))}
    if isinstance(value, (list, tuple)) and len(value) >= 2:
        return {"u": float(value[0]), "v": float(value[1])}
    return {"u": 0.0, "v": 0.0}


def _tuple_to_color3(value: Any) -> dict[str, float]:
    rgb = _color3_tuple(value)
    return {"r": float(rgb[0]), "g": float(rgb[1]), "b": float(rgb[2])}


def _color3_tuple(value: Any) -> tuple[float, float, float]:
    if isinstance(value, dict):
        return (
            float(value.get("r", 1.0)),
            float(value.get("g", 1.0)),
            float(value.get("b", 1.0)),
        )
    if isinstance(value, (list, tuple)) and len(value) >= 3:
        return (float(value[0]), float(value[1]), float(value[2]))
    return (1.0, 1.0, 1.0)


__all__ = [
    "FORMAT_VERSION",
    "export_scene_document_to_nif",
    "import_nif_to_scene_document",
    "read_material_document",
    "write_material_document",
]
