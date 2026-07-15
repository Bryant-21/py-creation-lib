"""Tests for archive file-table caching."""
import struct
import sqlite3
import time
import zlib
from pathlib import Path

import pytest

from creation_lib.ba2.archive_cache import ArchiveCache
from creation_lib.ba2.ba2_manager import BA2Manager
from creation_lib.ba2.ba2_reader import BA2File
from creation_lib.ba2.bsa_reader import BSAReader


# ---- helpers to build minimal test archives ----

BA2_MAGIC = b"BTDX"
BSA_MAGIC = b"BSA\x00"


def _build_minimal_ba2(path, file_name="meshes/test.nif", file_data=b"NIF_DATA"):
    """Build a minimal valid BA2 (GNRL, v1) with one file."""
    num_files = 1
    # Header: magic(4) + version(4) + type(4) + numFiles(4) + nameTableOffset(8) = 24
    # GNRL record: hash(4)+ext(4)+dirhash(4)+flags(4)+offset(8)+packed(4)+unpacked(4)+pad(4) = 36
    header_size = 24
    record_size = 36
    records_end = header_size + record_size * num_files
    # File data starts right after records
    data_offset = records_end
    data_end = data_offset + len(file_data)
    # Name table at end
    name_table_offset = data_end
    name_bytes = file_name.encode("utf-8")

    header = struct.pack(
        "<4s III Q",
        BA2_MAGIC,
        1,  # version
        0x4C524E47,  # GNRL
        num_files,
        name_table_offset,
    )
    record = struct.pack(
        "<IIII Q III",
        0, 0, 0, 0,  # hash, ext, dirhash, flags
        data_offset,
        0,  # packed_len=0 → uncompressed
        len(file_data),
        0,  # pad
    )
    name_table = struct.pack("<H", len(name_bytes)) + name_bytes
    path.write_bytes(header + record + file_data + name_table)
    return path


def _build_minimal_bsa(path, version=105, file_path="meshes/test.nif", file_data=b"NIF file content for testing"):
    """Build a minimal valid BSA with one file."""
    rel = file_path.replace("\\", "/")
    folder, _, name = rel.rpartition("/")
    folder_name = ((folder or "").encode("utf-8") + b"\x00") if folder else b""
    file_name = name.encode("utf-8") + b"\x00"
    archive_flags = 0x003  # has folder names + file names

    folder_count = 1
    file_count = 1
    total_folder_name_len = len(folder_name)
    total_file_name_len = len(file_name)

    header_size = 36
    folder_record_size = 24 if version >= 105 else 16
    folder_records_offset = header_size
    file_record_block_offset = folder_records_offset + folder_record_size * folder_count
    file_record_block_size = 1 + len(folder_name) + 16 * file_count
    file_name_block_offset = file_record_block_offset + file_record_block_size
    file_data_offset = file_name_block_offset + total_file_name_len

    stored_size = len(file_data)
    actual_data = file_data
    file_record_abs_offset = file_record_block_offset + total_file_name_len

    header = struct.pack(
        "<4s I I I I I I I HH",
        BSA_MAGIC, version, header_size, archive_flags,
        folder_count, file_count, total_folder_name_len, total_file_name_len,
        0, 0,
    )
    folder_hash = 0x0123456789ABCDEF
    if version >= 105:
        folder_record = struct.pack("<Q I I Q", folder_hash, file_count, 0, file_record_abs_offset)
    else:
        folder_record = struct.pack("<Q I I", folder_hash, file_count, file_record_abs_offset)

    file_record_block = struct.pack("<B", len(folder_name)) + folder_name
    file_hash = 0xFEDCBA9876543210
    file_record_block += struct.pack("<Q I I", file_hash, stored_size, file_data_offset)
    file_name_block = file_name

    path.write_bytes(header + folder_record + file_record_block + file_name_block + actual_data)
    return path


# ---- ArchiveCache unit tests ----

