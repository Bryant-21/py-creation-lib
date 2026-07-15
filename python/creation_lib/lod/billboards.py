"""
Billboard atlas generator for tree LOD.

Deterministic half: given pre-rendered RGBA tiles (numpy arrays),
CRC-dedup, guillotine-pack into an atlas, write the JSON manifest.

Headless render half: render each tree species' model via SceneRenderer FBO
readback (moderngl standalone context).

No os.environ reads for project config — all paths are explicit parameters.
"""
from __future__ import annotations

import json
import zlib
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING, Callable

import numpy as np

if TYPE_CHECKING:
    pass  # numpy is always available; moderngl is optional

# ---------------------------------------------------------------------------
# BillboardTile — the unit the packer consumes
# ---------------------------------------------------------------------------

@dataclass
class BillboardTile:
    model: str          # source LOD model path (the lookup key)
    index: int          # tree-list index assigned by the generator
    width: float        # in-game billboard width (from .txt sidecar / record)
    height: float       # in-game billboard height
    shift_z: float      # Z-shift of the billboard origin
    rgba: "np.ndarray"  # H×W×4 uint8, the rendered (or supplied) tile


# ---------------------------------------------------------------------------
# _BinNode — guillotine packer node (port: TwbBinPacker, wbLOD.pas:486-558)
# ---------------------------------------------------------------------------

class _BinNode:
    __slots__ = ("x", "y", "w", "h", "used", "right", "down")

    def __init__(self, x: int = 0, y: int = 0, w: int = 0, h: int = 0) -> None:
        self.x = x
        self.y = y
        self.w = w
        self.h = h
        self.used = False
        self.right: _BinNode | None = None
        self.down: _BinNode | None = None


def _find_node(root: _BinNode, w: int, h: int) -> _BinNode | None:
    """Recursive search for a free node large enough for (w, h).
    port: TwbBinPacker.FindNode (wbLOD.pas:486-497)
    """
    if root.used:
        result = _find_node(root.right, w, h) if root.right else None
        if result is None and root.down:
            result = _find_node(root.down, w, h)
        return result
    elif w <= root.w and h <= root.h:
        return root
    return None


def _split_node(node: _BinNode, w: int, h: int, px: int, py: int) -> _BinNode:
    """Mark node used; create right and down children.
    port: TwbBinPacker.SplitNode (wbLOD.pas:499-513)
    """
    node.used = True
    node.down = _BinNode(x=node.x, y=node.y + h + py, w=node.w, h=node.h - h - py)
    node.right = _BinNode(x=node.x + w + px, y=node.y, w=node.w - w - px, h=h)
    return node


def _guillotine_fit(
    blocks: list[dict],  # list of {w, h, crc, idx_in_blocks} — mutated to add x, y, fit
    atlas_w: int,
    atlas_h: int,
    padding_x: int = 2,
    padding_y: int = 2,
) -> bool:
    """Try to fit all blocks in atlas_w × atlas_h.
    Sorts by max-side descending (port: MaxSideSort wbLOD.pas:517-535).
    Sets block['x'], block['y'], block['fit'] = True on success.
    Returns True if all blocks fit.
    port: TwbBinPacker.Fit (wbLOD.pas:515-558)
    """
    # Sort by max(w, h) descending — port: MaxSideSort (wbLOD.pas:527)
    blocks.sort(key=lambda b: max(b["w"], b["h"]), reverse=True)

    root = _BinNode(0, 0, atlas_w, atlas_h)
    result = True
    for blk in blocks:
        node = _find_node(root, blk["w"], blk["h"])
        if node is not None:
            node = _split_node(node, blk["w"], blk["h"], padding_x, padding_y)
            blk["x"] = node.x
            blk["y"] = node.y
            blk["fit"] = True
        else:
            result = False
    return result


# ---------------------------------------------------------------------------
# pack_billboards — CRC-dedup + guillotine-pack (port: BuildAtlas wbLOD.pas:783-859)
# ---------------------------------------------------------------------------

