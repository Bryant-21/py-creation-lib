from __future__ import annotations

import math
import shutil
import tempfile
from pathlib import Path
from typing import Any

from creation_lib.animation.models import AnimationClip, AnimationEvent, AnimationKeyframe, BoneChannel
from creation_lib.core.game_profiles import get_profile
from creation_lib.havok.animation_reader import extract_clip
from creation_lib.havok.animation_writer import write_animation_xml
from creation_lib.havok.parsers.skeleton import SkeletonData, parse_skeleton
from creation_lib._native.havok_native import (
    DescriptorRegistry,
    HKXArrayMember,
    HKXDirectMember,
    HKXFile,
    HKXObject,
    HKXPointerMember,
    HKXStringMember,
    HKXType,
    detect_format,
    pack_xml_to_hkx,
    unpack_hkx_to_xml,
    write_xml_file,
)
from creation_lib.max.bridge import _apply_node_document, _apply_root_metadata
from creation_lib.nif.nif_file import NifFile

IDENTITY_ROTATION = (0.0, 0.0, 0.0, 1.0)
IDENTITY_SCALE = (1.0, 1.0, 1.0)


def import_hkx_skeleton_document(input_path: str) -> dict[str, Any]:
    source = Path(input_path)
    xml_path, tmp_dir = _xml_for_input(source)
    try:
        skeleton = parse_skeleton(xml_path)
    finally:
        _cleanup_tmp_dir(tmp_dir)

    game, experimental = _game_for_input(source)
    bone_records = _build_skeleton_bones(skeleton)
    cat_bindings = [
        {
            "bone_index": bone["index"],
            "bone_name": bone["name"],
            "cat_node_name": _suggest_cat_node_name(bone["name"]),
            "cat_node_handle": "",
            "translation_policy": "locked"
            if bone["lock_translation"]
            else "animation",
            "rotation_offset": [0.0, 0.0, 0.0, 1.0],
            "translation_offset": [0.0, 0.0, 0.0],
            "export_enabled": True,
        }
        for bone in bone_records
    ]
    warnings: list[str] = []
    if experimental:
        warnings.append("FO76 HKX import is experimental and export is disabled.")
    return {
        "format_version": 1,
        "kind": "havok_skeleton_json",
        "game": game,
        "source_path": str(source),
        "experimental": experimental,
        "skeleton": {
            "name": skeleton.name or source.stem,
            "bone_count": skeleton.bone_count,
            "bone_order": [bone["name"] for bone in bone_records],
            "bones": bone_records,
            "float_slots": list(skeleton.float_slots),
            "reference_floats": list(skeleton.reference_floats),
            "partition_names": list(skeleton.partition_names),
        },
        "cat_map": {
            "version": 1,
            "bindings": cat_bindings,
        },
        "warnings": warnings,
    }


def import_hkx_animation_document(
    input_path: str,
    *,
    skeleton_path: str | None = None,
) -> dict[str, Any]:
    source = Path(input_path)
    source_skeleton: dict[str, Any] | None = None
    skeleton_data: SkeletonData | None = None
    if skeleton_path:
        source_skeleton = import_hkx_skeleton_document(skeleton_path)
        skeleton_data = _skeleton_data_from_document(source_skeleton)

    xml_path, tmp_dir = _xml_for_input(source)
    try:
        clip = extract_clip(xml_path, skeleton_data)
    finally:
        _cleanup_tmp_dir(tmp_dir)

    if clip is None:
        raise ValueError(f"Unsupported or unreadable HKX animation: {input_path}")

    game, experimental = _game_for_input(source)
    if source_skeleton is None and clip.track_to_bone_indices:
        source_skeleton = {
            "format_version": 1,
            "kind": "havok_skeleton_json",
            "game": game,
            "source_path": "",
            "experimental": experimental,
            "skeleton": {
                "name": clip.original_skeleton_name or "",
                "bone_count": len(clip.channels),
                "bone_order": [channel.bone_name for channel in clip.channels],
                "bones": [
                    {
                        "index": index,
                        "name": channel.bone_name,
                        "parent_index": -1,
                        "lock_translation": False,
                        "reference_pose": {
                            "translation": [0.0, 0.0, 0.0],
                            "rotation": [0.0, 0.0, 0.0, 1.0],
                            "scale": [1.0, 1.0, 1.0],
                        },
                        "world_matrix": _identity_matrix(),
                    }
                    for index, channel in enumerate(clip.channels)
                ],
                "float_slots": [],
                "reference_floats": [],
                "partition_names": [],
            },
            "cat_map": {"version": 1, "bindings": []},
            "warnings": [],
        }

    warnings = list(clip.warnings)
    if experimental:
        warnings.append("FO76 HKX import is experimental and export is disabled.")
    return {
        "format_version": 1,
        "kind": "havok_clip_json",
        "game": game,
        "source_path": str(source),
        "skeleton_path": skeleton_path or "",
        "experimental": experimental,
        "clip": {
            "name": clip.name or source.stem,
            "duration": clip.duration,
            "native_fps": clip.native_fps,
            "is_additive": clip.is_additive,
            "original_skeleton_name": clip.original_skeleton_name,
            "track_to_bone_indices": list(clip.track_to_bone_indices),
            "events": [
                {"time": event.time, "text": event.text}
                for event in clip.events
            ],
            "channels": [_channel_to_dict(channel) for channel in clip.channels],
        },
        "skeleton": (source_skeleton or {}).get("skeleton"),
        "warnings": warnings,
    }


