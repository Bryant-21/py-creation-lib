"""Build Reachable Info — find orphan records via reachability BFS.

xEdit equivalent: `mniNavBuildReachable`. Starting from a curated set of
"entry-point" record types (records the engine loads regardless of refs)
plus all hardcoded FormIDs, BFS via outbound references and tag everything
visited as reachable. The complement is the "orphan" set — records present
in the plugin but not actually loaded by the game.

xEdit's invariant: BuildReachable requires BuildRef first. The native back-
reference index also serves the forward-ref query, so we don't need an
explicit pre-build call here.
"""

from __future__ import annotations

import logging

from creation_lib.esp.editor.session import EditorSession
from creation_lib.esp.native_runtime import (
    plugin_handle_call,
    plugin_handle_record_form_ids,
)

_log = logging.getLogger("creation_lib.esp.editor.reachable")

# Per xEdit (`mniNavBuildReachable` and `IsReached` whitelist).
# Game-agnostic entry points — these record types are loaded by the engine
# regardless of whether anything references them.
ENTRY_POINT_SIGS = {
    "ADDN", "ANIO", "AVIF", "BSGN", "CAMS", "COBJ", "CPTH",
    "DFOB", "DLVW", "DOBJ", "GMST", "IDLE", "LSCR", "NAVI",
    "RADS", "SKIL", "CLAS",
}

# FO4-specific entry points: workshops, quests, perks, magic effects.
ENTRY_POINT_SIGS_FO4 = ENTRY_POINT_SIGS | {"QUST", "PERK", "MGEF", "WRLD"}
ENTRY_POINT_SIGS_SKYRIMSE = ENTRY_POINT_SIGS | {"QUST", "PERK", "MGEF", "WRLD"}
ENTRY_POINT_SIGS_STARFIELD = ENTRY_POINT_SIGS | {"QUST", "PERK", "MGEF", "WRLD"}


def entry_points_for_game(game: str) -> set[str]:
    return {
        "fo4": ENTRY_POINT_SIGS_FO4,
        "fo76": ENTRY_POINT_SIGS_FO4,
        "skyrimse": ENTRY_POINT_SIGS_SKYRIMSE,
        "starfield": ENTRY_POINT_SIGS_STARFIELD,
        "fo3": ENTRY_POINT_SIGS,
        "fnv": ENTRY_POINT_SIGS,
        "oblivion": ENTRY_POINT_SIGS,
    }.get(game, ENTRY_POINT_SIGS)


def build_reachable_set(
    session: EditorSession,
    *,
    handle: int | None = None,
) -> set[int]:
    """BFS from entry points. Returns the set of FormIDs (in their owning
    plugin's local space) that are reachable.

    If `handle` is given, the result is restricted to records that belong
    to that plugin's load-order chain.
    """
    target_handles = (
        [handle] if handle is not None else [p.handle for p in session.plugins]
    )

    # Phase 1: seed from entry-point signatures.
    seeds: list[tuple[int, int]] = []  # (handle, form_id)
    for h in target_handles:
        plugin = session.get_by_handle(h)
        if plugin is None:
            continue
        sigs = entry_points_for_game(plugin.game)
        try:
            for form_id in plugin_handle_record_form_ids(h, sorted(sigs)):
                seeds.append((h, int(form_id)))
        except Exception:
            continue

    # Hardcoded form IDs that the game always loads (player ref, world coc, etc.)
    HARDCODED = {0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D}

    visited: set[int] = set()
    queue = [(h, fid) for h, fid in seeds]
    for fid in HARDCODED:
        located = session.resolve_form_id(fid)
        if located is not None:
            queue.append((located[0], fid))

    # Phase 2: BFS over outbound refs.
    while queue:
        handle_id, fid = queue.pop(0)
        if fid in visited:
            continue
        visited.add(fid)
        try:
            outbound = plugin_handle_call(handle_id, "get_referenced_form_ids", fid) or []
        except Exception:
            continue
        for ref_fid in outbound:
            ref_int = int(ref_fid)
            if ref_int in visited:
                continue
            located = session.resolve_form_id(ref_int)
            if located is None:
                continue
            queue.append((located[0], ref_int))

    return visited


def find_orphan_records(
    session: EditorSession,
    *,
    handle: int | None = None,
) -> list[int]:
    """Return FormIDs of records in `handle` (or active) not in the reachable set."""
    target_handle = handle if handle is not None else (
        session.active.handle if session.active else None
    )
    if target_handle is None:
        return []
    plugin = session.get_by_handle(target_handle)
    if plugin is None:
        return []
    reachable = build_reachable_set(session)

    orphans: list[int] = []
    try:
        form_ids = plugin_handle_record_form_ids(target_handle)
    except Exception:
        return orphans
    for fid in form_ids:
        form_id = int(fid)
        if form_id not in reachable:
            orphans.append(form_id)
    return orphans
