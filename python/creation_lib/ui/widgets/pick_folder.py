"""File / folder picker helpers backed by imgui_bundle's portable_file_dialogs.

Each function is synchronous: it spawns the native dialog via pfd and polls
until the user closes it.
"""
from __future__ import annotations

import time

from imgui_bundle import portable_file_dialogs as pfd

_POLL_INTERVAL = 0.02


def _tk_filters_to_pfd(filetypes: list[tuple[str, str]] | None) -> list[str]:
    """Convert [(name, pattern), ...] to pfd's flat [name, pattern, ...] list."""
    if not filetypes:
        return ["All files", "*.*"]
    flat: list[str] = []
    for name, pattern in filetypes:
        flat.append(name)
        flat.append(pattern)
    return flat


def _wait(dlg) -> None:
    while not dlg.ready():
        time.sleep(_POLL_INTERVAL)


def pick_folder(title: str = "Select Folder", default_path: str = "") -> str | None:
    """Open a native folder dialog. Returns path or None."""
    dlg = pfd.select_folder(title, default_path)
    _wait(dlg)
    return dlg.result() or None


def pick_file(
    title: str = "Select File",
    filetypes: list[tuple[str, str]] | None = None,
    default_path: str = "",
) -> str | None:
    """Open a native open-file dialog. Returns path or None."""
    dlg = pfd.open_file(title, default_path, _tk_filters_to_pfd(filetypes))
    _wait(dlg)
    results = dlg.result()
    return results[0] if results else None


def pick_save_file(
    title: str = "Save File",
    filetypes: list[tuple[str, str]] | None = None,
    default_ext: str = "",
    initialfile: str = "",
    default_path: str = "",
) -> str | None:
    """Open a native save-file dialog. Returns path or None.

    `default_ext` and `initialfile` are accepted for compatibility.
    `initialfile` is combined with `default_path` (if both provided) to seed the dialog.
    """
    start = default_path
    if initialfile:
        if start:
            import os
            start = os.path.join(start, initialfile)
        else:
            start = initialfile
    dlg = pfd.save_file(title, start, _tk_filters_to_pfd(filetypes))
    _wait(dlg)
    result = dlg.result()
    if not result:
        return None
    if default_ext and "." not in result.rsplit("/", 1)[-1].rsplit("\\", 1)[-1]:
        result += default_ext if default_ext.startswith(".") else "." + default_ext
    return result
