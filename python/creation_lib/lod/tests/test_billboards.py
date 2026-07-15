"""
Deterministic billboard packing + manifest writer tests.
Headless moderngl render tests (skip if no GPU).

No os.environ reads — all paths are explicit.
"""
from __future__ import annotations

import json
from pathlib import Path

import numpy as np
import pytest

from creation_lib.lod.billboards import BillboardTile, pack_billboards, write_manifest


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def solid_tile(h: int, w: int, color: tuple[int, int, int, int]) -> np.ndarray:
    """Return an H×W×4 uint8 array filled with `color`."""
    arr = np.zeros((h, w, 4), dtype=np.uint8)
    arr[:, :, :] = color
    return arr


# ---------------------------------------------------------------------------
# Deterministic packing tests (no GPU)
# ---------------------------------------------------------------------------

class TestPackTilesDeterministic:
    def test_pack_two_tiles_deterministic(self):
        """Two solid-color tiles packed twice → identical atlas bytes and entries."""
        tiles = [
            BillboardTile(model="meshes/trees/red.nif", index=0, width=100.0, height=200.0,
                          shift_z=4.0, rgba=solid_tile(64, 64, (255, 0, 0, 255))),
            BillboardTile(model="meshes/trees/green.nif", index=1, width=80.0, height=160.0,
                          shift_z=0.0, rgba=solid_tile(32, 32, (0, 255, 0, 255))),
        ]

        atlas1, entries1 = pack_billboards(tiles, max_atlas_size=512)
        atlas2, entries2 = pack_billboards(tiles, max_atlas_size=512)

        # Byte-identical RGBA atlas across two runs
        assert np.array_equal(atlas1, atlas2), "atlas bytes differ between runs (non-deterministic)"
        # Identical entries
        assert entries1 == entries2, "entries differ between runs (non-deterministic)"
        # Atlas dimensions are power-of-two and large enough to fit tiles
        h, w = atlas1.shape[:2]
        assert w >= 64, f"atlas width {w} < tile width 64"
        assert h >= 64, f"atlas height {h} < tile height 64"
        assert (w & (w - 1)) == 0, f"atlas width {w} is not power-of-two"
        assert (h & (h - 1)) == 0, f"atlas height {h} is not power-of-two"

        # UV rects: each entry's uv_* = block_rect / atlas_dim (wbLOD.pas:851-854)
        for entry in entries1:
            assert 0.0 <= entry["uv_min_x"] < entry["uv_max_x"] <= 1.0, \
                f"uv_min_x/uv_max_x out of range: {entry}"
            assert 0.0 <= entry["uv_min_y"] < entry["uv_max_y"] <= 1.0, \
                f"uv_min_y/uv_max_y out of range: {entry}"

    def test_pack_crc_dedup(self):
        """Two byte-identical tiles with different model keys → one atlas block, two entries with same UV."""
        tile_data = solid_tile(32, 32, (128, 64, 32, 255))
        tiles = [
            BillboardTile(model="meshes/trees/speciesA.nif", index=0, width=50.0, height=100.0,
                          shift_z=0.0, rgba=tile_data.copy()),
            BillboardTile(model="meshes/trees/speciesB.nif", index=1, width=50.0, height=100.0,
                          shift_z=0.0, rgba=tile_data.copy()),
        ]

        atlas, entries = pack_billboards(tiles, max_atlas_size=512)
        assert len(entries) == 2, f"expected 2 entries (both models), got {len(entries)}"

        # Both entries point to the same UV rect (CRC-dedup shares the block)
        uv_a = (entries[0]["uv_min_x"], entries[0]["uv_max_x"],
                entries[0]["uv_min_y"], entries[0]["uv_max_y"])
        uv_b = (entries[1]["uv_min_x"], entries[1]["uv_max_x"],
                entries[1]["uv_min_y"], entries[1]["uv_max_y"])
        assert uv_a == uv_b, f"dedup: UV rects differ for identical tiles: {uv_a} vs {uv_b}"

    def test_pack_grows_until_fit(self):
        """Many tiles that don't fit 128 → atlas grows; raises if exceeds max_atlas_size."""
        # 12 tiles of 64×64 each cannot fit in 128×128 (only 4 fit)
        tiles = [
            BillboardTile(
                model=f"meshes/trees/species{i}.nif",
                index=i, width=50.0, height=100.0, shift_z=0.0,
                rgba=solid_tile(64, 64, (i * 20 % 256, 128, 255, 255)),
            )
            for i in range(12)
        ]

        # Should succeed at 1024
        atlas, entries = pack_billboards(tiles, max_atlas_size=1024)
        h, w = atlas.shape[:2]
        assert w > 128 or h > 128, "expected growth beyond 128×128"
        assert len(entries) == 12

        # Should fail at 128 (can't fit 12 64×64 tiles in 128×128)
        with pytest.raises(ValueError, match="not enough space"):
            pack_billboards(tiles, max_atlas_size=128)

    def test_pack_returns_correct_schema_keys(self):
        """Each entry dict has exactly the contract keys."""
        tiles = [
            BillboardTile(model="meshes/trees/test.nif", index=0, width=100.0, height=300.0,
                          shift_z=4.0, rgba=solid_tile(32, 32, (200, 200, 200, 255))),
        ]
        _, entries = pack_billboards(tiles, max_atlas_size=512)
        assert len(entries) == 1
        expected_keys = {
            "model", "index", "width", "height", "shift_z",
            "uv_min_x", "uv_max_x", "uv_min_y", "uv_max_y",
        }
        actual_keys = set(entries[0].keys())
        assert actual_keys == expected_keys, \
            f"entry keys {actual_keys} do not match contract {expected_keys}"


