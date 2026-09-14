"""Copy-as-override and copy-as-new-record, as in xEdit.

- "Copy as override": same FormID; the target gains the source plugin and the
  plugins owning the record's outbound FormIDs as masters.
- "Copy as new record": fresh FormID in the target's own slot; the target only
  needs masters for plugins the record still references.

`deep=True` also copies records reachable via outbound FormIDs (BFS, deduped).
Master-add follows xEdit's `AddRequiredMasters`: a target may not take a master
that loads after it, so such a record is skipped and logged. High-byte remap
covers the record's own form_id and subrecord bytes tagged `formid` /
`formid_array` (native `remap_formids_in_record`).
"""

from __future__ import annotations

import logging

from creation_lib.esp.editor.session import EditorSession
from creation_lib.esp.native_runtime import (
    plugin_handle_call,
    plugin_handle_copy_record,
    plugin_handle_get,
)

_log = logging.getLogger("creation_lib.esp.editor.override")


def copy_as_override(
    session: EditorSession,
    source_form_id: int,
    *,
    deep: bool = False,
    target_handle: int | None = None,
) -> list[int]:
    """Copy `source_form_id` into a target plugin as an override.

    Returns the list of FormIDs (target-space) that were inserted.
    """
    target = _resolve_target(session, target_handle)
    return _copy_records(session, source_form_id, target_handle=target, as_new=False, deep=deep)


def copy_as_new(
    session: EditorSession,
    source_form_id: int,
    *,
    deep: bool = False,
    target_handle: int | None = None,
) -> list[int]:
    """Copy `source_form_id` into a target plugin as a brand-new record.

    Allocates a fresh FormID in the target's own slot; the target does NOT
    pick up the source plugin as a master (the new record is owned by the
    target). Outbound references still need their owning plugins as masters.

    Returns the list of newly-allocated FormIDs (target-space).
    """
    target = _resolve_target(session, target_handle)
    return _copy_records(session, source_form_id, target_handle=target, as_new=True, deep=deep)


def _resolve_target(session: EditorSession, target_handle: int | None) -> int:
    if target_handle is not None:
        return int(target_handle)
    if session.active is None:
        raise RuntimeError("No active plugin to receive overrides")
    return session.active.handle


def _copy_records(
    session: EditorSession,
    source_form_id: int,
    *,
    target_handle: int,
    as_new: bool,
    deep: bool,
) -> list[int]:
    target_plugin = session.get_by_handle(target_handle)
    if target_plugin is None:
        raise RuntimeError(f"Unknown target handle: {target_handle}")

    queue: list[int] = [int(source_form_id)]
    seen: set[int] = set()
    inserted: list[int] = []

    while queue:
        fid = queue.pop(0)
        if fid in seen:
            continue
        seen.add(fid)

        located = session.resolve_form_id(fid)
        if located is None:
            _log.warning("copy: source FormID 0x%08X not found in load order", fid)
            continue
        source_handle, record_summary = located
        if source_handle == target_handle and not as_new:
            # Already in the target; nothing to override. Still recurse for deep.
            if deep:
                queue.extend(_outbound(target_handle, fid))
            continue

        source_plugin = session.get_by_handle(source_handle)
        if source_plugin is None:
            continue

        required_masters = _collect_required_masters(
            session, source_handle, record_summary.form_id, include_source=not as_new
        )
        try:
            _ensure_masters(target_plugin, required_masters, session)
        except _MasterOrderError as exc:
            _log.error("copy: %s", exc)
            continue

        try:
            recorded_fid = plugin_handle_copy_record(
                source_handle,
                record_summary.form_id,
                target_handle,
                as_new=as_new,
            )
        except Exception:
            _log.exception("copy: native copy failed for source 0x%08X", fid)
            continue
        if recorded_fid is None:
            continue
        inserted.append(recorded_fid)

        if deep:
            queue.extend(_outbound(source_handle, fid))

    session._invalidate_cache()
    return inserted


# -- master collection -------------------------------------------------------


def _collect_required_masters(
    session: EditorSession,
    source_handle: int,
    source_form_id: int,
    *,
    include_source: bool,
) -> list[str]:
    """Mirror xEdit's `ReportRequiredMasters`.

    Walks the record's own FormID + every outbound reference and returns the
    list of plugin filenames that must be present as masters in the target.
    With `include_source=True` (override), the source plugin itself is added.
    With `include_source=False` (copy-as-new), only referenced plugins are.
    """
    required: list[str] = []
    seen_lower: set[str] = set()

    def _add(name: str) -> None:
        key = name.lower()
        if key in seen_lower:
            return
        seen_lower.add(key)
        required.append(name)

    if include_source:
        source_plugin = session.get_by_handle(source_handle)
        if source_plugin is not None:
            _add(source_plugin.plugin_name)

    # Outbound refs: ask the native crate for everything this record points at
    # and resolve each FormID's owning plugin via the load order.
    try:
        outbound = plugin_handle_call(source_handle, "get_referenced_form_ids", source_form_id) or []
    except Exception:
        outbound = []
    for ref_fid in outbound:
        located = session.resolve_form_id(int(ref_fid))
        if located is None:
            continue
        owning_handle, _ = located
        owning_plugin = session.get_by_handle(owning_handle)
        if owning_plugin is None:
            continue
        _add(owning_plugin.plugin_name)

    return required


class _MasterOrderError(RuntimeError):
    pass


def _ensure_masters(target_plugin, required: list[str], session: EditorSession) -> None:
    """Add each required master to `target_plugin` if not already present.

    Refuses (per xEdit) to add a master that loads later than the target.
    """
    if not required:
        return
    target_handle = target_plugin.handle
    existing = [m.lower() for m in (plugin_handle_get(target_handle, "masters") or [])]
    target_idx = target_plugin.load_order_index
    target_name_lower = target_plugin.plugin_name.lower()

    for master_name in required:
        key = master_name.lower()
        if key == target_name_lower or key in existing:
            continue
        owning = session.get_by_name(master_name)
        if owning is not None and owning.load_order_index >= target_idx:
            raise _MasterOrderError(
                f"required master {master_name!r} loads after target "
                f"{target_plugin.plugin_name!r} — refusing to add"
            )
        try:
            plugin_handle_call(target_handle, "add_master", master_name)
        except Exception:
            _log.exception("Failed to add master %s to %s", master_name, target_plugin.plugin_name)
            continue
        existing.append(key)


def _outbound(handle: int, form_id: int) -> list[int]:
    try:
        ids = plugin_handle_call(handle, "get_referenced_form_ids", form_id) or []
    except Exception:
        return []
    return [int(fid) for fid in ids]
