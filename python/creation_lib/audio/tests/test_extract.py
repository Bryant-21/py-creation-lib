from __future__ import annotations

import os
from pathlib import Path

import numpy as np
import pytest
import soundfile as sf

from creation_lib.audio import extract
from creation_lib.audio.release import create_fuz, create_xwm
from creation_lib.paths import get_resource_dir

pytestmark = pytest.mark.skipif(os.name != "nt", reason="bundled decoders are Windows executables")


def _tone_wav(path: Path) -> Path:
    sr = 44_100
    t = np.arange(sr) / sr
    sf.write(path, (0.3 * np.sin(2 * np.pi * 440 * t)).astype(np.float32), sr, subtype="PCM_16")
    return path


def _tone_fuz(tmp_path: Path) -> tuple[Path, Path, Path]:
    wav = _tone_wav(tmp_path / "tone.wav")
    xwm = tmp_path / "tone.xwm"
    assert create_xwm(str(wav), str(xwm), resource_dir=get_resource_dir())
    lip = tmp_path / "tone.lip"
    lip.write_bytes(b"\x01\x02\x03\x04" * 16)
    fuz = tmp_path / "tone.fuz"
    assert create_fuz(str(fuz), str(xwm), str(lip), resource_dir=get_resource_dir())
    return fuz, xwm, lip


def test_decode_xwm_round_trips_a_tone(tmp_path):
    wav = _tone_wav(tmp_path / "tone.wav")
    xwm = tmp_path / "tone.xwm"
    assert create_xwm(str(wav), str(xwm), resource_dir=get_resource_dir())

    decoded = extract.decode_xwm(xwm, tmp_path / "out" / "tone.wav")

    assert decoded == tmp_path / "out" / "tone.wav"
    data, rate = sf.read(decoded)
    assert rate == 44_100
    assert len(data) > 40_000

    with pytest.raises(FileNotFoundError):
        extract.decode_xwm(tmp_path / "missing.xwm", tmp_path / "missing.wav")


def test_decode_fuz_keeps_or_drops_lip(tmp_path):
    fuz, xwm, lip = _tone_fuz(tmp_path)

    kept = extract.decode_fuz(fuz, tmp_path / "kept", keep_lip=True)
    assert kept == tmp_path / "kept" / "tone.xwm"
    assert kept.read_bytes() == xwm.read_bytes()
    assert (tmp_path / "kept" / "tone.lip").read_bytes() == lip.read_bytes()

    extract.decode_fuz(fuz, tmp_path / "dropped")
    assert sorted(p.name for p in (tmp_path / "dropped").iterdir()) == ["tone.xwm"]


def test_decode_to_wav_handles_fuz_and_passthrough_wav(tmp_path):
    fuz, _xwm, _lip = _tone_fuz(tmp_path)
    out = tmp_path / "out"

    produced = extract.decode_to_wav(fuz, out)

    assert produced == out / "tone.wav"
    assert sorted(p.name for p in out.iterdir()) == ["tone.wav"]

    wav = _tone_wav(tmp_path / "already.wav")
    assert extract.decode_to_wav(wav, tmp_path / "unused") == wav


def test_decode_to_wav_routes_ogg_through_ffmpeg(tmp_path, monkeypatch):
    source = tmp_path / "voice.ogg"
    source.write_bytes(b"placeholder")
    seen = {}

    def fake_run(cmd, what):
        seen["cmd"] = [str(part) for part in cmd]
        (tmp_path / "out" / "voice.wav").write_bytes(b"wav")

    monkeypatch.setattr(extract, "_run", fake_run)

    result = extract.decode_to_wav(source, tmp_path / "out", ffmpeg_path="ffmpeg.exe")

    assert result == tmp_path / "out" / "voice.wav"
    assert seen["cmd"][0] == "ffmpeg.exe"


def test_decode_to_wav_rebuilds_wem_as_ogg_before_ffmpeg(tmp_path, monkeypatch):
    source = tmp_path / "voice.wem"
    source.write_bytes(b"placeholder")
    seen = {}

    def fake_wem_to_ogg(source_wem, output_ogg, codebook_executable):
        seen["convert"] = (source_wem, output_ogg, codebook_executable)
        Path(output_ogg).write_bytes(b"OggS")

    def fake_run(cmd, what):
        seen["cmd"] = [str(part) for part in cmd]
        seen["ogg_existed"] = Path(seen["cmd"][3]).is_file()
        (tmp_path / "out" / "voice.wav").write_bytes(b"wav")

    monkeypatch.setattr(extract.audio_native_runtime, "wem_to_ogg", fake_wem_to_ogg)
    monkeypatch.setattr(extract, "_run", fake_run)

    result = extract.decode_to_wav(
        source, tmp_path / "out", ffmpeg_path="ffmpeg.exe", wwise_codebooks=tmp_path / "Starfield.exe"
    )

    assert result == tmp_path / "out" / "voice.wav"
    source_wem, output_ogg, codebook_executable = seen["convert"]
    assert source_wem == str(source)
    assert output_ogg.endswith("voice.ogg")
    assert codebook_executable == str(tmp_path / "Starfield.exe")
    assert seen["cmd"][0] == "ffmpeg.exe"
    assert seen["cmd"][3] == output_ogg
    assert seen["ogg_existed"]
    assert not Path(output_ogg).exists(), "the intermediate ogg is cleaned up"


def test_decode_to_wav_rejects_invalid_inputs(tmp_path):
    unknown_suffix = tmp_path / "voice.mp3"
    unknown_suffix.write_bytes(b"x")
    with pytest.raises(ValueError):
        extract.decode_to_wav(unknown_suffix, tmp_path / "out")

    wem = tmp_path / "voice.wem"
    wem.write_bytes(b"placeholder")
    with pytest.raises(ValueError, match="wwise_codebooks"):
        extract.decode_to_wav(wem, tmp_path / "out", ffmpeg_path="ffmpeg.exe")