class TestWriteManifest:
    def test_write_manifest_schema(self, tmp_path: Path):
        """write_manifest output JSON parses and has exactly the contract keys; two runs byte-identical."""
        tiles = [
            BillboardTile(model="meshes/trees/pine.nif", index=0, width=150.0, height=512.0,
                          shift_z=8.0, rgba=solid_tile(64, 64, (0, 128, 0, 255))),
        ]
        _, entries = pack_billboards(tiles, max_atlas_size=512)

        manifest_path1 = write_manifest(
            out_dir=tmp_path, world="TestWorld",
            atlas_rel=r"Textures\Terrain\LODGen\TestWorld\TestWorldTreeLod.dds",
            atlas_n_rel=r"Textures\Terrain\LODGen\TestWorld\TestWorldTreeLod_n.dds",
            atlas_w=512, atlas_h=512,
            entries=entries,
        )
        assert manifest_path1.exists(), "manifest file not written"

        with open(manifest_path1) as f:
            data = json.load(f)

        # Top-level schema
        assert "atlas" in data
        assert "atlas_normal" in data
        assert "atlas_w" in data
        assert "atlas_h" in data
        assert "entries" in data

        # Entry schema matches the contract exactly
        expected_keys = {
            "model", "index", "width", "height", "shift_z",
            "uv_min_x", "uv_max_x", "uv_min_y", "uv_max_y",
        }
        for e in data["entries"]:
            assert set(e.keys()) == expected_keys, \
                f"entry keys {set(e.keys())} do not match contract {expected_keys}"

        # Second run → byte-identical (deterministic key + entry order)
        manifest_path2 = write_manifest(
            out_dir=tmp_path / "run2", world="TestWorld",
            atlas_rel=r"Textures\Terrain\LODGen\TestWorld\TestWorldTreeLod.dds",
            atlas_n_rel=r"Textures\Terrain\LODGen\TestWorld\TestWorldTreeLod_n.dds",
            atlas_w=512, atlas_h=512,
            entries=entries,
        )
        assert manifest_path1.read_bytes() == manifest_path2.read_bytes(), \
            "manifest is not byte-identical across two runs"

    def test_write_manifest_entries_sorted_by_index(self, tmp_path: Path):
        """write_manifest sorts entries by index for deterministic output."""
        # Provide entries in reverse order
        entries = [
            {"model": "b.nif", "index": 2, "width": 1.0, "height": 1.0, "shift_z": 0.0,
             "uv_min_x": 0.5, "uv_max_x": 1.0, "uv_min_y": 0.0, "uv_max_y": 1.0},
            {"model": "a.nif", "index": 0, "width": 1.0, "height": 1.0, "shift_z": 0.0,
             "uv_min_x": 0.0, "uv_max_x": 0.5, "uv_min_y": 0.0, "uv_max_y": 0.5},
            {"model": "c.nif", "index": 1, "width": 1.0, "height": 1.0, "shift_z": 0.0,
             "uv_min_x": 0.0, "uv_max_x": 0.5, "uv_min_y": 0.5, "uv_max_y": 1.0},
        ]
        mp = write_manifest(
            out_dir=tmp_path, world="W",
            atlas_rel="atlas.dds", atlas_n_rel="atlas_n.dds",
            atlas_w=256, atlas_h=256, entries=entries,
        )
        with open(mp) as f:
            data = json.load(f)
        indices = [e["index"] for e in data["entries"]]
        assert indices == sorted(indices), f"entries not sorted by index: {indices}"


