"""End-to-end validation: native `bsarchive_native` extract vs. Python oracle.

Each archive in the sample set is extracted via the native module and via the
existing pure-Python reader; every file must byte-match. The sample set is
small (first N entries per archive) so test runtime stays under a minute; full
coverage is exercised manually when validating format changes.

Archives are pulled from the local game install, controlled by environment
variables so CI can skip cleanly. Set in `.env`:

  FO4_DIR         — Fallout 4 install root
  SKYRIMSE_DIR    — Skyrim Special Edition install root
  STARFIELD_DIR   — Starfield install root

Or override with test-scoped overrides:

  MODKIT_TEST_FO4_DATA         — directory containing FO4 BA2s
  MODKIT_TEST_SKYRIMSE_DATA    — directory containing SSE BSAs
  MODKIT_TEST_STARFIELD_DATA   — directory containing Starfield BA2s
"""
from __future__ import annotations

import os
from pathlib import Path

import pytest

from creation_lib.ba2 import native_runtime

# Limit sample size per archive to keep runtime sane. Each sampled file is
# fully decompressed and byte-compared.
SAMPLE_PER_ARCHIVE = 40


def _resolve_data_dir(test_env: str, env_var: str) -> Path | None:
    raw = os.environ.get(test_env) or os.environ.get(env_var)
    if not raw:
        return None
    path = Path(raw)
    if path.name.lower() != "data":
        path = path / "Data"
    return path if path.is_dir() else None


FO4_DATA = _resolve_data_dir("MODKIT_TEST_FO4_DATA", "FO4_DIR")
SSE_DATA = _resolve_data_dir("MODKIT_TEST_SKYRIMSE_DATA", "SKYRIMSE_DIR")
STARFIELD_DATA = _resolve_data_dir("MODKIT_TEST_STARFIELD_DATA", "STARFIELD_DIR")


def _skip_if_missing(path: Path | None, label: str) -> Path:
    if path is None:
        pytest.skip(f"{label} not configured (set env var or .env)")
    return path


def _require_native():
    try:
        return native_runtime.load_native_module()
    except RuntimeError:
        pytest.skip("bsarchive_native extension not built — run maturin develop")


def _sample_files(all_files: list[str]) -> list[str]:
    # Deterministic sample: first N by sorted order.
    return sorted(all_files)[:SAMPLE_PER_ARCHIVE]


def _oracle_extract_ba2(archive_path: Path, file_path: str) -> bytes:
    from creation_lib.ba2.ba2_reader import BA2File

    reader = BA2File(archive_path)
    try:
        data = reader.extract(file_path)
    finally:
        reader.close()
    assert data is not None, f"oracle missing file: {file_path}"
    return data


def _oracle_extract_bsa(archive_path: Path, file_path: str) -> bytes:
    from creation_lib.ba2.bsa_reader import BSAReader

    reader = BSAReader(archive_path)
    try:
        data = reader.extract(file_path)
    finally:
        reader.close()
    assert data is not None, f"oracle missing file: {file_path}"
    return data


_DDS_MAGIC = b"DDS "
_DDS_HEADER_FIXED_SIZE = 128  # DDS_MAGIC + DDS_HEADER
_DDS_HEADER_DXT10_SIZE = 20


def _strip_dds_header(data: bytes) -> bytes:
    """Return the pixel-data portion of a DDS file, skipping the header.

    The native path uses DirectXTex to rebuild the DDS header from the
    stored `DXGI_FORMAT`. The Python oracle hand-crafts the header in
    `py_creation_lib/python/creation_lib/ba2/dds_header.py`. Both headers describe the same texture but
    their byte layout diverges (DirectXTex sets `dwDepth=1`, legacy
    `dwPitchOrLinearSize` choice, etc.). The pixel payload after the
    header is the load-bearing content that the game reads; comparing
    that proves the chunks were decompressed and concatenated identically.
    """
    if not data.startswith(_DDS_MAGIC):
        return data
    header_end = _DDS_HEADER_FIXED_SIZE
    # If the fourCC at offset 84 is "DX10", skip the 20-byte DXT10 extension.
    if len(data) >= header_end and data[84:88] == b"DX10":
        header_end += _DDS_HEADER_DXT10_SIZE
    return data[header_end:]


def _assert_extract_matches(archive: Path, oracle):
    _require_native()
    files = native_runtime.list_archive(str(archive))
    assert files, f"native reported no files in {archive.name}"
    for rel in _sample_files(files):
        native_bytes = native_runtime.extract_one(str(archive), rel)
        oracle_bytes = oracle(archive, rel)
        # For DDS files, DirectXTex and our hand-built header diverge in
        # non-material fields (depth/pitch). Compare pixel payloads.
        if rel.endswith(".dds"):
            native_payload = _strip_dds_header(native_bytes)
            oracle_payload = _strip_dds_header(oracle_bytes)
            assert native_payload == oracle_payload, (
                f"pixel payload mismatch for {rel} in {archive.name}: "
                f"native={len(native_payload)}B oracle={len(oracle_payload)}B"
            )
        else:
            assert native_bytes == oracle_bytes, (
                f"byte mismatch for {rel} in {archive.name}: "
                f"native={len(native_bytes)}B oracle={len(oracle_bytes)}B"
            )


