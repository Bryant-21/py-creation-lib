"""Thin Python boundary for native previs_native entrypoints."""

from __future__ import annotations

from importlib import import_module
from typing import Any


def load_native_module() -> Any:
    module = getattr(import_module("creation_lib._native"), "previs_native", None)
    if module is None:
        raise RuntimeError("creation_lib._native was built without previs_native; run scripts/ensure_native.py --package creation")
    return module
