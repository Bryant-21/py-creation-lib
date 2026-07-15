import pathlib

import numpy as np
import pytest
from creation_lib.palette.remap import (
    VALID_REMAP_VALUES, valid_remap_values, build_gradient, build_final_strip,
    PaletteZone, NEUTRAL_GREY, assign_columns, build_remap_texture, auto_convert,
)
from PIL import Image

def test_valid_remap_values_width32():
    vals = valid_remap_values(32)
    assert len(vals) == 32
    assert vals[0] == 0
    assert vals[31] == 255

def test_valid_remap_values_width64():
    vals = valid_remap_values(64)
    assert len(vals) == 64
    assert vals[0] == 0
    assert vals[63] == 255

def test_valid_remap_values_width128():
    vals = valid_remap_values(128)
    assert len(vals) == 128
    assert vals[0] == 0
    assert vals[127] == 255

def test_valid_remap_values_formula():
    for width in (32, 64, 128):
        vals = valid_remap_values(width)
        expected = [round(col / (width - 1) * 255) for col in range(width)]
        assert vals == expected


def _make_zone(index, avg_color_rgb, gradient_column, width=32):
    """Helper: build a PaletteZone with a dummy mask."""
    vals = valid_remap_values(width)
    z = PaletteZone(
        index=index,
        avg_color=np.array(avg_color_rgb, dtype=np.float32),
        pixel_mask=np.zeros((4, 4), dtype=bool),
        remap_value=vals[gradient_column],
        gradient_column=gradient_column,
    )
    return z

def test_gradient_shape():
    zones = [_make_zone(0, [200, 100, 50], 31)]
    grad = build_gradient(zones)
    assert grad.shape == (32, 32, 4)
    assert grad.dtype == np.uint8

def test_gradient_col0_is_neutral_grey():
    zones = [_make_zone(0, [200, 100, 50], 31)]
    grad = build_gradient(zones)
    for row in range(32):
        np.testing.assert_array_equal(grad[row, 0, :3], NEUTRAL_GREY)

def test_gradient_row0_is_grey_for_all_cols():
    zones = [_make_zone(0, [200, 100, 50], 16), _make_zone(1, [50, 150, 80], 31)]
    grad = build_gradient(zones)
    for col in range(1, 32):
        np.testing.assert_array_equal(grad[0, col, :3], NEUTRAL_GREY)

def test_gradient_last_row_matches_zone_color():
    color = [200, 100, 50]
    zones = [_make_zone(0, color, 31)]
    grad = build_gradient(zones)
    np.testing.assert_array_almost_equal(grad[31, 31, :3], color, decimal=0)

def test_gradient_alpha_is_255():
    zones = [_make_zone(0, [100, 100, 100], 16)]
    grad = build_gradient(zones)
    assert (grad[:, :, 3] == 255).all()

def test_gradient_width64_shape():
    zones = [_make_zone(0, [200, 100, 50], 63, width=64)]
    grad = build_gradient(zones, width=64)
    assert grad.shape == (32, 64, 4)

def test_gradient_width128_col0_is_grey():
    zones = [_make_zone(0, [200, 100, 50], 127, width=128)]
    grad = build_gradient(zones, width=128)
    for row in range(32):
        np.testing.assert_array_equal(grad[row, 0, :3], NEUTRAL_GREY)

def test_gradient_width64_last_col_matches_color():
    color = [200, 100, 50]
    zones = [_make_zone(0, color, 63, width=64)]
    grad = build_gradient(zones, width=64)
    np.testing.assert_array_almost_equal(grad[31, 63, :3], color, decimal=0)

def test_zones_from_remap_width64():
    from creation_lib.palette.remap import zones_from_remap
    vals = valid_remap_values(64)
    remap = np.zeros((4, 4), dtype=np.uint8)
    remap[0, :] = vals[10]
    remap[1, :] = vals[30]
    remap[2, :] = vals[63]
    zones = zones_from_remap(remap, width=64)
    assert len(zones) == 3
    cols = [z.gradient_column for z in zones]
    assert cols == [10, 30, 63]

def test_assign_columns_two_zones():
    # 2 zones → columns 1 and 31 (zones span 1-31; column 0 is reserved fixed grey)
    colors = [np.array([100.0, 50.0, 30.0]), np.array([50.0, 150.0, 80.0])]
    zones = assign_columns(colors)
    assert zones[0].gradient_column == 1
    assert zones[1].gradient_column == 31

