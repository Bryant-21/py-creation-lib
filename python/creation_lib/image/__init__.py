"""Image processing utilities -- dilation fill, MIP flooding, color analysis, upscaling.

All functions accept explicit parameters -- no global config.
"""

from __future__ import annotations

import json
import logging
import os
import shutil
import subprocess
import tempfile
import urllib.request
from pathlib import Path
from typing import Optional

from PIL import Image, ImageChops

_log = logging.getLogger("creation_lib.image")

# Supported file extensions for image counting
IMAGE_EXTS = (".png", ".jpg", ".jpeg", ".dds")

# Known ChaiNNer model URLs and extensions
MODEL_URLS = {
    "UltraSharpV2": "https://huggingface.co/Kim2091/UltraSharpV2/resolve/main/4x-UltraSharpV2.safetensors?download=true",
    "4x-Normal-RG0-BC1": "https://github.com/RunDevelopment/ESRGAN-models/raw/main/normals/4x-Normal-RG0-BC1.pth?download=true",
    "4x-Normal-RG0-BC7": "https://github.com/RunDevelopment/ESRGAN-models/raw/main/normals/4x-Normal-RG0-BC7.pth?download=true",
    "4x-Normal-RG0": "https://github.com/RunDevelopment/ESRGAN-models/raw/main/normals/4x-Normal-RG0.pth?download=true",
    "4x-PBRify_UpscalerV4": "https://github.com/Kim2091/Kim2091-Models/releases/download/4x-PBRify_UpscalerV4/4x-PBRify_UpscalerV4.pth?download=true",
    "4xTextures_GTAV_rgt-s_dither": "https://huggingface.co/Phips/4xTextures_GTAV_rgt-s_dither/resolve/main/4xTextures_GTAV_rgt-s_dither.safetensors?download=true",
    "4x-PBRify_UpscalerSIR-M_V2": "https://github.com/Kim2091/Kim2091-Models/releases/download/4x-PBRify_UpscalerSIR-M_V2/4x-PBRify_UpscalerSIR-M_V2.pth?download=true",
    "4xNomosWebPhoto_RealPLKSR": "https://github.com/Phhofm/models/releases/download/4xNomosWebPhoto_RealPLKSR/4xNomosWebPhoto_RealPLKSR.pth?download=true",
}

MODEL_EXTS = {
    "UltraSharpV2": ".safetensors",
    "4x-Normal-RG0-BC1": ".pth",
    "4x-Normal-RG0-BC7": ".pth",
    "4x-Normal-RG0": ".pth",
    "4x-PBRify_UpscalerV4": ".pth",
    "4xTextures_GTAV_rgt-s_dither": ".safetensors",
    "4x-PBRify_UpscalerSIR-M_V2": ".pth",
    "4xNomosWebPhoto_RealPLKSR": ".pth",
}


# ---------------------------------------------------------------------------
# Dilation fill (color bleed for transparent textures)
# ---------------------------------------------------------------------------

