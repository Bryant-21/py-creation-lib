"""Tests for raster-to-vector tracing."""
from __future__ import annotations

import pytest
import numpy as np

from creation_lib.swf.trace import trace_image, TraceSettings


class TestTraceImage:
    def test_simple_white_rect(self):
        """A solid white rectangle on black background should trace to ~1 shape."""
        # Create 100x100 image: white rect in center
        img = np.zeros((100, 100, 4), dtype=np.uint8)
        img[20:80, 20:80] = [255, 255, 255, 255]

        settings = TraceSettings(threshold=128, min_area=10)
        shapes = trace_image(img, settings)
        # vtracer in binary mode traces the boundary between B&W regions
        assert len(shapes) >= 1
        assert len(shapes[0].fill_styles) >= 1
        assert len(shapes[0].records) >= 3  # at least moveto + edges + end

    def test_threshold_filtering(self):
        """Low-contrast areas below threshold should be ignored."""
        img = np.full((50, 50, 4), 64, dtype=np.uint8)  # dark gray
        settings = TraceSettings(threshold=128)
        shapes = trace_image(img, settings)
        assert len(shapes) == 0  # nothing above threshold
