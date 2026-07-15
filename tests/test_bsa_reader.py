import struct
import zlib

import pytest

from creation_lib.ba2.bsa_reader import BSAReader

# BSA constants for building test fixtures
BSA_MAGIC = b"BSA\x00"  # 0x00415342 LE


def _build_test_bsa(tmp_path, version=105, compressed=False):
    """Build a minimal valid BSA file for testing.

    BSA v104/105 layout:
      Header (36 bytes):
        magic(4) + version(4) + offset(4) + archiveFlags(4) +
        folderCount(4) + fileCount(4) + totalFolderNameLen(4) +
        totalFileNameLen(4) + fileFlags(2) + padding(2)
      Folder records (folderCount * 16 bytes each):
        nameHash(8) + count(4) + offset(4)
        For v105: nameHash(8) + count(4) + padding(4) + offset(8)
      File record blocks (per folder):
        folderNameLen(1) + folderName(len) then per file: nameHash(8) + size(4) + offset(4)
      File name block:
        null-terminated strings for each file
      File data block:
        raw or compressed file data
    """
    bsa_path = tmp_path / "test.bsa"

    folder_name = b"meshes\x00"
    file_name = b"test.nif\x00"
    file_data = b"NIF file content for testing"

    if compressed:
        compressed_data = zlib.compress(file_data)
    else:
        compressed_data = file_data

    # Archive flags: 0x003 = has folder names + has file names
    # If compressed: flag 0x004 (default compressed)
    archive_flags = 0x003
    if compressed:
        archive_flags |= 0x004

    folder_count = 1
    file_count = 1
    total_folder_name_len = len(folder_name)  # includes null
    total_file_name_len = len(file_name)  # includes null

    # Compute offsets
    header_size = 36

    if version >= 105:
        folder_record_size = 24  # v105: hash(8) + count(4) + pad(4) + offset(8)
    else:
        folder_record_size = 16  # v104: hash(8) + count(4) + offset(4)

    folder_records_offset = header_size
    # File record block follows folder records
    file_record_block_offset = folder_records_offset + folder_record_size * folder_count

    # File record block: folderNameLen(1) + folderName + per-file: hash(8) + size(4) + offset(4)
    file_record_block_size = 1 + len(folder_name) + 16 * file_count

    # File name block follows file record blocks
    file_name_block_offset = file_record_block_offset + file_record_block_size

    # File data follows file name block
    file_data_offset = file_name_block_offset + total_file_name_len

    # When default_compressed flag is set, files are compressed by default.
    # Bit 30 in the size field TOGGLES compression (off if default is on).
    # So for a compressed file with default_compressed=True, do NOT set bit 30.
    if compressed:
        stored_size = len(compressed_data) + 4  # +4 for the uncompressed size prefix
        # Don't set bit 30 — default_compressed already means "compressed"
        actual_data = struct.pack("<I", len(file_data)) + compressed_data
    else:
        stored_size = len(file_data)
        actual_data = file_data

    # --- Build header ---
    header = struct.pack(
        "<4s I I I I I I I HH",
        BSA_MAGIC,
        version,
        header_size,  # offset to folder records (always 36)
        archive_flags,
        folder_count,
        file_count,
        total_folder_name_len,
        total_file_name_len,
        0,  # file flags
        0,  # padding
    )

    # --- Build folder record ---
    folder_hash = 0x0123456789ABCDEF  # dummy hash
    # offset points to start of file record block (including folder name prefix)
    # The offset is relative to total_file_name_len... per BSA spec it's
    # offset to the file record block for this folder (absolute from file start)
    # It includes the total_file_name_len added to the offset
    file_record_abs_offset = file_record_block_offset + total_file_name_len

    if version >= 105:
        folder_record = struct.pack("<Q I I Q", folder_hash, file_count, 0, file_record_abs_offset)
    else:
        folder_record = struct.pack("<Q I I", folder_hash, file_count, file_record_abs_offset)

    # --- Build file record block ---
    # folder name length byte (includes null terminator)
    file_record_block = struct.pack("<B", len(folder_name))
    file_record_block += folder_name

    # file record: hash(8) + size(4) + offset(4)
    file_hash = 0xFEDCBA9876543210
    file_record_block += struct.pack("<Q I I", file_hash, stored_size, file_data_offset)

    # --- Build file name block ---
    file_name_block = file_name

    # --- Assemble ---
    bsa_data = header + folder_record + file_record_block + file_name_block + actual_data
    bsa_path.write_bytes(bsa_data)
    return bsa_path


class TestBSAReader:
    def test_parse_header_rejects_non_bsa(self, tmp_path):
        bad = tmp_path / "bad.bsa"
        bad.write_bytes(b"\x00" * 100)
        with pytest.raises(ValueError, match="Not a BSA"):
            BSAReader(bad)

    def test_parse_header_rejects_truncated_file(self, tmp_path):
        bad = tmp_path / "short.bsa"
        bad.write_bytes(BSA_MAGIC + b"\x00" * 4)
        with pytest.raises(ValueError):
            BSAReader(bad)

    def test_read_v105_uncompressed(self, tmp_path):
        bsa_path = _build_test_bsa(tmp_path, version=105, compressed=False)
        reader = BSAReader(bsa_path)
        files = reader.list_files()
        assert len(files) == 1
        assert files[0] == "meshes/test.nif"

        data = reader.extract("meshes/test.nif")
        assert data == b"NIF file content for testing"
        reader.close()

    def test_read_v105_compressed(self, tmp_path):
        bsa_path = _build_test_bsa(tmp_path, version=105, compressed=True)
        reader = BSAReader(bsa_path)
        files = reader.list_files()
        assert len(files) == 1

        data = reader.extract("meshes/test.nif")
        assert data == b"NIF file content for testing"
        reader.close()

    def test_read_v104_uncompressed(self, tmp_path):
        bsa_path = _build_test_bsa(tmp_path, version=104, compressed=False)
        reader = BSAReader(bsa_path)
        files = reader.list_files()
        assert len(files) == 1
        data = reader.extract("meshes/test.nif")
        assert data == b"NIF file content for testing"
        reader.close()

    def test_extract_returns_none_for_missing(self, tmp_path):
        bsa_path = _build_test_bsa(tmp_path, version=105)
        reader = BSAReader(bsa_path)
        result = reader.extract("nonexistent/path.dds")
        assert result is None
        reader.close()

    def test_extract_case_insensitive(self, tmp_path):
        bsa_path = _build_test_bsa(tmp_path, version=105)
        reader = BSAReader(bsa_path)
        data = reader.extract("Meshes/Test.nif")
        assert data == b"NIF file content for testing"
        reader.close()

    def test_contains(self, tmp_path):
        bsa_path = _build_test_bsa(tmp_path, version=105)
        reader = BSAReader(bsa_path)
        assert reader.contains("meshes/test.nif")
        assert not reader.contains("missing.dds")
        reader.close()

    def test_list_files_with_filter(self, tmp_path):
        bsa_path = _build_test_bsa(tmp_path, version=105)
        reader = BSAReader(bsa_path)
        assert len(reader.list_files(suffix=".nif")) == 1
        assert len(reader.list_files(suffix=".dds")) == 0
        assert len(reader.list_files(prefix="meshes")) == 1
        assert len(reader.list_files(prefix="textures")) == 0
        reader.close()
