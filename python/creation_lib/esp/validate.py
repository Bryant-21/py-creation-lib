"""Authoring-dir validator — checks FormKey references inside a mod's YAML.

This is the public entry point for "is this mod's YAML internally consistent
and well-formed?". It uses the native ``validate_authoring`` function and
applies no Python policy of its own; see
``py_creation_lib/native/esp/src/authoring_validate.rs`` for the rules.

This validator only reads files inside ``yaml_dir`` and trusts that declared
masters are valid.
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