def pack_billboards(
    tiles: list[BillboardTile],
    max_atlas_size: int,
    padding: int = 2,
) -> tuple[np.ndarray, list[dict]]:
    """CRC-dedup + guillotine-pack tiles into an atlas.

    Returns ``(atlas_rgba, entries)`` where each entry is a manifest dict
    {model, index, width, height, shift_z, uv_min_x, uv_max_x, uv_min_y, uv_max_y}.

    Port: TwbLodTES5TreeList.BuildAtlas (wbLOD.pas:783-859):
    - CRC32-dedup (wbLOD.pas:793-801)
    - guillotine BinPacker with padding (wbLOD.pas:816-833)
    - UV = block_rect / atlas_dim (wbLOD.pas:851-854)

    Determinism: tiles are processed in input order (the caller must supply
    them sorted by model path for stable output). The sort-by-max-side inside
    _guillotine_fit is stable relative to equal-sized tiles.
    """
    # --- CRC-dedup ---
    # port: wbLOD.pas:793-809 — exclude duplicate textures by checksum
    seen_crc: dict[int, int] = {}  # crc32 → first-seen block index
    unique_blocks: list[dict] = []

    for tile in tiles:
        crc = zlib.crc32(tile.rgba.tobytes()) & 0xFFFFFFFF
        if crc not in seen_crc:
            block_idx = len(unique_blocks)
            seen_crc[crc] = block_idx
            h, w = tile.rgba.shape[:2]
            unique_blocks.append({
                "crc": crc,
                "w": w,
                "h": h,
                "x": 0,
                "y": 0,
                "fit": False,
                "rgba": tile.rgba,
            })

    if not unique_blocks:
        raise ValueError("no tiles to pack")

    # --- Grow-until-fit (port: wbLOD.pas:817-829) ---
    # Start at min(512, max_atlas_size) × min(512, max_atlas_size)
    atlas_w = min(512, max_atlas_size)
    atlas_h = min(512, max_atlas_size)

    while True:
        # Reset x/y/fit on each attempt (repack fresh each time)
        for blk in unique_blocks:
            blk["x"] = 0
            blk["y"] = 0
            blk["fit"] = False

        if _guillotine_fit(unique_blocks, atlas_w, atlas_h, padding, padding):
            break

        # Grow: if width <= height, double width; else double height (wbLOD.pas:823-826)
        if atlas_w <= atlas_h:
            atlas_w *= 2
        else:
            atlas_h *= 2

        if atlas_w > max_atlas_size or atlas_h > max_atlas_size:
            raise ValueError(
                f"Can't fit billboards on atlas, not enough space "
                f"(max_atlas_size={max_atlas_size}, need {atlas_w}×{atlas_h})"
            )

    # --- Compose atlas image ---
    atlas = np.zeros((atlas_h, atlas_w, 4), dtype=np.uint8)
    for blk in unique_blocks:
        if blk["fit"]:
            bh, bw = blk["h"], blk["w"]
            atlas[blk["y"]: blk["y"] + bh, blk["x"]: blk["x"] + bw] = blk["rgba"]

    # --- Build a crc → placed-block lookup for UV assignment ---
    crc_to_block: dict[int, dict] = {blk["crc"]: blk for blk in unique_blocks}

    # --- Build entries (port: wbLOD.pas:836-855) ---
    # Process tiles in input order so entries match the original index ordering.
    entries: list[dict] = []
    for tile in tiles:
        crc = zlib.crc32(tile.rgba.tobytes()) & 0xFFFFFFFF
        blk = crc_to_block[crc]
        # UV = block_rect / atlas_dim (wbLOD.pas:851-854)
        entries.append({
            "model": tile.model,
            "index": tile.index,
            "width": tile.width,
            "height": tile.height,
            "shift_z": tile.shift_z,
            "uv_min_x": blk["x"] / atlas_w,
            "uv_max_x": (blk["x"] + blk["w"]) / atlas_w,
            "uv_min_y": blk["y"] / atlas_h,
            "uv_max_y": (blk["y"] + blk["h"]) / atlas_h,
        })

    return atlas, entries


