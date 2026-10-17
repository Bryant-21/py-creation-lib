from creation_lib.previs.build import select_cells


def _cell(key, action, cluster=None, grid=None):
    return {"form_key": key, "action": action, "cluster": cluster, "grid": grid}


def test_previs_targets_bring_their_whole_exterior_cluster():
    report = {
        "cells": [
            _cell("000A01:Fallout4.esm", "rebuild_precombines", "000A00:Fallout4.esm", [1, 1]),
            _cell("000B01:Fallout4.esm", "warn", "000B00:Fallout4.esm", [7, 7]),
            _cell("000C01:Fallout4.esm", "rebuild_previs", "000C01:Fallout4.esm"),
        ],
        "clusters": [
            {"form_key": "000A00:Fallout4.esm", "grid": [0, 0], "action": "rebuild_previs",
             "members": ["000A00:Fallout4.esm", "000A01:Fallout4.esm", "000A02:Fallout4.esm"]},
            {"form_key": "000B00:Fallout4.esm", "grid": [6, 6], "action": "warn",
             "members": ["000B00:Fallout4.esm", "000B01:Fallout4.esm"]},
        ],
    }
    precombine, seed = select_cells(report)
    assert precombine == ["000A01:Fallout4.esm"]
    assert seed == ["000A00:Fallout4.esm", "000A01:Fallout4.esm", "000A02:Fallout4.esm", "000C01:Fallout4.esm"]


def _plugin(path, flags):
    path.write_bytes(b"TES4" + (0).to_bytes(4, "little") + flags.to_bytes(4, "little") + bytes(12))
    return path


def test_light_plugins_are_told_apart_by_flag_or_extension(tmp_path):
    from creation_lib.previs.build import is_light_plugin

    assert not is_light_plugin(_plugin(tmp_path / "Mod.esp", 0))
    assert is_light_plugin(_plugin(tmp_path / "Flagged.esp", 0x200))
    assert is_light_plugin(_plugin(tmp_path / "Light.esl", 0))


def test_stamping_a_plugin_refuses_light_plugins_and_plugins_outside_the_load_order(tmp_path):
    import pytest

    from creation_lib.previs.build import build_patch
    from creation_lib.previs.sources import LoadOrderSpec

    light = _plugin(tmp_path / "Light.esp", 0x200)
    spec = LoadOrderSpec("test", tmp_path, [light], [], [])
    report = {"cells": [], "clusters": []}
    with pytest.raises(ValueError, match="light"):
        build_patch(spec, report, tmp_path / "out", work_dir=tmp_path / "work", into_plugin=light)
    with pytest.raises(ValueError, match="not in the load order"):
        build_patch(spec, report, tmp_path / "out", work_dir=tmp_path / "work", into_plugin=_plugin(tmp_path / "Other.esp", 0))


def test_plugins_loading_with_the_masters_are_told_apart_by_flag_or_extension(tmp_path):
    from creation_lib.previs.build import loads_as_master

    assert not loads_as_master(_plugin(tmp_path / "Mod.esp", 0))
    assert loads_as_master(_plugin(tmp_path / "Flagged.esp", 0x1))
    assert loads_as_master(_plugin(tmp_path / "LightFlagged.esp", 0x200))
    assert loads_as_master(_plugin(tmp_path / "Master.esm", 0))


class _FakeNative:
    def __init__(self, masters):
        self.masters = masters
        self.patch_request = None

    def write_patch_seed(self, request):
        import json

        request = json.loads(request)
        return json.dumps({
            "plugin": request["output"], "masters": self.masters, "skipped": [],
            "cells": [{"form_key": "000001:Master.esm", "form_id": 0x01000001}],
        })

    def prepare_previs(self, *_args, **_kwargs):
        class Prepared:
            def finalize(self, *_date):
                return '{"warnings": [], "generated": 0, "skipped_unsupported": 0}'
        return Prepared()

    def write_patch_plugin(self, request):
        import json

        self.patch_request = json.loads(request)
        return '{"cells": [], "unchanged": []}'


def _build_with_masters(tmp_path, monkeypatch, masters, plugin_type):
    from creation_lib.previs import build
    from creation_lib.previs.sources import LoadOrderSpec

    plugins = [_plugin(tmp_path / "Master.esm", 0x1), _plugin(tmp_path / "Mod.esp", 0)]
    native = _FakeNative(masters)
    monkeypatch.setattr(build, "load_native_module", lambda: native)
    monkeypatch.setattr(build, "check_load_order", lambda *_a, **_k: {"summary": {}})
    report = {"cells": [{"form_key": "000001:Master.esm", "action": "rebuild_previs"}], "clusters": []}
    result = build.build_patch(LoadOrderSpec("test", tmp_path, plugins, [], []), report, tmp_path / "out",
                               work_dir=tmp_path / "work", patch_name="Fix.esm", plugin_type=plugin_type)
    return result.patch.name, native.patch_request["esm"]


def test_a_patch_is_an_esp_by_default_and_an_esm_on_request(tmp_path, monkeypatch):
    assert _build_with_masters(tmp_path, monkeypatch, ["Master.esm"], "esp") == ("Fix.esp", False)
    assert _build_with_masters(tmp_path, monkeypatch, ["Master.esm"], "esm") == ("Fix.esm", True)


def test_an_esm_patch_mastering_a_plain_esp_is_written_as_an_esp(tmp_path, monkeypatch):
    assert _build_with_masters(tmp_path, monkeypatch, ["Master.esm", "Mod.esp"], "esm") == ("Fix.esp", False)