class TestArchiveCache:
    def test_existing_db_without_meta_table_is_bootstrapped(self, tmp_path):
        cache_dir = tmp_path / "cache"
        cache_dir.mkdir()
        db_path = cache_dir / "archive_cache.sqlite"
        with sqlite3.connect(db_path) as conn:
            conn.execute(
                "CREATE TABLE archives ("
                "path TEXT PRIMARY KEY, mtime_ns INTEGER NOT NULL, size INTEGER NOT NULL, data TEXT NOT NULL)"
            )
            conn.execute(
                "CREATE TABLE unified_routing ("
                "id INTEGER PRIMARY KEY CHECK (id = 1), fingerprint TEXT NOT NULL, data BLOB NOT NULL)"
            )
            conn.commit()

        cache = ArchiveCache(cache_dir)
        assert cache.get_routing("missing-fingerprint") is None

        with sqlite3.connect(db_path) as conn:
            row = conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='meta'"
            ).fetchone()

        assert row is not None
        cache.close()

    def test_get_miss_returns_none(self, tmp_path):
        cache = ArchiveCache(tmp_path)
        ba2 = _build_minimal_ba2(tmp_path / "test.ba2")
        assert cache.get(ba2) is None
        cache.close()

    def test_put_then_get(self, tmp_path):
        cache = ArchiveCache(tmp_path)
        ba2 = _build_minimal_ba2(tmp_path / "test.ba2")
        data = {"archive_type": "GNRL", "files": {"meshes/test.nif": ["g", 24, 0, 8]}}
        cache.put(ba2, data)
        result = cache.get(ba2)
        assert result is not None
        assert result["archive_type"] == "GNRL"
        assert "meshes/test.nif" in result["files"]
        cache.close()

    def test_stale_after_modification(self, tmp_path):
        cache = ArchiveCache(tmp_path)
        ba2_path = tmp_path / "test.ba2"
        _build_minimal_ba2(ba2_path)
        cache.put(ba2_path, {"archive_type": "GNRL", "files": {}})

        # Modify the file (change its content → mtime/size changes)
        time.sleep(0.05)  # ensure mtime changes
        _build_minimal_ba2(ba2_path, file_data=b"DIFFERENT_DATA_LONGER")

        assert cache.get(ba2_path) is None
        cache.close()

    def test_cleanup_removes_stale(self, tmp_path):
        cache = ArchiveCache(tmp_path)
        ba2 = _build_minimal_ba2(tmp_path / "test.ba2")
        resolved = str(ba2.resolve())
        cache.put(ba2, {"files": {}})

        # Cleanup with no valid paths → removes entry
        cache.cleanup(set())
        assert cache.get(ba2) is None
        cache.close()

    def test_cleanup_keeps_valid(self, tmp_path):
        cache = ArchiveCache(tmp_path)
        ba2 = _build_minimal_ba2(tmp_path / "test.ba2")
        data = {"archive_type": "GNRL", "files": {}}
        cache.put(ba2, data)

        cache.cleanup({str(ba2.resolve()).lower()})
        assert cache.get(ba2) is not None
        cache.close()


# ---- BA2File cache round-trip ----

class TestBA2FileCacheRoundTrip:
    def test_to_cache_and_restore(self, tmp_path):
        ba2_path = _build_minimal_ba2(tmp_path / "test.ba2")
        original = BA2File(ba2_path)
        cached_data = original.to_cache()
        original.close()

        restored = BA2File(ba2_path, _cached=cached_data)
        assert restored.file_count == 1
        assert restored.archive_type == "GNRL"
        data = restored.extract("meshes/test.nif")
        assert data == b"NIF_DATA"
        restored.close()


# ---- BSAReader cache round-trip ----

class TestBSAReaderCacheRoundTrip:
    def test_to_cache_and_restore(self, tmp_path):
        bsa_path = _build_minimal_bsa(tmp_path / "test.bsa")
        original = BSAReader(bsa_path)
        cached_data = original.to_cache()
        original.close()

        restored = BSAReader(bsa_path, _cached=cached_data)
        assert restored.file_count == 1
        assert "BSA_v105" == restored.archive_type
        data = restored.extract("meshes/test.nif")
        assert data == b"NIF file content for testing"
        restored.close()


# ---- BA2Manager integration with cache ----

