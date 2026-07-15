"""Cleanup operations — Remove ITM, Undelete and Disable References.

xEdit equivalents:
- `mniNavRemoveIdenticalToMaster` — drop overrides that are byte-identical
  to their master (commonly created by the Creation Kit's "save also saves
  unmodified records" bug).
- `mniNavUndeleteAndDisableReferences` — restore deleted reference records
  and flag them as initially disabled with a player-parent XESP.

Both use the existing native validate report (which already detects ITM and
deleted records) as their input. The `validate.py` module is the source of
truth for "what's wrong"; this module fixes it.

Invariants (matching xEdit):
- Cannot remove injected master records (records owned by the source plugin
  but with a master's high byte). The native ITM check already excludes them.
- Cannot undelete NAVM (no XESP slot, navmesh-specific cleanup needed).
- Cannot undelete injected refs (record's plugin owner != current plugin).
"""

from __future__ import annotations

import logging

from creation_lib.esp.editor.session import EditorSession
from creation_lib.esp.editor.validate import IssueCategory, validate
from creation_lib.esp.native_runtime import (
    plugin_handle_call,
    plugin_handle_undelete_and_disable_refs,
)

_log = logging.getLogger("creation_lib.esp.editor.cleanup")

# Reference signatures eligible for Undelete & Disable. NAVM excluded — xEdit
# refuses to undelete navmeshes (they need different cleanup).
UNDELETE_SIGNATURES = {
    "REFR", "ACHR", "ACRE", "PGRE", "PMIS", "PARW",
    "PBAR", "PBEA", "PCON", "PFLA", "PHZD",
}


def remove_itm_records(
    session: EditorSession,
    *,
    handles: list[int] | None = None,
) -> list[int]:
    """Remove every ITM-tagged record from the given plugins (default: active).

    Returns the FormIDs that were removed. Re-runs validate to find ITMs;
    the caller does not need to pre-compute the report.
    """
    targets = handles if handles is not None else (
        [session.active.handle] if session.active else []
    )
    if not targets:
        return []

    saved_active = session._active_handle
    removed: list[int] = []
    try:
        for handle in targets:
            session._active_handle = handle
            report = validate(session)
            for issue in report.by_category(IssueCategory.ITM):
                if issue.form_id is None or issue.plugin_handle != handle:
                    continue
                try:
                    if plugin_handle_call(handle, "remove_record", int(issue.form_id)):
                        removed.append(int(issue.form_id))
                except Exception:
                    _log.exception("remove_record failed for 0x%08X", issue.form_id)
    finally:
        session._active_handle = saved_active

    if removed:
        session._invalidate_cache()
    return removed


def undelete_and_disable_refs(
    session: EditorSession,
    *,
    handles: list[int] | None = None,
) -> list[int]:
    """Undelete deleted REFR/ACHR/etc. records and set them initially disabled.

    Returns the FormIDs that were undeleted.

    For each candidate record:
      1. Clear the RECORD_FLAG_DELETED bit
      2. Set the RECORD_FLAG_INITIALLY_DISABLED bit
      3. Drop XTEL (teleport target), DATA (position) is preserved
      4. Add or replace XESP — parent ref = player (0x14), flag = 0x01
         (set enable state to opposite of parent → effectively disabled)
    """
    targets = handles if handles is not None else (
        [session.active.handle] if session.active else []
    )
    if not targets:
        return []

    fixed: list[int] = []
    for handle in targets:
        try:
            fixed.extend(
                plugin_handle_undelete_and_disable_refs(
                    handle,
                    sorted(UNDELETE_SIGNATURES),
                )
            )
        except Exception:
            _log.exception("undelete_and_disable_refs failed for handle %s", handle)

    if fixed:
        session._invalidate_cache()
    return fixed
