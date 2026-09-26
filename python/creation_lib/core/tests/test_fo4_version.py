from pathlib import Path

import pytest

from creation_lib.core import fo4_version
from creation_lib.core.fo4_version import (
    classify_ba2_target,
    detect_ba2_target,
    detect_exe_version,
    detect_fo4_version,
)


def test_detect_exe_version_and_fo4_version_return_none_when_exe_is_missing(tmp_path):
    assert detect_exe_version(tmp_path / "nope.exe") is None
    assert detect_fo4_version("") is None
    assert detect_fo4_version(str(tmp_path)) is None


def test_detect_fo4_version_checks_root_dir_and_parent_and_missing_exe(tmp_path, monkeypatch):
    # Phase 1: root dir is checked directly.
    exe = tmp_path / "Fallout4.exe"

    def fake_detect_exe_version_root(path):
        return "1.10.163.0" if Path(path) == exe else None

    monkeypatch.setattr(fo4_version, "detect_exe_version", fake_detect_exe_version_root)
    assert detect_fo4_version(str(tmp_path)) == "1.10.163.0"

    # Phase 2: when root is a Data dir, the parent dir is checked instead.
    data_dir = tmp_path / "Data"

    def fake_detect_exe_version_parent(path):
        return "1.10.984.0" if Path(path) == exe else None

    monkeypatch.setattr(fo4_version, "detect_exe_version", fake_detect_exe_version_parent)
    assert detect_fo4_version(str(data_dir)) == "1.10.984.0"

    # Phase 3: neither location having the exe returns None.
    monkeypatch.setattr(fo4_version, "detect_exe_version", lambda path: None)
    assert detect_fo4_version(str(tmp_path / "Data")) is None


@pytest.mark.parametrize(
    ("version", "expected"),
    [
        ("1.10.163.0", "og"),
        ("1.10.980.0", "nextgen"),
        ("1.10.984.0", "nextgen"),
        (None, "nextgen"),
        ("", "nextgen"),
        ("garbage", "nextgen"),
    ],
)
def test_classify_ba2_target(version, expected):
    assert classify_ba2_target(version) == expected


def test_detect_ba2_target_uses_injected_reader_and_defaults_to_nextgen_without_version():
    target, version = detect_ba2_target("C:/FO4", reader=lambda _root: "1.10.163.0")
    assert target == "og"
    assert version == "1.10.163.0"

    target, version = detect_ba2_target("C:/FO4", reader=lambda _root: None)
    assert target == "nextgen"
    assert version is None
