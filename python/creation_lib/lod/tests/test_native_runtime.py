from __future__ import annotations

import json
import sys
import types

import pytest

from creation_lib.lod import native_runtime


def test_load_native_module_returns_submodule(monkeypatch):
    fake = types.SimpleNamespace(generate_lod=lambda *a, **k: None)
    umbrella = types.ModuleType("creation_lib._native")
    umbrella.lodgen_native = fake
    monkeypatch.setitem(sys.modules, "creation_lib._native", umbrella)
    native_runtime._reset_for_tests()
    assert native_runtime.load_native_module() is fake
    assert native_runtime.is_available() is True


def test_load_native_module_missing_raises(monkeypatch):
    umbrella = types.ModuleType("creation_lib._native")  # no lodgen_native attr
    monkeypatch.setitem(sys.modules, "creation_lib._native", umbrella)
    sys.modules.pop("creation_lib._native.lodgen_native", None)
    native_runtime._reset_for_tests()
    with pytest.raises(RuntimeError):
        native_runtime.load_native_module()


def _fake_native():
    captured = {}

    class PyLodPaths:
        def __init__(
            self,
            data_dirs,
            output_dir,
            working_esm=None,
            source_data_dir=None,
            object_lod_overlay=None,
        ):
            self.data_dirs = list(data_dirs)
            self.output_dir = output_dir
            self.working_esm = working_esm
            self.source_data_dir = source_data_dir
            self.object_lod_overlay = object_lod_overlay

    class PyLodGenStats:
        def __init__(self):
            self.btr = 7
            self.bto = 3
            self.btt = 0
            self.dds = 14
            self.lod_written = True
            self.warnings = ["one bad quad"]

    def generate_lod(world_editor_id, settings_json, paths, progress):
        captured["world"] = world_editor_id
        captured["settings_json"] = settings_json
        captured["data_dirs"] = list(paths.data_dirs)
        captured["output_dir"] = paths.output_dir
        captured["working_esm"] = paths.working_esm
        captured["source_data_dir"] = paths.source_data_dir
        captured["object_lod_overlay"] = paths.object_lod_overlay
        captured["progress"] = progress
        if progress is not None:
            progress("terrain", 0.5)
        return PyLodGenStats()

    ns = types.SimpleNamespace(
        PyLodPaths=PyLodPaths, generate_lod=generate_lod, _captured=captured
    )
    return ns


def test_generate_lod_marshals_dict_settings(monkeypatch):
    fake = _fake_native()
    monkeypatch.setattr(native_runtime, "load_native_module", lambda: fake)
    events = []
    result = native_runtime.generate_lod(
        "DLC03FarHarbor",
        {"global": {"lod_min": 4, "lod_max": 32}},
        data_dirs=["C:/Data", "D:/mods"],
        output_dir="C:/out",
        progress=lambda m, f: events.append((m, f)),
    )
    assert fake._captured["world"] == "DLC03FarHarbor"
    assert json.loads(fake._captured["settings_json"]) == {"global": {"lod_min": 4, "lod_max": 32}}
    assert fake._captured["data_dirs"] == ["C:/Data", "D:/mods"]
    assert fake._captured["output_dir"] == "C:/out"
    # No explicit plugin_path → working_esm stays None (legacy data_dirs discovery).
    assert fake._captured["working_esm"] is None
    assert fake._captured["source_data_dir"] is None
    assert fake._captured["object_lod_overlay"] is None
    assert events == [("terrain", 0.5)]
    assert result.btr == 7 and result.dds == 14 and result.lod_written is True
    assert result.warnings == ("one bad quad",)


def test_generate_lod_passes_str_settings_through(monkeypatch):
    fake = _fake_native()
    monkeypatch.setattr(native_runtime, "load_native_module", lambda: fake)
    native_runtime.generate_lod(
        "W", '{"already":"json"}', data_dirs=[], output_dir="o", progress=None,
    )
    assert fake._captured["settings_json"] == '{"already":"json"}'
    assert fake._captured["progress"] is None


def test_generate_lod_marshals_plugin_path(monkeypatch):
    fake = _fake_native()
    monkeypatch.setattr(native_runtime, "load_native_module", lambda: fake)
    native_runtime.generate_lod(
        "APPALACHIA",
        {"global": {}},
        data_dirs=["C:/out/data", "X:/extracted/fo4"],
        output_dir="C:/out/data",
        plugin_path="C:/out/SeventySix.esm",
        progress=None,
    )
    # The working ESM is pinned as the sole plugin source; asset dirs stay separate.
    assert fake._captured["working_esm"] == "C:/out/SeventySix.esm"
    assert fake._captured["data_dirs"] == ["C:/out/data", "X:/extracted/fo4"]


def test_generate_lod_marshals_source_data_dir(monkeypatch):
    fake = _fake_native()
    monkeypatch.setattr(native_runtime, "load_native_module", lambda: fake)
    native_runtime.generate_lod(
        "APPALACHIA",
        {"global": {}, "objects": {"source": "fo76_bto"}},
        data_dirs=["C:/out/data", "X:/extracted/fo4"],
        output_dir="C:/out/data",
        plugin_path="C:/out/SeventySix.esm",
        source_data_dir="X:/extracted/fo76",
        progress=None,
    )
    assert fake._captured["source_data_dir"] == "X:/extracted/fo76"


def test_generate_lod_marshals_object_lod_overlay(monkeypatch):
    fake = _fake_native()
    monkeypatch.setattr(native_runtime, "load_native_module", lambda: fake)
    native_runtime.generate_lod(
        "Mojave",
        {"global": {}, "objects": {"source": "records"}},
        data_dirs=["C:/out/data"],
        output_dir="C:/out/data",
        plugin_path="C:/out/FNV.esm",
        object_lod_overlay="C:/out/.modkit/object_lod_overlay.v1.json",
        progress=None,
    )
    assert fake._captured["object_lod_overlay"] == (
        "C:/out/.modkit/object_lod_overlay.v1.json"
    )


def test_discover_worldspaces_uses_built_plugin(monkeypatch):
    calls = []
    fake = types.SimpleNamespace(
        discover_worldspaces=lambda plugin_path, game: calls.append(
            (plugin_path, game)
        )
        or ["Tamriel", "Blackreach"]
    )
    monkeypatch.setattr(native_runtime, "load_native_module", lambda: fake)

    assert native_runtime.discover_worldspaces(
        "C:/mods/Skyrim/Skyrim_Merged.esm"
    ) == ("Tamriel", "Blackreach")
    assert calls == [("C:/mods/Skyrim/Skyrim_Merged.esm", "fo4")]


def test_count_fo76_bto_tiles_uses_native_enumerator(monkeypatch):
    calls = []
    fake = types.SimpleNamespace(
        count_fo76_bto_tiles=lambda source_root, world: calls.append(
            (source_root, world)
        )
        or 37
    )
    monkeypatch.setattr(native_runtime, "load_native_module", lambda: fake)

    assert native_runtime.count_fo76_bto_tiles(
        "X:/extracted/fo76", "EXM1PittWorldspace"
    ) == 37
    assert calls == [("X:/extracted/fo76", "EXM1PittWorldspace")]
