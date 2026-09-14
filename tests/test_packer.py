from __future__ import annotations

from subprocess import CompletedProcess
from pathlib import Path
from unittest.mock import patch

import pytest

from creation_lib.build.packer import _tile_textures_for_xbox
from creation_lib.build.packer import pack_mod


def _build_mod_tree(root: Path, mod_name: str) -> Path:
    mod_dir = root / "mods" / mod_name
    (mod_dir / "data" / "Textures" / "Effects").mkdir(parents=True, exist_ok=True)
    (mod_dir / "data" / "Meshes").mkdir(parents=True, exist_ok=True)
    (mod_dir / "data" / "Textures" / "base.dds").write_bytes(b"base")
    (mod_dir / "data" / "Textures" / "Effects" / "effect.dds").write_bytes(b"effect")
    (mod_dir / "data" / "Meshes" / "model.nif").write_bytes(b"mesh")
    return mod_dir


def test_pack_mod_uses_separate_effects_resizes_for_pc_and_xbox(tmp_path):
    mod_name = "B21_TestPack"
    _build_mod_tree(tmp_path, mod_name)

    batch_resize_calls: list[dict] = []
    native_calls: list[tuple[str, str, str, bool]] = []
    archive2_calls: list[tuple[str, str, str, str]] = []

    def _fake_batch_resize(*, input_dir, output_dir, sizes, **kwargs):
        batch_resize_calls.append(
            {
                "input_dir": Path(input_dir).as_posix(),
                "output_dir": Path(output_dir).as_posix(),
                "sizes": list(sizes),
                "ignore_patterns": kwargs.get("ignore_patterns"),
            }
        )
        return {"processed": 0, "failed": 0, "errors": []}

    def _fake_native_pack(src, out, archive_type, compress=True, **kwargs):
        native_calls.append((Path(src).as_posix(), Path(out).name, archive_type, compress))
        Path(out).write_bytes(b"native")

    (tmp_path / "resource").mkdir()
    (tmp_path / "resource" / "xtexconv.exe").write_bytes(b"")

    with patch("creation_lib.build.packer.native_runtime.native_function_available", side_effect=lambda name: name == "pack_archive"), \
            patch("creation_lib.build.packer.native_runtime.pack_archive", side_effect=_fake_native_pack), \
            patch("creation_lib.build.packer._run_archive2", side_effect=lambda archive2, src, out, fmt, compression: archive2_calls.append((Path(src).as_posix(), Path(out).name, fmt, compression))), \
            patch("creation_lib.build.packer._tile_textures_for_xbox"), \
            patch("creation_lib.dds.batch_resize", side_effect=_fake_batch_resize):
        pack_mod(
            mod_name,
            pc=True,
            xbox=True,
            pc_max_res=1024,
            pc_effects_max_res=512,
            xbox_max_res=1024,
            xbox_effects_max_res=256,
            game="fo4",
            use_archive2=False,
            game_dir="C:/Games/Fallout4",
            project_root=tmp_path,
            resource_dir=tmp_path / "resource",
        )

    assert len(batch_resize_calls) == 4

    assert batch_resize_calls[0]["sizes"] == [1024]
    assert batch_resize_calls[0]["ignore_patterns"] == ["Effects", "effects"]
    assert batch_resize_calls[0]["input_dir"].endswith(r"data\Textures".replace("\\", "/"))
    assert batch_resize_calls[0]["output_dir"].endswith(r"_deploy_tmp\textures_pc_src\Textures".replace("\\", "/"))

    assert batch_resize_calls[1]["sizes"] == [512]
    assert batch_resize_calls[1]["input_dir"].endswith(r"data\Textures\Effects".replace("\\", "/"))
    assert batch_resize_calls[1]["output_dir"].endswith(r"_deploy_tmp\textures_pc_src\Textures\Effects".replace("\\", "/"))

    assert batch_resize_calls[2]["sizes"] == [1024]
    assert batch_resize_calls[2]["ignore_patterns"] == ["Effects", "effects"]
    assert batch_resize_calls[2]["output_dir"].endswith(r"_deploy_tmp\textures_xbox_src\Textures".replace("\\", "/"))

    assert batch_resize_calls[3]["sizes"] == [256]
    assert batch_resize_calls[3]["input_dir"].endswith(r"data\Textures\Effects".replace("\\", "/"))
    assert batch_resize_calls[3]["output_dir"].endswith(r"_deploy_tmp\textures_xbox_src\Textures\Effects".replace("\\", "/"))

    assert [call[1:] for call in native_calls] == [
        ("B21_TestPack - Main.ba2", "fo4", True),
        ("B21_TestPack - Textures.ba2", "fo4dds", True),
        ("B21_TestPack - Main_xbox.ba2", "fo4xbox", False),
        ("B21_TestPack - Textures_xbox.ba2", "fo4xboxdds", True),
    ]
    assert archive2_calls == []