# ---------------------------------------------------------------------------
# write_manifest — write the JSON manifest
# ---------------------------------------------------------------------------

def write_manifest(
    out_dir: Path,
    world: str,
    atlas_rel: str,
    atlas_n_rel: str,
    atlas_w: int,
    atlas_h: int,
    entries: list[dict],
) -> Path:
    """Write the BillboardManifest JSON.

    Entries are sorted by ``index`` for deterministic output.
    JSON key order is deterministic (sort_keys=True).
    Returns the path to the written manifest file.
    """
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    manifest = {
        "atlas": atlas_rel,
        "atlas_normal": atlas_n_rel,
        "atlas_w": atlas_w,
        "atlas_h": atlas_h,
        "entries": sorted(entries, key=lambda e: e["index"]),
    }

    manifest_path = out_dir / f"{world}_billboard_manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True), encoding="utf-8")
    return manifest_path


# ---------------------------------------------------------------------------
# Headless GL plumbing — moderngl is an OPTIONAL dependency.
# TODO: declare moderngl dependency in root pyproject.toml.
# ---------------------------------------------------------------------------

# Sentinel background colour the render clears to. SceneRenderer.render() forces
# an opaque clear (alpha=1.0, scene_renderer.py:1353-1354), so coverage-alpha is
# recovered by chroma-keying the readback against this exact colour. Pure magenta
# is chosen because foliage textures never contain it.
_BG_KEY = (1.0, 0.0, 1.0)


class BillboardRenderError(RuntimeError):
    """Raised when the headless GL path is unavailable (no moderngl / no context).

    Callers and tests should catch this to skip the GL path cleanly on a box
    without a GPU. It is distinct from RuntimeError so a real render failure
    is not silently swallowed.
    """


_SHARED_CTX = None  # module-cached standalone context (see _require_standalone_context)


def _require_standalone_context(ctx: "object | None"):
    """Return a usable moderngl standalone context, or raise BillboardRenderError.

    moderngl is imported lazily here so the deterministic packing/manifest code
    has no hard moderngl dependency.

    When the caller passes no context, a single module-level context is created
    and reused. This is deliberate and load-bearing for determinism: a fresh
    standalone context per call produces driver-dependent first-vs-later-render
    differences, whereas reusing one context renders byte-identically run to
    run. ``generate_billboards`` likewise renders every species through one
    shared context.
    """
    if ctx is not None:
        return ctx
    try:
        import moderngl  # noqa: PLC0415 — optional dep, imported lazily on purpose
    except ImportError as e:  # pragma: no cover - exercised only without moderngl
        raise BillboardRenderError(f"moderngl not installed: {e}") from e
    global _SHARED_CTX
    if _SHARED_CTX is None:
        try:
            _SHARED_CTX = moderngl.create_standalone_context()
        except Exception as e:  # pragma: no cover - exercised only without a GPU
            raise BillboardRenderError(f"no headless GL context: {e}") from e
    return _SHARED_CTX


def _fbo_readback(fbo_texture, size: int) -> "np.ndarray":
    """Read an RGBA8 moderngl texture back into an H×W×4 uint8 numpy array.

    Flips the rows (GL origin is bottom-left). This is the exact readback
    pattern used by the NIF editor screenshot path
    (ui/editor/exporters/screenshot.py:38-43).
    """
    from PIL import Image  # noqa: PLC0415

    data = fbo_texture.read()
    img = Image.frombytes("RGBA", (size, size), data).transpose(Image.FLIP_TOP_BOTTOM)
    return np.frombuffer(img.tobytes(), dtype=np.uint8).reshape(size, size, 4).copy()


# Per-channel chroma-key tolerance (0..255). Texture filtering / mip generation
# bleeds the magenta clear colour a few levels into edge pixels; an exact match
# leaves a thin magenta fringe. A small tolerance keys those edge pixels out too.
_BG_KEY_TOLERANCE = 8


