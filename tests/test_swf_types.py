"""Tests for SWF bit-level reader and primitive types."""
from __future__ import annotations

import struct
import pytest

from creation_lib.swf.types import BitReader, RECT, MATRIX, RGBA, COLOR


class TestBitReader:
    def test_read_ubits(self):
        # 0b10110000 = 0xB0
        reader = BitReader(bytes([0xB0]))
        assert reader.read_ubits(1) == 1
        assert reader.read_ubits(3) == 3  # 0b011
        assert reader.read_ubits(4) == 0  # 0b0000

    def test_read_sbits(self):
        # 0b11110000 = 0xF0 -> reading 4 signed bits = -1 (0b1111)
        reader = BitReader(bytes([0xF0]))
        assert reader.read_sbits(4) == -1

    def test_read_ubits_cross_byte(self):
        # Read 12 bits across two bytes: 0xFF 0xF0 -> 0xFFF = 4095
        reader = BitReader(bytes([0xFF, 0xF0]))
        assert reader.read_ubits(12) == 4095

    def test_align(self):
        reader = BitReader(bytes([0xB0, 0x42]))
        reader.read_ubits(3)
        reader.align()
        # After align, should be at byte 1
        assert reader.read_ubits(8) == 0x42

    def test_read_bytes_after_align(self):
        reader = BitReader(bytes([0xFF, 0x01, 0x02]))
        reader.read_ubits(3)
        reader.align()
        assert reader.read_ui8() == 0x01
        assert reader.read_ui8() == 0x02


class TestRECT:
    def test_parse_rect(self):
        # RECT with Nbits=5: xmin=0, xmax=550*20=11000, ymin=0, ymax=400*20=8000
        # Nbits=5 means each field is 5 bits -- but real SWFs use larger Nbits
        # Use a known FO4 header RECT: 550x400 in twips (x20)
        # Nbits=16, xmin=0, xmax=11000, ymin=0, ymax=8000
        # 5 bits for Nbits (16=0b10000), then 4x16 bits = 69 bits total
        import io as _io
        buf = _io.BytesIO()
        # Build bit by bit: Nbits=16 (5 bits) + 4 fields of 16 bits each
        bits = "10000"  # Nbits = 16
        bits += format(0, "016b")      # xmin = 0
        bits += format(11000, "016b")  # xmax = 11000 (550*20 twips)
        bits += format(0, "016b")      # ymin = 0
        bits += format(8000, "016b")   # ymax = 8000 (400*20 twips)
        # Pad to byte boundary
        while len(bits) % 8 != 0:
            bits += "0"
        data = bytes(int(bits[i:i+8], 2) for i in range(0, len(bits), 8))
        reader = BitReader(data)
        rect = RECT.parse(reader)
        assert rect.xmin == 0
        assert rect.xmax == 11000
        assert rect.ymin == 0
        assert rect.ymax == 8000

    def test_rect_to_pixels(self):
        rect = RECT(xmin=0, xmax=11000, ymin=0, ymax=8000)
        w, h = rect.width_px, rect.height_px
        assert w == 550
        assert h == 400


class TestMATRIX:
    def test_identity_matrix(self):
        # Identity MATRIX: HasScale=0, HasRotate=0, TranslateX=0, TranslateY=0
        # Bit layout: 0 (no scale) + 0 (no rotate) + Nbits(5)=0 + tx(0) + ty(0)
        # Minimum: 2 bits (HasScale=0, HasRotate=0) + 5 bits Nbits for translate + 0+0
        bits = "00" + "00000" + "0" * 6  # pad
        while len(bits) % 8 != 0:
            bits += "0"
        data = bytes(int(bits[i:i+8], 2) for i in range(0, len(bits), 8))
        reader = BitReader(data)
        m = MATRIX.parse(reader)
        assert m.scale_x == 1.0
        assert m.scale_y == 1.0
        assert m.rotate_skew_0 == 0.0
        assert m.translate_x == 0
        assert m.translate_y == 0


class TestRGBA:
    def test_parse_rgba(self):
        reader = BitReader(bytes([0xFF, 0x00, 0x80, 0xC0]))
        c = RGBA.parse(reader)
        assert c.r == 255
        assert c.g == 0
        assert c.b == 128
        assert c.a == 192

    def test_parse_rgb(self):
        reader = BitReader(bytes([0xFF, 0x00, 0x80]))
        c = COLOR.parse(reader)
        assert c.r == 255
        assert c.g == 0
        assert c.b == 128