def test_assign_columns_four_zones():
    # 4 zones → columns 1, 11, 21, 31 (evenly spaced across 1-31)
    colors = [np.array([float(i*50), 0.0, 0.0]) for i in range(4)]
    zones = assign_columns(colors)
    expected_cols = [round(i / 3 * 30) + 1 for i in range(4)]
    assert [z.gradient_column for z in zones] == expected_cols

def test_assign_columns_single_zone():
    # N=1 → column 31 (spec explicit: single zone = full paint)
    colors = [np.array([100.0, 100.0, 100.0])]
    zones = assign_columns(colors)
    assert zones[0].gradient_column == 31

def test_assign_columns_width64_two_zones():
    colors = [np.array([100.0, 50.0, 30.0]), np.array([50.0, 150.0, 80.0])]
    zones = assign_columns(colors, width=64)
    assert zones[0].gradient_column == 1
    assert zones[1].gradient_column == 63

def test_assign_columns_width128_four_zones():
    colors = [np.array([float(i * 50), 0.0, 0.0]) for i in range(4)]
    zones = assign_columns(colors, width=128)
    expected_cols = [round(i / 3 * 126) + 1 for i in range(4)]
    assert [z.gradient_column for z in zones] == expected_cols

def test_assign_columns_width64_remap_values():
    colors = [np.array([100.0, 50.0, 30.0]), np.array([50.0, 150.0, 80.0])]
    zones = assign_columns(colors, width=64)
    vals = valid_remap_values(64)
    for z in zones:
        assert z.remap_value == vals[z.gradient_column]

def test_assign_columns_width128_single_zone():
    colors = [np.array([100.0, 100.0, 100.0])]
    zones = assign_columns(colors, width=128)
    assert zones[0].gradient_column == 127

def test_gradient_nearest_column_tiebreak_goes_to_lower():
    # zones at cols 1 and 31 — col 16 is equidistant; should copy from col 1 (lower)
    zones = [_make_zone(0, [200, 50, 50], 1), _make_zone(1, [50, 200, 50], 31)]
    grad = build_gradient(zones)
    # col 16: dist to 1 = 15, dist to 31 = 15 — tie → copy col 1
    np.testing.assert_array_equal(grad[:, 16, :3], grad[:, 1, :3])

def test_assign_columns_remap_values_match_columns():
    colors = [np.array([float(i*40), 0.0, 0.0]) for i in range(6)]
    zones = assign_columns(colors)
    for z in zones:
        assert z.remap_value == VALID_REMAP_VALUES[z.gradient_column]

def test_build_remap_texture_shape():
    h, w = 8, 8
    label_map = np.zeros((h, w), dtype=np.int32)
    colors = [np.array([100.0, 100.0, 100.0])]
    zones = assign_columns(colors)
    zones[0].pixel_mask = np.ones((h, w), dtype=bool)
    remap = build_remap_texture((h, w), label_map, zones)
    assert remap.shape == (h, w)
    assert remap.dtype == np.uint8

def test_build_remap_texture_values():
    h, w = 4, 4
    label_map = np.zeros((h, w), dtype=np.int32)
    label_map[2:, :] = 1
    colors = [np.array([30.0, 100.0, 50.0]), np.array([180.0, 60.0, 40.0])]
    zones = assign_columns(colors)
    zones[0].pixel_mask = label_map == 0
    zones[1].pixel_mask = label_map == 1
    remap = build_remap_texture((h, w), label_map, zones)
    assert (remap[:2, :] == zones[0].remap_value).all()
    assert (remap[2:, :] == zones[1].remap_value).all()

def _make_two_zone_image():
    """4x4 RGBA image: top half rust (200,80,40), bottom half olive (80,160,60), fully opaque."""
    img = np.zeros((4, 4, 4), dtype=np.uint8)
    img[:2, :] = [200, 80, 40, 255]
    img[2:, :] = [80, 160, 60, 255]
    return Image.fromarray(img, mode="RGBA")

def test_build_final_strip_shape_32():
    zones = [_make_zone(0, [200, 100, 50], 31)]
    strip = build_final_strip(zones, width=32)
    assert strip.shape == (4, 32, 4)
    assert strip.dtype == np.uint8

def test_build_final_strip_shape_64():
    color = [200, 100, 50]
    zones = [PaletteZone(0, np.array(color, dtype=np.float32),
             np.zeros((0, 0), dtype=bool), valid_remap_values(64)[63], 63)]
    strip = build_final_strip(zones, width=64)
    assert strip.shape == (4, 64, 4)

