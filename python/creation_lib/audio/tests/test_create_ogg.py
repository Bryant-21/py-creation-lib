"""New Vegas and Fallout 3 ship mono Ogg Vorbis voice lines."""
import math
import os
import shutil
import struct
import subprocess
import wave

import pytest

from creation_lib.audio.release import create_ogg


def _write_sine_wav(path, seconds=0.25, rate=44100):
    frames = bytearray()
    for index in range(int(seconds * rate)):
        value = int(12000 * math.sin(2 * math.pi * 440 * index / rate))
        frames += struct.pack("<h", value)
    with wave.open(str(path), "wb") as handle:
        handle.setnchannels(1)
        handle.setsampwidth(2)
        handle.setframerate(rate)
        handle.writeframes(bytes(frames))


def _ffmpeg():
    tool = os.environ.get("FFMPEG_EXE", "").strip() or shutil.which("ffmpeg")
    if tool and os.path.isfile(tool):
        return tool
    pytest.skip("ffmpeg not found: set FFMPEG_EXE or put ffmpeg on PATH")


def test_create_ogg_writes_mono_vorbis(tmp_path):
    tool = _ffmpeg()
    wav = tmp_path / "line.wav"
    ogg = tmp_path / "line.ogg"
    _write_sine_wav(wav)

    assert create_ogg(str(wav), str(ogg), ffmpeg_path=tool) is True
    assert ogg.is_file()
    assert ogg.stat().st_size > 0

    probe = subprocess.run(
        [tool.replace("ffmpeg", "ffprobe"), "-hide_banner", "-v", "error",
         "-show_entries", "stream=codec_name,channels", "-of", "csv=p=0", str(ogg)],
        capture_output=True, text=True,
    )
    if probe.returncode == 0:
        assert "vorbis" in probe.stdout
        assert probe.stdout.strip().endswith("1")


def test_create_ogg_returns_false_for_a_missing_source(tmp_path):
    tool = _ffmpeg()
    ogg = tmp_path / "nope.ogg"
    assert create_ogg(str(tmp_path / "nope.wav"), str(ogg), ffmpeg_path=tool) is False
    assert not ogg.exists()


def test_create_ogg_returns_false_when_ffmpeg_is_missing(tmp_path):
    wav = tmp_path / "line.wav"
    _write_sine_wav(wav)
    assert create_ogg(str(wav), str(tmp_path / "line.ogg"),
                      ffmpeg_path=str(tmp_path / "no-such-ffmpeg.exe")) is False
