from __future__ import annotations

from pathlib import Path

import pytest

from creation_lib.dds import native_runtime


def _require_native():
    module = native_runtime.load_native_module()
    if module is None:
        pytest.skip("directxtex_native extension not built — run maturin develop")
    return module


def test_directxtex_native_roundtrip(tmp_path: Path):
    module = _require_native()
    width, height = 4, 4
    rgba = bytes(
        channel
        for y in range(height)
        for x in range(width)
        for channel in (x * 16, y * 16, 128, 255)
    )
    output_path = tmp_path / "roundtrip.dds"

    module.write_dds_rgba(
        str(output_path),
        width,
        height,
        rgba,
        format="BC7_UNORM",
        generate_mips=False,
    )

    assert output_path.is_file()
    payload = module.read_dds_rgba(str(output_path))
    assert int(payload["width"]) == width
    assert int(payload["height"]) == height
    assert len(bytes(payload["rgba"])) == width * height * 4

    info = module.texdiag_info(str(output_path))
    assert info["width"] == width
    assert info["height"] == height
    assert info["format"] == "BC7_UNORM"
    assert info["dimension"] == "2D"
    assert info["is_compressed"] is True


def test_convert_to_dds_palette_and_batch_resize_use_native_without_subprocess(
    monkeypatch, tmp_path: Path
):
    module = _require_native()
    from PIL import Image

    from creation_lib.dds.io import convert_to_dds
    from creation_lib.dds import batch_resize

    def fail_subprocess(*args, **kwargs):
        raise AssertionError("DDS subprocess should not run")

    monkeypatch.setattr("subprocess.run", fail_subprocess)

    input_path = tmp_path / "palette.png"
    output_path = tmp_path / "palette.dds"
    Image.new("RGBA", (4, 4), (16, 32, 48, 255)).save(input_path)
    convert_to_dds(
        str(input_path),
        str(output_path),
        is_palette=True,
        generate_mips=True,
    )
    assert output_path.is_file()

    input_dir = tmp_path / "input"
    output_dir = tmp_path / "output"
    input_dir.mkdir()
    src = input_dir / "normal_n.dds"
    rgba = bytes([128, 128, 255, 255] * 16)
    module.write_dds_rgba(
        str(src),
        4,
        4,
        rgba,
        format="BC5_UNORM",
        generate_mips=False,
    )

    result = batch_resize(
        str(input_dir),
        str(output_dir),
        [2],
        generate_mips=True,
        per_size_subfolders=False,
    )

    assert result["failed"] == 0
    assert (output_dir / "normal_n.dds").is_file()


def test_batch_resize_default_workers_use_half_cpu(monkeypatch, tmp_path: Path):
    from creation_lib import dds

    input_dir = tmp_path / "input"
    output_dir = tmp_path / "output"
    input_dir.mkdir()
    (input_dir / "texture.dds").write_bytes(b"dds")
    seen: dict[str, int] = {}

    class Future:
        def result(self) -> str:
            return "Copied: texture.dds"

    class Executor:
        def __init__(self, *, max_workers: int):
            seen["max_workers"] = max_workers

        def __enter__(self):
            return self

        def __exit__(self, *_args):
            return None

        def submit(self, *_args, **_kwargs):
            return Future()

    monkeypatch.setattr(dds.os, "cpu_count", lambda: 8)
    monkeypatch.setattr("concurrent.futures.ThreadPoolExecutor", Executor)
    monkeypatch.setattr("concurrent.futures.as_completed", lambda futures: futures)

    result = dds.batch_resize(
        str(input_dir),
        str(output_dir),
        [2],
    )

    assert result["processed"] == 1
    assert result["failed"] == 0
    assert seen == {"max_workers": 4}


def test_directxtex_runtime_wrappers_when_module_missing(monkeypatch):
    monkeypatch.setattr(native_runtime, "_NATIVE_MODULE", None)
    monkeypatch.setattr(native_runtime, "_NATIVE_IMPORT_ATTEMPTED", True)
    assert native_runtime.read_dds_rgba("unused") is None
    assert native_runtime.write_dds_rgba("unused", 1, 1, b"\x00\x00\x00\x00") is False
    assert native_runtime.texdiag_info("unused") is None


def test_directxtex_runtime_loads_umbrella_submodule(monkeypatch):
    monkeypatch.setattr(native_runtime, "_NATIVE_MODULE", None)
    monkeypatch.setattr(native_runtime, "_NATIVE_IMPORT_ATTEMPTED", False)

    umbrella = type(
        "Umbrella",
        (),
        {
            "directxtex_native": type(
                "DXT", (), {"read_dds_rgba": staticmethod(lambda path: {"path": path})}
            )()
        },
    )()

    def fake_import_module(name: str):
        if name == "directxtex_native":
            raise ImportError(name)
        if name == "creation_lib._native":
            return umbrella
        raise ImportError(name)

    monkeypatch.setattr(native_runtime, "import_module", fake_import_module)

    module = native_runtime.load_native_module()

    assert module is umbrella.directxtex_native


