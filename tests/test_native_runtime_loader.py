from types import SimpleNamespace


def test_load_native_module_falls_back_to_nested_extension(monkeypatch):
    from creation_lib.ba2 import native_runtime

    native_runtime._NATIVE_MODULE = None
    native_runtime._NATIVE_IMPORT_ATTEMPTED = False

    package_module = SimpleNamespace()
    extension_module = SimpleNamespace(archive_info=lambda path: {"path": path})
    calls: list[str] = []

    def _fake_import(name: str):
        calls.append(name)
        if name == "bsarchive_native":
            return package_module
        if name == "bsarchive_native.bsarchive_native":
            return extension_module
        raise ImportError(name)

    monkeypatch.setattr(native_runtime, "import_module", _fake_import)

    module = native_runtime.load_native_module()

    assert module is extension_module
    assert calls == ["bsarchive_native", "bsarchive_native.bsarchive_native"]


def test_load_native_module_uses_top_level_when_it_has_native_exports(monkeypatch):
    from creation_lib.ba2 import native_runtime

    native_runtime._NATIVE_MODULE = None
    native_runtime._NATIVE_IMPORT_ATTEMPTED = False

    top_level_module = SimpleNamespace(list_archive=lambda path: [path])
    calls: list[str] = []

    def _fake_import(name: str):
        calls.append(name)
        if name == "bsarchive_native":
            return top_level_module
        raise ImportError(name)

    monkeypatch.setattr(native_runtime, "import_module", _fake_import)

    module = native_runtime.load_native_module()

    assert module is top_level_module
    assert calls == ["bsarchive_native"]


def test_load_native_module_falls_back_to_umbrella_submodule(monkeypatch):
    from creation_lib.ba2 import native_runtime

    native_runtime._NATIVE_MODULE = None
    native_runtime._NATIVE_IMPORT_ATTEMPTED = False

    umbrella_module = SimpleNamespace(
        bsarchive_native=SimpleNamespace(list_archive=lambda path: [path]),
    )
    calls: list[str] = []

    def _fake_import(name: str):
        calls.append(name)
        if name in {"bsarchive_native", "bsarchive_native.bsarchive_native"}:
            raise ImportError(name)
        if name == "creation_lib._native":
            return umbrella_module
        raise ImportError(name)

    monkeypatch.setattr(native_runtime, "import_module", _fake_import)

    module = native_runtime.load_native_module()

    assert module is umbrella_module.bsarchive_native
    assert calls == ["bsarchive_native", "creation_lib._native"]


def test_load_native_module_raises_when_extension_is_missing(monkeypatch):
    from creation_lib.ba2 import native_runtime

    native_runtime._NATIVE_MODULE = None
    native_runtime._NATIVE_IMPORT_ATTEMPTED = False

    def _fake_import(name: str):
        raise ImportError(name)

    monkeypatch.setattr(native_runtime, "import_module", _fake_import)

    import pytest

    with pytest.raises(RuntimeError, match="required for archive operations"):
        native_runtime.load_native_module()
