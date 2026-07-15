"""SWF primitive types and bit-level binary reader.

SWF uses bit-packed fields. BitReader handles sub-byte reads.
All coordinates are in twips (1 pixel = 20 twips).
"""
from __future__ import annotations

import struct
from dataclasses import dataclass, field
from typing import BinaryIO


TWIPS_PER_PIXEL = 20


class BitReader:
    """Bit-level reader over a bytes buffer."""

    __slots__ = ("_data", "_pos", "_bit_pos")

    def __init__(self, data: bytes | bytearray | memoryview):
        self._data = bytes(data)
        self._pos = 0      # byte position
        self._bit_pos = 0   # bit offset within current byte (0-7, MSB first)

    @classmethod
    def from_stream(cls, stream: BinaryIO, length: int) -> "BitReader":
        return cls(stream.read(length))

    @property
    def remaining(self) -> int:
        """Remaining bytes (approximate -- ignores partial bits)."""
        return len(self._data) - self._pos

    def read_ubits(self, n: int) -> int:
        """Read n unsigned bits, MSB first."""
        if n == 0:
            return 0
        result = 0
        for _ in range(n):
            byte_val = self._data[self._pos]
            bit = (byte_val >> (7 - self._bit_pos)) & 1
            result = (result << 1) | bit
            self._bit_pos += 1
            if self._bit_pos == 8:
                self._bit_pos = 0
                self._pos += 1
        return result

    def read_sbits(self, n: int) -> int:
        """Read n signed bits (two's complement)."""
        if n == 0:
            return 0
        val = self.read_ubits(n)
        if val >= (1 << (n - 1)):
            val -= (1 << n)
        return val

    def read_fbits(self, n: int) -> float:
        """Read n-bit signed fixed-point 16.16."""
        return self.read_sbits(n) / 65536.0

    def align(self) -> None:
        """Advance to next byte boundary."""
        if self._bit_pos != 0:
            self._bit_pos = 0
            self._pos += 1

    def read_ui8(self) -> int:
        self.align()
        val = self._data[self._pos]
        self._pos += 1
        return val

    def read_ui16(self) -> int:
        self.align()
        val = struct.unpack_from("<H", self._data, self._pos)[0]
        self._pos += 2
        return val

    def read_si16(self) -> int:
        self.align()
        val = struct.unpack_from("<h", self._data, self._pos)[0]
        self._pos += 2
        return val

    def read_ui32(self) -> int:
        self.align()
        val = struct.unpack_from("<I", self._data, self._pos)[0]
        self._pos += 4
        return val

    def read_bytes(self, n: int) -> bytes:
        self.align()
        data = self._data[self._pos:self._pos + n]
        self._pos += n
        return data

    def read_string(self) -> str:
        """Read null-terminated string."""
        self.align()
        end = self._data.index(0, self._pos)
        s = self._data[self._pos:end].decode("utf-8", errors="replace")
        self._pos = end + 1
        return s

    def tell(self) -> int:
        """Current byte position."""
        return self._pos

    def seek(self, pos: int) -> None:
        self._pos = pos
        self._bit_pos = 0


class BitWriter:
    """Bit-level writer to a bytearray."""

    __slots__ = ("_data", "_current_byte", "_bit_pos")

    def __init__(self):
        self._data = bytearray()
        self._current_byte = 0
        self._bit_pos = 0  # bits written in current byte

    def write_ubits(self, n: int, value: int) -> None:
        for i in range(n - 1, -1, -1):
            bit = (value >> i) & 1
            self._current_byte = (self._current_byte << 1) | bit
            self._bit_pos += 1
            if self._bit_pos == 8:
                self._data.append(self._current_byte)
                self._current_byte = 0
                self._bit_pos = 0

    def write_sbits(self, n: int, value: int) -> None:
        if value < 0:
            value = value + (1 << n)
        self.write_ubits(n, value)

    def write_fbits(self, n: int, value: float) -> None:
        self.write_sbits(n, int(value * 65536.0))

    def align(self) -> None:
        if self._bit_pos != 0:
            self._current_byte <<= (8 - self._bit_pos)
            self._data.append(self._current_byte)
            self._current_byte = 0
            self._bit_pos = 0

    def write_ui8(self, val: int) -> None:
        self.align()
        self._data.append(val & 0xFF)

    def write_ui16(self, val: int) -> None:
        self.align()
        self._data.extend(struct.pack("<H", val))

    def write_si16(self, val: int) -> None:
        self.align()
        self._data.extend(struct.pack("<h", val))

    def write_ui32(self, val: int) -> None:
        self.align()
        self._data.extend(struct.pack("<I", val))

    def write_bytes(self, data: bytes) -> None:
        self.align()
        self._data.extend(data)

    def write_string(self, s: str) -> None:
        self.align()
        self._data.extend(s.encode("utf-8"))
        self._data.append(0)

    def getvalue(self) -> bytes:
        self.align()
        return bytes(self._data)


