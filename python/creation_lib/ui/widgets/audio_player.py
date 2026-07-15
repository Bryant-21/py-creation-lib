"""Inline imgui audio player widget."""
from __future__ import annotations

import logging
import shutil
import subprocess
import time
from pathlib import Path

import numpy as np
from imgui_bundle import imgui

_log = logging.getLogger(__name__)


def _format_time(seconds: float) -> str:
    minutes, secs = divmod(max(0, int(seconds)), 60)
    return f"{minutes:02d}:{secs:02d}"


def _load_audio_via_ffmpeg(path: str, sample_rate: int = 44100, channels: int = 1) -> tuple[np.ndarray, int]:
    ffmpeg_path = shutil.which("ffmpeg")
    if not ffmpeg_path:
        raise FileNotFoundError("ffmpeg not found on PATH")
    cmd = [
        ffmpeg_path,
        "-y",
        "-i",
        path,
        "-f",
        "f32le",
        "-acodec",
        "pcm_f32le",
        "-af",
        "aresample=resampler=soxr",
        "-ac",
        str(channels),
        "-ar",
        str(sample_rate),
        "pipe:1",
    ]
    result = subprocess.run(cmd, capture_output=True, check=False)
    if result.returncode != 0:
        detail = result.stderr.decode("utf-8", errors="replace").strip() or "ffmpeg decode failed"
        raise RuntimeError(detail)
    data = np.frombuffer(result.stdout, dtype=np.float32).flatten()
    if data.size == 0:
        raise RuntimeError("Decoded audio was empty")
    return data, sample_rate