def export_hkx_animation_document(document: dict[str, Any], output_path: str) -> dict[str, Any]:
    game = str(document.get("game") or "fo4").strip().lower()
    if game != "fo4":
        raise ValueError("HKX export is only enabled for Fallout 4.")
    if bool(document.get("experimental")):
        raise ValueError("Experimental FO76 imports cannot be exported yet.")

    clip_doc = dict(document.get("clip") or {})
    if bool(clip_doc.get("is_additive")):
        raise ValueError("Additive HKX clips are import-only in this implementation.")

    skeleton_doc = dict(document.get("skeleton") or {})
    bone_order = list(skeleton_doc.get("bone_order") or [])
    if not bone_order:
        raise ValueError("Missing skeleton bone order for HKX export.")

    clip = _clip_from_document(document)

    tmp_dir = Path(tempfile.mkdtemp(prefix="mb21_hkx_export_"))
    xml_path = tmp_dir / "animation.xml"
    try:
        write_animation_xml(clip, bone_order, xml_path)
        pack_xml_to_hkx(str(xml_path), output_path)
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)

    return {
        "output_path": str(output_path),
        "game": game,
        "bone_count": len(bone_order),
        "duration": clip.duration,
        "native_fps": clip.native_fps,
    }


def export_hkx_skeleton_document(
    document: dict[str, Any],
    output_path: str,
) -> dict[str, Any]:
    game = str(document.get("game") or "fo4").strip().lower()
    if game != "fo4":
        raise ValueError("HKX skeleton export is only enabled for Fallout 4.")

    skeleton_doc = skeleton_scene_to_skeleton_document(document)

    tmp_dir = Path(tempfile.mkdtemp(prefix="mb21_hkx_skeleton_export_"))
    xml_path = tmp_dir / "skeleton.xml"
    try:
        write_skeleton_xml(skeleton_doc, xml_path)
        pack_xml_to_hkx(str(xml_path), output_path)
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)

    return {
        "output_path": str(output_path),
        "game": game,
        "bone_count": len(list(skeleton_doc.get("bones") or [])),
        "skeleton_name": str(skeleton_doc.get("name") or ""),
    }


def export_skeleton_nif_document(
    document: dict[str, Any],
    output_path: str,
) -> dict[str, Any]:
    scene_document = skeleton_scene_to_nif_document(document)
    game = str(scene_document.get("game") or "fo4").strip().lower()
    profile = get_profile(game)
    output = Path(output_path)
    output.parent.mkdir(parents=True, exist_ok=True)

    nif = _load_pruned_skeleton_template_nif()
    root_block = nif.get_block(0)
    if root_block is None:
        raise ValueError("Skeleton NIF template is missing block 0.")

    warnings: list[str] = []
    root_nodes = list(scene_document.get("root_nodes") or [])
    if not root_nodes:
        raise ValueError("Skeleton export scene document has no root nodes.")

    _apply_node_document(
        nif=nif,
        node_doc=root_nodes[0],
        owner_parent=None,
        materials_by_id={},
        output_path=output,
        warnings=warnings,
        profile=profile,
        forced_block=root_block,
    )
    _apply_root_metadata(
        nif=nif,
        root=root_block,
        metadata=(scene_document.get("metadata") or {}).get("root") or {},
    )
    nif.save(str(output))

    result = {
        "output_path": str(output),
        "game": game,
        "num_blocks": len(nif.blocks),
        "warnings": warnings,
    }
    result["skeleton_name"] = str(
        ((document.get("skeleton") or {}).get("name"))
        or document.get("skeleton_name")
        or ""
    )
    return result


