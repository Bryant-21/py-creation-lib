"""Export NIF files to FBX using Autodesk FBX SDK.

Follows Outfit Studio's FBXWrangler.cpp export pattern:
- Geometry: vertices, normals, UVs, triangles
- Skeleton: bone hierarchy with transforms
- Skinning: bone weights per vertex via FbxSkin/FbxCluster
- Materials: named stubs with diffuse color
- Axis conversion: NIF (x,y,z) -> FBX (y,z,x)
"""

import logging
import math
from dataclasses import dataclass

from .sdk import HAS_FBX, fbx

_log = logging.getLogger("fbx.nif_to_fbx")


@dataclass
class FbxExportOptions:
    include_skeleton: bool = True
    include_weights: bool = True
    include_materials: bool = True
    axis_conversion: bool = True


def export_nif_to_fbx(
    nif,
    output_path: str,
    options: FbxExportOptions | None = None,
) -> str | None:
    """Export a loaded NIF to FBX.

    Args:
        nif: NifFile instance with loaded data.
        output_path: Output .fbx file path.
        options: Export options.

    Returns:
        The output file path on success, or None on failure.
    """
    if not HAS_FBX:
        _log.error("Autodesk FBX SDK is not installed")
        return None

    if nif is None:
        _log.error("No NIF loaded to export")
        return None

    if options is None:
        options = FbxExportOptions()

    manager = fbx.FbxManager.Create()
    ios = fbx.FbxIOSettings.Create(manager, "IOSROOT")
    manager.SetIOSettings(ios)

    scene = fbx.FbxScene.Create(manager, "NifExportScene")

    try:
        # Build bone node map for skinning
        bone_node_map: dict[str, object] = {}

        # Export skeleton first (needed for skinning references)
        if options.include_skeleton:
            _export_skeleton(nif, scene, manager, bone_node_map)

        # Export geometry
        _export_geometry(
            nif, scene, manager, options, bone_node_map
        )

        # Export scene to file
        result = _write_fbx(manager, scene, output_path, options)
        if result:
            _log.info("Exported FBX to: %s", output_path)
            return output_path
        else:
            _log.error("FBX export failed")
            return None
    finally:
        manager.Destroy()


def export_worldspace_manifest_to_fbx(
    manifest,
    output_path: str,
    options: FbxExportOptions | None = None,
) -> str | None:
    """Export resolved worldspace placements to one composed FBX scene."""
    if not HAS_FBX:
        _log.error("Autodesk FBX SDK is not installed")
        return None

    if options is None:
        options = FbxExportOptions(
            include_skeleton=False,
            include_weights=False,
            include_materials=True,
        )

    from creation_lib.nif import NifFile

    manager = fbx.FbxManager.Create()
    ios = fbx.FbxIOSettings.Create(manager, "IOSROOT")
    manager.SetIOSettings(ios)
    scene = fbx.FbxScene.Create(manager, "WorldspaceExport")
    root_node = scene.GetRootNode()
    nif_cache = {}

    try:
        for placement in manifest.placements:
            mesh_path = str(placement.resolved_mesh_path)
            nif = nif_cache.get(mesh_path)
            if nif is None:
                nif = NifFile.load(mesh_path)
                nif_cache[mesh_path] = nif

            entry = placement.entry
            node_name = (
                f"ref_{int(entry.source_form_id) & 0xFFFFFFFF:08X}_"
                f"{_safe_node_name(entry.model_path)}"
            )
            placement_node = fbx.FbxNode.Create(manager, node_name)
            _apply_worldspace_transform(placement.transform, placement_node)
            root_node.AddChild(placement_node)

            _export_geometry(
                nif,
                scene,
                manager,
                options,
                {},
                parent_node=placement_node,
                name_prefix=f"{int(entry.source_form_id) & 0xFFFFFFFF:08X}_",
            )

        result = _write_fbx(manager, scene, output_path, options)
        if result:
            _log.info(
                "Exported worldspace FBX to %s (%d placements)",
                output_path,
                len(manifest.placements),
            )
            return output_path
        _log.error("Worldspace FBX export failed")
        return None
    finally:
        manager.Destroy()


