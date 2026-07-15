"""Recolor DDS textures — hue shift, tint, colorize, gradient, analyze.

Handles the full pipeline through directxtex_native for DDS files.
Supports multiple recoloring strategies for different texture types.

DDS format flags:
    diffuse   -> BC7_UNORM with -srgbi (default)
    normal    -> BC5_UNORM
    emissive  -> BC7_UNORM (no srgb)
    grayscale -> BC4_UNORM
    raw       -> skip DDS conversion, output PNG
"""
from __future__ import annotations

import colorsys
import shutil
from pathlib import Path

import numpy as np
from PIL import Image

from creation_lib.dds.io import load_dds, save_image


# ── DDS conversion helpers ──────────────────────────────────────────────

DDS_FORMATS = {
    "diffuse": ("BC7_UNORM", "BC7 sRGB"),
    "emissive": ("BC7_UNORM", "BC7 linear"),
    "normal": ("BC5_UNORM", "BC5 linear"),
    "grayscale": ("BC4_UNORM", "BC4 linear"),
    "raw": ("", "PNG (no DDS conversion)"),
}


def dds_to_png(dds_path: Path, work_dir: Path) -> Path:
    """Convert DDS -> PNG using directxtex_native. Returns path to PNG."""
    work_dir.mkdir(parents=True, exist_ok=True)
    img = load_dds(str(dds_path), mode="RGBA")
    png_path = work_dir / (dds_path.stem + ".png")
    img.save(png_path)
    return png_path


def png_to_dds(png_path: Path, output_path: Path, fmt: str = "diffuse") -> Path:
    """Convert PNG -> DDS using directxtex_native. Returns path to DDS."""
    if fmt == "raw":
        # Just copy the PNG as output
        if output_path.suffix.lower() == ".dds":
            output_path = output_path.with_suffix(".png")
        shutil.copy2(png_path, output_path)
        return output_path

    dds_format, _desc = DDS_FORMATS[fmt]
    output_path.parent.mkdir(parents=True, exist_ok=True)
    with Image.open(png_path) as img:
        save_image(img.convert("RGBA"), str(output_path), format=dds_format)
    return output_path


def load_input(input_path: Path, work_dir: Path) -> Image.Image:
    """Load a DDS or PNG file as a PIL Image."""
    if input_path.suffix.lower() == ".dds":
        png = dds_to_png(input_path, work_dir)
        return Image.open(png).convert("RGBA")
    else:
        return Image.open(input_path).convert("RGBA")


def save_output(img: Image.Image, output_path: Path, work_dir: Path, fmt: str):
    """Save a PIL Image as DDS (via PNG intermediate) or PNG."""
    png_path = work_dir / "output.png"
    img.save(png_path)
    png_to_dds(png_path, output_path, fmt)


def parse_color(s: str) -> tuple[int, int, int]:
    """Parse 'R,G,B' string (0-255) or '#RRGGBB' hex."""
    s = s.strip()
    if s.startswith("#"):
        s = s[1:]
        return (int(s[0:2], 16), int(s[2:4], 16), int(s[4:6], 16))
    parts = [int(x.strip()) for x in s.split(",")]
    if len(parts) != 3:
        raise ValueError(f"Color must be R,G,B or #RRGGBB, got: {s}")
    return (parts[0], parts[1], parts[2])


# ── Recolor operations ──────────────────────────────────────────────────

def hue_shift(img: Image.Image, degrees: float) -> Image.Image:
    """Rotate all pixel hues by the given number of degrees.

    Positive = counterclockwise on the color wheel.
    Examples: blue(240)->purple(280) = +40, green(120)->blue(240) = +120
    """
    arr = np.array(img, dtype=np.float32)
    rgb = arr[:, :, :3] / 255.0
    alpha = arr[:, :, 3:4]

    # Vectorized RGB->HSV
    r, g, b = rgb[:,:,0], rgb[:,:,1], rgb[:,:,2]
    maxc = np.maximum(np.maximum(r, g), b)
    minc = np.minimum(np.minimum(r, g), b)
    diff = maxc - minc

    # Hue
    h = np.zeros_like(maxc)
    mask = diff > 0
    rm = mask & (maxc == r)
    gm = mask & (maxc == g) & ~rm
    bm = mask & (maxc == b) & ~rm & ~gm
    h[rm] = ((g[rm] - b[rm]) / diff[rm]) % 6
    h[gm] = ((b[gm] - r[gm]) / diff[gm]) + 2
    h[bm] = ((r[bm] - g[bm]) / diff[bm]) + 4
    h = h / 6.0  # Normalize to 0-1

    # Saturation
    s = np.where(maxc > 0, diff / maxc, 0)
    v = maxc

    # Shift hue
    h = (h + degrees / 360.0) % 1.0

    # HSV->RGB (vectorized)
    i = (h * 6.0).astype(np.int32)
    f = h * 6.0 - i
    p = v * (1 - s)
    q = v * (1 - f * s)
    t = v * (1 - (1 - f) * s)

    i = i % 6
    out = np.zeros_like(rgb)
    for idx, (c0, c1, c2) in enumerate([(v,t,p),(q,v,p),(p,v,t),(p,q,v),(t,p,v),(v,p,q)]):
        m = i == idx
        out[:,:,0][m] = c0[m]
        out[:,:,1][m] = c1[m]
        out[:,:,2][m] = c2[m]

    out = np.clip(out * 255.0, 0, 255)
    result = np.concatenate([out, alpha], axis=2).astype(np.uint8)
    return Image.fromarray(result, "RGBA")


