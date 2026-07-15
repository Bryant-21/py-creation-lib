"""BSA archive reader for Skyrim SE (and Skyrim LE).

Read-only implementation supporting BSA versions 104 (Skyrim LE) and 105
(Skyrim SE). Parses the folder/file record hierarchy and extracts files
on demand with zlib decompression.

Follows the same API pattern as BA2File in ba2_reader.py.
"""
from __future__ import annotations

import logging
import struct
import zlib
from pathlib import Path

try:
    import lz4.frame as _lz4f
except ImportError:
    _lz4f = None

# LZ4 frame magic number
_LZ4_FRAME_MAGIC = b"\x04\x22\x4D\x18"

_log = logging.getLogger("ba2.bsa_reader")

BSA_MAGIC = b"BSA\x00"  # 0x00415342 LE

# Archive flag bits
_FLAG_HAS_FOLDER_NAMES = 0x001
_FLAG_HAS_FILE_NAMES = 0x002
_FLAG_DEFAULT_COMPRESSED = 0x004
_FLAG_EMBED_FILE_NAMES = 0x100

# Size field flag — bit 30 toggles compression for individual files
_COMPRESSION_TOGGLE_BIT = 1 << 30
_SIZE_MASK = ~_COMPRESSION_TOGGLE_BIT & 0xFFFFFFFF


class _BSAFileEntry:
    """A single file entry in a BSA archive."""
    __slots__ = ("path", "offset", "raw_size", "is_compressed")

    def __init__(self, path: str, offset: int, raw_size: int, is_compressed: bool):
        self.path = path
        self.offset = offset
        self.raw_size = raw_size
        self.is_compressed = is_compressed


