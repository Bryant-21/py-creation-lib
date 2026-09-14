"""Authoring-dir validator: checks FormKey references inside a mod's YAML.

Wraps native ``validate_authoring`` with no Python policy; the rules live in
``py_creation_lib/native/esp/src/authoring_validate.rs``. Reads only files inside
``yaml_dir`` and trusts declared masters.
"""

from __future__ import annotations

from pathlib import Path

from creation_lib.esp import native_runtime as _esp_native


def validate_authoring(yaml_dir: str | Path) -> tuple[list[dict], int]:
    """Validate a mod's authoring YAML directory.

    Returns ``(errors, checked_count)``. Each error is a dict shaped
    ``{"file", "line", "formkey", "reason"}``. ``file`` is relative to the
    mod root (parent of ``yaml_dir``) when possible.
    """
    yaml_dir = str(yaml_dir)
    result = _esp_native.validate_authoring(yaml_dir)
    errors = list(result.get("errors", []))
    checked = int(result.get("checked", 0))
    return errors, checked