def test_pack_mod_defaults_effects_max_to_main_size(tmp_path):
    mod_name = "B21_TestPack"
    _build_mod_tree(tmp_path, mod_name)

    batch_resize_calls: list[dict] = []

    def _fake_batch_resize(*, input_dir, output_dir, sizes, **kwargs):
        batch_resize_calls.append(
            {
                "input_dir": Path(input_dir).as_posix(),
                "output_dir": Path(output_dir).as_posix(),
                "sizes": list(sizes),
            }
        )
        return {"processed": 0, "failed": 0, "errors": []}

    with patch("creation_lib.build.packer.native_runtime.native_function_available", side_effect=lambda name: name == "pack_archive"), \
            patch("creation_lib.build.packer.native_runtime.pack_archive", side_effect=lambda src, out, *args, **kwargs: Path(out).write_bytes(b"native")), \
            patch("creation_lib.build.packer._tile_textures_for_xbox"), \
            patch("creation_lib.dds.batch_resize", side_effect=_fake_batch_resize):
        pack_mod(
            mod_name,
            pc=True,
            xbox=False,
            pc_max_res=2048,
            pc_effects_max_res=None,
            game="fo4",
            use_archive2=False,
            project_root=tmp_path,
        )

    assert len(batch_resize_calls) == 2
    assert batch_resize_calls[0]["sizes"] == [2048]
    assert batch_resize_calls[1]["sizes"] == [2048]


def test_tile_textures_for_xbox_uses_xbox_cli_flags_without_overwrite(tmp_path):
    src_root = tmp_path / "src"
    dest_root = tmp_path / "dest"
    (src_root / "Effects" / "B21_PlasmaCaster").mkdir(parents=True, exist_ok=True)
    (src_root / "Effects" / "B21_PlasmaCaster" / "CritPlasmaGradBlue.dds").write_bytes(b"dds")

    calls: list[list[str]] = []

    def _fake_run(cmd, capture_output, text):
        calls.append(list(cmd))
        return CompletedProcess(cmd, 0, stdout="ok", stderr="")

    with patch("creation_lib.build.packer._find_xtexconv", return_value="xtexconv.exe"), \
            patch("creation_lib.build.packer.subprocess.run", side_effect=_fake_run):
        _tile_textures_for_xbox(str(src_root), str(dest_root), xtexconv_path="xtexconv.exe")

    assert len(calls) == 1
    assert calls[0][1] == "-xbox"
    assert "-y" not in calls[0]
    assert calls[0][2] == "-o"


def test_pack_mod_prefers_native_pack_when_available(tmp_path):
    mod_name = "B21_TestPack"
    _build_mod_tree(tmp_path, mod_name)

    native_calls: list[tuple[str, str, str, bool, int]] = []

    def _fake_batch_resize(*, input_dir, output_dir, sizes, **kwargs):
        return {"processed": 0, "failed": 0, "errors": []}

    def _fake_native_pack(src, out, archive_type, compress=True, compression_level=6, share_data=False, manifest_path=None, **kwargs):
        native_calls.append((Path(src).as_posix(), Path(out).name, archive_type, compress, compression_level))
        Path(out).write_bytes(b"native")

    with patch("creation_lib.build.packer.native_runtime.native_function_available", side_effect=lambda name: name == "pack_archive"), \
            patch("creation_lib.build.packer.native_runtime.pack_archive", side_effect=_fake_native_pack), \
            patch("creation_lib.build.packer._tile_textures_for_xbox"), \
            patch("creation_lib.dds.batch_resize", side_effect=_fake_batch_resize):
        pack_mod(
            mod_name,
            pc=True,
            xbox=False,
            pc_max_res=0,
            game="fo4",
            use_archive2=False,
            project_root=tmp_path,
        )

    assert native_calls == [
        (
            Path(tmp_path / "mods" / mod_name / "data").as_posix(),
            "B21_TestPack - Main.ba2",
            "fo4",
            True,
            9,
        ),
        (
            Path(tmp_path / "mods" / mod_name / "data").as_posix(),
            "B21_TestPack - Textures.ba2",
            "fo4dds",
            True,
            9,
        ),
    ]


