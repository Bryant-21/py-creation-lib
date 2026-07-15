"""Audio processing utilities -- ffmpeg wrapper, weapon sound generation.

All functions accept explicit parameters.
"""

from __future__ import annotations

import logging
import os
import random
import re
import struct
import subprocess
import wave
from typing import Callable

import numpy as np

_log = logging.getLogger("creation_lib.audio")


def _load_wav_mono_f32(path: str) -> tuple[np.ndarray, int]:
    """Load a PCM WAV file as mono float32 in [-1, 1]. PCM only."""
    with wave.open(path, "rb") as w:
        sr = w.getframerate()
        n_frames = w.getnframes()
        n_channels = w.getnchannels()
        sampwidth = w.getsampwidth()
        raw = w.readframes(n_frames)

    if sampwidth == 1:
        # 8-bit WAV is unsigned
        data = np.frombuffer(raw, dtype=np.uint8).astype(np.float32)
        data = (data - 128.0) / 128.0
    elif sampwidth == 2:
        data = np.frombuffer(raw, dtype=np.int16).astype(np.float32) / 32768.0
    elif sampwidth == 3:
        # 24-bit little-endian PCM: unpack to int32
        b = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 3)
        ints = (b[:, 0].astype(np.int32)
                | (b[:, 1].astype(np.int32) << 8)
                | (b[:, 2].astype(np.int32) << 16))
        ints = np.where(ints & 0x800000, ints - 0x1000000, ints)
        data = ints.astype(np.float32) / 8388608.0
    elif sampwidth == 4:
        data = np.frombuffer(raw, dtype=np.int32).astype(np.float32) / 2147483648.0
    else:
        raise ValueError(f"unsupported WAV sample width: {sampwidth} bytes")

    if n_channels > 1:
        data = data.reshape(-1, n_channels).mean(axis=1)
    return data.astype(np.float32, copy=False), sr


try:
    from creation_lib.scientific.native_runtime import butter_filter as _butter_filter
    HAS_NATIVE_FILTERS = True
except ImportError:
    HAS_NATIVE_FILTERS = False


# ---------------------------------------------------------------------------
# Audio loading via ffmpeg
# ---------------------------------------------------------------------------

def load_audio(file: str, sampling_rate: int, ffmpeg_path: str,
               channels: int = 1) -> np.ndarray:
    """Load audio file via ffmpeg, return as float32 numpy array.

    Args:
        file: Path to audio file.
        sampling_rate: Target sample rate.
        ffmpeg_path: Path to ffmpeg executable.
        channels: Number of output channels.

    Returns:
        float32 numpy array of audio samples.
    """
    cmd = [
        ffmpeg_path, "-y", "-i", file, "-f", "f32le", "-acodec", "pcm_f32le",
        "-af", "aresample=resampler=soxr", "-ac", str(channels),
        "-ar", str(sampling_rate), "pipe:1",
    ]

    process = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    out, err = process.communicate()
    if process.returncode != 0:
        raise RuntimeError(f"FFmpeg error: {err.decode('utf-8', errors='replace')}")

    return np.frombuffer(out, np.float32).flatten()


def combine_wav_files(input_files: list[str], output_path: str,
                      ffmpeg_path: str, target_rate: int = 24500,
                      silence_duration: float = 0.5) -> str:
    """Concatenate WAV files with silence gaps between them.

    Args:
        input_files: List of input WAV paths.
        output_path: Path to save the combined WAV.
        ffmpeg_path: Path to ffmpeg executable.
        target_rate: Target sample rate.
        silence_duration: Silence duration between files in seconds.

    Returns:
        Path to the output file.
    """
    import soundfile as sf

    combined_data = None
    for input_file in input_files:
        audio_data = load_audio(input_file, target_rate, ffmpeg_path)
        if combined_data is None:
            combined_data = audio_data
        else:
            silence_samples = int(silence_duration * target_rate)
            silence = np.zeros(silence_samples, dtype=np.float32)
            combined_data = np.concatenate((combined_data, silence, audio_data))

    if combined_data is not None:
        sf.write(output_path, combined_data, target_rate)
    return output_path