def _build_weld_map(vertex_data_list) -> tuple[list[int], list[tuple[float, float, float]]]:
    """Build a position-weld map over NIF vertex data.

    NIF BSTriShape stores split verts — multiple verts sharing a position
    but with different UVs/normals/tangents at UV seams and hard edges.
    Max/FBX consumers want one control point per unique position with
    seams carried by per-face-corner normals/UVs.

    Returns:
        orig_to_welded: list[int] where orig_to_welded[nif_vert_idx] = welded_idx
        welded_positions: list[(x, y, z)] in NIF-space (pre-axis-swizzle)
    """
    orig_to_welded: list[int] = []
    welded_positions: list[tuple[float, float, float]] = []
    pos_to_welded: dict[tuple[int, int, int], int] = {}

    for vd in vertex_data_list:
        v = vd.get("Vertex") or {}
        x = float(v.get("x", 0.0))
        y = float(v.get("y", 0.0))
        z = float(v.get("z", 0.0))
        # Round to 6 decimals via integer key (avoids float dict hashing drift).
        key = (round(x * 1_000_000), round(y * 1_000_000), round(z * 1_000_000))
        welded_idx = pos_to_welded.get(key)
        if welded_idx is None:
            welded_idx = len(welded_positions)
            pos_to_welded[key] = welded_idx
            welded_positions.append((x, y, z))
        orig_to_welded.append(welded_idx)

    return orig_to_welded, welded_positions


