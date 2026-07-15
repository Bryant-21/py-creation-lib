"""Import FBX files using Autodesk FBX SDK."""

from __future__ import annotations

import logging
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path

import numpy as np

from creation_lib.skinning.skin_data import SkinData

from .sdk import HAS_FBX, fbx

_log = logging.getLogger("fbx.import_fbx")


@dataclass
class FbxMeshData:
    name: str
    vertices: np.ndarray
    triangles: np.ndarray
    normals: np.ndarray
    uvs: np.ndarray
    bone_names: list[str]
    bone_indices: np.ndarray
    weights: np.ndarray
    bone_parents: list[int]
    translation: dict[str, float]
    rotation: list[list[float]]
    scale: float


def _matrix_to_numpy(matrix) -> np.ndarray:
    return np.array(
        [[float(matrix.Get(row, col)) for col in range(4)] for row in range(4)],
        dtype=np.float32,
    )


def _transform_position(position: np.ndarray, matrix: np.ndarray) -> np.ndarray:
    x, y, z = position
    return np.array(
        [
            matrix[0, 0] * x + matrix[0, 1] * y + matrix[0, 2] * z + matrix[0, 3],
            matrix[1, 0] * x + matrix[1, 1] * y + matrix[1, 2] * z + matrix[1, 3],
            matrix[2, 0] * x + matrix[2, 1] * y + matrix[2, 2] * z + matrix[2, 3],
        ],
        dtype=np.float32,
    )


def _transform_normal(normal: np.ndarray, matrix: np.ndarray) -> np.ndarray:
    transformed = matrix[:3, :3] @ normal
    length = float(np.linalg.norm(transformed))
    if length <= 1e-8:
        return np.zeros(3, dtype=np.float32)
    return (transformed / length).astype(np.float32)


def _load_scene(filepath: str | Path):
    if not HAS_FBX:
        raise ImportError("Autodesk FBX SDK is not installed")

    path = Path(filepath)
    if not path.exists():
        raise FileNotFoundError(f"FBX file not found: {path}")

    manager = fbx.FbxManager.Create()
    ios = fbx.FbxIOSettings.Create(manager, "IOSROOT")
    manager.SetIOSettings(ios)

    importer = fbx.FbxImporter.Create(manager, "")
    if not importer.Initialize(str(path), -1, manager.GetIOSettings()):
        status = importer.GetStatus().GetErrorString()
        importer.Destroy()
        manager.Destroy()
        raise ValueError(f"Failed to initialize FBX importer: {status}")

    scene = fbx.FbxScene.Create(manager, path.stem)
    if not importer.Import(scene):
        status = importer.GetStatus().GetErrorString()
        importer.Destroy()
        manager.Destroy()
        raise ValueError(f"Failed to import FBX scene: {status}")
    importer.Destroy()

    converter = fbx.FbxGeometryConverter(manager)
    converter.Triangulate(scene, True)
    return manager, scene


def _iter_nodes(node):
    yield node
    for i in range(node.GetChildCount()):
        yield from _iter_nodes(node.GetChild(i))


def _node_transform_fields(node) -> tuple[dict[str, float], list[list[float]], float]:
    translation_vec = node.EvaluateLocalTranslation()
    local = node.EvaluateLocalTransform()
    scale_vec = node.EvaluateLocalScaling()
    return (
        {
            "x": float(translation_vec[0]),
            "y": float(translation_vec[1]),
            "z": float(translation_vec[2]),
        },
        [[float(local.Get(row, col)) for col in range(3)] for row in range(3)],
        float(scale_vec[0]),
    )


def _collect_skinning(mesh) -> tuple[dict[int, list[tuple[int, float]]], list[str], list[int]]:
    cp_influences: dict[int, list[tuple[int, float]]] = defaultdict(list)
    bone_names: list[str] = []
    bone_parents: list[int] = []
    bone_name_to_index: dict[str, int] = {}
    bone_nodes: dict[str, object] = {}

    for deformer_index in range(mesh.GetDeformerCount()):
        deformer = mesh.GetDeformer(deformer_index)
        if not isinstance(deformer, fbx.FbxSkin):
            continue

        for cluster_index in range(deformer.GetClusterCount()):
            cluster = deformer.GetCluster(cluster_index)
            link = cluster.GetLink()
            if link is None:
                continue

            bone_name = link.GetName()
            if bone_name not in bone_name_to_index:
                bone_name_to_index[bone_name] = len(bone_names)
                bone_names.append(bone_name)
                bone_parents.append(-1)
                bone_nodes[bone_name] = link

            bone_index = bone_name_to_index[bone_name]
            cp_indices = cluster.GetControlPointIndices() or []
            cp_weights = cluster.GetControlPointWeights() or []
            for cp_index, weight in zip(cp_indices, cp_weights):
                weight = float(weight)
                if weight <= 0:
                    continue
                cp_influences[int(cp_index)].append((bone_index, weight))

    for bone_name, bone_index in bone_name_to_index.items():
        parent = bone_nodes[bone_name].GetParent()
        while parent is not None:
            parent_name = parent.GetName()
            if parent_name in bone_name_to_index:
                bone_parents[bone_index] = bone_name_to_index[parent_name]
                break
            parent = parent.GetParent()

    return cp_influences, bone_names, bone_parents