def test_build_final_strip_shape_128():
    color = [200, 100, 50]
    zones = [PaletteZone(0, np.array(color, dtype=np.float32),
             np.zeros((0, 0), dtype=bool), valid_remap_values(128)[127], 127)]
    strip = build_final_strip(zones, width=128)
    assert strip.shape == (4, 128, 4)

def test_build_final_strip_col0_is_grey():
    zones = [_make_zone(0, [200, 100, 50], 31)]
    strip = build_final_strip(zones, width=32)
    for row in range(4):
        np.testing.assert_array_equal(strip[row, 0, :3], NEUTRAL_GREY)

def test_build_final_strip_zone_color():
    color = [200, 100, 50]
    zones = [_make_zone(0, color, 16)]
    strip = build_final_strip(zones, width=32)
    for row in range(4):
        np.testing.assert_array_almost_equal(strip[row, 16, :3], color, decimal=0)
    np.testing.assert_array_equal(strip[0], strip[1])
    np.testing.assert_array_equal(strip[0], strip[2])
    np.testing.assert_array_equal(strip[0], strip[3])

def test_build_final_strip_alpha_opaque():
    zones = [_make_zone(0, [200, 100, 50], 31)]
    strip = build_final_strip(zones, width=32)
    assert (strip[:, :, 3] == 255).all()

def test_auto_convert_returns_result():
    img = _make_two_zone_image()
    result = auto_convert(img, n_zones=2)
    assert result.remap.shape == (4, 4)
    assert result.gradient.shape == (4, 32, 4)  # now a 4px strip, not 32x32
    assert len(result.zones) == 2

def test_auto_convert_zones_sorted_by_2g_minus_r_minus_b():
    img = _make_two_zone_image()
    result = auto_convert(img, n_zones=2)
    # olive (80,160,60): 2*160-80-60=180  vs  rust (200,80,40): 2*80-200-40=-80
    # rust zone should come first (lower 2G-R-B)
    metrics = [2 * z.avg_color[1] - z.avg_color[0] - z.avg_color[2] for z in result.zones]
    assert metrics[0] <= metrics[1]

def test_auto_convert_remap_quantized():
    img = _make_two_zone_image()
    result = auto_convert(img, n_zones=2)
    unique_values = set(result.remap.flatten().tolist())
    valid = valid_remap_values(32)
    for v in unique_values:
        assert v in valid

def test_auto_convert_width64():
    img = _make_two_zone_image()
    result = auto_convert(img, n_zones=2, width=64)
    assert result.remap.shape == (4, 4)
    assert result.gradient.shape == (4, 64, 4)
    assert len(result.zones) == 2
    assert result.zones[0].gradient_column == 1
    assert result.zones[1].gradient_column == 63

def test_transparent_pixels_get_remap_zero():
    """Alpha=0 pixels should receive remap_value=0 (column 0, neutral grey)."""
    img = np.zeros((4, 4, 4), dtype=np.uint8)
    img[:, :, :3] = [200, 80, 40]      # some color
    img[:2, :, 3] = 255                 # top half opaque
    img[2:, :, 3] = 0                   # bottom half transparent
    pil = Image.fromarray(img, mode="RGBA")
    result = auto_convert(pil, n_zones=2)
    assert (result.remap[2:, :] == 0).all()

def test_n1_single_zone_maps_to_col31():
    """N=1 -> single zone should land on column 31."""
    img = np.full((4, 4, 4), 128, dtype=np.uint8)
    img[:, :, 3] = 255
    pil = Image.fromarray(img, mode="RGBA")
    result = auto_convert(pil, n_zones=1)
    assert len(result.zones) == 1
    assert result.zones[0].gradient_column == 31

def test_banded_bands_are_flat():
    """Each 4-row band must have identical rows; band 0 = grey, band 7 = zone color."""
    zones = [_make_zone(0, [200, 100, 50], 31)]
    grad = build_gradient(zones, banded=True)
    # All rows within each 4-row band are identical
    for band in range(8):
        base = band * 4
        for row in range(base + 1, base + 4):
            np.testing.assert_array_equal(grad[row], grad[base],
                err_msg=f"band {band}: row {row} != row {base}")
    # Band 0: assigned column should be neutral grey (t=0)
    np.testing.assert_array_equal(grad[0, 31, :3], NEUTRAL_GREY)
    # Band 7: assigned column should equal zone color (within ±2 for uint8 rounding)
    np.testing.assert_array_almost_equal(grad[28, 31, :3], [200, 100, 50], decimal=0)


