from __future__ import annotations

import pytest

from creation_lib.audio import voice_reference

_FOREIGN = r"N:\Steam Games\steamapps\common\Fallout 4\Data\Fallout4 - Voices.ba2"


def _row(archive_path: str, filename: str = "00112d18_1.fuz"):
    return (
        "fo4",
        "Fallout4.esm",
        "00112D18",
        1,
        "The moving walkway is ending.",
        filename,
        "announcer_airportvoice",
        [],
        archive_path,
        f"sound/voice/fallout4.esm/announcer_airportvoice/{filename}",
        "0001A0B2",
        "Topic",
    )


def _game_dir(tmp_path, *archives: str):
    data_dir = tmp_path / "Data"
    data_dir.mkdir()
    for name in archives:
        (data_dir / name).write_bytes(b"BTDX")
    return data_dir


def _stub_index(tmp_path, monkeypatch, rows):
    db_dir = tmp_path / "db"
    db_dir.mkdir()
    (db_dir / "fo4_voice_reference.db").write_bytes(b"")
    monkeypatch.setattr(
        voice_reference.esp_native_runtime,
        "voice_reference_read_index",
        lambda path: rows,
    )
    return db_dir


def test_shipped_index_archive_paths_are_rebased_to_the_local_game(tmp_path, monkeypatch):
    data_dir = _game_dir(tmp_path, "Fallout4 - Voices.ba2")
    db_dir = _stub_index(tmp_path, monkeypatch, [_row(_FOREIGN)])

    index = voice_reference.load_cached_voice_reference(game="fo4", data_dir=data_dir, db_dir=db_dir)

    assert index is not None
    assert index.lines[0].archive_path == str(data_dir / "Fallout4 - Voices.ba2")


def test_rebase_is_applied_even_when_the_archive_is_absent_locally(tmp_path, monkeypatch):
    """A missing archive must report the local path, not the build machine's."""
    data_dir = _game_dir(tmp_path)
    db_dir = _stub_index(tmp_path, monkeypatch, [_row(_FOREIGN)])

    index = voice_reference.load_cached_voice_reference(game="fo4", data_dir=data_dir, db_dir=db_dir)

    assert index.lines[0].archive_path == str(data_dir / "Fallout4 - Voices.ba2")
    assert "Steam Games" not in index.lines[0].archive_path


def test_archives_the_caller_supplied_explicitly_are_honoured(tmp_path, monkeypatch):
    """A caller may load archives from outside the data dir; that list wins."""
    data_dir = _game_dir(tmp_path)
    outside = tmp_path / "elsewhere"
    outside.mkdir()
    external = outside / "Modded - Voices.ba2"
    external.write_bytes(b"BTDX")
    db_dir = _stub_index(tmp_path, monkeypatch, [_row(r"N:\old\Modded - Voices.ba2")])

    index = voice_reference.load_cached_voice_reference(
        game="fo4", data_dir=data_dir, db_dir=db_dir, archive_paths=[external]
    )

    assert index.lines[0].archive_path == str(external)


def test_rebase_handles_posix_and_windows_separators(tmp_path):
    data_dir = _game_dir(tmp_path, "Fallout4 - Voices.ba2")
    lines = [
        voice_reference.VoiceLine.from_native_row(_row(_FOREIGN, "a.fuz")),
        voice_reference.VoiceLine.from_native_row(_row("/mnt/games/fo4/Data/Fallout4 - Voices.ba2", "b.fuz")),
        voice_reference.VoiceLine.from_native_row(_row("", "c.fuz")),
    ]

    voice_reference._rebase_archive_paths(lines, data_dir, [data_dir / "Fallout4 - Voices.ba2"])

    assert lines[0].archive_path == str(data_dir / "Fallout4 - Voices.ba2")
    assert lines[1].archive_path == str(data_dir / "Fallout4 - Voices.ba2")
    assert lines[2].archive_path == ""


def test_extract_voice_line_names_the_missing_archive(tmp_path):
    line = voice_reference.VoiceLine.from_native_row(_row(str(tmp_path / "Data" / "Gone.ba2")))

    with pytest.raises(FileNotFoundError, match="Archive not found"):
        voice_reference.extract_voice_line(line, tmp_path / "out")
