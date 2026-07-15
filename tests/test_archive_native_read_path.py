from pathlib import Path

import pytest

from creation_lib.ba2.ba2_manager import BA2Manager
from creation_lib.preprocessor import extraction


def test_ba2_manager_prefers_native_read_backend(tmp_path, monkeypatch: pytest.MonkeyPatch):
    archive = tmp_path / "TestArchive.ba2"
    archive.write_bytes(b"stub")

    monkeypatch.setattr(
        "creation_lib.ba2.ba2_manager.native_runtime.native_function_available",
        lambda name: name in {"list_archive", "archive_info", "extract_one"},
    )
    monkeypatch.setattr(
        "creation_lib.ba2.ba2_manager.native_runtime.list_archive",
        lambda path: ["textures/test.dds"],
    )
    monkeypatch.setattr(
        "creation_lib.ba2.ba2_manager.native_runtime.archive_info",
        lambda path: {"format": "fo4_dx10", "version": 1, "file_count": 1},
    )

    extract_calls: list[tuple[str, str]] = []

    def _fake_extract_one(archive_path: str, file_path: str) -> bytes:
        extract_calls.append((archive_path, file_path))
        return b"DDS"

    monkeypatch.setattr(
        "creation_lib.ba2.ba2_manager.native_runtime.extract_one",
        _fake_extract_one,
    )

    mgr = BA2Manager(use_cache=False)
    mgr.scan_directories([tmp_path])

    assert mgr.total_file_count == 1
    assert mgr.archives[0]["type"] == "DX10"
    assert mgr.find("Data\\Textures\\Test.DDS") == b"DDS"
    assert extract_calls == [(str(archive), "textures/test.dds")]

    mgr.close_all()


def test_extract_one_prefers_native_archive_backend(tmp_path, monkeypatch: pytest.MonkeyPatch):
    archive = tmp_path / "TestArchive.bsa"
    archive.write_bytes(b"stub")
    output_dir = tmp_path / "out"

    native_calls: list[tuple[str, str, str | None, int]] = []

    def _fake_native_extract(archive_path: str, out_dir: str, *, format: str | None = None, workers: int = 0) -> int:
        native_calls.append((archive_path, out_dir, format, workers))
        target = Path(out_dir) / "meshes" / "test.nif"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(b"NIF")
        return 1

    monkeypatch.setattr(extraction.native_runtime, "extract_archive", _fake_native_extract)

    result = extraction.extract_one(
        archive,
        output_dir,
        "bsa",
        file_workers=4,
    )

    assert result == (archive, 1, None)
    assert native_calls == [(str(archive), str(output_dir), "bsa", 4)]
    assert (output_dir / "meshes" / "test.nif").read_bytes() == b"NIF"