def test_banded_same_endpoint():
    """Smooth and banded gradients must agree at row 31 (both t=1.0 = full zone color)."""
    zones = [_make_zone(0, [200, 100, 50], 16), _make_zone(1, [50, 150, 80], 31)]
    grad_smooth = build_gradient(zones, banded=False)
    grad_banded = build_gradient(zones, banded=True)
    for z in zones:
        col = z.gradient_column
        diff = np.abs(
            grad_smooth[31, col, :3].astype(int) - grad_banded[31, col, :3].astype(int)
        )
        assert diff.max() <= 2, (
            f"col {col}: smooth={grad_smooth[31, col, :3]} banded={grad_banded[31, col, :3]}"
        )


def test_banded_monotone_per_column():
    """Each assigned column must vary monotonically from grey to zone color across bands."""
    zones = [_make_zone(0, [200, 100, 50], 31)]
    grad = build_gradient(zones, banded=True)
    col = 31
    band_colors = [grad[b * 4, col, :3].astype(int) for b in range(8)]
    mono_channels = 0
    for ch in range(3):
        vals = [c[ch] for c in band_colors]
        if all(vals[i] <= vals[i + 1] + 1 for i in range(len(vals) - 1)):
            mono_channels += 1
        elif all(vals[i] >= vals[i + 1] - 1 for i in range(len(vals) - 1)):
            mono_channels += 1
    assert mono_channels >= 2, f"Only {mono_channels}/3 channels are monotone: {band_colors}"


EXAMPLES_DIR = pathlib.Path(__file__).parents[3] / "remappedexamples"

@pytest.mark.skipif(not EXAMPLES_DIR.exists(), reason="remappedexamples not found")
def test_roundtrip_semantic_reconstruction():
    """Each pixel's remap value should point to a gradient column whose color
    approximates the original source color (within +-30 per channel for 90% of pixels).
    """
    from PIL import Image as PILImage
    src_path = EXAMPLES_DIR / "CrateLarge01ColorGuide_d.png"
    src = PILImage.open(src_path)
    result = auto_convert(src, n_zones=6)

    if src.mode != "RGBA":
        src = src.convert("RGBA")
    src_rgba = np.array(src, dtype=np.int16)
    h, w = src_rgba.shape[:2]

    # Only check opaque pixels
    alpha_mask = src_rgba[:, :, 3] >= 128

    remap = result.remap.astype(np.int16)
    grad = result.gradient  # uint8 32x32 RGBA

    # For each pixel: remap_val -> gradient column -> reconstructed color
    cols = np.round(remap / 255.0 * 31).astype(np.int32)
    cols = np.clip(cols, 0, 31)

    reconstructed = grad[0, cols, :3].astype(np.int16)  # strip row 0 = full paint
    source_rgb = src_rgba[:, :, :3]

    diff = np.abs(reconstructed - source_rgb)
    max_diff_per_pixel = diff.max(axis=2)  # worst channel per pixel

    opaque_pixels = alpha_mask.sum()
    good = (max_diff_per_pixel[alpha_mask] <= 30).sum()
    agreement = good / opaque_pixels

    assert agreement > 0.90, (
        f"Semantic reconstruction agreement {agreement:.1%} below 90% "
        f"(checked {opaque_pixels} opaque pixels, +-30 per channel)"
    )


# ===========================================================================
# zones_from_remap
# ===========================================================================

def test_zones_from_remap_recovers_unique_values():
    from creation_lib.palette.remap import zones_from_remap, VALID_REMAP_VALUES
    # Build a small remap with 3 zones using valid remap values
    remap = np.zeros((4, 4), dtype=np.uint8)
    remap[0, :] = VALID_REMAP_VALUES[5]   # col 5
    remap[1, :] = VALID_REMAP_VALUES[15]  # col 15
    remap[2, :] = VALID_REMAP_VALUES[31]  # col 31
    # row 3 stays 0 (transparent)
    zones = zones_from_remap(remap)
    assert len(zones) == 3
    cols = [z.gradient_column for z in zones]
    assert cols == [5, 15, 31]
    # Pixel masks should be correct
    assert zones[0].pixel_mask.sum() == 4
    assert zones[1].pixel_mask.sum() == 4
    assert zones[2].pixel_mask.sum() == 4