def skeleton_scene_to_skeleton_document(document: dict[str, Any]) -> dict[str, Any]:
    stored_skeleton = dict(document.get("skeleton") or {})
    scene_bones = _normalize_scene_export_bones(document.get("bones") or [])
    if not scene_bones:
        raise ValueError("Skeleton export JSON is missing scene bones.")

    world_by_index = {
        int(bone["index"]): transform_matrix(bone["world_matrix"])
        for bone in scene_bones
    }
    output_bones: list[dict[str, Any]] = []
    for bone in scene_bones:
        bone_index = int(bone["index"])
        parent_index = int(bone.get("parent_index", -1))
        world_matrix = world_by_index[bone_index]
        if parent_index >= 0 and parent_index in world_by_index:
            local_matrix = _matrix_multiply(
                world_matrix,
                _invert_matrix(world_by_index[parent_index]),
            )
        else:
            local_matrix = world_matrix
        translation, rotation, scale = _decompose_matrix(local_matrix)
        output_bones.append(
            {
                "index": bone_index,
                "name": str(bone.get("name") or f"bone_{bone_index}"),
                "parent_index": parent_index,
                "lock_translation": bool(bone.get("lock_translation")),
                "reference_pose": {
                    "translation": [float(value) for value in translation],
                    "rotation": [float(value) for value in rotation],
                    "scale": [float(value) for value in scale],
                },
                "world_matrix": world_matrix,
            }
        )

    skeleton_name = (
        str(stored_skeleton.get("name") or "").strip()
        or str(document.get("skeleton_name") or "").strip()
        or str(output_bones[0]["name"])
    )
    return {
        "name": skeleton_name,
        "bone_count": len(output_bones),
        "bone_order": [str(bone["name"]) for bone in output_bones],
        "bones": output_bones,
        "float_slots": list(stored_skeleton.get("float_slots") or []),
        "reference_floats": [
            float(value) for value in (stored_skeleton.get("reference_floats") or [])
        ],
        "partition_names": [str(value) for value in (stored_skeleton.get("partition_names") or [])],
    }


def skeleton_scene_to_nif_document(document: dict[str, Any]) -> dict[str, Any]:
    game = str(document.get("game") or "fo4").strip().lower()
    skeleton_doc = skeleton_scene_to_skeleton_document(document)
    bones = list(skeleton_doc.get("bones") or [])
    if not bones:
        raise ValueError("Skeleton export JSON is missing scene bones.")

    children_by_parent: dict[int, list[dict[str, Any]]] = {}
    roots: list[dict[str, Any]] = []
    for bone in bones:
        bone_doc = _bone_to_nif_node_document(bone, bones)
        parent_index = int(bone.get("parent_index", -1))
        if parent_index >= 0:
            children_by_parent.setdefault(parent_index, []).append(bone_doc)
        else:
            roots.append(bone_doc)

    bone_docs_by_index = {
        int(bone["index"]): _bone_to_nif_node_document(bone, bones)
        for bone in bones
    }
    for bone in bones:
        parent_index = int(bone["index"])
        parent_doc = bone_docs_by_index[parent_index]
        parent_doc["children"] = children_by_parent.get(parent_index, [])
    root_docs = [bone_docs_by_index[int(bone["index"])] for bone in bones if int(bone.get("parent_index", -1)) < 0]

    if len(root_docs) == 1:
        scene_root = dict(root_docs[0])
    else:
        scene_root = {
            "id": "mb21-export-root",
            "type": "node",
            "nif_type": "NiNode",
            "name": str(skeleton_doc.get("name") or "SkeletonRoot"),
            "transform": {
                "translation": {"x": 0.0, "y": 0.0, "z": 0.0},
                "rotation": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                "scale": 1.0,
            },
            "metadata": {"extra_string_data": []},
            "children": root_docs,
        }
    scene_root["id"] = "mb21-export-root"

    return {
        "format_version": 1,
        "game": game,
        "source_path": str(document.get("source_path") or ""),
        "root_nodes": [scene_root],
        "export_settings": {"synthesize_scene_root": True},
        "materials": [],
        "metadata": {"root": {"bsx_flags": 2}},
        "warnings": [],
    }


