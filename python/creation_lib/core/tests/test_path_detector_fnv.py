from creation_lib.core.game_profiles import GAME_PROFILES
from creation_lib.core import path_detector as pd


def test_fnv_folder_candidates_use_steam_folder_name():
    cands = pd._steam_folder_candidates("fnv", GAME_PROFILES["fnv"])
    # display_name "Fallout: New Vegas" contains a colon, so it can't be a folder name
    assert "Fallout New Vegas" in cands


def test_detect_fnv_via_vdf_finds_steam_folder(tmp_path, monkeypatch):
    monkeypatch.setattr(pd, "_detect_from_registry", lambda *a, **k: None)
    monkeypatch.setattr(pd, "_detect_from_gog_registry", lambda *a, **k: None)
    monkeypatch.setattr(pd, "_detect_from_common_paths", lambda *a, **k: None)

    pf86 = tmp_path / "PF86"
    (pf86 / "Steam" / "config").mkdir(parents=True)
    lib = tmp_path / "Lib"
    fnv = lib / "steamapps" / "common" / "Fallout New Vegas"
    (fnv / "Data").mkdir(parents=True)
    (fnv / "FalloutNV.exe").write_bytes(b"x")
    (fnv / "Data" / "Fallout - Meshes.bsa").write_bytes(b"x")

    vdf = pf86 / "Steam" / "config" / "libraryfolders.vdf"
    lib_escaped = str(lib).replace("\\", "\\\\")
    vdf.write_text(
        '"libraryfolders"\n{\n\t"0"\n\t{\n\t\t"path"\t\t"%s"\n\t}\n}\n' % lib_escaped,
        encoding="utf-8",
    )
    monkeypatch.setenv("ProgramFiles(x86)", str(pf86))

    result = pd.detect_game_path("fnv")
    assert result is not None
    assert result.replace("\\", "/").endswith("Fallout New Vegas")