def tint(img: Image.Image, color: tuple[int, int, int]) -> Image.Image:
    """Multiply each pixel's RGB by the given color (normalized).

    Best for white/gray base textures that need a solid color applied.
    Only processes pixels with alpha > 0 to preserve transparent regions.
    """
    arr = np.array(img, dtype=np.float32)
    factors = np.array([color[0]/255.0, color[1]/255.0, color[2]/255.0], dtype=np.float32)
    # Only recolor visible pixels — leave transparent pixels untouched
    visible = arr[:, :, 3] > 0
    arr[:, :, :3][visible] *= factors
    arr = np.clip(arr, 0, 255).astype(np.uint8)
    return Image.fromarray(arr, "RGBA")


def colorize(img: Image.Image, color: tuple[int, int, int]) -> Image.Image:
    """Force all pixels to the hue and saturation of the target color,
    preserving each pixel's original luminance. This gives a uniform
    color wash while keeping detail/shading.
    """
    target_h, target_s, _ = colorsys.rgb_to_hsv(color[0]/255, color[1]/255, color[2]/255)

    arr = np.array(img, dtype=np.float32)
    rgb = arr[:, :, :3] / 255.0
    alpha = arr[:, :, 3:4]

    # Compute luminance (perceived brightness)
    lum = 0.299 * rgb[:,:,0] + 0.587 * rgb[:,:,1] + 0.114 * rgb[:,:,2]

    # Convert target hue+sat + per-pixel luminance -> RGB
    # Use HSV with target H and S, pixel V = luminance
    h_arr = np.full_like(lum, target_h)
    s_arr = np.full_like(lum, target_s)
    v_arr = lum

    i = (h_arr * 6.0).astype(np.int32)
    f = h_arr * 6.0 - i
    p = v_arr * (1 - s_arr)
    q = v_arr * (1 - f * s_arr)
    t = v_arr * (1 - (1 - f) * s_arr)

    i = i % 6
    out = np.zeros_like(rgb)
    for idx, (c0, c1, c2) in enumerate([(v_arr,t,p),(q,v_arr,p),(p,v_arr,t),(p,q,v_arr),(t,p,v_arr),(v_arr,p,q)]):
        m = i == idx
        out[:,:,0][m] = c0[m]
        out[:,:,1][m] = c1[m]
        out[:,:,2][m] = c2[m]

    out = np.clip(out * 255.0, 0, 255)
    result = np.concatenate([out, alpha], axis=2).astype(np.uint8)
    return Image.fromarray(result, "RGBA")


def make_gradient(color_from: tuple[int, int, int], color_to: tuple[int, int, int],
                  width: int = 256, height: int = 16) -> Image.Image:
    """Generate a horizontal gradient strip from one color to another.

    Used for grayscale-to-palette lookup textures.
    """
    arr = np.zeros((height, width, 4), dtype=np.uint8)
    for x in range(width):
        t = x / (width - 1)
        r = int(color_from[0] * (1 - t) + color_to[0] * t)
        g = int(color_from[1] * (1 - t) + color_to[1] * t)
        b = int(color_from[2] * (1 - t) + color_to[2] * t)
        arr[:, x] = [r, g, b, 255]
    return Image.fromarray(arr, "RGBA")


def analyze(img: Image.Image) -> dict:
    """Analyze dominant colors and hue distribution of a texture."""
    arr = np.array(img, dtype=np.float32)
    rgb = arr[:, :, :3]
    alpha = arr[:, :, 3]

    # Only analyze non-transparent pixels
    mask = alpha > 10
    if not mask.any():
        return {"error": "All pixels are transparent"}

    pixels = rgb[mask]  # shape: (N, 3)
    avg_color = pixels.mean(axis=0).astype(int).tolist()

    # Hue distribution on filtered pixels
    r, g, b = pixels[:,0]/255, pixels[:,1]/255, pixels[:,2]/255
    maxc = np.maximum(np.maximum(r, g), b)
    minc = np.minimum(np.minimum(r, g), b)
    diff = maxc - minc
    saturated = diff > 0.05  # Only count pixels with some color

    hues = np.zeros_like(maxc)
    rm = saturated & (maxc == r)
    gm = saturated & (maxc == g) & ~rm
    bm = saturated & (maxc == b) & ~rm & ~gm
    hues[rm] = (((g[rm] - b[rm]) / diff[rm]) % 6) * 60
    hues[gm] = (((b[gm] - r[gm]) / diff[gm]) + 2) * 60
    hues[bm] = (((r[bm] - g[bm]) / diff[bm]) + 4) * 60

    # Bucket hues
    hue_names = ["Red", "Orange", "Yellow", "Green", "Cyan", "Blue", "Purple", "Magenta"]
    hue_ranges = [(0,22.5), (22.5,45), (45,75), (75,165), (165,195), (195,265), (265,325), (325,360)]
    sat_hues = hues[saturated]
    total_sat = len(sat_hues) if len(sat_hues) > 0 else 1

    distribution = {}
    for name, (lo, hi) in zip(hue_names, hue_ranges):
        count = np.sum((sat_hues >= lo) & (sat_hues < hi))
        pct = count / total_sat * 100
        if pct > 1:
            distribution[name] = f"{pct:.0f}%"

    avg_sat = float(np.mean(diff[saturated])) if saturated.any() else 0
    avg_val = float(np.mean(maxc))
    gray_pct = np.sum(~saturated) / len(diff) * 100

    return {
        "dimensions": f"{img.width}x{img.height}",
        "avg_color_rgb": avg_color,
        "avg_saturation": f"{avg_sat:.2f}",
        "avg_brightness": f"{avg_val:.2f}",
        "gray_pixels": f"{gray_pct:.0f}%",
        "hue_distribution": distribution,
        "suggestion": (
            "Use 'tint' (gray base)" if gray_pct > 60
            else "Use 'hue-shift' or 'colorize' (colored base)"
        ),
    }
