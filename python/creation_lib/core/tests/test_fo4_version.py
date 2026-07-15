import sys
from pathlib import Path

import pytest

from creation_lib.core import fo4_version
from creation_lib.core.fo4_version import detect_exe_version, detect_fo4_version


def test_detect_exe_version_returns_none_for_missing_file(tmp_path):
    assert detect_exe_version(tmp_path / "nope.exe") is None


def test_detect_fo4_version_returns_none_without_exe(tmp_path):
    assert detect_fo4_version(str(tmp_path)) is None


def test_detect_fo4_version_returns_none_for_empty_root():
    assert detect_fo4_version("") is None


def test_detect_fo4_version_checks_root_dir_directly(tmp_path, monkeypatch):
    exe = tmp_path / "Fallout4.exe"

    def fake_detect_exe_version(path):
        return "1.10.163.0" if Path(path) == exe else None

    monkeypatch.setattr(fo4_version, "detect_exe_version", fake_detect_exe_version)
    assert detect_fo4_version(str(tmp_path)) == "1.10.163.0"


def test_detect_fo4_version_checks_parent_when_root_is_data_dir(tmp_path, monkeypatch):
    data_dir = tmp_path / "Data"
    exe = tmp_path / "Fallout4.exe"

    def fake_detect_exe_version(path):
        return "1.10.984.0" if Path(path) == exe else None

    monkeypatch.setattr(fo4_version, "detect_exe_version", fake_detect_exe_version)
    assert detect_fo4_version(str(data_dir)) == "1.10.984.0"


def test_detect_fo4_version_returns_none_when_neither_location_has_exe(tmp_path, monkeypatch):
    monkeypatch.setattr(fo4_version, "detect_exe_version", lambda path: None)
    assert detect_fo4_version(str(tmp_path / "Data")) is None


@pytest.mark.skipif(sys.platform != "win32", reason="Windows file-version API")
def test_detect_exe_version_reads_real_windows_exe():
    notepad = Path(r"C:\Windows\System32\notepad.exe")
    if not notepad.is_file():
        pytest.skip("notepad.exe not present")
    version = detect_exe_version(notepad)
    assert version is not None
    parts = version.split(".")
    assert len(parts) == 4
    assert all(p.isdigit() for p in parts)
    assert int(parts[0]) >= 6


from creation_lib.core.fo4_version import classify_ba2_target, detect_ba2_target


def test_classify_og_for_last_og_build():
    assert classify_ba2_target("1.10.163.0") == "og"


def test_classify_nextgen_for_first_ng_build():
    assert classify_ba2_target("1.10.980.0") == "nextgen"


def test_classify_nextgen_for_later_ng_build():
    assert classify_ba2_target("1.10.984.0") == "nextgen"


def test_classify_unknown_defaults_to_nextgen():
    assert classify_ba2_target(None) == "nextgen"
    assert classify_ba2_target("") == "nextgen"
    assert classify_ba2_target("garbage") == "nextgen"


def test_detect_ba2_target_uses_injected_reader():
    target, version = detect_ba2_target("C:/FO4", reader=lambda _root: "1.10.163.0")
    assert target == "og"
    assert version == "1.10.163.0"


def test_detect_ba2_target_missing_version_is_nextgen():
    target, version = detect_ba2_target("C:/FO4", reader=lambda _root: None)
    assert target == "nextgen"
    assert version is None
