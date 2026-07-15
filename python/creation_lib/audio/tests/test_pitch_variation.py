import numpy as np

from creation_lib.audio import _pitch_shift_resample_preserve_length, random_pitch


def _transient_input() -> np.ndarray:
    audio = np.zeros(4096, dtype=np.float32)
    audio[0] = 1.0
    audio[1:200] = np.linspace(1.0, 0.0, 199, dtype=np.float32)
    return audio


def test_transient_safe_pitch_shift_avoids_peak_overshoot() -> None:
    audio = _transient_input()

    for semitones in (-0.5, 0.5):
        shifted = _pitch_shift_resample_preserve_length(audio, semitones)
        peak = float(np.max(np.abs(shifted)))
        peak_index = int(np.argmax(np.abs(shifted)))
        assert peak <= 1.01
        assert peak_index == 0


def test_random_pitch_transient_safe_uses_resample_path(monkeypatch) -> None:
    audio = _transient_input()
    monkeypatch.setattr("creation_lib.audio.random.uniform", lambda _a, _b: 0.5)

    shifted = random_pitch(audio, 48_000, 0.5, transient_safe=True)

    assert shifted.dtype == np.float32
    assert float(np.max(np.abs(shifted))) <= 1.01
    assert int(np.argmax(np.abs(shifted))) == 0
