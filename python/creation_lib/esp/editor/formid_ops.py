"""FormID operations — Change, Renumber, Compact-for-ESL, Inject into Master.

xEdit equivalents:
- `mniNavChangeFormID` — reassign one record's FormID
- `mniNavRenumberFormIDsFrom` — bulk reassign starting at a base object_id
- `mniNavCompactFormIDs` — pack object_ids into [0x800, 0x1000) for ESL
- `mniNavRenumberFormIDsInject` — move records into a master plugin's namespace

All four ops are special cases of the same primitive:

    apply_object_id_mapping(session, owning_handle, object_id_map)

which walks every loaded plugin, rewrites the record's own form_id and every
formid/formid_array subrecord whose object_id is in the map AND whose high
byte references `owning_handle` in that plugin's master space.

Inject is slightly different: it also changes the *owning plugin* of each
record from the source to the target master, so the high byte is rewritten
per-plugin.

Invariants (matching xEdit):
- Cannot change a FormID to 0 or 0x14 (player)
- ESL compact requires the plugin own ≤ 4096 records
- Renumber's range must not collide with existing object_ids in `owning_handle`
"""

from __future__ import annotations

import logging

from creation_lib.esp.editor.session import EditorSession
from creation_lib.esp.native_runtime import (
    plugin_handle_call,
    plugin_handle_apply_object_id_mapping,
    plugin_handle_get,
    plugin_handle_owned_object_ids,
)

_log = logging.getLogger("creation_lib.esp.editor.formid_ops")


def apply_object_id_mapping(
    session: EditorSession,
    owning_handle: int,
    object_id_map: dict[int, int],
    *,
    new_owning_handle: int | None = None,
) -> int:
    """Apply `object_id_map` to every record/ref pointing at `owning_handle`.

    Pre-conditions:
    - `object_id_map` keys are 24-bit values (0..0xFFFFFF)
    - If `new_owning_handle` is not None and != owning_handle, this is the
      "Inject into master" case: every referenced FormID's high byte gets
      rewritten to point at `new_owning_handle` and target plugins gain
      `new_owning_handle` as a master.

    Returns the number of records that were rewritten (across all plugins).
    """
    if not object_id_map:
        return 0
    owning_plugin = session.get_by_handle(owning_handle)
    if owning_plugin is None:
        raise RuntimeError(f"unknown owning handle {owning_handle}")
    target_plugin = (
        session.get_by_handle(new_owning_handle) if new_owning_handle is not None else owning_plugin
    )
    if target_plugin is None:
        raise RuntimeError(f"unknown target handle {new_owning_handle}")

    rewritten = 0
    for plugin in session.plugins:
        old_high = _master_index(plugin, owning_plugin)
        if old_high is None:
            continue
        # Ensure the target master is present in this plugin if injecting.
        if target_plugin is not owning_plugin:
            _ensure_master_present(plugin, target_plugin)
            new_high = _master_index(plugin, target_plugin)
            if new_high is None:
                _log.warning(
                    "skipping %s — could not add target master %s",
                    plugin.plugin_name, target_plugin.plugin_name,
                )
                continue
        else:
            new_high = old_high
        rewritten += _apply_to_plugin(
            plugin.handle, old_high, new_high, object_id_map,
        )
    session._invalidate_cache()
    return rewritten


def change_form_id(
    session: EditorSession,
    old_form_id: int,
    new_form_id: int,
) -> int:
    """Reassign `old_form_id` to `new_form_id`. Both must share an owner."""
    if new_form_id in (0, 0x14):
        raise ValueError(f"refusing to assign reserved FormID 0x{new_form_id:08X}")
    located = session.resolve_form_id(old_form_id)
    if located is None:
        raise ValueError(f"FormID 0x{old_form_id:08X} not found")
    owning_handle, _ = located
    old_obj = old_form_id & 0x00FF_FFFF
    new_obj = new_form_id & 0x00FF_FFFF
    return apply_object_id_mapping(session, owning_handle, {old_obj: new_obj})