def scene_bake_to_hkx_document(scene_bake: dict[str, Any]) -> dict[str, Any]:
    game = str(scene_bake.get("game") or "fo4").strip().lower()
    skeleton_doc = dict(scene_bake.get("skeleton") or {})
    bones = list(skeleton_doc.get("bones") or [])
    if not bones:
        raise ValueError("Scene bake JSON is missing skeleton bones.")

    sample_rate = float(((scene_bake.get("range") or {}).get("fps")) or 30.0)
    if sample_rate <= 0:
        sample_rate = 30.0

    frames = list(scene_bake.get("frames") or [])
    if not frames:
        raise ValueError("Scene bake JSON has no sampled frames.")

    per_bone_rotations: dict[int, list[AnimationKeyframe]] = {bone["index"]: [] for bone in bones}
    per_bone_translations: dict[int, list[AnimationKeyframe]] = {bone["index"]: [] for bone in bones}
    per_bone_scales: dict[int, list[AnimationKeyframe]] = {bone["index"]: [] for bone in bones}

    for frame_index, frame in enumerate(frames):
        time_value = float(frame.get("time") or (frame_index / sample_rate))
        transforms_by_index = {
            int(transform["bone_index"]): transform
            for transform in frame.get("transforms") or []
            if "bone_index" in transform
        }
        local_matrices = {
            bone["index"]: transform_matrix(
                transforms_by_index[bone["index"]]["world_matrix"]
            )
            for bone in bones
            if bone["index"] in transforms_by_index
        }
        for bone in bones:
            bone_index = int(bone["index"])
            world_matrix = local_matrices.get(bone_index)
            if world_matrix is None:
                continue
            parent_index = int(bone.get("parent_index", -1))
            if parent_index >= 0 and parent_index in local_matrices:
                local_matrix = _matrix_multiply(
                    world_matrix,
                    _invert_matrix(local_matrices[parent_index]),
                )
            else:
                local_matrix = world_matrix
            translation, rotation, scale = _decompose_matrix(local_matrix)
            per_bone_translations[bone_index].append(
                AnimationKeyframe(time=time_value, value=translation)
            )
            per_bone_rotations[bone_index].append(
                AnimationKeyframe(time=time_value, value=rotation)
            )
            per_bone_scales[bone_index].append(
                AnimationKeyframe(time=time_value, value=scale)
            )

    channels = []
    for bone in bones:
        bone_index = int(bone["index"])
        channels.append(
            BoneChannel(
                bone_name=str(bone["name"]),
                rotations=tuple(per_bone_rotations[bone_index]),
                translations=tuple(per_bone_translations[bone_index]),
                scales=tuple(per_bone_scales[bone_index]),
            )
        )

    duration = float(frames[-1].get("time") or ((len(frames) - 1) / sample_rate))
    events = tuple(
        AnimationEvent(time=float(event.get("time") or 0.0), text=str(event.get("text") or ""))
        for event in scene_bake.get("events") or []
        if str(event.get("text") or "").strip()
    )
    cat_map = dict(scene_bake.get("cat_map") or {})
    return {
        "format_version": 1,
        "kind": "havok_clip_json",
        "game": game,
        "source_path": str(scene_bake.get("source_animation_path") or ""),
        "skeleton_path": str(scene_bake.get("source_skeleton_path") or ""),
        "experimental": False,
        "clip": {
            "name": str(scene_bake.get("clip_name") or "scene_bake"),
            "duration": duration,
            "native_fps": sample_rate,
            "is_additive": False,
            "original_skeleton_name": str(skeleton_doc.get("name") or "skeleton"),
            "track_to_bone_indices": [int(bone["index"]) for bone in bones],
            "events": [
                {"time": event.time, "text": event.text}
                for event in events
            ],
            "channels": [_channel_to_dict(channel) for channel in channels],
        },
        "skeleton": skeleton_doc,
        "cat_map": cat_map,
        "warnings": [],
    }


def transform_matrix(raw_matrix: list[list[float]]) -> list[list[float]]:
    return [[float(value) for value in row] for row in raw_matrix]


def write_skeleton_xml(
    skeleton_doc: dict[str, Any],
    output_path: str | Path,
) -> None:
    output = Path(output_path)
    output.parent.mkdir(parents=True, exist_ok=True)
    bones = list(skeleton_doc.get("bones") or [])
    hkx_file = HKXFile(
        class_version=11,
        contents_version="hk_2014.1.0-r1",
    )
    hkx_file.objects = [
        HKXObject(
            name="#90",
            class_name="hkRootLevelContainer",
            members=[
                HKXArrayMember(
                    "namedVariants",
                    HKXType.STRUCT,
                    [
                        _named_variant_object(
                            "Merged Animation Container",
                            "hkaAnimationContainer",
                            "#91",
                        ),
                        _named_variant_object(
                            "Resource Data",
                            "hkMemoryResourceContainer",
                            "#93",
                        ),
                    ],
                    ctype="hkRootLevelContainerNamedVariant",
                )
            ],
        ),
        HKXObject(
            name="#91",
            class_name="hkaAnimationContainer",
            members=[
                HKXArrayMember("skeletons", HKXType.POINTER, ["#92"], ctype="hkaSkeleton"),
                HKXArrayMember("animations", HKXType.POINTER, [], ctype="hkaAnimation"),
                HKXArrayMember("bindings", HKXType.POINTER, [], ctype="hkaAnimationBinding"),
                HKXArrayMember("attachments", HKXType.POINTER, [], ctype="hkaBoneAttachment"),
                HKXArrayMember("skins", HKXType.POINTER, [], ctype="hkaMeshBinding"),
            ],
        ),
        HKXObject(
            name="#92",
            class_name="hkaSkeleton",
            members=[
                HKXStringMember("name", str(skeleton_doc.get("name") or "Skeleton")),
                HKXArrayMember(
                    "parentIndices",
                    HKXType.INT16,
                    [int(bone.get("parent_index", -1)) for bone in bones],
                ),
                HKXArrayMember(
                    "bones",
                    HKXType.STRUCT,
                    [_bone_object_for_hkx(bone) for bone in bones],
                    ctype="hkaBone",
                ),
                HKXArrayMember(
                    "referencePose",
                    HKXType.QSTRANSFORM,
                    [
                        _reference_pose_qstransform(dict(bone.get("reference_pose") or {}))
                        for bone in bones
                    ],
                ),
                HKXArrayMember(
                    "referenceFloats",
                    HKXType.REAL,
                    [
                        float(value)
                        for value in (skeleton_doc.get("reference_floats") or [])
                    ],
                ),
                HKXArrayMember(
                    "floatSlots",
                    HKXType.STRINGPTR,
                    [str(value) for value in (skeleton_doc.get("float_slots") or [])],
                ),
                HKXArrayMember(
                    "localFrames",
                    HKXType.STRUCT,
                    [],
                    ctype="hkaSkeletonLocalFrameOnBone",
                ),
                HKXArrayMember(
                    "partitions",
                    HKXType.STRUCT,
                    [
                        HKXObject(
                            name="",
                            class_name="hkaSkeletonPartition",
                            members=[
                                HKXStringMember("name", str(partition_name)),
                            ],
                        )
                        for partition_name in (skeleton_doc.get("partition_names") or [])
                    ],
                    ctype="hkaSkeletonPartition",
                ),
            ],
        ),
        HKXObject(
            name="#93",
            class_name="hkMemoryResourceContainer",
            members=[
                HKXStringMember("name", ""),
                HKXArrayMember("resourceHandles", HKXType.POINTER, [], ctype="hkResourceHandle"),
                HKXArrayMember("children", HKXType.POINTER, [], ctype="hkMemoryResourceContainer"),
            ],
        ),
    ]
    write_xml_file(hkx_file, DescriptorRegistry(), str(output))


