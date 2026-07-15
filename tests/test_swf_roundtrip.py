"""End-to-end SWF parse and document model tests."""
from __future__ import annotations

import pytest
from pathlib import Path

from creation_lib.swf.parser import parse_swf, SwfDocument
from creation_lib.swf.types import RGBA


class TestParseSwfSynthetic:
    """Test with synthetically constructed SWF binary."""

    def test_minimal_swf(self):
        """A minimal valid CWS SWF: header + FileAttributes + SetBackgroundColor + ShowFrame + End."""
        import struct, zlib

        # Build uncompressed body
        body = bytearray()

        # FileAttributes tag (id=69, length=4)
        _write_tag_header(body, 69, 4)
        body.extend(struct.pack("<I", 0))  # no flags

        # SetBackgroundColor (id=9, length=3)
        _write_tag_header(body, 9, 3)
        body.extend(bytes([0x33, 0x33, 0x33]))

        # ShowFrame (id=1, length=0)
        _write_tag_header(body, 1, 0)

        # End (id=0, length=0)
        _write_tag_header(body, 0, 0)

        # Build RECT for frame_size: 550x400 in twips (x20)
        rect_bits = _build_rect_bits(0, 11000, 0, 8000)

        # Header: version(1) + file_length(4) + RECT + fps(2) + frame_count(2)
        header = bytearray()
        header.extend(rect_bits)
        # FPS: 30.0 as 8.8 fixed = 30 << 8 = 7680
        header.extend(struct.pack("<H", 30 << 8))
        # Frame count
        header.extend(struct.pack("<H", 1))
        header.extend(body)

        compressed = zlib.compress(bytes(header))

        swf_data = bytearray()
        swf_data.extend(b"CWS")        # signature
        swf_data.append(17)             # version
        file_len = 8 + len(header)      # 8 = sig(3) + version(1) + length(4)
        swf_data.extend(struct.pack("<I", file_len))
        swf_data.extend(compressed)

        doc = parse_swf(bytes(swf_data))
        assert doc.header.version == 17
        assert doc.header.frame_size.width_px == 550
        assert doc.header.frame_size.height_px == 400
        assert doc.background_color.r == 0x33
        assert doc.main_timeline.frame_count == 1


class TestRoundTrip:
    def test_synthetic_roundtrip(self):
        """parse -> write -> parse produces identical document."""
        import struct, zlib

        # Build a minimal SWF (same as TestParseSwfSynthetic.test_minimal_swf)
        body = bytearray()
        _write_tag_header(body, 69, 4)
        body.extend(struct.pack("<I", 0))
        _write_tag_header(body, 9, 3)
        body.extend(bytes([0x33, 0x33, 0x33]))
        _write_tag_header(body, 1, 0)
        _write_tag_header(body, 0, 0)

        rect_bits = _build_rect_bits(0, 11000, 0, 8000)
        header = bytearray()
        header.extend(rect_bits)
        header.extend(struct.pack("<H", 30 << 8))
        header.extend(struct.pack("<H", 1))
        header.extend(body)

        compressed = zlib.compress(bytes(header))
        swf_data = bytearray(b"CWS")
        swf_data.append(17)
        swf_data.extend(struct.pack("<I", 8 + len(header)))
        swf_data.extend(compressed)

        # Round-trip
        from creation_lib.swf.writer import write_swf
        doc = parse_swf(bytes(swf_data))
        output = write_swf(doc)
        doc2 = parse_swf(output)

        assert doc2.header.version == doc.header.version
        assert doc2.header.frame_size.width_px == doc.header.frame_size.width_px
        assert doc2.header.frame_size.height_px == doc.header.frame_size.height_px
        assert doc2.background_color.r == doc.background_color.r
        assert doc2.main_timeline.frame_count == doc.main_timeline.frame_count


def _write_tag_header(buf: bytearray, tag_id: int, length: int) -> None:
    """Write a SWF tag header (short or long form)."""
    import struct
    if length < 63:
        tag_code_and_length = (tag_id << 6) | length
        buf.extend(struct.pack("<H", tag_code_and_length))
    else:
        tag_code_and_length = (tag_id << 6) | 0x3F
        buf.extend(struct.pack("<H", tag_code_and_length))
        buf.extend(struct.pack("<I", length))


def _build_rect_bits(xmin: int, xmax: int, ymin: int, ymax: int) -> bytes:
    """Build a RECT as bytes."""
    from creation_lib.swf.types import BitWriter, RECT
    w = BitWriter()
    RECT(xmin=xmin, xmax=xmax, ymin=ymin, ymax=ymax).write(w)
    w.align()
    return w.getvalue()