# ---------------------------------------------------------------------------
# Headless moderngl render tests (skip if no GPU)
# ---------------------------------------------------------------------------

def _try_gl_context():
    """Return a standalone GL context, or skip the test if GL is unavailable."""
    mgl = pytest.importorskip("moderngl")
    try:
        return mgl.create_standalone_context()
    except Exception as e:  # pragma: no cover - environment dependent
        pytest.skip(f"no headless GL: {e}")


# Trivial fragment that paints the whole triangle one colour with full alpha.
_QUAD_VS = """
#version 330
in vec2 in_pos;
void main() { gl_Position = vec4(in_pos, 0.0, 1.0); }
"""
_QUAD_FS = """
#version 330
out vec4 frag;
uniform vec4 u_color;
void main() { frag = u_color; }
"""


def _render_solid_triangle(ctx, size, color):
    """Render one solid-colour triangle into a fresh RGBA8 FBO and read it back.

    Exercises the SAME ensure_fbo + _fbo_readback path the billboard renderer
    uses, without needing a NIF fixture — validates the GL pipeline end-to-end.
    """
    import numpy as _np
    from creation_lib.lod.billboards import _fbo_readback

    tex = ctx.texture((size, size), 4)
    fbo = ctx.framebuffer(color_attachments=[tex])
    fbo.use()
    ctx.clear(0.0, 0.0, 0.0, 0.0)

    prog = ctx.program(vertex_shader=_QUAD_VS, fragment_shader=_QUAD_FS)
    prog["u_color"].value = color
    # A big centred triangle covering the middle of the viewport.
    verts = _np.array([-0.6, -0.6, 0.6, -0.6, 0.0, 0.7], dtype="f4")
    vbo = ctx.buffer(verts.tobytes())
    vao = ctx.vertex_array(prog, [(vbo, "2f", "in_pos")])
    vao.render()

    return _fbo_readback(tex, size)


class TestFboPipeline:
    """Validate the offscreen FBO render + readback path with a trivial quad.

    These run whenever a GL context is available (RTX/CI-with-GPU); they do not
    need a NIF fixture, so they exercise the novel GL plumbing directly.
    """

    def test_fbo_render_triangle_non_blank(self):
        ctx = _try_gl_context()
        size = 64
        arr = _render_solid_triangle(ctx, size, (1.0, 0.0, 0.0, 1.0))
        assert arr.shape == (size, size, 4)
        assert arr.dtype == np.uint8
        # Centre pixel is inside the triangle → red, opaque.
        cy, cx = size // 2, size // 2
        assert tuple(arr[cy, cx]) == (255, 0, 0, 255), f"centre={tuple(arr[cy, cx])}"
        # Top-left corner is outside the triangle → cleared transparent.
        assert tuple(arr[2, 2]) == (0, 0, 0, 0), f"corner={tuple(arr[2, 2])}"
        # Something actually rendered.
        assert arr[:, :, 3].max() == 255

    def test_fbo_render_deterministic(self):
        ctx = _try_gl_context()
        size = 64
        a = _render_solid_triangle(ctx, size, (0.0, 1.0, 0.0, 1.0))
        b = _render_solid_triangle(ctx, size, (0.0, 1.0, 0.0, 1.0))
        assert a.tobytes() == b.tobytes(), "two identical renders differ (non-deterministic)"