def _cleanup_tmp_dir(tmp_dir: Path | None) -> None:
    if tmp_dir is not None:
        shutil.rmtree(tmp_dir, ignore_errors=True)


def _normalize_scene_export_bones(raw_bones: list[dict[str, Any]]) -> list[dict[str, Any]]:
    ordered = sorted(raw_bones, key=lambda item: int(item.get("index", 0)))
    old_to_new = {
        int(bone.get("index", index)): index for index, bone in enumerate(ordered)
    }
    normalized: list[dict[str, Any]] = []
    for index, bone in enumerate(ordered):
        original_parent = int(bone.get("parent_index", -1))
        normalized.append(
            {
                **dict(bone),
                "index": index,
                "parent_index": old_to_new.get(original_parent, -1)
                if original_parent >= 0
                else -1,
                "world_matrix": transform_matrix(bone.get("world_matrix") or _identity_matrix()),
            }
        )
    return normalized


def _bone_to_nif_node_document(
    bone: dict[str, Any],
    all_bones: list[dict[str, Any]],
) -> dict[str, Any]:
    world_matrix = transform_matrix(bone.get("world_matrix") or _identity_matrix())
    parent_index = int(bone.get("parent_index", -1))
    parent_world = _identity_matrix()
    if parent_index >= 0:
        parent_world = transform_matrix(
            next(
                candidate.get("world_matrix")
                for candidate in all_bones
                if int(candidate.get("index", -1)) == parent_index
            )
        )
    local_matrix = (
        _matrix_multiply(world_matrix, _invert_matrix(parent_world))
        if parent_index >= 0
        else world_matrix
    )
    translation, rotation, scale = _decompose_matrix(local_matrix)
    rotation_matrix = _quaternion_to_rotation_matrix(rotation)
    return {
        "id": f"havok-bone-{int(bone.get('index', 0))}",
        "type": "node",
        "nif_type": "NiNode",
        "name": str(bone.get("name") or f"bone_{int(bone.get('index', 0))}"),
        "transform": {
            "translation": {
                "x": float(translation[0]),
                "y": float(translation[1]),
                "z": float(translation[2]),
            },
            "rotation": rotation_matrix,
            "scale": float(scale[0]) if scale else 1.0,
        },
        "metadata": {"extra_string_data": []},
        "children": [],
    }


def _quaternion_to_rotation_matrix(
    rotation: tuple[float, float, float, float],
) -> list[list[float]]:
    matrix = _compose_matrix((0.0, 0.0, 0.0), rotation, (1.0, 1.0, 1.0))
    return [row[:3] for row in matrix[:3]]


def _named_variant_object(name: str, class_name: str, variant: str) -> HKXObject:
    return HKXObject(
        name="",
        class_name="hkRootLevelContainerNamedVariant",
        members=[
            HKXStringMember("name", name),
            HKXStringMember("className", class_name),
            HKXPointerMember("variant", variant),
        ],
    )


def _bone_object_for_hkx(bone: dict[str, Any]) -> HKXObject:
    return HKXObject(
        name="",
        class_name="hkaBone",
        members=[
            HKXStringMember("name", str(bone.get("name") or "")),
            HKXDirectMember(
                "lockTranslation",
                HKXType.BOOL,
                bool(bone.get("lock_translation")),
            ),
        ],
    )


