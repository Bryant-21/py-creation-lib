from __future__ import annotations

from pathlib import Path

import pytest

from creation_lib.ui.workspaces.voice_browser import workspace as workspace_module
from creation_lib.ui.workspaces.voice_browser.workspace import VoiceBrowserWorkspace


@pytest.mark.parametrize("suffix", [".ogg", ".wem", ".fuz", ".xwm"])
def test_prepare_playable_audio_delegates_to_extract(tmp_path, monkeypatch, suffix: str) -> None:
    workspace = VoiceBrowserWorkspace.__new__(VoiceBrowserWorkspace)
    source = tmp_path / f"voice{suffix}"
    source.write_bytes(b"placeholder")

    def fake_decode(path: Path, output_dir: Path, *, keep_lip: bool = False) -> Path:
        assert path == source
        assert output_dir == tmp_path
        assert keep_lip is False
        return output_dir / f"{path.stem}.wav"

    monkeypatch.setattr(workspace_module, "decode_to_wav", fake_decode)

    result = workspace._prepare_playable_audio(source, tmp_path, keep_lip=False)

    assert result == tmp_path / "voice.wav"
