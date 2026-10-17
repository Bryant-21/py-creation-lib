"""Runs the native previsbine checker over a load order."""

from __future__ import annotations

import json
from typing import Callable

from creation_lib.previs.native_runtime import load_native_module
from creation_lib.previs.sources import LoadOrderSpec


def check_load_order(
    spec: LoadOrderSpec,
    *,
    ignore_base_only: bool = True,
    restore_disabled: bool = False,
    unprecombined_threshold: int | None = None,
    rebuild_plugin: str | None = None,
    rebuild_all: bool = False,
    workers: int | None = None,
    progress: Callable[[str, int, int], None] | None = None,
) -> dict:
    """Returns the report: ``summary``, ``cells`` and ``clusters`` (see ``detect::CheckReport``).

    ``rebuild_plugin`` (a file name) marks every CELL that plugin touches for
    a rebuild, broken or not; ``rebuild_all`` marks every CELL of the load
    order, as for a full previsbine base such as PPF.esm.
    """
    request = spec.request(
        ignore_base_only=ignore_base_only,
        restore_disabled=restore_disabled,
        unprecombined_threshold=unprecombined_threshold,
        rebuild_plugin=rebuild_plugin,
        rebuild_all=rebuild_all or None,
    )
    raw = load_native_module().check_load_order(json.dumps(request), workers, progress)
    return json.loads(raw)
