"""Block copy and dependency resolution operations."""
from __future__ import annotations
import copy
from typing import Any

from ..nif_file import NifFile, NifBlock
from ..schema import get_schema


def deep_copy_block(block: NifBlock) -> NifBlock:
    """Create a deep copy of a block with all field values cloned."""
    new_fields = []
    new_map = {}
    for name, val in block.fields:
        copied = copy.deepcopy(val)
        new_fields.append((name, copied))
        new_map[name] = copied
    new_block = NifBlock(
        block_id=block.block_id,
        type_name=block.type_name,
        fields=new_fields,
        _field_map=new_map,
    )
    return new_block


def _get_forward_refs(block: NifBlock, schema) -> list[int]:
    """Return Ref/Ptr indices that are true dependencies (not back-references).

    Excludes fields that point upward/sideways in the scene graph:
    - Target: controllers point to the object they control (upward)
    - Manager: NiControllerSequence points back to NiControllerManager
    - Scene: NiDefaultAVObjectPalette points to root node
    - Extra Targets: NiMultiTargetTransformController points to controlled nodes
    - AV Object: palette entries point to scene graph nodes
    """
    back_ref_fields = {
        "Target", "Manager", "Scene", "Extra Targets",
        "Affected Nodes", "Affected Node Pointers",
    }
    ref_types = {"Ref", "Ptr"}
    refs = []
    all_fields = schema.get_all_fields(block.type_name)
    field_defs = {f.name: f for f in all_fields}
    for f in all_fields:
        if f.suffix:
            field_defs[f"{f.name}:{f.suffix}"] = f

    def _collect(val, fdef):
        """Collect Ref/Ptr values, recursing into structs."""
        if fdef.type in ref_types or fdef.template in ref_types:
            if isinstance(val, int) and val >= 0:
                refs.append(val)
            elif isinstance(val, list):
                refs.extend(v for v in val if isinstance(v, int) and v >= 0)
        elif isinstance(val, dict):
            _walk_struct(val, fdef.type)
        elif isinstance(val, list) and val and isinstance(val[0], dict):
            for item in val:
                _walk_struct(item, fdef.type)

    def _walk_struct(struct_val, struct_type):
        """Walk a struct's fields for Ref/Ptr values, respecting back-ref exclusions."""
        struct_def = schema.structs.get(struct_type)
        if struct_def is None:
            return
        struct_fdefs = {f.name: f for f in struct_def.fields}
        for key, val in struct_val.items():
            if key in back_ref_fields:
                continue
            sfdef = struct_fdefs.get(key)
            if sfdef is None:
                continue
            _collect(val, sfdef)

    for name, val in block.fields:
        if name in back_ref_fields:
            continue
        fdef = field_defs.get(name)
        if fdef is None:
            continue
        _collect(val, fdef)

    # Special handling for NiDefaultAVObjectPalette: skip all Objs AV Object refs
    if block.type_name == "NiDefaultAVObjectPalette":
        return []

    return refs


def collect_dependency_tree(nif: NifFile, block_ids: list[int]) -> list[int]:
    """Walk forward Ref/Ptr links and return all transitively referenced block IDs,
    topologically sorted (dependencies before dependents).

    Excludes back-references (Target, Manager, Scene, palette entries) to prevent
    circular dependencies when copying between NIFs.
    """
    visited: set[int] = set()
    order: list[int] = []

    def _walk(bid: int) -> None:
        if bid in visited or bid < 0 or bid >= len(nif.blocks):
            return
        visited.add(bid)
        block = nif.blocks[bid]
        refs = _get_forward_refs(block, nif.schema)
        for ref in refs:
            _walk(ref)
        order.append(bid)

    for bid in block_ids:
        _walk(bid)

    return order


def copy_blocks(
    source: NifFile,
    block_ids: list[int],
    target: NifFile,
    attach_to: int | None = None,
) -> dict[int, int]:
    """Copy blocks from source to target NIF.

    Resolves dependency tree, deep-copies blocks, remaps Ref/Ptr indices.
    Returns mapping of source block_id -> new target block_id.
    """
    schema = source.schema
    all_deps = collect_dependency_tree(source, block_ids)

    # Map source IDs to new target IDs
    id_map: dict[int, int] = {}
    new_blocks: list[NifBlock] = []
    next_id = len(target.blocks)

    for src_id in all_deps:
        src_block = source.blocks[src_id]
        new_block = deep_copy_block(src_block)
        new_block.block_id = next_id
        id_map[src_id] = next_id
        new_blocks.append(new_block)
        next_id += 1

    # Remap Ref/Ptr in copied blocks (recursing into structs)
    ref_types = {"Ref", "Ptr"}

    def _remap_value(val, fdef):
        """Remap Ref/Ptr values, recursing into structs. Returns new value."""
        if fdef.type in ref_types:
            if isinstance(val, int):
                return id_map.get(val, -1)
            elif isinstance(val, list):
                return [id_map.get(v, -1) if isinstance(v, int) else v for v in val]
        elif isinstance(val, dict):
            return _remap_struct(val, fdef.type)
        elif isinstance(val, list) and val and isinstance(val[0], dict):
            return [_remap_struct(item, fdef.type) for item in val]
        return val

    def _remap_struct(struct_val, struct_type):
        """Remap Refs inside a struct dict."""
        struct_def = schema.structs.get(struct_type)
        if struct_def is None:
            return struct_val
        struct_fdefs = {f.name: f for f in struct_def.fields}
        new_struct = dict(struct_val)
        for key, val in struct_val.items():
            sfdef = struct_fdefs.get(key)
            if sfdef is None:
                continue
            new_struct[key] = _remap_value(val, sfdef)
        return new_struct

    for block in new_blocks:
        all_fields = schema.get_all_fields(block.type_name)
        field_defs = {}
        for f in all_fields:
            field_defs[f.name] = f
            if f.suffix:
                field_defs[f"{f.name}:{f.suffix}"] = f

        for i, (name, val) in enumerate(block.fields):
            fdef = field_defs.get(name)
            if fdef is None:
                continue
            new_val = _remap_value(val, fdef)
            if new_val is not val:
                block.fields[i] = (name, new_val)
                block._field_map[name] = new_val

    # Add to target
    for block in new_blocks:
        target.blocks.append(block)
        target._update_header_for_new_block(block.type_name)

    # Attach copied root blocks as children if requested
    if attach_to is not None and attach_to >= 0:
        parent = target.get_block(attach_to)
        if parent:
            for src_id in block_ids:
                if src_id in id_map:
                    new_id = id_map[src_id]
                    children = parent.get_field("Children")
                    if isinstance(children, list):
                        children.append(new_id)
                        parent.set_field("Children", children)
                    num = parent.get_field("Num Children")
                    if isinstance(num, int):
                        parent.set_field("Num Children", num + 1)

    return id_map