def _export_geometry(
    nif,
    scene,
    manager,
    options,
    bone_node_map,
    *,
    parent_node=None,
    name_prefix: str = "",
):
    """Export all BSTriShape blocks as FBX meshes."""
    schema = nif.schema
    root_node = parent_node or scene.GetRootNode()

    for block in nif.blocks:
        if not schema.is_subtype_of(block.type_name, "BSTriShape"):
            continue

        vertex_data_list = block.get_field("Vertex Data") or []
        triangles_list = block.get_field("Triangles") or []
        if not vertex_data_list or not triangles_list:
            continue

        name = block.get_field("Name") or f"Shape_{block.block_id}"
        if isinstance(name, list):
            name = "".join(str(c) for c in name)

        # Create mesh
        export_name = f"{name_prefix}{name}"
        mesh = fbx.FbxMesh.Create(manager, str(export_name))
        n_verts = len(vertex_data_list)

        # --- Weld positions ---
        orig_to_welded, welded_positions = _build_weld_map(vertex_data_list)
        n_welded = len(welded_positions)

        # --- Control points (welded vertices) ---
        mesh.InitControlPoints(n_welded)
        for i, (x, y, z) in enumerate(welded_positions):
            # NIF (x,y,z) -> FBX (y,z,x) axis swizzle
            mesh.SetControlPointAt(fbx.FbxVector4(y, z, x), i)

        # Pre-compute triangle corner stream as original-NIF vert indices,
        # shared by normals/UVs index arrays.
        tri_corners_orig: list[int] = []
        for tri in triangles_list:
            tri_corners_orig.append(int(tri.get("v1", tri.get("V1", 0))))
            tri_corners_orig.append(int(tri.get("v2", tri.get("V2", 0))))
            tri_corners_orig.append(int(tri.get("v3", tri.get("V3", 0))))

        # --- Normals (per-polygon-vertex via index-to-direct) ---
        has_normals = any(vd.get("Normal") for vd in vertex_data_list)
        if has_normals:
            norm_element = mesh.CreateElementNormal()
            norm_element.SetMappingMode(
                fbx.FbxLayerElement.EMappingMode.eByPolygonVertex
            )
            norm_element.SetReferenceMode(
                fbx.FbxLayerElement.EReferenceMode.eIndexToDirect
            )
            direct = norm_element.GetDirectArray()
            for vd in vertex_data_list:
                n = vd.get("Normal")
                if n:
                    nx = float(n.get("x", 0))
                    ny = float(n.get("y", 0))
                    nz = float(n.get("z", 0))
                    direct.Add(fbx.FbxVector4(ny, nz, nx))
                else:
                    direct.Add(fbx.FbxVector4(0, 0, 1))
            index_array = norm_element.GetIndexArray()
            index_array.SetCount(len(tri_corners_orig))
            for corner, orig_vi in enumerate(tri_corners_orig):
                index_array.SetAt(corner, orig_vi)

        # --- UVs (per-polygon-vertex via index-to-direct) ---
        has_uvs = any(vd.get("UV") for vd in vertex_data_list)
        if has_uvs:
            uv_element = mesh.CreateElementUV(f"{export_name}UV")
            uv_element.SetMappingMode(
                fbx.FbxLayerElement.EMappingMode.eByPolygonVertex
            )
            uv_element.SetReferenceMode(
                fbx.FbxLayerElement.EReferenceMode.eIndexToDirect
            )
            direct = uv_element.GetDirectArray()
            for vd in vertex_data_list:
                uv = vd.get("UV")
                if uv:
                    u = float(uv.get("u", 0))
                    v_coord = float(uv.get("v", 0))
                    direct.Add(fbx.FbxVector2(u, v_coord))
                else:
                    direct.Add(fbx.FbxVector2(0, 0))
            index_array = uv_element.GetIndexArray()
            index_array.SetCount(len(tri_corners_orig))
            for corner, orig_vi in enumerate(tri_corners_orig):
                index_array.SetAt(corner, orig_vi)

        # --- Triangles (welded indices) ---
        for tri in triangles_list:
            v1 = int(tri.get("v1", tri.get("V1", 0)))
            v2 = int(tri.get("v2", tri.get("V2", 0)))
            v3 = int(tri.get("v3", tri.get("V3", 0)))
            mesh.BeginPolygon()
            mesh.AddPolygon(orig_to_welded[v1])
            mesh.AddPolygon(orig_to_welded[v2])
            mesh.AddPolygon(orig_to_welded[v3])
            mesh.EndPolygon()

        # Create node and attach mesh
        mesh_node = fbx.FbxNode.Create(manager, str(export_name))
        mesh_node.SetNodeAttribute(mesh)

        # Set shape transform
        _apply_block_transform(block, mesh_node)

        root_node.AddChild(mesh_node)

        # --- Material stub ---
        if options.include_materials:
            _add_material_stub(nif, block, scene, manager, mesh_node)

        # --- Skinning ---
        if options.include_weights and bone_node_map:
            _add_skinning(nif, block, scene, manager, mesh, mesh_node,
                          root_node, bone_node_map, orig_to_welded)

        _log.info(
            "Exported shape '%s': %d verts (%d welded), %d tris",
            name, n_verts, n_welded, len(triangles_list),
        )


