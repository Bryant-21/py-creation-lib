from __future__ import annotations

from pathlib import Path

from creation_lib.audio.voice_reference import (
    VoiceLine,
    build_voice_reference,
    extract_voice_line,
    group_voice_lines,
    load_cached_voice_reference,
    search_voice_lines,
    voice_reference_sqlite_cache_path,
)


def test_build_voice_reference_prefers_native_sqlite_cache(tmp_path: Path, monkeypatch) -> None:
    data_dir = tmp_path / "Data"
    strings_dir = data_dir / "Strings"
    strings_dir.mkdir(parents=True)
    plugin_path = data_dir / "Fallout4.esm"
    archive_path = data_dir / "Fallout4 - Voices.ba2"
    plugin_path.write_bytes(b"plugin")
    archive_path.write_bytes(b"archive")
    calls: list[dict[str, object]] = []

    def _build_index(
        db_path: str,
        game: str,
        data_dir_arg: str,
        strings_dir_arg: str,
        language: str,
        cache_key: str,
        plugin_paths: list[str],
        archive_paths: list[str],
        *,
        force: bool = False,
    ) -> tuple[str, int, bool]:
        calls.append(
            {
                "db_path": db_path,
                "game": game,
                "data_dir": data_dir_arg,
                "strings_dir": strings_dir_arg,
                "language": language,
                "cache_key": cache_key,
                "plugin_paths": plugin_paths,
                "archive_paths": archive_paths,
                "force": force,
            }
        )
        assert db_path.endswith("_voice_reference.db")
        return db_path, 1, False

    def _read_index(db_path: str) -> list[tuple[object, ...]]:
        return [
            (
                "fo4",
                "Fallout4.esm",
                "00001234",
                1,
                "Native row.",
                "00001234_1.fuz",
                "male",
                ["Test NPC"],
                str(archive_path),
                "sound/voice/fallout4.esm/male/00001234_1.fuz",
                "",
                "",
            )
        ]

    monkeypatch.setattr("creation_lib.audio.voice_reference.esp_native_runtime.voice_reference_build_index", _build_index)
    monkeypatch.setattr("creation_lib.audio.voice_reference.esp_native_runtime.voice_reference_read_index", _read_index)

    index = build_voice_reference(
        game="fo4",
        data_dir=data_dir,
        strings_dir=strings_dir,
        db_dir=tmp_path / "cache",
        plugin_paths=[plugin_path],
        archive_paths=[archive_path],
    )

    assert calls
    assert index.lines[0].response_text == "Native row."
    assert index.lines[0].characters == ["Test NPC"]


def test_load_cached_voice_reference_does_not_build(tmp_path: Path, monkeypatch) -> None:
    data_dir = tmp_path / "Data"
    strings_dir = data_dir / "Strings"
    strings_dir.mkdir(parents=True)
    plugin_path = data_dir / "Fallout4.esm"
    archive_path = data_dir / "Fallout4 - Voices.ba2"
    plugin_path.write_bytes(b"plugin")
    archive_path.write_bytes(b"archive")
    cache_path = voice_reference_sqlite_cache_path(
        game="fo4",
        data_dir=data_dir,
        strings_dir=strings_dir,
        db_dir=tmp_path / "cache",
        plugin_paths=[plugin_path],
        archive_paths=[archive_path],
    )
    assert cache_path is not None
    cache_path.parent.mkdir(parents=True)
    cache_path.write_bytes(b"sqlite placeholder")

    def _build_index(*args, **kwargs):
        raise AssertionError("cache load should not build")

    def _read_index(db_path: str) -> list[tuple[object, ...]]:
        assert Path(db_path) == cache_path
        return [
            (
                "fo4",
                "Fallout4.esm",
                "00001234",
                1,
                "Cached row.",
                "00001234_1.fuz",
                "male",
                ["Cached NPC"],
                str(archive_path),
                "sound/voice/fallout4.esm/male/00001234_1.fuz",
                "",
                "",
            )
        ]

    monkeypatch.setattr("creation_lib.audio.voice_reference.esp_native_runtime.voice_reference_build_index", _build_index)
    monkeypatch.setattr("creation_lib.audio.voice_reference.esp_native_runtime.voice_reference_read_index", _read_index)

    index = load_cached_voice_reference(
        game="fo4",
        data_dir=data_dir,
        strings_dir=strings_dir,
        db_dir=tmp_path / "cache",
        plugin_paths=[plugin_path],
        archive_paths=[archive_path],
    )

    assert index is not None
    assert index.lines[0].response_text == "Cached row."


def test_load_cached_voice_reference_falls_back_to_latest_sqlite(tmp_path: Path, monkeypatch) -> None:
    data_dir = tmp_path / "Data"
    strings_dir = data_dir / "Strings"
    strings_dir.mkdir(parents=True)
    plugin_path = data_dir / "Fallout4.esm"
    archive_path = data_dir / "Fallout4 - Voices.ba2"
    plugin_path.write_bytes(b"plugin")
    archive_path.write_bytes(b"archive")
    fallback = tmp_path / "cache" / "voice_reference" / "fo4_old.sqlite"
    fallback.parent.mkdir(parents=True)
    fallback.write_bytes(b"sqlite placeholder")

    def _read_index(db_path: str) -> list[tuple[object, ...]]:
        assert Path(db_path) == fallback
        return [
            (
                "fo4",
                "Fallout4.esm",
                "00001234",
                1,
                "Cached row.",
                "00001234_1.fuz",
                "male",
                ["???", "\x08, &", "Good Name"],
                str(archive_path),
                "sound/voice/fallout4.esm/male/00001234_1.fuz",
                "",
                "",
            )
        ]

    monkeypatch.setattr("creation_lib.audio.voice_reference.esp_native_runtime.voice_reference_read_index", _read_index)

    index = load_cached_voice_reference(
        game="fo4",
        data_dir=data_dir,
        strings_dir=strings_dir,
        db_dir=tmp_path / "cache",
        plugin_paths=[plugin_path],
        archive_paths=[archive_path],
    )

    assert index is not None
    assert index.lines[0].characters == ["Good Name"]


def test_extract_voice_line_uses_native_archive_backend(tmp_path: Path, monkeypatch) -> None:
    calls: list[tuple[str, str]] = []

    def _extract_one(archive: str, member: str) -> bytes:
        calls.append((archive, member))
        return b"FUZ"

    monkeypatch.setattr("creation_lib.audio.voice_reference.native_runtime.extract_one", _extract_one)
    line = VoiceLine(
        game="fo4",
        plugin="Fallout4.esm",
        info_form_id="00001234",
        response_number=1,
        response_text="Test",
        response_filename="00001234_1.fuz",
        archive_path="Voices.ba2",
        member_path="sound/voice/fallout4.esm/male/00001234_1.fuz",
    )

    written = extract_voice_line(line, tmp_path)

    assert written == tmp_path / "00001234_1.fuz"
    assert written.read_bytes() == b"FUZ"
    assert calls == [("Voices.ba2", "sound/voice/fallout4.esm/male/00001234_1.fuz")]
