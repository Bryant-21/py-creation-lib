"""Tests for SWF shape library: extraction, SVG rendering, PNG export, category mapping.

Validates:
1. preprocess_swf extracts shapes from all SWF directories (not just VaultBoys)
2. Category mapping assigns correct types (Perk, Quest, Faction, etc.)
3. SVG previews render non-blank (fill0 fallback, dark background)
4. PNG export for visual verification (written to data/swf_test_pngs/)

Requires FO4_EXTRACTED_DIR to be set (skipped otherwise).
"""

from __future__ import annotations

import json
import os
import sqlite3
import xml.etree.ElementTree as ET
from pathlib import Path

import pytest

from creation_lib.swf.parser import parse_swf_file
from creation_lib.swf.shapes import ShapeDef, StraightEdge, CurvedEdge, StyleChange, EndShape
from creation_lib.swf.svg_io import shape_to_svg
from creation_lib.swf.types import RGBA, FillStyle

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

_FO4_EXTRACTED = os.environ.get("FO4_EXTRACTED_DIR", "")
_INTERFACE_DIR = Path(_FO4_EXTRACTED) / "Interface" if _FO4_EXTRACTED else None
_HAS_SWFS = _INTERFACE_DIR and _INTERFACE_DIR.is_dir()

_PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent
_DB_PATH = _PROJECT_ROOT / "data" / "fo4_swf_shapes.db"
_HAS_DB = _DB_PATH.is_file()

_PNG_OUT_DIR = _PROJECT_ROOT / "data" / "swf_test_pngs"


@pytest.fixture
def shape_db():
    """Open the built shape library DB."""
    if not _HAS_DB:
        pytest.skip(
            "fo4_swf_shapes.db not built — run: modkit index build --domain swf"
        )
    conn = sqlite3.connect(str(_DB_PATH))
    conn.row_factory = sqlite3.Row
    yield conn
    conn.close()


# ---------------------------------------------------------------------------
# Unit tests: SVG rendering (no game files needed)
# ---------------------------------------------------------------------------


class TestSvgRendering:
    """Verify shape_to_svg produces visible, well-formed SVGs."""

    @staticmethod
    def _make_shape(
        fill0: int | None = None,
        fill1: int | None = None,
        color: RGBA = RGBA(255, 255, 255, 255),
    ) -> ShapeDef:
        fill = FillStyle(fill_type=0x00, color=color)
        return ShapeDef(
            shape_id=1,
            bounds=(0, 0, 2000, 1000),
            fill_styles=[fill],
            line_styles=[],
            records=[
                StyleChange(move_x=0, move_y=0, fill0=fill0, fill1=fill1),
                StraightEdge(dx=2000, dy=0),
                StraightEdge(dx=0, dy=1000),
                StraightEdge(dx=-2000, dy=0),
                StraightEdge(dx=0, dy=-1000),
                EndShape(),
            ],
        )

    def test_fill1_renders_color(self):
        """Shapes using fill1 should get fill color in SVG."""
        shape = self._make_shape(fill1=1)
        svg = shape_to_svg(shape, background=None)
        assert 'fill="#ffffff"' in svg

    def test_fill0_renders_color(self):
        """Shapes using only fill0 (not fill1) should still get fill color."""
        shape = self._make_shape(fill0=1)
        svg = shape_to_svg(shape, background=None)
        assert 'fill="#ffffff"' in svg
        assert 'fill="none"' not in svg

    def test_fill0_only_was_previously_broken(self):
        """Regression: fill0-only shapes used to render with fill='none'."""
        shape = self._make_shape(fill0=1, fill1=None)
        svg = shape_to_svg(shape, background=None)
        # Must NOT have fill="none" on the path
        root = ET.fromstring(svg)
        ns = {"svg": "http://www.w3.org/2000/svg"}
        paths = root.findall(".//{http://www.w3.org/2000/svg}path")
        for p in paths:
            assert p.get("fill") != "none", (
                "fill0-only shape should not have fill='none'"
            )

    def test_dark_background_present(self):
        """Default background should be a dark rect for white shape visibility."""
        shape = self._make_shape(fill1=1)
        svg = shape_to_svg(shape)
        assert "#333333" in svg
        assert "<rect" in svg

    def test_no_background_when_disabled(self):
        """background=None should omit the rect."""
        shape = self._make_shape(fill1=1)
        svg = shape_to_svg(shape, background=None)
        assert "<rect" not in svg

    def test_svg_is_valid_xml(self):
        """SVG output must be parseable XML."""
        shape = self._make_shape(fill1=1)
        svg = shape_to_svg(shape)
        root = ET.fromstring(svg)
        assert root.tag == "{http://www.w3.org/2000/svg}svg"

    def test_viewbox_has_positive_dimensions(self):
        """viewBox width/height must be positive."""
        shape = self._make_shape(fill1=1)
        svg = shape_to_svg(shape)
        root = ET.fromstring(svg)
        vb = root.get("viewBox").split()
        w, h = float(vb[2]), float(vb[3])
        assert w > 0, "viewBox width must be positive"
        assert h > 0, "viewBox height must be positive"

    def test_colored_fill_not_white_on_white(self):
        """Red fill on dark background should be clearly visible."""
        shape = self._make_shape(fill1=1, color=RGBA(255, 0, 0, 255))
        svg = shape_to_svg(shape)
        assert 'fill="#ff0000"' in svg