def _export_skeleton(nif, scene, manager, bone_node_map):
    """Export NIF bone hierarchy as FBX skeleton."""
    schema = nif.schema
    root_node = scene.GetRootNode()

    # Create skeleton root
    skel_attr = fbx.FbxSkeleton.Create(manager, "NifSkeleton")
    skel_attr.SetSkeletonType(fbx.FbxSkeleton.EType.eRoot)
    skel_node = fbx.FbxNode.Create(manager, "NifSkeleton")
    skel_node.SetNodeAttribute(skel_attr)
    root_node.AddChild(skel_node)

    # Find all NiNode blocks that are bones (referenced by skin instances)
    bone_ids = _collect_bone_ids(nif)

    # Walk the NiNode hierarchy and create bones
    for block in nif.blocks:
        if block.type_name not in ("NiNode", "BSFadeNode"):
            continue

        block_id = block.block_id
        if block_id not in bone_ids and block_id != 0:
            continue

        name = block.get_field("Name") or f"Bone_{block_id}"
        if isinstance(name, list):
            name = "".join(str(c) for c in name)
        name = str(name)

        if name in bone_node_map:
            continue

        bone_attr = fbx.FbxSkeleton.Create(manager, name)
        bone_attr.SetSkeletonType(fbx.FbxSkeleton.EType.eLimbNode)
        bone_attr.Size.Set(1.0)

        bone_node = fbx.FbxNode.Create(manager, name)
        bone_node.SetNodeAttribute(bone_attr)

        # Apply transform
        _apply_block_transform(block, bone_node)

        bone_node_map[name] = bone_node

    # Build parent-child hierarchy
    _build_bone_hierarchy(nif, skel_node, bone_node_map, bone_ids)


def _collect_bone_ids(nif) -> set[int]:
    """Collect all block IDs referenced as bones by skin instances."""
    bone_ids = set()
    for block in nif.blocks:
        if block.type_name in (
            "NiSkinInstance", "BSDismemberSkinInstance",
            "BSSkin::Instance",
        ):
            bone_refs = block.get_field("Bones") or []
            for ref in bone_refs:
                bone_id = int(ref) if isinstance(ref, (int, float)) else -1
                if bone_id >= 0:
                    bone_ids.add(bone_id)
                    # Also add parents up the chain
                    _collect_parent_bones(nif, bone_id, bone_ids)
    return bone_ids


def _collect_parent_bones(nif, block_id: int, bone_ids: set[int]):
    """Walk up the NiNode hierarchy and add all parents as bones."""
    for block in nif.blocks:
        if block.type_name not in ("NiNode", "BSFadeNode"):
            continue
        children = block.get_field("Children") or []
        for child_ref in children:
            child_id = int(child_ref) if isinstance(child_ref, (int, float)) else -1
            if isinstance(child_ref, dict):
                child_id = int(child_ref.get("block_id", child_ref.get("Ref", -1)))
            if child_id == block_id:
                parent_id = block.block_id
                if parent_id not in bone_ids:
                    bone_ids.add(parent_id)
                    _collect_parent_bones(nif, parent_id, bone_ids)
                return


def _build_bone_hierarchy(nif, skel_node, bone_node_map, bone_ids):
    """Attach bone nodes in parent-child hierarchy."""
    attached = set()

    for block in nif.blocks:
        if block.type_name not in ("NiNode", "BSFadeNode"):
            continue

        parent_name = block.get_field("Name") or f"Bone_{block.block_id}"
        if isinstance(parent_name, list):
            parent_name = "".join(str(c) for c in parent_name)
        parent_name = str(parent_name)

        parent_fbx = bone_node_map.get(parent_name)
        if parent_fbx is None:
            continue

        children = block.get_field("Children") or []
        for child_ref in children:
            child_id = int(child_ref) if isinstance(child_ref, (int, float)) else -1
            if isinstance(child_ref, dict):
                child_id = int(child_ref.get("block_id", child_ref.get("Ref", -1)))
            if child_id < 0 or child_id not in bone_ids:
                continue

            child_block = nif.get_block(child_id)
            if child_block is None:
                continue

            child_name = child_block.get_field("Name") or f"Bone_{child_id}"
            if isinstance(child_name, list):
                child_name = "".join(str(c) for c in child_name)
            child_name = str(child_name)

            child_fbx = bone_node_map.get(child_name)
            if child_fbx and child_name not in attached:
                parent_fbx.AddChild(child_fbx)
                attached.add(child_name)

    # Attach any un-parented bones to the skeleton root
    for name, node in bone_node_map.items():
        if name not in attached:
            skel_node.AddChild(node)
            attached.add(name)