# --- FO4 -------------------------------------------------------------------


@pytest.fixture(scope="module")
def fo4_data() -> Path:
    return _skip_if_missing(FO4_DATA, "FO4_DIR")


def test_fo4_misc_extract(fo4_data: Path):
    archive = fo4_data / "Fallout4 - Misc.ba2"
    if not archive.is_file():
        pytest.skip(f"{archive} not present")
    _assert_extract_matches(archive, _oracle_extract_ba2)


def test_fo4_textures_extract(fo4_data: Path):
    archive = fo4_data / "Fallout4 - Textures1.ba2"
    if not archive.is_file():
        pytest.skip(f"{archive} not present")
    _assert_extract_matches(archive, _oracle_extract_ba2)


# --- Skyrim SE -------------------------------------------------------------


@pytest.fixture(scope="module")
def sse_data() -> Path:
    return _skip_if_missing(SSE_DATA, "SKYRIMSE_DIR")


def test_sse_misc_extract(sse_data: Path):
    archive = sse_data / "Skyrim - Misc.bsa"
    if not archive.is_file():
        pytest.skip(f"{archive} not present")
    _assert_extract_matches(archive, _oracle_extract_bsa)


# --- Starfield -------------------------------------------------------------


@pytest.fixture(scope="module")
def sf_data() -> Path:
    return _skip_if_missing(STARFIELD_DATA, "STARFIELD_DIR")


def test_starfield_misc_extract(sf_data: Path):
    archive = sf_data / "Starfield - Misc.ba2"
    if not archive.is_file():
        pytest.skip(f"{archive} not present")
    _assert_extract_matches(archive, _oracle_extract_ba2)


# --- archive_info ---------------------------------------------------------


def test_archive_info_fo4_misc(fo4_data: Path):
    _require_native()
    archive = fo4_data / "Fallout4 - Misc.ba2"
    if not archive.is_file():
        pytest.skip(f"{archive} not present")
    info = native_runtime.archive_info(str(archive))
    assert info is not None
    assert info["format"] in {"fo4_gnrl", "fo4_dx10"}
    assert info["file_count"] > 0


def test_archive_info_sse_misc(sse_data: Path):
    _require_native()
    archive = sse_data / "Skyrim - Misc.bsa"
    if not archive.is_file():
        pytest.skip(f"{archive} not present")
    info = native_runtime.archive_info(str(archive))
    assert info is not None
    assert info["format"] == "tes4"
    assert info["version"] in {103, 104, 105}


# --- bulk extract --------------------------------------------------------


def test_extract_archive_writes_files(tmp_path, sse_data: Path):
    _require_native()
    archive = sse_data / "Skyrim - Misc.bsa"
    if not archive.is_file():
        pytest.skip(f"{archive} not present")
    # Extract a small archive end-to-end and verify file count matches list.
    native_count = native_runtime.extract_archive(
        str(archive), str(tmp_path), workers=2
    )
    expected_count = len(native_runtime.list_archive(str(archive)))
    assert native_count == expected_count, (
        f"extract_archive wrote {native_count} files, listing reports {expected_count}"
    )
    # Spot-check: at least one .seq file landed on disk with non-zero size.
    seq_files = list(tmp_path.rglob("*.seq"))
    assert seq_files, "expected at least one .seq file extracted"
    assert all(f.stat().st_size > 0 for f in seq_files)


# --- unavailable native module ------------------------------------------


def test_python_wrappers_raise_when_module_missing(monkeypatch):
    # Simulate import failure by clearing the cached module.
    monkeypatch.setattr(native_runtime, "_NATIVE_MODULE", None)
    monkeypatch.setattr(native_runtime, "_NATIVE_IMPORT_ATTEMPTED", True)
    with pytest.raises(RuntimeError, match="required for archive operations"):
        native_runtime.list_archive("unused")
    with pytest.raises(RuntimeError, match="required for archive operations"):
        native_runtime.archive_info("unused")
    with pytest.raises(RuntimeError, match="required for archive operations"):
        native_runtime.extract_one("unused", "unused")
    with pytest.raises(RuntimeError, match="required for archive operations"):
        native_runtime.extract_archive("unused", "unused")
    with pytest.raises(RuntimeError, match="required for archive operations"):
        native_runtime.pack_archive("unused", "unused", "fo4")