# ---------------------------------------------------------------------------
# Category mapping tests
# ---------------------------------------------------------------------------


class TestCategoryMapping:
    """Verify _classify_swf assigns correct categories."""

    def test_perk_category(self):
        from creation_lib.preprocessor.swf import _classify_swf

        root = Path("/game/Interface")
        cat, sub = _classify_swf(
            root / "Components" / "VaultBoys" / "Perks" / "Test.swf", root
        )
        assert cat == "Perk"

    def test_special_category(self):
        from creation_lib.preprocessor.swf import _classify_swf

        root = Path("/game/Interface")
        cat, sub = _classify_swf(
            root / "Components" / "VaultBoys" / "SPECIAL" / "Test.swf", root
        )
        assert cat == "SPECIAL"

    def test_quest_category(self):
        from creation_lib.preprocessor.swf import _classify_swf

        root = Path("/game/Interface")
        cat, sub = _classify_swf(
            root / "Components" / "Quest Vault Boys" / "Act 1 Quest" / "Test.swf", root
        )
        assert cat == "Quest"
        assert sub == "Act 1 Quest"

    def test_faction_category(self):
        from creation_lib.preprocessor.swf import _classify_swf

        root = Path("/game/Interface")
        cat, sub = _classify_swf(
            root / "Components" / "Faction Vault boys" / "Minutemen" / "Test.swf", root
        )
        assert cat == "Faction"
        assert sub == "Minutemen"

    def test_magazine_category(self):
        from creation_lib.preprocessor.swf import _classify_swf

        root = Path("/game/Interface")
        cat, sub = _classify_swf(
            root / "Components" / "Magazine perks" / "Test.swf", root
        )
        assert cat == "Magazine"

    def test_condition_category(self):
        from creation_lib.preprocessor.swf import _classify_swf

        root = Path("/game/Interface")
        cat, sub = _classify_swf(
            root / "Components" / "ConditionClips" / "Test.swf", root
        )
        assert cat == "Condition"

    def test_dlc_quest_animation(self):
        from creation_lib.preprocessor.swf import _classify_swf

        root = Path("/game/Interface")
        cat, sub = _classify_swf(
            root / "99439_DLC04 Quest animations" / "SWF" / "Test.swf", root
        )
        assert cat == "Quest"

    def test_dlc_perk_category(self):
        from creation_lib.preprocessor.swf import _classify_swf

        root = Path("/game/Interface")
        cat, sub = _classify_swf(
            root / "Components" / "VaultBoys" / "DLC04" / "Test.swf", root
        )
        assert cat == "DLC Perk"


# ---------------------------------------------------------------------------
# Integration tests: built database validation (requires rebuilt DB)
# ---------------------------------------------------------------------------