def _reference_pose_qstransform(pose: dict[str, Any]) -> list[float]:
    translation = [float(v) for v in (pose.get("translation") or [0.0, 0.0, 0.0])]
    rotation = [float(v) for v in (pose.get("rotation") or [0.0, 0.0, 0.0, 1.0])]
    scale = [float(v) for v in (pose.get("scale") or [1.0, 1.0, 1.0])]
    return [
        translation[0],
        translation[1],
        translation[2],
        0.0,
        rotation[0],
        rotation[1],
        rotation[2],
        rotation[3],
        scale[0],
        scale[1],
        scale[2],
        0.0,
    ]


def _fmt_float(value: float) -> str:
    return f"{float(value):.9g}"


def _fo4_skeleton_nif_template_path() -> Path:
    from creation_lib.paths import get_resource_dir

    return Path(get_resource_dir()) / "skeleton.nif"


def _load_pruned_skeleton_template_nif() -> NifFile:
    nif = NifFile.load(str(_fo4_skeleton_nif_template_path()))
    root = nif.get_block(0)
    if root is None:
        raise ValueError("Skeleton NIF template is missing block 0.")

    remove_ids = _collect_nif_references(
        nif,
        [_ref_id for _ref_id in (root.get_field("Children") or []) if isinstance(_ref_id, int)],
    )
    remove_ids.extend(
        _ref_id
        for _ref_id in (root.get_field("Extra Data List") or [])
        if isinstance(_ref_id, int)
    )
    nif.remove_blocks(sorted(set(block_id for block_id in remove_ids if block_id > 0)))

    root = nif.get_block(0)
    if root is None:
        raise ValueError("Skeleton NIF template pruning removed the root block.")
    root.set_field("Children", [])
    root.set_field("Num Children", 0)
    root.set_field("Extra Data List", [])
    root.set_field("Num Extra Data List", 0)
    root.set_field("Controller", -1)
    root.set_field("Collision Object", -1)
    return nif


def _collect_nif_references(nif: NifFile, start_ids: list[int]) -> list[int]:
    seen: set[int] = set()
    stack = [block_id for block_id in start_ids if block_id >= 0]
    while stack:
        block_id = stack.pop()
        if block_id in seen:
            continue
        block = nif.get_block(block_id)
        if block is None:
            continue
        seen.add(block_id)
        stack.extend(
            ref_id
            for ref_id in block.get_refs(nif.schema)
            if isinstance(ref_id, int) and ref_id >= 0 and ref_id not in seen
        )
    return sorted(seen)


def _xml_for_input(input_path: Path) -> tuple[Path, Path | None]:
    if input_path.suffix.lower() == ".xml":
        return input_path, None
    tmp_dir = Path(tempfile.mkdtemp(prefix="mb21_hkx_import_"))
    xml_path = tmp_dir / f"{input_path.stem}.xml"
    xml_path.write_text(unpack_hkx_to_xml(str(input_path)), encoding="utf-8")
    return xml_path, tmp_dir


def _game_for_input(input_path: Path) -> tuple[str, bool]:
    try:
        fmt = detect_format(input_path.read_bytes())
    except OSError:
        fmt = None
    if fmt and fmt[0] == "tagfile":
        return "fo76", True
    return "fo4", False


def _build_skeleton_bones(skeleton: SkeletonData) -> list[dict[str, Any]]:
    world_matrices = _world_matrices_for_skeleton(skeleton)
    bones: list[dict[str, Any]] = []
    for index, name in enumerate(skeleton.bone_names):
        pose = skeleton.reference_pose[index] if index < len(skeleton.reference_pose) else {
            "t": [0.0, 0.0, 0.0],
            "q": [0.0, 0.0, 0.0, 1.0],
            "s": [1.0, 1.0, 1.0],
        }
        bones.append(
            {
                "index": index,
                "name": name,
                "parent_index": skeleton.parent_indices[index]
                if index < len(skeleton.parent_indices)
                else -1,
                "lock_translation": skeleton.lock_translation[index]
                if index < len(skeleton.lock_translation)
                else False,
                "reference_pose": {
                    "translation": [float(v) for v in pose.get("t") or [0.0, 0.0, 0.0]],
                    "rotation": [float(v) for v in pose.get("q") or [0.0, 0.0, 0.0, 1.0]],
                    "scale": [float(v) for v in pose.get("s") or [1.0, 1.0, 1.0]],
                },
                "world_matrix": world_matrices[index],
            }
        )
    return bones


def _world_matrices_for_skeleton(skeleton: SkeletonData) -> list[list[list[float]]]:
    world_matrices = [_identity_matrix() for _ in skeleton.bone_names]
    for index, pose in enumerate(skeleton.reference_pose):
        local_matrix = _compose_matrix(
            tuple(float(v) for v in pose.get("t") or (0.0, 0.0, 0.0)),
            tuple(float(v) for v in pose.get("q") or IDENTITY_ROTATION),
            tuple(float(v) for v in pose.get("s") or IDENTITY_SCALE),
        )
        parent_index = skeleton.parent_indices[index] if index < len(skeleton.parent_indices) else -1
        if parent_index >= 0:
            world_matrices[index] = _matrix_multiply(local_matrix, world_matrices[parent_index])
        else:
            world_matrices[index] = local_matrix
    return world_matrices


