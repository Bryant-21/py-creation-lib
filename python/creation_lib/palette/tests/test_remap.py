import numpy as np
import pytest
from creation_lib.palette.remap import (
    VALID_REMAP_VALUES, valid_remap_values, build_gradient, build_final_strip,
    PaletteZone, NEUTRAL_GREY, assign_columns, build_remap_texture, auto_convert,
)
from PIL import Image


@pytest.mark.parametrize("width", [32, 64, 128])
def test_valid_remap_values_formula(width):
    vals = valid_remap_values(width)
    expected = [round(col / (width - 1) * 255) for col in range(width)]
    assert vals == expected
    assert len(vals) == width
    assert vals[0] == 0
    assert vals[-1] == 255


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


def test_build_gradient_basic_properties():
    """Shape/dtype/alpha, col0=grey, row0=grey and last row=zone color, all in one build."""
    zones = [_make_zone(0, [200, 100, 50], 16), _make_zone(1, [50, 150, 80], 31)]
    grad = build_gradient(zones)
    assert grad.shape == (32, 32, 4)
    assert grad.dtype == np.uint8
    assert (grad[:, :, 3] == 255).all()
    for row in range(32):
        np.testing.assert_array_equal(grad[row, 0, :3], NEUTRAL_GREY)
    for col in range(1, 32):
        np.testing.assert_array_equal(grad[0, col, :3], NEUTRAL_GREY)
    np.testing.assert_array_almost_equal(grad[31, 31, :3], [50, 150, 80], decimal=0)


@pytest.mark.parametrize("width", [64, 128])
def test_gradient_matches_width32_behavior_at_other_widths(width):
    color = [200, 100, 50]
    last_col = width - 1
    zones = [_make_zone(0, color, last_col, width=width)]
    grad = build_gradient(zones, width=width)
    assert grad.shape == (32, width, 4)
    for row in range(32):
        np.testing.assert_array_equal(grad[row, 0, :3], NEUTRAL_GREY)
    np.testing.assert_array_almost_equal(grad[31, last_col, :3], color, decimal=0)


def test_gradient_nearest_column_tiebreak_goes_to_lower():
    # zones at cols 1 and 31 — col 16 is equidistant; should copy from col 1 (lower)
    zones = [_make_zone(0, [200, 50, 50], 1), _make_zone(1, [50, 200, 50], 31)]
    grad = build_gradient(zones)
    # col 16: dist to 1 = 15, dist to 31 = 15 — tie → copy col 1
    np.testing.assert_array_equal(grad[:, 16, :3], grad[:, 1, :3])


def test_banded_bands_are_flat_and_monotone():
    """Each 4-row band is flat; band 0=grey, band 7=zone color; values move
    monotonically band-to-band; smooth and banded agree at the endpoint."""
    zones = [_make_zone(0, [200, 100, 50], 16), _make_zone(1, [50, 150, 80], 31)]
    grad = build_gradient(zones, banded=True)
    for band in range(8):
        base = band * 4
        for row in range(base + 1, base + 4):
            np.testing.assert_array_equal(grad[row], grad[base],
                err_msg=f"band {band}: row {row} != row {base}")
    np.testing.assert_array_equal(grad[0, 31, :3], NEUTRAL_GREY)
    np.testing.assert_array_almost_equal(grad[28, 31, :3], [50, 150, 80], decimal=0)

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

    grad_smooth = build_gradient(zones, banded=False)
    for z in zones:
        c = z.gradient_column
        diff = np.abs(grad_smooth[31, c, :3].astype(int) - grad[31, c, :3].astype(int))
        assert diff.max() <= 2, f"col {c}: smooth={grad_smooth[31, c, :3]} banded={grad[31, c, :3]}"


def test_zones_from_remap_recovers_unique_values():
    from creation_lib.palette.remap import zones_from_remap
    # 4x4 remap with 3 zones at various widths/columns; row 3 stays 0 (transparent).
    remap = np.zeros((4, 4), dtype=np.uint8)
    remap[0, :] = VALID_REMAP_VALUES[5]
    remap[1, :] = VALID_REMAP_VALUES[15]
    remap[2, :] = VALID_REMAP_VALUES[31]
    zones = zones_from_remap(remap)
    assert len(zones) == 3
    assert [z.gradient_column for z in zones] == [5, 15, 31]
    assert all(z.pixel_mask.sum() == 4 for z in zones)

    vals64 = valid_remap_values(64)
    remap64 = np.zeros((4, 4), dtype=np.uint8)
    remap64[0, :] = vals64[10]
    remap64[1, :] = vals64[30]
    remap64[2, :] = vals64[63]
    zones64 = zones_from_remap(remap64, width=64)
    assert [z.gradient_column for z in zones64] == [10, 30, 63]


