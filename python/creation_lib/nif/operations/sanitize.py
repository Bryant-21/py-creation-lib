"""Sanitization and optimization operations."""
from ..actions import OperationResult


def sanitize_links(nif) -> OperationResult:
    """Set invalid Ref/Ptr values to -1."""
    num_blocks = len(nif.blocks)
    fixed = 0
    schema = nif.schema
    for block in nif.blocks:
        all_fields = schema.get_all_fields(block.type_name)
        fdef_map = {}
        for f in all_fields:
            key = f"{f.name}:{f.suffix}" if f.suffix else f.name
            fdef_map[key] = f
            fdef_map[f.name] = f
        for name, value in block.fields:
            fdef = fdef_map.get(name)
            if fdef and fdef.type in ("Ref", "Ptr"):
                if isinstance(value, int) and value >= num_blocks:
                    block.set_field(name, -1)
                    fixed += 1
    return OperationResult(True, f"Sanitized {fixed} invalid link(s)")


def remove_bogus_nodes(nif) -> OperationResult:
    """Remove empty NiNodes with no children and no extra data."""
    schema = nif.schema
    to_remove = []
    for block in nif.blocks:
        if block.block_id == 0:
            continue
        if not schema.is_subtype_of(block.type_name, "NiNode"):
            continue
        children = block.get_field("Children") or []
        extra = block.get_field("Extra Data List") or []
        valid_children = [r for r in children if _get_ref_id(r) >= 0]
        valid_extra = [r for r in extra if _get_ref_id(r) >= 0]
        if not valid_children and not valid_extra:
            to_remove.append(block.block_id)
    if to_remove:
        nif.remove_blocks(to_remove)
    return OperationResult(True, f"Removed {len(to_remove)} empty node(s)", to_remove)


def reorder_blocks(nif) -> OperationResult:
    """Reorder blocks in depth-first scene graph order starting from root.

    Fixes parent/child ordering so that parents always precede children.
    Updates all Ref/Ptr fields to reflect the new block indices.
    """
    if not nif.blocks:
        return OperationResult(True, "No blocks to reorder")

    schema = nif.schema
    visited = []
    visited_set = set()

    def _visit(bid: int):
        if bid in visited_set or bid < 0 or bid >= len(nif.blocks):
            return
        visited_set.add(bid)
        visited.append(bid)
        block = nif.blocks[bid]
        for ref in block.get_refs(schema):
            _visit(ref)

    # Start from root (block 0), then pick up any orphans
    _visit(0)
    for i in range(len(nif.blocks)):
        if i not in visited_set:
            _visit(i)

    if visited == list(range(len(nif.blocks))):
        return OperationResult(True, "Blocks already in correct order")

    # Build old->new index mapping
    old_to_new = {old_id: new_id for new_id, old_id in enumerate(visited)}
    new_blocks = [nif.blocks[old_id] for old_id in visited]

    # Update block_ids
    for new_id, block in enumerate(new_blocks):
        block.block_id = new_id

    # Remap all Ref/Ptr values in all blocks
    _remap_refs(new_blocks, schema, old_to_new)

    nif.blocks = new_blocks

    # Update header
    nif.header.num_blocks = len(new_blocks)
    if nif.header.block_type_index:
        nif.header.block_type_index = [nif.header.block_type_index[old_id] for old_id in visited]
    if nif.header.block_sizes:
        nif.header.block_sizes = [nif.header.block_sizes[old_id]
                                   for old_id in visited
                                   if old_id < len(nif.header.block_sizes)]

    return OperationResult(True, f"Reordered {len(new_blocks)} blocks")


def fix_invalid_names(nif) -> OperationResult:
    """Replace invalid or empty Name fields with sensible defaults."""
    fixed = 0
    modified = []
    for block in nif.blocks:
        name = block.get_field("Name")
        if name is None:
            continue
        if not isinstance(name, str) or not name.strip():
            default_name = f"{block.type_name}_{block.block_id}"
            block.set_field("Name", default_name)
            fixed += 1
            modified.append(block.block_id)
    return OperationResult(True, f"Fixed {fixed} invalid name(s)", modified)


def fill_blank_controllers(nif) -> OperationResult:
    """Set unlinked Controller fields to -1 (None).

    Some blocks have Controller refs pointing at deleted or invalid blocks.
    """
    num_blocks = len(nif.blocks)
    fixed = 0
    modified = []
    for block in nif.blocks:
        ctrl = block.get_field("Controller")
        if ctrl is None:
            continue
        ref_id = _get_ref_id(ctrl)
        if ref_id >= num_blocks:
            block.set_field("Controller", -1)
            fixed += 1
            modified.append(block.block_id)
    return OperationResult(True, f"Fixed {fixed} blank controller(s)", modified)


def sort_key_groups(nif, block_id: int | None = None) -> OperationResult:
    """Sort NiKeyframeData key groups by time for correct interpolation."""
    targets = ([nif.get_block(block_id)] if block_id is not None
               else nif.find_blocks("NiKeyframeData"))
    count = 0
    modified = []
    for block in targets:
        if block is None:
            continue
        changed = False
        for group_name in ("Translations", "Rotations", "Scales",
                           "XYZ Rotations", "Quaternion Keys"):
            keys = block.get_field(group_name)
            if not isinstance(keys, list) or len(keys) < 2:
                continue
            sorted_keys = sorted(keys, key=lambda k: float(k.get("Time", k.get("time", 0))))
            if sorted_keys != keys:
                block.set_field(group_name, sorted_keys)
                changed = True
        if changed:
            count += 1
            modified.append(block.block_id)
    return OperationResult(True, f"Sorted key groups on {count} block(s)", modified)


def sanitize_all(nif) -> OperationResult:
    """Run all sanitization passes in recommended order."""
    results = []
    warnings = []
    for fn in (sanitize_links, remove_bogus_nodes, fix_invalid_names,
               fill_blank_controllers, reorder_blocks):
        r = fn(nif)
        results.append(r)
        if r.warnings:
            warnings.extend(r.warnings)

    total_modified = set()
    for r in results:
        total_modified.update(r.modified_block_ids)

    desc = "; ".join(r.description for r in results)
    return OperationResult(True, f"Sanitize all: {desc}", list(total_modified), warnings)


def _remap_refs(blocks, schema, old_to_new):
    """Remap Ref/Ptr integer values in all blocks according to old_to_new map.

    Recurses into struct fields and struct arrays (e.g.
    ``NiDefaultAVObjectPalette.Objs[].AV Object``) so nested Ref/Ptr
    fields get remapped along with top-level ones.
    """
    from ..nif_file import remap_block_refs
    for block in blocks:
        remap_block_refs(block, old_to_new, schema, missing_default=-1)


def _get_ref_id(ref) -> int:
    if isinstance(ref, (int, float)):
        return int(ref)
    if isinstance(ref, dict):
        return int(ref.get("value", ref.get("Value", -1)))
    return -1
