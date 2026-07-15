"""NIF operations — pure functions (nif, block_id, **kwargs) -> OperationResult."""
from .normals import fix_normals, flip_normals, normalize_normals
from .mesh import update_bounds, prune_degenerate_tris, flip_faces, flip_uvs_v
from .sanitize import (
    sanitize_links, remove_bogus_nodes, reorder_blocks,
    fix_invalid_names, fill_blank_controllers, sort_key_groups, sanitize_all,
)
from .tangent_space import generate_tangent_space
from .strips import triangulate, strippify
from .blocks import convert_block_type
from .copy import copy_blocks, collect_dependency_tree, deep_copy_block
from .collision import create_convex_hull, convert_collision_shape, generate_collision, remove_collision
from .collision_mesh import (
    create_mopp_shape, extract_mopp_geometry,
    create_compressed_mesh_shape, extract_compressed_mesh,
)
from .collision_parts import (
    identify_parts, generate_per_part_collision,
    PartMapping, DetectedPart, WEAPON_PRESETS,
)
from .skeleton import fix_bone_bounds, mirror_skeleton
from .texture import extract_texture_paths
