"""Per-part collision orchestrator — identifies parts by name, generates per-part shapes.

Walks the NIF hierarchy, pattern-matches node/shape names to part mappings,
and generates appropriate collision shapes per part. All shapes are combined
into a flat bhkListShape (no nesting).
"""
from __future__ import annotations

from dataclasses import dataclass, field

import numpy as np

from ..actions import OperationResult
from .collision import (
    HAVOK_SCALE_FO4, DEFAULT_RADIUS, FO4_LAYERS,
    _extract_vertices, _auto_discover_shapes, _create_convex_shape,
    _create_box_shape, _create_list_shape, _build_hierarchy,
    _find_collision_object, _collect_collision_subtree,
)
from .collision_shapes import (
    create_capsule_shape, create_cylinder_shape, create_sphere_shape,
    pick_best_primitive, create_optimized_collision,
)


@dataclass
class PartMapping:
    """Maps a name pattern to a collision shape type."""
    pattern: str       # case-insensitive substring match
    shape_type: str    # capsule, cylinder, sphere, box, convex_hull, auto, optimized


@dataclass
class DetectedPart:
    """A detected part with its matched pattern, shape type, and mesh data."""
    node_block_id: int
    node_name: str
    matched_pattern: str
    shape_type: str
    mesh_block_ids: list[int] = field(default_factory=list)
    vertex_count: int = 0


# Default weapon part presets
WEAPON_PRESETS: list[PartMapping] = [
    PartMapping("barrel", "capsule"),
    PartMapping("muzzle", "cylinder"),
    PartMapping("receiver", "optimized"),
    PartMapping("stock", "capsule"),
    PartMapping("grip", "convex_hull"),
    PartMapping("magazine", "box"),
    PartMapping("scope", "convex_hull"),
]


def _match_pattern(name: str, mappings: list[PartMapping]) -> PartMapping | None:
    """Find the first matching PartMapping for a given name (case-insensitive)."""
    name_lower = name.lower()
    for mapping in mappings:
        if mapping.pattern.lower() in name_lower:
            return mapping
    return None


def _collect_children_recursive(nif, node_id: int, type_filter: set[str] | None = None) -> list[int]:
    """Recursively collect child block IDs of a node, optionally filtered by type."""
    node = nif.get_block(node_id)
    if not node:
        return []
    children_refs = node.get_field("Children") or []
    result = []
    for ref in children_refs:
        ref_id = ref if isinstance(ref, int) else -1
        if isinstance(ref, dict):
            ref_id = int(ref.get("value", ref.get("Value", -1)))
        if ref_id < 0:
            continue
        child = nif.get_block(ref_id)
        if not child:
            continue
        if type_filter is None or child.type_name in type_filter:
            result.append(ref_id)
        # Recurse into child nodes
        if child.type_name in ("NiNode", "BSFadeNode", "BSLeafAnimNode"):
            result.extend(_collect_children_recursive(nif, ref_id, type_filter))
    return result


_MESH_TYPES = {"BSTriShape", "BSSubIndexTriShape", "BSMeshLODTriShape"}


def identify_parts(nif, root_id: int, mappings: list[PartMapping],
                    group_by: str = "node") -> list[DetectedPart]:
    """Match ``mappings`` against the hierarchy under ``root_id`` to find collision parts.

    ``group_by="node"`` matches NiNode names and combines their children;
    ``"shape"`` matches BSTriShape names individually.
    """
    parts: list[DetectedPart] = []

    if group_by == "node":
        _scan_nodes_for_parts(nif, root_id, mappings, parts)
    elif group_by == "shape":
        _scan_shapes_for_parts(nif, root_id, mappings, parts)

    return parts


def _scan_nodes_for_parts(nif, node_id: int, mappings: list[PartMapping],
                           parts: list[DetectedPart]) -> None:
    """Recursively scan NiNode names for pattern matches."""
    node = nif.get_block(node_id)
    if not node:
        return

    node_name = node.get_field("Name") or ""
    match = _match_pattern(node_name, mappings)

    if match and node.type_name in ("NiNode", "BSFadeNode", "BSLeafAnimNode"):
        # Collect all BSTriShape children (direct only)
        mesh_ids = _auto_discover_shapes(nif, node_id)
        if mesh_ids:
            vert_count = 0
            for mid in mesh_ids:
                verts = _extract_vertices(nif, mid)
                if verts is not None:
                    vert_count += len(verts)
            parts.append(DetectedPart(
                node_block_id=node_id,
                node_name=node_name,
                matched_pattern=match.pattern,
                shape_type=match.shape_type,
                mesh_block_ids=mesh_ids,
                vertex_count=vert_count,
            ))

    # Recurse into child nodes
    children_refs = node.get_field("Children") or []
    for ref in children_refs:
        ref_id = ref if isinstance(ref, int) else -1
        if isinstance(ref, dict):
            ref_id = int(ref.get("value", ref.get("Value", -1)))
        if ref_id < 0:
            continue
        child = nif.get_block(ref_id)
        if child and child.type_name in ("NiNode", "BSFadeNode", "BSLeafAnimNode"):
            _scan_nodes_for_parts(nif, ref_id, mappings, parts)


