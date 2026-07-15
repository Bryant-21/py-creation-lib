"""Tests for the native-backed previs merge Python wrapper."""
from __future__ import annotations

from pathlib import Path

import pytest

from creation_lib.nif import previs_merge


def _make_mod(tmp_path: Path) -> tuple[Path, Path]:
    mods_dir = tmp_path / "mods"
    mod_dir = mods_dir / "B21_Test"
    previs_tmp = mod_dir / "previs_tmp"
    previs_tmp.mkdir(parents=True)
    (mod_dir / "B21_Test.esp").write_bytes(b"plugin")
    return mods_dir, previs_tmp


def _patch_native(monkeypatch: pytest.MonkeyPatch) -> list[tuple]:
    import creation_lib.esp.authoring as authoring
    import creation_lib.esp.native_runtime as native_runtime

    calls: list[tuple] = []

    def require_native_function(name: str):
        assert name == "merge_previs_native"

        def merge_previs_native(*args):
            calls.append(args)
            return (1, 2, 0, 0, [])

        return merge_previs_native

    monkeypatch.setattr(authoring, "get_plugin_ext", lambda _mod_dir: "esp")
    monkeypatch.setattr(native_runtime, "_require_native_function", require_native_function)
    return calls


def test_merge_precombined_passes_only_combined_esp(tmp_path, monkeypatch):
    mods_dir, previs_tmp = _make_mod(tmp_path)
    combined = previs_tmp / "CombinedObjects.esp"
    combined.write_bytes(b"combined")
    calls = _patch_native(monkeypatch)

    previs_merge.merge_precombined("B21_Test", mods_dir=mods_dir)

    assert len(calls) == 1
    assert calls[0][1] == str(combined)
    assert calls[0][2] is None


def test_merge_previs_can_run_previs_only(tmp_path, monkeypatch):
    mods_dir, previs_tmp = _make_mod(tmp_path)
    previs = previs_tmp / "PreVis.esp"
    previs.write_bytes(b"previs")
    calls = _patch_native(monkeypatch)

    previs_merge.merge_previs(
        "B21_Test",
        mods_dir=mods_dir,
        include_combined=False,
        include_previs=True,
    )

    assert len(calls) == 1
    assert calls[0][1] is None
    assert calls[0][2] == str(previs)


def test_merge_previs_rejects_no_enabled_phases(tmp_path):
    mods_dir, _previs_tmp = _make_mod(tmp_path)

    with pytest.raises(ValueError, match="at least one merge phase"):
        previs_merge.merge_previs(
            "B21_Test",
            mods_dir=mods_dir,
            include_combined=False,
            include_previs=False,
        )
