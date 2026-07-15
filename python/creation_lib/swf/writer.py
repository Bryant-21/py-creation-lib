"""SWF binary writer -- serializes SwfDocument to .swf.

Produces CWS (zlib-compressed) SWF version 17 by default,
matching Fallout 4's expected format.
"""
from __future__ import annotations

import struct
import zlib
from pathlib import Path

from creation_lib.swf.types import BitWriter
from creation_lib.swf.tags import SwfTag, write_tag, TAG_END, TAG_DEFINE_SPRITE
from creation_lib.swf.parser import SwfDocument


def write_swf(doc: SwfDocument, version: int | None = None) -> bytes:
    """Serialize SwfDocument to SWF binary (CWS zlib compression)."""
    ver = version or doc.header.version or 17

    # Serialize all tags to bytes
    tag_bytes = _write_tag_stream(doc.tags)

    # Build uncompressed body: RECT + fps + frame_count + tags
    body_writer = BitWriter()
    doc.header.frame_size.write(body_writer)
    body_writer.align()
    # FPS as 8.8 fixed point
    fps_int = int(doc.header.fps)
    body_writer.write_ui16(fps_int << 8)
    body_writer.write_ui16(doc.header.frame_count)
    body_writer.write_bytes(tag_bytes)

    body = body_writer.getvalue()

    # File length = 8 (header prefix) + body length
    file_length = 8 + len(body)

    # Compress
    compressed = zlib.compress(body, level=6)

    # Assemble: signature(3) + version(1) + file_length(4) + compressed_body
    out = bytearray()
    out.extend(b"CWS")
    out.append(ver)
    out.extend(struct.pack("<I", file_length))
    out.extend(compressed)

    return bytes(out)


def write_swf_file(doc: SwfDocument, path: str | Path, version: int | None = None) -> None:
    """Write SwfDocument to a file."""
    Path(path).write_bytes(write_swf(doc, version))


def _write_tag_stream(tags: list[SwfTag]) -> bytes:
    """Serialize a list of tags to a byte stream."""
    out = bytearray()

    def tag_writer(nested_tags: list[SwfTag]) -> bytes:
        return _write_tag_stream(nested_tags)

    for tag in tags:
        tag_id, body = write_tag(tag, tag_writer=tag_writer)
        _write_tag_to_buf(out, tag_id, body)

    return bytes(out)


def _write_tag_to_buf(buf: bytearray, tag_id: int, body: bytes) -> None:
    """Write a tag header + body to buffer."""
    length = len(body)
    if length < 63:
        tag_code_and_length = (tag_id << 6) | length
        buf.extend(struct.pack("<H", tag_code_and_length))
    else:
        tag_code_and_length = (tag_id << 6) | 0x3F
        buf.extend(struct.pack("<H", tag_code_and_length))
        buf.extend(struct.pack("<I", length))
    buf.extend(body)