# ---------------------------------------------------------------------------
# DSP helpers
# ---------------------------------------------------------------------------

def db_to_linear(db: float) -> float:
    """Convert decibels to linear amplitude."""
    return 10 ** (db / 20)


def highpass_filter(data: np.ndarray, cutoff: float, fs: float) -> np.ndarray:
    """Apply a highpass Butterworth filter."""
    if not HAS_NATIVE_FILTERS:
        raise ImportError("native scientific filters are required for filter operations")
    return _butter_filter(data, cutoff, fs, "highpass")


def lowpass_filter(data: np.ndarray, cutoff: float, fs: float) -> np.ndarray:
    """Apply a lowpass Butterworth filter."""
    if not HAS_NATIVE_FILTERS:
        raise ImportError("native scientific filters are required for filter operations")
    return _butter_filter(data, cutoff, fs, "lowpass")


def tilt_eq(audio: np.ndarray, amount: float) -> np.ndarray:
    """Apply tilt EQ. amount > 0 = brighter, < 0 = bassier."""
    t = np.linspace(-1, 1, len(audio))
    tilt = 1 + (t * amount * 0.1)
    return audio * tilt


def trim_tail(audio: np.ndarray, sr: int, threshold_db: float) -> np.ndarray:
    """Trim audio tail below threshold."""
    envelope = np.abs(audio)
    envelope_db = 20 * np.log10(envelope + 1e-8)
    cutoff_index = len(audio)
    for i in range(len(envelope_db) - 1, 0, -1):
        if envelope_db[i] > threshold_db:
            cutoff_index = i
            break
    return audio[:cutoff_index]


def _pitch_shift_resample_preserve_length(audio: np.ndarray, semitones: float) -> np.ndarray:
    """Pitch-shift short transients by resampling, then pad/truncate to the original length."""
    target_len = len(audio)
    if semitones == 0 or target_len == 0:
        return audio.copy()

    ratio = 2.0 ** (semitones / 12.0)
    shifted_len = max(1, int(round(target_len / ratio)))
    source_positions = np.linspace(0, target_len - 1, shifted_len, dtype=np.float32)
    shifted = np.interp(source_positions, np.arange(target_len, dtype=np.float32), audio).astype(np.float32)
    output = np.zeros(target_len, dtype=np.float32)
    copy_len = min(target_len, len(shifted))
    output[:copy_len] = shifted[:copy_len]
    return output


def random_pitch(audio: np.ndarray, sr: int, pitch_variation: float,
                 transient_safe: bool = False) -> np.ndarray:
    """Apply random pitch shift in semitones."""
    if pitch_variation <= 0:
        return audio
    semitone = random.uniform(-pitch_variation, pitch_variation)
    if transient_safe:
        return _pitch_shift_resample_preserve_length(audio, semitone)
    from creation_lib.audio.native_runtime import pitch_shift
    return pitch_shift(audio, sr, semitone)


def random_gain(audio: np.ndarray, gain_variation_db: float) -> np.ndarray:
    """Apply random gain variation in dB."""
    if gain_variation_db <= 0:
        return audio
    gain_db = random.uniform(-gain_variation_db, gain_variation_db)
    return audio * db_to_linear(gain_db)


def _match_peak_level(audio: np.ndarray, reference: np.ndarray) -> np.ndarray:
    audio_peak = float(np.max(np.abs(audio))) if len(audio) else 0.0
    reference_peak = float(np.max(np.abs(reference))) if len(reference) else 0.0
    if audio_peak <= 1e-8 or reference_peak <= 1e-8:
        return audio
    return audio * (reference_peak / audio_peak)


