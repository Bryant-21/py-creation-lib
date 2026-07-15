"""Master-list operations — Add Masters, Sort Masters, Clean Masters.

xEdit equivalents: `mniNavAddMasters`, `mniNavSortMasters`, `mniNavCleanMasters`.

The native helper `plugin_handle_set_masters` swaps the masters list and
remaps every record's FormID high byte atomically (via the crate-private
`remap_formids_in_items`). Python just decides what the new list looks like.

Invariants (matching xEdit):
- Game master is never removed by Clean Masters
- Sort Masters reorders by current load order; `LoadedPlugin.load_order_index`
  is the source of truth
- A master that loads after the target plugin cannot be added (would be
  invalid). Add Masters refuses such entries.
"""

from __future__ import annotations

import logging
from typing import Iterable

from creation_lib.esp.editor.session import EditorSession, LoadedPlugin
from creation_lib.esp.native_runtime import (
    plugin_handle_call,
    plugin_handle_get,
)

_log = logging.getLogger("creation_lib.esp.editor.masters_ops")


# Per-game "primary" master that Clean Masters never removes.
_GAME_MASTERS = {
    "fo4": "Fallout4.esm",
    "fo76": "SeventySix.esm",
    "skyrimse": "Skyrim.esm",
    "starfield": "Starfield.esm",
    "fo3": "Fallout3.esm",
    "fnv": "FalloutNV.esm",
    "oblivion": "Oblivion.esm",
}


def add_masters(
    handle: int,
    names: Iterable[str],
    *,
    session: EditorSession,
) -> list[str]:
    """Add `names` as masters to `handle`. Returns the names actually added.

    Skips any name that already exists, equals the target plugin, or loads
    after the target.
    """
    target = session.get_by_handle(handle)
    if target is None:
        raise RuntimeError(f"unknown handle {handle}")
    existing = {m.lower() for m in (plugin_handle_get(handle, "masters") or [])}
    target_idx = target.load_order_index
    target_name_lower = target.plugin_name.lower()
    added: list[str] = []
    for name in names:
        key = name.lower()
        if key in existing or key == target_name_lower:
            continue
        owning = session.get_by_name(name)
        if owning is not None and owning.load_order_index >= target_idx:
            _log.warning(
                "add_masters: skipping %s (loads after target %s)",
                name, target.plugin_name,
            )
            continue
        try:
            plugin_handle_call(handle, "add_master", name)
        except Exception:
            _log.exception("add_masters: native add_master failed for %s", name)
            continue
        existing.add(key)
        added.append(name)
    if added:
        session._invalidate_cache()
    return added


def sort_masters(handle: int, *, session: EditorSession) -> None:
    """Reorder the target's masters to match current load order."""
    target = session.get_by_handle(handle)
    if target is None:
        raise RuntimeError(f"unknown handle {handle}")
    current = list(plugin_handle_get(handle, "masters") or [])
    sizes = list(plugin_handle_get(handle, "master_sizes") or [])
    sizes = (sizes + [0] * len(current))[: len(current)]

    # Pair each master with its load_order_index; unloaded plugins go to the end
    # in their original relative order.
    def _key(item: tuple[int, str, int]) -> tuple[int, int]:
        idx_in_list, name, _size = item
        owning = session.get_by_name(name)
        order = owning.load_order_index if owning is not None else 10**9 + idx_in_list
        return (order, idx_in_list)

    indexed = list(zip(range(len(current)), current, sizes))
    indexed.sort(key=_key)
    new_pairs = [(name, size) for _, name, size in indexed]
    if [n for n, _ in new_pairs] == current:
        return
    plugin_handle_call(handle, "set_masters", new_pairs)
    session._invalidate_cache()


def clean_masters(handle: int, *, session: EditorSession) -> list[str]:
    """Drop masters with no referencing FormIDs. Returns dropped names."""
    target = session.get_by_handle(handle)
    if target is None:
        raise RuntimeError(f"unknown handle {handle}")
    masters = list(plugin_handle_get(handle, "masters") or [])
    sizes = list(plugin_handle_get(handle, "master_sizes") or [])
    sizes = (sizes + [0] * len(masters))[: len(masters)]
    if not masters:
        return []

    used = _used_master_indices(handle)
    game_master = _GAME_MASTERS.get(target.game, "").lower()

    keep: list[tuple[str, int]] = []
    dropped: list[str] = []
    for idx, (name, size) in enumerate(zip(masters, sizes)):
        if idx in used or name.lower() == game_master:
            keep.append((name, size))
        else:
            dropped.append(name)

    if not dropped:
        return []
    plugin_handle_call(handle, "set_masters", keep)
    session._invalidate_cache()
    return dropped


# -- helpers -----------------------------------------------------------------


def _used_master_indices(handle: int) -> set[int]:
    """Walk every record in `handle` and return the set of high-byte indices
    referenced by either the record's own FormID (when overriding a master)
    or any `formid` / `formid_array` subrecord.
    """
    try:
        from creation_lib.esp.native_runtime import plugin_handle_used_master_indices

        return set(plugin_handle_used_master_indices(handle))
    except Exception:
        return set()
