"""Tests for the BSReflection stream reader port.

Reference: refs/fo76texconv/Texture Converter 0.8 - Source/py_creation_lib/python/creation_lib/libfo76utils/
src/bsrefl.cpp + bsrefl.hpp.
"""
from __future__ import annotations

import struct

import pytest

from creation_lib.material_tools._bsrefl import (
    BSReflStream,
    ChunkType,
    StringType,
    find_master_string,
    read_bsrefl,
)
from creation_lib.material_tools._bsrefl_stringtable import STRING_TABLE


# ---------------------------------------------------------------------------
# Master string table sanity
# ---------------------------------------------------------------------------

def test_master_string_table_has_1156_entries():
    assert len(STRING_TABLE) == 1156


def test_master_string_table_known_anchors():
    # Anchored against enums in bsrefl.hpp.
    assert STRING_TABLE[0] == "null"
    assert STRING_TABLE[1] == "String"
    assert STRING_TABLE[4] == "<ref>"
    assert STRING_TABLE[7] == "int8_t"
    assert STRING_TABLE[12] == "uint32_t"
    assert STRING_TABLE[16] == "float"
    assert STRING_TABLE[18] == "<unknown>"
    assert STRING_TABLE[228] == "BSResource::ID"


def test_find_master_string_binary_search():
    # findString() in cpp starts the search at index 19 (after the primitives)
    # and uses binary search; primitives at indices 0-18 are NOT locatable via
    # this lookup. Verify with a known late-table entry.
    idx = find_master_string("BSResource::ID")
    assert idx == 228
    assert find_master_string("this::symbol::does::not::exist") == -1


# ---------------------------------------------------------------------------
# Hand-crafted BETH stream
# ---------------------------------------------------------------------------

def _beth_magic() -> bytes:
    # 8 bytes: "BETH\0\0\0\0" interpreted as little-endian u64 0x0000000848544542.
    return struct.pack("<Q", 0x0000000848544542)


def _build_minimal_stream(extra_chunks: list[tuple[int, bytes]]) -> bytes:
    """Build a BETH/STRT stream with an empty string table plus extra chunks.

    extra_chunks is a list of (chunk_type, chunk_body_bytes). The function
    prepends the mandatory BETH header + STRT chunk.
    """
    buf = bytearray()
    buf += _beth_magic()
    buf += struct.pack("<I", 4)  # version
    # chunks_remaining: 1 (STRT) + len(extra_chunks); header parser subtracts 2,
    # but in fact cpp reads [chunksRemaining], consumes STRT, then stores
    # chunksRemaining - 2. So to end with len(extra) chunks visible to the
    # user, chunks_remaining in the header needs to be len(extra) + 2.
    buf += struct.pack("<I", len(extra_chunks) + 2)
    # First chunk type: STRT
    buf += struct.pack("<I", ChunkType.STRT.value)
    # STRT body: n (u32) = 0 -> no strings in the string table.
    buf += struct.pack("<I", 0)
    # Remaining chunks: [type u32][size u32][body]
    for ctype, body in extra_chunks:
        buf += struct.pack("<II", ctype, len(body))
        buf += body
    return bytes(buf)


def test_stream_reads_beth_header_and_empty_strt():
    data = _build_minimal_stream([])
    stream = BSReflStream(data)
    assert stream.chunks_remaining == 0
    # No more chunks to read
    chunk_type, chunk = stream.read_chunk()
    assert chunk_type == 0
    assert chunk is None


def test_stream_rejects_bad_magic():
    bad = b"XXXX" + b"\x00" * 20
    with pytest.raises(ValueError, match="invalid reflection stream header"):
        BSReflStream(bad)


def test_stream_rejects_bad_version():
    buf = bytearray()
    buf += _beth_magic()
    buf += struct.pack("<I", 999)  # bad version
    buf += struct.pack("<I", 2)
    buf += struct.pack("<I", ChunkType.STRT.value)
    buf += struct.pack("<I", 0)
    with pytest.raises(ValueError, match="unsupported reflection stream version"):
        BSReflStream(bytes(buf))


def test_stream_rejects_missing_strt():
    buf = bytearray()
    buf += _beth_magic()
    buf += struct.pack("<I", 4)
    buf += struct.pack("<I", 2)
    buf += struct.pack("<I", ChunkType.TYPE.value)  # not STRT
    buf += struct.pack("<I", 0)
    with pytest.raises(ValueError, match="missing string table"):
        BSReflStream(bytes(buf))


