from __future__ import annotations

from pathlib import Path

import pytest

from creation_lib.ui.workspaces.voice_browser.workspace import VoiceBrowserWorkspace


@pytest.mark.parametrize("suffix", [".ogg", ".wem"])
def test_prepare_playable_audio_routes_ogg_and_wem(tmp_path, monkeypatch, suffix: str) -> None:
    workspace = VoiceBrowserWorkspace.__new__(VoiceBrowserWorkspace)
    source = tmp_path / f"voice{suffix}"
    source.write_bytes(b"placeholder")

    def fake_convert(audio_path: Path, output_dir: Path) -> Path:
        assert audio_path == source
        assert output_dir == tmp_path
        return output_dir / f"{audio_path.stem}.wav"

    monkeypatch.setattr(workspace, "_convert_audio_to_wav", fake_convert)

    result = workspace._prepare_playable_audio(source, tmp_path, keep_lip=False)

    assert result == tmp_path / "voice.wav"