def _add_skinning(nif, shape_block, scene, manager, mesh, mesh_node,
                  root_node, bone_node_map, orig_to_welded):
    """Add skin deformer with bone weight clusters to a mesh.

    orig_to_welded maps NIF vert idx → welded control-point idx so cluster
    indices reference the deduplicated control points. Split verts that
    collapse to the same welded idx (same position, different UV/normal)
    have identical skin weights by construction, so we dedup via a set
    rather than summing.
    """
    skin_ref = (
        shape_block.get_field("Skin Instance")
        or shape_block.get_field("Skin")
    )
    if skin_ref is None:
        return

    if isinstance(skin_ref, dict):
        skin_id = int(skin_ref.get("block_id", skin_ref.get("Ref", -1)))
    elif isinstance(skin_ref, (int, float)):
        skin_id = int(skin_ref)
    else:
        return

    if skin_id < 0:
        return

    skin_block = nif.get_block(skin_id)
    if skin_block is None:
        return

    # Get bone names from skin instance
    bone_refs = skin_block.get_field("Bones") or []
    bone_names = []
    for ref in bone_refs:
        bone_id = int(ref) if isinstance(ref, (int, float)) else -1
        if bone_id >= 0:
            bone_block = nif.get_block(bone_id)
            if bone_block:
                name = bone_block.get_field("Name") or f"Bone_{bone_id}"
                if isinstance(name, int):
                    name = nif.get_string(name) or f"Bone_{bone_id}"
                bone_names.append(str(name))
            else:
                bone_names.append(f"Bone_{bone_id}")
        else:
            bone_names.append(f"Bone_{len(bone_names)}")

    if not bone_names:
        return

    # Extract per-vertex bone weights and indices
    vertex_data_list = shape_block.get_field("Vertex Data") or []

    # Build per-bone weight maps keyed by welded idx:
    # bone_index -> {welded_vi: weight}
    per_bone_weights: dict[int, dict[int, float]] = {}

    def _record(bone_idx: int, welded_vi: int, weight: float) -> None:
        if weight <= 0:
            return
        bucket = per_bone_weights.setdefault(bone_idx, {})
        existing = bucket.get(welded_vi)
        if existing is None or weight > existing:
            # All original verts mapped to this welded position share
            # a skin weight by construction — take max to be safe against
            # tiny float drift; never sum (would double-count).
            bucket[welded_vi] = weight

    for vi, vd in enumerate(vertex_data_list):
        welded_vi = orig_to_welded[vi] if vi < len(orig_to_welded) else vi
        bw_list = vd.get("Bone Weights") or vd.get("BoneWeights") or []
        bi_list = vd.get("Bone Indices") or []

        if isinstance(bw_list, list) and bw_list:
            if isinstance(bw_list[0], dict):
                # Combined format: [{"index": N, "weight": F}, ...]
                for bw in bw_list:
                    idx = int(bw.get("index", bw.get("Index", 0)))
                    weight = float(bw.get("weight", bw.get("Weight", 0)))
                    _record(idx, welded_vi, weight)
            else:
                # Separate flat lists
                for j in range(min(4, len(bw_list))):
                    weight = float(bw_list[j])
                    if weight > 0 and j < len(bi_list):
                        idx = int(bi_list[j])
                        _record(idx, welded_vi, weight)

    if not per_bone_weights:
        return

    # Create FBX skin deformer
    shape_name = shape_block.get_field("Name") or f"Shape_{shape_block.block_id}"
    if isinstance(shape_name, list):
        shape_name = "".join(str(c) for c in shape_name)

    skin = fbx.FbxSkin.Create(scene, f"{shape_name}_skin")

    for bone_idx, bone_name in enumerate(bone_names):
        if bone_idx not in per_bone_weights:
            continue

        joint_node = bone_node_map.get(bone_name)
        if joint_node is None:
            continue

        cluster = fbx.FbxCluster.Create(scene, f"{bone_name}_cluster")
        cluster.SetLink(joint_node)
        cluster.SetLinkMode(fbx.FbxCluster.ELinkMode.eTotalOne)

        for welded_vi, weight in per_bone_weights[bone_idx].items():
            cluster.AddControlPointIndex(welded_vi, weight)

        # Set transform matrices
        cluster.SetTransformMatrix(
            mesh_node.EvaluateGlobalTransform()
        )
        cluster.SetTransformLinkMatrix(
            joint_node.EvaluateGlobalTransform()
        )

        skin.AddCluster(cluster)

    if skin.GetClusterCount() > 0:
        mesh.AddDeformer(skin)


