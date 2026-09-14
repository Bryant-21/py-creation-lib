"""BSReflection stream reader, used only by materials_cdb.py.

Port of libfo76utils ``bsrefl.cpp``/``bsrefl.hpp`` that keeps the C++ control
flow 1:1 for side-by-side reading. ``StringType`` holds the in-band meta-type ids
CDB records use (String_None=0, String_String=1, ..., and the BSMaterial_* class
ids). The type/class registry built from TYPE/CLAS chunks lives in
``materials_cdb.py``.
"""
from __future__ import annotations

import struct
from dataclasses import dataclass
from enum import IntEnum
from typing import Optional

from creation_lib.material_tools._bsrefl_stringtable import STRING_TABLE


# ---------------------------------------------------------------------------
# Enums
# ---------------------------------------------------------------------------

class ChunkType(IntEnum):
    """Chunk kinds that can appear in a BETH reflection stream.

    Values are little-endian u32 reads of the four-char-code in cpp, i.e.
    ``ChunkType_STRT = 0x54525453`` <=> b"STRT".
    """

    NONE = 0
    BETH = 0x48544542  # "BETH"
    STRT = 0x54525453  # "STRT"
    TYPE = 0x45505954  # "TYPE"
    CLAS = 0x53414C43  # "CLAS"
    LIST = 0x5453494C  # "LIST"
    MAPC = 0x4350414D  # "MAPC"
    OBJT = 0x544A424F  # "OBJT"
    DIFF = 0x46464944  # "DIFF"
    USER = 0x52455355  # "USER"
    USRD = 0x44525355  # "USRD"


class StringType(IntEnum):
    """Subset of the master string-table indices used as meta-type ids.

    These are the ``String_*`` enum values in bsrefl.hpp. We only mirror the
    primitives plus a handful of anchors here; the full list of 1156 entries
    lives in ``STRING_TABLE``. Lookups of BSMaterial_* class ids should go
    through ``find_master_string`` rather than through enum members.
    """

    NONE = 0
    STRING = 1
    LIST = 2
    MAP = 3
    REF = 4
    INT8 = 7
    UINT8 = 8
    INT16 = 9
    UINT16 = 10
    INT32 = 11
    UINT32 = 12
    INT64 = 13
    UINT64 = 14
    BOOL = 15
    FLOAT = 16
    DOUBLE = 17
    UNKNOWN = 18
    BS_RESOURCE_ID = 228
    XMFLOAT2 = 1096
    XMFLOAT3 = 1097
    XMFLOAT4 = 1098


# ---------------------------------------------------------------------------
# Master string table binary search
# ---------------------------------------------------------------------------

def find_master_string(s: str) -> int:
    """Binary search for ``s`` in ``STRING_TABLE``.

    Mirrors ``BSReflStream::findString(const char *s)`` in bsrefl.cpp, which
    starts the search at index 19 (skipping the primitive type names) and
    returns -1 on miss.
    """
    n0 = 19
    n2 = len(STRING_TABLE)
    while n2 > n0 + 1:
        n1 = (n0 + n2) >> 1
        if s < STRING_TABLE[n1]:
            n2 = n1
        else:
            n0 = n1
    if n2 > n0 and STRING_TABLE[n0] == s:
        return n0
    return -1


# ---------------------------------------------------------------------------
# Per-chunk reader
# ---------------------------------------------------------------------------