@pytest.mark.parametrize(("width", "last_col"), [(32, 31), (64, 63), (128, 127)])
def test_assign_columns_two_zones(width, last_col):
    # 2 zones → columns 1 and last (zones span 1..last; column 0 is reserved fixed grey)
    colors = [np.array([100.0, 50.0, 30.0]), np.array([50.0, 150.0, 80.0])]
    zones = assign_columns(colors, width=width)
    assert zones[0].gradient_column == 1
    assert zones[1].gradient_column == last_col
    vals = valid_remap_values(width)
    for z in zones:
        assert z.remap_value == vals[z.gradient_column]


@pytest.mark.parametrize(("width", "last_col"), [(32, 30), (128, 126)])
def test_assign_columns_four_zones(width, last_col):
    # 4 zones → evenly spaced across 1..width-1
    colors = [np.array([float(i * 50), 0.0, 0.0]) for i in range(4)]
    zones = assign_columns(colors, width=width)
    expected_cols = [round(i / 3 * last_col) + 1 for i in range(4)]
    assert [z.gradient_column for z in zones] == expected_cols
    if width == 32:
        # remap_value always matches the column's valid remap table entry.
        for z in zones:
            assert z.remap_value == VALID_REMAP_VALUES[z.gradient_column]


@pytest.mark.parametrize(("width", "last_col"), [(32, 31), (128, 127)])
def test_assign_columns_single_zone(width, last_col):
    # N=1 → single zone lands on the last column (spec explicit: single zone = full paint)
    colors = [np.array([100.0, 100.0, 100.0])]
    zones = assign_columns(colors, width=width)
    assert zones[0].gradient_column == last_col


def test_build_remap_texture():
    h, w = 4, 4
    label_map = np.zeros((h, w), dtype=np.int32)
    label_map[2:, :] = 1
    colors = [np.array([30.0, 100.0, 50.0]), np.array([180.0, 60.0, 40.0])]
    zones = assign_columns(colors)
    zones[0].pixel_mask = label_map == 0
    zones[1].pixel_mask = label_map == 1
    remap = build_remap_texture((h, w), label_map, zones)
    assert remap.shape == (h, w)
    assert remap.dtype == np.uint8
    assert (remap[:2, :] == zones[0].remap_value).all()
    assert (remap[2:, :] == zones[1].remap_value).all()


def _make_two_zone_image():
    """4x4 RGBA image: top half rust (200,80,40), bottom half olive (80,160,60), fully opaque."""
    img = np.zeros((4, 4, 4), dtype=np.uint8)
    img[:2, :] = [200, 80, 40, 255]
    img[2:, :] = [80, 160, 60, 255]
    return Image.fromarray(img, mode="RGBA")


def test_build_final_strip_basic_properties():
    """Shape/dtype/alpha, col0=grey, and each row identical with the zone color."""
    color = [200, 100, 50]
    zones = [_make_zone(0, color, 16), _make_zone(1, [80, 160, 60], 31)]
    strip = build_final_strip(zones, width=32)
    assert strip.shape == (4, 32, 4)
    assert strip.dtype == np.uint8
    assert (strip[:, :, 3] == 255).all()
    for row in range(4):
        np.testing.assert_array_equal(strip[row, 0, :3], NEUTRAL_GREY)
        np.testing.assert_array_almost_equal(strip[row, 16, :3], color, decimal=0)
    np.testing.assert_array_equal(strip[0], strip[1])
    np.testing.assert_array_equal(strip[0], strip[2])
    np.testing.assert_array_equal(strip[0], strip[3])


@pytest.mark.parametrize("width", [64, 128])
def test_build_final_strip_shape_other_widths(width):
    color = [200, 100, 50]
    last_col = width - 1
    zones = [PaletteZone(0, np.array(color, dtype=np.float32),
             np.zeros((0, 0), dtype=bool), valid_remap_values(width)[last_col], last_col)]
    strip = build_final_strip(zones, width=width)
    assert strip.shape == (4, width, 4)


