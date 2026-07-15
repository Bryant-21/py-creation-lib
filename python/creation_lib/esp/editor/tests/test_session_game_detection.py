from __future__ import annotations

from creation_lib.esp.editor.session import detect_game


def test_detect_game_uses_target_game_sidecar_over_source_game(tmp_path):
    plugin = tmp_path / "SeventySix.esm"
    plugin.write_bytes(b"")
    (tmp_path / ".game").write_text("fo4\n", encoding="utf-8")
    (tmp_path / ".source_game").write_text("fo76\n", encoding="utf-8")

    assert detect_game(plugin, fallback="fo76") == "fo4"