@dataclass
class Chunk:
    """A typed byte-range inside a BETH stream. Methods return ``(ok, value)``.

    Mirrors ``BSReflStream::Chunk`` in bsrefl.hpp. The ``ok`` flag is False
    when the underlying buffer has been exhausted (cpp returns bool and sets
    filePos = fileBufSize on short read).
    """

    data: bytes
    pos: int = 0

    @property
    def size(self) -> int:
        return len(self.data)

    # -- primitive helpers -------------------------------------------------

    def read_u8(self) -> tuple[bool, int]:
        if self.pos >= len(self.data):
            return False, 0
        v = self.data[self.pos]
        self.pos += 1
        return True, v

    def read_bool(self) -> tuple[bool, bool]:
        ok, v = self.read_u8()
        return ok, bool(v)

    def read_u16(self) -> tuple[bool, int]:
        if self.pos + 2 > len(self.data):
            self.pos = len(self.data)
            return False, 0
        (v,) = struct.unpack_from("<H", self.data, self.pos)
        self.pos += 2
        return True, v

    def read_u32(self) -> tuple[bool, int]:
        if self.pos + 4 > len(self.data):
            self.pos = len(self.data)
            return False, 0
        (v,) = struct.unpack_from("<I", self.data, self.pos)
        self.pos += 4
        return True, v

    def read_float(self) -> tuple[bool, float]:
        if self.pos + 4 > len(self.data):
            self.pos = len(self.data)
            return False, 0.0
        (raw,) = struct.unpack_from("<I", self.data, self.pos)
        self.pos += 4
        # cpp flushes denormals (and -0 within the same test) to zero:
        #   if (!((tmp + 0x00800000U) & 0x7F000000U)) tmp = 0U;
        # Mirror that bit-twiddling 1:1.
        if not (((raw + 0x00800000) & 0x7F000000) & 0xFFFFFFFF):
            raw = 0
        (f,) = struct.unpack("<f", struct.pack("<I", raw & 0xFFFFFFFF))
        return True, f

    def read_float_0_to_1(self) -> tuple[bool, float]:
        ok, f = self.read_float()
        if not ok:
            return False, 0.0
        if f < 0.0:
            f = 0.0
        elif f > 1.0:
            f = 1.0
        return True, f

    def read_string(self) -> tuple[bool, str]:
        ok, length = self.read_u16()
        if not ok:
            return False, ""
        if self.pos + length > len(self.data):
            self.pos = len(self.data)
            return False, ""
        raw = self.data[self.pos : self.pos + length]
        self.pos += length
        # cpp trims trailing NULs implicitly via FileBuffer::readString.
        return True, raw.rstrip(b"\x00").decode("utf-8", errors="replace")

    def read_i32(self) -> tuple[bool, int]:
        if self.pos + 4 > len(self.data):
            self.pos = len(self.data)
            return False, 0
        (v,) = struct.unpack_from("<i", self.data, self.pos)
        self.pos += 4
        return True, v

    def read_u64(self) -> tuple[bool, int]:
        if self.pos + 8 > len(self.data):
            self.pos = len(self.data)
            return False, 0
        (v,) = struct.unpack_from("<Q", self.data, self.pos)
        self.pos += 8
        return True, v

    def read_i64(self) -> tuple[bool, int]:
        if self.pos + 8 > len(self.data):
            self.pos = len(self.data)
            return False, 0
        (v,) = struct.unpack_from("<q", self.data, self.pos)
        self.pos += 8
        return True, v

    def read_double(self) -> tuple[bool, float]:
        ok, raw = self.read_u64()
        if not ok:
            return False, 0.0
        (d,) = struct.unpack("<d", struct.pack("<Q", raw))
        return True, d

    def get_field_number(self, n: int, n_max: int, is_diff: bool) -> tuple[bool, int]:
        """Advance to the next field number in this chunk body.

        Mirrors ``BSReflStream::Chunk::getFieldNumber`` (bsrefl.hpp:191):
          * OBJT (not DIFF): n is incremented; returns (True, n) while n <= n_max.
          * DIFF: reads a u16 from the body as the next field index.
        Returns ``(False, n)`` when no more fields are available.
        """
        if not is_diff:
            n += 1
            return (n <= n_max, n)
        ok, v = self.read_u16()
        if not ok:
            return False, n
        n = v
        # cpp: if (int16_t(n) <= int16_t(nMax)): return (int16_t(n) >= 0)
        sn = v if v < 0x8000 else v - 0x10000
        smax = n_max if n_max < 0x8000 else n_max - 0x10000
        if sn <= smax:
            return (sn >= 0, n)
        # Out-of-range: consume rest of chunk.
        self.pos = len(self.data)
        return False, n


# ---------------------------------------------------------------------------
# Stream
# ---------------------------------------------------------------------------

# "BETH" tag plus its u32 chunk size (8: the version and chunk-count u32s),
# read as one little-endian u64 (cpp 0x0000000848544542ULL).
_BETH_MAGIC: int = 0x0000000848544542