def _add_material_stub(nif, shape_block, scene, manager, mesh_node):
    """Add a named material stub to the mesh node."""
    # Look for BSLightingShaderProperty or BSEffectShaderProperty
    props = shape_block.get_field("Properties") or []
    shader_name = None
    for prop_ref in props:
        prop_id = int(prop_ref) if isinstance(prop_ref, (int, float)) else -1
        if isinstance(prop_ref, dict):
            prop_id = int(prop_ref.get("block_id", prop_ref.get("Ref", -1)))
        if prop_id < 0:
            continue
        prop_block = nif.get_block(prop_id)
        if prop_block and prop_block.type_name in (
            "BSLightingShaderProperty", "BSEffectShaderProperty",
        ):
            shader_name = prop_block.get_field("Name")
            if isinstance(shader_name, list):
                shader_name = "".join(str(c) for c in shader_name)
            break

    # Also check the BS Properties array (FO4 format)
    if shader_name is None:
        bs_props = shape_block.get_field("BS Properties") or []
        for prop_ref in bs_props:
            prop_id = int(prop_ref) if isinstance(prop_ref, (int, float)) else -1
            if prop_id < 0:
                continue
            prop_block = nif.get_block(prop_id)
            if prop_block and prop_block.type_name in (
                "BSLightingShaderProperty", "BSEffectShaderProperty",
            ):
                shader_name = prop_block.get_field("Name")
                if isinstance(shader_name, list):
                    shader_name = "".join(str(c) for c in shader_name)
                break

    if not shader_name:
        shape_name = shape_block.get_field("Name") or f"Shape_{shape_block.block_id}"
        if isinstance(shape_name, list):
            shape_name = "".join(str(c) for c in shape_name)
        shader_name = f"{shape_name}_mat"

    material = fbx.FbxSurfacePhong.Create(scene, str(shader_name))
    mesh_node.AddMaterial(material)


def _apply_block_transform(block, fbx_node):
    """Apply a NIF block's Translation/Rotation/Scale to an FBX node."""
    trans = block.get_field("Translation")
    if trans:
        x = float(trans.get("x", 0))
        y = float(trans.get("y", 0))
        z = float(trans.get("z", 0))
        # NIF (x,y,z) -> FBX (y,z,x)
        fbx_node.LclTranslation.Set(fbx.FbxDouble3(y, z, x))

    rotation = block.get_field("Rotation")
    if rotation:
        # NIF stores 3x3 rotation matrix — convert to Euler degrees
        euler = _rotation_matrix_to_euler(rotation)
        if euler:
            fbx_node.LclRotation.Set(fbx.FbxDouble3(*euler))

    scale = block.get_field("Scale")
    if scale is not None:
        s = float(scale)
        fbx_node.LclScaling.Set(fbx.FbxDouble3(s, s, s))


def _apply_worldspace_transform(transform, fbx_node):
    x, y, z = transform.position
    fbx_node.LclTranslation.Set(fbx.FbxDouble3(y, z, x))

    rx, ry, rz = transform.rotation
    fbx_node.LclRotation.Set(
        fbx.FbxDouble3(
            -math.degrees(rx),
            -math.degrees(ry),
            -math.degrees(rz),
        )
    )

    scale = float(transform.scale)
    fbx_node.LclScaling.Set(fbx.FbxDouble3(scale, scale, scale))


