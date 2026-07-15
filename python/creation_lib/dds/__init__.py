"""DDS texture utilities -- header reading, batch operations."""

from __future__ import annotations

import logging
import os
import shutil
import struct
from typing import Callable, Optional

from PIL import Image

from .io import (
    convert_to_dds,
    load_dds,
    load_image,
    save_image,
)

_log = logging.getLogger("creation_lib.dds")

# DXGI format constants
DXGI_FORMAT_BC7_UNORM = 98
DXGI_FORMAT_BC7_UNORM_SRGB = 99


def read_dds_size(path: str) -> tuple[int, int]:
    """Return (width, height) for a DDS file.

    Raises ValueError if not a DDS file or header too small.
    """
    with open(path, 'rb') as f:
        header = f.read(128)
    if len(header) < 128 or header[:4] != b'DDS ':
        raise ValueError('Not a DDS file')
    height = struct.unpack_from('<I', header, 12)[0]
    width = struct.unpack_from('<I', header, 16)[0]
    return width, height


def read_dds_dxgi_format(path: str) -> int | None:
    """Return DXGI format integer if DDS has DX10 header, else None.

    Only DX10+ DDS can be BC7. Safe to call on any DDS file path.
    """
    try:
        with open(path, 'rb') as f:
            header = f.read(148)  # 128 + at least 20 for DX10
        if len(header) < 128 or header[:4] != b'DDS ':
            return None
        # DDS_PIXELFORMAT dwFourCC at offset 84
        fourcc = header[84:88]
        if fourcc != b'DX10':
            return None
        if len(header) < 148:
            with open(path, 'rb') as f:
                header = f.read(148)
            if len(header) < 148:
                return None
        dxgi_fmt = struct.unpack_from('<I', header, 128)[0]
        return int(dxgi_fmt)
    except Exception:
        return None


def is_bc7_dxgi(dxgi: int | None) -> bool:
    """Check if DXGI format is BC7 (UNORM or SRGB)."""
    return dxgi in (DXGI_FORMAT_BC7_UNORM, DXGI_FORMAT_BC7_UNORM_SRGB)


def is_bc7_linear(dxgi: int | None) -> bool:
    """Check if DXGI format is BC7_UNORM (linear, not sRGB)."""
    return dxgi == DXGI_FORMAT_BC7_UNORM


