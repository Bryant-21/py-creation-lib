"""Voice archives tagged for another language must not be indexed.

Bethesda ships identical member paths across per-language voice archives, so
indexing all of them makes the audio behind a line non-deterministic.
"""
import pytest

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


def test_numbered_and_untagged_archives_are_kept(tmp_path):
    numbered_dir = tmp_path / "numbered"
    numbered_dir.mkdir()
    _touch(numbered_dir, "Skyrim - Voices_en0.bsa", "Skyrim - Voices_de0.bsa")
    found = [path.name for path in discover_archives(numbered_dir)]
    assert found == ["Skyrim - Voices_en0.bsa"]

    # FO4, FNV and FO3 name voice archives without a language code.
    untagged_dir = tmp_path / "untagged"
    untagged_dir.mkdir()
    _touch(
        untagged_dir,
        "Fallout4 - Voices.ba2",
        "Fallout - Voices1.bsa",
        "Starfield - Voices01.ba2",
        "Starfield - VoicesPatch.ba2",
        "Fallout4 - Textures1.ba2",
    )
    found = sorted(path.name for path in discover_archives(untagged_dir))
    assert found == [
        "Fallout - Voices1.bsa",
        "Fallout4 - Textures1.ba2",
        "Fallout4 - Voices.ba2",
        "Starfield - Voices01.ba2",
        "Starfield - VoicesPatch.ba2",
    ]


@pytest.mark.parametrize(
    ("language", "expected"),
    [
        ("German", "SFBGS003 - Voices_de.ba2"),
        ("Klingon", "SFBGS003 - Voices_en.ba2"),
    ],
    ids=["matching-language", "unknown-language-falls-back-to-english"],
)
def test_language_selects_matching_or_falls_back_to_english(tmp_path, language, expected):
    _touch(tmp_path, "SFBGS003 - Voices_en.ba2", "SFBGS003 - Voices_de.ba2")
    found = [path.name for path in discover_archives(tmp_path, language=language)]
    assert found == [expected]


def test_missing_directory_is_still_empty(tmp_path):
    assert discover_archives(tmp_path / "nope") == []
