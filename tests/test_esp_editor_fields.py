"""Tests for BYTES/STRUCT/ARRAY codecs in py_creation_lib/python/creation_lib/esp/editor/fields.py."""
from __future__ import annotations

import struct

import pytest

from creation_lib.esp.editor.fields import (
    Field,
    FieldKind,
    decode_array,
    decode_bytes,
    decode_struct,
    encode_array,
    encode_bytes,
    encode_field,
    encode_struct,
)


# ---------------------------------------------------------------------------
# BYTES codec
# ---------------------------------------------------------------------------

def test_decode_bytes_passthrough():
    raw = b"\x01\x02\x03"
    assert decode_bytes(raw) == raw


@pytest.mark.parametrize(
    ("value", "expected"),
    [
        (b"\xAA\xBB", b"\xAA\xBB"),
        (bytearray(b"\x01\x02"), b"\x01\x02"),
        ("0011AABB", b"\x00\x11\xAA\xBB"),
        ("00 11 AA BB", b"\x00\x11\xAA\xBB"),
    ],
    ids=["bytes", "bytearray", "hex-string", "hex-string-with-spaces"],
)
def test_encode_bytes(value, expected):
    assert encode_bytes(value) == expected


# ---------------------------------------------------------------------------
# STRUCT codec
# ---------------------------------------------------------------------------

@pytest.mark.parametrize(
    ("raw", "layout", "expected"),
    [
        (struct.pack("<HH", 10, 20), "<HH", (10, 20)),
        (b"\x01\x02", None, b"\x01\x02"),
        (b"\x01", "<I", b"\x01"),
    ],
    ids=["layout-matches-data", "no-layout-falls-back-to-bytes", "data-too-short-falls-back-to-bytes"],
)
def test_decode_struct(raw, layout, expected):
    assert decode_struct(raw, layout) == expected


def test_encode_struct_roundtrip():
    layout = "<Hf"
    encoded = encode_struct((42, 3.14), layout)
    decoded = decode_struct(encoded, layout)
    assert decoded[0] == 42
    assert abs(decoded[1] - 3.14) < 1e-5


@pytest.mark.parametrize(
    ("value", "layout", "expected"),
    [
        (b"\x01\x02\x03\x04", "<I", b"\x01\x02\x03\x04"),
        ("AABB", None, b"\xAA\xBB"),
        (("not", "valid"), "<HH", b""),
    ],
    ids=["bytes-passthrough", "no-layout-hex-string", "bad-values-returns-empty"],
)
def test_encode_struct(value, layout, expected):
    assert encode_struct(value, layout) == expected


# ---------------------------------------------------------------------------
# ARRAY codec
# ---------------------------------------------------------------------------

@pytest.mark.parametrize(
    ("raw", "layout", "expected"),
    [
        (struct.pack("<HHH", 1, 2, 3), "<H", [1, 2, 3]),
        (b"\x01\x02", None, b"\x01\x02"),
        (b"", "<H", b""),
    ],
    ids=["scalar-elements", "no-layout-falls-back-to-bytes", "empty-data"],
)
def test_decode_array(raw, layout, expected):
    assert decode_array(raw, layout) == expected


def test_decode_array_struct_elements():
    layout = "<Hf"
    elem1 = struct.pack(layout, 10, 1.5)
    elem2 = struct.pack(layout, 20, 2.5)
    result = decode_array(elem1 + elem2, layout)
    assert len(result) == 2
    assert result[0][0] == 10
    assert abs(result[0][1] - 1.5) < 1e-5


def test_encode_array_roundtrip():
    values = [10, 20, 30]
    encoded = encode_array(values, "<H")
    assert decode_array(encoded, "<H") == values


def test_encode_array_tuple_elements():
    layout = "<Hf"
    encoded = encode_array([(1, 1.0), (2, 2.0)], layout)
    decoded = decode_array(encoded, layout)
    assert len(decoded) == 2
    assert decoded[0][0] == 1
    assert abs(decoded[1][1] - 2.0) < 1e-5
    assert encode_array("0102", None) == b"\x01\x02"


# ---------------------------------------------------------------------------
# encode_field dispatch for BYTES/STRUCT/ARRAY
# ---------------------------------------------------------------------------

def _make_field(kind: FieldKind, raw: bytes, layout: str | None = None) -> Field:
    return Field(name="test", signature="TEST", kind=kind, value=raw, raw=raw, struct_layout=layout)


@pytest.mark.parametrize(
    ("kind", "raw", "layout", "value", "expected"),
    [
        (FieldKind.BYTES, b"\x01\x02", None, b"\x03\x04", b"\x03\x04"),
        (FieldKind.BYTES, b"\x00\x00", None, "AABB", b"\xAA\xBB"),
        (FieldKind.STRUCT, struct.pack("<HH", 5, 10), "<HH", (5, 10), struct.pack("<HH", 5, 10)),
        (FieldKind.ARRAY, struct.pack("<HH", 1, 2), "<H", [1, 2], struct.pack("<HH", 1, 2)),
    ],
    ids=["bytes", "bytes-hex-string", "struct-roundtrip", "array-roundtrip"],
)
def test_encode_field_dispatches_by_kind(kind, raw, layout, value, expected):
    fld = _make_field(kind, raw, layout)
    assert encode_field(fld, value) == expected
