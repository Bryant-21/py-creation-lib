from pathlib import Path

import pytest

from creation_lib.ba2 import BA2Manager


def _install_native_stubs(monkeypatch: pytest.MonkeyPatch, *, files: list[str], payload: bytes = b"DATA") -> list[tuple[str, str]]:
    monkeypatch.setattr(
        "creation_lib.ba2.ba2_manager.native_runtime.list_archive",
        lambda path: files,
    )
    monkeypatch.setattr(
        "creation_lib.ba2.ba2_manager.native_runtime.archive_info",
        lambda path: {"format": "fo4_dx10", "version": 1, "file_count": len(files)},
    )
    calls: list[tuple[str, str]] = []

    def _extract_one(archive_path: str, member_path: str) -> bytes:
        calls.append((archive_path, member_path))
        return payload

    monkeypatch.setattr(
        "creation_lib.ba2.ba2_manager.native_runtime.extract_one",
        _extract_one,
    )
    return calls


def test_scan_directories_collects_supported_archive_extensions(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    (tmp_path / "textures.ba2").write_bytes(b"")
    (tmp_path / "meshes.bsa").write_bytes(b"")
    (tmp_path / "readme.txt").write_text("ignore", encoding="utf-8")
    _install_native_stubs(monkeypatch, files=["textures/test.dds"])

    mgr = BA2Manager(use_cache=False)
    mgr.scan_directories([tmp_path])

    assert mgr.archive_count == 2
    mgr.close_all()


def test_find_uses_native_routing_and_normalizes_paths(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    archive = tmp_path / "textures.ba2"
    archive.write_bytes(b"")
    extract_calls = _install_native_stubs(monkeypatch, files=["textures/test.dds"], payload=b"DDS")

    mgr = BA2Manager(use_cache=False)
    mgr.scan_directories([tmp_path])

    assert mgr.find("Data\\Textures\\Test.DDS") == b"DDS"
    assert extract_calls == [(str(archive), "textures/test.dds")]
    mgr.close_all()


def test_find_returns_none_for_missing_member(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    (tmp_path / "textures.ba2").write_bytes(b"")
    _install_native_stubs(monkeypatch, files=["textures/test.dds"])

    mgr = BA2Manager(use_cache=False)
    mgr.scan_directories([tmp_path])

    assert mgr.find("textures/missing.dds") is None
    mgr.close_all()


def test_close_all_clears_native_manager_state(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    (tmp_path / "textures.ba2").write_bytes(b"")
    _install_native_stubs(monkeypatch, files=["textures/test.dds"])

    mgr = BA2Manager(use_cache=False)
    mgr.scan_directories([tmp_path])
    _ = mgr.total_file_count

    mgr.close_all()

    assert mgr.archive_count == 0
    assert mgr.total_file_count == 0
