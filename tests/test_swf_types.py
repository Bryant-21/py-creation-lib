"""Tests for SWF bit-level reader and primitive types."""
from __future__ import annotations

from creation_lib.swf.types import BitReader, RECT, MATRIX, RGBA, COLOR


class TestBitReader:
    def test_read_ubits(self):
        # 0b10110000 = 0xB0
        reader = BitReader(bytes([0xB0]))
        assert reader.read_ubits(1) == 1
        assert reader.read_ubits(3) == 3  # 0b011
        assert reader.read_ubits(4) == 0  # 0b0000

    def test_align(self):
        reader = BitReader(bytes([0xB0, 0x42]))
        reader.read_ubits(3)
        reader.align()
        # After align, should be at byte 1
        assert reader.read_ubits(8) == 0x42


class TestRECT:
    def test_parse_rect(self):
        # Build bit by bit: Nbits=16 (5 bits) + 4 fields of 16 bits each
        # Known FO4 header RECT: 550x400 in twips (x20)
        bits = "10000"  # Nbits = 16
        bits += format(0, "016b")      # xmin = 0
        bits += format(11000, "016b")  # xmax = 11000 (550*20 twips)
        bits += format(0, "016b")      # ymin = 0
        bits += format(8000, "016b")   # ymax = 8000 (400*20 twips)
        while len(bits) % 8 != 0:
            bits += "0"
        data = bytes(int(bits[i:i+8], 2) for i in range(0, len(bits), 8))
        reader = BitReader(data)
        rect = RECT.parse(reader)
        assert rect.xmin == 0
        assert rect.xmax == 11000
        assert rect.ymin == 0
        assert rect.ymax == 8000


class TestMATRIX:
    def test_identity_matrix(self):
        # Identity MATRIX: HasScale=0, HasRotate=0, Nbits(5)=0, tx=0, ty=0
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
    def test_parse_rgba_and_rgb(self):
        c = RGBA.parse(BitReader(bytes([0xFF, 0x00, 0x80, 0xC0])))
        assert (c.r, c.g, c.b, c.a) == (255, 0, 128, 192)

        c3 = COLOR.parse(BitReader(bytes([0xFF, 0x00, 0x80])))
        assert (c3.r, c3.g, c3.b) == (255, 0, 128)
