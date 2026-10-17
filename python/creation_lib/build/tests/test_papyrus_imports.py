import json
from types import SimpleNamespace
import pytest

from creation_lib.build.deployer import compile_papyrus


@pytest.mark.parametrize("converted", [False, True])
def test_converted_mod_uses_pex_event_headers_with_ck_sources_installed(tmp_path, monkeypatch, converted):
    mod = tmp_path / "B21_Test"
    sources = mod / "Scripts/Source/User"
    sources.mkdir(parents=True)
    (sources / "Child.psc").write_text(
        'Scriptname Child Extends ScriptObject\nFunction Send()\n'
        ' SendCustomEvent("ConvertedEvent")\nEndFunction\n')
    if converted:
        (mod / ".source_game").write_text("fo76")
    game_data = tmp_path / "Game/Data"
    base = game_data / "Scripts/Source/Base"
    base.mkdir(parents=True)
    (base / "ScriptObject.psc").write_text(
        "Scriptname ScriptObject\nFunction SendCustomEvent(CustomEventName name) Native\n")
    headers = tmp_path / "PexHeaders"
    headers.mkdir()
    (headers / "ScriptObject.psc").write_text(
        "Scriptname ScriptObject\nFunction SendCustomEvent(String name) Native\n")
    monkeypatch.setattr("creation_lib.pex.corpus.bundled_corpus_root", lambda game: headers)

    if converted:
        assert compile_papyrus(mod, "fo4", game_data) == 1
        assert (mod / "data/Scripts/Child.pex").is_file()
    else:
        with pytest.raises(RuntimeError, match="Papyrus compile failed"):
            compile_papyrus(mod, "fo4", game_data)


@pytest.mark.parametrize("raises", [False, True])
def test_compile_failure_preserves_existing_pex(tmp_path, monkeypatch, raises):
    sources = tmp_path / "Scripts/Source/User"
    sources.mkdir(parents=True)
    (sources / "Child.psc").write_text("Scriptname Child\n")
    output = tmp_path / "data/Scripts/Child.pex"
    output.parent.mkdir(parents=True)
    output.write_bytes(b"previous build")

    def failed_compile(*args, **kwargs):
        if raises:
            raise RuntimeError("compiler unavailable")
        return SimpleNamespace(ok=False, pex_bytes=None, diagnostics=[{"message": "invalid source"}])

    monkeypatch.setattr("creation_lib.pex.native_runtime.compile_psc", failed_compile)
    with pytest.raises(RuntimeError, match="Papyrus compile failed"):
        compile_papyrus(tmp_path, "fo4", tmp_path / "Game/Data")
    assert output.read_bytes() == b"previous build"
def test_mod_imports_reach_both_compilers_without_shipping_dependencies(tmp_path, monkeypatch):
    mod = tmp_path / "mods/B21_Test"
    sources = mod / "Scripts/Source/User"
    dependency = mod.parent / "Converted/Scripts/Source/User"
    sources.mkdir(parents=True)
    dependency.mkdir(parents=True)
    (dependency / "ParentScript.psc").write_text(
        "Scriptname ParentScript Extends Quest\nInt Property Counter Auto\n")
    (sources / "Child.psc").write_text(
        "Scriptname Child Extends ParentScript\nInt Function ReadEntry()\n"
        " Return Counter\nEndFunction\n")
    (mod / ".papyrus-imports.json").write_text(json.dumps(["../Converted/Scripts/Source/User"]))
    (mod / ".papyrus-stock.json").write_text(json.dumps(["Child.psc"]))
    (sources / "Native.psc").write_text("Scriptname Native Extends Quest\n")
    calls = []

    def stock(selected, **kwargs):
        calls.append(kwargs["imports"])
        assert selected == [sources / "Child.psc"]
        return {selected[0].resolve(): b"verified stock output"}

    def native(source, **kwargs):
        from creation_lib.pex.native_runtime import CompileResult
        calls.append(kwargs["imports"])
        assert kwargs["source_path"].endswith("Native.psc")
        return CompileResult(ok=True, pex_bytes=b"native output")

    monkeypatch.setattr("creation_lib.build.papyrus_verification.verify_stock_sources", stock)
    monkeypatch.setattr("creation_lib.pex.native_runtime.compile_psc", native)
    assert compile_papyrus(mod, "fo4", tmp_path / "Game/Data") == 2
    assert not (mod / "data/Scripts/ParentScript.pex").exists()
    assert (mod / "data/Scripts/Child.pex").read_bytes() == b"verified stock output"
    assert (mod / "data/Scripts/Native.pex").read_bytes() == b"native output"
    assert len(calls) == 2
    assert all(paths[:2] == [str(sources), str(dependency.resolve())] for paths in calls)


@pytest.mark.parametrize("configuration", [{}, ["missing"], [4]])
def test_invalid_import_configuration_fails_before_replacing_outputs(tmp_path, configuration):
    sources = tmp_path / "Scripts/Source/User"
    sources.mkdir(parents=True)
    (sources / "Child.psc").write_text("Scriptname Child\n")
    output = tmp_path / "data/Scripts/Child.pex"
    output.parent.mkdir(parents=True)
    output.write_bytes(b"previous build")
    (tmp_path / ".papyrus-imports.json").write_text(json.dumps(configuration))
    with pytest.raises(ValueError):
        compile_papyrus(tmp_path, "fo4", tmp_path / "Game/Data")
    assert output.read_bytes() == b"previous build"


def test_f4se_extended_sources_precede_vanilla_base_in_both_compilers(tmp_path, monkeypatch):
    mod = tmp_path / "mods/B21_Test"
    sources = mod / "Scripts/Source/User"
    sources.mkdir(parents=True)
    (sources / "Child.psc").write_text("Scriptname Child Extends Quest\n")
    (sources / "Native.psc").write_text("Scriptname Native Extends Quest\n")
    (mod / ".papyrus-stock.json").write_text(json.dumps(["Child.psc"]))
    game_data = tmp_path / "Game/Data"
    extended = game_data / "Scripts/Source"
    base = extended / "Base"
    base.mkdir(parents=True)
    (base / "Form.psc").write_text("Scriptname Form Native Hidden\n")
    (extended / "Form.psc").write_text(
        "Scriptname Form Native Hidden\nString Function GetName() Native\n")
    calls = []

    def stock(selected, **kwargs):
        calls.append(kwargs["imports"])
        return {selected[0].resolve(): b"stock output"}

    def native(source, **kwargs):
        from creation_lib.pex.native_runtime import CompileResult
        calls.append(kwargs["imports"])
        return CompileResult(ok=True, pex_bytes=b"native output")

    monkeypatch.setattr("creation_lib.build.papyrus_verification.verify_stock_sources", stock)
    monkeypatch.setattr("creation_lib.pex.native_runtime.compile_psc", native)
    assert compile_papyrus(mod, "fo4", game_data) == 2
    assert len(calls) == 2
    for imports in calls:
        assert str(extended) in imports
        assert imports.index(str(extended)) < imports.index(str(base))