def dilation_fill(rgba_img: Image.Image, out_path: str | Path | None = None,
                  max_iters: int = 64) -> tuple[Image.Image, bool]:
    """Apply dilation fill to transparent regions of an RGBA image.

    Expands opaque pixel colors into adjacent transparent pixels using
    8-directional non-wrapping shifts.

    Args:
        rgba_img: Input RGBA PIL Image.
        out_path: Optional path to save the result as PNG.
        max_iters: Maximum number of dilation iterations.

    Returns:
        (result_image, filled_any) -- the processed image and whether any fill occurred.
    """
    if rgba_img.mode != 'RGBA':
        rgba_img = rgba_img.convert('RGBA')

    r, g, b, a = rgba_img.split()
    base_rgb = Image.merge('RGB', (r, g, b))
    known = a.point(lambda v: 255 if v > 0 else 0)
    unknown = a.point(lambda v: 0 if v > 0 else 255)
    unknown_initial = unknown.copy()

    if unknown.getbbox() is None:
        if out_path:
            rgba_img.save(str(out_path), format='PNG')
        return rgba_img, False

    neighbors = [
        (-1, 0), (1, 0), (0, -1), (0, 1),
        (-1, -1), (-1, 1), (1, -1), (1, 1),
    ]

    def shift_no_wrap(img: Image.Image, dx: int, dy: int) -> Image.Image:
        w, h = img.size
        result = Image.new(img.mode, (w, h))
        src_x0 = max(0, -dx)
        src_y0 = max(0, -dy)
        src_x1 = min(w, w - dx) if dx >= 0 else w
        src_y1 = min(h, h - dy) if dy >= 0 else h
        if src_x0 >= src_x1 or src_y0 >= src_y1:
            return result
        region = img.crop((src_x0, src_y0, src_x1, src_y1))
        dst_x = max(0, dx)
        dst_y = max(0, dy)
        result.paste(region, (dst_x, dst_y))
        return result

    filled_any = False
    for _ in range(max_iters):
        iter_filled = False
        for dx, dy in neighbors:
            shifted_known = shift_no_wrap(known, dx, dy)
            fill_mask = ImageChops.multiply(shifted_known, unknown)
            if fill_mask.getbbox() is None:
                continue
            shifted_rgb = shift_no_wrap(base_rgb, dx, dy)
            base_rgb.paste(shifted_rgb, mask=fill_mask)
            unknown = ImageChops.subtract(unknown, fill_mask)
            known = ImageChops.lighter(known, fill_mask)
            iter_filled = True
            filled_any = True
        if not iter_filled:
            break

    filled_mask = ImageChops.subtract(unknown_initial, unknown)
    new_alpha = ImageChops.lighter(a, filled_mask)
    nr, ng, nb = base_rgb.split()
    out_img = Image.merge('RGBA', (nr, ng, nb, new_alpha))

    if out_path:
        out_img.save(str(out_path), format='PNG')
        if filled_any:
            _log.info("Color fill applied: %s", out_path)

    return out_img, filled_any


# ---------------------------------------------------------------------------
# ChaiNNer upscaling integration
# ---------------------------------------------------------------------------

def json_safe_path(path: str) -> str:
    """Return a JSON-safe absolute path with forward slashes."""
    try:
        return os.path.abspath(path).replace("\\", "/")
    except Exception:
        return str(path).replace("\\", "/")


def resolve_model_path(model_name: str, models_dir: str) -> Optional[str]:
    """Resolve a local model path by trying known extensions.

    Returns None if not found.
    """
    os.makedirs(models_dir, exist_ok=True)

    if os.path.isabs(model_name) and os.path.exists(model_name):
        return model_name

    given_path = os.path.join(models_dir, model_name)
    name_has_ext = os.path.splitext(model_name)[1] != ""

    candidates: list[str] = []
    if name_has_ext:
        candidates.append(given_path)
    else:
        known_ext = MODEL_EXTS.get(model_name)
        if known_ext:
            candidates.append(os.path.join(models_dir, model_name + known_ext))
        for ext in (".pth", ".safetensors", ".onnx"):
            if known_ext != ext:
                candidates.append(os.path.join(models_dir, model_name + ext))
        candidates.append(os.path.join(models_dir, model_name))

    for c in candidates:
        if os.path.exists(c):
            return c
    return None


def download_model(model_name: str, models_dir: str) -> str:
    """Download model by name. Returns local path."""
    os.makedirs(models_dir, exist_ok=True)
    url = MODEL_URLS.get(model_name)
    if not url:
        raise ValueError(f"No download URL configured for model '{model_name}'.")

    ext = MODEL_EXTS.get(model_name, ".onnx")
    base = model_name
    if not os.path.splitext(base)[1]:
        base += ext
    target = os.path.join(models_dir, base)

    tmp_path, _ = urllib.request.urlretrieve(url)
    if os.path.exists(target):
        os.remove(target)
    shutil.move(tmp_path, target)
    _log.info("Downloaded model %s -> %s", model_name, target)
    return target