def test_stream_iterates_trailing_chunks():
    # Two trailing chunks: a TYPE chunk with body b"hi" and a LIST chunk empty.
    data = _build_minimal_stream([
        (ChunkType.TYPE.value, b"hi"),
        (ChunkType.LIST.value, b""),
    ])
    stream = BSReflStream(data)
    assert stream.chunks_remaining == 2

    ctype1, chunk1 = stream.read_chunk()
    assert ctype1 == ChunkType.TYPE.value
    assert chunk1 is not None
    assert chunk1.size == 2
    assert bytes(chunk1.data) == b"hi"

    ctype2, chunk2 = stream.read_chunk()
    assert ctype2 == ChunkType.LIST.value
    assert chunk2 is not None
    assert chunk2.size == 0

    ctype3, chunk3 = stream.read_chunk()
    assert ctype3 == 0
    assert chunk3 is None


def test_stream_string_table_maps_entries():
    # Build a STRT with one entry ("BSResource::ID") followed by a null
    # terminator byte. cpp stores the strtOffs -> master index mapping.
    name = b"BSResource::ID"
    strt_body = struct.pack("<I", len(name) + 1) + name + b"\x00"
    buf = bytearray()
    buf += _beth_magic()
    buf += struct.pack("<I", 4)
    buf += struct.pack("<I", 2)  # STRT + 0 user chunks
    buf += struct.pack("<I", ChunkType.STRT.value)
    buf += strt_body
    stream = BSReflStream(bytes(buf))
    # Header layout up to the STRT body: 8 (magic) + 4 (ver) + 4 (chunksRem)
    # + 4 (STRT type) + 4 (STRT length) = 24 bytes. So the first string byte
    # is at file offset 24, and strtOffs = filePos - 24 = 0.
    master_index = stream.find_string_by_offset(0)
    assert master_index == 228  # BSResource::ID


# ---------------------------------------------------------------------------
# Chunk reader helpers
# ---------------------------------------------------------------------------

def test_chunk_reads_primitive_values():
    # Build a chunk body that holds: u8, u16, u32, float, bool, string.
    body = bytearray()
    body += struct.pack("<B", 7)               # u8
    body += struct.pack("<H", 0x1234)          # u16
    body += struct.pack("<I", 0xDEADBEEF)      # u32
    body += struct.pack("<f", 3.5)             # float
    body += struct.pack("<B", 1)               # bool
    s = b"hello"
    body += struct.pack("<H", len(s)) + s      # length-prefixed string
    data = _build_minimal_stream([(ChunkType.OBJT.value, bytes(body))])
    stream = BSReflStream(data)
    ctype, chunk = stream.read_chunk()
    assert ctype == ChunkType.OBJT.value
    assert chunk is not None

    ok, u8 = chunk.read_u8()
    assert ok and u8 == 7
    ok, u16 = chunk.read_u16()
    assert ok and u16 == 0x1234
    ok, u32 = chunk.read_u32()
    assert ok and u32 == 0xDEADBEEF
    ok, f = chunk.read_float()
    assert ok and abs(f - 3.5) < 1e-6
    ok, b = chunk.read_bool()
    assert ok and b is True
    ok, string_val = chunk.read_string()
    assert ok and string_val == "hello"

    # Past end returns false
    ok, _ = chunk.read_u8()
    assert not ok


def test_chunk_readfloat_flushes_denormals_to_zero():
    # A denormal (exponent==0, mantissa!=0) should become zero per cpp behavior.
    # Denormal float: 0x00000001
    body = struct.pack("<I", 0x00000001)
    data = _build_minimal_stream([(ChunkType.OBJT.value, body)])
    stream = BSReflStream(data)
    _, chunk = stream.read_chunk()
    assert chunk is not None
    ok, f = chunk.read_float()
    assert ok and f == 0.0


# ---------------------------------------------------------------------------
# Compatibility wrapper for plan scaffold API
# ---------------------------------------------------------------------------

def test_read_bsrefl_wrapper_returns_stream():
    data = _build_minimal_stream([])
    stream, new_offset = read_bsrefl(data, 0)
    assert isinstance(stream, BSReflStream)
    assert new_offset == len(data)


def test_chunk_type_and_string_type_enum_values():
    assert ChunkType.BETH.value == 0x48544542
    assert ChunkType.STRT.value == 0x54525453
    assert ChunkType.TYPE.value == 0x45505954
    assert ChunkType.CLAS.value == 0x53414C43
    assert ChunkType.LIST.value == 0x5453494C
    assert ChunkType.MAPC.value == 0x4350414D
    assert ChunkType.OBJT.value == 0x544A424F
    assert ChunkType.DIFF.value == 0x46464944
    assert ChunkType.USER.value == 0x52455355
    assert ChunkType.USRD.value == 0x44525355

    assert StringType.NONE.value == 0
    assert StringType.STRING.value == 1
    assert StringType.REF.value == 4
    assert StringType.UINT32.value == 12
    assert StringType.FLOAT.value == 16
    assert StringType.UNKNOWN.value == 18
    assert StringType.BS_RESOURCE_ID.value == 228