def _extract_mesh_data(node, bake_transform: bool) -> FbxMeshData | None:
    mesh = node.GetMesh()
    if mesh is None:
        return None

    control_points = mesh.GetControlPoints()
    if control_points is None or mesh.GetControlPointsCount() == 0:
        return None

    uv_name = None
    if mesh.GetElementUVCount() > 0:
        uv_element = mesh.GetElementUV(0)
        if uv_element is not None:
            uv_name = uv_element.GetName()

    cp_influences, bone_names, bone_parents = _collect_skinning(mesh)
    local_matrix = _matrix_to_numpy(node.EvaluateLocalTransform())

    unique_vertices: list[np.ndarray] = []
    unique_normals: list[np.ndarray] = []
    unique_uvs: list[np.ndarray] = []
    source_control_points: list[int] = []
    unique_map: dict[tuple[float, ...], int] = {}
    triangles: list[list[int]] = []

    for polygon_index in range(mesh.GetPolygonCount()):
        polygon_size = mesh.GetPolygonSize(polygon_index)
        if polygon_size < 3:
            continue

        polygon_indices: list[int] = []
        for corner_index in range(polygon_size):
            cp_index = int(mesh.GetPolygonVertex(polygon_index, corner_index))
            cp = control_points[cp_index]
            position = np.array(
                [float(cp[0]), float(cp[1]), float(cp[2])],
                dtype=np.float32,
            )
            if bake_transform:
                position = _transform_position(position, local_matrix)

            normal_vec = fbx.FbxVector4()
            mesh.GetPolygonVertexNormal(polygon_index, corner_index, normal_vec)
            normal = np.array(
                [float(normal_vec[0]), float(normal_vec[1]), float(normal_vec[2])],
                dtype=np.float32,
            )
            if bake_transform:
                normal = _transform_normal(normal, local_matrix)

            uv = np.zeros(2, dtype=np.float32)
            if uv_name:
                uv_vec = fbx.FbxVector2()
                mapped, _ = mesh.GetPolygonVertexUV(
                    polygon_index, corner_index, uv_name, uv_vec
                )
                if mapped:
                    uv = np.array([float(uv_vec[0]), float(uv_vec[1])], dtype=np.float32)

            key = (
                float(round(position[0], 6)),
                float(round(position[1], 6)),
                float(round(position[2], 6)),
                float(round(normal[0], 6)),
                float(round(normal[1], 6)),
                float(round(normal[2], 6)),
                float(round(uv[0], 6)),
                float(round(uv[1], 6)),
            )
            vertex_index = unique_map.get(key)
            if vertex_index is None:
                vertex_index = len(unique_vertices)
                unique_map[key] = vertex_index
                unique_vertices.append(position)
                unique_normals.append(normal)
                unique_uvs.append(uv)
                source_control_points.append(cp_index)
            polygon_indices.append(vertex_index)

        for fan_index in range(1, len(polygon_indices) - 1):
            triangles.append(
                [
                    polygon_indices[0],
                    polygon_indices[fan_index],
                    polygon_indices[fan_index + 1],
                ]
            )

    if not unique_vertices or not triangles:
        return None

    max_bones = 4
    weights = np.zeros((len(unique_vertices), max_bones), dtype=np.float32)
    bone_indices = np.zeros((len(unique_vertices), max_bones), dtype=np.int32)
    for vertex_index, cp_index in enumerate(source_control_points):
        influences = sorted(
            cp_influences.get(cp_index, []),
            key=lambda item: item[1],
            reverse=True,
        )[:max_bones]
        total = sum(weight for _, weight in influences)
        if total <= 1e-8:
            continue
        for slot, (bone_index, weight) in enumerate(influences):
            bone_indices[vertex_index, slot] = bone_index
            weights[vertex_index, slot] = float(weight / total)

    translation, rotation, scale = _node_transform_fields(node)
    return FbxMeshData(
        name=node.GetName() or f"FBX_Mesh_{node.GetUniqueID()}",
        vertices=np.asarray(unique_vertices, dtype=np.float32),
        triangles=np.asarray(triangles, dtype=np.uint32),
        normals=np.asarray(unique_normals, dtype=np.float32),
        uvs=np.asarray(unique_uvs, dtype=np.float32),
        bone_names=bone_names,
        bone_indices=bone_indices,
        weights=weights,
        bone_parents=bone_parents,
        translation=translation,
        rotation=rotation,
        scale=scale,
    )