def _safe_node_name(value: str) -> str:
    name = value.replace("\\", "/").rsplit("/", 1)[-1].rsplit(".", 1)[0]
    cleaned = "".join(ch if ch.isalnum() or ch in {"_", "-"} else "_" for ch in name)
    return cleaned or "placement"


def _rotation_matrix_to_euler(rot) -> tuple[float, float, float] | None:
    """Convert NIF 3x3 rotation matrix to Euler angles in degrees.

    NIF rotation format: dict with m11..m33 or nested list.
    Applies axis swizzle to match FBX coordinate system.
    """
    if isinstance(rot, dict):
        # Dict format: m11, m12, ... m33
        m = [
            [float(rot.get("m11", 1)), float(rot.get("m12", 0)), float(rot.get("m13", 0))],
            [float(rot.get("m21", 0)), float(rot.get("m22", 1)), float(rot.get("m23", 0))],
            [float(rot.get("m31", 0)), float(rot.get("m32", 0)), float(rot.get("m33", 1))],
        ]
    elif isinstance(rot, list) and len(rot) >= 3:
        m = []
        for row in rot[:3]:
            if isinstance(row, list) and len(row) >= 3:
                m.append([float(row[0]), float(row[1]), float(row[2])])
            else:
                return None
    else:
        return None

    # Apply NIF→FBX axis swizzle to rotation matrix
    # NIF (x,y,z) → FBX (y,z,x), so we permute rows and columns
    # Original: R_nif, Swizzled: P * R_nif * P^-1 where P swaps axes
    sm = [
        [m[1][1], m[1][2], m[1][0]],
        [m[2][1], m[2][2], m[2][0]],
        [m[0][1], m[0][2], m[0][0]],
    ]

    # Extract Euler angles (XYZ order) from rotation matrix
    sy = math.sqrt(sm[0][0] ** 2 + sm[1][0] ** 2)
    singular = sy < 1e-6

    if not singular:
        rx = math.atan2(sm[2][1], sm[2][2])
        ry = math.atan2(-sm[2][0], sy)
        rz = math.atan2(sm[1][0], sm[0][0])
    else:
        rx = math.atan2(-sm[1][2], sm[1][1])
        ry = math.atan2(-sm[2][0], sy)
        rz = 0

    return (
        math.degrees(rx),
        math.degrees(ry),
        math.degrees(rz),
    )


def _write_fbx(manager, scene, output_path: str, options=None) -> bool:
    """Write the FBX scene to file."""
    # Apply axis conversion after all geometry/skeleton is built
    if options and options.axis_conversion:
        fbx.FbxAxisSystem.Max.ConvertScene(scene)

    exporter = fbx.FbxExporter.Create(manager, "")
    ios = manager.GetIOSettings()

    # Use native binary FBX format (index 0)
    format_index = manager.GetIOPluginRegistry().GetNativeWriterFormat()

    if not exporter.Initialize(output_path, format_index, ios):
        _log.error("Failed to initialize FBX exporter for: %s", output_path)
        exporter.Destroy()
        return False

    # Configure export settings (use module constants, not raw strings)
    ios.SetBoolProp(fbx.EXP_FBX_MATERIAL, True)
    ios.SetBoolProp(fbx.EXP_FBX_TEXTURE, True)
    ios.SetBoolProp(fbx.EXP_FBX_EMBEDDED, False)
    ios.SetBoolProp(fbx.EXP_FBX_SHAPE, True)
    ios.SetBoolProp(fbx.EXP_FBX_ANIMATION, True)
    ios.SetBoolProp(fbx.EXP_FBX_GLOBAL_SETTINGS, True)

    # Set compatible version (FBX 2014)
    exporter.SetFileExportVersion(
        "FBX201400",
        fbx.FbxSceneRenamer.ERenamingMode.eNone,
    )

    result = exporter.Export(scene)
    exporter.Destroy()
    return result
