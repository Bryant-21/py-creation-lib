"""native_runtime wrapper error path when the compiled extension is unavailable.

The byte-exact native-vs-oracle extraction contract lives in
test_archive_extraction_contract.py (synthetic archives, no real game data).
"""
from __future__ import annotations

import pytest

from creation_lib.ba2 import native_runtime


def test_python_wrappers_raise_when_module_missing(monkeypatch):
    # Simulate import failure by clearing the cached module.
    monkeypatch.setattr(native_runtime, "_NATIVE_MODULE", None)
    monkeypatch.setattr(native_runtime, "_NATIVE_IMPORT_ATTEMPTED", True)
    with pytest.raises(RuntimeError, match="required for archive operations"):
        native_runtime.list_archive("unused")
    with pytest.raises(RuntimeError, match="required for archive operations"):
        native_runtime.archive_info("unused")
    with pytest.raises(RuntimeError, match="required for archive operations"):
        native_runtime.extract_one("unused", "unused")
    with pytest.raises(RuntimeError, match="required for archive operations"):
        native_runtime.extract_archive("unused", "unused")
    with pytest.raises(RuntimeError, match="required for archive operations"):
        native_runtime.pack_archive("unused", "unused", "fo4")
