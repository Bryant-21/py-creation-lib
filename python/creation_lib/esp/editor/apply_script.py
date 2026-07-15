"""Apply Script boundary.

Apply Script must run against Rust-owned record operations, not Python against
full Record objects written back to Rust (native-boundary rule).
"""

from __future__ import annotations

import logging
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable

from creation_lib.esp.editor.session import EditorSession, LoadedPlugin

_log = logging.getLogger("creation_lib.esp.editor.apply_script")


@dataclass
class ScriptContext:
    """Runtime context passed to user script lifecycle hooks."""

    session: EditorSession
    target: LoadedPlugin | None = None
    processed_count: int = 0
    error_count: int = 0
    aborted: bool = False
    silent: bool = False
    messages: list[str] = field(default_factory=list)

    def log(self, msg: str) -> None:
        text = str(msg)
        self.messages.append(text)
        _log.info("script: %s", text)

    def abort(self) -> None:
        self.aborted = True


def run_script(
    script_path: str | Path,
    records: Iterable,
    session: EditorSession,
    *,
    target_handle: int | None = None,
) -> ScriptContext:
    """Apply Script is disabled until scripts target Rust-owned record APIs."""
    path = Path(script_path)
    if not path.is_file():
        raise FileNotFoundError(f"script not found: {path}")
    _ = records, target_handle
    ctx = ScriptContext(session=session, target=session.active)
    ctx.error_count = 1
    ctx.log("Apply Script is disabled until scripts run through Rust record APIs.")
    return ctx