def extract_fbx_meshes(
    filepath: str | Path,
    *,
    bake_transforms: bool = False,
) -> list[FbxMeshData]:
    manager, scene = _load_scene(filepath)
    try:
        root = scene.GetRootNode()
        if root is None:
            return []

        meshes: list[FbxMeshData] = []
        for node in _iter_nodes(root):
            mesh_data = _extract_mesh_data(node, bake_transform=bake_transforms)
            if mesh_data is not None:
                meshes.append(mesh_data)
        return meshes
    finally:
        manager.Destroy()


def _to_nif_vertex_data(mesh: FbxMeshData) -> list[dict]:
    vertex_data: list[dict] = []
    for position, normal, uv in zip(mesh.vertices, mesh.normals, mesh.uvs):
        vertex_data.append(
            {
                "Vertex": {
                    "x": float(position[0]),
                    "y": float(position[1]),
                    "z": float(position[2]),
                },
                "Normal": {
                    "x": float(normal[0]),
                    "y": float(normal[1]),
                    "z": float(normal[2]),
                },
                "UV": {
                    "u": float(uv[0]),
                    "v": float(uv[1]),
                },
            }
        )
    return vertex_data


def _to_nif_triangles(mesh: FbxMeshData) -> list[dict]:
    return [
        {"v1": int(tri[0]), "v2": int(tri[1]), "v3": int(tri[2])}
        for tri in mesh.triangles
    ]


def import_fbx_to_shape(nif, block_id: int, filepath: str) -> int:
    if not HAS_FBX:
        _log.error("Autodesk FBX SDK is not installed")
        return -1

    block = nif.get_block(block_id)
    if not block:
        _log.error("Block %d not found", block_id)
        return -1

    try:
        meshes = extract_fbx_meshes(filepath, bake_transforms=True)
    except Exception as exc:
        _log.error("Failed to load FBX: %s", exc)
        return -1

    if not meshes:
        _log.error("No meshes found in FBX file")
        return -1

    merged_vertices: list[np.ndarray] = []
    merged_normals: list[np.ndarray] = []
    merged_uvs: list[np.ndarray] = []
    merged_triangles: list[list[int]] = []
    vertex_offset = 0
    for mesh in meshes:
        merged_vertices.extend(mesh.vertices)
        merged_normals.extend(mesh.normals)
        merged_uvs.extend(mesh.uvs)
        for tri in mesh.triangles:
            merged_triangles.append(
                [
                    int(tri[0]) + vertex_offset,
                    int(tri[1]) + vertex_offset,
                    int(tri[2]) + vertex_offset,
                ]
            )
        vertex_offset += len(mesh.vertices)

    merged_mesh = FbxMeshData(
        name=Path(filepath).stem,
        vertices=np.asarray(merged_vertices, dtype=np.float32),
        triangles=np.asarray(merged_triangles, dtype=np.uint32),
        normals=np.asarray(merged_normals, dtype=np.float32),
        uvs=np.asarray(merged_uvs, dtype=np.float32),
        bone_names=[],
        bone_indices=np.zeros((len(merged_vertices), 4), dtype=np.int32),
        weights=np.zeros((len(merged_vertices), 4), dtype=np.float32),
        bone_parents=[],
        translation={"x": 0.0, "y": 0.0, "z": 0.0},
        rotation=[
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ],
        scale=1.0,
    )

    block.set_field("Vertex Data", _to_nif_vertex_data(merged_mesh))
    block.set_field("Num Vertices", int(len(merged_mesh.vertices)))
    block.set_field("Triangles", _to_nif_triangles(merged_mesh))
    block.set_field("Num Triangles", int(len(merged_mesh.triangles)))

    _log.info(
        "Imported %d verts, %d tris from %s",
        len(merged_mesh.vertices),
        len(merged_mesh.triangles),
        filepath,
    )
    return int(len(merged_mesh.vertices))