def _add_early_reflections(audio: np.ndarray, sr: int) -> np.ndarray:
    tap_count = random.randint(1, 3)
    min_delay = max(1, int(0.006 * sr))
    max_delay = max(min_delay, int(0.035 * sr))
    output = np.zeros(len(audio) + max_delay, dtype=np.float32)
    output[:len(audio)] = audio

    for _ in range(tap_count):
        delay = random.randint(min_delay, max_delay)
        gain = db_to_linear(random.uniform(-24.0, -12.0))
        output[delay:delay + len(audio)] += audio * gain

    return output


def _random_tone_color(audio: np.ndarray, sr: int) -> np.ndarray:
    colored = tilt_eq(audio, random.uniform(-0.75, 0.75))
    if HAS_NATIVE_FILTERS and random.random() > 0.5:
        max_cutoff = min(11_000.0, sr * 0.475)
        min_cutoff = min(5_000.0, max_cutoff)
        if min_cutoff < max_cutoff:
            colored = lowpass_filter(colored, random.uniform(min_cutoff, max_cutoff), sr)
    colored = _match_peak_level(colored, audio)
    return colored.astype(np.float32, copy=False)


def _make_gun_shot_variant(audio: np.ndarray, sr: int, pitch_variation: float,
                           gain_variation: float, highpass_enabled: bool,
                           tone_color_enabled: bool,
                           early_reflections_enabled: bool) -> np.ndarray:
    shot = audio.copy()
    shot = random_pitch(shot, sr, pitch_variation, transient_safe=True)
    shot = random_gain(shot, gain_variation)

    if highpass_enabled and HAS_NATIVE_FILTERS and random.random() > 0.5:
        shot = highpass_filter(shot, random.uniform(80, 200), sr)
    if tone_color_enabled:
        shot = _random_tone_color(shot, sr)
    if early_reflections_enabled:
        shot = _add_early_reflections(shot, sr)

    return shot.astype(np.float32, copy=False)


def _choose_shot_variant(variant_count: int, previous_idx: int | None) -> int:
    if variant_count <= 1:
        return 0
    idx = random.randrange(variant_count - 1)
    if previous_idx is not None and idx >= previous_idx:
        idx += 1
    return idx


# ---------------------------------------------------------------------------
# WAV marker/loop chunk builders
# ---------------------------------------------------------------------------

def build_cue_chunk(marker_defs: list[tuple[int, str]]) -> bytes:
    """Build a WAV cue chunk from marker definitions (position, label)."""
    cue_count = len(marker_defs)
    chunk = struct.pack("<4sI", b"cue ", 4 + cue_count * 24)
    chunk += struct.pack("<I", cue_count)
    for i, (pos, _) in enumerate(marker_defs):
        chunk += struct.pack("<IIIIII", i + 1, pos, 0x64617461, 0, 0, pos)
    return chunk


def build_smpl_chunk(loop_start: int, loop_end: int, sample_rate: int = 48000) -> bytes:
    """Build a WAV sampler chunk with loop definition."""
    chunk = struct.pack("<4sI", b"smpl", 60)
    chunk += struct.pack("<I", 0)  # manufacturer
    chunk += struct.pack("<I", 0)  # product
    chunk += struct.pack("<I", int(1e9 / sample_rate))  # sample period
    chunk += struct.pack("<I", 60)  # MIDI unity note
    chunk += struct.pack("<I", 0)  # MIDI pitch fraction
    chunk += struct.pack("<I", 0)  # SMPTE format
    chunk += struct.pack("<I", 0)  # SMPTE offset
    chunk += struct.pack("<I", 1)  # number of loops
    chunk += struct.pack("<I", 0)  # sampler data
    # Loop definition
    chunk += struct.pack("<I", 0)  # cue point ID
    chunk += struct.pack("<I", 0)  # type 0 = forward loop
    chunk += struct.pack("<I", loop_start)
    chunk += struct.pack("<I", loop_end)
    chunk += struct.pack("<I", 0)  # fraction
    chunk += struct.pack("<I", 0)  # play count (0 = infinite)
    return chunk


