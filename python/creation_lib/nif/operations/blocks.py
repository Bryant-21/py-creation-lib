"""Block type conversion operations."""
from ..actions import OperationResult


def convert_block_type(nif, block_id: int, new_type: str) -> OperationResult:
    """Convert a block to a compatible type (must share a common ancestor).

    Preserves all fields that exist in both the old and new types.
    Fields unique to the old type are dropped; fields unique to the new type
    get default values.
    """
    block = nif.get_block(block_id)
    if not block:
        return OperationResult(False, f"Block {block_id} not found")

    old_type = block.type_name
    if old_type == new_type:
        return OperationResult(True, f"Block {block_id} is already {new_type}", [block_id])

    schema = nif.schema

    # Verify new_type exists in schema
    if new_type not in schema.niobjects:
        return OperationResult(False, f"Unknown block type: {new_type}")

    # Check abstract
    obj_def = schema.niobjects[new_type]
    if obj_def.abstract:
        return OperationResult(False, f"Cannot convert to abstract type: {new_type}")

    # Check compatibility: must share a common ancestor
    old_hierarchy = set(schema.get_type_hierarchy(old_type))
    new_hierarchy = set(schema.get_type_hierarchy(new_type))
    common = old_hierarchy & new_hierarchy
    if not common:
        return OperationResult(
            False,
            f"{old_type} and {new_type} share no common ancestor",
        )

    # Get field definitions for new type
    new_fields = schema.get_all_fields(new_type)
    new_field_names = {f.name for f in new_fields}

    # Preserve fields that exist in both types
    preserved = []
    for name, value in block.fields:
        bare = name.split(":")[0] if ":" in name else name
        if bare in new_field_names:
            preserved.append((name, value))

    # Update block
    block.type_name = new_type
    block.fields = preserved
    block._field_map = {name: val for name, val in preserved}

    # Update header block type info
    if block_id < len(nif.header.block_type_index):
        # Add new type name if not present
        if new_type not in nif.header.block_type_names:
            nif.header.block_type_names.append(new_type)
        type_idx = nif.header.block_type_names.index(new_type)
        nif.header.block_type_index[block_id] = type_idx

    return OperationResult(
        True,
        f"Converted block {block_id} from {old_type} to {new_type}",
        [block_id],
    )