# ===========================================================================
# sample_variant_colors
# ===========================================================================

def test_sample_variant_colors_averages_correctly():
    from PIL import Image
    from creation_lib.palette.remap import sample_variant_colors, PaletteZone, VALID_REMAP_VALUES
    # 4x4 remap with 2 zones
    remap = np.zeros((4, 4), dtype=np.uint8)
    remap[:2, :] = VALID_REMAP_VALUES[10]  # top 2 rows = zone 0
    remap[2:, :] = VALID_REMAP_VALUES[20]  # bottom 2 rows = zone 1
    zones = [
        PaletteZone(0, np.zeros(3), remap[:2, :] > 0, VALID_REMAP_VALUES[10], 10),
        PaletteZone(1, np.zeros(3), remap[2:, :] > 0, VALID_REMAP_VALUES[20], 20),
    ]
    # Variant image: top half red, bottom half blue
    variant = np.zeros((4, 4, 4), dtype=np.uint8)
    variant[:2, :] = [255, 0, 0, 255]
    variant[2:, :] = [0, 0, 255, 255]
    img = Image.fromarray(variant, "RGBA")
    colors = sample_variant_colors(img, remap, zones)
    assert len(colors) == 2
    np.testing.assert_allclose(colors[0], [255, 0, 0], atol=1)
    np.testing.assert_allclose(colors[1], [0, 0, 255], atol=1)


# ===========================================================================
# build_variant_gradient
# ===========================================================================

def test_build_variant_gradient_shape():
    from creation_lib.palette.remap import build_variant_gradient
    colors_a = [np.array([255, 0, 0], dtype=np.float32)]
    colors_b = [np.array([0, 255, 0], dtype=np.float32)]
    grad = build_variant_gradient([colors_a, colors_b], [16], band_height=4)
    assert grad.shape == (8, 32, 4)  # 2 variants * 4px
    assert grad.dtype == np.uint8


def test_build_variant_gradient_band_colors():
    from creation_lib.palette.remap import build_variant_gradient, NEUTRAL_GREY
    red = [np.array([255, 0, 0], dtype=np.float32)]
    blue = [np.array([0, 0, 255], dtype=np.float32)]
    grad = build_variant_gradient([red, blue], [16], band_height=4)
    # Band 0, col 16 should be red
    np.testing.assert_array_equal(grad[0, 16, :3], [255, 0, 0])
    np.testing.assert_array_equal(grad[3, 16, :3], [255, 0, 0])  # all 4 rows same
    # Band 1, col 16 should be blue
    np.testing.assert_array_equal(grad[4, 16, :3], [0, 0, 255])
    np.testing.assert_array_equal(grad[7, 16, :3], [0, 0, 255])
    # Col 0 should be neutral grey everywhere
    for row in range(8):
        np.testing.assert_array_equal(grad[row, 0, :3], NEUTRAL_GREY)


def test_build_variant_gradient_max_128():
    from creation_lib.palette.remap import build_variant_gradient
    # 40 variants * 4px = 160, should cap at 128 (32 variants)
    colors = [np.array([100, 100, 100], dtype=np.float32)]
    variants = [colors] * 40
    grad = build_variant_gradient(variants, [16], band_height=4)
    assert grad.shape[0] == 128


def test_build_variant_gradient_fills_unassigned_cols():
    from creation_lib.palette.remap import build_variant_gradient
    red = [np.array([200, 50, 50], dtype=np.float32)]
    grad = build_variant_gradient([red], [16], band_height=4)
    # Col 1 (unassigned) should copy from nearest assigned (col 16)
    np.testing.assert_array_equal(grad[0, 1, :3], grad[0, 16, :3])

def test_build_variant_gradient_width64():
    from creation_lib.palette.remap import build_variant_gradient
    red = [np.array([255, 0, 0], dtype=np.float32)]
    grad = build_variant_gradient([red], [32], band_height=4, width=64)
    assert grad.shape == (4, 64, 4)
    np.testing.assert_array_equal(grad[0, 32, :3], [255, 0, 0])

def test_build_variant_gradient_width128():
    from creation_lib.palette.remap import build_variant_gradient
    blue = [np.array([0, 0, 255], dtype=np.float32)]
    grad = build_variant_gradient([blue], [64], band_height=4, width=128)
    assert grad.shape == (4, 128, 4)
    np.testing.assert_array_equal(grad[0, 64, :3], [0, 0, 255])