def get_or_download_model(model_name: str, models_dir: str) -> str:
    """Return local path to model; download if not present."""
    local = resolve_model_path(model_name, models_dir)
    if local and os.path.exists(local):
        return local
    return download_model(model_name, models_dir)


def parse_chain_for_ids(chain_path: str) -> dict[str, str]:
    """Parse a ChaiNNer .chn (JSON) file and extract node IDs."""
    try:
        with open(chain_path, 'r', encoding='utf-8') as f:
            data = json.load(f)
        nodes = data.get('content', {}).get('nodes', [])
        result: dict[str, str] = {}
        for node in nodes:
            d = node.get('data', {})
            schema = (d.get('schemaId') or '').lower()
            node_id = d.get('id') or node.get('id')
            if not schema or not node_id:
                continue
            if schema == 'chainner:image:load' and 'load' not in result:
                result['load'] = node_id
            elif schema == 'chainner:image:load_images' and 'load_images' not in result:
                result['load_images'] = node_id
            elif schema == 'chainner:pytorch:load_model' and 'load_model' not in result:
                result['load_model'] = node_id
            elif schema == 'chainner:image:save' and 'save' not in result:
                result['save'] = node_id
            elif schema == 'chainner:pytorch:upscale_image' and 'upscale' not in result:
                result['upscale'] = node_id
        return result
    except Exception as e:
        _log.exception('Failed parsing chain file %s: %s', chain_path, e)
        return {}


def run_chainner(input_png: str, model_path: str, out_dir: str,
                 expected_output_png: str,
                 chainner_path: str, chain_path: str) -> bool:
    """Invoke ChaiNNer CLI for single-image upscale. Returns True on success.

    Args:
        input_png: Path to input PNG image.
        model_path: Path to the model file.
        out_dir: Output directory.
        expected_output_png: Expected output filename for verification.
        chainner_path: Path to ChaiNNer.exe.
        chain_path: Path to the .chn chain file.
    """
    if not os.path.exists(chain_path):
        raise FileNotFoundError(f"ChaiNNer chain not found: {chain_path}")

    ids = parse_chain_for_ids(chain_path)
    save_id = ids.get("save")
    load_id = ids.get("load")
    load_model_id = ids.get("load_model")
    upscale_id = ids.get("upscale")
    if not (save_id and load_id and load_model_id):
        raise ValueError("Required nodes not found in chain")

    # Compute upscale factor targeting ~4K
    try:
        with Image.open(input_png) as _im:
            w, h = _im.size
        longest = max(w, h)
    except Exception:
        longest = 0
    factor = max(1, int(round(4096 / float(longest)))) if longest > 0 else 1

    inputs_map = {
        f"#{load_id}:0": json_safe_path(input_png),
        f"#{load_model_id}:0": json_safe_path(model_path),
        f"#{save_id}:1": json_safe_path(out_dir),
    }
    if upscale_id:
        inputs_map[f"#{upscale_id}:5"] = factor

    overrides = {"inputs": inputs_map}
    ov_fd, ov_path = tempfile.mkstemp(prefix="overrides_", suffix=".json")
    os.close(ov_fd)
    with open(ov_path, "w", encoding="utf-8") as f:
        json.dump(overrides, f)

    cmd = [chainner_path, "run", chain_path, "--override", ov_path]
    _log.info("Running ChaiNNer: %s", " ".join(cmd))
    try:
        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        if proc.stdout:
            for line in proc.stdout:
                _log.info("[ChaiNNer] %s", line.rstrip())
        proc.wait()
        return proc.returncode == 0 and os.path.exists(expected_output_png)
    except Exception as e:
        _log.exception("Failed to launch ChaiNNer: %s", e)
        return False