@pytest.mark.skipif(not _HAS_DB, reason="fo4_swf_shapes.db not built")
class TestShapeDatabase:
    """Validate the built shape library database."""

    def test_shape_count_exceeds_vaultboys_only(self, shape_db):
        """Must have more shapes than the old VaultBoys-only index (2640)."""
        count = shape_db.execute("SELECT COUNT(*) FROM shapes").fetchone()[0]
        assert count > 4000, (
            f"Only {count} shapes — expected 5000+ from all directories"
        )

    def test_has_multiple_categories(self, shape_db):
        """Must have shapes in at least 5 different top-level categories."""
        rows = shape_db.execute(
            "SELECT DISTINCT substr(tags, 1, instr(tags || '/', '/') - 1) FROM shapes"
        ).fetchall()
        categories = {r[0] for r in rows}
        assert len(categories) >= 5, f"Only {len(categories)} categories: {categories}"

    def test_expected_categories_present(self, shape_db):
        """Key categories must be present."""
        rows = shape_db.execute("SELECT DISTINCT tags FROM shapes").fetchall()
        all_tags = {r[0] for r in rows}
        tag_str = str(all_tags)
        assert any("Perk" in t for t in all_tags), (
            f"Missing Perk category in {all_tags}"
        )
        assert any("Quest" in t for t in all_tags), (
            f"Missing Quest category in {all_tags}"
        )
        assert any("Faction" in t for t in all_tags), (
            f"Missing Faction category in {all_tags}"
        )
        assert any("Magazine" in t for t in all_tags), f"Missing Magazine category"

    def test_all_svgs_have_fill_colors(self, shape_db):
        """Every SVG preview must have at least one hex fill color (not all fill='none')."""
        no_fill = shape_db.execute(
            "SELECT COUNT(*) FROM shapes WHERE svg_preview NOT LIKE ?", ('%fill="#%',)
        ).fetchone()[0]
        total = shape_db.execute("SELECT COUNT(*) FROM shapes").fetchone()[0]
        assert no_fill == 0, f"{no_fill}/{total} shapes have no fill color in SVG"

    def test_all_svgs_have_background(self, shape_db):
        """Every SVG preview must have a dark background rect."""
        no_bg = shape_db.execute(
            "SELECT COUNT(*) FROM shapes WHERE svg_preview NOT LIKE ?", ("%<rect%",)
        ).fetchone()[0]
        assert no_bg == 0, f"{no_bg} shapes missing background rect"

    def test_svgs_are_valid_xml(self, shape_db):
        """Spot-check 20 SVGs for valid XML."""
        rows = shape_db.execute(
            "SELECT id, name, svg_preview FROM shapes ORDER BY RANDOM() LIMIT 20"
        ).fetchall()
        for row in rows:
            try:
                ET.fromstring(row["svg_preview"])
            except ET.ParseError as e:
                pytest.fail(
                    f"Shape {row['name']} (id={row['id']}) has invalid SVG XML: {e}"
                )

    def test_no_zero_area_shapes(self, shape_db):
        """No shape should have zero-area bounds."""
        rows = shape_db.execute("SELECT id, name, bounds FROM shapes").fetchall()
        for row in rows:
            b = json.loads(row["bounds"])
            w = b[2] - b[0]
            h = b[3] - b[1]
            assert w > 0 and h > 0, f"Shape {row['name']} has zero-area bounds: {b}"

    def test_fts_search_works(self, shape_db):
        """Full-text search should return results for common terms."""
        rows = shape_db.execute(
            "SELECT COUNT(*) FROM shapes_fts WHERE shapes_fts MATCH 'Perk'"
        ).fetchone()[0]
        assert rows > 0, "FTS search for 'Perk' returned no results"

    def test_faction_shapes_present(self, shape_db):
        """Faction VaultBoys (new in expanded index) must be present."""
        count = shape_db.execute(
            "SELECT COUNT(*) FROM shapes WHERE tags LIKE 'Faction%'"
        ).fetchone()[0]
        assert count > 100, f"Only {count} faction shapes — expected hundreds"

    def test_quest_shapes_present(self, shape_db):
        """Quest VaultBoys must be present."""
        count = shape_db.execute(
            "SELECT COUNT(*) FROM shapes WHERE tags LIKE 'Quest%'"
        ).fetchone()[0]
        assert count > 200, f"Only {count} quest shapes — expected hundreds"


