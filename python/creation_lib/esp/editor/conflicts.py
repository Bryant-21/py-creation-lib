"""Cross-plugin conflict detection for compatibility-patch authoring.

This module is a thin Python adapter over the Rust scanner in
`creation_lib._native.esp_authoring_core.scan_conflicts_native`. The Rust side
walks every loaded handle's parsed record graph, hashes records, buckets
by FormID, classifies status, and emits one report per FormID overridden
by ≥2 plugins — all without marshalling records into Python.

The Python layer keeps lightweight `OverrideEntry` / `ConflictReport` /
`ConflictScan` dataclasses that the UI consumes. Conflict entries carry only
metadata; patch copy operations call Rust directly when they need to clone a
winner into a target plugin.
"""

from __future__ import annotations

import logging
import threading
from dataclasses import dataclass, field
from typing import Iterable

from creation_lib.esp.editor.session import ConflictStatus, EditorSession
from creation_lib.esp.native_runtime import scan_conflicts

_log = logging.getLogger("creation_lib.esp.editor.conflicts")


_STATUS_FROM_NATIVE: dict[str, ConflictStatus] = {
    "override": ConflictStatus.OVERRIDE,
    "conflict": ConflictStatus.CONFLICT,
}


class OverrideEntry:
    """One plugin's metadata view of a record at a given FormID."""

    __slots__ = (
        "plugin_handle",
        "plugin_name",
        "load_order_index",
        "form_id",
        "payload_hash",
    )

    def __init__(
        self,
        *,
        plugin_handle: int,
        plugin_name: str,
        load_order_index: int,
        form_id: int,
        payload_hash: int,
    ) -> None:
        self.plugin_handle = plugin_handle
        self.plugin_name = plugin_name
        self.load_order_index = load_order_index
        self.form_id = form_id
        self.payload_hash = payload_hash


@dataclass
class ConflictReport:
    """Cross-plugin override chain for a single FormID."""

    form_id: int
    signature: str
    editor_id: str | None
    chain: list[OverrideEntry]
    status: ConflictStatus
    mergeable: bool

    @property
    def winner(self) -> OverrideEntry:
        return self.chain[-1]

    @property
    def master_entry(self) -> OverrideEntry:
        return self.chain[0]


@dataclass
class ConflictScan:
    """Result of `ConflictScanner.scan` — addressable several ways.

    - ``by_form_id`` is keyed by the *winner's raw on-disk form_id* (the
      value returned by ``ConflictReport.form_id``). For UI lookups scoped
      to a specific plugin, prefer ``report_for(handle, form_id)`` since
      non-winner entries have different raw form_ids.
    """

    by_form_id: dict[int, ConflictReport] = field(default_factory=dict)
    by_signature: dict[str, list[int]] = field(default_factory=dict)
    by_handle_form_id: dict[tuple[int, int], ConflictReport] = field(default_factory=dict)

    def __len__(self) -> int:
        return len(self.by_form_id)

    def __iter__(self):
        return iter(self.by_form_id.values())

    def reports_for(self, signature: str) -> list[ConflictReport]:
        return [self.by_form_id[fid] for fid in self.by_signature.get(signature, [])]

    def report_for(self, plugin_handle: int, form_id: int) -> ConflictReport | None:
        """Look up the conflict report for a specific plugin's view of a record.

        Each chain entry carries the form_id as it appears on disk in *that*
        plugin (different plugins encode the same record with different
        master bytes), so this is the safe entry point for UI nav-tree
        coloring where the form_id comes from the plugin being rendered.
        """
        return self.by_handle_form_id.get((int(plugin_handle), int(form_id)))


class ConflictScanner:
    """Dispatches to the native Rust scanner; rebuilds Python dataclasses."""

    def scan(
        self,
        session: EditorSession,
        *,
        signatures: Iterable[str] | None = None,
        cancel_event: threading.Event | None = None,
    ) -> ConflictScan:
        if not session.plugins:
            return ConflictScan()
        sig_list = [str(s) for s in signatures] if signatures else None
        handles = [
            (int(p.handle), str(p.plugin_name), int(p.load_order_index))
            for p in session.plugins
        ]
        if cancel_event is not None and cancel_event.is_set():
            return ConflictScan()
        try:
            payload = scan_conflicts(handles, signatures=sig_list)
        except Exception:
            _log.exception("Native conflict scan failed")
            return ConflictScan()

        scan = ConflictScan()
        for report_dict in payload:
            form_id = int(report_dict["form_id"])
            signature = str(report_dict["signature"])
            editor_id_raw = report_dict.get("editor_id")
            editor_id = str(editor_id_raw) if editor_id_raw is not None else None
            status = _STATUS_FROM_NATIVE.get(
                str(report_dict["status"]), ConflictStatus.CONFLICT
            )
            mergeable = bool(report_dict["mergeable"])
            chain_payload = report_dict.get("chain") or []
            chain = [
                OverrideEntry(
                    plugin_handle=int(c["plugin_handle"]),
                    plugin_name=str(c["plugin_name"]),
                    load_order_index=int(c["load_order_index"]),
                    # Each entry's form_id is the raw on-disk value from
                    # *that* plugin's perspective, not the report-level
                    # winner form_id — they generally differ because the
                    # high byte indexes each plugin's own masters list.
                    form_id=int(c.get("form_id", form_id)),
                    payload_hash=int(c["payload_hash"]),
                )
                for c in chain_payload
            ]
            report = ConflictReport(
                form_id=form_id,
                signature=signature,
                editor_id=editor_id,
                chain=chain,
                status=status,
                mergeable=mergeable,
            )
            scan.by_form_id[form_id] = report
            scan.by_signature.setdefault(signature, []).append(form_id)
            for entry in chain:
                scan.by_handle_form_id[(entry.plugin_handle, entry.form_id)] = report

        for fids in scan.by_signature.values():
            fids.sort()
        return scan
