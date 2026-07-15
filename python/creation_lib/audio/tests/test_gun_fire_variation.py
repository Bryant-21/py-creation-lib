import wave
from pathlib import Path

import numpy as np

from creation_lib.audio import (
    _add_early_reflections,
    _choose_shot_variant,
    _random_tone_color,
    db_to_linear,
    generate_gun_fire,
)


def _write_mono_wav(path: Path, samples: np.ndarray, sr: int = 8_000) -> None:
    pcm = (np.clip(samples, -1.0, 1.0) * 32767).astype(np.int16)
    with wave.open(str(path), "wb") as wav_f:
        wav_f.setnchannels(1)
        wav_f.setsampwidth(2)
        wav_f.setframerate(sr)
        wav_f.writeframes(pcm.tobytes())


def test_early_reflections_adds_delayed_tap(monkeypatch) -> None:
    audio = np.zeros(100, dtype=np.float32)
    audio[0] = 1.0
    monkeypatch.setattr("creation_lib.audio.random.randint", lambda low, _high: low)
    monkeypatch.setattr("creation_lib.audio.random.uniform", lambda _low, high: high)

    reflected = _add_early_reflections(audio, 1_000)

    assert len(reflected) == 135
    assert reflected[0] == 1.0
    assert np.isclose(reflected[6], db_to_linear(-12.0))


def test_choose_shot_variant_avoids_immediate_repeat(monkeypatch) -> None:
    monkeypatch.setattr("creation_lib.audio.random.randrange", lambda _stop: 0)

    assert _choose_shot_variant(3, 0) == 1
    assert _choose_shot_variant(3, 2) == 0


def test_tone_color_preserves_shot_peak(monkeypatch) -> None:
    audio = np.zeros(128, dtype=np.float32)
    audio[0] = 0.75
    audio[1:32] = np.linspace(0.5, 0.01, 31, dtype=np.float32)

    def fake_uniform(low: float, high: float) -> float:
        if low == -0.75 and high == 0.75:
            return 0.0
        return high

    monkeypatch.setattr("creation_lib.audio.HAS_NATIVE_FILTERS", True)
    monkeypatch.setattr("creation_lib.audio.random.random", lambda: 1.0)
    monkeypatch.setattr("creation_lib.audio.random.uniform", fake_uniform)
    monkeypatch.setattr("creation_lib.audio.lowpass_filter", lambda data, _cutoff, _sr: data * 0.25)

    colored = _random_tone_color(audio, 48_000)

    assert np.isclose(np.max(np.abs(colored)), np.max(np.abs(audio)))


def test_generate_gun_fire_optional_variation_writes_file(tmp_path) -> None:
    source = tmp_path / "weapon_single.wav"
    samples = np.zeros(256, dtype=np.float32)
    samples[0] = 0.8
    samples[1:96] = np.linspace(0.6, 0.01, 95, dtype=np.float32)
    _write_mono_wav(source, samples)

    result = generate_gun_fire(
        source_wav=str(source),
        output_dir=str(tmp_path),
        rpms=[600],
        shot_count=4,
        tail_threshold=-80.0,
        pitch_variation=0.0,
        gain_variation=0.0,
        jitter_ms=0,
        highpass_enabled=False,
        shot_variant_count=3,
        early_reflections_enabled=True,
        tone_color_enabled=True,
    )

    assert result["errors"] == []
    assert len(result["files"]) == 1
    assert Path(result["files"][0]).exists()