def _compose_matrix(
    translation: tuple[float, float, float],
    rotation: tuple[float, float, float, float],
    scale: tuple[float, float, float],
) -> list[list[float]]:
    rx, ry, rz, rw = rotation
    xx = rx * rx
    yy = ry * ry
    zz = rz * rz
    xy = rx * ry
    xz = rx * rz
    yz = ry * rz
    wx = rw * rx
    wy = rw * ry
    wz = rw * rz
    rot = [
        [1.0 - 2.0 * (yy + zz), 2.0 * (xy + wz), 2.0 * (xz - wy), 0.0],
        [2.0 * (xy - wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz + wx), 0.0],
        [2.0 * (xz + wy), 2.0 * (yz - wx), 1.0 - 2.0 * (xx + yy), 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
    scaled = [
        [rot[0][0] * scale[0], rot[0][1] * scale[0], rot[0][2] * scale[0], 0.0],
        [rot[1][0] * scale[1], rot[1][1] * scale[1], rot[1][2] * scale[1], 0.0],
        [rot[2][0] * scale[2], rot[2][1] * scale[2], rot[2][2] * scale[2], 0.0],
        [translation[0], translation[1], translation[2], 1.0],
    ]
    return scaled


def _decompose_matrix(
    matrix: list[list[float]],
) -> tuple[tuple[float, float, float], tuple[float, float, float, float], tuple[float, float, float]]:
    translation = (matrix[3][0], matrix[3][1], matrix[3][2])
    row0 = matrix[0][:3]
    row1 = matrix[1][:3]
    row2 = matrix[2][:3]
    sx = _vector_length(row0) or 1.0
    sy = _vector_length(row1) or 1.0
    sz = _vector_length(row2) or 1.0
    rotation_matrix = [
        [row0[0] / sx, row0[1] / sx, row0[2] / sx],
        [row1[0] / sy, row1[1] / sy, row1[2] / sy],
        [row2[0] / sz, row2[1] / sz, row2[2] / sz],
    ]
    rotation = _matrix_to_quaternion(rotation_matrix)
    return translation, rotation, (sx, sy, sz)


def _matrix_to_quaternion(rotation: list[list[float]]) -> tuple[float, float, float, float]:
    trace = rotation[0][0] + rotation[1][1] + rotation[2][2]
    if trace > 0.0:
        s = math.sqrt(trace + 1.0) * 2.0
        w = 0.25 * s
        x = (rotation[2][1] - rotation[1][2]) / s
        y = (rotation[0][2] - rotation[2][0]) / s
        z = (rotation[1][0] - rotation[0][1]) / s
    elif rotation[0][0] > rotation[1][1] and rotation[0][0] > rotation[2][2]:
        s = math.sqrt(1.0 + rotation[0][0] - rotation[1][1] - rotation[2][2]) * 2.0
        w = (rotation[2][1] - rotation[1][2]) / s
        x = 0.25 * s
        y = (rotation[0][1] + rotation[1][0]) / s
        z = (rotation[0][2] + rotation[2][0]) / s
    elif rotation[1][1] > rotation[2][2]:
        s = math.sqrt(1.0 + rotation[1][1] - rotation[0][0] - rotation[2][2]) * 2.0
        w = (rotation[0][2] - rotation[2][0]) / s
        x = (rotation[0][1] + rotation[1][0]) / s
        y = 0.25 * s
        z = (rotation[1][2] + rotation[2][1]) / s
    else:
        s = math.sqrt(1.0 + rotation[2][2] - rotation[0][0] - rotation[1][1]) * 2.0
        w = (rotation[1][0] - rotation[0][1]) / s
        x = (rotation[0][2] + rotation[2][0]) / s
        y = (rotation[1][2] + rotation[2][1]) / s
        z = 0.25 * s
    return (x, y, z, w)


def _vector_length(values: list[float]) -> float:
    return math.sqrt(sum(value * value for value in values))


def _channel_to_dict(channel: BoneChannel) -> dict[str, Any]:
    return {
        "bone_name": channel.bone_name,
        "priority": channel.priority,
        "rotations": [_keyframe_to_dict(key) for key in channel.rotations],
        "translations": [_keyframe_to_dict(key) for key in channel.translations],
        "scales": [_keyframe_to_dict(key) for key in channel.scales],
    }


def _keyframe_to_dict(keyframe: AnimationKeyframe) -> dict[str, Any]:
    return {
        "time": keyframe.time,
        "value": list(keyframe.value),
    }


def _skeleton_data_from_document(document: dict[str, Any]) -> SkeletonData:
    skeleton_doc = dict(document.get("skeleton") or {})
    bones = list(skeleton_doc.get("bones") or [])
    return SkeletonData(
        name=str(skeleton_doc.get("name") or ""),
        bone_count=len(bones),
        bone_names=[str(bone.get("name") or "") for bone in bones],
        parent_indices=[int(bone.get("parent_index", -1)) for bone in bones],
        reference_pose=[
            {
                "t": list(
                    ((bone.get("reference_pose") or {}).get("translation"))
                    or [0.0, 0.0, 0.0]
                ),
                "q": list(
                    ((bone.get("reference_pose") or {}).get("rotation"))
                    or [0.0, 0.0, 0.0, 1.0]
                ),
                "s": list(
                    ((bone.get("reference_pose") or {}).get("scale"))
                    or [1.0, 1.0, 1.0]
                ),
            }
            for bone in bones
        ],
        lock_translation=[bool(bone.get("lock_translation")) for bone in bones],
        float_count=len(list(skeleton_doc.get("float_slots") or [])),
        float_slots=list(skeleton_doc.get("float_slots") or []),
        reference_floats=list(skeleton_doc.get("reference_floats") or []),
        partition_names=list(skeleton_doc.get("partition_names") or []),
    )


def _clip_from_document(document: dict[str, Any]) -> AnimationClip:
    clip_doc = dict(document.get("clip") or {})
    events = tuple(
        AnimationEvent(time=float(event.get("time") or 0.0), text=str(event.get("text") or ""))
        for event in clip_doc.get("events") or []
        if str(event.get("text") or "").strip()
    )
    channels = tuple(
        BoneChannel(
            bone_name=str(channel.get("bone_name") or ""),
            priority=int(channel.get("priority") or 26),
            rotations=tuple(_keyframe_from_dict(item) for item in channel.get("rotations") or []),
            translations=tuple(_keyframe_from_dict(item) for item in channel.get("translations") or []),
            scales=tuple(_keyframe_from_dict(item) for item in channel.get("scales") or []),
        )
        for channel in clip_doc.get("channels") or []
    )
    return AnimationClip(
        name=str(clip_doc.get("name") or "animation"),
        duration=float(clip_doc.get("duration") or 0.0),
        cycle_type="clamp",
        frequency=1.0,
        accum_root="",
        channels=channels,
        float_channels=(),
        events=events,
        source_format="hkx",
        warnings=tuple(document.get("warnings") or []),
        native_fps=float(clip_doc.get("native_fps") or 30.0),
        is_additive=bool(clip_doc.get("is_additive")),
        track_to_bone_indices=tuple(int(value) for value in clip_doc.get("track_to_bone_indices") or []),
        original_skeleton_name=str(clip_doc.get("original_skeleton_name") or ""),
    )


def _keyframe_from_dict(value: dict[str, Any]) -> AnimationKeyframe:
    return AnimationKeyframe(
        time=float(value.get("time") or 0.0),
        value=tuple(float(component) for component in value.get("value") or ()),
    )


def _suggest_cat_node_name(bone_name: str) -> str:
    normalized = bone_name.lower()
    mapping = {
        "base01": "Pelvis",
        "pelvis": "Pelvis",
        "spine": "Spine",
        "head": "Head",
        "camera": "Head",
        "larm": "LeftArm",
        "rarm": "RightArm",
        "lleg": "LeftLeg",
        "rleg": "RightLeg",
        "lhand": "LeftHand",
        "rhand": "RightHand",
        "lfoot": "LeftFoot",
        "rfoot": "RightFoot",
    }
    for token, target in mapping.items():
        if token in normalized:
            return target
    return bone_name


def _identity_matrix() -> list[list[float]]:
    return [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]


def _matrix_multiply(a: list[list[float]], b: list[list[float]]) -> list[list[float]]:
    result = [[0.0] * 4 for _ in range(4)]
    for row in range(4):
        for col in range(4):
            result[row][col] = sum(a[row][idx] * b[idx][col] for idx in range(4))
    return result


def _invert_matrix(matrix: list[list[float]]) -> list[list[float]]:
    work = [row[:] for row in matrix]
    inverse = _identity_matrix()
    for col in range(4):
        pivot_row = max(range(col, 4), key=lambda row: abs(work[row][col]))
        pivot = work[pivot_row][col]
        if abs(pivot) < 1e-10:
            raise ValueError("Matrix is singular and cannot be inverted.")
        if pivot_row != col:
            work[col], work[pivot_row] = work[pivot_row], work[col]
            inverse[col], inverse[pivot_row] = inverse[pivot_row], inverse[col]
        factor = work[col][col]
        work[col] = [value / factor for value in work[col]]
        inverse[col] = [value / factor for value in inverse[col]]
        for row in range(4):
            if row == col:
                continue
            elim = work[row][col]
            work[row] = [work[row][i] - elim * work[col][i] for i in range(4)]
            inverse[row] = [inverse[row][i] - elim * inverse[col][i] for i in range(4)]
    return inverse
