"""Thin Python boundary for native audio_native entrypoints."""
from __future__ import annotations

import numpy as np

from creation_lib._native import audio_native as _native


def pitch_shift(samples: np.ndarray, sr: int, semitones: float) -> np.ndarray:
    """Pitch-shift `samples` by `semitones`, preserving duration."""
    if samples.dtype != np.float32:
        samples = samples.astype(np.float32)
    return _native.pitch_shift(samples, int(sr), float(semitones))
