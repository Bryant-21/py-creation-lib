"""GPU BC7 routing through the dds.io write chokepoint.

The native write_dds_rgba binding routes BC7 to the GPU encoder (with silent
CPU fallback) when use_gpu is set, and keeps every other format on the CPU
path. These tests pin that contract from the Python boundary and hold on any
box -- with or without a GPU -- because they assert validity + invariants, not
GPU-specific bytes (the GPU-vs-CPU byte difference is exercised separately by
the conversion A/B, which only proves a GPU was present).
"""
from __future__ import annotations

import pytest
from PIL import Image

from creation_lib.dds import io as dds_io
from creation_lib.dds import native_runtime

pytestmark = pytest.mark.skipif(
    not native_runtime.native_function_available("write_dds_rgba"),
    reason="directxtex_native.write_dds_rgba unavailable",
)


def _make_img(w: int = 64, h: int = 64) -> Image.Image:
    # Gradient + cross term so BC7 has real per-block work to do.
    px = bytearray()
    for y in range(h):
        for x in range(w):
            px += bytes(((x * 4) & 255, (y * 4) & 255, (x * y) & 255, 255))
    return Image.frombytes("RGBA", (w, h), bytes(px))


def test_bc7_gpu_and_cpu_both_produce_valid_dds(tmp_path):
    img = _make_img()
    gpu = tmp_path / "g.dds"
    cpu = tmp_path / "c.dds"
    dds_io.save_image(img, str(gpu), format="BC7", use_gpu=True)
    dds_io.save_image(img, str(cpu), format="BC7", use_gpu=False)
    assert gpu.exists() and cpu.exists()
    for p in (gpu, cpu):
        back = dds_io.load_dds(str(p))
        assert back.size == (64, 64)


def test_cpu_path_is_deterministic(tmp_path):
    img = _make_img()
    a = tmp_path / "a.dds"
    b = tmp_path / "b.dds"
    dds_io.save_image(img, str(a), format="BC7", use_gpu=False)
    dds_io.save_image(img, str(b), format="BC7", use_gpu=False)
    assert a.read_bytes() == b.read_bytes()


def test_non_bc7_is_unaffected_by_use_gpu(tmp_path):
    # BC5 never reaches the GPU encoder, so use_gpu must not change the bytes.
    img = _make_img()
    g = tmp_path / "g_n.dds"
    c = tmp_path / "c_n.dds"
    dds_io.save_image(img, str(g), format="BC5", use_gpu=True)
    dds_io.save_image(img, str(c), format="BC5", use_gpu=False)
    assert g.read_bytes() == c.read_bytes()


def test_default_is_gpu_on():
    # The chokepoint defaults to GPU so every caller benefits without opting in.
    import inspect

    assert inspect.signature(dds_io.save_image).parameters["use_gpu"].default is True
    assert inspect.signature(dds_io.convert_to_dds).parameters["use_gpu"].default is True
    assert (
        inspect.signature(native_runtime.write_dds_rgba).parameters["use_gpu"].default
        is True
    )
