"""DDS file operations backed by directxtex_native."""

from __future__ import annotations

import os

from PIL import Image

from . import native_runtime


_FORMAT_ALIASES = {
    "BC1": "BC1_UNORM",
    "BC3": "BC3_UNORM",
    "BC4": "BC4_UNORM",
    "BC5": "BC5_UNORM",
    "BC6H": "BC6H_UF16",
    "BC7": "BC7_UNORM",
}


def _normalize_dds_format(format: str) -> str:
    value = (format or "BC7_UNORM").strip()
    return _FORMAT_ALIASES.get(value.upper(), value)


def _native_load_dds(path: str, mode: str) -> Image.Image | None:
    payload = native_runtime.read_dds_rgba(path)
    if payload is None:
        return None
    img = Image.frombytes(
        "RGBA",
        (int(payload["width"]), int(payload["height"])),
        bytes(payload["rgba"]),
    )
    return img.convert(mode)


def _native_convert_to_dds(
    input_path: str,
    output_path: str,
    *,
    format: str,
    generate_mips: bool,
    use_gpu: bool = True,
) -> bool:
    format = _normalize_dds_format(format)
    with Image.open(input_path) as im:
        rgba = im.convert("RGBA")
        return native_runtime.write_dds_rgba(
            output_path,
            rgba.width,
            rgba.height,
            rgba.tobytes(),
            format=format,
            generate_mips=generate_mips,
            use_gpu=use_gpu,
        )


def _native_save_dds_image(
    img: Image.Image,
    output_path: str,
    *,
    format: str,
    generate_mips: bool,
    use_gpu: bool = True,
) -> bool:
    format = _normalize_dds_format(format)
    rgba = img.convert("RGBA")
    return native_runtime.write_dds_rgba(
        output_path,
        rgba.width,
        rgba.height,
        rgba.tobytes(),
        format=format,
        generate_mips=generate_mips,
        use_gpu=use_gpu,
    )


def load_dds(path: str, mode: str = "RGBA") -> Image.Image:
    """Load a DDS file through directxtex_native."""
    if not native_runtime.native_function_available("read_dds_rgba"):
        raise RuntimeError("directxtex_native.read_dds_rgba is unavailable")
    native_img = _native_load_dds(path, mode)
    if native_img is None:
        raise RuntimeError("directxtex_native.read_dds_rgba is unavailable")
    return native_img


def convert_to_dds(input_path: str, output_path: str,
                   format: str = "BC7_UNORM", generate_mips: bool = False,
                   is_palette: bool = False, use_gpu: bool = True) -> None:
    """Convert image to DDS through directxtex_native.

    use_gpu routes BC7 output through the GPU encoder (with CPU fallback);
    it has no effect on other formats. Set False to force the CPU encoder.
    """
    dds_format = "R8G8B8A8_UNORM" if is_palette else _normalize_dds_format(format)
    if not native_runtime.native_function_available("write_dds_rgba"):
        raise RuntimeError("directxtex_native.write_dds_rgba is unavailable")
    if not _native_convert_to_dds(
        input_path,
        output_path,
        format=dds_format,
        generate_mips=generate_mips,
        use_gpu=use_gpu,
    ):
        raise RuntimeError("directxtex_native.write_dds_rgba is unavailable")


def load_image(path: str, mode: str = "RGBA") -> Image.Image:
    """Load any image. DDS files use directxtex_native."""
    ext = os.path.splitext(path)[1].lower()
    if ext == ".dds":
        return load_dds(path, mode)
    with Image.open(path) as im:
        return im.convert(mode)


def save_image(img: Image.Image, path: str, is_palette: bool = False, format: str | None = None,
               generate_mips: bool = False, use_gpu: bool = True) -> None:
    """Save image. DDS files use directxtex_native.

    use_gpu routes BC7 output through the GPU encoder (with CPU fallback);
    it has no effect on other formats. Set False to force the CPU encoder.
    """
    if path.lower().endswith(".dds"):
        dds_format = (
            "R8G8B8A8_UNORM"
            if is_palette
            else _normalize_dds_format(format or "BC7_UNORM")
        )
        if not native_runtime.native_function_available("write_dds_rgba"):
            raise RuntimeError("directxtex_native.write_dds_rgba is unavailable")
        if not _native_save_dds_image(
            img,
            path,
            format=dds_format,
            generate_mips=generate_mips,
            use_gpu=use_gpu,
        ):
            raise RuntimeError("directxtex_native.write_dds_rgba is unavailable")
    else:
        img.save(path)
