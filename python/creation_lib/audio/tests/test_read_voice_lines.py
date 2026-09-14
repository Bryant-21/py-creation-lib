from __future__ import annotations

import pytest

from creation_lib.audio import voice_reference

_ROW = (
    "fo4",
    "Fallout4.esm",
    "00112D18",
    1,
    "The moving walkway is ending. Please watch your step.",
    "00112d18_1.fuz",
    "announcer_airportvoice",
    [],
    r"D:\FO4\Data\Fallout4 - Voices.ba2",
    "sound/voice/fallout4.esm/announcer_airportvoice/00112d18_1.fuz",
    "0001A0B2",
    "Topic",
)


def test_read_voice_lines_converts_native_rows(tmp_path, monkeypatch):
    db = tmp_path / "fo4_voice_reference.db"
    db.write_bytes(b"")
    monkeypatch.setattr(
        voice_reference.esp_native_runtime,
        "voice_reference_read_index",
        lambda path: [_ROW],
    )

    lines = voice_reference.read_voice_lines(db)

    assert len(lines) == 1
    assert lines[0].voice_type == "announcer_airportvoice"
    assert lines[0].response_filename == "00112d18_1.fuz"
    assert lines[0].plugin == "Fallout4.esm"
    assert lines[0].available


def test_read_voice_lines_missing_file(tmp_path):
    with pytest.raises(FileNotFoundError):
        voice_reference.read_voice_lines(tmp_path / "nope.db")
