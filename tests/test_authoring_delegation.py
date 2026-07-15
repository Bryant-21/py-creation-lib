"""Verify creation_lib.esp.authoring routes through the in-process native runtime.

``serialize`` and ``deserialize`` must delegate to
``creation_lib.esp.native_runtime.export_authoring_dir_native`` and
``creation_lib.esp.api.build_authoring_dir`` respectively, with no subprocess calls.
"""
from __future__ import annotations

from pathlib import Path
from unittest.mock import patch

import pytest

from creation_lib.esp import authoring


def test_serialize_calls_export_authoring_dir_native(tmp_path: Path) -> None:
    fake_esp = tmp_path / "FakePlugin.esp"
    fake_esp.write_bytes(b"")  # presence check only

    with patch("creation_lib.esp.native_runtime.export_authoring_dir_native") as native_export:
        with patch("subprocess.run") as subprocess_run:
            yaml_dir = authoring.serialize(
                fake_esp,
                tmp_path / "out",
                game="fo4",
                data_folder=None,
            )

    assert subprocess_run.call_count == 0, "Authoring layer must not shell out"
    assert native_export.call_count == 1
    args, kwargs = native_export.call_args
    assert args[0] == str(fake_esp)
    assert args[1] == str(tmp_path / "out" / "yaml")
    assert kwargs["game"] == "fo4"
    assert kwargs["format"] == "yaml"
    assert yaml_dir == tmp_path / "out" / "yaml"


def test_deserialize_calls_build_authoring_dir(tmp_path: Path) -> None:
    yaml_dir = tmp_path / "yaml"
    yaml_dir.mkdir()
    (yaml_dir / "plugin.yaml").write_text("plugin: TestMod.esp\ngame: fo4\n", encoding="utf-8")

    output = tmp_path / "TestMod.esp"

    def _materialize(*_args, **_kwargs) -> None:
        output.write_bytes(b"\x00")  # pretend the build wrote a file

    with patch("creation_lib.esp.api.build_authoring_dir", side_effect=_materialize) as build:
        with patch("subprocess.run") as subprocess_run:
            result = authoring.deserialize(
                yaml_dir,
                output,
                game="fo4",
                data_folder=None,
            )

    assert subprocess_run.call_count == 0, "Authoring layer must not shell out"
    assert build.call_count == 1
    assert result == output


def test_deserialize_rejects_legacy_authoring_format(tmp_path: Path) -> None:
    yaml_dir = tmp_path / "yaml"
    yaml_dir.mkdir()
    (yaml_dir / "spriggit-meta.json").write_text("{}", encoding="utf-8")

    with pytest.raises(RuntimeError) as exc:
        authoring.deserialize(
            yaml_dir,
            tmp_path / "out.esp",
            game="fo4",
        )

    message = str(exc.value)
    assert "Legacy authoring YAML format detected" in message
    assert "modkit mod import" in message
    assert "Spriggit YAML format" not in message


def test_deserialize_rejects_directory_without_plugin_manifest(tmp_path: Path) -> None:
    yaml_dir = tmp_path / "yaml"
    yaml_dir.mkdir()

    with pytest.raises(RuntimeError, match="does not look like an ESP authoring directory"):
        authoring.deserialize(
            yaml_dir,
            tmp_path / "out.esp",
            game="fo4",
        )


def test_get_plugin_ext_reads_plugin_yaml(tmp_path: Path) -> None:
    mod_dir = tmp_path / "B21_Test"
    yaml_dir = mod_dir / "yaml"
    yaml_dir.mkdir(parents=True)
    (yaml_dir / "plugin.yaml").write_text(
        "format_version: 1\nplugin: B21_Test.esl\ngame: fo4\n",
        encoding="utf-8",
    )

    assert authoring.get_plugin_ext(mod_dir) == "esl"


def test_get_plugin_ext_falls_back_to_sibling_plugin(tmp_path: Path) -> None:
    mod_dir = tmp_path / "B21_Test"
    mod_dir.mkdir()
    (mod_dir / "B21_Test.esp").write_bytes(b"")

    assert authoring.get_plugin_ext(mod_dir) == "esp"


def test_get_plugin_ext_default_is_esp(tmp_path: Path) -> None:
    mod_dir = tmp_path / "Empty"
    mod_dir.mkdir()
    assert authoring.get_plugin_ext(mod_dir) == "esp"


def test_new_mod_yaml_writes_scaffold(tmp_path: Path) -> None:
    mod_dir = tmp_path / "B21_NewMod"
    mod_dir.mkdir()

    yaml_dir = authoring.new_mod_yaml(
        "B21_NewMod",
        mod_dir,
        game="fo4",
        plugin_ext="esl",
        mod_prefix="B21",
    )

    assert yaml_dir == mod_dir / "yaml"
    assert (yaml_dir / "plugin.yaml").is_file()
    assert not (yaml_dir / "spriggit-meta.json").exists()
    assert not (yaml_dir / "RecordData.yaml").exists()

    text = (yaml_dir / "plugin.yaml").read_text(encoding="utf-8")
    assert "B21_NewMod.esl" in text
    assert "fo4" in text