class BSReflStream:
    """Parser for a BETH-wrapped reflection stream.

    Usage::

        stream = BSReflStream(data)
        while True:
            ctype, chunk = stream.read_chunk()
            if ctype == 0:
                break
            ...  # dispatch on ChunkType(ctype)

    Mirrors ``BSReflStream`` in bsrefl.cpp/hpp. The constructor performs the
    header + STRT validation eagerly via ``_read_string_table``.
    """

    def __init__(self, data: bytes) -> None:
        self._data: bytes = data
        self._pos: int = 0
        self.chunks_remaining: int = 0
        # Maps strtOffs (offset into the STRT body, relative to file start
        # minus 24) -> master STRING_TABLE index, or -1 for unrecognized
        # strings. Mirrors ``stringMap`` in cpp.
        self._string_map: dict[int, int] = {}
        self._read_string_table()

    # -- header + STRT -----------------------------------------------------

    def _read_u32(self) -> int:
        if self._pos + 4 > len(self._data):
            raise ValueError("unexpected end of reflection stream")
        (v,) = struct.unpack_from("<I", self._data, self._pos)
        self._pos += 4
        return v

    def _read_u64(self) -> int:
        if self._pos + 8 > len(self._data):
            raise ValueError("unexpected end of reflection stream")
        (v,) = struct.unpack_from("<Q", self._data, self._pos)
        self._pos += 8
        return v

    def _read_string_table(self) -> None:
        # Minimum header: 8 (magic) + 4 (version) + 4 (chunks) + 4 (STRT type)
        # + 4 (STRT length) = 24 bytes; the cpp code checks `fileBufSize < 24`
        # up front.
        if len(self._data) < 24 or self._read_u64() != _BETH_MAGIC:
            raise ValueError("invalid reflection stream header")
        version = self._read_u32()
        if version != 4:
            raise ValueError("unsupported reflection stream version")
        self.chunks_remaining = self._read_u32()
        if self.chunks_remaining < 2 or self._read_u32() != ChunkType.STRT.value:
            raise ValueError("missing string table in reflection stream")
        self.chunks_remaining -= 2

        # STRT body: u32 total-bytes followed by that many bytes of
        # null-terminated strings.
        n = self._read_u32()
        if self._pos + n > len(self._data):
            raise ValueError("unexpected end of reflection stream")
        end = self._pos + n
        # Walk the bytes looking for null-terminators; each substring gets
        # its strtOffs (entry start - 24) recorded in stringMap.
        while self._pos < end:
            entry_start = self._pos - 24
            str_start = self._pos
            while self._pos < end and self._data[self._pos] != 0:
                self._pos += 1
            raw = self._data[str_start : self._pos]
            if self._pos >= end:
                raise ValueError(
                    "string table is not terminated in reflection stream"
                )
            # Advance past the NUL terminator.
            self._pos += 1
            s = raw.decode("utf-8", errors="replace")
            self._string_map[entry_start] = find_master_string(s)

    # -- chunk iteration ---------------------------------------------------

    def read_chunk(self) -> tuple[int, Optional[Chunk]]:
        """Read the next [type u32][size u32][body] chunk.

        Returns ``(chunk_type, Chunk)`` on success or ``(0, None)`` at end of
        stream. Mirrors ``BSReflStream::readChunk`` in bsrefl.hpp.
        """
        if self.chunks_remaining == 0:
            return 0, None
        self.chunks_remaining -= 1
        if self._pos + 8 > len(self._data):
            raise ValueError("unexpected end of reflection stream")
        chunk_type = self._read_u32()
        chunk_size = self._read_u32()
        if self._pos + chunk_size > len(self._data):
            raise ValueError("unexpected end of reflection stream")
        body = self._data[self._pos : self._pos + chunk_size]
        self._pos += chunk_size
        return chunk_type, Chunk(data=body, pos=0)

    # -- string resolution -------------------------------------------------

    def find_string_by_offset(self, strt_offs: int) -> int:
        """Resolve a strtOffs (as stored inline in CDB fields) to a master
        ``STRING_TABLE`` index.

        Mirrors ``BSReflStream::findString(unsigned int strtOffs) const``.
        Values >= 0xFFFFFF01 are treated as in-line meta-type ids: cpp
        computes ``min(strtOffs - 0xFFFFFF01, 18)``, which maps them onto the
        String_None .. String_Unknown primitive slots.
        """
        if strt_offs in self._string_map:
            n = self._string_map[strt_offs]
            if n >= 0:
                return n
        # Fall-through: inline primitive type id.
        # cpp: unsigned int n = strtOffs - 0xFFFFFF01U;
        #      return std::min(n, 18U);
        n = (strt_offs - 0xFFFFFF01) & 0xFFFFFFFF
        return min(n, 18)

    def get_string(self, strt_offs: int) -> str:
        """Return the master string for a strtOffs (or ``STRING_TABLE[18]``
        = "<unknown>" if the offset does not resolve)."""
        idx = self.find_string_by_offset(strt_offs)
        if 0 <= idx < len(STRING_TABLE):
            return STRING_TABLE[idx]
        return STRING_TABLE[18]

    @property
    def position(self) -> int:
        return self._pos


# ---------------------------------------------------------------------------
# One-shot reader
# ---------------------------------------------------------------------------

def read_bsrefl(data: bytes, offset: int = 0) -> tuple[BSReflStream, int]:
    """Parse ``data[offset:]`` and drain every chunk; return ``(stream, new_offset)``.

    ``new_offset`` is the absolute position after the last chunk, where a caller
    would resume if the stream were embedded in a larger container.
    """
    if offset:
        data = data[offset:]
    stream = BSReflStream(data)
    # Drain remaining chunks to advance position to end-of-stream.
    while True:
        ctype, _ = stream.read_chunk()
        if ctype == 0:
            break
    return stream, offset + stream.position