def _scan_shapes_for_parts(nif, node_id: int, mappings: list[PartMapping],
                            parts: list[DetectedPart]) -> None:
    """Recursively scan BSTriShape names for pattern matches."""
    node = nif.get_block(node_id)
    if not node:
        return

    children_refs = node.get_field("Children") or []
    for ref in children_refs:
        ref_id = ref if isinstance(ref, int) else -1
        if isinstance(ref, dict):
            ref_id = int(ref.get("value", ref.get("Value", -1)))
        if ref_id < 0:
            continue
        child = nif.get_block(ref_id)
        if not child:
            continue

        if child.type_name in _MESH_TYPES:
            shape_name = child.get_field("Name") or ""
            match = _match_pattern(shape_name, mappings)
            if match:
                verts = _extract_vertices(nif, ref_id)
                vert_count = len(verts) if verts is not None else 0
                parts.append(DetectedPart(
                    node_block_id=ref_id,
                    node_name=shape_name,
                    matched_pattern=match.pattern,
                    shape_type=match.shape_type,
                    mesh_block_ids=[ref_id],
                    vertex_count=vert_count,
                ))
        elif child.type_name in ("NiNode", "BSFadeNode", "BSLeafAnimNode"):
            _scan_shapes_for_parts(nif, ref_id, mappings, parts)


def _extract_triangles(nif, block_id: int) -> np.ndarray | None:
    """Extract triangle indices from a BSTriShape as Nx3 int array."""
    block = nif.get_block(block_id)
    if not block:
        return None
    tris = block.get_field("Triangles") or []
    if not tris:
        return None
    return np.array(
        [[int(t.get("v1", 0)), int(t.get("v2", 0)), int(t.get("v3", 0))]
         for t in tris],
        dtype=np.int32,
    )


def _create_shape_for_part(nif, part: DetectedPart,
                            radius: float = DEFAULT_RADIUS) -> list[int]:
    """Create collision shape(s) for a single detected part.

    Returns list of shape block IDs (may be multiple for optimized mode).
    """
    # Gather all vertices from the part's meshes
    all_verts = []
    for mid in part.mesh_block_ids:
        verts = _extract_vertices(nif, mid)
        if verts is not None and len(verts) >= 3:
            all_verts.append(verts)

    if not all_verts:
        return []

    combined = np.vstack(all_verts)

    shape_type = part.shape_type

    if shape_type == "capsule":
        sid = create_capsule_shape(nif, combined, radius)
        return [sid] if sid is not None else []

    elif shape_type == "cylinder":
        sid = create_cylinder_shape(nif, combined, radius)
        return [sid] if sid is not None else []

    elif shape_type == "sphere":
        result = create_sphere_shape(nif, combined, radius)
        if result is None:
            return []
        return [result[0]]  # transform_id is the top-level shape

    elif shape_type == "box":
        from .collision import _create_box_shape
        result = _create_box_shape(nif, combined, radius)
        if result is None:
            return []
        return [result[0]]  # transform_id

    elif shape_type == "convex_hull":
        sid = _create_convex_shape(nif, combined, radius)
        return [sid] if sid is not None else []

    elif shape_type == "auto":
        sid = pick_best_primitive(nif, combined, radius)
        return [sid] if sid is not None else []

    elif shape_type == "optimized":
        # Gather triangle indices for adjacency-based clustering
        all_tris = []
        vert_offset = 0
        for mid in part.mesh_block_ids:
            tris = _extract_triangles(nif, mid)
            verts = _extract_vertices(nif, mid)
            if tris is not None and verts is not None:
                all_tris.append(tris + vert_offset)
                vert_offset += len(verts)
            elif verts is not None:
                vert_offset += len(verts)

        tri_indices = np.vstack(all_tris) if all_tris else None
        return create_optimized_collision(nif, combined, tri_indices, radius)

    return []


def generate_per_part_collision(
    nif,
    root_id: int,
    parts: list[DetectedPart],
    layer: str = "STATIC",
    mass: float = 0.0,
    friction: float = 0.5,
    restitution: float = 0.4,
    radius: float = DEFAULT_RADIUS,
    replace: bool = True,
) -> OperationResult:
    """Generate collision for ``identify_parts`` results, combined into one flat bhkListShape."""
    if not parts:
        return OperationResult(False, "No parts provided for collision generation")

    root = nif.get_block(root_id)
    if not root:
        return OperationResult(False, f"Root node {root_id} not found")

    # Handle existing collision
    if replace:
        existing_coll = _find_collision_object(nif, root_id)
        if existing_coll is not None:
            subtree = _collect_collision_subtree(nif, existing_coll)
            root.set_field("Collision Object", -1)
            nif.remove_blocks(subtree)
            root = nif.get_block(root_id)

    # Generate shapes for each part
    all_shape_ids: list[int] = []
    created_ids: list[int] = []
    part_descriptions: list[str] = []

    for part in parts:
        shape_ids = _create_shape_for_part(nif, part, radius)
        if shape_ids:
            all_shape_ids.extend(shape_ids)
            created_ids.extend(shape_ids)
            part_descriptions.append(
                f"{part.node_name}({part.matched_pattern})={part.shape_type}"
            )

    if not all_shape_ids:
        return OperationResult(False, "Failed to create any collision shapes for parts")

    # Wrap in ListShape if multiple, or use directly if single
    if len(all_shape_ids) == 1:
        top_shape_id = all_shape_ids[0]
    else:
        top_shape_id = _create_list_shape(nif, all_shape_ids)
        if top_shape_id is None:
            return OperationResult(False, "Failed to create bhkListShape")
        created_ids.append(top_shape_id)

    # Build hierarchy
    result = _build_hierarchy(nif, root_id, top_shape_id, layer, mass, friction, restitution)
    if result is None:
        return OperationResult(False, "Failed to build collision hierarchy")

    coll_obj_id, rigid_body_id = result
    created_ids.extend([coll_obj_id, rigid_body_id])

    desc = ", ".join(part_descriptions)
    return OperationResult(
        True,
        f"Generated per-part collision: {desc} (layer={layer})",
        created_ids,
    )