def test_pack_mod_fo4_og_uses_v1_archive_tokens(tmp_path):
    mod_name = "B21_TestPack"
    _build_mod_tree(tmp_path, mod_name)

    archive_types: list[str] = []

    def _fake_native_pack(src, out, archive_type, **kwargs):
        archive_types.append(archive_type)
        Path(out).write_bytes(b"native")

    with patch(
        "creation_lib.build.packer.native_runtime.native_function_available",
        return_value=True,
    ), patch(
        "creation_lib.build.packer.native_runtime.pack_mod_archives",
        side_effect=AssertionError("OG packing must use the v1-capable path"),
    ), patch(
        "creation_lib.build.packer.native_runtime.pack_archive",
        side_effect=_fake_native_pack,
    ):
        pack_mod(
            mod_name,
            pc=True,
            xbox=False,
            game="fo4",
            project_root=tmp_path,
            fo4_ba2_target="og",
        )

    assert archive_types == ["fo4og", "fo4ogdds"]


def test_pack_mod_fo4_og_writes_v1_header(tmp_path):
    mod_name = "B21_TestPack"
    mod_dir = tmp_path / "mods" / mod_name
    mesh = mod_dir / "data" / "Meshes" / "model.nif"
    mesh.parent.mkdir(parents=True)
    mesh.write_bytes(b"mesh")

    pack_mod(
        mod_name,
        pc=True,
        xbox=False,
        game="fo4",
        project_root=tmp_path,
        fo4_ba2_target="og",
    )

    header = (mod_dir / f"{mod_name} - Main.ba2").read_bytes()[:8]
    assert header[:4] == b"BTDX"
    assert int.from_bytes(header[4:8], "little") == 1


def test_pack_mod_prefers_native_pack_for_xbox_fo4(tmp_path):
    mod_name = "B21_TestPack"
    _build_mod_tree(tmp_path, mod_name)

    native_calls: list[tuple[str, str, str, bool, int]] = []

    def _fake_batch_resize(*, input_dir, output_dir, sizes, **kwargs):
        return {"processed": 0, "failed": 0, "errors": []}

    def _fake_native_pack(src, out, archive_type, compress=True, compression_level=6, share_data=False, manifest_path=None, **kwargs):
        native_calls.append((Path(src).as_posix(), Path(out).name, archive_type, compress, compression_level))
        Path(out).write_bytes(b"native")

    (tmp_path / "resource").mkdir(exist_ok=True)
    (tmp_path / "resource" / "xtexconv.exe").write_bytes(b"")

    with patch("creation_lib.build.packer.native_runtime.native_function_available", side_effect=lambda name: name == "pack_archive"), \
            patch("creation_lib.build.packer.native_runtime.pack_archive", side_effect=_fake_native_pack), \
            patch("creation_lib.build.packer._find_archive2") as find_archive2, \
            patch("creation_lib.build.packer._run_archive2") as run_archive2, \
            patch("creation_lib.build.packer._tile_textures_for_xbox"), \
            patch("creation_lib.dds.batch_resize", side_effect=_fake_batch_resize):
        pack_mod(
            mod_name,
            pc=False,
            xbox=True,
            xbox_max_res=0,
            xbox_effects_max_res=0,
            game="fo4",
            use_archive2=False,
            project_root=tmp_path,
            resource_dir=tmp_path / "resource",
        )

    assert native_calls == [
        (
            Path(tmp_path / "mods" / mod_name / "_deploy_tmp" / "planned_main").as_posix(),
            "B21_TestPack - Main_xbox.ba2",
            "fo4xbox",
            False,
            9,
        ),
        (
            Path(tmp_path / "mods" / mod_name / "_deploy_tmp" / "planned_textures").as_posix(),
            "B21_TestPack - Textures_xbox.ba2",
            "fo4xboxdds",
            True,
            9,
        ),
    ]
    find_archive2.assert_not_called()
    run_archive2.assert_not_called()


def test_pack_mod_requires_native_pack_when_archive2_not_requested(tmp_path):
    mod_name = "B21_TestPack"
    _build_mod_tree(tmp_path, mod_name)

    with patch("creation_lib.build.packer.native_runtime.native_function_available", return_value=False):
        with pytest.raises(RuntimeError, match="pack_archive\\(\\) is required"):
            pack_mod(
                mod_name,
                pc=True,
                xbox=False,
                game="fo4",
                use_archive2=False,
                project_root=tmp_path,
            )
