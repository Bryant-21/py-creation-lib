from __future__ import annotations

from pathlib import Path

from creation_lib.mod import inspector


LEGACY_SPRIGGIT = "sprig" + "git"


def test_inspect_mod_reports_yaml_serialization_key_when_skipped(tmp_path: Path) -> None:
    mod_dir = tmp_path / "ExampleMod"
    mod_dir.mkdir()
    (mod_dir / "ExampleMod.esp").write_bytes(b"TES4")

    report = inspector.inspect_mod(
        mod_dir,
        game="fo4",
        skip_authoring_yaml=True,
    )

    assert "yaml_serialization" in report
    assert LEGACY_SPRIGGIT not in report
    assert report["yaml_serialization"] == {"serialized": [], "skipped": [], "errors": []}


def test_serialize_plugins_uses_authoring_yaml_temp_dir_name(
    tmp_path: Path,
    monkeypatch,
) -> None:
    mod_dir = tmp_path / "ExampleMod"
    mod_dir.mkdir()
    (mod_dir / "Main.esp").write_bytes(b"TES4")
    (mod_dir / "Patch.esp").write_bytes(b"TES4")
    serialize_calls: list[tuple[str, Path]] = []

    def fake_serialize(plugin_path, output_dir, *, game, data_folder, error_on_unknown, on_progress):
        serialize_calls.append((Path(plugin_path).name, Path(output_dir)))
        created = Path(output_dir) / "yaml"
        created.mkdir(parents=True, exist_ok=True)
        (created / "plugin.yaml").write_text(f"plugin: {Path(plugin_path).name}\n", encoding="utf-8")
        return created

    monkeypatch.setattr("creation_lib.esp.authoring.serialize", fake_serialize)

    result = inspector._serialize_plugins(
        mod_dir,
        [
            {"name": "Main.esp", "size_bytes": 4, "type": "esp"},
            {"name": "Patch.esp", "size_bytes": 4, "type": "esp"},
        ],
        game="fo4",
        data_folder=None,
        force=True,
        all_plugins=True,
        on_progress=None,
    )

    assert ("Patch.esp", mod_dir / "_tmp_authoring_yaml_Patch") in serialize_calls
    assert not (mod_dir / f"_tmp_{LEGACY_SPRIGGIT}_Patch").exists()
    assert not (mod_dir / "_tmp_authoring_yaml_Patch").exists()
    assert (mod_dir / "patches" / "Patch" / "yaml" / "plugin.yaml").is_file()
    assert [entry["plugin"] for entry in result["serialized"]] == ["Main.esp", "Patch.esp"]