def build_label_chunk(marker_defs: list[tuple[int, str]]) -> bytes:
    """Build a WAV label (adtl LIST) chunk."""
    entries = []
    for i, (_, label_text) in enumerate(marker_defs):
        text_bytes = label_text.encode("ascii") + b"\x00"
        size = 4 + len(text_bytes)
        entry = struct.pack("<4sI", b"labl", size) + struct.pack("<I", i + 1) + text_bytes
        entries.append(entry)
    adtl_data = b"".join(entries)
    chunk = struct.pack("<4sI", b"LIST", len(adtl_data) + 4) + b"adtl" + adtl_data
    return chunk


def has_loop_or_cue_markers(wav_path: str) -> bool:
    """Return True if WAV has smpl (loop) or cue (marker) chunks.

    Such files must NOT be XWM-encoded — xWMAEncode discards these chunks,
    breaking looping audio and marker-based playback.

    Scans RIFF chunk headers without decoding audio data.
    """
    try:
        with open(wav_path, "rb") as f:
            header = f.read(12)
            if len(header) < 12 or header[:4] != b"RIFF" or header[8:12] != b"WAVE":
                return False
            while True:
                chunk_header = f.read(8)
                if len(chunk_header) < 8:
                    break
                chunk_id = chunk_header[:4]
                chunk_size = struct.unpack("<I", chunk_header[4:8])[0]
                if chunk_id in (b"smpl", b"cue "):
                    return True
                # Skip chunk data (pad to even boundary per RIFF spec)
                f.seek(chunk_size + (chunk_size & 1), 1)
    except (OSError, struct.error):
        return False
    return False


