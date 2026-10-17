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


def test_loose_install_scripts_become_headers_ahead_of_the_corpus(monkeypatch, tmp_path):
    """Script extenders ship loose .pex only; their additions (Form.GetName) must resolve."""
    from creation_lib.pex import headers

    scripts = tmp_path / "Scripts"
    scripts.mkdir()
    (scripts / "Form.pex").write_bytes(b"pex")
    monkeypatch.setattr(corpus, "_cache_root", lambda: tmp_path / "cache")
    calls = []

    def _emit(path, manifest=None):
        calls.append(path)
        yield "Form", "Scriptname Form Extends ScriptObject\n\nString Function GetName() Native\n"

    monkeypatch.setattr(headers, "emit_headers_for_file", _emit)
    root = corpus.install_headers_root("fo4", scripts)
    assert "GetName" in (root / "Form.psc").read_text(encoding="utf-8")
    assert corpus.install_headers_root("fo4", scripts) == root
    assert len(calls) == 1


def test_no_loose_install_scripts_means_no_install_headers(tmp_path):
    assert corpus.install_headers_root("fo4", tmp_path) is None
