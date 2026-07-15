"""DDS header builder for BA2 DX10 texture extraction.

Ported from NifSkope's filebuf.cpp:453-508 (writeDDSHeader).
Builds a 148-byte DDS header (128 DDS_HEADER + 20 DDS_HEADER_DXT10).
"""
import struct

from .dxgi_formats import get_format_info

# DDS magic + header constants
DDS_MAGIC = b"DDS "
DDS_HEADER_SIZE = 124
DDS_PIXELFORMAT_SIZE = 32

# dwFlags
DDSD_CAPS = 0x1
DDSD_HEIGHT = 0x2
DDSD_WIDTH = 0x4
DDSD_PIXELFORMAT = 0x1000
DDSD_MIPMAPCOUNT = 0x20000
DDSD_LINEARSIZE = 0x80000

# dwCaps
DDSCAPS_TEXTURE = 0x1000
DDSCAPS_MIPMAP = 0x400000
DDSCAPS_COMPLEX = 0x8

# dwCaps2
DDSCAPS2_CUBEMAP = 0x200
DDSCAPS2_CUBEMAP_ALL = 0xFE00  # all 6 faces

# DX10 fourcc
FOURCC_DX10 = b"DX10"


def build_dds_header(
    dxgi_format: int,
    width: int,
    height: int,
    mip_count: int,
    is_cubemap: bool = False,
) -> bytes:
    """Build a complete DDS header for a DX10 texture.

    Returns 148 bytes: 4 (magic) + 124 (DDS_HEADER) + 20 (DDS_HEADER_DXT10).
    """
    bpp, is_compressed = get_format_info(dxgi_format)
    if bpp == 0:
        bpp = 4  # fallback

    # Compute pitch/linear size
    if is_compressed:
        block_w = max(1, (width + 3) // 4)
        block_h = max(1, (height + 3) // 4)
        pitch_or_linear = block_w * block_h * bpp
    else:
        pitch_or_linear = width * bpp

    flags = DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT | DDSD_LINEARSIZE
    caps = DDSCAPS_TEXTURE
    caps2 = 0

    if mip_count > 1:
        flags |= DDSD_MIPMAPCOUNT
        caps |= DDSCAPS_MIPMAP | DDSCAPS_COMPLEX

    if is_cubemap:
        caps |= DDSCAPS_COMPLEX
        caps2 = DDSCAPS2_CUBEMAP | DDSCAPS2_CUBEMAP_ALL

    # DDS_HEADER (124 bytes)
    header = struct.pack(
        "<I"    # dwSize = 124
        "I"     # dwFlags
        "I"     # dwHeight
        "I"     # dwWidth
        "I"     # dwPitchOrLinearSize
        "I"     # dwDepth
        "I"     # dwMipMapCount
        "28s"   # dwReserved1[11] (44 bytes, but 28 here + overlap)
        ,
        DDS_HEADER_SIZE,
        flags,
        height,
        width,
        pitch_or_linear,
        0,      # depth
        mip_count,
        b"\x00" * 28,
    )
    # dwReserved1 remaining (16 bytes to reach offset 72 = pixel format start)
    header += b"\x00" * 16

    # DDS_PIXELFORMAT (32 bytes) — using DX10 extension
    header += struct.pack(
        "<I"    # dwSize = 32
        "I"     # dwFlags = 0x4 (DDPF_FOURCC)
        "4s"    # dwFourCC = "DX10"
        "I"     # dwRGBBitCount
        "I"     # dwRBitMask
        "I"     # dwGBitMask
        "I"     # dwBBitMask
        "I"     # dwABitMask
        ,
        DDS_PIXELFORMAT_SIZE,
        0x4,    # DDPF_FOURCC
        FOURCC_DX10,
        0, 0, 0, 0, 0,
    )

    # dwCaps, dwCaps2, dwCaps3, dwCaps4, dwReserved2
    header += struct.pack("<IIIII", caps, caps2, 0, 0, 0)

    # DDS_HEADER_DXT10 (20 bytes)
    array_size = 6 if is_cubemap else 1
    resource_dim = 3  # D3D10_RESOURCE_DIMENSION_TEXTURE2D (cubemaps use miscFlag, not resource_dim)
    misc_flag = 0x4 if is_cubemap else 0   # D3D10_RESOURCE_MISC_TEXTURECUBE
    dx10 = struct.pack(
        "<IIIII",
        dxgi_format,
        resource_dim,
        misc_flag,
        array_size,
        0,  # miscFlags2
    )

    return DDS_MAGIC + header + dx10
