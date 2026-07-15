"""BA2 archive reader for Fallout 4.

Ported from NifSkope's ba2file.cpp. Supports GNRL and DX10 archive types.
Reads file tables on init, extracts individual files on-demand with zlib.
"""
from __future__ import annotations

import logging
import struct
import zlib
try:
    import lz4.block as _lz4
except ImportError:
    _lz4 = None
from pathlib import Path

from .dds_header import build_dds_header

_log = logging.getLogger("ba2.reader")

# BA2 header constants
BA2_MAGIC = b"BTDX"
BA2_TYPE_GNRL = 0x4C524E47  # "GNRL" as uint32 LE
BA2_TYPE_DX10 = 0x30315844  # "DX10" as uint32 LE

# Struct sizes
GNRL_RECORD_SIZE = 36
DX10_HEADER_SIZE = 24
DX10_CHUNK_SIZE = 24


class _GNRLRecord:
    """General archive file record."""
    __slots__ = ("offset", "packed_len", "unpacked_len")

    def __init__(self, offset: int, packed_len: int, unpacked_len: int):
        self.offset = offset
        self.packed_len = packed_len
        self.unpacked_len = unpacked_len


class _DX10Record:
    """DX10 texture archive file record."""
    __slots__ = ("width", "height", "num_mips", "dxgi_format", "flags", "chunks")

    def __init__(self, width: int, height: int, num_mips: int,
                 dxgi_format: int, flags: int,
                 chunks: list[tuple[int, int, int, int, int]]):
        self.width = width
        self.height = height
        self.num_mips = num_mips
        self.dxgi_format = dxgi_format
        self.flags = flags
        # chunks: list of (offset, packed_len, unpacked_len, start_mip, end_mip)
        self.chunks = chunks