class BSAReader:
    """Read-only access to a BSA archive (Skyrim LE/SE).

    Usage::

        reader = BSAReader("path/to/archive.bsa")
        data = reader.extract("meshes/actor/character/defaultmale.nif")
        reader.close()
    """

    def __init__(self, path: str | Path, *, _cached: dict | None = None):
        self._path = Path(path)
        self._f = open(self._path, "rb")
        self._files: dict[str, _BSAFileEntry] = {}
        self._version: int = 0
        self._archive_flags: int = 0
        self._embed_file_names: bool = False
        if _cached is not None:
            self._restore_from_cache(_cached)
        else:
            self._parse()

    @property
    def path(self) -> Path:
        return self._path

    @property
    def file_count(self) -> int:
        return len(self._files)

    @property
    def archive_type(self) -> str:
        return f"BSA_v{self._version}"

    def _parse(self):
        """Parse the BSA header, folder records, file records, and name table."""
        f = self._f
        f.seek(0)

        header = f.read(36)
        if len(header) < 36:
            raise ValueError(f"Not a BSA file (too small): {self._path}")

        magic = header[0:4]
        if magic != BSA_MAGIC:
            raise ValueError(f"Not a BSA file: {self._path} (magic={magic!r})")

        (version, folder_offset, archive_flags,
         folder_count, file_count,
         total_folder_name_len, total_file_name_len,
         file_flags, _pad) = struct.unpack_from("<IIIIIIIHH", header, 4)

        self._version = version
        self._archive_flags = archive_flags
        self._embed_file_names = bool(archive_flags & _FLAG_EMBED_FILE_NAMES)
        default_compressed = bool(archive_flags & _FLAG_DEFAULT_COMPRESSED)
        has_folder_names = bool(archive_flags & _FLAG_HAS_FOLDER_NAMES)
        has_file_names = bool(archive_flags & _FLAG_HAS_FILE_NAMES)

        _log.debug("BSA header: v%d, %d folders, %d files, flags=0x%X",
                    version, folder_count, file_count, archive_flags)

        # --- Read folder records ---
        # v104: 16 bytes each (hash8 + count4 + offset4)
        # v105: 24 bytes each (hash8 + count4 + pad4 + offset8)
        f.seek(36)
        folder_records = []
        for _ in range(folder_count):
            if version >= 105:
                data = f.read(24)
                if len(data) < 24:
                    break
                _hash, count, _pad, offset = struct.unpack_from("<Q I I Q", data)
            else:
                data = f.read(16)
                if len(data) < 16:
                    break
                _hash, count, offset = struct.unpack_from("<Q I I", data)
            folder_records.append((count, offset))

        # --- Read file record blocks (one per folder) ---
        # Each block starts with folder name (if flag set), then file records
        folder_names: list[str] = []
        file_entries_raw: list[list[tuple[int, int]]] = []  # per folder: [(size, offset), ...]

        for count, _offset in folder_records:
            folder_name = ""
            if has_folder_names:
                name_len_byte = f.read(1)
                if name_len_byte:
                    name_len = name_len_byte[0]
                    name_bytes = f.read(name_len)
                    # Strip null terminator
                    folder_name = name_bytes.rstrip(b"\x00").decode("utf-8", errors="replace")

            folder_names.append(folder_name)

            # Read file records for this folder
            entries = []
            for _ in range(count):
                rec = f.read(16)
                if len(rec) < 16:
                    break
                _fhash, size, offset = struct.unpack_from("<Q I I", rec)
                entries.append((size, offset))
            file_entries_raw.append(entries)

        # --- Read file name block ---
        file_names: list[str] = []
        if has_file_names:
            for _ in range(file_count):
                chars = []
                while True:
                    b = f.read(1)
                    if not b or b == b"\x00":
                        break
                    chars.append(b)
                file_names.append(b"".join(chars).decode("utf-8", errors="replace"))

        # --- Build lookup dict ---
        name_idx = 0
        for folder_idx, entries in enumerate(file_entries_raw):
            folder_name = folder_names[folder_idx] if folder_idx < len(folder_names) else ""
            for size_raw, offset in entries:
                file_name = file_names[name_idx] if name_idx < len(file_names) else f"unnamed_{name_idx}"
                name_idx += 1

                # Determine compression
                compression_toggled = bool(size_raw & _COMPRESSION_TOGGLE_BIT)
                actual_size = size_raw & _SIZE_MASK
                is_compressed = default_compressed ^ compression_toggled

                full_path = f"{folder_name}/{file_name}" if folder_name else file_name
                key = full_path.lower().replace("\\", "/")

                self._files[key] = _BSAFileEntry(
                    path=full_path,
                    offset=offset,
                    raw_size=actual_size,
                    is_compressed=is_compressed,
                )

        _log.debug("Opened BSA: %s (v%d, %d files)", self._path.name, version, len(self._files))

    # -- Cache serialization --------------------------------------------------

    def to_cache(self) -> dict:
        """Serialize the file table for persistent caching."""
        files: dict[str, list] = {}
        for name, entry in self._files.items():
            files[name] = [entry.path, entry.offset, entry.raw_size, entry.is_compressed]
        return {
            "version": self._version,
            "archive_flags": self._archive_flags,
            "embed_file_names": self._embed_file_names,
            "files": files,
        }

    def _restore_from_cache(self, data: dict) -> None:
        """Rebuild the file table from cached data (skip header parse)."""
        self._version = data["version"]
        self._archive_flags = data["archive_flags"]
        self._embed_file_names = data.get("embed_file_names", False)
        for name, rec in data["files"].items():
            self._files[name] = _BSAFileEntry(rec[0], rec[1], rec[2], rec[3])
        _log.debug("Opened BSA: %s (v%d, %d files) [cached]",
                   self._path.name, self._version, len(self._files))

    def extract(self, path: str) -> bytes | None:
        """Extract a file from the archive by path. Returns None if not found."""
        key = path.lower().replace("\\", "/")
        entry = self._files.get(key)
        if entry is None:
            return None

        try:
            self._f.seek(entry.offset)
            data_size = entry.raw_size

            # When embed-file-names flag (0x100) is set, each file's data
            # starts with a bstring (1 byte length + string) — skip it
            if self._embed_file_names:
                bstring_len = self._f.read(1)[0]
                self._f.seek(bstring_len, 1)  # skip the embedded name
                data_size -= 1 + bstring_len

            if entry.is_compressed:
                # First 4 bytes are the uncompressed size
                orig_size = struct.unpack("<I", self._f.read(4))[0]
                compressed_data = self._f.read(data_size - 4)
                # Auto-detect: LZ4 frame (Skyrim AE) vs zlib (Skyrim LE/SE)
                if compressed_data[:4] == _LZ4_FRAME_MAGIC:
                    if _lz4f is None:
                        _log.warning("LZ4 compressed BSA but lz4 not installed")
                        return None
                    return _lz4f.decompress(compressed_data)
                return zlib.decompress(compressed_data)
            else:
                return self._f.read(data_size)
        except Exception as e:
            _log.debug("BSA extract failed for %s: %s", path, e)
            return None

    def list_files(self, prefix: str = "", suffix: str = "") -> list[str]:
        """List all file paths, optionally filtered by prefix/suffix."""
        prefix_lower = prefix.lower().replace("\\", "/")
        suffix_lower = suffix.lower()
        result = []
        for key in self._files:
            if prefix_lower and not key.startswith(prefix_lower):
                continue
            if suffix_lower and not key.endswith(suffix_lower):
                continue
            result.append(key)
        return result

    def contains(self, path: str) -> bool:
        """Check if a file exists in the archive."""
        key = path.lower().replace("\\", "/")
        return key in self._files

    def close(self):
        """Close the archive file handle."""
        if self._f and not self._f.closed:
            self._f.close()

    def __del__(self):
        self.close()

    def __repr__(self) -> str:
        return f"BSAReader({self._path.name!r}, v{self._version}, {self.file_count} files)"
