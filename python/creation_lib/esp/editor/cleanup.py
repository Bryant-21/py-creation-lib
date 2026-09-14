"""Cleanup operations: Remove ITM, and Undelete and Disable References.

xEdit equivalents:
- `mniNavRemoveIdenticalToMaster`: drop overrides byte-identical to their master
  (the Creation Kit's "save also saves unmodified records" bug creates these).
- `mniNavUndeleteAndDisableReferences`: restore deleted refs and flag them
  initially disabled with a player-parent XESP.

Both act on the native validate report; `validate.py` detects, this module fixes.
As in xEdit, injected master records (source-owned but with a master's high byte)
are never removed (the native ITM check excludes them), NAVM is never undeleted
(no XESP slot), and injected refs are never undeleted.
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

    Returns the undeleted FormIDs. Each record gets RECORD_FLAG_DELETED cleared,
    RECORD_FLAG_INITIALLY_DISABLED set, XTEL dropped (DATA position kept), and an
    XESP with parent = player (0x14) and flag 0x01 (opposite of parent, so disabled).
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