@dataclass
class RECT:
    """SWF RECT -- axis-aligned bounding box in twips."""
    xmin: int = 0
    xmax: int = 0
    ymin: int = 0
    ymax: int = 0

    @classmethod
    def parse(cls, reader: BitReader) -> "RECT":
        nbits = reader.read_ubits(5)
        return cls(
            xmin=reader.read_sbits(nbits),
            xmax=reader.read_sbits(nbits),
            ymin=reader.read_sbits(nbits),
            ymax=reader.read_sbits(nbits),
        )

    def write(self, writer: BitWriter) -> None:
        vals = [self.xmin, self.xmax, self.ymin, self.ymax]
        nbits = max(_sbits_needed(v) for v in vals) if any(v != 0 for v in vals) else 0
        writer.write_ubits(5, nbits)
        for v in vals:
            writer.write_sbits(nbits, v)

    @property
    def width_px(self) -> int:
        return (self.xmax - self.xmin) // TWIPS_PER_PIXEL

    @property
    def height_px(self) -> int:
        return (self.ymax - self.ymin) // TWIPS_PER_PIXEL


@dataclass
class MATRIX:
    """SWF MATRIX -- 2D affine transform."""
    scale_x: float = 1.0
    scale_y: float = 1.0
    rotate_skew_0: float = 0.0
    rotate_skew_1: float = 0.0
    translate_x: int = 0
    translate_y: int = 0

    @classmethod
    def parse(cls, reader: BitReader) -> "MATRIX":
        sx, sy = 1.0, 1.0
        r0, r1 = 0.0, 0.0
        has_scale = reader.read_ubits(1)
        if has_scale:
            nbits = reader.read_ubits(5)
            sx = reader.read_fbits(nbits)
            sy = reader.read_fbits(nbits)
        has_rotate = reader.read_ubits(1)
        if has_rotate:
            nbits = reader.read_ubits(5)
            r0 = reader.read_fbits(nbits)
            r1 = reader.read_fbits(nbits)
        nbits = reader.read_ubits(5)
        tx = reader.read_sbits(nbits)
        ty = reader.read_sbits(nbits)
        reader.align()
        return cls(scale_x=sx, scale_y=sy, rotate_skew_0=r0, rotate_skew_1=r1,
                   translate_x=tx, translate_y=ty)

    def write(self, writer: BitWriter) -> None:
        has_scale = self.scale_x != 1.0 or self.scale_y != 1.0
        writer.write_ubits(1, int(has_scale))
        if has_scale:
            nbits = max(_fbits_needed(self.scale_x), _fbits_needed(self.scale_y))
            writer.write_ubits(5, nbits)
            writer.write_fbits(nbits, self.scale_x)
            writer.write_fbits(nbits, self.scale_y)
        has_rotate = self.rotate_skew_0 != 0.0 or self.rotate_skew_1 != 0.0
        writer.write_ubits(1, int(has_rotate))
        if has_rotate:
            nbits = max(_fbits_needed(self.rotate_skew_0), _fbits_needed(self.rotate_skew_1))
            writer.write_ubits(5, nbits)
            writer.write_fbits(nbits, self.rotate_skew_0)
            writer.write_fbits(nbits, self.rotate_skew_1)
        vals = [self.translate_x, self.translate_y]
        nbits = max(_sbits_needed(v) for v in vals) if any(v != 0 for v in vals) else 0
        writer.write_ubits(5, nbits)
        writer.write_sbits(nbits, self.translate_x)
        writer.write_sbits(nbits, self.translate_y)
        writer.align()


@dataclass
class COLOR:
    """SWF RGB color."""
    r: int = 0
    g: int = 0
    b: int = 0

    @classmethod
    def parse(cls, reader: BitReader) -> "COLOR":
        return cls(r=reader.read_ui8(), g=reader.read_ui8(), b=reader.read_ui8())

    def write(self, writer: BitWriter) -> None:
        writer.write_ui8(self.r)
        writer.write_ui8(self.g)
        writer.write_ui8(self.b)