def run_chainner_directory(folder: str, model_name: str, out_dir: str,
                           glob_pattern: str,
                           chainner_path: str, chain_path: str,
                           models_dir: str) -> bool:
    """Invoke ChaiNNer CLI for multi-image upscale. Returns True on success."""
    if not os.path.exists(chain_path):
        raise FileNotFoundError(f"ChaiNNer multi-image chain not found: {chain_path}")

    ids = parse_chain_for_ids(chain_path)
    save_id = ids.get("save")
    load_images_id = ids.get("load_images")
    load_model_id = ids.get("load_model")
    if not (save_id and load_images_id and load_model_id):
        raise ValueError("Required nodes not found in multi chain")

    model_path = get_or_download_model(model_name, models_dir)

    inputs_map = {
        f"#{load_images_id}:0": json_safe_path(folder),
        f"#{load_images_id}:3": glob_pattern,
        f"#{load_model_id}:0": json_safe_path(model_path),
        f"#{save_id}:1": json_safe_path(out_dir),
    }
    overrides = {"inputs": inputs_map}
    ov_fd, ov_path = tempfile.mkstemp(prefix="overrides_dir_", suffix=".json")
    os.close(ov_fd)
    with open(ov_path, "w", encoding="utf-8") as f:
        json.dump(overrides, f)

    cmd = [chainner_path, "run", chain_path, "--override", ov_path]
    _log.info("Running ChaiNNer (dir): %s | glob=%s | model=%s", folder, glob_pattern, model_name)
    try:
        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        if proc.stdout:
            for line in proc.stdout:
                _log.info("[ChaiNNer] %s", line.rstrip())
        proc.wait()
        return proc.returncode == 0
    except Exception as e:
        _log.exception("Failed to launch ChaiNNer (dir): %s", e)
        return False


def count_images_by_suffix(folder: str, include_subdirs: bool) -> tuple[int, int]:
    """Return counts for (_d, _n) images in folder."""
    d_count = 0
    n_count = 0
    try:
        if include_subdirs:
            for root, _, files in os.walk(folder):
                for fn in files:
                    ext = os.path.splitext(fn)[1].lower()
                    if ext not in IMAGE_EXTS:
                        continue
                    stem = os.path.splitext(fn)[0].lower()
                    if stem.endswith("_d"):
                        d_count += 1
                    elif stem.endswith("_n"):
                        n_count += 1
        else:
            for fn in os.listdir(folder):
                if not os.path.isfile(os.path.join(folder, fn)):
                    continue
                ext = os.path.splitext(fn)[1].lower()
                if ext not in IMAGE_EXTS:
                    continue
                stem = os.path.splitext(fn)[0].lower()
                if stem.endswith("_d"):
                    d_count += 1
                elif stem.endswith("_n"):
                    n_count += 1
    except Exception as e:
        _log.warning("Failed counting images in %s: %s", folder, e)
    return d_count, n_count


def upscale_directory_two_pass(folder: str, out_dir: str | None,
                               include_subdirs: bool,
                               textures_model_name: str,
                               normals_model_name: str,
                               chainner_path: str,
                               chain_path: str,
                               models_dir: str) -> tuple[int, int, int]:
    """Run ChaiNNer twice: diffuse (_d) then normals (_n).

    Returns (saved, skipped, failed).
    """
    if not out_dir:
        out_dir = folder
    os.makedirs(out_dir, exist_ok=True)

    base = "**/*" if include_subdirs else "*"
    glob_d = f"{base}_d*"
    glob_n = f"{base}_n*"

    d_count, n_count = count_images_by_suffix(folder, include_subdirs)
    saved = skipped = failed = 0

    if d_count > 0:
        ok_d = run_chainner_directory(folder, textures_model_name, out_dir, glob_d,
                                      chainner_path, chain_path, models_dir)
        if ok_d:
            saved += d_count
        else:
            failed += d_count

    if n_count > 0:
        ok_n = run_chainner_directory(folder, normals_model_name, out_dir, glob_n,
                                      chainner_path, chain_path, models_dir)
        if ok_n:
            saved += n_count
        else:
            failed += n_count

    return saved, skipped, failed