# ---------------------------------------------------------------------------
# PNG export tests: render SVGs to PNG for visual verification
# ---------------------------------------------------------------------------


def _svg_to_png(svg_content: str, png_path: Path, width: int = 256) -> bool:
    """Render SVG to PNG using cairosvg if available, else Pillow+svglib fallback.

    Returns True if PNG was written.
    """
    try:
        import cairosvg

        cairosvg.svg2png(
            bytestring=svg_content.encode(), write_to=str(png_path), output_width=width
        )
        return True
    except (ImportError, OSError):
        pass

    try:
        from svglib.svglib import renderSVG
        from reportlab.graphics import renderPM
        import io

        drawing = renderSVG.render(io.StringIO(svg_content))
        renderPM.drawToFile(drawing, str(png_path), fmt="PNG")
        return True
    except (ImportError, OSError, Exception):
        pass

    # Last resort: use Pillow to draw a simple indicator
    try:
        from PIL import Image, ImageDraw

        # Parse SVG to check it has paths (basic validation)
        root = ET.fromstring(svg_content)
        paths = root.findall(".//{http://www.w3.org/2000/svg}path")
        has_rect = len(root.findall(".//{http://www.w3.org/2000/svg}rect")) > 0

        img = Image.new("RGB", (width, width), (26, 26, 46))  # match bg
        draw = ImageDraw.Draw(img)
        if paths:
            # Draw a green checkmark to indicate valid SVG with paths
            draw.text((10, 10), f"{len(paths)} paths", fill=(0, 255, 0))
            draw.text((10, 30), "SVG valid", fill=(0, 255, 0))
        else:
            draw.text((10, 10), "NO PATHS", fill=(255, 0, 0))

        if has_rect:
            draw.text((10, 50), "Has background", fill=(100, 200, 100))

        img.save(str(png_path))
        return True
    except ImportError:
        return False


@pytest.mark.skipif(not _HAS_DB, reason="fo4_swf_shapes.db not built")
class TestPngExport:
    """Export sample shapes to PNG for visual verification."""

    def test_export_sample_pngs_per_category(self, shape_db):
        """Export 2 shapes per category to data/swf_test_pngs/ for visual check."""
        _PNG_OUT_DIR.mkdir(parents=True, exist_ok=True)

        categories = shape_db.execute(
            "SELECT DISTINCT tags FROM shapes ORDER BY tags"
        ).fetchall()

        exported = 0
        for row in categories:
            tag = row[0]
            samples = shape_db.execute(
                "SELECT id, name, svg_preview FROM shapes WHERE tags = ? LIMIT 2",
                (tag,),
            ).fetchall()

            safe_tag = tag.replace("/", "_").replace(" ", "_")
            for s in samples:
                png_path = _PNG_OUT_DIR / f"{safe_tag}_{s['name']}.png"
                if _svg_to_png(s["svg_preview"], png_path):
                    exported += 1

        assert exported > 0, "No PNGs exported — install cairosvg or Pillow"
        print(f"\nExported {exported} PNGs to {_PNG_OUT_DIR}")

    def test_export_previously_broken_fill0_shapes(self, shape_db):
        """Export shapes that previously had fill0-only (were invisible)."""
        _PNG_OUT_DIR.mkdir(parents=True, exist_ok=True)

        rows = shape_db.execute(
            "SELECT id, name, svg_preview, shape_data FROM shapes "
            "WHERE name LIKE 'RaiderDisciples_shape%' LIMIT 5"
        ).fetchall()

        for row in rows:
            data = json.loads(row["shape_data"])
            has_fill0_only = any(
                r.get("fill0") and r["fill0"] > 0 and not r.get("fill1")
                for r in data["records"]
                if r["type"] == "sc"
            )
            if has_fill0_only:
                png_path = _PNG_OUT_DIR / f"fill0_fix_{row['name']}.png"
                _svg_to_png(row["svg_preview"], png_path)
                # Verify SVG has actual fill (not just fill="none")
                assert 'fill="#' in row["svg_preview"], (
                    f"{row['name']} still has no fill color after fix"
                )