class TestCoverageAlpha:
    """_coverage_alpha keys out the clear colour to reconstruct coverage."""

    def test_coverage_keys_out_background(self):
        from creation_lib.lod.billboards import _BG_KEY, _coverage_alpha
        bg = tuple(round(c * 255.0) for c in _BG_KEY)
        arr = np.zeros((4, 4, 4), dtype=np.uint8)
        arr[:, :, :3] = bg            # all background
        arr[1, 1, :3] = (10, 20, 30)  # one "covered" pixel
        out = _coverage_alpha(arr, _BG_KEY)
        assert out[1, 1, 3] == 255, "covered pixel should be opaque"
        assert out[0, 0, 3] == 0, "background pixel should be transparent"
        # RGB is untouched.
        assert tuple(out[1, 1, :3]) == (10, 20, 30)


# Repo root, for locating an optional real FO4 LOD nif fixture.
_REPO_ROOT = Path(__file__).resolve().parents[5]
# A small real FO4 LOD nif present when the extracted corpus is available
# (same fixture the Rust object tests use). Lets the full NIF render
# path be exercised on a GPU box; skipped cleanly when the corpus is absent.
_FO4_LOD_NIF = _REPO_ROOT / "extracted" / "fo4" / "Meshes" / "DLC03" / "LOD" / \
    "Architecture" / "Barn" / "BarnDoorMedL01_LOD.nif"
_FO4_DATA_DIR = _REPO_ROOT / "extracted" / "fo4"


def _model_fixture():
    """Return (model_path, data_dirs) for the full NIF render path, or None."""
    for cand in (
        Path(__file__).parent / "fixtures" / "synthetic_tree.nif",
        Path("tests/fixtures/synthetic_tree.nif"),
    ):
        if cand.exists():
            return str(cand), [cand.parent]
    if _FO4_LOD_NIF.is_file():
        return str(_FO4_LOD_NIF), [_FO4_DATA_DIR]
    return None


class TestRenderSpeciesTile:
    """Full NIF render path. Runs only when both a GL context and a NIF model
    are available; otherwise skips (the FBO-pipeline tests above are the
    always-on GL gate)."""

    def test_render_species_tile_smoke(self):
        _try_gl_context()  # skip cleanly when no GPU; render uses the shared ctx
        from creation_lib.lod.billboards import render_species_tile
        fx = _model_fixture()
        if fx is None:
            pytest.skip("no NIF model fixture available (synthetic or extracted corpus)")
        model, data_dirs = fx
        tile = render_species_tile(model, data_dirs=data_dirs, size=64, brightness=1.0)
        assert tile.shape == (64, 64, 4), f"unexpected tile shape: {tile.shape}"
        assert tile.dtype == np.uint8, f"unexpected dtype: {tile.dtype}"
        assert tile[:, :, 3].max() > 0, "all alpha zero — nothing rendered"

    def test_render_species_tile_deterministic(self):
        # The shared module context is used (ctx=None) — a fresh standalone
        # context per call is driver-nondeterministic on the first render.
        _try_gl_context()
        from creation_lib.lod.billboards import render_species_tile
        fx = _model_fixture()
        if fx is None:
            pytest.skip("no NIF model fixture available")
        model, data_dirs = fx
        a = render_species_tile(model, data_dirs=data_dirs, size=64, brightness=1.0)
        b = render_species_tile(model, data_dirs=data_dirs, size=64, brightness=1.0)
        assert a.tobytes() == b.tobytes(), "two renders of the same model differ"

    def test_brightness_scales_rgb_gpu(self):
        _try_gl_context()
        from creation_lib.lod.billboards import render_species_tile
        fx = _model_fixture()
        if fx is None:
            pytest.skip("no NIF model fixture available")
        model, data_dirs = fx
        tile_full = render_species_tile(model, data_dirs=data_dirs, size=64, brightness=1.0)
        tile_dim = render_species_tile(model, data_dirs=data_dirs, size=64, brightness=0.5)
        cov = tile_full[:, :, 3] > 0
        mean_full = tile_full[:, :, :3][cov].mean() if cov.any() else 0.0
        if mean_full < 8.0:
            pytest.skip("model renders too dark for a meaningful brightness comparison")
        mean_dim = tile_dim[:, :, :3][cov].mean()
        assert mean_dim < mean_full, f"dim ({mean_dim}) not less than full ({mean_full})"