@dataclass
class RGBA:
    """SWF RGBA color with alpha."""
    r: int = 0
    g: int = 0
    b: int = 0
    a: int = 255

    @classmethod
    def parse(cls, reader: BitReader) -> "RGBA":
        return cls(r=reader.read_ui8(), g=reader.read_ui8(),
                   b=reader.read_ui8(), a=reader.read_ui8())

    def write(self, writer: BitWriter) -> None:
        writer.write_ui8(self.r)
        writer.write_ui8(self.g)
        writer.write_ui8(self.b)
        writer.write_ui8(self.a)

    def to_hex(self) -> str:
        return f"#{self.r:02x}{self.g:02x}{self.b:02x}"

    @classmethod
    def from_hex(cls, h: str, alpha: int = 255) -> "RGBA":
        h = h.lstrip("#")
        return cls(r=int(h[0:2], 16), g=int(h[2:4], 16), b=int(h[4:6], 16), a=alpha)


@dataclass
class GradientRecord:
    """Single gradient stop."""
    ratio: int  # 0-255
    color: RGBA


@dataclass
class Gradient:
    """SWF gradient fill."""
    spread: int = 0       # 0=pad, 1=reflect, 2=repeat
    interpolation: int = 0  # 0=normal, 1=linear
    records: list[GradientRecord] = field(default_factory=list)

    @classmethod
    def parse(cls, reader: BitReader, has_alpha: bool) -> "Gradient":
        spread = reader.read_ubits(2)
        interp = reader.read_ubits(2)
        count = reader.read_ubits(4)
        records = []
        for _ in range(count):
            ratio = reader.read_ui8()
            color = RGBA.parse(reader) if has_alpha else _rgb_to_rgba(COLOR.parse(reader))
            records.append(GradientRecord(ratio=ratio, color=color))
        return cls(spread=spread, interpolation=interp, records=records)

    def write(self, writer: BitWriter, has_alpha: bool) -> None:
        writer.write_ubits(2, self.spread)
        writer.write_ubits(2, self.interpolation)
        writer.write_ubits(4, len(self.records))
        for rec in self.records:
            writer.write_ui8(rec.ratio)
            if has_alpha:
                rec.color.write(writer)
            else:
                COLOR(rec.color.r, rec.color.g, rec.color.b).write(writer)


@dataclass
class FillStyle:
    """SWF fill style: solid, linear gradient, radial gradient, or bitmap."""
    fill_type: int  # 0x00=solid, 0x10=linear grad, 0x12=radial grad, 0x40-0x43=bitmap
    color: RGBA | None = None
    gradient_matrix: MATRIX | None = None
    gradient: Gradient | None = None
    bitmap_id: int | None = None
    bitmap_matrix: MATRIX | None = None

    @classmethod
    def parse(cls, reader: BitReader, has_alpha: bool) -> "FillStyle":
        ft = reader.read_ui8()
        color = None
        grad_matrix = None
        grad = None
        bmp_id = None
        bmp_matrix = None

        if ft == 0x00:
            color = RGBA.parse(reader) if has_alpha else _rgb_to_rgba(COLOR.parse(reader))
        elif ft in (0x10, 0x12, 0x13):
            grad_matrix = MATRIX.parse(reader)
            grad = Gradient.parse(reader, has_alpha)
        elif ft in (0x40, 0x41, 0x42, 0x43):
            bmp_id = reader.read_ui16()
            bmp_matrix = MATRIX.parse(reader)

        return cls(fill_type=ft, color=color, gradient_matrix=grad_matrix,
                   gradient=grad, bitmap_id=bmp_id, bitmap_matrix=bmp_matrix)

    def write(self, writer: BitWriter, has_alpha: bool) -> None:
        writer.write_ui8(self.fill_type)
        if self.fill_type == 0x00:
            if has_alpha:
                self.color.write(writer)
            else:
                COLOR(self.color.r, self.color.g, self.color.b).write(writer)
        elif self.fill_type in (0x10, 0x12, 0x13):
            self.gradient_matrix.write(writer)
            self.gradient.write(writer, has_alpha)
        elif self.fill_type in (0x40, 0x41, 0x42, 0x43):
            writer.write_ui16(self.bitmap_id)
            self.bitmap_matrix.write(writer)


@dataclass
class LineStyle:
    """SWF line style (SWF <=3: width + color)."""
    width: int  # twips
    color: RGBA

    @classmethod
    def parse(cls, reader: BitReader, has_alpha: bool) -> "LineStyle":
        w = reader.read_ui16()
        c = RGBA.parse(reader) if has_alpha else _rgb_to_rgba(COLOR.parse(reader))
        return cls(width=w, color=c)

    def write(self, writer: BitWriter, has_alpha: bool) -> None:
        writer.write_ui16(self.width)
        if has_alpha:
            self.color.write(writer)
        else:
            COLOR(self.color.r, self.color.g, self.color.b).write(writer)