def batch_resize(input_dir: str, output_dir: str, sizes: list[int],
                 generate_mips: bool = False,
                 no_upscale: bool = True, per_size_subfolders: bool = True,
                 bc3_convert: bool = False, recurse: bool = True,
                 downscale_method: str = "lanczos",
                 ignore_patterns: list[str] | None = None,
                 progress_callback: Callable[[int, int, str], None] | None = None,
                 cancel_check: Callable[[], bool] | None = None,
                 use_gpu: bool = True) -> dict:
    """Batch resize DDS files.

    Args:
        input_dir: Source directory containing DDS files.
        output_dir: Output directory for resized files.
        sizes: List of target sizes (max dimension).
        generate_mips: Whether to generate mipmaps.
        no_upscale: If True, copy files that are already at or below target size.
        per_size_subfolders: Create size-named subfolders in output_dir.
        bc3_convert: Convert BC7 textures to BC3 format.
        recurse: Recursively process subdirectories.
        downscale_method: Resampling method ("lanczos", "bicubic", etc.).
        ignore_patterns: Glob patterns for directories to skip.
        progress_callback: Called with (current, total, message).
        cancel_check: Returns True if operation should be cancelled.

    Returns:
        dict with keys: processed, failed, errors.
    """
    import concurrent.futures
    import fnmatch
    import threading

    sizes = sorted(set(int(s) for s in sizes if s > 0))
    result = {"processed": 0, "failed": 0, "errors": []}

    if not sizes:
        return result

    ignore_patterns = [p.strip().replace('\\', '/').lstrip('./') for p in (ignore_patterns or []) if p.strip()]

    def _match_ignored(rel_dir: str) -> bool:
        if not ignore_patterns:
            return False
        rel_dir = (rel_dir or '').strip()
        if not rel_dir or rel_dir == '.':
            return False
        norm = rel_dir.replace('\\', '/').strip('/')
        base = os.path.basename(norm)
        for pat in ignore_patterns:
            try:
                if fnmatch.fnmatchcase(norm, pat) or fnmatch.fnmatchcase(base, pat):
                    return True
            except Exception:
                pass
        return False

    # Gather DDS files
    dds_files: list[str] = []
    src_abs = os.path.abspath(input_dir)
    out_abs = os.path.abspath(output_dir)

    # Compute skip roots if output is under input
    skip_rel_roots: list[str] = []
    if os.path.normcase(out_abs).startswith(os.path.normcase(src_abs)):
        out_rel = os.path.relpath(out_abs, src_abs).replace('\\', '/').strip('/')
        out_rel_norm = '' if out_rel in ('', '.') else out_rel
        if per_size_subfolders and sizes:
            if out_rel_norm:
                skip_rel_roots = [f"{out_rel_norm}/{size}" for size in sizes]
            else:
                skip_rel_roots = [str(size) for size in sizes]
        else:
            skip_rel_roots = [out_rel_norm] if out_rel_norm else []
    skip_rel_roots = [sk for sk in skip_rel_roots if sk and sk != '.']

    def _should_skip_dir(rel_child: str) -> bool:
        rel_child = (rel_child or '').replace('\\', '/').strip('/')
        if any(rel_child == sk or rel_child.startswith(sk + '/') for sk in skip_rel_roots):
            return True
        return _match_ignored(rel_child)

    for root, dirs, files in os.walk(input_dir):
        rel_root = os.path.relpath(root, input_dir)
        if _should_skip_dir(rel_root):
            dirs[:] = []
            continue
        dirs[:] = [d for d in dirs if not _should_skip_dir(os.path.join(rel_root, d))]
        if not recurse:
            dirs[:] = []
        for fn in files:
            if fn.lower().endswith('.dds'):
                dds_files.append(os.path.join(root, fn))

    # Detect BC7 textures
    bc7_sources: set[str] = set()
    if bc3_convert:
        for src_path in dds_files:
            dxgi = read_dds_dxgi_format(src_path)
            if is_bc7_dxgi(dxgi):
                bc7_sources.add(src_path)

    total = len(dds_files) * len(sizes)
    if total == 0:
        return result

    os.makedirs(output_dir, exist_ok=True)
    for size in sizes:
        dest_root = os.path.join(output_dir, str(size)) if per_size_subfolders else output_dir
        os.makedirs(dest_root, exist_ok=True)

    processed = 0
    processed_lock = threading.Lock()

    def _resize_task(size: int, src_path: str) -> str:
        if cancel_check and cancel_check():
            return 'aborted'

        rel_path = os.path.relpath(src_path, input_dir)
        rel_dir = os.path.dirname(rel_path)
        dest_root = os.path.join(output_dir, str(size)) if per_size_subfolders else output_dir
        out_dir_task = os.path.join(dest_root, rel_dir)
        os.makedirs(out_dir_task, exist_ok=True)
        out_path = os.path.join(out_dir_task, os.path.basename(src_path))

        w = h = 0
        try:
            w, h = read_dds_size(src_path)
        except Exception:
            pass
        max_dim = max(w, h)

        needs_bc7_conversion = bc3_convert and src_path in bc7_sources

        if no_upscale and max_dim and max_dim <= size and not needs_bc7_conversion:
            try:
                shutil.copy2(src_path, out_path)
                return f'Copied (no upscale): {rel_path}'
            except Exception as e:
                return f'ERROR copying {rel_path}: {e}'

        return _pillow_resize_and_save(src_path, out_path, size, needs_bc7_conversion,
                                       w, h, no_upscale, generate_mips,
                                       downscale_method, input_dir, output_dir,
                                       use_gpu=use_gpu)

    # Process with thread pool
    threads = max(1, (os.cpu_count() or 2) // 2)
    with concurrent.futures.ThreadPoolExecutor(max_workers=threads) as ex:
        futures = []
        for size in sizes:
            for src_path in dds_files:
                futures.append(ex.submit(_resize_task, size, src_path))

        for fut in concurrent.futures.as_completed(futures):
            if cancel_check and cancel_check():
                break
            try:
                message = fut.result()
            except Exception as e:
                message = f'ERROR: {e}'

            with processed_lock:
                processed += 1
                p = processed

            if message.startswith('ERROR'):
                result["failed"] += 1
                result["errors"].append(message)
            elif message != 'aborted':
                result["processed"] += 1

            if progress_callback:
                progress_callback(p, total, message)

    return result


def _pillow_resize_and_save(src_path: str, out_path: str, size: int,
                            needs_bc7_conversion: bool,
                            orig_w: int, orig_h: int,
                            no_upscale: bool,
                            generate_mips: bool, downscale_method: str,
                            input_dir: str, output_dir: str,
                            use_gpu: bool = True) -> str:
    """Resize a DDS file using Pillow and re-encode natively."""
    target_fmt = 'BC3_UNORM' if needs_bc7_conversion else None

    if orig_w <= 0 or orig_h <= 0:
        try:
            im_probe = load_image(src_path, mode='RGBA')
            orig_w, orig_h = im_probe.width, im_probe.height
            im_probe.close()
        except Exception:
            orig_w, orig_h = 0, 0

    new_w, new_h = orig_w, orig_h
    if orig_w and orig_h:
        scale = size / max(orig_w, orig_h)
        if no_upscale and scale >= 1.0 and not needs_bc7_conversion:
            try:
                shutil.copy2(src_path, out_path)
                return f'Copied (no upscale): {os.path.relpath(src_path, input_dir)}'
            except Exception as e:
                return f'ERROR copying {src_path}: {e}'
        if scale < 1.0:
            new_w = max(1, int(round(orig_w * scale)))
            new_h = max(1, int(round(orig_h * scale)))

    resample_map = {
        'nearest': Image.Resampling.NEAREST,
        'bilinear': Image.Resampling.BILINEAR,
        'bicubic': Image.Resampling.BICUBIC,
        'lanczos': Image.Resampling.LANCZOS,
        'box': Image.Resampling.BOX,
        'hamming': Image.Resampling.HAMMING,
    }
    resample = resample_map.get(downscale_method, Image.Resampling.LANCZOS)

    try:
        im = load_image(src_path, mode='RGBA')
        if new_w != im.width or new_h != im.height:
            im = im.resize((new_w, new_h), resample=resample)
        save_image(im, out_path, is_palette=False,
                   generate_mips=generate_mips,
                   format=target_fmt or 'BC7_UNORM',
                   use_gpu=use_gpu)
        return f'Resized to {size}: {os.path.relpath(src_path, input_dir)}'
    except Exception as e:
        return f'ERROR Pillow resize for {os.path.relpath(src_path, input_dir)}: {e}'
    finally:
        try:
            im.close()
        except Exception:
            pass


def batch_to_png(input_dir: str, output_dir: str, recurse: bool = True,
                 progress_callback: Callable[[int, int, str], None] | None = None,
                 cancel_check: Callable[[], bool] | None = None) -> dict:
    """Batch convert DDS files to PNG.

    Args:
        input_dir: Source directory containing DDS files.
        output_dir: Output directory for PNG files.
        recurse: Recursively process subdirectories.
        progress_callback: Called with (current, total, message).
        cancel_check: Returns True if operation should be cancelled.

    Returns:
        dict with keys: processed, failed.
    """
    result = {"processed": 0, "failed": 0}

    dds_files: list[str] = []
    for root, dirs, files in os.walk(input_dir):
        if not recurse:
            dirs[:] = []
        for fn in files:
            if fn.lower().endswith('.dds'):
                dds_files.append(os.path.join(root, fn))

    total = len(dds_files)
    if total == 0:
        return result

    os.makedirs(output_dir, exist_ok=True)

    for i, dds_path in enumerate(dds_files):
        if cancel_check and cancel_check():
            break

        rel_path = os.path.relpath(dds_path, input_dir)
        out_subdir = os.path.join(output_dir, os.path.dirname(rel_path))
        os.makedirs(out_subdir, exist_ok=True)
        out_png = os.path.join(out_subdir, os.path.splitext(os.path.basename(dds_path))[0] + '.png')

        try:
            img = load_dds(dds_path, mode='RGBA')
            img.save(out_png)
            img.close()
            result["processed"] += 1
            msg = f'Converted: {rel_path}'
        except Exception as e:
            result["failed"] += 1
            msg = f'ERROR {rel_path}: {e}'
            _log.warning("Failed to convert %s: %s", dds_path, e)

        if progress_callback:
            progress_callback(i + 1, total, msg)

    return result