def _write_wav_with_markers(output_file: str, pcm: np.ndarray, sr: int,
                            marker_defs: list[tuple[int, str]],
                            loop_start: int | None = None,
                            loop_end: int | None = None) -> None:
    """Write a WAV file with optional cue markers and loop points."""
    with wave.open(output_file, "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(sr)
        wav.writeframes(pcm.tobytes())

    if marker_defs and len(marker_defs) >= 2 and loop_start is not None and loop_end is not None:
        with open(output_file, "r+b") as f:
            f.seek(0, 2)
            f.write(build_cue_chunk(marker_defs))
            f.write(build_smpl_chunk(loop_start, loop_end, sample_rate=sr))
            f.write(build_label_chunk(marker_defs))
            file_size = f.tell()
            f.seek(4)
            f.write(struct.pack("<I", file_size - 8))


# ---------------------------------------------------------------------------
# Gun fire sound generation
# ---------------------------------------------------------------------------

def generate_gun_fire(source_wav: str, output_dir: str,
                      rpms: list[int],
                      shot_count: int = 12,
                      tail_threshold: float = -35.0,
                      pitch_variation: float = 0.5,
                      gain_variation: float = 2.0,
                      jitter_ms: int = 8,
                      highpass_enabled: bool = True,
                      tilt_amount: float = 0.0,
                      base_reinforcement: bool = False,
                      shot_variant_count: int = 0,
                      early_reflections_enabled: bool = False,
                      tone_color_enabled: bool = False,
                      progress_callback: Callable[[int, int, str], None] | None = None,
                      cancel_check: Callable[[], bool] | None = None) -> dict:
    """Generate gun firing sound variations at different RPMs.

    Args:
        source_wav: Path to single-shot WAV file.
        output_dir: Directory to save generated files.
        rpms: List of RPM values to generate.
        shot_count: Number of shots in the sequence.
        tail_threshold: Threshold in dB for tail trimming.
        pitch_variation: Pitch variation in semitones.
        gain_variation: Gain variation in dB.
        jitter_ms: Timing jitter in milliseconds.
        highpass_enabled: Apply random highpass filter for realism.
        tilt_amount: Tilt EQ amount.
        base_reinforcement: Add low-passed version for punch.
        shot_variant_count: Number of prebuilt shot variants to rotate through. 0 disables the pool.
        early_reflections_enabled: Add short random delay taps per shot for space.
        tone_color_enabled: Apply subtle per-shot tonal color changes.
        progress_callback: Called with (current, total, message).
        cancel_check: Returns True to cancel.

    Returns:
        dict with keys: files (list of output paths), errors (list of error messages).
    """
    result: dict = {"files": [], "errors": []}
    total = len(rpms)

    audio, sr = _load_wav_mono_f32(source_wav)
    base_shot = trim_tail(audio, sr, tail_threshold)

    if tilt_amount != 0:
        base_shot = tilt_eq(base_shot, tilt_amount)

    if base_reinforcement and HAS_NATIVE_FILTERS:
        low = lowpass_filter(base_shot, 150, sr)
        base_shot = base_shot + low

    for rpm_idx, rpm in enumerate(rpms):
        if cancel_check and cancel_check():
            break

        try:
            shot_spacing_sec = 60.0 / rpm
            shot_spacing_samples = int(shot_spacing_sec * sr)

            final_length = shot_spacing_samples * shot_count + len(audio)
            output = np.zeros(final_length)
            shot_markers = []
            variant_pool = []
            if shot_variant_count > 1:
                variant_pool = [
                    _make_gun_shot_variant(
                        base_shot,
                        sr,
                        pitch_variation,
                        gain_variation,
                        highpass_enabled,
                        tone_color_enabled,
                        early_reflections_enabled,
                    )
                    for _ in range(shot_variant_count)
                ]

            cursor = 0
            previous_variant_idx = None
            for i in range(shot_count):
                if variant_pool:
                    variant_idx = _choose_shot_variant(len(variant_pool), previous_variant_idx)
                    previous_variant_idx = variant_idx
                    shot = variant_pool[variant_idx].copy()
                else:
                    shot = _make_gun_shot_variant(
                        base_shot,
                        sr,
                        pitch_variation,
                        gain_variation,
                        highpass_enabled,
                        tone_color_enabled,
                        early_reflections_enabled,
                    )

                jitter = int(random.uniform(-jitter_ms, jitter_ms) * sr / 1000)
                placement = max(cursor + jitter, 0)

                end = placement + len(shot)
                if end > len(output):
                    padding = np.zeros(end - len(output))
                    output = np.concatenate([output, padding])

                output[placement:end] += shot
                shot_markers.append(placement)
                cursor += shot_spacing_samples

            # Normalize
            max_val = np.max(np.abs(output))
            if max_val > 0:
                output /= max_val + 1e-8

            pcm = (output * 32767).astype(np.int16)

            # Output filename
            base_name = os.path.splitext(os.path.basename(source_wav))[0]
            if "single" in base_name.lower():
                base_name = re.sub(r'single', 'auto', base_name, flags=re.IGNORECASE)
            else:
                base_name = f"{base_name}_auto"

            output_file = os.path.join(output_dir, f"{base_name}_{rpm}.wav")

            # Build markers
            if len(shot_markers) >= 2:
                loop_start = shot_markers[1]
                loop_end = shot_markers[-1]

                pos_to_labels: dict[int, list[str]] = {}
                for i, pos in enumerate(shot_markers):
                    pos_to_labels.setdefault(pos, []).append(f"SHOT_{i + 1}")
                pos_to_labels.setdefault(loop_start, []).append("LOOP_START")
                pos_to_labels.setdefault(loop_end, []).append("LOOP_END")

                marker_defs = []
                for pos in sorted(pos_to_labels.keys()):
                    labels = sorted(pos_to_labels[pos], key=lambda l: (0 if l.startswith("SHOT") else 1, l))
                    marker_defs.append((pos, " + ".join(labels)))

                _write_wav_with_markers(output_file, pcm, sr, marker_defs, loop_start, loop_end)
            else:
                with wave.open(output_file, "wb") as wav_f:
                    wav_f.setnchannels(1)
                    wav_f.setsampwidth(2)
                    wav_f.setframerate(sr)
                    wav_f.writeframes(pcm.tobytes())

            result["files"].append(output_file)
            _log.info("Created: %s", output_file)

        except Exception as e:
            result["errors"].append(f"RPM {rpm}: {e}")
            _log.error("Failed to generate RPM %d: %s", rpm, e)

        if progress_callback:
            progress_callback(rpm_idx + 1, total, f"Generated RPM {rpm}")

    return result


# ---------------------------------------------------------------------------
# Laser beam sound generation
# ---------------------------------------------------------------------------

def find_zero_crossing(audio: np.ndarray, start_idx: int, direction: int = 1,
                       max_search: int = 1000) -> int:
    """Find the nearest zero crossing from start_idx."""
    for i in range(max_search):
        idx = start_idx + (i * direction)
        if idx <= 0 or idx >= len(audio) - 1:
            return start_idx
        if audio[idx - 1] <= 0 < audio[idx] or audio[idx - 1] >= 0 > audio[idx]:
            return idx
    return start_idx


def generate_laser_beam(source_wav: str, output_dir: str,
                        loop_duration: float = 2.0,
                        pitch_variation: float = 0.3,
                        gain_variation: float = 0.5,
                        tail_threshold: float = -40.0,
                        highpass_enabled: bool = False,
                        highpass_cutoff: float = 200.0,
                        tilt_amount: float = 0.0,
                        progress_callback: Callable[[int, int, str], None] | None = None,
                        cancel_check: Callable[[], bool] | None = None) -> dict:
    """Generate laser weapon sound effect with loop points.

    Args:
        source_wav: Path to single-shot WAV file.
        output_dir: Directory to save generated file.
        loop_duration: Duration of the loop section in seconds.
        pitch_variation: Pitch variation in semitones.
        gain_variation: Gain variation in dB.
        tail_threshold: Threshold in dB for tail trimming.
        highpass_enabled: Apply highpass filter.
        highpass_cutoff: Highpass cutoff frequency.
        tilt_amount: Tilt EQ amount.
        progress_callback: Called with (current, total, message).
        cancel_check: Returns True to cancel.

    Returns:
        dict with keys: files (list of output paths), errors (list of error messages).
    """
    result: dict = {"files": [], "errors": []}

    try:
        audio, sr = _load_wav_mono_f32(source_wav)

        if progress_callback:
            progress_callback(1, 5, "Loaded audio")

        # Attack: first ~100ms
        attack_samples = int(0.1 * sr)
        attack = audio[:attack_samples].copy()

        # Tail: last ~400ms
        tail_samples = int(0.4 * sr)
        tail_start = max(len(audio) - tail_samples, attack_samples)
        tail = audio[tail_start:].copy()
        tail_fade_in = min(int(0.05 * sr), len(tail) // 4)
        if tail_fade_in > 0:
            tail[:tail_fade_in] *= np.sin(np.linspace(0, np.pi / 2, tail_fade_in)) ** 2

        # Body: middle portion
        body = audio[attack_samples:tail_start].copy()

        if progress_callback:
            progress_callback(2, 5, "Building loop")

        # Build layered loop
        loop_samples = int(loop_duration * sr)
        crossfade_len = int(0.05 * sr)
        segment_len = min(int(0.4 * sr), max(int(0.2 * sr), len(body) // 2))

        # Find most stable segment
        best_start = 0
        best_variance = float('inf')
        for i in range(0, max(1, len(body) - segment_len), segment_len // 8):
            seg = body[i:i + segment_len]
            if len(seg) < segment_len:
                continue
            variance = np.std(np.abs(seg))
            if variance < best_variance and np.mean(np.abs(seg)) > 0.01:
                best_variance = variance
                best_start = i

        base_segment = body[best_start:best_start + segment_len].copy()
        if len(base_segment) < segment_len:
            base_segment = body[:segment_len].copy() if len(body) >= segment_len else body.copy()

        if progress_callback:
            progress_callback(3, 5, "Layering variations")

        # Layer multiple copies
        loop_section = np.zeros(loop_samples)
        layer_count = 3

        for layer in range(layer_count):
            cursor = int(layer * segment_len / layer_count)

            while cursor < loop_samples:
                if cancel_check and cancel_check():
                    return result

                seg = base_segment.copy()
                if pitch_variation > 0:
                    seg = random_pitch(seg, sr, pitch_variation * 0.3)
                if gain_variation > 0:
                    seg = random_gain(seg, gain_variation * 0.3)
                if highpass_enabled and HAS_NATIVE_FILTERS and random.random() > 0.7:
                    seg = highpass_filter(seg, random.uniform(100, 300), sr)
                if tilt_amount != 0:
                    seg = tilt_eq(seg, tilt_amount * random.uniform(0.5, 1.5))

                fade_len = min(crossfade_len, len(seg) // 4)
                if fade_len > 0:
                    seg[:fade_len] *= np.sin(np.linspace(0, np.pi / 2, fade_len)) ** 2
                    seg[-fade_len:] *= np.cos(np.linspace(0, np.pi / 2, fade_len)) ** 2

                end_pos = min(cursor + len(seg), loop_samples)
                copy_len = end_pos - cursor
                loop_section[cursor:end_pos] += seg[:copy_len]
                cursor += segment_len - crossfade_len

        # Normalize loop
        loop_rms = np.sqrt(np.mean(loop_section ** 2))
        if loop_rms > 0:
            loop_section = loop_section * (0.12 / loop_rms)

        # Make seamlessly loopable
        fade_len = int(0.05 * sr)
        if len(loop_section) > fade_len * 2:
            fade_out = np.cos(np.linspace(0, np.pi / 2, fade_len)) ** 2
            fade_in = np.sin(np.linspace(0, np.pi / 2, fade_len)) ** 2
            loop_section[-fade_len:] = loop_section[-fade_len:] * fade_out + loop_section[:fade_len] * fade_in

        if progress_callback:
            progress_callback(4, 5, "Assembling output")

        # Crossfade attack into loop
        attack_fade = min(crossfade_len, len(attack), len(loop_section))
        if attack_fade > 0:
            attack[-attack_fade:] *= np.cos(np.linspace(0, np.pi / 2, attack_fade)) ** 2
            loop_section[:attack_fade] *= np.sin(np.linspace(0, np.pi / 2, attack_fade)) ** 2

        # Crossfade loop into tail
        tail_fade = min(crossfade_len, len(loop_section), len(tail))
        if tail_fade > 0:
            loop_section[-tail_fade:] *= np.cos(np.linspace(0, np.pi / 2, tail_fade)) ** 2
            tail[:tail_fade] *= np.sin(np.linspace(0, np.pi / 2, tail_fade)) ** 2

        output = np.concatenate([attack, loop_section, tail])

        loop_start_sample = len(attack)
        loop_end_sample = len(attack) + len(loop_section) - 1

        # Normalize
        max_val = np.max(np.abs(output))
        if max_val > 0:
            output /= max_val + 1e-8

        pcm = (output * 32767).astype(np.int16)

        # Output filename
        base_name = os.path.splitext(os.path.basename(source_wav))[0]
        if "single" in base_name.lower():
            base_name = re.sub(r'single', 'beam', base_name, flags=re.IGNORECASE)
        else:
            base_name = f"{base_name}_beam"

        output_file = os.path.join(output_dir, f"{base_name}.wav")

        marker_defs = [
            (loop_start_sample, "LOOP_START"),
            (loop_end_sample, "LOOP_END"),
        ]
        _write_wav_with_markers(output_file, pcm, sr, marker_defs,
                                loop_start_sample, loop_end_sample)

        result["files"].append(output_file)
        _log.info("Created beam sound: %s", output_file)
        _log.info("Loop region: %d - %d samples (%.3fs - %.3fs)",
                   loop_start_sample, loop_end_sample,
                   loop_start_sample / sr, loop_end_sample / sr)

        if progress_callback:
            progress_callback(5, 5, "Done")

    except Exception as e:
        result["errors"].append(str(e))
        _log.error("Failed to generate laser beam: %s", e)

    return result
