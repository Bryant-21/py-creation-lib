"""Validation tests against real Fallout 4 Pipboy SWF files.

These tests require extracted FO4 game assets. Skip if not available.
"""
from __future__ import annotations

import os
from pathlib import Path

import pytest

from creation_lib.swf.parser import parse_swf_file, parse_swf, SwfDocument
from creation_lib.swf.writer import write_swf

# Find extracted SWFs
_FO4_EXTRACTED = os.environ.get("FO4_EXTRACTED_DIR", "")
_SWF_DIR = Path(_FO4_EXTRACTED) / "Interface" / "Components" / "VaultBoys" if _FO4_EXTRACTED else None
_HAS_SWFS = _SWF_DIR and _SWF_DIR.is_dir()


@pytest.fixture
def sample_swf_paths() -> list[Path]:
    """Get up to 5 sample SWF files for testing."""
    if not _HAS_SWFS:
        pytest.skip("FO4 extracted SWFs not available")
    swfs = sorted(_SWF_DIR.rglob("*.swf"))[:5]
    if not swfs:
        pytest.skip("No SWF files found")
    return swfs


@pytest.mark.skipif(not _HAS_SWFS, reason="FO4 SWFs not available")
class TestRealSwfParsing:
    def test_parse_multiple_swfs(self, sample_swf_paths: list[Path]):
        """Parse real SWFs without errors."""
        for path in sample_swf_paths:
            doc = parse_swf_file(path)
            assert doc.header.version > 0
            assert doc.header.frame_size.width_px > 0
            assert doc.main_timeline.frame_count > 0

    def test_roundtrip_preserves_structure(self, sample_swf_paths: list[Path]):
        """Parse->write->parse preserves key document properties."""
        for path in sample_swf_paths:
            doc1 = parse_swf_file(path)
            swf_bytes = write_swf(doc1)
            doc2 = parse_swf(swf_bytes)

            assert doc2.header.version == doc1.header.version
            assert doc2.header.frame_size.width_px == doc1.header.frame_size.width_px
            assert doc2.header.frame_size.height_px == doc1.header.frame_size.height_px
            assert len(doc2.shapes) == len(doc1.shapes)
            assert doc2.main_timeline.frame_count == doc1.main_timeline.frame_count

    def test_shapes_have_valid_bounds(self, sample_swf_paths: list[Path]):
        """All parsed shapes should have non-degenerate bounds."""
        for path in sample_swf_paths:
            doc = parse_swf_file(path)
            for sid, shape in doc.shapes.items():
                bx, by, bw, bh = shape.bounds
                # Bounds should be non-zero for real shapes
                assert bw >= bx, f"Shape {sid} in {path.name} has invalid x bounds"
                assert bh >= by, f"Shape {sid} in {path.name} has invalid y bounds"

    def test_pipboy_conventions(self, sample_swf_paths: list[Path]):
        """Verify FO4 Pipboy conventions: 550x400, #333333 bg, v17."""
        for path in sample_swf_paths:
            doc = parse_swf_file(path)
            # All FO4 Pipboy SWFs should be 550x400
            assert doc.header.frame_size.width_px == 550, f"{path.name}: unexpected width"
            assert doc.header.frame_size.height_px == 400, f"{path.name}: unexpected height"
            assert doc.background_color.r == 0x33, f"{path.name}: unexpected bg color"
            assert doc.header.version == 17, f"{path.name}: unexpected version"
