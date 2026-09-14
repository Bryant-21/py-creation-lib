"""Per-game dispatch of the voice release chain.

The external tools are stubbed; this pins which ones run, in what order,
and what the caller gets back.
"""
import pytest

from creation_lib.audio import release


@pytest.fixture
def calls(monkeypatch, tmp_path):
    recorded = []

    def fake_lip(wav_path, lip_path, transcript, *, ffmpeg_path="ffmpeg",
                 game="Fallout4", language="USEnglish", resource_dir=None):
        recorded.append(("lip", game))
        open(lip_path, "wb").write(b"\x01\x00\x00\x00lip")
        return True

    def fake_xwm(wav_path, xwm_path, *, resource_dir=None):
        recorded.append(("xwm", None))
        open(xwm_path, "wb").write(b"xwm")
        return True

    def fake_fuz(fuz_path, xwm_path, lip_path, *, resource_dir=None):
        recorded.append(("fuz", None))
        open(fuz_path, "wb").write(b"FUZE")
        return True

    def fake_ogg(wav_path, ogg_path, *, ffmpeg_path="ffmpeg"):
        recorded.append(("ogg", None))
        open(ogg_path, "wb").write(b"OggS")
        return True

    monkeypatch.setattr(release, "create_lip", fake_lip)
    monkeypatch.setattr(release, "create_xwm", fake_xwm)
    monkeypatch.setattr(release, "create_fuz", fake_fuz)
    monkeypatch.setattr(release, "create_ogg", fake_ogg)
    return recorded


@pytest.fixture
def wav(tmp_path):
    path = tmp_path / "0001a2b3_1.wav"
    path.write_bytes(b"RIFF")
    return path


def test_fo4_runs_lip_then_xwm_then_fuz(calls, wav, tmp_path):
    result = release.create_voice_release(
        str(wav), game="fo4", transcript="Hello there.",
        out_dir=tmp_path, resource_dir=tmp_path,
    )
    assert [name for name, _ in calls] == ["lip", "xwm", "fuz"]
    assert calls[0][1] == "Fallout4"
    assert result.primary == tmp_path / "0001a2b3_1.fuz"
    assert result.lip == tmp_path / "0001a2b3_1.lip"
    assert result.intermediates == (tmp_path / "0001a2b3_1.xwm",)


def test_skyrimse_uses_the_same_chain_with_its_own_facefx_type(calls, wav, tmp_path):
    result = release.create_voice_release(
        str(wav), game="skyrimse", transcript="Hello there.",
        out_dir=tmp_path, resource_dir=tmp_path,
    )
    assert [name for name, _ in calls] == ["lip", "xwm", "fuz"]
    assert calls[0][1] == "Skyrim"
    assert result.primary.suffix == ".fuz"


def test_fnv_writes_a_sidecar_lip_and_an_ogg(calls, wav, tmp_path):
    result = release.create_voice_release(
        str(wav), game="fnv", transcript="Hello there.",
        out_dir=tmp_path, resource_dir=tmp_path,
    )
    assert [name for name, _ in calls] == ["lip", "ogg"]
    assert calls[0][1] == "Skyrim"
    assert result.primary == tmp_path / "0001a2b3_1.ogg"
    assert result.lip == tmp_path / "0001a2b3_1.lip"
    assert result.intermediates == ()


def test_fo3_matches_fnv(calls, wav, tmp_path):
    result = release.create_voice_release(
        str(wav), game="fo3", transcript="Hello there.",
        out_dir=tmp_path, resource_dir=tmp_path,
    )
    assert [name for name, _ in calls] == ["lip", "ogg"]
    assert result.primary.suffix == ".ogg"


def test_starfield_copies_the_wav_and_writes_no_lip(calls, wav, tmp_path):
    out = tmp_path / "out"
    result = release.create_voice_release(
        str(wav), game="starfield", transcript="Hello there.",
        out_dir=out, resource_dir=tmp_path,
    )
    assert calls == []
    assert result.primary == out / "0001a2b3_1.wav"
    assert result.primary.is_file()
    assert result.lip is None


def test_an_existing_lip_is_reused_instead_of_generated(calls, wav, tmp_path):
    original = tmp_path / "original.lip"
    original.write_bytes(b"\x01\x00\x00\x00original")
    result = release.create_voice_release(
        str(wav), game="fnv", transcript="Hello there.",
        out_dir=tmp_path, existing_lip=str(original), resource_dir=tmp_path,
    )
    assert [name for name, _ in calls] == ["ogg"]
    assert result.lip == original


def test_fuz_is_abandoned_when_lip_generation_fails(monkeypatch, calls, wav, tmp_path):
    monkeypatch.setattr(release, "create_lip", lambda *a, **k: False)
    result = release.create_voice_release(
        str(wav), game="fo4", transcript="Hello there.",
        out_dir=tmp_path, resource_dir=tmp_path,
    )
    assert result is None


def test_ogg_still_ships_when_lip_generation_fails(monkeypatch, calls, wav, tmp_path):
    """A new FNV line with no original lip is audio-only, not a failure."""
    monkeypatch.setattr(release, "create_lip", lambda *a, **k: False)
    result = release.create_voice_release(
        str(wav), game="fnv", transcript="Hello there.",
        out_dir=tmp_path, resource_dir=tmp_path,
    )
    assert result is not None
    assert result.primary.suffix == ".ogg"
    assert result.lip is None


def test_a_profile_with_no_voice_fields_passes_the_wav_through(calls, wav, tmp_path):
    """oblivion and fo76 keep the defaults, which means wav pass-through."""
    out = tmp_path / "out"
    result = release.create_voice_release(
        str(wav), game="fo76", transcript="Hello there.",
        out_dir=out, resource_dir=tmp_path,
    )
    assert calls == []
    assert result.primary == out / "0001a2b3_1.wav"


def test_an_unrecognised_container_raises(monkeypatch, wav, tmp_path):
    class BogusProfile:
        voice_container = "flac"
        voice_lip = None
        facefx_game = None
        display_name = "Bogus"

    monkeypatch.setattr(release, "get_profile", lambda game: BogusProfile())
    with pytest.raises(ValueError, match="flac"):
        release.create_voice_release(
            str(wav), game="whatever", transcript="Hello there.",
            out_dir=tmp_path, resource_dir=tmp_path,
        )