def _coverage_alpha(arr: "np.ndarray", bg_key: tuple[float, float, float]) -> "np.ndarray":
    """Set alpha=0 where a pixel is (near) the background key colour, else 255.

    SceneRenderer clears the FBO opaque, so the geometry's own alpha is not
    available; coverage is reconstructed by keying out the known clear colour.
    A small per-channel tolerance (``_BG_KEY_TOLERANCE``) absorbs the magenta
    bleed that filtering introduces at geometry edges.
    """
    key = np.array([round(c * 255.0) for c in bg_key], dtype=np.int16)
    diff = np.abs(arr[:, :, :3].astype(np.int16) - key)
    is_bg = np.all(diff <= _BG_KEY_TOLERANCE, axis=2)
    out = arr.copy()
    out[:, :, 3] = np.where(is_bg, 0, 255).astype(np.uint8)
    return out


def _apply_brightness(arr: "np.ndarray", brightness: float) -> "np.ndarray":
    """Scale the RGB channels by ``brightness`` (alpha = coverage, left intact).

    Honors the ``billboard_brightness`` setting. brightness==1.0 is a no-op.
    """
    if brightness == 1.0:
        return arr
    out = arr.copy()
    rgb = out[:, :, :3].astype(np.float32) * brightness
    out[:, :, :3] = np.clip(rgb, 0, 255).astype(np.uint8)
    return out


# ---------------------------------------------------------------------------
# render_species_tile — headless moderngl render (raises if no GPU)
# ---------------------------------------------------------------------------

def _render_species_tile_with_bounds(
    model_path: str,
    data_dirs: list[Path],
    size: int,
    brightness: float,
    ctx: "object | None" = None,
) -> tuple["np.ndarray", tuple[tuple[float, float, float], tuple[float, float, float]]]:
    """Render one tree's full 3D model and return the RGBA tile plus scene bounds.

    Front-facing view (the canonical billboard is a flat front card), framed on
    the model's geometry bounding sphere. A very narrow field of view from far
    away approximates an orthographic projection (SceneRenderer's camera is
    perspective-only). Alpha is reconstructed as coverage by keying out the
    background; RGB is multiplied by ``brightness``. Returns H×W×4 uint8.

    Requires ``moderngl`` and the repo SceneRenderer (creation_lib.renderer).
    Raises ``BillboardRenderError`` if the GL context cannot be created — callers
    and tests should guard with pytest.importorskip / try/except for headless CI.

    No os.environ reads for game config — texture_dirs come from ``data_dirs``.

    FBO readback pattern: ui/editor/exporters/screenshot.py:38-43.
    """
    import glm  # noqa: PLC0415

    from creation_lib.core.game_profiles import get_profile  # noqa: PLC0415
    from creation_lib.renderer.scene_renderer import SceneRenderer  # noqa: PLC0415
    from creation_lib.renderer.nif_loader import load_nif_to_scene  # noqa: PLC0415
    from creation_lib.renderer.camera import OrbitCamera  # noqa: PLC0415
    from creation_lib.renderer.lighting import LightingSetup  # noqa: PLC0415

    ctx = _require_standalone_context(ctx)

    renderer = SceneRenderer(ctx)
    renderer.init_shaders()
    renderer.ensure_fbo(size, size)
    renderer.bg_color = _BG_KEY

    # load_nif_to_scene resolves textures via Path.is_dir() — pass Path objects.
    texture_dirs = [Path(d) for d in data_dirs]
    scene_node, _nif = load_nif_to_scene(
        model_path, ctx,
        program=renderer.programs.get("default"),
        texture_dirs=texture_dirs,
        ba2_mgr=None,
        nif_id="billboard",
        game_profile=get_profile("fo4"),
    )
    renderer.scene_root = scene_node

    # Frame a front-facing camera on the model bounds.
    min_pt = glm.vec3(1e30)
    max_pt = glm.vec3(-1e30)
    renderer._compute_scene_bounds(scene_node, min_pt, max_pt)
    if min_pt.x > max_pt.x:  # empty scene → unit sphere at origin
        center, radius = glm.vec3(0.0), 1.0
        bounds = ((-0.5, -0.5, 0.0), (0.5, 0.5, 1.0))
    else:
        center = (min_pt + max_pt) * 0.5
        radius = max(glm.length(max_pt - min_pt) * 0.5, 1e-3)
        bounds = (
            (float(min_pt.x), float(min_pt.y), float(min_pt.z)),
            (float(max_pt.x), float(max_pt.y), float(max_pt.z)),
        )

    camera = OrbitCamera()
    camera.set_front()           # azimuth=90, elevation=0 → looking down +Y
    camera.frame_on_bounds(center, radius)
    camera.fov = 5.0             # narrow FOV ≈ orthographic front projection
    camera.distance = radius / max(np.tan(np.radians(camera.fov) * 0.5), 1e-6)
    camera.near = max(camera.distance - radius * 2.0, 0.01)
    camera.far = camera.distance + radius * 2.0

    lighting = LightingSetup()

    # SceneRenderer.render() ends by restoring the default framebuffer via
    # ctx.screen.use() (scene_renderer.py:1536). A standalone (windowless)
    # context has no default framebuffer — ctx.screen is None — so that final
    # cleanup raises after the scene has already been drawn into renderer.fbo.
    # Tolerate ONLY that case (ctx.screen is None); any other error is real.
    try:
        renderer.render(camera, lighting)
    except AttributeError:
        if getattr(ctx, "screen", None) is not None:
            raise

    arr = _fbo_readback(renderer.fbo_texture, size)
    arr = _coverage_alpha(arr, _BG_KEY)
    return _apply_brightness(arr, brightness), bounds