def renumber_form_ids_from(
    session: EditorSession,
    handle: int,
    base_object_id: int,
) -> int:
    """Renumber every record owned by `handle` to sequential object_ids
    starting at `base_object_id`. Refuses if the destination range collides
    with existing record object_ids that aren't being renumbered."""
    if base_object_id < 0x800 or base_object_id > 0x00FF_FFFF:
        raise ValueError(f"base_object_id 0x{base_object_id:06X} out of range")
    owned = _owned_object_ids(handle)
    if not owned:
        return 0
    owned_sorted = sorted(owned)
    new_ids = list(range(base_object_id, base_object_id + len(owned_sorted)))
    if max(new_ids) > 0x00FF_FFFF:
        raise ValueError(f"renumber overflows: would need up to 0x{max(new_ids):06X}")
    # Detect collisions: a destination id that's owned but not in our source set.
    src_set = set(owned_sorted)
    for new_id in new_ids:
        if new_id in src_set:
            continue  # being remapped anyway
        if new_id in owned:
            raise ValueError(f"renumber would collide with existing 0x{new_id:06X}")
    mapping = dict(zip(owned_sorted, new_ids))
    # Drop identity entries to skip needless rewrites.
    mapping = {k: v for k, v in mapping.items() if k != v}
    if not mapping:
        return 0
    rewritten = apply_object_id_mapping(session, handle, mapping)
    # Update header.next_object_id
    new_next = max(new_ids) + 1
    plugin_handle_call(handle, "set_header_next_object_id", new_next)
    return rewritten


def compact_for_esl(session: EditorSession, handle: int) -> int:
    """Pack owned object_ids into [0x800, 0x1000). Refuses if > 2048 records."""
    owned = _owned_object_ids(handle)
    if len(owned) > 0x800:
        raise ValueError(
            f"too many records for ESL: {len(owned)} > 2048 "
            "(ESL object_id range is 0x800..0xFFF)"
        )
    return renumber_form_ids_from(session, handle, 0x800)


def inject_into_master(
    session: EditorSession,
    source_handle: int,
    target_handle: int,
) -> int:
    """Move every record owned by `source_handle` into `target_handle`'s
    namespace, allocating fresh object_ids in the target."""
    if source_handle == target_handle:
        return 0
    target_plugin = session.get_by_handle(target_handle)
    source_plugin = session.get_by_handle(source_handle)
    if target_plugin is None or source_plugin is None:
        raise RuntimeError("unknown source/target handle")

    src_objs = sorted(_owned_object_ids(source_handle))
    if not src_objs:
        return 0

    # Allocate fresh object_ids in target, one per source record.
    target_taken = _owned_object_ids(target_handle)
    next_id = int(plugin_handle_get(target_handle, "next_object_id") or 0x800)
    new_objs: list[int] = []
    for _ in src_objs:
        while next_id in target_taken or next_id < 0x800:
            next_id += 1
        new_objs.append(next_id)
        target_taken.add(next_id)
        next_id += 1
    mapping = dict(zip(src_objs, new_objs))

    rewritten = apply_object_id_mapping(
        session, source_handle, mapping, new_owning_handle=target_handle,
    )
    plugin_handle_call(target_handle, "set_header_next_object_id", next_id)
    return rewritten


# -- helpers -----------------------------------------------------------------


def _master_index(plugin, owning_plugin) -> int | None:
    """Return the high byte for `owning_plugin` as seen from `plugin`."""
    if plugin.handle == owning_plugin.handle:
        masters = plugin_handle_get(plugin.handle, "masters") or []
        return len(masters) & 0xFF
    masters = plugin_handle_get(plugin.handle, "masters") or []
    target_lower = owning_plugin.plugin_name.lower()
    for i, name in enumerate(masters):
        if name.lower() == target_lower:
            return i
    return None


def _ensure_master_present(plugin, master_plugin) -> None:
    if plugin.handle == master_plugin.handle:
        return
    masters_lower = {m.lower() for m in (plugin_handle_get(plugin.handle, "masters") or [])}
    if master_plugin.plugin_name.lower() in masters_lower:
        return
    try:
        plugin_handle_call(plugin.handle, "add_master", master_plugin.plugin_name)
    except Exception:
        _log.exception("ensure_master_present: add_master %s failed", master_plugin.plugin_name)


def _owned_object_ids(handle: int) -> set[int]:
    """All object_ids (24-bit) owned by this plugin (high byte == own_index)."""
    try:
        return set(plugin_handle_owned_object_ids(handle))
    except Exception:
        return set()


def _apply_to_plugin(
    handle: int,
    old_high: int,
    new_high: int,
    object_id_map: dict[int, int],
) -> int:
    """Rewrite every record + ref in `handle` matching the mapping.

    Returns the number of records that were rewritten (not the number of
    individual FormIDs touched).
    """
    return plugin_handle_apply_object_id_mapping(
        handle,
        old_high,
        new_high,
        object_id_map,
    )
