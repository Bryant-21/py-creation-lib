"""DDS/image texture loading for ModernGL.

DDS formats are decoded through directxtex_native.
BC5 two-channel normals have Z reconstructed in the shader (normal_decode.glsl).
Non-DDS formats (PNG/TGA) use Pillow.

Two-phase API for parallel loading:
  decode_texture(path)  — CPU/IO only, no GL context needed, safe on threads
  upload_decoded(ctx, decoded)  — GPU upload, must run on the GL thread
"""
from __future__ import annotations
from concurrent.futures import ThreadPoolExecutor, as_completed
import logging
import os
import tempfile
from pathlib import Path

from PIL import Image
import moderngl

from creation_lib.dds import native_runtime
from creation_lib.dds import load_image as _dds_load_image

_log = logging.getLogger("renderer.dds")


class DecodedTexture:
    """Raw decoded texture data, ready for GPU upload. No GL context needed."""
    __slots__ = ('size', 'components', 'data')

    def __init__(self, size: tuple[int, int], components: int, data: bytes):
        self.size = size
        self.components = components
        self.data = data


def _decoded_from_native_payload(payload) -> DecodedTexture | None:
    """Convert directxtex_native payload dict to a DecodedTexture."""
    if payload is None:
        return None
    return DecodedTexture(
        size=(int(payload["width"]), int(payload["height"])),
        components=4,
        data=bytes(payload["rgba"]),
    )


def _decode_via_native_runtime(path: Path) -> DecodedTexture | None:
    """Decode a DDS file via directxtex_native when available."""
    try:
        payload = native_runtime.read_dds_rgba(str(path))
    except Exception as e:
        _log.debug("directxtex_native decode failed for %s: %s", path.name, e)
        return None
    return _decoded_from_native_payload(payload)


def decode_texture(filepath: str) -> DecodedTexture | None:
    """Decode a texture file to raw bytes. No GL context required.

    Safe to call from background threads. Returns None on failure.

    DDS: directxtex_native.
    Non-DDS (PNG/TGA): Pillow.
    """
    path = Path(filepath)
    if not path.exists():
        return None

    if path.suffix.lower() == ".dds":
        try:
            decoded = _decode_dds_file(path)
            if decoded:
                return decoded
        except Exception as e:
            _log.debug("native DDS decode failed for %s: %s", path.name, e)

        # Pillow fallback for simple DDS (DXT1/DXT3/DXT5, uncompressed)
        try:
            img = Image.open(path).convert("RGBA")
            return DecodedTexture(size=img.size, components=4, data=img.tobytes())
        except Exception as e:
            _log.debug("Pillow DDS fallback failed for %s: %s", path.name, e)
    else:
        # Non-DDS (PNG, TGA, BMP, etc.) — Pillow handles these natively
        try:
            img = Image.open(path).convert("RGBA")
            return DecodedTexture(size=img.size, components=4, data=img.tobytes())
        except Exception as e:
            _log.debug("Pillow decode failed for %s: %s", path.name, e)

    return None


def decode_texture_bytes(data: bytes, name: str = "") -> DecodedTexture | None:
    """Decode a texture from in-memory bytes (e.g. from BA2 archive).

    Same pipeline as decode_texture() but from bytes instead of a file path.
    DDS: directxtex_native -> Pillow fallback. Non-DDS: Pillow.
    """
    import io

    is_dds = len(data) >= 4 and data[:4] == b"DDS "

    if is_dds:
        try:
            with tempfile.NamedTemporaryFile(suffix=".dds", delete=False) as tmp:
                tmp.write(data)
                tmp_path = Path(tmp.name)
            result = _decode_dds_file(tmp_path)
            tmp_path.unlink(missing_ok=True)
            if result:
                return result
        except Exception as e:
            _log.debug("native DDS decode failed for %s: %s", name, e)

        # Pillow fallback for simple DDS
        try:
            img = Image.open(io.BytesIO(data)).convert("RGBA")
            return DecodedTexture(size=img.size, components=4, data=img.tobytes())
        except Exception as e:
            _log.debug("Pillow DDS fallback failed for %s: %s", name, e)
    else:
        # Non-DDS (PNG, TGA, etc.)
        try:
            img = Image.open(io.BytesIO(data)).convert("RGBA")
            return DecodedTexture(size=img.size, components=4, data=img.tobytes())
        except Exception as e:
            _log.debug("Pillow decode failed for %s: %s", name, e)

    return None