def test_auto_convert_basic_contract():
    """Shape/zone-count, sort order, remap quantization, all from one conversion."""
    img = _make_two_zone_image()
    result = auto_convert(img, n_zones=2)
    assert result.remap.shape == (4, 4)
    assert result.gradient.shape == (4, 32, 4)  # a 4px strip, not 32x32
    assert len(result.zones) == 2
    # olive (80,160,60): 2*160-80-60=180  vs  rust (200,80,40): 2*80-200-40=-80
    # rust zone should come first (lower 2G-R-B)
    metrics = [2 * z.avg_color[1] - z.avg_color[0] - z.avg_color[2] for z in result.zones]
    assert metrics[0] <= metrics[1]
    valid = set(valid_remap_values(32))
    assert set(result.remap.flatten().tolist()) <= valid

    result64 = auto_convert(img, n_zones=2, width=64)
    assert result64.remap.shape == (4, 4)
    assert result64.gradient.shape == (4, 64, 4)
    assert len(result64.zones) == 2
    assert result64.zones[0].gradient_column == 1
    assert result64.zones[1].gradient_column == 63


def test_auto_convert_edge_cases_transparency_and_single_zone():
    # Alpha=0 pixels should receive remap_value=0 (column 0, neutral grey).
    img = np.zeros((4, 4, 4), dtype=np.uint8)
    img[:, :, :3] = [200, 80, 40]      # some color
    img[:2, :, 3] = 255                 # top half opaque
    img[2:, :, 3] = 0                   # bottom half transparent
    pil = Image.fromarray(img, mode="RGBA")
    result = auto_convert(pil, n_zones=2)
    assert (result.remap[2:, :] == 0).all()

    # N=1 -> single zone should land on the last column.
    img2 = np.full((4, 4, 4), 128, dtype=np.uint8)
    img2[:, :, 3] = 255
    pil2 = Image.fromarray(img2, mode="RGBA")
    result2 = auto_convert(pil2, n_zones=1)
    assert len(result2.zones) == 1
    assert result2.zones[0].gradient_column == 31


def test_sample_variant_colors_averages_correctly():
    from creation_lib.palette.remap import sample_variant_colors
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


def test_build_variant_gradient_basic_properties():
    """Shape/dtype, band colors, col0=grey, unassigned-column fill and the
    32-variant (128px) cap, all from a couple of small builds."""
    from creation_lib.palette.remap import build_variant_gradient
    red = [np.array([255, 0, 0], dtype=np.float32)]
    blue = [np.array([0, 0, 255], dtype=np.float32)]
    grad = build_variant_gradient([red, blue], [16], band_height=4)
    assert grad.shape == (8, 32, 4)
    assert grad.dtype == np.uint8
    np.testing.assert_array_equal(grad[0, 16, :3], [255, 0, 0])
    np.testing.assert_array_equal(grad[3, 16, :3], [255, 0, 0])
    np.testing.assert_array_equal(grad[4, 16, :3], [0, 0, 255])
    np.testing.assert_array_equal(grad[7, 16, :3], [0, 0, 255])
    for row in range(8):
        np.testing.assert_array_equal(grad[row, 0, :3], NEUTRAL_GREY)
    # Col 1 (unassigned) should copy from the nearest assigned column (16).
    np.testing.assert_array_equal(grad[0, 1, :3], grad[0, 16, :3])

    # 40 variants * 4px = 160, should cap at 128 (32 variants).
    variants = [[np.array([100, 100, 100], dtype=np.float32)]] * 40
    capped = build_variant_gradient(variants, [16], band_height=4)
    assert capped.shape[0] == 128


@pytest.mark.parametrize(("width", "col"), [(64, 32), (128, 64)])
def test_build_variant_gradient_other_widths(width, col):
    from creation_lib.palette.remap import build_variant_gradient
    red = [np.array([255, 0, 0], dtype=np.float32)]
    grad = build_variant_gradient([red], [col], band_height=4, width=width)
    assert grad.shape == (4, width, 4)
    np.testing.assert_array_equal(grad[0, col, :3], [255, 0, 0])