class BA2File:
    """Read-only access to a BA2 archive.

    Usage::

        ba2 = BA2File("path/to/archive.ba2")
        data = ba2.extract("textures/weapons/gun/diffuse.dds")
        ba2.close()
    """

    def __init__(self, path: str | Path, *, _cached: dict | None = None):
        self._path = Path(path)
        self._f = open(self._path, "rb")
        self._files: dict[str, _GNRLRecord | _DX10Record] = {}
        self._archive_type: str = ""
        self._use_lz4 = False  # True for Starfield DX10 v3 archives
        if _cached is not None:
            self._restore_from_cache(_cached)
        else:
            self._parse_header()

    @property
    def path(self) -> Path:
        return self._path

    @property
    def file_count(self) -> int:
        return len(self._files)

    @property
    def archive_type(self) -> str:
        return self._archive_type

    def _parse_header(self):
        """Parse BA2 header, file records, and name table."""
        f = self._f
        f.seek(0)

        # Header: magic(4) + version(4) + type(4) + numFiles(4) + nameTableOffset(8)
        header = f.read(24)
        if len(header) < 24:
            raise ValueError(f"BA2 too small: {self._path}")

        magic = header[0:4]
        if magic != BA2_MAGIC:
            raise ValueError(f"Not a BA2 file: {self._path} (magic={magic!r})")

        version, arc_type, num_files = struct.unpack_from("<III", header, 4)
        name_table_offset = struct.unpack_from("<Q", header, 16)[0]
        self._version = version

        # BA2 version → header size mapping (per NifSkope ba2file.cpp):
        #   v1/v7/v8: 24-byte header (FO4, FO4 Next-Gen/CC)
        #   v2/v3:    32-byte header (Starfield — 8 extra bytes)
        # DX10 textures: v3 has an additional 4 bytes (header=36)
        _KNOWN_VERSIONS = {1, 2, 3, 7, 8}
        if version not in _KNOWN_VERSIONS:
            _log.warning("BA2 version %d may not be supported: %s — attempting parse anyway",
                         version, self._path.name)

        # Starfield v2/v3 has 8 extra header bytes before file records
        if version in (2, 3):
            extra = f.read(8)  # skip unknown v2/v3 header extension

        if arc_type == BA2_TYPE_GNRL:
            self._archive_type = "GNRL"
            records = self._parse_gnrl_records(num_files)
        elif arc_type == BA2_TYPE_DX10:
            self._archive_type = "DX10"
            if version == 3:
                f.read(4)  # DX10 v3 has 4 more header bytes (total 36)
                self._use_lz4 = True  # Starfield DX10 v3 uses LZ4
            records = self._parse_dx10_records(num_files)
        else:
            raise ValueError(f"Unknown BA2 type: 0x{arc_type:08X} in {self._path}")

        # Read name table
        names = self._parse_name_table(name_table_offset, num_files)

        # Build lookup dict
        for i, name in enumerate(names):
            if i < len(records):
                # Normalize: lowercase, forward slashes
                key = name.lower().replace("\\", "/")
                self._files[key] = records[i]

        _log.debug("Opened BA2: %s (%s, %d files)",
                   self._path.name, self._archive_type, len(self._files))

    def _parse_gnrl_records(self, num_files: int) -> list[_GNRLRecord]:
        """Parse GNRL file records starting at offset 24."""
        records = []
        data = self._f.read(GNRL_RECORD_SIZE * num_files)
        for i in range(num_files):
            off = i * GNRL_RECORD_SIZE
            # nameHash(4), ext(4), dirHash(4), flags(4), offset(8), packedLen(4), unpackedLen(4), pad(4)
            _, _, _, _, file_offset, packed_len, unpacked_len, _ = struct.unpack_from(
                "<IIII Q III", data, off
            )
            records.append(_GNRLRecord(file_offset, packed_len, unpacked_len))
        return records

    def _parse_dx10_records(self, num_files: int) -> list[_DX10Record]:
        """Parse DX10 texture file records starting at offset 24."""
        f = self._f
        records = []
        for _ in range(num_files):
            # Header: nameHash(4), ext(4), dirHash(4), unk8(1), numChunks(1),
            #          chunkHeaderSize(2), height(2), width(2), numMips(1),
            #          dxgiFormat(1), flags(2)
            hdr = f.read(DX10_HEADER_SIZE)
            if len(hdr) < DX10_HEADER_SIZE:
                break
            (_, _, _, unk8, num_chunks, chunk_hdr_size,
             height, width, num_mips, dxgi_format, flags) = struct.unpack_from(
                "<III B B H HH B B H", hdr, 0
            )
            # Read chunks
            chunks = []
            for _ in range(num_chunks):
                chunk = f.read(DX10_CHUNK_SIZE)
                if len(chunk) < DX10_CHUNK_SIZE:
                    break
                # offset(8), packedLen(4), unpackedLen(4), startMip(2), endMip(2), pad(4)
                c_offset, c_packed, c_unpacked, c_start_mip, c_end_mip, _ = struct.unpack_from(
                    "<Q II HH I", chunk, 0
                )
                chunks.append((c_offset, c_packed, c_unpacked, c_start_mip, c_end_mip))
            records.append(_DX10Record(width, height, num_mips, dxgi_format, flags, chunks))
        return records

    def _parse_name_table(self, offset: int, num_files: int) -> list[str]:
        """Parse the name table at the given offset."""
        if offset == 0:
            return [f"unnamed_{i}" for i in range(num_files)]

        f = self._f
        f.seek(offset)
        names = []
        for _ in range(num_files):
            len_data = f.read(2)
            if len(len_data) < 2:
                break
            name_len = struct.unpack_from("<H", len_data, 0)[0]
            name = f.read(name_len)
            # Strip null terminator if present
            if name and name[-1:] == b"\x00":
                name = name[:-1]
            names.append(name.decode("utf-8", errors="replace"))
        return names

    # -- Cache serialization --------------------------------------------------

    def to_cache(self) -> dict:
        """Serialize the file table for persistent caching."""
        files: dict[str, list] = {}
        for name, rec in self._files.items():
            if isinstance(rec, _GNRLRecord):
                files[name] = ["g", rec.offset, rec.packed_len, rec.unpacked_len]
            else:
                files[name] = [
                    "d", rec.width, rec.height, rec.num_mips,
                    rec.dxgi_format, rec.flags, rec.chunks,
                ]
        return {
            "archive_type": self._archive_type,
            "use_lz4": self._use_lz4,
            "files": files,
        }

    def _restore_from_cache(self, data: dict) -> None:
        """Rebuild the file table from cached data (skip header parse)."""
        self._archive_type = data["archive_type"]
        self._use_lz4 = data.get("use_lz4", False)
        for name, rec in data["files"].items():
            if rec[0] == "g":
                self._files[name] = _GNRLRecord(rec[1], rec[2], rec[3])
            else:
                chunks = [tuple(c) for c in rec[6]]
                self._files[name] = _DX10Record(
                    rec[1], rec[2], rec[3], rec[4], rec[5], chunks,
                )
        _log.debug("Opened BA2: %s (%s, %d files) [cached]",
                   self._path.name, self._archive_type, len(self._files))

    def extract(self, path: str) -> bytes | None:
        """Extract a file from the archive by path.

        Returns complete file bytes (DDS with header for DX10 textures),
        or None if not found.
        """
        key = path.lower().replace("\\", "/")
        record = self._files.get(key)
        if record is None:
            return None

        if isinstance(record, _GNRLRecord):
            return self._extract_gnrl(record)
        elif isinstance(record, _DX10Record):
            return self._extract_dx10(record)
        return None

    def _extract_gnrl(self, rec: _GNRLRecord) -> bytes | None:
        """Extract a GNRL file record."""
        try:
            self._f.seek(rec.offset)
            if rec.packed_len == 0:
                # Uncompressed
                return self._f.read(rec.unpacked_len)
            raw = self._f.read(rec.packed_len)
            return zlib.decompress(raw)
        except Exception as e:
            _log.debug("GNRL extract failed: %s", e)
            return None

    def _extract_dx10(self, rec: _DX10Record) -> bytes | None:
        """Extract a DX10 texture, building a complete DDS file."""
        try:
            # FO4 "cubemaps" are simple 2D env maps, not true 6-face cubemaps.
            # Never set cubemap flags — the pixel data is a flat 2D texture.
            header = build_dds_header(
                rec.dxgi_format, rec.width, rec.height,
                rec.num_mips, is_cubemap=False,
            )
            # Decompress and concatenate all chunks
            pixel_data = bytearray()
            for offset, packed_len, unpacked_len, _, _ in rec.chunks:
                self._f.seek(offset)
                if packed_len == 0:
                    chunk_data = self._f.read(unpacked_len)
                elif self._use_lz4:
                    if _lz4 is None:
                        _log.warning("lz4 package not installed — cannot decompress Starfield textures")
                        return None
                    raw = self._f.read(packed_len)
                    chunk_data = _lz4.decompress(raw, uncompressed_size=unpacked_len)
                else:
                    raw = self._f.read(packed_len)
                    chunk_data = zlib.decompress(raw)
                pixel_data.extend(chunk_data)
            return bytes(header) + bytes(pixel_data)
        except Exception as e:
            _log.debug("DX10 extract failed: %s", e)
            return None

    def list_files(self, prefix: str = "", suffix: str = "") -> list[str]:
        """List all file paths in the archive, optionally filtered.

        Args:
            prefix: Only include paths starting with this (case-insensitive).
            suffix: Only include paths ending with this (case-insensitive).

        Returns:
            List of normalized file paths (lowercase, forward slashes).
        """
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
        return f"BA2File({self._path.name!r}, {self._archive_type}, {self.file_count} files)"