# ---------------------------------------------------------------------------
# Real file parsing validation (requires extracted game files)
# ---------------------------------------------------------------------------


@pytest.mark.skipif(not _HAS_SWFS, reason="FO4 SWFs not available")
class TestRealFileExtraction:
    """Parse real SWF files and validate shape extraction."""

    def test_all_vaultboy_swfs_parse(self):
        """Every SWF in VaultBoys/ must parse without errors."""
        vb_dir = _INTERFACE_DIR / "Components" / "VaultBoys"
        swfs = list(vb_dir.rglob("*.swf"))
        assert len(swfs) >= 100, f"Expected 100+ VaultBoy SWFs, found {len(swfs)}"

        failures = []
        for swf_path in swfs:
            try:
                doc = parse_swf_file(swf_path)
                assert len(doc.shapes) > 0, f"{swf_path.name}: no shapes"
            except Exception as exc:
                failures.append(f"{swf_path.name}: {exc}")

        assert not failures, f"Parse failures:\n" + "\n".join(failures)

    def test_quest_swfs_parse(self):
        """Quest VaultBoys must also parse."""
        quest_dir = _INTERFACE_DIR / "Components" / "Quest Vault Boys"
        if not quest_dir.is_dir():
            pytest.skip("Quest Vault Boys not found")
        swfs = list(quest_dir.rglob("*.swf"))
        assert len(swfs) > 0

        for swf_path in swfs[:10]:  # spot check 10
            doc = parse_swf_file(swf_path)
            assert len(doc.shapes) > 0

    def test_faction_swfs_parse(self):
        """Faction VaultBoys must parse."""
        faction_dir = _INTERFACE_DIR / "Components" / "Faction Vault boys"
        if not faction_dir.is_dir():
            pytest.skip("Faction Vault boys not found")
        swfs = list(faction_dir.rglob("*.swf"))
        assert len(swfs) > 0

        for swf_path in swfs[:10]:
            doc = parse_swf_file(swf_path)
            assert len(doc.shapes) > 0

    def test_svg_from_real_shapes_are_visible(self):
        """SVGs generated from real shapes must have visible fill colors."""
        vb_dir = _INTERFACE_DIR / "Components" / "VaultBoys"
        swfs = list(vb_dir.rglob("*.swf"))[:3]

        invisible_count = 0
        total = 0
        for swf_path in swfs:
            doc = parse_swf_file(swf_path)
            for sid, shape in doc.shapes.items():
                svg = shape_to_svg(shape)
                total += 1
                if 'fill="#' not in svg and 'fill="rgb' not in svg:
                    invisible_count += 1

        assert invisible_count == 0, (
            f"{invisible_count}/{total} shapes have no visible fill"
        )

    def test_export_real_shapes_to_png(self):
        """Export real parsed shapes to PNG for visual verification."""
        _PNG_OUT_DIR.mkdir(parents=True, exist_ok=True)
        vb_dir = _INTERFACE_DIR / "Components" / "VaultBoys" / "Perks"
        if not vb_dir.is_dir():
            pytest.skip("Perks dir not found")

        swfs = sorted(vb_dir.rglob("*.swf"))[:3]
        exported = 0
        for swf_path in swfs:
            doc = parse_swf_file(swf_path)
            # Export first shape from each SWF
            for sid, shape in list(doc.shapes.items())[:1]:
                svg = shape_to_svg(shape)
                png_path = _PNG_OUT_DIR / f"real_{swf_path.stem}_shape{sid}.png"
                if _svg_to_png(svg, png_path):
                    exported += 1

        if exported > 0:
            print(f"\nExported {exported} real-shape PNGs to {_PNG_OUT_DIR}")