def upload_decoded(
    ctx: moderngl.Context,
    decoded: DecodedTexture,
    *,
    build_mipmaps: bool = True,
) -> moderngl.Texture:
    """Upload pre-decoded texture data to the GPU. Must be called on the GL thread."""
    tex = ctx.texture(decoded.size, decoded.components, decoded.data)
    if build_mipmaps:
        tex.build_mipmaps()
        tex.filter = (moderngl.LINEAR_MIPMAP_LINEAR, moderngl.LINEAR)
        tex.anisotropy = 4.0
    else:
        tex.filter = (moderngl.LINEAR, moderngl.LINEAR)
    return tex


def load_texture(ctx: moderngl.Context, filepath: str) -> moderngl.Texture:
    """Load a texture file into a ModernGL texture (synchronous, single-phase)."""
    path = Path(filepath)
    if not path.exists():
        _log.warning("Texture not found: %s", filepath)
        return _error_texture(ctx)

    decoded = decode_texture(filepath)
    if decoded:
        return upload_decoded(ctx, decoded)

    _log.warning("Could not load texture: %s", filepath)
    return _error_texture(ctx)


def load_cubemap(ctx: moderngl.Context, filepath: str) -> moderngl.Texture:
    """Load a DDS environment map as a regular 2D texture.

    FO4 env maps are single 2D images (latlong projection), not 6-face cubemaps.
    The shader converts the reflection vector to UV coordinates.

    Falls back to 1x1 gray texture on failure.
    """
    path = Path(filepath)
    if not path.is_file():
        return _fallback_envmap(ctx)

    decoded = decode_texture(str(path))
    if decoded is None:
        _log.warning("  envmap %s: decode failed", path.name)
        return _fallback_envmap(ctx)

    tex = upload_decoded(ctx, decoded, build_mipmaps=False)
    _log.info("  envmap loaded %s  size=%dx%d", path.name, decoded.size[0], decoded.size[1])
    return tex


def load_cubemap_bytes(ctx: moderngl.Context, data: bytes,
                       name: str = "") -> moderngl.Texture | None:
    """Load a DDS environment map from in-memory bytes (e.g. from BA2).

    FO4 env maps are single 2D images. Returns a regular 2D texture or None.
    """
    decoded = decode_texture_bytes(data, name=name)
    if decoded is None:
        _log.warning("  envmap bytes %s: decode failed", name)
        return None

    tex = upload_decoded(ctx, decoded, build_mipmaps=False)
    _log.info("  envmap loaded from bytes %s  size=%dx%d",
              name, decoded.size[0], decoded.size[1])
    return tex


def _decode_dds_file(path: Path) -> DecodedTexture | None:
    """Decompress DDS to raw bytes through directxtex_native. No GL context needed.

    BC5 normal Z is reconstructed in the shader (normal_decode.glsl).
    """
    native_decoded = _decode_via_native_runtime(path)
    if native_decoded is not None:
        return native_decoded
    try:
        img = _dds_load_image(str(path))
        return DecodedTexture(size=img.size, components=4, data=img.tobytes())
    except Exception as e:
        _log.warning("DDS decode failed for %s: %s", path.name, e)
        return None


def batch_decode_dds(paths: list[Path]) -> dict[str, DecodedTexture | None]:
    """Batch-decode multiple DDS files through directxtex_native.

    Returns {str(path): DecodedTexture | None} for each input path.
    """
    if not paths:
        return {}

    results: dict[str, DecodedTexture | None] = {}
    cpu_count = os.cpu_count() or 8
    workers = max(1, min(len(paths), max(8, cpu_count // 2), 16))

    with ThreadPoolExecutor(max_workers=workers) as pool:
        futures = {pool.submit(_decode_dds_file, p): p for p in paths}
        for fut in as_completed(futures):
            path = futures[fut]
            try:
                results[str(path)] = fut.result()
            except Exception as e:
                _log.debug("Native batch decode failed for %s: %s", path.name, e)
                results[str(path)] = None

    return results


def _load_via_directxtex(ctx: moderngl.Context, path: Path) -> moderngl.Texture:
    """Decode DDS through directxtex_native, then upload."""
    decoded = _decode_dds_file(path)
    if not decoded:
        raise RuntimeError(f"DDS decode failed for {path.name}")
    return upload_decoded(ctx, decoded)


def _error_texture(ctx: moderngl.Context) -> moderngl.Texture:
    """1x1 magenta error texture."""
    return ctx.texture((1, 1), 4, b'\xff\x00\xff\xff')


def _fallback_envmap(ctx: moderngl.Context) -> moderngl.Texture:
    """1x1 gray 2D env map fallback."""
    return ctx.texture((1, 1), 4, b'\x40\x40\x40\xff')