class TestBA2ManagerWithCache:
    @staticmethod
    def _stub_native_runtime(monkeypatch, archive_map):
        def _list_archive(path):
            return list(archive_map[str(Path(path).resolve()).lower()]["files"].keys())

        def _archive_info(path):
            return {"format": "fo4_dx10" if path.lower().endswith(".ba2") else "tes4", "version": 1}

        def _extract_one(path, file_path):
            return archive_map[str(Path(path).resolve()).lower()]["files"].get(file_path)

        monkeypatch.setattr("creation_lib.ba2.ba2_manager.native_runtime.list_archive", _list_archive)
        monkeypatch.setattr("creation_lib.ba2.ba2_manager.native_runtime.archive_info", _archive_info)
        monkeypatch.setattr("creation_lib.ba2.ba2_manager.native_runtime.extract_one", _extract_one)

    def test_scan_populates_cache(self, tmp_path, monkeypatch):
        archive_dir = tmp_path / "data"
        archive_dir.mkdir()
        ba2_path = _build_minimal_ba2(archive_dir / "test.ba2")
        bsa_path = _build_minimal_bsa(archive_dir / "test.bsa", file_path="textures/test.dds", file_data=b"DDS")
        self._stub_native_runtime(
            monkeypatch,
            {
                str(ba2_path.resolve()).lower(): {"files": {"meshes/test.nif": b"NIF_DATA"}},
                str(bsa_path.resolve()).lower(): {"files": {"textures/test.dds": b"DDS"}},
            },
        )

        cache_dir = tmp_path / "cache"
        mgr = BA2Manager(cache_dir=cache_dir)
        mgr.scan_directories([archive_dir])
        assert mgr.archive_count == 2
        assert mgr.total_file_count == 2
        mgr.close_all()

        # Second scan should use cache
        mgr2 = BA2Manager(cache_dir=cache_dir)
        mgr2.scan_directories([archive_dir])
        assert mgr2.archive_count == 2
        assert mgr2.total_file_count == 2

        # Verify extraction still works from cached load
        data = mgr2.find("meshes/test.nif")
        assert data is not None
        mgr2.close_all()

    def test_cache_invalidated_on_change(self, tmp_path, monkeypatch):
        archive_dir = tmp_path / "data"
        archive_dir.mkdir()
        ba2_path = archive_dir / "test.ba2"
        _build_minimal_ba2(ba2_path)
        archive_map = {
            str(ba2_path.resolve()).lower(): {"files": {"meshes/test.nif": b"NIF_DATA"}},
        }
        self._stub_native_runtime(monkeypatch, archive_map)

        cache_dir = tmp_path / "cache"
        mgr = BA2Manager(cache_dir=cache_dir)
        mgr.scan_directories([archive_dir])
        mgr.close_all()

        # Modify archive
        time.sleep(0.05)
        _build_minimal_ba2(ba2_path, file_name="meshes/changed.nif", file_data=b"NEW")
        archive_map[str(ba2_path.resolve()).lower()] = {"files": {"meshes/changed.nif": b"NEW"}}

        mgr2 = BA2Manager(cache_dir=cache_dir)
        mgr2.scan_directories([archive_dir])
        assert mgr2.find("meshes/changed.nif") == b"NEW"
        assert mgr2.find("meshes/test.nif") is None
        mgr2.close_all()

    def test_new_archive_detected(self, tmp_path, monkeypatch):
        archive_dir = tmp_path / "data"
        archive_dir.mkdir()
        first_path = _build_minimal_ba2(archive_dir / "first.ba2")
        archive_map = {
            str(first_path.resolve()).lower(): {"files": {"meshes/test.nif": b"NIF_DATA"}},
        }
        self._stub_native_runtime(monkeypatch, archive_map)

        cache_dir = tmp_path / "cache"
        mgr = BA2Manager(cache_dir=cache_dir)
        mgr.scan_directories([archive_dir])
        assert mgr.archive_count == 1
        mgr.close_all()

        # Add a new archive
        second_path = _build_minimal_ba2(archive_dir / "second.ba2",
                                         file_name="textures/new.dds", file_data=b"DDS")
        archive_map[str(second_path.resolve()).lower()] = {"files": {"textures/new.dds": b"DDS"}}

        mgr2 = BA2Manager(cache_dir=cache_dir)
        mgr2.scan_directories([archive_dir])
        assert mgr2.archive_count == 2
        assert mgr2.find("textures/new.dds") == b"DDS"
        mgr2.close_all()

    def test_use_cache_false_skips_caching(self, tmp_path, monkeypatch):
        archive_dir = tmp_path / "data"
        archive_dir.mkdir()
        ba2_path = _build_minimal_ba2(archive_dir / "test.ba2")
        self._stub_native_runtime(
            monkeypatch,
            {str(ba2_path.resolve()).lower(): {"files": {"meshes/test.nif": b"NIF_DATA"}}},
        )

        mgr = BA2Manager(use_cache=False)
        mgr.scan_directories([archive_dir])
        assert mgr.archive_count == 1
        mgr.close_all()
