"""Shared Voice Browser workspace."""
from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

from .workspace import VoiceBrowserWorkspace


@dataclass
class VoiceAction:
    label: str
    handler: Callable[[object, str], None]  # (voice_line, wav_path)
    tooltip: str = ""


__all__ = ["VoiceBrowserWorkspace", "VoiceAction"]
