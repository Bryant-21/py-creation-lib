"""Build Reference Info — warm the native back-reference cache.

The native crate (`plugin_index.rs`) lazily builds a back-reference index
the first time `get_referencing_form_ids` or similar is called. This module
exposes a UI-friendly "Build Reference Info" button that pre-warms the
index across every loaded plugin so subsequent ReferencedBy lookups are
instant.

xEdit equivalent: `mniNavBuildRef`.
"""

from __future__ import annotations

import logging
from typing import Callable

from creation_lib.esp.editor.session import EditorSession
from creation_lib.esp.native_runtime import (
    plugin_handle_force_build_refs_section,
    plugin_handle_get,
)

_log = logging.getLogger("creation_lib.esp.editor.reference_info")


def build_reference_index(
    session: EditorSession,
    *,
    on_progress: Callable[[str, int, int], None] | None = None,
) -> int:
    """Walk every loaded plugin, touching each record once to populate the
    native back-reference index. Returns the total number of records touched.

    `on_progress(plugin_name, done, total)` is invoked once per plugin.
    """
    plugins = session.plugins
    total_plugins = len(plugins)
    total_records = 0
    for i, plugin in enumerate(plugins):
        if on_progress is not None:
            on_progress(plugin.plugin_name, i, total_plugins)
        try:
            plugin_handle_force_build_refs_section(plugin.handle)
            total_records += int(plugin_handle_get(plugin.handle, "record_count", 0) or 0)
        except Exception:
            _log.exception("force_build_refs_section failed for %s", plugin.plugin_name)
            continue
    if on_progress is not None:
        on_progress("done", total_plugins, total_plugins)
    return total_records

