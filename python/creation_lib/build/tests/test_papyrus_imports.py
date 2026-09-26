import json
import pytest

from creation_lib.build.deployer import compile_papyrus
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
