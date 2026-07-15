"""Tests for BYTES/STRUCT/ARRAY codecs in py_creation_lib/python/creation_lib/esp/editor/fields.py."""
from __future__ import annotations

import struct

import pytest

from creation_lib.esp.editor.fields import (
    Field,
    FieldKind,
    decode_bytes,
    encode_bytes,
    decode_struct,
    encode_struct,
    decode_array,
    encode_array,
    encode_field,
)


# ---------------------------------------------------------------------------
# BYTES codec
# ---------------------------------------------------------------------------

def test_decode_bytes_passthrough():
    raw = b"\x01\x02\x03"
    assert decode_bytes(raw) == raw


def test_encode_bytes_from_bytes():
    raw = b"\xAA\xBB"
    assert encode_bytes(raw) == raw


def test_encode_bytes_from_bytearray():
    assert encode_bytes(bytearray(b"\x01\x02")) == b"\x01\x02"


def test_encode_bytes_from_hex_string():
    assert encode_bytes("0011AABB") == b"\x00\x11\xAA\xBB"


def test_encode_bytes_hex_string_with_spaces():
    assert encode_bytes("00 11 AA BB") == b"\x00\x11\xAA\xBB"


def test_encode_bytes_roundtrip():
    raw = bytes(range(8))
    assert encode_bytes(raw) == raw


# ---------------------------------------------------------------------------
# STRUCT codec
# ---------------------------------------------------------------------------

def test_decode_struct_basic():
    layout = "<HH"
    data = struct.pack(layout, 10, 20)
    result = decode_struct(data, layout)
    assert result == (10, 20)


def test_decode_struct_no_layout_falls_back_to_bytes():
    raw = b"\x01\x02"
    result = decode_struct(raw, None)
    assert result == raw


def test_decode_struct_short_data_falls_back_to_bytes():
    layout = "<I"
    raw = b"\x01"  # too short for uint32
    result = decode_struct(raw, layout)
    assert result == raw


def test_encode_struct_roundtrip():
    layout = "<Hf"
    values = (42, 3.14)
    encoded = encode_struct(values, layout)
    decoded = decode_struct(encoded, layout)
    assert decoded[0] == 42
    assert abs(decoded[1] - 3.14) < 1e-5


def test_encode_struct_from_bytes_passthrough():
    raw = b"\x01\x02\x03\x04"
    assert encode_struct(raw, "<I") == raw


def test_encode_struct_no_layout_hex_string():
    assert encode_struct("AABB", None) == b"\xAA\xBB"


def test_encode_struct_bad_values_returns_empty():
    result = encode_struct(("not", "valid"), "<HH")
    assert result == b""


# ---------------------------------------------------------------------------
# ARRAY codec
# ---------------------------------------------------------------------------

def test_decode_array_uint16():
    layout = "<H"
    data = struct.pack("<HHH", 1, 2, 3)
    result = decode_array(data, layout)
    assert result == [1, 2, 3]


def test_decode_array_struct_elements():
    layout = "<Hf"
    elem1 = struct.pack(layout, 10, 1.5)
    elem2 = struct.pack(layout, 20, 2.5)
    result = decode_array(elem1 + elem2, layout)
    assert len(result) == 2
    assert result[0][0] == 10
    assert abs(result[0][1] - 1.5) < 1e-5


def test_decode_array_no_layout_falls_back_to_bytes():
    raw = b"\x01\x02"
    assert decode_array(raw, None) == raw


def test_decode_array_empty_data():
    result = decode_array(b"", "<H")
    assert result == b""


def test_encode_array_roundtrip():
    layout = "<H"
    values = [10, 20, 30]
    encoded = encode_array(values, layout)
    decoded = decode_array(encoded, layout)
    assert decoded == values


def test_encode_array_tuple_elements():
    layout = "<Hf"
    values = [(1, 1.0), (2, 2.0)]
    encoded = encode_array(values, layout)
    decoded = decode_array(encoded, layout)
    assert len(decoded) == 2
    assert decoded[0][0] == 1
    assert abs(decoded[1][1] - 2.0) < 1e-5


def test_encode_array_from_hex_string():
    result = encode_array("0102", None)
    assert result == b"\x01\x02"


# ---------------------------------------------------------------------------
# encode_field dispatch for BYTES/STRUCT/ARRAY
# ---------------------------------------------------------------------------

def _make_field(kind: FieldKind, raw: bytes, layout: str | None = None) -> Field:
    return Field(
        name="test",
        signature="TEST",
        kind=kind,
        value=raw,
        raw=raw,
        struct_layout=layout,
    )


def test_encode_field_bytes_kind():
    fld = _make_field(FieldKind.BYTES, b"\x01\x02")
    result = encode_field(fld, b"\x03\x04")
    assert result == b"\x03\x04"


def test_encode_field_bytes_hex_string():
    fld = _make_field(FieldKind.BYTES, b"\x00\x00")
    result = encode_field(fld, "AABB")
    assert result == b"\xAA\xBB"


def test_encode_field_struct_kind_roundtrip():
    layout = "<HH"
    raw = struct.pack(layout, 5, 10)
    fld = _make_field(FieldKind.STRUCT, raw, layout)
    result = encode_field(fld, (5, 10))
    assert result == raw


def test_encode_field_array_kind_roundtrip():
    layout = "<H"
    raw = struct.pack("<HH", 1, 2)
    fld = _make_field(FieldKind.ARRAY, raw, layout)
    result = encode_field(fld, [1, 2])
    assert result == raw
