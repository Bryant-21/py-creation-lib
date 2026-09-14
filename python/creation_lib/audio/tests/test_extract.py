from __future__ import annotations

import os
import shutil
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


def test_decode_fuz_returns_xwm_and_keeps_lip(tmp_path):
    fuz, xwm, lip = _tone_fuz(tmp_path)
    out = tmp_path / "out"

    produced = extract.decode_fuz(fuz, out, keep_lip=True)

    assert produced == out / "tone.xwm"
    assert produced.read_bytes() == xwm.read_bytes()
    assert (out / "tone.lip").read_bytes() == lip.read_bytes()


def test_decode_fuz_drops_lip_by_default(tmp_path):
    fuz, _xwm, _lip = _tone_fuz(tmp_path)
    out = tmp_path / "out"

    extract.decode_fuz(fuz, out)

    assert sorted(p.name for p in out.iterdir()) == ["tone.xwm"]


def test_decode_to_wav_from_fuz_leaves_only_the_wav(tmp_path):
    fuz, _xwm, _lip = _tone_fuz(tmp_path)
    out = tmp_path / "out"

    produced = extract.decode_to_wav(fuz, out)

    assert produced == out / "tone.wav"
    assert sorted(p.name for p in out.iterdir()) == ["tone.wav"]


def test_decode_to_wav_passes_through_wav(tmp_path):
    wav = _tone_wav(tmp_path / "tone.wav")
    assert extract.decode_to_wav(wav, tmp_path / "out") == wav


def test_decode_xwm_missing_input_raises(tmp_path):
    with pytest.raises(FileNotFoundError):
        extract.decode_xwm(tmp_path / "missing.xwm", tmp_path / "missing.wav")


def test_decode_to_wav_rejects_unknown_suffix(tmp_path):
    source = tmp_path / "voice.mp3"
    source.write_bytes(b"x")
    with pytest.raises(ValueError):
        extract.decode_to_wav(source, tmp_path / "out")


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


def test_decode_to_wav_needs_a_wwise_codebook_source_for_wem(tmp_path):
    source = tmp_path / "voice.wem"
    source.write_bytes(b"placeholder")
    with pytest.raises(ValueError, match="wwise_codebooks"):
        extract.decode_to_wav(source, tmp_path / "out", ffmpeg_path="ffmpeg.exe")


def _starfield_dir() -> Path | None:
    root = os.environ.get("STARFIELD_DIR")
    return Path(root) if root and (Path(root) / "Starfield.exe").is_file() else None


@pytest.mark.skipif(
    _starfield_dir() is None or shutil.which("ffmpeg") is None,
    reason="needs STARFIELD_DIR and ffmpeg on PATH",
)
def test_decode_to_wav_decodes_a_starfield_voice_line(tmp_path):
    from creation_lib.ba2 import native_runtime as ba2

    root = _starfield_dir()
    wem = tmp_path / "00c0c1b2.wem"
    wem.write_bytes(
        bytes(
            ba2.extract_one(
                str(root / "Data" / "Starfield - Voices02.ba2"),
                "sound/voice/starfield.esm/robotmodelavasco/00c0c1b2.wem",
            )
        )
    )

    decoded = extract.decode_to_wav(wem, tmp_path / "out", wwise_codebooks=root / "Starfield.exe")

    data, rate = sf.read(decoded)
    assert rate == 44_100
    assert len(data) == 630_781
    assert np.sqrt(np.mean(data**2)) > 0.01
