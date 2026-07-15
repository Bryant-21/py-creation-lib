"""Compatibility-patch authoring on top of `EditorSession`.

A "patch plugin" is a freshly-created `.esp` that lists conflicting source
plugins as masters and contains overrides combining each source's edits.

This module supplies the imperative primitives the UI calls:

- `create_patch_plugin` — build an empty patch handle and register it as the
  session's patch target.
- `add_winner_to_patch` — copy the load-order winner of a conflict report into
  the patch as a single override (no merging).
- `automerge_to_patch` — dispatch mergeable reports to the Rust-native merge
  operation; falls back to copy-winner for non-mergeable signatures.

Cross-plugin FormID master-index remapping is handled inside native copy and
merge operations.
"""

from __future__ import annotations

import logging
from typing import Iterable

from creation_lib.esp.editor.conflicts import ConflictReport
from creation_lib.esp.editor.override import copy_as_override
from creation_lib.esp.editor.session import EditorSession
from creation_lib.esp.native_runtime import (
    plugin_handle_call,
    plugin_handle_get,
    plugin_handle_merge_conflict_to_patch,
    plugin_handle_new,
)

_log = logging.getLogger("creation_lib.esp.editor.patch")


class PatchError(RuntimeError):
    """Raised when the patch plugin cannot be created or modified."""


def create_patch_plugin(
    session: EditorSession,
    plugin_name: str,
    *,
    game: str | None = None,
    set_as_target: bool = True,
) -> int:
    """Create a new empty patch plugin and return its native handle.

    `plugin_name` should include the `.esp` extension (matches xEdit's
    convention). The patch is registered with the session both as a regular
    `LoadedPlugin` (so cross-plugin resolution works) and as the session's
    `_patch_handle` (for UI tagging).
    """
    if not plugin_name:
        raise PatchError("plugin_name is required")
    resolved_game = game
    if resolved_game is None:
        if session.active is not None:
            resolved_game = session.active.game
        elif session.plugins:
            resolved_game = session.plugins[0].game
        else:
            resolved_game = "fo4"

    handle = plugin_handle_new(plugin_name, resolved_game)
    handle_id = int(handle)

    # Register as a LoadedPlugin so resolve_form_id and the UI tree see it.
    from creation_lib.esp.editor.session import LoadedPlugin

    plugin = LoadedPlugin(
        handle=handle_id,
        path="",
        game=resolved_game,
        is_master=False,
        load_order_index=len(session._plugins),
        plugin_name=plugin_name,
    )
    session._plugins.append(plugin)
    if set_as_target:
        session._patch_handle = handle_id
        session._active_handle = handle_id
    session._invalidate_cache()
    return handle_id


def clear_patch_target(session: EditorSession) -> None:
    """Forget the current patch target (does not close the plugin)."""
    session._patch_handle = None


def ensure_patch_masters(
    session: EditorSession,
    patch_handle: int,
    plugin_names: Iterable[str],
) -> None:
    """Add each name to the patch's master table if not already present."""
    existing = {
        m.lower() for m in (plugin_handle_get(patch_handle, "masters") or [])
    }
    for name in plugin_names:
        if not name or name.lower() in existing:
            continue
        try:
            plugin_handle_call(patch_handle, "add_master", name)
            existing.add(name.lower())
        except Exception:
            _log.exception("Failed to add master %s to patch handle %s", name, patch_handle)


def add_winner_to_patch(
    session: EditorSession,
    patch_handle: int,
    form_id: int,
) -> bool:
    """Copy the load-order winner of `form_id` into the patch as an override.

    Returns True if a record was inserted, False otherwise. Adds the winner's
    plugin to the patch's master table on demand.
    """
    located = session.resolve_form_id(form_id)
    if located is None:
        _log.warning("add_winner_to_patch: form_id 0x%08X not found", form_id)
        return False
    source_handle, _ = located
    if source_handle == patch_handle:
        return False
    inserted = copy_as_override(session, form_id, target_handle=patch_handle)
    return bool(inserted)


def add_winners_to_patch(
    session: EditorSession,
    patch_handle: int,
    reports: Iterable[ConflictReport],
) -> int:
    """Copy the winner of each conflict report into the patch.

    Adds all chain plugins as patch masters so the patch is logically
    dependent on every source being loaded — matches xEdit's default
    "compatibility patch" behavior.
    """
    reports = list(reports)
    masters_to_add: set[str] = set()
    for rpt in reports:
        for entry in rpt.chain:
            masters_to_add.add(entry.plugin_name)
    ensure_patch_masters(session, patch_handle, masters_to_add)

    count = 0
    for rpt in reports:
        try:
            if add_winner_to_patch(session, patch_handle, rpt.form_id):
                count += 1
        except Exception:
            _log.exception("Failed to add winner 0x%08X", rpt.form_id)
    return count


def automerge_to_patch(
    session: EditorSession,
    patch_handle: int,
    report: ConflictReport,
) -> bool:
    """Merge a report into the patch when Rust supports it, else copy winner."""
    ensure_patch_masters(
        session,
        patch_handle,
        [e.plugin_name for e in report.chain],
    )
    if report.mergeable:
        chain = [
            (
                int(entry.plugin_handle),
                str(entry.plugin_name),
                int(entry.load_order_index),
                int(entry.form_id),
            )
            for entry in report.chain
        ]
        if plugin_handle_merge_conflict_to_patch(patch_handle, report.signature, chain):
            session._invalidate_cache()
            return True
    return add_winner_to_patch(session, patch_handle, report.form_id)
