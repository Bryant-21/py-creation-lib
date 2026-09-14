"""Voice archives tagged for another language must not be indexed.

Bethesda ships identical member paths across per-language voice archives, so
indexing all of them makes the audio behind a line non-deterministic.
"""
from creation_lib.audio.voice_reference import discover_archives


def _touch(directory, *names):
    for name in names:
        (directory / name).write_bytes(b"")


def test_foreign_language_voice_archives_are_skipped(tmp_path):
    _touch(
        tmp_path,
        "SFBGS003 - Voices_en.ba2",
        "SFBGS003 - Voices_de.ba2",
        "SFBGS003 - Voices_es.ba2",
        "SFBGS003 - Voices_fr.ba2",
        "SFBGS003 - Voices_ja.ba2",
    )
    found = [path.name for path in discover_archives(tmp_path)]
    assert found == ["SFBGS003 - Voices_en.ba2"]


def test_numbered_english_archives_are_kept(tmp_path):
    _touch(tmp_path, "Skyrim - Voices_en0.bsa", "Skyrim - Voices_de0.bsa")
    found = [path.name for path in discover_archives(tmp_path)]
    assert found == ["Skyrim - Voices_en0.bsa"]


def test_untagged_archives_are_always_kept(tmp_path):
    """FO4, FNV and FO3 name voice archives without a language code."""
    _touch(
        tmp_path,
        "Fallout4 - Voices.ba2",
        "Fallout - Voices1.bsa",
        "Starfield - Voices01.ba2",
        "Starfield - VoicesPatch.ba2",
        "Fallout4 - Textures1.ba2",
    )
    found = sorted(path.name for path in discover_archives(tmp_path))
    assert found == [
        "Fallout - Voices1.bsa",
        "Fallout4 - Textures1.ba2",
        "Fallout4 - Voices.ba2",
        "Starfield - Voices01.ba2",
        "Starfield - VoicesPatch.ba2",
    ]


def test_a_different_language_selects_its_own_archives(tmp_path):
    _touch(tmp_path, "SFBGS003 - Voices_en.ba2", "SFBGS003 - Voices_de.ba2")
    found = [path.name for path in discover_archives(tmp_path, language="German")]
    assert found == ["SFBGS003 - Voices_de.ba2"]


def test_unknown_language_falls_back_to_english(tmp_path):
    _touch(tmp_path, "SFBGS003 - Voices_en.ba2", "SFBGS003 - Voices_de.ba2")
    found = [path.name for path in discover_archives(tmp_path, language="Klingon")]
    assert found == ["SFBGS003 - Voices_en.ba2"]


def test_missing_directory_is_still_empty(tmp_path):
    assert discover_archives(tmp_path / "nope") == []
