"""DXGI format size table for BA2 DX10 texture archives.

Ported from NifSkope's filebuf.cpp:432 (dxgiFormatSizeTable).
Maps DXGI format enum -> (bytes_per_block, is_block_compressed).
"""

# (bytes_per_block, is_block_compressed)
# Block-compressed formats use 4x4 pixel blocks.
DXGI_FORMAT_SIZES: dict[int, tuple[int, bool]] = {
    # R8G8B8A8 variants
    28: (4, False),   # DXGI_FORMAT_R8G8B8A8_UNORM
    29: (4, False),   # DXGI_FORMAT_R8G8B8A8_UNORM_SRGB (same size)

    # BC1 (DXT1) — 8 bytes per 4x4 block
    71: (8, True),    # DXGI_FORMAT_BC1_UNORM
    72: (8, True),    # DXGI_FORMAT_BC1_UNORM_SRGB

    # BC2 (DXT3) — 16 bytes per 4x4 block
    74: (16, True),   # DXGI_FORMAT_BC2_UNORM
    75: (16, True),   # DXGI_FORMAT_BC2_UNORM_SRGB

    # BC3 (DXT5) — 16 bytes per 4x4 block
    77: (16, True),   # DXGI_FORMAT_BC3_UNORM
    78: (16, True),   # DXGI_FORMAT_BC3_UNORM_SRGB

    # BC4 — 8 bytes per 4x4 block
    80: (8, True),    # DXGI_FORMAT_BC4_UNORM
    81: (8, True),    # DXGI_FORMAT_BC4_SNORM

    # BC5 (ATI2) — 16 bytes per 4x4 block
    83: (16, True),   # DXGI_FORMAT_BC5_UNORM
    84: (16, True),   # DXGI_FORMAT_BC5_SNORM

    # B8G8R8A8
    87: (4, False),   # DXGI_FORMAT_B8G8R8A8_UNORM
    91: (4, False),   # DXGI_FORMAT_B8G8R8A8_UNORM_SRGB

    # BC6H — 16 bytes per 4x4 block
    95: (16, True),   # DXGI_FORMAT_BC6H_UF16
    96: (16, True),   # DXGI_FORMAT_BC6H_SF16

    # BC7 — 16 bytes per 4x4 block
    98: (16, True),   # DXGI_FORMAT_BC7_UNORM
    99: (16, True),   # DXGI_FORMAT_BC7_UNORM_SRGB

    # Single-channel
    61: (1, False),   # DXGI_FORMAT_R8_UNORM
    49: (2, False),   # DXGI_FORMAT_R16_FLOAT
    54: (4, False),   # DXGI_FORMAT_R32_FLOAT

    # Two-channel
    56: (2, False),   # DXGI_FORMAT_R8G8_UNORM

    # 16-bit
    85: (2, False),   # DXGI_FORMAT_B5G6R5_UNORM
    86: (2, False),   # DXGI_FORMAT_B5G5R5A1_UNORM

    # R16G16B16A16
    10: (8, False),   # DXGI_FORMAT_R16G16B16A16_FLOAT

    # R10G10B10A2
    24: (4, False),   # DXGI_FORMAT_R10G10B10A2_UNORM

    # R11G11B10
    26: (4, False),   # DXGI_FORMAT_R11G11B10_FLOAT
}


def get_format_info(dxgi_format: int) -> tuple[int, bool]:
    """Return (bytes_per_block, is_block_compressed) for a DXGI format.

    Returns (0, False) for unknown formats.
    """
    return DXGI_FORMAT_SIZES.get(dxgi_format, (0, False))