@dataclass
class LineStyle2:
    """SWF 4+ line style with extended caps/joins."""
    width: int
    start_cap: int = 0
    join_style: int = 0
    no_hscale: bool = False
    no_vscale: bool = False
    pixel_hinting: bool = False
    no_close: bool = False
    end_cap: int = 0
    miter_limit: float = 0.0
    color: RGBA | None = None
    fill_style: FillStyle | None = None

    @classmethod
    def parse(cls, reader: BitReader) -> "LineStyle2":
        w = reader.read_ui16()
        start_cap = reader.read_ubits(2)
        join = reader.read_ubits(2)
        has_fill = reader.read_ubits(1)
        no_h = bool(reader.read_ubits(1))
        no_v = bool(reader.read_ubits(1))
        pixel_hint = bool(reader.read_ubits(1))
        _reserved = reader.read_ubits(5)
        no_close = bool(reader.read_ubits(1))
        end_cap = reader.read_ubits(2)
        miter = 0.0
        if join == 2:
            miter = reader.read_ui16() / 256.0  # fixed 8.8
        color = None
        fill = None
        if has_fill:
            fill = FillStyle.parse(reader, has_alpha=True)
        else:
            color = RGBA.parse(reader)
        return cls(width=w, start_cap=start_cap, join_style=join,
                   no_hscale=no_h, no_vscale=no_v, pixel_hinting=pixel_hint,
                   no_close=no_close, end_cap=end_cap, miter_limit=miter,
                   color=color, fill_style=fill)

    def write(self, writer: BitWriter) -> None:
        writer.write_ui16(self.width)
        writer.write_ubits(2, self.start_cap)
        writer.write_ubits(2, self.join_style)
        has_fill = 1 if self.fill_style else 0
        writer.write_ubits(1, has_fill)
        writer.write_ubits(1, int(self.no_hscale))
        writer.write_ubits(1, int(self.no_vscale))
        writer.write_ubits(1, int(self.pixel_hinting))
        writer.write_ubits(5, 0)  # reserved
        writer.write_ubits(1, int(self.no_close))
        writer.write_ubits(2, self.end_cap)
        if self.join_style == 2:
            writer.write_ui16(int(self.miter_limit * 256))
        if has_fill:
            self.fill_style.write(writer, has_alpha=True)
        else:
            self.color.write(writer)


def _rgb_to_rgba(c: COLOR) -> RGBA:
    return RGBA(r=c.r, g=c.g, b=c.b, a=255)


def _sbits_needed(value: int) -> int:
    """Minimum bits to represent a signed value."""
    if value == 0:
        return 0
    if value > 0:
        return value.bit_length() + 1
    return (value + 1).bit_length() + 1 if value != -1 else 2


def _fbits_needed(value: float) -> int:
    """Minimum bits for a fixed-point 16.16 value."""
    return _sbits_needed(int(value * 65536.0))


def parse_fill_styles(reader: BitReader, has_alpha: bool) -> list[FillStyle]:
    count = reader.read_ui8()
    if count == 0xFF:
        count = reader.read_ui16()
    return [FillStyle.parse(reader, has_alpha) for _ in range(count)]


def parse_line_styles(reader: BitReader, shape_version: int, has_alpha: bool) -> list[LineStyle | LineStyle2]:
    count = reader.read_ui8()
    if count == 0xFF:
        count = reader.read_ui16()
    if shape_version >= 4:
        return [LineStyle2.parse(reader) for _ in range(count)]
    return [LineStyle.parse(reader, has_alpha) for _ in range(count)]


def write_fill_styles(writer: BitWriter, styles: list[FillStyle], has_alpha: bool) -> None:
    if len(styles) < 0xFF:
        writer.write_ui8(len(styles))
    else:
        writer.write_ui8(0xFF)
        writer.write_ui16(len(styles))
    for s in styles:
        s.write(writer, has_alpha)


def write_line_styles(writer: BitWriter, styles: list[LineStyle | LineStyle2],
                      shape_version: int, has_alpha: bool) -> None:
    if len(styles) < 0xFF:
        writer.write_ui8(len(styles))
    else:
        writer.write_ui8(0xFF)
        writer.write_ui16(len(styles))
    for s in styles:
        if shape_version >= 4 and isinstance(s, LineStyle2):
            s.write(writer)
        else:
            s.write(writer, has_alpha)
