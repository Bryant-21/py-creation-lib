"""The shipped per-game type universes."""
import tarfile

import pytest

from creation_lib.pex import corpus


@pytest.mark.parametrize("game", ["fo4", "skyrimse", "starfield"])
def test_a_corpus_ships_for_each_supported_game(game):
    assert corpus.bundled_corpus_archive(game) is not None


def test_unknown_game_has_no_corpus():
    assert corpus.bundled_corpus_archive("morrowind") is None


def test_fo4_corpus_carries_the_anchor_types():
    """Every FO4 type resolves through these three; without them nothing compiles."""
    archive = corpus.bundled_corpus_archive("fo4")
    with tarfile.open(archive, "r:gz") as bundle:
        stems = {name.rsplit("/", 1)[-1].lower() for name in bundle.getnames()}
    for anchor in ("scriptobject.psc", "form.psc", "objectreference.psc"):
        assert anchor in stems


def test_expanding_is_idempotent_and_yields_a_usable_universe():
    root = corpus.bundled_corpus_root("fo4")
    assert root is not None and (root / ".complete").is_file()
    assert corpus.bundled_corpus_root("fo4") == root
    assert corpus.bundled_flags_file(root) is not None
    assert len(list(root.rglob("*.psc"))) > 1000


def test_a_partial_expansion_is_never_served(monkeypatch, tmp_path):
    """A half-corpus is worse than none: it types the rest of the calls as None."""
    monkeypatch.setattr(corpus, "_cache_root", lambda: tmp_path)

    def _boom(*args, **kwargs):
        raise OSError("disk full")

    monkeypatch.setattr(tarfile, "open", _boom)
    with pytest.raises(OSError):
        corpus.bundled_corpus_root("fo4")
    assert not any(tmp_path.glob("fo4-*/.complete"))
