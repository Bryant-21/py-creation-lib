"""NIF import/merge engine for kitbashing workflows."""
from __future__ import annotations

import logging
from dataclasses import dataclass, field

from creation_lib.nif.nif_file import NifFile

_log = logging.getLogger(__name__)

# Block type names for each category
_ANIMATION_TYPES = frozenset({
    "NiControllerManager",
    "NiControllerSequence",
    "NiMultiTargetTransformController",
})
_CONNECT_POINT_TYPES = frozenset({
    "BSConnectPoint::Parents",
    "BSConnectPoint::Children",
})
_ROOT_EXTRA_DATA_TYPES = frozenset({
    "BSXFlags",
    "BSBehaviorGraphExtraData",
    "NiDefaultAVObjectPalette",
})


@dataclass
class ImportOptions:
    """Which categories of blocks to import."""
    import_geometry: bool = True
    import_animations: bool = True
    import_connect_points: bool = True
    import_root_extra_data: bool = True

    def to_dict(self) -> dict:
        return {
            "import_geometry": self.import_geometry,
            "import_animations": self.import_animations,
            "import_connect_points": self.import_connect_points,
            "import_root_extra_data": self.import_root_extra_data,
        }

    @classmethod
    def from_dict(cls, d: dict) -> ImportOptions:
        return cls(
            import_geometry=d.get("import_geometry", True),
            import_animations=d.get("import_animations", True),
            import_connect_points=d.get("import_connect_points", True),
            import_root_extra_data=d.get("import_root_extra_data", True),
        )


@dataclass
class ImportResult:
    """Result of an import operation."""
    imported_count: int = 0
    skipped: list[str] = field(default_factory=list)
    error: str = ""


DEFAULT_IMPORT_OPTIONS = ImportOptions()


def categorize_children(nif: NifFile, root_block_id: int) -> dict[str, list[int]]:
    """Categorize root's direct children into geometry/animations/connect_points/root_extra_data.

    Only direct children of the root are categorized. Returns a dict mapping
    category name to list of block IDs.
    """
    categories: dict[str, list[int]] = {
        "geometry": [],
        "animations": [],
        "connect_points": [],
        "root_extra_data": [],
    }

    root = nif.get_block(root_block_id)
    if root is None:
        return categories

    children_ids = root.get_field("Children") or []
    for child_ref in children_ids:
        if isinstance(child_ref, dict):
            bid = child_ref.get("block_id", -1)
        else:
            bid = child_ref
        if bid < 0 or bid >= len(nif.blocks):
            continue

        block = nif.get_block(bid)
        if block is None:
            continue

        type_name = block.type_name
        if type_name in _ANIMATION_TYPES:
            categories["animations"].append(bid)
        elif type_name in _CONNECT_POINT_TYPES:
            categories["connect_points"].append(bid)
        elif type_name in _ROOT_EXTRA_DATA_TYPES:
            categories["root_extra_data"].append(bid)
        else:
            categories["geometry"].append(bid)

    return categories


def _get_existing_extra_data_types(nif: NifFile, root_block_id: int) -> set[str]:
    """Get set of root extra data type names already present on the target root."""
    root = nif.get_block(root_block_id)
    if root is None:
        return set()

    existing = set()
    extra_ids = root.get_field("Extra Data List") or []
    for ref in extra_ids:
        bid = ref if isinstance(ref, int) else ref.get("block_id", -1) if isinstance(ref, dict) else -1
        if 0 <= bid < len(nif.blocks):
            block = nif.get_block(bid)
            if block and block.type_name in _ROOT_EXTRA_DATA_TYPES:
                existing.add(block.type_name)

    # Also check direct children
    children_ids = root.get_field("Children") or []
    for child_ref in children_ids:
        bid = child_ref if isinstance(child_ref, int) else child_ref.get("block_id", -1) if isinstance(child_ref, dict) else -1
        if 0 <= bid < len(nif.blocks):
            block = nif.get_block(bid)
            if block and block.type_name in _ROOT_EXTRA_DATA_TYPES:
                existing.add(block.type_name)

    return existing


def import_nif(app, source: NifFile, options: ImportOptions) -> ImportResult:
    """Import the block categories selected in ``options`` from ``source`` into ``app.nif``."""
    from creation_lib.nif.actions import SnapshotAction
    from creation_lib.nif.operations.sanitize import sanitize_links

    target = app.nif
    if target is None:
        return ImportResult(error="No NIF file loaded")

    if not source.blocks:
        return ImportResult(error="Source NIF has no blocks")

    # Verify source root is an NiNode subtype
    source_root = source.get_block(0)
    if source_root is None or not source.schema.is_subtype_of(source_root.type_name, "NiNode"):
        return ImportResult(error="Source NIF root (block 0) is not an NiNode subtype")

    # Categorize source root's children
    categories = categorize_children(source, 0)

    # Filter block IDs based on options
    selected_ids: list[int] = []
    skipped: list[str] = []

    if options.import_geometry:
        selected_ids.extend(categories["geometry"])
    if options.import_animations:
        selected_ids.extend(categories["animations"])
    if options.import_connect_points:
        selected_ids.extend(categories["connect_points"])

    if options.import_root_extra_data:
        existing_types = _get_existing_extra_data_types(target, 0)
        for bid in categories["root_extra_data"]:
            block = source.get_block(bid)
            if block and block.type_name in existing_types:
                skipped.append(f"Skipped {block.type_name} (already exists on target root)")
                _log.info("Skipped %s block %d — target root already has one", block.type_name, bid)
            else:
                selected_ids.append(bid)

    if not selected_ids:
        return ImportResult(imported_count=0, skipped=skipped)

    # Capture before state for undo
    cmd = SnapshotAction(_description="Import NIF")
    cmd.capture_before(target)

    # Copy blocks with dependency resolution
    from creation_lib.nif.operations.copy import copy_blocks

    blocks_before = len(target.blocks)
    copy_blocks(source, selected_ids, target, attach_to=0)
    blocks_after = len(target.blocks)
    imported_count = blocks_after - blocks_before

    # Secondary safety net
    sanitize_links(target)

    # Capture after state and push undo
    cmd.capture_after(target)
    app.undo_manager.push(app.registry.active_id, cmd)
    app._nif_dirty = True

    for msg in skipped:
        _log.info(msg)

    return ImportResult(imported_count=imported_count, skipped=skipped)