def render_species_tile(
    model_path: str,
    data_dirs: list[Path],
    size: int,
    brightness: float,
    ctx: "object | None" = None,
) -> "np.ndarray":
    tile, _bounds = _render_species_tile_with_bounds(
        model_path,
        data_dirs=data_dirs,
        size=size,
        brightness=brightness,
        ctx=ctx,
    )
    return tile


# ---------------------------------------------------------------------------
# billboard atlas path helper (creation_lib.naming has no LOD/billboard helper)
# ---------------------------------------------------------------------------

def billboard_atlas_rel_path(world: str) -> str:
    r"""Data-relative path for the billboard atlas DDS.

    Mirrors the Rust naming::billboard_atlas convention
    (Textures\Terrain\LODGen\<World>\<World>TreeLod.dds).
    """
    return f"Textures/Terrain/LODGen/{world}/{world}TreeLod.dds"


# ---------------------------------------------------------------------------
# generate_billboards — full pipeline
# ---------------------------------------------------------------------------

def generate_billboards(
    species: list[dict],
    data_dirs: list[Path],
    out_dir: Path,
    world: str,
    atlas_size: int,
    brightness: float,
    tile_size: int = 256,
    progress: Callable[[str, float], None] | None = None,
) -> Path:
    """Full generator: render each species, pack, encode atlas DDS + _n, write manifest.

    ``species`` rows: {model, billboard, index, width, height, shift_z}.
    Each species is rendered at ``tile_size``×``tile_size``; tiles are packed
    (and the atlas grown) up to ``atlas_size`` (the ``billboard_atlas_size``
    setting). Species are rendered in deterministic (model-sorted) order.
    Returns the manifest path. Headless — no UI interaction. No os.environ for
    game config.

    Raises ``BillboardRenderError`` if the GL context is unavailable.
    """
    from creation_lib.dds.native_runtime import write_dds_rgba  # noqa: PLC0415

    out_dir = Path(out_dir)
    ctx = _require_standalone_context(None)
    tile_size = min(tile_size, atlas_size)

    ordered_species = sorted(species, key=lambda s: s["model"])
    total = max(len(ordered_species), 1)
    tiles: list[BillboardTile] = []
    for idx, sp in enumerate(ordered_species):
        if progress is not None:
            progress(f"rendering billboard species {idx + 1}/{total}", idx / total)
        render_model = sp.get("render_model") or sp["model"]
        rgba, bounds = _render_species_tile_with_bounds(
            render_model, data_dirs=data_dirs, size=tile_size,
            brightness=brightness, ctx=ctx,
        )
        width, height, shift_z = _species_dimensions(sp, bounds)
        tiles.append(BillboardTile(
            model=sp["model"],
            index=sp["index"],
            width=width,
            height=height,
            shift_z=shift_z,
            rgba=rgba,
        ))
    if progress is not None:
        progress("packing billboard atlas", 0.98)

    atlas_rgba, entries = pack_billboards(tiles, max_atlas_size=atlas_size)
    h, w = atlas_rgba.shape[:2]

    atlas_rel = billboard_atlas_rel_path(world)
    atlas_dds_path = out_dir / atlas_rel
    atlas_dds_path.parent.mkdir(parents=True, exist_ok=True)

    if not write_dds_rgba(
        str(atlas_dds_path), w, h, atlas_rgba.tobytes(),
        format="BC3_UNORM", generate_mips=True,
    ):
        raise RuntimeError(f"failed to write atlas DDS: {atlas_dds_path}")

    # Flat-normal sibling (_n.dds): default tangent-space normal (128,128,255).
    atlas_n_dds_path = atlas_dds_path.with_name(atlas_dds_path.stem + "_n.dds")
    flat_normal = np.empty((h, w, 4), dtype=np.uint8)
    flat_normal[:, :, :] = (128, 128, 255, 255)
    if not write_dds_rgba(
        str(atlas_n_dds_path), w, h, flat_normal.tobytes(),
        format="BC3_UNORM", generate_mips=True,
    ):
        raise RuntimeError(f"failed to write atlas normal DDS: {atlas_n_dds_path}")

    atlas_n_rel = str(atlas_n_dds_path.relative_to(out_dir)).replace("/", "\\")
    atlas_rel = atlas_rel.replace("/", "\\")

    return write_manifest(
        out_dir=out_dir, world=world,
        atlas_rel=atlas_rel, atlas_n_rel=atlas_n_rel,
        atlas_w=w, atlas_h=h,
        entries=entries,
    )


