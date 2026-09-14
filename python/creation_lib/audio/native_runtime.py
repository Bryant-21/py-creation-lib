"""Thin Python boundary for native audio_native entrypoints."""
from __future__ import annotations

import numpy as np

from creation_lib._native import audio_native as _native


def encode_at9(source_wav: str, output_at9: str) -> None:
    _native.encode_at9(source_wav, output_at9)


def wem_to_ogg(source_wem: str, output_ogg: str, codebook_executable: str) -> None:
    """Rebuild a Wwise Vorbis .wem as Ogg Vorbis.

    `codebook_executable` is the game executable the Wwise runtime is linked
    into (Starfield.exe); its codebook table is what the .wem references.
    """
    _native.wem_to_ogg(source_wem, output_ogg, codebook_executable)


def pitch_shift(samples: np.ndarray, sr: int, semitones: float) -> np.ndarray:
    """Pitch-shift `samples` by `semitones`, preserving duration."""
    if samples.dtype != np.float32:
        samples = samples.astype(np.float32)
    return _native.pitch_shift(samples, int(sr), float(semitones))
