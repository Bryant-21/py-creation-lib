"""Does FaceFXWrapper's Skyrim generator produce FNV-shaped lip?

FNV and FO3 ship no lip generator of their own. The bundled FaceFXWrapper
advertises only Skyrim and Fallout4 types, but all three games' lip files open
with the same version header, so the Skyrim type may be usable. This compares a
generated lip against a real one from the game archives.

Skipped unless New Vegas is installed.
"""
import os
import shutil
import struct
import wave
from pathlib import Path

import pytest

from creation_lib.audio.release import create_lip
from creation_lib.core.game_profiles import GAME_PROFILES
from creation_lib.core.path_detector import detect_game_path
from creation_lib.paths import get_resource_dir


def _ffmpeg():
    # Matches test_create_ogg.py's helper so both tests agree on what counts
    # as "ffmpeg is available" - a missing tool is an environment gap, not a
    # verdict on the FaceFX generator type this file is actually probing.
    tool = os.environ.get("FFMPEG_EXE", "").strip() or shutil.which("ffmpeg")
    if tool and os.path.isfile(tool):
        return tool
    pytest.skip("ffmpeg not found: set FFMPEG_EXE or put ffmpeg on PATH")


def _fnv_voices_archive():
    root = detect_game_path("fnv")
    if not root:
        pytest.skip("Fallout New Vegas is not installed")
    data = Path(root) / "Data"
    for name in sorted(os.listdir(data)):
        if name.lower().startswith("fallout - voices") and name.lower().endswith(".bsa"):
            return data / name
    pytest.skip("No FNV voices BSA found")


def _original_lip_bytes():
    from creation_lib.ba2 import native_runtime

    archive = _fnv_voices_archive()
    members = [m for m in native_runtime.list_archive(str(archive)) if m.lower().endswith(".lip")]
    if not members:
        pytest.skip("No .lip members in the FNV voices BSA")
    return bytes(native_runtime.extract_one(str(archive), members[0]))


def _write_silence_wav(path, seconds=1.0, rate=44100):
    with wave.open(str(path), "wb") as handle:
        handle.setnchannels(1)
        handle.setsampwidth(2)
        handle.setframerate(rate)
        handle.writeframes(struct.pack("<h", 0) * int(seconds * rate))


def test_a_real_fnv_lip_opens_with_version_one():
    assert _original_lip_bytes()[:4] == b"\x01\x00\x00\x00"


def test_skyrim_generator_output_matches_the_fnv_header(tmp_path):
    facefx = GAME_PROFILES["fnv"].facefx_game
    if not facefx:
        pytest.skip("FNV lip generation is disabled; reuse-of-original is the shipped path")

    ffmpeg = _ffmpeg()
    original = _original_lip_bytes()
    wav = tmp_path / "probe.wav"
    lip = tmp_path / "probe.lip"
    _write_silence_wav(wav)

    produced = create_lip(
        str(wav), str(lip), "This is a test of the emergency broadcast system.",
        ffmpeg_path=ffmpeg, game=facefx, resource_dir=get_resource_dir(),
    )
    assert produced is True, (
        f"FaceFXWrapper rejected game type {facefx!r}. Set facefx_game=None for "
        "fnv and fo3 and rely on reuse-of-original."
    )
    assert lip.is_file() and lip.stat().st_size > 0
    assert lip.read_bytes()[:4] == original[:4], (
        "Generated lip header differs from the game's. Set facefx_game=None for "
        "fnv and fo3 and rely on reuse-of-original."
    )
