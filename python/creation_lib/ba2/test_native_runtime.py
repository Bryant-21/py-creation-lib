from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
from threading import Event
from types import SimpleNamespace
import time
import pytest


def test_archive_entry_count_uses_native_header_reader(monkeypatch):
    import creation_lib.ba2.native_runtime as runtime

    calls = []
    native = SimpleNamespace(archive_entry_count=lambda path: calls.append(path) or 42)
    monkeypatch.setattr(runtime, "_NATIVE_MODULE", native)
    monkeypatch.setattr(runtime, "_NATIVE_IMPORT_ATTEMPTED", True)
    assert runtime.archive_entry_count("Main.ba2") == 42
    assert calls == ["Main.ba2"]


@pytest.mark.parametrize(
    ("entries", "output_path", "archive_type", "kwargs", "expected_kwargs", "expected_error"),
    [
        (
            [("X:/src/a.nif", "Meshes/a.nif")],
            "X:/out/archive.ba2",
            "fo4",
            dict(
                texture_archive=False,
                compress=True,
                compression_level=9,
                share_data=False,
                manifest_path="X:/manifest.json",
                jobs=3,
            ),
            {
                "compress": True,
                "compression_level": 9,
                "share_data": False,
                "manifest_path": "X:/manifest.json",
                "jobs": 3,
            },
            None,
        ),
        (
            [("source.dds", "Textures/source.dds")],
            "Textures_ps.ba2",
            "fo4psdds",
            dict(texture_archive=True),
            None,
            None,
        ),
        (
            [("X:/src/a.nif", "Meshes/a.nif")],
            "X:/out/archive.ba2",
            "fo4",
            dict(texture_archive=True),
            None,
            "texture_archive=True requires a texture archive type",
        ),
    ],
)
def test_pack_archive_entries_forwards_options(
    monkeypatch, entries, output_path, archive_type, kwargs, expected_kwargs, expected_error
):
    import creation_lib.ba2.native_runtime as runtime

    calls = []

    def fake_pack_archive_entries(entries, output_path, archive_type, **kwargs):
        calls.append((entries, output_path, archive_type, kwargs))
        return len(entries)

    fake_native = SimpleNamespace(pack_archive_entries=fake_pack_archive_entries)
    monkeypatch.setattr(runtime, "_NATIVE_MODULE", fake_native)
    monkeypatch.setattr(runtime, "_NATIVE_IMPORT_ATTEMPTED", True)

    if expected_error is not None:
        with pytest.raises(ValueError, match=expected_error):
            runtime.pack_archive_entries(entries, output_path, archive_type, **kwargs)
        assert calls == []
        return

    result = runtime.pack_archive_entries(entries, output_path, archive_type, **kwargs)

    assert result == len(entries)
    if expected_kwargs is not None:
        assert calls == [(entries, output_path, archive_type, expected_kwargs)]
    else:
        assert calls[0][:3] == (entries, output_path, archive_type)


def test_pack_archive_entries_refreshes_stale_native_module(monkeypatch):
    import creation_lib.ba2.native_runtime as runtime

    calls = []

    def fake_pack_archive_entries(entries, output_path, archive_type, **kwargs):
        calls.append((entries, output_path, archive_type, kwargs))
        return len(entries)

    stale_native = SimpleNamespace(pack_archive=lambda *args, **kwargs: None)
    fresh_native = SimpleNamespace(
        pack_archive=lambda *args, **kwargs: None,
        pack_archive_entries=fake_pack_archive_entries,
    )
    monkeypatch.setattr(runtime, "_NATIVE_MODULE", stale_native)
    monkeypatch.setattr(runtime, "_NATIVE_IMPORT_ATTEMPTED", True)
    monkeypatch.setattr(runtime, "_load_umbrella_submodule", lambda: fresh_native)

    result = runtime.pack_archive_entries(
        [("X:/src/a.nif", "Meshes/a.nif")],
        "X:/out/archive.ba2",
        "fo4",
    )

    assert result == 1
    assert runtime._NATIVE_MODULE is fresh_native
    assert calls == [
        (
            [("X:/src/a.nif", "Meshes/a.nif")],
            "X:/out/archive.ba2",
            "fo4",
            {
                "compress": True,
                "compression_level": None,
                "share_data": False,
                "manifest_path": None,
                "jobs": 0,
            },
        )
    ]


def test_load_native_module_is_thread_safe(monkeypatch):
    import creation_lib.ba2.native_runtime as runtime

    fake_native = SimpleNamespace(pack_archive=lambda *args, **kwargs: None)
    start_event = Event()

    def fake_import_module(name: str):
        if name != "bsarchive_native":
            raise ImportError(name)
        start_event.set()
        time.sleep(0.1)
        return fake_native

    monkeypatch.setattr(runtime, "import_module", fake_import_module)
    monkeypatch.setattr(runtime, "_NATIVE_MODULE", None)
    monkeypatch.setattr(runtime, "_NATIVE_IMPORT_ATTEMPTED", False)

    with ThreadPoolExecutor(max_workers=2) as executor:
        first = executor.submit(runtime.load_native_module)
        assert start_event.wait(timeout=1), "first thread never started import"
        second = executor.submit(runtime.load_native_module)

        assert first.result() is fake_native
        assert second.result() is fake_native


def test_extract_archive_forwards_workers_and_progress(monkeypatch):
    import creation_lib.ba2.native_runtime as runtime

    calls = []

    def fake_extract_archive(archive, output_dir, format_name, workers, progress):
        calls.append((archive, output_dir, format_name, workers, progress))
        if progress is not None:
            progress({"completed": 1, "total": 1})
        return 1

    fake_native = SimpleNamespace(extract_archive=fake_extract_archive)
    monkeypatch.setattr(runtime, "_NATIVE_MODULE", fake_native)
    monkeypatch.setattr(runtime, "_NATIVE_IMPORT_ATTEMPTED", True)
    progress_events = []

    result = runtime.extract_archive(
        "X:/game/Data/a.ba2",
        "X:/out",
        format="ba2",
        workers=7,
        progress=progress_events.append,
    )

    assert result == 1
    assert calls == [
        (
            "X:/game/Data/a.ba2",
            "X:/out",
            "ba2",
            7,
            progress_events.append,
        )
    ]
    assert progress_events == [{"completed": 1, "total": 1}]
