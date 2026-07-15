"""Unit tests for extract_game_data manifest helpers."""
import json
import sys
import os
from pathlib import Path
import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from creation_lib.preprocessor.extraction import (
    load_manifest,
    build_manifest,
    manifest_matches,
    save_manifest,
)


# ---------------------------------------------------------------------------
# load_manifest
# ---------------------------------------------------------------------------

def test_load_manifest_missing_returns_none(tmp_path):
    assert load_manifest(tmp_path) is None


def test_load_manifest_corrupt_returns_none(tmp_path):
    (tmp_path / ".ba2_manifest.json").write_text("not json")
    assert load_manifest(tmp_path) is None


def test_load_manifest_returns_dict(tmp_path):
    data = {"game": "fo4", "source_dir": "C:/Data", "extracted_at": "2026-01-01T00:00:00",
            "archives": {"Fallout4.ba2": {"size": 100, "mtime": 1.0}}}
    (tmp_path / ".ba2_manifest.json").write_text(json.dumps(data))
    result = load_manifest(tmp_path)
    assert result["game"] == "fo4"
    assert result["archives"]["Fallout4.ba2"]["size"] == 100


# ---------------------------------------------------------------------------
# build_manifest
# ---------------------------------------------------------------------------

def test_build_manifest_structure(tmp_path):
    # Create fake archive files
    a = tmp_path / "Fallout4.ba2"
    a.write_bytes(b"x" * 100)
    manifest = build_manifest("fo4", tmp_path, [a])
    assert manifest["game"] == "fo4"
    assert manifest["source_dir"] == str(tmp_path)
    assert "extracted_at" in manifest
    assert "Fallout4.ba2" in manifest["archives"]
    entry = manifest["archives"]["Fallout4.ba2"]
    assert entry["size"] == 100
    assert isinstance(entry["mtime"], float)


# ---------------------------------------------------------------------------
# manifest_matches
# ---------------------------------------------------------------------------

def test_manifest_matches_identical(tmp_path):
    a = tmp_path / "Fallout4.ba2"
    a.write_bytes(b"x" * 200)
    archives = [a]
    m = build_manifest("fo4", tmp_path, archives)
    assert manifest_matches(m, tmp_path, archives) is True


def test_manifest_matches_source_dir_mismatch(tmp_path, tmp_path_factory):
    a = tmp_path / "Fallout4.ba2"
    a.write_bytes(b"x" * 200)
    other_dir = tmp_path_factory.mktemp("other")
    m = build_manifest("fo4", tmp_path, [a])
    assert manifest_matches(m, other_dir, [a]) is False


def test_manifest_matches_archive_added_ignored(tmp_path):
    a = tmp_path / "Fallout4.ba2"
    b = tmp_path / "Fallout4 - Textures1.ba2"
    a.write_bytes(b"x" * 100)
    b.write_bytes(b"y" * 100)
    m = build_manifest("fo4", tmp_path, [a])  # built with just 'a'
    assert manifest_matches(m, tmp_path, [a, b]) is True  # new archives are ignored


def test_manifest_matches_archive_removed(tmp_path):
    a = tmp_path / "Fallout4.ba2"
    b = tmp_path / "Fallout4 - Textures1.ba2"
    a.write_bytes(b"x" * 100)
    b.write_bytes(b"y" * 100)
    m = build_manifest("fo4", tmp_path, [a, b])
    assert manifest_matches(m, tmp_path, [a]) is False  # 'b' removed


def test_manifest_matches_size_changed(tmp_path):
    a = tmp_path / "Fallout4.ba2"
    a.write_bytes(b"x" * 100)
    m = build_manifest("fo4", tmp_path, [a])
    a.write_bytes(b"x" * 200)  # different size
    assert manifest_matches(m, tmp_path, [a]) is False


def test_manifest_matches_mtime_changed(tmp_path):
    a = tmp_path / "Fallout4.ba2"
    a.write_bytes(b"x" * 100)
    m = build_manifest("fo4", tmp_path, [a])
    # Tamper with mtime in the manifest (same size, different mtime)
    m["archives"]["Fallout4.ba2"]["mtime"] = 0.0
    assert manifest_matches(m, tmp_path, [a]) is False


def test_manifest_matches_none_manifest(tmp_path):
    a = tmp_path / "Fallout4.ba2"
    a.write_bytes(b"x" * 100)
    assert manifest_matches(None, tmp_path, [a]) is False


# ---------------------------------------------------------------------------
# save_manifest
# ---------------------------------------------------------------------------

def test_save_manifest_writes_file(tmp_path):
    m = {"game": "fo4", "source_dir": str(tmp_path), "extracted_at": "2026-01-01T00:00:00",
         "archives": {}}
    save_manifest(tmp_path, m)
    result = json.loads((tmp_path / ".ba2_manifest.json").read_text())
    assert result["game"] == "fo4"


def test_save_manifest_overwrites_existing(tmp_path):
    (tmp_path / ".ba2_manifest.json").write_text(json.dumps({"old": True}))
    m = {"game": "skyrimse", "source_dir": str(tmp_path), "extracted_at": "2026-01-01T00:00:00",
         "archives": {}}
    save_manifest(tmp_path, m)
    result = json.loads((tmp_path / ".ba2_manifest.json").read_text())
    assert result["game"] == "skyrimse"
    assert "old" not in result