class InlineAudioPlayer:
    def __init__(self) -> None:
        self._loaded_file = ""
        self._display_name = ""
        self._raw_data: np.ndarray | None = None
        self._sample_rate = 44100
        self._duration = 0.0
        self._play_pos = 0.0
        self._playing = False
        self._play_start_time = 0.0
        self._play_start_offset = 0.0
        self._volume = 1.0
        self._error_msg = ""
        self._output_devices: list[str] = []
        self._output_device_ids: list[int | None] = []
        self._out_device_idx = 0
        self._enumerate_devices()

    @property
    def is_playing(self) -> bool:
        return self._playing

    @property
    def has_audio(self) -> bool:
        return self._raw_data is not None and self._duration > 0

    @property
    def loaded_file(self) -> str:
        return self._loaded_file

    def _enumerate_devices(self) -> None:
        try:
            import sounddevice as sd

            self._output_devices = []
            self._output_device_ids = []
            for idx, device in enumerate(sd.query_devices()):
                if device["max_output_channels"] > 0:
                    self._output_devices.append(str(device["name"]))
                    self._output_device_ids.append(idx)
            if not self._output_devices:
                self._output_devices = ["Default"]
                self._output_device_ids = [None]
        except Exception:
            _log.exception("Could not enumerate audio devices")
            self._output_devices = ["Default"]
            self._output_device_ids = [None]

    def clear(self) -> None:
        self.stop()
        self._loaded_file = ""
        self._display_name = ""
        self._raw_data = None
        self._sample_rate = 44100
        self._duration = 0.0
        self._play_pos = 0.0
        self._error_msg = ""

    def set_error(self, message: str) -> None:
        self.clear()
        self._error_msg = message

    def load_file(self, path: str, display_name: str) -> None:
        self.stop()
        try:
            import soundfile as sf

            data, sample_rate = sf.read(path, always_2d=False)
            if getattr(data, "ndim", 1) > 1:
                data = data[:, 0]
            raw = np.asarray(data, dtype=np.float32)
        except Exception:
            raw, sample_rate = _load_audio_via_ffmpeg(path)
        if raw.size == 0:
            raise RuntimeError("Audio file had no samples")
        self._loaded_file = path
        self._display_name = display_name or Path(path).name
        self._raw_data = raw
        self._sample_rate = int(sample_rate)
        self._duration = float(raw.size) / float(sample_rate)
        self._play_pos = 0.0
        self._error_msg = ""

    def play(self, from_pos: float | None = None) -> None:
        if self._raw_data is None:
            return
        try:
            import sounddevice as sd

            sd.stop()
            if from_pos is not None:
                self._play_pos = max(0.0, min(from_pos, 1.0))
            start_sample = int(self._play_pos * len(self._raw_data))
            remaining = self._raw_data[start_sample:]
            if remaining.size == 0:
                self._play_pos = 0.0
                remaining = self._raw_data
            play_data = remaining * self._volume if self._volume < 0.999 else remaining
            device = None
            if 0 <= self._out_device_idx < len(self._output_device_ids):
                device = self._output_device_ids[self._out_device_idx]
            self._play_start_offset = self._play_pos * self._duration
            self._play_start_time = time.time()
            self._playing = True
            sd.play(play_data, self._sample_rate, device=device)
        except Exception as exc:
            self._playing = False
            raise RuntimeError(f"Audio playback failed: {exc}") from exc

    def pause(self) -> None:
        if self._playing and self._duration > 0:
            elapsed = time.time() - self._play_start_time
            current_time = min(self._play_start_offset + elapsed, self._duration)
            self._play_pos = current_time / self._duration
        try:
            import sounddevice as sd

            sd.stop()
        except Exception:
            pass
        self._playing = False

    def stop(self) -> None:
        try:
            import sounddevice as sd

            sd.stop()
        except Exception:
            pass
        self._playing = False
        self._play_pos = 0.0

    def seek(self, pos: float) -> None:
        self._play_pos = max(0.0, min(pos, 1.0))
        if self._playing:
            self.play(from_pos=self._play_pos)

    def update(self) -> None:
        if not self._playing or self._duration <= 0:
            return
        elapsed = time.time() - self._play_start_time
        current_time = self._play_start_offset + elapsed
        if current_time >= self._duration:
            self._playing = False
            self._play_pos = 0.0
            return
        self._play_pos = current_time / self._duration

    def draw(self, suffix: str = "") -> None:
        from imgui_bundle import icons_fontawesome_6 as fa

        self.update()

        if self._error_msg:
            imgui.push_style_color(imgui.Col_.text, imgui.ImVec4(1.0, 0.35, 0.35, 1.0))
            imgui.text_wrapped(self._error_msg)
            imgui.pop_style_color()

        if not self.has_audio:
            imgui.text_disabled("Preview is not loaded.")
            return

        play_label = getattr(fa, "ICON_FA_PAUSE", "||") if self._playing else getattr(fa, "ICON_FA_PLAY", ">")
        if imgui.button(f"{play_label}##audio_toggle{suffix}"):
            if self._playing:
                self.pause()
            else:
                self.play()
        if imgui.is_item_hovered():
            imgui.set_tooltip("Pause" if self._playing else "Play")

        imgui.same_line()
        if imgui.button(f"{getattr(fa, 'ICON_FA_STOP', '[]')}##audio_stop{suffix}"):
            self.stop()
        if imgui.is_item_hovered():
            imgui.set_tooltip("Stop")

        imgui.same_line()
        imgui.text_unformatted(self._display_name or Path(self._loaded_file).name)
        imgui.same_line()
        imgui.text_disabled(f"{_format_time(self._play_pos * self._duration)} / {_format_time(self._duration)}")

        imgui.set_next_item_width(-1)
        changed, new_pos = imgui.slider_float(
            f"##audio_seek{suffix}",
            self._play_pos,
            0.0,
            1.0,
            format="",
        )
        if changed:
            self.seek(new_pos)

        imgui.text_disabled("Volume")
        imgui.same_line()
        imgui.set_next_item_width(180)
        changed, new_volume = imgui.slider_float(
            f"##audio_volume{suffix}",
            self._volume * 100.0,
            0.0,
            100.0,
            format="%.0f%%",
        )
        if changed:
            self._volume = new_volume / 100.0

        imgui.same_line()
        imgui.text_disabled("Output")
        imgui.same_line()
        imgui.set_next_item_width(max(140, imgui.get_content_region_avail().x))
        changed, new_idx = imgui.combo(
            f"##audio_device{suffix}",
            self._out_device_idx,
            self._output_devices,
        )
        if changed:
            self._out_device_idx = new_idx
