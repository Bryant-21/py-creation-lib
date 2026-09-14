"""Raster-to-vector tracing via vtracer.

Pipeline: image -> threshold B&W -> vtracer -> SVG paths -> ShapeDefs.
"""
from __future__ import annotations

from dataclasses import dataclass

import numpy as np

from creation_lib.swf.shapes import ShapeDef
from creation_lib.swf.svg_io import svg_to_shapes


@dataclass
class TraceSettings:
    threshold: int = 128       # B&W threshold (0-255)
    min_area: int = 25         # minimum path area in pixels
    curve_tolerance: float = 1.0
    simplify_tolerance: float = 1.0
    color_precision: int = 6
    mode: str = "binary"       # "binary" for B&W, "color" for multi-color


def trace_image(
    image: np.ndarray,
    settings: TraceSettings | None = None,
) -> list[ShapeDef]:
    """Trace an RGBA uint8 (H, W, 4) image to SWF shapes."""
    import io
    import vtracer
    from PIL import Image

    if settings is None:
        settings = TraceSettings()

    h, w = image.shape[:2]

    # Convert to grayscale and threshold
    if image.ndim == 3:
        if image.shape[2] == 4:
            gray = np.mean(image[:, :, :3], axis=2)
        else:
            gray = np.mean(image, axis=2)
    else:
        gray = image.astype(float)

    # Apply threshold
    bw = np.where(gray >= settings.threshold, 255, 0).astype(np.uint8)

    # Check if there's anything to trace
    if np.all(bw == 0):
        return []

    # Convert to RGBA PNG bytes for vtracer
    rgba = np.zeros((h, w, 4), dtype=np.uint8)
    rgba[:, :, 0] = bw
    rgba[:, :, 1] = bw
    rgba[:, :, 2] = bw
    rgba[:, :, 3] = 255

    pil_img = Image.fromarray(rgba, "RGBA")
    buf = io.BytesIO()
    pil_img.save(buf, format="PNG")
    png_bytes = buf.getvalue()

    # Trace to SVG
    svg_str = vtracer.convert_raw_image_to_svg(
        png_bytes,
        colormode="binary",
        filter_speckle=settings.min_area,
        path_precision=8,
    )

    if not svg_str or "<path" not in svg_str:
        return []

    return svg_to_shapes(svg_str)


def trace_image_file(path: str, settings: TraceSettings | None = None) -> list[ShapeDef]:
    """Trace an image file (PNG, DDS, JPG, BMP) to SWF shapes."""
    from PIL import Image

    img = Image.open(path).convert("RGBA")
    return trace_image(np.array(img), settings)
