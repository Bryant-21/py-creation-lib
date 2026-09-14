"""FO76 → FO4 conversion parity (Python vs Rust).

Skips when `FO76_EXTRACTED_DIR` (from `.env`) is unset or doesn't exist.
Picks a handful of representative FO76 HKX files (weapon behaviors,
weapon animations, behavior wrappers) and verifies that
`creation_lib.havok_convert.HavokConverter` (Python) and
`creation_lib._native.havok_native.havok_convert_bytes` (Rust) produce
semantically equivalent FO4-compatible output. Byte-exact equality is not
required; the FO76 byte-exact fixture suite covers the core packfile path.
"""

from __future__ import annotations

import os
import tempfile
from pathlib import Path

import pytest

FO4_VERSION_ID = 53


def _fo76_extracted_dir() -> Path | None:
    raw = os.environ.get("FO76_EXTRACTED_DIR")
    if not raw:
        return None
    candidate = Path(raw)
    return candidate if candidate.exists() else None


_FIXTURES = [
    "meshes/actors/character/behaviors/weaponbehavior.hkx",
    "meshes/actors/character/behaviors/chargeupwrappingweaponbehavior.hkx",
    "meshes/actors/character/behaviors/powerarmorheavyweaponwrappingbehavior.hkx",
]


def _resolve_fixtures(extracted_dir: Path) -> list[Path]:
    return [extracted_dir / rel for rel in _FIXTURES if (extracted_dir / rel).exists()]


@pytest.mark.skipif(
    _fo76_extracted_dir() is None,
    reason="FO76_EXTRACTED_DIR env var not set or path doesn't exist",
)
@pytest.mark.parametrize("fixture_index", range(len(_FIXTURES)))
def test_fo76_conversion_python_and_rust_agree_semantically(fixture_index: int):
    """Convert one FO76 HKX through Python and Rust and compare structure.

    Asserts:
      * Both outputs are non-empty.
      * Both parse cleanly as FO4-era HKX via `creation_lib.hkxpack.load_hkx`.
      * Both produce the same root object class.
      * Both produce the same total object count.
    """
    extracted = _fo76_extracted_dir()
    assert extracted is not None  # guarded by skipif

    fixtures = _resolve_fixtures(extracted)
    if not fixtures or fixture_index >= len(fixtures):
        pytest.skip(f"Fixture {_FIXTURES[fixture_index]} not present")

    src_path = fixtures[fixture_index]
    src_bytes = src_path.read_bytes()
    assert src_bytes, f"Empty source file: {src_path}"

    from creation_lib.havok_convert import HavokConverter

    py_converter = HavokConverter()
    py_bytes = py_converter.convert_bytes(src_bytes, target_version=FO4_VERSION_ID)

    import creation_lib._native.havok_native as havok_native

    rust_bytes = havok_native.havok_convert_bytes(src_bytes, "fo4")

    assert py_bytes, f"Python converter produced empty output for {src_path.name}"
    assert rust_bytes, f"Rust converter produced empty output for {src_path.name}"

    from creation_lib.hkxpack import load_hkx

    with tempfile.TemporaryDirectory() as tmpdir:
        py_path = Path(tmpdir) / "py.hkx"
        py_path.write_bytes(py_bytes)
        rust_path = Path(tmpdir) / "rust.hkx"
        rust_path.write_bytes(rust_bytes)
        py_hkx, _ = load_hkx(str(py_path))
        rust_hkx, _ = load_hkx(str(rust_path))

    py_root = py_hkx.objects[0].class_name if py_hkx.objects else None
    rust_root = rust_hkx.objects[0].class_name if rust_hkx.objects else None
    assert py_root == rust_root, (
        f"Root class mismatch for {src_path.name}: "
        f"Python={py_root!r}, Rust={rust_root!r}"
    )

    py_count = len(py_hkx.objects)
    rust_count = len(rust_hkx.objects)
    assert py_count == rust_count, (
        f"Object count mismatch for {src_path.name}: "
        f"Python={py_count}, Rust={rust_count}"
    )
