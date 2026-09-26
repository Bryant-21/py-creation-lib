"""Unit tests for extract_game_data manifest helpers."""
import json

import pytest

from creation_lib.preprocessor.extraction import (
    build_manifest,
    load_manifest,
    manifest_matches,
    save_manifest,
)


@pytest.mark.parametrize(
    ("contents", "expected"),
    [(None, None), ("not json", None)],
    ids=["missing", "corrupt"],
)
def test_load_manifest_returns_none_when_unreadable(tmp_path, contents, expected):
    if contents is not None:
        (tmp_path / ".ba2_manifest.json").write_text(contents)
    assert load_manifest(tmp_path) is expected


def test_load_manifest_returns_dict(tmp_path):
    data = {
        "game": "fo4",
        "source_dir": "C:/Data",
        "extracted_at": "2026-01-01T00:00:00",
        "archives": {"Fallout4.ba2": {"size": 100, "mtime": 1.0}},
    }
    (tmp_path / ".ba2_manifest.json").write_text(json.dumps(data))
    result = load_manifest(tmp_path)
    assert result["game"] == "fo4"
    assert result["archives"]["Fallout4.ba2"]["size"] == 100


def test_build_manifest_structure_round_trips_through_save(tmp_path):
    a = tmp_path / "Fallout4.ba2"
    a.write_bytes(b"x" * 100)
    manifest = build_manifest("fo4", tmp_path, [a])
    assert manifest["game"] == "fo4"
    assert manifest["source_dir"] == str(tmp_path)
    assert "extracted_at" in manifest
    entry = manifest["archives"]["Fallout4.ba2"]
    assert entry["size"] == 100
    assert isinstance(entry["mtime"], float)

    save_manifest(tmp_path, manifest)
    result = json.loads((tmp_path / ".ba2_manifest.json").read_text())
    assert result["game"] == "fo4"


def test_save_manifest_overwrites_existing(tmp_path):
    (tmp_path / ".ba2_manifest.json").write_text(json.dumps({"old": True}))
    m = {"game": "skyrimse", "source_dir": str(tmp_path), "extracted_at": "2026-01-01T00:00:00", "archives": {}}
    save_manifest(tmp_path, m)
    result = json.loads((tmp_path / ".ba2_manifest.json").read_text())
    assert result["game"] == "skyrimse"
    assert "old" not in result


def _same(a, m):
    pass


def _bigger_size(a, m):
    a.write_bytes(b"x" * 200)


def _tampered_mtime(a, m):
    m["archives"]["Fallout4.ba2"]["mtime"] = 0.0


@pytest.mark.parametrize(
    ("mutate", "expected"),
    [(_same, True), (_bigger_size, False), (_tampered_mtime, False)],
    ids=["identical", "size-changed", "mtime-changed"],
)
def test_manifest_matches_detects_drift(tmp_path, mutate, expected):
    a = tmp_path / "Fallout4.ba2"
    a.write_bytes(b"x" * 100)
    m = build_manifest("fo4", tmp_path, [a])
    mutate(a, m)
    assert manifest_matches(m, tmp_path, [a]) is expected


def test_manifest_matches_source_dir_and_archive_set_and_none_manifest(tmp_path, tmp_path_factory):
    a = tmp_path / "Fallout4.ba2"
    b = tmp_path / "Fallout4 - Textures1.ba2"
    a.write_bytes(b"x" * 100)
    b.write_bytes(b"y" * 100)

    assert manifest_matches(None, tmp_path, [a]) is False

    m = build_manifest("fo4", tmp_path, [a])  # built with just 'a'
    assert manifest_matches(m, tmp_path, [a, b]) is True  # new archives are ignored

    other_dir = tmp_path_factory.mktemp("other")
    assert manifest_matches(m, other_dir, [a]) is False  # source dir changed

    m_both = build_manifest("fo4", tmp_path, [a, b])
    assert manifest_matches(m_both, tmp_path, [a]) is False  # 'b' removed