def _species_dimensions(
    species: dict,
    bounds: tuple[tuple[float, float, float], tuple[float, float, float]],
) -> tuple[float, float, float]:
    min_pt, max_pt = bounds
    derived_width = max(max_pt[0] - min_pt[0], max_pt[1] - min_pt[1], 1.0)
    derived_height = max(max_pt[2] - min_pt[2], 1.0)
    width = float(species.get("width") or derived_width)
    height = float(species.get("height") or derived_height)
    shift_z = float(species.get("shift_z") if species.get("shift_z") is not None else min_pt[2])
    if width <= 0.0:
        width = derived_width
    if height <= 0.0:
        height = derived_height
    return width, height, shift_z


def generate_fo76_bto_tree_billboards(
    world: str,
    settings: dict,
    *,
    source_data_dir: Path,
    data_dirs: list[Path],
    out_dir: Path,
    progress: Callable[[str, float], None] | None = None,
) -> tuple[Path | None, int]:
    from creation_lib.lod import native_runtime as lod_native_runtime  # noqa: PLC0415

    rows = lod_native_runtime.collect_fo76_bto_tree_billboard_species(
        world,
        settings,
        source_data_dir=source_data_dir,
    )
    if progress is not None:
        progress(f"collected {len(rows)} billboard tree species", 0.0)
    if not rows:
        return None, 0

    species = [
        {
            "model": row["model"],
            "render_model": row["render_model"],
            "index": idx,
            "width": 0.0,
            "height": 0.0,
            "shift_z": None,
        }
        for idx, row in enumerate(rows)
    ]
    manifest = generate_billboards(
        species,
        data_dirs=data_dirs,
        out_dir=out_dir,
        world=world,
        atlas_size=int(settings.get("trees", {}).get("billboard_atlas_size", 2048)),
        brightness=float(settings.get("trees", {}).get("billboard_brightness", 1.0)),
        progress=progress,
    )
    return manifest, len(species)