def import_fbx_scene(nif, filepath: str) -> int:
    if not HAS_FBX:
        _log.error("Autodesk FBX SDK is not installed")
        return -1

    try:
        meshes = extract_fbx_meshes(filepath, bake_transforms=False)
    except Exception as exc:
        _log.error("Failed to load FBX: %s", exc)
        return -1

    if not meshes:
        _log.error("No meshes found in FBX file")
        return -1

    root_block = None
    for block in nif.blocks:
        if block.type_name in ("NiNode", "BSFadeNode"):
            root_block = block
            break

    if root_block is None:
        _log.error("No root NiNode found in NIF")
        return -1

    shapes_created = 0
    children = root_block.get_field("Children") or []
    for mesh in meshes:
        new_block = nif.add_block("BSTriShape")
        new_block.set_field("Name", mesh.name)
        new_block.set_field("Vertex Data", _to_nif_vertex_data(mesh))
        new_block.set_field("Num Vertices", int(len(mesh.vertices)))
        new_block.set_field("Triangles", _to_nif_triangles(mesh))
        new_block.set_field("Num Triangles", int(len(mesh.triangles)))
        new_block.set_field("Translation", mesh.translation)
        new_block.set_field("Rotation", mesh.rotation)
        new_block.set_field("Scale", mesh.scale)

        children.append({"block_id": new_block.block_id})
        shapes_created += 1
        _log.info(
            "Created BSTriShape '%s': %d verts, %d tris",
            mesh.name,
            len(mesh.vertices),
            len(mesh.triangles),
        )

    root_block.set_field("Children", children)
    root_block.set_field("Num Children", len(children))
    return shapes_created


def load_fbx_skin_data(path: str | Path) -> SkinData:
    meshes = extract_fbx_meshes(path, bake_transforms=True)
    if not meshes:
        raise ValueError(f"No meshes found in FBX file: {path}")

    all_vertices: list[np.ndarray] = []
    all_normals: list[np.ndarray] = []
    all_uvs: list[np.ndarray] = []
    all_triangles: list[np.ndarray] = []
    all_weights: list[np.ndarray] = []
    all_bone_indices: list[np.ndarray] = []
    global_bones: list[str] = []
    global_bone_map: dict[str, int] = {}
    global_parents: dict[str, str | None] = {}
    vertex_offset = 0

    for mesh in meshes:
        all_vertices.append(mesh.vertices)
        all_normals.append(mesh.normals)
        all_uvs.append(mesh.uvs)
        all_triangles.append(mesh.triangles + vertex_offset)
        vertex_offset += len(mesh.vertices)

        remapped_indices = np.zeros_like(mesh.bone_indices)
        for local_index, bone_name in enumerate(mesh.bone_names):
            if bone_name not in global_bone_map:
                global_bone_map[bone_name] = len(global_bones)
                global_bones.append(bone_name)
            global_index = global_bone_map[bone_name]
            mask = (mesh.bone_indices == local_index) & (mesh.weights > 0)
            remapped_indices[mask] = global_index

            parent_index = (
                mesh.bone_parents[local_index]
                if local_index < len(mesh.bone_parents)
                else -1
            )
            if parent_index >= 0 and parent_index < len(mesh.bone_names):
                global_parents[bone_name] = mesh.bone_names[parent_index]
            else:
                global_parents.setdefault(bone_name, None)

        all_weights.append(mesh.weights)
        all_bone_indices.append(remapped_indices)

    triangles = np.concatenate(all_triangles, axis=0)
    bone_parents = [
        global_bone_map[parent_name] if parent_name in global_bone_map else -1
        for parent_name in (global_parents.get(name) for name in global_bones)
    ]

    return SkinData(
        vertices=np.concatenate(all_vertices, axis=0),
        triangles=triangles,
        normals=np.concatenate(all_normals, axis=0),
        uvs=np.concatenate(all_uvs, axis=0),
        bone_names=global_bones,
        weights=np.concatenate(all_weights, axis=0),
        bone_indices=np.concatenate(all_bone_indices, axis=0),
        segment_ids=np.full(len(triangles), -1, dtype=np.int32),
        max_bones_per_vertex=4,
        bone_parents=bone_parents,
    )


__all__ = [
    "FbxMeshData",
    "extract_fbx_meshes",
    "import_fbx_scene",
    "import_fbx_to_shape",
    "load_fbx_skin_data",
]