class TestApplyBrightness:
    """Brightness scaling is a pure post-readback op — tested without a GPU."""

    def test_brightness_scales_rgb_not_alpha(self):
        from creation_lib.lod.billboards import _apply_brightness
        arr = np.zeros((2, 2, 4), dtype=np.uint8)
        arr[:, :, :3] = (200, 100, 50)
        arr[:, :, 3] = 255
        out = _apply_brightness(arr, 0.5)
        assert tuple(out[0, 0, :3]) == (100, 50, 25), f"got {tuple(out[0, 0, :3])}"
        assert out[0, 0, 3] == 255, "alpha (coverage) must not be scaled"

    def test_brightness_one_is_noop(self):
        from creation_lib.lod.billboards import _apply_brightness
        arr = np.full((2, 2, 4), 123, dtype=np.uint8)
        out = _apply_brightness(arr, 1.0)
        assert np.array_equal(out, arr)


class TestGenerateBillboardsEndToEnd:
    """Full headless generator: render → pack → atlas DDS + _n → manifest."""

    def test_generate_billboards_writes_atlas_and_manifest(self, tmp_path: Path):
        _try_gl_context()
        from creation_lib.lod.billboards import generate_billboards
        fx = _model_fixture()
        if fx is None:
            pytest.skip("no NIF model fixture available")
        model, data_dirs = fx
        species = [
            {"model": model, "billboard": "a.dds", "index": 0,
             "width": 100.0, "height": 300.0, "shift_z": 4.0},
            {"model": model, "billboard": "b.dds", "index": 1,
             "width": 120.0, "height": 280.0, "shift_z": 8.0},
        ]
        manifest_path = generate_billboards(
            species, data_dirs=data_dirs, out_dir=tmp_path,
            world="TestWorld", atlas_size=512, brightness=1.0, tile_size=128,
        )
        assert manifest_path.exists()
        data = json.loads(manifest_path.read_text())
        assert data["atlas_w"] > 0 and data["atlas_h"] > 0
        assert len(data["entries"]) == 2
        # Input width/height/index round-trip into the manifest entries.
        by_index = {e["index"]: e for e in data["entries"]}
        assert by_index[0]["width"] == 100.0
        assert by_index[1]["height"] == 280.0
        # Atlas DDS + flat-normal sibling exist on disk at the manifest paths.
        atlas = tmp_path / data["atlas"].replace("\\", "/")
        normal = tmp_path / data["atlas_normal"].replace("\\", "/")
        assert atlas.is_file() and atlas.stat().st_size > 0
        assert normal.is_file() and normal.stat().st_size > 0
        # Identical species → CRC-dedup shares one atlas block (same UV rect).
        e0, e1 = data["entries"][0], data["entries"][1]
        assert (e0["uv_min_x"], e0["uv_max_x"]) == (e1["uv_min_x"], e1["uv_max_x"])


class TestBillboardRenderErrorContract:
    """The optional-dependency contract: a clear, catchable error type exists."""

    def test_render_error_is_runtimeerror_subclass(self):
        from creation_lib.lod.billboards import BillboardRenderError
        assert issubclass(BillboardRenderError, RuntimeError)
